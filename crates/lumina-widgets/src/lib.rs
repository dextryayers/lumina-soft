//! Widget framework for Lumina.
//!
//! # What this crate provides
//!
//! | Module | Responsibility |
//! |---|---|
//! | `widget` | The [`Widget`] trait, [`WidgetContext`], [`WidgetInput`], [`DrawList`], [`Quad`], [`WidgetRegistry`] |
//! | `layout` | [`FlexLayout`]: the single-row flexbox that arranges widgets |
//! | `clock` | [`ClockWidget`]: time, date, second timezone |
//! | `calendar` | [`CalendarWidget`]: month grid with today highlighted |
//! | `config` | Config-driven registry: widget order and max_widgets |
//!
//! # Design rules
//!
//! 1. **The frame path allocates nothing.** Widgets emit quads into a
//!    pre-allocated [`DrawList`] that is cleared and refilled each frame.
//! 2. **Layout is a pure function** of the available size and the widgets'
//!    desired sizes, so it is testable without a window.
//! 3. **Input is routed by hit testing**, not by widget order, so a widget that
//!    is visually on top receives the events that land on it.
//! 4. **Lifecycle hooks** let widgets acquire and release resources without
//!    polling, and react to state transitions.

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

pub mod alloc_test;
pub mod battery;
pub mod calendar;
pub mod clock;
pub mod config;
pub mod input;
pub mod layout;
pub mod media;
pub mod metrics;
pub mod network;
pub mod notify;
pub mod paint;
pub mod pomodoro;
pub mod script;
pub mod scrub;
pub mod shortcuts;
pub mod snapshot;
pub mod volume;
pub mod weather;
pub mod widget;

pub use alloc_test::{active_allocations, total_allocations, total_deallocations, reset_counters, FrameGuard};
pub use battery::BatteryWidget;
pub use calendar::CalendarWidget;
pub use clock::ClockWidget;
pub use config::{build_registry, registry_from_config, update_registry};
pub use input::InputRouter;
pub use layout::{layout_row, layout_row_with_options, FlexLayout, VerticalAlign};
pub use media::MediaWidget;
pub use metrics::MetricsWidget;
pub use network::NetworkWidget;
pub use notify::{IslandNotification, NotificationWidget};
pub use pomodoro::{PomodoroEvent, PomodoroPhase, PomodoroTimer, PomodoroWidget};
pub use script::{ScriptStatus, ScriptWidget};
pub use scrub::ScrubController;
pub use shortcuts::ShortcutsWidget;
pub use snapshot::{
    format_countdown, format_short_duration, BatterySnapshot, FormattedRate, History,
    MetricsSnapshot, NetworkSnapshot, WeatherCache, WeatherCondition, WeatherSnapshot,
};
pub use volume::VolumeSlider;
pub use weather::{
    parse_open_meteo, parse_open_weather_map, request_url, WeatherError, WeatherTransport,
    WeatherWidget,
};
pub use widget::{DrawList, Quad, Widget, WidgetContext, WidgetInput, WidgetRegistry};

#[cfg(feature = "http")]
pub use weather::UreqTransport;

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");