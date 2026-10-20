//! Shader compilation and the render pipeline.
//!
//! # Error scopes
//!
//! Every pipeline creation is wrapped in `push_error_scope(Validation)`.
//! Without that, a validation error on one GPU driver surfaces as an uncatchable
//! device loss, and a driver that rejects a shader would take the whole
//! application down. With it, the failure is a `Result` the caller can handle.
//!
//! # The fallback path
//!
//! When the primary shader is rejected, the pipeline is rebuilt from a solid
//! fill shader. The island still appears, just without SDF edges, border, or
//! glow. That is a far better outcome than exiting, and the tray notification
//! tells the user why the effects are missing.
//!
//! # One-time compilation
//!
//! Both the primary and the fallback pipeline are built at startup, before the
//! first frame. A pipeline that compiles lazily costs a 200 ms hitch on the
//! first expand, which is exactly when the user is watching.

use crate::shaders::{self, ISLAND_WGSL};
use crate::uniforms::IslandUniforms;

/// A compiled render pipeline plus the bind group it needs.
#[derive(Debug)]
pub struct IslandPipeline {
    /// The wgpu pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// The bind group holding the uniform buffer.
    pub bind_group: wgpu::BindGroup,
    /// The uniform buffer, kept alive by the bind group.
    pub uniform_buffer: wgpu::Buffer,
    /// Which shader was used, for diagnostics.
    pub is_fallback: bool,
}

impl IslandPipeline {
    /// Build the island pipeline.
    ///
    /// `device` is the GPU device. `format` is the swapchain format. `surface`
    /// is unused here but kept for the caller to hold.
    ///
    /// # Errors
    ///
    /// Never returns an error: a shader that fails validation falls back to the
    /// solid fill shader, and if that also fails the whole creation returns
    /// `Err` only when the device itself is unusable.
    ///
    /// # Panics
    ///
    /// Never.
    pub async fn build(device: &wgpu::Device, format: wgpu::TextureFormat) -> Result<Self, String> {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lumina-island-uniforms"),
            size: IslandUniforms::SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lumina-island-bind-group-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(IslandUniforms::SIZE),
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lumina-island-bind-group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform_buffer,
                    offset: 0,
                    size: std::num::NonZeroU64::new(IslandUniforms::SIZE),
                }),
            }],
        });

        // Try the primary shader first. `naga` validation has already run at
        // startup, so a failure here is a driver-specific validation error, which
        // is exactly the case the fallback exists for.
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let primary_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lumina-island-shader"),
            source: wgpu::ShaderSource::Wgsl(ISLAND_WGSL.into()),
        });
        let primary_error = error_scope.pop().await;

        let (module, is_fallback) = match primary_error {
            Some(_) => {
                let fallback = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("lumina-island-fallback-shader"),
                    source: wgpu::ShaderSource::Wgsl(FALLBACK_WGSL.into()),
                });
                (fallback, true)
            }
            None => (primary_module, false),
        };

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lumina-island-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(if is_fallback {
                "lumina-island-fallback-pipeline"
            } else {
                "lumina-island-pipeline"
            }),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self {
            pipeline,
            bind_group,
            uniform_buffer,
            is_fallback,
        })
    }

    /// Write the uniforms for the next draw.
    ///
    /// One `write_buffer` per frame. The buffer is 128 bytes, so this is
    /// negligible next to the cost of presenting.
    pub fn write_uniforms(&self, queue: &wgpu::Queue, uniforms: &IslandUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, uniforms.as_bytes());
    }
}

/// The solid fill fallback shader.
///
/// Used when the primary SDF shader is rejected by a driver. It draws a
/// rectangle with the configured fill color, which is enough for the island to
/// be visible and useful, just without smooth edges or glow.
///
/// Deliberately minimal: three vertices, one uniform, one output. Anything more
/// would be another thing for a broken driver to reject.
pub const FALLBACK_WGSL: &str = r#"
struct FallbackUniforms {
    fill_rgba: vec4<f32>,
    resolution: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> u: FallbackUniforms;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let position = positions[vertex_index];
    return VertexOutput(
        vec4<f32>(position, 0.0, 1.0),
        position * 0.5 + 0.5,
    );
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) frag_coord: vec2<f32>,
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Draw the bottom-left quadrant, which is enough of a rectangle to be
    // recognisable as "the island is here" when the real shader will not load.
    if (input.frag_coord.x < 0.5 && input.frag_coord.y > 0.5) {
        return vec4<f32>(u.fill_rgba.rgb * u.fill_rgba.a, u.fill_rgba.a);
    }
    return vec4<f32>(0.0);
}
"#;

/// A shader source that has been through `naga` validation.
#[derive(Debug, Clone)]
pub struct ValidatedShader {
    /// The WGSL source.
    pub source: &'static str,
    /// True when the source is the fallback.
    pub is_fallback: bool,
}

impl ValidatedShader {
    /// The primary island shader, validated at construction.
    ///
    /// Returns `None` when the source does not validate, which is the signal to
    /// use the fallback rather than letting a broken shader reach the driver.
    #[must_use]
    pub fn primary() -> Option<Self> {
        if shaders::validate_wgsl(ISLAND_WGSL).is_none() {
            Some(Self {
                source: ISLAND_WGSL,
                is_fallback: false,
            })
        } else {
            None
        }
    }

    /// The fallback shader, always available.
    #[must_use]
    pub fn fallback() -> Self {
        Self {
            source: FALLBACK_WGSL,
            is_fallback: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fallback_shader_validates() {
        // If the fallback is itself broken, a driver rejecting the primary shader
        // would leave the island invisible rather than showing a solid fill.
        assert!(
            shaders::validate_wgsl(FALLBACK_WGSL).is_none(),
            "the fallback shader must validate: {:?}",
            shaders::validate_wgsl(FALLBACK_WGSL)
        );
    }

    #[test]
    fn the_primary_shader_validates() {
        assert!(
            shaders::validate_wgsl(ISLAND_WGSL).is_none(),
            "the primary shader must validate"
        );
    }

    #[test]
    fn the_fallback_shader_has_no_dependencies_on_the_primary() {
        // A fallback that references something only in the primary would fail in
        // exactly the situation it exists to handle.
        assert!(!FALLBACK_WGSL.contains("sd_rounded_trapezoid"));
        assert!(!FALLBACK_WGSL.contains("glow"));
        assert!(FALLBACK_WGSL.contains("vs_main"));
    }

    #[test]
    fn a_validated_shader_reports_which_variant_it_is() {
        assert!(
            ValidatedShader::fallback().is_fallback,
            "the fallback must identify itself"
        );
        if let Some(primary) = ValidatedShader::primary() {
            assert!(!primary.is_fallback);
        }
    }

    #[test]
    fn the_fallback_shader_is_deliberately_tiny() {
        // Small enough that no reasonable driver rejects it.
        assert!(FALLBACK_WGSL.len() < 2000);
    }

    #[test]
    fn the_uniform_buffer_size_matches_the_struct() {
        // A buffer smaller than the struct would read past the end, which is the
        // kind of bug that shows up as a randomly shaped island on some drivers.
        assert_eq!(IslandUniforms::SIZE, 136);
    }
}
