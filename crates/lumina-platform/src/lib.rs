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

pub mod accessibility;
pub mod autostart;
pub mod capability;
pub mod hotkey;
pub mod memory;
pub mod monitor;
pub mod overlay;
pub mod placement;
pub mod single_instance;
pub mod time;
pub mod tray;
pub mod wayland;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod wayland_hotkeys;
#[cfg(all(unix, not(target_os = "macos")))]
pub mod x11;
#[cfg(all(unix, not(target_os = "macos")))]
pub mod x11_hotkeys;

#[cfg(target_os = "windows")]
pub mod win32;
#[cfg(target_os = "windows")]
pub mod win32_hotkeys;

pub mod null;

pub use accessibility::{
    deliver_announcement, notification_command, os_reduced_motion, os_screen_reader_active,
    screen_reader_enabled, AnnouncementGate, REDUCED_MOTION_ENV, SCREEN_READER_ENV,
};
pub use autostart::{AutostartError, WINDOWS_RUN_KEY, WINDOWS_VALUE_NAME};
pub use capability::{Capabilities, Platform, PlatformKind};
pub use hotkey::{
    bindings_from_config, configured_hotkeys, conflicts_from_config, find_conflicts,
    hotkey_problems, Hotkey, HotkeyBinding, HotkeyConflict, HotkeyError, HotkeyEvent,
    HotkeyParseError, HotkeyRegistry, Key, Modifiers, NullHotkeyRegistry,
};
pub use memory::{process_memory, ProcessMemory};
pub use monitor::{Monitor, MonitorSource, ScaleFactor};
pub use overlay::{OverlayBackend, OverlayError};
pub use placement::{compute_placement, fit_inside, Placement, WindowGeometry};
pub use single_instance::{claim, is_running, notify_existing, AlreadyRunning, InstanceGuard};
pub use time::{
    civil_from_days, civil_from_unix_with_offset, local_civil_time, local_utc_offset_minutes,
    weekday_from_days, CivilTime,
};
pub use tray::{TrayCommand, TrayIcon, TrayMenu, TrayMenuItem, TrayMenuState, TrayState};

#[cfg(all(unix, not(target_os = "macos")))]
pub use wayland::{WaylandBackend, WaylandSurfaceKind};

#[cfg(all(unix, not(target_os = "macos")))]
pub use x11_hotkeys::X11Hotkeys;

#[cfg(all(unix, not(target_os = "macos")))]
pub use wayland_hotkeys::{session_is_wayland, session_is_wayland_from, WaylandHotkeys};

#[cfg(target_os = "windows")]
pub use win32_hotkeys::Win32Hotkeys;

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
