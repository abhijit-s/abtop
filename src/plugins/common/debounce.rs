//! Per-key debouncer used by plugin workers to suppress repeats of the
//! same logical event inside a configurable window.
//!
//! The key is a `(producer_idx, identity_hash)` tuple — `producer_idx`
//! disambiguates multiple rules (in the notifier) or stays 0 (in the
//! system notifier, which has exactly one conduit), and
//! `identity_hash` is supplied by [`super::event_key::event_key_hash`].

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Tracks last-fired timestamps for `(producer_idx, key_hash)` pairs.
/// Garbage-collects when the map exceeds 1024 entries by dropping
/// anything older than `10 × max_debounce`.
#[derive(Debug, Default)]
pub struct Debouncer {
    last: HashMap<(usize, u64), Instant>,
    max_debounce_ms: u64,
}

impl Debouncer {
    pub fn new(max_debounce_ms: u64) -> Self {
        Self {
            last: HashMap::new(),
            max_debounce_ms,
        }
    }

    /// Returns true iff the event should fire (i.e. either it's the
    /// first time for this key or the last fire was longer ago than
    /// `effective_ms`).
    pub fn allow(&mut self, key: (usize, u64), effective_ms: u64) -> bool {
        self.note_max(effective_ms);
        let now = Instant::now();
        let allow = match self.last.get(&key) {
            None => true,
            Some(prev) => {
                now.saturating_duration_since(*prev) >= Duration::from_millis(effective_ms)
            }
        };
        if allow {
            self.last.insert(key, now);
            self.gc(now);
        }
        allow
    }

    fn note_max(&mut self, ms: u64) {
        if ms > self.max_debounce_ms {
            self.max_debounce_ms = ms;
        }
    }

    fn gc(&mut self, now: Instant) {
        if self.last.len() <= 1024 {
            return;
        }
        let horizon = Duration::from_millis(self.max_debounce_ms.saturating_mul(10).max(60_000));
        self.last
            .retain(|_, t| now.saturating_duration_since(*t) < horizon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debouncer_blocks_within_window() {
        let mut d = Debouncer::new(1000);
        assert!(d.allow((0, 1), 1000));
        // Second call within the window — blocked.
        assert!(!d.allow((0, 1), 1000));
    }

    #[test]
    fn debouncer_separates_keys() {
        let mut d = Debouncer::new(1000);
        assert!(d.allow((0, 1), 1000));
        // Different producer_idx or key_hash -> independent.
        assert!(d.allow((1, 1), 1000));
        assert!(d.allow((0, 2), 1000));
    }

    #[test]
    fn debouncer_zero_window_always_allows() {
        let mut d = Debouncer::new(0);
        assert!(d.allow((0, 1), 0));
        assert!(d.allow((0, 1), 0));
        assert!(d.allow((0, 1), 0));
    }
}
