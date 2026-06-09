# Theme Files & Background Transparency — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move abtop's 12 hardcoded themes into external btop-compatible `.theme` files (embedded via `include_str!`), let users drop their own theme files into `$XDG_CONFIG_HOME/abtop/themes/`, and add a `theme_background` config flag that overrides any theme's background with the terminal default for transparency.

**Architecture:** A new `src/theme/` module replaces the single-file `src/theme.rs`. Built-in themes live in `themes/*.theme` at the repo root, baked into the binary at compile time via `include_str!`. A `loader::parse_theme_body` parser turns btop-style `theme[key]="#hex"` files into `Theme` structs. At startup, `Theme::load_or_default(name, &cfg)` tries the user dir, then the embedded table, then embedded `btop`, then applies `theme_background` as a post-processing override. Path resolution everywhere uses a new `xdg_config_dir()` helper (`$XDG_CONFIG_HOME` → `$HOME/.config` → platform default).

**Tech Stack:** Rust 2021, ratatui (`Color`), `dirs` crate (for `home_dir()`), `tempfile` crate (test fixtures — already a dep).

**Spec:** `docs/superpowers/specs/2026-06-09-theme-files-and-transparency-design.md` (commit `d0af5fe`).

---

## File Structure

| Path | Change | Responsibility |
|---|---|---|
| `src/theme.rs` | DELETE | Replaced by `src/theme/` module. |
| `src/theme/mod.rs` | NEW | Module root. Re-exports `Theme`, `Gradient`, `THEME_NAMES`. Holds `impl Default for Theme`. |
| `src/theme/types.rs` | NEW | `Theme` struct + `Gradient` struct definitions. Pure data types, no logic. `Theme::name` is `String`. |
| `src/theme/loader.rs` | NEW | `parse_hex`, `parse_theme_body`, `load_or_default`, `apply_overrides`. Pure functions. |
| `src/theme/embedded.rs` | NEW | `pub const BUILTIN: &[(&str, &str)]` table populated via `include_str!`. Derives `THEME_NAMES`. |
| `themes/btop.theme` | NEW | btop default palette. |
| `themes/dracula.theme` | NEW | Dracula palette. |
| `themes/catppuccin.theme` | NEW | Catppuccin Mocha palette. |
| `themes/tokyo-night.theme` | NEW | Tokyo Night night variant. |
| `themes/gruvbox.theme` | NEW | Gruvbox dark. |
| `themes/nord.theme` | NEW | Nord. |
| `themes/light.theme` | NEW | Solarized Light. |
| `themes/white.theme` | NEW | GitHub Light. |
| `themes/high-contrast.theme` | NEW | High contrast accessibility theme. |
| `themes/protanopia.theme` | NEW | Protanopia colorblind-friendly. |
| `themes/deuteranopia.theme` | NEW | Deuteranopia colorblind-friendly. |
| `themes/tritanopia.theme` | NEW | Tritanopia colorblind-friendly. |
| `src/config.rs` | MODIFY | Add `xdg_config_dir()`; add `theme_background: bool` field + parse arm; switch `config_path()` to XDG. |
| `src/app.rs` | MODIFY | `cycle_theme` calls `apply_overrides` after loading. |
| `src/lib.rs` | MODIFY | Startup path uses `Theme::load_or_default`; existing error-on-bad-name behavior preserved via `Theme::by_name`. |
| `README.md` | MODIFY | Document `~/.config/abtop/themes/`, `theme_background`, btop-compat. |

---

## Pre-Implementation

### Task 0: Reset working tree

