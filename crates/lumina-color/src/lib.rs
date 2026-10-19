//! RGB accent engines for Lumina.
//!
//! # The five engines
//!
//! Every engine answers the same question: given a configuration, the time, and
//! optionally live audio or cover art, what is the accent colour for this frame?
//! Keeping that question in one place means the shader, widgets, and diagnostics
//! all agree about what "the colour" is.
//!
//! | Engine | Input | Output |
//! |---|---|---|
//! | Static | `rgb.primary` | one fixed colour |
//! | Breathing | time | a slow lightness pulse |
//! | Cycling | time | hue rotation through the wheel |
//! | Reactive | [`AudioAggregates`] | bass glow, mid lightness, treble hue |
//! | Album | cover art colours | an extracted dominant colour |
//!
//! # Crossfades
//!
//! Switching engines or profiles does not snap. The previous palette is frozen
//! and interpolated toward the live palette in OkLab over
//! `rgb.transition_ms`. Reduced motion turns the crossfade off, because a
//! colour fade is still motion to a user who asked for less of it.
//!
//! # Accessibility
//!
//! A user can pick any fill and any text colour, including combinations that are
//! unreadable. Rather than ignoring that, every resolved palette is checked
//! against the WCAG target in `[accessibility]`; when it fails, the text
//! lightness is adjusted until it passes, and [`ColorEngine::contrast_adjusted`]
//! reports that it happened. Disabling `accessibility.enforce_contrast` is the
//! only way to suppress the adjustment.

#![deny(missing_docs)]

use lumina_audio::AudioAggregates;
use lumina_config::schema::{AccessibilityConfig, Config, RgbMode};
use lumina_core::color::{parse_hex, OkLab, Palette, Srgb};

/// Default primary accent, matching the shipped configuration.
const DEFAULT_PRIMARY: &str = "#7C3AED";
/// Default secondary accent, matching the shipped configuration.
const DEFAULT_SECONDARY: &str = "#06B6D4";
/// Default fill, matching the shipped configuration.
const DEFAULT_FILL: &str = "#12121ACC";
/// Default border, matching the shipped configuration.
const DEFAULT_BORDER: &str = "#FFFFFF24";
/// Default text colour.
const DEFAULT_TEXT: &str = "#FFFFFFFF";
/// Default dim text colour.
const DEFAULT_DIM_TEXT: &str = "#B3FFFFFF";

/// How much of the border colour is replaced by the live accent.
///
/// A border that is entirely accent loses the deliberate border colour a theme
/// author chose; a border with no accent stops following the music. Two thirds
/// keeps both readable.
const BORDER_ACCENT_WEIGHT: f32 = 0.66;

/// One live accent frame plus the glow strength that belongs to it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Accent {
    color: Srgb,
    secondary: Srgb,
    glow_intensity: f32,
}

/// A palette crossfade in progress.
#[derive(Debug, Clone, Copy)]
struct Transition {
    from: Palette,
    elapsed_s: f32,
    duration_s: f32,
}

/// Resolves the palette for the current frame.
///
/// The engine is cheap to keep for the life of the process. It allocates only
/// when the configuration is replaced and never inside [`Self::update`].
#[derive(Debug, Clone)]
pub struct ColorEngine {
    config: Config,
    palette: Palette,
    transition: Option<Transition>,
    time_s: f32,
    contrast_adjusted: bool,
    frames: u64,
}

impl ColorEngine {
    /// Build an engine from a validated configuration.
    #[must_use]
    pub fn new(config: &Config) -> Self {
        let accent = accent_for(config, 0.0, AudioAggregates::default(), None);
        let (palette, contrast_adjusted) = build_palette(config, accent);
        Self {
            config: config.clone(),
            palette,
            transition: None,
            time_s: 0.0,
            contrast_adjusted,
            frames: 0,
        }
    }

