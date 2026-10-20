//! The desktop runtime: window lifecycle, GPU present loop, and IPC bridge.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lumina_audio::AudioAggregates;
use lumina_color::ColorEngine;
use lumina_config::{validate, Config, GpuBackend, MonitorSelector, MotionConfig, RgbMode};
use lumina_core::geometry::{Geometry, Rect, Scalar, Vec2};
use lumina_core::spring::Spring;
use lumina_core::state::{IslandState, StateMachine, StateRequest, TransitionSource};
use lumina_ipc::{
    transport, Command, Event, EventSender, GeometrySnapshot, Handler, IpcAddress, IpcError,
    NotifyArgs, RgbSnapshot, Server, ServerHandle, ServerOptions, SetRgbArgs, ShowArgs,
    StateSnapshot,
};
use lumina_platform::monitor::ScaleFactor;
use lumina_platform::placement::{compute_placement, Placement};
use lumina_platform::single_instance::{self, InstanceGuard};
use lumina_render::pipeline::IslandPipeline;
use lumina_render::render::{FrameIntent, RenderLoop};
use lumina_render::uniforms::{FrameParams, IslandUniforms};
use pollster::block_on;
use serde_json::{json, Value};
use wgpu::{
    Backends, CompositeAlphaMode, DeviceDescriptor, Instance, InstanceDescriptor, PresentMode,
    RequestAdapterOptions, Surface, SurfaceConfiguration, SurfaceError, TextureFormat,
    TextureUsages,
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowLevel};

const RUNTIME_NAME: &str = "lumina";
const IPC_TIMEOUT: Duration = Duration::from_secs(2);
const ACTIVATION_POLL: Duration = Duration::from_millis(250);
const QUIT_GRACE: Duration = Duration::from_millis(120);
const MIN_SURFACE: u32 = 16;

/// Options accepted by the desktop runtime.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeOptions {
    /// Configuration file to load. A missing file means built-in defaults.
    pub config_path: PathBuf,
    /// Start without revealing the window.
    pub hidden: bool,
}

/// How a runtime invocation finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunOutcome {
    /// The event loop exited normally.
    Exited,
    /// Another primary instance already owns the desktop session.
    AlreadyRunning,
}

/// A runtime startup or event-loop failure.
#[derive(Debug)]
pub(crate) enum RuntimeError {
    /// Native Wayland is not supported by this build.
    #[cfg(all(unix, not(target_os = "macos")))]
    WaylandUnsupported,
    /// The configuration could not be read or parsed.
    Config(String),
    /// A window or GPU resource could not be created.
    Startup(String),
    /// The event loop itself failed.
    EventLoop(String),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(all(unix, not(target_os = "macos")))]
            Self::WaylandUnsupported => formatter.write_str(
                "native Wayland sessions are not supported yet; run under X11 or use XWayland",
            ),
            Self::Config(message) => write!(formatter, "configuration: {message}"),
            Self::Startup(message) => write!(formatter, "startup: {message}"),
            Self::EventLoop(message) => write!(formatter, "event loop: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

/// Start the island and block until it exits.
pub(crate) fn run(options: RuntimeOptions) -> Result<RunOutcome, RuntimeError> {
    if let Some(error) = session_error() {
        return Err(error);
    }
    let config = load_config(&options.config_path)?;

    let guard = if config.general.single_instance {
        match single_instance::claim(RUNTIME_NAME) {
            Ok(guard) => Some(guard),
            Err(_) => return Ok(RunOutcome::AlreadyRunning),
        }
    } else {
        None
    };

    let event_loop = EventLoop::<RuntimeEvent>::with_user_event()
        .build()
        .map_err(|error| RuntimeError::EventLoop(error.to_string()))?;
    let proxy = event_loop.create_proxy();
    let mut app = RuntimeApp::new(options, config, proxy, guard);

    let loop_result = event_loop.run_app(&mut app);
    if let Some(error) = app.fatal.take() {
        return Err(RuntimeError::Startup(error));
    }
    loop_result.map_err(|error| RuntimeError::EventLoop(error.to_string()))?;
    Ok(app.outcome)
}

fn session_error() -> Option<RuntimeError> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let session = lumina_platform::wayland::session_from(
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("DISPLAY").ok().as_deref(),
        );
        if session == lumina_platform::wayland::detect::Session::Wayland {
            return Some(RuntimeError::WaylandUnsupported);
        }
    }
    None
}

fn load_config(path: &Path) -> Result<Config, RuntimeError> {
    let parsed = if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|error| {
            RuntimeError::Config(format!("could not read {}: {error}", path.display()))
        })?;
        Config::from_toml_str(&text).map_err(|error| {
            RuntimeError::Config(format!("could not parse {}: {error}", path.display()))
        })?
    } else {
        Config::default()
    };

    let (config, report) = validate(parsed);
    for issue in report.issues() {
        eprintln!("configuration: {issue}");
    }
    Ok(config)
}

