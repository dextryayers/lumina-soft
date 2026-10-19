//! Dynamics: the silence gate and the rolling normalizer.
//!
//! These are the two devices that keep the visualizer honest when the stream
//! does something ordinary but visually meaningless:
//!
//! * A desktop that has been playing nothing for ten minutes should show()
//!   bars, not jittering noise-floor triangles. That is the
//!   [`SilenceGate`], with a 200 ms ramp both ways so gating itself never
//!   pops.
//! * A quiet jazz mix and a loud EDM master should both look alive, without
//!   the bar for one living at 10% and the other saturated. That is the
//!   [`RollingNormalizer`], which tracks a slowly falling rolling peak over
//!   the last few seconds and shifts the whole frame so the loudest recent
//!   content sits near a target rail.

use crate::bands::DB_FLOOR;

/// The PLAN §17 default silence threshold: -60 dBFS RMS.
pub const DEFAULT_SILENCE_THRESHOLD_DB: f32 = -60.0;
/// The leaky RMS average follows the stream with this time constant: fast
/// enough that real dead air reads as dead, slow enough that a hi-hat does
/// not blip the measure.
pub const SILENCE_METER_MS: f32 = 20.0;
/// How much continuous quiet it takes to call a stream silent: the PLAN
/// §17 default, half a second of sub-threshold RMS.
pub const SILENCE_WINDOW_MS: f32 = 500.0;
/// The un-gate ramp: shorter than the gate ramp, hearing silence fade in
/// slowly feels broken while a fade out should be gradual.
pub const GATE_RAMP_MS: f32 = 200.0;
/// The rolling normalizer's target rail, measured as the loudest band.
pub const NORMALIZER_TARGET_DB: f32 = -3.0;
/// How far the normalizer will reach in either direction: beyond +-12 dB the
/// mix is what it is, boosting further distorts the relative balance the
/// user actually picked the track for.
pub const NORMALIZER_MAX_GAIN_DB: f32 = 12.0;
/// The rolling peak's fall time: a track that just got quieter should look
/// quieter, but not for the first fraction of a second.
pub const NORMALIZER_DECAY_S: f32 = 3.0;

/// A leaky RMS detector with a configured threshold, in linear amplitude.
///
/// Gating is a hold timer, not an instant read of the leaky average: the
/// stream counts as silent only after the tracked RMS has stayed below the
/// threshold for [`SILENCE_WINDOW_MS`] of wall time. The leak average is
/// what keeps the *measure* from spiking on transients; the hold counter is
/// what keeps a 40 ms fade between songs from flickering the visualizer.
#[derive(Debug, Clone)]
pub struct SilenceGate {
    /// Per sample leak factor for the squared amplitude integrator.
    leak: f32,
    /// The running squared RMS.
    square_mean: f32,
    /// Linear amplitude threshold.
    threshold: f32,
    /// How many samples the RMS has been below the threshold, in a row.
    quiet_samples: u32,
    /// Samples per second, for turning samples into wall time.
    rate: f32,
}

impl SilenceGate {
    /// Build a gate for a stream at the given rate, gating below
    /// `threshold_db` (dBFS RMS) after half a second of quiet.
    #[must_use]
    pub fn new(sample_rate: u32, threshold_db: f32) -> Self {
        let rate = sample_rate.max(8_000) as f32;
        // Time constant of the meter itself; the hold counter below is
        // what the listen window buys.
        let tau = SILENCE_METER_MS / 1000.0;
        let leak = (-1.0 / (rate * tau)).exp();
        // Linear amplitude from dBFS: 10^(db/20). The exp2 shortcut some
        // calculators reach for would silently mean powers of two.
        let threshold = 10f32.powf(threshold_db.clamp(DB_FLOOR, 6.0) / 20.0);
        Self {
            leak,
            square_mean: 0.0,
            threshold,
            quiet_samples: 0,
            rate,
        }
    }

    /// How much silence in a row it takes to count as silent.
    fn hold_samples(&self) -> f32 {
        self.rate * (SILENCE_WINDOW_MS / 1000.0).max(0.01)
    }

