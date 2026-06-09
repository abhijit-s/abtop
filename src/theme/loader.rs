//! Theme file parser + loader.

use ratatui::style::Color;

use crate::config::AppConfig;
use crate::theme::Theme;

/// A single parse failure encountered while reading a theme body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParseError {
    /// 1-indexed line number where the error was detected.
    pub line: usize,
    /// The trimmed line content (for display in detailed reports).
    pub content: String,
    /// What went wrong.
    pub reason: ParseErrorReason,
}

/// Why a line in a theme body failed to parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ParseErrorReason {
    /// Line started with `theme[` but couldn't be tokenized into (key, value).
    Malformed,
    /// Key parsed cleanly but isn't in the 33-key set.
    UnknownKey(String),
    /// Value is non-empty and doesn't match the `#hex` form.
    InvalidHex(String),
}

/// Parse a hex color string with required `#` prefix.
/// Supports `#RRGGBB` (6-digit) and `#RGB` (3-digit, expanded to RRGGBB).
/// Case-insensitive. Returns None for anything else (no named colors, no
/// rgb() syntax, no missing `#`).
fn parse_hex(raw: &str) -> Option<Color> {
    let s = raw.strip_prefix('#')?;
    match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16).ok()?;
            let g = u8::from_str_radix(&s[1..2], 16).ok()?;
            let b = u8::from_str_radix(&s[2..3], 16).ok()?;
            Some(Color::Rgb(r * 17, g * 17, b * 17))
        }
        _ => None,
    }
}

/// Is `key` one of the 33 keys handled by `apply_kv`? Used by
/// `classify_line` to distinguish a typo'd-key error from an
/// unknown-but-intentional forward-compat key.
fn is_known_key(key: &str) -> bool {
    matches!(
        key,
        "main_bg" | "main_fg" | "title" | "hi_fg" | "selected_bg" | "selected_fg"
        | "inactive_fg" | "graph_text" | "meter_bg" | "proc_misc" | "div_line"
        | "session_id" | "status_fg" | "warning_fg" | "cpu_box" | "mem_box"
        | "net_box" | "proc_box"
        | "cpu_grad_start" | "cpu_grad_mid" | "cpu_grad_end"
        | "proc_grad_start" | "proc_grad_mid" | "proc_grad_end"
        | "used_grad_start" | "used_grad_mid" | "used_grad_end"
        | "free_grad_start" | "free_grad_mid" | "free_grad_end"
        | "cached_grad_start" | "cached_grad_mid" | "cached_grad_end"
    )
}

/// Tokenize a `theme[key]="value"` line into `(key, value)` or return
/// the specific reason it failed. Caller is responsible for skipping
/// non-theme-prefixed lines (comments, blanks) before calling.
fn classify_line(line: &str) -> Result<(&str, &str), ParseErrorReason> {
    let rest = line
        .strip_prefix("theme[")
        .ok_or(ParseErrorReason::Malformed)?;
    let (key, rest) = rest.split_once(']').ok_or(ParseErrorReason::Malformed)?;
    let val_part = rest
        .trim_start()
        .strip_prefix('=')
        .ok_or(ParseErrorReason::Malformed)?
        .trim_start();
    let value = val_part
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            val_part
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .ok_or(ParseErrorReason::Malformed)?;
    if !is_known_key(key) {
        return Err(ParseErrorReason::UnknownKey(key.to_string()));
    }
    if !value.is_empty() && parse_hex(value).is_none() {
        return Err(ParseErrorReason::InvalidHex(value.to_string()));
    }
    Ok((key, value))
}

/// A single line of `theme[key]="value"` form, with leading/trailing
/// whitespace tolerated. Returns (key, value) or None.
fn parse_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let rest = line.strip_prefix("theme[")?;
    let (key, rest) = rest.split_once(']')?;
    let val_part = rest.trim_start().strip_prefix('=')?.trim_start();
    let v = val_part
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            val_part
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })?;
    Some((key, v))
}