fn maximal_logical_size(config: &Config) -> (Scalar, Scalar) {
    let mut half_top: Scalar = 0.0;
    let mut half_bottom: Scalar = 0.0;
    let mut height: Scalar = 0.0;
    let mut radius: Scalar = 0.0;

    for state in IslandState::ALL {
        let geometry = state.geometry().clamped();
        half_top = half_top.max(geometry.half_top);
        half_bottom = half_bottom.max(geometry.half_bottom);
        height = height.max(geometry.height);
        radius = radius.max(geometry.radius);
    }

    let geometry = Geometry::new(half_top, half_bottom, height, radius);
    let padding = config.appearance.glow_spread.max(0.0) + 2.0;
    let size = geometry.padded_size(padding);
    (
        size.width.max(MIN_SURFACE as Scalar),
        size.height.max(MIN_SURFACE as Scalar),
    )
}

fn backend_candidates(backend: GpuBackend) -> Vec<(&'static str, Backends)> {
    match backend {
        GpuBackend::Auto => auto_backend_candidates(),
        GpuBackend::Vulkan => vec![("Vulkan", Backends::VULKAN)],
        GpuBackend::Dx12 => vec![("DX12", Backends::DX12)],
        GpuBackend::Metal => vec![("Metal", Backends::METAL)],
        GpuBackend::Gl => vec![("GL", Backends::GL)],
    }
}

