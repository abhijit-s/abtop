//! Configuration for the System Notifier plugin.
//!
//! Deserializes from the `[plugins.system_notifier]` TOML table. The
//! schema is intentionally single-conduit + flat — multi-conduit
//! routing belongs in the user's conduit script, not in this plugin
//! (see the ideation doc, Q2).
//!
//! Hot reload follows the same pattern as the Notifier:
//! [`SharedSystemNotifierConfig`] is an `Arc<RwLock<_>>` that the
//! config loader writes through, bumping `generation` so the worker
//! observes the change on its next iteration.

use serde::Deserialize;
use std::sync::{Arc, RwLock};

const DEFAULT_DEBOUNCE_MS: u64 = 5_000;
const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_TITLE: &str = "{kind}";

/// Shared, mutable handle to a [`SystemNotifierConfig`]. The worker
/// holds this and re-reads it under a short read-lock per line so the
/// config loader can hot-swap the conduit / templates / filter without
/// restarting the plugin.
pub type SharedSystemNotifierConfig = Arc<RwLock<SystemNotifierConfig>>;

fn default_debounce() -> u64 {
    DEFAULT_DEBOUNCE_MS
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_title() -> String {
    DEFAULT_TITLE.to_string()
}

/// Configuration for the System Notifier plugin.
#[derive(Clone, Debug, Deserialize)]
pub struct SystemNotifierConfig {
    /// Start the worker at process start. CLI override:
    /// `--plugin-system-notify` / `--no-plugin-system-notify`. The
    /// `enabled` TOML alias matches the spec wording for
    /// `[plugins.system_notifier]`.
    #[serde(default, alias = "enabled")]
    pub enabled_at_startup: bool,

    /// Path to the user-provided conduit binary or script. Subject to
    /// `~` and `${VAR}` expansion via
    /// [`crate::event_config::interpolation`] at invocation time.
    #[serde(default)]
    pub conduit: String,

    /// Optional extra positional arguments passed to the conduit
    /// before its stdin is closed. Defaults to empty.
    #[serde(default)]
    pub conduit_args: Vec<String>,

    /// Optional event-type filter. Empty = all types. Names match the
    /// `AppEvent` serde tag (`"StatusChanged"`, `"RateLimited"`, ...).
    #[serde(default)]
    pub on: Vec<String>,

    /// Title template — rendered with the same `{field}` substitution
    /// the notifier uses. Defaults to `"{kind}"`.
    #[serde(default = "default_title")]
    pub title: String,

    /// Body template. Defaults to empty.
    #[serde(default)]
    pub body: String,

    /// Default debounce window in milliseconds. Suppresses repeats of
    /// the same `(event_identity)` inside this window.
    #[serde(default = "default_debounce")]
    pub debounce_ms: u64,

    /// Per-invocation wall-clock timeout. After this many milliseconds
    /// the worker kills the conduit subprocess and logs once.
    #[serde(default = "default_timeout")]
    pub conduit_timeout_ms: u64,

    /// Monotonic generation counter — bumped by the config loader each
    /// time a field changes so the worker can re-snapshot under a
    /// short read-lock. Skipped by serde so it never appears in user
    /// TOML.
    #[serde(skip)]
    pub generation: u64,
}

impl Default for SystemNotifierConfig {
    fn default() -> Self {
        Self {
            enabled_at_startup: false,
            conduit: String::new(),
            conduit_args: Vec::new(),
            on: Vec::new(),
            title: default_title(),
            body: String::new(),
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            conduit_timeout_ms: DEFAULT_TIMEOUT_MS,
            generation: 0,
        }
    }
}

impl SystemNotifierConfig {
    /// Validate that the conduit field is populated whenever the
    /// plugin is enabled. Surfaced to the config loader so a missing
    /// conduit shows up in the `parse_errors` banner.
    pub fn validate(&self) -> Result<(), String> {
        if self.enabled_at_startup && self.conduit.trim().is_empty() {
            return Err("conduit is required when system_notifier.enabled = true".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse via the same envelope users write: `[plugins.system_notifier]`.
    fn parse(snippet: &str) -> SystemNotifierConfig {
        let cf: crate::event_config::schema::ConfigFile =
            toml::from_str(snippet).expect("parse ConfigFile");
        cf.plugins
            .system_notifier
            .expect("system_notifier table missing")
    }

    #[test]
    fn config_round_trips_minimal_snippet() {
        let cfg = parse(
            r#"
            [plugins.system_notifier]
            enabled = true
            conduit = "/bin/notify.sh"
            "#,
        );
        assert!(cfg.enabled_at_startup);
        assert_eq!(cfg.conduit, "/bin/notify.sh");
        assert!(cfg.on.is_empty());
        assert!(cfg.body.is_empty());
        assert_eq!(cfg.title, DEFAULT_TITLE);
    }

    #[test]
    fn config_round_trips_full_snippet() {
        let cfg = parse(
            r#"
            [plugins.system_notifier]
            enabled = true
            conduit = "~/bin/notify.sh"
            conduit_args = ["--quiet"]
            on = ["StatusChanged", "RateLimited"]
            title = "abtop: {kind}"
            body  = "{session_id}: {detail}"
            debounce_ms = 1000
            conduit_timeout_ms = 7500
            "#,
        );
        assert!(cfg.enabled_at_startup);
        assert_eq!(cfg.conduit, "~/bin/notify.sh");
        assert_eq!(cfg.conduit_args, vec!["--quiet".to_string()]);
        assert_eq!(
            cfg.on,
            vec!["StatusChanged".to_string(), "RateLimited".to_string()]
        );
        assert_eq!(cfg.title, "abtop: {kind}");
        assert_eq!(cfg.body, "{session_id}: {detail}");
        assert_eq!(cfg.debounce_ms, 1000);
        assert_eq!(cfg.conduit_timeout_ms, 7500);
    }

    #[test]
    fn validator_rejects_missing_conduit() {
        let cfg = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: String::new(),
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("conduit is required"));
    }

    #[test]
    fn validator_rejects_whitespace_only_conduit() {
        let cfg = SystemNotifierConfig {
            enabled_at_startup: true,
            conduit: "   ".to_string(),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validator_accepts_disabled_without_conduit() {
        let cfg = SystemNotifierConfig {
            enabled_at_startup: false,
            conduit: String::new(),
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn config_default_timeout_is_5000() {
        let cfg = SystemNotifierConfig::default();
        assert_eq!(cfg.conduit_timeout_ms, 5_000);
    }

    #[test]
    fn config_default_title_is_kind_marker() {
        let cfg = SystemNotifierConfig::default();
        assert_eq!(cfg.title, "{kind}");
    }

    #[test]
    fn config_default_debounce_is_5000() {
        let cfg = SystemNotifierConfig::default();
        assert_eq!(cfg.debounce_ms, 5_000);
    }

    #[test]
    fn empty_table_uses_defaults() {
        let cfg = parse(
            r#"
            [plugins.system_notifier]
            "#,
        );
        assert!(!cfg.enabled_at_startup);
        assert_eq!(cfg.conduit, "");
        assert_eq!(cfg.title, DEFAULT_TITLE);
        assert_eq!(cfg.debounce_ms, 5_000);
        assert_eq!(cfg.conduit_timeout_ms, 5_000);
    }
}