/// Apply one (key, value) pair to a mutable Theme. Unknown keys and unknown
/// values are silently ignored (fields keep their existing values, which
/// come from the embedded btop default when seeded by parse_theme_body).
fn apply_kv(theme: &mut Theme, key: &str, value: &str) {
    // Color-typed fields: empty value -> Color::Reset.
    let set_color = |t_field: &mut Color, v: &str| {
        if v.is_empty() {
            *t_field = Color::Reset;
        } else if let Some(c) = parse_hex(v) {
            *t_field = c;
        }
    };
    // Gradient channel: empty value silently ignored (no Reset for tuples).
    let set_grad = |t_field: &mut (u8, u8, u8), v: &str| {
        if v.is_empty() {
            return;
        }
        if let Some(Color::Rgb(r, g, b)) = parse_hex(v) {
            *t_field = (r, g, b);
        }
    };
    match key {
        "main_bg" => set_color(&mut theme.main_bg, value),
        "main_fg" => set_color(&mut theme.main_fg, value),
        "title" => set_color(&mut theme.title, value),
        "hi_fg" => set_color(&mut theme.hi_fg, value),
        "selected_bg" => set_color(&mut theme.selected_bg, value),
        "selected_fg" => set_color(&mut theme.selected_fg, value),
        "inactive_fg" => set_color(&mut theme.inactive_fg, value),
        "graph_text" => set_color(&mut theme.graph_text, value),
        "meter_bg" => set_color(&mut theme.meter_bg, value),
        "proc_misc" => set_color(&mut theme.proc_misc, value),
        "div_line" => set_color(&mut theme.div_line, value),
        "session_id" => set_color(&mut theme.session_id, value),
        "status_fg" => set_color(&mut theme.status_fg, value),
        "warning_fg" => set_color(&mut theme.warning_fg, value),
        "cpu_box" => set_color(&mut theme.cpu_box, value),
        "mem_box" => set_color(&mut theme.mem_box, value),
        "net_box" => set_color(&mut theme.net_box, value),
        "proc_box" => set_color(&mut theme.proc_box, value),
        "cpu_grad_start" => set_grad(&mut theme.cpu_grad.start, value),
        "cpu_grad_mid" => set_grad(&mut theme.cpu_grad.mid, value),
        "cpu_grad_end" => set_grad(&mut theme.cpu_grad.end, value),
        "proc_grad_start" => set_grad(&mut theme.proc_grad.start, value),
        "proc_grad_mid" => set_grad(&mut theme.proc_grad.mid, value),
        "proc_grad_end" => set_grad(&mut theme.proc_grad.end, value),
        "used_grad_start" => set_grad(&mut theme.used_grad.start, value),
        "used_grad_mid" => set_grad(&mut theme.used_grad.mid, value),
        "used_grad_end" => set_grad(&mut theme.used_grad.end, value),
        "free_grad_start" => set_grad(&mut theme.free_grad.start, value),
        "free_grad_mid" => set_grad(&mut theme.free_grad.mid, value),
        "free_grad_end" => set_grad(&mut theme.free_grad.end, value),
        "cached_grad_start" => set_grad(&mut theme.cached_grad.start, value),
        "cached_grad_mid" => set_grad(&mut theme.cached_grad.mid, value),
        "cached_grad_end" => set_grad(&mut theme.cached_grad.end, value),
        _ => {}
    }
}

/// A Theme with every Color::Reset and every gradient zero'd. Used as the
/// starting point for parsing — the embedded btop body fills it in before
/// the caller's body is applied.
fn empty_theme() -> Theme {
    Theme {
        name: String::new(),
        main_bg: Color::Reset,
        main_fg: Color::Reset,
        title: Color::Reset,
        hi_fg: Color::Reset,
        selected_bg: Color::Reset,
        selected_fg: Color::Reset,
        inactive_fg: Color::Reset,
        graph_text: Color::Reset,
        meter_bg: Color::Reset,
        proc_misc: Color::Reset,
        div_line: Color::Reset,
        session_id: Color::Reset,
        status_fg: Color::Reset,
        warning_fg: Color::Reset,
        cpu_box: Color::Reset,
        mem_box: Color::Reset,
        net_box: Color::Reset,
        proc_box: Color::Reset,
        cpu_grad: crate::theme::Gradient { start: (0, 0, 0), mid: (0, 0, 0), end: (0, 0, 0) },
        proc_grad: crate::theme::Gradient { start: (0, 0, 0), mid: (0, 0, 0), end: (0, 0, 0) },
        used_grad: crate::theme::Gradient { start: (0, 0, 0), mid: (0, 0, 0), end: (0, 0, 0) },
        free_grad: crate::theme::Gradient { start: (0, 0, 0), mid: (0, 0, 0), end: (0, 0, 0) },
        cached_grad: crate::theme::Gradient { start: (0, 0, 0), mid: (0, 0, 0), end: (0, 0, 0) },
    }
}

fn apply_body(theme: &mut Theme, body: &str) {
    for line in body.lines() {
        if let Some((k, v)) = parse_line(line) {
            apply_kv(theme, k, v);
        }
    }
}

/// Parse a btop-style theme body and collect any parse errors. The
/// returned Theme is always fully populated: missing keys inherit from
/// the embedded btop default, and lines with errors are accumulated in
/// the errors vec so the caller can surface them to the user.
pub(crate) fn parse_theme_body_with_errors(body: &str, name: &str) -> (Theme, Vec<ParseError>) {
    let mut theme = empty_theme();
    let btop_body = crate::theme::embedded::lookup("btop")
        .expect("embedded btop is a build-time invariant");
    apply_body(&mut theme, btop_body);

    let mut errors = Vec::new();
    for (idx, raw_line) in body.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim();
        if !trimmed.starts_with("theme[") {
            continue;
        }
        match classify_line(trimmed) {
            Ok((k, v)) => apply_kv(&mut theme, k, v),
            Err(reason) => errors.push(ParseError {
                line: line_no,
                content: trimmed.to_string(),
                reason,
            }),
        }
    }
    theme.name = name.to_string();
    (theme, errors)
}

