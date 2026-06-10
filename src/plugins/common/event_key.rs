//! Stable identity hash for event payloads.
//!
//! Picks the first available "key" field in priority order from a flat
//! JSON object. The hash itself is small + fast (FNV-1a) — collisions
//! are fine because the producer index in the debouncer key already
//! discriminates between different rules.

use serde_json::Value;

/// Stable hash for the event's identity. Picks the first available
/// "key" field in priority order. Returns 0 when no recognized key is
/// present (which collapses all such events into a single debounce
/// bucket — acceptable since these are typically singleton events like
/// `HostLoadHigh`).
pub fn event_key_hash(ctx: &Value) -> u64 {
    let obj = match ctx.as_object() {
        Some(o) => o,
        None => return 0,
    };
    for key in ["session_id", "provider", "port", "pid"] {
        if let Some(v) = obj.get(key) {
            return fnv1a(v.to_string().as_bytes());
        }
    }
    0
}

fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01B3;
    let mut h = OFFSET;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_key_hash_prefers_session_id() {
        let h1 = event_key_hash(&json!({"session_id": "abc", "provider": "claude"}));
        let h2 = event_key_hash(&json!({"session_id": "abc", "provider": "openai"}));
        let h3 = event_key_hash(&json!({"session_id": "xyz", "provider": "claude"}));
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn event_key_hash_falls_through_to_provider() {
        let h1 = event_key_hash(&json!({"provider": "claude"}));
        let h2 = event_key_hash(&json!({"provider": "openai"}));
        assert_ne!(h1, h2);
    }

    #[test]
    fn event_key_hash_returns_zero_for_unkeyed() {
        assert_eq!(event_key_hash(&json!({"load1": 5.0})), 0);
        assert_eq!(event_key_hash(&json!([])), 0);
    }
}
