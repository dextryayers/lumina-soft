# lumina-node

Zero-dependency Node.js client for Lumina's newline-delimited JSON IPC, with
TypeScript definitions.

## Install

```sh
npm install ./clients/node
```

## Use

```js
const { LuminaClient } = require('lumina-node');

const client = await LuminaClient.connect();
try {
  const status = await client.status();
  console.log(status.state);

  await client.notify({
    title: 'Build done',
    body: 'lumina 0.1.0-alpha',
    accent: '#34c759',
    duration_ms: 5000,
  });
} finally {
  await client.close();
}
```

Every command has a helper, and `call` accepts any command plus its `params`
object:

```js
await client.call('set_config', {
  path: 'widgets.clock.show_seconds',
  value: true,
});
```

## Events

The client is an `EventEmitter`. Events arrive as `event` plus their own name:

```js
client.on('event', (message) => console.log(message.event, message.data));
client.on('media_changed', (data) => console.log(data.media));
```

`nextEvent()` also consumes the buffer as a promise, and returns `null` after a
timeout:

```js
const event = await client.nextEvent(30_000);
```

## Address

The default matches the Rust server: `$XDG_RUNTIME_DIR/lumina.sock` (or
`/tmp/lumina-$UID.sock`) on POSIX and `\\.\pipe\lumina-<user-sid>` on Windows.
Set `LUMINA_SOCKET` or `LUMINA_PIPE` to override it, or call
`LuminaClient.connect(address)`.

## Errors

`RemoteError` carries the server's `code` and `message`. `ProtocolError`
indicates a framing violation, and `ConnectionClosed` means the connection
ended. Calls accept `{ timeout }`; a timeout rejects with code `Timeout` and does
not close the connection.

## Tests

```sh
cd clients/node
node --test
```
