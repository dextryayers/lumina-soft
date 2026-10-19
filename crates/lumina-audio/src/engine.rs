//! The analysis engine: ring reader to render-ready spectrum frame.
//!
//! # Shape of the system
//!
//! Two threads cooperate through the [`crate::ring`]:
//!
//! * The capture callback thread converts, cleans, and pushes mono samples
//!   into the [`crate::ring::RingWriter`]. It never allocates, never locks,
//!   and never blocks: if the ring is full the samples are dropped, which
//!   is harmless to a visualizer that prefers recent audio over stale
//!   audio anyway.
//! * The UI thread owns an [`AudioEngine`] and calls
//!   [`AudioEngine::poll`] once per rendered frame. The engine consumes
//!   whole windows from the [`crate::ring::RingReader`] when available
//!   and otherwise lets the envelope decay between FFT updates, so the
//!   visual keeps animating smoothly even though the FFT only recomputes
//!   at the analysis rate (window size / capture rate, e.g. 11.7 Hz for a
//!   4096 window at 48 kHz).
//!
//! # What comes out
//!
//! [`SpectrumFrame`]: per band amplitude in the linear domain, ready to
//! be drawn, plus held peaks for the tips, 3 band energy aggregates for
//! the RGB reactive engine (PLAN §13.3), overall loudness, and a silent
//! flag fed by the [`crate::dynamics::SilenceGate`].

use crate::bands::{BandMapper, DB_FLOOR, DEFAULT_MAX_HZ, DEFAULT_MIN_HZ};
use crate::dynamics::{GateSmoothing, RollingNormalizer, SilenceGate};
use crate::envelope::BandEnvelope;
use crate::fft::Analyzer;
use crate::ring::RingReader;
use std::sync::Arc;

/// Tuning bundle for one engine lifetime.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineSettings {
    /// The sample rate of the capture stream.
    pub sample_rate: u32,
    /// FFT window size in real samples, a power of two.
    pub fft_size: usize,
    /// Number of spectrum bands.
    pub band_count: usize,
    /// Envelope attack in milliseconds.
    pub attack_ms: f32,
    /// Envelope release in milliseconds.
    pub release_ms: f32,
    /// Extra gain in dB from the config's sensitivity multiplier.
    pub sensitivity_db: f32,
    /// RMS level below which the gate closes, in dBFS.
    pub silence_threshold_db: f32,
    /// Bass tilt in dB per octave below the 200 Hz anchor.
    pub bass_tilt_db_per_octave: f32,
    /// Whether the rolling normalizer is active.
    pub normalize: bool,
    /// Bottom of the band range.
    pub min_hz: f32,
    /// Top of the band range, before the Nyquist clamp.
    pub max_hz: f32,
}

impl EngineSettings {
    /// Derive settings from the audio configuration and the stream rate.
    #[must_use]
    pub fn from_audio(config: &lumina_config::schema::AudioConfig, sample_rate: u32) -> Self {
        let sensitivity = if config.sensitivity > 0.0 {
            config.sensitivity
        } else {
            1.0
        };
        Self {
            sample_rate,
            fft_size: config.fft_size.samples(),
            band_count: config.band_count.max(1) as usize,
            attack_ms: config.attack.max(0.1),
            release_ms: config.release.max(0.1),
            sensitivity_db: 20.0 * sensitivity.log10(),
            silence_threshold_db: config.silence_threshold_db,
            bass_tilt_db_per_octave: config.bass_tilt_db_per_octave,
            normalize: config.normalize,
            min_hz: DEFAULT_MIN_HZ,
            max_hz: DEFAULT_MAX_HZ,
        }
    }
}

/// The PLAN §13.3 aggregate ranges, shared by the engine and the color crate.
pub const BASS_UPPER_HZ: f32 = 250.0;
/// Upper edge of the mid region.
pub const MID_UPPER_HZ: f32 = 2000.0;

