//! The clock widget: time, date, and an optional second timezone.

use lumina_core::geometry::{Rect, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};

use crate::trait::{DrawList, Quad, Widget, WidgetContext, WidgetInput};

/// The clock widget.
#[derive(Debug, Clone)]
pub struct ClockWidget {
    /// The widget's id.
    pub id: &'static str,
    /// Whether to show seconds.
    pub show_seconds: bool,
    /// Whether to show the date.
    pub show_date: bool,
    /// Date format string.
    pub date_format: String,
    /// 12 or 24 hour.
    pub format_24h: bool,
    /// Whether to show the weekday.
    pub show_weekday: bool,
    /// A second timezone, if configured.
    pub second_tz: Option<String>,
    /// Whether to pad the hour with a leading zero.
    pub hour_leading_zero: bool,
}

impl ClockWidget {
    /// Construct a clock widget with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: "clock",
            show_seconds: false,
            show_date: true,
            date_format: "%d %b".to_string(),
            format_24h: true,
            show_weekday: false,
            second_tz: None,
            hour_leading_zero: true,
        }
    }

    /// Format the current time as a string.
    ///
    /// Uses a simple formatter rather than pulling in `chrono` for the widget
    /// itself, because the widget only needs to format the time it is given.
    /// The orchestrator passes in the current time as a `(hour, minute, second)`
    /// tuple.
    #[must_use]
    pub fn format_time(&self, hour: u32, minute: u32, second: u32) -> String {
        if self.format_24h {
            if self.show_seconds {
                format!("{:02}:{:02}:{:02}", hour, minute, second)
            } else {
                format!("{:02}:{:02}", hour, minute)
            }
        } else {
            let period = if hour < 12 { "AM" } else { "PM" };
            let h12 = if hour == 0 {
                12
            } else if hour > 12 {
                hour - 12
            } else {
                hour
            };
            if self.hour_leading_zero {
                if self.show_seconds {
                    format!("{:02}:{:02}:{:02} {}", h12, minute, second, period)
                } else {
                    format!("{:02}:{:02} {}", h12, minute, period)
                }
            } else if self.show_seconds {
                format!("{}:{:02}:{:02} {}", h12, minute, second, period)
            } else {
                format!("{}:{:02} {}", h12, minute, period)
            }
        }
    }

    /// Format the date as a string.
    ///
    /// A minimal strftime-like formatter for the format strings Lumina
    /// documents. Supports `%d`, `%b`, `%B`, `%m`, `%y`, `%Y`, `%a`, `%A`.
    #[must_use]
    pub fn format_date(&self, year: i32, month: u32, day: u32, weekday: u32) -> String {
        let mut result = String::new();
        let mut chars = self.date_format.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('d') => result.push_str(&format!("{:02}", day)),
                    Some('b') => result.push_str(short_month(month)),
                    Some('B') => result.push_str(long_month(month)),
                    Some('m') => result.push_str(&format!("{:02}", month)),
                    Some('y') => result.push_str(&format!("{:02}", (year % 100).abs())),
                    Some('Y') => result.push_str(&format!("{:04}", year)),
                    Some('a') => result.push_str(short_weekday(weekday)),
                    Some('A') => result.push_str(long_weekday(weekday)),
                    Some('%') => result.push('%'),
                    Some(other) => {
                        result.push('%');
                        result.push(other);
                    }
                    None => result.push('%'),
                }
            } else {
                result.push(c);
            }
        }
        result
    }
}

impl Default for ClockWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ClockWidget {
    fn id(&self) -> &'static str {
        self.id
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        // The clock wants enough room for "00:00" or "00:00:00" plus the date.
        let width = if self.show_seconds { 80.0 } else { 60.0 };
        let height = 20.0;
        Size::new(width, height)
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        // Emit a single quad for the clock's background. The actual text is
        // drawn by the renderer using the MSDF atlas.
        out.push(Quad::new(
            Vec2::new(rect.x, rect.y),
            rect.size(),
            Vec2::ZERO,
            Vec2::ZERO,
            [0.0, 0.0, 0.0, 0.0],
        ));
    }

    fn on_input(&mut self, _ev: &WidgetInput, _ctx: &WidgetContext, _rect: Rect) -> bool {
        false
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        None
    }

    fn priority(&self) -> u8 {
        IslandState::Clock.priority()
    }
}

