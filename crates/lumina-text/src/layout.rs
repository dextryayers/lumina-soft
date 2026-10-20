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
            return Self {
                glyphs: Vec::new(),
                width: 0.0,
                height: 0.0,
                truncated: false,
                needs_marquee: false,
            };
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
            let (uv_min, uv_max) =
                entry.map_or((Vec2::ZERO, Vec2::ZERO), super::atlas::GlyphEntry::uv);
            let glyph_width = entry.map_or(scale * 0.5, |e| e.em_width() * scale);
            let glyph_height = entry.map_or(scale, |e| e.em_height() * scale);

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

/// Cache statistics for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutCacheStats {
    /// Number of lookups served from the cache.
    pub hits: u64,
    /// Number of lookups that required shaping and layout.
    pub misses: u64,
    /// Entries currently retained.
    pub entries: usize,
    /// Maximum entries the cache will retain.
    pub capacity: usize,
}

impl LayoutCacheStats {
    /// Fraction of lookups served from the cache, in `0.0..=1.0`.
    #[must_use]
    pub fn hit_rate(&self) -> f32 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f32 / total as f32
        }
    }
}

/// A bounded cache of measured strings to positioned glyphs.
///
/// Widgets ask for the same label every frame, so caching the full layout
/// removes repeated shaping, atlas lookups, and glyph allocation from steady
/// state. The key includes the text, font size, container width, alignment,
/// overflow mode, and shaper flags. A new atlas (different glyph count or
/// storage) also produces a miss.
///
/// The cache is intentionally a small linear structure: with the default 64
/// entries, scanning keys is cheaper than hashing long strings and requires no
/// per-lookup allocation. Eviction removes the least recently used entry.
///
/// The returned reference points into the cache. Callers must re-borrow on the
/// next lookup rather than holding it across another call that may evict it.
#[derive(Debug)]
pub struct TextLayoutCache {
    entries: Vec<CacheEntry>,
    capacity: usize,
    clock: u64,
    hits: u64,
    misses: u64,
}

#[derive(Debug)]
struct CacheEntry {
    key: CacheKey,
    layout: Layout,
    last_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    text: String,
    font_size_bits: u32,
    available_width_bits: u32,
    alignment: TextAlignment,
    overflow: Overflow,
    apply_kerning: bool,
    atlas_len: usize,
    atlas_ptr: usize,
}

impl TextLayoutCache {
    /// Create a cache retaining at most `capacity` layouts.
    ///
    /// A capacity of zero is promoted to one so insertion always succeeds.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity.min(64)),
            capacity: capacity.max(1),
            clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Lay out `text`, reusing a cached result when the inputs are unchanged.
    #[allow(clippy::too_many_arguments)]
    pub fn layout(
        &mut self,
        text: &str,
        shaper: &super::shaping::Shaper,
        atlas: &super::atlas::Atlas,
        font_size: Scalar,
        available_width: Scalar,
        alignment: TextAlignment,
        overflow: Overflow,
    ) -> &Layout {
        let font_size_bits = font_size.to_bits();
        let available_width_bits = available_width.to_bits();
        let atlas_len = atlas.glyphs.len();
        let atlas_ptr = atlas.glyphs.as_ptr() as usize;

        let stamp = self.clock;
        self.clock = self.clock.wrapping_add(1);

        // Compare against borrowed fields so a hit does not allocate a key.
        let cached = self.entries.iter().position(|entry| {
            let key = &entry.key;
            key.text == text
                && key.font_size_bits == font_size_bits
                && key.available_width_bits == available_width_bits
                && key.alignment == alignment
                && key.overflow == overflow
                && key.apply_kerning == shaper.apply_kerning
                && key.atlas_len == atlas_len
                && key.atlas_ptr == atlas_ptr
        });
        if let Some(index) = cached {
            self.entries[index].last_used = stamp;
            self.hits = self.hits.saturating_add(1);
            return &self.entries[index].layout;
        }

        self.misses = self.misses.saturating_add(1);
        let shaped = shaper.shape(text, atlas, font_size);
        let layout = Layout::new(
            &shaped,
            atlas,
            font_size,
            available_width,
            alignment,
            overflow,
        );

        if self.entries.len() >= self.capacity {
            if let Some((index, _)) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.last_used)
            {
                self.entries.swap_remove(index);
            }
        }

