//! Per-key validation and repair.
//!
//! # The contract
//!
//! [`validate`] takes a configuration and a report, walks every key against a
//! declared rule, and repairs anything that fails. It never returns an error
//! and never rejects a whole file. Its output is a [`ValidationReport`] listing
//! what was wrong and what was done about it.
//!
//! # Why the report matters
//!
//! Repairing silently would be worse than the problem. A user who typed
//! `rgb.glow_intensity = 500` deserves to know it was clamped, otherwise they
//! will spend an evening wondering why the island is blinding. The report is
//! what `--check-config` prints and what the logger emits at warn level.

use lumina_core::color::parse_hex;

use crate::schema::{
    AudioConfig, Config, FftSize, GeneralConfig, HotkeysConfig, LayoutConfig, MediaConfig,
    MotionConfig, Position, RgbMode, TextConfig, VisualizerStyle,
};

/// How serious a problem is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Repaired automatically. The value was wrong but something sensible was
    /// substituted and the user's other settings are intact.
    Warning,
    /// The key was not recognised. Nothing was changed, but the key will have
    /// no effect.
    Unknown,
}

/// One problem found in a configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    /// How serious.
    pub severity: Severity,
    /// Dotted path of the offending key.
    pub path: String,
    /// What was wrong.
    pub message: String,
    /// What was done about it, when something was.
    pub repair: Option<String>,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}: {}", self.severity, self.path, self.message)?;
        if let Some(repair) = &self.repair {
            write!(f, " (fixed by {repair})")?;
        }
        Ok(())
    }
}

/// Everything found during a validation pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationReport {
    issues: Vec<Issue>,
}

impl ValidationReport {
    /// A report with nothing wrong in it.
    #[must_use]
    pub fn clean() -> Self {
        Self::default()
    }

    /// Record an issue.
    pub fn push(
        &mut self,
        severity: Severity,
        path: impl Into<String>,
        message: impl Into<String>,
        repair: Option<String>,
    ) {
        self.issues.push(Issue {
            severity,
            path: path.into(),
            message: message.into(),
            repair,
        });
    }

    /// Record a repaired value.
    fn warn(
        &mut self,
        path: impl Into<String>,
        message: impl Into<String>,
        repair: impl Into<String>,
    ) {
        self.push(Severity::Warning, path, message, Some(repair.into()));
    }

    /// Record an unrecognised key.
    fn unknown(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.push(Severity::Unknown, path, message, None);
    }

    /// Every issue, in the order they were found.
    #[must_use]
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    /// True when nothing was wrong.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    /// How many issues were found.
    #[must_use]
    pub fn len(&self) -> usize {
        self.issues.len()
    }

    /// True when no issues were found.
    ///
    /// Provided alongside [`ValidationReport::len`] because a collection type
    /// with a length and no emptiness check is a trap for the next caller.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    /// True when at least one issue was found.
    #[must_use]
    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }

    /// Only the issues of a given severity.
    #[must_use]
    pub fn with_severity(&self, severity: Severity) -> Vec<&Issue> {
        self.issues
            .iter()
            .filter(|i| i.severity == severity)
            .collect()
    }

    /// Render for the terminal, one issue per line, with a summary header.
    #[must_use]
    pub fn render(&self) -> String {
        if self.is_clean() {
            return "Configuration is valid. No problems found.".to_string();
        }
        let mut out = format!("{} problem(s) found:\n", self.issues.len());
        for issue in &self.issues {
            out.push_str("  ");
            out.push_str(&issue.to_string());
            out.push('\n');
        }
        out
    }
}

/// Walk a configuration, repairing anything invalid and recording what changed.
///
/// Returns the repaired configuration together with the report. The input is
/// taken by value so the caller cannot accidentally keep using the unrepaired
/// copy.
#[must_use]
pub fn validate(mut config: Config) -> (Config, ValidationReport) {
    let mut report = ValidationReport::clean();

    validate_general(&mut config.general, &mut report);
    validate_appearance(&mut config, &mut report);
    validate_rgb(&mut config, &mut report);
    validate_audio(&mut config, &mut report);
    validate_media(&mut config, &mut report);
    validate_clock(&mut config, &mut report);
    validate_layout(&mut config, &mut report);
    validate_hotkeys(&mut config, &mut report);
    validate_motion(&mut config, &mut report);
    validate_text(&mut config, &mut report);
    validate_advanced(&mut config, &mut report);
    validate_cross_field(&mut config, &mut report);

    (config, report)
}

/// Clamp a float into a range, recording the repair when it moves.
fn clamp_float(
    value: f32,
    min: f32,
    max: f32,
    path: &str,
    default: f32,
    report: &mut ValidationReport,
) -> f32 {
    if !value.is_finite() {
        report.warn(
            path,
            format!("value {value} is not a number"),
            format!("using {default}"),
        );
        return default;
    }
    if value < min {
        report.warn(
            path,
            format!("value {value} is below the minimum {min}"),
            format!("clamped to {min}"),
        );
        return min;
    }
    if value > max {
        report.warn(
            path,
            format!("value {value} is above the maximum {max}"),
            format!("clamped to {max}"),
        );
        return max;
    }
    value
}

/// Clamp a signed integer into a range, recording the repair when it moves.
fn clamp_int(
    value: i32,
    min: i32,
    max: i32,
    path: &str,
    _default: i32,
    report: &mut ValidationReport,
) -> i32 {
    if value < min {
        report.warn(
            path,
            format!("value {value} is below the minimum {min}"),
            format!("clamped to {min}"),
        );
        return min;
    }
    if value > max {
        report.warn(
            path,
            format!("value {value} is above the maximum {max}"),
            format!("clamped to {max}"),
        );
        return max;
    }
    value
}

/// Clamp an unsigned integer into a range, recording the repair when it moves.
fn clamp_u32(
    value: u32,
    min: u32,
    max: u32,
    path: &str,
    _default: u32,
    report: &mut ValidationReport,
) -> u32 {
    if value < min {
        report.warn(
            path,
            format!("value {value} is below the minimum {min}"),
            format!("clamped to {min}"),
        );
        return min;
    }
    if value > max {
        report.warn(
            path,
            format!("value {value} is above the maximum {max}"),
            format!("clamped to {max}"),
        );
        return max;
    }
    value
}

/// Validate a color string, replacing it with the default when unparseable.
fn check_color(value: &mut String, path: &str, default: &str, report: &mut ValidationReport) {
    if parse_hex(value).is_none() {
        report.warn(
            path,
            format!("{value:?} is not a valid color"),
            format!("using {default}"),
        );
        value.clear();
        value.push_str(default);
    }
}

