//! Windows named pipe transport.
//!
//! The pipe name includes the current user's SID and the security descriptor
//! grants full control to that SID only. A DACL is attached explicitly instead
//! of relying on the default, so a permissive machine-wide policy cannot open
//! the control surface to other users.
//!
//! Every pipe handle is opened for overlapped I/O. Background events are
//! written by a different thread than the one reading requests, and Windows
//! serializes blocking I/O on the same file object. Overlapped operations keep
//! a pending read from stalling a write, which is required for the push event
//! channel and for responses on a connection that is waiting for its next
//! request.

use std::ffi::{c_void, OsStr};
use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use windows::core::{HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    LocalFree, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, FALSE, HANDLE, HLOCAL,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

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
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                None,
            )
        } {
            Ok(handle) => {
                let pipe = unsafe { OwnedHandle::from_raw_handle(handle.0) };
                return Ok(Box::new(PipeConnection::new(pipe)));
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

fn create_event() -> windows::core::Result<OwnedHandle> {
    let handle = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }?;
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}

fn is_pending(error: &windows::core::Error) -> bool {
    error.code() == HRESULT::from_win32(ERROR_IO_PENDING.0)
}

fn is_disconnect(error: &windows::core::Error) -> bool {
    let code = error.code();
    code == HRESULT::from_win32(ERROR_BROKEN_PIPE.0)
        || code == HRESULT::from_win32(ERROR_PIPE_NOT_CONNECTED.0)
        || code == HRESULT::from_win32(ERROR_NO_DATA.0)
}

fn run_overlapped(
    handle: HANDLE,
    operation: impl FnOnce(&mut OVERLAPPED) -> windows::core::Result<()>,
) -> windows::core::Result<u32> {
    let event = create_event()?;
    let mut overlapped = OVERLAPPED {
        hEvent: HANDLE(event.as_raw_handle()),
        ..Default::default()
    };
    match operation(&mut overlapped) {
        Ok(()) => {}
        Err(error) if is_pending(&error) => {}
        Err(error) => return Err(error),
    }
    let mut transferred = 0u32;
    unsafe { GetOverlappedResult(handle, &raw const overlapped, &raw mut transferred, true) }?;
    Ok(transferred)
}

struct PendingConnect {
    pipe: Option<OwnedHandle>,
    event: OwnedHandle,
    overlapped: Box<OVERLAPPED>,
}

impl std::fmt::Debug for PendingConnect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingConnect")
            .field("connected", &self.pipe.is_some())
            .finish_non_exhaustive()
    }
}

unsafe impl Send for PendingConnect {}

impl Drop for PendingConnect {
    fn drop(&mut self) {
        let Some(pipe) = self.pipe.as_ref() else {
            return;
        };
        let handle = HANDLE(pipe.as_raw_handle());
        let mut transferred = 0u32;
        unsafe {
            let _ = CancelIoEx(handle, Some(&raw const *self.overlapped));
            let _ = GetOverlappedResult(
                handle,
                &raw const *self.overlapped,
                &raw mut transferred,
                true,
            );
        }
    }
}

enum Connect {
    Pending(PendingConnect),
    Connected(OwnedHandle),
}

#[derive(Debug)]
struct PipeListener {
    name: String,
    address: IpcAddress,
    security: SecurityDescriptor,
    pending: Option<PendingConnect>,
}

impl PipeListener {
    fn create_instance(&self) -> io::Result<OwnedHandle> {
        let wide = wide_string(&self.name);
        let attributes = self.security.attributes();
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
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

    fn begin_connect(&self) -> io::Result<Connect> {
        let pipe = self.create_instance()?;
        let event = create_event().map_err(io::Error::from)?;
        let mut overlapped = Box::new(OVERLAPPED {
            hEvent: HANDLE(event.as_raw_handle()),
            ..Default::default()
        });
        let handle = HANDLE(pipe.as_raw_handle());
        match unsafe { ConnectNamedPipe(handle, Some(&raw mut *overlapped)) } {
            Ok(()) => Ok(Connect::Connected(pipe)),
            Err(error) if error.code() == HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) => {
                Ok(Connect::Connected(pipe))
            }
            Err(error) if is_pending(&error) => Ok(Connect::Pending(PendingConnect {
                pipe: Some(pipe),
                event,
                overlapped,
            })),
            Err(error) => Err(io::Error::from(error)),
        }
    }
}

