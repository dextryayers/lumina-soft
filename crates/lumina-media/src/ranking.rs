//! Player ranking: choosing which session to show when several are active.

use crate::session::{MediaSession, MediaSnapshot, PlaybackStatus};

/// A session paired with its ranking score.
#[derive(Debug, Clone)]
pub struct RankedSession {
    /// The session.
    pub session: Box<dyn MediaSession>,
    /// The computed score. Higher wins.
    pub score: i32,
}

/// Rank sessions and return the best one.
///
/// Scoring:
/// * +100 when playing, because a playing track is what the user wants to see
/// * +30 when metadata is complete, because a title and artist are the point
/// * +20 when the player is in the user's preferred list
/// * -50 when the session is a browser with no focused window, because a
///   background tab playing audio is not the player the user is watching
///
/// Ties break toward the session that was provided first, which keeps the
/// selection stable across polls.
#[must_use]
pub fn rank_sessions(
    sessions: Vec<Box<dyn MediaSession>>,
    preferred: &[String],
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
        if preferred.iter().any(|p| session.identity().contains(p.as_str())) {
            score += 20;
        }
        if is_background_browser(&session.identity()) {
            score -= 50;
        }

        if best.as_ref().is_none_or(|b| score > b.score) {
            best = Some(RankedSession { session, score });
        }
    }

    best
}

/// True when the identity looks like a browser rather than a music player.
///
/// A browser tab playing audio is usually not the player the user is watching,
/// so it ranks below a dedicated player. This is a heuristic, not a guarantee.
fn is_background_browser(identity: &str) -> bool {
    let lower = identity.to_ascii_lowercase();
    lower.contains("firefox") || lower.contains("chrome") || lower.contains("edge") || lower.contains("safari")
}

/// Take a snapshot from the best session, or `None` when nothing is active.
#[must_use]
pub fn best_snapshot(
    sessions: Vec<Box<dyn MediaSession>>,
    preferred: &[String],
) -> Option<MediaSnapshot> {
    rank_sessions(sessions, preferred).map(|r| r.session.snapshot())
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
        Box::new(FakeSession {
            identity: identity.to_string(),
            metadata: Metadata {
                title: if complete { Some("Song".into()) } else { None },
                artist: if complete { Some("Artist".into()) } else { None },
                ..Default::default()
            },
            status: if playing { PlaybackStatus::Playing } else { PlaybackStatus::Paused },
        })
    }

    #[test]
    fn an_empty_list_yields_nothing() {
        assert!(rank_sessions(vec![], &[]).is_none());
    }

    #[test]
    fn a_playing_session_beats_a_paused_one() {
        let paused = session("Spotify", false, true);
        let playing = session("VLC", true, true);
        let best = rank_sessions(vec![paused, playing], &[]).expect("one session");
        assert_eq!(best.session.identity(), "VLC");
    }

    #[test]
    fn a_preferred_player_wins_even_when_paused() {
        let preferred = session("mpv", false, true);
        let other = session("VLC", true, true);
        let best = rank_sessions(vec![preferred, other], &["mpv"]).expect("one session");
        assert_eq!(best.session.identity(), "mpv");
    }

    #[test]
    fn a_background_browser_ranks_below_a_dedicated_player() {
        let browser = session("Firefox", true, true);
        let player = session("Spotify", true, true);
        let best = rank_sessions(vec![browser, player], &[]).expect("one session");
        assert_eq!(best.session.identity(), "Spotify");
    }

    #[test]
    fn complete_metadata_scores_higher_than_incomplete() {
        let incomplete = session("VLC", true, false);
        let complete = session("Spotify", true, true);
        let best = rank_sessions(vec![incomplete, complete], &[]).expect("one session");
        assert_eq!(best.session.identity(), "Spotify");
    }

    #[test]
    fn best_snapshot_returns_the_winning_snapshot() {
        let playing = session("VLC", true, true);
        let snapshot = best_snapshot(vec![playing], &[]).expect("a snapshot");
        assert_eq!(snapshot.identity, "VLC");
        assert!(snapshot.is_playing());
    }

    #[test]
    fn ties_break_toward_the_first_session() {
        let first = session("A", true, true);
        let second = session("B", true, true);
        let best = rank_sessions(vec![first, second], &[]).expect("one session");
        assert_eq!(best.session.identity(), "A");
    }

    #[test]
    fn a_paused_session_with_no_metadata_still_ranks() {
        let session = session("VLC", false, false);
        let best = rank_sessions(vec![session], &[]).expect("one session");
        assert_eq!(best.score, 0);
    }
}