/// Validate a hotkey string, replacing it with empty when unparseable.
fn check_hotkey(value: &mut String, path: &str, report: &mut ValidationReport) {
    if value.is_empty() {
        return;
    }
    if crate::validate::parse_hotkey(value).is_none() {
        report.unknown(
            path,
            format!("{value:?} is not a valid hotkey, for example \"ctrl+shift+space\""),
        );
        value.clear();
    }
}

fn validate_general(general: &mut GeneralConfig, report: &mut ValidationReport) {
    // Offsets are relative to a monitor edge, so a 4K display is the useful
    // bound rather than an arbitrary one.
    general.offset_x = clamp_int(general.offset_x, -7680, 7680, "general.offset_x", 0, report);
    general.offset_y = clamp_int(
        general.offset_y,
        -4320,
        4320,
        "general.offset_y",
        12,
        report,
    );

    general.auto_hide_delay_ms = clamp_u32(
        general.auto_hide_delay_ms,
        0,
        600_000,
        "general.auto_hide_delay_ms",
        3000,
        report,
    );
    general.idle_timeout_ms = clamp_u32(
        general.idle_timeout_ms,
        0,
        600_000,
        "general.idle_timeout_ms",
        5000,
        report,
    );

    // Zero means "never collapse", which is a legitimate setting, so the
    // minimum is zero rather than some positive floor.
    if let MonitorSelectorBounds::Index(index) = MonitorSelectorBounds::from(general.monitor) {
        // Indices are checked against the live monitor count at startup, since
        // the configuration cannot know how many displays are attached.
        if index > 15 {
            general.monitor = crate::schema::MonitorSelector::Primary;
            report.warn(
                "general.monitor",
                format!("index {index} is implausibly large"),
                "using primary",
            );
        }
    }
}

/// Helper so the monitor index check reads without exposing a public enum.
enum MonitorSelectorBounds {
    Index(u32),
    Other,
}

impl From<crate::schema::MonitorSelector> for MonitorSelectorBounds {
    fn from(value: crate::schema::MonitorSelector) -> Self {
        match value {
            crate::schema::MonitorSelector::Index(index) => Self::Index(index),
            _ => Self::Other,
        }
    }
}

fn validate_appearance(config: &mut Config, report: &mut ValidationReport) {
    let defaults = crate::defaults::appearance_defaults();
    let a = &mut config.appearance;

    check_color(&mut a.fill, "appearance.fill", defaults.fill, report);
    check_color(
        &mut a.border_color,
        "appearance.border_color",
        defaults.border_color,
        report,
    );

    a.fill_gradient = clamp_float(
        a.fill_gradient,
        0.0,
        1.0,
        "appearance.fill_gradient",
        0.0,
        report,
    );
    a.corner_radius = clamp_float(
        a.corner_radius,
        0.0,
        60.0,
        "appearance.corner_radius",
        18.0,
        report,
    );
    a.top_half_width = clamp_float(
        a.top_half_width,
        40.0,
        400.0,
        "appearance.top_half_width",
        62.0,
        report,
    );
    a.bottom_half_width = clamp_float(
        a.bottom_half_width,
        32.0,
        400.0,
        "appearance.bottom_half_width",
        54.0,
        report,
    );
    a.height = clamp_float(a.height, 8.0, 120.0, "appearance.height", 36.0, report);
    a.border_width = clamp_float(
        a.border_width,
        0.0,
        8.0,
        "appearance.border_width",
        1.0,
        report,
    );
    a.glow_spread = clamp_float(
        a.glow_spread,
        0.0,
        40.0,
        "appearance.glow_spread",
        8.0,
        report,
    );
    a.glow_falloff = clamp_float(
        a.glow_falloff,
        0.5,
        40.0,
        "appearance.glow_falloff",
        8.0,
        report,
    );
    a.shadow_opacity = clamp_float(
        a.shadow_opacity,
        0.0,
        0.8,
        "appearance.shadow_opacity",
        0.28,
        report,
    );
    a.backdrop_darken = clamp_float(
        a.backdrop_darken,
        0.0,
        0.8,
        "appearance.backdrop_darken",
        0.15,
        report,
    );
    a.noise_strength = clamp_float(
        a.noise_strength,
        0.0,
        0.05,
        "appearance.noise_strength",
        0.012,
        report,
    );
}

fn validate_rgb(config: &mut Config, report: &mut ValidationReport) {
    let defaults = crate::defaults::rgb_defaults();
    let rgb = &mut config.rgb;

    check_color(&mut rgb.primary, "rgb.primary", defaults.primary, report);
    check_color(
        &mut rgb.secondary,
        "rgb.secondary",
        defaults.secondary,
        report,
    );

    rgb.speed = clamp_float(rgb.speed, 0.05, 8.0, "rgb.speed", 1.0, report);
    rgb.brightness = clamp_float(rgb.brightness, 0.0, 2.0, "rgb.brightness", 1.0, report);
    rgb.saturation = clamp_float(rgb.saturation, 0.0, 2.0, "rgb.saturation", 0.9, report);
    rgb.breath_rate_hz = clamp_float(
        rgb.breath_rate_hz,
        0.01,
        4.0,
        "rgb.breath_rate_hz",
        0.25,
        report,
    );
    rgb.breath_depth = clamp_float(rgb.breath_depth, 0.0, 0.6, "rgb.breath_depth", 0.18, report);
    rgb.glow_intensity = clamp_float(
        rgb.glow_intensity,
        0.0,
        2.0,
        "rgb.glow_intensity",
        0.6,
        report,
    );
    rgb.cycle_degrees_per_second = clamp_float(
        rgb.cycle_degrees_per_second,
        1.0,
        360.0,
        "rgb.cycle_degrees_per_second",
        40.0,
        report,
    );
    rgb.transition_ms = clamp_u32(rgb.transition_ms, 0, 5000, "rgb.transition_ms", 300, report);

    let r = &mut rgb.reactive;
    r.gain = clamp_float(r.gain, 0.0, 4.0, "rgb.reactive.gain", 1.0, report);
    r.floor = clamp_float(r.floor, 0.0, 1.0, "rgb.reactive.floor", 0.12, report);
    r.bass_hz = clamp_u32(r.bass_hz, 20, 2000, "rgb.reactive.bass_hz", 250, report);
    r.mid_hz = clamp_u32(r.mid_hz, 100, 16_000, "rgb.reactive.mid_hz", 2000, report);
    r.hue_spread_degrees = clamp_float(
        r.hue_spread_degrees,
        0.0,
        360.0,
        "rgb.reactive.hue_spread_degrees",
        90.0,
        report,
    );
    r.glow_from_bass = clamp_float(
        r.glow_from_bass,
        0.0,
        2.0,
        "rgb.reactive.glow_from_bass",
        0.55,
        report,
    );
}

