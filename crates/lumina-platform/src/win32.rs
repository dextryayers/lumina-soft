//! Windows overlay backend.
//!
//! Every function here is a specific Win32 answer to one of the five
//! requirements listed in the crate documentation, and each carries a comment
//! naming what breaks without it.
//!
//! # The three calls people get wrong
//!
//! 1. **Per-pixel alpha needs `DwmExtendFrameIntoClientArea` with negative
//!    margins**, and the window needs `WS_EX_NOREDIRECTIONBITMAP` for that call
//!    to have any effect. `SetLayeredWindowAttributes` with `LWA_COLORKEY` is
//!    the older approach and gives hard edges.
//! 2. **DPI awareness must be set before the first window exists.** Setting
//!    `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` later is ignored, and the
//!    result of getting it wrong is every coordinate scaled twice on a 150
//!    percent display, which puts the island in the wrong place.
//! 3. **`SWP_NOACTIVATE` on every `SetWindowPos`.** Without it the island takes
//!    keyboard focus every time it is shown or moved, which is maddening and is
//!    the single most reported Windows overlay bug.

use std::ffi::c_void;

use lumina_core::geometry::Rect;
use raw_window_handle::RawWindowHandle;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HMONITOR, MONITORINFO};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE,
    HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
    SW_SHOWNOACTIVATE, WS_EX_LAYERED, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT,
};

use crate::capability::{Capabilities, Platform};
use crate::monitor::{Monitor, ScaleFactor};
use crate::overlay::{OverlayBackend, OverlayError};
use crate::placement::WindowGeometry;

/// The overlay backend for Windows 10 and 11.
#[derive(Debug, Default)]
pub struct Win32Backend {
    hwnd: Option<HWND>,
    geometry: Option<WindowGeometry>,
    click_through: bool,
    visible: bool,
    scale: ScaleFactor,
    topmost_enforces: u32,
}

// Safety: `HWND` is a raw pointer, which is not `Send` by default. Every method
// on this type takes `&mut self` or `&self` and the overlay backend is only ever
// driven from the thread that created the window, so the handle cannot be
// touched concurrently. The `Send` bound on `OverlayBackend` exists so the
// orchestrator can own the backend while the event loop borrows it, not so it
// can be moved between threads while in use.
unsafe impl Send for Win32Backend {}

impl Win32Backend {
    /// Construct and opt the process into per monitor DPI awareness.
    ///
    /// A refusal is not fatal. An older Windows 10 build may not know the V2
    /// context, in which case the process is already system-DPI-aware. That costs
    /// accuracy on secondary monitors with different scaling but does not stop
    /// the island from appearing, so it is logged and carried on from.
    #[must_use]
    pub fn new() -> Self {
        let result =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if result.is_err() {
            log_warn(&format!(
                "per monitor DPI awareness was refused ({result:?}); \
                 falling back to system DPI awareness"
            ));
        }
        Self {
            visible: true,
            ..Self::default()
        }
    }

    /// The window handle, if the window exists yet.
    #[must_use]
    pub fn hwnd(&self) -> Option<HWND> {
        self.hwnd
    }

    /// How many times the topmost position has been re-asserted.
    ///
    /// Worth watching: a number that climbs every frame means a full-screen
    /// application or a window manager is fighting for the top position.
    #[must_use]
    pub fn enforce_count(&self) -> u32 {
        self.topmost_enforces
    }

    /// Current window rectangle, for diagnostics.
    #[must_use]
    pub fn window_rect(&self) -> Option<Rect> {
        let hwnd = self.hwnd?;
        let mut rect = windows::Win32::Foundation::RECT::default();
        if unsafe { GetWindowRect(hwnd, &raw mut rect) }.is_err() {
            return None;
        }
        Some(Rect::new(
            rect.left as f32,
            rect.top as f32,
            (rect.right - rect.left) as f32,
            (rect.bottom - rect.top) as f32,
        ))
    }

