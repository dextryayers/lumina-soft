//! Pure mapping from Lumina's placement vocabulary to layer-shell vocabulary.
//!
//! Layer-shell places a surface using four anchors and four margins. Lumina
//! speaks in positions (`top-center`, `bottom-left`, ...) and signed offsets,
//! so this module is the single translation point. It also decides the
//! exclusive zone, the input region, the layer, and keyboard interactivity.
//!
//! The margins deliberately encode signed offsets, including negative margins
//! for centred positions. Layer-shell allows margins to be negative, and for a
//! surface anchored to both left and right the pair `(-x, x)` shifts a centred
//! surface right by `x` while keeping the centring constraint. Doing the
//! arithmetic here means the backend never reasons about it again.

use lumina_config::Position;

/// A layer-shell anchor bit set.
///
/// The bit values match `zwlr_layer_surface_v1.anchor`, so converting at the
/// protocol boundary is a cast, not a lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor(u8);

impl Anchor {
    /// Anchor to the top edge.
    pub const TOP: Self = Self(1);
    /// Anchor to the bottom edge.
    pub const BOTTOM: Self = Self(2);
    /// Anchor to the left edge.
    pub const LEFT: Self = Self(4);
    /// Anchor to the right edge.
    pub const RIGHT: Self = Self(8);

    /// The raw protocol bit set.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0 as u32
    }

    /// True when every bit in `other` is present.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Combine two anchors.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// True when the surface is anchored to both opposing horizontal edges.
    ///
    /// Such a surface is stretched to the monitor width unless the compositor
    /// is told an explicit width, which is how centred sizes are requested.
    #[must_use]
    pub const fn is_horizontally_centered(self) -> bool {
        self.contains(Self::LEFT) && self.contains(Self::RIGHT)
    }
}

/// Distances from each anchored edge, in physical pixels.
///
/// Only the margins for the anchored edges have meaning. The protocol accepts
/// the full set, and sending zeros for the unanchored sides is mandatory rather
/// than merely tidy: some compositors apply an anchored margin even when the
/// matching edge is absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Margins {
    /// Distance from the top edge.
    pub top: i32,
    /// Distance from the right edge.
    pub right: i32,
    /// Distance from the bottom edge.
    pub bottom: i32,
    /// Distance from the left edge.
    pub left: i32,
}

impl Margins {
    /// Construct a margin set.
    #[must_use]
    pub const fn new(top: i32, right: i32, bottom: i32, left: i32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// The margin on the vertically anchored edge.
    #[must_use]
    pub const fn vertical(self, position: Position) -> i32 {
        if position.is_top() {
            self.top
        } else {
            self.bottom
        }
    }
}

/// Which compositor layer the island asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    /// Below ordinary application windows.
    Background,
    /// Above the desktop but below ordinary windows.
    Bottom,
    /// Above ordinary and full-screen windows. This is what an island wants.
    #[default]
    Top,
    /// Above everything, including lock screens. Deliberately not used: an
    /// overlay that survives a lock screen is a security problem, not a feature.
    Overlay,
}

impl Layer {
    /// The layer an island should use.
    #[must_use]
    pub const fn for_island() -> Self {
        Self::Top
    }
}

/// How the surface participates in keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyboardInteractivity {
    /// Never receive keyboard input.
    None,
    /// Receive it when the user clicks the surface, or when the compositor
    /// decides the surface is the user's focus target.
    #[default]
    OnDemand,
    /// Always receive it, like a modal dialog. Never appropriate for an island.
    Exclusive,
}

impl KeyboardInteractivity {
    /// The mode an island should request.
    ///
    /// `OnDemand` lets the island be operated from the keyboard while it is
    /// expanded without ever stealing focus from the application the user was
    /// typing in. `Exclusive` would do exactly that.
    #[must_use]
    pub const fn for_island() -> Self {
        Self::OnDemand
    }
}