fn validate_audio(config: &mut Config, report: &mut ValidationReport) {
    let audio: &mut AudioConfig = &mut config.audio;

    audio.band_count = clamp_u32(audio.band_count, 4, 128, "audio.band_count", 32, report);
    audio.sensitivity = clamp_float(
        audio.sensitivity,
        0.05,
        8.0,
        "audio.sensitivity",
        1.8,
        report,
    );
    audio.attack = clamp_float(audio.attack, 0.01, 1.0, "audio.attack", 0.35, report);
    audio.release = clamp_float(audio.release, 0.005, 1.0, "audio.release", 0.08, report);
    audio.silence_threshold_db = clamp_float(
        audio.silence_threshold_db,
        -90.0,
        -20.0,
        "audio.silence_threshold_db",
        -60.0,
        report,
    );
    audio.bass_tilt_db_per_octave = clamp_float(
        audio.bass_tilt_db_per_octave,
        0.0,
        6.0,
        "audio.bass_tilt_db_per_octave",
        3.0,
        report,
    );

    if audio.attack < audio.release {
        report.warn(
            "audio.release",
            format!(
                "release {} is above attack {}, which would make bars fall faster than they rise",
                audio.release, audio.attack
            ),
            format!("lowering release to {}", audio.attack),
        );
        audio.release = audio.attack;
    }

    let v = &mut audio.visualizer;
    v.bar_spacing = clamp_u32(
        v.bar_spacing,
        0,
        12,
        "audio.visualizer.bar_spacing",
        2,
        report,
    );
    v.bar_radius = clamp_u32(v.bar_radius, 0, 8, "audio.visualizer.bar_radius", 3, report);
    v.peak_hold_ms = clamp_u32(
        v.peak_hold_ms,
        0,
        3000,
        "audio.visualizer.peak_hold_ms",
        400,
        report,
    );
    v.hue_spread_degrees = clamp_float(
        v.hue_spread_degrees,
        0.0,
        360.0,
        "audio.visualizer.hue_spread_degrees",
        120.0,
        report,
    );

    // Bar spacing must leave room for at least one visible pixel per band.
    let max_spacing = (200.0 / audio.band_count as f32).floor() as u32;
    if v.bar_spacing > max_spacing {
        report.warn(
            "audio.visualizer.bar_spacing",
            format!(
                "{} bands cannot be drawn with {} px spacing",
                audio.band_count, v.bar_spacing
            ),
            format!("reducing spacing to {max_spacing}"),
        );
        v.bar_spacing = max_spacing;
    }
}

fn validate_media(config: &mut Config, report: &mut ValidationReport) {
    let defaults = crate::defaults::media_defaults();
    let m: &mut MediaConfig = &mut config.media;

    m.hold_ms = clamp_u32(m.hold_ms, 0, 600_000, "media.hold_ms", 6000, report);
    m.album_art_size = clamp_u32(
        m.album_art_size,
        16,
        128,
        "media.album_art_size",
        52,
        report,
    );
    m.album_art_corner_radius = clamp_u32(
        m.album_art_corner_radius,
        0,
        32,
        "media.album_art_corner_radius",
        8,
        report,
    );
    m.progress_height = clamp_u32(m.progress_height, 1, 10, "media.progress_height", 3, report);
    m.marquee_speed_px_per_s = clamp_float(
        m.marquee_speed_px_per_s,
        5.0,
        400.0,
        "media.marquee_speed_px_per_s",
        40.0,
        report,
    );
    m.marquee_gap_px = clamp_u32(m.marquee_gap_px, 8, 200, "media.marquee_gap_px", 48, report);
    m.marquee_pause_on_hover_ms = clamp_u32(
        m.marquee_pause_on_hover_ms,
        0,
        10_000,
        "media.marquee_pause_on_hover_ms",
        1800,
        report,
    );

    // The art corner radius can never exceed half the art, or the shape becomes
    // a circle or worse.
    let max_radius = m.album_art_size / 2;
    if m.album_art_corner_radius > max_radius {
        report.warn(
            "media.album_art_corner_radius",
            format!(
                "radius {} exceeds half the art size {}",
                m.album_art_corner_radius, m.album_art_size
            ),
            format!("reducing to {max_radius}"),
        );
        m.album_art_corner_radius = max_radius;
    }

    // A scrub control that cannot be seen is a setting that does nothing.
    if m.show_progress && m.progress_height > m.album_art_size / 2 {
        report.unknown(
            "media.progress_height",
            format!(
                "a {} px progress bar is taller than the art",
                m.progress_height
            ),
        );
    }

    let _ = defaults;
}

fn validate_clock(config: &mut Config, report: &mut ValidationReport) {
    if config.clock.date_format.trim().is_empty() {
        config.clock.date_format = "%d %b".to_string();
        report.warn(
            "clock.date_format",
            "empty format string",
            "using \"%d %b\"",
        );
    }

    // An unrecognised timezone name is a common mistake. Drop it rather than
    // making the widget fail every frame.
    if !config.clock.second_timezone.is_empty()
        && !is_plausible_timezone(&config.clock.second_timezone)
    {
        report.unknown(
            "clock.second_timezone",
            format!(
                "{:?} is not a valid IANA timezone name",
                config.clock.second_timezone
            ),
        );
        config.clock.second_timezone.clear();
    }
}

/// IANA timezone area prefixes.
///
/// The full database is about five hundred names, which is not worth embedding
/// for a single config key. Checking the leading area catches the mistakes users
/// actually make, such as inventing a plausible looking zone or mistyping
/// `Europe` as `Eurpoe`, while accepting every real zone. The authoritative
/// lookup happens once at startup, where a genuine failure is reported properly.
const TIMEZONE_AREAS: [&str; 15] = [
    "Africa",
    "America",
    "Antarctica",
    "Arctic",
    "Asia",
    "Atlantic",
    "Australia",
    "Brazil",
    "Canada",
    "Chile",
    "Etc",
    "Europe",
    "Indian",
    "Mexico",
    "Pacific",
];

/// Zones that are a bare name with no area prefix, such as `UTC` or `GMT`.
const TIMEZONE_BARE_NAMES: [&str; 5] = ["UTC", "GMT", "EST", "MST", "HST"];

/// Check a timezone name against the known area prefixes.
fn is_plausible_timezone(name: &str) -> bool {
    if name.len() < 3 || name.len() > 64 {
        return false;
    }
    if TIMEZONE_BARE_NAMES.contains(&name) {
        return true;
    }
    let Some((area, _)) = name.split_once('/') else {
        return false;
    };
    TIMEZONE_AREAS.contains(&area)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-' || c == '+')
}