    /// Feed one sample.
    pub fn detect(&mut self, sample: f32) {
        self.square_mean += (1.0 - self.leak) * (sample * sample - self.square_mean);
        if self.square_mean.sqrt() < self.threshold {
            self.quiet_samples += 1;
        } else {
            self.quiet_samples = 0;
        }
    }

    /// Feed a whole buffer, the common path.
    pub fn detect_slice(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.detect(sample);
        }
    }

    /// Current linear RMS the detector is holding.
    #[must_use]
    pub fn rms(&self) -> f32 {
        self.square_mean.sqrt()
    }

    /// Threshold in linear amplitude.
    #[must_use]
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// True when the stream has been quiet long enough to gate.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.quiet_samples as f32 >= self.hold_samples()
    }

    /// Clear the integrator and the hold counter, on stream restart.
    pub fn reset(&mut self) {
        self.square_mean = 0.0;
        self.quiet_samples = 0;
    }

    /// The hold counter, exposed for engine wiring tests.
    #[must_use]
    pub fn quiet_samples(&self) -> u32 {
        self.quiet_samples
    }
}

/// Smooths the on/off gate transition over 200 ms of analysis frames.
///
/// The engine samples `step` once per frame and multiplies the rendered
/// bands by the returned factor, so the gate itself never allocates and
/// never touches the sample stream.
#[derive(Debug, Clone)]
pub struct GateSmoothing {
    /// Current multiplier in `[0, 1]`.
    ramp: f32,
    /// Whether the gate considers the stream silent.
    silent: bool,
    /// Coefficient to lose the signal (silently decay the bars).
    ratio_down: f32,
    /// Coefficient to restore the signal when sound comes back.
    ratio_up: f32,
}

impl GateSmoothing {
    /// Build smoothing for an analysis frame rate.
    #[must_use]
    pub fn new(analysis_rate_hz: f32) -> Self {
        let rate = analysis_rate_hz.max(1.0);
        Self {
            ramp: 0.0,
            silent: true,
            ratio_down: one_frame_coefficient(GATE_RAMP_MS, rate),
            ratio_up: one_frame_coefficient(GATE_RAMP_MS / 4.0, rate),
        }
    }

    /// Advance one frame given the gate's verdict. Returns the multiplier
    /// to apply to the band output.
    pub fn step(&mut self, is_silent: bool) -> f32 {
        self.silent = is_silent;
        if is_silent {
            self.ramp -= self.ratio_down * self.ramp;
        } else {
            self.ramp += self.ratio_up * (1.0 - self.ramp);
        }
        self.ramp.clamp(0.0, 1.0)
    }

    /// The instant multiplier, without stepping.
    #[must_use]
    pub fn factor(&self) -> f32 {
        self.ramp
    }

    /// Whether the last frame's verdict said silence.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.silent
    }

    /// Snap to a state, on stream restart: start ungated, and let the
    /// detector re-learn.
    pub fn reset(&mut self) {
        self.ramp = 1.0;
        self.silent = false;
    }
}

/// Rolling spectrum flattener: keeps a slowly falling peak of the loudest
/// band and shifts the frame so that peak sits near the target rail.
#[derive(Debug, Clone)]
pub struct RollingNormalizer {
    /// The rolling loud band level, in dB.
    peak_db: f32,
    /// Decay coefficient toward observed levels, per analysis frame.
    decay: f32,
    /// Current applied offset, in dB.
    gain_db: f32,
    /// Whether enabled at all: a disabled normalizer is a no-op, not a
    /// fixture the engine must special case.
    enabled: bool,
}

impl RollingNormalizer {
    /// Build for an analysis frame rate; `None` disables.
    #[must_use]
    pub fn new(analysis_rate_hz: f32, enabled: bool) -> Self {
        let rate = analysis_rate_hz.max(1.0);
        Self {
            peak_db: DB_FLOOR,
            decay: one_frame_coefficient(NORMALIZER_DECAY_S * 1000.0, rate),
            gain_db: 0.0,
            enabled,
        }
    }

