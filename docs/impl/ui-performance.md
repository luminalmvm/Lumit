# UI performance — the 60/120 mandate, measured and answered

**Status: binding** (K-676, 2026-08-30). This note records the owner's interface-speed
mandate as engineering fact: what was measured on the real machine against the real
composition **in the owner's own conditions**, where each millisecond actually sits,
the architecture that removes them, and the ordered work packages — each gated by a
number from the same instrument that found the problem.
[13-PERFORMANCE-RULES.md](../13-PERFORMANCE-RULES.md) §2 owns the budgets (B1, B2 are
these rules); this note is the *how* for the panels that must meet them. The
Timeline's behaviour itself is [timeline-interaction.md](timeline-interaction.md) and
nothing here changes it.

## In plain terms

The whole point of Lumit is that the interface never slows down, however big the
composition — the founding grievance with After Effects (docs/13 §2.1). The promise in
numbers: **interacting with anything answers on the next frame, the interface never
drops below 60 frames a second (16.6 ms a frame), and each frame is budgeted at
8.3 ms** — so a 120 Hz or 165 Hz screen is actually fed, and a 60 Hz screen has
headroom rather than luck. The preview picture is allowed to lag (it degrades,
docs/13 §4); the chrome around it is not.

This was not true when measured, and — the useful part — *how* untrue depended on the
conditions as much as the code. The owner's observation ("part of the reason you don't
see the same issues when testing is because you don't fullscreen the window, and also
the preview never shows anything") is treated here as method, not anecdote: §2.2
measures exactly that delta, and it is a factor of four. A click that edits pays a
second-long storm of engine walks; scrolling rebuilds and re-records three screenfuls
to show one new row; zooming and scrubbing build cheaply now and still run at 20 fps
on the **raster** thread at the owner's window size, where no widget count helps. One
answer would have missed; the table is why each fix has the shape it has.

## 1. The budget

| Number | Meaning |
|---|---|
| **8.3 ms** | The frame budget every interaction is designed and tested against (120 fps; docs/13 B1 says ≤ 8 ms and keeps that figure). |
| **16.6 ms** | The floor. An interaction frame over it has dropped below 60 fps and is a defect wherever it happens (docs/13: "Any UI-thread stall > 16 ms is a bug regardless of budget"). |
| **next frame** | Input acknowledgement (docs/13 B2): the pressed row lights, the grabbed handle moves, on the first frame after the press. What the gesture *causes* may then arrive asynchronously without holding that frame. |
| **~0.5–1.2 ms** | The measured price of **one synchronous bridge call that touches the document** on this machine (`time_of_frame` 0.56–0.7 ms, `animated_mask_paths_at` 0.5–1.3 ms, `get_kind` 0.4 ms, `list_effects` up to 7.5 ms once — against 0.02 ms for cache-index reads). This is the number that turns "zero bridge calls during gestures" from taste into arithmetic: a handful of such calls is the whole 8.3 ms budget. |

Two clarifications the mandate adds to docs/13:

- **The budgets hold on the owner's real documents in the owner's real conditions** —
  window maximised on the main monitor, the preview showing the real edit — not only
  on a synthetic comp in a small test window. §2.1 defines the conditions; every §7
  gate is phrased against them.
- **A 120 fps cap for energy** is the stated ideal. Flutter 3.47 exposes no
  frame-rate-cap API on Windows (`Display.refreshRate` is read-only; there is no
  `setPreferredFrameRate`), and the embedder paces to the monitor. What Lumit does
  instead: **draw nothing at rest** — an idle editor schedules zero frames, measured
  (§2.3, true in all four conditions), and a regression of that is a defect — and keep
  per-frame work inside 8.3 ms so pacing at 165 Hz costs little. If the framework
  grows a cap API, take it; nothing blocks on it.

## 2. What was measured

### 2.1 Machine, conditions, method

