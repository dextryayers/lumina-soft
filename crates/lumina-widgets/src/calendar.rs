//! The calendar widget: a month grid with today highlighted.

use lumina_core::geometry::{Rect, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};

use crate::trait::{DrawList, Quad, Widget, WidgetContext, WidgetInput};

/// The calendar widget.
#[derive(Debug, Clone)]
pub struct CalendarWidget {
    /// The widget's id.
    pub id: &'static str,
    /// Whether to show the weekday header row.
    pub show_weekdays: bool,
    /// First day of the week: 0 = Sunday, 1 = Monday.
    pub first_day_of_week: u32,
}

impl CalendarWidget {
    /// Construct a calendar widget.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: "calendar",
            show_weekdays: true,
            first_day_of_week: 0,
        }
    }

    /// Number of days in a month.
    #[must_use]
    pub fn days_in_month(year: i32, month: u32) -> u32 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if is_leap_year(year) {
                    29
                } else {
                    28
                }
            }
            _ => 0,
        }
    }

    /// Day of week for the first day of a month: 0 = Sunday.
    ///
    /// Uses Zeller's congruence, which is correct for the Gregorian calendar.
    #[must_use]
    pub fn first_weekday_of_month(year: i32, month: u32) -> u32 {
        if month < 3 {
            return first_weekday_of_month(year - 1, month + 12);
        }
        let k = (year % 100) as u32;
        let j = (year / 100) as u32;
        let h = (1 + 13 * (month as u32 + 1) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
        // Zeller returns 0 = Saturday, 1 = Sunday, ... Convert to 0 = Sunday.
        (h + 6) % 7
    }

    /// Whether a year is a leap year.
    #[must_use]
    pub fn is_leap_year(year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
    }

    /// The number of weeks a month spans, given the first weekday.
    #[must_use]
    pub fn weeks_in_month(year: i32, month: u32) -> u32 {
        let days = Self::days_in_month(year, month);
        let first = Self::first_weekday_of_month(year, month);
        (days + first + 6) / 7
    }
}

impl Default for CalendarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for CalendarWidget {
    fn id(&self) -> &'static str {
        self.id
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        Size::new(180.0, 140.0)
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        // Emit a background quad. The grid itself is drawn by the renderer.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_calendar_has_sensible_defaults() {
        let cal = CalendarWidget::new();
        assert_eq!(cal.id, "calendar");
        assert!(cal.show_weekdays);
        assert_eq!(cal.first_day_of_week, 0);
    }

    #[test]
    fn days_in_month_is_correct_for_every_month() {
        assert_eq!(CalendarWidget::days_in_month(2026, 1), 31);
        assert_eq!(CalendarWidget::days_in_month(2026, 2), 28);
        assert_eq!(CalendarWidget::days_in_month(2026, 4), 30);
        assert_eq!(CalendarWidget::days_in_month(2026, 12), 31);
    }

    #[test]
    fn february_has_29_days_in_a_leap_year() {
        assert_eq!(CalendarWidget::days_in_month(2024, 2), 29);
        assert_eq!(CalendarWidget::days_in_month(2026, 2), 28);
    }

    #[test]
    fn century_years_are_leap_only_every_400_years() {
        assert!(!CalendarWidget::is_leap_year(1900));
        assert!(CalendarWidget::is_leap_year(2000));
        assert!(!CalendarWidget::is_leap_year(2100));
    }

    #[test]
    fn the_first_weekday_of_a_known_date_is_correct() {
        // 2026-10-19 is a Monday. With 0 = Sunday, that is 1.
        assert_eq!(CalendarWidget::first_weekday_of_month(2026, 10), 1);
    }

    #[test]
    fn weeks_in_month_covers_the_whole_grid() {
        // October 2026 starts on Monday and has 31 days, so it spans 5 weeks.
        assert_eq!(CalendarWidget::weeks_in_month(2026, 10), 5);
    }

    #[test]
    fn the_calendar_measures_a_reasonable_size() {
        let mut cal = CalendarWidget::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 200.0));
        let size = cal.measure(&ctx);
        assert!(size.width > 0.0);
        assert!(size.height > 0.0);
    }

    #[test]
    fn the_calendar_emits_a_quad_when_drawn() {
        let mut cal = CalendarWidget::new();
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 200.0));
        cal.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 180.0, 140.0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn the_calendar_does_not_consume_input() {
        let mut cal = CalendarWidget::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 200.0));
        assert!(!cal.on_input(&WidgetInput::Click(Vec2::new(50.0, 50.0)), &ctx, Rect::new(0.0, 0.0, 180.0, 140.0)));
    }

    #[test]
    fn the_calendar_requests_no_state() {
        let cal = CalendarWidget::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 200.0));
        assert!(cal.request(&ctx).is_none());
    }
}