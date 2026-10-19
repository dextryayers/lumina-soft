//! Wayland global hotkeys.
//!
//! # Status
//!
//! Native Wayland sessions have no global key grab. The supported path is the
//! `org.freedesktop.portal.GlobalShortcuts` portal, which arrives with the
//! Wayland layer in Phase 12. Until it exists, [`WaylandHotkeys`] reports
//! [`HotkeyError::Unsupported`] on every registration instead of pretending to
//! have registered a binding.
//!
//! Under XWayland the session still talks X11, so [`crate::x11_hotkeys`] works
//! and this module is not consulted.

use crate::hotkey::{HotkeyBinding, HotkeyError, HotkeyEvent, HotkeyRegistry};

/// True when the environment describes a native Wayland session.
///
/// `XDG_SESSION_TYPE=wayland` wins; otherwise a set `WAYLAND_DISPLAY` is
/// enough, because that is what a client launched from a Wayland terminal
/// sees even when the session type variable is missing.
#[must_use]
pub fn session_is_wayland() -> bool {
    session_is_wayland_from(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    )
}

/// The environment test behind [`session_is_wayland`], separated for tests.
#[must_use]
pub fn session_is_wayland_from(session_type: Option<&str>, wayland_display: Option<&str>) -> bool {
    if let Some(session_type) = session_type {
        return session_type.eq_ignore_ascii_case("wayland");
    }
    wayland_display.is_some_and(|display| !display.is_empty())
}

/// The placeholder backend for native Wayland sessions.
///
/// It exists so backend selection can return a concrete type with a precise
/// error instead of silently dropping the user's bindings.
#[derive(Debug, Default)]
pub struct WaylandHotkeys;

impl WaylandHotkeys {
    /// The portal is not implemented yet, so construction fails explicitly.
    ///
    /// # Errors
    ///
    /// Always returns [`HotkeyError::Unsupported`] until Phase 12.
    pub fn new() -> Result<Self, HotkeyError> {
        Err(HotkeyError::Unsupported)
    }
}

impl HotkeyRegistry for WaylandHotkeys {
    fn register(&mut self, _binding: HotkeyBinding) -> Result<(), HotkeyError> {
        Err(HotkeyError::Unsupported)
    }

    fn unregister(&mut self, _action: &str) -> Result<(), HotkeyError> {
        Ok(())
    }

    fn poll(&mut self) -> Vec<HotkeyEvent> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_type_variable_wins() {
        assert!(session_is_wayland_from(Some("wayland"), None));
        assert!(session_is_wayland_from(Some("Wayland"), Some(":0")));
        assert!(!session_is_wayland_from(Some("x11"), Some("wayland-0")));
    }

    #[test]
    fn a_wayland_display_is_used_when_the_session_type_is_missing() {
        assert!(session_is_wayland_from(None, Some("wayland-0")));
        assert!(!session_is_wayland_from(None, Some("")));
        assert!(!session_is_wayland_from(None, None));
    }

    #[test]
    fn registration_reports_the_missing_portal() {
        let mut backend = WaylandHotkeys;
        let binding = HotkeyBinding {
            action: "toggle_visibility".to_string(),
            hotkey: crate::hotkey::Hotkey::parse("ctrl+alt+l").unwrap().unwrap(),
        };
        assert_eq!(backend.register(binding), Err(HotkeyError::Unsupported));
        assert_eq!(backend.unregister("toggle_visibility"), Ok(()));
        assert_eq!(backend.poll().len(), 0);
    }
}
