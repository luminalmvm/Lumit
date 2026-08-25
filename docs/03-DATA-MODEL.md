# Lumit data model

**Status: canonical.** The object model every other document builds on. Terminology per
[01-GLOSSARY.md](01-GLOSSARY.md); decisions per [02-DECISIONS.md](02-DECISIONS.md).
Serialisation of this model is specified in [10-FILE-FORMAT.md](10-FILE-FORMAT.md); how it
compiles into the evaluation graph is specified in [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md).

Sketches below use Rust-flavoured pseudocode. They describe shape and invariants, not final
field names.

---

## 1. Foundations

### 1.1 Identity

Every model object carries a stable **UUIDv7 id**, assigned at creation and never reused.
All cross-references (layer parenting, mattes, clip sources, expression links) are by id.
Names are display strings only; renaming MUST never break a reference.

### 1.2 Time is rational

Authoritative time is never floating point: it is an exact rational number of seconds, with
a rational `FrameRate` beside it (the rule is [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)
§2; the type and its arithmetic are [impl/rational-time.md](impl/rational-time.md)).

The four timebases - `SourceTime`, `ClipTime`, `LayerTime`, `CompTime`
([01-GLOSSARY.md](01-GLOSSARY.md) §4) - are distinct newtypes over `Rational`. Conversions
between them are explicit functions, and the Retime map ([04-RETIMING.md](04-RETIMING.md)) is
the only nontrivial conversion.

### 1.3 Non-destructive rule

Per K-024: no operation on this model modifies source media, and no operation is
irreversible within a session (everything goes through the operation journal, §10).
Baking exists only inside the export pipeline and produces no model mutations.

---

## 2. Project

v1 ships this as lumit-core's `Document` — a flat item store. The richer
`Project`/`ProjectSettings` shape is the intended direction; the settings it
would hold (a display-transform default, an expression-engine version) arrive
with those features (colour management, expressions), so v1 has no
`ProjectSettings` yet.

```rust
struct Document {
    id: Uuid,
    items: Vec<ProjectItem>,   // flat storage; Project-panel order = Vec order; folders hold children by id
    auto_folders: AutoFolders, // where new solids / comps are auto-filed (K-068)
    anti_aliasing: AntiAliasing,           // coverage samples per pixel: Off/X2/X4/X8, default X4 (K-274)
    cache_location: Option<CacheLocation>, // this project's own frame-cache folder (K-215)
    ui_state: Option<serde_json::Value>,   // how the interface was arranged, opaque (K-245)
}
```

`anti_aliasing` is how hard the renderer works at the edges of transformed layers (K-274,
[impl/anti-aliasing.md](impl/anti-aliasing.md)). It is a **project** property rather than a
preference, and that is the decision, not an implementation detail: it changes what a comp
looks like, so it must travel in the `.lum` and match when the file is opened on another
machine. **One value serves preview and export** — a preview that anti-aliased differently
from the file would break the K-031 preview-equals-export identity that the whole render path
is built around. Default `X8`: on, eight coverage samples per pixel (K-286), falling back to four on a card
that will not give eight. Set through an ordinary
op, so it is undoable and journalled like any other change to the picture, and — unlike
`cache_location` — it *does* change pixels, so the sample count is part of a frame's content
hash (docs/06 §5.2) and a frame banked at one setting is never served at another.

What a given graphics card will actually do is a separate question from what the project asks
for. The count is asked of the adapter and never assumed; one that cannot manage the count
falls back to the highest it will, down to off, and the interface says which is in use. That is
a fact about the machine, never an error and never a rewrite of the project.

`cache_location` is the one piece of *machine* preference the document carries, and it is here on
purpose (K-215, docs/06 §5.4): where a project's rendered frames are parked belongs to the
project — a scratch drive it lives on, or beside itself so the cache travels with a copy — and a
setting held in one machine's settings file could not travel with it. `None`, the usual case,
means the project follows the application-wide choice. It is set through an ordinary op, so it is
undoable and journalled like any other change, and it changes no pixel: cache entries are named
by content, and where they are kept is not part of that name.

`ui_state` is the other field that is not the work itself: how the interface was arranged for
this project (K-245) — the panel layout, the open comp tabs, the playhead, the selection — as
whatever JSON the frontend wrote. **The engine never reads inside it.** It is carried so that a
project handed to somebody else opens the way its author left it; the shape belongs to the
frontend, and an engine that understood it would have to change every time a panel gained a
setting. Unlike `cache_location` it is *not* an op: recording it is not undoable, not
journalled, and does not move the store's revision, because moving a panel is not an edit to the
work (`DocumentStore::set_ui_state`). The frontend writes it just before a save.

