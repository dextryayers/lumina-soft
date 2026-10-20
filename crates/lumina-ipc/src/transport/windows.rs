//! Windows named pipe transport.
//!
//! The pipe name includes the current user's SID and the security descriptor
//! grants full control to that SID only. A DACL is attached explicitly instead
//! of relying on the default, so a permissive machine-wide policy cannot open
//! the control surface to other users.

use std::ffi::{c_void, OsStr};
use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use windows::core::{HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    LocalFree, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING,
    FALSE, HANDLE, HLOCAL,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, SetNamedPipeHandleState, PIPE_NOWAIT, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{Connection, IpcAddress, Listener};

const SDDL: &str = "D:P(A;;GA;;;OW)";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

/// Build the default pipe name for this user.
pub(super) fn default_for_user() -> io::Result<IpcAddress> {
    let sid = user_sid()?;
    Ok(IpcAddress::Pipe(format!(r"\\.\pipe\lumina-{sid}")))
}

pub(super) fn bind(address: &IpcAddress) -> io::Result<Box<dyn Listener>> {
    let IpcAddress::Pipe(name) = address;
    Ok(Box::new(PipeListener {
        name: name.clone(),
        address: address.clone(),
        security: SecurityDescriptor::new(SDDL)?,
        pending: None,
    }))
}

pub(super) fn connect(address: &IpcAddress) -> io::Result<Box<dyn Connection>> {
    let IpcAddress::Pipe(name) = address;
    let wide = wide_string(name);
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                (windows::Win32::Foundation::GENERIC_READ
                    | windows::Win32::Foundation::GENERIC_WRITE)
                    .0,
                windows::Win32::Storage::FileSystem::FILE_SHARE_READ
                    | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        } {
            Ok(handle) => {
                let pipe = unsafe { OwnedHandle::from_raw_handle(handle.0) };
                set_wait_mode(&pipe)?;
                return Ok(Box::new(PipeConnection(File::from(pipe))));
            }
            Err(error)
                if (error.code() == HRESULT::from_win32(ERROR_PIPE_BUSY.0)
                    || error.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0))
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(io::Error::from(error)),
        }
    }
}

fn set_wait_mode(pipe: &OwnedHandle) -> io::Result<()> {
    let mode = PIPE_READMODE_BYTE | PIPE_WAIT;
    unsafe {
        SetNamedPipeHandleState(
            HANDLE(pipe.as_raw_handle()),
            Some(&raw const mode),
            None,
            None,
        )
    }
    .map_err(io::Error::from)
}

fn wide_string(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(Some(0)).collect()
}

fn user_sid() -> io::Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token)
            .map_err(io::Error::from)?;
        let token = OwnedHandle::from_raw_handle(token.0);

        let mut needed = 0u32;
        let _ = GetTokenInformation(
            HANDLE(token.as_raw_handle()),
            TokenUser,
            None,
            0,
            &raw mut needed,
        );
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let words = (needed as usize).div_ceil(size_of::<usize>());
        let mut buffer = vec![0usize; words];
        GetTokenInformation(
            HANDLE(token.as_raw_handle()),
            TokenUser,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            needed,
            &raw mut needed,
        )
        .map_err(io::Error::from)?;

        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid_string = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &raw mut sid_string).map_err(io::Error::from)?;
        let text = sid_string
            .to_string()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "SID is not UTF-16"))?;
        let _ = LocalFree(Some(HLOCAL(sid_string.0.cast())));
        Ok(text)
    }
}

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

unsafe impl Send for SecurityDescriptor {}

impl SecurityDescriptor {
    fn new(sddl: &str) -> io::Result<Self> {
        let wide = wide_string(sddl);
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                1,
                &raw mut descriptor,
                None,
            )
        }
        .map_err(io::Error::from)?;
        Ok(Self(descriptor))
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
            lpSecurityDescriptor: self.0 .0,
            bInheritHandle: FALSE,
        }
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        let _ = unsafe { LocalFree(Some(HLOCAL(self.0 .0))) };
    }
}

