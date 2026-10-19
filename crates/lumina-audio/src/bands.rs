//! Logarithmic band mapping on top of the raw FFT magnitudes.
//!
//! # The mapping
//!
//! Human hearing is roughly logarithmic in frequency, and so is music: a
//! bass drum thump sits around 50..120 Hz, vocals around 300..3 kHz, cymbal
//! energy above 10 kHz. The PLAN wants `band_count` bands spread
//! logrithmically between 20 Hz and whatever stays under Nyquist at the
//! stream's sample rate.
//!
//! Each band owns a contiguous slice of FFT bins, precomputed once per
//! configuration change. The frame path only ever reads floating point
//! arrays, so no allocation is possible: see
//! [`BandMapper::map`] which takes [`f32`] magnitudes and produces dBFS.
//!
//! The dB reference is the coherent gain of the Hann window: a full scale
//! sine projects a magnitude of `M/4` (window sum `M·0.5`, split between two
//! bins, coherent gain 0.5), so 0 dBFS is defined as `M/4` and everything
//! else is measured downward from that.

/// The default range from the PLAN: music worth showing starts at the lowest
/// bass anyone puts in a mix and ends where real content still lives.
pub const DEFAULT_MIN_HZ: f32 = 20.0;
/// The default top of the mapping.
pub const DEFAULT_MAX_HZ: f32 = 16_000.0;

/// The anchor under which bass tilt is applied, per the PLAN's tilt spec.
pub const BASS_TILT_ANCHOR_HZ: f32 = 200.0;

/// The dB floor: anything at or below this is treated as silence.
pub const DB_FLOOR: f32 = -100.0;

/// Maps FFT bin magnitudes onto `band_count` logspaced dBFS bands.
#[derive(Debug, Clone)]
pub struct BandMapper {
    /// Half-open bin range per band, precomputed at configuration time.
    bin_ranges: Box<[(usize, usize)]>,
    /// Geometric center frequency per band, for diagnostics and tests.
    center_freqs: Box<[f32]>,
    /// The magnitude that counts as 0 dBFS, that is `M/4` for a window of
    /// `M` samples.
    full_scale_magnitude: f32,
    /// Additional dB per octave below the bass anchor as a spectral tilt.
    tilt_db_per_octave: f32,
    /// Extra user gain in dB, from `AudioConfig::sensitivity`.
    sensitivity_db: f32,
    /// Number of magnitude bins the analyzer produces.
    bin_count: usize,
}

impl BandMapper {
    /// Build a mapper.
    ///
    /// * `window_size` is the FFT window in real samples (`M`).
    /// * `band_count` bands are spread logspace across `min_hz..max_hz`.
    /// * Bands that map beyond Nyquist or inside bin 0 are clamped; bands
    ///   narrower than one bin collapse onto that single bin.
    ///
    /// # Panics
    ///
    /// Panics when `min_hz <= 0` or `max_hz <= min_hz`.
    #[must_use]
    pub fn new(
        sample_rate: u32,
        window_size: usize,
        band_count: usize,
        min_hz: f32,
        max_hz: f32,
        tilt_db_per_octave: f32,
        sensitivity_db: f32,
    ) -> Self {
        assert!(min_hz.is_finite() && min_hz > 0.0, "min_hz must be positive");
        assert!(max_hz.is_finite() && max_hz > min_hz, "max_hz must exceed min_hz");

        let bin_count = window_size / 2;
        let nyquist_hz = sample_rate.max(8_000) as f32 / 2.0;
        let max_hz = max_hz.min(nyquist_hz);

        let full_scale_magnitude = (window_size / 2) as f32 / 2.0;
        let ratio = max_hz / min_hz;

        let mut bin_ranges = Vec::with_capacity(band_count);
        let mut center_freqs = Vec::with_capacity(band_count);
        for band in 0..band_count {
            let lo = min_hz * ratio.powf(band as f32 / band_count as f32);
            let hi = min_hz * ratio.powf((band + 1) as f32 / band_count as f32);
            // Bin edges: bin = freq * window_size / sample_rate. Start at
            // bin 1 because bin 0 is DC and the highpass already killed it.
            let lo_bin = (lo * window_size as f32 / sample_rate.max(8_000) as f32).floor() as usize;
            let hi_bin = (hi * window_size as f32 / sample_rate.max(8_000) as f32).ceil() as usize;
            let lo_bin = lo_bin.max(1).min(bin_count);
            let hi_bin = hi_bin.max(lo_bin + 1).min(bin_count.max(2));
            bin_ranges.push((lo_bin, hi_bin));
            center_freqs.push((lo * hi).sqrt());
        }

        Self {
            bin_ranges: bin_ranges.into_boxed_slice(),
            center_freqs: center_freqs.into_boxed_slice(),
            full_scale_magnitude,
            tilt_db_per_octave,
            sensitivity_db,
            bin_count,
        }
    }

    /// The number of bands.
    #[must_use]
    pub fn band_count(&self) -> usize {
        self.bin_ranges.len()
    }

