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

- **The lens flare is not bit-stable on this machine, today.**
    `lumit-gpu`'s `fx::tests::wgsl_lens_flare_matches_the_cpu_frame_reference_and_neutrals`
    fails its own "GPU lens flare must be bit-stable" assertion on a clean `main`
    (checked 2026-08-08 by stashing every local change and running it alone): two
    runs of the same flare give different pixels. Bit-stability is the property
    the whole additive-blend draw order exists to protect
    ([impl/lens-flare.md](impl/lens-flare.md) §2.4), so this is a real
    regression and not a flaky test - and it means the two flare performance
    items below cannot be measured honestly until it is understood. Find which
    stage varies before changing anything.
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

## Now - Flutter frontend parity and regressions

Flutter is the only frontend (K-174, K-182); git history is the parity reference.
These are v1-scope surfaces it does not yet match.

**Viewer bar ([07-UI-SPEC.md](07-UI-SPEC.md) §2.2):**
- The wireframe/overlay *menu*; guides menu; region-of-interest;
    background-colour swatch.
- **Preview resolution is on the bar, but only Full / Half / Quarter** (§2.2
    item 2). The bar dropdown, the View menu, the three chords and the command
    palette all set the `scale` every render request carries, and the dropdown
    is disabled while adaptive playback chooses the tier itself — but **Third**
    and **Auto** have no rows, and the choice is shell-wide rather than stored
    per composition in the project as §2.2 asks.
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
1. **Turning the flow switch off discards the Flow group.** `FlowParams` lives
    inside the `Flow` variant of `Interpolation`, so there is nowhere to keep it
    while the policy is Nearest. Comparing a flow shot against the plain one is
    ordinary and should not cost the tuning. Move `FlowParams` onto the layer
    beside the policy (pre-release, no migration); `flow_rows_frb_test.dart`
    pins the current behaviour and inverts when this lands.
2. **`PreviewEngine::default` still builds its pool without a GPU**, so that
    path measures flow on a headless device of its own; the headless renderer
    the Flutter frontend drives shares the render device correctly. Pass a
    context in, or delete the path if nothing drives it.
3. **The remaining CPU work in synthesis is the luma conversion and the frame
    uploads** — about 70 ms of the 79 ms a 1080p interpolation costs, against
    8 ms for the flow itself. Both would go if the decoded frame reached the
    card once and stayed there, which is the `DrawSource` change K-331 sketched.
4. **A measurement harness on real gameplay** (K-332 follow-up), so the learned
    ceiling — RIFE-class synthesis, WAFT-class flow — is judged against numbers
    rather than impressions. A learned synthesiser emits no flow field, so Fast
    motion blur and Datamosh need DIS vectors regardless.

**Not to be built: a `flow/` disk tier.** docs/06 §5.4 reserves the folder and it
should stay empty. Measuring a 1080p pair on the GPU costs ~8 ms; reading 37 MB
of stored field off an SSD costs more. It would be a cache slower than the thing
it caches. The RAM tier (`DEFAULT_FLOW_CACHE_BYTES`) is the one that pays.

**The LUT effect's GPU path ignores a non-default domain**
([impl/lut.md](impl/lut.md) §3 status): `fx_lut.wgsl` skips the
`DOMAIN_MIN`/`DOMAIN_MAX` remap the CPU oracle applies, so such a cube renders
silently wrong. Pass the six domain floats through `LutParams`, or refuse
non-default-domain cubes as a labelled no-op. The LUT caches also key by path
alone - no mtime, no LRU bound (§4).

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
colour-management; preview-mode (Cached/Realtime) toggle; CUDA on/off;
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

**The performance harness and its CI gates are not built**
([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) §7.3): no reference comp in the
repository, no headless benchmark scenarios, no budget gates per merge. The per-node
profiler (§7.1) now has its first visible piece - the render-time column (K-276) - and the
rest of it (continuous timestamp-query collection, the recording mode, the panel) is in the
entry above.

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

## Later - roadmap features not yet built

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
- **No progress for the idle cache fill** - it is not a frame anyone is waiting
    for, so the bar stays quiet for it.
- The two recorded behavioural deviations (export queue-snapshot timing;
    share-export VBR cap) - see [02-DECISIONS.md](02-DECISIONS.md).