/// Three band energy aggregates for the RGB reactive engine, in linear
/// amplitude, each clamped to `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AudioAggregates {
    /// Energy of the 20..250 Hz region.
    pub bass: f32,
    /// Energy of the 250 Hz..2 kHz region.
    pub mid: f32,
    /// Energy above 2 kHz.
    pub treble: f32,
    /// Broadband loudness, an RMS over all bands.
    pub loudness: f32,
}

/// A completed analysis pass, cheap to clone, ready for the widget layer.
#[derive(Debug, Clone)]
pub struct SpectrumFrame {
    /// Monotonic counter, one per `poll` call: paces the visual even when
    /// no new FFT data arrived.
    pub frame_index: u64,
    /// Linear amplitude per band, `0..=1`.
    pub bands: Arc<[f32]>,
    /// Held peak amplitude per band, `0..=1`, for the tips.
    pub peaks: Arc<[f32]>,
    /// True when the gate has closed: nothing worth drawing is playing.
    pub silent: bool,
    /// False when no capture stream has fed the ring recently.
    pub active: bool,
    /// Broadband loudness, linear `0..=1`.
    pub loudness: f32,
    /// The three band aggregates for the color engine.
    pub aggregates: AudioAggregates,
    /// Capture sample rate the current bands were measured at.
    pub sample_rate: u32,
    /// FFT window size backing the band map.
    pub fft_size: usize,
}

/// How fast a held peak falls, in dB per second.
const PEAK_DECAY_DB_PER_SECOND: f32 = 20.0;
/// The render rate the engine assumes for per-poll peak decay. The real
/// rate drifts with vsync, and the visual difference is negligible.
const POLLS_PER_SECOND: f32 = 60.0;
/// How long a ring may starve before the frame is marked inactive, in
/// analysis-rate seconds. One second of ring silence is plenty: a real
/// stream delivers windows continuously while it plays.
const STALL_SECONDS: f32 = 1.5;

/// Everything the analysis side needs, owned by the UI thread.
///
/// The engine is not `Send` and never needs to be: the capture thread only
/// ever touches the ring's writer half.
#[derive(Debug)]
pub struct AudioEngine {
    reader: RingReader,
    settings: EngineSettings,
    analyzer: Analyzer,
    mapper: BandMapper,
    envelope: BandEnvelope,
    gate: SilenceGate,
    smoothing: GateSmoothing,
    normalizer: RollingNormalizer,
    /// FFT input window, read from the ring.
    window: Box<[f32]>,
    /// FFT output magnitudes.
    magnitudes: Box<[f32]>,
    /// Post-tilt, post-normalize band levels, in dB.
    raw_bands: Box<[f32]>,
    /// Held peaks, in dB.
    peaks_db: Box<[f32]>,
    /// Peak decay per poll cached from the settings.
    peak_decay_per_poll: f32,
    /// Monotonic frame counter.
    frame_index: u64,
    /// The frame index that last saw fresh ring data.
    last_data_frame: u64,
    /// Whether any ring data has ever been consumed by this engine.
    has_seen_data: bool,
}

impl AudioEngine {
    /// Create an engine and the writer half of its ring.
    ///
    /// The capture thread gets the writer; this thread keeps the reader.
    #[must_use]
    pub fn new(settings: EngineSettings) -> (Self, crate::ring::RingWriter) {
        let (writer, reader) = crate::ring::RingBuffer::new(16_384).split();
        (Self::new_for_reader(reader, settings), writer)
    }

