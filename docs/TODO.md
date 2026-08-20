# TODO - the work backlog

**Status: living.** The single source of truth for work that is planned but not
done, and the one document that says what is built. The specs describe the
target; gaps live here.

**How to use it.** Keep entries to one line plus a source pointer. Move an item
up the sections as it becomes actionable; **delete it when it lands** - its
regression test is the permanent record, per
[14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md). Landed work does not belong
in a backlog. [16-ROADMAP.md](16-ROADMAP.md) stays the aspirational phase plan;
this file is the concrete backlog underneath it.

---

## Now - the preview must keep up

These sit above everything else: they are what the editor feels like in the hand.

- **The flare's raster still draws the cells it culled.** After K-263 a batch
    draws exactly its own cells, but a cell the guards kill is still stored and
    still submitted as a degenerate off-screen triangle. Compacting to just the
    live cells would cut the vertex work again; it must be a **prefix-sum**
    compaction, not an atomic append, because additive blending is float
    addition and the drawn order has to stay fixed or the frame stops being
    bit-stable (docs/impl/lens-flare.md §2.4). Measure the live fraction first.
    Same shape of win in Matte mode from skipping dead light slots with an
    indirect dispatch: eight slots are always dispatched, however many sources
    the detection actually found.
- **Replace `poll(Maintain::Wait)` with a keyed mutex** - every present waits for
    the card to go idle before handing the texture over (`shared.rs`,
    `shared_linux.rs`, `shared_metal.rs`; find it by the call, not a line number).
    Playback from the card has slack to hide the stall; playback from memory does
    not. **Its own branch and its own pull request** - surgery on the
    shared-texture chain, where a mistake shows as tearing, not as an error.
    Measure first: the 2026-07-30 fixes may have made it moot. Not a revival of
    the read-back transport (deleted in K-183) - the Viewer receives a GPU handle
    and nothing else.
- **Playback's remaining bridge chatter scales with rows on screen** - one
    `sample_scalar` per animated row plus one `time_of_frame`. Batch per frame if
    it ever bites, the way `time_of_frame` already was.
    (`bridge_call_budget_test.dart` is the gate.)

---

## Now - the effect registry (K-381, docs/impl/effect-registry.md §6)

The migration is done. All 35 built-ins declare themselves in
`lumit-core/src/fx/effects/`, one file each; `catalogue.rs` generates both halves of the
catalogue from one list; a frame resolves every effect through one generic loop into the
arena, and `run_ops` and `cpu::apply_stack` dispatch by name with no match over effects
left in either. `Resolved`, `ResolvedOps`, `resolve_one`, the free `rescale_px` and the
hand-written `BUILTINS` literal are all deleted, and with them the migration-only
`the_generated_schema_matches_the_hand_written_one`. What is left is §6 step 5.

- **Dynamic parameters** - derived from a custom shader's uniforms or a node graph's exposed
    inputs; then **spare parameters**, the user's own sliders for expressions to read. The
    rules are settled (§4 of the note); the panel affordances are not built.
- **Bridge and panel**: `list_parameters` and the Effect Controls read the schema, so they
    follow for free - except for dynamic parameters, which are per *instance* rather than per
    effect and need a bridge call that takes an instance id.

- **Rescale a derived spatial value, or stop deriving one.** Not a migration step - an open
    defect the migration uncovered. Scanlines' `derived.roll_px` is
    in raster pixels, but `ResolvedStack::rescale_spatial` only moves values whose id matches
    a schema parameter with a spatial unit - a derived id matches nothing, so a stack resolved
    against one raster and reused at another (`realise.rs`, a precomp at a different size)
    scales the line period and leaves the roll offset behind, and the pattern's phase shifts
    with the size. Before K-385 neither moved, so the phase was right and the period wrong;
    now it is the other way about. The Lens flare's `derived.light*` entries are the same
    shape - raster pixels under a derived id - though there the old `rescale_px` match did
    not move them either, so a Lights-mode flare on a resized precomp is no worse than it
    was and no better. Two fixes, both small: give `EffectDef` a
    `derived_spatial()` list the rescale pass consults (keeps the resolve maths bit-identical),
    or derive the roll in *periods* rather than pixels and multiply in `packed()` (no new API,
    but the f64 product rounds once more and so is not bit-identical to the old arm).

---

## Now - Flutter frontend parity and regressions

Flutter is the only frontend (K-174, K-182); git history is the parity reference.
These are v1-scope surfaces it does not yet match.

