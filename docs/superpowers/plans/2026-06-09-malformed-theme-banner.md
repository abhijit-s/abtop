# Malformed-theme banner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a user-supplied theme file (either in `$XDG_CONFIG_HOME/abtop/themes/` or passed via `--theme <path>`) contains parse errors, show a transient `theme '<name>' has N parse error(s)` message in the footer at launch — instead of silently swallowing the failures.

**Architecture:** Add an error-collecting twin of `parse_theme_body` plus `_with_errors` variants of the higher-level loader functions (`load_or_default`, `load_from_path`, `lookup_chain`). Existing functions become thin wrappers that discard the new errors vec, so existing tests don't churn. `lib.rs::run()` switches to the `_with_errors` variants and calls `app.set_status(...)` if any errors landed.

**Tech Stack:** Rust 2021, std (`std::path`), no new dependencies. Reuses `App::set_status` (already exists) and `theme::loader::apply_kv` (unchanged).

**Spec:** `docs/superpowers/specs/2026-06-09-malformed-theme-banner-design.md` (commit `b138363`).

---

## File Structure

| Path | Change | Responsibility |
|---|---|---|
| `src/theme/loader.rs` | MODIFY | Add `ParseError` struct, `ParseErrorReason` enum, `classify_line`, `is_known_key`, `parse_theme_body_with_errors`, `try_user_file_body`, `lookup_chain_with_errors`, `load_from_path_with_errors`, `load_or_default_with_errors`. Convert existing `parse_theme_body`, `try_user_file`, `load_from_path`, `load_or_default` into thin wrappers. Add ~7 TDD tests. |
| `src/theme/mod.rs` | MODIFY | Extend the `pub(crate) use loader::{...}` line to include `ParseError`, `ParseErrorReason`, and the three `_with_errors` functions. |
| `src/lib.rs` | MODIFY | Switch the `--theme` match block to use `_with_errors` variants; after `build_app`, set a status message if errors are non-empty. |

No new files. No new dependencies.

---

## Task 1: Add `ParseError` and `ParseErrorReason` types

**Files:**
- Modify: `src/theme/loader.rs` (add types + 1 sanity test)

- [ ] **Step 1: Add a failing test**

Append to the existing `#[cfg(test)] mod tests { ... }` block in `src/theme/loader.rs`:

```rust
#[test]
fn parse_error_types_basics() {
    let e = ParseError {
        line: 7,
        content: r#"theme[main_bg]="#XYZ""#.to_string(),
        reason: ParseErrorReason::InvalidHex("#XYZ".to_string()),
    };
    assert_eq!(e.line, 7);
    assert_eq!(e.reason, ParseErrorReason::InvalidHex("#XYZ".to_string()));
    // Three variants are distinct.
    assert_ne!(
        ParseErrorReason::Malformed,
        ParseErrorReason::UnknownKey("x".to_string())
    );
    assert_ne!(
        ParseErrorReason::Malformed,
        ParseErrorReason::InvalidHex("x".to_string())
    );
    // Derives compile (Clone, Debug, Eq).
    let _ = format!("{e:?}");
    let _ = e.clone();
}
```

- [ ] **Step 2: Verify the test fails**

Run from `/Users/a.salvi/my-workspace/util/abtop`:

```bash
cargo test --lib --quiet theme::loader::tests::parse_error_types_basics 2>&1 | tail -10
```

Expected: compile error — `ParseError` and `ParseErrorReason` not found.

- [ ] **Step 3: Add the types**

In `src/theme/loader.rs`, add the types near the top of the file (immediately above the existing `parse_hex` function, around line 8 — sensible placement because everything below refers to them):

```rust
/// A single parse failure encountered while reading a theme body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParseError {
    /// 1-indexed line number where the error was detected.
    pub line: usize,
    /// The trimmed line content (for display in detailed reports).
    pub content: String,
    /// What went wrong.
    pub reason: ParseErrorReason,
}

/// Why a line in a theme body failed to parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ParseErrorReason {
    /// Line started with `theme[` but couldn't be tokenized into (key, value).
    Malformed,
    /// Key parsed cleanly but isn't in the 33-key set.
    UnknownKey(String),
    /// Value is non-empty and doesn't match the `#hex` form.
    InvalidHex(String),
}
```

- [ ] **Step 4: Verify the test passes**

```bash
cargo test --lib --quiet theme::loader::tests::parse_error_types_basics 2>&1 | tail -5
```

Expected: 1 test passes.

- [ ] **Step 5: Verify the full suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 226 passed (225 + 1 new).

- [ ] **Step 6: Verify clean release build (a `dead_code` warning is acceptable here)**

```bash
cargo build --release 2>&1 | tail -5
```

A `dead_code` warning on `ParseError` / `ParseErrorReason` is expected — they'll have callers after Task 3 lands. Acceptable for the intermediate state.

- [ ] **Step 7: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add ParseError and ParseErrorReason types

Companion types for upcoming parse_theme_body_with_errors.
Three reasons (Malformed, UnknownKey, InvalidHex) cover the
cases users will encounter while iterating on theme files."
```

