//! Template substitution for notifier titles/bodies.
//!
//! Tokens of the form `{field}` are looked up against a flat
//! [`serde_json::Value::Object`] (the serialized event plus a small set of
//! wire-level extras: `kind`, `ts_ms`, `v`). Missing fields render as the
//! empty string — templates never panic on absent data.
//!
//! For the `Osascript` backend we also expose [`escape_for_osascript`]
//! which escapes `"` and `\` so the final string can be spliced into an
//! AppleScript string literal without breaking the surrounding quotes.

use serde_json::Value;

/// Substitute `{field}` tokens in `template` against the JSON object
/// `ctx`. A `field` is one or more characters that are NOT `}`. An
/// unmatched `{` is emitted verbatim. Missing or non-string scalar
/// fields are stringified; objects/arrays serialize as JSON.
pub fn render(template: &str, ctx: &Value) -> String {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'{' {
            // Find the matching `}`. If none, treat the `{` as literal.
            if let Some(end_rel) = bytes[i + 1..].iter().position(|c| *c == b'}') {
                let end = i + 1 + end_rel;
                let key = &template[i + 1..end];
                out.push_str(&lookup(ctx, key));
                i = end + 1;
                continue;
            }
        }
        // Push one full UTF-8 char starting at `i`.
        let ch_len = utf8_char_len(b);
        out.push_str(&template[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Look up `key` against the JSON object. Missing field => empty string.
/// Scalars stringify directly; strings unwrap (so no surrounding quotes
/// in the rendered output); objects/arrays serialize as compact JSON.
fn lookup(ctx: &Value, key: &str) -> String {
    let obj = match ctx.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    match obj.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(v) => v.to_string(),
    }
}

fn utf8_char_len(first_byte: u8) -> usize {
    match first_byte {
        b if b < 0x80 => 1,
        b if (b & 0xE0) == 0xC0 => 2,
        b if (b & 0xF0) == 0xE0 => 3,
        b if (b & 0xF8) == 0xF0 => 4,
        _ => 1,
    }
}

/// Escape `s` for splicing into an AppleScript double-quoted string
/// literal. Backslashes first (so the escapes we just added don't get
/// double-escaped), then double-quotes.
pub fn escape_for_osascript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn substitutes_string_fields() {
        let ctx = json!({"session_id": "abc", "tool": "Read"});
        assert_eq!(
            render("session {session_id} ran {tool}", &ctx),
            "session abc ran Read"
        );
    }

    #[test]
    fn missing_field_renders_empty() {
        let ctx = json!({"a": 1});
        assert_eq!(render("[{missing}]", &ctx), "[]");
    }

    #[test]
    fn unmatched_open_brace_is_literal() {
        let ctx = json!({});
        // No closing brace -> the `{` stays verbatim.
        assert_eq!(render("hello { world", &ctx), "hello { world");
    }

    #[test]
    fn numeric_field_stringifies() {
        let ctx = json!({"pid": 1234, "load1": 4.5});
        assert_eq!(render("pid={pid} load={load1}", &ctx), "pid=1234 load=4.5");
    }

    #[test]
    fn no_tokens_passes_through() {
        let ctx = json!({});
        assert_eq!(render("plain text", &ctx), "plain text");
    }

    #[test]
    fn empty_template_yields_empty() {
        let ctx = json!({"x": 1});
        assert_eq!(render("", &ctx), "");
    }

    #[test]
    fn handles_utf8_in_template() {
        let ctx = json!({"name": "café"});
        assert_eq!(render("→ {name} ☕", &ctx), "→ café ☕");
    }

    #[test]
    fn osascript_escape_handles_quotes_and_backslashes() {
        assert_eq!(escape_for_osascript(r#"a"b\c"#), r#"a\"b\\c"#);
        // Order matters: backslash first, then quote. A raw `\"` becomes `\\\"`.
        assert_eq!(escape_for_osascript(r#"\"#), r#"\\"#);
        assert_eq!(escape_for_osascript(""), "");
    }
}