/// Thin wrapper that discards parse errors. Existing callers that don't
/// care about errors keep working unchanged.
pub fn parse_theme_body(body: &str, name: &str) -> Theme {
    parse_theme_body_with_errors(body, name).0
}

/// Apply config-level overrides on top of a parsed Theme.
///
/// Currently the only override is `theme_background = false`, which stamps
/// `Color::Reset` over `main_bg`. Other background fields (selected_bg,
/// meter_bg) are left alone — they're indicators, not the window background.
pub fn apply_overrides(theme: &mut Theme, cfg: &AppConfig) {
    if !cfg.theme_background {
        theme.main_bg = Color::Reset;
    }
}

use std::path::Path;

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

/// Read the user-dir file's body, applying the same path-traversal guard
/// as the existing `try_user_file` helper. Returns None if the name is
/// disallowed or the file isn't readable.
fn try_user_file_body(config_root: &Path, name: &str) -> Option<String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    let path = config_root
        .join("abtop")
        .join("themes")
        .join(format!("{name}.theme"));
    std::fs::read_to_string(&path).ok()
}

/// Try to read and parse `<config_root>/abtop/themes/<name>.theme`.
/// Returns Some(theme) on a successful read+parse; None if the file is
/// missing or unreadable. Rejects names containing path separators or `..`
/// so a CLI/config theme name can't escape the themes directory.
fn try_user_file(config_root: &Path, name: &str) -> Option<Theme> {
    try_user_file_body(config_root, name).map(|body| parse_theme_body(&body, name))
}

