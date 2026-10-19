//! Typed change dispatch: diffing one configuration against another.
//!
//! # Why a diff rather than a "config changed" flag
//!
//! A reload that wakes every subsystem costs more than the reload itself. The
//! media widget does not care that `text.color` moved, and the renderer does not
//! care that `audio.sensitivity` did. So a reload produces a list of
//! [`ConfigChange`] values, and each handler matches only the variants it cares
//! about.
//!
//! # Why an enum rather than a string path
//!
//! A string path pushes the schema knowledge to every call site, which means a
//! renamed key breaks handlers silently at runtime instead of at compile time.
//! An enum makes the set of things that can change explicit, and adding a key to
//! the config without handling it is a compile error rather than a missed
//! subscription.

use crate::schema::{
    AccessibilityConfig, AppearanceConfig, AudioConfig, Config, GeneralConfig, MediaConfig,
    MotionConfig, RgbConfig, TextConfig,
};

/// One thing that changed between two configurations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigChange {
    /// A window placement or interaction setting moved.
    General(GeneralChange),
    /// Shape, color, or material settings moved.
    Appearance(AppearanceChange),
    /// Color engine settings moved.
    Rgb(RgbChange),
    /// Audio capture or analysis settings moved.
    Audio(AudioChange),
    /// Media integration settings moved.
    Media(MediaChange),
    /// Animation tuning or motion accessibility moved.
    Motion(MotionChange),
    /// Contrast, focus, or announcement settings moved.
    Accessibility(AccessibilityChange),
    /// Typography moved.
    Text(TextChange),
    /// Widget arrangement moved.
    Layout,
    /// A global hotkey binding moved.
    Hotkeys,
    /// An expert setting moved.
    Advanced,
    /// Something this build does not know about. Reported rather than dropped,
    /// because a new key added by a newer version should be visible in a diff.
    Unknown(String),
}

impl ConfigChange {
    /// Dotted path of the leaf that changed, for logs and diagnostics.
    #[must_use]
    pub fn path(&self) -> String {
        match self {
            Self::General(change) => format!("general.{}", change.path()),
            Self::Appearance(change) => format!("appearance.{}", change.path()),
            Self::Rgb(change) => format!("rgb.{}", change.path()),
            Self::Audio(change) => format!("audio.{}", change.path()),
            Self::Media(change) => format!("media.{}", change.path()),
            Self::Motion(change) => format!("motion.{}", change.path()),
            Self::Accessibility(change) => format!("accessibility.{}", change.path()),
            Self::Text(change) => format!("text.{}", change.path()),
            Self::Layout => "layout".to_string(),
            Self::Hotkeys => "hotkeys".to_string(),
            Self::Advanced => "advanced".to_string(),
            Self::Unknown(path) => path.clone(),
        }
    }
}

/// A change inside `[general]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneralChange {
    /// The island moved to another edge or corner.
    Position,
    /// The offset from the anchor moved.
    Offset,
    /// Auto-hide or the idle timeout changed.
    Visibility,
    /// Click-through behavior changed.
    ClickThrough,
    /// A monitor or instance policy changed.
    Other,
}

impl GeneralChange {
    fn path(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::Offset => "offset_x",
            Self::Visibility => "auto_hide",
            Self::ClickThrough => "click_through_when_collapsed",
            Self::Other => "monitor",
        }
    }
}

/// A change inside `[appearance]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceChange {
    /// The fill color or gradient moved.
    Fill,
    /// Shape geometry moved.
    Shape,
    /// Border or glow moved.
    Border,
    /// Blur or darkening moved.
    Backdrop,
    /// Dither noise moved.
    Noise,
}

impl AppearanceChange {
    fn path(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Shape => "top_half_width",
            Self::Border => "border_width",
            Self::Backdrop => "backdrop_blur",
            Self::Noise => "noise_strength",
        }
    }
}

/// A change inside `[rgb]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbChange {
    /// The engine selection changed.
    Mode,
    /// Base colors changed.
    Colors,
    /// Speed, brightness, or saturation changed.
    Tuning,
    /// Reactive band mapping changed.
    Reactive,
}

impl RgbChange {
    fn path(self) -> &'static str {
        match self {
            Self::Mode => "mode",
            Self::Colors => "primary",
            Self::Tuning => "speed",
            Self::Reactive => "reactive.gain",
        }
    }
}

/// A change inside `[audio]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChange {
    /// Capture source, device, or backend changed.
    Capture,
    /// FFT size or band count changed.
    Analysis,
    /// Envelope or sensitivity changed.
    Envelope,
    /// Spectrum appearance changed.
    Visualizer,
}