**Viewer bar ([07-UI-SPEC.md](07-UI-SPEC.md) §2.2):**
- The wireframe/overlay *menu*; guides menu; region-of-interest.
- **The colour-management badge is a readout and cannot yet be clicked**
    (§2.2 item 8). It is built: always on the bar, naming the display
    transform, and saying the picture is not the export while the exposure or
    the tone map is engaged (K-314). §2.2 also asks that clicking it open
    colour settings — there are none to open, so it is plainly not a control
    rather than a button that does nothing. It names the one built-in transform
    pair (scene-linear → sRGB) as a constant; when the transform becomes a
    choice (docs/06 §3.3's OCIO slot) this is the readout that must follow it.
- **Tone mapping's explanation now lives in the badge's tooltip** and nowhere
    else on screen (owner, 2026-08-08). The toggle itself is an icon whose own
    tooltip is its name, because §13.2 keeps tooltips to that; the badge is a
    readout, which §13.2 does allow a sentence, so that is where "what this
    does" went. If hints of this kind ever get a home of their own, this is
    still a candidate to move.

**Toolbar tools ([07-UI-SPEC.md](07-UI-SPEC.md) §1.7):** what is armed is a
*tool*; what each tool then does is the backlog.
- **Razor** - a Sequence layer's eased ramps refuse a cut (`UncuttableClip`).
- **Shape layers** ([impl/shape-layers.md](impl/shape-layers.md)) - owed: nested
    groups and the shape **modifiers** (repeater, trim paths, wiggle, offset
    paths), gradient fills, dashed strokes, joins and caps other than round, and
    animated paths.
- **Path editing on the picture** - mask and shape-layer points drag (K-224,
    K-307). Still owed: a **paint stroke's** points, which are a stored gesture
    rather than a path and so are their own piece of work; no path's bezier
    **handles** can be dragged, so the `Alt`-drag that re-links a broken tangent
    pair exists only while a point is being *placed* - and the model has no
    linked/broken flag, so adding one is a
    [03-DATA-MODEL.md](03-DATA-MODEL.md) change and a decision, not just a
    gesture; and the Pen's add/delete/convert-vertex siblings and dragging a
    whole path by a segment.
- **Mask paths have no per-key op.** `SetLayerMasks` rewrites the whole list for
    every keyframe drag, so one drag is one undo step only because the drag is
    staged - a per-key op would make it so by construction (K-344). **Lighten**
    and **Darken** are the two mask modes still unbuilt, and feather is uniform:
    the variable-width, per-vertex kind is a model change
    ([03-DATA-MODEL.md](03-DATA-MODEL.md) §7).
- **Variable-width mask feather** (K-338) - After Effects has had this since CS6:
    the **Mask Feather Tool** (`G` cycles onto it, under the Pen) drops *feather
    points* along an existing mask path, each dragging its own radius in or out,
    so one edge of a mask can be razor-sharp and another 200 px soft. It is what
    a sky replacement wants - crisp along the horizon, blending away at the
    corner. It needs a second point set on the path, its own tool, and a
    rasteriser that varies the ramp width along the boundary rather than using
    one number. `ToolMode.penMaskFeather` already exists in the toolbar as a stub
    with an icon and a string and nothing behind it.
- **Type** - vertical type (needs `lumit-text` to lay a line downwards); true
    glyph metrics across the bridge (the caret, the anchor and the gizmo all use
    the same half-an-em estimate, and one measured advance width would replace
    all three); multiple lines and a character panel (font, tracking, leading,
    alignment - the document is one styled run, [03-DATA-MODEL.md](03-DATA-MODEL.md)
    §9.1); per-character and per-word animators.
- **Paint** (brush/clone stamp/eraser, [impl/paint.md](impl/paint.md)) - owed:
    **pressure and tilt** from a tablet, **brush shapes** other than round,
    **spacing** and **scatter**; **write-on** (a stroke's own start and end times,
    which is what makes paint animate in After Effects - nothing in the model
    yet); **per-stroke blending modes**; painting in **Layer view** rather than on
    the composite; **a GPU stamping path** (the rasteriser is a CPU loop beside
    the mask one, and it changes the rasteriser, not the stored stroke); and
    **paint on a Precomp layer's nested pixels**, which never come back to the
    CPU, so a stroke on one currently marks nothing.
- **Camera** - a separate point of interest (AE's two-node camera) is an engine
    change; the Unified Camera tool; depth-of-field handles on the picture; a
    keyframed camera cannot be dragged (no single value to add to); a drag
    spanning several layers is one undo step per layer, because no op carries
    edits to more than one.
- **Roto** and **Puppet** - disabled on the strip until there is an engine behind
    them ([16-ROADMAP.md](16-ROADMAP.md)). Roto wants a segmentation model and
    per-frame stroke propagation; Puppet wants a mesh, pins and a deformer.
**Smooth zooming everywhere else.** The shared helper is built
(`widgets/smooth_zoom.dart`, K-293) and the Timeline reads it. Still cutting
rather than flying: the **graph editor's** zoom and auto-fit — a matter of
holding a `SmoothZoom` and reading its value, with no design left in it.

**Layer controls in the Viewer ([07-UI-SPEC.md](07-UI-SPEC.md) §2.3):**
- **Motion paths** (§2.4) - a keyed position draws no path and its keys cannot be
    dragged there.
- **Scale and rotation of a multiple selection** - each layer keeps its own box
    and only a lone selection grows handles; AE scales a set about one shared box.
- **Snapping** - nothing outside the Timeline's keyframe magnet snaps to
    anything (§4.5, §1.7).
- **Parent-aware and 3D gizmos** - the box is built from the layer's own
    transform, so a parented layer's ignores its parent and a 3D layer's ignores
    the camera.
- **A keyframed position draws no box**, so an animated layer cannot be picked on
    the picture. It wants the value *at the playhead*, which the read model does
    not carry.

**Pixel pickers ([07-UI-SPEC.md](07-UI-SPEC.md) §6.1):**
- The x/y coordinate pick - no Flutter row pairs x and y into one control yet
    (the magnifier already carries the mode).
- The on-Viewer crosshair handle for point parameters - a point parameter can be
    picked but not dragged on the picture.

**Bridge ([17-BRIDGE-CONTRACT.md](17-BRIDGE-CONTRACT.md)):**
- **A panic throws rather than reporting.** frb contains every panic but surfaces
    it as a thrown Dart exception, so no call site may treat a throw as
    impossible. The `no-panics-in-frb-api` grep is prevention, not a fix.
- **clippy is blind to the frb surface** - `#[frb(...)]` is a proc-macro
    attribute and restriction lints skip macro-expanded code, so
    `unwrap_used`/`panic`/`todo` never fire on an annotated function. The real fix
    is to stop needing the grep.
- **`ProjectReference::state()` hands the raw `Arc<RwLock<…>>` out**, so a caller
    can hold a project lock as long as it likes and in any order. The order is
    written down and tested; nothing enforces it at the type level.
- **The macOS IOSurface Viewer path is unproven** - CI links the bundle but
    nobody has launched the .app (K-033).
- **The macOS .app is not relocatable** - the podspec links keg-only FFmpeg by
    absolute Homebrew path. Distribution needs the dylibs vendored and install
    names rewritten (K-033).
- **The macOS build is single-architecture** - `pkg-config-rs` refuses to
    cross-compile and a keg holds one architecture, so `ARCHS` is pinned to the
    runner's. A universal bundle needs both `ffmpeg@7` kegs and per-slice `-L`
    flags (K-033), plus a decision on whether Intel macs are supported at all.
- **The iOS podspec is misnamed** - `rust_lib_lumit_flutter` against a pubspec
    name of `lumit_bridge`. Same fix macOS took; iOS has no target and no CI job.
- **The shared-texture chain has no keyed mutex** (a torn frame is possible in
    principle), and the D3D12 → D3D11 legacy-handle hop the Windows path rides is
    not described in [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md).
- **The Scopes' trace crosses the bridge as pixels**, a byte at a time, and is a
    fixed 256×256 whatever the panel size - so a large Scopes panel shows it
    visibly soft. It could take the shared-texture route and a size that follows
    the panel.
- **The matte render-alone pass stays at full comp resolution** whatever the
    preview scale (K-186) - correctness-safe, but it is the one composite the
    scale does not shrink.
- **The Linux DMA-BUF path has never run on a Linux machine with a GPU** (K-033).
    It fails calmly on the adapter-less CI runner, which proves the failure is
    calm and nothing about the path working.
- **frb's SSE codec encodes `Vec<u8>` one byte at a time** - now taxes only
    thumbnails and scope traces, but worth the bulk codec if traces feel late.
- **Engine subsystems with no frb API** - the Retime **graph**
    (`with_segment_ease`, `with_segment_speeds`, `with_segment_as_rate` in
    `lumit-core/src/retime.rs`) and the curve view that makes ramps editable;
    `trim_to_source_end`.
- **The audio mix is rebuilt from scratch** whenever the comp's audio signature
    changes, rather than patched.

**Retime follow-up after K-249.** **The eased ramp shapes are gone from
clips** — `Clip::with_ramp` takes two speeds and runs straight between them,
which is what the envelope authors. Slow/Fast/Smooth/Sharp come back with the
preset-shelf rework below, rebuilt on the property like everything else K-249
moved.

**Video memory is only read on Windows.** `video_memory_bytes` answers the
first DXGI adapter's dedicated memory there and 0 everywhere else, so the GPU
cache ceiling falls back to the frontend's documented figure on macOS and
Linux. Wants Metal's `recommendedMaxWorkingSetSize` and the Vulkan adapter's
device-local heap (K-033).

**Appearance.** The seven built-in schemes still restate every colour
individually; only the two Timeline tokens default from the mode. Owed after
K-298: a swatch strip per row **inside** the picker's menu (it previews the
selection only), and a place to keep themes other than the workspace file, so an
imported theme travels with the user rather than the machine's settings.

**Shell and onboarding:**
- **The boot splash says only what `boot_log` says.** It is mounted now
    (`BootGate` in main.dart) and streams the engine's own boot log, which is
    all the engine can tell it: there is no notice stream to subscribe to, so a
    module that took a long time coming up, or came up degraded, cannot say so
    on the splash. Wants an engine-side boot event stream before it can.
- **Pop-out panel windows are removed** (K-182). Rebuild from git history
    (`flutter-frontend-alternative`, pre-K-182) when pop-out is wanted, and land
    it wired end to end.
- **Workspace machinery beyond the presets** ([07-UI-SPEC.md](07-UI-SPEC.md)
    §1.6) - user workspaces (save-as/rename/export), the chrome switcher strip,
    and Alt+Shift+1-9.
- **First-run setup screen: the four-card version** (K-006, K-246,
    [07-UI-SPEC.md](07-UI-SPEC.md) §13.1) - §13.1's four cards, a small image over
    each choice. The plain one-question screen is built
    (`shell/first_run_frb.dart`).
- **Command palette** - recents are session-lived, and only genuinely bound
    shortcuts are taught (today just undo/redo).

**Timeline panel:**
- **Retime in the graph editor** behaves exactly as any other property - same
    value and speed graphs, nothing extra. Retime-specific affordances come later
    (see *Retime UI wiring* under Next); the parity rule itself is spec, and lives
    in [04-RETIMING.md](04-RETIMING.md).
- **The Timeline's two halves are still two widget trees, and one vertical
    scrollable cannot hold both.** This was once written down here as a session's
    refactor — build each layer as a row holding both halves inside a single
    vertical scrollable, and alignment holds by construction. It does not work,
    and the reason is worth keeping so it is not re-derived. The ruler and the
    cache bar scroll sideways with the lanes but must not scroll with the rows,
    which means the lanes' horizontal scroll view has to sit *above* the vertical
    one; a single `Scrollable` has a single subtree, so everything inside that
    vertical scroll view then scrolls sideways with the lanes — including the
    outline, which must not move (`timeline_alignment_test.dart` says so, and the
    outline has a horizontal scroll of its own for narrow panels). Putting the
    horizontal scrolls underneath instead gives one per row, and `_hLane` asserts
    the moment a second position attaches to it, which is what `_positionOf`
    exists to survive. The only arrangement that satisfies both is to drop the
    lanes' horizontal *viewport* and offset them by a transform, with `_hLane`
    anchored on the ruler band — and that costs horizontal trackpad panning over
    the lanes, the very fault the `dragDevices` comment in the panel records as
    invisible to anyone using a mouse. Not worth it. `blockHeights` stays
    whichever way it goes: `layerDropSlot`, `layerDragTarget` and `LayerDragSlide`
    each want every block's height, not one row's. What the merge was really
    reaching for has landed instead — each layer is now decided **once** into a
    `LayerRow` (its fold rows, its open Sequence view, its height) and both halves
    read that, so they can differ in what they draw but no longer in what a layer
    is. The scroll mirror and its guard flag stay.
- **The lane keyframe selection selects and eases, nothing more** - moving or
    deleting a *whole lane selection* is not built (the graph view has both), nor
    are `=`/`-`/`\` or edge-follow during playback.
- **Column widths and the property selection are session-lived** - fold into the
    workspace when per-workspace column layouts land ([07-UI-SPEC.md](07-UI-SPEC.md)
    §4.2).
- **~4 order-dependent tests in the Flutter suite.** Each passes alone; the suite
    passes at `--concurrency=1`, which is what CI runs. They contend for the
    shared engine (audio device, render worker) across test *files*. Give those
    files a serial marker or make the engine per-file - the serial run is a
    mitigation, not the fix, and it costs wall-clock. A concrete signature,
    measured 2026-08-12 *within* `viewer_panel_frb_test.dart` alone on the
    owner's machine: five frame-arrival tests fail late in the file with
    "Could not create the renderer … device request failed: Not enough memory
    left" — the workers the earlier tests spun up exhaust the device, so the
    failing set shifts run to run and every member passes alone. Whatever fixes
    the contention should make that message impossible, not rarer.
    The job itself is healthy again: the `flutter frontend (Linux build + analyze
    + test)` job could not complete at all mid-PR #97 (every run reached
    `timeline_panel_frb_test.dart`, logged `vkAllocateMemory failed`, and was
    killed), and PR #97 merged with it green after two root-cause fixes — a
    project that stops being shown is closed and its render worker stops with it
    (`ProjectReference::close()`, so per-test workers and GPU devices no longer
    pile up), and the frame-name memo is cleared on `SetViewerLook`. So the Dart
    suite *is* verified on CI; what remains open is the order-dependence itself,
    for which `flutter test --concurrency=1` (ci.yml) is still the mitigation.
- **Beat tap has no key left** - [07-UI-SPEC.md](07-UI-SPEC.md) §10 wants `8`
    during playback to tap a beat, and K-254 gave the bare digits to the numbered
    markers. Needs its own chord or a modal reading.
- **Snapping covers the lane key drag and the razor only** (K-292). Still landing
    where the pointer puts them: the layer **bar** drag, the work-area handles
    and marker drags. The arithmetic is shared and pure
    (`panels/timeline_snap.dart`), so each is wiring rather than design.
- **Volume keyframes draw no lane diamonds and no graph curve** - volume is not
    in the comp read model; fold it into `BridgeLayerInfo` if either matters.

**Render-time indicator follow-ups (K-276 landed the column).** What ships measures by
*fencing* — the render waits for the card at each layer and each effect before reading
the clock, and re-renders held frames to do it — so it is opt-in and never runs during
playback. §7.1's target is continuous collection at negligible cost, which wants **GPU
timestamp queries**: a query set per frame, timestamps written around each node's own
submission (every effect kernel already submits its own command buffer, so nothing
inside `lumit-gpu`'s kernels changes), resolved a frame later. With those in the switch
could go. Also owed from §7.1: **sorting** the Timeline column, a **profiler panel**
with the recording mode (totals, percentiles, cache hit rates, time per
degradation-ladder step), and per-layer numbers for the layers *inside* a Precomp.

**The preview progress bar's fractions are stage weights, not measurements**
(K-276). Decode is assumed the long pole and each top-level layer an equal share of
the composite, so a comp whose one adjustment layer outcosts the twenty layers below
it fills unevenly; feeding the profiler's measured costs back as the weights is the
fix. Also unbuilt: an **export**'s progress still has its own path
([07-UI-SPEC.md](07-UI-SPEC.md) §14) rather than sharing this one.

## Next - engine/bridge follow-ups

**The displacement class still takes the generic matte.** K-395 names
"displacement-class effects scale their vectors before sampling" as an override
worth having, beside the four that landed (Gaussian blur's radius, Glow's seed
gate, Depth of field's depth, the Lens flare's source detection). Turbulent
displace and its neighbours currently take the strength dissolve, which for a
displacement is the *veil* failure the blur had: the pixels still moved the full
distance and the result is faded back over them, rather than moving less far.
The hook is in place and the change is per effect — declare
`matte = ("matte", "<what it means>")`, read `aux.matte()` in the kernel, scale
the vector, update the oracle op-for-op (docs/impl/effect-registry.md §2.5b).

**The Lens flare's Matte row has no Invert.** Every other matte row carries one
(K-395); the flare's predates the uniform row and it has no `matte_invert`, so
the row draws the picker alone. Adding one is a parameter, a version bump and a
line in the detect kernel — small, but it changes what a saved flare renders if
done carelessly, so it wants its own K-258 test.


**Fast motion blur only works on footage layers.** docs/08 §3.2 says the effect
is "applied per layer or, **most commonly, on an adjustment layer over the whole
montage**", and that case is a silent passthrough — as is a Precomp layer. Only
Footage layers are given a `flow_field` at all, because the decode worker is the
only thing that measures flow and it only ever sees decoded source frames. An
adjustment layer's "source" is the composite of everything beneath it, which
exists as a GPU texture and never as decoded frames.

The shape it needs: build the below-stack at the neighbour time the way
`temporal_below` already does for Posterize and `accumulation_below` for §3.26
(docs/impl/temporal-rerender.md), render both to textures in `realise`, and
measure between them. That last part wants a texture entry point on the flow
engine — `GpuFlow` takes a CPU `Gray` today, so it needs one small kernel
converting an RGBA texture to the luma buffer the pyramid starts from, after
which the whole measurement stays on the card. Doing it by reading the two
composites back to the CPU would work and would cost more than the flow does.

**Flow's remaining K-331 work.** The engine, the GPU port, the cache and the
controls have landed. What is left:
1. **`PreviewEngine::default` still builds its pool without a GPU**, so that
    path measures flow on a headless device of its own; the headless renderer
    the Flutter frontend drives shares the render device correctly. Pass a
    context in, or delete the path if nothing drives it.
2. **The remaining CPU work in synthesis is the luma conversion and the frame
    uploads** — about 70 ms of the 79 ms a 1080p interpolation costs, against
    8 ms for the flow itself. Both would go if the decoded frame reached the
    card once and stayed there, which is the `DrawSource` change K-331 sketched.
3. **The learned ceiling** — RIFE-class synthesis, WAFT-class flow — now has a
    judge to be measured against: `flow_quality.rs` and `clip_cadence.rs` landed
    with K-332's follow-up and the K-390 programme ran through them
    (docs/impl/optical-flow.md §4.5–§4.7, §5.5). A learned synthesiser emits no
    flow field, so Fast motion blur and Datamosh need DIS vectors regardless.
4. **A second matching cost is measured out, not open.** Census scoring (K-390
    item 1) cost game capture 0.0073 against a 0.005 allowance; choosing census
    or SSD per patch from the Hessian trace (K-393) recovered most of it
    (gameplay −0.0043, anime +0.0012, cartoon +0.0045, synthetic +0.0026) and
    missed on the **cinematic** instead, by 0.0002. No setting on the sweep
    cleared all four conditions and the frontier's shape says a hard switch
    cannot, so **both were reverted and the inverse search ships as SSD**
    (docs/impl/optical-flow.md §5.5.1, §5.5.2 hold the tables). Only one avenue
    is left open, and only if somebody funds it: blend the two costs across a
    band, or give a patch hysteresis so its mode agrees with its neighbours'.
    Either needs the two costs on a common scale — a real design question — and
    a second measurement, and either would need §5.5.2's 16 px checkerboard
    parity scene brought back, since it left with the revert.
5. **Line art is still behind a crossfade on the worst blocks**, by −0.0095 of
    worst-5% block SSIM on anime and more on cartoon. Both attempts at it are
    now measured rather than argued: census matching (K-390 item 1) closed part
    of the gap at a cost elsewhere that the bar refused, and edge-aware
    densification (K-391) lost on four of five clips because a field-space
    solve cannot add evidence. A third attempt must be **evidence-bearing** —
    something that measures line art better, not something that smooths a
    finished field again.

**Not to be built: a `flow/` disk tier.** docs/06 §5.4 reserves the folder and it
should stay empty. Measuring a 1080p pair on the GPU costs ~8 ms; reading 37 MB
of stored field off an SSD costs more. It would be a cache slower than the thing
it caches. The RAM tier (`DEFAULT_FLOW_CACHE_BYTES`) is the one that pays.

**Localisation follow-ups (K-303).** The seam is built and the strings are out of the
code (`flutter_ui/lib/l10n/`, `crowdin.yml`); what is left is other people's turn and
three small gaps:

- **Confirm the Crowdin language settings took, on the next pull (K-311).** The first
  pull landed five languages and reddened main twice, both from Crowdin settings, both
  since corrected there (the `zh`/`zh_Hant` mapping, en-US off) but not yet synced.
  After the next `crowdin pull translations`, `test/l10n/arb_test.dart` passing is the
  proof; if `@@locale` comes back hyphenated anyway, the fix moves into CI as a
  rewrite step on the sync branch (K-303 has the history).
- **The two numbered shortcut labels stay English.** `lumit-keymap` builds "Add marker
  {n} at the playhead" and "Go to marker {n}" with `format!`, so they are not literals
  the lookup table can hold (`lib/l10n/engine_labels.dart`). Give the bridge the number
  separately, or the label a stable id, and they join the rest.
- **No CI check that the source file was pushed.** A string added here is invisible to
  translators until somebody runs `crowdin push sources` by hand. Worth a release-time
  step once the project exists.

**Lens flare follow-ups (K-256..K-264, [impl/lens-flare.md](impl/lens-flare.md))** — the
shipped core is docs/08 §3.27; its performance items sit in **Now** above. Still owed:
the **Lights source wiring**; an **image aperture** file parameter; the **lens
designer** (`lens_file` landed in K-264, so its output has a place to go); an
**Occlusion layer** reference; **adaptive grid refinement at vignette folds**, the real
cure for both K-264/K-265 known limits (K-265 lists the six ablations already ruled
out — do not re-chase them with guards). Panel side: the pair row's dropper on
**Transform's px@comp pairs** (the pick exists since K-260); **Radial blur's centre
migration** to px@comp (K-260); one-op writes for a paired keyframe toggle.

**The stale-fd race on a Linux Viewer resize** (`lumit-render/src/headless.rs`'s
`shared_dmabuf` re-create, with `lumit-gpu/src/shared_linux.rs`'s `Drop`). The
exported descriptor is closed when `SharedDmabuf` drops, but the descriptor
*number* travels to Dart asynchronously, so two quick resizes can have Dart
register a closed fd - or one the OS has since reissued. Either hold the previous
`SharedDmabuf` for one generation, or `dup()` at export so the number in flight
owns itself.

**Ramp preset shelf rework** - the Linear/Slow/Fast/Smooth/Sharp buttons need a
general rethink (owner, 2026-08-02) before they return on the property path; not
a Vegas-mode concern ([04-RETIMING.md](04-RETIMING.md) §12.2).

**Retime UI wiring** (UI/command affordances - [04-RETIMING.md](04-RETIMING.md);
post-K-249 these return on the **property** path — the segment calls named here
are the reference for behaviour, not wiring targets):
- Freeze-at-playhead (`insert_freeze` built, no caller); Hold preset button;
    RATE/MAP type chips; kink badge; graph overrun band + source-out reference
    line; compensating Alt-drag; copy/paste a retime between clips;
    outward-trim-extends-map; the retime keyboard shortcuts (§12); Blend
    interpolation toggle; Flow-params UI and the source-rate advisory badge.
- Precomp retiming - Precomp layers carry no Retime today; decide the intended
    scope before building.
- The Time-lens **vertical (source-position) boundary drag** has no bridge op -
    `Retime::from_source_keyframes` (`lumit-core/src/retime.rs`) is unexposed, and
    the `SetLayerRetime` op this entry used to name alongside it no longer exists
    at all, K-249 having moved Retime onto the property path.

**Bridge reads left outside the read model** - the Source card's text/camera
fields for the selected layer, the Viewer's missing-file probe, and the
marker/work-area reads on a Timeline rebuild. Fold any into
`BridgeLayerInfo`/`BridgeCompModel` if they show up in the budget ranking.

**Thin-view debts the 2026-08-10 audit left for engine API** - each is Dart
doing the engine's job and each wants one bridge call:
- `viewer_camera.dart` re-derives the renderer's Ry·Rx·Rz basis and picks the
    active camera itself; wants `comp.activeCameraPose(frame)`.
- `viewer_type.dart` mirrors the engine's text-width estimate (caret, anchor,
    gizmo all share it); wants a `layer.textMetrics` read.
- `viewer_gizmo.dart`'s `_pathBeingEdited` parses `<layer>/masks/<mask>/path`
    strings in a widget; wants the selection model to expose the pair.
- The shape tool's Ctrl+Z pops draft points locally (a second undo meaning);
    wants engine-side draft ops so undo stays the document's.
- `fx_console_context.dart`'s `_keyTransformGroup` builds and sorts keyframe
    lists in Dart, two bridge calls per comparison; wants a held-keyframe write
    op on the layer.
- `FlowRowsFrb.build` (Effect controls) still reads four flow getters in
    build; same class of defect the audit cleared from the Timeline's rows.
- `theme_tokens.dart`'s `_with` restatement wants `LumitTheme.copyWith` in
    `theme.dart`, whose four-field shape is documented as deliberate - an
    owner call, not a mechanical fold.
- `headless.rs`'s four per-platform present-target-pool bodies share one dance;
    fold them on a machine that compiles the macOS/Linux paths.
- `ExpressionContext::comp_time` is raw `f64` across an engine boundary
    (docs/14 typed time); rhai's seam is f64 regardless, so the typed carry is
    a three-file ripple best taken while `fx/resolved.rs` is quiet.

**`LumitAppNew` rebuilds the whole app on any `LumitUiState.notifyListeners`** (a
`ListenableBuilder` above everything), and un-scoped document changes do the same
via `LumitState`. Reads are nearly free; the widget-tree rebuild is not. Scoping
the visible tree remains.

**The Windows shared-texture test races, rarely.**
`lumit-gpu`'s `shared::tests::the_legacy_handle_yields_the_pixels_angle_style`
failed one CI run with `[0, 0, 0, 0]`. `present` ends with a `CopyResource` and a
`Flush`, which submits without waiting, and the test's reader opens the shared
texture on a third device with no keyed mutex to wait on. Fix with a
`D3D11_QUERY_EVENT` on the reader (test-side only) or by landing the keyed-mutex
handshake. Wants a Windows machine to write it on.

**Playback scheduler - what remains**
([impl/playback-scheduler.md](impl/playback-scheduler.md)): in-render epoch tokens
(composites are serial on one worker thread, so cancellation latency is one
frame's render rather than §1's 15 ms), and §6's real-window benches (A/V drift
over 10 minutes, the underrun ladder). Re-run
`integration_test/playback_bench_test.dart` to price the stack; it needs a
1080p60 fixture and a Windows device, so it is run by hand.

**Settings pages not built ([07-UI-SPEC.md](07-UI-SPEC.md) §15):**
colour-management; preview-mode (Every frame/Adaptive) toggle; CUDA on/off;
plugins/decoder page; autosave interval/keep; export defaults (preset + filename
template). Each lands wired to the engine through the bridge, not as a Dart-side
setting nothing reads.

**Engineering-rules tooling still owed** ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)):
fuzz targets for the `.lum` deserialiser and journal replayer (§6); the **edition-2024
move** (§9 - the toolchain pin landed in K-272, the edition did not); the
`indexing_slicing` / `arithmetic_side_effects` clippy denies after a hot-path sweep (§4);
`clippy::pedantic` with curated allows (§7); the golden-frame EXR export corpus (§6).

**Four unmaintained dependencies are deliberately ignored in `deny.toml`** (K-272).
`ttf-parser` (via fontdue, via `lumit-text`) is the one with a real successor: moving
the rasteriser to `skrifa` is its own piece of work with its own glyph-metric tests.
`bincode` 1.x, `paste` and `smartstring` (via rhai, retired 2026-08-11 in favour of
compact_str/smol_str) leave when the dependencies that pull them update.

**The FFmpeg 8 migration is finished and parked as draft PR #102.** rsmpeg's
`ffmpeg8` feature, zero source changes, decode proven byte-identical over real
frames on 7.1 and 8.1 (software and D3D11VA). Blocked on macOS only: Homebrew
has no `ffmpeg@8` formula and plain `ffmpeg` is already 9.x, which no published
rsmpeg supports. Unblock: homebrew-core ships `ffmpeg@8`, or someone with a Mac
proves `brew extract --version=8.x ffmpeg <tap>`; then rebase, flip the
version gate, one green run, merge. Until then main stays on the immutable
n7.1.1 dated pin.

**A genuinely FFmpeg-free build is not possible yet (K-273).** `lumit_bridge
--no-default-features` compiles the bridge's own decode paths out, but `lumit-render` and
`lumit-audio` depend on `lumit-media` unconditionally, so the library is still linked and
the build still needs it installed. Making those two deps optional — and the render/audio
paths that use them — is what "builds without FFmpeg" would actually take.

**The three-tier cache's remaining sharp edges.** K-277 bounded the disk tier's write
queue after it reached 81 GB on an idle Mac; the same shape of question is worth asking of
the *other* unbounded `mpsc` channels the worker owns (the loaded-frame return, the
prefetcher's results) — none carries whole frames as freely as the park queue did, but none
counts its depth either. Also owed from that hunt: nothing reports how deep the park queue
is running, so a machine whose disk cannot keep up degrades silently (frames simply stop
reaching disk).

**What the performance harness still cannot measure** (K-389 built it: `crates/lumit-bench`
drives the reference comp headless through B3, B4, B5, B6, B7 and B11, and the job
`performance gates (ratio vs baseline)` gates the ratio to a checked-in baseline). Five
budgets are outside its reach and remain manual release checks, each needing its own
instrumentation:

- **B1 and B2 — UI frame time and input acknowledgement.** They belong to the Flutter
    thread, which no engine-side harness has. Wants frame timing recorded in the app
    (`SchedulerBinding`'s frame callbacks) and a way to drive an interaction from a test,
    so "8 ms during a drag" becomes a number rather than a feeling.
- **B8 — export throughput.** The encoder is not in the harness. A timed export of the
    same reference comp at the YouTube 1080p60 preset is the measurement; it needs hardware
    encode present to mean anything, which no runner has.
- **B9 — device loss to preview resumed.** Needs a real device to lose.
- **B10 — A/V drift during playback.** Needs the audio device and the clock the player
    actually runs on.

Also owed: **a floor-class runner** (§7.3's Iris Xe-class machine, the standing open
question), and **the reference-hardware pin** — the absolute budgets are asserted only under
`LUMIT_REFERENCE_HW=1`, so until a self-hosted runner sets it, nothing in CI checks a
budget's actual number. A **stress comp** (4K, 20 layers) and the per-effect cost-class
benchmarks of §7.3 are unbuilt too. The per-node profiler (§7.1) now has its first visible
piece - the render-time column (K-276) - and the rest of it (continuous timestamp-query
collection, the recording mode, the panel) is in the entry above.

**CI coverage the Flutter port left thin:**
- **macOS and Windows CI do not require an adapter.** `LUMIT_REQUIRE_GPU` turns
    a "no adapter" skip into a failure and the Linux job sets it (K-269); the
    other two do not, because nobody has confirmed those runners enumerate one.
    One run with the variable set says whether they can.
- **Nothing in CI proves a Viewer frame arrives.** The Linux job is the only one
    running the Flutter suite and has no GPU, so the six Viewer tests that wait
    for a frame skip there on `LUMIT_NO_ZERO_COPY_VIEWER=1`. They still fail on a
    regression on any machine with a real adapter, so the owner's box is the gate.
    A Linux runner with a GPU, or a Windows job running `flutter test`, closes
    this and verifies the DMA-BUF path at the same time.
- **The Flutter suite runs at `--concurrency=1`** - the mitigation for the
    order-dependent tests above, not the fix.
- **Registering a texture cannot happen in a widget test**, so
    `integration_test/shared_texture_test.dart`, run by hand on a real window, is
    the only coverage of that path.

**Threading / platform:**
- **Shared-texture producer/consumer fence** - only if a live run shows tearing;
    verify on the machine first.
- **Linux packaging** - the Flutter Linux build needs its own packaging when a
    Linux release matters.
- **Export options still to build** ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)
    §7) - one-click vertical variants (centre-crop reframe), user presets
    serialised beside the built-ins, export priority and encoder preference order,
    the 48 kHz-only audio rate becoming a choice, and free width/height boxes
    (sizes are preset-driven today).
- **Export status still speaks the old idiom** - `export.rs` replies in JSON
    strings (`err_json`) polled on a timer; follow the worker's typed-stream way.

- **The menu bar names its own backlog (K-244).** Every row marked
    "(Not implemented)" in File/Edit/Composition/Layer/Animation/View/Help is a
    command with a place waiting for it: Close project, History,
    layer settings and the mask/transform/blending/matte/style families, the
    whole Animation menu, the View menu's grid/ruler/wireframe/snap rows,
    Trim and Crop comp to work area, and Add to export queue (Check for
    updates is built — K-296; so are the View menu's magnification and
    resolution rows and the Help menu's two documentation links). Delete each
    mark as the command lands. Suggested chords for the AE-shaped ones are in
    K-244.

- **A Flatpak remote, so `flatpak update` has something to update from (K-297).**
    Releases ship a single-file `.flatpak` bundle, which installs perfectly well
    and then never updates: `flatpak update` needs a remote. Export an OSTree
    repo in `release.yml`, publish it (Cloudflare Pages beside the site, K-279)
    and ship a `.flatpakref`, or submit to Flathub and let it host. Until then
    Lumit tells Flatpak users the install command rather than offering a button.

## Later

**AE import, phase 1 (K-410, docs/impl/ae-import.md §6) - the walker and the reader
landed 2026-08-21; three things are still open.** `tools/ae-bridge/` holds the
ExtendScript walker and the fixture builder, `crates/lumit-import/` holds the bundle
open and the capture types, with fourteen tests on a hand-written synthetic bundle.
 - **The golden bundle is one AE sitting away.** `make-fixture.jsx` has never been run:
   it needs a real After Effects, which CI does not have and this machine's tests cannot
   substitute for. Until the owner runs it once, `crates/lumit-import/tests/fixtures/
   synthetic.lum-bundle/` stands in - written by hand against the walker's output, which
   makes it an honest description of the schema and *not* evidence the walker produces
   it. The first run is also the first proof that the match names in
   `make-fixture.jsx` are the ones AE ships; the builder's step harness is there so a
   drifted name costs one checklist row rather than the whole sitting.
 - **The mapping does not exist yet** (phase 2): capture → `lumit_core::Document`, the
   effect table built from `tools/ae-audit/ae-audit-report.json`, placeholders, and the
   report struct. `lumit-import` deliberately does not depend on `lumit-core` yet,
   because nothing in the reader needs it.
 - **The surface does not exist yet** (phase 3): `import_ae_bundle(path) → report`
   across the bridge, the File menu entry, the report panel (docs/11 §9), and footage
   relink through `resolve_all_media`. No user-facing string has been written, so
   `app_en.arb` is untouched by phase 1 and will need its first keys at phase 3.

**AE effect parity, wave 1 (docs/impl/ae-effect-parity.md) - landed in full 2026-08-20.**
Eighteen Tier-A effects in four family batches: ~~colour (Curves, Levels, Brightness, Hue
and saturation)~~ **K-396/K-397**, ~~generate (Fill, Gradient, Noise, Fractal noise)~~
**K-398**, ~~distort (Turbulent displace, Tile, Offset, Mirror, Lens distort)~~ **K-399**,
~~utilities and transitions (Drop shadow, Set matte, Channel blur, Linear wipe, Radial
wipe)~~ **K-400**. docs/11's seed table is trued for all eighteen.

**AE effect parity, wave 2 (docs/impl/ae-effect-parity.md) - landed in full 2026-08-20.**
All of Tier B, by owner's ruling, with one standing exclusion (no particle-world port).
Six batches: ~~Distort I (Corner pin, Displacement map, Polar coordinates, Twirl,
Spherize)~~ **K-402**, ~~Distort II (Ripple, Wave warp, Bezier warp, Warp, Roughen
edges)~~ **K-403**, ~~Stylise I (Posterize, Threshold, Tritone, Photo filter, Black and
white, Shadow highlight)~~ **K-404**, ~~Stylise II (Median, Mosaic, Find edges, Emboss,
Texturize, Broadcast safe)~~ **K-405, landed 2026-08-20, catalogue at 75**,
~~Transitions (Venetian blinds, Iris wipe, Card wipe)~~ **K-406, landed 2026-08-20,
catalogue at 78**, ~~Draw and grain (Beam, Lightning, Radio waves, Vegas, Add grain)~~
**K-407, landed 2026-08-20, catalogue at 83**. Scribble, Stroke and Vegas' Mask/Path
half stopped on the mask seam and landed with it the next day - **K-408, landed
2026-08-21, catalogue at 85**. docs/11's seed table is trued for all thirty-two, with no
substitutes left in it.
 - **A mask-path row names one mask, and three AE controls want a set** (K-408, docs/08
   §3.78-§3.79). Scribble, Stroke and Vegas' Mask/Path source are built and the import's
   substitutes are retired; what is still reported against the seam is AE's **All Masks**
   and **Stroke Sequentially**, and Scribble's two multi-mask Fill Types. All three want a
   row naming a *set* of masks - a small extension of `ParamKind::MaskPath` and a list
   rather than a slot in the carriage. Nobody has asked for it.
 - **A path drawing is capped at 512 straight pieces** (K-409, docs/08 §3.78). The geometry
   rides in a uniform, exactly as Lightning's bolt does, and past the cap every consumer
   coarsens rather than drawing part of a shape: the hatch widens its spacing, the dots
   space out, the chain straightens. A storage buffer is the answer the day something wants
   tens of thousands of pieces; nothing does, so none was built.
 - **Lightning ships four of AE's eight types, and no Alpha Obstacle** (K-407, docs/08
   §3.74). Breaking, Bouncey, Anywhere and Vertical map to the nearest of the four and are
   reported; Alpha Obstacle asks the bolt to route around the layer's own alpha, which is a
   *search* rather than a formula and would change the effect's cost class. If it is ever
   wanted it wants a distance field of the alpha and a bolt built against it, both of which
   the host-side generator could do without touching the kernel.
 - **Beam has no 3D perspective** (K-407, docs/08 §3.73), for K-406's reason: AE's
   foreshortens the beam from a camera of its own, and Lumit keeps cameras on the
   composition (docs/06). The same composition-camera input that would give Card wipe its
   grid would give Beam this.
 - **Radio waves ships one Stroke width where AE tapers from a start to an end**, and only
   its Polygon wave type (K-407, docs/08 §3.75). A taper needs the *age* to reach the
   stroke's width, which it already does for the fade — so it is a cheap addition whenever
   somebody wants it. Image Contours is Vegas, and so is Mask now (K-408, its Mask/Path
   source) - both are reported as suggestions rather than built into Radio waves itself.
 - **Vegas' Segment length is a length, not a count** (K-407, docs/08 §3.76). AE traces the
   contour into a path and can therefore count segments *around* it. **On the Mask/Path
   source this is fixed** (K-408): there the dashes are spaced by measured distance round
   the mask, so they stay even however hard it curves, and the import converts AE's Segments
   exactly. It is only the contour half that still drifts in phase on a curve, because it
   still never traces one - the machinery that would let it is now sitting next door.
 - **Card wipe has no camera, no back layer, and no Card Scale** (K-406, docs/08 §3.72).
   Each card is projected in its own local frame at a fixed viewing distance, because
   Lumit keeps cameras on the composition (docs/06) and has none on an effect. If effects
   ever get a composition-camera input, the grid could be projected through it and AE's
   Camera Position / Corner Pins / Composite Camera would stop being reported. A back
   layer would need a second layer row, which §3.68's test says a card wipe can justify.
 - **Card wipe's Flip order has no Gradient entry** (K-406, docs/08 §3.72). AE reads that
   order from a gradient *layer*; Lumit's one layer row is the universal Matte, and a card
   wipe wants to say "only over the sky" as well as "in this order". A Gradient order can
   arrive later on a row of its own without moving anything. Randomness plus Seed covers
   the intent meanwhile, and the import approximates from the gradient's spread.
 - **Median's Radius is capped at 3 and cannot be typed past** (K-405, docs/08 §3.64), the
   only control in the catalogue for which that is true. The cost is the fourth power of
   the radius, so a larger window needs a different algorithm - a per-tile histogram, or a
   separable approximation that is no longer a median - and either is its own programme
   with its own oracle. The import writes 3 and reports the instance as approximated.
 - **Texturize's Placement cannot honour AE's *native-size* Tile and Centre** (K-405,
   docs/08 §3.68). The layer carriage renders a referenced layer at this raster, so the
   texture arrives frame-shaped and Scale is what says how big one copy is. If a layer
   input ever carries its source's own dimensions alongside the texture, the three
   Placements could use them and the import would stop approximating the size.
 - **The Stylise II proof renders on the CPU, and the fixtures are gradients.** Median,
   Find edges and Emboss are the first effects whose picture cannot be judged on the smooth
   clips in `C:/tmp/lumit-shots` at all, and the batch was judged on a screenshot instead.
   A fixture with real high-frequency detail in it - a resolution chart, a page of type -
   would serve every future edge-detecting or despeckling effect.
 - **Shadow highlight has no Auto amounts, and probably never should** (K-404, docs/08
   §3.63). AE's is a whole-frame histogram reduction smoothed across neighbouring frames,
   which makes a grade whose answer at a frame depends on the shot around it. If it is
   ever wanted, it is a *scene analysis* feature with its own cache and its own doc, not a
   checkbox on this effect — and the import already reports it.
 - **Shadow highlight ships one Radius where AE ships two.** The second full-frame
   gaussian is real work for the softness of a mask; if a shot ever needs the shadows'
   mask measured at one scale and the highlights' at another, the kernel takes a second
   bound texture and the uniform grows one float.
 - **The old distort kernels still guard a texture fetch instead of clamping it**
   (K-402): Mirror, Tile, Lens distort, Drop shadow, Transform, Shake and the blur
   family all carry the early-return form of `tap`, which the compiler may hoist above
   its own bounds check. A pixel whose four bilinear taps are *all* outside the frame can
   come back opaque instead of empty. Wave 2's five kernels use the clamp-and-`select`
   form; the rest want the same one-line change, and an oracle case that drives every tap
   outside at once so the fix is held.
 - **Bezier warp's twelve points want on-picture handles.** v1 ships them as
   twenty-four ordinary rows, four corners open and the eight tangents behind their
   edges' headings (docs/08 §3.55). Dragging a Bezier patch in the Viewer is the same
   overlay job Corner pin's four points want and should land with them; the stored form
   is AE's clockwise walk and survives the editor.
 - **Warp has no Warp Axis, and Wave warp no noise wave types.** Both are recorded
   skips (docs/08 §3.56, §3.54) that the import reports rather than approximates. The
   axis swap is six lines whenever someone misses it; the noise wave types are §3.37's
   field wearing a wave's clothes and probably never want building.
 - **Curves wants a drawn curve editor.** v1 ships the stored form (five knots a
   channel, K-396) as twenty ordinary rows. The editor is a panel change and not a
   data one: `customEffectRows` in `effect_controls_panel_frb.dart` is the hook, and a
   curve authored today survives it.
 - ~~**The noise core is built but only half used.**~~ **Done 2026-08-20.** Turbulent
   displace reads the same field. The WGSL half moved to `fx_noise_core.wgsl`, which is
   prepended to both kernels at pipeline build (WGSL has no `include`), so there is one
   twin of `lumit-core/src/fx/noise.rs` rather than one per effect.
 - **Fractal noise is missing five AE controls**, all of them one more scalar through
   the same loop: Sub rotation, Sub offset, Perspective offset, Centre subscale, and
   the Overflow modes beyond Clip (docs/08 §3.37). None changes what the effect is;
   they land when a real project asks.
 - roadmap features not yet built

Grouped by the phase they belong to in [16-ROADMAP.md](16-ROADMAP.md). A pointer
list, not a re-statement of the roadmap.

- **Media engine ([05-ARCHITECTURE.md](05-ARCHITECTURE.md) §6).** The one-copy
    D3D11→DX12 interop and VideoToolbox (K-033); proxy generation; image-sequence
    footage; the resource governor; ProRes/DNxHR intermediate export (v1 is
    H.264/HEVC only); the 8-/32-bpc working-depth switch (v1 is fp16 only); OCIO
    v2 colour management and its UI.
- **Audio - the largest gap** ([07-UI-SPEC.md](07-UI-SPEC.md) §10,
    [09-AUDIO.md](09-AUDIO.md)): the whole **Audio panel** and level meters; the
    beat-marker tuning controls (sensitivity, BPM-grid, range); persistent
    waveform peak files (the multi-zoom summary is built on demand and cached for
    the session, K-280 — never written to the project sidecar, so it is rebuilt
    next time the project opens).
- **File format ([10-FILE-FORMAT.md](10-FILE-FORMAT.md)).** Embedded `thumbs/`
    previews in the `.lum`; the per-project sidecar `proxies/`, `peaks/` and
    `flow/` directories (only `frames/` and the global media index exist).
- **Design ([15-DESIGN.md](15-DESIGN.md)).** Bundle JetBrains Mono, Schibsted
    Grotesk and Source Serif 4 (only Inter is wired); add the 13/14/20 px
    type-scale steps to the theme; identity colour tokens for Shape and Null
    layers (§6.1 reserves the values; both kinds borrow today).
- **Platform.** The macOS pass - native menu bar, VideoToolbox, ProRes
    (K-033); it also owes `application:openFile:` (a double-clicked
    `.lum` opening, K-252). The document `.icns` files now ship inside the
    bundle, so the icons themselves are done.
    The Metal/IOSurface Viewer path is unverified on real hardware.
    Developer ID signing and notarisation landed (K-310) but have never run —
    the first tag after that entry is their first execution, and a pre-release
    tag is the way to rehearse it. Signing the Windows installer is still
    blocked on buying a certificate, so the installer ships unsigned and
    SmartScreen still warns.
- **Phase 2 - Retime.** Flow interpolation policies; automatic beat snapping
    across edit/retime points ([04-RETIMING.md](04-RETIMING.md),
    [09-AUDIO.md](09-AUDIO.md)).
- **Phase 3 - The look.** Per-layer motion blur polish and the scopes GPU pass
    ([08-EFFECTS.md](08-EFFECTS.md)); importing a preset file from outside the
    presets folder is still a manual copy. A **tone mapping effect** belongs here
    too (owner, 2026-08-06): the grade that actually lands in the export, distinct
    from the Viewer's preview-only toggle in **Next** above, and it wants
    [08-EFFECTS.md](08-EFFECTS.md) §3 to gain its entry and a curve chosen in
    [02-DECISIONS.md](02-DECISIONS.md) - the same choice both then share.
    This gate is the v1.0 milestone.
- **Phase 4 - Extensibility** (whole docs, nothing built -
    [11-AE-IMPORT.md](11-AE-IMPORT.md), [12-PLUGINS.md](12-PLUGINS.md)). AE
    import (Bridge panel, `.aep` parser, Lottie, fidelity report); the OFX host;
    the LFX C ABI + validator; expressions (QuickJS-ng). Placeholder
    round-tripping already preserves unknown effects/expressions.
- **Phase 5 - AE parity march.** 2.5D cameras/lights/DOF, tracker/stabiliser,
    keying, rotoscoping, particles, tier-2 effects, text animators, shape
    operators, the Composer audio workspace ([09-AUDIO.md](09-AUDIO.md)).
- **Phase 6 - Beyond parity.** Node view over the evaluation graph, Blender scene
    import, Lottie export, OpenTimelineIO interchange, render-farm/CLI export
    (K-023, K-036).

## Deliberately deferred (not backlog)

Recorded so they are not re-proposed as gaps:

- **The render worker pool, measured and deliberately not built (2026-07-31).**
    [impl/playback-scheduler.md](impl/playback-scheduler.md) §2 reserves GPU
    submits to one thread, so the only work a pool could take is the processor
    half of a frame - naming it, planning the decode, building the draw list.
    That half measured **0.03 ms at 32 animated layers against 200 ms for the
    whole frame**, or 0.015%, and it is an absolute CPU cost that does not shrink
    on a faster card, so its share only falls on real hardware. Spreading it over
    threads saves nothing at any layer count. The same measurement found the
    command-buffer item under *Now*, which is where the win actually is. Anyone
    reaching for the pool again should re-run the stopwatch first: if the
    processor half has not grown, this entry still stands.
- **Rotation gizmo affordance** - the previous frontend never offered one; not a
    regression.
- **The workspace strip ticks nothing after a restart.** `Workspace.activePreset`
    is session-only on purpose: what persists is the arrangement, which the user
    is free to drag about, so a ticked preset could claim a layout the panels no
    longer match (`state/workspace.dart`).
- **Re-time the flare after K-373 and K-375.** The tent now reaches a full grid
    step, which is four times the fragments per splat, and the deposit moved from
    the raster blender to a compute scatter with a compare-and-swap float add,
    whose cost depends on contention (many splats on one pixel) rather than on
    fill rate. Both are correctness fixes worth their price, but the price is
    unmeasured on a real card: docs/13-PERFORMANCE-RULES.md budgets gate merges,
    and the per-frame figure in docs/impl/lens-flare.md predates both. The
    `lens_flare_frame_cost` measurement test is the place to read it.
- **The idle cache fill is not interruptible.** It composites one frame per turn,
    so a scrub arriving mid-frame waits for that composite to finish - up to a
    couple of seconds on a comp with a Lens flare. The 200 ms lull it waits for
    means a continuous drag never meets it; a pause-then-scrub does. Fixing it
    means cancelling work already handed to the GPU, which docs/14 asks for in
    general and the flare's render pass does not yet offer. Named in K-372 so it
    is not rediscovered as the cache-key bug that entry fixed.
- **No progress for the idle cache fill** - it is not a frame anyone is waiting
    for, so the bar stays quiet for it.
- The two recorded behavioural deviations (export queue-snapshot timing;
    share-export VBR cap) - see [02-DECISIONS.md](02-DECISIONS.md).
