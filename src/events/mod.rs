//! Pub-sub event system.
//!
//! Public surface:
//! - [`AppEvent`] — the wire-contract enum of state-transition events.
//! - [`WireRecord`] — NDJSON envelope adding `v` and `ts_ms`.

mod types;

pub use types::{AppEvent, EventKind, EventTier, WireRecord, WIRE_VERSION};
