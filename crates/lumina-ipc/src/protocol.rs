//! Wire protocol: commands, responses, error codes, and state snapshots.
//!
//! A request is a single JSON object with an `id`, a `command`, and an optional
//! `params` object:
//!
//! ```json
//! {"id":7,"command":"notify","params":{"title":"Build done","body":"lumina 0.1.0-alpha"}}
//! ```
//!
//! Responses echo the same `id` and carry either `data` or `error`. The payload
//! is nested on purpose: `shortcut` has its own `id` field, so a flat envelope
//! would collide with the request id.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::framing::MAX_LINE_BYTES;

/// Maximum number of concurrent IPC connections.
pub const MAX_CLIENTS: usize = 8;

/// Longest notification title accepted by [`NotifyArgs`].
pub const MAX_TITLE_CHARS: usize = 256;

/// Longest notification body accepted by [`NotifyArgs`].
pub const MAX_BODY_CHARS: usize = 2048;

/// Identifier attached to a request and echoed by its response.
///
/// Numbers and strings are both accepted so clients can use a monotonic counter
/// or a UUID without changing the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// A numeric id, the default for `lumina-send` and the Python client.
    Number(i64),
    /// A string id, convenient for clients that use UUIDs.
    String(String),
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => write!(formatter, "{number}"),
            Self::String(text) => write!(formatter, "{text}"),
        }
    }
}

/// Stable error codes from the protocol specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IpcErrorCode {
    /// The line was not valid JSON or a required field was missing.
    BadRequest,
    /// The command string is not recognized.
    UnknownCommand,
    /// A field was present but out of range or of the wrong type.
    InvalidArgument,
    /// A profile name does not exist.
    UnknownProfile,
    /// The targeted media session disappeared.
    PlayerGone,
    /// No media session matches the requested player.
    PlayerNotFound,
    /// The command exists but is gated behind a configuration flag.
    Disabled,
    /// The command is unavailable on this platform.
    NotSupported,
    /// More than 120 commands per second arrived from one client.
    RateLimited,
    /// An internal bug; the log carries the details.
    Internal,
    /// A request line exceeded the 64 KiB framing limit.
    LineTooLong,
}

impl IpcErrorCode {
    /// Every code, in declaration order, for clients and documentation tests.
    pub const ALL: [Self; 11] = [
        Self::BadRequest,
        Self::UnknownCommand,
        Self::InvalidArgument,
        Self::UnknownProfile,
        Self::PlayerGone,
        Self::PlayerNotFound,
        Self::Disabled,
        Self::NotSupported,
        Self::RateLimited,
        Self::Internal,
        Self::LineTooLong,
    ];

    /// The exact spelling used on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "BadRequest",
            Self::UnknownCommand => "UnknownCommand",
            Self::InvalidArgument => "InvalidArgument",
            Self::UnknownProfile => "UnknownProfile",
            Self::PlayerGone => "PlayerGone",
            Self::PlayerNotFound => "PlayerNotFound",
            Self::Disabled => "Disabled",
            Self::NotSupported => "NotSupported",
            Self::RateLimited => "RateLimited",
            Self::Internal => "Internal",
            Self::LineTooLong => "LineTooLong",
        }
    }
}

impl fmt::Display for IpcErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for IpcErrorCode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|code| code.as_str() == text)
            .ok_or_else(|| format!("unknown error code `{text}`"))
    }
}

/// A protocol error, serialized as `{"code":...,"message":...}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcError {
    /// Stable machine-readable classification.
    pub code: IpcErrorCode,
    /// Human-readable detail; never contains secrets or trace ids.
    pub message: String,
}

impl IpcError {
    /// Build an error from a code and a message.
    pub fn new(code: IpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The request could not be parsed.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(IpcErrorCode::BadRequest, message)
    }

    /// The command string is unknown.
    pub fn unknown_command(name: &str) -> Self {
        Self::new(
            IpcErrorCode::UnknownCommand,
            format!("unknown command `{name}`"),
        )
    }

