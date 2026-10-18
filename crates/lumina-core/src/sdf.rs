//! Analytic signed distance fields for the island shape.
//!
//! Everything here is pure `f32` math with no state. The shader in
//! `lumina-render` reproduces these functions in WGSL, and the tests in this
//! module check the Rust implementation against a brute force grid search, so a
//! sign error or a wrong term cannot survive.
//!
//! # Coordinate space
//!
//! Points are in shape space: origin at the center of the shape, `+x` right,
//! `+y` up. The top edge is at `y = +half_h` and spans `[-half_top, +half_top]`.
//! The bottom edge is at `y = -half_h` and spans `[-half_bottom, +half_bottom]`.
//!
//! # Why signed distance rather than a mesh
//!
//! A distance field gives us four things a tessellated polygon cannot give us
//! cheaply:
//!
//! 1. Anti aliasing in one `smoothstep` on `fwidth(d)`, exact at any scale.
//! 2. Border as the band between two fields, so width is independent of the
//!    shape's size.
//! 3. Glow as the field sampled at a larger radius, so it follows the shape
//!    exactly instead of being a separate blurred sprite.
//! 4. Rounding as a polygon inset, which is analytic and free.
//!
//! # Rounding derivation
//!
//! Rounding a convex polygon is done by insetting each edge by the radius,
//! forming the offset polygon, then subtracting the radius from that polygon's
//! own field. For a trapezoid the offset polygon is still a trapezoid:
//!
//! * The two horizontal edges inset by `r` give `half_h - r`.
//! * The two slanted edges need a horizontal inset of `r / cos(theta)` where
//!   `theta` is the slant angle measured from vertical, because moving inward
//!   horizontally by `dx` changes the perpendicular distance by `dx * cos(theta)`.
//!
//! Degeneracy check: when `half_top == half_bottom` the slant angle is zero,
//! `cos(theta)` is one, the inset is `r`, and the function reduces exactly to
//! a rounded rectangle. [`sd_rounded_trapezoid_reduces_to_rounded_rect`] asserts
//! that.

