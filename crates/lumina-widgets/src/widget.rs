//! Widget trait, registry, and the draw list.
//!
//! # The trait
//!
//! Every widget implements [`Widget`]. The trait is deliberately small: measure,
//! draw, handle input, request state, and react to lifecycle events. That is
//! everything a widget can do.
//!
//! # Lifecycle hooks
//!
//! The orchestrator calls [`Widget::on_show`] when the widget becomes visible
//! and [`Widget::on_hide`] when it is hidden. These hooks let widgets acquire
//! and release resources without polling.
//!
//! [`Widget::on_state_enter`] and [`Widget::on_state_leave`] fire when the
//! island enters or leaves a state the widget cares about, so a widget can
//! prepare content before it appears and clean up after it disappears.
//!
//! # The draw list
//!
//! Widgets emit quads into a [`DrawList`] rather than drawing directly. The draw
//! list is a pre-allocated buffer that is cleared and refilled each frame, so the
//! frame path performs no allocation. Quads are emitted with their position, size,
//! UV rectangle, and color, and the renderer batches them into one instanced draw.

use lumina_config::schema::{LayoutAlign, LayoutConfig, WidgetConfig, WidgetEntry};
use lumina_core::geometry::{Rect, Scalar, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};

use crate::layout::{FlexLayout, VerticalAlign};

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
    /// The widget's configured id from the layout order.
    ///
    /// This is a static string borrowed from the widget itself rather than a
    /// fresh allocation, because the context is rebuilt for every widget every
    /// frame.
    pub config_id: &'static str,
    /// The widget's configured flex weight from config.
    pub config_flex: Scalar,
    /// The widget's configured margin from config.
    pub config_margin: Scalar,
    /// Whether the widget is currently hovered.
    pub hovered: bool,
    /// Whether the widget has keyboard focus.
    pub focused: bool,
    /// The widget's minimum width from config.
    pub min_width: Scalar,
    /// The widget's maximum width from config.
    pub max_width: Scalar,
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
            config_id: "",
            config_flex: 1.0,
            config_margin: 0.0,
            hovered: false,
            focused: false,
            min_width: 1.0,
            max_width: available.width,
        }
    }

    /// Construct a context with config-driven values.
    #[must_use]
    pub fn with_config(
        state: IslandState,
        available: Size,
        config_id: &'static str,
        config_flex: Scalar,
        config_margin: Scalar,
    ) -> Self {
        Self {
            state,
            available,
            flex: config_flex,
            margin_leading: config_margin,
            enabled: true,
            now: std::time::SystemTime::now(),
            elapsed: 0.0,
            config_id,
            config_flex,
            config_margin,
            hovered: false,
            focused: false,
            min_width: 1.0,
            max_width: available.width,
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
    /// Key pressed.
    KeyPress(u32),
    /// Key released.
    KeyRelease(u32),
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

    /// Called when the widget becomes visible.
    ///
    /// The widget should acquire resources it needs to draw. The default
    /// implementation does nothing.
    fn on_show(&mut self, _ctx: &WidgetContext) {}

    /// Called when the widget is hidden.
    ///
    /// The widget should release resources acquired in [`Widget::on_show`]. The
    /// default implementation does nothing.
    fn on_hide(&mut self, _ctx: &WidgetContext) {}

    /// Called when the island enters a state this widget cares about.
    ///
    /// The widget should prepare content before it appears. The default
    /// implementation does nothing.
    fn on_state_enter(&mut self, _state: IslandState, _ctx: &WidgetContext) {}

    /// Called when the island leaves a state this widget cares about.
    ///
    /// The widget should clean up after itself. The default implementation does
    /// nothing.
    fn on_state_leave(&mut self, _state: IslandState, _ctx: &WidgetContext) {}

    /// Whether this widget wants to handle the given state.
    ///
    /// The default implementation returns `true` for all states, meaning the
    /// widget is interested in every state transition.
    fn cares_about(&self, _state: IslandState) -> bool {
        true
    }
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

    /// Create a draw list with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            quads: Vec::with_capacity(capacity),
        }
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

    /// Reserve capacity for at least `additional` more quads.
    pub fn reserve(&mut self, additional: usize) {
        self.quads.reserve(additional);
    }

    /// The total capacity of the underlying buffer.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.quads.capacity()
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
    /// Z-order for layering. Higher values draw on top.
    pub z_order: u16,
}

