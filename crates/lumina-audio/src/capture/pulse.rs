//! PulseAudio capture via the simple API.
//!
//! On modern Linux the "pulse" and "pipewire" backend names are two doors
//! into the same room: PipeWire ships a `pipewire-pulse` compatibility
//! server that speaks the exact same protocol on
//! `$XDG_RUNTIME_DIR/pulse/native`. So one implementation serves both; see
//! the crate README and [`super::resolve_backend`].
//!
//! # Stream shape
//!
//! The simple API is a blocking reader, which is the right fit here: the
//! worker thread owns the read loop exclusively, and there is no callback
//! with a hard deadline to violate. Reads are cut into 10 ms chunks so a
//! stop request is honored within a single read cycle.
//!
//! # Source selection
//!
//! * `System` capture means "what plays on the speakers": Pulse source
//!   `@DEFAULT_MONITOR@` (the daemon resolves it), or `name.monitor` for a
//!   specific sink.
//! * `Microphone` means the given source name, or the daemon default.

use super::{spawn_worker, stop_flagged, CaptureError, CaptureFormat, CaptureRequest, ReadySignal};
use crate::ring::RingWriter;
use libpulse_binding::channelmap::Map as ChannelMap;
use libpulse_binding::sample::{Format as SampleFormat, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;
use std::sync::Arc;

/// The chunk consumed per read: 10 ms of stereo f32.
const CHUNK_FRAMES: usize = 480;
const CHUNK_CHANNELS: usize = 2;

/// Spawn a PulseAudio capture worker.
///
/// The worker negotiates a 2 channel 48 kHz f32 stream, which every
/// PulseAudio daemon resamples to natively, tightening the engine's
/// assumptions instead of making the DSP resize for arbitrary rates.
pub fn spawn(
    request: &CaptureRequest,
    writer: RingWriter,
) -> Result<super::RunningCapture, CaptureError> {
    // Copy the mutable request bits into the thread.
    let source = request.source;
    let device = request.device.clone();

    spawn_worker("pulse", move |stop: Arc<_>, signal: ReadySignal| {
        run_loop(stop, signal, source, &device, writer);
    })
}

/// The full worker body: open, report, loop.
///
/// The ready channel is one shot: it carries the negotiated format at
/// startup and is then dropped, so a far later read failure cannot block
/// on a receiver that already moved on.
fn run_loop(
    stop: Arc<std::sync::atomic::AtomicBool>,
    signal: ReadySignal,
    source: lumina_config::schema::AudioSource,
    device: &str,
    writer: RingWriter,
) {
    match source {
        lumina_config::schema::AudioSource::None => {
            signal.report(Err(CaptureError::DeviceUnavailable(
                "audio capture disabled by configuration".into(),
            )));
            return;
        }
        _ => {}
    }

    let spec = Spec {
        format: SampleFormat::F32le,
        rate: 48_000,
        channels: CHUNK_CHANNELS as u8,
    };
    let channel_map = ChannelMap::default().init_stereo().clone();
    let source_name = pulse_source_for(source, device);

    let simple = Simple::new(
        None,
        "lumina",
        Direction::Record,
        Some(source_name.as_str()),
        "lumina-spectrum",
        &spec,
        Some(&channel_map),
        None,
    );
    let simple = match simple {
        Ok(simple) => simple,
        Err(error) => {
            signal.report(Err(CaptureError::DeviceUnavailable(format!(
                "pulse: cannot open source {source_name}: {error}"
            ))));
            return;
        }
    };

    let format = CaptureFormat {
        sample_rate: 48_000,
        channels: CHUNK_CHANNELS as u16,
        loopback: matches!(source, lumina_config::schema::AudioSource::System),
    };
    signal.report(Ok(format));
    // One shot sent; release the channel so later errors never block.
    drop(signal);

    let mut post = crate::dsp::CapturePost::new(CHUNK_CHANNELS as u16, 48_000);
    let mut interleaved = vec![0.0f32; CHUNK_FRAMES * CHUNK_CHANNELS];
    let mut mono_scratch = vec![0.0f32; CHUNK_FRAMES];

    loop {
        if stop_flagged(&stop) {
            return;
        }
        // The server speaks little endian f32; `interleaved` views the same
        // storage as bytes for the read call, and on every little endian
        // target the cast round trips. Big endian platforms are not a
        // PulseAudio deployment reality.
        //
        // SAFETY: `interleaved` is a live `[f32]` allocation; casting to
        // `u8` doubles the length by a factor of 4 (`f32` size) and the
        // slice is fully initialized before any read of `interleaved`.
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                interleaved.as_mut_ptr().cast::<u8>(),
                interleaved.len() * 4,
            )
        };
        // `read` blocks until the full chunk arrived; that has no deadline,
        // so stop latency is bounded by the chunk: 10 ms.
        if simple.read(bytes).is_err() {
            // A failed read means the daemon closed or dropped the stream;
            // there is no recovery from the simple API, so the worker
            // exits and the running handle reports not running.
            return;
        }
        let frames = post.process_interleaved(&interleaved, &mut mono_scratch);
        if frames > 0 {
            writer.write(&mono_scratch[..frames]);
        }
    }
}

