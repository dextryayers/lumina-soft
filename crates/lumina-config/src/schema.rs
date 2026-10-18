//! The configuration tree.
//!
//! Every section is a plain struct with a `Default` impl. The `Default` impls in
//! [`crate::defaults`] are the single source of truth for the shipped values, so
//! the file written on first run and the compiled-in fallback can never drift.

use serde::{Deserialize, Serialize};

use lumina_core::color::Srgb;

/// Top level configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Schema version, used to migrate older files.
    pub version: u32,
    /// Window placement, position, and interaction behaviour.
    pub general: GeneralConfig,
    /// Shape, color, and material.
    pub appearance: AppearanceConfig,
    /// Color engine selection.
    pub rgb: RgbConfig,
    /// Audio capture and analysis.
    pub audio: AudioConfig,
    /// Media player integration.
    pub media: MediaConfig,
    /// Clock and date.
    pub clock: ClockConfig,
    /// System metrics sampling.
    pub metrics: MetricsConfig,
    /// Battery display.
    pub battery: BatteryConfig,
    /// Network throughput display.
    pub network: NetworkConfig,
    /// Pomodoro timer.
    pub pomodoro: PomodoroConfig,
    /// Weather, the only widget that can make a network request.
    pub weather: WeatherConfig,
    /// Widget arrangement.
    pub layout: LayoutConfig,
    /// Global hotkeys.
    pub hotkeys: HotkeysConfig,
    /// User defined actions.
    pub shortcuts: ShortcutsConfig,
    /// System tray.
    pub tray: TrayConfig,
    /// Animation tuning and accessibility.
    pub motion: MotionConfig,
    /// Typography.
    pub text: TextConfig,
    /// Expert settings that most users never touch.
    pub advanced: AdvancedConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: crate::SCHEMA_VERSION,
            general: GeneralConfig::default(),
            appearance: AppearanceConfig::default(),
            rgb: RgbConfig::default(),
            audio: AudioConfig::default(),
            media: MediaConfig::default(),
            clock: ClockConfig::default(),
            metrics: MetricsConfig::default(),
            battery: BatteryConfig::default(),
            network: NetworkConfig::default(),
            pomodoro: PomodoroConfig::default(),
            weather: WeatherConfig::default(),
            layout: LayoutConfig::default(),
            hotkeys: HotkeysConfig::default(),
            shortcuts: ShortcutsConfig::default(),
            tray: TrayConfig::default(),
            motion: MotionConfig::default(),
            text: TextConfig::default(),
            advanced: AdvancedConfig::default(),
        }
    }
}

impl Config {
    /// Look up a value by dotted path, used by the IPC `set_config` command and
    /// by tests that assert a key is reachable.
    ///
    /// Returns `None` for an unknown path. Paths use the same spelling as the
    /// file, for example `rgb.mode` or `audio.visualizer.band_count`.
    #[must_use]
    pub fn get_path(&self, path: &str) -> Option<String> {
        let mut node: toml::Value = toml::Value::try_from(self).ok()?;
        let mut current = &mut node;
        for segment in path.split('.') {
            current = current.get_mut(segment)?;
        }
        Some(render_toml_value(current))
    }

    /// Render this configuration back to TOML with comments.
    ///
    /// Used when writing the first-run file and when a user asks to save the
    /// current effective configuration from the tray menu.
    #[must_use]
    pub fn to_toml_string(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_else(|e| {
            // Serializing our own types cannot fail in practice, but returning
            // a comment plus the error beats panicking in a background process.
            format!("# Failed to serialize configuration: {e}\n")
        })
    }