    /// A payload field was invalid.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(IpcErrorCode::InvalidArgument, message)
    }

    /// A profile name was requested that does not exist.
    pub fn unknown_profile(name: &str) -> Self {
        Self::new(
            IpcErrorCode::UnknownProfile,
            format!("unknown profile `{name}`"),
        )
    }

    /// A targeted player disappeared.
    pub fn player_gone(name: &str) -> Self {
        Self::new(
            IpcErrorCode::PlayerGone,
            format!("player `{name}` is no longer available"),
        )
    }

    /// No player matched the requested identity.
    pub fn player_not_found(name: &str) -> Self {
        Self::new(
            IpcErrorCode::PlayerNotFound,
            format!("no player matches `{name}`"),
        )
    }

    /// The command is gated behind a configuration flag.
    pub fn disabled(message: impl Into<String>) -> Self {
        Self::new(IpcErrorCode::Disabled, message)
    }

    /// The command is unavailable on this platform.
    pub fn not_supported(message: impl Into<String>) -> Self {
        Self::new(IpcErrorCode::NotSupported, message)
    }

    /// The client exceeded its command budget.
    pub fn rate_limited() -> Self {
        Self::new(
            IpcErrorCode::RateLimited,
            "more than 120 commands per second",
        )
    }

    /// An internal failure; details belong in the log.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(IpcErrorCode::Internal, message)
    }

    /// The request line exceeded [`MAX_LINE_BYTES`].
    pub fn line_too_long() -> Self {
        Self::new(
            IpcErrorCode::LineTooLong,
            format!("request lines are limited to {MAX_LINE_BYTES} bytes"),
        )
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for IpcError {}

fn parse_args<T>(name: &str, payload: &Value) -> Result<T, IpcError>
where
    T: serde::de::DeserializeOwned + Validate,
{
    let parsed: T = serde_json::from_value(payload.clone())
        .map_err(|error| IpcError::invalid_argument(format!("{name}: {error}")))?;
    parsed.validate()?;
    Ok(parsed)
}

fn require_no_args(name: &str, payload: &Value) -> Result<(), IpcError> {
    match payload {
        Value::Object(map) if map.is_empty() => Ok(()),
        Value::Null => Ok(()),
        Value::Object(_) => Err(IpcError::invalid_argument(format!(
            "`{name}` does not take any fields"
        ))),
        other => Err(IpcError::bad_request(format!(
            "payload must be an object, got {}",
            json_type(other)
        ))),
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

trait Validate {
    fn validate(&self) -> Result<(), IpcError>;
}

fn validate_text(field: &str, value: &str, max_chars: usize) -> Result<(), IpcError> {
    if value.trim().is_empty() {
        return Err(IpcError::invalid_argument(format!(
            "`{field}` must not be empty"
        )));
    }
    if value.chars().count() > max_chars {
        return Err(IpcError::invalid_argument(format!(
            "`{field}` is longer than {max_chars} characters"
        )));
    }
    Ok(())
}

fn validate_color(field: &str, value: &str) -> Result<(), IpcError> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let valid = matches!(hex.len(), 3 | 6) && hex.chars().all(|c| c.is_ascii_hexdigit());
    if !valid {
        return Err(IpcError::invalid_argument(format!(
            "`{field}` must be a hex color such as `#ff8800`"
        )));
    }
    Ok(())
}

fn validate_duration(field: &str, value: u64, max_ms: u64) -> Result<(), IpcError> {
    if value > max_ms {
        return Err(IpcError::invalid_argument(format!(
            "`{field}` must be at most {max_ms} milliseconds"
        )));
    }
    Ok(())
}

fn validate_finite(field: &str, value: f32, min: f32, max: f32) -> Result<(), IpcError> {
    if !value.is_finite() || value < min || value > max {
        return Err(IpcError::invalid_argument(format!(
            "`{field}` must be between {min} and {max}"
        )));
    }
    Ok(())
}

/// Payload for `show`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowArgs {
    /// Widget to focus, for example `media` or `calendar`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<String>,
    /// How long to stay expanded before collapsing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Keep the island expanded until told otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<bool>,
}

impl Validate for ShowArgs {
    fn validate(&self) -> Result<(), IpcError> {
        if let Some(widget) = &self.widget {
            validate_text("widget", widget, 64)?;
        }
        if let Some(duration) = self.duration_ms {
            validate_duration("duration_ms", duration, 86_400_000)?;
        }
        Ok(())
    }
}

/// Payload for `notify`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotifyArgs {
    /// Notification title; required.
    pub title: String,
    /// Optional second line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Icon name from the configured icon set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Accent color as `#rgb` or `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Visible duration before the notification fades.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Shell command to run when the notification is clicked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

impl Validate for NotifyArgs {
    fn validate(&self) -> Result<(), IpcError> {
        validate_text("title", &self.title, MAX_TITLE_CHARS)?;
        if let Some(body) = &self.body {
            validate_text("body", body, MAX_BODY_CHARS)?;
        }
        if let Some(icon) = &self.icon {
            validate_text("icon", icon, 128)?;
        }
        if let Some(accent) = &self.accent {
            validate_color("accent", accent)?;
        }
        if let Some(duration) = self.duration_ms {
            validate_duration("duration_ms", duration, 60_000)?;
        }
        if let Some(action) = &self.action {
            validate_text("action", action, 1024)?;
        }
        Ok(())
    }
}

/// Payload for `set_rgb`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetRgbArgs {
    /// Engine name, for example `wave`, `reactive`, or `album`.
    pub mode: String,
    /// Optional fixed color override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Animation speed multiplier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
}