/// Map the configured source and device onto a Pulse source name.
#[must_use]
pub fn pulse_source_for(source: lumina_config::schema::AudioSource, device: &str) -> String {
    match source {
        lumina_config::schema::AudioSource::System => {
            if device_eq_default(device) {
                "@DEFAULT_MONITOR@".into()
            } else if device.ends_with(".monitor") {
                device.to_string()
            } else {
                format!("{device}.monitor")
            }
        }
        lumina_config::schema::AudioSource::Microphone => {
            if device_eq_default(device) {
                // The daemon default input.
                String::new()
            } else {
                device.to_string()
            }
        }
        lumina_config::schema::AudioSource::None => "@DEFAULT_MONITOR@".into(),
    }
}

/// The config's notion of "default", tolerant of blanks.
fn device_eq_default(device: &str) -> bool {
    device.trim().is_empty() || device.trim().eq_ignore_ascii_case("default")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_capture_targets_monitor_sources() {
        assert_eq!(
            pulse_source_for(lumina_config::schema::AudioSource::System, "default"),
            "@DEFAULT_MONITOR@"
        );
        assert_eq!(
            pulse_source_for(
                lumina_config::schema::AudioSource::System,
                "alsa_out.pci-0000"
            ),
            "alsa_out.pci-0000.monitor"
        );
        assert_eq!(
            pulse_source_for(
                lumina_config::schema::AudioSource::System,
                "alsa_out.monitor"
            ),
            "alsa_out.monitor"
        );
    }

    #[test]
    fn microphone_capture_uses_sources_verbatim() {
        assert_eq!(
            pulse_source_for(lumina_config::schema::AudioSource::Microphone, "bluez_mic"),
            "bluez_mic"
        );
        assert_eq!(
            pulse_source_for(lumina_config::schema::AudioSource::Microphone, "default"),
            ""
        );
    }

    #[test]
    fn spawn_refuses_disabled_source_correctly() {
        // A None source cannot open a stream; the worker reports it.
        let (writer, _reader) = crate::ring::RingBuffer::new(4096).split();
        let request = CaptureRequest {
            backend: lumina_config::schema::AudioBackend::Pulse,
            source: lumina_config::schema::AudioSource::None,
            device: "default".into(),
        };
        // On windows the pulse backend build never links libpulse, so this
        // test only asserts the plumbing where the backend exists.
        if cfg!(target_os = "linux") {
            let running = spawn(&request, writer).expect("spawn returns a worker");
            match running.wait_ready(std::time::Duration::from_secs(2)) {
                Err(super::super::CaptureError::DeviceUnavailable(msg)) => {
                    assert!(msg.contains("disabled"), "message: {msg}");
                }
                other => panic!("expected DeviceUnavailable, got {other:?}"),
            }
        }
    }
}