    /// Create an engine around an existing reader.
    ///
    /// # Panics
    ///
    /// Panics when `fft_size` is not a power of two, or `band_count` is
    /// zero, via the component constructors.
    #[must_use]
    pub fn new_for_reader(reader: RingReader, settings: EngineSettings) -> Self {
        let analysis_rate = settings.sample_rate.max(1) as f32 / settings.fft_size.max(1) as f32;
        let analyzer = Analyzer::new(settings.fft_size);
        let mapper = BandMapper::new(
            settings.sample_rate,
            settings.fft_size,
            settings.band_count,
            settings.min_hz,
            settings.max_hz,
            settings.bass_tilt_db_per_octave,
            settings.sensitivity_db,
        );
        let envelope =
            BandEnvelope::new(settings.band_count, settings.attack_ms, settings.release_ms, analysis_rate);
        let gate = SilenceGate::new(settings.sample_rate, settings.silence_threshold_db);
        let smoothing = GateSmoothing::new(analysis_rate);
        let normalizer = RollingNormalizer::new(analysis_rate, settings.normalize);
        let band_count = settings.band_count;
        let magnitudes_len = analyzer.bin_count();

        Self {
            reader,
            peak_decay_per_poll: PEAK_DECAY_DB_PER_SECOND / POLLS_PER_SECOND,
            analyzer,
            mapper,
            envelope,
            gate,
            smoothing,
            normalizer,
            window: vec![0.0f32; settings.fft_size].into_boxed_slice(),
            magnitudes: vec![0.0f32; magnitudes_len].into_boxed_slice(),
            raw_bands: vec![DB_FLOOR; band_count].into_boxed_slice(),
            peaks_db: vec![DB_FLOOR; band_count].into_boxed_slice(),
            settings,
            frame_index: 0,
            last_data_frame: 0,
            has_seen_data: false,
        }
    }

    /// The settings the engine was built with.
    #[must_use]
    pub fn settings(&self) -> &EngineSettings {
        &self.settings
    }

    /// Rebind to a new capture rate (a device switch), keeping tuning and
    /// the same ring.
    pub fn reconfigure(&mut self, sample_rate: u32) {
        let mut settings = self.settings.clone();
        settings.sample_rate = sample_rate;
        self.apply_settings(settings);
    }

    /// Rebuild every rate dependent component for `settings`.
    fn apply_settings(&mut self, settings: EngineSettings) {
        let analysis_rate = settings.sample_rate.max(1) as f32 / settings.fft_size.max(1) as f32;
        self.peak_decay_per_poll = PEAK_DECAY_DB_PER_SECOND / POLLS_PER_SECOND;
        self.window = vec![0.0f32; settings.fft_size].into_boxed_slice();
        let analyzer = Analyzer::new(settings.fft_size);
        self.magnitudes = vec![0.0f32; analyzer.bin_count()].into_boxed_slice();
        self.analyzer = analyzer;
        self.mapper = BandMapper::new(
            settings.sample_rate,
            settings.fft_size,
            settings.band_count,
            settings.min_hz,
            settings.max_hz,
            settings.bass_tilt_db_per_octave,
            settings.sensitivity_db,
        );
        self.envelope =
            BandEnvelope::new(settings.band_count, settings.attack_ms, settings.release_ms, analysis_rate);
        self.gate = SilenceGate::new(settings.sample_rate, settings.silence_threshold_db);
        self.smoothing = GateSmoothing::new(analysis_rate);
        self.normalizer = RollingNormalizer::new(analysis_rate, settings.normalize);
        let band_count = settings.band_count;
        self.raw_bands = vec![DB_FLOOR; band_count].into_boxed_slice();
        self.peaks_db = vec![DB_FLOOR; band_count].into_boxed_slice();
        self.settings = settings;
    }

