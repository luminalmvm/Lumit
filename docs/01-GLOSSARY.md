# Glossary

Every identifier, comment, UI string, commit and doc uses these terms. A new concept gets
its entry here first. AE and Vegas names are noted only to help people arriving from them.

## 1. Project structure

| Term | Meaning |
|---|---|
| **Project** | The whole document, saved as a `.lum` file. One open at a time |
| **Asset** | Anything in the Project panel: footage, audio, sequences, stills, comps |
| **Footage item** | An asset pointing at a media file. Lumit never changes the file |
| **Source layer** | One layer of a layered image file such as a PSD or an Illustrator document, which a footage item can read on its own |
| **Audio item** | An asset pointing at an audio file |
| **Packed project** | A `.lum` that also carries its footage files inside it. The verbs are **pack** and **unpack** |
| **Shared project** | A project several people edit at once. The **host** shares it and owns the file, the **guests** join with an **invite**, which is written as an **invite link** |
| **Relay** | Somebody's own server that passes a shared project's edits between a host and guests who cannot reach each other. It cannot read them |
| **Folder** | A group in the Project panel. Not *bin* |
| **Composition (comp)** | Resolution, frame rate, duration, background, and a layer stack or a node graph |
| **Node graph** | A comp whose picture is made by nodes and wires instead of layers |

## 2. Layers

A **layer** is one row in a comp. The bottom layer draws first and each one above
composites over it.

| Layer | Meaning |
|---|---|
| **Footage layer** | One footage item. Has Retime |
| **Sequence layer** | Clips cut back to back on one row, each with its own source, trim and Retime. The layer's own effects apply to the whole output |
| **Precomp layer** | Another comp as a source. The verb is **precompose**. Has Retime |
| **Solid**, **Text**, **Shape**, **Null** layers | Flat colour, styled text, vector shapes, a transform-only rig |
| **Adjustment layer** | Applies its effects to everything below. Also a switch any drawing layer can carry |
| **Layer group** | A header row folding layers together in the Timeline. Organisation only, the render never reads it |
| **Audio layer** | An audio item, or footage's audio |
| **Camera**, **Light** layers | 3D viewpoint and light. Only affect 3D layers |

- **Clip**: an entry inside a Sequence layer, and only there. Clips on a picture layer
  never overlap. On an audio-only Sequence layer they may, and the overlap is a crossfade.
  A cut between clips is an **edit point**.
- **Anchor point**: the point the transform pivots around. Position places it.
- **Parenting**: a layer can follow another layer's transform. No cycles.
- **Switches**: per-layer toggles. Visible, audible, solo, lock, shy, quality, motion blur,
  adjustment, 3D, collapse.

## 3. Animation

