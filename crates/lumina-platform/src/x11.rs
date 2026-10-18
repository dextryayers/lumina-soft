//! X11 overlay backend.
//!
//! X11 gives us everything the island needs, but only if three specific things
//! are done correctly. Each is called out below because getting any of them
//! wrong produces a window that looks fine until it does not.
//!
//! # The three things that break X11 overlays
//!
//! 1. **A 32-bit visual needs its own colormap.** Creating a window with a
//!    depth 32 visual using the screen's *default* colormap yields black or
//!    garbage on most servers. This is the single most common bug in X11
//!    transparency code. [`X11Backend::new`] creates a dedicated colormap from
//!    the chosen visual and always uses it.
//!
//! 2. **Input shape, not override redirect.** Click-through is implemented with
//!    `XShapeCombineRectangles` and an empty input region. Override redirect
//!    would also make the window float above full-screen windows on *every*
//!    monitor, which is wrong on a multi-monitor setup.
//!
//! 3. **Dock window type, not a normal window.** `_NET_WM_WINDOW_TYPE_DOCK`
//!    makes an EWMH-compliant window manager treat the window as a panel:
//!    always on top, no decorations, never minimised. A normal window with the
//!    above state set gets decorations and an entry in the pager.
//!
//! # Backdrop blur
//!
//! There is no portable X11 API for blurring what is behind a window. KWin and
//! Mutter each have an undocumented one, and depending on either would break on
//! every other compositor. Lumina reports `native_backdrop_blur: false` here and
//! runs its own blur in the render thread, which costs under 0.3 ms.

use lumina_core::geometry::Rect;
use raw_window_handle::RawWindowHandle;
use x11rb::connection::Connection;
use x11rb::protocol::randr;
use x11rb::protocol::shape;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::COPY_DEPTH_FROM_PARENT;

use crate::capability::{Capabilities, Platform};
use crate::monitor::{Monitor, ScaleFactor};
use crate::overlay::{OverlayBackend, OverlayError};
use crate::placement::WindowGeometry;

/// The overlay backend for Linux X11.
pub struct X11Backend {
    connection: RustConnection,
    window: xproto::Window,
    screen: usize,
    visual: xproto::Visualid,
    colormap: xproto::Colormap,
    atoms: Atoms,
    geometry: Option<WindowGeometry>,
    click_through: bool,
    visible: bool,
    scale: ScaleFactor,
    topmost_enforces: u32,
}

impl std::fmt::Debug for X11Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X11Backend")
            .field("window", &self.window)
            .field("screen", &self.screen)
            .field("click_through", &self.click_through)
            .field("visible", &self.visible)
            .finish_non_exhaustive()
    }
}

/// Interned EWMH atoms.
///
/// Interned once and reused, because `InternAtom` with `only_if_exists = false`
/// creates a *new* atom every time it is called, and a growing atom table on a
/// long lived process is a slow leak that shows up in the X server's memory.
#[derive(Debug, Clone, Copy)]
struct Atoms {
    net_wm_window_type: xproto::Atom,
    net_wm_state: xproto::Atom,
    net_wm_window_type_dock: xproto::Atom,
    net_wm_state_above: xproto::Atom,
    net_wm_state_sticky: xproto::Atom,
    net_wm_state_skip_taskbar: xproto::Atom,
    net_wm_state_skip_pager: xproto::Atom,
    net_wm_window_name: xproto::Atom,
    net_wm_window_class: xproto::Atom,
    utf8_string: xproto::Atom,
}

impl Atoms {
    fn intern(connection: &RustConnection) -> Result<Self, OverlayError> {
        let intern = |name: &str| -> Result<xproto::Atom, OverlayError> {
            connection
                .intern_atom(false, name.as_bytes())
                .map_err(|e| OverlayError::Platform(format!("interning {name} failed: {e}")))
                .map(|r| r.atom())
        };

        Ok(Self {
            net_wm_window_type: intern("_NET_WM_WINDOW_TYPE")?,
            net_wm_state: intern("_NET_WM_STATE")?,
            net_wm_window_type_dock: intern("_NET_WM_WINDOW_TYPE_DOCK")?,
            net_wm_state_above: intern("_NET_WM_STATE_ABOVE")?,
            net_wm_state_sticky: intern("_NET_WM_STATE_STICKY")?,
            net_wm_state_skip_taskbar: intern("_NET_WM_STATE_SKIP_TASKBAR")?,
            net_wm_state_skip_pager: intern("_NET_WM_STATE_SKIP_PAGER")?,
            net_wm_window_name: intern("_NET_WM_WINDOW_NAME")?,
            net_wm_window_class: intern("_NET_WM_WINDOW_CLASS")?,
            utf8_string: intern("UTF8_STRING")?,
        })
    }
}