    /// Parse TOML into a configuration, ignoring any unknown sections.
    ///
    /// Unknown *sections* are tolerated here because a file written by a newer
    /// version should still load in an older binary. Unknown *keys* inside a
    /// known section are reported by [`crate::validate`] with a suggestion.
    pub fn from_toml_str(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

/// Render a TOML value as the string an IPC client expects to read back.
///
/// Arrays and inline tables come back as their TOML source form, which is the
/// same text a user would have written.
fn render_toml_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(render_toml_value).collect();
            format!("[{}]", rendered.join(", "))
        }
        toml::Value::Table(table) => {
            let rendered: Vec<String> = table
                .iter()
                .map(|(k, v)| format!("{k} = {}", render_toml_value(v)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

/// Where the island sits relative to its monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum Position {
    /// Centered on the top edge.
    #[default]
    TopCenter,
    /// Left aligned to the top edge.
    TopLeft,
    /// Right aligned to the top edge.
    TopRight,
    /// Centered on the bottom edge.
    BottomCenter,
    /// Left aligned to the bottom edge.
    BottomLeft,
    /// Right aligned to the bottom edge.
    BottomRight,
}

impl Position {
    /// All variants, used to render the config file comments and to test
    /// exhaustively.
    pub const ALL: [Self; 6] = [
        Self::TopCenter,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomCenter,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    /// Stable identifier used in the file, the tray, and IPC.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::TopCenter => "top-center",
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomCenter => "bottom-center",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
        }
    }

    /// Parse an identifier, case insensitive and tolerant of separators.
    #[must_use]
    pub fn from_id(input: &str) -> Option<Self> {
        let normalized: String = input
            .trim()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        Self::ALL.into_iter().find(|p| {
            p.id()
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
                == normalized
        })
    }

    /// True when this position anchors to the top edge.
    #[must_use]
    pub const fn is_top(self) -> bool {
        matches!(self, Self::TopCenter | Self::TopLeft | Self::TopRight)
    }

    /// Horizontal alignment as a fraction of the monitor width, where 0.5 is
    /// centered. Computed rather than matched so adding a position cannot get
    /// this wrong.
    #[must_use]
    pub const fn horizontal_fraction(self) -> f32 {
        match self {
            Self::TopLeft | Self::BottomLeft => 0.0,
            Self::TopRight | Self::BottomRight => 1.0,
            Self::TopCenter | Self::BottomCenter => 0.5,
        }
    }
}

/// Which monitor the island lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MonitorSelector {
    /// The monitor the operating system calls primary.
    Primary,
    /// Whichever monitor currently contains the pointer.
    Cursor,
    /// A zero based index into the monitor list.
    Index(u32),
}

/// Window placement, position, and interaction behaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeneralConfig {
    /// Anchor edge or corner.
    pub position: Position,
    /// Horizontal offset from the anchor, in logical pixels. Positive is right.
    pub offset_x: i32,
    /// Vertical offset from the anchor, in logical pixels. Positive is down.
    pub offset_y: i32,
    /// Which monitor to live on.
    pub monitor: MonitorSelector,
    /// Fade out when idle.
    pub auto_hide: bool,
    /// How long before auto hide kicks in, in milliseconds.
    pub auto_hide_delay_ms: u32,
    /// Keep the island clickable even while collapsed.
    pub always_clickable: bool,
    /// Collapse after this long with nothing happening, in milliseconds.
    pub idle_timeout_ms: u32,
    /// Let clicks pass through to whatever is underneath while collapsed.
    pub click_through_when_collapsed: bool,
    /// Start with the island hidden.
    pub start_hidden: bool,
    /// Refuse to start a second copy.
    pub single_instance: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            position: Position::default(),
            offset_x: 0,
            offset_y: 12,
            monitor: MonitorSelector::Primary,
            auto_hide: false,
            auto_hide_delay_ms: 3000,
            always_clickable: false,
            idle_timeout_ms: 5000,
            click_through_when_collapsed: true,
            start_hidden: false,
            single_instance: true,
        }
    }
}

/// The color engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum RgbMode {
    /// A single fixed color.
    Static,
    /// Sine wave modulation of brightness.
    #[default]
    Breathing,
    /// Hue rotation through the full wheel.
    Cycling,
    /// Driven by live audio energy.
    Reactive,
    /// Derived from the current album art.
    Album,
}

impl RgbMode {
    /// All variants.
    pub const ALL: [Self; 5] = [
        Self::Static,
        Self::Breathing,
        Self::Cycling,
        Self::Reactive,
        Self::Album,
    ];

    /// Stable identifier.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Breathing => "breathing",
            Self::Cycling => "cycling",
            Self::Reactive => "reactive",
            Self::Album => "album",
        }
    }

    /// Parse an identifier, case insensitive.
    #[must_use]
    pub fn from_id(input: &str) -> Option<Self> {
        let normalized: String = input.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|m| m.id() == normalized)
    }
}

/// Shape, color, and material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppearanceConfig {
    /// Background fill as a hex string, alpha included.
    pub fill: String,
    /// Gradient strength between the primary and secondary color, 0 to 1.
    pub fill_gradient: f32,
    /// Corner radius in logical pixels.
    pub corner_radius: f32,
    /// Half width of the wider top edge.
    pub top_half_width: f32,
    /// Half width of the narrower bottom edge.
    pub bottom_half_width: f32,
    /// Full height in logical pixels.
    pub height: f32,
    /// When false the shape is a regular trapezoid rather than an inverted one.
    pub inverted_trapezoid: bool,
    /// Border stroke width, zero disables it.
    pub border_width: f32,
    /// Border color as a hex string.
    pub border_color: String,
    /// Glow extent in logical pixels.
    pub glow_spread: f32,
    /// Glow decay distance. Larger is tighter.
    pub glow_falloff: f32,
    /// Drop shadow strength, zero disables it.
    pub shadow_opacity: f32,
    /// Whether to blur whatever is behind the island.
    pub backdrop_blur: BackdropMode,
    /// How much to darken the blurred backdrop.
    pub backdrop_darken: f32,
    /// Dither strength, which removes gradient banding on 8 bit displays.
    pub noise_strength: f32,
}

/// Backdrop blur policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum BackdropMode {
    /// Use the platform's blur if it has one, otherwise fall back to ours.
    #[default]
    Auto,
    /// Always use our own blur.
    On,
    /// Never blur.
    Off,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            fill: "#12121ACC".to_string(),
            fill_gradient: 0.0,
            corner_radius: 18.0,
            top_half_width: 62.0,
            bottom_half_width: 54.0,
            height: 36.0,
            inverted_trapezoid: true,
            border_width: 1.0,
            border_color: "#FFFFFF24".to_string(),
            glow_spread: 8.0,
            glow_falloff: 8.0,
            shadow_opacity: 0.28,
            backdrop_blur: BackdropMode::Auto,
            backdrop_darken: 0.15,
            noise_strength: 0.012,
        }
    }
}