impl std::fmt::Debug for SecurityDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecurityDescriptor")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct PipeListener {
    name: String,
    address: IpcAddress,
    security: SecurityDescriptor,
    pending: Option<OwnedHandle>,
}

impl PipeListener {
    fn create_instance(&self) -> io::Result<OwnedHandle> {
        let wide = wide_string(&self.name);
        let attributes = self.security.attributes();
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                crate::protocol::MAX_CLIENTS as u32,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                0,
                Some(&raw const attributes),
            )
        };
        if handle.is_invalid() {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
    }
}

impl Listener for PipeListener {
    fn accept(&mut self) -> io::Result<Option<Box<dyn Connection>>> {
        let pipe = match self.pending.take() {
            Some(pipe) => pipe,
            None => self.create_instance()?,
        };
        let connected = match unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) } {
            Ok(()) => true,
            Err(error) if error.code() == HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) => true,
            Err(error) if error.code() == HRESULT::from_win32(ERROR_PIPE_LISTENING.0) => false,
            Err(error) => return Err(io::Error::from(error)),
        };
        if !connected {
            self.pending = Some(pipe);
            return Ok(None);
        }
        set_wait_mode(&pipe)?;
        Ok(Some(Box::new(PipeConnection(File::from(pipe)))))
    }

    fn address(&self) -> &IpcAddress {
        &self.address
    }
}

#[derive(Debug)]
struct PipeConnection(File);

impl Read for PipeConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for PipeConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Connection for PipeConnection {
    fn try_clone_box(&self) -> io::Result<Box<dyn Connection>> {
        Ok(Box::new(PipeConnection(self.0.try_clone()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;

    fn unique_pipe(tag: &str) -> IpcAddress {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        IpcAddress::Pipe(format!(
            r"\\.\pipe\lumina-test-{}-{tag}-{counter}",
            std::process::id()
        ))
    }

    #[test]
    fn the_default_pipe_is_scoped_to_the_user_sid() {
        let address = default_for_user().unwrap();
        assert!(address.as_str().starts_with(r"\\.\pipe\lumina-S-1-5-"));
        assert!(address.as_str().len() > r"\\.\pipe\lumina-S-1-5-".len());
    }

    #[test]
    fn a_pipe_round_trips_bytes() {
        let address = unique_pipe("roundtrip");
        let (sender, receiver) = mpsc::channel();
        let listener_address = address.clone();
        let thread = std::thread::spawn(move || {
            let mut listener = bind(&listener_address).unwrap();
            let connection = loop {
                match listener.accept() {
                    Ok(Some(connection)) => break connection,
                    Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            assert!(
                sender.send(connection).is_ok(),
                "receiver dropped before accepting"
            );
        });

        let mut client = connect(&address).unwrap();
        client.write_all(b"ping\n").unwrap();
        client.flush().unwrap();

        let mut server = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("server did not accept");
        let mut buffer = [0; 5];
        server.read_exact(&mut buffer).unwrap();
        assert_eq!(&buffer, b"ping\n");
        drop(client);
        thread.join().unwrap();
    }

    #[test]
    fn connecting_without_a_server_fails() {
        let address = unique_pipe("missing");
        let start = Instant::now();
        let error = connect(&address).unwrap_err();
        assert_ne!(error.to_string().len(), 0);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_connection_can_be_cloned_for_reading_and_writing() {
        let address = unique_pipe("clone");
        let (sender, receiver) = mpsc::channel();
        let listener_address = address.clone();
        let thread = std::thread::spawn(move || {
            let mut listener = bind(&listener_address).unwrap();
            let connection = loop {
                match listener.accept() {
                    Ok(Some(connection)) => break connection,
                    Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            assert!(
                sender.send(connection).is_ok(),
                "receiver dropped before accepting"
            );
        });
        let client = connect(&address).unwrap();
        let _clone = client.try_clone_box().unwrap();
        let _server = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("server did not accept");
        thread.join().unwrap();
    }
}