impl Listener for PipeListener {
    fn accept(&mut self) -> io::Result<Option<Box<dyn Connection>>> {
        let mut pending = match self.pending.take() {
            Some(pending) => pending,
            None => match self.begin_connect()? {
                Connect::Connected(pipe) => {
                    return Ok(Some(Box::new(PipeConnection::new(pipe))));
                }
                Connect::Pending(pending) => pending,
            },
        };

        match unsafe { WaitForSingleObject(HANDLE(pending.event.as_raw_handle()), 0) } {
            WAIT_OBJECT_0 => {
                let pipe = pending
                    .pipe
                    .as_ref()
                    .expect("pending pipe handles are owned");
                let mut transferred = 0u32;
                unsafe {
                    GetOverlappedResult(
                        HANDLE(pipe.as_raw_handle()),
                        &raw const *pending.overlapped,
                        &raw mut transferred,
                        false,
                    )
                }
                .map_err(io::Error::from)?;
                let pipe = pending.pipe.take().expect("pending pipe handles are owned");
                Ok(Some(Box::new(PipeConnection::new(pipe))))
            }
            WAIT_TIMEOUT => {
                self.pending = Some(pending);
                Ok(None)
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn address(&self) -> &IpcAddress {
        &self.address
    }
}

#[derive(Debug)]
struct PipeConnection(File);

impl PipeConnection {
    fn new(handle: OwnedHandle) -> Self {
        Self(File::from(handle))
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }
}

impl Read for PipeConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let handle = self.handle();
        match run_overlapped(handle, |overlapped| unsafe {
            ReadFile(handle, Some(buffer), None, Some(overlapped))
        }) {
            Ok(transferred) => Ok(transferred as usize),
            Err(error) if is_disconnect(&error) => Ok(0),
            Err(error) => Err(io::Error::from(error)),
        }
    }
}

impl Write for PipeConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let handle = self.handle();
        run_overlapped(handle, |overlapped| unsafe {
            WriteFile(handle, Some(buffer), None, Some(overlapped))
        })
        .map(|transferred| transferred as usize)
        .map_err(io::Error::from)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
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

    fn accept_one(listener: &mut dyn Listener) -> Box<dyn Connection> {
        loop {
            match listener.accept() {
                Ok(Some(connection)) => return connection,
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(error) => panic!("accept failed: {error}"),
            }
        }
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
            let connection = accept_one(listener.as_mut());
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
            let connection = accept_one(listener.as_mut());
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

    #[test]
    fn writes_do_not_wait_for_a_pending_read() {
        let address = unique_pipe("duplex");
        let (ready_sender, ready_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let listener_address = address.clone();
        let thread = std::thread::spawn(move || {
            let mut listener = bind(&listener_address).unwrap();
            let connection = accept_one(listener.as_mut());
            let mut reader = connection.try_clone_box().unwrap();
            let mut writer = connection;
            let mut ping = [0u8; 5];
            reader.read_exact(&mut ping).unwrap();
            assert_eq!(&ping, b"ping\n");
            ready_sender.send(()).unwrap();
            std::thread::spawn(move || {
                let mut extra = [0u8; 1];
                let _ = reader.read_exact(&mut extra);
            });
            std::thread::sleep(Duration::from_millis(250));
            writer.write_all(b"pong\n").unwrap();
            writer.flush().unwrap();
            result_sender.send(true).unwrap();
        });

        let mut client = connect(&address).unwrap();
        let mut reader = client.try_clone_box().unwrap();
        client.write_all(b"ping\n").unwrap();
        client.flush().unwrap();
        ready_receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("the server did not read the request");
        let mut pong = [0u8; 5];
        reader.read_exact(&mut pong).unwrap();
        assert_eq!(&pong, b"pong\n");
        assert!(result_receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("the server did not write the reply"));
        thread.join().unwrap();
    }
}
