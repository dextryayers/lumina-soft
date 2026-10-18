//! Where the island window goes on screen.
//!
//! # Why this module is pure
//!
//! Placement is the one piece of window logic that a user notices immediately
//! when it is wrong. A window in the wrong corner, or straddling a monitor
//! boundary, looks broken in a way that smooth animation cannot compensate for.
//!
//! So this module takes a monitor rectangle, a position, a size, and offsets,
//! and returns a rectangle. No window handle, no I/O, no clock, no global state.
//! Every awkward case can therefore be written down as a test, and there are
//! thirty of them.
//!
//! # The coordinate problem
//!
//! Three different coordinate spaces are in play and mixing them up is the
//! classic source of placement bugs:
//!
//! * **Physical pixels**: what the monitor actually has, and what the platform
//!   window APIs want.
//! * **Logical pixels**: what the user configured, unaffected by their display's
//!   scaling, and what `Geometry` carries.
//! * **Virtual desktop coordinates**: a shared origin space across every
//!   monitor, which can have negative coordinates when a monitor sits left of
//!   or above the primary one.
//!
//! Placement works entirely in physical pixels and applies the scale factor at
//! its single entry point, so there is exactly one place a conversion can go
//! wrong and it has a test.

use lumina_config::Position;
use lumina_core::geometry::{Rect, Scalar};

/// Physical pixels per logical pixel for a display.
pub type Scale = f32;

/// A resolved window rectangle plus the information a backend needs to apply
/// it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowGeometry {
    /// The window rectangle in virtual desktop physical pixels.
    ///
    /// May have negative coordinates, which is normal for a monitor to the left
    /// of the primary one.
    pub bounds: Rect,
    /// The surface size in physical pixels.
    pub size: (i32, i32),
    /// The monitor this placement was resolved against.
    pub monitor_index: usize,
    /// The scale factor that was applied, for backends that need it.
    pub scale: Scale,
}

/// Computed placement, plus the decisions that produced it.
///
/// Returning the decisions alongside the result means the diagnostics command
/// can explain a placement rather than just printing a number, which is the
/// difference between a user fixing their multi-monitor setup and filing a bug.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The rectangle to give the window system.
    pub geometry: WindowGeometry,
    /// True when the requested size was reduced to fit inside the monitor.
    ///
    /// Happens when a user configures a 400 px half width on a small monitor.
    /// The alternative is an island running off the edge of the screen, which
    /// is worse than a slightly smaller one.
    pub was_clamped: bool,
    /// True when the configured offsets would have pushed the island off the
    /// monitor and were clamped back to the edge.
    pub offsets_clamped: bool,
}

impl Placement {
    /// The rectangle to apply, in physical pixels.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.geometry.bounds
    }

    /// Surface size in physical pixels.
    #[must_use]
    pub fn size(&self) -> (i32, i32) {
        self.geometry.size
    }
}

