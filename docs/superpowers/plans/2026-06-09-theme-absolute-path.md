# `--theme <path>` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend `abtop --theme <arg>` to accept a path (`/abs/x.theme`, `./rel.theme`, `~/home.theme`) alongside the existing basename lookup, without persisting path-mode themes to `config.toml`.

**Architecture:** New `pub(crate) fn load_from_path(path) -> Result<Theme, String>` in `src/theme/loader.rs` reads the file and parses it via existing `parse_theme_body`. Two small helpers in `src/lib.rs` (`is_theme_path_arg` + `expand_tilde`) decide which mode the arg takes. The existing `--theme <arg>` startup block forks into a path branch (load + apply_overrides + no save) vs the existing name branch (unchanged). `cycle_theme` save behavior is untouched.

**Tech Stack:** Rust 2021, std (`std::fs`, `std::path`), `dirs` crate (already a dep, used for home expansion). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-06-09-theme-absolute-path-design.md` (commit `f1b074e`).

---

## File Structure

| Path | Change | Responsibility |
|---|---|---|
| `src/theme/loader.rs` | MODIFY | Add `load_from_path()` + 4 TDD tests. |
| `src/theme/mod.rs` | MODIFY | Extend the `pub(crate) use loader::{...}` line to include `load_from_path`. |
| `src/lib.rs` | MODIFY | Add `is_theme_path_arg` + `expand_tilde` helpers with inline tests; fork the existing `--theme` startup block into path-mode vs name-mode. |

No new files. No new dependencies.

---

## Task 1: Add `load_from_path` to `src/theme/loader.rs`

**Files:**
- Modify: `src/theme/loader.rs` (add function + 4 TDD tests)

- [ ] **Step 1: Add failing tests**

Append to the existing `#[cfg(test)] mod tests` block in `src/theme/loader.rs`:

```rust
#[test]
fn load_from_path_reads_a_theme_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("scratch.theme");
    std::fs::write(&path, r#"theme[main_bg]="#112233""#).unwrap();
    let t = load_from_path(&path).expect("load should succeed");
    assert_eq!(t.main_bg, Color::Rgb(0x11, 0x22, 0x33));
}

#[test]
fn load_from_path_returns_err_on_missing_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("does-not-exist.theme");
    let result = load_from_path(&path);
    let err = result.expect_err("missing file must error");
    assert!(
        err.contains("failed to read"),
        "error should mention read failure: {err}"
    );
}

#[test]
fn load_from_path_uses_file_stem_as_name() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("my-scratch.theme");
    std::fs::write(&path, "").unwrap();
    let t = load_from_path(&path).expect("load should succeed");
    assert_eq!(t.name, "my-scratch");
}

#[test]
fn load_from_path_handles_extension_other_than_theme() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("x.txt");
    std::fs::write(&path, "").unwrap();
    let t = load_from_path(&path).expect("load should succeed");
    // file_stem() strips only the LAST extension, so "x.txt" -> "x".
    assert_eq!(t.name, "x");
}
```

- [ ] **Step 2: Verify the tests fail**

Run from `/Users/a.salvi/my-workspace/util/abtop`:

```bash
cargo test --lib --quiet theme::loader::tests::load_from_path 2>&1 | tail -10
```

Expected: compile error — `load_from_path` not found.

- [ ] **Step 3: Implement `load_from_path`**

Add to `src/theme/loader.rs`. A natural spot is just below `dump_embedded` (so all the path-aware operations sit together):

```rust
/// Load a theme directly from a filesystem path. Returns the parsed Theme
/// with `name` derived from `path.file_stem()`. Errors propagate as
/// String messages for CLI display.
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

- [ ] **Step 4: Verify the tests pass**

```bash
cargo test --lib --quiet theme::loader::tests::load_from_path 2>&1 | tail -10
```

Expected: 4 tests pass.

- [ ] **Step 5: Verify the full suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 221 passed (217 + 4 new).

- [ ] **Step 6: Verify clean release build**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: clean (a `dead_code` warning on `load_from_path` is acceptable here — it'll have a caller after Task 4 lands. If you want to silence it now, add `#[allow(dead_code)] // wired up in Task 4` above the fn, and remove the attribute when Task 4 lands.)

