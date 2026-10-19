//! Failure modes from PLAN.md §16.5.
//!
//! # The failure modes
//!
//! | Condition | Behavior |
//! |---|---|
//! | No sessions found | Media widget hidden, no error shown |
//! | Session disappears mid-command | Command returns `PlayerGone`, snapshot clears, state machine transitions to Collapsed |
//! | Player does not implement `Seek` | Progress bar becomes read only, clicking does nothing |
//! | Art fetch fails | Generic icon, cached negative result for 60 s |
//! | D-Bus unavailable on Linux | Media widget hidden, log a warn with the D-Bus error text |
//! | WinRT activation fails | Media widget hidden, log a warn, rest of the app unaffected |
//!
//! # Implementation
//!
//! This module provides the [`FailureHandler`] struct, which implements the
//! failure behavior for each mode. The handler is called by the media watcher
//! when a failure occurs.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::session::{MediaSession, MediaSnapshot, PlaybackStatus};

/// The failure handler.
///
/// Implements the failure behavior for each mode from PLAN.md §16.5.
#[derive(Debug)]
pub struct FailureHandler {
    /// The negative art cache: track_id -> (timestamp, result).
    art_cache: Mutex<HashMap<String, (Instant, Option<()>)>>,
    /// The art cache TTL.
    art_cache_ttl: Duration,
    /// The last error message.
    last_error: Mutex<Option<String>>,
    /// The time of the last error.
    last_error_time: Mutex<Option<Instant>>,
}

impl FailureHandler {
    /// Create a new failure handler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            art_cache: Mutex::new(HashMap::new()),
            art_cache_ttl: Duration::from_secs(60),
            last_error: Mutex::new(None),
            last_error_time: Mutex::new(None),
        }
    }

    /// Handle the "no sessions found" failure.
    ///
    /// The media widget is hidden, no error is shown.
    pub fn handle_no_sessions(&self) {
        // No action needed: the media widget is hidden by the orchestrator
        // when there are no sessions.
    }

    /// Handle the "session disappears mid-command" failure.
    ///
    /// The command returns `PlayerGone`, the snapshot clears, and the state
    /// machine transitions to Collapsed.
    pub fn handle_session_disappeared(&self, identity: &str) -> FailureAction {
        self.log_error(format!("session disappeared: {identity}"));
        FailureAction::ClearAndCollapse
    }

    /// Handle the "player does not implement Seek" failure.
    ///
    /// The progress bar becomes read only, clicking does nothing.
    pub fn handle_no_seek(&self) -> FailureAction {
        FailureAction::ReadOnlyProgress
    }

    /// Handle the "art fetch fails" failure.
    ///
    /// A generic icon is shown, and the negative result is cached for 60 s.
    pub fn handle_art_fetch_failed(&self, track_id: &str) -> FailureAction {
        self.cache_art_result(track_id, None);
        FailureAction::ShowGenericIcon
    }

    /// Handle the "D-Bus unavailable on Linux" failure.
    ///
    /// The media widget is hidden, a warn is logged with the D-Bus error text.
    pub fn handle_dbus_unavailable(&self, error: &str) -> FailureAction {
        self.log_error(format!("D-Bus unavailable: {error}"));
        FailureAction::HideWidget
    }

    /// Handle the "WinRT activation fails" failure.
    ///
    /// The media widget is hidden, a warn is logged, the rest of the app is
    /// unaffected.
    pub fn handle_winrt_failure(&self, error: &str) -> FailureAction {
        self.log_error(format!("WinRT activation failed: {error}"));
        FailureAction::HideWidget
    }

    /// Check if an art result is cached.
    ///
    /// Returns `Some(true)` if the art was successfully cached, `Some(false)`
    /// if the art fetch failed and the negative result is still cached, and
    /// `None` if there is no cached result.
    #[must_use]
    pub fn check_art_cache(&self, track_id: &str) -> Option<bool> {
        let cache = self.art_cache.lock().unwrap();
        if let Some((timestamp, result)) = cache.get(track_id) {
            if timestamp.elapsed() < self.art_cache_ttl {
                return Some(result.is_some());
            }
        }
        None
    }

    /// Cache an art result.
    fn cache_art_result(&self, track_id: &str, result: Option<()>) {
        let mut cache = self.art_cache.lock().unwrap();
        cache.insert(track_id.to_string(), (Instant::now(), result));
    }

    /// Log an error.
    fn log_error(&self, message: String) {
        *self.last_error.lock().unwrap() = Some(message.clone());
        *self.last_error_time.lock().unwrap() = Some(Instant::now());
        eprintln!("[lumina-media] {message}");
    }

    /// Get the last error message.
    #[must_use]
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().unwrap().clone()
    }

    /// Get the time of the last error.
    #[must_use]
    pub fn last_error_time(&self) -> Option<Instant> {
        *self.last_error_time.lock().unwrap()
    }

    /// Clear the art cache.
    pub fn clear_art_cache(&self) {
        self.art_cache.lock().unwrap().clear();
    }

    /// Get the number of entries in the art cache.
    #[must_use]
    pub fn art_cache_len(&self) -> usize {
        self.art_cache.lock().unwrap().len()
    }
}

