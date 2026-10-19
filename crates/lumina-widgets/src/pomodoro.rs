//! The pomodoro widget: focus and break phases with a dotted progress ring.
//!
//! # Timekeeping
//!
//! The widget owns a [`PomodoroTimer`] that advances by an explicit elapsed
//! duration. Nothing inside the timer reads the clock, so phase boundaries are
//! deterministic in tests and a delayed frame does not lose time: if a frame
//! stalls for longer than the remaining phase, the timer consumes the surplus
//! into the next phase.
//!
//! # Notifications
//!
//! When a phase changes and `notify_phases` is on, the widget requests the
//! notify state for a few seconds so the island announces the change. The
//! request is only issued while the announcement window is open, so the timer
//! itself never keeps the island pinned.

use std::time::Duration;

use lumina_config::schema::PomodoroConfig;
use lumina_core::geometry::{Rect, Size, Vec2};
use lumina_core::state::{IslandState, StateRequest};

use crate::paint::{self, ACCENT, DIM, GOOD, TRACK, WARN};
use crate::snapshot::format_countdown;
use crate::widget::{DrawList, Widget, WidgetContext, WidgetInput};

/// Length of a phase announcement.
const NOTIFY_HOLD_MS: u64 = 5_000;
/// Segments in the progress ring.
const RING_SEGMENTS: usize = 24;

/// One focus or break phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PomodoroPhase {
    /// Concentrated work.
    Focus,
    /// A short break between focus phases.
    ShortBreak,
    /// A long break after a configured number of focus phases.
    LongBreak,
}

impl PomodoroPhase {
    /// A short human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::ShortBreak => "Short break",
            Self::LongBreak => "Long break",
        }
    }

    /// True for either break phase.
    #[must_use]
    pub const fn is_break(self) -> bool {
        !matches!(self, Self::Focus)
    }
}

/// A phase transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PomodoroEvent {
    /// The phase that ended.
    pub completed: PomodoroPhase,
    /// The phase that began.
    pub started: PomodoroPhase,
}

/// The phase timer, independent of rendering and of the wall clock.
#[derive(Debug, Clone, PartialEq)]
pub struct PomodoroTimer {
    /// The current phase.
    pub phase: PomodoroPhase,
    /// Time left in the current phase.
    pub remaining: Duration,
    /// Whether the timer is counting down.
    pub running: bool,
    /// Focus phases completed since the last long break.
    pub completed_focus: u32,
    /// Focus phase length.
    pub focus: Duration,
    /// Short break length.
    pub short_break: Duration,
    /// Long break length.
    pub long_break: Duration,
    /// Focus phases before a long break.
    pub rounds_before_long: u32,
}

impl PomodoroTimer {
    /// Construct a timer from configuration.
    ///
    /// Every duration is clamped to at least one second and at most three
    /// hours; a zero minute phase would otherwise complete instantly forever.
    #[must_use]
    pub fn from_config(config: &PomodoroConfig) -> Self {
        let minutes = |value: u32| Duration::from_secs(u64::from(value.clamp(1, 180)) * 60);
        Self {
            phase: PomodoroPhase::Focus,
            remaining: minutes(config.focus_minutes),
            running: false,
            completed_focus: 0,
            focus: minutes(config.focus_minutes),
            short_break: minutes(config.short_break_minutes),
            long_break: minutes(config.long_break_minutes),
            rounds_before_long: config.rounds_before_long.clamp(1, 12),
        }
    }

    /// Construct a timer from explicit durations.
    #[must_use]
    pub fn new(
        focus: Duration,
        short_break: Duration,
        long_break: Duration,
        rounds_before_long: u32,
    ) -> Self {
        Self {
            phase: PomodoroPhase::Focus,
            remaining: focus,
            running: false,
            completed_focus: 0,
            focus,
            short_break,
            long_break,
            rounds_before_long: rounds_before_long.max(1),
        }
    }

    /// The full length of the current phase.
    #[must_use]
    pub fn phase_duration(&self) -> Duration {
        match self.phase {
            PomodoroPhase::Focus => self.focus,
            PomodoroPhase::ShortBreak => self.short_break,
            PomodoroPhase::LongBreak => self.long_break,
        }
    }

