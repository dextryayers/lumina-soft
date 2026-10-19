//! The analysis engine: ring reader to render-ready spectrum frame.
//!
//! # Shape of the system
//!
//! Two threads cooperate through the [`crate::ring`]:
//!
//! * The capture callback thread converts, cleans, and pushes mono samples
//!   into the [`crate::ring::RingWriter`]. It never allocates, never locks,
//!   and never blocks: if the ring is full the samples are dropped, which
//!   is harmless to a visualizer.
//! * The UI thread owns an [`AudioEngine`] and calls
//!   [`AudioEngine::poll`] once per rendered frame. The engine consumes
//!   whole windows from the [`crate::ring::RingReader`] when available and
//!   otherwise lets the envelope decay between FFT updates, so the visual
//!   keeps animating smoothly even though the FFT only recomputes at the
//!   analysis rate.
//!
//! # What comes out
//!
//! [`SpectrumFrame`]: per band amplitude in the linear domain, ready to be
//! drawn, plus held peaks for the tips, 3 band energy aggregates for the
//! RGB reactive engine (PLAN §13.3), overall loudness, and a silent flag
//! fed by the [`crate::dynamics::SilenceGate`].

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
    /// RMS level below which the gate closes, dBFS.
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
    /// False only when no capture stream has fed the ring recently.
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

/// Everything the analysis side needs, owned by the UI thread.
///
/// The lifetime parameters are all erased behind the ring split and `Arc`s;
/// the engine itself is neither `Send` nor `Sync` and never needs to be.
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
    /// Held peaks, in dB, kept one frame behind the envelope.
    peaks_db: Box<[f32]>,
    /// Monotonic frame counter.
    frame_index: u64,
    /// The last frame index that saw fresh ring data.
    last_data_frame: u64,
    /// The ring's drop count at the last poll, for diagnostics.
    last_dropped: u64,
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
        let analysis_rate = f32::from(settings.sample_rate.max(1)) / settings.fft_size.max(1) as f32;
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
        let envelope = BandEnvelope::new(settings.band_count, settings.attack_ms, settings.release_ms, analysis_rate);
        let gate = SilenceGate::new(settings.sample_rate, settings.silence_threshold_db);
        let smoothing = GateSmoothing::new(analysis_rate);
        let normalizer =
            RollingNormalizer::new(analysis_rate, settings.normalize);
        let window = vec![0.0f32; settings.fft_size].into_boxed_slice();
        let magnitudes = vec![0.0f32; analyzer.bin_count()].into_boxed_slice();
        let raw_bands = vec![DB_FLOOR; settings.band_count].into_boxed_slice();
        let peaks_db = vec![DB_FLOOR; settings.band_count].into_boxed_slice();

        Self {
            reader,
            settings,
            analyzer,
            mapper,
            envelope,
            gate,
            smoothing,
            normalizer,
            window,
            magnitudes,
            raw_bands,
            peaks_db,
            frame_index: 0,
            last_data_frame: 0,
            last_dropped: 0,
        }
    }

    /// The settings the engine was built with.
    #[must_use]
    pub fn settings(&self) -> &EngineSettings {
        &self.settings
    }

    /// Rebind to a new capture rate (a device switch), keeping tuning.
    pub fn reconfigure(&mut self, sample_rate: u32) {
        let mut settings = self.settings.clone();
        settings.sample_rate = sample_rate;
        *self = Self::new_for_reader(std::mem::replace(
            &mut self.reader,
            // Unreachable: replaced back immediately below.
            Self::placeholder_reader(),
        ), settings);
    }

    fn placeholder_reader() -> RingReader {
        crate::ring::RingBuffer::new(4).split().1
    }

    /// Advance the analysis one poll and return the current frame.
    ///
    /// Call once per redraw. Fresh FFT passes only happen when a full window
    /// is buffered; between them the envelope and peaks decay so the widget
    /// still animates at display rate.
    pub fn poll(&mut self) -> SpectrumFrame {
        self.frame_index += 1;
        let window_len = self.settings.fft_size;

        if self.reader.pending() >= window_len
            && self.reader.read_exact(&mut self.window[..window_len]).is_some()
        {
            self.gate.detect_slice(&self.window);
            self.analyzer.process(&self.window, &self.magnitudes &mut [*to_complete]);
        }
        Self::unfinished_marker()
    }
}
