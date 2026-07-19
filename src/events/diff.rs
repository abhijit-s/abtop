//! Pure-function diff over two [`Snapshot`] values.
//!
//! `diff(prev, next, slow_tick_ran, now_ms, state)` is the single source of
//! truth for "what changed between ticks that's worth telling someone about."
//!
//! Detection rules:
//!
//! - **Lifecycle**: new `session_id` in next → `SessionStarted`. Missing
//!   `session_id` from prev → `SessionEnded`. v1 carries no reason field.
//! - **Status**: surviving session's `status` differs → `StatusChanged`.
//! - **Context threshold**: three independent per-bucket latches (70 / 90 /
//!   95). Fires on `>= bucket` from `< bucket`, clears at the next-lower
//!   hysteresis margin (65 / 85 / 90). Latches are independent — a session
//!   climbing 60 → 96 emits three events.
//! - **Token rate spike**: aggregate rate > 3× rolling baseline, latched.
//! - **Rate limits / orphan ports / MCP servers** are slow-tier and only
//!   evaluate when `slow_tick_ran == true`.
//! - **Activity**: new tool calls (by tail-length growth), compaction count
//!   growth, new subagents.
//! - **Host load**: `load1` crossing the configured threshold, latched.
//!
//! The function is pure — no `App`, no `&mut self` (`state` is `&mut` but
//! deterministic given inputs). First-tick suppression lives at the caller
//! (`App::emit_events()`).

use crate::events::types::AppEvent;
use crate::snapshot::{SessionView, Snapshot};
use std::collections::{HashMap, HashSet};

/// Hysteresis thresholds for the three `ContextThreshold` buckets.
///
/// Each row is `(bucket, fire_at, clear_below)`. Buckets are independent
/// — see plan "Decisions resolved (2026-06-10)" item 3.
pub const CONTEXT_BUCKETS: &[(u8, f64, f64)] =
    &[(70, 70.0, 65.0), (90, 90.0, 85.0), (95, 95.0, 90.0)];

/// Multiplier above the rolling baseline for `TokenRateSpike`. Cleared
/// when the rate falls back below `SPIKE_CLEAR_MULTIPLIER * baseline`.
pub const SPIKE_FIRE_MULTIPLIER: f64 = 3.0;
pub const SPIKE_CLEAR_MULTIPLIER: f64 = 1.5;

/// `HostLoadHigh` fire threshold for `load1`. Cleared at
/// `HOST_LOAD_CLEAR`.
pub const HOST_LOAD_FIRE: f64 = 4.0;
pub const HOST_LOAD_CLEAR: f64 = 3.0;

/// Mutable detector state. Lives on `App` (one instance per process);
/// persists across ticks; reset to default by tests and on publisher
/// disabled-to-enabled transitions.
#[derive(Debug, Default, Clone)]
pub struct EventDifferState {
    /// Per-`(session_id, bucket)` latch — `true` means the bucket has
    /// fired and not yet cleared.
    context_bucket_armed: HashMap<(String, u8), bool>,
    /// Per-provider quota latch — `true` means we last emitted
    /// `RateLimited` for this provider and have not yet emitted
    /// `RateLimitCleared`.
    rate_limit_armed: HashMap<String, bool>,
    /// Token-rate spike latch. `true` means we last emitted a spike
    /// event and the rate has not yet returned to baseline range.
    token_spike_armed: bool,
    /// Rolling token-rate baseline (exponential moving average).
    token_rate_baseline: Option<f64>,
    /// Host load1 latch — `true` means we last emitted `HostLoadHigh`
    /// and load has not fallen below the clear threshold.
    host_load_armed: bool,
    /// Tracked tool-call tail length per `(session_id, agent_cli)` so
    /// we can detect new appends across ticks. The tail is bounded
    /// (24) in `Snapshot`, so we compare new tail-end items against
    /// what was seen previously.
    tool_call_tail_len: HashMap<String, usize>,
    /// Tracked compaction counts per session so we only emit on
    /// increments.
    compaction_seen: HashMap<String, u32>,
    /// Tracked subagent names per session so a re-emergence does not
    /// re-fire.
    subagents_seen: HashMap<String, HashSet<String>>,
}