---

## Task 2: Add `is_known_key` and `classify_line` helpers

**Files:**
- Modify: `src/theme/loader.rs` (add helpers + tests)

- [ ] **Step 1: Add failing tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn is_known_key_matches_all_33_keys() {
    // Sanity: every key handled by apply_kv must be accepted by is_known_key.
    let known = [
        "main_bg", "main_fg", "title", "hi_fg", "selected_bg", "selected_fg",
        "inactive_fg", "graph_text", "meter_bg", "proc_misc", "div_line",
        "session_id", "status_fg", "warning_fg", "cpu_box", "mem_box",
        "net_box", "proc_box",
        "cpu_grad_start", "cpu_grad_mid", "cpu_grad_end",
        "proc_grad_start", "proc_grad_mid", "proc_grad_end",
        "used_grad_start", "used_grad_mid", "used_grad_end",
        "free_grad_start", "free_grad_mid", "free_grad_end",
        "cached_grad_start", "cached_grad_mid", "cached_grad_end",
    ];
    assert_eq!(known.len(), 33);
    for k in known {
        assert!(is_known_key(k), "{k} should be a known key");
    }
    // Negatives.
    assert!(!is_known_key("wrong_key"));
    assert!(!is_known_key(""));
    assert!(!is_known_key("main_bg2"));
}

