//! Theme file parser + loader.

use ratatui::style::Color;

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
}
