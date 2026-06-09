# `t`-cycle picks up user-dir themes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `App::cycle_theme()` walk a runtime list populated from `theme::list_available()` at startup, so dropping `*.theme` files into `$XDG_CONFIG_HOME/abtop/themes/` makes them appear in the `t`-key cycle after the next launch.

**Architecture:** Add a private `cycle_names: Vec<String>` field on `App` that defaults empty. `cycle_theme()` walks `self.cycle_names` when populated, falls back to the existing `crate::theme::THEME_NAMES` const when empty. `build_app()` in `lib.rs` populates the field at startup via `theme::list_available()`. Tests pass through the fallback path with zero code changes at their callsites.

**Tech Stack:** Rust 2021, no new dependencies. Reuses `theme::list_available()` from Phase B1.

**Spec:** `docs/superpowers/specs/2026-06-09-t-cycle-user-themes-design.md` (commit `33e6805`).

---

## File Structure

| Path | Change | Responsibility |
|---|---|---|
| `src/app.rs` | MODIFY | Add `cycle_names: Vec<String>` field, `set_cycle_names` setter, and rewrite `cycle_theme()` to walk the runtime list with a const-fallback. Add 2 inline tests. |
| `src/lib.rs` | MODIFY | `build_app()` populates `cycle_names` from `theme::list_available()` after constructing the App. |

No new files. No new dependencies. No changes to `theme::` re-exports.

---

## Task 1: Add `cycle_names` field, `set_cycle_names` setter, rewrite `cycle_theme`, add 2 TDD tests

This is one cohesive change to `App` — the field, the initializer, the setter, and the method that consumes the field all land together. Splitting them would either leave a half-wired field (warning) or skip the TDD step.

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add failing tests**

