//! Global hotkey syntax, normalization, and conflict detection.
//!
//! # Syntax
//!
//! A binding is a `+` separated list of modifiers followed by exactly one key.
//! Modifiers are `ctrl`, `alt`, `shift`, `super` (aliases `meta`, `win`, and
//! `cmd`), case insensitive and in any order. Keys are letters, digits, named
//! keys such as `f12` or `pageup`, or the punctuation names listed in
//! [`Key::from_name`]. Surrounding whitespace is ignored and `ctrl + alt + l`
//! parses the same as `CTRL+ALT+L`.
//!
//! An empty or whitespace-only binding means "unbound".

use std::fmt;

/// Modifier keys held with the main key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Modifiers {
    /// Control.
    pub ctrl: bool,
    /// Alt, or Option on macOS.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// Super, Windows, or Command.
    pub super_key: bool,
}

impl Modifiers {
    /// No modifiers at all.
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        super_key: false,
    };

    /// True when no modifier is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        !self.ctrl && !self.alt && !self.shift && !self.super_key
    }

    /// Win32 modifier flags for `RegisterHotKey`, including `MOD_NOREPEAT`.
    #[must_use]
    pub fn windows_flags(self) -> u32 {
        (u32::from(self.alt) * 0x0001)
            | (u32::from(self.ctrl) * 0x0002)
            | (u32::from(self.shift) * 0x0004)
            | (u32::from(self.super_key) * 0x0008)
            | 0x4000
    }

    /// Render the modifiers in canonical order.
    fn write_to(self, out: &mut String) {
        for (enabled, label) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.super_key, "Super"),
        ] {
            if enabled {
                if !out.is_empty() {
                    out.push('+');
                }
                out.push_str(label);
            }
        }
    }
}

/// The non-modifier part of a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A letter, digit, or punctuation character.
    Char(char),
    /// Tab.
    Tab,
    /// Return or Enter.
    Enter,
    /// Escape.
    Escape,
    /// Space.
    Space,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// A function key, 1 through 24.
    Function(u8),
    /// Caps Lock.
    CapsLock,
    /// Num Lock.
    NumLock,
    /// Scroll Lock.
    ScrollLock,
    /// Print Screen.
    PrintScreen,
    /// Pause.
    Pause,
}

