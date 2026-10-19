//! Multi-player cycling with hotkey and auto-fallback.
//!
//! # The cycling model
//!
//! When multiple players are active, the user can cycle through them with the
//! `media.cycle_players` hotkey. The cycling is circular: after the last player,
//! it wraps back to the first.
//!
//! # Auto-fallback
//!
//! When the active player stops playing or disappears, the watcher
//! automatically falls back to the next best player. This is implemented by
//! re-ranking all sessions and selecting the new best one.
//!
//! # Sticky selection
//!
//! Once a player is selected (either by cycling or by auto-fallback), it stays
//! selected until it stops playing or disappears. This prevents the active
//! player from flickering between sessions.

use crate::session::{MediaSession, MediaSnapshot};
use crate::ranking::{RankingConfig, MediaWatcher};

/// The player cycler.
///
/// Manages the list of active players and cycles through them.
#[derive(Debug, Clone)]
pub struct PlayerCycler {
    /// The list of active player identities.
    players: Vec<String>,
    /// The index of the currently selected player.
    current_index: usize,
    /// The media watcher.
    watcher: MediaWatcher,
}

impl PlayerCycler {
    /// Create a new player cycler.
    #[must_use]
    pub fn new(config: &RankingConfig) -> Self {
        Self {
            players: Vec::new(),
            current_index: 0,
            watcher: MediaWatcher::new(config.clone(), 1000),
        }
    }

    /// Get the list of active players.
    #[must_use]
    pub fn players(&self) -> &[String] {
        &self.players
    }

    /// Get the index of the currently selected player.
    #[must_use]
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    /// Get the identity of the currently selected player.
    #[must_use]
    pub fn current_player(&self) -> Option<&str> {
        self.players.get(self.current_index).map(std::string::String::as_str)
    }

    /// Update the list of active players.
    ///
    /// This should be called whenever the set of active sessions changes.
    /// The cycler preserves the current selection if the player is still
    /// active, and falls back to the first player otherwise.
    pub fn update_players(&mut self, players: Vec<String>) {
        let old_player = self.current_player().map(std::string::ToString::to_string);
        self.players = players;
        if let Some(old) = old_player {
            if let Some(index) = self.players.iter().position(|p| p == &old) {
                self.current_index = index;
            } else {
                self.current_index = 0;
            }
        } else {
            self.current_index = 0;
        }
    }

    /// Cycle to the next player.
    ///
    /// Returns the identity of the new current player.
    #[must_use]
    pub fn cycle_next(&mut self) -> Option<&str> {
        if self.players.is_empty() {
            return None;
        }
        self.current_index = (self.current_index + 1) % self.players.len();
        self.watcher.set_last_selected(self.current_player().map(std::string::ToString::to_string));
        self.current_player()
    }

    /// Cycle to the previous player.
    ///
    /// Returns the identity of the new current player.
    #[must_use]
    pub fn cycle_previous(&mut self) -> Option<&str> {
        if self.players.is_empty() {
            return None;
        }
        if self.current_index == 0 {
            self.current_index = self.players.len() - 1;
        } else {
            self.current_index -= 1;
        }
        self.watcher.set_last_selected(self.current_player().map(std::string::ToString::to_string));
        self.current_player()
    }

    /// Select a specific player by identity.
    ///
    /// Returns true if the player was found and selected.
    pub fn select_player(&mut self, identity: &str) -> bool {
        if let Some(index) = self.players.iter().position(|p| p == identity) {
            self.current_index = index;
            self.watcher.set_last_selected(Some(identity.to_string()));
            true
        } else {
            false
        }
    }

    /// Poll all sessions and return the snapshot of the current player.
    ///
    /// If the current player is no longer active, the cycler falls back to
    /// the next best player.
    pub fn poll(&mut self, sessions: Vec<Box<dyn MediaSession>>) -> Option<MediaSnapshot> {
        // Update the player list
        let players: Vec<String> = sessions.iter().map(|s| s.identity()).collect();
        self.update_players(players);

        // Poll the watcher
        let snapshot = self.watcher.poll(sessions)?;

        // If the current player is not the best, fall back
        if let Some(current) = self.current_player() {
            if snapshot.identity != current {
                // The current player is no longer the best, fall back
                self.watcher.set_last_selected(Some(snapshot.identity.clone()));
            }
        }

        Some(snapshot)
    }

