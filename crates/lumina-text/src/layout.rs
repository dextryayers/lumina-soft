//! Text layout: positioning glyphs inside a container.
//!
//! # The single-row model
//!
//! Lumina's text lives in a horizontal strip inside the island, so the layout
//! is a single row with three alignment options: left, center, right. Multi-line
//! layout is out of scope, because no widget needs it.

use lumina_core::geometry::{Scalar, Vec2};

/// How text is aligned horizontally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignment {
    /// Left aligned.
    Left,
    /// Centered.
    Center,
    /// Right aligned.
    Right,
}

/// How text that does not fit is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    /// Truncate with an ellipsis.
    Ellipsis,
    /// Scroll horizontally.
    Marquee,
    /// Clip without indication.
    Clip,
}

/// A laid out glyph, with its position in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    /// Position in the container.
    pub position: Vec2,
    /// Size in logical pixels.
    pub size: Vec2,
    /// Atlas UV minimum coordinate.
    pub uv_min: Vec2,
    /// Atlas UV maximum coordinate.
    pub uv_max: Vec2,
    /// The glyph's advance in ems.
    pub advance: Scalar,
}

/// A laid out line of text.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// The glyphs, positioned.
    pub glyphs: Vec<PositionedGlyph>,
    /// Total width in logical pixels.
    pub width: Scalar,
    /// Total height in logical pixels.
    pub height: Scalar,
    /// Whether the text was truncated.
    pub truncated: bool,
    /// Whether the text needs marquee scrolling.
    pub needs_marquee: bool,
}

impl Layout {
    /// Lay out a shaped run inside a container.
    ///
    /// `font_size` is in logical pixels. `available_width` is the container's
    /// width. `alignment` is the horizontal alignment.
    #[must_use]
    pub fn new(
        shaped: &super::shaping::ShapedText,
        atlas: &super::atlas::Atlas,
        font_size: Scalar,
        available_width: Scalar,
        alignment: TextAlignment,
        overflow: Overflow,
    ) -> Self {
        if shaped.is_empty() {
            return Self { glyphs: Vec::new(), width: 0.0, height: 0.0, truncated: false, needs_marquee: false };
        }

        let scale = font_size;
        let total_width = shaped.width * scale;

        // Determine whether the text fits.
        let needs_marquee = total_width > available_width && overflow == Overflow::Marquee;
        let truncated = total_width > available_width && overflow == Overflow::Ellipsis;

        // Effective width after truncation.
        let effective_width = if truncated {
            available_width
        } else {
            total_width.min(available_width)
        };

        // Horizontal offset for alignment.
        let offset_x = match alignment {
            TextAlignment::Left => 0.0,
            TextAlignment::Center => (available_width - effective_width) * 0.5,
            TextAlignment::Right => available_width - effective_width,
        };

        let mut glyphs = Vec::with_capacity(shaped.glyphs.len());
        let mut x = offset_x;

        for glyph in &shaped.glyphs {
            let entry = atlas.glyphs.get(glyph.glyph_index);
            let (uv_min, uv_max) = entry.map(|e| e.uv()).unwrap_or((Vec2::ZERO, Vec2::ZERO));
            let glyph_width = entry.map(|e| e.em_width() * scale).unwrap_or(scale * 0.5);
            let glyph_height = entry.map(|e| e.em_height() * scale).unwrap_or(scale);

            // Stop adding glyphs once we run out of room (ellipsis case).
            if truncated && x + glyph_width > available_width - scale * 0.5 {
                // Add an ellipsis glyph if we have one, then stop.
                break;
            }

            glyphs.push(PositionedGlyph {
                position: Vec2::new(x, 0.0),
                size: Vec2::new(glyph_width, glyph_height),
                uv_min,
                uv_max,
                advance: glyph.advance,
            });
            x += glyph.advance * scale;
        }

        Self {
            glyphs,
            width: effective_width,
            height: shaped.height,
            truncated,
            needs_marquee,
        }
    }

