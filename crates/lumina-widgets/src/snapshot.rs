//! Data snapshots and pure helpers shared by the data driven widgets.
//!
//! # Why snapshots
//!
//! Widgets never touch the operating system. They consume immutable snapshots
//! produced by the service layer and turn them into quads. That split keeps the
//! frame path free of blocking I/O and makes every rendering decision testable
//! without a device, a network, or a battery.
//!
//! Each snapshot has an `Option` for every reading that can be missing. A
//! machine without a battery, a driver that does not expose GPU load, or a
//! weather fetch that has not completed yet are all normal states, not errors.
//! Widgets render a muted placeholder for those readings and never request a
//! state they cannot fill.
//!
//! # History
//!
//! [`History`] is a fixed capacity ring used for sparklines. Pushing once per
//! sampling interval is the only mutation, and the buffer never grows beyond the
//! configured number of bars, so a widget can run for days without allocating.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use lumina_config::schema::NetworkUnit;

/// Largest sparkline history any widget accepts.
///
/// A thousand bars is roughly a minute of samples at 60 Hz, or sixteen minutes
/// at one sample per second. Beyond that a sparkline is a solid block on every
/// display, so the value is clamped rather than allowed to grow without bound.
pub const MAX_HISTORY: usize = 1024;

/// One system metrics sample.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MetricsSnapshot {
    /// Processor load, in percent. `None` when the platform cannot report it.
    pub cpu_percent: Option<f32>,
    /// Committed memory load, in percent. `None` when it cannot be read.
    pub ram_percent: Option<f32>,
    /// Graphics load, in percent. `None` on drivers without a load counter.
    pub gpu_percent: Option<f32>,
    /// Outgoing bytes per second.
    pub upload_bps: u64,
    /// Incoming bytes per second.
    pub download_bps: u64,
}

impl MetricsSnapshot {
    /// Clamp every percentage into `0..=100` and drop non finite values.
    ///
    /// Service layer bugs and unusual drivers should produce a missing reading,
    /// never a bar drawn at negative width or infinity.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.cpu_percent = sanitize_percent(self.cpu_percent);
        self.ram_percent = sanitize_percent(self.ram_percent);
        self.gpu_percent = sanitize_percent(self.gpu_percent);
        self
    }

    /// The highest available load, used for threshold alerts.
    #[must_use]
    pub fn peak_percent(&self) -> Option<f32> {
        let values = [self.cpu_percent, self.ram_percent, self.gpu_percent];
        values.into_iter().flatten().reduce(f32::max)
    }
}

fn sanitize_percent(value: Option<f32>) -> Option<f32> {
    value.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 100.0))
}

/// A fixed capacity history of samples in `0..=100`.
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    values: VecDeque<f32>,
    capacity: usize,
}

impl History {
    /// Create a history that holds at most `capacity` samples.
    ///
    /// A capacity of zero is raised to one: a history that can hold nothing
    /// would make [`History::push`] a silent no-op, which is never what a caller
    /// asks for.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.clamp(1, MAX_HISTORY);
        Self {
            values: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push one sample, dropping the oldest when full.
    pub fn push(&mut self, value: f32) {
        if !value.is_finite() {
            return;
        }
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value.clamp(0.0, 100.0));
    }

    /// Remove every sample, keeping the capacity.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// The samples, oldest first.
    #[must_use]
    pub fn values(&self) -> &VecDeque<f32> {
        &self.values
    }

    /// The number of stored samples.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when no sample has been stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The configured capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The most recent sample.
    #[must_use]
    pub fn latest(&self) -> Option<f32> {
        self.values.back().copied()
    }

    /// Scale every stored sample to `0.0..=1.0` for drawing.
    ///
    /// The scale is fixed at 100 percent rather than auto-scaled to the maximum
    /// sample, so a quiet machine does not render a dramatic looking sparkline
    /// out of noise.
    #[must_use]
    pub fn normalized(&self) -> impl Iterator<Item = f32> + '_ {
        self.values.iter().map(|v| (v / 100.0).clamp(0.0, 1.0))
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new(24)
    }
}

