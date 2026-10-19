# lumina-py

Standard-library Python client for Lumina's newline-delimited JSON IPC. It has
no runtime dependencies and ships `py.typed`.

## Install

```sh
pip install ./clients/python
```

## Synchronous use

```python
from lumina_py import LuminaClient

with LuminaClient() as client:
    print(client.status()["state"])
    client.notify(
        "Build done",
        body="lumina 0.1.0-alpha",
        accent="#34c759",
        duration_ms=5000,
    )
```

Every command has a helper, and `call` accepts any command plus its `params`
object:

```python
client.call("set_config", {"path": "widgets.clock.show_seconds", "value": True})
```

## Events

A background reader keeps events that arrive while a command is in flight. Use
`next_event` to consume them; it returns `None` when the timeout expires.

```python
with LuminaClient() as client:
    while True:
        event = client.next_event(timeout=30)
        if event is None:
            continue
        print(event["event"], event["data"])
```

## Asynchronous use

`AsyncLuminaClient` moves the blocking transport into the default executor, so
the event loop is never blocked. Calls and `next_event` share one connection and
must not be awaited concurrently.

```python
import asyncio
from lumina_py import AsyncLuminaClient

async def main():
    async with AsyncLuminaClient() as client:
        status = await client.call("status")
        print(status["state"])

asyncio.run(main())
```

## Address

The default matches the Rust server: `$XDG_RUNTIME_DIR/lumina.sock` (or
`/tmp/lumina-$UID.sock`) on Linux and `\\.\pipe\lumina-<user-sid>` on Windows.
Set `LUMINA_SOCKET` or `LUMINA_PIPE` to override it, or pass `address=` to the
constructor.

## Errors

`RemoteError` carries the server's `code` and `message`. `ProtocolError` means
the stream violated framing or JSON rules, and `ConnectionClosed` means the
server went away. `call(..., timeout=)` raises `TimeoutError`; events that
arrive later are still readable.

## Tests

```sh
cd clients/python
python -m unittest discover -s tests
```