| Term | Meaning |
|---|---|
| **Property** | A named animatable value. Properties nest in **property groups** |
| **Keyframe** | A time and value, with hold, linear or bezier (**speed** and **influence**, AE's maths) |
| **Graph editor** | Edits curves as a **value graph** or a **speed graph**. Two views of the same data |
| **Expression** | A per-property script that computes the value each frame. Written in **JavaScript** or **Rhai**, which the expression's own language choice says |
| **Custom controls** | An effect holding a named set of controls for expressions to read. What an After Effects pseudo effect imports as |
| **Marker** | A labelled point or span. **Beat markers** come from audio onset detection |
| **Motion blur** | Three things: the layer switch, the **Motion blur** effect (optical flow), and **Accumulation motion blur** (re-renders and averages) |

## 4. Time and Retime

Four timebases, and code always says which one a number is in: **source time**, **clip
time**, **layer time**, **comp time**.

**Retime** is the one retiming system: a map from layer or clip time to source time, made
of segments. The value graph is AE's time remapping, the speed graph is Vegas' velocity.
They are two views, not two features.

| Term | Meaning |
|---|---|
| **Speed** | The slope of the Retime map. 100% normal, 0% freeze, negative reverse |
| **Freeze** | A stretch at speed 0 |
| **Overrun** | Retime asking past the media's ends. Holds the end frame, marked in the Timeline, never moves edit points |
| **Frame interpolation** | Nearest, blend or flow, for in-between source frames |
| **Stretch** | A command that rewrites the Retime map for a new speed. Not a stored multiplier |

## 5. Render, preview, export

Not interchangeable.

| Term | Meaning |
|---|---|
| **Render** | The engine making pixels, for anything |
| **Preview** | Playback inside Lumit. Never writes user files |
| **Export** | Writing a media file. May **bake**, which never changes the project |
| **Evaluation graph** | What the layer stack compiles into. Users never see the term |
| **Cache** | Frames stored in VRAM, RAM and disk tiers, keyed by content hash |
| **Proxy** | A smaller stand-in for footage |
| **Preview resolution** | Full, Half, Third, Quarter, Auto |
| **Adaptive degradation** | Lowering preview quality under load. Never touches export |

## 6. Compositing

| Term | Meaning |
|---|---|
| **Mask** | A bezier path gating a layer's alpha |
| **Matte** | Another layer's alpha or luma gating this one. Not *track matte* |
| **Roto brush** | Builds a matte from painted strokes, carried by optical flow. **Refine edge** is its soft boundary |
| **Blend mode** | How a layer composites over what's below |
| **Effect** | One operation in a layer's **effect stack**, or one box in a node graph |
| **Driver** | A node that makes a value rather than a picture and drives a parameter through a wire |
| **Read**, **Input**, **Output** nodes | Bring an item in, take a value or picture from outside, the one picture the graph shows |
| **Merge**, **Switch**, **Time offset** nodes | A over B, pick one input, show the input at another time |
| **Split channels**, **Combine channels** | Picture into four greyscale pictures, and back |
| **Node graph effect** | Applies a node graph to a layer |
| **Wire**, **port** | A connection on the Graph panel, and the typed socket it plugs into |
| **Points stream** | Per-frame particle data an effect produces. Never stored |
| **Working space** | Scene-linear, premultiplied, fp16 (fp32 opt-in) |
| **OCIO config** | An OpenColorIO config a project can name |

## 7. Interface

| Term | Meaning |
|---|---|
| **Panel** | A dockable piece of UI |
| **Workspace** | A saved panel layout |
| **Graph panel** | Draws a layer's effects as nodes and wires. Not the evaluation graph |
| **Viewer** | Shows a comp, footage or layer. Holds one, two or four **views** |
| **Timeline** | A comp's layers against time |
| **Work area** | The span used for preview and default export |
| **Playhead** | The current time. Not *CTI* |
| **Scopes** | Waveform, vectorscope, histogram |
| **Easing panel** | One curve shape applied to selected keyframe spans |
| **Text panel** | Sets a text layer's font, size, spacing and outline |
| **Paragraph panel** | Sets how a text layer's lines are aligned and spaced |
| **Expressions panel** | Keeps saved expressions and puts one on the selected properties |
| **Stack** (of panels) | A panel group drawn as twirled panels one above another, not as tabs |
| **Flowchart** | A comp drawn between the comps that place it and the comps it places |

## 8. Extensibility

| Term | Meaning |
|---|---|
| **OFX** | OpenFX. Lumit hosts it |
| **LFX** | Lumit's own planned plugin API |
| **CLAP**, **VST3** | Audio plugin standards Lumit hosts. No VST2 |
| **Addon** | An optional download from Settings: the model runtime and model packs. Analysis only, never generation |
| **Model pack** | One addon holding one model |
| **Preset** | Saved effects, properties or animation |

## 9. Words we don't use

| Don't say | Say |
|---|---|
| Track, line (a timeline row) | Layer, Sequence layer |
| Velocity (the quantity) | Speed |
| Time remap | Retime |
| Bin | Folder |
| CTI | Playhead |
| Render (meaning export) | Export |
| Event | Clip |
| Pre-render (user-facing) | Cache, bake |

- **To track** something through a shot is the trade's verb and stays. A track is one
  followed feature, never a timeline row.
- **To clip** in keying and colour (clip black, clipped highlights) stays.
- The Audio timeline panel calls its rows tracks, in its own strings and files only.
  Everywhere else it's a Sequence layer with `audio_only` set.
- The Retime graph labels its lenses Time and Velocity. Speed stays the word for the
  quantity.
- **Switch** means both the node and a layer's toggles.
