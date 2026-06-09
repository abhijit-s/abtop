//! Theme file parser + loader.

use ratatui::style::Color;

use crate::config::AppConfig;
use crate::theme::Theme;

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

/// Parse a btop-style theme body. Returns a fully-populated Theme: missing
/// keys are backfilled from the embedded btop default. The returned theme's
/// `name` is the caller-supplied `name`.
pub fn parse_theme_body(body: &str, name: &str) -> Theme {
    let mut theme = Theme::btop();
    theme.name = name.to_string();
    for line in body.lines() {
        if let Some((k, v)) = parse_line(line) {
            apply_kv(&mut theme, k, v);
        }
    }
    theme
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

/// Try to read and parse `<config_root>/abtop/themes/<name>.theme`.
/// Returns Some(theme) on a successful read+parse; None if the file is
/// missing or unreadable.
fn try_user_file(config_root: &Path, name: &str) -> Option<Theme> {
    let path = config_root
        .join("abtop")
        .join("themes")
        .join(format!("{name}.theme"));
    let body = std::fs::read_to_string(&path).ok()?;
    Some(parse_theme_body(&body, name))
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

/// Resolve a theme with a last-resort fallback to embedded `btop`. Always
/// returns a Theme. The returned theme has NOT had `apply_overrides`
/// applied — callers must run that afterward if the config flag should
/// affect it.
pub fn load_chain(config_root: &Path, name: &str) -> Theme {
    lookup_chain(config_root, name).unwrap_or_else(|| {
        let body = crate::theme::embedded::lookup("btop")
            .expect("embedded btop is a build-time invariant");
        parse_theme_body(body, "btop")
    })
}

/// Public entry point used by startup. Resolves the name against the
/// current XDG config root and applies config-level overrides.
pub fn load_or_default(name: &str, cfg: &AppConfig) -> Theme {
    let mut theme = load_chain(&crate::config::xdg_config_dir(), name);
    apply_overrides(&mut theme, cfg);
    theme
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
        let mut t = Theme::btop();
        let original_bg = t.main_bg;
        apply_overrides(&mut t, &cfg_with_bg(false));
        assert_eq!(t.main_bg, Color::Reset);
        assert_ne!(t.main_bg, original_bg);
    }

    #[test]
    fn apply_overrides_keep_theme_when_flag_default_true() {
        let mut t = Theme::btop();
        let original_bg = t.main_bg;
        apply_overrides(&mut t, &cfg_with_bg(true));
        assert_eq!(t.main_bg, original_bg);
    }

    #[test]
    fn apply_overrides_leaves_other_bg_fields_alone() {
        let mut t = Theme::btop();
        let original_selected = t.selected_bg;
        let original_meter = t.meter_bg;
        apply_overrides(&mut t, &cfg_with_bg(false));
        assert_eq!(t.selected_bg, original_selected);
        assert_eq!(t.meter_bg, original_meter);
    }

    #[test]
    fn apply_overrides_already_reset_main_bg_stays_reset() {
        let mut t = Theme::btop();
        t.main_bg = Color::Reset;
        apply_overrides(&mut t, &cfg_with_bg(true));
        assert_eq!(t.main_bg, Color::Reset);
    }

    #[test]
    fn every_embedded_theme_matches_its_rust_constructor() {
        let pairs: &[(&str, fn() -> Theme)] = &[
            ("btop",          Theme::btop),
            ("dracula",       Theme::dracula),
            ("catppuccin",    Theme::catppuccin),
            ("tokyo-night",   Theme::tokyo_night),
            ("gruvbox",       Theme::gruvbox),
            ("nord",          Theme::nord),
            ("light",         Theme::light),
            ("white",         Theme::white),
            ("high-contrast", Theme::high_contrast),
            ("protanopia",    Theme::protanopia),
            ("deuteranopia",  Theme::deuteranopia),
            ("tritanopia",    Theme::tritanopia),
        ];
        let mut failures: Vec<String> = Vec::new();
        for (name, ctor) in pairs {
            let body = crate::theme::embedded::lookup(name)
                .unwrap_or_else(|| panic!("'{name}' missing from BUILTIN"));
            let parsed = parse_theme_body(body, name);
            let from_rust = ctor();
            if parsed != from_rust {
                failures.push(format!(
                    "  '{name}' drift:\n    parsed: {:?}\n    rust:   {:?}",
                    parsed, from_rust
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "embedded themes drifted from Rust constructors:\n{}",
            failures.join("\n")
        );
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
}
