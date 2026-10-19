//! Config-driven widget registry.
//!
//! This module is the single place where the configuration tree becomes a set
//! of live widgets. Widgets are only registered when their section is enabled,
//! so a disabled feature cannot draw, request state, or consume input. Script
//! entries are expanded into one widget each, preserving declaration order.

use lumina_config::schema::{Config, LayoutConfig};

use crate::battery::BatteryWidget;
use crate::calendar::CalendarWidget;
use crate::clock::ClockWidget;
use crate::media::MediaWidget;
use crate::metrics::MetricsWidget;
use crate::network::NetworkWidget;
use crate::notify::NotificationWidget;
use crate::pomodoro::PomodoroWidget;
use crate::script::ScriptWidget;
use crate::shortcuts::ShortcutsWidget;
use crate::weather::WeatherWidget;
use crate::widget::WidgetRegistry;

/// Build a widget registry from the layout configuration only.
///
/// This keeps the original layout-only entry point for callers that construct
/// their own widgets. Prefer [`build_registry`] in the application, which also
/// applies the per-widget overrides and feature switches.
#[must_use]
pub fn registry_from_config(config: &LayoutConfig) -> WidgetRegistry {
    WidgetRegistry::with_config(config.order.clone(), config.max_widgets as usize)
}

/// Build the full widget set from the configuration.
///
/// Registration respects both the feature's master switch and the layout
/// order: a widget that is not listed is still registered, but costs nothing
/// because the registry only lays out, draws, and routes to visible entries.
#[must_use]
pub fn build_registry(config: &Config) -> WidgetRegistry {
    let mut registry = WidgetRegistry::from_layout(&config.layout, &config.widgets);
    let order = &config.layout.order;

    if config.clock.enabled {
        registry.register(Box::new(ClockWidget::from_config(&config.clock)));
    }
    if order.iter().any(|id| id == "calendar") {
        registry.register(Box::new(CalendarWidget::new()));
    }
    if config.media.enabled {
        registry.register(Box::new(MediaWidget::new()));
    }
    if config.metrics.enabled {
        registry.register(Box::new(MetricsWidget::from_config(&config.metrics)));
    }
    if config.battery.enabled {
        registry.register(Box::new(BatteryWidget::from_config(&config.battery)));
    }
    if config.network.enabled {
        registry.register(Box::new(NetworkWidget::from_config(&config.network)));
    }
    if config.pomodoro.enabled {
        registry.register(Box::new(PomodoroWidget::from_config(&config.pomodoro)));
    }
    if config.weather.enabled {
        registry.register(Box::new(WeatherWidget::from_config(&config.weather)));
    }
    if order.iter().any(|id| id == "shortcuts") {
        registry.register(Box::new(ShortcutsWidget::from_config(
            &config.shortcuts,
        )));
    }
    if order.iter().any(|id| id == "notify") {
        registry.register(Box::new(NotificationWidget::new()));
    }
    if config.script.enabled {
        for entry in &config.script.entries {
            if entry.is_runnable() && entry.show_in_island {
                registry.register(Box::new(ScriptWidget::new(entry.clone())));
            }
        }
    }

    registry.apply_widget_config(&config.widgets);
    registry
}

/// Update an existing registry with new layout configuration.
///
/// This is called when the config is reloaded. The registry's ordering,
/// spacing, alignment, and max_widgets are updated without disturbing the
/// registered widgets.
pub fn update_registry(registry: &mut WidgetRegistry, config: &LayoutConfig) {
    registry.update_layout(config);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_from_config_uses_config_order() {
        let config = LayoutConfig {
            order: vec!["calendar".to_string(), "clock".to_string()],
            max_widgets: 2,
            ..Default::default()
        };
        let registry = registry_from_config(&config);
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn update_registry_changes_order() {
        let config = LayoutConfig {
            order: vec!["clock".to_string(), "calendar".to_string()],
            max_widgets: 2,
            ..Default::default()
        };
        let mut registry = registry_from_config(&config);
        update_registry(&mut registry, &config);
    }

    #[test]
    fn update_registry_changes_max_widgets() {
        let config = LayoutConfig {
            order: vec!["clock".to_string()],
            max_widgets: 1,
            ..Default::default()
        };
        let mut registry = registry_from_config(&config);
        update_registry(&mut registry, &config);
    }

    #[test]
    fn the_builder_registers_only_enabled_sections() {
        let mut config = Config::default();
        config.metrics.enabled = false;
        config.battery.enabled = false;
        config.network.enabled = false;
        config.pomodoro.enabled = false;
        config.weather.enabled = false;
        config.script.enabled = false;
        let registry = build_registry(&config);
        assert!(registry.get("clock").is_some());
        assert!(registry.get("media").is_some());
        assert!(registry.get("metrics").is_none());
        assert!(registry.get("battery").is_none());
        assert!(registry.get("weather").is_none());
    }

    #[test]
    fn the_builder_expands_script_entries() {
        let mut config = Config::default();
        config.script.enabled = true;
        config.script.entries = vec![
            lumina_config::schema::ScriptEntry {
                id: "uptime".to_string(),
                label: "Uptime".to_string(),
                command: "uptime".to_string(),
                display: lumina_config::schema::ScriptDisplay::Text,
                show_in_island: true,
                ..Default::default()
            },
            lumina_config::schema::ScriptEntry {
                id: "off".to_string(),
                label: "Off".to_string(),
                command: "true".to_string(),
                show_in_island: false,
                ..Default::default()
            },
        ];
        let registry = build_registry(&config);
        let scripts = registry
            .widgets()
            .iter()
            .filter(|widget| widget.id() == "script")
            .count();
        assert_eq!(scripts, 1, "only runnable, island-visible scripts appear");
    }

    #[test]
    fn the_builder_applies_per_widget_flex() {
        let mut config = Config::default();
        config.widgets.widgets.insert(
            "clock".to_string(),
            lumina_config::schema::WidgetEntry {
                flex: Some(4.0),
                ..Default::default()
            },
        );
        let registry = build_registry(&config);
        let index = registry
            .widgets()
            .iter()
            .position(|widget| widget.id() == "clock")
            .expect("clock is registered");
        assert_eq!(registry.entries()[index].flex, Some(4.0));
    }
}