impl AppearanceConfig {
    /// Build the clamped geometry this configuration describes.
    ///
    /// Clamping lives in [`lumina_core::Geometry`] so the renderer and the
    /// config path can never disagree about the limits.
    #[must_use]
    pub fn geometry(&self) -> lumina_core::Geometry {
        use lumina_core::Geometry;
        Geometry::new(
            self.top_half_width,
            self.bottom_half_width,
            self.height,
            self.corner_radius,
        )
        .clamped()
    }
}

/// Color engine selection and tuning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RgbConfig {
    /// Which engine produces the palette.
    pub mode: RgbMode,
    /// Base color as a hex string.
    pub primary: String,
    /// Secondary color, used by gradients and the cycling engine.
    pub secondary: String,
    /// Global speed multiplier for whichever engine is active.
    pub speed: f32,
    /// Lightness multiplier applied to the resolved palette.
    pub brightness: f32,
    /// Chroma multiplier applied to the resolved palette.
    pub saturation: f32,
    /// Breathing engine rate in hertz.
    pub breath_rate_hz: f32,
    /// Breathing engine depth, as a fraction of full lightness.
    pub breath_depth: f32,
    /// Glow strength multiplier.
    pub glow_intensity: f32,
    /// Cycling engine speed in degrees per second.
    pub cycle_degrees_per_second: f32,
    /// Crossfade duration when switching engines or profiles, in milliseconds.
    pub transition_ms: u32,
    /// Reactive engine tuning.
    pub reactive: ReactiveConfig,
}

impl Default for RgbConfig {
    fn default() -> Self {
        Self {
            mode: RgbMode::default(),
            primary: "#7C3AED".to_string(),
            secondary: "#06B6D4".to_string(),
            speed: 1.0,
            brightness: 1.0,
            saturation: 0.9,
            breath_rate_hz: 0.25,
            breath_depth: 0.18,
            glow_intensity: 0.6,
            cycle_degrees_per_second: 40.0,
            transition_ms: 300,
            reactive: ReactiveConfig::default(),
        }
    }
}

/// Reactive engine tuning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReactiveConfig {
    /// Overall sensitivity multiplier.
    pub gain: f32,
    /// How much of the mapping is always on, so the island is never dead.
    pub floor: f32,
    /// Upper edge of the bass band, in hertz.
    pub bass_hz: u32,
    /// Upper edge of the mid band, in hertz.
    pub mid_hz: u32,
    /// How far treble energy rotates the hue, in degrees.
    pub hue_spread_degrees: f32,
    /// How much bass energy drives the glow.
    pub glow_from_bass: f32,
}

impl Default for ReactiveConfig {
    fn default() -> Self {
        Self {
            gain: 1.0,
            floor: 0.12,
            bass_hz: 250,
            mid_hz: 2000,
            hue_spread_degrees: 90.0,
            glow_from_bass: 0.55,
        }
    }
}

/// Audio capture and analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    /// Master switch for the audio engine.
    pub enabled: bool,
    /// Where the audio comes from.
    pub source: AudioSource,
    /// Device name substring, or `default`.
    pub device: String,
    /// Capture backend, or `auto` to probe.
    pub backend: AudioBackend,
    /// FFT window size.
    pub fft_size: FftSize,
    /// Number of spectrum bands.
    pub band_count: u32,
    /// Overall sensitivity multiplier.
    pub sensitivity: f32,
    /// Envelope attack coefficient, how fast bars rise.
    pub attack: f32,
    /// Envelope release coefficient, how fast bars fall.
    pub release: f32,
    /// Level below which the spectrum fades out, in dBFS.
    pub silence_threshold_db: f32,
    /// Low frequency tilt, in dB per octave.
    pub bass_tilt_db_per_octave: f32,
    /// Track a rolling maximum so quiet tracks still show a spectrum.
    pub normalize: bool,
    /// Spectrum appearance.
    pub visualizer: SpectrumConfig,
}

/// Where audio is captured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum AudioSource {
    /// Whatever the system is playing.
    #[default]
    System,
    /// The default input device. Documented as capturing room audio.
    Microphone,
    /// No capture at all.
    None,
}

/// Capture backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum AudioBackend {
    /// Probe in preference order.
    #[default]
    Auto,
    /// PulseAudio monitor source.
    Pulse,
    /// PipeWire loopback node.
    Pipewire,
    /// WASAPI loopback, Windows only.
    Wasapi,
}

/// FFT window size, named because the raw numbers are meaningless to a user.
///
/// The variant names carry a digit run that `rename_all = "kebab-case"` cannot
/// split, so each variant is renamed explicitly rather than by attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FftSize {
    /// 1024 samples.
    #[serde(rename = "size-1024")]
    Size1024,
    /// 2048 samples.
    #[serde(rename = "size-2048")]
    Size2048,
    /// 4096 samples. The default, which resolves the low end at 48 kHz.
    #[serde(rename = "size-4096")]
    #[default]
    Size4096,
    /// 8192 samples.
    #[serde(rename = "size-8192")]
    Size8192,
}

