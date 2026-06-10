//! System Notifier plugin — surfaces events through a user-supplied
//! conduit binary.
//!
//! Unlike the sibling [`crate::plugins::notifier`] (which ships
//! compiled-in OS-notification backends), this plugin delegates the
//! "actually surface a notification" step to a user-configured conduit
//! script. It still owns the connect-loop and NDJSON parsing — the user
//! owns the script that takes title/body/event JSON and does whatever
//! they want with it (osascript, ntfy, curl webhook, ...).
//!
//! Failure semantics: log + drop. A dedicated invocation thread reads
//! from a bounded `mpsc::sync_channel(32)`; the socket-reader thread
//! `try_send`s into the channel and increments a drop counter on full
//! so a slow conduit can't stall socket reads. Per-invocation
//! wall-clock timeout defaults to 5 seconds — see [`invoke`].
//!
//! Hot reload follows the Notifier pattern: `Arc<RwLock<_>>` config +
//! `generation` counter. The reader thread re-snapshots on each loop
//! iteration when the generation advances.

pub mod config;
pub mod invoke;

pub use config::{SharedSystemNotifierConfig, SystemNotifierConfig};

use crate::events::WireRecord;
use crate::plugins::common::debounce::Debouncer;
use crate::plugins::common::event_key::event_key_hash;
use crate::plugins::common::template;
use crate::plugins::{Plugin, PluginCtx, PluginHandle};
use invoke::{build_request, InvokeError, InvokeRequest, Invoker, RealInvoker};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Bounded invocation-channel depth. Reader `try_send`s into it; on
/// full it drops the event and bumps a counter so a slow conduit
/// can't stall socket reads.
const CHANNEL_DEPTH: usize = 32;

/// Plugin type — registered with [`crate::plugins::PluginRegistry`].
pub struct SystemNotifier {
    pub config: SharedSystemNotifierConfig,
}

impl SystemNotifier {
    pub fn new(config: SystemNotifierConfig) -> Self {
        Self {
            config: Arc::new(std::sync::RwLock::new(config)),
        }
    }

    pub fn from_shared(config: SharedSystemNotifierConfig) -> Self {
        Self { config }
    }

    pub fn shared(&self) -> SharedSystemNotifierConfig {
        Arc::clone(&self.config)
    }
}

impl Plugin for SystemNotifier {
    fn name(&self) -> &'static str {
        "system_notifier"
    }

    fn start(&self, ctx: PluginCtx) -> PluginHandle {
        let shared = Arc::clone(&self.config);
        let enabled_for_handle = Arc::clone(&ctx.enabled);
        let join = std::thread::Builder::new()
            .name("abtop-system-notifier".into())
            .spawn(move || {
                run_with_invoker(ctx, shared, Arc::new(RealInvoker));
            })
            .expect("spawn abtop-system-notifier thread");
        PluginHandle {
            name: "system_notifier",
            enabled: enabled_for_handle,
            join,
        }
    }
}

/// Worker entry. Spawns the dedicated invocation thread, then runs
/// the socket-reader loop until shutdown. The invocation thread is
/// joined inside this function so a misbehaving conduit's drain time
/// is bounded by the timeout, not the registry's shutdown.
pub fn run_with_invoker(
    ctx: PluginCtx,
    shared: SharedSystemNotifierConfig,
    invoker: Arc<dyn Invoker>,
) {
    let (tx, rx) = sync_channel::<InvokeRequest>(CHANNEL_DEPTH);

    // Invocation thread: blocks on recv(), calls the invoker. Exits
    // when the channel closes (tx dropped at worker shutdown).
    let inv_join = std::thread::Builder::new()
        .name("abtop-system-notifier-invoke".into())
        .spawn(move || {
            let mut last_logged: Option<Instant> = None;
            while let Ok(req) = rx.recv() {
                if let Err(e) = invoker.invoke(&req) {
                    log_rate_limited(&mut last_logged, &e);
                }
            }
        })
        .expect("spawn abtop-system-notifier-invoke thread");

    reader_loop(&ctx, &shared, &tx);

    // Drop the sender so the invocation thread sees the channel
    // close and exits its recv loop cleanly.
    drop(tx);
    let _ = inv_join.join();
}

