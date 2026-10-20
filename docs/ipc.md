# IPC protocol

Lumina exposes one local control surface. Shell scripts, editor plugins, stream
decks, CI reporters, and test harnesses all use the same protocol, implemented
by `crates/lumina-ipc` and shipped with three clients: the `lumina-send` CLI,
`lumina-py`, and `lumina-node`.

The protocol is newline-delimited UTF-8 JSON. It is intentionally small enough
to use from `nc`, a shell pipeline, or a throwaway script, while still having
stable error codes and event delivery for long-lived integrations.

## Transport

| Platform | Address | Protection |
|---|---|---|
| Linux | `$XDG_RUNTIME_DIR/lumina.sock`, falling back to `/tmp/lumina-$UID.sock` | Mode `0600`, owned by the user |
| Windows | `\\.\pipe\lumina-<user-sid>` | DACL grants the owner full control |

Clients resolve the address automatically. Override it with `LUMINA_SOCKET` or
`LUMINA_PIPE`, or with `--socket` / `--pipe` on `lumina-send`.

A stale Unix socket left by a crash is removed after confirming nobody is
listening. A live socket is never stolen.

## Framing

- One JSON object per line, ending in `\n`. A trailing `\r` is accepted.
- Lines are limited to 64 KiB. A longer line receives a `LineTooLong` response
  and the connection is closed.
- Requests without an `id` are answered with `"id": null`.
- Numbers and strings are both valid ids, so a client may use a counter or a
  UUID.
- Unknown top-level fields are rejected. Command fields belong in `params`.

Each request receives exactly one response with the same `id`. Events may be
interleaved with responses on the same stream.

### Request

```json
{"id":7,"command":"notify","params":{"title":"Build done","body":"lumina 0.1.0-alpha"}}
```

The envelope has three fields:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `id` | number, string, or `null` | no | Echoed by the response |
| `command` | string | yes | One of the commands below |
| `params` | object | no | Command fields; omitted or `{}` for no-argument commands |

`params` is nested rather than flattened because `shortcut` has its own `id`
field. Nesting keeps the request id unambiguous:

```json
{"id":"req-1","command":"shortcut","params":{"id":"toggle_visibility"}}
```

### Response

Success:

```json
{"id":7,"ok":true,"data":{"state":"expanded"}}
```

Failure:

```json
{"id":7,"ok":false,"error":{"code":"UnknownProfile","message":"unknown profile `night`"}}
```

`data` and `error` are omitted when they do not apply. A client should match on
`id` and `ok`, then branch on `error.code` when `ok` is false.

### Event

```json
{"event":"state_changed","data":{"state":"expanded"}}
```

Events have no request id. They are pushed to every connected client.

## Commands

| Command | `params` | Result | Notes |
|---|---|---|---|
| `status` | none | `StateSnapshot` | Full state dump; the main debugging tool |
| `show` | `widget`, `duration_ms`, `pin` | `"ok"` | Brings the island into a visible state |
| `hide` | none | `"ok"` | Hides the island |
| `collapse` | none | `"ok"` | Returns to `collapsed` immediately |
| `notify` | `title`, `body`, `icon`, `accent`, `duration_ms`, `action` | `notification_id` | Fires the notification widget |
| `set_rgb` | `mode`, `color`, `speed` | `"ok"` | Session RGB override |
| `cycle_rgb` | none | `mode` | Advances to the next engine |
| `set_profile` | `name` | `"ok"` | `UnknownProfile` for an unknown name |
| `get_config` | `section` optional | object | Reads the effective config |
| `set_config` | `path`, `value`, `persist` | `"ok"` | `persist: false` keeps the change session-only |
| `reload_config` | none | `"ok"` | Same as the reload hotkey; restarts the listener if `advanced.ipc_name` or `advanced.ipc_enabled` changed |
| `media` | `action`, `player`, `position_ms`, `volume` | `"ok"` | See media actions below |
| `player_list` | none | array | Identities and statuses |
| `shortcut` | `id` | `"ok"` | Runs a configured shortcut action |
| `execute` | `command`, `args`, `timeout_ms` | `{exit_code, stdout}` | Opt-in behind `[advanced].ipc_allow_exec` |
| `spectrum` | `peak` | array of numbers | Current band magnitudes |
| `geometry` | `top_half_width`, `bottom_half_width`, `height`, `corner_radius` | `"ok"` | Live shape editing |
| `debug_overlay` | `enabled` | `"ok"` | Frame time and state readout |
| `quit` | none | `"ok"` | Responds, then shuts the server down |

### Field details

`show`

- `widget`: 1 to 64 characters.
- `duration_ms`: up to 86,400,000 (24 hours).
- `pin`: keep the state until told otherwise.

`notify`

- `title` is required, 1 to 256 characters.
- `body` is up to 2,048 characters.
- `icon` is an icon name, up to 128 characters.
- `accent` is `#rgb` or `#rrggbb`.
- `duration_ms` is up to 60,000.
- `action` is a shell command run when the card is clicked. The application owns
  the decision to run it; the IPC library does not execute anything.

`set_rgb`

- `mode` is one of `static`, `breathing`, `cycling`, `reactive`, or `album`.
- `color` is `#rgb` or `#rrggbb`.
- `speed` is between 0 and 10.

`set_config`

- `path` is a dotted key path such as `widgets.clock.show_seconds`.
- `value` is any JSON value accepted by that key.
- `persist` defaults to true at the application layer; set it to false for a
  session-only change.

`media`

- `action` is one of `play`, `pause`, `play_pause`, `next`, `previous`, `stop`,
  `seek`, or `volume`.
- `player` selects a session by identity. Omit it to use the active player.
- `seek` requires `position_ms`; `volume` requires `volume` (0 to 100).
- A session that disappeared after ranking returns `PlayerGone`. An identity
  that never matched returns `PlayerNotFound`.

