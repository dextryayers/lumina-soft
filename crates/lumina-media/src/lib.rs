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

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

pub mod gsmtc;
pub mod ranking;
pub mod session;

pub use ranking::rank_sessions;
pub use session::{MediaSession, MediaSnapshot, Metadata, PlaybackStatus};

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");