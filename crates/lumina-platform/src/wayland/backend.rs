//! A Wayland overlay backend built directly on `wayland-client`.
//!
//! # Surface kinds
//!
//! The backend prefers `zwlr_layer_shell_v1`, because it is the only protocol
//! that gives an ordinary client all four things an island needs: an anchor on
//! a chosen output, a margin, an exclusive zone, and an input region. When the
//! compositor does not advertise it (GNOME is the common case), the backend
//! still creates a window through `xdg-shell` so Lumina can start and render.
//! That fallback loses positioning, click-through, and space reservation, which
//! is reported through [`Capabilities`] rather than silently ignored.
//!
//! # What this module does not do
//!
//! It does not allocate buffers. The renderer creates a wgpu surface from the
//! `wl_display` and `wl_surface` pointers exposed through
//! [`WaylandBackend::raw_display_handle`] and [`WaylandBackend::raw_window_handle`],
//! and presents frames itself. This keeps the platform crate free of graphics
//! dependencies and means the backend can be tested for creation on a headless
//! compositor without a GPU.
//!
//! # Configure events
//!
//! A configure is recorded as soon as it arrives. With no buffer attached there
//! is nothing to resize, so the backend acknowledges it immediately after
//! updating its recorded size. Once the renderer is attached it must read
//! [`WaylandBackend::pending_configure`] from its frame loop and resize the
//! swapchain before the next commit; the acknowledgement has already happened,
//! so the only visible effect of getting that order wrong is one stretched
//! frame, never a stalled surface.
//!
//! # Keyboard input
//!
//! Seats and keyboards are bound so key events are not discarded at the
//! protocol level. Keys are delivered as raw evdev codes: translating them to
//! symbols needs an xkb keymap, which is the window layer's job in Phase 6.
//! Modifier bit fields are recorded verbatim for the same consumer.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::NonNull;

use lumina_config::Position;
use lumina_core::geometry::Rect;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_compositor, wl_keyboard, wl_output, wl_region, wl_registry, wl_seat, wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::capability::Capabilities;
use crate::monitor::{Monitor, ScaleFactor};
use crate::overlay::{OverlayBackend, OverlayError};
use crate::placement::WindowGeometry;
use crate::wayland::configure::{ConfigureTracker, PendingConfigure};
use crate::wayland::detect::{platform_for, Session};
use crate::wayland::placement::{layer_placement, Layer, LayerPlacement};

/// The default size requested before the renderer reports its first geometry.
const INITIAL_SIZE: (u32, u32) = (320, 36);

/// The namespace shown to the compositor in layer-shell debugging tools.
const NAMESPACE: &str = "lumina";

/// Which protocol carries the overlay surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaylandSurfaceKind {
    /// `zwlr_layer_shell_v1`, with anchoring, margins, and exclusive zones.
    LayerShell,
    /// `xdg-shell` as an ordinary window. Used when layer-shell is absent.
    XdgToplevel,
}

/// One key event, in evdev code space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// Raw evdev key code, as sent by the compositor.
    pub key: u32,
    /// True for press and repeat, false for release.
    pub pressed: bool,
    /// Compositor timestamp in milliseconds.
    pub time: u32,
}

/// Raw modifier state from the compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyboardModifiers {
    /// Depressed modifier bits.
    pub depressed: u32,
    /// Latched modifier bits.
    pub latched: u32,
    /// Locked modifier bits, such as caps lock.
    pub locked: u32,
    /// Active keyboard group.
    pub group: u32,
}

/// A connected Wayland overlay backend.
pub struct WaylandBackend {
    connection: Connection,
    queue: EventQueue<State>,
    compositor: wl_compositor::WlCompositor,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    xdg_wm_base: Option<xdg_wm_base::XdgWmBase>,
    kind: WaylandSurfaceKind,
    state: State,
}

impl std::fmt::Debug for WaylandBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WaylandBackend")
            .field("kind", &self.kind)
            .field("output", &self.state.output_index)
            .field("configured", &self.state.tracker.configured_size())
            .field("scale", &self.state.scale)
            .finish_non_exhaustive()
    }
}

