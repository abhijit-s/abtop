# `--list-themes` and `--dump-theme` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two early-return CLI flags to abtop: `--list-themes` (print embedded + user-dir themes with source markers) and `--dump-theme <name> [--force]` (write an embedded theme's body to the user themes dir for editing).

**Architecture:** Listing + dump logic lives in `src/theme/loader.rs` as pure functions over a `&Path` config root. CLI handlers in `src/lib.rs::run()` mirror the existing `--version` / `--setup` early-return pattern: short-circuit before config load and app build. Both functions reuse `embedded::BUILTIN` and `config::xdg_config_dir()` from Phase A — no parser changes, no architectural changes.

**Tech Stack:** Rust 2021, std (`std::fs`, `std::path`, `std::env::args`), `tempfile` crate (test fixtures — already a dep), `ratatui::style::Color` (only via existing parser path, not directly added here).

**Spec:** `docs/superpowers/specs/2026-06-09-list-and-dump-theme-design.md` (commit `0a4aea3`).

---

## File Structure

| Path | Change | Responsibility |
|---|---|---|
| `src/theme/loader.rs` | MODIFY | Add `Source` enum, `ThemeListing` struct, `list_available()`, `dump_embedded()` + tests. |
| `src/theme/mod.rs` | MODIFY | Extend the `pub use loader::{...}` line to re-export the three new public symbols. |
| `src/lib.rs` | MODIFY | Two new early-return blocks in `run()` for `--list-themes` and `--dump-theme`. |
| `README.md` | MODIFY | Document the two new flags in the existing "Theming" section. |

No new files. No new dependencies.

---

## Task 1: Add `Source` enum and `ThemeListing` struct

**Files:**
- Modify: `src/theme/loader.rs` (append types near the existing public items)

- [ ] **Step 1: Add a failing test**

Append to the existing `#[cfg(test)] mod tests` block in `src/theme/loader.rs`:

```rust
#[test]
fn theme_listing_basics() {
    let l = ThemeListing { name: "btop".to_string(), source: Source::Builtin };
    assert_eq!(l.name, "btop");
    assert_eq!(l.source, Source::Builtin);

    // Equality and Clone work for use in test assertions.
    let l2 = l.clone();
    assert_eq!(l, l2);

    // Debug formats without panicking.
    let _ = format!("{l:?}");

    // Three variants are distinct.
    assert_ne!(Source::Builtin, Source::User);
    assert_ne!(Source::Builtin, Source::UserOverride);
    assert_ne!(Source::User, Source::UserOverride);
}
```

- [ ] **Step 2: Verify the test fails**

Run from `/Users/a.salvi/my-workspace/util/abtop`:

```bash
cargo test --lib --quiet theme::loader::tests::theme_listing_basics 2>&1 | tail -10
```

Expected: compile error — `Source` and `ThemeListing` not found.

- [ ] **Step 3: Add the types**

In `src/theme/loader.rs`, just below the existing `use std::path::Path;` line (or at a sensible location above the existing `try_user_file` function), add:

```rust
/// Where a theme comes from when surfaced by `list_available`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Shipped via `embedded::BUILTIN`; no user file shadows it.
    Builtin,
    /// User file at `$XDG_CONFIG_HOME/abtop/themes/<name>.theme`, name not in BUILTIN.
    User,
    /// User file shadows a BUILTIN entry with the same name.
    UserOverride,
}

/// One entry in the output of `list_available`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeListing {
    pub name: String,
    pub source: Source,
}
```

- [ ] **Step 4: Verify the test passes**

Run:

```bash
cargo test --lib --quiet theme::loader::tests::theme_listing_basics 2>&1 | tail -5
```

Expected: 1 test passes.

- [ ] **Step 5: Verify the full suite still passes**

Run:

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 204 passed (was 203 + 1 new test).

- [ ] **Step 6: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add Source and ThemeListing types

Companion types for the upcoming list_available() function. Both
types are public because the lib.rs CLI handlers will construct
and read them."
```

---

## Task 2: Add `list_available` function

**Files:**
- Modify: `src/theme/loader.rs` (add function + 5 TDD tests)

- [ ] **Step 1: Add failing tests**

Append to the `#[cfg(test)] mod tests` block in `src/theme/loader.rs`:

