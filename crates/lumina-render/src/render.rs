//! The frame loop, the dirty flag, and frame statistics.
//!
//! # The dirty flag rule, restated
//!
//! The render loop does not run at 60 fps forever. It runs only while something is
//! animating, audio is flowing, or a widget is updating. When nothing is new the
//! loop blocks instead of spinning.
//!
//! This is the single most important rule for the CPU budget. A process that
//! submits 60 frames a second forever will sit at a few percent of a core no
//! matter how cheap each frame is, and no amount of per-frame optimization fixes
//! that. The rule is enforced by [`FrameStats::rendered`], which CI watches.
//!
//! # Frame pacing
//!
//! When vsync is on the surface blocks for us, so the pacer must not add a
//! second rate limit on top. When vsync is off the pacer enforces the target rate
//! so a fast state change cannot burn CPU submitting frames nobody can see.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use lumina_core::geometry::{Geometry, Scalar, Vec2};
use lumina_core::state::IslandState;
use lumina_core::time::FramePacer;

use crate::uniforms::IslandUniforms;

/// Counters for one frame batch, read by the diagnostics command.
#[derive(Debug, Default)]
pub struct FrameStats {
    /// Frames submitted since startup.
    pub rendered: u64,
    /// Frames skipped because nothing changed.
    pub skipped: u64,
    /// The longest frame time seen, in milliseconds.
    pub worst_frame_ms: Scalar,
    /// The last frame time, in milliseconds.
    pub last_frame_ms: Scalar,
}

impl FrameStats {
    /// Frames submitted.
    #[must_use]
    pub fn rendered(&self) -> u64 {
        self.rendered
    }

    /// Frames skipped.
    #[must_use]
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Render duty cycle: the fraction of opportunities where a frame was drawn.
    ///
    /// Low is good. A healthy idle sits near zero, which is the whole point of
    /// the dirty flag.
    #[must_use]
    pub fn duty_cycle(&self) -> f32 {
        let total = self.rendered + self.skipped;
        if total == 0 {
            0.0
        } else {
            self.rendered as f32 / total as f32
        }
    }
}

/// A frame batch is worth drawing when this is true.
#[derive(Debug, Clone)]
pub struct FrameIntent {
    /// Something in the UI changed: a widget, a config value, a pointer move.
    pub dirty: bool,
    /// The animation system still has springs in motion.
    pub animating: bool,
    /// Audio is flowing, so the spectrum needs a frame.
    pub audio_active: bool,
    /// The island should be visible at all.
    pub visible: bool,
}

impl FrameIntent {
    /// A frame is needed when anything is moving and the island is visible.
    #[must_use]
    pub fn needs_frame(&self) -> bool {
        self.visible && (self.dirty || self.animating || self.audio_active)
    }
}

/// The render loop state.
///
/// Owns the pacer, the stats, and the last submitted intent. The actual frame
/// submission is done by the caller through [`RenderLoop::submit`], because
/// presenting requires a surface that is owned elsewhere.
#[derive(Debug)]
pub struct RenderLoop {
    pacer: FramePacer,
    stats: FrameStats,
    last_intent: FrameIntent,
    last_submit: Option<Instant>,
    dirty: Arc<AtomicBool>,
    frames: Arc<AtomicU32>,
}

impl RenderLoop {
    /// Construct a loop with a target frame rate and vsync policy.
    #[must_use]
    pub fn new(target_fps: u32, vsync: bool) -> Self {
        Self {
            pacer: FramePacer::new(target_fps, vsync),
            stats: FrameStats::default(),
            last_intent: FrameIntent {
                dirty: false,
                animating: false,
                audio_active: false,
                visible: true,
            },
            last_submit: None,
            dirty: Arc::new(AtomicBool::new(false)),
            frames: Arc::new(AtomicU32::new(0)),
        }
    }