impl Validate for SetRgbArgs {
    fn validate(&self) -> Result<(), IpcError> {
        validate_text("mode", &self.mode, 64)?;
        if let Some(color) = &self.color {
            validate_color("color", color)?;
        }
        if let Some(speed) = self.speed {
            validate_finite("speed", speed, 0.0, 10.0)?;
        }
        Ok(())
    }
}

/// Payload for `set_profile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetProfileArgs {
    /// Exact profile name.
    pub name: String,
}

impl Validate for SetProfileArgs {
    fn validate(&self) -> Result<(), IpcError> {
        validate_text("name", &self.name, 128)
    }
}

/// Payload for `get_config`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetConfigArgs {
    /// Optional dotted section such as `widgets.clock`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
}

impl Validate for GetConfigArgs {
    fn validate(&self) -> Result<(), IpcError> {
        if let Some(section) = &self.section {
            validate_text("section", section, 128)?;
        }
        Ok(())
    }
}

/// Payload for `set_config`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetConfigArgs {
    /// Dotted path such as `widgets.clock.show_seconds`.
    pub path: String,
    /// New value; any JSON value the target key accepts.
    pub value: Value,
    /// Write the change to disk. When false the change lives until restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist: Option<bool>,
}

impl Validate for SetConfigArgs {
    fn validate(&self) -> Result<(), IpcError> {
        validate_text("path", &self.path, 256)?;
        if self.path.starts_with('.') || self.path.ends_with('.') || self.path.contains("..") {
            return Err(IpcError::invalid_argument(
                "`path` must be dot-separated key names",
            ));
        }
        if let Value::String(text) = &self.value {
            if text.chars().count() > 4096 {
                return Err(IpcError::invalid_argument(
                    "`value` strings are limited to 4096 characters",
                ));
            }
        }
        Ok(())
    }
}

/// Playback actions accepted by `media`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaAction {
    /// Start or resume playback.
    Play,
    /// Pause playback.
    Pause,
    /// Toggle between playing and paused.
    PlayPause,
    /// Skip to the next track.
    Next,
    /// Return to the previous track.
    Previous,
    /// Stop playback.
    Stop,
    /// Seek to `position_ms`.
    Seek,
    /// Set the player volume; 0 to 100.
    Volume,
}

/// Payload for `media`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaArgs {
    /// The action to perform.
    pub action: MediaAction,
    /// Optional player identity; the active player is used when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
    /// Target position for [`MediaAction::Seek`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<u64>,
    /// Target volume for [`MediaAction::Volume`], 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
}

impl Validate for MediaArgs {
    fn validate(&self) -> Result<(), IpcError> {
        if let Some(player) = &self.player {
            validate_text("player", player, 128)?;
        }
        if self.action == MediaAction::Seek && self.position_ms.is_none() {
            return Err(IpcError::invalid_argument(
                "`seek` requires `position_ms`",
            ));
        }
        if let Some(position) = self.position_ms {
            validate_duration("position_ms", position, 86_400_000)?;
        }
        if self.action == MediaAction::Volume && self.volume.is_none() {
            return Err(IpcError::invalid_argument("`volume` requires `volume`"));
        }
        if let Some(volume) = self.volume {
            if volume > 100 {
                return Err(IpcError::invalid_argument(
                    "`volume` must be between 0 and 100",
                ));
            }
        }
        Ok(())
    }
}

/// Payload for `shortcut`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutArgs {
    /// Configured shortcut action id, for example `toggle_visibility`.
    pub id: String,
}

impl Validate for ShortcutArgs {
    fn validate(&self) -> Result<(), IpcError> {
        validate_text("id", &self.id, 128)
    }
}

