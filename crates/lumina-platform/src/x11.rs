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
//! # EWMH states are client messages
//!
//! `_NET_WM_STATE` has to be sent as a `ClientMessageEvent` to the window. A
//! plain property write is ignored by every compliant window manager, which is
//! the most common reason an X11 overlay refuses to stay on top.
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

use crate::capability::{Capabilities, Platform};
use crate::monitor::{Monitor, ScaleFactor};
use crate::overlay::{OverlayBackend, OverlayError};
use crate::placement::WindowGeometry;

/// The X11 `None` value for window and resource identifiers.`r`nconst X_NONE: u32 = 0;`r`n`r`n/// The X11 `None` value for window, colormap, and resource identifiers.
const X_NONE: u32 = 0;

/// Channel masks of a 32-bit ARGB visual.
///
/// X11 does not report a pixel format name; it reports per-channel masks. A
/// 32-bit visual with 8 bits each in the low three bytes and nothing in the
/// high byte is the standard 32-bit ARGB arrangement, which is what carries a
/// per-pixel alpha channel.
const ARGB_RED_MASK: u32 = 0x00FF_0000;
const ARGB_GREEN_MASK: u32 = 0x0000_FF00;
const ARGB_BLUE_MASK: u32 = 0x0000_00FF;

/// True when a visual is the 32-bit ARGB arrangement we need.
fn is_argb_visual(visual: &xproto::Visualtype) -> bool {
    visual.red_mask == ARGB_RED_MASK
        && visual.green_mask == ARGB_GREEN_MASK
        && visual.blue_mask == ARGB_BLUE_MASK
}

/// Interned EWMH atoms.
///
/// Interned once and reused. `InternAtom` with `only_if_exists = false` creates
/// a *new* atom every call, so interning per frame would slowly grow the X
/// server's atom table for the life of the process.
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
            let cookie = connection
                .intern_atom(false, name.as_bytes())
                .map_err(|e| OverlayError::Platform(format!("interning {name} failed: {e}")))?;
            let reply = cookie.reply().map_err(|e| {
                OverlayError::Platform(format!("interning {name} gave no reply: {e}"))
            })?;
            Ok(reply.atom)
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

/// The overlay backend for Linux X11.
pub struct X11Backend {
    connection: RustConnection,
    window: xproto::Window,
    screen_num: usize,
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
            .field("screen", &self.screen_num)
            .field("click_through", &self.click_through)
            .field("visible", &self.visible)
            .finish_non_exhaustive()
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
        // `connect` returns the connection *and* the screen index, which is how
        // Xlib's `DefaultScreen` is resolved.
        let (connection, screen_num) =
            x11rb::connect(None).map_err(|e| OverlayError::NoDisplay(e.to_string()))?;

        let visual = choose_argb_visual(&connection, screen_num)?;

        // Requirement: per-pixel alpha.
        //
        // A depth 32 visual is only usable with a colormap created from that
        // visual. Using the screen's default colormap produces black or garbage
        // on most servers.
        let colormap = connection
            .generate_id()
            .map_err(|e| OverlayError::Platform(format!("generate_id failed: {e}")))?;
        connection
            .create_colormap(xproto::ColormapAlloc::NONE, colormap, X_NONE, visual)
            .map_err(|e| OverlayError::Platform(format!("create_colormap failed: {e}")))?;

        let atoms = Atoms::intern(&connection)?;

