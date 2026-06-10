//! Compiled-in plugin host.
//!
//! Plugins are ordinary Rust types that implement [`Plugin`]; each one
//! owns a worker thread that connects to the events Unix socket like
//! any external consumer would (e.g. `nc -U`). The publisher and the
//! registry share exactly one piece of knowledge: the socket path.
//!
//! No dynamic loading, no API for cross-cutting state — only the
//! socket and the [`PluginCtx`] structure (an enabled flag, a
//! shutdown flag, and a path).
//!
//! Panic isolation: each plugin's worker is wrapped in `catch_unwind`
//! at the call site that drives it. Panics terminate only the thread.

pub mod common;
#[cfg(feature = "plugin-notifier")]
pub mod notifier;
#[cfg(feature = "plugin-system-notifier")]
pub mod system_notifier;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::JoinHandle;

/// Aggregated plugin-config struct. U7 will populate this from a TOML
/// loader; U6 lands the shape so the registry has a stable build entry.
#[derive(Clone, Debug, Default)]
pub struct PluginsConfig {
    #[cfg(feature = "plugin-notifier")]
    pub notifier: notifier::NotifierConfig,
    #[cfg(feature = "plugin-system-notifier")]
    pub system_notifier: system_notifier::SystemNotifierConfig,
}

/// Result of [`PluginsConfig::build_registry`] — the registry itself
/// plus the shared handles the hot-reload path needs to mutate
/// running plugins without restarting them.
#[derive(Default)]
pub struct BuiltRegistry {
    pub registry: PluginRegistry,
    /// Shared notifier config (`Arc<RwLock<NotifierConfig>>`) when the
    /// notifier was started; `None` otherwise.
    #[cfg(feature = "plugin-notifier")]
    pub notifier_shared: Option<notifier::SharedNotifierConfig>,
    /// Shared system_notifier config when the System Notifier was
    /// started; `None` otherwise. Mirrors `notifier_shared`.
    #[cfg(feature = "plugin-system-notifier")]
    pub system_notifier_shared: Option<system_notifier::SharedSystemNotifierConfig>,
}

impl PluginsConfig {
    /// Build a registry from this config + an events socket path.
    /// Spawns one worker per enabled plugin. Plugins that are
    /// disabled-at-startup are NOT spawned at all (no idle thread).
    #[allow(unused_variables, unused_mut)]
    pub fn build_registry(&self, socket_path: PathBuf) -> BuiltRegistry {
        let mut built = BuiltRegistry::default();
        #[cfg(feature = "plugin-notifier")]
        {
            if self.notifier.enabled_at_startup {
                let plugin = notifier::Notifier::new(self.notifier.clone());
                built.notifier_shared = Some(plugin.shared());
                built.registry.start(plugin, socket_path.clone(), true);
            }
        }
        #[cfg(feature = "plugin-system-notifier")]
        {
            if self.system_notifier.enabled_at_startup {
                let plugin = system_notifier::SystemNotifier::new(self.system_notifier.clone());
                built.system_notifier_shared = Some(plugin.shared());
                built.registry.start(plugin, socket_path, true);
            }
        }
        built
    }
}

/// Static metadata describing one compiled-in plugin. Surfaced by
/// `abtop --list-plugins` so users can discover plugins and grab a
/// copy-pasteable config snippet without leaving the terminal.
///
/// Each plugin module owns its own `pub fn info() -> PluginInfo`; the
/// aggregate is assembled by [`list_available`].
#[derive(Clone, Debug)]
pub struct PluginInfo {
    /// Plugin identifier, matches `Plugin::name()`.
    pub name: &'static str,
    /// Cargo feature flag that gates the plugin.
    pub feature: &'static str,
    /// Whether `feature` is part of `[features].default` in Cargo.toml.
    pub default_on: bool,
    /// Whether the plugin auto-spawns at process start without any
    /// explicit `--plugin-<x>` flag.
    pub startup_enabled: bool,
    /// One-paragraph plain-text description.
    pub description: &'static str,
    /// One-paragraph plain-text startup behavior summary.
    pub startup: &'static str,
    /// Copy-pasteable TOML block keyed for `~/.config/abtop/config.toml`.
    pub example_config: &'static str,
    /// Pointer to the canonical docs section.
    pub docs_pointer: &'static str,
}

