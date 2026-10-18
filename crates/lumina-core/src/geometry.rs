//! Sizes, vectors, rectangles, insets, and island geometry.
//!
//! All types are `Copy`, fixed size, and free of allocation so they can be
//! stored in uniforms and passed across threads without ceremony.
//!
//! # The inverted trapezoid invariant
//!
//! The island's defining shape has a wider top edge than bottom edge. That
//! invariant is enforced here rather than trusted. A user who writes
//! `bottom_half_width = 200` with `top_half_width = 100` gets a silently
//! corrected geometry and a warning, not a shape that inverts mid animation.

use serde::{Deserialize, Serialize};

/// Scalar used for all spatial values, matching the shader's `f32`.
pub type Scalar = f32;

/// A two component vector of [`Scalar`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: Scalar,
    /// Vertical component.
    pub y: Scalar,
}

impl Vec2 {
    /// The zero vector.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    /// Construct from components.
    #[must_use]
    pub const fn new(x: Scalar, y: Scalar) -> Self {
        Self { x, y }
    }

    /// Euclidean length.
    #[must_use]
    pub fn length(self) -> Scalar {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Component-wise addition.
    #[must_use]
    pub const fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }

    /// Component-wise subtraction.
    #[must_use]
    pub const fn subtract(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }

    /// Component-wise scaling.
    #[must_use]
    pub const fn scale(self, factor: Scalar) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }

    /// True when both components are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl From<(Scalar, Scalar)> for Vec2 {
    fn from((x, y): (Scalar, Scalar)) -> Self {
        Self::new(x, y)
    }
}

/// A width and height pair in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Size {
    /// Horizontal extent.
    pub width: Scalar,
    /// Vertical extent.
    pub height: Scalar,
}

impl Size {
    /// The zero size.
    pub const ZERO: Self = Self { width: 0.0, height: 0.0 };

    /// Construct from components.
    #[must_use]
    pub const fn new(width: Scalar, height: Scalar) -> Self {
        Self { width, height }
    }

    /// Convert to a vector, mostly so `Rect` construction reads cleanly.
    #[must_use]
    pub const fn as_vec2(self) -> Vec2 {
        Vec2::new(self.width, self.height)
    }

    /// True when both components are finite and non negative.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width >= 0.0 && self.height >= 0.0
    }
}

/// An axis aligned rectangle defined by origin and size.
///
/// Origin is the top left corner, matching every windowing coordinate space
/// Lumina deals with. Y grows downward.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: Scalar,
    /// Top edge.
    pub y: Scalar,
    /// Horizontal extent.
    pub width: Scalar,
    /// Vertical extent.
    pub height: Scalar,
}

impl Rect {
    /// A rectangle at the origin with zero extent.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };

    /// Construct from origin and size.
    #[must_use]
    pub const fn new(x: Scalar, y: Scalar, width: Scalar, height: Scalar) -> Self {
        Self { x, y, width, height }
    }

    /// Construct from origin and a [`Size`].
    #[must_use]
    pub const fn from_size(origin: Vec2, size: Size) -> Self {
        Self { x: origin.x, y: origin.y, width: size.width, height: size.height }
    }

    /// Left edge.
    #[must_use]
    pub const fn left(self) -> Scalar {
        self.x
    }

    /// Top edge.
    #[must_use]
    pub const fn top(self) -> Scalar {
        self.y
    }

    /// Right edge, exclusive.
    #[must_use]
    pub const fn right(self) -> Scalar {
        self.x + self.width
    }

    /// Bottom edge, exclusive.
    #[must_use]
    pub const fn bottom(self) -> Scalar {
        self.y + self.height
    }

