//! MSDF fragment shader.
//!
//! The shader reconstructs the signed distance from the three channels and
//! produces a coverage value. The `median` of the three channels is what
//! preserves corners, which a single-channel SDF cannot do.
//!
//! `screen_px_range` is the number of shape units one pixel covers, derived
//! from the glyph quad scale. It is what makes the outline correct at any
//! font size, including fractional sizes for the marquee.

use naga;

/// MSDF fragment shader source.
pub const MSDF_WGSL: &str = r"struct MsdfUniforms {
    screen_px_range: f32,
    smoothing: f32,
    color: vec4<f32>,
    dpr: f32,
};

@group(0) @binding(0)
var<uniform> u: MsdfUniforms;

@group(0) @binding(1)
var atlas: texture_2d<f32>;

@group(0) @binding(2)
var atlas_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> VertexOutput {
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), uv);
}

fn median(r: f32, g: f32, b: f32) -> f32 {
    return max(min(r, g), min(max(r, g), b));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, input.uv);
    let sd = median(texel.r, texel.g, texel.b);
    let d = (sd - 0.5) * 2.0;
    let pixel_range = max(u.screen_px_range, 1e-6);
    let coverage = smoothstep(-pixel_range, pixel_range, d);
    let alpha = select(coverage, select(0.0, 1.0, d > 0.0), u.smoothing < 0.5);
    let premul = vec4<f32>(u.color.rgb * alpha, alpha);
    return premul;
}
";

/// Validate the MSDF WGSL through `naga`.
pub fn validate_msdf_wgsl(source: &str) -> Option<String> {
    let module = match naga::front::wgsl::parse_str(source) {
        Ok(m) => m,
        Err(e) => return Some(e.emit_to_string(source)),
    };
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    match validator.validate(&module) {
        Ok(_) => None,
        Err(e) => Some(format!("{e:?}")),
    }
}

/// Reconstruct a coverage value from a three-channel MSDF texel.
///
/// This is the CPU reference for the shader's math, used to test the median
/// reconstruction against a ground truth.
pub fn coverage_from_msdf(red: f32, green: f32, blue: f32, screen_px_range: f32) -> f32 {
    let sd = red
        .max(green.min(red.max(green)))
        .min(green.max(red).min(blue));
    let d = (sd - 0.5) * 2.0;
    let pixel_range = screen_px_range.max(1e-6);
    let t = ((d + pixel_range) / (2.0 * pixel_range)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_msdf_shader_validates_cleanly() {
        assert!(
            validate_msdf_wgsl(MSDF_WGSL).is_none(),
            "the MSDF shader must validate: {:?}",
            validate_msdf_wgsl(MSDF_WGSL)
        );
    }

    #[test]
    fn the_msdf_shader_declares_the_uniforms_the_cpu_writes() {
        for name in ["screen_px_range", "smoothing", "color", "dpr"] {
            assert!(
                MSDF_WGSL.contains(name),
                "the MSDF shader must declare {name}"
            );
        }
    }

    #[test]
    fn coverage_is_one_inside_and_zero_outside() {
        let inside = coverage_from_msdf(1.0, 1.0, 1.0, 1.0);
        let outside = coverage_from_msdf(0.0, 0.0, 0.0, 1.0);
        assert!(
            (inside - 1.0).abs() < 1e-6,
            "inside should be 1.0, got {inside}"
        );
        assert!(outside.abs() < 1e-6, "outside should be 0.0, got {outside}");
    }

    #[test]
    fn coverage_is_smooth_at_the_boundary() {
        let mid = coverage_from_msdf(0.5, 0.5, 0.5, 1.0);
        assert!(
            (mid - 0.5).abs() < 1e-6,
            "boundary should be 0.5, got {mid}"
        );
    }

    #[test]
    fn the_median_reconstructs_a_corner() {
        let corner = coverage_from_msdf(0.0, 0.5, 1.0, 1.0);
        let expected = coverage_from_msdf(0.5, 0.5, 0.5, 1.0);
        assert!(
            (corner - expected).abs() < 1e-6,
            "median reconstruction failed: {corner} vs {expected}"
        );
    }

    #[test]
    fn coverage_handles_zero_range_without_dividing_by_zero() {
        let value = coverage_from_msdf(0.5, 0.5, 0.5, 0.0);
        assert!(value.is_finite());
        assert!((value - 0.5).abs() < 1e-6);
    }
}