    /// Whether the normalizer adjusts levels.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The offset currently applied to every band, in dB.
    #[must_use]
    pub fn current_gain_db(&self) -> f32 {
        self.gain_db
    }

    /// Clear state, on stream restart: the first frames re-establish the
    /// peak from scratch.
    pub fn reset(&mut self) {
        self.peak_db = DB_FLOOR;
        self.gain_db = 0.0;
    }

    /// Normalize a frame of bands in place, in dB.
    ///
    /// The rolling peak jumps instantly to anything louder than itself and
    /// falls with a 3 s time constant otherwise, which is the "rolling
    /// window" behavior without keeping a literal window buffer.
    pub fn normalize(&mut self, bands: &mut [f32]) {
        if !self.enabled || bands.is_empty() {
            return;
        }
        let frame_peak = bands.iter().copied().fold(DB_FLOOR, f32::max);
        if frame_peak > self.peak_db {
            self.peak_db = frame_peak;
        } else {
            self.peak_db += self.decay * (frame_peak - self.peak_db);
        }
        let wanted = NORMALIZER_TARGET_DB - self.peak_db;
        self.gain_db = wanted.clamp(-NORMALIZER_MAX_GAIN_DB, NORMALIZER_MAX_GAIN_DB);
        for band in bands.iter_mut() {
            *band = (*band + self.gain_db).clamp(DB_FLOOR, 6.0);
        }
    }
}

