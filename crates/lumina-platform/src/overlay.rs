//! The overlay backend trait.
//!
//! Everything the island window can be asked to do, in one place, so an
//! application never issues a platform call directly.

use raw_window_handle::RawWindowHandle;

use crate::capability::Capabilities;
use crate::monitor::{Monitor, ScaleFactor};
use crate::placement::WindowGeometry;

/// Something went wrong talking to the display server.
///
/// Backends report and continue. An overlay that cannot go topmost is still
/// worth showing, so no operation here is allowed to be fatal.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayError {
    /// The window handle was not usable.
    NoWindowHandle,
    /// The platform refused the operation.
    Platform(String),
    /// The operation is not supported on this platform.
    Unsupported(&'static str),
    /// A display server connection failed.
    NoDisplay(String),
}

impl std::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWindowHandle => f.write_str("the window handle is not available yet"),
            Self::Platform(message) => write!(f, "platform call failed: {message}"),
            Self::Unsupported(what) => write!(f, "{what} is not supported on this platform"),
            Self::NoDisplay(message) => write!(f, "cannot reach the display server: {message}"),
        }
    }
}

impl std::error::Error for OverlayError {}

/// Control over the island window.
///
/// Implemented once per display server. Every method is fallible and every
/// failure is recoverable: the caller logs and carries on, because a partially
/// working island is far better than no island.
pub trait OverlayBackend: Send {
    /// What this platform supports.
    fn capabilities(&self) -> Capabilities;

    /// Every attached monitor.
    fn monitors(&self) -> Vec<Monitor>;

    /// Apply the window style that keeps the island out of the taskbar and out
    /// of the alt-tab switcher, and gives it per-pixel alpha.
    ///
    /// Called once after the window exists, and again after a style reset.
    fn configure_window(&mut self, window: RawWindowHandle) -> Result<(), OverlayError>;

    /// Move and resize the window.
    fn apply_geometry(&mut self, geometry: &WindowGeometry) -> Result<(), OverlayError>;

    /// Re-assert the always-on-top position.
    ///
    /// Called after monitor changes and after the window manager reports a
    /// position change, because full-screen applications and some window
    /// managers will otherwise take the top position.
    fn enforce_topmost(&mut self) -> Result<(), OverlayError>;

    /// Turn click-through on or off.
    ///
    /// When on, mouse events pass through to whatever is underneath. Must not
    /// require recreating the window, because toggling it on hover would then
    /// flicker.
    fn set_click_through(&mut self, enabled: bool) -> Result<(), OverlayError>;

    /// Whether click-through is currently on.
    fn click_through(&self) -> bool;

    /// Show or hide the window without destroying it.
    fn set_visible(&mut self, visible: bool) -> Result<(), OverlayError>;

    /// Whether the window is currently visible.
    fn is_visible(&self) -> bool;

    /// The scale factor of the monitor the window is on.
    fn current_scale(&self) -> ScaleFactor;

    /// Per frame work that must happen on the platform's own thread.
    ///
    /// On Windows some state must be set from the thread that created the
    /// window. On X11 the connection has to be flushed. Doing this in one place
    /// keeps the call sites free of platform conditionals.
    fn tick(&mut self) -> Result<(), OverlayError>;

    /// Free any platform resources. Called once during shutdown.
    fn shutdown(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::Monitor;
    use crate::null::NullBackend;
    use lumina_core::geometry::Rect;

    #[test]
    fn overlay_errors_render_readable_messages() {
        assert!(OverlayError::NoWindowHandle
            .to_string()
            .contains("window handle"));
        assert!(OverlayError::Platform("code 5".into())
            .to_string()
            .contains("code 5"));
        assert!(OverlayError::Unsupported("layer-shell")
            .to_string()
            .contains("layer-shell"));
        assert!(OverlayError::NoDisplay("no $DISPLAY".into())
            .to_string()
            .contains("no $DISPLAY"));
    }

    #[test]
    fn overlay_errors_never_contain_an_em_dash() {
        for error in [
            OverlayError::NoWindowHandle,
            OverlayError::Platform("a".into()),
            OverlayError::Unsupported("b"),
            OverlayError::NoDisplay("c".into()),
        ] {
            let text = error.to_string();
            assert!(!text.contains('\u{2014}'), "em dash in {text}");
            assert!(!text.contains('\u{2013}'), "en dash in {text}");
        }
    }

    /// A backend is only useful if the whole trait can be exercised without a
    /// display server, which is what makes headless integration tests and the
    /// packaging smoke test possible.
    ///
    /// `configure_window` is deliberately not called here: it needs a real
    /// window handle, which by definition does not exist in a headless test.
    /// The real backends cover it, and the null backend ignores the handle
    /// anyway.
    #[test]
    fn the_trait_is_fully_exercisable_without_a_display() {
        let mut backend = NullBackend::new();

        let monitor = Monitor::new(0, "test", Rect::new(0.0, 0.0, 1920.0, 1080.0), true);
        let geometry = crate::placement::compute_placement(
            monitor.bounds,
            0,
            1.0,
            lumina_config::Position::TopCenter,
            (124.0, 36.0),
            0,
            0,
        )
        .geometry;
        backend
            .apply_geometry(&geometry)
            .expect("null backend accepts geometry");

        backend
            .set_click_through(true)
            .expect("null backend accepts this");
        assert!(backend.click_through());
        backend
            .set_click_through(false)
            .expect("null backend accepts this");
        assert!(!backend.click_through());

        backend
            .set_visible(false)
            .expect("null backend accepts this");
        assert!(!backend.is_visible());
        backend
            .set_visible(true)
            .expect("null backend accepts this");
        assert!(backend.is_visible());

        backend
            .enforce_topmost()
            .expect("null backend accepts this");
        backend.tick().expect("null backend accepts this");
        assert_eq!(backend.current_scale(), ScaleFactor::unscaled());
        assert!(backend.monitors().is_empty());
        backend.shutdown();
    }

    #[test]
    fn the_null_backend_reports_no_capabilities_rather_than_pretending() {
        let caps = NullBackend::new().capabilities();
        assert_eq!(caps.platform, crate::capability::Platform::Unsupported);
    }

    #[test]
    fn the_null_backend_records_state_so_the_trait_can_be_asserted_on() {
        let mut backend = NullBackend::new();
        let geometry = crate::placement::compute_placement(
            Rect::new(0.0, 0.0, 1920.0, 1080.0),
            3,
            2.0,
            lumina_config::Position::BottomLeft,
            (200.0, 60.0),
            4,
            8,
        )
        .geometry;
        backend
            .apply_geometry(&geometry)
            .expect("null backend accepts geometry");

        assert_eq!(backend.last_geometry(), Some(geometry));
        assert_eq!(backend.current_scale(), ScaleFactor::new(2.0));
        assert_eq!(backend.enforce_count(), 0);
        backend
            .enforce_topmost()
            .expect("null backend accepts this");
        assert_eq!(backend.enforce_count(), 1);
    }
}
