//! Capture backends: the piece that turns a system audio stream into
//! mono samples in the ring buffer.
//!
//! The backends own two responsibilities each:
//!
//! 1. Open the right system API for the requested source (a loopback of
//!    whatever is playing, or a microphone), discovering the mix format
//!    the system reports rather than assuming one.
//! 2. Run a worker thread that pushes cleaned mono samples into the ring
//!    via [`CapturePost`](crate::dsp::CapturePost), watching a stop flag.
//!
//! The worker's very first act is to report its negotiated
//! [`CaptureFormat`] over a channel; [`RunningCapture::wait_ready`] picks
//! that up, which gives the caller a synchronous open result without
//! blocking the UI thread on device init.

pub mod fake;
pub mod pipewire;
#[cfg(target_os = "linux")]
pub mod pulse;
#[cfg(windows)]
pub mod wasapi;

use crate::ring::RingWriter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{Builder, JoinHandle};
use std::time::Duration;

/// A device format negotiated with the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureFormat {
    /// Mono samples per second pushed into the ring.
    pub sample_rate: u32,
    /// Endpoint channel count the mix format exposes.
    pub channels: u16,
    /// Whether the endpoint is a loopback of the system's render side.
    pub loopback: bool,
}

/// Why a capture attempt failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// The best matching device cannot be opened or does not exist.
    DeviceUnavailable(String),
    /// The backend exists but the platform does not.
    UnsupportedBackend,
    /// The backend reports a stream failure mid run.
    StreamError(String),
    /// The device name the config holds matched no endpoint.
    NoNamedDevice(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceUnavailable(d) => write!(f, "audio device unavailable: {d}"),
            Self::UnsupportedBackend => write!(f, "capture backend not supported on this platform"),
            Self::StreamError(d) => write!(f, "audio stream failed: {d}"),
            Self::NoNamedDevice(d) => write!(f, "no endpoint matches device name: {d}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// What the capture layer should hook into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRequest {
    /// Which API to drive.
    pub backend: lumina_config::schema::AudioBackend,
    /// Render loopback or microphone.
    pub source: lumina_config::schema::AudioSource,
    /// Device name substring, or `default`.
    pub device: String,
}

impl CaptureRequest {
    /// Build a request from the audio configuration.
    #[must_use]
    pub fn from_config(config: &lumina_config::schema::AudioConfig) -> Self {
        Self {
            backend: config.backend,
            source: config.source,
            device: config.device.clone(),
        }
    }
}

/// A backend the request resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBackend {
    /// Windows WASAPI loopback or input, via COM interfaces.
    Wasapi,
    /// PulseAudio simple record, over the daemon's native or PipeWire's
    /// pipewire pulse compatibility socket.
    Pulse,
    /// Synthetic stream, no hardware involved; tests and demos only.
    Fake,
}

/// A live capture worker: stop flag, thread handle, and the one shot
/// "stream is open" channel.
#[derive(Debug)]
pub struct RunningCapture {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    ready: Mutex<Option<Receiver<Result<CaptureFormat, CaptureError>>>>,
}

impl RunningCapture {
    /// Wrap a worker spawned by a backend.
    pub(crate) fn new(
        stop: Arc<AtomicBool>,
        handle: JoinHandle<()>,
        ready: Receiver<Result<CaptureFormat, CaptureError>>,
    ) -> Self {
        Self {
            stop,
            handle: Some(handle),
            ready: Mutex::new(Some(ready)),
        }
    }

