//! Configuration schema for Lumina.
//!
//! # Design
//!
//! The whole configuration is one deserializable tree of plain structs with a
//! `Default` impl per section. That gives three things at once:
//!
//! 1. Typed access with no string keys at the call site.
//! 2. A single source of truth for every default, so the shipped file and the
//!    compiled fallback can never drift apart.
//! 3. Serde's own error messages, which name the exact path that failed.
//!
//! Validation is deliberately *not* done by serde. A single wrong value must not
//! cause the whole file to be rejected, because a user who mistyped one RGB
//! value should not lose their entire configuration. See [`crate::validate`].

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

pub mod defaults;
pub mod profile;
pub mod schema;
pub mod theme;
pub mod validate;
pub mod watch;

pub use schema::{
    AppearanceConfig, AudioConfig, BatteryConfig, Config, GeneralConfig, HotkeysConfig, LayoutConfig,
    MediaConfig, MetricsConfig, MotionConfig, NetworkConfig, Placement, Position, PomodoroConfig,
    RgbConfig, ShortcutConfig, SpectrumConfig, TextConfig, TrayConfig, VisualizerStyle, WeatherConfig,
    WidgetConfig,
};
pub use validate::{Issue, Severity, ValidationReport, validate};
pub use profile::{ProfileStore, resolve};

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Schema version written into the config file.
///
/// Bumped only when a key changes meaning or is removed, never when a key is
/// merely added. See [`schema::Config::version`].
pub const SCHEMA_VERSION: u32 = 1;