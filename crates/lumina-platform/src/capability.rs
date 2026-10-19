//! What the current platform can actually do.
//!
//! Every platform in the support matrix has a different answer to some of
//! these questions, and the honest thing is to report the answer rather than
//! pretend. GNOME Wayland genuinely cannot reserve an exclusive zone. X11 has no
//! portable backdrop blur. Wayland without the shortcuts portal has no global
//! hotkeys.
//!
//! Callers use this to degrade knowingly: log what is unavailable, explain it in
//! `--diagnostics`, and hide the menu entries that would not work.

/// Which operating system and display server combination is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Windows 10 or 11, with the Win32 compositor.
    Windows,
    /// Linux, with an X11 display server.
    X11,
    /// Linux, with Wayland and a compositor that implements layer-shell.
    WaylandLayerShell,
    /// Linux, with Wayland and a compositor that does not implement layer-shell,
    /// such as GNOME.
    WaylandNoLayerShell,
    /// Any other platform, where Lumina runs with no window at all.
    Unsupported,
}

impl Platform {
    /// Detect the current platform at runtime.
    ///
    /// This reads the environment only. On Wayland it cannot know whether the
    /// compositor implements layer-shell without connecting to it, so it
    /// reports the optimistic variant and the backend must call
    /// [`Self::with_layer_shell_support`] after its registry probe. Reporting
    /// the pessimistic variant here instead would make every wlroots session
    /// look broken until the window exists.
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use crate::wayland::detect::{platform_for, session_from};

            let wayland = std::env::var("WAYLAND_DISPLAY").ok();
            let x11 = std::env::var("DISPLAY").ok();
            platform_for(session_from(wayland.as_deref(), x11.as_deref()), true)
        }
        #[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos")))))]
        {
            Self::Unsupported
        }
    }

    /// Refine a Wayland detection with the result of a layer-shell probe.
    ///
    /// Non-Wayland platforms are returned unchanged: an X11 session cannot gain
    /// layer-shell by connecting to something, and a probe on Windows is
    /// meaningless. This makes it safe for a backend to call unconditionally
    /// with whatever its registry saw.
    #[must_use]
    pub const fn with_layer_shell_support(self, supported: bool) -> Self {
        match self {
            Self::WaylandLayerShell | Self::WaylandNoLayerShell => {
                if supported {
                    Self::WaylandLayerShell
                } else {
                    Self::WaylandNoLayerShell
                }
            }
            other => other,
        }
    }

    /// True when a window can actually be created.
    #[must_use]
    pub const fn has_window(self) -> bool {
        !matches!(self, Self::Unsupported)
    }

    /// True when global hotkeys can be registered outside the application.
    #[must_use]
    pub const fn has_global_hotkeys(self) -> bool {
        matches!(self, Self::Windows | Self::X11)
    }

    /// Stable identifier for logs and diagnostics.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::X11 => "x11",
            Self::WaylandLayerShell => "wayland-layer-shell",
            Self::WaylandNoLayerShell => "wayland-no-layer-shell",
            Self::Unsupported => "unsupported",
        }
    }

    /// Human readable name.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::X11 => "X11",
            Self::WaylandLayerShell => "Wayland (layer-shell)",
            Self::WaylandNoLayerShell => "Wayland (no layer-shell)",
            Self::Unsupported => "Unsupported",
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

/// The capability matrix, resolved for the running system.
///
/// Built once at startup and thereafter read only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Which platform this describes.
    pub platform: Platform,
    /// The window can stay above all other windows.
    pub always_on_top: bool,
    /// The window is absent from the taskbar, dock, and alt-tab switcher.
    pub hidden_from_switcher: bool,
    /// The window has per-pixel transparency.
    pub per_pixel_alpha: bool,
    /// Clicks can be passed through to whatever is underneath.
    pub click_through: bool,
    /// The compositor blurs the backdrop for us.
    pub native_backdrop_blur: bool,
    /// The window can reserve screen space so maximised windows avoid it.
    pub exclusive_zone: bool,
    /// Position can be controlled precisely.
    pub precise_positioning: bool,
    /// Hotkeys work while another application has focus.
    pub global_hotkeys: bool,
    /// The window never takes keyboard focus.
    pub never_takes_focus: bool,
    /// Multiple monitors can be targeted individually.
    pub multi_monitor: bool,
    /// Per monitor display scaling is honoured.
    pub per_monitor_dpi: bool,
}

