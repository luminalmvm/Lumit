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

- **Take the lens flare's bake off the render thread.** Choosing a lens blocks
    the picture for about half a second of pure CPU optics (measured, K-263) -
    the single longest stall the effect has - and the bake is still a closure the
    render thread runs inside the frame (`lumit-render/src/fxops.rs`, the
    `Resolved::LensFlare` arm). Run it beside the render and a freeze becomes a
    wait you can see.
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
    colour-management indicator; background-colour swatch.

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
- **Mask paths cannot be keyframed** ([03-DATA-MODEL.md](03-DATA-MODEL.md) has
    them as animatable); there is no mask **mode** (add/subtract/intersect) -
    every mask adds; **mask feather** has neither a control nor a renderer path.
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

**Bound keys with nothing behind them.** The **Panels** context's three bindings
(`panel.focus.next`, `panel.focus.prev`, `panel.search.focus`) have no commands.
Either build them or drop the bindings.

**Appearance.** The seven built-in schemes still restate every colour
individually; only the two Timeline tokens default from the mode. Owed after
K-298: a swatch strip per row **inside** the picker's menu (it previews the
selection only), and a place to keep themes other than the workspace file, so an
imported theme travels with the user rather than the machine's settings.

**Shell and onboarding:**
- **The boot splash is not mounted.** `flutter_ui/lib/shell/splash.dart` exists
    and only its test imports it. Engine-side events cannot post a notice either:
    there is no notice stream, only `boot_log`.
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
- **The Flow column is reserved, not wired** - per-layer optical flow has no
    engine backing. Build the engine model first, then the fold-out's Flow group.
- **The Timeline's two halves are built twice and kept in step by hand.**
    `_Outline` and `_LayerArea` are separate widget trees walking the same layer
    list, aligned only because both read the same numbers, with vertical scroll
    mirrored behind a reentrancy flag. Building a layer **once** as a row holding
    both halves inside one vertical scrollable (the lane side keeping its own
    horizontal controller) gives alignment by construction. It deletes
    `blockHeights`, both controllers' sync and the guard flag rather than adding
    anything. A session's refactor, no behaviour change, alignment tests as the
    net.
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
    mitigation, not the fix, and it costs wall-clock.
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

**Localisation follow-ups (K-303).** The seam is built and the strings are out of the
code (`flutter_ui/lib/l10n/`, `crowdin.yml`); what is left is other people's turn and
three small gaps:

- **Create the Crowdin project and point it at this repo.** File-based, source
  `app_en.arb`, targets German, Kazakh, Ukrainian and Simplified Chinese. Then set
  `CROWDIN_PROJECT_ID` and `CROWDIN_PERSONAL_TOKEN` and run `crowdin push sources`. The
  four `app_*.arb` files here are empty placeholders until the first `pull`.
- **The two numbered shortcut labels stay English.** `lumit-keymap` builds "Add marker
  {n} at the playhead" and "Go to marker {n}" with `format!`, so they are not literals
  the lookup table can hold (`lib/l10n/engine_labels.dart`). Give the bridge the number
  separately, or the label a stable id, and they join the rest.
- **No CI check that the source file was pushed.** A string added here is invisible to
  translators until somebody runs `crowdin push sources` by hand. Worth a release-time
  step once the project exists.

**Lens flare follow-ups (K-256..K-264, [impl/lens-flare.md](impl/lens-flare.md))** — the
shipped core is docs/08 §3.27; its performance items sit in **Now** above. Still owed,
each stable against the shipped parameters: the
**Lights source wiring** (the mode is in the
dropdown and resolves as Manual until light layers can act as flare sources); aperture
**dirt / scratches** overlays and an **image aperture** file parameter; the **lens
designer** (a window building a prescription element by element with a live lens
diagram — the `lens_file` parameter landed in K-264, so the designer's output has a
place to go); an **Occlusion layer** reference fading the flare when the light is
covered; **adaptive grid refinement at vignette folds** — the K-264/K-265 known limits: a
mild ripple on hard vignetted edges of extreme-defocus ghosts at Normal, and the
toothed fold corona on a zoom shot past its native stop (K-265 lists the six
ablations already ruled out — do not re-chase it with guards); refinement at the
folds is the real cure for both. The panel side owes the pair row's dropper to
**Transform's px@comp pairs** (the pixel-writing pick exists since K-260 — the flare's
Light uses it; Transform's rows just aren't wired to it), **Radial blur's centre
migration** from the grandfathered % of frame to px@comp (K-260 convention), and one-op
writes for a paired keyframe toggle (two ops today).

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

