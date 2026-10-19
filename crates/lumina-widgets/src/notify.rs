//! The notification widget: a transient card with title, body, and action.
//!
//! # Queue
//!
//! Notifications are kept in a small bounded queue, newest first. Pushing when
//! the queue is full drops the oldest entry, so a broken producer cannot grow
//! memory without bound. [`NotificationWidget::tick`] removes entries whose
//! time has run out; the orchestrator calls it once per frame.
//!
//! # Input
//!
//! A click anywhere dismisses the card. When the card carries an action label,
//! a click inside the action pill records the action id and dismisses the card,
//! which the orchestrator can forward to the IPC layer. Body clicks record a
//! separate dismissal id so both outcomes are observable.

use std::collections::VecDeque;

use lumina_core::geometry::{Rect, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};

use crate::paint::{self, ACCENT, DIM, TEXT, TRACK};
use crate::widget::{DrawList, Widget, WidgetContext, WidgetInput};

/// The largest number of queued notifications.
pub const MAX_QUEUE: usize = 8;

/// One notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IslandNotification {
    /// Stable identifier, used to correlate action callbacks.
    pub id: u64,
    /// Headline.
    pub title: String,
    /// Body text.
    pub body: String,
    /// Optional icon name.
    pub icon: Option<String>,
    /// Accent color for the leading strip.
    pub accent: [u8; 4],
    /// Optional action label.
    pub action: Option<String>,
    /// Monotonic time the notification was created.
    pub created_at_ms: u64,
    /// Total display duration.
    pub duration_ms: u64,
}

impl IslandNotification {
    /// Build a notification with the standard accent and no action.
    #[must_use]
    pub fn new(
        id: u64,
        title: impl Into<String>,
        body: impl Into<String>,
        now_ms: u64,
        duration_ms: u64,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            body: body.into(),
            icon: None,
            accent: [110, 199, 255, 255],
            action: None,
            created_at_ms: now_ms,
            duration_ms: duration_ms.max(1),
        }
    }

    /// Add an icon name.
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Add an action label.
    #[must_use]
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Add an accent color.
    #[must_use]
    pub fn with_accent(mut self, accent: [u8; 4]) -> Self {
        self.accent = accent;
        self
    }

    /// Milliseconds left at `now_ms`.
    #[must_use]
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        self.created_at_ms
            .saturating_add(self.duration_ms)
            .saturating_sub(now_ms)
    }

    /// Completed fraction at `now_ms`, from zero to one.
    #[must_use]
    pub fn progress(&self, now_ms: u64) -> f32 {
        if self.duration_ms == 0 {
            return 1.0;
        }
        let elapsed = now_ms.saturating_sub(self.created_at_ms);
        (elapsed as f32 / self.duration_ms as f32).clamp(0.0, 1.0)
    }

    /// True once the display duration has elapsed.
    #[must_use]
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.created_at_ms.saturating_add(self.duration_ms)
    }

    fn accent_color(&self) -> [f32; 4] {
        [
            f32::from(self.accent[0]) / 255.0,
            f32::from(self.accent[1]) / 255.0,
            f32::from(self.accent[2]) / 255.0,
            f32::from(self.accent[3]) / 255.0,
        ]
    }
}

/// The notification widget.
#[derive(Debug, Clone, Default)]
pub struct NotificationWidget {
    queue: VecDeque<IslandNotification>,
    next_id: u64,
    activated: Option<u64>,
    dismissed: Option<u64>,
}

impl NotificationWidget {
    /// Construct an empty widget.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a notification and return its id.
    ///
    /// The newest notification is shown first.
    pub fn push(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        duration_ms: u64,
        now_ms: u64,
    ) -> u64 {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        self.push_notification(IslandNotification::new(
            id,
            title,
            body,
            now_ms,
            duration_ms,
        ));
        id
    }

    /// Add a fully built notification.
    pub fn push_notification(&mut self, notification: IslandNotification) {
        while self.queue.len() >= MAX_QUEUE {
            self.queue.pop_back();
        }
        self.next_id = self.next_id.max(notification.id);
        self.queue.push_front(notification);
    }

    /// Remove expired notifications and return how many were dropped.
    pub fn tick(&mut self, now_ms: u64) -> usize {
        let before = self.queue.len();
        self.queue.retain(|n| !n.is_expired(now_ms));
        before - self.queue.len()
    }