impl EventDifferState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Number of ticks to smooth the token-rate baseline over via EMA. A
/// half-life of ~30 ticks (~1 minute at 2s cadence) keeps the baseline
/// from chasing transient spikes.
const TOKEN_RATE_BASELINE_HALFLIFE: f64 = 30.0;

/// Compute the diff between two snapshots and update detector state.
///
/// `slow_tick_ran` MUST be true only when the collector's slow-tier
/// branch actually executed this tick. When false, slow-tier variants
/// (rate limits, orphan ports, MCP servers) are short-circuited and
/// emit nothing — the diff for those domains is deferred to the next
/// slow tick.
///
/// `_now_ms` is reserved for future use (e.g., `RateLimited` reset
/// timestamps); not consumed yet. Plumbed in to keep the signature
/// stable.
pub fn diff(
    state: &mut EventDifferState,
    prev: &Snapshot,
    next: &Snapshot,
    slow_tick_ran: bool,
    _now_ms: u64,
) -> Vec<AppEvent> {
    let mut events = Vec::new();

    diff_sessions(state, prev, next, &mut events);
    diff_context_thresholds(state, prev, next, &mut events);
    diff_token_rate(state, next, &mut events);
    diff_host_load(state, next, &mut events);
    diff_activity(state, prev, next, &mut events);

    if slow_tick_ran {
        diff_rate_limits(state, prev, next, &mut events);
        diff_orphan_ports(prev, next, &mut events);
        diff_mcp_servers(prev, next, &mut events);
    }

    events
}

fn diff_sessions(
    state: &mut EventDifferState,
    prev: &Snapshot,
    next: &Snapshot,
    out: &mut Vec<AppEvent>,
) {
    let prev_by_id: HashMap<&str, &SessionView> = prev
        .sessions
        .iter()
        .map(|s| (s.session_id.as_str(), s))
        .collect();
    let next_by_id: HashMap<&str, &SessionView> = next
        .sessions
        .iter()
        .map(|s| (s.session_id.as_str(), s))
        .collect();

    // SessionStarted: in next, not in prev.
    for (id, view) in &next_by_id {
        if !prev_by_id.contains_key(id) {
            out.push(AppEvent::SessionStarted {
                session_id: (*id).to_string(),
                agent_cli: view.agent_cli.to_string(),
                pid: view.pid,
            });
        }
    }

    // SessionEnded: in prev, not in next. Also purge detector state.
    for id in prev_by_id.keys() {
        if !next_by_id.contains_key(id) {
            let sid = (*id).to_string();
            out.push(AppEvent::SessionEnded {
                session_id: sid.clone(),
            });
            for (bucket, _, _) in CONTEXT_BUCKETS {
                state.context_bucket_armed.remove(&(sid.clone(), *bucket));
            }
            state.tool_call_tail_len.remove(&sid);
            state.compaction_seen.remove(&sid);
            state.subagents_seen.remove(&sid);
        }
    }

    // StatusChanged: in both, status differs.
    for (id, next_view) in &next_by_id {
        if let Some(prev_view) = prev_by_id.get(id) {
            if prev_view.status != next_view.status {
                out.push(AppEvent::StatusChanged {
                    session_id: (*id).to_string(),
                    from: prev_view.status.clone(),
                    to: next_view.status.clone(),
                });
            }
        }
    }
}

fn diff_context_thresholds(
    state: &mut EventDifferState,
    _prev: &Snapshot,
    next: &Snapshot,
    out: &mut Vec<AppEvent>,
) {
    // Independent per-bucket latches. Climbing 60→96 emits three events
    // (70, 90, 95). Oscillating 88↔96 only affects the 95 latch.
    for session in &next.sessions {
        let pct = session.context_percent;
        for (bucket, fire_at, clear_below) in CONTEXT_BUCKETS {
            let key = (session.session_id.clone(), *bucket);
            let was_armed = state
                .context_bucket_armed
                .get(&key)
                .copied()
                .unwrap_or(false);
            if !was_armed && pct >= *fire_at {
                out.push(AppEvent::ContextThreshold {
                    session_id: session.session_id.clone(),
                    bucket: *bucket,
                });
                state.context_bucket_armed.insert(key, true);
            } else if was_armed && pct < *clear_below {
                state.context_bucket_armed.insert(key, false);
            }
        }
    }
}