impl WaylandBackend {
    /// Connect to the session compositor with the shipped placement.
    pub fn new() -> Result<Self, OverlayError> {
        Self::with_placement(Position::TopCenter, 0, 12, false)
    }

    /// Connect to the session compositor with an explicit placement policy.
    pub fn with_placement(
        position: Position,
        offset_x: i32,
        offset_y: i32,
        reserve_space: bool,
    ) -> Result<Self, OverlayError> {
        let connection = Connection::connect_to_env()
            .map_err(|error| OverlayError::NoDisplay(error.to_string()))?;
        let (globals, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|error| OverlayError::NoDisplay(error.to_string()))?;
        let qh = queue.handle();

        let compositor = globals
            .bind::<wl_compositor::WlCompositor, _, _>(&qh, 4..=6, ())
            .map_err(|error| OverlayError::NoDisplay(error.to_string()))?;
        let layer_shell = globals
            .bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(&qh, 1..=5, ())
            .ok();
        let xdg_wm_base = globals
            .bind::<xdg_wm_base::XdgWmBase, _, _>(&qh, 1..=6, ())
            .ok();

        let kind = if layer_shell.is_some() {
            WaylandSurfaceKind::LayerShell
        } else if xdg_wm_base.is_some() {
            WaylandSurfaceKind::XdgToplevel
        } else {
            return Err(OverlayError::NoDisplay(
                "the compositor offers neither layer-shell nor xdg-shell".into(),
            ));
        };

        let mut state = State::new(position, offset_x, offset_y, reserve_space);

        let output_globals: Vec<(u32, String, u32)> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|global| global.interface == "wl_output")
                .map(|global| (global.name, global.interface.clone(), global.version))
                .collect()
        });
        for (index, (name, _interface, version)) in output_globals.into_iter().enumerate() {
            let proxy = globals.registry().bind::<wl_output::WlOutput, _, _>(
                name,
                version.min(4),
                &qh,
                index,
            );
            state.outputs.push(OutputBinding {
                proxy,
                name: String::new(),
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                scale: 1,
                refresh_mhz: 0,
                index,
            });
        }

        if let Ok(seat) = globals.bind::<wl_seat::WlSeat, _, _>(&qh, 1..=7, ()) {
            state.seat = Some(seat);
        }

        queue
            .roundtrip(&mut state)
            .map_err(|error| OverlayError::NoDisplay(error.to_string()))?;
        for output in &mut state.outputs {
            output.apply_defaults();
        }
        state.scale = state
            .outputs
            .first()
            .map_or(1.0, |output| output.scale as f32);

        let mut backend = Self {
            connection,
            queue,
            compositor,
            layer_shell,
            xdg_wm_base,
            kind,
            state,
        };
        backend.create_surface(None)?;
        Ok(backend)
    }

    /// Which protocol carries the surface.
    #[must_use]
    pub const fn surface_kind(&self) -> WaylandSurfaceKind {
        self.kind
    }

    /// The configure that arrived most recently, if any.
    ///
    /// The renderer reads this each frame and resizes its swapchain to the
    /// reported size. The size is in surface-local pixels; multiply by the
    /// scale factor for the buffer size.
    #[must_use]
    pub const fn pending_configure(&self) -> Option<PendingConfigure> {
        self.state.tracker.pending()
    }

    /// The size the surface has been configured at, in surface-local pixels.
    #[must_use]
    pub const fn configured_size(&self) -> (u32, u32) {
        self.state.tracker.configured_size()
    }

    /// Change the placement policy and apply it to the live surface.
    pub fn set_placement(
        &mut self,
        position: Position,
        offset_x: i32,
        offset_y: i32,
        reserve_space: bool,
    ) -> Result<(), OverlayError> {
        self.state.position = position;
        self.state.offset_x = offset_x;
        self.state.offset_y = offset_y;
        self.state.reserve_space = reserve_space;
        self.recompute_layer();
        self.apply_layer_geometry()
    }

    /// Move the island to a specific output, recreating the surface if needed.
    ///
    /// `None` lets the compositor choose. This is the implementation of the
    /// per-output rule from the plan: layer surfaces cannot move between
    /// outputs, so a change destroys the surface and creates a new one.
    pub fn set_output(&mut self, output: Option<usize>) -> Result<(), OverlayError> {
        if self.kind != WaylandSurfaceKind::LayerShell {
            self.state.output_index = output;
            self.state.scale = output
                .and_then(|index| {
                    self.state
                        .outputs
                        .iter()
                        .find(|candidate| candidate.index == index)
                })
                .map_or(1.0, |candidate| candidate.scale as f32);
            return Ok(());
        }
        if self.state.output_index == output {
            return Ok(());
        }
        self.recreate_surface(output)
    }

    /// Raw display handle for the graphics API.
    #[must_use]
    pub fn raw_display_handle(&self) -> Option<RawDisplayHandle> {
        let pointer = self.connection.display().id().as_ptr().cast::<c_void>();
        let pointer = NonNull::new(pointer)?;
        Some(WaylandDisplayHandle::new(pointer).into())
    }

    /// Raw surface handle for the graphics API.
    #[must_use]
    pub fn raw_window_handle(&self) -> Option<RawWindowHandle> {
        let surface = &self.state.surface.as_ref()?.surface;
        let pointer = surface.id().as_ptr().cast::<c_void>();
        let pointer = NonNull::new(pointer)?;
        Some(WaylandWindowHandle::new(pointer).into())
    }

    /// Drain raw key events collected since the last call.
    pub fn take_key_events(&mut self) -> Vec<KeyEvent> {
        self.state.keys.drain(..).collect()
    }

    /// The current raw modifier state.
    #[must_use]
    pub const fn keyboard_modifiers(&self) -> KeyboardModifiers {
        self.state.modifiers
    }

    /// True while a keyboard has focus on the island.
    #[must_use]
    pub const fn keyboard_focused(&self) -> bool {
        self.state.keyboard_focused
    }

    /// True when the compositor closed the surface.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.state.closed
    }

    /// The output indices currently known, in enumeration order.
    #[must_use]
    pub fn output_indices(&self) -> Vec<usize> {
        self.state.outputs.iter().map(|o| o.index).collect()
    }

    fn recompute_layer(&mut self) {
        let size = self.state.tracker.preferred_size();
        self.state.layer = layer_placement(
            self.state.position,
            self.state.offset_x,
            self.state.offset_y,
            self.state.reserve_space,
            size.1,
            self.state.click_through,
        );
    }

    fn create_surface(&mut self, output: Option<usize>) -> Result<(), OverlayError> {
        self.recompute_layer();
        let qh = self.queue.handle();
        let surface = self.compositor.create_surface(&qh, ());
        let role = match self.kind {
            WaylandSurfaceKind::LayerShell => {
                let layer_shell = self
                    .layer_shell
                    .as_ref()
                    .ok_or(OverlayError::Unsupported("layer-shell"))?;
                let output_proxy = output.and_then(|index| {
                    self.state
                        .outputs
                        .iter()
                        .find(|candidate| candidate.index == index)
                        .map(|candidate| &candidate.proxy)
                });
                let protocol_layer = match self.state.layer.layer {
                    Layer::Background => zwlr_layer_shell_v1::Layer::Background,
                    Layer::Bottom => zwlr_layer_shell_v1::Layer::Bottom,
                    Layer::Top => zwlr_layer_shell_v1::Layer::Top,
                    Layer::Overlay => zwlr_layer_shell_v1::Layer::Overlay,
                };
                let layer_surface = layer_shell.get_layer_surface(
                    &surface,
                    output_proxy,
                    protocol_layer,
                    NAMESPACE.into(),
                    &qh,
                    (),
                );
                let size = self.state.tracker.preferred_size();
                layer_surface.set_size(size.0, size.1);
                layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::from_bits_truncate(
                    self.state.layer.anchor.bits(),
                ));
                let margins = self.state.layer.margins;
                layer_surface.set_margin(margins.top, margins.right, margins.bottom, margins.left);
                layer_surface.set_exclusive_zone(self.state.layer.exclusive_zone);
                if layer_surface.version() >= 4 {
                    layer_surface.set_keyboard_interactivity(match self.state.layer.keyboard {
                        crate::wayland::placement::KeyboardInteractivity::None => {
                            zwlr_layer_surface_v1::KeyboardInteractivity::None
                        }
                        crate::wayland::placement::KeyboardInteractivity::OnDemand => {
                            zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand
                        }
                        crate::wayland::placement::KeyboardInteractivity::Exclusive => {
                            zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive
                        }
                    });
                }
                Role::Layer { layer_surface }
            }
            WaylandSurfaceKind::XdgToplevel => {
                let wm_base = self
                    .xdg_wm_base
                    .as_ref()
                    .ok_or(OverlayError::Unsupported("xdg-shell"))?;
                let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
                let toplevel = xdg_surface.get_toplevel(&qh, ());
                toplevel.set_title("Lumina".into());
                toplevel.set_app_id(NAMESPACE.into());
                Role::Toplevel {
                    xdg_surface,
                    xdg_toplevel: toplevel,
                }
            }
        };

        self.state.surface = Some(Surface { surface, role });
        self.state.closed = false;
        self.state.output_index = output;
        self.state.scale = output
            .and_then(|index| self.state.outputs.iter().find(|o| o.index == index))
            .map_or_else(
                || self.state.outputs.first().map_or(1.0, |o| o.scale as f32),
                |o| o.scale as f32,
            );

        self.apply_input_region_to_surface();
        self.queue
            .roundtrip(&mut self.state)
            .map_err(|error| OverlayError::Platform(error.to_string()))?;
        if self.state.closed {
            return Err(OverlayError::Platform(
                "the compositor closed the surface during creation".into(),
            ));
        }
        Ok(())
    }

    fn recreate_surface(&mut self, output: Option<usize>) -> Result<(), OverlayError> {
        let preferred = self.state.tracker.preferred_size();
        self.destroy_surface();
        self.state.tracker = ConfigureTracker::new(preferred);
        self.create_surface(output)
    }

    fn destroy_surface(&mut self) {
        let Some(surface) = self.state.surface.take() else {
            return;
        };
        match surface.role {
            Role::Layer { layer_surface } => {
                if layer_surface.version() >= 3 {
                    layer_surface.destroy();
                }
            }
            Role::Toplevel {
                xdg_surface,
                xdg_toplevel,
            } => {
                xdg_toplevel.destroy();
                xdg_surface.destroy();
            }
        }
        surface.surface.destroy();
        let _ = self.connection.flush();
    }

    fn apply_layer_geometry(&mut self) -> Result<(), OverlayError> {
        let Some(surface) = &self.state.surface else {
            return Err(OverlayError::NoWindowHandle);
        };
        let Role::Layer { layer_surface } = &surface.role else {
            return Ok(());
        };
        let size = self.state.tracker.preferred_size();
        layer_surface.set_size(size.0, size.1);
        layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::from_bits_truncate(
            self.state.layer.anchor.bits(),
        ));
        let margins = self.state.layer.margins;
        layer_surface.set_margin(margins.top, margins.right, margins.bottom, margins.left);
        layer_surface.set_exclusive_zone(self.state.layer.exclusive_zone);
        surface.surface.commit();
        Ok(())
    }

    fn apply_input_region_to_surface(&mut self) {
        let Some(surface) = &self.state.surface else {
            return;
        };
        if self.state.click_through {
            let qh = self.queue.handle();
            let region = self.compositor.create_region(&qh, ());
            surface.surface.set_input_region(Some(&region));
            region.destroy();
        } else {
            surface.surface.set_input_region(None);
        }
        surface.surface.commit();
    }
}