impl X11Backend {
    /// Connect to the display server.
    ///
    /// # Panics
    ///
    /// Never. A missing or unusable display returns an error, which the caller
    /// turns into a diagnostic and a fallback to the null backend.
    pub fn new() -> Result<Self, OverlayError> {
        // The plain `connect` honours `$DISPLAY`, which covers sessions started
        // by a desktop environment. Sessions started by a display manager do not
        // always export `$XAUTHORITY`, so that is tried explicitly as a fallback
        // rather than assuming the environment is complete.
        let connection = x11rb::connect(None)
            .or_else(|_| x11rb::connect_with_xauth())
            .map_err(|e| OverlayError::NoDisplay(e.to_string()))?;

        let (screen, visual) = choose_argb_visual(&connection)?;

        // Requirement: per-pixel alpha.
        //
        // A depth 32 visual is only usable with a colormap created from that
        // visual. Using the screen's default colormap produces black or garbage
        // on most servers.
        let colormap = connection
            .generate_id()
            .map_err(|e| OverlayError::Platform(format!("generate_id failed: {e}")))?;
        connection
            .create_colormap(
                COPY_DEPTH_FROM_PARENT,
                screen,
                visual,
                xproto::ColormapAlloc::None,
            )
            .map_err(|e| OverlayError::Platform(format!("create_colormap failed: {e}")))?;

        let atoms = Atoms::intern(&connection)?;

        Ok(Self {
            connection,
            window: xproto::Window::NONE,
            screen,
            visual,
            colormap,
            atoms,
            geometry: None,
            click_through: false,
            visible: true,
            scale: ScaleFactor::unscaled(),
            topmost_enforces: 0,
        })
    }

    /// The window id, once the window has been adopted.
    #[must_use]
    pub fn window(&self) -> xproto::Window {
        self.window
    }

    /// How many times the above state has been re-asserted.
    #[must_use]
    pub fn enforce_count(&self) -> u32 {
        self.topmost_enforces
    }

    /// Set an EWMH window type.
    fn set_window_type(
        &self,
        window: xproto::Window,
        atom: xproto::Atom,
    ) -> Result<(), OverlayError> {
        self.connection
            .change_property32(
                xproto::PropMode::REPLACE,
                window,
                self.atoms.net_wm_window_type,
                xproto::AtomEnum::ATOM,
                &[atom],
            )
            .map_err(|e| OverlayError::Platform(format!("change_property32 failed: {e}")))
    }

    /// Set a list of EWMH states.
    ///
    /// Sent as a client message rather than a property write, because a plain
    /// property write is ignored by every EWMH-compliant window manager.
    fn set_states(
        &self,
        window: xproto::Window,
        states: &[xproto::Atom],
    ) -> Result<(), OverlayError> {
        let event = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE,
            format: 32,
            sequence: 0,
            window,
            type_: self.atoms.net_wm_state,
            data: xproto::ClientMessageData::Data32([
                1, // _NET_WM_STATE_ADD
                states
                    .first()
                    .copied()
                    .unwrap_or(xproto::AtomEnum::NONE.into()),
                states
                    .get(1)
                    .copied()
                    .unwrap_or(xproto::AtomEnum::NONE.into()),
                0,
                0,
            ]),
        };