impl FftSize {
    /// Raw sample count.
    #[must_use]
    pub const fn samples(self) -> usize {
        match self {
            Self::Size1024 => 1024,
            Self::Size2048 => 2048,
            Self::Size4096 => 4096,
            Self::Size8192 => 8192,
        }
    }

    /// All variants.
    pub const ALL: [Self; 4] = [
        Self::Size1024,
        Self::Size2048,
        Self::Size4096,
        Self::Size8192,
    ];

    /// The identifier as it appears in the file.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Size1024 => "size-1024",
            Self::Size2048 => "size-2048",
            Self::Size4096 => "size-4096",
            Self::Size8192 => "size-8192",
        }
    }
}

/// Spectrum appearance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpectrumConfig {
    /// How bands are drawn.
    pub style: VisualizerStyle,
    /// Gap between bars.
    pub bar_spacing: u32,
    /// Bar corner radius.
    pub bar_radius: u32,
    /// How long a peak marker lingers, in milliseconds.
    pub peak_hold_ms: u32,
    /// Hue rotation across the band range, in degrees.
    pub hue_spread_degrees: f32,
    /// Brighten bars that reach the top.
    pub hot_tip: bool,
    /// Draw a one pixel baseline so idle bands are still visible.
    pub baseline: bool,
}

/// Spectrum drawing style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum VisualizerStyle {
    /// Vertical bars from the bottom.
    #[default]
    Bars,
    /// Bars mirrored about the horizontal centre line.
    Mirror,
    /// Time domain waveform.
    Waveform,
    /// Small dots instead of bars.
    Dots,
}

impl VisualizerStyle {
    /// All variants.
    pub const ALL: [Self; 4] = [Self::Bars, Self::Mirror, Self::Waveform, Self::Dots];

    /// Stable identifier.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Bars => "bars",
            Self::Mirror => "mirror",
            Self::Waveform => "waveform",
            Self::Dots => "dots",
        }
    }
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        Self {
            style: VisualizerStyle::Bars,
            bar_spacing: 2,
            bar_radius: 3,
            peak_hold_ms: 400,
            hue_spread_degrees: 120.0,
            hot_tip: true,
            baseline: true,
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            source: AudioSource::System,
            device: "default".to_string(),
            backend: AudioBackend::Auto,
            fft_size: FftSize::default(),
            band_count: 32,
            sensitivity: 1.8,
            attack: 0.35,
            release: 0.08,
            silence_threshold_db: -60.0,
            bass_tilt_db_per_octave: 3.0,
            normalize: true,
            visualizer: SpectrumConfig::default(),
        }
    }
}

/// Media player integration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaConfig {
    /// Master switch.
    pub enabled: bool,
    /// How long the media state stays up after playback stops, in milliseconds.
    pub hold_ms: u32,
    /// Show cover art.
    pub show_album_art: bool,
    /// Cover art edge length.
    pub album_art_size: u32,
    /// Cover art corner radius.
    pub album_art_corner_radius: u32,
    /// Show the progress bar.
    pub show_progress: bool,
    /// Progress bar thickness.
    pub progress_height: u32,
    /// Allow seeking by clicking the progress bar.
    pub scrub_on_click: bool,
    /// Show a volume control.
    pub show_volume: bool,
    /// Scroll long titles.
    pub marquee: bool,
    /// Marquee speed in logical pixels per second.
    pub marquee_speed_px_per_s: f32,
    /// Gap between marquee repetitions.
    pub marquee_gap_px: u32,
    /// How long the marquee pauses when the pointer arrives, in milliseconds.
    pub marquee_pause_on_hover_ms: u32,
    /// Player identifiers to prefer, most wanted first.
    pub preferred_players: Vec<String>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hold_ms: 6000,
            show_album_art: true,
            album_art_size: 52,
            album_art_corner_radius: 8,
            show_progress: true,
            progress_height: 3,
            scrub_on_click: true,
            show_volume: false,
            marquee: true,
            marquee_speed_px_per_s: 40.0,
            marquee_gap_px: 48,
            marquee_pause_on_hover_ms: 1800,
            preferred_players: Vec::new(),
        }
    }
}

/// Clock and date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClockConfig {
    /// Master switch.
    pub enabled: bool,
    /// 12 or 24 hour.
    pub format: ClockFormat,
    /// Show seconds.
    pub show_seconds: bool,
    /// Show the date.
    pub show_date: bool,
    /// Date format as a strftime string.
    pub date_format: String,
    /// Show the weekday name.
    pub show_weekday: bool,
    /// A second timezone as an IANA name, empty for none.
    pub second_timezone: String,
    /// Pad the hour with a leading zero.
    pub hour_leading_zero: bool,
}

