# lumina-text

MSDF text and icon rendering for Lumina.

## What this crate is for

Text is rendered as a multi-channel signed distance field. The glyph outline is
stored once, at a small size, as three channels encoding the distance to the
edge. At render time the fragment shader reconstructs the distance and
produces a coverage value, which gives crisp text at any scale from a single
atlas with no per-size rasterization.

## Why MSDF rather than SDF

A plain signed distance field blurs corners because a single channel cannot
represent a corner exactly: the distance field of a corner is rounded. MSDF
stores three independent channels, and the median of the three reconstructs
the corner correctly. The difference is visible at small sizes and at oblique
angles, which is exactly where Lumina's text lives.

## The atlas

A 1024 x 1024 RGB8 atlas holds the bundled font. It is embedded at compile
time with `include_bytes!`, so there is no runtime generation cost and no heap
allocation at startup. Custom fonts use a runtime fallback path that is slower
but only triggers when the user supplies their own font.

## Module layout

| Module | Responsibility |
|---|---|
| `msdf` | WGSL shader source, `naga` validation, CPU coverage reference |
| `atlas` | Glyph atlas storage, UV lookup, embedded bytes |
| `shaping` | Grapheme segmentation, glyph mapping, kerning, RTL detection |
| `layout` | Single-row text layout, alignment, overflow, marquee |