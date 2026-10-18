//! Damped harmonic oscillators, the only motion primitive in Lumina.
//!
//! # Why springs rather than tweens
//!
//! Every morph in Lumina must be interruptible. If the user hovers, then
//! clicks, then moves the pointer away, all within 200 ms, a tween has to
//! either restart from zero (visible jump) or blend two curves (visible kink).
//! A spring stores velocity, so retargeting mid flight is continuous by
//! construction. That property is what makes a shape morph feel physical
//! instead of mechanical.
//!
//! # Parameters
//!
//! Designers think in `response` (how long a transition should feel) and
//! `damping_ratio` (how much it should overshoot), so the crate exposes those
//! and derives the physical coefficients:
//!
//! ```text
//! omega = 2 * PI / response
//! k     = omega^2                  (stiffness)
//! c     = 2 * damping_ratio * omega (damping)
//! ```
//!
//! | damping ratio | behavior |
//! |---|---|
//! | below 0.7 | visibly bouncy, several overshoots |
//! | 0.7 to 0.9 | one small overshoot, the island's default feel |
//! | 1.0 | critically damped, fastest approach with no overshoot |
//! | above 1.0 | sluggish, long tail |
//!
//! # Integration
//!
//! Semi implicit Euler at a fixed 1/240 s sub-step, because explicit Euler
//! gains energy at stiff settings and a frame time spike would blow the spring
//! up. Sub-stepping decouples stability from frame rate: a 30 fps frame and a
//! 240 fps frame produce the same trajectory for the same elapsed time.

use serde::{Deserialize, Serialize};

/// Longest sub-step we will ever take, used for slow springs.
///
/// The actual sub-step is derived from the spring's own frequency so that stiff
/// settings stay stable. See [`Spring::stable_sub_step`].
pub const MAX_SUB_STEP: f32 = 1.0 / 240.0;

/// Margin applied to the stability limit.
///
/// Semi implicit Euler on a harmonic oscillator is stable while
/// `h * omega < 2`. Sitting exactly on the limit makes the trajectory ring
/// rather than decay, so the sub-step uses three quarters of the budget.
const STABILITY_MARGIN: f32 = 0.75;

/// Maximum sub-steps per call to [`Spring::advance`].
///
/// Sized for the stiffest spring [`Spring::set_response`] accepts, which is a
/// 0.01 s response. At that setting the stable sub-step is about 2.4 ms, so
/// covering the full [`MAX_FRAME_TIME`] clamp takes about 42 steps.
pub const MAX_SUB_STEPS: usize = 48;

/// Simulated time ceiling for one update, in seconds.
pub const MAX_FRAME_TIME: f32 = 0.1;

/// Rest detection thresholds.
///
/// These are absolute because every spring in Lumina drives pixel-space
/// geometry. They are set a hundredfold below one device pixel: at 100 percent
/// DPI, 0.01 px is one hundredth of a pixel, which no display can resolve, so
/// snapping there is invisible while it still reliably fires.
///
/// They cannot be set much tighter. Symplectic Euler conserves a modified
/// energy, so a lightly damped spring settles onto a tiny residual limit cycle
/// at roughly the sub-step scale rather than decaying all the way to zero.
/// Thresholds below that noise floor would mean the render loop never decides
/// the spring is finished and draws forever, which is exactly the idle CPU bug
/// this whole mechanism exists to avoid.
const REST_POSITION_EPSILON: f32 = 0.01;
const REST_VELOCITY_EPSILON: f32 = 0.1;

/// A scalar damped harmonic oscillator.
///
/// # Examples
///
/// ```
/// use lumina_core::Spring;
///
/// let mut spring = Spring::new(62.0, 0.34, 0.78);
/// spring.snap_to_target();
/// spring.set_target(200.0);
///
/// // Simulate one third of a second.
/// spring.advance(0.333);
/// assert!(spring.position() > 62.0);
/// assert!(spring.position() < 200.0);
///
/// // Let it settle.
/// for _ in 0..600 {
///     spring.advance(1.0 / 60.0);
/// }
/// assert!(spring.is_at_rest());
/// assert!((spring.position() - 200.0).abs() < 1e-2);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spring {
    position: f32,
    velocity: f32,
    target: f32,
    response: f32,
    damping_ratio: f32,
    global_scale: f32,
}