/// Clock hour format.
///
/// Renamed explicitly because `rename_all = "kebab-case"` would turn `H12`
/// into `h12`, and users write `12h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClockFormat {
    /// 12 hour with an am or pm suffix.
    #[serde(rename = "12h")]
    H12,
    /// 24 hour.
    #[serde(rename = "24h")]
    #[default]
    H24,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            format: ClockFormat::H24,
            show_seconds: false,
            show_date: true,
            date_format: "%d %b".to_string(),
            show_weekday: false,
            second_timezone: String::new(),
            hour_leading_zero: true,
        }
    }
}

/// System metrics sampling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetricsConfig {
    /// Master switch.
    pub enabled: bool,
    /// Sampling interval in milliseconds.
    pub interval_ms: u32,
    /// Show processor load.
    pub show_cpu: bool,
    /// Show memory usage.
    pub show_ram: bool,
    /// Show graphics load, where the driver exposes it.
    pub show_gpu: bool,
    /// Show network throughput.
    pub show_network: bool,
    /// History length of the sparkline.
    pub sparkline_bars: u32,
    /// Raise the island above this load, zero disables.
    pub alert_on_threshold: f32,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_ms: 1000,
            show_cpu: true,
            show_ram: true,
            show_gpu: false,
            show_network: false,
            sparkline_bars: 24,
            alert_on_threshold: 0.0,
        }
    }
}

/// Battery display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatteryConfig {
    /// Master switch.
    pub enabled: bool,
    /// Sampling interval in milliseconds.
    pub interval_ms: u32,
    /// Show the percentage.
    pub show_percentage: bool,
    /// Show estimated time remaining.
    pub show_time_remaining: bool,
    /// Show a bolt icon while charging.
    pub show_charging_bolt: bool,
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_ms: 5000,
            show_percentage: true,
            show_time_remaining: true,
            show_charging_bolt: true,
        }
    }
}

/// Network throughput display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Master switch.
    pub enabled: bool,
    /// Sampling interval in milliseconds.
    pub interval_ms: u32,
    /// Show upload rate.
    pub show_upload: bool,
    /// Show download rate.
    pub show_download: bool,
    /// Show the wireless network name, Linux only and requires privileges.
    pub show_ssid: bool,
    /// Unit selection, or `auto` to scale.
    pub unit: NetworkUnit,
}

/// Throughput unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum NetworkUnit {
    /// Scale automatically between units.
    #[default]
    Auto,
    /// Bytes per second.
    B,
    /// Kilobytes per second.
    Kb,
    /// Megabytes per second.
    Mb,
    /// Gigabytes per second.
    Gb,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_ms: 1000,
            show_upload: true,
            show_download: true,
            show_ssid: false,
            unit: NetworkUnit::Auto,
        }
    }
}

/// Pomodoro timer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PomodoroConfig {
    /// Master switch.
    pub enabled: bool,
    /// Focus phase length in minutes.
    pub focus_minutes: u32,
    /// Short break length in minutes.
    pub short_break_minutes: u32,
    /// Long break length in minutes.
    pub long_break_minutes: u32,
    /// Focus phases before a long break.
    pub rounds_before_long: u32,
    /// Announce phase changes through the notify widget.
    pub notify_phases: bool,
}

impl Default for PomodoroConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            rounds_before_long: 4,
            notify_phases: true,
        }
    }
}

/// Weather, the only widget permitted to make a network request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WeatherConfig {
    /// Master switch.
    pub enabled: bool,
    /// Which provider to query.
    pub provider: WeatherProvider,
    /// API key, required by OpenWeatherMap and ignored by Open-Meteo.
    pub api_key: String,
    /// Latitude in decimal degrees.
    pub latitude: f64,
    /// Longitude in decimal degrees.
    pub longitude: f64,
    /// Measurement system.
    pub units: WeatherUnits,
    /// How long a cached reading stays fresh, in minutes.
    pub cache_ttl_minutes: u32,
    /// How often to refresh, in milliseconds.
    pub interval_ms: u32,
}

/// Weather provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum WeatherProvider {
    /// Open-Meteo, which needs no key.
    #[default]
    OpenMeteo,
    /// OpenWeatherMap, which needs a key.
    OpenWeatherMap,
}

/// Measurement system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum WeatherUnits {
    /// Celsius and kilometres per hour.
    #[default]
    Metric,
    /// Fahrenheit and miles per hour.
    Imperial,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: WeatherProvider::OpenMeteo,
            api_key: String::new(),
            latitude: 0.0,
            longitude: 0.0,
            units: WeatherUnits::Metric,
            cache_ttl_minutes: 15,
            interval_ms: 900_000,
        }
    }
}

/// Widget arrangement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutConfig {
    /// Horizontal padding inside the island.
    pub padding_x: f32,
    /// Vertical padding inside the island.
    pub padding_y: f32,
    /// Gap between widgets.
    pub spacing: f32,
    /// Vertical alignment of widgets within the island.
    pub align: LayoutAlign,
    /// Widget identifiers in display order.
    pub order: Vec<String>,
    /// How many widgets may be visible at once.
    pub max_widgets: u32,
}

/// Vertical widget alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum LayoutAlign {
    /// Against the top edge.
    Top,
    /// Centred.
    #[default]
    Center,
    /// Against the bottom edge.
    Bottom,
}

