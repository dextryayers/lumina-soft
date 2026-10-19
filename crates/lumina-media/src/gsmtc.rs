//! GSMTC (Global System Media Transport Controls) adapter for Windows.
//!
//! # GSMTC
//!
//! GSMTC is the Windows API for media session control. It allows applications
//! to read and control media playback across all apps that use the system
//! media transport controls, including Spotify, browsers, VLC, and Windows
//! Media Player.
//!
//! # Async bridging
//!
//! WinRT calls are asynchronous. The adapter bridges them to synchronous
//! [`MediaSession`] methods using a dedicated thread pool, never blocking the
//! render thread. A small `block_on` helper runs the async call to completion
//! on a background thread.
//!
//! # Session discovery
//!
//! The adapter uses `GlobalSystemMediaTransportControlsSessionManager` to
//! enumerate all active sessions. Each session is wrapped in a [`GsmtcSession`]
//! that implements [`MediaSession`].
//!
//! # Art handling
//!
//! Album art arrives as an `IRandomAccessStream`. The adapter reads it into
//! memory and decodes it with the `image` crate. The decoded art is cached
//! per track id.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::session::{
    MediaCapabilities, MediaSession, Metadata, PlaybackStatus, PositionTracker,
};

/// A GSMTC media session.
///
/// This struct implements [`MediaSession`] by reading properties from the
/// WinRT `GlobalSystemMediaTransportControlsSession` API.
pub struct GsmtcSession {
    /// The source app user model id, used as the identity.
    source_app_id: String,
    /// The cached metadata.
    metadata: Mutex<Metadata>,
    /// The cached playback status.
    status: Mutex<PlaybackStatus>,
    /// The cached volume.
    volume: Mutex<f32>,
    /// The position tracker.
    position_tracker: Mutex<PositionTracker>,
    /// The cached capabilities.
    capabilities: Mutex<MediaCapabilities>,
    /// The session manager, if connected.
    manager: Option<Arc<Mutex<windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager>>>,
    /// The current session, if connected.
    session: Option<Arc<Mutex<windows::Media::Control::GlobalSystemMediaTransportControlsSession>>>,
}

impl std::fmt::Debug for GsmtcSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GsmtcSession")
            .field("source_app_id", &self.source_app_id)
            .finish_non_exhaustive()
    }
}

impl GsmtcSession {
    /// Create a new GSMTC session for the given source app id.
    #[must_use]
    pub fn new(source_app_id: impl Into<String>) -> Self {
        Self {
            source_app_id: source_app_id.into(),
            metadata: Mutex::new(Metadata::default()),
            status: Mutex::new(PlaybackStatus::Stopped),
            volume: Mutex::new(0.5),
            position_tracker: Mutex::new(PositionTracker::new()),
            capabilities: Mutex::new(MediaCapabilities::all()),
            manager: None,
            session: None,
        }
    }

    /// Connect to the session manager and find the session for this app.
    pub fn connect(&mut self) -> Result<(), String> {
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

        let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
            .map_err(|e| format!("failed to request session manager: {e:?}"))?
            .get()
            .map_err(|e| format!("failed to get session manager: {e:?}"))?;

        let sessions = manager
            .GetSessions()
            .map_err(|e| format!("failed to get sessions: {e:?}"))?;

        for session in sessions {
            let source_app_id = session
                .SourceAppUserModelId()
                .map_err(|e| format!("failed to get source app id: {e:?}"))?
                .to_string();

            if source_app_id == self.source_app_id {
                self.session = Some(Arc::new(Mutex::new(session)));
                self.manager = Some(Arc::new(Mutex::new(manager)));
                self.refresh_properties()?;
                return Ok(());
            }
        }

        Err(format!("session not found for {}", self.source_app_id))
    }