Open `src/app.rs` and find the existing `#[cfg(test)] mod tests` block (if there isn't one, scroll to the bottom of the file). Append:

```rust
#[cfg(test)]
mod cycle_theme_tests {
    use super::*;
    use crate::config::PanelVisibility;

    #[test]
    fn cycle_theme_falls_back_to_THEME_NAMES_when_cycle_names_empty() {
        // App constructed without set_cycle_names: cycle_names is empty,
        // so cycle_theme falls back to the embedded THEME_NAMES const.
        let mut app = App::new_with_config(
            Theme::default(),
            &[],
            PanelVisibility::default(),
        );
        // Theme::default() is btop (THEME_NAMES[0]); cycling moves to dracula (index 1).
        assert_eq!(app.theme.name, "btop");
        app.cycle_theme();
        assert_eq!(app.theme.name, "dracula");
    }

    #[test]
    fn cycle_theme_uses_app_cycle_names_when_set() {
        // App with an explicit cycle list of just ["dracula", "btop"].
        // Cycling from dracula must land on btop (the runtime list, not
        // the embedded THEME_NAMES order which would go dracula -> catppuccin).
        let mut app = App::new_with_config(
            Theme::by_name("dracula").expect("dracula in BUILTIN"),
            &[],
            PanelVisibility::default(),
        );
        app.set_cycle_names(vec!["dracula".to_string(), "btop".to_string()]);
        assert_eq!(app.theme.name, "dracula");
        app.cycle_theme();
        assert_eq!(app.theme.name, "btop");
    }
}
```

- [ ] **Step 2: Verify the tests fail**

Run from `/Users/a.salvi/my-workspace/util/abtop`:

```bash
cargo test --lib --quiet app::cycle_theme_tests 2>&1 | tail -10
```

Expected: compile error — `set_cycle_names` not found, and possibly `cycle_names` field not found if reachable from the test path.

- [ ] **Step 3: Add the field to the `App` struct**

Open `src/app.rs` and find the `pub struct App` block (around line 89). The last field before the closing `}` is `pub view_open: bool,` (around line 150). Add the new field immediately after, just before the closing `}`:

```rust
    /// View leader overlay (`v`) visibility.
    pub view_open: bool,
    /// Theme names the `t` key cycles through. Built once at startup
    /// from `theme::list_available()` so user-dir themes appear in the
    /// cycle. Empty → fall back to `crate::theme::THEME_NAMES` (used by
    /// tests and any code path that constructs `App` without calling
    /// `set_cycle_names`).
    cycle_names: Vec<String>,
}
```

- [ ] **Step 4: Initialize the field in `new_with_config_and_claude_dirs`**

Still in `src/app.rs`, find the struct literal inside `new_with_config_and_claude_dirs` (around lines 174-217). The last initializer is `view_open: false,`. Add immediately after:

```rust
            help_open: false,
            view_open: false,
            cycle_names: Vec::new(),
        }
    }
```

- [ ] **Step 5: Add `set_cycle_names` to the `impl App` block**

Locate the `impl App { ... }` block. A natural spot for the new method is alongside `set_status` (around line 492) or right after `cycle_theme`. Place it just below `cycle_theme`:

```rust
    /// Set the list of theme names the `t` key cycles through. Called by
    /// `build_app` at startup with the output of `theme::list_available()`
    /// so user-dir themes appear in the cycle alongside embedded ones.
    /// Empty input is accepted and triggers the `THEME_NAMES` fallback
    /// in `cycle_theme`.
    pub(crate) fn set_cycle_names(&mut self, names: Vec<String>) {
        self.cycle_names = names;
    }
```

- [ ] **Step 6: Rewrite `cycle_theme`**

Replace the existing `cycle_theme` body (around lines 473-489):

```rust
    pub fn cycle_theme(&mut self) {
        // Use the runtime cycle list when populated; fall back to the
        // embedded const for tests / older construction paths.
        let next_name: String = if self.cycle_names.is_empty() {
            let names = crate::theme::THEME_NAMES;
            let current = names
                .iter()
                .position(|&n| n == self.theme.name)
                .unwrap_or(0);
            let next = (current + 1) % names.len();
            names[next].to_string()
        } else {
            let current = self
                .cycle_names
                .iter()
                .position(|n| n == &self.theme.name)
                .unwrap_or(0);
            let next = (current + 1) % self.cycle_names.len();
            self.cycle_names[next].clone()
        };
        let cfg = crate::config::load_config();
        let mut new_theme = Theme::by_name(&next_name).unwrap_or_default();
        crate::theme::apply_overrides(&mut new_theme, &cfg);
        self.theme = new_theme;
        if let Err(e) = crate::config::save_theme(&next_name) {
            self.set_status(format!("theme: {} (save failed: {})", next_name, e));
        } else {
            self.set_status(format!("theme: {}", next_name));
        }
    }
```

The two branches differ only in where they pull from; the rest (load_config, by_name, apply_overrides, save_theme, set_status) is identical to the existing code.

- [ ] **Step 7: Verify the tests pass**

```bash
cargo test --lib --quiet app::cycle_theme_tests 2>&1 | tail -5
```

Expected: 2 tests pass.

- [ ] **Step 8: Verify the full suite passes**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 216 passed (214 + 2 new). The existing snapshot/UI tests that construct App with `new_with_config(Theme::default(), &[], PanelVisibility::default())` still work because `cycle_names` defaults to `Vec::new()` and `cycle_theme` falls back to `THEME_NAMES`.

- [ ] **Step 9: Verify the build is clean (no warnings)**

```bash
cargo build --release 2>&1 | tail -5
```

Expected: clean release build, no warnings about unused fields or dead code.

- [ ] **Step 10: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): cycle_theme walks runtime cycle_names with const fallback

Adds a private cycle_names: Vec<String> field on App that defaults
empty. cycle_theme walks self.cycle_names when populated and falls
back to crate::theme::THEME_NAMES otherwise, so tests and older
construction paths work unchanged. A pub(crate) set_cycle_names
setter is added for build_app to populate the field at startup."
```

---

## Task 2: Populate `cycle_names` in `build_app`

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Locate `build_app`**

Open `src/lib.rs`. The function is around lines 79-88. It currently reads:

```rust
fn build_app(theme: theme::Theme, cfg: &config::AppConfig) -> App {
    App::new_with_config_and_claude_dirs(
        theme,
        &cfg.hidden_agents,
        cfg.panels,
        &cfg.claude_config_dirs,
    )
}
```

- [ ] **Step 2: Rewrite the function to populate `cycle_names`**

Replace the body with:

```rust
fn build_app(theme: theme::Theme, cfg: &config::AppConfig) -> App {
    let mut app = App::new_with_config_and_claude_dirs(
        theme,
        &cfg.hidden_agents,
        cfg.panels,
        &cfg.claude_config_dirs,
    );
    let cycle_names: Vec<String> = theme::list_available(&config::xdg_config_dir())
        .into_iter()
        .map(|l| l.name)
        .collect();
    app.set_cycle_names(cycle_names);
    app
}
```

`theme::list_available` is `pub(crate)` (per B1 final-review tightening); `src/lib.rs` is in the same crate so this is reachable.

- [ ] **Step 3: Verify the build is clean**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: clean release build.

- [ ] **Step 4: Verify the full test suite passes**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 216 passed (no regression — `build_app` isn't covered by unit tests, but other code that exercises the production path stays green).

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs
git commit -m "feat(lib): populate App.cycle_names from theme::list_available at startup

build_app now calls list_available right after constructing the App
and pipes the names into set_cycle_names. Mid-session theme file
additions still require a restart; B5 (reload-on-edit) will address
that."
```

