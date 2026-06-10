//! Pub-sub event system.
//!
//! Public surface:
//! - [`AppEvent`] — the wire-contract enum of state-transition events.
//! - [`WireRecord`] — NDJSON envelope adding `v` and `ts_ms`.

mod diff;
pub mod publisher;
pub mod socket_path;
mod types;

pub use diff::{diff, EventDifferState};
pub use publisher::EventPublisher;
pub use types::{AppEvent, EventKind, EventTier, WireRecord, WIRE_VERSION};
