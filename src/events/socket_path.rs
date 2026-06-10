//! Resolve and validate the on-disk path used for the events Unix socket.
//!
//! Resolution order (highest precedence first):
//!
//! 1. Explicit override (CLI flag or `ABTOP_EVENTS_SOCKET` env var).
//! 2. `$XDG_RUNTIME_DIR/abtop.sock` — Linux, when set.
//! 3. `$TMPDIR/abtop.sock` — macOS default.
//! 4. `/tmp/abtop-$UID.sock` — universal fallback.
//!
//! BSD/macOS caps `sun_path` at 104 chars. [`validate_sun_path_length`]
//! returns a clear error string when the resolved path would overflow.

use std::env;
use std::path::PathBuf;

/// macOS / BSD `sun_path` length limit. Linux is 108 — we use the
/// stricter value so the same path works on every supported OS.
pub const SUN_PATH_MAX: usize = 104;

/// Compute the default events socket path for this process.
///
/// `override_path` short-circuits resolution. Otherwise the environment
/// is inspected — `ABTOP_EVENTS_SOCKET` wins over XDG/TMPDIR/UID
/// fallbacks.
pub fn resolve(override_path: Option<&str>) -> PathBuf {
    if let Some(p) = override_path {
        return PathBuf::from(p);
    }
    if let Ok(p) = env::var("ABTOP_EVENTS_SOCKET") {
        return PathBuf::from(p);
    }
    if let Ok(dir) = env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("abtop.sock");
        }
    }
    if let Ok(dir) = env::var("TMPDIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("abtop.sock");
        }
    }
    let uid = current_uid();
    PathBuf::from(format!("/tmp/abtop-{uid}.sock"))
}

/// Best-effort current UID. Falls back to 0 when the lookup fails or
/// on non-Unix targets. The value is only used as a fragment of a
/// per-user socket filename, so a wrong value just produces a less
/// collision-resistant path — not a security issue.
fn current_uid() -> u32 {
    if let Ok(uid) = env::var("UID") {
        if let Ok(n) = uid.parse() {
            return n;
        }
    }
    // SUDO_UID / LOGNAME fallbacks are noise; the `id -u` shellout is
    // cheap and works on every Unix.
    #[cfg(unix)]
    {
        if let Ok(output) = std::process::Command::new("id").arg("-u").output() {
            if let Ok(s) = std::str::from_utf8(&output.stdout) {
                if let Ok(n) = s.trim().parse() {
                    return n;
                }
            }
        }
    }
    0
}

/// Returns `Err` if the path's bytes would not fit in a `sockaddr_un.sun_path`.
/// The error message names the env var the user can set to escape the limit.
pub fn validate_sun_path_length(path: &std::path::Path) -> Result<(), String> {
    let bytes = path.to_string_lossy();
    if bytes.len() >= SUN_PATH_MAX {
        return Err(format!(
            "socket path too long ({} >= {}). Set ABTOP_EVENTS_SOCKET to a shorter path.",
            bytes.len(),
            SUN_PATH_MAX
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn override_wins_over_env() {
        let p = resolve(Some("/explicit/path.sock"));
        assert_eq!(p, PathBuf::from("/explicit/path.sock"));
    }

    #[test]
    fn validates_path_length() {
        let short = Path::new("/tmp/x.sock");
        assert!(validate_sun_path_length(short).is_ok());
        let long: String = "/tmp/".chars().chain(std::iter::repeat('a').take(150)).collect();
        let err = validate_sun_path_length(Path::new(&long)).unwrap_err();
        assert!(err.contains("ABTOP_EVENTS_SOCKET"));
    }

    #[test]
    fn fallback_includes_uid() {
        // Without env vars set, fallback is /tmp/abtop-$UID.sock.
        // We don't unset env here to avoid races with parallel tests;
        // just verify the format if no XDG/TMPDIR are present.
        std::env::remove_var("ABTOP_EVENTS_SOCKET");
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var("TMPDIR");
        let p = resolve(None);
        let s = p.to_string_lossy().to_string();
        assert!(s.starts_with("/tmp/abtop-"));
        assert!(s.ends_with(".sock"));
    }
}
