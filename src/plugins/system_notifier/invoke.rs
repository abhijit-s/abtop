//! Conduit invocation layer for the System Notifier plugin.
//!
//! Builds the curated env-var set (`ABTOP_TITLE`, `ABTOP_BODY`,
//! `ABTOP_EVENT_TYPE`, `ABTOP_TS_MS`, plus `ABTOP_FIELD_<KEY>` for
//! each top-level event field), spawns the user's conduit binary,
//! pipes the full [`WireRecord`] JSON on stdin, and enforces a
//! per-invocation wall-clock timeout (default 5s, configurable via
//! `conduit_timeout_ms`).
//!
//! The [`Invoker`] trait splits real subprocess spawning ([`RealInvoker`])
//! from a test-only [`CaptureInvoker`] that records calls in memory —
//! lets the worker tests in S4 stay fast and deterministic without
//! shelling out.

use crate::event_config::interpolation::expand;
use crate::events::WireRecord;
use crate::plugins::system_notifier::config::SystemNotifierConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};

/// Hard cap on the size of any `ABTOP_FIELD_*` env var. Some platforms
/// cap a single env var around 128 KiB and the total env at ~1 MiB —
/// the full payload is still on stdin, so truncation here only affects
/// the convenience shortcut, never the lossless path.
const MAX_FIELD_ENV_BYTES: usize = 4 * 1024;

/// One conduit invocation, prepared by the worker. Holds the bytes
/// the invocation thread needs and nothing more — once a request is
/// in flight the worker doesn't touch the config again.
#[derive(Clone, Debug)]
pub struct InvokeRequest {
    /// Resolved conduit path (after `~` / `${VAR}` expansion).
    pub conduit: String,
    pub conduit_args: Vec<String>,
    /// Rendered title (post-template substitution).
    pub title: String,
    /// Rendered body.
    pub body: String,
    /// Full event record — serialized to JSON for stdin AND walked for
    /// `ABTOP_FIELD_*` env vars.
    pub record: WireRecord,
    /// Per-invocation wall-clock timeout.
    pub timeout_ms: u64,
}

/// Result of an invocation. The worker logs failures (rate-limited)
/// but never retries — drop semantics are intentional.
#[derive(Debug)]
pub enum InvokeError {
    /// Subprocess didn't return inside `timeout_ms`. Child was killed
    /// and reaped before this error returned.
    Timeout,
    /// Subprocess exited with a non-zero status. Tail of stderr is
    /// captured for the log.
    NonZeroExit { status: i32, stderr_tail: String },
    /// `std::process::Command::spawn` failed — most likely the conduit
    /// path doesn't exist or isn't executable.
    SpawnFailed(std::io::Error),
    /// Writing the JSON payload to stdin failed.
    StdinWriteFailed(std::io::Error),
}

impl std::fmt::Display for InvokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvokeError::Timeout => write!(f, "conduit timed out"),
            InvokeError::NonZeroExit {
                status,
                stderr_tail,
            } => {
                if stderr_tail.is_empty() {
                    write!(f, "conduit exited with status {status}")
                } else {
                    write!(f, "conduit exited with status {status}: {stderr_tail}")
                }
            }
            InvokeError::SpawnFailed(e) => write!(f, "conduit spawn failed: {e}"),
            InvokeError::StdinWriteFailed(e) => write!(f, "conduit stdin write failed: {e}"),
        }
    }
}

impl std::error::Error for InvokeError {}

/// Construct an [`InvokeRequest`] for `rec` from the current config
/// snapshot. The renderer for `title` / `body` lives in
/// [`crate::plugins::common::template`]; the worker passes the
/// already-rendered strings via this helper so the invocation thread
/// never holds template state.
pub fn build_request(
    cfg: &SystemNotifierConfig,
    rec: &WireRecord,
    title: String,
    body: String,
) -> InvokeRequest {
    InvokeRequest {
        // expand() returns None when interpolation fails (e.g. an
        // unresolvable `${VAR}`); fall back to the raw string in that
        // case so the spawn error surfaces the actual problem.
        conduit: expand(&cfg.conduit).unwrap_or_else(|| cfg.conduit.clone()),
        conduit_args: cfg.conduit_args.clone(),
        title,
        body,
        record: rec.clone(),
        timeout_ms: cfg.conduit_timeout_ms,
    }
}

