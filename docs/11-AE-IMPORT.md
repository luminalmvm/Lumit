# After Effects import

**Status: canonical.** This document specifies how After Effects projects come into Luminal.
It implements K-060 (import strategy), and leans on K-025 (AE-compatible keyframe maths) and
K-021 (Retime) in [02-DECISIONS.md](02-DECISIONS.md). Terminology follows
[01-GLOSSARY.md](01-GLOSSARY.md) exactly; After Effects' own feature names appear in quotes
when describing AE itself. RFC-2119 keywords (MUST, SHOULD, MAY) are binding.

The target user is migrating *from* After Effects, often mid-project, often with a folder of
community project files ("CC packs") they depend on. Import exists to make that migration
undramatic: what can carry over carries over exactly; what cannot is preserved, labelled, and
never silently dropped.

---

## 1. Strategy overview

Three routes, in fidelity order (K-060):

| Route | Requires | Fidelity | Status in UI |
|---|---|---|---|
| **Luminal Bridge** (primary) | After Effects installed, any recent version | High — the scripting DOM is documented public API | "Imported from After Effects" |
| **Direct `.aep` parsing** (secondary) | Nothing | Structure only, best-effort | "Recovered from .aep — structure only" |
| **Lottie / bodymovin JSON** (tertiary) | A `.json` export | High within Lottie's own scope | "Imported from Lottie" |

The Bridge is the proven pattern (bodymovin/Lottie walks the same DOM in the other
direction): let After Effects itself be the parser. Every keyframe, easing handle, mask
path, and expression string is available through documented scripting API on any `.aep` the
user's AE can open — including old versions, which AE upconverts on open. Direct parsing of
the RIFX container is reverse engineering against an undocumented, version-drifting binary
format and is therefore permanently second-class (§7).

All three routes converge on the same import pipeline: produce a Luminal project fragment,
run footage relink (§2.5), and present the import report (§9). Every import MUST produce an
import report; no route may fail silently or partially without saying so.

---

## 2. The Luminal Bridge

### 2.1 What it is

A free panel that runs **inside the user's After Effects** — ExtendScript/CEP at first
(widest version coverage), UXP later if Adobe's deprecation timeline forces it. Distributed
as a `.zxp` from the Luminal site and repository, licensed GPLv3 with the rest of the
project. The panel walks `app.project` via the scripting DOM and writes a **Luminal bundle**:
a folder (or zip) containing a versioned JSON document plus an optional footage collection.

The Bridge MUST NOT require Luminal to be installed on the same machine, and MUST NOT
require network access. It writes a bundle; Luminal opens bundles. A studio can export on
one machine and import on another.

### 2.2 The export walk

The panel traverses the project in this order. Everything listed is available from the
ExtendScript DOM and MUST be captured:

1. **Project items** — the folder tree, footage items (file path, interpretation: frame
   rate override, alpha interpretation, fields/pulldown flags, loop count, colour profile
   name), solids (colour, size), placeholders/missing footage (flagged), and compositions.
2. **Comp settings** — width, height, pixel aspect ratio, frame rate, duration, start
   timecode, background colour, motion blur settings ("shutter angle", "shutter phase",
   samples per frame, adaptive sample limit), renderer name (Classic 3D / Advanced 3D /
   CINEMA 4D — recorded verbatim; see matrix), "preserve frame rate/resolution when nested"
   flags.
3. **Layers**, per comp, in stacking order — type (footage, solid, precomp, text, shape,
   null, adjustment, camera, light, audio, guide), name, label colour, in/out points, start
   time, "stretch" percentage, parent reference, switches (visible, audible, solo, lock,
   shy, quality, motion blur, adjustment, 3D, collapse/continuously-rasterise, frame
   blending mode, guide-layer flag), blend mode, "preserve underlying transparency", and
   the matte reference: both the 23.0+ selectable form (`trackMatteType` + matte layer
   reference) and the legacy layer-above form, normalised to Luminal's matte model
   (any-layer reference + alpha/luma + inverted).
4. **Property groups, recursively** — every animatable property with its match name,
   display name, dimensionality, static value or keyframes, expression state, and for
   dimension-separated properties the per-dimension curves.