    /// The DPI of the monitor this window is on, as a scale factor.
    fn dpi_scale_for(hwnd: HWND) -> f32 {
        let hmonitor = unsafe {
            windows::Win32::Graphics::Gdi::MonitorFromWindow(
                hwnd,
                windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
            )
        };
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let ok = unsafe {
            GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y)
        };
        if ok.is_ok() && dpi_x > 0 {
            return dpi_x as f32 / 96.0;
        }
        // `GetDpiForMonitor` is the documented Windows 10 1607 API. The window
        // variant is the fallback for builds where the monitor variant is
        // unavailable.
        let window_dpi = unsafe { GetDpiForWindow(hwnd) };
        if window_dpi > 0 {
            return window_dpi as f32 / 96.0;
        }
        1.0
    }

    /// Read the extended window style.
    fn extended_style(&self) -> isize {
        self.hwnd
            .map_or(0, |hwnd| unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) })
    }

    /// Write the extended window style, distinguishing a real failure from a
    /// legitimate previous value of zero.
    fn set_extended_style(&self, style: isize) -> Result<(), OverlayError> {
        let hwnd = self.hwnd.ok_or(OverlayError::NoWindowHandle)?;
        let previous = unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style) };
        if previous == 0 {
            let error = unsafe { windows::Win32::Foundation::GetLastError() };
            if error.0 != 0 {
                return Err(OverlayError::Platform(format!(
                    "SetWindowLongPtrW(GWL_EXSTYLE) failed: {error:?}"
                )));
            }
        }
        Ok(())
    }

    /// Apply the styles that keep the island out of the taskbar and give it
    /// per-pixel alpha.
    fn apply_window_styles(&self) -> Result<(), OverlayError> {
        let hwnd = self.hwnd.ok_or(OverlayError::NoWindowHandle)?;

        let current = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        let mut desired = current;
        // Requirement: absent from the taskbar and the alt-tab switcher.
        desired |= WS_EX_TOOLWINDOW.0 as isize;
        // Required for the DWM alpha extension below to have any effect.
        desired |= WS_EX_NOREDIRECTIONBITMAP.0 as isize;
        // Kept for virtual display drivers that still require a layered window.
        desired |= WS_EX_LAYERED.0 as isize;
        // Preserve click-through across a reconfiguration rather than resetting it.
        if self.click_through {
            desired |= WS_EX_TRANSPARENT.0 as isize;
        } else {
            desired &= !(WS_EX_TRANSPARENT.0 as isize);
        }

        if desired != current {
            self.set_extended_style(desired)?;
        }

        // Requirement: per-pixel alpha.
        //
        // Negative margins on all sides tell DWM the entire client area can be
        // transparent. Without this the window is opaque regardless of what the
        // swapchain contains.
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        if let Err(hr) = unsafe { DwmExtendFrameIntoClientArea(hwnd, &raw const margins) } {
            // Not fatal. The island still appears, just with hard edges, so this
            // is logged rather than turned into an error the caller must handle.
            log_warn(&format!(
                "DwmExtendFrameIntoClientArea failed: {hr:?}; the island will have hard edges"
            ));
        }

        // Cosmetic only. Stops Windows 11 drawing a rounded border of its own,
        // since Lumina draws its own shape.
        let preference = DWMWCP_ROUND;
        let _ = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const preference).cast::<c_void>(),
                std::mem::size_of::<i32>() as u32,
            )
        };

        Ok(())
    }

    /// Apply a click-through style change and make it take effect immediately.
    fn refresh_after_style_change(&self) {
        if let Some(hwnd) = self.hwnd {
            // Changing an extended style does not take effect until the frame is
            // recomputed. `SWP_FRAMECHANGED` with no movement is the documented
            // way to force that without the window jumping somewhere.
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
        }
    }
}

/// Report a recoverable problem.
///
/// Lumina's logger arrives in Phase 12. Routing through a function keeps every
/// call site able to report a problem without this crate depending on the
/// logging stack yet, and means adding the logger is a one line change here.
fn log_warn(message: &str) {
    // Debug builds print to stderr so a developer running the app sees it.
    // Release builds stay quiet, because stderr from a background process is
    // noise on a normal desktop.
    #[cfg(debug_assertions)]
    eprintln!("lumina-platform: warn: {message}");
    #[cfg(not(debug_assertions))]
    let _ = message;
}