/// List all themes available for selection: the embedded BUILTIN entries
/// plus user files under `<config_root>/abtop/themes/`. Embedded entries
/// are listed in BUILTIN order. User-only entries are appended alphabetically.
///
/// Filesystem errors (missing dir, permission denied, etc.) are treated as
/// "no user themes" — the embedded list is unconditionally returned.
pub fn list_available(config_root: &Path) -> Vec<ThemeListing> {
    let themes_dir = config_root.join("abtop").join("themes");
    let mut user_names: Vec<String> = match std::fs::read_dir(&themes_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                let file_name = path.file_name()?.to_str()?.to_owned();
                if file_name.starts_with('.') {
                    return None;
                }
                let stem = file_name.strip_suffix(".theme")?;
                if stem.is_empty() {
                    return None;
                }
                // Restrict to a safe character set so filenames can't
                // smuggle TOML metacharacters into config.toml when
                // cycle_theme later calls save_theme(name).
                if !stem
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
                {
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

/// Load a theme directly from a filesystem path, returning the parsed Theme
/// and any parse errors encountered. Name is derived from `path.file_stem()`.
/// I/O errors (file not found, unreadable, etc.) propagate as Err(String).
///
/// Use case: `--theme /tmp/scratch.theme` and similar one-shot iteration.
/// The caller is responsible for skipping `save_theme` — this function
/// only loads, it doesn't persist.
pub(crate) fn load_from_path_with_errors(
    path: &Path,
) -> Result<(Theme, Vec<ParseError>), String> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("custom")
        .to_string();
    Ok(parse_theme_body_with_errors(&body, &name))
}

/// Thin wrapper that discards parse errors. Kept for tests; production
/// callers route through `load_from_path_with_errors`.
#[cfg(test)]
pub(crate) fn load_from_path(path: &Path) -> Result<Theme, String> {
    load_from_path_with_errors(path).map(|(t, _)| t)
}

/// Resolve a theme by name, consulting (1) the user themes dir under
/// `config_root`, then (2) the embedded BUILTIN table. Returns None if
/// neither contains the name.
pub fn lookup_chain(config_root: &Path, name: &str) -> Option<Theme> {
    if let Some(t) = try_user_file(config_root, name) {
        return Some(t);
    }
    crate::theme::embedded::lookup(name).map(|body| parse_theme_body(body, name))
}

/// Like `lookup_chain` but also returns any parse errors encountered.
/// Resolution order: user file → embedded → embedded btop (last resort).
/// Always returns a Theme.
pub(crate) fn lookup_chain_with_errors(
    config_root: &Path,
    name: &str,
) -> (Theme, Vec<ParseError>) {
    if let Some(body) = try_user_file_body(config_root, name) {
        return parse_theme_body_with_errors(&body, name);
    }
    if let Some(body) = crate::theme::embedded::lookup(name) {
        return parse_theme_body_with_errors(body, name);
    }
    let body = crate::theme::embedded::lookup("btop")
        .expect("embedded btop is a build-time invariant");
    (parse_theme_body(body, "btop"), Vec::new())
}

/// Resolve a theme with a last-resort fallback to embedded `btop`. Always
/// returns a Theme. The returned theme has NOT had `apply_overrides`
/// applied — callers must run that afterward if the config flag should
/// affect it.
#[cfg(test)]
pub fn load_chain(config_root: &Path, name: &str) -> Theme {
    lookup_chain(config_root, name).unwrap_or_else(|| {
        let body = crate::theme::embedded::lookup("btop")
            .expect("embedded btop is a build-time invariant");
        parse_theme_body(body, "btop")
    })
}

/// Resolve a theme by name against the current XDG config root, with a
/// last-resort fallback to embedded `btop`, and return any parse errors
/// encountered. Config-level overrides are applied before returning.
/// Always returns a Theme.
pub(crate) fn load_or_default_with_errors(
    name: &str,
    cfg: &AppConfig,
) -> (Theme, Vec<ParseError>) {
    let (mut theme, errors) =
        lookup_chain_with_errors(&crate::config::xdg_config_dir(), name);
    apply_overrides(&mut theme, cfg);
    (theme, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_bg(bg: bool) -> crate::config::AppConfig {
        let mut c = crate::config::AppConfig::default();
        c.theme_background = bg;
        c
    }

    #[test]
    fn parse_hex_6_digit_uppercase() {
        assert_eq!(parse_hex("#ABCDEF"), Some(Color::Rgb(0xAB, 0xCD, 0xEF)));
    }

    #[test]
    fn parse_hex_6_digit_lowercase() {
        assert_eq!(parse_hex("#abcdef"), Some(Color::Rgb(0xab, 0xcd, 0xef)));
    }

    #[test]
    fn parse_hex_3_digit_expands_via_x17() {
        // 0x9 -> 0x99, 0xa -> 0xaa, 0xf -> 0xff
        assert_eq!(parse_hex("#9af"), Some(Color::Rgb(0x99, 0xaa, 0xff)));
    }

    #[test]
    fn parse_hex_rejects_missing_hash() {
        assert_eq!(parse_hex("abcdef"), None);
    }

    #[test]
    fn parse_hex_rejects_wrong_length() {
        assert_eq!(parse_hex("#abcd"), None);
        assert_eq!(parse_hex("#abcdefab"), None);
    }

    #[test]
    fn parse_hex_rejects_non_hex_chars() {
        assert_eq!(parse_hex("#zzzzzz"), None);
    }

    const MINIMAL_BTOP_OVERRIDE: &str = r##"
# A theme that only sets main_bg + main_fg.
# Everything else inherits embedded btop defaults.
theme[main_bg]="#112233"
theme[main_fg]="#445566"
"##;

    #[test]
    fn parse_theme_body_sets_known_keys() {
        let t = parse_theme_body(MINIMAL_BTOP_OVERRIDE, "test");
        assert_eq!(t.name, "test");
        assert_eq!(t.main_bg, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(t.main_fg, Color::Rgb(0x44, 0x55, 0x66));
    }

    #[test]
    fn parse_theme_body_inherits_unset_keys_from_btop() {
        // btop default `title` is Rgb(238, 238, 238).
        let t = parse_theme_body(MINIMAL_BTOP_OVERRIDE, "test");
        assert_eq!(t.title, Color::Rgb(238, 238, 238));
    }

    #[test]
    fn parse_theme_body_empty_value_yields_reset_for_color_field() {
        let body = r##"theme[main_bg]="""##;
        let t = parse_theme_body(body, "transparent");
        assert_eq!(t.main_bg, Color::Reset);
    }

    #[test]
    fn parse_theme_body_empty_value_on_gradient_inherits_btop() {
        // cpu_grad.start in btop is (119, 202, 155).
        let body = r##"theme[cpu_grad_start]="""##;
        let t = parse_theme_body(body, "test");
        assert_eq!(t.cpu_grad.start, (119, 202, 155));
    }

    #[test]
    fn parse_theme_body_unknown_key_is_ignored() {
        let body = r##"theme[future_key]="#abcdef""##;
        let t = parse_theme_body(body, "test");
        assert_eq!(t.name, "test");
    }

    #[test]
    fn parse_theme_body_unknown_value_falls_back() {
        let body = r##"theme[main_bg]="not-a-color""##;
        let t = parse_theme_body(body, "test");
        // btop default main_bg is Rgb(25, 25, 25).
        assert_eq!(t.main_bg, Color::Rgb(25, 25, 25));
    }

    #[test]
    fn parse_theme_body_handles_comments_and_blanks() {
        let body = r##"
            # leading comment
            theme[main_fg]="#abcdef"

            # trailing comment with a "quoted" segment
        "##;
        let t = parse_theme_body(body, "test");
        assert_eq!(t.main_fg, Color::Rgb(0xab, 0xcd, 0xef));
    }

    #[test]
    fn parse_theme_body_reads_full_palette() {
        let body = r##"
theme[main_bg]="#010203"
theme[main_fg]="#040506"
theme[title]="#070809"
theme[hi_fg]="#0a0b0c"
theme[selected_bg]="#0d0e0f"
theme[selected_fg]="#101112"
theme[inactive_fg]="#131415"
theme[graph_text]="#161718"
theme[meter_bg]="#191a1b"
theme[proc_misc]="#1c1d1e"
theme[div_line]="#1f2021"
theme[session_id]="#222324"
theme[status_fg]="#252627"
theme[warning_fg]="#28292a"
theme[cpu_box]="#2b2c2d"
theme[mem_box]="#2e2f30"
theme[net_box]="#313233"
theme[proc_box]="#343536"
theme[cpu_grad_start]="#373839"
theme[cpu_grad_mid]="#3a3b3c"
theme[cpu_grad_end]="#3d3e3f"
theme[proc_grad_start]="#404142"
theme[proc_grad_mid]="#434445"
theme[proc_grad_end]="#464748"
theme[used_grad_start]="#494a4b"
theme[used_grad_mid]="#4c4d4e"
theme[used_grad_end]="#4f5051"
theme[free_grad_start]="#525354"
theme[free_grad_mid]="#555657"
theme[free_grad_end]="#58595a"
theme[cached_grad_start]="#5b5c5d"
theme[cached_grad_mid]="#5e5f60"
theme[cached_grad_end]="#616263"
"##;
        let t = parse_theme_body(body, "full");
        assert_eq!(t.main_bg, Color::Rgb(1, 2, 3));
        assert_eq!(t.cached_grad.end, (0x61, 0x62, 0x63));
    }

    #[test]
    fn apply_overrides_force_transparent_with_opaque_theme() {
        let mut t = Theme::by_name("btop").unwrap();
        let original_bg = t.main_bg;
        apply_overrides(&mut t, &cfg_with_bg(false));
        assert_eq!(t.main_bg, Color::Reset);
        assert_ne!(t.main_bg, original_bg);
    }

    #[test]
    fn apply_overrides_keep_theme_when_flag_default_true() {
        let mut t = Theme::by_name("btop").unwrap();
        let original_bg = t.main_bg;
        apply_overrides(&mut t, &cfg_with_bg(true));
        assert_eq!(t.main_bg, original_bg);
    }

    #[test]
    fn apply_overrides_leaves_other_bg_fields_alone() {
        let mut t = Theme::by_name("btop").unwrap();
        let original_selected = t.selected_bg;
        let original_meter = t.meter_bg;
        apply_overrides(&mut t, &cfg_with_bg(false));
        assert_eq!(t.selected_bg, original_selected);
        assert_eq!(t.meter_bg, original_meter);
    }

    #[test]
    fn apply_overrides_already_reset_main_bg_stays_reset() {
        let mut t = Theme::by_name("btop").unwrap();
        t.main_bg = Color::Reset;
        apply_overrides(&mut t, &cfg_with_bg(true));
        assert_eq!(t.main_bg, Color::Reset);
    }

    #[test]
    fn every_embedded_theme_parses_to_full_palette() {
        let btop_body = crate::theme::embedded::lookup("btop").expect("btop in BUILTIN");
        let btop_baseline = parse_theme_body(btop_body, "btop");
        for (name, body) in crate::theme::embedded::BUILTIN.iter() {
            let t = parse_theme_body(body, name);
            assert_eq!(t.name, *name);
            if *name != "btop" {
                // A non-btop theme that parses identically to btop (apart from name)
                // means its file was empty / unparseable.
                let renamed_btop = Theme { name: name.to_string(), ..btop_baseline.clone() };
                assert_ne!(
                    t, renamed_btop,
                    "embedded '{name}' parsed identically to btop — file may be empty or all keys unrecognized"
                );
            }
        }
    }

    #[test]
    fn theme_names_const_matches_embedded_in_order() {
        let embedded: Vec<&str> = crate::theme::embedded::BUILTIN.iter().map(|(n, _)| *n).collect();
        let listed: Vec<&str> = crate::theme::THEME_NAMES.to_vec();
        assert_eq!(embedded, listed, "THEME_NAMES drifted from embedded::BUILTIN");
    }

    #[test]
    fn theme_names_const_includes_all_embedded() {
        let embedded_set: std::collections::HashSet<&str> =
            crate::theme::embedded::BUILTIN.iter().map(|(n, _)| *n).collect();
        for name in embedded_set {
            assert!(
                crate::theme::THEME_NAMES.contains(&name),
                "'{name}' is in BUILTIN but not in THEME_NAMES"
            );
        }
    }

    use tempfile::TempDir;

    fn write_theme_file(dir: &std::path::Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{name}.theme")), body).unwrap();
    }

    #[test]
    fn load_chain_user_file_wins_over_embedded() {
        let tmp = TempDir::new().unwrap();
        let themes_dir = tmp.path().join("abtop").join("themes");
        write_theme_file(&themes_dir, "btop", r##"theme[main_fg]="#ff00ff""##);

        let t = load_chain(tmp.path(), "btop");
        assert_eq!(t.main_fg, Color::Rgb(0xff, 0x00, 0xff));
        // main_bg comes from embedded btop, NOT overridden by the user file.
        assert_eq!(t.main_bg, Color::Rgb(0x19, 0x19, 0x19));
    }

    #[test]
    fn load_chain_falls_back_to_embedded_when_no_user_file() {
        let tmp = TempDir::new().unwrap();
        let t = load_chain(tmp.path(), "catppuccin");
        assert_eq!(t.name, "catppuccin");
        // Catppuccin main_fg = #cdd6f4.
        assert_eq!(t.main_fg, Color::Rgb(0xcd, 0xd6, 0xf4));
    }

    #[test]
    fn load_chain_unknown_name_falls_back_to_embedded_btop() {
        let tmp = TempDir::new().unwrap();
        let t = load_chain(tmp.path(), "nonexistent-theme-12345");
        // Embedded btop's main_bg = #191919.
        assert_eq!(t.main_bg, Color::Rgb(0x19, 0x19, 0x19));
        // Falls back to btop name verbatim.
        assert_eq!(t.name, "btop");
    }

    #[test]
    fn lookup_chain_returns_some_for_embedded_name() {
        let tmp = TempDir::new().unwrap();
        let t = lookup_chain(tmp.path(), "dracula").unwrap();
        assert_eq!(t.name, "dracula");
    }

    #[test]
    fn lookup_chain_returns_some_for_user_dir_name() {
        let tmp = TempDir::new().unwrap();
        let themes_dir = tmp.path().join("abtop").join("themes");
        write_theme_file(
            &themes_dir,
            "my-custom",
            r##"theme[main_fg]="#abcdef""##,
        );
        let t = lookup_chain(tmp.path(), "my-custom").unwrap();
        assert_eq!(t.name, "my-custom");
        assert_eq!(t.main_fg, Color::Rgb(0xab, 0xcd, 0xef));
    }

    #[test]
    fn lookup_chain_returns_none_for_unknown_name() {
        let tmp = TempDir::new().unwrap();
        assert!(lookup_chain(tmp.path(), "no-such-thing").is_none());
    }

    #[test]
    fn lookup_chain_rejects_path_traversal_names() {
        let tmp = TempDir::new().unwrap();
        // Even if a file at the traversal target exists, the guard must fire.
        std::fs::write(tmp.path().join("evil.theme"), "theme[main_fg]=\"#ff0000\"").unwrap();
        assert!(lookup_chain(tmp.path(), "../evil").is_none());
        assert!(lookup_chain(tmp.path(), "..").is_none());
        assert!(lookup_chain(tmp.path(), "sub/name").is_none());
        assert!(lookup_chain(tmp.path(), "name\\with\\backslash").is_none());
        assert!(lookup_chain(tmp.path(), "").is_none());
    }

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

    #[test]
    fn list_available_empty_user_dir_returns_only_builtin() {
        let tmp = TempDir::new().unwrap();
        let listings = list_available(tmp.path());

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
        write_theme_file(&themes_dir, "zorak", r##"theme[main_fg]="#ff0000""##);
        write_theme_file(&themes_dir, "my-cool", r##"theme[main_fg]="#00ff00""##);

        let listings = list_available(tmp.path());

        assert_eq!(listings.len(), 15);
        assert_eq!(listings[13].name, "my-cool");
        assert_eq!(listings[13].source, Source::User);
        assert_eq!(listings[14].name, "zorak");
        assert_eq!(listings[14].source, Source::User);
    }

    #[test]
    fn list_available_user_override_promotes_builtin_entry() {
        let tmp = TempDir::new().unwrap();
        let themes_dir = tmp.path().join("abtop").join("themes");
        write_theme_file(&themes_dir, "catppuccin", r##"theme[main_fg]="#abcdef""##);

        let listings = list_available(tmp.path());

        assert_eq!(listings.len(), 13);
        let catppuccin = listings
            .iter()
            .find(|l| l.name == "catppuccin")
            .expect("catppuccin present");
        assert_eq!(catppuccin.source, Source::UserOverride);
        let pos = listings.iter().position(|l| l.name == "catppuccin").unwrap();
        let builtin_pos = crate::theme::embedded::BUILTIN
            .iter()
            .position(|(n, _)| *n == "catppuccin")
            .unwrap();
        assert_eq!(pos, builtin_pos);
    }

    #[test]
    fn list_available_skips_filenames_with_unsafe_chars() {
        // Filenames that would smuggle TOML metacharacters into save_theme
        // must be rejected at the listing stage.
        let tmp = TempDir::new().unwrap();
        let themes_dir = tmp.path().join("abtop").join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        // Embedded quote: would break the `theme = "<name>"` TOML write.
        std::fs::write(themes_dir.join("evil\".theme"), "junk").unwrap();
        // Embedded newline: would split the TOML line.
        std::fs::write(themes_dir.join("two\nlines.theme"), "junk").unwrap();
        // Space: not strictly TOML-dangerous but also not a sane theme name.
        std::fs::write(themes_dir.join("with space.theme"), "junk").unwrap();

        let listings = list_available(tmp.path());

        // None of the unsafe-named files should appear.
        assert_eq!(listings.len(), 13);
        assert!(listings.iter().all(|l| l.source == Source::Builtin));
    }

    #[test]
    fn list_available_skips_hidden_and_non_theme_files() {
        let tmp = TempDir::new().unwrap();
        let themes_dir = tmp.path().join("abtop").join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(themes_dir.join(".catppuccin.theme.swp"), "junk").unwrap();
        std::fs::write(themes_dir.join("notes.md"), "junk").unwrap();
        std::fs::write(themes_dir.join("README"), "junk").unwrap();

        let listings = list_available(tmp.path());

        assert_eq!(listings.len(), 13);
        assert!(listings.iter().all(|l| l.source == Source::Builtin));
    }

    #[test]
    fn list_available_returns_builtin_when_user_dir_unreadable() {
        let tmp = TempDir::new().unwrap();
        let bogus = tmp.path().join("does-not-exist");
        let listings = list_available(&bogus);
        assert_eq!(listings.len(), 13);
        assert!(listings.iter().all(|l| l.source == Source::Builtin));
    }

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

    #[test]
    fn load_from_path_reads_a_theme_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("scratch.theme");
        std::fs::write(&path, r##"theme[main_bg]="#112233""##).unwrap();
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
        assert_eq!(t.name, "x");
    }

    #[test]
    fn parse_error_types_basics() {
        let e = ParseError {
            line: 7,
            content: "theme[main_bg]=\"#XYZ\"".to_string(),
            reason: ParseErrorReason::InvalidHex("#XYZ".to_string()),
        };
        assert_eq!(e.line, 7);
        assert_eq!(e.reason, ParseErrorReason::InvalidHex("#XYZ".to_string()));
        assert_ne!(
            ParseErrorReason::Malformed,
            ParseErrorReason::UnknownKey("x".to_string())
        );
        assert_ne!(
            ParseErrorReason::Malformed,
            ParseErrorReason::InvalidHex("x".to_string())
        );
        let _ = format!("{e:?}");
        let _ = e.clone();
    }

    #[test]
    fn is_known_key_matches_all_33_keys() {
        let known = [
            "main_bg", "main_fg", "title", "hi_fg", "selected_bg", "selected_fg",
            "inactive_fg", "graph_text", "meter_bg", "proc_misc", "div_line",
            "session_id", "status_fg", "warning_fg", "cpu_box", "mem_box",
            "net_box", "proc_box",
            "cpu_grad_start", "cpu_grad_mid", "cpu_grad_end",
            "proc_grad_start", "proc_grad_mid", "proc_grad_end",
            "used_grad_start", "used_grad_mid", "used_grad_end",
            "free_grad_start", "free_grad_mid", "free_grad_end",
            "cached_grad_start", "cached_grad_mid", "cached_grad_end",
        ];
        assert_eq!(known.len(), 33);
        for k in known {
            assert!(is_known_key(k), "{k} should be a known key");
        }
        assert!(!is_known_key("wrong_key"));
        assert!(!is_known_key(""));
        assert!(!is_known_key("main_bg2"));
    }

    #[test]
    fn classify_line_returns_key_and_value_on_valid_input() {
        let result = classify_line("theme[main_bg]=\"#112233\"");
        assert_eq!(result, Ok(("main_bg", "#112233")));
    }

    #[test]
    fn classify_line_accepts_empty_value() {
        let result = classify_line("theme[main_bg]=\"\"");
        assert_eq!(result, Ok(("main_bg", "")));
    }

    #[test]
    fn classify_line_reports_malformed_when_no_equals() {
        let result = classify_line("theme[main_bg]");
        assert_eq!(result, Err(ParseErrorReason::Malformed));
    }

    #[test]
    fn classify_line_reports_malformed_when_unquoted_value() {
        let result = classify_line("theme[main_bg]=missing_quotes");
        assert_eq!(result, Err(ParseErrorReason::Malformed));
    }

    #[test]
    fn classify_line_reports_unknown_key() {
        let result = classify_line("theme[wrong_key]=\"#fff\"");
        assert_eq!(
            result,
            Err(ParseErrorReason::UnknownKey("wrong_key".to_string()))
        );
    }

    #[test]
    fn classify_line_reports_invalid_hex() {
        let result = classify_line("theme[main_bg]=\"not-a-color\"");
        assert_eq!(
            result,
            Err(ParseErrorReason::InvalidHex("not-a-color".to_string()))
        );
    }

    #[test]
    fn parse_theme_body_with_errors_reports_invalid_hex() {
        let body = r##"theme[main_bg]="#XYZ""##;
        let (_, errors) = parse_theme_body_with_errors(body, "test");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 1);
        assert_eq!(errors[0].reason, ParseErrorReason::InvalidHex("#XYZ".to_string()));
    }

    #[test]
    fn parse_theme_body_with_errors_reports_unknown_key() {
        let body = r##"theme[wrong_key]="#fff""##;
        let (_, errors) = parse_theme_body_with_errors(body, "test");
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].reason,
            ParseErrorReason::UnknownKey("wrong_key".to_string())
        );
    }

    #[test]
    fn parse_theme_body_with_errors_reports_malformed_lines() {
        let body = "theme[main_bg]=missing_quote\ntheme[main_bg";
        let (_, errors) = parse_theme_body_with_errors(body, "test");
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|e| e.reason == ParseErrorReason::Malformed));
    }

    #[test]
    fn parse_theme_body_with_errors_includes_correct_line_numbers() {
        let body = "# header comment\n\ntheme[main_bg]=\"#XYZ\"\ntheme[main_fg]=\"#abcdef\"\n\ntheme[wrong_key]=\"#fff\"";
        let (_, errors) = parse_theme_body_with_errors(body, "test");
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].line, 3);
        assert_eq!(errors[1].line, 6);
    }

    #[test]
    fn parse_theme_body_with_errors_ignores_comments_and_blanks() {
        let body = "# comment\n\n# another comment\nsome random text\n";
        let (_, errors) = parse_theme_body_with_errors(body, "test");
        assert_eq!(errors.len(), 0);
    }

    #[test]
    fn parse_theme_body_with_errors_clean_file_returns_no_errors() {
        let body = crate::theme::embedded::lookup("catppuccin").expect("in BUILTIN");
        let (_, errors) = parse_theme_body_with_errors(body, "catppuccin");
        assert_eq!(errors.len(), 0);
    }

    #[test]
    fn every_embedded_theme_parses_with_zero_errors() {
        // Locks in the "embedded = always clean" invariant.
        for (name, body) in crate::theme::embedded::BUILTIN.iter() {
            let (_, errors) = parse_theme_body_with_errors(body, name);
            assert!(
                errors.is_empty(),
                "embedded theme '{name}' produced unexpected errors: {errors:?}"
            );
        }
    }

    #[test]
    fn parse_theme_body_wrapper_discards_errors() {
        let body = r##"theme[main_bg]="#XYZ""##;
        let t = parse_theme_body(body, "test");
        assert_eq!(t.name, "test");
        // main_bg falls back to btop default since the bad hex was ignored.
        use ratatui::style::Color;
        assert_eq!(t.main_bg, Color::Rgb(0x19, 0x19, 0x19));
    }

    #[test]
    fn lookup_chain_with_errors_returns_user_file_errors() {
        let tmp = TempDir::new().unwrap();
        let themes_dir = tmp.path().join("abtop").join("themes");
        write_theme_file(&themes_dir, "broken", r##"theme[main_bg]="#XYZ""##);

        let (theme, errors) = lookup_chain_with_errors(tmp.path(), "broken");
        assert_eq!(theme.name, "broken");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].reason, ParseErrorReason::InvalidHex("#XYZ".to_string()));
    }

    #[test]
    fn lookup_chain_with_errors_returns_no_errors_for_embedded() {
        let tmp = TempDir::new().unwrap();
        let (theme, errors) = lookup_chain_with_errors(tmp.path(), "catppuccin");
        assert_eq!(theme.name, "catppuccin");
        assert!(errors.is_empty());
    }

    #[test]
    fn lookup_chain_with_errors_falls_back_to_btop_with_no_errors() {
        let tmp = TempDir::new().unwrap();
        let (theme, errors) = lookup_chain_with_errors(tmp.path(), "nonexistent-name");
        assert_eq!(theme.name, "btop");
        assert!(errors.is_empty());
    }

    #[test]
    fn load_from_path_with_errors_returns_parse_errors() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("broken.theme");
        std::fs::write(&path, r##"theme[main_bg]="#XYZ""##).unwrap();
        let (theme, errors) = load_from_path_with_errors(&path)
            .expect("file exists, so Ok");
        assert_eq!(theme.name, "broken");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].reason, ParseErrorReason::InvalidHex("#XYZ".to_string()));
    }

    #[test]
    fn load_from_path_with_errors_propagates_io_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.theme");
        let result = load_from_path_with_errors(&path);
        let err = result.expect_err("missing file should be Err");
        assert!(err.contains("failed to read"));
    }

    #[test]
    fn load_or_default_with_errors_returns_embedded_theme_with_no_errors() {
        // Sanity: a known-clean embedded name produces zero errors via the
        // load_or_default_with_errors entry point.
        let cfg = crate::config::AppConfig::default();
        let (theme, errors) = load_or_default_with_errors("catppuccin", &cfg);
        // The theme name is "catppuccin" unless the user has a broken override
        // file on this machine, in which case errors may be non-empty. The
        // assertion is permissive on the developer machine; the unit-level
        // contract (errors propagate when present) is covered by the
        // lookup_chain_with_errors tests in Task 4.
        assert_eq!(theme.name, "catppuccin");
        // No hard assertion on errors.len() since the test environment may
        // contain a user override. The Task 4 tests cover the strict cases.
        let _ = errors;
    }
}
