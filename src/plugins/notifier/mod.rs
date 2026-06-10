//! Notifier reference plugin.
//!
//! Connects to the abtop events Unix socket like any external consumer,
//! parses NDJSON [`crate::events::WireRecord`]s, matches them against
//! user-defined [`rules::Rule`]s, and dispatches desktop notifications
//! via a [`backend::Backend`].
//!
//! The worker loop is fully cooperative with the shared [`PluginCtx`]:
//!
//! - `shutdown` => return (and the registry joins us cleanly)
//! - `enabled = false` => discard inbound lines (pause semantics, mirrors
//!   the publisher's own pause flag — useful when a user toggles the
//!   plugin off at runtime)
//!
//! On socket failure we reconnect with exponential backoff (200ms → 5s
//! cap) and reset on success. This is the consumer side of the same
//! contract the publisher describes in
//! [`crate::events::publisher`].

pub mod backend;
pub mod rules;
pub mod template;

use crate::events::WireRecord;
use crate::plugins::{Plugin, PluginCtx, PluginHandle};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use backend::Backend;
use rules::{compile, event_key_hash, matches, CompiledRule, Debouncer};

const DEFAULT_DEBOUNCE_MS: u64 = 5_000;

/// Shared, mutable handle to a [`NotifierConfig`]. The notifier worker
/// holds this and re-reads it under a short read-lock per line so the
/// config loader can hot-swap rules without restarting the plugin.
pub type SharedNotifierConfig = Arc<RwLock<NotifierConfig>>;

fn default_debounce() -> u64 {
    DEFAULT_DEBOUNCE_MS
}

/// Configuration for the notifier plugin. Deserializes from the
/// `[plugins.notifier]` TOML table (the loader lands in U7).
#[derive(Clone, Debug, Deserialize)]
pub struct NotifierConfig {
    /// Start the worker at process start. CLI override:
    /// `--plugin-notify` / `--no-plugin-notify`. The `enabled` TOML
    /// alias matches the spec wording for `[plugins.notifier]`.
    #[serde(default, alias = "enabled")]
    pub enabled_at_startup: bool,
    /// Preferred backend, or `None` for auto. `"auto"` in TOML maps to
    /// `None` (see [`backend_deser`]).
    #[serde(default, deserialize_with = "backend_deser")]
    pub backend: Option<Backend>,
    /// Default debounce window in milliseconds. Per-rule overrides
    /// take precedence.
    #[serde(default = "default_debounce")]
    pub debounce_ms: u64,
    /// User-defined matching rules.
    #[serde(default)]
    pub rule: Vec<rules::Rule>,
    /// Monotonic generation counter — bumped by the config loader each
    /// time the rule list, debounce window, or backend changes so the
    /// worker can re-compile its rules without diffing every field.
    /// Skipped by serde so it never appears in user TOML.
    #[serde(skip)]
    pub generation: u64,
}

impl Default for NotifierConfig {
    fn default() -> Self {
        Self {
            enabled_at_startup: false,
            backend: None,
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            rule: Vec::new(),
            generation: 0,
        }
    }
}

/// Custom backend deserializer so `"auto"` → `None` and the named
/// variants flow through [`Backend`]'s own `Deserialize`.
fn backend_deser<'de, D>(de: D) -> Result<Option<Backend>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // The untagged enum tries variants in declaration order — listing
    // `Backend(Backend)` first means the named variants ("stderr",
    // "osascript", …) succeed via Backend's own Deserialize and we
    // only fall through to the string-catch-all for `"auto"` or any
    // unrecognized name.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Backend(Backend),
        Auto(String),
    }
    let r = Option::<Repr>::deserialize(de)?;
    Ok(match r {
        None => None,
        Some(Repr::Backend(b)) => Some(b),
        Some(Repr::Auto(s)) if s == "auto" => None,
        Some(Repr::Auto(s)) => {
            return Err(serde::de::Error::custom(format!(
                "unknown backend '{s}' — valid: osascript, notify-send, terminal-notifier, stderr, auto"
            )));
        }
    })
}

/// The plugin type that's registered with the [`crate::plugins::PluginRegistry`].
///
/// Holds a [`SharedNotifierConfig`] handle rather than an owned config
/// so the U7 config loader can mutate rules / debounce / backend at
/// runtime without restarting the worker thread.
pub struct Notifier {
    pub config: SharedNotifierConfig,
}

