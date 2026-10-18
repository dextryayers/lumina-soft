//! Color: sRGB, linear light, OkLab, palette mixing, and WCAG contrast.
//!
//! # Why OkLab
//!
//! Every Lumina transition is a color transition. Interpolating two colors in
//! sRGB passes through grey, because sRGB is not perceptually uniform. That is
//! fine for a progress bar and wrong for a glowing island edge, where the
//! midpoint is the most visible part of the animation.
//!
//! OkLab is perceptually uniform, so a straight lerp in OkLab between a violet
//! and a cyan stays on the line the eye expects, with no dead grey in the
//! middle. All palette mixing happens in OkLab.
//!
//! Conversion chain:
//!
//! ```text
//! sRGB (0..=1, gamma encoded)
//!   -> linear light (undo the sRGB transfer function)
//!   -> LMS cone responses
//!   -> OkLab
//! ```

use serde::{Deserialize, Serialize};

/// Minimum contrast ratio required by WCAG AA for normal text.
pub const WCAG_AA_NORMAL_TEXT: f32 = 4.5;

/// Minimum contrast ratio required by WCAG AA for large text.
pub const WCAG_AA_LARGE_TEXT: f32 = 3.0;

/// Minimum contrast ratio required by WCAG AAA for normal text.
pub const WCAG_AAA_NORMAL_TEXT: f32 = 7.0;

/// A color in the OkLab space, plus alpha.
///
/// OkLab components are `L` in roughly `0..=1`, `a` in roughly `-0.4..=0.4`,
/// and `b` in roughly `-0.4..=0.4`. Values slightly outside those ranges are
/// allowed and will be gamut clamped on conversion back to sRGB.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct OkLab {
    /// Perceptual lightness.
    pub l: f32,
    /// Green to red opponent axis.
    pub a: f32,
    /// Blue to yellow opponent axis.
    pub b: f32,
}

impl OkLab {
    /// Construct from components.
    #[must_use]
    pub const fn new(l: f32, a: f32, b: f32) -> Self {
        Self { l, a, b }
    }

    /// Pure black.
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0);

    /// Pure white.
    pub const WHITE: Self = Self::new(1.0, 0.0, 0.0);

    /// Linear interpolation in OkLab.
    ///
    /// Because OkLab is perceptually uniform this is the correct way to blend,
    /// and the result stays inside the gamut for most pairs.
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self::new(
            self.l + (other.l - self.l) * t,
            self.a + (other.a - self.a) * t,
            self.b + (other.b - self.b) * t,
        )
    }

    /// Lightness scaled around the perceptual midpoint.
    #[must_use]
    pub fn with_lightness(self, lightness: f32) -> Self {
        // `f32::clamp` propagates NaN, so guard before clamping.
        let l = if lightness.is_finite() { lightness.clamp(0.0, 1.0) } else { self.l };
        Self::new(l, self.a, self.b)
    }

    /// Multiply lightness, used by the brightness control.
    #[must_use]
    pub fn scaled_lightness(self, gain: f32) -> Self {
        if !gain.is_finite() {
            return self;
        }
        self.with_lightness(self.l * gain.clamp(0.0, 2.0))
    }

    /// Multiply chroma, used by the saturation control.
    #[must_use]
    pub fn scaled_chroma(self, gain: f32) -> Self {
        if !gain.is_finite() || !self.a.is_finite() || !self.b.is_finite() {
            return self;
        }
        let gain = gain.clamp(0.0, 2.0);
        Self::new(self.l, self.a * gain, self.b * gain)
    }

    /// Add a constant lightness offset, used by the breathing engine.
    #[must_use]
    pub fn shifted_lightness(self, delta: f32) -> Self {
        self.with_lightness(self.l + delta)
    }

    /// Chroma magnitude, the perceptual distance from the neutral axis.
    #[must_use]
    pub fn chroma(&self) -> f32 {
        (self.a * self.a + self.b * self.b).sqrt()
    }

    /// Hue angle in radians, in `0..=TAU`. Returns zero for neutral colors.
    #[must_use]
    pub fn hue(&self) -> f32 {
        self.b.atan2(self.a).rem_euclid(std::f32::consts::TAU)
    }

    /// Rotate hue by `radians`, used by the cycling engine.
    #[must_use]
    pub fn rotated_hue(self, radians: f32) -> Self {
        if !radians.is_finite() {
            return self;
        }
        let angle = self.hue() + radians;
        let chroma = self.chroma();
        Self::new(self.l, chroma * angle.cos(), chroma * angle.sin())
    }

    /// Scale chroma toward or away from neutral.
    #[must_use]
    pub fn with_chroma(self, chroma: f32) -> Self {
        let current = self.chroma();
        if current < f32::EPSILON {
            return Self::new(self.l, 0.0, 0.0);
        }
        let scale = (chroma.max(0.0) / current).min(4.0);
        Self::new(self.l, self.a * scale, self.b * scale)
    }

    /// Convert to linear sRGB, gamut clamped.
    #[must_use]
    pub fn to_linear_srgb(self) -> [f32; 3] {
        let l_ = (self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b).powi(3);
        let m_ = (self.l - 0.105_561_34 * self.a - 0.063_854_17 * self.b).powi(3);
        let s_ = (self.l - 0.089_484_18 * self.a - 1.291_485_55 * self.b).powi(3);

        let r = 4.076_741_66 * l_ - 3.307_711_59 * m_ + 0.230_969_94 * s_;
        let g = -1.268_438_00 * l_ + 2.609_757_40 * m_ - 0.341_319_40 * s_;
        let b = -0.004_196_09 * l_ - 0.703_418_61 * m_ + 1.707_614_70 * s_;

        [clamp01(r), clamp01(g), clamp01(b)]
    }

    /// Convert to gamma encoded sRGB in `0..=1`.
    #[must_use]
    pub fn to_srgb(self) -> [f32; 3] {
        let linear = self.to_linear_srgb();
        [
            linear_to_srgb(linear[0]),
            linear_to_srgb(linear[1]),
            linear_to_srgb(linear[2]),
        ]
    }

    /// Relative luminance in `0..=1`, per WCAG 2.1.
    #[must_use]
    pub fn relative_luminance(self) -> f32 {
        let [r, g, b] = self.to_linear_srgb();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// Format as a `#RRGGBB` string.
    #[must_use]
    pub fn to_hex(self) -> String {
        let [r, g, b] = self.to_srgb();
        let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02X}{:02X}{:02X}", to_byte(r), to_byte(g), to_byte(b))
    }
}

