# Phase B4 — Malformed-theme banner — design

**Status:** Approved (local fork)
**Date:** 2026-06-09
**Scope:** Phase B item 4. Surface theme parse errors in the footer status message instead of silent fallback.
**Target:** `~/my-workspace/util/abtop` (local fork at `ba4e058`). No upstream PR.
**Builds on:** Phase A + B1 + B2 + B3 — `theme::loader::parse_theme_body`, `theme::load_from_path`, `theme::load_or_default`, `App::set_status`.

## Goal

When a user's theme file (`$XDG_CONFIG_HOME/abtop/themes/<name>.theme`, or `--theme <path>`) contains parse errors, the user sees a transient footer message at launch: `theme '<name>' has N parse errors`. Today those errors are silently swallowed and the theme falls back to btop defaults for the failing fields — confusing during iteration.

## Non-goals

- No persistent banner / dismiss-required UI (a transient `status_msg` is enough; B4 is a polish item, not a critical alert).
- No detailed in-banner content (line numbers, offending values) — the count alone signals "look at your file." Detail can come in B5+ when a logger / verbose-mode lands.
- No stderr output before TUI launch. The footer is the only surface.
- No new key binding to view error details.
- Embedded themes do NOT produce errors in practice (the parity test loop in Phase A enforced byte-equality with the Rust constructors; the smoke test in B1 enforces full-palette population). Code path still works for them; just always returns an empty error list.
- `--list-themes` / `--dump-theme` don't load themes for use; their behavior is unchanged.

## Error detection rules

`parse_theme_body_with_errors` walks each line in order and applies these checks:

1. **Skip** lines that don't start with `theme[` after `trim()`. (Comments, blanks, anything off-format — intentional, not an error.)
2. **`Malformed`** if a `theme[`-prefixed line can't be tokenized into `(key, value)`. Cases: no closing `]`, no `=`, no quote pair around the value.
3. **`UnknownKey`** if the key parsed cleanly but isn't in the 33-key set handled by `apply_kv`.
4. **`InvalidHex`** if the key was recognized AND the value is non-empty AND `parse_hex(value)` returned `None`. Empty values are valid (`Color::Reset` semantic) and don't count as errors.

Each error captures the **1-indexed line number** and the **trimmed line content**, plus the reason variant.

## Architecture

The parser layer gains an error-collecting twin function. Existing callers that don't care about errors keep working unchanged. New callers in `lib.rs::run()` use the `_with_errors` variants and stash the result on the constructed `App` via the existing `set_status` mechanism.

```
                          startup (lib.rs::run())
                                   │
                                   ▼
          let (theme, errors) = load_*_with_errors(...)
                                   │
                                   ▼
                          build_app(theme, &cfg)
                                   │
                                   ▼
                if !errors.is_empty():
                    app.set_status("theme 'X' has N parse errors")
                                   │
                                   ▼
                          run TUI event loop
                          (footer renders status_msg for 3s)
```

## Code changes

### `src/theme/loader.rs` — new types and `_with_errors` variants

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

**New `parse_theme_body_with_errors`** — does the actual line walk:

```rust
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
```

**Existing `parse_theme_body` becomes a thin wrapper** so the ~12 existing call sites + tests don't churn:

```rust
pub(crate) fn parse_theme_body(body: &str, name: &str) -> Theme {
    parse_theme_body_with_errors(body, name).0
}
```

**`classify_line` is the new tokenizer** that returns either `Ok((key, value))` or `Err(ParseErrorReason)`:

```rust
fn classify_line(line: &str) -> Result<(&str, &str), ParseErrorReason> {
    let rest = line.strip_prefix("theme[")
        .ok_or(ParseErrorReason::Malformed)?;
    let (key, rest) = rest.split_once(']')
        .ok_or(ParseErrorReason::Malformed)?;
    let val_part = rest.trim_start()
        .strip_prefix('=')
        .ok_or(ParseErrorReason::Malformed)?
        .trim_start();
    let value = val_part
        .strip_prefix('"').and_then(|s| s.strip_suffix('"'))
        .or_else(|| val_part.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
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

Note: `classify_line` returns the same `(key, value)` shape as the existing `parse_line` so `apply_kv` (unchanged) consumes it. The existing `parse_line` stays around for the legacy `apply_body` path used by the btop seed (no error reporting needed for the embedded body).

**`is_known_key`** is a small helper that returns true if `key` is one of the 33 keys handled by `apply_kv`:

```rust
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