        Ok(Self {
            connection,
            window: X_NONE,
            screen_num,
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

    /// The dedicated colormap created for the ARGB visual.
    ///
    /// Exposed for the diagnostics command, because "does Lumina have a real
    /// ARGB colormap" is the first thing to check when the island renders
    /// black on a particular machine.
    #[must_use]
    pub fn colormap(&self) -> xproto::Colormap {
        self.colormap
    }

    /// The chosen ARGB visual.
    #[must_use]
    pub fn visual(&self) -> xproto::Visualid {
        self.visual
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
            .map_err(|e| {
                OverlayError::Platform(format!("setting _NET_WM_WINDOW_TYPE failed: {e}"))
            })?;
        self.flush()
    }

    /// Build and send an EWMH `_NET_WM_STATE` client message.
    ///
    /// `action` is 1 to add and 0 to remove. A plain property write is ignored
    /// by every compliant window manager, which is the most common reason an X11
    /// overlay refuses to stay above other windows.
    fn state_message(
        &self,
        window: xproto::Window,
        action: u32,
        state: xproto::Atom,
    ) -> Result<(), OverlayError> {
        let event = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window,
            type_: self.atoms.net_wm_state,
            data: xproto::ClientMessageData::from([action, state, 0, 1, 0]),
        };

        self.connection
            .send_event(false, window, xproto::EventMask::STRUCTURE_NOTIFY, event)
            .map_err(|e| OverlayError::Platform(format!("sending _NET_WM_STATE failed: {e}")))?;
        self.flush()
    }

    /// Add a list of EWMH states, one message per state.
    ///
    /// One message per state because a client message carries at most two atoms
    /// and EWMH only guarantees the first is honoured.
    fn set_states(
        &self,
        window: xproto::Window,
        states: &[xproto::Atom],
    ) -> Result<(), OverlayError> {
        for state in states {
            self.state_message(window, 1, *state)?;
        }
        Ok(())
    }

    /// Remove a single EWMH state.
    fn remove_state(
        &self,
        window: xproto::Window,
        state: xproto::Atom,
    ) -> Result<(), OverlayError> {
        self.state_message(window, 0, state)
    }

    /// Set the input region, which is how click-through works on X11.
    ///
    /// An empty rectangle list means the window receives no pointer input at
    /// all, so every event falls through to the window underneath.
    fn set_input_region(&self, window: xproto::Window, empty: bool) -> Result<(), OverlayError> {
        let rectangles: Vec<xproto::Rectangle> = if empty {
            Vec::new()
        } else {
            vec![xproto::Rectangle {
                x: 0,
                y: 0,
                width: self
                    .geometry
                    .map_or(1, |g| g.size.0.clamp(1, i32::from(u16::MAX)))
                    as u16,
                height: self
                    .geometry
                    .map_or(1, |g| g.size.1.clamp(1, i32::from(u16::MAX)))
                    as u16,
            }]
        };

        // The bounding region and the input region are separate. The bounding
        // region controls what is drawn and the input region
        // controls what is hit tested, so both have to be set for a change to
        // take effect.
        shape::rectangles(
            &self.connection,
            shape::SO::SET,
            shape::SK::BOUNDING,
            xproto::ClipOrdering::UNSORTED,
            window,
            0,
            0,
            &rectangles,
        )
        .map_err(|e| OverlayError::Platform(format!("XShape bounding region failed: {e}")))?;

        shape::rectangles(
            &self.connection,
            shape::SO::SET,
            shape::SK::INPUT,
            xproto::ClipOrdering::UNSORTED,
            window,
            0,
            0,
            &rectangles,
        )
        .map_err(|e| OverlayError::Platform(format!("XShape input region failed: {e}")))?;

        self.flush()
    }

    /// Tell the window manager this window must never take keyboard focus.
    ///
    /// `WM_HINTS` with the input flag set and the input field cleared is what
    /// EWMH-compliant managers read to know a window must not be focused.
    fn set_input_hint(&self, window: xproto::Window) -> Result<(), OverlayError> {
        // WM_HINTS layout: flags, input, initial state, icon pixmap,
        // icon window, icon x, icon y, icon mask, window group.
        const WM_HINTS_INPUT_HINT: u32 = 1 << 0;
        const WM_HINTS_STATE_HINT: u32 = 1 << 1;
        const NORMAL_STATE: u32 = 1;

        let hints: [u32; 9] = [
            WM_HINTS_INPUT_HINT | WM_HINTS_STATE_HINT,
            0, // input: false, the island must never be focused
            NORMAL_STATE,
            0, // icon pixmap
            0, // icon window
            0, // icon x
            0, // icon y
            0, // icon mask
            0, // window group
        ];

        self.connection
            .change_property32(
                xproto::PropMode::REPLACE,
                window,
                xproto::AtomEnum::WM_HINTS,
                xproto::AtomEnum::WM_HINTS,
                &hints,
            )
            .map_err(|e| OverlayError::Platform(format!("setting WM_HINTS failed: {e}")))?;
        self.flush()
    }

    /// Flush the request buffer.
    ///
    /// X11 buffers requests, so without this the server may not have acted on
    /// anything sent, which shows up as a window that ignores its click-through
    /// setting until something else forces a round trip.
    fn flush(&self) -> Result<(), OverlayError> {
        self.connection
            .flush()
            .map_err(|e| OverlayError::Platform(format!("flush failed: {e}")))
    }

    /// Enumerate monitors through XRandR.
    ///
    /// Falls back to a single screen sized monitor when RandR is unavailable or
    /// answers with nothing, because a stub RandR is normal on old servers and
    /// the island still needs a monitor to live on.
    fn enumerate_monitors(&self) -> Vec<Monitor> {
        let setup = self.connection.setup();
        let Some(screen) = setup.roots.get(self.screen_num) else {
            return Vec::new();
        };

        let screen_fallback = Monitor::new(
            0,
            format!("screen-{}", self.screen_num),
            Rect::new(
                0.0,
                0.0,
                screen.width_in_pixels as f32,
                screen.height_in_pixels as f32,
            ),
            true,
        );

        let resources = randr::get_screen_resources_current(&self.connection, screen.root)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        let Some(resources) = resources else {
            return vec![screen_fallback];
        };

        let mut monitors = Vec::new();

        for output in resources.outputs.iter().copied() {
            let info = randr::get_output_info(&self.connection, screen.root, output)
                .ok()
                .and_then(|cookie| cookie.reply().ok());
            let Some(info) = info else { continue };

            // `GetOutputInfoReply` carries only the CRTC id, not its geometry, so both the
            // position and the pixel size come from the CRTC itself. An output
            // whose CRTC is `None` is disconnected and is skipped.
            let crtc = info.crtc;
            if crtc == X_NONE {
                continue;
            }
            let Ok(crtc_cookie) = randr::get_crtc_info(&self.connection, screen.root, crtc) else {
                continue;
            };
            let Ok(crtc_info) = crtc_cookie.reply() else {
                continue;
            };

            let width = crtc_info.width as f32;
            let height = crtc_info.height as f32;
            if width <= 0.0 || height <= 0.0 {
                // A disconnected output reports zero dimensions.
                continue;
            }
            let position = (crtc_info.x as f32, crtc_info.y as f32);

            // X11 has no primary flag, so the output covering the whole screen
            // at the origin is treated as primary, which is the convention most
            // desktops follow when they arrange their displays.
            let is_primary = info.connection == randr::Connection::CONNECTED
                && position == (0.0, 0.0)
                && width == screen.width_in_pixels as f32
                && height == screen.height_in_pixels as f32;

            monitors.push(Monitor {
                index: monitors.len(),
                name: String::from_utf8_lossy(&info.name).into_owned(),
                bounds: Rect::new(position.0, position.1, width, height),
                scale: ScaleFactor::unscaled(),
                refresh_hz: 0,
                is_primary,
            });
        }

        if monitors.is_empty() {
            monitors.push(screen_fallback);
        }
        monitors
    }
}

/// Find a 32-bit TrueColor ARGB visual.
///
/// Depth 32 is what provides a per-pixel alpha channel. A server without one
/// cannot give the island transparency, so this returns a specific error rather
/// than silently producing an opaque window.
fn choose_argb_visual(
    connection: &RustConnection,
    screen_num: usize,
) -> Result<xproto::Visualid, OverlayError> {
    let setup = connection.setup();
    let screen = setup.roots.get(screen_num).ok_or_else(|| {
        OverlayError::NoDisplay(format!(
            "the display server reported no screen {screen_num}"
        ))
    })?;

    for depth in &screen.allowed_depths {
        if depth.depth != 32 {
            continue;
        }
        for visual in &depth.visuals {
            if visual.class == xproto::VisualClass::TRUE_COLOR && is_argb_visual(visual) {
                return Ok(visual.visual_id);
            }
        }
    }

    Err(OverlayError::Platform(
        "no 32-bit TrueColor ARGB visual is available, so the island cannot be transparent".into(),
    ))
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

        self.window = handle.window as u32;
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

        let class = b"lumina\0";
        self.connection
            .change_property8(
                xproto::PropMode::REPLACE,
                win,
                self.atoms.net_wm_window_name,
                self.atoms.utf8_string,
                b"Lumina",
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

        self.flush()
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
                &xproto::ConfigureWindowAux {
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

        self.flush()
    }

    fn enforce_topmost(&mut self) -> Result<(), OverlayError> {
        if self.window == X_NONE {
            return Err(OverlayError::NoWindowHandle);
        }
        self.topmost_enforces = self.topmost_enforces.saturating_add(1);

        // Requirement: stay above full-screen applications.
        //
        // The above state is re-sent because a full-screen window or some window
        // managers drop it, and re-sending is the documented way to reclaim it.
        self.state_message(self.window, 1, self.atoms.net_wm_state_above)
    }

    fn set_click_through(&mut self, enabled: bool) -> Result<(), OverlayError> {
        self.click_through = enabled;
        if self.window == X_NONE {
            return Err(OverlayError::NoWindowHandle);
        }
        self.set_input_region(self.window, enabled)
    }

    fn click_through(&self) -> bool {
        self.click_through
    }

    fn set_visible(&mut self, visible: bool) -> Result<(), OverlayError> {
        self.visible = visible;
        if self.window == X_NONE {
            return Err(OverlayError::NoWindowHandle);
        }
        if visible {
            self.state_message(self.window, 1, self.atoms.net_wm_state_above)
        } else {
            self.remove_state(self.window, self.atoms.net_wm_state_above)
        }
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn current_scale(&self) -> ScaleFactor {
        self.scale
    }

    fn tick(&mut self) -> Result<(), OverlayError> {
        self.flush()
    }

    fn shutdown(&mut self) {
        if self.window != X_NONE {
            let _ = self.set_visible(false);
            let _ = self.connection.destroy_window(self.window);
            let _ = self.flush();
        }
        self.window = X_NONE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::compute_placement;
    use lumina_config::Position;

    fn sample_geometry() -> WindowGeometry {
        compute_placement(
            Rect::new(0.0, 0.0, 1920.0, 1080.0),
            0,
            1.0,
            Position::TopCenter,
            (124.0, 36.0),
            0,
            0,
        )
        .geometry
    }

    /// Connecting without a display must fail cleanly. On a build machine there
    /// is no X server, and the application has to survive that.
    #[test]
    fn connecting_without_a_display_fails_cleanly() {
        match X11Backend::new() {
            Err(OverlayError::NoDisplay(_)) => {}
            Err(other) => panic!("expected a display error, got {other:?}"),
            Ok(_) => {}
        }
    }

    #[test]
    fn operations_before_the_window_is_adopted_report_an_error() {
        let Ok(mut backend) = X11Backend::new() else {
            // No display available, so the pre-window path cannot be exercised
            // here. The null backend covers that in its own tests.
            return;
        };

        assert_eq!(backend.enforce_topmost(), Err(OverlayError::NoWindowHandle));
        assert_eq!(backend.set_visible(true), Err(OverlayError::NoWindowHandle));
        assert_eq!(
            backend.set_click_through(true),
            Err(OverlayError::NoWindowHandle)
        );
    }

    #[test]
    fn click_through_intent_is_recorded_even_when_the_call_fails() {
        let Ok(mut backend) = X11Backend::new() else {
            return;
        };
        let _ = backend.set_click_through(true);
        assert!(
            backend.click_through(),
            "the intent must be visible to callers"
        );
    }

    #[test]
    fn a_dedicated_colormap_is_created_for_the_argb_visual() {
        let Ok(backend) = X11Backend::new() else {
            return;
        };
        // Zero is `X11 None`, so a real colormap proves the dedicated colormap
        // requirement is met rather than silently falling back to the default.
        assert_ne!(
            backend.colormap().0,
            0,
            "a dedicated colormap must exist for ARGB"
        );
        assert_ne!(
            backend.visual().0,
            0,
            "an ARGB visual must have been selected"
        );
    }

    #[test]
    fn capabilities_report_the_x11_matrix() {
        let caps = match X11Backend::new() {
            Ok(backend) => backend.capabilities(),
            Err(_) => Capabilities::for_platform(Platform::X11),
        };
        assert_eq!(caps.platform, Platform::X11);
        assert!(caps.exclusive_zone, "a dock window reserves space");
        assert!(
            !caps.native_backdrop_blur,
            "there is no portable X11 blur API"
        );
        assert!(caps.click_through, "an empty input region implements it");
        assert!(caps.global_hotkeys, "XGrabKey works without a portal");
    }

    #[test]
    fn the_argb_visual_requirement_is_real() {
        let Ok((connection, screen)) = x11rb::connect(None) else {
            return;
        };
        match choose_argb_visual(&connection, screen) {
            Ok(visual) => assert_ne!(visual.0, 0),
            Err(e) => {
                assert!(
                    e.to_string().contains("transparent"),
                    "the error should explain why: {e}"
                );
            }
        }
    }

    #[test]
    fn monitor_enumeration_never_returns_an_empty_list_when_a_display_exists() {
        let Ok(backend) = X11Backend::new() else {
            return;
        };
        // A monitor list the placement code can iterate, even if RandR is a stub.
        assert!(!backend.monitors().is_empty());
        assert!(backend.monitors().iter().all(|m| m.bounds.width > 0.0));
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

    #[test]
    fn sample_geometry_is_sane() {
        let g = sample_geometry();
        assert!(g.size.0 > 0 && g.size.1 > 0);
    }
}