/// Payload for `execute`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteArgs {
    /// Program to run. Never interpreted by a shell.
    pub command: String,
    /// Arguments passed verbatim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Hard timeout; defaults to 10 seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl Validate for ExecuteArgs {
    fn validate(&self) -> Result<(), IpcError> {
        validate_text("command", &self.command, 4096)?;
        if self.args.len() > 64 {
            return Err(IpcError::invalid_argument(
                "`args` accepts at most 64 entries",
            ));
        }
        for arg in &self.args {
            if arg.chars().count() > 4096 {
                return Err(IpcError::invalid_argument(
                    "each `args` entry is limited to 4096 characters",
                ));
            }
        }
        if let Some(timeout) = self.timeout_ms {
            validate_duration("timeout_ms", timeout, 30_000)?;
        }
        Ok(())
    }
}

/// Payload for `spectrum`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpectrumArgs {
    /// Ask the handler to return band peaks since the last call.
    #[serde(default)]
    pub peak: bool,
}

impl Validate for SpectrumArgs {
    fn validate(&self) -> Result<(), IpcError> {
        Ok(())
    }
}

/// Payload for `geometry`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryArgs {
    /// Half-width of the top row, in logical pixels.
    pub top_half_width: f32,
    /// Half-width of the bottom row, in logical pixels.
    pub bottom_half_width: f32,
    /// Overall island height.
    pub height: f32,
    /// Corner radius; clamped by the renderer to half the height.
    pub corner_radius: f32,
}

impl Validate for GeometryArgs {
    fn validate(&self) -> Result<(), IpcError> {
        validate_finite("top_half_width", self.top_half_width, 1.0, 10_000.0)?;
        validate_finite("bottom_half_width", self.bottom_half_width, 1.0, 10_000.0)?;
        validate_finite("height", self.height, 1.0, 2_000.0)?;
        validate_finite("corner_radius", self.corner_radius, 0.0, 2_000.0)?;
        Ok(())
    }
}

/// Payload for `debug_overlay`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebugOverlayArgs {
    /// Whether the frame time and state readout is visible.
    pub enabled: bool,
}

impl Validate for DebugOverlayArgs {
    fn validate(&self) -> Result<(), IpcError> {
        Ok(())
    }
}

/// A validated command with its payload.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Full state dump.
    Status,
    /// Bring the island into a visible state.
    Show(ShowArgs),
    /// Hide the island.
    Hide,
    /// Return to the collapsed state immediately.
    Collapse,
    /// Fire the notification widget.
    Notify(NotifyArgs),
    /// Override the RGB engine for the session.
    SetRgb(SetRgbArgs),
    /// Advance to the next RGB engine.
    CycleRgb,
    /// Switch profiles.
    SetProfile(SetProfileArgs),
    /// Read the effective configuration.
    GetConfig(GetConfigArgs),
    /// Change a configuration value.
    SetConfig(SetConfigArgs),
    /// Re-read the configuration from disk.
    ReloadConfig,
    /// Control media playback.
    Media(MediaArgs),
    /// List known media players.
    PlayerList,
    /// Run a configured shortcut action.
    Shortcut(ShortcutArgs),
    /// Run a program, when `[advanced].ipc_allow_exec` is enabled.
    Execute(ExecuteArgs),
    /// Read current spectrum bands.
    Spectrum(SpectrumArgs),
    /// Edit the island shape live.
    Geometry(GeometryArgs),
    /// Toggle the debug overlay.
    DebugOverlay(DebugOverlayArgs),
    /// Graceful shutdown.
    Quit,
}

