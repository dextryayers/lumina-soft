//! Canonical default values.
//!
//! Every default in the schema is implemented by delegating here. That makes
//! this module the single source of truth, which is what lets a test assert
//! that the shipped defaults validate clean: there is exactly one set of
//! numbers to check, not two that can drift.
//!
//! The accessors exist because the validator needs a *reference* default to
//! report what it substituted, and reading it from the value being repaired
//! would report the already-clamped number instead of the original.

use crate::schema::{Config, MediaConfig};

/// Island fill and border colours.
#[derive(Debug, Clone, Copy)]
pub struct ColorDefaults {
    /// Default island fill.
    pub fill: &'static str,
    /// Default border colour.
    pub border_color: &'static str,
    /// Default primary accent.
    pub primary: &'static str,
    /// Default secondary accent.
    pub secondary: &'static str,
}

/// The two text colours, which have different names from the rest.
#[derive(Debug, Clone, Copy)]
pub struct TextColors {
    /// Primary text.
    pub color: &'static str,
    /// Secondary, dimmer text.
    pub dim_color: &'static str,
}

/// Every shipped colour, for tests that assert they all parse.
#[must_use]
pub fn all_default_colors() -> ColorDefaults {
    ColorDefaults {
        fill: "#12121ACC",
        border_color: "#FFFFFF24",
        primary: "#7C3AED",
        secondary: "#06B6D4",
    }
}

/// Island fill and border defaults.
#[must_use]
pub fn appearance_defaults() -> ColorDefaults {
    ColorDefaults {
        fill: "#12121ACC",
        border_color: "#FFFFFF24",
        ..all_default_colors()
    }
}

/// Colour engine defaults.
#[must_use]
pub fn rgb_defaults() -> ColorDefaults {
    ColorDefaults {
        primary: "#7C3AED",
        secondary: "#06B6D4",
        ..all_default_colors()
    }
}

/// Text colour defaults.
#[must_use]
pub fn text_defaults() -> TextColors {
    TextColors {
        color: "#FFFFFFFF",
        dim_color: "#B3FFFFFF",
    }
}

/// Numeric reference values for the validator's repair messages.
#[derive(Debug, Clone, Copy)]
pub struct NumericDefaults {
    /// Maximum glow intensity.
    pub glow_intensity: f32,
    /// Default track title size.
    pub title_size: f32,
}

/// Numeric defaults shared with the validator.
#[must_use]
pub fn numeric_defaults() -> NumericDefaults {
    NumericDefaults {
        glow_intensity: 2.0,
        title_size: 15.0,
    }
}

/// Media defaults, exposed so the settings summary and tests share one source.
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
/// explaining *why* a key exists is worth more than a comment restating its
/// type. Two tests below keep it honest: one asserts it parses and validates
/// clean, the other asserts it still describes exactly the compiled defaults.
#[must_use]
pub fn default_toml() -> String {
    DEFAULT_TOML.to_string()
}

/// Static first-run file, embedded at compile time.
const DEFAULT_TOML: &str = include_str!("../assets/default.toml");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{validate, ValidationReport};
    use lumina_core::color::parse_hex;

    #[test]
    fn defaults_validate_clean() {
        let (config, report) = validate(default_config());
        assert!(
            report.is_clean(),
            "built-in defaults must validate clean: {}",
            report.render()
        );
        assert_eq!(config, default_config());
    }

    #[test]
    fn numeric_defaults_are_within_their_validator_ranges() {
        let defaults = numeric_defaults();
        assert!((0.0..=2.0).contains(&defaults.glow_intensity));
        assert!((12.0..=96.0).contains(&defaults.title_size));
    }

    #[test]
    fn every_default_color_parses() {
        let all = all_default_colors();
        for (name, value) in [
            ("fill", all.fill),
            ("border_color", all.border_color),
            ("primary", all.primary),
            ("secondary", all.secondary),
        ] {
            assert!(
                parse_hex(value).is_some(),
                "default {name} {value:?} does not parse"
            );
        }

        let text = text_defaults();
        assert!(
            parse_hex(text.color).is_some(),
            "default text colour does not parse"
        );
        assert!(
            parse_hex(text.dim_color).is_some(),
            "default dim text colour does not parse"
        );
    }

    #[test]
    fn default_toml_is_not_empty_and_starts_with_a_header() {
        assert!(DEFAULT_TOML.starts_with("# Lumina"));
        assert!(DEFAULT_TOML.contains("lumina --check-config"));
    }

    #[test]
    fn shipped_toml_parses_and_validates_clean() {
        let parsed = Config::from_toml_str(DEFAULT_TOML).expect("the shipped file must parse");
        let (_config, report) = validate(parsed);
        assert!(
            report.is_clean(),
            "the shipped file must be valid: {}",
            report.render()
        );
    }

    #[test]
    fn shipped_toml_describes_exactly_the_compiled_defaults() {
        // This is the test that stops the hand written file and the Rust
        // structs from drifting apart.
        let parsed = Config::from_toml_str(DEFAULT_TOML).expect("the shipped file must parse");
        assert_eq!(
            parsed,
            default_config(),
            "the shipped file has drifted from the compiled defaults"
        );
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
            "[script]",
            "[tray]",
            "[motion]",
            "[text]",
            "[advanced]",
        ] {
            assert!(
                DEFAULT_TOML.contains(section),
                "the shipped file is missing {section}"
            );
        }
    }

    #[test]
    fn shipped_toml_carries_a_comment_for_most_keys() {
        // A shipped file with no explanations is a documentation failure, so
        // the comment count is held at or above the key count.
        let key_lines = DEFAULT_TOML
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#') && !t.starts_with('[') && t.contains('=')
            })
            .count();
        let comment_lines = DEFAULT_TOML
            .lines()
            .filter(|l| l.trim_start().starts_with('#'))
            .count();

        assert!(
            comment_lines >= key_lines,
            "the shipped file has {comment_lines} comment lines for {key_lines} keys; \
             every key should be explained"
        );
    }

    #[test]
    fn default_config_serializes_back_into_valid_toml() {
        let text = default_config().to_toml_string();
        let parsed = Config::from_toml_str(&text).expect("serialized defaults must reparse");
        assert_eq!(parsed, default_config());
    }

    #[test]
    fn clean_report_is_reachable_from_the_shipped_file() {
        let parsed = Config::from_toml_str(DEFAULT_TOML).expect("parses");
        let (_, report) = validate(parsed);
        assert!(!report.has_issues(), "{}", report.render());
        assert_eq!(report.render(), ValidationReport::clean().render());
    }
}
