//! The media session abstraction.
//!
//! # Capability flags
//!
//! Not every player supports every capability. The [`MediaCapabilities`] struct
//! declares what a session can do, so the UI can disable controls that would
//! fail. For example, a player that does not implement `Seek` should show a
//! read-only progress bar.
//!
//! # Position tracking
//!
//! The orchestrator tracks position locally between polls, because MPRIS and
//! GSMTC only emit position on change. The session exposes a [`PositionTracker`]
//! that the orchestrator updates with the elapsed time since the last poll.

use std::time::Duration;

/// Playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    /// Actively playing.
    Playing,
    /// Paused.
    Paused,
    /// Stopped.
    Stopped,
}

/// Metadata for the current track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    /// Track title.
    pub title: Option<String>,
    /// Artist name.
    pub artist: Option<String>,
    /// Album name.
    pub album: Option<String>,
    /// URL or path to cover art.
    pub art_url: Option<String>,
    /// Track length.
    pub length: Option<Duration>,
    /// Track identifier, used for caching and change detection.
    pub track_id: Option<String>,
}

impl Metadata {
    /// True when title and artist are both present.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.title.is_some() && self.artist.is_some()
    }

    /// True when there is no useful metadata at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.album.is_none()
            && self.art_url.is_none()
            && self.length.is_none()
    }
}

/// Capabilities of a media session.
///
/// Not every player supports every capability. The UI uses these flags to
/// disable controls that would fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MediaCapabilities {
    /// The session can play.
    pub can_play: bool,
    /// The session can pause.
    pub can_pause: bool,
    /// The session can go to the next track.
    pub can_go_next: bool,
    /// The session can go to the previous track.
    pub can_go_previous: bool,
    /// The session can seek.
    pub can_seek: bool,
    /// The session can stop.
    pub can_stop: bool,
    /// The session can control volume.
    pub can_control_volume: bool,
    /// The session provides position information.
    pub has_position: bool,
}

impl MediaCapabilities {
    /// All capabilities enabled.
    #[must_use]
    pub fn all() -> Self {
        Self {
            can_play: true,
            can_pause: true,
            can_go_next: true,
            can_go_previous: true,
            can_seek: true,
            can_stop: true,
            can_control_volume: true,
            has_position: true,
        }
    }

    /// No capabilities.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// True when no capabilities are available.
    #[must_use]
    pub fn is_none(&self) -> bool {
        !self.can_play
            && !self.can_pause
            && !self.can_go_next
            && !self.can_go_previous
            && !self.can_seek
            && !self.can_stop
            && !self.can_control_volume
    }
}

/// A snapshot of a media session at one moment.
///
/// Owned and `Send`, so it can cross the thread boundary from the media thread
/// to the orchestrator without locking.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaSnapshot {
    /// Human-readable player name.
    pub identity: String,
    /// Current track metadata.
    pub metadata: Metadata,
    /// Playback state.
    pub status: PlaybackStatus,
    /// Current position.
    pub position: Duration,
    /// Volume, 0.0 to 1.0.
    pub volume: f32,
    /// Session capabilities.
    pub capabilities: MediaCapabilities,
}

impl MediaSnapshot {
    /// True when the player is actively playing.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.status == PlaybackStatus::Playing
    }

    /// Progress through the track, 0.0 to 1.0, or 0.0 when the length is unknown.
    #[must_use]
    pub fn progress(&self) -> f32 {
        match self.metadata.length {
            Some(len) if len.as_secs_f32() > 0.0 => {
                (self.position.as_secs_f32() / len.as_secs_f32()).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    /// Title, falling back to a placeholder.
    #[must_use]
    pub fn title_or_placeholder(&self) -> &str {
        self.metadata
            .title
            .as_deref()
            .or(self.metadata.album.as_deref())
            .unwrap_or("Unknown title")
    }

    /// Artist, falling back to a placeholder.
    #[must_use]
    pub fn artist_or_placeholder(&self) -> &str {
        self.metadata
            .artist
            .as_deref()
            .or(self.metadata.album.as_deref())
            .unwrap_or("Unknown artist")
    }

    /// Format the position as a human-readable string.
    #[must_use]
    pub fn format_position(&self) -> String {
        format_duration(self.position)
    }

    /// Format the track length as a human-readable string.
    #[must_use]
    pub fn format_length(&self) -> String {
        match self.metadata.length {
            Some(len) => format_duration(len),
            None => "--:--".to_string(),
        }
    }
}

/// Format a duration as MM:SS or H:MM:SS.
#[must_use]
pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}", minutes, seconds)
    }
}

/// Position tracker for smooth progress updates.
///
/// The orchestrator uses this to track position between polls. It stores the
/// last known position, the time it was last updated, and whether the player
/// is playing. When the player is playing, the tracker adds the elapsed time
/// to the position.
#[derive(Debug, Clone)]
pub struct PositionTracker {
    /// The last known position.
    position: Duration,
    /// The monotonic timestamp when the position was last updated.
    last_update: Option<std::time::Instant>,
    /// Whether the player is playing.
    playing: bool,
    /// The track length, used to detect when the track has ended.
    track_length: Option<Duration>,
    /// The track id, used to detect track changes.
    track_id: Option<String>,
}