A `ProjectItem` (the intended **Asset**) is one of the following. **v1 ships
`Footage`, `Folder`, `Composition`, `Solid`**; the audio/still/sequence kinds
are future (audio is currently only a footage layer's stream, §5.2):

| Asset | v1? | Contents |
|---|---|---|
| `Footage` (`FootageItem`) | yes | Media reference (§3); interpretation and proxy state are future |
| `AudioItem` | future | Audio-only media reference |
| `StillItem` | future | Single image |
| `SequenceItem` | future | Image sequence (pattern, fps) |
| `Composition` | yes | §4 |
| `Folder` | yes | Ordered children ids |
| `Solid` (`SolidDef`) | yes | Shared solid definition (colour, size) — solids are items so they dedupe |

### 3. Media references and interpretation

```rust
struct MediaRef {
    relative_path: String,     // what the FILE stores: rebased on save, / slashes (K-173)
    absolute_path: String,     // session-state: where the file is on THIS machine —
                               // never serialized (it embeds the username; K-173)
    fingerprint: Option<Fingerprint>, // stamped on save; drives relink step 3 (10 §2)
}
```

**Future** — not in v1 yet:

- a `FootageInterpretation` (frame-rate override, alpha mode, colour-space tag, loop count,
  timecode policy) — v1 treats every source as sRGB with no per-item overrides.
(The **missing**-footage state is built: the automatic resolver runs on open, a lost file
gets a project badge and a generated colour-bar slate in comps, and *Relink…* re-points it
along with its siblings — [07-UI-SPEC.md](07-UI-SPEC.md) §3.3.)

### 3a. Proxies (K-501)

A **proxy** is a low-resolution stand-in for a footage item: the file the Viewer decodes
while you work, with the full-resolution original swapped back in for delivery. It is a
*second* media reference beside the item's own, never a replacement.

```rust
struct ProxyRef {
    media: MediaRef,   // resolved, relinked and fingerprinted exactly like the original
    enabled: bool,     // this item's own "use proxy" switch (default on)
}

// on Document:
proxies: BTreeMap<Uuid, ProxyRef>,  // by footage item id; absent when empty
use_proxies: bool,                  // the project-wide master switch (default on)
```

Stored as a map beside the items rather than as a field on `FootageItem`, for the reason
`item_labels` is (§2): only one of the four kinds of project item can carry one, almost no
item does, and an entry that outlives a deleted item is what makes undoing the delete bring
the proxy back with it. Both fields are absent from a `.lum` that has nothing to say about
proxies, so every file written before them round-trips byte for byte.

Three rules decide what a proxy may do, and all three exist so that a small picture can
never be mistaken for a large one:

- **The original's numbers still govern.** Size, rate and duration are the original's
  whatever file the pixels come out of, so px@comp (K-419) stays the original's raster and
  no transform, mask or effect parameter changes meaning when a proxy is switched on. A
  proxy is simply decoded at fewer pixels — the same thing preview resolution already does.
- **A proxy that disagrees about the footage is refused.** A different frame count or a
  different rate means frame 300 of the stand-in is not frame 300 of the original; that
  proxy falls back to the original, as do one that is missing, unreadable or not yet
  probed. A missing *original* still shows the colour-bar slate whatever proxy is attached
  — the layer is the original, and a lost clip must lead to the relink.
- **Delivery reads the originals.** The export's own override is off by default whatever
  the project is set to ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §5.7).

Proxies are local working files: **collect for sharing drops them**, so a shared copy
carries the media it delivers and nothing else.

---

## 4. Composition

```rust
struct Composition {
    id: Uuid,
    name: String,
    width: u32, height: u32,            // no hard cap enforced yet (16384² is the intended limit)
    frame_rate: FrameRate,
    duration: CompTime,
    background: LinearColour,
    motion_blur: MotionBlur,            // shutter_angle (deg), shutter_phase, samples; off by default
    work_area: Option<(CompTime, CompTime)>,  // None = full comp
    markers: Vec<Marker>,
    layers: Vec<Layer>,                 // index 0 = top of the stack
}
// Future: `pixel_aspect` (v1 is square-pixel only), and working depth — K-069
// made bit depth a project-wide switch (not the per-comp `CompDepth` of the
// superseded K-026), and v1 renders fp16 only regardless. The 16384² dimension
// cap is intended but not yet enforced.
```

Comp frame rate is presentational (it defines frame boundaries for snapping and export);
evaluation is defined at arbitrary rational times so nested comps of differing rates stay exact.

---

## 5. Layers

### 5.1 Common layer core

Every layer, regardless of type:

```rust
struct Layer {
    id: Uuid,
    name: String,                      // defaults from source; user-renameable
    kind: LayerKind,                   // one of §5.2
    in_point: CompTime,                // may be negative — the layer may start before comp 0 (K-153)
    out_point: CompTime,               // exclusive; out > in; may exceed the comp duration (K-153)
    start_offset: CompTime,            // where layer time 0 sits on the comp timeline; may be negative
    parent: Option<Uuid>,              // transform parenting (K-103); a missing/cyclic parent degrades to none
    label: u8,                         // index into the theme label palette (TL2); organisational, never rendered
    blend: BlendMode,
    matte: Option<MatteRef>,           // { layer, channel: Alpha|Luma, inverted, source } (K-142)
    transform: TransformGroup,         // §6
    masks: Vec<Mask>,                  // §7
    paint: Vec<PaintStroke>,           // §7.1, stamped before masks
    effects: Vec<EffectInstance>,      // §8, ordered top-to-bottom
    volume_db: Property,               // K-172: animatable Volume (docs/09 §6); 0 dB unity, −100 = −∞
    audio_only: bool,                  // K-435: sound, no picture — an Audio layer (§5.7). Default false.
    retime: Option<Property>,          // K-197: Retime as an ordinary keyframable property —
                                       // layer-local time → source time, in seconds. None = not
                                       // retimed (no row, no map). Ctrl+Alt+T installs the identity.
    markers: Vec<Marker>,              // §11, K-254: the layer's OWN cues, drawn on its bar.
                                       // Times are layer-local. A comp dropped into another
                                       // brings a copy of its markers here; the two lists are
                                       // unrelated from then on.
    graph: LayerGraph,                 // K-471: the layer's driver graph (§8.1) — additive
                                       // wiring beside the effect stack. Empty by default,
                                       // absent from the file when empty.
    switches: Switches,
}
// Future (not in v1): `stretch` (uniform rate multiplier).
// Mute stays the `audible` switch, and audio comes only from a footage layer's own
// stream (§5.2, docs/09); the once-sketched `audio: AudioProps` grouping collapsed
// to the single `volume_db` property when it shipped (K-172) — fades are its
// keyframes, so v1 needed nothing more.

struct Switches {
    visible: bool, audible: bool, locked: bool,
    solo: bool,                        // K-105: while any layer is soloed, only soloed layers render
    fx: bool,                          // docs/08 §1.5: off bypasses the layer's whole effect stack (default on)
    motion_blur: bool,                 // K-120: per-layer shutter smear (needs the comp master on)
    three_d: bool,                     // 2.5D: position in z, honour the active camera
    collapse: bool,                    // Precomp layers: transform concatenation (docs/06 §1.4)
    shy: bool,                         // docs/07 §4.2: hidden from the layer list; never changes pixels
    guide: bool,                       // K-497: reference only — the Viewer draws it, no file carries it
    accepts_lights: bool,              // K-361: the comp's Light layers shade this one (default on)
}
// Future switches (K-168, deferred): `quality` (Draft|Full — needs a bicubic
// sampler choice). `adjustment` is not a
// switch — an adjustment layer is a LayerKind (§5.2).
```

