//! Compile-time table of bundled themes. Each entry is (name, raw .theme body).

pub const BUILTIN: &[(&str, &str)] = &[
    ("btop", include_str!("../../themes/btop.theme")),
];

/// Look up an embedded theme's raw body by name.
pub fn lookup(name: &str) -> Option<&'static str> {
    BUILTIN.iter().find(|(n, _)| *n == name).map(|(_, body)| *body)
}