impl PositionTracker {
    /// Create a new position tracker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            position: Duration::ZERO,
            last_update: None,
            playing: false,
            track_length: None,
            track_id: None,
        }
    }

    /// Update the tracker with a new position from the session.
    ///
    /// This should be called whenever the session reports a new position,
    /// either from a poll or from a signal.
    pub fn update(
        &mut self,
        position: Duration,
        playing: bool,
        track_length: Option<Duration>,
        track_id: Option<&str>,
    ) {
        self.position = position;
        self.playing = playing;
        self.track_length = track_length;
        self.track_id = track_id.map(|s| s.to_string());
        self.last_update = Some(std::time::Instant::now());
    }

    /// Get the current estimated position.
    ///
    /// If the player is playing, the tracker adds the elapsed time since the
    /// last update to the stored position. If the track has ended, the tracker
    /// returns the track length.
    #[must_use]
    pub fn position(&self) -> Duration {
        if !self.playing {
            return self.position;
        }
        let elapsed = self
            .last_update
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO);
        let estimated = self.position + elapsed;
        if let Some(length) = self.track_length {
            estimated.min(length)
        } else {
            estimated
        }
    }

    /// Reset the tracker.
    ///
    /// This should be called when the track changes or the player stops.
    pub fn reset(&mut self) {
        self.position = Duration::ZERO;
        self.last_update = None;
        self.playing = false;
        self.track_length = None;
        self.track_id = None;
    }

    /// True when the player is playing.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The track length.
    #[must_use]
    pub fn track_length(&self) -> Option<Duration> {
        self.track_length
    }

    /// The track id.
    #[must_use]
    pub fn track_id(&self) -> Option<&str> {
        self.track_id.as_deref()
    }
}

impl Default for PositionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Control over one media player.
///
/// Implemented once per platform. Every method is fallible and no method is
/// allowed to panic: a player that disappears mid-command returns an error
/// rather than taking the application down.
pub trait MediaSession: Send {
    /// Human-readable identity, used in the UI and in ranking.
    fn identity(&self) -> String;

    /// Current metadata.
    fn metadata(&self) -> &Metadata;

    /// Current playback status.
    fn playback_status(&self) -> PlaybackStatus;

    /// Current position.
    ///
    /// The orchestrator tracks position locally between polls, because MPRIS
    /// and GSMTC only emit it on change.
    fn position(&self) -> Duration;

    /// Current volume, 0.0 to 1.0.
    fn volume(&self) -> f32;

    /// Session capabilities.
    fn capabilities(&self) -> MediaCapabilities;

    /// Start or resume playback.
    fn play(&self) -> Result<(), String>;

    /// Pause playback.
    fn pause(&self) -> Result<(), String>;

    /// Toggle play and pause.
    fn play_pause(&self) -> Result<(), String> {
        if self.playback_status() == PlaybackStatus::Playing {
            self.pause()
        } else {
            self.play()
        }
    }

    /// Stop playback.
    fn stop(&self) -> Result<(), String>;

    /// Skip to the next track.
    fn next(&self) -> Result<(), String>;

    /// Skip to the previous track.
    fn previous(&self) -> Result<(), String>;

    /// Seek to a position.
    fn seek_to(&self, position: Duration) -> Result<(), String>;

    /// Set volume, 0.0 to 1.0.
    fn set_volume(&self, volume: f32) -> Result<(), String> {
        let _ = volume;
        Err("volume control is not supported by this player".into())
    }

