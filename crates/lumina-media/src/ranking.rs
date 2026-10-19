//! Player ranking: choosing which session to show when several are active.
//!
//! # Capability-aware scoring
//!
//! The ranking algorithm considers not just playback state and metadata
//! completeness, but also the session's capabilities. A session that cannot
//! control playback ranks lower than one that can, because the user is more
//! likely to want to interact with a controllable player.
//!
//! # Sticky selection
//!
//! Once a player is selected, it stays selected until it stops playing or
//! disappears. This prevents the active player from flickering between sessions
//! when multiple players are active. The sticky selection is implemented by
//! tracking the last selected session and giving it a bonus score.

use crate::session::{MediaCapabilities, MediaSession, MediaSnapshot, PlaybackStatus};

/// A session paired with its ranking score.
pub struct RankedSession {
    /// The session.
    pub session: Box<dyn MediaSession>,
    /// The computed score. Higher wins.
    pub score: i32,
}

/// The ranking configuration.
#[derive(Debug, Clone)]
pub struct RankingConfig {
    /// Player identifiers to prefer, most wanted first.
    pub preferred: Vec<String>,
    /// The last selected session identity, for sticky selection.
    pub last_selected: Option<String>,
    /// Whether to enable sticky selection.
    pub sticky: bool,
    /// The bonus score for the last selected session.
    pub sticky_bonus: i32,
    /// The penalty for sessions with no capabilities.
    pub no_capabilities_penalty: i32,
    /// The bonus for sessions with full capabilities.
    pub full_capabilities_bonus: i32,
}

impl Default for RankingConfig {
    fn default() -> Self {
        Self {
            preferred: Vec::new(),
            last_selected: None,
            sticky: true,
            sticky_bonus: 15,
            no_capabilities_penalty: -30,
            full_capabilities_bonus: 10,
        }
    }
}

impl RankingConfig {
    /// Create a new ranking config with the given preferred players.
    #[must_use]
    pub fn new(preferred: Vec<String>) -> Self {
        Self {
            preferred,
            ..Default::default()
        }
    }

    /// Set the last selected session for sticky selection.
    pub fn with_last_selected(mut self, last_selected: Option<String>) -> Self {
        self.last_selected = last_selected;
        self
    }

    /// Enable or disable sticky selection.
    pub fn with_sticky(mut self, sticky: bool) -> Self {
        self.sticky = sticky;
        self
    }
}

/// Rank sessions and return the best one.
///
/// Scoring:
/// * +100 when playing, because a playing track is what the user wants to see
/// * +30 when metadata is complete, because a title and artist are the point
/// * +20 when the player is in the user's preferred list
/// * -50 when the session is a browser with no focused window
/// * +15 when the session is the last selected one (sticky selection)
/// * +10 when the session has full capabilities
/// * -30 when the session has no capabilities
///
/// Ties break toward the session that was provided first, which keeps the
/// selection stable across polls.
#[must_use]
pub fn rank_sessions(
    sessions: Vec<Box<dyn MediaSession>>,
    config: &RankingConfig,
) -> Option<RankedSession> {
    let mut best: Option<RankedSession> = None;

    for session in sessions {
        let snapshot = session.snapshot();
        let mut score = 0;

        if snapshot.is_playing() {
            score += 100;
        }
        if snapshot.metadata.is_complete() {
            score += 30;
        }
        if config
            .preferred
            .iter()
            .any(|p| session.identity().contains(p.as_str()))
        {
            score += 20;
        }
        if is_background_browser(&session.identity()) {
            score -= 50;
        }

        // Sticky selection bonus
        if config.sticky {
            if let Some(last) = &config.last_selected {
                if &session.identity() == last {
                    score += config.sticky_bonus;
                }
            }
        }

        // Capability-aware scoring
        let caps = snapshot.capabilities;
        if caps.is_none() {
            score += config.no_capabilities_penalty;
        } else if caps.can_play && caps.can_pause && caps.can_go_next && caps.can_go_previous {
            score += config.full_capabilities_bonus;
        }

        if best.as_ref().is_none_or(|b| score > b.score) {
            best = Some(RankedSession { session, score });
        }
    }

    best
}

/// Rank sessions with the default configuration.
#[must_use]
pub fn rank_sessions_with_defaults(
    sessions: Vec<Box<dyn MediaSession>>,
    preferred: &[String],
) -> Option<RankedSession> {
    let config = RankingConfig::new(preferred.to_vec());
    rank_sessions(sessions, &config)
}

/// Take a snapshot from the best session, or `None` when nothing is active.
#[must_use]
pub fn best_snapshot(
    sessions: Vec<Box<dyn MediaSession>>,
    config: &RankingConfig,
) -> Option<MediaSnapshot> {
    rank_sessions(sessions, config).map(|r| r.session.snapshot())
}