/// Metadata for `--list-plugins` rendering. The example config block
/// must round-trip through [`NotifierConfig`]'s `Deserialize` — see the
/// `example_config_round_trips` unit test below.
pub fn info() -> crate::plugins::PluginInfo {
    crate::plugins::PluginInfo {
        name: "notifier",
        feature: "plugin-notifier",
        default_on: true,
        startup_enabled: false,
        description: "Dispatches desktop notifications when published events match \
                      user-defined rules. Backends: osascript, notify-send, \
                      terminal-notifier, or stderr fallback.",
        startup: "disabled by default. Enable with --plugin-notify or set \
                  `enabled = true` under [plugins.notifier] in \
                  ~/.config/abtop/config.toml.",
        example_config: r#"[plugins.notifier]
enabled = true
backend = "auto"          # auto | osascript | notify-send | terminal-notifier | stderr
debounce_ms = 5000

[[plugins.notifier.rule]]
on    = ["RateLimited"]
title = "abtop: rate limited"
body  = "{provider}: {detail}"
"#,
        docs_pointer: "AGENTS.md -> \"Live event stream\" -> Notifier",
    }
}

impl Notifier {
    /// Construct from an owned config (the common case — wraps in an
    /// `Arc<RwLock<_>>`).
    pub fn new(config: NotifierConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
        }
    }

    /// Construct from an existing shared handle. The hot-reload path
    /// uses this so the registry and the loader share the same
    /// `Arc<RwLock<_>>`.
    pub fn from_shared(config: SharedNotifierConfig) -> Self {
        Self { config }
    }

    /// Borrow the shared config handle (used by `App::apply_event_config`
    /// to mutate rules in place).
    pub fn shared(&self) -> SharedNotifierConfig {
        Arc::clone(&self.config)
    }
}

impl Plugin for Notifier {
    fn name(&self) -> &'static str {
        "notifier"
    }

    fn start(&self, ctx: PluginCtx) -> PluginHandle {
        let shared = Arc::clone(&self.config);
        let enabled_for_handle = Arc::clone(&ctx.enabled);
        let join = std::thread::Builder::new()
            .name("abtop-notifier".into())
            .spawn(move || {
                run(ctx, shared);
            })
            .expect("spawn abtop-notifier thread");
        PluginHandle {
            name: "notifier",
            enabled: enabled_for_handle,
            join,
        }
    }
}

/// Worker entry point. Loops over (connect → read → dispatch) until
/// shutdown. Errors at any layer demote to "drop the stream and back
/// off"; the only exit path is `ctx.shutdown`.
///
/// The worker re-compiles rules whenever the shared config's
/// `generation` counter advances — the config loader bumps this when
/// rules, debounce, or backend change at runtime.
fn run(ctx: PluginCtx, shared: SharedNotifierConfig) {
    // Snapshot the initial config under the read lock so we don't
    // hold the lock across `Backend::probe` (which may shell out).
    let (initial_backend_pref, initial_rules, initial_debounce_ms, mut last_generation) = {
        let cfg = shared.read().expect("notifier config lock poisoned");
        (
            cfg.backend.clone(),
            cfg.rule.clone(),
            cfg.debounce_ms,
            cfg.generation,
        )
    };
    let mut backend = Backend::probe(initial_backend_pref);
    let mut compiled = compile(initial_rules);
    let mut default_debounce_ms = initial_debounce_ms;
    for r in compiled.iter().filter(|r| r.disabled) {
        eprintln!(
            "notifier: rule #{} disabled — invalid `when`: {}",
            r.idx,
            r.disable_reason.as_deref().unwrap_or("(no reason)")
        );
    }
    let mut debouncer = Debouncer::new(default_debounce_ms);
    let mut backoff = Duration::from_millis(200);
    let backoff_cap = Duration::from_secs(5);

    while !ctx.shutdown.load(Ordering::Relaxed) {
        let stream = match connect_with_shutdown(&ctx, &backoff) {
            Some(s) => s,
            None => return, // shutdown observed during backoff
        };
        // Successful connect — reset backoff.
        backoff = Duration::from_millis(200);

        process_stream(
            stream,
            &ctx,
            &shared,
            &mut backend,
            &mut compiled,
            &mut default_debounce_ms,
            &mut last_generation,
            &mut debouncer,
        );

        // Stream ended (EOF, error, or shutdown). If shutdown, the
        // outer while will exit; otherwise we reconnect.
        if ctx.shutdown.load(Ordering::Relaxed) {
            return;
        }

        // Brief backoff before reconnecting after EOF.
        sleep_with_shutdown(&ctx, Duration::from_millis(200));
        // Grow backoff in case the next connect also fails — capped.
        backoff = (backoff * 2).min(backoff_cap);
    }
}

