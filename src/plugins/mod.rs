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

#[cfg(feature = "plugin-notifier")]
pub mod notifier;

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
                built.registry.start(plugin, socket_path, true);
            }
        }
        built
    }
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
