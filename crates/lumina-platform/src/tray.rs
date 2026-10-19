//! Tray icon raster generation and menu model.
//!
//! # Why the model is separate
//!
//! The tray menu is a data structure, not a pile of Win32 calls. Building it
//! here means the entries, their check marks, and their labels can be tested
//! without a desktop session, and the platform backend only has to translate a
//! flat list into its native menu. The icon is generated the same way: a pure
//! function from [`TrayState`] and a pixel size to RGBA, so every state is
//! deterministic and reviewable.

use lumina_config::schema::{Config, RgbMode};

/// What the tray icon should communicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayState {
    /// The island is hidden.
    #[default]
    Hidden,
    /// The island is visible and idle.
    Idle,
    /// Media is playing or a notification is up.
    Active,
    /// Audio is muted.
    Muted,
}

impl TrayState {
    /// All states, used by tests and by the icon preview in the docs.
    pub const ALL: [Self; 4] = [Self::Hidden, Self::Idle, Self::Active, Self::Muted];

    /// Stable identifier.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Idle => "idle",
            Self::Active => "active",
            Self::Muted => "muted",
        }
    }

    /// Human label used in the tray tooltip.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hidden => "Lumina (hidden)",
            Self::Idle => "Lumina",
            Self::Active => "Lumina (active)",
            Self::Muted => "Lumina (muted)",
        }
    }

    /// Base color as straight RGBA bytes.
    #[must_use]
    pub const fn color(self) -> [u8; 4] {
        match self {
            Self::Hidden => [148, 163, 184, 255],
            Self::Idle => [124, 58, 237, 255],
            Self::Active => [6, 182, 212, 255],
            Self::Muted => [100, 116, 139, 255],
        }
    }
}

/// A generated RGBA icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayIcon {
    /// Edge length in pixels.
    pub size: u32,
    /// Straight RGBA pixels, row major from the top left.
    pub pixels: Vec<u8>,
}

impl TrayIcon {
    /// Render the icon for a state at the requested edge length.
    ///
    /// Sizes are clamped to 8 through 64 pixels: smaller cannot show the mark,
    /// larger wastes memory for a tray icon. Coverage is supersampled four by
    /// four so the circle edge stays smooth at 16 and 22 pixels.
    #[must_use]
    pub fn render(state: TrayState, size: u32) -> Self {
        let size = size.clamp(8, 64);
        let mut pixels = vec![0_u8; (size * size * 4) as usize];
        let center = size as f32 * 0.5;
        let radius = size as f32 * 0.42;
        let base = state.color();
        let samples = 4_u32;
        let sample_count = (samples * samples) as f32;

        for y in 0..size {
            for x in 0..size {
                let mut covered = 0.0_f32;
                for sy in 0..samples {
                    for sx in 0..samples {
                        let px = x as f32 + (sx as f32 + 0.5) / samples as f32;
                        let py = y as f32 + (sy as f32 + 0.5) / samples as f32;
                        let dx = px - center;
                        let dy = py - center;
                        if dx * dx + dy * dy <= radius * radius {
                            covered += 1.0;
                        }
                    }
                }
                let alpha = covered / sample_count;
                if alpha <= 0.0 {
                    continue;
                }

                let dx = x as f32 + 0.5 - center;
                let dy = y as f32 + 0.5 - center;
                let gradient = 1.0 - (dy / radius).clamp(-1.0, 1.0) * 0.18;
                let mut color = [
                    (base[0] as f32 * gradient) as u8,
                    (base[1] as f32 * gradient) as u8,
                    (base[2] as f32 * gradient) as u8,
                ];

                // Four point sparkle. The L1 diamond is the star's body and the
                // small circle in the middle keeps the shape from looking like
                // a rotated square at 16 pixels.
                let star = dx.abs() + dy.abs() <= radius * 0.62;
                let core = dx * dx + dy * dy <= (radius * 0.16).powi(2);
                if star || core {
                    let mark = match state {
                        TrayState::Hidden => [226, 232, 240],
                        TrayState::Muted => [203, 213, 225],
                        _ => [255, 255, 255],
                    };
                    color = mark;
                }

                let index = ((y * size + x) * 4) as usize;
                pixels[index] = color[0];
                pixels[index + 1] = color[1];
                pixels[index + 2] = color[2];
                pixels[index + 3] = (alpha * 255.0).round() as u8;
            }
        }

        Self { size, pixels }
    }

