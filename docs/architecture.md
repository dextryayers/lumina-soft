# Architecture

Lumina is a Rust workspace with a deliberately narrow runtime. The engine,
configuration, platform, and protocol layers can be built and tested without a
display or a GPU. The orchestrator is the only component allowed to own the
window, render loop, and long-lived service threads.

> **Status.** The libraries below are implemented and covered by headless tests.
> The orchestrator and the window that it drives are still in progress. This
> document describes the target runtime contract so every component is built
> against the same invariants; it does not claim that the island is already
> visible on screen.

## Workspace boundaries

```text
lumina-app
  |-- lumina-config ---- lumina-core
  |-- lumina-render ---- lumina-config, lumina-core
  |-- lumina-widgets --- lumina-config, lumina-core, lumina-text, lumina-media
  |-- lumina-platform -- lumina-config, lumina-core
  |-- lumina-text ------ lumina-core
  |-- lumina-color ----- lumina-core
  |-- lumina-audio ----- lumina-config, lumina-core
  |-- lumina-media ----- lumina-core
  `-- lumina-ipc ------- (no UI dependencies)
```

- `lumina-core` owns shared data types, the state machine, geometry, spring
  integration, and the priority arbiter. It has no window or GPU dependency.
- `lumina-config` owns schema parsing, validation and repair, profiles, themes,
  and hot reload. It never starts a renderer.
- `lumina-render` owns WGSL shaders, pipelines, the SDF body pass, the frame
  loop, and headless render tests.
- `lumina-widgets` owns layout and widget behavior without owning a window.
- `lumina-platform` isolates Windows, X11, and Wayland differences behind
  capability probes and per-user autostart helpers.
- `lumina-ipc` is independently testable and exposes no UI types.
- `lumina-app` is the composition root. It must contain wiring, not algorithms
  that belong in a library.

## Runtime threads

The runtime uses eight long-lived threads. Thread names are stable so logs and
crash reports can identify the owner.

| Thread | Name | Owns | Publishes | Rate |
|---|---|---|---|---|
| Main | `lumina-main` | `wgpu::Instance`, surface, event loop | Commands to services | Event driven |
| Orchestrator | `lumina-orch` | `IslandState`, config snapshot, widget state | `StateSnapshot` | Event driven |
| Audio capture | `lumina-audio-cap` | Device handle and ring-buffer writer | Raw samples | Device callback |
| Audio analyzer | `lumina-audio-an` | FFT plan and band array | `BandFrame` | 60 Hz |
| Media watcher | `lumina-media` | Platform media-session proxy | `MediaSnapshot` | Event driven, 1 Hz fallback |
| Metrics sampler | `lumina-metrics` | Sampling timer | `MetricFrame` | 1 Hz, configurable |
| Config watcher | `lumina-cfg` | File notification handle | `ConfigUpdate` | On change, 150 ms debounce |
| IPC server | `lumina-ipc` | Listener and client registry | `Command` | Per request |

The audio callback never blocks. It writes samples into a lock-free
single-producer/single-consumer ring; the analyzer is the only consumer. A slow
FFT can drop frames, but it cannot stall the device callback.

## Channels and snapshots

```text
 [audio capture] --SPSC f32 ring--> [audio analyzer]
                                          |
                                          | BandFrame, 60 Hz
                                          v
 [media watcher] --MediaSnapshot--+
 [metrics sampler] --MetricFrame--+       +--> [orchestrator]
 [config watcher] --ConfigUpdate--+       |      mutates state
 [ipc server] ------Command-------+       |      publishes StateSnapshot
                                          |
 [main thread input] --InputEvent---------+
                                          |
                                          v
                                  [main thread render]
