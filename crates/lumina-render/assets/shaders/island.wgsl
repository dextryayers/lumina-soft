// Lumina island body shader.
//
// One full-target quad. The island shape is evaluated per pixel as a signed
// distance field, and everything else (border, glow, fill) is derived from that
// field in the same fragment shader. That is what makes the shape resolution
// independent and the glow follow the shape exactly rather than being a
// separate blurred sprite.
//
// Coordinate convention: `p` is in shape space, origin at the center of the
// island, +y up. `dpr` is the device pixel ratio so one logical pixel is
// `dpr` device pixels.

struct Uniforms {
    // Geometry, in shape space (centered origin).
    half_bottom: f32,
    half_top: f32,
    half_h: f32,
    radius: f32,

    // Appearance.
    fill_rgba: vec4<f32>,       // premultiplied
    fill_accent_rgba: vec4<f32>,
    border_rgba: vec4<f32>,
    border_width: f32,
    glow_rgb: vec3<f32>,
    glow_intensity: f32,
    glow_falloff: f32,
    glow_spread: f32,
    noise_strength: f32,

    // Transform.
    origin_px: vec2<f32>,       // island center in target pixels
    inv_extent: vec2<f32>,      // 1 / shape half extents in pixels
    resolution: vec2<f32>,
    dpr: f32,
    time_seconds: f32,

    // Derived at the CPU, so the shader never has to branch on state.
    content_opacity: f32,
    gradient_strength: f32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Three vertices covering the whole target. No vertex buffer, no attributes.
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let position = positions[vertex_index];
    return VertexOutput(
        vec4<f32>(position, 0.0, 1.0),
        vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5),
    );
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) shape_coord: vec2<f32>,
}

// Signed distance to an isosceles trapezoid.
//
// Mirrors `lumina_core::sdf::sd_trapezoid` exactly. The tests in that module
// verify the Rust version against a brute-force reference, so agreement between
// the two is what keeps the on-screen shape correct.
fn sd_trapezoid(p: vec2<f32>, half_bottom: f32, half_top: f32, half_h: f32) -> f32 {
    let k1 = vec2<f32>(half_top, half_h);
    let k2 = vec2<f32>(half_top - half_bottom, 2.0 * half_h);

    // Fold to the right half plane. The trapezoid is symmetric about x = 0.
    var q = vec2<f32>(abs(p.x), p.y);

    let edge_half = select(half_bottom, half_top, q.y > 0.0);
    let ca = vec2<f32>(q.x - min(q.x, edge_half), abs(q.y) - half_h);

    let k2_len_sq = dot(k2, k2);
    let t = select(0.0, clamp(dot(k1 - q, k2) / k2_len_sq, 0.0, 1.0), k2_len_sq > 0.0);
    let cb = q - k1 + k2 * t;

    let inside = cb.x < 0.0 && ca.y < 0.0;
    let sign = select(1.0, -1.0, inside);

    return sign * sqrt(min(dot(ca, ca), dot(cb, cb)));
}

// Signed distance to a rounded isosceles trapezoid.
//
// Rounding a convex polygon is done by insetting each edge by the radius,
// forming the offset polygon, then subtracting the radius from that polygon's
// own field. For a trapezoid the offset polygon is still a trapezoid:
//   horizontal edges inset by r give half_h - r
//   slanted edges need a horizontal inset of r / cos(theta)
//     because moving inward horizontally by dx changes the perpendicular
//     distance by dx * cos(theta).
fn sd_rounded_trapezoid(
    p: vec2<f32>, half_bottom: f32, half_top: f32, half_h: f32, radius: f32
) -> f32 {
    if half_h <= 0.0 {
        return 1e9;
    }
    let r = min(radius, min(half_h, half_top * 0.5));
    if r <= 0.0 {
        return sd_trapezoid(p, half_bottom, half_top, half_h);
    }
    let slant_angle = atan2(half_top - half_bottom, 2.0 * half_h);
    let inset_x = r / max(cos(slant_angle), 0.15);
    return sd_trapezoid(p, half_bottom - inset_x, half_top - inset_x, half_h - r) - r;
}

