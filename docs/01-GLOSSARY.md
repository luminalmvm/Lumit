# Luminal glossary

**Status: canonical.** Every document, UI string, code identifier, and commit message in this
repository uses these terms exactly. If a concept is not named here, name it here first, then
use it. Terminology drift is a bug; file it like one.

Luminal is layer-based like After Effects, with one deliberate extension (the Sequence layer,
which brings Vegas-style cutting). Terms below note their AE/Vegas equivalents where that helps
someone arriving from those tools, but the Luminal term is the only one used in Luminal.

---

## 1. Project structure

| Term | Definition |
|---|---|
| **Project** | The complete editable document: assets, compositions, settings. Serialised as a `.lum` file (see [10-FILE-FORMAT.md](10-FILE-FORMAT.md)). Exactly one project is open at a time. |
| **Asset** | Anything importable that lives in the Project panel: footage items, audio items, image sequences, still images, and compositions themselves. |
| **Footage item** | An asset referencing a media file on disk (video, image, image sequence). Luminal never modifies the file; the project stores a reference plus interpretation settings (frame rate override, alpha interpretation, colour space tag). |
| **Audio item** | An asset referencing an audio file. |
| **Folder** | A grouping node in the Project panel. (AE calls this a folder too; Premiere calls it a bin — *bin* is not a Luminal term.) |
| **Composition (comp)** | A timeline with fixed resolution, frame rate, duration, and background colour, containing an ordered stack of layers. Comps can be nested via Precomp layers. |

## 2. Layers

A **layer** is one row in a composition's timeline. Layers stack: the bottom layer renders
first, each layer above composites over the result (full render order in
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)). Every layer has transform properties, an
in point and out point on the comp timeline, switches, and (where visual) masks, effects,
and a blend mode.

| Layer type | Definition |
|---|---|
| **Footage layer** | A layer whose source is exactly one footage item. Supports Retime (§4). The AE-style default. |
| **Retimeable layer** | Collective term for the layer kinds that carry their own Retime: Footage layers and Precomp layers. (Clips carry Retime individually inside Sequence layers.) |
| **Sequence layer** | A layer that contains an ordered run of **clips** cut back-to-back on its single row. This is Luminal's Vegas-style editing surface: each clip has its own source, trim, and Retime. Effects, masks, transforms, and switches on a Sequence layer apply to its whole output, after clip retiming. |
| **Precomp layer** | A layer whose source is another composition. The verb is **precompose**. Supports Retime (§4), as AE users expect of time-remapped precomps. |
| **Solid layer** | A layer of flat colour at a fixed size. |
| **Text layer** | Editable styled text. |
| **Shape layer** | Vector shape groups with fills, strokes, and path operations. |
| **Null layer** | An invisible transform-only layer used for parenting rigs. |
| **Adjustment layer** | An invisible layer whose effect stack is applied to the composite of everything below it. |
| **Audio layer** | A layer whose source is an audio item (or the audio channel of footage). Multiple audio layers per comp; see [09-AUDIO.md](09-AUDIO.md). |
| **Camera layer** | A 3D viewpoint. Only affects 3D layers. See 2.5D in [03-DATA-MODEL.md](03-DATA-MODEL.md). |
| **Light layer** | A 3D light source. Only affects 3D layers with lighting enabled. |

**Clip** — an entry inside a Sequence layer only: a reference to one source (footage item or
comp) plus a source in/out trim, a Retime, and per-clip render policies (frame interpolation
mode). Clips on the same Sequence layer never overlap; a cut between two clips is an **edit
point**. Ordinary layers do not contain clips.