/// Re-read the shared config if its generation counter advanced. Cheap
/// when nothing changed (a single read-lock + u64 compare). Updates
/// `compiled`, `default_debounce_ms`, and `backend` in place when a
/// new generation is observed.
fn refresh_if_changed(
    shared: &SharedNotifierConfig,
    backend: &mut Backend,
    compiled: &mut Vec<CompiledRule>,
    default_debounce_ms: &mut u64,
    last_generation: &mut u64,
) {
    let (new_backend_pref, new_rules, new_debounce_ms, new_gen) = {
        let cfg = match shared.read() {
            Ok(c) => c,
            Err(_) => return, // lock poisoned — skip; next read may recover
        };
        if cfg.generation == *last_generation {
            return;
        }
        (
            cfg.backend.clone(),
            cfg.rule.clone(),
            cfg.debounce_ms,
            cfg.generation,
        )
    };
    *compiled = compile(new_rules);
    *default_debounce_ms = new_debounce_ms;
    *backend = Backend::probe(new_backend_pref);
    *last_generation = new_gen;
}

#[cfg(unix)]
fn connect_with_shutdown(
    ctx: &PluginCtx,
    initial_backoff: &Duration,
) -> Option<std::os::unix::net::UnixStream> {
    use std::os::unix::net::UnixStream;
    let mut backoff = *initial_backoff;
    let cap = Duration::from_secs(5);
    loop {
        if ctx.shutdown.load(Ordering::Relaxed) {
            return None;
        }
        match UnixStream::connect(&ctx.socket_path) {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
                return Some(s);
            }
            Err(_) => {
                sleep_with_shutdown(ctx, backoff);
                backoff = (backoff * 2).min(cap);
            }
        }
    }
}

#[cfg(not(unix))]
fn connect_with_shutdown(_ctx: &PluginCtx, _initial: &Duration) -> Option<std::fs::File> {
    // Non-unix platforms have no UDS — the plugin is a no-op. We
    // never publish on these platforms either.
    None
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn process_stream(
    stream: std::os::unix::net::UnixStream,
    ctx: &PluginCtx,
    shared: &SharedNotifierConfig,
    backend: &mut Backend,
    rules: &mut Vec<CompiledRule>,
    default_debounce_ms: &mut u64,
    last_generation: &mut u64,
    debouncer: &mut Debouncer,
) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        if ctx.shutdown.load(Ordering::Relaxed) {
            return;
        }
        let line = match line {
            Ok(l) => l,
            // ErrorKind::WouldBlock surfaces as the read_timeout we set;
            // treat it like "no data yet, keep looping" by continuing.
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                refresh_if_changed(shared, backend, rules, default_debounce_ms, last_generation);
                continue;
            }
            Err(_) => return, // hangup; the outer loop reconnects
        };
        // Cheap hot-reload check before dispatching.
        refresh_if_changed(shared, backend, rules, default_debounce_ms, last_generation);
        if line.is_empty() {
            continue;
        }
        if !ctx.enabled.load(Ordering::Relaxed) {
            continue; // pause: discard while paused
        }
        handle_line(&line, backend, rules, debouncer, *default_debounce_ms);
    }
}

#[cfg(not(unix))]
#[allow(clippy::too_many_arguments)]
fn process_stream(
    _stream: std::fs::File,
    _ctx: &PluginCtx,
    _shared: &SharedNotifierConfig,
    _backend: &mut Backend,
    _rules: &mut Vec<CompiledRule>,
    _default_debounce_ms: &mut u64,
    _last_generation: &mut u64,
    _debouncer: &mut Debouncer,
) {
}

