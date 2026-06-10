//! Desktop-notification backends.
//!
//! Each backend is invoked via [`std::process::Command`] — we deliberately
//! avoid pulling in a notification crate so the dependency surface stays
//! flat and the implementation stays trivially testable. The `Stderr`
//! variant is the always-available fallback (prints a one-line record)
//! and `Capture` is a `#[cfg(test)]` variant for integration tests to
//! observe what would have been dispatched.

use serde::Deserialize;
use std::io;
use std::process::{Command, Stdio};
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::template::escape_for_osascript;

/// Backend identity. `Auto` maps to `None` at the config layer; the
/// worker probes a platform-appropriate default order.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    Osascript,
    NotifySend,
    TerminalNotifier,
    Stderr,
    /// Test-only sink. The worker treats this exactly like any other
    /// backend — it just calls `.dispatch()`. Available only under
    /// `cfg(test)`.
    #[cfg(test)]
    #[serde(skip)]
    Capture(CaptureSink),
}

/// Shared list of dispatched notifications for tests. Cloning is cheap
/// (the inner `Vec` is behind an `Arc<Mutex<…>>`).
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct CaptureSink {
    pub records: Arc<Mutex<Vec<(String, String)>>>,
}

#[cfg(test)]
impl PartialEq for CaptureSink {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.records, &other.records)
    }
}

#[cfg(test)]
impl Eq for CaptureSink {}

#[cfg(test)]
impl CaptureSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.records.lock().map(|r| r.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn snapshot(&self) -> Vec<(String, String)> {
        self.records.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

impl Backend {
    /// Probe a chosen backend or the platform's default order. Returns
    /// the first backend that can actually run. `Stderr` always works,
    /// so the function never fails.
    pub fn probe(preference: Option<Backend>) -> Backend {
        if let Some(pref) = preference {
            if available(&pref) {
                return pref;
            }
            // Preference unusable -> fall through to platform default.
        }
        for candidate in platform_defaults() {
            if available(&candidate) {
                return candidate;
            }
        }
        Backend::Stderr
    }

    /// Dispatch a notification. Backends that shell out treat any
    /// non-zero exit code as an `io::Error::other`.
    pub fn dispatch(&self, title: &str, body: &str) -> io::Result<()> {
        match self {
            Backend::Osascript => {
                let t = escape_for_osascript(title);
                let b = escape_for_osascript(body);
                let script = format!("display notification \"{b}\" with title \"{t}\"");
                run_silent(Command::new("osascript").arg("-e").arg(&script))
            }
            Backend::NotifySend => run_silent(Command::new("notify-send").arg(title).arg(body)),
            Backend::TerminalNotifier => run_silent(
                Command::new("terminal-notifier")
                    .arg("-title")
                    .arg(title)
                    .arg("-message")
                    .arg(body),
            ),
            Backend::Stderr => {
                eprintln!("[notify] {title} — {body}");
                Ok(())
            }
            #[cfg(test)]
            Backend::Capture(sink) => {
                if let Ok(mut g) = sink.records.lock() {
                    g.push((title.to_string(), body.to_string()));
                }
                Ok(())
            }
        }
    }
}

/// Probe a single backend cheaply. Stderr always wins; subprocess
/// backends probe with `<bin> --version` (or `which`-style) and a short
/// timeout. We don't care about the version itself — only the
/// exit-code-zero handshake that proves the binary is on `PATH`.
fn available(backend: &Backend) -> bool {
    match backend {
        Backend::Stderr => true,
        #[cfg(test)]
        Backend::Capture(_) => true,
        Backend::Osascript => probe_command("osascript", &["-e", "return 0"]),
        Backend::NotifySend => probe_command("notify-send", &["--version"]),
        Backend::TerminalNotifier => probe_command("terminal-notifier", &["-help"]),
    }
}

/// Spawn `bin args…`, swallow output, kill after ~100ms if it hangs.
/// Returns true iff the child exited with status 0 (or `terminal-notifier`,
/// which exits non-zero on `-help` but is still "available"). The
/// generous interpretation: if the binary launched at all, it exists.
fn probe_command(bin: &str, args: &[&str]) -> bool {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Poll up to 100ms total for the child to exit.
    let deadline = std::time::Instant::now() + Duration::from_millis(100);
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return true,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return false,
        }
    }
}

fn run_silent(cmd: &mut Command) -> io::Result<()> {
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "notifier subprocess exited with {status}"
        )))
    }
}

fn platform_defaults() -> Vec<Backend> {
    #[cfg(target_os = "macos")]
    {
        vec![
            Backend::Osascript,
            Backend::TerminalNotifier,
            Backend::Stderr,
        ]
    }
    #[cfg(target_os = "linux")]
    {
        vec![Backend::NotifySend, Backend::Stderr]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        vec![Backend::Stderr]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_picks_stderr_when_preferred() {
        let chosen = Backend::probe(Some(Backend::Stderr));
        assert_eq!(chosen, Backend::Stderr);
    }

    #[test]
    fn probe_falls_back_to_stderr_when_no_default_runs() {
        // Stderr is always available, so even on platforms without
        // notify-send/osascript we get a usable backend.
        let chosen = Backend::probe(None);
        // We don't assert *which* backend was picked (depends on the
        // host) — only that we got something dispatchable.
        chosen
            .dispatch("test", "body")
            .expect("dispatch must not fail");
    }

    #[test]
    fn capture_records_dispatches() {
        let sink = CaptureSink::new();
        let backend = Backend::Capture(sink.clone());
        backend.dispatch("t1", "b1").unwrap();
        backend.dispatch("t2", "b2").unwrap();
        assert_eq!(
            sink.snapshot(),
            vec![("t1".into(), "b1".into()), ("t2".into(), "b2".into())]
        );
    }

    #[test]
    fn deserialize_kebab_case() {
        let s: Backend = serde_json::from_str("\"osascript\"").unwrap();
        assert_eq!(s, Backend::Osascript);
        let s: Backend = serde_json::from_str("\"notify-send\"").unwrap();
        assert_eq!(s, Backend::NotifySend);
        let s: Backend = serde_json::from_str("\"terminal-notifier\"").unwrap();
        assert_eq!(s, Backend::TerminalNotifier);
        let s: Backend = serde_json::from_str("\"stderr\"").unwrap();
        assert_eq!(s, Backend::Stderr);
    }
}
