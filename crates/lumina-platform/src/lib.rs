//! Display server abstraction for Lumina.
//!
//! # What this crate is for
//!
//! The island is a window that has to do five things no ordinary window does:
//!
//! 1. Stay above every other window, including full-screen ones.
//! 2. Be invisible to the taskbar, the dock, and the alt-tab switcher.
//! 3. Have per-pixel transparency.
//! 4. Optionally let clicks pass through to whatever is underneath.
//! 5. Never steal keyboard focus.
//!
//! None of those are provided by a portable windowing abstraction, which is
//! exactly why this crate exists instead of calling `winit` directly from the
//! application.
//!
//! # Layering
//!
//! ```text
//!                      lumina-app
//!                           |
//!             +-------------+-------------+
//!             |                           |
//!      OverlayBackend              MonitorSource
//!        (win32 / x11 /              (win32 / x11 /
//!         wayland / null)            stub for Phase 12)
//!             |                           |
//!             +-------------+-------------+
//!                           |
//!                   placement.rs
//!               (pure geometry, fully tested)
//! ```
//!
//! `placement` is deliberately pure: it takes a monitor rectangle, a position,
//! a size, and offsets, and returns a rectangle. No windowing code, no I/O, no
//! clock. That makes the geometry that decides where the island appears on the
//! user's screen completely testable, which matters because it is the one piece
//! of windowing logic that users notice immediately when it is wrong.
//!
//! # Capability reporting
//!
//! Platforms differ in what they will actually honour. Rather than pretend,
//! [`Capabilities`] records what the current platform supports and the caller
//! can degrade knowingly. The differences are enumerated in the project plan
//! under "Platform capability matrix".

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

pub mod capability;
pub mod monitor;
pub mod overlay;
pub mod placement;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod x11;

#[cfg(target_os = "windows")]
pub mod win32;

pub mod null;

pub use capability::{Capabilities, Platform, PlatformKind};
pub use monitor::{Monitor, MonitorSource, ScaleFactor};
pub use overlay::{OverlayBackend, OverlayError};
pub use placement::{compute_placement, fit_inside, Placement, WindowGeometry};

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
