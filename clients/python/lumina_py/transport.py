"""Platform transports for the Lumina newline-delimited JSON protocol.

The protocol needs a full-duplex byte stream that can be read and written from
two threads. Unix uses an ``AF_UNIX`` socket; Windows uses the named pipe from
the protocol specification. Both share the same buffering and line-length
rules so the client behaves identically on every platform.
"""

from __future__ import annotations

import os
import socket
import sys
from typing import Optional, Protocol

MAX_LINE_BYTES = 64 * 1024
"""Maximum request or event line, excluding the trailing newline."""

_READ_CHUNK = 64 * 1024


class FrameError(Exception):
    """A line was longer than the protocol permits or was not valid UTF-8."""


class Transport(Protocol):
    """The byte-stream operations the client needs."""

    def read_line(self) -> Optional[bytes]:
        """Return the next line without its newline, or ``None`` at EOF."""

    def write_line(self, line: bytes) -> None:
        """Write one line and append exactly one newline."""

    def close(self) -> None:
        """Close the underlying handle. Must be idempotent."""


class _BufferedStream:
    def __init__(self) -> None:
        self._buffer = bytearray()

    def _read_chunk(self, size: int) -> bytes:
        raise NotImplementedError

    def _write_all(self, data: bytes) -> None:
        raise NotImplementedError

    def _close(self) -> None:
        raise NotImplementedError

    def read_line(self) -> Optional[bytes]:
        while True:
            newline = self._buffer.find(b"\n")
            if newline >= 0:
                line = bytes(self._buffer[:newline])
                del self._buffer[: newline + 1]
                if line.endswith(b"\r"):
                    line = line[:-1]
                if len(line) > MAX_LINE_BYTES:
                    raise FrameError(
                        f"line exceeds the {MAX_LINE_BYTES}-byte limit"
                    )
                return line
            if len(self._buffer) > MAX_LINE_BYTES:
                raise FrameError(f"line exceeds the {MAX_LINE_BYTES}-byte limit")
            chunk = self._read_chunk(_READ_CHUNK)
            if not chunk:
                if not self._buffer:
                    return None
                line = bytes(self._buffer)
                self._buffer.clear()
                if line.endswith(b"\r"):
                    line = line[:-1]
                return line
            self._buffer.extend(chunk)

    def write_line(self, line: bytes) -> None:
        self._write_all(line + b"\n")

    def close(self) -> None:
        self._close()


class _UnixTransport(_BufferedStream):
    def __init__(self, path: str) -> None:
        super().__init__()
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            connection.connect(path)
        except BaseException:
            connection.close()
            raise
        self._socket = connection
        self._reader = connection.makefile("rb")

    def _read_chunk(self, size: int) -> bytes:
        return self._reader.read(size) or b""

    def _write_all(self, data: bytes) -> None:
        self._socket.sendall(data)

    def _close(self) -> None:
        try:
            self._socket.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        try:
            self._reader.close()
        finally:
            self._socket.close()


class _PipeTransport(_BufferedStream):
    def __init__(self, path: str) -> None:
        super().__init__()
        self._file = open(path, "r+b", buffering=0)

    def _read_chunk(self, size: int) -> bytes:
        return self._file.read(size) or b""

    def _write_all(self, data: bytes) -> None:
        view = memoryview(data)
        while view:
            written = self._file.write(view)
            if not written:
                raise OSError("named pipe accepted zero bytes")
            view = view[written:]

    def _close(self) -> None:
        try:
            import ctypes
            import msvcrt

            handle = msvcrt.get_osfhandle(self._file.fileno())
            ctypes.windll.kernel32.CancelIoEx(ctypes.c_void_p(handle), None)
        except BaseException:
            pass
        self._file.close()


def default_address() -> str:
    """Resolve the per-user control address for this platform."""
    if sys.platform == "win32":
        override = os.environ.get("LUMINA_PIPE")
        if override:
            return override
        return rf"\\.\pipe\lumina-{_windows_sid()}"
    override = os.environ.get("LUMINA_SOCKET")
    if override:
        return override
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if runtime:
        return os.path.join(runtime, "lumina.sock")
    return f"/tmp/lumina-{os.getuid()}.sock"


def connect(address: Optional[str] = None) -> Transport:
    """Open a transport to ``address`` or the platform default."""
    target = default_address() if address is None else address
    if sys.platform == "win32":
        return _PipeTransport(target)
    return _UnixTransport(target)


def _windows_sid() -> str:
    import ctypes
    from ctypes import wintypes

    advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

    token_query = 0x0008
    token_user = 1

    kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    kernel32.LocalFree.argtypes = [wintypes.HLOCAL]
    kernel32.LocalFree.restype = wintypes.HLOCAL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL

    advapi32.OpenProcessToken.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.HANDLE),
    ]
    advapi32.OpenProcessToken.restype = wintypes.BOOL
    advapi32.GetTokenInformation.argtypes = [
        wintypes.HANDLE,
        ctypes.c_int,
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
    ]
    advapi32.GetTokenInformation.restype = wintypes.BOOL
    advapi32.ConvertSidToStringSidW.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    advapi32.ConvertSidToStringSidW.restype = wintypes.BOOL

    token = wintypes.HANDLE()
    if not advapi32.OpenProcessToken(
        kernel32.GetCurrentProcess(), token_query, ctypes.byref(token)
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        needed = wintypes.DWORD()
        advapi32.GetTokenInformation(token, token_user, None, 0, ctypes.byref(needed))
        if not needed.value:
            raise ctypes.WinError(ctypes.get_last_error())
        buffer = ctypes.create_string_buffer(needed.value)
        if not advapi32.GetTokenInformation(
            token,
            token_user,
            ctypes.cast(buffer, ctypes.c_void_p),
            needed,
            ctypes.byref(needed),
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        sid = ctypes.cast(buffer, ctypes.POINTER(ctypes.c_void_p))[0]
        string_sid = ctypes.c_void_p()
        if not advapi32.ConvertSidToStringSidW(
            ctypes.c_void_p(sid), ctypes.byref(string_sid)
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            return ctypes.wstring_at(string_sid.value)
        finally:
            kernel32.LocalFree(string_sid)
    finally:
        kernel32.CloseHandle(token)