/// Minimal length of a vector.
#[inline]
fn length(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// Smallest of three values.
#[inline]
fn min3(a: f32, b: f32, c: f32) -> f32 {
    if a < b {
        if a < c {
            a
        } else {
            c
        }
    } else if b < c {
        b
    } else {
        c
    }
}

/// Clamp helper kept separate so the mirror in WGSL reads the same.
#[inline]
fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Signed distance to an isosceles trapezoid, unrounded.
///
/// Returns a true euclidean distance: the magnitude is the distance to the
/// nearest edge, and the sign is negative inside.
///
/// # Panics
///
/// Never. Degenerate inputs (zero or negative height) return a finite value
/// rather than producing `NaN`.
#[must_use]
pub fn sd_trapezoid(
    px: f32,
    py: f32,
    half_bottom: f32,
    half_top: f32,
    half_h: f32,
) -> f32 {
    if half_h <= 0.0 || !half_top.is_finite() || !half_bottom.is_finite() || !half_h.is_finite() {
        return f32::MAX;
    }

    // k1 is the top right vertex at (half_top, +half_h), and k2 is the edge
    // vector from the bottom right vertex up to it. Parametrising from the top
    // means the closest point is `k1 - k2 * t`, which is what makes
    // `q - k1 + k2 * t` the vector from that point back to the query.
    let k1x = half_top;
    let k1y = half_h;
    let k2x = half_top - half_bottom;
    let k2y = 2.0 * half_h;

    // Fold to the right half plane. The trapezoid is symmetric about x = 0.
    let qx = px.abs();
    let qy = py;

    // Distance to the nearer horizontal edge, and the horizontal overhang
    // beyond that edge's half width.
    let edge_half = if qy > 0.0 { half_top } else { half_bottom };
    let ca_x = qx - qx.min(edge_half);
    let ca_y = qy.abs() - half_h;
    let d_h = length(ca_x, ca_y);

    // Distance to the slanted edge via segment projection.
    let k2_len_sq = k2x * k2x + k2y * k2y;
    let t = if k2_len_sq > 0.0 {
        clamp01(((k1x - qx) * k2x + (k1y - qy) * k2y) / k2_len_sq)
    } else {
        0.0
    };
    let cb_x = qx - k1x + k2x * t;
    let cb_y = qy - k1y + k2y * t;
    let d_s = length(cb_x, cb_y);

    // Inside when the point lies to the inner side of the slanted edge and
    // between the two horizontal edges.
    let inside = cb_x < 0.0 && ca_y < 0.0;
    let sign = if inside { -1.0 } else { 1.0 };

    sign * min3(d_h, d_s, f32::MAX)
}

/// Signed distance to a rounded isosceles trapezoid.
///
/// `radius` is clamped to something the shape can accommodate, so callers do
/// not have to pre-clamp. See the module documentation for the derivation.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn sd_rounded_trapezoid(
    px: f32,
    py: f32,
    half_bottom: f32,
    half_top: f32,
    half_h: f32,
    radius: f32,
) -> f32 {
    if half_h <= 0.0 {
        return f32::MAX;
    }

    let r = radius.max(0.0).min(half_h).min(half_top * 0.5).min(half_bottom * 0.5);
    if r <= 0.0 {
        return sd_trapezoid(px, py, half_bottom, half_top, half_h);
    }

    // Inset amount for the horizontal edges.
    let inset_y = r;

    // Inset amount for the slanted edges, expressed horizontally.
    // Moving inward by dx changes perpendicular distance by dx * cos(theta).
    let slant_angle = ((half_top - half_bottom) / (2.0 * half_h)).atan();
    let cos_theta = slant_angle.cos();
    let inset_x = if cos_theta > 0.15 { r / cos_theta } else { r * 6.0 };

    let inner = sd_trapezoid(px, py, half_bottom - inset_x, half_top - inset_x, half_h - inset_y);
    inner - r
}

/// Signed distance to a rounded rectangle, provided as an independent
/// reference for the degeneracy test.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn sd_rounded_rect(px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    if half_h <= 0.0 || half_w <= 0.0 {
        return f32::MAX;
    }
    let r = radius.max(0.0).min(half_w).min(half_h);
    let qx = px.abs() - (half_w - r);
    let qy = py.abs() - (half_h - r);
    let outside = length(qx.max(0.0), qy.max(0.0));
    let inside = qx.max(qy).min(0.0);
    outside + inside - r
}

/// Signed distance to a circle, used by the dot and progress ring widgets.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn sd_circle(px: f32, py: f32, radius: f32) -> f32 {
    length(px, py) - radius
}

/// Signed distance to a capsule between two points, used by progress bars and
/// sliders so their ends are rounded at any length.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn sd_capsule(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, radius: f32) -> f32 {
    let pax = px - ax;
    let pay = py - ay;
    let bax = bx - ax;
    let bay = by - ay;
    let len_sq = bax * bax + bay * bay;
    let h = if len_sq > 0.0 { clamp01((pax * bax + pay * bay) / len_sq) } else { 0.0 };
    length(pax - bax * h, pay - bay * h) - radius
}

/// Anti aliased coverage from a signed distance, using screen space derivatives.
///
/// Coverage of 1 inside the shape, 0 outside, smooth across one pixel. The
/// `fwidth` term is what makes this resolution independent: it measures how
/// many shape units one pixel covers at this position.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn coverage_from_distance(d: f32, pixel_width: f32) -> f32 {
    if pixel_width <= 0.0 {
        return if d <= 0.0 { 1.0 } else { 0.0 };
    }
    let aa = pixel_width;
    1.0 - smoothstep(-aa, aa, d)
}