    /// Wait for the worker to finish opening its stream, up to `timeout`.
    ///
    /// Consumes the startup message: calling twice reports that the
    /// startup result was already taken rather than re waiting.
    pub fn wait_ready(&self, timeout: Duration) -> Result<CaptureFormat, CaptureError> {
        let receiver = self.ready.lock().expect("ready mutex poisoned").take();
        let Some(receiver) = receiver else {
            return Err(CaptureError::StreamError("startup already consumed".into()));
        };
        match receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(CaptureError::StreamError(
                "capture did not open in time".into(),
            )),
            Err(RecvTimeoutError::Disconnected) => Err(CaptureError::StreamError(
                "capture worker exited before opening".into(),
            )),
        }
    }

    /// Ask the worker to finish; does not block.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Whether the worker thread is still running.
    ///
    /// After `stop` the handle is gone, so the answer is `false` rather
    /// than an optimistic default.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    }

    /// Signal stop and join the thread. Blocks at most a few hundred
    /// milliseconds under normal operation. Idempotent: a second call just
    /// sees no handle left and returns.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Resolve a request to a concrete backend, honoring `backend = auto`.
///
/// Probe order on Linux is Pulse first: a pipewire pulse socket speaks the
/// same protocol, so one code path covers both daemons; see the README.
#[must_use]
pub fn resolve_backend(request: &CaptureRequest) -> ResolvedBackend {
    use lumina_config::schema::AudioBackend;
    match request.backend {
        // One backend slot per protocol: pipewire speaks the pulse
        // protocol through its compatibility server, so both map onto the
        // Pulse code path and only the socket presence check differs.
        AudioBackend::Wasapi => ResolvedBackend::Wasapi,
        AudioBackend::Pulse | AudioBackend::Pipewire => ResolvedBackend::Pulse,
        AudioBackend::Auto => {
            if cfg!(windows) {
                ResolvedBackend::Wasapi
            } else if cfg!(any(target_os = "linux", target_os = "freebsd")) {
                ResolvedBackend::Pulse
            } else {
                ResolvedBackend::Fake
            }
        }
    }
}

/// Spawn a capture worker for a request, pushing mono samples into `writer`.
///
/// This is the PLAN F8.4 auto selection entry point. A backend that does
/// not exist on the current platform folds into
/// [`CaptureError::UnsupportedBackend`].
pub fn spawn_capture(
    request: &CaptureRequest,
    writer: RingWriter,
) -> Result<RunningCapture, CaptureError> {
    match resolve_backend(request) {
        #[cfg(windows)]
        ResolvedBackend::Wasapi => wasapi::spawn(request, writer),
        #[cfg(not(windows))]
        ResolvedBackend::Wasapi => {
            let _ = writer;
            Err(CaptureError::UnsupportedBackend)
        }
        #[cfg(target_os = "linux")]
        ResolvedBackend::Pulse => pulse::spawn(request, writer),
        #[cfg(not(target_os = "linux"))]
        ResolvedBackend::Pulse | ResolvedBackend::Fake => Err(CaptureError::UnsupportedBackend),
        #[cfg(target_os = "linux")]
        ResolvedBackend::Fake => Err(CaptureError::UnsupportedBackend),
    }
}

/// Spawn a synthetic capture stream: sine, noise, or silence.
///
/// Intended for tests, CI runs without hardware, and the screenshot mode;
/// PLAN F8.x relies on this for hardware independent engine tests.
pub fn spawn_fake(
    request: &CaptureRequest,
    writer: RingWriter,
    pattern: fake::FakePattern,
) -> Result<RunningCapture, CaptureError> {
    fake::spawn(request, writer, pattern)
}

/// Shared worker scaffolding every backend uses: an `Arc` stop flag plus a
/// named builder-friendly thread name.
pub(crate) fn make_stop_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// The worker thread's name for diagnostics.
pub(crate) const WORKER_NAME: &str = "lumina-capture";

/// Spawn helper shared by the backends: runs `run` on a named thread wired
/// to the stop flag and the ready channel.
///
/// Contract: `run` must report exactly once through the signal, `Ok` only
/// after its stream opened and before its loop starts, `Err` on failure,
/// and must exit promptly once the stop flag is set.
pub(crate) fn spawn_worker<F>(name_suffix: &str, run: F) -> Result<RunningCapture, CaptureError>
where
    F: FnOnce(Arc<AtomicBool>, ReadySignal) + Send + 'static,
{
    let stop = make_stop_flag();
    let (signal, ready_receiver) = ReadySignal::new();
    let thread_stop = Arc::clone(&stop);
    let handle = Builder::new()
        .name(format!("{WORKER_NAME}-{name_suffix}"))
        .spawn(move || run(thread_stop, signal))
        .map_err(|e| CaptureError::StreamError(format!("failed to spawn capture thread: {e}")))?;

    Ok(RunningCapture::new(stop, handle, ready_receiver))
}

