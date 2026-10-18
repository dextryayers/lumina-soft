//! Themes: reusable appearance bundles.
//!
//! A theme is a partial configuration file containing only the keys it changes,
//! plus an optional background image. Users drop a file into the themes
//! directory and it appears in the tray menu with no registration step.
//!
//! # Cascading order
//!
//! ```text
//! built-in defaults
//!   -> base config.toml
//!     -> theme file
//!       -> profile (may inherit)
//!         -> command line
//!           -> session IPC commands
//! ```
//!
//! Later layers win. The order is fixed and documented so that "why is this
//! colour wrong" always has the same answer.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::schema::Config;

/// A theme file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Human readable name, shown in the tray menu.
    pub name: String,
    /// Name of another theme to start from. Empty means the base config.
    pub extends: String,
    /// The theme's own overrides, using the same dotted syntax as profiles.
    #[serde(flatten)]
    pub overrides: toml::Table,
    /// Optional background image behind the island.
    pub background: Option<Background>,
}

impl Theme {
    /// Name to show in a menu, falling back to the filename stem.
    #[must_use]
    pub fn display_name(&self, path: &Path) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        path.file_stem().map_or_else(
            || "Unnamed".to_string(),
            |s| s.to_string_lossy().into_owned(),
        )
    }

    /// True when this theme changes anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty() && self.background.is_none()
    }
}

/// An image drawn behind the island.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Background {
    /// Path to the image file.
    pub path: String,
    /// Blur strength applied to the background, 0 to 1.
    pub blur_strength: f32,
    /// Opacity, 0 to 1.
    pub opacity: f32,
    /// How to fill the island's area.
    pub mode: BackgroundMode,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            path: String::new(),
            blur_strength: 0.6,
            opacity: 0.25,
            mode: BackgroundMode::Cover,
        }
    }
}

/// How a background image fills the island.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundMode {
    /// Scale to cover, cropping the overflow. The default.
    #[default]
    Cover,
    /// Scale to fit, leaving bars.
    Contain,
    /// Tile at native size.
    Tile,
    /// Stretch to the island's exact size.
    Stretch,
}

/// A named set of themes discovered on disk.
#[derive(Debug, Clone, Default)]
pub struct ThemeStore {
    themes: Vec<(String, Theme, PathBuf)>,
    problems: Vec<String>,
}

impl ThemeStore {
    /// A store with no themes registered.
    ///
    /// Useful in tests and when a user has an empty themes directory, where
    /// only the built-in themes would otherwise be present.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Load every `.toml` file in `directory`, plus the built-in themes.
    ///
    /// A theme that fails to parse is reported and skipped. One malformed file
    /// must not stop the others from loading, and it must never stop Lumina from
    /// starting.
    #[must_use]
    pub fn load(directory: &Path) -> Self {
        let mut store = Self::default();
        store.add_built_in();

        let Ok(entries) = std::fs::read_dir(directory) else {
            // No themes directory is the normal case, not an error.
            return store;
        };

        // Sort by filename so the menu order is stable across runs, which
        // `read_dir` does not guarantee.
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();

        for path in paths {
            match std::fs::read_to_string(&path) {
                Ok(text) => match toml::from_str::<Theme>(&text) {
                    Ok(theme) => {
                        let name = theme.display_name(&path);
                        store.themes.push((name, theme, path));
                    }
                    Err(e) => store.problems.push(format!("{}: {e}", path.display())),
                },
                Err(e) => store.problems.push(format!("{}: {e}", path.display())),
            }
        }

        store
    }

    /// Register the themes that ship with Lumina.
    fn add_built_in(&mut self) {
        for (name, theme) in builtin_themes() {
            self.themes
                .push((name.to_string(), theme, PathBuf::from(name)));
        }
    }

    /// Register a theme programmatically, used by tests and by the IPC command.
    pub fn insert(&mut self, name: impl Into<String>, theme: Theme) {
        self.themes.push((name.into(), theme, PathBuf::new()));
    }