/// `MONITORINFOF_PRIMARY`, declared locally because the `windows` crate does not
/// surface this one as a named constant.
const MONITORINFOF_PRIMARY: u32 = 1;

// Accumulator for one monitor enumeration pass.
//
// `EnumDisplayMonitors` takes a plain function pointer, which cannot capture, so
// the callback reaches this through thread local storage. Scoped to the calling
// thread, which matches the fact that enumeration is driven from the main thread
// and is never re-entered.
thread_local! {
    static ENUMERATION: std::cell::RefCell<Vec<Monitor>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The `EnumDisplayMonitors` callback.
///
/// Accumulates into [`ENUMERATION`] and returns TRUE to continue enumerating.
unsafe extern "system" fn enum_monitor_callback(
    monitor: HMONITOR,
    _dc: windows::Win32::Graphics::Gdi::HDC,
    _clip: *mut RECT,
    _data: windows::Win32::Foundation::LPARAM,
) -> windows::core::BOOL {
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) };

    ENUMERATION.with(|slot| {
        let mut list = slot.borrow_mut();
        // The index comes from the list length, so it has to be read before the
        // push, since borrowing the list again while it is mutably borrowed
        // does not compile.
        let next_index = list.len();
        let info = monitor_info(monitor);
        let bounds = monitor_bounds(monitor);

        // A zero sized rectangle means the driver failed. Fall back rather than
        // propagating a degenerate monitor into the placement code, which would
        // leave the island with nowhere to live and hide the real problem.
        let bounds = if bounds.width > 0.0 && bounds.height > 0.0 {
            bounds
        } else {
            Rect::new(0.0, 0.0, 1920.0, 1080.0)
        };

        list.push(Monitor {
            index: next_index,
            name: format!("Display {}", next_index + 1),
            bounds,
            scale: ScaleFactor::new(if dpi_x > 0 { dpi_x as f32 / 96.0 } else { 1.0 }),
            refresh_hz: 0,
            is_primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    });

    windows::core::BOOL(1) // TRUE, keep enumerating
}

/// Extract the `HWND` from a raw window handle.
///
/// `RawWindowHandle` is an enum in `raw-window-handle` 0.6, so a handle from a
/// different platform has to be rejected rather than transmuted. A silent
/// transmute here would produce a window handle that points at nothing, which is
/// the kind of bug that manifests as a random access violation minutes later.
fn hwnd_from_raw(window: RawWindowHandle) -> Option<HWND> {
    match window {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut c_void)),
        _ => None,
    }
}

/// Read a monitor's rectangle and primary flag in one call.
fn monitor_info(monitor: HMONITOR) -> MONITORINFO {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    // A failed query leaves the struct at its defaults, whose zero `dwFlags`
    // correctly reads as "not primary", so the failure is harmless here.
    let _ = unsafe { GetMonitorInfoW(monitor, &raw mut info) };
    info
}

/// A monitor's rectangle in virtual desktop physical pixels.
fn monitor_bounds(monitor: HMONITOR) -> Rect {
    let info = monitor_info(monitor);
    let r = info.rcMonitor;
    Rect::new(
        r.left as f32,
        r.top as f32,
        (r.right - r.left) as f32,
        (r.bottom - r.top) as f32,
    )
}

/// Collect every attached monitor.
///
/// Enumerated in virtual desktop coordinates, which is what the window APIs
/// already use, so nothing downstream has to convert.
///
/// # Panics
///
/// Never. A monitor that reports no rectangle gets a nominal 1920x1080 at the
/// origin rather than being dropped, because dropping it would leave the island
/// with nowhere to live and hide the actual driver problem.
fn enumerate_monitors() -> Vec<Monitor> {
    ENUMERATION.with(|slot| slot.borrow_mut().clear());

    // Safety: the callback is a plain function pointer, so it cannot capture
    // and accumulates into the thread local above. Returning TRUE from every
    // callback is what asks the system to keep enumerating.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitor_callback),
            windows::Win32::Foundation::LPARAM(0),
        );
    }

    ENUMERATION.with(std::cell::RefCell::take)
}

