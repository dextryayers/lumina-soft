//! The metrics widget: processor, memory, graphics, and network load.
//!
//! # Rendering
//!
//! Each enabled reading is one row: a short label placeholder, a gauge, and a
//! sparkline underneath. A reading the platform cannot supply renders as a muted
//! track, never as a zero, so "unknown" and "idle" are visually distinct.
//!
//! # Alerts
//!
//! `alert_on_threshold` is a fraction of full scale. When any available reading
//! is at or above it the widget requests the [`IslandState::Metrics`] state for
//! a few seconds. The request is stateless: while the reading stays high the
//! arbiter keeps the state alive, and it expires on its own once the load drops.

use std::time::{SystemTime, UNIX_EPOCH};

use lumina_config::schema::MetricsConfig;
use lumina_core::geometry::{Rect, Size};
use lumina_core::state::{IslandState, StateRequest};

use crate::paint::{self, ACCENT, DIM, MUTED};
use crate::snapshot::{History, MetricsSnapshot};
use crate::widget::{DrawList, Widget, WidgetContext, WidgetInput};

/// How long an alert keeps the island expanded after the last high sample.
const ALERT_HOLD_MS: u64 = 4_000;

/// The metrics widget.
#[derive(Debug, Clone)]
pub struct MetricsWidget {
    /// Whether to show processor load.
    pub show_cpu: bool,
    /// Whether to show memory load.
    pub show_ram: bool,
    /// Whether to show graphics load.
    pub show_gpu: bool,
    /// Whether to show network throughput.
    pub show_network: bool,
    /// The threshold that triggers an alert, as a fraction of full scale.
    pub alert_on_threshold: f32,
    /// The latest sample.
    pub snapshot: MetricsSnapshot,
    /// Processor history.
    pub cpu_history: History,
    /// Memory history.
    pub ram_history: History,
    /// Graphics history.
    pub gpu_history: History,
    /// Download history, normalised logarithmically.
    pub download_history: History,
    /// Upload history, normalised logarithmically.
    pub upload_history: History,
}

impl MetricsWidget {
    /// Construct a widget from the metrics configuration.
    #[must_use]
    pub fn from_config(config: &MetricsConfig) -> Self {
        let bars = config.sparkline_bars as usize;
        Self {
            show_cpu: config.show_cpu,
            show_ram: config.show_ram,
            show_gpu: config.show_gpu,
            show_network: config.show_network,
            alert_on_threshold: config.alert_on_threshold.clamp(0.0, 1.0),
            snapshot: MetricsSnapshot::default(),
            cpu_history: History::new(bars),
            ram_history: History::new(bars),
            gpu_history: History::new(bars),
            download_history: History::new(bars),
            upload_history: History::new(bars),
        }
    }

    /// Construct a widget with every reading enabled.
    #[must_use]
    pub fn new() -> Self {
        Self::from_config(&MetricsConfig {
            enabled: true,
            show_gpu: true,
            show_network: true,
            ..MetricsConfig::default()
        })
    }

    /// Store a new sample and append the available readings to their histories.
    pub fn update(&mut self, snapshot: MetricsSnapshot) {
        self.snapshot = snapshot.sanitized();
        if let Some(cpu) = self.snapshot.cpu_percent {
            self.cpu_history.push(cpu);
        }
        if let Some(ram) = self.snapshot.ram_percent {
            self.ram_history.push(ram);
        }
        if let Some(gpu) = self.snapshot.gpu_percent {
            self.gpu_history.push(gpu);
        }
        if self.show_network {
            self.download_history
                .push(throughput_fraction(self.snapshot.download_bps));
            self.upload_history
                .push(throughput_fraction(self.snapshot.upload_bps));
        }
    }

    /// The enabled rows, in drawing order.
    fn rows(&self) -> impl Iterator<Item = Row> + '_ {
        let rows = [
            (self.show_cpu, Row::Cpu),
            (self.show_ram, Row::Ram),
            (self.show_gpu, Row::Gpu),
            (self.show_network, Row::Network),
        ];
        rows.into_iter()
            .filter(|(enabled, _)| *enabled)
            .map(|(_, row)| row)
    }

    /// Number of rows the widget will draw.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows().count()
    }
}

impl Default for MetricsWidget {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Cpu,
    Ram,
    Gpu,
    Network,
}