// Anti-aliased coverage from a signed distance.
//
// `fwidth(d)` is how many shape units one pixel covers at this position, which
// is what makes the edge exact at any zoom level without supersampling.
fn coverage(d: f32, pixel_width: f32) -> f32 {
    if pixel_width <= 0.0 {
        return select(0.0, 1.0, d <= 0.0);
    }
    return 1.0 - smoothstep(-pixel_width, pixel_width, d);
}

// Border as the band between two fields, so its width is independent of the
// shape's overall size.
fn border_mask(outer: f32, inner: f32, pixel_width: f32) -> f32 {
    return clamp(coverage(outer, pixel_width) - coverage(inner, pixel_width), 0.0, 1.0);
}

// Exponential glow falloff from the field sampled at an inflated radius.
//
// Exponential rather than a smoothstep band because light falls off
// exponentially, and because a smoothstep band puts a visible hard ring at its
// boundary where the derivative jumps.
fn glow(d: f32, radius: f32) -> f32 {
    if radius <= 0.001 {
        return 0.0;
    }
    return exp(-max(d, 0.0) / radius);
}

// Deterministic hash for dither noise, so the noise never animates.
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
}

@fragment
fn fs_main(input: VertexOutput) -> FragmentOutput {
    // Shape space: origin at the island center, +y up.
    let frag_px = input.shape_coord * u.resolution;
    let pixel = 1.0 / max(u.dpr, 0.001);
    let p = vec2<f32>(
        (frag_px.x - u.origin_px.x) / max(u.dpr, 0.001),
        (u.origin_px.y - frag_px.y) / max(u.dpr, 0.001),
    );

    let shape = sd_rounded_trapezoid(p, u.half_bottom, u.half_top, u.half_h, u.radius);

    // Early out for pixels that can only be glow. Skipping the rest of the math
    // here is what makes the glow cheap on a large display.
    let aa = fwidth(shape) + 1e-6;
    let glow_start = u.glow_spread + u.glow_falloff * 2.0;

    var fill = vec4<f32>(0.0);
    var glow_amount = 0.0;

    if (shape < glow_start) {
        // Fill, with a vertical gradient toward the accent color when configured.
        let gradient_t = clamp((p.y / max(u.half_h, 0.001)) * 0.5 + 0.5, 0.0, 1.0);
        let gradient = mix(u.fill_rgba.rgb, u.fill_accent_rgba.rgb, gradient_t * u.gradient_strength);
        let fill_alpha = max(u.fill_rgba.a, u.fill_accent_rgba.a * u.gradient_strength);
        fill = vec4<f32>(gradient * fill_alpha, fill_alpha);

        // Border: a band at the shape's isoline, so it stays hairline thin at
        // any size.
        let inner = sd_rounded_trapezoid(
            p, u.half_bottom, u.half_top, u.half_h, max(u.radius - u.border_width, 0.0)
        );
        let border = border_mask(shape, inner, aa);
        fill = mix(fill, vec4<f32>(u.border_rgba.rgb * u.border_rgba.a, u.border_rgba.a), border);

        // Glow, sampled at an inflated radius so it follows the shape.
        let glow_d = sd_rounded_trapezoid(
            p,
            u.half_bottom + u.glow_spread,
            u.half_top + u.glow_spread,
            u.half_h + u.glow_spread,
            u.radius + u.glow_spread
        );
        glow_amount = glow(glow_d, u.glow_falloff) * u.glow_intensity;
    }

    // Dither noise, which removes gradient banding on an 8 bit display.
    let noise = (hash(floor(frag_px / max(u.dpr, 1.0))) - 0.5) * u.noise_strength;

    var color = fill;
    color = mix(color, vec4<f32>(u.glow_rgb, 1.0), clamp(glow_amount, 0.0, 1.0) * coverage(shape, glow_start));
    color.r = color.r + noise;
    color.g = color.g + noise;
    color.b = color.b + noise;

    // Outside the shape and outside the glow, discard entirely. Writing a
    // transparent pixel instead would still cost a blend.
    let visible = coverage(shape, aa) + clamp(glow_amount, 0.0, 1.0);
    if (visible <= 0.001) {
        discard;
    }

    return FragmentOutput(color);
}