/// Catalogue of plugins compiled into this binary. Empty when every
/// `plugin-*` feature is disabled.
pub fn list_available() -> Vec<PluginInfo> {
    #[allow(unused_mut)]
    let mut v: Vec<PluginInfo> = Vec::new();
    #[cfg(feature = "plugin-notifier")]
    {
        v.push(crate::plugins::notifier::info());
    }
    #[cfg(feature = "plugin-system-notifier")]
    {
        v.push(crate::plugins::system_notifier::info());
    }
    v
}

/// Render the plugin catalogue. Generic over `Write` so tests can
/// capture output into a `Vec<u8>`; the CLI handler passes
/// `&mut std::io::stdout().lock()`.
pub fn print_catalogue<W: std::io::Write>(
    out: &mut W,
    plugins: &[PluginInfo],
) -> std::io::Result<()> {
    if plugins.is_empty() {
        writeln!(out, "No plugins compiled into this binary.")?;
        writeln!(
            out,
            "(Plugins are gated behind Cargo features. Rebuild with e.g."
        )?;
        writeln!(
            out,
            " `cargo install --features plugin-notifier` to enable one.)"
        )?;
        return Ok(());
    }
    writeln!(out, "Available plugins (compiled into this binary):")?;
    writeln!(out)?;
    for p in plugins {
        let default_marker = if p.default_on { "default-on" } else { "opt-in" };
        writeln!(
            out,
            "  {name:<40}  [feature: {feature} · {marker}]",
            name = p.name,
            feature = p.feature,
            marker = default_marker,
        )?;
        write_wrapped(out, "    Description: ", p.description, 17)?;
        write_wrapped(out, "    Startup:     ", p.startup, 17)?;
        writeln!(out)?;
        writeln!(
            out,
            "    Example config snippet (drop into ~/.config/abtop/config.toml):"
        )?;
        writeln!(out)?;
        for line in p.example_config.lines() {
            if line.is_empty() {
                writeln!(out)?;
            } else {
                writeln!(out, "      {line}")?;
            }
        }
        writeln!(out)?;
        writeln!(out, "    Docs: {}", p.docs_pointer)?;
        writeln!(out)?;
    }
    Ok(())
}

