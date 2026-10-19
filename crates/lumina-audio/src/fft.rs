//! Hann windowed real FFT over the ring samples.
//!
//! # The packed real trick
//!
//! The input is `2N` real samples. Feeding those as `2N` complex numbers to
//! rustfft doubles the work for no reason. Instead the even samples become
//! the real half and the odd samples the imaginary half of an `N` point
//! complex transform, and the full `2N` real spectrum is untangled with
//! precomputed twiddle factors. Same result, half the multiplies.
//!
//! Derivation, with `a[n] = x[2n]`, `b[n] = x[2n+1]`, `C = FFT_N(a + j·b)`,
//! `A = FFT_N(a)`, `B = FFT_N(b)`:
//!
//! ```text
//! C[k] = A[k] + j·B[k]
//!   => A[k] = (C[k] + conj(C[(N-k) % N])) / 2
//!   => B[k] = -j·(C[k] - conj(C[(N-k) % N])) / 2
//!
//! X[m] = A[m % N] + exp(-j·2π·m / (2N)) · B[m % N]
//!   => X[2k]   = A[k] + W_N^k · B[k]              (W_N^k = exp(-j·2π·k/N))
//!   => X[2k+1] = A[k] + exp(-j·π·(2k+1)/N) · B[k]
//! ```
//!
//! Because a real input's spectrum is symmetric about Nyquist, the loop only
//! untangles `k` in `[0, N/2)`, which yields bins `0 .. N`, that is
//! [`N = window_size / 2`] usable magnitude bins. The Nyquist bin itself is
//! dropped: the band mapper only cares about the audible range well below it.
//!
//! # Why no allocation matters here
//!
//! The analysis thread runs roughly 47 times a second. Allocating the
//! scratch vector each frame would be harmless for correctness and still a
//! needless jitter source, so the plan, window, twiddles, and scratch are
//! built once in [`Analyzer::new`] and the frame path only writes into
//! preallocated boxed slices.

use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::sync::Arc;

/// The Hann window's coherent gain: a full scale sine's projected magnitude
/// is halved by the window, which the band mapper normalizes away.
const HANN_COHERENT_GAIN: f32 = 0.5;

/// A warm FFT analyzer for one window size.
pub struct Analyzer {
    /// The full real window size, always a power of two.
    window_size: usize,
    /// Half the window: the size of the complex transform, and the number of
    /// magnitude bins produced.
    half: usize,
    /// The planned `half` point complex forward transform.
    plan: Arc<dyn Fft<f32>>,
    /// Scratch rustfft asks for, allocated once.
    scratch: Box<[Complex<f32>]>,
    /// The Hann window, symmetric, one entry per real sample.
    window: Box<[f32]>,
    /// `exp(-j·2π·k/N)` for the untangle, k in `[0, N/2)`.
    twiddles: Box<[Complex<f32>]>,
    /// Packed input, reused each frame.
    packed: Box<[Complex<f32>]>,
}

impl core::fmt::Debug for Analyzer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The plan is Arc<dyn Fft>, which is not Debug, so it is described
        // by the sizes it serves.
        f.debug_struct("Analyzer")
            .field("window_size", &self.window_size)
            .field("bin_count", &self.half)
            .field("plan_scratch_len", &self.plan.get_inplace_scratch_len())
            .field("scratch_len", &self.scratch.len())
            .field("window_len", &self.window.len())
            .field("twiddles_len", &self.twiddles.len())
            .field("packed_len", &self.packed.len())
            .finish()
    }
}

