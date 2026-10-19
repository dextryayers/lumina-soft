//! The shortcuts widget: a row of user configured action tiles.
//!
//! Only actions that are runnable, visible in the island, and not restricted to
//! IPC are shown. Clicking a tile returns the action id through
//! [`ShortcutsWidget::take_activated`]; the orchestrator owns process spawning,
//! so the widget stays free of side effects.
//!
//! When there are more actions than fit, the tail is summarised as `+N` and the
//! remaining actions stay reachable through the hotkey or IPC paths.

use lumina_config::schema::{ShortcutConfig, ShortcutsConfig};
use lumina_core::geometry::{Rect, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};

use crate::paint::{self, ACCENT, DIM, MUTED, TEXT};
use crate::widget::{DrawList, Widget, WidgetContext, WidgetInput};

/// Maximum number of tiles the widget lays out at once.
pub const MAX_TILES: usize = 6;
const TILE_SIZE: f32 = 28.0;
const GAP: f32 = 6.0;

/// The shortcuts widget.
#[derive(Debug, Clone, Default)]
pub struct ShortcutsWidget {
    actions: Vec<ShortcutConfig>,
    activated: Option<String>,
    hovered: Option<usize>,
}

impl ShortcutsWidget {
    /// Build the widget from the shortcuts configuration.
    #[must_use]
    pub fn from_config(config: &ShortcutsConfig) -> Self {
        let actions = config
            .list
            .iter()
            .filter(|action| action.show_in_island && !action.ipc_only && action.is_runnable())
            .cloned()
            .collect();
        Self {
            actions,
            activated: None,
            hovered: None,
        }
    }

    /// The actions that will be displayed, in configuration order.
    #[must_use]
    pub fn actions(&self) -> &[ShortcutConfig] {
        &self.actions
    }

    /// Number of actions that will be displayed.
    #[must_use]
    pub fn visible_actions(&self) -> usize {
        self.actions.len().min(MAX_TILES)
    }

    /// Number of actions folded into the overflow indicator.
    #[must_use]
    pub fn overflow(&self) -> usize {
        self.actions.len().saturating_sub(MAX_TILES)
    }

    /// The id of the action that was just activated, if any.
    pub fn take_activated(&mut self) -> Option<String> {
        self.activated.take()
    }

    /// The index of the tile under a point, taking gaps into account.
    #[must_use]
    pub fn tile_at(&self, position: Vec2, rect: Rect) -> Option<usize> {
        if position.y < rect.y || position.y > rect.y + TILE_SIZE {
            return None;
        }
        let visible = self.visible_actions();
        for index in 0..visible {
            let x = rect.x + index as f32 * (TILE_SIZE + GAP);
            if position.x >= x && position.x <= x + TILE_SIZE {
                return Some(index);
            }
        }
        None
    }
}

impl Widget for ShortcutsWidget {
    fn id(&self) -> &'static str {
        "shortcuts"
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        let tiles = self.visible_actions().max(1) as f32;
        let overflow = if self.overflow() > 0 { 18.0 } else { 0.0 };
        Size::new(
            tiles * TILE_SIZE + (tiles - 1.0) * GAP + overflow,
            TILE_SIZE,
        )
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        if self.actions.is_empty() {
            paint::block(
                out,
                Rect::new(
                    rect.x,
                    rect.y + TILE_SIZE * 0.5 - 1.0,
                    rect.width.min(40.0),
                    2.0,
                ),
                MUTED,
                1.0,
            );
            return;
        }

        for index in 0..self.visible_actions() {
            let x = rect.x + index as f32 * (TILE_SIZE + GAP);
            let tile = Rect::new(x, rect.y, TILE_SIZE, TILE_SIZE);
            let hovered = self.hovered == Some(index);
            paint::block(
                out,
                tile,
                if hovered {
                    [1.0, 1.0, 1.0, 0.18]
                } else {
                    [1.0, 1.0, 1.0, 0.08]
                },
                8.0,
            );
            paint::block(
                out,
                Rect::new(tile.x + 8.0, tile.y + 5.0, 12.0, 12.0),
                ACCENT,
                4.0,
            );
            paint::label(
                out,
                Rect::new(tile.x + 4.0, tile.y + 19.0, TILE_SIZE - 8.0, 4.0),
                DIM,
            );
        }

