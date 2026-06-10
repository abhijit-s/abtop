//! Pub-sub event system.
//!
//! Public surface:
//! - [`AppEvent`] — the wire-contract enum of state-transition events.
//! - [`WireRecord`] — NDJSON envelope adding `v` and `ts_ms`.

mod diff;
mod types;

pub use diff::{diff, EventDifferState};
pub use types::{AppEvent, EventKind, EventTier, WireRecord, WIRE_VERSION};