    /// A pixel as RGBA, or `None` outside the image.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.size || y >= self.size {
            return None;
        }
        let index = ((y * self.size + x) * 4) as usize;
        Some([
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
            self.pixels[index + 3],
        ])
    }

    /// True when the pixel is fully transparent.
    #[must_use]
    pub fn is_transparent(&self, x: u32, y: u32) -> bool {
        self.pixel(x, y).is_some_and(|pixel| pixel[3] == 0)
    }
}

/// What selecting a menu item asks the application to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    /// Show the island when hidden, hide it when visible.
    ToggleVisibility,
    /// Jump to the widget whose layout id is given.
    ShowWidget(String),
    /// Switch the color engine.
    SetRgbMode(RgbMode),
    /// Advance to the next widget state.
    NextWidget,
    /// Toggle the autostart entry.
    ToggleAutostart,
    /// Open the configuration file in the default editor.
    OpenSettings,
    /// Open the local documentation.
    OpenDocumentation,
    /// Show the about dialog.
    About,
    /// Shut down cleanly.
    Quit,
}

/// One entry in the tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuItem {
    /// Stable identifier the backend passes back on selection.
    pub id: String,
    /// Visible text.
    pub label: String,
    /// Whether the entry is checkable.
    pub checkable: bool,
    /// Whether the check mark is currently set.
    pub checked: bool,
    /// Whether the entry can be selected.
    pub enabled: bool,
    /// The action, or `None` for a separator or submenu header.
    pub command: Option<TrayCommand>,
    /// Nested entries for a submenu.
    pub children: Vec<TrayMenuItem>,
}

impl TrayMenuItem {
    /// An actionable entry.
    #[must_use]
    pub fn action(id: impl Into<String>, label: impl Into<String>, command: TrayCommand) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checkable: false,
            checked: false,
            enabled: true,
            command: Some(command),
            children: Vec::new(),
        }
    }

    /// Mark the entry checkable and set its current state.
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checkable = true;
        self.checked = checked;
        self
    }

    /// A submenu header with children.
    #[must_use]
    pub fn submenu(id: impl Into<String>, label: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checkable: false,
            checked: false,
            enabled: !children.is_empty(),
            command: None,
            children,
        }
    }

    /// A horizontal separator.
    #[must_use]
    pub fn separator() -> Self {
        Self {
            id: "-".to_string(),
            label: String::new(),
            checkable: false,
            checked: false,
            enabled: false,
            command: None,
            children: Vec::new(),
        }
    }
}

/// Live state the menu reflects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrayMenuState {
    /// The island is visible.
    pub visible: bool,
    /// The autostart entry exists.
    pub autostart: bool,
    /// The active color engine.
    pub rgb_mode: RgbMode,
}

/// The complete tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenu {
    /// Top-level entries.
    pub items: Vec<TrayMenuItem>,
}

