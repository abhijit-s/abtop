//! Theme file parser + loader.

use ratatui::style::Color;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
