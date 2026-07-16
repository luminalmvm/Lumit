# Luminal data model

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

```rust
struct Project {
    id: Uuid,
    settings: ProjectSettings,      // display transform default, expression engine version
    assets: Vec<Asset>,             // tree via Folder
    compositions: Vec<Uuid>,        // comps are assets too; this orders the Project panel
}
```

An **Asset** is one of:

| Asset | Contents |
|---|---|
| `FootageItem` | Media reference (§3), interpretation, proxy state |
| `AudioItem` | Audio-only media reference |
| `StillItem` | Single image |
| `SequenceItem` | Image sequence (pattern, fps) |
| `Composition` | §4 |
| `Folder` | Ordered children ids |
| `SolidDef` | Shared solid definition (colour, size) — solids are assets so they dedupe |

### 3. Media references and interpretation

```rust
struct MediaRef {
    relative_path: String,     // relative to project file where possible
    absolute_path: String,     // last known absolute location
    fingerprint: Fingerprint,  // size + mtime + head/tail content hash, for relinking
}

struct FootageInterpretation {
    frame_rate_override: Option<FrameRate>,
    alpha: AlphaMode,               // straight | premultiplied(colour) | ignore | guess
    colour_space: ColourSpaceTag,   // default: Rec.709/sRGB assumption for game captures
    loop_count: u32,
    start_timecode_policy: TcPolicy,
}
```

A footage item whose file cannot be found enters a **missing** state: it keeps all metadata,
renders as a labelled placeholder slate, and never blocks project open. Relink flow in
[07-UI-SPEC.md](07-UI-SPEC.md).

---

## 4. Composition

```rust
struct Composition {
    id: Uuid,
    name: String,
    width: u32, height: u32,            // hard cap 16384×16384 in v1
    pixel_aspect: Rational,             // 1:1 default
    frame_rate: FrameRate,
    duration: CompTime,
    background: LinearColour,
    depth: CompDepth,                   // Fp16 (default) | Fp32   (K-026, per-comp)
    motion_blur: MotionBlurSettings,    // shutter_angle (deg), shutter_phase, max_samples
    work_area: (CompTime, CompTime),
    markers: Vec<Marker>,
    layers: Vec<Layer>,                 // index 0 = top of the stack
}
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
    in_point: CompTime,
    out_point: CompTime,               // exclusive; out > in
    start_offset: CompTime,            // where layer time 0 sits on the comp timeline
    stretch: Rational,                 // uniform rate multiplier; rescales this layer's keyframes
    parent: Option<Uuid>,              // transform parenting; cycles are invalid states
    switches: Switches,
    blend_mode: BlendMode,
    matte: Option<MatteRef>,           // { layer: Uuid, channel: Alpha|Luma, inverted: bool }
    transform: TransformGroup,         // §6
    masks: Vec<Mask>,                  // §7
    effects: Vec<EffectInstance>,      // §8, ordered top-to-bottom
    markers: Vec<Marker>,
    audio: Option<AudioProps>,         // level (animatable), mute — when the source has audio
}

struct Switches {
    visible: bool, audible: bool, solo: bool, locked: bool, shy: bool,
    quality: Quality,                  // Draft | Full
    motion_blur: bool,
    adjustment: bool,
    three_d: bool,
    collapse: bool,                    // Precomp layers: transform concatenation
}
```

Invariants:
- `in_point`/`out_point` clamp within the comp duration for rendering but MAY extend beyond
  (AE-style) without data loss.
- A matte reference to a missing/deleted layer degrades to "no matte" with a badge, never an error.
- Any layer can serve as a matte for any number of consumers; the engine evaluates it once
  ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)).

### 5.2 Layer kinds

| Kind | Source payload | Notes |
|---|---|---|
| `Footage { item: Uuid, retime: Retime }` | One footage/still/sequence item | The AE-style default. Retime per [04-RETIMING.md](04-RETIMING.md). |
| `Sequence { clips: Vec<Clip> }` | Its clips | §5.3. |
| `Precomp { comp: Uuid, retime: Retime }` | Another composition | `collapse` switch defers rasterisation. Cycles invalid. Retime supported (a non-identity Retime forces an intermediate, disabling collapse). |
| `Solid { def: Uuid }` | A SolidDef | |
| `Text { document: TextDocument }` | §9.1 | |
| `Shape { contents: Vec<ShapeElement> }` | §9.2 | |
| `Null` | — | Transform-only, invisible. |
| `Adjustment` | — | `adjustment` switch implied; effect stack applies to composite below. |
| `Audio { item: Uuid }` | An audio item | No visual payload. |
| `Camera { cam: CameraProps }` | §9.3 | 3D only. |
| `Light { light: LightProps }` | §9.3 | 3D only. |

### 5.3 Clips (Sequence layers only)