    /// Theme names in menu order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.themes
            .iter()
            .map(|(name, _, _)| name.as_str())
            .collect()
    }

    /// Look up a theme by name, case insensitive.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.themes
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, theme, _)| theme)
    }

    /// Problems found while loading. These are logged, never fatal.
    #[must_use]
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// How many themes are available.
    #[must_use]
    pub fn len(&self) -> usize {
        self.themes.len()
    }

    /// True when no themes are available.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    /// Layer `theme` on top of `base`, resolving `extends` one level.
    ///
    /// Returns the base unchanged when the theme is missing or empty, so a theme
    /// name that does not resolve degrades to "no theme" rather than to an
    /// error.
    #[must_use]
    pub fn apply(&self, base: &Config, name: &str) -> Config {
        let Some(theme) = self.get(name) else {
            return base.clone();
        };

        let mut working = base.clone();

        // `extends` is resolved first so the theme's own keys win over its
        // parent's. One level only, matching profiles.
        if !theme.extends.is_empty() && !theme.extends.eq_ignore_ascii_case(name) {
            if let Some(parent) = self.get(&theme.extends) {
                if !parent.overrides.is_empty() {
                    working = crate::merge::overlay_config(&working, &parent.overrides);
                }
            }
        }

        if !theme.overrides.is_empty() {
            working = crate::merge::overlay_config(&working, &theme.overrides);
        }

        working
    }
}

/// The eight themes that ship with Lumina.
#[must_use]
pub fn builtin_themes() -> Vec<(&'static str, Theme)> {
    vec![
        ("default", default_theme()),
        ("midnight", midnight_theme()),
        ("neon", neon_theme()),
        ("sunset", sunset_theme()),
        ("mono", mono_theme()),
        ("glass", glass_theme()),
        ("nord", nord_theme()),
        ("catppuccin", catppuccin_theme()),
    ]
}

fn theme_from(text: &str) -> Theme {
    toml::from_str(text).unwrap_or_default()
}

/// The default theme, which changes nothing.
#[must_use]
pub fn default_theme() -> Theme {
    theme_from(
        r#"
        name = "Default"
        extends = ""
    "#,
    )
}

/// Cool, low contrast, easy on the eyes at night.
#[must_use]
pub fn midnight_theme() -> Theme {
    theme_from(
        r##"
        name = "Midnight"

        [appearance]
        fill = "#0A0A14F2"
        border_color = "#3A3A5A66"
        corner_radius = 22.0
        glow_spread = 14.0
        glow_falloff = 10.0

        [rgb]
        mode = "breathing"
        primary = "#4C6EF5"
        secondary = "#7048E8"
        glow_intensity = 0.5
    "##,
    )
}

/// High saturation cyan and magenta with a wide glow.
#[must_use]
pub fn neon_theme() -> Theme {
    theme_from(
        r##"
        name = "Neon"

        [appearance]
        fill = "#05050BF5"
        border_color = "#00E5FFAA"
        corner_radius = 10.0
        glow_spread = 20.0
        glow_falloff = 5.0
        border_width = 1.5

        [rgb]
        mode = "cycling"
        primary = "#FF2D95"
        secondary = "#00E5FF"
        glow_intensity = 1.4
        cycle_degrees_per_second = 25.0
    "##,
    )
}

/// Warm oranges and pinks.
#[must_use]
pub fn sunset_theme() -> Theme {
    theme_from(
        r##"
        name = "Sunset"

        [appearance]
        fill = "#1A1020EC"
        border_color = "#FF9E80AA"
        corner_radius = 20.0
        glow_spread = 12.0
        glow_falloff = 9.0

        [rgb]
        mode = "breathing"
        primary = "#FF7043"
        secondary = "#FFB300"
        breath_rate_hz = 0.18
        breath_depth = 0.22
        glow_intensity = 0.85
    "##,
    )
}

/// Neutral greys, for users who want no colour at all.
#[must_use]
pub fn mono_theme() -> Theme {
    theme_from(
        r##"
        name = "Mono"

        [appearance]
        fill = "#141414F0"
        border_color = "#FFFFFF30"
        corner_radius = 16.0
        glow_spread = 6.0
        glow_falloff = 12.0

        [rgb]
        mode = "static"
        primary = "#E8E8E8"
        secondary = "#A0A0A0"
        saturation = 0.0
        glow_intensity = 0.25
    "##,
    )
}

