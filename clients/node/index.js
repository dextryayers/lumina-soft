'use strict';

const net = require('node:net');
const { execFileSync } = require('node:child_process');
const { EventEmitter } = require('node:events');
const { TextDecoder } = require('node:util');

const MAX_LINE_BYTES = 64 * 1024;
const MAX_BUFFERED_EVENTS = 256;

class LuminaError extends Error {
  constructor(message, code = null, response = null) {
    super(message);
    this.name = new.target.name;
    this.code = code;
    this.response = response;
  }
}

class RemoteError extends LuminaError {}

class ProtocolError extends LuminaError {}

class ConnectionClosed extends LuminaError {}

let cachedSid = null;

function windowsSid() {
  if (cachedSid !== null) return cachedSid;
  const output = execFileSync('whoami', ['/user'], {
    encoding: 'utf8',
    windowsHide: true,
  });
  const match = output.match(/\bS-\d+(?:-\d+)+\b/);
  if (match === null) {
    throw new LuminaError(
      'could not determine the current Windows SID; set LUMINA_PIPE',
      'Address',
    );
  }
  cachedSid = match[0];
  return cachedSid;
}

function defaultAddress() {
  if (process.platform === 'win32') {
    return process.env.LUMINA_PIPE || `\\\\.\\pipe\\lumina-${windowsSid()}`;
  }
  if (process.env.LUMINA_SOCKET) return process.env.LUMINA_SOCKET;
  if (process.env.XDG_RUNTIME_DIR) {
    return `${process.env.XDG_RUNTIME_DIR}/lumina.sock`;
  }
  return `/tmp/lumina-${process.getuid()}.sock`;
}

class LuminaClient extends EventEmitter {
  constructor(socket, options = {}) {
    super();
    this.socket = socket;
    this.timeout = options.timeout ?? null;
    this._buffer = Buffer.alloc(0);
    this._decoder = new TextDecoder('utf-8', { fatal: true });
    this._nextId = 0;
    this._pending = new Map();
    this._events = [];
    this._eventWaiters = [];
    this._closed = false;
    this._closeEmitted = false;
    this._lastError = null;

    socket.on('data', (chunk) => this._onData(chunk));
    socket.on('error', (error) => {
      this._lastError = error;
    });
    socket.on('close', () => this._finalize(this._lastError));
  }

  static connect(address = defaultAddress(), options = {}) {
    return new Promise((resolve, reject) => {
      const socket = net.createConnection(address);
      const onError = (error) => {
        socket.destroy();
        reject(error);
      };
      socket.once('error', onError);
      socket.once('connect', () => {
        socket.off('error', onError);
        resolve(new LuminaClient(socket, options));
      });
    });
  }

  get closed() {
    return this._closed;
  }

  call(command, params = {}, options = {}) {
    if (this._closed) {
      return Promise.reject(new ConnectionClosed('client is closed'));
    }
    if (typeof command !== 'string' || command.length === 0) {
      return Promise.reject(new TypeError('command must be a non-empty string'));
    }
    if (
      params === null ||
      typeof params !== 'object' ||
      Array.isArray(params)
    ) {
      return Promise.reject(new TypeError('params must be an object'));
    }
    const id = this._nextId + 1;
    this._nextId = id;
    const line = `${JSON.stringify({ id, command, params })}\n`;
    const timeout = options.timeout ?? this.timeout;

    return new Promise((resolve, reject) => {
      const entry = { resolve, reject, timer: null };
      if (timeout !== null && timeout !== undefined) {
        entry.timer = setTimeout(() => {
          if (this._pending.delete(id)) {
            reject(
              new LuminaError(
                `\`${command}\` timed out after ${timeout} ms`,
                'Timeout',
              ),
            );
          }
        }, timeout);
        entry.timer.unref?.();
      }
      this._pending.set(id, entry);
      this.socket.write(line, (error) => {
        if (error && this._pending.delete(id)) {
          if (entry.timer) clearTimeout(entry.timer);
          reject(error);
        }
      });
    });
  }

  nextEvent(timeout = null) {
    if (this._events.length > 0) {
      return Promise.resolve(this._events.shift());
    }
    if (this._closed) {
      return Promise.reject(new ConnectionClosed('client is closed'));
    }
    return new Promise((resolve, reject) => {
      const waiter = { resolve, reject, timer: null };
      if (timeout !== null && timeout !== undefined) {
        waiter.timer = setTimeout(() => {
          const index = this._eventWaiters.indexOf(waiter);
          if (index >= 0) this._eventWaiters.splice(index, 1);
          resolve(null);
        }, timeout);
        waiter.timer.unref?.();
      }
      this._eventWaiters.push(waiter);
    });
  }

  close() {
    if (this._closed) return Promise.resolve();
    return new Promise((resolve) => {
      this.once('close', resolve);
      this.socket.destroy();
    });
  }

  status() {
    return this.call('status');
  }

  show(params = {}) {
    return this.call('show', params);
  }

  hide() {
    return this.call('hide');
  }

  collapse() {
    return this.call('collapse');
  }

  notify(params) {
    return this.call('notify', params);
  }

  setRgb(params) {
    return this.call('set_rgb', params);
  }

  cycleRgb() {
    return this.call('cycle_rgb');
  }