```rust
#[test]
fn list_available_empty_user_dir_returns_only_builtin() {
    let tmp = TempDir::new().unwrap();
    // Note: tmp.path()/abtop/themes/ does not exist.
    let listings = list_available(tmp.path());

    // Should be exactly the 13 embedded themes in BUILTIN order, all Builtin.
    assert_eq!(listings.len(), 13);
    for (i, (name, _)) in crate::theme::embedded::BUILTIN.iter().enumerate() {
        assert_eq!(listings[i].name, *name);
        assert_eq!(listings[i].source, Source::Builtin);
    }
}

#[test]
fn list_available_user_only_themes_appended_alphabetically() {
    let tmp = TempDir::new().unwrap();
    let themes_dir = tmp.path().join("abtop").join("themes");
    write_theme_file(&themes_dir, "zorak", r#"theme[main_fg]="#ff0000""#);
    write_theme_file(&themes_dir, "my-cool", r#"theme[main_fg]="#00ff00""#);

    let listings = list_available(tmp.path());

    // 13 builtin + 2 user-only = 15 total.
    assert_eq!(listings.len(), 15);
    // User-only entries appear after the 13 builtins.
    assert_eq!(listings[13].name, "my-cool");
    assert_eq!(listings[13].source, Source::User);
    assert_eq!(listings[14].name, "zorak");
    assert_eq!(listings[14].source, Source::User);
}

#[test]
fn list_available_user_override_promotes_builtin_entry() {
    let tmp = TempDir::new().unwrap();
    let themes_dir = tmp.path().join("abtop").join("themes");
    write_theme_file(&themes_dir, "catppuccin", r#"theme[main_fg]="#abcdef""#);

    let listings = list_available(tmp.path());

    // Still 13 entries — no duplicate, just promoted.
    assert_eq!(listings.len(), 13);
    let catppuccin = listings
        .iter()
        .find(|l| l.name == "catppuccin")
        .expect("catppuccin present");
    assert_eq!(catppuccin.source, Source::UserOverride);
    // Catppuccin should still appear in BUILTIN order, not appended.
    let pos = listings.iter().position(|l| l.name == "catppuccin").unwrap();
    let builtin_pos = crate::theme::embedded::BUILTIN
        .iter()
        .position(|(n, _)| *n == "catppuccin")
        .unwrap();
    assert_eq!(pos, builtin_pos);
}

#[test]
fn list_available_skips_hidden_and_non_theme_files() {
    let tmp = TempDir::new().unwrap();
    let themes_dir = tmp.path().join("abtop").join("themes");
    std::fs::create_dir_all(&themes_dir).unwrap();
    // Hidden vim swap-style file should be skipped.
    std::fs::write(themes_dir.join(".catppuccin.theme.swp"), "junk").unwrap();
    // Non-.theme files should be skipped.
    std::fs::write(themes_dir.join("notes.md"), "junk").unwrap();
    std::fs::write(themes_dir.join("README"), "junk").unwrap();

    let listings = list_available(tmp.path());

    // None of the junk should appear; only the 13 builtins.
    assert_eq!(listings.len(), 13);
    assert!(listings.iter().all(|l| l.source == Source::Builtin));
}

#[test]
fn list_available_returns_builtin_when_user_dir_unreadable() {
    // Pass a config root whose abtop/themes/ doesn't exist.
    let tmp = TempDir::new().unwrap();
    let bogus = tmp.path().join("does-not-exist");
    let listings = list_available(&bogus);
    assert_eq!(listings.len(), 13);
    assert!(listings.iter().all(|l| l.source == Source::Builtin));
}
```

