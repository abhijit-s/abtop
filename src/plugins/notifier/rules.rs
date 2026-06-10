//! Rule matching + debouncing for the notifier plugin.
//!
//! v1 grammar for `when` is intentionally tiny: exactly one
//! `<field> <op> <literal>` comparison, no `and`/`or`. Users who need
//! conjunction write two rules. Parsing happens once at config load —
//! invalid `when` expressions disable just the offending rule and log
//! a single line, so a typo can't take down the worker.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// One user-defined rule. Field names mirror the TOML / serde layout
/// described in the design doc.
#[derive(Clone, Debug, Deserialize)]
pub struct Rule {
    /// Event type names this rule applies to. Empty = wildcard.
    #[serde(default)]
    pub on: Vec<String>,
    /// Optional filter expression (`<field> <op> <literal>`).
    #[serde(default)]
    pub when: Option<String>,
    /// Title template (supports `{field}` substitution).
    pub title: String,
    /// Body template.
    pub body: String,
    /// Per-rule override of the config-level debounce.
    #[serde(default)]
    pub debounce_ms: Option<u64>,
}

/// Compiled form of a rule. Built once at startup; the worker reuses
/// it for the lifetime of the plugin.
#[derive(Clone, Debug)]
pub struct CompiledRule {
    pub on: Vec<String>,
    pub when: Option<CompiledWhen>,
    pub title: String,
    pub body: String,
    pub debounce_ms: Option<u64>,
    /// Original index in the user's `rule` list. Used as the first half
    /// of the debounce key so rules don't collide with each other.
    pub idx: usize,
    /// True when the user provided a `when` expression that failed to
    /// parse. The worker skips such rules entirely.
    pub disabled: bool,
    /// Reason for `disabled` — surfaced once via stderr at startup.
    pub disable_reason: Option<String>,
}

/// Parsed `when` expression. Compares a single field against a literal.
#[derive(Clone, Debug)]
pub struct CompiledWhen {
    pub field: String,
    pub op: Op,
    pub literal: Literal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Ge,
    Le,
    Gt,
    Lt,
}

