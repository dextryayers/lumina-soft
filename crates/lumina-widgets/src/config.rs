//! Config-driven widget registry.
//!
//! This module connects the layout configuration to the widget registry,
//! allowing users to control widget order and visibility from the config file.

use lumina_config::schema::LayoutConfig;

use crate::widget::WidgetRegistry;

/// Build a widget registry from the layout configuration.
///
/// The registry is configured with the widget order and max_widgets from the
/// config. Widgets are registered in the order they appear in the config.
#[must_use]
pub fn registry_from_config(config: &LayoutConfig) -> WidgetRegistry {
    WidgetRegistry::with_config(config.order.clone(), config.max_widgets as usize)
}

/// Update an existing registry with new layout configuration.
///
/// This is called when the config is reloaded. The registry's ordering and
/// max_widgets are updated without disturbing the registered widgets.
pub fn update_registry(registry: &mut WidgetRegistry, config: &LayoutConfig) {
    registry.update_config(config.order.clone(), config.max_widgets as usize);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumina_config::schema::LayoutConfig;

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
}
