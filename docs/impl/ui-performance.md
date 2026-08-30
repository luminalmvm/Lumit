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
backend looked like the lever and is not one — Skia is off the table by the owner's
ruling and Impeller has no faster configuration on Windows (§4.1) — so what is left to
us is *how much area re-records per frame* (§4.2, §4.3), never converting rows to a
canvas (§4.4).

### 3.4 The per-frame sync calls on scrub

Two, measured together at ~1.2–1.3 ms a frame — a sixth of the 8.3 ms budget:

- `animated_mask_paths_at` (0.5–1.3 ms): keyed per (comp, frame, revision), so a
  scrub re-asks every frame — to hear "no animated masks" on this comp every time.
  Whether *any* mask is animated can only change with the document: an empty answer
  is valid for the whole revision, whatever the frame.
  **Corrected by WP-4** (§4.5): on this comp the answer is *not* empty — four of the
  Clips comp's eight masks are keyed — so the empty short-circuit alone would never
  have fired. The condition that makes it free is that the interpolated shape is only
  ever *drawn* on an outlined layer.
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

### 4.1 The backend is Impeller, and what is left of the gap is the backend's own

The owner's ruling of 2026-08-30 settles what §2.4's table opened: *"we do not want skia
imo at all. we need to get impeller working at these high framerates no matter what."*
Skia is **not** a shipping backend for Lumit at any number. `--no-enable-impeller` keeps
one use only — a diagnostic reference that says how much headroom the hardware has — and
says nothing about what ships. So the shipped Windows runner starts the engine on
Flutter's default, which is Impeller's OpenGLES backend, and carries **no pin at all**:
the smallest runner is the one with nothing to say about backends (K-677).

**Vulkan is not reachable on Windows in 3.47** — read out of the engine, then confirmed by
running it. `impeller-backend` is a real switch (`shell/common/switch_defs.h`), parsed into
`settings.requested_rendering_backend` (`shell/common/switches.cc`), and consumed in
exactly two places in the tree: Android's `flutter_main.cc` and the headless tester. The
Windows embedder has one GPU compositor, `CompositorOpenGL`, built against
`impeller/renderer/backend/gles` alone (`shell/platform/windows/BUILD.gn`), over an ANGLE
display whose type is the literal `EGL_PLATFORM_ANGLE_TYPE_D3D11_ANGLE`
(`shell/platform/windows/egl/manager.cc`); the word *vulkan* does not occur in the
embedder's sources. Measured anyway, in the owner's conditions, with the switch pair set
through `FLUTTER_ENGINE_SWITCHES`: the engine prints `Using the Impeller rendering backend
(OpenGLESSDF)` and the table does not move (zoom fly 24.2 fps / 38.1 ms raster, against the
unswitched run's 19.3 / 49.7 — one run's noise, not a backend).

**Our own paint is not what costs.** The Impeller-expensive patterns were audited across
the whole Dart tree: **zero** `saveLayer`, zero `BackdropFilter`, zero `ShaderMask`, five
`Opacity`, seven rounded or path clips — in an entire editor. And the same probe run
carries its own control: Graph mode, which is **one** `CustomPaint`, rasters 33.8 ms where
the widget-built lane band rasters 49.7 ms in the same window with the same live preview.
Our content is worth ~16 ms of the ~50; the ~34 ms a single painter still costs is a floor
no widget-side saving reaches.

**What that floor is, from the embedder's source.** `CompositorOpenGL::CreateBackingStore`
gives Impeller a **4× MSAA** offscreen colour renderbuffer plus a 4× MSAA depth/stencil
renderbuffer at the full window size, and `Present` resolve-blits that whole surface into
the window every frame; the same function's Skia branch allocates a plain single-sample
texture FBO and blits that. Neither backend has damage or partial-repaint plumbing on
Windows — no such term appears in the platform directory — so every frame pays for the
whole window whatever changed. Hence a cost that is area-proportional and nearly
content-blind: 10.5 ms at 1280×720, 30–50 ms at 2560×1369, ≈ **8 ms per megapixel**, on a
card that should fill that area hundreds of times a second. The further ~18 ms a
*republishing* external texture adds (§2.2) sits on the same thread and stays undiagnosed
for a related reason: our side of that transport is one stable DXGI shared handle,
registered once and not re-registered while it plays (`viewer_texture_bridge.cpp`,
`viewer_texture_controller.dart`), and the embedder wraps rather than copies it
(`embedder_external_texture_gl.cc`, `TextureGLES::WrapTexture`) — so there is no per-frame
copy, format conversion or handle churn of ours left to remove.

