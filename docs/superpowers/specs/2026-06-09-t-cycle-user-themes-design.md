# Phase B2 — `t`-cycle picks up user-dir themes — design

**Status:** Approved (local fork)
**Date:** 2026-06-09
**Scope:** Phase B item 2. Extend the interactive `t` key to cycle through user-dir themes alongside embedded ones.
**Target:** `~/my-workspace/util/abtop` (local fork of `graykode/abtop`, currently at commit `f51db3c`). No upstream PR.
**Builds on:** Phase A + B1 (`f51db3c`) — `theme::list_available()`, `App::cycle_theme()`, `THEME_NAMES` const, `config::xdg_config_dir()`.

## Goal

Pressing `t` in the TUI cycles through all themes the user can actually pick: the 13 embedded themes plus any `*.theme` files in `$XDG_CONFIG_HOME/abtop/themes/`. The cycle order matches `list_available()` output (embedded in BUILTIN order, then user-only themes alphabetically).

## Non-goals

- No new lookup chain — reuses `theme::list_available()` from B1.
- No live-rescan on every `t`-press. Cycle list is cached at App startup.
- No banner / status message about user themes (already covered by `--list-themes` from B1).
- No reload-on-file-change (Phase B5).
- No change to `--theme <name>` CLI validation — that still goes through `Theme::by_name`.

## Architecture

The cycle list moves from a compile-time const (`crate::theme::THEME_NAMES`) onto the `App` struct as a runtime field populated at startup. `cycle_theme()` reads `self.cycle_names` instead of the const. Mid-session theme file additions don't appear in the cycle until the next launch.

```
                       startup
                          │
                          ▼
                   lib.rs::build_app()
                          │
                ┌─────────┴───────────┐
                ▼                     ▼
       App::new_with_config()   list_available(&xdg_config_dir())
                │                     │
                │                     ▼
                │              Vec<ThemeListing>
                │                     │
                │                     ▼
                │              .iter().map(|l| l.name.clone()).collect()
                │                     │
                ▼                     ▼
       app.cycle_names ← Vec<String> (cached, lives for app's lifetime)
                          │
                          ▼
                  app runs; user presses 't'
                          │
                          ▼
            App::cycle_theme() reads self.cycle_names
                          │
                          ▼
            next name → load_or_default + apply_overrides + save_theme
```

## Code changes

### `src/app.rs` — new field + cycle-list source

Add a new private field on `App`:

```rust
pub struct App {
    // ... existing fields ...
    /// Theme names the `t` key cycles through. Built once at startup from
    /// `theme::list_available()` so user-dir themes appear in the cycle.
    /// Falls back to `crate::theme::THEME_NAMES` when empty (tests / older
    /// construction paths).
    cycle_names: Vec<String>,
}
```

Initialize it to `Vec::new()` in every existing `App` constructor (`new_with_config`, etc.). This preserves the test path: empty `cycle_names` → fall back to the embedded `THEME_NAMES` const.

Add one setter:

```rust
impl App {
    /// Set the theme-cycle list. Called by `build_app` at startup with the
    /// output of `theme::list_available()`. Empty input is accepted and
    /// triggers the `THEME_NAMES` fallback in `cycle_theme`.
    pub(crate) fn set_cycle_names(&mut self, names: Vec<String>) {
        self.cycle_names = names;
    }
}
```

(`pub(crate)` because only `build_app` calls it; same minimal-surface discipline as the B1 final review.)

Update `cycle_theme()`:

```rust
pub fn cycle_theme(&mut self) {
    // Use the runtime cycle list when populated; fall back to the
    // embedded const for tests / older paths.
    let names_owned: Vec<&str>;
    let names: &[&str] = if self.cycle_names.is_empty() {
        crate::theme::THEME_NAMES
    } else {
        names_owned = self.cycle_names.iter().map(|s| s.as_str()).collect();
        &names_owned
    };

    let current = names
        .iter()
        .position(|&n| n == self.theme.name)
        .unwrap_or(0);
    let next = (current + 1) % names.len();
    let cfg = crate::config::load_config();
    let mut new_theme = Theme::by_name(names[next]).unwrap_or_default();
    crate::theme::apply_overrides(&mut new_theme, &cfg);
    self.theme = new_theme;
    // ... existing save_theme + status message stays unchanged
}
```

The `Vec<&str>` reborrow keeps the type uniform between the const and the runtime fallback.

### `src/lib.rs::build_app` — populate at startup

After `App::new_with_config` returns and before `build_app` ends, set the cycle list:

```rust
fn build_app(theme: theme::Theme, cfg: &config::AppConfig) -> App {
    let mut app = App::new_with_config(
        theme,
        &cfg.hidden_agents,
        cfg.panels,
        &cfg.claude_config_dirs,
    );
    let listings = theme::list_available(&config::xdg_config_dir());
    let cycle_names: Vec<String> = listings.into_iter().map(|l| l.name).collect();
    app.set_cycle_names(cycle_names);
    app
}
```