The owner's Windows machine: RTX 5080, main monitor **2560×1440 at 165 Hz**, OS scale
1.0 (`devicePixelRatio` 1.0; the interface itself draws at the ×1.1 presentation
baseline, K-560, so a maximised window rasters the monitor's full area whatever the
baseline — the baseline changes how many rows fit, not how many pixels are filled).
Flutter 3.47, `flutter run --profile`, the Impeller OpenGLES backend unless a row
says Skia. The project is the real edit, *Set me Free Converted.lum*, the **Clips**
comp fronted — 64 layers, the project's largest.

**The conditions are the experiment.** Four, each printed by the probe at the top of
its output so no run can misreport itself:

- **Owner (max + live):** window maximised (2560×1369 client), media resolving, the
  Viewer (1292×628) showing real frames — during scrubs, renders arriving; between
  them, the idle cache fill keeps the engine rendering.
- **max + empty:** same window, same project structure, media missing — the Viewer
  presents placeholder frames only.
- **agent (1280×720 + empty):** the runner's default window over missing media — the
  condition every earlier session measured in, unawares.
- **Skia (max + live):** the owner condition with `--no-enable-impeller`.

A trap worth its own sentence, found the hard way: **a bare copy of a `.lum` into
scratch is the empty condition**, silently. Media references are saved relative to
the project file (K-173; the absolute path is never serialised), so the copy's media
goes missing and the preview shows nothing — while `render_frame` traffic carries on
against placeholders and looks alive in the logs. The probe therefore prints which
condition it is actually in, and its live runs junction the media folder beside the
copy (§6).

Gestures are driven synthetically through `GestureBinding.handlePointerEvent` — the
real hit-test path — by the parked probe (§6), which records every `FrameTiming`
(UI-thread build ms, raster-thread ms, vsync-to-raster-finish span) and counts every
bridge crossing per gesture through a counting frb handler. Caveats: a lived-in
desktop, so medians are the trusted column; `FrameTiming` reports arrive batched, so
per-click frame counts are approximate while acks and bridge deltas are exact; and
the probe's click holds the button ~40 ms, which is inside every ack figure.

### 2.2 The conditions comparison — the owner's delta as a number

Same binary, same gestures, Impeller. Medians; fps is frames delivered over the
gesture's wall clock.

| Condition | Zoom fly fps (raster ms) | Playhead drag fps (raster ms) | Scroll lanes fps |
|---|---|---|---|
| 1280×720 + empty (the agent trap) | **84.9** (10.5) | **79.6** (11.0) | 12.1 |
| maximised + empty | 30.3 (30.0) | 30.1 (29.9) | 8.5 |
| maximised + live (the owner) | **19.7** (48.8) | **19.9** (47.7) | 8.6 |

Two multiplications, both on the raster thread: the maximised window costs ~3× the
small one (raster 10.5 → 30.0 with nothing else changed), and a *live* picture costs
another ~1.6× on this backend (30.0 → 48.8 — the engine renders and republishes while
the gesture runs, and the Impeller compositor pays ~18 ms a frame for it; §2.4 shows
Skia pays ~nothing for the same traffic). An agent testing in the small empty window
sees **four times** the frame rate the owner sees. This is why every gate in §7 names
its conditions.

### 2.3 The gesture table in the owner's conditions

Impeller (the shipped default), maximised, live preview, 2026-08-30. Spans are
vsync-to-raster-finish; a span over 17 ms missed 60 fps.