5. **Keyframes, exactly** — per key: time, value, per-side interpolation type (linear /
   bezier / continuous / auto / hold), temporal ease as `(speed, influence)` pairs per side
   per dimension (from `keyInTemporalEase`/`keyOutTemporalEase`; influence in AE's
   0.1–100 range), spatial in/out tangents where spatial, auto-bezier and roving flags.
   Because Luminal keyframe maths is AE-compatible (K-025), this is a value copy, not a
   conversion; nothing is resampled or baked.
6. **"Time Remap" keyframes** — exported as an ordinary keyframed property; the importer
   converts the curve to Retime as retime segments (the `MapSegment` records defined in
   [04-RETIMING.md](04-RETIMING.md)). Hold keys become freezes; layer bars extended beyond
   the last key become overrun holds; the layer's frame-blending switch maps to the
   frame-interpolation policy ("Frame Mix" → blend, "Pixel Motion" → flow, off → nearest).
   This mapping is exact: AE's "time remap" value graph and Luminal's Retime value graph are
   the same mathematical object.
7. **Masks** — path keyframes (vertices + tangents + closed flag), mode (add, subtract,
   intersect, lighten, darken, difference, none), feather (x/y), opacity, expansion,
   inverted. Variable-width feather points are captured where the DOM exposes them and
   flagged in the report where approximated.
8. **Expressions** — the source text of every enabled expression, verbatim, plus the
   enabled/disabled state. Never evaluated, never rewritten by the Bridge. Luminal decides
   at import time whether each expression can run (see matrix and
   [12-PLUGINS.md](12-PLUGINS.md) §4).
9. **Effects** — per instance: match name, display name, enabled state, and a full
   parameter dump (values or keyframes per parameter, using the same keyframe capture as
   §5; parameter match names included). The Bridge does not know which effects Luminal can
   map — it captures everything and lets the importer decide (§5, §6). Parameters whose
   values the DOM exposes only as opaque custom data are captured as raw data blobs and
   flagged.
10. **Text layers** — source text (including keyframed source text), character/paragraph
    styling per the DOM's text document object, path options, and animator groups with
    their selectors and animated properties. Animators import to the extent Luminal's text
    engine supports them (see matrix); the full structure is preserved in the bundle either
    way so later Luminal versions can re-import without re-exporting.
11. **Shape layers** — the full contents tree: groups, path primitives, bezier paths,
    fills/strokes/gradients (with dashes and taper), and path operations (Trim Paths,
    Repeater, Offset, Round Corners, Zig Zag, Wiggle Paths, Merge Paths…), each with its
    properties and keyframes.
12. **Cameras and lights** — one/two-node camera flag, point of interest, zoom, depth of
    field parameters; light type, intensity, colour, cone, falloff, shadow settings.
13. **Markers** — comp and layer markers with time, duration, comment.
14. **Footage collection (optional)** — when the user ticks "collect footage", the panel
    copies referenced media into the bundle's `footage/` folder and records both original
    and collected relative paths, plus file size and a fast content hash for relink
    verification. Collection MUST be opt-in; bundles default to paths only.

The walk MUST be resilient: a property the panel cannot read is recorded as an
`unreadable` entry with its match name and the ExtendScript error, and the walk continues.
One broken property never aborts an export.

### 2.3 The bundle format

A bundle is a folder or zip:

```
MyProject.lum-bundle/
  manifest.json        # bundle schema version, AE version, Bridge version, export date
  project.json         # the exported project fragment
  footage/             # optional collected media
  report.json          # per-item outcomes the Bridge itself already knows (unreadables)
```

`project.json` is expressed **in the Luminal project schema** defined in
[10-FILE-FORMAT.md](10-FILE-FORMAT.md), extended with an `ae` namespace for
AE-only carry-through data (match names, unmapped parameters, raw blobs, renderer names).
There is deliberately no separate interchange dialect to maintain: the Bridge emits what a
`.lum` file contains, plus annotations. The `ae` namespace is preserved on load, save,
and round-trip — Luminal never strips what it does not understand (§6).

