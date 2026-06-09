//! Compile-time table of bundled themes. Each entry is (name, raw .theme body).

pub const BUILTIN: &[(&str, &str)] = &[
    ("btop",          include_str!("../../themes/btop.theme")),
    ("dracula",       include_str!("../../themes/dracula.theme")),
    ("catppuccin",    include_str!("../../themes/catppuccin.theme")),
    ("catppuccin-transparent", include_str!("../../themes/catppuccin-transparent.theme")),
    ("tokyo-night",   include_str!("../../themes/tokyo-night.theme")),
    ("gruvbox",       include_str!("../../themes/gruvbox.theme")),
    ("nord",          include_str!("../../themes/nord.theme")),
    ("light",         include_str!("../../themes/light.theme")),
    ("white",         include_str!("../../themes/white.theme")),
    ("high-contrast", include_str!("../../themes/high-contrast.theme")),
    ("protanopia",    include_str!("../../themes/protanopia.theme")),
    ("deuteranopia",  include_str!("../../themes/deuteranopia.theme")),
    ("tritanopia",    include_str!("../../themes/tritanopia.theme")),
];

/// Look up an embedded theme's raw body by name.
pub fn lookup(name: &str) -> Option<&'static str> {
    BUILTIN.iter().find(|(n, _)| *n == name).map(|(_, body)| *body)
}