| Gesture | build ms med | raster ms med | span ms med | fps | frames > 17 ms | sync bridge during gesture |
|---|---|---|---|---|---|---|
| Idle, 3 s | 0 frames scheduled | — | — | — | 0 | 0 |
| Select: click a row's name (revisit) | 0.1 | — | — | — | 0–1 frames | 0 |
| Select: first visit to a layer | worst frame **38.7–66.8** | — | — | — | 1–3 frames | 14–22 calls, 4.3–12.5 ms |
| Scroll lanes (wheel) | 3.4 (p90 **74.9**) | **42.3** | 82.2 | **8.6** | all | 0 |
| Scroll outline (wheel) | 3.3 (p90 69.8) | 41.3 | 78.3 | 8.3 | all | 0 |
| Zoom fly (Ctrl+wheel) | 10.9 | **48.8** | 94.2 | **19.7** | all | 1 revision check/frame |
| Playhead drag, fresh spans | 6.9 | **47.7** | 93.6 | **19.9** | all | ~1.3 ms/frame (§3.4) |
| Playhead drag, revisited spans | 6.6 | 41.7 | 81.4 | 22.5 | all | same |
| Work-area end-handle drag | 4.1 (max **121**) | 30.3 | 59.5 | 30.2 | all | ~1.2 ms/frame + ~100 ms commit frame |
| Graph mode, zoom fly | 6.7 | 33.0 | 63.7 | 27.9 | all | 1 revision check/frame |
| Graph mode, playhead drag | 4.9 | 34.3 | 67.8 | 26.6 | all | as lanes |
| Playhead drag, render-time measuring off | 5.0 | 36.7 | 70.6 | 25.6 | all | same — measuring is **not** implicated |

Reading it:

- **Idle is right, even live.** No rebuild leak, no polling loop, zero frames over
  three seconds in every condition. Every cost is gesture-caused, which is what makes
  it fixable.
- **A revisit click is nearly right already** — 0.1 ms builds, zero bridge calls. The
  first visit to a layer runs one **39–67 ms build frame** (a panel-wide `setState`)
  plus 14–22 sync calls populating that layer's rows (`get_kind` ×6, `get_graph`,
  `get_text`, `list_effects` — the last cost 7.5 ms once). They memoise correctly:
  revisits cost nothing. §3.1.
- **A click that commits an op is the disaster** — measured earlier at ~228 sync
  calls and 0.68–0.85 s down-to-settled (§3.1); its storm is condition-independent
  and shows again here as the work-area drag's ~100 ms release frame and the
  first-visit clicks' `quiet` tails. Most timeline clicks *are* edits (switches,
  solo, lock), so this is a large part of "clicking feels slow".
- **Scroll never has a good frame** (8–12 fps in every condition, zero bridge calls):
  the window-slide frames rebuild three screenfuls (build p90 ~75 ms at 57 visible
  rows) and the raster thread re-records the whole band. Genuinely UI-thread: Skia
  moves scroll from 8.6 to only 9.9 fps. §3.2.
- **Zoom, scrub, work-area build cheaply** (K-293/K-638/K-647/K-649 and the band
  layer did their work: 4–11 ms builds) **and still run at 20–30 fps**: the raster
  thread's 30–49 ms is the whole story, and it is window-sized, not lane-sized —
  Graph mode (one CustomPaint) rasters the same. §3.3.
- **Fresh versus revisited spans differ by ~6 ms of raster, not by frame rate class**
  (47.7 vs 41.7; on Skia they are identical at 5.25). Presenting from the bank versus
  renders arriving is **not** what makes scrubbing slow; the chrome's own repaint is.
  What a live picture *does* cost on this backend is §2.2's +18 ms — the compositor's
  price for a republishing texture, paid per frame whether fresh or banked.

### 2.4 The backend A/B in the owner's conditions

The same probe, the same build, the same comp, the owner's window and live preview —
one flag (`--no-enable-impeller`) swapping Impeller GLES for Skia:

