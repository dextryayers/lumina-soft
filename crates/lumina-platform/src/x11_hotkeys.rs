//! X11 global hotkeys through `XGrabKey`.
//!
//! # Lock modifiers
//!
//! X11 reports Caps Lock, Num Lock, and Scroll Lock as ordinary modifiers, so a
//! single `XGrabKey` for `Ctrl+Alt+L` does not fire while Num Lock is on. Every
//! binding is therefore grabbed once per lock combination. The event matcher
//! masks those bits out before comparing, which keeps activation independent of
//! the user's lock state.
//!
//! # Ownership
//!
//! The backend owns its own connection to the display. It does not share the
//! overlay window or the monitor connection, so hotkeys keep working while the
//! island is hidden or its window is being rebuilt.

use std::collections::HashMap;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
use x11rb::rust_connection::RustConnection;

use crate::hotkey::{Hotkey, HotkeyBinding, HotkeyError, HotkeyEvent, HotkeyRegistry, Key};

/// Keysyms that are not plain ASCII.
mod keysym {
    pub(super) const TAB: u32 = 0xFF09;
    pub(super) const ENTER: u32 = 0xFF0D;
    pub(super) const ESCAPE: u32 = 0xFF1B;
    pub(super) const BACKSPACE: u32 = 0xFF08;
    pub(super) const DELETE: u32 = 0xFFFF;
    pub(super) const INSERT: u32 = 0xFF63;
    pub(super) const HOME: u32 = 0xFF50;
    pub(super) const END: u32 = 0xFF57;
    pub(super) const PAGE_UP: u32 = 0xFF55;
    pub(super) const PAGE_DOWN: u32 = 0xFF56;
    pub(super) const UP: u32 = 0xFF52;
    pub(super) const DOWN: u32 = 0xFF54;
    pub(super) const LEFT: u32 = 0xFF51;
    pub(super) const RIGHT: u32 = 0xFF53;
    pub(super) const F1: u32 = 0xFFBE;
    pub(super) const CAPS_LOCK: u32 = 0xFFE5;
    pub(super) const NUM_LOCK: u32 = 0xFF7F;
    pub(super) const SCROLL_LOCK: u32 = 0xFF14;
    pub(super) const PRINT_SCREEN: u32 = 0xFF61;
    pub(super) const PAUSE: u32 = 0xFF13;
}

/// Translate a key to its X11 keysym.
#[must_use]
pub fn key_to_keysym(key: Key) -> u32 {
    match key {
        Key::Char(ch) => ch.to_ascii_uppercase() as u32,
        Key::Tab => keysym::TAB,
        Key::Enter => keysym::ENTER,
        Key::Escape => keysym::ESCAPE,
        Key::Space => 0x20,
        Key::Backspace => keysym::BACKSPACE,
        Key::Delete => keysym::DELETE,
        Key::Insert => keysym::INSERT,
        Key::Home => keysym::HOME,
        Key::End => keysym::END,
        Key::PageUp => keysym::PAGE_UP,
        Key::PageDown => keysym::PAGE_DOWN,
        Key::Up => keysym::UP,
        Key::Down => keysym::DOWN,
        Key::Left => keysym::LEFT,
        Key::Right => keysym::RIGHT,
        Key::Function(number) => keysym::F1 + u32::from(number) - 1,
        Key::CapsLock => keysym::CAPS_LOCK,
        Key::NumLock => keysym::NUM_LOCK,
        Key::ScrollLock => keysym::SCROLL_LOCK,
        Key::PrintScreen => keysym::PRINT_SCREEN,
        Key::Pause => keysym::PAUSE,
    }
}

fn modifier_mask(hotkey: Hotkey) -> ModMask {
    let mut mask = ModMask::default();
    if hotkey.modifiers.shift {
        mask |= ModMask::SHIFT;
    }
    if hotkey.modifiers.ctrl {
        mask |= ModMask::CONTROL;
    }
    if hotkey.modifiers.alt {
        mask |= ModMask::M1;
    }
    if hotkey.modifiers.super_key {
        mask |= ModMask::M4;
    }
    mask
}

/// Every lock combination grabbed for one binding.
///
/// Caps Lock is `Lock`, Num Lock is `Mod2`, and Scroll Lock is `Mod3` on some
/// layouts and `Mod5` on others, so both are covered. The event matcher masks
/// all of them out before comparing.
fn lock_variants() -> [ModMask; 16] {
    [
        ModMask::default(),
        ModMask::LOCK,
        ModMask::M2,
        ModMask::M3,
        ModMask::M5,
        ModMask::LOCK | ModMask::M2,
        ModMask::LOCK | ModMask::M3,
        ModMask::LOCK | ModMask::M5,
        ModMask::M2 | ModMask::M3,
        ModMask::M2 | ModMask::M5,
        ModMask::M3 | ModMask::M5,
        ModMask::LOCK | ModMask::M2 | ModMask::M3,
        ModMask::LOCK | ModMask::M2 | ModMask::M5,
        ModMask::LOCK | ModMask::M3 | ModMask::M5,
        ModMask::M2 | ModMask::M3 | ModMask::M5,
        ModMask::LOCK | ModMask::M2 | ModMask::M3 | ModMask::M5,
    ]
}

#[derive(Debug, Clone)]
struct Grab {
    keycode: u8,
    hotkey: Hotkey,
    action: String,
}

/// The X11 hotkey backend.
pub struct X11Hotkeys {
    connection: RustConnection,
    root: x11rb::protocol::xproto::Window,
    keycodes: HashMap<u32, u8>,
    grabs: Vec<Grab>,
}

impl std::fmt::Debug for X11Hotkeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X11Hotkeys")
            .field("grabs", &self.grabs.len())
            .finish_non_exhaustive()
    }
}

