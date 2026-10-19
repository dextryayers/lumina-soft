//! The weather widget: current conditions from Open-Meteo or OpenWeatherMap.
//!
//! # Networking boundary
//!
//! The widget itself never opens a socket. [`WeatherWidget::refresh`] takes a
//! [`WeatherTransport`], builds the provider URL, parses the response, and
//! stores it in the cache. The orchestrator supplies the transport, which keeps
//! the widget testable and lets offline builds opt out of TLS entirely. The
//! `http` crate feature provides [`UreqTransport`] for production builds.
//!
//! # Caching and failures
//!
//! A reading stays fresh for `cache_ttl_minutes`. Past that, the last reading is
//! still rendered with a staleness dot while a refresh is pending, so a flaky
//! network never blanks the card. Failures are recorded for logging and retried
//! no sooner than a minute, with the stale reading visible in the meantime.

use std::fmt;
use std::time::{Duration, Instant};

use lumina_config::schema::{WeatherConfig, WeatherProvider, WeatherUnits};
use lumina_core::geometry::{Rect, Size};
use lumina_core::state::{IslandState, StateRequest};

use crate::paint::{self, ACCENT, DIM, MUTED, TEXT, TRACK, WARN};
use crate::snapshot::{WeatherCache, WeatherCondition, WeatherSnapshot};
use crate::widget::{DrawList, Widget, WidgetContext, WidgetInput};

/// Shortest retry after a failed refresh.
const RETRY_FLOOR: Duration = Duration::from_secs(60);
/// Timeout for a weather request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// A weather failure, suitable for a warn level log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeatherError {
    /// The configuration cannot produce a valid request.
    InvalidConfig(String),
    /// The transport failed.
    Http(String),
    /// The response was not the documented shape.
    Parse(String),
}

impl fmt::Display for WeatherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) | Self::Http(message) | Self::Parse(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for WeatherError {}

/// A blocking HTTP GET, injected so the widget owns no transport.
pub trait WeatherTransport: Send {
    /// Fetch `url` and return the body as text.
    fn get(&self, url: &str, timeout: Duration) -> Result<String, String>;
}

/// A [`WeatherTransport`] backed by `ureq`.
///
/// Available with the `http` crate feature. The agent is built per request,
/// which is cheap relative to the network round trip and avoids shared state
/// between the UI thread and the fetch thread.
#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy, Default)]
pub struct UreqTransport;

#[cfg(feature = "http")]
impl WeatherTransport for UreqTransport {
    fn get(&self, url: &str, timeout: Duration) -> Result<String, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(timeout)
            .timeout_read(timeout)
            .build();
        agent
            .get(url)
            .call()
            .map_err(|error| error.to_string())?
            .into_string()
            .map_err(|error| error.to_string())
    }
}

/// The weather widget.
#[derive(Debug)]
pub struct WeatherWidget {
    config: WeatherConfig,
    cache: WeatherCache,
    last_error: Option<String>,
    next_attempt: Option<Instant>,
}

impl WeatherWidget {
    /// Build the widget from configuration.
    #[must_use]
    pub fn from_config(config: &WeatherConfig) -> Self {
        let ttl = Duration::from_secs(u64::from(config.cache_ttl_minutes.max(1)) * 60);
        Self {
            config: config.clone(),
            cache: WeatherCache::new(ttl),
            last_error: None,
            next_attempt: None,
        }
    }

    /// The configuration in use.
    #[must_use]
    pub fn config(&self) -> &WeatherConfig {
        &self.config
    }

    /// True when the widget should ask for a refresh at `now`.
    #[must_use]
    pub fn needs_refresh(&self, now: Instant) -> bool {
        if self.config.provider == WeatherProvider::OpenWeatherMap
            && self.config.api_key.trim().is_empty()
        {
            return false;
        }
        let retry_ready = self.next_attempt.is_none_or(|next| now >= next);
        retry_ready && self.cache.needs_refresh(now)
    }

