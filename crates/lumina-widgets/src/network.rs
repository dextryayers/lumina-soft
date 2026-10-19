//! The network widget: upload and download rates, with an optional SSID.
//!
//! Rates are sampled by the service layer and handed in as bytes per second.
//! The widget keeps a short history for the inline sparkline and formats values
//! with the configured unit. An SSID is shown only when the platform can read it
//! without elevated privileges, and is optional everywhere else.

use lumina_config::schema::{NetworkConfig, NetworkUnit};
use lumina_core::geometry::{Rect, Size};
use lumina_core::state::{IslandState, StateRequest};

use crate::paint::{self, ACCENT, DIM, GOOD, MUTED};
use crate::snapshot::{FormattedRate, History, NetworkSnapshot};
use crate::widget::{DrawList, Widget, WidgetContext, WidgetInput};

/// The network widget.
#[derive(Debug, Clone)]
pub struct NetworkWidget {
    /// Whether to show the upload rate.
    pub show_upload: bool,
    /// Whether to show the download rate.
    pub show_download: bool,
    /// Whether to show the SSID when available.
    pub show_ssid: bool,
    /// The display unit.
    pub unit: NetworkUnit,
    /// The sampling interval from config, exposed for the service layer.
    pub interval_ms: u32,
    /// The latest reading.
    pub snapshot: NetworkSnapshot,
    /// Download history, normalised logarithmically.
    pub download_history: History,
    /// Upload history, normalised logarithmically.
    pub upload_history: History,
}

impl NetworkWidget {
    /// Construct a widget from the network configuration.
    #[must_use]
    pub fn from_config(config: &NetworkConfig) -> Self {
        Self {
            show_upload: config.show_upload,
            show_download: config.show_download,
            show_ssid: config.show_ssid,
            unit: config.unit,
            interval_ms: config.interval_ms.max(200),
            snapshot: NetworkSnapshot::default(),
            download_history: History::new(24),
            upload_history: History::new(24),
        }
    }

    /// Construct a widget with both directions enabled.
    #[must_use]
    pub fn new() -> Self {
        Self::from_config(&NetworkConfig {
            enabled: true,
            ..NetworkConfig::default()
        })
    }

    /// Store a new reading and advance both histories.
    pub fn update(&mut self, snapshot: NetworkSnapshot) {
        self.download_history
            .push(fraction(snapshot.download_bps));
        self.upload_history.push(fraction(snapshot.upload_bps));
        self.snapshot = snapshot;
    }

    /// The download rate rendered for display, for example `"2.4 MB/s"`.
    #[must_use]
    pub fn download_text(&self) -> String {
        FormattedRate::new(self.snapshot.download_bps, self.unit).render()
    }

    /// The upload rate rendered for display.
    #[must_use]
    pub fn upload_text(&self) -> String {
        FormattedRate::new(self.snapshot.upload_bps, self.unit).render()
    }

    /// The visible SSID when the platform supplied one.
    #[must_use]
    pub fn ssid(&self) -> Option<&str> {
        self.snapshot.ssid.as_deref()
    }

    /// Number of visible rows.
    #[must_use]
    pub fn row_count(&self) -> usize {
        usize::from(self.show_download) + usize::from(self.show_upload)
    }
}

impl Default for NetworkWidget {
    fn default() -> Self {
        Self::new()
    }
}

fn fraction(bytes_per_second: u64) -> f32 {
    if bytes_per_second == 0 {
        return 0.0;
    }
    let log = (bytes_per_second as f32 + 1.0).log10();
    (log / 8.0 * 100.0).clamp(0.0, 100.0)
}

