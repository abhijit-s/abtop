# Phase B5 — Reload active theme on file change — design

**Status:** Approved (local fork)
**Date:** 2026-06-09
**Scope:** Phase B item 5, minimal variant. Detect mtime changes on the active theme file and reload it inside the existing tick loop.
**Target:** `~/my-workspace/util/abtop` (local fork at `37726c8`). No upstream PR.
**Builds on:** Phase A + B1 + B2 + B3 + B4 — `theme::load_or_default_with_errors`, `theme::load_from_path_with_errors`, `theme::apply_overrides`, `App::set_status`, `App::tick`.

## Goal

When the user edits the file that backs the active theme — either `$XDG_CONFIG_HOME/abtop/themes/<active>.theme` or the file passed via `--theme <path>` — abtop notices on the next tick (~2 seconds) and re-renders with the new colors. No abtop restart needed. Standard iteration loop: edit → save → look at abtop.

## Non-goals

- **No `notify` crate.** mtime polling inside the existing `App::tick` is enough; new latency budget is one tick (~2s), which is fine for theme iteration. Avoids a 30k-LOC indirect dep tree and cross-platform watcher quirks.
- **No themes-dir watch.** New files in `~/.config/abtop/themes/` still don't join the `t`-cycle until restart. Tracked as a future spec (B5b).
- **No config.toml watch.** Toggling `theme_background` still needs restart. Tracked as a future spec (B5c).
- **No manual reload key binding.** Polling is fast enough that an explicit gesture isn't needed.
- **No reload for embedded-only themes** (no source file to watch). Pressing `t` to cycle and landing back on the same name still works as today.

## Behavior contract

1. At startup, if the resolved theme has a backing file, record `(path, mtime)`. If the theme came from `embedded::BUILTIN` (no user override), the source field is `None` and no polling happens.
2. On every `App::tick`, if the source is `Some`:
   - `stat` the path; on read failure (file deleted, permission denied) **silently leave the theme as-is** and clear the stored mtime so a re-creation triggers a fresh reload.
   - If mtime is newer than stored mtime: re-read the file, parse, apply overrides, swap `self.theme`, update stored mtime, post status message.
3. Status message on successful reload:
   - Zero parse errors: `theme '<name>' reloaded`.
   - Non-zero parse errors: `theme '<name>' reloaded with N parse error(s)` (B4 wording, with "reloaded" prefix).
4. mtime poll cost: one `fs::metadata` call per tick when a source is set. Negligible.

## Architecture

```
                          startup
                             │
                             ▼
                       lib.rs::run()
                             │
            ┌────────────────┴────────────────┐
            │                                 │
   --theme <path>?                  name lookup
   compute source                  compute source
   = Some(path, mtime)             = Some(path, mtime) IFF user-dir file
                                   = None for embedded
            │                                 │
            └────────────────┬────────────────┘
                             ▼
                  build_app(theme, &cfg)
                             │
                             ▼
                  app.set_theme_source(source)
                             │
                             ▼
                  enter event loop
                             │
                             ▼
                  App::tick (every ~2s)
                             │
                             ▼
                  check_for_theme_reload(&mut self)
                             │
            ┌────────────────┴────────────────┐
            │                                 │
       mtime same                     mtime newer
            │                                 │
            ▼                                 ▼
       (no-op)                       read + parse + apply_overrides
                                      swap self.theme
                                      update stored mtime
                                      set_status("…reloaded…")
```

## Code changes

### New type in `src/app.rs`

```rust
/// Where the active theme came from on disk. Used by App::tick to detect
/// mid-session edits and reload. None for embedded themes (no source).
pub(crate) struct ThemeSource {
    /// Absolute path to the .theme file.
    pub path: std::path::PathBuf,
    /// Modification time at the last successful read, or None if the file
    /// was missing at the last check. A returned-to-existence path with a
    /// fresh mtime triggers a reload.
    pub mtime: Option<std::time::SystemTime>,
}
```

### New field on `App`

```rust
pub struct App {
    // ... existing fields ...

    /// Optional source for the active theme. Some(...) for user-dir files
    /// and --theme <path>; None for embedded themes. Polled in App::tick.
    theme_source: Option<ThemeSource>,
}
```

Initialized to `None` in `App::new_with_config_and_claude_dirs`'s struct literal — matching the B2 cycle_names pattern (existing tests don't touch it; production wires it via a setter from `lib.rs`).

### New setter on `App`

```rust
impl App {
    pub(crate) fn set_theme_source(&mut self, source: Option<ThemeSource>) {
        self.theme_source = source;
    }
}
```

### New tick helper on `App`

```rust
impl App {
    /// Re-stat the active theme's source file and reload if mtime moved
    /// forward. Called from App::tick. No-op for embedded-only themes.
    fn check_for_theme_reload(&mut self) {
        let Some(source) = self.theme_source.as_mut() else {
            return;
        };
        let current_mtime = std::fs::metadata(&source.path)
            .and_then(|m| m.modified())
            .ok();
        let needs_reload = match (current_mtime, source.mtime) {
            (Some(now), Some(then)) => now > then,
            (Some(_), None) => true,   // file came back; treat as new
            (None, _) => false,        // file missing; keep current theme
        };
        if !needs_reload {
            // Track the missing-state so re-creation re-triggers.
            source.mtime = current_mtime;
            return;
        }
        // Read + parse + apply overrides.
        let cfg = crate::config::load_config();
        match crate::theme::load_from_path_with_errors(&source.path) {
            Ok((mut new_theme, errors)) => {
                crate::theme::apply_overrides(&mut new_theme, &cfg);
                // Preserve the theme name from the prior theme so cycle
                // index lookups stay sane (file_stem may not match the
                // saved name for user-dir themes resolved by name).
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
                // Transient read error (partial write, etc.). Try again next tick.
            }
        }
    }
}
```