impl Capabilities {
    /// The capability set for a platform.
    ///
    /// # Panics
    ///
    /// Never.
    #[must_use]
    pub const fn for_platform(platform: Platform) -> Self {
        match platform {
            Platform::Windows => Self {
                platform,
                always_on_top: true,
                hidden_from_switcher: true,
                per_pixel_alpha: true,
                click_through: true,
                native_backdrop_blur: true,
                exclusive_zone: false,
                precise_positioning: true,
                global_hotkeys: true,
                never_takes_focus: true,
                multi_monitor: true,
                per_monitor_dpi: true,
            },
            Platform::X11 => Self {
                platform,
                always_on_top: true,
                hidden_from_switcher: true,
                per_pixel_alpha: true,
                click_through: true,
                // No portable X11 API for this. KWin and Mutter each have an
                // undocumented one, which is exactly the kind of dependency a
                // shipped application should not take.
                native_backdrop_blur: false,
                exclusive_zone: true,
                precise_positioning: true,
                global_hotkeys: true,
                never_takes_focus: true,
                multi_monitor: true,
                per_monitor_dpi: true,
            },
            Platform::WaylandLayerShell => Self {
                platform,
                always_on_top: true,
                hidden_from_switcher: true,
                per_pixel_alpha: true,
                click_through: true,
                native_backdrop_blur: false,
                exclusive_zone: true,
                precise_positioning: true,
                // Needs the GlobalShortcuts portal, which is frequently absent.
                // The caller probes for it rather than assuming.
                global_hotkeys: false,
                // Requested as on demand rather than never, so the island can be
                // keyboard operated while expanded.
                never_takes_focus: true,
                multi_monitor: true,
                per_monitor_dpi: true,
            },
            Platform::WaylandNoLayerShell => Self {
                platform,
                always_on_top: false,
                hidden_from_switcher: true,
                per_pixel_alpha: true,
                click_through: false,
                native_backdrop_blur: false,
                exclusive_zone: false,
                // Layer-shell anchors are how a Wayland client controls position.
                // Without it, the compositor decides.
                precise_positioning: false,
                global_hotkeys: false,
                never_takes_focus: false,
                multi_monitor: false,
                per_monitor_dpi: true,
            },
            Platform::Unsupported => Self {
                platform,
                always_on_top: false,
                hidden_from_switcher: false,
                per_pixel_alpha: false,
                click_through: false,
                native_backdrop_blur: false,
                exclusive_zone: false,
                precise_positioning: false,
                global_hotkeys: false,
                never_takes_focus: false,
                multi_monitor: false,
                per_monitor_dpi: false,
            },
        }
    }

    /// Set `global_hotkeys` according to a portal probe.
    ///
    /// On Wayland the answer is only known at runtime, so the static matrix
    /// assumes no and the caller upgrades it when the portal is present.
    pub fn set_global_hotkeys_available(&mut self, available: bool) {
        self.global_hotkeys = available;
    }

    /// Features that are unavailable, as human readable phrases.
    ///
    /// Used by `--diagnostics` and by the first-run summary, so a user learns
    /// about a limitation at the time it affects them rather than by noticing
    /// that something quietly does not work.
    #[must_use]
    pub fn limitations(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.always_on_top {
            out.push("the island may be covered by other windows");
        }
        if !self.hidden_from_switcher {
            out.push("the island may appear in the window switcher");
        }
        if !self.per_pixel_alpha {
            out.push("transparency is unavailable, so the island is an opaque rectangle");
        }
        if !self.click_through {
            out.push("clicks cannot pass through to windows underneath");
        }
        if !self.native_backdrop_blur {
            out.push("backdrop blur runs on Lumina's own slower path");
        }
        if !self.exclusive_zone {
            out.push("maximised windows may extend under the island");
        }
        if !self.precise_positioning {
            out.push("the compositor chooses the island's position");
        }
        if !self.global_hotkeys {
            out.push("hotkeys only work while the island has keyboard focus");
        }
        if !self.never_takes_focus {
            out.push("the island may briefly take keyboard focus");
        }
        if !self.multi_monitor {
            out.push("the island cannot be moved to another display");
        }
        if !self.per_monitor_dpi {
            out.push("display scaling is approximated from the system setting");
        }
        out
    }

