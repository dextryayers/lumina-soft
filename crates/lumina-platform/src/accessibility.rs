//! Operating-system accessibility signals.
//!
//! Two settings matter to Lumina:
//!
//! - **Reduced motion.** Windows exposes `SPI_GETCLIENTAREAANIMATION`; GNOME and
//!   KDE expose their own animation keys. The config's `motion.reduced` flag is
//!   ORed with this by the orchestrator.
//! - **Screen reader.** Windows exposes `SPI_GETSCREENREADER`. The UI is an
//!   overlay, not an accessibility-tree citizen, so announcements are mirrored
//!   through platform notifications instead. [`AnnouncementGate`] bounds their
//!   rate; delivery is the orchestrator's responsibility because it owns the
//!   notification integration.
//!
//! Detection is deliberately best-effort. On platforms with no portable query,
//! the answer is `false` rather than a guess. `LUMINA_REDUCED_MOTION` and
//! `LUMINA_SCREEN_READER` override detection for testing and for users whose
//! desktop does not expose a setting.

use std::time::{Duration, Instant};

use lumina_config::schema::{AccessibilityConfig, ScreenReaderMode};

/// Environment variable that forces reduced motion on (`1`, `true`, `yes`, `on`)
/// or off (`0`, `false`, `no`, `off`) regardless of the desktop setting.
pub const REDUCED_MOTION_ENV: &str = "LUMINA_REDUCED_MOTION";

/// Environment variable that forces screen-reader mode for announcement tests.
pub const SCREEN_READER_ENV: &str = "LUMINA_SCREEN_READER";

/// True when the operating system asks applications to reduce motion.
///
/// The environment override wins so the behavior can be exercised on CI and by
/// users on desktops Lumina cannot query.
#[must_use]
pub fn os_reduced_motion() -> bool {
    if let Some(forced) = boolean_env(REDUCED_MOTION_ENV) {
        return forced;
    }
    platform_reduced_motion()
}

/// True when a screen reader is currently running.
///
/// This is a hint, not a guarantee: a reader may start after Lumina. The config
/// mode `On` should still be honoured even when this returns `false`.
#[must_use]
pub fn os_screen_reader_active() -> bool {
    if let Some(forced) = boolean_env(SCREEN_READER_ENV) {
        return forced;
    }
    platform_screen_reader_active()
}

/// Resolve whether announcements should be delivered at all.
#[must_use]
pub fn screen_reader_enabled(config: &AccessibilityConfig) -> bool {
    match config.screen_reader {
        ScreenReaderMode::On => true,
        ScreenReaderMode::Off => false,
        ScreenReaderMode::Auto => os_screen_reader_active(),
    }
}

/// A rate limiter for screen-reader announcements.
///
/// State changes can fire faster than a person can hear them: a volume slider
/// alone produces dozens of updates per second. The gate collapses rapid
/// changes to at most one announcement per `announce_min_interval_ms`, while
/// still allowing the first announcement immediately.
#[derive(Debug, Clone)]
pub struct AnnouncementGate {
    mode: ScreenReaderMode,
    min_interval: Duration,
    last: Option<Instant>,
    dropped: u64,
}

impl AnnouncementGate {
    /// Build a gate from the accessibility configuration.
    #[must_use]
    pub fn from_config(config: &AccessibilityConfig) -> Self {
        Self::new(config.screen_reader, config.announce_min_interval_ms)
    }

    /// Build a gate from an explicit mode and interval.
    ///
    /// Intervals are clamped to 0..=60 seconds to match config validation.
    #[must_use]
    pub fn new(mode: ScreenReaderMode, min_interval_ms: u32) -> Self {
        Self {
            mode,
            min_interval: Duration::from_millis(u64::from(min_interval_ms.min(60_000))),
            last: None,
            dropped: 0,
        }
    }

    /// Replace the policy without forgetting the last announcement time.
    pub fn reconfigure(&mut self, config: &AccessibilityConfig) {
        self.mode = config.screen_reader;
        self.min_interval =
            Duration::from_millis(u64::from(config.announce_min_interval_ms.min(60_000)));
    }