```rust
struct Clip {
    id: Uuid,
    source: ClipSource,            // FootageItem | Composition
    source_in: SourceTime,         // trim into the source
    source_out: SourceTime,        // exclusive
    place: ClipTimeSpan,           // start + duration in layer time; derived from edits, stored explicitly
    retime: Retime,                // exact rational boundaries — see 04-RETIMING.md
    interpolation: FrameInterp,    // Nearest | Blend | Flow  (render policy, not part of the map)
    label: LabelColour,
}
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
struct Property<T: PropValue> {
    id: Uuid,
    animation: Animation<T>,
    expression: Option<Expression>,   // §6.4 — applied after keyframe evaluation
}

enum Animation<T> {
    Static(T),
    Keyframed(Vec<Keyframe<T>>),      // sorted by time, unique times
}
```

`PropValue` types: `f64`, `Vec2`, `Vec3`, `LinearColour`, `bool`, `enum` (hold-only
interpolation for the last two), `BezierPath` (mask/shape geometry), `TextDocument`.

Property addressing is by stable path of ids (not display names), so expressions and the AE
Bridge survive renames: `layer(id).effect(id).param(id)`.

### 6.2 Keyframes — AE-compatible maths (K-025)

```rust
struct Keyframe<T> {
    time: OwnerTime,                  // timebase of the owning object
    value: T,
    interp_in:  SideInterp,           // approaching this key
    interp_out: SideInterp,           // leaving this key
    spatial: Option<SpatialTangents>, // Vec2/Vec3 positional properties only
    roving: bool,                     // spatial properties only
    label: Option<LabelColour>,
}

enum SideInterp {
    Hold,
    Linear,
    Bezier { speed: f64, influence: f64 },   // speed: units/sec; influence: 0.1..=100.0 (%)
}
```

Between two keys `(t1,v1) → (t2,v2)` with bezier sides, the value curve is the cubic bezier
with control points at

```
P1 = (t1 + influence_out·Δt, v1 + speed_out·influence_out·Δt)
P2 = (t2 − influence_in·Δt,  v2 − speed_in·influence_in·Δt)      where Δt = t2 − t1
```

— exactly AE's model, so Bridge import ([11-AE-IMPORT.md](11-AE-IMPORT.md)) is lossless and
the speed graph in the graph editor is the true derivative. Easy-ease presets are speed 0,
influence 33.33%.

Spatial properties additionally carry in/out tangents in value space defining the motion
path; **roving** keyframes surrender their time and are repositioned to equalise speed along
the path (recomputed whenever neighbours change).

### 6.3 Evaluation order of one property

```
keyframe/static evaluation → expression (may read the pre-expression value) → clamp/validate
```

A property's evaluated value at a time is pure: same project, same time, same value —
no wall clock, no external state ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)).

### 6.4 Expressions

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
struct Mask {
    id: Uuid,
    path: Property<BezierPath>,       // closed or open; animatable
    mode: MaskMode,                   // None|Add|Subtract|Intersect|Lighten|Darken|Difference
    inverted: bool,
    opacity: Property<f64>,           // 0..100
    feather: Property<Vec2>,          // px at layer scale
    expansion: Property<f64>,         // px, signed
}
```

Masks apply in order before the effect stack ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)).
Variable-width feather is post-v1; the model reserves per-vertex feather data.

## 8. Effects

```rust
struct EffectInstance {
    id: Uuid,
    effect: EffectKey,        // { namespace: Builtin|Ofx|Kfx|Placeholder, match_name, version }
    enabled: bool,
    params: PropertyGroup,    // declared by the effect; all animatable, expression-visible
}
```

**Placeholder** effects (from AE import, or a missing plugin) keep `match_name` and the full
parameter dump, render as identity with a badge, and round-trip through save untouched
([11-AE-IMPORT.md](11-AE-IMPORT.md)).

## 9. Rich layer payloads

### 9.1 Text

v1 `TextDocument`: styled runs (font family/weight, size, fill, stroke, tracking, leading),
point vs paragraph text, alignment. Per-character animators are post-v1; the document model
keeps text as structured runs (never rasterised into the project) so animators bolt on later.

### 9.2 Shape

v1 `ShapeElement` tree: groups; parametric rectangle/ellipse/polystar; bezier path; fill
(solid, linear/radial gradient); stroke (width, caps, joins, dashes); trim paths. Repeater,
offset, wiggle-path are tier 2 ([08-EFFECTS.md](08-EFFECTS.md) keeps the list).

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
    colour: LabelColour,
    kind: MarkerKind,        // User | Beat { confidence: f32 } | Chapter
}
```

Beat markers are ordinary markers with provenance; regenerating beats replaces only
`Beat`-kind markers ([09-AUDIO.md](09-AUDIO.md)).

## 12. Schema evolution

The model is versioned (`schema_version` in the project file). Rules, binding:
- Additive changes only where possible; unknown fields MUST be preserved on load/save
  (forward compatibility for shared projects, K-065).
- Any breaking change ships with a migration and a decision-log entry.
- Pre-1.0, migrations may be dropped after six months; post-1.0, never.

## Open questions

- Maximum comp size: 16384² is the common GPU texture limit; do we macro-tile to exceed it
  (AE allows 30000²) or cap and revisit?
- Should `stretch` survive long-term, or is it sugar the UI lowers into Retime? (It rescales
  keyframes, which Retime deliberately does not — kept for AE compatibility for now.)
- Per-vertex mask feather data reserved but unspecified — spec when variable-width feather lands.
- Gradient model for text stroke/fill v1 or tier 2?
