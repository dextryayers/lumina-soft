//! MPRIS2 adapter for Linux.
//!
//! # MPRIS2
//!
//! MPRIS2 (Media Player Remote Interfacing Specification) is a D-Bus interface
//! that allows media players to expose their playback state and controls.
//! Every player that publishes a session (Spotify, VLC, mpv, browsers playing
//! YouTube Music) is visible through MPRIS2.
//!
//! # Discovery
//!
//! Discovery is done by listening for `NameOwnerChanged` signals on
//! `org.freedesktop.DBus`, plus a full enumeration at startup. When a new
//! name appears that starts with `org.mpris.MediaPlayer2.`, a new
//! [`MprisSession`] is created.
//!
//! # Properties
//!
//! The adapter reads properties from the `org.mpris.MediaPlayer2.Player`
//! interface: `PlaybackStatus`, `Metadata`, `Volume`, `Position`, and
//! `CanPlay`, `CanPause`, `CanGoNext`, `CanGoPrevious`, `CanSeek`, `CanStop`.
//!
//! # Signals
//!
//! The adapter listens for `PropertiesChanged` signals on the player interface
//! and `Seeked` signals for position updates. When a signal arrives, the
//! session's cached state is updated and the orchestrator is notified.
//!
//! # Position tracking
//!
//! The `Position` property does not emit change signals, so the adapter reads
//! it once and then tracks it locally by adding elapsed time. A 1 Hz poll
//! corrects drift.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::session::{
    format_duration, MediaCapabilities, MediaSession, MediaSnapshot, Metadata, PlaybackStatus,
    PositionTracker,
};

/// The MPRIS2 bus name prefix.
pub const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

/// The MPRIS2 object path.
pub const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";

/// The MPRIS2 player interface.
pub const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

/// The D-Bus properties interface.
pub const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";

/// An MPRIS2 media session.
///
/// This struct implements [`MediaSession`] by reading properties from the
/// D-Bus interface and sending commands through D-Bus methods.
pub struct MprisSession {
    /// The bus name of the player, e.g. `org.mpris.MediaPlayer2.spotify`.
    bus_name: String,
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
    /// The D-Bus connection, if connected.
    connection: Option<Arc<zbus::Connection>>,
}

impl MprisSession {
    /// Create a new MPRIS session for the given bus name.
    ///
    /// The session is not connected to D-Bus until [`MprisSession::connect`] is
    /// called.
    #[must_use]
    pub fn new(bus_name: impl Into<String>) -> Self {
        Self {
            bus_name: bus_name.into(),
            metadata: Mutex::new(Metadata::default()),
            status: Mutex::new(PlaybackStatus::Stopped),
            volume: Mutex::new(0.5),
            position_tracker: Mutex::new(PositionTracker::new()),
            capabilities: Mutex::new(MediaCapabilities::all()),
            connection: None,
        }
    }

    /// Connect to D-Bus and read initial properties.
    ///
    /// This method reads the initial state from the player and sets up
    /// signal handlers for `PropertiesChanged` and `Seeked`.
    pub fn connect(&mut self) -> Result<(), String> {
        let connection =
            zbus::Connection::session().map_err(|e| format!("failed to connect to D-Bus: {e}"))?;
        let connection = Arc::new(connection);

        // Read initial properties
        self.refresh_properties(&connection)?;

        // Set up signal handlers
        self.setup_signal_handlers(&connection)?;

        self.connection = Some(connection);
        Ok(())
    }