/// Parse a single NDJSON line and dispatch matching rules. `_meta`
/// drop markers (which omit the `type` field) are silently skipped.
fn handle_line(
    line: &str,
    backend: &Backend,
    rules: &[CompiledRule],
    debouncer: &mut Debouncer,
    default_debounce_ms: u64,
) {
    // Cheap reject for meta lines: they carry `_meta` but no `type`.
    // The full WireRecord deserialize will fail on them via serde's
    // missing-discriminator error, but skipping early avoids the
    // allocation cost.
    if line.contains("\"_meta\":") {
        return;
    }
    let rec: WireRecord = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(_) => return, // unknown / future schema — ignore
    };
    // Build the matching context once: a flat JSON object of {v, ts_ms,
    // kind, ...event_fields}. We construct from the parsed WireRecord
    // (which already round-trips through serde) so the lookup keys
    // exactly match what users see in the wire format.
    let ctx_val = match build_ctx(&rec) {
        Some(v) => v,
        None => return,
    };
    let type_name = rec.event.type_name();
    let key_hash = event_key_hash(&ctx_val);

    for rule in rules {
        if !matches(rule, type_name, &ctx_val) {
            continue;
        }
        let effective = rule.debounce_ms.unwrap_or(default_debounce_ms);
        if !debouncer.allow((rule.idx, key_hash), effective) {
            continue;
        }
        let title = template::render(&rule.title, &ctx_val);
        let body = template::render(&rule.body, &ctx_val);
        if let Err(e) = backend.dispatch(&title, &body) {
            // Don't crash on a single failed dispatch — log once and
            // keep serving.
            eprintln!("notifier: dispatch failed: {e}");
        }
    }
}

fn build_ctx(rec: &WireRecord) -> Option<serde_json::Value> {
    let mut v = serde_json::to_value(rec).ok()?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert(
            "kind".to_string(),
            serde_json::Value::String(rec.event.type_name().to_string()),
        );
    }
    Some(v)
}

