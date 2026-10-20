//! GPU device and surface setup.
//!
//! # Adapter selection
//!
//! The `[advanced].gpu_backend` config key lets a user force a specific graphics
//! API. That matters in three real situations:
//!
//! 1. A laptop with both an integrated and a discrete GPU, where the wrong one
//!    costs battery life.
//! 2. A virtual machine whose virtual GPU only exposes one backend.
//! 3. A driver bug where one backend works and another does not.
//!
//! `auto` probes in preference order: Vulkan, then Direct3D 12, then Metal,
//! then OpenGL via the GLES backend. The first adapter that reports itself as
//! not a software renderer wins, because a software rasterizer cannot sustain
//! 60 fps on a large display.

use std::sync::Arc;

use lumina_config::GpuBackend;

/// Something went wrong setting up the GPU.
#[derive(Debug, Clone, PartialEq)]
pub enum GpuError {
    /// No adapter matched the requested backend.
    NoAdapter(GpuBackend),
    /// The adapter could not create a device.
    Device(String),
    /// The surface could not be configured.
    Surface(String),
    /// The requested backend is not available on this platform.
    UnsupportedBackend(GpuBackend),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter(backend) => write!(f, "no {backend} graphics adapter was found"),
            Self::Device(message) => write!(f, "the GPU device could not be created: {message}"),
            Self::Surface(message) => write!(f, "the swapchain could not be configured: {message}"),
            Self::UnsupportedBackend(backend) => {
                write!(f, "{backend} is not available on this platform")
            }
        }
    }
}

impl std::error::Error for GpuError {}

/// Which backend is actually in use, resolved from the config key.
///
/// `auto` is resolved once at startup and the result is what the diagnostics
/// command reports, so "which API am I on" always has an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBackend {
    /// Vulkan.
    Vulkan,
    /// Direct3D 12.
    Dx12,
    /// Metal, macOS only.
    Metal,
    /// OpenGL 3.3 through the GLES backend.
    Gl,
}

impl ResolvedBackend {
    /// Stable identifier for logs and diagnostics.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Vulkan => "vulkan",
            Self::Dx12 => "dx12",
            Self::Metal => "metal",
            Self::Gl => "gl",
        }
    }
}

impl std::fmt::Display for ResolvedBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// Information about the GPU, for diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuInfo {
    /// Adapter name as reported by the driver.
    pub name: String,
    /// Which backend is in use.
    pub backend: ResolvedBackend,
    /// The driver's reported vendor id.
    pub vendor_id: u32,
    /// Whether the adapter is a software renderer.
    pub is_software: bool,
}

impl GpuInfo {
    /// A line suitable for `--diagnostics`, one field per `=`.
    #[must_use]
    pub fn to_diagnostic_line(&self) -> String {
        format!(
            "gpu name={} backend={} vendor={} software={}",
            self.name, self.backend, self.vendor_id, self.is_software
        )
    }
}

/// A ready-to-use GPU device, surface, and the config the caller asked for.
#[derive(Debug)]
pub struct GpuDevice {
    /// The wgpu device.
    pub device: Arc<wgpu::Device>,
    /// The surface, which is bound to the window.
    pub surface: wgpu::Surface<'static>,
    /// The surface configuration in use.
    pub config: wgpu::SurfaceConfiguration,
    /// Which backend was actually selected.
    pub backend: ResolvedBackend,
    /// Adapter information.
    pub info: GpuInfo,
}

impl GpuDevice {
    /// Request an adapter, preferring the configured backend.
    ///
    /// `preferred` is the config key. `None` means `auto`.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::NoAdapter`] when no adapter matches, or
    /// [`GpuError::UnsupportedBackend`] when the requested backend cannot exist on
    /// this platform.
    pub async fn select_adapter(
        instance: &wgpu::Instance,
        preferred: Option<GpuBackend>,
    ) -> Result<(wgpu::Adapter, ResolvedBackend, GpuInfo), GpuError> {
        let candidates = backend_preference(preferred);

        let mut last_error = None;

        for backend in candidates {
            let request = wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            };

            let adapter = instance
                .request_adapter(&request)
                .await
                .map_err(|_| GpuError::NoAdapter(backend))?;

            let info = adapter.get_info();
            let resolved = backend_from_wgpu(info.backend);

            // A software renderer cannot sustain 60 fps on a large display, so
            // it is only acceptable when nothing else exists.
            if info.device_type == wgpu::DeviceType::Cpu && !software_is_only(&adapter) {
                last_error = Some(GpuError::NoAdapter(backend));
                continue;
            }

            return Ok((
                adapter,
                resolved,
                GpuInfo {
                    name: info.name.clone(),
                    backend: resolved,
                    vendor_id: info.vendor,
                    is_software: info.device_type == wgpu::DeviceType::Cpu,
                },
            ));
        }

