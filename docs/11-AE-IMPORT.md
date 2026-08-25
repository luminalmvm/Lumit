# After Effects import

**Status: the conversion and the surface are built, and the golden bundle is in.**
The Bridge walker (`tools/ae-bridge/`), the bundle reader and the whole
capture → `Document` mapping — items, comps, layers, keyframes, masks, mattes, retime,
the §5 effect table and the §9 report — live in `crates/lumit-import/`
(docs/impl/ae-import.md §6 phases 1 and 2). **Phase 3 is built too, and since K-418 it
takes both routes**: File ▸ Import ▸ After Effects project… opens a *file* chooser for
the `.aep` itself, File ▸ Import ▸ Bridge bundle folder… opens the folder chooser for a
Bridge bundle, and both call the one `LumitBridgeState::import_ae_bundle`, which decides
from the bytes which reader to use (§7), maps what it read and adopts the document the way
opening a `.lum` does. Footage is looked for through the same `resolve_all_media` flow
(§2.5's absolute-path step only, for the reason §2.5 now gives), and the report window (§9)
lists what changed whichever route was taken. `make-fixture.jsx` **has** now been run
on a live After Effects 26.0: `tools/ae-bridge/fixtures/fixture.lum-bundle/` is the
golden bundle, and `crates/lumit-import/tests/golden.rs` asserts every §5 checklist
row through the mapped document, alongside the hand-written bundles that document the
schema and the awkward half. One thing §5 asks for is still owed — the golden-frame
tests of every mapped conversion, which need After Effects *renders* of the fixture —
along with two checklist rows the sitting showed do not come across (a roving key and a
3D layer's Orientation), and §9's row navigation and its persistence in the project
(docs/TODO.md). This document implements K-060 (import strategy), and leans on
K-025 (AE-compatible keyframe maths) and K-021 (Retime) in [02-DECISIONS.md](02-DECISIONS.md). Terminology follows
[01-GLOSSARY.md](01-GLOSSARY.md) exactly; After Effects' own feature names appear in quotes
when describing AE itself. RFC-2119 keywords (MUST, SHOULD, MAY) are binding.

The target user is migrating *from* After Effects, often mid-project, often with a folder of
community project files ("CC packs") they depend on. Import exists to make that migration
undramatic: what can carry over carries over exactly; what cannot is preserved, labelled, and
never silently dropped.

---

## 1. Strategy overview

Three routes (K-060, priority revised by K-418):

| Route | Requires | Fidelity | Status in UI |
|---|---|---|---|
| **Direct `.aep` parsing** (primary, K-418) | Nothing — pick the `.aep` | Measured per category against AE's own account of the golden fixture; anything unrecovered is a report row, never a guess | "Imported from After Effects" (+ per-item report rows for anything unrecovered) |
| **Lumit Bridge** (fidelity backstop + verification harness) | After Effects installed, any recent version | High — the scripting DOM is documented public API | "Imported from After Effects" |
| **Lottie / bodymovin JSON** (tertiary) | A `.json` export | High within Lottie's own scope | "Imported from Lottie" |

K-418's ruling: the user-facing front door is the `.aep` itself — seamless, no script
sitting — and the parser is a second *front end* to the same capture the Bridge
writes, so both routes share one mapping, one effect table, one report. The Bridge
remains first-class forever: it is the route that cannot drift with AE versions, the
export path for machines without Lumit, and the source of the golden fixtures the
parser is measured against.

The Bridge is the proven pattern (bodymovin/Lottie walks the same DOM in the other
direction): let After Effects itself be the parser. Every keyframe, easing handle, mask
path, and expression string is available through documented scripting API on any `.aep` the
user's AE can open — including old versions, which AE upconverts on open. Direct parsing of
the RIFX container is reverse engineering against an undocumented, version-drifting binary
format, and that has not changed: K-418 made it the front door because it is seamless and
because every claim it makes is measured against the Bridge's own account of the golden
fixture, not because the format became trustworthy. So it is the route that may break on a
new After Effects, and the Bridge is the one that cannot (§7).

All three routes converge on the same import pipeline: produce a Lumit project fragment,
run footage relink (§2.5), and present the import report (§9). Every import MUST produce an
import report; no route may fail silently or partially without saying so.

---

## 2. The Lumit Bridge

### 2.1 What it is

A free panel that runs **inside the user's After Effects** — ExtendScript/CEP at first
(widest version coverage), UXP later if Adobe's deprecation timeline forces it. Distributed
as a `.zxp` from the Lumit site and repository, licensed GPLv3 with the rest of the
project. The panel walks `app.project` via the scripting DOM and writes a **Lumit bundle**:
a folder (or zip) containing a versioned JSON document plus an optional footage collection.

The Bridge MUST NOT require Lumit to be installed on the same machine, and MUST NOT
require network access. It writes a bundle; Lumit opens bundles. A studio can export on
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
   reference) and the legacy layer-above form, normalised to Lumit's matte model
   (any-layer reference + alpha/luma + inverted).
4. **Property groups, recursively** — every animatable property with its match name,
   display name, dimensionality, static value or keyframes, expression state, and for
   dimension-separated properties the per-dimension curves.
5. **Keyframes, exactly** — per key: time, value, per-side interpolation type (linear /
   bezier / continuous / auto / hold), temporal ease as `(speed, influence)` pairs per side
   per dimension (from `keyInTemporalEase`/`keyOutTemporalEase`; influence in AE's
   0.1–100 range), spatial in/out tangents where spatial, auto-bezier and roving flags.
   Because Lumit keyframe maths is AE-compatible (K-025), this is a value copy, not a
   conversion; nothing is resampled or baked.
6. **"Time Remap" keyframes** — exported as an ordinary keyframed property; the importer
   converts the curve to Retime as retime segments (the `MapSegment` records defined in
   [04-RETIMING.md](04-RETIMING.md)). Hold keys become freezes; layer bars extended beyond
   the last key become overrun holds; the layer's frame-blending switch maps to the
   frame-interpolation policy ("Frame Mix" → blend, "Pixel Motion" → flow, off → nearest).
   This mapping is exact: AE's "time remap" value graph and Lumit's Retime value graph are
   the same mathematical object.
7. **Masks** — path keyframes (vertices + tangents + closed flag), mode (add, subtract,
   intersect, lighten, darken, difference, none), feather (x/y), opacity, expansion,
   inverted. Variable-width feather points are captured where the DOM exposes them and
   flagged in the report where approximated.
8. **Expressions** — the source text of every enabled expression, verbatim, plus the
   enabled/disabled state. Never evaluated, never rewritten by the Bridge. Lumit decides
   at import time whether each expression can run (see matrix and
   [12-PLUGINS.md](12-PLUGINS.md) §4).
9. **Effects** — per instance: match name, display name, enabled state, and a full
   parameter dump (values or keyframes per parameter, using the same keyframe capture as
   §5; parameter match names included). The Bridge does not know which effects Lumit can
   map — it captures everything and lets the importer decide (§5, §6). Parameters whose
   values the DOM exposes only as opaque custom data are captured as raw data blobs and
   flagged.
10. **Text layers** — source text (including keyframed source text), character/paragraph
    styling per the DOM's text document object, path options, and animator groups with
    their selectors and animated properties. Animators import to the extent Lumit's text
    engine supports them (see matrix); the full structure is preserved in the bundle either
    way so later Lumit versions can re-import without re-exporting.
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
  capture.json         # the faithful AE-shaped capture of the walk (§2.2)
  footage/             # optional collected media
  report.json          # per-item outcomes the Bridge itself already knows (unreadables)
```

`capture.json` is a **faithful, AE-shaped record of the walk** (K-410, superseding the
Lumit-schema `project.json` this section first specified): item ids are AE's, times are
the DOM's float seconds, property trees keep their match names, and nothing is converted.
Every conversion — rational time, ids, keyframe carriage, effect mapping — happens in
Rust, in `lumit-import`, where the regression suite covers it; the Bridge is a walker
with one try/catch per property and no opinions, which is also what survives Adobe's
version drift. The capture schema is versioned in `manifest.json` and owned by
[impl/ae-import.md](impl/ae-import.md). The importer's output is an ordinary Lumit
document; AE-only carry-through data (match names, unmapped parameters, renderer names)
lands in that document's `ae` namespace and is preserved on load, save, and round-trip —
Lumit never strips what it does not understand (§6).

`manifest.json` carries a semver bundle version. Lumit MUST refuse bundles with a newer
major version (with a "please update Lumit" message) and MUST accept older ones via
migration, same policy as `.lum` files.

### 2.4 What the DOM does not expose

The scripting DOM has genuine holes, and the Bridge inherits them: "Roto Brush" strokes and
spans, "Puppet" pin meshes (pins are readable, the mesh is not), paint strokes' full brush
state, per-character 3D animator internals beyond the documented properties, and
third-party effect parameters that the vendor exposes only as custom data. These are
placeholder or unsupported rows in the matrix (§4) regardless of route.

### 2.5 Footage relink on import

On opening a bundle (or a parsed `.aep`), Lumit resolves footage in this order: collected
`footage/` copy (hash-verified) → original absolute path → original path re-rooted against
the bundle's location → user-directed search folder (recursive, matched by filename then
verified by hash/size where available). Unresolved items import as offline footage items
with full interpretation settings intact, listed in the import report; relinking later is
the standard relink flow from [10-FILE-FORMAT.md](10-FILE-FORMAT.md). Import never blocks
on missing media.

**Today only the second step runs** (docs/TODO.md). There is no collected `footage/` to
check, so a file is found where After Effects recorded it and nowhere else: the capture
carries an absolute path, and an absolute path re-rooted against the folder the bundle or
the `.aep` sits in is still itself. An import on the machine the project was made on
therefore relinks everything; one carried to another machine imports every item offline,
with a row apiece — which is the promised outcome, reached by fewer routes than this list.

---

## 3. Mapping semantics

The load-bearing conversions, stated once:

- **Keyframes** are copied value-for-value (K-025): interpolation types, per-side speed and
  influence, spatial tangents, roving flags. Lumit's evaluator reproduces AE's cubic
  bezier in (time, value) space, so imported curves evaluate identically.
- **"Time remapping" → Retime**, losslessly, per §2.2 item 6. AE's "time stretch" maps to
  Lumit's Stretch, including its keyframe-rescaling behaviour and negative (reversed)
  values — **until the engine has a Stretch field, a stretched layer imports as the
  equivalent Retime and the report says so**: the two describe the same mapping from
  layer time to source time, and the keyframe rescale needs nothing done to it because
  the DOM already reports key times on the composition's clock, after the stretch.
- **"Track mattes" → mattes.** Both AE generations normalise to Lumit's model (chosen
  layer + alpha/luma + inverted). Legacy above-layer mattes get the matte layer's video
  switch state preserved. "Preserve underlying transparency" maps directly.
- **Layer order, parenting, adjustment layers, guide layers, solids, nulls** map 1:1.
- **Collapse**: AE's "collapse transformations / continuously rasterize" switch maps to
  Lumit's collapse switch on Precomp layers; the render-order consequences match because
  Lumit's compositor implements the same semantics
  ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)).
- **3D** maps onto Lumit's 2.5D (K-023): 3D layer flags, cameras (both node types),
  lights, material options. Comps using AE's "Advanced 3D" or "CINEMA 4D" renderers import
  with geometry-dependent features flagged (see matrix).
- **Colour**: AE projects are 8/16/32-bpc project-wide with an optional linearised working
  space; Lumit is scene-linear fp16/fp32 per comp (K-026). Values convert on import;
  comps that relied on non-linear 8-bpc blending arithmetic are flagged mapped-with-
  differences because blend results can shift subtly.

---

## 4. The fidelity matrix

The centrepiece. Four grades:

- **lossless** — evaluates identically in Lumit; no visual difference by construction.
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
| "Time stretch" | mapped | → the equivalent Retime while the engine has no Stretch field, including negative (reversed) values; reported, and lossless in what it evaluates to |
| "Time remapping" | lossless | → Retime segments; hold keys → freezes; extended bars → overrun |
| Frame blending ("Frame Mix"/"Pixel Motion") | mapped | → blend/flow frame interpolation; flow output differs (different optical-flow engine) |
| Masks (path, feather, opacity, expansion, modes) | lossless | Variable-width feather: mapped — approximated until Lumit ships it. Two exceptions, both reported: the **Lighten** and **Darken** modes are not built ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §2) and import as Add, and AE's separate x/y feather averages into Lumit's single width. A **RotoBezier** mask's tangents are computed by AE rather than stored, so it imports as the polygon through its vertices |
| "Track mattes" (legacy + 23.0 selectable) | lossless | → matte |
| Blend modes — standard 25 | lossless | Normal, Darken, Multiply, Color Burn, Linear Burn, Darker Color, Add, Lighten, Screen, Color Dodge, Linear Dodge, Lighter Color, Overlay, Soft/Hard/Linear/Vivid/Pin Light, Hard Mix, Difference, Exclusion, Subtract, Divide, Hue, Saturation, Color, Luminosity |
| Blend modes — "Classic" variants | mapped | Import as the modern counterpart; AE 4.x maths not reproduced |
| Blend modes — Dissolve, Dancing Dissolve | mapped | Import as Normal, flagged; no Lumit equivalent yet |
| Blend modes — Stencil/Silhouette (×4), Alpha Add, Luminescent Premul | mapped | Import as Normal, flagged; candidates for [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) additions |
| "Preserve underlying transparency" | lossless | |
| Adjustment layers | lossless | |
| Precomps + collapse | lossless | Collapse edge cases (effect-on-collapsed-layer) follow Lumit's compositor; flagged when AE would have broken collapse |
| Solids, nulls, guide layers | lossless | |
| Markers (comp + layer) | lossless | |
| AE built-in effects with a Lumit equivalent | mapped | Via the match-name table (§5); parameters and keyframes carried; pixel output near-identical, not bit-identical |
| AE built-in effects without an equivalent | placeholder | Full parameter dump preserved; §6 |
| Third-party effects (Twixtor, RSMB, Sapphire, Deep Glow, …) | placeholder | Always — internals never map (K-060). The user MAY apply the vendor's OFX build manually alongside; parameters do not transfer automatically because AE and OFX builds do not share parameter layouts |
| Expressions | mapped | Imported as source text; run when they use only the implemented API subset ([12-PLUGINS.md](12-PLUGINS.md) §4), else disabled with a badge and listed in the report |
| Text layers — source text + styling | mapped | Font fallback differences possible; missing fonts flagged |
| Text animators + range selectors | mapped | Core animators map; unsupported selector modes become placeholders on the animator |
| Shape layers — paths, fills, strokes, groups | lossless | |
| Shape path operations | mapped | Trim Paths, Repeater, Offset, Round Corners, Zig Zag map; Merge Paths modes and Wiggle Paths flagged where semantics differ |
| Cameras, lights, 3D layer flags | mapped | → Lumit 2.5D (K-023); depth of field mapped; renderer-specific shading differs |
| "Advanced 3D" / "CINEMA 4D" renderer features (extrusion, 3D models, environment lights) | unsupported | Layers import flat with a report entry; C4D-renderer comps flagged prominently |
| Layer styles | placeholder | Until Lumit ships equivalents |
| "Roto Brush" / "Refine Edge" strokes | unsupported | DOM does not expose strokes; the effect imports as a placeholder so the layer keeps its slot |
| "Puppet" pins | unsupported | v1; pin data preserved in the `ae` namespace for a future engine |
| Paint / Clone strokes | unsupported | |
| "Essential Graphics" rigs | mapped | Exposed properties import as plain properties; the rig/template structure does not |
| Audio layers + level keyframes | lossless | Per [09-AUDIO.md](09-AUDIO.md) |
| Render queue / output modules | unsupported | Deliberate; Lumit's export queue is its own thing |

A comp made of transforms, keyframes, masks, mattes, standard blend modes, retiming, and
mapped effects — which describes the overwhelming majority of montage projects and CC packs
— imports lossless-or-mapped end to end.

---

## 5. The effect mapping table

Lumit maintains a versioned data file (`ae-effect-map.toml`, shipped with the app and
updatable independently) mapping AE effect **match names** to Lumit built-in effects
([08-EFFECTS.md](08-EFFECTS.md)) with per-parameter correspondence and unit/range
conversion. Seeded with the montage staples:

| AE effect (match name) | Lumit effect |
|---|---|
| "Gaussian Blur" (`ADBE Gaussian Blur 2`) | **Gaussian blur** (built, docs/08 §3.8) — mapped: Blurriness is Lumit's Radius and carries the whole look, AE's raster pixels being Lumit's px@comp (§2.3, K-419) — the number is the same. AE's Blur Dimensions (Horizontal / Vertical / both) and its Repeat Edge Pixels have no counterpart — Lumit's Gaussian blurs on both axes and its edge policy is the fixed Repeat K-137 settled on — and both are reported |
| "Directional Blur" (`ADBE Motion Blur`) | **Directional blur** (built, docs/08 §3.9) — direct on both controls: Direction is Lumit's Angle and carries AE's convention (degrees from straight up, clockwise) so the number converts unchanged. Blur Length is pixels on both sides (px@comp in Lumit, §2.3), so it converts unchanged too |
| "Radial Blur" (`ADBE Radial Blur`) | **Radial blur** (built, docs/08 §3.10) — mapped: Amount, Center and the two Types (Spin and Zoom) all have counterparts. One conversion and one report. Amount is pixels on both sides (px@comp, §2.3); **Center converts to Lumit's per cent of the frame**, each axis dividing through its own dimension. AE's Antialiasing (Best Quality) and its Random Seed have no counterpart and are reported — Lumit resamples the same way at every quality (§2.3) and its radial blur is not seeded |
| "Glow" (`ADBE Glo2`) | **Glow** (built, docs/08 §3.28) — mapped: Glow Threshold, Glow Radius and Glow Intensity are the three controls that carry the look, and the radius and the intensity convert unchanged. **Threshold converts out of eight bits into light** — AE's is a display value (its 60% default arrives as 153) and Lumit's is scene-linear, so the conversion is the sRGB transfer function, which is not a straight line: the value on every keyframe is exact and any *handle* on one is left in the old units and reported. The output is brighter and cleaner than AE's because the halo is built from real light rather than from an eight-bit copy, which is reported. AE's Glow Based On, Glow Operation, Glow Dimensions, Composite Original and **the whole Glow Colors group** — the A and B colours, the looping, the phase and the midpoint — have no counterpart and are reported: Lumit's glow tints the halo with one colour and does not ramp it |
| "Curves" (`ADBE CurvesCustom`) | **Curves** (built, docs/08 §3.30) — the mapping is defined (AE's point list **carried whole**, K-412: both sides now store a curve as its own control points, so the conversion is a copy of at most sixteen pairs per channel with nothing sampled away) but **via the Bridge the instance imports as a placeholder**: the point list is a `CUSTOM_VALUE` blob AE's own scripting DOM cannot read (K-410; the 2026-08-20 audit confirms `ADBE CurvesCustom-0001` is the one property with no readable value). The mapping arms the day a blob decoder lands (§7 shares the problem) |
| "Levels" (`ADBE Easy Levels2`) | **Levels** (built, docs/08 §3.31) — mapped: AE clamps at the input white, Lumit carries highlights on (scene-linear, §2.1). The scripting DOM exposes **one** set of five numbers plus the Channel picker naming which channel they belong to, so the import writes that channel's group (RGB → Master) and leaves the others neutral; an instance on the Alpha channel is reported, Lumit's Levels having no alpha lane. AE's Clip To Output Black and Clip To Output White have no counterpart and are reported, as §3.36's "Clip result values" is |
| "Hue/Saturation" (`ADBE HUE SATURATION`) | **Hue and saturation** (built, docs/08 §3.33) — mapped: master and the six ranges convert directly; range weights are saturation-scaled, and **Colorize** has no equivalent yet, so a colourised instance imports as a **placeholder** and says so. As with Levels, the DOM exposes one set of three sliders plus the Channel Control picker naming the range they belong to, so the import writes that range's group and leaves the rest neutral. AE's Channel Range is a `CUSTOM_VALUE` blob and is reported |
| "Brightness & Contrast" (`ADBE Brightness & Contrast 2`) | **Brightness** (built, docs/08 §3.32) — one effect carrying both sliders under AE's names and AE's neutral point (K-397) |
| "Tint" (`ADBE Tint`) | **Tint** (built, docs/08 §3.23) — direct: Map Black To and Map White To are Lumit's two colours and cross into scene-linear light like every other imported colour (§3), and **Amount to Tint is Mix**, the same per cent under a different name |
| "Fill" (`ADBE Fill`) | **Fill** (built, docs/08 §3.34) — exact for a whole-alpha fill; AE's Opacity maps to Mix, and a *mask-targeted* Fill (with its Invert and the two Feather controls) reports rather than approximates |
| "Gradient Ramp" (`ADBE Ramp`) | **Gradient** (built, docs/08 §3.35) — direct: the two points, the two colours, Ramp Scatter and "Blend with original" all have counterparts. Mapped in one respect: Lumit interpolates in scene-linear light (§2.1), so a long ramp's midpoint sits where the light says rather than where AE's display range put it |
| "Noise" (`ADBE Noise`) | **Noise** (built, docs/08 §3.36) — Amount and "Use colour noise" map directly; AE's "Clip result values" has no counterpart, because scene-linear has headroom and nothing needs clipping (reported as a mapped conversion) |
| "Transform" (`ADBE Geometry2`) | Transform effect |
| "Motion Tile" (`ADBE Tile`) | **Tile** (built, docs/08 §3.39) — direct: Tile Center, the four Tile/Output per cents, Mirror Edges, Phase and Horizontal Phase Shift all have counterparts. AE's *default* is the identity and Lumit's is a 2×2 repeat (§1.2), which changes nothing on import because the import writes the values |
| "Offset" (`ADBE Offset`) | **Offset** (built, docs/08 §3.40) — mapped: AE's "Shift Center To" is a destination point and Lumit stores the shift, so the import subtracts the frame centre; "Blend With Original" maps to Mix |
| "Mirror" (`ADBE Mirror`) | **Mirror** (built, docs/08 §3.41) — direct: Reflection Center and Reflection Angle convert one for one |
| "Optics Compensation" (`ADBE Optics Compensation`) | **Lens distort** (built, docs/08 §3.42) — direct on the controls that carry the look: Field of View, Reverse Lens Distortion, FOV Orientation and View Center all have counterparts, and Lumit's Field of view has AE's meaning (the frame's rectilinear field of view across the chosen half-extent). AE's Optimal Pixels / Resize has no equivalent and is reported — Lumit renders effects at the frame's raster (docs/08 §2.3) and has no per-effect resize |
| "Turbulent Displace" (`ADBE Turbulent Displace`) | **Turbulent displace** (built, docs/08 §3.38) — mapped: Size, Complexity, Offset (turbulence), Evolution and the cycle convert directly, and AE's Turbulent/Horizontal/Vertical Displacement modes are the three Lumit ships. **Amount converts through AE's own base**, because Lumit's is a length in px@comp rather than a per cent (§3.38 decision 5, the same divergence §3.37 decision 1 records for Fractal noise's Scale). Bulge, Twist, the three "Smoother" variants, Cross Displacement, Resize Layer, Antialiasing and the seven mixed Pinning combinations have no equivalent yet and are reported. **Pinning maps at one index only**: the audit records a dropdown's default but not its option strings, so the one pairing that can be pinned from evidence is AE's own default - every edge - and any other index imports at every edge and is reported rather than mapped to a guess |
| "Fractal Noise" (`ADBE Fractal Noise`) | **Fractal noise** (built, docs/08 §3.37) — mapped: Contrast, Brightness, Complexity, Sub Influence, Sub Scaling, Evolution and the cycle convert directly. **Scale converts through AE's own base**, because Lumit's is a length in px@comp rather than a per cent (§3.37 decision 1) — Adobe does not publish that base, so the import uses the factor that lands AE's default of 100% on Lumit's declared default of 200 px, which is the one point at which both specifications claim the two look alike; AE's dozen Fractal Types collapse onto Basic/Turbulent, its four Noise Types onto Value/Perlin, and Overflow, Sub Rotation, Sub Offset, Perspective Offset and Centre Subscale have no equivalent yet and are reported |
| "Drop Shadow" (`ADBE Drop Shadow`) | **Drop shadow** (built, docs/08 §3.43) — direct: Shadow Color, Opacity, Direction, Distance, Softness and Shadow Only all have counterparts, and Lumit's Direction carries AE's convention (from straight up, clockwise) so the number converts unchanged. **Opacity converts out of eight bits**: AE stores it 0..255 (its 50 % default arrives as 127.5) where Lumit reads a per cent, so the number is divided through and the keyframe handles with it. AE's per-mask targeting has no equivalent and is reported, for the reason §3.34 gives |
| "Set Matte" (`ADBE Set Matte3`) | **Set matte** (built, docs/08 §3.44) — direct on the two controls that decide the picture: "Take Matte From Layer" is the universal Matte row (docs/08 §2.6), "Use For Matte" is Channel, "Invert Matte" is the Matte row's Invert, and "Composite Matte with Original" is Combine with existing alpha. **The default channel differs** (Lumit's is Luminance, AE's the alpha) and the import writes the value, so nothing converts wrongly. "Stretch Matte to Fit" and "Premultiply Matte Layer" have no equivalent and need none — Lumit renders the matte at this raster and composites premultiplied throughout — and both are reported |
| "Channel Blur" (`ADBE Channel Blur`) | **Channel blur** (built, docs/08 §3.45) — mapped: the four radii and "Repeat Edge Pixels" convert directly — AE's radii are raster pixels and Lumit's are px@comp (§2.3, K-419), the same number. AE's Blur Dimensions (Horizontal / Vertical / both) has no counterpart — Lumit's is always both — and is reported |
| "Linear Wipe" (`ADBE Linear Wipe`) | **Linear wipe** (built, docs/08 §3.46) — direct: Transition Completion, Wipe Angle and Feather convert one for one, and the angle carries AE's convention. Lumit adds a Wipe centre AE does not have, which defaults to the frame centre and so changes nothing on import |
| "Radial Wipe" (`ADBE Radial Wipe`) | **Radial wipe** (built, docs/08 §3.47) — direct: Transition Completion, Start Angle, Wipe Center, Feather and the three Wipe directions all have counterparts (AE's "Counterclockwise" is Lumit's **Anticlockwise**, docs/01 §9 — British English, same behaviour) |
| "Venetian Blinds" (`ADBE Venetian Blinds`) | **Venetian blinds** (built, docs/08 §3.70) — direct: Transition Completion, Direction, Width and Feather all convert one for one, and Direction carries AE's convention (degrees from straight up, clockwise). Mapped in one respect: **Width converts from AE's raster pixels to px@comp** (§2.3), so a preview and an export show the same rank of slats. Lumit's Completion defaults to 50 where AE's is 0 (§1.2), which changes nothing on import because the import writes the value |
| "Iris Wipe" (`ADBE IRIS_WIPE`) | **Iris wipe** (built, docs/08 §3.71) — direct: Iris Center, Iris Points, Use Inner Radius, Rotation and Feather all have counterparts, and — like AE's — the effect has no Completion, the radius being the transition. The two radii are layer pixels in AE and px@comp in Lumit (§2.3, K-419), so they convert unchanged. AE's Iris Points range of 6..32 is Lumit's exactly, at both ends |
| "Card Wipe" (`APC CardWipeCam`) | **Card wipe** (built, docs/08 §3.72) — mapped: Transition Completion, Transition Width, Rows, Columns, Randomness and Random Seed convert one for one; Flip Axis's X and Y are **Horizontal axis** and **Vertical axis**, Flip Direction's Positive and Negative are **Forwards** and **Backwards** (docs/01 §9 — an option is named for what it does), and Random is Random in both. Four things are reported rather than approximated. **The whole camera system** — Camera Position, Corner Pins, Composite Camera, the Lighting and Material groups, Position Jitter and Rotation Jitter — has no counterpart: Lumit keeps cameras on the composition (docs/06), so each card is projected in its own local frame at a fixed viewing distance (§3.72's fourth decision). **Back Layer** is not carried, and Lumit's picture is AE's own with the back layer empty. **Card Scale** is not carried. And **Flip Order's Gradient** entry, with its Gradient Layer, is not carried — §3.68's test says a card wipe has a second thing to say about *where*, so the one layer row it has stays the universal Matte; an instance using Gradient imports as Left to right and is reported as approximated, with the Gradient Layer reported beside it. **The spread is not recovered from the layer**: the capture carries a layer *index*, not the layer's pixels, so AE's own Timing Randomness is what comes across and a number invented from a picture nobody has read is not |
| "Corner Pin" (`ADBE Corner Pin`) | **Corner pin** (built, docs/08 §3.48) — direct: the four corner points convert one for one, and Lumit's projective map is AE's. Lumit adds an Edges control AE does not have, which defaults to Transparent — AE's only behaviour — and so changes nothing on import. AE's Perspective Corner Pin (`ADBE Corner Pin`'s 3D sibling) and its "expand output" have no equivalent and are reported |
| "Displacement Map" (`ADBE Displacement Map`) | **Displacement map** (built, docs/08 §3.49) — mapped on the controls that decide the picture: "Displacement Map Layer" is the universal Matte row (docs/08 §2.6), the two "Use For … Displacement" pickers are Horizontal channel and Vertical channel, and the two "Max … Displacement" sliders are the Amounts. Two conversions and one report. **The Amounts convert through AE's own base**, Lumit's being lengths in px@comp rather than per cents (§3.49, §3.38 decision 5's reasoning again). AE's Hue, Lightness, Saturation, Full and Off channel choices collapse onto the five Lumit ships (docs/08 §1.2's shared channel list), Off becoming an Amount of 0. And **Displacement Map Behaviour** (Centre Map / Stretch Map to Fit / Tile Map) is reported rather than approximated: the Matte row renders the referenced layer at this raster, which is Stretch to Fit and always was |
| "Polar Coordinates" (`ADBE Polar Coordinates`) | **Polar coordinates** (built, docs/08 §3.50) — direct: Interpolation and the two Type of Conversion values convert one for one, and Lumit's Interpolation carries AE's meaning (a morph along the map, not a dissolve). AE's centre is the layer's, as Lumit's is the frame's, so there is nothing to convert |
| "Twirl" (`ADBE Twirl`) | **Twirl** (built, docs/08 §3.51) — mapped: Angle and Twirl Center convert directly. **Twirl Radius converts through AE's own base**, being a per cent of the layer where Lumit's is px@comp (§2.3): AE's 100 is the circle that just contains the layer, whose radius is half its diagonal, so the number becomes that per cent of half the comp's diagonal, in pixels |
| "Spherize" (`ADBE Spherize`) | **Spherize** (built, docs/08 §3.52) — mapped: Center of Sphere converts directly, and **AE's one signed Radius becomes two controls** — its magnitude is Radius (px@comp, the same number), its sign is Bulge at ±100 (§3.52's fourth note). Nothing is lost, and a negative AE radius imports as a pinch of the same size |
| "Ripple" (`ADBE Ripple`) | **Ripple** (built, docs/08 §3.53) — mapped: Center of Ripple converts directly and the two Conversion Types are the two Lumit ships. **Radius converts through AE's own base**, being a per cent of the layer where Lumit's three lengths are px@comp (§2.3, K-419): it becomes pixels for §3.51's reason, while Wave Height and Wave Width are AE's raster pixels and convert unchanged. **Wave Speed has no control and needs none**: it reads the clock, which §2.4 forbids, so the import writes two Evolution keyframes of `360 × speed` degrees a second — the same motion, deterministic |
| "Wave Warp" (`ADBE Wave Warp`) | **Wave warp** (built, docs/08 §3.54) — mapped: Wave Height, Wave Width, Direction, Phase and **all eight Pinning combinations** convert one for one (the angle carries AE's convention, degrees from straight up clockwise), and the two lengths convert from AE's raster pixels to px@comp. Sine, Square, Triangle, Sawtooth and Circle are the five Lumit ships; AE's **Noise and Smooth Noise** wave types and its **Antialiasing** switch have no equivalent and are reported (the Warp Axis this row once also named belongs to `ADBE WRPMESH`, not here - the 2026-08-20 audit lists no such property on Wave Warp) — a wave shape that needs a seed is a §3.37 field, and Lumit resamples bilinearly everywhere |
| "Bezier Warp" (`ADBE BEZMESH`) | **Bezier warp** (built, docs/08 §3.55) — direct: the four vertices and eight tangents convert one for one, in AE's own clockwise walk from the upper left, and the Coons patch they bound is AE's. **Quality converts in meaning rather than in kind**: AE's buys smaller triangles and Lumit's buys Newton steps, since every pixel inverts the patch exactly instead of being drawn as a mesh, so the number carries across and means "more accurate" on both |
| "Warp" (`ADBE WRPMESH`) | **Warp** (built, docs/08 §3.56) — mapped: Bend, Horizontal Distortion and Vertical Distortion convert one for one, and thirteen of AE's fifteen Warp Styles are the thirteen Lumit ships under their own names. The exact curve of each style is Lumit's own — AE's is Photoshop's undocumented mesh — so this is a look-for-look conversion and is reported as mapped. AE's **Shell Lower** and **Shell Upper** styles and its **Warp Axis** switch have no equivalent and are reported |
| "Roughen Edges" (`ADBE Roughen Edges`) | **Roughen edges** (built, docs/08 §3.57) — mapped: Border, Complexity, Offset (turbulence), Evolution and the cycle convert directly, while **Edge Sharpness and Fractal Influence are bare factors in AE where Lumit reads per cents** and are multiplied by a hundred (AE's 1.00 is Lumit's 100 %); Stretch Width or Height has no counterpart and is reported. and **AE's seven Edge Types become three plus a switch** — Roughen, Cut and Spiky with Colour edge off or on (§3.57 decision 2), which is a lossless conversion in both directions. AE's **Photocopy** and **Photocopy Color** convert to Cut with Colour edge on and are reported as approximations. **Scale converts through AE's own base**, Lumit's being a length in px@comp (§3.37 decision 1) |
| "Posterize" (`ADBE Posterize`) | **Posterize** (built, docs/08 §3.58) — direct: AE's one Level is Lumit's Levels and converts unchanged. Mapped in one respect, and it is the point of the effect: **AE quantises an 8-bit display value and Lumit quantises a perceptual position in scene-linear light** (§3.58 decision 1), so the bands land in the same places rather than at the same numbers — a value-for-value conversion would put nearly every band in the highlights |
| "Threshold" (`ADBE Threshold`) | **Threshold** (built, docs/08 §3.59) — direct: AE's Level is Lumit's Level, on the same perceptual placement §3.58 records. Lumit adds a **Softness** AE does not have, which defaults to 0 — AE's hard cut — and so changes nothing on import |
| "Tritone" (`ADBE Tritone`) | **Tritone** (built, docs/08 §3.60) — direct: Highlights, Midtones and Shadows convert one for one and "Blend With Original" maps to Mix. Mapped in two respects, both §2.1's: the three stops are placed **perceptually** (§3.58's square root), so a midtone lands on the grey a person points at rather than on half the light, and a highlight above white is **scaled** rather than clamped to the Highlights colour |
| "Photo Filter" (`ADBE Photo Filter`) | **Photo filter** (built, docs/08 §3.61) — direct on every control: the Filter dropdown's twenty-one entries, Color (Lumit's Colour, used in Custom), Density and Preserve Luminosity all have counterparts under the same names. **The twenty named filters are Lumit's own chromaticities under Adobe's names** — Adobe's exact values are not published — so this is a look-for-look conversion and is reported as mapped, exactly as §3.56's thirteen Warp styles are |
| "Black & White" (`ADBE Black&White`) | **Black and white** (built, docs/08 §3.62) — direct: the six weights convert one for one and carry AE's defaults, and Tint and Tint Color are the two Lumit ships. Mapped in one respect: **Tint colour is divided through by its own luma** (§3.62), so it changes the picture's hue and not its exposure, and an imported dark tint tints rather than darkens |
| "Shadow/Highlight" (`ADBE ShadowHighlight`) | **Shadow highlight** (built, docs/08 §3.63) — mapped: Shadow Amount, Highlight Amount, both Tonal Widths, Color Correction, Midtone Contrast and "Blend With Original" convert one for one. One conversion and one report. **AE's two Radii become Lumit's one** and the import averages them (§3.63), a second full-frame gaussian being a great deal of work for a mask's softness; the mean stays in pixels (px@comp, §2.3). **Auto Amounts, Temporal Smoothing and Scene Detect are reported**, not approximated: an instance using them imports with AE's default manual pair written in, because a grade whose answer at a frame depends on the shot around it is not this effect (§3.63). AE's Black Clip and White Clip have no counterpart — scene-linear has headroom and nothing needs clipping (§2.1) — and are reported, as §3.36's "Clip result values" is |
| "Median" (`ADBE Median`) | **Median** (built, docs/08 §3.64) — mapped: AE's Radius is Lumit's and "Operate on Alpha Channel" is Operate on alpha. One conversion and one cap. **The radius is a length in px@comp** rather than raster pixels (§2.3) — a reinterpretation rather than an arithmetic conversion, the two rasters being the same one, and what it buys is that a Half preview medians the same window the export will; and **Lumit's radius is capped at 3** where AE's runs to 50 (§3.64 decision 2, the cost being the fourth power of it). An instance over the cap imports at 3 and is reported as approximated — the first conversion in the table limited by a *budget* rather than by a semantic |
| "Mosaic" (`ADBE Mosaic`) | **Mosaic** (built, docs/08 §3.65) — direct: Horizontal Blocks, Vertical Blocks and Sharp Colors all convert one for one and carry AE's meanings. Mapped in one respect, and only in the averaged mode: **Lumit samples the block on an at-most-8×8 grid** rather than reading every pixel of it (§3.65 note 2), which is the same flat colour on any block worth mosaicking and an exact mean on any block under eight pixels across |
| "Find Edges" (`ADBE Find Edges`) | **Find edges** (built, docs/08 §3.66) — direct: AE's Invert is Lumit's **Invert edges** (renamed only to keep it apart from the universal Matte row's own Invert) and "Blend With Original" is Mix. Mapped in one respect, §2.1's: **the gradient is taken on a perceptual position** (§3.58's square root) rather than on the light, so the lines land where a person would draw them rather than on the specular highlights, which is what an 8-bit Sobel gives AE for free |
| "Emboss" (`ADBE Emboss`) | **Emboss** (built, docs/08 §3.67) — direct: Direction, Relief, Contrast and "Blend With Original" all have counterparts, and Direction carries AE's convention (degrees from straight up, clockwise). **Relief converts from AE's raster pixels to px@comp** (§2.3), and the difference is taken perceptually for §3.66's reason |
| "Texturize" (`ADBE Texturize`) | **Texturize** (built, docs/08 §3.68) — mapped: "Texture Layer" is Lumit's own **Texture** row (not the universal Matte row — §3.68 decision 1 explains why this one is not §3.49's shape), Light Direction converts one for one and **Texture Contrast is a bare factor in AE where Lumit reads a per cent**, so it is multiplied by a hundred as §3.57's two are. Three conversions. **Texture Placement converts as a fitting**: AE's Stretch Texture to Fit is Lumit's Stretch at Scale 100 exactly, while its Tile and Center are the texture layer's *native* size, which the layer carriage has not preserved (docs/impl/layer-input.md) — so the choice converts and the size is reported as approximated. And **Relief is a control AE does not have**, its relief being one pixel of whatever raster it was handed; the default of 1 px@comp is AE's behaviour at full resolution |
| "Broadcast Colors" (`ADBE Broadcast Colors`) | **Broadcast safe** (built, docs/08 §3.69) — direct: Broadcast Locale is Standard (NTSC / PAL), "How To Make Color Safe" is How to treat with the same four entries, and Maximum Signal Amplitude is Maximum signal in the same IRE units. Mapped in one respect: **the signal is encoded with §3.58's square root rather than the real transfer function** (§3.69 decision 2), because the answer is a threshold and the two render paths have to agree about it exactly; across the range the difference is under two IRE, inside the margin a limit of 110 already carries. The name is Lumit's, docs/01 §9: an effect is named for what it does |
| "Beam" (`ADBE Laser`) | **Beam** (built, docs/08 §3.73) — direct: Starting Point, Ending Point, Length, Time, Starting Thickness, Ending Thickness, Softness, Inside Color, Outside Color and Composite on Original all have counterparts under the same names, and Length and Time carry AE's meanings exactly. Two conversions and one report. **The two thicknesses convert from AE's raster pixels to px@comp** (§2.3). **Softness is measured against the rim rather than against the whole width** (§3.73's third note), so the number carries across as a look rather than as a length. And AE's **3D Perspective** is not carried and is reported: it foreshortens the beam from a camera Lumit keeps on the composition (docs/06), which is §3.72's ruling on Card Wipe's camera in a smaller costume |
| "Advanced Lightning" (`ADBE Lightning 2`) | **Lightning** (built, docs/08 §3.74) — mapped: Origin, Direction, Conductivity State, Forking, Decay, the Core group's Radius and Color, the Glow group's Radius, Opacity and Color, and Composite on Original all convert — **Core Opacity does not**, Lumit's core being drawn at full strength and its one opacity belonging to the glow, and it is reported — and **Conductivity State means the same thing on both sides** — the depth axis of the field the bolt is displaced by, which is the one exact parity in the batch. Three conversions. **Four of AE's eight Lightning Types are built** (Direction, Strike, Omni, Two-way Strike); Breaking, Bouncey, Anywhere and Vertical map to the nearest of the four and are reported as approximated (§3.74's third decision). **The bolt's shape is Lumit's own** — AE's displacement is undocumented — so this is a look-for-look conversion, reported as mapped exactly as §3.56's Warp styles are, with AE's Turbulence arriving on Lumit's single Amplitude through the same defaults-align base §3.37's Scale takes, Adobe publishing no range for it. And **Alpha Obstacle, Decay Main Core and the whole Expert group** (Complexity, Min/Max Forking Distance, Termination Threshold, Main Core Collision Only, Fractal Type, Core Drain, Fork Strength and Fork Variation) have no counterpart and are reported |
| "Radio Waves" (`APC Radio Waves`) | **Radio waves** (built, docs/08 §3.75) — mapped: Producer Point, Frequency, Expansion, Orientation (Lumit's **Rotation**), Spin, Lifespan, and the Stroke group's Color, Opacity, Fade-in Time and Fade-out Time all convert, along with the Polygon group's Sides, Star and Star Depth — **Star Depth arrives as its magnitude**, Lumit's depth being unsigned, and a negative AE depth is reported. Four conversions and two reports. **AE's clock becomes Lumit's Time control** (§2.4, §3.75's first note): the import writes two Time keyframes running at one second a second, which is AE's motion exactly and is deterministic. **Expansion converts from AE's raster pixels a second to px@comp a second** (§2.3), as does Start Width — Lumit ships one **Stroke width** where AE ships a start and an end pair, and the import takes the start and reports the taper. **Only the Polygon wave type is built**: Image Contours maps to §3.76 Vegas and is reported as a suggestion, and Mask now maps to §3.76 Vegas on its Mask/Path source (K-408) and is reported as the same kind of suggestion — a mask marched round by dashes rather than emitted from a point, which is the shape of it, not the motion. And **Parameters Are Set At, Render Quality, Reflection, Direction, Velocity and Curviness** have no counterpart and are reported — the first two are AE's own performance affordances, and the rest are per-wave motion Lumit's closed-form ring cannot carry |
| "Vegas" (`APC Vegas`) | **Vegas** (built, docs/08 §3.76) — mapped, on its Image Contours half: Threshold, the Rendering group's Color, Width, Hardness, Start Opacity and the Segments group's Length and Rotation all have counterparts, and Rotation carries AE's meaning (it marches the segments along the contour). Three conversions and three reports. **AE's Segments count becomes Lumit's Segment length** in px@comp (§3.76's second decision), the import converting through the imported contour's own perimeter where one can be measured and reporting the instance as approximated where one cannot. **The contour is a level set of the perceptual luma** rather than AE's edge detector, so Threshold converts in meaning rather than in kind (§3.76's first and last decisions). **Input Layer and Invert Input** convert to the Source choice where the input is the layer itself; a *different* input layer is reported, Vegas having no second layer row. **AE's Channel picker converts to the same Source choice**, its first entry being Lumit's Luminance; any other entry imports as Luminance and is reported, Lumit's contour being a level set of the perceptual luma by construction. And AE's **Pre-Blur, Tolerance, Render (All/Selected), Selection, Segment Distribution, Random Phase, Random Seed, Mid-point Opacity, Mid-point Position, End Opacity and both Blend Modes** have no counterpart and are reported — a blend mode is the layer's (docs/06), and the rest describe a *traced path* Lumit does not build. **AE’s Mask/Path source is carried too** (K-408): Source converts to Lumit’s Mask/Path option and the named mask comes across on the mask-path row, and on that half **Segments converts exactly** rather than approximately — the mask has a measurable perimeter, so the count divides it into a Segment length with no guesswork (§3.76’s Mask/Path decision). An instance naming a mask the import did not bring over falls back to the first mask and is reported |
| "Add Grain" (`VISINF Grain Implant`) | **Add grain** (built, docs/08 §3.77) — mapped: Intensity, Size, Softness, the Color group's Monochromatic and its Red, Green and Blue channel balances, and the Animation group's Animation Speed and Random Seed all convert. Four conversions and three reports. **Size converts from AE's raster pixels to px@comp** (§2.3), and **AE's multipliers become Lumit's per cents**: Intensity and the channel balances multiply by 100, which the balances' own neutral of 1.0 against Lumit's 100 pins exactly, while Softness has no neutral to pin it and takes §3.37 Scale's defaults-align base. **Animation Speed becomes Lumit's Animate switch** — a non-zero speed is Animate on, zero is off — because a grain that redraws at a *rate* reads the clock, which §2.4 forbids (§3.53's ruling a fourth time). **AE's Tonal Ranges group, with its movable shadow, midtone and highlight boundaries, becomes Lumit's three fixed hat functions**: the three amounts convert one for one and the four boundary controls are reported. And **the grain field is Lumit's own** — AE's is a sampled film stock — so this is a look-for-look conversion, reported as mapped. AE's **Viewing Mode, Blending Mode, Preset, Aspect Ratio, Saturation and the whole Application group** have no counterpart and are reported: a blending mode is the layer's (docs/06), a preset is a §5 preset, and the rest are refinements of a look Intensity, Size and Softness already reach |
| "Remove Grain" (`VISINF Grain Removal`) | Placeholder + report — a denoiser is its own programme, not an effect port (docs/impl/ae-effect-parity.md's recorded skips) |
| "Scribble" (`ADBE Scribble Fill`) | **Scribble** (built, docs/08 §3.78) — mapped, and the first import to carry a **mask reference** across (K-408): Mask, Angle, Stroke Width, Stroke Options’ Spacing and Path Overlap, Start, End, Wiggle Type, Wiggles/Second, Random Seed, Color, Opacity and Composite all convert. Three conversions and three reports. **Stroke Width, Spacing and Path Overlap convert from AE’s raster pixels to px@comp** (§2.3), and travel together. **Wiggle Type maps option for option** — Static, Jagged and Wiggly mean the same thing on both sides (§3.78’s third decision), which is the one exact parity in this pair. **Fill Type is carried only in its single-mask forms**: Inside, Left Edge and Right Edge import as the plain fill and are reported as approximated, and the two multi-mask modes are reported against the seam, §1.2’s row naming one mask by design. And AE’s **Edge Options, Curviness, the three Variation sliders, Fill Type’s edge widths and the Blend Mode** have no counterpart and are reported: a blend mode is the layer’s (docs/06), and the variations are per-stroke randomness Lumit’s single waver already supplies |
| "Stroke" (`ADBE Stroke`) | **Stroke** (built, docs/08 §3.79) — mapped: Path, Color, Brush Hardness, Opacity, Start, End, Spacing and Paint Style all have counterparts, and **Paint Style maps option for option** — On Original Image, On Transparent and Reveal Original Image mean the same thing on both sides. Two conversions and two reports. **Brush Size doubles**: AE’s is a radius and Lumit’s Brush size is a width (§3.79’s second decision), and it converts from raster pixels to px@comp on the way (§2.3). **Spacing carries unchanged**, being a per cent of the brush on both sides. And **All Masks and Stroke Sequentially** are reported: §1.2’s mask-path row names one mask by design, so a stroke over every mask at once waits on a seam that names a set (docs/impl/ae-effect-parity.md) — an instance with All Masks on imports pointed at the first mask and says so |
| "Echo" (`ADBE Echo`) | **Echo** (built, docs/08 §3.25) — mapped: Number Of Echoes is Lumit's Echoes, Decay is Decay on the same 0..1 scale, and Echo Operator maps entry for entry where Lumit has one — Add, Screen, Composite In Back and Composite In Front are Add, Screen, **Behind** and **In front**, and Maximum and Minimum are **Lighten** and **Darken** (docs/01 §9 — an option is named for what it does). Two reports. **Echo Time is not carried**: Lumit declares its temporal window as the previous sixteen *frames* (§2.5), so the echoes step one frame at a time and an instance whose Echo Time is not one frame back is reported. AE's Starting Intensity and its Blend operator have no counterpart and are reported |
| "Posterize Time" (`ADBE Posterize Time`) | **Posterize time** (built, docs/08 §3.26) — direct, and the only one-control row in the table: AE's Frame Rate is Lumit's and means the same thing. Lumit adds a Phase AE does not have, which defaults to 0 — AE's behaviour — and so changes nothing on import |
| "Timewarp" (`ADBE Timewarp`) | Placeholder + report suggestion to use Retime with flow interpolation |
| "Slider Control" (`ADBE Slider Control`) | **Slider control** (built, docs/08 §3.80) — **pending audit** (K-414): direct, the one property onto the one row in the same units, keyframes and expressions included. Nothing is converted and nothing is reported |
| "Angle Control" (`ADBE Angle Control`) | **Angle control** (built, docs/08 §3.81) — **pending audit**: direct. Degrees on both sides and unbounded on both, so a rig that winds past a full turn winds past it here |
| "Checkbox Control" (`ADBE Checkbox Control`) | **Checkbox control** (built, docs/08 §3.82) — **pending audit**: direct. AE stores a checkbox as a number, which is the switch |
| "Color Control" (`ADBE Color Control`) | **Colour control** (built, docs/08 §3.83) — **pending audit**: direct, the swatch crossing from the project’s display space into scene-linear light like every other imported colour (§3) |
| "Point Control" (`ADBE Point Control`) | **Point control** (built, docs/08 §3.84) — **pending audit**: direct, AE’s raster pixels read as px@comp (§2.3), onto Lumit’s adjacent `_x`/`_y` pair |

Match names in this table were **verified against a live After Effects 26.0**
(2026-08-20, `tools/ae-audit/` — the report is the evidence): 48 of the original 60
matched exactly, and twelve were corrected to what AE actually ships — the old Atomic
Power effects carry `APC` (Vegas, Radio Waves, Card Wipe as `APC CardWipeCam`), the
grain suite carries `VISINF` (Grain Implant / Grain Removal), Beam is `ADBE Laser`,
Advanced Lightning is `ADBE Lightning 2`, Scribble is `ADBE Scribble Fill`,
Shadow/Highlight is `ADBE ShadowHighlight`, Iris Wipe is `ADBE IRIS_WIPE`, Warp is
`ADBE WRPMESH` and Bezier Warp is `ADBE BEZMESH`. Property-level mapping works from
the audited property trees, not from memory. Every mapped conversion
MUST have a golden-frame test: the AE-rendered frame and the Lumit-rendered frame of a
reference comp compared within a stated tolerance. Unmapped match names fall through to
placeholders — never to the closest guess.

**The five Controls rows are marked pending** (K-414): their match names are the famous
ones but were not in that sitting’s audited set, so they enter the table pending and
`tools/ae-audit/claimed-matchnames.txt` gained them (60 names to 65) so the next sitting
confirms them. A pending row is safe to ship for §6’s reason: a match name this table
has wrong is a name nothing claims, and an unclaimed name becomes a placeholder with
every parameter kept, never a guess.

Two rules run through every row above and are stated once here rather than in each:

- **An option list maps by position, and the position is anchored on the audit.** After
  Effects stores a dropdown as an integer, so a wrong order is a silently wrong picture
  rather than an error. The audit records each effect's *default* value, which is the one
  index a live After Effects has confirmed, and every list below is pinned so that AE's
  default lands on the entry AE's own documentation names as its default. Where a list is
  longer than Lumit's, the entries with no counterpart map to the nearest and are
  reported as approximated (§9's grade), never carried silently.
- **Where a base is undocumented, the defaults are the anchor.** A handful of AE controls
  are per cents of a private base — Fractal Noise's Scale is the standing example, and
  Advanced Lightning's Turbulence and Add Grain's Softness join it. Adobe publishes no
  base for any of them, so the import uses the factor that lands After Effects' own
  default on Lumit's declared default for the same control, and reports the conversion.
  That is the one point at which both specifications claim the two effects look alike; it
  is a stated choice rather than a measurement, and a golden-frame test is what will
  replace it with one.

---

## 6. Placeholder behaviour

A placeholder is an inert effect node that:

- keeps the original display name, match name, enabled state, and the **complete parameter
  dump including keyframes and expressions** (parameters are real Lumit properties: they
  animate, appear in the graph editor, and are expression-readable, they just drive
  nothing);
- renders as **identity** — input passed through unchanged;
- shows a subtle badge in Effect Controls ("not rendered — imported from After Effects"),
  in the calm style of [15-DESIGN.md](15-DESIGN.md) — no red, no warning triangle
  theatrics;
- is **never lost**: saving a `.lum` project preserves placeholders and their `ae`
  namespace data byte-for-byte, so a project can be opened, edited around, saved, and the
  placeholder data survives indefinitely. If a later Lumit version (or an installed OFX/
  LFX effect registered as an upgrade target in `ae-effect-map.toml`) gains a mapping, the
  user is offered — never forced — a per-instance upgrade.

The same mechanism serves missing OFX/LFX plugins at project-open time
([12-PLUGINS.md](12-PLUGINS.md) §1), so "placeholder" is one concept everywhere.

---

## 7. Direct `.aep` parsing

**The primary route since K-418.** Honest scope: **recover what we can, measure what
we recovered** — the golden fixture's `.aep` sits beside AE's own account of its
contents, and the parser's recovery is a per-category number asserted in CI, not a
hope. The parser emits the same capture shape the Bridge writes (§2.3), so both
routes share one mapping, one effect table, one report.

`.aep` is a RIFX container (RIFF, big-endian sizes, form type `Egg!`) of nested LIST chunks.
Chunk shapes are publicly known; many field semantics are not, and Adobe changes details
across versions without documentation. Lumit builds on the community reverse-engineering
work — `forticheprod/aep_parser` (the most complete public description, MIT, licence
checked 2026-08-21) and `boltframe/aftereffects-aep-parser` (Go, MIT, explicitly partial) —
**read as documentation and reimplemented in Rust** inside `lumit-import`, with attribution
in the impl note. Nothing is vendored (K-418).

Realistically recoverable: the project item tree and folder structure, footage paths and
basic interpretation, comp settings, layer stacks with names/types/in-out/start/order,
blend mode and switch flags, basic transform values, and a useful subset of keyframe data.
Progressively unreliable to unrecoverable: full temporal ease semantics across all property
classes, expressions storage, mask feather detail, text and shape contents, effect
parameter blobs (typed per match name, third-party blobs opaque).

Policy:

- The report MUST open automatically after a direct parse, and anything the parser
  could not recover MUST be a row in it (K-418 retires the blanket "structure only"
  label in favour of per-item honesty — the measured recovery earned the front door).
- Anything ambiguous imports as a placeholder or as a static value with a report entry —
  the parser MUST NOT guess silently.
- A parse failure on one chunk skips that chunk and continues; the report lists skipped
  chunks. **Built**: those skips arrive as `Reason::ChunkUnreadable` rows on the same
  summary the mapping's own rows ride, and only on this route — a property the *Bridge*
  could not read is already an unreadable node in the capture, and saying it twice is
  worse than not saying it. Whole-file failure falls back to "import footage references
  only" where the footage table is readable — **not built**: a whole-file failure today
  is the calm notice naming the Bridge route, and the project already open stands.
- New AE versions MAY break the parser at any time; this is stated in the UI copy. The
  Bridge remains the answer for fidelity — and is what the failure notice points at.

**The surface** (K-418, docs/impl/ae-import.md §7 phase C): the user picks the `.aep`
from File ▸ Import ▸ After Effects project…, and nothing runs inside After Effects at
all. The route is chosen engine-side by `lumit_import::open_ae`: a folder is a bundle, a
file whose first four bytes are `RIFX`/`RIFF` is an `.aep`, and anything else goes to the
bundle reader. The extension is a hint and the magic is the truth, so a renamed project
still opens and a zipped bundle offered by the same picker is still read as a zip.

`.aepx` (AE's XML save) is the same data with the interesting chunks hex-encoded; it MAY
share the parser back-end but is not a separate fidelity route.

---

## 8. Lottie import

Cheap extra on-ramp: bodymovin/Lottie JSON is documented, has `lottie-web` as a reference
implementation, and a large template ecosystem. Lumit imports Lottie shape/text/image/
precomp layers, transforms, and keyframes into ordinary comps — Lottie easing converts
exactly (it is the same bezier model, normalised). Features outside Lottie's scope simply
do not arrive, and Lottie features Lumit lacks (specific layer effects) follow the same
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
  **Not built** (docs/TODO.md): rows name their item and do not yet lead to it.
- **Persistence**: the report is stored in the project (`ae` namespace) and reopenable from
  the File menu; it is also written next to what was imported as `import-report.json` for
  tooling.
  **Not built** (docs/TODO.md): the report lives as long as its window does.
- Expressions disabled at import are their own filter, so a user can work through them.
  The filter is by outcome today; a reason-level filter comes with the persistence work.

**The reasons are engine data, not engine prose** (K-303, docs/17). A row crosses as the
stable id of its reason plus that reason's facts by name, and the sentence is written in the
frontend, in the reader's language — a sentence built with `format!` on the engine's side
could not be translated at all. The built surface is `flutter_ui/lib/shell/ae_report_frb.dart`
over `LumitBridgeState::import_ae_bundle`, which hands the whole report across once.

The report is informative, never blocking: import always completes, the project always
opens.

---

## 10. Non-goals

- **Loading `.aex` AE plugins — never.** Modern AE plugins depend on SmartFX, AEGP suites,
  and Adobe GPU internals that no third-party host can honestly re-implement, and the few
  shipping attempts (Grass Valley's EDIUS bridge) support only a hit-and-miss subset.
  Adobe's SDK explicitly states it neither supports nor recommends third-party hosts, and
  the SDK licence plus plugin vendors' host-locked activation make the legal exposure real
  while the same vendors already ship OFX builds. Lumit routes all plugin demand through
  OFX and LFX ([12-PLUGINS.md](12-PLUGINS.md)) — see K-061.
- **`.ffx` preset files.** Closed RIFX-family binary, no complete public parser, version-
  drifting. Mitigation: apply the preset inside AE and export with the Bridge — presets
  become ordinary properties. Native `.ffx` reading MAY be revisited if the direct-parse
  property decoder matures (§7 shares the problem).
- **Guaranteed visual parity of any comp using unmapped effects.** The matrix is the
  contract; placeholders are the mechanism; the report is the disclosure. Lumit does not
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
  Lumit implementations (upgrading four matrix rows to lossless), and where do they sit in
  [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)'s compositing order?
- **Text animator scope**: which selector modes and per-character-3D features Lumit's text
  engine will support decides several matrix rows; blocked on the text spec in
  [03-DATA-MODEL.md](03-DATA-MODEL.md).
- **Bundle size**: property-heavy projects (thousands of keyframed masks) may produce very
  large `capture.json` files; decide a compression policy (zip member compression is
  probably enough) with real-world CC-pack samples.
- **Kaitai grammar licence**: confirm `forticheprod/aep_parser`'s licence is compatible with
  GPLv3 vendoring, or reimplement from the published chunk documentation.
