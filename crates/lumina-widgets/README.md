# lumina-widgets

Widget framework for Lumina.

## What this crate provides

| Module | Responsibility |
|---|---|
| `widget` | The `Widget` trait, `WidgetContext`, `WidgetInput`, `WidgetEntry`, `DrawList`/`Quad`, and `WidgetRegistry` (visibility, layout, input routing, hover tracking) |
| `layout` | `FlexLayout`: the single-row flexbox that arranges widgets |
| `config` | `build_registry(&Config)`, which turns the full configuration into a ready registry |
| `clock` | `ClockWidget`: time, date, second timezone |
| `calendar` | `CalendarWidget`: month grid with today highlighted |
| `media` | Now-playing card with playback state |
| `spectrum` | Analyser bars driven by the audio snapshot |
| `metrics` | CPU, GPU, and memory readouts |
| `battery` | Charge level and charging state |
| `network` | Interface, throughput, and signal |
| `pomodoro` | Work/break timer with progress |
| `notify` | Notification cards with expiry and dismiss |
| `shortcuts` | User defined actions from `[[shortcuts.list]]` |
| `script` | Output of a user command with a hard timeout |
| `weather` | Current conditions from the configured provider |
| `snapshot` | Shared sample histories for sparklines |
| `paint` | Quad helpers used by every widget |

## Design rules

1. **The frame path allocates nothing.** Widgets emit quads into a
   pre-allocated `DrawList` that is cleared and refilled each frame.
2. **Layout is a pure function** of the available size and the widgets'
   desired sizes, so it is testable without a window.
3. **Input is routed by hit testing**, not by widget order, so a widget that
   is visually on top receives the events that land on it.
4. **One definition of visibility.** A widget is shown only when its
   `[widgets.<id>]` entry is enabled and it falls inside `max_widgets`; layout,
   drawing, input, and state requests all iterate that same visible set, so a
   widget can never receive input while it is not drawn.
5. **Per-widget overrides beat container defaults.** `flex`, `margin_leading`,
   and `enabled` from `[widgets.<id>]` replace the shared `[layout]` values;
   the validator clamps them so a typo cannot break the strip.
6. **Data widgets degrade visibly.** A missing battery, audio stream, network,
   or weather reading renders an explicit unavailable state instead of
   vanishing, which keeps the strip width predictable.