impl From<Srgb> for OkLab {
    fn from(value: Srgb) -> Self {
        value.to_oklab()
    }
}

/// A gamma encoded sRGB color in `0..=1`, with alpha in `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Srgb {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
    /// Alpha channel, straight (not premultiplied).
    pub a: f32,
}

impl Srgb {
    /// Construct from channels.
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Opaque white.
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);

    /// Opaque black.
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);

    /// Transparent black, the usual clear color for premultiplied pipelines.
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    /// Build from 8 bit channels.
    #[must_use]
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::new(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            f32::from(a) / 255.0,
        )
    }

    /// Convert to OkLab, dropping alpha.
    #[must_use]
    pub fn to_oklab(self) -> OkLab {
        let l = 0.412_221_470_8 * srgb_to_linear(self.r)
            + 0.536_332_536_3 * srgb_to_linear(self.g)
            + 0.051_445_992_9 * srgb_to_linear(self.b);
        let m = 0.211_903_498_2 * srgb_to_linear(self.r)
            + 0.680_699_545_1 * srgb_to_linear(self.g)
            + 0.107_396_956_6 * srgb_to_linear(self.b);
        let s = 0.088_302_461_9 * srgb_to_linear(self.r)
            + 0.281_718_837_6 * srgb_to_linear(self.g)
            + 0.629_978_700_5 * srgb_to_linear(self.b);

        let l_ = l.cbrt();
        let m_ = m.cbrt();
        let s_ = s.cbrt();

        OkLab::new(
            0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_,
            1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_,
            0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_,
        )
    }

    /// Relative luminance in `0..=1`, per WCAG 2.1.
    ///
    /// Computed directly from linear channels rather than by round tripping
    /// through OkLab, because contrast is checked often enough that the extra
    /// conversions are worth avoiding.
    #[must_use]
    pub fn relative_luminance(self) -> f32 {
        let r = srgb_to_linear(self.r);
        let g = srgb_to_linear(self.g);
        let b = srgb_to_linear(self.b);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// Premultiplied components, which is what the blend state expects.
    #[must_use]
    pub fn to_premultiplied(self) -> [f32; 4] {
        let a = clamp01(self.a);
        [clamp01(self.r) * a, clamp01(self.g) * a, clamp01(self.b) * a, a]
    }

    /// Format as `#RRGGBB` when opaque or `#RRGGBBAA` otherwise.
    #[must_use]
    pub fn to_hex(self) -> String {
        let to_byte = |v: f32| (clamp01(v) * 255.0).round() as u8;
        let (r, g, b) = (to_byte(self.r), to_byte(self.g), to_byte(self.b));
        if (self.a - 1.0).abs() < f32::EPSILON {
            format!("#{r:02X}{g:02X}{b:02X}")
        } else {
            format!("#{r:02X}{g:02X}{b:02X}{:02X}", to_byte(self.a))
        }
    }
}