/// Pluggable invocation backend. Real builds use [`RealInvoker`]; the
/// worker tests in S4 substitute [`CaptureInvoker`] to record calls
/// without shelling out.
pub trait Invoker: Send + Sync + 'static {
    fn invoke(&self, req: &InvokeRequest) -> Result<(), InvokeError>;
}

/// Default invoker — spawns the conduit as a subprocess.
#[derive(Default, Debug, Clone, Copy)]
pub struct RealInvoker;

impl Invoker for RealInvoker {
    fn invoke(&self, req: &InvokeRequest) -> Result<(), InvokeError> {
        invoke_real(req)
    }
}

/// Spawn the conduit subprocess, set curated env vars, pipe the
/// `WireRecord` JSON on stdin, and wait up to `timeout_ms` for exit.
pub fn invoke_real(req: &InvokeRequest) -> Result<(), InvokeError> {
    use std::process::{Command, Stdio};

    let env_vars = build_env(&req.title, &req.body, &req.record);
    let payload = serde_json::to_vec(&req.record).unwrap_or_else(|_| b"{}".to_vec());

    let mut cmd = Command::new(&req.conduit);
    cmd.args(&req.conduit_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (k, v) in &env_vars {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().map_err(InvokeError::SpawnFailed)?;

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(&payload) {
            // Reap so we don't leave a zombie.
            let _ = child.kill();
            let _ = child.wait();
            return Err(InvokeError::StdinWriteFailed(e));
        }
        // Append a trailing newline — convenient for line-buffered
        // conduits (e.g. `read line` in shell).
        let _ = stdin.write_all(b"\n");
        // Dropping `stdin` closes it; explicit for clarity.
        drop(stdin);
    }

    let timeout = Duration::from_millis(req.timeout_ms);
    let status = match wait_with_deadline(&mut child, timeout) {
        Some(s) => s,
        None => {
            // Timeout: kill + reap, then surface the error.
            let _ = child.kill();
            let _ = child.wait();
            return Err(InvokeError::Timeout);
        }
    };

    if status.success() {
        return Ok(());
    }

    // Read whatever stderr produced before exit (best-effort).
    let stderr_tail = if let Some(mut err) = child.stderr.take() {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        let s = String::from_utf8_lossy(&buf).to_string();
        let trimmed = s.trim();
        // Cap the tail to keep the log line bounded.
        if trimmed.len() > 256 {
            format!("{}…", &trimmed[..256])
        } else {
            trimmed.to_string()
        }
    } else {
        String::new()
    };

    Err(InvokeError::NonZeroExit {
        status: status.code().unwrap_or(-1),
        stderr_tail,
    })
}

/// Poll `child.try_wait()` until exit or deadline. Returns `Some(status)`
/// on exit, `None` on timeout. 25ms poll interval — plenty accurate
/// for the 5-second default timeout, no extra crate needed.
fn wait_with_deadline(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    let step = Duration::from_millis(25);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            // try_wait failing is treated as "not yet" — we'll either
            // hit the deadline or eventually succeed.
            Err(_) => {}
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let remaining = deadline - now;
        std::thread::sleep(remaining.min(step));
    }
}

/// Build the curated env-var set. Returns a sorted map so the output
/// is deterministic in tests.
fn build_env(title: &str, body: &str, rec: &WireRecord) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    env.insert("ABTOP_TITLE".to_string(), title.to_string());
    env.insert("ABTOP_BODY".to_string(), body.to_string());
    env.insert(
        "ABTOP_EVENT_TYPE".to_string(),
        rec.event.type_name().to_string(),
    );
    env.insert("ABTOP_TS_MS".to_string(), rec.ts_ms.to_string());

    // Walk the serialized event for top-level fields. `WireRecord`
    // flattens the event, so its serialized object already contains
    // `v`, `ts_ms`, `type`, and the variant's fields at the top level.
    // We skip `v`, `ts_ms`, and `type` (already exposed above) and
    // surface the variant's own fields as `ABTOP_FIELD_<UPPER_KEY>`.
    if let Ok(Value::Object(obj)) = serde_json::to_value(rec) {
        for (k, v) in obj {
            if matches!(k.as_str(), "v" | "ts_ms" | "type") {
                continue;
            }
            let stringified = match &v {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                other => other.to_string(),
            };
            let truncated = truncate_to(&stringified, MAX_FIELD_ENV_BYTES);
            let key = format!("ABTOP_FIELD_{}", k.to_uppercase());
            env.insert(key, truncated);
        }
    }
    env
}