impl Key {
    /// Parse a key name, case insensitive.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.trim().to_ascii_lowercase();
        if lower.chars().count() == 1 {
            let ch = lower.chars().next()?;
            if ch.is_ascii_alphanumeric() {
                return Some(Self::Char(ch.to_ascii_uppercase()));
            }
            if ch.is_ascii_punctuation() {
                return Some(Self::Char(ch));
            }
        }
        let key = match lower.as_str() {
            "tab" => Self::Tab,
            "enter" | "return" => Self::Enter,
            "escape" | "esc" => Self::Escape,
            "space" => Self::Space,
            "backspace" => Self::Backspace,
            "delete" | "del" => Self::Delete,
            "insert" | "ins" => Self::Insert,
            "home" => Self::Home,
            "end" => Self::End,
            "pageup" | "pgup" => Self::PageUp,
            "pagedown" | "pgdn" => Self::PageDown,
            "up" | "arrowup" => Self::Up,
            "down" | "arrowdown" => Self::Down,
            "left" | "arrowleft" => Self::Left,
            "right" | "arrowright" => Self::Right,
            "capslock" | "caps" => Self::CapsLock,
            "numlock" => Self::NumLock,
            "scrolllock" => Self::ScrollLock,
            "printscreen" | "prtsc" => Self::PrintScreen,
            "pause" => Self::Pause,
            "plus" => Self::Char('+'),
            "minus" => Self::Char('-'),
            "comma" => Self::Char(','),
            "period" | "dot" => Self::Char('.'),
            "slash" => Self::Char('/'),
            "backslash" => Self::Char('\\'),
            "semicolon" => Self::Char(';'),
            "quote" | "apostrophe" => Self::Char('\''),
            "backquote" | "grave" => Self::Char('`'),
            "bracketleft" => Self::Char('['),
            "bracketright" => Self::Char(']'),
            "equal" => Self::Char('='),
            _ => {
                let digits = lower.strip_prefix('f')?;
                let number: u8 = digits.parse().ok()?;
                if (1..=24).contains(&number) {
                    Self::Function(number)
                } else {
                    return None;
                }
            }
        };
        Some(key)
    }

    /// Canonical label used in the config file and the UI.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Char(ch) => ch.to_string(),
            Self::Tab => "Tab".to_string(),
            Self::Enter => "Enter".to_string(),
            Self::Escape => "Escape".to_string(),
            Self::Space => "Space".to_string(),
            Self::Backspace => "Backspace".to_string(),
            Self::Delete => "Delete".to_string(),
            Self::Insert => "Insert".to_string(),
            Self::Home => "Home".to_string(),
            Self::End => "End".to_string(),
            Self::PageUp => "PageUp".to_string(),
            Self::PageDown => "PageDown".to_string(),
            Self::Up => "Up".to_string(),
            Self::Down => "Down".to_string(),
            Self::Left => "Left".to_string(),
            Self::Right => "Right".to_string(),
            Self::Function(number) => format!("F{number}"),
            Self::CapsLock => "CapsLock".to_string(),
            Self::NumLock => "NumLock".to_string(),
            Self::ScrollLock => "ScrollLock".to_string(),
            Self::PrintScreen => "PrintScreen".to_string(),
            Self::Pause => "Pause".to_string(),
        }
    }

    /// Win32 virtual key code, or `None` for a key Win32 cannot register.
    #[must_use]
    pub fn windows_virtual_key(self) -> Option<u32> {
        let code = match self {
            Self::Char(ch) => match ch {
                'A'..='Z' | '0'..='9' => u32::from(ch as u8),
                ';' => 0xBA,
                '=' => 0xBB,
                ',' => 0xBC,
                '-' => 0xBD,
                '.' => 0xBE,
                '/' => 0xBF,
                '`' => 0xC0,
                '[' => 0xDB,
                '\\' => 0xDC,
                ']' => 0xDD,
                '\'' => 0xDE,
                '+' => 0xBB,
                _ => return None,
            },
            Self::Tab => 0x09,
            Self::Enter => 0x0D,
            Self::Escape => 0x1B,
            Self::Space => 0x20,
            Self::Backspace => 0x08,
            Self::Delete => 0x2E,
            Self::Insert => 0x2D,
            Self::Home => 0x24,
            Self::End => 0x23,
            Self::PageUp => 0x21,
            Self::PageDown => 0x22,
            Self::Up => 0x26,
            Self::Down => 0x28,
            Self::Left => 0x25,
            Self::Right => 0x27,
            Self::Function(number) => 0x70 + u32::from(number) - 1,
            Self::CapsLock => 0x14,
            Self::NumLock => 0x90,
            Self::ScrollLock => 0x91,
            Self::PrintScreen => 0x2C,
            Self::Pause => 0x13,
        };
        Some(code)
    }


}

/// A parsed global hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hotkey {
    /// Held modifiers.
    pub modifiers: Modifiers,
    /// The main key.
    pub key: Key,
}

impl Hotkey {
    /// Parse a binding, or return `None` for an unbound empty string.
    ///
    /// # Errors
    ///
    /// Returns a [`HotkeyParseError`] when a token is not a modifier, several
    /// main keys are present, or the key name is unknown.
    pub fn parse(input: &str) -> Result<Option<Self>, HotkeyParseError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let mut modifiers = Modifiers::NONE;
        let mut key: Option<Key> = None;

        for token in trimmed.split('+') {
            let token = token.trim();
            if token.is_empty() {
                return Err(HotkeyParseError::EmptyToken(input.to_string()));
            }
            if let Some(flag) = modifier_from_name(token) {
                if key.is_some() {
                    return Err(HotkeyParseError::ModifierAfterKey(token.to_string()));
                }
                match flag {
                    Modifier::Ctrl => modifiers.ctrl = true,
                    Modifier::Alt => modifiers.alt = true,
                    Modifier::Shift => modifiers.shift = true,
                    Modifier::Super => modifiers.super_key = true,
                }
                continue;
            }
            if key.is_some() {
                return Err(HotkeyParseError::MultipleKeys(input.to_string()));
            }
            key = Some(Key::from_name(token).ok_or_else(|| HotkeyParseError::UnknownKey(token.to_string()))?);
        }