**Anchor point** (a.k.a. the layer's **origin**) — the point, in the layer's own pixel
coordinates, that the transform pivots about: scale and rotation happen around it, and
**Position** places *it* in comp space. New layers default their anchor to the centre of
their content, so a fresh layer sits centred and pivots about its middle (the AE default).
The UI labels the two properties Anchor x / Anchor y.

**Parenting** — a layer may name another layer as its parent; transforms concatenate. No cycles.

**Switches** — per-layer toggles: visible, audible, solo, lock, shy, quality (draft/full),
motion blur, adjustment, 3D, collapse (for Precomp layers). Defined in
[03-DATA-MODEL.md](03-DATA-MODEL.md).

## 3. Animation

| Term | Definition |
|---|---|
| **Property** | A named animatable value on a layer, effect, mask, or clip (e.g. Position, Opacity, a blur radius). Properties nest in **property groups**. |
| **Keyframe** | A (time, value) anchor on a property, with per-side interpolation: hold, linear, or bezier with **speed** (units/second) and **influence** (percentage handle reach), matching AE's keyframe maths so imports are lossless. |
| **Curve** | The evaluated function of a property over time. |
| **Graph editor** | The panel that edits curves, with two views: the **value graph** (value against time) and the **speed graph** (first derivative against time). These are *views of the same data*, never separate data. |
| **Expression** | A per-property script (JavaScript) that computes the property's value each frame, optionally reading other properties. See [12-PLUGINS.md](12-PLUGINS.md) §scripting. |
| **Marker** | A labelled point (or span) on a comp, layer, or asset. **Beat markers** are markers generated by audio onset detection. |
| **Motion blur** | Per-layer shutter-simulated blur computed from motion, with comp-level shutter angle/phase settings. |

## 4. Time and retiming

Luminal has four timebases. Being explicit about which one a number lives in is mandatory in
code and docs (see [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md) on time types).

| Timebase | Definition |
|---|---|
| **Source time** | Seconds within a media file or nested comp, before any retiming. |
| **Clip time** | Seconds from the start of a clip, within a Sequence layer. |
| **Layer time** | Seconds from a layer's in point. |
| **Comp time** | Seconds from the start of a composition. |

**Retime** — Luminal's single retiming system: a per-layer (Footage and Precomp layers) or
per-clip (Sequence layer) mapping from layer/clip time to source time, stored as an ordered list of
**retime segments** (see [04-RETIMING.md](04-RETIMING.md)). Edited in the graph editor through
either the value graph (AE-style time remapping) or the speed graph (Vegas-style velocity).
There is **no separate "time remap" and "velocity" feature** — those are AE/Vegas names for the
two views of Retime.

| Term | Definition |
|---|---|
| **Speed** | The derivative of the retime map: 1.0 = normal, 0.0 = freeze, negative = reverse. The UI shows percentages (100%, 0%, −100%). The Retime graph channel *labels* its derivative lens **Velocity** and its value lens **Time** (K-076, Vegas/AE heritage); "speed" stays the term for the quantity itself. |
| **Freeze** | A retime region of speed 0. |
| **Overrun** | The state where a retime map requests source time beyond the media's end (or before its start). Luminal renders a hold of the boundary frame and marks the region visibly in the timeline. Overrun never moves clip boundaries or edit points. |
| **Frame interpolation** | How non-integer source frames are synthesised: **nearest** (duplicate), **blend** (crossfade), or **flow** (optical-flow synthesis). A per-clip/per-layer render policy, independent of the retime map itself. |
| **Stretch** | A layer-level uniform rate multiplier (AE's time stretch). Unlike Retime, stretch rescales the layer's keyframes. |

## 5. Rendering, preview, and export

These three words are **not interchangeable**.

| Term | Definition |
|---|---|
| **Render** | The engine producing pixels/samples for any purpose — preview or export. Internal act. |
| **Preview** | Interactive playback inside Luminal. Never writes user files. |
| **Export** | Writing a deliverable media file via the export queue. Export may **bake** (flatten retimes, rasterise, pre-composite) for speed; baking exists only inside the export pipeline and never alters the project. |
| **Evaluation graph** | The immutable DAG the layer stack compiles into for rendering. Users never see this term. |
| **Cache** | Stored intermediate frames, in three tiers: **VRAM cache**, **RAM cache**, **disk cache**. Cache entries are keyed by content hash, never by timeline position. |
| **Cache bar** | The timeline stripe showing which frames are cached (per tier). |
| **Proxy** | A lower-resolution/intermediate-codec stand-in for a footage item, generated in the background and toggled globally. |
| **Preview resolution** | Full / Half / Third / Quarter / Auto — true raster downsampling in the Viewer, per-comp. |
| **Adaptive degradation** | The engine's automatic quality reduction under load (resolution, skipped effects) during interaction only; it must never affect export. See [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md). |

## 6. Compositing

| Term | Definition |
|---|---|
| **Mask** | A bezier path on a layer that gates its alpha, with feather, expansion, opacity, and a combine mode. |
| **Matte** | Using another layer's alpha or luma to gate this layer. Any layer can be chosen as a matte from a dropdown (AE 2023-style); one matte layer can serve many layers. *Track matte* is the AE name; Luminal says **matte**. |
| **Blend mode** | Per-layer composite operator (Normal, Add, Screen, Multiply, Overlay, …). Full list in [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md). |
| **Effect** | One image (or audio) operation instance in a layer's **effect stack**, ordered top-to-bottom. Built-in effects, OFX plugins, and KFX plugins are all "effects" to the user. |
| **Working space** | The engine's internal pixel format: scene-linear, premultiplied alpha, fp16 (fp32 opt-in per comp). |

## 7. Interface

| Term | Definition |
|---|---|
| **Panel** | A dockable UI unit (Timeline, Viewer, Project, Effect Controls, Scopes, …). Full inventory in [07-UI-SPEC.md](07-UI-SPEC.md). |
| **Workspace** | A named, saveable arrangement of panels. Ships with presets (Edit, Effects, Colour, Audio); fully user-rearrangeable. |
| **Viewer** | The panel that displays a comp (or footage/layer) with its toolbar: preview resolution, magnification, channel view, transparency grid, guides, and wireframe toggles. |
| **Timeline** | The panel showing a comp's layer stack against time, with expandable property lanes, keyframes, and cache bars. |
| **Work area** | The comp-time span used for preview and default export range. |
| **Playhead** | The current-time indicator. *CTI* is not a Luminal term. |
| **Scopes** | Waveform, vectorscope, histogram panels (GPU-computed). |
| **Composer** | The planned audio workspace for sound design against the edit. See [09-AUDIO.md](09-AUDIO.md). |

## 8. Extensibility

| Term | Definition |
|---|---|
| **KFX** | Luminal's native plugin API: stable C ABI, sandboxed out-of-process execution. See [12-PLUGINS.md](12-PLUGINS.md). |
| **OFX** | The OpenFX standard; Luminal is an OFX host, which is how Twixtor, RSMB, Sapphire et al. run. |
| **Preset** | A saved, shareable configuration of effects/properties/animations, importable per layer. |

## 9. Words we do not use

| Banned term | Use instead | Why |
|---|---|---|
| **Track** / **line** | Layer, or Sequence layer | "Track" imports NLE semantics that don't match layer stacking; ambiguity here is exactly what this glossary exists to prevent. |
| **Velocity** | Speed (the quantity) | Reversed as the UI *label* for the Retime graph's derivative lens only (K-076); "speed" stays the word for the quantity everywhere else. |
| **Time remap(ping)** | Retime (value graph) | AE legacy name for one view of Retime. Acceptable in AE-import docs when describing AE itself. |
| **Bin** | Folder | Premiere-ism. |
| **CTI** | Playhead | |
| **Render** (meaning export) | Export | Render is the engine's act, not the user's. |
| **Event** | Clip | Vegas-ism. |
| **Pre-render** (user-facing) | Cache / bake | Reserved for internal cache warming. |