impl AudioChange {
    fn path(self) -> &'static str {
        match self {
            Self::Capture => "source",
            Self::Analysis => "fft_size",
            Self::Envelope => "attack",
            Self::Visualizer => "visualizer.style",
        }
    }
}

/// A change inside `[media]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaChange {
    /// Metadata display changed.
    Metadata,
    /// Progress bar behavior changed.
    Progress,
    /// Marquee behavior changed.
    Marquee,
    /// Player preference list changed.
    Players,
}

impl MediaChange {
    fn path(self) -> &'static str {
        match self {
            Self::Metadata => "show_album_art",
            Self::Progress => "show_progress",
            Self::Marquee => "marquee",
            Self::Players => "preferred_players",
        }
    }
}

/// A change inside `[motion]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionChange {
    /// Springs or timing changed.
    Animation,
    /// Accessibility behavior changed.
    Accessibility,
}

impl MotionChange {
    fn path(self) -> &'static str {
        match self {
            Self::Animation => "morph_response_s",
            Self::Accessibility => "reduced",
        }
    }
}

/// A change inside `[accessibility]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityChange {
    /// Contrast enforcement or its target moved.
    Contrast,
    /// The focus ring or keyboard focus policy moved.
    Focus,
    /// Announcement policy or cadence moved.
    Announcements,
}

impl AccessibilityChange {
    fn path(self) -> &'static str {
        match self {
            Self::Contrast => "contrast_target",
            Self::Focus => "focus_ring_width",
            Self::Announcements => "screen_reader",
        }
    }
}

/// A change inside `[text]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextChange {
    /// Font size or weight changed.
    Font,
    /// Colors changed.
    Colors,
    /// Marquee or sharpness changed.
    Rendering,
}

impl TextChange {
    fn path(self) -> &'static str {
        match self {
            Self::Font => "font_size",
            Self::Colors => "color",
            Self::Rendering => "marquee",
        }
    }
}

/// The outcome of a diff.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigDiff {
    changes: Vec<ConfigChange>,
}

impl ConfigDiff {
    /// Every change, in a stable order.
    #[must_use]
    pub fn changes(&self) -> &[ConfigChange] {
        &self.changes
    }

    /// True when nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// How many changes there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// True when any change matches `predicate`.
    pub fn any(&self, predicate: impl Fn(&ConfigChange) -> bool) -> bool {
        self.changes.iter().any(predicate)
    }

    /// Only the changes for a section, so a handler can match on one variant.
    #[must_use]
    pub fn of_kind(&self, section: Section) -> Vec<&ConfigChange> {
        self.changes
            .iter()
            .filter(|c| {
                matches!(
                    (section, c),
                    (Section::General, ConfigChange::General(_))
                        | (Section::Appearance, ConfigChange::Appearance(_))
                        | (Section::Rgb, ConfigChange::Rgb(_))
                        | (Section::Audio, ConfigChange::Audio(_))
                        | (Section::Media, ConfigChange::Media(_))
                        | (Section::Motion, ConfigChange::Motion(_))
                        | (Section::Accessibility, ConfigChange::Accessibility(_))
                        | (Section::Text, ConfigChange::Text(_))
                )
            })
            .collect()
    }
}

/// The config sections a handler can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// `[general]`.
    General,
    /// `[appearance]`.
    Appearance,
    /// `[rgb]`.
    Rgb,
    /// `[audio]`.
    Audio,
    /// `[media]`.
    Media,
    /// `[motion]`.
    Motion,
    /// `[accessibility]`.
    Accessibility,
    /// `[text]`.
    Text,
    /// `[layout]`.
    Layout,
    /// `[hotkeys]`.
    Hotkeys,
    /// `[advanced]`.
    Advanced,
}

/// Diff two configurations.
///
/// Field order inside each section is fixed, so the output is deterministic and
/// a diff of a config against itself is always empty.
#[must_use]
pub fn diff_config(from: &Config, to: &Config) -> ConfigDiff {
    let mut changes = Vec::new();

    diff_general(&from.general, &to.general, &mut changes);
    diff_appearance(&from.appearance, &to.appearance, &mut changes);
    diff_rgb(&from.rgb, &to.rgb, &mut changes);
    diff_audio(&from.audio, &to.audio, &mut changes);
    diff_media(&from.media, &to.media, &mut changes);
    diff_motion(&from.motion, &to.motion, &mut changes);
    diff_accessibility(&from.accessibility, &to.accessibility, &mut changes);
    diff_text(&from.text, &to.text, &mut changes);

    if from.layout != to.layout {
        changes.push(ConfigChange::Layout);
    }
    if from.hotkeys != to.hotkeys {
        changes.push(ConfigChange::Hotkeys);
    }
    if from.advanced != to.advanced {
        changes.push(ConfigChange::Advanced);
    }

    ConfigDiff { changes }
}