/// Default widget order, shared by the config default and the docs.
pub const DEFAULT_WIDGET_ORDER: [&str; 8] = [
    "clock",
    "media",
    "spectrum",
    "metrics",
    "battery",
    "network",
    "pomodoro",
    "shortcuts",
];

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            padding_x: 12.0,
            padding_y: 6.0,
            spacing: 10.0,
            align: LayoutAlign::Center,
            order: DEFAULT_WIDGET_ORDER
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            max_widgets: 3,
        }
    }
}

/// Global hotkeys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HotkeysConfig {
    /// Show or hide the island.
    pub toggle_visibility: String,
    /// Advance to the next widget state.
    pub cycle_widget: String,
    /// Go back to the previous widget state.
    pub cycle_back: String,
    /// Advance the color engine.
    pub rgb_next_mode: String,
    /// Switch to the next profile.
    pub profile_next: String,
    /// Toggle playback.
    pub media_play_pause: String,
    /// Next track.
    pub media_next: String,
    /// Previous track.
    pub media_previous: String,
    /// Stop playback.
    pub media_stop: String,
    /// Switch the active media player.
    pub cycle_players: String,
    /// Collapse immediately.
    pub collapse_now: String,
    /// Force a configuration reload.
    pub reload_config: String,
    /// Toggle the frame time overlay.
    pub toggle_debug_overlay: String,
}

impl Default for HotkeysConfig {
    fn default() -> Self {
        Self {
            toggle_visibility: "ctrl+alt+l".to_string(),
            cycle_widget: "ctrl+alt+tab".to_string(),
            cycle_back: "ctrl+shift+tab".to_string(),
            rgb_next_mode: "ctrl+alt+r".to_string(),
            profile_next: "ctrl+alt+p".to_string(),
            media_play_pause: String::new(),
            media_next: String::new(),
            media_previous: String::new(),
            media_stop: String::new(),
            cycle_players: String::new(),
            collapse_now: String::new(),
            reload_config: String::new(),
            toggle_debug_overlay: "ctrl+alt+d".to_string(),
        }
    }
}

/// User defined actions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShortcutsConfig {
    /// The configured actions.
    pub list: Vec<ShortcutConfig>,
}

/// One user defined action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShortcutConfig {
    /// Unique identifier used by the IPC `shortcut` command.
    pub id: String,
    /// Tooltip and accessibility name.
    pub label: String,
    /// Bundled icon name or a path to a user SVG.
    pub icon: String,
    /// Command to run.
    pub command: String,
    /// Arguments, preferred over a quoted command string.
    pub args: Vec<String>,
    /// Working directory for the spawned process.
    pub cwd: String,
    /// Global hotkey that triggers this action.
    pub hotkey: String,
    /// Whether to show this action in the island.
    pub show_in_island: bool,
    /// Trigger only through IPC, never by clicking.
    pub ipc_only: bool,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            icon: String::new(),
            command: String::new(),
            args: Vec::new(),
            cwd: String::new(),
            hotkey: String::new(),
            show_in_island: true,
            ipc_only: false,
        }
    }
}

impl ShortcutConfig {
    /// A shortcut is runnable only if it names a command. A shortcut with only
    /// an `ipc_only` flag and no command would silently do nothing.
    #[must_use]
    pub fn is_runnable(&self) -> bool {
        !self.id.is_empty() && !self.command.is_empty()
    }
}

/// System tray.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrayConfig {
    /// Master switch.
    pub enabled: bool,
    /// Show the tray icon at startup rather than only on demand.
    pub show_on_startup: bool,
    /// Icon name, or `auto` to pick from the current state.
    pub icon: String,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_on_startup: false,
            icon: "auto".to_string(),
        }
    }
}

/// Animation tuning and accessibility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MotionConfig {
    /// Disable springs and overshoot entirely.
    pub reduced: bool,
    /// Global animation speed multiplier.
    pub master_scale: f32,
    /// Morph spring response in seconds.
    pub morph_response_s: f32,
    /// Morph spring damping ratio.
    pub morph_damping: f32,
    /// Content crossfade duration in milliseconds.
    pub content_fade_ms: u32,
    /// Animate the island in on startup.
    pub startup_reveal: bool,
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self {
            reduced: false,
            master_scale: 1.0,
            morph_response_s: 0.34,
            morph_damping: 0.78,
            content_fade_ms: 160,
            startup_reveal: true,
        }
    }
}

/// Typography.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextConfig {
    /// Base font size in logical pixels.
    pub font_size: f32,
    /// Which bundled weight to use.
    pub font_weight: FontWeight,
    /// Track and artist title size.
    pub title_size: f32,
    /// Artist size.
    pub artist_size: f32,
    /// Path to a user font file, empty for the bundled one.
    pub font_file: String,
    /// Primary text color.
    pub color: String,
    /// Secondary text color.
    pub dim_color: String,
    /// Scroll text that does not fit.
    pub marquee: bool,
    /// Use a hard threshold instead of smooth coverage.
    pub sharp: bool,
    /// Extra spacing between glyphs.
    pub letter_spacing: f32,
}