        if self.overflow() > 0 {
            let x = rect.x + self.visible_actions() as f32 * (TILE_SIZE + GAP);
            paint::label(
                out,
                Rect::new(x, rect.y + TILE_SIZE * 0.5 - 4.0, 14.0, 8.0),
                TEXT,
            );
        }
    }

    fn on_input(&mut self, ev: &WidgetInput, _ctx: &WidgetContext, rect: Rect) -> bool {
        match ev {
            WidgetInput::PointerMove(position) => {
                self.hovered = self.tile_at(*position, rect);
                false
            }
            WidgetInput::Click(position) => {
                let Some(index) = self.tile_at(*position, rect) else {
                    return false;
                };
                let Some(action) = self.actions.get(index) else {
                    return false;
                };
                self.activated = Some(action.id.clone());
                true
            }
            WidgetInput::PointerLeave => {
                self.hovered = None;
                false
            }
            _ => false,
        }
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        None
    }

    fn priority(&self) -> u8 {
        IslandState::Shortcuts.priority()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(id: &str, visible: bool, ipc_only: bool) -> ShortcutConfig {
        ShortcutConfig {
            id: id.to_string(),
            label: id.to_string(),
            command: "true".to_string(),
            show_in_island: visible,
            ipc_only,
            ..ShortcutConfig::default()
        }
    }

    fn config(actions: Vec<ShortcutConfig>) -> ShortcutsConfig {
        ShortcutsConfig { list: actions }
    }

    fn ctx() -> WidgetContext {
        WidgetContext::new(IslandState::Shortcuts, Size::new(300.0, 40.0))
    }

    fn rect() -> Rect {
        Rect::new(0.0, 0.0, 280.0, 28.0)
    }

    #[test]
    fn only_visible_runnable_non_ipc_actions_are_kept() {
        let widget = ShortcutsWidget::from_config(&config(vec![
            action("a", true, false),
            action("b", false, false),
            action("c", true, true),
        ]));
        assert_eq!(widget.actions().len(), 1);
        assert_eq!(widget.actions()[0].id, "a");
    }

    #[test]
    fn actions_without_a_command_are_dropped() {
        let mut empty = action("empty", true, false);
        empty.command.clear();
        let widget = ShortcutsWidget::from_config(&config(vec![empty]));
        assert_eq!(widget.actions().len(), 0);
    }

    #[test]
    fn overflow_is_summarised() {
        let actions = (0..8)
            .map(|i| action(&format!("a{i}"), true, false))
            .collect();
        let widget = ShortcutsWidget::from_config(&config(actions));
        assert_eq!(widget.visible_actions(), MAX_TILES);
        assert_eq!(widget.overflow(), 2);
    }

    #[test]
    fn clicking_a_tile_reports_the_action_id() {
        let mut widget = ShortcutsWidget::from_config(&config(vec![
            action("a", true, false),
            action("b", true, false),
        ]));
        let consumed = widget.on_input(
            &WidgetInput::Click(Vec2::new(TILE_SIZE + GAP + 2.0, 10.0)),
            &ctx(),
            rect(),
        );
        assert!(consumed);
        assert_eq!(widget.take_activated().as_deref(), Some("b"));
    }

    #[test]
    fn clicking_a_gap_does_not_activate() {
        let mut widget = ShortcutsWidget::from_config(&config(vec![action("a", true, false)]));
        let consumed = widget.on_input(
            &WidgetInput::Click(Vec2::new(TILE_SIZE + 2.0, 10.0)),
            &ctx(),
            rect(),
        );
        assert!(!consumed);
        assert_eq!(widget.take_activated(), None);
    }

    #[test]
    fn an_empty_widget_draws_a_muted_placeholder() {
        let mut widget = ShortcutsWidget::default();
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, rect());
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn drawing_tiles_emits_one_group_per_action() {
        let mut widget = ShortcutsWidget::from_config(&config(vec![
            action("a", true, false),
            action("b", true, false),
        ]));
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, rect());
        assert!(out.len() >= 6);
    }

    #[test]
    fn the_widget_never_requests_a_state() {
        let widget = ShortcutsWidget::from_config(&config(vec![action("a", true, false)]));
        assert!(widget.request(&ctx()).is_none());
    }
}