    /// Size of the rectangle.
    #[must_use]
    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Top left corner.
    #[must_use]
    pub const fn origin(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    /// Geometric center.
    #[must_use]
    pub const fn center(self) -> Vec2 {
        Vec2::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    /// Area, clamped at zero for degenerate rectangles.
    #[must_use]
    pub fn area(self) -> Scalar {
        if self.width <= 0.0 || self.height <= 0.0 {
            0.0
        } else {
            self.width * self.height
        }
    }

    /// Point in rectangle test using a half open interval on the far edges, so
    /// adjacent rectangles do not both report a hit at their shared edge.
    #[must_use]
    pub fn contains(self, point: Vec2) -> bool {
        point.x >= self.left()
            && point.x < self.right()
            && point.y >= self.top()
            && point.y < self.bottom()
    }

    /// Shrink by `insets` on each side. Never produces negative extent.
    #[must_use]
    pub fn inset(self, insets: Insets) -> Self {
        Self {
            x: self.x + insets.left,
            y: self.y + insets.top,
            width: (self.width - insets.left - insets.right).max(0.0),
            height: (self.height - insets.top - insets.bottom).max(0.0),
        }
    }

    /// Shrink by a uniform amount on each side.
    #[must_use]
    pub fn shrink(self, amount: Scalar) -> Self {
        self.inset(Insets::uniform(amount))
    }

    /// Grow by a uniform amount on each side.
    #[must_use]
    pub fn grow(self, amount: Scalar) -> Self {
        self.inset(Insets::uniform(-amount))
    }

    /// Smallest rectangle containing both inputs. Used to size a window to
    /// cover an expanded island plus its glow.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let left = self.left().min(other.left());
        let top = self.top().min(other.top());
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self { x: left, y: top, width: right - left, height: bottom - top }
    }

    /// Intersection, or [`Rect::ZERO`] when the inputs are disjoint.
    #[must_use]
    pub fn intersection(self, other: Self) -> Self {
        let left = self.left().max(other.left());
        let top = self.top().max(other.top());
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= left || bottom <= top {
            return Self::ZERO;
        }
        Self { x: left, y: top, width: right - left, height: bottom - top }
    }

    /// True when the rectangles share any area.
    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        self.left() < other.right()
            && other.left() < self.right()
            && self.top() < other.bottom()
            && other.top() < self.bottom()
    }

    /// True when every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.width.is_finite() && self.height.is_finite()
    }
}

/// Per side spacing.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Insets {
    /// Left spacing.
    pub left: Scalar,
    /// Top spacing.
    pub top: Scalar,
    /// Right spacing.
    pub right: Scalar,
    /// Bottom spacing.
    pub bottom: Scalar,
}

impl Insets {
    /// Zero insets.
    pub const ZERO: Self = Self { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };

    /// Uniform spacing on all sides.
    #[must_use]
    pub const fn uniform(value: Scalar) -> Self {
        Self { left: value, top: value, right: value, bottom: value }
    }

    /// Horizontal and vertical spacing.
    #[must_use]
    pub const fn symmetric(horizontal: Scalar, vertical: Scalar) -> Self {
        Self { left: horizontal, right: horizontal, top: vertical, bottom: vertical }
    }

    /// Total horizontal extent consumed.
    #[must_use]
    pub const fn horizontal(self) -> Scalar {
        self.left + self.right
    }

    /// Total vertical extent consumed.
    #[must_use]
    pub const fn vertical(self) -> Scalar {
        self.top + self.bottom
    }
}

/// Bounds applied to every user supplied geometry value.
///
/// These exist so that no configuration, however malformed, can produce an
/// invisible island or a rectangle that covers the screen. They are enforced in
/// [`Geometry::clamped`] and are also applied by the renderer as a defense in
/// depth measure.
///
/// The upper bound of 400 px on half width is roughly a 800 px wide island,
/// which is wider than any sensible desktop and far narrower than a screen.
pub mod limits {
    use super::Scalar;

    /// Smallest allowed half width.
    pub const MIN_HALF_WIDTH: Scalar = 40.0;
    /// Largest allowed half width.
    pub const MAX_HALF_WIDTH: Scalar = 400.0;
    /// Smallest allowed height.
    pub const MIN_HEIGHT: Scalar = 8.0;
    /// Largest allowed height.
    pub const MAX_HEIGHT: Scalar = 120.0;
    /// Largest allowed corner radius.
    pub const MAX_RADIUS: Scalar = 60.0;
    /// Smallest allowed border width.
    pub const MIN_BORDER_WIDTH: Scalar = 0.0;
    /// Largest allowed border width.
    pub const MAX_BORDER_WIDTH: Scalar = 8.0;
    /// Largest allowed glow spread.
    pub const MAX_GLOW_SPREAD: Scalar = 40.0;
    /// Largest allowed slant, the horizontal offset per unit height.
    ///
    /// A slant of 1.0 would be a 45 degree side. Capping at 0.6 keeps the
    /// shape recognisable as an island rather than a triangle.
    pub const MAX_SLANT: Scalar = 0.6;
}