    /// True when the text fits without truncation or scrolling.
    #[must_use]
    pub fn fits(&self) -> bool {
        !self.truncated && !self.needs_marquee
    }
}

/// Marquee animation state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marquee {
    /// Current horizontal offset in logical pixels.
    pub offset: Scalar,
    /// Scroll speed in logical pixels per second.
    pub speed: Scalar,
    /// Gap between repetitions.
    pub gap: Scalar,
    /// Whether the marquee is currently paused.
    pub paused: bool,
    /// How long to pause when the pointer enters, in seconds.
    pub pause_duration: Scalar,
    /// Time remaining in the current pause.
    pub pause_remaining: Scalar,
}

impl Marquee {
    /// Construct a marquee with sensible defaults.
    #[must_use]
    pub fn new(speed: Scalar, gap: Scalar, pause_duration: Scalar) -> Self {
        Self {
            offset: 0.0,
            speed: speed.max(1.0),
            gap: gap.max(1.0),
            paused: false,
            pause_duration,
            pause_remaining: 0.0,
        }
    }

    /// Advance the marquee by `dt` seconds.
    ///
    /// Returns the new offset. When the offset exceeds the text width, it
    /// wraps around with the configured gap.
    pub fn advance(&mut self, dt: Scalar, text_width: Scalar) -> Scalar {
        if self.paused {
            if self.pause_remaining > 0.0 {
                self.pause_remaining = (self.pause_remaining - dt).max(0.0);
                if self.pause_remaining <= 0.0 {
                    self.paused = false;
                }
                return self.offset;
            }
            self.paused = false;
        }

        self.offset += self.speed * dt.max(0.0);
        // Modulo rather than a single subtraction, because a large dt (after a
        // stall, or a very fast speed) can carry the offset past the wrap point
        // several times over.
        let wrap_at = text_width + self.gap;
        if wrap_at > 0.0 && self.offset >= wrap_at {
            self.offset %= wrap_at;
        }
        self.offset
    }

    /// Pause the marquee, for example when the pointer enters the island.
    pub fn pause(&mut self) {
        self.paused = true;
        self.pause_remaining = self.pause_duration;
    }

    /// Resume the marquee.
    pub fn resume(&mut self) {
        self.paused = false;
        self.pause_remaining = 0.0;
    }

    /// True when the marquee is currently paused.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused
    }
}

