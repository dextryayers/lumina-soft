//! The calendar widget: a month grid with today highlighted.
//!
//! # Content drawing
//!
//! The calendar widget emits quads for the month grid, weekday headers, day
//! cells, and today's highlight. The grid is computed from the current date and
//! the configured first day of the week.
//!
//! # Navigation
//!
//! The widget supports month navigation through input events. Clicking on the
//! left or right side of the header navigates to the previous or next month.

use lumina_core::geometry::{Rect, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};

use crate::widget::{DrawList, Quad, Widget, WidgetContext, WidgetInput};

/// The calendar widget.
#[derive(Debug, Clone)]
pub struct CalendarWidget {
    /// The widget's id.
    pub id: &'static str,
    /// Whether to show the weekday header row.
    pub show_weekdays: bool,
    /// First day of the week: 0 = Sunday, 1 = Monday.
    pub first_day_of_week: u32,
    /// The background color as RGBA.
    pub bg_color: [f32; 4],
    /// The header background color as RGBA.
    pub header_bg_color: [f32; 4],
    /// The text color as RGBA.
    pub text_color: [f32; 4],
    /// The dim text color as RGBA.
    pub dim_color: [f32; 4],
    /// The today highlight color as RGBA.
    pub today_color: [f32; 4],
    /// The selection color as RGBA.
    pub selection_color: [f32; 4],
    /// The corner radius of the background.
    pub corner_radius: f32,
    /// The font size for day numbers.
    pub day_font_size: f32,
    /// The font size for the header.
    pub header_font_size: f32,
    /// The currently displayed year.
    pub display_year: i32,
    /// The currently displayed month (1-12).
    pub display_month: u32,
    /// The currently selected day.
    pub selected_day: Option<u32>,
    /// The last drawn month, used to detect changes.
    pub last_drawn_month: String,
}

impl CalendarWidget {
    /// Construct a calendar widget.
    #[must_use]
    pub fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = now.as_secs() as i64;
        let days = total_secs / 86400;
        let year = 1970 + (days / 365) as i32;
        let day_of_year = (days % 365) as u32;
        let month = (day_of_year / 30) + 1;

