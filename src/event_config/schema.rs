//! Wire-level serde schema for `~/.config/abtop/config.toml`. The
//! types are deliberately permissive — every field is optional and
//! unknown fields are ignored (no `deny_unknown_fields`) so a
//! forward-rolled config doesn't break older binaries.
//!
//! The schema is rendered into the engine-level
//! [`crate::events::publisher`] settings and the
//! [`crate::plugins::notifier::NotifierConfig`] elsewhere in the
//! `event_config` module. This file only worries about parsing.

use serde::Deserialize;

/// Top-level shape. Matches the `[events]` and `[plugins.*]` tables.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub events: EventsConfig,
    #[serde(default)]
    pub plugins: PluginsTable,
}

/// `[events]` table.
#[derive(Clone, Debug, Deserialize)]
pub struct EventsConfig {
    /// Whether the publisher should be enabled at startup. CLI flags
    /// (`--events`, `--events-off`) still override.
    #[serde(default = "default_events_enabled")]
    pub enabled: bool,
    /// Socket path. Subject to `${UID}` / `${XDG_RUNTIME_DIR}` / `~`
    /// expansion via [`crate::event_config::interpolation`]. `None`
    /// means "let the socket-path resolver pick the default".
    #[serde(default)]
    pub socket: Option<String>,
    /// Per-connection NDJSON backlog depth. Hard-applied at startup;
    /// changes require restart.
    #[serde(default = "default_backlog")]
    pub backlog: usize,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            enabled: default_events_enabled(),
            socket: None,
            backlog: default_backlog(),
        }
    }
}

fn default_events_enabled() -> bool {
    // Off by default: the user must opt-in via either the config file
    // or `--events`. Mirrors the U6 CLI behavior.
    false
}

fn default_backlog() -> usize {
    256
}

/// `[plugins.*]` container. Currently only the notifier table is
/// recognized; adding more plugins later means extending this struct.
///
/// Even when the `plugin-notifier` feature is disabled we still parse
/// the table (as an opaque `toml::Value`) so a config file shared
/// across builds doesn't error out. The non-feature build just ignores
/// the parsed value.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PluginsTable {
    #[cfg(feature = "plugin-notifier")]
    #[serde(default)]
    pub notifier: Option<crate::plugins::notifier::NotifierConfig>,
    #[cfg(not(feature = "plugin-notifier"))]
    #[serde(default)]
    pub notifier: Option<toml::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_uses_defaults() {
        let cf: ConfigFile = toml::from_str("").unwrap();
        assert!(!cf.events.enabled);
        assert_eq!(cf.events.backlog, 256);
        assert!(cf.events.socket.is_none());
        assert!(cf.plugins.notifier.is_none());
    }

    #[test]
    fn events_section_round_trips() {
        let cf: ConfigFile = toml::from_str(
            r#"
            [events]
            enabled = true
            socket = "/tmp/abtop.sock"
            backlog = 512
            "#,
        )
        .unwrap();
        assert!(cf.events.enabled);
        assert_eq!(cf.events.socket.as_deref(), Some("/tmp/abtop.sock"));
        assert_eq!(cf.events.backlog, 512);
    }

    #[test]
    fn unknown_top_level_keys_are_ignored() {
        // Forward-compat: a future `[telemetry]` table must not break
        // this version of the binary.
        let cf: ConfigFile = toml::from_str(
            r#"
            [events]
            enabled = true

            [telemetry]
            kind = "prometheus"
            "#,
        )
        .unwrap();
        assert!(cf.events.enabled);
    }

    #[test]
    fn malformed_toml_returns_error() {
        let res: Result<ConfigFile, _> = toml::from_str("this isn't valid TOML = =\n[broken");
        assert!(res.is_err());
    }

    #[test]
    fn events_partial_section_inherits_defaults() {
        let cf: ConfigFile = toml::from_str(
            r#"
            [events]
            enabled = true
            "#,
        )
        .unwrap();
        assert!(cf.events.enabled);
        assert_eq!(cf.events.backlog, 256);
    }
}