    /// The cached reading and whether it is past its TTL.
    #[must_use]
    pub fn reading(&self, now: Instant) -> Option<(&WeatherSnapshot, bool)> {
        self.cache.stale(now)
    }

    /// The last refresh error, if the most recent attempt failed.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Fetch and store a reading.
    pub fn refresh<T: WeatherTransport + ?Sized>(
        &mut self,
        transport: &T,
        now: Instant,
    ) -> Result<WeatherSnapshot, WeatherError> {
        let url = request_url(&self.config)?;
        let body = transport
            .get(&url, REQUEST_TIMEOUT)
            .map_err(WeatherError::Http)?;
        let snapshot = match self.config.provider {
            WeatherProvider::OpenMeteo => parse_open_meteo(&body)?,
            WeatherProvider::OpenWeatherMap => parse_open_weather_map(&body)?,
        };
        self.cache.store(snapshot, now);
        self.last_error = None;
        let retry = Duration::from_millis(u64::from(self.config.interval_ms.max(60_000)));
        self.next_attempt = Some(now + retry.max(RETRY_FLOOR));
        Ok(snapshot)
    }

    /// Record a failed refresh so the next attempt waits at least a minute.
    pub fn record_failure(&mut self, error: &WeatherError, now: Instant) {
        self.last_error = Some(error.to_string());
        self.next_attempt = Some(now + RETRY_FLOOR);
    }

/// The temperature rendered with the configured unit, for example `"21°C"`.
    #[must_use]
    pub fn temperature_text(&self, now: Instant) -> Option<String> {
        let (snapshot, _) = self.cache.stale(now)?;
        let imperial = self.config.units == WeatherUnits::Imperial;
        Some(format!(
            "{:.0}{}",
            snapshot.temperature(imperial),
            WeatherSnapshot::temperature_unit(imperial)
        ))
    }

    /// The condition label of the stored reading.
    #[must_use]
    pub fn condition_label(&self) -> Option<&'static str> {
        Some(self.cache.latest()?.condition.label())
    }

    fn draw_icon(&self, out: &mut DrawList, icon: Rect, snapshot: &WeatherSnapshot, stale: bool) {
        let color = if stale {
            [1.0, 1.0, 1.0, 0.45]
        } else {
            match snapshot.condition {
                WeatherCondition::Clear if snapshot.is_day => WARN,
                WeatherCondition::Clear => DIM,
                WeatherCondition::Thunder => WARN,
                _ => ACCENT,
            }
        };
        match snapshot.condition {
            WeatherCondition::Clear => {
                let dot = icon.width.min(icon.height) * 0.55;
                paint::block(
                    out,
                    Rect::new(icon.x + (icon.width - dot) * 0.5, icon.y + (icon.height - dot) * 0.5, dot, dot),
                    color,
                    dot * 0.5,
                );
            }
            WeatherCondition::PartlyCloudy | WeatherCondition::Cloudy | WeatherCondition::Unknown => {
                paint::block(
                    out,
                    Rect::new(icon.x + 1.0, icon.y + 5.0, icon.width - 2.0, icon.height - 7.0),
                    color,
                    (icon.height - 7.0) * 0.5,
                );
                if snapshot.condition == WeatherCondition::PartlyCloudy {
                    paint::block(
                        out,
                        Rect::new(icon.x + 2.0, icon.y + 1.0, 7.0, 7.0),
                        WARN,
                        3.5,
                    );
                }
            }
            WeatherCondition::Fog => {
                for row in 0..3 {
                    paint::block(
                        out,
                        Rect::new(icon.x + row as f32, icon.y + 4.0 + row as f32 * 5.0, icon.width - 2.0 * row as f32, 2.0),
                        color,
                        1.0,
                    );
                }
            }
            WeatherCondition::Drizzle | WeatherCondition::Rain => {
                paint::block(
                    out,
                    Rect::new(icon.x + 1.0, icon.y + 2.0, icon.width - 2.0, icon.height * 0.55),
                    color,
                    icon.height * 0.3,
                );
                for drop in 0..3 {
                    paint::block(
                        out,
                        Rect::new(icon.x + 3.0 + drop as f32 * 5.0, icon.y + icon.height - 5.0, 2.0, 4.0),
                        ACCENT,
                        1.0,
                    );
                }
            }
            WeatherCondition::Snow => {
                for flake in 0..4 {
                    let x = icon.x + (flake % 2) as f32 * (icon.width * 0.5);
                    let y = icon.y + (flake / 2) as f32 * (icon.height * 0.4) + 1.0;
                    paint::block(out, Rect::new(x + 1.0, y, 4.0, 4.0), TEXT, 2.0);
                }
            }
            WeatherCondition::Thunder => {
                paint::block(
                    out,
                    Rect::new(icon.x + 2.0, icon.y + 1.0, icon.width - 4.0, icon.height * 0.5),
                    color,
                    icon.height * 0.25,
                );
                paint::block(
                    out,
                    Rect::new(icon.x + icon.width * 0.45, icon.y + icon.height * 0.45, 4.0, icon.height * 0.5),
                    WARN,
                    1.0,
                );
            }
        }
    }
}

