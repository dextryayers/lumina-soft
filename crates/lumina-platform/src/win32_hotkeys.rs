//! Win32 global hotkeys through `RegisterHotKey`.
//!
//! # Threading
//!
//! `RegisterHotKey` delivers `WM_HOTKEY` to the thread that registered the
//! key, so the backend owns a dedicated thread with a message loop. Commands
//! arrive over a channel and wake the loop with `PostThreadMessage`; events go
//! back over another channel and are drained by [`HotkeyRegistry::poll`].
//!
//! # Failure modes
//!
//! A combination claimed by another application, or by another Lumina binding,
//! is reported as [`HotkeyError::AlreadyRegistered`] instead of being ignored.
//! Keys without a virtual-key mapping are rejected before reaching Win32.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_HOTKEY, WM_QUIT,
};

use crate::hotkey::{
    HotkeyBinding, HotkeyError, HotkeyEvent, HotkeyRegistry,
};

/// A private thread message used to wake the loop after a command is queued.
const WM_LUMINA_COMMAND: u32 = 0x8000 + 1;

enum Command {
    Register(HotkeyBinding, Sender<Result<(), HotkeyError>>),
    Unregister(String, Sender<Result<(), HotkeyError>>),
    Stop,
}

/// The Win32 hotkey backend.
pub struct Win32Hotkeys {
    commands: Sender<Command>,
    events: Receiver<HotkeyEvent>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Win32Hotkeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Win32Hotkeys")
            .field("thread_id", &self.thread_id)
            .finish_non_exhaustive()
    }
}

impl Win32Hotkeys {
    /// Start the backend thread.
    ///
    /// # Errors
    ///
    /// Returns [`HotkeyError::Backend`] when the thread cannot report its id.
    pub fn new() -> Result<Self, HotkeyError> {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let (id_tx, id_rx) = mpsc::channel();

        let thread = thread::Builder::new()
            .name("lumina-hotkeys".to_string())
            .spawn(move || run_loop(command_rx, event_tx, id_tx))
            .map_err(|error| HotkeyError::Backend(error.to_string()))?;

        let thread_id = id_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| HotkeyError::Backend("the hotkey thread did not start".to_string()))?;

        Ok(Self {
            commands: command_tx,
            events: event_rx,
            thread_id,
            thread: Some(thread),
        })
    }

    fn send(&self, command: Command) -> Result<(), HotkeyError> {
        self.commands
            .send(command)
            .map_err(|_| HotkeyError::Backend("the hotkey thread stopped".to_string()))?;
        unsafe {
            PostThreadMessageW(
                self.thread_id,
                WM_LUMINA_COMMAND,
                WPARAM(0),
                LPARAM(0),
            )
        }
        .map_err(|error| HotkeyError::Backend(error.to_string()))
    }
}

impl HotkeyRegistry for Win32Hotkeys {
    fn register(&mut self, binding: HotkeyBinding) -> Result<(), HotkeyError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.send(Command::Register(binding, reply_tx))?;
        reply_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| HotkeyError::Backend("the hotkey thread did not reply".to_string()))?
    }

    fn unregister(&mut self, action: &str) -> Result<(), HotkeyError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.send(Command::Unregister(action.to_string(), reply_tx))?;
        reply_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| HotkeyError::Backend("the hotkey thread did not reply".to_string()))?
    }

    fn poll(&mut self) -> Vec<HotkeyEvent> {
        let mut events = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        events
    }
}

