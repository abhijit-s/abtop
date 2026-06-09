# Theme reload on file change Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the file backing the active theme is edited mid-session, abtop detects the new mtime within ~2 seconds and reloads — no restart needed.

**Architecture:** Add a `ThemeSource { path, mtime }` struct + an optional field on `App`. At startup, `lib.rs::run()` computes the source from the resolved theme name or path. A new `App::check_for_theme_reload` method polls `fs::metadata(path)` and, if mtime advanced, re-reads the file via existing `theme::load_from_path_with_errors`, applies config overrides, and swaps `self.theme`. The reload check is called at the END of the existing `App::tick`, so headless `tick_no_summaries` (used by `--json`/`--once`) does NOT trigger it.

**Tech Stack:** Rust 2021, std (`std::fs`, `std::time::SystemTime`, `std::path::PathBuf`). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-06-09-theme-reload-on-change-design.md` (commit `c544cfe`).

---

## File Structure

| Path | Change | Responsibility |
|---|---|---|
| `src/app.rs` | MODIFY | Add `ThemeSource` struct, `theme_source` field, `set_theme_source` setter, `check_for_theme_reload` method, and a hook into `App::tick`. Add 7 TDD tests. |
| `src/lib.rs` | MODIFY | Compute the optional `ThemeSource` after resolving `initial_theme`; pass via `set_theme_source` to each `build_app`/`run_app` construction site. |

No new files. No new dependencies.

---

## Task 1: Add `ThemeSource`, field, and setter on `App`

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add a failing test**

Append at the bottom of `src/app.rs` (after the existing test modules):

```rust
#[cfg(test)]
mod theme_source_tests {
    use super::*;
    use crate::config::PanelVisibility;
    use std::path::PathBuf;

    #[test]
    fn set_theme_source_round_trip() {
        let mut app = App::new_with_config(
            Theme::default(),
            &[],
            PanelVisibility::default(),
        );
        let src = ThemeSource {
            path: PathBuf::from("/tmp/x.theme"),
            mtime: None,
        };
        app.set_theme_source(Some(src.clone()));
        // Field is private; verify via re-set + tick no-op semantics.
        // We can't assert equality directly without exposing the field,
        // so this test just exercises construct + set + clear paths.
        app.set_theme_source(None);
    }
}
```

- [ ] **Step 2: Verify the test fails**

Run from `/Users/a.salvi/my-workspace/util/abtop`:

```bash
cargo test --lib --quiet app::theme_source_tests 2>&1 | tail -10
```

Expected: compile error — `ThemeSource`, `App::set_theme_source` not found.

- [ ] **Step 3: Add the `ThemeSource` struct**

In `src/app.rs`, near the top of the file (right after the existing `use` statements; sensible location: just before `pub struct App`), add:

```rust
/// Where the active theme came from on disk. Used by `App::tick` to
/// detect mid-session edits and reload. `None` for embedded themes
/// (no source file to watch).
#[derive(Clone, Debug)]
pub(crate) struct ThemeSource {
    /// Absolute path to the `.theme` file.
    pub path: std::path::PathBuf,
    /// Modification time at the last successful read; `None` means the
    /// file was missing at the last check, so a returned-to-existence
    /// path with any mtime triggers a fresh reload.
    pub mtime: Option<std::time::SystemTime>,
}
```

- [ ] **Step 4: Add the field to `App`**

Find `pub struct App { ... }` (around line 89). The last field is now (after B2) `cycle_names: Vec<String>,`. Add immediately after, before the closing `}`:

```rust
    /// Theme names the `t` key cycles through. Built once at startup
    /// from `theme::list_available()` so user-dir themes appear in the
    /// cycle. Empty → fall back to `crate::theme::THEME_NAMES`.
    cycle_names: Vec<String>,
    /// Source of the active theme on disk, or None for embedded.
    /// Polled in `App::tick` for mid-session reload (B5).
    theme_source: Option<ThemeSource>,
}
```

- [ ] **Step 5: Initialize the new field in the constructor**

Find the struct literal inside `new_with_config_and_claude_dirs` (around lines 174-220). The last initializer is `cycle_names: Vec::new(),`. Add immediately after:

```rust
            cycle_names: Vec::new(),
            theme_source: None,
        }
    }
