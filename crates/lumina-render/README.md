# lumina-render

GPU rendering for Lumina.

## What this crate is for

The island is not a widget tree. It is a single signed distance field evaluated
per pixel in a fragment shader, driven by a handful of scalars that a spring
solves toward a target. That makes the correctness of the shader the single most
important rendering decision: if the SDF is wrong, the shape is subtly incorrect
in ways nobody can name but everybody can see.

## The three passes

| Pass | What it draws | Cost |
|---|---|---|
| Island body | SDF fill, border, glow in one fragment shader | One full-target quad |
| Content | Album art, icons, text, progress, controls | Instanced quads |
| Spectrum | Bars or waveform, 32 bands | One instanced draw |

## The dirty flag rule

The render loop does not run at 60 fps forever. It runs only while something is
animating, audio is flowing, or a widget is updating. When nothing is new the
loop blocks instead of spinning. This is the single most important rule for the
CPU budget: a process that submits 60 frames a second forever will sit at a few
percent of a core no matter how cheap each frame is.

## Shader validation

Every WGSL source is validated at startup through `naga` before it reaches the
driver. A shader that fails validation is caught, logged, and replaced with a
solid fill fallback rather than taking the application down with it. This means
a broken shader is caught in CI on a machine with no graphics hardware at all,
which is the only way to test shaders without a GPU in the loop.

## Error scopes

`wgpu` supports scoped error handling, which is how a pipeline creation failure
is caught. Without it, a validation error on one driver would surface as an
uncatchable device loss. Every pipeline creation is wrapped in
`push_error_scope(Validation)` and the fallback shader is used when validation
rejects the primary one.

## Module layout

| Module | Responsibility |
|---|---|
| `shaders` | WGSL sources and `naga` validation |
| `uniforms` | The `IslandUniforms` struct and its `wgpu` buffer layout |
| `device` | Adapter selection, surface configuration, GPU info |
| `pipeline` | Shader compilation, render pipeline, error scopes |
| `render` | The frame loop, dirty flag, frame statistics |