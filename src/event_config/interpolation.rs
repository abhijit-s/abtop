//! Variable interpolation used by the `[events]` section of the config
//! file. Supports `${UID}`, `${XDG_RUNTIME_DIR}`, `${XDG_CONFIG_HOME}`,
//! `${TMPDIR}`, `${HOME}`, and a leading `~` (home-relative path).
//!
//! Fall-through semantics:
//! - **Known variable, unset or empty** → returns `None`. The caller
//!   drops the override and falls back to platform-default resolution.
//!   This prevents disasters like `socket = "${XDG_RUNTIME_DIR}/abtop.sock"`
//!   binding to a literal `${XDG_RUNTIME_DIR}/` directory in the cwd
//!   on macOS, where the variable isn't set.
//! - **Unknown variable** → left verbatim in the returned string. The
//!   downstream bind step catches the unresolved `${...}` and refuses
//!   to bind, surfacing a clear error rather than creating literal-named
//!   directories on disk.

use std::path::PathBuf;

/// Names this module recognises. Unset/empty values for these trigger
/// a `None` return so the caller knows to use platform defaults.
const KNOWN_VARS: &[&str] = &[
    "UID",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "TMPDIR",
    "HOME",
];

fn is_known(name: &str) -> bool {
    KNOWN_VARS.contains(&name)
}

/// Expand `${VAR}` placeholders and a leading `~/` in `input`.
///
/// Returns `Some(expanded)` on success, or `None` when a documented
/// known variable is unset/empty (signalling "fall through to default").
/// Unknown variables stay in the returned string verbatim — the bind
/// step is responsible for rejecting any residual `${...}`.
pub fn expand_with<F, G, H>(input: &str, env: F, uid: G, home: H) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
    G: Fn() -> Option<String>,
    H: Fn() -> Option<PathBuf>,
{
    let with_tilde = expand_tilde(input, &home);
    let mut out = String::with_capacity(with_tilde.len());
    let bytes = with_tilde.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = with_tilde[i + 2..].find('}') {
                let name = &with_tilde[i + 2..i + 2 + end];
                let replacement = match name {
                    "UID" => uid(),
                    other => env(other),
                };
                match replacement {
                    Some(v) if !v.is_empty() => out.push_str(&v),
                    _ => {
                        if is_known(name) {
                            // Documented-known but unset/empty: tell
                            // the caller to fall through to defaults.
                            return None;
                        }
                        // Unknown var: leave verbatim. Bind-time will
                        // catch any residual `${...}`.
                        out.push_str(&with_tilde[i..i + 3 + end]);
                    }
                }
                i = i + 3 + end;
                continue;
            }
        }
        out.push(with_tilde[i..].chars().next().unwrap());
        i += with_tilde[i..].chars().next().unwrap().len_utf8();
    }
    Some(out)
}

fn expand_tilde<H>(input: &str, home: &H) -> String
where
    H: Fn() -> Option<PathBuf>,
{
    if let Some(rest) = input.strip_prefix("~/") {
        if let Some(h) = home() {
            return h.join(rest).to_string_lossy().into_owned();
        }
    }
    if input == "~" {
        if let Some(h) = home() {
            return h.to_string_lossy().into_owned();
        }
    }
    input.to_string()
}

/// Process-default expansion. Reads `${UID}` from the running process
/// (via `libc::getuid` on Linux, `id -u` on macOS) and the environment
/// for everything else.
///
/// Returns `None` when a documented-known variable is unset/empty so
/// the caller falls through to platform-default resolution.
pub fn expand(input: &str) -> Option<String> {
    expand_with(
        input,
        |k| std::env::var(k).ok(),
        current_uid_string,
        dirs::home_dir,
    )
}

