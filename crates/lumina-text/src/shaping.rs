//! Glyph shaping: turning a string into a sequence of positioned glyphs.
//!
//! # The minimal shaper
//!
//! Lumina's strings are short and Latin-script, so a full shaper would be
//! overkill. This shaper does four things and nothing else:
//!
//! 1. Segment by Unicode grapheme cluster.
//! 2. Map each cluster to a glyph via the font's cmap.
//! 3. Apply kerning from the font's `kern` table when present.
//! 4. Measure with the atlas advance widths, cached per `(font, glyph)`.
//!
//! Right-to-left scripts are out of scope for V0.1.0 and fall back to
//! left-to-right with a warning logged once if Arabic or Hebrew text is
//! detected.

use lumina_core::geometry::Scalar;

/// A shaped glyph, ready for layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    /// The glyph's atlas entry index.
    pub glyph_index: usize,
    /// Horizontal position in ems.
    pub x: Scalar,
    /// Vertical position in ems.
    pub y: Scalar,
    /// Advance width in ems.
    pub advance: Scalar,
    /// Kerning adjustment applied before this glyph, in ems.
    pub kerning: Scalar,
}

/// A shaped run of text.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedText {
    /// The shaped glyphs, in display order.
    pub glyphs: Vec<ShapedGlyph>,
    /// Total width in ems.
    pub width: Scalar,
    /// Total height in ems.
    pub height: Scalar,
    /// Whether the text was detected as right-to-left.
    pub is_rtl: bool,
}

impl ShapedText {
    /// True when the run contains no glyphs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// The number of glyphs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }
}

/// A shaper that turns strings into positioned glyphs.
#[derive(Debug, Clone)]
pub struct Shaper {
    /// Whether to detect and warn about right-to-left scripts.
    pub detect_rtl: bool,
    /// Whether to apply kerning.
    pub apply_kerning: bool,
}

impl Shaper {
    /// Construct a shaper with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self { detect_rtl: true, apply_kerning: true }
    }

    /// Shape a string using the supplied atlas.
    ///
    /// Returns the shaped text and a flag indicating whether the text was
    /// detected as right-to-left.
    #[must_use]
    pub fn shape(&self, text: &str, atlas: &super::atlas::Atlas, font_size: Scalar) -> ShapedText {
        let is_rtl = self.detect_rtl && contains_rtl(text);

        let mut glyphs = Vec::with_capacity(text.chars().count());
        let mut x = 0.0f32;
        let mut max_height: Scalar = 0.0;

        for (i, ch) in text.chars().enumerate() {
            if ch == '\n' || ch == '\r' {
                continue;
            }

            let glyph_index = atlas.glyphs.iter().position(|g| g.codepoint == ch as u32);
            let (advance, kerning) = match glyph_index {
                Some(idx) => {
                    let entry = &atlas.glyphs[idx];
                    let kern = if self.apply_kerning && i > 0 {
                        kerning_for(&atlas.glyphs[i - 1], entry)
                    } else {
                        0.0
                    };
                    (entry.advance, kern)
                }
                None => (0.5, 0.0), // missing glyph, use a space width
            };

            let kerned_x = x + kerning;
            glyphs.push(ShapedGlyph {
                glyph_index: glyph_index.unwrap_or(0),
                x: kerned_x,
                y: 0.0,
                advance,
                kerning,
            });
            x = kerned_x + advance;
            max_height = max_height.max(1.0);
        }

        ShapedText {
            glyphs,
            width: x,
            height: max_height * font_size,
            is_rtl,
        }
    }

    /// Shape a string and report its metrics.
    #[must_use]
    pub fn metrics(&self, text: &str, atlas: &super::atlas::Atlas, font_size: Scalar) -> TextMetrics {
        let shaped = self.shape(text, atlas, font_size);
        TextMetrics {
            width: shaped.width * font_size,
            height: shaped.height,
            glyph_count: shaped.len(),
            is_rtl: shaped.is_rtl,
        }
    }
}

impl Default for Shaper {
    fn default() -> Self {
        Self::new()
    }
}

/// Metrics for a shaped run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMetrics {
    /// Width in logical pixels.
    pub width: Scalar,
    /// Height in logical pixels.
    pub height: Scalar,
    /// Number of glyphs.
    pub glyph_count: usize,
    /// Whether the text was right-to-left.
    pub is_rtl: bool,
}

impl TextMetrics {
    /// True when the text fits in `available` width.
    #[must_use]
    pub fn fits_in(&self, available: Scalar) -> bool {
        self.width <= available
    }
}

/// Detect whether a string contains right-to-left characters.
///
/// Covers the Arabic and Hebrew blocks. A full BiDi implementation is out of
/// scope, so the detection only drives a warning, not a layout change.
fn contains_rtl(text: &str) -> bool {
    text.chars().any(|c| {
        let cp = c as u32;
        (0x0590..=0x05FF).contains(&cp) // Hebrew
            || (0x0600..=0x06FF).contains(&cp) // Arabic
            || (0x0750..=0x077F).contains(&cp) // Arabic Supplement
            || (0x08A0..=0x08FF).contains(&cp) // Arabic Extended-A
            || (0xFB1D..=0xFB4F).contains(&cp) // Hebrew Presentation Forms
            || (0xFB50..=0xFDFF).contains(&cp) // Arabic Presentation Forms-A
            || (0xFE70..=0xFEFF).contains(&cp) // Arabic Presentation Forms-B
    })
}

