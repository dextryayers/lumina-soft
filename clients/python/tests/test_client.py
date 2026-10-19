import asyncio
import json
import os
import queue
import sys
import unittest
from unittest import mock

from lumina_py import (
    AsyncLuminaClient,
    ConnectionClosed,
    FrameError,
    LuminaClient,
    ProtocolError,
    RemoteError,
    default_address,
)
from lumina_py.transport import MAX_LINE_BYTES, _BufferedStream


class FakeTransport:
    def __init__(self, responder=None):
        self.responder = responder
        self.sent = []
        self.incoming = queue.Queue()
        self.closed = False

    def write_line(self, line):
        request = json.loads(line)
        self.sent.append(request)
        if self.responder is None:
            return
        response = self.responder(request)
        if response is None:
            return
        messages = response if isinstance(response, list) else [response]
        for message in messages:
            self.incoming.put((json.dumps(message) + "\n").encode("utf-8"))

    def read_line(self):
        item = self.incoming.get()
        if item is None:
            return None
        if isinstance(item, BaseException):
            raise item
        return item

    def close(self):
        self.closed = True
        self.incoming.put(None)


class ChunkedStream(_BufferedStream):
    def __init__(self, chunks):
        super().__init__()
        self.chunks = list(chunks)
        self.written = bytearray()

    def _read_chunk(self, size):
        return self.chunks.pop(0) if self.chunks else b""

    def _write_all(self, data):
        self.written.extend(data)

    def _close(self):
        pass


class ClientTests(unittest.TestCase):
    def test_a_call_uses_the_nested_params_envelope(self):
        transport = FakeTransport(
            lambda request: {
                "id": request["id"],
                "ok": True,
                "data": {"state": "collapsed"},
            }
        )
        with LuminaClient(_transport=transport) as client:
            status = client.status()
        self.assertEqual(status, {"state": "collapsed"})
        self.assertEqual(len(transport.sent), 1)
        request = transport.sent[0]
        self.assertEqual(request["command"], "status")
        self.assertEqual(request["params"], {})
        self.assertIsInstance(request["id"], int)

    def test_notify_omits_empty_fields(self):
        transport = FakeTransport(
            lambda request: {"id": request["id"], "ok": True, "data": "n-1"}
        )
        client = LuminaClient(_transport=transport)
        client.notify("Build done", body="ok", accent="#34c759")
        self.assertEqual(
            transport.sent[0]["params"],
            {"title": "Build done", "body": "ok", "accent": "#34c759"},
        )
        client.close()

    def test_events_arriving_before_a_response_are_buffered(self):
        def responder(request):
            return [
                {"event": "state_changed", "data": {"state": "expanded"}},
                {"id": request["id"], "ok": True, "data": "ok"},
            ]

        transport = FakeTransport(responder)
        with LuminaClient(_transport=transport) as client:
            self.assertEqual(client.hide(), "ok")
            event = client.next_event(timeout=1.0)
        self.assertEqual(event, {"event": "state_changed", "data": {"state": "expanded"}})

    def test_remote_errors_expose_the_code(self):
        transport = FakeTransport(
            lambda request: {
                "id": request["id"],
                "ok": False,
                "error": {"code": "UnknownProfile", "message": "no such profile"},
            }
        )
        with LuminaClient(_transport=transport) as client:
            with self.assertRaises(RemoteError) as raised:
                client.set_profile("missing")
        self.assertEqual(raised.exception.code, "UnknownProfile")
        self.assertIn("no such profile", str(raised.exception))

    def test_a_call_can_time_out_without_losing_the_reader(self):
        transport = FakeTransport(None)
        client = LuminaClient(_transport=transport)
        with self.assertRaises(TimeoutError):
            client.call("status", timeout=0.02)
        self.assertIsNone(client.next_event(timeout=0.02))
        client.close()
        self.assertTrue(transport.closed)

    def test_calls_after_close_are_rejected(self):
        transport = FakeTransport(None)
        client = LuminaClient(_transport=transport)
        client.close()
        with self.assertRaises(ConnectionClosed):
            client.call("status", timeout=0.02)

    def test_async_client_delegates_without_blocking_the_loop(self):
        transport = FakeTransport(
            lambda request: {"id": request["id"], "ok": True, "data": "pong"}
        )

        async def scenario():
            async with AsyncLuminaClient(_transport=transport) as client:
                return await client.call("status")

        self.assertEqual(asyncio.run(scenario()), "pong")

    def test_invalid_utf8_is_a_protocol_error(self):
        transport = FakeTransport(None)
        transport.incoming.put(b"\xff\xfe\n")
        with LuminaClient(_transport=transport) as client:
            with self.assertRaises(ProtocolError):
                client.call("status", timeout=0.2)


class FramingTests(unittest.TestCase):
    def test_crlf_is_stripped(self):
        stream = ChunkedStream([b'{"a":1}\r\n'])
        self.assertEqual(stream.read_line(), b'{"a":1}')

    def test_a_trailing_line_without_a_newline_is_returned(self):
        stream = ChunkedStream([b'{"a":1}'])
        self.assertEqual(stream.read_line(), b'{"a":1}')
        self.assertIsNone(stream.read_line())

    def test_an_overlong_line_is_rejected(self):
        stream = ChunkedStream([b"x" * (MAX_LINE_BYTES + 1)])
        with self.assertRaises(FrameError):
            stream.read_line()

    def test_write_line_appends_exactly_one_newline(self):
        stream = ChunkedStream([])
        stream.write_line(b"{}")
        self.assertEqual(bytes(stream.written), b"{}\n")


class AddressTests(unittest.TestCase):
    @unittest.skipIf(sys.platform == "win32", "unix address")
    def test_xdg_runtime_dir_wins(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("LUMINA_SOCKET", None)
            os.environ["XDG_RUNTIME_DIR"] = "/run/user/1000"
            self.assertEqual(default_address(), "/run/user/1000/lumina.sock")

    @unittest.skipIf(sys.platform == "win32", "unix address")
    def test_the_socket_override_wins(self):
        with mock.patch.dict(os.environ, {"LUMINA_SOCKET": "/tmp/custom.sock"}):
            self.assertEqual(default_address(), "/tmp/custom.sock")

    @unittest.skipUnless(sys.platform == "win32", "windows address")
    def test_the_pipe_override_wins(self):
        with mock.patch.dict(
            os.environ, {"LUMINA_PIPE": r"\\.\pipe\lumina-test"}
        ):
            self.assertEqual(default_address(), r"\\.\pipe\lumina-test")


if __name__ == "__main__":
    unittest.main()