- [ ] **Step 7: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add load_from_path for direct file loading

Reads a .theme file from any filesystem path and parses it via
parse_theme_body. Derives the theme's name from path.file_stem().
String-typed error keeps the CLI handler simple. Caller is
responsible for skipping save_theme; load_from_path only loads."
```

---

## Task 2: Re-export `load_from_path` from `src/theme/mod.rs`

**Files:**
- Modify: `src/theme/mod.rs`

- [ ] **Step 1: Extend the existing `pub(crate) use` line**

The current re-export block reads:

```rust
mod loader;
// Loader helpers are crate-internal: app.rs / lib.rs / cycle_theme consume them,
// but nothing outside the crate should bind to these names (the crate is
// published to crates.io and these are wiring details, not stable API).
pub(crate) use loader::{
    apply_overrides, dump_embedded, list_available, load_or_default, Source,
};
```

Add `load_from_path`:

```rust
pub(crate) use loader::{
    apply_overrides, dump_embedded, list_available, load_from_path, load_or_default, Source,
};
```

(Alphabetical order is roughly preserved.)

- [ ] **Step 2: Verify the build is clean**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: clean.

- [ ] **Step 3: Verify the test suite still passes**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 221 passed.

- [ ] **Step 4: Commit**

```bash
git add src/theme/mod.rs
git commit -m "feat(theme): re-export load_from_path as pub(crate)

So lib.rs's --theme handler can call theme::load_from_path(...)
without reaching into the private loader module."
```

---

## Task 3: Add `is_theme_path_arg` + `expand_tilde` helpers to `src/lib.rs`

**Files:**
- Modify: `src/lib.rs` (add helpers + inline tests)

- [ ] **Step 1: Add failing tests**

`src/lib.rs` may or may not have an existing `#[cfg(test)] mod tests` block. If it does, append to it; otherwise add a new one at the bottom of the file. Test code:

```rust
#[cfg(test)]
mod theme_arg_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn is_theme_path_arg_detects_separators() {
        // Bare names are not paths.
        assert!(!is_theme_path_arg("catppuccin"));
        assert!(!is_theme_path_arg("my-theme"));
        assert!(!is_theme_path_arg("scratch.theme"));
        assert!(!is_theme_path_arg(""));

        // Path-shaped args.
        assert!(is_theme_path_arg("/tmp/x.theme"));
        assert!(is_theme_path_arg("./scratch.theme"));
        assert!(is_theme_path_arg("../up.theme"));
        assert!(is_theme_path_arg("~/foo.theme"));
        assert!(is_theme_path_arg("~"));
        assert!(is_theme_path_arg("C:\\Users\\me\\x.theme"));
        assert!(is_theme_path_arg("dir/file.theme"));
    }

    #[test]
    fn expand_tilde_expands_home_relative_paths() {
        let home = dirs::home_dir().expect("home_dir for this test platform");
        assert_eq!(expand_tilde("~/foo.theme"), home.join("foo.theme"));
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn expand_tilde_passes_through_non_tilde_paths() {
        assert_eq!(expand_tilde("/tmp/x.theme"), PathBuf::from("/tmp/x.theme"));
        assert_eq!(expand_tilde("./rel"), PathBuf::from("./rel"));
        assert_eq!(expand_tilde("relative"), PathBuf::from("relative"));
    }

    #[test]
    fn expand_tilde_does_not_expand_user_relative_tilde() {
        // ~root/path is a path with a tilde but NOT one we expand — we
        // only handle bare ~ and ~/ prefixes. The arg passes through.
        assert_eq!(expand_tilde("~root/path"), PathBuf::from("~root/path"));
    }
}
```

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test --lib --quiet theme_arg_tests 2>&1 | tail -10
```

Expected: compile error — `is_theme_path_arg` and `expand_tilde` not found.

- [ ] **Step 3: Add the helpers to `src/lib.rs`**

Add at a sensible location near the top of `src/lib.rs` (e.g. just above `build_app`, around line 78). The functions are crate-internal helpers:

```rust
/// Decide whether `--theme <arg>` should be treated as a path. Returns
/// true if the arg contains a path separator (`/` or `\`) or starts with
/// `~` (home-relative). Otherwise the arg is a theme name and resolves
/// through the basename lookup chain.
fn is_theme_path_arg(arg: &str) -> bool {
    arg.contains('/') || arg.contains('\\') || arg.starts_with('~')
}

