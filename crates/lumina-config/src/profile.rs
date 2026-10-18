//! Named configuration profiles.
//!
//! A profile is a partial configuration that layers over the base. Users switch
//! between them at runtime with a hotkey or the tray menu, and Lumina crossfades
//! the result so the switch is not a jump cut.
//!
//! # Why one level of inheritance
//!
//! Profiles can inherit from other profiles, but only one level deep. That is a
//! deliberate restriction rather than an omission: deep chains make it genuinely
//! hard to answer "why is this key set to that value", and the answer is the
//! first thing a user asks when a profile misbehaves. One level covers every
//! real use case, which is "mostly the defaults, but with these differences".

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::Config;

/// The `[profiles]` section of the configuration file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileSection {
    /// Which profile to activate at startup.
    pub default: String,
    /// Every configured profile, keyed by name.
    pub profiles: BTreeMap<String, Profile>,
}

/// One profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Name of the profile to inherit from. Empty means the base config.
    ///
    /// One level only. A chain longer than one is rejected at load time with a
    /// clear message rather than being followed.
    pub inherit: String,
    /// Overrides, written with dotted paths such as `rgb.mode`.
    ///
    /// A `BTreeMap` rather than a struct so a profile can contain any subset of
    /// keys, which is the entire point of a profile. Quoted dotted keys are
    /// expanded into nested tables before merging, so `"rgb.mode" = "reactive"`
    /// and `[profiles.name.rgb] mode = "reactive"` mean the same thing.
    #[serde(flatten)]
    pub overrides: BTreeMap<String, toml::Value>,
}

/// The outcome of resolving profiles.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileStore {
    active: String,
    profiles: BTreeMap<String, Profile>,
    default_name: String,
}

impl ProfileStore {
    /// Build a store from a parsed profile section, rejecting invalid chains.
    ///
    /// Returns the store plus a list of problems. Problems are reported rather
    /// than fatal: a broken profile is skipped and the base config is used, so
    /// one bad profile cannot stop Lumina from starting.
    #[must_use]
    pub fn from_section(section: ProfileSection) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let ProfileSection { default: requested_default, profiles } = section;

        // Reject anything that inherits more than one level deep, and anything
        // that inherits from a name that does not exist.
        for (name, profile) in &profiles {
            if profile.inherit.is_empty() {
                continue;
            }
            if !profiles.contains_key(&profile.inherit) {
                problems.push(format!(
                    "profile {name:?} inherits from {:?}, which does not exist",
                    profile.inherit
                ));
            } else if !profiles[&profile.inherit].inherit.is_empty() {
                // Covers both a two level chain and a direct cycle, and names
                // the offending profile so the message is actionable.
                problems.push(format!(
                    "profile {name:?} inherits from {:?}, which itself has an inherit; \
                     only one level is supported and cycles are not allowed",
                    profile.inherit
                ));
            }
        }

        // Pick the default, falling back sensibly.
        let default_name = if requested_default.is_empty() {
            if profiles.contains_key("balanced") {
                "balanced".to_string()
            } else if let Some(first) = profiles.keys().next() {
                first.clone()
            } else {
                String::new()
            }
        } else if profiles.contains_key(&requested_default) {
            requested_default
        } else {
            problems.push(format!("default profile {requested_default:?} does not exist, using the base config"));
            String::new()
        };

