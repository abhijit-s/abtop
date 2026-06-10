//! `AppEvent` — the public wire contract for the abtop pub-sub event system.
//!
//! Variants serialize via `#[serde(tag = "type")]` (internally tagged), so each
//! NDJSON record carries a `type` discriminator alongside the event-specific
//! fields. Every record is wrapped in [`WireRecord`] which adds a schema
//! version (`v`) and a millisecond timestamp (`ts_ms`).
//!
//! Wire format (NDJSON, one record per line):
//!
//! ```json
//! {"v":1,"ts_ms":1718039040000,"type":"StatusChanged","session_id":"abc","from":"Idle","to":"Working"}
//! ```
//!
//! v1 design note: `SessionEnded` carries only `session_id`. Exit-code /
//! crash-vs-disappear attribution is deferred — see the resolved-decisions
//! note in the implementation plan.

use crate::model::SessionStatus;
use serde::{Deserialize, Serialize};

/// One observable state transition surfaced by `events::diff`. The
/// serialized form is the public wire contract; field renames are
/// breaking changes that must bump `WireRecord::v`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AppEvent {
    /// A new `session_id` appeared in the snapshot.
    SessionStarted {
        session_id: String,
        agent_cli: String,
        pid: u32,
    },
    /// A `session_id` present in the previous snapshot is gone.
    /// v1 carries no `reason` — that attribution is deferred.
    SessionEnded { session_id: String },
    /// A session's coarse activity state changed.
    StatusChanged {
        session_id: String,
        from: SessionStatus,
        to: SessionStatus,
    },
    /// A session's context fill crossed a configured bucket (70/90/95)
    /// upward. Each bucket is an independent latch with hysteresis —
    /// see `EventDifferState` for the clear thresholds.
    ContextThreshold { session_id: String, bucket: u8 },
    /// The aggregate token rate spiked above the rolling baseline.
    TokenRateSpike { rate: f64, baseline: f64 },
    /// A rate-limit provider entered a limited state.
    RateLimited {
        provider: String,
        resets_at_ms: Option<u64>,
    },
    /// A previously-limited provider is no longer limited.
    RateLimitCleared { provider: String },
    /// A port appeared in `orphan_ports` that was not present previously.
    OrphanPortAppeared { port: u16, prior_pid: u32 },
    /// A new MCP server PID was detected.
    McpServerAppeared { pid: u32, parent_cli: String },
    /// A previously-detected MCP server PID is gone.
    McpServerVanished { pid: u32 },
    /// A new tool call was appended to a session's tool-call tail.
    ToolCalled {
        session_id: String,
        tool: String,
        arg: String,
    },
    /// A session's compaction count incremented.
    CompactionDetected { session_id: String },
    /// A new subagent appeared on a session.
    SubagentSpawned { session_id: String, name: String },
    /// Host 1-minute load crossed the configured high threshold.
    HostLoadHigh { load1: f64 },
}

/// Latency tier. Fast events are emitted every tick; slow events only
/// when the slow collector branch ran on the current tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EventTier {
    Fast,
    Slow,
}

/// Coarse domain grouping for rule matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    Lifecycle,
    Status,
    Threshold,
    Quota,
    Resource,
    Activity,
    Host,
}

impl AppEvent {
    pub fn tier(&self) -> EventTier {
        match self {
            AppEvent::SessionStarted { .. }
            | AppEvent::SessionEnded { .. }
            | AppEvent::StatusChanged { .. }
            | AppEvent::ContextThreshold { .. }
            | AppEvent::TokenRateSpike { .. }
            | AppEvent::ToolCalled { .. }
            | AppEvent::CompactionDetected { .. }
            | AppEvent::SubagentSpawned { .. }
            | AppEvent::HostLoadHigh { .. } => EventTier::Fast,
            AppEvent::RateLimited { .. }
            | AppEvent::RateLimitCleared { .. }
            | AppEvent::OrphanPortAppeared { .. }
            | AppEvent::McpServerAppeared { .. }
            | AppEvent::McpServerVanished { .. } => EventTier::Slow,
        }
    }

