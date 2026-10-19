# Script widgets

Script widgets run a command on a fixed interval and show its most recent
output inside the island. They are off by default because a config file can
execute programs.

## Configuration

```toml
[script]
enabled = true

[[script.entries]]
id = "uptime"
label = "Uptime"
command = "uptime"
args = ["-p"]
interval_ms = 60000
timeout_ms = 5000
display_ms = 8000
max_lines = 3
display = "text"
show_in_island = true

[[script.entries]]
id = "vpn"
label = "VPN"
command = "wg"
args = ["show", "wg0"]
display = "icon"
icon = "shield"
interval_ms = 30000
```

The widget appears when `script` is listed in `[layout].order`:

```toml
[layout]
order = ["clock", "media", "script"]
```

Each runnable entry becomes one tile, in declaration order. Entries with
`show_in_island = false` are validated and kept, but not drawn; this lets a
future tray or IPC action reuse them. The validator drops entries with an empty
`id` or `command`, renames duplicate ids, fills a missing label from the id, and
clamps `interval_ms` to 250 through 86,400,000, `timeout_ms` to 1,000 through
120,000, `display_ms` to 500 through 60,000, and `max_lines` to 1 through 8.

## Execution model and security

- Commands are executed **directly**, never through a shell. `command` is the
  program name or an absolute path; `args` is a separate array. There is no
  `sh -c`, no `cmd /C`, no glob expansion, and no environment variable
  substitution. A config author cannot accidentally turn a label into a shell
  injection.
- `cwd` is optional. An empty value inherits Lumina's working directory.
- Standard input is closed. Standard output and standard error are captured on
  dedicated reader threads, so a chatty command cannot block the UI.
- A run is killed when `timeout_ms` elapses. On Windows the child is terminated;
  on Unix it receives `SIGKILL` through the standard library. A timed-out run is
  shown as `TimedOut` until the next scheduled run.
- Output is bounded while reading: lines are truncated to 120 characters
  including the ellipsis, and only the newest eight lines are retained. A
  command that prints gigabytes costs the same as one that prints a screen.
- Only one run per entry is active at a time. A command that is still running
  when the next interval arrives is not started twice.
- Scripts do not request new island states. While output is fresh they request
  the existing `Notify` state, the same low-risk path notifications use.

Running programs from a config file is a real capability. Keep the config file
owner-writable, review third-party configs before enabling `[script]`, and
prefer fixed absolute paths for anything that runs with elevated privileges.
Lumina does not elevate, does not run scripts as another user, and never
downloads commands.

## Display

`display = "text"` shows the newest lines with the label above them.
`display = "icon"` shows the configured icon and a one-line status. Clicking a
tile starts it immediately; right-clicking hides the output until the next run.
An empty command, a missing program, and a non-zero exit code are all rendered
as an explicit failure state rather than silently disappearing.

## Failure behavior

| Situation | What the user sees |
|---|---|
| Program not found | `Failed` with the operating system message |
| Non-zero exit | `Ok(code)` color and the final output line |
| Timeout | `TimedOut` after the child is killed |
| No output yet | The label and a muted waiting line |
| Invalid entry | The validator's report; the entry is not registered |
