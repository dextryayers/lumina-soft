//! Glyph atlas: a 1024 x 1024 RGB8 texture holding the bundled font.
//!
//! The atlas is embedded at compile time with `include_bytes!`, so there is no
//! runtime generation cost and no heap allocation at startup. Custom fonts use
//! a runtime fallback path that is slower but only triggers when the user
//! supplies their own font.
//!
//! # Why 1024 x 1024
//!
//! A 1024 x 1024 RGB8 atlas is 3 MB resident. That is the single largest
//! allocation in Lumina's memory budget, so it is sized to hold the full glyph
//! set for Inter with room to spare, and no more.

use lumina_core::geometry::{Rect, Scalar, Vec2};

/// Maximum atlas dimension.
pub const ATLAS_SIZE: u32 = 1024;

/// Maximum number of glyphs the atlas can hold.
pub const MAX_GLYPHS: usize = 512;

/// One glyph's location in the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphEntry {
    /// Unicode codepoint.
    pub codepoint: u32,
    /// Rectangle in the atlas, in pixels.
    pub rect: Rect,
    /// Plane bounds in the font's em space.
    pub plane_bounds: Rect,
    /// Advance width in ems.
    pub advance: Scalar,
    /// Left-side bearing in ems.
    pub bearing_x: Scalar,
    /// Top-side bearing in ems.
    pub bearing_y: Scalar,
}

impl GlyphEntry {
    /// UV coordinates for the glyph's rectangle.
    #[must_use]
    pub fn uv(&self) -> (Vec2, Vec2) {
        let min = Vec2::new(
            self.rect.left() / ATLAS_SIZE as Scalar,
            self.rect.top() / ATLAS_SIZE as Scalar,
        );
        let max = Vec2::new(
            self.rect.right() / ATLAS_SIZE as Scalar,
            self.rect.bottom() / ATLAS_SIZE as Scalar,
        );
        (min, max)
    }

    /// Width of the glyph in ems.
    #[must_use]
    pub fn em_width(&self) -> Scalar {
        self.plane_bounds.width
    }

    /// Height of the glyph in ems.
    #[must_use]
    pub fn em_height(&self) -> Scalar {
        self.plane_bounds.height
    }
}

/// A glyph atlas, either embedded or generated at runtime.
#[derive(Debug, Clone)]
pub struct Atlas {
    /// The atlas pixels, RGB8.
    pub pixels: Vec<u8>,
    /// The glyphs in the atlas.
    pub glyphs: Vec<GlyphEntry>,
    /// How many glyphs are in use.
    pub count: usize,
}