/// Truncate `s` to at most `max_bytes`, snipping at a UTF-8 boundary.
fn truncate_to(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut cut = max_bytes;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s[..cut].to_string()
}

/// Test-only invoker that records every call instead of shelling out.
/// Used by S4's worker tests.
#[cfg(test)]
#[derive(Default, Clone)]
pub struct CaptureInvoker {
    inner: std::sync::Arc<std::sync::Mutex<Vec<InvokeRequest>>>,
}

#[cfg(test)]
impl CaptureInvoker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<InvokeRequest> {
        self.inner.lock().expect("capture invoker lock").clone()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().expect("capture invoker lock").len()
    }
}

#[cfg(test)]
impl Invoker for CaptureInvoker {
    fn invoke(&self, req: &InvokeRequest) -> Result<(), InvokeError> {
        self.inner
            .lock()
            .expect("capture invoker lock")
            .push(req.clone());
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::events::AppEvent;
    use std::path::PathBuf;

    fn tmp_path(label: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.join(format!("ab-si-{label}-{pid}-{nanos}"))
    }

    fn sample_rec() -> WireRecord {
        WireRecord::new(
            AppEvent::StatusChanged {
                session_id: "abc".to_string(),
                from: crate::model::SessionStatus::Thinking,
                to: crate::model::SessionStatus::Executing,
            },
            42,
        )
    }

    fn rec_with_arg(arg: String) -> WireRecord {
        WireRecord::new(
            AppEvent::ToolCalled {
                session_id: "s".to_string(),
                tool: "Read".to_string(),
                arg,
            },
            7,
        )
    }

    fn write_script(path: &PathBuf, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, body).expect("write script");
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }

    fn req(conduit: &str, timeout_ms: u64) -> InvokeRequest {
        InvokeRequest {
            conduit: conduit.to_string(),
            conduit_args: vec![],
            title: "T".to_string(),
            body: "B".to_string(),
            record: sample_rec(),
            timeout_ms,
        }
    }

    #[test]
    fn build_env_includes_curated_keys() {
        let rec = sample_rec();
        let env = build_env("hello", "world", &rec);
        assert_eq!(env.get("ABTOP_TITLE").map(|s| s.as_str()), Some("hello"));
        assert_eq!(env.get("ABTOP_BODY").map(|s| s.as_str()), Some("world"));
        assert_eq!(
            env.get("ABTOP_EVENT_TYPE").map(|s| s.as_str()),
            Some("StatusChanged")
        );
        assert_eq!(env.get("ABTOP_TS_MS").map(|s| s.as_str()), Some("42"));
        assert_eq!(
            env.get("ABTOP_FIELD_SESSION_ID").map(|s| s.as_str()),
            Some("abc")
        );
        assert_eq!(
            env.get("ABTOP_FIELD_FROM").map(|s| s.as_str()),
            Some("Thinking")
        );
        // We must NOT leak the wire-level fields as ABTOP_FIELD_*.
        assert!(env.get("ABTOP_FIELD_V").is_none());
        assert!(env.get("ABTOP_FIELD_TS_MS").is_none());
        assert!(env.get("ABTOP_FIELD_TYPE").is_none());
    }

    #[test]
    fn build_env_truncates_oversized_field() {
        // 8 KiB arg; ABTOP_FIELD_ARG must end up <= 4 KiB.
        let big = "x".repeat(8 * 1024);
        let rec = rec_with_arg(big);
        let env = build_env("t", "b", &rec);
        let arg = env.get("ABTOP_FIELD_ARG").expect("ARG env present");
        assert!(arg.len() <= MAX_FIELD_ENV_BYTES, "got len {}", arg.len());
    }

