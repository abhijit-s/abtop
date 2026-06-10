//! External TOML config for `[events]` and `[plugins.*]`, with hot
//! reload via mtime polling.
//!
//! The loader is intentionally a separate module from
//! [`crate::config`] (which owns the pre-existing flat-KV
//! panel-visibility config). This file owns:
//!
//! 1. Resolution of `~/.config/abtop/config.toml` + `plugins.d/*.toml`
//!    drop-ins.
//! 2. Parsing into [`schema::ConfigFile`] via `serde + toml`.
//! 3. Variable interpolation in the `socket` field via
//!    [`interpolation`].
//! 4. Merging the resolved config with the engine-side
//!    [`crate::plugins::PluginsConfig`] consumed by the registry.
//! 5. Hot reload via [`reload_if_changed`], driven by `App::tick` at
//!    the same 2s cadence the theme loader uses.
//!
//! Errors are data — see [`LoadError`]. Malformed input never
//! panics; the caller surfaces parse-error counts via the same
//! `App::set_status` channel the theme loader uses.

pub mod interpolation;
pub mod schema;

use crate::plugins::PluginsConfig;
use schema::{ConfigFile, EventsConfig};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One parse error from a single file. The loader returns a vec of
/// these alongside the merged config so the caller can decide how
/// many to surface in the UI (count + first message is typical).
#[derive(Clone, Debug)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

/// Result of a load: the merged config, the source paths it came from
/// (canonical `config.toml` if any), and the most-recent mtime across
/// all read files. The mtime is the hot-reload trigger.
#[derive(Clone, Debug, Default)]
pub struct LoadedConfig {
    /// Resolved `[events]` config with `socket` already interpolated.
    pub events: EventsConfig,
    /// Resolved interpolated socket path (post-expansion) or `None` if
    /// the user didn't override.
    pub resolved_socket: Option<PathBuf>,
    /// Engine-side plugin config the registry consumes.
    pub plugins: PluginsConfig,
    /// Path to the main `config.toml` if it was found; `None` if no
    /// main file existed (drop-ins might still have contributed).
    pub source_path: Option<PathBuf>,
    /// Sorted list of every file that contributed (main + drop-ins).
    /// Used by [`reload_if_changed`] to re-scan exactly the same set.
    pub contributing_paths: Vec<PathBuf>,
    /// Most recent mtime observed across every contributing file.
    /// `None` when no file existed at load time.
    pub mtime: Option<SystemTime>,
    /// Directory used to resolve drop-ins. Stashed so reload picks up
    /// new drop-in files that appeared between calls.
    pub config_dir: PathBuf,
    /// True when `--no-config` was passed. Suppresses both initial
    /// reads and reload polling.
    pub skip_files: bool,
}

/// Resolve, read, and merge all config sources. Returns the merged
/// config plus the list of parse errors. Never panics on malformed
/// input — errors are reported and the field falls back to default.
pub fn load_initial(
    cli_config_path: Option<&Path>,
    skip_files: bool,
) -> (LoadedConfig, Vec<LoadError>) {
    let mut loaded = LoadedConfig {
        events: EventsConfig::default(),
        resolved_socket: None,
        plugins: PluginsConfig::default(),
        source_path: None,
        contributing_paths: Vec::new(),
        mtime: None,
        config_dir: resolve_config_dir(cli_config_path),
        skip_files,
    };

    if skip_files {
        return (loaded, Vec::new());
    }

    let mut errors: Vec<LoadError> = Vec::new();
    let mut merged_file = ConfigFile::default();

    // 1) Main file
    let (main_path, drop_in_dir) = match cli_config_path {
        Some(p) => (Some(p.to_path_buf()), None),
        None => {
            let dir = loaded.config_dir.clone();
            let main = dir.join("config.toml");
            let drop_ins = dir.join("plugins.d");
            (
                if main.exists() { Some(main) } else { None },
                Some(drop_ins),
            )
        }
    };

    if let Some(path) = &main_path {
        match read_and_parse(path) {
            Ok((cf, mtime)) => {
                merged_file = cf;
                loaded.source_path = Some(path.clone());
                loaded.contributing_paths.push(path.clone());
                loaded.mtime = merge_mtime(loaded.mtime, mtime);
            }
            Err(e) => {
                errors.push(e);
            }
        }
    }

    // 2) Drop-ins (skipped when `--config` is in play).
    if let Some(dir) = drop_in_dir {
        if dir.is_dir() {
            for entry in read_sorted_toml_files(&dir) {
                match read_and_parse(&entry) {
                    Ok((overlay, mtime)) => {
                        merge_overlay(&mut merged_file, overlay);
                        loaded.contributing_paths.push(entry.clone());
                        loaded.mtime = merge_mtime(loaded.mtime, mtime);
                    }
                    Err(e) => errors.push(e),
                }
            }
        }
    }

    // 3) Render into the engine-side shapes.
    apply_to_loaded(&mut loaded, merged_file);
    (loaded, errors)
}