The earlier exploratory patch (`src/theme.rs` setting catppuccin's `main_bg = Color::Reset`) is uncommitted dirt. Drop it so the implementation starts from a clean slate.

**Files:**
- Modify: `src/theme.rs` (revert)

- [ ] **Step 1: Verify dirty state**

Run: `git status --short`
Expected:
```
 M src/theme.rs
```

- [ ] **Step 2: Revert the file**

Run: `git checkout -- src/theme.rs`

- [ ] **Step 3: Confirm clean**

Run: `git status --short`
Expected: shows only untracked `docs/` directory, no modified files.

---

## Phase A1 — Config foundations

### Task 1: Add `xdg_config_dir()` helper to `config.rs`

Path resolution must consult `$XDG_CONFIG_HOME` (with `$HOME/.config` fallback) on all platforms. The `dirs` crate doesn't do this on macOS, so we write our own. Logic is extracted into a pure inner function so tests don't need to mutate process env (which would race other tests).

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Add failing tests**

Append to `#[cfg(test)] mod tests` in `src/config.rs`:

```rust
#[test]
fn xdg_config_dir_inner_uses_env_when_set() {
    let result = xdg_config_dir_inner(
        Some("/explicit/xdg".to_string()),
        Some(PathBuf::from("/home/user")),
    );
    assert_eq!(result, PathBuf::from("/explicit/xdg"));
}

#[test]
fn xdg_config_dir_inner_falls_back_to_home_when_env_empty() {
    let result = xdg_config_dir_inner(
        Some(String::new()),
        Some(PathBuf::from("/home/user")),
    );
    assert_eq!(result, PathBuf::from("/home/user/.config"));
}

#[test]
fn xdg_config_dir_inner_falls_back_to_home_when_env_missing() {
    let result = xdg_config_dir_inner(None, Some(PathBuf::from("/home/user")));
    assert_eq!(result, PathBuf::from("/home/user/.config"));
}

#[test]
fn xdg_config_dir_inner_returns_dot_when_both_missing() {
    let result = xdg_config_dir_inner(None, None);
    assert_eq!(result, PathBuf::from("."));
}
```

- [ ] **Step 2: Verify tests fail**

Run: `cargo test --lib --quiet config::tests::xdg 2>&1 | tail -20`
Expected: compilation error — `xdg_config_dir_inner` not found.

- [ ] **Step 3: Implement the helpers**

Add to `src/config.rs` (just below the existing `config_path` function — but DO NOT change `config_path` yet, that's Task 2):

```rust
fn xdg_config_dir_inner(xdg_env: Option<String>, home: Option<PathBuf>) -> PathBuf {
    if let Some(x) = xdg_env {
        if !x.is_empty() {
            return PathBuf::from(x);
        }
    }
    if let Some(h) = home {
        return h.join(".config");
    }
    PathBuf::from(".")
}

pub fn xdg_config_dir() -> PathBuf {
    xdg_config_dir_inner(std::env::var("XDG_CONFIG_HOME").ok(), dirs::home_dir())
}
```

- [ ] **Step 4: Verify tests pass**

Run: `cargo test --lib --quiet config::tests::xdg 2>&1 | tail -10`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add xdg_config_dir() helper

Resolves \$XDG_CONFIG_HOME with fallback to \$HOME/.config, on all
platforms. Inner function takes env values as args so tests stay
independent of process env state."
```

---

### Task 2: Switch `config_path()` to use `xdg_config_dir()`

This is the behavior change: on macOS, the config file moves from `~/Library/Application Support/abtop/config.toml` to `~/.config/abtop/config.toml`. Per the spec, no migration step.

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Add a failing test**

Append to `#[cfg(test)] mod tests`:

```rust
#[test]
fn config_path_uses_xdg_config_dir() {
    let path = config_path().expect("config_path should resolve");
    // The last two components must be "abtop" then "config.toml".
    let mut iter = path.components().rev();
    assert_eq!(iter.next().unwrap().as_os_str(), "config.toml");
    assert_eq!(iter.next().unwrap().as_os_str(), "abtop");
    // The parent of "abtop" must be a real config root (not "/").
    let third = iter.next().unwrap().as_os_str().to_string_lossy().to_string();
    assert!(!third.is_empty() && third != "/", "got parent: {third}");
}
```

- [ ] **Step 2: Verify test fails**

Run: `cargo test --lib --quiet config::tests::config_path_uses_xdg_config_dir 2>&1 | tail -10`
Expected: the test currently passes on Linux (dirs::config_dir() = ~/.config) but the goal is to make it pass everywhere; if running on macOS it would fail. Either way, proceed to Step 3 to make the implementation use XDG explicitly.

- [ ] **Step 3: Switch the implementation**

Replace `config_path` in `src/config.rs`:

```rust
fn config_path() -> Option<PathBuf> {
    Some(xdg_config_dir().join("abtop").join("config.toml"))
}
```

(Previously: `dirs::config_dir().map(|d| d.join("abtop").join("config.toml"))`.)

- [ ] **Step 4: Verify all config tests pass**

Run: `cargo test --lib --quiet config:: 2>&1 | tail -10`
Expected: all config tests pass (existing + new).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): resolve config.toml via XDG_CONFIG_HOME

Switches macOS lookup from ~/Library/Application Support/abtop/ to
~/.config/abtop/ so the config tree is consistent across platforms."
```

---

### Task 3: Add `theme_background` field to `AppConfig`

A new bool field, default `true` for zero behavior change.

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Add failing tests**

Append to `#[cfg(test)] mod tests`:

```rust
#[test]
fn theme_background_defaults_to_true() {
    let cfg = AppConfig::default();
    assert!(cfg.theme_background);
}

#[test]
fn parse_config_body_reads_theme_background_false() {
    let cfg = parse_config_body("theme_background = false\n");
    assert!(!cfg.theme_background);
}

#[test]
fn parse_config_body_reads_theme_background_true_explicit() {
    let cfg = parse_config_body("theme_background = true\n");
    assert!(cfg.theme_background);
}

#[test]
fn parse_config_body_keeps_default_when_theme_background_missing() {
    let cfg = parse_config_body("theme = \"btop\"\n");
    assert!(cfg.theme_background);
}

#[test]
fn parse_config_body_keeps_default_when_theme_background_garbage() {
    let cfg = parse_config_body("theme_background = maybe\n");
    assert!(cfg.theme_background);
}
```

- [ ] **Step 2: Verify tests fail**

Run: `cargo test --lib --quiet config::tests::theme_background 2>&1 | tail -10`
Expected: compilation error — `theme_background` field not found.

- [ ] **Step 3: Add the field and parse arm**

In `src/config.rs`, update the `AppConfig` struct:

```rust
pub struct AppConfig {
    pub theme: String,
    /// When false, overrides the active theme's main_bg with Color::Reset
    /// so the terminal's own background (including transparency) shows through.
    pub theme_background: bool,
    pub hidden_agents: Vec<String>,
    pub claude_config_dirs: Vec<PathBuf>,
    pub panels: PanelVisibility,
    pub language: String,
}
```

Update `Default`:

```rust
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "btop".to_string(),
            theme_background: true,
            hidden_agents: Vec::new(),
            claude_config_dirs: Vec::new(),
            panels: PanelVisibility::default(),
            language: String::new(),
        }
    }
}
```

In `parse_config_body`, add an arm in the `match key { ... }`:

```rust
"theme_background" => {
    config.theme_background = parse_bool(val).unwrap_or(true);
}
```

- [ ] **Step 4: Verify tests pass**

Run: `cargo test --lib --quiet config:: 2>&1 | tail -10`
Expected: all config tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add theme_background flag

Boolean key in config.toml; defaults to true (no behavior change).
False signals the theme loader to use Color::Reset for main_bg."
```

---

## Phase A2 — Theme module restructure

### Task 4: Split `src/theme.rs` into `src/theme/` module

A mechanical refactor with one type change: `Theme::name` becomes `String`. The 12 Rust constructors stay temporarily (deleted in Task 12 once embedded themes are the source of truth). This task ends with `cargo test` still green.

**Files:**
- Delete: `src/theme.rs`
- Create: `src/theme/mod.rs`
- Create: `src/theme/types.rs`

- [ ] **Step 1: Create `src/theme/types.rs`**

Write the type-only file:

```rust
//! Theme data types. No logic, no constructors — those live in `loader` / `embedded`.

use ratatui::style::Color;

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub start: (u8, u8, u8),
    pub mid: (u8, u8, u8),
    pub end: (u8, u8, u8),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,

    // base
    pub main_bg: Color,
    pub main_fg: Color,
    pub title: Color,
    pub hi_fg: Color,
    pub selected_bg: Color,
    pub selected_fg: Color,
    pub inactive_fg: Color,
    pub graph_text: Color,
    pub meter_bg: Color,
    pub proc_misc: Color,
    pub div_line: Color,
    pub session_id: Color,

    // semantic
    pub status_fg: Color,
    pub warning_fg: Color,

    // box borders
    pub cpu_box: Color,
    pub mem_box: Color,
    pub net_box: Color,
    pub proc_box: Color,

    // gradients
    pub cpu_grad: Gradient,
    pub proc_grad: Gradient,
    pub used_grad: Gradient,
    pub free_grad: Gradient,
    pub cached_grad: Gradient,
}
```

- [ ] **Step 2: Move existing constructors into `src/theme/mod.rs`**

Run: `mv src/theme.rs src/theme/mod.rs`

Then edit `src/theme/mod.rs`:

1. Replace the file's top section (lines 1–48 of the original — the `use`, `Gradient` struct, `Theme` struct, `Default impl`) with:

```rust
//! Theme module — types in `types`, parsing/loading in `loader`, bundled
//! palettes in `embedded`. Constructors below are temporary and will be
//! removed once embedded `.theme` files become the source of truth.

mod types;
pub use types::{Gradient, Theme};
```

2. Below that, **keep** the `pub const THEME_NAMES` and `impl Theme { ... }` blocks (the 12 `fn <name>() -> Self` constructors and `by_name`/`Default` are still needed for parity tests in Task 8).

3. In every constructor body, change:
   - `name: "btop"` → `name: "btop".to_string()`
   - (Apply to all 12 constructors.)

4. Re-add the `Default` impl at the bottom of `mod.rs`:

```rust
impl Default for Theme {
    fn default() -> Self {
        Self::btop()
    }
}
```

- [ ] **Step 3: Verify the build compiles**

Run: `cargo build --release 2>&1 | tail -5`
Expected: clean build (warnings ok).

- [ ] **Step 4: Verify existing tests still pass**

Run: `cargo test --lib --quiet 2>&1 | tail -10`
Expected: all existing tests pass.

The existing test in `theme/mod.rs` (around line 712 of the old file) asserts `t.name, "btop"` — this still works because `String == &str` is defined via `PartialEq`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(theme): split theme.rs into theme/ module

Types move to theme/types.rs with PartialEq + Clone + Debug derived.
Theme::name is now String for upcoming user-loaded themes. The 12
Rust constructors remain in theme/mod.rs as parity sources, to be
removed in a follow-up commit."
```

---

## Phase A3 — Parser

### Task 5: Add `parse_hex` to `src/theme/loader.rs`

Pure function. 6-digit and 3-digit forms, case-insensitive, `#`-prefix required.

**Files:**
- Create: `src/theme/loader.rs`
- Modify: `src/theme/mod.rs`

- [ ] **Step 1: Wire the module in `src/theme/mod.rs`**

Add at the top, below the existing `mod types;` line:

```rust
mod loader;
pub use loader::{apply_overrides, load_or_default, parse_theme_body};
```

(The `parse_hex` symbol stays module-private — only the higher-level functions are public.)

- [ ] **Step 2: Create `src/theme/loader.rs` with failing tests**

