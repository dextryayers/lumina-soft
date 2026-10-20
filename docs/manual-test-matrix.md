# Manual test matrix

This is the V0.1.0 acceptance matrix. It is intentionally larger than the
automated suite because it covers real compositors, drivers, devices, and
installers. Run it against a release candidate, not a development build.

> **Status.** The runtime orchestrator is not wired yet, so every desktop run
> below is blocked on the window milestone. Installer rows are additionally
> blocked on the packaging tools and a published artifact. Do not mark a row as
> passed from a headless self-test. Headless equivalents live in CI.

## Status legend

| Status | Meaning |
|---|---|
| `PASS` | Observed behavior matched the expected result, with evidence attached |
| `FAIL` | Behavior differed; an issue link and workaround are recorded |
| `EXCEPTION` | The failure is accepted for this release and documented in the notes |
| `PENDING` | Not executed yet |
| `BLOCKED` | Cannot be executed because a prerequisite is unavailable |

Every run record needs the build version, target triple, artifact name and
checksum, OS build, session type, GPU and driver, audio server, and a short
screen recording or log. Do not attach private notification bodies, media paths,
or API keys.

## Mandatory OS and session combinations

These 19 rows are mandatory. Windows sessions are native; Linux sessions are
X11, wlroots Wayland, KDE Wayland, and GNOME Wayland with the documented
fallback.

| ID | OS | Session | Install | Expected baseline |
|---|---|---|---|---|
| M01 | Windows 10 22H2 | Native | MSI | Collapsed island appears at top center; tray and hotkeys work |
| M02 | Windows 11 23H2 | Native | MSI | Collapsed island appears; DPI and autostart work |
| M03 | Windows 11 25H2 | Native | MSI | Collapsed island appears; DPI and autostart work |
| M04 | Ubuntu 22.04 | X11 | deb | ARGB overlay, always-on-top, hotkeys, IPC |
| M05 | Ubuntu 22.04 | Wayland wlroots | AppImage | Layer-shell overlay, no focus stealing, IPC |
| M06 | Ubuntu 22.04 | Wayland KDE | AppImage | Layer-shell or documented KDE path, always-on-top |
| M07 | Ubuntu 22.04 | Wayland GNOME | AppImage | Documented fallback; no forced non-working overlay |
| M08 | Ubuntu 24.04 | X11 | deb | ARGB overlay, always-on-top, hotkeys, IPC |
| M09 | Ubuntu 24.04 | Wayland wlroots | AppImage | Layer-shell overlay, no focus stealing, IPC |
| M10 | Ubuntu 24.04 | Wayland KDE | AppImage | Layer-shell or documented KDE path, always-on-top |
| M11 | Ubuntu 24.04 | Wayland GNOME | AppImage | Documented fallback; no forced non-working overlay |
| M12 | Debian 12 | X11 | deb | ARGB overlay, always-on-top, hotkeys, IPC |
| M13 | Debian 12 | Wayland wlroots | AppImage | Layer-shell overlay, no focus stealing, IPC |
| M14 | Debian 12 | Wayland KDE | AppImage | Layer-shell or documented KDE path, always-on-top |
| M15 | Debian 12 | Wayland GNOME | AppImage | Documented fallback; no forced non-working overlay |
| M16 | Debian 13 | X11 | deb | ARGB overlay, always-on-top, hotkeys, IPC |
| M17 | Debian 13 | Wayland wlroots | AppImage | Layer-shell overlay, no focus stealing, IPC |
| M18 | Debian 13 | Wayland KDE | AppImage | Layer-shell or documented KDE path, always-on-top |
| M19 | Debian 13 | Wayland GNOME | AppImage | Documented fallback; no forced non-working overlay |

Status: `BLOCKED` for all 19 until the orchestrator and artifacts exist.

## DPI and display scaling

| ID | Environment | DPI | Expected |
|---|---|---|---|
| M20 | Windows 11 | 100% | Crisp text, expected geometry, no blur |
| M21 | Windows 11 | 125% | Text and island scale, no clipping |
| M22 | Windows 11 | 150% | Text and island scale, no clipping |
| M23 | Windows 11 | 200% | Text and island scale, no clipping |
| M24 | Windows 11 + Ubuntu | Mixed 100/150% dual monitor | Correct scale after moving between monitors; position preserved |

Status: `BLOCKED` on the window milestone.

## Media players

Run each row on Windows and Linux, with the player playing a known track.

| ID | Player | Expected |
|---|---|---|
| M25 | Spotify desktop | Art, title, artist, transport, progress, seek |
| M26 | Chrome media session | Metadata, play/pause, next/previous where supported |
| M27 | Firefox media session | Metadata, play/pause, next/previous where supported |
| M28 | VLC | Metadata and transport commands |
| M29 | mpv | Metadata and transport commands |
| M30 | None playing | Media widget hidden; no error |

