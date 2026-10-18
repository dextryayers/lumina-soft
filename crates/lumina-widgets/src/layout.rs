//! Flex row layout: the single layout model Lumina needs.
//!
//! # The model
//!
//! Widgets are arranged left to right in a horizontal strip. Each widget has a
//! desired size, a flex weight, and a margin. The layout engine:
//!
//! 1. Measures every widget.
//! 2. If the total exceeds the available width, shrinks proportionally to
//!    `(min_width, desired_width)`.
//! 3. If the total is less than the available width, distributes the surplus by
//!    flex weight.
//! 4. Places widgets left to right with their margins.
//!
//! # Why flex weights
//!
//! A widget that shows a title wants more room than one that shows an icon. Flex
//! weights let the user express that without hardcoding sizes, and they make the
//! layout survive a widget being added or removed without a redesign.

use lumina_core::geometry::{Rect, Scalar, Size, Vec2};

/// A flex row layout.
#[derive(Debug, Clone)]
pub struct FlexLayout {
    available: Size,
    items: Vec<Item>,
    cursor: Scalar,
}

#[derive(Debug, Clone, Copy)]
struct Item {
    desired: Size,
    flex: Scalar,
    margin_leading: Scalar,
    min_width: Scalar,
}

impl FlexLayout {
    /// Create a layout for a container of the given size.
    #[must_use]
    pub fn new(available: Size) -> Self {
        Self { available, items: Vec::new(), cursor: 0.0 }
    }

    /// Add an item.
    pub fn add(&mut self, desired: Size, flex: Scalar, margin_leading: Scalar) {
        self.items.push(Item {
            desired,
            flex: flex.max(0.0),
            margin_leading: margin_leading.max(0.0),
            min_width: desired.width.min(1.0),
        });
    }

    /// Compute the final rectangles.
    ///
    /// Returns one rectangle per item, in the order they were added.
    pub fn finish(mut self) -> Vec<Rect> {
        let total_desired: Scalar = self.items.iter().map(|i| i.desired.width + i.margin_leading).sum();
        let available = self.available.width;

        let mut rects = Vec::with_capacity(self.items.len());

        if total_desired <= available || self.items.is_empty() {
            // Everything fits. Distribute the surplus by flex weight.
            let surplus = available - total_desired;
            let total_flex: Scalar = self.items.iter().map(|i| i.flex).sum();
            for item in &self.items {
                let extra = if total_flex > 0.0 {
                    surplus * (item.flex / total_flex)
                } else {
                    0.0
                };
                let width = item.desired.width + extra;
                rects.push(Rect::new(self.cursor, 0.0, width, self.available.height));
                self.cursor += width + item.margin_leading;
            }
        } else {
            // Shrink proportionally to (min_width, desired_width).
            let overflow = total_desired - available;
            let shrinkable: Scalar = self
                .items
                .iter()
                .map(|i| (i.desired.width - i.min_width).max(0.0))
                .sum();
            for item in &self.items {
                let shrink = if shrinkable > 0.0 {
                    overflow * ((item.desired.width - item.min_width).max(0.0) / shrinkable)
                } else {
                    0.0
                };
                let width = (item.desired.width - shrink).max(item.min_width);
                rects.push(Rect::new(self.cursor, 0.0, width, self.available.height));
                self.cursor += width + item.margin_leading;
            }
        }

        rects
    }
}

/// Layout a set of widgets into a horizontal strip.
///
/// `sizes` and `flexes` must be the same length. Returns one rectangle per
/// widget, in the same order.
#[must_use]
pub fn layout_row(available: Size, sizes: &[Size], flexes: &[Scalar], margins: &[Scalar]) -> Vec<Rect> {
    let mut layout = FlexLayout::new(available);
    for i in 0..sizes.len() {
        layout.add(sizes[i], flexes.get(i).copied().unwrap_or(1.0), margins.get(i).copied().unwrap_or(0.0));
    }
    layout.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_layout_produces_no_rectangles() {
        assert!(FlexLayout::new(Size::new(100.0, 40.0)).finish().is_empty());
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
    fn surplus_is_distributed_by_flex_weight() {
        let mut layout = FlexLayout::new(Size::new(300.0, 40.0));
        layout.add(Size::new(100.0, 40.0), 3.0, 0.0);
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        let rects = layout.finish();
        // Surplus is 100, split 3:1, so 75 and 25.
        assert!((rects[0].width - 175.0).abs() < 1e-6);
        assert!((rects[1].width - 125.0).abs() < 1e-6);
    }

    #[test]
    fn overflow_shrinks_proportionally() {
        let mut layout = FlexLayout::new(Size::new(150.0, 40.0));
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        let rects = layout.finish();
        // Total desired is 200, available is 150, overflow is 50.
        // Each shrinks by 25, so both are 75.
        assert!((rects[0].width - 75.0).abs() < 1e-6);
        assert!((rects[1].width - 75.0).abs() < 1e-6);
    }

    #[test]
    fn margins_are_respected() {
        let mut layout = FlexLayout::new(Size::new(300.0, 40.0));
        layout.add(Size::new(100.0, 40.0), 1.0, 10.0);
        layout.add(Size::new(100.0, 40.0), 1.0, 0.0);
        let rects = layout.finish();
        assert_eq!(rects[0].x, 0.0);
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
        assert!(layout_row(Size::new(100.0, 40.0), &[], &[], &[]).is_empty());
    }

    #[test]
    fn layout_row_tolerates_mismatched_flex_and_margin_lengths() {
        let sizes = vec![Size::new(100.0, 40.0)];
        let rects = layout_row(Size::new(200.0, 40.0), &sizes, &[], &[]);
        assert_eq!(rects.len(), 1);
        // No flex weight means no surplus distribution.
        assert_eq!(rects[0].width, 100.0);
    }
}