fn diff_token_rate(state: &mut EventDifferState, next: &Snapshot, out: &mut Vec<AppEvent>) {
    let rate = next.token_rate;
    let baseline = state.token_rate_baseline.unwrap_or(rate);
    if let Some(b) = state.token_rate_baseline {
        // EMA: new = old + (rate - old) / halflife
        let updated = b + (rate - b) / TOKEN_RATE_BASELINE_HALFLIFE;
        state.token_rate_baseline = Some(updated);
    } else {
        state.token_rate_baseline = Some(rate);
    }
    if baseline <= 0.0 {
        return;
    }
    let ratio = rate / baseline;
    if !state.token_spike_armed && ratio >= SPIKE_FIRE_MULTIPLIER {
        out.push(AppEvent::TokenRateSpike { rate, baseline });
        state.token_spike_armed = true;
    } else if state.token_spike_armed && ratio < SPIKE_CLEAR_MULTIPLIER {
        state.token_spike_armed = false;
    }
}

fn diff_host_load(state: &mut EventDifferState, next: &Snapshot, out: &mut Vec<AppEvent>) {
    let Some(host) = next.host else {
        return;
    };
    let load1 = host.load1;
    if !state.host_load_armed && load1 >= HOST_LOAD_FIRE {
        out.push(AppEvent::HostLoadHigh { load1 });
        state.host_load_armed = true;
    } else if state.host_load_armed && load1 < HOST_LOAD_CLEAR {
        state.host_load_armed = false;
    }
}

fn diff_activity(
    state: &mut EventDifferState,
    _prev: &Snapshot,
    next: &Snapshot,
    out: &mut Vec<AppEvent>,
) {
    for session in &next.sessions {
        // Tool calls: emit ONE ToolCalled per net-new tail item since
        // the last tick. Snapshot's tail is bounded; we compare current
        // length to the last-seen length. If the tail shrank (session
        // hot-restart), reset baseline silently.
        let cur_len = session.tool_calls.len();
        let prev_len = state
            .tool_call_tail_len
            .get(&session.session_id)
            .copied()
            .unwrap_or(0);
        if cur_len > prev_len {
            let new_items_count = cur_len - prev_len;
            let new_items = &session.tool_calls[cur_len - new_items_count..];
            for item in new_items {
                out.push(AppEvent::ToolCalled {
                    session_id: session.session_id.clone(),
                    tool: item.name.clone(),
                    arg: item.arg.clone(),
                });
            }
        }
        state
            .tool_call_tail_len
            .insert(session.session_id.clone(), cur_len);

        // Compaction count growth.
        let cur_compaction = session.compaction_count;
        let prev_compaction = state
            .compaction_seen
            .get(&session.session_id)
            .copied()
            .unwrap_or(cur_compaction);
        if cur_compaction > prev_compaction {
            for _ in 0..(cur_compaction - prev_compaction) {
                out.push(AppEvent::CompactionDetected {
                    session_id: session.session_id.clone(),
                });
            }
        }
        state
            .compaction_seen
            .insert(session.session_id.clone(), cur_compaction);

        // Subagents: emit for any name not seen before for this session.
        let seen = state
            .subagents_seen
            .entry(session.session_id.clone())
            .or_default();
        for sa in &session.subagents {
            if seen.insert(sa.name.clone()) {
                // Only emit on first sighting; never re-emit.
                // First-tick baseline suppression is the caller's job
                // (App::emit_events skips diff on first tick).
                out.push(AppEvent::SubagentSpawned {
                    session_id: session.session_id.clone(),
                    name: sa.name.clone(),
                });
            }
        }
    }
}

