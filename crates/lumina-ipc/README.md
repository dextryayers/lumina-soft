# lumina-ipc

The control surface for Lumina: newline-delimited JSON over a per-user Unix
socket or Windows named pipe.

## Why a hand-written protocol

The contract is deliberately small and boring: one UTF-8 JSON object per line,
a request `id` echoed by exactly one response, and unsolicited event lines on the
same stream. No HTTP server, no protobuf schema, no port to expose. The
transport is a filesystem object owned by the current user, and the same parser
serves `lumina-send`, the Rust client, and the Python and Node clients.

Requests nest command fields under `params`:

```json
{"id":7,"command":"notify","params":{"title":"Build done"}}
```

Nesting is not cosmetic. `shortcut` has a field called `id`; a flat envelope
would collide with the request identifier. Validation rejects unknown top-level
fields with a message that points at `params`.

## Layers

| Module | Responsibility |
|---|---|
| `protocol` | Command parsing and validation, response envelopes, error codes, state snapshots |
| `framing` | 64 KiB line reader, CRLF stripping, UTF-8 checks, encoder |
| `rate_limit` | Per-client token bucket, 120 commands per second |
| `events` | Event types and fan-out to subscribers |
| `transport` | Address resolution, `0600` Unix socket, SID-scoped named pipe with an owner-only DACL |
| `server` | Accept loop, serialized dispatcher, per-client reader/writer, shutdown |
| `client` | Blocking client with response/event demultiplexing |

The server never runs handlers on connection threads. Each connection has a
reader and a writer, but every command goes through one dispatcher, so two
scripts racing to show a widget still produce one deterministic order. A bad
command is answered and leaves the connection open. Only framing violations
close it.

## Security

The Unix socket lives in the user's runtime directory and is chmod `0600`; a
live socket is never stolen, only a stale one is replaced. The Windows pipe is
created with an explicit DACL that grants the owner full control. `execute`
exists in the protocol but the application must gate it behind
`[advanced].ipc_allow_exec`; the library never runs a program on its own.

## Use

```rust
use lumina_ipc::transport::bind;
use lumina_ipc::{
    Client, Command, EventSender, IpcAddress, NotifyArgs, Server, ServerOptions,
};

let address = IpcAddress::default_for_user()?;
let listener = bind(&address)?;
let server = Server::new(listener, handler, ServerOptions::default());
let events = EventSender::new();
let handle = server.spawn(&events)?;

let mut client = Client::connect(handle.address())?;
client.call(&Command::Notify(NotifyArgs {
    title: "Build done".to_string(),
    body: Some("lumina 0.1.0-alpha".to_string()),
    icon: None,
    accent: None,
    duration_ms: Some(5000),
    action: None,
}))?;
```

`Handler` runs on the dispatcher thread and should return quickly. Long work
belongs on another thread; the whole control surface waits for the current
command.

The full wire reference, including command fields, error codes, event payloads,
and client examples, is in [`docs/ipc.md`](../../docs/ipc.md).
