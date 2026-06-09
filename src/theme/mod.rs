//! Theme module — types live in `types`. The embedded `themes/*.theme`
//! files are the source of truth; `Theme::by_name` resolves them via the
//! XDG → user-config → built-in lookup chain in `loader`.

mod types;
pub use types::{Gradient, Theme};

mod loader;
pub use loader::{apply_overrides, load_or_default, parse_theme_body};

mod embedded;

pub const THEME_NAMES: &[&str] = &[
    "btop",
    "dracula",
    "catppuccin",
    "tokyo-night",
    "gruvbox",
    "nord",
    "light",
    "white",
    "high-contrast",
    "protanopia",
    "deuteranopia",
    "tritanopia",
];

impl Theme {
    pub fn by_name(name: &str) -> Option<Self> {
        loader::lookup_chain(&crate::config::xdg_config_dir(), name)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::by_name("btop").expect("embedded btop must resolve")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_presets_load() {
        for name in THEME_NAMES {
            assert!(Theme::by_name(name).is_some(), "theme '{}' not found", name);
        }
    }

    #[test]
    fn unknown_returns_none() {
        assert!(Theme::by_name("nonexistent").is_none());
    }

    #[test]
    fn default_is_btop() {
        let t = Theme::default();
        assert_eq!(t.name, "btop");
    }
}
