# After Effects import — implementation note

**Spec:** [11-AE-IMPORT.md](../11-AE-IMPORT.md). **Decisions:** K-060 (strategy), K-410
(the Bridge captures, Rust converts), K-025 (AE-compatible keyframe maths), K-021/K-249
(Retime). This note owns the capture schema, the ExtendScript traps, the fixture
strategy, and the phase plan. Specs say *what*; this is the *how*.

## In plain terms

Getting a project out of After Effects has two halves. The first half runs *inside*
After Effects as a script: it walks everything in the project — every composition,
layer, keyframe, mask and effect — and writes what it finds into a folder of JSON
files, changing nothing along the way. It is deliberately a courier, not a translator:
it records what AE said, in AE's own words. The second half runs inside Lumit, in
Rust: it reads that folder and does all the actual translating — AE's clock times
become Lumit's exact rational times, AE's effects become Lumit's effects through a
mapping table, and anything that cannot translate becomes a clearly-labelled
placeholder instead of quietly disappearing. Splitting it this way means the half that
does the thinking is the half our test suite can actually run: the script side cannot
be tested by CI (it needs a real After Effects), so it is kept too simple to get wrong.

## 1. The pieces

| Piece | Where | What |
|---|---|---|
| The walker | `tools/ae-bridge/lumit-bridge.jsx` | ExtendScript; walks `app.project` per docs/11 §2.2 and writes a bundle folder. One try/catch per property; a failure becomes an `unreadable` entry, never an abort. |
| The fixture builder | `tools/ae-bridge/make-fixture.jsx` | Builds a deterministic test project covering the feature matrix (§5 below), saves it as `fixture.aep`, then runs the walker on it — one sitting in AE produces the repo's golden bundle. |
| The reader | `crates/lumit-import/src/capture.rs` | Serde types mirroring the capture schema; bundle open (folder or zip), manifest versioning (refuse newer major, accept older). |
| The mapping | `crates/lumit-import/src/map/` | Capture → `lumit_core::Document`: items, comps, layers, keyframes, mattes, masks, markers, retime, blend modes, effects. |
| The effect table | `crates/lumit-import/src/map/fx_colour.rs`, `fx_distort.rs` | The docs/11 §5 table as data: match name → Lumit effect + per-parameter conversion (unit, base, option collapse). Built from `tools/ae-audit/ae-audit-report.json`'s property trees, never from memory. |
| The report | `crates/lumit-import/src/report.rs` | Per-item outcomes (imported / adjusted / placeholder / skipped) with reasons; serialises as `import-report.json` and crosses the bridge for the panel. |

`lumit-import` is an engine crate: depends on `lumit-core` only, no IO assumptions
beyond reading the bundle it is handed, no panics, budgeted allocations.

## 2. The capture schema (bundle version 1.0.0)

`manifest.json`:

```json
{ "format": "lumit-ae-bundle", "version": "1.0.0",
  "ae_version": "26.0x67", "bridge_version": "1.0.0", "exported": "2026-08-21" }
```