        match key {
            Some(key) => Ok(Some(Self { modifiers, key })),
            None => Err(HotkeyParseError::MissingKey(input.to_string())),
        }
    }

    /// Parse a binding, treating an empty string as unbound.
    ///
    /// # Errors
    ///
    /// Propagates [`HotkeyParseError`] from [`Hotkey::parse`].
    pub fn parse_optional(input: &str) -> Result<Option<Self>, HotkeyParseError> {
        Self::parse(input)
    }

    /// Canonical text form, suitable for writing back to the config file.
    #[must_use]
    pub fn to_config_string(self) -> String {
        let mut out = String::new();
        self.modifiers.write_to(&mut out);
        if !out.is_empty() {
            out.push('+');
        }
        out.push_str(&self.key.label());
        out
    }

    /// True when the binding has no modifiers.
    #[must_use]
    pub const fn is_plain_key(self) -> bool {
        self.modifiers.is_empty()
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_config_string())
    }
}

enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

fn modifier_from_name(name: &str) -> Option<Modifier> {
    match name.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Modifier::Ctrl),
        "alt" | "option" => Some(Modifier::Alt),
        "shift" => Some(Modifier::Shift),
        "super" | "meta" | "win" | "windows" | "cmd" | "command" => Some(Modifier::Super),
        _ => None,
    }
}

/// Why a binding could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyParseError {
    /// An empty token between separators, for example `ctrl++l`.
    EmptyToken(String),
    /// A modifier appeared after the main key, for example `l+ctrl`.
    ModifierAfterKey(String),
    /// An unrecognised key name.
    UnknownKey(String),
    /// More than one main key was supplied.
    MultipleKeys(String),
    /// Only modifiers were supplied.
    MissingKey(String),
}

impl fmt::Display for HotkeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyToken(input) => write!(f, "{input:?} contains an empty key token"),
            Self::ModifierAfterKey(token) => {
                write!(f, "modifier {token:?} must come before the key")
            }
            Self::UnknownKey(token) => write!(f, "unknown key {token:?}"),
            Self::MultipleKeys(input) => write!(f, "{input:?} contains more than one key"),
            Self::MissingKey(input) => write!(f, "{input:?} has no key"),
        }
    }
}

impl std::error::Error for HotkeyParseError {}

/// A hotkey claimed by more than one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyConflict {
    /// The shared binding.
    pub hotkey: Hotkey,
    /// Actions claiming it, in sorted order.
    pub actions: Vec<String>,
}