```

- [ ] **Step 6: Add the setter**

Inside `impl App { ... }`, locate `pub(crate) fn set_cycle_names(...)` (added in B2). Add the new setter immediately below it:

```rust
    /// Set the source file watched by `App::tick` for mid-session reload.
    /// Called by `build_app` at startup with the resolved theme's path
    /// + initial mtime. `None` disables polling (used for embedded-only
    /// themes that have no file backing).
    pub(crate) fn set_theme_source(&mut self, source: Option<ThemeSource>) {
        self.theme_source = source;
    }
```

- [ ] **Step 7: Verify the test passes**

```bash
cargo test --lib --quiet app::theme_source_tests 2>&1 | tail -5
```

Expected: 1 test passes.

- [ ] **Step 8: Verify the full suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 248 passed (247 + 1 new).

- [ ] **Step 9: Verify clean release build**

```bash
cargo build --release 2>&1 | tail -5
```

Expected: clean. A `dead_code` warning on `theme_source` may appear because the field is set but never read until Task 2 — acceptable here.

- [ ] **Step 10: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): add ThemeSource field + setter for mid-session reload

Adds pub(crate) ThemeSource { path, mtime } and a private
theme_source: Option<ThemeSource> field on App, defaulting to None.
A new pub(crate) set_theme_source setter lets build_app populate it
at startup. Existing tests stay unchanged; the field is only read
by check_for_theme_reload (added in the next task)."
```

---

## Task 2: Add `check_for_theme_reload` + hook into `App::tick`

**Files:**
- Modify: `src/app.rs` (method + tick hook + 6 TDD tests)

- [ ] **Step 1: Add failing tests**

Append to the `theme_source_tests` module:

```rust
    use std::time::Duration;

    fn write_theme_file_with(path: &std::path::Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    fn read_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    #[test]
    fn check_for_theme_reload_no_source_is_noop() {
        let mut app = App::new_with_config(
            Theme::default(),
            &[],
            PanelVisibility::default(),
        );
        let original_name = app.theme.name.clone();
        app.check_for_theme_reload();
        assert_eq!(app.theme.name, original_name);
    }

    #[test]
    fn check_for_theme_reload_reloads_when_mtime_advances() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("scratch.theme");
        write_theme_file_with(&path, r##"theme[main_bg]="#111111""##);
        let old_mtime = read_mtime(&path);

        let mut app = App::new_with_config(
            Theme::default(),
            &[],
            PanelVisibility::default(),
        );
        app.set_theme_source(Some(ThemeSource { path: path.clone(), mtime: old_mtime }));

        // Sleep briefly so the next write produces a distinct mtime.
        // Filesystem mtime resolution is typically 1ns–1s; 50ms is plenty
        // for ext4/apfs and avoids slowing the test suite noticeably.
        std::thread::sleep(Duration::from_millis(50));
        write_theme_file_with(&path, r##"theme[main_bg]="#abcdef""##);

        app.check_for_theme_reload();
        // After reload, main_bg should reflect the new content.
        use ratatui::style::Color;
        assert_eq!(app.theme.main_bg, Color::Rgb(0xab, 0xcd, 0xef));
    }

    #[test]
    fn check_for_theme_reload_no_op_when_mtime_unchanged() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("scratch.theme");
        write_theme_file_with(&path, r##"theme[main_bg]="#111111""##);
        let mtime = read_mtime(&path);

        let mut app = App::new_with_config(
            Theme::default(),
            &[],
            PanelVisibility::default(),
        );
        let original_main_bg = app.theme.main_bg;
        app.set_theme_source(Some(ThemeSource { path, mtime }));

        // No write between set + tick → no reload.
        app.check_for_theme_reload();
        assert_eq!(app.theme.main_bg, original_main_bg);
    }

    #[test]
    fn check_for_theme_reload_handles_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("never-existed.theme");

        let mut app = App::new_with_config(
            Theme::default(),
            &[],
            PanelVisibility::default(),
        );
        let original_main_bg = app.theme.main_bg;
        app.set_theme_source(Some(ThemeSource { path, mtime: None }));

        // Should not panic, theme unchanged.
        app.check_for_theme_reload();
        assert_eq!(app.theme.main_bg, original_main_bg);
    }

    #[test]
    fn check_for_theme_reload_after_file_recreation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("scratch.theme");
        write_theme_file_with(&path, r##"theme[main_bg]="#111111""##);
        let mtime = read_mtime(&path);

        let mut app = App::new_with_config(
            Theme::default(),
            &[],
            PanelVisibility::default(),
        );
        app.set_theme_source(Some(ThemeSource { path: path.clone(), mtime }));

        // Delete + recreate with new content.
        std::fs::remove_file(&path).unwrap();
        app.check_for_theme_reload(); // file missing → no panic, mtime stays
        std::thread::sleep(Duration::from_millis(50));
        write_theme_file_with(&path, r##"theme[main_bg]="#abcdef""##);

        // Next tick should reload.
        app.check_for_theme_reload();
        use ratatui::style::Color;
        assert_eq!(app.theme.main_bg, Color::Rgb(0xab, 0xcd, 0xef));
    }

    #[test]
    fn check_for_theme_reload_preserves_theme_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("scratch.theme");
        write_theme_file_with(&path, r##"theme[main_bg]="#111111""##);
        let mtime = read_mtime(&path);

        // Start the app with a named theme (catppuccin) but point the
        // source at scratch.theme (path stem = "scratch"). After reload,
        // theme.name must stay "catppuccin", NOT become "scratch".
        let mut app = App::new_with_config(
            Theme::by_name("catppuccin").expect("embedded catppuccin"),
            &[],
            PanelVisibility::default(),
        );
        app.set_theme_source(Some(ThemeSource { path: path.clone(), mtime }));

        std::thread::sleep(Duration::from_millis(50));
        write_theme_file_with(&path, r##"theme[main_bg]="#abcdef""##);
        app.check_for_theme_reload();

        assert_eq!(app.theme.name, "catppuccin");
    }

    #[test]
    fn check_for_theme_reload_status_message_with_parse_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("scratch.theme");
        write_theme_file_with(&path, r##"theme[main_bg]="#111111""##);
        let mtime = read_mtime(&path);

        let mut app = App::new_with_config(
            Theme::default(),
            &[],
            PanelVisibility::default(),
        );
        app.set_theme_source(Some(ThemeSource { path: path.clone(), mtime }));

        // Rewrite with a bad hex.
        std::thread::sleep(Duration::from_millis(50));
        write_theme_file_with(&path, r##"theme[main_bg]="#XYZ""##);
        app.check_for_theme_reload();

        let status = app.status_msg.as_ref().map(|(s, _)| s.as_str()).unwrap_or("");
        assert!(
            status.contains("parse error"),
            "status should mention parse error, got: {status:?}"
        );
    }
```

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test --lib --quiet app::theme_source_tests::check_for_theme_reload 2>&1 | tail -10
```

Expected: compile errors — `check_for_theme_reload` not found.

- [ ] **Step 3: Implement `check_for_theme_reload`**

Inside `impl App { ... }`, add the new method (a natural spot is right after `set_theme_source`):

```rust
    /// Re-stat the active theme's source file and reload if mtime has
    /// advanced. Called from `App::tick`. No-op for embedded-only themes
    /// (`theme_source.is_none()`).
    fn check_for_theme_reload(&mut self) {
        let Some(source) = self.theme_source.as_mut() else {
            return;
        };
        let current_mtime = std::fs::metadata(&source.path)
            .and_then(|m| m.modified())
            .ok();
        let needs_reload = match (current_mtime, source.mtime) {
            (Some(now), Some(then)) => now > then,
            // File just appeared (was missing → present).
            (Some(_), None) => true,
            // File missing: keep current theme, no panic.
            (None, _) => {
                // Remember the missing state so re-creation triggers reload.
                source.mtime = None;
                return;
            }
        };
        if !needs_reload {
            return;
        }
        let cfg = crate::config::load_config();
        match crate::theme::load_from_path_with_errors(&source.path) {
            Ok((mut new_theme, errors)) => {
                crate::theme::apply_overrides(&mut new_theme, &cfg);
                // Preserve the existing theme's name so cycle-position
                // lookups stay sane and the user doesn't see a surprise
                // rename after editing.
                new_theme.name = self.theme.name.clone();
                self.theme = new_theme;
                source.mtime = current_mtime;
                let count = errors.len();
                let msg = if count == 0 {
                    format!("theme '{}' reloaded", self.theme.name)
                } else {
                    let suffix = if count == 1 { "" } else { "s" };
                    format!(
                        "theme '{}' reloaded with {count} parse error{suffix}",
                        self.theme.name
                    )
                };
                self.set_status(msg);
            }
            Err(_) => {
                // Transient read error (partial write etc.). Try again
                // next tick — don't advance source.mtime.
            }
        }
    }