(Yes, this duplicates the key list from `apply_kv`. Acceptable for now; a future refactor could extract a shared constant. Documented as a known mild-duplication in the spec.)

**New `load_from_path_with_errors`:**

```rust
pub(crate) fn load_from_path_with_errors(path: &Path) -> Result<(Theme, Vec<ParseError>), String> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("custom")
        .to_string();
    Ok(parse_theme_body_with_errors(&body, &name))
}
```

Existing `load_from_path` becomes a thin wrapper:

```rust
pub(crate) fn load_from_path(path: &Path) -> Result<Theme, String> {
    load_from_path_with_errors(path).map(|(t, _)| t)
}
```

**New `load_or_default_with_errors`:**

```rust
pub(crate) fn load_or_default_with_errors(name: &str, cfg: &AppConfig) -> (Theme, Vec<ParseError>) {
    let (mut theme, errors) = lookup_chain_with_errors(&crate::config::xdg_config_dir(), name);
    apply_overrides(&mut theme, cfg);
    (theme, errors)
}
```

Where `lookup_chain_with_errors` mirrors the existing `lookup_chain`'s lookup priority but returns the parser errors too:

```rust
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
    // Last-resort fallback: embedded btop, no possibility of errors.
    let body = crate::theme::embedded::lookup("btop")
        .expect("embedded btop is a build-time invariant");
    (parse_theme_body(body, "btop"), Vec::new())
}
```

`try_user_file_body` is a small refactor of the existing `try_user_file` — splits the read step from the parse step so we can route the body through `parse_theme_body_with_errors` instead of `parse_theme_body`. The existing `try_user_file` (used elsewhere) stays as a thin wrapper:

```rust
fn try_user_file_body(config_root: &Path, name: &str) -> Option<String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    let path = config_root.join("abtop").join("themes").join(format!("{name}.theme"));
    std::fs::read_to_string(&path).ok()
}

fn try_user_file(config_root: &Path, name: &str) -> Option<Theme> {
    try_user_file_body(config_root, name).map(|body| parse_theme_body(&body, name))
}
```

### `src/theme/mod.rs` — extend the re-export

Add the new `_with_errors` variants and the `ParseError`/`ParseErrorReason` types to the existing `pub(crate) use loader::{...}` line:

```rust
pub(crate) use loader::{
    apply_overrides, dump_embedded, list_available,
    load_from_path, load_from_path_with_errors,
    load_or_default, load_or_default_with_errors,
    ParseError, ParseErrorReason, Source,
};
```

### `src/lib.rs::run()` — use the `_with_errors` variants and stash

Replace the existing match block (post-B3) with one that captures errors:

```rust
let (initial_theme, parse_errors): (theme::Theme, Vec<theme::ParseError>) = match &cli_theme_name {
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

After `build_app(initial_theme, &cfg)` returns, set a status message if errors are present:

```rust
let mut app = build_app(initial_theme, &cfg);
if !parse_errors.is_empty() {
    let theme_name = app.theme.name.clone();
    app.set_status(format!(
        "theme '{}' has {} parse error{}",
        theme_name,
        parse_errors.len(),
        if parse_errors.len() == 1 { "" } else { "s" }
    ));
}
```

(Singular/plural matters slightly; cheap to do.)

## Public surface change

- `theme::ParseError`, `theme::ParseErrorReason` — new types, `pub(crate)`.
- `theme::parse_theme_body_with_errors`, `theme::load_from_path_with_errors`, `theme::load_or_default_with_errors` — new functions, `pub(crate)`.
- `theme::loader::classify_line`, `is_known_key`, `try_user_file_body`, `lookup_chain_with_errors` — new module-private helpers.

No new external crate API.

## Behavior table

| Theme source | Errors in file? | Footer at launch | Theme actually used |
|---|---|---|---|
| Embedded only (e.g. `--theme catppuccin`) | (always 0) | No status | Embedded catppuccin |
| User-dir clean file | 0 | No status | User file's values |
| User-dir file with bad hex | 1 | `theme 'X' has 1 parse error` (3s) | Errors-fields fall back to btop default; clean fields applied |
| `--theme /tmp/x.theme` clean | 0 | No status | Path file |
| `--theme /tmp/x.theme` malformed | N | `theme 'X' has N parse errors` (3s) | Same fallback semantics as above |

## Test surface

In `src/theme/loader.rs`:

- `parse_theme_body_with_errors_reports_invalid_hex` — `theme[main_bg]="#XYZ"` → 1 error, `ParseErrorReason::InvalidHex("#XYZ")`, line 1.
- `parse_theme_body_with_errors_reports_unknown_key` — `theme[wrong_key]="#fff"` → 1 error, `ParseErrorReason::UnknownKey("wrong_key")`.
- `parse_theme_body_with_errors_reports_malformed_lines` — multiple bad-shape lines → multiple `Malformed`.
- `parse_theme_body_with_errors_includes_correct_line_numbers` — errors on lines 3 and 7 → reported as 3 and 7 (1-indexed).
- `parse_theme_body_with_errors_ignores_comments_and_blanks` — comments + blanks → 0 errors.
- `parse_theme_body_with_errors_clean_file_returns_no_errors` — embedded catppuccin body → 0 errors.
- `every_embedded_theme_parses_with_zero_errors` — loop over `BUILTIN` and assert each produces an empty errors vec. Locks in the "embedded = always clean" invariant.

Existing parse_theme_body tests stay unchanged.

In `src/lib.rs`: integration is manual-smoke only (the banner display path is exercised end-to-end by dropping a malformed file and observing the footer at launch).

Manual smoke:

```sh
mkdir -p ~/.config/abtop/themes
cat > ~/.config/abtop/themes/broken.theme <<'EOF'
theme[main_bg]="#XYZ"
theme[wrong_key]="#fff"
theme[main_bg]=missingquotes
EOF
abtop --theme broken
# Expect at launch: footer shows "theme 'broken' has 3 parse errors" for 3s
# Then auto-clears; the theme still loads (with broken fields falling back to btop)
rm ~/.config/abtop/themes/broken.theme
```

## Error handling philosophy

Matches Phase A's infallible-where-possible discipline:
- `parse_theme_body_with_errors` is still infallible — it always returns a Theme, plus a (possibly empty) errors vec. Loading never panics.
- File-read errors in `load_from_path_with_errors` propagate as `Err(String)`, same as today.
- `lookup_chain_with_errors` and `load_or_default_with_errors` are infallible.
- The footer message is best-effort; if `set_status` somehow fails (it can't, today — it's just an option assignment) the user just doesn't see the banner.

## Implementation notes

- `is_known_key` duplicates the key list from `apply_kv`. Acceptable for now; a future refactor could extract a shared `&'static [&str]` constant. The full-palette test already locks in the 33-key surface.
- `try_user_file_body` is a refactor split — extract the file-read into its own function. The existing `try_user_file` becomes a one-line wrapper.
- The parser's order of error detection matters for the test assertions: `Malformed` is checked before `UnknownKey` (key must parse first), `InvalidHex` is checked last (key must be recognized first).

## Build & install

Same as Phase A/B1/B2/B3:

```sh
cd ~/my-workspace/util/abtop
cargo test --lib
cargo build --release
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

## Acceptance criteria

1. `cargo test --lib` passes (test count grows by ~7 new tests).
2. `cargo build --release` clean with no warnings.
3. A user-dir theme with malformed hex shows a footer message at launch.
4. A path theme with malformed hex shows the same footer.
5. The theme still loads — error fields fall back to btop defaults — exactly as before B4. No regression in successful-load semantics.
6. Embedded themes never produce errors (verified by `every_embedded_theme_parses_with_zero_errors`).
7. `--list-themes` and `--dump-theme` are unaffected.
8. The existing 5+ `parse_theme_body` tests pass unchanged.

## Out of scope (other Phase B items)

- B5: Reload-on-file-change — separate spec.
- B6: macOS Library → XDG migration — deferred indefinitely.