---

## Task 3: Build, install, and smoke test

**Files:** none (build + install + manual smoke)

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 216 passed.

- [ ] **Step 2: Build release**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: `Finished` with no errors.

- [ ] **Step 3: Install**

```bash
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

- [ ] **Step 4: Smoke test — embedded-only cycle still works**

Verify no user themes are present first:

```bash
~/.local/libexec/abtop --list-themes | head -3
```

If any line ends with `(user)` or `(user override)`, move them aside before continuing:

```bash
mkdir -p /tmp/abtop-stash
[ -d ~/.config/abtop/themes ] && mv ~/.config/abtop/themes /tmp/abtop-stash/
```

Confirm no user themes:

```bash
~/.local/libexec/abtop --list-themes | rg '(user|user override)'
# Expect: no output
```

- [ ] **Step 5: Smoke test — user theme appears in the cycle list**

Drop a user theme:

```bash
mkdir -p ~/.config/abtop/themes
cp themes/catppuccin.theme ~/.config/abtop/themes/zorak.theme
```

Run abtop and verify it sees the user theme in the listing:

```bash
~/.local/libexec/abtop --list-themes | tail -3
```

Expected: the last line is `zorak (user)`.

- [ ] **Step 6: Smoke test — cycle reaches the user theme**

This requires the interactive TUI. From a real terminal:

```bash
~/.local/libexec/abtop
```

Press `t` 13 times to walk through the embedded themes; the 14th press should land on `zorak`. The footer should show `theme: zorak`. Press `q` to quit.

Skip this step if you don't have a terminal available right now — the unit tests already prove the cycle walk uses the runtime list.

- [ ] **Step 7: Verify the saved theme survives between launches**

After cycling to `zorak` and quitting:

```bash
rg '^theme = ' ~/.config/abtop/config.toml
```

Expected: `theme = "zorak"`.

Now relaunch abtop briefly via `--once` and quit:

```bash
~/.local/libexec/abtop --once | head -1
echo "exit: $?"
```

Expected: a normal snapshot prints; exit 0. (The startup path resolved `zorak` from the user dir.)

- [ ] **Step 8: Clean up**

```bash
rm ~/.config/abtop/themes/zorak.theme
# If you stashed earlier:
[ -d /tmp/abtop-stash/themes ] && mv /tmp/abtop-stash/themes ~/.config/abtop/themes && rmdir /tmp/abtop-stash
```

Reset the config theme so the next session starts clean:

```bash
~/.local/libexec/abtop --theme btop --once > /dev/null
```

- [ ] **Step 9: No commit needed — install is a side effect.**

---

## Acceptance criteria

1. `cargo test --lib` passes (216 tests; was 214 at B2 start, +2 new).
2. `cargo build --release` clean.
3. With no user themes, pressing `t` cycles through the 13 embedded themes exactly as before B2.
4. With `~/.config/abtop/themes/zorak.theme` present at startup, the cycle has 14 entries; `zorak` appears after `tritanopia`.
5. A saved theme name pointing to a user file survives the next launch (file present); fallback to btop when the file is removed.
6. No new `pub` items in `theme::` or other module re-exports.

## Out of scope (other Phase B items)

- B3: `abtop --theme <absolute-path>` — separate spec.
- B4: Banner UI on malformed theme file — separate spec.
- B5: Reload-on-file-change — separate spec.
- B6: macOS Library → XDG migration — deferred indefinitely.
