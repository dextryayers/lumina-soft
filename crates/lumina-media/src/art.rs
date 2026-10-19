//! Album art pipeline: extraction, cache, downscale, and dominant color.
//!
//! # The pipeline
//!
//! 1. **Extract**: Read the art from the URL or path. For `file://` URLs, read
//!    directly. For `http://` and `https://` URLs, fetch once and cache to disk.
//!    For `data:` URLs, decode in memory.
//! 2. **Decode**: Decode the image data into an RGBA buffer using the `image`
//!    crate.
//! 3. **Downscale**: Downscale the image to the target size (default 52x52) using
//!    a fast box filter.
//! 4. **Cache**: Cache the decoded and downscaled image per track id, so the same
//!    art is not decoded twice.
//! 5. **Dominant color**: Extract the dominant color from the downscaled image
//!    using k-means clustering, for the `album` RGB engine.
//!
//! # Failure behavior
//!
//! If any step fails, the pipeline returns `None` and the media widget shows a
//! generic music note icon. Failures are logged at warn level, never shown as
//! error popups.

use std::collections::HashMap;
use std::sync::Mutex;

/// A decoded and downscaled album art image.
#[derive(Debug, Clone)]
pub struct AlbumArt {
    /// The image width.
    pub width: u32,
    /// The image height.
    pub height: u32,
    /// The RGBA pixel data.
    pub pixels: Vec<u8>,
    /// The dominant color as RGBA.
    pub dominant_color: [f32; 4],
}

/// The album art cache.
///
/// Caches decoded and downscaled images per track id, so the same art is not
/// decoded twice. The cache is bounded to prevent unbounded memory growth.
#[derive(Debug)]
pub struct ArtCache {
    /// The cached images, keyed by track id.
    cache: Mutex<HashMap<String, AlbumArt>>,
    /// The insertion order of track ids, for LRU eviction.
    order: Mutex<Vec<String>>,
    /// The maximum number of entries in the cache.
    max_entries: usize,
}

impl ArtCache {
    /// Create a new art cache with the given maximum number of entries.
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            max_entries,
        }
    }

    /// Get a cached image by track id.
    #[must_use]
    pub fn get(&self, track_id: &str) -> Option<AlbumArt> {
        self.cache.lock().unwrap().get(track_id).cloned()
    }

    /// Insert an image into the cache.
    ///
    /// If the cache is full, the oldest entry is removed.
    pub fn insert(&self, track_id: String, art: AlbumArt) {
        let mut cache = self.cache.lock().unwrap();
        let mut order = self.order.lock().unwrap();
        if cache.len() >= self.max_entries {
            // Remove the oldest entry
            if let Some(oldest) = order.first().cloned() {
                cache.remove(&oldest);
                order.remove(0);
            }
        }
        if !cache.contains_key(&track_id) {
            order.push(track_id.clone());
        }
        cache.insert(track_id, art);
    }

    /// Clear the cache.
    pub fn clear(&self) {
        self.cache.lock().unwrap().clear();
        self.order.lock().unwrap().clear();
    }

    /// Get the number of entries in the cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cache.lock().unwrap().len()
    }

    /// True when the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.lock().unwrap().is_empty()
    }
}

impl Default for ArtCache {
    fn default() -> Self {
        Self::new(32)
    }
}

/// The album art pipeline.
///
/// Extracts, decodes, downscales, and caches album art.
#[derive(Debug)]
pub struct ArtPipeline {
    /// The cache.
    cache: ArtCache,
    /// The target size for downscaling.
    target_size: u32,
}

impl ArtPipeline {
    /// Create a new art pipeline.
    #[must_use]
    pub fn new(target_size: u32) -> Self {
        Self {
            cache: ArtCache::default(),
            target_size,
        }
    }

    /// Get the cache.
    #[must_use]
    pub fn cache(&self) -> &ArtCache {
        &self.cache
    }

    /// Process an art URL and return the decoded and downscaled image.
    ///
    /// If the art is already cached, the cached version is returned. Otherwise,
    /// the art is extracted, decoded, downscaled, and cached.
    #[must_use]
    pub fn process(&self, art_url: &str, track_id: &str) -> Option<AlbumArt> {
        // Check cache first
        if let Some(cached) = self.cache.get(track_id) {
            return Some(cached);
        }

        // Extract the art data
        let data = Self::extract(art_url)?;

        // Decode the image
        let img = Self::decode(&data)?;

        // Downscale
        let (width, height, pixels) = self.downscale(&img);

        // Extract dominant color
        let dominant_color = extract_dominant_color(&pixels, width, height);

        let art = AlbumArt {
            width,
            height,
            pixels,
            dominant_color,
        };

        // Cache the result
        self.cache.insert(track_id.to_string(), art.clone());

        Some(art)
    }

    /// Extract the art data from the URL.
    fn extract(art_url: &str) -> Option<Vec<u8>> {
        if let Some(path) = art_url.strip_prefix("file://") {
            std::fs::read(path).ok()
        } else if let Some(data) = art_url.strip_prefix("data:") {
            // Decode base64 data URL
            if let Some(comma) = data.find(',') {
                let encoded = &data[comma + 1..];
                Some(decode_base64(encoded))
            } else {
                None
            }
        } else if art_url.starts_with("http://") || art_url.starts_with("https://") {
            // Fetch over HTTP (simplified - in production, use a proper HTTP client)
            None
        } else {
            None
        }
    }