/// Expand a leading `~/` (or bare `~`) to the user's home directory.
/// Returns the input unchanged if expansion isn't possible or the path
/// doesn't start with `~`. Does NOT support `~someuser/path` —
/// that arg passes through.
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

- [ ] **Step 4: Verify the tests pass**

```bash
cargo test --lib --quiet theme_arg_tests 2>&1 | tail -10
```

Expected: 4 tests pass.

- [ ] **Step 5: Verify the full suite still passes**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 225 passed (221 + 4 new).

- [ ] **Step 6: Verify clean release build**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: clean. Same `dead_code` caveat as Task 1 if the helpers aren't yet called — they'll be wired in Task 4. If a warning fires, add `#[allow(dead_code)]` to both and remove in Task 4.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs
git commit -m "feat(lib): add is_theme_path_arg + expand_tilde helpers

Decides path-mode vs name-mode for --theme <arg> and expands a
leading ~/ to dirs::home_dir(). Used in Task 4 to fork the
startup theme-resolution flow."
```

---

## Task 4: Fork the `--theme` startup flow in `src/lib.rs::run()`

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Locate the existing block**

The current block (around `src/lib.rs:178-191`) is:

```rust
    // Validate CLI --theme exists in user dir or embedded; hard-fail if not.
    if let Some(name) = &cli_theme_name {
        if theme::Theme::by_name(name).is_none() {
            eprintln!(
                "unknown theme '{}'. available: {}",
                name,
                theme::THEME_NAMES.join(", ")
            );
            std::process::exit(1);
        }
    }

    let resolved_name = cli_theme_name.unwrap_or_else(|| cfg.theme.clone());
    let initial_theme: theme::Theme = theme::load_or_default(&resolved_name, &cfg);
