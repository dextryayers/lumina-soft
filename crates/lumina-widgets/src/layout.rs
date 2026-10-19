//! Flex row layout: the single layout model Lumina needs.
//!
//! # The model
//!
//! Widgets are arranged left to right in a horizontal strip. Each widget has a
//! desired size, a flex weight, a margin, and optional min/max constraints. The
//! layout engine:
//!
//! 1. Measures every widget.
//! 2. If the total exceeds the available width, shrinks proportionally to
//!    `(min_width, desired_width)`.
//! 3. If the total is less than the available width, distributes the surplus by
//!    flex weight, respecting each widget's `max_width`.
//! 4. Places widgets left to right with their margins.
//!
//! # Alignment
//!
//! Widgets can be aligned vertically within the strip: top, center, or bottom.
//! The alignment is applied after the horizontal layout is computed.
//!
//! # Why flex weights
//!
//! A widget that shows a title wants more room than one that shows an icon. Flex
//! weights let the user express that without hardcoding sizes, and they make the
//! layout survive a widget being added or removed without a redesign.

use lumina_core::geometry::{Rect, Scalar, Size};

/// Vertical alignment of widgets within the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlign {
    /// Against the top edge.
    Top,
    /// Centered vertically.
    #[default]
    Center,
    /// Against the bottom edge.
    Bottom,
}

/// A flex row layout.
#[derive(Debug, Clone)]
pub struct FlexLayout {
    available: Size,
    items: Vec<Item>,
    cursor: Scalar,
    align: VerticalAlign,
    spacing: Scalar,
}

#[derive(Debug, Clone, Copy)]
struct Item {
    desired: Size,
    flex: Scalar,
    margin_leading: Scalar,
    min_width: Scalar,
    max_width: Scalar,
}

impl FlexLayout {
    /// Create a layout for a container of the given size.
    #[must_use]
    pub fn new(available: Size) -> Self {
        Self {
            available,
            items: Vec::new(),
            cursor: 0.0,
            align: VerticalAlign::Center,
            spacing: 0.0,
        }
    }

    /// Set the vertical alignment.
    pub fn set_align(&mut self, align: VerticalAlign) {
        self.align = align;
    }

    /// Set the spacing between widgets.
    pub fn set_spacing(&mut self, spacing: Scalar) {
        self.spacing = spacing.max(0.0);
    }

    /// Add an item.
    pub fn add(&mut self, desired: Size, flex: Scalar, margin_leading: Scalar) {
        self.add_with_constraints(desired, flex, margin_leading, 1.0, Scalar::INFINITY);
    }

    /// Add an item with min/max width constraints.
    pub fn add_with_constraints(
        &mut self,
        desired: Size,
        flex: Scalar,
        margin_leading: Scalar,
        min_width: Scalar,
        max_width: Scalar,
    ) {
        self.items.push(Item {
            desired,
            flex: flex.max(0.0),
            margin_leading: margin_leading.max(0.0),
            min_width: min_width.max(0.0),
            max_width: max_width.max(min_width),
        });
    }

    /// Compute the final rectangles.
    ///
    /// Returns one rectangle per item, in the order they were added.
    pub fn finish(mut self) -> Vec<Rect> {
        let total_desired: Scalar = self
            .items
            .iter()
            .map(|i| i.desired.width + i.margin_leading)
            .sum::<Scalar>()
            + self.spacing * (self.items.len().saturating_sub(1) as Scalar);
        let available = self.available.width;

        let mut rects = Vec::with_capacity(self.items.len());

        if total_desired <= available || self.items.is_empty() {
            // Everything fits. Place at desired width, left-aligned, so the island
            // wraps its content instead of stretching widgets to fill the screen.
            for item in &self.items {
                let width = item.desired.width.min(item.max_width);
                let y = self.align_y(item.desired.height);
                self.cursor += item.margin_leading;
                rects.push(Rect::new(self.cursor, y, width, item.desired.height));
                self.cursor += width + self.spacing;
            }
        } else {
            // Shrink proportionally to (min_width, desired_width).
            let overflow = total_desired - available;
            let shrinkable: Scalar = self
                .items
                .iter()
                .map(|i| (i.desired.width - i.min_width).max(0.0) * i.flex)
                .sum();
            for item in &self.items {
                let shrink = if shrinkable > 0.0 {
                    overflow
                        * (((item.desired.width - item.min_width).max(0.0) * item.flex)
                            / shrinkable)
                } else {
                    0.0
                };
                let width = (item.desired.width - shrink)
                    .max(item.min_width)
                    .min(item.max_width);
                let y = self.align_y(item.desired.height);
                self.cursor += item.margin_leading;
                rects.push(Rect::new(self.cursor, y, width, item.desired.height));
                self.cursor += width + self.spacing;
            }
        }

        rects
    }