/// Rate-limited stderr log — at most one line per 30s per worker.
fn log_rate_limited(last_logged: &mut Option<Instant>, err: &InvokeError) {
    let now = Instant::now();
    let should_log = match *last_logged {
        None => true,
        Some(prev) => now.duration_since(prev) >= Duration::from_secs(30),
    };
    if should_log {
        eprintln!("system_notifier: invocation failed: {err}");
        *last_logged = Some(now);
    }
}

/// Socket reader loop. Mirrors the Notifier's connect-loop discipline:
/// exponential backoff 200ms → 5s cap, NDJSON line iteration, hot
/// reload via the `generation` counter.
fn reader_loop(
    ctx: &PluginCtx,
    shared: &SharedSystemNotifierConfig,
    tx: &SyncSender<InvokeRequest>,
) {
    // Snapshot the initial config.
    let (mut snapshot, mut last_generation) = {
        let cfg = shared.read().expect("system_notifier config lock poisoned");
        (cfg.clone(), cfg.generation)
    };
    let mut debouncer = Debouncer::new(snapshot.debounce_ms);
    let mut drop_counter: u64 = 0;
    let mut last_drop_log: Option<Instant> = None;
    let mut backoff = Duration::from_millis(200);
    let backoff_cap = Duration::from_secs(5);

    while !ctx.shutdown.load(Ordering::Relaxed) {
        let stream = match connect_with_shutdown(ctx, &backoff) {
            Some(s) => s,
            None => return,
        };
        backoff = Duration::from_millis(200);

        process_stream(
            stream,
            ctx,
            shared,
            tx,
            &mut snapshot,
            &mut last_generation,
            &mut debouncer,
            &mut drop_counter,
            &mut last_drop_log,
        );

        if ctx.shutdown.load(Ordering::Relaxed) {
            return;
        }
        sleep_with_shutdown(ctx, Duration::from_millis(200));
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
    None
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn process_stream(
    stream: std::os::unix::net::UnixStream,
    ctx: &PluginCtx,
    shared: &SharedSystemNotifierConfig,
    tx: &SyncSender<InvokeRequest>,
    snapshot: &mut SystemNotifierConfig,
    last_generation: &mut u64,
    debouncer: &mut Debouncer,
    drop_counter: &mut u64,
    last_drop_log: &mut Option<Instant>,
) {
    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        if ctx.shutdown.load(Ordering::Relaxed) {
            return;
        }
        let line = match line {
            Ok(l) => l,
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                refresh_if_changed(shared, snapshot, last_generation);
                continue;
            }
            Err(_) => return,
        };
        refresh_if_changed(shared, snapshot, last_generation);
        if line.is_empty() {
            continue;
        }
        if !ctx.enabled.load(Ordering::Relaxed) {
            continue;
        }
        handle_line(&line, snapshot, debouncer, tx, drop_counter, last_drop_log);
    }
}

#[cfg(not(unix))]
#[allow(clippy::too_many_arguments)]
fn process_stream(
    _stream: std::fs::File,
    _ctx: &PluginCtx,
    _shared: &SharedSystemNotifierConfig,
    _tx: &SyncSender<InvokeRequest>,
    _snapshot: &mut SystemNotifierConfig,
    _last_generation: &mut u64,
    _debouncer: &mut Debouncer,
    _drop_counter: &mut u64,
    _last_drop_log: &mut Option<Instant>,
) {
}

/// Refresh the cached config snapshot if the generation counter has
/// advanced. Cheap when nothing changed (a read-lock + u64 compare).
fn refresh_if_changed(
    shared: &SharedSystemNotifierConfig,
    snapshot: &mut SystemNotifierConfig,
    last_generation: &mut u64,
) {
    let cfg = match shared.read() {
        Ok(c) => c,
        Err(_) => return,
    };
    if cfg.generation == *last_generation {
        return;
    }
    *snapshot = cfg.clone();
    *last_generation = cfg.generation;
}