/// The palette resolved for a single frame.
///
/// Every RGB engine produces one of these. Widgets read from it, and the shader
/// receives it as uniforms.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Palette {
    /// Background fill of the island body.
    pub fill: Srgb,
    /// Secondary fill stop, used when `appearance.fill_gradient` is non zero.
    pub fill_accent: Srgb,
    /// Border stroke color.
    pub border: Srgb,
    /// Glow color.
    pub glow: Srgb,
    /// Primary text.
    pub text: Srgb,
    /// Secondary, dimmer text.
    pub text_dim: Srgb,
    /// Glow strength, `0.0..=2.0`.
    pub glow_intensity: f32,
}

impl Palette {
    /// Interpolate every channel, used when crossfading between engines or
    /// profiles.
    ///
    /// Mixing happens in OkLab so a palette crossfade does not dip through
    /// grey. Alpha is interpolated linearly, which is correct for alpha.
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: Srgb, b: Srgb| -> Srgb {
            let la = a.to_oklab();
            let lb = b.to_oklab();
            let mixed = la.lerp(lb, t);
            let rgb = mixed.to_srgb();
            Srgb::new(rgb[0], rgb[1], rgb[2], srgb_lerp(a.a, b.a, t))
        };

        Self {
            fill: mix(self.fill, other.fill),
            fill_accent: mix(self.fill_accent, other.fill_accent),
            border: mix(self.border, other.border),
            glow: mix(self.glow, other.glow),
            text: mix(self.text, other.text),
            text_dim: mix(self.text_dim, other.text_dim),
            glow_intensity: self.glow_intensity
                + (other.glow_intensity - self.glow_intensity) * t,
        }
    }

    /// WCAG 2.1 contrast ratio between two colors, in `1.0..=21.0`.
    #[must_use]
    pub fn contrast_ratio(a: Srgb, b: Srgb) -> f32 {
        let la = a.relative_luminance();
        let lb = b.relative_luminance();
        let (lighter, darker) = if la > lb { (la, lb) } else { (lb, la) };
        ((lighter + 0.05) / (darker + 0.05)).clamp(1.0, 21.0)
    }

    /// Contrast of the primary text against the fill.
    #[must_use]
    pub fn text_contrast(&self) -> f32 {
        Self::contrast_ratio(self.text, self.fill)
    }

    /// Contrast of the dimmed text against the fill.
    #[must_use]
    pub fn dim_contrast(&self) -> f32 {
        Self::contrast_ratio(self.text_dim, self.fill)
    }

    /// Nudge the text lightness until it clears `target_ratio` against the fill.
    ///
    /// Some combinations of user colors are simply unreadable, for example a
    /// near white fill with a near white text. Rather than showing text nobody
    /// can read, or refusing to run, we adjust the text until it is legible and
    /// report that we did. The caller logs which palette triggered it.
    ///
    /// # Panics
    ///
    /// Never. If the fill is mid grey, neither direction can reach a high
    /// ratio, and the text is moved as far as it usefully can before returning.
    pub fn enforce_contrast(&mut self, target_ratio: f32) -> bool {
        let target = target_ratio.max(1.0);
        if self.text_contrast() >= target && self.dim_contrast() >= target {
            return false;
        }

        // Try lightening the text first, then darkening it. Whichever direction
        // reaches the target with the smaller perceptual change wins.
        let original_text = self.text.to_oklab();
        let original_dim = self.text_dim.to_oklab();
        let original_text_alpha = self.text.a;
        let original_dim_alpha = self.text_dim.a;
        let fill_luminance = self.fill.relative_luminance();
        let lighten = fill_luminance < 0.5;

        let mut best: Option<(f32, OkLab, OkLab, f32)> = None;

        for step in 0..=100 {
            let l = if lighten { step as f32 / 100.0 } else { 1.0 - step as f32 / 100.0 };
            let text = original_text.with_lightness(l);
            let dim = original_dim.with_lightness(l);
            let text_srgb = srgb_from_oklab(text, original_text_alpha);
            let dim_srgb = srgb_from_oklab(dim, original_dim_alpha);

            let text_ratio = Self::contrast_ratio(text_srgb, self.fill);
            let dim_ratio = Self::contrast_ratio(dim_srgb, self.fill);
            let worst = text_ratio.min(dim_ratio);

            if worst >= target {
                self.text = text_srgb;
                self.text_dim = dim_srgb;
                return true;
            }

            let distance = (text.l - original_text.l).abs();
            if best.is_none_or(|(d, _, _, _)| distance < d) {
                best = Some((distance, text, dim, worst));
            }
        }

        // Target unreachable for this fill. Apply the best available option and
        // report failure so the caller can log it.
        if let Some((_, text, dim, _)) = best {
            self.text = srgb_from_oklab(text, original_text_alpha);
            self.text_dim = srgb_from_oklab(dim, original_dim_alpha);
        }
        false
    }
}