/// One battery sample.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BatterySnapshot {
    /// Whether a battery is present at all. Desktops usually report `false`.
    pub present: bool,
    /// Charge in percent.
    pub percent: f32,
    /// Whether external power is connected.
    pub charging: bool,
    /// Estimated seconds until empty, or until full while charging.
    pub time_remaining_secs: Option<u64>,
}

impl BatterySnapshot {
    /// Clamp the charge and discard impossible estimates.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.percent = if self.percent.is_finite() {
            self.percent.clamp(0.0, 100.0)
        } else {
            0.0
        };
        self.time_remaining_secs = self.time_remaining_secs.filter(|s| *s > 0);
        self
    }

    /// True when the reading is usable for display.
    #[must_use]
    pub fn is_usable(self) -> bool {
        self.present && self.percent.is_finite()
    }

    /// The charge expressed as `0.0..=1.0`.
    #[must_use]
    pub fn fraction(self) -> f32 {
        (self.percent / 100.0).clamp(0.0, 1.0)
    }
}

/// One network throughput sample.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkSnapshot {
    /// Outgoing bytes per second.
    pub upload_bps: u64,
    /// Incoming bytes per second.
    pub download_bps: u64,
    /// Wireless network name, when the platform can report it without extra
    /// privileges.
    pub ssid: Option<String>,
}

/// A rate in bytes per second, rendered for display.
///
/// The unit and the numeric value are kept apart so the widget can draw them at
/// different weights without parsing its own output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FormattedRate {
    /// The value in the chosen unit.
    pub value: f32,
    /// The unit suffix, for example `MB/s`.
    pub unit: &'static str,
}

impl FormattedRate {
    /// Format `bytes_per_sec` using the configured unit.
    #[must_use]
    pub fn new(bytes_per_sec: u64, unit: NetworkUnit) -> Self {
        let bytes = bytes_per_sec as f64;
        match unit {
            NetworkUnit::B => Self {
                value: bytes as f32,
                unit: "B/s",
            },
            NetworkUnit::Kb => Self {
                value: (bytes / 1_000.0) as f32,
                unit: "KB/s",
            },
            NetworkUnit::Mb => Self {
                value: (bytes / 1_000_000.0) as f32,
                unit: "MB/s",
            },
            NetworkUnit::Gb => Self {
                value: (bytes / 1_000_000_000.0) as f32,
                unit: "GB/s",
            },
            NetworkUnit::Auto => {
                if bytes >= 1_000_000_000.0 {
                    Self {
                        value: (bytes / 1_000_000_000.0) as f32,
                        unit: "GB/s",
                    }
                } else if bytes >= 1_000_000.0 {
                    Self {
                        value: (bytes / 1_000_000.0) as f32,
                        unit: "MB/s",
                    }
                } else if bytes >= 1_000.0 {
                    Self {
                        value: (bytes / 1_000.0) as f32,
                        unit: "KB/s",
                    }
                } else {
                    Self {
                        value: bytes as f32,
                        unit: "B/s",
                    }
                }
            }
        }
    }

    /// Render as `"12.3 MB/s"`.
    #[must_use]
    pub fn render(&self) -> String {
        if self.value >= 100.0 || self.value.fract().abs() < 0.05 {
            format!("{:.0} {}", self.value, self.unit)
        } else {
            format!("{:.1} {}", self.value, self.unit)
        }
    }
}