    /// Replace the configuration, starting a crossfade when the visual inputs
    /// changed.
    ///
    /// Returns `true` when a crossfade was started. Unrelated changes (for
    /// example a hotkey) do not restart the fade, which would otherwise flash
    /// the island every time the config watcher fires.
    pub fn reconfigure(&mut self, config: &Config) -> bool {
        let changed = self.config.rgb != config.rgb
            || self.config.appearance != config.appearance
            || self.config.text != config.text
            || self.config.accessibility != config.accessibility
            || self.config.motion.reduced != config.motion.reduced;

        if !changed {
            return false;
        }

        let duration_s = if config.motion.reduced || config.rgb.transition_ms == 0 {
            0.0
        } else {
            config.rgb.transition_ms as f32 / 1000.0
        };

        self.config = config.clone();
        if duration_s <= 0.0 {
            self.transition = None;
            let accent = accent_for(
                &self.config,
                self.time_s,
                AudioAggregates::default(),
                None,
            );
            let (palette, adjusted) = build_palette(&self.config, accent);
            self.palette = palette;
            self.contrast_adjusted = adjusted;
        } else {
            self.transition = Some(Transition {
                from: self.palette,
                elapsed_s: 0.0,
                duration_s,
            });
        }
        true
    }

    /// Advance the engine by `dt` seconds and resolve the palette.
    ///
    /// `audio` is the current three-band aggregate, or
    /// [`AudioAggregates::default`] when capture is unavailable. `album` is the
    /// dominant cover colour when one has been extracted.
    ///
    /// The returned palette is also available through [`Self::palette`].
    pub fn update(
        &mut self,
        dt: f32,
        audio: AudioAggregates,
        album: Option<Srgb>,
    ) -> Palette {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        self.time_s = (self.time_s + dt).rem_euclid(86_400.0);
        self.frames = self.frames.saturating_add(1);

        let accent = accent_for(&self.config, self.time_s, audio, album);
        let (mut live, adjusted) = build_palette(&self.config, accent);

        if let Some(transition) = &mut self.transition {
            transition.elapsed_s += dt;
            let t = if transition.duration_s <= f32::EPSILON {
                1.0
            } else {
                (transition.elapsed_s / transition.duration_s).clamp(0.0, 1.0)
            };
            live = transition.from.lerp(live, smoothstep(t));
            if t >= 1.0 {
                self.transition = None;
            }

            // Interpolating two legible palettes can pass through an illegible
            // midpoint, so the result is checked again. The adjustment is only
            // reported when contrast enforcement is enabled.
            if self.config.accessibility.enforce_contrast
                && live.enforce_contrast(self.contrast_target())
            {
                self.contrast_adjusted = true;
            }
        }

        self.palette = live;
        if adjusted {
            self.contrast_adjusted = true;
        }
        self.palette
    }

    /// The palette resolved by the last update.
    #[must_use]
    pub fn palette(&self) -> Palette {
        self.palette
    }

    /// True when text colours have been adjusted for legibility since the last
    /// [`Self::reset_contrast_report`].
    #[must_use]
    pub fn contrast_adjusted(&self) -> bool {
        self.contrast_adjusted
    }

    /// Clear the contrast adjustment report, for a diagnostics window that
    /// shows an indicator only for the current state.
    pub fn reset_contrast_report(&mut self) {
        self.contrast_adjusted = false;
    }

    /// Seconds of animation time accumulated.
    #[must_use]
    pub fn time_s(&self) -> f32 {
        self.time_s
    }

    /// Frames resolved since construction, used by the debug overlay.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The active engine.
    #[must_use]
    pub fn mode(&self) -> RgbMode {
        self.config.rgb.mode
    }

    /// True while a palette crossfade is in progress.
    #[must_use]
    pub fn is_transitioning(&self) -> bool {
        self.transition.is_some()
    }

    /// The configured contrast target, clamped to a sane range even when the
    /// caller bypassed validation.
    fn contrast_target(&self) -> f32 {
        let accessibility: &AccessibilityConfig = &self.config.accessibility;
        if accessibility.contrast_target.is_finite() {
            accessibility.contrast_target.clamp(3.0, 7.0)
        } else {
            4.5
        }
    }
}

