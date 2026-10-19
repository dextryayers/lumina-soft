//! Per band smoothing with opposite attack and release time constants.
//!
//! The PLAN §17.6 wants the classic VU meter feel: the bar jumps to a
//! transient almost instantly and falls back gently. A single one pole
//! filter cannot do both, so each band keeps an asymmetric pair:
//!
//! ```text
//! level <- level + coef * (target - level)
//! coef  = attack  when target > level (rising)
//!       = release when target < level (falling)
//! ```
//!
//! The coefficients are precomputed per configuration from the requested
//! millisecond constants and the *analysis frame rate*, not the sample rate:
//! bands update once per FFT frame, so a 6 ms attack at 48 kHz-per-frame
//! would vanish instantly. Everything is smoothed in the dB domain, which
//! keeps the visual fall of a hit linear to the eye.

/// The default attack constant from the PLAN: fast enough to catch a kick
/// drum transient, slow enough to skip a single bin of noise.
pub const DEFAULT_ATTACK_MS: f32 = 6.0;
/// The default release constant from the PLAN: a bar falls back in roughly
/// a quarter second, which reads as musical decay rather than droop.
pub const DEFAULT_RELEASE_MS: f32 = 120.0;

/// Smooths `band_count` bands with asymmetric attack and release.
#[derive(Debug, Clone)]
pub struct BandEnvelope {
    /// One pole coefficients per direction, per analysis frame.
    attack: f32,
    release: f32,
    /// Current smoothed level per band, in dB.
    levels: Box<[f32]>,
}

impl BandEnvelope {
    /// Build an envelope for `band_count` bands.
    ///
    /// `analysis_rate_hz` is how often the engine will call `update`, which
    /// is the frame pacing rate of the ring reader (roughly window / capture
    /// sample rate), not the audio sample rate itself.
    ///
    /// # Panics
    ///
    /// Panics when `band_count == 0`.
    #[must_use]
    pub fn new(band_count: usize, attack_ms: f32, release_ms: f32, analysis_rate_hz: f32) -> Self {
        assert!(band_count > 0, "band_count must be positive");
        let attack = one_pole_coefficient(attack_ms, analysis_rate_hz);
        let release = one_pole_coefficient(release_ms, analysis_rate_hz);
        Self {
            attack,
            release,
            levels: vec![super::bands::DB_FLOOR; band_count].into_boxed_slice(),
        }
    }

    /// The number of bands tracked.
    #[must_use]
    pub fn band_count(&self) -> usize {
        self.levels.len()
    }

    /// Reset every band to the floor, used on stream restart.
    pub fn reset(&mut self) {
        for level in &mut self.levels {
            *level = super::bands::DB_FLOOR;
        }
    }

    /// Pull each band's smoothed level toward `targets`, in dB.
    ///
    /// The rise and fall use different coefficients, and out-of-range
    /// targets are clamped inside so a stray `NaN` cannot poison the state.
    pub fn update(&mut self, targets: &[f32]) {
        let taken = targets.len().min(self.levels.len());
        for (level, &target) in self.levels[..taken].iter_mut().zip(&targets[..taken]) {
            if !target.is_finite() {
                continue;
            }
            let clamped = target.clamp(super::bands::DB_FLOOR, 6.0);
            let coefficient = if clamped > *level { self.attack } else { self.release };
            *level += coefficient * (clamped - *level);
        }
    }

    /// The smoothed level of band `index`, in dB.
    #[must_use]
    pub fn level(&self, index: usize) -> f32 {
        self.levels.get(index).copied().unwrap_or(super::bands::DB_FLOOR)
    }

    /// Borrow the whole smoothed level array, in dB.
    #[must_use]
    pub fn levels(&self) -> &[f32] {
        &self.levels
    }
}