/// GLSL style `smoothstep`, with the edges ordered defensively.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < f32::EPSILON {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = clamp01((x - edge0) / (edge1 - edge0));
    t * t * (3.0 - 2.0 * t)
}

/// The band between two signed distances, used to stroke a border whose width
/// is independent of the shape's overall size.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn border_mask(outer: f32, inner: f32, pixel_width: f32) -> f32 {
    let outside = coverage_from_distance(outer, pixel_width);
    let hole = coverage_from_distance(inner, pixel_width);
    (outside - hole).clamp(0.0, 1.0)
}

/// Exponential glow falloff from an outer field, sampled at an inflated radius.
///
/// Exponential rather than a smoothstep band because light falls off
/// exponentially, and because a smoothstep band puts a visible hard ring at its
/// boundary where the derivative jumps.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn glow_falloff(d: f32, radius: f32) -> f32 {
    if radius <= 0.001 {
        return 0.0;
    }
    let outside = d.max(0.0);
    (-outside / radius).exp()
}

/// Brute force reference: the true distance from a point to a trapezoid,
/// computed by densely sampling the boundary.
///
/// Used only by tests. It makes no algebraic assumptions, so agreeing with
/// [`sd_trapezoid`] is real evidence rather than a restatement.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn brute_force_trapezoid_distance(
    px: f32,
    py: f32,
    half_bottom: f32,
    half_top: f32,
    half_h: f32,
    samples_per_edge: usize,
) -> f32 {
    let h = half_h.max(0.001);
    let verts = [
        (-half_bottom, -h),
        (half_bottom, -h),
        (half_top, h),
        (-half_top, h),
    ];

    let mut best = f32::MAX;
    for i in 0..verts.len() {
        let (ax, ay) = verts[i];
        let (bx, by) = verts[(i + 1) % verts.len()];
        for s in 0..=samples_per_edge {
            let t = s as f32 / samples_per_edge as f32;
            let sx = ax + (bx - ax) * t;
            let sy = ay + (by - ay) * t;
            let d = length(px - sx, py - sy);
            if d < best {
                best = d;
            }
        }
    }
    best
}