        (Self { active: default_name.clone(), profiles, default_name }, problems)
    }

    /// Store with no profiles at all.
    #[must_use]
    pub fn empty() -> Self {
        Self { active: String::new(), profiles: BTreeMap::new(), default_name: String::new() }
    }

    /// Name of the active profile, empty when only the base config is in use.
    #[must_use]
    pub fn active(&self) -> &str {
        &self.active
    }

    /// Name of the profile to start with.
    #[must_use]
    pub fn default_name(&self) -> &str {
        &self.default_name
    }

    /// Every profile name, in alphabetical order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.profiles.keys().map(String::as_str).collect()
    }

    /// How many profiles exist.
    #[must_use]
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// True when no profiles are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Switch to `name`, returning false when it does not exist.
    pub fn activate(&mut self, name: &str) -> bool {
        if !self.profiles.contains_key(name) {
            return false;
        }
        self.active = name.to_string();
        true
    }

    /// Switch to the next profile in alphabetical order, wrapping.
    ///
    /// Returns the newly active name, empty when there are no profiles.
    pub fn activate_next(&mut self) -> &str {
        let names = self.names();
        if names.is_empty() {
            return &self.active;
        }
        let position = names.iter().position(|n| *n == self.active).map_or(names.len() - 1, |i| (i + 1) % names.len());
        self.active = names[position].to_string();
        &self.active
    }

    /// Apply the active profile on top of `base`.
    ///
    /// Returns the base config unchanged when no profile is active, so the
    /// common case costs nothing.
    #[must_use]
    pub fn apply(&self, base: &Config) -> Config {
        if self.active.is_empty() {
            return base.clone();
        }
        let Some(profile) = self.profiles.get(&self.active) else {
            return base.clone();
        };
        if profile.overrides.is_empty() {
            return base.clone();
        }

        // Rebuild the flat dotted map into a nested TOML document and hand it
        // to the shared overlay, so profiles and themes layer identically.
        let flat = toml::Table::from_iter(
            profile.overrides.iter().map(|(k, v)| (k.clone(), v.clone())),
        );
        crate::merge::overlay_config(base, &flat)
    }

    /// Resolve the effective configuration for the active profile.
    ///
    /// `config` supplies the base; the result is base plus active profile.
    #[must_use]
    pub fn resolve(store: &ProfileStore, config: &Config) -> Config {
        store.apply(config)
    }
}

