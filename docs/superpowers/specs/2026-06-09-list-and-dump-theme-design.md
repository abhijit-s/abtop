# Phase B1 — `--list-themes` and `--dump-theme` CLI flags — design

**Status:** Approved (local fork)
**Date:** 2026-06-09
**Scope:** Phase B item 1. Two new early-return CLI flags. No parser changes, no architectural changes.
**Target:** `~/my-workspace/util/abtop` (local fork of `graykode/abtop`, currently on commit `180bde3`). No upstream PR.
**Builds on:** Phase A (commit `180bde3`) — embedded `themes/*.theme` files, `theme::loader::lookup_chain`, `theme::embedded::BUILTIN`, `config::xdg_config_dir()`.

## Goal

Add two ergonomic CLI flags that improve discoverability and editability of themes without touching the resolution chain or parser:

- `abtop --list-themes` — print all available theme names (embedded + user-dir) with a marker showing source.
- `abtop --dump-theme <name>` — write the embedded body of `<name>` to `$XDG_CONFIG_HOME/abtop/themes/<name>.theme` so the user can edit it in place without copying from the source tree.

## Non-goals

- No new theme resolution logic — uses existing `embedded::BUILTIN` and `xdg_config_dir()`.
- No interactive prompts (clean scripting story).
- No JSON output mode (could be added later if needed; plain text covers the use case).
- No support for dumping user-only themes (they're already on disk; nothing to dump).
- No changes to the `t`-cycle behavior in this spec (Phase B item 2).
- No live reload or file watching (Phase B item 5).

## Architecture

Both flags are early-return handlers in `src/lib.rs::run()`, mirroring the existing `--version` / `--update` / `--setup` pattern. They short-circuit before config load and app build.

The listing logic — "what themes exist and where do they come from" — lives in `src/theme/loader.rs` as a pure function. The CLI handlers in `lib.rs` are thin formatters/writers on top.

```
                        abtop --list-themes
                                 │
                                 ▼
                   lib.rs::run() early-return
                                 │
                                 ▼
            theme::loader::list_available(&xdg_config_dir())
                                 │
                                 ▼
                       Vec<ThemeListing>
                                 │
                                 ▼
                  format + println per line; exit 0


                  abtop --dump-theme <name> [--force]
                                 │
                                 ▼
                   lib.rs::run() early-return
                                 │
                                 ▼
              embedded::lookup(<name>) → Some(body) | None
                                 │
                  ┌──────────────┴──────────────┐
                  ▼                             ▼
            error: not embedded         <xdg>/abtop/themes/<name>.theme
            exit 1                              │
                                                ▼
                                  exists? + no --force? → error exit 1
                                                │
                                                ▼
                                   write body; print path; exit 0
```

## CLI surface

### `--list-themes`

**Synopsis:**
```
abtop --list-themes
```

**Output (stdout):** one entry per line. Each line is `<name> (<source>)` where `<source>` is one of:

- `built-in` — name is in `embedded::BUILTIN` and no user file shadows it.
- `user override` — name is in `embedded::BUILTIN` AND a user file at `$XDG_CONFIG_HOME/abtop/themes/<name>.theme` exists.
- `user` — name is NOT in `embedded::BUILTIN`; only a user file exists.

**Order:**
1. All `BUILTIN` entries in declaration order (`btop`, `dracula`, `catppuccin`, `catppuccin-transparent`, `tokyo-night`, `gruvbox`, `nord`, `light`, `white`, `high-contrast`, `protanopia`, `deuteranopia`, `tritanopia`). Each is `(built-in)` or `(user override)`.
2. User-only themes (those NOT in `BUILTIN`), sorted alphabetically. Each is `(user)`.

**Example:**
```
$ abtop --list-themes
btop (built-in)
dracula (built-in)
catppuccin (user override)
catppuccin-transparent (built-in)
tokyo-night (built-in)
gruvbox (built-in)
nord (built-in)
light (built-in)
white (built-in)
high-contrast (built-in)
protanopia (built-in)
deuteranopia (built-in)
tritanopia (built-in)
my-cool (user)
zorak (user)
```

**Exit code:** always 0 if the flag is recognized. If listing user-dir files fails (permissions, symlink loop), the user-only section is skipped and exit is still 0 — embedded list is unconditionally available.

**No additional flags.** `--list-themes --json` and similar are explicit non-goals.

### `--dump-theme <name> [--force]`

**Synopsis:**
```
abtop --dump-theme <name>
abtop --dump-theme <name> --force
```

**Behavior:**

1. Look up `<name>` in `embedded::BUILTIN`. If not present → error and exit 1.
2. Compute target path: `xdg_config_dir().join("abtop").join("themes").join(format!("{name}.theme"))`.
3. If the target file already exists AND `--force` is NOT set → error and exit 1.
4. Create the parent directory if it doesn't exist (`std::fs::create_dir_all`).
5. Write the embedded body to the target path.
6. Print `wrote <path>` to stdout and exit 0.

**Path-traversal hardening:** `<name>` must NOT contain `/`, `\`, or `..` (same guard as `try_user_file` from Phase A's final-review fix). Rejected names error before any filesystem touch.

**Behavior table:**

| Invocation | Embedded? | Target exists? | --force? | Action | Exit |
|---|---|---|---|---|---|
| `--dump-theme catppuccin` | yes | no | no | write, print path | 0 |
| `--dump-theme catppuccin` | yes | yes | no | error: file exists, suggest --force | 1 |
| `--dump-theme catppuccin --force` | yes | yes | yes | overwrite, print path | 0 |
| `--dump-theme catppuccin --force` | yes | no | yes | write, print path | 0 |
| `--dump-theme nonexistent` | no | — | — | error: not embedded | 1 |
| `--dump-theme my-cool` (user-only) | no | — | — | error: not embedded; nothing to dump | 1 |
| `--dump-theme ../evil` | — | — | — | error: invalid theme name | 1 |
| `--dump-theme` (no arg) | — | — | — | error: requires a theme name | 1 |

**Error messages (stderr):**

- Missing arg: `--dump-theme requires a theme name`, then `available: <comma-joined embedded names>` on the next line (same format as `--theme`'s existing missing-arg error).
- Unknown embedded: `'<name>' is not an embedded theme; nothing to dump`, then `available: <comma-joined embedded names>` on the next line.
- File exists: `<absolute-path> already exists. Re-run with --force to overwrite.`
- Invalid name: `invalid theme name '<name>': contains '/', '\\', or '..'`.

**Success message (stdout):** `wrote <absolute-path>`.

## Module changes

```
src/theme/loader.rs
├── new enum Source { Builtin, User, UserOverride }
├── new struct ThemeListing { name: String, source: Source }
└── new pub fn list_available(config_root: &Path) -> Vec<ThemeListing>

src/theme/mod.rs
└── pub use loader::{apply_overrides, load_or_default, list_available, ThemeListing, Source};
    (extend the existing re-export line)

src/lib.rs
├── early-return: --list-themes → call list_available, format, println per line
└── early-return: --dump-theme <name> [--force] → embedded::lookup, write logic
```

### Public surface added

- `theme::Source` (enum)
- `theme::ThemeListing` (struct)
- `theme::list_available(&Path) -> Vec<ThemeListing>`

All three are needed externally by `lib.rs` and may be reused by Phase B item 2 (t-cycle of user themes).

## Implementation notes

- `list_available` reads the user themes dir via `std::fs::read_dir`. Errors (dir missing, permission denied) yield an empty user list — `list_available` is infallible so the embedded list is always returned.
- Filename → theme name: strip the `.theme` extension. Files without `.theme` are skipped.
- Hidden files (starting with `.`) are skipped to avoid editor swap files (`.catppuccin.theme.swp` from vim, etc.).
- The `--list-themes` formatter uses simple `println!("{} ({})", name, source_str)`. No padding, no colors.
- The `--dump-theme` flag-pair parsing mirrors `--theme <name>`: scan args for `--dump-theme`, take the next arg as the name; scan args separately for `--force` anywhere.

## Error handling philosophy

Matches Phase A's infallible-config philosophy where possible. `list_available` is infallible — it always returns at least the embedded list. `dump_theme` is fallible (filesystem I/O can fail) and reports clearly to stderr.

Both handlers exit immediately on completion or error. They never load `config.toml`, never build the `App`, never start the terminal. Safe to invoke from scripts and CI.

## Test surface

In `theme/loader.rs`:

- `list_available_empty_user_dir_returns_only_builtin` — tempdir with no `abtop/themes/`, asserts 13 entries all `Source::Builtin`.
- `list_available_user_only_themes_appended_alphabetically` — tempdir with two user-only `.theme` files (`zorak.theme`, `my-cool.theme`), asserts user entries appear after embedded, sorted as `my-cool`, `zorak`, both `Source::User`.
- `list_available_user_override_promotes_builtin_entry` — user file `catppuccin.theme` shadows the embedded entry; asserts the catppuccin entry is `Source::UserOverride` and appears in BUILTIN position (not appended).
- `list_available_skips_hidden_and_non_theme_files` — tempdir with `.catppuccin.theme.swp`, `notes.md`, `README` — none appear in output.
- `list_available_returns_builtin_when_user_dir_unreadable` — pass a path that doesn't exist; embedded list returned.

In `src/lib.rs` (or via integration test):

- `--dump-theme` end-to-end test using tempdir + `XDG_CONFIG_HOME` env var (or by carving out a testable helper that takes `config_root` as an arg).
- Refuse-on-exists path.
- `--force` overwrite path.
- Reject embedded-not-found.
- Reject path-traversal name.

## Build & install

Same as Phase A:

```sh
cd ~/my-workspace/util/abtop
cargo test --lib
cargo build --release
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

Smoke tests:

```sh
abtop --list-themes
# expect 13 (built-in) entries

abtop --dump-theme catppuccin
# expect: wrote ~/.config/abtop/themes/catppuccin.theme

abtop --dump-theme catppuccin
# expect error: file exists, suggest --force

abtop --dump-theme catppuccin --force
# expect: wrote ~/.config/abtop/themes/catppuccin.theme

abtop --list-themes | grep catppuccin
# expect: catppuccin (user override)
#         catppuccin-transparent (built-in)
```

## Out of scope (Phase B remaining items)

- `t`-cycle picks up user-dir themes (Phase B2 — separate spec).
- Banner UI on malformed theme file (Phase B4 — separate spec).
- `--theme <absolute-path>` (Phase B3 — separate spec).
- Reload-on-file-change (Phase B5 — separate spec).
- macOS `~/Library/Application Support/abtop/` → XDG migration (Phase B6 — separate spec, deferred indefinitely on this machine).