    /// Completed fraction of the current phase, from zero to one.
    #[must_use]
    pub fn progress(&self) -> f32 {
        let total = self.phase_duration().as_secs_f32();
        if total <= 0.0 {
            return 1.0;
        }
        let remaining = self.remaining.as_secs_f32();
        ((total - remaining) / total).clamp(0.0, 1.0)
    }

    /// Remaining seconds, rounded up so the display never shows zero while the
    /// timer is still running.
    #[must_use]
    pub fn remaining_secs(&self) -> u64 {
        let secs = self.remaining.as_secs();
        if self.remaining.subsec_nanos() > 0 {
            secs + 1
        } else {
            secs
        }
    }

    /// The remaining time as a clock face.
    #[must_use]
    pub fn remaining_text(&self) -> String {
        format_countdown(self.remaining_secs())
    }

    /// Start or pause.
    pub fn toggle(&mut self) {
        self.running = !self.running;
        if self.running && self.remaining.is_zero() {
            self.remaining = self.phase_duration();
        }
    }

    /// Restart the current phase without changing the phase.
    pub fn reset_phase(&mut self) {
        self.remaining = self.phase_duration();
    }

    /// Abandon the current phase and start the next one without counting it.
    ///
    /// Skipping a focus phase does not increment the completed counter, so a
    /// user cannot reach a long break by repeatedly skipping.
    pub fn skip(&mut self) -> PomodoroEvent {
        let completed = self.phase;
        let started = self.next_phase();
        self.phase = started;
        self.remaining = self.phase_duration();
        PomodoroEvent { completed, started }
    }

    /// The phase that follows the current one.
    fn next_phase(&self) -> PomodoroPhase {
        match self.phase {
            PomodoroPhase::Focus => {
                if self.completed_focus > 0
                    && self.completed_focus % self.rounds_before_long == 0
                {
                    PomodoroPhase::LongBreak
                } else {
                    PomodoroPhase::ShortBreak
                }
            }
            PomodoroPhase::ShortBreak | PomodoroPhase::LongBreak => PomodoroPhase::Focus,
        }
    }

    /// Advance the timer by `elapsed`, returning every phase transition.
    ///
    /// A paused timer does not advance. An elapsed duration longer than several
    /// phases is consumed into each phase in turn, with a safety bound so a
    /// pathological duration cannot spin forever.
    pub fn advance(&mut self, elapsed: Duration) -> Vec<PomodoroEvent> {
        let mut events = Vec::new();
        if !self.running || elapsed.is_zero() {
            return events;
        }
        let mut left = elapsed;
        for _ in 0..16 {
            if left < self.remaining || self.remaining.is_zero() {
                self.remaining = self.remaining.saturating_sub(left);
                break;
            }
            left = left.saturating_sub(self.remaining);
            let completed = self.phase;
            if completed == PomodoroPhase::Focus {
                self.completed_focus = self.completed_focus.saturating_add(1);
            }
            let started = self.next_phase();
            self.phase = started;
            self.remaining = self.phase_duration();
            events.push(PomodoroEvent {
                completed,
                started,
            });
        }
        events
    }
}

/// The pomodoro widget.
#[derive(Debug, Clone)]
pub struct PomodoroWidget {
    /// The timer.
    pub timer: PomodoroTimer,
    /// Whether phase changes are announced through the notify state.
    pub notify_phases: bool,
    /// Tick at which the current announcement ends.
    pub notify_until_ms: Option<u64>,
}

impl PomodoroWidget {
    /// Construct a widget from the pomodoro configuration.
    #[must_use]
    pub fn from_config(config: &PomodoroConfig) -> Self {
        Self {
            timer: PomodoroTimer::from_config(config),
            notify_phases: config.notify_phases,
            notify_until_ms: None,
        }
    }

    /// Construct a stopped twenty-five minute timer.
    #[must_use]
    pub fn new() -> Self {
        Self::from_config(&PomodoroConfig {
            enabled: true,
            ..PomodoroConfig::default()
        })
    }

    /// Advance the timer, opening an announcement window when a phase changed.
    pub fn tick(&mut self, elapsed: Duration, now_ms: u64) -> Vec<PomodoroEvent> {
        let events = self.timer.advance(elapsed);
        if !events.is_empty() && self.notify_phases {
            self.notify_until_ms = Some(now_ms.saturating_add(NOTIFY_HOLD_MS));
        }
        events
    }