/// Bundled font weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum FontWeight {
    /// Regular weight.
    #[default]
    Regular,
    /// Semibold weight.
    Semibold,
}

impl Default for TextConfig {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            font_weight: FontWeight::Regular,
            title_size: 15.0,
            artist_size: 13.0,
            font_file: String::new(),
            color: "#FFFFFFFF".to_string(),
            dim_color: "#B3FFFFFF".to_string(),
            marquee: true,
            sharp: false,
            letter_spacing: 0.0,
        }
    }
}

/// Expert settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdvancedConfig {
    /// Frame rate ceiling when not vsynced.
    pub target_fps: u32,
    /// Wait for the display refresh before presenting.
    pub vsync: bool,
    /// Graphics API selection.
    pub gpu_backend: GpuBackend,
    /// Log verbosity.
    pub log_level: LogLevel,
    /// Log file path, empty for the platform default.
    pub log_file: String,
    /// Master switch for the control socket.
    pub ipc_enabled: bool,
    /// Socket or pipe name, empty for the platform default.
    pub ipc_name: String,
    /// Allow the dangerous `execute` IPC command.
    pub ipc_allow_exec: bool,
    /// Request the low power adapter profile and cap the frame rate.
    pub power_saving: bool,
    /// Path to a user WGSL file overriding the island effect.
    pub experimental_wgsl: String,
}

/// Graphics API selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum GpuBackend {
    /// Probe in preference order.
    #[default]
    Auto,
    /// Vulkan.
    Vulkan,
    /// Direct3D 12.
    Dx12,
    /// Metal, macOS only.
    Metal,
    /// OpenGL 3.3 via the GLES backend.
    Gl,
}

/// Log verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum LogLevel {
    /// Nothing but failures.
    Error,
    /// Failures and recoverable problems.
    Warn,
    /// Lifecycle milestones.
    #[default]
    Info,
    /// Per subsystem detail.
    Debug,
    /// Everything, including per frame.
    Trace,
}

impl LogLevel {
    /// Parse a level name, case insensitive.
    #[must_use]
    pub fn from_id(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }
}

impl Default for AdvancedConfig {
    fn default() -> Self {
        Self {
            target_fps: 60,
            vsync: true,
            gpu_backend: GpuBackend::Auto,
            log_level: LogLevel::Info,
            log_file: String::new(),
            ipc_enabled: true,
            ipc_name: String::new(),
            ipc_allow_exec: false,
            power_saving: true,
            experimental_wgsl: String::new(),
        }
    }
}

/// Per-widget overrides.
///
/// Any widget accepts its own RGB block and geometry hints, so the media widget
/// can be red while the clock stays neutral. See `lumina-color` for the
/// precedence chain.
///
/// Note this struct cannot use `deny_unknown_fields` together with `flatten`,
/// which serde does not support. Unknown per-widget keys are therefore tolerated
/// here and reported by the validator, which is the same treatment every other
/// unknown key receives.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetConfig {
    /// Per-widget settings, keyed by widget id such as `media` or `clock`.
    #[serde(flatten)]
    pub widgets: std::collections::BTreeMap<String, WidgetEntry>,
}

/// Settings for one widget.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WidgetEntry {
    /// Hide this widget without removing its configuration.
    pub enabled: Option<bool>,
    /// Color override for this widget.
    pub rgb: Option<WidgetRgb>,
    /// Share of the surplus width this widget absorbs.
    pub flex: Option<f32>,
    /// Extra padding on this widget's leading edge.
    pub margin_leading: Option<f32>,
}

/// Color override for one widget.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WidgetRgb {
    /// Replace the global engine with a fixed color.
    pub override_palette: Option<bool>,
    /// Color to use when overriding.
    pub primary: Option<String>,
    /// Glow strength for this widget.
    pub glow_intensity: Option<f32>,
}