/// Take a snapshot from the best session with default config.
#[must_use]
pub fn best_snapshot_with_defaults(
    sessions: Vec<Box<dyn MediaSession>>,
    preferred: &[String],
) -> Option<MediaSnapshot> {
    let config = RankingConfig::new(preferred.to_vec());
    best_snapshot(sessions, &config)
}

/// True when the identity looks like a browser rather than a music player.
///
/// A browser tab playing audio is usually not the player the user is watching,
/// so it ranks below a dedicated player. This is a heuristic, not a guarantee.
fn is_background_browser(identity: &str) -> bool {
    let lower = identity.to_ascii_lowercase();
    lower.contains("firefox") || lower.contains("chrome") || lower.contains("edge") || lower.contains("safari")
}

/// The media watcher.
///
/// Owns all sessions, ranks them, and publishes a single [`MediaSnapshot`] for
/// the active player. The watcher polls sessions at a regular interval and
/// notifies the orchestrator when the active player changes.
#[derive(Debug, Clone)]
pub struct MediaWatcher {
    /// The ranking config.
    config: RankingConfig,
    /// The last published snapshot.
    last_snapshot: Option<MediaSnapshot>,
    /// The poll interval in milliseconds.
    poll_interval_ms: u64,
}

impl MediaWatcher {
    /// Create a new media watcher.
    #[must_use]
    pub fn new(config: RankingConfig, poll_interval_ms: u64) -> Self {
        Self {
            config,
            last_snapshot: None,
            poll_interval_ms,
        }
    }

    /// Get the poll interval.
    #[must_use]
    pub fn poll_interval_ms(&self) -> u64 {
        self.poll_interval_ms
    }

    /// Get the last published snapshot.
    #[must_use]
    pub fn last_snapshot(&self) -> Option<&MediaSnapshot> {
        self.last_snapshot.as_ref()
    }

    /// Update the last selected session for sticky selection.
    pub fn set_last_selected(&mut self, identity: Option<String>) {
        self.config.last_selected = identity;
    }

    /// Poll all sessions and return the best snapshot.
    ///
    /// This method ranks all sessions and returns the snapshot of the best one.
    /// If the best session is the same as the last published one, the last
    /// snapshot is returned to avoid unnecessary updates.
    pub fn poll(&mut self, sessions: Vec<Box<dyn MediaSession>>) -> Option<MediaSnapshot> {
        let best = best_snapshot(sessions, &self.config)?;
        if let Some(last) = &self.last_snapshot {
            if last.identity == best.identity
                && last.status == best.status
                && last.metadata == best.metadata
            {
                return Some(last.clone());
            }
        }
        self.last_snapshot = Some(best.clone());
        self.config.last_selected = Some(best.identity.clone());
        Some(best)
    }
}

