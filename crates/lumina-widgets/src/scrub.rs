//! Scrub on click with hover preview and time tooltip.
//!
//! # The scrub interaction
//!
//! The progress bar supports two interactions:
//!
//! 1. **Hover preview**: When the pointer hovers over the progress bar, a
//!    preview indicator shows the position that would be seeked to if the user
//!    clicked. A tooltip shows the time at that position.
//! 2. **Click to seek**: When the user clicks on the progress bar, the player
//!    seeks to the clicked position.
//!
//! # Time tooltip
//!
//! The tooltip shows the time at the hover position, formatted as MM:SS or
//! H:MM:SS. The tooltip is drawn above the progress bar, centered on the hover
//! position.

use lumina_core::geometry::{Rect, Size, Vec2};
use lumina_media::session::format_duration;

use crate::widget::{DrawList, Quad};

/// The scrub controller.
///
/// Handles hover preview and click-to-seek on the progress bar.
#[derive(Debug, Clone)]
pub struct ScrubController {
    /// The progress bar rectangle.
    bar_rect: Rect,
    /// The current progress, 0.0 to 1.0.
    progress: f32,
    /// The hover position, 0.0 to 1.0, or None when not hovering.
    hover_position: Option<f32>,
    /// Whether the player can seek.
    can_seek: bool,
    /// The track length in seconds.
    track_length_secs: f32,
    /// The tooltip text.
    tooltip_text: String,
    /// The tooltip position.
    tooltip_pos: Vec2,
    /// The tooltip visible flag.
    tooltip_visible: bool,
}

impl ScrubController {
    /// Create a new scrub controller.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bar_rect: Rect::ZERO,
            progress: 0.0,
            hover_position: None,
            can_seek: true,
            track_length_secs: 0.0,
            tooltip_text: String::new(),
            tooltip_pos: Vec2::ZERO,
            tooltip_visible: false,
        }
    }

    /// Update the progress bar rectangle.
    pub fn set_bar_rect(&mut self, rect: Rect) {
        self.bar_rect = rect;
    }

    /// Update the current progress.
    pub fn set_progress(&mut self, progress: f32) {
        self.progress = progress.clamp(0.0, 1.0);
    }

    /// Update the track length.
    pub fn set_track_length(&mut self, length_secs: f32) {
        self.track_length_secs = length_secs;
    }

    /// Update whether the player can seek.
    pub fn set_can_seek(&mut self, can_seek: bool) {
        self.can_seek = can_seek;
    }

    /// Get the hover position.
    #[must_use]
    pub fn hover_position(&self) -> Option<f32> {
        self.hover_position
    }

    /// Get the tooltip text.
    #[must_use]
    pub fn tooltip_text(&self) -> &str {
        &self.tooltip_text
    }

    /// Whether the tooltip is visible.
    #[must_use]
    pub fn tooltip_visible(&self) -> bool {
        self.tooltip_visible
    }

    /// Handle a pointer move event.
    ///
    /// Returns true if the hover state changed.
    pub fn handle_pointer_move(&mut self, pos: Vec2) -> bool {
        if !self.can_seek {
            return false;
        }
        let old_hover = self.hover_position;
        if self.is_over_bar(pos) {
            let position = self.position_from_x(pos.x);
            self.hover_position = Some(position);
            self.update_tooltip(position);
            self.tooltip_visible = true;
        } else {
            self.hover_position = None;
            self.tooltip_visible = false;
        }
        old_hover != self.hover_position
    }

    /// Handle a click event.
    ///
    /// Returns the position to seek to, or None if the click was not on the bar.
    pub fn handle_click(&mut self, pos: Vec2) -> Option<f32> {
        if !self.can_seek {
            return None;
        }
        if self.is_over_bar(pos) {
            let position = self.position_from_x(pos.x);
            self.hover_position = Some(position);
            self.update_tooltip(position);
            Some(position)
        } else {
            None
        }
    }

    /// Handle a pointer leave event.
    pub fn handle_pointer_leave(&mut self) {
        self.hover_position = None;
        self.tooltip_visible = false;
    }

    /// Check if a position is over the progress bar.
    fn is_over_bar(&self, pos: Vec2) -> bool {
        let bar_top = self.bar_rect.y - 4.0;
        let bar_bottom = self.bar_rect.y + self.bar_rect.height + 4.0;
        pos.x >= self.bar_rect.x
            && pos.x <= self.bar_rect.x + self.bar_rect.width
            && pos.y >= bar_top
            && pos.y <= bar_bottom
    }

    /// Convert an x coordinate to a progress position.
    fn position_from_x(&self, x: f32) -> f32 {
        if self.bar_rect.width <= 0.0 {
            return 0.0;
        }
        ((x - self.bar_rect.x) / self.bar_rect.width).clamp(0.0, 1.0)
    }

    /// Update the tooltip text and position.
    fn update_tooltip(&mut self, position: f32) {
        let time_secs = position * self.track_length_secs;
        self.tooltip_text = format_duration(std::time::Duration::from_secs_f32(time_secs));
        self.tooltip_pos = Vec2::new(
            self.bar_rect.x + self.bar_rect.width * position,
            self.bar_rect.y - 16.0,
        );
    }

    /// Emit the scrub preview and tooltip quads.
    pub fn draw(&self, out: &mut DrawList) {
        if !self.can_seek {
            return;
        }

        // Hover preview indicator
        if let Some(hover) = self.hover_position {
            let hover_x = self.bar_rect.x + self.bar_rect.width * hover;
            out.push(Quad::new(
                Vec2::new(hover_x, self.bar_rect.y),
                Size::new(2.0, self.bar_rect.height),
                Vec2::ZERO,
                Vec2::ZERO,
                [1.0, 1.0, 1.0, 0.5],
            ));
        }

        // Tooltip
        if self.tooltip_visible && !self.tooltip_text.is_empty() {
            let tooltip_width = self.tooltip_text.len() as f32 * 6.0 + 8.0;
            let tooltip_height = 14.0;
            let tooltip_x = (self.tooltip_pos.x - tooltip_width * 0.5)
                .clamp(self.bar_rect.x, self.bar_rect.x + self.bar_rect.width - tooltip_width);
            let tooltip_y = self.tooltip_pos.y;

            // Tooltip background
            out.push(
                Quad::new(
                    Vec2::new(tooltip_x, tooltip_y),
                    Size::new(tooltip_width, tooltip_height),
                    Vec2::ZERO,
                    Vec2::ZERO,
                    [0.1, 0.1, 0.1, 0.9],
                )
                .with_radius(4.0),
            );

            // Tooltip text
            out.push(Quad::new(
                Vec2::new(tooltip_x + 4.0, tooltip_y + 2.0),
                Size::new(tooltip_width - 8.0, tooltip_height - 4.0),
                Vec2::ZERO,
                Vec2::ZERO,
                [1.0, 1.0, 1.0, 1.0],
            ));
        }
    }
}