/// Heavy backdrop blur with a low fill, for a frosted look.
#[must_use]
pub fn glass_theme() -> Theme {
    theme_from(
        r##"
        name = "Glass"

        [appearance]
        fill = "#FFFFFF14"
        border_color = "#FFFFFF38"
        corner_radius = 26.0
        glow_spread = 6.0
        backdrop_blur = "on"
        backdrop_darken = 0.05
        shadow_opacity = 0.12

        [rgb]
        mode = "static"
        primary = "#FFFFFF"
        secondary = "#E0E0E0"
        saturation = 0.15
        glow_intensity = 0.3
    "##,
    )
}

/// The Nord palette.
#[must_use]
pub fn nord_theme() -> Theme {
    theme_from(
        r##"
        name = "Nord"

        [appearance]
        fill = "#2E3440F2"
        border_color = "#81A1C166"
        corner_radius = 14.0
        glow_spread = 10.0
        glow_falloff = 9.0

        [rgb]
        mode = "static"
        primary = "#88C0D0"
        secondary = "#81A1C1"
        glow_intensity = 0.55
    "##,
    )
}

/// Catppuccin Mocha.
#[must_use]
pub fn catppuccin_theme() -> Theme {
    theme_from(
        r##"
        name = "Catppuccin Mocha"

        [appearance]
        fill = "#1E1E2EF2"
        border_color = "#CDD6F44D"
        corner_radius = 18.0
        glow_spread = 10.0
        glow_falloff = 9.0

        [rgb]
        mode = "static"
        primary = "#CBA6F7"
        secondary = "#89B4FA"
        glow_intensity = 0.7
    "##,
    )
}