impl TrayMenu {
    /// Build the menu from configuration and live state.
    ///
    /// Widget entries follow the layout order, so the tray mirrors the island
    /// instead of duplicating a second list that could drift. Disabled widgets
    /// are omitted.
    #[must_use]
    pub fn build(config: &Config, state: TrayMenuState) -> Self {
        let mut widgets: Vec<TrayMenuItem> = config
            .layout
            .order
            .iter()
            .filter(|id| !id.is_empty())
            .enumerate()
            .map(|(index, id)| {
                TrayMenuItem::action(
                    format!("widget.{id}"),
                    widget_label(id),
                    TrayCommand::ShowWidget(id.clone()),
                )
                .with_enabled(index < config.layout.max_widgets as usize)
            })
            .collect();
        if widgets.is_empty() {
            widgets.push(TrayMenuItem::action(
                "widget.none",
                "No widgets configured",
                TrayCommand::NextWidget,
            ));
        }

        let rgb = RgbMode::ALL
            .iter()
            .map(|mode| {
                TrayMenuItem::action(
                    format!("rgb.{}", mode.id()),
                    rgb_label(*mode),
                    TrayCommand::SetRgbMode(*mode),
                )
                .checked(*mode == state.rgb_mode)
            })
            .collect();

        let items = vec![
            TrayMenuItem::action(
                "toggle",
                if state.visible {
                    "Hide Lumina"
                } else {
                    "Show Lumina"
                },
                TrayCommand::ToggleVisibility,
            )
            .checked(state.visible),
            TrayMenuItem::separator(),
            TrayMenuItem::submenu("widgets", "Widgets", widgets),
            TrayMenuItem::submenu("rgb", "Color Engine", rgb),
            TrayMenuItem::action("next", "Next Widget", TrayCommand::NextWidget),
            TrayMenuItem::separator(),
            TrayMenuItem::action(
                "autostart",
                "Start with the system",
                TrayCommand::ToggleAutostart,
            )
            .checked(state.autostart),
            TrayMenuItem::action("settings", "Settings...", TrayCommand::OpenSettings),
            TrayMenuItem::action("docs", "Documentation", TrayCommand::OpenDocumentation),
            TrayMenuItem::action("about", "About Lumina", TrayCommand::About),
            TrayMenuItem::separator(),
            TrayMenuItem::action("quit", "Quit Lumina", TrayCommand::Quit),
        ];

        Self { items }
    }

    /// Find an item by id at any depth.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&TrayMenuItem> {
        fn walk<'a>(items: &'a [TrayMenuItem], id: &str) -> Option<&'a TrayMenuItem> {
            for item in items {
                if item.id == id {
                    return Some(item);
                }
                if let Some(found) = walk(&item.children, id) {
                    return Some(found);
                }
            }
            None
        }
        walk(&self.items, id)
    }

    /// The command for an id, if it is an actionable leaf.
    #[must_use]
    pub fn command(&self, id: &str) -> Option<&TrayCommand> {
        self.find(id)
            .filter(|item| item.enabled)
            .and_then(|item| item.command.as_ref())
    }

    /// Total number of entries including submenu children.
    #[must_use]
    pub fn len(&self) -> usize {
        fn count(items: &[TrayMenuItem]) -> usize {
            items.iter().map(|item| 1 + count(&item.children)).sum()
        }
        count(&self.items)
    }

    /// True when the menu has no entries apart from separators.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items
            .iter()
            .all(|item| item.command.is_none() && item.children.is_empty())
    }
}