impl Analyzer {
    /// Build an analyzer for a window of `window_size` real samples.
    ///
    /// # Panics
    ///
    /// Panics when `window_size` is not a power of two of at least four.
    #[must_use]
    pub fn new(window_size: usize) -> Self {
        assert!(
            window_size.is_power_of_two() && window_size >= 4,
            "fft window must be a power of two >= 4"
        );
        let half = window_size / 2;
        let mut planner = FftPlanner::<f32>::new();
        let plan: Arc<dyn Fft<f32>> = planner.plan_fft_forward(half);

        let scratch_len = plan.get_inplace_scratch_len().max(half);
        let scratch = vec![Complex::ZERO; scratch_len].into_boxed_slice();

        let window = (0..window_size)
            .map(|i| hann(i, window_size))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        // Untangle twiddles: one W_N^k = exp(-j·2π·k/N) per real bin k in
        // [0, N/2); the real spectrum is X[k] = A[k] + W_N^k·B[k] directly.
        let twiddles = (0..half)
            .map(|k| Complex::from_polar(1.0, -std::f32::consts::TAU * k as f32 / window_size as f32))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let packed = vec![Complex::ZERO; half].into_boxed_slice();

        Self {
            window_size,
            half,
            plan,
            scratch,
            window,
            twiddles,
            packed,
        }
    }

    /// The configured window size.
    #[must_use]
    pub fn window_size(&self) -> usize {
        self.window_size
    }

    /// How many magnitude bins the spectrum has: half the window, the real
    /// FFT of a real signal is symmetric about Nyquist, and the Nyquist bin
    /// itself is dropped.
    #[must_use]
    pub fn bin_count(&self) -> usize {
        self.half
    }

    /// The Hann window gain at sample `i`, exposed for tests and diagnostics.
    #[must_use]
    pub fn window_at(&self, index: usize) -> f32 {
        self.window.get(index).copied().unwrap_or(0.0)
    }

    /// Run one frame: window `samples`, FFT, write magnitudes to `out`.
    ///
    /// `samples.len()` must equal the analyzer's window size; `out` must be
    /// at least [`bin_count`](Self::bin_count) long.
    ///
    /// # Panics
    ///
    /// Panics when `samples` is shorter than the window. The engine always
    /// passes a full window because it reads through
    /// [`RingReader::read_exact`](crate::ring::RingReader::read_exact).
    pub fn process(&mut self, samples: &[f32], out_magnitudes: &mut [f32]) {
        assert!(
            samples.len() == self.window_size,
            "fft input must match the window: {} != {}",
            samples.len(),
            self.window_size
        );
        debug_assert!(self.scratch.len() >= self.plan.get_inplace_scratch_len());
        let half = self.half;

        // Pack: even samples as real parts, odd samples as imaginary parts.
        for k in 0..half {
            let even = samples[2 * k] * self.window[2 * k];
            let odd = samples[2 * k + 1] * self.window[2 * k + 1];
            self.packed[k] = Complex::new(even, odd);
        }

        self.plan
            .process_with_scratch(&mut self.packed, &mut self.scratch);

        // Untangle. For the packed real transform,
        // X[k] = A[k] + W_N^k·B[k] with
        //   A[k] = (Z[k] + conj(Z[M-k])) / 2,
        //   B[k] = -j·(Z[k] - conj(Z[M-k])) / 2,
        // one output per k in [0, M). The Nyquist bin X[M] (= Z[0].re -
        // Z[0].im) is dropped per bin_count's contract.
        for k in 0..half {
            let c_here = self.packed[k];
            let c_mirror = self.packed[(half - k) % half].conj();
            let sum = (c_here + c_mirror) * Complex::new(0.5, 0.0);
            let diff = (c_here - c_mirror) * Complex::new(0.5, 0.0);
            let b_term = Complex::new(diff.im, -diff.re);

            let bin = sum + self.twiddles[k] * b_term;
            if k < out_magnitudes.len() {
                out_magnitudes[k] = bin.norm();
            }
        }
    }
}