**So WP-1 ships Impeller and does not meet WP-1's gate.** What it changes in the repo is
nothing: the runner has no pin, which is the configuration the ruling asks for. What it
leaves is §7.1's issue, drafted for the owner to file, and §7.2's follow-up package — the
mandate keeps pulling. The shipped configuration's text is Impeller's SDF path, which the
engine names in its own startup line (`OpenGLESSDF`); the pins that a rasteriser change
could have moved — `icon_crispness_test`, `ui_scale_test` — are pure arithmetic over stroke
widths and scale factors, backend-independent by construction, and CI's widget tests raster
through Skia whatever the app ships.

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
  builds the entering blocks and the two blanks, nothing else. **The cached block is
  also keyed by its index**, which turns out to be half the saving on its own: a
  `Column`'s children are matched to their elements *by key* once the list slides, and
  unkeyed blocks each land on the element that held their neighbour — a different
  layer, often a different shape — so the framework re-inflates rather than updates,
  cache or no cache.
- **A `RepaintBoundary` per block** (keyed by layer id) inside the band's boundary, so
  the raster thread records the entering block alone and scrolling inside the window
  is a pure layer translate.

This is the graph-view lesson applied where it actually bites — *stop walking widgets
per frame* — without converting rows to a canvas. The per-block boundary adds a layer
per block (~57 on screen maximised); if zoom-fly raster regresses from the added
layers (a flight re-records every block anyway), the boundaries are disabled during
flights — the probe decides, not taste.