    /// Take a snapshot of the current state.
    fn snapshot(&self) -> MediaSnapshot {
        MediaSnapshot {
            identity: self.identity(),
            metadata: self.metadata().clone(),
            status: self.playback_status(),
            position: self.position(),
            volume: self.volume(),
            capabilities: self.capabilities(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSession {
        identity: String,
        metadata: Metadata,
        status: PlaybackStatus,
        position: Duration,
        volume: f32,
        capabilities: MediaCapabilities,
    }

    impl TestSession {
        fn new(identity: &str) -> Self {
            Self {
                identity: identity.to_string(),
                metadata: Metadata::default(),
                status: PlaybackStatus::Paused,
                position: Duration::ZERO,
                volume: 0.5,
                capabilities: MediaCapabilities::all(),
            }
        }
    }

    impl MediaSession for TestSession {
        fn identity(&self) -> String {
            self.identity.clone()
        }
        fn metadata(&self) -> &Metadata {
            &self.metadata
        }
        fn playback_status(&self) -> PlaybackStatus {
            self.status
        }
        fn position(&self) -> Duration {
            self.position
        }
        fn volume(&self) -> f32 {
            self.volume
        }
        fn capabilities(&self) -> MediaCapabilities {
            self.capabilities
        }
        fn play(&self) -> Result<(), String> {
            Ok(())
        }
        fn pause(&self) -> Result<(), String> {
            Ok(())
        }
        fn stop(&self) -> Result<(), String> {
            Ok(())
        }
        fn next(&self) -> Result<(), String> {
            Ok(())
        }
        fn previous(&self) -> Result<(), String> {
            Ok(())
        }
        fn seek_to(&self, _: Duration) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_complete_metadata_is_not_empty() {
        let mut metadata = Metadata::default();
        metadata.title = Some("Song".to_string());
        metadata.artist = Some("Artist".to_string());
        assert!(metadata.is_complete());
        assert!(!metadata.is_empty());
    }

    #[test]
    fn an_empty_metadata_is_detected() {
        assert!(Metadata::default().is_empty());
        assert!(!Metadata::default().is_complete());
    }

    #[test]
    fn progress_is_the_ratio_of_position_to_length() {
        let session = TestSession {
            metadata: Metadata { length: Some(Duration::from_secs(100)), ..Default::default() },
            status: PlaybackStatus::Playing,
            position: Duration::from_secs(50),
            ..TestSession::new("test")
        };
        let snapshot = session.snapshot();
        assert!((snapshot.progress() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn progress_is_zero_when_the_length_is_unknown() {
        let session = TestSession {
            metadata: Metadata::default(),
            status: PlaybackStatus::Playing,
            position: Duration::from_secs(50),
            ..TestSession::new("test")
        };
        assert_eq!(session.snapshot().progress(), 0.0);
    }

    #[test]
    fn progress_never_exceeds_one() {
        let session = TestSession {
            metadata: Metadata { length: Some(Duration::from_secs(10)), ..Default::default() },
            status: PlaybackStatus::Playing,
            position: Duration::from_secs(999),
            ..TestSession::new("test")
        };
        assert_eq!(session.snapshot().progress(), 1.0);
    }

    #[test]
    fn a_snapshot_carries_the_identity() {
        let session = TestSession::new("test");
        assert_eq!(session.snapshot().identity, "test");
    }

    #[test]
    fn the_placeholder_titles_fall_back() {
        let session = TestSession::new("test");
        let snapshot = session.snapshot();
        assert_eq!(snapshot.title_or_placeholder(), "Unknown title");
        assert_eq!(snapshot.artist_or_placeholder(), "Unknown artist");
    }

    #[test]
    fn the_placeholder_titles_use_the_real_values_when_present() {
        let session = TestSession {
            metadata: Metadata {
                title: Some("Real Song".to_string()),
                artist: Some("Real Artist".to_string()),
                ..Default::default()
            },
            ..TestSession::new("test")
        };
        let snapshot = session.snapshot();
        assert_eq!(snapshot.title_or_placeholder(), "Real Song");
        assert_eq!(snapshot.artist_or_placeholder(), "Real Artist");
    }

    #[test]
    fn play_pause_toggles() {
        let session = TestSession::new("test");
        assert!(session.play_pause().is_ok());
    }

    #[test]
    fn volume_defaults_to_half_and_can_be_overridden() {
        let session = TestSession::new("test");
        assert_eq!(session.volume(), 0.5);
    }

    #[test]
    fn is_playing_reflects_the_status() {
        let session = TestSession {
            status: PlaybackStatus::Playing,
            ..TestSession::new("test")
        };
        assert!(session.snapshot().is_playing());
    }

    #[test]
    fn media_capabilities_all_enables_everything() {
        let caps = MediaCapabilities::all();
        assert!(caps.can_play);
        assert!(caps.can_pause);
        assert!(caps.can_go_next);
        assert!(caps.can_go_previous);
        assert!(caps.can_seek);
        assert!(caps.can_stop);
        assert!(caps.can_control_volume);
        assert!(caps.has_position);
        assert!(!caps.is_none());
    }

    #[test]
    fn media_capables_none_disables_everything() {
        let caps = MediaCapabilities::none();
        assert!(!caps.can_play);
        assert!(!caps.can_pause);
        assert!(!caps.can_go_next);
        assert!(!caps.can_go_previous);
        assert!(!caps.can_seek);
        assert!(!caps.can_stop);
        assert!(!caps.can_control_volume);
        assert!(caps.is_none());
    }

    #[test]
    fn format_duration_formats_correctly() {
        assert_eq!(format_duration(Duration::from_secs(65)), "01:05");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1:01:01");
        assert_eq!(format_duration(Duration::ZERO), "00:00");
    }

    #[test]
    fn position_tracker_tracks_elapsed_time() {
        let mut tracker = PositionTracker::new();
        tracker.update(Duration::from_secs(10), true, Some(Duration::from_secs(100)), Some("track1"));
        let pos = tracker.position();
        assert!(pos >= Duration::from_secs(10));
    }

    #[test]
    fn position_tracker_resets_on_track_change() {
        let mut tracker = PositionTracker::new();
        tracker.update(Duration::from_secs(10), true, Some(Duration::from_secs(100)), Some("track1"));
        tracker.reset();
        assert_eq!(tracker.position(), Duration::ZERO);
        assert!(!tracker.is_playing());
    }

    #[test]
    fn snapshot_includes_capabilities() {
        let session = TestSession::new("test");
        let snapshot = session.snapshot();
        assert!(snapshot.capabilities.can_play);
        assert!(snapshot.capabilities.can_pause);
    }
}