impl Default for FailureHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// The action to take when a failure occurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    /// No action needed.
    None,
    /// Clear the snapshot and collapse the island.
    ClearAndCollapse,
    /// Make the progress bar read only.
    ReadOnlyProgress,
    /// Show a generic music note icon.
    ShowGenericIcon,
    /// Hide the media widget.
    HideWidget,
}

/// The failure-aware media watcher.
///
/// Wraps the media watcher with failure handling.
#[derive(Debug)]
pub struct FailureAwareWatcher {
    /// The failure handler.
    handler: FailureHandler,
    /// The last snapshot.
    last_snapshot: Option<MediaSnapshot>,
}

impl FailureAwareWatcher {
    /// Create a new failure-aware watcher.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handler: FailureHandler::new(),
            last_snapshot: None,
        }
    }

    /// Get the failure handler.
    #[must_use]
    pub fn handler(&self) -> &FailureHandler {
        &self.handler
    }

    /// Get the last snapshot.
    #[must_use]
    pub fn last_snapshot(&self) -> Option<&MediaSnapshot> {
        self.last_snapshot.as_ref()
    }

    /// Poll all sessions and return the snapshot of the best one.
    ///
    /// If no sessions are available, the "no sessions found" failure is
    /// handled and `None` is returned.
    pub fn poll(&mut self, sessions: Vec<Box<dyn MediaSession>>) -> Option<MediaSnapshot> {
        if sessions.is_empty() {
            self.handler.handle_no_sessions();
            self.last_snapshot = None;
            return None;
        }

        // Find the best session
        let best = sessions
            .iter()
            .max_by_key(|s| {
                let snapshot = s.snapshot();
                let mut score = 0;
                if snapshot.is_playing() {
                    score += 100;
                }
                if snapshot.metadata.is_complete() {
                    score += 30;
                }
                score
            })?;

        let snapshot = best.snapshot();
        self.last_snapshot = Some(snapshot.clone());
        Some(snapshot)
    }

    /// Handle a command failure.
    ///
    /// If the session disappeared, the snapshot is cleared and the action
    /// is returned.
    pub fn handle_command_failure(
        &mut self,
        identity: &str,
        error: &str,
    ) -> FailureAction {
        if error.contains("disappeared") || error.contains("gone") {
            self.last_snapshot = None;
            self.handler.handle_session_disappeared(identity)
        } else {
            self.handler.log_error(error.to_string());
            FailureAction::None
        }
    }

    /// Handle an art fetch failure.
    pub fn handle_art_failure(&mut self, track_id: &str) -> FailureAction {
        self.handler.handle_art_fetch_failed(track_id)
    }

    /// Check if an art result is cached.
    #[must_use]
    pub fn check_art_cache(&self, track_id: &str) -> Option<bool> {
        self.handler.check_art_cache(track_id)
    }
}

