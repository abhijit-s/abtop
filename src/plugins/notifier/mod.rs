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
use std::sync::Arc;
use std::time::{Duration, Instant};

use backend::Backend;
use rules::{compile, event_key_hash, matches, CompiledRule, Debouncer};

const DEFAULT_DEBOUNCE_MS: u64 = 5_000;

fn default_debounce() -> u64 {
    DEFAULT_DEBOUNCE_MS
}

/// Configuration for the notifier plugin. Deserializes from the
/// `[plugins.notifier]` TOML table (the loader lands in U7).
#[derive(Clone, Debug, Deserialize)]
pub struct NotifierConfig {
    /// Start the worker at process start. CLI override:
    /// `--plugin-notify` / `--no-plugin-notify`.
    #[serde(default)]
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
}

impl Default for NotifierConfig {
    fn default() -> Self {
        Self {
            enabled_at_startup: false,
            backend: None,
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            rule: Vec::new(),
        }
    }
}

/// Custom backend deserializer so `"auto"` → `None` and the named
/// variants flow through [`Backend`]'s own `Deserialize`.
fn backend_deser<'de, D>(de: D) -> Result<Option<Backend>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Auto(String),
        Backend(Backend),
    }
    let r = Option::<Repr>::deserialize(de)?;
    Ok(match r {
        None => None,
        Some(Repr::Auto(s)) if s == "auto" => None,
        Some(Repr::Auto(s)) => {
            // Try to reparse — Repr::Auto only catches strings the
            // Backend enum rejected. Surface a clearer error.
            return Err(serde::de::Error::custom(format!(
                "unknown backend '{s}' — valid: osascript, notify-send, terminal-notifier, stderr, auto"
            )));
        }
        Some(Repr::Backend(b)) => Some(b),
    })
}

/// The plugin type that's registered with the [`crate::plugins::PluginRegistry`].
pub struct Notifier {
    pub config: NotifierConfig,
}

impl Notifier {
    pub fn new(config: NotifierConfig) -> Self {
        Self { config }
    }
}

impl Plugin for Notifier {
    fn name(&self) -> &'static str {
        "notifier"
    }

    fn start(&self, ctx: PluginCtx) -> PluginHandle {
        let cfg = self.config.clone();
        let enabled_for_handle = Arc::clone(&ctx.enabled);
        let join = std::thread::Builder::new()
            .name("abtop-notifier".into())
            .spawn(move || {
                run(ctx, cfg);
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
fn run(ctx: PluginCtx, cfg: NotifierConfig) {
    let backend = Backend::probe(cfg.backend.clone());
    let compiled = compile(cfg.rule.clone());
    for r in compiled.iter().filter(|r| r.disabled) {
        eprintln!(
            "notifier: rule #{} disabled — invalid `when`: {}",
            r.idx,
            r.disable_reason.as_deref().unwrap_or("(no reason)")
        );
    }
    let mut debouncer = Debouncer::new(cfg.debounce_ms);
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
            &backend,
            &compiled,
            &mut debouncer,
            cfg.debounce_ms,
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
fn process_stream(
    stream: std::os::unix::net::UnixStream,
    ctx: &PluginCtx,
    backend: &Backend,
    rules: &[CompiledRule],
    debouncer: &mut Debouncer,
    default_debounce_ms: u64,
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
                continue;
            }
            Err(_) => return, // hangup; the outer loop reconnects
        };
        if line.is_empty() {
            continue;
        }
        if !ctx.enabled.load(Ordering::Relaxed) {
            continue; // pause: discard while paused
        }
        handle_line(&line, backend, rules, debouncer, default_debounce_ms);
    }
}

#[cfg(not(unix))]
fn process_stream(
    _stream: std::fs::File,
    _ctx: &PluginCtx,
    _backend: &Backend,
    _rules: &[CompiledRule],
    _debouncer: &mut Debouncer,
    _default_debounce_ms: u64,
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

    // Silence the unused-import lint when the file is built without
    // the unix subprocess machinery.
    #[allow(dead_code)]
    fn _ensure_atomic_used() -> AtomicBool {
        AtomicBool::new(false)
    }
}