    /// Compute the y offset for a widget of the given height.
    fn align_y(&self, widget_height: Scalar) -> Scalar {
        match self.align {
            VerticalAlign::Top => 0.0,
            VerticalAlign::Center => (self.available.height - widget_height) * 0.5,
            VerticalAlign::Bottom => (self.available.height - widget_height).max(0.0),
        }
    }

    /// The total width that would be consumed by all items.
    #[must_use]
    pub fn total_width(&self) -> Scalar {
        self.items
            .iter()
            .map(|i| i.desired.width + i.margin_leading)
            .sum::<Scalar>()
            + self.spacing * (self.items.len().saturating_sub(1) as Scalar)
    }

    /// The number of items in the layout.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when the layout has no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Layout a set of widgets into a horizontal strip.
///
/// `sizes` and `flexes` must be the same length. Returns one rectangle per
/// widget, in the same order.
#[must_use]
pub fn layout_row(
    available: Size,
    sizes: &[Size],
    flexes: &[Scalar],
    margins: &[Scalar],
) -> Vec<Rect> {
    let mut layout = FlexLayout::new(available);
    for (i, size) in sizes.iter().enumerate() {
        layout.add(
            *size,
            flexes.get(i).copied().unwrap_or(1.0),
            margins.get(i).copied().unwrap_or(0.0),
        );
    }
    layout.finish()
}

/// Layout a set of widgets with alignment and spacing.
#[must_use]
pub fn layout_row_with_options(
    available: Size,
    sizes: &[Size],
    flexes: &[Scalar],
    margins: &[Scalar],
    align: VerticalAlign,
    spacing: Scalar,
) -> Vec<Rect> {
    let mut layout = FlexLayout::new(available);
    layout.set_align(align);
    layout.set_spacing(spacing);
    for (i, size) in sizes.iter().enumerate() {
        layout.add(
            *size,
            flexes.get(i).copied().unwrap_or(1.0),
            margins.get(i).copied().unwrap_or(0.0),
        );
    }
    layout.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_layout_produces_no_rectangles() {
        assert_eq!(
            FlexLayout::new(Size::new(100.0, 40.0)).finish(),
            [] as [lumina_core::Rect; 0]
        );
    }

    #[test]
    fn items_that_fit_are_placed_left_to_right() {
        let mut layout = FlexLayout::new(Size::new(300.0, 40.0));
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        let rects = layout.finish();
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].x, 0.0);
        assert_eq!(rects[1].x, 100.0);
    }

    #[test]
    fn items_that_fit_keep_their_desired_width() {
        let mut layout = FlexLayout::new(Size::new(300.0, 40.0));
        layout.add(Size::new(100.0, 40.0), 3.0, 0.0);
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        let rects = layout.finish();
        assert_eq!(rects[0].width, 100.0);
        assert_eq!(rects[1].width, 100.0);
    }

    #[test]
    fn overflow_shrinks_proportionally() {
        let mut layout = FlexLayout::new(Size::new(150.0, 40.0));
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        let rects = layout.finish();
        assert!((rects[0].width - 75.0).abs() < 1e-6);
        assert!((rects[1].width - 75.0).abs() < 1e-6);
    }

    #[test]
    fn margins_are_respected() {
        let mut layout = FlexLayout::new(Size::new(300.0, 40.0));
        layout.add(Size::new(100.0, 40.0), 1.0, 10.0);
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        let rects = layout.finish();
        assert_eq!(rects[0].x, 10.0);
        assert_eq!(rects[1].x, 110.0);
    }

    #[test]
    fn a_widget_never_shrinks_below_its_minimum() {
        let mut layout = FlexLayout::new(Size::new(50.0, 40.0));
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        let rects = layout.finish();
        assert!(rects[0].width >= 1.0, "must not shrink below min_width");
    }

    #[test]
    fn layout_row_matches_the_manual_api() {
        let sizes = vec![Size::new(100.0, 40.0), Size::new(50.0, 40.0)];
        let flexes = vec![1.0, 1.0];
        let margins = vec![0.0, 0.0];
        let rects = layout_row(Size::new(200.0, 40.0), &sizes, &flexes, &margins);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].x, 0.0);
        assert_eq!(rects[1].x, 100.0);
    }

    #[test]
    fn layout_row_handles_empty_input() {
        assert_eq!(
            layout_row(Size::new(100.0, 40.0), &[], &[], &[]),
            [] as [lumina_core::Rect; 0]
        );
    }

    #[test]
    fn layout_row_tolerates_mismatched_flex_and_margin_lengths() {
        let sizes = vec![Size::new(100.0, 40.0)];
        let rects = layout_row(Size::new(200.0, 40.0), &sizes, &[], &[]);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].width, 100.0);
    }

    #[test]
    fn vertical_align_top_places_widgets_at_the_top() {
        let mut layout = FlexLayout::new(Size::new(200.0, 40.0));
        layout.set_align(VerticalAlign::Top);
        layout.add(Size::new(50.0, 20.0), 1.0, 0.0);
        let rects = layout.finish();
        assert_eq!(rects[0].y, 0.0);
    }

    #[test]
    fn vertical_align_center_places_widgets_in_the_middle() {
        let mut layout = FlexLayout::new(Size::new(200.0, 40.0));
        layout.set_align(VerticalAlign::Center);
        layout.add(Size::new(50.0, 20.0), 1.0, 0.0);
        let rects = layout.finish();
        assert_eq!(rects[0].y, 10.0);
    }

    #[test]
    fn vertical_align_bottom_places_widgets_at_the_bottom() {
        let mut layout = FlexLayout::new(Size::new(200.0, 40.0));
        layout.set_align(VerticalAlign::Bottom);
        layout.add(Size::new(50.0, 20.0), 1.0, 0.0);
        let rects = layout.finish();
        assert_eq!(rects[0].y, 20.0);
    }

    #[test]
    fn spacing_is_applied_between_widgets() {
        let mut layout = FlexLayout::new(Size::new(300.0, 40.0));
        layout.set_spacing(10.0);
        layout.add(Size::new(50.0, 40.0), 1.0, 0.0);
        layout.add(Size::new(50.0, 40.0), 1.0, 0.0);
        let rects = layout.finish();
        assert_eq!(rects[0].x, 0.0);
        assert_eq!(rects[1].x, 60.0);
    }

    #[test]
    fn max_width_constrains_widget_size() {
        let mut layout = FlexLayout::new(Size::new(300.0, 40.0));
        layout.add_with_constraints(Size::new(200.0, 40.0), 1.0, 0.0, 1.0, 100.0);
        let rects = layout.finish();
        assert_eq!(rects[0].width, 100.0);
    }

    #[test]
    fn min_width_prevents_shrinking_below_minimum() {
        let mut layout = FlexLayout::new(Size::new(50.0, 40.0));
        layout.add_with_constraints(Size::new(100.0, 40.0), 1.0, 0.0, 20.0, Scalar::INFINITY);
        let rects = layout.finish();
        assert!(rects[0].width >= 20.0);
    }

    #[test]
    fn layout_row_with_options_applies_alignment_and_spacing() {
        let sizes = vec![Size::new(50.0, 20.0), Size::new(50.0, 20.0)];
        let flexes = vec![1.0, 1.0];
        let margins = vec![0.0, 0.0];
        let rects = layout_row_with_options(
            Size::new(200.0, 40.0),
            &sizes,
            &flexes,
            &margins,
            VerticalAlign::Bottom,
            10.0,
        );
        assert_eq!(rects[0].y, 20.0);
        assert_eq!(rects[1].x, 60.0);
    }

    #[test]
    fn total_width_sums_all_items() {
        let mut layout = FlexLayout::new(Size::new(300.0, 40.0));
        layout.add(Size::new(50.0, 40.0), 1.0, 5.0);
        layout.add(Size::new(60.0, 40.0), 1.0, 10.0);
        assert_eq!(layout.total_width(), 125.0);
    }

    #[test]
    fn layout_len_and_is_empty_work() {
        let mut layout = FlexLayout::new(Size::new(100.0, 40.0));
        assert!(layout.is_empty());
        assert_eq!(layout.len(), 0);
        layout.add(Size::new(50.0, 40.0), 1.0, 0.0);
        assert!(!layout.is_empty());
        assert_eq!(layout.len(), 1);
    }
}