/// Resolve the live accent for an engine.
fn accent_for(
    config: &Config,
    time_s: f32,
    audio: AudioAggregates,
    album: Option<Srgb>,
) -> Accent {
    let rgb = &config.rgb;
    let primary = parse_hex(&rgb.primary)
        .unwrap_or_else(|| parse_hex(DEFAULT_PRIMARY).unwrap_or_default());
    let secondary = parse_hex(&rgb.secondary)
        .unwrap_or_else(|| parse_hex(DEFAULT_SECONDARY).unwrap_or_default());
    let speed = finite_or(rgb.speed, 1.0).clamp(0.05, 8.0);

    let (mut color, mut glow) = match rgb.mode {
        RgbMode::Static => (primary, rgb.glow_intensity),
        RgbMode::Breathing => {
            let rate = finite_or(rgb.breath_rate_hz, 0.25).clamp(0.01, 4.0);
            let depth = finite_or(rgb.breath_depth, 0.18).clamp(0.0, 0.6);
            let phase = std::f32::consts::TAU * rate * speed * time_s;
            let base = primary.to_oklab();
            let lightness = (base.l + depth * phase.sin()).clamp(0.0, 1.0);
            let breath = 1.0 + (depth * 0.5 * phase.sin());
            (
                color_from_oklab(base.with_lightness(lightness), primary.a),
                rgb.glow_intensity * breath.max(0.0),
            )
        }
        RgbMode::Cycling => {
            let degrees = finite_or(rgb.cycle_degrees_per_second, 40.0).clamp(1.0, 360.0);
            let radians = (degrees * speed * time_s).to_radians();
            (
                color_from_oklab(primary.to_oklab().rotated_hue(radians), primary.a),
                rgb.glow_intensity,
            )
        }
        RgbMode::Reactive => {
            let r = &rgb.reactive;
            let gain = finite_or(r.gain, 1.0).clamp(0.0, 4.0);
            let floor = finite_or(r.floor, 0.12).clamp(0.0, 1.0);
            let bass = unit(audio.bass * gain);
            let mid = unit(audio.mid * gain);
            let treble = unit(audio.treble * gain);
            let loudness = unit(audio.loudness * gain);

            let hue = treble * finite_or(r.hue_spread_degrees, 90.0).to_radians();
            let base = primary.to_oklab().rotated_hue(hue);
            let lightness = (base.l * (0.9 + 0.22 * mid)).clamp(0.0, 1.0);
            let energy = floor + (1.0 - floor) * loudness;
            let dynamic_glow = rgb.glow_intensity + bass * r.glow_from_bass.max(0.0);
            (
                color_from_oklab(base.with_lightness(lightness), primary.a),
                dynamic_glow * energy.max(0.0),
            )
        }
        RgbMode::Album => (album.unwrap_or(primary), rgb.glow_intensity),
    };

    let saturation = finite_or(rgb.saturation, 0.9).clamp(0.0, 2.0);
    let brightness = finite_or(rgb.brightness, 1.0).clamp(0.0, 2.0);
    let tune = |source: Srgb| {
        color_from_oklab(
            source
                .to_oklab()
                .scaled_chroma(saturation)
                .scaled_lightness(brightness),
            source.a,
        )
    };
    color = tune(color);
    glow = finite_or(glow, 0.0).clamp(0.0, 2.0);

    Accent {
        color,
        secondary: tune(secondary),
        glow_intensity: glow,
    }
}

