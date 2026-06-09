# abtop theming work: tracking

Resume pointer for theming + transparency work on the local abtop fork.

## Status: Phase A done + catppuccin-transparent variant + Phase B1 done

Last updated: 2026-06-09. Binary installed at `~/.local/libexec/abtop`.

### Phases shipped

| Phase | Spec | Plan | Outcome |
|---|---|---|---|
| A — theme files + transparency | [`specs/2026-06-09-theme-files-and-transparency-design.md`](specs/2026-06-09-theme-files-and-transparency-design.md) | [`plans/2026-06-09-theme-files-and-transparency.md`](plans/2026-06-09-theme-files-and-transparency.md) | 14 tasks + catppuccin-transparent variant; lib tests 0 → 203 |
| B1 — `--list-themes` / `--dump-theme` | [`specs/2026-06-09-list-and-dump-theme-design.md`](specs/2026-06-09-list-and-dump-theme-design.md) | [`plans/2026-06-09-list-and-dump-theme.md`](plans/2026-06-09-list-and-dump-theme.md) | 8 tasks; lib tests 203 → 214 |
| B2 — t-cycle picks up user themes | [`specs/2026-06-09-t-cycle-user-themes-design.md`](specs/2026-06-09-t-cycle-user-themes-design.md) | [`plans/2026-06-09-t-cycle-user-themes.md`](plans/2026-06-09-t-cycle-user-themes.md) | 3 tasks + review fix; lib tests 214 → 217 |
| B3 — `--theme <path>` accepts file paths | [`specs/2026-06-09-theme-absolute-path-design.md`](specs/2026-06-09-theme-absolute-path-design.md) | [`plans/2026-06-09-theme-absolute-path.md`](plans/2026-06-09-theme-absolute-path.md) | 5 tasks; lib tests 217 → 225 |

13 embedded themes now ship: the original 12 plus `catppuccin-transparent` (catppuccin with `main_bg=""`), added post-Phase A as a baked-in convenience variant. Available via `--theme catppuccin-transparent` or `theme = "catppuccin-transparent"` in config.toml, without needing the `theme_background = false` flag.

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
| 15 | Add `catppuccin-transparent` embedded variant (post-Phase A) | done | `13a13a5` |

### Phase B1 progress (8 tasks)

| # | Task | Status | Commit |
|---|---|---|---|
| 1 | `Source` enum + `ThemeListing` struct in `loader.rs` | done | `018ecc1` |
| 2 | `list_available()` function | done | `e7d6775` |
| 3 | `dump_embedded()` function | done | `4d7ed05` |
| 4 | Re-export new symbols from `theme/mod.rs` | done | `336b35b` |
| 5 | `--list-themes` early-return handler in `lib.rs` | done | `11a28d7` |
| 6 | `--dump-theme [--force]` early-return handler | done | `a084325` |
| 7 | README docs ("Discovering and editing themes" subsection) | done | `a67e7f2` |
| 8 | Build + install + smoke | done | (no commit — install side effect) |

### Phase B2 progress (3 tasks)

| # | Task | Status | Commit |
|---|---|---|---|
| 1 | App `cycle_names` field + `set_cycle_names` setter + `cycle_theme` rewrite + 2 TDD tests | done | `09a357f` |
| 2 | `build_app` populates `cycle_names` from `list_available`; remove temporary `#[allow(dead_code)]` | done | `ee840a9` |
| 3 | Build + install + smoke | done | (no commit — install side effect) |

## Acceptance criteria (verified)

### Phase A
- ✅ `cargo test --lib` is green (202 tests; now 214 after B1).
- ✅ `cargo build --release` produces a working binary.
- ✅ `abtop --once` runs without panicking.
- ✅ Four smoke tests pass:
  - Default snapshot output.
  - `theme = "catppuccin"` + `theme_background = false` in `~/.config/abtop/config.toml`.
  - User-defined `~/.config/abtop/themes/loud.theme` resolved via `--theme loud`.
  - `--theme catppuccin-transparent` resolves the embedded variant.