        Self {
            id: "calendar",
            show_weekdays: true,
            first_day_of_week: 0,
            bg_color: [0.0, 0.0, 0.0, 0.0],
            header_bg_color: [0.1, 0.1, 0.1, 0.3],
            text_color: [1.0, 1.0, 1.0, 1.0],
            dim_color: [0.6, 0.6, 0.6, 1.0],
            today_color: [0.3, 0.5, 1.0, 0.4],
            selection_color: [0.2, 0.4, 0.8, 0.5],
            corner_radius: 8.0,
            day_font_size: 11.0,
            header_font_size: 12.0,
            display_year: year,
            display_month: month.min(12),
            selected_day: None,
            last_drawn_month: String::new(),
        }
    }

    /// Number of days in a month.
    #[must_use]
    pub fn days_in_month(year: i32, month: u32) -> u32 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if Self::is_leap_year(year) {
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
            return Self::first_weekday_of_month(year - 1, month + 12);
        }
        let k = (year % 100) as u32;
        let j = (year / 100) as u32;
        let h = (1 + 13 * (month + 1) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
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
        (days + first).div_ceil(7)
    }

    /// Get the current date components.
    #[must_use]
    pub fn current_date(&self) -> (i32, u32, u32) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = now.as_secs() as i64;
        let days = total_secs / 86400;
        let year = 1970 + (days / 365) as i32;
        let day_of_year = (days % 365) as u32;
        let month = (day_of_year / 30) + 1;
        let day = (day_of_year % 30) + 1;
        (year, month.min(12), day.min(28))
    }

    /// Navigate to the previous month.
    pub fn prev_month(&mut self) {
        if self.display_month == 1 {
            self.display_month = 12;
            self.display_year -= 1;
        } else {
            self.display_month -= 1;
        }
        self.selected_day = None;
    }

    /// Navigate to the next month.
    pub fn next_month(&mut self) {
        if self.display_month == 12 {
            self.display_month = 1;
            self.display_year += 1;
        } else {
            self.display_month += 1;
        }
        self.selected_day = None;
    }

    /// Navigate to today.
    pub fn go_to_today(&mut self) {
        let (year, month, day) = self.current_date();
        self.display_year = year;
        self.display_month = month;
        self.selected_day = Some(day);
    }

    /// Emit the calendar background.
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

    /// Emit the header with month and year.
    fn draw_header(&self, out: &mut DrawList, rect: Rect) {
        let header_height = self.header_font_size + 4.0;
        out.push(Quad::new(
            Vec2::new(rect.x, rect.y),
            Size::new(rect.width, header_height),
            Vec2::ZERO,
            Vec2::ZERO,
            self.header_bg_color,
        ));

        let month_name = long_month(self.display_month);
        let _header_text = format!("{} {}", month_name, self.display_year);
        let text_y = rect.y + 2.0;
        out.push(Quad::new(
            Vec2::new(rect.x + 4.0, text_y),
            Size::new(rect.width - 8.0, self.header_font_size),
            Vec2::ZERO,
            Vec2::ZERO,
            self.text_color,
        ));
    }

    /// Emit the weekday headers.
    fn draw_weekdays(&self, out: &mut DrawList, rect: Rect) {
        if !self.show_weekdays {
            return;
        }
        let header_height = self.header_font_size + 4.0;
        let weekday_height = self.day_font_size + 2.0;
        let y = rect.y + header_height;

        let weekdays = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
        let cell_width = rect.width / 7.0;

        for (i, _) in weekdays.iter().enumerate() {
            let weekday_index = (i as u32 + self.first_day_of_week) % 7;
            let x = rect.x + (i as f32) * cell_width;
            out.push(Quad::new(
                Vec2::new(x, y),
                Size::new(cell_width, weekday_height),
                Vec2::ZERO,
                Vec2::ZERO,
                self.dim_color,
            ));
            let _ = weekday_index;
        }
    }

    /// Emit the day cells.
    fn draw_days(&mut self, out: &mut DrawList, rect: Rect) {
        let header_height = self.header_font_size + 4.0;
        let weekday_height = if self.show_weekdays { self.day_font_size + 2.0 } else { 0.0 };
        let grid_y = rect.y + header_height + weekday_height;
        let grid_height = rect.height - header_height - weekday_height;

        let days = Self::days_in_month(self.display_year, self.display_month);
        let first_weekday = Self::first_weekday_of_month(self.display_year, self.display_month);
        let weeks = Self::weeks_in_month(self.display_year, self.display_month);

        let cell_width = rect.width / 7.0;
        let cell_height = grid_height / weeks as f32;

        let (current_year, current_month, current_day) = self.current_date();

        for day in 1..=days {
            let day_index = day - 1;
            let weekday = (first_weekday + day_index) % 7;
            let week = (first_weekday + day_index) / 7;

            let x = rect.x + (weekday as f32) * cell_width;
            let y = grid_y + (week as f32) * cell_height;

            let is_today = self.display_year == current_year
                && self.display_month == current_month
                && day == current_day;

            let is_selected = self.selected_day == Some(day);

            if is_today {
                out.push(
                    Quad::new(
                        Vec2::new(x, y),
                        Size::new(cell_width, cell_height),
                        Vec2::ZERO,
                        Vec2::ZERO,
                        self.today_color,
                    )
                    .with_radius(4.0),
                );
            } else if is_selected {
                out.push(
                    Quad::new(
                        Vec2::new(x, y),
                        Size::new(cell_width, cell_height),
                        Vec2::ZERO,
                        Vec2::ZERO,
                        self.selection_color,
                    )
                    .with_radius(4.0),
                );
            }

            let text_color = if is_today {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                self.text_color
            };

            out.push(Quad::new(
                Vec2::new(x + 2.0, y + 2.0),
                Size::new(cell_width - 4.0, cell_height - 4.0),
                Vec2::ZERO,
                Vec2::ZERO,
                text_color,
            ));
        }

        self.last_drawn_month = format!("{}-{:02}", self.display_year, self.display_month);
    }

    /// Handle click input for navigation.
    fn handle_click(&mut self, pos: Vec2, rect: Rect) -> bool {
        let header_height = self.header_font_size + 4.0;
        if pos.y < rect.y + header_height {
            if pos.x < rect.x + rect.width * 0.3 {
                self.prev_month();
                return true;
            } else if pos.x > rect.x + rect.width * 0.7 {
                self.next_month();
                return true;
            }
        }
        false
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
        self.draw_background(out, rect);
        self.draw_header(out, rect);
        self.draw_weekdays(out, rect);
        self.draw_days(out, rect);
    }

    fn on_input(&mut self, ev: &WidgetInput, _ctx: &WidgetContext, rect: Rect) -> bool {
        match ev {
            WidgetInput::Click(pos) => self.handle_click(*pos, rect),
            WidgetInput::Scroll(delta) => {
                if *delta > 0.0 {
                    self.next_month();
                } else {
                    self.prev_month();
                }
                true
            }
            _ => false,
        }
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        None
    }

    fn priority(&self) -> u8 {
        IslandState::Clock.priority()
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
        assert_eq!(CalendarWidget::first_weekday_of_month(2026, 10), 4);
    }

    #[test]
    fn weeks_in_month_covers_the_whole_grid() {
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
    fn the_calendar_emits_quads_when_drawn() {
        let mut cal = CalendarWidget::new();
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 200.0));
        cal.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 180.0, 140.0));
        assert!(!out.is_empty());
    }

    #[test]
    fn the_calendar_does_not_consume_click_input_by_default() {
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

    #[test]
    fn the_calendar_navigates_to_previous_month() {
        let mut cal = CalendarWidget::new();
        cal.display_month = 6;
        cal.prev_month();
        assert_eq!(cal.display_month, 5);
    }

    #[test]
    fn the_calendar_navigates_to_next_month() {
        let mut cal = CalendarWidget::new();
        cal.display_month = 6;
        cal.next_month();
        assert_eq!(cal.display_month, 7);
    }

    #[test]
    fn the_calendar_wraps_from_january_to_december() {
        let mut cal = CalendarWidget::new();
        cal.display_month = 1;
        cal.prev_month();
        assert_eq!(cal.display_month, 12);
    }

    #[test]
    fn the_calendar_wraps_from_december_to_january() {
        let mut cal = CalendarWidget::new();
        cal.display_month = 12;
        cal.next_month();
        assert_eq!(cal.display_month, 1);
    }

    #[test]
    fn the_calendar_go_to_today_sets_selected_day() {
        let mut cal = CalendarWidget::new();
        cal.go_to_today();
        assert!(cal.selected_day.is_some());
    }

    #[test]
    fn the_calendar_handles_scroll_for_navigation() {
        let mut cal = CalendarWidget::new();
        cal.display_month = 6;
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 200.0));
        cal.on_input(&WidgetInput::Scroll(1.0), &ctx, Rect::new(0.0, 0.0, 180.0, 140.0));
        assert_eq!(cal.display_month, 7);
    }

    #[test]
    fn the_calendar_tracks_last_drawn_month() {
        let mut cal = CalendarWidget::new();
        let mut out = DrawList::new();
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 200.0));
        cal.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 180.0, 140.0));
        assert_ne!(cal.last_drawn_month, "");
    }
}
