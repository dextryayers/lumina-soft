//! Monotonic clocks, fixed step accumulation, and frame pacing.
//!
//! # The one rule that makes the idle budget achievable
//!
//! The render loop must stop submitting frames when nothing is moving. A
//! process that submits 60 frames a second forever will sit at a few percent of
//! a CPU core no matter how cheap each frame is, and no amount of per frame
//! optimization fixes that. [`FramePacer`] plus the `is_at_rest` checks in
//! [`crate::spring::Spring`] are what let Lumina hit 0.5 percent idle.
//!
//! # Why fixed step
//!
//! Variable `dt` is the classic source of animation bugs: a 200 ms hitch makes
//! an exponential decay spring behave differently from a 16 ms frame, so the
//! same transition looks different depending on machine load.
//! [`FixedStepAccumulator`] splits elapsed time into whole steps plus a
//! remainder, so physics always advances in identical increments.

use std::time::{Duration, Instant};

/// Default target frame rate.
pub const DEFAULT_TARGET_FPS: u32 = 60;

/// Default simulation step for accumulators that want smooth but cheap logic.
pub const DEFAULT_SIM_STEP: Duration = Duration::from_millis(16);

/// A monotonic clock, in seconds since the process started.
///
/// Wraps [`Instant`], which is monotonic and therefore immune to the wall clock
/// being adjusted. A user changing the system time must never affect animation.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    origin: Instant,
}

impl Clock {
    /// Start a clock now.
    #[must_use]
    pub fn new() -> Self {
        Self { origin: Instant::now() }
    }

    /// Seconds elapsed since construction.
    #[must_use]
    pub fn elapsed_secs(&self) -> f32 {
        self.origin.elapsed().as_secs_f32()
    }

    /// Milliseconds elapsed since construction.
    #[must_use]
    pub fn elapsed_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }

    /// Duration elapsed since construction.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.origin.elapsed()
    }

    /// Reset to zero.
    pub fn reset(&mut self) {
        self.origin = Instant::now();
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

/// Splits elapsed wall clock time into fixed simulation steps plus a remainder.
#[derive(Debug, Clone)]
pub struct FixedStepAccumulator {
    step: Duration,
    accumulator: Duration,
    max_steps: u32,
}

impl FixedStepAccumulator {
    /// Construct with an explicit step size and a cap on steps per call.
    #[must_use]
    pub fn new(step: Duration, max_steps: u32) -> Self {
        Self { step: step.max(Duration::from_micros(100)), accumulator: Duration::ZERO, max_steps: max_steps.max(1) }
    }

    /// Construct with [`DEFAULT_SIM_STEP`].
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_SIM_STEP, 5)
    }

    /// Feed elapsed time in and get the number of whole steps to run.
    ///
    /// The remainder is retained for the next call, so no time is lost or
    /// double counted. When the cap is hit the leftover is discarded rather
    /// than carried, which prevents a death spiral where a slow frame causes a
    /// slower next frame.
    #[must_use]
    pub fn steps(&mut self, elapsed: Duration) -> u32 {
        self.accumulator = self.accumulator.saturating_add(elapsed);

        let step = self.step;
        let max = self.max_steps;
        let mut steps = 0u32;

        while self.accumulator >= step && steps < max {
            self.accumulator -= step;
            steps += 1;
        }

        if steps == max {
            self.accumulator = Duration::ZERO;
        }

        steps
    }

    /// Time held back for the next call.
    #[must_use]
    pub fn remainder(&self) -> Duration {
        self.accumulator
    }

    /// Discard the remainder.
    pub fn reset(&mut self) {
        self.accumulator = Duration::ZERO;
    }

    /// Step size in use.
    #[must_use]
    pub fn step(&self) -> Duration {
        self.step
    }

    /// Change the step size, clamping the accumulator to at most one step so a
    /// step change cannot cause a burst of catch up steps.
    pub fn set_step(&mut self, step: Duration) {
        self.step = step.max(Duration::from_micros(100));
        if self.accumulator > self.step {
            self.accumulator = Duration::ZERO;
        }
    }
}

impl Default for FixedStepAccumulator {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Decides when the render loop is allowed to submit a frame.
///
/// Three inputs decide this: a target frame rate, whether presentation is
/// vsynced, and whether there is anything new to draw. When nothing is new the
/// pacer reports `false` and the loop blocks instead of spinning.
#[derive(Debug, Clone)]
pub struct FramePacer {
    target_fps: u32,
    vsync: bool,
    min_interval: Duration,
    last_submit: Option<Instant>,
}

impl FramePacer {
    /// Construct a pacer.
    #[must_use]
    pub fn new(target_fps: u32, vsync: bool) -> Self {
        let target_fps = target_fps.clamp(1, 480);
        Self {
            target_fps,
            vsync,
            min_interval: Duration::from_secs_f64(1.0 / f64::from(target_fps)),
            last_submit: None,
        }
    }

    /// Target frame rate.
    #[must_use]
    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }

    /// Whether presentation is vsynced.
    #[must_use]
    pub fn vsync(&self) -> bool {
        self.vsync
    }

    /// Change the target frame rate, preserving the last submit time.
    pub fn set_target_fps(&mut self, target_fps: u32) {
        self.target_fps = target_fps.clamp(1, 480);
        self.min_interval = Duration::from_secs_f64(1.0 / f64::from(self.target_fps));
    }

    /// Minimum wall clock interval between submits at the current rate.
    #[must_use]
    pub fn min_interval(&self) -> Duration {
        self.min_interval
    }

    /// May a frame be submitted now?
    ///
    /// When vsync is on, the answer is always yes and the surface blocks for us.
    /// When vsync is off, the answer enforces the target rate so a fast state
    /// change cannot burn CPU submitting frames nobody can see.
    #[must_use]
    pub fn should_submit(&self, now: Instant, dirty: bool) -> bool {
        if !dirty {
            return false;
        }
        if self.vsync {
            return true;
        }
        match self.last_submit {
            Some(last) => now.duration_since(last) >= self.min_interval,
            None => true,
        }
    }

    /// Record that a frame was submitted.
    pub fn record_submit(&mut self, now: Instant) {
        self.last_submit = Some(now);
    }

    /// Time until the next permitted submit, or zero when one is already due.
    #[must_use]
    pub fn time_until_next(&self, now: Instant) -> Duration {
        match self.last_submit {
            Some(last) => self.min_interval.saturating_sub(now.duration_since(last)),
            None => Duration::ZERO,
        }
    }

    /// Forget the last submit, so the next frame is immediate. Used after a
    /// state change that must be visible on the very next present.
    pub fn reset(&mut self) {
        self.last_submit = None;
    }
}

impl Default for FramePacer {
    fn default() -> Self {
        Self::new(DEFAULT_TARGET_FPS, true)
    }
}

/// Exponential moving average, for smoothing noisy numbers such as frame time.
#[derive(Debug, Clone, Copy)]
pub struct Ema {
    value: f32,
    alpha: f32,
    initialized: bool,
}

impl Ema {
    /// Construct with a smoothing factor in `(0, 1]`.
    ///
    /// Lower alpha means more smoothing and more lag.
    #[must_use]
    pub fn new(alpha: f32) -> Self {
        Self { value: 0.0, alpha: alpha.clamp(0.001, 1.0), initialized: false }
    }

    /// Feed a sample.
    pub fn update(&mut self, sample: f32) {
        if !sample.is_finite() {
            return;
        }
        if self.initialized {
            self.value += self.alpha * (sample - self.value);
        } else {
            self.value = sample;
            self.initialized = true;
        }
    }