**The per-block boundary landed with WP-2**, ahead of the rest of this section,
because the click needed it: with the layer selection listenable and each block
rebuilding in 0.1 ms, a select click still cost a **9.8–15.3 ms** frame, all of it
the band's single boundary re-recording fifty-seven rows to move the light on two
(§7's WP-2 row). It sits in `LayerBlock`, which both halves of the table build
through, so one line covers rows and bars. The flight worry above was then measured
rather than argued: with the layers in, zoom-fly raster **fell** 33.0 → 28.5 ms and
the playhead sweep 47.7 → 27.7 — less picture to re-record, not more — so no
flight-time disabling is needed.

**The first bullet landed with WP-3** (K-678, 2026-08-30) and the two halves of it
separate cleanly in the probe. Keying the blocks by index alone took the lane slide's
build p90 from **80.2 ms to 24.5**; reusing the widget instance on top of it took it to
**6.0 ms** (med 2.6, max 11.8) — inside the 8.3 ms budget, from thirteen times over it.
The band now re-records the rows that entered rather than the window, so raster med fell
with it, 42.5 → 32.6–34.8 across the two post-change runs. What the package does **not**
reach is the frame rate: 9.5 fps against the gate's 60, because ~33 ms of the ~63 ms span
is the window-sized raster floor WP-1 measured and could not move (§4.1, §7.2). The
UI-thread half of §3.2 is answered; the rest of that gesture's cost is the backend's.

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

**Two things the click paid for that this section had not seen** (found by
attributing the surviving frame with a stopwatch per builder, WP-2):

- **The menu bar built every row of every menu on every layer click** — 12.9–15.5 ms
  of it, most of it the Effect menu's entry per effect in the catalogue — because a
  row's enabled state reads the selection, so the bar listens to it, and its sections
  held their rows as built lists. A `MenuSection` now holds its rows as a closure the
  heading calls when it opens: a closed menu costs the record, an open one costs what
  it always did, on a deliberate press. That also took the bar's own document
  questions (`history`, `getKind`) out of the click — **9–16 sync calls, 1.3–2.8 ms**,
  from 14–22 and 4.3–12.5.
- **Fronting a panel that was already in front** repainted the shell and wrote the
  workspace to disk. `activatePanelTab` now says whether a tab actually moved and
  `frontPanel` only `touch`es when one did: the Effect controls are fronted on every
  layer click (docs/07 item 6.28) and are nearly always fronted already.

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
stays, and so does `render_frame` — asking for the picture is what a scrub *is*, not a
question about the document.

**Both named instances landed with WP-4** (K-679, 2026-08-30), and the shape each took
is worth recording because neither is the shape §3.4 assumed.

- `animated_mask_paths_at` is **not** the empty answer §3.4 assumed, and finding that
  out changed the fix. The Clips comp carries eight masks, **four of them keyed** (three
  on visible layers), so the call was doing real interpolation work every frame — it was
  never going to short-circuit on "nothing is animated". What makes it free is the
  second condition nobody had written down: the interpolated shape is *drawn* only on an
  **outlined** layer (`_GizmoPainter`'s `maskedBoxes`, and `_editablePointBoxes` for its
  points — both the selection). So the ask is made when a mask is keyed **and** its
  layer is outlined, both known here for nothing: `BridgeMask.pathKeys` rides in the
  read model, and the outline set moved onto `LumitUiState.outlinedLayerIds` so the
  gizmo that draws from it and the stage that asks from it cannot drift apart. A scrub
  with those three layers unselected — which is every scrub that is not editing them —
  now costs nothing; select one and it is asked per frame again, because its vertices
  genuinely differ frame by frame. The flag goes *into* `AnimatedMaskPaths.refresh` and
  into its memo key, so both ways it can go false (the last path key deleted, the layer
  deselected) empty the held copy rather than leaving a mask drawing a shape it no
  longer has, and flipping it true mid-frame re-asks rather than serving the memo.
- `time_of_frame` could not ride `sample_scalars`: that call takes the time as an
  argument, so the conversion has to have happened before it is made. It is
  **pre-warmed a page at a time** instead — a new `times_of_frames(first, count)`
  returns 512 consecutive exact times in one crossing (capped engine-side, so a silly
  span is trimmed rather than allocated), and a miss in `comp_time.dart` warms the page
  the frame lands in. A scrub crosses the seam once per ~8 seconds of 60 fps footage
  instead of once per frame it has not visited.

**What WP-4 did not remove, and why it is WP-5's**: the read model's revision check,
one `document_revision` per frame of a zoom fly (96 calls over 96 frames, 4.7 ms — 0.049
ms a frame, the whole per-frame bill of that gesture). `CompModel._freshen` can skip it
while a frame is being built, on the argument that a frame cannot see the document move:
a change arrives on the stream between frames and calls `refresh`, and a panel that
commits its own op calls `refresh` too. **Tried, measured green on the probe, and
reverted** — 17 tests across `timeline_panel_frb_test` and `timeline_extras_frb_test`
fail, because a `pump()` that follows an op without turning the event loop never
delivers the stream event and the per-frame check is what they were relying on. That is
the same "an edit's follow-on is one wave" problem WP-5 owns (§7 item 5), and it should
be answered there once, not smuggled in here at the cost of the suite that guards it.

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
  backend, ~0 on Skia); the +18 ms live-picture tax in §2.2 is the compositor's, and
  outlived WP-1 — §4.1 shows our end of that transport has nothing left to give, so it
  travels to §7.2 with the rest of the gap.
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

