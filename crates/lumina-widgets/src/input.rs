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
    /// The widget ids corresponding to each rect.
    widget_ids: Vec<String>,
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
            widget_ids: Vec::new(),
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
        self.widget_ids = (0..self.rects.len())
            .map(|i| format!("widget_{i}"))
            .collect();
    }

    /// Update the layout rectangles with widget ids.
    pub fn update_rects_with_ids(&mut self, rects: Vec<Rect>, ids: Vec<String>) {
        self.rects = rects;
        self.widget_ids = ids;
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

    /// Set the scroll offset directly.
    pub fn set_scroll_offset(&mut self, offset: Scalar) {
        self.scroll_offset = offset.clamp(0.0, self.max_scroll_offset);
    }

    /// Route a pointer move event.
    ///
    /// Returns the index of the widget that is now hovered, if any.
    pub fn route_pointer_move(
        &mut self,
        pos: Vec2,
        widgets: &mut [Box<dyn Widget>],
        ctx: &WidgetContext,
    ) -> Option<usize> {
        self.last_pointer_pos = pos;
        self.pointer_inside = true;

        let new_hovered = self.hit_test(pos);
        let old_hovered = self
            .hovered_id
            .as_ref()
            .and_then(|id| Self::widget_index(id, widgets));
        if new_hovered != old_hovered {
            // Leave the old widget
            if let Some(idx) = old_hovered {
                if let Some(rect) = self.rects.get(idx) {
                    widgets[idx].on_input(&WidgetInput::PointerLeave, ctx, *rect);
                }
            }
            // Enter the new widget
            if let Some(new_idx) = new_hovered {
                if let Some(rect) = self.rects.get(new_idx) {
                    widgets[new_idx].on_input(&WidgetInput::PointerEnter, ctx, *rect);
                }
            }
            // Store the widget's own id, not a positional placeholder. The
            // placeholder does not match `Widget::id`, so leave events and
            // focused input would be dropped on every subsequent frame.
            self.hovered_id = new_hovered.map(|i| widgets[i].id().to_string());
        }

        // Send the move event to the hovered widget
        if let Some(hovered_idx) = new_hovered {
            if let Some(rect) = self.rects.get(hovered_idx) {
                widgets[hovered_idx].on_input(&WidgetInput::PointerMove(pos), ctx, *rect);
            }
        }

        new_hovered
    }

    /// Route a pointer leave event.
    pub fn route_pointer_leave(&mut self, widgets: &mut [Box<dyn Widget>], ctx: &WidgetContext) {
        self.pointer_inside = false;
        if let Some(old_id) = &self.hovered_id {
            if let Some(idx) = Self::widget_index(old_id, widgets) {
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
        if let Some(hit_idx) = self.hit_test(pos) {
            if let Some(rect) = self.rects.get(hit_idx) {
                if widgets[hit_idx].on_input(&WidgetInput::Click(pos), ctx, *rect) {
                    self.focused_id = Some(widgets[hit_idx].id().to_string());
                    return true;
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
        if let Some(hit_idx) = self.hit_test(pos) {
            if let Some(rect) = self.rects.get(hit_idx) {
                return widgets[hit_idx].on_input(&WidgetInput::RightClick(pos), ctx, *rect);
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
            if let Some(idx) = Self::widget_index(hovered_id, widgets) {
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
            if let Some(idx) = Self::widget_index(focused_id, widgets) {
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
            if let Some(idx) = Self::widget_index(focused_id, widgets) {
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

    /// Move focus to the next widget in registration order, wrapping at the end.
    ///
    /// The caller maps Tab and Shift+Tab to this and [`Self::focus_previous`].
    /// Returns the newly focused id, or `None` when there are no widgets.
    pub fn focus_next(&mut self, widgets: &[Box<dyn Widget>]) -> Option<&str> {
        if widgets.is_empty() {
            self.focused_id = None;
            return None;
        }
        let current = self
            .focused_id
            .as_ref()
            .and_then(|id| Self::widget_index(id, widgets));
        let next = current.map_or(0, |index| (index + 1) % widgets.len());
        self.focused_id = Some(widgets[next].id().to_string());
        self.focused_id.as_deref()
    }

    /// Move focus to the previous widget in registration order, wrapping at the start.
    ///
    /// Returns the newly focused id, or `None` when there are no widgets.
    pub fn focus_previous(&mut self, widgets: &[Box<dyn Widget>]) -> Option<&str> {
        if widgets.is_empty() {
            self.focused_id = None;
            return None;
        }
        let current = self
            .focused_id
            .as_ref()
            .and_then(|id| Self::widget_index(id, widgets));
        let previous = current.map_or(widgets.len() - 1, |index| {
            (index + widgets.len() - 1) % widgets.len()
        });
        self.focused_id = Some(widgets[previous].id().to_string());
        self.focused_id.as_deref()
    }

    /// Activate the focused widget, as Enter or Space would.
    ///
    /// The router synthesizes a primary click at the widget's centre, so
    /// pointer-only widgets become keyboard-operable without every widget
    /// reimplementing activation. Returns `true` when the widget consumed it.
    pub fn activate_focused(
        &mut self,
        widgets: &mut [Box<dyn Widget>],
        ctx: &WidgetContext,
    ) -> bool {
        let Some(index) = self
            .focused_id
            .as_ref()
            .and_then(|id| Self::widget_index(id, widgets))
        else {
            return false;
        };
        let Some(rect) = self.rects.get(index) else {
            return false;
        };
        widgets[index].on_input(&WidgetInput::Click(rect.center()), ctx, *rect)
    }

    /// Drop focus in response to Escape.
    ///
    /// Returns `true` when something was focused, so the caller can distinguish
    /// "Escape collapsed the island" from "Escape did nothing".
    pub fn dismiss_focus(&mut self) -> bool {
        self.focused_id.take().is_some()
    }

    /// Clear focus when the focused widget is no longer registered.
    ///
    /// Layout rebuilds can remove a widget while it holds focus. Calling this
    /// after updating rects and widgets keeps key events from being routed to a
    /// stale id forever.
    pub fn retain_focus(&mut self, widgets: &[Box<dyn Widget>]) {
        if self
            .focused_id
            .as_ref()
            .is_some_and(|id| Self::widget_index(id, widgets).is_none())
        {
            self.focused_id = None;
        }
    }

    /// Hit test: find the widget index under the given position.
    fn hit_test(&self, pos: Vec2) -> Option<usize> {
        for (i, rect) in self.rects.iter().enumerate() {
            if rect.contains(pos) {
                return Some(i);
            }
        }
        None
    }

    /// Find the index of a widget by id.
    fn widget_index(id: &str, widgets: &[Box<dyn Widget>]) -> Option<usize> {
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
    use crate::clock::ClockWidget;
    use crate::widget::{DrawList, Widget, WidgetContext, WidgetInput};
    use lumina_core::geometry::{Rect, Size, Vec2};
    use lumina_core::state::IslandState;

    struct ClickableWidget;

    impl Widget for ClickableWidget {
        fn id(&self) -> &'static str {
            "clickable"
        }
        fn measure(&mut self, _ctx: &WidgetContext) -> Size {
            Size::new(100.0, 40.0)
        }
        fn draw(&mut self, _out: &mut DrawList, _ctx: &WidgetContext, _rect: Rect) {}
        fn on_input(&mut self, _ev: &WidgetInput, _ctx: &WidgetContext, _rect: Rect) -> bool {
            true
        }
        fn request(&self, _ctx: &WidgetContext) -> Option<lumina_core::state::StateRequest> {
            None
        }
        fn priority(&self) -> u8 {
            0
        }
    }

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
        assert_eq!(hovered, Some(0));
        assert_eq!(
            router.hovered_id(),
            Some("clock"),
            "hover must store the widget's real id, not a positional placeholder"
        );
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
        let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClickableWidget)];
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        let consumed = router.route_click(Vec2::new(50.0, 20.0), &mut widgets, &ctx);
        assert!(consumed);
        assert_eq!(router.focused_id(), Some("clickable"));
        assert!(router.route_key_press(65, &mut widgets, &ctx));
    }

    #[test]
    fn tab_traversal_cycles_forward_and_backward() {
        let mut router = InputRouter::new();
        let widgets: Vec<Box<dyn Widget>> =
            vec![Box::new(ClickableWidget), Box::new(ClockWidget::new())];

        assert_eq!(router.focus_next(&widgets), Some("clickable"));
        assert_eq!(router.focus_next(&widgets), Some("clock"));
        assert_eq!(
            router.focus_next(&widgets),
            Some("clickable"),
            "focus wraps at the end"
        );
        assert_eq!(
            router.focus_previous(&widgets),
            Some("clock"),
            "reverse traversal wraps at the start"
        );
        assert_eq!(router.focus_next(&[]), None);
        assert!(router.focused_id().is_none());
    }

    #[test]
    fn enter_can_activate_the_focused_widget_without_a_pointer() {
        let mut router = InputRouter::new();
        router.update_rects(vec![Rect::new(0.0, 0.0, 100.0, 40.0)]);
        let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClickableWidget)];
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        router.set_focus("clickable");
        assert!(router.activate_focused(&mut widgets, &ctx));
        assert!(router.dismiss_focus());
        assert!(router.focused_id().is_none());
        assert!(!router.dismiss_focus());
    }

    #[test]
    fn focus_is_dropped_when_its_widget_is_removed() {
        let mut router = InputRouter::new();
        let widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClickableWidget)];
        router.set_focus("clickable");
        router.retain_focus(&widgets);
        assert_eq!(router.focused_id(), Some("clickable"));

        router.retain_focus(&[]);
        assert!(router.focused_id().is_none());
    }

    #[test]
    fn route_scroll_scrolls_island_when_no_widget_consumes() {
        let mut router = InputRouter::new();
        router.update_rects(vec![Rect::new(0.0, 0.0, 100.0, 40.0)]);
        router.set_max_scroll_offset(100.0);
        let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClockWidget::new())];
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        router.route_pointer_move(Vec2::new(50.0, 20.0), &mut widgets, &ctx);
        assert!(router.hovered_id().is_some());
        let consumed = router.route_scroll(1.0, &mut widgets, &ctx);
        assert!(consumed);
        assert!(router.scroll_offset() >= 0.0);
    }

    #[test]
    fn route_key_press_goes_to_focused_widget() {
        let mut router = InputRouter::new();
        router.update_rects(vec![Rect::new(0.0, 0.0, 100.0, 40.0)]);
        let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(ClickableWidget)];
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        router.set_focus("clickable");
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
        router.set_scroll_offset(100.0);
        assert_eq!(router.scroll_offset(), 50.0);
    }
}
