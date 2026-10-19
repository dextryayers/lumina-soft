//! Wayland support: layer-shell placement, configure handling, and fallback.
//!
//! # Why Wayland needs its own module
//!
//! On X11 and Windows a client asks the display server for a rectangle and gets
//! it. On Wayland a client cannot say "put me at (960, 12)". The compositor
//! owns placement, and the only protocol that lets an ordinary application
//! choose an edge, a margin, and an exclusive zone is
//! `zwlr_layer_shell_v1`, which is a wlroots extension. Compositors that do not
//! implement it, GNOME being the important one, force a choice between an
//! `xdg_toplevel` window the compositor centres, or no window at all.
//!
//! The parts that can be decided without a compositor live here as pure
//! functions: which anchors to request, which margins express the user's
//! offsets, how much space to reserve, when an input region must be empty, and
//! how to handle a configure event without racing the compositor. That is what
//! the tests cover on every platform, including CI runners with no display
//! server at all.
//!
//! # The configure handshake
//!
//! A layer surface must create a surface, commit it once with no buffer to ask
//! for a configure, wait for `configure(serial, width, height)`, apply the size,
//! then acknowledge that exact serial. Acking before applying, or acking a stale
//! serial, leaves a buffer of the wrong size attached and produces the classic
//! one frame flicker. [`configure::ConfigureTracker`] makes the order explicit
//! and rejects serials that are not newer than the last one seen.

pub mod configure;
pub mod detect;
pub mod idle;
pub mod placement;

#[cfg(all(unix, not(target_os = "macos")))]
mod backend;

#[cfg(all(unix, not(target_os = "macos")))]
pub use backend::{KeyEvent, KeyboardModifiers, WaylandBackend, WaylandSurfaceKind};