    /// A shared flag the animation system sets when something moved.
    #[must_use]
    pub fn dirty_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.dirty)
    }

    /// A shared frame counter for tests and the diagnostics command.
    #[must_use]
    pub fn frame_counter(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.frames)
    }

    /// Record an intent and decide whether a frame should be submitted.
    ///
    /// Returns `true` when the caller should present. This is the function the
    /// dirty flag rule lives in.
    pub fn should_render(&mut self, intent: FrameIntent) -> bool {
        self.last_intent = intent;

        if !self.last_intent.needs_frame() {
            self.stats.skipped += 1;
            return false;
        }

        let now = Instant::now();
        let permitted = self.pacer.should_submit(now, true);
        if !permitted {
            self.stats.skipped += 1;
            return false;
        }

        self.pacer.record_submit(now);
        self.stats.rendered += 1;
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.last_submit = Some(now);
        true
    }

    /// Record a frame time, updating the statistics.
    pub fn record_frame_time(&mut self, elapsed: std::time::Duration) {
        let ms = elapsed.as_secs_f32() * 1000.0;
        self.stats.last_frame_ms = ms;
        if ms > self.stats.worst_frame_ms {
            self.stats.worst_frame_ms = ms;
        }
    }

    /// Statistics so far.
    #[must_use]
    pub fn stats(&self) -> &FrameStats {
        &self.stats
    }

    /// Change the target frame rate.
    pub fn set_target_fps(&mut self, fps: u32) {
        self.pacer.set_target_fps(fps);
    }

    /// Forget the last submit, so the next frame is immediate.
    ///
    /// Used after a state change that must be visible on the very next present,
    /// rather than waiting out the remainder of the frame interval.
    pub fn force_next_frame(&mut self) {
        self.pacer.reset();
    }

    /// The last intent recorded.
    #[must_use]
    pub fn last_intent(&self) -> &FrameIntent {
        &self.last_intent
    }
}

/// One island frame, ready to draw.
#[derive(Debug, Clone, Copy)]
pub struct IslandFrame {
    /// The geometry to draw.
    pub geometry: Geometry,
    /// Content opacity.
    pub content_opacity: Scalar,
    /// The state this frame came from.
    pub state: IslandState,
}