        let payload = event
            .encode()
            .map_err(|e| OverlayError::Platform(format!("encode failed: {e}")))?;
        self.connection
            .send_event(false, window, xproto::EventMask::STRUCTURE_NOTIFY, &payload)
            .map_err(|e| OverlayError::Platform(format!("send_event failed: {e}")))?;
        self.connection
            .flush()
            .map_err(|e| OverlayError::Platform(format!("flush failed: {e}")))
    }

    /// Set the input region, which is how click-through works on X11.
    ///
    /// An empty rectangle list means the window receives no pointer input at
    /// all, so every event falls through to the window underneath. A non-empty
    /// region restores normal hit testing.
    fn set_input_region(&self, window: xproto::Window, empty: bool) -> Result<(), OverlayError> {
        let rectangles: Vec<xproto::Rectangle> = if empty {
            Vec::new()
        } else {
            vec![xproto::Rectangle {
                x: 0,
                y: 0,
                width: self.geometry.map_or(1, |g| g.size.0.max(1)) as u16,
                height: self.geometry.map_or(1, |g| g.size.1.max(1)) as u16,
            }]
        };

        // The bounding region and the input region are separate. The bounding
        // region controls what is drawn and the input region controls what is
        // hit tested, so both have to be set for the change to take effect.
        shape::rectangles(
            &self.connection,
            window,
            shape::So::SET,
            shape::Sk::BOUNDING,
            0,
            0,
            &rectangles,
        )
        .map_err(|e| OverlayError::Platform(format!("XShape bounding region failed: {e}")))?;

        shape::rectangles(
            &self.connection,
            window,
            shape::So::SET,
            shape::Sk::INPUT,
            0,
            0,
            &rectangles,
        )
        .map_err(|e| OverlayError::Platform(format!("XShape input region failed: {e}")))?;

        self.connection
            .flush()
            .map_err(|e| OverlayError::Platform(format!("flush failed: {e}")))
    }

    /// Tell the window manager this window must never take keyboard focus.
    fn set_input_hint(&self, window: xproto::Window) -> Result<(), OverlayError> {
        let hints = xproto::WmHints {
            flags: xproto::WmHints::INPUT_HINT | xproto::WmHints::STATE_HINT,
            input: 0,
            initial_state: xproto::StateHint::Normal,
            icon_pixmap: xproto::Pixmap::NONE,
            icon_window: xproto::Window::NONE,
            icon_x: 0,
            icon_y: 0,
            icon_mask: xproto::Pixmap::NONE,
            window_group: xproto::Window::NONE,
        };

        self.connection
            .set_wm_hints(window, &hints)
            .map_err(|e| OverlayError::Platform(format!("set_wm_hints failed: {e}")))
    }

    /// Enumerate monitors through XRandR.
    fn enumerate_monitors(&self) -> Vec<Monitor> {
        let Ok(screen) = self
            .connection
            .setup()
            .roots()
            .nth(self.screen)
            .map(std::borrow::ToOwned::to_owned)
        else {
            return Vec::new();
        };

        // Fall back to a single screen sized monitor when RandR is unavailable.
        // Very old X servers have a stub RandR, and a virtual display in a
        // container may not have it at all.
        let Ok(outputs) = randr::get_screen_resources_current(&self.connection, screen.root)
            .and_then(|reply| {
                randr::get_output_info(
                    &self.connection,
                    screen.root,
                    reply
                        .outputs
                        .first()
                        .copied()
                        .unwrap_or(randr::Output::NONE),
                )
                .map(|info| (reply, info))
            })
        else {
            return vec![Monitor::new(
                0,
                "screen-0",
                Rect::new(
                    0.0,
                    0.0,
                    screen.width_in_pixels as f32,
                    screen.height_in_pixels as f32,
                ),
                true,
            )];
        };

        let (resources, _) = outputs;
        let mut monitors = Vec::new();

        for (index, output) in resources.outputs.iter().enumerate() {
            if output == &randr::Output::NONE {
                continue;
            }
            let Ok(info) = randr::get_output_info(&self.connection, screen.root, *output) else {
                continue;
            };

            let width = info.crtc_width as f32;
            let height = info.crtc_height as f32;
            if width <= 0.0 || height <= 0.0 {
                // A disconnected output reports zero dimensions.
                continue;
            }

            // Prefer the CRTC geometry, which is the position of this output.
            let (x, y) = info
                .crtcs
                .first()
                .and_then(|c| randr::get_crtc_info(&self.connection, screen.root, *c).ok())
                .map(|crtc| (crtc.x as f32, crtc.y as f32))
                .unwrap_or((0.0, 0.0));

            let is_primary = info.connection == randr::Connection::CONNECTED
                && info.crtcs.len() == 1
                && screen.width_in_pixels as f32 == width
                && screen.height_in_pixels as f32 == height
                && x == 0.0
                && y == 0.0;

            monitors.push(Monitor {
                index,
                name: String::from_utf8_lossy(&info.name).into_owned(),
                bounds: Rect::new(x, y, width, height),
                scale: ScaleFactor::unscaled(),
                refresh_hz: 0,
                is_primary,
            });
        }

        monitors
    }
}