```rust
//! Theme file parser + loader.

use crate::config::AppConfig;
use crate::theme::types::{Gradient, Theme};
use ratatui::style::Color;
use std::path::PathBuf;

/// Parse a hex color string with optional `#` prefix.
/// Supports `#RRGGBB` (6-digit) and `#RGB` (3-digit, expanded to RRGGBB).
/// Case-insensitive. Returns None for anything else (no named colors, no
/// rgb() syntax).
fn parse_hex(raw: &str) -> Option<Color> {
    let s = raw.strip_prefix('#')?;
    match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16).ok()?;
            let g = u8::from_str_radix(&s[1..2], 16).ok()?;
            let b = u8::from_str_radix(&s[2..3], 16).ok()?;
            Some(Color::Rgb(r * 17, g * 17, b * 17)) // 0x9 * 17 = 0x99
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_6_digit_uppercase() {
        assert_eq!(parse_hex("#ABCDEF"), Some(Color::Rgb(0xAB, 0xCD, 0xEF)));
    }

    #[test]
    fn parse_hex_6_digit_lowercase() {
        assert_eq!(parse_hex("#abcdef"), Some(Color::Rgb(0xab, 0xcd, 0xef)));
    }

    #[test]
    fn parse_hex_3_digit_expands_via_x17() {
        // 0x9 -> 0x99, 0xa -> 0xaa, 0xf -> 0xff
        assert_eq!(parse_hex("#9af"), Some(Color::Rgb(0x99, 0xaa, 0xff)));
    }

    #[test]
    fn parse_hex_rejects_missing_hash() {
        assert_eq!(parse_hex("abcdef"), None);
    }

    #[test]
    fn parse_hex_rejects_wrong_length() {
        assert_eq!(parse_hex("#abcd"), None);
        assert_eq!(parse_hex("#abcdefab"), None);
    }

    #[test]
    fn parse_hex_rejects_non_hex_chars() {
        assert_eq!(parse_hex("#zzzzzz"), None);
    }
}
```

- [ ] **Step 3: Verify tests pass**

Run: `cargo test --lib --quiet theme::loader::tests::parse_hex 2>&1 | tail -10`
Expected: 6 tests pass.

(Note: unused-import warnings for `AppConfig`, `Gradient`, `Theme`, `PathBuf` are expected — they'll be used by later tasks.)

- [ ] **Step 4: Commit**

```bash
git add src/theme/loader.rs src/theme/mod.rs
git commit -m "feat(theme): add hex color parser

Supports btop-style #RRGGBB and #RGB forms, case-insensitive,
requires the # prefix. Returns None for anything else."
```

---

### Task 6: Add `parse_theme_body` to the loader

Reads `theme[key]="value"` lines into a `Theme`. Empty value on a `Color` field → `Color::Reset`. Missing keys / unknown values / unknown keys all fall back to the embedded btop default for that field. Gradient keys (`*_grad_start`/`mid`/`end`) cannot be reset and fall back to btop default on empty.

**Files:**
- Modify: `src/theme/loader.rs`

- [ ] **Step 1: Add failing tests**

Append to the `#[cfg(test)] mod tests` block in `src/theme/loader.rs`:

```rust
const MINIMAL_BTOP_OVERRIDE: &str = r#"
# A theme that only sets main_bg + main_fg.
# Everything else inherits embedded btop defaults.
theme[main_bg]="#112233"
theme[main_fg]="#445566"
"#;

#[test]
fn parse_theme_body_sets_known_keys() {
    let t = parse_theme_body(MINIMAL_BTOP_OVERRIDE, "test");
    assert_eq!(t.name, "test");
    assert_eq!(t.main_bg, Color::Rgb(0x11, 0x22, 0x33));
    assert_eq!(t.main_fg, Color::Rgb(0x44, 0x55, 0x66));
}

#[test]
fn parse_theme_body_inherits_unset_keys_from_btop() {
    // btop default `title` is Rgb(238, 238, 238).
    let t = parse_theme_body(MINIMAL_BTOP_OVERRIDE, "test");
    assert_eq!(t.title, Color::Rgb(238, 238, 238));
}

#[test]
fn parse_theme_body_empty_value_yields_reset_for_color_field() {
    let body = r#"theme[main_bg]="""#;
    let t = parse_theme_body(body, "transparent");
    assert_eq!(t.main_bg, Color::Reset);
}

#[test]
fn parse_theme_body_empty_value_on_gradient_inherits_btop() {
    // cpu_grad.start in btop is (119, 202, 155).
    let body = r#"theme[cpu_grad_start]="""#;
    let t = parse_theme_body(body, "test");
    assert_eq!(t.cpu_grad.start, (119, 202, 155));
}

#[test]
fn parse_theme_body_unknown_key_is_ignored() {
    let body = r#"theme[future_key]="#abcdef""#;
    let t = parse_theme_body(body, "test");
    // Sanity: nothing panicked, theme returned with name set.
    assert_eq!(t.name, "test");
}

#[test]
fn parse_theme_body_unknown_value_falls_back() {
    let body = r#"theme[main_bg]="not-a-color""#;
    let t = parse_theme_body(body, "test");
    // btop default main_bg is Rgb(25, 25, 25).
    assert_eq!(t.main_bg, Color::Rgb(25, 25, 25));
}

#[test]
fn parse_theme_body_handles_comments_and_blanks() {
    let body = r#"
        # leading comment
        theme[main_fg]="#abcdef"

        # trailing comment with a "quoted" segment
    "#;
    let t = parse_theme_body(body, "test");
    assert_eq!(t.main_fg, Color::Rgb(0xab, 0xcd, 0xef));
}

#[test]
fn parse_theme_body_reads_full_palette() {
    // A complete file should round-trip every Color field.
    let body = r#"
theme[main_bg]="#010203"
theme[main_fg]="#040506"
theme[title]="#070809"
theme[hi_fg]="#0a0b0c"
theme[selected_bg]="#0d0e0f"
theme[selected_fg]="#101112"
theme[inactive_fg]="#131415"
theme[graph_text]="#161718"
theme[meter_bg]="#191a1b"
theme[proc_misc]="#1c1d1e"
theme[div_line]="#1f2021"
theme[session_id]="#222324"
theme[status_fg]="#252627"
theme[warning_fg]="#28292a"
theme[cpu_box]="#2b2c2d"
theme[mem_box]="#2e2f30"
theme[net_box]="#313233"
theme[proc_box]="#343536"
theme[cpu_grad_start]="#373839"
theme[cpu_grad_mid]="#3a3b3c"
theme[cpu_grad_end]="#3d3e3f"
theme[proc_grad_start]="#404142"
theme[proc_grad_mid]="#434445"
theme[proc_grad_end]="#464748"
theme[used_grad_start]="#494a4b"
theme[used_grad_mid]="#4c4d4e"
theme[used_grad_end]="#4f5051"
theme[free_grad_start]="#525354"
theme[free_grad_mid]="#555657"
theme[free_grad_end]="#58595a"
theme[cached_grad_start]="#5b5c5d"
theme[cached_grad_mid]="#5e5f60"
theme[cached_grad_end]="#616263"
"#;
    let t = parse_theme_body(body, "full");
    assert_eq!(t.main_bg, Color::Rgb(1, 2, 3));
    assert_eq!(t.cached_grad.end, (0x61, 0x62, 0x63));
}
```

- [ ] **Step 2: Verify tests fail**

Run: `cargo test --lib --quiet theme::loader::tests::parse_theme_body 2>&1 | tail -10`
Expected: compilation error — `parse_theme_body` not defined.

- [ ] **Step 3: Implement the parser**

Add to `src/theme/loader.rs` (below `parse_hex`):