/// The complete layer-surface request derived from user configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerPlacement {
    /// Which edges to anchor to.
    pub anchor: Anchor,
    /// Margins from those edges.
    pub margins: Margins,
    /// Exclusive zone in surface-local pixels, zero to ignore the island.
    pub exclusive_zone: i32,
    /// Which layer to sit on.
    pub layer: Layer,
    /// Keyboard interactivity mode.
    pub keyboard: KeyboardInteractivity,
    /// True when the input region must be empty.
    pub click_through: bool,
}

/// The anchor set for a position.
///
/// Centred positions anchor to both horizontal edges so the compositor centres
/// the surface. Edge positions anchor to one.
#[must_use]
pub const fn anchor_for(position: Position) -> Anchor {
    let horizontal = match position {
        Position::TopLeft | Position::BottomLeft => Anchor::LEFT,
        Position::TopRight | Position::BottomRight => Anchor::RIGHT,
        Position::TopCenter | Position::BottomCenter => Anchor::LEFT.union(Anchor::RIGHT),
    };
    let vertical = if position.is_top() {
        Anchor::TOP
    } else {
        Anchor::BOTTOM
    };
    horizontal.union(vertical)
}

/// The margins that express the user's logical offsets.
///
/// # Semantics
///
/// * Positive `offset_x` means "move right" and positive `offset_y` means
///   "move down", matching [`crate::placement`].
/// * On an edge anchor an offset that would push the surface off the monitor is
///   clamped to the edge, matching the X11 and Windows backends.
/// * On a centred anchor the pair of opposing margins shifts the surface
///   without breaking the centring constraint.
/// * The vertical offset is clamped on every position, because an island
///   floating above the top edge or below the bottom edge is never intended.
#[must_use]
pub const fn margins_for(position: Position, offset_x: i32, offset_y: i32) -> Margins {
    let horizontal = match position {
        Position::TopLeft | Position::BottomLeft => Margins::new(0, 0, 0, clamp_positive(offset_x)),
        Position::TopRight | Position::BottomRight => {
            Margins::new(0, clamp_positive(-offset_x), 0, 0)
        }
        Position::TopCenter | Position::BottomCenter => {
            Margins::new(0, offset_x, 0, -offset_x)
        }
    };
    if position.is_top() {
        Margins::new(
            clamp_positive(offset_y),
            horizontal.right,
            0,
            horizontal.left,
        )
    } else {
        Margins::new(
            0,
            horizontal.right,
            clamp_positive(-offset_y),
            horizontal.left,
        )
    }
}

/// Clamp a margin to the non-negative range.
///
/// A negative margin on the anchored edge would move the surface off screen;
/// on the opposite edge it is the mechanism that shifts a centred surface, so
/// this helper is only used on anchored edges.
const fn clamp_positive(value: i32) -> i32 {
    if value > 0 {
        value
    } else {
        0
    }
}

/// The exclusive zone to request.
///
/// * `reserve_space` false requests [`EXCLUSIVE_IGNORE`], so maximised windows
///   pass underneath the island. This is the default because an island that
///   silently shrinks every window is a surprise.
/// * `reserve_space` true requests the surface extent plus its margin on the
///   anchored edge, which is exactly the strip the island occupies.
///
/// The protocol uses a signed integer where positive means a fixed reservation,
/// zero means "ignore the surface", and negative means "decide for me". The
/// result never uses the negative form: a compositor that reserves a different
/// amount than Lumina drew would produce a visible gap, so the value is always
/// explicit.
#[must_use]
pub fn exclusive_zone(reserve_space: bool, extent: u32, margin: i32) -> i32 {
    if !reserve_space {
        return EXCLUSIVE_IGNORE;
    }
    let total = i64::from(extent) + i64::from(margin.max(0));
    total.clamp(0, i64::from(i32::MAX)) as i32
}

/// Do not reserve any space.
pub const EXCLUSIVE_IGNORE: i32 = 0;