#[cfg(unix)]
fn current_uid_string() -> Option<String> {
    // SAFETY: getuid() is always safe to call — it has no preconditions
    // and never fails. The `unsafe` block is only required because libc
    // declares it `extern "C"`. We already depend on `libc` on Linux;
    // on macOS we read uid via the `id -u` shell fallback to avoid
    // adding libc as a mac-only dep. The shell fallback keeps the
    // dependency tree the same on macOS while still working for
    // `${UID}` in the default config.
    #[cfg(target_os = "linux")]
    {
        Some(unsafe { libc::getuid() }.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        // macOS / BSD: shell out to `id -u`. Cached at process scope
        // would be nicer but the loader runs once per reload — and
        // reloads are at the 2s tick cadence at fastest, so the cost
        // is negligible.
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                } else {
                    None
                }
            })
    }
}

#[cfg(not(unix))]
fn current_uid_string() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        input: &str,
        env_pairs: &[(&str, &str)],
        uid: Option<&str>,
        home: Option<&str>,
    ) -> Option<String> {
        let env: std::collections::HashMap<String, String> = env_pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        expand_with(
            input,
            |k| env.get(k).cloned(),
            || uid.map(|s| s.to_string()),
            || home.map(PathBuf::from),
        )
    }

    #[test]
    fn passthrough_for_plain_strings() {
        assert_eq!(
            run("/tmp/abtop.sock", &[], Some("1000"), Some("/h")).as_deref(),
            Some("/tmp/abtop.sock")
        );
    }

    #[test]
    fn expands_uid() {
        assert_eq!(
            run("/tmp/abtop.${UID}.sock", &[], Some("501"), Some("/h")).as_deref(),
            Some("/tmp/abtop.501.sock")
        );
    }

    #[test]
    fn expands_xdg_runtime_dir() {
        assert_eq!(
            run(
                "${XDG_RUNTIME_DIR}/abtop.sock",
                &[("XDG_RUNTIME_DIR", "/run/user/501")],
                Some("501"),
                Some("/h"),
            )
            .as_deref(),
            Some("/run/user/501/abtop.sock")
        );
    }

    #[test]
    fn unset_known_var_returns_none() {
        // Known var, no env entry → caller falls through to default.
        // This is the macOS-without-XDG_RUNTIME_DIR case that previously
        // caused a literal `${XDG_RUNTIME_DIR}` directory to be created
        // on disk at bind time.
        assert!(run(
            "${XDG_RUNTIME_DIR}/abtop.sock",
            &[],
            Some("501"),
            Some("/h")
        )
        .is_none());
    }

    #[test]
    fn empty_known_var_also_returns_none() {
        // Explicitly-empty env value is treated the same as unset.
        assert!(run(
            "${XDG_RUNTIME_DIR}/abtop.sock",
            &[("XDG_RUNTIME_DIR", "")],
            Some("501"),
            Some("/h"),
        )
        .is_none());
    }

    #[test]
    fn unset_uid_returns_none() {
        // UID is treated as a known var: if `id -u` fails, fall through.
        assert!(run("/tmp/abtop.${UID}.sock", &[], None, Some("/h")).is_none());
    }

    #[test]
    fn unknown_variable_left_verbatim() {
        // Unknown vars stay literal so the bind step can refuse them
        // (catches typos like `${XGD_RUNTIME_DIR}`).
        assert_eq!(
            run("/tmp/${UNKNOWN}/x", &[], Some("0"), Some("/h")).as_deref(),
            Some("/tmp/${UNKNOWN}/x")
        );
    }

    #[test]
    fn expands_home_prefix() {
        assert_eq!(
            run("~/sockets/abtop.sock", &[], Some("0"), Some("/users/a")).as_deref(),
            Some("/users/a/sockets/abtop.sock")
        );
    }

    #[test]
    fn bare_tilde_expands() {
        assert_eq!(run("~", &[], Some("0"), Some("/h")).as_deref(), Some("/h"));
    }

    #[test]
    fn tilde_not_at_start_left_alone() {
        assert_eq!(
            run("/path/with~tilde", &[], Some("0"), Some("/h")).as_deref(),
            Some("/path/with~tilde")
        );
    }

    #[test]
    fn dangling_dollar_brace_left_alone() {
        // Pathological input: `${` with no closing `}`. We leave it.
        assert_eq!(
            run("/tmp/${UID/x", &[], Some("501"), Some("/h")).as_deref(),
            Some("/tmp/${UID/x")
        );
    }
}