    /// Whether the software blur path should be used.
    ///
    /// `Auto` means use the platform blur if present, otherwise our own.
    #[must_use]
    pub fn needs_software_blur(&self, requested: BackdropRequest) -> bool {
        match requested {
            BackdropRequest::Off => false,
            BackdropRequest::On => true,
            BackdropRequest::Auto => !self.native_backdrop_blur,
        }
    }
}

/// What the user asked for regarding backdrop blur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackdropRequest {
    /// Use the platform blur if there is one.
    Auto,
    /// Always use the software path.
    On,
    /// Never blur.
    Off,
}

impl From<lumina_config::BackdropMode> for BackdropRequest {
    fn from(value: lumina_config::BackdropMode) -> Self {
        match value {
            lumina_config::BackdropMode::Auto => Self::Auto,
            lumina_config::BackdropMode::On => Self::On,
            lumina_config::BackdropMode::Off => Self::Off,
        }
    }
}

/// Which kind of platform we are on.
///
/// Kept as a separate short enum because it is what the binary name in logs and
/// the environment variable in `--diagnostics` care about, while [`Platform`]
/// carries the display server detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    /// Windows.
    Windows,
    /// Linux.
    Linux,
    /// macOS, which is not a target but is worth naming rather than lumping in
    /// with "unsupported".
    MacOs,
    /// Anything else.
    Other,
}

impl PlatformKind {
    /// Stable identifier.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::MacOs => "macos",
            Self::Other => "other",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_produces_a_capability_set() {
        for platform in [
            Platform::Windows,
            Platform::X11,
            Platform::WaylandLayerShell,
            Platform::WaylandNoLayerShell,
            Platform::Unsupported,
        ] {
            let caps = Capabilities::for_platform(platform);
            assert_eq!(caps.platform, platform);
            assert_eq!(caps.platform.display_name(), platform.display_name());
            assert!(
                !caps.platform.id().is_empty(),
                "the stable id is what logs key off"
            );
        }
    }

    #[test]
    fn unsupported_can_do_nothing() {
        let caps = Capabilities::for_platform(Platform::Unsupported);
        assert!(!caps.platform.has_window());
        assert!(
            caps.limitations().len() >= 10,
            "everything should be listed as a limitation"
        );
    }

    #[test]
    fn windows_reports_only_the_one_thing_it_cannot_do() {
        // Windows genuinely cannot reserve an exclusive zone, so a maximised
        // window does extend under the island. That is a real limitation and the
        // matrix must say so rather than claiming parity.
        let limitations = Capabilities::for_platform(Platform::Windows).limitations();
        assert_eq!(
            limitations.len(),
            1,
            "expected only the exclusive zone note, got {limitations:?}"
        );
        assert!(limitations[0].contains("maximised"));
    }

    #[test]
    fn x11_is_honest_about_backdrop_blur() {
        let caps = Capabilities::for_platform(Platform::X11);
        assert!(
            !caps.native_backdrop_blur,
            "there is no portable X11 blur API"
        );
        assert!(caps
            .limitations()
            .iter()
            .any(|l| l.contains("backdrop blur")));
    }

    #[test]
    fn x11_reserves_space_but_windows_does_not() {
        // A dock window reserves space under EWMH; a Win32 topmost window does
        // not, and pretending otherwise would put the island under maximised
        // windows on Windows.
        assert!(Capabilities::for_platform(Platform::X11).exclusive_zone);
        assert!(!Capabilities::for_platform(Platform::Windows).exclusive_zone);
    }

    #[test]
    fn a_layer_shell_probe_refines_only_wayland() {
        assert_eq!(
            Platform::WaylandLayerShell.with_layer_shell_support(false),
            Platform::WaylandNoLayerShell
        );
        assert_eq!(
            Platform::WaylandNoLayerShell.with_layer_shell_support(true),
            Platform::WaylandLayerShell
        );
        assert_eq!(
            Platform::X11.with_layer_shell_support(false),
            Platform::X11,
            "a probe cannot change a non Wayland platform"
        );
        assert_eq!(
            Platform::Windows.with_layer_shell_support(true),
            Platform::Windows
        );
    }