The helper `write_theme_file` already exists in the test module from Phase A (defined near the `load_chain_*` tests).

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test --lib --quiet theme::loader::tests::list_available 2>&1 | tail -10
```

Expected: compile error — `list_available` not found.

- [ ] **Step 3: Implement `list_available`**

Add to `src/theme/loader.rs` (a good location is just below `try_user_file` so all the user-dir-aware helpers are co-located):

```rust
/// List all themes available for selection: the embedded BUILTIN entries
/// plus user files under `<config_root>/abtop/themes/`. Embedded entries
/// are listed in BUILTIN order. User-only entries are appended alphabetically.
///
/// Filesystem errors (missing dir, permission denied, etc.) are treated as
/// "no user themes" — the embedded list is unconditionally returned.
pub fn list_available(config_root: &Path) -> Vec<ThemeListing> {
    // Collect user theme basenames first (Vec<String>).
    let themes_dir = config_root.join("abtop").join("themes");
    let mut user_names: Vec<String> = match std::fs::read_dir(&themes_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                let file_name = path.file_name()?.to_str()?.to_owned();
                // Skip hidden files (e.g. .swp from vim).
                if file_name.starts_with('.') {
                    return None;
                }
                // Strip the .theme extension.
                let stem = file_name.strip_suffix(".theme")?;
                if stem.is_empty() {
                    return None;
                }
                Some(stem.to_owned())
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    user_names.sort();

    let user_set: std::collections::HashSet<&str> =
        user_names.iter().map(|s| s.as_str()).collect();
    let builtin_set: std::collections::HashSet<&str> = crate::theme::embedded::BUILTIN
        .iter()
        .map(|(n, _)| *n)
        .collect();

    let mut out: Vec<ThemeListing> = Vec::new();

    // 1. Builtins in declaration order, promoted to UserOverride if shadowed.
    for (name, _) in crate::theme::embedded::BUILTIN.iter() {
        let source = if user_set.contains(name) {
            Source::UserOverride
        } else {
            Source::Builtin
        };
        out.push(ThemeListing {
            name: (*name).to_string(),
            source,
        });
    }

    // 2. User-only names (those not in BUILTIN), already alphabetically sorted.
    for name in &user_names {
        if !builtin_set.contains(name.as_str()) {
            out.push(ThemeListing {
                name: name.clone(),
                source: Source::User,
            });
        }
    }

    out
}
```

- [ ] **Step 4: Verify the tests pass**

```bash
cargo test --lib --quiet theme::loader::tests::list_available 2>&1 | tail -10
```

Expected: 5 tests pass.

- [ ] **Step 5: Verify the full suite passes**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 209 passed (204 + 5 new).

- [ ] **Step 6: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add list_available() for theme discovery

Returns embedded BUILTIN entries in declaration order plus user-dir
themes alphabetically appended. User files shadowing a BUILTIN entry
promote that entry to Source::UserOverride; user-only files are
Source::User. Filesystem errors yield an embedded-only result —
the function is infallible."
```

---

## Task 3: Add `dump_embedded` function

**Files:**
- Modify: `src/theme/loader.rs` (add function + 5 TDD tests)

- [ ] **Step 1: Add failing tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn dump_embedded_writes_new_file() {
    let tmp = TempDir::new().unwrap();
    let result = dump_embedded(tmp.path(), "catppuccin", false);
    let path = result.expect("dump should succeed");

    assert_eq!(
        path,
        tmp.path().join("abtop").join("themes").join("catppuccin.theme")
    );
    let body = std::fs::read_to_string(&path).unwrap();
    // Sanity: the written body matches the embedded body byte-for-byte.
    let embedded = crate::theme::embedded::lookup("catppuccin")
        .expect("catppuccin in BUILTIN");
    assert_eq!(body, embedded);
}

#[test]
fn dump_embedded_refuses_existing_without_force() {
    let tmp = TempDir::new().unwrap();
    let themes_dir = tmp.path().join("abtop").join("themes");
    std::fs::create_dir_all(&themes_dir).unwrap();
    let target = themes_dir.join("catppuccin.theme");
    std::fs::write(&target, "existing content").unwrap();

    let result = dump_embedded(tmp.path(), "catppuccin", false);
    let err = result.expect_err("should refuse to overwrite without --force");
    assert!(
        err.contains("already exists"),
        "error message should mention exists: {err}"
    );
    assert!(
        err.contains("--force"),
        "error message should suggest --force: {err}"
    );

    // File contents must be untouched.
    let body = std::fs::read_to_string(&target).unwrap();
    assert_eq!(body, "existing content");
}

#[test]
fn dump_embedded_overwrites_with_force() {
    let tmp = TempDir::new().unwrap();
    let themes_dir = tmp.path().join("abtop").join("themes");
    std::fs::create_dir_all(&themes_dir).unwrap();
    let target = themes_dir.join("catppuccin.theme");
    std::fs::write(&target, "existing content").unwrap();

    let result = dump_embedded(tmp.path(), "catppuccin", true);
    result.expect("should overwrite with force");

    let body = std::fs::read_to_string(&target).unwrap();
    let embedded = crate::theme::embedded::lookup("catppuccin").unwrap();
    assert_eq!(body, embedded);
}

#[test]
fn dump_embedded_rejects_non_embedded_name() {
    let tmp = TempDir::new().unwrap();
    let result = dump_embedded(tmp.path(), "not-a-real-theme", false);
    let err = result.expect_err("non-embedded name must error");
    assert!(
        err.contains("not an embedded theme"),
        "error should say not embedded: {err}"
    );
    // Nothing was written.
    assert!(!tmp.path().join("abtop").exists());
}

#[test]
fn dump_embedded_rejects_path_traversal_names() {
    let tmp = TempDir::new().unwrap();
    for bad in ["../evil", "..", "sub/name", "name\\back", ""] {
        let result = dump_embedded(tmp.path(), bad, true);
        let err = result.expect_err(&format!("'{bad}' should be rejected"));
        assert!(
            err.contains("invalid theme name"),
            "error should mention invalid name: {err}"
        );
    }
}
```

- [ ] **Step 2: Verify the tests fail**

```bash
cargo test --lib --quiet theme::loader::tests::dump_embedded 2>&1 | tail -10
```

Expected: compile error — `dump_embedded` not found.

- [ ] **Step 3: Implement `dump_embedded`**

Add to `src/theme/loader.rs`, just below `list_available`:

```rust
/// Write the embedded body of `name` to `<config_root>/abtop/themes/<name>.theme`.
///
/// Returns the absolute path of the written file on success.
///
/// Errors:
/// - `name` contains path separators or `..` → invalid theme name.
/// - `name` is not in `embedded::BUILTIN` → nothing to dump.
/// - Target file exists and `force` is false → refuse.
/// - I/O failure during mkdir or write → propagate the OS error.
pub fn dump_embedded(
    config_root: &Path,
    name: &str,
    force: bool,
) -> Result<std::path::PathBuf, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!(
            "invalid theme name '{name}': contains '/', '\\\\', or '..'"
        ));
    }
    let body = crate::theme::embedded::lookup(name).ok_or_else(|| {
        let available: Vec<&str> = crate::theme::embedded::BUILTIN
            .iter()
            .map(|(n, _)| *n)
            .collect();
        format!(
            "'{name}' is not an embedded theme; nothing to dump. available: {}",
            available.join(", ")
        )
    })?;

    let themes_dir = config_root.join("abtop").join("themes");
    let target = themes_dir.join(format!("{name}.theme"));

    if target.exists() && !force {
        return Err(format!(
            "{} already exists. Re-run with --force to overwrite.",
            target.display()
        ));
    }

    std::fs::create_dir_all(&themes_dir)
        .map_err(|e| format!("failed to create {}: {e}", themes_dir.display()))?;
    std::fs::write(&target, body)
        .map_err(|e| format!("failed to write {}: {e}", target.display()))?;

    Ok(target)
}
```

- [ ] **Step 4: Verify the tests pass**

```bash
cargo test --lib --quiet theme::loader::tests::dump_embedded 2>&1 | tail -10
```

Expected: 5 tests pass.

- [ ] **Step 5: Verify the full suite passes**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 214 passed (209 + 5 new).

- [ ] **Step 6: Commit**

```bash
git add src/theme/loader.rs
git commit -m "feat(theme): add dump_embedded() for editing builtins in place