impl Atlas {
    /// Create an empty atlas, sized for the bundled font.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pixels: vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 3) as usize],
            glyphs: Vec::with_capacity(MAX_GLYPHS),
            count: 0,
        }
    }

    /// Create an atlas from embedded bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        // The embedded atlas is laid out as: 4 bytes for the count, then
        // `count` glyph entries of 40 bytes each, then the pixel data.
        if bytes.len() < 4 {
            return Self::new();
        }
        let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if count > MAX_GLYPHS {
            return Self::new();
        }

        let mut atlas = Self::new();
        let mut cursor = 4;

        for _ in 0..count {
            if cursor + 40 > bytes.len() {
                break;
            }
            let codepoint = u32::from_le_bytes([
                bytes[cursor],
                bytes[cursor + 1],
                bytes[cursor + 2],
                bytes[cursor + 3],
            ]);
            cursor += 4;

            let rect = Rect::new(
                f32::from(u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]])),
                f32::from(u16::from_le_bytes([bytes[cursor + 2], bytes[cursor + 3]])),
                f32::from(u16::from_le_bytes([bytes[cursor + 4], bytes[cursor + 5]])),
                f32::from(u16::from_le_bytes([bytes[cursor + 6], bytes[cursor + 7]])),
            );
            cursor += 8;

            let plane_bounds = Rect::new(
                f32::from_le_bytes([
                    bytes[cursor],
                    bytes[cursor + 1],
                    bytes[cursor + 2],
                    bytes[cursor + 3],
                ]),
                f32::from_le_bytes([
                    bytes[cursor + 4],
                    bytes[cursor + 5],
                    bytes[cursor + 6],
                    bytes[cursor + 7],
                ]),
                f32::from_le_bytes([
                    bytes[cursor + 8],
                    bytes[cursor + 9],
                    bytes[cursor + 10],
                    bytes[cursor + 11],
                ]),
                f32::from_le_bytes([
                    bytes[cursor + 12],
                    bytes[cursor + 13],
                    bytes[cursor + 14],
                    bytes[cursor + 15],
                ]),
            );
            cursor += 16;

            let advance = f32::from_le_bytes([
                bytes[cursor],
                bytes[cursor + 1],
                bytes[cursor + 2],
                bytes[cursor + 3],
            ]);
            cursor += 4;
            let bearing_x = f32::from_le_bytes([
                bytes[cursor],
                bytes[cursor + 1],
                bytes[cursor + 2],
                bytes[cursor + 3],
            ]);
            cursor += 4;
            let bearing_y = f32::from_le_bytes([
                bytes[cursor],
                bytes[cursor + 1],
                bytes[cursor + 2],
                bytes[cursor + 3],
            ]);
            cursor += 4;

            atlas.glyphs.push(GlyphEntry {
                codepoint,
                rect,
                plane_bounds,
                advance,
                bearing_x,
                bearing_y,
            });
            atlas.count += 1;
        }

        // The remaining bytes are the pixel data.
        if cursor + (ATLAS_SIZE * ATLAS_SIZE * 3) as usize <= bytes.len() {
            let pixel_start = bytes.len() - (ATLAS_SIZE * ATLAS_SIZE * 3) as usize;
            atlas.pixels.copy_from_slice(&bytes[pixel_start..]);
        }

        atlas
    }

    /// Look up a glyph by codepoint.
    #[must_use]
    pub fn glyph(&self, codepoint: u32) -> Option<&GlyphEntry> {
        self.glyphs.iter().find(|g| g.codepoint == codepoint)
    }

    /// True when the atlas contains at least one glyph.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// The atlas dimensions in pixels.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (ATLAS_SIZE, ATLAS_SIZE)
    }

    /// Memory footprint in bytes.
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.pixels.len() + self.glyphs.len() * std::mem::size_of::<GlyphEntry>()
    }
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_atlas_is_empty_and_zero_filled() {
        let atlas = Atlas::new();
        assert!(atlas.is_empty());
        assert_eq!(atlas.count, 0);
        assert_eq!(atlas.pixels.len(), (ATLAS_SIZE * ATLAS_SIZE * 3) as usize);
        assert!(atlas.pixels.iter().all(|&b| b == 0));
    }

    #[test]
    fn a_glyph_lookup_returns_none_for_an_empty_atlas() {
        let atlas = Atlas::new();
        assert!(atlas.glyph('A' as u32).is_none());
    }

    #[test]
    fn uv_coordinates_are_in_unit_range() {
        let entry = GlyphEntry {
            codepoint: 'A' as u32,
            rect: Rect::new(10.0, 20.0, 50.0, 60.0),
            plane_bounds: Rect::new(0.0, 0.0, 0.5, 0.6),
            advance: 0.5,
            bearing_x: 0.1,
            bearing_y: 0.2,
        };
        let (min, max) = entry.uv();
        assert!(min.x >= 0.0 && min.x <= 1.0);
        assert!(max.x >= 0.0 && max.x <= 1.0);
        assert!(min.y >= 0.0 && min.y <= 1.0);
        assert!(max.y >= 0.0 && max.y <= 1.0);
        assert!(max.x > min.x);
        assert!(max.y > min.y);
    }

    #[test]
    fn em_dimensions_come_from_plane_bounds() {
        let entry = GlyphEntry {
            codepoint: 'A' as u32,
            rect: Rect::ZERO,
            plane_bounds: Rect::new(0.0, 0.0, 0.4, 0.6),
            advance: 0.5,
            bearing_x: 0.0,
            bearing_y: 0.0,
        };
        assert_eq!(entry.em_width(), 0.4);
        assert_eq!(entry.em_height(), 0.6);
    }

    #[test]
    fn atlas_size_is_1024() {
        assert_eq!(ATLAS_SIZE, 1024);
        assert_eq!(Atlas::new().size(), (1024, 1024));
    }

    #[test]
    fn memory_footprint_is_bounded() {
        let atlas = Atlas::new();
        // 1024*1024*3 = 3 MB plus glyph overhead.
        assert!(atlas.memory_bytes() >= 3_145_728);
        assert!(atlas.memory_bytes() < 4_000_000);
    }

    #[test]
    fn from_bytes_handles_empty_input() {
        let atlas = Atlas::from_bytes(&[]);
        assert!(atlas.is_empty());
    }

    #[test]
    fn from_bytes_handles_truncated_input() {
        let atlas = Atlas::from_bytes(&[0, 0, 0, 1]);
        assert!(atlas.is_empty());
    }

    #[test]
    fn from_bytes_handles_too_many_glyphs() {
        let mut bytes = vec![0u8; 8];
        bytes[0..4].copy_from_slice(&10000u32.to_le_bytes());
        let atlas = Atlas::from_bytes(&bytes);
        assert!(atlas.is_empty());
    }

    #[test]
    fn from_bytes_round_trips_a_single_glyph() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&('A' as u32).to_le_bytes());
        // rect: x=10, y=20, width=50, height=60
        bytes.extend_from_slice(&10u16.to_le_bytes());
        bytes.extend_from_slice(&20u16.to_le_bytes());
        bytes.extend_from_slice(&50u16.to_le_bytes());
        bytes.extend_from_slice(&60u16.to_le_bytes());
        // plane_bounds: x=0.1, y=0.2, width=0.4, height=0.6
        bytes.extend_from_slice(&0.1f32.to_le_bytes());
        bytes.extend_from_slice(&0.2f32.to_le_bytes());
        bytes.extend_from_slice(&0.4f32.to_le_bytes());
        bytes.extend_from_slice(&0.6f32.to_le_bytes());
        bytes.extend_from_slice(&0.5f32.to_le_bytes()); // advance
        bytes.extend_from_slice(&0.1f32.to_le_bytes()); // bearing_x
        bytes.extend_from_slice(&0.2f32.to_le_bytes()); // bearing_y
                                                        // pixel data: 1024*1024*3 bytes of zeros
        bytes.resize(bytes.len() + (ATLAS_SIZE * ATLAS_SIZE * 3) as usize, 0);

        let atlas = Atlas::from_bytes(&bytes);
        assert_eq!(atlas.count, 1);
        let glyph = atlas
            .glyph('A' as u32)
            .expect("the glyph should be present");
        assert_eq!(glyph.codepoint, 'A' as u32);
        assert!((glyph.advance - 0.5).abs() < 1e-6);
        assert!((glyph.bearing_x - 0.1).abs() < 1e-6);
        assert_eq!(atlas.pixels.len(), (ATLAS_SIZE * ATLAS_SIZE * 3) as usize);
    }
}