/// Render a duration as a compact human string.
///
/// Used by the battery and pomodoro widgets: `"1h 05m"`, `"45m"`, `"40s"`.
/// Values above 99 hours are capped, which is the point where a time estimate
/// is guessing rather than measuring.
#[must_use]
pub fn format_short_duration(total_secs: u64) -> String {
    let secs = total_secs.min(99 * 3600);
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// Render a countdown as `MM:SS` or `H:MM:SS`.
#[must_use]
pub fn format_countdown(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// The weather condition, reduced to the families Lumina can draw.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WeatherCondition {
    /// Clear sky, day or night.
    #[default]
    Clear,
    /// A few clouds.
    PartlyCloudy,
    /// Overcast.
    Cloudy,
    /// Fog or depositing rime.
    Fog,
    /// Drizzle in any intensity.
    Drizzle,
    /// Rain in any intensity.
    Rain,
    /// Snow in any intensity.
    Snow,
    /// Thunderstorm.
    Thunder,
    /// A code the provider added after this build was written.
    Unknown,
}

impl WeatherCondition {
    /// Map a WMO weather interpretation code to a condition family.
    ///
    /// The full table is in the Open-Meteo documentation. Unknown codes map to
    /// [`WeatherCondition::Unknown`] and render as a neutral cloud.
    #[must_use]
    pub fn from_wmo(code: u8) -> Self {
        match code {
            0 => Self::Clear,
            1 | 2 => Self::PartlyCloudy,
            3 => Self::Cloudy,
            45 | 48 => Self::Fog,
            51..=57 => Self::Drizzle,
            61..=67 | 80..=82 => Self::Rain,
            71..=77 | 85 | 86 => Self::Snow,
            95..=99 => Self::Thunder,
            _ => Self::Unknown,
        }
    }

    /// A short human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clear => "Clear",
            Self::PartlyCloudy => "Partly cloudy",
            Self::Cloudy => "Cloudy",
            Self::Fog => "Fog",
            Self::Drizzle => "Drizzle",
            Self::Rain => "Rain",
            Self::Snow => "Snow",
            Self::Thunder => "Thunder",
            Self::Unknown => "Weather",
        }
    }
}

/// One weather reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherSnapshot {
    /// Temperature in Celsius.
    pub temperature_c: f32,
    /// Apparent temperature in Celsius, when the provider reports it.
    pub feels_like_c: Option<f32>,
    /// Relative humidity in percent.
    pub humidity_percent: Option<u8>,
    /// Wind speed in kilometres per hour.
    pub wind_kph: f32,
    /// Condition family.
    pub condition: WeatherCondition,
    /// Whether it is daytime at the configured location.
    pub is_day: bool,
}

impl WeatherSnapshot {
    /// Temperature expressed in the configured unit.
    #[must_use]
    pub fn temperature(&self, imperial: bool) -> f32 {
        if imperial {
            self.temperature_c * 9.0 / 5.0 + 32.0
        } else {
            self.temperature_c
        }
    }

    /// Wind speed expressed in the configured unit.
    #[must_use]
    pub fn wind(&self, imperial: bool) -> f32 {
        if imperial {
            self.wind_kph * 0.621_371
        } else {
            self.wind_kph
        }
    }

    /// The unit suffix for temperature.
    #[must_use]
    pub const fn temperature_unit(imperial: bool) -> &'static str {
        if imperial {
            "°F"
        } else {
            "°C"
        }
    }
}

/// Time based cache for a weather reading.
///
/// The cache keeps one reading and its fetch time. [`WeatherCache::fresh`]
/// returns the reading only while it is inside the TTL; [`WeatherCache::stale`]
/// returns it regardless, so the widget can keep showing the last good reading
/// while a refresh is in flight instead of flashing an empty card.
#[derive(Debug, Clone)]
pub struct WeatherCache {
    snapshot: Option<WeatherSnapshot>,
    fetched_at: Option<Instant>,
    ttl: Duration,
}