/// Where themes live for a given configuration directory.
#[must_use]
pub fn themes_directory(config_dir: &Path) -> PathBuf {
    config_dir.join("themes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::RgbMode;
    use crate::validate::validate;

    #[test]
    fn all_builtin_themes_parse_and_validate_clean() {
        for (name, theme) in builtin_themes() {
            // The default theme is deliberately empty: it exists so the tray can
            // offer "back to default" as a real choice, and it must be a no-op.
            if name == "default" {
                assert!(theme.is_empty(), "the default theme must change nothing");
            } else {
                assert!(!theme.is_empty(), "{name} has no content");
            }

            let base = Config::default();
            let applied = ThemeStore::empty()
                .with_theme(name, theme.clone())
                .apply(&base, name);
            let (checked, report) = validate(applied.clone());
            assert!(
                report.is_clean(),
                "theme {name} produces invalid settings: {}",
                report.render()
            );
            assert_eq!(
                checked, applied,
                "theme {name} must not be altered by validation"
            );
        }
    }

    #[test]
    fn eight_themes_ship_with_lumina() {
        assert_eq!(builtin_themes().len(), 8);
    }

    #[test]
    fn a_theme_overrides_the_base_config() {
        let store = ThemeStore::empty().with_theme("neon", neon_theme());
        let base = Config::default();
        let themed = store.apply(&base, "neon");

        assert_eq!(themed.rgb.mode, RgbMode::Cycling);
        assert_eq!(themed.appearance.fill, "#05050BF5");
        assert_eq!(themed.appearance.glow_spread, 20.0);
        // Untouched keys keep the base value.
        assert_eq!(themed.appearance.height, base.appearance.height);
        assert_eq!(themed.rgb.breath_rate_hz, base.rgb.breath_rate_hz);
    }

    #[test]
    fn a_theme_can_extend_another_theme() {
        let store = ThemeStore::empty()
            .with_theme("midnight", midnight_theme())
            .with_theme(
                "midnight-neon",
                theme_from(
                    r##"
                    name = "Midnight Neon"
                    extends = "midnight"
                    "rgb.primary" = "#00E5FF"
                    "rgb.glow_intensity" = 1.2
                "##,
                ),
            );

        let base = Config::default();
        let parent = store.apply(&base, "midnight");
        let child = store.apply(&base, "midnight-neon");

        // The child's own keys win.
        assert_eq!(child.rgb.primary, "#00E5FF");
        assert_eq!(child.rgb.glow_intensity, 1.2);
        // The parent's keys come through.
        assert_eq!(child.appearance.fill, parent.appearance.fill);
        assert_eq!(
            child.appearance.corner_radius,
            parent.appearance.corner_radius
        );
    }

    #[test]
    fn an_unknown_theme_name_degrades_to_the_base_config() {
        let store = ThemeStore::empty().with_theme("neon", neon_theme());
        let base = Config::default();
        assert_eq!(store.apply(&base, "does-not-exist"), base);
    }

    #[test]
    fn theme_lookup_is_case_insensitive() {
        let store = ThemeStore::empty().with_theme("Neon", neon_theme());
        assert!(store.get("neon").is_some());
        assert!(store.get("NEON").is_some());
        assert!(store.get("NeOn").is_some());
    }

    #[test]
    fn default_theme_changes_nothing() {
        let store = ThemeStore::empty().with_theme("default", default_theme());
        let base = Config::default();
        assert_eq!(store.apply(&base, "default"), base);
    }

    #[test]
    fn mono_theme_is_actually_desaturated() {
        let store = ThemeStore::empty().with_theme("mono", mono_theme());
        let themed = store.apply(&Config::default(), "mono");
        assert_eq!(themed.rgb.saturation, 0.0, "mono means no chroma");
    }

    #[test]
    fn glass_theme_requests_backdrop_blur() {
        let store = ThemeStore::empty().with_theme("glass", glass_theme());
        let themed = store.apply(&Config::default(), "glass");
        assert_eq!(
            themed.appearance.backdrop_blur,
            crate::schema::BackdropMode::On
        );
    }

    #[test]
    fn theme_names_are_listed_in_registration_order() {
        let store = ThemeStore::empty()
            .with_theme("alpha", default_theme())
            .with_theme("beta", default_theme());
        assert_eq!(store.names(), vec!["alpha", "beta"]);
        assert_eq!(store.len(), 2);
        assert!(!store.is_empty());
    }

    #[test]
    fn loading_a_missing_directory_is_not_an_error() {
        let store = ThemeStore::load(Path::new("/nonexistent/lumina/themes/path"));
        // The eight built-in themes are still there.
        assert!(store.len() >= 8);
        assert!(store.problems().is_empty(), "{:?}", store.problems());
    }

    #[test]
    fn display_name_falls_back_to_the_filename() {
        let path = Path::new("/themes/my-look.toml");
        let unnamed = Theme::default();
        assert_eq!(unnamed.display_name(path), "my-look");

        let named = Theme {
            name: "Pretty Name".to_string(),
            ..Theme::default()
        };
        assert_eq!(named.display_name(path), "Pretty Name");
    }

    #[test]
    fn background_defaults_are_sensible() {
        let background = Background::default();
        assert_eq!(background.mode, BackgroundMode::Cover);
        assert!((background.opacity - 0.25).abs() < 1e-6);
        assert_eq!(background.path, "");
    }

    #[test]
    fn a_theme_with_a_background_still_validates() {
        let theme = theme_from(
            r#"
            name = "WithImage"

            [background]
            path = "~/wallpaper.png"
            blur_strength = 0.8
            opacity = 0.4
            mode = "tile"
            "#,
        );
        assert!(theme.background.is_some());
        let background = theme.background.as_ref().expect("present");
        assert_eq!(background.mode, BackgroundMode::Tile);

        let applied = ThemeStore::empty()
            .with_theme("t", theme)
            .apply(&Config::default(), "t");
        let (checked, report) = validate(applied.clone());
        assert!(report.is_clean(), "{}", report.render());
        assert_eq!(
            checked, applied,
            "validation must not alter a valid theme result"
        );
    }

    impl ThemeStore {
        /// Build a store containing exactly one theme, for tests.
        fn with_theme(mut self, name: &str, theme: Theme) -> Self {
            self.insert(name, theme);
            self
        }
    }
}