    #[test]
    fn invoke_writes_json_to_stdin() {
        let out = tmp_path("stdin");
        let script = tmp_path("stdin-script.sh");
        write_script(&script, &format!("#!/bin/sh\ncat > {}\n", out.display()));
        let res = invoke_real(&req(script.to_str().unwrap(), 5_000));
        assert!(res.is_ok(), "invoke_real failed: {res:?}");
        let captured = std::fs::read_to_string(&out).expect("read stdin capture");
        // Strip trailing newline appended by invoke_real.
        let trimmed = captured.trim_end_matches('\n');
        let parsed: serde_json::Value =
            serde_json::from_str(trimmed).expect("captured stdin is valid JSON");
        assert_eq!(
            parsed.get("type").and_then(|v| v.as_str()),
            Some("StatusChanged")
        );
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn invoke_sets_env_vars() {
        let out = tmp_path("env");
        let script = tmp_path("env-script.sh");
        write_script(
            &script,
            &format!(
                "#!/bin/sh\nenv | grep '^ABTOP_' | sort > {}\n",
                out.display()
            ),
        );
        let mut r = req(script.to_str().unwrap(), 5_000);
        r.title = "the-title".to_string();
        r.body = "the-body".to_string();
        let res = invoke_real(&r);
        assert!(res.is_ok(), "invoke_real failed: {res:?}");
        let captured = std::fs::read_to_string(&out).expect("read env capture");
        assert!(
            captured.contains("ABTOP_TITLE=the-title"),
            "got:\n{captured}"
        );
        assert!(captured.contains("ABTOP_BODY=the-body"));
        assert!(captured.contains("ABTOP_EVENT_TYPE=StatusChanged"));
        assert!(captured.contains("ABTOP_TS_MS=42"));
        assert!(captured.contains("ABTOP_FIELD_SESSION_ID=abc"));
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn invoke_returns_nonzero_on_failure() {
        let script = tmp_path("nz-script.sh");
        write_script(&script, "#!/bin/sh\nexit 7\n");
        let res = invoke_real(&req(script.to_str().unwrap(), 5_000));
        match res {
            Err(InvokeError::NonZeroExit { status, .. }) => assert_eq!(status, 7),
            other => panic!("expected NonZeroExit(7), got {other:?}"),
        }
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn invoke_timeout_kills_runaway() {
        let script = tmp_path("to-script.sh");
        write_script(&script, "#!/bin/sh\nsleep 10\n");
        let start = Instant::now();
        let res = invoke_real(&req(script.to_str().unwrap(), 200));
        let elapsed = start.elapsed();
        match res {
            Err(InvokeError::Timeout) => {}
            other => panic!("expected Timeout, got {other:?}"),
        }
        // Should return within ~500ms (200ms timeout + reap slack), not 10s.
        assert!(
            elapsed < Duration::from_millis(2_000),
            "timeout took too long: {elapsed:?}"
        );
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn invoke_spawn_failed_on_missing_path() {
        let res = invoke_real(&req(
            "/nonexistent/path/abtop-test-conduit-doesnt-exist",
            500,
        ));
        match res {
            Err(InvokeError::SpawnFailed(_)) => {}
            other => panic!("expected SpawnFailed, got {other:?}"),
        }
    }

    #[test]
    fn capture_invoker_records_requests() {
        let inv = CaptureInvoker::new();
        let r = req("/bin/true", 1_000);
        inv.invoke(&r).expect("capture invoker never fails");
        inv.invoke(&r).expect("capture invoker never fails");
        assert_eq!(inv.len(), 2);
        let snap = inv.snapshot();
        assert_eq!(snap[0].title, "T");
        assert_eq!(snap[1].body, "B");
    }

    #[test]
    fn build_request_expands_tilde_in_conduit_path() {
        let cfg = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: "~/notify.sh".to_string(),
            ..Default::default()
        };
        let req = build_request(&cfg, &sample_rec(), "t".into(), "b".into());
        // expand() expands `~` against $HOME — the resulting path must
        // no longer start with a literal tilde (assuming HOME is set,
        // which it is in any reasonable test env).
        assert!(
            !req.conduit.starts_with('~'),
            "tilde should be expanded, got {}",
            req.conduit
        );
    }
}
