//! System Notifier plugin — surfaces events through a user-supplied
//! conduit binary.
//!
//! Unlike the sibling [`crate::plugins::notifier`] (which ships
//! compiled-in OS-notification backends), this plugin delegates the
//! "actually surface a notification" step to a user-configured conduit
//! script. It still owns the connect-loop and NDJSON parsing — the user
//! owns the script that takes title/body/event JSON and does whatever
//! they want with it (osascript, ntfy, curl webhook, ...).
//!
//! Failure semantics: log + drop, single dedicated invocation thread
//! fed by a bounded channel from the socket reader, 5-second
//! wall-clock timeout per conduit invocation. See [`invoke`] for
//! details.

pub mod config;
pub mod invoke;

pub use config::{SharedSystemNotifierConfig, SystemNotifierConfig};
