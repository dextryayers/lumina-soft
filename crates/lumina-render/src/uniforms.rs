//! The uniform buffer layout shared by the shader and the CPU.
//!
//! The field order here must match `assets/shaders/island.wgsl` exactly, because
//! `wgpu` uploads the struct as a byte buffer. A mismatch shows up as a silently
//! wrong render, which is the worst kind of bug to track down.
//!
//! The struct is 144 bytes, comfortably inside the 64 KB uniform buffer limit,
//! and is uploaded with a single `write_buffer` per frame.

use lumina_core::color::Palette;
use lumina_core::geometry::{Geometry, Scalar, Vec2};
use lumina_core::state::IslandState;

/// Everything the island shader needs for one frame.
///
/// Marked `#[repr(C)]` so the field order is guaranteed. Every field is `f32`
/// or a fixed array of `f32`, so there is no padding surprise.
///
/// The struct is 144 bytes, comfortably inside the 64 KB uniform buffer limit,
/// and is uploaded with a single `write_buffer` per frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct IslandUniforms {
    /// Half width of the lower edge.
    pub half_bottom: Scalar,
    /// Half width of the upper edge.
    pub half_top: Scalar,
    /// Half height.
    pub half_h: Scalar,
    /// Corner radius.
    pub radius: Scalar,

    /// Premultiplied background fill.
    pub fill_rgba: [f32; 4],
    /// Premultiplied gradient stop.
    pub fill_accent_rgba: [f32; 4],
    /// Premultiplied border stroke.
    pub border_rgba: [f32; 4],
    /// Border thickness in logical pixels.
    pub border_width: Scalar,
    /// Glow color, not premultiplied.
    pub glow_rgb: [f32; 3],
    /// Glow strength.
    pub glow_intensity: Scalar,
    /// Glow decay distance. Larger is tighter.
    pub glow_falloff: Scalar,
    /// Glow extent beyond the shape.
    pub glow_spread: Scalar,
    /// Dither noise amplitude.
    pub noise_strength: Scalar,

    /// Island center in target pixels.
    pub origin_px: Vec2,
    /// Reciprocal of the shape's half extents in pixels.
    pub inv_extent: Vec2,
    /// Target size in pixels.
    pub resolution: Vec2,
    /// Device pixel ratio.
    pub dpr: Scalar,
    /// Seconds since startup.
    pub time_seconds: Scalar,

    /// Content opacity, driven by the content fade spring.
    pub content_opacity: Scalar,
    /// Gradient strength between the fill and accent color.
    pub gradient_strength: Scalar,
    /// Trailing padding to the uniform struct's 16 byte alignment.
    pub padding: [f32; 2],
}

/// The screen-space parameters for one frame's uniforms.
///
/// Groups the values that change every frame but are not part of the island
/// geometry: where the island is, how large the output surface is, how far
/// along the frame is, and how visible the content is.
#[must_use]
#[derive(Debug, Clone, Copy)]
pub struct FrameParams {
    /// The island origin, in device pixels.
    pub position: Vec2,
    /// The output resolution, in device pixels.
    pub resolution: Vec2,
    /// The device pixel ratio. Zero or negative falls back to `1.0`.
    pub dpr: Scalar,
    /// Seconds since startup, for the animated gradient.
    pub time_seconds: Scalar,
    /// Content opacity, clamped to `0.0..=1.0`.
    pub content_opacity: Scalar,
}

impl IslandUniforms {
    /// Total size in bytes, which is what `wgpu` needs for the buffer.
    pub const SIZE: u64 = std::mem::size_of::<Self>() as u64;

