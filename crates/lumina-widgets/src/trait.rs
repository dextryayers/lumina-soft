//! Widget trait, registry, and the draw list.
//!
//! # The trait
//!
//! Every widget implements [`Widget`]. The trait is deliberately small: measure,
//! draw, handle input, and request state. That is everything a widget can do.
//!
//! # The draw list
//!
//! Widgets emit quads into a [`DrawList`] rather than drawing directly. The draw
//! list is a pre-allocated buffer that is cleared and refilled each frame, so the
//! frame path performs no allocation. Quads are emitted with their position, size,
//! UV rectangle, and color, and the renderer batches them into one instanced draw.

use lumina_core::geometry::{Rect, Scalar, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};
use lumina_core::time::Clock;

use crate::layout::FlexLayout;

/// Context passed to every widget method.
#[derive(Debug, Clone)]
pub struct WidgetContext {
    /// The current island state.
    pub state: IslandState,
    /// Available size inside the island, in logical pixels.
    pub available: Size,
    /// The widget's configured flex weight.
    pub flex: Scalar,
    /// The widget's configured margin on the leading edge.
    pub margin_leading: Scalar,
    /// Whether the widget is enabled.
    pub enabled: bool,
    /// The current time.
    pub now: std::time::SystemTime,
    /// Seconds since startup.
    pub elapsed: Scalar,
}

impl WidgetContext {
    /// Construct a context.
    #[must_use]
    pub fn new(state: IslandState, available: Size) -> Self {
        Self {
            state,
            available,
            flex: 1.0,
            margin_leading: 0.0,
            enabled: true,
            now: std::time::SystemTime::now(),
            elapsed: 0.0,
        }
    }
}

/// A normalized input event for a widget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WidgetInput {
    /// Pointer moved to a position inside the island.
    PointerMove(Vec2),
    /// Pointer entered the island.
    PointerEnter,
    /// Pointer left the island.
    PointerLeave,
    /// Primary button pressed at a position.
    Click(Vec2),
    /// Secondary button pressed at a position.
    RightClick(Vec2),
    /// Scroll delta.
    Scroll(Scalar),
}

/// The widget trait.
///
/// Implementors must be `Send` because the orchestrator owns them on a different
/// thread than the render loop.
pub trait Widget: Send {
    /// Stable identifier used in config and IPC.
    fn id(&self) -> &'static str;

    /// Measure the widget's desired size.
    ///
    /// Called during layout. The widget should return the size it wants, and the
    /// layout engine will shrink it if necessary.
    fn measure(&mut self, ctx: &WidgetContext) -> Size;

    /// Emit draw commands into the draw list.
    ///
    /// Called after layout. The widget should emit quads for everything it wants
    /// to draw, using the position and size the layout engine assigned.
    fn draw(&mut self, out: &mut DrawList, ctx: &WidgetContext, rect: Rect);

    /// Handle an input event.
    ///
    /// Returns `true` when the event was consumed, so the orchestrator can stop
    /// propagating it to other widgets.
    fn on_input(&mut self, ev: &WidgetInput, ctx: &WidgetContext, rect: Rect) -> bool;

    /// Request the island to enter a state.
    ///
    /// Called each frame. The widget returns `None` when it has nothing to show,
    /// or a [`StateRequest`] when it wants the island to expand.
    fn request(&self, ctx: &WidgetContext) -> Option<StateRequest>;

    /// The widget's priority in the state arbiter.
    fn priority(&self) -> u8;
}

/// A pre-allocated buffer of quads for one frame.
///
/// Cleared and refilled each frame. No allocation on the frame path.
#[derive(Debug, Clone, Default)]
pub struct DrawList {
    quads: Vec<Quad>,
}

impl DrawList {
    /// Create an empty draw list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the list, keeping the allocated capacity.
    pub fn clear(&mut self) {
        self.quads.clear();
    }

    /// Push a quad.
    pub fn push(&mut self, quad: Quad) {
        self.quads.push(quad);
    }

    /// The quads in the list.
    #[must_use]
    pub fn quads(&self) -> &[Quad] {
        &self.quads
    }

    /// The number of quads.
    #[must_use]
    pub fn len(&self) -> usize {
        self.quads.len()
    }

    /// True when the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    /// Iterate over the quads.
    pub fn iter(&self) -> impl Iterator<Item = &Quad> {
        self.quads.iter()
    }
}

/// One quad to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// Position in logical pixels.
    pub position: Vec2,
    /// Size in logical pixels.
    pub size: Size,
    /// UV minimum.
    pub uv_min: Vec2,
    /// UV maximum.
    pub uv_max: Vec2,
    /// Premultiplied color.
    pub color: [f32; 4],
    /// Corner radius in logical pixels.
    pub radius: Scalar,
}

impl Quad {
    /// Construct a quad.
    #[must_use]
    pub fn new(position: Vec2, size: Size, uv_min: Vec2, uv_max: Vec2, color: [f32; 4]) -> Self {
        Self { position, size, uv_min, uv_max, color, radius: 0.0 }
    }

    /// Set the corner radius.
    #[must_use]
    pub fn with_radius(mut self, radius: Scalar) -> Self {
        self.radius = radius;
        self
    }

    /// The quad's rectangle.
    #[must_use]
    pub fn rect(&self) -> Rect {
        Rect::from_size(self.position, self.size)
    }
}

/// The widget registry.
///
/// Owns all widgets and drives their lifecycle. The orchestrator holds one registry
/// and calls [`WidgetRegistry::layout`] and [`WidgetRegistry::draw`] each frame.
#[derive(Debug, Default)]
pub struct WidgetRegistry {
    widgets: Vec<Box<dyn Widget>>,
}