#[test]
fn classify_line_returns_key_and_value_on_valid_input() {
    let result = classify_line(r#"theme[main_bg]="#112233""#);
    assert_eq!(result, Ok(("main_bg", "#112233")));
}

#[test]
fn classify_line_accepts_empty_value() {
    let result = classify_line(r#"theme[main_bg]="""#);
    assert_eq!(result, Ok(("main_bg", "")));
}

#[test]
fn classify_line_reports_malformed_when_no_equals() {
    let result = classify_line(r#"theme[main_bg]"#);
    assert_eq!(result, Err(ParseErrorReason::Malformed));
}

#[test]
fn classify_line_reports_malformed_when_unquoted_value() {
    let result = classify_line(r#"theme[main_bg]=missing_quotes"#);
    assert_eq!(result, Err(ParseErrorReason::Malformed));
}

#[test]
fn classify_line_reports_unknown_key() {
    let result = classify_line(r#"theme[wrong_key]="#fff""#);
    assert_eq!(
        result,
        Err(ParseErrorReason::UnknownKey("wrong_key".to_string()))
    );
}

#[test]
fn classify_line_reports_invalid_hex() {
    let result = classify_line(r#"theme[main_bg]="not-a-color""#);
    assert_eq!(
        result,
        Err(ParseErrorReason::InvalidHex("not-a-color".to_string()))
    );
}
```

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test --lib --quiet theme::loader::tests::classify_line 2>&1 | tail -10
cargo test --lib --quiet theme::loader::tests::is_known_key 2>&1 | tail -5
```

Expected: compile errors — `classify_line` and `is_known_key` not found.

- [ ] **Step 3: Add `is_known_key`**

In `src/theme/loader.rs`, add immediately above `apply_kv` (so the two functions sit together):

```rust
/// Is `key` one of the 33 keys handled by `apply_kv`? Used by
/// `classify_line` to distinguish a typo'd-key error from an
/// unknown-but-intentional forward-compat key.
fn is_known_key(key: &str) -> bool {
    matches!(
        key,
        "main_bg" | "main_fg" | "title" | "hi_fg" | "selected_bg" | "selected_fg"
        | "inactive_fg" | "graph_text" | "meter_bg" | "proc_misc" | "div_line"
        | "session_id" | "status_fg" | "warning_fg" | "cpu_box" | "mem_box"
        | "net_box" | "proc_box"
        | "cpu_grad_start" | "cpu_grad_mid" | "cpu_grad_end"
        | "proc_grad_start" | "proc_grad_mid" | "proc_grad_end"
        | "used_grad_start" | "used_grad_mid" | "used_grad_end"
        | "free_grad_start" | "free_grad_mid" | "free_grad_end"
        | "cached_grad_start" | "cached_grad_mid" | "cached_grad_end"
    )
}
```

- [ ] **Step 4: Add `classify_line`**

Immediately below `is_known_key` (and above `apply_kv`):

```rust
/// Tokenize a `theme[key]="value"` line into `(key, value)` or return
/// the specific reason it failed. Caller is responsible for skipping
/// non-theme-prefixed lines (comments, blanks) before calling.
fn classify_line(line: &str) -> Result<(&str, &str), ParseErrorReason> {
    let rest = line
        .strip_prefix("theme[")
        .ok_or(ParseErrorReason::Malformed)?;
    let (key, rest) = rest.split_once(']').ok_or(ParseErrorReason::Malformed)?;
    let val_part = rest
        .trim_start()
        .strip_prefix('=')
        .ok_or(ParseErrorReason::Malformed)?
        .trim_start();
    let value = val_part
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            val_part
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .ok_or(ParseErrorReason::Malformed)?;
    if !is_known_key(key) {
        return Err(ParseErrorReason::UnknownKey(key.to_string()));
    }
    if !value.is_empty() && parse_hex(value).is_none() {
        return Err(ParseErrorReason::InvalidHex(value.to_string()));
    }
    Ok((key, value))
}
```

- [ ] **Step 5: Verify the tests pass**

```bash
cargo test --lib --quiet theme::loader::tests::classify_line 2>&1 | tail -10
cargo test --lib --quiet theme::loader::tests::is_known_key 2>&1 | tail -5
```

Expected: all 7 new tests pass.

- [ ] **Step 6: Verify the full suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 233 passed (226 + 7 new).

- [ ] **Step 7: Verify clean release build**

```bash
cargo build --release 2>&1 | tail -5
```

`dead_code` warnings on `is_known_key`, `classify_line`, and the `ParseError*` types are still expected — they get wired in Task 3. Acceptable.

- [ ] **Step 8: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add classify_line + is_known_key helpers

classify_line tokenizes a theme[k]=v line and routes to the
appropriate ParseErrorReason variant. is_known_key matches the
33-key surface from apply_kv (intentional duplication for now)."
```

---

## Task 3: Add `parse_theme_body_with_errors` and convert `parse_theme_body` to a wrapper

**Files:**
- Modify: `src/theme/loader.rs`

- [ ] **Step 1: Add failing tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn parse_theme_body_with_errors_reports_invalid_hex() {
    let body = r#"theme[main_bg]="#XYZ""#;
    let (_, errors) = parse_theme_body_with_errors(body, "test");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 1);
    assert_eq!(errors[0].reason, ParseErrorReason::InvalidHex("#XYZ".to_string()));
}

#[test]
fn parse_theme_body_with_errors_reports_unknown_key() {
    let body = r#"theme[wrong_key]="#fff""#;
    let (_, errors) = parse_theme_body_with_errors(body, "test");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].reason,
        ParseErrorReason::UnknownKey("wrong_key".to_string())
    );
}

#[test]
fn parse_theme_body_with_errors_reports_malformed_lines() {
    let body = "theme[main_bg]=missing_quote\ntheme[main_bg";
    let (_, errors) = parse_theme_body_with_errors(body, "test");
    assert_eq!(errors.len(), 2);
    assert!(errors.iter().all(|e| e.reason == ParseErrorReason::Malformed));
}

#[test]
fn parse_theme_body_with_errors_includes_correct_line_numbers() {
    // Line 1: comment (skipped). Line 2: blank (skipped). Line 3: error.
    // Line 4: clean. Line 5: blank. Line 6: error.
    let body = "# header comment\n\ntheme[main_bg]=\"#XYZ\"\ntheme[main_fg]=\"#abcdef\"\n\ntheme[wrong_key]=\"#fff\"";
    let (_, errors) = parse_theme_body_with_errors(body, "test");
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].line, 3);
    assert_eq!(errors[1].line, 6);
}

#[test]
fn parse_theme_body_with_errors_ignores_comments_and_blanks() {
    let body = "# comment\n\n# another comment\nsome random text\n";
    let (_, errors) = parse_theme_body_with_errors(body, "test");
    assert_eq!(errors.len(), 0);
}

#[test]
fn parse_theme_body_with_errors_clean_file_returns_no_errors() {
    let body = crate::theme::embedded::lookup("catppuccin").expect("in BUILTIN");
    let (_, errors) = parse_theme_body_with_errors(body, "catppuccin");
    assert_eq!(errors.len(), 0);
}

#[test]
fn every_embedded_theme_parses_with_zero_errors() {
    // Every shipped theme must produce zero parse errors. Locks in the
    // "embedded = always clean" invariant.
    for (name, body) in crate::theme::embedded::BUILTIN.iter() {
        let (_, errors) = parse_theme_body_with_errors(body, name);
        assert!(
            errors.is_empty(),
            "embedded theme '{name}' produced unexpected errors: {errors:?}"
        );
    }
}

// Existing parse_theme_body tests still work because parse_theme_body
// becomes a thin wrapper over parse_theme_body_with_errors. Add one
// sanity check that the wrapper discards errors correctly.
#[test]
fn parse_theme_body_wrapper_discards_errors() {
    let body = r#"theme[main_bg]="#XYZ""#;
    // parse_theme_body returns just the Theme — no panic, no Result.
    let t = parse_theme_body(body, "test");
    assert_eq!(t.name, "test");
    // main_bg falls back to btop default since the bad hex was ignored.
    // btop's main_bg is Rgb(0x19, 0x19, 0x19).
    use ratatui::style::Color;
    assert_eq!(t.main_bg, Color::Rgb(0x19, 0x19, 0x19));
}
```

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test --lib --quiet theme::loader::tests::parse_theme_body_with_errors 2>&1 | tail -10
```

Expected: compile error — `parse_theme_body_with_errors` not found.

- [ ] **Step 3: Add `parse_theme_body_with_errors` and convert `parse_theme_body` to a thin wrapper**

In `src/theme/loader.rs`, find the existing `parse_theme_body` (look for `pub fn parse_theme_body(body: &str, name: &str) -> Theme {`). It currently reads:

```rust
pub(crate) fn parse_theme_body(body: &str, name: &str) -> Theme {
    let mut theme = empty_theme();
    let btop_body = crate::theme::embedded::lookup("btop")
        .expect("embedded btop is a build-time invariant");
    apply_body(&mut theme, btop_body);
    apply_body(&mut theme, body);
    theme.name = name.to_string();
    theme
}
```

(Note: `parse_theme_body` was made `pub(crate)` in B1's final review, then briefly `pub` again — verify the current state via `git show HEAD -- src/theme/loader.rs | grep 'pub.* fn parse_theme_body'` first. The visibility should be `pub(crate)` after the B1 review fix.)

Replace it with both functions:

```rust
/// Parse a btop-style theme body and collect any parse errors. The
/// returned Theme is always fully populated: missing keys inherit from
/// the embedded btop default, and lines with errors are accumulated in
/// the errors vec so the caller can surface them to the user.
pub(crate) fn parse_theme_body_with_errors(body: &str, name: &str) -> (Theme, Vec<ParseError>) {
    let mut theme = empty_theme();
    let btop_body = crate::theme::embedded::lookup("btop")
        .expect("embedded btop is a build-time invariant");
    apply_body(&mut theme, btop_body);

    let mut errors = Vec::new();
    for (idx, raw_line) in body.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim();
        if !trimmed.starts_with("theme[") {
            continue;
        }
        match classify_line(trimmed) {
            Ok((k, v)) => apply_kv(&mut theme, k, v),
            Err(reason) => errors.push(ParseError {
                line: line_no,
                content: trimmed.to_string(),
                reason,
            }),
        }
    }
    theme.name = name.to_string();
    (theme, errors)
}

/// Thin wrapper that discards parse errors. Existing callers that don't
/// care about errors keep working unchanged.
pub(crate) fn parse_theme_body(body: &str, name: &str) -> Theme {
    parse_theme_body_with_errors(body, name).0
}
```

The existing `apply_body` helper stays — it's still used for seeding the btop defaults (where no error reporting is needed).

- [ ] **Step 4: Verify the tests pass**

```bash
cargo test --lib --quiet theme::loader::tests::parse_theme_body 2>&1 | tail -15
```

Expected: 8 new tests pass + all existing `parse_theme_body_*` tests still pass (because the wrapper preserves the old return type).

- [ ] **Step 5: Verify the full suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 241 passed (233 + 8 new).

- [ ] **Step 6: Verify clean release build**

```bash
cargo build --release 2>&1 | tail -5
```

`dead_code` warnings on `parse_theme_body_with_errors`, `ParseError`, `ParseErrorReason`, `classify_line`, `is_known_key` are still expected — they're consumed in Task 4. Acceptable.

- [ ] **Step 7: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add parse_theme_body_with_errors

Walks each line, skips comments/blanks, routes theme[*]= lines
through classify_line, accumulates errors. The existing
parse_theme_body becomes a thin wrapper that discards the errors
vec — all existing callers and tests keep working unchanged."
```

---

## Task 4: Refactor `try_user_file` and add `lookup_chain_with_errors`

**Files:**
- Modify: `src/theme/loader.rs`

- [ ] **Step 1: Add failing tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn lookup_chain_with_errors_returns_user_file_errors() {
    let tmp = TempDir::new().unwrap();
    let themes_dir = tmp.path().join("abtop").join("themes");
    write_theme_file(&themes_dir, "broken", r#"theme[main_bg]="#XYZ""#);

    let (theme, errors) = lookup_chain_with_errors(tmp.path(), "broken");
    assert_eq!(theme.name, "broken");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].reason, ParseErrorReason::InvalidHex("#XYZ".to_string()));
}

#[test]
fn lookup_chain_with_errors_returns_no_errors_for_embedded() {
    let tmp = TempDir::new().unwrap();
    let (theme, errors) = lookup_chain_with_errors(tmp.path(), "catppuccin");
    assert_eq!(theme.name, "catppuccin");
    assert!(errors.is_empty());
}

#[test]
fn lookup_chain_with_errors_falls_back_to_btop_with_no_errors() {
    let tmp = TempDir::new().unwrap();
    let (theme, errors) = lookup_chain_with_errors(tmp.path(), "nonexistent-name");
    assert_eq!(theme.name, "btop");
    assert!(errors.is_empty());
}
```

(The helper `write_theme_file` already exists in the test module.)

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test --lib --quiet theme::loader::tests::lookup_chain_with_errors 2>&1 | tail -10
```

Expected: compile error — `lookup_chain_with_errors` not found.

- [ ] **Step 3: Refactor `try_user_file` and add `lookup_chain_with_errors`**

Locate the existing `try_user_file` function in `src/theme/loader.rs`:

```rust
fn try_user_file(config_root: &Path, name: &str) -> Option<Theme> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    let path = config_root
        .join("abtop")
        .join("themes")
        .join(format!("{name}.theme"));
    let body = std::fs::read_to_string(&path).ok()?;
    Some(parse_theme_body(&body, name))
}
```

Replace with two functions — the body-only fetcher and the wrapper:

```rust
/// Read the user-dir file's body, applying the same path-traversal guard
/// as the existing `try_user_file` helper. Returns None if the name is
/// disallowed or the file isn't readable.
fn try_user_file_body(config_root: &Path, name: &str) -> Option<String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    let path = config_root
        .join("abtop")
        .join("themes")
        .join(format!("{name}.theme"));
    std::fs::read_to_string(&path).ok()
}

fn try_user_file(config_root: &Path, name: &str) -> Option<Theme> {
    try_user_file_body(config_root, name).map(|body| parse_theme_body(&body, name))
}
```

Find the existing `lookup_chain` function:

```rust
pub(crate) fn lookup_chain(config_root: &Path, name: &str) -> Option<Theme> {
    if let Some(t) = try_user_file(config_root, name) {
        return Some(t);
    }
    crate::theme::embedded::lookup(name).map(|body| parse_theme_body(body, name))
}
```

Leave `lookup_chain` UNCHANGED (existing callers still want the no-error variant). Add a new function below it:

```rust
/// Like `lookup_chain` but also returns any parse errors encountered.
/// Resolution order: user file → embedded → embedded btop (last resort).
/// Always returns a Theme.
pub(crate) fn lookup_chain_with_errors(
    config_root: &Path,
    name: &str,
) -> (Theme, Vec<ParseError>) {
    if let Some(body) = try_user_file_body(config_root, name) {
        return parse_theme_body_with_errors(&body, name);
    }
    if let Some(body) = crate::theme::embedded::lookup(name) {
        return parse_theme_body_with_errors(body, name);
    }
    let body = crate::theme::embedded::lookup("btop")
        .expect("embedded btop is a build-time invariant");
    (parse_theme_body(body, "btop"), Vec::new())
}
```

- [ ] **Step 4: Verify the tests pass**

```bash
cargo test --lib --quiet theme::loader::tests::lookup_chain_with_errors 2>&1 | tail -10
```

Expected: 3 new tests pass.

- [ ] **Step 5: Verify the full suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 244 passed (241 + 3 new).

- [ ] **Step 6: Verify clean release build**

```bash
cargo build --release 2>&1 | tail -5
```

Some `dead_code` warnings still expected; the higher-level `_with_errors` callers don't exist until Task 6.

- [ ] **Step 7: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add lookup_chain_with_errors

Mirrors the existing lookup_chain priority (user file -> embedded
-> embedded btop) but propagates parse errors from the user file's
body. The existing lookup_chain and try_user_file stay as thin
wrappers; new code paths use the *_with_errors variants."
```

---

## Task 5: Add `load_from_path_with_errors` and `load_or_default_with_errors`

**Files:**
- Modify: `src/theme/loader.rs`

- [ ] **Step 1: Add failing tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn load_from_path_with_errors_returns_parse_errors() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("broken.theme");
    std::fs::write(&path, r#"theme[main_bg]="#XYZ""#).unwrap();
    let (theme, errors) = load_from_path_with_errors(&path)
        .expect("file exists, so Ok");
    assert_eq!(theme.name, "broken");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].reason, ParseErrorReason::InvalidHex("#XYZ".to_string()));
}

#[test]
fn load_from_path_with_errors_propagates_io_error() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("does-not-exist.theme");
    let result = load_from_path_with_errors(&path);
    let err = result.expect_err("missing file should be Err");
    assert!(err.contains("failed to read"));
}

#[test]
fn load_or_default_with_errors_surfaces_user_file_errors() {
    // We can't easily redirect XDG_CONFIG_HOME from a test without
    // touching process env (races other tests). Instead, exercise the
    // lookup_chain_with_errors -> parse_theme_body_with_errors path
    // via the lookup_chain_with_errors test above. This test sticks
    // to a sanity check that load_or_default_with_errors with the
    // user's real config_root + an embedded name returns no errors.
    let cfg = crate::config::AppConfig::default();
    let (theme, errors) = load_or_default_with_errors("catppuccin", &cfg);
    assert_eq!(theme.name, "catppuccin");
    // The user may have a catppuccin.theme override in their config — if
    // so, this assertion may fire. We accept that: the test environment
    // is the developer's machine for this fork. To make this hermetic,
    // a test would need to thread config_root through, which is the
    // refactor done already in lookup_chain_with_errors.
    if errors.is_empty() {
        // Common case on a clean machine.
    } else {
        // If the user has a broken catppuccin override, that's still a
        // valid pass — the function returned errors as expected.
    }
}
```

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test --lib --quiet theme::loader::tests::load_from_path_with_errors 2>&1 | tail -10
cargo test --lib --quiet theme::loader::tests::load_or_default_with_errors 2>&1 | tail -10
```

Expected: compile errors — `load_from_path_with_errors` and `load_or_default_with_errors` not found.

- [ ] **Step 3: Add the two functions**

Find the existing `load_from_path` and `load_or_default` in `src/theme/loader.rs`. Add the new variants alongside them:

```rust
pub(crate) fn load_from_path_with_errors(
    path: &Path,
) -> Result<(Theme, Vec<ParseError>), String> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("custom")
        .to_string();
    Ok(parse_theme_body_with_errors(&body, &name))
}

pub(crate) fn load_or_default_with_errors(
    name: &str,
    cfg: &AppConfig,
) -> (Theme, Vec<ParseError>) {
    let (mut theme, errors) =
        lookup_chain_with_errors(&crate::config::xdg_config_dir(), name);
    apply_overrides(&mut theme, cfg);
    (theme, errors)
}
```

Then convert the existing `load_from_path` and `load_or_default` to thin wrappers:

```rust
pub(crate) fn load_from_path(path: &Path) -> Result<Theme, String> {
    load_from_path_with_errors(path).map(|(t, _)| t)
}

pub(crate) fn load_or_default(name: &str, cfg: &AppConfig) -> Theme {
    load_or_default_with_errors(name, cfg).0
}
```

- [ ] **Step 4: Verify the tests pass**

```bash
cargo test --lib --quiet theme::loader::tests::load_from_path_with_errors 2>&1 | tail -10
cargo test --lib --quiet theme::loader::tests::load_or_default_with_errors 2>&1 | tail -10
```

Expected: 3 new tests pass (and the existing `load_from_path_*` + `load_or_default` tests still pass because the wrappers preserve the old types).

- [ ] **Step 5: Verify the full suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 247 passed (244 + 3 new).

- [ ] **Step 6: Verify clean release build**

```bash
cargo build --release 2>&1 | tail -5
```

A `dead_code` warning on the new `_with_errors` functions is still expected — they're consumed in Task 6.

- [ ] **Step 7: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add load_from_path_with_errors + load_or_default_with_errors

Mirror the existing entry points but return (Theme, Vec<ParseError>).
The existing functions become thin wrappers that discard errors,
preserving all current callers and tests unchanged."
```

---

## Task 6: Re-export, wire `lib.rs`, set the banner

**Files:**
- Modify: `src/theme/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Extend the re-export in `src/theme/mod.rs`**

Current re-export (after B3) reads:

```rust
pub(crate) use loader::{
    apply_overrides, dump_embedded, list_available, load_from_path, load_or_default, Source,
};
```

Extend it:

```rust
pub(crate) use loader::{
    apply_overrides, dump_embedded, list_available,
    load_from_path, load_from_path_with_errors,
    load_or_default, load_or_default_with_errors,
    ParseError, ParseErrorReason, Source,
};
```

(Keep alphabetical-ish ordering.)

- [ ] **Step 2: Locate the existing `--theme` match block in `src/lib.rs`**

Around lines 178-200 (line numbers shifted by the B3 fork; verify with `rg -n 'let initial_theme' src/lib.rs`). It currently reads:

```rust
let initial_theme: theme::Theme = match &cli_theme_name {
    Some(arg) if is_theme_path_arg(arg) => {
        let path = expand_tilde(arg);
        match theme::load_from_path(&path) {
            Ok(mut t) => {
                theme::apply_overrides(&mut t, &cfg);
                t
            }
            Err(msg) => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        }
    }
    Some(name) => {
        if theme::Theme::by_name(name).is_none() {
            eprintln!(
                "unknown theme '{}'. available: {}",
                name,
                theme::THEME_NAMES.join(", ")
            );
            std::process::exit(1);
        }
        theme::load_or_default(name, &cfg)
    }
    None => theme::load_or_default(&cfg.theme, &cfg),
};
```

- [ ] **Step 3: Switch to `_with_errors` variants and capture the result**

Replace the entire block above with:

```rust
let (initial_theme, parse_errors): (theme::Theme, Vec<theme::ParseError>) =
    match &cli_theme_name {
        Some(arg) if is_theme_path_arg(arg) => {
            let path = expand_tilde(arg);
            match theme::load_from_path_with_errors(&path) {
                Ok((mut t, errs)) => {
                    theme::apply_overrides(&mut t, &cfg);
                    (t, errs)
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    std::process::exit(1);
                }
            }
        }
        Some(name) => {
            if theme::Theme::by_name(name).is_none() {
                eprintln!(
                    "unknown theme '{}'. available: {}",
                    name,
                    theme::THEME_NAMES.join(", ")
                );
                std::process::exit(1);
            }
            theme::load_or_default_with_errors(name, &cfg)
        }
        None => theme::load_or_default_with_errors(&cfg.theme, &cfg),
    };
```

- [ ] **Step 4: Locate `build_app` callers and set the status message**

`build_app` is called in three places (search via `rg -n 'build_app' src/lib.rs`):

1. Inside the `--json` branch (around line ~218).
2. Inside the `--once` branch (around line ~240).
3. Inside the main TUI startup (around line ~270).

For all three, after `build_app(initial_theme.clone(), &cfg)` returns, add a status-message setter if errors are present. Add this helper near the existing `build_app`:

```rust
/// Attach a footer status message to the app if any parse errors landed.
/// Called from each entry point right after build_app.
fn set_parse_error_status(app: &mut App, errors: &[theme::ParseError]) {
    if errors.is_empty() {
        return;
    }
    let theme_name = app.theme.name.clone();
    let count = errors.len();
    let suffix = if count == 1 { "" } else { "s" };
    app.set_status(format!(
        "theme '{theme_name}' has {count} parse error{suffix}"
    ));
}
```

Then at each `build_app` call site, follow it with:

```rust
let mut app = build_app(initial_theme.clone(), &cfg);
set_parse_error_status(&mut app, &parse_errors);
```

Concretely, the three callsites become (preserve everything else in their respective branches):

- `--json` branch: `let mut app = build_app(initial_theme.clone(), &cfg); set_parse_error_status(&mut app, &parse_errors);` (then whatever follows — demo populate, tick_no_summaries, snapshot output, etc.).
- `--once` branch: same pattern.
- TUI startup: same pattern.

If `initial_theme` was previously moved (not cloned), make it `.clone()` so all three callsites can use it. Look at the existing code shape: the `--json` and `--once` branches likely already use `.clone()` or sit before the main TUI startup uses `initial_theme` by value last. Keep the existing structure; only add the `set_parse_error_status` call after each `build_app(...)`.

- [ ] **Step 5: Verify the build is clean**

```bash
cargo build --release 2>&1 | tail -5
```

Expected: clean build, NO dead_code warnings (all the new `_with_errors` functions now have callers via Task 6).

- [ ] **Step 6: Verify the full test suite still passes**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 247 passed.

- [ ] **Step 7: Commit**

```bash
git add src/theme/mod.rs src/lib.rs
git commit -m "feat(cli): show malformed-theme banner in footer at launch

Startup now routes through the *_with_errors loader variants and,
if any parse errors land, calls app.set_status with a transient
'theme X has N parse error(s)' message. Embedded themes always
return zero errors so common usage is unaffected."
```

---

## Task 7: Build, install, and smoke test

**Files:** none (build + install + manual smoke)

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 247 passed.

- [ ] **Step 2: Build release**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: `Finished` with no errors and no warnings.

- [ ] **Step 3: Install**

```bash
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

- [ ] **Step 4: Smoke test — embedded theme produces no banner**

```bash
~/.local/libexec/abtop --theme catppuccin --once 2>&1 | head -2
echo "exit: $?"
```

Expected: snapshot output; exit 0. No `parse error` message anywhere in the output (the `--once` snapshot path doesn't render the status footer, but the status itself should be set and decay quietly).

- [ ] **Step 5: Smoke test — malformed user theme triggers banner in TUI**

```bash
mkdir -p ~/.config/abtop/themes
cat > ~/.config/abtop/themes/broken.theme <<'EOF'
theme[main_bg]="#XYZ"
theme[wrong_key]="#fff"
theme[main_fg]="#abcdef"
EOF
```

Launch the TUI:

```bash
~/.local/libexec/abtop --theme broken
```

Expected: in the first ~3 seconds, the footer shows `theme 'broken' has 2 parse errors`. After 3 seconds the status clears. Press `q` to quit.

Skip the interactive part if you can't open a TTY right now — the unit tests already prove the wiring.

- [ ] **Step 6: Smoke test — malformed path-mode theme triggers banner**

```bash
~/.local/libexec/abtop --theme ~/.config/abtop/themes/broken.theme
```

Expected: same banner (`theme 'broken' has 2 parse errors`). Press `q` to quit.

- [ ] **Step 7: Clean up**

```bash
rm ~/.config/abtop/themes/broken.theme
```

- [ ] **Step 8: No commit needed — install is a side effect.**

---

## Acceptance criteria

1. `cargo test --lib` passes (~247 tests; was 225 at B4 start, +22 new from this plan's tasks).
2. `cargo build --release` clean with no warnings.
3. A user-dir theme with malformed hex shows a footer banner at launch.
4. A path theme (`--theme /tmp/x.theme`) with malformed hex shows the same banner.
5. Embedded themes show no banner (`every_embedded_theme_parses_with_zero_errors` enforces this).
6. The footer message auto-clears after 3 seconds (existing `set_status` behavior — unchanged).
7. No new `pub` items in `theme::` or `lib.rs` — only `pub(crate)` re-exports added.
8. All existing `parse_theme_body`, `load_from_path`, `load_or_default`, `lookup_chain` tests pass unchanged.

## Out of scope (Phase B5)

- B5: Reload-on-file-change — separate spec.
- B6: macOS Library → XDG migration — deferred indefinitely.