/// Build the complete layer-surface request.
///
/// `extent` is the surface height in physical pixels, which is the dimension
/// that matters for a vertically anchored island. The margin is added so the
/// reserved strip includes the gap between the island and the screen edge.
#[must_use]
pub fn layer_placement(
    position: Position,
    offset_x: i32,
    offset_y: i32,
    reserve_space: bool,
    extent: u32,
    click_through: bool,
) -> LayerPlacement {
    let margins = margins_for(position, offset_x, offset_y);
    LayerPlacement {
        anchor: anchor_for(position),
        margins,
        exclusive_zone: exclusive_zone(reserve_space, extent, margins.vertical(position)),
        layer: Layer::for_island(),
        keyboard: KeyboardInteractivity::for_island(),
        click_through,
    }
}

/// Which part of a surface accepts pointer input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputRegion {
    /// The whole surface accepts input.
    #[default]
    Full,
    /// Nothing accepts input, so clicks reach whatever is underneath.
    Empty,
}

/// The input region for a click-through state.
///
/// Wayland expresses click-through by setting an empty input region rather than
/// by a window style. That is better than the Windows and X11 approaches: the
/// change can be made at any time without recreating the surface, so toggling
/// click-through on hover has no flicker.
#[must_use]
pub const fn input_region(click_through: bool) -> InputRegion {
    if click_through {
        InputRegion::Empty
    } else {
        InputRegion::Full
    }
}