`manifest.json` carries a semver bundle version. Luminal MUST refuse bundles with a newer
major version (with a "please update Luminal" message) and MUST accept older ones via
migration, same policy as `.lum` files.

### 2.4 What the DOM does not expose

The scripting DOM has genuine holes, and the Bridge inherits them: "Roto Brush" strokes and
spans, "Puppet" pin meshes (pins are readable, the mesh is not), paint strokes' full brush
state, per-character 3D animator internals beyond the documented properties, and
third-party effect parameters that the vendor exposes only as custom data. These are
placeholder or unsupported rows in the matrix (§4) regardless of route.

### 2.5 Footage relink on import

On opening a bundle (or a parsed `.aep`), Luminal resolves footage in this order: collected
`footage/` copy (hash-verified) → original absolute path → original path re-rooted against
the bundle's location → user-directed search folder (recursive, matched by filename then
verified by hash/size where available). Unresolved items import as offline footage items
with full interpretation settings intact, listed in the import report; relinking later is
the standard relink flow from [10-FILE-FORMAT.md](10-FILE-FORMAT.md). Import never blocks
on missing media.

---

## 3. Mapping semantics

The load-bearing conversions, stated once:

- **Keyframes** are copied value-for-value (K-025): interpolation types, per-side speed and
  influence, spatial tangents, roving flags. Luminal's evaluator reproduces AE's cubic
  bezier in (time, value) space, so imported curves evaluate identically.
- **"Time remapping" → Retime**, losslessly, per §2.2 item 6. AE's "time stretch" maps to
  Luminal's Stretch, including its keyframe-rescaling behaviour and negative (reversed)
  values.
- **"Track mattes" → mattes.** Both AE generations normalise to Luminal's model (chosen
  layer + alpha/luma + inverted). Legacy above-layer mattes get the matte layer's video
  switch state preserved. "Preserve underlying transparency" maps directly.
- **Layer order, parenting, adjustment layers, guide layers, solids, nulls** map 1:1.
- **Collapse**: AE's "collapse transformations / continuously rasterize" switch maps to
  Luminal's collapse switch on Precomp layers; the render-order consequences match because
  Luminal's compositor implements the same semantics
  ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)).
- **3D** maps onto Luminal's 2.5D (K-023): 3D layer flags, cameras (both node types),
  lights, material options. Comps using AE's "Advanced 3D" or "CINEMA 4D" renderers import
  with geometry-dependent features flagged (see matrix).
- **Colour**: AE projects are 8/16/32-bpc project-wide with an optional linearised working
  space; Luminal is scene-linear fp16/fp32 per comp (K-026). Values convert on import;
  comps that relied on non-linear 8-bpc blending arithmetic are flagged mapped-with-
  differences because blend results can shift subtly.

---

## 4. The fidelity matrix

The centrepiece. Four grades:

- **lossless** — evaluates identically in Luminal; no visual difference by construction.
- **mapped** — mapped with documented differences; the report says what changed.
- **placeholder** — imported as an inert node preserving all data (§6); renders as identity.
- **unsupported** — cannot be represented; skipped, counted, and named in the report.

