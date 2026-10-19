# lumina-config

Typed configuration for Lumina, with per-key validation and hot reload.

## The rule this crate exists to enforce

**One wrong value must never cost a user their whole configuration.**

That single requirement drives the entire design. If a user mistypes a color or
types a number out of range, they get that one key corrected to a sane default,
a warning that names the key and the problem, and everything else they wrote
kept exactly as they wrote it.

The alternative, which is what a plain `serde` deserialize gives you, is
all-or-nothing: one bad byte anywhere and every setting reverts. For a program
whose entire promise is "fully customizable", that is an unacceptable failure
mode.

## How it works

| Stage | Module | Responsibility |
|---|---|---|
| 1. Parse | `toml` | Turn bytes into a `toml::Value` tree. A syntax error here is the only genuinely fatal case. |
| 2. Shape | `schema` | Deserialize the tree into typed structs, section by section. A failure in one section leaves the others intact. |
| 3. Validate | `validate` | Walk every key against a range, enum, or type rule. Produce a list of `Issue` values. |
| 4. Repair | `validate` | Apply the documented fallback for each issue. |
| 5. Report | `validate` | Return a `ValidationReport` the caller logs and the `--check-config` command prints. |

## Unknown keys

An unknown key is a warning, not an error. Users type `rgb.glow` when the key is
`rgb.glow_intensity`, and telling them the nearest valid spelling is more useful
than silently ignoring them. Suggestions use edit distance with a threshold of
three, which catches transpositions and abbreviations without inventing matches.

## Cross-field rules

Some values are only wrong in combination. `half_bottom_width` wider than
`half_top_width` is fine on its own and an inverted island together. Cross-field
rules run after per-key validation and are reported at the same severity, so a
user sees all the problems in their file in one pass rather than fixing them one
run at a time.

## Widgets, scripts, and hotkeys

`[widgets.<id>]` overrides the shared `[layout]` defaults per widget: `enabled`,
`flex`, `margin_leading`, and `rgb`. Unknown ids are dropped with a warning and
out-of-range values are clamped, so a typo cannot hide a widget or break the
strip. `[script]` entries are validated with the same philosophy: the command is
split without a shell, lines are capped at eight and 120 columns, and a bad
entry is disabled rather than failing the whole file. `[hotkeys]` strings are
parsed, normalized, and checked for duplicates; an unparseable binding is
cleared and reported, and two actions may not silently share a combination.

## Hot reload

`watch` wraps `notify` with a 150 ms debounce, because every editor writes files
in bursts and reloading on each event would re-parse the same file three times.
After a reload the effective config is diffed against the previous one and only
the changed keys are dispatched, so a reload that changes nothing touches
nothing and does not wake the render loop.

Keys that genuinely cannot be applied live are marked in the schema and reported
by `--check-config` rather than being silently ignored at runtime.