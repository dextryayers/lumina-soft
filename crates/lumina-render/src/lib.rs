//! GPU rendering for Lumina.
//!
//! # What this crate is for
//!
//! The island is not a widget tree. It is a single signed distance field
//! evaluated per pixel in a fragment shader, driven by a handful of scalars that
//! a spring solves toward a target. That makes the correctness of the shader the
//! single most important rendering decision: if the SDF is wrong, the shape is
//! subtly incorrect in ways nobody can name but everybody can see.
//!
//! # The three passes
//!
//! | Pass | What it draws | Cost |
//! |---|---|---|
//! | Island body | SDF fill, border, glow in one fragment shader | One full-target quad |
//! | Content | Album art, icons, text, progress, controls | Instanced quads |
//! | Spectrum | Bars or waveform, 32 bands | One instanced draw |
//!
//! # The dirty flag rule
//!
//! The render loop does not run at 60 fps forever. It runs only while something is
//! animating, audio is flowing, or a widget is updating. When nothing is new the
//! loop blocks instead of spinning. This is the single most important rule for
//! the CPU budget: a process that submits 60 frames a second forever will sit at
//! a few percent of a core no matter how cheap each frame is.
//!
//! # Shader validation
//!
//! Every WGSL source is validated at startup through `naga` before it reaches
//! the driver. A shader that fails validation is caught, logged, and replaced
//! with a solid fill fallback rather than taking the application down with it.

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

pub mod device;
pub mod pipeline;
pub mod render;
pub mod shaders;
pub mod uniforms;

pub use device::{GpuDevice, GpuError, GpuInfo};
pub use render::{FrameStats, RenderLoop};

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