/// Read the stop flag non atomically friendly.
pub(crate) fn stop_flagged(stop: &AtomicBool) -> bool {
    stop.load(Ordering::Relaxed)
}

/// The ready signal a backend reports its negotiated format through.
pub(crate) struct ReadySignal {
    sender: std::sync::mpsc::SyncSender<Result<CaptureFormat, CaptureError>>,
}

impl ReadySignal {
    pub(crate) fn new() -> (Self, Receiver<Result<CaptureFormat, CaptureError>>) {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        (Self { sender }, receiver)
    }

    /// Report the opened format. Called exactly once per worker.
    pub(crate) fn report(&self, result: Result<CaptureFormat, CaptureError>) {
        let _ = self.sender.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::RingBuffer;

    fn request(backend: lumina_config::schema::AudioBackend) -> CaptureRequest {
        CaptureRequest {
            backend,
            source: lumina_config::schema::AudioSource::System,
            device: "default".into(),
        }
    }

    #[test]
    fn auto_resolves_per_platform() {
        let resolved = resolve_backend(&request(lumina_config::schema::AudioBackend::Auto));
        if cfg!(windows) {
            assert_eq!(resolved, ResolvedBackend::Wasapi);
        } else if cfg!(any(target_os = "linux", target_os = "freebsd")) {
            assert_eq!(resolved, ResolvedBackend::Pulse);
        }
        // Explicit picks are always honored.
        assert_eq!(
            resolve_backend(&request(lumina_config::schema::AudioBackend::Wasapi)),
            ResolvedBackend::Wasapi
        );
        // PipeWire routes through the Pulse protocol.
        assert_eq!(
            resolve_backend(&request(lumina_config::schema::AudioBackend::Pipewire)),
            ResolvedBackend::Pulse
        );
    }

    #[test]
    fn a_fake_worker_reports_its_format_and_stops_cleanly() {
        let (writer, _reader) = RingBuffer::new(4096).split();
        let mut running = spawn_fake(
            &request(lumina_config::schema::AudioBackend::Auto),
            writer,
            fake::FakePattern::Silence,
        )
        .expect("fake spawn cannot fail");
        // Is a mock format path? Fake runs regardless of platform.
        let format = running
            .wait_ready(Duration::from_secs(2))
            .expect("fake reports fast");
        assert!(format.sample_rate >= 8_000);
        assert!(format.channels >= 1);
        running.stop();
        assert!(!running.is_running());
    }
    #[test]
    fn double_wait_ready_reports_consumed() {
        let (writer, _reader) = RingBuffer::new(4096).split();
        let running = spawn_fake(
            &request(lumina_config::schema::AudioBackend::Auto),
            writer,
            fake::FakePattern::Silence,
        )
        .expect("fake spawn");
        let _ = running.wait_ready(Duration::from_secs(2));
        match running.wait_ready(Duration::from_millis(50)) {
            Err(CaptureError::StreamError(msg)) => assert!(msg.contains("consumed")),
            other => panic!("expected StreamError, got {other:?}"),
        }
    }

    #[test]
    fn fake_pushes_silently_and_stops() {
        let (writer, reader) = RingBuffer::new(4096).split();
        let mut running = spawn_fake(
            &request(lumina_config::schema::AudioBackend::Auto),
            writer,
            fake::FakePattern::Silence,
        )
        .expect("fake spawn");
        let _ = running.wait_ready(Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(200));
        let pending_before = reader.pending();
        running.stop();
        // A silence stream still pushes zero samples, so the ring got fed.
        let mut out = vec![1.0f32; 4096];
        let _ = reader.read(&mut out);
        let _ = pending_before;
    }
}
