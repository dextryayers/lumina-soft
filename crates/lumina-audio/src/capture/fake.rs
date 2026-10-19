//! Synthetic capture: sine sweeps, noise, and silence with no hardware.
//!
//! This is how the visualizer gets exercised on CI machines, in tests, and
//! during a screenshot mode: a `FakePattern` is a signed generator whose
//! output goes straight into the ring at a nominal 48 kHz stereo, so every
//! downstream stage (downmix, DC removal, FFT, bands, envelope) runs the
//! exact same code a live stream would.

use super::{spawn_worker, stop_flagged, CaptureError, CaptureFormat, CaptureRequest, ReadySignal};
use crate::ring::RingWriter;
use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::Duration;

/// What the fake stream emits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FakePattern {
    /// A pure sine at the given frequency and peak amplitude.
    Sine {
        /// Peak frequency in hertz.
        hz: f32,
        /// Peak amplitude, `0..=1`.
        amplitude: f32,
    },
    /// White noise at the given peak amplitude, LCG generated so tests stay
    /// deterministic run to run.
    Noise {
        /// Peak amplitude, `0..=1`.
        amplitude: f32,
    },
    /// All zeros: drives the silence gate.
    Silence,
}

impl FakePattern {
    /// Sample the pattern at stream position `index`.
    fn sample(self, index: u64) -> f32 {
        // A 32 bit LCG keeps noise deterministic yet flat-ish.
        fn lcg(state: &mut u64) -> f32 {
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let bits = (*state >> 33) as u32;
            (bits as f32 / u32::MAX as f32) * 2.0 - 1.0
        }
        match self {
            Self::Sine { hz, amplitude } => {
                let phase = TAU * hz * (index as f32) / 48_000.0;
                amplitude * phase.sin()
            }
            Self::Noise { amplitude } => {
                let mut state = index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                amplitude * lcg(&mut state)
            }
            Self::Silence => 0.0,
        }
    }

    /// Small chunk size: keeps stop latency under a few milliseconds.
    const CHUNK: usize = 512;
}

/// Spawn a fake capture worker writing `pattern` into the ring.
///
/// The negotiated stereo 48 kHz format matches the PLAN's reference mix.
pub fn spawn(
    request: &CaptureRequest,
    writer: RingWriter,
    pattern: FakePattern,
) -> Result<super::RunningCapture, CaptureError> {
    let _ = request;
    spawn_worker("fake", move |stop: Arc<_>, signal: ReadySignal| {
        let format = CaptureFormat {
            sample_rate: 48_000,
            channels: 2,
            loopback: false,
        };
        signal.report(Ok(format));

        let mut post = crate::dsp::CapturePost::new(2, 48_000);
        let mut interleaved = vec![0.0f32; FakePattern::CHUNK * 2];
        let mut mono = vec![0.0f32; FakePattern::CHUNK];
        let mut index = 0u64;

        loop {
            if stop_flagged(&stop) {
                return;
            }
            // Stereo interleaved: both channels of a frame share the
            // pattern value, which keeps the generator frame aligned.
            for (slot_index, slot) in interleaved.iter_mut().enumerate() {
                let frame = index + (slot_index as u64) / 2;
                *slot = pattern.sample(frame);
            }
            let frames = post.process_interleaved(&interleaved, &mut mono);
            if frames > 0 {
                writer.write(&mono[..frames]);
            }
            index += FakePattern::CHUNK as u64;
            std::thread::sleep(Duration::from_millis(2));
        }
    })
}
