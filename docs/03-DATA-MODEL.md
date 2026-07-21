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

Authoritative time is never floating point (see [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)).

```rust
struct RationalTime { num: i64, den: u32 }   // seconds = num / den
struct FrameRate    { num: u32, den: u32 }   // e.g. 60000/1001
```

The four timebases (source, clip, layer, comp — [01-GLOSSARY.md](01-GLOSSARY.md) §4) are
distinct newtypes over `RationalTime`. Conversions between them are explicit functions, and
the Retime map ([04-RETIMING.md](04-RETIMING.md)) is the only nontrivial conversion.

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
}
```

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
// v1 stores only the path pair.
struct MediaRef {
    relative_path: String,     // relative to project file where possible
    absolute_path: String,     // last known absolute location
}
```

**Future** — none of this is in v1 yet:

- a `fingerprint` (size + mtime + head/tail content hash) for reliable relinking (a
  `Fingerprint` type exists in `lumit-media` but is not stored on the reference);
- a `FootageInterpretation` (frame-rate override, alpha mode, colour-space tag, loop count,
  timecode policy) — v1 treats every source as sRGB with no per-item overrides;
- the **missing**-footage state (placeholder slate + relink flow, [07-UI-SPEC.md](07-UI-SPEC.md)).

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
    effects: Vec<EffectInstance>,      // §8, ordered top-to-bottom
    volume_db: Property,               // K-172: animatable Volume (docs/09 §6); 0 dB unity, −100 = −∞
    switches: Switches,
}
// Future (not in v1): `stretch` (uniform rate multiplier) and per-layer `markers`.
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
}
// Future switches (K-168, deferred): `shy` (needs an outline filter row) and
// `quality` (Draft|Full — needs a bicubic sampler choice). `adjustment` is not a
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
| `Camera { zoom: Property }` | yes | — | AE camera: `zoom` is focal distance in comp pixels (z=0 maps 1:1). Only affects 3D-switch layers; the topmost visible camera is active. |
| `Adjustment` | yes | — | No source of its own; its masks + effect stack apply to the composite of every layer beneath it, within its span. (There is no `adjustment` switch — it is this kind.) |
| `Shape { contents: Vec<ShapeElement> }` | future | §9.2 | |
| `Null` | future | — | Transform-only, invisible. |
| `Audio { item: Uuid }` | future | An audio item | v1 audio is only a footage layer's own stream (§5.2, docs/09). |
| `Light` | future | — | Paired with Camera; not in v1. |
| `Light { light: LightProps }` | §9.3 | 3D only. |

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
}
```

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
keyframe/static evaluation → [expression — FUTURE] → clamp/validate
```

The **expression** stage is future (§6.4); v1 evaluates keyframes/static only. A property's
evaluated value at a time is pure regardless: same project, same time, same value — no wall
clock, no external state ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)).

### 6.4 Expressions (future — no engine in v1)

The expression engine (JavaScript on QuickJS, K-063 / [12-PLUGINS.md](12-PLUGINS.md)) is not in
v1: `Property` has no expression slot yet. The intended shape:

```rust
struct Expression {
    source: String,          // JavaScript, ES2018 surface — see 12-PLUGINS.md
    enabled: bool,
    last_error: Option<ExprError>,   // runtime state, not serialised as authority
}
```

An expression failure disables that expression with a badge and falls back to the
pre-expression value. It never fails the render.

---

## 7. Masks

```rust
// v1: a static, Add-mode mask.
struct Mask {
    id: Uuid,
    name: String,
    path: BezierPath,                 // closed; static (not yet animatable)
    inverted: bool,
    opacity: f64,                     // 0..100, static
}
```

Masks apply in order before the effect stack ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)).
**Future:** an animatable `path`/`opacity` (`Property<…>`), the full `mode` set
(`None|Add|Subtract|Intersect|Lighten|Darken|Difference` — v1 is **Add only**), and `feather` /
`expansion`. Variable-width feather is post-v1; the model will reserve per-vertex feather data.

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

## 9. Rich layer payloads

### 9.1 Text

v1 `TextDocument` is a **single run**: `{ text, size, fill }` — one font (embedded Inter), one
size, one fill, single line. The styled-runs model — font family/weight, stroke, tracking,
leading, point vs paragraph text, alignment, and per-character animators — is **future**; the
document stays structured (never rasterised into the project) so runs and animators bolt on
later.

### 9.2 Shape (future — no Shape layer in v1)

There is no `LayerKind::Shape` yet. The intended `ShapeElement` tree: groups; parametric
rectangle/ellipse/polystar; bezier path; fill (solid, linear/radial gradient); stroke (width,
caps, joins, dashes); trim paths. Repeater, offset, wiggle-path are tier 2
([08-EFFECTS.md](08-EFFECTS.md) keeps the list).

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