/// Find bindings claimed by more than one action.
///
/// Unbound actions are represented by `None` and ignored. The result is sorted
/// by binding so a validation report is stable between runs.
#[must_use]
pub fn find_conflicts(bindings: &[(String, Option<Hotkey>)]) -> Vec<HotkeyConflict> {
    let mut groups: Vec<(Hotkey, Vec<String>)> = Vec::new();
    for (action, hotkey) in bindings {
        let Some(hotkey) = hotkey else {
            continue;
        };
        if let Some((_, actions)) = groups.iter_mut().find(|(key, _)| key == hotkey) {
            actions.push(action.clone());
        } else {
            groups.push((*hotkey, vec![action.clone()]));
        }
    }
    groups.retain(|(_, actions)| actions.len() > 1);
    groups.sort_by_key(|(hotkey, _)| *hotkey);
    for (_, actions) in &mut groups {
        actions.sort();
        actions.dedup();
    }
    groups
        .into_iter()
        .map(|(hotkey, actions)| HotkeyConflict { hotkey, actions })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Hotkey {
        Hotkey::parse(input).unwrap().expect("bound")
    }

    #[test]
    fn modifiers_and_keys_parse_case_insensitively() {
        assert_eq!(parse("CTRL+Alt+l"), parse("ctrl+alt+L"));
        assert_eq!(
            parse("ctrl+alt+l"),
            Hotkey {
                modifiers: Modifiers {
                    ctrl: true,
                    alt: true,
                    ..Modifiers::NONE
                },
                key: Key::Char('L'),
            }
        );
    }

    #[test]
    fn modifier_order_does_not_matter() {
        assert_eq!(parse("shift+ctrl+alt+x"), parse("ctrl+alt+shift+x"));
        assert_eq!(parse("alt+ctrl+shift+x"), parse("ctrl+shift+alt+x"));
    }

    #[test]
    fn meta_is_an_alias_for_super() {
        assert_eq!(parse("meta+space"), parse("super+space"));
        assert_eq!(parse("win+space"), parse("super+space"));
    }

    #[test]
    fn whitespace_around_tokens_is_ignored() {
        assert_eq!(parse(" ctrl + alt + l "), parse("ctrl+alt+l"));
    }

    #[test]
    fn an_empty_binding_is_unbound() {
        assert_eq!(Hotkey::parse("").unwrap(), None);
        assert_eq!(Hotkey::parse("   ").unwrap(), None);
    }

    #[test]
    fn named_keys_and_function_keys_parse() {
        assert_eq!(parse("ctrl+tab").key, Key::Tab);
        assert_eq!(parse("ctrl+f12").key, Key::Function(12));
        assert_eq!(parse("ctrl+pageup").key, Key::PageUp);
        assert_eq!(parse("ctrl+esc").key, Key::Escape);
        assert_eq!(parse("ctrl+plus").key, Key::Char('+'));
    }

    #[test]
    fn out_of_range_function_keys_are_rejected() {
        assert!(Hotkey::parse("ctrl+f25").is_err());
        assert!(Hotkey::parse("ctrl+f0").is_err());
    }

    #[test]
    fn modifier_after_the_key_is_rejected() {
        assert_eq!(
            Hotkey::parse("l+ctrl"),
            Err(HotkeyParseError::ModifierAfterKey("ctrl".to_string()))
        );
    }

    #[test]
    fn multiple_keys_are_rejected() {
        assert!(matches!(
            Hotkey::parse("ctrl+a+b"),
            Err(HotkeyParseError::MultipleKeys(_))
        ));
    }

    #[test]
    fn modifiers_without_a_key_are_rejected() {
        assert!(matches!(
            Hotkey::parse("ctrl+alt"),
            Err(HotkeyParseError::MissingKey(_))
        ));
    }

    #[test]
    fn an_empty_token_is_rejected() {
        assert!(Hotkey::parse("ctrl++l").is_err());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert_eq!(
            Hotkey::parse("ctrl+teleport"),
            Err(HotkeyParseError::UnknownKey("teleport".to_string()))
        );
    }

    #[test]
    fn the_canonical_form_is_stable() {
        assert_eq!(parse("alt+ctrl+SHIFT+l").to_config_string(), "Ctrl+Alt+Shift+L");
        assert_eq!(parse("super+space").to_config_string(), "Super+Space");
        assert_eq!(parse("ctrl+f5").to_config_string(), "Ctrl+F5");
    }

    #[test]
    fn every_known_key_round_trips_through_its_label() {
        for key in [
            Key::Tab,
            Key::Enter,
            Key::Escape,
            Key::Space,
            Key::Backspace,
            Key::Delete,
            Key::Insert,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Function(24),
            Key::CapsLock,
            Key::NumLock,
            Key::ScrollLock,
            Key::PrintScreen,
            Key::Pause,
            Key::Char(';'),
        ] {
            assert_eq!(Key::from_name(&key.label()), Some(key), "{key:?}");
        }
    }

    #[test]
    fn win32_codes_are_defined_for_supported_keys() {
        assert_eq!(Key::Char('A').windows_virtual_key(), Some(0x41));
        assert_eq!(Key::Function(1).windows_virtual_key(), Some(0x70));
        assert_eq!(Key::Function(24).windows_virtual_key(), Some(0x87));
        assert_eq!(Key::Char(';').windows_virtual_key(), Some(0xBA));
    }

    #[test]
    fn conflicts_are_grouped_and_sorted() {
        let toggle = parse("ctrl+alt+l");
        let rgb = parse("ctrl+alt+r");
        let bindings = vec![
            ("toggle".to_string(), Some(toggle)),
            ("rgb".to_string(), Some(rgb)),
            ("other".to_string(), Some(toggle)),
            ("unbound".to_string(), None),
        ];
        let conflicts = find_conflicts(&bindings);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].hotkey, toggle);
        assert_eq!(conflicts[0].actions, vec!["other", "toggle"]);
    }

    #[test]
    fn identical_actions_do_not_conflict_with_themselves() {
        let bindings = vec![("toggle".to_string(), Some(parse("ctrl+alt+l")))];
        assert!(find_conflicts(&bindings).is_empty());
    }
}
