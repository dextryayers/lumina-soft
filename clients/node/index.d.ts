import { EventEmitter } from 'node:events';
import { Socket } from 'node:net';

export type Json =
  | null
  | boolean
  | number
  | string
  | Json[]
  | { [key: string]: Json };

export type EventKind =
  | 'media_changed'
  | 'state_changed'
  | 'rgb_changed'
  | 'config_reloaded'
  | 'config_error';

export interface LuminaEvent {
  event: EventKind;
  data: Record<string, Json>;
}

export interface ClientOptions {
  /** Default per-call timeout in milliseconds. */
  timeout?: number | null;
}

export interface CallOptions {
  /** Timeout for this call in milliseconds. */
  timeout?: number;
}

export interface ShowParams {
  widget?: string;
  duration_ms?: number;
  pin?: boolean;
}

export interface NotifyParams {
  title: string;
  body?: string;
  icon?: string;
  accent?: string;
  duration_ms?: number;
  action?: string;
}

export interface SetRgbParams {
  mode: string;
  color?: string;
  speed?: number;
}

export type MediaAction =
  | 'play'
  | 'pause'
  | 'play_pause'
  | 'next'
  | 'previous'
  | 'stop'
  | 'seek'
  | 'volume';

export interface MediaParams {
  action: MediaAction;
  player?: string;
  position_ms?: number;
  volume?: number;
}

export interface ExecuteParams {
  command: string;
  args?: string[];
  timeout_ms?: number;
}

export interface GeometryParams {
  top_half_width: number;
  bottom_half_width: number;
  height: number;
  corner_radius: number;
}

export interface MediaSnapshot {
  player: string;
  title: string;
  artist: string;
  status: 'playing' | 'paused' | 'stopped' | string;
  position_ms?: number;
  duration_ms?: number;
}

export interface RgbSnapshot {
  mode: string;
  color: string;
  speed: number;
}

export interface GeometrySnapshot {
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
  monitor?: string;
}

export interface StateSnapshot {
  version: string;
  state: string;
  visible: boolean;
  pinned: boolean;
  profile: string;
  geometry?: GeometrySnapshot;
  rgb: RgbSnapshot;
  media?: MediaSnapshot;
  uptime_ms: number;
}

export class LuminaError extends Error {
  constructor(message: string, code?: string | null, response?: unknown);
  code: string | null;
  response: unknown;
}

export class RemoteError extends LuminaError {}

export class ProtocolError extends LuminaError {}

export class ConnectionClosed extends LuminaError {}

export class LuminaClient extends EventEmitter {
  constructor(socket: Socket, options?: ClientOptions);
  static connect(
    address?: string,
    options?: ClientOptions
  ): Promise<LuminaClient>;

  readonly socket: Socket;
  timeout: number | null;
  readonly closed: boolean;

  call<T = Json>(
    command: string,
    params?: Record<string, Json>,
    options?: CallOptions
  ): Promise<T>;
  nextEvent(timeout?: number): Promise<LuminaEvent | null>;
  close(): Promise<void>;

  status(): Promise<StateSnapshot>;
  show(params?: ShowParams): Promise<Json>;
  hide(): Promise<Json>;
  collapse(): Promise<Json>;
  notify(params: NotifyParams): Promise<string>;
  setRgb(params: SetRgbParams): Promise<Json>;
  cycleRgb(): Promise<string>;
  setProfile(name: string): Promise<Json>;
  getConfig(section?: string | null): Promise<Record<string, Json>>;
  setConfig(path: string, value: Json, persist?: boolean): Promise<Json>;
  reloadConfig(): Promise<Json>;
  media(action: MediaAction, options?: Omit<MediaParams, 'action'>): Promise<Json>;
  playerList(): Promise<Array<Record<string, Json>>>;
  shortcut(shortcutId: string): Promise<Json>;
  execute(
    command: string,
    args?: string[],
    timeoutMs?: number
  ): Promise<{ exit_code: number; stdout: string }>;
  spectrum(peak?: boolean): Promise<number[]>;
  geometry(params: GeometryParams): Promise<Json>;
  debugOverlay(enabled: boolean): Promise<Json>;
  quit(): Promise<Json>;

  on(event: 'event', listener: (event: LuminaEvent) => void): this;
  on(
    event: 'protocolError',
    listener: (error: ProtocolError) => void
  ): this;
  on(event: EventKind, listener: (data: Record<string, Json>) => void): this;
  on(event: 'close', listener: (reason: ConnectionClosed) => void): this;
  once(event: 'event', listener: (event: LuminaEvent) => void): this;
  once(event: 'close', listener: (reason: ConnectionClosed) => void): this;
  once(event: EventKind, listener: (data: Record<string, Json>) => void): this;
}

export const MAX_LINE_BYTES: number;
export function defaultAddress(): string;