| Gesture | Impeller GLES (default) | Skia |
|---|---|---|
| Zoom fly — fps / raster med / span med | 19.7 / 48.8 / 94.2 | **99.5 / 5.1 / 11.2** |
| Playhead drag, fresh — fps / raster / span | 19.9 / 47.7 / 93.6 | **124.7 / 5.3 / 10.2** |
| Playhead drag, revisited | 22.5 / 41.7 / 81.4 | **127.3 / 5.3 / 10.2** |
| Work-area drag | 30.2 / 30.3 / 59.5 | **145.3 / 5.0 / 9.2** |
| Graph-mode zoom | 27.9 / 33.0 / 63.7 | **114.3 / 4.8 / 9.9** |
| Playhead drag, measuring off — frames > 17 ms | 72 of 72 | **0 of 409** |
| Scroll lanes — fps (build p90) | 8.6 (74.9) | 9.9 (69.1) |
| First-visit click, worst build frame | 52.7 | 66.8 |

**The Impeller OpenGLES backend costs ~25 ms of raster thread per maximised frame
against Skia's ~5 ms, and a live picture widens that by another ~18 ms** — on its own
the difference between 20 fps and 125–146 fps in the hand. On Skia the zoom fly, both
playhead sweeps and the work-area drag all clear the 60 fps floor with 6–8× headroom
at their medians and p90 spans of 10–15 ms, with today's widget code. What Skia does
*not* fix: the scroll (still ~10 fps — UI-thread build), the first-visit and
edit-commit build storms, and the per-frame sync calls (§3.4, now ~10 % of a 145 fps
frame interval). Those are Lumit's own, and their packages stand.

## 3. Where each millisecond sits

### 3.1 The click, pointer to lit row — two different clicks

**A clean select** (the name cell): pointer down → `OutlineRow`'s raw `Listener`
(outside the arena — correct, keep) → `_selectLayer` → **`setState` on the whole
`TimelinePanelFrb`** + `ui.setSelection` (ValueNotifiers — correct). The panel-wide
rebuild is the 39–67 ms first-visit frame; the property-selection path already knows
better — `TimelineSelection`/`LayerSelection`/`_LayerBlock` exist precisely so "a
click repaints the rows whose selectedness changed instead of the whole Timeline"
(measured at 858 widgets before that landed) — but **layer** selectedness never
joined it: rows and bars still take `selected` as a build-time flag, so the panel
must rebuild to move it. The first-visit bridge reads are the fold rows and Effect
controls populating for a layer not seen before; they memoise correctly (revisits:
zero calls).

**A click that commits an op** (a switch cell; any edit): the op lands, the document
revision moves, and the per-revision walks run **synchronously on the UI thread, one
bridge call per item**: the Viewer's `_factsOf` refill (`getLayers` +
`get_source_item` × 64 layers), `LayerBounds` re-measuring (`get_source_item` per
layer, `get_size` per precomp), the Project panel re-reading `get_settings` per comp
(48 comps). Measured whole (1280×720 runs, backend-independent): ~228 calls and
~90 ms of engine time per click, 0.68–0.85 s down-to-settled across the burst's
rebuild waves. The same storm is the work-area drag's ~100 ms release frame in every
condition (§3.5) — it is the follow-on of *any* edit on a big project.

### 3.2 Scroll

A wheel notch moves the shared vertical scroll; `LazyBlocks` (K-638) recomputes its
three-screenful window; any slide calls `setState` and **rebuilds every block in the
window** — each a `Bar` (hover regions, trim handles, summary-key painter, waveform)
plus a `KeyLane` per open property row — and the band sits behind **one**
`RepaintBoundary` (K-649), so the raster thread **re-records the whole three-screenful
picture** to show one new row at the edge. At the owner's window that is a ~75 ms
build on each slide frame plus ~42 ms of raster per frame — 8.6 fps, both halves (the
scroll mirrors). K-638's virtualisation bounded the cost — 2,000 layers cost what 57
do, and that stands — what is left is that the window is rebuilt and re-recorded
wholesale instead of incrementally. This one is genuinely UI-thread: Skia's better
raster still leaves it at 9.9 fps on its ~69 ms slide builds. §4.3 is the fix.

### 3.3 Zoom and scrub are raster-bound, and two comparisons say where

