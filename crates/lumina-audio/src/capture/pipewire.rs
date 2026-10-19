//! PipeWire capture.
//!
//! # Why this module is thin
//!
//! PipeWire ships a PulseAudio compatibility server (`pipewire-pulse`):
//! a socket at `$XDG_RUNTIME_DIR/pulse/native` that speaks the exact
//! PulseAudio native protocol. Connecting to it through
//! [`super::pulse`] captures the loopback with zero additional native
//! dependencies, versus linking `libpipewire` whose private ABI churns
//! between versions. The README documents this choice.
//!
//! What this module adds over the raw pulse path is discovery and
//! failure glare: it verifies the compatibility socket exists before
//! handing the job over, so a misconfigured podman container or a flatpak
//! sandbox gets a precise error instead of PulseAudio's generic
//! "connection refused".
//!
//! On Windows this module still compiles (it is platform independent
//! glue) and its [`spawn`] reports
//! [`CaptureError::UnsupportedBackend`](super::CaptureError::UnsupportedBackend).

use super::{CaptureError, CaptureRequest, RunningCapture};

/// Where the pipewire pulse compatibility socket usually lives.
pub fn compat_socket_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(
        || std::path::PathBuf::from("/run/user/0"),
        std::path::PathBuf::from,
    );
    base.join("pulse")
}

/// Spawn a PipeWire capture via the pulse protocol path.
///
/// When the compatibility socket is missing, the error names it so the
/// user (and support) know exactly what to fix.
#[cfg_attr(not(target_os = "linux"), allow(clippy::needless_pass_by_value))]
pub fn spawn(
    request: &CaptureRequest,
    writer: crate::ring::RingWriter,
) -> Result<RunningCapture, CaptureError> {
    #[cfg(target_os = "linux")]
    {
        let socket = compat_socket_dir().join("native");
        if !socket.exists() {
            return Err(CaptureError::DeviceUnavailable(format!(
                "pipewire-pulse compatibility socket not found: {}",
                socket.display()
            )));
        }
        // Resolve through the same request: pipewire maps to pulse in
        // `resolve_backend`, so the Pulse daemon name check is satisfied
        // by the compatibility server.
        let mut through_pulse = request.clone();
        through_pulse.backend = lumina_config::schema::AudioBackend::Pulse;
        super::pulse::spawn(&through_pulse, writer)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = &request;
        let _ = writer;
        Err(CaptureError::UnsupportedBackend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_dir_is_under_xdg_runtime() {
        if std::env::var_os("XDG_RUNTIME_DIR").is_some() {
            let dir = compat_socket_dir();
            assert!(dir.ends_with("pulse"), "socket dir: {}", dir.display());
        }
    }
}