Why preserve `self.theme.name` instead of using `path.file_stem()`?
- `lib.rs::run` may have resolved the theme by name (`--theme catppuccin`) and the file is `~/.config/abtop/themes/catppuccin.theme`. The path's stem and the saved name happen to match.
- But `--theme /tmp/scratch.theme` derives name from file_stem = "scratch". If user pressed `t` after launch and landed on "scratch", that's the saved name.
- Either way, preserving the existing name avoids surprise renaming after reload. Simplest invariant.

### `App::tick` integration

Find the existing `pub fn tick(&mut self)` in `src/app.rs`. Add the reload check at the END of the function body (after existing collector polling, summaries, etc.):

```rust
pub fn tick(&mut self) {
    // ... existing tick body unchanged ...

    self.check_for_theme_reload();
}
```

This way an unhandled panic in `check_for_theme_reload` wouldn't break existing telemetry collection. Reload is purely additive.

### `src/lib.rs` — compute and stash source after `build_app`

In `lib.rs::run()`, between resolving `initial_theme` (the existing `match &cli_theme_name { … }` block) and the first `build_app` call, compute the source:

```rust
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
            let mtime = std::fs::metadata(&user_path).and_then(|m| m.modified()).ok();
            Some(crate::app::ThemeSource { path: user_path, mtime })
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
            let mtime = std::fs::metadata(&user_path).and_then(|m| m.modified()).ok();
            Some(crate::app::ThemeSource { path: user_path, mtime })
        } else {
            None
        }
    }
};
```

After each of the three `build_app` callsites (and inside `run_app`), call:

```rust
app.set_theme_source(theme_source.clone());
```

The `.clone()` is needed because there are three callsites. `ThemeSource` derives `Clone`.

### `src/app.rs` — re-export `ThemeSource` and the setter

`ThemeSource` is `pub(crate)` so `lib.rs` can construct it. The new setter is `pub(crate)`.

## Public surface change

- `app::ThemeSource` — new struct, `pub(crate)`.
- `App.theme_source` — new private field.
- `App::set_theme_source(Option<ThemeSource>)` — new `pub(crate)` setter.
- `App::check_for_theme_reload` — new private method.

Nothing new in `theme::` or anywhere else. `notify` crate NOT added to `Cargo.toml`.

## Test surface

In `src/app.rs` (new `#[cfg(test)] mod reload_tests` or appended to an existing one):

- `theme_source_none_does_not_panic_on_tick` — App without `set_theme_source` ticks normally; no theme change.
- `tick_reloads_when_mtime_advances` — write file, set_theme_source with old mtime, write file with different content, tick (or call `check_for_theme_reload` directly to bypass collector noise), assert theme.main_bg matches the new content.
- `tick_does_not_reload_when_mtime_unchanged` — same setup but don't touch the file between ticks; assert theme.main_bg is the original.
- `tick_handles_missing_file_gracefully` — set_theme_source pointing to a path; delete the file; tick; assert no panic and theme unchanged.
- `tick_reloads_after_file_recreates` — file present at startup → deleted → recreated. After recreation, next tick should reload.
- `reload_preserves_theme_name` — start from "catppuccin"; edit file; reload; assert `theme.name == "catppuccin"` (file_stem-derived name not used).
- `reload_with_parse_errors_sets_appropriate_status` — write a file with one bad hex; reload; assert status message contains "1 parse error".

These tests should call `check_for_theme_reload` directly (not full `tick`) to avoid pulling in the entire collector pipeline. `check_for_theme_reload` is a `fn` (not pub) — tests living in the same module can call it. If made `pub(crate)` they can also live in an integration test, but the inline module approach is cheaper.

Manual smoke:

```sh
# Drop a user theme
cp themes/dracula.theme ~/.config/abtop/themes/scratch.theme
abtop --theme scratch &
# In another shell: edit ~/.config/abtop/themes/scratch.theme
# Within ~2-3 seconds, abtop should re-render with the new colors
# and show "theme 'scratch' reloaded" in the footer for 3s.
fg
# press q to quit
rm ~/.config/abtop/themes/scratch.theme
```

## Error handling philosophy

Matches Phase A's infallibility:
- Read failure on poll → no-op; try next tick.
- Parse errors → reload still happens, banner shows the count (reuses B4).
- mtime call fails → treat as "missing file" (no reload, no panic).
- The `notify` crate-free design means no async, no channels, no threads — just synchronous polling. Predictable.

## Performance considerations

- One `fs::metadata` call per tick when a source exists. ~5µs on local fs.
- One file read + parse on every detected change. The file is small (typically <4KB). ~50µs.
- No memory growth: `theme_source` is bounded-size; reload swaps `Theme` in place.

## Build & install

```sh
cd ~/my-workspace/util/abtop
cargo test --lib
cargo build --release
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

## Acceptance criteria

1. `cargo test --lib` passes (test count grows by ~7 new tests).
2. `cargo build --release` clean with no warnings.
3. Editing the active user-dir theme file while abtop runs causes a reload within ~3 seconds, with a footer status message confirming.
4. Editing the file passed via `--theme <path>` similarly triggers reload.
5. Deleting the file mid-session does not panic; abtop continues with the in-memory theme. Re-creating the file with new content triggers a reload.
6. Reload preserves `self.theme.name` (no rename surprise after editing).
7. Embedded-only themes (no source file at startup) do not trigger any polling.
8. No new dependencies in `Cargo.toml`.

## Out of scope (future specs)

- B5b: themes-dir watch — new files mid-session join `cycle_names`.
- B5c: config.toml watch — `theme_background` toggle takes effect live.
- B6: macOS Library → XDG migration — deferred indefinitely.