Writes the body of an embedded theme to the user themes dir so the
user can edit it. Refuses to overwrite an existing file unless force
is true. Rejects path-traversal names with the same guard as
try_user_file. I/O errors propagate as String messages for CLI
display."
```

---

## Task 4: Re-export the three new public symbols

**Files:**
- Modify: `src/theme/mod.rs`

- [ ] **Step 1: Extend the existing `pub use` line**

The current top of `src/theme/mod.rs` includes:

```rust
mod loader;
pub use loader::{apply_overrides, load_or_default};
```

Edit that line to:

```rust
mod loader;
pub use loader::{apply_overrides, dump_embedded, list_available, load_or_default, Source, ThemeListing};
```

- [ ] **Step 2: Verify the build is clean**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: clean build.

- [ ] **Step 3: Verify all tests pass**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 214 passed.

- [ ] **Step 4: Commit**

```bash
git add src/theme/mod.rs
git commit -m "feat(theme): re-export list_available, dump_embedded, Source, ThemeListing

Crate API surface for the upcoming lib.rs CLI handlers."
```

---

## Task 5: Wire `--list-themes` early-return handler in `lib.rs`

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Locate the existing early-return block**

Open `src/lib.rs`. Find the block (around lines 90–106) that handles `--version`, `--update`, and `--setup`. It looks like:

```rust
pub fn run() -> io::Result<()> {
    // --version / -V flag: print version and exit
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("abtop {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // --update flag: self-update via GitHub releases installer
    if std::env::args().any(|a| a == "--update") {
        return run_update();
    }

    // --setup flag: configure StatusLine hook and exit
    if std::env::args().any(|a| a == "--setup") {
        setup::run_setup();
        return Ok(());
    }

    // Load config once; …
```

- [ ] **Step 2: Add the `--list-themes` block**

Insert this block immediately after the `--setup` block and before the `// Load config once;` line:

```rust
    // --list-themes flag: print available themes and exit
    if std::env::args().any(|a| a == "--list-themes") {
        let listings = theme::list_available(&config::xdg_config_dir());
        for l in listings {
            let source_str = match l.source {
                theme::Source::Builtin => "built-in",
                theme::Source::User => "user",
                theme::Source::UserOverride => "user override",
            };
            println!("{} ({})", l.name, source_str);
        }
        return Ok(());
    }
```

- [ ] **Step 3: Verify the build is clean**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: clean build.

- [ ] **Step 4: Run the full test suite (no behavior regression)**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 214 passed.

- [ ] **Step 5: Manual smoke test with the staged binary**

```bash
./target/release/abtop --list-themes
```

Expected output: 13 lines, all ending with `(built-in)`, in BUILTIN order:

```
btop (built-in)
dracula (built-in)
catppuccin (built-in)
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
```

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs
git commit -m "feat(cli): add --list-themes early-return flag

Prints embedded + user-dir themes with source markers
(built-in / user / user override). Exits 0; does not load config
or build the app."
```

---

## Task 6: Wire `--dump-theme [--force]` early-return handler in `lib.rs`

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Add the `--dump-theme` block**

Insert this block immediately after the `--list-themes` block from Task 5 (still before `// Load config once;`):

```rust
    // --dump-theme <name> [--force] flag: write an embedded theme body to the
    // user themes dir so it can be edited in place. Exits without loading
    // the config or building the app.
    if let Some(pos) = std::env::args().position(|a| a == "--dump-theme") {
        let val = std::env::args().nth(pos + 1);
        let name = match val {
            Some(n) if !n.starts_with('-') => n,
            _ => {
                eprintln!("--dump-theme requires a theme name");
                let available: Vec<&str> = theme::THEME_NAMES.iter().copied().collect();
                eprintln!("available: {}", available.join(", "));
                std::process::exit(1);
            }
        };
        let force = std::env::args().any(|a| a == "--force");
        match theme::dump_embedded(&config::xdg_config_dir(), &name, force) {
            Ok(path) => {
                println!("wrote {}", path.display());
                return Ok(());
            }
            Err(msg) => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        }
    }
```

- [ ] **Step 2: Verify the build is clean**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: clean build.

- [ ] **Step 3: Run the full test suite**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 214 passed.

- [ ] **Step 4: Manual smoke test — happy path**

Use a scratch XDG dir to avoid polluting the real `~/.config/abtop/themes/`:

```bash
SCRATCH=$(mktemp -d)
XDG_CONFIG_HOME="$SCRATCH" ./target/release/abtop --dump-theme catppuccin
```

Expected output: `wrote /tmp/.../abtop/themes/catppuccin.theme` (path may vary).

Verify the file matches the embedded body:

```bash
diff "$SCRATCH/abtop/themes/catppuccin.theme" themes/catppuccin.theme
```

Expected: no output (files identical).

- [ ] **Step 5: Manual smoke test — refuse-on-exists**

```bash
XDG_CONFIG_HOME="$SCRATCH" ./target/release/abtop --dump-theme catppuccin
echo "exit: $?"
```

Expected: stderr contains `already exists. Re-run with --force to overwrite.`; exit code is 1.

- [ ] **Step 6: Manual smoke test — `--force` overwrite**

```bash
XDG_CONFIG_HOME="$SCRATCH" ./target/release/abtop --dump-theme catppuccin --force
echo "exit: $?"
```

Expected: `wrote ...`; exit code is 0.

- [ ] **Step 7: Manual smoke test — unknown theme**

```bash
XDG_CONFIG_HOME="$SCRATCH" ./target/release/abtop --dump-theme nonexistent
echo "exit: $?"
```

Expected: stderr contains `'nonexistent' is not an embedded theme`; exit code is 1.

- [ ] **Step 8: Manual smoke test — missing arg**

```bash
./target/release/abtop --dump-theme
echo "exit: $?"
```

Expected: stderr contains `--dump-theme requires a theme name` and the available list; exit code is 1.

- [ ] **Step 9: Manual smoke test — path traversal**

```bash
XDG_CONFIG_HOME="$SCRATCH" ./target/release/abtop --dump-theme ../evil
echo "exit: $?"
```

Expected: stderr contains `invalid theme name '../evil'`; exit code is 1.

- [ ] **Step 10: Clean up the scratch dir**

```bash
rm -rf "$SCRATCH"
```

- [ ] **Step 11: Commit**

```bash
git add src/lib.rs
git commit -m "feat(cli): add --dump-theme <name> [--force] early-return flag

Writes the embedded body of <name> to \$XDG_CONFIG_HOME/abtop/themes/
<name>.theme. Refuses to overwrite without --force. Rejects
path-traversal names with the same guard as theme resolution."
```

---

## Task 7: Document the new flags in README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Locate the existing Theming section**

Open `README.md`. Find the `## Theming` heading (around line 71) and the `### Custom themes` subsection that follows. The new content goes between the `### Custom themes` block and the `### Transparent background` block.

- [ ] **Step 2: Add a "### Discovering and editing themes" subsection**

Insert this content immediately after the existing `### Custom themes` block (right before `### Transparent background`):

```markdown
### Discovering and editing themes

List all themes available right now — embedded plus anything you've dropped
into `$XDG_CONFIG_HOME/abtop/themes/`:

```sh
$ abtop --list-themes
btop (built-in)
dracula (built-in)
catppuccin (built-in)
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
```

If a user file shadows an embedded theme of the same name, the embedded
entry is marked `(user override)`. User-only themes (no embedded counterpart)
are appended at the bottom and marked `(user)`.

To edit one of the built-in themes, dump its body to the user dir first:

```sh
$ abtop --dump-theme catppuccin
wrote /home/me/.config/abtop/themes/catppuccin.theme

$ $EDITOR ~/.config/abtop/themes/catppuccin.theme
# tweak away; user file now overrides the embedded version
```

`--dump-theme` refuses to overwrite an existing file. Pass `--force` to
overwrite. Only embedded themes can be dumped (user-only themes are
already on disk).
```

**IMPORTANT — code fence handling:** the inner triple-backtick blocks (`\`\`\`sh`) inside the markdown block above need to render as real fenced code in the final README, not as nested fence tokens. To insert them correctly, use the Edit tool with the actual text as `old_string` placeholder and the actual fenced text as `new_string`. The "outer" backticks shown in this plan are conceptual — write only the real, leaf-level fences into the README.

In other words, what should end up in `README.md` is:

```
### Discovering and editing themes

List all themes available right now — embedded plus anything you've dropped
into `$XDG_CONFIG_HOME/abtop/themes/`:

(then a real ```sh code fence containing the abtop --list-themes example)

If a user file shadows an embedded theme of the same name...

(then a real ```sh code fence containing the --dump-theme example)

`--dump-theme` refuses to overwrite an existing file...
```

- [ ] **Step 3: Verify the markdown renders cleanly**

```bash
head -130 README.md
```

Eyeball: section heading is `### Discovering and editing themes`; both inner code blocks are valid (open with ```\`\`\`sh```, close with ```\`\`\````); no leftover artifacts from the plan's nested-fence note.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document --list-themes and --dump-theme in README"
```

---

## Task 8: Build, install, and final smoke test

**Files:** none (build + install)

- [ ] **Step 1: Run the full test suite one more time**

```bash
cargo test --lib --quiet 2>&1 | tail -3
```

Expected: 214 passed.

- [ ] **Step 2: Build release**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: `Finished` with no errors.

- [ ] **Step 3: Install**

```bash
install -m 755 target/release/abtop ~/.local/libexec/abtop
```

- [ ] **Step 4: Smoke test the installed binary — list**

```bash
abtop --list-themes
```

Expected: 13 lines (assuming no user themes in `~/.config/abtop/themes/`), all `(built-in)`. If you currently have a user theme there, you'll see additional `(user)` lines.

- [ ] **Step 5: Smoke test the installed binary — dump, into real config**

```bash
abtop --dump-theme catppuccin --force
```

Expected: `wrote /Users/a.salvi/.config/abtop/themes/catppuccin.theme`. The `--force` is fine here because we're seeding a real edit-target.

- [ ] **Step 6: Smoke test the installed binary — list with override**

```bash
abtop --list-themes | head -5
```

Expected: the `catppuccin` line is now `(user override)`.

- [ ] **Step 7: Remove the demo override**

```bash
rm ~/.config/abtop/themes/catppuccin.theme
abtop --list-themes | grep catppuccin
```

Expected: `catppuccin (built-in)` and `catppuccin-transparent (built-in)`.

- [ ] **Step 8: No commit needed — install is a side effect**

The binary installed at `~/.local/libexec/abtop` is on PATH (via env-osx.zsh:114). All Phase B1 commits are already on `main`.

---

## Acceptance criteria

All of the following must hold for Phase B1 to be complete:

1. `cargo test --lib` reports 214 passed (up from 203 at the start).
2. `cargo build --release` produces a working binary that runs without panicking on `--list-themes` and `--dump-theme`.
3. `abtop --list-themes` prints the 13 embedded themes plus any user-dir entries, with correct source markers (`built-in`, `user override`, `user`).
4. `abtop --dump-theme catppuccin` writes `~/.config/abtop/themes/catppuccin.theme`.
5. `abtop --dump-theme catppuccin` (without `--force`) on an existing file errors with exit code 1.
6. `abtop --dump-theme catppuccin --force` overwrites cleanly.
7. `abtop --dump-theme nonexistent` errors with exit code 1.
8. `abtop --dump-theme ../evil` errors with exit code 1 and never touches the filesystem.
9. `~/.local/libexec/abtop` is the installed binary.

## Out of scope (other Phase B items)

- `t`-cycle picks up user-dir themes (Phase B2 — separate spec).
- Banner UI on malformed theme file (Phase B4 — separate spec).
- `--theme <absolute-path>` (Phase B3 — separate spec).
- Reload-on-file-change (Phase B5 — separate spec).
- macOS `~/Library/Application Support/abtop/` → XDG migration (Phase B6 — deferred indefinitely).
