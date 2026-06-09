# abtop Phase A — Theme files & transparency: tracking

Resume pointer for the work specified in
[`specs/2026-06-09-theme-files-and-transparency-design.md`](specs/2026-06-09-theme-files-and-transparency-design.md)
and planned in
[`plans/2026-06-09-theme-files-and-transparency.md`](plans/2026-06-09-theme-files-and-transparency.md).

## Status: Phase A complete

Last updated: 2026-06-09. Binary installed at `~/.local/libexec/abtop`.

### Phase A progress (14 tasks)

| # | Task | Status | Commit |
|---|---|---|---|
| 0 | Reset working tree (revert exploratory catppuccin patch) | done | — |
| 1 | Add `xdg_config_dir()` helper to config.rs | done | `e32c20e` + `2ae2a21` (doc fixup) |
| 2 | Switch `config_path()` to use `xdg_config_dir()` | done | `15de9eb` + `c9f388c` (test tightening) |
| 3 | Add `theme_background` field to `AppConfig` | done | `48a236c` |
| 4 | Split `src/theme.rs` into `src/theme/` module | done | `6d886e0` + `8cc1929` (cleanup) |
| 5 | Add `parse_hex` to `src/theme/loader.rs` | done | `05e7007` |
| 6 | Add `parse_theme_body` to the loader | done | `8f8307b` |
| 7 | Add `apply_overrides` | done | `d83dd89` |
| 8 | Embed `btop` theme as parity sentinel | done | `37e9a15` |
| 9 | Embed the remaining 11 themes | done | `4ffed1a` |
| 10 | Add `Theme::load_or_default` and update `by_name` | done | `df4243d` |
| 11 | Wire startup and `cycle_theme` to the new chain | done | `54d0202` |
| 12 | Delete the 12 Rust constructors | done | `d43bfc3` |
| 13 | Update README | done | `a2d2acc` |
| 14 | Build release and install to `~/.local/libexec/abtop` | done | (no commit — install side effect) |

## Acceptance criteria (verified)

- ✅ `cargo test --lib` is green (202 tests).
- ✅ `cargo build --release` produces a working binary.
- ✅ `abtop --once` runs without panicking.
- ✅ Three smoke tests pass:
  - Default snapshot output.
  - `theme = "catppuccin"` + `theme_background = false` in `~/.config/abtop/config.toml`.
  - User-defined `~/.config/abtop/themes/loud.theme` resolved via `--theme loud`.
- ✅ Binary installed at `~/.local/libexec/abtop` (on PATH via env-osx.zsh).

## Visual confirmation pending

The end-to-end transparency check requires running the interactive TUI in a terminal that has transparency configured. The CLI smoke tests don't exercise that visual path. Run `abtop` in a transparent-bg terminal (Alacritty / Ghostty / iTerm2 with transparency on) to confirm the catppuccin background shows through.

## Phase B (deferred — separate spec)

- `t`-cycle merges user-dir themes with embedded set.
- Banner in UI on malformed theme file.
- `abtop --theme <absolute-path>`.
- `abtop --list-themes`, `abtop --dump-theme <name>`.
- Reload-on-file-change.
- macOS `~/Library/Application Support/abtop/config.toml` → XDG migration.

## Minor known issues (non-blocking)

- Spec text section "Theme file format" says "39 keys total (24 + 15)"; the actual key count is 33 (12 base + 2 semantic + 4 box-border + 15 gradient channels). Implementation matches reality at 33; the prose drift is in the spec doc only.