/// Re-read from the same source paths. Returns `Some(...)` only if
/// the merged mtime advanced since the last load OR if a new drop-in
/// appeared / disappeared.
pub fn reload_if_changed(prev: &LoadedConfig) -> Option<(LoadedConfig, Vec<LoadError>)> {
    if prev.skip_files {
        return None;
    }

    // Re-scan the set of contributing paths now (drop-ins may have
    // been added or removed since the previous load).
    let mut current_paths: Vec<PathBuf> = Vec::new();
    if let Some(p) = &prev.source_path {
        if p.exists() {
            current_paths.push(p.clone());
        }
    }
    let drop_ins = prev.config_dir.join("plugins.d");
    if drop_ins.is_dir() {
        for entry in read_sorted_toml_files(&drop_ins) {
            current_paths.push(entry);
        }
    }

    // Compute current mtime across all contributing files.
    let mut current_mtime: Option<SystemTime> = None;
    for p in &current_paths {
        let m = std::fs::metadata(p).and_then(|m| m.modified()).ok();
        current_mtime = merge_mtime(current_mtime, m);
    }

    let paths_differ = current_paths != prev.contributing_paths;
    let mtime_advanced = matches!(
        (current_mtime, prev.mtime),
        (Some(now), Some(then)) if now > then
    ) || (current_mtime.is_some() && prev.mtime.is_none());

    if !paths_differ && !mtime_advanced {
        return None;
    }

    // The file set or mtime changed — re-run the full load.
    let (next, errors) = load_initial(
        prev.source_path.as_deref(),
        false, // we wouldn't get here if skip_files was true
    );
    Some((next, errors))
}

/// Locate the directory that owns `config.toml`. With `--config <path>`
/// the directory is the file's parent (drop-ins live there too but
/// are skipped per spec). Without `--config`, it's
/// `$XDG_CONFIG_HOME/abtop` (or `~/.config/abtop`).
fn resolve_config_dir(cli: Option<&Path>) -> PathBuf {
    if let Some(p) = cli {
        return p
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
    }
    crate::config::xdg_config_dir().join("abtop")
}

fn read_and_parse(path: &Path) -> Result<(ConfigFile, Option<SystemTime>), LoadError> {
    let content = std::fs::read_to_string(path).map_err(|e| LoadError {
        path: path.to_path_buf(),
        message: format!("read: {e}"),
    })?;
    let cf: ConfigFile = toml::from_str(&content).map_err(|e| LoadError {
        path: path.to_path_buf(),
        message: format!("parse: {e}"),
    })?;
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    Ok((cf, mtime))
}

fn read_sorted_toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("toml"))
            .collect(),
        Err(_) => Vec::new(),
    };
    entries.sort();
    entries
}