    /// The notification currently on screen.
    #[must_use]
    pub fn current(&self) -> Option<&IslandNotification> {
        self.queue.front()
    }

    /// Milliseconds left on the current notification.
    #[must_use]
    pub fn remaining_ms(&self, now_ms: u64) -> Option<u64> {
        self.current().map(|n| n.remaining_ms(now_ms))
    }

    /// The number of queued notifications.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// True when nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Remove and return the current notification.
    pub fn dismiss_current(&mut self) -> Option<IslandNotification> {
        let current = self.queue.pop_front()?;
        self.dismissed = Some(current.id);
        Some(current)
    }

    /// Take the id of the last dismissed notification, if any.
    pub fn take_dismissed(&mut self) -> Option<u64> {
        self.dismissed.take()
    }

    /// Take the id of the notification whose action was just activated.
    pub fn take_action(&mut self) -> Option<u64> {
        self.activated.take()
    }

    fn action_rect(rect: Rect) -> Rect {
        Rect::new(
            rect.right() - 54.0,
            rect.y + rect.height * 0.5 - 9.0,
            46.0,
            18.0,
        )
    }

    fn draw_card(&self, out: &mut DrawList, rect: Rect, notification: &IslandNotification, now_ms: u64) {
        paint::block(out, rect, [0.09, 0.1, 0.12, 0.96], 10.0);
        paint::block(
            out,
            Rect::new(rect.x, rect.y + 8.0, 3.0, rect.height - 16.0),
            notification.accent_color(),
            1.5,
        );

        let icon = Rect::new(rect.x + 10.0, rect.y + 8.0, 16.0, 16.0);
        paint::block(out, icon, ACCENT, 5.0);

        let text_x = rect.x + 34.0;
        let text_width = (rect.width - 44.0 - if notification.action.is_some() { 56.0 } else { 0.0 })
            .max(1.0);
        paint::label(out, Rect::new(text_x, rect.y + 6.0, text_width, 10.0), TEXT);
        paint::label(
            out,
            Rect::new(text_x, rect.y + 20.0, text_width, 8.0),
            DIM,
        );

        if notification.action.is_some() {
            let action = Self::action_rect(rect);
            paint::block(out, action, [1.0, 1.0, 1.0, 0.14], 6.0);
            paint::label(
                out,
                Rect::new(action.x + 8.0, action.y + 6.0, action.width - 16.0, 6.0),
                TEXT,
            );
        }

        let progress = notification.progress(now_ms);
        if progress < 1.0 {
            paint::block(
                out,
                Rect::new(rect.x + 10.0, rect.bottom() - 4.0, rect.width - 20.0, 2.0),
                TRACK,
                1.0,
            );
            paint::block(
                out,
                Rect::new(
                    rect.x + 10.0,
                    rect.bottom() - 4.0,
                    ((rect.width - 20.0) * (1.0 - progress)).max(1.0),
                    2.0,
                ),
                notification.accent_color(),
                1.0,
            );
        }
    }
}

