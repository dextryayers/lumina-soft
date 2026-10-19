'use strict';

const assert = require('node:assert/strict');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  ConnectionClosed,
  LuminaClient,
  LuminaError,
  ProtocolError,
  defaultAddress,
} = require('../index.js');

let sequence = 0;

function addressFor(label) {
  sequence += 1;
  if (process.platform === 'win32') {
    return `\\\\.\\pipe\\lumina-test-${process.pid}-${sequence}-${label}`;
  }
  return path.join(
    os.tmpdir(),
    `lumina-test-${process.pid}-${sequence}-${label}.sock`,
  );
}

async function startServer(handler) {
  const address = addressFor('server');
  const sockets = new Set();
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.setEncoding('utf8');
    let buffer = '';
    socket.on('data', (chunk) => {
      buffer += chunk;
      for (;;) {
        const newline = buffer.indexOf('\n');
        if (newline === -1) break;
        let line = buffer.slice(0, newline);
        buffer = buffer.slice(newline + 1);
        if (line.endsWith('\r')) line = line.slice(0, -1);
        if (line.length > 0) handler(JSON.parse(line), socket);
      }
    });
  });
  await new Promise((resolve, reject) => {
    const onError = (error) => {
      server.off('listening', onListening);
      reject(error);
    };
    const onListening = () => {
      server.off('error', onError);
      resolve();
    };
    server.once('error', onError);
    server.once('listening', onListening);
    server.listen(address);
  });
  return {
    address,
    async close() {
      for (const socket of sockets) socket.destroy();
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

function send(socket, message) {
  socket.write(`${JSON.stringify(message)}\n`);
}

test('call matches responses by id and returns data', async () => {
  const server = await startServer((request, socket) => {
    send(socket, {
      id: request.id,
      ok: true,
      data: { state: 'collapsed' },
    });
  });
  const client = await LuminaClient.connect(server.address);
  try {
    const status = await client.status();
    assert.deepEqual(status, { state: 'collapsed' });
  } finally {
    await client.close();
    await server.close();
  }
});

test('events are pushed through EventEmitter before the response', async () => {
  const server = await startServer((request, socket) => {
    send(socket, {
      event: 'state_changed',
      data: { state: 'expanded' },
    });
    send(socket, { id: request.id, ok: true, data: 'ok' });
  });
  const client = await LuminaClient.connect(server.address);
  const seen = [];
  client.on('state_changed', (data) => seen.push(data));
  try {
    assert.equal(await client.hide(), 'ok');
    assert.deepEqual(seen, [{ state: 'expanded' }]);
  } finally {
    await client.close();
    await server.close();
  }
});

test('nextEvent resolves from the buffer and from the socket', async () => {
  const server = await startServer((request, socket) => {
    send(socket, {
      event: 'media_changed',
      data: { media: { title: 'Track' } },
    });
    send(socket, { id: request.id, ok: true, data: 'ok' });
  });
  const client = await LuminaClient.connect(server.address);
  try {
    const waiting = client.nextEvent(1000);
    assert.equal(await client.hide(), 'ok');
    const event = await waiting;
    assert.equal(event.event, 'media_changed');
    assert.equal(event.data.media.title, 'Track');
  } finally {
    await client.close();
    await server.close();
  }
});

test('remote errors carry the protocol code', async () => {
  const server = await startServer((request, socket) => {
    send(socket, {
      id: request.id,
      ok: false,
      error: { code: 'UnknownProfile', message: 'no such profile' },
    });
  });
  const client = await LuminaClient.connect(server.address);
  try {
    await assert.rejects(client.setProfile('missing'), (error) => {
      assert.ok(error instanceof LuminaError);
      assert.equal(error.code, 'UnknownProfile');
      assert.match(error.message, /no such profile/);
      return true;
    });
  } finally {
    await client.close();
    await server.close();
  }
});

test('a call can time out and the client stays usable', async () => {
  const server = await startServer(() => {});
  const client = await LuminaClient.connect(server.address);
  try {
    await assert.rejects(client.call('status', {}, { timeout: 50 }), (error) => {
      assert.equal(error.code, 'Timeout');
      return true;
    });
    assert.equal(client.closed, false);
  } finally {
    await client.close();
    await server.close();
  }
});

test('an overlong line is a protocol error', async () => {
  const server = await startServer((_request, socket) => {
    socket.write('x'.repeat(64 * 1024 + 1));
  });
  const client = await LuminaClient.connect(server.address);
  const errors = [];
  client.on('protocolError', (error) => errors.push(error));
  try {
    await assert.rejects(client.status(), (error) => {
      assert.ok(error instanceof ProtocolError);
      return true;
    });
    assert.equal(errors.length, 1);
  } finally {
    await client.close();
    await server.close();
  }
});

test('closing rejects pending calls with ConnectionClosed', async () => {
  const server = await startServer(() => {});
  const client = await LuminaClient.connect(server.address);
  const pending = client.call('status');
  await client.close();
  await assert.rejects(pending, ConnectionClosed);
  await server.close();
});

test('the pipe and socket overrides win', () => {
  const key = process.platform === 'win32' ? 'LUMINA_PIPE' : 'LUMINA_SOCKET';
  const previous = process.env[key];
  process.env[key] = 'custom-address';
  try {
    assert.equal(defaultAddress(), 'custom-address');
  } finally {
    if (previous === undefined) delete process.env[key];
    else process.env[key] = previous;
  }
});