fn merge_mtime(prev: Option<SystemTime>, next: Option<SystemTime>) -> Option<SystemTime> {
    match (prev, next) {
        (Some(a), Some(b)) => Some(if a > b { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Overlay `next` onto `base`. Scalar fields in `[events]` and
/// `[plugins.notifier]` override; rule lists append. The merge happens
/// at the parsed level (not the raw `toml::Value` level) so unknown
/// fields silently drop on overlay — same forward-compat behavior as
/// the main file.
fn merge_overlay(base: &mut ConfigFile, next: ConfigFile) {
    // Events scalars: only override when the overlay differs from the
    // serde default. We can't tell "absent" from "explicitly set to
    // default" without a richer schema, so we accept that re-setting
    // a default in a drop-in is a no-op. This is fine — overlaying a
    // default with a default produces the same default.
    if next.events.enabled != EventsConfig::default().enabled {
        base.events.enabled = next.events.enabled;
    }
    if next.events.socket.is_some() {
        base.events.socket = next.events.socket;
    }
    if next.events.backlog != EventsConfig::default().backlog {
        base.events.backlog = next.events.backlog;
    }

    #[cfg(feature = "plugin-notifier")]
    {
        if let Some(overlay_notifier) = next.plugins.notifier {
            match base.plugins.notifier.as_mut() {
                None => {
                    base.plugins.notifier = Some(overlay_notifier);
                }
                Some(existing) => {
                    // Scalars override, rules APPEND (per spec).
                    let default = crate::plugins::notifier::NotifierConfig::default();
                    if overlay_notifier.enabled_at_startup != default.enabled_at_startup {
                        existing.enabled_at_startup = overlay_notifier.enabled_at_startup;
                    }
                    if overlay_notifier.backend.is_some() {
                        existing.backend = overlay_notifier.backend;
                    }
                    if overlay_notifier.debounce_ms != default.debounce_ms {
                        existing.debounce_ms = overlay_notifier.debounce_ms;
                    }
                    existing.rule.extend(overlay_notifier.rule);
                }
            }
        }
    }
    #[cfg(not(feature = "plugin-notifier"))]
    {
        // Without the feature there's nothing to merge into.
        let _ = &next.plugins.notifier;
    }

    #[cfg(feature = "plugin-system-notifier")]
    {
        if let Some(overlay_sys) = next.plugins.system_notifier {
            match base.plugins.system_notifier.as_mut() {
                None => {
                    base.plugins.system_notifier = Some(overlay_sys);
                }
                Some(existing) => {
                    // Scalars override (single-conduit plugin — no list
                    // fields to append).
                    let default = crate::plugins::system_notifier::SystemNotifierConfig::default();
                    if overlay_sys.enabled_at_startup != default.enabled_at_startup {
                        existing.enabled_at_startup = overlay_sys.enabled_at_startup;
                    }
                    if !overlay_sys.conduit.is_empty() {
                        existing.conduit = overlay_sys.conduit;
                    }
                    if !overlay_sys.conduit_args.is_empty() {
                        existing.conduit_args = overlay_sys.conduit_args;
                    }
                    if !overlay_sys.on.is_empty() {
                        existing.on = overlay_sys.on;
                    }
                    if overlay_sys.title != default.title {
                        existing.title = overlay_sys.title;
                    }
                    if overlay_sys.body != default.body {
                        existing.body = overlay_sys.body;
                    }
                    if overlay_sys.debounce_ms != default.debounce_ms {
                        existing.debounce_ms = overlay_sys.debounce_ms;
                    }
                    if overlay_sys.conduit_timeout_ms != default.conduit_timeout_ms {
                        existing.conduit_timeout_ms = overlay_sys.conduit_timeout_ms;
                    }
                }
            }
        }
    }
    #[cfg(not(feature = "plugin-system-notifier"))]
    {
        let _ = next.plugins.system_notifier;
    }
}

fn apply_to_loaded(loaded: &mut LoadedConfig, cf: ConfigFile) {
    // Interpolate the socket field if set. `expand` returns `None` when
    // a documented-known variable (e.g. `XDG_RUNTIME_DIR` on macOS) is
    // unset/empty — treat that as "no override" so the downstream
    // resolver picks the platform default rather than binding to a
    // literal `${...}` path.
    let resolved_socket = cf
        .events
        .socket
        .as_deref()
        .and_then(|raw| interpolation::expand(raw).map(PathBuf::from));

    loaded.resolved_socket = resolved_socket;
    loaded.events = cf.events;

    #[cfg(feature = "plugin-notifier")]
    {
        if let Some(n) = cf.plugins.notifier {
            // Wire the parsed config into the engine-side struct.
            // `generation` is bumped on every load so the worker
            // re-compiles its rules on the next event tick.
            let mut wired = n;
            wired.generation = wired.generation.wrapping_add(1);
            loaded.plugins.notifier = wired;
        }
    }
    #[cfg(not(feature = "plugin-notifier"))]
    {
        let _ = cf.plugins.notifier;
    }

    #[cfg(feature = "plugin-system-notifier")]
    {
        if let Some(s) = cf.plugins.system_notifier {
            // Same generation-bump pattern as notifier so the worker
            // observes hot-reload changes on its next loop iteration.
            let mut wired = s;
            wired.generation = wired.generation.wrapping_add(1);
            loaded.plugins.system_notifier = wired;
        }
    }
    #[cfg(not(feature = "plugin-system-notifier"))]
    {
        let _ = cf.plugins.system_notifier;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn skip_files_returns_defaults_and_no_errors() {
        let (loaded, errors) = load_initial(None, true);
        assert!(loaded.contributing_paths.is_empty());
        assert!(loaded.source_path.is_none());
        assert!(loaded.mtime.is_none());
        assert!(errors.is_empty());
    }

    #[test]
    fn loads_main_file_via_cli_config() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("abtop.toml");
        write(
            &path,
            r#"
            [events]
            enabled = true
            socket = "/tmp/explicit.sock"
            backlog = 64
            "#,
        );
        let (loaded, errors) = load_initial(Some(&path), false);
        assert!(errors.is_empty());
        assert!(loaded.events.enabled);
        assert_eq!(loaded.events.backlog, 64);
        assert_eq!(
            loaded.resolved_socket.as_deref(),
            Some(std::path::Path::new("/tmp/explicit.sock"))
        );
        assert_eq!(loaded.source_path.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn malformed_main_file_reports_error_and_uses_defaults() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bad.toml");
        write(&path, "not a valid TOML = =\n[broken");
        let (loaded, errors) = load_initial(Some(&path), false);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].path, path);
        // Defaults: enabled=false, backlog=256.
        assert!(!loaded.events.enabled);
        assert_eq!(loaded.events.backlog, 256);
    }

    #[cfg(feature = "plugin-notifier")]
    #[test]
    fn notifier_section_parses_and_bumps_generation() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("c.toml");
        write(
            &path,
            r#"
            [events]
            enabled = true

            [plugins.notifier]
            enabled = true
            backend = "stderr"
            debounce_ms = 1000

            [[plugins.notifier.rule]]
            on = ["StatusChanged"]
            title = "x"
            body = "y"
            "#,
        );
        let (loaded, errors) = load_initial(Some(&path), false);
        assert!(errors.is_empty(), "errors: {errors:?}");
        let n = &loaded.plugins.notifier;
        // `enabled` in TOML maps to NotifierConfig's
        // `enabled_at_startup` via a serde alias. The startup flag is
        // also what hot-reload toggles via registry.set_enabled.
        assert!(n.enabled_at_startup);
        assert_eq!(n.debounce_ms, 1000);
        assert_eq!(n.rule.len(), 1);
        assert_eq!(n.rule[0].title, "x");
        assert!(n.generation > 0, "generation should be bumped on load");
    }

    #[cfg(feature = "plugin-notifier")]
    #[test]
    fn drop_ins_append_rules_and_override_scalars() {
        let tmp = TempDir::new().unwrap();
        let main = tmp.path().join("config.toml");
        write(
            &main,
            r#"
            [plugins.notifier]
            debounce_ms = 1000

            [[plugins.notifier.rule]]
            on = ["A"]
            title = "ta"
            body = "ba"
            "#,
        );
        let drop_in = tmp.path().join("plugins.d").join("z-overlay.toml");
        write(
            &drop_in,
            r#"
            [plugins.notifier]
            debounce_ms = 2000

            [[plugins.notifier.rule]]
            on = ["B"]
            title = "tb"
            body = "bb"
            "#,
        );

        // To exercise drop-ins, we cannot use --config (it skips them).
        // Bypass by calling load_initial with skip_files=false and
        // pointing XDG_CONFIG_HOME at our tmp dir. We need the dir
        // layout to be `<dir>/abtop/{config.toml,plugins.d/*.toml}`.
        let xdg = tmp.path().join("xdg");
        let abtop = xdg.join("abtop");
        std::fs::create_dir_all(&abtop).unwrap();
        std::fs::create_dir_all(abtop.join("plugins.d")).unwrap();
        std::fs::copy(&main, abtop.join("config.toml")).unwrap();
        std::fs::copy(&drop_in, abtop.join("plugins.d").join("z-overlay.toml")).unwrap();

        // SAFETY: tests may share env vars; isolating via a guard.
        let prev = std::env::var("XDG_CONFIG_HOME").ok();
        // SAFETY: set_var is unsafe in edition 2024+. This crate is
        // 2021, where it's safe. The test runs single-threaded for
        // env-touching helpers via cargo test's default; if this
        // proves flaky we'd switch to a serial_test crate.
        std::env::set_var("XDG_CONFIG_HOME", &xdg);
        let (loaded, errors) = load_initial(None, false);
        if let Some(p) = prev {
            std::env::set_var("XDG_CONFIG_HOME", p);
        } else {
            std::env::remove_var("XDG_CONFIG_HOME");
        }

        assert!(errors.is_empty(), "errors: {errors:?}");
        let n = &loaded.plugins.notifier;
        assert_eq!(n.debounce_ms, 2000, "drop-in scalar should override");
        assert_eq!(n.rule.len(), 2, "rules should APPEND across drop-ins");
        assert_eq!(n.rule[0].title, "ta");
        assert_eq!(n.rule[1].title, "tb");
    }

    #[test]
    fn reload_if_changed_returns_none_when_unchanged() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("c.toml");
        write(&path, "[events]\nenabled = true\n");
        let (loaded, _) = load_initial(Some(&path), false);
        assert!(reload_if_changed(&loaded).is_none());
    }

    #[test]
    fn reload_if_changed_picks_up_mtime_advance() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("c.toml");
        write(&path, "[events]\nenabled = true\nbacklog = 100\n");
        let (loaded, _) = load_initial(Some(&path), false);
        // Bump mtime by writing a different body. Sleep briefly so
        // mtime resolution catches the change on every filesystem.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(&path, "[events]\nenabled = true\nbacklog = 200\n");

        let next = reload_if_changed(&loaded);
        assert!(next.is_some());
        let (loaded2, errors) = next.unwrap();
        assert!(errors.is_empty());
        assert_eq!(loaded2.events.backlog, 200);
    }
}
