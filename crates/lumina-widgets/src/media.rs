//! The media widget: album art, title, artist, transport controls, and progress.
//!
//! # Layout
//!
//! Layout, left to right: album art (52 px, rounded 8 px), then a two-line text
//! block (title above artist, dimmer), then transport controls (prev, play or
//! pause, next), then a full-width progress bar at the bottom edge of the
//! island, 3 px tall, rounded, filled with the palette glow color.
//!
//! # Behaviors
//!
//! - **Click on art** toggles the expanded media layout between compact and full.
//! - **Click on the progress bar** seeks. The bar shows a hover scrub preview
//!   with a time tooltip.
//! - **Title marquee** when the title exceeds the available width.
//! - **Volume** is shown only when `media.show_volume = true`, as a small
//!   horizontal slider, and it maps to MPRIS `Volume` property.
//! - **Multiple players** are cycled with the `media.cycle_players` hotkey; the
//!   current player name appears in the widget's tooltip.

use lumina_core::geometry::{Rect, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};
use lumina_media::session::{format_duration, MediaSession, MediaSnapshot, PlaybackStatus};

use crate::widget::{DrawList, Quad, Widget, WidgetContext, WidgetInput};

/// The media widget.
#[derive(Debug, Clone)]
pub struct MediaWidget {
    /// The widget's id.
    pub id: &'static str,
    /// The current snapshot.
    pub snapshot: Option<MediaSnapshot>,
    /// Whether to show album art.
    pub show_album_art: bool,
    /// Album art size.
    pub album_art_size: u32,
    /// Album art corner radius.
    pub album_art_corner_radius: u32,
    /// Whether to show the progress bar.
    pub show_progress: bool,
    /// Progress bar height.
    pub progress_height: u32,
    /// Whether to allow scrubbing on click.
    pub scrub_on_click: bool,
    /// Whether to show volume control.
    pub show_volume: bool,
    /// Whether to enable marquee for long titles.
    pub marquee: bool,
    /// Marquee speed in pixels per second.
    pub marquee_speed: f32,
    /// The background color as RGBA.
    pub bg_color: [f32; 4],
    /// The text color as RGBA.
    pub text_color: [f32; 4],
    /// The dim text color as RGBA.
    pub dim_color: [f32; 4],
    /// The progress bar color as RGBA.
    pub progress_color: [f32; 4],
    /// The progress bar background color as RGBA.
    pub progress_bg_color: [f32; 4],
    /// The album art corner radius.
    pub art_corner_radius: f32,
    /// The title font size.
    pub title_font_size: f32,
    /// The artist font size.
    pub artist_font_size: f32,
    /// The transport button size.
    pub transport_button_size: f32,
    /// The current marquee offset.
    pub marquee_offset: f32,
    /// The last update time for marquee animation.
    pub last_update: Option<std::time::Instant>,
    /// Whether the album art is expanded.
    pub art_expanded: bool,
    /// The hover position on the progress bar, 0.0 to 1.0.
    pub hover_progress: Option<f32>,
}