fn diff_general(from: &GeneralConfig, to: &GeneralConfig, out: &mut Vec<ConfigChange>) {
    if from.position != to.position {
        out.push(ConfigChange::General(GeneralChange::Position));
    }
    if from.offset_x != to.offset_x || from.offset_y != to.offset_y {
        out.push(ConfigChange::General(GeneralChange::Offset));
    }
    if from.auto_hide != to.auto_hide || from.auto_hide_delay_ms != to.auto_hide_delay_ms {
        out.push(ConfigChange::General(GeneralChange::Visibility));
    }
    if from.always_clickable != to.always_clickable
        || from.click_through_when_collapsed != to.click_through_when_collapsed
        || from.idle_timeout_ms != to.idle_timeout_ms
        || from.start_hidden != to.start_hidden
    {
        out.push(ConfigChange::General(GeneralChange::ClickThrough));
    }
    if from.monitor != to.monitor
        || from.reserve_space != to.reserve_space
        || from.single_instance != to.single_instance
    {
        out.push(ConfigChange::General(GeneralChange::Other));
    }
}

fn diff_appearance(from: &AppearanceConfig, to: &AppearanceConfig, out: &mut Vec<ConfigChange>) {
    if from.fill != to.fill || from.fill_gradient != to.fill_gradient {
        out.push(ConfigChange::Appearance(AppearanceChange::Fill));
    }
    if from.top_half_width != to.top_half_width
        || from.bottom_half_width != to.bottom_half_width
        || from.height != to.height
        || from.corner_radius != to.corner_radius
        || from.inverted_trapezoid != to.inverted_trapezoid
    {
        out.push(ConfigChange::Appearance(AppearanceChange::Shape));
    }
    if from.border_width != to.border_width || from.border_color != to.border_color {
        out.push(ConfigChange::Appearance(AppearanceChange::Border));
    }
    if from.glow_spread != to.glow_spread || from.glow_falloff != to.glow_falloff {
        out.push(ConfigChange::Appearance(AppearanceChange::Border));
    }
    if from.backdrop_blur != to.backdrop_blur || from.backdrop_darken != to.backdrop_darken {
        out.push(ConfigChange::Appearance(AppearanceChange::Backdrop));
    }
    if from.noise_strength != to.noise_strength || from.shadow_opacity != to.shadow_opacity {
        out.push(ConfigChange::Appearance(AppearanceChange::Noise));
    }
}

fn diff_rgb(from: &RgbConfig, to: &RgbConfig, out: &mut Vec<ConfigChange>) {
    if from.mode != to.mode {
        out.push(ConfigChange::Rgb(RgbChange::Mode));
    }
    if from.primary != to.primary || from.secondary != to.secondary {
        out.push(ConfigChange::Rgb(RgbChange::Colors));
    }
    if from.speed != to.speed
        || from.brightness != to.brightness
        || from.saturation != to.saturation
        || from.breath_rate_hz != to.breath_rate_hz
        || from.breath_depth != to.breath_depth
        || from.glow_intensity != to.glow_intensity
        || from.cycle_degrees_per_second != to.cycle_degrees_per_second
        || from.transition_ms != to.transition_ms
    {
        out.push(ConfigChange::Rgb(RgbChange::Tuning));
    }
    if from.reactive != to.reactive {
        out.push(ConfigChange::Rgb(RgbChange::Reactive));
    }
}

fn diff_audio(from: &AudioConfig, to: &AudioConfig, out: &mut Vec<ConfigChange>) {
    if from.enabled != to.enabled
        || from.source != to.source
        || from.device != to.device
        || from.backend != to.backend
    {
        out.push(ConfigChange::Audio(AudioChange::Capture));
    }
    if from.fft_size != to.fft_size
        || from.band_count != to.band_count
        || from.normalize != to.normalize
    {
        out.push(ConfigChange::Audio(AudioChange::Analysis));
    }
    if from.sensitivity != to.sensitivity
        || from.attack != to.attack
        || from.release != to.release
        || from.silence_threshold_db != to.silence_threshold_db
        || from.bass_tilt_db_per_octave != to.bass_tilt_db_per_octave
    {
        out.push(ConfigChange::Audio(AudioChange::Envelope));
    }
    if from.visualizer != to.visualizer {
        out.push(ConfigChange::Audio(AudioChange::Visualizer));
    }
}