/// A parsed color together with whether it was valid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedColor {
    /// The color.
    pub value: Srgb,
    /// True when the input parsed cleanly.
    pub valid: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips_through_toml() {
        let config = Config::default();
        let text = config.to_toml_string();
        let parsed = Config::from_toml_str(&text).expect("the serialized default must parse");
        assert_eq!(
            config, parsed,
            "default config must survive a TOML round trip"
        );
    }

    #[test]
    fn section_defaults_match_the_standalone_defaults() {
        let config = Config::default();
        assert_eq!(config.general, GeneralConfig::default());
        assert_eq!(config.appearance, AppearanceConfig::default());
        assert_eq!(config.rgb, RgbConfig::default());
        assert_eq!(config.audio, AudioConfig::default());
        assert_eq!(config.layout, LayoutConfig::default());
    }

    #[test]
    fn missing_sections_fall_back_to_defaults() {
        let parsed =
            Config::from_toml_str("[rgb]\nmode = \"cycling\"\n").expect("partial file parses");
        assert_eq!(parsed.rgb.mode, RgbMode::Cycling);
        assert_eq!(parsed.appearance, AppearanceConfig::default());
        assert_eq!(parsed.general, GeneralConfig::default());
    }

    #[test]
    fn position_ids_round_trip() {
        for position in Position::ALL {
            assert_eq!(Position::from_id(position.id()), Some(position));
        }
        assert_eq!(Position::from_id("Top-Center"), Some(Position::TopCenter));
        assert_eq!(
            Position::from_id("  top_center "),
            Some(Position::TopCenter)
        );
        assert_eq!(Position::from_id("nowhere"), None);
    }

    #[test]
    fn horizontal_fraction_matches_alignment() {
        assert_eq!(Position::TopLeft.horizontal_fraction(), 0.0);
        assert_eq!(Position::TopCenter.horizontal_fraction(), 0.5);
        assert_eq!(Position::TopRight.horizontal_fraction(), 1.0);
        assert_eq!(Position::BottomCenter.horizontal_fraction(), 0.5);
    }

    #[test]
    fn is_top_distinguishes_edges() {
        assert!(Position::TopCenter.is_top());
        assert!(Position::TopLeft.is_top());
        assert!(!Position::BottomCenter.is_top());
        assert!(!Position::BottomRight.is_top());
    }

    #[test]
    fn rgb_mode_ids_round_trip() {
        for mode in RgbMode::ALL {
            assert_eq!(RgbMode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(RgbMode::from_id("REACTIVE"), Some(RgbMode::Reactive));
        assert_eq!(RgbMode::from_id("nope"), None);
    }

    #[test]
    fn visualizer_style_ids_round_trip() {
        for style in VisualizerStyle::ALL {
            assert_eq!(style.id(), style.id().to_string());
        }
    }

    #[test]
    fn fft_size_reports_real_sample_counts() {
        assert_eq!(FftSize::Size1024.samples(), 1024);
        assert_eq!(FftSize::default().samples(), 4096);
        for size in FftSize::ALL {
            assert!(
                size.samples().is_power_of_two(),
                "FFT sizes must be powers of two"
            );
        }
    }

    #[test]
    fn appearance_builds_clamped_geometry() {
        // A user who swaps the two widths still gets a valid inverted trapezoid.
        let appearance = AppearanceConfig {
            top_half_width: 10.0,
            bottom_half_width: 900.0,
            ..AppearanceConfig::default()
        };
        let geometry = appearance.geometry();
        assert!(geometry.is_inverted_trapezoid());
        assert!(geometry.height >= 8.0);
    }

    #[test]
    fn appearance_geometry_clamps_out_of_range_values() {
        let appearance = AppearanceConfig {
            corner_radius: 5000.0,
            height: -20.0,
            ..AppearanceConfig::default()
        };
        let geometry = appearance.geometry();
        assert!(geometry.height > 0.0, "negative height must be repaired");
        assert!(geometry.effective_radius() <= geometry.half_h());
    }

    #[test]
    fn shortcut_is_runnable_only_with_an_id_and_a_command() {
        let mut shortcut = ShortcutConfig::default();
        assert!(!shortcut.is_runnable(), "empty shortcut is not runnable");
        shortcut.id = "a".to_string();
        assert!(!shortcut.is_runnable(), "no command means nothing to run");
        shortcut.command = "echo hi".to_string();
        assert!(shortcut.is_runnable());
    }

    #[test]
    fn layout_default_order_is_the_documented_order() {
        let layout = LayoutConfig::default();
        assert_eq!(layout.order, DEFAULT_WIDGET_ORDER.to_vec());
    }

    #[test]
    fn log_level_parsing_is_lenient_about_aliases() {
        assert_eq!(LogLevel::from_id("WARN"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_id(" warning "), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_id("loud"), None);
    }

    #[test]
    fn log_levels_are_ordered_by_verbosity() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }

    #[test]
    fn get_path_resolves_nested_keys() {
        let mut config = Config::default();
        config.rgb.mode = RgbMode::Cycling;
        config.audio.band_count = 48;

        assert_eq!(config.get_path("rgb.mode").as_deref(), Some("cycling"));
        assert_eq!(config.get_path("audio.band_count").as_deref(), Some("48"));
        assert_eq!(
            config.get_path("audio.visualizer.style").as_deref(),
            Some("bars")
        );
        assert_eq!(config.get_path("rgb.nonexistent"), None);
        assert_eq!(config.get_path("nonexistent.key"), None);
    }

    #[test]
    fn unknown_keys_inside_a_known_section_are_rejected_by_serde() {
        // Serde is strict inside sections so that typos surface as errors,
        // which the validator then converts into suggestions.
        let result = Config::from_toml_str("[rgb]\nmod = \"cycling\"\n");
        assert!(result.is_err(), "a misspelled key must not silently parse");
    }

    #[test]
    fn weather_provider_defaults_to_the_keyless_option() {
        let config = Config::default();
        assert_eq!(config.weather.provider, WeatherProvider::OpenMeteo);
        assert_eq!(config.weather.api_key, "");
    }

    #[test]
    fn monitor_selector_serializes_by_variant() {
        let primary = toml::Value::try_from(MonitorSelector::Primary).expect("serialize");
        assert_eq!(primary.as_str(), Some("primary"));
    }
}