    pub fn kind(&self) -> EventKind {
        match self {
            AppEvent::SessionStarted { .. } | AppEvent::SessionEnded { .. } => {
                EventKind::Lifecycle
            }
            AppEvent::StatusChanged { .. } => EventKind::Status,
            AppEvent::ContextThreshold { .. } | AppEvent::TokenRateSpike { .. } => {
                EventKind::Threshold
            }
            AppEvent::RateLimited { .. } | AppEvent::RateLimitCleared { .. } => EventKind::Quota,
            AppEvent::OrphanPortAppeared { .. }
            | AppEvent::McpServerAppeared { .. }
            | AppEvent::McpServerVanished { .. } => EventKind::Resource,
            AppEvent::ToolCalled { .. }
            | AppEvent::CompactionDetected { .. }
            | AppEvent::SubagentSpawned { .. } => EventKind::Activity,
            AppEvent::HostLoadHigh { .. } => EventKind::Host,
        }
    }

    /// Discriminator string matching the serde tag. Useful for rule
    /// matching by `on = ["StatusChanged", ...]` lists.
    pub fn type_name(&self) -> &'static str {
        match self {
            AppEvent::SessionStarted { .. } => "SessionStarted",
            AppEvent::SessionEnded { .. } => "SessionEnded",
            AppEvent::StatusChanged { .. } => "StatusChanged",
            AppEvent::ContextThreshold { .. } => "ContextThreshold",
            AppEvent::TokenRateSpike { .. } => "TokenRateSpike",
            AppEvent::RateLimited { .. } => "RateLimited",
            AppEvent::RateLimitCleared { .. } => "RateLimitCleared",
            AppEvent::OrphanPortAppeared { .. } => "OrphanPortAppeared",
            AppEvent::McpServerAppeared { .. } => "McpServerAppeared",
            AppEvent::McpServerVanished { .. } => "McpServerVanished",
            AppEvent::ToolCalled { .. } => "ToolCalled",
            AppEvent::CompactionDetected { .. } => "CompactionDetected",
            AppEvent::SubagentSpawned { .. } => "SubagentSpawned",
            AppEvent::HostLoadHigh { .. } => "HostLoadHigh",
        }
    }
}

/// Current wire-schema version. Bump on breaking changes (field
/// renames, removals, type changes). Additive fields do NOT bump.
pub const WIRE_VERSION: u8 = 1;

/// One NDJSON record: `{v, ts_ms, ...event_fields}`.
///
/// `#[serde(flatten)]` on the event places the variant's discriminator
/// and fields at the top level of the JSON object alongside `v` and
/// `ts_ms`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireRecord {
    pub v: u8,
    pub ts_ms: u64,
    #[serde(flatten)]
    pub event: AppEvent,
}