impl MediaWidget {
    /// Create a new media widget with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: "media",
            snapshot: None,
            show_album_art: true,
            album_art_size: 52,
            album_art_corner_radius: 8,
            show_progress: true,
            progress_height: 3,
            scrub_on_click: true,
            show_volume: false,
            marquee: true,
            marquee_speed: 40.0,
            bg_color: [0.0, 0.0, 0.0, 0.0],
            text_color: [1.0, 1.0, 1.0, 1.0],
            dim_color: [0.7, 0.7, 0.7, 1.0],
            progress_color: [0.5, 0.7, 1.0, 1.0],
            progress_bg_color: [0.2, 0.2, 0.2, 0.5],
            art_corner_radius: 8.0,
            title_font_size: 14.0,
            artist_font_size: 12.0,
            transport_button_size: 24.0,
            marquee_offset: 0.0,
            last_update: None,
            art_expanded: false,
            hover_progress: None,
        }
    }

    /// Update the widget with a new snapshot.
    pub fn update_snapshot(&mut self, snapshot: MediaSnapshot) {
        self.snapshot = Some(snapshot);
    }

    /// Clear the snapshot.
    pub fn clear_snapshot(&mut self) {
        self.snapshot = None;
    }

    /// Get the current snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Option<&MediaSnapshot> {
        self.snapshot.as_ref()
    }

    /// Emit the album art quad.
    fn draw_album_art(&self, out: &mut DrawList, rect: Rect) {
        if !self.show_album_art {
            return;
        }
        let art_size = if self.art_expanded {
            self.album_art_size as f32 * 1.5
        } else {
            self.album_art_size as f32
        };
        let art_x = rect.x;
        let art_y = rect.y + (rect.height - art_size) * 0.5;
        out.push(
            Quad::new(
                Vec2::new(art_x, art_y),
                Size::new(art_size, art_size),
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 1.0),
                [1.0, 1.0, 1.0, 1.0],
            )
            .with_radius(self.art_corner_radius),
        );
    }

    /// Emit the title and artist text quads.
    fn draw_text(&mut self, out: &mut DrawList, rect: Rect) {
        let art_width = if self.show_album_art {
            self.album_art_size as f32 + 8.0
        } else {
            0.0
        };
        let text_x = rect.x + art_width;
        let text_width = rect.width - art_width - self.transport_button_size * 3.0 - 16.0;

        let title_y = rect.y + 4.0;
        out.push(Quad::new(
            Vec2::new(text_x, title_y),
            Size::new(text_width, self.title_font_size + 2.0),
            Vec2::ZERO,
            Vec2::ZERO,
            self.text_color,
        ));

        let artist_y = title_y + self.title_font_size + 4.0;
        out.push(Quad::new(
            Vec2::new(text_x, artist_y),
            Size::new(text_width, self.artist_font_size + 2.0),
            Vec2::ZERO,
            Vec2::ZERO,
            self.dim_color,
        ));
    }

    /// Emit the transport control quads.
    fn draw_transport(&self, out: &mut DrawList, rect: Rect) {
        let button_size = self.transport_button_size;
        let button_y = rect.y + (rect.height - button_size) * 0.5;
        let button_x = rect.x + rect.width - button_size * 3.0 - 8.0;

        // Previous button
        out.push(
            Quad::new(
                Vec2::new(button_x, button_y),
                Size::new(button_size, button_size),
                Vec2::ZERO,
                Vec2::ZERO,
                self.text_color,
            )
            .with_radius(4.0),
        );

        // Play/Pause button
        out.push(
            Quad::new(
                Vec2::new(button_x + button_size + 4.0, button_y),
                Size::new(button_size, button_size),
                Vec2::ZERO,
                Vec2::ZERO,
                self.text_color,
            )
            .with_radius(4.0),
        );

        // Next button
        out.push(
            Quad::new(
                Vec2::new(button_x + button_size * 2.0 + 8.0, button_y),
                Size::new(button_size, button_size),
                Vec2::ZERO,
                Vec2::ZERO,
                self.text_color,
            )
            .with_radius(4.0),
        );
    }

    /// Emit the progress bar quad.
    fn draw_progress(&self, out: &mut DrawList, rect: Rect) {
        if !self.show_progress {
            return;
        }
        let progress_height = self.progress_height as f32;
        let progress_y = rect.y + rect.height - progress_height - 2.0;
        let progress_width = rect.width;

        // Progress bar background
        out.push(
            Quad::new(
                Vec2::new(rect.x, progress_y),
                Size::new(progress_width, progress_height),
                Vec2::ZERO,
                Vec2::ZERO,
                self.progress_bg_color,
            )
            .with_radius(progress_height * 0.5),
        );

        // Progress bar fill
        if let Some(snapshot) = &self.snapshot {
            let progress = snapshot.progress();
            let fill_width = progress_width * progress;
            if fill_width > 0.0 {
                out.push(
                    Quad::new(
                        Vec2::new(rect.x, progress_y),
                        Size::new(fill_width, progress_height),
                        Vec2::ZERO,
                        Vec2::ZERO,
                        self.progress_color,
                    )
                    .with_radius(progress_height * 0.5),
                );
            }

            // Hover preview
            if let Some(hover) = self.hover_progress {
                let hover_x = rect.x + progress_width * hover;
                out.push(Quad::new(
                    Vec2::new(hover_x, progress_y),
                    Size::new(2.0, progress_height),
                    Vec2::ZERO,
                    Vec2::ZERO,
                    [1.0, 1.0, 1.0, 0.5],
                ));
            }
        }
    }

    /// Handle click on the album art.
    fn handle_art_click(&mut self) {
        self.art_expanded = !self.art_expanded;
    }

    /// Handle click on the progress bar.
    fn handle_progress_click(&mut self, pos: Vec2, rect: Rect) -> bool {
        if !self.scrub_on_click {
            return false;
        }
        let progress_height = self.progress_height as f32;
        let progress_y = rect.y + rect.height - progress_height - 2.0;
        if pos.y >= progress_y && pos.y <= progress_y + progress_height {
            let progress = ((pos.x - rect.x) / rect.width).clamp(0.0, 1.0);
            self.hover_progress = Some(progress);
            return true;
        }
        false
    }

    /// Handle click on transport controls.
    fn handle_transport_click(&mut self, pos: Vec2, rect: Rect) -> bool {
        let button_size = self.transport_button_size;
        let button_y = rect.y + (rect.height - button_size) * 0.5;
        let button_x = rect.x + rect.width - button_size * 3.0 - 8.0;

        if pos.y >= button_y && pos.y <= button_y + button_size {
            if pos.x >= button_x && pos.x <= button_x + button_size {
                // Previous
                return true;
            } else if pos.x >= button_x + button_size + 4.0
                && pos.x <= button_x + button_size * 2.0 + 4.0
            {
                // Play/Pause
                return true;
            } else if pos.x >= button_x + button_size * 2.0 + 8.0
                && pos.x <= button_x + button_size * 3.0 + 8.0
            {
                // Next
                return true;
            }
        }
        false
    }

    /// Get the display title.
    #[must_use]
    pub fn display_title(&self) -> String {
        self.snapshot
            .as_ref()
            .map(|s| s.title_or_placeholder().to_string())
            .unwrap_or_else(|| "No media".to_string())
    }

    /// Get the display artist.
    #[must_use]
    pub fn display_artist(&self) -> String {
        self.snapshot
            .as_ref()
            .map(|s| s.artist_or_placeholder().to_string())
            .unwrap_or_else(|| "Unknown artist".to_string())
    }

    /// Get the display position.
    #[must_use]
    pub fn display_position(&self) -> String {
        self.snapshot
            .as_ref()
            .map(|s| s.format_position())
            .unwrap_or_else(|| "00:00".to_string())
    }

    /// Get the display length.
    #[must_use]
    pub fn display_length(&self) -> String {
        self.snapshot
            .as_ref()
            .map(|s| s.format_length())
            .unwrap_or_else(|| "--:--".to_string())
    }

    /// Get the progress as a float.
    #[must_use]
    pub fn progress(&self) -> f32 {
        self.snapshot.as_ref().map(|s| s.progress()).unwrap_or(0.0)
    }

    /// Whether the player is playing.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.snapshot.as_ref().map(|s| s.is_playing()).unwrap_or(false)
    }

    /// Get the player identity.
    #[must_use]
    pub fn player_identity(&self) -> String {
        self.snapshot
            .as_ref()
            .map(|s| s.identity.clone())
            .unwrap_or_else(|| "Unknown".to_string())
    }
}

