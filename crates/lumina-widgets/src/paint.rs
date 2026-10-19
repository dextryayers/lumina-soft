//! Shared drawing primitives for the data widgets.
//!
//! These are deliberately small axis aligned helpers. The renderer's SDF rounds
//! corners, so a square quad with a radius of half its size is a dot, and a row
//! of dots at circle positions is a progress ring. No helper allocates.

use lumina_core::geometry::{Rect, Scalar, Size, Vec2};

use crate::snapshot::History;
use crate::widget::{DrawList, Quad};

/// Primary text color, matching the island's default foreground.
pub(crate) const TEXT: [f32; 4] = [0.96, 0.97, 0.98, 1.0];
/// Secondary text color for units and hints.
pub(crate) const DIM: [f32; 4] = [0.66, 0.69, 0.74, 1.0];
/// Track color for gauges.
pub(crate) const TRACK: [f32; 4] = [1.0, 1.0, 1.0, 0.12];
/// Accent used for CPU and active states.
pub(crate) const ACCENT: [f32; 4] = [0.42, 0.78, 1.0, 1.0];
/// Green, used for healthy battery and completed focus.
pub(crate) const GOOD: [f32; 4] = [0.44, 0.88, 0.63, 1.0];
/// Amber, used for warnings.
pub(crate) const WARN: [f32; 4] = [1.0, 0.74, 0.32, 1.0];
/// Red, used for critical readings.
pub(crate) const CRITICAL: [f32; 4] = [1.0, 0.42, 0.42, 1.0];
/// Grey used for unavailable readings.
pub(crate) const MUTED: [f32; 4] = [0.5, 0.53, 0.58, 0.55];

/// Push a plain quad with an optional corner radius.
pub(crate) fn block(out: &mut DrawList, rect: Rect, color: [f32; 4], radius: Scalar) {
    if rect.width <= 0.0 || rect.height <= 0.0 || color[3] <= 0.0 {
        return;
    }
    out.push(
        Quad::new(
            Vec2::new(rect.x, rect.y),
            rect.size(),
            Vec2::ZERO,
            Vec2::ZERO,
            color,
        )
        .with_radius(radius),
    );
}

/// Push a placeholder for a run of text.
///
/// The renderer replaces these quads with glyphs from the atlas. Keeping the
/// text out of the widget means the frame path never touches font shaping.
pub(crate) fn label(out: &mut DrawList, rect: Rect, color: [f32; 4]) {
    block(out, rect, color, 2.0);
}

/// Draw a horizontal gauge with the given fill fraction.
pub(crate) fn gauge(
    out: &mut DrawList,
    rect: Rect,
    fraction: f32,
    fill: [f32; 4],
    radius: Scalar,
) {
    block(out, rect, TRACK, radius);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return;
    }
    let filled = Rect::new(
        rect.x,
        rect.y,
        (rect.width * fraction).max(1.0),
        rect.height,
    );
    block(out, filled, fill, radius);
}

/// Draw a sparkline inside `rect`, oldest sample left.
///
/// Bars are one pixel wide with a one pixel gap, which is the classic shape and
/// stays legible at the twenty-four bar default.
pub(crate) fn sparkline(out: &mut DrawList, rect: Rect, history: &History, color: [f32; 4]) {
    let count = history.len();
    if count == 0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let slot = rect.width / count as Scalar;
    let bar_width = (slot - 1.0).max(1.0);
    for (i, value) in history.normalized().enumerate() {
        let height = (value * rect.height).max(1.0);
        let x = rect.x + i as Scalar * slot;
        let bar = Rect::new(x, rect.y + rect.height - height, bar_width, height);
        block(out, bar, color, 1.0);
    }
}

/// Draw `segments` dots evenly spaced around a ring.
///
/// `progress` is the completed fraction; segments at or below it use `fill`,
/// the rest use `track`. The ring is drawn clockwise from the top.
pub(crate) fn dots_ring(
    out: &mut DrawList,
    center: Vec2,
    radius: Scalar,
    segments: usize,
    progress: f32,
    fill: [f32; 4],
    track: [f32; 4],
) {
    if segments == 0 || radius <= 0.0 {
        return;
    }
    let progress = progress.clamp(0.0, 1.0);
    let completed = (progress * segments as f32).round() as usize;
    let dot = 2.0;
    for i in 0..segments {
        let angle = -std::f32::consts::FRAC_PI_2
            + i as f32 / segments as f32 * std::f32::consts::TAU;
        let position = Vec2::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        let color = if i < completed { fill } else { track };
        let origin = Vec2::new(position.x - dot * 0.5, position.y - dot * 0.5);
        block(
            out,
            Rect::from_size(origin, Size::uniform(dot)),
            color,
            dot * 0.5,
        );
    }
}

/// Color for a percentage reading, from green through amber to red.
pub(crate) fn severity_color(percent: f32) -> [f32; 4] {
    if percent >= 90.0 {
        CRITICAL
    } else if percent >= 70.0 {
        WARN
    } else {
        ACCENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_sized_blocks_are_skipped() {
        let mut out = DrawList::new();
        block(&mut out, Rect::new(0.0, 0.0, 0.0, 10.0), TEXT, 0.0);
        block(&mut out, Rect::new(0.0, 0.0, 10.0, -1.0), TEXT, 0.0);
        assert!(out.is_empty());
    }

    #[test]
    fn a_gauge_with_no_fill_draws_only_the_track() {
        let mut out = DrawList::new();
        gauge(&mut out, Rect::new(0.0, 0.0, 100.0, 4.0), 0.0, GOOD, 2.0);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_gauge_clamps_overflowing_fraction() {
        let mut out = DrawList::new();
        gauge(&mut out, Rect::new(0.0, 0.0, 100.0, 4.0), 2.0, GOOD, 2.0);
        assert_eq!(out.len(), 2);
        assert_eq!(out.quads()[1].size.width, 100.0);
    }

    #[test]
    fn a_sparkline_draws_one_bar_per_sample() {
        let mut history = History::new(4);
        history.push(10.0);
        history.push(40.0);
        let mut out = DrawList::new();
        sparkline(&mut out, Rect::new(0.0, 0.0, 40.0, 10.0), &history, ACCENT);
        assert_eq!(out.len(), 2);
        assert!(out.quads()[1].size.height > out.quads()[0].size.height);
    }

    #[test]
    fn an_empty_sparkline_draws_nothing() {
        let mut out = DrawList::new();
        sparkline(
            &mut out,
            Rect::new(0.0, 0.0, 40.0, 10.0),
            &History::new(4),
            ACCENT,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn a_ring_draws_every_segment() {
        let mut out = DrawList::new();
        dots_ring(
            &mut out,
            Vec2::new(20.0, 20.0),
            10.0,
            16,
            0.5,
            GOOD,
            TRACK,
        );
        assert_eq!(out.len(), 16);
        assert_eq!(out.quads()[0].color, GOOD);
        assert_eq!(out.quads()[15].color, TRACK);
    }

    #[test]
    fn severity_colors_escalate() {
        assert_eq!(severity_color(10.0), ACCENT);
        assert_eq!(severity_color(75.0), WARN);
        assert_eq!(severity_color(95.0), CRITICAL);
    }
}