impl Widget for WeatherWidget {
    fn id(&self) -> &'static str {
        "weather"
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        Size::new(
            if self.config.units == WeatherUnits::Imperial {
                132.0
            } else {
                124.0
            },
            32.0,
        )
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        let now = Instant::now();
        let Some((snapshot, stale)) = self.cache.stale(now) else {
            paint::block(out, rect, TRACK, 8.0);
            paint::block(
                out,
                Rect::new(rect.x + 8.0, rect.y + rect.height * 0.5 - 2.0, 18.0, 4.0),
                MUTED,
                2.0,
            );
            return;
        };
        let snapshot = *snapshot;

        let icon = Rect::new(rect.x + 4.0, rect.y + (rect.height - 20.0) * 0.5, 20.0, 20.0);
        self.draw_icon(out, icon, &snapshot, stale);

        let text_x = icon.right() + 8.0;
        paint::label(
            out,
            Rect::new(text_x, rect.y + 6.0, (rect.right() - text_x - 4.0).max(1.0), 10.0),
            if stale { MUTED } else { TEXT },
        );
        paint::label(
            out,
            Rect::new(text_x, rect.y + 19.0, (rect.right() - text_x - 4.0).max(1.0), 6.0),
            DIM,
        );
        if stale {
            paint::block(
                out,
                Rect::new(rect.right() - 6.0, rect.y + 4.0, 3.0, 3.0),
                WARN,
                1.5,
            );
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

/// Build the provider URL for a configuration.
pub fn request_url(config: &WeatherConfig) -> Result<String, WeatherError> {
    if !config.latitude.is_finite() || !(-90.0..=90.0).contains(&config.latitude) {
        return Err(WeatherError::InvalidConfig(
            "latitude must be between -90 and 90".to_string(),
        ));
    }
    if !config.longitude.is_finite() || !(-180.0..=180.0).contains(&config.longitude) {
        return Err(WeatherError::InvalidConfig(
            "longitude must be between -180 and 180".to_string(),
        ));
    }
    let imperial = config.units == WeatherUnits::Imperial;
    match config.provider {
        WeatherProvider::OpenMeteo => {
            let mut url = format!(
                "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}&current=temperature_2m,apparent_temperature,relative_humidity_2m,wind_speed_10m,weather_code,is_day&timezone=auto",
                config.latitude, config.longitude
            );
            if imperial {
                url.push_str("&temperature_unit=fahrenheit&wind_speed_unit=mph");
            }
            Ok(url)
        }
        WeatherProvider::OpenWeatherMap => {
            if config.api_key.trim().is_empty() {
                return Err(WeatherError::InvalidConfig(
                    "openweathermap requires an api_key".to_string(),
                ));
            }
            Ok(format!(
                "https://api.openweathermap.org/data/2.5/weather?lat={:.4}&lon={:.4}&units=metric&appid={}",
                config.latitude,
                config.longitude,
                config.api_key.trim()
            ))
        }
    }
}

fn number(value: &serde_json::Value, key: &str) -> Result<f64, WeatherError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| WeatherError::Parse(format!("missing numeric field `{key}`")))
}

