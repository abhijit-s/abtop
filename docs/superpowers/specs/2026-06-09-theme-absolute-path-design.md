# Phase B3 — `--theme <path>` accepts file paths — design

**Status:** Approved (local fork)
**Date:** 2026-06-09
**Scope:** Phase B item 3. Extend `--theme <arg>` to accept a path to a `.theme` file alongside the existing basename lookup.
**Target:** `~/my-workspace/util/abtop` (local fork at `85c0dcb`). No upstream PR.
**Builds on:** Phase A + B1 + B2 — `theme::loader::parse_theme_body`, `theme::apply_overrides`, `Theme::by_name`, `App.cycle_names`.

## Goal

`abtop --theme /tmp/scratch.theme` (or `./scratch.theme`, or `~/foo.theme`) reads that file directly and uses it for the session, bypassing the basename lookup chain. The existing `abtop --theme catppuccin` semantic is unchanged.

Use case: iterate on a theme outside `~/.config/abtop/themes/` — edit `/tmp/scratch.theme`, relaunch, repeat.

## Non-goals

- No persistence — path-loaded themes are one-shot. config.toml is **not** written.
- No new flag (`--theme-file` was considered and rejected). Path detection is automatic based on the arg shape.
- No support for fetching themes over HTTP / URL schemes.
- No JSON / alternate format support — parser is still btop-format only.
- No directory-of-themes resolution (`--theme /tmp/themes/` is not valid).

## Path detection rule

`--theme <arg>` is treated as a path if **any** of these is true:

- `arg` contains a `/`.
- `arg` contains a `\`.
- `arg` starts with `~` (followed by `/` or end-of-string).

Otherwise, `arg` is a basename and resolves through the existing chain (`Theme::by_name` → `try_user_file` → `embedded::lookup`).

**Examples:**

| `--theme` arg | Path or name? |
|---|---|
| `catppuccin` | name |
| `my-cool` | name |
| `scratch.theme` | name (no `/`) |
| `/tmp/scratch.theme` | path |
| `./scratch.theme` | path |
| `../scratch.theme` | path |
| `~/foo.theme` | path (with `~` expansion) |
| `~root/foo.theme` | path (with `~` expansion delegated to `dirs::home_dir()`; if it can't expand, the raw path passes through and the file read fails) |
| `C:\Users\me\x.theme` | path (Windows-style separator) |
| `scratch` | name |

Rationale: the rule catches every sensible path form without requiring a separate flag. It excludes bare CWD-relative filenames (`scratch.theme`), but that's idiomatic Unix — users prefix with `./` to mean "the file in CWD."

## Tilde expansion

`~` and `~/` expand via `dirs::home_dir()`. Examples:

- `~/foo.theme` → `<home>/foo.theme`
- `~` (no slash) → `<home>` (unusable — file read fails because home is a directory; reported via `Err`)

If `dirs::home_dir()` returns `None`, the `~`-prefixed path passes through unmodified. `std::fs::read_to_string` will then fail with the OS error, which is reported to the user.

`~someuser/path` (user-relative) is **not** supported. The character after `~` must be `/` or the string-end for expansion; otherwise the path is passed through as-is.

## Save behavior

When `--theme` resolves to a path, `save_theme(name)` is **not called**. `config.toml`'s `theme = "..."` line is left unchanged. The next launch (without `--theme`) reads the saved theme from config.toml as before.

When `--theme` resolves to a name (the existing behavior), `save_theme` is called as today.

## Architecture

```
              abtop --theme <arg>
                        │
                        ▼
              lib.rs::run() arg parse
                        │
                ┌───────┴────────┐
                │                │
        is_path_arg(&arg)?       │
                │                │
        ┌───────┴────────┐       │
        │                │       │
       yes              no       │
        │                │       │
        ▼                ▼       │
  expand_tilde       Theme::by_name
        │                │       │
        ▼                ▼       │
  load_from_path    (existing flow: validate, hard-fail,
        │           load_or_default, save_theme)
        ▼
  apply_overrides(&cfg) ───────────────────► continue startup
