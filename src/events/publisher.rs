//! Non-blocking event publisher over a Unix domain socket (UDS).
//!
//! [`EventPublisher::disabled`] returns a no-op publisher whose
//! [`publish`](EventPublisher::publish) call is a sub-microsecond
//! branch. [`EventPublisher::bind_uds`] opens a `UnixListener`, spawns
//! an accept thread, and routes each accepted connection through a
//! bounded `SyncSender<Bytes>` channel to a dedicated writer thread.
//!
//! Per-connection backpressure is implemented via `try_send`: when a
//! consumer can't keep up, its queue fills and further events are
//! dropped — the per-connection drop count is sent inline as a
//! `{"v":1,"_meta":"dropped","count":N}` record on the next successful
//! send for that connection.
//!
//! The tick thread is never blocked. The accept thread, writer
//! threads, and the publisher itself share no locks except a short
//! `Mutex<Vec<ConnState>>` guarding the active connection list.

use crate::events::types::{AppEvent, WireRecord, WIRE_VERSION};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-connection bounded queue depth. A slow consumer holds at most
/// this many enqueued lines before drops kick in.
pub const DEFAULT_BACKLOG: usize = 256;

/// State for one connected consumer.
struct ConnState {
    sender: SyncSender<Vec<u8>>,
    drops: Arc<AtomicU64>,
    dead: Arc<AtomicBool>,
}

#[derive(Default)]
struct Inner {
    conns: Mutex<Vec<ConnState>>,
}

/// Non-blocking publisher. Cloning is cheap; share an `Arc<EventPublisher>`
/// across the tick thread and the accept thread.
pub struct EventPublisher {
    enabled: AtomicBool,
    /// Runtime pause flag. When true, `publish()` returns early without
    /// touching the connection list — the socket stays bound and consumers
    /// remain connected. Toggled from the TUI via the `e` key.
    paused: AtomicBool,
    inner: Arc<Inner>,
    socket_path: Option<PathBuf>,
    /// Cumulative drop counter across ALL connections, for tests/metrics.
    total_drops: AtomicU64,
    /// Signals the accept thread to exit. Set on drop.
    shutdown: Arc<AtomicBool>,
    /// Handle to the accept thread, taken on drop.
    accept_thread: Mutex<Option<JoinHandle<()>>>,
}