#[derive(Clone, Debug)]
pub enum Literal {
    Str(String),
    Num(f64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parse a `when` expression. Grammar (whitespace-tolerant):
///   <field>  ::= [A-Za-z_][A-Za-z0-9_]*
///   <op>     ::= "==" | "!=" | ">=" | "<=" | ">" | "<"
///   <literal>::= "\"" any-non-quote "\"" | number
pub fn parse_when(src: &str) -> Result<CompiledWhen, ParseError> {
    let s = src.trim();
    if s.is_empty() {
        return Err(ParseError("empty when expression".into()));
    }

    // Identifier: leading letters/digits/underscore. Stop at first
    // non-ident byte so the operator can begin.
    let ident_end = s
        .bytes()
        .position(|b| !(b.is_ascii_alphanumeric() || b == b'_'))
        .ok_or_else(|| ParseError(format!("missing operator in: {src}")))?;
    if ident_end == 0 {
        return Err(ParseError(format!("expected field name in: {src}")));
    }
    let field = s[..ident_end].to_string();
    let rest = s[ident_end..].trim_start();

    // Two-character ops first so `>=` doesn't tokenize as `>`.
    let (op, after_op) = if let Some(r) = rest.strip_prefix("==") {
        (Op::Eq, r)
    } else if let Some(r) = rest.strip_prefix("!=") {
        (Op::Ne, r)
    } else if let Some(r) = rest.strip_prefix(">=") {
        (Op::Ge, r)
    } else if let Some(r) = rest.strip_prefix("<=") {
        (Op::Le, r)
    } else if let Some(r) = rest.strip_prefix('>') {
        (Op::Gt, r)
    } else if let Some(r) = rest.strip_prefix('<') {
        (Op::Lt, r)
    } else {
        return Err(ParseError(format!("unknown operator in: {src}")));
    };

    let lit_src = after_op.trim();
    if lit_src.is_empty() {
        return Err(ParseError(format!("missing literal in: {src}")));
    }

    let literal = if let Some(quoted) = lit_src.strip_prefix('"') {
        let end = quoted
            .find('"')
            .ok_or_else(|| ParseError(format!("unterminated string literal in: {src}")))?;
        // Reject trailing garbage after the closing quote.
        let tail = quoted[end + 1..].trim();
        if !tail.is_empty() {
            return Err(ParseError(format!(
                "trailing tokens after literal in: {src}"
            )));
        }
        Literal::Str(quoted[..end].to_string())
    } else {
        // Numeric literal.
        let n: f64 = lit_src
            .parse()
            .map_err(|_| ParseError(format!("invalid numeric literal in: {src}")))?;
        Literal::Num(n)
    };

    Ok(CompiledWhen { field, op, literal })
}

/// Compile a list of user-defined rules. Rules with invalid `when`
/// clauses are returned in `disabled = true` state with a stable index
/// — the caller logs them once and skips matching against them.
pub fn compile(rules: Vec<Rule>) -> Vec<CompiledRule> {
    rules
        .into_iter()
        .enumerate()
        .map(|(idx, r)| {
            let (when, disabled, reason) = match &r.when {
                None => (None, false, None),
                Some(src) => match parse_when(src) {
                    Ok(w) => (Some(w), false, None),
                    Err(e) => (None, true, Some(e.0)),
                },
            };
            CompiledRule {
                on: r.on,
                when,
                title: r.title,
                body: r.body,
                debounce_ms: r.debounce_ms,
                idx,
                disabled,
                disable_reason: reason,
            }
        })
        .collect()
}

/// Evaluate a compiled `when` against a JSON value. Numeric comparisons
/// coerce both sides to `f64`; string comparisons use Eq/Ne only —
/// other operators against string literals return false (no match).
pub fn eval_when(when: &CompiledWhen, ctx: &Value) -> bool {
    let field_val = match ctx.as_object().and_then(|o| o.get(&when.field)) {
        Some(v) => v,
        None => return false,
    };
    match (&when.literal, field_val) {
        (Literal::Num(rhs), v) => {
            let lhs = match v {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.parse::<f64>().ok(),
                Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                _ => None,
            };
            match lhs {
                Some(l) => cmp_num(l, *rhs, when.op),
                None => false,
            }
        }
        (Literal::Str(rhs), v) => {
            let lhs = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => return false,
            };
            match when.op {
                Op::Eq => lhs == *rhs,
                Op::Ne => lhs != *rhs,
                _ => false,
            }
        }
    }
}

fn cmp_num(lhs: f64, rhs: f64, op: Op) -> bool {
    match op {
        Op::Eq => lhs == rhs,
        Op::Ne => lhs != rhs,
        Op::Ge => lhs >= rhs,
        Op::Le => lhs <= rhs,
        Op::Gt => lhs > rhs,
        Op::Lt => lhs < rhs,
    }
}

/// Decide whether `rule` applies to the event represented by `ctx`.
/// `type_name` is the serde discriminator (e.g. `"StatusChanged"`).
pub fn matches(rule: &CompiledRule, type_name: &str, ctx: &Value) -> bool {
    if rule.disabled {
        return false;
    }
    if !rule.on.is_empty() && !rule.on.iter().any(|n| n == type_name) {
        return false;
    }
    match &rule.when {
        None => true,
        Some(w) => eval_when(w, ctx),
    }
}

/// Stable hash for the event's identity. Picks the first available
/// "key" field in priority order. The hash itself is small + fast
/// (FNV-1a) — collisions are fine because the rule index already
/// participates in the debounce key.
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

/// Tracks last-fired timestamps for `(rule_idx, key_hash)` pairs.
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
    use serde_json::json;

    #[test]
    fn parse_when_all_operators() {
        for (src, op) in [
            ("x == 1", Op::Eq),
            ("x != 1", Op::Ne),
            ("x >= 1", Op::Ge),
            ("x <= 1", Op::Le),
            ("x > 1", Op::Gt),
            ("x < 1", Op::Lt),
        ] {
            let w = parse_when(src).unwrap();
            assert_eq!(w.op, op, "wrong op for {src}");
            assert_eq!(w.field, "x");
        }
    }