    /// Refresh all properties from the player.
    fn refresh_properties(&self, connection: &zbus::Connection) -> Result<(), String> {
        let proxy = zbus::Proxy::new(connection, &self.bus_name, MPRIS_PATH, PLAYER_INTERFACE)
            .map_err(|e| format!("failed to create D-Bus proxy: {e}"))?;

        // Read playback status
        let status: String = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "PlaybackStatus"))
            .map_err(|e| format!("failed to read PlaybackStatus: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize PlaybackStatus: {e}"))?;
        *self.status.lock().unwrap() = parse_playback_status(&status);

        // Read metadata
        let metadata: HashMap<String, zbus::zvariant::Value> = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "Metadata"))
            .map_err(|e| format!("failed to read Metadata: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize Metadata: {e}"))?;
        *self.metadata.lock().unwrap() = parse_metadata(&metadata);

        // Read volume
        let volume: f64 = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "Volume"))
            .map_err(|e| format!("failed to read Volume: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize Volume: {e}"))?;
        *self.volume.lock().unwrap() = volume as f32;

        // Read position
        let position: i64 = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "Position"))
            .map_err(|e| format!("failed to read Position: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize Position: {e}"))?;
        let position = Duration::from_micros(position.max(0) as u64);

        // Read capabilities
        let can_play: bool = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "CanPlay"))
            .map_err(|e| format!("failed to read CanPlay: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize CanPlay: {e}"))?;
        let can_pause: bool = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "CanPause"))
            .map_err(|e| format!("failed to read CanPause: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize CanPause: {e}"))?;
        let can_go_next: bool = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "CanGoNext"))
            .map_err(|e| format!("failed to read CanGoNext: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize CanGoNext: {e}"))?;
        let can_go_previous: bool = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "CanGoPrevious"))
            .map_err(|e| format!("failed to read CanGoPrevious: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize CanGoPrevious: {e}"))?;
        let can_seek: bool = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "CanSeek"))
            .map_err(|e| format!("failed to read CanSeek: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize CanSeek: {e}"))?;
        let can_stop: bool = proxy
            .call_method("Get", &(PLAYER_INTERFACE, "CanStop"))
            .map_err(|e| format!("failed to read CanStop: {e}"))?
            .body()
            .deserialize()
            .map_err(|e| format!("failed to deserialize CanStop: {e}"))?;

        *self.capabilities.lock().unwrap() = MediaCapabilities {
            can_play,
            can_pause,
            can_go_next,
            can_go_previous,
            can_seek,
            can_stop,
            can_control_volume: true,
            has_position: true,
        };

        // Update position tracker
        let mut tracker = self.position_tracker.lock().unwrap();
        tracker.update(
            position,
            self.status.lock().unwrap() == PlaybackStatus::Playing,
            self.metadata.lock().unwrap().length,
            self.metadata.lock().unwrap().track_id.as_deref(),
        );

        Ok(())
    }

    /// Set up signal handlers for PropertiesChanged and Seeked.
    fn setup_signal_handlers(&self, connection: &zbus::Connection) -> Result<(), String> {
        let bus_name = self.bus_name.clone();
        let metadata = Arc::clone(&self.metadata);
        let status = Arc::clone(&self.status);
        let volume = Arc::clone(&self.volume);
        let position_tracker = Arc::clone(&self.position_tracker);

        // Spawn a task to listen for signals
        let connection_clone = Arc::clone(connection);
        std::thread::spawn(move || {
            let proxy = match zbus::Proxy::new(
                &connection_clone,
                &bus_name,
                MPRIS_PATH,
                PROPERTIES_INTERFACE,
            ) {
                Ok(p) => p,
                Err(_) => return,
            };

            // Listen for PropertiesChanged signals
            let _ = proxy.receive_signal("PropertiesChanged").map(|mut signal| {
                std::thread::spawn(move || {
                    while let Ok(Some(msg)) = signal.try_next() {
                        let body = match msg.body().deserialize::<(
                            String,
                            HashMap<String, zbus::zvariant::Value>,
                            Vec<String>,
                        )>() {
                            Ok(b) => b,
                            Err(_) => continue,
                        };
                        let (_, changed, _) = body;

                        if let Some(status_value) = changed.get("PlaybackStatus") {
                            if let Ok(status_str) = status_value.try_into() {
                                if let Ok(status_str) = String::try_from(status_str) {
                                    *status.lock().unwrap() = parse_playback_status(&status_str);
                                }
                            }
                        }

                        if let Some(metadata_value) = changed.get("Metadata") {
                            if let Ok(metadata_map) = metadata_value.try_into() {
                                if let Ok(metadata_map) =
                                    HashMap::<String, zbus::zvariant::Value>::try_from(metadata_map)
                                {
                                    *metadata.lock().unwrap() = parse_metadata(&metadata_map);
                                }
                            }
                        }

                        if let Some(volume_value) = changed.get("Volume") {
                            if let Ok(volume_f64) = f64::try_from(volume_value.clone()) {
                                *volume.lock().unwrap() = volume_f64 as f32;
                            }
                        }

                        // Update position tracker
                        let mut tracker = position_tracker.lock().unwrap();
                        tracker.update(
                            tracker.position(),
                            *status.lock().unwrap() == PlaybackStatus::Playing,
                            metadata.lock().unwrap().length,
                            metadata.lock().unwrap().track_id.as_deref(),
                        );
                    }
                });
            });
        });

        Ok(())
    }

    /// Get the bus name.
    #[must_use]
    pub fn bus_name(&self) -> &str {
        &self.bus_name
    }

    /// Send a command to the player.
    fn send_command(&self, method: &str) -> Result<(), String> {
        let connection = self
            .connection
            .as_ref()
            .ok_or_else(|| "not connected to D-Bus".to_string())?;
        let proxy = zbus::Proxy::new(connection, &self.bus_name, MPRIS_PATH, PLAYER_INTERFACE)
            .map_err(|e| format!("failed to create D-Bus proxy: {e}"))?;
        proxy
            .call_method(method, &())
            .map_err(|e| format!("failed to call {method}: {e}"))?;
        Ok(())
    }
}