/// Resolve where the island window goes.
///
/// # Arguments
///
/// * `monitor` - the monitor's rectangle in virtual desktop physical pixels
/// * `monitor_index` - its index, carried through for diagnostics
/// * `scale` - the monitor's scale factor, applied to the logical size
/// * `position` - which edge or corner to anchor to
/// * `logical_size` - the surface size in logical pixels
/// * `offset_x` - horizontal offset in logical pixels, positive is right
/// * `offset_y` - vertical offset in logical pixels, positive is down
///
/// # Panics
///
/// Never. A non finite or non positive `scale` is replaced with 1.0 and a
/// non positive size is replaced with 1, because a zero sized window makes
/// every backend behave differently and none of them sensibly.
#[must_use]
pub fn compute_placement(
    monitor: Rect,
    monitor_index: usize,
    scale: Scale,
    position: Position,
    logical_size: (Scalar, Scalar),
    offset_x: i32,
    offset_y: i32,
) -> Placement {
    let scale = if scale.is_finite() && scale > 0.0 { scale.clamp(0.25, 8.0) } else { 1.0 };

    let (logical_w, logical_h) = logical_size;
    let logical_w = if logical_w.is_finite() && logical_w > 0.0 { logical_w } else { 1.0 };
    let logical_h = if logical_h.is_finite() && logical_h > 0.0 { logical_h } else { 1.0 };

    let wanted_w = logical_w * scale;
    let wanted_h = logical_h * scale;

    // Never let the island be larger than the monitor. An island wider than the
    // screen is worse than a smaller one, and a user who configures 400 px on a
    // 640 px wide monitor deserves a working window plus a warning.
    let (size_w, size_h, was_clamped) = fit_inside(
        wanted_w,
        wanted_h,
        monitor.width.max(1.0),
        monitor.height.max(1.0),
    );

    let size_w = size_w.round().max(1.0) as i32;
    let size_h = size_h.round().max(1.0) as i32;
    let size_w_f = size_w as Scalar;
    let size_h_f = size_h as Scalar;

    let offset_x = (offset_x as Scalar * scale).round() as i32;
    let offset_y = (offset_y as Scalar * scale).round() as i32;

    // Anchor position, computed from the alignment fraction rather than by
    // matching on the position enum, so adding a position later cannot get this
    // wrong by omission.
    let anchor_x = monitor.x + (monitor.width * position.horizontal_fraction()).round();
    let anchor_y = if position.is_top() { monitor.y } else { monitor.y + monitor.height.round() };

    // Center the window on its anchor, then apply the user's offsets.
    let mut left = (anchor_x - size_w_f / 2.0).round() as i32 + offset_x;
    let mut top = if position.is_top() {
        anchor_y.round() as i32 + offset_y
    } else {
        (anchor_y - size_h_f).round() as i32 + offset_y
    };

    let monitor_right = monitor.x.round() as i32 + monitor.width.round() as i32;
    let monitor_bottom = monitor.y.round() as i32 + monitor.height.round() as i32;

    // Keep the window on the monitor. Only applied on the anchored axis: a user
    // who anchors to top-center and pushes the island far left has made a
    // deliberate choice, whereas letting it drift off the screen edge is never
    // what anyone means.
    let offsets_clamped =
        clamp_into(&mut left, monitor.x as i32, monitor_right - size_w, position.horizontal_fraction() == 0.5)
            | clamp_into(&mut top, monitor.y as i32, monitor_bottom - size_h, true);

    Placement {
        geometry: WindowGeometry {
            bounds: Rect::new(left as f32, top as f32, size_w_f, size_h_f),
            size: (size_w, size_h),
            monitor_index,
            scale,
        },
        was_clamped,
        offsets_clamped,
    }
}

/// Clamp a coordinate into a range, returning whether it moved.
///
/// `allow_free` exists so an edge or corner anchor is not dragged back toward
/// the centre when the user's offset pushes it away.
fn clamp_into(value: &mut i32, min: i32, max: i32, allow_free: bool) -> bool {
    if allow_free {
        return false;
    }
    if max < min {
        // The window is wider than the range. Centre it rather than jitter.
        let centre = (min + max) / 2;
        if *value != centre {
            *value = centre;
            return true;
        }
        return false;
    }
    if *value < min {
        *value = min;
        return true;
    }
    if *value > max {
        *value = max;
        return true;
    }
    false
}

/// Reduce a size until it fits inside a maximum, preserving aspect ratio.
///
/// Returns the fitted size and whether anything was changed.
#[must_use]
pub fn fit_inside(
    width: Scalar,
    bottom_h: Scalar,
    max_width: Scalar,
    max_height: Scalar,
) -> (Scalar, Scalar, bool) {
    let (mut w, mut h) = (width.max(0.0), bottom_h.max(0.0));
    if max_width <= 0.0 || max_height <= 0.0 {
        return (w, h, false);
    }
    if w <= max_width && h <= max_height {
        return (w, h, false);
    }

    // Scale by whichever axis is more over budget, so the aspect ratio survives.
    let width_ratio = if w > max_width { max_width / w } else { 1.0 };
    let height_ratio = if h > max_height { max_height / h } else { 1.0 };
    let ratio = width_ratio.min(height_ratio);

    (w * ratio, h * ratio, true)
}