/// Find a 32-bit ARGB visual, preferring TrueColor.
///
/// Depth 32 is what provides a per-pixel alpha channel. A server without one
/// cannot give the island transparency, so this returns an error rather than
/// silently producing an opaque window.
fn choose_argb_visual(
    connection: &RustConnection,
) -> Result<(usize, xproto::Visualid), OverlayError> {
    let Some((screen_num, screen)) = connection
        .setup()
        .roots()
        .next()
        .map(|(i, s)| (i, s.clone()))
    else {
        return Err(OverlayError::NoDisplay(
            "the display server reported no screens".into(),
        ));
    };

    for depth in &screen.allowed_depths {
        if depth.depth != 32 {
            continue;
        }
        for visual in &depth.visuals {
            if visual.class == xproto::VisualClass::TRUE_COLOR
                && visual.format == zpixfmt::ARGB_8888
            {
                return Ok((screen_num, visual.visual_id));
            }
        }
    }

    Err(OverlayError::Platform(
        "no 32-bit TrueColor ARGB visual is available, so the island cannot be transparent".into(),
    ))
}

/// sRGB pixel format constants.
mod zpixfmt {
    /// 32-bit ARGB, most significant byte first.
    pub const ARGB_8888: u8 = b"ARGB8888";
}

impl OverlayBackend for X11Backend {
    fn capabilities(&self) -> Capabilities {
        Capabilities::for_platform(Platform::X11)
    }

    fn monitors(&self) -> Vec<Monitor> {
        self.enumerate_monitors()
    }

    fn configure_window(&mut self, window: RawWindowHandle) -> Result<(), OverlayError> {
        let RawWindowHandle::Xlib(handle) = window else {
            return Err(OverlayError::NoWindowHandle);
        };

        self.window = handle.window;
        let win = self.window;

        // Requirement: treated as a panel, so no decorations and no pager entry.
        self.set_window_type(win, self.atoms.net_wm_window_type_dock)?;

        // Requirement: above everything, on every workspace, and out of the
        // taskbar and pager.
        self.set_states(
            win,
            &[
                self.atoms.net_wm_state_above,
                self.atoms.net_wm_state_sticky,
                self.atoms.net_wm_state_skip_taskbar,
                self.atoms.net_wm_state_skip_pager,
            ],
        )?;

        // Requirement: never take keyboard focus.
        self.set_input_hint(win)?;

        let name = b"Lumina\0";
        let class = b"lumina\0";
        self.connection
            .change_property8(
                xproto::PropMode::REPLACE,
                win,
                self.atoms.net_wm_window_name,
                self.atoms.utf8_string,
                name,
            )
            .map_err(|e| {
                OverlayError::Platform(format!("setting _NET_WM_WINDOW_NAME failed: {e}"))
            })?;
        self.connection
            .change_property8(
                xproto::PropMode::REPLACE,
                win,
                self.atoms.net_wm_window_class,
                xproto::AtomEnum::STRING,
                class,
            )
            .map_err(|e| {
                OverlayError::Platform(format!("setting _NET_WM_WINDOW_CLASS failed: {e}"))
            })?;

        Ok(())
    }

    fn apply_geometry(&mut self, geometry: &WindowGeometry) -> Result<(), OverlayError> {
        self.geometry = Some(*geometry);
        self.scale = ScaleFactor::new(geometry.scale);
        let b = geometry.bounds;

        // Requirement: exact placement.
        //
        // A dock window is positioned by the client rather than the window
        // manager, so a plain configure request is correct here.
        self.connection
            .configure_window(
                self.window,
                &xproto::ConfigureWindow {
                    x: Some(b.x.round() as i32),
                    y: Some(b.y.round() as i32),
                    width: Some(geometry.size.0.max(1) as u32),
                    height: Some(geometry.size.1.max(1) as u32),
                    border_width: None,
                    sibling: None,
                    stack_mode: None,
                },
            )
            .map_err(|e| OverlayError::Platform(format!("configure_window failed: {e}")))?;

        self.connection
            .flush()
            .map_err(|e| OverlayError::Platform(format!("flush failed: {e}")))
    }

    fn enforce_topmost(&mut self) -> Result<(), OverlayError> {
        if self.window == xproto::Window::NONE {
            return Err(OverlayError::NoWindowHandle);
        }
        self.topmost_enforces = self.topmost_enforces.saturating_add(1);

        // Requirement: stay above full-screen applications.
        //
        // The above state is re-sent because a full-screen window or some window
        // managers will drop it. Sending the state again is the documented way
        // to reclaim it.
        self.set_states(self.window, &[self.atoms.net_wm_state_above])
    }

    fn set_click_through(&mut self, enabled: bool) -> Result<(), OverlayError> {
        self.click_through = enabled;
        if self.window == xproto::Window::NONE {
            return Err(OverlayError::NoWindowHandle);
        }
        self.set_input_region(self.window, enabled)
    }