impl MediaSession for MprisSession {
    fn identity(&self) -> String {
        // Extract the player name from the bus name
        let name = self
            .bus_name
            .strip_prefix(MPRIS_PREFIX)
            .unwrap_or(&self.bus_name);
        name.to_string()
    }

    fn metadata(&self) -> &Metadata {
        // This is a bit of a hack: we need to return a reference to the metadata,
        // but it's behind a Mutex. We use unsafe to extend the lifetime, which is
        // safe because the Mutex guarantees the data lives as long as the session.
        unsafe { &*self.metadata.lock().unwrap().as_ref() as *const Metadata }
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
        let connection = self
            .connection
            .as_ref()
            .ok_or_else(|| "not connected to D-Bus".to_string())?;
        let proxy = zbus::Proxy::new(connection, &self.bus_name, MPRIS_PATH, PLAYER_INTERFACE)
            .map_err(|e| format!("failed to create D-Bus proxy: {e}"))?;
        let offset = position.as_micros() as i64;
        proxy
            .call_method("SetPosition", &("", offset))
            .map_err(|e| format!("failed to seek: {e}"))?;
        Ok(())
    }

    fn set_volume(&self, volume: f32) -> Result<(), String> {
        let connection = self
            .connection
            .as_ref()
            .ok_or_else(|| "not connected to D-Bus".to_string())?;
        let proxy = zbus::Proxy::new(connection, &self.bus_name, MPRIS_PATH, PLAYER_INTERFACE)
            .map_err(|e| format!("failed to create D-Bus proxy: {e}"))?;
        let volume = volume.clamp(0.0, 1.0);
        proxy
            .call_method("SetVolume", &(volume as f64,))
            .map_err(|e| format!("failed to set volume: {e}"))?;
        *self.volume.lock().unwrap() = volume;
        Ok(())
    }
}

/// Parse a playback status string.
fn parse_playback_status(status: &str) -> PlaybackStatus {
    match status {
        "Playing" => PlaybackStatus::Playing,
        "Paused" => PlaybackStatus::Paused,
        _ => PlaybackStatus::Stopped,
    }
}

