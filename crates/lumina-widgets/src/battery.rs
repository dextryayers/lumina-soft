//! The battery widget: charge, charging state, and time remaining.
//!
//! # Missing batteries
//!
//! Desktops, virtual machines, and many containers have no battery. The widget
//! reports `present == false` through [`BatterySnapshot`] and draws nothing;
//! requesting a state it cannot fill would make the island expand to an empty
//! card.
//!
//! # Low battery alerts
//!
//! When the charge crosses the low threshold while discharging, the widget
//! requests the metrics state for a few seconds. The hold is set once on the
//! transition, so a battery sitting at four percent does not keep the island
//! pinned forever.

use std::time::{SystemTime, UNIX_EPOCH};

use lumina_config::schema::BatteryConfig;
use lumina_core::geometry::{Rect, Size};
use lumina_core::state::{IslandState, StateRequest};

use crate::paint::{self, CRITICAL, DIM, GOOD, MUTED, TRACK, WARN};
use crate::snapshot::{format_short_duration, BatterySnapshot};
use crate::widget::{DrawList, Widget, WidgetContext, WidgetInput};

/// Charge at or below which the widget alerts, in percent.
pub const LOW_BATTERY_PERCENT: f32 = 15.0;
/// How long a low battery alert holds the state open.
const ALERT_HOLD_MS: u64 = 6_000;

/// The battery widget.
#[derive(Debug, Clone)]
pub struct BatteryWidget {
    /// Whether to show the percentage as text.
    pub show_percentage: bool,
    /// Whether to show the time estimate.
    pub show_time_remaining: bool,
    /// Whether to draw the charging bolt.
    pub show_charging_bolt: bool,
    /// The sampling interval from config, exposed for the service layer.
    pub interval_ms: u32,
    /// The latest reading.
    pub snapshot: BatterySnapshot,
    was_low: bool,
    alert_until_ms: Option<u64>,
}

impl BatteryWidget {
    /// Construct a widget from the battery configuration.
    #[must_use]
    pub fn from_config(config: &BatteryConfig) -> Self {
        Self {
            show_percentage: config.show_percentage,
            show_time_remaining: config.show_time_remaining,
            show_charging_bolt: config.show_charging_bolt,
            interval_ms: config.interval_ms.max(200),
            snapshot: BatterySnapshot::default(),
            was_low: false,
            alert_until_ms: None,
        }
    }

    /// Construct a widget with every element enabled.
    #[must_use]
    pub fn new() -> Self {
        Self::from_config(&BatteryConfig {
            enabled: true,
            ..BatteryConfig::default()
        })
    }

    /// Store a new reading, evaluated at the monotonic millisecond `now_ms`.
    pub fn update(&mut self, snapshot: BatterySnapshot, now_ms: u64) {
        self.snapshot = snapshot.sanitized();
        let low = self.is_low();
        if low && !self.was_low {
            self.alert_until_ms = Some(now_ms.saturating_add(ALERT_HOLD_MS));
        }
        self.was_low = low;
    }

    /// True when the battery is present, discharging, and at or below the low
    /// threshold.
    #[must_use]
    pub fn is_low(&self) -> bool {
        self.snapshot.is_usable()
            && !self.snapshot.charging
            && self.snapshot.percent <= LOW_BATTERY_PERCENT
    }

    /// The display color for the current charge.
    #[must_use]
    pub fn charge_color(&self) -> [f32; 4] {
        if !self.snapshot.is_usable() {
            MUTED
        } else if self.snapshot.percent <= LOW_BATTERY_PERCENT {
            CRITICAL
        } else if self.snapshot.percent <= 35.0 {
            WARN
        } else {
            GOOD
        }
    }

    /// The time estimate rendered for display, when configured and available.
    #[must_use]
    pub fn time_text(&self) -> Option<String> {
        if !self.show_time_remaining {
            return None;
        }
        self.snapshot
            .time_remaining_secs
            .map(format_short_duration)
    }

    fn alert_is_live(&self) -> bool {
        self.alert_until_ms
            .is_some_and(|until| now_ms() < until)
    }
}

impl Default for BatteryWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for BatteryWidget {
    fn id(&self) -> &'static str {
        "battery"
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        let width = if self.show_time_remaining { 92.0 } else { 66.0 };
        Size::new(width, 20.0)
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        if !self.snapshot.is_usable() {
            return;
        }