impl Command {
    /// The exact command spelling used on the wire.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Show(_) => "show",
            Self::Hide => "hide",
            Self::Collapse => "collapse",
            Self::Notify(_) => "notify",
            Self::SetRgb(_) => "set_rgb",
            Self::CycleRgb => "cycle_rgb",
            Self::SetProfile(_) => "set_profile",
            Self::GetConfig(_) => "get_config",
            Self::SetConfig(_) => "set_config",
            Self::ReloadConfig => "reload_config",
            Self::Media(_) => "media",
            Self::PlayerList => "player_list",
            Self::Shortcut(_) => "shortcut",
            Self::Execute(_) => "execute",
            Self::Spectrum(_) => "spectrum",
            Self::Geometry(_) => "geometry",
            Self::DebugOverlay(_) => "debug_overlay",
            Self::Quit => "quit",
        }
    }

    /// Every command name, for completions and documentation.
    pub const ALL_NAMES: [&'static str; 19] = [
        "status",
        "show",
        "hide",
        "collapse",
        "notify",
        "set_rgb",
        "cycle_rgb",
        "set_profile",
        "get_config",
        "set_config",
        "reload_config",
        "media",
        "player_list",
        "shortcut",
        "execute",
        "spectrum",
        "geometry",
        "debug_overlay",
        "quit",
    ];

    /// Parse a command and its flat payload.
    pub fn from_payload(name: &str, payload: &Value) -> Result<Self, IpcError> {
        let command = match name {
            "status" => {
                require_no_args(name, payload)?;
                Self::Status
            }
            "show" => Self::Show(parse_args(name, payload)?),
            "hide" => {
                require_no_args(name, payload)?;
                Self::Hide
            }
            "collapse" => {
                require_no_args(name, payload)?;
                Self::Collapse
            }
            "notify" => Self::Notify(parse_args(name, payload)?),
            "set_rgb" => Self::SetRgb(parse_args(name, payload)?),
            "cycle_rgb" => {
                require_no_args(name, payload)?;
                Self::CycleRgb
            }
            "set_profile" => Self::SetProfile(parse_args(name, payload)?),
            "get_config" => Self::GetConfig(parse_args(name, payload)?),
            "set_config" => Self::SetConfig(parse_args(name, payload)?),
            "reload_config" => {
                require_no_args(name, payload)?;
                Self::ReloadConfig
            }
            "media" => Self::Media(parse_args(name, payload)?),
            "player_list" => {
                require_no_args(name, payload)?;
                Self::PlayerList
            }
            "shortcut" => Self::Shortcut(parse_args(name, payload)?),
            "execute" => Self::Execute(parse_args(name, payload)?),
            "spectrum" => Self::Spectrum(parse_args(name, payload)?),
            "geometry" => Self::Geometry(parse_args(name, payload)?),
            "debug_overlay" => Self::DebugOverlay(parse_args(name, payload)?),
            "quit" => {
                require_no_args(name, payload)?;
                Self::Quit
            }
            other => return Err(IpcError::unknown_command(other)),
        };
        Ok(command)
    }

    /// Serialize just the payload, as an object. No-argument commands produce
    /// an empty object.
    pub fn payload(&self) -> Result<Value, IpcError> {
        let value = match self {
            Self::Status
            | Self::Hide
            | Self::Collapse
            | Self::CycleRgb
            | Self::ReloadConfig
            | Self::PlayerList
            | Self::Quit => Value::Object(Map::new()),
            Self::Show(args) => to_value(args)?,
            Self::Notify(args) => to_value(args)?,
            Self::SetRgb(args) => to_value(args)?,
            Self::SetProfile(args) => to_value(args)?,
            Self::GetConfig(args) => to_value(args)?,
            Self::SetConfig(args) => to_value(args)?,
            Self::Media(args) => to_value(args)?,
            Self::Shortcut(args) => to_value(args)?,
            Self::Execute(args) => to_value(args)?,
            Self::Spectrum(args) => to_value(args)?,
            Self::Geometry(args) => to_value(args)?,
            Self::DebugOverlay(args) => to_value(args)?,
        };
        Ok(value)
    }

    /// Render the request object with the given id.
    pub fn to_value(&self, id: Option<&RequestId>) -> Result<Value, IpcError> {
        let mut object = Map::new();
        object.insert(
            "id".to_string(),
            id.map_or(Value::Null, |id| {
                serde_json::to_value(id).unwrap_or(Value::Null)
            }),
        );
        object.insert("command".to_string(), Value::String(self.name().to_string()));
        object.insert("params".to_string(), self.payload()?);
        Ok(Value::Object(object))
    }
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, IpcError> {
    serde_json::to_value(value).map_err(|error| IpcError::internal(error.to_string()))
}

/// A parsed request line.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Caller-chosen id; `None` is echoed as `null`.
    pub id: Option<RequestId>,
    /// The validated command.
    pub command: Command,
}