| AE feature | Grade | Notes |
|---|---|---|
| Project folder tree, footage items, interpretation | lossless | Loop count, alpha mode, fps override all carried |
| Comp settings (size, fps, duration, background) | lossless | |
| Layer stack, in/out, start, label, switches | lossless | |
| Transforms + temporal/spatial keyframes | lossless | K-025; includes hold, roving, separated dimensions |
| Parenting | lossless | |
| "Time stretch" | lossless | → Stretch, including keyframe rescale and negative values |
| "Time remapping" | lossless | → Retime segments; hold keys → freezes; extended bars → overrun |
| Frame blending ("Frame Mix"/"Pixel Motion") | mapped | → blend/flow frame interpolation; flow output differs (different optical-flow engine) |
| Masks (path, feather, opacity, expansion, modes) | lossless | Variable-width feather: mapped — approximated until Luminal ships it |
| "Track mattes" (legacy + 23.0 selectable) | lossless | → matte |
| Blend modes — standard 25 | lossless | Normal, Darken, Multiply, Color Burn, Linear Burn, Darker Color, Add, Lighten, Screen, Color Dodge, Linear Dodge, Lighter Color, Overlay, Soft/Hard/Linear/Vivid/Pin Light, Hard Mix, Difference, Exclusion, Subtract, Divide, Hue, Saturation, Color, Luminosity |
| Blend modes — "Classic" variants | mapped | Import as the modern counterpart; AE 4.x maths not reproduced |
| Blend modes — Dissolve, Dancing Dissolve | mapped | Import as Normal, flagged; no Luminal equivalent yet |
| Blend modes — Stencil/Silhouette (×4), Alpha Add, Luminescent Premul | mapped | Import as Normal, flagged; candidates for [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) additions |
| "Preserve underlying transparency" | lossless | |
| Adjustment layers | lossless | |
| Precomps + collapse | lossless | Collapse edge cases (effect-on-collapsed-layer) follow Luminal's compositor; flagged when AE would have broken collapse |
| Solids, nulls, guide layers | lossless | |
| Markers (comp + layer) | lossless | |
| AE built-in effects with a Luminal equivalent | mapped | Via the match-name table (§5); parameters and keyframes carried; pixel output near-identical, not bit-identical |
| AE built-in effects without an equivalent | placeholder | Full parameter dump preserved; §6 |
| Third-party effects (Twixtor, RSMB, Sapphire, Deep Glow, …) | placeholder | Always — internals never map (K-060). The user MAY apply the vendor's OFX build manually alongside; parameters do not transfer automatically because AE and OFX builds do not share parameter layouts |
| Expressions | mapped | Imported as source text; run when they use only the implemented API subset ([12-PLUGINS.md](12-PLUGINS.md) §4), else disabled with a badge and listed in the report |
| Text layers — source text + styling | mapped | Font fallback differences possible; missing fonts flagged |
| Text animators + range selectors | mapped | Core animators map; unsupported selector modes become placeholders on the animator |
| Shape layers — paths, fills, strokes, groups | lossless | |
| Shape path operations | mapped | Trim Paths, Repeater, Offset, Round Corners, Zig Zag map; Merge Paths modes and Wiggle Paths flagged where semantics differ |
| Cameras, lights, 3D layer flags | mapped | → Luminal 2.5D (K-023); depth of field mapped; renderer-specific shading differs |
| "Advanced 3D" / "CINEMA 4D" renderer features (extrusion, 3D models, environment lights) | unsupported | Layers import flat with a report entry; C4D-renderer comps flagged prominently |
| Layer styles | placeholder | Until Luminal ships equivalents |
| "Roto Brush" / "Refine Edge" strokes | unsupported | DOM does not expose strokes; the effect imports as a placeholder so the layer keeps its slot |
| "Puppet" pins | unsupported | v1; pin data preserved in the `ae` namespace for a future engine |
| Paint / Clone strokes | unsupported | |
| "Essential Graphics" rigs | mapped | Exposed properties import as plain properties; the rig/template structure does not |
| Audio layers + level keyframes | lossless | Per [09-AUDIO.md](09-AUDIO.md) |
| Render queue / output modules | unsupported | Deliberate; Luminal's export queue is its own thing |

A comp made of transforms, keyframes, masks, mattes, standard blend modes, retiming, and
mapped effects — which describes the overwhelming majority of montage projects and CC packs
— imports lossless-or-mapped end to end.

---

## 5. The effect mapping table

Luminal maintains a versioned data file (`ae-effect-map.toml`, shipped with the app and
updatable independently) mapping AE effect **match names** to Luminal built-in effects
([08-EFFECTS.md](08-EFFECTS.md)) with per-parameter correspondence and unit/range
conversion. Seeded with the montage staples:

| AE effect (match name) | Luminal effect |
|---|---|
| "Gaussian Blur" (`ADBE Gaussian Blur 2`) | Blur |
| "Directional Blur" (`ADBE Motion Blur`) | Directional blur |
| "Radial Blur" (`ADBE Radial Blur`) | Radial blur |
| "Glow" (`ADBE Glo2`) | Glow (exposure-aware; output brighter-cleaner — mapped, not lossless) |
| "Curves" (`ADBE CurvesCustom`) | Curves |
| "Hue/Saturation" (`ADBE HUE SATURATION`) | Hue/saturation |
| "Brightness & Contrast" (`ADBE Brightness & Contrast 2`) | Brightness/contrast |
| "Tint" (`ADBE Tint`) | Tint |
| "Fill" (`ADBE Fill`) | Fill |
| "Transform" (`ADBE Geometry2`) | Transform effect |
| "Motion Tile" (`ADBE Tile`) | Tile |
| "Mirror" (`ADBE Mirror`) | Mirror |
| "Optics Compensation" (`ADBE Optics Compensation`) | Lens distort |
| "Turbulent Displace" (`ADBE Turbulent Displace`) | Turbulent displace |
| "Fractal Noise" (`ADBE Fractal Noise`) | Fractal noise |
| "Echo" (`ADBE Echo`) | Echo |
| "Posterize Time" (`ADBE Posterize Time`) | Posterise time |
| "Timewarp" (`ADBE Timewarp`) | Placeholder + report suggestion to use Retime with flow interpolation |

Match names in this table MUST be verified against a live AE instance during
implementation; the table above is the seed list, not the audit. Every mapped conversion
MUST have a golden-frame test: the AE-rendered frame and the Luminal-rendered frame of a
reference comp compared within a stated tolerance. Unmapped match names fall through to
placeholders — never to the closest guess.

---

## 6. Placeholder behaviour

A placeholder is an inert effect node that:

- keeps the original display name, match name, enabled state, and the **complete parameter
  dump including keyframes and expressions** (parameters are real Luminal properties: they
  animate, appear in the graph editor, and are expression-readable, they just drive
  nothing);
- renders as **identity** — input passed through unchanged;
- shows a subtle badge in Effect Controls ("not rendered — imported from After Effects"),
  in the calm style of [15-DESIGN.md](15-DESIGN.md) — no red, no warning triangle
  theatrics;
- is **never lost**: saving a `.lum` project preserves placeholders and their `ae`
  namespace data byte-for-byte, so a project can be opened, edited around, saved, and the
  placeholder data survives indefinitely. If a later Luminal version (or an installed OFX/
  KFX effect registered as an upgrade target in `ae-effect-map.toml`) gains a mapping, the
  user is offered — never forced — a per-instance upgrade.

The same mechanism serves missing OFX/KFX plugins at project-open time
([12-PLUGINS.md](12-PLUGINS.md) §1), so "placeholder" is one concept everywhere.

---

## 7. Direct `.aep` parsing

For users without After Effects. Honest scope: **recover what we can**.

`.aep` is a RIFX container (RIFF, big-endian sizes, form type `Egg!`) of nested LIST chunks.
Chunk shapes are publicly known; many field semantics are not, and Adobe changes details
across versions without documentation. Luminal builds on the community reverse-engineering
work — the Kaitai Struct grammar from `forticheprod/aep_parser` (the most complete public
description, maintained for pipeline introspection) and `boltframe/aftereffects-aep-parser`
(Go, explicitly partial) — reimplemented in Rust inside `luminal-project`, with licence
compliance checked before vendoring any grammar.

Realistically recoverable: the project item tree and folder structure, footage paths and
basic interpretation, comp settings, layer stacks with names/types/in-out/start/order,
blend mode and switch flags, basic transform values, and a useful subset of keyframe data.
Progressively unreliable to unrecoverable: full temporal ease semantics across all property
classes, expressions storage, mask feather detail, text and shape contents, effect
parameter blobs (typed per match name, third-party blobs opaque).

Policy:

- Direct parse results MUST be labelled "structure only" in the UI and the import report,
  and the report MUST open automatically after a direct parse.
- Anything ambiguous imports as a placeholder or as a static value with a report entry —
  the parser MUST NOT guess silently.
- A parse failure on one chunk skips that chunk and continues; the report lists skipped
  chunks. Whole-file failure falls back to "import footage references only" where the
  footage table is readable.
- New AE versions MAY break the parser at any time; this is stated in the UI copy. The
  Bridge remains the answer for fidelity.

