//! Monitor enumeration and scale factors.
//!
//! A monitor is a rectangle in virtual desktop coordinates plus a scale factor.
//! That is the minimum a backend must supply, and it is everything the placement
//! code needs, which is why nothing else crosses the boundary.

use lumina_core::geometry::Rect;

/// Physical pixels per logical pixel.
///
/// A value of 1.0 is unscaled, 1.5 is 150 percent scaling, and so on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleFactor(f32);

impl ScaleFactor {
    /// Smallest scale we will honour.
    ///
    /// Below this a window becomes unusably small and division by the scale
    /// starts producing noisy coordinates.
    pub const MIN: f32 = 0.25;
    /// Largest scale we will honour.
    ///
    /// Above this, coordinates exceed what any realistic virtual desktop spans.
    pub const MAX: f32 = 8.0;

    /// Construct, rejecting values that are not usable.
    ///
    /// A non finite or non positive scale falls back to 1.0 rather than being
    /// clamped to the minimum, because a monitor that reports scale 0 is broken
    /// and pretending it is 0.25 is worse than pretending it is 1.0.
    #[must_use]
    pub fn new(value: f32) -> Self {
        if value.is_finite() && value > 0.0 {
            Self(value.clamp(Self::MIN, Self::MAX))
        } else {
            Self(1.0)
        }
    }

    /// Unscaled.
    #[must_use]
    pub const fn unscaled() -> Self {
        Self(1.0)
    }

    /// The raw value.
    #[must_use]
    pub const fn value(self) -> f32 {
        self.0
    }

    /// Convert a length from logical to physical pixels.
    #[must_use]
    pub fn to_physical(self, logical: f32) -> f32 {
        (logical * self.0).round()
    }

    /// Convert a length from physical to logical pixels.
    #[must_use]
    pub fn to_logical(self, physical: f32) -> f32 {
        physical / self.0
    }
}

impl Default for ScaleFactor {
    fn default() -> Self {
        Self::unscaled()
    }
}

/// One display.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// Stable index in the enumeration order.
    ///
    /// Index order is not stable across reboots on every platform, so this is
    /// for addressing within a session, not for permanent configuration.
    pub index: usize,
    /// Human readable name, for diagnostics and the config file.
    pub name: String,
    /// Rectangle in virtual desktop physical pixels.
    ///
    /// May have negative coordinates.
    pub bounds: Rect,
    /// Physical pixels per logical pixel.
    pub scale: ScaleFactor,
    /// Refresh rate in hertz, zero when the platform does not report it.
    pub refresh_hz: u32,
    /// True when this is the platform's designated primary display.
    pub is_primary: bool,
}

impl Monitor {
    /// Construct with defaults for everything but the essentials.
    #[must_use]
    pub fn new(index: usize, name: impl Into<String>, bounds: Rect, is_primary: bool) -> Self {
        Self {
            index,
            name: name.into(),
            bounds,
            scale: ScaleFactor::unscaled(),
            refresh_hz: 0,
            is_primary,
        }
    }

    /// True when this monitor contains the given virtual desktop point.
    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.bounds.contains(lumina_core::geometry::Vec2::new(x, y))
    }

    /// True when this monitor overlaps `rect` at all.
    #[must_use]
    pub fn intersects(&self, rect: Rect) -> bool {
        self.bounds.intersects(rect)
    }

    /// Area in square physical pixels.
    #[must_use]
    pub fn area(&self) -> f32 {
        self.bounds.area()
    }

    /// Width in logical pixels, which is what a user sees in their display
    /// settings.
    #[must_use]
    pub fn logical_width(&self) -> f32 {
        self.scale.to_logical(self.bounds.width)
    }

    /// Height in logical pixels.
    #[must_use]
    pub fn logical_height(&self) -> f32 {
        self.scale.to_logical(self.bounds.height)
    }
}