    /// Current smoothed value, or zero before the first sample.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// True once at least one sample has been folded in.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Clear back to the uninitialized state.
    pub fn reset(&mut self) {
        self.value = 0.0;
        self.initialized = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic_and_starts_near_zero() {
        let clock = Clock::new();
        assert!(clock.elapsed_secs() < 1.0);
        let first = clock.elapsed_ms();
        std::thread::sleep(Duration::from_millis(12));
        assert!(clock.elapsed_ms() > first);
    }

    #[test]
    fn clock_reset_returns_to_zero() {
        let mut clock = Clock::new();
        std::thread::sleep(Duration::from_millis(5));
        clock.reset();
        assert!(clock.elapsed_ms() < 50);
    }

    #[test]
    fn accumulator_returns_whole_steps_and_keeps_the_remainder() {
        let mut acc = FixedStepAccumulator::new(Duration::from_millis(10), 10);
        // 25 ms is two whole steps plus a 5 ms remainder.
        assert_eq!(acc.steps(Duration::from_millis(25)), 2);
        assert_eq!(acc.remainder(), Duration::from_millis(5));

        // The next 5 ms completes the third step, nothing is lost.
        assert_eq!(acc.steps(Duration::from_millis(5)), 1);
        assert_eq!(acc.remainder(), Duration::ZERO);
    }

    #[test]
    fn accumulator_conserves_time_across_many_small_inputs() {
        let mut acc = FixedStepAccumulator::new(Duration::from_millis(16), 100);
        let mut total_steps = 0u32;
        // Feed 1000 x 1 ms, which is exactly 1 second, at a 16 ms step.
        for _ in 0..1_000 {
            total_steps += acc.steps(Duration::from_millis(1));
        }
        // 1 second / 16 ms = 62.5 steps.
        assert!(
            (62..=63).contains(&total_steps),
            "expected about 62 steps, got {total_steps}"
        );
    }

    #[test]
    fn accumulator_drops_remainder_instead_of_death_spiralling() {
        let mut acc = FixedStepAccumulator::new(Duration::from_millis(10), 3);
        // A 10 second stall would be 1000 steps without the cap.
        assert_eq!(acc.steps(Duration::from_secs(10)), 3);
        assert_eq!(acc.remainder(), Duration::ZERO, "leftover must be dropped, not carried");

        // And the next frame is not stuck trying to catch up.
        assert_eq!(acc.steps(Duration::from_millis(10)), 1);
    }

    #[test]
    fn accumulator_rejects_degenerate_step_sizes() {
        let mut acc = FixedStepAccumulator::new(Duration::ZERO, 0);
        assert!(acc.step() >= Duration::from_micros(100), "zero step must be clamped");
        assert!(acc.steps(Duration::from_millis(1)) <= 1);
    }

    #[test]
    fn accumulator_step_change_discards_stale_remainder() {
        let mut acc = FixedStepAccumulator::new(Duration::from_millis(10), 10);
        let steps = acc.steps(Duration::from_millis(9));
        assert_eq!(steps, 0, "9 ms is less than one 10 ms step");
        assert_eq!(acc.remainder(), Duration::from_millis(9));
        acc.set_step(Duration::from_millis(5));
        assert_eq!(acc.remainder(), Duration::ZERO, "step change must not cause a burst");
    }

    #[test]
    fn pacer_refuses_to_submit_when_nothing_changed() {
        let pacer = FramePacer::new(60, true);
        assert!(!pacer.should_submit(Instant::now(), false));
    }

    #[test]
    fn pacer_allows_first_submit_immediately() {
        let mut pacer = FramePacer::new(60, false);
        let now = Instant::now();
        assert!(pacer.should_submit(now, true));
        pacer.record_submit(now);
    }

    #[test]
    fn pacer_enforces_the_target_rate_without_vsync() {
        let mut pacer = FramePacer::new(60, false);
        let start = Instant::now();
        pacer.record_submit(start);

        // Immediately after: not yet.
        assert!(!pacer.should_submit(start, true));
        // After the interval: yes.
        assert!(pacer.should_submit(start + Duration::from_millis(20), true));
    }

    #[test]
    fn pacer_does_not_rate_limit_when_vsynced() {
        let mut pacer = FramePacer::new(60, true);
        let start = Instant::now();
        pacer.record_submit(start);
        // With vsync the surface blocks for us, so the pacer must not add a
        // second rate limit on top of it.
        assert!(pacer.should_submit(start, true));
    }

    #[test]
    fn pacer_reset_forces_an_immediate_frame() {
        let mut pacer = FramePacer::new(60, false);
        let start = Instant::now();
        pacer.record_submit(start);
        assert!(!pacer.should_submit(start, true));
        pacer.reset();
        assert!(pacer.should_submit(start, true));
    }

    #[test]
    fn pacer_clamps_absurd_frame_rates() {
        let pacer = FramePacer::new(0, false);
        assert_eq!(pacer.target_fps(), 1);
        let pacer = FramePacer::new(100_000, false);
        assert_eq!(pacer.target_fps(), 480);
    }

    #[test]
    fn pacer_target_change_updates_the_interval() {
        let mut pacer = FramePacer::new(60, false);
        pacer.set_target_fps(30);
        assert_eq!(pacer.min_interval(), Duration::from_secs_f64(1.0 / 30.0));
    }

    #[test]
    fn time_until_next_shrinks_to_zero_as_the_interval_elapses() {
        let mut pacer = FramePacer::new(60, false);
        let start = Instant::now();
        pacer.record_submit(start);
        assert!(pacer.time_until_next(start) > Duration::ZERO);
        assert_eq!(pacer.time_until_next(start + Duration::from_secs(1)), Duration::ZERO);
    }

    #[test]
    fn ema_seeds_from_the_first_sample_then_smooths() {
        let mut ema = Ema::new(0.5);
        assert!(!ema.is_initialized());
        assert_eq!(ema.value(), 0.0);

        ema.update(100.0);
        assert_eq!(ema.value(), 100.0, "first sample must not be smoothed");
        assert!(ema.is_initialized());

        ema.update(0.0);
        assert_eq!(ema.value(), 50.0);
        ema.update(0.0);
        assert_eq!(ema.value(), 25.0);
    }

    #[test]
    fn ema_ignores_non_finite_samples() {
        let mut ema = Ema::new(0.5);
        ema.update(f32::NAN);
        assert!(!ema.is_initialized(), "a NaN must not initialize the average");
        ema.update(10.0);
        ema.update(f32::INFINITY);
        assert_eq!(ema.value(), 10.0);
    }

    #[test]
    fn ema_clamps_its_alpha() {
        let mut clamped_low = Ema::new(0.0);
        clamped_low.update(10.0);
        clamped_low.update(0.0);
        // An alpha clamped just above zero keeps almost all of the old value.
        assert!(clamped_low.value() > 9.0, "alpha 0 should mean almost no movement");

        let clamped_high = Ema::new(99.0);
        assert!(clamped_high.alpha <= 1.0);
    }

    #[test]
    fn ema_reset_clears_state() {
        let mut ema = Ema::new(0.5);
        ema.update(42.0);
        ema.reset();
        assert!(!ema.is_initialized());
        assert_eq!(ema.value(), 0.0);
    }
}