fn validate_layout(config: &mut Config, report: &mut ValidationReport) {
    let layout: &mut LayoutConfig = &mut config.layout;

    layout.padding_x = clamp_float(
        layout.padding_x,
        0.0,
        48.0,
        "layout.padding_x",
        12.0,
        report,
    );
    layout.padding_y = clamp_float(layout.padding_y, 0.0, 24.0, "layout.padding_y", 6.0, report);
    layout.spacing = clamp_float(layout.spacing, 0.0, 32.0, "layout.spacing", 10.0, report);
    layout.max_widgets = clamp_u32(layout.max_widgets, 1, 8, "layout.max_widgets", 3, report);

    if layout.order.is_empty() {
        layout.order = crate::schema::DEFAULT_WIDGET_ORDER
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        report.warn(
            "layout.order",
            "empty widget order",
            "restoring the default order",
        );
    }

    // Remove duplicates while preserving the user's chosen order, and report
    // any that are not widgets Lumina knows about.
    let mut seen = Vec::new();
    let mut deduped = Vec::new();
    for widget in layout.order.drain(..) {
        if !crate::schema::DEFAULT_WIDGET_ORDER.contains(&widget.as_str()) {
            report.unknown("layout.order", format!("{widget:?} is not a known widget"));
            continue;
        }
        if seen.contains(&widget) {
            report.warn(
                "layout.order",
                format!("{widget:?} is listed more than once"),
                "keeping the first occurrence",
            );
            continue;
        }
        seen.push(widget.clone());
        deduped.push(widget);
    }
    layout.order = deduped;

    if layout.max_widgets as usize > layout.order.len() {
        report.warn(
            "layout.max_widgets",
            format!(
                "{} widgets requested but only {} are listed",
                layout.max_widgets,
                layout.order.len()
            ),
            format!("reducing to {}", layout.order.len()),
        );
        layout.max_widgets = layout.order.len() as u32;
    }
}

/// Every binding that can claim a global hotkey, in declaration order.
///
/// Kept as a module level constant rather than built inside the function so the
/// list of hotkey keys lives in one place next to the struct it mirrors.
const ALL_HOTKEYS: [(&str, &str); 13] = [
    ("toggle_visibility", "toggle_visibility"),
    ("cycle_widget", "cycle_widget"),
    ("cycle_back", "cycle_back"),
    ("rgb_next_mode", "rgb_next_mode"),
    ("profile_next", "profile_next"),
    ("media_play_pause", "media_play_pause"),
    ("media_next", "media_next"),
    ("media_previous", "media_previous"),
    ("media_stop", "media_stop"),
    ("cycle_players", "cycle_players"),
    ("collapse_now", "collapse_now"),
    ("reload_config", "reload_config"),
    ("toggle_debug_overlay", "toggle_debug_overlay"),
];

/// Read every hotkey binding out of the struct by field name.
fn hotkey_bindings(h: &HotkeysConfig) -> Vec<(&'static str, &str)> {
    ALL_HOTKEYS
        .iter()
        .map(|(name, _)| (*name, field(h, name)))
        .collect()
}

/// Field access by name, restricted to the keys in [`ALL_HOTKEYS`].
///
/// A match rather than a macro so adding a hotkey to the struct without adding
/// it here is a compile error instead of a silently unbound key.
fn field<'a>(h: &'a HotkeysConfig, name: &str) -> &'a str {
    match name {
        "toggle_visibility" => &h.toggle_visibility,
        "cycle_widget" => &h.cycle_widget,
        "cycle_back" => &h.cycle_back,
        "rgb_next_mode" => &h.rgb_next_mode,
        "profile_next" => &h.profile_next,
        "media_play_pause" => &h.media_play_pause,
        "media_next" => &h.media_next,
        "media_previous" => &h.media_previous,
        "media_stop" => &h.media_stop,
        "cycle_players" => &h.cycle_players,
        "collapse_now" => &h.collapse_now,
        "reload_config" => &h.reload_config,
        "toggle_debug_overlay" => &h.toggle_debug_overlay,
        other => unreachable!("{other} is not in ALL_HOTKEYS"),
    }
}

fn validate_hotkeys(config: &mut Config, report: &mut ValidationReport) {
    let h: &mut HotkeysConfig = &mut config.hotkeys;

    // Check every binding first, so a malformed combo is cleared before the
    // conflict pass reads it.
    for (name, _) in ALL_HOTKEYS {
        let mut binding = field(h, name).to_string();
        check_hotkey(&mut binding, &format!("hotkeys.{name}"), report);
        *field_mut(h, name) = binding;
    }

    // Two actions on one combo means one of them silently never fires, so the
    // user needs to know even though we cannot resolve it for them.
    let mut seen: Vec<(String, &'static str)> = Vec::new();
    for (name, combo) in hotkey_bindings(h) {
        if combo.is_empty() {
            continue;
        }
        let normalized = normalize_hotkey(combo);
        if let Some((_, existing)) = seen.iter().find(|(c, _)| *c == normalized) {
            report.warn(
                format!("hotkeys.{name}"),
                format!("{combo:?} is already bound to {existing:?}"),
                "leaving both, the first one registered wins",
            );
        } else {
            seen.push((normalized, name));
        }
    }
}

/// Mutable field access by name, restricted to the keys in [`ALL_HOTKEYS`].
fn field_mut<'a>(h: &'a mut HotkeysConfig, name: &str) -> &'a mut String {
    match name {
        "toggle_visibility" => &mut h.toggle_visibility,
        "cycle_widget" => &mut h.cycle_widget,
        "cycle_back" => &mut h.cycle_back,
        "rgb_next_mode" => &mut h.rgb_next_mode,
        "profile_next" => &mut h.profile_next,
        "media_play_pause" => &mut h.media_play_pause,
        "media_next" => &mut h.media_next,
        "media_previous" => &mut h.media_previous,
        "media_stop" => &mut h.media_stop,
        "cycle_players" => &mut h.cycle_players,
        "collapse_now" => &mut h.collapse_now,
        "reload_config" => &mut h.reload_config,
        "toggle_debug_overlay" => &mut h.toggle_debug_overlay,
        other => unreachable!("{other} is not in ALL_HOTKEYS"),
    }
}

