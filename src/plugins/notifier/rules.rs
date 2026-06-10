//! Rule matching + debouncing for the notifier plugin.
//!
//! v1 grammar for `when` is intentionally tiny: exactly one
//! `<field> <op> <literal>` comparison, no `and`/`or`. Users who need
//! conjunction write two rules. Parsing happens once at config load —
//! invalid `when` expressions disable just the offending rule and log
//! a single line, so a typo can't take down the worker.

use serde::Deserialize;
use serde_json::Value;

// Re-exports of primitives that used to live in this file but now sit
// under `plugins::common`. Keeping the re-exports preserves
// `notifier::rules::Debouncer` / `notifier::rules::event_key_hash`
// callsites unchanged.
pub use crate::plugins::common::debounce::Debouncer;
pub use crate::plugins::common::event_key::event_key_hash;

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
}