```

- [ ] **Step 4: Hook into `App::tick`**

Find the existing `pub fn tick(&mut self)` (around line 528). It currently reads:

```rust
    pub fn tick(&mut self) {
        self.tick_no_summaries();
        self.drain_and_retry_summaries();
    }
```

Add the reload check at the end:

```rust
    pub fn tick(&mut self) {
        self.tick_no_summaries();
        self.drain_and_retry_summaries();
        self.check_for_theme_reload();
    }
```

(Do NOT call it from `tick_no_summaries` — headless `--json`/`--once` consumers should not pay the polling cost.)

- [ ] **Step 5: Verify the tests pass**

```bash
cargo test --lib --quiet app::theme_source_tests 2>&1 | tail -15
```

Expected: all 7 tests pass.

- [ ] **Step 6: Verify the full suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 254 passed (248 + 6 new — Step 1 of this Task contributes 6, Step 1 of Task 1 contributed 1, total 7 new across both tasks). Net: 247 + 7 = 254.

- [ ] **Step 7: Verify clean release build**

```bash
cargo build --release 2>&1 | tail -5
```

Expected: clean — no warnings. `theme_source` now has both a setter and a reader.

- [ ] **Step 8: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): poll theme file mtime in tick + reload on change

Adds check_for_theme_reload() called at the end of App::tick (not
tick_no_summaries, so --json/--once don't pay the poll cost). Reads
the file via theme::load_from_path_with_errors, preserves the
theme's current name, surfaces a 'reloaded' status (or 'reloaded
with N parse errors' if any). Handles missing file gracefully; a
file that disappears then reappears triggers a reload."
```

---

## Task 3: Compute and stash `ThemeSource` in `lib.rs::run()`

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Locate the existing `match &cli_theme_name` block**

In `src/lib.rs::run()`, find the existing block (post-B4) that resolves `initial_theme` and `parse_errors`. It looks like:

```rust
let (initial_theme, parse_errors): (theme::Theme, Vec<theme::ParseError>) =
    match &cli_theme_name {
        Some(arg) if is_theme_path_arg(arg) => { ... }
        Some(name) => { ... }
        None => theme::load_or_default_with_errors(&cfg.theme, &cfg),
    };
```

- [ ] **Step 2: Compute the optional `ThemeSource` right after**

Immediately AFTER the closing `};` of that match, insert:

```rust
// Compute the source-of-truth path for runtime reloads (B5). None for
// embedded-only themes; Some(path, mtime) for user-dir files and
// --theme <path> args.
let theme_source: Option<crate::app::ThemeSource> = match &cli_theme_name {
    Some(arg) if is_theme_path_arg(arg) => {
        let path = expand_tilde(arg);
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Some(crate::app::ThemeSource { path, mtime })
    }
    Some(name) => {
        let user_path = config::xdg_config_dir()
            .join("abtop")
            .join("themes")
            .join(format!("{name}.theme"));
        if user_path.exists() {
            let mtime = std::fs::metadata(&user_path)
                .and_then(|m| m.modified())
                .ok();
            Some(crate::app::ThemeSource {
                path: user_path,
                mtime,
            })
        } else {
            None
        }
    }
    None => {
        let user_path = config::xdg_config_dir()
            .join("abtop")
            .join("themes")
            .join(format!("{}.theme", cfg.theme));
        if user_path.exists() {
            let mtime = std::fs::metadata(&user_path)
                .and_then(|m| m.modified())
                .ok();
            Some(crate::app::ThemeSource {
                path: user_path,
                mtime,
            })
        } else {
            None
        }
    }
};
```

- [ ] **Step 3: Find all `build_app` callsites and the `run_app` body**

Search:

```bash
rg -n 'build_app\(|fn run_app|set_parse_error_status' src/lib.rs
```

You will find:
1. The `--json` branch: `let mut app = build_app(initial_theme.clone(), &cfg);` followed by `set_parse_error_status(&mut app, &parse_errors);`.
2. The `--once` branch: same pattern.
3. The TUI startup: `let app_result = run_app(&mut terminal, demo_mode, initial_theme, exit_on_jump, &cfg.hidden_agents, cfg.panels, &cfg.claude_config_dirs, &parse_errors);`.
4. `fn run_app(... parse_errors: &[theme::ParseError]) -> io::Result<()>`: builds the App internally, calls `set_parse_error_status`.

- [ ] **Step 4: Wire the source into `--json` and `--once` branches**

After each existing `set_parse_error_status(&mut app, &parse_errors);` line in BOTH the `--json` and `--once` branches, add:

```rust
        app.set_theme_source(theme_source.clone());
```

(Indentation matches the surrounding code in each branch.)

- [ ] **Step 5: Thread the source into `run_app`**

Update the existing `run_app` call (around line 308 area):

```rust
let app_result = run_app(
    &mut terminal,
    demo_mode,
    initial_theme,
    exit_on_jump,
    &cfg.hidden_agents,
    cfg.panels,
    &cfg.claude_config_dirs,
    &parse_errors,
    theme_source.clone(),
);
```

(Note the new `theme_source.clone()` arg at the end.)

Update the `run_app` function signature (around line 327) to accept the new parameter:

```rust
fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    demo_mode: bool,
    initial_theme: theme::Theme,
    exit_on_jump: bool,
    hidden_agents: &[String],
    panels: config::PanelVisibility,
    claude_config_dirs: &[std::path::PathBuf],
    parse_errors: &[theme::ParseError],
    theme_source: Option<crate::app::ThemeSource>,
) -> io::Result<()> {
```

In the body of `run_app`, after `set_parse_error_status(&mut app, parse_errors);`, add:

```rust
    app.set_theme_source(theme_source);
```

(No `.clone()` here — `run_app` owns the value via the function arg.)

- [ ] **Step 6: Verify the build is clean**

```bash
cargo build --release 2>&1 | tail -5
```

Expected: clean release build, no warnings.

- [ ] **Step 7: Verify the full test suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 254 passed.

- [ ] **Step 8: Commit**

```bash
git add src/lib.rs
git commit -m "feat(cli): wire ThemeSource through run() into App for live reload

Computes the source file (xdg user-dir or --theme path) at startup,
captures initial mtime, and threads the optional ThemeSource through
build_app + run_app so App::check_for_theme_reload has something to
poll. Embedded-only themes get None (no polling)."
```

---

## Task 4: Build, install, and smoke test