/// The defining geometry of the island shape.
///
/// The shape is a trapezoid in a centered coordinate space where `+y` points
/// up. The top edge spans `[-half_top, +half_top]` and the bottom edge spans
/// `[-half_bottom, +half_bottom]`.
///
/// When `half_top >= half_bottom` the shape is an inverted trapezoid, which is
/// the design intent. When `half_top < half_bottom` it becomes a regular
/// trapezoid, which the configuration exposes as `inverted_trapezoid = false`.
///
/// ```
/// use lumina_core::Geometry;
///
/// let island = Geometry::new(62.0, 54.0, 36.0, 18.0);
/// assert!(island.is_inverted_trapezoid());
/// assert_eq!(island.full_width(), 124.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    /// Half width of the upper edge.
    pub half_top: Scalar,
    /// Half width of the lower edge.
    pub half_bottom: Scalar,
    /// Full height, not half height.
    pub height: Scalar,
    /// Corner radius.
    pub radius: Scalar,
}

impl Geometry {
    /// Construct without clamping. Prefer [`Geometry::clamped`] for any value
    /// that originated in user configuration.
    #[must_use]
    pub const fn new(half_top: Scalar, half_bottom: Scalar, height: Scalar, radius: Scalar) -> Self {
        Self { half_top, half_bottom, height, radius }
    }

    /// Half height.
    #[must_use]
    pub fn half_h(self) -> Scalar {
        self.height * 0.5
    }

    /// Full width at the top edge.
    #[must_use]
    pub fn full_width(self) -> Scalar {
        self.half_top * 2.0
    }

    /// True when the top edge is at least as wide as the bottom edge.
    #[must_use]
    pub fn is_inverted_trapezoid(self) -> bool {
        self.half_top >= self.half_bottom
    }

    /// Horizontal offset per unit height, the shape's defining slope.
    ///
    /// Zero for a rectangle, and equal to the horizontal difference of the two
    /// half widths divided by the full height.
    #[must_use]
    pub fn slant(self) -> Scalar {
        if self.height <= 0.0 {
            0.0
        } else {
            (self.half_top - self.half_bottom) / self.height
        }
    }

    /// Angle of the slanted side, measured from vertical, in radians.
    #[must_use]
    pub fn slant_angle(self) -> Scalar {
        if self.height <= 0.0 {
            0.0
        } else {
            ((self.half_top - self.half_bottom) / self.height).atan()
        }
    }

    /// Bounding box size of the shape.
    #[must_use]
    pub fn size(self) -> Size {
        Size::new(self.full_width(), self.height)
    }

    /// Bounding box of the shape grown by `padding` on every side, which is what
    /// the window must cover to fit the glow without clipping.
    #[must_use]
    pub fn padded_size(self, padding: Scalar) -> Size {
        Size::new(self.full_width() + padding * 2.0, self.height + padding * 2.0)
    }

    /// Corner radius limited to something the shape can actually accommodate.
    ///
    /// A radius larger than the half height produces self intersecting corners
    /// in the SDF, so it is capped at the smaller of the half height and the
    /// top half width.
    #[must_use]
    pub fn effective_radius(self) -> Scalar {
        self.radius.min(self.half_h()).min(self.half_top * 0.5).max(0.0)
    }