`capture.json`, shapes in outline (all times are the DOM's float seconds, verbatim;
all ids are AE's own integers; nothing is converted — K-410):

- **`project`** — the project-wide settings no item carries: `{ bits_per_channel,
  working_space, linear_blending, linearize_working_space, expression_engine }`. The
  colour flagging in [11-AE-IMPORT.md](../11-AE-IMPORT.md) §3 needs the bit depth and
  the working space, and neither can be recovered from the items after the fact.
- **`items[]`** — the flat item list, each `{ id, name, parent_id, kind }` where kind is
  one of `folder`, `comp`, `footage` (`{ path, width, height, fps, native_fps, duration,
  fps_override, alpha, premul_colour, invert_alpha, loop, fields, remove_pulldown,
  is_still, is_placeholder, is_missing }`), `solid` (`{ colour, width, height }`).
- **`comps[]`** — `{ id, width, height, par, fps, duration, start, bg_colour,
  motion_blur: { enabled, shutter_angle, shutter_phase, samples, adaptive_limit },
  renderer, preserve_nested_fps, preserve_nested_resolution, markers[], layers[] }`.
  There is deliberately **no `name`**: a comp is also an item, and the name lives
  there once, joined by `id`.
- **`layers[]`** — in stacking order: `{ index, name, kind, source_id, in_point,
  out_point, start_time, stretch, parent_index, label, blend, preserve_transparency,
  auto_orient, light_type, matte: { type, layer_index, is_track_matte },
  switches: { enabled, audio, solo, lock, shy, quality, motion_blur, adjustment,
  three_d, collapse, frame_blending, guide, effects_active }, markers[],
  time_remap_enabled, properties }`. Kind: `footage | solid | precomp | text | shape |
  null | adjustment | camera | light | audio`. `auto_orient` carries the camera's
  one/two-node flag; `is_track_matte` marks a layer being *used* as somebody's matte,
  which is all the legacy above-layer form says.
- **`markers[]`** — comp and layer alike: `{ t, comment, duration, chapter, label }`.
- **`properties`** — a recursive tree mirroring the DOM: group nodes
  `{ match_name, name, enabled, mask?, group: [children] }`, leaf nodes
  `{ match_name, name, value_type, value | keyframes[], expression,
  expression_enabled, separated: [follower trees], unreadable: "error text" }`.
  `enabled` is a group's own switch and is what carries an effect instance's on/off
  state (docs/11 §2.2 item 9). Effects live under the `ADBE Effect Parade` group
  exactly as AE stores them; masks under `ADBE Mask Parade`, each mask node carrying
  `mask: { mode, inverted, roto_bezier, locked, colour }` beside its property tree,
  whose Path values are `{ vertices, in_tangents, out_tangents, closed }`; text under
  `ADBE Text Properties` with the text document captured as a flat dict of the
  attributes the DOM exposes, each in its own try/catch. `ADBE Marker` is the one
  match name the tree skips — its keys are MarkerValues, captured as `markers[]`.
- **`keyframes[]`** — per key, everything docs/11 §2.2 item 5 lists: `{ t, v,
  in_interp, out_interp, in_ease: [{speed, influence}…], out_ease, in_tangent,
  out_tangent, roving, auto_bezier, continuous, spatial_auto_bezier,
  spatial_continuous }`. A value copy of what the DOM returns; no resampling, no
  baking.

**The vocabulary is AE's own, verbatim.** Every enum-valued field above —
`blend`, `matte.type`, `mask.mode`, `switches.quality`, `switches.frame_blending`,
`auto_orient`, `light_type`, `alpha`, `fields`, and a key's `in_interp`/`out_interp` —
is the **name of the ExtendScript constant**, spelled exactly as the DOM spells it:
`SCREEN`, `ALPHA_INVERTED`, `SUBTRACT`, `BEST`, `PIXEL_MOTION`, `BEZIER`, `HOLD`.
The walker never lower-cases, re-spells or normalises one, because that would be a
conversion in the half CI cannot run (K-410); an enum member the walker's own name
list does not know falls through as the raw number stringified, which is honest
rather than wrong. The one exception is **`value_type`**, a closed vocabulary shared
with `tools/ae-audit/audit.jsx` so the two kits describe a property the same way:
`float`, `point`, `point3`, `colour`, `custom_blob`, `layer`, `mask`, `shape`,
`text`, `marker`, `group`, `other`. `custom_blob` is AE's `CUSTOM_VALUE` — the one
that cannot be read (§3).

`report.json`: `{ unreadables: [{ comp, layer, path, match_name, error }] }`, where
`comp` and `layer` are **names** — this half is the human-facing one, and the
machine-facing link already exists in `capture.json`.

Schema growth is additive; the Rust reader ignores unknown keys (mirroring
docs/10 §1.1's rule) and refuses only a newer *major* version.

## 3. ExtendScript traps (the ones that already bit, and the known ones)

- **`app.effects` is a 0-based plain array**, not a 1-based AE collection — the audit
  hit this. Everything else (`numProperties`, `numKeys`, layer indices) is 1-based.
- **`CUSTOM_VALUE` properties cannot be read.** `prop.value` throws or returns
  nothing useful for Curves' point list, Levels' histogram, Hue/Saturation's channel
  ranges (audit: `ADBE CurvesCustom-0001`, `ADBE Easy Levels2-0002`,
  `ADBE HUE SATURATION-0003`). Record `unreadable`, keep walking. Levels and
  Hue/Saturation still map fine from their plain sibling properties; Curves does not
  (K-410's honesty note).
- **ExtendScript has no `JSON`** — reuse `tools/ae-audit/audit.jsx`'s escaper/writer.
  ES3 only: no `Array.prototype.map`, no `const`, no getters.
- **`addSolid` needs all six arguments** including duration — the audit hit this too.
- **TextDocument attributes throw when not applicable** (box-text fields on point
  text, per-character-3D fields when off): one try/catch per attribute.
- **Separated dimensions**: `prop.isSeparationLeader` → walk
  `prop.getSeparationFollower(i)`; the leader's own keyframes are not the animation.
  The followers are *also* ordinary children of the Transform group
  (`ADBE Position_0`, `_1`, `_2`), separated or not, so a capture lists them
  twice; the ones under `separated` are the ones with the animation on them.
- **Temporal ease** (`keyInTemporalEase`) returns an array of ease objects *per
  dimension* — but spatial properties return exactly one. Capture the array as-is.
- **Mattes have two generations**: 23.0+ `layer.trackMatteType` +
  `layer.trackMatteLayer` (capture the referenced layer's index), and the legacy
  layer-above form. Capture what the DOM has; normalisation is Rust's job.
- **Markers**: comp markers via `comp.markerProperty`, layer markers via
  `layer.property("ADBE Marker")`; each key's value is a MarkerValue (comment,
  duration).
- **Effect application dialogs**: some legacy effects open dialogs on `addProperty` —
  irrelevant to the walker (it never applies effects), relevant to the fixture
  builder (stick to the audited 60, which all applied cleanly).
- **File writes need the preference on** (Scripting & Expressions → allow file
  access) — say so in the README, as the audit's does.

## 4. Mapping rules pinned for the Rust side

- **Time**: the capture's float seconds convert against the comp's frame rate per
  [rational-time.md](rational-time.md): `round(t × fps)` frames when within 1e-6 of a
  frame, else the exact rational nearest at denominator `fps × 1000`. Keyframes may
  legitimately sit between frames; never frame-snap a key that is not on one.
- **Keyframes are a value copy** (K-025): interpolation types, per-side
  speed/influence, spatial tangents, roving. No resampling — Lumit's evaluator is the
  same cubic.
- **Ids**: AE integer ids → fresh UUIDv7s, with the AE id recorded in the `ae`
  namespace so re-imports and the report can name the original.
- **Effects**: match name found in the table → mapped instance (per-parameter
  conversion, keyframes carried through the same converter); not found → placeholder
  (docs/11 §6) carrying the full dump. Never the closest guess.
- **Every conversion writes a report row** when it adjusted anything (docs/11 §9's
  grades). The report is data first (a struct), prose second.
- **The importer never fails a whole import**: a comp that cannot map imports as an
  empty comp with a report entry; parse errors on one item skip that item.

## 5. The fixture (what `make-fixture.jsx` builds)

One deterministic project exercising every mapped feature, so the golden bundle in
`tools/ae-bridge/fixtures/` regression-tests the whole chain. Coverage checklist:
nested comps (A contains B); solids, a null, an adjustment layer, a guide layer; a
parenting chain; position keys with bezier ease + a hold key + roving + spatial
tangents; separated position on one layer; rotation and opacity keys; stretch at 50%
and −100%; an alpha matte (inverted) and a legacy luma matte; two masks (one with
animated path, feather, subtract mode); comp and layer markers with comment +
duration; time remap with a hold key; frame blending both modes; a spread of blend
modes including one with no equivalent (Dissolve); shy/lock/solo switches; a 3D
layer + two-node camera + one light; a text layer with styling; a shape layer
(rectangle + ellipse, gradient fill, Trim Paths, Repeater); one enabled and one
disabled expression; and effect instances: Gaussian Blur (keyframed), Tint, Fill,
Transform, Fractal Noise (choice params), Levels, Hue/Saturation, Curves (the
unreadable), Drop Shadow, Vegas (mask source), Scribble (mask reference) — plus one
match name Lumit does not ship, for the placeholder path.

The builder saves `fixture.aep` beside the bundle (the direct-parse route's future
test asset), builds inside an undo group, and does not touch an open project.

**What the first real sitting corrected.** Three of the assumptions the
hand-written fixtures were built on turned out not to be After Effects':

- **A Null and an Adjustment layer are backed by a solid *item*.** Letting the
  source item decide the layer kind imported a rig's null as the white card it
  is made of, and an adjustment layer as an opaque solid over the whole comp.
  The layer's own `kind` now wins for those two.
- **Setting `stretch = -100` reflects the layer about its own zero**, so the bar
  arrives sitting entirely *before* comp time zero with `in_point` and
  `out_point` the other way round. The two ends are read in order (a swap, not
  a repair — nothing about the layer changed), and the Retime is then AE's own
  arithmetic with no special case: source time is layer time times the rate.
  The reflection the mapper used to apply on top of that doubled the turn.
- **AE 26 records the modern matte form for both generations**, so a
  `trackMatteType`-only assignment comes back naming its layer outright. The
  legacy above-layer form still exists in older projects and keeps its test in
  `edges.lum-bundle`.

**Two checklist rows did not come through**, and both are owed in
docs/TODO.md rather than papered over: the **roving** key (After Effects did
not apply `setRovingAtKey`, so the capture records `roving: false` — the walker
reads `keyRoving` correctly and there is nothing to import), and a 3D layer's
**Orientation** and Material Options, which the capture carries and the mapper
has nowhere to put — the one place in the mapping that loses something without
a report row.

## 6. Phases

1. **The walker and the reader** — `tools/ae-bridge/` + `lumit-import`'s capture
   types and bundle open. **Built, and the golden bundle is in.**
   `make-fixture.jsx` was run on a live After Effects 26.0 on 2026-08-20;
   `tools/ae-bridge/fixtures/fixture.lum-bundle/` (two comps, 24 layers, 109
   unreadables) is the repo's golden fixture, and
   `crates/lumit-import/tests/golden.rs` is the regression suite that gates it —
   every §5 checklist row asserted through the *mapped* document, with every
   expected number computed in the test from the fixture's own inputs. The
   hand-written `synthetic.lum-bundle` and `edges.lum-bundle` stay: they are the
   schema's readable documentation and the awkward half (both generations of
   matte, the damaged captures) that one well-formed AE project does not
   contain. The sitting also confirmed this note's match names are the ones AE
   ships, and turned up three things no synthetic bundle had (see §5).
2. **The mapping** — capture → `Document`, the effect table, placeholders, the
   report. **Built**, in `src/map/`: `mod.rs`/`layers.rs`/`props.rs`/`time.rs` for the
   structure and the keyframe value copy, `fx_colour.rs` and `fx_distort.rs` for the
   docs/11 §5 table (all sixty rows — fifty-seven mapped, three at a placeholder on
   purpose), `effects.rs` for the placeholder road, `report.rs` for the typed report.
   Tested against two fixtures (`synthetic.lum-bundle` for the ordinary half,
   `edges.lum-bundle` for the awkward one) plus per-effect conversion tests and a
   save-and-reload round trip through `lumit-project`. **Still owed**: the golden-frame
   comparisons docs/11 §5 requires of every mapped conversion (phase 4 — they need AE
   renders), and a second audit pass enumerating dropdown option *strings*, without
   which Turbulent displace's Pinning maps at its default index only and several other
   orders rest on AE's documented defaults rather than on evidence.
3. **The surface** — **Built.** `LumitBridgeState::import_ae_bundle(path,
   on_change_stream) → Option<BridgeImportedProject>` in
   `crates/lumit-bridge/src/api/import.rs`, which maps the bundle and then adopts
   the document through `api::state::adopt` — the road `open_project` now takes
   too, so an import lets the previous project's worker and GPU device go exactly
   as an open does. Footage relinks through that same `resolve_all_media` step,
   rooted at the bundle's folder — which today finds a file **only at the
   absolute path After Effects recorded**, because the mapper stores that path
   on both sides of the `MediaRef` and an absolute path re-rooted against a
   folder is still itself. So docs/11 §2.5's re-rooting step is inert, and stays
   inert until the collected `footage/` copy it exists to find is written (v1
   skips it) and the mapper has something genuinely relative to store. An item
   whose file is nowhere imports offline with a `MediaNotFound` row and never
   holds the import up. File ▸ Import After Effects
   bundle… (`flutter_ui/lib/shell/menu_bar_frb.dart`) opens a **folder** chooser
   and shows `shell/ae_report_frb.dart` — docs/11 §9's summary line, filter by
   outcome, and a row per reason. A reason crosses as a stable id plus its facts
   and the sentence is written in `l10n/engine_labels.dart` (K-303), gated by
   `test/l10n/engine_labels_test.dart` reading the `Reason` enum. **Still owed**:
   the picker cannot reach a *zipped* bundle the reader can open, §9's row
   navigation, and §9's persistence of the report in the project — all three in
   docs/TODO.md.
4. **Later, separately**: golden-frame comparisons (needs AE renders of the fixture),
   the CEP panel packaging (`.zxp` — v1 ships the `.jsx` run via File → Scripts,
   exactly like the audit), Lottie, direct `.aep` parsing.

## Open questions

- Bundle zip vs folder: v1 reads both, the walker writes a folder (ExtendScript has
  no zip); revisit when the CEP panel packages one.
- Shape-layer and text-animator mapping depth is decided by their engine features,
  not by the importer — capture is complete either way (docs/11 §2.2 items 10–11).

## 7. The direct `.aep` parser (K-418)

**In plain terms.** An `.aep` is a RIFX container: RIFF with big-endian sizes, form
type `Egg!`, a tree of `LIST` chunks. The parser walks that tree and fills the same
`Capture` the Bridge writes, so everything downstream — mapping, effect table,
report — is shared. Its correctness is *measured*: `tools/ae-bridge/fixtures/` holds
`fixture.aep` beside the bundle After Effects itself exported from it, and the
differential tests compare the parsed capture against AE's account field by field,
per category, with the recovery numbers asserted in CI.

**Reference implementation** (read as documentation, nothing vendored):
`forticheprod/aep_parser` on GitHub, MIT (licence checked 2026-08-21, closing
docs/11's old open question) — since its Kaitai days it has matured into hand-written
chunk parsers; the modules that matter are `src/py_aep/binary/` (`chunk.py` the RIFX
walk, `item_chunks.py`/`composition_chunks.py`/`layer_chunks.py` the `idta`/`cdta`/
`ldta` records, `property_chunks.py` the `tdgp`/`tdbs`/`tdb4`/`cdat` property system,
`ldat_chunks.py` the keyframe records per property class) and `src/py_aep/parsers/`
(`effect.py`, `text.py`, `marker.py`, `gradient.py`). `boltframe/aftereffects-aep-parser`
(Go, MIT) is the second opinion. Attribution comment at the top of the Rust module.

Chunk knowledge pinned (verify each against the reference and the fixture, not
memory): names arrive in `Utf8` chunks; match names in `tdmn`; a property group is
`LIST tdgp`, a property's metadata `tdbs`/`tdb4` (dimension count, animated flag,
kind bits), its static value `cdat` (f64s), its keyframes `LIST list` → `lhd3`+`ldat`
(fixed-size records per property class, carrying time, value, and the temporal ease
floats); items `idta`, comp settings `cdta`, layers `ldta` (bitfields for the
switches, matte type, blend mode as an index), expressions as `Utf8` beside their
property. String encoding is UTF-8 with occasional MacRoman legacy; sizes pad to
even. Where a record's layout is version-dependent, the parser keys on the file's
`nhed`/app version and refuses fields it does not know rather than misreading them.

**The funnel rule (binding):** the parser NEVER invents vocabulary. Where the DOM
route writes an ExtendScript constant name (`SCREEN`, `BEZIER`, `ALPHA_INVERTED`),
the parser writes the same name, translated from the aep's numeric code through one
table per enum whose entries are proven by the differential tests. A code with no
table entry falls through as the stringified number — the reader's unknown-handling
already copes, and the report says so.

Phases: **A** container + items/comps/layers structure → differential green on every
non-property field of the golden capture; **B** property trees, keyframes, effects,
masks, expressions → differential per-category recovery numbers; **C** the surface —
the picker takes the `.aep` file (docs/11 §1's seamless front door), parse → the one
mapping, plus the stretch goals (the Curves `CUSTOM_VALUE` blob is IN the file, so
the K-412 sixteen-point target may finally be reachable — measured, not promised).

### 7.1 Phase A: the layouts that are proved

**Built.** `crates/lumit-import/src/aep/` — `rifx.rs` (the container walk),
`enums.rs` (the funnel tables), `mod.rs` (the structure decode and `open_aep`) —
with `tests/aep_differential.rs` as the gate. Everything below was read out of
`fixture.aep` and checked against `fixture.lum-bundle/capture.json`, the same
project as After Effects itself described it; offsets are **byte offsets into
the chunk body**, sizes big-endian, and this table is the authoritative
Rust-side map. Anything *not* listed here is not read, on purpose.

**The tree.** `RIFX` (form `Egg!`) → loose project chunks + `LIST Fold`. `Fold`
holds `fdta` then one `LIST Item` per top-level item; a folder item carries
`LIST Sfdr` holding its own `LIST Item` children, and the Bridge's item order is
a **depth-first pre-order** walk of exactly that tree. An item is
`iide`,`idpc`,`idta`,`Utf8`,… ; a comp adds `cdta`, `LIST PRin` and its layers; a
footage item adds `LIST Pin ` (holding `sspc` and `opti`).

**Only `LIST Layr` is a layer.** A comp also holds `DLay`, six `SLay`, three
`CLay` (the viewer's default/side/custom view cameras) and `SecL` (a hidden
layer that exists to carry the comp's markers) — eleven layer-shaped records per
comp, none of them a layer.

| Record | Offset | Field |
|---|---|---|
| `head` (20) | 4 (u32 bitfield) | version word: `major = ((w>>26)&0x1F)*8 + ((w>>19)&7)`, `minor = (w>>15)&0xF`, `build = w&0xFF`. Fixture `0x0f100643` → `26.0x67`, the manifest's own string. |
| `nnhd` (40) / `nhed` (32) | 24 (u8) | colour-depth **exponent**: 0→8, 1→16, 2→32 bpc |
| root, no payload | — | `lnrb` present = linear blending; `lnrp` present = linearise working space |
| root | `PwCs` then `Utf8` | working-space profile JSON; a literal `{}` is the DOM's `None`. Identified by the marker, never by "first profile envelope" — `pdvc` writes an identical one for the display space. |
| root | `LIST ExEn` ▸ `Utf8` | expression engine (`javascript-1.0`) |
| `idta` (84) | 0 (u16) | item type: 1 folder, 4 comp, 7 footage |
| | 16 (u32) | item id — the id `Comp.id` and `Layer.source_id` point at |
| | 0x3B (u8) | label colour |
| `sspc` (222+) | 32 (u16), 36 (u16) | width, height |
| `opti` | 0..4 | asset type; `Soli` marks a solid |
| | 14/18/22 (f32) | solid colour R/G/B |
| | 26 (strz, 256) | **the solid's name** — a solid's own `Utf8` chunk is empty |
| `cdta` (204) | 44/48 | duration (rational: s32 dividend / u32 divisor) |
| | 52,53,54 (u8) | background colour |
| | 139 (u8) | comp flags: bit 7 preserve nested resolution, bit 5 preserve nested frame rate, bit 3 motion blur, bit 0 hide shy |
| | 140/142 (u16) | width, height |
| | 144/148 (u32) | pixel aspect (dividend/divisor) |
| | 156 (u16) + 158 (u16) | frame rate = whole + fraction/65536 |
| | 164/168 | display start time (rational, signed) |
| | 174 (u16) | shutter angle |
| | 180 (s32) | shutter phase |
| | 196 (s32) | motion-blur adaptive sample limit |
| | 200 (s32) | motion-blur samples per frame |
| `LIST PRin` ▸ `prin` (104) | 4 (ascii 48) | renderer match name. The file's own name differs from scripting's: `ADBE Escher` **is** what the DOM calls `ADBE Advanced 3d`. |
| `ldta` (164) | 0 (u32) | layer id — what `parent_id` and the matte reference point at |
| | 4 (u16) | quality: 0 wireframe, 1 draft, 2 best |
| | 8 (s32) with 108 (u32) | stretch as dividend/divisor; the capture's percentage is ×100 |
| | 12/16, 20/24, 28/32 | start time, in point, out point (rationals, **unstretched**) |
| | 37 (u8) | bit 4 chars-toward-camera, 3 per-character 3D, 2 frame-blend *kind* (1 = pixel motion), 1 guide layer, 0 name-was-set |
| | 38 (u8) | bit 7 null, 6 point-of-interest auto-orient (camera/light), 5 camera auto-orient, 3 solo, 2 3D, 1 adjustment, 0 auto-orient along path |
| | 39 (u8) | bit 7 collapse, 6 shy, 5 locked, 4 frame blending on, 3 motion blur, 2 effects active, 1 audio, 0 enabled |
| | 40 (u32) | source item id; `0` and `0xFFFFFFFF` both mean none |
| | 61 (u8) | label colour |
| | 64 (strz, 32) | layer name, when the file put it there |
| | 99 (u8) | blend mode, the SDK's `PF_Xfer` index |
| | 103 (u8) | bit 0 preserve underlying transparency, bit 1 dancing dissolve |
| | 107 (u8) | matte type: 0 none, 1 alpha, 2 alpha inverted, 3 luma, 4 luma inverted |
| | 131 (u8) | layer type: 0 AV, 1 light, 2 camera, 3 text, 4 shape |
| | 132 (u32) | parent's **layer id** |
| | 139 (u8) | light type: 0 parallel, 1 spot, 2 point, 3 ambient, 4 environment |
| | 160 (u32, AE ≥ 23) | matte layer's **layer id** |
| `tdb4` (124) | 68 (u8) | `animated` — which is what scripting reports as the layer's `timeRemapEnabled` for `ADBE Time Remapping` |

**Five things the layout alone does not tell you**, each of which produces a
project that opens and is wrong:

- **A layer's name is not one field.** It is the `Utf8` chunk beside `ldta` when
  that is non-empty, then `ldta`+64, and otherwise **the source item's name** —
  which is what AE displays for a layer nobody renamed, and is why eighteen of
  the fixture's layers have an empty name chunk.
- **A null and an adjustment layer are backed by a solid item** (§5 again): the
  layer's own bits at `ldta`+38 decide, never the source.
- **In and out points are stored unstretched.** Scripting reports
  `start + (raw − start) × stretch`. At a negative stretch the two ends come
  back the other way round — a swap, not a repair.
- **Camera and light layers have no blend mode, transparency flag, matte block
  or `timeRemapEnabled`**, and only five of the thirteen switches, because the
  scripting DOM does not offer the rest on a rig. The capture must be absent
  there, not `NORMAL`.
- **"Is somebody's matte" is a fact about the other layer.** `is_track_matte` is
  filled in after the whole stack is read, from who points at whom.

**Owed, and honest about it.** (1) The **footage interpretation** fields — path,
frame rate, alpha, fields, pulldown, loop, missing — are not read at all: the
golden project is solids and comps with no file footage in it, so not one offset
could be checked against AE, and an unchecked offset is the silently-wrong
import this route exists to avoid. A fixture with real footage is owed.
(2) A **reflected layer's** ends land 1/3000 s further out in AE's arithmetic
than in the file's (`−0.000333` / `−10.000333` rather than `−0` / `−10`), as if
AE reflects inclusive indices on an internal grid — one sample is not enough to
prove the grid, so the differential test compares those two within a frame and
records the delta rather than curve-fitting it. (3) Every funnel-table row the
fixture does not exercise is marked `reference` in `enums.rs`.

### 7.2 Phase B: the property system

**Built.** `crates/lumit-import/src/aep/props.rs` — the `tdgp`/`tdbs`/`tdb4`
trees, static values, keyframes, effects, masks, shapes, markers and
expressions — with `tests/aep_differential.rs` grown from five tests to
thirteen and every recovery number pinned in CI.

**The one thing to understand before any number below means anything: an
`.aep` stores only what is not at its default.** A layer nobody moved has no
`ADBE Position` record in it; a solid at 100% opacity has no `ADBE Opacity`.
That is not damage and it is not something to guess around — the property is
absent from the capture exactly as it is absent from the file, and the mapping
layer already treats an absent property as "use the default", which is what
After Effects does on open. So of the golden capture's **3,319 property leaves,
2,734 are simply not in the file**; the claim the differential asserts is the
useful one: *every leaf that is there is right, and nothing is reported that
After Effects did not report*.

**Recovery on the fixture** (each number an assertion, not a measurement):

| Category | Recovered | Notes |
|---|---|---|
| Static property values | **646 exact**, 0 wrong, 0 invented | of 652 leaves the file stores |
| Keyframes | **27 of 27** (11 properties) | time, value, per-side interpolation, ease, spatial tangents |
| Expressions | **2 of 2** | source text and the on/off flag, one of each |
| Effect instances | **13 of 13** | match name, display name (`fnam`), on/off switch |
| Effect parameters | every one the file stores | names from `pard`, values in DOM units |
| Masks | **2 of 2** | mode, inverted, RotoBezier, locked, colour, and the paths |
| Mask paths | static and animated | vertices and both tangents, denormalised |
| Markers | **4 of 4** | 2 layer, 2 comp (one per comp), time/duration/comment/chapter/label |
| Separated dimensions | **1 of 1** | followers carry the animation, as the DOM reports it |
| `CUSTOM_VALUE` blobs | **3 of 3, as raw bytes** | the DOM cannot read these at all |
| Group on/off switches | all, incl. Layer Styles' derived one | |
| Property display names | **83, none wrong** | the ones the file holds; the other 1,106 are AE resources, not file data — see below |
| Text document | match-named, marked unreadable | phase C. A **gradient** is unmeasured: the fixture holds no `GCst` at all |

**Damage is not a crash.** This parser eats untrusted files, so the differential
also damages the golden one sixty-four deterministic ways — cut short, single
bytes flipped, chunk sizes overwritten with `0xFFFFFFFF`, runs zeroed — and
requires an answer from every one: forty-eight parse partially (the walk of a
damaged container costs that container and no more) and sixteen refuse with a
typed error. The whole sweep is timed as well as checked, because a length the
file declared being trusted somewhere shows up as a hang before it shows up as
anything else.

**End to end**: `map_capture` on the parsed capture and on the golden bundle
produce documents with **identical counts** — 22 items, 2 comps, 24 layers, 13
effects, 2 masks, 4 markers, 8 animated transform properties. The import report
is the only difference: 58 rows against the Bridge's 64, and the six are
properties the Bridge's own DOM could not read.

**The layouts proved.** Offsets are byte offsets into the chunk body,
big-endian.

| Record | Offset | Field |
|---|---|---|
| `tdsb` (4) | 3 (u8) | bit 0 enabled (a group's own switch, and an effect instance's), bit 1 dimensions-separated on a leaf |
| | 0 (u8) | RotoBezier, on a mask shape's `tdsb` only |
| `tdb4` (124) | 2 (u16) | dimension count |
| | 5 (u8) | bit 3 the property is *spatial* |
| | 57 (u8) | bit 0 the property carries no numeric value |
| | 59 (u8) | bit 3 vector, bit 2 integer, **bit 0 colour** |
| | 68 (u8) | animated |
| | 119 (u8) | bit 0 the expression is switched **off** |
| `tdsn` | ▸ `Utf8` | display name; the literal `-_0_/-` is AE's "nobody renamed this" sentinel |
| `cdat` | 0.. | static value: `dimensions` big-endian f64s, then the ease numbers |
| `Utf8` in a `tdbs` | — | the expression source |
| `lhd3` (52) | 10 (u16) | key count |
| | 18 (u16) | **bytes per record** — the class discriminator; never assume it |
| | 23 (u8) | list type code, paired with the size |
| `mkif` (48) | 0/1 (u8) | inverted, locked |
| | 6 (u16) | mode, the SDK's `PF_MaskMode`: 0 none, 1 add, 2 subtract, 3 intersect, 4 lighten, 5 darken, 6 difference |
| | 45/46/47 (u8) | outline colour |
| `shph` (24) | 3 (u8) | bit 0 normalised, **bit 3 open** (so closed = not that bit) |
| | 4/8/12/16 (f32) | bounding box: left, top, right, bottom |
| `NmHd` (20) | 8 (u32) | marker duration in **600ths of a second** |
| | 16 (u8) | label colour |
| `pard` (148) | 15 (u8) | the SDK parameter type: **11 arbitrary data**, **13/14 topic start/end**, 15 declared-empty |
| | 16 (strz, 32) | the plug-in's own parameter name — the one scripting reports |

**The keyframe record**, in every class: 8 bytes of header — time as a signed
32-bit count of the comp's internal timebase units (`cdta`+8) *relative to the
layer's start*, then in-interpolation (1 linear, 2 bezier, 3 hold),
out-interpolation, label, and a flag byte with roving at bit 5, temporal
auto-bezier at 4 and temporal continuity at 3. The payload is chosen by
`(list type, record size)`:

| Type/size | Class | Payload after the header |
|---|---|---|
| 4 / 48 | 1-D | 5 f64: value, in speed, in influence, out speed, out influence |
| 4 / 80 | 1-D, longer record | the same five, read the same way (reference) |
| 4 / 88 | 2-D | the same five, ×2 axes, grouped by field |
| 4 / 128 | 3-D | the same five, ×3 axes |
| 4 / 104 | 2-D spatial | 3 pad, a spatial flag byte (bit 1 auto-bezier, bit 0 continuous), 4 pad, then 5 f64 (one unknown, then one ease per side, *not* per axis), then value×2, in-tangents×2, out-tangents×2 |
| 4 / 128 + spatial | 3-D spatial | the same, ×3 |
| 4 / 152 | colour | 2 f64 unread, 4 ease f64, then A,R,G,B |
| 4 / 64 | valueless | 2 f64 unread, 4 ease f64 — a mask path's keys, whose *values* are the `shap` records in the sibling `omks`, one per key, in order |
| 4 / 16 | marker | time and flags only; the comment and duration are the matching `Nmrd` in `mrky` |
| 4 / 8 | shape point | a pair of big-endian f32s (inside a `shap`, not a keyframe list) |

**Seven conversions the file does not do for you**, each proven against the
golden capture and each producing a plausible-looking wrong project if missed:

- **Opacity, Scale and Mask Opacity are fractions on disk** and percentages in
  the DOM. A layer at "1" is a layer nobody notices is wrong until it is.
- **A colour is A,R,G,B in 0–255** on disk and R,G,B,A in 0–1 in the DOM. Read
  it in the file's own order and the alpha lands in the red channel.
- **An effect's two-dimensional point is a fraction of the composition.**
- **An anchor point is a fraction of the layer's *source*** — but only when the
  layer has one. A shape, text or null layer stores it in raw pixels.
- **A mask path is normalised twice**: to its own `shph` bounding box, and that
  box to the layer's size. Mask space is the *layer's*, not the comp's.
- **A linear or held key's ease is all zeros in the file.** After Effects works
  it out on demand: a held side has no speed and the default influence (100/6);
  a linear side has the segment's own constant speed and the same influence;
  and at the ends, or against a held neighbour, no speed. A parser that copies
  the bytes gives every linear key a speed of nought.
- **A spatial linear speed is the length of the *motion path*, not the chord.**
  After Effects measures the cubic through the two keys' own spatial handles.
  On the fixture the chord is up to 2.5% short; walking the curve in 1,024
  pieces agrees with AE's own number to six significant figures. (The reference
  implementation uses the chord, so this is measured here rather than borrowed.)

**Five shapes worth knowing about the tree itself:**

- **A layer's properties hang off its own `LIST tdgp`**: `tdsb`, `tdsn`, then
  alternating `tdmn` (a NUL-terminated 40-byte match name) and the node it
  names, closed by `tdmn "ADBE Group End"`. The node is `LIST tdbs` for a leaf,
  `LIST tdgp` for a group, `LIST sspc` for an effect, `LIST om-s` for a path,
  `LIST otst` for an Orientation, `LIST mrst` for markers, `LIST OvG2` +
  `LIST tdgp` for Essential Properties overrides.
- **An effect is `LIST sspc`** = `fnam` (▸ `Utf8`, the display name), `LIST
  parT` (the parameter definitions), `LIST tdgp` (the values) and `pgui`. Its
  index-0 slot (`…-0000`) is AE's own internal parameter and is not exposed by
  scripting, so it must not reach the capture. Parameters match the DOM's by
  match name directly.
- **The `CUSTOM_VALUE` blob is a `LIST aRbs` beside the parameter's `tdbs`**,
  holding one `aRbp` of raw bytes (the default is the identically-shaped `aRbp`
  in `parT`). Curves' is 1,644 bytes.
- **`ADBE Layer Styles` has no switch of its own.** Scripting reports it as on
  when any style below it is on, and Blending Options mirrors that; reading the
  group's own `tdsb` says "on" for every layer in every project.
- **Comp markers are not in `cdta`.** They are the `ADBE Marker` property of
  the hidden `SecL` layer, a `LIST mrst` = a `tdbs` of times beside a `mrky` of
  `Nmrd` records.

**Owed, and honest about it.** (1) **Property display names** are After
Effects' own localised resources, not data in the file — a property nobody
renamed carries the `-_0_/-` sentinel — so 1,106 of the golden's names have no
source in the project at all. The mapper already falls back to the match name;
a name table would be a table of Adobe's English strings, which is a separate
decision. Effect parameters, effect instances and masks *do* get their real
names: 83 of them, and the differential asserts every one equal to After
Effects' own, so a drifted `pard` offset cannot hand a parameter its
neighbour's name unnoticed. (2) **A text document (`btds`)** is its own
encoding — a COS blob — and arrives carrying its match name and a note saying
so; phase C. A **gradient (`GCst`)** is owed the same and has none, and this is
unmeasured rather than done: `fixture.aep` contains no `GCst` chunk at all,
because the shape layer's gradient sits at its default and the file stores only
what does not. A fixture with a non-default gradient is owed before anything can
be claimed about it. (3) **The
project-level `LIST EfdG`** carries every effect's parameter definitions and is
the fallback for a layer whose effect has an empty `parT` (Gaussian Blur's is);
it is not read, which costs only the topic/arbitrary classification for such an
effect, and none of the fixture's needed it. (4) **A mask path's linear speed**
is reported by the DOM as exactly 1.0 per segment; one sample cannot say
whether that is a constant or a duration-derived number, so it is recorded
rather than curve-fitted. Nothing downstream reads a linear side's speed.
(5) **Decoding** the arbitrary-data blobs — K-412's sixteen-point Curves target
is now reachable in principle, since the bytes are in hand.
