# abtop Phase A — Theme files & transparency: tracking

Resume pointer for the work specified in
[`specs/2026-06-09-theme-files-and-transparency-design.md`](specs/2026-06-09-theme-files-and-transparency-design.md)
and planned in
[`plans/2026-06-09-theme-files-and-transparency.md`](plans/2026-06-09-theme-files-and-transparency.md).

## Status: planning complete, implementation not started

Last updated: 2026-06-09.

### Phase A progress (14 tasks)

| # | Task | Status |
|---|---|---|
| 0 | Reset working tree (revert exploratory catppuccin patch) | not started |
| 1 | Add `xdg_config_dir()` helper to config.rs | not started |
| 2 | Switch `config_path()` to use `xdg_config_dir()` | not started |
| 3 | Add `theme_background` field to `AppConfig` | not started |
| 4 | Split `src/theme.rs` into `src/theme/` module | not started |
| 5 | Add `parse_hex` to `src/theme/loader.rs` | not started |
| 6 | Add `parse_theme_body` to the loader | not started |
| 7 | Add `apply_overrides` | not started |
| 8 | Embed `btop` theme as parity sentinel | not started |
| 9 | Embed the remaining 11 themes | not started |
| 10 | Add `Theme::load_or_default` and update `by_name` | not started |
| 11 | Wire startup and `cycle_theme` to the new chain | not started |
| 12 | Delete the 12 Rust constructors | not started |
| 13 | Update README | not started |
| 14 | Build release and install to `~/.local/libexec/abtop` | not started |

Update this table as each task lands (check off the box and bump status).

## Working-tree state at handoff

- `src/theme.rs` has an uncommitted exploratory patch (catppuccin `main_bg → Color::Reset`). Task 0 reverts this — that's the first thing to do on resume.
- `target/release/abtop` may exist from a previous `cargo build`; harmless, will be overwritten by Task 14.
- Spec committed at `d0af5fe`, plan committed at `b771ebc`. Both on `main`.

## Decisions locked in during brainstorming

- **File format**: btop-compatible `theme[key]="#hex"` shell-array syntax. 39 keys per theme (24 colors + 5 gradients × 3 channels). Empty value = `Color::Reset` for `Color` fields; for gradient channels empty = fall back to embedded `btop` default.
- **Bundled strategy**: all 12 current themes become embedded `themes/*.theme` files included via `include_str!`. Rust constructors deleted in Task 12.
- **Transparency knob**: BOTH file-level (`theme[main_bg]=""`) and global config flag (`theme_background = false`). Global override wins.
- **Path resolution**: new `xdg_config_dir()` helper resolves `$XDG_CONFIG_HOME` → `$HOME/.config` → `.`. Applied uniformly to `config.toml` and `themes/`. On macOS this moves the config file from `~/Library/Application Support/abtop/` to `~/.config/abtop/` (no migration in Phase A — target machine has no existing config).
- **Two entry points**: `Theme::by_name(name) -> Option<Self>` (validation, no fallback) + `Theme::load_or_default(name, &cfg) -> Self` (startup, full chain). Spec section "Module shape" has details.
- **Scope**: Phase A only. Phase B follow-ups (t-cycle of user themes, banner UI on bad file, `--list-themes`, `--dump-theme`, reload-on-edit) listed at the bottom of the spec and plan.

## Resume from cold start

1. `cd /Users/a.salvi/my-workspace/util/abtop`
2. `git log --oneline -5` — should show `b771ebc docs: add implementation plan...` and `d0af5fe docs: add design spec...` on top.
3. Read this file for current task pointer, then open the plan and start at the next unchecked task.
4. Execution mode is undecided (subagent-driven vs inline `executing-plans`). Decide before starting Task 0.

## Why this work exists

User wants a transparent catppuccin background in abtop (so the terminal's own background shows through), mirroring how btop achieves it with `theme_background = false` in `btop.conf`. The proper fix is to externalize abtop's hardcoded Rust themes into `.theme` files + add the same config knob, rather than maintaining a one-off catppuccin patch in the local fork. The fork lives at `~/my-workspace/util/abtop` and stays local — no upstream PR.
