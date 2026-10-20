# Themes

A Lumina theme is a small TOML file that overrides part of the configuration.
Themes are data, not code: they cannot run commands, read files, or change IPC
permissions. A theme that fails to parse is reported and skipped, so one broken
file never prevents Lumina from starting.

Themes follow the same layering rules as every other configuration source:

```text
compiled defaults
  -> config.toml
    -> theme
      -> profile
        -> command line
          -> session-only IPC changes
```

Later layers win. A theme only needs to contain the keys it changes; every other
value comes from the layer below it.

## Locations

Lumina discovers `.toml` files in the `themes` directory next to `config.toml`:

| Platform | Configuration file | Theme directory |
|---|---|---|
| Windows | `%APPDATA%\lumina\config.toml` | `%APPDATA%\lumina\themes\` |
| Linux | `$XDG_CONFIG_HOME/lumina/config.toml` or `~/.config/lumina/config.toml` | `.../lumina/themes/` |

Set `LUMINA_CONFIG` or pass `--config PATH` to use a different location. The
theme directory is always `themes/` beside the selected configuration file.
Built-in themes are available even when the directory does not exist.

Files are read in filename order, which keeps menu order stable. Theme names are
matched case-insensitively.

## Quick start

Export a built-in theme, edit it, and validate the result:

```sh
lumina --export-theme midnight --output midnight-custom.toml
lumina --check-theme midnight-custom.toml
```

List everything Lumina can see, including built-ins and local files:

```sh
lumina --list-themes
```

Copy the file into the theme directory and select it by its `name`, or through
the tray menu once the runtime orchestrator is wired.

## File format

```toml
name = "Midnight Blue"
extends = "midnight"

[appearance]
fill = "#0A1020F2"
border_color = "#4C6EF566"
corner_radius = 20.0

[rgb]
mode = "breathing"
primary = "#4C6EF5"
secondary = "#7048E8"
glow_intensity = 0.6

[text]
color = "#F8FAFC"
dim_color = "#94A3B8"

[accessibility]
enforce_contrast = true
contrast_target = 4.5
```

`name` is shown in the theme picker. If omitted, the filename stem is used.
`extends` is optional and copies the overrides of another theme first, then
applies this file's own keys. Only one parent is resolved; cycles are ignored.

Any configuration table can be overridden, including `appearance`, `rgb`,
`text`, `motion`, `accessibility`, `widgets`, `hotkeys`, and `profiles`. See the
generated [`configuration.md`](configuration.md) reference for every key and its
valid range.

Colors accept `#RRGGBB` or `#RRGGBBAA`. The alpha channel is significant: use
`FF` for opaque surfaces in high-contrast themes.

## Background images

The theme schema reserves an optional background image:

```toml
[background]
path = "C:/Users/me/Pictures/aurora.jpg"
blur_strength = 0.6
opacity = 0.25
mode = "cover"
```

`mode` is one of `cover`, `contain`, `tile`, or `stretch`. Image loading and
rendering are part of the runtime milestone; the fields are parsed and preserved
today but do not yet appear on screen. Prefer color-only themes until that
landing is announced.

## Built-in themes

| Name | File | Intent |
|---|---|---|
| `default` | none | No overrides; the base configuration |
| `midnight` | `midnight.toml` | Cool, low-contrast colors for night use |
| `neon` | `neon.toml` | Cyan and magenta, wide glow, cycling RGB |
| `sunset` | `sunset.toml` | Warm orange and amber with slow breathing |
| `mono` | `mono.toml` | Neutral greys with saturation removed |
| `glass` | `glass.toml` | Frosted fill with backdrop blur |
| `nord` | `nord.toml` | The Nord palette |
| `catppuccin` | `catppuccin.toml` | Catppuccin Mocha |
| `high-contrast` | `high-contrast.toml` | Opaque black, white text, 7.0 contrast, no glow |

The eight non-default themes are shipped as editable examples in
[`examples/themes/`](examples/themes/). Integration tests require each example
to match its built-in theme exactly and to apply cleanly to the default
configuration.

## Validation and safety

- `lumina --check-theme PATH` applies a theme to the selected base configuration
  and runs the full validator. It exits with `1` when the result has problems.
- `lumina --check-config` validates the current configuration, including an
  active theme, and suggests repairs for invalid keys.
- A malformed theme is reported by `--list-themes` and skipped at load time.
- Unknown theme files never block startup and never modify the base
  configuration.
- Contrast enforcement and reduced-motion settings are applied after theme
  overrides, so a theme cannot silently defeat accessibility guarantees.
- High-contrast mode is a normal theme: inspect it, extend it, or use it as a
  starting point for your own accessible palette.

## Accessibility checklist

When publishing a theme, verify that:

- Body text reaches the configured contrast target against `appearance.fill`.
- `dim_color` remains readable for secondary labels.
- Status is not conveyed by color alone; icons and text remain present.
- Motion is subtle, and `motion.reduced = true` still looks correct.
- The theme remains legible with `backdrop_blur = "on"` and with transparency
  disabled.
- `lumina --check-theme` reports no problems on both a default base and a
  high-contrast base.

See [`accessibility.md`](accessibility.md) for the full contrast and focus model.