fn auto_backend_candidates() -> Vec<(&'static str, Backends)> {
    #[cfg(target_os = "windows")]
    {
        vec![
            ("Vulkan", Backends::VULKAN),
            ("DX12", Backends::DX12),
            ("GL", Backends::GL),
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![("Vulkan", Backends::VULKAN), ("GL", Backends::GL)]
    }
    #[cfg(target_os = "macos")]
    {
        vec![("Metal", Backends::METAL)]
    }
}

fn choose_format(formats: &[TextureFormat]) -> Option<TextureFormat> {
    formats
        .iter()
        .copied()
        .find(TextureFormat::is_srgb)
        .or_else(|| formats.first().copied())
}

fn choose_present_mode(modes: &[PresentMode], vsync: bool) -> PresentMode {
    if vsync {
        PresentMode::Fifo
    } else if modes.contains(&PresentMode::Immediate) {
        PresentMode::Immediate
    } else if modes.contains(&PresentMode::Mailbox) {
        PresentMode::Mailbox
    } else {
        PresentMode::Fifo
    }
}

fn choose_alpha_mode(modes: &[CompositeAlphaMode]) -> Option<CompositeAlphaMode> {
    [
        CompositeAlphaMode::PreMultiplied,
        CompositeAlphaMode::PostMultiplied,
        CompositeAlphaMode::Inherit,
    ]
    .into_iter()
    .find(|mode| modes.contains(mode))
}

struct GpuState {
    surface: Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: SurfaceConfiguration,
    pipeline: IslandPipeline,
    backend: &'static str,
}

impl GpuState {
    fn new(window: &Arc<Window>, config: &Config) -> Result<Self, String> {
        let size = surface_size(window);
        let mut failures = Vec::new();

        for (name, backends) in backend_candidates(config.advanced.gpu_backend) {
            match Self::create(window, size, config, backends, name) {
                Ok(state) => return Ok(state),
                Err(error) => failures.push(format!("{name}: {error}")),
            }
        }

        Err(format!(
            "no usable graphics backend was found ({})",
            failures.join("; ")
        ))
    }

    fn create(
        window: &Arc<Window>,
        size: (u32, u32),
        config: &Config,
        backends: Backends,
        name: &'static str,
    ) -> Result<Self, String> {
        let instance = Instance::new(InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let surface = instance
            .create_surface(Arc::clone(window))
            .map_err(|error| error.to_string())?;

        let adapter = block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: if config.advanced.power_saving {
                wgpu::PowerPreference::LowPower
            } else {
                wgpu::PowerPreference::HighPerformance
            },
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or_else(|| "no adapter for this surface".to_string())?;

        let capabilities = surface.get_capabilities(&adapter);
        let format = choose_format(&capabilities.formats)
            .ok_or_else(|| "no surface texture format".to_string())?;
        let present_mode = choose_present_mode(&capabilities.present_modes, config.advanced.vsync);
        let alpha_mode = choose_alpha_mode(&capabilities.alpha_modes)
            .ok_or_else(|| "no transparent composite mode".to_string())?;

        let (device, queue) = block_on(adapter.request_device(
            &DeviceDescriptor {
                label: Some("lumina-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .map_err(|error| error.to_string())?;

        device.on_uncaptured_error(Box::new(|error| {
            eprintln!("lumina: GPU error: {error}");
        }));

        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0,
            height: size.1,
            present_mode,
            alpha_mode,
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };

        device.push_error_scope(wgpu::ErrorFilter::Validation);
        surface.configure(&device, &surface_config);
        if let Some(error) = block_on(device.pop_error_scope()) {
            return Err(format!("surface configuration failed: {error}"));
        }

        let pipeline = block_on(IslandPipeline::build(&device, format))
            .map_err(|error| format!("pipeline creation failed: {error}"))?;
        if pipeline.is_fallback {
            eprintln!("lumina: the island shader was rejected; using the solid fill fallback");
        }

        Ok(Self {
            surface,
            device,
            queue,
            config: surface_config,
            pipeline,
            backend: name,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width < MIN_SURFACE || height < MIN_SURFACE {
            return;
        }
        if self.config.width == width && self.config.height == height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn render(
        &mut self,
        window: &Window,
        uniforms: &IslandUniforms,
    ) -> Result<RenderOutcome, String> {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(SurfaceError::Timeout) => return Ok(RenderOutcome::Skipped),
            Err(SurfaceError::Outdated | SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    Ok(frame) => frame,
                    Err(SurfaceError::Timeout) => return Ok(RenderOutcome::Skipped),
                    Err(error) => return Err(format!("surface acquisition failed: {error}")),
                }
            }
            Err(SurfaceError::OutOfMemory) => {
                return Err("the GPU ran out of memory".to_string());
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lumina-frame"),
            });
        self.pipeline.write_uniforms(&self.queue, uniforms);

        {
            let color_attachment = Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lumina-island-pass"),
                color_attachments: &[color_attachment],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &self.pipeline.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        window.pre_present_notify();
        frame.present();
        Ok(RenderOutcome::Presented)
    }
}

enum RenderOutcome {
    Presented,
    Skipped,
}

fn surface_size(window: &Window) -> (u32, u32) {
    let size = window.inner_size();
    (size.width.max(MIN_SURFACE), size.height.max(MIN_SURFACE))
}

struct Morph {
    half_top: Spring,
    half_bottom: Spring,
    height: Spring,
    radius: Spring,
    content: Spring,
}

impl Morph {
    fn new(state: IslandState, motion: &MotionConfig) -> Self {
        let geometry = state.geometry();
        let mut morph = Self {
            half_top: Spring::new(
                geometry.half_top,
                motion.morph_response_s,
                motion.morph_damping,
            ),
            half_bottom: Spring::new(
                geometry.half_bottom,
                motion.morph_response_s,
                motion.morph_damping,
            ),
            height: Spring::new(
                geometry.height,
                motion.morph_response_s,
                motion.morph_damping,
            ),
            radius: Spring::new(
                geometry.radius,
                motion.morph_response_s,
                motion.morph_damping,
            ),
            content: Spring::new(
                state.content_opacity(),
                (motion.content_fade_ms.max(1) as f32) / 1000.0,
                1.0,
            ),
        };
        morph.apply_motion(motion);
        morph
    }

    fn apply_motion(&mut self, motion: &MotionConfig) {
        for spring in [
            &mut self.half_top,
            &mut self.half_bottom,
            &mut self.height,
            &mut self.radius,
        ] {
            spring.set_response(motion.morph_response_s);
            spring.set_damping_ratio(motion.morph_damping);
            spring.set_global_scale(motion.master_scale);
        }
        self.content
            .set_response((motion.content_fade_ms.max(1) as f32) / 1000.0);
        self.content.set_damping_ratio(1.0);
        self.content.set_global_scale(motion.master_scale);
    }

    fn springs_mut(&mut self) -> [&mut Spring; 5] {
        [
            &mut self.half_top,
            &mut self.half_bottom,
            &mut self.height,
            &mut self.radius,
            &mut self.content,
        ]
    }

    fn target(&mut self, state: IslandState, motion: &MotionConfig) {
        let geometry = state.geometry();
        self.half_top.set_target(geometry.half_top);
        self.half_bottom.set_target(geometry.half_bottom);
        self.height.set_target(geometry.height);
        self.radius.set_target(geometry.radius);
        self.content.set_target(state.content_opacity());

        self.apply_motion(motion);
        if motion.reduced {
            for spring in self.springs_mut() {
                spring.snap_to_target();
            }
        }
    }

    fn advance(&mut self, dt: f32) {
        for spring in self.springs_mut() {
            spring.advance(dt);
        }
    }

    fn geometry(&self) -> Geometry {
        Geometry::new(
            self.half_top.position(),
            self.half_bottom.position(),
            self.height.position(),
            self.radius.position(),
        )
        .clamped()
    }

    fn content_opacity(&self) -> f32 {
        self.content.position().clamp(0.0, 1.0)
    }

    fn is_animating(&self) -> bool {
        [
            &self.half_top,
            &self.half_bottom,
            &self.height,
            &self.radius,
            &self.content,
        ]
        .into_iter()
        .any(|spring| !spring.is_at_rest())
    }
}

struct RuntimeEvent {
    command: Command,
    reply: SyncSender<Result<Value, IpcError>>,
}

struct IpcBridge {
    proxy: EventLoopProxy<RuntimeEvent>,
    timeout: Duration,
}

impl Handler for IpcBridge {
    fn handle(&mut self, command: Command) -> Result<Value, IpcError> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.proxy
            .send_event(RuntimeEvent {
                command,
                reply: sender,
            })
            .map_err(|_| IpcError::internal("the Lumina event loop is not running"))?;
        receiver
            .recv_timeout(self.timeout)
            .map_err(|_| IpcError::internal("the Lumina event loop did not answer in time"))?
    }
}

enum ShowAction {
    Collapse,
    Request(StateRequest),
}

fn show_action(args: &ShowArgs, now_ms: u64) -> Result<ShowAction, IpcError> {
    let target = match args.widget.as_deref() {
        Some(widget) => IslandState::from_id(widget)
            .ok_or_else(|| IpcError::invalid_argument(format!("unknown widget `{widget}`")))?,
        None => IslandState::Media,
    };

    if target == IslandState::Collapsed {
        return Ok(ShowAction::Collapse);
    }
    if target == IslandState::Hover {
        return Err(IpcError::invalid_argument(
            "hover cannot be requested explicitly",
        ));
    }

    let request = match (args.duration_ms, args.pin) {
        (Some(_), Some(true)) => StateRequest::persistent(target, TransitionSource::Ipc),
        (Some(duration), _) => {
            StateRequest::timed(target, TransitionSource::Ipc).with_duration_from(now_ms, duration)
        }
        (None, Some(false)) => {
            StateRequest::timed(target, TransitionSource::Ipc).with_duration_from(now_ms, 5_000)
        }
        (None, _) => StateRequest::persistent(target, TransitionSource::Ipc),
    };
    Ok(ShowAction::Request(request))
}

fn palette_is_animating(config: &Config, state: IslandState) -> bool {
    !config.motion.reduced
        && state != IslandState::Collapsed
        && state != IslandState::Hover
        && config.rgb.mode != RgbMode::Static
}

fn config_section(config: &Config, section: Option<&str>) -> Result<Value, IpcError> {
    let mut value =
        serde_json::to_value(config).map_err(|error| IpcError::internal(error.to_string()))?;
    if let Some(section) = section {
        for segment in section.split('.').filter(|part| !part.is_empty()) {
            value = value.get(segment).cloned().ok_or_else(|| {
                IpcError::invalid_argument(format!("unknown section `{section}`"))
            })?;
        }
    }
    Ok(value)
}

fn ipc_address(config: &Config) -> io::Result<IpcAddress> {
    let custom = config.advanced.ipc_name.trim();
    if custom.is_empty() {
        return IpcAddress::default_for_user();
    }

    #[cfg(windows)]
    {
        let name = if custom.starts_with(r"\\.\pipe\") {
            custom.to_string()
        } else {
            format!(r"\\.\pipe\{custom}")
        };
        Ok(IpcAddress::pipe(name))
    }

    #[cfg(unix)]
    {
        if custom.contains('/') {
            Ok(IpcAddress::unix(custom))
        } else {
            let mut path = std::env::temp_dir();
            path.push(custom);
            Ok(IpcAddress::unix(path))
        }
    }
}

struct RuntimeApp {
    config: Config,
    config_path: PathBuf,
    state: StateMachine,
    morph: Morph,
    color: ColorEngine,
    render_loop: RenderLoop,
    events: EventSender,
    proxy: EventLoopProxy<RuntimeEvent>,
    guard: Option<InstanceGuard>,
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    placement: Option<Placement>,
    monitor_name: Option<String>,
    ipc: Option<ServerHandle>,
    started: Instant,
    last_frame: Instant,
    last_activity: Instant,
    pending_exit: Option<Instant>,
    visible: bool,
    window_revealed: bool,
    dirty: bool,
    animating: bool,
    pointer_inside: bool,
    fatal: Option<String>,
    outcome: RunOutcome,
}

impl RuntimeApp {
    fn new(
        options: RuntimeOptions,
        config: Config,
        proxy: EventLoopProxy<RuntimeEvent>,
        guard: Option<InstanceGuard>,
    ) -> Self {
        let state = StateMachine::new();
        let morph = Morph::new(state.current(), &config.motion);
        let color = ColorEngine::new(&config);
        let render_loop = RenderLoop::new(
            config.advanced.target_fps.clamp(1, 240),
            config.advanced.vsync,
        );
        let RuntimeOptions {
            config_path,
            hidden,
        } = options;
        let visible = !hidden && !config.general.start_hidden;

        Self {
            config,
            config_path,
            state,
            morph,
            color,
            render_loop,
            events: EventSender::default(),
            proxy,
            guard,
            window: None,
            gpu: None,
            placement: None,
            monitor_name: None,
            ipc: None,
            started: Instant::now(),
            last_frame: Instant::now(),
            last_activity: Instant::now(),
            pending_exit: None,
            visible,
            window_revealed: false,
            dirty: visible,
            animating: false,
            pointer_inside: false,
            fatal: None,
            outcome: RunOutcome::Exited,
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let (placement, monitor_name) = resolve_placement(event_loop, &self.config)?;
        let (width, height) = placement.size();
        let bounds = placement.bounds();
        let attributes = Window::default_attributes()
            .with_title("Lumina")
            .with_inner_size(PhysicalSize::new(
                width.max(MIN_SURFACE as i32) as u32,
                height.max(MIN_SURFACE as i32) as u32,
            ))
            .with_position(PhysicalPosition::new(
                bounds.x.round() as i32,
                bounds.y.round() as i32,
            ))
            .with_resizable(false)
            .with_transparent(true)
            .with_decorations(false)
            .with_visible(false)
            .with_window_level(WindowLevel::AlwaysOnTop);

        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| format!("could not create the island window: {error}"))?,
        );

        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowExtWindows;
            window.set_skip_taskbar(true);
        }

        let gpu = GpuState::new(&window, &self.config)?;
        eprintln!(
            "lumina: {} backend, {}x{} at {},{}",
            gpu.backend, gpu.config.width, gpu.config.height, bounds.x, bounds.y
        );

        self.window = Some(Arc::clone(&window));
        self.gpu = Some(gpu);
        self.placement = Some(placement);
        self.monitor_name = monitor_name;
        self.update_click_through();
        self.start_ipc();

        if self.visible {
            window.request_redraw();
        }
        Ok(())
    }

    fn start_ipc(&mut self) {
        if !self.config.advanced.ipc_enabled {
            return;
        }

        let address = match ipc_address(&self.config) {
            Ok(address) => address,
            Err(error) => {
                eprintln!("lumina: could not resolve the IPC address: {error}");
                return;
            }
        };
        let listener = match transport::bind(&address) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!(
                    "lumina: IPC is unavailable at {}: {error}",
                    address.as_str()
                );
                return;
            }
        };
        let bridge = IpcBridge {
            proxy: self.proxy.clone(),
            timeout: IPC_TIMEOUT,
        };
        match Server::new(listener, bridge, ServerOptions::default()).spawn(&self.events) {
            Ok(handle) => {
                eprintln!("lumina: IPC listening on {}", handle.address().as_str());
                self.ipc = Some(handle);
            }
            Err(error) => eprintln!("lumina: could not start IPC: {error}"),
        }
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if self.gpu.is_none() {
            return;
        }

        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().clamp(0.0, 0.1);
        self.last_frame = now;

        if self.state.resolve(self.now_ms()) {
            let _ = self
                .events
                .emit(&Event::state_changed(self.state.current().id()));
            self.update_click_through();
            self.dirty = true;
        }

        let current = self.state.current();
        self.morph.target(current, &self.config.motion);
        self.morph.advance(dt);
        let geometry = self.morph.geometry();
        let palette = self.color.update(dt, AudioAggregates::default(), None);
        let animating = self.morph.is_animating() || palette_is_animating(&self.config, current);
        self.animating = animating;

        let intent = FrameIntent {
            dirty: self.dirty,
            animating,
            audio_active: false,
            visible: self.visible,
        };
        if !self.render_loop.should_render(intent) {
            return;
        }

        let size = surface_size(&window);
        let origin = Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5);
        let scale = window.scale_factor().clamp(0.25, 8.0) as f32;
        let uniforms = IslandUniforms::for_frame(
            geometry,
            &palette,
            &self.config.appearance,
            FrameParams {
                position: origin,
                resolution: Vec2::new(size.0 as f32, size.1 as f32),
                dpr: scale,
                time_seconds: self.started.elapsed().as_secs_f32(),
                content_opacity: self.morph.content_opacity(),
            },
        );

        let result = match self.gpu.as_mut() {
            Some(gpu) => {
                gpu.resize(size.0, size.1);
                gpu.render(&window, &uniforms)
            }
            None => return,
        };

        match result {
            Ok(RenderOutcome::Presented) => {
                self.dirty = false;
                if self.visible && !self.window_revealed {
                    window.set_visible(true);
                    self.window_revealed = true;
                }
            }
            Ok(RenderOutcome::Skipped) => {}
            Err(error) => {
                self.fatal = Some(error);
                event_loop.exit();
            }
        }
    }

    fn set_state_request(&mut self, request: StateRequest) {
        self.state.request(request);
        self.last_activity = Instant::now();
        self.visible = true;
        self.dirty = true;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn collapse(&mut self) {
        self.state.clear_all();
        self.state.release_hold();
        self.state.force(IslandState::Collapsed);
        self.state.set_pointer_inside(false);
        self.pointer_inside = false;
        self.last_activity = Instant::now();
        self.dirty = true;
        self.update_click_through();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn hide(&mut self) {
        self.visible = false;
        self.window_revealed = false;
        if let Some(window) = &self.window {
            window.set_visible(false);
        }
        self.state.clear_all();
        self.state.release_hold();
        self.state.force(IslandState::Collapsed);
        self.state.set_pointer_inside(false);
        self.pointer_inside = false;
        self.morph
            .target(IslandState::Collapsed, &self.config.motion);
        for spring in self.morph.springs_mut() {
            spring.snap_to_target();
        }
        self.animating = false;
        self.dirty = false;
        self.last_activity = Instant::now();
        self.update_click_through();
    }

    fn show(&mut self) {
        let was_hidden = !self.visible;
        self.visible = true;
        self.last_activity = Instant::now();
        self.dirty = true;
        if was_hidden {
            self.window_revealed = false;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        self.update_click_through();
    }

    fn activate(&mut self) {
        self.set_state_request(StateRequest::persistent(
            IslandState::Media,
            TransitionSource::Ipc,
        ));
        self.show();
    }

    fn update_click_through(&self) {
        let Some(window) = &self.window else {
            return;
        };
        let pass_through = self.state.current() == IslandState::Collapsed
            && self.config.general.click_through_when_collapsed
            && !self.config.general.always_clickable;
        if let Err(error) = window.set_cursor_hittest(!pass_through) {
            eprintln!("lumina: could not update click-through: {error}");
        }
    }

    fn set_pointer_inside(&mut self, inside: bool) {
        if self.pointer_inside == inside {
            return;
        }
        self.pointer_inside = inside;
        self.state.set_pointer_inside(inside);
        self.last_activity = Instant::now();
        self.dirty = true;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn pointer_is_inside(&self, x: f64, y: f64) -> bool {
        let Some(window) = &self.window else {
            return false;
        };
        let size = window.inner_size();
        let geometry = self.morph.geometry();
        let scale = window.scale_factor();
        let half_width = f64::from(geometry.half_top.max(geometry.half_bottom)) * scale;
        let half_height = f64::from(geometry.height) * 0.5 * scale;
        let center_x = f64::from(size.width) * 0.5;
        let center_y = f64::from(size.height) * 0.5;
        (x - center_x).abs() <= half_width && (y - center_y).abs() <= half_height
    }

    fn handle_command(
        &mut self,
        event_loop: &ActiveEventLoop,
        command: Command,
    ) -> Result<Value, IpcError> {
        let name = command.name();
        self.last_activity = Instant::now();
        match command {
            Command::Status => {
                let snapshot = self.snapshot();
                serde_json::to_value(snapshot)
                    .map_err(|error| IpcError::internal(error.to_string()))
            }
            Command::Show(args) => match show_action(&args, self.now_ms())? {
                ShowAction::Collapse => {
                    self.collapse();
                    Ok(json!("ok"))
                }
                ShowAction::Request(request) => {
                    self.set_state_request(request);
                    self.show();
                    Ok(json!("ok"))
                }
            },
            Command::Hide => {
                self.hide();
                Ok(json!("ok"))
            }
            Command::Collapse => {
                self.collapse();
                Ok(json!("ok"))
            }
            Command::Notify(args) => Ok(self.notify(args)),
            Command::SetRgb(args) => self.set_rgb(args),
            Command::CycleRgb => Ok(self.cycle_rgb()),
            Command::GetConfig(args) => config_section(&self.config, args.section.as_deref()),
            Command::ReloadConfig => self.reload(event_loop),
            Command::Quit => {
                self.pending_exit = Some(Instant::now() + QUIT_GRACE);
                Ok(json!("ok"))
            }
            _ => Err(IpcError::not_supported(format!(
                "`{name}` is not available in this build"
            ))),
        }
    }

    fn notify(&mut self, _args: NotifyArgs) -> Value {
        let now = self.now_ms();
        let request = StateRequest::timed(IslandState::Notify, TransitionSource::Ipc)
            .with_duration_from(now, 5_000);
        self.set_state_request(request);
        self.show();
        json!(format!("notify-{now}"))
    }

    fn set_rgb(&mut self, args: SetRgbArgs) -> Result<Value, IpcError> {
        let mode = RgbMode::from_id(&args.mode).ok_or_else(|| {
            IpcError::invalid_argument(format!("unknown RGB mode `{}`", args.mode))
        })?;
        if let Some(color) = args.color {
            if lumina_core::color::parse_hex(&color).is_none() {
                return Err(IpcError::invalid_argument("color must be #rgb or #rrggbb"));
            }
            self.config.rgb.primary = color;
        }
        if let Some(speed) = args.speed {
            self.config.rgb.speed = speed;
        }
        self.config.rgb.mode = mode;
        self.color.reconfigure(&self.config);
        self.emit_rgb_changed();
        self.dirty = true;
        self.show();
        Ok(json!("ok"))
    }

    fn cycle_rgb(&mut self) -> Value {
        let current = self.config.rgb.mode;
        let index = RgbMode::ALL
            .iter()
            .position(|mode| *mode == current)
            .unwrap_or(0);
        let next = RgbMode::ALL[(index + 1) % RgbMode::ALL.len()];
        self.config.rgb.mode = next;
        self.color.reconfigure(&self.config);
        self.emit_rgb_changed();
        self.dirty = true;
        self.show();
        json!(next.id())
    }

    fn emit_rgb_changed(&self) {
        let _ = self.events.emit(&Event::rgb_changed(&RgbSnapshot {
            mode: self.config.rgb.mode.id().to_string(),
            color: self.color.palette().glow.to_hex(),
            speed: self.config.rgb.speed,
        }));
    }

    fn reload(&mut self, event_loop: &ActiveEventLoop) -> Result<Value, IpcError> {
        let config = match load_config(&self.config_path) {
            Ok(config) => config,
            Err(error) => {
                let message = error.to_string();
                let _ = self.events.emit(&Event::config_error(
                    self.config_path.display().to_string(),
                    &message,
                ));
                return Err(IpcError::invalid_argument(message));
            }
        };

        self.apply_config(event_loop, config)?;
        let _ = self.events.emit(&Event::config_reloaded("default"));
        Ok(json!("ok"))
    }

    fn apply_config(
        &mut self,
        event_loop: &ActiveEventLoop,
        config: Config,
    ) -> Result<(), IpcError> {
        let (placement, monitor_name) =
            resolve_placement(event_loop, &config).map_err(IpcError::internal)?;
        self.config = config;
        self.color.reconfigure(&self.config);
        self.morph.apply_motion(&self.config.motion);
        self.render_loop = RenderLoop::new(
            self.config.advanced.target_fps.clamp(1, 240),
            self.config.advanced.vsync,
        );

        let (width, height) = placement.size();
        if let Some(window) = &self.window {
            window.set_outer_position(PhysicalPosition::new(
                placement.bounds().x.round() as i32,
                placement.bounds().y.round() as i32,
            ));
            let _ = window.request_inner_size(PhysicalSize::new(
                width.max(MIN_SURFACE as i32) as u32,
                height.max(MIN_SURFACE as i32) as u32,
            ));
        }
        if let Some(gpu) = &mut self.gpu {
            gpu.resize(
                width.max(MIN_SURFACE as i32) as u32,
                height.max(MIN_SURFACE as i32) as u32,
            );
        }
        self.placement = Some(placement);
        self.monitor_name = monitor_name;
        self.update_click_through();
        self.dirty = true;
        Ok(())
    }

    fn snapshot(&self) -> StateSnapshot {
        let geometry = self.placement.map(|placement| {
            let scale = placement.geometry.scale;
            let bounds = placement.bounds();
            GeometrySnapshot {
                x: bounds.x / scale,
                y: bounds.y / scale,
                width: bounds.width / scale,
                height: bounds.height / scale,
                scale,
                monitor: self.monitor_name.clone(),
            }
        });

        StateSnapshot {
            version: env!("CARGO_PKG_VERSION").to_string(),
            state: self.state.current().id().to_string(),
            visible: self.visible,
            pinned: self.state.requests().iter().any(|request| request.pinned),
            profile: "default".to_string(),
            geometry,
            rgb: RgbSnapshot {
                mode: self.config.rgb.mode.id().to_string(),
                color: self.color.palette().glow.to_hex(),
                speed: self.config.rgb.speed,
            },
            media: None,
            uptime_ms: self.started.elapsed().as_millis() as u64,
        }
    }

    fn auto_hide(&mut self) {
        if !self.config.general.auto_hide
            || !self.visible
            || self.pointer_inside
            || self.state.current() != IslandState::Collapsed
        {
            return;
        }
        let delay = Duration::from_millis(u64::from(self.config.general.auto_hide_delay_ms));
        if self.last_activity.elapsed() >= delay {
            self.hide();
        }
    }

    fn schedule(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(deadline) = self.pending_exit {
            if Instant::now() >= deadline {
                event_loop.exit();
                return;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            return;
        }

        if let Some(message) = self.guard.as_ref().and_then(InstanceGuard::take_activation) {
            if message.trim() == "show" {
                self.activate();
            }
        }

        self.auto_hide();

        let needs_frame = self.visible && (self.dirty || self.animating);
        if needs_frame {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            let fps = self.config.advanced.target_fps.clamp(1, 240);
            let interval = Duration::from_secs_f32(1.0 / fps as f32);
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + interval));
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + ACTIVATION_POLL));
        }
    }
}

impl ApplicationHandler<RuntimeEvent> for RuntimeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(error) = self.initialize(event_loop) {
            self.fatal = Some(error);
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.draw(event_loop),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                self.dirty = true;
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = window.inner_size();
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                self.dirty = true;
                window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let inside = self.pointer_is_inside(position.x, position.y);
                self.set_pointer_inside(inside);
            }
            WindowEvent::CursorLeft { .. } => self.set_pointer_inside(false),
            WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } if self.pointer_inside && self.state.current() != IslandState::Collapsed => {
                self.collapse();
            }
            WindowEvent::Occluded(false) => {
                self.dirty = true;
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RuntimeEvent) {
        let result = self.handle_command(event_loop, event.command);
        let _ = event.reply.send(result);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.schedule(event_loop);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(handle) = self.ipc.take() {
            handle.shutdown();
        }
    }
}