fn handle_line(
    line: &str,
    cfg: &SystemNotifierConfig,
    debouncer: &mut Debouncer,
    tx: &SyncSender<InvokeRequest>,
    drop_counter: &mut u64,
    last_drop_log: &mut Option<Instant>,
) {
    // Skip `_meta` records (publisher drop markers).
    if line.contains("\"_meta\":") {
        return;
    }
    let rec: WireRecord = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(_) => return,
    };

    let type_name = rec.event.type_name();
    // Apply on filter — empty list means "all types".
    if !cfg.on.is_empty() && !cfg.on.iter().any(|n| n == type_name) {
        return;
    }

    let ctx_val = match build_ctx(&rec) {
        Some(v) => v,
        None => return,
    };
    let key_hash = event_key_hash(&ctx_val);

    // Single producer (this plugin has only one conduit), so producer
    // index is always 0 in the debouncer key.
    if !debouncer.allow((0, key_hash), cfg.debounce_ms) {
        return;
    }

    let title = template::render(&cfg.title, &ctx_val);
    let body = template::render(&cfg.body, &ctx_val);
    let request = build_request(cfg, &rec, title, body);

    match tx.try_send(request) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            *drop_counter += 1;
            log_drops_rate_limited(*drop_counter, last_drop_log);
        }
        Err(TrySendError::Disconnected(_)) => {
            // Invocation thread exited — nothing we can do. Reader
            // will keep parsing in case it comes back (it won't in
            // current design, but the cost is zero).
        }
    }
}