impl Widget for NetworkWidget {
    fn id(&self) -> &'static str {
        "network"
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        let width = if self.ssid().is_some() {
            132.0
        } else {
            96.0
        };
        Size::new(width, (self.row_count().max(1) * 12 + 2) as f32)
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        let rows = self.row_count();
        if rows == 0 {
            return;
        }
        let row_height = rect.height / rows as f32;
        let mut visible = 0;
        if self.show_download {
            draw_row(
                out,
                rect,
                row_height,
                visible,
                &self.download_history,
                GOOD,
                false,
            );
            visible += 1;
        }
        if self.show_upload {
            draw_row(
                out,
                rect,
                row_height,
                visible,
                &self.upload_history,
                ACCENT,
                true,
            );
            if self.show_ssid && self.snapshot.ssid.is_none() {
                paint::label(
                    out,
                    Rect::new(rect.x, rect.y + row_height * 2.0, rect.width, 2.0),
                    MUTED,
                );
            }
        }
    }

    fn on_input(&mut self, _ev: &WidgetInput, _ctx: &WidgetContext, _rect: Rect) -> bool {
        false
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        None
    }

    fn priority(&self) -> u8 {
        IslandState::Metrics.priority()
    }
}

fn draw_row(
    out: &mut DrawList,
    rect: Rect,
    row_height: f32,
    index: usize,
    history: &History,
    color: [f32; 4],
    up: bool,
) {
    let y = rect.y + index as f32 * row_height;
    let arrow_x = rect.x + 1.0;
    let arrow_y = y + row_height * 0.5 - 2.0;
    paint::block(
        out,
        Rect::new(arrow_x, arrow_y, 4.0, 4.0),
        if up { ACCENT } else { GOOD },
        1.0,
    );
    paint::label(out, Rect::new(rect.x + 8.0, y + 1.0, 28.0, 2.0), DIM);
    let gauge = Rect::new(rect.x + 8.0, y + 5.0, rect.width - 8.0, 2.0);
    paint::gauge(
        out,
        gauge,
        history.latest().unwrap_or(0.0) / 100.0,
        color,
        1.0,
    );
    let spark = Rect::new(rect.x + 8.0, y + 8.0, rect.width - 8.0, 2.0);
    if !history.is_empty() {
        paint::sparkline(out, spark, history, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> WidgetContext {
        WidgetContext::new(IslandState::Metrics, Size::new(200.0, 40.0))
    }

    #[test]
    fn rates_are_formatted_in_the_configured_unit() {
        let mut widget = NetworkWidget::from_config(&NetworkConfig {
            unit: NetworkUnit::Mb,
            ..NetworkConfig::default()
        });
        widget.update(NetworkSnapshot {
            upload_bps: 2_500_000,
            download_bps: 1_000_000,
            ssid: None,
        });
        assert_eq!(widget.download_text(), "1 MB/s");
        assert_eq!(widget.upload_text(), "2.5 MB/s");
    }

    #[test]
    fn updates_advance_both_histories() {
        let mut widget = NetworkWidget::new();
        widget.update(NetworkSnapshot {
            upload_bps: 1_000,
            download_bps: 10_000,
            ssid: Some("home".to_string()),
        });
        assert_eq!(widget.download_history.len(), 1);
        assert_eq!(widget.upload_history.len(), 1);
        assert_eq!(widget.ssid(), Some("home"));
    }

    #[test]
    fn a_disabled_direction_is_not_drawn() {
        let mut widget = NetworkWidget::from_config(&NetworkConfig {
            show_upload: false,
            ..NetworkConfig::default()
        });
        assert_eq!(widget.row_count(), 1);
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 96.0, 14.0));
        assert!(!out.is_empty());
    }

    #[test]
    fn with_no_rows_enabled_the_widget_draws_nothing() {
        let mut widget = NetworkWidget::from_config(&NetworkConfig {
            show_upload: false,
            show_download: false,
            ..NetworkConfig::default()
        });
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 96.0, 14.0));
        assert!(out.is_empty());
    }

    #[test]
    fn the_widget_never_requests_a_state() {
        let widget = NetworkWidget::new();
        assert!(widget.request(&ctx()).is_none());
    }
}

