//! Template substitution for notifier titles/bodies — re-exports the
//! shared implementation in [`crate::plugins::common::template`] so
//! existing callsites (`super::template::render`,
//! `template::escape_for_osascript`) continue to compile unchanged.
//!
//! The substantive code lives in `plugins::common::template`.

pub use crate::plugins::common::template::{escape_for_osascript, render};