/// Build the final palette from the live accent.
fn build_palette(config: &Config, accent: Accent) -> (Palette, bool) {
    let appearance = &config.appearance;

    let fill = parse_hex(&appearance.fill)
        .unwrap_or_else(|| parse_hex(DEFAULT_FILL).unwrap_or_default());
    let border_base = parse_hex(&appearance.border_color)
        .unwrap_or_else(|| parse_hex(DEFAULT_BORDER).unwrap_or_default());
    let border = mix_srgb(border_base, with_alpha(accent.color, border_base.a), BORDER_ACCENT_WEIGHT);
    let gradient = finite_or(appearance.fill_gradient, 0.0).clamp(0.0, 1.0);
    let fill_accent = if gradient > 0.0 {
        mix_srgb(fill, with_alpha(accent.secondary, fill.a), gradient)
    } else {
        fill
    };

    let text = parse_hex(&config.text.color)
        .unwrap_or_else(|| parse_hex(DEFAULT_TEXT).unwrap_or_default());
    let text_dim = parse_hex(&config.text.dim_color)
        .unwrap_or_else(|| parse_hex(DEFAULT_DIM_TEXT).unwrap_or_default());

    let mut palette = Palette {
        fill,
        fill_accent,
        border,
        glow: with_alpha(accent.color, 1.0),
        text,
        text_dim,
        glow_intensity: accent.glow_intensity,
    };

    let adjusted = if config.accessibility.enforce_contrast {
        let target = if config.accessibility.contrast_target.is_finite() {
            config.accessibility.contrast_target.clamp(3.0, 7.0)
        } else {
            4.5
        };
        palette.enforce_contrast(target)
    } else {
        false
    };

    (palette, adjusted)
}

/// Album mode without an extracted colour: fall back to the primary.
///
/// This exists separately so the intent is testable and so a future extraction
/// failure has one obvious place to report from. The caller normally supplies
/// the extracted colour, so `None` here really does mean "no cover art".
fn extract_album_color(_album: Option<Srgb>) -> Option<Srgb> {
    None
}

/// Convert OkLab back to sRGB, preserving the source alpha.
fn color_from_oklab(lab: OkLab, alpha: f32) -> Srgb {
    lumina_core::color::srgb_from_oklab(lab, alpha)
}

/// Mix two sRGB colours in OkLab while preserving the first colour's alpha.
fn mix_srgb(a: Srgb, b: Srgb, t: f32) -> Srgb {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    color_from_oklab(a.to_oklab().lerp(b.to_oklab(), t), a.a)
}

/// Replace a colour's alpha without changing its channels.
fn with_alpha(color: Srgb, alpha: f32) -> Srgb {
    Srgb::new(color.r, color.g, color.b, alpha.clamp(0.0, 1.0))
}