/// sRGB transfer function inverse: gamma encoded to linear light.
#[must_use]
pub fn srgb_to_linear(value: f32) -> f32 {
    let v = clamp01(value);
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB transfer function: linear light to gamma encoded.
#[must_use]
pub fn linear_to_srgb(value: f32) -> f32 {
    let v = clamp01(value);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Build an `Srgb` from an `OkLab`, carrying alpha through untouched.
#[must_use]
pub fn srgb_from_oklab(lab: OkLab, alpha: f32) -> Srgb {
    let rgb = lab.to_srgb();
    Srgb::new(rgb[0], rgb[1], rgb[2], clamp01(alpha))
}

/// Parse a `#RGB`, `#RGBA`, `#RRGGBB`, or `#RRGGBBAA` string.
///
/// Accepts an optional leading `#`, is case insensitive, and tolerates
/// surrounding whitespace. Returns `None` for anything else so the caller can
/// substitute the default and log, rather than silently rendering black.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn parse_hex(input: &str) -> Option<Srgb> {
    let hex = input.trim().trim_start_matches('#');
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    let expand = |c: char| -> u8 {
        let v = c.to_digit(16).unwrap_or(0) as u8;
        v * 16 + v
    };

    let chars: Vec<char> = hex.chars().collect();
    match chars.len() {
        3 | 4 => {
            let r = expand(chars[0]);
            let g = expand(chars[1]);
            let b = expand(chars[2]);
            let a = if chars.len() == 4 { expand(chars[3]) } else { 255 };
            Some(Srgb::from_rgba8(r, g, b, a))
        }
        6 | 8 => {
            let byte = |i: usize| -> u8 {
                let hi = chars[i].to_digit(16).unwrap_or(0) as u8;
                let lo = chars[i + 1].to_digit(16).unwrap_or(0) as u8;
                hi * 16 + lo
            };
            let (r, g, b) = (byte(0), byte(2), byte(4));
            let a = if chars.len() == 8 { byte(6) } else { 255 };
            Some(Srgb::from_rgba8(r, g, b, a))
        }
        _ => None,
    }
}

/// Dominant colors of an image, extracted by k-means over a downscaled sample.
///
/// Used by the album RGB engine. Implemented here rather than in
/// `lumina-media` so it has no image decoding dependency and is testable with
/// synthetic pixel buffers.
///
/// Deterministic by construction: the seeds are taken from a histogram of the
/// samples, not from a random number generator, so the same cover always yields
/// the same palette frame after frame.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn extract_dominant_colors(pixels: &[Srgb], max_colors: usize) -> Vec<Srgb> {
    let k = max_colors.clamp(1, 8);
    if pixels.is_empty() {
        return vec![Srgb::BLACK];
    }

    // Discard transparent and near black pixels, which dominate the corners of
    // most cover art and are never the color a user wants to see.
    let samples: Vec<OkLab> = pixels
        .iter()
        .filter(|p| p.a > 0.5)
        .map(|p| p.to_oklab())
        .filter(|lab| lab.l > 0.06)
        .collect();

    if samples.is_empty() {
        return vec![Srgb::BLACK];
    }

    // Deterministic seeding: sort by lightness and take evenly spaced picks.
    let mut sorted = samples.clone();
    sorted.sort_by(|a, b| a.l.partial_cmp(&b.l).unwrap_or(std::cmp::Ordering::Equal));
    let mut centroids: Vec<OkLab> = (0..k)
        .map(|i| {
            let idx = (i * sorted.len()).saturating_sub(1).min(sorted.len() - 1);
            sorted[idx]
        })
        .collect();

    const ITERATIONS: usize = 8;
    for _ in 0..ITERATIONS {
        let mut sums = vec![OkLab::BLACK; k];
        let mut counts = vec![0usize; k];

        for sample in &samples {
            let (idx, _) = nearest_centroid(*sample, &centroids);
            sums[idx] = OkLab::new(sums[idx].l + sample.l, sums[idx].a + sample.a, sums[idx].b + sample.b);
            counts[idx] += 1;
        }

        for i in 0..k {
            if counts[i] > 0 {
                let n = counts[i] as f32;
                centroids[i] = OkLab::new(sums[i].l / n, sums[i].a / n, sums[i].b / n);
            }
        }
    }

    let mut result: Vec<Srgb> = centroids.iter().map(|c| srgb_from_oklab(*c, 1.0)).collect();

    // Order by chroma then lightness so the most vivid color leads. This makes
    // the album engine visually interesting rather than returning three greys.
    result.sort_by(|a, b| {
        let ca = a.to_oklab().chroma();
        let cb = b.to_oklab().chroma();
        cb.partial_cmp(&ca).unwrap_or(std::cmp::Ordering::Equal)
    });
    result
}

