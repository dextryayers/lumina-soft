# Changelog

All notable changes to Lumina are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Configuration schema
changes are versioned separately and must include a repair path.

## [Unreleased]

### Added

- Configuration CLI commands: `--init-config`, `--print-config`,
  `--list-themes`, `--export-theme`, `--check-theme`, and `--config`.
- Theme export and parse APIs, plus eight validated theme examples.
- Generated configuration reference and a drift check in `xtask`.
- Release verification task covering docs, packaging metadata, changelog,
  license, themes, workspace versions, and CI commands.
- Architecture, theme authoring, troubleshooting, manual-test, and release
  documentation.
- Windows and Linux packaging definitions and shell completions.

### Changed

- `--diagnostics` and `--check-config` accept a custom configuration path.
- IPC documentation examples use valid RGB mode names.

### Fixed

- Stale `wave` examples in `docs/ipc.md` that did not match the protocol.

## [0.1.0-alpha.1] - 2026-10-20

This entry records the alpha codebase. It is **not** a published release. The
runtime orchestrator and window are still in progress, and no installer artifact
has passed the release checklist.

### Added

- Configuration engine with TOML parsing, annotated defaults, per-key repair,
  validation warnings and errors, profiles, themes, inheritance, and hot reload.
- Island state machine, priority arbiter, SDF geometry, spring integration, and
  explicit transition edge cases.
- Color engine with sRGB and OkLab conversion, WCAG contrast enforcement,
  palette modes, and high-contrast support.
- Text shaping, layout cache, glyph atlas, overflow handling, and alignment.
- Widget framework and widget modules for clock, media, spectrum, metrics,
  battery, network, pomodoro, weather, notifications, scripts, and shortcuts.
- Audio capture and analysis library with a single-producer/single-consumer ring
  buffer, FFT bands, and envelope attack/release behavior.
- Media session abstraction and platform backends for MPRIS on Linux and GSMTC
  on Windows.
- Platform capability probes, X11 and Wayland helpers, Windows helpers,
  single-instance handling, and per-user autostart management.
- WGSL renderer with SDF body, border, glow, backdrop blur, content pass,
  spectrum pass, shader validation, fallback shader, and dirty-flag frame loop.
- IPC protocol with 19 commands, 5 events, 11 stable error codes, NDJSON framing,
  rate limiting, up to 8 clients, Unix socket and Windows named-pipe transports,
  plus Python, Node, and `lumina-send` clients.
- CLI version, diagnostics, configuration checks, headless self-test, CPU
  benchmarks, baseline comparison, and custom output paths.
- Headless frame-loop allocation test and committed Windows benchmark baseline.
- Process-memory diagnostics for Windows and Linux.
- Accessibility model covering contrast, keyboard focus, reduced motion,
  high-contrast theme, minimum text size, DPI scaling, and screen-reader
  mirroring hooks.
- Security model documentation for IPC permissions, command execution, API-key
  handling, WGSL validation, and the absence of telemetry.

### Changed

- The headless app reports platform capabilities and points to the runtime
  milestone instead of pretending to own a window.

### Security

- IPC limits requests to 120 commands per second per connection and disables
  `execute` unless explicitly enabled in advanced configuration.
- Shortcut and script execution is documented as user-controlled local process
  execution, not a sandbox.

### Known limitations

- No window or runtime orchestrator is wired, so the island cannot be displayed.
- Installer artifacts are defined but have not been built or clean-room tested.
- Media and screen-reader integration are implemented as libraries and platform
  hooks; their runtime event wiring is part of the orchestrator milestone.
- Windows builds are unsigned for this alpha.
- A dedicated performance runner is still required for a trustworthy 5% CI
  regression gate.

[Unreleased]: https://github.com/lumina-sh/lumina/compare/v0.1.0-alpha.1...HEAD
[0.1.0-alpha.1]: https://github.com/lumina-sh/lumina/releases/tag/v0.1.0-alpha.1
