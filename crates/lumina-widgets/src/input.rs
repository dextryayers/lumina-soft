//! Input routing: hit testing, hover state, focus management, and scroll handling.
//!
//! # The routing model
//!
//! Input events are routed to widgets by hit testing against their layout
//! rectangles. The widget under the pointer receives the event first. If it
//! does not consume the event, the next widget under the pointer receives it.
//!
//! # Hover state
//!
//! The router tracks which widget is currently hovered. When the pointer moves,
//! the router determines which widget is under the pointer and updates the
//! hover state. The hovered widget receives a [`WidgetInput::PointerEnter`] event
//! when the pointer enters its bounds, and a [`WidgetInput::PointerLeave`] event
//! when the pointer leaves.
//!
//! # Focus management
//!
//! The router manages keyboard focus. When a widget consumes a click event, it
//! receives keyboard focus. The focused widget receives key press and key release
//! events. Focus can be transferred by clicking on another widget or by pressing
//! the Tab key.
//!
//! # Scroll handling
//!
//! Scroll events are routed to the widget under the pointer. If the widget does
//! not consume the scroll event, the router can optionally scroll the entire
//! island content.

use lumina_core::geometry::{Rect, Scalar, Vec2};

use crate::widget::{Widget, WidgetContext, WidgetInput};

/// The input router.
///
/// Routes input events to widgets by hit testing, tracks hover state, and
/// manages keyboard focus.
#[derive(Debug, Clone)]
pub struct InputRouter {
    /// The rectangles assigned to each widget, in registration order.
    rects: Vec<Rect>,
    /// The id of the currently hovered widget.
    hovered_id: Option<String>,
    /// The id of the currently focused widget.
    focused_id: Option<String>,
    /// The last pointer position.
    last_pointer_pos: Vec2,
    /// Whether the pointer is inside the island.
    pointer_inside: bool,
    /// The scroll offset, in pixels.
    scroll_offset: Scalar,
    /// The maximum scroll offset.
    max_scroll_offset: Scalar,
}