fn short_month(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    }
}

fn long_month(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}

fn short_weekday(weekday: u32) -> &'static str {
    match weekday {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        6 => "Sat",
        _ => "???",
    }
}

fn long_weekday(weekday: u32) -> &'static str {
    match weekday {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_clock_has_sensible_defaults() {
        let clock = ClockWidget::new();
        assert_eq!(clock.id, "clock");
        assert!(!clock.show_seconds);
        assert!(clock.show_date);
        assert!(clock.format_24h);
    }

    #[test]
    fn 24_hour_format_pads_the_hour() {
        let clock = ClockWidget::new();
        assert_eq!(clock.format_time(9, 5, 3), "09:05");
    }

    #[test]
    fn 24_hour_format_shows_seconds_when_configured() {
        let mut clock = ClockWidget::new();
        clock.show_seconds = true;
        assert_eq!(clock.format_time(9, 5, 3), "09:05:03");
    }

    #[test]
    fn 12_hour_format_converts_midnight_and_noon() {
        let mut clock = ClockWidget::new();
        clock.format_24h = false;
        assert_eq!(clock.format_time(0, 0, 0), "12:00 AM");
        assert_eq!(clock.format_time(12, 0, 0), "12:00 PM");
        assert_eq!(clock.format_time(13, 30, 0), "01:30 PM");
    }

    #[test]
    fn 12_hour_format_can_omit_the_leading_zero() {
        let mut clock = ClockWidget::new();
        clock.format_24h = false;
        clock.hour_leading_zero = false;
        assert_eq!(clock.format_time(9, 5, 0), "9:05 AM");
    }

    #[test]
    fn the_date_format_supports_the_documented_specifiers() {
        let clock = ClockWidget::new();
        assert_eq!(clock.format_date(2026, 10, 19, 1), "19 Oct");
    }

    #[test]
    fn the_date_format_supports_full_month_and_year() {
        let mut clock = ClockWidget::new();
        clock.date_format = "%d %B %Y".to_string();
        assert_eq!(clock.format_date(2026, 10, 19, 1), "19 October 2026");
    }

    #[test]
    fn the_date_format_supports_weekday_names() {
        let mut clock = ClockWidget::new();
        clock.date_format = "%A".to_string();
        assert_eq!(clock.format_date(2026, 10, 19, 1), "Monday");
    }

    #[test]
    fn the_date_format_handles_a_literal_percent() {
        let mut clock = ClockWidget::new();
        clock.date_format = "100%%".to_string();
        assert_eq!(clock.format_date(2026, 10, 19, 1), "100%");
    }

    #[test]
    fn the_date_format_passes_through_unknown_specifiers() {
        let mut clock = ClockWidget::new();
        clock.date_format = "%Q".to_string();
        assert_eq!(clock.format_date(2026, 10, 19, 1), "%Q");
    }

    #[test]
    fn the_clock_measures_a_reasonable_size() {
        let mut clock = ClockWidget::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        let size = clock.measure(&ctx);
        assert!(size.width > 0.0);
        assert!(size.height > 0.0);
    }

    #[test]
    fn the_clock_emits_a_quad_when_drawn() {
        let mut clock = ClockWidget::new();
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        clock.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 100.0, 20.0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn the_clock_does_not_consume_input() {
        let mut clock = ClockWidget::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        assert!(!clock.on_input(&WidgetInput::Click(Vec2::new(50.0, 10.0)), &ctx, Rect::new(0.0, 0.0, 100.0, 20.0)));
    }

    #[test]
    fn the_clock_requests_no_state() {
        let clock = ClockWidget::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        assert!(clock.request(&ctx).is_none());
    }

    #[test]
    fn the_clock_priority_matches_the_state_arbiter() {
        let clock = ClockWidget::new();
        assert_eq!(clock.priority(), IslandState::Clock.priority());
    }
}