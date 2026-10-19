//! Sample conditioning inside the capture callback: downmix, DC removal,
//! highpass.
//!
//! Everything here runs on the realtime thread, so each piece is a plain
//! struct with a small state and no allocation. The chain is applied
//! *before* the ring because cleaning the signal at the source means the
//! ring carries meanful audio only, and a 20 ms burst is 960 frames: the
//! state these filters keep is tiny.

/// Downmix interleaved multi-channel audio to mono into `out`.
///
/// Channels are averaged, not just copied: on stereo loopback a track that is
/// panned hard left would otherwise read as silence on the right channel and
/// the visualizer would lose half its energy.
///
/// Returns the number of frames written into `out`. `out` must be at least
/// `interleaved.len() / channels` long, which the caller guarantees because
/// both halves come from the same burst size.
///
/// # Panics
///
/// Never for `channels >= 1`. A zero channel count is filtered out by the
/// caller: no endpoint reports zero channels and the mix format is validated
/// at `open` time.
pub fn downmix_to_mono(interleaved: &[f32], channels: u16, out: &mut [f32]) -> usize {
    let channels = channels.max(1) as usize;
    let frames = interleaved.len() / channels;
    let frames = frames.min(out.len());
    for frame in 0..frames {
        let start = frame * channels;
        let chunk = &interleaved[start..start + channels];
        let sum: f32 = chunk.iter().sum();
        out[frame] = sum / channels as f32;
    }
    frames
}

/// A leaky-integrator DC blocker.
///
/// Subtracts a running mean of the input with a leak factor `alpha`. The
/// default from the PLAN is 0.999 per sample: at 48 kHz that is a time
/// constant of about 20 ms, fast enough to follow the endpoint's own DC
/// wander but slow enough to leave everything above roughly 8 Hz alone.
///
/// Formally, per sample:
///
/// ```text
/// mean  <- alpha * mean + (1 - alpha) * x
/// out   = x - mean
/// ```
#[derive(Debug, Clone)]
pub struct DcRemover {
    mean: f32,
    alpha: f32,
}

impl DcRemover {
    /// Create a remover with the given per-sample leak factor.
    #[must_use]
    pub fn new(alpha: f32) -> Self {
        Self {
            mean: 0.0,
            alpha: if alpha.is_finite() { alpha.clamp(0.9, 0.999_9) } else { 0.999 },
        }
    }

    /// Create a remover configured for a sample rate, targeting the PLAN
    /// default alpha of 0.999 at 48 kHz scaled to the actual rate, so the
    /// time constant stays constant in wall time rather than in samples.
    #[must_use]
    pub fn for_rate(sample_rate: u32) -> Self {
        // alpha_rate = exp(-dt / tau), tau = 20 ms.
        let tau = 0.02;
        let dt = 1.0 / sample_rate.max(8_000) as f32;
        Self::new((-dt / tau).exp())
    }

    /// Reset the integrator to zero, used after a stream restart so old DC
    /// does not leak into a new device's signal.
    pub fn reset(&mut self) {
        self.mean = 0.0;
    }

    /// Process one sample.
    pub fn process(&mut self, sample: f32) -> f32 {
        self.process_float(sample)
    }

    fn process_float(&mut self, sample: f32) -> f32 {
        self.mean += (1.0 - self.alpha) * (sample - self.mean);
        sample - self.mean
    }

    /// Process a buffer in place.
    pub fn process_slice(&mut self, samples: &mut [f32]) {
        for sample in samples {
            *sample = self.process_float(*sample);
        }
    }
}

/// A one pole highpass that kills rumble and DC offset below 30 Hz.
///
/// The classic form, with `a = exp(-2*pi*f_c / f_s)`:
///
/// ```text
/// y[n] = a * y[n-1] + a * (x[n] - x[n-1])
/// ```
///
/// At 30 Hz and 48 kHz the 3 dB point sits where the PLAN puts it, and a
/// first order slope buys -6 dB per octave below it, which is plenty for
/// killing a sub-woofer's DC stage and the endpoint's turn-on thump.
#[derive(Debug, Clone)]
pub struct HighPass {
    previous_x: f32,
    previous_y: f32,
    coefficient: f32,
    active: bool,
}

impl HighPass {
    /// Create a highpass with the given cutoff at the given sample rate.
    #[must_use]
    pub fn new(cutoff_hz: f32, sample_rate: u32) -> Self {
        let rate = sample_rate.max(8_000) as f32;
        let cutoff = cutoff_hz.max(1.0);
        let coefficient = (-std::f32::consts::TAU * cutoff / rate).exp();
        Self {
            previous_x: 0.0,
            previous_y: 0.0,
            coefficient,
            active: true,
        }
    }

    /// Process one sample.
    pub fn process(&mut self, sample: f32) -> f32 {
        if !self.active {
            return sample;
        }
        let out = self.coefficient * (self.previous_y + sample - self.previous_x);
        self.previous_x = sample;
        self.previous_y = out;
        out
    }

    /// Process a buffer in place.
    pub fn process_slice(&mut self, samples: &mut [f32]) {
        for sample in samples {
            *sample = self.process(*sample);
        }
    }

    /// Reset the filter state, used after a stream restart.
    pub fn reset(&mut self) {
        self.previous_x = 0.0;
        self.previous_y = 0.0;
    }
}

/// The capture-side post processor: downmix, DC remover, and highpass in the
/// order the PLAN §17.2 specifies.
///
/// One instance per stream. The buffers it needs are allocated once at
/// construction so the callback path is allocation free.
#[derive(Debug, Clone)]
pub struct CapturePost {
    dc: DcRemover,
    highpass: HighPass,
    channels: u16,
}