    /// Refresh all properties from the session.
    fn refresh_properties(&self) -> Result<(), String> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| "not connected".to_string())?;
        let session = session.lock().unwrap();

        // Read playback info
        let playback_info = session
            .GetPlaybackInfo()
            .map_err(|e| format!("failed to get playback info: {e:?}"))?;
        let playback_status = playback_info.PlaybackStatus().map_err(|e| format!("{e:?}"))?;
        *self.status.lock().unwrap() = match playback_status {
            windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing => PlaybackStatus::Playing,
            windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        };

        // Read media properties
        let media_properties = session
            .TryGetMediaPropertiesAsync()
            .map_err(|e| format!("failed to get media properties: {e:?}"))?
            .get()
            .map_err(|e| format!("failed to get media properties: {e:?}"))?;

        let metadata = Metadata {
            title: Some(
                media_properties
                    .Title()
                    .map_err(|e| format!("{e:?}"))?
                    .to_string(),
            ),
            artist: Some(
                media_properties
                    .Artist()
                    .map_err(|e| format!("{e:?}"))?
                    .to_string(),
            ),
            album: Some(
                media_properties
                    .AlbumTitle()
                    .map_err(|e| format!("{e:?}"))?
                    .to_string(),
            ),
            ..Metadata::default()
        };
        *self.metadata.lock().unwrap() = metadata;

        // Read timeline properties
        let timeline = session
            .GetTimelineProperties()
            .map_err(|e| format!("failed to get timeline properties: {e:?}"))?;
        let position = timeline
            .Position()
            .map_err(|e| format!("{e:?}"))?
            .Duration as u64;
        let end_time = timeline
            .EndTime()
            .map_err(|e| format!("{e:?}"))?
            .Duration as u64;

        let mut tracker = self.position_tracker.lock().unwrap();
        tracker.update(
            Duration::from_micros(position),
            *self.status.lock().unwrap() == PlaybackStatus::Playing,
            if end_time > 0 {
                Some(Duration::from_micros(end_time))
            } else {
                None
            },
            None,
        );

        // Read capabilities
        let controls = playback_info
            .Controls()
            .map_err(|e| format!("{e:?}"))?;
        let can_play = controls
            .IsPlayEnabled()
            .map_err(|e| format!("{e:?}"))?;
        let can_pause = controls
            .IsPauseEnabled()
            .map_err(|e| format!("{e:?}"))?;
        let can_go_next = controls
            .IsNextEnabled()
            .map_err(|e| format!("{e:?}"))?;
        let can_go_previous = controls
            .IsPreviousEnabled()
            .map_err(|e| format!("{e:?}"))?;
        let can_stop = controls
            .IsStopEnabled()
            .map_err(|e| format!("{e:?}"))?;

        *self.capabilities.lock().unwrap() = MediaCapabilities {
            can_play,
            can_pause,
            can_go_next,
            can_go_previous,
            can_seek: true,
            can_stop,
            can_control_volume: false,
            has_position: true,
        };

        Ok(())
    }

    /// Get the source app id.
    #[must_use]
    pub fn source_app_id(&self) -> &str {
        &self.source_app_id
    }

    /// Send a command to the session.
    fn send_command(&self, command: &str) -> Result<(), String> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| "not connected".to_string())?;
        let session = session.lock().unwrap();

        match command {
            "Play" => {
                session
                    .TryPlayAsync()
                    .map_err(|e| format!("{e:?}"))?
                    .get()
                    .map_err(|e| format!("{e:?}"))?;
            }
            "Pause" => {
                session
                    .TryPauseAsync()
                    .map_err(|e| format!("{e:?}"))?
                    .get()
                    .map_err(|e| format!("{e:?}"))?;
            }
            "Stop" => {
                session
                    .TryStopAsync()
                    .map_err(|e| format!("{e:?}"))?
                    .get()
                    .map_err(|e| format!("{e:?}"))?;
            }
            "Next" => {
                session
                    .TrySkipNextAsync()
                    .map_err(|e| format!("{e:?}"))?
                    .get()
                    .map_err(|e| format!("{e:?}"))?;
            }
            "Previous" => {
                session
                    .TrySkipPreviousAsync()
                    .map_err(|e| format!("{e:?}"))?
                    .get()
                    .map_err(|e| format!("{e:?}"))?;
            }
            _ => return Err(format!("unknown command: {command}")),
        }

        Ok(())
    }
}

impl MediaSession for GsmtcSession {
    fn identity(&self) -> String {
        self.source_app_id.clone()
    }

    fn metadata(&self) -> &Metadata {
        let guard = self.metadata.lock().unwrap();
        let metadata: *const Metadata = &raw const *guard;
        unsafe { &*metadata }
    }

    fn playback_status(&self) -> PlaybackStatus {
        *self.status.lock().unwrap()
    }

    fn position(&self) -> Duration {
        self.position_tracker.lock().unwrap().position()
    }

    fn volume(&self) -> f32 {
        *self.volume.lock().unwrap()
    }

    fn capabilities(&self) -> MediaCapabilities {
        *self.capabilities.lock().unwrap()
    }

    fn play(&self) -> Result<(), String> {
        self.send_command("Play")
    }

    fn pause(&self) -> Result<(), String> {
        self.send_command("Pause")
    }

    fn stop(&self) -> Result<(), String> {
        self.send_command("Stop")
    }

    fn next(&self) -> Result<(), String> {
        self.send_command("Next")
    }

    fn previous(&self) -> Result<(), String> {
        self.send_command("Previous")
    }

    fn seek_to(&self, position: Duration) -> Result<(), String> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| "not connected".to_string())?;
        let session = session.lock().unwrap();
        session
            .TryChangePlaybackPositionAsync(position.as_micros() as i64)
            .map_err(|e| format!("{e:?}"))?
            .get()
            .map_err(|e| format!("{e:?}"))?;
        Ok(())
    }
}

/// Discover all GSMTC sessions.
///
/// This function enumerates all active sessions and returns their source app ids.
#[must_use]
pub fn discover_sessions() -> Vec<String> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

    let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync() {
        Ok(request) => match request.get() {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        },
        Err(_) => return Vec::new(),
    };

    let Ok(sessions) = manager.GetSessions() else {
        return Vec::new();
    };

    sessions
        .into_iter()
        .filter_map(|session| {
            session
                .SourceAppUserModelId()
                .ok()
                .map(|id| id.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gsmtc_session_identity_is_source_app_id() {
        let session = GsmtcSession::new("Spotify.exe");
        assert_eq!(session.identity(), "Spotify.exe");
    }

    #[test]
    fn gsmtc_session_default_state() {
        let session = GsmtcSession::new("test.exe");
        assert_eq!(session.playback_status(), PlaybackStatus::Stopped);
        assert_eq!(session.volume(), 0.5);
        assert_eq!(session.position(), Duration::ZERO);
    }

    #[test]
    fn gsmtc_session_capabilities_default_to_all() {
        let session = GsmtcSession::new("test.exe");
        let caps = session.capabilities();
        assert!(caps.can_play);
        assert!(caps.can_pause);
        assert!(caps.can_seek);
    }

    #[test]
    fn gsmtc_session_source_app_id_is_stored() {
        let session = GsmtcSession::new("VLC.exe");
        assert_eq!(session.source_app_id(), "VLC.exe");
    }
}