        self.entries.push(CacheEntry {
            key: CacheKey {
                text: text.to_owned(),
                font_size_bits,
                available_width_bits,
                alignment,
                overflow,
                apply_kerning: shaper.apply_kerning,
                atlas_len,
                atlas_ptr,
            },
            layout,
            last_used: stamp,
        });
        &self.entries.last().expect("entry was just pushed").layout
    }

    /// Drop every cached layout, for example after hot-reloading a font.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Cache statistics since the last [`Self::reset_stats`].
    #[must_use]
    pub fn stats(&self) -> LayoutCacheStats {
        LayoutCacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.entries.len(),
            capacity: self.capacity,
        }
    }

    /// Reset hit and miss counters without evicting layouts.
    pub fn reset_stats(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }
}

impl Default for TextLayoutCache {
    fn default() -> Self {
        Self::new(64)
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
    use crate::shaping::{ShapedGlyph, ShapedText};

    fn shaped_text(width: f32) -> ShapedText {
        ShapedText {
            glyphs: vec![ShapedGlyph {
                glyph_index: 0,
                x: 0.0,
                y: 0.0,
                advance: width,
                kerning: 0.0,
            }],
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
        let shaped = ShapedText {
            glyphs: Vec::new(),
            width: 0.0,
            height: 0.0,
            is_rtl: false,
        };
        let layout = Layout::new(
            &shaped,
            &empty_atlas(),
            14.0,
            100.0,
            TextAlignment::Left,
            Overflow::Clip,
        );
        assert_eq!(layout.glyphs.len(), 0);
        assert_eq!(layout.width, 0.0);
        assert!(!layout.truncated);
        assert!(!layout.needs_marquee);
    }

    #[test]
    fn text_that_fits_is_not_truncated() {
        let shaped = shaped_text(50.0);
        let layout = Layout::new(
            &shaped,
            &empty_atlas(),
            1.0,
            100.0,
            TextAlignment::Left,
            Overflow::Clip,
        );
        assert!(!layout.truncated);
        assert!(!layout.needs_marquee);
        assert!(layout.fits());
    }

    #[test]
    fn text_that_overflows_with_ellipsis_is_truncated() {
        let shaped = shaped_text(200.0);
        let layout = Layout::new(
            &shaped,
            &empty_atlas(),
            1.0,
            100.0,
            TextAlignment::Left,
            Overflow::Ellipsis,
        );
        assert!(layout.truncated);
        assert!(!layout.needs_marquee);
        assert!(!layout.fits());
    }

    #[test]
    fn text_that_overflows_with_marquee_scrolls() {
        let shaped = shaped_text(200.0);
        let layout = Layout::new(
            &shaped,
            &empty_atlas(),
            1.0,
            100.0,
            TextAlignment::Left,
            Overflow::Marquee,
        );
        assert!(!layout.truncated);
        assert!(layout.needs_marquee);
    }

    #[test]
    fn center_alignment_offsets_the_text() {
        let shaped = shaped_text(50.0);
        let left = Layout::new(
            &shaped,
            &empty_atlas(),
            1.0,
            100.0,
            TextAlignment::Left,
            Overflow::Clip,
        );
        let center = Layout::new(
            &shaped,
            &empty_atlas(),
            1.0,
            100.0,
            TextAlignment::Center,
            Overflow::Clip,
        );
        assert_eq!(left.glyphs[0].position.x, 0.0);
        assert!((center.glyphs[0].position.x - 25.0).abs() < 1e-6);
    }

    #[test]
    fn right_alignment_offsets_the_text() {
        let shaped = shaped_text(50.0);
        let right = Layout::new(
            &shaped,
            &empty_atlas(),
            1.0,
            100.0,
            TextAlignment::Right,
            Overflow::Clip,
        );
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
        assert_eq!(
            offset, 40.0,
            "negative dt must not move the marquee backwards"
        );
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

    #[test]
    fn a_repeated_layout_is_served_from_the_cache() {
        let mut cache = TextLayoutCache::default();
        let atlas = empty_atlas();
        let shaper = crate::shaping::Shaper::new();

        let first = cache
            .layout(
                "Now playing",
                &shaper,
                &atlas,
                14.0,
                300.0,
                TextAlignment::Left,
                Overflow::Ellipsis,
            )
            .clone();
        let second = cache
            .layout(
                "Now playing",
                &shaper,
                &atlas,
                14.0,
                300.0,
                TextAlignment::Left,
                Overflow::Ellipsis,
            )
            .clone();

        assert_eq!(first, second);
        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 1);
        assert!(stats.hit_rate() > 0.49);
    }

    #[test]
    fn a_changed_layout_input_misses_the_cache() {
        let mut cache = TextLayoutCache::new(8);
        let atlas = empty_atlas();
        let shaper = crate::shaping::Shaper::new();

        let inputs = [
            (
                "Title",
                14.0,
                300.0,
                TextAlignment::Left,
                Overflow::Ellipsis,
            ),
            (
                "Title",
                16.0,
                300.0,
                TextAlignment::Left,
                Overflow::Ellipsis,
            ),
            (
                "Title",
                14.0,
                200.0,
                TextAlignment::Left,
                Overflow::Ellipsis,
            ),
            (
                "Title",
                14.0,
                300.0,
                TextAlignment::Center,
                Overflow::Ellipsis,
            ),
            ("Title", 14.0, 300.0, TextAlignment::Left, Overflow::Clip),
            (
                "Other",
                14.0,
                300.0,
                TextAlignment::Left,
                Overflow::Ellipsis,
            ),
        ];
        for (text, size, width, alignment, overflow) in inputs {
            cache.layout(text, &shaper, &atlas, size, width, alignment, overflow);
        }
        assert_eq!(cache.stats().misses, inputs.len() as u64);
        assert_eq!(cache.stats().entries, inputs.len());
    }

    #[test]
    fn the_cache_is_bounded_and_evicts_the_least_recently_used_entry() {
        let mut cache = TextLayoutCache::new(2);
        let atlas = empty_atlas();
        let shaper = crate::shaping::Shaper::new();
        let layout = |cache: &mut TextLayoutCache, text: &str| {
            cache.layout(
                text,
                &shaper,
                &atlas,
                14.0,
                300.0,
                TextAlignment::Left,
                Overflow::Ellipsis,
            );
        };

        layout(&mut cache, "A");
        layout(&mut cache, "B");
        layout(&mut cache, "A"); // A becomes most recently used
        layout(&mut cache, "C"); // evicts B
        assert_eq!(cache.stats().entries, 2);

        layout(&mut cache, "A");
        assert_eq!(cache.stats().hits, 2);
        layout(&mut cache, "B");
        assert_eq!(cache.stats().misses, 4);
    }

    #[test]
    fn clearing_the_cache_forces_a_new_measurement() {
        let mut cache = TextLayoutCache::new(1);
        let atlas = empty_atlas();
        let shaper = crate::shaping::Shaper::new();
        let layout = |cache: &mut TextLayoutCache| {
            cache.layout(
                "A",
                &shaper,
                &atlas,
                14.0,
                300.0,
                TextAlignment::Left,
                Overflow::Ellipsis,
            );
        };
        layout(&mut cache);
        layout(&mut cache);
        cache.clear();
        layout(&mut cache);
        assert_eq!(cache.stats().misses, 2);
        assert_eq!(cache.stats().entries, 1);
    }
}