impl Request {
    /// Parse one NDJSON line into a request.
    pub fn parse(line: &str) -> Result<Self, IpcError> {
        let value: Value = serde_json::from_str(line)
            .map_err(|error| IpcError::bad_request(format!("invalid JSON: {error}")))?;
        let Value::Object(mut object) = value else {
            return Err(IpcError::bad_request("request must be a JSON object"));
        };
        let id = match object.remove("id") {
            None | Some(Value::Null) => None,
            Some(value) => Some(serde_json::from_value(value).map_err(|_| {
                IpcError::bad_request("`id` must be a number or a string")
            })?),
        };
        let Some(Value::String(name)) = object.remove("command") else {
            return Err(IpcError::bad_request("missing required field `command`"));
        };
        let payload = object.remove("params").unwrap_or(Value::Null);
        if let Some(unknown) = object.keys().next() {
            return Err(IpcError::bad_request(format!(
                "unexpected top-level field `{unknown}`; command fields belong in `params`"
            )));
        }
        let command = Command::from_payload(&name, &payload)?;
        Ok(Self { id, command })
    }

    /// Serialize the request as one line, including the trailing newline.
    pub fn to_line(&self) -> Result<String, IpcError> {
        let mut line = serde_json::to_string(&self.to_value()?)
            .map_err(|error| IpcError::internal(error.to_string()))?;
        line.push('\n');
        Ok(line)
    }

    /// Serialize the request object.
    pub fn to_value(&self) -> Result<Value, IpcError> {
        self.command.to_value(self.id.as_ref())
    }
}

/// A response to exactly one request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Echo of the request id.
    pub id: Option<RequestId>,
    /// Whether the command succeeded.
    pub ok: bool,
    /// Result payload for successful commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Error detail for failed commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<IpcError>,
}