impl Default for Marquee {
    fn default() -> Self {
        Self::new(40.0, 48.0, 1.8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::Atlas;
    use crate::shaping::{ShapedText, ShapedGlyph};

    fn shaped_text(width: f32) -> ShapedText {
        ShapedText {
            glyphs: vec![ShapedGlyph { glyph_index: 0, x: 0.0, y: 0.0, advance: width, kerning: 0.0 }],
            width,
            height: 1.0,
            is_rtl: false,
        }
    }

    fn empty_atlas() -> Atlas {
        Atlas::new()
    }

    #[test]
    fn an_empty_text_produces_an_empty_layout() {
        let shaped = ShapedText { glyphs: Vec::new(), width: 0.0, height: 0.0, is_rtl: false };
        let layout = Layout::new(&shaped, &empty_atlas(), 14.0, 100.0, TextAlignment::Left, Overflow::Clip);
        assert!(layout.glyphs.is_empty());
        assert_eq!(layout.width, 0.0);
        assert!(!layout.truncated);
        assert!(!layout.needs_marquee);
    }

    #[test]
    fn text_that_fits_is_not_truncated() {
        let shaped = shaped_text(50.0);
        let layout = Layout::new(&shaped, &empty_atlas(), 1.0, 100.0, TextAlignment::Left, Overflow::Clip);
        assert!(!layout.truncated);
        assert!(!layout.needs_marquee);
        assert!(layout.fits());
    }

    #[test]
    fn text_that_overflows_with_ellipsis_is_truncated() {
        let shaped = shaped_text(200.0);
        let layout = Layout::new(&shaped, &empty_atlas(), 1.0, 100.0, TextAlignment::Left, Overflow::Ellipsis);
        assert!(layout.truncated);
        assert!(!layout.needs_marquee);
        assert!(!layout.fits());
    }

    #[test]
    fn text_that_overflows_with_marquee_scrolls() {
        let shaped = shaped_text(200.0);
        let layout = Layout::new(&shaped, &empty_atlas(), 1.0, 100.0, TextAlignment::Left, Overflow::Marquee);
        assert!(!layout.truncated);
        assert!(layout.needs_marquee);
    }

    #[test]
    fn center_alignment_offsets_the_text() {
        let shaped = shaped_text(50.0);
        let left = Layout::new(&shaped, &empty_atlas(), 1.0, 100.0, TextAlignment::Left, Overflow::Clip);
        let center = Layout::new(&shaped, &empty_atlas(), 1.0, 100.0, TextAlignment::Center, Overflow::Clip);
        assert_eq!(left.glyphs[0].position.x, 0.0);
        assert!((center.glyphs[0].position.x - 25.0).abs() < 1e-6);
    }

    #[test]
    fn right_alignment_offsets_the_text() {
        let shaped = shaped_text(50.0);
        let right = Layout::new(&shaped, &empty_atlas(), 1.0, 100.0, TextAlignment::Right, Overflow::Clip);
        assert!((right.glyphs[0].position.x - 50.0).abs() < 1e-6);
    }

    #[test]
    fn marquee_advances_over_time() {
        let mut marquee = Marquee::new(40.0, 48.0, 0.0);
        let first = marquee.advance(1.0, 200.0);
        assert!((first - 40.0).abs() < 1e-6);
        let second = marquee.advance(1.0, 200.0);
        assert!((second - 80.0).abs() < 1e-6);
    }

    #[test]
    fn marquee_wraps_around_after_the_text_width_plus_gap() {
        let mut marquee = Marquee::new(100.0, 10.0, 0.0);
        let offset = marquee.advance(2.5, 100.0); // 250 px, wraps at 110
        assert!(offset < 110.0, "should have wrapped, got {offset}");
    }

    #[test]
    fn marquee_paused_does_not_advance() {
        let mut marquee = Marquee::new(40.0, 48.0, 1.0);
        marquee.pause();
        let offset = marquee.advance(0.5, 200.0);
        assert_eq!(offset, 0.0, "a paused marquee must not scroll");
        assert!(marquee.is_paused());
    }

    #[test]
    fn marquee_resumes_after_the_pause_duration() {
        let mut marquee = Marquee::new(40.0, 48.0, 1.0);
        marquee.pause();
        marquee.advance(0.5, 200.0);
        marquee.advance(0.6, 200.0); // pause expires here
        assert!(!marquee.is_paused());
        let offset = marquee.advance(0.1, 200.0);
        assert!(offset > 0.0, "should have started scrolling again");
    }

    #[test]
    fn marquee_handles_zero_dt() {
        let mut marquee = Marquee::new(40.0, 48.0, 0.0);
        let offset = marquee.advance(0.0, 200.0);
        assert_eq!(offset, 0.0);
    }

    #[test]
    fn marquee_handles_negative_dt() {
        let mut marquee = Marquee::new(40.0, 48.0, 0.0);
        marquee.advance(1.0, 200.0);
        let offset = marquee.advance(-1.0, 200.0);
        assert_eq!(offset, 40.0, "negative dt must not move the marquee backwards");
    }

    #[test]
    fn marquee_handles_zero_text_width() {
        let mut marquee = Marquee::new(40.0, 48.0, 0.0);
        let offset = marquee.advance(1.0, 0.0);
        assert_eq!(offset, 40.0, "zero width means no wrap, just scroll");
    }

    #[test]
    fn marquee_defaults_are_sensible() {
        let marquee = Marquee::default();
        assert!(marquee.speed > 0.0);
        assert!(marquee.gap > 0.0);
    }
}