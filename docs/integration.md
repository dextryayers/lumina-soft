# Hotkeys, tray, autostart, and single instance

## Hotkeys

Bindings live in `[hotkeys]`. The syntax is modifiers followed by one key,
separated by `+`, case insensitive, with modifiers in any order:

```toml
[hotkeys]
toggle_visibility = "ctrl+alt+l"
cycle_widget = "ctrl+alt+tab"
cycle_back = "ctrl+shift+tab"
rgb_next_mode = "ctrl+alt+r"
profile_next = "ctrl+alt+p"
```

Supported modifiers are `ctrl`, `alt`, `shift`, and `super`; `meta`, `win`,
`windows`, `cmd`, and `command` are aliases for `super`. Keys are letters,
digits, `f1` through `f24`, named keys such as `tab`, `enter`, `escape`,
`space`, `pageup`, `pagedown`, `home`, `end`, `insert`, `delete`, the arrow
keys, locks, `printscreen`, and `pause`, or the punctuation names `plus`,
`minus`, `comma`, `period`, `slash`, `backslash`, `semicolon`, `quote`,
`backquote`, `bracketleft`, `bracketright`, and `equal`. An empty string means
unbound. Two actions may not share a binding; the validator reports the
conflict instead of silently dropping one.

On Windows the backend uses `RegisterHotKey` on a dedicated message-loop
thread, including `MOD_NOREPEAT`. On X11 the binding is translated to
`XGrabKey` with Caps Lock, Num Lock, and Scroll Lock variants (X11 reports
Scroll Lock as `Mod3` or `Mod5` depending on the layout), so the combination
works regardless of lock state. Both backends live in `lumina-platform` and
are covered by tests.

Native Wayland sessions cannot grab global keys. The supported path is the
`org.freedesktop.portal.GlobalShortcuts` portal, which is not implemented yet.
`WaylandHotkeys::new` reports `HotkeyError::Unsupported` and the caller runs
without global hotkeys. The layer-shell window itself is implemented, so native
Wayland users get the island and IPC but not global key bindings. Under XWayland
the session still talks X11, so the X11 backend applies. macOS is not a target.

## Tray

The menu mirrors `[layout].order` and the RGB mode, plus fixed entries for
visibility, autostart, settings, documentation, about, and quit. Icons are
generated at runtime for four states: hidden, idle, active, and muted. Left
click toggles visibility; right click opens the menu.

The menu model (`TrayMenu`) and icon raster (`TrayIcon`) are implemented and
tested in `lumina-platform`. The native host is the part that needs the
application's message loop: Windows uses `Shell_NotifyIcon`, and Linux requires
a `StatusNotifierItem` host such as KDE, Xfce, or GNOME with the AppIndicator
extension. That host is wired in with the window in Phase 6; without one, the
same actions remain available through hotkeys and IPC.

## Autostart

```text
lumina --autostart status
lumina --autostart install
lumina --autostart remove
```

Windows writes the per-user value `Lumina` under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Linux writes
`~/.config/autostart/lumina.desktop` (or `$XDG_CONFIG_HOME/autostart`).
Both record the absolute executable path and add `--hidden` when the island
should start hidden, so autostart does not override an explicit preference.

## Single instance

The first launch owns a per-user named mutex on Windows or a unix socket on
Linux. A second launch writes a short activation message and exits with status
zero; the primary reads it with `InstanceGuard::take_activation` and shows the
island. A stale activation file from a crashed session is removed when the next
primary starts, and a stale unix socket is only reclaimed after confirming that
nobody is listening.
