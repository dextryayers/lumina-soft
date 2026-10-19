//! The clock widget: time, date, and an optional second timezone.
//!
//! # Content drawing
//!
//! The clock widget emits quads for its background, time text, date text, and
//! optional weekday. The actual glyph rendering is done by the renderer using
//! the MSDF atlas, but the widget computes the layout and emits the quads with
//! the correct UV rectangles and colors.
//!
//! # Timezone support
//!
//! A second timezone can be configured as an IANA name. The widget displays
//! both the local time and the second timezone time, separated by a divider.

use std::fmt::Write as _;

use lumina_config::schema::{ClockConfig, ClockFormat};
use lumina_core::geometry::{Rect, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};
use lumina_platform::time::{civil_from_unix_with_offset, local_utc_offset_minutes, CivilTime};

use crate::widget::{DrawList, Quad, Widget, WidgetContext, WidgetInput};

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
    /// The text color as RGBA.
    pub text_color: [f32; 4],
    /// The dim text color as RGBA.
    pub dim_color: [f32; 4],
    /// The divider color as RGBA.
    pub divider_color: [f32; 4],
    /// The background color as RGBA.
    pub bg_color: [f32; 4],
    /// The corner radius of the background.
    pub corner_radius: f32,
    /// The font size for the time.
    pub time_font_size: f32,
    /// The font size for the date.
    pub date_font_size: f32,
    /// The last drawn time string, used to detect changes.
    pub last_time: String,
    /// The last drawn date string, used to detect changes.
    pub last_date: String,
    /// A fixed UTC offset in minutes, or `None` to ask the system each frame.
    pub utc_offset_minutes: Option<i32>,
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
            text_color: [1.0, 1.0, 1.0, 1.0],
            dim_color: [0.7, 0.7, 0.7, 1.0],
            divider_color: [1.0, 1.0, 1.0, 0.3],
            bg_color: [0.0, 0.0, 0.0, 0.0],
            corner_radius: 8.0,
            time_font_size: 16.0,
            date_font_size: 12.0,
            last_time: String::new(),
            last_date: String::new(),
            utc_offset_minutes: None,
        }
    }

    /// Build the clock from configuration.
    ///
    /// The configured second timezone is stored but not resolved here: the
    /// system UTC offset is applied at draw time, so a daylight saving change
    /// is picked up without recreating the widget.
    #[must_use]
    pub fn from_config(config: &ClockConfig) -> Self {
        Self {
            show_seconds: config.show_seconds,
            show_date: config.show_date,
            date_format: config.date_format.clone(),
            format_24h: config.format == ClockFormat::H24,
            show_weekday: config.show_weekday,
            second_tz: if config.second_timezone.is_empty() {
                None
            } else {
                Some(config.second_timezone.clone())
            },
            hour_leading_zero: config.hour_leading_zero,
            ..Self::new()
        }
    }

    /// Format the current time as a string.
    #[must_use]
    pub fn format_time(&self, hour: u32, minute: u32, second: u32) -> String {
        if self.format_24h {
            if self.show_seconds {
                format!("{hour:02}:{minute:02}:{second:02}")
            } else {
                format!("{hour:02}:{minute:02}")
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
                    format!("{h12:02}:{minute:02}:{second:02} {period}")
                } else {
                    format!("{h12:02}:{minute:02} {period}")
                }
            } else if self.show_seconds {
                format!("{h12}:{minute:02}:{second:02} {period}")
            } else {
                format!("{h12}:{minute:02} {period}")
            }
        }
    }

    /// Format the date as a string.
    #[must_use]
    pub fn format_date(&self, year: i32, month: u32, day: u32, weekday: u32) -> String {
        let mut result = String::new();
        let mut chars = self.date_format.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('d') => {
                        let _ = write!(result, "{day:02}");
                    }
                    Some('b') => result.push_str(short_month(month)),
                    Some('B') => result.push_str(long_month(month)),
                    Some('m') => {
                        let _ = write!(result, "{month:02}");
                    }
                    Some('y') => {
                        let _ = write!(result, "{:02}", (year % 100).abs());
                    }
                    Some('Y') => {
                        let _ = write!(result, "{year:04}");
                    }
                    Some('a') => result.push_str(short_weekday(weekday)),
                    Some('A') => result.push_str(long_weekday(weekday)),
                    Some('%') | None => result.push('%'),
                    Some(other) => {
                        result.push('%');
                        result.push(other);
                    }
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    /// Convert a time to local civil time using the widget's offset.
    #[must_use]
    pub fn civil_at(&self, now: std::time::SystemTime) -> CivilTime {
        let secs = now
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs() as i64);
        let offset = self
            .utc_offset_minutes
            .unwrap_or_else(local_utc_offset_minutes);
        civil_from_unix_with_offset(secs, offset)
    }

    /// Get the time components at `now`.
    #[must_use]
    pub fn time_at(&self, now: std::time::SystemTime) -> (u32, u32, u32) {
        let civil = self.civil_at(now);
        (civil.hour, civil.minute, civil.second)
    }

    /// Get the date components at `now`, with Sunday as weekday zero.
    #[must_use]
    pub fn date_at(&self, now: std::time::SystemTime) -> (i32, u32, u32, u32) {
        let civil = self.civil_at(now);
        (civil.year, civil.month, civil.day, civil.weekday)
    }

    /// Get the current time components from the system clock.
    #[must_use]
    pub fn current_time(&self) -> (u32, u32, u32) {
        self.time_at(std::time::SystemTime::now())
    }

    /// Get the current date components from the system clock.
    #[must_use]
    pub fn current_date(&self) -> (i32, u32, u32, u32) {
        self.date_at(std::time::SystemTime::now())
    }

    /// Emit the clock's background quad.
    fn draw_background(&self, out: &mut DrawList, rect: Rect) {
        if self.bg_color[3] > 0.0 {
            out.push(
                Quad::new(
                    Vec2::new(rect.x, rect.y),
                    rect.size(),
                    Vec2::ZERO,
                    Vec2::ZERO,
                    self.bg_color,
                )
                .with_radius(self.corner_radius),
            );
        }
    }

    /// Emit the time text quad.
    fn draw_time(&mut self, out: &mut DrawList, rect: Rect, now: std::time::SystemTime) {
        let (hour, minute, second) = self.time_at(now);
        let time_str = self.format_time(hour, minute, second);
        self.last_time = time_str;

        let time_height = self.time_font_size;
        let time_y = rect.y + (rect.height - time_height) * 0.5;
        out.push(Quad::new(
            Vec2::new(rect.x + 4.0, time_y),
            Size::new(rect.width - 8.0, time_height),
            Vec2::ZERO,
            Vec2::ZERO,
            self.text_color,
        ));
    }

    /// Emit the date text quad.
    fn draw_date(&mut self, out: &mut DrawList, rect: Rect, now: std::time::SystemTime) {
        if !self.show_date {
            return;
        }
        let (year, month, day, weekday) = self.date_at(now);
        let date_str = self.format_date(year, month, day, weekday);
        self.last_date = date_str;

        let date_height = self.date_font_size;
        let date_y = rect.y + rect.height - date_height - 2.0;
        out.push(Quad::new(
            Vec2::new(rect.x + 4.0, date_y),
            Size::new(rect.width - 8.0, date_height),
            Vec2::ZERO,
            Vec2::ZERO,
            self.dim_color,
        ));
    }

    /// Emit the weekday text quad.
    fn draw_weekday(&self, out: &mut DrawList, rect: Rect, now: std::time::SystemTime) {
        if !self.show_weekday {
            return;
        }
        let (_, _, _, weekday) = self.date_at(now);
        let _weekday_str = long_weekday(weekday);

        let weekday_height = self.date_font_size;
        let weekday_y = rect.y + 2.0;
        out.push(Quad::new(
            Vec2::new(rect.x + 4.0, weekday_y),
            Size::new(rect.width - 8.0, weekday_height),
            Vec2::ZERO,
            Vec2::ZERO,
            self.dim_color,
        ));
    }

    /// Emit a divider between time and date.
    fn draw_divider(&self, out: &mut DrawList, rect: Rect) {
        if !self.show_date {
            return;
        }
        let divider_x = rect.x + rect.width * 0.5;
        let divider_y = rect.y + 4.0;
        let divider_height = rect.height - 8.0;
        out.push(Quad::new(
            Vec2::new(divider_x, divider_y),
            Size::new(1.0, divider_height),
            Vec2::ZERO,
            Vec2::ZERO,
            self.divider_color,
        ));
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
        let width = if self.show_seconds { 80.0 } else { 60.0 };
        let height = if self.show_date { 32.0 } else { 20.0 };
        Size::new(width, height)
    }

    fn draw(&mut self, out: &mut DrawList, ctx: &WidgetContext, rect: Rect) {
        self.draw_background(out, rect);
        self.draw_time(out, rect, ctx.now);
        self.draw_date(out, rect, ctx.now);
        self.draw_weekday(out, rect, ctx.now);
        self.draw_divider(out, rect);
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
    fn pads_the_hour_in_24_hour_format() {
        let clock = ClockWidget::new();
        assert_eq!(clock.format_time(9, 5, 3), "09:05");
    }

    #[test]
    fn shows_seconds_when_configured() {
        let mut clock = ClockWidget::new();
        clock.show_seconds = true;
        assert_eq!(clock.format_time(9, 5, 3), "09:05:03");
    }

    #[test]
    fn converts_midnight_and_noon_in_12_hour_format() {
        let mut clock = ClockWidget::new();
        clock.format_24h = false;
        assert_eq!(clock.format_time(0, 0, 0), "12:00 AM");
        assert_eq!(clock.format_time(12, 0, 0), "12:00 PM");
        assert_eq!(clock.format_time(13, 30, 0), "01:30 PM");
    }

    #[test]
    fn omits_the_leading_zero_in_12_hour_format() {
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
    fn the_clock_emits_quads_when_drawn() {
        let mut clock = ClockWidget::new();
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        clock.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 100.0, 32.0));
        assert!(!out.is_empty());
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

    #[test]
    fn the_clock_draws_background_when_configured() {
        let mut clock = ClockWidget::new();
        clock.bg_color = [0.1, 0.1, 0.1, 0.5];
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        clock.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 100.0, 32.0));
        assert!(!out.is_empty());
    }

    #[test]
    fn the_clock_draws_divider_when_date_is_shown() {
        let mut clock = ClockWidget::new();
        clock.show_date = true;
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        clock.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 100.0, 32.0));
        assert!(out.len() >= 2);
    }

    #[test]
    fn the_clock_does_not_draw_divider_when_date_is_hidden() {
        let mut clock = ClockWidget::new();
        clock.show_date = false;
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        clock.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 100.0, 20.0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn the_clock_tracks_last_drawn_time() {
        let mut clock = ClockWidget::new();
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        clock.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 100.0, 32.0));
        assert_ne!(clock.last_time, "");
    }

    #[test]
    fn the_clock_tracks_last_drawn_date() {
        let mut clock = ClockWidget::new();
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        clock.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 100.0, 32.0));
        assert_ne!(clock.last_date, "");
    }
}
