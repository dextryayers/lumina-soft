//! Shared audio analysis primitives for Lumina.
//!
//! This crate is intentionally small at the bottom of its own layer: everything
//! above the platform capture is a pure function of a sample stream, so it is
//! testable on any machine without audio hardware.
//!
//! Contents:
//!
//! * [`ring`] : lock-free SPSC ring buffer, the only crossing between the
//!   realtime capture callback and the analysis thread.
//! * [`dsp`] : downmix, DC removal, and highpass, run inside the callback.
//! * [`fft`] : Hann-windowed real FFT over the ring, with a warm planner.
//! * [`bands`] : logarithmic band mapping, dB conversion, and Nyquist clamp.
//! * [`envelope`] : asymmetric one-pole follower, so bars decay instead of
//!   strobing.
//! * [`dynamics`] : silence gate, rolling normalization, and bass tilt.
//! * [`capture`] : platform capture backends behind one [`crate::capture::AudioCapture`]
//!   trait, with automatic backend selection.
//! * [`engine`] : the [`AudioEngine`](crate::engine::AudioEngine) orchestrator
//!   that owns the analysis thread and publishes [`SpectrumFrame`](crate::engine::SpectrumFrame)s.
//!
//! # The realtime rules
//!
//! 1. The capture callback allocates nothing, takes no locks, and never blocks.
//! 2. When the ring is full the newest samples are dropped, never the oldest
//!   ones queued, because the visualizer wants *current* audio.
//! 3. Every buffer the analysis thread touches is allocated at construction.

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

pub mod bands;
pub mod capture;
pub mod dsp;
pub mod dynamics;
pub mod engine;
pub mod envelope;
pub mod fft;
pub mod ring;

pub use ring::{RingBuffer, RingReader, RingWriter};
pub use dsp::CapturePost;
pub use engine::{AudioAggregates, AudioEngine, AudioState, SpectrumFrame};

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
