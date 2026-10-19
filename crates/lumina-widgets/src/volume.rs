//! Volume slider with MPRIS/GSMTC volume control.
//!
//! # The volume slider
//!
//! The volume slider is a small horizontal slider that appears in the media
//! widget when `media.show_volume = true`. It maps to the MPRIS `Volume`
//! property on Linux and the GSMTC volume control on Windows.
//!
//! # Interaction
//!
//! - **Click and drag**: Click on the slider to set the volume, or drag to
//!   adjust it continuously.
//! - **Scroll**: Scroll over the slider to adjust the volume in small steps.
//! - **Mute toggle**: Click on the speaker icon to mute or unmute.

use lumina_core::geometry::{Rect, Size, Vec2};

use crate::widget::{DrawList, Quad};

/// The volume slider controller.
///
/// Handles volume adjustment through click, drag, and scroll interactions.
#[derive(Debug, Clone)]
pub struct VolumeSlider {
    /// The slider rectangle.
    slider_rect: Rect,
    /// The current volume, 0.0 to 1.0.
    volume: f32,
    /// Whether the slider is being dragged.
    dragging: bool,
    /// Whether the slider is hovered.
    hovered: bool,
    /// The mute toggle button rectangle.
    mute_rect: Rect,
    /// Whether the volume is muted.
    muted: bool,
    /// The volume before muting, for restore.
    pre_mute_volume: f32,
    /// The slider track color.
    track_color: [f32; 4],
    /// The slider fill color.
    fill_color: [f32; 4],
    /// The slider handle color.
    handle_color: [f32; 4],
    /// The mute button color.
    mute_color: [f32; 4],
    /// The slider height.
    slider_height: f32,
    /// The handle radius.
    handle_radius: f32,
}

impl VolumeSlider {
    /// Create a new volume slider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slider_rect: Rect::ZERO,
            volume: 0.5,
            dragging: false,
            hovered: false,
            mute_rect: Rect::ZERO,
            muted: false,
            pre_mute_volume: 0.5,
            track_color: [0.2, 0.2, 0.2, 0.5],
            fill_color: [0.5, 0.7, 1.0, 1.0],
            handle_color: [1.0, 1.0, 1.0, 1.0],
            mute_color: [0.7, 0.7, 0.7, 1.0],
            slider_height: 4.0,
            handle_radius: 6.0,
        }
    }

    /// Update the slider rectangle.
    pub fn set_slider_rect(&mut self, rect: Rect) {
        self.slider_rect = rect;
    }

    /// Update the mute button rectangle.
    pub fn set_mute_rect(&mut self, rect: Rect) {
        self.mute_rect = rect;
    }

    /// Update the current volume.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    /// Get the current volume.
    #[must_use]
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Whether the slider is being dragged.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Whether the slider is hovered.
    #[must_use]
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// Whether the volume is muted.
    #[must_use]
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Handle a pointer move event.
    ///
    /// Returns true if the volume changed.
    pub fn handle_pointer_move(&mut self, pos: Vec2) -> bool {
        let old_volume = self.volume;
        self.hovered = self.is_over_slider(pos) || self.is_over_mute(pos);
        if self.dragging {
            self.volume = self.volume_from_x(pos.x);
        }
        old_volume != self.volume
    }

    /// Handle a click event.
    ///
    /// Returns true if the click was consumed.
    pub fn handle_click(&mut self, pos: Vec2) -> bool {
        if self.is_over_mute(pos) {
            self.toggle_mute();
            return true;
        }
        if self.is_over_slider(pos) {
            self.dragging = true;
            self.volume = self.volume_from_x(pos.x);
            return true;
        }
        false
    }

    /// Handle a pointer up event.
    pub fn handle_pointer_up(&mut self) {
        self.dragging = false;
    }

    /// Handle a pointer leave event.
    pub fn handle_pointer_leave(&mut self) {
        self.hovered = false;
        self.dragging = false;
    }

    /// Handle a scroll event.
    ///
    /// Returns true if the volume changed.
    pub fn handle_scroll(&mut self, delta: f32) -> bool {
        if !self.hovered {
            return false;
        }
        let old_volume = self.volume;
        let step = if delta > 0.0 { 0.05 } else { -0.05 };
        self.volume = (self.volume + step).clamp(0.0, 1.0);
        old_volume != self.volume
    }

    /// Toggle mute.
    pub fn toggle_mute(&mut self) {
        if self.muted {
            self.volume = self.pre_mute_volume;
            self.muted = false;
        } else {
            self.pre_mute_volume = self.volume;
            self.volume = 0.0;
            self.muted = true;
        }
    }

    /// Check if a position is over the slider.
    fn is_over_slider(&self, pos: Vec2) -> bool {
        let slider_top = self.slider_rect.y - 4.0;
        let slider_bottom = self.slider_rect.y + self.slider_rect.height + 4.0;
        pos.x >= self.slider_rect.x
            && pos.x <= self.slider_rect.x + self.slider_rect.width
            && pos.y >= slider_top
            && pos.y <= slider_bottom
    }

    /// Check if a position is over the mute button.
    fn is_over_mute(&self, pos: Vec2) -> bool {
        self.mute_rect.contains(pos)
    }

    /// Convert an x coordinate to a volume.
    fn volume_from_x(&self, x: f32) -> f32 {
        if self.slider_rect.width <= 0.0 {
            return 0.0;
        }
        ((x - self.slider_rect.x) / self.slider_rect.width).clamp(0.0, 1.0)
    }

    /// Emit the volume slider quads.
    fn draw(&self, out: &mut DrawList) {
        // Slider track
        out.push(
            Quad::new(
                Vec2::new(self.slider_rect.x, self.slider_rect.y),
                Size::new(self.slider_rect.width, self.slider_height),
                Vec2::ZERO,
                Vec2::ZERO,
                self.track_color,
            )
            .with_radius(self.slider_height * 0.5),
        );

        // Slider fill
        let fill_width = self.slider_rect.width * self.volume;
        if fill_width > 0.0 {
            out.push(
                Quad::new(
                    Vec2::new(self.slider_rect.x, self.slider_rect.y),
                    Size::new(fill_width, self.slider_height),
                    Vec2::ZERO,
                    Vec2::ZERO,
                    self.fill_color,
                )
                .with_radius(self.slider_height * 0.5),
            );
        }

        // Slider handle
        let handle_x = self.slider_rect.x + fill_width;
        let handle_y = self.slider_rect.y + self.slider_height * 0.5;
        out.push(
            Quad::new(
                Vec2::new(handle_x - self.handle_radius, handle_y - self.handle_radius),
                Size::new(self.handle_radius * 2.0, self.handle_radius * 2.0),
                Vec2::ZERO,
                Vec2::ZERO,
                self.handle_color,
            )
            .with_radius(self.handle_radius),
        );

        // Mute button
        out.push(
            Quad::new(
                Vec2::new(self.mute_rect.x, self.mute_rect.y),
                self.mute_rect.size(),
                Vec2::ZERO,
                Vec2::ZERO,
                self.mute_color,
            )
            .with_radius(4.0),
        );
    }
}