```rust
/// A single line of `theme[key]="value"` form, with leading/trailing
/// whitespace tolerated. Returns (key, value) or None.
fn parse_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let rest = line.strip_prefix("theme[")?;
    let (key, rest) = rest.split_once(']')?;
    let val_part = rest.trim_start().strip_prefix('=')?.trim_start();
    // value is quoted; allow either " or ' to match btop tolerance.
    let v = val_part
        .strip_prefix('"').and_then(|s| s.strip_suffix('"'))
        .or_else(|| val_part.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))?;
    Some((key, v))
}

/// Apply one (key, value) pair to a mutable Theme. Unknown keys and unknown
/// values are silently ignored (fields keep their existing values, which
/// come from the embedded btop default if seeded via `btop_default`).
fn apply_kv(theme: &mut Theme, key: &str, value: &str) {
    // Color-typed fields: empty value -> Color::Reset.
    let set_color = |t_field: &mut Color, v: &str| {
        if v.is_empty() {
            *t_field = Color::Reset;
        } else if let Some(c) = parse_hex(v) {
            *t_field = c;
        }
    };
    // Gradient channel: empty value silently ignored (no Reset semantics for tuples).
    let set_grad = |t_field: &mut (u8, u8, u8), v: &str| {
        if v.is_empty() {
            return;
        }
        if let Some(Color::Rgb(r, g, b)) = parse_hex(v) {
            *t_field = (r, g, b);
        }
    };
    match key {
        "main_bg" => set_color(&mut theme.main_bg, value),
        "main_fg" => set_color(&mut theme.main_fg, value),
        "title" => set_color(&mut theme.title, value),
        "hi_fg" => set_color(&mut theme.hi_fg, value),
        "selected_bg" => set_color(&mut theme.selected_bg, value),
        "selected_fg" => set_color(&mut theme.selected_fg, value),
        "inactive_fg" => set_color(&mut theme.inactive_fg, value),
        "graph_text" => set_color(&mut theme.graph_text, value),
        "meter_bg" => set_color(&mut theme.meter_bg, value),
        "proc_misc" => set_color(&mut theme.proc_misc, value),
        "div_line" => set_color(&mut theme.div_line, value),
        "session_id" => set_color(&mut theme.session_id, value),
        "status_fg" => set_color(&mut theme.status_fg, value),
        "warning_fg" => set_color(&mut theme.warning_fg, value),
        "cpu_box" => set_color(&mut theme.cpu_box, value),
        "mem_box" => set_color(&mut theme.mem_box, value),
        "net_box" => set_color(&mut theme.net_box, value),
        "proc_box" => set_color(&mut theme.proc_box, value),
        "cpu_grad_start" => set_grad(&mut theme.cpu_grad.start, value),
        "cpu_grad_mid" => set_grad(&mut theme.cpu_grad.mid, value),
        "cpu_grad_end" => set_grad(&mut theme.cpu_grad.end, value),
        "proc_grad_start" => set_grad(&mut theme.proc_grad.start, value),
        "proc_grad_mid" => set_grad(&mut theme.proc_grad.mid, value),
        "proc_grad_end" => set_grad(&mut theme.proc_grad.end, value),
        "used_grad_start" => set_grad(&mut theme.used_grad.start, value),
        "used_grad_mid" => set_grad(&mut theme.used_grad.mid, value),
        "used_grad_end" => set_grad(&mut theme.used_grad.end, value),
        "free_grad_start" => set_grad(&mut theme.free_grad.start, value),
        "free_grad_mid" => set_grad(&mut theme.free_grad.mid, value),
        "free_grad_end" => set_grad(&mut theme.free_grad.end, value),
        "cached_grad_start" => set_grad(&mut theme.cached_grad.start, value),
        "cached_grad_mid" => set_grad(&mut theme.cached_grad.mid, value),
        "cached_grad_end" => set_grad(&mut theme.cached_grad.end, value),
        _ => {}
    }
}

/// Parse a btop-style theme body. Returns a fully-populated Theme: missing
/// keys are backfilled from the embedded btop default. The returned theme's
/// `name` is the caller-supplied `name`.
pub fn parse_theme_body(body: &str, name: &str) -> Theme {
    let mut theme = Theme::btop();
    theme.name = name.to_string();
    for line in body.lines() {
        if let Some((k, v)) = parse_line(line) {
            apply_kv(&mut theme, k, v);
        }
    }
    theme
}
```

- [ ] **Step 4: Verify tests pass**

Run: `cargo test --lib --quiet theme::loader::tests::parse_theme_body 2>&1 | tail -20`
Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add parse_theme_body for btop-style theme files

Reads theme[key]=\"value\" lines into a Theme struct. Empty value on a
Color field yields Color::Reset; on a gradient channel it falls back
to the embedded btop default. Unknown keys and malformed values are
silently ignored."
```

---

### Task 7: Add `apply_overrides`

Post-processing step that enforces `theme_background = false` by stamping `Color::Reset` over the active theme's `main_bg`.

**Files:**
- Modify: `src/theme/loader.rs`

- [ ] **Step 1: Add failing tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
fn cfg_with_bg(bg: bool) -> AppConfig {
    let mut c = AppConfig::default();
    c.theme_background = bg;
    c
}

#[test]
fn apply_overrides_force_transparent_with_opaque_theme() {
    let mut t = Theme::btop();
    let original_bg = t.main_bg;
    apply_overrides(&mut t, &cfg_with_bg(false));
    assert_eq!(t.main_bg, Color::Reset);
    assert_ne!(t.main_bg, original_bg); // sanity check: btop is opaque
}

#[test]
fn apply_overrides_keep_theme_when_flag_default_true() {
    let mut t = Theme::btop();
    let original_bg = t.main_bg;
    apply_overrides(&mut t, &cfg_with_bg(true));
    assert_eq!(t.main_bg, original_bg);
}

#[test]
fn apply_overrides_leaves_other_bg_fields_alone() {
    let mut t = Theme::btop();
    let original_selected = t.selected_bg;
    let original_meter = t.meter_bg;
    apply_overrides(&mut t, &cfg_with_bg(false));
    assert_eq!(t.selected_bg, original_selected);
    assert_eq!(t.meter_bg, original_meter);
}

#[test]
fn apply_overrides_already_reset_main_bg_stays_reset() {
    let mut t = Theme::btop();
    t.main_bg = Color::Reset;
    apply_overrides(&mut t, &cfg_with_bg(true));
    assert_eq!(t.main_bg, Color::Reset);
}
```

- [ ] **Step 2: Verify tests fail**

Run: `cargo test --lib --quiet theme::loader::tests::apply_overrides 2>&1 | tail -10`
Expected: compilation error — `apply_overrides` not found.

- [ ] **Step 3: Implement the function**

Add to `src/theme/loader.rs` (below `parse_theme_body`):

```rust
/// Apply config-level overrides on top of a parsed Theme.
///
/// Currently the only override is `theme_background = false`, which stamps
/// `Color::Reset` over `main_bg`. Other background fields (selected_bg,
/// meter_bg) are left alone — they're indicators, not the window background.
pub fn apply_overrides(theme: &mut Theme, cfg: &AppConfig) {
    if !cfg.theme_background {
        theme.main_bg = Color::Reset;
    }
}
```

- [ ] **Step 4: Verify tests pass**

Run: `cargo test --lib --quiet theme::loader::tests::apply_overrides 2>&1 | tail -10`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add apply_overrides for theme_background flag

Post-processing step that forces main_bg to Color::Reset when the
config flag is false. Other indicator backgrounds (selected, meter)
are intentionally untouched."
```

---

## Phase A4 — Embedded themes

### Task 8: Embed `btop` theme as the parity sentinel

Convert `Theme::btop()` to a `.theme` file at the repo root, then prove the parser produces an identical Theme. This catches transcription errors and locks in the parity contract for the other 11 themes.

**Files:**
- Create: `themes/btop.theme`
- Create: `src/theme/embedded.rs`
- Modify: `src/theme/mod.rs`
- Modify: `src/theme/loader.rs` (parity test)

- [ ] **Step 1: Create `themes/btop.theme`**

Write to `themes/btop.theme`:

```sh
# btop — default palette. Exact RGB values from btop_theme.cpp Default_theme.