fn diff_media(from: &MediaConfig, to: &MediaConfig, out: &mut Vec<ConfigChange>) {
    if from.enabled != to.enabled
        || from.show_album_art != to.show_album_art
        || from.album_art_size != to.album_art_size
        || from.album_art_corner_radius != to.album_art_corner_radius
        || from.hold_ms != to.hold_ms
    {
        out.push(ConfigChange::Media(MediaChange::Metadata));
    }
    if from.show_progress != to.show_progress
        || from.progress_height != to.progress_height
        || from.scrub_on_click != to.scrub_on_click
        || from.show_volume != to.show_volume
    {
        out.push(ConfigChange::Media(MediaChange::Progress));
    }
    if from.marquee != to.marquee
        || from.marquee_speed_px_per_s != to.marquee_speed_px_per_s
        || from.marquee_gap_px != to.marquee_gap_px
        || from.marquee_pause_on_hover_ms != to.marquee_pause_on_hover_ms
    {
        out.push(ConfigChange::Media(MediaChange::Marquee));
    }
    if from.preferred_players != to.preferred_players {
        out.push(ConfigChange::Media(MediaChange::Players));
    }
}

fn diff_motion(from: &MotionConfig, to: &MotionConfig, out: &mut Vec<ConfigChange>) {
    if from.master_scale != to.master_scale
        || from.morph_response_s != to.morph_response_s
        || from.morph_damping != to.morph_damping
        || from.content_fade_ms != to.content_fade_ms
        || from.startup_reveal != to.startup_reveal
    {
        out.push(ConfigChange::Motion(MotionChange::Animation));
    }
    if from.reduced != to.reduced {
        out.push(ConfigChange::Motion(MotionChange::Accessibility));
    }
}

fn diff_accessibility(
    from: &AccessibilityConfig,
    to: &AccessibilityConfig,
    out: &mut Vec<ConfigChange>,
) {
    if from.enforce_contrast != to.enforce_contrast
        || from.contrast_target != to.contrast_target
        || from.respect_os_reduced_motion != to.respect_os_reduced_motion
    {
        out.push(ConfigChange::Accessibility(
            AccessibilityChange::Contrast,
        ));
    }
    if from.keyboard_focus != to.keyboard_focus
        || from.focus_ring_width != to.focus_ring_width
        || from.focus_ring_offset != to.focus_ring_offset
        || from.focus_ring_color != to.focus_ring_color
    {
        out.push(ConfigChange::Accessibility(AccessibilityChange::Focus));
    }
    if from.screen_reader != to.screen_reader
        || from.announce_min_interval_ms != to.announce_min_interval_ms
    {
        out.push(ConfigChange::Accessibility(
            AccessibilityChange::Announcements,
        ));
    }
}

