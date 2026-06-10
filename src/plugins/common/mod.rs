//! Shared primitives used by more than one plugin worker.
//!
//! These are honest utilities — debounce bookkeeping, template
//! substitution, and a stable hash for event identity — not a rule
//! grammar. The `notifier` plugin keeps its own `rules.rs` with the
//! `when` matcher; the `system_notifier` plugin consumes only what's
//! here.
//!
//! Kept unconditional (i.e. not feature-gated) because every type
//! defined here is small and used through plugin-feature gated callers
//! already. Compiling them always avoids feature combinations where one
//! plugin pulls them in via a path the other plugin gates out.

pub mod debounce;
pub mod event_key;
pub mod template;