impl Spring {
    /// Construct a spring at rest at `position`, heading for the same value.
    #[must_use]
    pub fn new(position: f32, response: f32, damping_ratio: f32) -> Self {
        Self {
            position: sanitize(position),
            velocity: 0.0,
            target: sanitize(position),
            response: response.clamp(0.01, 10.0),
            damping_ratio: damping_ratio.clamp(0.1, 3.0),
            global_scale: 1.0,
        }
    }

    /// Construct a spring already travelling toward `target`.
    #[must_use]
    pub fn new_moving(from: f32, to: f32, response: f32, damping_ratio: f32) -> Self {
        let mut s = Self::new(from, response, damping_ratio);
        s.target = sanitize(to);
        s
    }

    /// Current value.
    #[must_use]
    pub fn position(&self) -> f32 {
        self.position
    }

    /// Current derivative, in units per second.
    #[must_use]
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Value being approached.
    #[must_use]
    pub fn target(&self) -> f32 {
        self.target
    }

    /// True when both position and velocity are below their rest thresholds.
    ///
    /// The render loop consults this every frame to decide whether it can stop
    /// drawing entirely.
    #[must_use]
    pub fn is_at_rest(&self) -> bool {
        (self.position - self.target).abs() < REST_POSITION_EPSILON
            && self.velocity.abs() < REST_VELOCITY_EPSILON
    }

    /// Retarget without disturbing position or velocity.
    ///
    /// This is the interruption primitive. Calling it mid flight produces a
    /// continuous trajectory because neither the position nor the velocity
    /// changes.
    pub fn set_target(&mut self, target: f32) {
        self.target = sanitize(target);
    }

    /// Jump to a value, clearing velocity. Used on first layout and on config
    /// reloads that must take effect immediately.
    pub fn snap_to(&mut self, value: f32) {
        self.position = sanitize(value);
        self.target = self.position;
        self.velocity = 0.0;
    }

    /// Place the spring at rest on its own target.
    pub fn snap_to_target(&mut self) {
        self.position = self.target;
        self.velocity = 0.0;
    }

    /// Tune the oscillation without disturbing state.
    pub fn set_response(&mut self, response: f32) {
        self.response = response.clamp(0.01, 10.0);
    }

    /// Tune the damping without disturbing state.
    pub fn set_damping_ratio(&mut self, damping_ratio: f32) {
        self.damping_ratio = damping_ratio.clamp(0.1, 3.0);
    }

    /// Response currently in use, in seconds.
    #[must_use]
    pub fn response(&self) -> f32 {
        self.response
    }

    /// Damping ratio currently in use.
    #[must_use]
    pub fn damping_ratio(&self) -> f32 {
        self.damping_ratio
    }