  setProfile(name) {
    return this.call('set_profile', { name });
  }

  getConfig(section = null) {
    return this.call('get_config', section === null ? {} : { section });
  }

  setConfig(path, value, persist = undefined) {
    const params = { path, value };
    if (persist !== undefined) params.persist = persist;
    return this.call('set_config', params);
  }

  reloadConfig() {
    return this.call('reload_config');
  }

  media(action, options = {}) {
    return this.call('media', { action, ...options });
  }

  playerList() {
    return this.call('player_list');
  }

  shortcut(shortcutId) {
    return this.call('shortcut', { id: shortcutId });
  }

  execute(command, args = undefined, timeoutMs = undefined) {
    const params = { command };
    if (args !== undefined) params.args = args;
    if (timeoutMs !== undefined) params.timeout_ms = timeoutMs;
    return this.call('execute', params);
  }

  spectrum(peak = undefined) {
    return this.call('spectrum', peak === undefined ? {} : { peak });
  }

  geometry(params) {
    return this.call('geometry', params);
  }

  debugOverlay(enabled) {
    return this.call('debug_overlay', { enabled });
  }

  quit() {
    return this.call('quit');
  }

  _onData(chunk) {
    this._buffer =
      this._buffer.length === 0
        ? Buffer.from(chunk)
        : Buffer.concat([this._buffer, chunk]);

    for (;;) {
      const newline = this._buffer.indexOf(0x0a);
      if (newline === -1) break;
      let line = this._buffer.subarray(0, newline);
      this._buffer = this._buffer.subarray(newline + 1);
      if (line.length > 0 && line[line.length - 1] === 0x0d) {
        line = line.subarray(0, line.length - 1);
      }
      if (line.length > MAX_LINE_BYTES) {
        this._fail(
          new ProtocolError(
            `line exceeds the ${MAX_LINE_BYTES}-byte limit`,
            'LineTooLong',
          ),
        );
        return;
      }
      let text;
      try {
        text = this._decoder.decode(line);
      } catch {
        this._fail(new ProtocolError('line is not valid UTF-8'));
        return;
      }
      let message;
      try {
        message = JSON.parse(text);
      } catch (error) {
        this._fail(new ProtocolError(`invalid JSON: ${error.message}`));
        return;
      }
      if (message === null || typeof message !== 'object' || Array.isArray(message)) {
        this._fail(new ProtocolError('each line must be a JSON object'));
        return;
      }
      this._dispatch(message);
    }

    if (this._buffer.length > MAX_LINE_BYTES) {
      this._fail(
        new ProtocolError(
          `line exceeds the ${MAX_LINE_BYTES}-byte limit`,
          'LineTooLong',
        ),
      );
    }
  }

  _dispatch(message) {
    if (Object.prototype.hasOwnProperty.call(message, 'event')) {
      this._emitEvent(message);
      return;
    }
    const entry = this._pending.get(message.id);
    if (entry === undefined) {
      if (this.listenerCount('protocolError') > 0) {
        this.emit(
          'protocolError',
          new ProtocolError(`unmatched response id ${JSON.stringify(message.id)}`),
        );
      }
      return;
    }
    this._pending.delete(message.id);
    if (entry.timer) clearTimeout(entry.timer);

    if (message.ok === true) {
      entry.resolve(message.data ?? null);
      return;
    }
    if (message.ok === false) {
      const error =
        message.error !== null && typeof message.error === 'object'
          ? message.error
          : {};
      entry.reject(
        new RemoteError(
          String(error.message ?? 'command failed'),
          String(error.code ?? 'Internal'),
          message,
        ),
      );
      return;
    }
    entry.reject(new ProtocolError('response is missing a boolean `ok` field'));
  }

  _emitEvent(message) {
    const waiter = this._eventWaiters.shift();
    if (waiter) {
      if (waiter.timer) clearTimeout(waiter.timer);
      waiter.resolve(message);
    } else {
      if (this._events.length >= MAX_BUFFERED_EVENTS) this._events.shift();
      this._events.push(message);
    }
    this.emit('event', message);
    if (typeof message.event === 'string') {
      this.emit(message.event, message.data);
    }
  }

  _fail(error) {
    if (this._closed) return;
    if (error instanceof ProtocolError) this.emit('protocolError', error);
    this._finalize(error);
    this.socket.destroy();
  }

  _finalize(error) {
    if (this._closeEmitted) return;
    this._closeEmitted = true;
    this._closed = true;
    const reason =
      error === null || error === undefined
        ? new ConnectionClosed('connection closed')
        : error instanceof LuminaError
          ? error
          : new ConnectionClosed(`connection failed: ${error.message}`);

    for (const entry of this._pending.values()) {
      if (entry.timer) clearTimeout(entry.timer);
      entry.reject(reason);
    }
    this._pending.clear();
    for (const waiter of this._eventWaiters) {
      if (waiter.timer) clearTimeout(waiter.timer);
      waiter.reject(reason);
    }
    this._eventWaiters.length = 0;
    this._buffer = Buffer.alloc(0);
    this.emit('close', reason);
  }
}

module.exports = {
  ConnectionClosed,
  LuminaClient,
  LuminaError,
  MAX_LINE_BYTES,
  ProtocolError,
  RemoteError,
  defaultAddress,
};
