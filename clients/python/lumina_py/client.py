"""Synchronous and asynchronous clients for the Lumina IPC protocol."""

from __future__ import annotations

import asyncio
import json
import queue
import threading
import time
from collections import deque
from typing import Any, Deque, Dict, Optional

from . import transport as transport_module
from .transport import FrameError

_MAX_BUFFERED_RESPONSES = 64


class LuminaError(Exception):
    """Base class for client-side failures."""


class RemoteError(LuminaError):
    """The server rejected a command."""

    def __init__(self, code: str, message: str, response: dict) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
        self.response = response


class ProtocolError(LuminaError):
    """The server sent a line that does not follow the protocol."""


class ConnectionClosed(LuminaError):
    """The connection ended before the operation completed."""


class _ReaderFailure:
    def __init__(self, error: BaseException) -> None:
        self.error = error


def _encode(message: dict) -> bytes:
    return json.dumps(message, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def _is_event(message: dict) -> bool:
    return "event" in message


class LuminaClient:
    """A blocking client with optional helper methods for every command.

    One reader thread parses incoming lines, so a command can time out without
    losing later events. Calls are serialized: use one client per thread.
    """

    def __init__(
        self,
        address: Optional[str] = None,
        *,
        timeout: Optional[float] = None,
        _transport: Optional[transport_module.Transport] = None,
    ) -> None:
        self._transport = (
            transport_module.connect(address) if _transport is None else _transport
        )
        self._timeout = timeout
        self._next_id = 0
        self._events: Deque[dict] = deque()
        self._responses: Dict[Any, dict] = {}
        self._messages: "queue.Queue[object]" = queue.Queue()
        self._failure: Optional[BaseException] = None
        self._closed = False
        self._call_lock = threading.Lock()
        self._reader = threading.Thread(
            target=self._read_loop, name="lumina-py-reader", daemon=True
        )
        self._reader.start()

    def call(
        self,
        command: str,
        params: Optional[dict] = None,
        *,
        timeout: Optional[float] = None,
    ) -> Any:
        """Send a command and return its data, raising :class:`RemoteError`."""
        if not isinstance(command, str) or not command:
            raise TypeError("command must be a non-empty string")
        if params is not None and not isinstance(params, dict):
            raise TypeError("params must be a dict")
        with self._call_lock:
            self._raise_failure()
            request_id = self._next_id + 1
            self._next_id = request_id
            request = {
                "id": request_id,
                "command": command,
                "params": {} if params is None else params,
            }
            self._transport.write_line(_encode(request))
            deadline = self._deadline(timeout)
            while True:
                message = self._next_message(deadline)
                if message is None:
                    raise TimeoutError(f"`{command}` did not respond in time")
                if _is_event(message):
                    self._events.append(message)
                    continue
                if message.get("id") != request_id:
                    self._store_response(message)
                    continue
                return self._result(message)

    def next_event(self, timeout: Optional[float] = None) -> Optional[dict]:
        """Return the next event, waiting up to ``timeout`` seconds."""
        if self._events:
            return self._events.popleft()
        deadline = self._deadline(timeout)
        while True:
            message = self._next_message(deadline)
            if message is None:
                return None
            if _is_event(message):
                return message
            self._store_response(message)

    def close(self) -> None:
        """Close the connection. Safe to call more than once."""
        if self._closed:
            return
        self._closed = True
        try:
            self._transport.close()
        except OSError:
            pass
        if threading.current_thread() is not self._reader:
            self._reader.join(timeout=1.0)

    def __enter__(self) -> "LuminaClient":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def status(self, *, timeout: Optional[float] = None) -> dict:
        return self.call("status", timeout=timeout)

    def show(
        self,
        widget: Optional[str] = None,
        duration_ms: Optional[int] = None,
        pin: Optional[bool] = None,
        *,
        timeout: Optional[float] = None,
    ) -> Any:
        return self.call(
            "show",
            _compact(widget=widget, duration_ms=duration_ms, pin=pin),
            timeout=timeout,
        )

    def hide(self, *, timeout: Optional[float] = None) -> Any:
        return self.call("hide", timeout=timeout)

    def collapse(self, *, timeout: Optional[float] = None) -> Any:
        return self.call("collapse", timeout=timeout)

    def notify(
        self,
        title: str,
        body: Optional[str] = None,
        icon: Optional[str] = None,
        accent: Optional[str] = None,
        duration_ms: Optional[int] = None,
        action: Optional[str] = None,
        *,
        timeout: Optional[float] = None,
    ) -> Any:
        return self.call(
            "notify",
            _compact(
                title=title,
                body=body,
                icon=icon,
                accent=accent,
                duration_ms=duration_ms,
                action=action,
            ),
            timeout=timeout,
        )

    def set_rgb(
        self,
        mode: str,
        color: Optional[str] = None,
        speed: Optional[float] = None,
        *,
        timeout: Optional[float] = None,
    ) -> Any:
        return self.call(
            "set_rgb",
            _compact(mode=mode, color=color, speed=speed),
            timeout=timeout,
        )

    def cycle_rgb(self, *, timeout: Optional[float] = None) -> Any:
        return self.call("cycle_rgb", timeout=timeout)

    def set_profile(self, name: str, *, timeout: Optional[float] = None) -> Any:
        return self.call("set_profile", {"name": name}, timeout=timeout)

    def get_config(
        self, section: Optional[str] = None, *, timeout: Optional[float] = None
    ) -> Any:
        return self.call("get_config", _compact(section=section), timeout=timeout)

    def set_config(
        self,
        path: str,
        value: Any,
        persist: Optional[bool] = None,
        *,
        timeout: Optional[float] = None,
    ) -> Any:
        return self.call(
            "set_config",
            {"path": path, "value": value, **_compact(persist=persist)},
            timeout=timeout,
        )

    def reload_config(self, *, timeout: Optional[float] = None) -> Any:
        return self.call("reload_config", timeout=timeout)

    def media(
        self,
        action: str,
        player: Optional[str] = None,
        position_ms: Optional[int] = None,
        volume: Optional[int] = None,
        *,
        timeout: Optional[float] = None,
    ) -> Any:
        return self.call(
            "media",
            _compact(
                action=action,
                player=player,
                position_ms=position_ms,
                volume=volume,
            ),
            timeout=timeout,
        )

    def player_list(self, *, timeout: Optional[float] = None) -> Any:
        return self.call("player_list", timeout=timeout)

    def shortcut(self, shortcut_id: str, *, timeout: Optional[float] = None) -> Any:
        return self.call("shortcut", {"id": shortcut_id}, timeout=timeout)

    def execute(
        self,
        command: str,
        args: Optional[list] = None,
        timeout_ms: Optional[int] = None,
        *,
        timeout: Optional[float] = None,
    ) -> Any:
        return self.call(
            "execute",
            _compact(command=command, args=args, timeout_ms=timeout_ms),
            timeout=timeout,
        )

    def spectrum(
        self, peak: Optional[bool] = None, *, timeout: Optional[float] = None
    ) -> Any:
        return self.call("spectrum", _compact(peak=peak), timeout=timeout)

    def geometry(
        self,
        top_half_width: float,
        bottom_half_width: float,
        height: float,
        corner_radius: float,
        *,
        timeout: Optional[float] = None,
    ) -> Any:
        return self.call(
            "geometry",
            {
                "top_half_width": top_half_width,
                "bottom_half_width": bottom_half_width,
                "height": height,
                "corner_radius": corner_radius,
            },
            timeout=timeout,
        )

    def debug_overlay(
        self, enabled: bool, *, timeout: Optional[float] = None
    ) -> Any:
        return self.call("debug_overlay", {"enabled": enabled}, timeout=timeout)

    def quit(self, *, timeout: Optional[float] = None) -> Any:
        return self.call("quit", timeout=timeout)

    def _deadline(self, timeout: Optional[float]) -> Optional[float]:
        effective = self._timeout if timeout is None else timeout
        if effective is None:
            return None
        return time.monotonic() + effective

    def _remaining(self, deadline: Optional[float]) -> Optional[float]:
        if deadline is None:
            return None
        return max(0.0, deadline - time.monotonic())

    def _next_message(self, deadline: Optional[float]) -> Optional[dict]:
        self._raise_failure()
        remaining = self._remaining(deadline)
        if remaining is not None and remaining <= 0:
            return None
        try:
            item = self._messages.get(timeout=remaining)
        except queue.Empty:
            return None
        if isinstance(item, _ReaderFailure):
            self._failure = item.error
            raise item.error
        return item

    def _raise_failure(self) -> None:
        if self._closed:
            raise ConnectionClosed("client is closed")
        if self._failure is not None:
            raise self._failure

    def _store_response(self, response: dict) -> None:
        if len(self._responses) >= _MAX_BUFFERED_RESPONSES:
            self._responses.pop(next(iter(self._responses)))
        self._responses[response.get("id")] = response

    def _result(self, response: dict) -> Any:
        ok = response.get("ok")
        if not isinstance(ok, bool):
            raise ProtocolError("response is missing a boolean `ok` field")
        if ok:
            return response.get("data")
        error = response.get("error")
        if not isinstance(error, dict):
            raise ProtocolError("error response is missing its `error` object")
        raise RemoteError(
            str(error.get("code", "Internal")),
            str(error.get("message", "")),
            response,
        )

    def _read_loop(self) -> None:
        try:
            while True:
                line = self._transport.read_line()
                if line is None:
                    raise ConnectionClosed("server closed the connection")
                try:
                    text = line.decode("utf-8")
                except UnicodeDecodeError as error:
                    raise ProtocolError("line is not valid UTF-8") from error
                try:
                    message = json.loads(text)
                except json.JSONDecodeError as error:
                    raise ProtocolError(f"invalid JSON: {error}") from error
                if not isinstance(message, dict):
                    raise ProtocolError("each line must be a JSON object")
                self._messages.put(message)
        except FrameError as error:
            self._messages.put(_ReaderFailure(ProtocolError(str(error))))
        except BaseException as error:
            self._messages.put(_ReaderFailure(error))


class AsyncLuminaClient:
    """An asyncio wrapper around :class:`LuminaClient`.

    Blocking socket work runs in the default executor, so the event loop stays
    responsive. The synchronous client still serializes calls, which means an
    ``await next_event()`` must not run at the same time as ``await call()``.
    """

    def __init__(
        self,
        address: Optional[str] = None,
        *,
        timeout: Optional[float] = None,
        _transport: Optional[transport_module.Transport] = None,
    ) -> None:
        self._client = LuminaClient(
            address, timeout=timeout, _transport=_transport
        )

    @classmethod
    async def connect(
        cls,
        address: Optional[str] = None,
        *,
        timeout: Optional[float] = None,
    ) -> "AsyncLuminaClient":
        return await asyncio.to_thread(cls, address, timeout=timeout)

    async def call(
        self,
        command: str,
        params: Optional[dict] = None,
        *,
        timeout: Optional[float] = None,
    ) -> Any:
        return await asyncio.to_thread(
            self._client.call, command, params, timeout=timeout
        )

    async def next_event(self, timeout: Optional[float] = None) -> Optional[dict]:
        return await asyncio.to_thread(self._client.next_event, timeout)

    async def close(self) -> None:
        await asyncio.to_thread(self._client.close)

    async def __aenter__(self) -> "AsyncLuminaClient":
        return self

    async def __aexit__(self, *_exc: object) -> None:
        await self.close()


def _compact(**values: Any) -> dict:
    return {key: value for key, value in values.items() if value is not None}