impl WeatherCache {
    /// Create a cache with the given time to live.
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            snapshot: None,
            fetched_at: None,
            ttl,
        }
    }

    /// Store a reading, stamping it with `now`.
    pub fn store(&mut self, snapshot: WeatherSnapshot, now: Instant) {
        self.snapshot = Some(snapshot);
        self.fetched_at = Some(now);
    }

    /// Discard the reading.
    pub fn clear(&mut self) {
        self.snapshot = None;
        self.fetched_at = None;
    }

    /// The stored reading regardless of age.
    #[must_use]
    pub fn latest(&self) -> Option<&WeatherSnapshot> {
        self.snapshot.as_ref()
    }

    /// The reading when it is still inside its TTL.
    #[must_use]
    pub fn fresh(&self, now: Instant) -> Option<&WeatherSnapshot> {
        let fetched_at = self.fetched_at?;
        if now.saturating_duration_since(fetched_at) < self.ttl {
            self.snapshot.as_ref()
        } else {
            None
        }
    }

    /// The reading even when it is past its TTL, plus whether it is stale.
    #[must_use]
    pub fn stale(&self, now: Instant) -> Option<(&WeatherSnapshot, bool)> {
        let snapshot = self.snapshot.as_ref()?;
        let fetched_at = self.fetched_at?;
        Some((
            snapshot,
            now.saturating_duration_since(fetched_at) >= self.ttl,
        ))
    }

    /// True when a refresh should be requested at `now`.
    #[must_use]
    pub fn needs_refresh(&self, now: Instant) -> bool {
        self.fresh(now).is_none()
    }

    /// The configured time to live.
    #[must_use]
    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_are_clamped_and_non_finite_values_are_dropped() {
        let snapshot = MetricsSnapshot {
            cpu_percent: Some(150.0),
            ram_percent: Some(f32::NAN),
            gpu_percent: Some(-4.0),
            upload_bps: 0,
            download_bps: 0,
        }
        .sanitized();
        assert_eq!(snapshot.cpu_percent, Some(100.0));
        assert_eq!(snapshot.ram_percent, None);
        assert_eq!(snapshot.gpu_percent, Some(0.0));
    }

    #[test]
    fn peak_percent_ignores_missing_readings() {
        let snapshot = MetricsSnapshot {
            cpu_percent: Some(10.0),
            gpu_percent: Some(40.0),
            ..MetricsSnapshot::default()
        };
        assert_eq!(snapshot.peak_percent(), Some(40.0));
        assert_eq!(MetricsSnapshot::default().peak_percent(), None);
    }

    #[test]
    fn history_drops_the_oldest_sample_when_full() {
        let mut history = History::new(3);
        history.push(10.0);
        history.push(20.0);
        history.push(30.0);
        history.push(40.0);
        assert_eq!(history.len(), 3);
        assert_eq!(history.values().iter().copied().collect::<Vec<_>>(), vec![20.0, 30.0, 40.0]);
        assert_eq!(history.latest(), Some(40.0));
    }

    #[test]
    fn history_ignores_non_finite_samples() {
        let mut history = History::new(2);
        history.push(f32::INFINITY);
        history.push(f32::NAN);
        assert!(history.is_empty());
    }

    #[test]
    fn history_clamps_capacity_to_a_sane_range() {
        assert_eq!(History::new(0).capacity(), 1);
        assert_eq!(History::new(usize::MAX).capacity(), MAX_HISTORY);
    }

    #[test]
    fn history_normalizes_against_full_scale() {
        let mut history = History::new(2);
        history.push(50.0);
        history.push(100.0);
        let normalized: Vec<f32> = history.normalized().collect();
        assert!((normalized[0] - 0.5).abs() < 1e-6);
        assert!((normalized[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn battery_fraction_and_usability() {
        let battery = BatterySnapshot {
            present: true,
            percent: 50.0,
            charging: false,
            time_remaining_secs: Some(3600),
        };
        assert!(battery.is_usable());
        assert!((battery.fraction() - 0.5).abs() < 1e-6);
        assert!(!BatterySnapshot::default().is_usable());
    }

    #[test]
    fn battery_time_estimates_of_zero_are_discarded() {
        let battery = BatterySnapshot {
            present: true,
            percent: 40.0,
            charging: true,
            time_remaining_secs: Some(0),
        }
        .sanitized();
        assert_eq!(battery.time_remaining_secs, None);
    }

    #[test]
    fn rates_scale_automatically() {
        assert_eq!(FormattedRate::new(500, NetworkUnit::Auto).render(), "500 B/s");
        assert_eq!(FormattedRate::new(1500, NetworkUnit::Auto).render(), "1.5 KB/s");
        assert_eq!(FormattedRate::new(2_500_000, NetworkUnit::Auto).render(), "2.5 MB/s");
        assert_eq!(FormattedRate::new(2_500_000_000, NetworkUnit::Auto).render(), "2.5 GB/s");
    }

    #[test]
    fn rates_honour_a_fixed_unit() {
        assert_eq!(FormattedRate::new(2_500_000, NetworkUnit::Mb).render(), "2.5 MB/s");
        assert_eq!(FormattedRate::new(2_500_000, NetworkUnit::Kb).render(), "2500 KB/s");
    }

    #[test]
    fn durations_render_compactly() {
        assert_eq!(format_short_duration(30), "30s");
        assert_eq!(format_short_duration(90), "1m");
        assert_eq!(format_short_duration(3660), "1h 01m");
        assert_eq!(format_short_duration(u64::MAX), "99h 00m");
    }

    #[test]
    fn countdowns_render_as_clock_faces() {
        assert_eq!(format_countdown(59), "00:59");
        assert_eq!(format_countdown(25 * 60), "25:00");
        assert_eq!(format_countdown(3661), "1:01:01");
    }

    #[test]
    fn wmo_codes_map_to_condition_families() {
        assert_eq!(WeatherCondition::from_wmo(0), WeatherCondition::Clear);
        assert_eq!(WeatherCondition::from_wmo(2), WeatherCondition::PartlyCloudy);
        assert_eq!(WeatherCondition::from_wmo(45), WeatherCondition::Fog);
        assert_eq!(WeatherCondition::from_wmo(55), WeatherCondition::Drizzle);
        assert_eq!(WeatherCondition::from_wmo(65), WeatherCondition::Rain);
        assert_eq!(WeatherCondition::from_wmo(75), WeatherCondition::Snow);
        assert_eq!(WeatherCondition::from_wmo(96), WeatherCondition::Thunder);
        assert_eq!(WeatherCondition::from_wmo(200), WeatherCondition::Unknown);
    }

    #[test]
    fn weather_cache_expires_on_ttl() {
        let now = Instant::now();
        let mut cache = WeatherCache::new(Duration::from_secs(60));
        assert!(cache.needs_refresh(now));
        assert!(cache.fresh(now).is_none());

        let snapshot = WeatherSnapshot {
            temperature_c: 21.0,
            feels_like_c: Some(20.0),
            humidity_percent: Some(50),
            wind_kph: 10.0,
            condition: WeatherCondition::Clear,
            is_day: true,
        };
        cache.store(snapshot, now);
        assert!(!cache.needs_refresh(now));
        assert_eq!(cache.fresh(now), Some(&snapshot));

        let later = now + Duration::from_secs(61);
        assert!(cache.fresh(later).is_none());
        let (stale, is_stale) = cache.stale(later).expect("stale reading is retained");
        assert_eq!(stale, &snapshot);
        assert!(is_stale);
    }

    #[test]
    fn weather_cache_can_be_cleared() {
        let now = Instant::now();
        let mut cache = WeatherCache::new(Duration::from_secs(60));
        cache.store(
            WeatherSnapshot {
                temperature_c: 1.0,
                feels_like_c: None,
                humidity_percent: None,
                wind_kph: 0.0,
                condition: WeatherCondition::Snow,
                is_day: true,
            },
            now,
        );
        cache.clear();
        assert!(cache.fresh(now).is_none());
    }

    #[test]
    fn weather_temperature_units_convert_both_ways() {
        let snapshot = WeatherSnapshot {
            temperature_c: 0.0,
            feels_like_c: None,
            humidity_percent: None,
            wind_kph: 100.0,
            condition: WeatherCondition::Clear,
            is_day: true,
        };
        assert!((snapshot.temperature(true) - 32.0).abs() < 1e-4);
        assert!((snapshot.wind(true) - 62.1371).abs() < 1e-3);
        assert_eq!(WeatherSnapshot::temperature_unit(true), "°F");
        assert_eq!(WeatherSnapshot::temperature_unit(false), "°C");
    }
}