        let shell = Rect::new(rect.x, rect.y + 3.0, 34.0, 14.0);
        paint::block(out, shell, TRACK, 4.0);
        paint::block(
            out,
            Rect::new(shell.right() + 1.0, shell.y + 4.0, 2.0, 6.0),
            TRACK,
            1.0,
        );

        let inset = shell.width * self.snapshot.fraction();
        if inset > 0.0 {
            paint::block(
                out,
                Rect::new(shell.x + 2.0, shell.y + 2.0, (inset - 4.0).max(1.0), 10.0),
                self.charge_color(),
                2.0,
            );
        }

        if self.snapshot.charging && self.show_charging_bolt {
            let bolt = Rect::new(shell.x + 14.0, shell.y + 3.0, 5.0, 8.0);
            paint::block(out, bolt, [1.0, 1.0, 1.0, 0.9], 1.0);
        }

        let mut text_x = shell.right() + 8.0;
        if self.show_percentage {
            paint::label(out, Rect::new(text_x, rect.y + 5.0, 22.0, 10.0), DIM);
            text_x += 26.0;
        }
        if self.time_text().is_some() {
            paint::label(
                out,
                Rect::new(text_x, rect.y + 5.0, (rect.right() - text_x).max(1.0), 10.0),
                DIM,
            );
        }
    }

    fn on_input(&mut self, _ev: &WidgetInput, _ctx: &WidgetContext, _rect: Rect) -> bool {
        false
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        if self.alert_is_live() {
            Some(StateRequest::until(
                IslandState::Metrics,
                self.alert_until_ms.unwrap_or_else(now_ms),
            ))
        } else {
            None
        }
    }

    fn priority(&self) -> u8 {
        IslandState::Metrics.priority()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> WidgetContext {
        WidgetContext::new(IslandState::Metrics, Size::new(200.0, 40.0))
    }

    fn battery(percent: f32, charging: bool) -> BatterySnapshot {
        BatterySnapshot {
            present: true,
            percent,
            charging,
            time_remaining_secs: Some(3_600),
        }
    }

    #[test]
    fn a_machine_without_a_battery_draws_nothing() {
        let mut widget = BatteryWidget::new();
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 66.0, 20.0));
        assert!(out.is_empty());
    }

    #[test]
    fn a_usable_battery_draws_shell_fill_and_labels() {
        let mut widget = BatteryWidget::new();
        widget.update(battery(50.0, false), 0);
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 92.0, 20.0));
        assert!(out.len() >= 4);
    }

    #[test]
    fn charge_color_escalates_with_low_charge() {
        let mut widget = BatteryWidget::new();
        widget.update(battery(10.0, false), 0);
        assert_eq!(widget.charge_color(), CRITICAL);
        widget.update(battery(25.0, false), 0);
        assert_eq!(widget.charge_color(), WARN);
        widget.update(battery(80.0, false), 0);
        assert_eq!(widget.charge_color(), GOOD);
        assert_eq!(BatteryWidget::new().charge_color(), MUTED);
    }

    #[test]
    fn a_low_battery_requests_the_metrics_state() {
        let mut widget = BatteryWidget::new();
        widget.update(battery(10.0, false), 1_000);
        let request = widget.request(&ctx()).expect("low battery alerts");
        assert_eq!(request.state, IslandState::Metrics);
        assert_eq!(request.expires_at_ms, Some(7_000));
    }

    #[test]
    fn a_charging_battery_never_alerts() {
        let mut widget = BatteryWidget::new();
        widget.update(battery(5.0, true), 1_000);
        assert!(!widget.is_low());
        assert!(widget.request(&ctx()).is_none());
    }

    #[test]
    fn the_alert_is_set_once_per_transition() {
        let mut widget = BatteryWidget::new();
        widget.update(battery(30.0, false), 1_000);
        widget.update(battery(10.0, false), 2_000);
        assert_eq!(widget.alert_until_ms, Some(8_000));
        widget.update(battery(9.0, false), 3_000);
        assert_eq!(widget.alert_until_ms, Some(8_000));
    }

    #[test]
    fn the_time_estimate_respects_the_configuration() {
        let mut widget = BatteryWidget::new();
        widget.update(battery(50.0, false), 0);
        assert_eq!(widget.time_text().as_deref(), Some("1h 00m"));
        widget.show_time_remaining = false;
        assert_eq!(widget.time_text(), None);
    }

    #[test]
    fn charging_draws_the_bolt() {
        let mut widget = BatteryWidget::new();
        widget.update(battery(50.0, true), 0);
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 92.0, 20.0));
        assert!(out.len() >= 5);
    }
}