/// A provider is "rate limited" when either window's percentage is at
/// or above [`RATE_LIMITED_PCT`].
const RATE_LIMITED_PCT: f64 = 90.0;

fn is_limited(rl: &crate::model::RateLimitInfo) -> bool {
    rl.five_hour_pct.unwrap_or(0.0) >= RATE_LIMITED_PCT
        || rl.seven_day_pct.unwrap_or(0.0) >= RATE_LIMITED_PCT
}

fn diff_rate_limits(
    state: &mut EventDifferState,
    _prev: &Snapshot,
    next: &Snapshot,
    out: &mut Vec<AppEvent>,
) {
    let mut seen: HashSet<String> = HashSet::new();
    for rl in &next.rate_limits {
        let provider = rl.source.clone();
        seen.insert(provider.clone());
        let now_limited = is_limited(rl);
        let was_armed = state
            .rate_limit_armed
            .get(&provider)
            .copied()
            .unwrap_or(false);
        if now_limited && !was_armed {
            let resets_at_ms = rl
                .five_hour_resets_at
                .or(rl.seven_day_resets_at)
                .map(|s| s.saturating_mul(1000));
            out.push(AppEvent::RateLimited {
                provider: provider.clone(),
                resets_at_ms,
            });
            state.rate_limit_armed.insert(provider, true);
        } else if !now_limited && was_armed {
            out.push(AppEvent::RateLimitCleared {
                provider: provider.clone(),
            });
            state.rate_limit_armed.insert(provider, false);
        }
    }
    // Providers that disappeared entirely are treated as "cleared".
    let to_clear: Vec<String> = state
        .rate_limit_armed
        .iter()
        .filter(|(_, &armed)| armed)
        .filter(|(k, _)| !seen.contains(*k))
        .map(|(k, _)| k.clone())
        .collect();
    for provider in to_clear {
        out.push(AppEvent::RateLimitCleared {
            provider: provider.clone(),
        });
        state.rate_limit_armed.insert(provider, false);
    }
}

fn diff_orphan_ports(prev: &Snapshot, next: &Snapshot, out: &mut Vec<AppEvent>) {
    let prev_ports: HashSet<u16> = prev.orphan_ports.iter().map(|p| p.port).collect();
    for op in &next.orphan_ports {
        if !prev_ports.contains(&op.port) {
            out.push(AppEvent::OrphanPortAppeared {
                port: op.port,
                prior_pid: op.pid,
            });
        }
    }
}