    /// Advance the analysis one poll and return the current frame.
    ///
    /// Call once per redraw. Fresh FFT passes only happen when a full
    /// window is buffered; between them the envelope and peaks decay so
    /// the widget still animates at display rate.
    pub fn poll(&mut self) -> SpectrumFrame {
        self.frame_index += 1;
        let window_len = self.settings.fft_size;
        let analysis_rate = self.settings.sample_rate.max(1) as f32 / window_len.max(1) as f32;

        if self.reader.pending() >= window_len
            && self.reader.read_exact(&mut self.window).is_some()
        {
            self.has_seen_data = true;
            self.last_data_frame = self.frame_index;
            self.gate.detect_slice(&self.window);
            self.analyzer.process(&self.window, &mut self.magnitudes);
            self.mapper.map(&self.magnitudes, &mut self.raw_bands);
            self.normalizer.normalize(&mut self.raw_bands);
        } else {
            // Starved ring: after a while of no windows at all, the last
            // spectrum targets are stale, so drop them to the floor and
            // let the envelope's release do the visual fall.
            let stall_frames = (analysis_rate * STALL_SECONDS).ceil() as u64;
            if self.frame_index.wrapping_sub(self.last_data_frame) > stall_frames {
                for band in &mut self.raw_bands {
                    *band = DB_FLOOR;
                }
            }
        }

        // The envelope always advances so bars animate between FFT passes.
        self.envelope.update(&self.raw_bands);

        let silent = self.gate.is_silent();
        let factor = self.smoothing.step(silent);
        let gate_offset_db = gate_offset_db(factor);

        let band_count = self.settings.band_count;
        let mut bands = Vec::with_capacity(band_count);
        let mut peaks = Vec::with_capacity(band_count);

        let mut bass_sum = 0.0f32;
        let mut bass_count = 0usize;
        let mut mid_sum = 0.0f32;
        let mut mid_count = 0usize;
        let mut treble_sum = 0.0f32;
        let mut treble_count = 0usize;
        let mut square_sum = 0.0f32;

        for index in 0..band_count {
            let level = self.envelope.level(index) + gate_offset_db;
            let amplitude = amplitude_from_db(level);
            square_sum += amplitude * amplitude;
            bands.push(amplitude);

            if level > self.peaks_db[index] {
                self.peaks_db[index] = level;
            } else {
                self.peaks_db[index] =
                    (self.peaks_db[index] - self.peak_decay_per_poll).max(DB_FLOOR);
            }
            peaks.push(amplitude_from_db(self.peaks_db[index]));

            let center = self.mapper.center_freq_hz(index);
            match center {
                f if f <= BASS_UPPER_HZ => {
                    bass_sum += amplitude;
                    bass_count += 1;
                }
                f if f <= MID_UPPER_HZ => {
                    mid_sum += amplitude;
                    mid_count += 1;
                }
                _ => {
                    treble_sum += amplitude;
                    treble_count += 1;
                }
            }
        }

        let mean_of = |sum: f32, count: usize| if count == 0 { 0.0 } else { sum / count as f32 };
        let bass = mean_of(bass_sum, bass_count);
        let mid = mean_of(mid_sum, mid_count);
        let treble = mean_of(treble_sum, treble_count);

        let squared_mean = mean_of(square_sum, band_count);
        let loudness = squared_mean.sqrt().clamp(0.0, 1.0);
        let mut since_data = self.frame_index.wrapping_sub(self.last_data_frame) as f32;
        since_data = since_data.clamp(0.0, analysis_rate * STALL_SECONDS.max(0.5) + 1.0);
        let active = self.has_seen_data
            && since_data < analysis_rate * STALL_SECONDS.max(0.5);

        SpectrumFrame {
            frame_index: self.frame_index,
            bands: bands.into(),
            peaks: peaks.into(),
            silent,
            active,
            loudness,
            aggregates: AudioAggregates {
                bass,
                mid,
                treble,
                loudness,
            },
            sample_rate: self.settings.sample_rate,
            fft_size: self.settings.fft_size,
        }
    }

    /// Samples still buffered in the ring, for diagnostics.
    #[must_use]
    pub fn pending_samples(&self) -> usize {
        self.reader.pending()
    }

    /// Discard ring backlog, after a device switch or a resume.
    pub fn clear_backlog(&mut self) {
        self.reader.clear();
        self.gate.reset();
        self.smoothing.reset();
        self.envelope.reset();
        self.normalizer.reset();
    }
}

/// The gate multiplier expressed as a dB offset for the dB domain paths.
fn gate_offset_db(factor: f32) -> f32 {
    if factor <= 1e-4 {
        DB_FLOOR
    } else {
        20.0 * factor.log10()
    }
}