/// Wrap `text` to ~62 columns of payload after `prefix`, then indent
/// continuation lines by `indent` spaces. Keeps the catalogue readable
/// in an 80-column terminal without pulling in a wrapping crate.
fn write_wrapped<W: std::io::Write>(
    out: &mut W,
    prefix: &str,
    text: &str,
    indent: usize,
) -> std::io::Result<()> {
    const WIDTH: usize = 62;
    let pad: String = " ".repeat(indent);
    let mut first = true;
    let mut line = String::new();
    for word in text.split_whitespace() {
        let extra = if line.is_empty() {
            word.len()
        } else {
            line.len() + 1 + word.len()
        };
        if extra > WIDTH && !line.is_empty() {
            if first {
                writeln!(out, "{prefix}{line}")?;
                first = false;
            } else {
                writeln!(out, "{pad}{line}")?;
            }
            line.clear();
            line.push_str(word);
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    if !line.is_empty() {
        if first {
            writeln!(out, "{prefix}{line}")?;
        } else {
            writeln!(out, "{pad}{line}")?;
        }
    }
    Ok(())
}

/// Trait implemented by each compiled-in plugin.
pub trait Plugin: Send + 'static {
    /// Stable identifier — also used as the registry key for runtime
    /// `set_enabled(name, …)` toggles.
    fn name(&self) -> &'static str;

    /// Spawn the plugin's worker thread. The plugin owns the thread
    /// and must respect `ctx.shutdown` to exit cleanly.
    fn start(&self, ctx: PluginCtx) -> PluginHandle;
}

/// Context handed to a plugin at startup. `enabled` is a runtime
/// pause-flag (toggleable from the registry), `shutdown` signals
/// process exit.
#[derive(Clone)]
pub struct PluginCtx {
    pub socket_path: PathBuf,
    pub enabled: Arc<AtomicBool>,
    pub shutdown: Arc<AtomicBool>,
}

/// Live handle to a started plugin. The registry owns these; on
/// shutdown each is joined.
pub struct PluginHandle {
    pub name: &'static str,
    pub enabled: Arc<AtomicBool>,
    pub join: JoinHandle<()>,
}

/// Registry of started plugins.
#[derive(Default)]
pub struct PluginRegistry {
    handles: Vec<PluginHandle>,
    shutdown: Arc<AtomicBool>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start `plugin` against the given socket path. The plugin's
    /// `enabled` flag defaults to `enabled_default`.
    pub fn start<P: Plugin>(&mut self, plugin: P, socket_path: PathBuf, enabled_default: bool) {
        let enabled = Arc::new(AtomicBool::new(enabled_default));
        let ctx = PluginCtx {
            socket_path,
            enabled: Arc::clone(&enabled),
            shutdown: Arc::clone(&self.shutdown),
        };
        let handle = plugin.start(ctx);
        self.handles.push(handle);
    }

    /// Toggle a plugin's pause flag. Returns true if the named plugin
    /// existed.
    pub fn set_enabled(&self, name: &str, on: bool) -> bool {
        for h in &self.handles {
            if h.name == name {
                h.enabled.store(on, std::sync::atomic::Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.handles.iter().map(|h| h.name).collect()
    }

    /// Snapshot the registry's per-plugin enabled flags as a control
    /// surface for [`App::apply_event_config`]. Cloning each
    /// `Arc<AtomicBool>` is cheap and lets the App toggle plugins at
    /// runtime without owning the full registry (which still owns the
    /// JoinHandles for shutdown).
    pub fn control_handles(&self) -> Vec<(&'static str, Arc<AtomicBool>)> {
        self.handles
            .iter()
            .map(|h| (h.name, Arc::clone(&h.enabled)))
            .collect()
    }

    /// Signal shutdown to all started plugins and join their threads.
    /// Plugins MUST observe the shutdown flag periodically — uncooperative
    /// plugins simply have their `join()` block indefinitely; for that
    /// reason the per-thread loops we ship use short poll intervals.
    pub fn shutdown(self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
        for h in self.handles {
            let _ = h.join.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    /// A minimal plugin that flips an external "ran" flag and exits
    /// when its shutdown flag is set.
    struct EchoPlugin {
        ran: Arc<AtomicBool>,
    }

    impl Plugin for EchoPlugin {
        fn name(&self) -> &'static str {
            "echo"
        }

        fn start(&self, ctx: PluginCtx) -> PluginHandle {
            let ran = Arc::clone(&self.ran);
            let enabled_for_handle = Arc::clone(&ctx.enabled);
            let enabled_for_thread = Arc::clone(&ctx.enabled);
            let shutdown = Arc::clone(&ctx.shutdown);
            let join = std::thread::spawn(move || {
                while !shutdown.load(Ordering::Relaxed) {
                    if enabled_for_thread.load(Ordering::Relaxed) {
                        ran.store(true, Ordering::Relaxed);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
            PluginHandle {
                name: "echo",
                enabled: enabled_for_handle,
                join,
            }
        }
    }

    #[test]
    fn lifecycle_runs_and_shuts_down() {
        let mut registry = PluginRegistry::new();
        let ran = Arc::new(AtomicBool::new(false));
        registry.start(
            EchoPlugin {
                ran: Arc::clone(&ran),
            },
            PathBuf::from("/dev/null"),
            true,
        );
        // Give the worker a chance to flip the flag.
        std::thread::sleep(Duration::from_millis(50));
        assert!(ran.load(Ordering::Relaxed), "worker should have run");
        registry.shutdown();
    }

    #[test]
    fn set_enabled_toggles_named_plugin() {
        let mut registry = PluginRegistry::new();
        let ran = Arc::new(AtomicBool::new(false));
        registry.start(
            EchoPlugin {
                ran: Arc::clone(&ran),
            },
            PathBuf::from("/dev/null"),
            false,
        );
        assert!(registry.set_enabled("echo", true));
        std::thread::sleep(Duration::from_millis(50));
        assert!(ran.load(Ordering::Relaxed), "should run after enable");
        assert!(!registry.set_enabled("missing", true));
        registry.shutdown();
    }

    #[test]
    fn print_catalogue_empty_says_no_plugins() {
        let mut buf: Vec<u8> = Vec::new();
        print_catalogue(&mut buf, &[]).expect("write to Vec never fails");
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("No plugins compiled into this binary."));
        assert!(s.contains("--features plugin-notifier"));
    }

    #[test]
    fn print_catalogue_renders_plugin_info() {
        let info = PluginInfo {
            name: "demo-plugin",
            feature: "plugin-demo",
            default_on: true,
            startup_enabled: false,
            description: "Test plugin used in unit tests only.",
            startup: "disabled by default.",
            example_config: "[plugins.demo]\nenabled = true\n",
            docs_pointer: "docs/demo.md",
        };
        let mut buf: Vec<u8> = Vec::new();
        print_catalogue(&mut buf, &[info]).expect("write to Vec never fails");
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Available plugins"));
        assert!(s.contains("demo-plugin"));
        assert!(s.contains("plugin-demo"));
        assert!(s.contains("default-on"));
        assert!(s.contains("[plugins.demo]"));
        assert!(s.contains("enabled = true"));
        assert!(s.contains("docs/demo.md"));
    }

    #[cfg(feature = "plugin-notifier")]
    #[test]
    fn list_available_includes_notifier_when_feature_on() {
        let v = list_available();
        assert!(v.iter().any(|p| p.name == "notifier"));
    }

    #[cfg(feature = "plugin-system-notifier")]
    #[test]
    fn list_available_includes_system_notifier_when_feature_on() {
        let v = list_available();
        assert!(v.iter().any(|p| p.name == "system_notifier"));
    }

    #[cfg(all(feature = "plugin-notifier", feature = "plugin-system-notifier"))]
    #[test]
    fn list_available_descriptions_distinguish_plugins() {
        // Make doubly sure the catalogue can be read by a user
        // without confusion: each description must carry its own
        // differentiator phrase.
        let v = list_available();
        let n = v.iter().find(|p| p.name == "notifier").unwrap();
        let s = v.iter().find(|p| p.name == "system_notifier").unwrap();
        assert!(
            n.description.to_lowercase().contains("built-in"),
            "notifier description must call out its built-in backends: {}",
            n.description
        );
        assert!(
            s.description.to_lowercase().contains("conduit"),
            "system_notifier description must call out the conduit: {}",
            s.description
        );
    }

    #[cfg(feature = "plugin-system-notifier")]
    #[test]
    fn build_registry_starts_system_notifier_when_enabled() {
        let cfg = PluginsConfig {
            #[cfg(feature = "plugin-notifier")]
            notifier: notifier::NotifierConfig::default(),
            system_notifier: system_notifier::SystemNotifierConfig {
                enabled_at_startup: true,
                conduit: "/bin/true".to_string(),
                ..Default::default()
            },
        };
        let built = cfg.build_registry(PathBuf::from("/dev/null"));
        assert!(
            built.system_notifier_shared.is_some(),
            "system_notifier_shared should be populated when started"
        );
        assert!(built.registry.names().contains(&"system_notifier"));
        // The worker is connecting in the background to /dev/null,
        // which fails — it backs off and retries until shutdown. The
        // shutdown flag is observed at each backoff iteration.
        built.registry.shutdown();
    }

    #[cfg(feature = "plugin-system-notifier")]
    #[test]
    fn build_registry_skips_system_notifier_when_disabled() {
        let cfg = PluginsConfig {
            #[cfg(feature = "plugin-notifier")]
            notifier: notifier::NotifierConfig::default(),
            system_notifier: system_notifier::SystemNotifierConfig {
                enabled_at_startup: false,
                ..Default::default()
            },
        };
        let built = cfg.build_registry(PathBuf::from("/dev/null"));
        assert!(
            built.system_notifier_shared.is_none(),
            "system_notifier should not be started when disabled"
        );
        built.registry.shutdown();
    }

    #[test]
    fn names_reflects_started_plugins() {
        let mut registry = PluginRegistry::new();
        registry.start(
            EchoPlugin {
                ran: Arc::new(AtomicBool::new(false)),
            },
            PathBuf::from("/dev/null"),
            true,
        );
        assert_eq!(registry.names(), vec!["echo"]);
        registry.shutdown();
    }
}