/// Where monitors come from.
///
/// A trait rather than a function so tests can supply a fixed list, and so the
/// Wayland backend added in Phase 12 can report one output at a time without the
/// caller caring.
pub trait MonitorSource: Send {
    /// Enumerate every attached monitor.
    ///
    /// Called at startup and whenever a display change event arrives. Must not
    /// panic: a backend that cannot enumerate reports an empty list and logs.
    fn monitors(&self) -> Vec<Monitor>;

    /// The monitor a point falls on, or `None` if it is off every display.
    fn monitor_at(&self, x: f32, y: f32) -> Option<Monitor> {
        // Later monitors win on overlap, which matches the behaviour of both
        // Windows and the common X11 setups where the most recently added
        // display is drawn on top.
        self.monitors().into_iter().rev().find(|m| m.contains(x, y))
    }

    /// The designated primary monitor.
    fn primary(&self) -> Option<Monitor> {
        let monitors = self.monitors();
        monitors
            .iter()
            .find(|m| m.is_primary)
            .cloned()
            .or_else(|| pick_primary_by_proximity(&monitors))
    }
}

/// Fall back to a monitor near the desktop origin when no monitor is flagged
/// primary.
///
/// Rule, in order:
/// 1. Smallest Manhattan distance of the monitor's origin from the desktop
///    origin. Users read "primary" as "the one with the menu bar", and the menu
///    bar is at the top left, so a monitor whose top left corner is near the
///    desktop origin is the one they mean.
/// 2. Largest area, as the tie break, because two monitors at the same distance
///    means a stacked arrangement and the larger one is the main one.
///
/// Getting this wrong puts the island on the wrong screen, which is exactly the
/// kind of bug that makes an overlay feel untrustworthy.
#[must_use]
pub fn pick_primary_by_proximity(monitors: &[Monitor]) -> Option<Monitor> {
    monitors
        .iter()
        .min_by(|a, b| {
            let rank = |m: &Monitor| (m.bounds.x.abs() + m.bounds.y.abs(), -m.area());
            rank(a)
                .partial_cmp(&rank(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
}

/// Resolve a monitor selector against a list.
///
/// # Panics
///
/// Never. An out of range index falls back to the primary monitor, because a
/// user who removed a display should not lose their island entirely.
#[must_use]
pub fn resolve_selector(
    monitors: &[Monitor],
    primary_index: usize,
    index: Option<usize>,
    cursor: Option<(f32, f32)>,
) -> Option<Monitor> {
    if let Some((x, y)) = cursor {
        if let Some(found) = monitors.iter().rev().find(|m| m.contains(x, y)) {
            return Some(found.clone());
        }
    }

    match index {
        Some(i) => monitors
            .get(i)
            .cloned()
            .or_else(|| monitors.get(primary_index).cloned()),
        None => monitors
            .get(primary_index)
            .cloned()
            .or_else(|| pick_primary_by_proximity(monitors)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumina_config::MonitorSelector;

    /// Build a fixed monitor for tests, with explicit coordinates.
    fn at(index: usize, x: f32, y: f32, width: f32, height: f32, primary: bool) -> Monitor {
        Monitor::new(
            index,
            format!("monitor-{index}"),
            Rect::new(x, y, width, height),
            primary,
        )
    }

    struct Fixed(Vec<Monitor>);

    impl MonitorSource for Fixed {
        fn monitors(&self) -> Vec<Monitor> {
            self.0.clone()
        }
    }

    #[test]
    fn scale_factor_rejects_unusable_values() {
        assert_eq!(
            ScaleFactor::new(0.0).value(),
            1.0,
            "zero falls back to unscaled"
        );
        assert_eq!(ScaleFactor::new(-1.0).value(), 1.0);
        assert_eq!(ScaleFactor::new(f32::NAN).value(), 1.0);
        assert_eq!(ScaleFactor::new(f32::INFINITY).value(), 1.0);
    }

    #[test]
    fn scale_factor_clamps_to_the_usable_range() {
        assert_eq!(ScaleFactor::new(0.01).value(), ScaleFactor::MIN);
        assert_eq!(ScaleFactor::new(1000.0).value(), ScaleFactor::MAX);
        assert_eq!(
            ScaleFactor::new(1.5).value(),
            1.5,
            "a usable value is untouched"
        );
    }

    #[test]
    fn scale_factor_converts_both_directions() {
        let scale = ScaleFactor::new(1.5);
        assert_eq!(scale.to_physical(100.0), 150.0);
        assert_eq!(scale.to_logical(150.0), 100.0);
        assert_eq!(ScaleFactor::unscaled().to_physical(100.0), 100.0);
    }

    #[test]
    fn scale_factor_rounds_to_whole_physical_pixels() {
        // 1.25 scaling of 101 is 126.25, which must become 126 rather than being
        // handed to the window manager as a fractional coordinate.
        assert_eq!(ScaleFactor::new(1.25).to_physical(101.0), 126.0);
    }

    #[test]
    fn monitor_containment_handles_negative_origins() {
        // A monitor to the left of the primary spans negative x, from -1920 to 0.
        let left = at(1, -1920.0, 0.0, 1920.0, 1080.0, false);
        assert!(left.contains(-1900.0, 500.0));
        assert!(left.contains(-10.0, 500.0));
        assert!(!left.contains(10.0, 500.0), "positive x is on the primary");
        assert!(!left.contains(-1900.0, 2000.0), "below the monitor");
    }

    #[test]
    fn monitor_logical_size_divides_by_the_scale() {
        let mut m = at(0, 0.0, 0.0, 3840.0, 2160.0, true);
        m.scale = ScaleFactor::new(2.0);
        assert_eq!(m.logical_width(), 1920.0);
        assert_eq!(m.logical_height(), 1080.0);
    }

    #[test]
    fn primary_is_the_flagged_monitor() {
        let source = Fixed(vec![
            at(0, -1920.0, 0.0, 1920.0, 1080.0, false),
            at(1, 0.0, 0.0, 1920.0, 1080.0, true),
        ]);
        assert_eq!(source.primary().map(|m| m.index), Some(1));
    }

    #[test]
    fn primary_falls_back_to_the_one_nearest_the_desktop_origin() {
        // No monitor is flagged, which happens on some X11 setups. The one whose
        // top left corner is at the origin is the one with the menu bar.
        let source = Fixed(vec![
            at(0, -1920.0, 0.0, 1920.0, 1080.0, false),
            at(1, 0.0, 0.0, 1280.0, 720.0, false),
        ]);
        let primary = source.primary().expect("one monitor should be chosen");
        assert_eq!(
            primary.index, 1,
            "the monitor at the origin wins over the one to its left"
        );
    }

    #[test]
    fn proximity_beats_area_when_the_two_disagree() {
        // A large monitor to the right does not become the primary just because
        // it is large. Proximity is the stronger signal.
        let source = Fixed(vec![
            at(0, 0.0, 0.0, 800.0, 600.0, false),
            at(1, 100.0, 0.0, 3840.0, 2160.0, false),
        ]);
        assert_eq!(source.primary().map(|m| m.index), Some(0));
    }

    #[test]
    fn area_breaks_the_tie_between_equally_close_monitors() {
        // Two monitors stacked at the same distance from the origin: the larger
        // one is the main one.
        let source = Fixed(vec![
            at(0, 0.0, 0.0, 800.0, 600.0, false),
            at(1, 0.0, 0.0, 3840.0, 2160.0, false),
        ]);
        assert_eq!(source.primary().map(|m| m.index), Some(1));
    }

    #[test]
    fn monitor_at_finds_the_display_under_a_point() {
        let source = Fixed(vec![
            at(0, 0.0, 0.0, 1920.0, 1080.0, true),
            at(1, 1920.0, 0.0, 2560.0, 1440.0, false),
        ]);
        assert_eq!(source.monitor_at(100.0, 100.0).map(|m| m.index), Some(0));
        assert_eq!(source.monitor_at(2000.0, 100.0).map(|m| m.index), Some(1));
        assert!(
            source.monitor_at(-500.0, -500.0).is_none(),
            "off every display"
        );
    }

    #[test]
    fn selector_index_wins_when_in_range() {
        let monitors = vec![
            at(0, 0.0, 0.0, 1920.0, 1080.0, true),
            at(1, 1920.0, 0.0, 2560.0, 1440.0, false),
        ];
        let chosen = resolve_selector(&monitors, 0, Some(1), None).expect("a monitor");
        assert_eq!(chosen.index, 1);
    }

    #[test]
    fn selector_cursor_wins_over_index() {
        let monitors = vec![
            at(0, 0.0, 0.0, 1920.0, 1080.0, true),
            at(1, 1920.0, 0.0, 2560.0, 1440.0, false),
        ];
        let chosen =
            resolve_selector(&monitors, 0, Some(0), Some((2000.0, 10.0))).expect("a monitor");
        assert_eq!(
            chosen.index, 1,
            "the cursor is on monitor 1, so that is where the island goes"
        );
    }

    #[test]
    fn cursor_off_every_display_falls_back_to_the_index() {
        let monitors = vec![
            at(0, 0.0, 0.0, 1920.0, 1080.0, true),
            at(1, 1920.0, 0.0, 2560.0, 1440.0, false),
        ];
        let chosen =
            resolve_selector(&monitors, 0, Some(1), Some((-9000.0, -9000.0))).expect("a monitor");
        assert_eq!(chosen.index, 1);
    }

    #[test]
    fn an_out_of_range_index_falls_back_to_primary() {
        // A user who unplugged the display their config named should still get
        // their island, on the primary, rather than nothing.
        let monitors = vec![at(0, 0.0, 0.0, 1920.0, 1080.0, true)];
        let chosen = resolve_selector(&monitors, 0, Some(7), None).expect("a monitor");
        assert_eq!(chosen.index, 0);
    }

    #[test]
    fn no_monitors_yields_none_rather_than_a_panic() {
        assert!(resolve_selector(&[], 0, None, None).is_none());
        assert!(resolve_selector(&[], 0, Some(3), None).is_none());
    }

    #[test]
    fn selector_enum_maps_onto_resolution() {
        // The config enum and the resolution function have to agree, so this
        // checks the mapping rather than the enum in isolation.
        let monitors = vec![
            at(0, 0.0, 0.0, 1920.0, 1080.0, true),
            at(1, 1920.0, 0.0, 2560.0, 1440.0, false),
        ];

        let for_selector = |selector: MonitorSelector| {
            let (index, cursor) = match selector {
                MonitorSelector::Primary => (None, None),
                MonitorSelector::Cursor => (None, Some((3000.0, 100.0))),
                MonitorSelector::Index(i) => (Some(i as usize), None),
            };
            resolve_selector(&monitors, 0, index, cursor)
        };

        assert_eq!(
            for_selector(MonitorSelector::Primary).map(|m| m.index),
            Some(0)
        );
        assert_eq!(
            for_selector(MonitorSelector::Index(1)).map(|m| m.index),
            Some(1)
        );
        assert_eq!(
            for_selector(MonitorSelector::Cursor).map(|m| m.index),
            Some(1)
        );
    }

    #[test]
    fn intersection_detects_overlapping_monitors() {
        let a = at(0, 0.0, 0.0, 1920.0, 1080.0, true);
        assert!(a.intersects(Rect::new(1900.0, 1000.0, 100.0, 100.0)));
        assert!(!a.intersects(Rect::new(2000.0, 100.0, 100.0, 100.0)));
    }

    #[test]
    fn primary_breaks_the_tie_between_equally_close_monitors_by_area() {
        // Two monitors at the same distance from the origin: the larger one is
        // the main one.
        let source = Fixed(vec![
            at(0, 0.0, 0.0, 800.0, 600.0, false),
            at(1, 0.0, 0.0, 3840.0, 2160.0, false),
        ]);
        assert_eq!(source.primary().map(|m| m.index), Some(1));
    }
}