impl Default for VolumeSlider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_volume_slider_has_default_volume() {
        let slider = VolumeSlider::new();
        assert!((slider.volume() - 0.5).abs() < 0.01);
        assert!(!slider.is_muted());
    }

    #[test]
    fn volume_slider_handles_click_on_slider() {
        let mut slider = VolumeSlider::new();
        slider.set_slider_rect(Rect::new(0.0, 0.0, 100.0, 4.0));
        let consumed = slider.handle_click(Vec2::new(50.0, 2.0));
        assert!(consumed);
        assert!(slider.is_dragging());
        assert!((slider.volume() - 0.5).abs() < 0.01);
    }

    #[test]
    fn volume_slider_handles_click_on_mute() {
        let mut slider = VolumeSlider::new();
        slider.set_mute_rect(Rect::new(0.0, 0.0, 20.0, 20.0));
        let consumed = slider.handle_click(Vec2::new(10.0, 10.0));
        assert!(consumed);
        assert!(slider.is_muted());
    }

    #[test]
    fn volume_slider_toggle_mute_restores_volume() {
        let mut slider = VolumeSlider::new();
        slider.set_volume(0.7);
        slider.set_mute_rect(Rect::new(0.0, 0.0, 20.0, 20.0));
        slider.handle_click(Vec2::new(10.0, 10.0));
        assert!(slider.is_muted());
        assert_eq!(slider.volume(), 0.0);
        slider.handle_click(Vec2::new(10.0, 10.0));
        assert!(!slider.is_muted());
        assert!((slider.volume() - 0.7).abs() < 0.01);
    }

    #[test]
    fn volume_slider_handles_scroll() {
        let mut slider = VolumeSlider::new();
        slider.set_slider_rect(Rect::new(0.0, 0.0, 100.0, 4.0));
        slider.handle_pointer_move(Vec2::new(50.0, 2.0));
        let changed = slider.handle_scroll(1.0);
        assert!(changed);
        assert!(slider.volume() > 0.5);
    }

    #[test]
    fn volume_slider_handles_pointer_up() {
        let mut slider = VolumeSlider::new();
        slider.set_slider_rect(Rect::new(0.0, 0.0, 100.0, 4.0));
        slider.handle_click(Vec2::new(50.0, 2.0));
        assert!(slider.is_dragging());
        slider.handle_pointer_up();
        assert!(!slider.is_dragging());
    }

    #[test]
    fn volume_slider_handles_pointer_leave() {
        let mut slider = VolumeSlider::new();
        slider.set_slider_rect(Rect::new(0.0, 0.0, 100.0, 4.0));
        slider.handle_pointer_move(Vec2::new(50.0, 2.0));
        assert!(slider.is_hovered());
        slider.handle_pointer_leave();
        assert!(!slider.is_hovered());
    }

    #[test]
    fn volume_slider_emits_quads_when_drawn() {
        let mut slider = VolumeSlider::new();
        slider.set_slider_rect(Rect::new(0.0, 0.0, 100.0, 4.0));
        slider.set_mute_rect(Rect::new(110.0, 0.0, 20.0, 20.0));
        let mut out = DrawList::new();
        slider.draw(&mut out);
        assert!(out.len() > 0);
    }

    #[test]
    fn volume_slider_volume_is_clamped() {
        let mut slider = VolumeSlider::new();
        slider.set_volume(1.5);
        assert_eq!(slider.volume(), 1.0);
        slider.set_volume(-0.5);
        assert_eq!(slider.volume(), 0.0);
    }

    #[test]
    fn volume_slider_click_outside_is_not_consumed() {
        let mut slider = VolumeSlider::new();
        slider.set_slider_rect(Rect::new(0.0, 0.0, 100.0, 4.0));
        let consumed = slider.handle_click(Vec2::new(200.0, 200.0));
        assert!(!consumed);
    }
}