Invariants:
- A layer sits freely across the comp boundaries (K-153): `in_point` may be **negative**
  (the layer starts before comp time 0) and `out_point` may exceed the comp **duration**.
  Only `out > in` is enforced. The engine renders and plays a layer solely where its span
  `[in_point, out_point)` **intersects the comp window `[0, comp_end)`** — frames outside the
  window are simply never sampled — so an over-hanging head or tail is carried without data
  loss and is recoverable by sliding the layer. Import never trims a long clip to fit: a
  footage/precomp layer keeps its full source/nested duration, positioned from the comp start.
- A matte reference to a missing/deleted layer degrades to "no matte" with a badge, never an error.
- Any layer can serve as a matte for any number of consumers; the engine evaluates it once
  ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)).
- `source: LayerInputSource` (default `EffectsAndMasks`, K-142, revising K-125's `after_effects`
  bool — the most complete source is the sensible default for a new matte/depth input):
  **None** gates by the matte layer's **raw** pixels (no masks, no effects); **Masks** gates
  by the source plus its own masks; **EffectsAndMasks** runs the matte layer's effect stack
  into the matte first (a keyed or blurred matte). v1 skips the source's *temporal* effects
  through a matte (echo/flow degrade to a still — [docs/impl/layer-input.md](impl/layer-input.md)).
  A project saved with K-125's `after_effects` bool migrates on load (`true` →
  `EffectsAndMasks`, `false` → `Masks` so no masks are dropped, absent → the default
  `EffectsAndMasks`).

### 5.2 Layer kinds

| Kind | v1? | Source payload | Notes |
|---|---|---|---|
| `Footage { item: Uuid, retime: Option<Retime> }` | yes | One footage item | The AE-style default. `None` = source rate. Retime per [04-RETIMING.md](04-RETIMING.md). |
| `Sequence { clips: Vec<Clip> }` | yes | Its clips | §5.3. |
| `Precomp { comp: Uuid }` | yes | Another composition | `collapse` switch defers rasterisation. Cycles invalid. **Precomp-level retime is future** — the `retime` field is not on the kind yet; nest through a Sequence clip to retime a comp for now. |
| `Solid { def: Uuid }` | yes | A SolidDef | |
| `Text { document: TextDocument }` | yes | §9.1 | v1: one run. |
| `Camera { zoom: Property, solve_link: Option<Uuid> }` | yes | — | AE camera: `zoom` is focal distance in comp pixels (z=0 maps 1:1). Only affects 3D-switch layers; the topmost visible camera is active. `solve_link` is §5.6's solve link (K-417); `None` — the usual case — is a camera the user drives by hand. |
| `Adjustment` | yes | — | No source of its own; its masks + effect stack apply to the composite of every layer beneath it, within its span. (There is no `adjustment` switch — it is this kind.) |
| `Null` | yes | — | No source and no size; carries only a transform, so layers parent to it and move as a rig. Never draws, emits no node in the evaluation graph, and reports no picture — so it is not offered as a matte or a layer-valued effect parameter. Masks and effects can be added to it but never run (as on a Camera). The bridge enum names this kind `NullLayer` for Dart's sake only (K-206). |
| `Shape { contents: Vec<ShapeItem> }` | yes | Its vector art | §7.2 (K-237). Flat list; nested groups and modifiers are future (§9.2). |
| `Light { light: Box<LightDef> }` | yes | §5.5 | A source of light other layers see (K-360). Draws no pixels of its own, like a Camera; its placement is the ordinary layer transform. |

**There is no `Audio` kind (K-435).** An Audio layer — a layer whose source is an audio item,
or the audio channel of footage — is a `Footage` layer with `audio_only` set on the layer
(§5.1). See §5.7.

### 5.3 Clips (Sequence layers only)

```rust
struct Clip {
    id: Uuid,
    source: ClipSource,            // Footage(Uuid) | Comp(Uuid)
    source_in: SourceTime,         // trim into the source
    source_out: SourceTime,        // exclusive
    place_start: Rational,         // clip start on the layer timeline (the doc's ClipTimeSpan,
    place_duration: Rational,      //   stored as start + duration)
    retime: Retime,                // exact rational boundaries — see 04-RETIMING.md
    interpolation: Interpolation,  // Nearest | Blend | Flow  (render policy, not part of the map)
}
// Future: a per-clip `label` (LabelColour).
```

Invariants (binding, per K-020/K-022):
- Clips on one Sequence layer MUST NOT overlap. Gaps are allowed and render transparent.
- An **edit point** is the shared boundary of two adjacent clips. Retime edits MUST NOT move
  `place` of any clip (the beat-sync covenant).
- Cutting a clip produces two clips whose retimes are exact partitions of the original
  ([04-RETIMING.md](04-RETIMING.md) §cutting).