`execute`

- `command` is a program path, never interpreted by a shell.
- `args` is an array of at most 64 strings, each up to 4,096 characters.
- `timeout_ms` is up to 30,000; the default is 10,000.
- When execution is disabled, the command returns `Disabled`.

`spectrum`

- Without `peak`, returns the current band magnitudes.
- With `peak: true`, returns the peak magnitudes since the previous call and
  resets the peak tracker. This is the mode external visualizers should use.

`geometry`

- All four values are finite and positive.
- `corner_radius` is clamped by the renderer to half the height.

### State snapshot

`status` returns the shape below. Optional fields are omitted when unknown.

```json
{
  "version": "0.1.0-alpha.1",
  "state": "expanded",
  "visible": true,
  "pinned": false,
  "profile": "default",
  "geometry": {
    "x": 660.0,
    "y": 8.0,
    "width": 600.0,
    "height": 44.0,
    "scale": 1.5,
    "monitor": "DP-1"
  },
  "rgb": {"mode": "reactive", "color": "#5ac8fa", "speed": 1.0},
  "media": {
    "player": "spotify",
    "title": "Track",
    "artist": "Artist",
    "status": "playing",
    "position_ms": 42000,
    "duration_ms": 210000
  },
  "uptime_ms": 123456
}
```

## Error codes

| Code | Meaning |
|---|---|
| `BadRequest` | Not valid JSON, or a required field is missing |
| `UnknownCommand` | The command string is not recognized |
| `InvalidArgument` | A field is present but out of range or the wrong type |
| `UnknownProfile` | The profile name does not exist |
| `PlayerGone` | The targeted media session disappeared |
| `PlayerNotFound` | No session matches the requested player |
| `Disabled` | The command exists but is gated behind a configuration flag |
| `NotSupported` | The command is unavailable on this platform |
| `RateLimited` | More than 120 commands per second from one client |
| `Internal` | A bug; details are in the log |
| `LineTooLong` | The request exceeded the 64 KiB framing limit |

Invalid commands do not close the connection. Only framing violations
(`LineTooLong`, invalid UTF-8) do. A client should treat `LineTooLong` as fatal
and reconnect with a smaller payload.

## Events

| Event | Data | Fired when |
|---|---|---|
| `media_changed` | `{"media": MediaSnapshot}` | Active player, status, or metadata changes |
| `state_changed` | `{"state": "expanded"}` | The island enters a new state |
| `rgb_changed` | `{"rgb": RgbSnapshot}` | The RGB engine or palette changes |
| `config_reloaded` | `{"profile": "default"}` | A reload completes successfully |
| `config_error` | `{"key": "...", "message": "..."}` | A reload fails validation |

Events are best effort. A client that cannot keep up may be dropped from the
broadcast list and must reconnect.

## Limits and concurrency

- At most 8 concurrent connections.
- 120 commands per second per client, with a small burst allowance.
- Every client has its own id space.
- Commands are serialized through the application's dispatcher, so ordering is
  global and deterministic.
- Slow handlers delay every connection. Keep `execute` and other blocking work
  off the dispatcher thread.

## Clients

### `lumina-send`

```sh
lumina-send notify --title "Build done" --body "lumina 0.1.0-alpha"
lumina-send set_rgb --mode reactive --color "#ff9500" --speed 1.4
lumina-send media --action seek --position-ms 60000
lumina-send get_config --section widgets.clock
lumina-send status --pretty
```

Options map to `params` by replacing dashes with underscores. `--no-pin`
negates a boolean, `--arg` appends to an array, and `--payload '{"key":1}'`
merges a raw object. `--compact` prints one line, `--quiet` prints nothing, and
`--completions bash|zsh|fish` prints a completion script.

Exit codes: `0` success, `1` connection failure, `2` protocol or argument error,
`3` usage error.

### `lumina-py`

```python
from lumina_py import LuminaClient

with LuminaClient() as client:
    client.notify("Build done", body="lumina 0.1.0-alpha")
```

See `clients/python/README.md` for the async client and event loop.

### `lumina-node`

```js
const { LuminaClient } = require('lumina-node');

const client = await LuminaClient.connect();
await client.notify({ title: 'Build done' });
client.on('media_changed', (data) => console.log(data.media));
await client.close();
```

See `clients/node/README.md` for the TypeScript surface.

## Examples

- [`examples/make-notify.sh`](examples/make-notify.sh): wrap `make` and report
  success or failure.
- [`examples/github-actions-reporter.yml`](examples/github-actions-reporter.yml):
  report workflow conclusions from a self-hosted runner.
- [`examples/mpv-osd.lua`](examples/mpv-osd.lua): notify on file load and
  pause/resume from mpv.

## Security

There is no authentication beyond the operating system. Unix socket permissions
and the Windows pipe DACL restrict access to the current user. Do not expose the
socket through a shared directory or a proxy.

`execute` and notification `action` are powerful by design. Keep
`[advanced].ipc_allow_exec` off unless a script genuinely needs it, and validate
anything that constructs those fields from untrusted input.

## Stability

The envelope, command names, error codes, and event names are stable for the
0.1 series. New optional fields may be added; clients must ignore fields they do
not understand inside `data` and event payloads. Unknown fields in requests are
rejected, so an older server will refuse a newer request that sends one.

## Testing

`cargo test -p lumina-ipc` covers parsing, framing, rate limiting, fan-out, and
end-to-end request/response over the real transport. The deterministic mutation
harness exercises random bytes against the parser and framing layer:

```sh
cargo run -p lumina-ipc --example fuzz_protocol -- 60
```

The Python and Node clients carry their own test suites; see their READMEs.