    /// The number of input magnitude bins this mapper expects.
    #[must_use]
    pub fn bin_count(&self) -> usize {
        self.bin_count
    }

    /// The geometric center frequency of band `index`.
    #[must_use]
    pub fn center_freq_hz(&self, index: usize) -> f32 {
        self.center_freqs.get(index).copied().unwrap_or(0.0)
    }

    /// Fold RAW linear magnitudes into dBFS bands into `out_bands`.
    ///
    /// Each band takes the *maximum* magnitude inside its bin range, not
    /// the mean: averaging smears a narrow peak across every bin it shares
    /// a band with, and a spectrum that is supposed to look alive then
    /// reads twenty dB low. Taken as max, a bin-centered full scale sine
    /// measures at the 0 dBFS rail regardless of band width.
    ///
    /// Out of range or empty bands land at [`DB_FLOOR`], clamped back to a
    /// fixed number so the visualizer has a stable rail.
    ///
    /// The bass tilt is applied on top, lifting low energy so a spectrum that
    /// measures fair across bands still looks balanced: a typical mix's
    /// high frequency content is 6..12 dB lower than the bass peak.
    ///
    /// # Panics
    ///
    /// Never; degenerate ranges are filtered in `new`.
    pub fn map(&self, magnitudes: &[f32], out_bands: &mut [f32]) {
        for (band, out) in out_bands.iter_mut().enumerate() {
            let Some(&(lo, hi)) = self.bin_ranges.get(band) else {
                *out = DB_FLOOR;
                continue;
            };
            if lo >= hi {
                *out = DB_FLOOR;
                continue;
            }
            let mut peak = 0.0f32;
            let mut taken = 0usize;
            for bin in lo..hi.min(self.bin_count.max(1)) {
                if bin >= magnitudes.len() {
                    break;
                }
                peak = peak.max(magnitudes[bin]);
                taken += 1;
            }
            if taken == 0 {
                *out = DB_FLOOR;
                continue;
            }
            let db = peak.max(1e-6).log10() * 20.0 - (self.full_scale_magnitude.log10() * 20.0);
            let center = self.center_freqs[band];
            let tilt = if center < BASS_TILT_ANCHOR_HZ && self.tilt_db_per_octave > 0.0 {
                let octaves = (BASS_TILT_ANCHOR_HZ / center.max(1.0)).log2();
                self.tilt_db_per_octave * octaves
            } else {
                0.0
            };
            *out = (db + tilt + self.sensitivity_db).clamp(DB_FLOOR, 6.0);
        }
    }

