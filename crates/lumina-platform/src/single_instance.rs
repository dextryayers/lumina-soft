//! One Lumina per user session, with a friendly activation hand-off.
//!
//! # Design
//!
//! A second launch must not create a second island, and it must not fail
//! silently either: the user double-clicked the application and expects the
//! existing one to appear. The first instance owns a named mutex on Windows or a
//! unix socket on Linux. A second instance writes a short message to a per-user
//! activation file and exits successfully; the first instance polls
//! [`InstanceGuard::take_activation`] and shows itself.
//!
//! The activation file is deliberately simple. A named pipe would avoid the
//! file, but it needs a server thread and a reconnect loop for a message that is
//! at most "show". A file is atomic on both platforms, survives the primary
//! being busy for a frame, and can be inspected when something goes wrong.

use std::path::PathBuf;

/// Why an instance could not become the primary one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyRunning;

impl std::fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another Lumina instance is already running")
    }
}

impl std::error::Error for AlreadyRunning {}

/// Stable FNV-1a hash so the mutex, socket, and activation file names match
/// between processes without leaking the user's arbitrary name into a path.
fn stable_hash(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn activation_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("lumina-{:016x}.activate", stable_hash(name)))
}

/// The activation file path for a session name.
///
/// Exposed for diagnostics; the file is an implementation detail and should not
/// be written by anything except [`notify_existing`].
#[must_use]
pub fn activation_file(name: &str) -> PathBuf {
    activation_path(name)
}

/// Write a message for the primary instance.
///
/// Returns `false` when the file could not be written, which normally means the
/// temp directory is read-only. Callers may ignore it: the second instance
/// still exits, and the user can use the tray or hotkey instead.
pub fn notify_existing(name: &str, message: &str) -> bool {
    std::fs::write(activation_path(name), message).is_ok()
}

/// True when a primary instance for `name` is currently running.
///
/// [`notify_existing`] always writes the activation file, so callers that are
/// not already sure an instance exists (the `--activate` command, for example)
/// must probe with this first.
#[must_use]
pub fn is_running(name: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_ACCESS_RIGHTS};

        /// `SYNCHRONIZE` from the platform SDK; the `windows` crate exposes it
        /// outside `Win32::Foundation` in this version.
        const SYNCHRONIZE: u32 = 0x0010_0000;

        let mut wide: Vec<u16> = format!("Local\\Lumina.{:016x}", stable_hash(name))
            .encode_utf16()
            .collect();
        wide.push(0);
        match unsafe {
            OpenMutexW(
                SYNCHRONIZATION_ACCESS_RIGHTS(SYNCHRONIZE),
                false,
                PCWSTR(wide.as_ptr()),
            )
        } {
            Ok(handle) => {
                let _ = unsafe { CloseHandle(handle) };
                true
            }
            Err(_) => false,
        }
    }

    #[cfg(all(unix, not(target_os = "windows")))]
    {
        use std::os::unix::net::UnixStream;

        let socket = std::env::temp_dir().join(format!("lumina-{:016x}.sock", stable_hash(name)));
        UnixStream::connect(socket).is_ok()
    }

    #[cfg(not(any(target_os = "windows", unix)))]
    {
        let _ = name;
        false
    }
}

/// The primary-instance handle.
///
/// Holding it keeps the lock held. Dropping it releases the mutex or socket and
/// removes the activation file.
pub struct InstanceGuard {
    name: String,
    activation: PathBuf,
    #[cfg(target_os = "windows")]
    handle: windows::Win32::Foundation::HANDLE,
    #[cfg(all(unix, not(target_os = "windows")))]
    listener: std::os::unix::net::UnixListener,
    #[cfg(all(unix, not(target_os = "windows")))]
    socket: PathBuf,
}

impl std::fmt::Debug for InstanceGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceGuard")
            .field("name", &self.name)
            .field("activation", &self.activation)
            .finish_non_exhaustive()
    }
}