/// Map a byte rate onto `0..=100` on a logarithmic scale.
///
/// A linear scale makes everything below a hundred kilobytes look empty. The
/// logarithm gives ordinary browsing and a large download both a visible bar;
/// ten megabytes per second lands at roughly 88 percent.
fn throughput_fraction(bytes_per_second: u64) -> f32 {
    if bytes_per_second == 0 {
        return 0.0;
    }
    let log = (bytes_per_second as f32 + 1.0).log10();
    (log / 8.0 * 100.0).clamp(0.0, 100.0)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Widget for MetricsWidget {
    fn id(&self) -> &'static str {
        "metrics"
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        let rows = self.row_count().max(1) as f32;
        Size::new(88.0, rows * 14.0 + 4.0)
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        let rows = self.row_count();
        if rows == 0 {
            return;
        }
        let row_height = rect.height / rows as f32;
        for (visible_index, row) in self.rows().enumerate() {
            let y = rect.y + visible_index as f32 * row_height;
            let line = Rect::new(rect.x, y, rect.width, row_height);
            paint::label(out, Rect::new(line.x, line.y + 1.0, 26.0, 8.0), DIM);

            let gauge_rect = Rect::new(line.x + 30.0, line.y + 2.0, line.width - 30.0, 4.0);
            let (fraction, color) = match row {
                Row::Cpu => (
                    self.snapshot.cpu_percent.map_or(0.0, |v| v / 100.0),
                    self.snapshot.cpu_percent.map_or(MUTED, paint::severity_color),
                ),
                Row::Ram => (
                    self.snapshot.ram_percent.map_or(0.0, |v| v / 100.0),
                    self.snapshot.ram_percent.map_or(MUTED, paint::severity_color),
                ),
                Row::Gpu => (
                    self.snapshot.gpu_percent.map_or(0.0, |v| v / 100.0),
                    self.snapshot.gpu_percent.map_or(MUTED, paint::severity_color),
                ),
                Row::Network => (
                    {
                        let down = throughput_fraction(self.snapshot.download_bps) / 100.0;
                        let up = throughput_fraction(self.snapshot.upload_bps) / 100.0;
                        down.max(up)
                    },
                    ACCENT,
                ),
            };
            paint::gauge(out, gauge_rect, fraction, color, 2.0);
            paint::label(out, Rect::new(line.x, line.y + 7.0, line.width, 1.0), MUTED);

            let spark_rect = Rect::new(line.x, line.y + 9.0, line.width, (row_height - 10.0).max(1.0));
            let history = match row {
                Row::Cpu => &self.cpu_history,
                Row::Ram => &self.ram_history,
                Row::Gpu => &self.gpu_history,
                Row::Network => &self.download_history,
            };
            if !history.is_empty() {
                paint::sparkline(out, spark_rect, history, color);
            }
        }
    }

    fn on_input(&mut self, _ev: &WidgetInput, _ctx: &WidgetContext, _rect: Rect) -> bool {
        false
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        if self.alert_on_threshold <= 0.0 {
            return None;
        }
        let peak = self.snapshot.peak_percent()? / 100.0;
        if peak >= self.alert_on_threshold {
            Some(StateRequest::until(
                IslandState::Metrics,
                now_ms() + ALERT_HOLD_MS,
            ))
        } else {
            None
        }
    }

    fn priority(&self) -> u8 {
        IslandState::Metrics.priority()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> WidgetContext {
        WidgetContext::new(IslandState::Metrics, Size::new(200.0, 40.0))
    }

    #[test]
    fn a_default_widget_enables_cpu_ram_and_network() {
        let widget = MetricsWidget::from_config(&MetricsConfig {
            enabled: true,
            show_network: true,
            ..MetricsConfig::default()
        });
        assert_eq!(widget.row_count(), 3);
    }

    #[test]
    fn sampling_advances_only_available_histories() {
        let mut widget = MetricsWidget::new();
        widget.update(MetricsSnapshot {
            cpu_percent: Some(25.0),
            ram_percent: None,
            gpu_percent: Some(10.0),
            upload_bps: 1000,
            download_bps: 2000,
        });
        assert_eq!(widget.cpu_history.latest(), Some(25.0));
        assert!(widget.ram_history.is_empty());
        assert_eq!(widget.gpu_history.latest(), Some(10.0));
        assert!(widget.download_history.latest().is_some());
    }

    #[test]
    fn drawing_without_data_still_draws_placeholders() {
        let mut widget = MetricsWidget::new();
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 88.0, 32.0));
        assert!(!out.is_empty());
    }

    #[test]
    fn drawing_with_data_emits_gauges_and_sparklines() {
        let mut widget = MetricsWidget::new();
        widget.update(MetricsSnapshot {
            cpu_percent: Some(80.0),
            ram_percent: Some(40.0),
            gpu_percent: Some(10.0),
            upload_bps: 1_000,
            download_bps: 5_000_000,
        });
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 88.0, 60.0));
        assert!(out.len() > widget.row_count() * 4);
    }

    #[test]
    fn a_high_reading_requests_the_metrics_state() {
        let mut widget = MetricsWidget::new();
        widget.alert_on_threshold = 0.8;
        widget.update(MetricsSnapshot {
            cpu_percent: Some(85.0),
            ..MetricsSnapshot::default()
        });
        let request = widget.request(&ctx()).expect("threshold is crossed");
        assert_eq!(request.state, IslandState::Metrics);
        assert!(request.expires_at_ms.is_some());
    }

    #[test]
    fn a_low_reading_does_not_request_a_state() {
        let mut widget = MetricsWidget::new();
        widget.alert_on_threshold = 0.8;
        widget.update(MetricsSnapshot {
            cpu_percent: Some(10.0),
            ..MetricsSnapshot::default()
        });
        assert!(widget.request(&ctx()).is_none());
    }

    #[test]
    fn zero_threshold_disables_alerts() {
        let mut widget = MetricsWidget::new();
        widget.alert_on_threshold = 0.0;
        widget.update(MetricsSnapshot {
            cpu_percent: Some(100.0),
            ..MetricsSnapshot::default()
        });
        assert!(widget.request(&ctx()).is_none());
    }

    #[test]
    fn throughput_is_logarithmic() {
        assert_eq!(throughput_fraction(0), 0.0);
        assert!(throughput_fraction(1_000_000) > 60.0);
        assert!(throughput_fraction(100_000_000) > 90.0);
    }

    #[test]
    fn the_widget_measures_rows_not_pixels() {
        let mut widget = MetricsWidget::from_config(&MetricsConfig {
            show_cpu: true,
            show_ram: false,
            show_gpu: false,
            show_network: false,
            ..MetricsConfig::default()
        });
        let size = widget.measure(&ctx());
        assert_eq!(size.height, 18.0);
    }
}