```

## Code changes

### `src/theme/loader.rs` — new `load_from_path` function

```rust
/// Load a theme directly from a filesystem path. Returns the parsed Theme
/// with `name` derived from `path.file_stem()`. Errors propagate as String
/// messages for the CLI to print.
///
/// Use case: `--theme /tmp/scratch.theme` and similar one-shot iteration.
/// The caller is responsible for skipping `save_theme` — this function
/// only loads, it doesn't persist.
pub(crate) fn load_from_path(path: &Path) -> Result<Theme, String> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("custom")
        .to_string();
    Ok(parse_theme_body(&body, &name))
}
```

No new dependencies. `Path` and `parse_theme_body` already in scope.

### `src/lib.rs` — fork the `--theme` arg path

A small helper:

```rust
/// Decide whether `--theme <arg>` should be treated as a path.
fn is_theme_path_arg(arg: &str) -> bool {
    arg.contains('/') || arg.contains('\\') || arg.starts_with('~')
}

/// Expand a leading `~/` or bare `~` using `dirs::home_dir()`. Returns
/// the input unchanged if expansion isn't possible or the path doesn't
/// start with `~`.
fn expand_tilde(arg: &str) -> std::path::PathBuf {
    if let Some(rest) = arg.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if arg == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    std::path::PathBuf::from(arg)
}
```

The startup validation block currently (post-B1) looks like:

```rust
if let Some(name) = &cli_theme_name {
    if theme::Theme::by_name(name).is_none() {
        eprintln!("unknown theme '{}'. available: {}",
            name, theme::THEME_NAMES.join(", "));
        std::process::exit(1);
    }
}

let resolved_name = cli_theme_name.unwrap_or_else(|| cfg.theme.clone());
let initial_theme: theme::Theme = theme::load_or_default(&resolved_name, &cfg);
```

Becomes:

```rust
let initial_theme: theme::Theme = match &cli_theme_name {
    Some(arg) if is_theme_path_arg(arg) => {
        let path = expand_tilde(arg);
        match theme::loader::load_from_path(&path) {
            Ok(mut t) => {
                theme::apply_overrides(&mut t, &cfg);
                t
            }
            Err(msg) => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        }
    }
    Some(name) => {
        if theme::Theme::by_name(name).is_none() {
            eprintln!("unknown theme '{}'. available: {}",
                name, theme::THEME_NAMES.join(", "));
            std::process::exit(1);
        }
        theme::load_or_default(name, &cfg)
    }
    None => theme::load_or_default(&cfg.theme, &cfg),
};
```

The path branch reaches `theme::loader::load_from_path` directly. Since `loader` is `mod loader` (private), we need to either:
- a) Re-export `load_from_path` via `pub(crate) use loader::load_from_path` in `theme/mod.rs` (matches B1/B2 discipline);
- b) Make `mod loader` → `pub(crate) mod loader` so `crate::theme::loader::*` is callable from `lib.rs`.

Option (a) is more in line with the existing pattern (`apply_overrides`, `load_or_default` are re-exported, not reached via path).

So `theme/mod.rs` re-export grows:

```rust
pub(crate) use loader::{
    apply_overrides, dump_embedded, list_available, load_from_path, load_or_default, Source,
};
```

### Existing `save_theme` call site

The existing save logic for `--theme <name>` lives **inside** `App::cycle_theme` (called by the `t` key), not in startup. Startup never calls `save_theme` today — config.toml is read at startup but only written when the user presses `t` or toggles a panel. So the "don't save" non-goal is already satisfied trivially: the path branch doesn't add a `save_theme` call. No change to `cycle_theme`.

The `t` key behavior after launching with a path-theme: the path-derived name (e.g. `scratch`) won't be in `cycle_names` or `THEME_NAMES`, so `position()` returns `None`, `unwrap_or(0)` lands the cycle at index 0 (btop). Pressing `t` from a path-theme always advances to btop on the first press, then cycles normally. **Subsequent `t`-presses also write the new theme to config.toml** (existing `cycle_theme` behavior), so cycling out of a path-theme persists the new selection as today. This is the intended ergonomic.

## Public surface change

- `theme::loader::load_from_path` — new function.
- `theme::load_from_path` — re-exported as `pub(crate)`.

Nothing else added. No `Source` / `ThemeListing` changes. No new CLI flag.

## Test surface

In `src/theme/loader.rs`:

- `load_from_path_reads_a_theme_file` — write a small btop-style body to a tempdir file; call `load_from_path`; assert returned Theme's `main_bg` matches the parsed value.
- `load_from_path_returns_err_on_missing_file` — `load_from_path(&PathBuf::from("/nonexistent/x.theme"))` → `Err`.
- `load_from_path_uses_file_stem_as_name` — `/tmp/scratch.theme` → `theme.name == "scratch"`.
- `load_from_path_handles_extension_other_than_theme` — `load_from_path(/tmp/x.txt)` → succeeds, `theme.name == "x"`.

In `src/lib.rs` (or via separate helper module):

- `is_theme_path_arg_detects_separators` — unit test the rule. Cases: `"catppuccin"` → false; `"./x.theme"` → true; `"/tmp/x"` → true; `"~/x"` → true; `"~"` → true; `"my-theme.theme"` → false; `"C:\\x"` → true.
- `expand_tilde_handles_tilde_prefix` — `~/foo` → `<home>/foo`; `~` → `<home>`; `/abs` → `/abs` (unchanged); `relative` → `PathBuf::from("relative")` (unchanged).

Manual smoke:

```sh
# Path mode
cp themes/catppuccin.theme /tmp/scratch.theme
abtop --theme /tmp/scratch.theme --once | head -1
# Expect: snapshot prints; no panic; no save to config.toml