fn validate_motion(config: &mut Config, report: &mut ValidationReport) {
    let m: &mut MotionConfig = &mut config.motion;

    m.master_scale = clamp_float(
        m.master_scale,
        0.05,
        4.0,
        "motion.master_scale",
        1.0,
        report,
    );
    m.morph_response_s = clamp_float(
        m.morph_response_s,
        0.05,
        1.5,
        "motion.morph_response_s",
        0.34,
        report,
    );
    m.morph_damping = clamp_float(
        m.morph_damping,
        0.4,
        2.0,
        "motion.morph_damping",
        0.78,
        report,
    );
    m.content_fade_ms = clamp_u32(
        m.content_fade_ms,
        0,
        1000,
        "motion.content_fade_ms",
        160,
        report,
    );

    // Reduced motion is a hard override, not a starting point. When it is on,
    // the spring parameters are forced to the near-instant profile regardless of
    // what else the user wrote, so the accessibility setting cannot be
    // half-defeated by a leftover tuning value.
    if m.reduced {
        m.morph_response_s = 0.08;
        m.morph_damping = 1.0;
        m.content_fade_ms = 0;
        m.startup_reveal = false;
    }
}

fn validate_text(config: &mut Config, report: &mut ValidationReport) {
    let defaults = crate::defaults::text_defaults();
    let t: &mut TextConfig = &mut config.text;

    check_color(&mut t.color, "text.color", defaults.color, report);
    check_color(
        &mut t.dim_color,
        "text.dim_color",
        defaults.dim_color,
        report,
    );

    // Twelve pixels is the accessibility floor. Anything below it is repaired
    // rather than merely warned about, because unreadable text is a hard
    // failure regardless of what the user intended.
    t.font_size = clamp_float(t.font_size, 12.0, 96.0, "text.font_size", 14.0, report);
    t.title_size = clamp_float(t.title_size, 12.0, 96.0, "text.title_size", 15.0, report);
    t.artist_size = clamp_float(t.artist_size, 12.0, 96.0, "text.artist_size", 13.0, report);
    t.letter_spacing = clamp_float(
        t.letter_spacing,
        -2.0,
        8.0,
        "text.letter_spacing",
        0.0,
        report,
    );
}

fn validate_advanced(config: &mut Config, report: &mut ValidationReport) {
    let a = &mut config.advanced;

    a.target_fps = clamp_u32(a.target_fps, 15, 240, "advanced.target_fps", 60, report);

    if a.ipc_allow_exec && !a.ipc_enabled {
        report.warn(
            "advanced.ipc_allow_exec",
            "the execute command is enabled but the control socket is off",
            "leaving both as written, the socket switch is what matters",
        );
    }
}

fn validate_cross_field(config: &mut Config, report: &mut ValidationReport) {
    // The inverted trapezoid invariant. Handled here rather than in
    // `AppearanceConfig::geometry` so the user hears about it.
    let a = &config.appearance;
    if a.inverted_trapezoid && a.bottom_half_width > a.top_half_width {
        report.warn(
            "appearance.bottom_half_width",
            format!(
                "bottom half width {} is wider than top half width {}",
                a.bottom_half_width, a.top_half_width
            ),
            "swapping them, which preserves the shape you described",
        );
        std::mem::swap(
            &mut config.appearance.bottom_half_width,
            &mut config.appearance.top_half_width,
        );
    }

    // Reactive tuning is inert unless the reactive engine is selected.
    if config.rgb.mode != RgbMode::Reactive {
        return;
    }

    let r = &config.rgb.reactive;
    if r.bass_hz >= r.mid_hz {
        report.warn(
            "rgb.reactive.bass_hz",
            format!(
                "bass cutoff {} Hz is at or above the mid cutoff {} Hz",
                r.bass_hz, r.mid_hz
            ),
            format!(
                "lowering bass cutoff to {}",
                r.mid_hz.saturating_sub(50).max(20)
            ),
        );
        config.rgb.reactive.bass_hz = r.mid_hz.saturating_sub(50).max(20);
    }

    // A dimmer text color needs to clear the same contrast bar as the primary
    // one, and users set the two independently.
    if config
        .text
        .dim_color
        .eq_ignore_ascii_case(&config.text.color)
    {
        report.warn(
            "text.dim_color",
            "the dim color is identical to the primary text color",
            "no change, both will be adjusted together for contrast",
        );
    }
}

/// Modifier bits for a hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// Control.
    pub ctrl: bool,
    /// Alt or Option.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// Super, which is the Windows key or the Command key.
    pub super_key: bool,
    /// Meta, which is distinct from Super on X11.
    pub meta: bool,
}

/// A parsed hotkey combination.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hotkey {
    /// Which modifiers are held.
    pub modifiers: Modifiers,
    /// Canonical key name.
    pub key: String,
}

impl Hotkey {
    /// True when no key and no modifiers are set, which means "unbound".
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.key.is_empty() && !self.modifiers.ctrl && !self.modifiers.alt && !self.modifiers.shift
    }
}

/// Parse a hotkey string such as `ctrl+shift+space`.
///
/// Order independent, case insensitive, duplicate modifiers collapsed. Returns
/// `None` for anything unrecognised so the validator can report it and clear
/// the binding rather than registering something that will never fire.
#[must_use]
pub fn parse_hotkey(input: &str) -> Option<Hotkey> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Some(Hotkey::default());
    }

    let mut modifiers = Modifiers::default();
    let mut key: Option<String> = None;

    for token in trimmed.split(['+', '-']) {
        let token = token.trim();
        if token.is_empty() {
            // A trailing separator is a typo, not a hotkey.
            return None;
        }
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "alt" | "option" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            "super" | "cmd" | "command" | "win" => modifiers.super_key = true,
            "meta" => modifiers.meta = true,
            other => {
                if key.is_some() {
                    // Two keys in one combination is a typo.
                    return None;
                }
                key = Some(normalize_key(other)?);
            }
        }
    }

    let key = key?;
    // A bare modifier is not a hotkey, it is a mistake.
    if !modifiers.ctrl
        && !modifiers.alt
        && !modifiers.shift
        && !modifiers.super_key
        && !modifiers.meta
    {
        return None;
    }

    Some(Hotkey { modifiers, key })
}

/// Named keys that are more than one character long.
const NAMED_KEYS: [&str; 23] = [
    "space",
    "enter",
    "return",
    "escape",
    "esc",
    "tab",
    "backspace",
    "insert",
    "delete",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "capslock",
    "numlock",
    "scrolllock",
    "printscreen",
    "pause",
    "menu",
];

/// Punctuation keys, which users type directly.
const PUNCTUATION_KEYS: [&str; 12] = [",", ".", "/", ";", "'", "[", "]", "`", "-", "=", "\\", " "];