```

- [ ] **Step 2: Replace with a path-vs-name fork**

Replace those 14 lines with:

```rust
    let initial_theme: theme::Theme = match &cli_theme_name {
        Some(arg) if is_theme_path_arg(arg) => {
            // Path mode: read the file directly, apply config overrides,
            // do NOT save_theme (path themes are one-shot).
            let path = expand_tilde(arg);
            match theme::load_from_path(&path) {
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
            // Name mode: existing flow. Validate via by_name and hard-fail
            // on miss, then load_or_default + apply_overrides happen inside
            // load_or_default itself.
            if theme::Theme::by_name(name).is_none() {
                eprintln!(
                    "unknown theme '{}'. available: {}",
                    name,
                    theme::THEME_NAMES.join(", ")
                );
                std::process::exit(1);
            }
            theme::load_or_default(name, &cfg)
        }
        None => theme::load_or_default(&cfg.theme, &cfg),
    };
```

If Task 1 or Task 3 added `#[allow(dead_code)]` attributes, **remove them now** — the symbols are reached.

- [ ] **Step 3: Verify the build is clean**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: clean release build with no warnings (especially no `dead_code` warning on `load_from_path`, `is_theme_path_arg`, or `expand_tilde`).

- [ ] **Step 4: Verify the full suite still passes**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 225 passed.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs
git commit -m "feat(cli): --theme <arg> accepts file paths

Forks the existing startup block: args containing /, \\, or
starting with ~ go through load_from_path; everything else uses
the basename chain unchanged. Path themes don't persist to
config.toml (no save_theme call at startup); the existing
cycle_theme save behavior is untouched."
```

---

## Task 5: Build, install, and smoke test

**Files:** none (build + install + manual smoke)

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 225 passed.

- [ ] **Step 2: Build release**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: `Finished` with no errors.

- [ ] **Step 3: Install**

```bash
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

- [ ] **Step 4: Smoke test — name mode unchanged**

```bash
~/.local/libexec/abtop --theme catppuccin --once 2>&1 | head -1
echo "exit: $?"
```

Expected: snapshot output; exit 0.

Verify config.toml still tracks whatever it tracked:

```bash
rg '^theme = ' ~/.config/abtop/config.toml
```

Expected: `theme = "catppuccin"` (or whatever was there). (`--once` doesn't trigger `cycle_theme`, so no save fires here either — but the test below also covers explicit non-save.)

- [ ] **Step 5: Smoke test — happy-path absolute path**

```bash
cp themes/dracula.theme /tmp/scratch.theme
~/.local/libexec/abtop --theme /tmp/scratch.theme --once 2>&1 | head -1
echo "exit: $?"
```

Expected: snapshot output; exit 0.

Verify config.toml is unchanged:

```bash
rg '^theme = ' ~/.config/abtop/config.toml
```

Expected: same value as in Step 4 — path mode did NOT save.

- [ ] **Step 6: Smoke test — relative path**

```bash
cd /Users/a.salvi/my-workspace/util/abtop
~/.local/libexec/abtop --theme ./themes/dracula.theme --once 2>&1 | head -1
echo "exit: $?"
```

Expected: snapshot output; exit 0.

- [ ] **Step 7: Smoke test — tilde expansion**

```bash
cp themes/dracula.theme ~/scratch-test.theme
~/.local/libexec/abtop --theme ~/scratch-test.theme --once 2>&1 | head -1
echo "exit: $?"
rm ~/scratch-test.theme
```

Expected: snapshot output; exit 0.

- [ ] **Step 8: Smoke test — missing file**

```bash
~/.local/libexec/abtop --theme /nonexistent/missing.theme --once 2>&1
echo "exit: $?"
```

Expected: stderr contains `failed to read /nonexistent/missing.theme: No such file or directory`; exit 1.

- [ ] **Step 9: Smoke test — bare-name fall through to name mode**

```bash
~/.local/libexec/abtop --theme scratch.theme --once 2>&1
echo "exit: $?"
```

Expected: stderr contains `unknown theme 'scratch.theme'. available: …` (because `scratch.theme` has no `/` so it's a name, and there's no `scratch.theme` in BUILTIN or user dir); exit 1. This verifies the path-detection rule excludes bare filenames with extensions.

- [ ] **Step 10: Clean up the scratch file**

```bash
rm /tmp/scratch.theme
```

- [ ] **Step 11: No commit needed — install is a side effect.**

---

## Acceptance criteria

1. `cargo test --lib` passes (225 tests; was 217 at B3 start, +8 new).
2. `cargo build --release` clean with no warnings.
3. `abtop --theme catppuccin` works unchanged (basename mode).
4. `abtop --theme /tmp/scratch.theme` reads the file and runs.
5. `abtop --theme ./relative.theme` and `abtop --theme ~/home.theme` both work via the same code path.
6. `abtop --theme /missing/x.theme` exits 1 with a readable error message.
7. config.toml is NOT modified by startup when `--theme` is a path.
8. No new `pub` items in `theme::` or other module re-exports — `load_from_path` is added to the `pub(crate)` re-export list only.
9. `t`-key cycle still works from a path-launched theme (lands at btop on first press via the existing position-fallback).

## Out of scope (other Phase B items)

- B4: Banner UI on malformed theme file — separate spec.
- B5: Reload-on-file-change — separate spec.
- B6: macOS `~/Library/Application Support/abtop/` → XDG migration — deferred indefinitely.
