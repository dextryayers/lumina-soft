//! Shared mathematical and state primitives for Lumina.
//!
//! This crate is the bottom of the dependency graph. It depends on `serde` and
//! nothing else internal, which keeps it trivially testable and lets every other
//! crate depend on it without creating a cycle.
//!
//! Contents:
//!
//! * [`geometry`] : sizes, rectangles, insets, and island geometry plus clamping.
//! * [`sdf`] : analytic signed distance fields for the island shape.
//! * [`spring`] : damped harmonic oscillators driving every animation.
//! * [`color`] : sRGB, linear, OkLab, and WCAG contrast utilities.
//! * [`state`] : island states, normalized input events, and the state machine.
//! * [`time`] : monotonic clocks, frame pacing, and fixed step integration.

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

pub mod color;
pub mod geometry;
pub mod sdf;
pub mod spring;
pub mod state;
pub mod time;

pub use geometry::{Geometry, Insets, Rect, Scalar, Size, Vec2};
pub use spring::Spring;
pub use state::{IslandState, StateMachine, TransitionSource};

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