**Files:** none (build + install + manual smoke)

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 254 passed.

- [ ] **Step 2: Build release**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: `Finished` with no errors and no warnings.

- [ ] **Step 3: Install**

```bash
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

- [ ] **Step 4: Smoke test — embedded theme, no polling (negative case)**

```bash
~/.local/libexec/abtop --theme catppuccin --once 2>&1 | head -2
echo "exit: $?"
```

Expected: snapshot, exit 0. No `theme_source` was set because embedded themes have no source file — this should not panic and behave identically to pre-B5.

Note: if you have `~/.config/abtop/themes/catppuccin.theme` from earlier B1/B3 smoke testing, this WILL set a source (user override). Remove the file first to test the embedded-only path:

```bash
rm -f ~/.config/abtop/themes/catppuccin.theme
~/.local/libexec/abtop --theme catppuccin --once 2>&1 | head -2
```

- [ ] **Step 5: Smoke test — interactive reload of a user-dir theme**

```bash
mkdir -p ~/.config/abtop/themes
cp themes/dracula.theme ~/.config/abtop/themes/scratch.theme
~/.local/libexec/abtop --theme scratch &
ABTOP_PID=$!
sleep 3
```

In another shell (or as a background command in this one):

```bash
# Modify the file in place
sed -i.bak 's/#ff79c6/#abcdef/' ~/.config/abtop/themes/scratch.theme
# Wait for the next tick (~2-3s)
sleep 4
```

Switch back to the abtop TUI; the colors should have updated, with a footer status message `theme 'scratch' reloaded`. Press `q` to quit.

Skip the interactive part if you don't have a TTY available right now — unit tests already prove the wiring; the smoke test is just the end-to-end visual confirmation.

- [ ] **Step 6: Smoke test — `--theme <path>` reload**

```bash
cp themes/dracula.theme /tmp/scratch.theme
~/.local/libexec/abtop --theme /tmp/scratch.theme &
sleep 3
sed -i.bak 's/#ff79c6/#abcdef/' /tmp/scratch.theme
sleep 4
```

Same expectation as Step 5. Press `q` to quit.

- [ ] **Step 7: Smoke test — file deletion doesn't crash**

```bash
cp themes/dracula.theme /tmp/scratch.theme
~/.local/libexec/abtop --theme /tmp/scratch.theme &
sleep 3
rm /tmp/scratch.theme
sleep 4
# abtop should still be running with the old theme
kill %1 2>/dev/null
```

Expected: abtop keeps running with the in-memory theme.

- [ ] **Step 8: Clean up**

```bash
rm -f ~/.config/abtop/themes/scratch.theme /tmp/scratch.theme /tmp/scratch.theme.bak ~/.config/abtop/themes/scratch.theme.bak
```

- [ ] **Step 9: No commit needed — install is a side effect.**

---

## Acceptance criteria

1. `cargo test --lib` passes (254 tests; was 247 at B5 start, +7 new).
2. `cargo build --release` clean with no warnings.
3. Editing the active user-dir theme file while abtop runs causes a reload within ~3 seconds, with a footer status message confirming.
4. Editing the file passed via `--theme <path>` similarly triggers reload.
5. Deleting the file mid-session does not panic; abtop continues with the in-memory theme. Re-creating the file with new content triggers a reload on the next tick.
6. Reload preserves `self.theme.name`.
7. Embedded-only themes (no source file at startup) do not trigger any polling.
8. `--json` and `--once` (which use `tick_no_summaries` directly) do not pay the polling cost.
9. No new dependencies in `Cargo.toml`.
10. No new `pub` items — `ThemeSource`, `set_theme_source`, `check_for_theme_reload` are all `pub(crate)` or private.

## Out of scope (future specs)

- B5b: themes-dir watch — new files mid-session join `cycle_names`.
- B5c: config.toml watch — `theme_background` toggle takes effect live.
- B6: macOS Library → XDG migration — deferred indefinitely.
