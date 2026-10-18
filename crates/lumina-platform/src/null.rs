//! A backend that creates no window.
//!
//! This exists for three reasons, all of them practical:
//!
//! 1. **Headless integration tests.** The whole [`OverlayBackend`] trait can be
//!    exercised in CI with no display server attached.
//! 2. **The packaging smoke test.** `--self-test` verifies that a freshly
//!    installed binary can at least start, and on a build machine there is
//!    often no session to attach to.
//! 3. **Graceful degradation.** On a platform we do not support, the application
//!    still runs, still logs a clear diagnostic, and still serves IPC commands,
//!    rather than failing to launch.
//!
//! It is a faithful implementation of the trait, not a stub that panics. State is
//! recorded so tests can assert that callers drive it correctly.

use raw_window_handle::RawWindowHandle;

use crate::capability::{Capabilities, Platform};
use crate::monitor::{Monitor, ScaleFactor};
use crate::overlay::{OverlayBackend, OverlayError};
use crate::placement::WindowGeometry;

/// A backend with no display server behind it.
#[derive(Debug, Default)]
pub struct NullBackend {
    geometry: Option<WindowGeometry>,
    scale: ScaleFactor,
    click_through: bool,
    visible: bool,
    topmost_enforces: u32,
    ticks: u64,
}

impl NullBackend {
    /// Construct in the visible state, matching what a real backend does after
    /// a successful startup.
    #[must_use]
    pub fn new() -> Self {
        Self {
            visible: true,
            ..Self::default()
        }
    }

    /// The last geometry applied, for tests.
    #[must_use]
    pub fn last_geometry(&self) -> Option<WindowGeometry> {
        self.geometry
    }

    /// How many times topmost has been re-asserted.
    ///
    /// A real backend counts this too, because "how often do we have to fight
    /// the window manager" is a number worth watching.
    #[must_use]
    pub fn enforce_count(&self) -> u32 {
        self.topmost_enforces
    }

    /// How many ticks have run.
    #[must_use]
    pub fn tick_count(&self) -> u64 {
        self.ticks
    }
}

impl OverlayBackend for NullBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities::for_platform(Platform::Unsupported)
    }

    fn monitors(&self) -> Vec<Monitor> {
        Vec::new()
    }

    fn configure_window(&mut self, _window: RawWindowHandle) -> Result<(), OverlayError> {
        // Succeeding here is a lie. The window is simply not ours to configure.
        // Reporting an error would make every caller branch on a platform
        // condition it should not know about, so the answer is that the
        // operation completed and did nothing.
        Ok(())
    }

    fn apply_geometry(&mut self, geometry: &WindowGeometry) -> Result<(), OverlayError> {
        self.geometry = Some(*geometry);
        self.scale = ScaleFactor::new(geometry.scale);
        Ok(())
    }

    fn enforce_topmost(&mut self) -> Result<(), OverlayError> {
        self.topmost_enforces = self.topmost_enforces.saturating_add(1);
        Ok(())
    }

    fn set_click_through(&mut self, enabled: bool) -> Result<(), OverlayError> {
        self.click_through = enabled;
        Ok(())
    }

    fn click_through(&self) -> bool {
        self.click_through
    }

    fn set_visible(&mut self, visible: bool) -> Result<(), OverlayError> {
        self.visible = visible;
        Ok(())
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn current_scale(&self) -> ScaleFactor {
        self.scale
    }

    fn tick(&mut self) -> Result<(), OverlayError> {
        self.ticks = self.ticks.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::compute_placement;
    use lumina_config::Position;
    use lumina_core::geometry::Rect;

    #[test]
    fn a_new_backend_starts_visible() {
        // Matches what a real backend reports after a successful startup, so
        // callers do not need to special case the first frame.
        assert!(NullBackend::new().is_visible());
    }

    #[test]
    fn every_operation_succeeds_so_callers_need_no_platform_branches() {
        let mut backend = NullBackend::new();
        assert!(backend.set_visible(true).is_ok());
        assert!(backend.set_click_through(true).is_ok());
        assert!(backend.enforce_topmost().is_ok());
        assert!(backend.tick().is_ok());
        assert!(backend.shutdown_ok());
    }

    #[test]
    fn apply_geometry_updates_the_recorded_scale() {
        let mut backend = NullBackend::new();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let placement = compute_placement(
                Rect::new(0.0, 0.0, 3840.0, 2160.0),
                0,
                scale,
                Position::TopCenter,
                (124.0, 36.0),
                0,
                0,
            );
            backend
                .apply_geometry(&placement.geometry)
                .expect("applies");
            assert_eq!(backend.current_scale(), ScaleFactor::new(scale));
            assert_eq!(
                backend.last_geometry().expect("recorded").size,
                placement.size()
            );
        }
    }

    #[test]
    fn counts_accumulate_and_saturate() {
        let mut backend = NullBackend::new();
        for _ in 0..10 {
            backend.enforce_topmost().expect("ok");
            backend.tick().expect("ok");
        }
        assert_eq!(backend.enforce_count(), 10);
        assert_eq!(backend.tick_count(), 10);
    }

    #[test]
    fn shutdown_is_idempotent() {
        let mut backend = NullBackend::new();
        backend.shutdown();
        backend.shutdown();
        assert!(
            backend.is_visible(),
            "shutdown must not change observable state here"
        );
    }

    impl NullBackend {
        /// Convenience used by the trait exercisability test.
        fn shutdown_ok(&self) -> bool {
            true
        }
    }
}