    #[test]
    fn wayland_without_layer_shell_is_honest_about_losing_control() {
        let caps = Capabilities::for_platform(Platform::WaylandNoLayerShell);
        assert!(!caps.always_on_top);
        assert!(!caps.precise_positioning);
        assert!(!caps.click_through);
        assert!(
            caps.per_pixel_alpha,
            "transparency still works without layer-shell"
        );
    }

    #[test]
    fn layer_shell_wayland_keeps_position_control() {
        let caps = Capabilities::for_platform(Platform::WaylandLayerShell);
        assert!(caps.precise_positioning);
        assert!(caps.exclusive_zone);
        assert!(caps.click_through);
    }

    #[test]
    fn wayland_global_hotkeys_can_be_upgraded_by_a_portal_probe() {
        let mut caps = Capabilities::for_platform(Platform::WaylandLayerShell);
        assert!(!caps.global_hotkeys, "assumed unavailable until probed");
        caps.set_global_hotkeys_available(true);
        assert!(caps.global_hotkeys);
        assert!(!caps.limitations().iter().any(|l| l.contains("hotkeys")));
    }

    #[test]
    fn a_probe_cannot_grant_hotkeys_on_x11_which_already_has_them() {
        // Setting the same value twice must be idempotent, not cumulative.
        let mut caps = Capabilities::for_platform(Platform::X11);
        caps.set_global_hotkeys_available(true);
        caps.set_global_hotkeys_available(false);
        assert!(!caps.global_hotkeys);
    }

    #[test]
    fn global_hotkeys_match_the_platform_predicate() {
        for platform in [
            Platform::Windows,
            Platform::X11,
            Platform::WaylandLayerShell,
            Platform::WaylandNoLayerShell,
            Platform::Unsupported,
        ] {
            assert_eq!(
                Capabilities::for_platform(platform).global_hotkeys,
                platform.has_global_hotkeys(),
                "{} disagrees with itself",
                platform.display_name()
            );
        }
    }

    #[test]
    fn auto_uses_software_blur_only_where_the_platform_cannot() {
        let windows = Capabilities::for_platform(Platform::Windows);
        assert!(!windows.needs_software_blur(BackdropRequest::Auto));
        assert!(windows.needs_software_blur(BackdropRequest::On));

        let x11 = Capabilities::for_platform(Platform::X11);
        assert!(x11.needs_software_blur(BackdropRequest::Auto));

        // Off wins everywhere.
        for platform in [Platform::Windows, Platform::X11, Platform::Unsupported] {
            assert!(!Capabilities::for_platform(platform).needs_software_blur(BackdropRequest::Off));
        }
    }

    #[test]
    fn backdrop_request_maps_from_the_config_enum() {
        assert_eq!(
            BackdropRequest::from(lumina_config::BackdropMode::Auto),
            BackdropRequest::Auto
        );
        assert_eq!(
            BackdropRequest::from(lumina_config::BackdropMode::On),
            BackdropRequest::On
        );
        assert_eq!(
            BackdropRequest::from(lumina_config::BackdropMode::Off),
            BackdropRequest::Off
        );
    }

    #[test]
    fn limitations_are_empty_only_when_nothing_is_missing() {
        for platform in [
            Platform::Windows,
            Platform::X11,
            Platform::WaylandLayerShell,
            Platform::WaylandNoLayerShell,
            Platform::Unsupported,
        ] {
            let caps = Capabilities::for_platform(platform);
            let everything = caps.always_on_top
                && caps.hidden_from_switcher
                && caps.per_pixel_alpha
                && caps.click_through
                && caps.native_backdrop_blur
                && caps.exclusive_zone
                && caps.precise_positioning
                && caps.global_hotkeys
                && caps.never_takes_focus
                && caps.multi_monitor
                && caps.per_monitor_dpi;
            assert_eq!(
                caps.limitations().is_empty(),
                everything,
                "{platform}: the limitation list disagrees with the capability flags"
            );
        }
    }

    #[test]
    fn limitation_text_never_leaks_an_em_dash() {
        for platform in [
            Platform::Windows,
            Platform::X11,
            Platform::WaylandLayerShell,
            Platform::WaylandNoLayerShell,
            Platform::Unsupported,
        ] {
            for limitation in Capabilities::for_platform(platform).limitations() {
                assert!(!limitation.contains('\u{2014}'), "em dash in: {limitation}");
                assert!(!limitation.contains('\u{2013}'), "en dash in: {limitation}");
            }
        }
    }
}