impl TrayMenuItem {
    /// Set whether an entry is selectable.
    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

fn widget_label(id: &str) -> String {
    match id {
        "clock" => "Clock",
        "media" => "Media",
        "spectrum" => "Spectrum",
        "metrics" => "Metrics",
        "battery" => "Battery",
        "network" => "Network",
        "pomodoro" => "Pomodoro",
        "shortcuts" => "Shortcuts",
        "calendar" => "Calendar",
        "weather" => "Weather",
        "notify" => "Notifications",
        "script" => "Scripts",
        other => other,
    }
    .to_string()
}

fn rgb_label(mode: RgbMode) -> String {
    match mode {
        RgbMode::Static => "Static",
        RgbMode::Breathing => "Breathing",
        RgbMode::Cycling => "Cycling",
        RgbMode::Reactive => "Reactive",
        RgbMode::Album => "Album Art",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_renders_a_distinct_icon() {
        let icons: Vec<TrayIcon> = TrayState::ALL
            .iter()
            .map(|state| TrayIcon::render(*state, 22))
            .collect();
        assert_eq!(icons.len(), 4);
        for icon in &icons {
            assert_eq!(icon.size, 22);
            assert_eq!(icon.pixels.len(), 22 * 22 * 4);
            assert!(!icon.is_transparent(11, 11), "the center is opaque");
            assert!(icon.is_transparent(0, 0), "corners are transparent");
        }
        for (index, icon) in icons.iter().enumerate() {
            for other in icons.iter().skip(index + 1) {
                assert_ne!(icon.pixels, other.pixels, "states must be distinguishable");
            }
        }
    }

    #[test]
    fn icon_size_is_clamped() {
        assert_eq!(TrayIcon::render(TrayState::Idle, 1).size, 8);
        assert_eq!(TrayIcon::render(TrayState::Idle, 500).size, 64);
    }

    #[test]
    fn rendering_is_deterministic() {
        let a = TrayIcon::render(TrayState::Active, 16);
        let b = TrayIcon::render(TrayState::Active, 16);
        assert_eq!(a, b);
    }

    #[test]
    fn pixel_lookup_rejects_coordinates_outside_the_icon() {
        let icon = TrayIcon::render(TrayState::Idle, 8);
        assert_eq!(icon.pixel(8, 0), None);
        assert_eq!(icon.pixel(0, 8), None);
        assert!(icon.pixel(0, 0).is_some());
    }

    #[test]
    fn the_menu_mirrors_the_layout_order() {
        let config = Config::default();
        let menu = TrayMenu::build(&config, TrayMenuState::default());
        assert_eq!(
            menu.find("widget.clock").map(|item| item.label.as_str()),
            Some("Clock")
        );
        assert_eq!(
            menu.command("widget.clock"),
            Some(&TrayCommand::ShowWidget("clock".to_string()))
        );
        assert!(menu.len() > 10);
    }

    #[test]
    fn the_toggle_label_and_check_mark_reflect_visibility() {
        let mut state = TrayMenuState::default();
        let menu = TrayMenu::build(&Config::default(), state);
        let toggle = menu.find("toggle").unwrap();
        assert_eq!(toggle.label, "Show Lumina");
        assert!(!toggle.checked);

        state.visible = true;
        let menu = TrayMenu::build(&Config::default(), state);
        let toggle = menu.find("toggle").unwrap();
        assert_eq!(toggle.label, "Hide Lumina");
        assert!(toggle.checked);
    }

    #[test]
    fn the_active_rgb_mode_is_checked() {
        let state = TrayMenuState {
            rgb_mode: RgbMode::Album,
            ..TrayMenuState::default()
        };
        let menu = TrayMenu::build(&Config::default(), state);
        assert!(menu.find("rgb.album").unwrap().checked);
        assert!(!menu.find("rgb.static").unwrap().checked);
        assert_eq!(
            menu.command("rgb.album"),
            Some(&TrayCommand::SetRgbMode(RgbMode::Album))
        );
    }

    #[test]
    fn autostart_state_is_reflected() {
        let state = TrayMenuState {
            autostart: true,
            ..TrayMenuState::default()
        };
        let menu = TrayMenu::build(&Config::default(), state);
        assert!(menu.find("autostart").unwrap().checked);
    }

    #[test]
    fn separators_are_not_commands() {
        let menu = TrayMenu::build(&Config::default(), TrayMenuState::default());
        assert_eq!(menu.command("-"), None);
        assert_eq!(menu.command("missing"), None);
    }

    #[test]
    fn disabled_widgets_stay_in_the_menu_but_are_not_selectable() {
        let mut config = Config::default();
        config.layout.max_widgets = 1;
        let menu = TrayMenu::build(&config, TrayMenuState::default());
        let second = &config.layout.order[1];
        let item = menu.find(&format!("widget.{second}")).unwrap();
        assert!(!item.enabled);
        assert_eq!(menu.command(&format!("widget.{second}")), None);
    }
}