fn sleep_with_shutdown(ctx: &PluginCtx, total: Duration) {
    // Poll the shutdown flag every 50ms so a clean exit doesn't have
    // to wait out the full backoff window.
    let step = Duration::from_millis(50);
    let deadline = Instant::now() + total;
    while Instant::now() < deadline {
        if ctx.shutdown.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(step.min(deadline - Instant::now()));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::events::{AppEvent, EventPublisher};
    use crate::plugins::PluginRegistry;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    fn tmp_sock_path(label: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // Keep the filename short to stay well under sun_path (104 on
        // macOS, 108 on Linux). The publisher's tests use the same
        // technique — we shorten the label prefix.
        dir.join(format!("ab-n-{label}-{pid}-{nanos}.sock"))
    }

    fn status_changed(id: &str) -> AppEvent {
        AppEvent::StatusChanged {
            session_id: id.to_string(),
            from: crate::model::SessionStatus::Thinking,
            to: crate::model::SessionStatus::Executing,
        }
    }

    /// Spin until the publisher reports at least `n` connected
    /// consumers, or `deadline` elapses. Returns the observed count.
    fn wait_for_conns(publisher: &EventPublisher, n: usize, deadline: Duration) -> usize {
        let start = Instant::now();
        loop {
            let c = publisher.conn_count();
            if c >= n || start.elapsed() >= deadline {
                return c;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Spin until `sink` has at least `n` records, or `deadline`
    /// elapses. Returns the observed count.
    fn wait_for_records(sink: &backend::CaptureSink, n: usize, deadline: Duration) -> usize {
        let start = Instant::now();
        loop {
            let c = sink.len();
            if c >= n || start.elapsed() >= deadline {
                return c;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn end_to_end_dispatch_pause_resume() {
        let path = tmp_sock_path("e2e");
        let publisher = EventPublisher::bind_uds(&path).expect("bind publisher");

        let sink = backend::CaptureSink::new();
        let cfg = NotifierConfig {
            enabled_at_startup: true,
            backend: Some(Backend::Capture(sink.clone())),
            debounce_ms: 0, // no debounce — test each publish independently
            rule: vec![rules::Rule {
                on: vec!["StatusChanged".into()],
                when: None,
                title: "session {session_id}".into(),
                body: "{from} -> {to}".into(),
                debounce_ms: None,
            }],
            generation: 0,
        };

        let mut registry = PluginRegistry::new();
        registry.start(Notifier::new(cfg), path.clone(), true);

        // Wait for the worker to connect.
        assert!(
            wait_for_conns(&publisher, 1, Duration::from_secs(3)) >= 1,
            "notifier should connect"
        );

        publisher.publish(&[status_changed("A")]);
        publisher.publish(&[status_changed("B")]);
        assert_eq!(
            wait_for_records(&sink, 2, Duration::from_secs(3)),
            2,
            "expected 2 dispatches"
        );

        // Pause publisher — published events never reach the worker.
        publisher.set_paused(true);
        publisher.publish(&[status_changed("C")]);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(sink.len(), 2, "no new dispatches during pause");

        // Resume.
        publisher.set_paused(false);
        publisher.publish(&[status_changed("D")]);
        assert_eq!(
            wait_for_records(&sink, 3, Duration::from_secs(3)),
            3,
            "dispatch should resume after unpause"
        );

        registry.shutdown();
        drop(publisher);
    }

    #[test]
    fn worker_respects_plugin_pause() {
        // A different scenario: the plugin's own `enabled` flag is the
        // discard switch on the consumer side. Toggle it off and the
        // worker keeps the socket connected but drops inbound lines.
        let path = tmp_sock_path("pcon");
        let publisher = EventPublisher::bind_uds(&path).expect("bind publisher");

        let sink = backend::CaptureSink::new();
        let cfg = NotifierConfig {
            enabled_at_startup: false, // start paused
            backend: Some(Backend::Capture(sink.clone())),
            debounce_ms: 0,
            rule: vec![rules::Rule {
                on: vec![],
                when: None,
                title: "{kind}".into(),
                body: String::new(),
                debounce_ms: None,
            }],
            generation: 0,
        };

        let mut registry = PluginRegistry::new();
        // Start with enabled=false on the registry side.
        registry.start(Notifier::new(cfg), path.clone(), false);
        assert!(wait_for_conns(&publisher, 1, Duration::from_secs(3)) >= 1);

        publisher.publish(&[status_changed("A")]);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(sink.len(), 0, "paused plugin discards inbound");

        registry.set_enabled("notifier", true);
        publisher.publish(&[status_changed("B")]);
        assert!(wait_for_records(&sink, 1, Duration::from_secs(3)) >= 1);

        registry.shutdown();
        drop(publisher);
    }

    #[test]
    fn debounce_suppresses_repeats_for_same_key() {
        let path = tmp_sock_path("deb");
        let publisher = EventPublisher::bind_uds(&path).expect("bind publisher");

        let sink = backend::CaptureSink::new();
        let cfg = NotifierConfig {
            enabled_at_startup: true,
            backend: Some(Backend::Capture(sink.clone())),
            debounce_ms: 60_000, // very long window
            rule: vec![rules::Rule {
                on: vec!["StatusChanged".into()],
                when: None,
                title: "x".into(),
                body: "y".into(),
                debounce_ms: None,
            }],
            generation: 0,
        };

        let mut registry = PluginRegistry::new();
        registry.start(Notifier::new(cfg), path.clone(), true);
        assert!(wait_for_conns(&publisher, 1, Duration::from_secs(3)) >= 1);

        publisher.publish(&[status_changed("A")]);
        publisher.publish(&[status_changed("A")]);
        publisher.publish(&[status_changed("A")]);
        // Allow one through; the next two should be suppressed.
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(sink.len(), 1, "debounce should suppress repeats");

        // A different session_id has its own debounce key.
        publisher.publish(&[status_changed("B")]);
        assert!(wait_for_records(&sink, 2, Duration::from_secs(3)) >= 2);

        registry.shutdown();
        drop(publisher);
    }

    #[test]
    fn build_ctx_includes_kind_field() {
        let rec = WireRecord::new(status_changed("abc"), 42);
        let ctx = build_ctx(&rec).unwrap();
        assert_eq!(
            ctx.get("kind").and_then(|v| v.as_str()),
            Some("StatusChanged")
        );
        assert_eq!(ctx.get("session_id").and_then(|v| v.as_str()), Some("abc"));
        assert_eq!(ctx.get("ts_ms").and_then(|v| v.as_u64()), Some(42));
    }

    #[test]
    fn handle_line_skips_meta_records() {
        let sink = backend::CaptureSink::new();
        let backend = Backend::Capture(sink.clone());
        let rules = compile(vec![rules::Rule {
            on: vec![],
            when: None,
            title: "t".into(),
            body: "b".into(),
            debounce_ms: None,
        }]);
        let mut deb = Debouncer::new(0);
        handle_line(
            "{\"v\":1,\"_meta\":\"dropped\",\"count\":7}",
            &backend,
            &rules,
            &mut deb,
            0,
        );
        assert_eq!(sink.len(), 0);
    }

    #[test]
    fn handle_line_dispatches_known_event() {
        let sink = backend::CaptureSink::new();
        let backend = Backend::Capture(sink.clone());
        let rules = compile(vec![rules::Rule {
            on: vec!["StatusChanged".into()],
            when: None,
            title: "ses {session_id}".into(),
            body: "{from} {to}".into(),
            debounce_ms: None,
        }]);
        let mut deb = Debouncer::new(0);
        let line =
            "{\"v\":1,\"ts_ms\":1,\"type\":\"StatusChanged\",\"session_id\":\"X\",\"from\":\"Thinking\",\"to\":\"Executing\"}";
        handle_line(line, &backend, &rules, &mut deb, 0);
        assert_eq!(sink.len(), 1);
        let snap = sink.snapshot();
        assert_eq!(snap[0].0, "ses X");
        assert_eq!(snap[0].1, "Thinking Executing");
    }

    #[test]
    fn hot_reload_updates_rules_via_shared_handle() {
        // Start the worker with a no-match rule list, then swap in a
        // matching rule via the shared `Arc<RwLock<NotifierConfig>>`.
        // Bumping `generation` is the signal the worker observes to
        // recompile, so dispatches that previously didn't match now
        // fire.
        let path = tmp_sock_path("hot");
        let publisher = EventPublisher::bind_uds(&path).expect("bind publisher");

        let sink = backend::CaptureSink::new();
        let initial = NotifierConfig {
            enabled_at_startup: true,
            backend: Some(Backend::Capture(sink.clone())),
            debounce_ms: 0,
            rule: vec![rules::Rule {
                on: vec!["NeverFiresEventKind".into()],
                when: None,
                title: "no".into(),
                body: "match".into(),
                debounce_ms: None,
            }],
            generation: 0,
        };
        let notifier = Notifier::new(initial);
        let shared = notifier.shared();

        let mut registry = PluginRegistry::new();
        registry.start(notifier, path.clone(), true);
        assert!(wait_for_conns(&publisher, 1, Duration::from_secs(3)) >= 1);

        // Baseline: no rule matches StatusChanged, so nothing dispatched.
        publisher.publish(&[status_changed("A")]);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(sink.len(), 0, "no rule should match initially");

        // Hot-swap the rule list to one that matches, bump generation.
        {
            let mut cfg = shared.write().expect("write lock");
            cfg.rule = vec![rules::Rule {
                on: vec!["StatusChanged".into()],
                when: None,
                title: "hot {session_id}".into(),
                body: "live".into(),
                debounce_ms: None,
            }];
            cfg.generation = cfg.generation.wrapping_add(1);
        }

        publisher.publish(&[status_changed("B")]);
        let got = wait_for_records(&sink, 1, Duration::from_secs(3));
        assert!(got >= 1, "expected dispatch after rule swap, got {got}");
        let snap = sink.snapshot();
        assert_eq!(snap.last().unwrap().0, "hot B");

        registry.shutdown();
        drop(publisher);
    }

    // Silence the unused-import lint when the file is built without
    // the unix subprocess machinery.
    #[allow(dead_code)]
    fn _ensure_atomic_used() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn example_config_round_trips() {
        // Guard against the example_config block in `info()` drifting
        // out of sync with NotifierConfig — every key in the snippet
        // must deserialize cleanly. We wrap the snippet under the
        // top-level [plugins] table to mirror how it sits in
        // ~/.config/abtop/config.toml.
        let snippet = super::info().example_config;
        // The snippet uses `[plugins.notifier]`, so parse via
        // ConfigFile to exercise the full path users actually take.
        let parsed: crate::event_config::schema::ConfigFile =
            toml::from_str(snippet).expect("notifier example_config should parse as ConfigFile");
        let n = parsed
            .plugins
            .notifier
            .expect("notifier table missing from example_config");
        assert!(n.enabled_at_startup, "example_config sets enabled = true");
        assert_eq!(n.debounce_ms, 5_000);
        assert_eq!(n.rule.len(), 1);
        assert_eq!(n.rule[0].on, vec!["RateLimited".to_string()]);
    }
}