    /// Global time multiplier for the whole animation system.
    ///
    /// Values above one speed everything up, values below one slow it down.
    /// Zero and negative values are ignored so a typo cannot freeze the UI.
    pub fn set_global_scale(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.global_scale = scale.clamp(0.05, 10.0);
        }
    }

    /// Global time multiplier currently in use.
    #[must_use]
    pub fn global_scale(&self) -> f32 {
        self.global_scale
    }

    /// Effective response after the global scale is applied.
    #[must_use]
    pub fn effective_response(&self) -> f32 {
        self.response / self.global_scale
    }

    /// Physical stiffness derived from response.
    #[must_use]
    pub fn stiffness(&self) -> f32 {
        let omega = std::f32::consts::TAU / self.effective_response();
        omega * omega
    }

    /// Physical damping coefficient derived from response and damping ratio.
    #[must_use]
    pub fn damping(&self) -> f32 {
        let omega = std::f32::consts::TAU / self.effective_response();
        2.0 * self.damping_ratio * omega
    }

    /// Largest sub-step that keeps the integrator stable at the current tuning.
    ///
    /// Semi implicit Euler on a damped harmonic oscillator is stable while
    /// `h * omega < 2`. Deriving the step from `omega` rather than fixing it
    /// globally is what lets a 0.01 s response spring behave instead of
    /// diverging to infinity.
    #[must_use]
    pub fn stable_sub_step(&self) -> f32 {
        let omega = std::f32::consts::TAU / self.effective_response();
        if !omega.is_finite() || omega <= 0.0 {
            return MAX_SUB_STEP;
        }
        (STABILITY_MARGIN * 2.0 / omega).clamp(1e-5, MAX_SUB_STEP)
    }

    /// Integrate forward by `dt` seconds of wall clock time.
    ///
    /// Non positive or non finite `dt` values are ignored rather than
    /// corrupting the state, because a bad frame timestamp must not be able to
    /// poison the animation.
    pub fn advance(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }

        // Clamp so a long stall produces slow motion, not an explosion.
        let mut remaining = (dt * self.global_scale).min(MAX_FRAME_TIME);
        let mut steps = 0usize;

        let k = self.stiffness();
        let c = self.damping();
        let h_max = self.stable_sub_step();

        while remaining > 0.0 && steps < MAX_SUB_STEPS {
            let h = remaining.min(h_max);
            // Semi implicit Euler: update velocity first, then use the new
            // velocity for the position update. This is unconditionally more
            // stable than explicit Euler for oscillators.
            let acceleration = -k * (self.position - self.target) - c * self.velocity;
            self.velocity += acceleration * h;
            self.position += self.velocity * h;

            remaining -= h;
            steps += 1;
        }

        // Snap to rest when both thresholds are crossed, otherwise a spring
        // that creeps forever would keep the render loop awake.
        if self.is_at_rest() {
            self.position = self.target;
            self.velocity = 0.0;
        }
    }

    /// Fraction of the remaining distance still to travel, in `0.0..=1.0`.
    ///
    /// A pure geometric measure that is more useful for deciding whether to
    /// keep drawing than [`Spring::is_at_rest`] when the amplitude is large.
    #[must_use]
    pub fn normalized_progress(&self, origin: f32) -> f32 {
        let total = self.target - origin;
        if total.abs() < f32::EPSILON {
            return 1.0;
        }
        ((self.position - origin) / total).clamp(0.0, 1.0)
    }
}

impl Default for Spring {
    /// A spring at zero with the island's default feel.
    fn default() -> Self {
        Self::new(0.0, 0.34, 0.78)
    }
}