/// Parse an Open-Meteo `current` response.
pub fn parse_open_meteo(body: &str) -> Result<WeatherSnapshot, WeatherError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| WeatherError::Parse(error.to_string()))?;
    let current = value
        .get("current")
        .ok_or_else(|| WeatherError::Parse("missing `current` object".to_string()))?;
    let temperature_c = number(current, "temperature_2m")?;
    let feels_like_c = current
        .get("apparent_temperature")
        .and_then(serde_json::Value::as_f64)
        .map(|v| v as f32);
    let humidity = current
        .get("relative_humidity_2m")
        .and_then(serde_json::Value::as_f64)
        .map(|v| v.clamp(0.0, 100.0) as u8);
    let wind_kph = current
        .get("wind_speed_10m")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0) as f32;
    let code = current
        .get("weather_code")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u8;
    let is_day = current
        .get("is_day")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1)
        != 0;
    Ok(WeatherSnapshot {
        temperature_c: temperature_c as f32,
        feels_like_c,
        humidity_percent: humidity,
        wind_kph,
        condition: WeatherCondition::from_wmo(code),
        is_day,
    })
}

/// Parse an OpenWeatherMap current weather response.
pub fn parse_open_weather_map(body: &str) -> Result<WeatherSnapshot, WeatherError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| WeatherError::Parse(error.to_string()))?;
    let main = value
        .get("main")
        .ok_or_else(|| WeatherError::Parse("missing `main` object".to_string()))?;
    let temperature_c = number(main, "temp")?;
    let feels_like_c = main
        .get("feels_like")
        .and_then(serde_json::Value::as_f64)
        .map(|v| v as f32);
    let humidity = main
        .get("humidity")
        .and_then(serde_json::Value::as_f64)
        .map(|v| v.clamp(0.0, 100.0) as u8);
    let wind_kph = value
        .get("wind")
        .and_then(|wind| wind.get("speed"))
        .and_then(serde_json::Value::as_f64)
        .map_or(0.0, |speed_mps| (speed_mps * 3.6) as f32);
    let code = value
        .get("weather")
        .and_then(serde_json::Value::as_array)
        .and_then(|weather| weather.first())
        .and_then(|entry| entry.get("id"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(800);
    let is_day = value
        .get("dt")
        .and_then(serde_json::Value::as_i64)
        .zip(value.get("sys"))
        .and_then(|(dt, sys)| {
            let sunrise = sys.get("sunrise").and_then(serde_json::Value::as_i64)?;
            let sunset = sys.get("sunset").and_then(serde_json::Value::as_i64)?;
            Some(dt >= sunrise && dt < sunset)
        })
        .unwrap_or(true);
    Ok(WeatherSnapshot {
        temperature_c: temperature_c as f32,
        feels_like_c,
        humidity_percent: humidity,
        wind_kph,
        condition: condition_from_owm(code as u16),
        is_day,
    })
}

fn condition_from_owm(code: u16) -> WeatherCondition {
    match code {
        200..=232 => WeatherCondition::Thunder,
        300..=321 => WeatherCondition::Drizzle,
        500..=531 => WeatherCondition::Rain,
        600..=622 => WeatherCondition::Snow,
        701 | 741 => WeatherCondition::Fog,
        711 | 721 | 731 | 751 | 761 | 762 | 771 | 781 => WeatherCondition::Fog,
        800 => WeatherCondition::Clear,
        801 | 802 => WeatherCondition::PartlyCloudy,
        803 | 804 => WeatherCondition::Cloudy,
        _ => WeatherCondition::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> WeatherConfig {
        WeatherConfig {
            enabled: true,
            latitude: 51.5074,
            longitude: -0.1278,
            ..WeatherConfig::default()
        }
    }

    fn ctx() -> WidgetContext {
        WidgetContext::new(IslandState::Metrics, Size::new(320.0, 80.0))
    }

    struct FixedTransport(&'static str);

    impl WeatherTransport for FixedTransport {
        fn get(&self, _url: &str, _timeout: Duration) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    const OPEN_METEO: &str = r#"{"current":{"temperature_2m":18.4,"apparent_temperature":17.1,"relative_humidity_2m":64,"wind_speed_10m":12.6,"weather_code":2,"is_day":1}}"#;

    #[test]
    fn open_meteo_url_contains_the_location_and_current_fields() {
        let url = request_url(&config()).expect("valid");
        assert!(url.starts_with("https://api.open-meteo.com/v1/forecast?"));
        assert!(url.contains("latitude=51.5074"));
        assert!(url.contains("longitude=-0.1278"));
        assert!(url.contains("weather_code"));
        assert!(!url.contains("temperature_unit"));
    }

    #[test]
    fn imperial_requests_ask_open_meteo_for_fahrenheit() {
        let url = request_url(&WeatherConfig {
            units: WeatherUnits::Imperial,
            ..config()
        })
        .expect("valid");
        assert!(url.contains("temperature_unit=fahrenheit"));
    }

    #[test]
    fn invalid_coordinates_are_rejected() {
        let error = request_url(&WeatherConfig {
            latitude: 120.0,
            ..config()
        })
        .expect_err("latitude out of range");
        assert!(matches!(error, WeatherError::InvalidConfig(_)));

        let error = request_url(&WeatherConfig {
            longitude: f64::NAN,
            ..config()
        })
        .expect_err("longitude is not finite");
        assert!(matches!(error, WeatherError::InvalidConfig(_)));
    }

    #[test]
    fn open_weather_map_requires_a_key() {
        let error = request_url(&WeatherConfig {
            provider: WeatherProvider::OpenWeatherMap,
            api_key: String::new(),
            ..config()
        })
        .expect_err("no key");
        assert!(matches!(error, WeatherError::InvalidConfig(_)));
    }

    #[test]
    fn open_meteo_response_is_parsed() {
        let snapshot = parse_open_meteo(OPEN_METEO).expect("parses");
        assert!((snapshot.temperature_c - 18.4).abs() < 1e-4);
        assert_eq!(snapshot.feels_like_c, Some(17.1));
        assert_eq!(snapshot.humidity_percent, Some(64));
        assert!((snapshot.wind_kph - 12.6).abs() < 1e-4);
        assert_eq!(snapshot.condition, WeatherCondition::PartlyCloudy);
        assert!(snapshot.is_day);
    }

    #[test]
    fn open_meteo_missing_current_is_an_error() {
        assert!(matches!(
            parse_open_meteo("{}"),
            Err(WeatherError::Parse(_))
        ));
        assert!(matches!(
            parse_open_meteo("not json"),
            Err(WeatherError::Parse(_))
        ));
    }

    #[test]
    fn open_weather_map_response_is_parsed() {
        let body = r#"{"dt":1700000000,"sys":{"sunrise":1699950000,"sunset":1700000000},"main":{"temp":12.5,"feels_like":11.0,"humidity":80},"wind":{"speed":5.0},"weather":[{"id":500}]}"#;
        let snapshot = parse_open_weather_map(body).expect("parses");
        assert!((snapshot.temperature_c - 12.5).abs() < 1e-4);
        assert_eq!(snapshot.humidity_percent, Some(80));
        assert!((snapshot.wind_kph - 18.0).abs() < 1e-4);
        assert_eq!(snapshot.condition, WeatherCondition::Rain);
        assert!(!snapshot.is_day);
    }

    #[test]
    fn owm_codes_map_to_families() {
        assert_eq!(condition_from_owm(800), WeatherCondition::Clear);
        assert_eq!(condition_from_owm(801), WeatherCondition::PartlyCloudy);
        assert_eq!(condition_from_owm(804), WeatherCondition::Cloudy);
        assert_eq!(condition_from_owm(211), WeatherCondition::Thunder);
        assert_eq!(condition_from_owm(601), WeatherCondition::Snow);
        assert_eq!(condition_from_owm(741), WeatherCondition::Fog);
        assert_eq!(condition_from_owm(999), WeatherCondition::Unknown);
    }

    #[test]
    fn refresh_stores_the_reading_and_sets_the_retry_floor() {
        let now = Instant::now();
        let mut widget = WeatherWidget::from_config(&config());
        let snapshot = widget
            .refresh(&FixedTransport(OPEN_METEO), now)
            .expect("fetch succeeds");
        assert!((snapshot.temperature_c - 18.4).abs() < 1e-4);
        assert!(!widget.needs_refresh(now));
        assert_eq!(widget.temperature_text(now).as_deref(), Some("18°C"));
        assert_eq!(widget.condition_label(), Some("Partly cloudy"));
        assert!(widget.reading(now).is_some_and(|(_, stale)| !stale));
    }

    #[test]
    fn a_transport_failure_is_recorded_and_rate_limited() {
        struct Failing;
        impl WeatherTransport for Failing {
            fn get(&self, _url: &str, _timeout: Duration) -> Result<String, String> {
                Err("offline".to_string())
            }
        }
        let now = Instant::now();
        let mut widget = WeatherWidget::from_config(&config());
        let error = widget
            .refresh(&Failing, now)
            .expect_err("transport fails");
        assert!(matches!(error, WeatherError::Http(_)));
        widget.record_failure(&error, now);
        assert!(!widget.needs_refresh(now));
        assert_eq!(widget.last_error(), Some("offline"));
        assert!(!widget.needs_refresh(now + Duration::from_secs(30)));
        assert!(widget.needs_refresh(now + Duration::from_secs(61)));
    }

    #[test]
    fn a_stale_reading_is_still_drawn() {
        let now = Instant::now();
        let mut widget = WeatherWidget::from_config(&WeatherConfig {
            cache_ttl_minutes: 1,
            interval_ms: 60_000,
            ..config()
        });
        widget
            .refresh(&FixedTransport(OPEN_METEO), now)
            .expect("fetch succeeds");
        let later = now + Duration::from_secs(120);
        assert!(widget.needs_refresh(later));
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 124.0, 32.0));
        assert!(!out.is_empty());
        assert!(widget.reading(later).is_some_and(|(_, stale)| stale));
    }

    #[test]
    fn no_reading_draws_a_placeholder() {
        let mut widget = WeatherWidget::from_config(&config());
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 124.0, 32.0));
        assert_eq!(out.len(), 2);
        assert_eq!(widget.temperature_text(Instant::now()), None);
    }

    #[test]
    fn every_condition_draws_something() {
        let now = Instant::now();
        let conditions = [
            WeatherCondition::Clear,
            WeatherCondition::PartlyCloudy,
            WeatherCondition::Cloudy,
            WeatherCondition::Fog,
            WeatherCondition::Drizzle,
            WeatherCondition::Rain,
            WeatherCondition::Snow,
            WeatherCondition::Thunder,
            WeatherCondition::Unknown,
        ];
        for condition in conditions {
            let mut widget = WeatherWidget::from_config(&config());
            widget.cache.store(
                WeatherSnapshot {
                    temperature_c: 10.0,
                    feels_like_c: None,
                    humidity_percent: None,
                    wind_kph: 5.0,
                    condition,
                    is_day: true,
                },
                now,
            );
            let mut out = DrawList::new();
            let ctx = ctx();
            widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 124.0, 32.0));
            assert!(!out.is_empty(), "{condition:?} drew nothing");
        }
    }

    #[test]
    fn imperial_display_converts_temperature() {
        let now = Instant::now();
        let mut widget = WeatherWidget::from_config(&WeatherConfig {
            units: WeatherUnits::Imperial,
            ..config()
        });
        widget
            .refresh(&FixedTransport(OPEN_METEO), now)
            .expect("fetch succeeds");
        assert_eq!(widget.temperature_text(now).as_deref(), Some("65°F"));
    }
}