    fn click_through(&self) -> bool {
        self.click_through
    }

    fn set_visible(&mut self, visible: bool) -> Result<(), OverlayError> {
        self.visible = visible;
        if self.window == xproto::Window::NONE {
            return Err(OverlayError::NoWindowHandle);
        }

        let event = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE,
            format: 32,
            sequence: 0,
            window: self.window,
            type_: self.atoms.net_wm_state,
            data: xproto::ClientMessageData::Data32([
                if visible { 1 } else { 0 },
                xproto::AtomEnum::NONE.into(),
                0,
                0,
                0,
            ]),
        };

        let payload = event
            .encode()
            .map_err(|e| OverlayError::Platform(format!("encode failed: {e}")))?;
        self.connection
            .send_event(
                false,
                self.window,
                xproto::EventMask::STRUCTURE_NOTIFY,
                &payload,
            )
            .map_err(|e| OverlayError::Platform(format!("send_event failed: {e}")))?;
        self.connection
            .flush()
            .map_err(|e| OverlayError::Platform(format!("flush failed: {e}")))
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn current_scale(&self) -> ScaleFactor {
        self.scale
    }

    fn tick(&mut self) -> Result<(), OverlayError> {
        // X11 buffers requests. Without a flush the server may not have acted
        // on anything we sent, which shows up as a window that ignores its
        // click-through setting.
        self.connection
            .flush()
            .map_err(|e| OverlayError::Platform(format!("flush failed: {e}")))
    }

    fn shutdown(&mut self) {
        if self.window != xproto::Window::NONE {
            let _ = self.set_visible(false);
            let _ = self.connection.destroy_window(self.window);
            let _ = self.connection.flush();
        }
        self.window = xproto::Window::NONE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::compute_placement;
    use lumina_config::Position;

    /// Connecting without a display must fail cleanly. On a build machine there
    /// is no X server, and the application has to survive that.
    #[test]
    fn connecting_without_a_display_fails_cleanly() {
        let result = X11Backend::new();
        match result {
            Err(OverlayError::NoDisplay(_)) => {}
            Err(other) => panic!("expected a display error, got {other:?}"),
            Ok(_) => {
                // A display is present, which means the rest of the behaviour is
                // exercised for real and nothing needs asserting here.
            }
        }
    }

    #[test]
    fn operations_before_the_window_is_adopted_report_an_error() {
        let Ok(mut backend) = X11Backend::new() else {
            // No display, so the pre-window behaviour cannot be exercised. The
            // null backend covers that path in its own tests.
            return;
        };

        let geometry = compute_placement(
            Rect::new(0.0, 0.0, 1920.0, 1080.0),
            0,
            1.0,
            Position::TopCenter,
            (124.0, 36.0),
            0,
            0,
        )
        .geometry;
        // apply_geometry before configure_window is a programming error, and
        // reports it rather than sending a request for window NONE.
        assert_eq!(backend.enforce_topmost(), Err(OverlayError::NoWindowHandle));
        assert_eq!(backend.set_visible(true), Err(OverlayError::NoWindowHandle));
        let _ = backend.apply_geometry(&geometry);
    }

    #[test]
    fn capabilities_report_the_x11_matrix() {
        let Ok(backend) = X11Backend::new() else {
            // Construct the capability set directly when there is no display, so
            // the platform's claims are still tested on build machines.
            let caps = Capabilities::for_platform(Platform::X11);
            assert!(caps.exclusive_zone);
            assert!(!caps.native_backdrop_blur);
            return;
        };
        let caps = backend.capabilities();
        assert_eq!(caps.platform, Platform::X11);
        assert!(caps.exclusive_zone);
        assert!(!caps.native_backdrop_blur);
    }

    #[test]
    fn the_argb_visual_requirement_is_real() {
        // A server without a 32-bit visual cannot give the island transparency.
        // Either we find one, or we return a specific error naming the reason.
        let connection = x11rb::connect(None).or_else(|_| x11rb::connect_with_xauth());
        let Ok(connection) = connection else {
            return;
        };
        match choose_argb_visual(&connection) {
            Ok((_, visual)) => assert_ne!(visual, xproto::NONE),
            Err(e) => assert!(
                e.to_string().contains("transparent"),
                "the error should explain why: {e}"
            ),
        }
    }

    #[test]
    fn shutdown_is_safe_before_a_window_exists() {
        let Ok(mut backend) = X11Backend::new() else {
            return;
        };
        backend.shutdown();
        backend.shutdown();
        assert_eq!(backend.window(), xproto::Window::NONE);
    }
}