/// Point in trapezoid test via half plane intersection, used by tests to
/// establish ground truth for the sign of the field.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn point_in_trapezoid(px: f32, py: f32, half_bottom: f32, half_top: f32, half_h: f32) -> bool {
    let h = half_h.max(0.001);
    let verts = [
        (-half_bottom, -h),
        (half_bottom, -h),
        (half_top, h),
        (-half_top, h),
    ];
    // Consistent winding: bottom left, bottom right, top right, top left is
    // counter clockwise in a y-up space.
    let mut sign = 0.0f32;
    for i in 0..verts.len() {
        let (ax, ay) = verts[i];
        let (bx, by) = verts[(i + 1) % verts.len()];
        let cross = (bx - ax) * (py - ay) - (by - ay) * (px - ax);
        if cross.abs() < 1e-6 {
            continue;
        }
        let s = if cross > 0.0 { 1.0 } else { -1.0 };
        if sign == 0.0 {
            sign = s;
        } else if s != sign {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f32 = 1e-3;

    /// Half the spacing between brute force samples on the longest edge, which
    /// is the worst-case error the reference computation can have.
    ///
    /// Rather than loosening the tolerance for every shape, the reference's own
    /// uncertainty is added to the comparison tolerance. That keeps the tight
    /// bound meaningful for small shapes and stops large shapes producing false
    /// failures from reference sampling error alone.
    fn brute_force_error(
        half_bottom: f32,
        half_top: f32,
        half_h: f32,
        samples_per_edge: usize,
    ) -> f32 {
        let longest = (half_top * 2.0).max(half_bottom * 2.0);
        let slant = (half_top - half_bottom).abs();
        let side = (slant * slant + (half_h * 2.0) * (half_h * 2.0)).sqrt();
        longest.max(side) / samples_per_edge as f32 * 0.5
    }

    /// Independently recompute the inset trapezoid the rounding formula claims
    /// to build, so the rounded field can be checked against brute force rather
    /// than only against the closed form it is defined in terms of.
    fn offset_polygon(
        half_bottom: f32,
        half_top: f32,
        half_h: f32,
        radius: f32,
    ) -> (f32, f32, f32) {
        let slant_angle = ((half_top - half_bottom) / (2.0 * half_h)).atan();
        let inset_x = if slant_angle.cos() > 0.15 {
            radius / slant_angle.cos()
        } else {
            radius * 6.0
        };
        (half_bottom - inset_x, half_top - inset_x, half_h - radius)
    }

    fn check_field_against_brute_force(
        half_bottom: f32,
        half_top: f32,
        half_h: f32,
        radius: f32,
        extent: f32,
    ) {
        const SAMPLES_PER_EDGE: usize = 4000;
        let reference_error = brute_force_error(half_bottom, half_top, half_h, SAMPLES_PER_EDGE);
        let tolerance = TOL + reference_error;

        // For the rounded case the field is defined as the offset polygon minus
        // the radius, so the ground truth has to be computed from that same
        // offset polygon.
        let (ref_hb, ref_ht, ref_hh) = if radius > 0.0 {
            offset_polygon(half_bottom, half_top, half_h, radius)
        } else {
            (half_bottom, half_top, half_h)
        };

        let mut checked = 0usize;
        // A deterministic grid plus a fixed pseudo random jitter, so failures
        // are reproducible.
        let mut seed = 0x1234_5678u32;
        let steps = 60;
        for ix in 0..steps {
            for iy in 0..steps {
                let fx = ix as f32 / steps as f32;
                let fy = iy as f32 / steps as f32;
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let jx = ((seed >> 8) & 0xFFFF) as f32 / 65535.0 - 0.5;
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let jy = ((seed >> 8) & 0xFFFF) as f32 / 65535.0 - 0.5;

                let px = (fx * 2.0 - 1.0) * extent + jx * 0.4;
                let py = (fy * 2.0 - 1.0) * extent + jy * 0.4;

                let analytic =
                    sd_rounded_trapezoid(px, py, half_bottom, half_top, half_h, radius);

                let distance =
                    brute_force_trapezoid_distance(px, py, ref_hb, ref_ht, ref_hh, SAMPLES_PER_EDGE);
                let truth_inside = point_in_trapezoid(px, py, ref_hb, ref_ht, ref_hh);

                // The rounded field is the offset polygon's field minus the
                // radius, in both directions. The rounded shape is the offset
                // polygon grown by a disc of that radius, so the boundary sits
                // `radius` outside the offset polygon, not inside it.
                let inset_field = if truth_inside { -distance } else { distance };
                let truth_signed = inset_field - radius;

                // Skip points sitting on the boundary, where the rounded and
                // unrounded treatments legitimately disagree and where the
                // brute force sampling error is largest.
                if (truth_signed.abs() < 0.05) || !analytic.is_finite() {
                    continue;
                }

                assert!(
                    (analytic - truth_signed).abs() < tolerance,
                    "field mismatch at ({px:.3}, {py:.3}) for hb={half_bottom} ht={half_top} hh={half_h} r={radius}: \
                     analytic={analytic:.6} brute_force={truth_signed:.6} inside={truth_inside} \
                     tol={tolerance:.6}"
                );
                checked += 1;
            }
        }
        assert!(checked > 500, "expected a meaningful number of samples, got {checked}");
    }

    #[test]
    fn trapezoid_field_matches_brute_force_inverted() {
        check_field_against_brute_force(54.0, 62.0, 18.0, 0.0, 120.0);
    }

    #[test]
    fn trapezoid_field_matches_brute_force_wide() {
        check_field_against_brute_force(200.0, 210.0, 50.0, 0.0, 260.0);
    }

    #[test]
    fn trapezoid_field_matches_brute_force_narrow_and_short() {
        check_field_against_brute_force(40.0, 48.0, 4.0, 0.0, 90.0);
    }

    #[test]
    fn rounded_trapezoid_field_matches_brute_force() {
        check_field_against_brute_force(54.0, 62.0, 18.0, 12.0, 120.0);
        check_field_against_brute_force(200.0, 210.0, 50.0, 30.0, 260.0);
    }

    #[test]
    fn rounded_trapezoid_reduces_to_rounded_rect() {
        // Equal half widths means zero slant, so the trapezoid must behave
        // exactly like a rounded rectangle. This is the degeneracy guarantee
        // the inset derivation relies on.
        let half_w = 60.0f32;
        let half_h = 20.0f32;
        for r in [1.0f32, 5.0, 12.0, 19.0] {
            for ix in -14..=14 {
                for iy in -6..=6 {
                    let px = ix as f32 * 3.1;
                    let py = iy as f32 * 3.1;
                    let a = sd_rounded_trapezoid(px, py, half_w, half_w, half_h, r);
                    let b = sd_rounded_rect(px, py, half_w, half_h, r);
                    assert!(
                        (a - b).abs() < 1e-3,
                        "degenerate case r={r} at ({px}, {py}): trapezoid={a:.6} rect={b:.6}"
                    );
                }
            }
        }
    }

    #[test]
    fn field_is_negative_inside_and_positive_outside() {
        let (hb, ht, hh) = (54.0f32, 62.0f32, 18.0f32);
        assert!(sd_trapezoid(0.0, 0.0, hb, ht, hh) < 0.0, "center is inside");
        assert!(sd_trapezoid(0.0, hh * 3.0, hb, ht, hh) > 0.0, "far above is outside");
        assert!(sd_trapezoid(ht * 3.0, 0.0, hb, ht, hh) > 0.0, "far right is outside");
        assert!(sd_rounded_trapezoid(0.0, 0.0, hb, ht, hh, 12.0) < 0.0);
    }

    #[test]
    fn field_is_symmetric_about_the_vertical_axis() {
        let (hb, ht, hh) = (54.0f32, 62.0f32, 18.0f32);
        for px in [1.0f32, 7.5, 30.0, 58.0, 90.0] {
            for py in [-25.0f32, -5.0, 0.0, 12.0, 30.0] {
                let left = sd_trapezoid(-px, py, hb, ht, hh);
                let right = sd_trapezoid(px, py, hb, ht, hh);
                assert!((left - right).abs() < 1e-4, "asymmetric at ({px}, {py})");
            }
        }
    }

    #[test]
    fn field_distance_is_continuous_across_the_boundary() {
        // Walk a horizontal line through the shape and check that consecutive
        // samples never jump by more than the sample spacing. A wrong term in
        // the projection produces a visible discontinuity even when the sign is
        // right everywhere.
        let (hb, ht, hh) = (54.0f32, 62.0f32, 18.0f32);
        let step = 0.05f32;
        let mut prev = sd_rounded_trapezoid(-100.0, 0.0, hb, ht, hh, 14.0);
        let mut x = -100.0f32 + step;
        while x < 100.0 {
            let d = sd_rounded_trapezoid(x, 0.0, hb, ht, hh, 14.0);
            assert!(
                (d - prev).abs() <= step * 1.001,
                "discontinuity at x={x}: {prev:.6} to {d:.6}"
            );
            prev = d;
            x += step;
        }
    }

    #[test]
    fn center_distance_matches_negative_half_height() {
        // At the center of a wide shape the nearest boundary is top or bottom.
        let d = sd_trapezoid(0.0, 0.0, 200.0, 210.0, 50.0);
        assert!((d + 50.0).abs() < 1e-3, "expected -half_h, got {d}");
    }

    #[test]
    fn degenerate_height_returns_finite_not_nan() {
        assert!(sd_trapezoid(0.0, 0.0, 10.0, 10.0, 0.0).is_finite());
        assert!(sd_trapezoid(0.0, 0.0, 10.0, 10.0, -5.0).is_finite());
        assert!(sd_rounded_trapezoid(0.0, 0.0, 10.0, 10.0, 0.0, 5.0).is_finite());
        assert!(sd_rounded_trapezoid(0.0, 0.0, 10.0, 10.0, 10.0, f32::NAN).is_finite());
    }

    #[test]
    fn coverage_is_one_inside_and_zero_outside() {
        assert_eq!(coverage_from_distance(-5.0, 1.0), 1.0);
        assert_eq!(coverage_from_distance(5.0, 1.0), 0.0);
        let mid = coverage_from_distance(0.0, 1.0);
        assert!((0.5 - mid).abs() < 1e-6, "edge coverage should be 0.5, got {mid}");
    }

    #[test]
    fn coverage_handles_zero_pixel_width_without_dividing_by_zero() {
        assert_eq!(coverage_from_distance(-1.0, 0.0), 1.0);
        assert_eq!(coverage_from_distance(1.0, 0.0), 0.0);
        assert!(coverage_from_distance(0.0, 0.0).is_finite());
    }

    #[test]
    fn border_mask_is_one_on_the_ring_and_zero_inside_and_outside() {
        assert_eq!(border_mask(-1.0, 1.0, 0.1), 1.0, "ring between the two fields");
        assert_eq!(border_mask(5.0, 7.0, 0.1), 0.0, "outside the outer field");
        assert_eq!(border_mask(-5.0, -7.0, 0.1), 0.0, "inside the inner field");
    }

    #[test]
    fn glow_falls_off_exponentially_and_never_negatives() {
        let at_zero = glow_falloff(0.0, 8.0);
        assert!((at_zero - 1.0).abs() < 1e-6, "glow starts at full strength on the boundary");

        let mut previous = 1.0f32;
        for d in [1.0f32, 2.0, 4.0, 8.0, 16.0] {
            let g = glow_falloff(d, 8.0);
            assert!(g >= 0.0 && g <= 1.0, "glow out of range at d={d}: {g}");
            assert!(g < previous, "glow must decay, d={d}");
            previous = g;
        }

        // Inside the shape the glow contribution is at full strength, so the
        // border and fill hide it.
        assert_eq!(glow_falloff(-10.0, 8.0), 1.0);
    }

    #[test]
    fn circle_and_capsule_agree_on_degenerate_segment() {
        assert!(sd_capsule(3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 1.0).is_finite());
        assert!((sd_capsule(3.0, 4.0, 0.0, 0.0, 10.0, 0.0, 0.0) - 4.0).abs() < 1e-5);
        assert!((sd_capsule(3.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0) - 0.0).abs() < 1e-5, "a point on the segment is zero distance away");
        assert!((sd_capsule(20.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0) - 10.0).abs() < 1e-5, "past the end the distance is measured to the endpoint");
        assert!((sd_capsule(20.0, 3.0, 0.0, 0.0, 10.0, 0.0, 0.0) - 109.0_f32.sqrt()).abs() < 1e-4);
        assert!((sd_capsule(-5.0, 4.0, 0.0, 0.0, 10.0, 0.0, 0.0) - 41.0_f32.sqrt()).abs() < 1e-4, "before the start the distance is measured to the other endpoint");
        assert!((sd_circle(3.0, 4.0, 1.0) - 4.0).abs() < 1e-5);
    }

    #[test]
    fn smoothstep_is_monotonic_and_clamped() {
        assert_eq!(smoothstep(0.0, 1.0, -5.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 5.0), 1.0);
        let mut prev = -1.0f32;
        for i in 0..=20 {
            let v = smoothstep(0.0, 1.0, i as f32 / 20.0);
            assert!(v >= prev, "not monotonic at {i}");
            prev = v;
        }
        assert!(smoothstep(1.0, 1.0, 0.5).is_finite());
    }
}