    /// Build the uniforms for one frame.
    ///
    /// `geometry` is the spring's current interpolated shape, `palette` is the
    /// resolved RGB palette, and `appearance` carries the border and glow
    /// settings. All values are in logical pixels except `origin_px` and
    /// `resolution`, which are in device pixels.
    #[must_use]
    pub fn for_frame(
        geometry: Geometry,
        palette: &Palette,
        appearance: &lumina_config::AppearanceConfig,
        params: FrameParams,
    ) -> Self {
        let size = geometry.size();
        let dpr = if params.dpr.is_finite() && params.dpr > 0.0 {
            params.dpr
        } else {
            1.0
        };
        let scale = dpr;

        let effective_radius = geometry.effective_radius();

        Self {
            half_bottom: geometry.half_bottom,
            half_top: geometry.half_top,
            half_h: geometry.half_h(),
            radius: effective_radius,
            fill_rgba: palette.fill.to_premultiplied(),
            fill_accent_rgba: palette.fill_accent.to_premultiplied(),
            border_rgba: palette.border.to_premultiplied(),
            border_width: appearance.border_width.max(0.0) * scale,
            glow_rgb: {
                let rgb = palette.glow.to_oklab().to_srgb();
                [rgb[0], rgb[1], rgb[2]]
            },
            glow_intensity: palette.glow_intensity.max(0.0),
            glow_falloff: appearance.glow_falloff.max(0.5) * scale,
            glow_spread: appearance.glow_spread.max(0.0) * scale,
            noise_strength: appearance.noise_strength.max(0.0),
            origin_px: params.position,
            inv_extent: Vec2::new(
                if size.width > 0.0 {
                    1.0 / (size.width * 0.5 * scale)
                } else {
                    0.0
                },
                if size.height > 0.0 {
                    1.0 / (size.height * 0.5 * scale)
                } else {
                    0.0
                },
            ),
            resolution: params.resolution,
            dpr,
            time_seconds: params.time_seconds,
            content_opacity: params.content_opacity.clamp(0.0, 1.0),
            gradient_strength: appearance.fill_gradient.clamp(0.0, 1.0),
            padding: [0.0; 2],
        }
    }

    /// Bytes for the `wgpu` uniform buffer.
    ///
    /// SAFETY: `Self` is `#[repr(C)]` and contains only `f32` and arrays of
    /// `f32`, so its bytes are a valid `wgpu` uniform layout.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(std::ptr::from_ref(self).cast::<u8>(), Self::SIZE as usize)
        }
    }

    /// The shader's expected layout, for the pipeline descriptor.
    pub fn buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: IslandUniforms::SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[],
        }
    }
}

/// The geometry and content opacity for one island state.
///
/// Returned by the animation orchestrator so the render loop never has to know
/// how a state maps to a shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateFrame {
    /// The state's default geometry.
    pub geometry: Geometry,
    /// The state's content opacity.
    pub content_opacity: Scalar,
}