- ✅ Binary installed at `~/.local/libexec/abtop` (on PATH via env-osx.zsh).

### Phase B1
- ✅ `cargo test --lib` is green (214 tests; was 203 at B1 start, +11 new).
- ✅ `abtop --list-themes` prints 13 lines all `(built-in)` with no user-dir themes; shows `(user override)` / `(user)` as appropriate.
- ✅ `abtop --dump-theme catppuccin` writes the embedded body to `~/.config/abtop/themes/catppuccin.theme`.
- ✅ Re-run without `--force` errors with exit 1 and the expected stderr message.
- ✅ `--force` overwrites cleanly with exit 0.
- ✅ `--dump-theme nonexistent` errors with exit 1 and lists available embedded themes.
- ✅ `--dump-theme ../evil` rejected before any filesystem touch; exit 1.
- ✅ After cleanup of the dumped file, `--list-themes` reverts the entry from `(user override)` to `(built-in)`.

## Visual confirmation pending

The end-to-end transparency check requires running the interactive TUI in a terminal that has transparency configured. The CLI smoke tests don't exercise that visual path. Run `abtop` in a transparent-bg terminal (Alacritty / Ghostty / iTerm2 with transparency on) to confirm the catppuccin background shows through.

## Phase B remaining items (deferred — separate spec each)

B1 (`--list-themes` / `--dump-theme`), B2 (`t`-cycle picks up user themes), and B3 (`--theme <path>`) shipped. Still open:


- B4: Banner in UI on malformed theme file.
- B5: Reload-on-file-change.
- B6: macOS `~/Library/Application Support/abtop/config.toml` → XDG migration (skip unless we ever share this fork).

## Final whole-Phase-B1 review

Ran a single subagent over the full B1 diff to catch cross-cutting issues. Verdict: approved with one Important tightening, landed in commit `03a2756`:

- `dump_embedded`, `list_available`, `apply_overrides`, `load_or_default`, `Source` narrowed from `pub use` to `pub(crate) use` in `theme/mod.rs`. The crates.io-visible API stays as just `Theme`, `Gradient`, `Theme::by_name` — the B1 internals are wiring details, not stable library surface. `ThemeListing` dropped from the re-export entirely since no in-crate consumer names it (return-type inference covers `lib.rs::run()`'s only usage).

Four other findings were minor/informational and intentionally skipped:
- `dump_embedded` returning `Result<PathBuf, String>` — resolved by the visibility tightening (internal API, prose-string return is fine when crate-internal).
- `list_available` allocates two HashSets per call — not a hot path; reviewer themselves said skip.
- `loader.rs` is now 820 lines covering parse/load/list/dump — defensible at this size; refactor only when another concern arrives.
- `dump_embedded` write is not atomic (TOCTOU between exists-check and write) — interactive single-user invocation; revisit if dumping ever becomes non-interactive.

214 lib tests still passing after the change.

## Final whole-Phase-A review

Ran a single subagent over the full diff `a19ac65..b27f4e1` to catch cross-cutting issues per-task reviews could miss. Verdict: approved with four small fixes, all landed in commit `9e6e346`:

1. `parse_theme_body` dropped from public re-export — kept the crate API to `apply_overrides` + `load_or_default` only (avoids implicit semver commitment on the parser).
2. Path-traversal guard added to `try_user_file` — `--theme '../../etc/passwd'` and friends now silently return `None`. New test `lookup_chain_rejects_path_traversal_names` covers `..`, `/`, `\`, and empty.
3. README bumped from "12 themes" to "13 themes" in both spots and `catppuccin-transparent` added to the list with a one-liner on what it is.
4. README clarifies that empty gradient channels fall back to btop (no `Color::Reset` for tuples).

203 lib tests passing after the change (was 202 + the new traversal test).

Spec doc cleanup (`b27f4e1`): "39 keys" prose corrected to "33 keys (12+2+4+15)" so it matches the implementation. Was the one known non-blocking drift; now resolved.