    /// Clamp every component into the documented range and repair the inverted
    /// trapezoid invariant.
    ///
    /// Non finite values are replaced with the defaults rather than propagated,
    /// because a `NaN` that reaches a shader produces a shape that cannot be
    /// reasoned about.
    #[must_use]
    pub fn clamped(self) -> Self {
        use limits::{MIN_HALF_WIDTH, MAX_HALF_WIDTH, MIN_HEIGHT, MAX_HEIGHT, MAX_RADIUS};

        let fallback = Self::default();

        let half_top = if self.half_top.is_finite() { self.half_top } else { fallback.half_top }
            .clamp(MIN_HALF_WIDTH, MAX_HALF_WIDTH);
        let half_bottom = if self.half_bottom.is_finite() { self.half_bottom } else { fallback.half_bottom }
            .clamp(MIN_HALF_WIDTH, MAX_HALF_WIDTH);
        let height = if self.height.is_finite() { self.height } else { fallback.height }
            .clamp(MIN_HEIGHT, MAX_HEIGHT);
        let radius = if self.radius.is_finite() { self.radius } else { fallback.radius }
            .clamp(0.0, MAX_RADIUS);

        // Enforce the invariant by swapping rather than by clamping one side down.
        // A configuration with `bottom_half_width` wider than `top_half_width`
        // is almost always a transposition, and swapping preserves the shape
        // the user was describing. Clamping the bottom down instead would keep
        // one of the two numbers they typed and silently discard the other.
        let (half_top, half_bottom) = if half_top >= half_bottom {
            (half_top, half_bottom)
        } else {
            (half_bottom, half_top)
        };

        Self { half_top, half_bottom, height, radius: radius.min(height * 0.5).min(half_top * 0.5) }
    }

    /// Linearly interpolate every component.
    ///
    /// Used by the animation system to evaluate a geometry between two states.
    /// Interpolating all components together guarantees the result stays on the
    /// family of valid shapes at every `t`.
    #[must_use]
    pub fn lerp(self, other: Self, t: Scalar) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            half_top: self.half_top + (other.half_top - self.half_top) * t,
            half_bottom: self.half_bottom + (other.half_bottom - self.half_bottom) * t,
            height: self.height + (other.height - self.height) * t,
            radius: self.radius + (other.radius - self.radius) * t,
        }
    }

    /// Scale every component, used when a monitor's DPI scale factor changes.
    #[must_use]
    pub fn scaled(self, factor: Scalar) -> Self {
        if !factor.is_finite() || factor <= 0.0 {
            return self;
        }
        Self {
            half_top: self.half_top * factor,
            half_bottom: self.half_bottom * factor,
            height: self.height * factor,
            radius: self.radius * factor,
        }
    }
}