(Adjust to the exact `build_app` signature — refer to the existing function around `src/lib.rs:78-88`.)

## Behavior table

| State | Effect on `t`-press |
|---|---|
| No user themes; saved theme `catppuccin` | cycle: btop → dracula → catppuccin → catppuccin-transparent → … → tritanopia → btop. **Same as today.** |
| User has `my-cool.theme`; on catppuccin | cycle still passes through embedded 13 first, then `my-cool` as a 14th slot. |
| User has `catppuccin.theme` (shadow) + `my-cool.theme` | catppuccin slot is now a `UserOverride`; still one slot. Cycle has 14 entries (13 embedded names + `my-cool`). |
| Saved theme `my-cool` but file deleted between launches | `Theme::by_name("my-cool")` returns None; `Theme::default()` → btop. Cycle list still has 13 (embedded) entries. `position(...)` returns None for "my-cool" → fallback to 0 → cycle starts at btop. |
| User drops `new.theme` mid-session | Won't appear in cycle until next launch. Documented limitation; B5 (reload-on-edit) addresses it. |

## Public surface change

- `App.cycle_names: Vec<String>` — new private field.
- `App::set_cycle_names(&mut self, Vec<String>)` — new public method (crate-internal, used only by `build_app`).

Nothing new exposed externally via `theme::` re-exports. `THEME_NAMES` stays as-is.

## Test surface

In `src/app.rs` (or a new test module if `app.rs` is already too large):

- `cycle_theme_falls_back_to_THEME_NAMES_when_cycle_names_empty` — construct App with `Theme::default()` and don't call `set_cycle_names`. Press `t` (call `cycle_theme()` directly). Verify the new theme is `dracula` (the second entry in BUILTIN order).
- `cycle_theme_uses_app_cycle_names_when_set` — construct App on `Theme::default()` (which is btop), then `set_cycle_names(vec!["dracula".to_string(), "btop".to_string()])`. Set current theme name to "dracula". Press `t` (call `cycle_theme()`). Verify the new theme name is "btop". Both names are embedded so no tempdir is needed; the test proves the cycle walks the runtime list rather than `THEME_NAMES`.
- Existing snapshot/UI tests (`Theme::default(), &[], PanelVisibility::default()` shape) must continue passing unchanged — they hit the fallback path.

Manual smoke test:

```sh
# Verify embedded-only cycle (no user themes)
abtop                               # press t several times, observe footer / save
# Verify user-theme cycle
mkdir -p ~/.config/abtop/themes
cp themes/catppuccin.theme ~/.config/abtop/themes/zorak.theme
abtop                               # press t; expect zorak to appear after tritanopia
rm ~/.config/abtop/themes/zorak.theme
```

## Implementation notes

- `cycle_names` field is `Vec<String>` (owned) rather than `Vec<&'static str>` because user-dir theme names are not `'static`. Cost: ~13 small allocations on startup. Negligible.
- The `Vec<&str>` reborrow inside `cycle_theme` is a stack-local view to keep the comparison/indexing code identical for both code paths. No heap allocation in the press loop itself beyond what `cycle_theme` already does (`save_theme` writes to disk on every press).
- `set_cycle_names` is `pub(crate)` — same minimal-surface discipline that B1's final review enforced for the `theme::` re-exports.

## Error handling philosophy

Matches Phase A/B1: infallible-where-possible.
- `list_available` is infallible (filesystem errors yield embedded-only list).
- Empty `cycle_names` falls back to `THEME_NAMES`.
- An unknown `self.theme.name` in the cycle (theme deleted mid-session, or save-corrupted) starts the cycle at index 0.
- `Theme::by_name(names[next]).unwrap_or_default()` covers the case where a user file vanished between startup-scan and the `t`-press.

## Build & install

Same as Phase A/B1:

```sh
cd ~/my-workspace/util/abtop
cargo test --lib
cargo build --release
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

## Acceptance criteria

1. `cargo test --lib` passes (test count grows by 2 new App tests).
2. `cargo build --release` clean.
3. With no user themes: `t` cycles through 13 embedded themes exactly as before.
4. With `~/.config/abtop/themes/my-cool.theme` present at startup: `t` cycles through 14 entries (13 embedded + `my-cool`); `my-cool` appears after `tritanopia`.
5. Saved theme `my-cool` survives a relaunch even if the file was deleted (falls back to btop, name preserved in config.toml).
6. No new `pub` items in `theme::` re-exports.

## Out of scope (other Phase B items)

- B3: `abtop --theme <absolute-path>` — separate spec.
- B4: Banner UI on malformed theme file — separate spec.
- B5: Reload-on-file-change — separate spec.
- B6: macOS `~/Library/Application Support/abtop/` → XDG migration — deferred indefinitely.
