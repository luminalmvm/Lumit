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