```

`StateSnapshot` is a plain `Copy` value published through a triple-buffered
slot. The orchestrator writes the free slot, atomically publishes the index, and
the renderer reads the published index. There are no locks and no per-frame
allocations on the render path.

The snapshot contains geometry targets, interpolated geometry, palette values,
content identity, opacity, and a small set of flags. Large assets such as album
art or text atlases are referenced indirectly, never copied into the snapshot.

## Startup and shutdown

Startup has a 400 ms budget on a warm machine:

| Step | Budget |
|---|---|
| Parse arguments and claim the single-instance lock | 5 ms |
| Load, validate, and layer configuration | 10 ms |
| Connect to the display server and negotiate the overlay | 40 ms |
| Position and create the window before it is shown | 20 ms |
| Initialize the adapter, device, surface, and swapchain | 120 ms |
| Build pipelines and upload atlases | 60 ms |
| Start services and warm media enumeration | 90 ms |
| Render the collapsed first frame and reveal | 20 ms |

The window is positioned before its first paint, so it never jumps from a
default location. If a service misses its budget, it starts late rather than
blocking the first frame.

Shutdown reverses the order with a hard 500 ms budget: stop rendering,
unregister hotkeys, send `Disconnect` to IPC clients, flush logs, wait for GPU
work with `device.poll(MainThread)`, then close the display connection. Springs
snap to their targets so the island does not animate while exiting.

## Island state machine

| State | Top half-width | Bottom half-width | Height | Priority |
|---|---:|---:|---:|---:|
| `Collapsed` | 62 | 54 | 18 | 0 |
| `Hover` | 66 | 56 | 22 | 1 |
| `Clock` | 100 | 92 | 30 | 2 |
| `Metrics` | 120 | 112 | 28 | 3 |
| `Shortcuts` | 170 | 160 | 34 | 4 |
| `Media` | 180 | 168 | 40 | 5 |
| `Spectrum` | 210 | 200 | 50 | 6 |
| `Notify` | 200 | 190 | 28 | 7 |

All values are logical pixels at 100% DPI and are scaled by the monitor factor
before reaching the shader.

A request is active when its trigger is inside its hold window, the pointer is
inside the island, or configuration or IPC has pinned it. The arbiter selects
the highest-priority active request. Any state may transition to any other
state; the master spring is retargeted rather than restarted, so velocity is
preserved.

Content crossfades over 140 ms while geometry springs over roughly 280 ms. A
lower-priority request that arrives while a higher-priority request holds is
queued. When the higher-priority request expires, the queued request becomes
eligible.

Explicit edge cases:

- A disappearing media session fades content out over 120 ms and collapses.
- A track change swaps content with a 100 ms crossfade without touching geometry.
- Two players starting together resolve by priority; the loser is remembered.
- An unplugged audio device freezes spectrum bars at zero and logs once.
- A disconnected monitor falls back to the primary monitor.
- A DPI change rescales geometry and text while preserving the active state.
- A config reload mid-transition retargets springs without restarting them.
- Two hotkeys within 10 ms: the last wins, the other is dropped with a debug log.

## Shape mathematics

The island is an isosceles inverted trapezoid: the top edge is wider than the
bottom edge. Its signed distance field is analytic, so geometry changes update
uniforms instead of rebuilding a mesh.

The shader works in centered local shape space, with `+x` right and `+y` up,
and maps to clip space in the vertex stage. Parameters are `half_bottom`,
`half_top`, `half_h`, `radius`, and `slant = (half_top - half_bottom) / (2 *
half_h)` when comparing the slanted edge.

The body pass evaluates a rounded trapezoid SDF, subtracts `border_width` for
the inner edge, and applies antialiasing with the screen-space derivative.
Border brightness, fill, and glow share the same distance evaluation. The glow
uses an exponential falloff and early-outs beyond `glow_spread`, so transparent
pixels outside the island cost little.

Geometry is clamped before it reaches the shader:

- `half_top >= half_bottom`
- `radius >= 0` and no larger than the smallest half-extent
- width and height remain positive after DPI scaling
- corner radius is reduced before the shape can self-intersect

Backdrop blur is a separate optional pass. On platforms with a native blur, the
platform path is preferred; otherwise a two-pass separable blur uses ping-pong
targets. The island body is drawn after the backdrop.

## Rendering

Every frame executes at most four passes:

| Pass | Purpose | When |
|---|---|---|
| P0 | Backdrop blur | Only when software blur is active |
| P1 | Island body: fill, border, glow | Every frame |
| P2 | Content: text, icons, album art, controls | Every frame |
| P3 | Spectrum bars or waveform | When spectrum is visible |

The body is one full-target quad. The fragment shader early-outs on distance, so
only the island's screen area runs the expensive path. Content is instanced
quads backed by the glyph atlas and image registry.

The surface uses premultiplied alpha, the preferred sRGB format, and
`RENDER_ATTACHMENT` usage. VSync selects `Fifo`; otherwise `Immediate` with a
frame pacer. Frame latency defaults to two. Power profile is low-power on
battery. The backbuffer count starts at two and rises to three only if measured
frame time exceeds 6 ms.

Pipeline creation is wrapped in a validation error scope. A rejected effects
pipeline falls back to the GLES compilation path, then to a solid rounded
rectangle shader. A GPU driver can make Lumina less pretty; it cannot make
Lumina exit.

The dirty-flag loop renders only when state changed, a spring is moving, audio
is active, or a scheduled tick is due. The idle branch blocks on a channel with
a timeout equal to the next tick. This is what keeps idle CPU near zero instead
of polling at the refresh rate.

## Animation

Motion uses damped springs integrated with semi-implicit Euler:

```text
omega = 2 * PI / response
k = omega * omega
c = 2 * damping_ratio * omega
a = -k * (x - target) - c * v
v += a * dt
x += v * dt
```

One master spring drives a scalar `t` in `[0, 1]`; geometry interpolates between
the outgoing and incoming parameter sets with `smoothstep(t)`. A single spring
guarantees the shape stays on the family of valid trapezoids. Independent
springs could let `half_bottom` overtake `half_top` and briefly invert the
shape.

| Spring | Response | Damping | Drives |
|---|---:|---:|---|
| `morph` | 0.34 s | 0.78 | Geometry interpolation |
| `content_fade` | 0.16 s | 1.0 | Content opacity |
| `hover_glow` | 0.12 s | 0.85 | Hover border brightness |
| `reveal` | 0.42 s | 1.0 | Startup scale-in |
| `notify_slide` | 0.28 s | 0.85 | Notification entry offset |
| `profile_crossfade` | 0.30 s | 1.0 | Profile color transition |

Springs integrate in fixed substeps of `1/240 s`, at most eight per frame.
Simulated time per frame is clamped to 100 ms, and a frame longer than 33 ms
advances only 33 ms. Animation slows under load instead of exploding.

Reduced motion sets every spring to `response = 0.08`, damping `1.0`, and skips
pulse, shake, and slide sequences. Transitions still communicate state changes
but never overshoot.

## Configuration and theming

The configuration stack is deterministic:

```text
compiled defaults -> config.toml -> theme -> profile -> CLI -> session IPC
```

Later layers win. A theme or profile contains only the keys it changes. Unknown
keys produce a warning with a spelling suggestion. Invalid values are repaired
per key rather than discarding the whole file. Hot reload debounces file events
for 150 ms and publishes a new snapshot only after validation.

Themes are TOML data, discovered from `themes/` beside `config.toml`, and are
resolved case-insensitively. `extends` resolves one parent; cycles are ignored.
Themes cannot run commands or alter IPC permissions.

## Reliability and security

Each worker wraps its body in `catch_unwind`. A panic in audio stops capture and
marks spectrum unavailable; a panic in media stops polling; a panic on the main
thread logs and restarts the render loop in a controlled way. Configuration and
state are owned by the orchestrator, which does not unwind, so a worker failure
cannot corrupt them.

The IPC socket is owned by the current user with mode `0600` on Unix and a
user-SID DACL on Windows. The rate limit is 120 commands per second per
connection. `execute` is disabled unless explicitly enabled in advanced
configuration. Shortcut commands are user-configured local execution and are
not sandboxed. There is no telemetry, no auto-update, and no listening network
socket.

## Testing contract

The architecture is designed to be testable without a display:

- `lumina-core` tests SDF correctness, spring settling, interruption, and state
  transitions.
- `lumina-config` tests every key's invalid types, ranges, enums, and repair
  paths, plus theme and profile layering.
- `lumina-render` validates shaders with `naga` and runs headless frame tests.
- `lumina-ipc` fuzzes the codec and verifies a Python and Node round trip.
- The app self-test exercises platform capability probes, memory diagnostics,
  render-loop allocation, and protocol limits.

See [`../PLAN.md`](../PLAN.md) for the full specification and
[`troubleshooting.md`](troubleshooting.md) for degraded-mode behavior.