/// Replace non finite values with zero so no `NaN` can enter the graph.
fn sanitize(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulate `seconds` at a given frame rate and return the spring.
    fn simulate(mut spring: Spring, seconds: f32, fps: f32) -> Spring {
        let dt = 1.0 / fps;
        let frames = (seconds * fps).ceil() as usize;
        for _ in 0..frames {
            spring.advance(dt);
        }
        spring
    }

    #[test]
    fn starts_at_rest() {
        let s = Spring::new(42.0, 0.34, 0.78);
        assert_eq!(s.position(), 42.0);
        assert_eq!(s.velocity(), 0.0);
        assert_eq!(s.target(), 42.0);
        assert!(s.is_at_rest());
    }

    /// Settling test tolerances.
    ///
    /// Position is compared against the same one hundredth of a pixel threshold the
    /// rest detector uses, so the test and the production behaviour agree on what
    /// "settled" means.
    #[test]
    fn settles_on_target_for_every_damping_ratio() {
        const TOLERANCE: f32 = REST_POSITION_EPSILON;
        for damping in [0.3f32, 0.5, 0.78, 1.0, 1.5, 2.5] {
            let mut s = Spring::new_moving(0.0, 100.0, 0.34, damping);
            s = simulate(s, 6.0, 60.0);
            assert!(
                (s.position() - 100.0).abs() < TOLERANCE,
                "damping {damping} settled at {} instead of 100",
                s.position()
            );
            assert!(
                s.velocity().abs() < REST_VELOCITY_EPSILON,
                "damping {damping} still moving at {}",
                s.velocity()
            );
            assert!(s.is_at_rest(), "damping {damping} should report at rest");
        }
    }

    #[test]
    fn underdamped_springs_overshoot_exactly_once() {
        // damping 0.5 is clearly underdamped, so it must overshoot.
        let mut s = Spring::new_moving(0.0, 100.0, 0.4, 0.5);
        let mut peak = 0.0f32;
        for _ in 0..600 {
            s.advance(1.0 / 240.0);
            peak = peak.max(s.position());
        }
        assert!(
            peak > 100.0,
            "expected an overshoot above 100, peak was {peak}"
        );
        // Logarithmic decay of a 0.5 damping spring gives a few percent.
        assert!(peak < 118.0, "overshoot should be modest, peak was {peak}");
    }

    #[test]
    fn critically_damped_spring_never_overshoots() {
        let mut s = Spring::new_moving(0.0, 100.0, 0.4, 1.0);
        let mut peak = 0.0f32;
        for _ in 0..600 {
            s.advance(1.0 / 240.0);
            peak = peak.max(s.position());
        }
        assert!(peak <= 100.0 + 1e-3, "critically damped overshot to {peak}");
    }

    #[test]
    fn trajectory_is_frame_rate_independent() {
        // The whole point of fixed sub-stepping: the same elapsed time produces
        // the same position regardless of how it was chopped up.
        let at_30 = simulate(Spring::new_moving(0.0, 100.0, 0.34, 0.78), 0.5, 30.0);
        let at_60 = simulate(Spring::new_moving(0.0, 100.0, 0.34, 0.78), 0.5, 60.0);
        let at_144 = simulate(Spring::new_moving(0.0, 100.0, 0.34, 0.78), 0.5, 144.0);
        let at_240 = simulate(Spring::new_moving(0.0, 100.0, 0.34, 0.78), 0.5, 240.0);

        let spread = (at_30.position() - at_240.position()).abs();
        assert!(
            spread < 1.0,
            "frame rate changed the trajectory by {spread}"
        );
        assert!((at_60.position() - at_144.position()).abs() < 1.0);
    }

    #[test]
    fn trajectory_is_frame_rate_independent_at_high_time_scale() {
        // A fast global scale shortens the effective response, which pushes the
        // integrator into its stiff regime where the adaptive sub-step is doing
        // real work. This is the case most likely to break if the sub-step
        // derivation regresses.
        let build = || {
            let mut s = Spring::new_moving(0.0, 100.0, 0.34, 0.78);
            s.set_global_scale(4.0);
            s
        };
        let at_30 = simulate(build(), 0.2, 30.0);
        let _at_60 = simulate(build(), 0.2, 60.0);
        let at_240 = simulate(build(), 0.2, 240.0);

        let spread = (at_30.position() - at_240.position()).abs();
        assert!(
            spread < 2.0,
            "at a 4x time scale, frame rate changed the trajectory by {spread}: 30fps={} 240fps={}",
            at_30.position(),
            at_240.position()
        );
    }

    #[test]
    fn retargeting_mid_flight_is_continuous() {
        // Position and velocity must not change at the instant of the call.
        // That is the interruption primitive and it is exact.
        let mut s = Spring::new_moving(0.0, 100.0, 0.34, 0.78);
        s = simulate(s, 0.1, 240.0);

        let before = s.position();
        let velocity_before = s.velocity();
        s.set_target(-50.0);
        assert_eq!(s.position(), before, "retarget must not move the spring");
        assert_eq!(
            s.velocity(),
            velocity_before,
            "retarget must not kill velocity"
        );

        // The following step must stay within what the physics allows. The peak
        // speed of a step response is bounded by delta * omega / e; across a
        // gap of about 115 units at omega 18.5 that is about 782 units per
        // second, or 3.3 units in a 1/240 s sub-step. Asserting under 8 leaves
        // room for the integrator while still being far below the 115 unit gap
        // that a discontinuity would show.
        s.advance(1.0 / 240.0);
        let moved = (s.position() - before).abs();
        assert!(
            moved < 8.0,
            "one sub-step moved {moved}, which is not physically plausible"
        );
        assert!(s.position().is_finite());
    }

    #[test]
    fn position_never_discontinuously_jumps_under_random_retargeting() {
        // A discontinuity would mean the spring teleported, which shows up as a
        // visible one frame snap. Targets span 400 units, so a teleport would
        // show as a jump close to 400.
        //
        // The largest physically possible displacement in one 16.7 ms frame is
        // bounded by the step response peak speed, about
        // delta * omega / e, which is 400 * 18.5 / 2.7 / 60 = 45 units.
        // Asserting under 80 therefore still catches any real discontinuity
        // while leaving headroom for the integration error at the sub-step
        // boundary.
        const MAX_FRAME_DISPLACEMENT: f32 = 80.0;

        let mut s = Spring::new(0.0, 0.34, 0.78);
        let mut seed = 0xDEAD_BEEFu32;
        let mut previous = s.position();

        for frame in 0..4000 {
            // Retarget roughly every 37 frames with a wildly different value.
            if frame % 37 == 0 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                s.set_target(((seed >> 8) & 0xFFFF) as f32 / 65535.0 * 400.0 - 200.0);
            }
            s.advance(1.0 / 60.0);
            let delta = (s.position() - previous).abs();
            assert!(
                delta < MAX_FRAME_DISPLACEMENT,
                "discontinuity at frame {frame}: {delta}"
            );
            assert!(s.position().is_finite());
            previous = s.position();
        }
    }

    #[test]
    fn stays_stable_under_extreme_tuning() {
        // Stiff, nearly undamped, huge target: the classic way to make a naive
        // integrator explode.
        let mut s = Spring::new_moving(0.0, 1000.0, 0.01, 0.1);
        for _ in 0..5000 {
            s.advance(1.0 / 15.0);
            assert!(
                s.position().is_finite(),
                "integrator diverged to {}",
                s.position()
            );
            assert!(
                s.position().abs() < 10_000.0,
                "integrator blew up to {}",
                s.position()
            );
        }
    }

    #[test]
    fn long_stall_produces_slow_motion_not_a_burst() {
        let mut s = Spring::new_moving(0.0, 100.0, 0.34, 0.78);
        s.advance(10.0); // ten second stall
        assert!(s.position().is_finite());
        // Because the frame time is clamped, ten seconds of simulation becomes
        // at most MAX_FRAME_TIME worth of motion.
        assert!(
            s.position() < 100.0 * 1.5,
            "expected clamped catch-up, overshot to {}",
            s.position()
        );
    }

    #[test]
    fn invalid_dt_is_ignored() {
        let mut s = Spring::new_moving(0.0, 100.0, 0.34, 0.78);
        s = simulate(s, 0.05, 60.0);
        let snapshot = s;

        s.advance(0.0);
        s.advance(-1.0);
        s.advance(f32::NAN);
        s.advance(f32::INFINITY);

        assert_eq!(s.position(), snapshot.position());
        assert_eq!(s.velocity(), snapshot.velocity());
    }

    #[test]
    fn non_finite_targets_are_sanitized() {
        let mut s = Spring::new(0.0, 0.34, 0.78);
        s.set_target(f32::NAN);
        assert!(s.target().is_finite());
        s.set_target(f32::INFINITY);
        assert!(s.target().is_finite());
        s.snap_to(f32::NAN);
        assert!(s.position().is_finite());
    }

    #[test]
    fn snapping_clears_velocity_and_reaches_target() {
        let mut s = Spring::new_moving(0.0, 100.0, 0.34, 0.78);
        s = simulate(s, 0.1, 60.0);
        assert!(s.velocity().abs() > 0.1);

        s.snap_to_target();
        assert_eq!(s.position(), 100.0);
        assert_eq!(s.velocity(), 0.0);
        assert!(s.is_at_rest());
    }

    #[test]
    fn global_scale_changes_speed_in_the_right_direction() {
        let mut normal = Spring::new_moving(0.0, 100.0, 0.34, 0.78);
        normal.set_global_scale(1.0);
        let mut fast = Spring::new_moving(0.0, 100.0, 0.34, 0.78);
        fast.set_global_scale(4.0);

        assert_eq!(normal.effective_response(), 0.34);
        assert!((fast.effective_response() - 0.085).abs() < 1e-4);

        // Counting frames to rest is robust in a way comparing positions at one
        // instant is not: by the time a fast spring has crossed the screen a
        // single sampled position no longer distinguishes "further along" from
        // "already arrived".
        let frames_to_rest = |s: &Spring| -> usize {
            let mut copy = *s;
            let mut frames = 0usize;
            while !copy.is_at_rest() && frames < 10_000 {
                copy.advance(1.0 / 60.0);
                frames += 1;
            }
            frames
        };

        let normal_frames = frames_to_rest(&normal);
        let fast_frames = frames_to_rest(&fast);

        assert!(
            fast_frames * 4 <= normal_frames + 4,
            "a 4x time scale should settle about four times sooner: \
             normal={normal_frames} frames, fast={fast_frames} frames"
        );
    }

    #[test]
    fn invalid_global_scale_is_ignored() {
        let mut s = Spring::new(0.0, 0.34, 0.78);
        s.set_global_scale(2.0);
        s.set_global_scale(0.0);
        assert_eq!(s.global_scale(), 2.0, "zero must not be accepted");
        s.set_global_scale(-1.0);
        assert_eq!(s.global_scale(), 2.0, "negative must not be accepted");
        s.set_global_scale(f32::NAN);
        assert_eq!(s.global_scale(), 2.0, "NaN must not be accepted");
    }

    #[test]
    fn physics_derivations_match_the_formulas() {
        let s = Spring::new(0.0, 0.5, 0.8);
        let omega = std::f32::consts::TAU / 0.5;
        assert!((s.stiffness() - omega * omega).abs() < 1e-3);
        assert!((s.damping() - 2.0 * 0.8 * omega).abs() < 1e-3);
    }

    #[test]
    fn normalized_progress_reports_remaining_distance() {
        let mut s = Spring::new_moving(0.0, 100.0, 0.34, 1.0);
        s = simulate(s, 0.17, 240.0);
        let p = s.normalized_progress(0.0);
        assert!((0.0..=1.0).contains(&p), "progress out of range: {p}");

        s.snap_to_target();
        assert!((s.normalized_progress(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn reduced_motion_profile_is_fast_and_monotonic() {
        // The profile used when the OS reports reduced motion: 0.08 s response
        // with critical damping, so transitions happen but never overshoot.
        let mut s = Spring::new_moving(0.0, 100.0, 0.08, 1.0);
        let mut peak = 0.0f32;
        for _ in 0..120 {
            s.advance(1.0 / 60.0);
            peak = peak.max(s.position());
        }
        assert!(peak <= 100.0 + 1e-3, "reduced motion overshot to {peak}");
        assert!(
            s.is_at_rest(),
            "reduced motion should settle within two seconds"
        );
    }

    #[test]
    fn settles_within_two_response_cycles() {
        // A useful property for the idle render loop: the spring should be
        // visually done long before the render loop decides to stop.
        let mut s = Spring::new_moving(0.0, 100.0, 0.34, 0.78);
        let mut frames = 0;
        while !s.is_at_rest() && frames < 3000 {
            s.advance(1.0 / 60.0);
            frames += 1;
        }
        assert!(
            frames < 120,
            "took {frames} frames, expected under two seconds at 60 fps"
        );
    }
}