fn resolve_placement(
    event_loop: &ActiveEventLoop,
    config: &Config,
) -> Result<(Placement, Option<String>), String> {
    let monitors: Vec<MonitorHandle> = event_loop.available_monitors().collect();
    let primary = event_loop.primary_monitor();
    let primary_index = primary
        .as_ref()
        .and_then(|handle| {
            monitors
                .iter()
                .position(|monitor| same_monitor(monitor, handle))
        })
        .unwrap_or(0);

    let selected_index = match config.general.monitor {
        MonitorSelector::Primary => primary_index,
        MonitorSelector::Cursor => {
            if primary.is_none() {
                eprintln!("lumina: no primary monitor was reported; using monitor 0");
            }
            primary_index
        }
        MonitorSelector::Index(index) => {
            let index = index as usize;
            if index < monitors.len() {
                index
            } else {
                eprintln!(
                    "lumina: monitor index {index} does not exist; using the primary monitor"
                );
                primary_index
            }
        }
    };

    let monitor = monitors
        .get(selected_index)
        .ok_or_else(|| "no monitors are available".to_string())?;
    let position = monitor.position();
    let size = monitor.size();
    let scale = ScaleFactor::new(monitor.scale_factor() as f32).value();
    let monitor_rect = Rect::new(
        position.x as Scalar,
        position.y as Scalar,
        size.width as Scalar,
        size.height as Scalar,
    );
    let logical_size = maximal_logical_size(config);

    Ok((
        compute_placement(
            monitor_rect,
            selected_index,
            scale,
            config.general.position,
            logical_size,
            config.general.offset_x,
            config.general.offset_y,
        ),
        monitor.name(),
    ))
}