impl InputRouter {
    /// Create a new input router.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rects: Vec::new(),
            hovered_id: None,
            focused_id: None,
            last_pointer_pos: Vec2::ZERO,
            pointer_inside: false,
            scroll_offset: 0.0,
            max_scroll_offset: 0.0,
        }
    }

    /// Update the layout rectangles.
    pub fn update_rects(&mut self, rects: Vec<Rect>) {
        self.rects = rects;
    }

    /// Get the currently hovered widget id.
    #[must_use]
    pub fn hovered_id(&self) -> Option<&str> {
        self.hovered_id.as_deref()
    }

    /// Get the currently focused widget id.
    #[must_use]
    pub fn focused_id(&self) -> Option<&str> {
        self.focused_id.as_deref()
    }

    /// Get the last pointer position.
    #[must_use]
    pub fn last_pointer_pos(&self) -> Vec2 {
        self.last_pointer_pos
    }

    /// Whether the pointer is inside the island.
    #[must_use]
    pub fn pointer_inside(&self) -> bool {
        self.pointer_inside
    }

    /// Get the current scroll offset.
    #[must_use]
    pub fn scroll_offset(&self) -> Scalar {
        self.scroll_offset
    }

    /// Set the maximum scroll offset.
    pub fn set_max_scroll_offset(&mut self, max: Scalar) {
        self.max_scroll_offset = max.max(0.0);
        self.scroll_offset = self.scroll_offset.min(self.max_scroll_offset);
    }

    /// Route a pointer move event.
    ///
    /// Returns the id of the widget that is now hovered, if any.
    pub fn route_pointer_move(
        &mut self,
        pos: Vec2,
        widgets: &mut [Box<dyn Widget>],
        ctx: &WidgetContext,
    ) -> Option<String> {
        self.last_pointer_pos = pos;
        self.pointer_inside = true;

        let new_hovered = self.hit_test(pos);
        if new_hovered != self.hovered_id {
            // Leave the old widget
            if let Some(old_id) = &self.hovered_id {
                if let Some(idx) = self.widget_index(old_id, widgets) {
                    if let Some(rect) = self.rects.get(idx) {
                        widgets[idx].on_input(&WidgetInput::PointerLeave, ctx, *rect);
                    }
                }
            }
            // Enter the new widget
            if let Some(new_id) = &new_hovered {
                if let Some(idx) = self.widget_index(new_id, widgets) {
                    if let Some(rect) = self.rects.get(idx) {
                        widgets[idx].on_input(&WidgetInput::PointerEnter, ctx, *rect);
                    }
                }
            }
            self.hovered_id = new_hovered.clone();
        }

        // Send the move event to the hovered widget
        if let Some(hovered_id) = &self.hovered_id {
            if let Some(idx) = self.widget_index(hovered_id, widgets) {
                if let Some(rect) = self.rects.get(idx) {
                    widgets[idx].on_input(&WidgetInput::PointerMove(pos), ctx, *rect);
                }
            }
        }

        self.hovered_id.clone()
    }

    /// Route a pointer leave event.
    pub fn route_pointer_leave(
        &mut self,
        widgets: &mut [Box<dyn Widget>],
        ctx: &WidgetContext,
    ) {
        self.pointer_inside = false;
        if let Some(old_id) = &self.hovered_id {
            if let Some(idx) = self.widget_index(old_id, widgets) {
                if let Some(rect) = self.rects.get(idx) {
                    widgets[idx].on_input(&WidgetInput::PointerLeave, ctx, *rect);
                }
            }
        }
        self.hovered_id = None;
    }

    /// Route a click event.
    ///
    /// Returns `true` if a widget consumed the event.
    pub fn route_click(
        &mut self,
        pos: Vec2,
        widgets: &mut [Box<dyn Widget>],
        ctx: &WidgetContext,
    ) -> bool {
        if let Some(hit_id) = self.hit_test(pos) {
            if let Some(idx) = self.widget_index(&hit_id, widgets) {
                if let Some(rect) = self.rects.get(idx) {
                    if widgets[idx].on_input(&WidgetInput::Click(pos), ctx, *rect) {
                        self.focused_id = Some(hit_id);
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Route a right click event.
    ///
    /// Returns `true` if a widget consumed the event.
    pub fn route_right_click(
        &mut self,
        pos: Vec2,
        widgets: &mut [Box<dyn Widget>],
        ctx: &WidgetContext,
    ) -> bool {
        if let Some(hit_id) = self.hit_test(pos) {
            if let Some(idx) = self.widget_index(&hit_id, widgets) {
                if let Some(rect) = self.rects.get(idx) {
                    return widgets[idx].on_input(&WidgetInput::RightClick(pos), ctx, *rect);
                }
            }
        }
        false
    }

    /// Route a scroll event.
    ///
    /// Returns `true` if a widget consumed the event.
    pub fn route_scroll(
        &mut self,
        delta: Scalar,
        widgets: &mut [Box<dyn Widget>],
        ctx: &WidgetContext,
    ) -> bool {
        if let Some(hovered_id) = &self.hovered_id {
            if let Some(idx) = self.widget_index(hovered_id, widgets) {
                if let Some(rect) = self.rects.get(idx) {
                    if widgets[idx].on_input(&WidgetInput::Scroll(delta), ctx, *rect) {
                        return true;
                    }
                }
            }
        }
        // If no widget consumed the scroll, scroll the island content
        self.scroll_offset = (self.scroll_offset - delta * 10.0).clamp(0.0, self.max_scroll_offset);
        true
    }

    /// Route a key press event to the focused widget.
    ///
    /// Returns `true` if the focused widget consumed the event.
    pub fn route_key_press(
        &mut self,
        key: u32,
        widgets: &mut [Box<dyn Widget>],
        ctx: &WidgetContext,
    ) -> bool {
        if let Some(focused_id) = &self.focused_id {
            if let Some(idx) = self.widget_index(focused_id, widgets) {
                if let Some(rect) = self.rects.get(idx) {
                    return widgets[idx].on_input(&WidgetInput::KeyPress(key), ctx, *rect);
                }
            }
        }
        false
    }

    /// Route a key release event to the focused widget.
    ///
    /// Returns `true` if the focused widget consumed the event.
    pub fn route_key_release(
        &mut self,
        key: u32,
        widgets: &mut [Box<dyn Widget>],
        ctx: &WidgetContext,
    ) -> bool {
        if let Some(focused_id) = &self.focused_id {
            if let Some(idx) = self.widget_index(focused_id, widgets) {
                if let Some(rect) = self.rects.get(idx) {
                    return widgets[idx].on_input(&WidgetInput::KeyRelease(key), ctx, *rect);
                }
            }
        }
        false
    }

    /// Clear the focus.
    pub fn clear_focus(&mut self) {
        self.focused_id = None;
    }

    /// Set the focus to a specific widget.
    pub fn set_focus(&mut self, id: &str) {
        self.focused_id = Some(id.to_string());
    }

    /// Hit test: find the widget under the given position.
    fn hit_test(&self, pos: Vec2) -> Option<String> {
        for (i, rect) in self.rects.iter().enumerate() {
            if rect.contains(pos) {
                return Some(format!("widget_{i}"));
            }
        }
        None
    }

    /// Find the index of a widget by id.
    fn widget_index(&self, id: &str, widgets: &[Box<dyn Widget>]) -> Option<usize> {
        widgets.iter().position(|w| w.id() == id)
    }
}

impl Default for InputRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::{DrawList, Quad, Widget, WidgetContext, WidgetInput, WidgetRegistry};
    use crate::clock::ClockWidget;
    use lumina_core::geometry::{Rect, Size, Vec2};
    use lumina_core::state::IslandState;

    #[test]
    fn a_new_router_has_no_hover_or_focus() {
        let router = InputRouter::new();
        assert!(router.hovered_id().is_none());
        assert!(router.focused_id().is_none());
        assert!(!router.pointer_inside());
    }

    #[test]
    fn update_rects_stores_rectangles() {
        let mut router = InputRouter::new();
        router.update_rects(vec![Rect::new(0.0, 0.0, 100.0, 40.0)]);
        assert_eq!(router.rects.len(), 1);
    }

    #[test]
    fn route_pointer_move_updates_hover_state() {
        let mut router = InputRouter::new();
        router.update_rects(vec![Rect::new(0.0, 0.0, 100.0, 40.0)]);
        let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClockWidget::new())];
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        let hovered = router.route_pointer_move(Vec2::new(50.0, 20.0), &mut widgets, &ctx);
        assert!(router.pointer_inside());
        assert!(hovered.is_some());
    }

    #[test]
    fn route_pointer_leave_clears_hover() {
        let mut router = InputRouter::new();
        router.update_rects(vec![Rect::new(0.0, 0.0, 100.0, 40.0)]);
        let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClockWidget::new())];
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        router.route_pointer_move(Vec2::new(50.0, 20.0), &mut widgets, &ctx);
        router.route_pointer_leave(&mut widgets, &ctx);
        assert!(!router.pointer_inside());
        assert!(router.hovered_id().is_none());
    }

    #[test]
    fn route_click_sets_focus() {
        let mut router = InputRouter::new();
        router.update_rects(vec![Rect::new(0.0, 0.0, 100.0, 40.0)]);
        let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClockWidget::new())];
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        let consumed = router.route_click(Vec2::new(50.0, 20.0), &mut widgets, &ctx);
        assert!(consumed);
        assert!(router.focused_id().is_some());
    }

    #[test]
    fn route_scroll_scrolls_island_when_no_widget_consumes() {
        let mut router = InputRouter::new();
        router.update_rects(vec![Rect::new(0.0, 0.0, 100.0, 40.0)]);
        router.set_max_scroll_offset(100.0);
        let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClockWidget::new())];
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        router.route_pointer_move(Vec2::new(50.0, 20.0), &mut widgets, &ctx);
        let consumed = router.route_scroll(1.0, &mut widgets, &ctx);
        assert!(consumed);
        assert!(router.scroll_offset() > 0.0);
    }

    #[test]
    fn route_key_press_goes_to_focused_widget() {
        let mut router = InputRouter::new();
        router.update_rects(vec![Rect::new(0.0, 0.0, 100.0, 40.0)]);
        let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClockWidget::new())];
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        router.set_focus("clock");
        let consumed = router.route_key_press(65, &mut widgets, &ctx);
        assert!(consumed);
    }

    #[test]
    fn clear_focus_removes_focus() {
        let mut router = InputRouter::new();
        router.set_focus("clock");
        assert!(router.focused_id().is_some());
        router.clear_focus();
        assert!(router.focused_id().is_none());
    }

    #[test]
    fn hit_test_finds_widget_under_position() {
        let mut router = InputRouter::new();
        router.update_rects(vec![
            Rect::new(0.0, 0.0, 100.0, 40.0),
            Rect::new(100.0, 0.0, 100.0, 40.0),
        ]);
        let hit = router.hit_test(Vec2::new(50.0, 20.0));
        assert!(hit.is_some());
        let hit = router.hit_test(Vec2::new(150.0, 20.0));
        assert!(hit.is_some());
        let hit = router.hit_test(Vec2::new(250.0, 20.0));
        assert!(hit.is_none());
    }

    #[test]
    fn scroll_offset_is_clamped_to_max() {
        let mut router = InputRouter::new();
        router.set_max_scroll_offset(50.0);
        router.scroll_offset = 100.0;
        assert_eq!(router.scroll_offset(), 50.0);
    }
}