abtop --theme ./themes/dracula.theme --once
# Expect: same

abtop --theme ~/.config/abtop/themes/catppuccin.theme --once
# (after dumping with --dump-theme catppuccin if not already present)

# Failure cases
abtop --theme /nonexistent.theme --once
# Expect: stderr "failed to read /nonexistent.theme: No such file or directory"; exit 1

# Name mode (unchanged)
abtop --theme catppuccin --once
# Expect: works as before
```

## Error handling

- Read failure (missing file, permission denied) → `Err(String)` with the OS error → printed to stderr → exit 1.
- Empty file → `parse_theme_body` succeeds with all keys at btop defaults (existing behavior). Probably user error but not an error to abtop.
- Malformed body → `parse_theme_body` skips unparseable lines silently (existing behavior).

No new failure modes added by B3. The only new error path is "file doesn't exist."

## Build & install

```sh
cd ~/my-workspace/util/abtop
cargo test --lib
cargo build --release
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

## Acceptance criteria

1. `cargo test --lib` passes (test count grows by ~6 new tests).
2. `cargo build --release` clean.
3. `abtop --theme /tmp/scratch.theme --once` reads the file and produces a snapshot.
4. `abtop --theme catppuccin --once` works unchanged (basename mode).
5. `abtop --theme /missing/x.theme --once` exits 1 with a readable error message.
6. config.toml is NOT modified by startup when `--theme` is a path. (Subsequent `t`-presses still write to config.toml — that's existing `cycle_theme` behavior, intentionally unchanged.)
7. `abtop --theme ~/foo.theme` expands `~` and reads from the resulting absolute path.

## Out of scope (other Phase B items)

- B4: Banner UI on malformed theme file — separate spec.
- B5: Reload-on-file-change — separate spec.
- B6: macOS Library → XDG migration — deferred indefinitely.