impl OverlayBackend for WaylandBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities::for_platform(platform_for(
            Session::Wayland,
            self.kind == WaylandSurfaceKind::LayerShell,
        ))
    }

    fn monitors(&self) -> Vec<Monitor> {
        self.state
            .outputs
            .iter()
            .enumerate()
            .map(|(position, output)| {
                let scale = ScaleFactor::new(output.scale as f32);
                let logical_width = scale.to_logical(output.width as f32);
                let logical_height = scale.to_logical(output.height as f32);
                let mut monitor = Monitor::new(
                    output.index,
                    if output.name.is_empty() {
                        format!("wayland-{}", output.index)
                    } else {
                        output.name.clone()
                    },
                    Rect::new(
                        output.x as f32,
                        output.y as f32,
                        logical_width.round(),
                        logical_height.round(),
                    ),
                    position == 0,
                );
                monitor.scale = scale;
                monitor.refresh_hz = (output.refresh_mhz.max(0) as u32) / 1000;
                monitor
            })
            .collect()
    }

    fn configure_window(&mut self, window: RawWindowHandle) -> Result<(), OverlayError> {
        let RawWindowHandle::Wayland(handle) = window else {
            return Err(OverlayError::NoWindowHandle);
        };
        let surface = &self
            .state
            .surface
            .as_ref()
            .ok_or(OverlayError::NoWindowHandle)?
            .surface;
        if handle.surface.as_ptr() != surface.id().as_ptr().cast::<c_void>() {
            return Err(OverlayError::Platform(
                "the supplied surface handle does not belong to this backend".into(),
            ));
        }
        Ok(())
    }

    fn apply_geometry(&mut self, geometry: &WindowGeometry) -> Result<(), OverlayError> {
        let scale = if geometry.scale.is_finite() && geometry.scale > 0.0 {
            geometry.scale
        } else {
            1.0
        };
        let logical_width = (geometry.size.0 as f32 / scale).round().max(1.0) as u32;
        let logical_height = (geometry.size.1 as f32 / scale).round().max(1.0) as u32;
        let requested = if self.state.outputs.is_empty() {
            None
        } else {
            Some(geometry.monitor_index)
        };

        if self.kind == WaylandSurfaceKind::LayerShell
            && crate::wayland::placement::output_changed(self.state.output_index, requested)
        {
            self.recreate_surface(requested)?;
        }

        self.state
            .tracker
            .set_preferred((logical_width, logical_height));
        self.recompute_layer();
        self.apply_layer_geometry()
    }

    fn enforce_topmost(&mut self) -> Result<(), OverlayError> {
        match self.kind {
            WaylandSurfaceKind::LayerShell => Ok(()),
            WaylandSurfaceKind::XdgToplevel => Err(OverlayError::Unsupported(
                "always-on-top requires layer-shell",
            )),
        }
    }

    fn set_click_through(&mut self, enabled: bool) -> Result<(), OverlayError> {
        if enabled && self.kind == WaylandSurfaceKind::XdgToplevel {
            return Err(OverlayError::Unsupported(
                "click-through requires layer-shell",
            ));
        }
        self.state.click_through = enabled;
        self.apply_input_region_to_surface();
        Ok(())
    }

    fn click_through(&self) -> bool {
        self.state.click_through
    }

    fn set_visible(&mut self, visible: bool) -> Result<(), OverlayError> {
        // A Wayland surface is mapped by attaching a buffer, which only the
        // renderer can do. Recording the state lets the renderer decide whether
        // to present the next frame without destroying the surface.
        self.state.visible = visible;
        Ok(())
    }

    fn is_visible(&self) -> bool {
        self.state.visible
    }

    fn current_scale(&self) -> ScaleFactor {
        ScaleFactor::new(self.state.scale)
    }

    fn tick(&mut self) -> Result<(), OverlayError> {
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| OverlayError::Platform(error.to_string()))?;
        self.connection
            .flush()
            .map_err(|error| OverlayError::Platform(error.to_string()))?;
        Ok(())
    }

    fn shutdown(&mut self) {
        self.destroy_surface();
    }
}