fn log_drops_rate_limited(count: u64, last_logged: &mut Option<Instant>) {
    let now = Instant::now();
    let should = match *last_logged {
        None => true,
        Some(prev) => now.duration_since(prev) >= Duration::from_secs(30),
    };
    if should {
        eprintln!("system_notifier: dropped {count} event(s) — invocation channel full");
        *last_logged = Some(now);
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
    use crate::plugins::system_notifier::invoke::CaptureInvoker;
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
        dir.join(format!("ab-sn-{label}-{pid}-{nanos}.sock"))
    }

    fn status_changed(id: &str) -> AppEvent {
        AppEvent::StatusChanged {
            session_id: id.to_string(),
            from: crate::model::SessionStatus::Thinking,
            to: crate::model::SessionStatus::Executing,
        }
    }

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

    fn wait_for_invocations(inv: &CaptureInvoker, n: usize, deadline: Duration) -> usize {
        let start = Instant::now();
        loop {
            let c = inv.len();
            if c >= n || start.elapsed() >= deadline {
                return c;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Spawn the plugin via `run_with_invoker` (not via PluginRegistry)
    /// so the test can inject a `CaptureInvoker`. Returns the thread
    /// join handle + a "shutdown" controller pair the test cleans up.
    struct TestWorker {
        invoker: CaptureInvoker,
        ctx: PluginCtx,
        join: std::thread::JoinHandle<()>,
    }

    impl TestWorker {
        fn start(cfg: SystemNotifierConfig, socket_path: PathBuf, enabled: bool) -> Self {
            let shutdown = Arc::new(AtomicBool::new(false));
            let enabled_flag = Arc::new(AtomicBool::new(enabled));
            let ctx = PluginCtx {
                socket_path,
                enabled: Arc::clone(&enabled_flag),
                shutdown: Arc::clone(&shutdown),
            };
            let invoker = CaptureInvoker::new();
            let invoker_clone = invoker.clone();
            let ctx_for_thread = ctx.clone();
            let shared: SharedSystemNotifierConfig = Arc::new(std::sync::RwLock::new(cfg));
            let join = std::thread::Builder::new()
                .name("test-abtop-system-notifier".into())
                .spawn(move || {
                    run_with_invoker(ctx_for_thread, shared, Arc::new(invoker_clone));
                })
                .expect("spawn test worker");
            Self { invoker, ctx, join }
        }

        fn shutdown(self) {
            self.ctx.shutdown.store(true, Ordering::Relaxed);
            let _ = self.join.join();
        }
    }

    #[test]
    fn end_to_end_invokes_conduit() {
        let path = tmp_sock_path("e2e");
        let publisher = EventPublisher::bind_uds(&path).expect("bind publisher");

        let cfg = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: "/bin/true".to_string(),
            debounce_ms: 0,
            title: "T-{session_id}".to_string(),
            body: "B-{kind}".to_string(),
            ..Default::default()
        };
        let worker = TestWorker::start(cfg, path.clone(), true);

        assert!(
            wait_for_conns(&publisher, 1, Duration::from_secs(3)) >= 1,
            "worker should connect"
        );

        publisher.publish(&[status_changed("A")]);
        publisher.publish(&[status_changed("B")]);
        assert_eq!(
            wait_for_invocations(&worker.invoker, 2, Duration::from_secs(3)),
            2,
            "expected 2 invocations"
        );

        let snap = worker.invoker.snapshot();
        assert_eq!(snap[0].title, "T-A");
        assert_eq!(snap[0].body, "B-StatusChanged");
        assert_eq!(snap[1].title, "T-B");

        worker.shutdown();
        drop(publisher);
    }

    #[test]
    fn pause_discards_inbound() {
        let path = tmp_sock_path("pause");
        let publisher = EventPublisher::bind_uds(&path).expect("bind publisher");

        let cfg = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: "/bin/true".to_string(),
            debounce_ms: 0,
            ..Default::default()
        };
        let worker = TestWorker::start(cfg, path.clone(), false); // start disabled
        assert!(wait_for_conns(&publisher, 1, Duration::from_secs(3)) >= 1);

        publisher.publish(&[status_changed("A")]);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(worker.invoker.len(), 0, "paused plugin discards inbound");

        // Resume.
        worker.ctx.enabled.store(true, Ordering::Relaxed);
        publisher.publish(&[status_changed("B")]);
        assert!(wait_for_invocations(&worker.invoker, 1, Duration::from_secs(3)) >= 1);

        worker.shutdown();
        drop(publisher);
    }

    #[test]
    fn on_filter_excludes_unmatched_events() {
        let path = tmp_sock_path("onf");
        let publisher = EventPublisher::bind_uds(&path).expect("bind publisher");

        let cfg = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: "/bin/true".to_string(),
            debounce_ms: 0,
            on: vec!["RateLimited".to_string()],
            ..Default::default()
        };
        let worker = TestWorker::start(cfg, path.clone(), true);
        assert!(wait_for_conns(&publisher, 1, Duration::from_secs(3)) >= 1);

        publisher.publish(&[status_changed("A")]);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            worker.invoker.len(),
            0,
            "StatusChanged should be filtered out"
        );

        worker.shutdown();
        drop(publisher);
    }

    #[test]
    fn debounce_suppresses_repeats() {
        let path = tmp_sock_path("deb");
        let publisher = EventPublisher::bind_uds(&path).expect("bind publisher");

        let cfg = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: "/bin/true".to_string(),
            debounce_ms: 60_000,
            ..Default::default()
        };
        let worker = TestWorker::start(cfg, path.clone(), true);
        assert!(wait_for_conns(&publisher, 1, Duration::from_secs(3)) >= 1);

        publisher.publish(&[status_changed("A")]);
        publisher.publish(&[status_changed("A")]);
        publisher.publish(&[status_changed("A")]);
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(worker.invoker.len(), 1, "debounce should suppress repeats");

        // Different session_id is independent.
        publisher.publish(&[status_changed("B")]);
        assert!(wait_for_invocations(&worker.invoker, 2, Duration::from_secs(3)) >= 2);

        worker.shutdown();
        drop(publisher);
    }

    #[test]
    fn hot_reload_picks_up_new_template() {
        let path = tmp_sock_path("hot");
        let publisher = EventPublisher::bind_uds(&path).expect("bind publisher");

        let initial = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: "/bin/true".to_string(),
            debounce_ms: 0,
            title: "V1-{session_id}".to_string(),
            ..Default::default()
        };
        let shared: SharedSystemNotifierConfig = Arc::new(std::sync::RwLock::new(initial));
        let shutdown = Arc::new(AtomicBool::new(false));
        let enabled = Arc::new(AtomicBool::new(true));
        let ctx = PluginCtx {
            socket_path: path.clone(),
            enabled: Arc::clone(&enabled),
            shutdown: Arc::clone(&shutdown),
        };
        let invoker = CaptureInvoker::new();
        let invoker_clone = invoker.clone();
        let shared_clone = Arc::clone(&shared);
        let join = std::thread::Builder::new()
            .name("test-hot-reload".into())
            .spawn(move || {
                run_with_invoker(ctx, shared_clone, Arc::new(invoker_clone));
            })
            .expect("spawn");
        assert!(wait_for_conns(&publisher, 1, Duration::from_secs(3)) >= 1);

        publisher.publish(&[status_changed("A")]);
        assert!(wait_for_invocations(&invoker, 1, Duration::from_secs(3)) >= 1);
        assert_eq!(invoker.snapshot()[0].title, "V1-A");

        // Hot swap.
        {
            let mut cfg = shared.write().expect("write lock");
            cfg.title = "V2-{session_id}".to_string();
            cfg.generation = cfg.generation.wrapping_add(1);
        }

        publisher.publish(&[status_changed("B")]);
        assert!(wait_for_invocations(&invoker, 2, Duration::from_secs(3)) >= 2);
        let snap = invoker.snapshot();
        assert_eq!(snap.last().unwrap().title, "V2-B");

        shutdown.store(true, Ordering::Relaxed);
        let _ = join.join();
        drop(publisher);
    }

    #[test]
    fn handle_line_skips_meta_records() {
        let cfg = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: "/bin/true".to_string(),
            debounce_ms: 0,
            ..Default::default()
        };
        let mut deb = Debouncer::new(0);
        let (tx, rx) = sync_channel::<InvokeRequest>(8);
        let mut drop_counter: u64 = 0;
        let mut last_drop: Option<Instant> = None;
        handle_line(
            "{\"v\":1,\"_meta\":\"dropped\",\"count\":7}",
            &cfg,
            &mut deb,
            &tx,
            &mut drop_counter,
            &mut last_drop,
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn channel_full_increments_drop_counter() {
        // A drop-on-full unit test using the sync_channel + try_send
        // path directly, without spinning a real publisher.
        let cfg = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: "/bin/true".to_string(),
            debounce_ms: 0,
            ..Default::default()
        };
        let mut deb = Debouncer::new(0);
        // Channel of depth 1 — fill it then verify the next line drops.
        let (tx, _rx) = sync_channel::<InvokeRequest>(1);
        let mut drop_counter: u64 = 0;
        let mut last_drop: Option<Instant> = None;
        let line1 = "{\"v\":1,\"ts_ms\":1,\"type\":\"StatusChanged\",\"session_id\":\"A\",\"from\":\"Thinking\",\"to\":\"Executing\"}";
        let line2 = "{\"v\":1,\"ts_ms\":2,\"type\":\"StatusChanged\",\"session_id\":\"B\",\"from\":\"Thinking\",\"to\":\"Executing\"}";
        handle_line(
            line1,
            &cfg,
            &mut deb,
            &tx,
            &mut drop_counter,
            &mut last_drop,
        );
        handle_line(
            line2,
            &cfg,
            &mut deb,
            &tx,
            &mut drop_counter,
            &mut last_drop,
        );
        assert_eq!(drop_counter, 1, "second send should have been dropped");
    }
}
