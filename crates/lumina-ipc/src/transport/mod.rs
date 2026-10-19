//! Per-user transport addresses and platform listeners.
//!
//! On Unix the address is `$XDG_RUNTIME_DIR/lumina.sock`, falling back to
//! `/tmp/lumina-$UID.sock` when the runtime directory is not set. On Windows it
//! is `\\.\pipe\lumina-<user-sid>`. Both names are derived from the current
//! user, so two users on the same machine never share a control socket.

use std::fmt;
use std::io::{self, Read, Write};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// A control socket address for the current platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcAddress {
    /// A Unix domain socket path.
    #[cfg(unix)]
    Unix(std::path::PathBuf),
    /// A Windows named pipe path.
    #[cfg(windows)]
    Pipe(String),
}

impl IpcAddress {
    /// The default per-user address.
    pub fn default_for_user() -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(unix::default_for_user())
        }
        #[cfg(windows)]
        {
            windows::default_for_user()
        }
    }

    /// Override with an explicit Unix socket path.
    #[cfg(unix)]
    pub fn unix(path: impl Into<std::path::PathBuf>) -> Self {
        Self::Unix(path.into())
    }

    /// Override with an explicit Windows pipe name.
    #[cfg(windows)]
    pub fn pipe(name: impl Into<String>) -> Self {
        Self::Pipe(name.into())
    }

    /// The address as a string suitable for logs and CLI flags.
    pub fn as_str(&self) -> &str {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => path.to_str().unwrap_or("<non-unicode socket path>"),
            #[cfg(windows)]
            Self::Pipe(name) => name,
        }
    }
}

impl fmt::Display for IpcAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A bidirectional byte stream.
pub trait Connection: Read + Write + Send + fmt::Debug {
    /// Duplicate the handle so reading and writing can happen on separate
    /// threads.
    fn try_clone_box(&self) -> io::Result<Box<dyn Connection>>;
}

/// A non-blocking accept source.
pub trait Listener: Send + fmt::Debug {
    /// Accept one pending connection.
    ///
    /// `Ok(None)` means no client is waiting; callers should retry. The
    /// implementation never blocks indefinitely, which lets the server observe
    /// shutdown requests promptly.
    fn accept(&mut self) -> io::Result<Option<Box<dyn Connection>>>;

    /// The address this listener is bound to.
    fn address(&self) -> &IpcAddress;
}

/// Bind a listener for `address`.
pub fn bind(address: &IpcAddress) -> io::Result<Box<dyn Listener>> {
    match address {
        #[cfg(unix)]
        IpcAddress::Unix(_) => unix::bind(address),
        #[cfg(windows)]
        IpcAddress::Pipe(_) => windows::bind(address),
    }
}

/// Connect to a running Lumina instance.
pub fn connect(address: &IpcAddress) -> io::Result<Box<dyn Connection>> {
    match address {
        #[cfg(unix)]
        IpcAddress::Unix(_) => unix::connect(address),
        #[cfg(windows)]
        IpcAddress::Pipe(_) => windows::connect(address),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_address_is_per_user() {
        let address = IpcAddress::default_for_user().unwrap();
        #[cfg(unix)]
        {
            assert!(address.as_str().ends_with("lumina.sock"), "{}", address);
            assert!(
                address.as_str().contains("lumina-") || address.as_str().contains("lumina.sock")
            );
        }
        #[cfg(windows)]
        {
            assert!(
                address.as_str().starts_with(r"\\.\pipe\lumina-S-1-5-"),
                "{}",
                address
            );
        }
    }

    #[test]
    fn display_matches_as_str() {
        let address = IpcAddress::default_for_user().unwrap();
        assert_eq!(address.to_string(), address.as_str());
        assert_ne!(address.to_string().len(), 0);
    }

    #[test]
    fn an_explicit_address_round_trips() {
        #[cfg(unix)]
        {
            let address = IpcAddress::unix("/tmp/lumina-test.sock");
            assert_eq!(address.as_str(), "/tmp/lumina-test.sock");
        }
        #[cfg(windows)]
        {
            let address = IpcAddress::pipe(r"\\.\pipe\lumina-test");
            assert_eq!(address.as_str(), r"\\.\pipe\lumina-test");
        }
    }
}