impl Default for ScrubController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_scrub_controller_has_no_hover() {
        let scrub = ScrubController::new();
        assert!(scrub.hover_position().is_none());
        assert!(!scrub.tooltip_visible());
    }

    #[test]
    fn scrub_controller_handles_pointer_move_over_bar() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.set_track_length(180.0);
        let changed = scrub.handle_pointer_move(Vec2::new(50.0, 51.0));
        assert!(changed);
        assert!(scrub.hover_position().is_some());
        assert!(scrub.tooltip_visible());
    }

    #[test]
    fn scrub_controller_handles_pointer_move_outside_bar() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.set_track_length(180.0);
        scrub.handle_pointer_move(Vec2::new(50.0, 51.0));
        let changed = scrub.handle_pointer_move(Vec2::new(50.0, 100.0));
        assert!(changed);
        assert!(scrub.hover_position().is_none());
        assert!(!scrub.tooltip_visible());
    }

    #[test]
    fn scrub_controller_handles_click_on_bar() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.set_track_length(180.0);
        let position = scrub.handle_click(Vec2::new(50.0, 51.0));
        assert!(position.is_some());
        assert!((position.unwrap() - 0.5).abs() < 0.01);
    }

    #[test]
    fn scrub_controller_handles_click_outside_bar() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        let position = scrub.handle_click(Vec2::new(50.0, 100.0));
        assert!(position.is_none());
    }

    #[test]
    fn scrub_controller_does_not_seek_when_cannot_seek() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.set_can_seek(false);
        let position = scrub.handle_click(Vec2::new(50.0, 51.0));
        assert!(position.is_none());
    }

    #[test]
    fn scrub_controller_handles_pointer_leave() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.handle_pointer_move(Vec2::new(50.0, 51.0));
        scrub.handle_pointer_leave();
        assert!(scrub.hover_position().is_none());
        assert!(!scrub.tooltip_visible());
    }

    #[test]
    fn scrub_controller_tooltip_shows_time() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.set_track_length(180.0);
        scrub.handle_pointer_move(Vec2::new(50.0, 51.0));
        assert_eq!(scrub.tooltip_text(), "01:30");
    }

    #[test]
    fn scrub_controller_tooltip_shows_correct_time_at_start() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.set_track_length(180.0);
        scrub.handle_pointer_move(Vec2::new(0.0, 51.0));
        assert_eq!(scrub.tooltip_text(), "00:00");
    }

    #[test]
    fn scrub_controller_tooltip_shows_correct_time_at_end() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.set_track_length(180.0);
        scrub.handle_pointer_move(Vec2::new(100.0, 51.0));
        assert_eq!(scrub.tooltip_text(), "03:00");
    }

    #[test]
    fn scrub_controller_emits_quads_when_hovering() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.set_track_length(180.0);
        scrub.handle_pointer_move(Vec2::new(50.0, 51.0));
        let mut out = DrawList::new();
        scrub.draw(&mut out);
        assert!(!out.is_empty());
    }

    #[test]
    fn scrub_controller_emits_no_quads_when_not_hovering() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        let mut out = DrawList::new();
        scrub.draw(&mut out);
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn scrub_controller_emits_no_quads_when_cannot_seek() {
        let mut scrub = ScrubController::new();
        scrub.set_bar_rect(Rect::new(0.0, 50.0, 100.0, 3.0));
        scrub.set_can_seek(false);
        scrub.handle_pointer_move(Vec2::new(50.0, 51.0));
        let mut out = DrawList::new();
        scrub.draw(&mut out);
        assert_eq!(out.len(), 0);
    }
}