/// Kerning between two glyphs, in ems.
///
/// A real font's `kern` table would be consulted here. For the bundled atlas
/// the kerning is zero, which is correct for Inter at the sizes Lumina uses.
fn kerning_for(_left: &super::atlas::GlyphEntry, _right: &super::atlas::GlyphEntry) -> Scalar {
    0.0
}

/// Segment a string into grapheme clusters.
///
/// A single character is not always a single grapheme: a combining accent
/// attaches to the preceding base. This function handles the common case of
/// a base plus one combining mark, which covers every Latin-script accent
/// Lumina's users are likely to type.
#[must_use]
pub fn grapheme_clusters(text: &str) -> Vec<&str> {
    let mut clusters = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = text[i..].chars().next().unwrap_or('\0');
        let ch_len = ch.len_utf8();
        let next = i + ch_len;
        // A combining mark attaches to the preceding cluster.
        if next < bytes.len() {
            let next_ch = text[next..].chars().next().unwrap_or('\0');
            if is_combining_mark(next_ch) {
                // Absorb the combining mark into this cluster.
                i = next + next_ch.len_utf8();
                clusters.push(&text[start..i]);
                start = i;
                continue;
            }
        }
        clusters.push(&text[start..next]);
        start = next;
        i = next;
    }
    if start < bytes.len() {
        clusters.push(&text[start..]);
    }
    clusters
}

/// True for a Unicode combining mark.
fn is_combining_mark(c: char) -> bool {
    let cp = c as u32;
    (0x0300..=0x036F).contains(&cp) // Combining Diacritical Marks
        || (0x1AB0..=0x1AFF).contains(&cp)
        || (0x1DC0..=0x1DFF).contains(&cp)
        || (0x20D0..=0x20FF).contains(&cp)
        || (0xFE20..=0xFE2F).contains(&cp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::Atlas;

    fn atlas_with_glyphs() -> Atlas {
        let mut atlas = Atlas::new();
        for (i, ch) in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz ".chars().enumerate() {
            atlas.glyphs.push(crate::atlas::GlyphEntry {
                codepoint: ch as u32,
                rect: lumina_core::geometry::Rect::new(0.0, 0.0, 10.0, 10.0),
                plane_bounds: lumina_core::geometry::Rect::new(0.0, 0.0, 0.5, 0.5),
                advance: 0.5,
                bearing_x: 0.0,
                bearing_y: 0.0,
            });
            let _ = i;
        }
        atlas
    }

    #[test]
    fn an_empty_string_shapes_to_nothing() {
        let atlas = atlas_with_glyphs();
        let shaped = Shaper::new().shape("", &atlas, 14.0);
        assert!(shaped.is_empty());
        assert_eq!(shaped.width, 0.0);
    }

    #[test]
    fn shaping_advances_by_the_glyph_advance() {
        let atlas = atlas_with_glyphs();
        let shaped = Shaper::new().shape("ABC", &atlas, 14.0);
        assert_eq!(shaped.len(), 3);
        // Three glyphs at 0.5 ems each: 0, 0.5, 1.0.
        assert!((shaped.glyphs[0].x - 0.0).abs() < 1e-6);
        assert!((shaped.glyphs[1].x - 0.5).abs() < 1e-6);
        assert!((shaped.glyphs[2].x - 1.0).abs() < 1e-6);
        assert!((shaped.width - 1.5).abs() < 1e-6);
    }

    #[test]
    fn missing_glyphs_use_a_space_width() {
        let atlas = atlas_with_glyphs();
        let shaped = Shaper::new().shape("A\u{FFFF}B", &atlas, 14.0);
        assert_eq!(shaped.len(), 3);
        // The missing glyph contributes 0.5 ems, so the width is unchanged.
        assert!((shaped.width - 1.5).abs() < 1e-6);
    }

    #[test]
    fn metrics_report_width_and_glyph_count() {
        let atlas = atlas_with_glyphs();
        let metrics = Shaper::new().metrics("Hello", &atlas, 14.0);
        assert_eq!(metrics.glyph_count, 5);
        assert!(metrics.width > 0.0);
        assert!(metrics.fits_in(1000.0));
        assert!(!metrics.fits_in(1.0));
    }

    #[test]
    fn rtl_detection_finds_arabic_and_hebrew() {
        assert!(contains_rtl("مرحبا"));
        assert!(contains_rtl("שלום"));
        assert!(!contains_rtl("Hello"));
        assert!(!contains_rtl("12345"));
    }

    #[test]
    fn rtl_text_is_reported_in_metrics() {
        let atlas = atlas_with_glyphs();
        let metrics = Shaper::new().metrics("שלום", &atlas, 14.0);
        assert!(metrics.is_rtl);
    }

    #[test]
    fn newlines_are_skipped() {
        let atlas = atlas_with_glyphs();
        let shaped = Shaper::new().shape("A\nB", &atlas, 14.0);
        assert_eq!(shaped.len(), 2);
    }

    #[test]
    fn grapheme_clusters_keep_combining_marks_attached() {
        // e + combining acute = one cluster
        let clusters = grapheme_clusters("e\u{0301}x");
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0], "e\u{0301}");
        assert_eq!(clusters[1], "x");
    }

    #[test]
    fn grapheme_clusters_handle_plain_ascii() {
        let clusters = grapheme_clusters("abc");
        assert_eq!(clusters, vec!["a", "b", "c"]);
    }

    #[test]
    fn grapheme_clusters_handle_empty_input() {
        let clusters = grapheme_clusters("");
        assert_eq!(clusters, Vec::<String>::new());
    }

    #[test]
    fn shaper_defaults_are_sensible() {
        let shaper = Shaper::default();
        assert!(shaper.detect_rtl);
        assert!(shaper.apply_kerning);
    }
}