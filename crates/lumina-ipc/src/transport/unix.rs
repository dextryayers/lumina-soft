//! Unix domain socket transport.
//!
//! The socket lives in `$XDG_RUNTIME_DIR`, which the desktop session creates
//! with the right ownership. The fallback path in `/tmp` is made safe by
//! including the numeric uid and, after binding, forcing mode `0600` and
//! checking that the path is not a symlink.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener as StdListener, UnixStream};
use std::path::{Path, PathBuf};

use super::{Connection, IpcAddress, Listener};

/// Build the default socket address for this user.
pub(super) fn default_for_user() -> io::Result<IpcAddress> {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return Ok(IpcAddress::Unix(PathBuf::from(runtime).join("lumina.sock")));
        }
    }
    let uid = unsafe { libc::getuid() };
    Ok(IpcAddress::Unix(PathBuf::from(format!(
        "/tmp/lumina-{uid}.sock"
    ))))
}

/// Bind the socket, replacing a stale file left by a crashed instance.
pub(super) fn bind(address: &IpcAddress) -> io::Result<Box<dyn Listener>> {
    let IpcAddress::Unix(path) = address;
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("another Lumina instance is listening on {}", path.display()),
            ));
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            ));
        }
        fs::remove_file(path)?;
    }

    let listener = StdListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(Box::new(SocketListener {
        listener,
        address: address.clone(),
        path: path.clone(),
    }))
}

/// Connect to an existing socket.
pub(super) fn connect(address: &IpcAddress) -> io::Result<Box<dyn Connection>> {
    let IpcAddress::Unix(path) = address;
    let stream = UnixStream::connect(path)?;
    stream.set_nonblocking(false)?;
    Ok(Box::new(SocketConnection(stream)))
}

#[derive(Debug)]
struct SocketListener {
    listener: StdListener,
    address: IpcAddress,
    path: PathBuf,
}

impl Listener for SocketListener {
    fn accept(&mut self) -> io::Result<Option<Box<dyn Connection>>> {
        match self.listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                Ok(Some(Box::new(SocketConnection(stream))))
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn address(&self) -> &IpcAddress {
        &self.address
    }
}

impl Drop for SocketListener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug)]
struct SocketConnection(UnixStream);

impl Read for SocketConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for SocketConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Connection for SocketConnection {
    fn try_clone_box(&self) -> io::Result<Box<dyn Connection>> {
        Ok(Box::new(SocketConnection(self.0.try_clone()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "lumina-ipc-{}-{tag}-{nanos}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("lumina.sock")
    }

    #[test]
    fn a_socket_round_trips_bytes() {
        let path = unique_path("roundtrip");
        let address = IpcAddress::Unix(path.clone());
        let mut listener = bind(&address).unwrap();
        let mut client = connect(&address).unwrap();
        let mut server = loop {
            if let Some(connection) = listener.accept().unwrap() {
                break connection;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        client.write_all(b"ping\n").unwrap();
        client.flush().unwrap();
        let mut buffer = [0; 5];
        server.read_exact(&mut buffer).unwrap();
        assert_eq!(&buffer, b"ping\n");
    }

    #[test]
    fn the_socket_is_owner_only() {
        let path = unique_path("mode");
        let address = IpcAddress::Unix(path.clone());
        let _listener = bind(&address).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_stale_socket_file_is_replaced() {
        let path = unique_path("stale");
        let address = IpcAddress::Unix(path.clone());
        drop(bind(&address).unwrap());
        assert!(!path.exists(), "dropping the listener removes the socket");
        let listener = bind(&address);
        assert!(listener.is_ok());
    }

    #[test]
    fn binding_twice_reports_in_use() {
        let path = unique_path("in-use");
        let address = IpcAddress::Unix(path);
        let _listener = bind(&address).unwrap();
        let error = bind(&address).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    }

    #[test]
    fn connecting_to_a_missing_socket_fails() {
        let path = unique_path("missing").with_extension("nope");
        let error = connect(&IpcAddress::Unix(path)).unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
        ));
    }
}