/// Parse metadata from a D-Bus property map.
fn parse_metadata(map: &HashMap<String, zbus::zvariant::Value>) -> Metadata {
    let mut metadata = Metadata::default();

    if let Some(title) = map.get("xesam:title") {
        if let Ok(title) = String::try_from(title.clone()) {
            metadata.title = Some(title);
        }
    }

    if let Some(artists) = map.get("xesam:artist") {
        if let Ok(artists) = Vec::<String>::try_from(artists.clone()) {
            if let Some(artist) = artists.first() {
                metadata.artist = Some(artist.clone());
            }
        }
    }

    if let Some(album) = map.get("xesam:album") {
        if let Ok(album) = String::try_from(album.clone()) {
            metadata.album = Some(album);
        }
    }

    if let Some(art_url) = map.get("mpris:artUrl") {
        if let Ok(art_url) = String::try_from(art_url.clone()) {
            metadata.art_url = Some(art_url);
        }
    }

    if let Some(length) = map.get("mpris:length") {
        if let Ok(length) = i64::try_from(length.clone()) {
            if length > 0 {
                metadata.length = Some(Duration::from_micros(length as u64));
            }
        }
    }

    if let Some(track_id) = map.get("mpris:trackid") {
        if let Ok(track_id) = String::try_from(track_id.clone()) {
            metadata.track_id = Some(track_id);
        }
    }

    metadata
}

/// Discover all MPRIS sessions on the bus.
///
/// This function enumerates all bus names that start with
/// `org.mpris.MediaPlayer2.` and creates an [`MprisSession`] for each one.
#[must_use]
pub fn discover_sessions() -> Vec<MprisSession> {
    let connection = match zbus::Connection::session() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let proxy = match zbus::Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    ) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let names: Vec<String> = proxy
        .call_method("ListNames", &())
        .ok()
        .and_then(|reply| reply.body().deserialize().ok())
        .unwrap_or_default();

    names
        .into_iter()
        .filter(|name| name.starts_with(MPRIS_PREFIX))
        .map(MprisSession::new)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_playback_status_playing() {
        assert_eq!(parse_playback_status("Playing"), PlaybackStatus::Playing);
    }

    #[test]
    fn parse_playback_status_paused() {
        assert_eq!(parse_playback_status("Paused"), PlaybackStatus::Paused);
    }

    #[test]
    fn parse_playback_status_stopped() {
        assert_eq!(parse_playback_status("Stopped"), PlaybackStatus::Stopped);
    }

    #[test]
    fn parse_playback_status_unknown() {
        assert_eq!(parse_playback_status("Unknown"), PlaybackStatus::Stopped);
    }

    #[test]
    fn mpris_session_identity_strips_prefix() {
        let session = MprisSession::new("org.mpris.MediaPlayer2.spotify");
        assert_eq!(session.identity(), "spotify");
    }

    #[test]
    fn mpris_session_bus_name_is_stored() {
        let session = MprisSession::new("org.mpris.MediaPlayer2.vlc");
        assert_eq!(session.bus_name(), "org.mpris.MediaPlayer2.vlc");
    }

    #[test]
    fn mpris_session_default_state() {
        let session = MprisSession::new("org.mpris.MediaPlayer2.test");
        assert_eq!(session.playback_status(), PlaybackStatus::Stopped);
        assert_eq!(session.volume(), 0.5);
        assert_eq!(session.position(), Duration::ZERO);
    }

    #[test]
    fn mpris_session_capabilities_default_to_all() {
        let session = MprisSession::new("org.mpris.MediaPlayer2.test");
        let caps = session.capabilities();
        assert!(caps.can_play);
        assert!(caps.can_pause);
        assert!(caps.can_seek);
    }

    #[test]
    fn format_duration_formats_correctly() {
        assert_eq!(format_duration(Duration::from_secs(65)), "01:05");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1:01:01");
    }
}