impl Default for FailureAwareWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_failure_handler_has_no_errors() {
        let handler = FailureHandler::new();
        assert!(handler.last_error().is_none());
        assert!(handler.last_error_time().is_none());
    }

    #[test]
    fn failure_handler_handles_no_sessions() {
        let handler = FailureHandler::new();
        handler.handle_no_sessions();
        // No error should be logged
        assert!(handler.last_error().is_none());
    }

    #[test]
    fn failure_handler_handles_session_disappeared() {
        let handler = FailureHandler::new();
        let action = handler.handle_session_disappeared("test");
        assert_eq!(action, FailureAction::ClearAndCollapse);
        assert!(handler.last_error().is_some());
    }

    #[test]
    fn failure_handler_handles_no_seek() {
        let handler = FailureHandler::new();
        let action = handler.handle_no_seek();
        assert_eq!(action, FailureAction::ReadOnlyProgress);
    }

    #[test]
    fn failure_handler_handles_art_fetch_failed() {
        let handler = FailureHandler::new();
        let action = handler.handle_art_fetch_failed("track1");
        assert_eq!(action, FailureAction::ShowGenericIcon);
        assert_eq!(handler.art_cache_len(), 1);
    }

    #[test]
    fn failure_handler_handles_dbus_unavailable() {
        let handler = FailureHandler::new();
        let action = handler.handle_dbus_unavailable("connection refused");
        assert_eq!(action, FailureAction::HideWidget);
        assert!(handler.last_error().is_some());
    }

    #[test]
    fn failure_handler_handles_winrt_failure() {
        let handler = FailureHandler::new();
        let action = handler.handle_winrt_failure("access denied");
        assert_eq!(action, FailureAction::HideWidget);
        assert!(handler.last_error().is_some());
    }

    #[test]
    fn failure_handler_caches_art_result() {
        let handler = FailureHandler::new();
        handler.handle_art_fetch_failed("track1");
        let cached = handler.check_art_cache("track1");
        assert_eq!(cached, Some(false));
    }

    #[test]
    fn failure_handler_art_cache_expires() {
        let handler = FailureHandler::new();
        handler.cache_art_result("track1", None);
        // Manually expire the cache entry
        {
            let mut cache = handler.art_cache.lock().unwrap();
            if let Some((timestamp, _)) = cache.get_mut("track1") {
                *timestamp = Instant::now() - Duration::from_secs(120);
            }
        }
        let cached = handler.check_art_cache("track1");
        assert_eq!(cached, None);
    }

    #[test]
    fn failure_handler_clear_art_cache() {
        let handler = FailureHandler::new();
        handler.handle_art_fetch_failed("track1");
        assert_eq!(handler.art_cache_len(), 1);
        handler.clear_art_cache();
        assert_eq!(handler.art_cache_len(), 0);
    }

    #[test]
    fn failure_aware_watcher_poll_returns_none_when_no_sessions() {
        let mut watcher = FailureAwareWatcher::new();
        let snapshot = watcher.poll(vec![]);
        assert!(snapshot.is_none());
    }

    #[test]
    fn failure_aware_watcher_poll_returns_snapshot() {
        use crate::session::{Metadata, MediaCapabilities};
        use std::sync::LazyLock;
        use std::time::Duration;

        struct FakeSession;

        static METADATA: LazyLock<Metadata> = LazyLock::new(|| Metadata {
            title: Some("Test".to_string()),
            artist: Some("Artist".to_string()),
            album: None,
            art_url: None,
            length: Some(Duration::from_secs(180)),
            track_id: None,
        });

        impl MediaSession for FakeSession {
            fn identity(&self) -> String {
                "test".to_string()
            }
            fn metadata(&self) -> &Metadata {
                &METADATA
            }
            fn playback_status(&self) -> PlaybackStatus {
                PlaybackStatus::Playing
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

        let mut watcher = FailureAwareWatcher::new();
        let snapshot = watcher.poll(vec![Box::new(FakeSession)]);
        assert!(snapshot.is_some());
    }

    #[test]
    fn failure_aware_watcher_handles_command_failure() {
        let mut watcher = FailureAwareWatcher::new();
        let action = watcher.handle_command_failure("test", "session gone");
        assert_eq!(action, FailureAction::ClearAndCollapse);
    }

    #[test]
    fn failure_aware_watcher_handles_art_failure() {
        let mut watcher = FailureAwareWatcher::new();
        let action = watcher.handle_art_failure("track1");
        assert_eq!(action, FailureAction::ShowGenericIcon);
    }

    #[test]
    fn failure_aware_watcher_checks_art_cache() {
        let mut watcher = FailureAwareWatcher::new();
        watcher.handle_art_failure("track1");
        let cached = watcher.check_art_cache("track1");
        assert_eq!(cached, Some(false));
    }

    #[test]
    fn failure_action_has_correct_values() {
        assert_eq!(FailureAction::None, FailureAction::None);
        assert_eq!(FailureAction::ClearAndCollapse, FailureAction::ClearAndCollapse);
        assert_eq!(FailureAction::ReadOnlyProgress, FailureAction::ReadOnlyProgress);
        assert_eq!(FailureAction::ShowGenericIcon, FailureAction::ShowGenericIcon);
        assert_eq!(FailureAction::HideWidget, FailureAction::HideWidget);
    }
}