- Layer-level properties (transform, effects, masks, matte, blend) apply to the Sequence
  layer's assembled output, after clip retiming — a glow keyframed on the layer is unaffected
  by where cuts fall.

### 5.5 Light layers (K-360)

`LayerKind::Light { light: LightDef }` — a source of light in the composition. Like a Camera
it draws no pixels of its own: it is something other layers *see*.

**Its placement is the ordinary layer transform**, not a second one of its own, so a light
animates, parents and is dragged with everything already built for layers. `LightDef` carries
only what a light *is*:

| Field | Meaning |
|---|---|
| `kind` | `point`, `spot` or `area` |
| `colour` | Scene-linear RGB, animatable per channel |
| `intensity` | Master gain |
| `half_size` | The emitting rectangle's half-width and half-height in comp pixels. **Area only** — the other kinds report zero extent whatever is stored, so a kind change cannot leave a stale size behind |
| `cone_deg` | The spot cone's half-angle; spot only. Aimed by the layer's own z rotation |
| `falloff_px` | Distance to nothing; zero means no falloff, which is usually what a flare source wants |

Half-extents rather than full, matching the Lens flare's Source size dials (§3.27, K-355), so
a light is measured from its centre outward like every other point-and-size pair in the model.

`Composition::lights_at(t)` resolves the **visible, in-span** lights at a comp time, **top of
the stack first** — the order the effects that read lights fill their slots in, so a frame
with more lights than an effect can carry spends them on the ones nearest the top. A light
switched off is not a light (K-230's rule for every layer).

**The area kind is the one that earns the layer.** An area light reaches the Lens flare's
Lights source mode with a real extent and flares as its own shape through the sampling K-355
already built for detected sources — a strip light throws bar-shaped ghosts, with no new
rendering code. Frame keys hash a light's own properties, unlike a Camera's (whose pose is
hashed at comp level), because a light that changed colour without renaming its frames would
serve a stale one.

**Two things read a light.** The Lens flare's Lights source mode (§3.27, K-360) puts a flare
where the light lands in the projected picture. The **lighting pass** (K-361,
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §1.8) shades every layer whose
`accepts_lights` switch is on, which is what makes a softbox fall across footage. The two
want different things from a light and both get them from the same resolve: the flare works
in the projected picture and ignores depth entirely, while shading needs `z` and the
out-of-plane rotations, because a rectangle in the same plane as the surface it lights is
edge-on and throws nothing.


### 5.6 The solve link on a Camera layer (K-417)

**In plain terms.** When a shot has been camera-tracked (§3.85 of
[08-EFFECTS.md](08-EFFECTS.md), [impl/tracking.md](impl/tracking.md)), the engine knows
where the real camera was on every frame of that *file*. The obvious thing to do with that
is to stamp it onto a Camera layer as several thousand keyframes. Lumit does not, at least
not at first: the Camera layer **points at** the tracked layer and derives its placement
afresh each frame. Re-solve the shot, trim it, reorder its cuts, ramp its speed — the camera
follows, because nothing was ever copied.

`solve_link` names a **layer in the same composition**: the one the analysis was run on, or
a Precomp layer that contains it. It is a layer id and nothing else — no cached poses, no
frame numbers, no copy of the solve.

**The derivation is the renderer's own time walk.** At composition time *t*, the linked
layer's time is walked to a moment of its source exactly as the render plan walks it to
choose a frame to decode: comp time, less the layer's `start_offset`, through the layer's
Retime (or, on a Sequence layer, through the clip under the playhead and *its* trim and
Retime), to a source time. That source time becomes a solved frame, and the solved frame
becomes the camera's position, rotation and zoom. Because it is the same walk, a reordered
Sequence layer, a speed ramp and a freeze all come out right without a special case — the
tracker ran on the file, once (K-248).

**Through a precomp.** A Precomp layer (or a comp-sourced clip) is walked *into*: the nested
composition's own tracked layer — the one carrying the Camera track effect, which is what
makes an effect the handle — continues the chain. This is the owner's workflow: a linked
camera in the parent comp points at the precomp layer, and the chain resolves through it to
the footage inside.

**Read-only while linked.** A linked camera's transform and zoom are derived, so they are
not the document's to edit: `SetTransformProperty` and `SetCameraZoom` on one refuse with
`OpError::CameraLinked`, and the panel draws the rows greyed and wearing a calm badge. The
rule is enforced in `ops::apply`, not in the interface, for the reason the lock is (K-291):
a rule enforced only in a panel is a rule the next caller does not know about.
`SetCameraSolveLink` is always accepted — a link that could not be undone would be a trap.

**Two honest failures, and neither is silent.** A link that asks for a moment outside what
was solved **holds** the nearest solved frame — the last derived motion — and reports that
it is holding. A link that cannot be followed at all (the layer deleted, the media offline,
nothing solved) falls back to the properties the document itself holds, and reports *that*.
Never a freeze nobody explained, never a crash.

**Convert to keyframes** severs the link and bakes the derived motion into ordinary
keyframes: one key per composition frame across the layer's span, linear on both sides, on
the six transform properties and on the zoom. They are real, editable keyframes and the
graph editor draws them like any others — the bake is honest about there being a lot of
them. It is one undoable step: the link is cleared *first* inside the batch (so the
read-only refusal does not stop the very edit that ends it) and restored *last* on undo.

The solves themselves live in the `track/` sidecar ([10-FILE-FORMAT.md](10-FILE-FORMAT.md)
§3), rebuildable like every sidecar tier; the model reaches them through a store trait, so
nothing in the document depends on the tracker. The store hands back poses **already in
Lumit's camera terms** — comp pixels, AE-style position and rotation, a `zoom` — because
turning a solve's world-to-camera rotations and source-pixel focal into those is a real piece
of work that belongs beside the solve, not in the model ([impl/tracking.md](impl/tracking.md)
§5b).

### 5.7 Audio layers (K-435)

**In plain terms.** An Audio layer is a layer that makes a sound and shows nothing.

`Layer::audio_only` is the whole of it. There is no `LayerKind::Audio`: the layer keeps its
footage source, its Volume, its waveform, its markers and its span, and the one thing that
differs is that no picture is asked for. `#[serde(default)]`, and not written out when false,
so every project saved before it existed reads unchanged.

- **Set on placement** when the media has no picture at all — a music file can only ever be
  an Audio layer, so it becomes one whichever route placed it. Media that will not probe is
  assumed to *have* a picture: a wrongly-flagged Audio layer would silently drop a picture
  the user placed, where a footage layer that turns out silent merely shows nothing.
- **Set deliberately** by *Add audio only* on a footage item, which is the case the flag
  exists for: the sound of a clip that does have a picture, on its own row, so it can be cut,
  faded and keyframed by itself. One `AddLayer` op, so one undo step.
- **Every picture path skips it**: the frame key (`feed_comp`, lumit-eval), the decode plan
  and the draw builder (lumit-render), the occlusion cull, and `comp_footage_items` — so a
  video placed for its sound alone is never probed or frame-indexed for a picture it will not
  show. The audio path (`AudioJobsBuilder::audio_jobs`) does not consult the flag at all.
- **The frame key skips it before the switch gate, not after.** A layer tested for `visible`
  first would change the picture's name by being hidden or shown; skipped ahead of the gate,
  none of an Audio layer's switches can reach the key. Muting, hiding, soloing or shying an
  Audio layer retires no rendered frame.
- **Solo is two questions.** `any_solo` — every soloed layer — is the mixer's; `any_picture_
  solo` — soloed layers that draw — is the compositor's, the cull's and the key's. Soloing a
  music track means "just this sound", never an empty picture.
- **Across the bridge** it reads as its own kind, `BridgeLayerKind::Audio`, so the Timeline
  draws it with its own glyph and no thumbnail; `has_picture` answers false whatever the file
  holds, which is what the outline reads to know the layer has no visibility switch to offer.

**Not built:** *Detach audio* — a linked Audio layer sharing a Footage layer's source, kept
in step with it (docs/09 §6). *Add audio only* makes an independent layer from the item, not
a link to an existing layer.

---

## 6. Properties, keyframes, animation

### 6.1 Property

A **property** is an animatable slot. Properties live in **property groups** forming a stable
tree (transform group, each effect's parameters, each mask's geometry, retime).

```rust
// v1: a Property is a scalar f64.
struct Property {
    animation: Animation,            // Static(f64) | Keyframed(Vec<Keyframe>)
}

enum Animation {
    Static(f64),
    Keyframed(Vec<Keyframe>),        // sorted by time, unique times
}
```

A multi-dimensional value (a Vec2 position, a Vec3 scale, a colour) is stored in v1 as
**separate per-dimension scalar properties** (`position_x`/`position_y`, …), not a generic
`Property<T>`. The generic `Property<T: PropValue>` over `Vec2`/`Vec3`/`LinearColour`/`bool`/
`enum`/`BezierPath`/`TextDocument`, the stable-`id` addressing, and the `expression` slot are
**future** — they arrive with the expression engine (§6.4, [12-PLUGINS.md](12-PLUGINS.md)),
which v1 does not have. There is no `PropValue` trait in v1.

### 6.2 Keyframes — AE-compatible maths (K-025)

```rust
// v1: value is f64 (see §6.1).
struct Keyframe {
    time: OwnerTime,                  // timebase of the owning object
    value: f64,
    interp_in:  SideInterp,           // approaching this key
    interp_out: SideInterp,           // leaving this key
}
// Future: `spatial: SpatialTangents` and `roving` (Vec2/Vec3 motion paths) and a
// per-keyframe `label` — they arrive with the motion-path unit.

enum SideInterp {
    Hold,
    Linear,
    Bezier { speed: f64, influence: f64 },   // speed: value-units/sec; influence: a fraction in (0, 1]
    Auto { clamped: bool, speed: f64, influence: f64 },  // speed/influence: the remembered free ease
}
```

The **`Auto` arm** is the graph strip's Auto (`clamped: false`) and Clamp (`clamped: true`)
tangent modes (K-506): the side's *speed* is computed from the key's two neighbours on
every read rather than stored, while its influence is its own. The `speed` and `influence`
it carries are **not evaluated** — they are the ease the side had when it was last free,
so that returning it to Free hands the custom ease back. The arithmetic, and why the
memory lives inside the arm, are [impl/keyframe-eval.md](impl/keyframe-eval.md) §6.

Between two keys `(t1,v1) → (t2,v2)` with bezier sides, the value curve is the cubic bezier
with control points at

```
P1 = (t1 + influence_out·Δt, v1 + speed_out·influence_out·Δt)
P2 = (t2 − influence_in·Δt,  v2 − speed_in·influence_in·Δt)      where Δt = t2 − t1
```

— exactly AE's model, so Bridge import ([11-AE-IMPORT.md](11-AE-IMPORT.md)) is lossless and
the speed graph in the graph editor is the true derivative. `influence` is stored as a
fraction in `(0, 1]` (AE's percentage ÷ 100); the easy-ease preset is speed 0, influence `1/3`
(AE's 33.3%).

Spatial properties would additionally carry in/out tangents in value space defining the motion
path, and **roving** keyframes would surrender their time to equalise speed along the path.
Both are **future** (the motion-path unit); v1 animates scalar dimensions independently.

### 6.3 Evaluation order of one property

```
keyframe/static evaluation → [expression — FUTURE] → [driver edge — K-471] → clamp/validate
```

The **expression** stage is future (§6.4); v1 evaluates keyframes/static only. The
**driver** stage (K-471, §8.1): a parameter with a wired input port in the layer's driver
graph takes the driver's value, overriding whatever the earlier stages produced, and the
Effect controls row says so. A property's
evaluated value at a time is pure regardless: same project, same time, same value — no wall
clock, no external state ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)).

### 6.4 Expressions

A property can hold a line of code instead of a number or a row of keyframes. The engine is
**Rhai** (K-305, superseding K-063's choice of JavaScript on QuickJS-ng);
[impl/expressions.md](impl/expressions.md) is the authority on how it works and
[12-PLUGINS.md](12-PLUGINS.md) §4 on what it exposes.

It is a third arm of `Animation`, alongside a static value and a keyframe list, so a
property is *either* keyframed *or* driven — never both:

```rust
enum Animation {
    Static(f64),
    Keyframed(Vec<Keyframe>),
    Expression(String),   // Rhai source; see 12-PLUGINS.md §4.2
}
```

The source is stored verbatim. There is no compiled form on disk and no separate
`enabled` flag: clearing the text is how an expression is removed, and the property returns
to the value it held before.

**Scalars only.** `Animation::Expression` reaches the scalar transform properties and Float
effect parameters. Point and colour properties cannot be driven yet. A text layer is the
one non-scalar case, and it carries its own optional `expression` on the `TextDocument`
(§9.1, K-306) rather than going through `Animation`, because its result is printed rather
than measured and so may be of any type.

**An expression failure never fails the render.** Today the fallbacks are blunt: a numeric
expression that errors resolves to `-1.0`, and a text one prints nothing. The specified
behaviour — the property falls back to its keyframed value and the expression is disabled
with a badge naming the error — is not built, and is carried as a known gap in
[impl/expressions.md](impl/expressions.md) §8. `last_error` is runtime state either way, and
is never serialised as authority.

---

## 7. Masks

```rust
// A mask: an animatable path, a mode, and three animatable numbers (K-340).
struct Mask {
    id: Uuid,
    name: String,
    path: BezierPath,                 // the shape when path_keys is empty
    path_keys: Vec<PathKeyframe>,     // empty = not animated (absent from the file)
    inverted: bool,
    opacity: Property,                // 0..100
    mode: MaskMode,                   // None | Add | Subtract | Intersect | Difference
    feather: Property,                // layer px, total ramp width (0 = hard edge)
    expansion: Property,              // layer px, + grows the shape, − shrinks it
}

struct PathKeyframe {
    time: Rational,                   // the owner's timebase — layer time for a layer's masks
    path: BezierPath,
    interp_in: SideInterp,            // Hold | Linear | Bezier { speed, influence }
    interp_out: SideInterp,
}
```

Masks apply in order before the effect stack ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)).
Each mask's own coverage is feathered and expanded first, then inverted, then faded by its
opacity, and only then folded into the running total by its `mode` — so inverting a feathered
mask gives the complement of the soft edge, not a soft edge on the complement. The fold runs
top to bottom and **order matters**. It starts from an empty frame when the topmost mask that
does anything is `Add`, and from a full frame otherwise, so that a lone `Subtract` mask cuts a
hole in the picture (the After Effects behaviour) rather than subtracting from nothing.

Feather and expansion are two readings of one signed distance field built from the mask's own
raster: expansion moves the edge, feather sets the width of the ramp across it. Both are in
layer pixels and scale with preview resolution, so a soft edge keeps its real width at half
resolution. A mask with neither takes a fast path and is used exactly as rasterised.

`mode`, `feather`, `expansion` and `path_keys` are omitted from the file when they hold their
defaults (`Add`, 0, 0, empty), so a project that predates them reads and writes
byte-identically and keeps the frames its cache has already banked.

`opacity`, `feather` and `expansion` are `Property`s but **do not write themselves as one
while they are still** (K-340): a static value writes as the bare number it always wrote,
and only a mask somebody has keyed writes the animation object. Reading takes either. The
same promise, for the same reason — the frame key names a mask by the bytes its list
serialises to, so an unkeyed mask must be byte-identical to what it was.

A mask whose `mode` is `None` **or** whose opacity is zero at the time being drawn does
nothing at all, and a layer whose masks are all in that state is unmasked and whole — not
blank (K-340).

### 7.0 The animated path

The path is the one part of a mask that animates, and it carries its **own** keyframe list
rather than going through `Property` — a `Property` holds one `f64`, and generifying it over a
whole shape would churn every scalar call site in the engine to buy nothing. `path_keys` is
that carrier: sorted, unique times, empty for a mask nobody has keyed. While it holds any key,
`path` is ignored; `Mask::path_at(t)` is the single reader, and it hands back the stored path
by reference in the ordinary unanimated case.

**Timing eases, no value graph.** A shape has no scalar to plot, so path keys draw as diamonds
only (as they do in After Effects). Each key still carries the ordinary `SideInterp` pair, and
it shapes the **interpolation parameter** — 0 at this key, 1 at the next — through exactly the
scalar evaluator, so `Hold`, `Linear` and AE speed/influence all behave as they do everywhere
else, and a key's own time always lands exactly on its own shape.

**Mismatched vertex counts resample upward.** Two keys need not have the same number of
points — "add a point halfway through" is a thing people do constantly, and refusing is not an
option. The sparser path is redrawn with as many vertices as the denser one by **splitting its
own segments** (de Casteljau at a parameter: the two halves *are* the original cubic, not an
approximation of it), so the reconciled path is geometrically the path it was — nothing bulges
or flattens. Which segments receive the extra points is fixed arithmetic — spread as evenly as
the count allows, remainder to the earliest segments — so the reconciliation is deterministic
and playback is repeatable. Then the two run vertex for vertex: position and both tangent
handles are straight-line blended. This is After Effects' behaviour, so an imported comp and a
hand-built one move alike.

**Closedness is held, not blended.** Whether a path is closed is not a quantity. Across a span
it takes the outgoing key's flag and flips at the next key — a `Hold` in all but name. The
geometry interpolates normally; only the closing segment appears or disappears, and it does so
on a frame boundary rather than smearing.

**Evaluation and the cache.** Masks are applied at the layer's own local time, the same clock
its transform and effects read (K-213), so a keyframed mask travels with a layer dragged along
the timeline. The frame-cache key (06 §5.2) carries no time of its own by design, and a
keyframed mask serialises identically at every frame — so the **evaluated** path joins the hash
whenever `path_keys` is non-empty, and only then, leaving every unanimated mask's key exactly
as it was.

The op is still `SetLayerMasks`, the whole list, exactly invertible.

**Future:** the `Lighten` / `Darken` modes, deliberately left out of the first mode set. Variable-width feather is later still; the model
will reserve per-vertex feather data.

### 7.1 Paint strokes (K-227)

```rust
struct PaintStroke {
    id: Uuid,
    name: String,
    points: Vec<(f64, f64)>,          // layer space, in the order drawn
    colour: LinearColour,
    width: f64,                       // brush diameter, layer pixels
    hardness: f64,                    // 0 fully soft .. 1 hard edge
    opacity: f64,                     // 0..100
    mode: PaintMode,                  // Paint | Erase | Clone
    clone_offset: (f64, f64),         // Clone: where the pixels come from
}
```

A stroke stores the **gesture**, never the pixels: it is re-stamped at whatever resolution the
frame is rendered at, so a stroke painted at a quarter preview exports at full size, and every
setting stays changeable. The path is a **polyline** rather than a `BezierPath` — a stroke is a
record of a drag, not a shape edited vertex by vertex — and it is thinned before it is stored
(samples closer than two screen pixels dropped, first and last always kept).

Strokes are stamped into the layer's own raster **before** its masks gate it and before its
effects run ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)). `Clone` reads the raster as it was
before *any* stroke in the pass was stamped, so a clone never picks up paint laid down beside
it. The op is `SetLayerPaint` — the whole list, exactly invertible, like `SetLayerMasks`.

**Future:** pressure and tilt, non-round brush shapes, spacing and scatter, write-on (per-stroke
start and end times), per-stroke blending modes, and a GPU stamping path. None of them changes
the shape above.

### 7.2 Shape layers (K-237)

```rust
LayerKind::Shape { contents: Vec<ShapeItem> }

struct ShapeItem {
    id: Uuid,
    name: String,
    path: BezierPath,                 // the mask's path type, unchanged
    fill: Option<LinearColour>,       // None draws no fill
    stroke: Option<LinearColour>,     // None, or a zero width, draws no outline
    stroke_width: f64,                // layer pixels
    opacity: f64,                     // 0..100
}
```

A shape layer's art **is** its picture: vector paths rasterised at whatever resolution the
frame is rendered at, so they stay crisp at any scale. The path type is the mask's, deliberately
— a shape's path and a mask's path differ in what they do, not in what they are.

The list is **flat**: After Effects' nested groups exist to carry its shape modifiers (repeater,
trim paths, wiggle), and none of those are built.

**The layer's natural size is the box its art fills**, bounding the curves by their control
points, and it *changes as the art is edited* — the only layer kind whose size is not fixed by
its source. Anything caching a layer's size must key on the document revision.

The op is `SetShapeContents` — the whole list, exactly invertible, like `SetLayerMasks` and
`SetLayerPaint`.

**Future:** nested groups and the shape modifiers, gradient fills, dashed strokes, joins and
caps other than round, and animated paths.

## 8. Effects

```rust
struct EffectInstance {
    id: Uuid,
    effect: EffectKey,        // { namespace: Builtin|Ofx|Lfx|Placeholder, match_name, version }
    enabled: bool,
    params: PropertyGroup,    // declared by the effect; all animatable, expression-visible
}
```

**Placeholder** effects (from AE import, or a missing plugin) keep `match_name` and the full
parameter dump, render as identity with a badge, and round-trip through save untouched
([11-AE-IMPORT.md](11-AE-IMPORT.md)).

An effect parameter may also **reference another layer** as an auxiliary input (a
Layer-reference parameter, [08-EFFECTS.md](08-EFFECTS.md) §1.2 — a depth pass for Depth of
field): the stored value is an optional layer id, the same by-id cross-reference §5.1's matte
uses, and a dangling reference degrades to a no-op exactly as a dangling matte does. A
companion `<id>_source` Choice holds its `LayerInputSource` sampling mode (None / Masks /
Effects and masks, K-142), the same three-way source a matte carries in §5.1.

### 8.1 Wiring — the layer's driver graph (K-471)

`effects` remains the only authority for the image chain: the Graph panel derives its
image-path nodes from the list, and every image-wire gesture lowers to `SetLayerEffects`.
Beside it, each layer carries an additive `LayerGraph`:

```rust
struct LayerGraph {
    nodes: Vec<EffectInstance>,        // drivers — data signature, no image kernel;
                                       // declared in the effect registry's Drivers category
    edges: Vec<Edge>,                  // driver output → parameter or matte input;
                                       // plus SourceMatte, the layer's own masked source alpha
    layout: Vec<(NodeRef, [f64; 2])>,  // canvas positions; missing entries auto-place
}
```

A driven parameter overrides its keyframes (§6.3); driver parameters are ordinary
properties on the path `<layer>/graph/<node>/<param>`. Edges never cross layers — a
cross-layer tap (Audio level reading another layer's sound) is a layer-reference
parameter (§8, above), drawn as a derived source node. An effect's Input port is by
construction the previous stack entry, so every graph state has an honest stack
rendering. One op, `SetLayerGraph`, is the whole-graph commit, mirroring
`SetLayerEffects`; a cycle, type mismatch or doubled input is refused at apply, and a
dangling layer reference degrades as a matte does. Port types and the points stream are
K-472. [impl/node-graph.md](impl/node-graph.md) is the authority on all of it.

## 9. Rich layer payloads

### 9.1 Text

v1 `TextDocument` is a **single run**: `{ text, expression, size, fill }` — one font (embedded
Inter), one size, one fill, single line. The styled-runs model — font family/weight, stroke,
tracking, leading, point vs paragraph text, alignment, and per-character animators — is
**future**; the document stays structured (never rasterised into the project) so runs and
animators bolt on later.

**The words can come from an expression (K-210).** `expression` is optional and absent from the file
when unset. When it is set, the layer's line at layer time *t* is that expression evaluated at
*t* and printed — the same language the numeric properties use (§6.4), except the answer is
shown rather than measured, so any result type is accepted and an evaluation error prints
nothing rather than failing the frame. `text` is untouched while an expression drives the
layer and is what the layer says again once the expression is cleared; an empty or
whitespace-only expression *is* "cleared", never "an expression that says nothing".

The rasteriser and the frame cache key both read the line through one resolver, so they can
never disagree about what the layer says — a disagreement would serve a cached frame of the
previous line. A frame-varying expression therefore keys per frame by construction, and a
constant one keys once. Per-character animation of an expression-driven line is **future**,
with the styled-runs model.

### 9.2 Shape — how the shipped flat list grows

`LayerKind::Shape` ships as §7.2's flat `Vec<ShapeItem>` (K-237). The intended growth is a
`ShapeElement` tree: groups; parametric rectangle/ellipse/polystar; fill (solid, linear/radial
gradient); stroke (width, caps, joins, dashes); trim paths. Repeater, offset, wiggle-path are
tier 2 ([08-EFFECTS.md](08-EFFECTS.md) keeps the list).

### 9.3 2.5D (K-023)

All transforms are 4×4 internally from day one; the `three_d` switch exposes z and full
rotation. The Phase 1 camera is the seed of `CameraProps`: `Camera { zoom: Property }` —
a one-node camera whose zoom is the AE model (focal distance in comp pixels; the z=0
plane maps 1:1, a layer at depth z scales by zoom/(z+zoom)), positioned and rotated by
the layer's own transform group, with the topmost visible camera active. `CameraProps`
v1 grows from there: one-node/two-node, focal length presets, depth of field (focus
distance, aperture, blur level). `LightProps` v1: ambient/point/spot/directional with
intensity, colour, cone; shadows post-v1. 2D layers ignore cameras (render in a fixed
orthographic pass), matching AE's mental model.

## 10. Undo, journal, dirty state

All mutations go through **operations** — small, serialisable, invertible commands
(`SetKeyframe`, `MoveClip`, `AddLayer`, …) applied to the document behind a single writer.
The **operation journal** is the undo/redo stack and the autosave crash-recovery log
([10-FILE-FORMAT.md](10-FILE-FORMAT.md) §autosave). The UI renders from immutable snapshots;
workers render from the snapshot current when their job was scheduled
([05-ARCHITECTURE.md](05-ARCHITECTURE.md)).

## 11. Markers

```rust
struct Marker {
    id: Uuid,
    time: OwnerTime,
    duration: Option<RationalTime>,
    label: String,
    kind: MarkerKind,        // User | Beat { confidence: f32 } | Chapter
}
// Future: a marker `colour` (LabelColour).
```

Beat markers are ordinary markers with provenance; regenerating beats replaces only
`Beat`-kind markers ([09-AUDIO.md](09-AUDIO.md)).

**Two owners (K-254).** A composition holds markers on its ruler; a layer holds markers of
its own on its bar, in `Layer::markers`, timed in the layer's own time so they move with it.
A layer's list is always a **copy**, never a view: dropping a composition into another
copies that comp's markers onto the layer with fresh ids, and editing either list afterwards
leaves the other alone. Pre-composing copies the comp's markers into the new composition and
leaves the Precomp layer's list empty — the cues are on the ruler above it already. One
marker per frame per owner: placing one where a marker already sits replaces it.

## 12. Schema evolution

The model is versioned (`schema_version` + a `min_reader` gate in the manifest — a file too new
for the reader is refused with a clear message, docs/10 §1). Rules, binding:
- Additive changes only where possible; unknown fields MUST be preserved on load/save
  (forward compatibility for shared projects, K-065).
- Post-1.0, any breaking change ships with a migration and a decision-log entry.

v1 reality (pre-1.0): there is **no migration framework** yet — compatibility rests on
additive fields with serde defaults, pervasive unknown-field preservation, and a few ad-hoc
`serde(from = …)` shims (e.g. the K-142 matte-source and K-147 scanline migrations). Under the
standing **pre-release no-migration policy**, breaking reshapes so far have simply not owed a
migration (they are logged in 02-DECISIONS instead). A registry lands as 1.0 nears.

## Open questions

- Maximum comp size: 16384² is the common GPU texture limit; do we macro-tile to exceed it
  (AE allows 30000²) or cap and revisit?
- Should `stretch` survive long-term, or is it sugar the UI lowers into Retime? (It rescales
  keyframes, which Retime deliberately does not — kept for AE compatibility for now.)
- Per-vertex mask feather data reserved but unspecified — spec when variable-width feather lands.
- Gradient model for text stroke/fill v1 or tier 2?