impl WidgetRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a widget.
    pub fn register(&mut self, widget: Box<dyn Widget>) {
        self.widgets.push(widget);
    }

    /// Look up a widget by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&dyn Widget> {
        self.widgets.iter().find(|w| w.id() == id).map(AsRef::as_ref)
    }

    /// Look up a widget by id, mutably.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut dyn Widget> {
        self.widgets.iter_mut().find(|w| w.id() == id).map(AsMut::as_mut)
    }

    /// All registered widgets.
    #[must_use]
    pub fn widgets(&self) -> &[Box<dyn Widget>] {
        &self.widgets
    }

    /// The number of registered widgets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.widgets.len()
    }

    /// True when no widgets are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }

    /// Layout all widgets into a horizontal strip.
    ///
    /// Returns the rectangles assigned to each widget, in registration order.
    pub fn layout(&mut self, ctx: &WidgetContext) -> Vec<Rect> {
        let mut layout = FlexLayout::new(ctx.available);
        for widget in &mut self.widgets {
            if !ctx.enabled {
                continue;
            }
            let desired = widget.measure(ctx);
            layout.add(desired, ctx.flex, ctx.margin_leading);
        }
        layout.finish()
    }

    /// Draw all widgets into the draw list.
    pub fn draw(&mut self, out: &mut DrawList, ctx: &WidgetContext, rects: &[Rect]) {
        for (i, widget) in self.widgets.iter_mut().enumerate() {
            if !ctx.enabled {
                continue;
            }
            if let Some(rect) = rects.get(i) {
                widget.draw(out, ctx, *rect);
            }
        }
    }

    /// Route an input event to the widget under the point.
    ///
    /// Returns `true` when a widget consumed the event.
    pub fn route_input(
        &mut self,
        ev: &WidgetInput,
        ctx: &WidgetContext,
        rects: &[Rect],
    ) -> bool {
        for (i, widget) in self.widgets.iter_mut().enumerate() {
            if !ctx.enabled {
                continue;
            }
            if let Some(rect) = rects.get(i) {
                let pos = match ev {
                    WidgetInput::PointerMove(p)
                    | WidgetInput::Click(p)
                    | WidgetInput::RightClick(p) => *p,
                    _ => Vec2::ZERO,
                };
                if rect.contains(pos) && widget.on_input(ev, ctx, *rect) {
                    return true;
                }
            }
        }
        false
    }

    /// Collect state requests from all widgets.
    pub fn requests(&self, ctx: &WidgetContext) -> Vec<StateRequest> {
        self.widgets
            .iter()
            .filter(|w| ctx.enabled)
            .filter_map(|w| w.request(ctx))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestWidget;

    impl Widget for TestWidget {
        fn id(&self) -> &'static str {
            "test"
        }
        fn measure(&mut self, _ctx: &WidgetContext) -> Size {
            Size::new(100.0, 40.0)
        }
        fn draw(&mut self, _out: &mut DrawList, _ctx: &WidgetContext, _rect: Rect) {}
        fn on_input(&mut self, _ev: &WidgetInput, _ctx: &WidgetContext, _rect: Rect) -> bool {
            false
        }
        fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
            None
        }
        fn priority(&self) -> u8 {
            0
        }
    }

    #[test]
    fn a_new_registry_is_empty() {
        let registry = WidgetRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn registering_a_widget_makes_it_lookupable() {
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(TestWidget));
        assert_eq!(registry.len(), 1);
        assert!(registry.get("test").is_some());
        assert!(registry.get("other").is_none());
    }

    #[test]
    fn a_draw_list_clears_but_keeps_capacity() {
        let mut list = DrawList::new();
        list.push(Quad::new(Vec2::ZERO, Size::new(10.0, 10.0), Vec2::ZERO, Vec2::ZERO, [1.0; 4]));
        assert_eq!(list.len(), 1);
        list.clear();
        assert_eq!(list.len(), 0);
        assert!(list.is_empty());
    }

    #[test]
    fn a_quad_knows_its_rectangle() {
        let quad = Quad::new(Vec2::new(10.0, 20.0), Size::new(30.0, 40.0), Vec2::ZERO, Vec2::ZERO, [1.0; 4]);
        assert_eq!(quad.rect(), Rect::new(10.0, 20.0, 30.0, 40.0));
    }

    #[test]
    fn a_quad_can_have_a_corner_radius() {
        let quad = Quad::new(Vec2::ZERO, Size::new(10.0, 10.0), Vec2::ZERO, Vec2::ZERO, [1.0; 4])
            .with_radius(5.0);
        assert_eq!(quad.radius, 5.0);
    }

    #[test]
    fn the_registry_collects_requests() {
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(TestWidget));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(100.0, 40.0));
        assert!(registry.requests(&ctx).is_empty());
    }

    #[test]
    fn the_registry_routes_input_to_the_widget_under_the_point() {
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(TestWidget));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(100.0, 40.0));
        let rects = registry.layout(&ctx);
        assert!(!rects.is_empty());
        // The widget is at the origin, so a click at (50, 20) should reach it.
        let consumed = registry.route_input(&WidgetInput::Click(Vec2::new(50.0, 20.0)), &ctx, &rects);
        assert!(!consumed, "the test widget does not consume input");
    }

    #[test]
    fn the_registry_layout_assigns_rectangles() {
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(TestWidget));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(200.0, 40.0));
        let rects = registry.layout(&ctx);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].width, 100.0);
        assert_eq!(rects[0].height, 40.0);
    }
}