    /// Get the media watcher.
    #[must_use]
    pub fn watcher(&self) -> &MediaWatcher {
        &self.watcher
    }
}

impl Default for PlayerCycler {
    fn default() -> Self {
        let config = RankingConfig::default();
        Self::new(&config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Metadata, MediaCapabilities, PlaybackStatus};
    use std::time::Duration;

    struct FakeSession {
        identity: String,
        status: PlaybackStatus,
    }

    impl MediaSession for FakeSession {
        fn identity(&self) -> String {
            self.identity.clone()
        }
        fn metadata(&self) -> &Metadata {
            static METADATA: Metadata = Metadata {
                title: None,
                artist: None,
                album: None,
                art_url: None,
                length: None,
                track_id: None,
            };
            &METADATA
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
            MediaCapabilities::all()
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

    fn session(identity: &str) -> Box<dyn MediaSession> {
        Box::new(FakeSession {
            identity: identity.to_string(),
            status: PlaybackStatus::Playing,
        })
    }

    #[test]
    fn a_new_cycler_has_no_players() {
        let cycler = PlayerCycler::new(&RankingConfig::default());
        assert_eq!(cycler.players().len(), 0);
        assert!(cycler.current_player().is_none());
    }

    #[test]
    fn cycler_update_players_sets_list() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        cycler.update_players(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(cycler.players().len(), 3);
        assert_eq!(cycler.current_player(), Some("a"));
    }

    #[test]
    fn cycler_cycle_next_wraps_around() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        cycler.update_players(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(cycler.cycle_next(), Some("b"));
        assert_eq!(cycler.cycle_next(), Some("c"));
        assert_eq!(cycler.cycle_next(), Some("a"));
    }

    #[test]
    fn cycler_cycle_previous_wraps_around() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        cycler.update_players(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(cycler.cycle_previous(), Some("c"));
        assert_eq!(cycler.cycle_previous(), Some("b"));
        assert_eq!(cycler.cycle_previous(), Some("a"));
    }

    #[test]
    fn cycler_select_player_finds_player() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        cycler.update_players(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert!(cycler.select_player("b"));
        assert_eq!(cycler.current_player(), Some("b"));
    }

    #[test]
    fn cycler_select_player_returns_false_for_unknown() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        cycler.update_players(vec!["a".to_string(), "b".to_string()]);
        assert!(!cycler.select_player("z"));
    }

    #[test]
    fn cycler_update_players_preserves_selection() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        cycler.update_players(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        cycler.select_player("b");
        cycler.update_players(vec!["a".to_string(), "b".to_string(), "d".to_string()]);
        assert_eq!(cycler.current_player(), Some("b"));
    }

    #[test]
    fn cycler_update_players_resets_when_current_removed() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        cycler.update_players(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        cycler.select_player("b");
        cycler.update_players(vec!["a".to_string(), "c".to_string(), "d".to_string()]);
        assert_eq!(cycler.current_player(), Some("a"));
    }

    #[test]
    fn cycler_poll_returns_snapshot() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        let sessions = vec![session("a"), session("b")];
        let snapshot = cycler.poll(sessions);
        assert!(snapshot.is_some());
    }

    #[test]
    fn cycler_poll_updates_player_list() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        let sessions = vec![session("a"), session("b")];
        cycler.poll(sessions);
        assert_eq!(cycler.players().len(), 2);
    }

    #[test]
    fn cycler_empty_players_returns_none_on_cycle() {
        let mut cycler = PlayerCycler::new(&RankingConfig::default());
        assert!(cycler.cycle_next().is_none());
        assert!(cycler.cycle_previous().is_none());
    }

    #[test]
    fn cycler_watcher_is_accessible() {
        let cycler = PlayerCycler::new(&RankingConfig::default());
        assert!(cycler.watcher().last_snapshot().is_none());
    }
}