impl Response {
    /// A successful response carrying `data` (possibly `null`).
    pub fn success(id: Option<RequestId>, data: Value) -> Self {
        Self {
            id,
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    /// A failed response carrying a protocol error.
    pub fn failure(id: Option<RequestId>, error: IpcError) -> Self {
        Self {
            id,
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Window geometry as reported in a [`StateSnapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeometrySnapshot {
    /// Logical x coordinate on the virtual desktop.
    pub x: f32,
    /// Logical y coordinate on the virtual desktop.
    pub y: f32,
    /// Logical width.
    pub width: f32,
    /// Logical height.
    pub height: f32,
    /// Scale factor of the monitor the island is on.
    pub scale: f32,
    /// Monitor name, when the backend can report one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
}

/// The active RGB engine as reported in a [`StateSnapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RgbSnapshot {
    /// Engine name.
    pub mode: String,
    /// Current effective color as `#rrggbb`.
    pub color: String,
    /// Animation speed multiplier.
    pub speed: f32,
}

impl Default for RgbSnapshot {
    fn default() -> Self {
        Self {
            mode: "wave".to_string(),
            color: "#5ac8fa".to_string(),
            speed: 1.0,
        }
    }
}

/// The active media session as reported in a [`StateSnapshot`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaSnapshot {
    /// Player identity, for example `spotify` or `mpv`.
    pub player: String,
    /// Track title.
    pub title: String,
    /// Track artist.
    pub artist: String,
    /// Playback status: `playing`, `paused`, or `stopped`.
    pub status: String,
    /// Position in milliseconds, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<u64>,
    /// Track duration in milliseconds, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Full state dump returned by `status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// Lumina version.
    pub version: String,
    /// Island state name, for example `collapsed` or `expanded`.
    pub state: String,
    /// Whether the island window is visible.
    pub visible: bool,
    /// Whether expansion is pinned.
    pub pinned: bool,
    /// Active profile name.
    pub profile: String,
    /// Current window geometry, when a window exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<GeometrySnapshot>,
    /// Active RGB engine.
    pub rgb: RgbSnapshot,
    /// Active media session, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaSnapshot>,
    /// Milliseconds since the orchestrator started.
    pub uptime_ms: u64,
}

impl Default for StateSnapshot {
    fn default() -> Self {
        Self {
            version: crate::VERSION.to_string(),
            state: "collapsed".to_string(),
            visible: false,
            pinned: false,
            profile: "default".to_string(),
            geometry: None,
            rgb: RgbSnapshot::default(),
            media: None,
            uptime_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(value: Value) -> Value {
        value
    }

    #[test]
    fn every_error_code_round_trips_through_its_wire_name() {
        for code in IpcErrorCode::ALL {
            assert_eq!(code.as_str().parse::<IpcErrorCode>(), Ok(code));
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!("\"{}\"", code.as_str()));
        }
    }

    #[test]
    fn unknown_error_codes_are_rejected_by_name() {
        assert!("Nope".parse::<IpcErrorCode>().is_err());
        assert!("badrequest".parse::<IpcErrorCode>().is_err());
    }

    #[test]
    fn a_notify_request_parses_its_params() {
        let request = Request::parse(
            r#"{"id":3,"command":"notify","params":{"title":"Build done","body":"ok"}}"#,
        )
        .unwrap();
        assert_eq!(request.id, Some(RequestId::Number(3)));
        let Command::Notify(args) = request.command else {
            panic!("expected a notify command");
        };
        assert_eq!(args.title, "Build done");
        assert_eq!(args.body.as_deref(), Some("ok"));
    }

    #[test]
    fn top_level_fields_outside_the_envelope_are_rejected() {
        let error = Request::parse(
            r#"{"id":1,"command":"notify","title":"Build done"}"#,
        )
        .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::BadRequest);
        assert!(error.message.contains("params"));
    }

    #[test]
    fn a_shortcut_id_does_not_collide_with_the_request_id() {
        let request =
            Request::parse(r#"{"id":9,"command":"shortcut","params":{"id":"toggle"}}"#).unwrap();
        let Command::Shortcut(args) = &request.command else {
            panic!("expected a shortcut command");
        };
        assert_eq!(args.id, "toggle");
        let line = request.to_line().unwrap();
        let reparsed = Request::parse(&line).unwrap();
        assert_eq!(reparsed.id, Some(RequestId::Number(9)));
    }

    #[test]
    fn string_ids_are_preserved_exactly() {
        let request =
            Request::parse(r#"{"id":"abc-1","command":"hide"}"#).unwrap();
        assert_eq!(request.id, Some(RequestId::String("abc-1".to_string())));
        let line = request.to_line().unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
    }

    #[test]
    fn a_missing_id_becomes_null() {
        let request = Request::parse(r#"{"command":"status"}"#).unwrap();
        assert_eq!(request.id, None);
        let value = request.to_value().unwrap();
        assert_eq!(value["id"], Value::Null);
    }

    #[test]
    fn a_missing_command_is_a_bad_request() {
        let error = Request::parse(r#"{"id":1}"#).unwrap_err();
        assert_eq!(error.code, IpcErrorCode::BadRequest);
    }

    #[test]
    fn invalid_json_is_a_bad_request() {
        let error = Request::parse("{").unwrap_err();
        assert_eq!(error.code, IpcErrorCode::BadRequest);
    }

    #[test]
    fn an_unknown_command_is_reported_by_name() {
        let error = Request::parse(r#"{"id":1,"command":"dance"}"#).unwrap_err();
        assert_eq!(error.code, IpcErrorCode::UnknownCommand);
        assert!(error.message.contains("dance"));
    }

    #[test]
    fn command_names_are_unique() {
        let mut names = Vec::new();
        for name in Command::ALL_NAMES {
            assert!(!names.contains(&name), "duplicate command {name}");
            names.push(name);
        }
        assert_eq!(names.len(), 19, "every documented command has a name");
        assert_eq!(
            Command::from_payload("dance", &Value::Null)
                .unwrap_err()
                .code,
            IpcErrorCode::UnknownCommand
        );
    }

    #[test]
    fn no_argument_commands_reject_extra_fields() {
        let error = Command::from_payload("hide", &payload(serde_json::json!({"x":1})))
            .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let error = Command::from_payload(
            "notify",
            &payload(serde_json::json!({"title":"hi","colour":"#fff"})),
        )
        .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn an_empty_title_is_rejected() {
        let error =
            Command::from_payload("notify", &payload(serde_json::json!({"title":"  "})))
                .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn colors_accept_three_and_six_digit_hex() {
        assert!(validate_color("accent", "#abc").is_ok());
        assert!(validate_color("accent", "a1b2c3").is_ok());
        assert!(validate_color("accent", "#abcd").is_err());
        assert!(validate_color("accent", "#gggggg").is_err());
        assert!(validate_color("accent", "").is_err());
    }

    #[test]
    fn show_durations_are_bounded() {
        let error = Command::from_payload(
            "show",
            &payload(serde_json::json!({"duration_ms": 86_400_001})),
        )
        .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn media_seek_requires_a_position() {
        let error =
            Command::from_payload("media", &payload(serde_json::json!({"action":"seek"})))
                .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn media_volume_requires_a_volume() {
        let error = Command::from_payload(
            "media",
            &payload(serde_json::json!({"action":"volume"})),
        )
        .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn media_rejects_out_of_range_volume() {
        let error = Command::from_payload(
            "media",
            &payload(serde_json::json!({"action":"volume","volume":101})),
        )
        .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn media_accepts_every_documented_action() {
        for action in ["play", "pause", "play_pause", "next", "previous", "stop"] {
            let command = Command::from_payload(
                "media",
                &payload(serde_json::json!({"action": action})),
            )
            .unwrap();
            assert_eq!(command.name(), "media");
        }
    }

    #[test]
    fn geometry_values_must_be_finite_and_positive() {
        let error = Command::from_payload(
            "geometry",
            &payload(serde_json::json!({
                "top_half_width": 0.0,
                "bottom_half_width": 100.0,
                "height": 40.0,
                "corner_radius": 12.0
            })),
        )
        .unwrap_err();
        assert_eq!(error.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn set_config_accepts_arbitrary_values() {
        let command = Command::from_payload(
            "set_config",
            &payload(serde_json::json!({
                "path": "general.start_hidden",
                "value": true,
                "persist": false
            })),
        )
        .unwrap();
        let Command::SetConfig(args) = command else {
            panic!("expected set_config");
        };
        assert_eq!(args.value, Value::Bool(true));
    }

    #[test]
    fn set_config_rejects_broken_paths() {
        for path in ["", ".general", "general.", "general..start"] {
            let error = Command::from_payload(
                "set_config",
                &payload(serde_json::json!({"path": path, "value": 1})),
            )
            .unwrap_err();
            assert_eq!(error.code, IpcErrorCode::InvalidArgument, "path {path}");
        }
    }

    #[test]
    fn responses_have_the_documented_shape() {
        let ok = serde_json::to_value(Response::success(
            Some(RequestId::Number(1)),
            Value::String("ok".to_string()),
        ))
        .unwrap();
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["data"], "ok");
        assert!(ok.get("error").is_none());

        let failed = serde_json::to_value(Response::failure(
            Some(RequestId::Number(1)),
            IpcError::unknown_command("dance"),
        ))
        .unwrap();
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["error"]["code"], "UnknownCommand");
        assert!(failed.get("data").is_none());
    }

    #[test]
    fn a_round_trip_preserves_every_command() {
        let payloads = [
            ("show", serde_json::json!({"widget":"media","pin":true})),
            ("notify", serde_json::json!({"title":"hi"})),
            ("set_rgb", serde_json::json!({"mode":"wave","speed":1.5})),
            ("set_profile", serde_json::json!({"name":"work"})),
            ("get_config", serde_json::json!({"section":"widgets"})),
            (
                "set_config",
                serde_json::json!({"path":"general.x","value":1}),
            ),
            (
                "media",
                serde_json::json!({"action":"seek","position_ms":1000}),
            ),
            ("shortcut", serde_json::json!({"id":"toggle_visibility"})),
            (
                "execute",
                serde_json::json!({"command":"git","args":["status"]}),
            ),
            ("spectrum", serde_json::json!({"peak":true})),
            (
                "geometry",
                serde_json::json!({
                    "top_half_width": 200.0,
                    "bottom_half_width": 220.0,
                    "height": 62.0,
                    "corner_radius": 26.0
                }),
            ),
            ("debug_overlay", serde_json::json!({"enabled":true})),
        ];
        for (name, value) in payloads {
            let command = Command::from_payload(name, &value).unwrap();
            let request = Request {
                id: Some(RequestId::Number(9)),
                command,
            };
            let line = request.to_line().unwrap();
            let parsed = Request::parse(&line).unwrap();
            assert_eq!(parsed, request);
        }
    }

    #[test]
    fn error_display_never_contains_an_em_dash() {
        let errors = [
            IpcError::bad_request("x"),
            IpcError::unknown_command("dance"),
            IpcError::invalid_argument("x"),
            IpcError::unknown_profile("x"),
            IpcError::player_gone("x"),
            IpcError::player_not_found("x"),
            IpcError::disabled("x"),
            IpcError::not_supported("x"),
            IpcError::rate_limited(),
            IpcError::internal("x"),
            IpcError::line_too_long(),
        ];
        for error in errors {
            assert!(!error.to_string().contains('\u{2014}'));
            assert!(!error.message.contains('\u{2014}'));
        }
    }
}
