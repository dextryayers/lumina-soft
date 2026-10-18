//! Widget framework for Lumina.
//!
//! # What this crate provides
//!
//! | Module | Responsibility |
//! |---|---|
//! | `trait` | The [`Widget`] trait, [`WidgetContext`], [`WidgetInput`], [`DrawList`], [`Quad`] |
//! | `layout` | [`FlexLayout`]: the single-row flexbox that arranges widgets |
//! | `clock` | [`ClockWidget`]: time, date, second timezone |
//! | `calendar` | [`CalendarWidget`]: month grid with today highlighted |
//!
//! # Design rules
//!
//! 1. **The frame path allocates nothing.** Widgets emit quads into a
//!    pre-allocated [`DrawList`] that is cleared and refilled each frame.
//! 2. **Layout is a pure function** of the available size and the widgets'
//!    desired sizes, so it is testable without a window.
//! 3. **Input is routed by hit testing**, not by widget order, so a widget that
//!    is visually on top receives the events that land on it.

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

pub mod calendar;
pub mod clock;
pub mod layout;
pub mod trait;

pub use calendar::CalendarWidget;
pub use clock::ClockWidget;
pub use layout::{layout_row, FlexLayout};
pub use trait::{DrawList, Quad, Widget, WidgetContext, WidgetInput, WidgetRegistry};

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");