impl WireRecord {
    pub fn new(event: AppEvent, ts_ms: u64) -> Self {
        Self {
            v: WIRE_VERSION,
            ts_ms,
            event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_session_started() {
        let ev = AppEvent::SessionStarted {
            session_id: "abc".to_string(),
            agent_cli: "claude".to_string(),
            pid: 1234,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"SessionStarted\""));
        assert!(json.contains("\"session_id\":\"abc\""));
        let back: AppEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn round_trip_session_ended_has_no_reason() {
        let ev = AppEvent::SessionEnded {
            session_id: "abc".to_string(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(!json.contains("reason"));
        let back: AppEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn round_trip_all_variants() {
        let events = vec![
            AppEvent::SessionStarted {
                session_id: "s".into(),
                agent_cli: "claude".into(),
                pid: 1,
            },
            AppEvent::SessionEnded {
                session_id: "s".into(),
            },
            AppEvent::StatusChanged {
                session_id: "s".into(),
                from: SessionStatus::Thinking,
                to: SessionStatus::Executing,
            },
            AppEvent::ContextThreshold {
                session_id: "s".into(),
                bucket: 90,
            },
            AppEvent::TokenRateSpike {
                rate: 100.0,
                baseline: 30.0,
            },
            AppEvent::RateLimited {
                provider: "claude".into(),
                resets_at_ms: Some(42),
            },
            AppEvent::RateLimitCleared {
                provider: "claude".into(),
            },
            AppEvent::OrphanPortAppeared {
                port: 8080,
                prior_pid: 99,
            },
            AppEvent::McpServerAppeared {
                pid: 7,
                parent_cli: "codex".into(),
            },
            AppEvent::McpServerVanished { pid: 7 },
            AppEvent::ToolCalled {
                session_id: "s".into(),
                tool: "Read".into(),
                arg: "/foo".into(),
            },
            AppEvent::CompactionDetected {
                session_id: "s".into(),
            },
            AppEvent::SubagentSpawned {
                session_id: "s".into(),
                name: "child".into(),
            },
            AppEvent::HostLoadHigh { load1: 4.5 },
        ];
        for ev in events {
            let json = serde_json::to_string(&ev).unwrap();
            let back: AppEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(ev, back, "round trip failed for {}", ev.type_name());
        }
    }

    #[test]
    fn tier_assignments() {
        assert_eq!(
            AppEvent::StatusChanged {
                session_id: "s".into(),
                from: SessionStatus::Thinking,
                to: SessionStatus::Executing,
            }
            .tier(),
            EventTier::Fast
        );
        assert_eq!(
            AppEvent::RateLimited {
                provider: "claude".into(),
                resets_at_ms: None,
            }
            .tier(),
            EventTier::Slow
        );
        assert_eq!(
            AppEvent::OrphanPortAppeared {
                port: 80,
                prior_pid: 1,
            }
            .tier(),
            EventTier::Slow
        );
        assert_eq!(
            AppEvent::HostLoadHigh { load1: 4.0 }.tier(),
            EventTier::Fast
        );
    }

    #[test]
    fn kind_assignments() {
        assert_eq!(
            AppEvent::SessionStarted {
                session_id: "s".into(),
                agent_cli: "claude".into(),
                pid: 1,
            }
            .kind(),
            EventKind::Lifecycle
        );
        assert_eq!(
            AppEvent::ContextThreshold {
                session_id: "s".into(),
                bucket: 70,
            }
            .kind(),
            EventKind::Threshold
        );
        assert_eq!(
            AppEvent::RateLimited {
                provider: "p".into(),
                resets_at_ms: None,
            }
            .kind(),
            EventKind::Quota
        );
        assert_eq!(
            AppEvent::HostLoadHigh { load1: 4.0 }.kind(),
            EventKind::Host
        );
    }

    #[test]
    fn wire_record_flattens_event_fields() {
        let ev = AppEvent::StatusChanged {
            session_id: "s".into(),
            from: SessionStatus::Thinking,
            to: SessionStatus::Executing,
        };
        let rec = WireRecord::new(ev.clone(), 1_700_000_000_000);
        let json = serde_json::to_string(&rec).unwrap();
        // Top-level fields v, ts_ms, type, session_id, from, to.
        assert!(json.starts_with("{\"v\":1,\"ts_ms\":1700000000000"));
        assert!(json.contains("\"type\":\"StatusChanged\""));
        assert!(json.contains("\"session_id\":\"s\""));
        // Round trip.
        let back: WireRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.v, WIRE_VERSION);
        assert_eq!(back.ts_ms, 1_700_000_000_000);
        assert_eq!(back.event, ev);
    }

    #[test]
    fn type_name_matches_serde_tag() {
        let ev = AppEvent::HostLoadHigh { load1: 5.0 };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(&format!("\"type\":\"{}\"", ev.type_name())));
    }
}