**Three unmaintained dependencies are deliberately ignored in `deny.toml`** (K-272).
`ttf-parser` (via fontdue, via `lumit-text`) is the one with a real successor: moving
the rasteriser to `skrifa` is its own piece of work with its own glyph-metric tests.
`bincode` 1.x and `paste` leave when the dependencies that pull them update.

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
- **Move footage probing off-thread** - synchronous today; needs a probe worker
    drained on `lumit_bridge_snapshot` plus a synchronous `ensure_probed` fallback
    for `convert_to_sequenced`, `trim_to_source_end`, `add_footage_layer` and
    relink. **Beat detection is the same shape** - it runs on the calling thread
    ([17-BRIDGE-CONTRACT.md](17-BRIDGE-CONTRACT.md) §Threading) and wants the same
    worker treatment.
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

- **Viewer-only exposure and auto tone mapping (asked for by the owner,
    2026-08-06).** Two controls in the Viewer bar
    ([07-UI-SPEC.md](07-UI-SPEC.md) §2.2, which gains their entries when they
    land), both **preview only - neither may change the export**, the same
    promise preview resolution and the region of interest already make.
    **(1) Exposure**: a small box that scrubs on drag and takes a typed number,
    with an aperture icon beside it, reading signed stops to one decimal -
    `+0.0`, `+1.4`, `-2.3`. The number must mean what the Exposure effect's does
    (K-106): the same `2^stops` gain in scene-linear, so the two agree.
    **(2) Auto tone mapping**: an icon that toggles it on and off, nothing more -
    no curve picker in the bar. It is the "what will this actually look like"
    switch for a comp whose values run past 1, keeping the low end readable
    instead of watching the highlights clip flat.
    Both belong **inside the display transform**, which
    [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.3 already reserves for
    exactly this ("the exposure control and channel isolation are viewer-only and
    sit inside this stage") - the display blit in `crates/lumit-gpu/src/lib.rs`
    (`display`, `display_bgra`, `display_scaled`), not the effect stack. Check
    before building that the frame cache holds pre-display frames: if it does,
    changing either control is a re-blit and must not throw a cached frame away.
    Three things to settle. **The curve is decision-sized** - Reinhard, an
    ACES fit and AgX all look different, and picking one is a
    [02-DECISIONS.md](02-DECISIONS.md) entry, not a code comment. **"Auto"
    needs a definition**: if it adapts to each frame's content the picture
    breathes as the shot cuts, so say whether it is a fixed curve or a measured
    one, and if measured, how it is smoothed. **Persistence is an owner call** -
    per comp in the project like preview resolution, or view state that resets.
    Whatever they are, the Viewer must say when the picture is not the export:
    the colour-management badge (§2.2 item 8) is where that lives, in
    [15-DESIGN.md](15-DESIGN.md)'s calm voice - a statement, never a warning.
    A tone mapping *effect* is separate work and sits in **Later** below.

- **The menu bar names its own backlog (K-244).** Every row marked
    "(Not implemented)" in File/Edit/Composition/Layer/Animation/View/Help is a
    command with a place waiting for it: Close project, History,
    layer settings and the mask/transform/blending/matte/style families, the
    whole Animation menu, the View menu's zoom/resolution/grid/ruler rows,
    Trim and Crop comp to work area, Add to export queue and the help links
    (Check for updates is built — K-296). Delete each mark as the command
    lands. Suggested chords for the AE-shaped ones are in K-244.

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
    `.lum` opening, K-252) and adding `packaging/macos/*.icns` to the bundle's
    resources. The Metal/IOSurface Viewer path is unverified on real hardware.
    Developer ID signing and notarisation landed (K-309) but have never run —
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