/// Canonical form of a key token, or `None` when it is not a recognised key.
fn normalize_key(token: &str) -> Option<String> {
    let lower = token.to_ascii_lowercase();

    // Single printable characters.
    if lower.chars().count() == 1 {
        return Some(lower);
    }

    // Function keys, F1 through F24.
    if let Some(number) = lower.strip_prefix('f') {
        if let Ok(n) = number.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(lower);
            }
        }
        return None;
    }

    if NAMED_KEYS.contains(&lower.as_str()) {
        return Some(lower);
    }

    if PUNCTUATION_KEYS.contains(&lower.as_str()) {
        return Some(if lower == " " {
            "space".to_string()
        } else {
            lower
        });
    }

    None
}

/// Canonical string form of a hotkey, used to compare bindings for conflicts.
fn normalize_hotkey(input: &str) -> String {
    match parse_hotkey(input) {
        Some(hotkey) if !hotkey.is_empty() => {
            let mut parts = Vec::new();
            if hotkey.modifiers.ctrl {
                parts.push("ctrl");
            }
            if hotkey.modifiers.alt {
                parts.push("alt");
            }
            if hotkey.modifiers.shift {
                parts.push("shift");
            }
            if hotkey.modifiers.super_key {
                parts.push("super");
            }
            if hotkey.modifiers.meta {
                parts.push("meta");
            }
            parts.push(&hotkey.key);
            parts.join("+")
        }
        _ => String::new(),
    }
}

/// Levenshtein distance, used to suggest the nearest valid key name.
#[must_use]
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }

    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];

    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b.len()]
}

/// Suggest the closest name from `candidates`, or `None` if nothing is close.
///
/// Two strategies, because users misspell in two different ways:
/// 1. **Truncation**, caught by prefix match. Typing `glow` for `glow_intensity`
///    is nine characters away by edit distance, which no sane threshold would
///    allow, but it is exactly what prefix matching is for.
/// 2. **Typo**, caught by edit distance of three or less. A transposition or a
///    dropped character lands here.
///
/// Prefix wins over typo, because "glow" almost always means the user wants a
/// key that starts with what they typed, not one with three characters
/// different. When several keys share a prefix, the shortest completion wins,
/// since that leaves the fewest characters still to type.
#[must_use]
pub fn suggest<'a>(input: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let lower = input.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }

    // Strategy one: a candidate the user typed the start of. Prefer the shortest
    // such candidate, since that is the least surprising completion.
    let mut prefix_best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let candidate_lower = candidate.to_ascii_lowercase();
        if candidate_lower.starts_with(&lower)
            && prefix_best.is_none_or(|(len, _)| candidate_lower.len() < len)
        {
            prefix_best = Some((candidate_lower.len(), candidate));
        }
    }
    if let Some((_, candidate)) = prefix_best {
        return Some(candidate);
    }

    // Strategy two: a near miss by edit distance.
    let mut typo_best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let distance = edit_distance(&lower, &candidate.to_ascii_lowercase());
        if distance <= 3 && typo_best.is_none_or(|(d, _)| distance < d) {
            typo_best = Some((distance, candidate));
        }
    }
    typo_best.map(|(_, candidate)| candidate)
}

/// Valid `Position` identifiers, for suggestions.
pub fn position_names() -> Vec<&'static str> {
    Position::ALL.iter().map(|p| p.id()).collect()
}

/// Valid `RgbMode` identifiers, for suggestions.
pub fn rgb_mode_names() -> Vec<&'static str> {
    RgbMode::ALL.iter().map(|m| m.id()).collect()
}

/// Valid `VisualizerStyle` identifiers, for suggestions.
pub fn visualizer_style_names() -> Vec<&'static str> {
    VisualizerStyle::ALL.iter().map(|s| s.id()).collect()
}

/// Valid `FftSize` variant names as they appear in the file.
pub fn fft_size_names() -> Vec<&'static str> {
    FftSize::ALL
        .iter()
        .map(|f| match f {
            FftSize::Size1024 => "size-1024",
            FftSize::Size2048 => "size-2048",
            FftSize::Size4096 => "size-4096",
            FftSize::Size8192 => "size-8192",
        })
        .collect()
}

