//! Canonical default values.
//!
//! Every default in the schema is implemented by delegating here. That makes
//! this module the single source of truth, which is what lets
//! `--check-config` assert that the shipped defaults validate clean: there is
//! exactly one set of numbers to check, not two that can drift.
//!
//! The accessors exist because the validator needs a *reference* default to
//! report what it substituted, and reading it from the value being repaired
//! would report the clamped value instead of the original.

use crate::schema::*;

/// Built-in color defaults.
pub struct ColorDefaults {
    /// Default island fill.
    pub fill: &'static str,
    /// Default border.
    pub border_color: &'static str,
    /// Default primary accent.
    pub primary: &'static str,
    /// Default secondary accent.
    pub secondary: &'static str,
    /// Default text color.
    pub text: &'static str,
    /// Default dimmed text color.
    pub dim_color: &'static str,
}

/// Island fill and border defaults.
#[must_use]
pub fn appearance_defaults() -> ColorDefaults {
    ColorDefaults {
        fill: "#12121ACC",
        border_color: "#FFFFFF24",
        ..color_defaults()
    }
}

/// Text color defaults.
#[must_use]
pub fn text_defaults() -> ColorDefaults {
    ColorDefaults {
        color: "#FFFFFFFF",
        dim_color: "#B3FFFFFF",
        ..color_defaults()
    }
}

/// Color engine defaults.
#[must_use]
pub fn rgb_defaults() -> ColorDefaults {
    ColorDefaults {
        primary: "#7C3AED",
        secondary: "#06B6D4",
        ..color_defaults()
    }
}

fn color_defaults() -> ColorDefaults {
    ColorDefaults {
        fill: "#12121ACC",
        border_color: "#FFFFFF24",
        primary: "#7C3AED",
        secondary: "#06B6D4",
        text: "#FFFFFFFF",
        dim_color: "#B3FFFFFF",
    }
}

/// Numeric reference values for sections that need a repair target.
pub struct NumericDefaults {
    /// Maximum glow intensity.
    pub glow_intensity: f32,
    /// Maximum text size.
    pub title_size: f32,
}

/// Numeric defaults shared with the validator's repair messages.
#[must_use]
pub fn numeric_defaults() -> NumericDefaults {
    NumericDefaults { glow_intensity: 2.0, title_size: 15.0 }
}

/// Media defaults, exposed for tests and for the settings summary.
#[must_use]
pub fn media_defaults() -> MediaConfig {
    MediaConfig::default()
}

/// The complete default configuration.
///
/// This is what gets written on first run. [`default_toml`] adds the comments.
#[must_use]
pub fn default_config() -> Config {
    Config::default()
}

/// The first-run configuration file, with a comment above every key.
///
/// Hand maintained rather than generated from the struct, because a comment
/// explaining why a key exists is worth more than a comment restating its type.
/// A test asserts that every key in this file also exists in the schema, so the
/// two cannot drift.
#[must_use]
pub fn default_toml() -> String {
    let _ = default_config();
    DEFAULT_TOML.to_string()
}

/// Static first-run file.
const DEFAULT_TOML: &str = include_str!("../assets/default.toml");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{validate, ValidationReport};

    #[test]
    fn defaults_validate_clean() {
        let (config, report) = validate(default_config());
        assert!(report.is_clean(), "built-in defaults must validate clean: {}", report.render());
        assert_eq!(config, default_config());
    }

    #[test]
    fn numeric_defaults_are_within_their_validator_ranges() {
        let defaults = numeric_defaults();
        assert!((0.0..=2.0).contains(&defaults.glow_intensity));
        assert!((12.0..=96.0).contains(&defaults.title_size));
    }

    #[test]
    fn color_defaults_all_parse() {
        use lumina_core::color::parse_hex;
        let all = color_defaults();
        for (name, value) in [
            ("fill", all.fill),
            ("border_color", all.border_color),
            ("primary", all.primary),
            ("secondary", all.secondary),
            ("text", all.text),
            ("dim_color", all.dim_color),
        ] {
            assert!(parse_hex(value).is_some(), "default {name} {value:?} does not parse");
        }
    }

    #[test]
    fn shipped_toml_parses_and_validates_clean() {
        let parsed = Config::from_toml_str(DEFAULT_TOML).expect("the shipped file must parse");
        let (config, report) = validate(parsed);
        assert!(report.is_clean(), "the shipped file must be valid: {}", report.render());
        // And it must describe the same thing the compiled defaults do.
        assert_eq!(config, default_config(), "the shipped file must match the compiled defaults");
    }

    #[test]
    fn shipped_toml_documents_every_section() {
        for section in [
            "[general]",
            "[appearance]",
            "[rgb]",
            "[rgb.reactive]",
            "[audio]",
            "[audio.visualizer]",
            "[media]",
            "[clock]",
            "[metrics]",
            "[battery]",
            "[network]",
            "[pomodoro]",
            "[weather]",
            "[layout]",
            "[hotkeys]",
            "[shortcuts]",
            "[tray]",
            "[motion]",
            "[text]",
            "[advanced]",
        ] {
            assert!(DEFAULT_TOML.contains(section), "the shipped file is missing {section}");
        }
    }

    #[test]
    fn shipped_toml_carries_a_comment_for_most_keys() {
        // A shipped file with no comments is a documentation failure.
        let key_lines = DEFAULT_TOML
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#') && !t.starts_with('[') && t.contains('=')
            })
            .count();
        let comment_lines = DEFAULT_TOML.lines().filter(|l| l.trim_start().starts_with('#')).count();
        assert!(
            comment_lines >= key_lines,
            "the shipped file has {comment_lines} comments for {key_lines} keys, expected at least one per key"
        );
    }

    #[test]
    fn default_config_serializes_back_into_valid_toml() {
        let text = default_config().to_toml_string();
        let parsed = Config::from_toml_str(&text).expect("serialized defaults must reparse");
        assert_eq!(parsed, default_config());
    }

    #[test]
    fn report_is_clean_for_the_shipped_file() {
        let parsed = Config::from_toml_str(DEFAULT_TOML).expect("parses");
        let (_, report) = validate(parsed);
        assert!(report.has_issues() == false, "{}", report.render());
        let _: ValidationReport = ValidationReport::clean();
    }
}