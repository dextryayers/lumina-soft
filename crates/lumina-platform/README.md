# lumina-platform

Display server abstraction for Lumina.

## The problem this crate solves

The island has to do five things that no portable windowing abstraction offers:

1. Stay above every other window, including full-screen ones.
2. Be invisible to the taskbar, the dock, and the alt-tab switcher.
3. Have per-pixel transparency.
4. Optionally let clicks pass through to whatever is underneath.
5. Never steal keyboard focus.

Calling `winit` directly from the application would mean scattering raw Win32
and XCB calls through the event loop. This crate puts a small, explicit trait in
front of them instead, so the application asks for behaviour rather than
issuing platform calls.

## Layering

```
                      lumina-app
                           |
             +-------------+-------------+
             |                           |
      OverlayBackend              MonitorSource
        (win32 / x11 /              (win32 / x11 /
         null backend)             null backend)
             |                           |
             +-------------+-------------+
                           |
                   placement.rs
               (pure geometry, fully tested)
```

`placement.rs` is pure on purpose. It takes a monitor rectangle, a position, a
window size, and offsets, and returns a rectangle. No window handle, no I/O, no
clock, no global state.

That constraint is worth stating plainly: **placement is the one piece of window
logic users notice immediately when it is wrong.** A window that appears in the
wrong corner or straddles two monitors looks broken in a way that no amount of
smooth animation compensates for. Making it a pure function means every awkward
case can be written down as a test, and there are thirty of them.

## Backends

| Backend | Platforms | Status |
|---|---|---|
| `win32` | Windows 10 and 11 | Full: topmost, tool window, per-pixel alpha, click-through, acrylic, DPI v2, monitor enumeration |
| `x11` | Linux X11 | Full: dock window type, ARGB visual with a dedicated colormap, EWMH atoms, empty input region for click-through, XRandR monitors |
| `null` | Anywhere | Headless and test. Creates no window, reports every operation as a no-op |

Wayland layer-shell arrives in Phase 12. Until then a Wayland session runs
Lumina through XWayland, which is a working arrangement with documented limits
rather than a broken one.

## Error policy

Nothing in this crate is allowed to take the process down. Every platform call
that can fail returns [`OverlayError`], the backends log and carry on, and the
`null` backend exists so the whole application can run with no display at all,
which is what makes headless integration tests and the packaging smoke test
possible.