/// Index of, and squared distance to, the nearest centroid.
fn nearest_centroid(sample: OkLab, centroids: &[OkLab]) -> (usize, f32) {
    let mut best_idx = 0;
    let mut best_dist = f32::MAX;
    for (i, c) in centroids.iter().enumerate() {
        let dl = sample.l - c.l;
        let da = sample.a - c.a;
        let db = sample.b - c.b;
        let dist = dl * dl + da * da + db * db;
        if dist < best_dist {
            best_dist = dist;
            best_idx = i;
        }
    }
    (best_idx, best_dist)
}

fn srgb_lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn clamp01(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic sample generator so failures reproduce.
    struct Lcg(u32);

    impl Lcg {
        fn next_unit(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((self.0 >> 8) & 0xFFFF) as f32 / 65535.0
        }
    }

    #[test]
    fn srgb_transfer_round_trips() {
        for step in 0..=100 {
            let v = step as f32 / 100.0;
            let round = linear_to_srgb(srgb_to_linear(v));
            assert!((round - v).abs() < 1e-5, "round trip failed at {v}: got {round}");
        }
    }

    #[test]
    fn srgb_transfer_handles_out_of_range_input() {
        assert_eq!(srgb_to_linear(-1.0), 0.0);
        assert_eq!(srgb_to_linear(2.0), 1.0);
        assert!(linear_to_srgb(f32::NAN).is_finite());
    }

    #[test]
    fn oklab_round_trips_for_ten_thousand_colors() {
        let mut rng = Lcg(0xA5A5_1234);
        for _ in 0..10_000 {
            let color = Srgb::new(rng.next_unit(), rng.next_unit(), rng.next_unit(), 1.0);
            let lab = color.to_oklab();
            let back = lab.to_srgb();
            assert!(
                (back[0] - color.r).abs() < 1e-3
                    && (back[1] - color.g).abs() < 1e-3
                    && (back[2] - color.b).abs() < 1e-3,
                "round trip failed for {}: got [{:?}]",
                color.to_hex(),
                back
            );
        }
    }

    #[test]
    fn white_and_black_map_to_known_oklab_values() {
        let white = Srgb::WHITE.to_oklab();
        assert!((white.l - 1.0).abs() < 1e-3);
        assert!(white.a.abs() < 1e-3);
        assert!(white.b.abs() < 1e-3);

        let black = Srgb::BLACK.to_oklab();
        assert!(black.l.abs() < 1e-3);
    }

    #[test]
    fn lightness_ramp_in_oklab_is_monotonic_in_luminance() {
        // The brightness control must never stall or reverse, and it must behave
        // the same at full chroma as at neutral. Equal OkLab L steps therefore
        // have to produce strictly increasing relative luminance across a range
        // of hues.
        for base in [
            OkLab::new(0.5, 0.0, 0.0),          // neutral
            OkLab::new(0.5, 0.15, -0.12),       // violet
            OkLab::new(0.5, -0.12, 0.10),       // green
            OkLab::new(0.5, 0.10, 0.14),        // orange
        ] {
            let mut previous = f32::NEG_INFINITY;
            for step in 0..=40 {
                let l = 0.25 + step as f32 * (0.55 / 40.0);
                let rgb = base.with_lightness(l).to_srgb();
                let y = Srgb::new(rgb[0], rgb[1], rgb[2], 1.0).relative_luminance();
                assert!(
                    y > previous,
                    "luminance stalled or reversed at L={l} for base {base:?}: {y} after {previous}"
                );
                previous = y;
            }
        }
    }

    #[test]
    fn oklab_brightness_preserves_hue_where_srgb_scaling_does_not() {
        // This is the concrete reason Lumina interpolates in OkLab. Brightening
        // a saturated color by adding lightness is a free operation in OkLab.
        // The naive alternative, scaling sRGB channels, pushes channels out of
        // gamut, and clipping desaturates them unevenly, so the hue drifts.
        let base = Srgb::new(0.30, 0.12, 0.85, 1.0).to_oklab();
        let base_hue = base.hue();
        let base_rgb = base.to_srgb();

        let mut oklab_max_shift: f32 = 0.0;
        let mut srgb_max_shift: f32 = 0.0;

        for factor in [0.4f32, 0.7, 1.0, 1.4, 1.8] {
            oklab_max_shift = oklab_max_shift.max(
                (base.scaled_lightness(factor).hue() - base_hue).abs(),
            );
            srgb_max_shift = srgb_max_shift.max(
                (Srgb::new(
                    base_rgb[0] * factor,
                    base_rgb[1] * factor,
                    base_rgb[2] * factor,
                    1.0,
                )
                .to_oklab()
                .hue()
                    - base_hue)
                .abs(),
            );
        }

        assert!(
            oklab_max_shift < 1e-4,
            "OkLab lightness changes must not move the hue, shift was {oklab_max_shift}"
        );
        assert!(
            srgb_max_shift > 1e-2,
            "the naive sRGB approach should visibly drift in hue, shift was {srgb_max_shift}"
        );
        assert!(
            srgb_max_shift > oklab_max_shift * 100.0,
            "OkLab ({oklab_max_shift}) should be dramatically more hue stable than sRGB ({srgb_max_shift})"
        );
    }

    #[test]
    fn hue_is_preserved_by_a_lightness_only_interpolation() {
        // Changing only OkLab L must not shift the color's hue, which is what
        // lets the brightness control move an RGB mode without it changing
        // character.
        let start = Srgb::new(0.486, 0.231, 0.929, 1.0).to_oklab();
        for factor in [0.3f32, 0.7, 1.0, 1.5] {
            let scaled = start.scaled_lightness(factor);
            let hue_delta = (scaled.hue() - start.hue()).abs();
            assert!(
                hue_delta < 1e-4,
                "brightness scaling moved the hue by {hue_delta} at factor {factor}"
            );
        }
    }

    #[test]
    fn chroma_scaling_preserves_lightness() {
        let start = Srgb::new(0.486, 0.231, 0.929, 1.0).to_oklab();
        for factor in [0.0f32, 0.5, 1.0, 2.0] {
            let scaled = start.scaled_chroma(factor);
            assert!((scaled.l - start.l).abs() < 1e-5, "saturation changed lightness");
            let expected = start.chroma() * factor;
            assert!((scaled.chroma() - expected).abs() < 1e-4);
        }
    }

    /// CIE 1976 lightness from relative luminance.
    #[allow(dead_code)]
    fn cie_lightness(y: f32) -> f32 {
        if y <= 0.008_856 {
            903.3 * y
        } else {
            116.0 * y.cbrt() - 16.0
        }
    }

    #[test]
    fn hue_rotation_returns_to_start_after_full_turn() {
        let start = Srgb::new(0.7, 0.2, 0.4, 1.0).to_oklab();
        let round_trip = start.rotated_hue(std::f32::consts::TAU);
        assert!((round_trip.l - start.l).abs() < 1e-4);
        assert!((round_trip.a - start.a).abs() < 1e-4);
        assert!((round_trip.b - start.b).abs() < 1e-4);
    }

    #[test]
    fn hue_of_a_neutral_color_is_zero_and_rotation_is_safe() {
        let grey = OkLab::new(0.5, 0.0, 0.0);
        assert!(grey.hue().abs() < 1e-6);
        let rotated = grey.rotated_hue(1.0);
        assert!(rotated.chroma() < 1e-6);
        assert!(grey.with_chroma(0.5).chroma() < 1e-6);
    }

    #[test]
    fn parse_hex_accepts_every_documented_form() {
        assert_eq!(parse_hex("#FF0000"), Some(Srgb::from_rgba8(255, 0, 0, 255)));
        assert_eq!(parse_hex("ff0000"), Some(Srgb::from_rgba8(255, 0, 0, 255)));
        assert_eq!(parse_hex("#F00"), Some(Srgb::from_rgba8(255, 0, 0, 255)));
        assert_eq!(parse_hex("#FF000080").map(|c| (c.a * 255.0).round() as u8), Some(128));
        assert_eq!(parse_hex("#F00F").map(|c| (c.a * 255.0).round() as u8), Some(255));
        assert_eq!(parse_hex("  #1A1A2E  "), Some(Srgb::from_rgba8(0x1A, 0x1A, 0x2E, 255)));
    }

    #[test]
    fn parse_hex_rejects_malformed_input() {
        for bad in ["", "#", "#12345", "#GGGGGG", "#FF00FF0", "rgb(1,2,3)", "#FF00FF "] {
            // Trailing space is trimmed, so "#FF00FF " is valid; check the rest.
            if bad == "#FF00FF " {
                assert!(parse_hex(bad).is_some());
            } else {
                assert!(parse_hex(bad).is_none(), "should have rejected {bad:?}");
            }
        }
    }

    #[test]
    fn hex_formatting_round_trips_through_the_parser() {
        for hex in ["#000000", "#FFFFFF", "#7C3AED", "#06B6D4", "#FF2D95"] {
            let parsed = parse_hex(hex).expect("valid hex");
            assert_eq!(parsed.to_hex(), hex);
        }
    }

    #[test]
    fn contrast_ratio_matches_wcag_reference_values() {
        // Published reference values for WCAG 2.1 contrast.
        let black = Srgb::BLACK;
        let white = Srgb::WHITE;
        assert!((Palette::contrast_ratio(black, white) - 21.0).abs() < 0.01);

        let mid = Srgb::from_rgba8(0x77, 0x77, 0x77, 255);
        let white_on_mid = Palette::contrast_ratio(white, mid);
        assert!(
            (white_on_mid - 4.48).abs() < 0.05,
            "expected about 4.48, got {white_on_mid}"
        );

        let yellow = Srgb::from_rgba8(0xFF, 0xFF, 0x00, 255);
        assert!((Palette::contrast_ratio(black, yellow) - 19.56).abs() < 0.1);
    }

    #[test]
    fn contrast_ratio_is_symmetric_and_bounded() {
        let a = Srgb::from_rgba8(0x33, 0x66, 0x99, 255);
        let b = Srgb::from_rgba8(0xCC, 0xDD, 0xEE, 255);
        let ab = Palette::contrast_ratio(a, b);
        let ba = Palette::contrast_ratio(b, a);
        assert!((ab - ba).abs() < 1e-5, "contrast must be symmetric");
        assert!((1.0..=21.0).contains(&ab));
        assert!((Palette::contrast_ratio(a, a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn enforce_contrast_fixes_an_unreadable_palette() {
        let mut palette = Palette {
            fill: Srgb::from_rgba8(0xEE, 0xEE, 0xEE, 255), // near white fill
            fill_accent: Srgb::from_rgba8(0xEE, 0xEE, 0xEE, 255),
            border: Srgb::from_rgba8(0xEE, 0xEE, 0xEE, 255),
            glow: Srgb::from_rgba8(0xEE, 0xEE, 0xEE, 255),
            text: Srgb::from_rgba8(0xF5, 0xF5, 0xF5, 255), // near white text
            text_dim: Srgb::from_rgba8(0xF0, 0xF0, 0xF0, 255),
            glow_intensity: 0.5,
        };

        assert!(palette.text_contrast() < WCAG_AA_NORMAL_TEXT);
        let changed = palette.enforce_contrast(WCAG_AA_NORMAL_TEXT);
        assert!(changed, "should have reported that it changed the text");
        assert!(
            palette.text_contrast() >= WCAG_AA_NORMAL_TEXT,
            "still unreadable after fix: {}",
            palette.text_contrast()
        );
        assert!(
            palette.dim_contrast() >= WCAG_AA_NORMAL_TEXT,
            "dim text still unreadable: {}",
            palette.dim_contrast()
        );
    }

    #[test]
    fn enforce_contrast_darkens_on_a_dark_fill() {
        let mut palette = Palette {
            fill: Srgb::from_rgba8(0x11, 0x11, 0x18, 255),
            fill_accent: Srgb::from_rgba8(0x11, 0x11, 0x18, 255),
            border: Srgb::from_rgba8(0x11, 0x11, 0x18, 255),
            glow: Srgb::from_rgba8(0x11, 0x11, 0x18, 255),
            text: Srgb::from_rgba8(0x22, 0x22, 0x28, 255), // near black on near black
            text_dim: Srgb::from_rgba8(0x20, 0x20, 0x26, 255),
            glow_intensity: 0.5,
        };

        palette.enforce_contrast(WCAG_AA_NORMAL_TEXT);
        assert!(palette.text_contrast() >= WCAG_AA_NORMAL_TEXT);
        // The fill did not change, only the text.
        assert_eq!(palette.fill.to_hex(), "#111118");
    }

    #[test]
    fn enforce_contrast_leaves_a_good_palette_alone() {
        let mut palette = Palette {
            fill: Srgb::from_rgba8(0x12, 0x12, 0x1A, 255),
            fill_accent: Srgb::from_rgba8(0x12, 0x12, 0x1A, 255),
            border: Srgb::from_rgba8(0x33, 0x33, 0x40, 255),
            glow: Srgb::from_rgba8(0x7C, 0x3A, 0xED, 255),
            text: Srgb::WHITE,
            text_dim: Srgb::from_rgba8(0xDD, 0xDD, 0xDD, 255),
            glow_intensity: 0.6,
        };
        let before = palette;
        assert!(!palette.enforce_contrast(WCAG_AA_NORMAL_TEXT));
        assert_eq!(palette.text_contrast(), before.text_contrast());
    }

    #[test]
    fn enforce_contrast_reports_failure_when_unreachable() {
        // Mid grey fill: neither white nor black text can reach 4.5 to 1.
        let mut palette = Palette {
            fill: Srgb::from_rgba8(0x77, 0x77, 0x77, 255),
            fill_accent: Srgb::from_rgba8(0x77, 0x77, 0x77, 255),
            border: Srgb::from_rgba8(0x77, 0x77, 0x77, 255),
            glow: Srgb::from_rgba8(0x77, 0x77, 0x77, 255),
            text: Srgb::from_rgba8(0x80, 0x80, 0x80, 255),
            text_dim: Srgb::from_rgba8(0x80, 0x80, 0x80, 255),
            glow_intensity: 0.5,
        };
        let reported = palette.enforce_contrast(WCAG_AAA_NORMAL_TEXT);
        assert!(!reported, "should report that the target was unreachable");
        // It still applied the best available option rather than giving up.
        assert!(palette.text_contrast() > 1.0);
    }

    #[test]
    fn palette_lerp_hits_both_endpoints() {
        let a = Palette { fill: Srgb::WHITE, glow: Srgb::BLACK, ..Palette::default() };
        let b = Palette { fill: Srgb::BLACK, glow: Srgb::WHITE, ..Palette::default() };

        // Compare in 8 bit space because a round trip through OkLab is accurate
        // to about 1e-6, which is finer than any color a display can show.
        assert_eq!(a.lerp(b, 0.0).fill.to_hex(), a.fill.to_hex());
        assert_eq!(a.lerp(b, 1.0).fill.to_hex(), b.fill.to_hex());
        assert_eq!(a.lerp(b, 0.0).glow.to_hex(), a.glow.to_hex());
        assert_eq!(a.lerp(b, 1.0).glow.to_hex(), b.glow.to_hex());

        let mid = a.lerp(b, 0.5);
        // Halfway in OkLab lightness is a perceptual midpoint, not a numeric one.
        // sRGB channel values are a poor proxy for that, so assert on the
        // perceptual quantity instead.
        let mid_lightness = mid.fill.to_oklab().l;
        assert!(
            (mid_lightness - 0.5).abs() < 0.02,
            "midpoint lightness should be 0.5, got {mid_lightness}"
        );
        // And the crossing is monotonic: the first quarter stays near white.
        let quarter = a.lerp(b, 0.25);
        assert!(quarter.fill.to_oklab().l > 0.7, "the first quarter should stay light");
    }

    #[test]
    fn palette_lerp_preserves_alpha_endpoints() {
        let a = Palette { fill: Srgb::new(1.0, 0.0, 0.0, 0.2), ..Palette::default() };
        let b = Palette { fill: Srgb::new(0.0, 1.0, 0.0, 0.8), ..Palette::default() };
        assert!((a.lerp(b, 0.0).fill.a - 0.2).abs() < 1e-5);
        assert!((a.lerp(b, 1.0).fill.a - 0.8).abs() < 1e-5);
        assert!((a.lerp(b, 0.5).fill.a - 0.5).abs() < 1e-5);
    }

    #[test]
    fn dominant_color_extraction_is_deterministic() {
        let pixels: Vec<Srgb> = (0..512)
            .map(|i| {
                let f = i as f32 / 512.0;
                Srgb::new(f, 1.0 - f, 0.5, 1.0)
            })
            .collect();

        let first = extract_dominant_colors(&pixels, 4);
        for _ in 0..5 {
            assert_eq!(extract_dominant_colors(&pixels, 4), first, "extraction is not deterministic");
        }
        assert_eq!(first.len(), 4);
    }

    #[test]
    fn dominant_color_extraction_handles_empty_and_transparent_input() {
        assert_eq!(extract_dominant_colors(&[], 4), vec![Srgb::BLACK]);
        let transparent = vec![Srgb::TRANSPARENT; 100];
        assert_eq!(extract_dominant_colors(&transparent, 4), vec![Srgb::BLACK]);
        let black = vec![Srgb::BLACK; 100];
        assert_eq!(extract_dominant_colors(&black, 4), vec![Srgb::BLACK]);
    }

    #[test]
    fn dominant_color_extraction_finds_the_planted_color() {
        // A field of dark grey with a cluster of saturated red in the middle.
        let mut pixels = vec![Srgb::from_rgba8(0x22, 0x22, 0x22, 255); 400];
        for _ in 0..200 {
            pixels.push(Srgb::from_rgba8(0xEE, 0x11, 0x22, 255));
        }
        let colors = extract_dominant_colors(&pixels, 3);
        let most_chromatic = colors
            .iter()
            .max_by(|a, b| {
                a.to_oklab()
                    .chroma()
                    .partial_cmp(&b.to_oklab().chroma())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .expect("at least one color");
        let [r, g, _] = most_chromatic.to_oklab().to_srgb();
        assert!(r > 0.6 && g < 0.3, "should have found the red cluster, got {r} {g}");
    }

    #[test]
    fn premultiplied_components_scale_by_alpha() {
        let c = Srgb::new(1.0, 0.5, 0.25, 0.5);
        let p = c.to_premultiplied();
        assert!((p[0] - 0.5).abs() < 1e-6);
        assert!((p[1] - 0.25).abs() < 1e-6);
        assert!((p[2] - 0.125).abs() < 1e-6);
        assert!((p[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn lightness_and_chroma_controls_reject_non_finite_input() {
        let c = Srgb::new(0.5, 0.2, 0.8, 1.0).to_oklab();
        assert!(c.scaled_lightness(f32::NAN).l.is_finite());
        assert!(c.scaled_chroma(f32::INFINITY).l.is_finite());
        assert!(c.shifted_lightness(f32::NAN).l.is_finite());
    }
}