/// Clamp audio energy to `0..=1`, treating NaN as silence.
fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Replace non-finite configuration values with a safe fallback.
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// Smooth Hermite interpolation for a crossfade with no velocity discontinuity.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Small pipeline helper for the builder-style colour operations above.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use lumina_config::schema::{AccessibilityConfig, RgbConfig};

    fn config_with(mode: RgbMode) -> Config {
        let mut config = Config::default();
        config.rgb.mode = mode;
        config.accessibility = AccessibilityConfig {
            contrast_target: 4.5,
            ..AccessibilityConfig::default()
        };
        config
    }

    fn audio(loudness: f32) -> AudioAggregates {
        AudioAggregates {
            bass: loudness,
            mid: loudness,
            treble: loudness * 0.5,
            loudness,
        }
    }

    #[test]
    fn every_engine_resolves_a_finite_opaque_accent() {
        for mode in RgbMode::ALL {
            let mut engine = ColorEngine::new(&config_with(mode));
            let palette = engine.update(1.0 / 60.0, audio(0.4), Some(Srgb::from_rgba8(200, 40, 90, 255)));
            for channel in [
                palette.fill.r,
                palette.fill.g,
                palette.fill.b,
                palette.border.r,
                palette.border.g,
                palette.border.b,
                palette.glow.r,
                palette.glow.g,
                palette.glow.b,
            ] {
                assert!(channel.is_finite(), "{mode:?} produced a non-finite channel");
                assert!((0.0..=1.0).contains(&channel), "{mode:?} produced {channel}");
            }
            assert_eq!(palette.glow.a, 1.0);
            assert!(palette.glow_intensity.is_finite());
        }
    }

    #[test]
    fn static_mode_uses_the_primary_colour() {
        let mut config = config_with(RgbMode::Static);
        config.rgb.primary = "#FF0000".to_string();
        let engine = ColorEngine::new(&config);
        let glow = engine.palette().glow;
        assert!((glow.r - 1.0).abs() < 1e-5);
        assert!(glow.g < 1e-5 && glow.b < 1e-5);
    }

    #[test]
    fn breathing_oscillates_but_never_leaves_the_gamut() {
        let mut config = config_with(RgbMode::Breathing);
        config.rgb.breath_rate_hz = 1.0;
        config.rgb.breath_depth = 0.4;
        let mut engine = ColorEngine::new(&config);
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;

        for step in 0..120 {
            let _ = step;
            let palette = engine.update(1.0 / 60.0, AudioAggregates::default(), None);
            let l = palette.glow.to_oklab().l;
            lo = lo.min(l);
            hi = hi.max(l);
        }

        assert!(hi - lo > 0.1, "breathing should visibly move lightness");
        assert!(lo >= 0.0 && hi <= 1.0);
    }

    #[test]
    fn cycling_rotates_the_hue_over_time() {
        let mut config = config_with(RgbMode::Cycling);
        config.rgb.primary = "#FF0000".to_string();
        config.rgb.cycle_degrees_per_second = 90.0;
        config.rgb.speed = 1.0;
        config.rgb.saturation = 1.0;
        let mut engine = ColorEngine::new(&config);
        let first = engine.update(0.0, AudioAggregates::default(), None).glow;
        let later = engine.update(1.0, AudioAggregates::default(), None).glow;
        let first_hue = first.to_oklab().hue();
        let later_hue = later.to_oklab().hue();
        assert!(
            (later_hue - first_hue).abs() > 0.5,
            "a quarter turn should be a large hue change"
        );
    }

    #[test]
    fn reactive_glow_rises_with_bass() {
        let mut engine = ColorEngine::new(&config_with(RgbMode::Reactive));
        let quiet = engine.update(0.0, AudioAggregates::default(), None).glow_intensity;
        let loud = engine
            .update(0.0, audio(1.0), None)
            .glow_intensity;
        assert!(loud > quiet, "bass should drive the glow");
    }

    #[test]
    fn reactive_never_goes_completely_dead_in_silence() {
        let mut config = config_with(RgbMode::Reactive);
        config.rgb.reactive.floor = 0.2;
        let mut engine = ColorEngine::new(&config);
        let palette = engine.update(0.0, AudioAggregates::default(), None);
        assert!(palette.glow_intensity > 0.0, "the floor should keep a glow");
    }

    #[test]
    fn album_mode_uses_the_supplied_dominant_colour() {
        let mut engine = ColorEngine::new(&config_with(RgbMode::Album));
        let album = Srgb::from_rgba8(10, 200, 30, 255);
        let palette = engine.update(0.0, AudioAggregates::default(), Some(album));
        let glow = palette.glow.to_oklab();
        let expected = album.to_oklab();
        assert!((glow.hue() - expected.hue()).abs() < 0.05);
    }

    #[test]
    fn album_mode_without_art_falls_back_to_primary() {
        let mut config = config_with(RgbMode::Album);
        config.rgb.primary = "#123456".to_string();
        config.rgb.saturation = 1.0;
        let engine = ColorEngine::new(&config);
        let expected = parse_hex("#123456").expect("valid");
        let glow = engine.palette().glow;
        assert!((glow.r - expected.r).abs() < 0.02);
        assert!((glow.g - expected.g).abs() < 0.02);
        assert!((glow.b - expected.b).abs() < 0.02);
    }

    #[test]
    fn brightness_and_saturation_affect_the_accent() {
        let mut dim = config_with(RgbMode::Static);
        dim.rgb.primary = "#00FF00".to_string();
        dim.rgb.brightness = 0.2;
        let dim_palette = ColorEngine::new(&dim).palette();

        let mut bright = dim.clone();
        bright.rgb.brightness = 1.0;
        let bright_palette = ColorEngine::new(&bright).palette();

        assert!(
            bright_palette.glow.to_oklab().l > dim_palette.glow.to_oklab().l,
            "brightness must move lightness"
        );

        let mut grey = bright;
        grey.rgb.saturation = 0.0;
        let grey_palette = ColorEngine::new(&grey).palette();
        assert!(
            grey_palette.glow.to_oklab().chroma() < 0.01,
            "zero saturation must remove chroma"
        );
    }

    #[test]
    fn text_contrast_is_enforced_across_modes_and_brightness() {
        let fills = ["#000000", "#FFFFFF", "#777777", "#12121ACC"];
        for mode in RgbMode::ALL {
            for fill in fills {
                for brightness in [0.0, 0.5, 1.0, 2.0] {
                    let mut config = config_with(mode);
                    config.appearance.fill = fill.to_string();
                    config.text.color = "#FFFFFF".to_string();
                    config.text.dim_color = "#EEEEEE".to_string();
                    config.rgb.brightness = brightness;
                    config.accessibility.contrast_target = 4.5;

                    let engine = ColorEngine::new(&config);
                    let palette = engine.palette();
                    assert!(
                        palette.text_contrast() >= 4.49,
                        "{mode:?} fill {fill} brightness {brightness}: text contrast {}",
                        palette.text_contrast()
                    );
                    assert!(
                        palette.dim_contrast() >= 4.49,
                        "{mode:?} fill {fill} brightness {brightness}: dim contrast {}",
                        palette.dim_contrast()
                    );
                }
            }
        }
    }

    #[test]
    fn contrast_enforcement_can_be_disabled_explicitly() {
        let mut config = config_with(RgbMode::Static);
        config.appearance.fill = "#FFFFFF".to_string();
        config.text.color = "#FFFFFF".to_string();
        config.text.dim_color = "#FFFFFF".to_string();
        config.accessibility.enforce_contrast = false;

        let engine = ColorEngine::new(&config);
        assert!(!engine.contrast_adjusted());
        assert!(engine.palette().text_contrast() < 1.1);
    }

    #[test]
    fn an_automatic_contrast_fix_is_reported() {
        let mut config = config_with(RgbMode::Static);
        config.appearance.fill = "#FFFFFF".to_string();
        config.text.color = "#FFFFFF".to_string();
        config.text.dim_color = "#FFFFFF".to_string();
        config.accessibility.enforce_contrast = true;
        config.accessibility.contrast_target = 7.0;

        let mut engine = ColorEngine::new(&config);
        assert!(engine.contrast_adjusted(), "white on white must be repaired");
        assert!(engine.palette().text_contrast() >= 6.99);
        engine.reset_contrast_report();
        assert!(!engine.contrast_adjusted());
    }

    #[test]
    fn a_crossfade_starts_on_a_visual_reconfigure_and_finishes() {
        let mut config = config_with(RgbMode::Static);
        config.rgb.transition_ms = 200;
        config.rgb.primary = "#FF0000".to_string();
        let mut engine = ColorEngine::new(&config);

        let mut changed = config.clone();
        changed.rgb.primary = "#0000FF".to_string();
        assert!(engine.reconfigure(&changed));
        assert!(engine.is_transitioning());

        let midway = engine.update(0.1, AudioAggregates::default(), None);
        assert!(engine.is_transitioning());
        assert!(midway.glow.b > 0.05, "the crossfade should have started");

        let done = engine.update(0.2, AudioAggregates::default(), None);
        assert!(!engine.is_transitioning());
        assert!(done.glow.b > 0.9 && done.glow.r < 0.1);
    }

    #[test]
    fn reduced_motion_switches_palettes_immediately() {
        let mut config = config_with(RgbMode::Static);
        config.rgb.transition_ms = 5000;
        config.motion.reduced = true;
        config.rgb.primary = "#FF0000".to_string();
        let mut engine = ColorEngine::new(&config);

        let mut changed = config.clone();
        changed.rgb.primary = "#0000FF".to_string();
        assert!(engine.reconfigure(&changed));
        assert!(!engine.is_transitioning());
        let palette = engine.palette();
        assert!(palette.glow.b > 0.9);
    }

    #[test]
    fn an_unrelated_change_does_not_restart_the_transition() {
        let config = config_with(RgbMode::Static);
        let mut engine = ColorEngine::new(&config);

        let mut changed = config.clone();
        changed.clock.show_seconds = true;
        assert!(!engine.reconfigure(&changed));
        assert!(!engine.is_transitioning());
    }

    #[test]
    fn invalid_colours_and_numbers_degrade_without_panicking() {
        let mut config = config_with(RgbMode::Reactive);
        config.rgb.primary = "not a colour".to_string();
        config.rgb.secondary = "#GGGGGG".to_string();
        config.appearance.fill = "nope".to_string();
        config.text.color = "also nope".to_string();
        config.rgb.speed = f32::NAN;
        config.rgb.brightness = f32::INFINITY;
        config.rgb.breath_depth = f32::NAN;
        config.appearance.fill_gradient = f32::NAN;
        config.accessibility.contrast_target = f32::NAN;

        let mut engine = ColorEngine::new(&config);
        let palette = engine.update(f32::NAN, AudioAggregates::default(), None);
        assert!(palette.fill.r.is_finite());
        assert!(palette.glow.r.is_finite());
        assert!(palette.text.r.is_finite());
        assert!(palette.glow_intensity.is_finite());
    }

    #[test]
    fn update_does_not_accumulate_errors_over_a_long_run() {
        let mut engine = ColorEngine::new(&config_with(RgbMode::Cycling));
        for _ in 0..100_000 {
            let palette = engine.update(1.0 / 60.0, AudioAggregates::default(), None);
            assert!(palette.glow.r.is_finite());
            assert!(palette.glow.g.is_finite());
            assert!(palette.glow.b.is_finite());
        }
        assert!(engine.time_s() < 86_400.0);
        assert_eq!(engine.frames(), 100_000);
    }

    #[test]
    fn two_engines_with_the_same_inputs_agree() {
        let config = config_with(RgbMode::Reactive);
        let mut a = ColorEngine::new(&config);
        let mut b = ColorEngine::new(&config);
        for _ in 0..180 {
            let pa = a.update(1.0 / 60.0, audio(0.6), None);
            let pb = b.update(1.0 / 60.0, audio(0.6), None);
            assert_eq!(pa, pb);
        }
    }

    #[test]
    fn palette_keeps_the_configured_fill_alpha() {
        let mut config = config_with(RgbMode::Static);
        config.appearance.fill = "#11223380".to_string();
        let engine = ColorEngine::new(&config);
        let alpha = engine.palette().fill.a;
        assert!((alpha - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn a_gradient_adds_a_second_fill_stop() {
        let mut config = config_with(RgbMode::Static);
        config.appearance.fill = "#000000".to_string();
        config.appearance.fill_gradient = 1.0;
        config.rgb.primary = "#FF0000".to_string();
        let mut engine = ColorEngine::new(&config);
        let palette = engine.update(0.0, AudioAggregates::default(), None);
        assert!(
            palette.fill_accent.to_oklab().chroma() > palette.fill.to_oklab().chroma(),
            "a full gradient should move the accent stop"
        );
    }

    #[test]
    fn a_border_keeps_its_alpha_and_gains_the_accent_hue() {
        let mut config = config_with(RgbMode::Static);
        config.appearance.border_color = "#FFFFFF40".to_string();
        config.rgb.primary = "#FF0000".to_string();
        config.rgb.saturation = 1.0;
        let engine = ColorEngine::new(&config);
        let border = engine.palette().border;
        assert!((border.a - 64.0 / 255.0).abs() < 1e-6);
        assert!(border.r > border.g);
    }

    #[test]
    fn the_secondary_colour_is_validated_but_does_not_break_static_mode() {
        let mut config = config_with(RgbMode::Static);
        config.rgb.secondary = "#00FF00".to_string();
        config.rgb.primary = "#FF0000".to_string();
        let engine = ColorEngine::new(&config);
        let glow = engine.palette().glow;
        assert!(glow.r > 0.9);
        assert!(glow.g < 0.1);
    }

    #[test]
    fn mode_helpers_reflect_the_configuration() {
        let config = config_with(RgbMode::Cycling);
        let engine = ColorEngine::new(&config);
        assert_eq!(engine.mode(), RgbMode::Cycling);
        assert!(!engine.is_transitioning());
        assert_eq!(engine.frames(), 0);
    }

    #[test]
    fn a_zero_transition_is_immediate_even_without_reduced_motion() {
        let mut config = config_with(RgbMode::Static);
        config.rgb.transition_ms = 0;
        let mut engine = ColorEngine::new(&config);
        let mut changed = config.clone();
        changed.rgb.primary = "#FFFFFF".to_string();
        assert!(engine.reconfigure(&changed));
        assert!(!engine.is_transitioning());
    }

    #[test]
    fn negative_dt_is_ignored() {
        let mut engine = ColorEngine::new(&config_with(RgbMode::Cycling));
        let before = engine.time_s();
        engine.update(-5.0, AudioAggregates::default(), None);
        assert_eq!(engine.time_s(), before);
    }

    #[test]
    fn the_engine_size_is_reasonable_for_an_embedded_ui() {
        // Guards against accidentally storing per-frame sample buffers in the
        // engine. A config clone is the only large field and it is replaced on
        // reload, not per frame.
        assert!(
            std::mem::size_of::<ColorEngine>() < 8 * 1024,
            "ColorEngine grew to {} bytes",
            std::mem::size_of::<ColorEngine>()
        );
    }

    #[test]
    fn config_defaults_resolve_a_legible_palette() {
        let engine = ColorEngine::new(&Config::default());
        assert!(engine.palette().text_contrast() >= 4.49);
        assert!(engine.palette().dim_contrast() >= 4.49);
    }

    #[test]
    fn a_reactive_config_with_extreme_values_stays_in_range() {
        let mut config = config_with(RgbMode::Reactive);
        config.rgb.reactive.gain = 4.0;
        config.rgb.reactive.floor = 1.0;
        config.rgb.reactive.glow_from_bass = 2.0;
        config.rgb.glow_intensity = 2.0;
        let mut engine = ColorEngine::new(&config);
        let palette = engine.update(
            0.016,
            AudioAggregates {
                bass: 10.0,
                mid: -1.0,
                treble: f32::NAN,
                loudness: 99.0,
            },
            None,
        );
        assert!((0.0..=2.0).contains(&palette.glow_intensity));
        assert!(palette.glow.r.is_finite());
    }

    #[test]
    fn default_primary_and_secondary_are_used_when_colours_are_empty() {
        let mut config = RgbConfig::default();
        config.primary.clear();
        config.secondary.clear();
        let mut full = Config::default();
        full.rgb = config;
        let engine = ColorEngine::new(&full);
        assert!(engine.palette().glow.r.is_finite());
    }

    #[test]
    fn smoothstep_is_monotonic_and_pins_the_endpoints() {
        assert_eq!(smoothstep(0.0), 0.0);
        assert_eq!(smoothstep(1.0), 1.0);
        let mut previous = 0.0;
        for step in 0..=100 {
            let value = smoothstep(step as f32 / 100.0);
            assert!(value >= previous);
            previous = value;
        }
    }
}