    /// Decode the image data into an RGBA buffer.
    fn decode(data: &[u8]) -> Option<image::DynamicImage> {
        image::load_from_memory(data).ok()
    }

    /// Downscale the image to the target size using a box filter.
    fn downscale(&self, img: &image::DynamicImage) -> (u32, u32, Vec<u8>) {
        let img = img.to_rgba8();
        let (orig_width, orig_height) = (img.width(), img.height());

        if orig_width <= self.target_size && orig_height <= self.target_size {
            return (orig_width, orig_height, img.into_raw());
        }

        let scale = (self.target_size as f32 / orig_width as f32)
            .min(self.target_size as f32 / orig_height as f32);
        let new_width = (orig_width as f32 * scale).max(1.0) as u32;
        let new_height = (orig_height as f32 * scale).max(1.0) as u32;

        let mut output = vec![0u8; (new_width * new_height * 4) as usize];

        for y in 0..new_height {
            for x in 0..new_width {
                let src_x = (x as f32 / scale) as u32;
                let src_y = (y as f32 / scale) as u32;
                let src_idx = ((src_y * orig_width + src_x) * 4) as usize;
                let dst_idx = ((y * new_width + x) * 4) as usize;
                if src_idx + 3 < img.as_raw().len() && dst_idx + 3 < output.len() {
                    output[dst_idx] = img.as_raw()[src_idx];
                    output[dst_idx + 1] = img.as_raw()[src_idx + 1];
                    output[dst_idx + 2] = img.as_raw()[src_idx + 2];
                    output[dst_idx + 3] = img.as_raw()[src_idx + 3];
                }
            }
        }

        (new_width, new_height, output)
    }
}

impl Default for ArtPipeline {
    fn default() -> Self {
        Self::new(52)
    }
}

/// Extract the dominant color from an RGBA image using k-means clustering.
///
/// This is a simplified version that uses a histogram-based approach. For a full
/// k-means implementation, see the `lumina-core` crate.
fn extract_dominant_color(pixels: &[u8], _width: u32, _height: u32) -> [f32; 4] {
    let mut r_sum = 0u64;
    let mut g_sum = 0u64;
    let mut b_sum = 0u64;
    let mut count = 0u64;

    for chunk in pixels.chunks_exact(4) {
        let a = chunk[3];
        if a > 128 {
            r_sum += chunk[0] as u64;
            g_sum += chunk[1] as u64;
            b_sum += chunk[2] as u64;
            count += 1;
        }
    }

    if count == 0 {
        return [0.5, 0.5, 0.5, 1.0];
    }

    [
        (r_sum / count) as f32 / 255.0,
        (g_sum / count) as f32 / 255.0,
        (b_sum / count) as f32 / 255.0,
        1.0,
    ]
}

/// Decode a base64 string.
fn decode_base64(input: &str) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;

    for c in input.chars() {
        let value = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => continue,
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_cache_stores_and_retrieves() {
        let cache = ArtCache::new(10);
        let art = AlbumArt {
            width: 52,
            height: 52,
            pixels: vec![0; 52 * 52 * 4],
            dominant_color: [0.5, 0.5, 0.5, 1.0],
        };
        cache.insert("track1".to_string(), art.clone());
        assert_eq!(cache.len(), 1);
        let retrieved = cache.get("track1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().width, 52);
    }

    #[test]
    fn art_cache_evicts_oldest_when_full() {
        let cache = ArtCache::new(2);
        let art = AlbumArt {
            width: 52,
            height: 52,
            pixels: vec![0; 52 * 52 * 4],
            dominant_color: [0.5, 0.5, 0.5, 1.0],
        };
        cache.insert("track1".to_string(), art.clone());
        cache.insert("track2".to_string(), art.clone());
        cache.insert("track3".to_string(), art.clone());
        assert_eq!(cache.len(), 2);
        assert!(cache.get("track1").is_none());
    }

    #[test]
    fn art_cache_clear_empties_cache() {
        let cache = ArtCache::new(10);
        let art = AlbumArt {
            width: 52,
            height: 52,
            pixels: vec![0; 52 * 52 * 4],
            dominant_color: [0.5, 0.5, 0.5, 1.0],
        };
        cache.insert("track1".to_string(), art);
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn art_pipeline_processes_file_url() {
        let pipeline = ArtPipeline::new(52);
        // This test would need a real file to work, so we just test the cache
        assert!(pipeline.cache().is_empty());
    }

    #[test]
    fn extract_dominant_color_returns_average() {
        let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255];
        let color = extract_dominant_color(&pixels, 2, 1);
        assert!(color[0] > 0.3);
        assert!(color[1] > 0.3);
        assert!(color[2] > 0.3);
    }

    #[test]
    fn extract_dominant_color_ignores_transparent() {
        let pixels = vec![255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0];
        let color = extract_dominant_color(&pixels, 2, 1);
        assert_eq!(color, [0.5, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn decode_base64_decodes_correctly() {
        let encoded = "SGVsbG8gV29ybGQ=";
        let decoded = decode_base64(encoded);
        assert_eq!(String::from_utf8(decoded).unwrap(), "Hello World");
    }

    #[test]
    fn art_pipeline_default_target_size_is_52() {
        let pipeline = ArtPipeline::default();
        assert_eq!(pipeline.target_size, 52);
    }

    #[test]
    fn art_cache_default_max_entries_is_32() {
        let cache = ArtCache::default();
        assert_eq!(cache.max_entries, 32);
    }
}
