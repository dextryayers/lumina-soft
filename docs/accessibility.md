# Accessibility

Lumina is an overlay rather than a conventional window, so accessibility is
built from explicit, testable pieces instead of relying on a platform
accessibility tree.

## Configuration

```toml
[accessibility]
enforce_contrast = true
contrast_target = 4.5
keyboard_focus = true
focus_ring_width = 2.0
focus_ring_offset = 2.0
focus_ring_color = "#FFFFFFFF"
respect_os_reduced_motion = true
screen_reader = "auto"          # auto | on | off
announce_min_interval_ms = 1500
```

Validation clamps `contrast_target` to `3.0..=7.0`, focus dimensions to
`0..=8`, and the announcement interval to `0..=60000` ms. A zero-width focus
ring is raised to `2.0` whenever keyboard focus is enabled.

## Contrast

Every text colour is checked against the resolved fill after a palette
transition. The engine adjusts luminance only when the contrast falls below the
target, so normal palettes are untouched. The bundled `high-contrast` theme
targets `7.0` with pure black and white.

## Keyboard

- The input router stores real widget ids, not positional placeholders.
- `InputRouter::focus_next` and `focus_previous` implement wrapping Tab and
  Shift+Tab traversal. A caller maps the platform keys.
- `InputRouter::retain_focus` drops focus when a layout change removes the
  focused widget.
- `InputRouter::activate_focused` maps Enter or Space to a primary click at the
  focused widget's centre; `dismiss_focus` maps Escape.
- `DrawList::push_focus_ring` emits a four-edge ring using the configured width,
  offset, and colour.

## Reduced motion

Set `motion.reduced = true`, or let Lumina query the OS. Windows uses
`SPI_GETCLIENTAREAANIMATION`; GNOME and KDE are checked through their animation
settings on Linux. `LUMINA_REDUCED_MOTION=1` or `0` overrides detection.
Reduced motion switches every palette transition to immediate and pairs with
the fast, critically damped spring profile.

## Screen reader mirroring

Overlay windows are not accessibility-tree citizens. Lumina therefore mirrors
state changes to platform notifications. Detection uses
`SPI_GETSCREENREADER` on Windows and Orca/AT-SPI environment markers on Linux;
`LUMINA_SCREEN_READER=1` overrides it. `AnnouncementGate` enforces
`announce_min_interval_ms`, collapses bursts, and reports how many
announcements were dropped. `deliver_announcement` sends the accepted message
through a Windows toast or `notify-send`, spawning the helper detached so a slow
notification daemon cannot stall the UI. The orchestrator owns calling it
because it owns the state-change stream.

## Diagnostics

```powershell
lumina --diagnostics
lumina --self-test
lumina --benchmark --output benchmark.json
```

`--diagnostics` includes platform capabilities, process resident memory, config
validity, the resolved accessibility settings, and OS reduced-motion/screen-reader
state. Paste it into the bug report template.

`--self-test` covers defaults, shipped config, themes, all RGB engines, WCAG
contrast, WGSL validation, shaping, layout caching, idle pacing, process memory,
and platform capability consistency without needing a display or GPU.

`--benchmark --baseline perf/baseline-<platform>.json` enforces the five percent
CPU regression gate. See `perf/README.md` for the reference-machine procedure.