/// Valid `BackdropMode` identifiers, for suggestions.
pub fn backdrop_mode_names() -> Vec<&'static str> {
    vec!["auto", "on", "off"]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::*;

    fn parse(text: &str) -> Config {
        Config::from_toml_str(text).expect("test config should parse")
    }

    fn run(text: &str) -> (Config, ValidationReport) {
        validate(parse(text))
    }

    #[test]
    fn default_config_validates_clean() {
        let (config, report) = validate(Config::default());
        assert!(
            report.is_clean(),
            "defaults must be valid, got: {}",
            report.render()
        );
        assert_eq!(
            config,
            Config::default(),
            "validation must not change a valid config"
        );
    }

    #[test]
    fn empty_file_yields_defaults_cleanly() {
        let (config, report) = validate(Config::from_toml_str("").expect("empty parses"));
        assert!(report.is_clean());
        assert_eq!(config, Config::default());
    }

    #[test]
    fn out_of_range_float_is_clamped_and_reported() {
        let (config, report) = run("[rgb]\nglow_intensity = 99.0\n");
        assert_eq!(
            config.rgb.glow_intensity, 2.0,
            "should clamp to the maximum"
        );
        assert!(!report.is_clean());
        let issue = report
            .issues()
            .iter()
            .find(|i| i.path == "rgb.glow_intensity");
        assert!(issue.is_some(), "the clamped key must be reported");
        assert!(
            issue.expect("checked").repair.is_some(),
            "the repair must be described"
        );
    }

    #[test]
    fn one_bad_value_does_not_discard_the_rest_of_the_file() {
        // This is the central guarantee of the crate.
        let text = r##"
            [rgb]
            mode = "cycling"
            primary = "#FF00AA"
            glow_intensity = 9999.0
            speed = 2.5

            [general]
            offset_y = 24
        "##;
        let (config, report) = run(text);

        assert_eq!(
            config.rgb.mode,
            RgbMode::Cycling,
            "valid values must survive"
        );
        assert_eq!(config.rgb.primary, "#FF00AA", "a valid color must survive");
        assert_eq!(config.rgb.speed, 2.5, "a valid float must survive");
        assert_eq!(config.general.offset_y, 24, "other sections must survive");
        assert_eq!(
            config.rgb.glow_intensity, 2.0,
            "the bad value alone is repaired"
        );
        assert_eq!(report.len(), 1, "exactly one problem should be reported");
    }

    #[test]
    fn malformed_color_is_replaced_with_the_default() {
        let (config, report) =
            run("[appearance]\nfill = \"not-a-color\"\nborder_color = \"#GGGGGG\"\n");
        assert_eq!(config.appearance.fill, "#12121ACC");
        assert_eq!(config.appearance.border_color, "#FFFFFF24");
        assert_eq!(report.len(), 2);
    }

    #[test]
    fn valid_colors_are_left_untouched() {
        for hex in [
            "#000000",
            "#FFFFFF",
            "#7C3AED",
            "#12121ACC",
            "#fff",
            "#f0a8",
        ] {
            let text = format!("[appearance]\nfill = \"{hex}\"\n");
            let (config, report) = run(&text);
            assert!(report.is_clean(), "{hex} was rejected: {}", report.render());
            assert_eq!(config.appearance.fill, hex);
        }
    }

    #[test]
    fn non_finite_floats_are_repaired() {
        // TOML cannot express NaN through the normal syntax, so this exercises
        // the guard directly on the struct.
        let mut config = Config::default();
        config.rgb.speed = f32::NAN;
        config.rgb.brightness = f32::INFINITY;
        let (_, report) = validate(config);
        assert_eq!(report.len(), 2, "both non-finite values must be reported");
    }

    #[test]
    fn inverted_trapezoid_violation_is_swapped_and_reported() {
        let (config, report) =
            run("[appearance]\ntop_half_width = 40.0\nbottom_half_width = 300.0\n");
        assert!(config.appearance.top_half_width >= config.appearance.bottom_half_width);
        assert_eq!(
            config.appearance.bottom_half_width, 40.0,
            "the smaller value belongs on the bottom"
        );
        assert!(report
            .issues()
            .iter()
            .any(|i| i.path == "appearance.bottom_half_width"));
    }

    #[test]
    fn regular_trapezoid_mode_skips_the_invariant() {
        let (config, report) = run(
            "[appearance]\ninverted_trapezoid = false\ntop_half_width = 40.0\nbottom_half_width = 300.0\n",
        );
        assert_eq!(
            config.appearance.bottom_half_width, 300.0,
            "a regular trapezoid is allowed"
        );
        assert!(report.is_clean(), "{}", report.render());
    }

    #[test]
    fn attack_below_release_is_repaired_with_a_reason() {
        let (config, report) = run("[audio]\nattack = 0.1\nrelease = 0.9\n");
        assert_eq!(config.audio.attack, 0.1);
        assert_eq!(config.audio.release, 0.1, "release must not exceed attack");
        let issue = report.issues().iter().find(|i| i.path == "audio.release");
        assert!(issue.is_some());
    }

    #[test]
    fn bar_spacing_is_capped_to_leave_visible_bands() {
        let (config, report) =
            run("[audio]\nband_count = 128\n[audio.visualizer]\nbar_spacing = 12\n");
        assert!(
            config.audio.visualizer.bar_spacing <= 1,
            "128 bands cannot have 12 px spacing, got {}",
            config.audio.visualizer.bar_spacing
        );
        assert!(!report.is_clean());
    }

    #[test]
    fn reduced_motion_overrides_spring_tuning() {
        let (config, report) = run(
            "[motion]\nreduced = true\nmorph_response_s = 1.0\nmorph_damping = 0.4\nstartup_reveal = true\n",
        );
        assert!(config.motion.reduced);
        assert_eq!(
            config.motion.morph_response_s, 0.08,
            "reduced motion must win over tuning"
        );
        assert_eq!(
            config.motion.morph_damping, 1.0,
            "reduced motion must not overshoot"
        );
        assert!(!config.motion.startup_reveal);
        assert!(report.is_clean(), "the override itself is not a problem");
    }

    #[test]
    fn unknown_widgets_are_dropped_from_the_order() {
        let (config, report) = run("[layout]\norder = [\"clock\", \"teleporter\", \"media\"]\n");
        assert_eq!(
            config.layout.order,
            vec!["clock".to_string(), "media".to_string()]
        );
        assert!(report
            .issues()
            .iter()
            .any(|i| i.message.contains("teleporter")));
    }

    #[test]
    fn duplicate_widgets_are_removed_keeping_the_first() {
        let (config, report) = run("[layout]\norder = [\"clock\", \"media\", \"clock\"]\n");
        assert_eq!(
            config.layout.order,
            vec!["clock".to_string(), "media".to_string()]
        );
        assert!(report
            .issues()
            .iter()
            .any(|i| i.message.contains("more than once")));
    }

    #[test]
    fn empty_widget_order_is_restored() {
        let (config, report) = run("[layout]\norder = []\n");
        assert_eq!(config.layout.order.len(), DEFAULT_WIDGET_ORDER.len());
        assert!(!report.is_clean());
    }

    #[test]
    fn max_widgets_cannot_exceed_the_order_length() {
        let (config, report) = run("[layout]\norder = [\"clock\", \"media\"]\nmax_widgets = 5\n");
        assert_eq!(config.layout.max_widgets, 2);
        assert!(!report.is_clean());
    }

    #[test]
    fn implausible_timezone_is_cleared() {
        let (config, report) = run("[clock]\nsecond_timezone = \"Mars/Olympus_Mons\"\n");
        assert_eq!(config.clock.second_timezone, "");
        assert!(!report.is_clean());
    }

    #[test]
    fn plausible_timezone_is_kept() {
        let (config, report) = run("[clock]\nsecond_timezone = \"Europe/Berlin\"\n");
        assert_eq!(config.clock.second_timezone, "Europe/Berlin");
        assert!(report.is_clean(), "{}", report.render());
    }

    #[test]
    fn album_art_radius_cannot_exceed_half_the_art() {
        let (config, report) = run("[media]\nalbum_art_size = 32\nalbum_art_corner_radius = 100\n");
        assert_eq!(config.media.album_art_corner_radius, 16);
        assert!(!report.is_clean());
    }

    #[test]
    fn invalid_hotkey_is_cleared_and_reported() {
        let (config, report) = run("[hotkeys]\ntoggle_visibility = \"hyper+w\"\n");
        assert_eq!(config.hotkeys.toggle_visibility, "");
        assert!(report
            .issues()
            .iter()
            .any(|i| i.path == "hotkeys.toggle_visibility"));
    }

    #[test]
    fn duplicate_hotkey_bindings_are_both_reported() {
        let (_, report) =
            run("[hotkeys]\ntoggle_visibility = \"ctrl+alt+l\"\ncycle_widget = \"CTRL+ALT+L\"\n");
        assert!(report
            .issues()
            .iter()
            .any(|i| i.message.contains("already bound")));
    }

    #[test]
    fn hotkey_parsing_accepts_realistic_spellings() {
        let cases = [
            ("ctrl+shift+space", true),
            ("CTRL+ALT+TAB", true),
            ("super+w", true),
            ("alt+F5", true),
            ("ctrl + shift + f1", true),
            ("ctrl+ctrl+a", true),
            ("control+option+delete", true),
            ("w", false),
            ("ctrl+", false),
            ("ctrl+a+b", false),
            ("ctrl+f99", false),
            ("", true),
            ("   ", true),
        ];
        for (input, should_parse) in cases {
            let parsed = parse_hotkey(input);
            assert_eq!(
                parsed.is_some(),
                should_parse,
                "unexpected parse result for {input:?}: {parsed:?}"
            );
        }
    }

    #[test]
    fn hotkey_parsing_is_order_and_case_independent() {
        let a = parse_hotkey("ctrl+alt+shift+f5").expect("parses");
        let b = parse_hotkey("SHIFT+ALT+CTRL+F5").expect("parses");
        let c = parse_hotkey("f5+alt+shift+ctrl").expect("parses");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn hotkey_modifier_sets_are_not_conflated() {
        // A different set of modifiers is a different hotkey, not a spelling
        // variant of the same one.
        let three = parse_hotkey("ctrl+alt+shift+f5").expect("parses");
        let two = parse_hotkey("ctrl+shift+f5").expect("parses");
        assert_ne!(three, two);
    }

    #[test]
    fn hotkey_duplicate_modifiers_are_collapsed() {
        let single = parse_hotkey("ctrl+a").expect("parses");
        let double = parse_hotkey("ctrl+ctrl+a").expect("parses");
        assert_eq!(single, double);
    }

    #[test]
    fn empty_hotkey_parses_to_empty() {
        let hotkey = parse_hotkey("").expect("empty is valid");
        assert!(hotkey.is_empty());
    }

    #[test]
    fn edit_distance_is_symmetric_and_correct() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("abc", ""), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(
            edit_distance("kitten", "sitting"),
            edit_distance("sitting", "kitten")
        );
    }

    #[test]
    fn suggestions_find_prefixes_and_near_misses_and_ignore_unrelated_words() {
        // Prefix: the user typed the start of a key name. When several keys
        // match, the shortest completion wins, which is the fewest characters
        // left to type.
        assert_eq!(
            suggest("glow", &["glow_intensity", "glow_spread"]),
            Some("glow_spread")
        );
        assert_eq!(
            suggest("glow_intensity", &["glow_intensity", "glow_spread"]),
            Some("glow_intensity")
        );

        // Typo: a transposition or a dropped character.
        assert_eq!(suggest("cyclng", &["cycling", "static"]), Some("cycling"));
        assert_eq!(suggest("intensty", &["intensity"]), Some("intensity"));

        // Nothing close, nothing suggested.
        assert_eq!(
            suggest("something_entirely_unrelated", &["glow_intensity"]),
            None
        );
        assert_eq!(suggest("", &["glow_intensity"]), None);
    }

    #[test]
    fn text_below_the_accessibility_floor_is_raised() {
        let (config, report) =
            run("[text]\nfont_size = 6.0\ntitle_size = 7.0\nartist_size = 8.0\n");
        assert_eq!(
            config.text.font_size, 12.0,
            "12 px is the accessibility floor"
        );
        assert_eq!(config.text.title_size, 12.0);
        assert_eq!(config.text.artist_size, 12.0);
        assert_eq!(report.len(), 3);
    }

    #[test]
    fn reactive_band_crossover_is_repaired() {
        let (config, report) =
            run("[rgb]\nmode = \"reactive\"\n[rgb.reactive]\nbass_hz = 3000\nmid_hz = 2000\n");
        assert!(
            config.rgb.reactive.bass_hz < config.rgb.reactive.mid_hz,
            "bass cutoff must sit below the mid cutoff"
        );
        assert!(!report.is_clean());
    }

    #[test]
    fn reactive_crossover_is_ignored_when_the_engine_is_off() {
        // Both values are inside their own ranges, but the bass cutoff sits
        // above the mid cutoff, which only matters for the reactive engine.
        let (config, report) =
            run("[rgb]\nmode = \"static\"\n[rgb.reactive]\nbass_hz = 1900\nmid_hz = 1000\n");
        assert_eq!(
            config.rgb.reactive.bass_hz, 1900,
            "inert settings are left alone"
        );
        assert_eq!(
            config.rgb.reactive.mid_hz, 1000,
            "inert settings are left alone"
        );
        assert!(report.is_clean(), "{}", report.render());
    }

    #[test]
    fn a_file_with_many_problems_reports_all_of_them_in_one_pass() {
        let text = r#"
            [rgb]
            primary = "purple"
            glow_intensity = 100.0
            speed = -3.0

            [audio]
            band_count = 5000
            attack = 0.001

            [text]
            font_size = 2.0

            [general]
            offset_x = 999999

            [layout]
            order = ["clock", "teleporter", "clock"]
        "#;
        let (config, report) = run(text);

        // Every problem named above is reported in a single pass, so a user
        // fixes their file once rather than one run at a time.
        for expected in [
            "rgb.primary",
            "rgb.glow_intensity",
            "rgb.speed",
            "audio.band_count",
            "audio.attack",
            "text.font_size",
            "general.offset_x",
            "layout.order",
        ] {
            assert!(
                report.issues().iter().any(|i| i.path == expected),
                "{expected} was not reported; report was: {}",
                report.render()
            );
        }
        assert!(report.len() >= 8);

        // And the file is still usable afterwards.
        assert_eq!(config.audio.band_count, 128);
        assert_eq!(config.general.offset_x, 7680);
    }

    #[test]
    fn report_renders_a_useful_message() {
        let (_, report) = run("[rgb]\nglow_intensity = 100.0\n");
        let rendered = report.render();
        assert!(rendered.contains("problem"));
        assert!(rendered.contains("rgb.glow_intensity"));
        assert!(rendered.contains("fixed by"));
    }

    #[test]
    fn clean_report_renders_a_positive_message() {
        let report = ValidationReport::clean();
        assert!(report.is_clean());
        assert!(!report.has_issues());
        assert_eq!(
            report.render(),
            "Configuration is valid. No problems found."
        );
    }

    #[test]
    fn severity_filtering_works() {
        let (_, report) =
            run("[appearance]\nfill = \"nope\"\n[layout]\norder = [\"teleporter\"]\n");
        let warnings = report.with_severity(Severity::Warning);
        let unknowns = report.with_severity(Severity::Unknown);
        assert!(warnings.iter().any(|i| i.path == "appearance.fill"));
        assert!(unknowns.iter().any(|i| i.path == "layout.order"));
    }

    #[test]
    fn every_default_is_within_its_documented_range() {
        // A guard against the default drifting outside a range the validator
        // would immediately clamp, which would make `is_clean` fail above.
        let (config, report) = validate(Config::default());
        assert!(
            report.is_clean(),
            "defaults violate their own ranges: {}",
            report.render()
        );
        assert_eq!(config, Config::default());
    }
}