/// Try to become the primary instance for `name`.
///
/// # Errors
///
/// Returns [`AlreadyRunning`] when another process holds the lock.
pub fn claim(name: &str) -> Result<InstanceGuard, AlreadyRunning> {
    let activation = activation_path(name);
    // A stale message from a previous session must not make the new primary
    // show itself immediately.
    let _ = std::fs::remove_file(&activation);

    #[cfg(target_os = "windows")]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};

        let mut wide: Vec<u16> = format!("Local\\Lumina.{:016x}", stable_hash(name))
            .encode_utf16()
            .collect();
        wide.push(0);
        let handle = unsafe {
            windows::Win32::System::Threading::CreateMutexW(None, true, PCWSTR(wide.as_ptr()))
        };
        match handle {
            Ok(handle) => {
                if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                    let _ = unsafe { CloseHandle(handle) };
                    return Err(AlreadyRunning);
                }
                Ok(InstanceGuard {
                    name: name.to_string(),
                    activation,
                    handle,
                })
            }
            Err(_) => Err(AlreadyRunning),
        }
    }

    #[cfg(all(unix, not(target_os = "windows")))]
    {
        use std::os::unix::net::{UnixListener, UnixStream};

        let socket = std::env::temp_dir().join(format!("lumina-{:016x}.sock", stable_hash(name)));
        match UnixListener::bind(&socket) {
            Ok(listener) => Ok(InstanceGuard {
                name: name.to_string(),
                activation,
                listener,
                socket,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                // Only remove a socket nobody is listening on. Unconditionally
                // unlinking would let a second process steal a live instance.
                if UnixStream::connect(&socket).is_ok() {
                    return Err(AlreadyRunning);
                }
                let _ = std::fs::remove_file(&socket);
                match UnixListener::bind(&socket) {
                    Ok(listener) => Ok(InstanceGuard {
                        name: name.to_string(),
                        activation,
                        listener,
                        socket,
                    }),
                    Err(_) => Err(AlreadyRunning),
                }
            }
            Err(_) => Err(AlreadyRunning),
        }
    }

    #[cfg(not(any(target_os = "windows", unix)))]
    {
        Ok(InstanceGuard {
            name: name.to_string(),
            activation,
        })
    }
}

impl InstanceGuard {
    /// The session name this guard claims.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Read and consume a pending activation message.
    ///
    /// Call once per frame or from the event loop. An empty message is treated
    /// as a plain "show" request.
    #[must_use]
    pub fn take_activation(&self) -> Option<String> {
        let message = std::fs::read_to_string(&self.activation).ok()?;
        let _ = std::fs::remove_file(&self.activation);
        Some(message)
    }

    /// True when a message is waiting, without consuming it.
    #[must_use]
    pub fn activation_pending(&self) -> bool {
        self.activation.is_file()
    }

    /// The underlying unix socket or activation file, for diagnostics.
    #[must_use]
    pub fn lock_path(&self) -> &std::path::Path {
        #[cfg(all(unix, not(target_os = "windows")))]
        {
            &self.socket
        }
        #[cfg(not(all(unix, not(target_os = "windows"))))]
        {
            &self.activation
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.activation);
        #[cfg(target_os = "windows")]
        {
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.handle) };
        }
        #[cfg(all(unix, not(target_os = "windows")))]
        {
            let _ = std::fs::remove_file(&self.socket);
            // Keep the field alive until the socket file is removed.
            let _ = &self.listener;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_name() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("lumina-test-{}-{nanos}", std::process::id())
    }

    #[test]
    fn the_hash_is_stable_and_name_dependent() {
        assert_eq!(stable_hash("lumina"), stable_hash("lumina"));
        assert_ne!(stable_hash("lumina"), stable_hash("Lumina"));
    }

    #[test]
    fn a_second_claim_is_rejected_and_released_on_drop() {
        let name = unique_name();
        let guard = claim(&name).expect("the first claim succeeds");
        assert_eq!(guard.name(), name);
        assert!(matches!(claim(&name), Err(AlreadyRunning)));
        drop(guard);
        let guard = claim(&name).expect("the lock is released");
        drop(guard);
    }

    #[test]
    fn activation_messages_round_trip() {
        let name = unique_name();
        let guard = claim(&name).expect("primary");
        assert!(!guard.activation_pending());
        assert!(notify_existing(&name, "show"));
        assert!(guard.activation_pending());
        assert_eq!(guard.take_activation().as_deref(), Some("show"));
        assert_eq!(guard.take_activation(), None);
        drop(guard);
    }

    #[test]
    fn a_running_instance_is_detectable() {
        let name = unique_name();
        assert!(!is_running(&name));
        let guard = claim(&name).expect("primary");
        assert!(is_running(&name));
        drop(guard);
        assert!(!is_running(&name));
    }

    #[test]
    fn a_new_primary_ignores_a_stale_message() {
        let name = unique_name();
        assert!(notify_existing(&name, "stale"));
        let guard = claim(&name).expect("primary");
        assert_eq!(guard.take_activation(), None);
        drop(guard);
    }
}