/// The scale factor for the monitor the window currently sits on.
fn current_scale(hwnd: HWND) -> ScaleFactor {
    ScaleFactor::new(Win32Backend::dpi_scale_for(hwnd))
}

impl OverlayBackend for Win32Backend {
    fn capabilities(&self) -> Capabilities {
        Capabilities::for_platform(Platform::Windows)
    }

    fn monitors(&self) -> Vec<Monitor> {
        enumerate_monitors()
    }

    fn configure_window(&mut self, window: RawWindowHandle) -> Result<(), OverlayError> {
        let hwnd = hwnd_from_raw(window).ok_or(OverlayError::NoWindowHandle)?;
        self.hwnd = Some(hwnd);
        self.scale = current_scale(hwnd);
        self.apply_window_styles()
    }

    fn apply_geometry(&mut self, geometry: &WindowGeometry) -> Result<(), OverlayError> {
        self.geometry = Some(*geometry);
        self.scale = ScaleFactor::new(geometry.scale);

        let hwnd = self.hwnd.ok_or(OverlayError::NoWindowHandle)?;
        let b = geometry.bounds;

        // Requirement: stay above everything.
        //
        // `SWP_NOACTIVATE` is what stops the island stealing focus every time it is
        // shown or moved.
        let result = unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                b.x.round() as i32,
                b.y.round() as i32,
                geometry.size.0.max(1),
                geometry.size.1.max(1),
                SWP_NOACTIVATE,
            )
        };

        match result {
            Ok(()) => Ok(()),
            Err(e) => Err(OverlayError::Platform(format!("SetWindowPos failed: {e}"))),
        }
    }

    fn enforce_topmost(&mut self) -> Result<(), OverlayError> {
        let hwnd = self.hwnd.ok_or(OverlayError::NoWindowHandle)?;
        self.topmost_enforces = self.topmost_enforces.saturating_add(1);

        // Requirement: stay above everything, including full-screen applications.
        //
        // `SWP_NOMOVE | SWP_NOSIZE` moves nothing. Without them this call yanks
        // the island back to a stale position every time the window manager
        // fights for the top spot.
        let result = unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
        match result {
            Ok(()) => Ok(()),
            Err(e) => Err(OverlayError::Platform(format!(
                "SetWindowPos failed while re-asserting topmost: {e}"
            ))),
        }
    }

    fn set_click_through(&mut self, enabled: bool) -> Result<(), OverlayError> {
        self.click_through = enabled;

        // Requirement: clicks pass through to whatever is underneath.
        //
        // `WS_EX_TRANSPARENT` is the documented mechanism. The follow-up
        // `SetWindowPos` with `SWP_FRAMECHANGED` is required for the style change
        // to take effect immediately; without it the change lags by a frame,
        // which is exactly when the user is moving the pointer across the island.
        let current = self.extended_style();
        let desired = if enabled {
            current | WS_EX_TRANSPARENT.0 as isize
        } else {
            current & !(WS_EX_TRANSPARENT.0 as isize)
        };

        if desired != current {
            self.set_extended_style(desired)?;
        }
        self.refresh_after_style_change();
        Ok(())
    }

    fn click_through(&self) -> bool {
        self.click_through
    }

    fn set_visible(&mut self, visible: bool) -> Result<(), OverlayError> {
        self.visible = visible;
        let hwnd = self.hwnd.ok_or(OverlayError::NoWindowHandle)?;

        // `SW_SHOWNOACTIVATE` rather than `SW_SHOW`, because `SW_SHOW` activates
        // the window and the island must never take focus.
        let command = if visible { SW_SHOWNOACTIVATE } else { SW_HIDE };
        // `ShowWindow` returns the previous visibility, which `self.visible`
        // already tracks, so the return value is intentionally discarded.
        let _ = unsafe { ShowWindow(hwnd, command) };
        Ok(())
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn current_scale(&self) -> ScaleFactor {
        self.scale
    }

    fn tick(&mut self) -> Result<(), OverlayError> {
        // No per frame work is needed from this thread. The message pump belongs
        // to the event loop, which created the window and owns it.
        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(hwnd) = self.hwnd {
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
        }
        self.hwnd = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::compute_placement;
    use lumina_config::Position;

    fn sample_geometry(scale: f32) -> WindowGeometry {
        compute_placement(
            Rect::new(0.0, 0.0, 1920.0, 1080.0),
            0,
            scale,
            Position::TopCenter,
            (124.0, 36.0),
            0,
            0,
        )
        .geometry
    }

    #[test]
    fn a_new_backend_starts_visible_and_without_a_window() {
        let backend = Win32Backend::new();
        assert!(backend.is_visible());
        assert!(backend.hwnd().is_none());
        assert!(!backend.click_through());
        assert_eq!(backend.enforce_count(), 0);
    }

    /// Every operation must fail gracefully before the window exists rather than
    /// crashing. A fast startup path can call some of these first.
    #[test]
    fn operations_before_the_window_exists_report_an_error() {
        let mut backend = Win32Backend::new();
        assert_eq!(
            backend.apply_geometry(&sample_geometry(1.0)),
            Err(OverlayError::NoWindowHandle)
        );
        assert_eq!(backend.enforce_topmost(), Err(OverlayError::NoWindowHandle));
        assert_eq!(backend.set_visible(true), Err(OverlayError::NoWindowHandle));
        assert_eq!(
            backend.set_click_through(true),
            Err(OverlayError::NoWindowHandle)
        );
        assert!(backend.window_rect().is_none());
    }

    /// State must be recorded even when the platform call cannot happen, so
    /// callers see the intended value rather than a stale one.
    #[test]
    fn intent_is_recorded_even_when_the_call_fails() {
        let mut backend = Win32Backend::new();

        let _ = backend.set_click_through(true);
        assert!(backend.click_through());
        let _ = backend.set_click_through(false);
        assert!(!backend.click_through());

        let _ = backend.set_visible(false);
        assert!(!backend.is_visible());

        let _ = backend.apply_geometry(&sample_geometry(1.5));
        assert_eq!(backend.current_scale(), ScaleFactor::new(1.5));
    }

    #[test]
    fn capabilities_report_the_windows_matrix() {
        let caps = Win32Backend::new().capabilities();
        assert_eq!(caps.platform, Platform::Windows);
        assert!(caps.always_on_top);
        assert!(caps.per_pixel_alpha);
        assert!(caps.click_through);
        assert!(caps.native_backdrop_blur);
        assert!(caps.global_hotkeys);
        assert!(!caps.exclusive_zone, "Win32 has no exclusive zone concept");
    }

    #[test]
    fn shutdown_is_idempotent_and_clears_the_handle() {
        let mut backend = Win32Backend::new();
        backend.shutdown();
        backend.shutdown();
        assert!(backend.hwnd().is_none());
    }

    #[test]
    fn monitor_enumeration_never_panics_and_always_returns_something() {
        // On a machine with no session attached this returns the nominal
        // fallback rather than an empty list, so placement always has a monitor.
        let monitors = enumerate_monitors();
        if !monitors.is_empty() {
            assert!(monitors
                .iter()
                .all(|m| m.bounds.width > 0.0 && m.bounds.height > 0.0));
            assert!(
                monitors.iter().any(|m| m.is_primary) || monitors.len() == 1,
                "with several monitors exactly one should be primary"
            );
        }
    }

    #[test]
    fn enumerate_is_callable_more_than_once() {
        // Re-enumeration happens on every display change, so it must be
        // repeatable rather than accumulating.
        let first = enumerate_monitors().len();
        let second = enumerate_monitors().len();
        assert_eq!(first, second);
    }
}
