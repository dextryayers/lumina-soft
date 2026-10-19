//! Media session abstraction for Lumina.
//!
//! # The one-adapter-per-OS rule
//!
//! Lumina never links to Spotify, YouTube, or any other player. It reads what
//! the operating system already knows:
//!
//! * **Linux**: MPRIS2 over D-Bus. Every player that publishes a session
//!   (Spotify, VLC, mpv, browsers playing YouTube Music) is visible.
//! * **Windows**: GSMTC via WinRT. Same coverage, different transport.
//!
//! This is what makes the media widget work for every player without per-player
//! code, and what keeps the binary small.
//!
//! # Ranking
//!
//! When several players are active at once, [`rank_sessions`] picks one. The
//! score rewards playing state, complete metadata, and user preference, and
//! penalizes background browser tabs.
//!
//! # Failure handling
//!
//! All failure modes from PLAN.md §16.5 are implemented in the [`failure`]
//! module. The [`FailureHandler`] struct provides methods for each failure
//! mode, and the [`FailureAwareWatcher`] wraps the media watcher with failure
//! handling.

#![deny(missing_docs)]

pub mod art;
pub mod failure;
pub mod player_cycle;
pub mod ranking;
pub mod session;

#[cfg(target_os = "linux")]
pub mod mpris;

#[cfg(target_os = "windows")]
pub mod gsmtc;

pub use art::{AlbumArt, ArtCache, ArtPipeline};
pub use failure::{FailureAction, FailureAwareWatcher, FailureHandler};
pub use player_cycle::PlayerCycler;
pub use ranking::{best_snapshot, best_snapshot_with_defaults, rank_sessions, rank_sessions_with_defaults, MediaWatcher, RankingConfig, RankedSession};
pub use session::{format_duration, MediaCapabilities, MediaSession, MediaSnapshot, Metadata, PlaybackStatus, PositionTracker};

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