    /// Whether the configured mode allows announcements on this system.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        match self.mode {
            ScreenReaderMode::On => true,
            ScreenReaderMode::Off => false,
            ScreenReaderMode::Auto => os_screen_reader_active(),
        }
    }

    /// Ask whether an announcement may be delivered at `now`.
    ///
    /// Returns `false` when the mode is disabled or the rate limit has not
    /// elapsed. A `true` result records `now`, so callers must announce when
    /// they receive it.
    pub fn should_announce(&mut self, now: Instant) -> bool {
        if !self.is_enabled() {
            return false;
        }
        if let Some(last) = self.last {
            if now.saturating_duration_since(last) < self.min_interval {
                self.dropped = self.dropped.saturating_add(1);
                return false;
            }
        }
        self.last = Some(now);
        true
    }

    /// Number of announcements suppressed by the rate limit.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Forget the last announcement, for example when the island collapses.
    pub fn reset(&mut self) {
        self.last = None;
        self.dropped = 0;
    }
}

/// Deliver one mirrored announcement through the platform notification channel.
///
/// Windows uses a toast through the WinRT notification API from a short
/// PowerShell invocation; Linux uses `notify-send`. The process is spawned and
/// detached so a slow notification daemon cannot stall the UI. Callers should
/// rate-limit with [`AnnouncementGate`] first.
///
/// Returns an error when the platform has no supported channel or the helper
/// cannot be started. A notification daemon that rejects the toast after startup
/// is not observable here.
pub fn deliver_announcement(title: &str, body: &str) -> std::io::Result<()> {
    let (program, args) = notification_command(title, body).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no notification channel is available on this platform",
        )
    })?;
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Build the notification helper command for the current platform.
///
/// Exposed for tests and for wording the diagnostics message. The title and
/// body are always separate argv entries; they are never interpolated into a
/// shell command.
#[must_use]
pub fn notification_command(title: &str, body: &str) -> Option<(String, Vec<String>)> {
    notification_impl(title, body)
}

#[cfg(target_os = "windows")]
#[allow(clippy::unnecessary_wraps)]
fn notification_impl(title: &str, body: &str) -> Option<(String, Vec<String>)> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] > $null
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument
$xml.LoadXml('<toast><visual><binding template="ToastText02"><text id="1"></text><text id="2"></text></binding></visual></toast>')
$texts = $xml.GetElementsByTagName('text')
$texts.Item(0).AppendChild($xml.CreateTextNode($args[0])) > $null
$texts.Item(1).AppendChild($xml.CreateTextNode($args[1])) > $null
$toast = New-Object Windows.UI.Notifications.ToastNotification $xml
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Lumina').Show($toast)
"#;
    Some((
        "powershell.exe".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-WindowStyle".to_string(),
            "Hidden".to_string(),
            "-Command".to_string(),
            SCRIPT.to_string(),
            title.to_string(),
            body.to_string(),
        ],
    ))
}

#[cfg(all(unix, not(target_os = "macos")))]
#[allow(clippy::unnecessary_wraps)]
fn notification_impl(title: &str, body: &str) -> Option<(String, Vec<String>)> {
    Some((
        "notify-send".to_string(),
        vec![
            "--app-name=Lumina".to_string(),
            "--urgency=low".to_string(),
            title.to_string(),
            body.to_string(),
        ],
    ))
}

#[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos")))))]
fn notification_impl(_title: &str, _body: &str) -> Option<(String, Vec<String>)> {
    None
}

fn boolean_env(name: &str) -> Option<bool> {
    parse_boolean(&std::env::var(name).ok()?)
}