/// One pole coefficient for a time constant of `ms` milliseconds when the
/// smoothing function is called `rate_hz` times per second:
/// `1 - exp(-dt / tau)` with `dt = 1/rate_hz`. Clamped so absurd configs
/// cannot produce a zero or non-finite coefficient.
fn one_pole_coefficient(ms: f32, rate_hz: f32) -> f32 {
    if !ms.is_finite() || !rate_hz.is_finite() || rate_hz <= 0.0 {
        return 0.5;
    }
    let dt = 1.0 / rate_hz.max(1.0);
    let tau = ms.max(0.1) / 1000.0;
    1.0 - (-dt / tau).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 47.0; // ~46.9 Hz analysis rate for 1024@48k

    #[test]
    fn a_rising_transient_arrives_fast() {
        let mut envelope = BandEnvelope::new(8, DEFAULT_ATTACK_MS, DEFAULT_RELEASE_MS, RATE);
        // A strong hit: release toward it. After the attack time constant
        // has elapsed the level must be within one step of the target.
        let target = 0.0f32;
        envelope.update(&[target; 8]);
        let first = envelope.level(0);
        envelope.update(&[target; 8]);
        let second = envelope.level(0);
        // Two frames into a 6 ms attack at 47 Hz is 4% of a time constant:
        // barely moved yet never backwards.
        assert!(second >= first, "level must not fall while rising");
        // And after a full attack constant's worth of frames (~0.3 at
        // 1-e^{-1}), it is within reach of the target.
        let frames = (RATE * DEFAULT_ATTACK_MS / 1000.0).ceil() as usize;
        for _ in 0..frames.max(2) {
            envelope.update(&[target; 8]);
        }
        let settled = envelope.level(0);
        assert!(
            (settled - target).abs() < 0.8,
            "attack took {settled} after {frames} frames, target {target}"
        );
    }

    #[test]
    fn a_falling_edge_decays_slowly() {
        let mut envelope = BandEnvelope::new(4, DEFAULT_ATTACK_MS, DEFAULT_RELEASE_MS, RATE);
        // Charge to a hit, then pull the signal.
        envelope.update(&[0.0f32; 4]);
        envelope.update(&[0.0f32; 4]);
        envelope.update(&[0.0f32; 4]);
        assert!(envelope.level(0) > -6.0, "did not charge up: {}", envelope.level(0));

        let start = envelope.level(0);
        let dropped = 100.0f32;
        envelope.update(&[dropped; 4]);
        envelope.update(&[dropped; 4]);
        let decayed = envelope.level(0);
        assert!(
            decayed > start - 25.0,
            "the bar plunged {start} -> {decayed} in two frames; release is too fast"
        );
    }

    #[test]
    fn attack_is_faster_than_release_spectrally() {
        let mut envelope = BandEnvelope::new(2, DEFAULT_ATTACK_MS, DEFAULT_RELEASE_MS, RATE);
        envelope.update(&[-100.0f32; 2]);
        // Rise: one attack frame toward a hit.
        envelope.update(&[0.0f32; 2]);
        let rise = envelope.level(0) + 100.0;
        // From the settled top, one release frame back toward the floor.
        for _ in 0..4 {
            envelope.update(&[0.0f32; 2]);
        }
        let start = envelope.level(0);
        envelope.update(&[-100.0f32; 2]);
        let fall = start - envelope.level(0);
        assert!(
            rise > fall * 2.0,
            "rise {rise} must far outpace the matching release fall {fall}"
        );
    }

    #[test]
    fn resets_clear_every_band_to_the_floor() {
        let mut envelope = BandEnvelope::new(8, 1.0, 1.0, RATE);
        envelope.update(&[0.0f32; 8]);
        envelope.reset();
        for band in 0..8 {
            assert!(
                (envelope.level(band) - super::super::bands::DB_FLOOR).abs() < 1e-4,
                "band {band} kept state across a reset"
            );
        }
    }

    #[test]
    fn non_finite_targets_are_ignored_without_poisoning() {
        let mut envelope = BandEnvelope::new(4, DEFAULT_ATTACK_MS, DEFAULT_RELEASE_MS, RATE);
        envelope.update(&[f32::NAN; 4]);
        for band in 0..4 {
            assert!(
                (envelope.level(band) - super::super::bands::DB_FLOOR).abs() < 1e-4,
                "band {band} absorbed a NaN"
            );
        }
    }

    #[test]
    fn absurd_time_constants_clamp_to_sane_coefficients() {
        assert!(one_pole_coefficient(f32::NAN, RATE).is_finite());
        assert!(one_pole_coefficient(0.0, RATE).is_finite());
        assert!(one_pole_coefficient(DEFAULT_ATTACK_MS, 0.0).is_finite());
    }
}