fn same_monitor(left: &MonitorHandle, right: &MonitorHandle) -> bool {
    left.position() == right.position()
        && left.size() == right.size()
        && left.scale_factor().to_bits() == right.scale_factor().to_bits()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_backend_order_is_platform_appropriate() {
        let names: Vec<_> = backend_candidates(GpuBackend::Auto)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_ne!(names.len(), 0);
        if cfg!(target_os = "windows") {
            assert_eq!(names, ["Vulkan", "DX12", "GL"]);
        }
    }

    #[test]
    fn transparent_alpha_modes_are_preferred_in_order() {
        assert_eq!(
            choose_alpha_mode(&[
                CompositeAlphaMode::Opaque,
                CompositeAlphaMode::PostMultiplied,
                CompositeAlphaMode::PreMultiplied,
            ]),
            Some(CompositeAlphaMode::PreMultiplied)
        );
        assert_eq!(choose_alpha_mode(&[CompositeAlphaMode::Opaque]), None);
    }

    #[test]
    fn present_mode_respects_vsync_and_capabilities() {
        assert_eq!(
            choose_present_mode(&[PresentMode::Immediate], true),
            PresentMode::Fifo
        );
        assert_eq!(
            choose_present_mode(&[PresentMode::Mailbox], false),
            PresentMode::Mailbox
        );
        assert_eq!(
            choose_present_mode(&[PresentMode::Fifo], false),
            PresentMode::Fifo
        );
    }

    #[test]
    fn padded_size_covers_the_widest_state_and_glow() {
        let mut config = Config::default();
        config.appearance.glow_spread = 12.0;
        let (width, height) = maximal_logical_size(&config);
        let widest = IslandState::ALL
            .into_iter()
            .map(|state| state.geometry().full_width())
            .fold(0.0, f32::max);
        assert!(width >= widest + 24.0);
        assert!(height >= 24.0);
    }

    #[test]
    fn show_defaults_to_a_pinned_media_request() {
        let action = show_action(
            &ShowArgs {
                widget: None,
                duration_ms: None,
                pin: None,
            },
            100,
        )
        .expect("show request");
        let ShowAction::Request(request) = action else {
            panic!("expected an expansion request");
        };
        assert_eq!(request.state, IslandState::Media);
        assert!(request.pinned);
        assert_eq!(request.expires_at_ms, None);
    }

    #[test]
    fn show_honours_unpinned_duration_and_rejects_hover() {
        let action = show_action(
            &ShowArgs {
                widget: Some("clock".to_string()),
                duration_ms: None,
                pin: Some(false),
            },
            100,
        )
        .expect("show request");
        let ShowAction::Request(request) = action else {
            panic!("expected an expansion request");
        };
        assert_eq!(request.state, IslandState::Clock);
        assert!(!request.pinned);
        assert_eq!(request.expires_at_ms, Some(5_100));

        assert!(show_action(
            &ShowArgs {
                widget: Some("hover".to_string()),
                duration_ms: None,
                pin: None,
            },
            0,
        )
        .is_err());
    }

    #[test]
    fn collapsing_through_show_is_an_explicit_action() {
        let action = show_action(
            &ShowArgs {
                widget: Some("collapsed".to_string()),
                duration_ms: None,
                pin: None,
            },
            0,
        )
        .expect("show request");
        assert!(matches!(action, ShowAction::Collapse));
    }

    #[test]
    fn config_sections_are_addressed_by_dots() {
        let config = Config::default();
        let value = config_section(&config, Some("appearance")).expect("section exists");
        assert!(value.is_object());
        assert!(config_section(&config, Some("appearance.missing")).is_err());
        assert!(config_section(&config, None).is_ok());
    }

    #[test]
    fn reduced_motion_snaps_the_morph() {
        let motion = MotionConfig {
            reduced: true,
            ..MotionConfig::default()
        };
        let mut morph = Morph::new(IslandState::Collapsed, &motion);
        morph.target(IslandState::Media, &motion);
        assert!(!morph.is_animating());
        assert_eq!(morph.geometry(), IslandState::Media.geometry());
    }

    #[test]
    fn animated_palettes_only_run_for_visible_expanded_widgets() {
        let config = Config::default();
        assert!(!palette_is_animating(&config, IslandState::Collapsed));
        assert!(!palette_is_animating(&config, IslandState::Hover));
        assert!(palette_is_animating(&config, IslandState::Media));
    }
}