Builds are 4–11 ms — the K-293 seam (only the lane half listens to the zoom) and
K-638's window hold. The raster thread then spends **30–49 ms** and the frame lands
many vsyncs late: 20–30 fps, the owner's complaint verbatim. The first control:
**Graph mode — one `CustomPaint` of curves — rasters 33 ms for the same gestures in
the same conditions.** So the cost is not the lane band's widget-drawn complexity; it
is the price of re-rasterising a maximised window on this backend, plus ~18 ms
whenever the Viewer's texture is republishing (§2.2's live-vs-empty column; scrubs
and the idle cache fill keep it republishing). The second control: §2.4 swaps the
backend and the same widget code runs at 99–146 fps with the live picture up. The
lever is the backend first (WP-1), then *how much area re-records per frame* (§4.2,
§4.3) — never converting rows to a canvas (§4.4).

### 3.4 The per-frame sync calls on scrub

Two, measured together at ~1.2–1.3 ms a frame — a sixth of the 8.3 ms budget:

- `animated_mask_paths_at` (0.5–1.3 ms): keyed per (comp, frame, revision), so a
  scrub re-asks every frame — to hear "no animated masks" on this comp every time.
  Whether *any* mask is animated can only change with the document: an empty answer
  is valid for the whole revision, whatever the frame.
- `time_of_frame` (~0.56–0.7 ms): memoised per frame number for the session
  (`state/comp_time.dart`), so it taxes frames the session has not asked about — a
  scrub across a long comp on a fresh session pays it per new frame (88–308 ms per
  sweep measured; a maximised zoom-to-fit happens to pre-warm the visible span, which
  is why some runs show it near zero — an accident of warm-up, not a fix). The batch
  that K-647 built (`sample_scalars`, 8 µs a row) crosses the seam at the same moment
  and could carry the time in the same crossing.

These matter *more* once WP-1 lands: at 145 fps the vsync interval is 6.9 ms, and
the pair is ~a fifth of it.

### 3.5 Work-area edge drag