theme[main_bg]="#191919"
theme[main_fg]="#cccccc"
theme[title]="#eeeeee"
theme[hi_fg]="#b54040"
theme[selected_bg]="#6a2f2f"
theme[selected_fg]="#eeeeee"
theme[inactive_fg]="#404040"
theme[graph_text]="#606060"
theme[meter_bg]="#404040"
theme[proc_misc]="#0de756"
theme[div_line]="#303030"
theme[session_id]="#b0a070"

theme[status_fg]="#dc4c4c"
theme[warning_fg]="#dca032"

theme[cpu_box]="#556d59"
theme[mem_box]="#6c6c4b"
theme[net_box]="#5c588d"
theme[proc_box]="#805252"

theme[cpu_grad_start]="#77ca9b"
theme[cpu_grad_mid]="#cbc06c"
theme[cpu_grad_end]="#dc4c4c"

theme[proc_grad_start]="#80d0a3"
theme[proc_grad_mid]="#dcd179"
theme[proc_grad_end]="#d45454"

theme[used_grad_start]="#592b26"
theme[used_grad_mid]="#d9626d"
theme[used_grad_end]="#ff4769"

theme[free_grad_start]="#384f21"
theme[free_grad_mid]="#b5e685"
theme[free_grad_end]="#dcff85"

theme[cached_grad_start]="#163350"
theme[cached_grad_mid]="#74e6fc"
theme[cached_grad_end]="#26c5ff"
```

The hex values are derived from the existing `Theme::btop()` constructor: each `Color::Rgb(r, g, b)` becomes `"#{r:02x}{g:02x}{b:02x}"`, each `Gradient { start: (r,g,b), ... }` becomes three lines. **The parity test in step 4 confirms exact equality, so if any hex is wrong the build fails loud.**

- [ ] **Step 2: Create `src/theme/embedded.rs`**

Write:

```rust
//! Compile-time table of bundled themes. Each entry is (name, raw .theme body).

pub const BUILTIN: &[(&str, &str)] = &[
    ("btop", include_str!("../../themes/btop.theme")),
];

/// Look up an embedded theme's raw body by name.
pub fn lookup(name: &str) -> Option<&'static str> {
    BUILTIN.iter().find(|(n, _)| *n == name).map(|(_, body)| *body)
}
```

- [ ] **Step 3: Wire the embedded module**

In `src/theme/mod.rs`, add at the top alongside the existing module decls:

```rust
mod embedded;
```

The existing `pub const THEME_NAMES: &[&str] = &[ ... 12 entries ... ]` stays unchanged in this task — it remains the source of truth for the `t`-cycle while only `btop` is embedded. Task 12 swaps it for a `theme_names()` function backed by `embedded::BUILTIN` once all 12 themes are embedded.

Add a smoke-style test to `src/theme/loader.rs` to guard against `THEME_NAMES` drifting from `BUILTIN` as themes get added in Task 9:

```rust
#[test]
fn theme_names_const_lists_only_embedded_names() {
    let embedded_set: std::collections::HashSet<&str> =
        crate::theme::embedded::BUILTIN.iter().map(|(n, _)| *n).collect();
    for name in crate::theme::THEME_NAMES {
        assert!(
            embedded_set.contains(name) || true,
            // Task 8 only embeds btop; this assertion relaxes until Task 9
            // fills the rest. The hard check fires in Task 9.
        );
    }
}
```

(This skeleton becomes a hard check in Task 9. Phrased this way so Task 8's commit still builds with only btop embedded.)

- [ ] **Step 4: Add the parity test**

In `src/theme/loader.rs`, append to the test module:

```rust
#[test]
fn embedded_btop_matches_rust_constructor() {
    let body = crate::theme::embedded::lookup("btop")
        .expect("btop must be in BUILTIN");
    let parsed = parse_theme_body(body, "btop");
    let from_rust = Theme::btop();
    assert_eq!(parsed, from_rust, "embedded btop.theme drifted from Theme::btop()");
}
```

- [ ] **Step 5: Verify the parity test passes**

Run: `cargo test --lib --quiet theme::loader::tests::embedded_btop_matches 2>&1 | tail -10`
Expected: 1 test passes. If it fails, fix `themes/btop.theme` until the test passes — that's the whole point of the sentinel.

- [ ] **Step 6: Verify the full test suite is still green**

Run: `cargo test --lib --quiet 2>&1 | tail -5`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add themes/btop.theme src/theme/embedded.rs src/theme/mod.rs src/theme/loader.rs
git commit -m "feat(theme): embed btop as the first .theme file

Adds themes/btop.theme + BUILTIN table + parity test asserting that
parse_theme_body(embedded_btop) == Theme::btop(). This sentinel
catches transcription errors when the other 11 themes are added."
```

---

### Task 9: Embed the remaining 11 themes

Translate each `Theme::<name>()` constructor into `themes/<name>.theme`. The parity test loop (added below) is the safety net.

**Files:**
- Create: `themes/dracula.theme`, `themes/catppuccin.theme`, `themes/tokyo-night.theme`, `themes/gruvbox.theme`, `themes/nord.theme`, `themes/light.theme`, `themes/white.theme`, `themes/high-contrast.theme`, `themes/protanopia.theme`, `themes/deuteranopia.theme`, `themes/tritanopia.theme`
- Modify: `src/theme/embedded.rs`
- Modify: `src/theme/loader.rs` (extend parity test)

- [ ] **Step 1: Transcribe one theme per file**

For each of the 11 remaining themes:

1. Open `src/theme/mod.rs`, locate `pub fn <name>() -> Self { ... }`.
2. Create `themes/<file-name>.theme` (the file name matches the `name:` field, so e.g. `tokyo-night.theme`, `high-contrast.theme`).
3. Transcribe each field: every `Color::Rgb(r, g, b)` becomes a `theme[<key>]="#{r:02x}{g:02x}{b:02x}"` line; every `Gradient { start: (r,g,b), mid: (r,g,b), end: (r,g,b) }` becomes three lines (`<grad>_grad_start` / `_mid` / `_end`).
4. Group with blank lines matching the format used in `themes/btop.theme`.

Example header for `themes/catppuccin.theme`:

```sh
# Catppuccin Mocha palette.

theme[main_bg]="#1e1e2e"
theme[main_fg]="#cdd6f4"
# ... (39 keys total, exactly the structure in btop.theme)
```

The parity test in Step 3 enforces correctness, so transcription mistakes fail loudly.

- [ ] **Step 2: Expand `BUILTIN` in `src/theme/embedded.rs`**

Replace the single-entry `BUILTIN` with all 12:

```rust
pub const BUILTIN: &[(&str, &str)] = &[
    ("btop",          include_str!("../../themes/btop.theme")),
    ("dracula",       include_str!("../../themes/dracula.theme")),
    ("catppuccin",    include_str!("../../themes/catppuccin.theme")),
    ("tokyo-night",   include_str!("../../themes/tokyo-night.theme")),
    ("gruvbox",       include_str!("../../themes/gruvbox.theme")),
    ("nord",          include_str!("../../themes/nord.theme")),
    ("light",         include_str!("../../themes/light.theme")),
    ("white",         include_str!("../../themes/white.theme")),
    ("high-contrast", include_str!("../../themes/high-contrast.theme")),
    ("protanopia",    include_str!("../../themes/protanopia.theme")),
    ("deuteranopia",  include_str!("../../themes/deuteranopia.theme")),
    ("tritanopia",    include_str!("../../themes/tritanopia.theme")),
];
```

- [ ] **Step 3: Add the parity-test loop**

In `src/theme/loader.rs`, append to the test module:

```rust
#[test]
fn every_embedded_theme_matches_its_rust_constructor() {
    let pairs: &[(&str, fn() -> Theme)] = &[
        ("btop",          Theme::btop),
        ("dracula",       Theme::dracula),
        ("catppuccin",    Theme::catppuccin),
        ("tokyo-night",   Theme::tokyo_night),
        ("gruvbox",       Theme::gruvbox),
        ("nord",          Theme::nord),
        ("light",         Theme::light),
        ("white",         Theme::white),
        ("high-contrast", Theme::high_contrast),
        ("protanopia",    Theme::protanopia),
        ("deuteranopia",  Theme::deuteranopia),
        ("tritanopia",    Theme::tritanopia),
    ];
    for (name, ctor) in pairs {
        let body = crate::theme::embedded::lookup(name)
            .unwrap_or_else(|| panic!("'{name}' missing from BUILTIN"));
        let parsed = parse_theme_body(body, name);
        let from_rust = ctor();
        assert_eq!(parsed, from_rust, "theme '{name}' drifted from Rust constructor");
    }
}
```

- [ ] **Step 4: Tighten the `THEME_NAMES`-vs-`BUILTIN` consistency test**

In `src/theme/loader.rs`, replace the relaxed assertion from Task 8 step 3 with a hard equality check (since all 12 entries should now be in both places):

```rust
#[test]
fn theme_names_const_matches_embedded_in_order() {
    let embedded: Vec<&str> = crate::theme::embedded::BUILTIN.iter().map(|(n, _)| *n).collect();
    let listed: Vec<&str> = crate::theme::THEME_NAMES.to_vec();
    assert_eq!(embedded, listed, "THEME_NAMES drifted from embedded::BUILTIN");
}
```

- [ ] **Step 5: Verify the parity loop passes**

Run: `cargo test --lib --quiet theme::loader::tests::every_embedded_theme 2>&1 | tail -20`
Expected: 1 test passes. If it fails, the assertion will name which theme drifted and show a diff between the parsed Theme and the Rust constructor's output — fix the offending `.theme` file accordingly.

- [ ] **Step 6: Verify the full suite is still green**

Run: `cargo test --lib --quiet 2>&1 | tail -5`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add themes/ src/theme/embedded.rs src/theme/loader.rs
git commit -m "feat(theme): embed the remaining 11 themes

Each themes/<name>.theme transcribes the matching Rust constructor.
Parity test loop asserts every embedded body parses to a Theme equal
to its constructor, guarding against transcription drift. Consistency
test pins THEME_NAMES const to BUILTIN order."
```

---

## Phase A5 — Loader chain & wiring

### Task 10: Add `Theme::load_or_default` and update `by_name`

Three resolution layers (user file → embedded → embedded btop), plus a `by_name(name)` variant for validation that consults user dir + embedded but does not fall back to btop.

**Files:**
- Modify: `src/theme/loader.rs`
- Modify: `src/theme/mod.rs`

- [ ] **Step 1: Add failing tests**

Append to `#[cfg(test)] mod tests` in `src/theme/loader.rs`:

```rust
use tempfile::TempDir;

fn write_theme_file(dir: &std::path::Path, name: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{name}.theme")), body).unwrap();
}

#[test]
fn load_chain_user_file_wins_over_embedded() {
    let tmp = TempDir::new().unwrap();
    let themes_dir = tmp.path().join("abtop").join("themes");
    write_theme_file(&themes_dir, "btop", r#"theme[main_fg]="#ff00ff""#);

    let t = load_chain(tmp.path(), "btop");
    // User file overrode main_fg; everything else still inherits embedded btop.
    assert_eq!(t.main_fg, Color::Rgb(0xff, 0x00, 0xff));
    // Sanity: main_bg comes from embedded btop, NOT touched by the user file.
    assert_eq!(t.main_bg, Color::Rgb(0x19, 0x19, 0x19));
}

#[test]
fn load_chain_falls_back_to_embedded_when_no_user_file() {
    let tmp = TempDir::new().unwrap();
    let t = load_chain(tmp.path(), "catppuccin");
    assert_eq!(t.name, "catppuccin");
    // Catppuccin main_fg = #cdd6f4.
    assert_eq!(t.main_fg, Color::Rgb(0xcd, 0xd6, 0xf4));
}

#[test]
fn load_chain_unknown_name_falls_back_to_embedded_btop() {
    let tmp = TempDir::new().unwrap();
    let t = load_chain(tmp.path(), "nonexistent-theme-12345");
    // Embedded btop's main_bg = #191919.
    assert_eq!(t.main_bg, Color::Rgb(0x19, 0x19, 0x19));
    // But the name field reflects the requested name (so cycling logic stays sane).
    // Actually: documenting expected behavior — we want the fallback to use
    // embedded btop verbatim, so name == "btop" in that case.
    assert_eq!(t.name, "btop");
}

#[test]
fn lookup_chain_returns_some_for_embedded_name() {
    let tmp = TempDir::new().unwrap();
    let t = lookup_chain(tmp.path(), "dracula").unwrap();
    assert_eq!(t.name, "dracula");
}

#[test]
fn lookup_chain_returns_some_for_user_dir_name() {
    let tmp = TempDir::new().unwrap();
    let themes_dir = tmp.path().join("abtop").join("themes");
    write_theme_file(
        &themes_dir,
        "my-custom",
        r#"theme[main_fg]="#abcdef""#,
    );
    let t = lookup_chain(tmp.path(), "my-custom").unwrap();
    assert_eq!(t.name, "my-custom");
    assert_eq!(t.main_fg, Color::Rgb(0xab, 0xcd, 0xef));
}

#[test]
fn lookup_chain_returns_none_for_unknown_name() {
    let tmp = TempDir::new().unwrap();
    assert!(lookup_chain(tmp.path(), "no-such-thing").is_none());
}
```

- [ ] **Step 2: Verify tests fail**

Run: `cargo test --lib --quiet theme::loader::tests::load_chain 2>&1 | tail -10`
Expected: compilation error — `load_chain` / `lookup_chain` not found.

- [ ] **Step 3: Implement the chain**

Add to `src/theme/loader.rs`:

```rust
use std::path::Path;

/// Try to read and parse `<config_root>/abtop/themes/<name>.theme`.
/// Returns Some(theme) on a successful read+parse; None if the file is
/// missing or unreadable.
fn try_user_file(config_root: &Path, name: &str) -> Option<Theme> {
    let path = config_root.join("abtop").join("themes").join(format!("{name}.theme"));
    let body = std::fs::read_to_string(&path).ok()?;
    Some(parse_theme_body(&body, name))
}

/// Resolve a theme by name, consulting (1) the user themes dir under
/// `config_root`, then (2) the embedded BUILTIN table. Returns None if
/// neither contains the name.
pub fn lookup_chain(config_root: &Path, name: &str) -> Option<Theme> {
    if let Some(t) = try_user_file(config_root, name) {
        return Some(t);
    }
    crate::theme::embedded::lookup(name).map(|body| parse_theme_body(body, name))
}

/// Resolve a theme with a last-resort fallback to embedded `btop`. Always
/// returns a Theme. The returned theme has NOT had `apply_overrides` applied
/// — callers must run that afterward if the config flag should affect it.
pub fn load_chain(config_root: &Path, name: &str) -> Theme {
    lookup_chain(config_root, name).unwrap_or_else(|| {
        // Embedded btop must always be parseable; that's enforced by the
        // parity test loop. If it's not, that's a programming error worth
        // crashing on at startup.
        let body = crate::theme::embedded::lookup("btop")
            .expect("embedded btop is a build-time invariant");
        parse_theme_body(body, "btop")
    })
}

/// Public entry point used by startup. Resolves the name against the
/// current XDG config root and applies config-level overrides.
pub fn load_or_default(name: &str, cfg: &AppConfig) -> Theme {
    let mut theme = load_chain(&crate::config::xdg_config_dir(), name);
    apply_overrides(&mut theme, cfg);
    theme
}
```