1. **WP-1 — Impeller at mandate speed. Done as far as it goes; its gate is not met**
   (K-677, 2026-08-30). §4.1: the shipped Windows runner takes Flutter's default —
   Impeller GLES, no pin, no knob — after Vulkan was ruled out of 3.47 on Windows from
   the engine's own sources and then measured inert, the Dart tree was audited clear of
   Impeller-expensive paint, and the residual cost was located in the embedder (4× MSAA
   offscreen + whole-window resolve blit per frame, no partial repaint, ≈ 8 ms/Mpix).
   *Gate (probe):* zoom fly and playhead drag — raster med **< 8 ms**, span p90
   **< 16.6 ms**, ≥ 100 fps. **Measured after: 49.7 / 97.0 / 19.3 and 35.0 / 68.9 /
   26.2 — unmet, by the backend, not by Lumit.** The pursuit continues as §7.2; the
   issue that would move it upstream is §7.1, drafted and unfiled.
2. **WP-2 — The click answers within the budget.** §4.4: layer selection into
   `TimelineSelection`; no `setState` in `_selectLayer`; rows and bars draw from
   their slice. *Gate (probe):* a first-visit outline click — worst build frame
   **< 8.3 ms** (was 39–67); revisit clicks unchanged at ~0. *Gate (CI):*
   rebuild-budget asserts a layer click rebuilds only the blocks whose slice changed
   (plus Effect controls' own subtree).
   **Landed 2026-08-30.** Worst build frame on a first-visit click **5.1–5.4 ms**
   (9.3 ms on a session's very first select, where the caches are cold), from
   38.7–66.8; sync calls in the click path **9–16 for 1.3–2.8 ms**, from 14–22 for
   4.3–12.5; a revisit still draws no frame at all. Three costs, in the order they
   were found: the panel-wide `setState` (§4.4), the menu bar's rows and the
   already-fronted panel front (§4.4's addendum), and the band's single repaint
   boundary (§4.3, brought forward). The last two were invisible to the widget
   counters — 600–950 rebuilds a click, of which the rows and bars are 4 — and
   turned up only by timing each builder in the running app.
3. **WP-3 — Scroll is incremental. Landed 2026-08-30 (K-678); its UI-thread gate is
   met and its raster gate is WP-1's, still unmet.** §4.3, both halves. *Gate (probe):*
   wheel-scroll the Clips comp — build p90 **< 8.3 ms** (slide frames were ~70–75 ms),
   raster med **< 8 ms** on the pinned backend, span p90 **< 16.6 ms**, ≥ 60 fps
   effective. **Measured after:** lane slide build p90 **5.96 ms** (med 2.62, max
   11.81), from 80.18 (med 3.00, max 100.68); the outline half 6.46 from 77.32; raster
   med 34.75 from 42.48, span med 62.85 from 81.65, 9.5 fps from 8.2. The build gate is
   met with room; the raster, span and fps rows are the ~33 ms window floor of §4.1 —
   the backend pin the gate's wording assumed does not exist (K-677), and Graph mode's
   single painter rasters the same in the same window. Nothing widget-side reaches
   them; they travel to §7.2. *Gate (CI):* a window slide builds only entering blocks
   — `rebuild_budget_test`'s "a scroll builds the rows it brings in, not the whole
   window": 3 rows and 3 bars on a slide of two rows, from 28 and 28. K-649's
   playhead-repaints-alone test keeps holding.
4. **WP-4 — Continuous gestures make zero per-frame document calls. Landed
   2026-08-30 (K-679); its gate is met on the drags it names.** §4.5 for
   `animated_mask_paths_at` and `time_of_frame`, and the drag paths audited for
   siblings. *Gate (probe):* playhead and work-area drags — **0** sync
   document-touching calls per frame on this comp; `cached_frames` exempt (and
   `render_frame` with it: asking for the picture is the gesture, not a question
   about the document). **Measured after**, the owner's conditions, medians:

   | Gesture | Before — calls / ms, per-frame name | After |
   |---|---|---|
   | Playhead drag, fresh (92→75 frames) | 301 / **74.0 ms** — `animated_mask_paths_at` ×90, 65.2 ms | 174 / **4.2 ms** — **×0** |
   | Playhead drag, revisited | 307 / 65.5 ms — ×93, 61.1 ms | 180 / **4.4 ms** — ×0 |
   | Work-area drag | 554 / 143.6 ms — ×75, 42.0 ms | 433 / **129.7 ms** — ×0 |
   | Graph-mode playhead drag | 310 / 60.5 ms — ×95, 56.4 ms | 169 / **4.3 ms** — ×0 |
   | Zoom fly | 96 / 4.7 ms — `document_revision` ×96 | 75 / 2.1 ms — ×75, **unchanged** |

   Everything left on the two playhead rows is exempt or a 2 Hz status timer;
   `time_of_frame` was already zero on the probe's sweeps (the zoom-to-fit warm-up
   §3.4 names) and is now zero on a cold one too, pinned in CI instead. The work-area
   row's remaining 130 ms is the release commit's walk, which is WP-5's. **The zoom's
   one revision check a frame is not removed** — §4.5 says why, and it is WP-5's as
   well. Frame timings across these runs moved by ±6 ms of raster in both directions
   and are §4.1's window floor, not this package's: nothing here enters a paint path.
   *Gate (CI):* `bridge_call_budget_test` — "a mask is asked about only while its drawn
   path moves" (0 calls still, 30 selected-and-keyed, 0 keyed-but-unselected) and
   `time_of_frame` pinned at **0** on a scrub, including one whose memory was emptied
   first.
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

### 7.1 The upstream issue, drafted (WP-1) — not filed

Ready to file at `flutter/flutter` when the owner chooses to; nobody posts it on the
owner's behalf. Re-measure before filing if the Flutter version has moved.

> **Title:** Impeller (OpenGLES) on Windows: ~25 ms raster per maximised frame vs
> Skia's ~5 ms, +18 ms more while an external texture republishes
>
> **Flutter:** 3.47, Windows 11, `flutter run --profile -d windows`.
> **Machine:** RTX 5080, 2560×1440 @ 165 Hz, OS scale 1.0 (dPR 1.0).
> **App:** a desktop editor with a timeline panel of ~57 visible rows and a Viewer
> showing engine-rendered frames through an external texture (`FlutterDesktopTexture`,
> D3D shared surfaces).
>
> **What happens.** With the default backend, every continuous UI gesture (Ctrl+wheel
> zoom, a playhead drag, a handle drag) spends 30–49 ms on the raster thread per frame
> at a maximised window, landing many vsyncs late — 20–30 fps. UI-thread build times
> for the same frames are 4–11 ms, so the app is not the bottleneck. Passing
> `--no-enable-impeller` runs the identical build and identical gestures at 99–146 fps
> with 5 ms raster.
>
> | Gesture (maximised, live preview) | Impeller GLES — fps / raster med / span med | Skia — fps / raster med / span med |
> |---|---|---|
> | Ctrl+wheel zoom | 19.7 / 48.8 ms / 94.2 ms | 99.5 / 5.1 ms / 11.2 ms |
> | Playhead drag | 19.9 / 47.7 ms / 93.6 ms | 124.7 / 5.3 ms / 10.2 ms |
> | Handle drag | 30.2 / 30.3 ms / 59.5 ms | 145.3 / 5.0 ms / 9.2 ms |
> | Single-`CustomPaint` view, same gestures | 27.9 / 33.0 ms / 63.7 ms | 114.3 / 4.8 ms / 9.9 ms |
> | Frames > 17 ms, one 2.5 s drag | 72 of 72 | 0 of 409 |
>
> **Two separable costs, both raster-thread.** (1) *Window area:* the same gesture in a
> 1280×720 window rasters 10.5 ms and at 2560×1369 rasters 30.0 ms, with nothing else
> changed — ~3×, roughly area-proportional, while Skia's stays ~5 ms at both.
> (2) *A republishing external texture:* with the same maximised window, media missing
> so the texture presents a static placeholder, raster is 30.0 ms; with the texture
> republishing each rendered frame it is 48.8 ms — **~18 ms per frame** for the presence
> of new texture content. Skia pays ~nothing for the same traffic (5.1 vs 5.0 ms), and
> the split is the same whether frames are freshly rendered or replayed from a cache,
> which points at the compositor's handling of the updated texture rather than at
> production of the frames.
>
> **Repro sketch.** A maximised Windows window; a scrolling/zooming widget band covering
> most of it; an external texture updated every frame via
> `TextureRegistrar::MarkTextureFrameAvailable`; drive a continuous pointer drag and read
> `FrameTiming.rasterDuration`. The ~3× window-area term reproduces without the texture;
> the ~18 ms term needs the texture republishing.
>
> **Two embedder details that look implicated**, from
> `shell/platform/windows/compositor_opengl.cc`: with Impeller the backing store is a
> 4× MSAA offscreen colour renderbuffer plus a 4× MSAA depth/stencil renderbuffer at
> full window size, resolve-blitted to the window in `Present` every frame, where the
> Skia branch of the same function uses a single-sample texture FBO; and the Windows
> embedder has no damage/partial-repaint plumbing on either backend, so every frame
> pays the whole window. The measured cost works out at ≈ 8 ms per megapixel.
>
> **Impact.** The app ships on Impeller by choice and takes a factor of 5–7 in interface
> frame rate for it on a high-end GPU — its 60 fps interface mandate is unreachable on
> Windows today, and `--no-enable-impeller` is kept only as a reference measurement.
> Happy to re-run this A/B, or a narrowed one, against any build you would like numbers
> from.

### 7.2 WP-7 — the Impeller gap, still open

WP-1 ran out of levers inside Lumit, not out of mandate. What is left is a raster-thread
floor of ~34 ms per maximised frame that a single `CustomPaint` pays as surely as 57
widget-built rows, plus ~18 ms more whenever the Viewer's texture republishes. The
package that reopens it, in the order evidence would be gathered:

- **Re-measure on each Flutter upgrade** — the §2.4 A/B and the §2.3 table, unchanged
  method, one run each. Impeller is under active work upstream; the cheapest possible
  fix is someone else's commit. Check specifically whether the Windows embedder has
  grown a Vulkan compositor (`shell/platform/windows/BUILD.gn` gaining
  `impeller/renderer/backend/vulkan`) or partial repaint.
- **File §7.1** (the owner files it; nobody posts on their behalf) and carry the reply.
- **Narrow the ~18 ms texture term to a repro outside Lumit** — a bare Windows Flutter
  app, one `Texture` widget over a D3D11 shared handle marked available each frame, a
  full-window gesture. Our end is already minimal (§4.1), so the value here is a
  standalone case an engine engineer can run, not another change to our transport.
- **Try the MSAA hypothesis where it can be tried** — a local engine build with the
  Impeller backing store made single-sample says in one number how much of the floor is
  the 4× MSAA offscreen and its resolve. That is an engine experiment, not a Lumit
  change, and it belongs to whoever answers §7.1.

Until one of those moves, WP-2..WP-6 stand on their own: they are the frames Lumit is
responsible for, and each is measured against the backend that ships.

## Open questions

- **The frame-pacing cap** (§1): revisit when Flutter exposes a preferred-frame-rate
  API on desktop; until then the cap is "fit in 8.3 ms and draw nothing at rest".
- **When Impeller reaches the mandate on Windows** (WP-1, §7.2): unanswered, and the
  answer is not in Lumit's tree — re-run the §2.4 A/B per Flutter upgrade and read the
  Windows embedder's compositor for a Vulkan backend or partial repaint. Skia stays a
  reference measurement, never a shipping backend (K-677).
- **Why a republishing texture costs Impeller ~18 ms** (§2.2) is left undiagnosed on
  purpose: the pin removes it, and the upstream issue is where the diagnosis belongs.
  If the pin ever comes off, re-measure live-vs-empty before believing the new
  backend.
- **macOS and Linux were not measured** — the table is one machine's. The probe runs
  anywhere Flutter does; run it before assuming any conclusion travels.
- **The graph editor's own scroll** repaints its one painter per scrolled pixel
  (an `AnimatedBuilder` on the scroll controller) — acceptable today; if it ever
  misses its gate, §4.3's treatment applies to its gutter labels.