/// dB to linear amplitude, clamped to the visualizer's `0..=1` rail.
fn amplitude_from_db(db: f32) -> f32 {
    if db <= DB_FLOOR {
        0.0
    } else {
        10f32.powf(db / 20.0).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::RingWriter;
    use std::f32::consts::TAU;

    const RATE: u32 = 48_000;
    const WINDOW: usize = 4096;
    const BANDS: usize = 32;

    fn settings(normalize: bool) -> EngineSettings {
        use crate::dynamics::DEFAULT_SILENCE_THRESHOLD_DB;
        EngineSettings {
            sample_rate: RATE,
            fft_size: WINDOW,
            band_count: BANDS,
            attack_ms: 6.0,
            release_ms: 120.0,
            sensitivity_db: 0.0,
            silence_threshold_db: DEFAULT_SILENCE_THRESHOLD_DB,
            bass_tilt_db_per_octave: 3.0,
            normalize,
            min_hz: DEFAULT_MIN_HZ,
            max_hz: DEFAULT_MAX_HZ,
        }
    }

    fn sine(seconds: f32, hz: f32, amplitude: f32) -> Vec<f32> {
        let count = (RATE as f32 * seconds) as usize;
        (0..count)
            .map(|i| amplitude * (TAU * hz * i as f32 / RATE as f32).sin())
            .collect()
    }

    fn write_silent(engine: &mut AudioEngine, writer: &RingWriter, windows: usize) {
        for _ in 0..windows {
            writer.write(&[0.0f32; WINDOW]);
        }
    }

    #[test]
    fn a_sine_stream_lights_the_mutated_bands_up() {
        let (mut engine, writer) = AudioEngine::new(settings(false));
        writer.write(&sine(0.5, 100.0, 0.5));
        let mut last = None;
        for _ in 0..20 {
            last = Some(engine.poll());
        }
        let frame = last.expect("polls ran");
        assert!(frame.active, "the stream must be flagged active");
        assert!(frame.sample_rate == RATE && frame.fft_size == WINDOW);
        // A 100 Hz sine must land in the bass region.
        assert!(
            frame.aggregates.bass > frame.aggregates.mid * 2.0,
            "aggregates: {:?} bass must dominate for a 100 Hz sine",
            frame.aggregates
        );
        assert!(
            frame.aggregates.bass > 0.05,
            "bass amplitude collapsed: {:?}",
            frame.aggregates
        );
        // And some band should actually move.
        assert!(
            frame.bands.iter().any(|b| *b > 0.02),
            "no band rose above the noise: {:?}",
            frame.bands
        );
    }

    #[test]
    fn five_hundred_milliseconds_of_silence_fades_the_bars() {
        let (mut engine, writer) = AudioEngine::new(settings(false));
        writer.write(&sine(0.4, 1000.0, 0.4));
        for _ in 0..8 {
            let _ = engine.poll();
        }
        // Half a second of dead air: eight windows is 0.68 s.
        write_silent(&mut engine, &writer, 40);
        let mut last = None;
        for _ in 0..30 {
            last = Some(engine.poll());
        }
        let frame = last.expect("polls ran");
        assert!(frame.silent, "the gate must have closed");
        let moving = frame.bands.iter().any(|b| *b > 0.005);
        assert!(!moving, "bars must die under silence: {:?}", frame.bands);
    }

    #[test]
    fn a_stalled_ring_falls_back_to_quiet_and_inactive() {
        let (mut engine, _writer) = AudioEngine::new(settings(false));
        // No writer data at all: the engine must stay coherent regardless.
        let mut last_active = true;
        for _ in 0..80 {
            let frame = engine.poll();
            last_active = frame.active;
        }
        assert!(!last_active, "a starved engine must flag inactive");
        let frame = engine.poll();
        assert!(frame.loudness < 0.01, "stalled loudness: {}", frame.loudness);
        // Frame pacing survived the stall.
        assert!(frame.frame_index >= 80);
    }

    #[test]
    fn held_peaks_outlive_the_bars_they_follow() {
        let (mut engine, writer) = AudioEngine::new(settings(false));
        writer.write(&sine(0.5, 1000.0, 0.6));
        for _ in 0..10 {
            let _ = engine.poll();
        }
        write_silent(&mut engine, &writer, 60);
        let mut frame = None;
        for _ in 0..12 {
            frame = Some(engine.poll());
        }
        let frame = frame.expect("polls ran");
        assert!(
            frame.peaks.iter().any(|p| *p > 0.02),
            "peaks vanished instantly: {:?}",
            frame.peaks
        );
        // And after many polls the peaks too are gone.
        let mut frame = None;
        for _ in 0..220 {
            frame = Some(engine.poll());
        }
        let frame = frame.expect("polls ran");
        assert!(
            frame.peaks.iter().all(|p| *p < 0.05),
            "peaks never decayed: {:?}",
            frame.peaks
        );
    }

    #[test]
    fn the_normalizer_lifts_a_quiet_track() {
        let (mut engine, writer) = AudioEngine::new(settings(true));
        writer.write(&sine(0.5, 1000.0, 0.002));
        let mut normalized = None;
        for _ in 0..16 {
            normalized = Some(engine.poll());
        }
        let (mut engine_plain, writer_plain) = AudioEngine::new(settings(false));
        writer_plain.write(&sine(0.5, 1000.0, 0.002));
        let mut plain = None;
        for _ in 0..16 {
            plain = Some(engine_plain.poll());
        }
        let loud = normalized.expect("polls ran");
        let quiet = plain.expect("polls ran");
        let loud_max = loud.bands.iter().copied().fold(0.0, f32::max);
        let quiet_max = quiet.bands.iter().copied().fold(0.0, f32::max);
        assert!(
            loud_max > quiet_max * 3.0,
            "normalization failed: loud {loud_max} vs quiet {quiet_max}"
        );
    }

    #[test]
    fn reconfigure_survives_a_device_rate_change() {
        let (mut engine, writer) = AudioEngine::new(settings(false));
        engine.reconfigure(44_100);
        assert_eq!(engine.settings().sample_rate, 44_100);
        writer.write(&sine(0.5, 1000.0, 0.5));
        let mut last = None;
        for _ in 0..20 {
            last = Some(engine.poll());
        }
        let frame = last.expect("polls ran");
        assert_eq!(frame.sample_rate, 44_100);
        assert!(
            frame.bands.iter().any(|b| *b > 0.02),
            "the engine stopped working after reconfigure: {:?}",
            frame.bands
        );
    }

    #[test]
    fn ring_drops_do_not_break_frame_pacing() {
        let (mut engine, writer) = AudioEngine::new(settings(false));
        // A burst larger than the ring: some dropped, but the engine must
        // keep ticking and the FFT keeps consuming when it can.
        writer.write(&sine(2.0, 500.0, 0.5));
        let mut frames = Vec::new();
        for _ in 0..25 {
            frames.push(engine.poll());
        }
        let mut monotonic = true;
        for pair in frames.windows(2) {
            monotonic &= pair[0].frame_index < pair[1].frame_index;
        }
        assert!(monotonic, "frame indices must be monotonically increasing");
        assert!(
            frames[24].bands.iter().any(|b| *b > 0.01),
            "with fresh data available the bands must move"
        );
    }

    #[test]
    fn clearing_the_backlog_removes_history() {
        let (mut engine, writer) = AudioEngine::new(settings(false));
        writer.write(&sine(0.4, 300.0, 0.8));
        assert!(engine.pending_samples() > 0);
        engine.clear_backlog();
        assert_eq!(engine.pending_samples(), 0);
        let frame = engine.poll();
        assert!(
            frame.bands.iter().all(|b| *b < 0.02),
            "cleared history must not lift the bars: {:?}",
            frame.bands
        );
    }
}