fn diff_mcp_servers(prev: &Snapshot, next: &Snapshot, out: &mut Vec<AppEvent>) {
    let prev_pids: HashSet<u32> = prev.mcp_servers.iter().map(|m| m.pid).collect();
    let next_pids: HashSet<u32> = next.mcp_servers.iter().map(|m| m.pid).collect();
    for m in &next.mcp_servers {
        if !prev_pids.contains(&m.pid) {
            out.push(AppEvent::McpServerAppeared {
                pid: m.pid,
                parent_cli: m.parent_cli.to_string(),
            });
        }
    }
    for pid in &prev_pids {
        if !next_pids.contains(pid) {
            out.push(AppEvent::McpServerVanished { pid: *pid });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_info::{AgentAggregate, HostMetrics};
    use crate::model::{OrphanPort, RateLimitInfo, SessionStatus};
    use crate::snapshot::{McpServerView, SessionView, Snapshot, ToolCallView};

    fn empty_snapshot() -> Snapshot {
        Snapshot {
            generated_at_ms: 0,
            host: None,
            aggregate: AgentAggregate::default(),
            token_rate: 0.0,
            interval_ms: 2000,
            sessions: Vec::new(),
            rate_limits: Vec::new(),
            orphan_ports: Vec::new(),
            mcp_servers: Vec::new(),
        }
    }

    fn session(id: &str, status: SessionStatus, pct: f64) -> SessionView {
        SessionView {
            agent_cli: "claude",
            pid: 1,
            session_id: id.to_string(),
            project_name: String::new(),
            cwd: String::new(),
            config_root: String::new(),
            status,
            model: String::new(),
            effort: String::new(),
            version: String::new(),
            context_percent: pct,
            context_window: 200_000,
            total_tokens: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            turn_count: 0,
            mem_mb: 0,
            git_branch: String::new(),
            git_added: 0,
            git_modified: 0,
            started_at_ms: 0,
            elapsed_secs: 0,
            summary: String::new(),
            current_task: None,
            children: Vec::new(),
            compaction_count: 0,
            token_history: Vec::new(),
            subagents: Vec::new(),
            tool_calls: Vec::new(),
            chat_messages: Vec::new(),
        }
    }

    #[test]
    fn empty_diff_is_empty() {
        let prev = empty_snapshot();
        let next = empty_snapshot();
        let mut state = EventDifferState::new();
        let events = diff(&mut state, &prev, &next, true, 0);
        assert!(events.is_empty());
    }

    #[test]
    fn session_started_when_id_appears() {
        let prev = empty_snapshot();
        let mut next = empty_snapshot();
        next.sessions
            .push(session("s1", SessionStatus::Thinking, 0.0));
        let mut state = EventDifferState::new();
        let events = diff(&mut state, &prev, &next, true, 0);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AppEvent::SessionStarted { session_id, .. } => assert_eq!(session_id, "s1"),
            ev => panic!("expected SessionStarted, got {ev:?}"),
        }
    }

    #[test]
    fn session_ended_when_id_disappears() {
        let mut prev = empty_snapshot();
        prev.sessions
            .push(session("s1", SessionStatus::Thinking, 0.0));
        let next = empty_snapshot();
        let mut state = EventDifferState::new();
        let events = diff(&mut state, &prev, &next, true, 0);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AppEvent::SessionEnded { .. }));
        // No `reason` field exists on v1.
        let json = serde_json::to_string(&events[0]).unwrap();
        assert!(!json.contains("reason"));
    }

    #[test]
    fn status_changed_when_status_differs() {
        let mut prev = empty_snapshot();
        prev.sessions
            .push(session("s1", SessionStatus::Thinking, 0.0));
        let mut next = empty_snapshot();
        next.sessions
            .push(session("s1", SessionStatus::Executing, 0.0));
        let mut state = EventDifferState::new();
        let events = diff(&mut state, &prev, &next, true, 0);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AppEvent::StatusChanged { from, to, .. } => {
                assert_eq!(*from, SessionStatus::Thinking);
                assert_eq!(*to, SessionStatus::Executing);
            }
            ev => panic!("expected StatusChanged, got {ev:?}"),
        }
    }

    #[test]
    fn context_bucket_ladder_climb_emits_three_events() {
        // 60 → 96 must emit three events (70, 90, 95) in one diff.
        let mut prev = empty_snapshot();
        prev.sessions
            .push(session("s1", SessionStatus::Thinking, 60.0));
        let mut next = empty_snapshot();
        next.sessions
            .push(session("s1", SessionStatus::Thinking, 96.0));
        let mut state = EventDifferState::new();
        // Prime baseline so the 60% prev doesn't count.
        state.context_bucket_armed.insert(("s1".into(), 70), false);
        state.context_bucket_armed.insert(("s1".into(), 90), false);
        state.context_bucket_armed.insert(("s1".into(), 95), false);
        let events = diff(&mut state, &prev, &next, false, 0);
        let buckets: Vec<u8> = events
            .iter()
            .filter_map(|e| match e {
                AppEvent::ContextThreshold { bucket, .. } => Some(*bucket),
                _ => None,
            })
            .collect();
        assert_eq!(buckets, vec![70, 90, 95]);
    }

    #[test]
    fn context_bucket_does_not_refire_without_clearing() {
        // Climbing to 96 once, then staying at 92 (above clear for 95
        // at 90, above clear for 90 at 85), emits nothing on tick 2.
        let mut state = EventDifferState::new();
        let prev_a = {
            let mut s = empty_snapshot();
            s.sessions
                .push(session("s1", SessionStatus::Thinking, 60.0));
            s
        };
        let next_a = {
            let mut s = empty_snapshot();
            s.sessions
                .push(session("s1", SessionStatus::Thinking, 96.0));
            s
        };
        let _e1 = diff(&mut state, &prev_a, &next_a, false, 0);
        let next_b = {
            let mut s = empty_snapshot();
            s.sessions
                .push(session("s1", SessionStatus::Thinking, 92.0));
            s
        };
        let e2 = diff(&mut state, &next_a, &next_b, false, 0);
        let ctxs: Vec<&AppEvent> = e2
            .iter()
            .filter(|e| matches!(e, AppEvent::ContextThreshold { .. }))
            .collect();
        assert!(ctxs.is_empty(), "expected no re-fire, got {ctxs:?}");
    }

    #[test]
    fn context_bucket_rearms_after_clear() {
        // 80 (fires 70) → 60 (clears 70) → 80 (re-fires 70).
        let mut state = EventDifferState::new();
        let s_60 = {
            let mut s = empty_snapshot();
            s.sessions
                .push(session("s1", SessionStatus::Thinking, 60.0));
            s
        };
        let s_80 = {
            let mut s = empty_snapshot();
            s.sessions
                .push(session("s1", SessionStatus::Thinking, 80.0));
            s
        };
        let e1 = diff(&mut state, &s_60, &s_80, false, 0);
        assert_eq!(e1.len(), 1);
        let e2 = diff(&mut state, &s_80, &s_60, false, 0);
        // Going down doesn't emit context events.
        assert!(e2
            .iter()
            .all(|e| !matches!(e, AppEvent::ContextThreshold { .. })));
        let e3 = diff(&mut state, &s_60, &s_80, false, 0);
        assert_eq!(e3.len(), 1);
        assert!(matches!(
            e3[0],
            AppEvent::ContextThreshold { bucket: 70, .. }
        ));
    }

    #[test]
    fn context_buckets_are_independent() {
        // Oscillating 88 ↔ 96 should only flap the 95 latch, never
        // touching the 70 or 90 buckets after the initial climb.
        let mut state = EventDifferState::new();
        let s_60 = {
            let mut s = empty_snapshot();
            s.sessions
                .push(session("s1", SessionStatus::Thinking, 60.0));
            s
        };
        let s_88 = {
            let mut s = empty_snapshot();
            s.sessions
                .push(session("s1", SessionStatus::Thinking, 88.0));
            s
        };
        let s_96 = {
            let mut s = empty_snapshot();
            s.sessions
                .push(session("s1", SessionStatus::Thinking, 96.0));
            s
        };
        // 60 → 88: emits 70 only (88 < 90, so 90/95 don't fire).
        let e1 = diff(&mut state, &s_60, &s_88, false, 0);
        let buckets1: Vec<u8> = e1
            .iter()
            .filter_map(|e| match e {
                AppEvent::ContextThreshold { bucket, .. } => Some(*bucket),
                _ => None,
            })
            .collect();
        assert_eq!(buckets1, vec![70]);
        // 88 → 96: emits 90 and 95.
        let e2 = diff(&mut state, &s_88, &s_96, false, 0);
        let buckets2: Vec<u8> = e2
            .iter()
            .filter_map(|e| match e {
                AppEvent::ContextThreshold { bucket, .. } => Some(*bucket),
                _ => None,
            })
            .collect();
        assert_eq!(buckets2, vec![90, 95]);
        // 96 → 88: clears 95 (88 < 90), keeps 70 and 90 armed (88 > 85, > 65).
        let _ = diff(&mut state, &s_96, &s_88, false, 0);
        // 88 → 96: re-fires 95 only.
        let e4 = diff(&mut state, &s_88, &s_96, false, 0);
        let buckets4: Vec<u8> = e4
            .iter()
            .filter_map(|e| match e {
                AppEvent::ContextThreshold { bucket, .. } => Some(*bucket),
                _ => None,
            })
            .collect();
        assert_eq!(buckets4, vec![95]);
    }

    #[test]
    fn slow_tier_suppressed_when_slow_tick_did_not_run() {
        let mut prev = empty_snapshot();
        let mut next = empty_snapshot();
        next.rate_limits.push(RateLimitInfo {
            source: "claude".into(),
            five_hour_pct: Some(95.0),
            ..Default::default()
        });
        let mut state = EventDifferState::new();
        let events_fast = diff(&mut state, &prev, &next, false, 0);
        assert!(
            events_fast.is_empty(),
            "slow tier must not emit on fast tick"
        );

        prev.rate_limits.push(RateLimitInfo {
            source: "claude".into(),
            five_hour_pct: Some(0.0),
            ..Default::default()
        });
        let mut state2 = EventDifferState::new();
        let events_slow = diff(&mut state2, &prev, &next, true, 0);
        assert!(matches!(events_slow[0], AppEvent::RateLimited { .. }));
    }

    #[test]
    fn rate_limited_fires_once_and_clears() {
        let mut state = EventDifferState::new();
        let limited = {
            let mut s = empty_snapshot();
            s.rate_limits.push(RateLimitInfo {
                source: "claude".into(),
                five_hour_pct: Some(95.0),
                five_hour_resets_at: Some(1_700_000_000),
                ..Default::default()
            });
            s
        };
        let unlimited = {
            let mut s = empty_snapshot();
            s.rate_limits.push(RateLimitInfo {
                source: "claude".into(),
                five_hour_pct: Some(40.0),
                ..Default::default()
            });
            s
        };
        let e1 = diff(&mut state, &empty_snapshot(), &limited, true, 0);
        let limited_count = e1
            .iter()
            .filter(|e| matches!(e, AppEvent::RateLimited { .. }))
            .count();
        assert_eq!(limited_count, 1);
        match e1
            .iter()
            .find_map(|e| match e {
                AppEvent::RateLimited { resets_at_ms, .. } => Some(*resets_at_ms),
                _ => None,
            })
            .unwrap()
        {
            Some(ms) => assert_eq!(ms, 1_700_000_000 * 1000),
            None => panic!("expected resets_at_ms to be Some"),
        }
        // Re-running with still-limited state should NOT re-fire.
        let e2 = diff(&mut state, &limited, &limited, true, 0);
        assert!(e2
            .iter()
            .all(|e| !matches!(e, AppEvent::RateLimited { .. })));
        let e3 = diff(&mut state, &limited, &unlimited, true, 0);
        assert!(e3
            .iter()
            .any(|e| matches!(e, AppEvent::RateLimitCleared { .. })));
    }

    #[test]
    fn orphan_port_appeared() {
        let prev = empty_snapshot();
        let mut next = empty_snapshot();
        next.orphan_ports.push(OrphanPort {
            port: 8080,
            pid: 99,
            command: "x".into(),
            project_name: "p".into(),
        });
        let mut state = EventDifferState::new();
        let on_slow = diff(&mut state, &prev, &next, true, 0);
        assert!(on_slow
            .iter()
            .any(|e| matches!(e, AppEvent::OrphanPortAppeared { .. })));
        let mut state2 = EventDifferState::new();
        let on_fast = diff(&mut state2, &prev, &next, false, 0);
        assert!(on_fast
            .iter()
            .all(|e| !matches!(e, AppEvent::OrphanPortAppeared { .. })));
    }

    #[test]
    fn mcp_server_appear_and_vanish() {
        let mut a = empty_snapshot();
        a.mcp_servers.push(McpServerView {
            pid: 7,
            parent_cli: "codex",
            profile: None,
            mem_kb: 0,
            active_count: 0,
            rollout_count: 0,
            last_activity_ms: None,
        });
        let b = empty_snapshot();
        let mut state = EventDifferState::new();
        // First diff: empty -> a, appears.
        let e1 = diff(&mut state, &b, &a, true, 0);
        assert!(e1
            .iter()
            .any(|e| matches!(e, AppEvent::McpServerAppeared { .. })));
        // Second diff: a -> empty, vanishes.
        let e2 = diff(&mut state, &a, &b, true, 0);
        assert!(e2
            .iter()
            .any(|e| matches!(e, AppEvent::McpServerVanished { pid: 7 })));
    }

    #[test]
    fn host_load_high_latches() {
        let mut state = EventDifferState::new();
        let mk = |load1: f64| {
            let mut s = empty_snapshot();
            s.host = Some(HostMetrics {
                cpu_pct: 0.0,
                mem_pct: 0.0,
                load1,
            });
            s
        };
        // Below threshold: no event.
        let _ = diff(&mut state, &mk(0.5), &mk(2.0), false, 0);
        // Crosses: fires.
        let e1 = diff(&mut state, &mk(2.0), &mk(4.5), false, 0);
        assert!(e1
            .iter()
            .any(|e| matches!(e, AppEvent::HostLoadHigh { .. })));
        // Stays high: no re-fire.
        let e2 = diff(&mut state, &mk(4.5), &mk(4.7), false, 0);
        assert!(e2
            .iter()
            .all(|e| !matches!(e, AppEvent::HostLoadHigh { .. })));
        // Drops below clear: silently clears (no event for clearing).
        let _ = diff(&mut state, &mk(4.7), &mk(2.0), false, 0);
        // Re-crosses: fires again.
        let e4 = diff(&mut state, &mk(2.0), &mk(4.5), false, 0);
        assert!(e4
            .iter()
            .any(|e| matches!(e, AppEvent::HostLoadHigh { .. })));
    }

    #[test]
    fn tool_called_emits_for_new_tail_items() {
        let mut state = EventDifferState::new();
        let mut prev = empty_snapshot();
        prev.sessions
            .push(session("s1", SessionStatus::Thinking, 0.0));
        let mut next = empty_snapshot();
        let mut s = session("s1", SessionStatus::Thinking, 0.0);
        s.tool_calls.push(ToolCallView {
            name: "Read".into(),
            arg: "/foo".into(),
            duration_ms: 0,
        });
        next.sessions.push(s);
        // First sighting: tail grew from 0 to 1, expect one event.
        let e1 = diff(&mut state, &prev, &next, false, 0);
        assert_eq!(
            e1.iter()
                .filter(|e| matches!(e, AppEvent::ToolCalled { .. }))
                .count(),
            1
        );
        // Same tail: no event.
        let e2 = diff(&mut state, &next, &next, false, 0);
        assert_eq!(
            e2.iter()
                .filter(|e| matches!(e, AppEvent::ToolCalled { .. }))
                .count(),
            0
        );
    }

    #[test]
    fn compaction_detected_on_increment() {
        let mut state = EventDifferState::new();
        let mut prev = empty_snapshot();
        prev.sessions
            .push(session("s1", SessionStatus::Thinking, 0.0));
        let mut next = empty_snapshot();
        let mut s = session("s1", SessionStatus::Thinking, 0.0);
        s.compaction_count = 1;
        next.sessions.push(s);
        // Prime state at 0
        diff(&mut state, &prev, &prev, false, 0);
        let e1 = diff(&mut state, &prev, &next, false, 0);
        assert_eq!(
            e1.iter()
                .filter(|e| matches!(e, AppEvent::CompactionDetected { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn session_ended_purges_detector_state() {
        let mut state = EventDifferState::new();
        let mut s_present = empty_snapshot();
        s_present
            .sessions
            .push(session("s1", SessionStatus::Thinking, 80.0));
        let s_absent = empty_snapshot();

        // Arm bucket 70.
        let _ = diff(&mut state, &empty_snapshot(), &s_present, false, 0);
        assert!(state
            .context_bucket_armed
            .get(&("s1".into(), 70))
            .copied()
            .unwrap_or(false));

        // Session vanishes -> state purged.
        let _ = diff(&mut state, &s_present, &s_absent, true, 0);
        assert!(
            state.context_bucket_armed.get(&("s1".into(), 70)).is_none(),
            "bucket state should be purged when session ends"
        );
    }
}