/// Symmetric Hann window: `0.5 - 0.5 * cos(2 pi i / (M - 1))`.
fn hann(index: usize, window_size: usize) -> f32 {
    let ratio = index as f32 / (window_size - 1) as f32;
    HANN_COHERENT_GAIN * (1.0 - (std::f32::consts::TAU * ratio).cos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// Reference DFT, slow and obviously correct, for cross checking. The
    /// analyzer always applies its Hann window, so the reference is fed the
    /// same windowed samples.
    fn reference_magnitudes(windowed: &[f32], bins: usize) -> Vec<f32> {
        (0..bins)
            .map(|k| {
                let angle = TAU * k as f32 / windowed.len() as f32;
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (n, &s) in windowed.iter().enumerate() {
                    let theta = angle * n as f32;
                    re += s * theta.cos();
                    im -= s * theta.sin();
                }
                (re * re + im * im).sqrt()
            })
            .collect()
    }

    /// Windower helper mirroring the analyzer's loop.
    fn windowed(analyzer: &Analyzer, samples: &[f32]) -> Vec<f32> {
        samples
            .iter()
            .enumerate()
            .map(|(i, &s)| s * analyzer.window_at(i))
            .collect()
    }

    fn sine(samples: usize, cycles: f32) -> Vec<f32> {
        (0..samples)
            .map(|i| (TAU * cycles * i as f32 / samples as f32).sin())
            .collect()
    }

    #[test]
    fn small_even_and_odd_deltas_reproduce_the_full_spectrum() {
        // Delta at index 0 and at odd index 1, cross checked against the
        // naive DFT: the untangle path must preserve phase information, not
        // just the even bins.
        let mut analyzer = Analyzer::new(16);
        let mut magnitudes = vec![0.0f32; 8];
        for offset in 0..4usize {
            let mut impulse = vec![0.0f32; 16];
            impulse[offset] = 1.0;
            analyzer.process(&impulse, &mut magnitudes);
            let reference = reference_magnitudes(&windowed(&analyzer, &impulse), 8);
            for (got, want) in magnitudes.iter().zip(reference.iter()) {
                assert!(
                    (got - want).abs() < 1e-4,
                    "impulse at {offset}: got {got}, want {want}"
                );
            }
        }
    }

    #[test]
    fn the_analyzer_matches_a_naive_reference_dft() {
        let size = 128;
        let mut analyzer = Analyzer::new(size);
        let samples = sine(size, 9.0); // between bins, exercises both overlaps
        let mut magnitudes = vec![0.0f32; size / 2];
        analyzer.process(&samples, &mut magnitudes);

        let reference = reference_magnitudes(&windowed(&analyzer, &samples), size / 2);
        for (k, (got, want)) in magnitudes.iter().zip(reference.iter()).enumerate() {
            assert!(
                (got - want).abs() < 0.02 * want.abs().max(1.0),
                "bin {k}: got {got}, want {want}"
            );
        }
    }

    #[test]
    fn pure_dc_lands_only_in_bin_zero() {
        let mut analyzer = Analyzer::new(256);
        let mut magnitudes = vec![0.0f32; 128];
        let ones = [1.0f32; 256];
        analyzer.process(&ones, &mut magnitudes);

        let dc = magnitudes[0];
        // Full scale DC times the Hann coherent gain: ~M/2.
        assert!(dc > 100.0, "DC bin collapsed: {dc}");
        // The symmetric window's cosine carries a 1/(N-1) mismatch versus
        // the DFT grid, whose stub lands on bin 1 at about -6 dB; real
        // sidelobes (bin 2 onward) sit below -30 dB relative.
        let worst = magnitudes.iter().skip(2).copied().fold(0.0, f32::max);
        assert!(
            worst < dc * 0.03,
            "DC leaked {worst} against {dc}"
        );
    }

    #[test]
    fn a_bin_centered_sine_peaks_in_its_bin() {
        let mut analyzer = Analyzer::new(4096);
        let mut magnitudes = vec![0.0f32; 2048];
        // 15 cycles over 4096 samples: bin 15 exactly.
        analyzer.process(&sine(4096, 15.0), &mut magnitudes);

        let peak_bin = magnitudes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map_or(usize::MAX, |(k, _)| k);
        assert_eq!(peak_bin, 15, "the sine must peak in its own bin");
        // Hann sidelobes fall fast relative to the peak.
        for (k, m) in magnitudes.iter().enumerate() {
            if k.abs_diff(15) > 16 {
                assert!(*m < magnitudes[15] * 0.01, "sidelobe too high at bin {k}");
            }
        }
    }

    #[test]
    fn the_window_is_hann_shaped_and_symmetric() {
        let analyzer = Analyzer::new(1024);
        let gain = analyzer.window_at(0);
        assert!(gain.abs() < 1e-5, "window must start at zero, got {gain}");
        let edge = analyzer.window_at(1023);
        assert!(edge.abs() < 1e-5, "window must end at zero, got {edge}");
        let peak = analyzer.window_at(512);
        assert!((peak - 1.0).abs() < 1e-5, "window must peak at one, got {peak}");
        // Symmetric window: w[i] == w[M-1-i].
        let left = analyzer.window_at(256);
        let right = analyzer.window_at(767);
        assert!((left - right).abs() < 1e-6, "window asymmetry at 256/767");
    }

    #[test]
    fn window_sizes_beyond_the_power_of_two_grid_are_rejected() {
        let result = std::panic::catch_unwind(|| Analyzer::new(3));
        assert!(result.is_err(), "missing power of two check");
        let result = std::panic::catch_unwind(|| Analyzer::new(0));
        assert!(result.is_err(), "zero size check");
    }

    /// Loose regulation test that catches O(N²) regressions in any profile.
    #[test]
    fn one_thousand_frames_of_4096_complete_quickly() {
        let mut analyzer = Analyzer::new(4096);
        let samples = sine(4096, 100.0);
        let mut magnitudes = vec![0.0f32; 2048];
        let started = std::time::Instant::now();
        for _ in 0..1000 {
            analyzer.process(&samples, &mut magnitudes);
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "1000 frames took {elapsed:?}, something is badly wrong"
        );
    }

    /// The PLAN §17.7 budget, measured properly. `#[ignore]` because a debug
    /// build cannot guarantee the release target; run with
    /// `cargo test --release -- --ignored`.
    #[test]
    #[ignore = "strict 0.035 ms/frame budget needs a release build"]
    fn the_release_frame_budget_is_met() {
        let mut analyzer = Analyzer::new(4096);
        let samples = sine(4096, 100.0);
        let mut magnitudes = vec![0.0f32; 2048];
        // Warm up so planner caches and page faults do not pollute the
        // measurement.
        for _ in 0..100 {
            analyzer.process(&samples, &mut magnitudes);
        }
        let started = std::time::Instant::now();
        for _ in 0..2000 {
            analyzer.process(&samples, &mut magnitudes);
        }
        let per_frame = started.elapsed() / 2000;
        eprintln!("probe per_frame={per_frame:?}");
        assert!(
            per_frame < std::time::Duration::from_micros(35),
            "the 4096 point frame took {per_frame:?}, budget is 35 us"
        );
    }

    #[test]
    #[ignore = "instrumentation probe"]
    fn probe_where_time_goes() {
        let mut analyzer = Analyzer::new(4096);
        let samples = sine(4096, 100.0);
        let mut magnitudes = vec![0.0f32; 2048];
        for _ in 0..500 {
            analyzer.process(&samples, &mut magnitudes);
        }

        // Pack + FFT only: stop before the untangle.
        let mut scratch = analyzer.scratch.clone();
        let started = Instant::now();
        for _ in 0..2000 {
            let half = analyzer.half;
            for k in 0..half {
                let even = samples[2 * k] * analyzer.window[2 * k];
                let odd = samples[2 * k + 1] * analyzer.window[2 * k + 1];
                analyzer.packed[k] = num_complex::Complex::new(even, odd);
            }
            analyzer
                .plan
                .process_with_scratch(&mut analyzer.packed, &mut scratch);
        }
        let fft_only = started.elapsed() / 2000;

        // Untangle only, on whatever the planner left behind.
        let started = Instant::now();
        for _ in 0..2000 {
            let half = analyzer.half;
            for k in 0..half {
                let c_here = analyzer.packed[k];
                let c_mirror = analyzer.packed[(half - k) % half].conj();
                let sum = (c_here + c_mirror) * num_complex::Complex::new(0.5, 0.0);
                let diff = (c_here - c_mirror) * num_complex::Complex::new(0.5, 0.0);
                let b_term = num_complex::Complex::new(diff.im, -diff.re);
                let bin = sum + analyzer.twiddles[k] * b_term;
                magnitudes[k] = bin.norm();
            }
        }
        let untangle_only = started.elapsed() / 2000;
        eprintln!("probe fft_only={fft_only:?} untangle={untangle_only:?}");
    }
}