The band layer works: build med 4.1 ms while the hand moves (the ruler's staged
`_dragFrame` and the grounds' listenables — no lane rebuilds). What remains is the
window-sized raster floor (30 ms; the playhead is not moving, so no republish tax),
the same two per-frame sync calls as the scrub, and the release: **one ~100–121 ms
frame** — `set_work_area` (~15 ms sync) plus §3.1's document-change walk. The drag
lags for the same reasons as everything else, plus the commit stall on letting go.

## 4. The architecture

Five rulings. Each is the cheapest structure that meets the budget — no new
framework, no second widget tree, no canvas rewrite of surfaces whose widgets already
fit it.

### 4.1 Ship on the backend the numbers chose, as revisited policy

§2.4 is not a taste: on this machine, at the owner's window, the default backend
cannot reach 60 fps for *any* timeline gesture with any plausible widget-side saving
(the floor is 30 ms of raster before Lumit draws a row), and Skia runs the same code
at 99–146 fps. So the shipped Windows runner starts the engine on Skia, with a
build-time knob keeping Impeller one flag away. The pin is **measured, revisited
policy, not doctrine**: Impeller is Flutter's stated future, so the pin's K entry
carries a review rule — re-run §2.4's A/B on each Flutter upgrade, and take Impeller
the release it matches Skia on this class of machine. An upstream issue with these
numbers is part of the package.

### 4.2 The paint-layer discipline: a repaint matrix, gated

K-626/K-649 layering, finished and enforced. What may rebuild and repaint per gesture:

| Gesture | May rebuild | May repaint (re-record) |
|---|---|---|
| Idle / hover a row | nothing | the hovered control's decoration |
| Playhead move | playhead listenables, time readouts | playhead layer, cache-bar painter, Viewer texture |
| Select click | the blocks whose slice changed; Effect controls (async) | those blocks |
| Vertical scroll | blocks **entering** the window | entering blocks; the rest translates |
| Zoom tick | the lane half (K-293) | lane band + ruler ticks; the outline not at all |
| Work-area drag | the ruler's staged band/handle; ground listenables | band layer, handle, ground wash |
| Document edit | one model-refresh wave | what the edit touched |

The gate: `rebuild_budget_test.dart` grows paint-count assertions per gesture (counted
off `RenderRepaintBoundary`, as K-649's test already does), so a regression is a red
test naming the gesture, not a feeling.

### 4.3 Scroll becomes incremental: widget identity plus per-block boundaries

Two mechanical changes inside `LazyBlocks` (`timeline_metrics_frb.dart`), no API
change:

- **Reuse identical child widgets across window slides.** The state caches the built
  block widget per index while `heights`/`builder` are unchanged (any panel rebuild
  hands in a new builder closure and drops the cache — which is what keeps a cached
  row honest about selection, theme and zoom). An unchanged child is then `identical`
  on the next build, and Flutter skips its rebuild *and* its layout: a window slide
  builds the entering blocks and the two blanks, nothing else.
- **A `RepaintBoundary` per block** (keyed by layer id) inside the band's boundary, so
  the raster thread records the entering block alone and scrolling inside the window
  is a pure layer translate.

This is the graph-view lesson applied where it actually bites — *stop walking widgets
per frame* — without converting rows to a canvas. The per-block boundary adds a layer
per block (~57 on screen maximised); if zoom-fly raster regresses from the added
layers (a flight re-records every block anyway), the boundaries are disabled during
flights — the probe decides, not taste.

### 4.4 Selection is listenable row state, never a panel `setState`

The property-selection path is already right: `TimelineSelection` published on a
`ValueNotifier`, `_LayerBlock` re-slicing per layer (`LayerSelection.of`) and
rebuilding only blocks whose slice changed. Layer selection joins it: the selected
layer ids (and the primary) move into `TimelineSelection`; `OutlineRow` and `Bar`
draw their lit state from their block's slice; `_selectLayer` publishes and **does not
`setState`**. The acknowledgement (docs/13 B2) is then two blocks' repaints on the
next frame, whatever else the click causes. The Effect controls panel keeps its
listener — it genuinely has a new stack to show — asynchronously, off the lit-row
frame.

And the full canvas-band conversion is priced, and parked. Direction (a) in full —
lanes and outline as data-driven `CustomPaint` bands, the graph editor's shape — was
evaluated honestly. It would work, and the plans for what widgets do today are
writable: hit-testing by the same row-height/axis maths the marquee and drop paths
already use as pure functions (`_keysIn`, `layerDropSlot`); cursors from one
`MouseRegion` computing its cursor from hit maths (timeline-interaction P2
preserved); drags resolved by the band's own recognisers; inline rename as the one
overlay widget summoned at the edited row; semantics via
`CustomPaint.semanticsBuilder`, one node per row — *more* accessibility than the rows
offer today. It is parked because the measurements refuse its premise: builds during
the costly gestures are already 4–11 ms, and Graph mode — which *is* one CustomPaint
— rasters within 2 ms of Layers mode in every condition (§3.3). The tree is not what
misses 60 fps; §4.3 removes the one gesture where it is. The conversion becomes the
answer only if the §7 gates cannot be met with the tree — then lane bands first, and
the outline (rename, pickers, switch cells: real widgets earning their keep) last or
never.

### 4.5 Per-frame engine questions are per-revision facts

The K-184 family's rule, extended from builds to gestures: **a value that can only
change with the document is asked once per revision, never per frame — and during a
continuous gesture the per-frame sync budget for document-touching calls is zero.**
Named instances: `animated_mask_paths_at` (empty-at-this-revision short-circuits the
per-frame ask); `time_of_frame` during a scrub (carried on `sample_scalars`' existing
crossing, or pre-warmed in one span call); the §3.1 document-change walks (the
Viewer's footage facts, `LayerBounds`, the Project panel's per-comp settings) refilled
from the read model or asynchronously, never as per-item sync crossings inside a build
wave. `cached_frames` (0.02 ms, a cache-index read that genuinely changes per frame)
stays.

## 5. What does not change

- **The two-trees refusal stands** (docs/TODO, "The Timeline's two halves are still
  two widget trees"): outline and lanes stay two scroll views with the mirror;
  everything here works within that.
- **K-638's `LazyBlocks` window, K-647's one-call sampling, K-648's navigator,
  K-649's playhead layer, the work-area band layer** all stand — §4.3 refines
  LazyBlocks' insides and replaces nothing.
- **The budget tests' guarantees hold and tighten**: zero bridge calls in rebuild
  paths (`bridge_call_budget_test.dart`), bounded rebuild counts
  (`rebuild_budget_test.dart`) — new gates land beside them, none loosen.
- **Every drag behaviour in [timeline-interaction.md](timeline-interaction.md)** —
  staging in Dart, one undo step per drag, Escape-abandons, snapping, the marquee,
  the block tools — is behaviour, untouched by paint-layer or listenable plumbing.
- **The engine and the preview change nothing for this note.** The scrub's render
  requests, the cache bank, and the shared-texture present measured out of the
  frame-cost story (§2.3: fresh vs revisited spans differ by ~6 ms on the failing
  backend, ~0 on Skia); the +18 ms live-picture tax in §2.2 is the failing
  compositor's, and goes with WP-1.
- **Flutter stays a thin view** (K-181): everything here is about *when* the view
  redraws, never about deciding document truth in Dart.

## 6. The probe, parked

`flutter_ui/lib/probe/perf_probe.dart` — compiled out of reach unless the build passes
`--dart-define=LUMIT_PROBE_PROJECT=<path-to-.lum>` (plus `LUMIT_PROBE_OUT=<file>`).
It opens the project, fronts the comp named *Clips* (or the first comp), **prints its
conditions** (window physical size, dpr, whether media resolves — §2.1's trap made
that mandatory), drives the gestures through the real hit-test path, and writes §2's
raw rows: per-gesture `FrameTiming` aggregates, per-click detail, and per-name
bridge-call deltas. Run, from `flutter_ui/`:

```
flutter run --profile -d windows --no-pub \
  --dart-define=LUMIT_PROBE_PROJECT=<dir>/SetMeFreeLive.lum \
  --dart-define=LUMIT_PROBE_OUT=<dir>/probe_out.txt
```

To measure the owner's conditions rather than the trap: put the `.lum` copy in its
own folder with the media reachable beside it (for *Set me Free*: junction `Clips`
next to the copy — `mklink /J <dir>\Clips "...\Set Me Free Edit\Clips"` — and copy
`songcutfull.mp4`), and run with the window maximised. A copy alone in a folder is
the empty-preview condition — useful on purpose, misleading by accident; trust the
`media beside project:` line, not the intent. `--no-enable-impeller` runs the same
table on Skia (§2.4). Every work package below re-runs the probe before and after; a
package is done when its gate row holds. docs/13 §7.3 names it the manual instrument
for B1/B2; it is deleted the day a real-window CI harness supersedes it.

## 7. Work packages

Ordered; each sized for one agent; each gate measured by the probe **in the owner's
conditions (§2.1: maximised, live preview)** against the same comp, medians unless
said otherwise; where marked, also asserted in a widget test so CI holds it.

1. **WP-1 — Ship on the backend the numbers chose.** §4.1: the shipped Windows
   runner starts the engine on Skia; a build-time knob keeps Impeller one flag away;
   its own K entry with §2.4's numbers and the per-upgrade review rule; the upstream
   issue filed. *Gate (probe):* zoom fly and playhead drag — raster med **< 8 ms**,
   span p90 **< 16.6 ms**, effective rate **≥ 100 fps** (measured achievable: 99.5
   and 124.7); a release-shape build shows the same backend in its startup log.
2. **WP-2 — The click answers within the budget.** §4.4: layer selection into
   `TimelineSelection`; no `setState` in `_selectLayer`; rows and bars draw from
   their slice. *Gate (probe):* a first-visit outline click — worst build frame
   **< 8.3 ms** (was 39–67); revisit clicks unchanged at ~0. *Gate (CI):*
   rebuild-budget asserts a layer click rebuilds only the blocks whose slice changed
   (plus Effect controls' own subtree).
3. **WP-3 — Scroll is incremental.** §4.3, both halves. *Gate (probe):* wheel-scroll
   the Clips comp — build p90 **< 8.3 ms** (slide frames were ~70–75 ms), raster med
   **< 8 ms** on the pinned backend, span p90 **< 16.6 ms**, ≥ 60 fps effective.
   *Gate (CI):* a window slide builds only entering blocks; K-649's
   playhead-repaints-alone test keeps holding.
4. **WP-4 — Continuous gestures make zero per-frame document calls.** §4.5 for
   `animated_mask_paths_at` (empty-at-revision short-circuit + regression test) and
   `time_of_frame` on the scrub path (ride `sample_scalars`' crossing or pre-warm the
   span); audit the drag paths for siblings. *Gate (probe):* playhead and work-area
   drags — **0** sync document-touching calls per frame on this comp;
   `cached_frames` exempt.
5. **WP-5 — An edit's follow-on is one wave.** §4.5 for the document-change walks:
   footage facts into the read model (or async refill), `LayerBounds` lazy/async, the
   Project panel's per-comp `get_settings` per revision. *Gate (probe):* a switch
   toggle on the Clips comp — sync bridge time in the following second **< 5 ms**
   (was ~90 ms), no build frame > 17 ms, settled **< 250 ms** (was 0.7–0.9 s); the
   work-area release frame **< 17 ms** (was ~100–121 ms). *Gate (CI):* bridge-budget
   asserts a document-revision bump causes ≤ 1 model read and no per-item sync walk
   from any mounted panel.
6. **WP-6 — The matrix becomes gates.** §4.2 lands as paint-count assertions in
   `rebuild_budget_test.dart` per gesture, counted off `RenderRepaintBoundary`.

After WP-1..4 the arithmetic for a scrub frame in the owner's conditions reads:
build ~3.5 ms and raster ~5 ms overlapping across their two threads, zero sync
calls, measured spans of 9–10 ms already today minus WP-4's millisecond — the 60 fps
floor cleared everywhere with 6–8× headroom and the 8.3 ms budget met or at its edge
on every continuous gesture; WP-5 then removes the one remaining >17 ms frame class
(the edit commit). That is the mandate met, not approached.

## Open questions

- **The frame-pacing cap** (§1): revisit when Flutter exposes a preferred-frame-rate
  API on desktop; until then the cap is "fit in 8.3 ms and draw nothing at rest".
- **When the Skia pin comes off** (WP-1): the trigger is an Impeller release whose
  probe run matches Skia's numbers on this class of machine — re-run the §2.4 A/B per
  Flutter upgrade; the pin's K entry carries the review rule.
- **Why a republishing texture costs Impeller ~18 ms** (§2.2) is left undiagnosed on
  purpose: the pin removes it, and the upstream issue is where the diagnosis belongs.
  If the pin ever comes off, re-measure live-vs-empty before believing the new
  backend.
- **macOS and Linux were not measured** — the table is one machine's. The probe runs
  anywhere Flutter does; run it before assuming any conclusion travels.
- **The graph editor's own scroll** repaints its one painter per scrolled pixel
  (an `AnimatedBuilder` on the scroll controller) — acceptable today; if it ever
  misses its gate, §4.3's treatment applies to its gutter labels.