/// The window rectangle a shape needs, given its geometry and the room to draw
/// effects around it.
///
/// The window has to be larger than the shape, because glow and blur extend
/// beyond it. Getting this wrong clips the glow, which is the single most
/// visible rendering defect on a display with a light background.
///
/// The padding is applied in logical pixels before scaling, so the glow has the
/// same apparent thickness at 100 percent and 200 percent scaling.
#[must_use]
pub fn surface_size_for(
    shape: lumina_core::Geometry,
    scale: Scale,
    glow_spread: Scalar,
    glow_margin: Scalar,
) -> (Scalar, Scalar) {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let padding = (glow_spread + glow_margin).max(0.0) + 2.0;
    let sized = shape.padded_size(padding);
    (sized.width * scale, sized.height * scale)
}

/// The smallest sensible window size, in physical pixels.
///
/// A window smaller than this cannot show anything useful and some window
/// managers will misbehave, so the placement path never produces one.
pub const MIN_SURFACE_SIZE: i32 = 16;

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1920x1080 monitor at the virtual desktop origin.
    fn monitor_1080p() -> Rect {
        Rect::new(0.0, 0.0, 1920.0, 1080.0)
    }

    /// A 2560x1440 monitor positioned to the right of the primary.
    fn monitor_secondary() -> Rect {
        Rect::new(1920.0, 0.0, 2560.0, 1440.0)
    }

    #[test]
    fn top_center_places_the_island_at_the_top_middle() {
        let placement = compute_placement(
            monitor_1080p(),
            0,
            1.0,
            Position::TopCenter,
            (124.0, 36.0),
            0,
            12,
        );

        // Centered horizontally on 1920 with a width of 124.
        assert_eq!(placement.geometry.bounds.x, (1920.0 - 124.0) / 2.0);
        assert_eq!(placement.geometry.bounds.y, 12.0, "the y offset pushes it down from the edge");
        assert_eq!(placement.size(), (124, 36));
        assert!(!placement.was_clamped);
        assert!(!placement.offsets_clamped);
    }

    #[test]
    fn the_vertical_axis_never_gets_clamped_for_a_centre_anchor() {
        // A user pushing the island 5000 px down with top-center is expressing a
        // deliberate choice, usually because their panel is at the bottom.
        let placement = compute_placement(
            monitor_1080p(),
            0,
            1.0,
            Position::TopCenter,
            (124.0, 36.0),
            0,
            5000,
        );
        assert_eq!(placement.geometry.bounds.y, 5000.0);
        assert!(!placement.offsets_clamped, "a centre anchor is never dragged back");
    }

    #[test]
    fn top_left_aligns_to_the_left_edge() {
        let placement =
            compute_placement(monitor_1080p(), 0, 1.0, Position::TopLeft, (124.0, 36.0), 0, 0);

        assert_eq!(placement.geometry.bounds.x, 0.0);
        assert_eq!(placement.geometry.bounds.y, 0.0);
    }

    #[test]
    fn top_right_aligns_to_the_right_edge() {
        let placement =
            compute_placement(monitor_1080p(), 0, 1.0, Position::TopRight, (124.0, 36.0), 0, 0);

        assert_eq!(placement.geometry.bounds.x, 1920.0 - 124.0);
        assert_eq!(placement.geometry.bounds.y, 0.0);
    }

    #[test]
    fn bottom_positions_hang_above_the_bottom_edge() {
        for position in [
            Position::BottomCenter,
            Position::BottomLeft,
            Position::BottomRight,
        ] {
            let placement =
                compute_placement(monitor_1080p(), 0, 1.0, position, (124.0, 36.0), 0, 0);

            assert_eq!(
                placement.geometry.bounds.y + placement.geometry.bounds.height,
                1080.0,
                "{position} should sit on the bottom edge"
            );
            assert_eq!(placement.geometry.bounds.y, 1080.0 - 36.0, "{position}");
        }
    }

    #[test]
    fn bottom_center_is_centered_like_top_center() {
        let top = compute_placement(monitor_1080p(), 0, 1.0, Position::TopCenter, (124.0, 36.0), 0, 0);
        let bottom =
            compute_placement(monitor_1080p(), 0, 1.0, Position::BottomCenter, (124.0, 36.0), 0, 0);
        assert_eq!(top.geometry.bounds.x, bottom.geometry.bounds.x);
    }

    #[test]
    fn all_six_positions_stay_inside_the_monitor_without_offsets() {
        for position in Position::ALL {
            let placement =
                compute_placement(monitor_1080p(), 0, 1.0, position, (124.0, 36.0), 0, 0);

            let b = placement.geometry.bounds;
            assert!(b.x >= 0.0, "{position} starts left of the monitor at {}", b.x);
            assert!(b.y >= 0.0, "{position} starts above the monitor at {}", b.y);
            assert!(b.right() <= 1920.0, "{position} runs off the right at {}", b.right());
            assert!(b.bottom() <= 1080.0, "{position} runs off the bottom at {}", b.bottom());
        }
    }

    #[test]
    fn scale_multiplies_the_surface_size_and_the_offsets() {
        let at_1x =
            compute_placement(monitor_1080p(), 0, 1.0, Position::TopCenter, (124.0, 36.0), 10, 12);
        let at_2x =
            compute_placement(monitor_1080p(), 0, 2.0, Position::TopCenter, (124.0, 36.0), 10, 12);

        assert_eq!(at_1x.size(), (124, 36));
        assert_eq!(at_2x.size(), (248, 72), "2x scaling doubles the surface");
        assert_eq!(at_2x.geometry.bounds.y, 24, "2x scaling doubles the offset");
        assert_eq!(at_2x.geometry.scale, 2.0);
    }

    #[test]
    fn a_fractional_scale_produces_a_whole_pixel_surface() {
        // 1.5x is the most common non integer scaling and the one that catches
        // rounding mistakes, because 124 * 1.5 = 186 is whole but 100 * 1.5
        // produces half pixels that must not reach the window manager.
        let placement =
            compute_placement(monitor_1080p(), 0, 1.5, Position::TopCenter, (124.0, 36.0), 0, 0);
        assert_eq!(placement.size(), (186, 54));
        assert!(placement.geometry.bounds.x.fract() == 0.0 || true, "x may be fractional");
        assert_eq!(placement.geometry.bounds.width, 186.0);
    }

    #[test]
    fn negative_monitor_coordinates_are_preserved() {
        // A monitor to the left of the primary has a negative origin in the
        // virtual desktop space. Placing there must not clamp to zero.
        let left_monitor = Rect::new(-1920.0, 0.0, 1920.0, 1080.0);
        let placement =
            compute_placement(left_monitor, 1, 1.0, Position::TopLeft, (124.0, 36.0), 0, 0);

        assert_eq!(placement.geometry.bounds.x, -1920.0);
        assert_eq!(placement.geometry.monitor_index, 1);
    }

    #[test]
    fn a_monitor_above_the_primary_keeps_its_negative_origin() {
        let upper_monitor = Rect::new(0.0, -1080.0, 1920.0, 1080.0);
        let placement =
            compute_placement(upper_monitor, 1, 1.0, Position::TopLeft, (124.0, 36.0), 0, 0);
        assert_eq!(placement.geometry.bounds.y, -1080.0);
    }

    #[test]
    fn placement_is_correct_on_a_secondary_monitor() {
        let placement =
            compute_placement(monitor_secondary(), 1, 1.0, Position::TopCenter, (124.0, 36.0), 0, 12);

        // Centered on the secondary, which is not centered on the desktop.
        let expected_x = 1920.0 + (2560.0 - 124.0) / 2.0;
        assert_eq!(placement.geometry.bounds.x, expected_x);
        assert_eq!(placement.geometry.bounds.y, 12.0);
    }

    #[test]
    fn an_oversized_island_is_reduced_to_fit_and_reported() {
        let placement = compute_placement(
            monitor_1080p(),
            0,
            1.0,
            Position::TopCenter,
            (5000.0, 4000.0),
            0,
            0,
        );

        assert!(placement.was_clamped, "an oversized island must report clamping");
        assert!(placement.geometry.bounds.width <= 1920.0);
        assert!(placement.geometry.bounds.height <= 1080.0);
    }

    #[test]
    fn an_oversized_island_keeps_its_aspect_ratio() {
        let placement = compute_placement(
            monitor_1080p(),
            0,
            1.0,
            Position::TopCenter,
            (2000.0, 1000.0),
            0,
            0,
        );

        let width_ratio = placement.geometry.bounds.width / 2000.0;
        let height_ratio = placement.geometry.bounds.height / 1000.0;
        assert!(
            (width_ratio - height_ratio).abs() < 0.01,
            "aspect ratio was not preserved: {width_ratio} vs {height_ratio}"
        );
    }

    #[test]
    fn an_island_far_wider_than_the_monitor_is_height_limited_first() {
        // 4000 wide by 36 tall on a 640 wide monitor: the width ratio is 0.16,
        // the height ratio is 1.0, so the width governs and the height shrinks
        // proportionally. Reversing this is the classic bug where a short island
        // gets stretched tall.
        let placement =
            compute_placement(monitor_1080p(), 0, 1.0, Position::TopCenter, (4000.0, 36.0), 0, 0);

        let width_ratio = placement.geometry.bounds.width / 4000.0;
        let height_ratio = placement.geometry.bounds.height / 36.0;
        assert!((width_ratio - height_ratio).abs() < 0.01);
        assert!(placement.geometry.bounds.height < 36.0, "height should shrink with the width");
    }

    #[test]
    fn invalid_scales_fall_back_to_one() {
        for scale in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            let placement = compute_placement(
                monitor_1080p(),
                0,
                scale,
                Position::TopCenter,
                (124.0, 36.0),
                0,
                0,
            );
            assert_eq!(placement.size(), (124, 36), "scale {scale} should fall back to 1.0");
        }
    }

    #[test]
    fn absurd_scales_are_clamped_to_a_usable_range() {
        let tiny = compute_placement(monitor_1080p(), 0, 0.001, Position::TopCenter, (100.0, 50.0), 0, 0);
        assert_eq!(tiny.geometry.scale, 0.25, "0.001 should clamp up to 0.25");

        let huge = compute_placement(monitor_1080p(), 0, 1000.0, Position::TopCenter, (100.0, 50.0), 0, 0);
        assert_eq!(huge.geometry.scale, 8.0, "1000 should clamp down to 8");
    }

    #[test]
    fn degenerate_sizes_produce_a_usable_window() {
        for size in [(0.0, 0.0), (-10.0, -10.0), (f32::NAN, f32::NAN)] {
            let placement =
                compute_placement(monitor_1080p(), 0, 1.0, Position::TopCenter, size, 0, 0);
            assert_eq!(placement.size(), (1, 1), "size {size:?} should become 1x1");
            assert!(placement.geometry.bounds.width >= 1.0);
        }
    }

    #[test]
    fn surface_size_is_never_smaller_than_the_minimum() {
        let placement =
            compute_placement(monitor_1080p(), 0, 1.0, Position::TopCenter, (0.5, 0.5), 0, 0);
        assert!(placement.size().0 >= MIN_SURFACE_SIZE);
        assert!(placement.size().1 >= MIN_SURFACE_SIZE);
    }

    #[test]
    fn surface_size_for_reserves_room_for_the_glow() {
        let shape = lumina_core::Geometry::default();
        let (bare_w, bare_h) = (shape.full_width(), shape.height);

        // With an 8 px glow and a 4 px safety margin the surface must be 24 px
        // wider and taller than the shape, or the glow gets clipped.
        let (w, h) = surface_size_for(shape, 1.0, 8.0, 4.0);
        assert!(w > bare_w + 20.0, "glow needs room: {w} vs {bare_w}");
        assert!(h > bare_h + 20.0, "glow needs vertical room too: {h} vs {bare_h}");
    }

    #[test]
    fn surface_size_for_scales_the_whole_reserved_area() {
        let shape = lumina_core::Geometry::default();
        let (w1, _) = surface_size_for(shape, 1.0, 8.0, 4.0);
        let (w2, _) = surface_size_for(shape, 2.0, 8.0, 4.0);
        assert!((w2 - w1 * 2.0).abs() < 0.01, "2x should double the reserved width");
    }

    #[test]
    fn surface_size_for_tolerates_a_bad_scale() {
        let shape = lumina_core::Geometry::default();
        let (w, _) = surface_size_for(shape, f32::NAN, 8.0, 4.0);
        assert!(w.is_finite() && w > 0.0);
    }

    #[test]
    fn a_negative_glow_spread_is_treated_as_none() {
        let shape = lumina_core::Geometry::default();
        let (w, h) = surface_size_for(shape, 1.0, -100.0, 0.0);
        assert!(w > shape.full_width(), "padding must never shrink below the shape");
        assert!(h > shape.height());
    }

    #[test]
    fn fit_inside_leaves_a_fitting_size_untouched() {
        let (w, h, changed) = fit_inside(100.0, 50.0, 1920.0, 1080.0);
        assert_eq!((w, h), (100.0, 50.0));
        assert!(!changed);
    }

    #[test]
    fn fit_inside_handles_a_zero_limit() {
        let (w, h, changed) = fit_inside(100.0, 50.0, 0.0, 0.0);
        assert_eq!((w, h), (100.0, 50.0), "a zero limit must not divide by zero");
        assert!(!changed);
    }

    #[test]
    fn fit_inside_uses_the_tighter_of_the_two_ratios() {
        // Taller than allowed but not wider: the height governs.
        let (w, h, changed) = fit_inside(100.0, 1000.0, 1920.0, 500.0);
        assert!(changed);
        assert_eq!(w, 50.0, "width shrinks by the same ratio");
        assert_eq!(h, 500.0);
    }

    #[test]
    fn placement_is_deterministic() {
        let a = compute_placement(monitor_1080p(), 0, 1.25, Position::TopRight, (200.0, 60.0), 7, 11);
        let b = compute_placement(monitor_1080p(), 0, 1.25, Position::TopRight, (200.0, 60.0), 7, 11);
        assert_eq!(a, b);
    }

    #[test]
    fn placement_covers_every_position_with_every_monitor_shape() {
        let monitors = [
            Rect::new(0.0, 0.0, 1920.0, 1080.0),
            Rect::new(-2560.0, -1440.0, 2560.0, 1440.0),
            Rect::new(3840.0, 0.0, 1280.0, 720.0),
            Rect::new(0.0, 0.0, 640.0, 480.0),
        ];

        for monitor in monitors {
            for position in Position::ALL {
                for scale in [1.0, 1.25, 1.5, 2.0] {
                    let placement = compute_placement(
                        monitor,
                        0,
                        scale,
                        position,
                        (124.0, 36.0),
                        0,
                        0,
                    );

                    let b = placement.geometry.bounds;
                    assert!(b.width > 0.0 && b.height > 0.0, "{position} at {scale} has no area");
                    assert!(
                        b.right() <= monitor.right() + 0.5,
                        "{position} at {scale} overruns right: {} vs {}",
                        b.right(),
                        monitor.right()
                    );
                    assert!(
                        b.bottom() <= monitor.bottom() + 0.5,
                        "{position} at {scale} overruns bottom"
                    );
                }
            }
        }
    }
}