Status: `BLOCKED` on the window and media integration milestones.

## Audio capture

| ID | Server or device | Expected |
|---|---|---|
| M31 | PulseAudio | Spectrum follows output, no clipping at normal volume |
| M32 | PipeWire | Spectrum follows output through the PulseAudio compatibility layer |
| M33 | Silent output | Bars at zero, no fake animation |
| M34 | Microphone only | Configured source sampled; no feedback loop |
| M35 | Device unplugged mid-session | Bars freeze at zero, one warning, capture recovers on reconnect |

Status: `BLOCKED` on the window milestone.

## Configuration and themes

| ID | Scenario | Expected |
|---|---|---|
| M36 | Defaults, first run | Annotated config created; validation clean; island at documented defaults |
| M37 | Heavily customized file | Every supported section applies; no ignored keys |
| M38 | Invalid keys and values | Repairs are applied; warnings name each key; process starts |
| M39 | Profiles and inherited profiles | Switching is live; inherited values resolve in the documented order |
| M40 | Theme switch mid-session | Colors and geometry transition without restart; no flash of defaults |

Status: `BLOCKED` on the window milestone. `--check-config`, theme examples, and
profile resolution have headless tests in CI.

## Artifact installation

Use a clean VM for each row. Verify uninstall, Start menu or desktop entry,
completions, autostart, and file ownership.

| ID | Artifact | Expected |
|---|---|---|
| M41 | Windows MSI | Per-user install without elevation; Start menu entry; uninstall removes files and shortcut |
| M42 | Windows NSIS setup | Per-user install; uninstaller; portable path remains independent |
| M43 | Windows portable ZIP | Extracts and runs without installation; config path remains per-user |
| M44 | Debian/Ubuntu deb | `/usr/bin/lumina`, desktop entry, icon, completions; apt remove is clean |
| M45 | RPM | Same staged payload as deb; rpm -e is clean |
| M46 | AppImage | Runs without installation; AppRun locates resources; no missing libraries |
| M47 | Alpine apk | musl build starts; desktop entry and completions installed |

Status: `PENDING` for MSI, NSIS, ZIP, deb, rpm, apk, and AppImage because the
host lacks packaging tools. `BLOCKED` on the runtime gate for a meaningful UI
test.

## IPC and integrations

| ID | Scenario | Expected |
|---|---|---|
| M48 | `lumina-send status` | Full state dump; exit code 0 |
| M49 | `lumina-send notify` | Notification appears with title and body; no shell interpretation |
| M50 | `lumina-send set_rgb` | Palette changes live; invalid enum returns a stable error code |
| M51 | `lumina-send media --action play_pause` | Playback toggles; `PlayerGone` reported when the session disappears |
| M52 | Burst above 120 commands/second | Excess requests return `RateLimited`; connection remains usable |
| M53 | Kill Lumina during a request | Client exits 1 with a clear connection error; no hang |
| M54 | Wrong socket permissions | IPC disabled or rejected; warning logged; no privilege escalation |
| M55 | `execute` while disabled | Rejected with `PermissionDenied`; enabling it allows the configured command |
| M56 | Python and Node clients | Each connects, sends status, parses a response, and exits 0 |

Status: `BLOCKED` on the runtime server lifecycle for M48-M51 and M53-M56.
The codec, rate-limit logic, and client parsers are covered by automated tests.

## Accessibility and keyboard

| ID | Scenario | Expected |
|---|---|---|
| M57 | Tab, Shift+Tab, Enter, Space, Escape | Focus order is visible; actions activate; Escape collapses |
| M58 | Forced contrast clamp | Text stays at the configured ratio across all RGB modes and brightness values |
| M59 | OS reduced motion | Transitions are near-instant; pulse, shake, and slide are skipped |
| M60 | Screen reader enabled | State changes and notifications are announced once, without duplicates |
| M61 | High-contrast theme | Opaque fill, white text, no glow, visible focus ring |

Status: `BLOCKED` on the window and platform-notification hookup. Contrast,
focus-model, and high-contrast theme logic have unit tests.

## Run record

Copy this block for each executed row:

```text
ID:
Build version:
Artifact and SHA-256:
OS build:
Session:
GPU and driver:
Audio server:
DPI:
Result: PASS | FAIL | EXCEPTION
Evidence:
Notes or issue link:
```

A release is not approvable while any mandatory row is `PENDING` or `BLOCKED`
without an accepted exception. The release checklist records the final result.