fn parse_boolean(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn platform_reduced_motion() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let mut animations_enabled = 1i32;
    let result = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(std::ptr::from_mut(&mut animations_enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    result.is_ok() && animations_enabled == 0
}

#[cfg(target_os = "windows")]
fn platform_screen_reader_active() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETSCREENREADER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let mut active = 0i32;
    let result = unsafe {
        SystemParametersInfoW(
            SPI_GETSCREENREADER,
            0,
            Some(std::ptr::from_mut(&mut active).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    result.is_ok() && active != 0
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_reduced_motion() -> bool {
    // GNOME/GTK desktops expose this through GSettings. KDE uses a duration
    // factor where zero means animations are disabled. Both commands are
    // consulted once at startup, not on the frame path.
    if let Some(output) = command_stdout(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "enable-animations"],
    ) {
        if output.trim() == "false" {
            return true;
        }
    }
    if let Some(output) = command_stdout(
        "kreadconfig6",
        &[
            "--file",
            "kdeglobals",
            "--group",
            "KDE",
            "--key",
            "AnimationDurationFactor",
        ],
    ) {
        if output.trim() == "0" {
            return true;
        }
    }
    false
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_screen_reader_active() -> bool {
    // There is no portable query. Orca sets this variable while running.
    std::env::var("ORCA_ACCESSIBILITY").is_ok() || std::env::var("AT_SPI_BUS").is_ok()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos")))))]
fn platform_reduced_motion() -> bool {
    false
}

#[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos")))))]
fn platform_screen_reader_active() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_announcement_is_immediate() {
        let mut gate = AnnouncementGate::new(ScreenReaderMode::On, 1_500);
        let start = Instant::now();
        assert!(gate.should_announce(start));
        assert_eq!(gate.dropped(), 0);
    }

    #[test]
    fn rapid_announcements_are_rate_limited() {
        let mut gate = AnnouncementGate::new(ScreenReaderMode::On, 1_500);
        let start = Instant::now();
        assert!(gate.should_announce(start));
        assert!(!gate.should_announce(start + Duration::from_millis(100)));
        assert!(!gate.should_announce(start + Duration::from_millis(1_499)));
        assert!(gate.should_announce(start + Duration::from_millis(1_500)));
        assert_eq!(gate.dropped(), 2);
    }

    #[test]
    fn a_disabled_mode_never_announces() {
        let mut gate = AnnouncementGate::new(ScreenReaderMode::Off, 0);
        let start = Instant::now();
        assert!(!gate.should_announce(start));
        assert!(!gate.should_announce(start + Duration::from_secs(10)));
        assert!(!gate.is_enabled());
    }

    #[test]
    fn explicit_on_does_not_depend_on_reader_detection() {
        let gate = AnnouncementGate::new(ScreenReaderMode::On, 0);
        assert!(gate.is_enabled());
    }

    #[test]
    fn reset_allows_an_immediate_announcement_again() {
        let mut gate = AnnouncementGate::new(ScreenReaderMode::On, 10_000);
        let start = Instant::now();
        assert!(gate.should_announce(start));
        assert!(!gate.should_announce(start + Duration::from_secs(1)));
        gate.reset();
        assert!(gate.should_announce(start + Duration::from_secs(1)));
    }

    #[test]
    fn intervals_are_clamped_like_config_validation() {
        let gate = AnnouncementGate::new(ScreenReaderMode::On, u32::MAX);
        assert_eq!(gate.min_interval, Duration::from_secs(60));
    }

    #[test]
    fn the_config_constructor_matches_the_schema_defaults() {
        let config = AccessibilityConfig::default();
        let gate = AnnouncementGate::from_config(&config);
        assert_eq!(gate.mode, config.screen_reader);
        assert_eq!(
            gate.min_interval,
            Duration::from_millis(u64::from(config.announce_min_interval_ms))
        );
    }

    #[test]
    fn notification_arguments_keep_text_out_of_the_script() {
        let (program, args) = notification_command("Lumina", "Now playing: A; rm -rf /")
            .expect("this platform has a notification channel");
        assert_ne!(program, "");
        assert!(args.iter().any(|arg| arg == "Now playing: A; rm -rf /"));
        #[cfg(target_os = "windows")]
        {
            let script = args
                .iter()
                .position(|arg| arg == "-Command")
                .expect("the command flag is present");
            assert!(!args[script + 1].contains("Now playing"));
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(program, "notify-send");
    }

    #[test]
    fn boolean_overrides_parse_common_spellings() {
        for value in ["1", "true", "YES", " on "] {
            assert_eq!(parse_boolean(value), Some(true), "{value}");
        }
        for value in ["0", "false", "NO", " off "] {
            assert_eq!(parse_boolean(value), Some(false), "{value}");
        }
        assert_eq!(parse_boolean("maybe"), None);
    }
}