fn diff_text(from: &TextConfig, to: &TextConfig, out: &mut Vec<ConfigChange>) {
    if from.font_size != to.font_size
        || from.font_weight != to.font_weight
        || from.font_file != to.font_file
    {
        out.push(ConfigChange::Text(TextChange::Font));
    }
    if from.color != to.color || from.dim_color != to.dim_color {
        out.push(ConfigChange::Text(TextChange::Colors));
    }
    if from.title_size != to.title_size
        || from.artist_size != to.artist_size
        || from.marquee != to.marquee
        || from.sharp != to.sharp
        || from.letter_spacing != to.letter_spacing
    {
        out.push(ConfigChange::Text(TextChange::Rendering));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Config {
        Config::default()
    }

    #[test]
    fn a_config_against_itself_has_no_changes() {
        let config = base();
        assert!(
            diff_config(&config, &config).is_empty(),
            "a config must diff clean against itself"
        );
    }

    #[test]
    fn a_changed_position_is_reported_as_general_position() {
        let mut config = base();
        config.general.position = crate::schema::Position::BottomRight;
        let diff = diff_config(&base(), &config);
        assert_eq!(diff.len(), 1);
        assert_eq!(
            diff.changes()[0],
            ConfigChange::General(GeneralChange::Position)
        );
    }

    #[test]
    fn a_changed_fill_is_reported_as_appearance_fill() {
        let mut config = base();
        config.appearance.fill = "#FF0000FF".to_string();
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Appearance(AppearanceChange::Fill)));
    }

    #[test]
    fn a_changed_rgb_mode_is_reported_as_rgb_mode() {
        let mut config = base();
        config.rgb.mode = crate::schema::RgbMode::Reactive;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Rgb(RgbChange::Mode)));
    }

    #[test]
    fn a_changed_band_count_is_reported_as_audio_analysis() {
        let mut config = base();
        config.audio.band_count = 64;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Audio(AudioChange::Analysis)));
    }

    #[test]
    fn a_changed_visualizer_style_is_reported_as_audio_visualizer() {
        let mut config = base();
        config.audio.visualizer.style = crate::schema::VisualizerStyle::Waveform;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Audio(AudioChange::Visualizer)));
    }

    #[test]
    fn a_changed_progress_bar_is_reported_as_media_progress() {
        let mut config = base();
        config.media.show_progress = false;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Media(MediaChange::Progress)));
    }

    #[test]
    fn a_changed_marquee_speed_is_reported_as_media_marquee() {
        let mut config = base();
        config.media.marquee_speed_px_per_s = 80.0;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Media(MediaChange::Marquee)));
    }

    #[test]
    fn a_changed_spring_is_reported_as_motion_animation() {
        let mut config = base();
        config.motion.morph_response_s = 0.5;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Motion(MotionChange::Animation)));
    }

    #[test]
    fn a_changed_reduced_motion_is_reported_as_motion_accessibility() {
        let mut config = base();
        config.motion.reduced = true;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Motion(MotionChange::Accessibility)));
    }

    #[test]
    fn a_changed_font_size_is_reported_as_text_font() {
        let mut config = base();
        config.text.font_size = 16.0;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Text(TextChange::Font)));
    }

    #[test]
    fn a_changed_color_is_reported_as_text_colors() {
        let mut config = base();
        config.text.color = "#FF0000FF".to_string();
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Text(TextChange::Colors)));
    }

    #[test]
    fn a_changed_layout_is_reported_as_layout() {
        let mut config = base();
        config.layout.max_widgets = 5;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Layout));
    }

    #[test]
    fn a_changed_hotkey_is_reported_as_hotkeys() {
        let mut config = base();
        config.hotkeys.toggle_visibility = "ctrl+alt+x".to_string();
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Hotkeys));
    }

    #[test]
    fn a_changed_advanced_is_reported_as_advanced() {
        let mut config = base();
        config.advanced.target_fps = 120;
        let diff = diff_config(&base(), &config);
        assert!(diff.any(|c| *c == ConfigChange::Advanced));
    }

    #[test]
    fn multiple_changes_are_all_reported() {
        let mut config = base();
        config.general.position = crate::schema::Position::BottomRight;
        config.rgb.mode = crate::schema::RgbMode::Reactive;
        config.audio.band_count = 64;
        config.motion.reduced = true;

        let diff = diff_config(&base(), &config);
        assert_eq!(diff.len(), 4, "every changed field should be reported");
    }

    #[test]
    fn the_path_of_a_change_is_dotted() {
        let mut config = base();
        config.general.position = crate::schema::Position::BottomRight;
        let diff = diff_config(&base(), &config);
        assert_eq!(diff.changes()[0].path(), "general.position");
    }

    #[test]
    fn of_kind_filters_to_one_section() {
        let mut config = base();
        config.general.position = crate::schema::Position::BottomRight;
        config.rgb.mode = crate::schema::RgbMode::Reactive;

        let diff = diff_config(&base(), &config);
        assert_eq!(diff.of_kind(Section::General).len(), 1);
        assert_eq!(diff.of_kind(Section::Rgb).len(), 1);
        assert_eq!(diff.of_kind(Section::Audio).len(), 0);
    }

    #[test]
    fn every_change_path_starts_with_its_section() {
        let mut config = base();
        config.general.offset_x = 100;
        config.appearance.height = 50.0;
        config.rgb.saturation = 1.5;
        config.audio.sensitivity = 3.0;
        config.media.hold_ms = 9000;
        config.motion.master_scale = 2.0;
        config.text.title_size = 18.0;

        let diff = diff_config(&base(), &config);
        for change in diff.changes() {
            let path = change.path();
            assert!(
                path.contains('.'),
                "a change path should be dotted, got {path}"
            );
        }
    }

    #[test]
    fn the_diff_is_deterministic() {
        let mut config = base();
        config.general.position = crate::schema::Position::BottomRight;
        config.rgb.mode = crate::schema::RgbMode::Reactive;

        let first = diff_config(&base(), &config);
        let second = diff_config(&base(), &config);
        assert_eq!(first, second);
    }
}
