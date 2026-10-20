# Troubleshooting

Start with the environment dump. It is designed to be pasted into a bug report
without editing:

```sh
lumina --diagnostics
lumina --check-config
lumina --self-test
```

Use `lumina --config PATH` when the configuration is not in the default
location. Exit code `0` means success, `1` means a validation or subsystem
problem, and `2` means the command line was malformed.

## First check: is the runtime expected to be visible?

The current alpha is a headless build. The engine, configuration, IPC, and
platform probes are tested, but the orchestrator that creates and paints the
island is still in progress. Running `lumina` with no arguments prints the
environment and the notice:

```text
Lumina is not yet able to create its window. See PLAN.md Phase 6.
```

That message is expected in this build. It is not a failed installation. The
first troubleshooting step for every UI report is to confirm that the binary is
newer than the runtime gate announcement.

## Degraded-mode matrix

Lumina is designed to lose features, not to fail to start. The rows below match
the published degradation contract.

| Missing or broken dependency | Expected behavior | What to check |
|---|---|---|
| No Vulkan, but OpenGL 3.3 is available | Renders through the GLES backend with reduced glow and blur | `--diagnostics` lists the selected adapter; check the driver's OpenGL support |
| No usable GPU API | Solid fallback shape, tray notification, process stays alive | Look for the fallback notice in the log; update the GPU driver first |
| No audio server | Spectrum reports unavailable; all other widgets continue | Check `PULSE_SERVER`, PipeWire/PulseAudio status, and `pactl info` |
| No D-Bus session bus | Media widget hides; no error dialog | Check `DBUS_SESSION_BUS_ADDRESS` and whether a session bus is running |
| No tray support | Menus remain reachable through IPC and hotkeys; warning logged once | Verify the desktop shell exports a StatusNotifier/AppIndicator host |
| IPC socket cannot be created with mode `0600` | IPC disabled; warning logged once | Inspect the runtime directory owner and permissions |
| Configuration directory is not writable | Built-in defaults are used; warning logged once, without repeating | Check `%APPDATA%\lumina` or `$XDG_CONFIG_HOME/lumina` ownership |
| Configuration file is missing | Defaults are used; no error | Run `--init-config` to create an annotated file |
| Configuration has invalid values | Invalid keys are repaired individually; warnings name the key | Run `--check-config` and apply the suggested value |
| Configuration has unknown keys | Warning with a spelling suggestion; unknown keys are ignored | Compare against the generated [configuration reference](configuration.md) |
| Theme file cannot be parsed | Theme is skipped; other themes still load | Run `--list-themes` to see the parser error |
| Theme directory is absent | Built-in themes remain available | This is normal; create `themes/` only when adding local themes |
| Screen reader bridge is unavailable | Visual UI continues; announcements are skipped | Confirm the platform notification service is present |
| Global hotkey registration fails | Hotkey is skipped; warning names the combination | Check for another application already owning the hotkey |
| Autostart entry cannot be written | Manual launch continues to work | Check the per-user startup directory permissions |

## Configuration problems

Run:

```sh
lumina --check-config
```

The report separates warnings from errors. A repaired value is applied in
memory, but the file on disk is not silently rewritten. Edit the reported key
and run the command again. To start from a known-good file:

```sh
lumina --init-config --force
```

`--force` replaces the selected configuration. Back up customizations first.

Common causes:

- A color without a leading `#`, or with an unsupported length.
- A numeric value outside the documented range.
- An unknown enum value. The valid values are listed in
  [`configuration.md`](configuration.md).
- A profile or theme that refers to a parent which does not exist.
- A script or shortcut command that contains shell syntax. Lumina executes
  programs directly, without a shell.

## Theme problems

```sh
lumina --list-themes
lumina --check-theme path/to/theme.toml
```

`--check-theme` applies the theme to the selected base configuration and runs
the full validator. A malformed theme is skipped by the loader, so it cannot
prevent Lumina from starting. If an exported theme is edited, keep the `name`
unique; duplicate display names are confusing even though lookup by filename and
case-insensitive name remains deterministic.

Image backgrounds are parsed but not rendered yet. Use color-only themes in the
current alpha.

## IPC problems

Check the protocol reference in [`ipc.md`](ipc.md). The client reports three
classes of failure:

| Exit code | Meaning | Action |
|---|---|---|
| `0` | Command accepted | None |
| `1` | Connection or protocol failure | Confirm Lumina is running and the socket or pipe exists |
| `2` | Command rejected with an error code | Read the printed error and adjust the payload |
| `3` | Invalid client command line | Run `lumina-send --help` |

On Unix the socket is normally under `$XDG_RUNTIME_DIR`. On Windows it is a
per-user named pipe. `lumina-send --socket PATH` and `--pipe NAME` override the
defaults for testing. A permissions error usually means the socket is owned by
another user or the runtime directory mode is too broad.

The server rate-limits at 120 commands per second per connection. A
`RateLimited` response is backpressure, not a crash. Close and reconnect only
after the burst has stopped.

## Audio and media

- Spectrum frozen at zero: check that capture is enabled, the device is not
  muted, and another exclusive-mode application is not holding it.
- Media widget hidden: no supported session is active, D-Bus is unavailable, or
  the platform session proxy failed. `lumina-send player_list` shows what the
  backend sees when the runtime is connected.
- Track metadata missing: the player may expose no title or art. Lumina falls
  back to the player identity and a neutral icon.
- Album art does not update: the player may cache the old URL. Skip to the next
  track and back before filing a bug.

## Performance

Idle rendering is dirty-flag driven. If CPU use is unexpectedly high:

1. Run `lumina --diagnostics` and note the adapter and platform.
2. Check whether an audio device is producing continuous input; spectrum is
   active while audio is active.
3. Run `lumina --benchmark` and compare with the committed baseline using
   `--baseline perf/baseline-windows.json`.
4. Capture a debug overlay once the orchestrator is available. Until then,
   headless benchmarks are the supported measurement.

Memory is reported by `--diagnostics`. The idle allocation test covers the frame
loop, but it does not replace a long-running soak on a real desktop.

## Platform notes

### Windows

- Per-user installation does not require elevation.
- SmartScreen may warn on unsigned alpha builds. Signing is out of scope for
  this release and is documented in the release checklist.
- Autostart writes to the current user's startup location. `--autostart
  status` shows the registered command.

### Linux

- X11 and Wayland are both built in. XWayland users use the X11 path.
- Wayland overlays require a compositor that supports the layer-shell protocol.
  GNOME does not provide the same always-on-top behavior; use the documented
  fallback instead of forcing a non-working mode.
- Install the PulseAudio or PipeWire development/runtime packages if audio
  capture is unavailable. A system without them still runs without spectrum.
- AppImage and distribution packages install shell completions. If completion
  is missing, check the package's file list before reporting a bug.

## Reporting a bug

Include:

- `lumina --version`
- `lumina --diagnostics`
- `lumina --check-config` output, with private paths redacted if necessary
- The exact command that failed and its exit code
- Whether the binary came from a package, AppImage, or a local build

Never include an API key, notification body, media path, or the full contents of
a private configuration in a public report. The diagnostics output is designed
to avoid those values, but configuration files are not.