impl Quad {
    /// Construct a quad.
    #[must_use]
    pub fn new(position: Vec2, size: Size, uv_min: Vec2, uv_max: Vec2, color: [f32; 4]) -> Self {
        Self { position, size, uv_min, uv_max, color, radius: 0.0, z_order: 0 }
    }

    /// Set the corner radius.
    #[must_use]
    pub fn with_radius(mut self, radius: Scalar) -> Self {
        self.radius = radius;
        self
    }

    /// Set the z-order.
    #[must_use]
    pub fn with_z_order(mut self, z_order: u16) -> Self {
        self.z_order = z_order;
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
///
/// The registry supports config-driven widget ordering, per-widget enablement,
/// flex weights, leading margins, spacing, alignment, and a `max_widgets` limit.
/// Disabled widgets and widgets past the limit are skipped by layout, drawing,
/// input routing, and state requests alike, so every consumer sees the same
/// visible set.
#[derive(Default)]
pub struct WidgetRegistry {
    widgets: Vec<Box<dyn Widget>>,
    entries: Vec<WidgetEntry>,
    config_order: Vec<String>,
    max_widgets: usize,
    spacing: Scalar,
    align: VerticalAlign,
    visible_count: usize,
    last_state: Option<IslandState>,
    hovered_id: Option<&'static str>,
}

impl std::fmt::Debug for WidgetRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WidgetRegistry")
            .field("widget_count", &self.widgets.len())
            .field("config_order", &self.config_order)
            .field("max_widgets", &self.max_widgets)
            .field("spacing", &self.spacing)
            .field("align", &self.align)
            .field("visible_count", &self.visible_count)
            .field("last_state", &self.last_state)
            .field("hovered_id", &self.hovered_id)
            .finish()
    }
}

/// Build the context a single widget sees.
///
/// Per-widget values from the registry override the container context, and the
/// hover flag is derived from the registry's tracked pointer target rather than
/// from a global flag that would mark every widget as hovered.
fn widget_context(
    ctx: &WidgetContext,
    id: &'static str,
    entry: &WidgetEntry,
    available: Size,
    hovered: bool,
) -> WidgetContext {
    let flex = entry.flex.unwrap_or(ctx.flex);
    let margin = entry.margin_leading.unwrap_or(ctx.margin_leading);
    WidgetContext {
        flex,
        margin_leading: margin,
        enabled: entry.enabled.unwrap_or(true),
        config_id: id,
        config_flex: flex,
        config_margin: margin,
        hovered,
        min_width: 1.0,
        max_width: available.width,
        ..ctx.clone()
    }
}