- [ ] **Step 4: Expose `by_name` via the chain**

In `src/theme/mod.rs`, update `Theme::by_name` to use `lookup_chain` instead of the hardcoded match. Find the existing implementation:

```rust
impl Theme {
    pub fn by_name(name: &str) -> Option<Self> {
        match name {
            "btop" => Some(Self::btop()),
            // ... (12 arms)
        }
    }
    // ...
}
```

Replace just the `by_name` method body:

```rust
impl Theme {
    pub fn by_name(name: &str) -> Option<Self> {
        loader::lookup_chain(&crate::config::xdg_config_dir(), name)
    }
    // ... (keep the 12 ctor methods + Default; they're deleted in Task 12)
}
```

- [ ] **Step 5: Verify tests pass**

Run: `cargo test --lib --quiet theme::loader::tests 2>&1 | tail -10`
Expected: all theme::loader tests pass (including the 6 new ones).

- [ ] **Step 6: Verify the full suite is still green**

Run: `cargo test --lib --quiet 2>&1 | tail -5`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/theme/loader.rs src/theme/mod.rs
git commit -m "feat(theme): add load_or_default chain with user-dir lookup

Resolution: user file in \$XDG_CONFIG_HOME/abtop/themes/<name>.theme,
then embedded BUILTIN, then embedded btop. Theme::by_name now consults
the same chain (without the last-resort fallback) so --theme <name>
accepts user-defined themes."
```

---

### Task 11: Wire startup and `cycle_theme` to the new chain

Replace the existing `Theme::by_name(...).unwrap_or_else(...)` startup logic with `Theme::load_or_default`. Preserve the existing "named theme not found" stderr message.

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/app.rs`

- [ ] **Step 1: Update `src/lib.rs` startup path**

The existing block (lines 109–140 of `src/lib.rs`) reads:

```rust
    let cfg = config::load_config();

    // --theme flag > config file > default
    let initial_theme = std::env::args()
        .position(|a| a == "--theme")
        .map(|pos| {
            let val = std::env::args().nth(pos + 1);
            match val {
                Some(name) if !name.starts_with('-') => name,
                Some(name) => {
                    eprintln!("--theme requires a theme name, got '{}'", name);
                    eprintln!("available: {}", theme::THEME_NAMES.join(", "));
                    std::process::exit(1);
                }
                None => {
                    eprintln!("--theme requires a theme name");
                    eprintln!("available: {}", theme::THEME_NAMES.join(", "));
                    std::process::exit(1);
                }
            }
        })
        .map(|name| {
            theme::Theme::by_name(&name).unwrap_or_else(|| {
                eprintln!(
                    "unknown theme '{}'. available: {}",
                    name,
                    theme::THEME_NAMES.join(", ")
                );
                std::process::exit(1);
            })
        })
        .or_else(|| theme::Theme::by_name(&cfg.theme));
```

`initial_theme` is `Option<Theme>` and is later used as `initial_theme.unwrap_or_default()` (line 150 + others). After this change, it becomes `Theme` (always populated).

Replace the entire block above with:

```rust
    let cfg = config::load_config();

    // --theme flag > config file > default
    let cli_theme_name: Option<String> = std::env::args()
        .position(|a| a == "--theme")
        .map(|pos| {
            let val = std::env::args().nth(pos + 1);
            match val {
                Some(name) if !name.starts_with('-') => name,
                Some(name) => {
                    eprintln!("--theme requires a theme name, got '{}'", name);
                    eprintln!("available: {}", theme::THEME_NAMES.join(", "));
                    std::process::exit(1);
                }
                None => {
                    eprintln!("--theme requires a theme name");
                    eprintln!("available: {}", theme::THEME_NAMES.join(", "));
                    std::process::exit(1);
                }
            }
        });

    // Validate CLI --theme exists in user dir or embedded; hard-fail if not.
    if let Some(name) = &cli_theme_name {
        if theme::Theme::by_name(name).is_none() {
            eprintln!(
                "unknown theme '{}'. available: {}",
                name,
                theme::THEME_NAMES.join(", ")
            );
            std::process::exit(1);
        }
    }

    let resolved_name = cli_theme_name.unwrap_or_else(|| cfg.theme.clone());
    let initial_theme: theme::Theme = theme::loader::load_or_default(&resolved_name, &cfg);
```

- [ ] **Step 2: Update `unwrap_or_default()` call sites**

Because `initial_theme` is now `Theme` not `Option<Theme>`, every `initial_theme.unwrap_or_default()` in `src/lib.rs` becomes just `initial_theme.clone()` (Theme is `Clone`).

Find them:

Run: `rg -n 'initial_theme' src/lib.rs`

For each match (likely 2–3), replace `initial_theme.unwrap_or_default()` with `initial_theme.clone()`. If a callsite passes by value and there's only one use after the rebinding, you can omit `.clone()` on the last use.

- [ ] **Step 3: Update `src/app.rs::cycle_theme`**

Locate `cycle_theme` (line 473 of `src/app.rs`):

```rust
    pub fn cycle_theme(&mut self) {
        let names = crate::theme::THEME_NAMES;
        let current = names
            .iter()
            .position(|&n| n == self.theme.name)
            .unwrap_or(0);
        let next = (current + 1) % names.len();
        self.theme = Theme::by_name(names[next]).unwrap_or_default();
        // ... (existing save_theme call + status message follows)
```

`App` does not currently hold an `AppConfig`. Verify with: `rg -n 'AppConfig|pub struct App' src/app.rs | head`.

If `App` lacks an `AppConfig` field, the simplest path is to read `theme_background` from disk on every cycle:

```rust
    pub fn cycle_theme(&mut self) {
        let names = crate::theme::THEME_NAMES;
        let current = names
            .iter()
            .position(|&n| n == self.theme.name)
            .unwrap_or(0);
        let next = (current + 1) % names.len();
        let cfg = crate::config::load_config();
        let mut new_theme = Theme::by_name(names[next]).unwrap_or_default();
        crate::theme::loader::apply_overrides(&mut new_theme, &cfg);
        self.theme = new_theme;
        // ... (existing save_theme call + status message stays unchanged)
```

(Reading config on each `t`-press is cheap — a small file read — and avoids threading state through `App` in Phase A.)

- [ ] **Step 4: Verify the build compiles**

Run: `cargo build --release 2>&1 | tail -5`
Expected: clean build.

- [ ] **Step 5: Verify all tests still pass**

Run: `cargo test --lib --quiet 2>&1 | tail -5`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/app.rs
git commit -m "feat(theme): route startup + cycling through load_or_default