impl Drop for Win32Hotkeys {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        let _ = unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0))
        };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_loop(
    commands: Receiver<Command>,
    events: Sender<HotkeyEvent>,
    id_tx: Sender<u32>,
) {
    unsafe {
        // Force the thread message queue to exist before publishing the id.
        // Without this, a very fast drop can post WM_QUIT before the queue
        // exists, lose it, and then block forever in join.
        let mut message = MSG::default();
        let _ = PeekMessageW(&mut message, HWND::default(), 0, 0, PM_NOREMOVE);
        let _ = id_tx.send(GetCurrentThreadId());
    }

    let mut registered: Vec<(i32, HotkeyBinding)> = Vec::new();
    let mut next_id: i32 = 1;

    loop {
        let mut message = MSG::default();
        let result = unsafe { GetMessageW(&mut message, HWND::default(), 0, 0) };
        if result.0 <= 0 {
            break;
        }

        if message.message == WM_HOTKEY {
            let id = message.wParam.0 as i32;
            if let Some((_, binding)) = registered.iter().find(|(key_id, _)| *key_id == id) {
                let _ = events.send(HotkeyEvent {
                    action: binding.action.clone(),
                    hotkey: binding.hotkey,
                });
            }
        }

        loop {
            match commands.try_recv() {
                Ok(Command::Register(binding, reply)) => {
                    let result = register_one(&mut registered, &mut next_id, binding);
                    let _ = reply.send(result);
                }
                Ok(Command::Unregister(action, reply)) => {
                    if let Some(index) = registered
                        .iter()
                        .position(|(_, binding)| binding.action == action)
                    {
                        let (id, _) = registered.remove(index);
                        unsafe {
                            let _ = UnregisterHotKey(HWND::default(), id);
                        }
                    }
                    let _ = reply.send(Ok(()));
                }
                Ok(Command::Stop) => {
                    for (id, _) in registered.drain(..) {
                        unsafe {
                            let _ = UnregisterHotKey(HWND::default(), id);
                        }
                    }
                    return;
                }
                Err(_) => break,
            }
        }
    }

    for (id, _) in registered.drain(..) {
        unsafe {
            let _ = UnregisterHotKey(HWND::default(), id);
        }
    }
}

fn register_one(
    registered: &mut Vec<(i32, HotkeyBinding)>,
    next_id: &mut i32,
    binding: HotkeyBinding,
) -> Result<(), HotkeyError> {
    if registered
        .iter()
        .any(|(_, existing)| existing.hotkey == binding.hotkey)
    {
        return Err(HotkeyError::AlreadyRegistered(binding.hotkey));
    }
    let Some(vk) = binding.hotkey.key.windows_virtual_key() else {
        return Err(HotkeyError::UnmappedKey(binding.hotkey.key.label()));
    };
    let id = *next_id;
    *next_id += 1;
    let flags = HOT_KEY_MODIFIERS(binding.hotkey.modifiers.windows_flags());
    match unsafe { RegisterHotKey(HWND::default(), id, flags, vk) } {
        Ok(()) => {
            registered.push((id, binding));
            Ok(())
        }
        Err(_) => Err(HotkeyError::AlreadyRegistered(binding.hotkey)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::{Hotkey, Key, Modifiers};

    #[test]
    fn a_backend_can_be_created_and_dropped() {
        let backend = Win32Hotkeys::new();
        assert!(backend.is_ok(), "the hotkey thread starts without a window");
    }

    #[test]
    fn unmapped_keys_are_rejected_without_touching_win32() {
        let mut registered = Vec::new();
        let mut next_id = 1;
        let binding = HotkeyBinding {
            action: "test".to_string(),
            hotkey: Hotkey {
                modifiers: Modifiers::NONE,
                key: Key::Char('!'),
            },
        };
        let result = register_one(&mut registered, &mut next_id, binding);
        assert!(matches!(result, Err(HotkeyError::UnmappedKey(_))));
        assert!(registered.is_empty());
    }

    #[test]
    fn duplicate_combinations_are_rejected_before_win32() {
        let mut registered = vec![(
            1,
            HotkeyBinding {
                action: "first".to_string(),
                hotkey: Hotkey::parse("ctrl+alt+l").unwrap().unwrap(),
            },
        )];
        let mut next_id = 2;
        let result = register_one(
            &mut registered,
            &mut next_id,
            HotkeyBinding {
                action: "second".to_string(),
                hotkey: Hotkey::parse("ctrl+alt+l").unwrap().unwrap(),
            },
        );
        assert!(matches!(result, Err(HotkeyError::AlreadyRegistered(_))));
        assert_eq!(registered.len(), 1);
    }
}