impl EventPublisher {
    /// A no-op publisher: [`publish`](Self::publish) returns immediately
    /// without serializing anything and never binds a socket. Use for
    /// the default "events off" path so the tick thread pays only a
    /// branch.
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            inner: Arc::new(Inner::default()),
            socket_path: None,
            total_drops: AtomicU64::new(0),
            shutdown: Arc::new(AtomicBool::new(false)),
            accept_thread: Mutex::new(None),
        })
    }

    /// Bind to a Unix domain socket at `path`. Spawns an accept thread.
    /// Recovers from a stale socket file (one whose listener is gone)
    /// by unlinking and rebinding. Errors when another live listener
    /// owns the path or the path overflows `sun_path`.
    #[cfg(unix)]
    pub fn bind_uds(path: &Path) -> std::io::Result<Arc<Self>> {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::{UnixListener, UnixStream};

        crate::events::socket_path::validate_sun_path_length(path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        // Refuse any path that still contains an unresolved `${...}`
        // placeholder. Catches typos in unknown variable names (e.g.
        // `${XGD_RUNTIME_DIR}`) so we don't end up creating literal
        // `${...}` directories at the `create_dir_all` call below.
        if let Some(s) = path.to_str() {
            if s.contains("${") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "socket path contains unresolved placeholder: {} \
                         (check `[events] socket` in your config)",
                        path.display()
                    ),
                ));
            }
        }

        if path.exists() {
            // Probe — if a live listener is on the other end, refuse.
            // Otherwise unlink and rebind.
            match UnixStream::connect(path) {
                Ok(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        format!("abtop already listening at {}", path.display()),
                    ));
                }
                Err(_) => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let listener = UnixListener::bind(path)?;
        // 0600: owner read/write only.
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));

        let inner = Arc::new(Inner::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let inner_for_thread = Arc::clone(&inner);
        let shutdown_for_thread = Arc::clone(&shutdown);

        // Non-blocking accept loop with a 100ms poll so we can react to
        // shutdown without leaving a hanging accept() call.
        listener.set_nonblocking(true)?;

        let handle = thread::Builder::new()
            .name("abtop-events-accept".into())
            .spawn(move || {
                accept_loop(listener, inner_for_thread, shutdown_for_thread);
            })?;

        Ok(Arc::new(Self {
            enabled: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            inner,
            socket_path: Some(path.to_path_buf()),
            total_drops: AtomicU64::new(0),
            shutdown,
            accept_thread: Mutex::new(Some(handle)),
        }))
    }

    /// Returns true if the publisher is in the "enabled" state, i.e.
    /// `publish` will attempt delivery to connected consumers.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Resolved socket path, if any. `None` for [`disabled`](Self::disabled).
    pub fn socket_path(&self) -> Option<&Path> {
        self.socket_path.as_deref()
    }

    /// Cumulative drop counter across all current and past consumers.
    /// Stable for tests; not a primary metric — see also the per-connection
    /// `_meta:dropped` records emitted inline.
    pub fn drop_count(&self) -> u64 {
        self.total_drops.load(Ordering::Relaxed)
    }

    /// Cumulative drop counter (alias of [`drop_count`](Self::drop_count))
    /// exposed under the name the UI uses.
    pub fn dropped_total(&self) -> u64 {
        self.total_drops.load(Ordering::Relaxed)
    }

    /// True when [`publish`](Self::publish) is currently suppressed via the
    /// runtime pause flag. Always false when the publisher is disabled.
    pub fn is_paused(&self) -> bool {
        self.enabled.load(Ordering::Relaxed) && self.paused.load(Ordering::Relaxed)
    }

    /// Flip the pause flag. No-op when the publisher is disabled —
    /// `disabled()` publishers have no socket bound, so pausing them is
    /// meaningless. The TUI surfaces the strict-mode message instead.
    pub fn set_paused(&self, paused: bool) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Number of currently-connected consumers. 0 when disabled.
    /// Briefly takes the conns mutex (held only during accept + publish
    /// broadcast, both very short) — safe to call from the UI redraw path.
    pub fn conn_count(&self) -> usize {
        if !self.enabled.load(Ordering::Relaxed) {
            return 0;
        }
        match self.inner.conns.lock() {
            Ok(guard) => guard
                .iter()
                .filter(|c| !c.dead.load(Ordering::Relaxed))
                .count(),
            Err(_) => 0,
        }
    }

    /// Try to deliver each event to every currently-connected consumer.
    /// Non-blocking: drops the line for any consumer whose queue is full.
    pub fn publish(&self, events: &[AppEvent]) {
        if !self.is_enabled() || events.is_empty() {
            return;
        }
        // Runtime pause: keep the socket bound and consumers connected, but
        // skip publishing entirely. One atomic load, no allocation.
        if self.paused.load(Ordering::Relaxed) {
            return;
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Serialize each event ONCE; clone bytes per consumer.
        let mut lines: Vec<Vec<u8>> = Vec::with_capacity(events.len());
        for ev in events {
            let rec = WireRecord::new(ev.clone(), now_ms);
            if let Ok(mut s) = serde_json::to_vec(&rec) {
                s.push(b'\n');
                lines.push(s);
            }
        }

        let mut conns = self.inner.conns.lock().unwrap();
        // Prune any dead connections that the writer threads marked.
        conns.retain(|c| !c.dead.load(Ordering::Relaxed));

        for conn in conns.iter() {
            // If we have buffered drops from prior publishes, emit one
            // _meta:dropped record before the next normal event.
            let pending = conn.drops.load(Ordering::Relaxed);
            if pending > 0 {
                let meta = format!(
                    "{{\"v\":{},\"_meta\":\"dropped\",\"count\":{}}}\n",
                    WIRE_VERSION, pending
                );
                if conn.sender.try_send(meta.into_bytes()).is_ok() {
                    conn.drops.store(0, Ordering::Relaxed);
                }
                // If the meta record itself doesn't fit, leave the drop
                // counter intact — we'll retry next publish.
            }
            for line in &lines {
                match conn.sender.try_send(line.clone()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        conn.drops.fetch_add(1, Ordering::Relaxed);
                        self.total_drops.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        conn.dead.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(unix)]
fn accept_loop(
    listener: std::os::unix::net::UnixListener,
    inner: Arc<Inner>,
    shutdown: Arc<AtomicBool>,
) {
    use std::time::Duration;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        match listener.accept() {
            Ok((stream, _addr)) => {
                // The listener is nonblocking, so accepted streams
                // inherit O_NONBLOCK on some platforms (Linux/macOS).
                // Force the connection back to blocking mode — the
                // writer thread will block on send when the peer's
                // recv buffer fills, but that doesn't reach the tick
                // thread because the bounded SyncSender handles
                // backpressure first.
                let _ = stream.set_nonblocking(false);
                let (tx, rx) = sync_channel::<Vec<u8>>(DEFAULT_BACKLOG);
                let drops = Arc::new(AtomicU64::new(0));
                let dead = Arc::new(AtomicBool::new(false));
                let dead_for_writer = Arc::clone(&dead);
                let _ = thread::Builder::new()
                    .name("abtop-events-writer".into())
                    .spawn(move || {
                        let mut stream = stream;
                        while let Ok(bytes) = rx.recv() {
                            if stream.write_all(&bytes).is_err() {
                                dead_for_writer.store(true, Ordering::Relaxed);
                                return;
                            }
                        }
                        dead_for_writer.store(true, Ordering::Relaxed);
                    });
                inner.conns.lock().unwrap().push(ConnState {
                    sender: tx,
                    drops,
                    dead,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

impl Drop for EventPublisher {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Ok(mut h) = self.accept_thread.lock() {
            if let Some(handle) = h.take() {
                // Give the accept thread one poll interval to exit.
                let _ = handle.join();
            }
        }
        if let Some(path) = &self.socket_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    fn tmp_sock_path(label: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!(
            "abtop-test-{}-{}-{}.sock",
            label,
            std::process::id(),
            uuid_like()
        ))
    }

    fn uuid_like() -> u64 {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }

    fn sample_event() -> AppEvent {
        AppEvent::StatusChanged {
            session_id: "s1".to_string(),
            from: crate::model::SessionStatus::Thinking,
            to: crate::model::SessionStatus::Executing,
        }
    }

    #[test]
    fn disabled_is_no_op() {
        let pub_ = EventPublisher::disabled();
        assert!(!pub_.is_enabled());
        pub_.publish(&[sample_event()]); // must not panic / not block
        assert_eq!(pub_.drop_count(), 0);
    }

    #[test]
    fn bind_creates_socket_and_unlinks_on_drop() {
        let path = tmp_sock_path("bind");
        let pub_ = EventPublisher::bind_uds(&path).expect("bind");
        assert!(path.exists());
        assert!(pub_.is_enabled());
        drop(pub_);
        // Give the accept thread a moment to wind down.
        thread::sleep(Duration::from_millis(150));
        assert!(!path.exists(), "socket file should be removed on drop");
    }

    #[test]
    fn stale_socket_is_reclaimed() {
        let path = tmp_sock_path("stale");
        // Create a dangling socket file (no listener) by creating an
        // empty regular file at the path — the connect probe will fail.
        std::fs::write(&path, "").unwrap();
        let pub_ = EventPublisher::bind_uds(&path).expect("rebind over stale");
        assert!(path.exists());
        drop(pub_);
    }

    #[test]
    fn second_bind_fails_when_live_listener_exists() {
        let path = tmp_sock_path("livebind");
        let _first = EventPublisher::bind_uds(&path).expect("first bind");
        let result = EventPublisher::bind_uds(&path);
        assert!(result.is_err(), "second bind must fail");
        if let Err(err) = result {
            let msg = format!("{err}");
            assert!(msg.contains("already listening"), "got: {msg}");
        }
    }

    #[test]
    fn connect_and_receive_one_event() {
        let path = tmp_sock_path("receive");
        let pub_ = EventPublisher::bind_uds(&path).expect("bind");

        // Wait for the accept thread to be ready: spin connecting.
        let mut stream = None;
        for _ in 0..50 {
            match UnixStream::connect(&path) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
        let stream = stream.expect("connect");
        // Ensure the test-side stream is blocking. The publisher's
        // accept loop uses `set_nonblocking(true)` on the listener;
        // the accepted streams inherit O_NONBLOCK on Linux/macOS,
        // and our connect() side could likewise be nonblocking under
        // certain socket-option propagation paths.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        // Wait long enough for the publisher's accept loop (100ms
        // poll interval) to pick up the new connection and the writer
        // thread to start.
        thread::sleep(Duration::from_millis(300));

        pub_.publish(&[sample_event()]);

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        assert!(line.contains("\"type\":\"StatusChanged\""), "got: {line}");
        assert!(line.contains("\"v\":1"), "got: {line}");
    }

    #[test]
    fn pause_blocks_publish() {
        let path = tmp_sock_path("pause");
        let pub_ = EventPublisher::bind_uds(&path).expect("bind");

        let mut stream = None;
        for _ in 0..50 {
            match UnixStream::connect(&path) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
        let stream = stream.expect("connect");
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        thread::sleep(Duration::from_millis(300));

        let mut reader = BufReader::new(stream);

        // A: published normally.
        let ev_a = AppEvent::StatusChanged {
            session_id: "A".to_string(),
            from: crate::model::SessionStatus::Thinking,
            to: crate::model::SessionStatus::Executing,
        };
        pub_.publish(&[ev_a]);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read A");
        assert!(line.contains("\"session_id\":\"A\""), "got: {line}");

        // Pause and publish B — must NOT arrive.
        assert!(!pub_.is_paused());
        pub_.set_paused(true);
        assert!(pub_.is_paused());
        let ev_b = AppEvent::StatusChanged {
            session_id: "B".to_string(),
            from: crate::model::SessionStatus::Thinking,
            to: crate::model::SessionStatus::Executing,
        };
        pub_.publish(&[ev_b]);

        // Set a short read timeout so this doesn't hang indefinitely if the
        // implementation is wrong — we EXPECT a timeout here.
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut paused_line = String::new();
        let read_result = reader.read_line(&mut paused_line);
        assert!(
            read_result.is_err() || paused_line.is_empty(),
            "unexpected payload during pause: {paused_line:?}"
        );

        // Resume, restore a generous timeout, publish C — must arrive.
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        pub_.set_paused(false);
        assert!(!pub_.is_paused());
        let ev_c = AppEvent::StatusChanged {
            session_id: "C".to_string(),
            from: crate::model::SessionStatus::Thinking,
            to: crate::model::SessionStatus::Executing,
        };
        pub_.publish(&[ev_c]);
        let mut resumed = String::new();
        reader.read_line(&mut resumed).expect("read C");
        assert!(resumed.contains("\"session_id\":\"C\""), "got: {resumed}");
    }

    #[test]
    fn set_paused_is_noop_when_disabled() {
        let pub_ = EventPublisher::disabled();
        pub_.set_paused(true);
        assert!(!pub_.is_paused(), "disabled publisher reports not-paused");
        assert_eq!(pub_.conn_count(), 0);
        assert_eq!(pub_.dropped_total(), 0);
        assert!(pub_.socket_path().is_none());
    }

    #[test]
    fn bind_refuses_path_with_unresolved_placeholder() {
        // Safety net: even if interpolation lets an unknown var slip
        // through verbatim (typo, forward-rolled config, etc.), bind
        // must refuse it rather than creating a literal `${...}`
        // directory on disk via `create_dir_all`.
        let dir = std::env::temp_dir().join("abtop_test_${UNRESOLVED}");
        let path = dir.join("abtop.sock");
        match EventPublisher::bind_uds(&path) {
            Ok(_) => panic!("bind must refuse paths containing ${{...}}"),
            Err(err) => {
                assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
                assert!(
                    err.to_string().contains("unresolved placeholder"),
                    "error message should explain the problem, got: {err}"
                );
            }
        }
        assert!(
            !dir.exists(),
            "no literal-named directory should have been created"
        );
    }

    #[test]
    fn tick_does_not_block_without_consumers() {
        let path = tmp_sock_path("noconsumer");
        let pub_ = EventPublisher::bind_uds(&path).expect("bind");
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            pub_.publish(&[sample_event()]);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "1000 publishes with no consumers took {elapsed:?}, should be sub-500ms"
        );
    }
}