    /// The current phase.
    #[must_use]
    pub fn phase(&self) -> PomodoroPhase {
        self.timer.phase
    }

    /// The phase label.
    #[must_use]
    pub fn phase_label(&self) -> &'static str {
        self.timer.phase.label()
    }

    /// The countdown text.
    #[must_use]
    pub fn remaining_text(&self) -> String {
        self.timer.remaining_text()
    }

    /// The ring color for the current phase.
    #[must_use]
    pub fn phase_color(&self) -> [f32; 4] {
        match self.timer.phase {
            PomodoroPhase::Focus => ACCENT,
            PomodoroPhase::ShortBreak => GOOD,
            PomodoroPhase::LongBreak => WARN,
        }
    }
}

impl Default for PomodoroWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for PomodoroWidget {
    fn id(&self) -> &'static str {
        "pomodoro"
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        Size::new(96.0, 30.0)
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        let ring_radius = (rect.height.min(28.0) * 0.5) - 3.0;
        let center = Vec2::new(rect.x + ring_radius + 3.0, rect.y + rect.height * 0.5);
        paint::dots_ring(
            out,
            center,
            ring_radius,
            RING_SEGMENTS,
            self.timer.progress(),
            self.phase_color(),
            TRACK,
        );

        let text_x = center.x + ring_radius + 8.0;
        let text_width = (rect.right() - text_x).max(1.0);
        paint::label(
            out,
            Rect::new(text_x, rect.y + 5.0, text_width * 0.7, 10.0),
            crate::paint::TEXT,
        );
        paint::label(
            out,
            Rect::new(text_x, rect.y + 18.0, text_width * 0.85, 6.0),
            DIM,
        );
        if self.timer.running {
            paint::block(
                out,
                Rect::new(rect.right() - 5.0, rect.y + 5.0, 3.0, 3.0),
                self.phase_color(),
                1.5,
            );
        }
    }

    fn on_input(&mut self, ev: &WidgetInput, _ctx: &WidgetContext, _rect: Rect) -> bool {
        match ev {
            WidgetInput::Click(_) => {
                self.timer.toggle();
                true
            }
            WidgetInput::RightClick(_) => {
                self.timer.reset_phase();
                true
            }
            _ => false,
        }
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        let until = self.notify_until_ms?;
        if self.notify_phases {
            Some(StateRequest::until(IslandState::Notify, until))
        } else {
            None
        }
    }

    fn priority(&self) -> u8 {
        IslandState::Notify.priority()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer() -> PomodoroTimer {
        PomodoroTimer::new(
            Duration::from_secs(25 * 60),
            Duration::from_secs(5 * 60),
            Duration::from_secs(15 * 60),
            4,
        )
    }

    fn ctx() -> WidgetContext {
        WidgetContext::new(IslandState::Notify, Size::new(200.0, 40.0))
    }

    #[test]
    fn a_new_timer_is_stopped_in_focus() {
        let timer = timer();
        assert_eq!(timer.phase, PomodoroPhase::Focus);
        assert!(!timer.running);
        assert_eq!(timer.remaining_text(), "25:00");
    }

    #[test]
    fn a_paused_timer_does_not_advance() {
        let mut timer = timer();
        assert!(timer.advance(Duration::from_secs(60)).is_empty());
        assert_eq!(timer.remaining_text(), "25:00");
    }

    #[test]
    fn focus_completion_starts_a_short_break() {
        let mut timer = timer();
        timer.toggle();
        let events = timer.advance(Duration::from_secs(25 * 60));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].completed, PomodoroPhase::Focus);
        assert_eq!(events[0].started, PomodoroPhase::ShortBreak);
        assert_eq!(timer.phase, PomodoroPhase::ShortBreak);
        assert_eq!(timer.remaining_secs(), 5 * 60);
        assert_eq!(timer.completed_focus, 1);
    }

    #[test]
    fn a_fourth_focus_completion_starts_a_long_break() {
        let mut timer = timer();
        timer.toggle();
        timer.advance(Duration::from_secs(25 * 60));
        timer.advance(Duration::from_secs(5 * 60));
        timer.advance(Duration::from_secs(25 * 60));
        timer.advance(Duration::from_secs(5 * 60));
        timer.advance(Duration::from_secs(25 * 60));
        timer.advance(Duration::from_secs(5 * 60));
        let events = timer.advance(Duration::from_secs(25 * 60));
        assert_eq!(events[0].started, PomodoroPhase::LongBreak);
        assert_eq!(timer.phase, PomodoroPhase::LongBreak);
        assert_eq!(timer.completed_focus, 4);
    }

    #[test]
    fn surplus_elapsed_time_carries_into_the_next_phase() {
        let mut timer = timer();
        timer.toggle();
        let events = timer.advance(Duration::from_secs(25 * 60 + 30));
        assert_eq!(events.len(), 1);
        assert_eq!(timer.remaining_secs(), 5 * 60 - 30);
    }

    #[test]
    fn skipping_does_not_count_a_focus_phase() {
        let mut timer = timer();
        let event = timer.skip();
        assert_eq!(event.started, PomodoroPhase::ShortBreak);
        assert_eq!(timer.completed_focus, 0);
    }

    #[test]
    fn progress_runs_from_zero_to_one() {
        let mut timer = timer();
        assert_eq!(timer.progress(), 0.0);
        timer.toggle();
        timer.advance(Duration::from_secs(25 * 60 / 2));
        assert!((timer.progress() - 0.5).abs() < 1e-3);
    }

    #[test]
    fn reset_restores_the_phase_duration() {
        let mut timer = timer();
        timer.toggle();
        timer.advance(Duration::from_secs(60));
        timer.reset_phase();
        assert_eq!(timer.remaining_text(), "25:00");
    }

    #[test]
    fn toggling_from_zero_restores_the_phase_duration() {
        let mut timer = timer();
        timer.remaining = Duration::ZERO;
        timer.toggle();
        assert_eq!(timer.remaining_text(), "25:00");
    }

    #[test]
    fn config_minutes_are_clamped() {
        let timer = PomodoroTimer::from_config(&PomodoroConfig {
            focus_minutes: 0,
            short_break_minutes: 0,
            long_break_minutes: 0,
            rounds_before_long: 0,
            ..PomodoroConfig::default()
        });
        assert_eq!(timer.focus, Duration::from_secs(60));
        assert_eq!(timer.rounds_before_long, 1);
        assert_eq!(timer.short_break, Duration::from_secs(60));
    }

    #[test]
    fn a_phase_change_opens_the_notify_window() {
        let mut widget = PomodoroWidget::new();
        widget.timer.toggle();
        widget.tick(Duration::from_secs(25 * 60), 1_000);
        let request = widget.request(&ctx()).expect("phase changes announce");
        assert_eq!(request.state, IslandState::Notify);
        assert_eq!(request.expires_at_ms, Some(6_000));
    }

    #[test]
    fn phase_notifications_can_be_disabled() {
        let mut widget = PomodoroWidget::new();
        widget.notify_phases = false;
        widget.timer.toggle();
        widget.tick(Duration::from_secs(25 * 60), 1_000);
        assert!(widget.request(&ctx()).is_none());
    }

    #[test]
    fn clicking_toggles_and_right_clicking_resets() {
        let mut widget = PomodoroWidget::new();
        let ctx = ctx();
        let rect = Rect::new(0.0, 0.0, 96.0, 30.0);
        assert!(widget.on_input(&WidgetInput::Click(Vec2::new(10.0, 10.0)), &ctx, rect));
        assert!(widget.timer.running);
        assert!(widget.on_input(
            &WidgetInput::RightClick(Vec2::new(10.0, 10.0)),
            &ctx,
            rect
        ));
        assert_eq!(widget.remaining_text(), "25:00");
    }

    #[test]
    fn drawing_emits_a_ring_and_labels() {
        let mut widget = PomodoroWidget::new();
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 96.0, 30.0));
        assert!(out.len() >= RING_SEGMENTS + 2);
    }

    #[test]
    fn phase_labels_and_colors_differ() {
        assert_ne!(PomodoroPhase::Focus.label(), PomodoroPhase::ShortBreak.label());
        assert!(PomodoroPhase::Focus.is_break() == false);
        assert!(PomodoroPhase::LongBreak.is_break());
        let mut widget = PomodoroWidget::new();
        let focus_color = widget.phase_color();
        widget.timer.skip();
        assert_ne!(widget.phase_color(), focus_color);
    }
}