impl Widget for NotificationWidget {
    fn id(&self) -> &'static str {
        "notify"
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        Size::new(300.0, 66.0)
    }

    fn draw(&mut self, out: &mut DrawList, ctx: &WidgetContext, rect: Rect) {
        let Some(notification) = self.queue.front() else {
            return;
        };
        let now_ms = ctx
            .now
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        self.draw_card(out, rect, notification, now_ms);
    }

    fn on_input(&mut self, ev: &WidgetInput, _ctx: &WidgetContext, rect: Rect) -> bool {
        let WidgetInput::Click(position) = ev else {
            return false;
        };
        let Some(current) = self.queue.front() else {
            return false;
        };
        let action_clicked =
            current.action.is_some() && Self::action_rect(rect).contains(*position);
        let id = current.id;
        if action_clicked {
            self.activated = Some(id);
        }
        self.queue.pop_front();
        self.dismissed = Some(id);
        true
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        let current = self.queue.front()?;
        let now_ms = now_ms();
        Some(StateRequest::until(
            IslandState::Notify,
            now_ms.saturating_add(current.remaining_ms(now_ms)),
        ))
    }

    fn priority(&self) -> u8 {
        IslandState::Notify.priority()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::new(0.0, 0.0, 300.0, 66.0)
    }

    fn ctx() -> WidgetContext {
        WidgetContext::new(IslandState::Notify, Size::new(320.0, 80.0))
    }

    #[test]
    fn the_newest_notification_is_shown_first() {
        let mut widget = NotificationWidget::new();
        widget.push("first", "one", 1_000, 0);
        widget.push("second", "two", 1_000, 0);
        assert_eq!(widget.current().expect("queued").title, "second");
    }

    #[test]
    fn the_queue_is_bounded() {
        let mut widget = NotificationWidget::new();
        for i in 0..(MAX_QUEUE + 4) {
            widget.push(format!("n{i}"), "", 1_000, 0);
        }
        assert_eq!(widget.len(), MAX_QUEUE);
        assert_eq!(
            widget.current().expect("queued").title,
            format!("n{}", MAX_QUEUE + 3)
        );
    }

    #[test]
    fn tick_removes_expired_notifications() {
        let mut widget = NotificationWidget::new();
        widget.push("short", "", 500, 0);
        assert_eq!(widget.tick(499), 0);
        assert_eq!(widget.tick(500), 1);
        assert!(widget.is_empty());
    }

    #[test]
    fn reminders_after_the_current_one_survive() {
        let mut widget = NotificationWidget::new();
        widget.push("first", "", 500, 0);
        widget.push("second", "", 10_000, 0);
        widget.tick(1_000);
        assert_eq!(widget.current().expect("queued").title, "second");
    }

    #[test]
    fn a_notification_reports_its_remaining_time() {
        let notification = IslandNotification::new(1, "t", "b", 1_000, 4_000);
        assert_eq!(notification.remaining_ms(2_000), 3_000);
        assert_eq!(notification.progress(2_000), 0.25);
        assert!(!notification.is_expired(4_999));
        assert!(notification.is_expired(5_000));
    }

    #[test]
    fn clicking_dismisses_and_records_the_id() {
        let mut widget = NotificationWidget::new();
        let id = widget.push("title", "body", 5_000, 0);
        let consumed = widget.on_input(
            &WidgetInput::Click(Vec2::new(40.0, 30.0)),
            &ctx(),
            rect(),
        );
        assert!(consumed);
        assert_eq!(widget.take_dismissed(), Some(id));
        assert!(widget.is_empty());
    }

    #[test]
    fn clicking_the_action_records_the_action_id() {
        let mut widget = NotificationWidget::new();
        widget.push_notification(
            IslandNotification::new(7, "title", "body", 0, 5_000).with_action("Open"),
        );
        widget.on_input(
            &WidgetInput::Click(Vec2::new(275.0, 33.0)),
            &ctx(),
            rect(),
        );
        assert_eq!(widget.take_action(), Some(7));
    }

    #[test]
    fn a_card_with_no_action_does_not_record_an_action() {
        let mut widget = NotificationWidget::new();
        widget.push("title", "body", 5_000, 0);
        widget.on_input(
            &WidgetInput::Click(Vec2::new(275.0, 33.0)),
            &ctx(),
            rect(),
        );
        assert_eq!(widget.take_action(), None);
    }

    #[test]
    fn a_queued_notification_requests_the_notify_state() {
        let mut widget = NotificationWidget::new();
        widget.push("title", "body", 5_000, 0);
        let request = widget.request(&ctx()).expect("notification is live");
        assert_eq!(request.state, IslandState::Notify);
    }

    #[test]
    fn an_empty_queue_requests_nothing() {
        assert!(NotificationWidget::new().request(&ctx()).is_none());
    }

    #[test]
    fn drawing_emits_the_card_and_a_progress_track() {
        let mut widget = NotificationWidget::new();
        widget.push_notification(
            IslandNotification::new(1, "title", "body", 0, 5_000).with_action("Open"),
        );
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, rect());
        assert!(out.len() >= 8);
    }

    #[test]
    fn accent_colors_are_converted_from_bytes() {
        let notification = IslandNotification::new(1, "t", "b", 0, 1_000)
            .with_accent([255, 128, 0, 255]);
        assert_eq!(notification.accent_color(), [1.0, 128.0 / 255.0, 0.0, 1.0]);
    }
}

