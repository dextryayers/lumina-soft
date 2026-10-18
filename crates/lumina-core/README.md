# lumina-core

Shared mathematical and state primitives for Lumina.

This crate sits at the bottom of the workspace dependency graph. It depends on
`serde` and no other internal crate, so every other crate can depend on it
without risking a cycle.

## Why the mathematics lives here

The island is not a widget tree. It is a single signed distance field evaluated
per pixel in a fragment shader, driven by a handful of scalars that a spring
solves toward a target. That makes the correctness of the geometry math, the
spring integrator, and the color conversions the single most important thing in
the project. If the trapezoid distance field is wrong, the shape is subtly
incorrect in ways nobody can name but everybody can see. If the spring
integrator is unstable, the island shakes under load. If the color pipeline
interpolates in sRGB, every transition goes muddy in the middle.

So these primitives live in one place, with no I/O, no platform access, and tests
that check against independent ground truth rather than against themselves.

## Modules

| Module | Contents |
|---|---|
| `geometry` | `Size`, `Vec2`, `Rect`, `Insets`, `Geometry`, and the clamping rules that make the inverted trapezoid invariant unbreakable |
| `sdf` | Trapezoid and rounded trapezoid distance fields, verified against brute force |
| `spring` | Damped harmonic oscillator with fixed step integration and retargeting |
| `color` | sRGB to linear to OkLab, palette mixing, WCAG contrast |
| `state` | Island states, the priority based transition arbiter, normalized events |
| `time` | Monotonic clock, fixed step accumulator, frame pacer |

## Design rules

1. **No allocation in this crate's hot paths.** `Vec2`, `Rect`, `Geometry`, and
   `Spring` are all fixed size and `Copy`.
2. **No panics on user input.** Every geometry value entering the render path
   passes through a clamp, so a bad configuration produces a wrong looking
   island rather than a crash or a screen filling rectangle.
3. **Tests check against ground truth.** The distance field is compared to a
   brute force computation over a grid, not against a second implementation of
   the same formula. The spring is checked for settling and for continuity under
   retargeting, not for producing a specific number that the implementation
   happens to produce.
4. **`f32` throughout.** These values are consumed by a shader. `f64` here would
   buy precision nobody can see and cost conversions nobody needs.