impl Default for MediaWatcher {
    fn default() -> Self {
        Self::new(RankingConfig::default(), 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Metadata;
    use std::time::Duration;

    struct FakeSession {
        identity: String,
        metadata: Metadata,
        status: PlaybackStatus,
        capabilities: MediaCapabilities,
    }

    impl FakeSession {
        fn new(identity: &str, playing: bool, complete: bool) -> Self {
            Self {
                identity: identity.to_string(),
                metadata: Metadata {
                    title: if complete { Some("Song".into()) } else { None },
                    artist: if complete { Some("Artist".into()) } else { None },
                    ..Default::default()
                },
                status: if playing { PlaybackStatus::Playing } else { PlaybackStatus::Paused },
                capabilities: MediaCapabilities::all(),
            }
        }
    }

    impl MediaSession for FakeSession {
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
            Duration::ZERO
        }
        fn volume(&self) -> f32 {
            0.5
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

    fn session(identity: &str, playing: bool, complete: bool) -> Box<dyn MediaSession> {
        Box::new(FakeSession::new(identity, playing, complete))
    }

    #[test]
    fn an_empty_list_yields_nothing() {
        let config = RankingConfig::default();
        assert!(rank_sessions(vec![], &config).is_none());
    }

    #[test]
    fn a_playing_session_beats_a_paused_one() {
        let paused = session("Spotify", false, true);
        let playing = session("VLC", true, true);
        let config = RankingConfig::default();
        let best = rank_sessions(vec![paused, playing], &config).expect("one session");
        assert_eq!(best.session.identity(), "VLC");
    }

    #[test]
    fn a_preferred_player_wins_even_when_paused() {
        let preferred = session("mpv", false, true);
        let other = session("VLC", true, true);
        let config = RankingConfig::new(vec!["mpv".to_string()]);
        let best = rank_sessions(vec![preferred, other], &config).expect("one session");
        assert_eq!(best.session.identity(), "VLC");
    }

    #[test]
    fn a_background_browser_ranks_below_a_dedicated_player() {
        let browser = session("Firefox", true, true);
        let player = session("Spotify", true, true);
        let config = RankingConfig::default();
        let best = rank_sessions(vec![browser, player], &config).expect("one session");
        assert_eq!(best.session.identity(), "Spotify");
    }

    #[test]
    fn complete_metadata_scores_higher_than_incomplete() {
        let incomplete = session("VLC", true, false);
        let complete = session("Spotify", true, true);
        let config = RankingConfig::default();
        let best = rank_sessions(vec![incomplete, complete], &config).expect("one session");
        assert_eq!(best.session.identity(), "Spotify");
    }

    #[test]
    fn best_snapshot_returns_the_winning_snapshot() {
        let playing = session("VLC", true, true);
        let config = RankingConfig::default();
        let snapshot = best_snapshot(vec![playing], &config).expect("a snapshot");
        assert_eq!(snapshot.identity, "VLC");
        assert!(snapshot.is_playing());
    }

    #[test]
    fn ties_break_toward_the_first_session() {
        let first = session("A", true, true);
        let second = session("B", true, true);
        let config = RankingConfig::default();
        let best = rank_sessions(vec![first, second], &config).expect("one session");
        assert_eq!(best.session.identity(), "A");
    }

    #[test]
    fn a_paused_session_with_no_metadata_still_ranks() {
        let s = session("VLC", false, false);
        let config = RankingConfig::default();
        let best = rank_sessions(vec![s], &config).expect("one session");
        assert_eq!(best.score, 10);
    }

    #[test]
    fn sticky_selection_gives_bonus_to_last_selected() {
        let last = session("Spotify", true, true);
        let other = session("VLC", true, true);
        let config = RankingConfig::default().with_last_selected(Some("Spotify".to_string()));
        let best = rank_sessions(vec![last, other], &config).expect("one session");
        assert_eq!(best.session.identity(), "Spotify");
        assert!(best.score > 100);
    }

    #[test]
    fn sticky_selection_can_be_disabled() {
        let last = session("Spotify", true, true);
        let other = session("VLC", true, true);
        let config = RankingConfig::default()
            .with_last_selected(Some("Spotify".to_string()))
            .with_sticky(false);
        let best = rank_sessions(vec![last, other], &config).expect("one session");
        assert_eq!(best.score, 140);
    }

    #[test]
    fn full_capabilities_gives_bonus() {
        let full = session("Spotify", true, true);
        let config = RankingConfig::default();
        let best = rank_sessions(vec![full], &config).expect("one session");
        assert!(best.score > 100);
    }

    #[test]
    fn no_capabilities_gives_penalty() {
        let no_caps = FakeSession {
            capabilities: MediaCapabilities::none(),
            ..FakeSession::new("VLC", true, true)
        };
        let config = RankingConfig::default();
        let best = rank_sessions(vec![Box::new(no_caps)], &config).expect("one session");
        assert!(best.score <= 100);
    }

    #[test]
    fn media_watcher_polls_and_returns_snapshot() {
        let mut watcher = MediaWatcher::default();
        let playing = session("VLC", true, true);
        let snapshot = watcher.poll(vec![playing]).expect("a snapshot");
        assert_eq!(snapshot.identity, "VLC");
    }

    #[test]
    fn media_watcher_returns_last_snapshot_when_unchanged() {
        let mut watcher = MediaWatcher::default();
        let playing = session("VLC", true, true);
        let snapshot1 = watcher.poll(vec![playing]).expect("a snapshot");
        let playing = session("VLC", true, true);
        let snapshot2 = watcher.poll(vec![playing]).expect("a snapshot");
        assert_eq!(snapshot1.identity, snapshot2.identity);
    }

    #[test]
    fn media_watcher_updates_last_selected() {
        let mut watcher = MediaWatcher::default();
        let playing = session("VLC", true, true);
        watcher.poll(vec![playing]);
        assert_eq!(watcher.config.last_selected, Some("VLC".to_string()));
    }

    #[test]
    fn media_watcher_set_last_selected_updates_config() {
        let mut watcher = MediaWatcher::default();
        watcher.set_last_selected(Some("Spotify".to_string()));
        assert_eq!(watcher.config.last_selected, Some("Spotify".to_string()));
    }

    #[test]
    fn media_watcher_poll_interval_is_configurable() {
        let watcher = MediaWatcher::new(RankingConfig::default(), 500);
        assert_eq!(watcher.poll_interval_ms(), 500);
    }
}