impl StateFrame {
    /// The frame for a state.
    #[must_use]
    pub fn for_state(state: IslandState) -> Self {
        Self {
            geometry: state.geometry(),
            content_opacity: state.content_opacity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_size_matches_the_shader_expectation() {
        // The shader declares a struct that must match this layout. If this
        // grows past 64 KB the upload fails, and if it shrinks the shader reads
        // garbage.
        assert_eq!(
            IslandUniforms::SIZE,
            144,
            "the uniform struct must be 144 bytes"
        );
    }

    #[test]
    fn as_bytes_returns_exactly_one_uniform() {
        let uniforms = IslandUniforms::default();
        assert_eq!(uniforms.as_bytes().len(), IslandUniforms::SIZE as usize);
    }

    #[test]
    fn the_buffer_layout_is_an_instance_step_with_no_attributes() {
        let layout = IslandUniforms::buffer_layout();
        assert_eq!(layout.array_stride, IslandUniforms::SIZE);
        assert_eq!(layout.attributes.len(), 0);
    }

    #[test]
    fn inv_extent_is_the_reciprocal_of_the_half_size() {
        let appearance = lumina_config::AppearanceConfig::default();
        let palette = Palette::default();
        let uniforms = IslandUniforms::for_frame(
            Geometry::default(),
            &palette,
            &appearance,
            FrameParams {
                position: Vec2::new(960.0, 540.0),
                resolution: Vec2::new(1920.0, 1080.0),
                dpr: 1.0,
                time_seconds: 0.0,
                content_opacity: 1.0,
            },
        );

        // The default island is 124 px wide, so half is 62 px.
        assert!((uniforms.inv_extent.x - 1.0 / 62.0).abs() < 1e-4);
        assert!((uniforms.inv_extent.y - 1.0 / 18.0).abs() < 1e-4);
    }

    #[test]
    fn inv_extent_does_not_divide_by_zero() {
        let appearance = lumina_config::AppearanceConfig::default();
        let palette = Palette::default();
        let uniforms = IslandUniforms::for_frame(
            Geometry::new(0.0, 0.0, 0.0, 0.0),
            &palette,
            &appearance,
            FrameParams {
                position: Vec2::ZERO,
                resolution: Vec2::new(1920.0, 1080.0),
                dpr: 1.0,
                time_seconds: 0.0,
                content_opacity: 1.0,
            },
        );
        assert!(uniforms.inv_extent.x.is_finite());
        assert!(uniforms.inv_extent.y.is_finite());
        assert_eq!(uniforms.inv_extent.x, 0.0);
    }

    #[test]
    fn a_zero_dpr_falls_back_to_one() {
        let appearance = lumina_config::AppearanceConfig::default();
        let palette = Palette::default();
        let uniforms = IslandUniforms::for_frame(
            Geometry::default(),
            &palette,
            &appearance,
            FrameParams {
                position: Vec2::ZERO,
                resolution: Vec2::new(1920.0, 1080.0),
                dpr: 0.0,
                time_seconds: 0.0,
                content_opacity: 1.0,
            },
        );
        assert_eq!(uniforms.dpr, 1.0);
    }

    #[test]
    fn every_value_is_finite_for_a_default_frame() {
        let appearance = lumina_config::AppearanceConfig::default();
        let palette = Palette::default();
        let uniforms = IslandUniforms::for_frame(
            Geometry::default(),
            &palette,
            &appearance,
            FrameParams {
                position: Vec2::new(960.0, 540.0),
                resolution: Vec2::new(1920.0, 1080.0),
                dpr: 1.0,
                time_seconds: 0.0,
                content_opacity: 1.0,
            },
        );

        let values = [
            uniforms.half_bottom,
            uniforms.half_top,
            uniforms.half_h,
            uniforms.radius,
            uniforms.border_width,
            uniforms.glow_intensity,
            uniforms.glow_falloff,
            uniforms.glow_spread,
            uniforms.noise_strength,
            uniforms.dpr,
            uniforms.time_seconds,
            uniforms.content_opacity,
            uniforms.gradient_strength,
        ];
        for value in values {
            assert!(
                value.is_finite(),
                "a non-finite uniform would corrupt the shape"
            );
        }
    }

    #[test]
    fn every_state_produces_a_valid_frame() {
        for state in IslandState::ALL {
            let frame = StateFrame::for_state(state);
            assert!(frame.geometry.height > 0.0, "{state} has no height");
            assert!(frame.content_opacity >= 0.0 && frame.content_opacity <= 1.0);
        }
    }

    #[test]
    fn the_geometry_is_preserved_into_the_uniforms() {
        let appearance = lumina_config::AppearanceConfig::default();
        let palette = Palette::default();
        let geometry = Geometry::new(210.0, 200.0, 50.0, 24.0);
        let uniforms = IslandUniforms::for_frame(
            geometry,
            &palette,
            &appearance,
            FrameParams {
                position: Vec2::ZERO,
                resolution: Vec2::new(1920.0, 1080.0),
                dpr: 1.0,
                time_seconds: 0.0,
                content_opacity: 1.0,
            },
        );
        assert_eq!(uniforms.half_top, 210.0);
        assert_eq!(uniforms.half_bottom, 200.0);
        assert_eq!(uniforms.half_h, 25.0);
        // The radius is clamped to half the height.
        assert_eq!(uniforms.radius, 24.0);
    }
}