impl Drop for WaylandBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The supported part of a surface wrapped in a role.
#[derive(Debug)]
struct Surface {
    surface: wl_surface::WlSurface,
    role: Role,
}

#[derive(Debug)]
enum Role {
    Layer {
        layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    },
    Toplevel {
        xdg_surface: xdg_surface::XdgSurface,
        xdg_toplevel: xdg_toplevel::XdgToplevel,
    },
}

#[derive(Debug, Clone)]
struct OutputBinding {
    proxy: wl_output::WlOutput,
    name: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: i32,
    refresh_mhz: i32,
    index: usize,
}

impl OutputBinding {
    fn apply_defaults(&mut self) {
        if self.width <= 0 {
            self.width = 1920;
        }
        if self.height <= 0 {
            self.height = 1080;
        }
        if self.scale <= 0 {
            self.scale = 1;
        }
    }
}

struct State {
    outputs: Vec<OutputBinding>,
    surface: Option<Surface>,
    tracker: ConfigureTracker,
    layer: LayerPlacement,
    position: Position,
    offset_x: i32,
    offset_y: i32,
    reserve_space: bool,
    click_through: bool,
    visible: bool,
    closed: bool,
    keys: VecDeque<KeyEvent>,
    modifiers: KeyboardModifiers,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    keyboard_focused: bool,
    /// Kept alive for the lifetime of the keyboard proxy.
    seat: Option<wl_seat::WlSeat>,
    xdg_size: Option<(u32, u32)>,
    output_index: Option<usize>,
    scale: f32,
}