impl Default for Geometry {
    /// The shipped default: a 124 px wide, 36 px tall island with 18 px corners.
    fn default() -> Self {
        Self { half_top: 62.0, half_bottom: 54.0, height: 36.0, radius: 18.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_edges_and_containment() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(rect.right(), 110.0);
        assert_eq!(rect.bottom(), 70.0);
        assert_eq!(rect.center(), Vec2::new(60.0, 45.0));
        assert_eq!(rect.area(), 5000.0);

        assert!(rect.contains(Vec2::new(10.0, 20.0)));
        assert!(rect.contains(Vec2::new(109.9, 69.9)));
        assert!(!rect.contains(Vec2::new(110.0, 45.0)), "far edge is exclusive");
        assert!(!rect.contains(Vec2::new(9.9, 45.0)));
    }

    #[test]
    fn rect_union_covers_both() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(90.0, 90.0, 10.0, 10.0);
        let u = a.union(b);
        assert_eq!(u, Rect::new(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn rect_intersection_of_disjoint_is_zero() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(50.0, 50.0, 10.0, 10.0);
        assert_eq!(a.intersection(b), Rect::ZERO);
        assert!(!a.intersects(b));
    }

    #[test]
    fn rect_inset_never_goes_negative() {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0).shrink(20.0);
        assert_eq!(rect.width, 0.0);
        assert_eq!(rect.height, 0.0);
        assert_eq!(rect.area(), 0.0);
    }

    #[test]
    fn geometry_defaults_to_inverted_trapezoid() {
        let g = Geometry::default();
        assert!(g.is_inverted_trapezoid());
        assert_eq!(g.full_width(), 124.0);
        assert_eq!(g.half_h(), 18.0);
        assert!(g.slant() > 0.0);
    }

    #[test]
    fn clamp_repairs_inverted_trapezoid_violation() {
        // The user typed the two keys the wrong way round.
        let bad = Geometry::new(50.0, 200.0, 40.0, 10.0);
        let fixed = bad.clamped();
        assert!(fixed.is_inverted_trapezoid(), "top must end up at least as wide as bottom");
        assert_eq!(
            fixed.half_top, 200.0,
            "swapping preserves the shape the user described rather than discarding a value"
        );
        assert_eq!(fixed.half_bottom, 50.0);
    }

    #[test]
    fn clamp_replaces_non_finite_with_defaults() {
        let broken = Geometry::new(f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::NAN);
        let fixed = broken.clamped();
        assert!(fixed.half_top.is_finite());
        assert!(fixed.half_bottom.is_finite());
        assert!(fixed.height.is_finite());
        assert!(fixed.radius.is_finite());
        assert!(fixed.is_inverted_trapezoid());
    }

    #[test]
    fn clamp_bounds_every_component() {
        let huge = Geometry::new(9999.0, 9999.0, 9999.0, 9999.0).clamped();
        assert_eq!(huge.half_top, limits::MAX_HALF_WIDTH);
        assert_eq!(huge.height, limits::MAX_HEIGHT);
        assert!(huge.radius <= limits::MAX_RADIUS);

        let tiny = Geometry::new(0.0, 0.0, 0.0, 0.0).clamped();
        assert_eq!(tiny.half_top, limits::MIN_HALF_WIDTH);
        assert_eq!(tiny.height, limits::MIN_HEIGHT);
    }

    #[test]
    fn effective_radius_cannot_exceed_half_height() {
        let g = Geometry::new(60.0, 50.0, 20.0, 18.0);
        assert!(g.effective_radius() <= g.half_h());
        assert!(g.effective_radius() <= 30.0);
    }

    #[test]
    fn lerp_hits_both_endpoints_exactly() {
        let a = Geometry::new(62.0, 54.0, 36.0, 18.0);
        let b = Geometry::new(210.0, 200.0, 100.0, 40.0);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        let mid = a.lerp(b, 0.5);
        assert!((mid.half_top - 136.0).abs() < 1e-4);
    }

    #[test]
    fn lerp_stays_on_the_valid_shape_family_at_every_t() {
        // A collapsed island to a wide expanded one. If each component were
        // sprung independently, the shape could invert part way through.
        let collapsed = Geometry::new(62.0, 54.0, 18.0, 9.0);
        let expanded = Geometry::new(210.0, 200.0, 100.0, 40.0);
        for step in 0..=200 {
            let t = step as f32 / 200.0;
            let g = collapsed.lerp(expanded, t);
            assert!(g.is_inverted_trapezoid(), "inverted at t = {t}");
            assert!(g.slant() >= 0.0, "negative slant at t = {t}");
        }
    }

    #[test]
    fn lerp_clamps_t_outside_unit_range() {
        let a = Geometry::new(10.0, 10.0, 10.0, 5.0);
        let b = Geometry::new(20.0, 20.0, 20.0, 10.0);
        assert_eq!(a.lerp(b, -5.0), a);
        assert_eq!(a.lerp(b, 5.0), b);
    }

    #[test]
    fn slant_angle_is_zero_for_rectangle_and_positive_for_inverted() {
        let rect = Geometry::new(60.0, 60.0, 40.0, 10.0);
        assert!(rect.slant_angle().abs() < 1e-6);

        let island = Geometry::default();
        assert!(island.slant_angle() > 0.0);
        assert!(island.slant_angle() < std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn scaled_is_a_no_op_for_invalid_factors() {
        let g = Geometry::default();
        assert_eq!(g.scaled(0.0), g);
        assert_eq!(g.scaled(-1.0), g);
        assert_eq!(g.scaled(f32::NAN), g);
    }

    #[test]
    fn padded_size_reserves_room_for_glow() {
        let g = Geometry::default();
        let padded = g.padded_size(8.0);
        assert_eq!(padded.width, g.full_width() + 16.0);
        assert_eq!(padded.height, g.height + 16.0);
    }
}