`.aepx` (AE's XML save) is the same data with the interesting chunks hex-encoded; it MAY
share the parser back-end but is not a separate fidelity route.

---

## 8. Lottie import

Cheap extra on-ramp: bodymovin/Lottie JSON is documented, has `lottie-web` as a reference
implementation, and a large template ecosystem. Luminal imports Lottie shape/text/image/
precomp layers, transforms, and keyframes into ordinary comps — Lottie easing converts
exactly (it is the same bezier model, normalised). Features outside Lottie's scope simply
do not arrive, and Lottie features Luminal lacks (specific layer effects) follow the same
placeholder rules. The Lottie importer doubles as a continuous validation of the bundle
importer's schema handling. Not a priority beyond that; it ships when it is nearly free.

---

## 9. The import report

Every import ends with the report — a panel listing per-item outcomes, in the calm voice of
[15-DESIGN.md](15-DESIGN.md):

- **Summary line**: "212 items imported · 14 adjusted · 6 placeholders · 2 skipped".
- **Grouped detail**, filterable by outcome: each row names the item (comp → layer →
  property path), the outcome (imported / adjusted / placeholder / skipped), and a
  one-line reason ("blend mode Dissolve has no equivalent — imported as Normal";
  "Twixtor Pro imported as placeholder — the OFX version can be applied manually").
- **Navigation**: double-clicking a row selects the item in the Project panel or Timeline.
- **Persistence**: the report is stored in the project (`ae` namespace) and reopenable from
  the File menu; it is also written next to the bundle as `import-report.json` for tooling.
- Expressions disabled at import are their own filter, so a user can work through them.

The report is informative, never blocking: import always completes, the project always
opens.

---

## 10. Non-goals

- **Loading `.aex` AE plugins — never.** Modern AE plugins depend on SmartFX, AEGP suites,
  and Adobe GPU internals that no third-party host can honestly re-implement, and the few
  shipping attempts (Grass Valley's EDIUS bridge) support only a hit-and-miss subset.
  Adobe's SDK explicitly states it neither supports nor recommends third-party hosts, and
  the SDK licence plus plugin vendors' host-locked activation make the legal exposure real
  while the same vendors already ship OFX builds. Luminal routes all plugin demand through
  OFX and KFX ([12-PLUGINS.md](12-PLUGINS.md)) — see K-061.
- **`.ffx` preset files.** Closed RIFX-family binary, no complete public parser, version-
  drifting. Mitigation: apply the preset inside AE and export with the Bridge — presets
  become ordinary properties. Native `.ffx` reading MAY be revisited if the direct-parse
  property decoder matures (§7 shares the problem).
- **Guaranteed visual parity of any comp using unmapped effects.** The matrix is the
  contract; placeholders are the mechanism; the report is the disclosure. Luminal does not
  promise that an arbitrary AE project renders identically, and the UI copy never implies
  it.

---

## Open questions

- **CEP end-of-life**: Adobe's UXP migration timeline for After Effects panels is unclear;
  when AE drops CEP, the Bridge needs a UXP port. Does UXP's scripting DOM expose the full
  keyframe surface (`keyInTemporalEase` et al.) — audit before committing the port.
- **Match-name audit**: the §5 seed table's match names need verification against a live AE
  install, and the golden-frame tolerance per mapped effect needs defining alongside
  [08-EFFECTS.md](08-EFFECTS.md).
- **Blend-mode gap**: do Stencil/Silhouette, Alpha Add, and Luminescent Premul earn real
  Luminal implementations (upgrading four matrix rows to lossless), and where do they sit in
  [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)'s compositing order?
- **Text animator scope**: which selector modes and per-character-3D features Luminal's text
  engine will support decides several matrix rows; blocked on the text spec in
  [03-DATA-MODEL.md](03-DATA-MODEL.md).
- **Bundle size**: property-heavy projects (thousands of keyframed masks) may produce very
  large `project.json` files; decide a compression policy (zip member compression is
  probably enough) with real-world CC-pack samples.
- **Kaitai grammar licence**: confirm `forticheprod/aep_parser`'s licence is compatible with
  GPLv3 vendoring, or reimplement from the published chunk documentation.