/// Whether moving to another output requires a new surface.
///
/// A layer surface is bound to one `wl_output` for its entire lifetime: there
/// is no protocol request to move it. Changing the target output therefore
/// means destroying the surface and creating a new one on the new output, while
/// keeping the wgpu swapchain and the render state alive. The backend uses this
/// predicate to make that decision in one place.
#[must_use]
pub const fn output_changed(current: Option<usize>, requested: Option<usize>) -> bool {
    match (current, requested) {
        (Some(a), Some(b)) => a != b,
        (None, None) => false,
        // A newly requested concrete output, or a request to follow the
        // compositor's default, both need a fresh surface.
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_use_the_protocol_bit_values() {
        assert_eq!(Anchor::TOP.bits(), 1);
        assert_eq!(Anchor::BOTTOM.bits(), 2);
        assert_eq!(Anchor::LEFT.bits(), 4);
        assert_eq!(Anchor::RIGHT.bits(), 8);
    }

    #[test]
    fn every_position_anchors_to_exactly_one_vertical_edge() {
        for position in Position::ALL {
            let anchor = anchor_for(position);
            assert_eq!(
                anchor.contains(Anchor::TOP),
                position.is_top(),
                "{position} vertical anchor disagrees with the position"
            );
            assert_eq!(
                anchor.contains(Anchor::BOTTOM),
                !position.is_top(),
                "{position} must anchor to exactly one vertical edge"
            );
        }
    }

    #[test]
    fn horizontal_anchors_follow_the_alignment() {
        for position in Position::ALL {
            let anchor = anchor_for(position);
            match position.horizontal_fraction() {
                0.0 => {
                    assert!(anchor.contains(Anchor::LEFT));
                    assert!(!anchor.contains(Anchor::RIGHT));
                }
                1.0 => {
                    assert!(anchor.contains(Anchor::RIGHT));
                    assert!(!anchor.contains(Anchor::LEFT));
                }
                _ => assert!(
                    anchor.is_horizontally_centered(),
                    "{position} must anchor to both horizontal edges"
                ),
            }
        }
    }

    #[test]
    fn top_centered_margins_shift_the_surface_without_breaking_centering() {
        let margins = margins_for(Position::TopCenter, 40, 12);
        assert_eq!(margins.top, 12);
        assert_eq!(margins.left, -40);
        assert_eq!(margins.right, 40);
        assert_eq!(margins.bottom, 0);
    }

    #[test]
    fn negative_center_offsets_shift_left() {
        let margins = margins_for(Position::TopCenter, -30, 0);
        assert_eq!(margins.left, 30);
        assert_eq!(margins.right, -30);
    }

    #[test]
    fn left_anchors_clamp_offsets_that_would_leave_the_monitor() {
        let inside = margins_for(Position::BottomLeft, 24, -8);
        assert_eq!(inside.left, 24);
        assert_eq!(inside.bottom, 8, "negative y moves up from the bottom edge");

        let outside = margins_for(Position::TopLeft, -24, -8);
        assert_eq!(outside.left, 0, "the island cannot move left of the edge");
        assert_eq!(outside.top, 0, "the island cannot move above the edge");
    }

    #[test]
    fn right_anchors_express_offsets_as_distance_from_the_right_edge() {
        let inside = margins_for(Position::TopRight, -24, 6);
        assert_eq!(inside.right, 24);
        assert_eq!(inside.top, 6);

        let outside = margins_for(Position::TopRight, 24, 6);
        assert_eq!(outside.right, 0, "positive x cannot push past the edge");
    }

    #[test]
    fn bottom_anchors_put_the_margin_on_the_bottom_edge() {
        let margins = margins_for(Position::BottomRight, 0, 16);
        assert_eq!(margins.bottom, 0, "down from the bottom edge is clamped");
        assert_eq!(margins.top, 0);
        let margins = margins_for(Position::BottomRight, 0, -16);
        assert_eq!(margins.bottom, 16);
    }

    #[test]
    fn vertical_margin_selects_the_anchored_edge() {
        let margins = margins_for(Position::TopCenter, 0, 9);
        assert_eq!(margins.vertical(Position::TopCenter), 9);
        let margins = margins_for(Position::BottomCenter, 0, -9);
        assert_eq!(margins.vertical(Position::BottomCenter), 9);
    }

    #[test]
    fn reserving_space_accounts_for_the_margin() {
        assert_eq!(exclusive_zone(true, 36, 12), 48);
        assert_eq!(exclusive_zone(true, 36, 0), 36);
        assert_eq!(exclusive_zone(false, 36, 12), EXCLUSIVE_IGNORE);
    }

    #[test]
    fn reserving_space_never_overflows() {
        assert_eq!(exclusive_zone(true, u32::MAX, 100), i32::MAX);
        assert_eq!(exclusive_zone(true, 0, -50), 0);
    }

    #[test]
    fn the_island_reserves_the_strip_it_occupies() {
        let placement = layer_placement(Position::TopCenter, 0, 12, true, 36, false);
        assert_eq!(placement.exclusive_zone, 48);
        assert_eq!(placement.anchor, Anchor::TOP.union(Anchor::LEFT).union(Anchor::RIGHT));
    }

    #[test]
    fn the_island_ignores_the_exclusive_zone_by_default() {
        let placement = layer_placement(Position::TopCenter, 0, 12, false, 36, false);
        assert_eq!(placement.exclusive_zone, EXCLUSIVE_IGNORE);
    }

    #[test]
    fn click_through_maps_to_an_empty_input_region() {
        assert_eq!(input_region(true), InputRegion::Empty);
        assert_eq!(input_region(false), InputRegion::Full);
    }

    #[test]
    fn the_island_sits_above_windows_but_below_the_lock_screen() {
        assert_eq!(Layer::for_island(), Layer::Top);
        assert_ne!(Layer::for_island(), Layer::Overlay);
    }

    #[test]
    fn keyboard_interactivity_is_on_demand_not_exclusive() {
        assert_eq!(
            KeyboardInteractivity::for_island(),
            KeyboardInteractivity::OnDemand
        );
        assert_eq!(KeyboardInteractivity::default(), KeyboardInteractivity::OnDemand);
    }

    #[test]
    fn changing_or_clearing_the_output_requires_a_new_surface() {
        assert!(!output_changed(Some(0), Some(0)));
        assert!(!output_changed(None, None));
        assert!(output_changed(Some(0), Some(1)));
        assert!(output_changed(Some(0), None));
        assert!(output_changed(None, Some(1)));
    }

    #[test]
    fn layer_placement_carries_the_click_through_and_keyboard_policy() {
        let placement = layer_placement(Position::BottomLeft, 10, -10, false, 36, true);
        assert!(placement.click_through);
        assert_eq!(placement.keyboard, KeyboardInteractivity::OnDemand);
        assert_eq!(placement.layer, Layer::Top);
    }
}
