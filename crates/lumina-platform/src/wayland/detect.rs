//! Runtime detection for Wayland and its layer-shell extension.
//!
//! Detection happens in two steps, and keeping them separate matters:
//!
//! 1. The environment says whether a Wayland session is in use. That answer is
//!    available before any connection is made.
//! 2. The registry says whether `zwlr_layer_shell_v1` is present. That answer
//!    only exists after connecting to the compositor and receiving the initial
//!    global list.
//!
//! [`crate::capability::Platform::detect`] can only perform step one, so it
//! reports the optimistic variant. The Wayland backend performs step two during
//! startup and downgrades to [`crate::capability::Platform::WaylandNoLayerShell`]
//! when the global is missing. Reporting what is true, rather than what the
//! protocol should provide, is the difference between a limitation a user can
//! work around and a bug they cannot explain.

use crate::capability::Platform;

/// Which display server the process should talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    /// An X11 session.
    X11,
    /// A Wayland session. The layer-shell global is not known yet.
    Wayland,
    /// Neither, for example a CI runner or an SSH session with no forwarding.
    Headless,
}

impl Session {
    /// True when this is a Wayland session.
    #[must_use]
    pub const fn is_wayland(self) -> bool {
        matches!(self, Self::Wayland)
    }
}

/// The protocol global that provides layer surfaces.
pub const LAYER_SHELL_GLOBAL: &str = "zwlr_layer_shell_v1";

/// Choose a session from environment values.
///
/// Empty values count as absent, because some login managers export the
/// variables as empty strings in sessions that do not use them. `WAYLAND_DISPLAY`
/// wins over `DISPLAY` when both are set, which is the normal situation in a
/// Wayland session with XWayland available.
#[must_use]
pub fn session_from(wayland_display: Option<&str>, x11_display: Option<&str>) -> Session {
    let present = |value: Option<&str>| value.is_some_and(|v| !v.trim().is_empty());
    if present(wayland_display) {
        Session::Wayland
    } else if present(x11_display) {
        Session::X11
    } else {
        Session::Headless
    }
}

/// True when a registry global list contains the layer-shell protocol.
///
/// The comparison is exact: protocol names are fixed strings and a prefix match
/// would accept a vendor fork with incompatible semantics.
#[must_use]
pub fn globals_include_layer_shell<'a>(globals: impl IntoIterator<Item = &'a str>) -> bool {
    globals.into_iter().any(|name| name == LAYER_SHELL_GLOBAL)
}

/// Resolve the platform from the session and a layer-shell probe.
#[must_use]
pub const fn platform_for(session: Session, layer_shell: bool) -> Platform {
    match session {
        Session::X11 => Platform::X11,
        Session::Wayland if layer_shell => Platform::WaylandLayerShell,
        Session::Wayland => Platform::WaylandNoLayerShell,
        Session::Headless => Platform::Unsupported,
    }
}

/// True when XWayland is available as an escape hatch.
///
/// A Wayland session that also exports `DISPLAY` lets the application fall back
/// to X11. That is a poorer experience for an overlay, because the compositor
/// still owns placement to a degree, but it is better than no window.
#[must_use]
pub fn xwayland_available(session: Session, x11_display: Option<&str>) -> bool {
    session.is_wayland() && x11_display.is_some_and(|value| !value.trim().is_empty())
}

/// An actionable explanation for a session that cannot anchor the island.
///
/// Returned text is shown by `--diagnostics` and the first-run summary, so it
/// names the missing protocol rather than saying "unsupported".
#[must_use]
pub const fn limitation_hint(
    session: Session,
    layer_shell: bool,
    xwayland: bool,
) -> Option<&'static str> {
    match (session, layer_shell) {
        (Session::Wayland, false) if xwayland => Some(
            "this compositor does not implement zwlr_layer_shell_v1; Lumina runs as an ordinary window, or under XWayland with DISPLAY set",
        ),
        (Session::Wayland, false) => Some(
            "this compositor does not implement zwlr_layer_shell_v1; Lumina runs as an ordinary window with the compositor choosing its position",
        ),
        (Session::Headless, _) => Some(
            "no Wayland or X11 display was found; start Lumina inside a graphical session",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_wins_when_both_displays_are_set() {
        assert_eq!(
            session_from(Some("wayland-1"), Some(":0")),
            Session::Wayland
        );
    }

    #[test]
    fn x11_is_used_when_only_display_is_set() {
        assert_eq!(session_from(None, Some(":0")), Session::X11);
    }

    #[test]
    fn empty_environment_values_are_absent() {
        assert_eq!(session_from(Some(""), Some("  ")), Session::Headless);
        assert_eq!(session_from(Some("   "), None), Session::Headless);
    }

    #[test]
    fn no_display_means_headless() {
        assert_eq!(session_from(None, None), Session::Headless);
    }

    #[test]
    fn layer_shell_detection_is_exact() {
        assert!(globals_include_layer_shell([
            "wl_compositor",
            "zwlr_layer_shell_v1"
        ]));
        assert!(!globals_include_layer_shell([
            "wl_compositor",
            "zwlr_layer_shell_v1_vendor_fork"
        ]));
        assert!(!globals_include_layer_shell(std::iter::empty()));
    }

    #[test]
    fn a_wayland_session_without_layer_shell_is_reported_honestly() {
        assert_eq!(
            platform_for(Session::Wayland, true),
            Platform::WaylandLayerShell
        );
        assert_eq!(
            platform_for(Session::Wayland, false),
            Platform::WaylandNoLayerShell
        );
    }

    #[test]
    fn x11_and_headless_ignore_the_layer_shell_probe() {
        assert_eq!(platform_for(Session::X11, false), Platform::X11);
        assert_eq!(platform_for(Session::Headless, true), Platform::Unsupported);
    }

    #[test]
    fn xwayland_is_only_an_option_on_wayland() {
        assert!(xwayland_available(Session::Wayland, Some(":1")));
        assert!(!xwayland_available(Session::Wayland, Some("")));
        assert!(!xwayland_available(Session::X11, Some(":0")));
        assert!(!xwayland_available(Session::Headless, None));
    }

    #[test]
    fn missing_layer_shell_produces_an_actionable_hint() {
        let hint = limitation_hint(Session::Wayland, false, false).expect("a hint");
        assert!(hint.contains("zwlr_layer_shell_v1"));
        assert!(
            limitation_hint(Session::Wayland, true, true).is_none(),
            "nothing is missing on a full layer-shell session"
        );
        assert!(limitation_hint(Session::X11, false, false).is_none());
    }

    #[test]
    fn a_headless_session_is_explained() {
        let hint = limitation_hint(Session::Headless, false, false).expect("a hint");
        assert!(hint.contains("graphical session"));
    }
}