/// Convenience free function form of [`ProfileStore::resolve`], re-exported at
/// the crate root so callers do not have to name the type.
#[must_use]
pub fn resolve(base: &Config, store: &ProfileStore) -> Config {
    store.apply(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{RgbMode, VisualizerStyle};

    fn section(text: &str) -> ProfileSection {
        toml::from_str(text).expect("profile section should parse")
    }

    fn config_with_mode(mode: &str) -> Config {
        let text = format!("[rgb]\nmode = \"{mode}\"\n");
        let mut config = Config::from_toml_str(&text).expect("parses");
        config = crate::validate::validate(config).0;
        config
    }

    #[test]
    fn empty_section_produces_an_empty_store() {
        let (store, problems) = ProfileStore::from_section(section(""));
        assert!(store.is_empty());
        assert_eq!(store.active(), "");
        assert_eq!(problems, [] as [std::string::String; 0]);
    }

    #[test]
    fn profile_overrides_the_base_config() {
        let section = section(
            r#"
            [profiles.gaming]
            "rgb.mode" = "reactive"
            "rgb.glow_intensity" = 1.5
        "#,
        );
        let (store, problems) = ProfileStore::from_section(section);
        assert!(problems.is_empty(), "{problems:?}");

        // "gaming" is the only profile, so it becomes the default.
        assert_eq!(store.active(), "gaming");

        let base = config_with_mode("breathing");
        let effective = store.apply(&base);

        assert_eq!(effective.rgb.mode, RgbMode::Reactive);
        assert_eq!(effective.rgb.glow_intensity, 1.5);
        // Keys the profile did not mention keep the base value.
        assert_eq!(effective.rgb.speed, base.rgb.speed);
        assert_eq!(effective.appearance, base.appearance);
    }

    #[test]
    fn profile_can_reach_nested_keys() {
        let section = section(
            r#"
            [profiles.focus]
            "audio.visualizer.style" = "waveform"
            "audio.band_count" = 64
        "#,
        );
        let (store, _) = ProfileStore::from_section(section);
        let effective = store.apply(&Config::default());

        assert_eq!(effective.audio.visualizer.style, VisualizerStyle::Waveform);
        assert_eq!(effective.audio.band_count, 64);
    }

    #[test]
    fn an_empty_profile_changes_nothing() {
        let section = section("[profiles.balanced]\n");
        let (store, problems) = ProfileStore::from_section(section);
        assert_eq!(problems, [] as [std::string::String; 0]);
        let base = Config::default();
        assert_eq!(store.apply(&base), base, "an empty profile must be a no-op");
    }

    #[test]
    fn unknown_default_profile_is_reported_not_fatal() {
        // `default` must come before any `[profiles.x]` table, otherwise TOML
        // scoping puts it inside that table instead of at the section root.
        let section = section(
            r#"
            default = "nonexistent"

            [profiles.gaming]
            "rgb.mode" = "reactive"
            "#,
        );
        let (store, problems) = ProfileStore::from_section(section);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("nonexistent"));
        assert_eq!(store.active(), "", "falls back to the base config");
    }

    #[test]
    fn inheritance_from_a_missing_profile_is_reported() {
        let section = section(
            r#"
            [profiles.gaming]
            inherit = "base-profile"
        "#,
        );
        let (_, problems) = ProfileStore::from_section(section);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("does not exist"));
    }

    #[test]
    fn inheritance_cycles_are_detected() {
        let section = section(
            r#"
            [profiles.a]
            inherit = "b"

            [profiles.b]
            inherit = "a"
        "#,
        );
        let (_, problems) = ProfileStore::from_section(section);
        assert_eq!(problems.len(), 2, "both members of the cycle are reported");
    }

    #[test]
    fn two_level_inheritance_is_reported() {
        let section = section(
            r#"
            [profiles.base_profile]
            "rgb.mode" = "static"

            [profiles.middle]
            inherit = "base_profile"

            [profiles.top]
            inherit = "middle"
        "#,
        );
        let (_, problems) = ProfileStore::from_section(section);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("only one level"), "got: {}", problems[0]);
    }

    #[test]
    fn switching_profiles_cycles_in_order_and_wraps() {
        let section = section(
            r"
            [profiles.alpha]
            [profiles.beta]
            [profiles.gamma]
        ",
        );
        let (mut store, _) = ProfileStore::from_section(section);
        // Alphabetical, so alpha is the automatic default.
        assert_eq!(store.active(), "alpha");

        assert_eq!(store.activate_next(), "beta");
        assert_eq!(store.activate_next(), "gamma");
        assert_eq!(store.activate_next(), "alpha", "should wrap");
    }

    #[test]
    fn activating_a_missing_profile_returns_false() {
        let (mut store, _) = ProfileStore::from_section(section("[profiles.alpha]\n"));
        assert!(!store.activate("beta"));
        assert_eq!(store.active(), "alpha", "a failed switch must not change state");
        assert!(store.activate("alpha"));
    }

    #[test]
    fn profile_names_are_listed_alphabetically() {
        let section = section("[profiles.zulu]\n[profiles.alpha]\n[profiles.mike]\n");
        let (store, _) = ProfileStore::from_section(section);
        assert_eq!(store.names(), vec!["alpha", "mike", "zulu"]);
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn balanced_is_preferred_as_the_default_when_present() {
        let section = section("[profiles.aaa]\n[profiles.balanced]\n[profiles.zzz]\n");
        let (store, _) = ProfileStore::from_section(section);
        assert_eq!(store.default_name(), "balanced");
    }

    #[test]
    fn switching_profiles_actually_changes_the_effective_config() {
        let section = section(
            r#"
            default = "calm"

            [profiles.calm]
            "rgb.mode" = "static"
            "rgb.glow_intensity" = 0.2

            [profiles.loud]
            "rgb.mode" = "reactive"
            "rgb.glow_intensity" = 1.8
        "#,
        );
        let (mut store, problems) = ProfileStore::from_section(section);
        assert!(problems.is_empty(), "{problems:?}");

        let base = Config::default();
        let calm = store.apply(&base);
        assert_eq!(calm.rgb.mode, RgbMode::Static);
        assert_eq!(calm.rgb.glow_intensity, 0.2);

        assert!(store.activate("loud"));
        let loud = store.apply(&base);
        assert_eq!(loud.rgb.mode, RgbMode::Reactive);
        assert_eq!(loud.rgb.glow_intensity, 1.8);
    }

    #[test]
    fn apply_leaves_the_base_untouched() {
        let section = section("[profiles.gaming]\n\"rgb.mode\" = \"reactive\"\n");
        let (store, _) = ProfileStore::from_section(section);
        let base = Config::default();
        let before = base.clone();
        let _ = store.apply(&base);
        assert_eq!(base, before, "apply must not mutate the base config");
    }

    #[test]
    fn free_function_resolve_matches_the_method() {
        let section = section("[profiles.gaming]\n\"rgb.mode\" = \"reactive\"\n");
        let (store, _) = ProfileStore::from_section(section);
        let base = Config::default();
        assert_eq!(resolve(&base, &store), store.apply(&base));
    }
}