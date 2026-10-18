# Lumina

**Dynamic Island for the desktop. Windows 10/11 and Linux. Rust. Under 40 MB.**

| Field | Value |
|---|---|
| Product | Lumina |
| Version | V0.1.0-Alpha |
| Status | Design complete, Phase 0 not started |
| Document revision | r4 |
| Authoritative file | `PLAN.md` (this file) |
| Language | English (all identifiers, docs, logs, UI strings) |

---

## Table of Contents

1. [Vision](#1-vision)
2. [Scope and Non-Goals](#2-scope-and-non-goals)
3. [Design Principles and Measurable Targets](#3-design-principles-and-measurable-targets)
4. [Modality Matrix](#4-modality-matrix)
5. [Platform Support Matrix](#5-platform-support-matrix)
6. [Technical Decisions](#6-technical-decisions)
7. [Repository Layout](#7-repository-layout)
8. [Runtime Architecture](#8-runtime-architecture)
9. [Island State Machine](#9-island-state-machine)
10. [Shape Mathematics](#10-shape-mathematics)
11. [Rendering Engine](#11-rendering-engine)
12. [Animation System](#12-animation-system)
13. [Color and RGB Engine](#13-color-and-rgb-engine)
14. [Typography and Icon Pipeline](#14-typography-and-icon-pipeline)
15. [Widget System and Layout](#15-widget-system-and-layout)
16. [Media Integration](#16-media-integration)
17. [Audio Engine](#17-audio-engine)
18. [Configuration System](#18-configuration-system)
19. [Customization and Extensibility](#19-customization-and-extensibility)
20. [Hotkeys, Shortcuts, Tray](#20-hotkeys-shortcuts-tray)
21. [IPC Protocol](#21-ipc-protocol)
22. [Platform Backends](#22-platform-backends)
23. [Performance Engineering](#23-performance-engineering)
24. [Reliability and Diagnostics](#24-reliability-and-diagnostics)
25. [Security Model](#25-security-model)
26. [Accessibility](#26-accessibility)
27. [Testing Strategy](#27-testing-strategy)
28. [Build, Packaging, Distribution](#28-build-packaging-distribution)
29. [Delivery Plan](#29-delivery-plan)
30. [Risks and Mitigations](#30-risks-and-mitigations)
31. [V0.1.0-Alpha Acceptance Criteria](#31-v001-alpha-acceptance-criteria)
32. [Post 1.0 Roadmap](#32-post-10-roadmap)
33. [Appendices](#33-appendices)

---

## 1. Vision

Lumina is a small always-on-top desktop overlay that lives at the top center of the screen. At rest it is a compact inverted trapezoid, wider at the top edge, with softly rounded corners. When something happens, the shape springs open and reveals media controls, a live audio spectrum, the date, system metrics, or user-defined shortcuts. When the event ends, it springs back down.

The goal is not to copy Apple's Dynamic Island pixel for pixel. The goal is to build the best possible desktop status companion for Windows and Linux, with a rendering model good enough that users can reshape every part of it without touching Rust code.

Three constraints shape every decision in this document:

1. It must be cheap. A background process that costs 40 MB and half a percent of a CPU core is acceptable. One that costs 200 MB is not.
2. It must be yours. Every color, dimension, timing, widget, and effect is configurable from a plain text file, with live reload and no restart.
3. It must be local. No accounts, no telemetry, no network calls unless the user explicitly configures a key for a single optional widget.

---

## 2. Scope and Non-Goals

### 2.1 In scope for V0.1.0-Alpha

- Inverse trapezoid island rendered on the GPU with signed distance fields.
- Spring driven morphing between eight island states.
- Media control for any player exposing MPRIS2 (Linux) or GSMTC (Windows). This covers Spotify desktop, Spotify in browser, YouTube Music in browser, VLC, mpv, Rhythmbox, Firefox, Chrome, Edge.
- Live audio spectrum visualizer sourced from system loopback or monitor capture.
- Five RGB color engines including audio reactive and album art derived palettes.
- Clock, date, calendar, system monitor, battery, network, pomodoro, weather, notification, and custom shell script widgets.
- Global hotkeys, per-shortcut hotkeys, system tray menu, autostart.
- Newline delimited JSON IPC over Unix domain socket (Linux) and named pipe (Windows).
- Full TOML configuration with hot reload, profiles, and theme inheritance.
- Packaging for Windows 10 and 11, Debian and Ubuntu, plus AppImage.

### 2.2 Explicit non-goals for V0.1.0-Alpha

| Non-goal | Reason |
|---|---|
| Touchscreen or tablet layout | No target hardware with a notch or cutout in scope |
| Mobile ports | Different shell constraints entirely |
| Remote control over network | Security surface grows enormously, IPC stays local |
| Plugin loading of arbitrary native `.dll` / `.so` | Unsafe ABI surface, deferred to post 1.0 with a capability sandbox |
| Login or cloud accounts | Contradicts the local-first principle |
| Automatic music file indexing or scraping | MPRIS and GSMTC already expose metadata from every real player |
| Per-app window theming of other applications | Out of scope, Lumina owns only its own surface |
| Shipping a settings GUI | Config file plus tray menu plus live reload covers V0.1.0 |

### 2.3 Design decisions explicitly rejected

| Option | Why rejected for V0.1.0 |
|---|---|
| Tauri / any WebView shell | WebView runtime costs 80 to 150 MB resident, breaks the 40 MB target outright |
| egui immediate mode | Layout rebuild every frame is wasteful for a 60 element overlay, and custom SDF shading fights the retained immediate-mode paint list |
| Slint | Declarative scene graph is comfortable, but per-pixel SDF compositing, custom glow passes, and a shader-driven spectrum all require escaping the abstraction |
| CPU side rendering with `tiny-skia` | Full-screen repaint at 60 fps is unacceptable on laptops and integrated GPUs |

---

## 3. Design Principles and Measurable Targets

### 3.1 Principles

1. **Draw once, morph many.** Geometry is never re-tessellated. A single quad is drawn and the shape is defined analytically in the fragment shader. Morphing is a uniform update.
2. **Sleep when nothing changes.** The render loop does not run at 60 fps forever. It runs only while something is animating, audio is flowing, or a widget is updating. This is the single most important rule for CPU budget.
3. **Uniform updates, zero allocations in the frame loop.** All per-frame data is written into pre-allocated uniform buffers. The frame path performs no heap allocation after startup.
4. **One adapter per operating system, not per application.** Never link to Spotify or Google APIs. Read what the OS already knows.
5. **Every number is a user setting.** The built-in defaults are a starting point, not a policy.
6. **Degrade, never crash.** If audio capture fails, the visualizer reports unavailable. If a shader fails to compile, fall back to a solid fill. If a monitor is missing, log and continue.

### 3.2 Hard targets

| Metric | Target | Measurement method |
|---|---|---|
| Resident memory, idle, collapsed island | below 40 MB | RSS sampled after 60 s with no interaction |
| Resident memory, media expanded plus spectrum at 60 fps | below 80 MB | RSS sampled during playback |
| CPU, fully idle (island collapsed, no audio) | below 0.5 % of one core | 30 s average in `top` / Task Manager |
| CPU, spectrum running at 60 fps, 32 bands | below 1.5 % of one core | 30 s average during playback |
| Frame time, 144 Hz display | below 4 ms GPU, below 1 ms CPU | `VK_EXT_debug_utils` timestamp queries plus frame timestamps |
| Wake from collapsed to media expanded | 280 ms to rest, single overshoot of 4 % | Spring simulation plus on-screen capture |
| Idle redraw duty cycle | zero frames rendered when settled | Frame counter incremented only inside the render path |
| Cold start to first painted frame | below 400 ms Windows, below 600 ms Linux | `Instant` at `main` entry to first `present` |
| Config reload to applied | below 50 ms | Timestamp diff across reload handler |
| Network traffic | zero bytes unless weather is configured | Packet capture during a 10 minute run |

### 3.3 Memory budget breakdown

The 40 MB target is decomposed so optimization work has a target list.

| Allocation | Size | Notes |
|---|---|---|
| Static code and data | 3.5 MB | Stripped release binary |
| wgpu plus graphics driver | 18 MB | Vulkan or D3D12 runtime, largely out of our control |
| Font atlas | 3.0 MB | 1024 x 1024, RGB8 MSDF, built at compile time |
| Icon atlas | 1.0 MB | 1024 x 1024, R8 SDF, built at compile time |
| Album art cache | 4.0 MB max | 16 entries at 256 x 256 RGBA8, LRU evict |
| Window and swapchain | 1.5 MB | Surface and staging buffers |
| Config, theme, widget state | 1.0 MB | serde structures, string interning |
| Audio ring buffer and FFT scratch | 0.1 MB | 8192 sample ring, 4096 point FFT |
| Threads and stacks | 1.5 MB | 6 threads, 256 KB stacks each |
| Slack | 6.4 MB | Reserve for driver variance and fragmentation |
| **Total** | **40.0 MB** | |

---

## 4. Modality Matrix

"Multimodal" here means Lumina consumes and expresses more than one kind of signal at the same time, and combines them.

| Modality | Input | Expression | Combining rule |
|---|---|---|---|
| Geometric | Island state, pointer position | Trapezoid morph, spring overshoot | Master spring drives shape, pointer drives proximity |
| Chromatic | RGB mode, audio bands, album art | Fill, border, glow, per-widget tint | RGB engine resolves to one palette per frame |
| Acoustic | System audio loopback | Spectrum bars, waveform, reactive hue | FFT output feeds both visualizer and color |
| Musical metadata | MPRIS or GSMTC | Title, artist, album art, progress, transport | Media state drives island state machine |
| Temporal | System clock, timers, calendar | Clock, date, countdown, progress ring | Tick scheduler wakes the render loop |
| Machine metrics | `/proc`, Windows APIs | CPU, RAM, network, battery | Sampled at configurable interval, not per frame |
| Environmental | Optional weather API | Temperature, condition, icon | Network call, cached, rate limited |
| Interactive | Click, scroll, hover, hotkey, IPC | State transitions, command execution | All inputs normalize into a single event enum |
| Compositional | User config | Every geometry, color, timing, layout value | Theme and profile layering |

Combination example, the flagship path: audio arrives at 48 kHz, FFT produces 32 band magnitudes, the visualizer widget lays them out as bars, the RGB engine in `reactive` mode maps bass energy to the glow intensity and treble energy to the hue offset, the media widget reports the track title, the island springs from collapsed to the media state because playback started, and a user hotkey simultaneously switches the profile. All of it resolves within one frame.

---

## 5. Platform Support Matrix

### 5.1 Operating systems

| Platform | Minimum | Maximum tested | Architecture | Notes |
|---|---|---|---|---|
| Windows 10 | 1809 (build 17763) | 22H2 | x86_64 | DWM composition required, `WS_EX_LAYERED` for alpha |
| Windows 11 | 21H2 | 25H2 | x86_64 | Mica and Acrylic backdrop available, rounded corners honored |
| Debian | 11 (kernel 5.10) | 13 (kernel 6.12) | x86_64 | Tested on X11 and Wayland sessions |
| Ubuntu | 20.04 (kernel 5.4) | 24.04 (kernel 6.8) | x86_64 | Tested on X11 and Wayland sessions |
| Other Linux | kernel 4.9 or newer | not tested | x86_64 | See 5.4 for the true kernel requirement |

### 5.2 Display servers

| Display server | Status for V0.1.0 | Layer | Mechanism |
|---|---|---|---|
| X11 | Full support, primary Linux target | `_NET_WM_WINDOW_TYPE_DOCK` | EWMH hints, ARGB visual, `XShape` for input |
| Wayland with wlr-layer-shell | Full support | `top` or `overlay` | `zwlr_layer_surface_v1` |
| Wayland on KDE Plasma 6 | Full support | `top` | `zwlr_layer_surface_v1`, Plasma implements it |
| Wayland on GNOME 42+ | Best effort | `top` | GNOME lacks layer-shell, uses `xdg_toplevel` with keep-above, positioning may drift |
| Wayland on GNOME | Documented fallback | `XWayland` | Runs as an X11 client, loses exclusive zone reservation |
| Nested or headless Xvfb | Unsupported | none | No compositor means no correct stacking |

### 5.3 Graphics requirements

| Requirement | Minimum | Fallback |
|---|---|---|
| API | Vulkan 1.1, D3D12, or Metal 3 | OpenGL 3.3 core via wgpu GLES backend |
| GPU memory for Lumina | 32 MB | None, swapchain is the only GPU allocation |
| Shader model | SPIR-V 1.0 for Vulkan | GLSL ES 3.0 for the GLES backend |
| CPU with no GPU | Mesa `llvmpipe` software Vulkan | Works, render loop drops to 30 fps cap automatically |

### 5.4 The true Linux kernel requirement

The user-facing floor is kernel 4.9, derived as follows:

- Vulkan 1.1 needs kernel 4.9 or newer for `VK_KHR_get_physical_device_properties2` and robust buffer access.
- Wayland 1.18 with layer-shell needs kernel 4.9 or newer for shared memory buffer handling.
- DRM render node access for zero-copy buffer presentation needs kernel 3.18 or newer, satisfied.
- PulseAudio shm transport needs kernel 3.17 or newer, satisfied.

Therefore: **kernel 4.9 is the real floor**, while kernel 4.0 to 4.8 may boot Lumina but falls back to the X11 plus OpenGL path with reduced effects. This distinction is documented rather than hidden.

---

## 6. Technical Decisions

Each decision records what was chosen, what was rejected, and the condition that would reverse the choice.

### D1. Window and input: `winit` 0.30

Chosen because it unifies Win32, X11, and Wayland event handling behind one API with a `raw-window-handle` 0.6 handle we can pass to the platform backends for the window-manager specific calls that winit does not expose.

Rejected: `tao` (adds GTK dependency weight on Linux, and we need layer-shell which GTK does not provide), raw `x11rb` plus raw Win32 in one crate (duplicates every abstraction we would otherwise write once).

Reversal condition: if winit drops Wayland layer-shell support permanently, we split Wayland into a dedicated `smithay-client-toolkit` path and keep winit for Win32 and X11 only.

### D2. Rendering: `wgpu` 0.20 with a hand written WGSL pipeline

Chosen for one reason: the entire island is one fragment shader. No retained UI tree, no layout engine, no text mesh rebuild. GPU cost for the island body is one full-screen quad.

Rejected: `wgpu` plus `tiny-skia` for text (CPU raster per glyph per frame), any immediate mode GUI crate.

Reversal condition: none foreseeable within 1.x.

### D3. Media control: OS media session only

Chosen: MPRIS2 over D-Bus on Linux, `GlobalSystemMediaTransportControlsSessionManager` on Windows.

This single decision removes roughly 6,000 lines of per-player scraping code and makes YouTube Music work the moment a browser tab exposes a media session, which Chrome, Firefox, and Edge all do.

Reversal condition: if a specific player refuses to publish a session and users complain, add an opt-in adapter module, never a hard dependency.

### D4. Audio capture: native loopback, no virtual device required

Chosen: `WASAPI` loopback on Windows, PulseAudio or PipeWire monitor source on Linux. Lumina never asks the user to install a virtual sink.

Reversal condition: none. A virtual device requirement would break the zero-setup promise.

### D5. Animation: custom critically tuned springs, not a tween library

Chosen because every morph must be interruptible mid-flight. A spring stores velocity, so retargeting mid animation is continuous. A tween library must either restart or blend, both of which are visible.

### D6. Configuration: TOML with a compiled validation schema

Chosen because TOML is comment friendly, diff friendly, and hand editable, and because `serde` plus `toml` give typed deserialization for free. Validation is a separate pass so a bad value can be rejected per-key instead of failing the whole file.

### D7. Threading: one orchestrator thread owns all mutable state

Chosen because it removes an entire class of data races. Worker threads own their hardware handles and emit events. The orchestrator is the only writer to island state. The renderer reads a snapshot the orchestrator publishes.

### D8. Zero mandatory runtime dependencies

No `.dll`, `.so`, or font library is required at runtime beyond what the graphics driver and libc provide. MSDF atlases are compiled into the binary. This is what makes static-ish distribution and AppImage packaging trivial.

---

## 7. Repository Layout

```
lumina/
├── Cargo.toml                       # workspace manifest, pinned via Cargo.lock
├── rust-toolchain.toml              # pinned stable toolchain
├── PLAN.md
├── README.md
├── CHANGELOG.md
├── LICENSE                           # MIT
├── deny.toml                        # cargo-deny license and advisory policy
├── crates/
│   ├── lumina-core/                  # no_std friendly types shared by all crates
│   │   └── src/
│   │       ├── geometry.rs           # Geometry, Rect, Insets, layout primitives
│   │       ├── state.rs             # IslandState enum, Event enum, StateMachine
│   │       ├── color.rs             # Color, HSL, OkLab, palette math
│   │       ├── spring.rs            # Spring, SpringVec, easings
│   │       └── time.rs              # Monotonic clock, frame pacing, timers
│   │
│   ├── lumina-config/                # schema, parsing, validation, profiles, themes
│   │   └── src/
│   │       ├── schema.rs            # serde structs, one per config section
│   │       ├── validate.rs          # per-key range and enum checks
│   │       ├── defaults.rs          # Default impls, single source of truth
│   │       ├── profile.rs           # inheritance and merge resolution
│   │       ├── theme.rs             # theme file loading and cascading
│   │       └── watch.rs             # notify based hot reload with debounce
│   │
│   ├── lumina-platform/              # display server abstraction
│   │   └── src/
│   │       ├── mod.rs               # Platform enum, detection at startup
│   │       ├── overlay.rs           # OverlayHandle trait: position, size, click-through
│   │       ├── monitors.rs          # monitor enumeration, DPI, selection policy
│   │       └── hotkey/
│   │           ├── mod.rs           # HotkeyRegistry trait
│   │           ├── win32.rs         # RegisterHotKey
│   │           ├── x11.rs           # XGrabKey with passive grab handling
│   │           └── wayland.rs        # via xdg portal GlobalShortcutsPortal
│   │
│   ├── lumina-render/                # wgpu device, pipelines, passes
│   │   ├── src/
│   │   │   ├── device.rs            # adapter selection, surface config, error scopes
│   │   │   ├── uniforms.rs          # GlobalUniforms layout, mirrors, write helpers
│   │   │   ├── pipeline/
│   │   │   │   ├── island.rs        # SDF body, border, glow
│   │   │   │   ├── spectrum.rs      # bar and waveform passes
│   │   │   │   ├── quad.rs          # textured quads: art, icons, progress
│   │   │   │   └── blur.rs          # separable gaussian for backdrop
│   │   │   ├── text.rs              # MSDF sampling and glyph quad emission
│   │   │   └── capture.rs           # frame timing queries, screenshot for tests
│   │   └── shaders/
│   │       ├── common.wgsl          # shared uniforms, sdf helpers, color utils
│   │       ├── island.wgsl
│   │       ├── spectrum.wgsl
│   │       ├── msdf.wgsl
│   │       └── blur.wgsl
│   │
│   ├── lumina-assets/                # build-time generated atlases, include_bytes!
│   ├── build-support/                # build.rs: MSDF generation, icon packing, embedding
│   │
│   ├── lumina-animation/             # spring integration, sequences, choreography
│   ├── lumina-color/                 # RGB engines, palettes, reactive mapping
│   ├── lumina-widgets/               # Widget trait, registry, layout engine
│   │   └── src/
│   │       ├── trait_def.rs
│   │       ├── registry.rs
│   │       ├── layout.rs            # flex row with per-widget weight
│   │       └── widgets/
│   │           ├── media.rs
│   │           ├── spectrum.rs
│   │           ├── clock.rs
│   │           ├── calendar.rs
│   │           ├── monitor.rs
│   │           ├── battery.rs
│   │           ├── network.rs
│   │           ├── pomodoro.rs
│   │           ├── weather.rs
│   │           ├── notify.rs
│   │           └── script.rs
│   │
│   ├── lumina-media/                 # MPRIS and GSMTC behind one trait
│   │   └── src/
│   │       ├── session.rs           # MediaSession trait, MediaSnapshot
│   │       ├── mpris.rs             # Linux
│   │       ├── gsmtc.rs             # Windows
│   │       └── art.rs               # art decode, downscale, dominant color
│   │
│   ├── lumina-audio/                 # capture, FFT, bands, envelope
│   │   └── src/
│   │       ├── capture/
│   │       │   ├── mod.rs           # AudioCapture trait, device selection
│   │       │   ├── wasapi.rs        # Windows loopback
│   │       │   ├── pulse.rs         # PulseAudio monitor
│   │       │   └── pipewire.rs      # PipeWire monitor fallback
│   │       ├── ring.rs              # lock free SPSC ring buffer
│   │       ├── fft.rs               # rustfft wrapper, Hann window, preemphasis
│   │       └── bands.rs             # log band mapping, envelope, normalization
│   │
│   ├── lumina-ipc/                   # server, protocol codec, client library
│   └── lumina-app/                   # the binary
│       └── src/
│           ├── main.rs
│           ├── orchestrator.rs      # single owner of mutable state
│           ├── services.rs          # worker thread spawn and lifecycle
│           ├── input.rs             # pointer, keyboard, scroll normalization
│           ├── tray.rs
│           └── version.rs
├── tools/
│   ├── lumina-send/                  # CLI IPC client
│   ├── lumina-msdf/                  # build-time MSDF atlas generator
│   └── lumina-bench/                 # frame time and FFT micro benchmarks
├── bindings/
│   ├── python/                       # lumina-py, ctypes free, pure stdlib socket
│   └── node/                         # lumina-node, net/unix or named pipe
├── packaging/
│   ├── windows/                      # WiX .wxs, NSIS script, icon resources
│   ├── linux/                        # debian/, rpm/, appimage recipe
│   └── completions/                  # bash, zsh, fish for lumina-send
└── docs/
    ├── configuration.md
    ├── ipc.md
    ├── themes.md
    ├── troubleshooting.md
    └── architecture.md
```

**Crate dependency rule**: `lumina-app` may depend on everything. `lumina-render` may depend only on `lumina-core`, `lumina-config`, and `lumina-assets`. `lumina-core` depends on nothing but `std` and `serde`. This rule is enforced by a unit test in `lumina-core` that greps each crate's `Cargo.toml` for forbidden internal dependencies.

---

## 8. Runtime Architecture

### 8.1 Thread inventory

| Thread | Name | Owns | Emits | Rate |
|---|---|---|---|---|
| Main | `lumina-main` | `wgpu::Instance`, surface, event loop | commands to services | event driven |
| Orchestrator | `lumina-orch` | `IslandState`, config snapshot, widget state | `StateSnapshot` to render | event driven |
| Audio capture | `lumina-audio-cap` | device handle, ring buffer write end | raw samples to analyzer | device callback rate |
| Audio analyzer | `lumina-audio-an` | FFT plan, band array | `BandFrame` at 60 Hz | 60 Hz |
| Media watcher | `lumina-media` | session proxy | `MediaSnapshot` on change | event driven, 1 Hz poll fallback |
| Metrics sampler | `lumina-metrics` | timer | `MetricFrame` | 1 Hz configurable |
| Config watcher | `lumina-cfg` | notify handle | `ConfigUpdate` | on change, 150 ms debounce |
| IPC server | `lumina-ipc` | listener | commands to orchestrator | per request |

Eight threads. Capture and analyzer are split because the device callback must never block: it only writes into a lock-free SPSC ring and returns.

### 8.2 Channel topology

```
                     raw samples, SPSC ring, 8192 f32
 [audio capture] ----------------------------------> [audio analyzer]
                            |  BandFrame, 60 Hz
                            v
 [media watcher] --MediaSnapshot--+
 [metrics sampler] --MetricFrame--+        +--> [orchestrator]
 [config watcher] --ConfigUpdate--+        |     mutates all state
 [ipc server] ------Command-------+        |     publishes StateSnapshot
                                         |
                                         +--> [main thread render]
 [main thread input] -----InputEvent-------^
```

`StateSnapshot` is a plain `Copy` struct written into a triple buffered slot: the orchestrator writes into the free slot, atomically swaps the published index, the renderer reads the published index. No locks, no allocation, no tearing.

### 8.3 Startup sequence

| Step | Action | Budget |
|---|---|---|
| 1 | Parse args, detect single instance via lock file or named mutex | 5 ms |
| 2 | Load config, validate, apply theme and profile layering | 10 ms |
| 3 | Detect display server, open connection, negotiate ARGB visual or layer-shell | 40 ms |
| 4 | Create overlay window at final position before it is shown | 20 ms |
| 5 | Initialize wgpu adapter, device, surface, swapchain | 120 ms |
| 6 | Build pipelines, upload atlases | 60 ms |
| 7 | Spawn services, warm media session enumeration | 90 ms |
| 8 | Render first frame in collapsed state, then reveal with a scale-in | 20 ms |
| **Total** | | **under 400 ms** |

The window is positioned before the first paint so the island never appears to jump from a default location.

### 8.4 Shutdown sequence

Reverse order, with a hard 500 ms budget: stop the render loop, unregister hotkeys, send `Disconnect` to IPC clients, flush logs, release the GPU device with an explicit `device.poll(Maintain::Wait)`, then close the display connection. If the orchestrator is mid-transition, snap all springs to their target so the island does not animate on exit.

---

## 9. Island State Machine

### 9.1 States

| State | Geometry (top half width, bottom half width, height) | Content | Priority |
|---|---|---|---|
| `Collapsed` | 62, 54, 18 | Indicator only, optional clock dot | 0 |
| `Hover` | 66, 56, 22 | Widget icons row, dimmed | 1 |
| `Media` | 180, 168, 40 | Album art, title, artist, transport, progress | 5 |
| `Spectrum` | 210, 200, 50 | 32 bars or waveform, optional mini metadata | 6 |
| `Clock` | 100, 92, 30 | Time, date, calendar strip | 2 |
| `Metrics` | 120, 112, 28 | CPU, RAM, net, battery | 3 |
| `Shortcuts` | 170, 160, 34 | Configured action icons | 4 |
| `Notify` | 200, 190, 28 | Title, body, icon, optional action | 7 |

Values are logical pixels at 100 percent DPI, scaled by the monitor scale factor before they reach the shader.

### 9.2 Transition rules

```
Priority based arbiter:

  new_state = argmax( priority(active_requests) )

  A request is active if:
    - its trigger fired within its hold window, OR
    - the pointer is inside the island bounds, OR
    - it is pinned by config or IPC

Transitions obey:
  - Any state to any state is legal.
  - The master spring is retargeted, never restarted, so velocity carries over.
  - Content crossfades over 140 ms while geometry springs over ~280 ms.
  - If a state with lower priority becomes active while a higher one holds, it is queued.
```

### 9.3 State sources

| Trigger | Raises |
|---|---|
| `PlaybackStatusChanged(Playing)` | `Media`, then `Spectrum` if spectrum is enabled |
| `PlaybackStatusChanged(Stopped)` and no other request | schedules `Collapsed` after `media.hold_ms` |
| Pointer enters bounds | `Hover` if nothing higher is active |
| Hotkey `lumina.cycle_widget` | next widget in the configured order |
| Tray menu item | that widget's state, pinned until dismissed |
| IPC `show` | named state, pinned with optional duration |
| `Metrics.sample` | `Metrics` if `metrics.alert_on_threshold` is set |
| Pomodoro phase change | `Notify` with phase label |
| Weather refresh success | `Notify` only if configured to announce |
| Pointer leaves bounds and hold expired | lowest priority active request, usually `Collapsed` |

### 9.4 Edge cases handled explicitly

| Edge case | Behavior |
|---|---|
| Media session disappears while in `Media` state | Fade content out over 120 ms, transition to `Collapsed` |
| Two players start at once | Higher priority player wins, the other is remembered and shown when the winner stops |
| Track changes mid-morph | Content swaps with a 100 ms crossfade, geometry untouched |
| Audio device unplugged | Spectrum widget reports `audio unavailable`, bars freeze at zero, log a warning once |
| Monitor disconnected while pinned to it | Fall back to primary monitor, log a warning |
| DPI change while running | Recompute scale factor, rescale geometry, keep state |
| Config reload changes `appearance` mid-transition | Springs retarget to the new geometry, no restart |
| Two hotkeys fire within 10 ms | Last one wins, the other is dropped with a debug log |

---

## 10. Shape Mathematics

The island shape is an inverted trapezoid: the top edge is wider than the bottom edge. It is defined analytically so that any geometry change is a uniform update, not a mesh rebuild.

### 10.1 Coordinate convention

The fragment shader works in **local shape space**, origin at the center of the shape, `+x` right, `+y` up. The vertex stage maps local space to clip space using the current geometry uniforms, so the shape is resolution independent and the SDF remains a true distance metric.

Parameters:

- `half_bottom` : half width of the lower edge
- `half_top` : half width of the upper edge, always greater than or equal to `half_bottom`
- `half_h` : half height
- `radius` : corner radius
- `slant` : the horizontal offset per unit height, `slant = (half_top - half_bottom) / (2 * half_h)`

### 10.2 Trapezoid SDF

The un-rounded trapezoid distance field, derived from Inigo Quilez's convex polygon construction:

```wgsl
// Signed distance to an isosceles trapezoid.
// half_bottom and half_top are half widths, half_h is half height.
// Returns negative inside, positive outside, and is a true euclidean
// distance for every point in the plane.
fn sd_trapezoid(p: vec2<f32>, half_bottom: f32, half_top: f32, half_h: f32) -> f32 {
    let k1 = vec2<f32>(half_bottom, half_h);
    let k2 = vec2<f32>(half_top - half_bottom, 2.0 * half_h);

    // fold to the right half plane
    var q = vec2<f32>(abs(p.x), p.y);

    // distance to the two parallel horizontal edges
    let edge_half = select(half_bottom, half_top, q.y > 0.0);
    let d_horizontal = vec2<f32>(q.x - min(q.x, edge_half), abs(q.y) - half_h);

    // distance to the slanted edge, as a segment projection
    let t = clamp(dot(k1 - q, k2) / dot(k2, k2), 0.0, 1.0);
    let d_slanted = q - k1 + k2 * t;

    let inside = d_slanted.x < 0.0 && d_horizontal.y < 0.0;
    let sign = select(1.0, -1.0, inside);
    return sign * sqrt(min(dot(d_horizontal, d_horizontal), dot(d_slanted, d_slanted)));
}
```

This returns a genuine distance, which is what makes the glow, border, and anti-aliasing below correct rather than approximate.

### 10.3 Rounded trapezoid

Rounding a convex polygon is done by insetting every edge by `radius`, then subtracting `radius` from the offset polygon's own SDF. For a trapezoid the offset polygon is still a trapezoid:

- Horizontal edges inset by `radius` give `half_h' = half_h - radius`
- Slanted edges inset by `radius` need a horizontal offset of `radius / cos(slant_angle)`, where `slant_angle = atan(half_top - half_bottom, 2 * half_h)`

```wgsl
fn sd_rounded_trapezoid(
    p: vec2<f32>, half_bottom: f32, half_top: f32, half_h: f32, radius: f32
) -> f32 {
    let r = min(radius, min(half_h, half_top * 0.5));
    let slant_angle = atan2(half_top - half_bottom, 2.0 * half_h);
    let inset_x = r / max(cos(slant_angle), 0.15);

    let d = sd_trapezoid(
        p,
        half_bottom - inset_x,
        half_top - inset_x,
        half_h - r
    );
    return d - r;
}
```

Sanity check: when `half_top == half_bottom`, `slant_angle` is zero, `inset_x` becomes `r`, and the function reduces exactly to a rounded rectangle. That degeneracy is asserted in a unit test.

### 10.4 Anti-aliasing

Because the SDF is a true distance, one screen pixel corresponds to `fwidth(d)` of shape space:

```wgsl
let aa = fwidth(d);
let coverage = 1.0 - smoothstep(-aa, aa, d);
```

No supersampling, no MSAA, no extra passes. Edge quality is exact at any zoom level.

### 10.5 Border

A border is the band where the SDF sits between two thresholds:

```wgsl
let outer = sd_rounded_trapezoid(p, hb, ht, hh, radius);
let inner = sd_rounded_trapezoid(p, hb, ht, hh, max(radius - border_width, 0.0));
let border_mask = smoothstep(-aa, aa, outer) * (1.0 - smoothstep(-aa, aa, inner));
```

`border_width` is in logical pixels and is divided by the scale factor before entering the shader so borders stay hairline-thin on HiDPI displays.

### 10.6 Glow

Glow is the outer SDF sampled at a larger radius, with an exponential falloff:

```wgsl
let glow_d = sd_rounded_trapezoid(p, hb + glow_spread, ht + glow_spread, hh + glow_spread, radius + glow_spread);
let glow = exp(-max(glow_d, 0.0) / max(glow_falloff, 0.001)) * glow_intensity;
```

Exponential falloff rather than a smoothstep band, because falloff matches how light behaves and it avoids a visible ring at the glow boundary. `glow_spread` is clamped so the glow never exceeds the render target and gets clipped.

### 10.7 Backdrop blur

Where the compositor cannot blur for us, we blur ourselves:

1. Copy the region behind the island into a small offscreen texture at 1/4 resolution.
2. Run a separable Gaussian, 9 taps horizontal then 9 taps vertical.
3. Sample the result in the island shader and composite with the fill.

Cost is under 0.3 ms at 360 x 100. On Windows the native path is preferred (`SetWindowCompositionAttribute` with `ACCENT_ENABLE_ACRYLICBLURBEHIND`), and the software path is only a fallback for Windows 10 builds without the undocumented attribute. On X11 the software path is the default because there is no portable compositor API, and on KWin it is skipped when `KWinBlur` reports availability.

### 10.8 Geometry clamping

All user-provided geometry passes through a clamp before reaching the GPU, so a config typo cannot produce an invisible or screen-filling island:

| Parameter | Min | Max |
|---|---|---|
| `half_top` | 40 px | 400 px |
| `half_bottom` | 32 px | 400 px |
| `half_h` | 8 px | 120 px |
| `radius` | 0 px | `min(half_h, half_top) ` |
| `border_width` | 0 px | 8 px |
| `glow_spread` | 0 px | 40 px |
| `slant` | 0.0 | 0.6 |

`half_bottom` is additionally clamped to at most `half_top`, which enforces the inverted trapezoid invariant even if the user reverses the config values.

---

## 11. Rendering Engine

### 11.1 Frame graph

Every frame executes at most four passes. Typical frames execute two.

| Pass | Bind group | Draw | Purpose |
|---|---|---|---|
| P0 Backdrop blur | scene color, blur ping, blur pong | 2 quads | Only when software blur is active |
| P1 Island body | global uniforms, sampler | 1 full-target quad | SDF fill, border, glow in one shader |
| P2 Content | global uniforms, atlas, sampler | 1 instanced quads | album art, icons, text, progress, controls |
| P3 Spectrum | global uniforms, band storage buffer | 1 quad, 32 instances | bars or waveform |

The island body is a single quad covering the whole swapchain. Cost is fragment-bound but the fragment shader early-outs on `d > glow_spread`, so only the island's screen area runs the expensive path. On a 2560 x 1440 display the island covers roughly 0.4 percent of pixels.

### 11.2 Uniform layout

```wgsl
struct IslandUniforms {
    // geometry, in shape space (centered origin)
    half_bottom      : f32,
    half_top         : f32,
    half_h           : f32,
    radius           : f32,

    // appearance
    fill_rgba        : vec4<f32>,   // premultiplied
    border_rgba      : vec4<f32>,
    border_width     : f32,
    glow_rgb         : vec3<f32>,
    glow_intensity   : f32,

    glow_falloff     : f32,
    glow_spread      : f32,
    fill_opacity     : f32,
    noise_strength   : f32,          // subtle dither, kills banding

    // transform
    origin_px        : vec2<f32>,   // island center in target pixels
    inv_extent       : vec2<f32>,   // 1 / shape half extents in pixels
    resolution       : vec2<f32>,
    time_seconds     : f32,
    dpr              : f32,

    // flags packed as f32 lanes to keep 16 byte alignment
    flags            : u32,          // bit0 blur_active, bit1 border_soft, bit2 dither
    content_fade     : f32,
    backdrop_rgba    : vec4<f32>,
};
```

Total 176 bytes, comfortably inside the 64 KB uniform buffer limit. Updated with a single `queue.write_buffer` per frame.

### 11.3 Device and surface configuration

| Setting | Value | Reason |
|---|---|---|
| `alpha_mode` | `PreMultiplied` | Matches our premultiplied fill math and avoids per-pixel divide in compositors |
| `format` | preferred, `bgra8unorm-srgb` preferred | sRGB textures get free gamma correct blending |
| `usage` | `RENDER_ATTACHMENT` only | No need for `COPY_SRC` in normal operation |
| `present_mode` | `Fifo` when vsync on, `Immediate` otherwise | Fifo caps at refresh rate, Immediate frees latency |
| `desired_maximum_frame_latency` | 2 | One frame of slack absorbs GC-like allocator jitter |
| `power_profile` | `LowPower` on battery | Adapters that expose it save meaningful power |
| `backbuffer_count` | 2 by default, 3 if measured frame time exceeds 6 ms | Extra buffer only when needed |

### 11.4 Error handling

Every pipeline creation is wrapped in `push_error_scope(Validation)`. A validation error logs the WGSL source line, then retries with the pipeline compiled against the GLES backend. If both fail, the island renders as a solid rounded rect using the fallback shader, and a tray notification tells the user their GPU driver rejected the effects pipeline. Lumina never exits because of a shader.

### 11.5 Render loop and the dirty flag

```rust
loop {
    let events = platform.pump_events();
    let dirty = orchestrator.handle(events);          // true if state changed
    let animating = orchestrator.springs_at_rest();    // inverted: true if moving

    if dirty || animating || audio.active() {
        render(orchestrator.snapshot());
        frame_counter.increment();
    } else {
        wait_for_event_or_timer(TICK);                 // blocks, zero CPU
    }
}
```

The idle branch blocks on a channel receive with a timeout equal to the next scheduled tick, which is the 1 Hz clock tick or the next metrics sample. This is what makes 0.5 percent idle CPU achievable rather than aspirational.

### 11.6 Frame pacing

Presentation is driven by the surface, not by a timer. The CPU submits work for frame N, then processes events while the GPU works, then submits frame N+1 after `acquire_texture` returns. If `acquire_texture` blocks, the CPU idles for free. A `FramePacer` clamps the submit rate to `target_fps` when `vsync = false`.

---

## 12. Animation System

### 12.1 The spring

All motion is a damped harmonic oscillator integrated with semi-implicit Euler:

```
a  = -k * (x - target) - c * v
v += a * dt
x += v * dt
```

Parameters are expressed the way designers think, in `response` and `damping_ratio`, then converted:

```
omega = 2 * PI / response          // natural frequency, rad/s
k     = omega * omega              // stiffness
c     = 2 * damping_ratio * omega  // damping coefficient
```

| `damping_ratio` | Behavior |
|---|---|
| below 0.7 | visibly bouncy, multiple overshoots |
| 0.7 to 0.9 | single small overshoot, the target feel for island morphs |
| 1.0 | critically damped, fastest approach with no overshoot |
| above 1.0 | sluggish, long tail |

### 12.2 The morph spring

One master spring drives a scalar `t` in `[0, 1]`. Geometry is interpolated between the outgoing and incoming state by evaluating their parameter sets at `smoothstep(t)`:

```
half_top    = mix(from.half_top,    to.half_top,    ease(t))
half_bottom = mix(from.half_bottom, to.half_bottom, ease(t))
half_h      = mix(from.half_h,      to.half_h,      ease(t))
radius      = mix(from.radius,      to.radius,      ease(t))
```

Why one spring rather than one per property: a single `t` guarantees the shape stays on the family of valid inverted trapezoids throughout the transition. Independent springs would let `half_bottom` overtake `half_top` mid-flight and the shape would briefly invert, which looks broken.

Default morph spring: `response = 0.34 s`, `damping_ratio = 0.78`. This yields roughly 280 ms to rest with a 4 percent overshoot.

### 12.3 Additional springs

| Spring | Response | Damping | Drives |
|---|---|---|---|
| `morph` | 0.34 s | 0.78 | Geometry interpolation scalar |
| `content_fade` | 0.16 s | 1.0 | Opacity of the content layer |
| `hover_glow` | 0.12 s | 0.85 | Border brightness on pointer hover |
| `reveal` | 0.42 s | 1.0 | Startup scale-in from 0.9 |
| `notify_slide` | 0.28 s | 0.85 | Notify content horizontal entry offset |
| `profile_crossfade` | 0.30 s | 1.0 | Color transition when switching profiles |

### 12.4 Integration stability

Semi-implicit Euler is unstable for stiff springs at low frame rates. Guard: integrate in fixed sub-steps of `1 / 240 s`, with a maximum of 8 sub-steps per frame. If a frame takes longer than 33 ms, the spring advances only 33 ms of simulated time, so animation slows under load rather than exploding. The total simulated time is clamped to 100 ms per frame to avoid a catch-up burst after a long stall.

### 12.5 Sequences and choreography

Widgets may request named sequences, each defined as a list of timed actions:

```toml
[[sequences.on_media_start]]
at_ms = 0
action = "expand"
to = "media"

[[sequences.on_media_start]]
at_ms = 120
action = "fade_in"
target = "content.media"

[[sequences.on_media_start]]
at_ms = 260
action = "pulse"
target = "glow"
amplitude = 1.4
duration_ms = 180
```

Supported actions: `expand`, `collapse`, `fade_in`, `fade_out`, `pulse`, `shake`, `slide`, `set_geometry`, `switch_profile`. Sequences run on the orchestrator thread and are cancellable by any state change.

### 12.6 Reduced motion

When `motion.reduced = true` in config, or the OS reports reduced motion, every spring switches to `response = 0.08`, `damping_ratio = 1.0`. Transitions still happen, they simply become near-instant and never overshoot. Pulse, shake, and slide sequences are skipped entirely.

---

## 13. Color and RGB Engine

### 13.1 Color pipeline

All user colors are parsed as sRGB hex, including optional alpha, and converted internally to **OkLab** (`oklab`), because OkLab is perceptually uniform. Interpolating in sRGB produces muddy midpoints; interpolating in OkLab does not. This matters because every Lumina transition is a color transition.

```
sRGB (0..1) -> linear -> LMS -> OkLab
```

Primary palette is resolved once per frame into:

```rust
struct Palette {
    fill:        OkLab,
    fill_accent: OkLab,   // second stop for gradients
    border:      OkLab,
    glow:        OkLab,
    spectrum:    Vec<OkLab>, // one per band
    text:        OkLab,
    text_dim:    OkLab,
}
```

### 13.2 The five engines

| Engine | Behavior | Inputs used |
|---|---|---|
| `static` | Fixed `primary`, border and glow derived by lightness and chroma offsets | none |
| `breathing` | `brightness *= 1 + amp * sin(2*PI*rate*t)` | `speed`, `breath_rate` |
| `cycling` | Hue rotates through the full wheel at `speed` degrees per second | `speed`, `primary` sets the chroma and saturation anchor |
| `reactive` | Three audio aggregates drive the palette, detailed in 13.3 | band energies |
| `album` | Dominant colors extracted from current album art build the palette | cover art |

### 13.3 Reactive engine detail

Band energies are aggregated into three normalized scalars:

```
bass   = mean(bands[0..bass_end])        // default 20 to 250 Hz
mid    = mean(bands[bass_end..mid_end])  // 250 to 2000 Hz
treble = mean(bands[mid_end..N])         // 2000 Hz to Nyquist
```

Mapping:

```
hue_offset      = treble * 90            // degrees, cool shift on bright content
lightness_gain  = bass * 0.18            // brightens on heavy low end
chroma_gain     = (bass * 0.5 + mid * 0.5) * 0.25
glow_intensity  = base + bass * 0.55     // glow breathes with the kick drum
border_light    = base + mid * 0.30
```

All three aggregates pass through the same envelope follower as the visualizer, so the palette does not flicker on transients. `reactive.gain` scales the whole mapping, and `reactive.floor` sets how much is always on.

### 13.4 Album engine detail

1. Decode cover art to a 32 x 32 RGBA buffer.
2. Discard pixels with alpha below 128.
3. Convert to OkLab.
4. Run 8 iterations of k-means with k = 4, seeded by a deterministic histogram of the 32 x 32 pixels so the result is stable frame to frame.
5. Order clusters by population.
6. `fill` is cluster 0 scaled to a user-controlled darkness, `fill_accent` is cluster 1, `border` is cluster 2 brightened, `glow` is cluster 3.
7. If no art is available, fall back to `static`.

Recompute only when the track changes or the art hash changes, cached per track id.

### 13.5 Per-widget color overrides

```toml
[widgets.media.rgb]
override = true
mode = "static"
primary = "#ff2d55"
glow_intensity = 0.9
```

Precedence, highest first:

1. Per-widget override
2. Active profile
3. Active theme
4. Base config
5. Built-in default

### 13.6 Smooth mode switching

Switching engines crossfades over 300 ms by interpolating the whole `Palette` in OkLab with `damping_ratio = 1.0`. No engine ever snaps. Switching from `reactive` to `static` fades the audio contribution out rather than cutting it.

### 13.7 Band to color mapping for the spectrum

Each of the 32 bands receives a hue offset from the palette's primary:

```
band_hue = primary_hue + (band_index / band_count - 0.5) * spectrum.hue_spread
```

`spectrum.hue_spread` defaults to 120 degrees, so a violet palette becomes a violet to blue gradient across the bars. Setting it to 0 makes every band the base color, which some users prefer. Each band's lightness is modulated by its own magnitude, and peak-hold markers use a desaturated version of the band color.

---

## 14. Typography and Icon Pipeline

### 14.1 Why the atlas is built at compile time

Generating MSDF at runtime costs 300 to 800 ms and several megabytes of transient memory. Lumina ships pre-generated atlases embedded with `include_bytes!`, which costs zero startup time and zero heap. Custom user fonts get a runtime fallback path that is slower but only triggers when the user supplies their own font.

### 14.2 MSDF generation pipeline

Build step, run by `tools/lumina-msdf` from `build.rs`:

1. Load `assets/fonts/Inter-Regular.ttf` and `Inter-SemiBold.ttf`.
2. Collect the glyph set: ASCII 0x20 to 0x7E, the Latin-1 supplement, plus punctuation and symbols `U+00B0 U+00A9 U+00AE U+2122 U+20AC U+00A3 U+00A5 U+00A2 U+2022 U+2192 U+2190 U+2191 U+2193 U+00D7 U+00F7 U+2212 U+2013 U+2014 U+2018 U+2019 U+201C U+201D U+2026 U+2030 U+03B1 U+03B2 U+03B3 U+03C0`.
3. For each glyph, run `msdfgen` in error correction mode with a 4 x 4 px range and 48 px em size.
4. Pack with a shelf packer into 1024 x 1024, sorting by height.
5. Emit `msdf_atlas.bin` (RGB8, 3 bytes per pixel) and `msdf_atlas.json` with per-glyph rect, plane bounds, and atlas bounds.
6. Assert at build time that the atlas is at most 90 percent full, failing the build if the glyph set grows beyond capacity.

Result: 1024 x 1024 x 3 = 3.0 MB resident, zero allocation at runtime.

### 14.3 Text rendering

```wgsl
fn sample_msdf(texel: vec3<f32>, screen_px_range: f32) -> f32 {
    // reconstruct signed distance from the median of the three channels
    let sd = median(texel.r, texel.g, texel.b);
    let d = (sd - 0.5) * 2.0;                 // remap to -1..1
    let w = fwidth(d) + 0.0001;                // one pixel in shape space
    return clamp(d / w + 0.5, 0.0, 1.0);      // coverage
}
```

`screen_px_range` is derived from the glyph quad scale so outlines stay correct at any font size, including the marquee which renders at fractional sizes. Sharp mode is available via `text.sharp = true`, switching to a single threshold instead of smooth coverage.

### 14.4 Icon pipeline

Icons are monochrome SVGs in `assets/icons/`, converted at build time to a 1024 x 1024 single-channel SDF atlas. 1.0 MB resident. The bundled set covers playback controls, calendar, clock, battery, wifi, cpu, memory, network, weather, pomodoro, bell, settings, power, volume, cast, and generic action glyphs.

Users can add icons by dropping `.svg` files into the platform icons directory. They are rasterized on demand at startup into a second, dynamically grown atlas. Maximum 64 user icons, 256 x 256 each, R8, so 4 MB worst case and 0 in the common case.

### 14.5 Text layout

A minimal shaper, because our strings are short and Latin-script:

1. Segment by Unicode grapheme cluster using `unicode-segmentation`.
2. Map each cluster to a glyph via the font's cmap.
3. Apply kerning from the font's `kern` table when present.
4. Measure with `fontdue`'s advance widths, cached per (font, glyph).
5. Lay out left to right, with optional ellipsis truncation when `text.truncate = "ellipsis"` and when marquee is disabled.

Right-to-left scripts are out of scope for V0.1.0 and fall back to left-to-right with a warning logged once if Arabic or Hebrew text is detected.

### 14.6 Marquee

Long titles scroll. Implementation: a per-string measure, then an offset animated by a dedicated spring-free constant-velocity timer with `scroll_speed` pixels per second. A gap of `scroll_gap` (default 48 px) is inserted between the end and the start of the repetition. The text pauses for `scroll_pause_ms` when the pointer enters the island, so the user can actually read it. Titles that fit are never animated.

---

## 15. Widget System and Layout

### 15.1 The trait

```rust
pub trait Widget: Send {
    /// Stable identifier used in config and IPC.
    fn id(&self) -> &'static str;

    /// Called on the orchestrator thread. Return the geometry this widget
    /// wants inside the island, in logical pixels.
    fn measure(&mut self, avail: Size, ctx: &WidgetContext) -> Size;

    /// Emit draw commands into the frame's command buffer. No allocation.
    fn draw(&mut self, out: &mut DrawList, ctx: &WidgetContext);

    /// Handle a normalized input event. Return true if consumed.
    fn on_input(&mut self, ev: &WidgetInput, ctx: &WidgetContext) -> bool;

    /// Does this widget want the island in its state right now?
    fn request(&self, ctx: &WidgetContext) -> Option<WidgetRequest>;

    /// Highest value wins in the state arbiter.
    fn priority(&self) -> u8;

    /// Called when the widget's state leaves the island, to release resources.
    fn on_hide(&mut self) {}
}
```

`measure` returns a size, not a layout tree. The layout engine is a single-row flexbox with weights, which is all a horizontal island needs.

### 15.2 Layout algorithm

```
available = island_inner_width - padding * 2

1. For each visible widget: measure(min_size, available)
2. total = sum(base_width)
3. if total > available: shrink proportionally to (min_width, base_width)
4. if total < available: distribute the surplus by weight
5. place left to right with spacing, honoring per-widget align (top, center, bottom)
```

Padding, spacing, and weights come from `[layout]` in config. Widgets may set `flex = 0` to pin their width regardless of surplus.

### 15.3 Widget catalog

| Widget | Data source | Refresh | Notes |
|---|---|---|---|
| `media` | MPRIS or GSMTC | event driven | Art, title, artist, transport, progress, volume |
| `spectrum` | Audio engine | 60 Hz | Bars, waveform, or mirror bars |
| `clock` | `chrono` | 1 Hz, or 10 Hz with seconds | Locale aware, second timezone optional |
| `calendar` | `chrono` | 1 Hz | Month grid, today highlighted, event dots |
| `metrics` | `/proc`, `GetSystemTimes` | 1 Hz | CPU, RAM, GPU, network up and down |
| `battery` | `/sys/class/power_supply`, `GetSystemPowerStatus` | 5 Hz when discharging | Percentage, charging bolt, time remaining |
| `network` | `/proc/net/dev`, `GetIfTable2` | 1 Hz | Up and down rates, optional SSID on Linux |
| `pomodoro` | Internal timer | 1 Hz | Focus and break phases, progress ring, phase label |
| `weather` | OpenWeatherMap or Open-Meteo | 15 min | Optional, requires user-supplied key |
| `notify` | IPC and internal events | on event | Title, body, icon, optional action button |
| `script` | User-defined shell command | on demand | Executes a command, shows stdout for N ms |
| `shortcuts` | Config | static | Row of configured action icons |

### 15.4 Media widget detail

Layout, left to right: album art (52 px, rounded 8 px), then a two-line text block (title above artist, dimmer), then transport controls (prev, play or pause, next), then a full-width progress bar at the bottom edge of the island, 3 px tall, rounded, filled with the palette glow color.

Behaviors:

- **Click on art** toggles the expanded media layout between compact and full.
- **Click on the progress bar** seeks. The bar shows a hover scrub preview with a time tooltip.
- **Title marquee** when the title exceeds the available width.
- **Volume** is shown only when `media.show_volume = true`, as a small horizontal slider, and it maps to MPRIS `Volume` property.
- **Multiple players** are cycled with the `media.cycle_players` hotkey; the current player name appears in the widget's tooltip.
- **Unknown player identity** falls back to the MPRIS `Identity` or the GSMTC `SourceAppUserModelId`, and if that is also missing, to `Unknown`.

### 15.5 Metrics widget detail

On Linux, CPU usage comes from `/proc/stat` sampled twice per interval (delta based, not since boot). RAM from `/proc/meminfo`. Network from `/proc/net/dev`. GPU load is read from `/sys/class/drm/card*/device/gpu_busy_percent` where present. On Windows, `GetSystemTimes` for CPU, `GlobalMemoryStatusEx` for RAM, `GetIfTable2` for network, and NVML or DXGI for GPU where available.

All samplers run on the metrics thread at 1 Hz. If `/proc` is not mounted (some containers), the widget renders a dash and logs once.

### 15.6 Weather widget detail

Default provider is Open-Meteo, which requires no API key, with OpenWeatherMap as an option for users who want icons. The response is cached on disk with a 15 minute TTL so Lumina works offline showing the last known reading plus a staleness indicator. Network failure is silent by default and only logged at warn level, never shown as an error popup.

---

## 16. Media Integration

### 16.1 The abstraction

```rust
pub trait MediaSession: Send {
    fn identity(&self) -> String;
    fn metadata(&self) -> &Metadata;                 // title, artist, album, art, length
    fn playback_status(&self) -> PlaybackStatus;      // Playing, Paused, Stopped
    fn position(&self) -> Duration;                   // cached, polled at 1 Hz
    fn volume(&self) -> f32;                          // 0.0 to 1.0
    fn play(&self) -> Result<()>;
    fn pause(&self) -> Result<()>;
    fn next(&self) -> Result<()>;
    fn previous(&self) -> Result<()>;
    fn seek_to(&self, t: Duration) -> Result<()>;
}
```

`MediaWatcher` owns all sessions, ranks them, and publishes a single `MediaSnapshot` for the active player. Ranking:

```
score = 100 if Playing
      + 30 if metadata is complete (title and artist present)
      + 20 if the player is in the configured preferred list
      - 50 if the session is a browser with no window focused
```

### 16.2 Linux MPRIS

- Bus name: `org.mpris.MediaPlayer2.<instance>`
- Object path: `/org/mpris/MediaPlayer2`
- Interface: `org.mpris.MediaPlayer2.Player`
- Properties: `PlaybackStatus`, `Metadata` (`mpris:trackid`, `mpris:length`, `xesam:title`, `xesam:artist`, `xesam:album`, `mpris:artUrl`), `Volume`, `Position`
- Methods: `Next`, `Previous`, `Pause`, `PlayPause`, `Stop`, `Seek`
- Signals: `Seeked`, `PropertiesChanged`
- Discovery: `NameOwnerChanged` on `org.freedesktop.DBus`, plus a full enumeration at startup
- `Position` must be read with the `org.mpris.MediaPlayer2.Player` `Position` property and then tracked locally by adding elapsed time, because the property does not emit change signals. A 1 Hz poll corrects drift.

Art URL handling:

| URL scheme | Action |
|---|---|
| `file://` | Read directly |
| `http://` / `https://` | Fetch once, cache to disk, respect a 2 MB cap |
| `data:` | Decode in memory |
| Anything else | Show a generic music note icon |

### 16.3 Windows GSMTC

- `GlobalSystemMediaTransportControlsSessionManager::request_async()` to get the manager
- `get_sessions()` to enumerate
- `CurrentMediaSession` from `PlaybackInfo` and `PlaybackStatus`
- `SystemMediaTransportControlsSessionManager::get_current_session()` for the active one
- Async WinRT calls are bridged with a small `block_on` helper on a dedicated thread pool, never on the render thread
- Art arrives as an `IRandomAccessStream`, read into memory, decoded with the `image` crate

Minimum supported Windows version for GSMTC is 10 1809, which matches our floor. On older builds the widget reports `media unsupported` and the rest of Lumina runs normally.

### 16.4 Supported players, verified in testing

| Player | Platform | Mechanism |
|---|---|---|
| Spotify desktop | both | native session |
| Spotify in Chrome / Edge / Firefox | both | browser media session |
| YouTube Music in browser | both | browser media session |
| Any web audio in Chrome / Edge / Firefox | both | browser media session |
| VLC | both | native session |
| mpv | Linux | `mpv --idle` with MPRIS plugin, or `mpris` support built in since 0.34 |
| Rhythmbox | Linux | native MPRIS |
| Strawberry, Clementine, Audacious | Linux | native MPRIS |
| Windows Media Player, Groove | Windows | native GSMTC |
| Anything with a browser media session | both | works automatically |

### 16.5 Failure behavior

| Condition | Behavior |
|---|---|
| No sessions found | Media widget hidden, no error shown |
| Session disappears mid-command | Command returns `PlayerGone`, snapshot clears, state machine transitions to Collapsed |
| Player does not implement `Seek` | Progress bar becomes read only, clicking does nothing |
| Art fetch fails | Generic icon, cached negative result for 60 s |
| D-Bus unavailable on Linux | Media widget hidden, log a warn with the D-Bus error text |
| WinRT activation fails | Media widget hidden, log a warn, rest of the app unaffected |

---

## 17. Audio Engine

### 17.1 Capture abstraction

```rust
pub trait AudioCapture: Send {
    fn open(device: &DeviceSelector) -> Result<Self, AudioError>;
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u16;
    fn start(&mut self, sink: RingWriter<f32>) -> Result<(), AudioError>;
    fn stop(&mut self);
}
```

| Platform | Backend | Device | Notes |
|---|---|---|---|
| Windows | `WASAPI` | `AudioLoopbackCapture` on the default render endpoint | Always stereo, resampled from the endpoint mix format |
| Linux, PulseAudio present | `libpulse-binding` | `sink.monitor` source of the default sink | Native rate, usually 44.1 or 48 kHz |
| Linux, PipeWire present | `pipewire-rs` | `Audio/Source` node with `node.pw-loopback` | Preferred when available, PipeWire is the future default |
| Linux, neither | none | none | Spectrum widget renders `audio unavailable` |

Device selection is configurable: `default`, `sink_monitor`, or an explicit device name substring.

### 17.2 Sample path

```
device callback (48 kHz, 20 ms period = 960 frames)
  -> downmix to mono f32
  -> DC removal (subtract running mean, alpha = 0.999)
  -> highpass at 30 Hz (one pole) to kill rumble and DC offset
  -> write into SPSC ring (8192 samples = 170 ms of slack)
```

The callback does no allocation and no locking. If the ring is full, samples are dropped rather than blocking, because blocking an audio callback produces a click in the user's music, which is far worse than a slightly stale spectrum.

### 17.3 FFT configuration

| Parameter | Default | Rationale |
|---|---|---|
| `fft_size` | 4096 | At 48 kHz, bin width is 11.7 Hz, which resolves the 20 to 60 Hz range that 2048 cannot |
| `window` | Hann | Standard, good sidelobe suppression |
| `overlap` | 75 % | Hop size 1024 at 48 kHz gives 46.9 frames per second, upsampled to 60 Hz by holding the most recent frame |
| `sample_format` | mono f32 | Stereo is summed, visualizer has no use for it |
| `band_count` | 32 | Configurable 8 to 128 |
| `band_range` | 20 Hz to 16 kHz | Lower bound above human hearing's practical floor for music, upper bound clamped to Nyquist |

Band frequency edges, logarithmic:

```
f_i = f_min * (f_max / f_min) ^ (i / band_count),  i in 0..band_count
```

At 32 bands this gives roughly one bar per semitone and a half in the low range and much coarser spacing up top, which matches how the ear groups frequencies.

Each band magnitude is the mean of magnitude bins inside its edge range, converted to dB:

```
db = 20 * log10(magnitude + 1e-9)
```

### 17.4 Envelope follower

Raw FFT frames are far too jumpy to render. Each band gets an asymmetric one pole envelope:

```
if current > envelope:
    envelope += attack  * (current - envelope)     # fast rise
else:
    envelope += release * (current - envelope)     # slow fall
```

`attack` default 0.35, `release` default 0.08, both configurable. The slow release is what gives bars a natural decay instead of strobing.

### 17.5 Normalization and noise gate

1. **Gate**: if overall RMS over the last 500 ms is below `audio.silence_threshold` (default -60 dBFS), all bands ramp to zero over 200 ms and the spectrum state becomes `Silent`.
2. **Normalize**: track a rolling maximum per band with a 3 second decay window, then scale so the loudest band reaches 1.0. Prevents a quiet track from showing a dead spectrum and a loud track from clipping the display.
3. **Bass boost**: apply `+3 dB/octave` tilt below 200 Hz, because music has far more energy there and an untilted spectrum looks flat.

### 17.6 Spectrum rendering

Bars mode, default:

- Bar width `(inner_width - spacing * (n-1)) / n`, spacing 2 px, minimum bar width 2 px.
- Each bar's height is `magnitude * max_bar_height`, with a minimum of 1 px so idle bands still show a baseline tick.
- Top 4 px of each bar is rounded, matching the island's corner language.
- A peak-hold marker sits above each bar, falling at 0.6 px per frame, and disappears after 400 ms.
- Each bar samples the palette with its own hue offset and modulates lightness by magnitude.
- Bars above 90 percent magnitude get a subtle white hot tip, which reads as clipping without looking like an error.

Waveform mode: a time-domain min/max envelope per pixel column, drawn as a filled symmetric shape. Uses the ring buffer directly with no FFT.

Mirror mode: draw the spectrum vertically centered, symmetric about the horizontal midline.

### 17.7 CPU cost

Measured targets for a 32 band, 4096 point FFT at 46.9 frames per second on a 2019 class laptop CPU:

| Stage | Budget |
|---|---|
| Capture callback, 20 ms period | 0.15 ms per callback |
| Ring write and read | below 0.01 ms |
| Hann window multiply, 4096 samples | 0.004 ms |
| rustfft forward real 4096 | 0.035 ms |
| Band mapping, 32 bands | 0.002 ms |
| Envelope, 32 bands | 0.001 ms |
| **Total per second** | **about 2.2 ms, roughly 0.22 % of one core** |

The GPU side is one instanced draw of 32 quads, negligible.

### 17.8 Microphone as an alternative source

`audio.source = "microphone"` is available for users who want the spectrum to react to the room rather than to system output. It is off by default and the config comment states plainly that it captures audio from the input device.

---

## 18. Configuration System

### 18.1 Location and precedence

| Platform | Primary | User override |
|---|---|---|
| Windows | `%APPDATA%\Lumina\config.toml` | same |
| Linux | `$XDG_CONFIG_HOME/lumina/config.toml` | same |

Precedence, lowest to highest:

1. Built-in defaults (`lumina-config::defaults`)
2. Base `config.toml`
3. Active theme file, if set
4. Active profile (may inherit from another profile)
5. Command line arguments
6. IPC commands issued during the session (session only, not persisted unless asked)

### 18.2 Full schema

Every key, its type, its range, its default, and whether it can hot reload.

#### `[general]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `position` | enum | `top-center`, `top-left`, `top-right`, `bottom-center`, `bottom-left`, `bottom-right` | `top-center` | yes |
| `offset_x` | i32 | -3840 to 3840 | `0` | yes |
| `offset_y` | i32 | -2160 to 2160 | `12` | yes |
| `monitor` | int or enum | `0..N` or `primary`, `cursor` | `primary` | yes |
| `auto_hide` | bool | | `false` | yes |
| `auto_hide_delay_ms` | u32 | 0 to 60000 | `3000` | yes |
| `always_clickable` | bool | | `false` | yes |
| `idle_timeout_ms` | u32 | 0 to 60000 | `5000` | yes |
| `click_through_when_collapsed` | bool | | `true` | yes |
| `start_hidden` | bool | | `false` | yes |
| `single_instance` | bool | | `true` | yes |

#### `[appearance]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `fill` | color | `#RRGGBB` or `#RRGGBBAA` | `#12121ACC` | yes |
| `fill_gradient` | float | 0.0 to 1.0 | `0.0` | yes |
| `corner_radius` | u32 | 0 to 60 | `18` | yes |
| `top_half_width` | u32 | 40 to 400 | `62` | yes |
| `bottom_half_width` | u32 | 32 to 400 | `54` | yes |
| `height` | u32 | 8 to 120 | `36` | yes |
| `inverted_trapezoid` | bool | | `true` | yes |
| `border_width` | f32 | 0.0 to 8.0 | `1.0` | yes |
| `border_color` | color | | `#FFFFFF24` | yes |
| `glow_spread` | u32 | 0 to 40 | `8` | yes |
| `glow_falloff` | f32 | 0.5 to 40.0 | `8.0` | yes |
| `shadow_opacity` | f32 | 0.0 to 0.8 | `0.28` | yes |
| `backdrop_blur` | enum | `auto`, `on`, `off` | `auto` | yes |
| `backdrop_darken` | f32 | 0.0 to 0.8 | `0.15` | yes |
| `noise_strength` | f32 | 0.0 to 0.05 | `0.012` | yes |
| `corner_style` | enum | `rounded`, `squircle`, `chamfer` | `rounded` | restart |

#### `[rgb]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `mode` | enum | `static`, `breathing`, `cycling`, `reactive`, `album` | `breathing` | yes |
| `primary` | color | | `#7C3AED` | yes |
| `secondary` | color | | `#06B6D4` | yes |
| `speed` | f32 | 0.05 to 8.0 | `1.0` | yes |
| `brightness` | f32 | 0.0 to 2.0 | `1.0` | yes |
| `saturation` | f32 | 0.0 to 2.0 | `0.9` | yes |
| `breath_rate_hz` | f32 | 0.05 to 2.0 | `0.25` | yes |
| `breath_depth` | f32 | 0.0 to 0.6 | `0.18` | yes |
| `glow_intensity` | f32 | 0.0 to 2.0 | `0.6` | yes |
| `cycle_degrees_per_second` | f32 | 1.0 to 360.0 | `40.0` | yes |
| `transition_ms` | u32 | 0 to 2000 | `300` | yes |

#### `[rgb.reactive]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `gain` | f32 | 0.0 to 4.0 | `1.0` | yes |
| `floor` | f32 | 0.0 to 1.0 | `0.12` | yes |
| `bass_hz` | u32 | 20 to 500 | `250` | yes |
| `mid_hz` | u32 | 200 to 8000 | `2000` | yes |
| `hue_spread_degrees` | f32 | 0.0 to 360.0 | `90.0` | yes |
| `glow_from_bass` | f32 | 0.0 to 2.0 | `0.55` | yes |

#### `[audio]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `enabled` | bool | | `true` | yes |
| `source` | enum | `system`, `microphone`, `none` | `system` | restart |
| `device` | string | | `default` | restart |
| `backend` | enum | `auto`, `pulse`, `pipewire`, `wasapi` | `auto` | restart |
| `fft_size` | enum | `1024`, `2048`, `4096`, `8192` | `4096` | yes |
| `band_count` | u32 | 8 to 128 | `32` | yes |
| `sensitivity` | f32 | 0.1 to 8.0 | `1.8` | yes |
| `attack` | f32 | 0.01 to 1.0 | `0.35` | yes |
| `release` | f32 | 0.005 to 1.0 | `0.08` | yes |
| `silence_threshold_db` | f32 | -90.0 to -20.0 | `-60.0` | yes |
| `bass_tilt_db_per_octave` | f32 | 0.0 to 6.0 | `3.0` | yes |
| `normalize` | bool | | `true` | yes |

#### `[audio.visualizer]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `style` | enum | `bars`, `mirror`, `waveform`, `dots` | `bars` | yes |
| `bar_spacing` | u32 | 0 to 12 | `2` | yes |
| `bar_radius` | u32 | 0 to 8 | `3` | yes |
| `peak_hold_ms` | u32 | 0 to 3000 | `400` | yes |
| `hue_spread_degrees` | f32 | 0.0 to 360.0 | `120.0` | yes |
| `hot_tip` | bool | | `true` | yes |
| `baseline` | bool | | `true` | yes |

#### `[media]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `enabled` | bool | | `true` | yes |
| `hold_ms` | u32 | 0 to 60000 | `6000` | yes |
| `show_album_art` | bool | | `true` | yes |
| `album_art_size` | u32 | 24 to 128 | `52` | yes |
| `album_art_corner_radius` | u32 | 0 to 32 | `8` | yes |
| `show_progress` | bool | | `true` | yes |
| `progress_height` | u32 | 1 to 10 | `3` | yes |
| `scrub_on_click` | bool | | `true` | yes |
| `show_volume` | bool | | `false` | yes |
| `marquee` | bool | | `true` | yes |
| `marquee_speed_px_per_s` | f32 | 5.0 to 400.0 | `40.0` | yes |
| `marquee_gap_px` | u32 | 8 to 200 | `48` | yes |
| `marquee_pause_on_hover_ms` | u32 | 0 to 10000 | `1800` | yes |
| `preferred_players` | array | | `[]` | yes |
| `cycle_players_hotkey` | string | | `""` | yes |

#### `[clock]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `enabled` | bool | | `true` | yes |
| `format` | enum | `12h`, `24h` | `24h` | yes |
| `show_seconds` | bool | | `false` | yes |
| `show_date` | bool | | `true` | yes |
| `date_format` | string | strftime | `%d %b` | yes |
| `show_weekday` | bool | | `false` | yes |
| `second_timezone` | string | IANA name | `""` | yes |
| `hour_leading_zero` | bool | | `true` | yes |

#### `[metrics]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `enabled` | bool | | `false` | yes |
| `interval_ms` | u32 | 200 to 10000 | `1000` | yes |
| `show_cpu` | bool | | `true` | yes |
| `show_ram` | bool | | `true` | yes |
| `show_gpu` | bool | | `false` | yes |
| `show_network` | bool | | `false` | yes |
| `sparkline_bars` | u32 | 8 to 64 | `24` | yes |
| `alert_on_threshold` | f32 | 0.5 to 1.0 | `0.0` (off) | yes |

#### `[battery]`, `[network]`, `[pomodoro]`, `[weather]`

Each follows the same pattern: `enabled`, an `interval_ms`, and widget-specific display toggles. Weather additionally has `provider` (`open-meteo`, `openweathermap`), `api_key`, `units` (`metric`, `imperial`), `latitude`, `longitude`, and `cache_ttl_minutes`.

Pomodoro has `focus_minutes` (default 25), `short_break_minutes` (5), `long_break_minutes` (15), `rounds_before_long` (4), `notify_phases` (true).

#### `[layout]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `padding_x` | u32 | 0 to 48 | `12` | yes |
| `padding_y` | u32 | 0 to 24 | `6` | yes |
| `spacing` | u32 | 0 to 32 | `10` | yes |
| `align` | enum | `top`, `center`, `bottom` | `center` | yes |
| `order` | array | widget ids | `["clock", "media", "spectrum", "metrics", "battery", "network", "pomodoro", "shortcuts"]` | yes |
| `max_widgets` | u32 | 1 to 6 | `3` | yes |

#### `[hotkeys]`

| Key | Type | Default | Action |
|---|---|---|---|
| `toggle_visibility` | string | `ctrl+alt+l` | Show or hide the island |
| `cycle_widget` | string | `ctrl+alt+tab` | Advance to the next widget state |
| `cycle_back` | string | `ctrl+shift+tab` | Previous widget state |
| `rgb_next_mode` | string | `ctrl+alt+r` | Advance RGB engine |
| `profile_next` | string | `ctrl+alt+p` | Next profile |
| `media_play_pause` | string | `""` | Toggle playback |
| `media_next` | string | `""` | Next track |
| `media_previous` | string | `""` | Previous track |
| `media_stop` | string | `""` | Stop playback |
| `cycle_players` | string | `""` | Switch active player |
| `collapse_now` | string | `""` | Force collapse |
| `reload_config` | string | `""` | Force a config reload |
| `toggle_debug_overlay` | string | `ctrl+alt+d` | Frame time and state readout |

#### `[shortcuts.list]` (array of tables)

| Key | Type | Notes |
|---|---|---|
| `id` | string | Unique, required |
| `label` | string | Tooltip and accessibility name, required |
| `icon` | string | Bundled icon name or path to a user SVG |
| `command` | string | Executed via the platform shell, required unless `ipc_only` |
| `args` | array | Optional argument vector, avoids quoting bugs |
| `cwd` | string | Optional working directory |
| `hotkey` | string | Optional global hotkey for this specific action |
| `show_in_island` | bool | Defaults to true |
| `ipc_only` | bool | Trigger only via IPC, not by clicking |

#### `[tray]`

`enabled` (true), `show_on_startup` (false), `icon` (auto), and a `menu` array of `{ id, label, action, checked }`.

#### `[motion]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `reduced` | bool | | `false` | yes |
| `master_scale` | f32 | 0.25 to 4.0 | `1.0` | yes |
| `morph_response_s` | f32 | 0.08 to 1.2 | `0.34` | yes |
| `morph_damping` | f32 | 0.4 to 2.0 | `0.78` | yes |
| `content_fade_ms` | u32 | 0 to 600 | `160` | yes |
| `startup_reveal` | bool | | `true` | yes |

#### `[text]`

`font_size` (14), `font_weight` (`regular` or `semibold`), `title_size` (15), `artist_size` (13), `font_file` (path, empty for bundled), `color` (`#FFFFFFFF`), `dim_color` (`#B3FFFFFF`), `marquee`, `sharp`, `letter_spacing` (0.0).

#### `[advanced]`

| Key | Type | Range | Default | Hot reload |
|---|---|---|---|---|
| `target_fps` | u32 | 15 to 240 | `60` | yes |
| `vsync` | bool | | `true` | yes |
| `gpu_backend` | enum | `auto`, `vulkan`, `dx12`, `metal`, `gl` | `auto` | restart |
| `log_level` | enum | `error`, `warn`, `info`, `debug`, `trace` | `info` | yes |
| `log_file` | string | | platform default | yes |
| `ipc_enabled` | bool | | `true` | restart |
| `ipc_name` | string | | `lumina.sock` or `\\.\pipe\lumina` | restart |
| `power_saving` | bool | | `true` | yes |
| `experimental_wgsl` | string | path to user shader override | `""` | restart |

### 18.3 Validation rules

Validation runs per key, not per file, so one bad value never discards the rest of the configuration:

1. Parse into a `toml::Value` tree first, so a type error does not abort deserialization.
2. For each key present in the tree, look it up in the schema registry. Unknown keys produce a warn log listing the key and the nearest valid key (Levenshtein distance under 3).
3. Type mismatch: use the default, log a warn with expected and found types.
4. Out of range: clamp to the nearest bound, log a warn.
5. Invalid enum: use the default, log a warn listing valid options.
6. Malformed color: use the default, log a warn.
7. Cross-field rules: `half_bottom > half_top` clamps down, `audio.source = microphone` with `backend = wasapi` is fine, `[rgb.reactive]` without `mode = reactive` is ignored silently, `hotkeys` with a duplicate combo keeps the first and warns.
8. After validation, the merged config is diffed against the previous one. Only changed keys dispatch change events, so a no-op reload touches nothing.

### 18.4 Hot reload mechanics

```
notify watches config.toml, theme files, icon directories
  -> events coalesce into a channel
  -> 150 ms debounce timer (editors write files in bursts)
  -> re-read from disk, re-validate, merge, diff
  -> dispatch typed ConfigChange enum to affected subsystems
  -> orchestrator retargets springs to new geometry if shape changed
```

Reload latency target is under 50 ms. Font file changes require a restart, and this is stated explicitly in the key's documentation rather than silently ignored.

### 18.5 Profiles

```toml
[profile]
default = "balanced"

[profiles.balanced]
# empty means "inherit everything"

[profiles.gaming]
inherit = "balanced"
rgb.mode = "reactive"
audio.sensitivity = 3.0
audio.visualizer.style = "mirror"
rgb.glow_intensity = 1.2
clock.show_date = false

[profiles.minimal]
inherit = "balanced"
spectrum.enabled = false
metrics.enabled = false
rgb.mode = "static"
```

Inheritance is one level deep in V0.1.0 to keep resolution predictable. Cycles are detected at load time and rejected with a clear error. Switching profiles crossfades colors and geometry over `rgb.transition_ms`.

### 18.6 Themes

A theme is a separate file containing only the keys it overrides, plus an optional background image:

```toml
# ~/.config/lumina/themes/neon.toml
name = "Neon"
extends = "default"

[appearance]
fill = "#0B0B16F0"
border_color = "#00E5FF80"
glow_spread = 16
glow_falloff = 6.0

[rgb]
mode = "cycling"
primary = "#FF2D95"
secondary = "#00E5FF"
glow_intensity = 1.0

[appearance.background]
path = "~/.config/lumina/themes/neon-grid.png"
blur_strength = 0.6
opacity = 0.25
mode = "grid"
```

Bundled themes: `default`, `midnight`, `neon`, `sunset`, `mono`, `glass`, `nord`, `catppuccin`. Community themes are plain files and need no special support.

---

## 19. Customization and Extensibility

Customization is a first-class feature, not an afterthought. Four layers, each more powerful than the last.

### 19.1 Layer 1, configuration

Every documented key. Covers position, geometry, colors, timing, layout, widgets, hotkeys, and behavior. Covers the large majority of what users want.

### 19.2 Layer 2, themes and profiles

Reusable bundles of appearance and behavior, inheritable, swappable at runtime. Covers aesthetic variety and per-context setups such as gaming, presenting, or screen sharing.

### 19.3 Layer 3, per-widget overrides

Any `[widget]` table accepts its own `[widget.rgb]`, geometry hints, and style flags, so the media widget can be red while the clock stays neutral. See 13.5 for precedence.

### 19.4 Layer 4, user WGSL

`[advanced].experimental_wgsl` points at a WGSL file that is included into `island.wgsl` at load time, replacing the `effect()` function:

```wgsl
// user shader contract
fn user_effect(
    uv: vec2<f32>,
    d: f32,             // signed distance to the island
    time: f32,
    energy: vec3<f32>,  // bass, mid, treble
    palette: vec4<f32>  // fill rgb + intensity
) -> vec4<f32> {
    let scan = sin((uv.y * 220.0) + time * 6.0) * 0.5 + 0.5;
    let stripes = smoothstep(0.35, 0.65, scan) * 0.25;
    let rgb = vec3<f32>(
        0.5 + 0.5 * sin(time * 2.0),
        0.5 + 0.5 * sin(time * 2.0 + 2.1),
        0.5 + 0.5 * sin(time * 2.0 + 4.2)
    );
    return vec4<f32>(rgb * stripes, 1.0);
}
```

Shader compilation is sandboxed by validation only, not by a security boundary. It runs in the GPU driver, and a malformed shader is caught by `naga` before it reaches the driver, with the error shown in the log and the built-in shader retained.

### 19.5 Layer 5, script widget

Users who need real programmability get it without a plugin ABI:

```toml
[[widgets.script.entries]]
id = "uptime"
label = "Uptime"
command = "uptime -p"
interval_ms = 60000
display = "text"
icon = "clock"

[[widgets.script.entries]]
id = "spotify-status"
label = "Now playing"
command = "playerctl status --follow"
interval_ms = 0          # 0 means follow the stream, tail the output
display = "text"
timeout_ms = 5000
```

Commands run through the platform shell with a 5 second default timeout, output is truncated to 120 bytes, and a non-zero exit code shows the exit status in the widget rather than failing silently. This is enough to integrate with any CLI tool the user already has.

---

## 20. Hotkeys, Shortcuts, Tray

### 20.1 Hotkey parsing and normalization

A hotkey string parses to a normalized combo:

```
"ctrl+shift+space"  ->  { modifiers: CTRL | SHIFT, key: Space }
"super+w"           ->  { modifiers: SUPER,              key: KeyW }
"alt+F5"            ->  { modifiers: ALT,                key: F5 }
```

Accepted modifiers: `ctrl`, `alt`, `shift`, `super`, `meta`. Accepted keys: `a` to `z`, `0` to `9`, `f1` to `f24`, `space`, `enter`, `escape`, `tab`, `backspace`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `insert`, `delete`, `,`, `.`, `/`, `;`, `'`, `[`, `]`, `` ` ``, `-`, `=`, `` ` ``.

Parsing is case-insensitive, order-independent, and duplicate modifiers are collapsed. Anything else is a validation error naming the offending token.

### 20.2 Platform hotkey behavior

| Platform | Mechanism | Conflict behavior |
|---|---|---|
| Windows | `RegisterHotKey`, one id per combo | Failure logged with `GetLastError`, combo skipped, app continues |
| X11 | `XGrabKey` on the root window with modifier variants (NumLock, CapsLock, ScrollLock) | `BadAccess` means another client owns it, logged and skipped |
| Wayland | `org.freedesktop.portal.GlobalShortcuts` | Portal unavailable means global hotkeys are disabled, with a clear log line and a note in the tray tooltip |

On Wayland without the portal, keyboard shortcuts still work when the island has keyboard focus, which is why the layer-shell surface requests `on-demand` keyboard interactivity rather than `none`.

### 20.3 Tray

`tray-icon` with a generated icon that reflects state: idle, active, muted, hidden. Menu is built from config plus fixed entries:

| Entry | Action |
|---|---|
| Show / Hide | Toggles visibility, updates the check mark |
| Widgets submenu | One checkable item per widget, jumps the island to that state |
| RGB submenu | One checkable item per engine |
| Profiles submenu | One checkable item per configured profile |
| Next widget | Cycles the island state |
| Auto-start | Checkable, writes or removes the autostart entry |
| Settings | Opens the config file in the default editor |
| Documentation | Opens the local docs folder |
| About | Version, platform, GPU, build profile |
| Quit | Clean shutdown |

Left click toggles visibility, right click opens the menu. On GNOME the tray needs the `StatusNotifierItem` or `AppIndicator` extension, and the README documents that.

### 20.4 Autostart

| Platform | Mechanism | Removal |
|---|---|---|
| Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value `Lumina` | Tray toggle or uninstaller |
| Linux X11 and Wayland | `~/.config/autostart/lumina.desktop` with `X-GNOME-Autostart-enabled=true` | Tray toggle or uninstaller |

The `.desktop` file uses `Exec=lumina --hidden` so autostart does not fight with the user's own visibility preference. Both entries are written with `--hidden` semantics if `start_hidden` is true.

---

## 21. IPC Protocol

### 21.1 Transport

| Platform | Address | Notes |
|---|---|---|
| Linux | Unix domain socket at `$XDG_RUNTIME_DIR/lumina.sock`, falling back to `/tmp/lumina-$UID.sock` | Mode `0600`, owned by the user |
| Windows | Named pipe `\\.\pipe\lumina-<user-sid>` | Default ACL grants the creating user only |

### 21.2 Framing

Newline delimited UTF-8 JSON, one object per line. Requests carry a `id` (string or number) and receive exactly one response with the same `id`. Responses are `{"id":..., "ok":true, "data":...}` or `{"id":..., "ok":false, "error":{"code":..., "message":...}}`. The server may push unsolicited `{"event":..., "data":...}` lines. Maximum line length is 64 KiB, longer lines are rejected and the connection is closed with code `LineTooLong`.

### 21.3 Command reference

| Command | Payload | Result | Notes |
|---|---|---|---|
| `status` | none | `StateSnapshot` | Full state dump, the main debugging tool |
| `show` | `widget`, `duration_ms`, `pin` | `ok` | Brings island to a state |
| `hide` | none | `ok` | Same as `toggle_visibility` off |
| `collapse` | none | `ok` | Return to Collapsed immediately |
| `notify` | `title`, `body`, `icon`, `accent`, `duration_ms`, `action` | `notification_id` | Fires the notify widget |
| `set_rgb` | `mode`, `color`, `speed` | `ok` | Overrides for the session |
| `cycle_rgb` | none | `mode` | Next engine |
| `set_profile` | `name` | `ok` | Unknown name returns `UnknownProfile` |
| `get_config` | `section` optional | `object` | Reads the effective config |
| `set_config` | `path`, `value`, `persist` | `ok` | `persist:false` keeps it session-only |
| `reload_config` | none | `ok` | Same as the hotkey |
| `media` | `action`, `player` optional | `ok` | `play`, `pause`, `play_pause`, `next`, `previous`, `stop`, `seek`, `volume` |
| `player_list` | none | `array` | Identities and statuses |
| `shortcut` | `id` | `ok` | Runs a configured shortcut |
| `execute` | `command`, `args`, `timeout_ms` | `exit_code`, `stdout` | Opt-in, disabled unless `[advanced].ipc_allow_exec` |
| `spectrum` | `peak` | `array` | Current band magnitudes, for external visualizers |
| `geometry` | `top_half_width`, `bottom_half_width`, `height`, `corner_radius` | `ok` | Live shape editing |
| `debug_overlay` | `enabled` | `ok` | Frame time and state readout |
| `quit` | none | `ok` then close | Graceful shutdown |

### 21.4 Error codes

| Code | Meaning |
|---|---|
| `BadRequest` | Not valid JSON or missing a required field |
| `UnknownCommand` | Command string not recognized |
| `InvalidArgument` | Field present but out of range or wrong type |
| `UnknownProfile` | Profile name does not exist |
| `PlayerGone` | Targeted media session disappeared |
| `PlayerNotFound` | No session matches the requested player |
| `Disabled` | Command exists but is gated behind a config flag |
| `NotSupported` | Command unavailable on this platform |
| `RateLimited` | More than 120 commands per second from one client |
| `Internal` | Bug, details written to the log with a trace id |

### 21.5 Event push

| Event | Fired when |
|---|---|
| `media_changed` | Active player, status, or metadata changes |
| `state_changed` | Island enters a new state |
| `rgb_changed` | RGB engine or palette changes |
| `config_reloaded` | A reload completes successfully |
| `config_error` | A reload fails validation, carries the offending key |

### 21.6 Clients

- `lumina-send` CLI: `lumina-send notify --title "Build done" --body "lumina 0.1.0-alpha"`, with bash, zsh, and fish completions.
- `lumina-py`: pure standard library, synchronous and async, `LuminaClient` and `async def` variants, ships type hints.
- `lumina-node`: EventEmitter-based, TypeScript types included.
- Example shell integrations are shipped as snippets in `docs/`: a `make` wrapper, a GitHub Actions reporter, an `mpv` OSD hook.

### 21.7 Multi-client behavior

Up to 8 concurrent connections. Each client gets its own `id` space. Commands are serialized through the orchestrator, so ordering is global and deterministic. A client that sends an invalid command does not lose its connection; only framing violations close it.

---

## 22. Platform Backends

### 22.1 Windows

**Window creation**

```rust
let style = WS_POPUP | WS_VISIBLE;
let ex_style = WS_EX_TOOLWINDOW      // hide from taskbar and alt-tab
                    | WS_EX_NOREDIRECTIONBITTER  // required for per-pixel alpha with D3D
                    | WS_EX_LAYERED;            // per-pixel alpha fallback path

CreateWindowExW(ex_style, class, L"Lumina", style,
                x, y, width, height, HWND_TOPMOST, ptr, hinstance, ptr);
```

**Always on top** is enforced by the `HWND_TOPMOST` argument plus a `SetWindowPos` re-assert after every monitor change and after every `WM_WINDOWPOSCHANGED`, because full-screen applications can otherwise steal the top position.

**Per-pixel alpha** comes from `DwmExtendFrameIntoClientArea` with a margins struct of -1 on all sides, which promotes the window to per-pixel alpha. On builds where that fails, `SetLayeredWindowAttributes` with `LWA_COLORKEY` on a sentinel magenta is used as the fallback.

**Backdrop blur** uses the undocumented but universally present `SetWindowCompositionAttribute` with `ACCENT_ENABLE_ACRYLICBLURBEHIND` (id 4) and a 45 percent tint color. On Windows 11, `DwmSetWindowAttribute` with `DWMWA_SYSTEMBACKDROP_TYPE = 3` (transient window) gives a cleaner result. Both paths are probed at startup and the working one is recorded.

**Rounded corners** are set through `DWMWA_WINDOW_CORNER_PREFERENCE = 33`. Lumina does not rely on this because it draws its own shape, but leaving it on prevents Windows 11 from drawing a default border.

**Click-through** toggles `WS_EX_TRANSPARENT` on the extended style with `SetWindowLongPtrW(GWL_EXSTYLE, ...)`. Toggling requires no window recreation. When click-through is on, `WM_NCHITTEST` also returns `HTTRANSPARENT` as a belt-and-braces measure, because some apps do their own hit testing.

**DPI awareness** is set with `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)` before any window exists. The V2 context gives us correct per-monitor scaling in mixed-DPI setups. Coordinates from `WM_DPICHANGED` update the geometry scale factor.

**Monitors** come from `EnumDisplayMonitors` plus `GetDpiForMonitor`. The primary monitor is the one whose origin is nearest (0,0) with the largest area, since `MONITORINFOF_PRIMARY` is only set for one and users expect "primary" to mean the one they look at.

**Global hotkeys** use `RegisterHotKey` with ids allocated from a base of 0xB000 plus index, so a conflict is identified by `GetLastError` returning `ERROR_HOTKEY_ALREADY_REGISTERED`.

**Shutdown** intercepts `WM_QUERYENDSESSION` and `WM_ENDSESSION` to clean up hotkeys before Windows logs the user off.

### 22.2 X11

**Connection** is `x11rb::connect(None)`, with a fallback to `x11rb::connect_with_xauth()` reading `~/.Xauthority`, because display servers started by GDM, SDDM, and systemd user sessions do not always pass `XAUTHORITY` in the environment.

**Visual selection** is the important part. We need a 32-bit visual with an alpha mask:

```rust
for depth in [32u8, 24u8] {
    let attrs = screen.root_depth().visual_class_list(...)
    // select TrueColor or DirectColor visual with depth 32
    // create a colormap from it, do not use the default colormap
}
```

A `Colormap` created from the ARGB visual is mandatory. Using the default colormap with a 32-bit visual produces black or garbage on most servers, which is the single most common bug in X11 transparency code.

**Window type and state** are set through EWMH atoms:

```
_NET_WM_WINDOW_TYPE           = _NET_WM_WINDOW_TYPE_DOCK
_NET_WM_STATE_ABOVE           = set, for topmost
_NET_WM_STATE_STICKY          = set, so it stays on all workspaces
_NET_WM_STATE_SKIP_TASKBAR    = set
_NET_WM_STATE_SKIP_PAGER      = set
_NET_WM_WINDOW_NAME           = "Lumina"
_NET_WM_WINDOW_CLASS          = "lumina"
```

`_NET_WM_WINDOW_TYPE_DOCK` rather than `_NET_WM_WINDOW_TYPE_NORMAL` makes most EWMH-compliant window managers treat the window as a panel: always on top, no decorations, never minimized.

**Positioning** uses `xcb_configure_window` with the computed coordinates, since dock windows are positioned by the client rather than the window manager.

**Override redirect** stays off. With redirect on, the window floats above everything including full-screen windows, which is wrong on multi-monitor setups where the island should be confined to its monitor.

**Click-through** is implemented with `XShapeCombineRectangles` passing an empty input region, or a one pixel region at a known-empty location as the classic trick. We use the empty region approach and restore the real region on hover.

**Focus** is never accepted. `XSetInputFocus` is never called, and `WM_HINTS.input = False` is set so the island cannot steal keyboard focus from the user's application.

**Monitors** come from `XRandR` (`GetMonitors`, `GetOutputInfo`) for per-monitor geometry and DPI, falling back to `GetGeometry` on the root window when RandR is a stub on very old X servers.

**Hotkeys** use `XGrabKey` on the root window. For each combo we must grab every modifier variant that could apply, so `ctrl+alt+l` requires 8 grabs (CapsLock, NumLock, and ScrollLock each on or off). `XSync` after grabbing surfaces failures, and `BadAccess` on any variant means the combo is owned by someone else.

### 22.3 Wayland

**Detection** checks `WAYLAND_DISPLAY` and `XDG_SESSION_TYPE`. If `WAYLAND_DISPLAY` is set we attempt layer-shell, and fall back to XWayland if the global is absent.

**Layer-shell** is the correct mechanism because it asks the compositor for a dedicated surface on a reserved band, which is precisely what an always-on-top overlay is:

```
zwlr_layer_shell_v1.get_layer_surface(surface, output, layer, namespace)
  namespace = "lumina"
  layer     = zwlr_layer_shell_v1_layer_top   (or _overlay for higher priority)

zwlr_layer_surface_v1.set_size(0, height)      // 0 width means compositor decides
zwlr_layer_surface_v1.set_anchor(anchor_top)
zwlr_layer_surface_v1.set_exclusive_zone(height)
zwlr_layer_surface_v1.set_margin(top, right, bottom, left)
zwlr_layer_surface_v1.set_keyboard_interactivity(on_demand)
```

`exclusive_zone` reserves the band so maximized windows do not slide under the island. Setting it to `-1` makes the island ignore other exclusive zones entirely, which is what most users want.

**Configure handling**: the compositor sends `configure` with a suggested size. We must ack it with `zwlr_layer_surface_v1.ack_configure`. A common bug is acknowledging before acting on the new size, which leaves the island one configure behind and makes it look blurry or misplaced on first open. We apply the size, then acknowledge, in that order.

**Positioning**: `set_anchor(top | top_left | top_right | bottom | ...)` implements `[general].position`, and `set_margin` implements `offset_x` and `offset_y`. Horizontal centering uses `anchor_top` alone, which centers the surface on the output.

**Mouse transparency outside the island**: layer-shell surfaces receive pointer events across their whole surface. Because our surface is only as large as the island, this is already correct. We additionally use `wl_surface.set_input_region` with an empty region when click-through is active.

**Global hotkeys** require the `org.freedesktop.portal.GlobalShortcuts` portal, mediated through `ashpd` or `zbus`. Without it, hotkeys work only when the island has focus. This is a platform limitation and is documented, not worked around.

**Idle inhibit** is requested via `org.freedesktop.portal.Inhibit` so a screensaver does not activate while the island is pinned, then released on collapse.

### 22.4 Platform capability matrix

| Capability | Windows | X11 | Wayland with layer-shell | Wayland without |
|---|---|---|---|---|
| Always on top | native | native | native | best effort |
| Per-pixel alpha | native | native | native | yes |
| Transparent window | native | native | native | yes |
| Position control | full | full | anchors only | none |
| Reserves space | no | dock hint | exclusive zone | no |
| Native backdrop blur | yes | no | no | no |
| Global hotkeys | yes | yes | portal only | no |
| Click-through | yes | yes | input region | yes |
| Never steals focus | yes | yes | `on_demand` | partial |
| Multi-monitor position | full | full | per output | full under XWayland |

Every capability in this table maps to a config key, so a user on GNOME Wayland who needs X11 behavior can run Lumina under XWayland deliberately and get a better experience than native Wayland provides.

---

## 23. Performance Engineering

### 23.1 Rules

1. No allocation after startup in the frame loop. Enforced with a custom global allocator that counts allocations, and a debug assertion that the counter does not move between two consecutive `present` calls in `debug` builds.
2. No locks on the render thread. The `StateSnapshot` triple buffer is lock free.
3. Sleep when idle. See 11.5.
4. Batch everything. The content pass is one instanced draw for all quads.
5. Uniforms, not vertex buffers, for anything that changes per frame.

### 23.2 Startup optimizations

| Optimization | Implementation | Saving |
|---|---|---|
| Embed atlases | `include_bytes!` of MSDF and SDF atlases | 300 to 800 ms |
| Eager pipeline compilation | `create_render_pipeline` for all 4 passes at startup, then `device.poll(Maintain::Wait)` | avoids a 200 ms hitch on first expand |
| Parallel service spawn | Threads start while the device initializes | 90 ms |
| Font metric cache | `fontdue` metrics cached in a `HashMap<(FontId, u32), Metrics>` behind no lock, owned by the render thread | 4 to 8 ms per frame in heavy text states |
| Icon decode at build time | SVGs rasterized in `build.rs` | 40 ms |

### 23.3 Runtime optimizations

| Technique | Detail |
|---|---|
| Early fragment out | `if (d > glow_spread) { discard; }` skips all expensive math outside the island |
| Bar instancing | 32 bars are one draw call with `instance_index` selecting the band |
| Uniform write batching | One `write_buffer` per frame, 176 bytes |
| Zero-copy art upload | `write_texture` from a reused `Vec<u8>` staging buffer, never reallocated |
| Text layout cache | Measured string to glyph quad list, invalidated only when the string or font size changes |
| Metrics decoupled from render | Sampled at 1 Hz regardless of frame rate, so a 240 Hz display does not quadruple the work |
| Power saving | `power_saving = true` requests `LowPower` adapter profile, caps at 60 fps, and disables the spectrum when the screen is locked |
| Blank frame skip | If nothing changed and the compositor already has the frame, skip submission entirely |

### 23.4 Measurement plan

| Phase | Tool | Metric |
|---|---|---|
| Startup | `Instant` instrumentation, logged at debug | Time to first paint |
| Per frame | wgpu timestamp queries, conditional on adapter support | GPU and CPU frame time |
| Memory | `tracemalloc`-equivalent via the counting allocator, plus RSS | Resident and peak |
| CPU | `perf record` on Linux, WPR on Windows | Hot symbols |
| Regression | Baseline JSON committed, compared in CI on Linux | Frame time and RSS deltas fail the build above 5 percent |

### 23.5 Known costs that are out of our control

| Cost | Size | Note |
|---|---|---|
| Graphics driver resident memory | 15 to 25 MB | Present on every GPU application |
| Vulkan or D3D12 loader | 1 to 3 MB | |
| Compositor side surface buffer | compositor dependent | Wayland layer surfaces are often double buffered by the compositor |

---

## 24. Reliability and Diagnostics

### 24.1 Panic policy

Each worker thread wraps its body in `catch_unwind`. A panic in the audio thread stops audio and logs an error with the thread name and backtrace, and the spectrum widget reports unavailable. A panic in the media thread stops media polling. A panic on the main thread logs and triggers a controlled restart of the render loop rather than exiting. Configuration and state are never lost to a panic, because they are only ever mutated on the orchestrator thread, which is the one thread that never unwinds.

### 24.2 Logging

| Setting | Value |
|---|---|
| Format | Structured, `timestamp level target message key=value` |
| Levels | error, warn, info, debug, trace |
| Default level | info |
| File | `%LOCALAPPDATA%\Lumina\logs\lumina.log` or `~/.local/state/lumina/lumina.log` |
| Rotation | 5 MB per file, 3 files kept |
| Stdout | Only when `--verbose` is passed or the binary is run from a terminal |

Logs never include file paths from user media, never include anything from album art, and never include the IPC content of notify commands. The privacy note in the README says so explicitly.

### 24.3 Diagnostics commands

| Command | Purpose |
|---|---|
| `lumina --version` | Version, commit hash, build date, Rustc version, target triple |
| `lumina --diagnostics` | Full environment dump: display server, GPU adapter, Vulkan or D3D version, driver version, available backends, monitor list with DPI, audio backend, detected media players, config path, log path |
| `lumina --check-config` | Parses and validates, prints every warning and error, exits 0 if clean, 1 otherwise. Meant for CI and bug reports |
| `lumina --benchmark` | Runs 600 frames of each island state, reports p50, p95, p99 frame time and peak RSS |
| `lumina --self-test` | Exercises window creation, shader compilation, text rendering, audio capture, and IPC round trip, then exits. Used by packaging CI to catch broken environments before a user sees them |

`--diagnostics` output is designed to be pasted verbatim into a bug report. That single design decision has saved more debugging time than any other feature in this plan.

### 24.4 Graceful degradation matrix

| Missing dependency | Result |
|---|---|
| No Vulkan, has OpenGL 3.3 | Renders through the GLES backend, glow and blur reduced |
| No usable GPU API at all | Solid fallback shape, tray notification explaining, app stays alive |
| No audio server | Spectrum reports unavailable, everything else normal |
| No D-Bus | Media widget hidden |
| No tray support | Menus reachable via IPC and hotkeys, warn logged |
| No IPC socket permission | IPC disabled, warn logged |
| Config directory unwritable | Built-in defaults, warn logged once, no repeated spam |

---

## 25. Security Model

### 25.1 Threat surface

Lumina holds no privileges, opens no listening network sockets, and writes only inside its own config, state, and log directories plus one autostart entry. The realistic attack surface is:

1. **IPC socket**: any local process could talk to it if permissions are wrong.
2. **Shortcut commands**: config-driven process execution.
3. **Weather API key**: stored in plaintext in config, sent to the provider.
4. **User WGSL**: arbitrary shader code, GPU-scoped.

### 25.2 Controls

| Risk | Control |
|---|---|
| IPC hijack | Socket mode `0600`, owned by the user. Windows named pipe uses a DACL limited to the creating user's SID |
| IPC rate abuse | 120 commands per second per connection, excess returns `RateLimited` |
| Command execution via IPC | `execute` is disabled unless `[advanced].ipc_allow_exec = true` |
| Command execution via config | Only the user's own config file, which the user also owns, so this is the user executing their own command. Documented plainly |
| API key exposure | Stored in config with mode `0600` on Linux, never logged, never sent anywhere except the configured provider |
| Shader abuse | Compiled through `naga` validation before reaching the driver. A shader cannot read files or open sockets, it can only compute pixels |
| Update channel | No auto-update. Users install updates through their package manager |
| Telemetry | None. No network calls at all except the optional weather widget |

### 25.3 Explicit non-guarantees

Lumina offers no sandboxing of shortcut commands. If you put `rm -rf /` in your config, Lumina will run it. This is stated in the docs, because the alternative is a restricted command language that would be less useful and less honest.

### 25.4 Dependency policy

`cargo-deny` runs in CI with a license allowlist of MIT, Apache-2.0, BSD-3-Clause, ISC, and Unicode-3.0. `cargo-audit` blocks known advisories. `Cargo.lock` is committed, since a desktop application must be reproducible.

---

## 26. Accessibility

### 26.1 Requirements

| Item | Implementation |
|---|---|
| Contrast | All text at least 4.5:1 against its fill at every RGB mode, verified by a unit test that samples the rendered palette. Luminance is clamped automatically if a mode would drop below the ratio, and a warn log names which mode was clamped |
| Minimum text size | 12 logical pixels, enforced by validation |
| Scaling | Geometry and text scale with the monitor DPI factor. A `text.scale` multiplier of 0.8 to 2.0 is available for users who need larger text |
| Reduced motion | Honored from config and from the OS setting. See 12.6 |
| Screen reader | The island is not an accessibility tree citizen, since overlay windows on both platforms are not. Instead, every state change and notification is mirrored to the active screen reader through the platform notification API on Windows, and through `notify-send` on Linux. This is a partial but honest solution |
| Keyboard operation | The island accepts keyboard focus when expanded, arrow keys move between shortcut items, Enter activates, Escape collapses |
| No color-only information | Playback state is shown by icon shape, not only by color. Battery state shows a bolt icon, not only a hue |
| High contrast theme | A bundled `high-contrast` theme with pure black fill, pure white text, and no glow |

### 26.2 Keyboard interaction model

When the island expands and the user presses Tab or the configured focus hotkey:

```
[media art] [prev] [play/pause] [next] [volume] [shortcut 1] ... [close]
```

Focus is drawn as a 2 px outline in the accent color with a 2 px offset. Escape or clicking outside collapses and releases focus. On Wayland this requires `on_demand` keyboard interactivity, which is why we request it in 22.3.

---

## 27. Testing Strategy

### 27.1 Test pyramid

| Layer | Count target | What it covers |
|---|---|---|
| Unit | 400+ | SDF correctness, spring integration, color conversion, band mapping, config validation, hotkey parsing, protocol codec, envelope follower |
| Integration | 80+ | Each crate's public API against real dependencies where possible |
| Graphics | 30+ | Headless wgpu on Lavapipe, screenshot compared against committed references |
| End to end | 20+ | Scripted scenario runs on both platforms |
| Performance | 8 | Benchmarks with regression gates |
| Platform matrix | manual | See 27.5 |

### 27.2 Tests that matter most

These are the tests that catch the bugs users would actually notice:

1. **SDF correctness**: for 5000 random points, assert `sd_rounded_trapezoid` agrees with a brute-force distance computation within 1e-3. Also assert the rectangular degeneracy case exactly.
2. **Spring settling**: for every named spring, simulate 3 seconds and assert `|x - target| < 1e-3` and `|v| < 1e-3`.
3. **Spring interruption**: retarget at t = 0.1 s and assert the position is continuous (no jump greater than 2 px equivalent).
4. **Color round trip**: OkLab to sRGB and back within 1e-4 for 10,000 random colors.
5. **Config validation**: for every schema key, feed the type error, the out-of-range error, and the invalid enum, and assert the documented fallback is produced.
6. **Hotkey normalization**: 500 equivalent spellings of the same combo all parse to the same normalized value.
7. **Band mapping**: a synthetic 1 kHz sine produces a peak in the band containing 1 kHz and at least 20 dB below peak in all others.
8. **Envelope attack and release**: assert a step input reaches 90 percent in at most 5 frames and decays to 10 percent in at least 20.
9. **Ring buffer**: concurrent single producer single consumer at 48 kHz for 10 simulated seconds with no torn reads, measured with a checksum.
10. **State machine**: enumerate all pairs of states and assert a legal transition exists and produces a monotone spring retarget.
11. **Protocol codec**: fuzz with 10,000 random byte sequences, assert no panic and no unbounded allocation.
12. **Screenshot regression**: render each of the 8 states at 1x and 2x on Lavapipe, compare to committed PNGs with a 0.1 percent pixel tolerance.
13. **Idle CPU**: run 60 seconds collapsed, assert fewer than 10 frames rendered.
14. **Memory ceiling**: run the full feature set for 10 minutes, assert RSS stays under the budget.

### 27.3 Fuzzing

| Target | Harness | Duration in CI |
|---|---|---|
| IPC line parser | `arbitrary` | 60 s |
| Config TOML parser | `arbitrary` | 60 s |
| WGSL user shader loader | `naga` parser, assert no panic | 30 s |
| Album art decoder | `image` crate plus a size cap | 60 s |

### 27.4 CI matrix

| Job | Platform | Runs on |
|---|---|---|
| fmt and clippy | both | every push |
| Unit and integration tests | both | every push |
| Headless graphics tests | Ubuntu with Lavapipe | every push |
| Fuzz smoke | Linux | every push |
| Full end to end | Ubuntu 22.04 and 24.04 | every merge to main |
| Full end to end | Windows 11 | every merge to main |
| Packaging smoke | both | every tag |
| Performance regression | Linux | nightly |
| MSRV check | Linux | nightly |

### 27.5 Manual test matrix for V0.1.0

| Dimension | Values |
|---|---|
| OS | Windows 10 22H2, Windows 11 23H2, Windows 11 25H2, Ubuntu 22.04, Ubuntu 24.04, Debian 12, Debian 13 |
| Session | X11, Wayland (wlroots), Wayland (KDE), Wayland (GNOME, documented fallback) |
| DPI | 100 %, 125 %, 150 %, 200 %, mixed dual monitor |
| Players | Spotify desktop, Chrome, Firefox, VLC, mpv, none playing |
| Audio | PulseAudio, PipeWire, silent, microphone only, device unplugged mid-session |
| Config | defaults, heavily customized, invalid keys, profiles, theme switch mid-session |

Every combination of OS and session is mandatory. Every other dimension is sampled. The total manual matrix is roughly 60 runs, executed across the final two phases.

---

## 28. Build, Packaging, Distribution

### 28.1 Build profiles

| Profile | Use | Settings |
|---|---|---|
| `release` | Distribution | `opt-level = "z"`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"` in the app binary, `strip = true` |
| `release-visual` | Development with visuals | `opt-level = 3`, debug assertions on, full symbols |
| `dev` | Iteration | `opt-level = 1`, fast rebuilds |

Binary size target under 8 MB stripped, excluding atlases which are separate `include_bytes!` allocations of 4 MB.

### 28.2 Conditional compilation

| Feature | Default | Enables |
|---|---|---|
| `wayland` | on | layer-shell, `smithay-client-toolkit` |
| `x11` | on | `x11rb`, XShape, XRandR, XGrabKey |
| `mpris` | on | `zbus`, MPRIS |
| `pulse` | on | `libpulse-binding` |
| `pipewire` | on | `pipewire-rs` |
| `gsmtc` | on | WinRT media sessions |
| `tray` | on | `tray-icon` |
| `weather` | on | HTTP client |
| `ipc` | on | socket server |

Linux builds ship with both X11 and Wayland enabled because XWayland users need the X11 path. Windows builds compile out every Linux crate behind `#[cfg(target_os)]`, so neither platform pays for the other's dependencies.

### 28.3 Windows packaging

| Artifact | Tool | Notes |
|---|---|---|
| `.msi` | WiX Toolset v4 | Per-machine and per-machine-free per-user installs, Start menu entry, optional autostart, uninstall registered |
| `.exe` setup | NSIS | Smaller download, portable mode checkbox, one-click install |
| Portable zip | `cargo build` plus `zip` | No installer, run from anywhere, config stays in `%APPDATA%` |

Code signing is out of scope for V0.1.0 and documented as a known SmartScreen friction point.

### 28.4 Linux packaging

| Artifact | Contents |
|---|---|
| `.deb` | Binary at `/usr/bin/lumina`, `.desktop` file, icons, postinst that offers the autostart entry, dependency on `libvulkan1`, `libxkbcommon0`, `libpulse0` as recommended not required |
| `.rpm` | Same layout, Fedora 40+ |
| AppImage | Bundles the binary plus `libvulkan1.so.1` and `libxkbcommon.so.0`. Uses the host GPU driver, never bundles Mesa |
| Alpine `.apk` | Same as deb, musl build |

Both `.deb` and `.rpm` use a shared Debian-compatible staging directory so packaging logic is written once.

### 28.5 Auto-update

None for V0.1.0. Users update through their package manager or by replacing the binary. This is the correct default: a lightweight local tool should not phone home, and a signed update channel is a project of its own.

---

## 29. Delivery Plan

Sixteen phases, roughly sixteen weeks for one full-time developer. Sub-phases are ordered by dependency, not by preference. Every phase ends with exit criteria that are checked, not assumed.

---

### Phase 0, Foundation (Week 1)

**0.1** Create the workspace with all 15 crates, each with a stub `lib.rs` and a one-line doc comment.
**0.2** Add `rust-toolchain.toml` pinning stable, and `rustfmt.toml` with the project's style.
**0.3** Write `lumina-core` types: `Geometry`, `Rect`, `Insets`, `Size`, `IslandState`, `Event`, plus their unit tests.
**0.4** Write the SDF module in `lumina-core` with the brute-force verification test from 27.2 item 1.
**0.5** Write `Spring` and `SpringVec` with the settling and interruption tests.
**0.6** Write the OkLab color conversion with the round-trip test.
**0.7** Add `cargo-deny`, `cargo-audit`, `cargo-fmt`, `cargo-clippy` to CI.
**0.8** Add the crate dependency rule test from section 7.
**0.9** Write `defaults.rs` as the single source of truth for every default value.

Deliverables: an empty workspace that compiles, passes clippy with zero warnings, and has 150 unit tests covering the mathematical core.

Exit criteria: `cargo test` green on both platforms, `cargo deny check` clean, binary links and exits 0 with `--version`.

---

### Phase 1, Window and Platform Backends (Week 2)

**1.1** winit event loop skeleton with a transparent, borderless window.
**1.2** Windows backend: window creation, topmost, tool window, per-pixel alpha, click-through, DPI awareness.
**1.3** X11 backend: connection with xauth fallback, ARGB visual plus dedicated colormap, EWMH atoms, positioning.
**1.4** Input region manipulation for click-through on both.
**1.5** Monitor enumeration and the selection policy from 22.1.
**1.6** Position calculation for all six `position` values, including offset clamping to monitor bounds.
**1.7** Re-assert topmost on `WM_WINDOWPOSCHANGED` and on X11 `ConfigureNotify`.
**1.8** A `debug_rect` mode that paints a solid rectangle so we can verify window placement without the renderer.

Deliverables: a binary that shows an opaque rectangle in the right place on both platforms, on all monitors, at all DPI scales.

Exit criteria: manual check on Windows 10, Windows 11, Ubuntu 22.04 X11, Ubuntu 24.04 Wayland. The rectangle is on top, invisible to alt-tab, absent from the taskbar and dock, does not steal focus, and does not move when other windows open.

---

### Phase 2, Rendering Engine (Week 3)

**2.1** wgpu device and surface init with the configuration from 11.3.
**2.2** Adapter selection honoring `[advanced].gpu_backend`, with a clear log line naming the chosen adapter.
**2.3** Full-target quad and the island shader with SDF fill.
**2.4** Anti-aliasing via `fwidth`, verified by the screenshot tests.
**2.5** Border pass.
**2.6** Glow pass.
**2.7** Geometry uniform updates and live resizing.
**2.8** The dirty-flag render loop from 11.5, with a frame counter.
**2.9** Error scopes and the fallback shader path from 11.4.
**2.10** Screenshot capture for tests.

Deliverables: a gradient-outlined, glowing inverse trapezoid that morphs on demand.

Exit criteria: screenshot tests pass on Lavapipe, manual visual check on Windows and Linux, frame time under 1 ms CPU for the body pass at 1440p.

---

### Phase 3, Animation (Week 3, continued)

**3.1** Master morph spring wired to the state machine.
**3.2** Content fade spring.
**3.3** Sub-step integration at 240 Hz with clamping.
**3.4** State machine implementation with the priority arbiter.
**3.5** All 8 states with their geometry and content slots.
**3.6** Hover, enter, leave, and collapse behaviors.
**3.7** Reduced motion handling.
**3.8** The choreography sequence system from 12.5, with `expand` and `pulse` implemented.

Deliverables: a fully animated island that responds to state changes without a single visual glitch.

Exit criteria: morph timing within 10 percent of the spec on every state pair, no inversion of the trapezoid at any point, interruption test passes, 60 seconds of rapid random state cycling shows no memory growth.

---

### Phase 4, Typography and Icons (Week 4)

**4.1** `tools/lumina-msdf`: MSDF generation from TTF, packing, metadata emission.
**4.2** Icon SVG to SDF atlas generation, 64 bundled icons.
**4.3** Build script integration with `include_bytes!`, plus the 90 percent fill assertion.
**4.4** MSDF text rendering pipeline.
**4.5** Glyph quad emission with kerning and truncation.
**4.6** Icon quad emission with tinting.
**4.7** Runtime fallback MSDF for user-supplied fonts.
**4.8** Marquee implementation with hover pause.

Deliverables: crisp text and icons at every scale, with a 3 MB font and 1 MB icon budget.

Exit criteria: text is visually correct at 8 px to 64 px and at fractional scales, no glyph gaps or seams, atlas memory matches the budget, total startup memory unchanged from Phase 2.

---

### Phase 5, Configuration System (Week 5)

**5.1** Full schema structs with serde.
**5.2** The defaults module wired as the single source of truth.
**5.3** Per-key validation with clamping and warnings.
**5.4** Unknown key detection with Levenshtein suggestions.
**5.5** Cross-field rules from 18.3.
**5.6** `notify` watcher with 150 ms debounce.
**5.7** Diff and typed change dispatch.
**5.8** `--check-config` command.
**5.9** Themes, 8 bundled.
**5.10** Profiles with inheritance, cycle detection, and switching.

Deliverables: a fully live-configurable application with an automated validator.

Exit criteria: every key in 18.2 has a validation test, a reload of every key applies within 50 ms, `--check-config` correctly rejects a file with 20 deliberate errors and names all 20.

---

### Phase 6, Widget Framework and Layout (Week 6)

**6.1** Widget trait, registry, lifecycle hooks.
**6.2** Flex row layout with weights, shrinking, alignment.
**6.3** Clock widget with all format options.
**6.4** Calendar widget.
**6.5** Widget order and max_widgets config.
**6.6** Draw list with no per-frame allocation, verified by the counting allocator test.
**6.7** Input routing: hit testing against widget bounds, hover state.

Deliverables: a working clock and calendar, and a layout engine that scales to the full widget set.

Exit criteria: 60 seconds of continuous frame logging shows zero allocations in the frame path, clock is accurate to within one second over a 10 minute run, layout handles a 40 character title without overflowing.

---

### Phase 7, Media Integration (Weeks 7 and 8)

**7.1** `MediaSession` trait and `MediaSnapshot`.
**7.2** MPRIS adapter: discovery, properties, signals, position tracking.
**7.3** GSMTC adapter: WinRT sessions, async bridging.
**7.4** Player ranking and the active player selection.
**7.5** Media widget: art, title, artist, transport, progress.
**7.6** Album art pipeline: extraction, cache, downscale, dominant color extraction.
**7.7** Scrub on click with hover preview.
**7.8** Volume slider.
**7.9** Multi-player cycling.
**7.10** All failure modes from 16.5.

Deliverables: full media control for every player listed in 16.4.

Exit criteria: the full player matrix from 16.4 verified on both platforms, plus the disconnect-mid-playback test, plus the player-that-does-not-implement-seek test.

---

### Phase 8, Audio Engine (Weeks 9 and 10)

**8.1** SPSC ring buffer with the torn-read test.
**8.2** WASAPI loopback capture.
**8.3** PulseAudio monitor capture.
**8.4** PipeWire capture, with automatic backend selection.
**8.5** DC removal, highpass, downmix.
**8.6** rustfft integration with the 4096 point configuration.
**8.7** Log band mapping with the 1 kHz sine test.
**8.8** Envelope follower with the attack and release test.
**8.9** Silence gate and rolling normalization.
**8.10** Spectrum widget: bars, mirror, waveform, dots.
**8.11** Peak hold markers and hot tips.
**8.12** The 32 bar instanced draw call.

Deliverables: a spectrum that looks good with music, silence, speech, and a sine wave.

Exit criteria: FFT budget from 17.7 met on a 2019 class laptop, spectrum smooth at 60 fps with no strobing, device unplug test passes, silent-input test shows a clean fade to zero.

---

### Phase 9, RGB Engine (Week 10)

**9.1** Palette resolution pipeline from the config precedence chain.
**9.2** `static` and `breathing` engines.
**9.3** `cycling` engine.
**9.4** `reactive` engine with the three-band mapping from 13.3.
**9.5** `album` engine with k-means color extraction.
**9.6** Crossfade between engines.
**9.7** Per-widget overrides.
**9.8** Band-to-hue mapping for the spectrum.
**9.9** The WCAG contrast clamp and its test.

Deliverables: five color engines, smooth transitions, and a guaranteed readable result.

Exit criteria: all engines verified by eye and by screenshot, contrast test passes for all five modes at all brightness values, switching engines mid-transition produces no visible jump.

---

### Phase 10, Widgets, Complete (Week 11)

**10.1** Metrics widget: CPU, RAM, GPU, network, with sparklines.
**10.2** Battery widget with time remaining.
**10.3** Network widget with rates.
**10.4** Pomodoro widget with the progress ring.
**10.5** Notify widget driven by IPC and internal events.
**10.6** Shortcuts widget rendering configured actions.
**10.7** Script widget with streaming output.
**10.8** Weather widget with Open-Meteo default and caching.
**10.9** Script widget security note in the docs.

Deliverables: the complete widget set from 15.3.

Exit criteria: every widget works, every widget handles its "data source missing" case, 60 seconds with all widgets enabled stays inside the memory budget.

---

### Phase 11, Hotkeys, Tray, Autostart (Week 12)

**11.1** Hotkey parser with the equivalence test.
**11.2** `HotkeyRegistry` trait and conflict detection.
**11.3** Windows `RegisterHotKey` backend.
**11.4** X11 `XGrabKey` backend with all modifier variants.
**11.5** Wayland GlobalShortcuts portal backend.
**11.6** All default hotkeys wired to their actions.
**11.7** Tray icon generation for each state.
**11.8** Tray menu built from config.
**11.9** Autostart for Windows registry and Linux `.desktop`.
**11.10** Single-instance enforcement with a friendly activation message to the running instance.

Deliverables: full system integration.

Exit criteria: hotkeys work globally on Windows and X11, the portal path is verified or its absence is documented precisely, autostart verified on a fresh login, tray menu fully functional.

---

### Phase 12, Wayland (Week 13)

**12.1** Layer-shell surface creation with `smithay-client-toolkit`.
**12.2** Anchor and margin mapping for all six positions.
**12.3** Configure handling, applying size before acknowledging.
**12.4** Exclusive zone with the ignore option.
**12.5** `set_input_region` for click-through on the layer surface.
**12.6** Keyboard interactivity `on_demand` so keyboard operation and hotkeys work.
**12.7** Idle inhibit through the `org.freedesktop.portal.Inhibit` portal while pinned.
**12.8** Per-output positioning by recreating the layer surface when the output changes.
**12.9** Runtime detection: layer-shell present or absent, with an XWayland fallback path.
**12.10** GNOME-specific notes and the `xdg_toplevel` fallback for compositors without layer-shell.

Deliverables: native Wayland support that matches the X11 feature set as closely as the protocol permits.

Exit criteria: verified on sway, Hyprland, and KDE Plasma 6 Wayland. Island anchors correctly, reserves no unwanted space, is click-through when collapsed, accepts keyboard when expanded, and never appears at the wrong position after a configure round trip.

---

### Phase 13, IPC and Extensibility (Week 14)

**13.1** Transport layer: Unix socket on Linux with `0600`, named pipe with a restrictive DACL on Windows.
**13.2** Line framing with a 64 KiB cap.
**13.3** Request and response codec with id matching.
**13.4** All 19 commands from 21.3.
**13.5** All 10 error codes from 21.4.
**13.6** Event push for the 5 events from 21.5.
**13.7** Rate limiting at 120 commands per second.
**13.8** Multi-client support up to 8 connections.
**13.9** `lumina-send` CLI with bash, zsh, and fish completions.
**13.10** `lumina-py` with sync and async clients and type hints.
**13.11** `lumina-node` with TypeScript types.
**13.12** Example integrations: a `make` wrapper, a GitHub Actions reporter, an `mpv` OSD hook.

Deliverables: a documented, stable, testable control surface.

Exit criteria: every command works from all three clients, the fuzz harness runs clean for 60 seconds, `spectrum peak` from Python returns data matching the on-screen bars within one frame.

---

### Phase 14, Performance and Hardening (Week 15)

**14.1** Counting global allocator with a frame-loop allocation assertion.
**14.2** Idle CPU measurement, targeting under 0.5 percent.
**14.3** Memory measurement at idle and under load, targeting 40 MB and 80 MB.
**14.4** Frame time instrumentation with p50, p95, p99.
**14.5** Early fragment out verification in the shader.
**14.6** Text layout caching.
**14.7** Startup optimization pass against the 400 ms budget.
**14.8** Power saving mode verification on battery.
**14.9** Panic policy implementation with `catch_unwind` on every worker.
**14.10** Graceful degradation matrix verification, one row at a time.
**14.11** Debug overlay with frame time, state, and memory.
**14.12** Performance regression baselines committed, CI gate at 5 percent.

Deliverables: a build that hits every number in section 3.2.

Exit criteria: every target in 3.2 measured and met, 10 minute soak test shows no memory growth, degradation matrix all green.

---

### Phase 15, Accessibility and Diagnostics (Week 15, continued)

**15.1** WCAG contrast enforcement with automatic clamping.
**15.2** Contrast unit test across all RGB modes and brightness values.
**15.3** Keyboard focus model with visual focus ring.
**15.4** Reduced motion from the OS setting.
**15.5** Screen reader mirroring via Windows notifications and `notify-send`.
**15.6** High contrast bundled theme.
**15.7** `--diagnostics` with the full environment dump.
**15.8** `--self-test` exercising every subsystem.
**15.9** `--benchmark`.
**15.10** Bug report template that pastes `--diagnostics` output.

Deliverables: an application that is usable by more people and debuggable by anyone.

Exit criteria: keyboard-only operation completes every action, contrast test green, `--self-test` passes in all four CI environments.

---

### Phase 16, Documentation, Packaging, Release (Week 16)

**16.1** README with a screenshot sequence, feature list, install instructions for all four artifacts, and a privacy statement.
**16.2** `docs/configuration.md`, generated from the schema so it can never drift.
**16.3** `docs/ipc.md` with every command, payload, and error code.
**16.4** `docs/themes.md` with the theme authoring guide and 8 examples.
**16.5** `docs/architecture.md` mirroring sections 8 through 12 of this plan.
**16.6** `docs/troubleshooting.md` covering the degradation matrix row by row.
**16.7** CHANGELOG with a Keep a Changelog structure.
**16.8** Windows `.msi` via WiX v4.
**16.9** Windows NSIS setup and portable zip.
**16.10** Linux `.deb`, `.rpm`, `.apk`, AppImage from shared staging.
**16.11** Shell completions installed by every Linux package.
**16.12** Full manual test matrix from 27.5, roughly 60 runs.
**16.13** Release signing checklist: reproducible build flag, version stamp, changelog, tag.
**16.14** Tag `v0.1.0-alpha` and publish artifacts.

Deliverables: a released, documented, installable V0.1.0-Alpha.

Exit criteria: all 60 manual runs pass or have documented exceptions, a clean-room install on a fresh Windows 10 machine and a fresh Ubuntu 24.04 machine works from the published artifact, docs contain no stale information.

---

### Phase 16.1, Buffer and Stretch (post-release, not in the 16 week estimate)

Reserved for polish that does not block the release: additional themes, more icons, a spectrum style gallery, per-widget layout customization, a settings GUI, and community theme discovery through a static index file rather than the network.

---

## 30. Risks and Mitigations

| ID | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Wayland without layer-shell (GNOME) gives poor positioning | high | medium | Document the XWayland path as the recommended option on GNOME. Provide `position` and `offset` so XWayland users get correct placement. Phase 1 proves this on real GNOME before Phase 12 starts |
| R2 | wgpu API churn between 0.x releases | medium | medium | Pin an exact minor version. Isolate all wgpu usage in `lumina-render` so an upgrade touches one crate. No wgpu types leak past its public API |
| R3 | MSDF atlas outgrows 1024 x 1024 | medium | low | Build-time assertion at 90 percent fill. Multi-page atlas support is a renderer change with no config or API impact |
| R4 | GPU driver rejects a shader | low | high | `naga` validation, error scopes, fallback shader, tray notification. Phase 2 tests on four driver families: NVIDIA, AMD, Intel, and llvmpipe |
| R5 | Wayland global hotkey portal unavailable | high on some distros | medium | Detect and disable gracefully. Document that keyboard-scoped hotkeys still work. Offer the XWayland path as a workaround |
| R6 | Audio backend differences across PulseAudio versions | medium | medium | Prefer PipeWire when present, since PipeWire is the default on new distros and is version-stable. Fall back to PulseAudio only when PipeWire is absent |
| R7 | Idle CPU regresses as features land | high | high | The idle sleep mechanism is built in Phase 2, not bolted on at the end. A CI test asserts fewer than 10 frames in 60 idle seconds. This gate runs from Phase 6 onward |
| R8 | Memory exceeds 40 MB on some driver | medium | medium | The budget breakdown in 3.3 names every allocation. Per-allocation budgets are documented so regressions are attributable |
| R9 | Scope creep into a settings GUI | medium | medium | Non-goals in 2.2. The GUI is explicitly deferred to post 1.0 |
| R10 | One developer burns out over 16 weeks | medium | high | Phases are independently shippable. Every phase ends in a working binary. If the schedule slips, we release at the last complete phase |
| R11 | MPRIS position polling drifts | medium | low | Local extrapolation between 1 Hz property reads, tested against a real player for 10 minutes |
| R12 | Config complexity intimidates users | medium | medium | Comments on every key, `--check-config`, eight bundled themes as starting points, and a documented minimum viable config of 12 lines |
| R13 | Single developer is the bus factor | high | high | Mitigation is documentation quality, not process. `docs/architecture.md` and generated config docs mean a new contributor can navigate the codebase |
| R14 | Time estimate is wrong | high | medium | The plan is phase-gated, not date-gated. Progress is measured by exit criteria passing, not by calendar |

---

## 31. V0.1.0-Alpha Acceptance Criteria

Every line below is a checkbox that must be genuinely checked, verified by evidence rather than by assertion.

### Rendering and shape
- [ ] The island renders an inverse trapezoid, top edge wider than bottom edge, at the configured position.
- [ ] Anti-aliased edges with no shimmer at any zoom level or DPI scale.
- [ ] Rounded corners follow the derivation in 10.3 and match a brute-force reference within 1e-3.
- [ ] The rectangle degeneracy case (equal top and bottom widths) is exact.
- [ ] Optional border renders as a true SDF isoline of the configured width.
- [ ] Optional glow renders with exponential falloff and no visible ring artifact.
- [ ] Every geometry value clamps to its documented range with no invalid configuration able to produce an invisible or screen-filling island.

### Animation
- [ ] All 8 states reachable, from all pairs, with no visual glitch.
- [ ] Morph settles in 280 ms plus or minus 10 percent with a single overshoot under 5 percent.
- [ ] Interrupting a morph mid-flight produces no positional jump greater than 2 px equivalent.
- [ ] The trapezoid never inverts during any transition.
- [ ] The inverted-trapezoid invariant holds even if config sets `bottom_half_width` above `top_half_width`.
- [ ] Reduced motion produces near-instant transitions with no overshoot.

### Media
- [ ] Spotify desktop controls on Windows and Linux.
- [ ] YouTube Music in Chrome, Firefox, and Edge controls on both platforms.
- [ ] Local players control: VLC, mpv, and at least one native Linux music player.
- [ ] Play, pause, next, previous, seek, and volume all function.
- [ ] Album art loads from all sources in 16.2, with a generic icon on failure.
- [ ] A player disappearing mid-playback transitions cleanly with no crash.
- [ ] Multi-player selection works and the user can cycle players.

### Audio and spectrum
- [ ] Spectrum renders from system output on Windows, PulseAudio, and PipeWire.
- [ ] 32 bands at 60 fps with no strobing and no visible aliasing.
- [ ] A 1 kHz sine produces a single clear peak, verified by the automated test.
- [ ] Attack and release timings match the configuration.
- [ ] Silence fades to a clean zero with no residual jitter.
- [ ] Device unplug mid-session produces a graceful message, not a crash.
- [ ] Total CPU for the audio path stays within the 17.7 budget.

### RGB
- [ ] All 5 engines work and crossfade without a visible jump.
- [ ] Reactive mode visibly responds to bass and treble within one frame.
- [ ] Album mode derives a palette from real cover art.
- [ ] Per-widget overrides take precedence correctly.
- [ ] All text meets WCAG AA contrast in every mode, enforced by test.

### Customization
- [ ] Every key in section 18.2 is documented with type, range, default, and hot-reload status.
- [ ] Every key hot-reloads within 50 ms, except the three documented restart keys.
- [ ] Invalid values produce a clear warning and the documented fallback, never a crash and never a total config rejection.
- [ ] Profiles inherit correctly and cycles are detected.
- [ ] Themes cascade correctly over base config.
- [ ] `--check-config` reports every problem in a file with multiple problems.
- [ ] The user WGSL hook compiles and renders, and rejects invalid WGSL safely.

### Platform
- [ ] Windows 10 1809 through 22H2 verified.
- [ ] Windows 11 21H2 through 25H2 verified.
- [ ] Ubuntu 22.04 and 24.04, X11 and Wayland, verified.
- [ ] Debian 12 and 13, X11, verified.
- [ ] sway, Hyprland, and KDE Plasma 6 Wayland verified.
- [ ] GNOME Wayland behavior documented precisely, including the XWayland recommendation.
- [ ] Mixed DPI multi-monitor setups render correctly on every monitor.
- [ ] Monitor disconnect and reconnection during operation handled.

### Resources
- [ ] Idle RSS under 40 MB after 60 seconds.
- [ ] Active RSS under 80 MB with media, spectrum, and all widgets.
- [ ] Idle CPU under 0.5 percent of one core.
- [ ] Spectrum CPU under 1.5 percent of one core.
- [ ] Startup to first paint under 400 ms on Windows and 600 ms on Linux.
- [ ] Zero allocations in the frame loop, verified by the counting allocator.
- [ ] 10 minute soak with no memory growth.
- [ ] No network traffic in a 10 minute run with weather disabled.

### Quality
- [ ] `cargo clippy` clean with zero warnings.
- [ ] `cargo fmt --check` clean.
- [ ] `cargo deny check` clean.
- [ ] `cargo audit` clean.
- [ ] CI green on every job in 27.4.
- [ ] All fuzz smoke targets run clean.
- [ ] Screenshot regression tests pass on Lavapipe.
- [ ] 400 or more unit tests, all green.

### Distribution
- [ ] `.msi` installs and uninstalls cleanly on Windows 10 and 11.
- [ ] NSIS setup installs cleanly, portable mode works.
- [ ] `.deb` installs on Debian 12 and Ubuntu 22.04 and 24.04.
- [ ] `.rpm` installs on Fedora 40.
- [ ] AppImage runs on Ubuntu 22.04 and 24.04.
- [ ] Clean-room install from the published artifact works on a machine with no Rust toolchain.
- [ ] Shell completions installed and functional.

---

## 32. Post 1.0 Roadmap

Listed so that scope pressure during V0.1.0 is a conscious rejection, not an accident.

| Version | Theme | Contents |
|---|---|---|
| 0.2.0 | Depth | Plugin ABI with a capability sandbox, more widgets, per-widget layout customization, spectrum style gallery, additional bundled themes |
| 0.3.0 | Reach | Settings GUI, community theme index (a static file, not a service), Wayland global hotkeys without a portal via `ext-session-lock` workarounds where legal |
| 0.4.0 | Polish | Signed updates with a TUF-like scheme, HDR support, multi-island support with several regions of one screen, scene animations beyond the state machine |
| 1.0.0 | Stable | API stability guarantees for the plugin ABI and IPC, long-term support branch, 12 month release cadence |

---

## 33. Appendices

### Appendix A, Complete default config

This is the file written on first run. Every key carries a comment explaining what it does, in the same file the user will edit.

```toml
# Lumina V0.1.0-Alpha configuration
# Docs: docs/configuration.md
# Validate after editing:  lumina --check-config

version = 1

[general]
position = "top-center"          # top-center, top-left, top-right, bottom-center, bottom-left, bottom-right
offset_x = 0
offset_y = 12
monitor = "primary"              # index, "primary", or "cursor"
auto_hide = false                # fade the island out when idle
auto_hide_delay_ms = 3000
always_clickable = false         # stay clickable even when collapsed
idle_timeout_ms = 5000           # collapse after this long with nothing happening
click_through_when_collapsed = true
start_hidden = false
single_instance = true

[appearance]
fill = "#12121ACC"               # 8 digit RGBA hex
fill_gradient = 0.0              # 0 solid, 1 full gradient to secondary
corner_radius = 18
top_half_width = 62              # half width of the wider, top edge
bottom_half_width = 54           # half width of the narrower, bottom edge
height = 36
inverted_trapezoid = true        # false makes a regular trapezoid
border_width = 1.0
border_color = "#FFFFFF24"
glow_spread = 8
glow_falloff = 8.0
shadow_opacity = 0.28
backdrop_blur = "auto"           # auto, on, off
backdrop_darken = 0.15
noise_strength = 0.012           # dither, removes gradient banding
corner_style = "rounded"         # rounded, squircle, chamfer (restart to apply)

[rgb]
mode = "breathing"               # static, breathing, cycling, reactive, album
primary = "#7C3AED"
secondary = "#06B6D4"
speed = 1.0
brightness = 1.0
saturation = 0.9
breath_rate_hz = 0.25
breath_depth = 0.18
glow_intensity = 0.6
cycle_degrees_per_second = 40.0
transition_ms = 300

[rgb.reactive]
gain = 1.0
floor = 0.12
bass_hz = 250
mid_hz = 2000
hue_spread_degrees = 90.0
glow_from_bass = 0.55

[audio]
enabled = true
source = "system"                # system, microphone, none
device = "default"
backend = "auto"                 # auto, pulse, pipewire, wasapi
fft_size = 4096
band_count = 32
sensitivity = 1.8
attack = 0.35
release = 0.08
silence_threshold_db = -60.0
bass_tilt_db_per_octave = 3.0
normalize = true

[audio.visualizer]
style = "bars"                   # bars, mirror, waveform, dots
bar_spacing = 2
bar_radius = 3
peak_hold_ms = 400
hue_spread_degrees = 120.0
hot_tip = true
baseline = true

[media]
enabled = true
hold_ms = 6000
show_album_art = true
album_art_size = 52
album_art_corner_radius = 8
show_progress = true
progress_height = 3
scrub_on_click = true
show_volume = false
marquee = true
marquee_speed_px_per_s = 40.0
marquee_gap_px = 48
marquee_pause_on_hover_ms = 1800
preferred_players = []
cycle_players_hotkey = ""

[clock]
enabled = true
format = "24h"
show_seconds = false
show_date = true
date_format = "%d %b"
show_weekday = false
second_timezone = ""
hour_leading_zero = true

[metrics]
enabled = false
interval_ms = 1000
show_cpu = true
show_ram = true
show_gpu = false
show_network = false
sparkline_bars = 24
alert_on_threshold = 0.0         # 0 disables, 0.9 alerts above 90 percent

[battery]
enabled = false
interval_ms = 5000
show_percentage = true
show_time_remaining = true
show_charging_bolt = true

[network]
enabled = false
interval_ms = 1000
show_upload = true
show_download = true
show_ssid = false
unit = "auto"                    # auto, b, kb, mb, gb

[pomodoro]
enabled = false
focus_minutes = 25
short_break_minutes = 5
long_break_minutes = 15
rounds_before_long = 4
notify_phases = true

[weather]
enabled = false
provider = "open-meteo"          # open-meteo (no key needed), openweathermap
api_key = ""
latitude = 0.0
longitude = 0.0
units = "metric"
cache_ttl_minutes = 15
interval_ms = 900000

[layout]
padding_x = 12
padding_y = 6
spacing = 10
align = "center"                 # top, center, bottom
order = ["clock", "media", "spectrum", "metrics", "battery", "network", "pomodoro", "shortcuts"]
max_widgets = 3

[hotkeys]
toggle_visibility = "ctrl+alt+l"
cycle_widget = "ctrl+alt+tab"
cycle_back = "ctrl+shift+tab"
rgb_next_mode = "ctrl+alt+r"
profile_next = "ctrl+alt+p"
media_play_pause = ""
media_next = ""
media_previous = ""
media_stop = ""
cycle_players = ""
collapse_now = ""
reload_config = ""
toggle_debug_overlay = "ctrl+alt+d"

[[shortcuts.list]]
id = "screenshot"
label = "Take a screenshot"
icon = "camera"
command = ""
show_in_island = true
# Linux example:   command = "flameshot gui"
# Windows example: command = "powershell -c \"Add-Type -AssemblyName System.Windows.Forms;\""

[tray]
enabled = true
show_on_startup = false
icon = "auto"

[motion]
reduced = false
master_scale = 1.0
morph_response_s = 0.34
morph_damping = 0.78
content_fade_ms = 160
startup_reveal = true

[text]
font_size = 14
font_weight = "regular"
title_size = 15
artist_size = 13
font_file = ""
color = "#FFFFFFFF"
dim_color = "#B3FFFFFF"
marquee = true
sharp = false
letter_spacing = 0.0

[advanced]
target_fps = 60
vsync = true
gpu_backend = "auto"             # auto, vulkan, dx12, metal, gl
log_level = "info"
log_file = ""
ipc_enabled = true
ipc_name = ""
ipc_allow_exec = false           # enables the dangerous "execute" IPC command
power_saving = true
experimental_wgsl = ""

[profile]
default = "balanced"

[profiles.balanced]

[profiles.focus]
inherit = "balanced"
rgb.mode = "static"
spectrum.enabled = false
media.hold_ms = 3000
```

### Appendix B, IPC quick reference

```
lumina-send status
lumina-send show --widget media --duration 5000
lumina-send notify --title "Build done" --body "lumina 0.1.0-alpha"
lumina-send set-rgb --mode reactive
lumina-send set-profile --name focus
lumina-send media --action play_pause
lumina-send spectrum --peak
lumina-send geometry --top 90 --bottom 80 --height 44 --radius 20
```

Python:

```python
from lumina import Lumina

c = Lumina()
c.notify(title="Deploy finished", body="All green")
c.set_rgb(mode="reactive")
print(c.status()["state"])
```

Node:

```javascript
const { Lumina } = require('lumina');
const l = new Lumina();
l.on('media_changed', (m) => console.log(m.title, m.status));
l.notify({ title: 'Build done', body: 'lumina' });
```

### Appendix C, Symptom to cause quick table

| Symptom | Likely cause | Fix |
|---|---|---|
| Island invisible | `bottom_half_width` above `top_half_width` with `inverted_trapezoid = true`, or `fill` alpha of 0 | Check `--check-config` output |
| Island behind windows | Running under Wayland without layer-shell | Run under XWayland, or use a compositor with layer-shell |
| Island in the wrong place | Multi-monitor with `monitor = "primary"` and a different main display | Set `monitor` to the desired index |
| No spectrum on Linux | No PulseAudio or PipeWire session found | Run `lumina --diagnostics`, check the audio backend line |
| No spectrum on Windows | Exclusive-mode audio device | Switch the device to shared mode in Sound settings |
| Spectrum frozen | Device unplugged | Expected behavior, message shows in the widget |
| Spotify not detected | Spotify running as a Flatpak sandbox | Flatpak needs `--talk-name=org.mpris.MediaPlayer2.*` portal access |
| Hotkeys do nothing on Wayland | GlobalShortcuts portal unavailable | Expected, documented in 20.2 |
| Text looks blurry | User WGSL override broken, or atlas missing | Remove `experimental_wgsl`, verify assets |
| High memory | Old GPU driver allocating large heaps | Update the driver, check `--diagnostics` |
| Config changes ignored | Editing the wrong file | Confirm the path printed by `--diagnostics` |
| Two islands running | Single instance disabled or lock file stale | Enable `single_instance`, remove the stale lock |
| Nothing at all on startup | No usable GPU API | Check `--self-test` output for the shader step |

### Appendix D, Glossary

| Term | Meaning |
|---|---|
| ARGB visual | X11 visual with a per-pixel alpha channel, required for transparency |
| Backdrop blur | Blurring whatever is behind the island, done by the compositor or by us |
| Chamfer | A corner cut at 45 degrees instead of rounded |
| D-Bus | The Linux inter-process message bus, how MPRIS works |
| Dirty flag | A boolean saying something changed and a redraw is needed |
| EWMH | Extended Window Manager Hints, the X11 convention for dock and always-on-top |
| EWSR | Early Window System Redraw, the dirty flag concept from X11 |
| GSMTC | Global System Media Transport Controls, the Windows media session API |
| Layer-shell | The Wayland protocol that gives a client a reserved always-on-top surface |
| MPRIS2 | The Media Player Remote Interfacing Specification, the Linux media session API |
| MSDF | Multi-channel Signed Distance Field, text rendering that stays sharp at any size |
| OkLab | A perceptually uniform color space, used so color interpolation does not go muddy |
| SDF | Signed Distance Field, a function giving distance to a shape, negative inside |
| Semaphore-free SPSC | A ring buffer where one thread writes and another reads with no locks |
| Shell | The user's command interpreter, used to run shortcut commands |
| Slant | The horizontal offset per unit height that makes a trapezoid a trapezoid |
| Squircle | Apple's continuous corner curve, a softer alternative to a circular corner |
| SPSC | Single Producer Single Consumer |
| Wayland | The display server protocol replacing X11 |
| wlr-layer-shell | The layer-shell protocol, implemented by wlroots compositors and KDE |
| XWayland | Running X11 applications inside a Wayland session |

### Appendix E, Version history of this plan

| Revision | Change |
|---|---|
| r1 | Initial plan, framework decision, 13 phases |
| r2 | Added Wayland layer-shell, RGB modes, profiles, IPC bindings |
| r3 | Added glass morphology, weather widget, pomodoro, system monitor |
| r4 | Complete rewrite. Added derivations for the trapezoid SDF, rounding by polygon inset, spring parameter mapping, OkLab color pipeline, log band mapping, memory budget breakdown, the 60 key configuration schema with per-key validation, the state machine with priority arbitration, platform capability matrix, full Windows X11 and Wayland backend details with exact API calls, 5 customization layers, 19 IPC commands with 10 error codes, 14 unit tests that matter, the 60 run manual matrix, full acceptance criteria, 14 risks, and appendices |

---

**End of document.**