# Theme files & background transparency — design

**Status:** Approved (local fork)
**Date:** 2026-06-09
**Scope:** Phase A — single self-contained change. Phase B items listed but deferred.
**Target:** `~/my-workspace/util/abtop` (local fork of `graykode/abtop` at v0.4.8). No upstream PR.

## Goal

Move abtop's theming from compiled-in Rust constants to external `.theme` files modeled on btop's format, and add a `theme_background` config flag so users can render abtop with a transparent background that lets the terminal show through. Achieve this without rebuilding abtop when a user wants to change or author a theme.

## Non-goals (Phase A)

- `t`-key cycling does not pick up user-dir themes (still cycles the embedded set only).
- No banner UI when a theme file is malformed (silent fallback to embedded default — matches abtop's existing infallible-config philosophy).
- No `--theme <absolute-path>` flag (name lookup only).
- No reload-on-file-change.
- No `abtop --list-themes` or `--dump-theme` CLI flag.
- No first-run scaffolding of `~/.config/abtop/themes/`.

These are tracked as Phase B follow-ups in the closing section.

## Architecture

```
xdg_config_dir() = ${XDG_CONFIG_HOME:-$HOME/.config}    # all platforms

startup:
  config.rs::load_config()  →  AppConfig {
                                   theme: String,
                                   theme_background: bool,   ← new
                                   ...
                                 }

  theme::loader::load(name, &cfg)  →  Theme
    1. xdg_config_dir()/abtop/themes/<name>.theme    (user file)
    2. EMBEDDED[<name>] via include_str!             (bundled)
    3. EMBEDDED["btop"]                              (last resort)
    → parse_theme_body(&str) → Theme
    → apply_overrides(&mut theme, &cfg)

  app.rs / ui/mod.rs render with Theme  (unchanged)
```

## Path resolution

A new helper, used everywhere abtop touches its config tree:

```rust
fn xdg_config_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() { return PathBuf::from(x); }
    }
    if let Some(home) = dirs::home_dir() { return home.join(".config"); }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from("."))
}
```

Applied to both:
- `config.toml`: `xdg_config_dir().join("abtop").join("config.toml")`
- Theme files: `xdg_config_dir().join("abtop").join("themes").join(format!("{name}.theme"))`

**Behavior change** for macOS users with an existing `~/Library/Application Support/abtop/config.toml`: the new path is `~/.config/abtop/config.toml`. No migration step is included in Phase A (the target machine has no existing config there).

## Theme file format

btop-compatible shell-array syntax. File extension: `.theme`. One key per line; lines that don't match are ignored (comments, blanks, anything weird).

Recognized line shape, ignoring whitespace:

```
theme[<key>]="<value>"
```

**Value forms:**
- `""` (empty) → `Color::Reset` (use terminal default) for any `Color`-typed field. **Uniform** — applies to any key, not just `main_bg`.
- `"#RRGGBB"` (6-digit hex, case-insensitive) → `Color::Rgb(r, g, b)`.
- `"#RGB"` (3-digit hex, case-insensitive) → expanded as `#RRGGBB` (`#abc` → `#aabbcc`). btop compatibility.

**Gradient keys are an exception** to the empty-value rule. `Gradient.start/mid/end` are `(u8, u8, u8)` tuples (used for foreground glyph interpolation in sparklines), not `Color` — there is no "reset" representation. An empty value on a gradient key (`theme[cpu_grad_start]=""`) is treated as missing and falls back to the embedded `"btop"` value for that key.

**Unknown values** (anything else): treated as missing — that key falls back to the embedded `"btop"` theme's value for that field.

**Unknown keys** (`theme[future_key]="..."`): silently ignored. Forward-compat.

**Missing keys**: filled in from the embedded `"btop"` theme. A partial theme file therefore works (users can author a "just override these 3 colors" file).

### Full key list (39)

Base (12): `main_bg`, `main_fg`, `title`, `hi_fg`, `selected_bg`, `selected_fg`, `inactive_fg`, `graph_text`, `meter_bg`, `proc_misc`, `div_line`, `session_id`.

Semantic (2): `status_fg`, `warning_fg`.

Box borders (4): `cpu_box`, `mem_box`, `net_box`, `proc_box`.

Gradients (5 × 3 = 15): `<grad>_start`, `<grad>_mid`, `<grad>_end` for each of `cpu_grad`, `proc_grad`, `used_grad`, `free_grad`, `cached_grad`. (Explicit `_grad_` naming preserves struct-field correspondence.)

### Example — catppuccin

```sh
# Catppuccin Mocha — main_bg empty for transparent terminal background.

theme[main_bg]=""
theme[main_fg]="#CDD6F4"
theme[title]="#CDD6F4"
theme[hi_fg]="#F38BA8"
theme[selected_bg]="#313244"
theme[selected_fg]="#CDD6F4"
theme[inactive_fg]="#6C7086"
theme[graph_text]="#9399B2"
theme[meter_bg]="#313244"
theme[proc_misc]="#A6E3A1"
theme[div_line]="#45475A"
theme[session_id]="#F9E2AF"

theme[status_fg]="#F38BA8"
theme[warning_fg]="#F9E2AF"

theme[cpu_box]="#89B4FA"
theme[mem_box]="#CBA6F7"
theme[net_box]="#F5C2E7"
theme[proc_box]="#F2CDCD"

theme[cpu_grad_start]="#A6E3A1"
theme[cpu_grad_mid]="#F9E2AF"
theme[cpu_grad_end]="#F38BA8"

theme[proc_grad_start]="#94E2D5"
theme[proc_grad_mid]="#F9E2AF"
theme[proc_grad_end]="#F38BA8"

theme[used_grad_start]="#313244"
theme[used_grad_mid]="#F5C2E7"
theme[used_grad_end]="#F38BA8"

theme[free_grad_start]="#1E1E2E"
theme[free_grad_mid]="#A6E3A1"
theme[free_grad_end]="#94E2D5"

theme[cached_grad_start]="#1E1E2E"
theme[cached_grad_mid]="#89B4FA"
theme[cached_grad_end]="#CBA6F7"
```

## Config additions

One new key in `config.toml`:

```toml
theme = "catppuccin"
theme_background = false   # default true; false ⇒ override main_bg with terminal default
```

- Added to `AppConfig` as `pub theme_background: bool` with default `true` (zero behavior change for users who don't set it).
- Parsed via a new arm in `parse_config_body`: `"theme_background" => config.theme_background = parse_bool(val).unwrap_or(true)`.
- Save support via existing `rewrite_kv_lines` for future Phase-B UI toggles.

## Override precedence

`apply_overrides(&mut theme, &cfg)`:

```rust
if !cfg.theme_background {
    theme.main_bg = Color::Reset;
}
```

Truth table:

| File `theme[main_bg]` | `theme_background` | Final `theme.main_bg` |
|---|---|---|
| `"#1E1E2E"` | `true` (default) | `Color::Rgb(30, 30, 46)` (opaque) |
| `"#1E1E2E"` | `false` | `Color::Reset` (transparent — global wins) |
| `""` | `true` | `Color::Reset` (file already empty) |
| `""` | `false` | `Color::Reset` |

Override applies only to `main_bg`. `selected_bg` and `meter_bg` keep their theme values (matches btop semantics — those are visible-state indicators, not backgrounds).

## Module shape

```
src/
  theme.rs                  → split into:
  theme/
    mod.rs                  ← re-exports Theme, Gradient (public surface unchanged)
    loader.rs               ← discovery, parsing, overrides
    embedded.rs             ← const BUILTIN: &[(&str, &str)] from include_str!

themes/                     ← new directory, 12 files embedded via include_str!
  btop.theme
  dracula.theme
  catppuccin.theme
  tokyo-night.theme
  gruvbox.theme
  nord.theme
  light.theme
  white.theme
  high-contrast.theme
  protanopia.theme
  deuteranopia.theme
  tritanopia.theme
```

**Two entry points** (the existing single one was ambiguous once a fallback chain exists):

- `Theme::by_name(name) -> Option<Self>` — keeps its current signature. Returns `Some` iff `name` resolves to either a user-dir file or an embedded entry. No last-resort fallback. Used by validation paths (`THEME_NAMES`-style membership checks, the `t`-cycle).
- `Theme::load_or_default(name, &cfg) -> Self` — new. Full chain: user file → embedded → embedded `"btop"`. Applies `apply_overrides`. Used by startup and `--theme <name>`.

The 12 Rust constructors (`Theme::btop()`, `Theme::catppuccin()`, ...) are deleted.

## Type changes

- `Theme::name`: `&'static str` → `String`. Required because user-loaded theme names aren't `'static`. Cascades:
  - `app.rs:477` — `.position(|&n| n == self.theme.name)`: the `&n` arm has type `&&'static str` from `THEME_NAMES`; comparison against `String` works via `Deref<Target=str>` on the right-hand side, so the line is unchanged.
  - `ui/config.rs:49` — `app.theme.name.to_string()` keeps compiling (`String::to_string` is a clone). No edit needed; line is touched only if we want to drop the redundant alloc.
  - 12 embedded constructors are deleted (themes load from files), so the field initializer disappears with them.
  - 1 existing test in `theme.rs` (`assert_eq!(t.name, "btop")`) keeps working — `String == &str` is defined via `PartialEq`.

- `THEME_NAMES`: `&'static [&'static str]` stays as-is, derived as the keys of `embedded::BUILTIN`. Used only for the `t`-cycle, which in Phase A still cycles the embedded set.

## Error handling

Matches abtop's existing infallible-config philosophy:
- Theme file missing → fall through to embedded.
- Theme file present but malformed (no valid keys) → fall through to embedded.
- Missing individual keys → backfill from embedded `"btop"`.
- Malformed value (not empty, not `#hex`) → treat key as missing.
- All embedded themes fail to parse → unrecoverable; panic at startup with a clear message (this would be a bug in the shipped `.theme` files, caught by tests).

No banner, no log line, no warning in Phase A. The `--debug` path (Phase B) can add diagnostics.

## File-by-file change plan

| File | Change |
|---|---|
| `src/theme.rs` | Delete; replaced by `src/theme/` module. |
| `src/theme/mod.rs` | New. `pub use loader::*; pub use embedded::THEME_NAMES;`. Re-exports `Theme`, `Gradient` from a new shared types module or inline. |
| `src/theme/loader.rs` | New. `parse_theme_body`, `load(name, &cfg) -> Theme`, `apply_overrides`, `parse_hex`. ~150 LOC including tests. |
| `src/theme/embedded.rs` | New. `pub const BUILTIN: &[(&str, &str)] = &[("btop", include_str!("../../themes/btop.theme")), ...]`. Derives `THEME_NAMES`. |
| `src/config.rs` | Add `theme_background: bool` to `AppConfig` (default `true`). Add parse arm. Add `xdg_config_dir()` helper, replace `dirs::config_dir()` callsite. New test for parse round-trip. |
| `src/app.rs` | Use `theme::load(name, &cfg)` instead of `Theme::by_name(name).unwrap_or_default()`. |
| `src/ui/config.rs` | Adjust `theme.name` access for `String` type. |
| `themes/*.theme` (×12) | New files. Hex values transcribed from current Rust constants. |
| `README.md` | Document `~/.config/abtop/themes/`, `theme_background`, btop-compat note. |
| `CLAUDE.md` / `AGENTS.md` | Not changed (local fork; English-only policy already in place). |

## Test surface

In `theme/loader.rs`:
- `parse_theme_body` — happy path (`theme[k]="#hex"` → field set), 3-digit hex expansion, mixed-case hex, whitespace tolerance, empty value → `Color::Reset`, unknown key ignored, malformed line ignored.
- `parse_hex` — valid 6-digit, valid 3-digit, invalid (too short, non-hex chars, missing `#`).
- `apply_overrides` — all four cells of the precedence truth table.
- Embedded sanity loop — for every `(name, body)` in `BUILTIN`, `parse_theme_body(body)` returns a fully-populated `Theme` with no panic. Catches drift between Rust struct fields and shipped `.theme` files.
- User-file shadows embedded — temp-dir test: write `<tmp>/btop.theme` with a distinct `main_fg`, verify it wins over embedded `btop`.

In `config.rs`:
- `theme_background` round-trip (true / false / missing → default true).
- `xdg_config_dir()` honors `XDG_CONFIG_HOME` when set, falls back to `$HOME/.config`.

## Build & install (local fork)

```sh
cd ~/my-workspace/util/abtop
cargo test --release        # all green, including new tests
cargo build --release
install -m 755 target/release/abtop ~/.local/libexec/abtop

# Smoke test
mkdir -p ~/.config/abtop
printf 'theme = "catppuccin"\ntheme_background = false\n' > ~/.config/abtop/config.toml
abtop                       # expect terminal background showing through
```

## Phase B (deferred follow-ups, separate spec)

- `t`-cycle picks up user-dir themes (scan `xdg_config_dir()/abtop/themes/*.theme` at startup, merge with embedded set, deterministic order).
- Malformed theme file → banner in the UI footer with the parse error and offending line.
- `abtop --theme <path>` accepts absolute paths.
- `abtop --list-themes` prints embedded + user themes.
- `abtop --dump-theme <name>` writes the embedded theme to `xdg_config_dir()/abtop/themes/<name>.theme` for in-place editing.
- Reload-on-file-change (watch user dir).
- One-time migration of `~/Library/Application Support/abtop/config.toml` on macOS startup if XDG path is empty.