impl State {
    fn new(position: Position, offset_x: i32, offset_y: i32, reserve_space: bool) -> Self {
        let layer = layer_placement(
            position,
            offset_x,
            offset_y,
            reserve_space,
            INITIAL_SIZE.1,
            false,
        );
        Self {
            outputs: Vec::new(),
            surface: None,
            tracker: ConfigureTracker::new(INITIAL_SIZE),
            layer,
            position,
            offset_x,
            offset_y,
            reserve_space,
            click_through: false,
            visible: true,
            closed: false,
            keys: VecDeque::new(),
            modifiers: KeyboardModifiers::default(),
            keyboard: None,
            keyboard_focused: false,
            seat: None,
            xdg_size: None,
            output_index: None,
            scale: 1.0,
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        // The global list helper keeps its own copy up to date. Hotplug while
        // running is handled by the registry only when the surface is recreated
        // for a known output; a full hotplug listener belongs with the renderer
        // integration in Phase 6.
    }
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _output: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        let Some(binding) = state.outputs.get_mut(*index) else {
            return;
        };
        match event {
            wl_output::Event::Geometry { x, y, .. } => {
                binding.x = x;
                binding.y = y;
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                refresh,
            } => {
                if flags.contains(wl_output::Mode::Current) {
                    binding.width = width;
                    binding.height = height;
                    binding.refresh_mhz = refresh;
                }
            }
            wl_output::Event::Scale { factor } => binding.scale = factor,
            wl_output::Event::Name { name } => binding.name = name,
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _data: &(),
        _connection: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(queue, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Key {
                time,
                key,
                state: key_state,
                ..
            } => {
                let pressed = matches!(key_state, WEnum::Value(wl_keyboard::KeyState::Pressed));
                state.keys.push_back(KeyEvent { key, pressed, time });
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                state.modifiers = KeyboardModifiers {
                    depressed: mods_depressed,
                    latched: mods_latched,
                    locked: mods_locked,
                    group,
                };
            }
            wl_keyboard::Event::Enter { .. } => state.keyboard_focused = true,
            wl_keyboard::Event::Leave { .. } => state.keyboard_focused = false,
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                if state.tracker.on_configure(serial, width, height).is_ok() {
                    layer_surface.ack_configure(serial);
                    state.tracker.ack();
                }
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            let (width, height) = state.xdg_size.unwrap_or((0, 0));
            if state.tracker.on_configure(serial, width, height).is_ok() {
                xdg_surface.ack_configure(serial);
                state.tracker.ack();
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                state.xdg_size = Some((width.max(0) as u32, height.max(0) as u32));
            }
            xdg_toplevel::Event::Close => state.closed = true,
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_region::WlRegion);
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_that_need_a_compositor_are_reported_as_unsupported_when_absent() {
        // Constructing the backend requires a live compositor, so the mapping
        // from surface kind to capabilities is what can be tested everywhere.
        let layer = Capabilities::for_platform(platform_for(Session::Wayland, true));
        let toplevel = Capabilities::for_platform(platform_for(Session::Wayland, false));
        assert!(layer.click_through);
        assert!(layer.exclusive_zone);
        assert!(!toplevel.click_through);
        assert!(!toplevel.exclusive_zone);
    }

    #[test]
    fn raw_key_state_is_preserved() {
        let event = KeyEvent {
            key: 30,
            pressed: true,
            time: 1234,
        };
        assert_eq!(event.key, 30);
        assert!(event.pressed);
    }
}