Startup applies theme_background overrides via the loader chain.
cycle_theme also re-applies overrides so toggling theme via 't'
preserves transparency state."
```

---

## Phase A6 — Cleanup

### Task 12: Delete the 12 Rust constructors

Now that the embedded `.theme` files are the source of truth (proven by the parity loop), the Rust constructors are dead weight.

**Files:**
- Modify: `src/theme/mod.rs`
- Modify: `src/theme/loader.rs` (drop the parity-loop test that references constructors)

- [ ] **Step 1: Verify parity loop passes one last time**

Run: `cargo test --lib --quiet theme::loader::tests::every_embedded_theme 2>&1 | tail -5`
Expected: 1 test passes. **DO NOT proceed if this fails — the embedded files are wrong.**

- [ ] **Step 2: Delete the 12 constructor functions in `src/theme/mod.rs`**

Remove these (current line ranges, approximate):
- `pub fn btop()` (≈ lines 84–132)
- `pub fn dracula()` (≈ lines 134–181)
- `pub fn catppuccin()` (≈ lines 183–232)
- `pub fn tokyo_night()` (≈ lines 234–281)
- `pub fn gruvbox()` (≈ 12 more)
- `pub fn nord()`
- `pub fn light()`
- `pub fn white()`
- `pub fn high_contrast()`
- `pub fn protanopia()`
- `pub fn deuteranopia()`
- `pub fn tritanopia()`

Keep the `impl Theme { pub fn by_name(...) ... }` block but with only the `by_name` method.

Replace the `impl Default for Theme` with:

```rust
impl Default for Theme {
    fn default() -> Self {
        // Embedded btop is a build-time invariant. If it ever fails to
        // parse, the loader tests catch that during `cargo test`.
        Theme::by_name("btop").expect("embedded btop must resolve")
    }
}
```

**Keep `pub const THEME_NAMES`** — it's already pinned to `BUILTIN` order by the consistency test added in Task 9. Callers in `src/app.rs` and `src/lib.rs` continue to use it unchanged.

- [ ] **Step 3: Delete the parity-loop test**

Now that the constructors are gone, the `every_embedded_theme_matches_its_rust_constructor` test won't compile. Delete it from `src/theme/loader.rs`. Also delete `embedded_btop_matches_rust_constructor` (Task 8's sentinel). The embedded `.theme` files are now the ground truth — drift can't happen because there's nothing to drift from.

Replace those two deleted tests with a "every embedded theme parses cleanly" smoke test:

```rust
#[test]
fn every_embedded_theme_parses_to_full_palette() {
    for (name, body) in crate::theme::embedded::BUILTIN.iter() {
        let t = parse_theme_body(body, name);
        assert_eq!(t.name, *name);
        // Sanity: a fully-populated palette doesn't leave any gradient channel
        // at the default (0,0,0) zero state unless the theme really uses black there.
        // Cheap check: at least one field differs from the embedded btop default.
        if *name != "btop" {
            let btop_body = crate::theme::embedded::lookup("btop").unwrap();
            let btop = parse_theme_body(btop_body, "btop");
            assert_ne!(t, Theme { name: name.to_string(), ..btop.clone() },
                "embedded '{name}' parsed identically to btop — likely empty file");
        }
    }
}
```

- [ ] **Step 4: Verify the build compiles**

Run: `cargo build --release 2>&1 | tail -5`
Expected: clean build. If `THEME_NAMES` references remain, the compiler will name the file/line.

- [ ] **Step 5: Verify all tests pass**

Run: `cargo test --lib --quiet 2>&1 | tail -10`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/theme/mod.rs src/theme/loader.rs
git commit -m "refactor(theme): delete Rust theme constructors

Embedded themes/*.theme files are now the sole source of truth.
The 12 Theme::<name>() constructors are removed, along with the
parity tests that proved equivalence between them and the .theme
files (no longer meaningful — nothing to drift from)."
```

---

## Phase A7 — Documentation & smoke test

### Task 13: Update README

Document the new themes directory, the `theme_background` flag, and the btop-compatible file format.

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a theming section**

Open `README.md`, find the "Usage" section. After the existing usage examples, add a new section:

```markdown
## Theming

abtop ships with 12 bundled themes; pick one via `--theme <name>` or by
setting `theme = "<name>"` in `$XDG_CONFIG_HOME/abtop/config.toml`
(default `~/.config/abtop/config.toml`).

Available embedded themes: `btop`, `dracula`, `catppuccin`, `tokyo-night`,
`gruvbox`, `nord`, `light`, `white`, `high-contrast`, `protanopia`,
`deuteranopia`, `tritanopia`.

### Custom themes

Drop a `*.theme` file into `$XDG_CONFIG_HOME/abtop/themes/` and reference
it by file basename. The format is btop-compatible:

\`\`\`sh
# ~/.config/abtop/themes/my-theme.theme
theme[main_bg]="#1e1e2e"      # 6-digit hex, or empty for terminal default
theme[main_fg]="#cdd6f4"
theme[selected_bg]="#313244"
# ... (see themes/btop.theme in the source tree for the full key list)
\`\`\`

Missing keys inherit from the embedded `btop` theme. Empty values on any
`Color` field render as `Color::Reset` (the terminal's own default), which
on terminals with transparency configured lets the background show through.

### Transparent background

Add `theme_background = false` to your `config.toml` to force `main_bg` to
the terminal default for any theme — no need to edit the theme file itself:

\`\`\`toml
theme = "catppuccin"
theme_background = false
\`\`\`
```

(Adjust the surrounding context to match the existing README structure.)

- [ ] **Step 2: Verify the doc renders cleanly**

If you have a Markdown previewer, eyeball it. Otherwise just confirm no syntax noise:

Run: `head -120 README.md`
Expected: the new section reads cleanly.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document themes/ dir and theme_background flag"
```

---

### Task 14: Build release and install to `~/.local/libexec/abtop`

The user's chosen install path (already on PATH via the earlier zsh change).

**Files:** none (build + copy)

- [ ] **Step 1: Run the full test suite one more time**

Run: `cargo test --release --quiet 2>&1 | tail -10`
Expected: all tests pass.

- [ ] **Step 2: Build release**

Run: `cargo build --release 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 3: Install the binary**

Run: `install -m 755 target/release/abtop ~/.local/libexec/abtop`

- [ ] **Step 4: Verify the binary is on PATH**

Run: `which abtop`
Expected: `/Users/a.salvi/.local/libexec/abtop`.

- [ ] **Step 5: Smoke test — transparent catppuccin**

Run:
```bash
mkdir -p ~/.config/abtop
printf 'theme = "catppuccin"\ntheme_background = false\n' > ~/.config/abtop/config.toml
abtop --once | head -20
```

Expected: a snapshot prints; no panic.

Then launch the TUI:

```bash
abtop
```

Expected: the catppuccin palette renders, but the terminal's own background (any transparency you have configured in your terminal emulator) shows through where abtop would otherwise paint Catppuccin's `#1e1e2e`. The selected-row and meter backgrounds are still colored — those use `selected_bg`/`meter_bg`, which `apply_overrides` deliberately doesn't touch.

Press `q` to quit.

- [ ] **Step 6: Smoke test — user-dir theme**

Test that a user-supplied theme is picked up:

```bash
mkdir -p ~/.config/abtop/themes
cat > ~/.config/abtop/themes/loud.theme <<'EOF'
theme[main_bg]="#ff00ff"
theme[main_fg]="#ffff00"
EOF

abtop --theme loud --once | head -5
```

Expected: snapshot output prints. (The visual confirmation comes from running `abtop --theme loud` interactively — the `--once` mode just exercises the resolution path.)

Clean up the test theme:

```bash
rm ~/.config/abtop/themes/loud.theme
```

- [ ] **Step 7: Done. No commit needed — the binary install is a side effect, not in-repo.**

---

## Acceptance criteria

All of the following must hold for Phase A to be complete:

1. `cargo test --release` is green.
2. `cargo build --release` produces a binary that runs without panicking on `--once`, `--json`, and the TUI mode.
3. `abtop --theme <embedded-name>` resolves all 12 embedded themes.
4. `abtop --theme <user-file-name>` resolves a theme dropped at `$XDG_CONFIG_HOME/abtop/themes/<name>.theme`.
5. With `theme_background = false` in `config.toml`, the rendered output uses `Color::Reset` for `main_bg` — visible as terminal transparency where configured.
6. With `theme_background = true` (or absent) in `config.toml`, the theme's own `main_bg` is used.
7. `selected_bg` and `meter_bg` retain their theme values regardless of `theme_background`.
8. `~/.local/libexec/abtop` is the installed binary.

## Phase B (deferred — separate plan)

- `t`-cycle merges user-dir themes with embedded set.
- Banner in UI on malformed theme file.
- `abtop --theme <absolute-path>`.
- `abtop --list-themes`, `abtop --dump-theme <name>`.
- Reload on file change.
- macOS `~/Library/Application Support/abtop/config.toml` → XDG migration.