impl WidgetRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_widgets: usize::MAX,
            ..Self::default()
        }
    }

    /// Create a registry with config-driven ordering.
    #[must_use]
    pub fn with_config(order: Vec<String>, max_widgets: usize) -> Self {
        Self {
            widgets: Vec::new(),
            entries: Vec::new(),
            config_order: order,
            max_widgets: max_widgets.max(1),
            spacing: 0.0,
            align: VerticalAlign::Center,
            visible_count: 0,
            last_state: None,
            hovered_id: None,
        }
    }

    /// Create a registry from the full widget and layout configuration.
    #[must_use]
    pub fn from_layout(layout: &LayoutConfig, widgets: &WidgetConfig) -> Self {
        let mut registry = Self::with_config(layout.order.clone(), layout.max_widgets as usize);
        registry.spacing = layout.spacing.max(0.0);
        registry.align = match layout.align {
            LayoutAlign::Top => VerticalAlign::Top,
            LayoutAlign::Center => VerticalAlign::Center,
            LayoutAlign::Bottom => VerticalAlign::Bottom,
        };
        registry.apply_widget_config(widgets);
        registry
    }

    /// Register a widget.
    pub fn register(&mut self, widget: Box<dyn Widget>) {
        self.register_with(widget, WidgetEntry::default());
    }

    /// Register a widget with per-widget configuration overrides.
    pub fn register_with(&mut self, widget: Box<dyn Widget>, entry: WidgetEntry) {
        self.widgets.push(widget);
        self.entries.push(entry);
        self.sort_widgets();
    }

    /// Apply per-widget config to every already-registered widget with a matching id.
    pub fn set_entry(&mut self, id: &str, entry: &WidgetEntry) {
        for (widget, current) in self.widgets.iter().zip(self.entries.iter_mut()) {
            if widget.id() == id {
                *current = entry.clone();
            }
        }
    }

    /// Apply a whole `[widgets]` config table to the registered widgets.
    pub fn apply_widget_config(&mut self, config: &WidgetConfig) {
        for (widget, entry) in self.widgets.iter().zip(self.entries.iter_mut()) {
            if let Some(configured) = config.widgets.get(widget.id()) {
                *entry = configured.clone();
            }
        }
    }

    /// Sort widgets and their entries by the config order.
    fn sort_widgets(&mut self) {
        if self.config_order.is_empty() {
            return;
        }
        let order = &self.config_order;
        let mut paired: Vec<_> = self
            .widgets
            .drain(..)
            .zip(self.entries.drain(..))
            .collect();
        paired.sort_by_key(|(widget, _)| {
            order
                .iter()
                .position(|id| id == widget.id())
                .unwrap_or(usize::MAX)
        });
        let (widgets, entries): (Vec<_>, Vec<_>) = paired.into_iter().unzip();
        self.widgets = widgets;
        self.entries = entries;
    }

    /// Look up a widget by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&dyn Widget> {
        self.widgets.iter().find(|w| w.id() == id).map(AsRef::as_ref)
    }

    /// Look up a widget by id, mutably.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut (dyn Widget + 'static)> {
        self.widgets.iter_mut().find(|w| w.id() == id).map(|b| &mut **b)
    }

    /// All registered widgets.
    #[must_use]
    pub fn widgets(&self) -> &[Box<dyn Widget>] {
        &self.widgets
    }

    /// The per-widget configuration entries, aligned with [`Self::widgets`].
    #[must_use]
    pub fn entries(&self) -> &[WidgetEntry] {
        &self.entries
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

    /// The number of widgets that are currently visible.
    #[must_use]
    pub fn visible_count(&self) -> usize {
        self.visible_count
    }

    /// The widget id currently under the pointer, if any.
    #[must_use]
    pub fn hovered_id(&self) -> Option<&'static str> {
        self.hovered_id
    }

    /// Update the config-driven ordering.
    pub fn update_config(&mut self, order: Vec<String>, max_widgets: usize) {
        self.config_order = order;
        self.max_widgets = max_widgets.max(1);
        self.sort_widgets();
    }

    /// Update spacing, alignment, ordering, and the visibility limit at once.
    pub fn update_layout(&mut self, layout: &LayoutConfig) {
        self.spacing = layout.spacing.max(0.0);
        self.align = match layout.align {
            LayoutAlign::Top => VerticalAlign::Top,
            LayoutAlign::Center => VerticalAlign::Center,
            LayoutAlign::Bottom => VerticalAlign::Bottom,
        };
        self.update_config(layout.order.clone(), layout.max_widgets as usize);
    }

    /// The visible widget indices in draw order.
    ///
    /// A widget is visible when it is enabled and falls inside `max_widgets`.
    /// This is the single definition used by layout, drawing, input routing,
    /// and requests.
    fn visible_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.enabled.unwrap_or(true))
            .map(|(index, _)| index)
            .take(self.max_widgets)
    }

    /// Layout all widgets into a horizontal strip.
    ///
    /// Returns the rectangles assigned to each visible widget, in visible order.
    /// Disabled widgets take no space and are not counted against `max_widgets`.
    pub fn layout(&mut self, ctx: &WidgetContext) -> Vec<Rect> {
        if !ctx.enabled {
            self.visible_count = 0;
            return Vec::new();
        }

        let mut layout = FlexLayout::new(ctx.available);
        layout.set_spacing(self.spacing);
        layout.set_align(self.align);

        let visible: Vec<usize> = self.visible_indices().collect();
        for &index in &visible {
            let widget_ctx = {
                let widget = &self.widgets[index];
                widget_context(
                    ctx,
                    widget.id(),
                    &self.entries[index],
                    ctx.available,
                    self.hovered_id == Some(widget.id()),
                )
            };
            let desired = self.widgets[index].measure(&widget_ctx);
            let flex = widget_ctx.flex;
            let margin = widget_ctx.margin_leading;
            layout.add(desired, flex, margin);
        }

        let rects = layout.finish();
        self.visible_count = rects.len();
        rects
    }

    /// Draw all visible widgets into the draw list.
    ///
    /// `rects` must be the value returned by the most recent [`Self::layout`]
    /// call for the same state, which keeps drawing and hit-testing to exactly
    /// the visible set.
    pub fn draw(&mut self, out: &mut DrawList, ctx: &WidgetContext, rects: &[Rect]) {
        if !ctx.enabled {
            return;
        }
        let visible: Vec<usize> = self.visible_indices().collect();
        for (slot, &index) in visible.iter().enumerate() {
            let Some(rect) = rects.get(slot) else {
                break;
            };
            let widget_ctx = {
                let widget = &self.widgets[index];
                widget_context(
                    ctx,
                    widget.id(),
                    &self.entries[index],
                    ctx.available,
                    self.hovered_id == Some(widget.id()),
                )
            };
            self.widgets[index].draw(out, &widget_ctx, *rect);
        }
    }

    /// Route an input event to the widget under the point.
    ///
    /// Pointer movement updates the tracked hover target before dispatch, so a
    /// widget sees `ctx.hovered == true` for the event that entered it and for
    /// every frame while the pointer stays there.
    ///
    /// Returns `true` when a widget consumed the event.
    pub fn route_input(
        &mut self,
        ev: &WidgetInput,
        ctx: &WidgetContext,
        rects: &[Rect],
    ) -> bool {
        if !ctx.enabled {
            return false;
        }

        let point = match ev {
            WidgetInput::PointerMove(point)
            | WidgetInput::Click(point)
            | WidgetInput::RightClick(point) => Some(*point),
            _ => None,
        };

        let visible: Vec<usize> = self.visible_indices().collect();
        let target = point.and_then(|point| {
            visible.iter().enumerate().find_map(|(slot, &index)| {
                rects
                    .get(slot)
                    .filter(|rect| rect.contains(point))
                    .map(|_| (slot, index))
            })
        });

        match ev {
            WidgetInput::PointerLeave => {
                self.hovered_id = None;
                return false;
            }
            WidgetInput::PointerMove(_) => {
                let target_id = target.map(|(_, index)| self.widgets[index].id());
                self.update_hover(target_id, ctx);
            }
            _ => {}
        }

        let Some((slot, index)) = target else {
            return false;
        };
        let rect = rects[slot];
        let widget_ctx = {
            let widget = &self.widgets[index];
            widget_context(
                ctx,
                widget.id(),
                &self.entries[index],
                ctx.available,
                self.hovered_id == Some(widget.id()),
            )
        };
        self.widgets[index].on_input(ev, &widget_ctx, rect)
    }

    /// Collect state requests from all visible widgets.
    pub fn requests(&self, ctx: &WidgetContext) -> Vec<StateRequest> {
        if !ctx.enabled {
            return Vec::new();
        }
        self.visible_indices()
            .filter_map(|index| {
                let widget = &self.widgets[index];
                let widget_ctx = widget_context(
                    ctx,
                    widget.id(),
                    &self.entries[index],
                    ctx.available,
                    self.hovered_id == Some(widget.id()),
                );
                widget.request(&widget_ctx)
            })
            .collect()
    }

    /// Notify all widgets of a state change.
    pub fn notify_state_change(&mut self, new_state: IslandState, ctx: &WidgetContext) {
        if let Some(old_state) = self.last_state {
            if old_state == new_state {
                return;
            }
            for widget in &mut self.widgets {
                if widget.cares_about(old_state) {
                    widget.on_state_leave(old_state, ctx);
                }
            }
        }
        for widget in &mut self.widgets {
            if widget.cares_about(new_state) {
                widget.on_state_enter(new_state, ctx);
            }
        }
        self.last_state = Some(new_state);
    }

    /// Notify all widgets that they are becoming visible.
    pub fn notify_show(&mut self, ctx: &WidgetContext) {
        for widget in &mut self.widgets {
            widget.on_show(ctx);
        }
    }

    /// Notify all widgets that they are being hidden.
    pub fn notify_hide(&mut self, ctx: &WidgetContext) {
        for widget in &mut self.widgets {
            widget.on_hide(ctx);
        }
    }

    /// Update hover state for all widgets.
    ///
    /// The previous implementation compared against the global `ctx.hovered`
    /// flag, which meant a hovered widget could never interrupt a hover. The
    /// tracked id is now authoritative: entering a widget emits the hover
    /// lifecycle event exactly once, leaving emits the matching leave event.
    pub fn update_hover(&mut self, hovered_id: Option<&str>, ctx: &WidgetContext) {
        let new_id = hovered_id.and_then(|target| {
            self.widgets
                .iter()
                .find(|widget| widget.id() == target)
                .map(|widget| widget.id())
        });
        if new_id == self.hovered_id {
            return;
        }
        let old_id = self.hovered_id;
        self.hovered_id = new_id;
        if let Some(old_id) = old_id {
            if let Some(index) = self.widgets.iter().position(|w| w.id() == old_id) {
                let mut widget_ctx = widget_context(
                    ctx,
                    old_id,
                    &self.entries[index],
                    ctx.available,
                    false,
                );
                widget_ctx.hovered = false;
                self.widgets[index].on_state_leave(IslandState::Hover, &widget_ctx);
            }
        }
        if let Some(new_id) = new_id {
            if let Some(index) = self.widgets.iter().position(|w| w.id() == new_id) {
                let mut widget_ctx = widget_context(
                    ctx,
                    new_id,
                    &self.entries[index],
                    ctx.available,
                    true,
                );
                widget_ctx.hovered = true;
                self.widgets[index].on_state_enter(IslandState::Hover, &widget_ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestWidget {
        id: &'static str,
        show_count: u32,
        hide_count: u32,
    }

    impl TestWidget {
        fn new(id: &'static str) -> Self {
            Self { id, show_count: 0, hide_count: 0 }
        }
    }

    impl Widget for TestWidget {
        fn id(&self) -> &'static str {
            self.id
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
        fn on_show(&mut self, _ctx: &WidgetContext) {
            self.show_count += 1;
        }
        fn on_hide(&mut self, _ctx: &WidgetContext) {
            self.hide_count += 1;
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
        registry.register(Box::new(TestWidget::new("test")));
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
    fn a_quad_can_have_a_z_order() {
        let quad = Quad::new(Vec2::ZERO, Size::new(10.0, 10.0), Vec2::ZERO, Vec2::ZERO, [1.0; 4])
            .with_z_order(5);
        assert_eq!(quad.z_order, 5);
    }

    #[test]
    fn the_registry_collects_requests() {
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(TestWidget::new("test")));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(100.0, 40.0));
        assert_eq!(registry.requests(&ctx), [] as [lumina_core::state::StateRequest; 0]);
    }

    #[test]
    fn the_registry_routes_input_to_the_widget_under_the_point() {
        let mut registry = WidgetRegistry::with_config(vec![], 10);
        registry.register(Box::new(TestWidget::new("test")));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(100.0, 40.0));
        let rects = registry.layout(&ctx);
        assert_ne!(rects, [] as [lumina_core::Rect; 0]);
        let consumed = registry.route_input(&WidgetInput::Click(Vec2::new(50.0, 20.0)), &ctx, &rects);
        assert!(!consumed, "the test widget does not consume input");
    }

    #[test]
    fn the_registry_layout_assigns_rectangles() {
        let mut registry = WidgetRegistry::with_config(vec![], 10);
        registry.register(Box::new(TestWidget::new("test")));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(200.0, 40.0));
        let rects = registry.layout(&ctx);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].width, 100.0);
        assert_eq!(rects[0].height, 40.0);
    }

    #[test]
    fn lifecycle_hooks_fire_on_show_and_hide() {
        let mut widget = TestWidget::new("test");
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(100.0, 40.0));
        assert_eq!(widget.show_count, 0);
        widget.on_show(&ctx);
        assert_eq!(widget.show_count, 1);
        widget.on_hide(&ctx);
        assert_eq!(widget.hide_count, 1);
    }

    #[test]
    fn config_driven_ordering_sorts_widgets() {
        let mut registry = WidgetRegistry::with_config(
            vec!["b".to_string(), "a".to_string()],
            10,
        );
        registry.register(Box::new(TestWidget::new("a")));
        registry.register(Box::new(TestWidget::new("b")));
        assert_eq!(registry.widgets()[0].id(), "b");
        assert_eq!(registry.widgets()[1].id(), "a");
    }

    #[test]
    fn max_widgets_limits_visible_widgets() {
        let mut registry = WidgetRegistry::with_config(vec![], 1);
        registry.register(Box::new(TestWidget::new("a")));
        registry.register(Box::new(TestWidget::new("b")));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(200.0, 40.0));
        let rects = registry.layout(&ctx);
        assert_eq!(rects.len(), 1);
        assert_eq!(registry.visible_count(), 1);
    }

    #[test]
    fn notify_state_change_calls_lifecycle_hooks() {
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(TestWidget::new("test")));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(100.0, 40.0));
        registry.notify_state_change(IslandState::Media, &ctx);
        registry.notify_state_change(IslandState::Media, &ctx);
        assert_eq!(registry.last_state, Some(IslandState::Media));
    }

    #[test]
    fn notify_show_and_hide_call_hooks() {
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(TestWidget::new("test")));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(100.0, 40.0));
        registry.notify_show(&ctx);
        registry.notify_hide(&ctx);
    }

    #[test]
    fn draw_list_with_capacity_preallocates() {
        let list = DrawList::with_capacity(100);
        assert!(list.capacity() >= 100);
        assert!(list.is_empty());
    }

    #[test]
    fn widget_context_with_config_sets_values() {
        let ctx = WidgetContext::with_config(
            IslandState::Media,
            Size::new(200.0, 40.0),
            "media",
            2.0,
            5.0,
        );
        assert_eq!(ctx.config_id, "media");
        assert_eq!(ctx.config_flex, 2.0);
        assert_eq!(ctx.config_margin, 5.0);
    }

    #[test]
    fn a_disabled_widget_takes_no_space_and_is_not_drawn() {
        let mut registry = WidgetRegistry::with_config(
            vec!["a".to_string(), "b".to_string()],
            2,
        );
        registry.register(Box::new(TestWidget::new("a")));
        registry.register_with(
            Box::new(TestWidget::new("b")),
            WidgetEntry {
                enabled: Some(false),
                ..Default::default()
            },
        );
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(300.0, 40.0));
        let rects = registry.layout(&ctx);
        assert_eq!(rects.len(), 1);
        assert_eq!(registry.visible_count(), 1);

        let mut out = DrawList::new();
        registry.draw(&mut out, &ctx, &rects);
        // Only the enabled widget emits quads; TestWidget draws nothing, so the
        // assertion is about the visible set rather than quad count.
        assert_eq!(registry.visible_count(), 1);
    }

    #[test]
    fn max_widgets_counts_only_enabled_widgets() {
        let mut registry = WidgetRegistry::with_config(vec![], 1);
        registry.register(Box::new(TestWidget::new("a")));
        registry.register_with(
            Box::new(TestWidget::new("b")),
            WidgetEntry {
                enabled: Some(false),
                ..Default::default()
            },
        );
        registry.register(Box::new(TestWidget::new("c")));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(300.0, 40.0));
        let rects = registry.layout(&ctx);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].width, 100.0);
    }

    #[test]
    fn per_widget_margins_and_flex_override_the_container() {
        let mut registry = WidgetRegistry::with_config(vec![], 4);
        registry.register_with(
            Box::new(TestWidget::new("a")),
            WidgetEntry {
                flex: Some(0.0),
                margin_leading: Some(12.0),
                ..Default::default()
            },
        );
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(300.0, 40.0));
        let rects = registry.layout(&ctx);
        assert_eq!(rects[0].x, 12.0);
        assert_eq!(rects[0].width, 100.0);
    }

    #[test]
    fn spacing_and_alignment_from_the_layout_are_applied() {
        let mut registry = WidgetRegistry::with_config(vec![], 4);
        registry.register(Box::new(TestWidget::new("a")));
        registry.register(Box::new(TestWidget::new("b")));
        let layout = LayoutConfig {
            spacing: 10.0,
            align: LayoutAlign::Top,
            order: vec!["a".to_string(), "b".to_string()],
            max_widgets: 4,
            ..Default::default()
        };
        registry.update_layout(&layout);
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(300.0, 100.0));
        let rects = registry.layout(&ctx);
        assert_eq!(rects[1].x, 110.0);
        assert_eq!(rects[0].y, 0.0);
    }

    #[test]
    fn pointer_movement_tracks_the_hovered_widget() {
        let mut registry = WidgetRegistry::with_config(vec![], 4);
        registry.register(Box::new(TestWidget::new("a")));
        registry.register(Box::new(TestWidget::new("b")));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(300.0, 40.0));
        let rects = registry.layout(&ctx);
        let consumed = registry.route_input(
            &WidgetInput::PointerMove(Vec2::new(105.0, 20.0)),
            &ctx,
            &rects,
        );
        assert!(!consumed);
        assert_eq!(registry.hovered_id(), Some("b"));
        registry.route_input(&WidgetInput::PointerLeave, &ctx, &rects);
        assert_eq!(registry.hovered_id(), None);
    }

    struct ConsumeWidget;

    impl Widget for ConsumeWidget {
        fn id(&self) -> &'static str {
            "consumer"
        }
        fn measure(&mut self, _ctx: &WidgetContext) -> Size {
            Size::new(100.0, 40.0)
        }
        fn draw(&mut self, _out: &mut DrawList, _ctx: &WidgetContext, _rect: Rect) {}
        fn on_input(&mut self, ev: &WidgetInput, _ctx: &WidgetContext, _rect: Rect) -> bool {
            matches!(ev, WidgetInput::Click(_))
        }
        fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
            None
        }
        fn priority(&self) -> u8 {
            0
        }
    }

    #[test]
    fn clicks_reach_the_widget_under_the_pointer() {
        let mut registry = WidgetRegistry::with_config(vec![], 4);
        registry.register(Box::new(ConsumeWidget));
        let ctx = WidgetContext::new(IslandState::Collapsed, Size::new(300.0, 40.0));
        let rects = registry.layout(&ctx);
        assert!(registry.route_input(
            &WidgetInput::Click(Vec2::new(50.0, 20.0)),
            &ctx,
            &rects,
        ));
        assert!(!registry.route_input(
            &WidgetInput::Click(Vec2::new(200.0, 20.0)),
            &ctx,
            &rects,
        ));
    }
}