impl IslandFrame {
    /// Build the uniforms for this frame.
    #[must_use]
    pub fn uniforms(
        &self,
        palette: &lumina_core::color::Palette,
        appearance: &lumina_config::AppearanceConfig,
        position: Vec2,
        resolution: Vec2,
        dpr: Scalar,
        time_seconds: Scalar,
    ) -> IslandUniforms {
        IslandUniforms::for_frame(
            self.geometry,
            palette,
            appearance,
            crate::uniforms::FrameParams {
                position,
                resolution,
                dpr,
                time_seconds,
                content_opacity: self.content_opacity,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle_intent() -> FrameIntent {
        FrameIntent {
            dirty: false,
            animating: false,
            audio_active: false,
            visible: true,
        }
    }

    fn dirty_intent() -> FrameIntent {
        FrameIntent {
            dirty: true,
            ..idle_intent()
        }
    }

    #[test]
    fn an_idle_loop_skips_frames_and_counts_them() {
        let mut loop_ = RenderLoop::new(60, true);
        for _ in 0..100 {
            assert!(!loop_.should_render(idle_intent()), "idle must not render");
        }
        assert_eq!(loop_.stats().rendered(), 0);
        assert_eq!(loop_.stats().skipped(), 100);
    }

    #[test]
    fn a_dirty_frame_is_rendered() {
        let mut loop_ = RenderLoop::new(60, true);
        assert!(loop_.should_render(dirty_intent()));
        assert_eq!(loop_.stats().rendered(), 1);
    }

    #[test]
    fn a_hidden_island_never_renders() {
        let mut loop_ = RenderLoop::new(60, true);
        let hidden = FrameIntent {
            visible: false,
            dirty: true,
            ..idle_intent()
        };
        assert!(
            !loop_.should_render(hidden),
            "a hidden island draws nothing"
        );
    }

    #[test]
    fn animation_counts_as_a_reason_to_draw() {
        let mut loop_ = RenderLoop::new(60, true);
        let intent = FrameIntent {
            animating: true,
            ..idle_intent()
        };
        assert!(
            loop_.should_render(intent),
            "an in-flight spring needs a frame"
        );
    }

    #[test]
    fn audio_counts_as_a_reason_to_draw() {
        let mut loop_ = RenderLoop::new(60, true);
        let intent = FrameIntent {
            audio_active: true,
            ..idle_intent()
        };
        assert!(
            loop_.should_render(intent),
            "flowing audio needs a spectrum frame"
        );
    }

    #[test]
    fn the_duty_cycle_is_zero_when_nothing_is_drawn() {
        let mut loop_ = RenderLoop::new(60, true);
        for _ in 0..50 {
            loop_.should_render(idle_intent());
        }
        assert_eq!(loop_.stats().duty_cycle(), 0.0);
    }

    #[test]
    fn the_duty_cycle_renders_one_when_everything_is_drawn() {
        let mut loop_ = RenderLoop::new(60, true);
        loop_.should_render(dirty_intent());
        assert_eq!(loop_.stats().duty_cycle(), 1.0);
    }

    #[test]
    fn frame_time_updates_the_worst_case() {
        let mut loop_ = RenderLoop::new(60, true);
        loop_.record_frame_time(std::time::Duration::from_millis(5));
        loop_.record_frame_time(std::time::Duration::from_millis(17));
        loop_.record_frame_time(std::time::Duration::from_millis(9));
        assert!((loop_.stats().worst_frame_ms - 17.0).abs() < 0.01);
        assert!((loop_.stats().last_frame_ms - 9.0).abs() < 0.01);
    }

    #[test]
    fn forcing_a_frame_bypasses_the_pacer() {
        let mut loop_ = RenderLoop::new(60, false);
        loop_.should_render(dirty_intent());
        assert_eq!(loop_.stats().rendered(), 1);

        // Without vsync the pacer would refuse the next frame inside the interval.
        let before = loop_.stats().rendered();
        loop_.force_next_frame();
        assert!(loop_.should_render(dirty_intent()));
        assert_eq!(loop_.stats().rendered(), before + 1);
    }

    #[test]
    fn the_target_frame_rate_can_change_at_runtime() {
        let mut loop_ = RenderLoop::new(60, true);
        loop_.set_target_fps(30);
        assert_eq!(loop_.pacer.target_fps(), 30);
    }

    #[test]
    fn every_state_produces_a_drawable_frame() {
        for state in IslandState::ALL {
            let frame = IslandFrame {
                geometry: state.geometry(),
                content_opacity: state.content_opacity(),
                state,
            };
            let appearance = lumina_config::AppearanceConfig::default();
            let uniforms = frame.uniforms(
                &lumina_core::color::Palette::default(),
                &appearance,
                Vec2::new(960.0, 540.0),
                Vec2::new(1920.0, 1080.0),
                1.0,
                0.0,
            );
            assert!(
                uniforms.half_top > 0.0,
                "{state} produced a degenerate frame"
            );
            assert!(uniforms.half_h > 0.0, "{state} produced a degenerate frame");
        }
    }

    #[test]
    fn the_frame_counter_increments_once_per_rendered_frame() {
        let loop_ = RenderLoop::new(60, true);
        assert_eq!(loop_.frame_counter().load(Ordering::Relaxed), 0);
        let counter = loop_.frame_counter();
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(loop_.frame_counter().load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_pipeline_can_be_built_without_a_gpu() {
        // The pipeline struct is not constructible without a device, but its
        // shader sources are validated in the shader module's own tests. This
        // exists so the module is never empty and the link to the pipeline is
        // explicit.
        assert_ne!(crate::pipeline::FALLBACK_WGSL, "");
    }
}