impl CapturePost {
    /// Build a post processor for a stream with the given channel count and
    /// sample rate.
    #[must_use]
    pub fn new(channels: u16, sample_rate: u32) -> Self {
        Self {
            dc: DcRemover::for_rate(sample_rate),
            highpass: HighPass::new(30.0, sample_rate),
            channels,
        }
    }

    /// Adjust for a change of endpoint mix format, for example when a device
    /// switch changes the sample rate under us. Resets the filter states so
    /// a rate mismatch cannot alias into the ring.
    pub fn reconfigure(&mut self, channels: u16, sample_rate: u32) {
        *self = Self::new(channels, sample_rate);
    }

    /// Convert one interleaved burst into cleaned mono samples in `out`.
    ///
    /// Returns the number of mono frames written.
    ///
    /// # Panics
    ///
    /// Never, for channel counts of at least one.
    pub fn process_interleaved(&mut self, interleaved: &[f32], out: &mut [f32]) -> usize {
        let frames = downmix_to_mono(interleaved, self.channels, out);
        if frames == 0 {
            return 0;
        }
        self.dc.process_slice(&mut out[..frames]);
        self.highpass.process_slice(&mut out[..frames]);
        frames
    }

    /// Reset both filters.
    pub fn reset(&mut self) {
        self.dc.reset();
        self.highpass.reset();
    }

    /// The channel count the post was built for.
    #[must_use]
    pub fn channels(&self) -> u16 {
        self.channels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    #[test]
    fn downmix_averages_channels_and_is_stateless() {
        let mut out = [0.0f32; 4];
        // Two stereo frames panned to opposite sides.
        assert_eq!(downmix_to_mono(&[1.0, 0.0, 0.0, 1.0], 2, &mut out), 2);
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert!((out[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn downmix_tolerates_a_short_output_buffer() {
        let mut out = [0.0f32; 2];
        assert_eq!(downmix_to_mono(&[1.0; 20], 2, &mut out), 2);
        assert!(out.iter().all(|s| *s == 1.0));
    }

    #[test]
    fn dc_remover_kills_a_constant_offset_quickly() {
        // A full scale DC offset must collapse to near zero within 20 ms,
        // which is roughly the leak integrator's time constant.
        let mut remover = DcRemover::for_rate(RATE);
        let mut last = 0.0f32;
        for _ in 0..(RATE / 100) {
            last = remover.process(0.5);
        }
        // One percent of one 20 ms time constant has passed, so the residual
        // should already be tiny.
        assert!(
            last.abs() < 0.02,
            "DC offset survived 10 ms of blocking: residual {last}"
        );
        // The mean sits essentially at the offset itself.
        assert!((remover.mean - 0.5).abs() < 0.02);
    }

    #[test]
    fn dc_remover_passes_slow_signals_alone() {
        // A barely audible 8 Hz hum should survive the DC blocker largely
        // intact; the attack that kills real DC is too slow to track it.
        let mut remover = DcRemover::for_rate(RATE);
        let mut peak = 0.0f32;
        for step in 0..RATE {
            let phase = std::f32::consts::TAU * 8.0 * step as f32 / RATE as f32;
            let sample = 0.25 * phase.sin();
            let out = remover.process(sample);
            peak = peak.max(out.abs());
        }
        assert!(
            peak > 0.18,
            "an 8 Hz tone was crushed by the DC remover, peak {peak}"
        );
    }

    #[test]
    fn highpass_kills_rumble_below_thirty_hertz() {
        let mut highpass = HighPass::new(30.0, RATE);
        // 10 Hz rumble, a typical turn-on thump tail.
        let mut low_peak = 0.0f32;
        for step in 0..RATE {
            let phase = std::f32::consts::TAU * 10.0 * step as f32 / RATE as f32;
            low_peak = low_peak.max(highpass.process(phase.sin()).abs());
        }
        assert!(
            low_peak < 0.45,
            "a 10 Hz rumble passed the 30 Hz highpass, peak {low_peak}"
        );
    }

    #[test]
    fn highpass_passes_music_frequencies() {
        let mut highpass = HighPass::new(30.0, RATE);
        // 100 Hz kick drum fundamental.
        let mut peak = 0.0f32;
        for step in 0..RATE {
            let phase = std::f32::consts::TAU * 100.0 * step as f32 / RATE as f32;
            peak = peak.max(highpass.process(phase.sin()).abs());
        }
        assert!(
            peak > 0.85,
            "a 100 Hz fundamental was attenuated by the highpass, peak {peak}"
        );
    }

    #[test]
    fn resetting_between_streams_does_not_carry_state() {
        let mut post = CapturePost::new(2, RATE);
        let mut out = [0.0f32; 4];
        // Charge the filters up with DC.
        post.process_interleaved(&[1.0; 8], &mut out);
        post.reset();
        post.process_interleaved(&[0.0; 8], &mut out);
        assert!(out.iter().all(|s| s.abs() < 1e-4), "state leaked across reset: {out:?}");
    }

    #[test]
    fn reconfigure_survives_a_sample_rate_change() {
        let mut post = CapturePost::new(2, 44_100);
        post.reconfigure(1, RATE);
        assert_eq!(post.channels(), 1);
        // Mono path still processes.
        let mut out = [0.0f32; 4];
        assert_eq!(post.process_interleaved(&[0.1, 0.2, 0.3, 0.4], &mut out), 4);
    }
}