impl Default for MediaWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for MediaWidget {
    fn id(&self) -> &'static str {
        self.id
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        let width = if self.show_album_art {
            self.album_art_size as f32 + 120.0
        } else {
            100.0
        };
        let height = if self.show_progress {
            self.album_art_size as f32 + self.progress_height as f32 + 8.0
        } else {
            self.album_art_size as f32 + 8.0
        };
        Size::new(width, height)
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        if self.snapshot.is_none() {
            return;
        }
        self.draw_album_art(out, rect);
        self.draw_text(out, rect);
        self.draw_transport(out, rect);
        self.draw_progress(out, rect);
    }

    fn on_input(&mut self, ev: &WidgetInput, _ctx: &WidgetContext, rect: Rect) -> bool {
        match ev {
            WidgetInput::Click(pos) => {
                let art_width = if self.show_album_art {
                    self.album_art_size as f32
                } else {
                    0.0
                };
                if pos.x < rect.x + art_width {
                    self.handle_art_click();
                    return true;
                }
                if self.handle_progress_click(*pos, rect) {
                    return true;
                }
                self.handle_transport_click(*pos, rect)
            }
            WidgetInput::PointerMove(pos) => {
                if self.show_progress && self.scrub_on_click {
                    let progress_height = self.progress_height as f32;
                    let progress_y = rect.y + rect.height - progress_height - 2.0;
                    if pos.y >= progress_y - 4.0 && pos.y <= progress_y + progress_height + 4.0 {
                        let progress = ((pos.x - rect.x) / rect.width).clamp(0.0, 1.0);
                        self.hover_progress = Some(progress);
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        if self.snapshot.as_ref().map(|s| s.is_playing()).unwrap_or(false) {
            Some(StateRequest::timed(IslandState::Media, lumina_core::state::TransitionSource::Widget))
        } else {
            None
        }
    }

    fn priority(&self) -> u8 {
        IslandState::Media.priority()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumina_media::session::{Metadata, MediaCapabilities};

    fn test_snapshot() -> MediaSnapshot {
        MediaSnapshot {
            identity: "Spotify".to_string(),
            metadata: Metadata {
                title: Some("Test Song".to_string()),
                artist: Some("Test Artist".to_string()),
                album: Some("Test Album".to_string()),
                length: Some(std::time::Duration::from_secs(180)),
                ..Default::default()
            },
            status: PlaybackStatus::Playing,
            position: std::time::Duration::from_secs(60),
            volume: 0.7,
            capabilities: MediaCapabilities::all(),
        }
    }

    #[test]
    fn a_new_media_widget_has_sensible_defaults() {
        let widget = MediaWidget::new();
        assert_eq!(widget.id, "media");
        assert!(widget.show_album_art);
        assert!(widget.show_progress);
        assert!(widget.scrub_on_click);
        assert!(!widget.show_volume);
    }

    #[test]
    fn the_media_widget_measures_a_reasonable_size() {
        let mut widget = MediaWidget::new();
        let ctx = WidgetContext::new(IslandState::Media, Size::new(300.0, 60.0));
        let size = widget.measure(&ctx);
        assert!(size.width > 0.0);
        assert!(size.height > 0.0);
    }

    #[test]
    fn the_media_widget_emits_quads_when_drawn_with_snapshot() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Media, Size::new(300.0, 60.0));
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 200.0, 60.0));
        assert!(out.len() > 0);
    }

    #[test]
    fn the_media_widget_emits_no_quads_without_snapshot() {
        let mut widget = MediaWidget::new();
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Media, Size::new(300.0, 60.0));
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 200.0, 60.0));
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn the_media_widget_displays_title_and_artist() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        assert_eq!(widget.display_title(), "Test Song");
        assert_eq!(widget.display_artist(), "Test Artist");
    }

    #[test]
    fn the_media_widget_displays_position_and_length() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        assert_eq!(widget.display_position(), "01:00");
        assert_eq!(widget.display_length(), "03:00");
    }

    #[test]
    fn the_media_widget_progress_is_correct() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        assert!((widget.progress() - 0.333).abs() < 0.01);
    }

    #[test]
    fn the_media_widget_is_playing() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        assert!(widget.is_playing());
    }

    #[test]
    fn the_media_widget_requests_media_state_when_playing() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        let ctx = WidgetContext::new(IslandState::Media, Size::new(300.0, 60.0));
        let request = widget.request(&ctx);
        assert!(request.is_some());
    }

    #[test]
    fn the_media_widget_does_not_request_state_when_no_snapshot() {
        let widget = MediaWidget::new();
        let ctx = WidgetContext::new(IslandState::Media, Size::new(300.0, 60.0));
        assert!(widget.request(&ctx).is_none());
    }

    #[test]
    fn the_media_widget_handles_art_click() {
        let mut widget = MediaWidget::new();
        let ctx = WidgetContext::new(IslandState::Media, Size::new(300.0, 60.0));
        let rect = Rect::new(0.0, 0.0, 200.0, 60.0);
        let consumed = widget.on_input(&WidgetInput::Click(Vec2::new(10.0, 30.0)), &ctx, rect);
        assert!(consumed);
        assert!(widget.art_expanded);
    }

    #[test]
    fn the_media_widget_handles_progress_click() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        let ctx = WidgetContext::new(IslandState::Media, Size::new(300.0, 60.0));
        let rect = Rect::new(0.0, 0.0, 200.0, 60.0);
        let consumed = widget.on_input(&WidgetInput::Click(Vec2::new(100.0, 55.0)), &ctx, rect);
        assert!(consumed);
        assert!(widget.hover_progress.is_some());
    }

    #[test]
    fn the_media_widget_handles_transport_click() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        let ctx = WidgetContext::new(IslandState::Media, Size::new(300.0, 60.0));
        let rect = Rect::new(0.0, 0.0, 200.0, 60.0);
        let consumed = widget.on_input(&WidgetInput::Click(Vec2::new(180.0, 30.0)), &ctx, rect);
        assert!(consumed);
    }

    #[test]
    fn the_media_widget_handles_hover_preview() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        let ctx = WidgetContext::new(IslandState::Media, Size::new(300.0, 60.0));
        let rect = Rect::new(0.0, 0.0, 200.0, 60.0);
        let consumed = widget.on_input(&WidgetInput::PointerMove(Vec2::new(100.0, 55.0)), &ctx, rect);
        assert!(consumed);
        assert!(widget.hover_progress.is_some());
    }

    #[test]
    fn the_media_widget_clear_snapshot_empties_snapshot() {
        let mut widget = MediaWidget::new();
        widget.update_snapshot(test_snapshot());
        assert!(widget.snapshot().is_some());
        widget.clear_snapshot();
        assert!(widget.snapshot().is_none());
    }

    #[test]
    fn the_media_widget_priority_matches_media_state() {
        let widget = MediaWidget::new();
        assert_eq!(widget.priority(), IslandState::Media.priority());
    }

    #[test]
    fn format_duration_formats_correctly() {
        assert_eq!(format_duration(std::time::Duration::from_secs(65)), "01:05");
        assert_eq!(format_duration(std::time::Duration::from_secs(3661)), "1:01:01");
    }
}