impl X11Hotkeys {
    /// Connect to the display and prepare keymap lookup.
    ///
    /// # Errors
    ///
    /// Returns [`HotkeyError::Unsupported`] when no X display can be reached.
    pub fn new() -> Result<Self, HotkeyError> {
        let (connection, screen) =
            RustConnection::connect(None).map_err(|_| HotkeyError::Unsupported)?;
        let root = connection
            .setup()
            .roots
            .get(screen)
            .map(|root| root.root)
            .ok_or(HotkeyError::Unsupported)?;
        let keycodes =
            keyboard_map(&connection).map_err(|error| HotkeyError::Backend(error.to_string()))?;
        Ok(Self {
            connection,
            root,
            keycodes,
            grabs: Vec::new(),
        })
    }

    fn resolve(&self, hotkey: Hotkey) -> Result<u8, HotkeyError> {
        self.keycodes
            .get(&key_to_keysym(hotkey.key))
            .copied()
            .ok_or_else(|| HotkeyError::UnmappedKey(hotkey.key.label()))
    }
}

fn keyboard_map(
    connection: &RustConnection,
) -> Result<HashMap<u32, u8>, x11rb::errors::ReplyOrIdError> {
    let setup = connection.setup();
    let min = u8::from(setup.min_keycode);
    let max = u8::from(setup.max_keycode);
    let count = max.saturating_sub(min).saturating_add(1);
    if count == 0 {
        return Ok(HashMap::new());
    }
    let reply = connection.get_keyboard_mapping(min, count)?.reply()?;
    let width = reply.keysyms.len() / usize::from(count);
    let mut map = HashMap::new();
    for (offset, chunk) in reply.keysyms.chunks(width.max(1)).enumerate() {
        let keycode = min + offset as u8;
        for keysym in chunk {
            map.entry(*keysym).or_insert(keycode);
        }
    }
    Ok(map)
}

impl HotkeyRegistry for X11Hotkeys {
    fn register(&mut self, binding: HotkeyBinding) -> Result<(), HotkeyError> {
        let keycode = self.resolve(binding.hotkey)?;
        if self.grabs.iter().any(|grab| grab.hotkey == binding.hotkey) {
            return Err(HotkeyError::AlreadyRegistered(binding.hotkey));
        }
        let base = modifier_mask(binding.hotkey);
        for lock in lock_variants() {
            let mask = base | lock;
            self.connection
                .grab_key(
                    false,
                    self.root,
                    mask,
                    keycode,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .map_err(|error| HotkeyError::Backend(error.to_string()))?;
        }
        let _ = self.connection.flush();
        self.grabs.push(Grab {
            keycode,
            hotkey: binding.hotkey,
            action: binding.action,
        });
        Ok(())
    }

    fn unregister(&mut self, action: &str) -> Result<(), HotkeyError> {
        if let Some(position) = self.grabs.iter().position(|grab| grab.action == action) {
            let grab = self.grabs.remove(position);
            let base = modifier_mask(grab.hotkey);
            for lock in lock_variants() {
                let mask = base | lock;
                self.connection
                    .ungrab_key(grab.keycode, self.root, mask)
                    .map_err(|error| HotkeyError::Backend(error.to_string()))?;
            }
            let _ = self.connection.flush();
        }
        Ok(())
    }

    fn poll(&mut self) -> Vec<HotkeyEvent> {
        let mut events = Vec::new();
        loop {
            match self.connection.poll_for_event() {
                Ok(Some(event)) => {
                    if let x11rb::protocol::Event::KeyPress(press) = event {
                        let state = u16::from(press.state)
                            & u16::from(
                                ModMask::SHIFT | ModMask::CONTROL | ModMask::M1 | ModMask::M4,
                            );
                        for grab in &self.grabs {
                            if grab.keycode == press.detail
                                && u16::from(modifier_mask(grab.hotkey)) == state
                            {
                                events.push(HotkeyEvent {
                                    action: grab.action.clone(),
                                    hotkey: grab.hotkey,
                                });
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        let _ = self.connection.flush();
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::KeyButMask;

    #[test]
    fn letters_map_to_uppercase_keysyms() {
        assert_eq!(key_to_keysym(Key::Char('a')), u32::from(b'A'));
        assert_eq!(key_to_keysym(Key::Char('Z')), u32::from(b'Z'));
        assert_eq!(key_to_keysym(Key::Char('7')), u32::from(b'7'));
    }

    #[test]
    fn function_keys_use_the_x11_range() {
        assert_eq!(key_to_keysym(Key::Function(1)), 0xFFBE);
        assert_eq!(key_to_keysym(Key::Function(24)), 0xFFBE + 23);
    }

    #[test]
    fn modifiers_map_to_the_expected_masks() {
        let hotkey = Hotkey::parse("ctrl+alt+shift+super+l").unwrap().unwrap();
        let mask = modifier_mask(hotkey);
        assert_ne!(mask & ModMask::CONTROL, ModMask::default());
        assert_ne!(mask & ModMask::SHIFT, ModMask::default());
        assert_ne!(mask & ModMask::M1, ModMask::default());
        assert_ne!(mask & ModMask::M4, ModMask::default());
    }

    #[test]
    fn lock_bits_are_masked_out_of_an_event_state() {
        let hotkey = Hotkey::parse("ctrl+l").unwrap().unwrap();
        let pressed = KeyButMask::CONTROL | KeyButMask::MOD2 | KeyButMask::LOCK;
        let state = u16::from(pressed)
            & u16::from(ModMask::SHIFT | ModMask::CONTROL | ModMask::M1 | ModMask::M4);
        assert_eq!(state, u16::from(modifier_mask(hotkey)));
    }
}