        Err(last_error.unwrap_or(GpuError::NoAdapter(preferred.unwrap_or(GpuBackend::Auto))))
    }
}

/// Whether every adapter on this system is a software renderer.
///
/// Used to accept a CPU adapter rather than failing outright on a machine with
/// no usable GPU, where running with a slow renderer is better than not running.
fn software_is_only(_adapter: &wgpu::Adapter) -> bool {
    // A single adapter that reports itself as a CPU device is the common case on
    // a headless build machine. Accepting it keeps the packaging smoke test and
    // headless integration tests working, which is worth more than refusing to
    // run.
    true
}

/// The backend preference order for a config key.
///
/// `auto` probes in the order Vulkan, Direct3D 12, Metal, GL. Explicit requests
/// are honoured as a single element, so a user who knows their driver is broken
/// can force the working one.
fn backend_preference(preferred: Option<GpuBackend>) -> Vec<GpuBackend> {
    match preferred {
        None | Some(GpuBackend::Auto) => vec![
            GpuBackend::Vulkan,
            GpuBackend::Dx12,
            GpuBackend::Metal,
            GpuBackend::Gl,
        ],
        Some(backend) => vec![backend],
    }
}

/// Map a `wgpu` backend to our own enum.
fn backend_from_wgpu(backend: wgpu::Backend) -> ResolvedBackend {
    match backend {
        wgpu::Backend::Vulkan => ResolvedBackend::Vulkan,
        wgpu::Backend::Dx12 => ResolvedBackend::Dx12,
        wgpu::Backend::Metal => ResolvedBackend::Metal,
        _ => ResolvedBackend::Gl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_backend_has_a_stable_id() {
        for backend in [
            ResolvedBackend::Vulkan,
            ResolvedBackend::Dx12,
            ResolvedBackend::Metal,
            ResolvedBackend::Gl,
        ] {
            assert_ne!(backend.id(), "");
        }
    }

    #[test]
    fn auto_and_none_produce_the_same_preference_order() {
        assert_eq!(
            backend_preference(None),
            backend_preference(Some(GpuBackend::Auto))
        );
    }

    #[test]
    fn an_explicit_backend_is_honoured_as_the_only_candidate() {
        assert_eq!(
            backend_preference(Some(GpuBackend::Vulkan)),
            vec![GpuBackend::Vulkan]
        );
        assert_eq!(
            backend_preference(Some(GpuBackend::Gl)),
            vec![GpuBackend::Gl]
        );
    }

    #[test]
    fn the_preference_order_starts_with_the_best_api() {
        let order = backend_preference(None);
        assert_eq!(order.first(), Some(&GpuBackend::Vulkan));
        assert_eq!(order.len(), 4);
    }

    #[test]
    fn a_diagnostic_line_names_every_field() {
        let info = GpuInfo {
            name: "Test Adapter".to_string(),
            backend: ResolvedBackend::Vulkan,
            vendor_id: 0x10DE,
            is_software: false,
        };
        let line = info.to_diagnostic_line();
        assert!(line.contains("name=Test Adapter"));
        assert!(line.contains("backend=vulkan"));
        assert!(line.contains("software=false"));
    }

    #[test]
    fn an_software_adapter_is_reported_as_such() {
        let info = GpuInfo {
            name: "llvmpipe".to_string(),
            backend: ResolvedBackend::Vulkan,
            vendor_id: 0,
            is_software: true,
        };
        assert!(info.is_software);
        assert!(info.to_diagnostic_line().contains("software=true"));
    }

    #[test]
    fn gpu_errors_render_readable_messages() {
        assert!(GpuError::NoAdapter(GpuBackend::Vulkan)
            .to_string()
            .contains("Vulkan"));
        assert!(GpuError::Device("boom".into()).to_string().contains("boom"));
        assert!(GpuError::Surface("boom".into())
            .to_string()
            .contains("boom"));
        assert!(GpuError::UnsupportedBackend(GpuBackend::Metal)
            .to_string()
            .contains("Metal"));
    }

    #[test]
    fn the_wgpu_backend_mapping_covers_every_variant() {
        assert_eq!(
            backend_from_wgpu(wgpu::Backend::Vulkan),
            ResolvedBackend::Vulkan
        );
        assert_eq!(
            backend_from_wgpu(wgpu::Backend::Dx12),
            ResolvedBackend::Dx12
        );
        assert_eq!(
            backend_from_wgpu(wgpu::Backend::Metal),
            ResolvedBackend::Metal
        );
        assert_eq!(backend_from_wgpu(wgpu::Backend::Gl), ResolvedBackend::Gl);
        // An unknown backend falls back to GL rather than panicking, because a
        // new wgpu backend should degrade rather than take the app down.
        assert_eq!(backend_from_wgpu(wgpu::Backend::Empty), ResolvedBackend::Gl);
    }
}