/// One frame coefficient for a ms time constant at `rate_hz` frames/s.
fn one_frame_coefficient(ms: f32, rate_hz: f32) -> f32 {
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
    use std::f32::consts::TAU;

    const RATE: u32 = 48_000;
    const ANALYSIS_RATE: f32 = 46.875; // 1024 samples per frame at 48 kHz

    fn sine(seconds: f32, hz: f32, amplitude: f32) -> Vec<f32> {
        let count = (RATE as f32 * seconds) as usize;
        (0..count)
            .map(|i| amplitude * (TAU * hz * i as f32 / RATE as f32).sin())
            .collect()
    }

    #[test]
    fn a_loud_stream_never_gates() {
        let samples = sine(2.0, 1000.0, 0.3);
        let mut gate = SilenceGate::new(RATE, DEFAULT_SILENCE_THRESHOLD_DB);
        gate.detect_slice(&samples);
        assert!(!gate.is_silent(), "a -10 dBFS stream must stay active");
        assert!(gate.rms() > 0.2, "rms tracked poorly: {}", gate.rms());
    }

    #[test]
    fn five_hundred_milliseconds_of_silence_gates() {
        let mut gate = SilenceGate::new(RATE, DEFAULT_SILENCE_THRESHOLD_DB);
        // Prime with signal, then give it half a second of dead air.
        gate.detect_slice(&sine(0.2, 1000.0, 0.3));
        assert!(!gate.is_silent());
        gate.detect_slice(&sine(1.0, 1000.0, 0.0));
        assert!(gate.is_silent(), "silence must gate after 500 ms");
    }

    #[test]
    fn above_the_threshold_noise_stays_active() {
        let mut gate = SilenceGate::new(RATE, DEFAULT_SILENCE_THRESHOLD_DB);
        // -55 dBFS RMS of noise rides just above the -60 dBFS threshold.
        gate.detect_slice(&sine(1.5, 1000.0, 0.0018));
        assert!(!gate.is_silent(), "-55 dBFS noise must not gate");
    }

    #[test]
    fn below_the_threshold_noise_gates() {
        let mut gate = SilenceGate::new(RATE, DEFAULT_SILENCE_THRESHOLD_DB);
        // -80 dBFS noise is below the -60 dBFS threshold.
        gate.detect_slice(&sine(1.5, 1000.0, 0.0001));
        assert!(gate.is_silent(), "-80 dBFS noise must gate");
    }

    #[test]
    fn gate_smoothing_ramps_down_over_two_hundred_milliseconds() {
        let mut smoothing = GateSmoothing::new(ANALYSIS_RATE);
        smoothing.reset();
        assert!(
            (smoothing.step(false) - 1.0).abs() < 1e-3,
            "active streams stay open"
        );
        // With a 200 ms time constant the ramp is at roughly 40% after one
        // ramp's worth of frames (9 at 46.875 Hz), and well below a tenth
        // after three constants' worth.
        let mut halfway = None;
        let mut nearly_gone = false;
        for frame in 1..=32 {
            let factor = smoothing.step(true);
            if halfway.is_none() && factor < 0.5 {
                halfway = Some(frame);
            }
            if frame > 16 && factor < 0.05 {
                nearly_gone = true;
            }
        }
        assert!(
            halfway.unwrap_or(64) <= 11,
            "ramp fell too fast: {halfway:?} frames"
        );
        assert!(nearly_gone, "the gate ramp never reached zero");
    }

    #[test]
    fn gate_smoothing_reopens_quickly_on_sound() {
        let mut smoothing = GateSmoothing::new(ANALYSIS_RATE);
        while smoothing.step(true) > 0.05 {}
        // Sound is back: within a few frames the multiplier must be
        // substantially open again.
        let mut reopened = false;
        for _ in 0..6 {
            if smoothing.step(false) > 0.7 {
                reopened = true;
            }
        }
        assert!(reopened, "the gate never reopened within 6 frames");
    }

    #[test]
    fn the_normalizer_levels_a_quiet_mix_up_to_the_target() {
        let mut normalizer = RollingNormalizer::new(ANALYSIS_RATE, true);
        // A quiet track: everything sits at -40 dB.
        let mut bands = [-40.0f32; 32];
        // The peak jumps instantly on the first frame, then the requested
        // gain is clamped to the +-12 dB budget.
        normalizer.normalize(&mut bands);
        assert!(
            (normalizer.current_gain_db() - NORMALIZER_MAX_GAIN_DB).abs() < 0.01,
            "quiet track gain: {}",
            normalizer.current_gain_db()
        );
        // -40 dB plus the clamped +12 dB read -28 dB.
        assert!(
            (bands[0] - (-28.0)).abs() < 0.01,
            "band after normalize: {}",
            bands[0]
        );
    }

    #[test]
    fn the_normalizer_leaves_a_loud_mix_alone_and_decays_slowly() {
        let mut normalizer = RollingNormalizer::new(ANALYSIS_RATE, true);
        // Loud frame: peak at -1.
        let mut loud = [-1.0f32; 32];
        normalizer.normalize(&mut loud);
        assert!(
            (normalizer.current_gain_db() - (NORMALIZER_TARGET_DB - (-1.0))).abs() < 0.5,
            "loud frame gain: {}",
            normalizer.current_gain_db()
        );
        // Then a quiet frame: the peak must NOT collapse instantly; the
        // 3 s decay holds the gain near its previous value for a while.
        let mut quiet = [-40.0f32; 32];
        normalizer.normalize(&mut quiet);
        assert!(
            normalizer.current_gain_db() > -20.0,
            "peak collapsed instantly: {}",
            normalizer.current_gain_db()
        );
    }

    #[test]
    fn a_disabled_normalizer_is_a_no_op() {
        let mut normalizer = RollingNormalizer::new(ANALYSIS_RATE, false);
        let mut bands = [-40.0f32; 8];
        let before = bands;
        normalizer.normalize(&mut bands);
        assert_eq!(bands, before);
        assert!(!normalizer.is_enabled());
        assert!((normalizer.current_gain_db()).abs() < 1e-6);
    }

    #[test]
    fn resets_clear_the_rolling_peak() {
        let mut normalizer = RollingNormalizer::new(ANALYSIS_RATE, true);
        let mut bands = [-5.0f32; 8];
        normalizer.normalize(&mut bands);
        assert!(normalizer.current_gain_db().abs() > 0.01);
        normalizer.reset();
        assert!((normalizer.current_gain_db()).abs() < 1e-6);
    }
}