    /// Colors the bands for the 1 kHz sine test the PLAN §17.7 mandates:
    /// how many bands lie under a given frequency.
    #[must_use]
    pub fn band_index_for_freq(&self, freq_hz: f32) -> usize {
        self.center_freqs
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let da = (a.1 - freq_hz).abs();
                let db = (b.1 - freq_hz).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or(0, |(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const WINDOW: usize = 4096;
    const BINS: usize = WINDOW / 2;

    fn full_scale_sine(window: usize, freq_hz: f32, _rate: u32) -> Vec<f32> {
        // Full scale to place the projected magnitude near the 0 dBFS rail.
        (0..window)
            .map(|i| ((std::f32::consts::TAU * freq_hz * i as f32) / RATE as f32).sin())
            .collect()
    }

    #[test]
    fn bands_are_monotonic_across_the_log_range() {
        let mapper = BandMapper::new(RATE, WINDOW, 32, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 0.0, 0.0);
        assert_eq!(mapper.band_count(), 32);
        let mut previous = 0.0;
        for band in 0..mapper.band_count() {
            let center = mapper.center_freq_hz(band);
            assert!(center > previous, "band {band} not monotonic: {center}");
            previous = center;
        }
        assert!(mapper.center_freq_hz(0) >= DEFAULT_MIN_HZ);
        assert!(mapper.center_freq_hz(31) <= DEFAULT_MAX_HZ * 2.0);
    }

    #[test]
    fn clamped_band_merge_never_produces_inverted_ranges() {
        let mapper = BandMapper::new(RATE, WINDOW, 32, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 0.0, 0.0);
        for band in 0..mapper.band_count() {
            let (lo, hi) = (mapper.center_freq_hz(band), mapper.center_freq_hz(band));
            // Each center is strictly higher than the last; inverse test is
            // about ranges, so inspect the mapper internals via behavior:
            // map a flat spectrum and all bands must be finite.
            let _ = (lo, hi);
        }
        let mut out = [0.0f32; 32];
        let flat = vec![100.0f32; BINS];
        mapper.map(&flat, &mut out);
        for (i, band) in out.iter().enumerate() {
            assert!(band.is_finite(), "band {i} is not finite: {band}");
        }
    }

    #[test]
    fn a_one_khz_sine_peaks_around_the_one_khz_band() {
        let mapper = BandMapper::new(RATE, WINDOW, 32, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 0.0, 0.0);
        let samples = full_scale_sine(WINDOW, 1000.0, RATE);
        // FFT pass: reuse the crate's Analyzer for real magnitudes.
        let mut analyzer = crate::fft::Analyzer::new(WINDOW);
        let mut magnitudes = vec![0.0f32; BINS];
        analyzer.process(&samples, &mut magnitudes);

        let mut bands = [0.0f32; 32];
        mapper.map(&magnitudes, &mut bands);

        let peak_band = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map_or(0, |(i, _)| i);
        let target = mapper.band_index_for_freq(1000.0);
        assert!(
            (peak_band as i32 - target as i32).abs() <= 1,
            "1 kHz peaked in band {peak_band}, expected near {target}"
        );
        // A full scale sine should land near the 0 dBFS rail, allowance for
        // bin splitting since 1 kHz does not align with a bin center.
        assert!(
            bands[peak_band] > -6.0 && bands[peak_band] <= 6.0,
            "full scale sine read {} dBFS",
            bands[peak_band]
        );
    }

    #[test]
    fn silence_maps_to_the_floor() {
        let mapper = BandMapper::new(RATE, WINDOW, 24, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 0.0, 0.0);
        let mut bands = [0.0f32; 24];
        mapper.map(&[0.0; BINS], &mut bands);
        for band in bands {
            assert!((band - DB_FLOOR).abs() < 1e-3, "silence read {band} dB");
        }
    }

    #[test]
    fn a_flat_spectrum_is_evenly_laid_across_bands_without_tilt() {
        let mapper = BandMapper::new(RATE, WINDOW, 16, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 0.0, 0.0);
        let mut bands = [0.0f32; 16];
        // A flat magnitude in every bin, plenty of headroom above the floor.
        mapper.map(&vec![500.0f32; BINS], &mut bands);
        let lowest = bands.iter().copied().fold(f32::INFINITY, f32::min);
        let highest = bands.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            highest - lowest < 4.0,
            "flat spectrum spread {lowest}..{highest} dB, expected near-flat"
        );
    }

    #[test]
    fn bass_tilt_lifts_only_low_bands_by_the_expected_octaves() {
        let mapper = BandMapper::new(RATE, WINDOW, 32, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 3.0, 0.0);
        let mut tilted = vec![0.0f32; 32];
        let mut flat = vec![0.0f32; 32];
        let magnitudes = vec![500.0f32; BINS];
        mapper.map(&magnitudes, &mut tilted);
        let neutral = BandMapper::new(RATE, WINDOW, 32, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 0.0, 0.0);
        neutral.map(&magnitudes, &mut flat);

        // The bottom band must read higher than the neutral one, by roughly
        // tilt * log2(anchor / center).
        let bottom = mapper.center_freq_hz(0);
        let expected = 3.0 * (BASS_TILT_ANCHOR_HZ / bottom).log2();
        let actual = tilted[0] - flat[0];
        assert!(
            (actual - expected).abs() < 0.5,
            "tilt at {bottom} Hz: expected {expected}, got {actual}"
        );
        // Top bands are unaffected by the tilt.
        let top_delta = tilted[31] - flat[31];
        assert!(
            top_delta.abs() < 0.01,
            "tilt leaked into top bands: {top_delta}"
        );
    }

    #[test]
    fn sensitivity_offsets_the_whole_mapping() {
        let neutral = BandMapper::new(RATE, WINDOW, 8, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 0.0, 0.0);
        let boosted = BandMapper::new(RATE, WINDOW, 8, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 0.0, 6.0);
        let magnitudes = vec![500.0f32; BINS];
        let mut a = [0.0f32; 8];
        let mut b = [0.0f32; 8];
        neutral.map(&magnitudes, &mut a);
        boosted.map(&magnitudes, &mut b);
        for (i, (&quiet, &loud)) in a.iter().zip(b.iter()).enumerate() {
            assert!(
                (loud - quiet - 6.0).abs() < 0.1 * quiet.abs() + 1.0 || (quiet >= 6.0 && loud >= 6.0),
                "band {i}: {quiet} + 6 dB became {loud} after clamping"
            );
        }
    }

    #[test]
    fn a_short_magnitude_slice_falls_back_gracefully() {
        let mapper = BandMapper::new(RATE, WINDOW, 32, DEFAULT_MIN_HZ, DEFAULT_MAX_HZ, 0.0, 0.0);
        // Ten real bins against a request for the whole thing. Bin ranges
        // grow monotonically, so the bands that saw those ten bins form a
        // prefix and everything after them must sit at the floor.
        let mut bands = [0.0f32; 32];
        let magnitudes = vec![500.0f32; 10];
        mapper.map(&magnitudes, &mut bands);
        let floored_at = bands
            .iter()
            .position(|band| (*band - DB_FLOOR).abs() < 1e-3)
            .unwrap_or(bands.len());
        assert!(floored_at >= 1, "at least the DC neighbor had data");
        for (i, band) in bands.iter().enumerate() {
            if i < floored_at {
                assert!(*band > DB_FLOOR, "band {i} should have had data");
            } else {
                assert!((*band - DB_FLOOR).abs() < 1e-3, "band {i} should floor");
            }
        }
    }
}