    #[test]
    fn parse_when_string_literal() {
        let w = parse_when("provider == \"claude\"").unwrap();
        assert_eq!(w.field, "provider");
        match w.literal {
            Literal::Str(s) => assert_eq!(s, "claude"),
            _ => panic!("expected string literal"),
        }
    }

    #[test]
    fn parse_when_rejects_junk() {
        assert!(parse_when("").is_err());
        assert!(parse_when("no_op_here").is_err());
        assert!(parse_when("x ?? 1").is_err());
        assert!(parse_when("x ==").is_err());
        assert!(parse_when("x == \"unterminated").is_err());
        assert!(parse_when("x == 1 garbage").is_err()); // numeric tail
        assert!(parse_when("x == \"ok\" garbage").is_err());
    }

    #[test]
    fn matches_wildcard_on_list() {
        let rule = CompiledRule {
            on: vec![],
            when: None,
            title: String::new(),
            body: String::new(),
            debounce_ms: None,
            idx: 0,
            disabled: false,
            disable_reason: None,
        };
        assert!(matches(&rule, "AnyEvent", &json!({})));
    }

    #[test]
    fn matches_filters_by_type_name() {
        let rule = CompiledRule {
            on: vec!["StatusChanged".into()],
            when: None,
            title: String::new(),
            body: String::new(),
            debounce_ms: None,
            idx: 0,
            disabled: false,
            disable_reason: None,
        };
        assert!(matches(&rule, "StatusChanged", &json!({})));
        assert!(!matches(&rule, "RateLimited", &json!({})));
    }

    #[test]
    fn matches_with_when_numeric_threshold() {
        let rule = CompiledRule {
            on: vec!["ContextThreshold".into()],
            when: Some(parse_when("bucket >= 90").unwrap()),
            title: String::new(),
            body: String::new(),
            debounce_ms: None,
            idx: 0,
            disabled: false,
            disable_reason: None,
        };
        assert!(matches(&rule, "ContextThreshold", &json!({"bucket": 90})));
        assert!(matches(&rule, "ContextThreshold", &json!({"bucket": 95})));
        assert!(!matches(&rule, "ContextThreshold", &json!({"bucket": 70})));
    }

    #[test]
    fn matches_with_when_string_eq() {
        let rule = CompiledRule {
            on: vec!["RateLimited".into()],
            when: Some(parse_when("provider == \"claude\"").unwrap()),
            title: String::new(),
            body: String::new(),
            debounce_ms: None,
            idx: 0,
            disabled: false,
            disable_reason: None,
        };
        assert!(matches(
            &rule,
            "RateLimited",
            &json!({"provider": "claude"})
        ));
        assert!(!matches(
            &rule,
            "RateLimited",
            &json!({"provider": "openai"})
        ));
    }

    #[test]
    fn disabled_rule_never_matches() {
        let rule = CompiledRule {
            on: vec![],
            when: None,
            title: String::new(),
            body: String::new(),
            debounce_ms: None,
            idx: 0,
            disabled: true,
            disable_reason: Some("test".into()),
        };
        assert!(!matches(&rule, "X", &json!({})));
    }

    #[test]
    fn compile_marks_invalid_when_disabled() {
        let rules = vec![Rule {
            on: vec![],
            when: Some("not a valid expr".into()),
            title: "t".into(),
            body: "b".into(),
            debounce_ms: None,
        }];
        let out = compile(rules);
        assert_eq!(out.len(), 1);
        assert!(out[0].disabled);
        assert!(out[0].disable_reason.is_some());
    }

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
        // Different rule_idx or key_hash -> independent.
        assert!(d.allow((1, 1), 1000));
        assert!(d.allow((0, 2), 1000));
    }

    #[test]
    fn event_key_hash_prefers_session_id() {
        let h1 = event_key_hash(&json!({"session_id": "abc", "provider": "claude"}));
        let h2 = event_key_hash(&json!({"session_id": "abc", "provider": "openai"}));
        let h3 = event_key_hash(&json!({"session_id": "xyz", "provider": "claude"}));
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }
}
