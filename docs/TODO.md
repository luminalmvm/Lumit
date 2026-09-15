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

## The finishing programme: COMPLETE (2026-08-25..26)

All seven waves landed and their per-wave summaries were pruned as they went. The history
was squashed on 2026-09-01, so git log is no longer the record of them and every commit
hash older than that is a dead pointer - which is why the entries below carry the fact
rather than the commit. What the programme's own runs surfaced and deliberately left open,
so it is not re-derived:

- **The physical Lens flare's headless blank is fixed; the example harness has not caught
    up.** The defect was the flare's own `blend`, injected under the Blend row's id and then
    read by an index into the *layer* modes: a fresh flare's Add (index 1) came back as
    Darken against the untouched input, so every flare on anything darker than itself
    rendered nothing. `a_default_lens_flare_lights_the_frame` in
    `crates/lumit-render/src/headless.rs` is the non-empty-output assertion this entry asked
    for. What is left is `effect_examples.rs`, which still carries the skip written around
    the defect ("adds nothing to a headless render") and whose own comment says to take the
    line out the day the additive pass reaches this renderer. Lift it and re-run the harness:
    the manual's flare page already draws a `<Compare>` over
    `web-docs/src/assets/effects/stylise/lens-flare.webp`, so what the run settles is whether
    that figure is a flare or the plate.
- **Shots sweep 4 crashes natively about one run in three** ("Lost connection to
    device", no Dart exception, different point each run). All its pictures were
    gathered across runs; the sweep is unreliable and the crash is an engine bug in
    hiding.
- **Six screenshots the manual still wants** (pages ship without them, no placeholders):
    camera-track.png, planar-track.png, project-settings-colour.png,
    viewer-colour-menu.png, text-animators.png, shape-combine.png.
- **Clone to points / Trail / Connect points and the Node graph effect have no example
    pictures** - the effect-example harness stages one effect on one layer, and all four
    want a graph behind them: a points stream arrives on a wire-only input
    (points-stream.md §4.1) that exists in a node graph and nowhere else. Now that a
    composition can *be* a graph, one graph in the example project would fix all four
    (`unillustrable` in `effect_examples.rs`).
- **The audio device-change stream** (a device vanishing mid-playback rebuilds on the
    next open, not live) - the remainder recorded when the Audio settings landed.
    `audio::devices()` still enumerates only when it is asked, and `set_device` still closes
    the stream and waits for the next prepare.
- Crowdin at the next push owes everything since the programme: its own batches (the
    pre-programme ~360 keys, 53 safe-lane keys, every FP key listed per commit, 63 changed
    tooltip values and tipBrushPressure), and then every key 0.3.x and 0.4.0 added on top -
    the node graph, the Audio panel and its fifteen effects, the extra Viewers, the
    governor's readouts, EXR and project colour depth, OCIO. English stands at 2,894 keys;
    German is 743 short, Kazakh 1,641, Ukrainian 1,436, and both Chinese 680 each, with
    Spanish and Polish still empty placeholders. `settingsHelpChromeLabels` is still unused
    in the frontend and still to cull.
- **The shader editor is highlighted; its palette is not ours.** Airyz took the honest road
    on 2026-09-08 - a WGSL TextMate grammar as a plain string through
    `Highlighter.addLanguage` (`flutter_ui/assets/data/grammar/wgsl.json`), no asset
    package, no `pub get` - and `ExpressionTextEditingController` now takes a language, so
    the shader well and the expression well share one controller. Two of the three reasons
    it was held back stand. The token colours are still VS Code's two default themes
    (`_defaultLightThemeFiles`/`_defaultDarkThemeFiles`), so six of Lumit's eight schemes get
    somebody else's palette, and the package still paints six hardcoded bracket colours and a
    `Color(0xFFff0000)` for an unmatched one with no API to turn them off - hex in the
    frontend is a defect. And `buildTextSpan` still discards the style it is handed, which
    drops the shader well's `height: 1.4` and can drift the line numbers out of step with
    the code.
- **A shader could be a project item** (Airyz, 2026-09-01): "i'd like if i could load a
    file in the project view and reference that instead, as it would give the same options
    for 'find missing footage' and stuff to swap the file to something else. It would also
    follow the same settings for relative/absolute file paths". Today Load from file copies
    the text into the instance (`extra["shader"]["source"]`) and the path is a
    memory rather than a link. Making it an item means a project-item kind that is not
    footage - `ProjectItem` is still Footage, Folder, Composition, Solid - the relink road
    reaching it, and a decision about what a missing shader renders, which is a
    decision-sized change, not an editor one.
- **The Project panel is not virtualised**, which docs/13-PERFORMANCE-RULES.md §5 names
    it in: `ListView(children: rows)` builds every row of the open tree whatever the
    viewport holds, with no `itemExtent` on a fixed-height list. The click and the probe
    storm behind the owner's "clicking a folder feels very slow" are fixed (the name
    cache and the coalesced rebuild, 2026-09-01); this is what is left, and it is the
    scroll half. `LazyBlocks` in timeline_metrics_frb.dart is the machinery to reuse.
    Note `_visibleIds` must keep the whole filtered tree - Ctrl+A reads it.
- **A click still rebuilds the whole Project panel.** `_select` is `setState` on the
    panel, where the Timeline publishes its selection on a notifier the rows read instead.
    Two rows change; every row rebuilds. Cheap now that the names are cached, and the next
    thing to do if the panel still feels heavy on a large project.
- **An undecodable file imports silently.** The dialogue no longer hides formats
    (the filter lists what the engine reads, with All files beside it), so what is left
    is the answer: a file FFmpeg cannot open becomes a footage item with no picture and
    no reason. `FootageReference::thumbnail` still returns `None` for undecodable and
    missing alike, and only missing wears a badge. Testers reading that as "it would not
    import" is what raised it. Wants the probe's error carried to the row and said in one
    line.
- **Some languages went backwards when the Crowdin branch merged.** Its five `.arb`s
    lost to the site tool's own output, which is newer and much fuller, but it had keys
    theirs do not: 152 in Kazakh, 134 in Ukrainian, 42 in German, 37 in each Chinese.
    Those keys are simply untranslated again, so the translation page already lists them
    for whoever wants them - nothing to do here. The community's earlier words for them
    are no longer reachable: the commit that held them predates the 2026-09-01 squash.
- **The flare's Matte mode always dispatches every light slot.** The raster half of this
    entry is gone with the raster: the splat sum moved off the fp16 blender into the
    fixed-point compute deposit (`fx_lens_flare_deposit.wgsl`), which skips a dead splat
    outright (`s.live < 0.5`) rather than drawing a degenerate quad, so there is no vertex
    work left to compact and the prefix-sum question is moot. What stands is the light
    list: Matte mode runs `MAX_SOURCES` - sixteen, not eight - candidate slots whatever the
    detection actually found, where an indirect dispatch would run the ones it filled.
    Measure the live fraction first. (docs/impl/lens-flare.md §4's pass structure still
    describes the retired quad raster and wants rewriting with it.)
- **Replace `poll(Maintain::Wait)` with a keyed mutex** - every present waits for
    the card to go idle before handing the texture over (`shared.rs`,
    `shared_linux.rs`, `shared_metal.rs`; find it by the call, not a line number).
    Playback from the card has slack to hide the stall; playback from memory does
    not. **Its own branch and its own pull request** - surgery on the
    shared-texture chain, where a mistake shows as tearing, not as an error.
    Measure first: the 2026-07-30 fixes may have made it moot. Not a revival of
    the deleted read-back transport - the Viewer receives a GPU handle
    and nothing else.

---

## Now - the interface answers at 60/120 (docs/impl/ui-performance.md)

The owner's interface mandate is measured and has an answer. The note is binding: the
gesture table taken in the owner's own conditions (window maximised, live preview —
the small-window empty-preview test trap flatters by 4×), the architecture, and the
work packages. **WP-2 through WP-6 landed 2026-08-30** (the select click, incremental
scroll, zero per-frame document calls during drags, the edit storm as one wave, and
the repaint matrix as CI gates in `rebuild_budget_test.dart`) - the note's §7 carries
each one's measured before-and-after. What is still open is the raster half, which is
not Lumit's:

- **WP-1 - LANDED, then reversed on shipping**: the unmet gap is the
    Windows embedder's own - the whole window re-records every frame, because the
    `FlutterCompositor` path cannot reach the engine's partial-repaint machinery, at
    ~8 ms/megapixel. **Not the MSAA resolve**: that hypothesis was built and A/B'd on
    2026-08-31 and the single-sample path measured ~5 ms *worse*, so §7's own item 1 still
    reads the old diagnosis and wants correcting against §7.1/§7.2. WP-7 in the note (§7.2)
    carries what is left: the drafted upstream issue (§7.1, still unfiled - the owner files
    it), narrowing the ~18 ms live-preview texture term to a repro outside Lumit *if* it
    reproduces at all (the same session could not), and watching upstream for a Vulkan
    compositor or damage plumbing. The local-engine partial-repaint prototype is evidence
    for the issue, not a pull request. The runner meanwhile pins Skia.
- **Per Flutter upgrade: re-run the §2.4 backend A/B** in the owner's conditions
    (docs/impl/ui-performance.md §2.1/§6, one run each backend) and flip
    `ImpellerSwitch::Disabled` back to `Default` in
    `flutter_ui/windows/runner/main.cpp` the day Impeller clears the 60 fps mandate.
    Standing, not one-off - delete only when the flip lands.
- **B1 and B2 have no standing gate in CI.** WP-6's matrix gates *rebuild counts*;
    the 8 ms UI-thread frame and the next-frame acknowledgement are still read off
    the probe by hand. The entry under *What the performance harness still cannot
    measure* below is where that lives.

## Now - the effect registry (docs/impl/effect-registry.md §6)

The migration is done. All 139 catalogue entries declare themselves one file each under
`lumit-core/src/fx/effects/` and `fx/drivers/`; `catalogue.rs` generates both halves of the
catalogue from one list; a frame resolves every effect through one generic loop into the
arena, and `run_ops` and `cpu::apply_stack` dispatch by name with no match over effects
left in either. `Resolved`, `ResolvedOps`, `resolve_one`, the free `rescale_px` and the
hand-written `BUILTINS` literal are all gone. What is left is §6 step 5.

- **Spare parameters and the panel affordances.** Dynamic parameters themselves are
    built: `EffectDef::derived(&inst)` is implemented by the Custom shader (from its
    uniforms), the Node graph effect (from the graph's exposed Inputs) and Extract
    channels, all three through one session-lived cache into a leaked
    `&'static [ParamSchema]`, and nothing downstream can tell a derived row from a
    declared one. What remains is the panel's half, where the rules (§4 of the note) put
    it: a derived row is **offered, never adopted** (docs/08 §3.95), and there is no
    gesture yet that adopts one or removes a row the graph or the shader no longer has -
    CS2's `sync_parameters` and `remove_unused_parameters` are unwritten. Then **spare
    parameters**, the user's own sliders for expressions to read, which need no shader at
    all and ride the same mechanism.

- **The effect manual is sixteen pages behind the catalogue.** The catalogue stands at 139
    and `web-docs/src/content/docs/effects/` holds 123 pages: the whole **Audio** family has
    none of its fifteen and no section on the index, and Extract channels has no page in
    Utility. `npm run docs:effects` from `web-docs/` creates and refreshes all of it inside
    the `GENERATED` markers - it will append the Audio heading and its table at the index's
    end, so that section's place in the order and its hand-written sentence are owed by
    hand, as are the sixteen pages' protected prose. Nothing in CI gates the manual, which
    is exactly why it is written down here.

---

## Now - Flutter frontend parity and regressions

Flutter is the only frontend; git history is the parity reference.
These are v1-scope surfaces it does not yet match.

**Timeline outline ([07-UI-SPEC.md](07-UI-SPEC.md) §4.2):**
- **A switch cell on a locked layer throws.** `lock_guards` refuses every switch
    but the lock, shy and the label (`lumit-core/src/ops.rs`), and the outline's
    cells now commit through one `set_switch_on_layers`, whose refusal for the
    *clicked* row is the whole call's — so clicking the eye, solo, fx, motion
    blur, 3D or guide on a locked row raises `LayerLocked` out of a tap handler
    instead of saying no. A locked *sibling* in a multi-selection already drops
    out of the batch silently, which is the manner the clicked row wants too.
    Pre-existing, and it wants one answer for all six cells rather than a guard
    per cell: either the cells stand down while the row is locked, or the
    refusal becomes a status-line notice.

**Viewer bars ([07-UI-SPEC.md](07-UI-SPEC.md) §2.2):**
- The wireframe/overlay menu's own *separation* (§2.2 item 5) — the view menu
    now carries the layer-controls switch, which turns wireframes, handles and hover
    highlight on and off as one; separating those from motion paths, mask paths and
    gizmo visibility, and the full wireframe display mode, is owed. The menu itself
    has filled out around it — grid, safe areas, rulers, the snapping magnet, the
    region of interest, the background colour and Clear guides — which is why it is
    a menu.
- **Degradation names a tier, not the steps it skipped** (§2.2 item 9). The bar's
    reading says the pixel count a frame was made at, which is the tier; §2.2 also asks
    that the indicator name what was degraded ("glow skipped"), and nothing reports that
    across the bridge yet.
- **Tone mapping has no explanation anywhere on screen.** The Viewer bar's toggle,
    the colour menu's row and the Settings switch are all bare names, because §13.2
    keeps a control's label to that — and the help sentences the Settings rows used
    to carry went with the drawing that had no room for them
    (`settingsRow`'s own note). The one-line "what this does" therefore lives only
    in the translator descriptions in `app_en.arb`, which nobody using Lumit reads.
    If hints of this kind ever get a home of their own, this is the first candidate
    to fill it.

**Toolbar tools ([07-UI-SPEC.md](07-UI-SPEC.md) §1.7):** what is armed is a
*tool*; what each tool then does is the backlog.
- **Shape layers** ([impl/shape-layers.md](impl/shape-layers.md)) - trim paths,
    dashed strokes, the repeater, offset paths, gradient fills, boolean combines
    and animated (morphing) paths have landed. Owed: nested groups, the
    **wiggle** modifier, gradient **stop lists** (a fill and one
    `gradient_colour`, which is two stops), and joins and caps other than round.
- **Path editing on the picture** - mask and shape-layer points drag. Still
    owed: a **paint stroke's** points, which are a stored gesture
    rather than a path and so are their own piece of work; no path's bezier
    **handles** can be dragged — the gizmo's point drag carries positions only,
    and the tangents are drawn and never aimed at — so the `Alt`-drag that
    re-links a broken tangent pair exists only while a point is being *placed*;
    and the model has no linked/broken flag (`mask::Vertex` is a position and two
    tangents and nothing else), so adding one is a
    [03-DATA-MODEL.md](03-DATA-MODEL.md) change and a decision, not just a
    gesture; and the Pen's add/delete/convert-vertex siblings and dragging a
    whole path by a segment.
- **Mask paths have no per-key op.** `SetLayerMasks` rewrites the whole list for
    every keyframe drag, so one drag is one undo step only because the drag is
    staged - a per-key op would make it so by construction.
- **The Mask Feather Tool** (the half under it is built) - a mask's feather
    can now be a width **per vertex**, keyed and dragged from its own Timeline
    rows, and switched on from the mask row's menu. What is still owed is After
    Effects' *tool*: `ToolMode.penMaskFeather` (`G` cycles onto it, under the
    Pen) remains a stub with an icon and a string and nothing behind it, so the
    widths cannot yet be dragged on the picture. Doing that properly wants
    feather points anchored by **arc length** rather than by vertex index, which
    would also close the feather's two recorded limits: deleting a point shifts the
    widths after it, and a path whose keys hold different point counts reads its
    widths against the reconciled vertices. AE's own variable feather is
    therefore still not imported ([11-AE-IMPORT.md](11-AE-IMPORT.md)).
- **Type** - vertical type (`ToolMode.typeVertical` is armed and does nothing;
    it needs `lumit-text` to lay a line downwards); true glyph metrics across the
    bridge (the caret, the anchor and the gizmo all go through
    `estimatedTextWidth`'s half-an-em, and one measured advance width would
    replace all three); multiple lines and a character panel (font, tracking,
    leading, alignment - the document is one styled run,
    [03-DATA-MODEL.md](03-DATA-MODEL.md) §9.1).
- **Paint** (brush/clone stamp/eraser, [impl/paint.md](impl/paint.md)) - owed:
    **tilt** from a tablet, which wants a brush tip that can *turn* and so a
    shaped brush with an angle before it — the stylus's tilt fields are read by
    nothing today; **spacing** and **scatter**; a keyed Start/End's **curve in
    the graph editor** (the Timeline lane draws and drags its diamonds, but
    `graphChannels` walks transform, effect, retime, volume and mask rows only);
    painting in **Layer view** rather than on the composite; and **a GPU
    stamping path** (the rasteriser is a CPU loop beside the mask one, and it
    changes the rasteriser, not the stored stroke - it is also what would retire
    the 8-bit read-back a painted Precomp pays).
- **Camera** ([impl/camera.md](impl/camera.md)) - the rebuild landed the eye
    model, one- and two-node cameras with a Point of interest, the camera
    settings dialogue, rendered depth of field, the Unified Camera tool and the
    3D views with their wireframes. Owed: **depth-of-field handles on the
    picture** (focus distance and aperture are rows and nothing more); the
    **iris** — shape, rotation, roundness, the diffraction fringe and the
    highlight controls are not built, so the blur is one gaussian radius read at
    a layer's anchor; a **keyframed camera cannot be dragged**
    (`viewer_camera.dart` stands the tools down whenever any of the placement
    rows is not a still value, because there is no single number to add to); and
    a **drag spanning several layers is one undo step per layer** — `Op::Batch`
    exists and the switch cells ride it now, but no bridge call batches a
    transform write across layers the way `set_switch_on_layers` batches a
    switch.
- **Roto propagation speed** - the tools shipped (geodesic solve,
    flow propagation, guided Refine edge, the `roto/` sidecar tier, both tools
    armed), but a propagated 1080p frame measures 895 ms against §7's 60 ms
    target - the WGSL ports [impl/roto.md](impl/roto.md) names are owed.
    Puppet shipped whole; its recorded upgrades (GPU warp,
    sparse factorisation) have fired triggers and wait on the same quiet day.

**Smooth zooming everywhere else.** The shared helper is built
(`widgets/smooth_zoom.dart`) and both the Timeline and the Audio timeline read
it. Still cutting rather than flying: the **graph editor's** zoom and auto-fit —
a matter of holding a `SmoothZoom` and reading its value, with no design left in
it.

**Layer controls in the Viewer ([07-UI-SPEC.md](07-UI-SPEC.md) §2.3):**
- **Motion paths** (§2.4) - a keyed position draws no path and its keys cannot be
    dragged there. There is no motion-path code in `flutter_ui/lib` at all; the
    multi-viewer work moved none of it, because there was none to move.
- **Scale and rotation of a multiple selection** - each layer keeps its own box
    and only a lone selection grows handles; AE scales a set about one shared box.
- **Snapping reaches for guides and the grid, and nothing else** (§2.2 item 6).
    `viewer_snap.dart` pulls a drag onto a guide or a grid line, measured in
    screen pixels, with `Ctrl` suspending it exactly as it does in the Timeline.
    What it has no targets for is **other layers** — their edges, their centres —
    which is the half of §4.5 and §1.7 still owed.
- **Parent-aware and 3D gizmos** - the box is built from the layer's own
    transform rows, so a parented layer's ignores its parent, and `ViewerLayerMap`
    is a 2D map, so a 3D layer's ignores the camera. The 3D views' wireframes are
    a separate thing the engine gathers (`CompositionReference::wireframes`) and
    sit under an `IgnorePointer`: drawn, never aimed at.
- **A keyframed position draws no box**, so an animated layer cannot be picked on
    the picture — `viewer_stage.dart` leaves out any layer whose position is not
    a still value, on the ground that a box in the wrong place is worse than
    none. It wants the value *at the playhead*, which the read model does not
    carry: a `BridgeScalar` is static, keyframed or an expression, and never the
    number a frame was drawn with.

**Pixel pickers ([07-UI-SPEC.md](07-UI-SPEC.md) §6.1):**
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
- **The macOS FFmpeg 8 route has never run on a Mac.** Homebrew has no
    `ffmpeg@8` formula and its plain `ffmpeg` is already 9.x, so
    `.github/actions/ffmpeg8-macos` extracts 8.1.2 out of homebrew-core's
    history, builds it from source and caches the keg. Every macOS job and the
    release DMG now take it, and its final step refuses anything whose
    libavutil is not 60.x. What is owed is **one green run on a real macOS
    runner**: the branch was written on Windows, so the extract, the source
    build, the cache-restore path and its receipt-driven dependency reinstall
    are all reasoned rather than observed. Retire the action outright when
    Homebrew ships `ffmpeg@8` (name the keg and delete it), or when rsmpeg
    gains an `ffmpeg9` feature.
- **The macOS IOSurface Viewer path is unproven** - CI links the bundle but
    nobody has launched the .app.
- **The macOS .app is not relocatable** - the podspec links Homebrew FFmpeg by
    absolute Homebrew path. Distribution needs the dylibs vendored and install
    names rewritten.
- **The macOS build is single-architecture** - `pkg-config-rs` refuses to
    cross-compile and a keg holds one architecture, so `ARCHS` is pinned to the
    runner's. A universal bundle needs both `ffmpeg@8.1.2` kegs and per-slice `-L`
    flags, plus a decision on whether Intel macs are supported at all.
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
    preview scale - correctness-safe, but it is the one composite the
    scale does not shrink.
- **The Linux DMA-BUF path has never run on a Linux machine with a GPU.**
    It fails calmly on the adapter-less CI runner, which proves the failure is
    calm and nothing about the path working.
- **frb's SSE codec encodes `Vec<u8>` one byte at a time** - now taxes only
    thumbnails and scope traces, but worth the bulk codec if traces feel late.
- **Engine subsystems with no frb API** - the Retime **graph**
    (`with_segment_ease`, `with_segment_speeds`, `with_segment_as_rate` in
    `lumit-core/src/retime.rs`) and the curve view that makes ramps editable;
    `trim_to_source_end`.
- **The audio mix is rebuilt from scratch** whenever the comp's audio signature
    changes, rather than patched: `prepare_once` walks the whole comp for jobs,
    hashes them, and loads the lot again when the hash moves.
- **The Sound mix row's waveform is strided.** `MixPlan::peaks` reads the loaded plan
    frame by frame and strides past 256 frames a bucket, so a one-frame transient can
    drop out of a comp-wide view. A peak pyramid built on the prepare worker is the
    upgrade if the row is ever cut against rather than read.
- **A hosted plugin's stepped parameter never reaches it.** `kind_of` in
    `crates/lumit-aplug/src/schema.rs` gives a stepped plugin parameter of nought to one a
    `ParamKind::Bool` row, and `bake_values` in `crates/lumit-render/src/export.rs` keeps
    the `EffectValue::Float` rows only, so a plugin's bypass or its mode switch is drawn,
    stored in the .lum and never sent. Either the row becomes an Int, as a built-in's mode
    row is, or the bake carries Bool as well.
- **A Precomp layer's effect rack is silent to the mixer.** `audio_chain_of` opens a rack
    on Footage, Sequence and clip layers only, so a rack on a nested comp's layer processes
    nothing; Convert to precomp copies a track's rack on to each clip layer for that
    reason. A bus chain, the nested comp summed and run through the Precomp layer's rack,
    is the upgrade, and it needs a bus stage in `MixPlan` first — the same stage the Audio
    timeline's missing buses and sends want.

**Retime follow-up after the property-path move.** **The eased ramp shapes are
gone from clips** — `Clip::with_ramp` takes two speeds and runs straight between
them, which is what the envelope authors. Slow/Fast/Smooth/Sharp come back with the
preset-shelf rework below, rebuilt on the property like everything else the move
carried.

**Appearance.** The seven built-in schemes still restate every colour
individually; only a handful default from the mode — the Timeline's out-of-range
and selection fills, the marker grey, the waveform palette and the modal scrim.
Still owed: a swatch strip per row **inside** the picker's menu (the strip built
beside it previews the selection only), and a place to keep themes other than the
workspace file, so an imported theme travels with the user rather than the
machine's settings.

**Shell and onboarding:**
- **The boot splash says only what `boot_log` says.** It is mounted now
    (`BootGate` in `shell/app_shell.dart`) and streams the engine's own boot log,
    which is all the engine can tell it: there is no notice stream to subscribe
    to, so a module that took a long time coming up, or came up degraded, cannot
    say so on the splash. Wants an engine-side boot event stream before it can.
- **First-run setup screen: the four-card version**
    ([07-UI-SPEC.md](07-UI-SPEC.md) §13.1) - §13.1's four cards, a small image over
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
    the moment a second position attaches to it, which is what `positionOf`
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
- **Column widths and the property selection are session-lived** - fold into the
    workspace when per-workspace column layouts land ([07-UI-SPEC.md](07-UI-SPEC.md)
    §4.2).
- **Beat tap has no key left** - [07-UI-SPEC.md](07-UI-SPEC.md) §10 wants `8`
    during playback to tap a beat, and the bare digits went to the numbered
    markers. The Audio panel's Tap button is the only way in. Needs its own chord
    or a modal reading.
- **Edge-follow has one manner, not two** - the lanes flip a page when the
    playhead leaves the viewport (TI-9); [07-UI-SPEC.md](07-UI-SPEC.md) §4.6 also
    wants a *smooth* follow and a setting to choose between them. `Shift+=` (zoom
    to the work area) is unbound for the same reason: neither was in TI-9's own
    sentence list.

**Render-time indicator follow-ups (the column landed).** What ships measures by
*fencing* — the render waits for the card at each layer and each effect before reading
the clock, and composites a held frame again on the idle turn to do it — so it
never runs during playback and only the frame under the playhead is measured. §7.1's target is continuous collection at negligible cost, which wants **GPU
timestamp queries**: a query set per frame, timestamps written around each node's own
submission (every effect kernel already submits its own command buffer, so nothing
inside `lumit-gpu`'s kernels changes), resolved a frame later. With those in the switch
could go. Also owed from §7.1: **sorting** the Timeline column, a **profiler panel**
with the recording mode (totals, percentiles, cache hit rates, time per
degradation-ladder step), and per-layer numbers for the layers *inside* a Precomp.

**The preview progress bar's fractions are stage weights, not measurements.**
Decode is assumed the long pole and each top-level layer an equal share of
the composite, so a comp whose one adjustment layer outcosts the twenty layers below
it fills unevenly; feeding the profiler's measured costs back as the weights is the
fix. Also unbuilt: an **export**'s progress still has its own path
([07-UI-SPEC.md](07-UI-SPEC.md) §14) rather than sharing this one.

## Now - the redesign programme

**All four phases of the 2026-08-23 redesign have landed** - the theme groundwork, the
panels and windows, the node graph and the Nodes workspace, and lumitlab.com - and with
them the points-stream programme and the timeline interaction programme's ten packages.
15-DESIGN §12A stays the binding description of the resting state, and the per-surface
metrics tests are what hold the surfaces to it: `export_metrics_test`,
`settings_metrics_test`, `project_panel_metrics_test`, `viewer_metrics_test`,
`welcome_metrics_test`, `graph_panel_metrics_test` and `timeline_alignment_test` each read
the approved drawings' own computed styles, so a value that disagrees with a drawing is a
defect. Two things outlive the programme:

- **The Chrome labels setting still has one reader.** The three-way setting ships as
    Icons and carries its word in every tooltip, but the only chrome that consults it is
    the Timeline's Switches / Modes / Parent toggles
    (`panels/timeline_toolbar_frb.dart`); no other button, tab or toggle changes when it
    does, and **Icons everywhere** - panel titles as glyphs - has no reader at all.
    Converting the rest of the chrome a surface at a time is what is left of phase 1
    (15-DESIGN §5.1).
- **Later, gated - the Flutter multi-window upgrade.** Blocked upstream: windowing is
    main-channel-only, flagged, and its API promises breaking changes, so Lumit takes no
    production dependency on it until it reaches the stable channel un-flagged. The status
    line in **docs/impl/multi-window.md** §1 was re-checked on the 3.47 upgrade and still
    finds no stable setting for `windowingFeature` at all, so `flutter config
    --enable-windowing` cannot turn it on here - re-check it again before planning any of
    this. The phase opens with that note's cheap spike (§6 step 2): can a second window
    composite the engine's shared Viewer texture at all? Then the `WindowManager` root on
    the main window only, the welcome window, the dialogs - mostly free, because every one
    is already an in-window `showDialog` overlay on the one pattern
    (`shell/dialog_frame.dart`), which was the migration prep and not a stopgap
    (multi-window.md §5) - the settings/theme/queue windows, and last the satellite
    tear-off panels, which is where the old pop-out-panel-windows rebuild item is folded
    in. A Viewer is already that unit: **several Viewers open at once in the one window**
    and a view id survives a move, so a second monitor is the only thing windowing adds
    there (docs/impl/multi-viewer.md §6).

## Now - the node graph composition (docs/impl/node-graph-comp.md)

**Shipped in 0.4.0** (`78589ef`): a composition whose picture is made by nodes and wires,
with Merge and Switch, Read and Input boxes, and the Node graph effect that applies one to
a layer. NG1 to NG6 landed together with round two's engine, bridge and documentation
halves, so the eleven boundaries v1 named are lifted everywhere but the frontend - each
subsection of the note's §5 carries a Built paragraph saying which half of it is in. What
is left is the Dart that the bridge already feeds, and both halves are recorded as known
gaps in 0.4.0's release note:

- **P4, the canvas and the Node panel** (§8): Save group and the saved-groups block in the
    comp canvas's console, the picture Input's preview picker in the Node panel's Input
    form, and the no-stream word on a box's rows - `_drivenInGraph` in
    `panels/node_panel.dart` writes `noStream: false` rather than reading the box's inputs.
    `saveGraphGroup`, `insertGraphGroup`, `listGraphGroups` and `BridgeGraphInput.preview`
    all cross the seam and nothing calls them.
- **P5, the Timeline and Effect controls** (§8): a graph's Fx boxes as Timeline rows with
    their lanes under the `n:<box>` prefix, the placed graph's Inputs as a section in
    Effect controls and a fold in the Timeline, the dimmed collapse and audio cells, and
    the Retime clock face. `BridgeCompModel.graph_boxes`, `BridgeLayerInfo.graph_inputs`
    and `collapse_forced` are filled and read nowhere, and the Timeline still draws only
    the hint placeholder for a node graph (`NodeGraphTimeline`,
    `panels/timeline_layer_rows_frb.dart`).
- **The manual is ahead of both.** `web-docs/.../use/node-graphs.mdx` already describes the
    Effect controls section and the Timeline's box rows as things the editor draws; it
    becomes true when P4 and P5 land, and until then it overclaims.

## Next - colour management: OCIO (docs/impl/ocio.md)

**All six work packages have landed**, and after them the four OCIO effects (`d0aab70` -
colour space, display, look and file transform, plus the project's choice to composite in
the config's `scene_linear` role) and the partial-config reading that greys out each name
a config cannot make with its reason (`147ef31`). docs/impl/ocio.md stays the binding
description; `crates/lumit-colour`'s suites - the eight CLF documents vendored byte for
byte, the `aces-1.2` and `aces-cg` reference fixtures, `tests/refusals/` as the taxonomy's
corpus - and `crates/lumit-render/tests/ocio_parity.rs`'s colour matrix are the record.
One thing is still owed, and it blocks nothing:

- **Exact Rust ports of the eight vendored styles** (§4.1 tier two → tier one), one at a
    time, each landing against the `aces-cg.fixture` or `builtins.fixture` rows that gate
    the bake it replaces. The number they exist to bring down is **0.117 at the Rec.709
    blue primary**, which is what a 65-point cube costs on the ACES 2.0 rendering; inside
    the gamut the same bake is better than 2 × 10⁻³. It also reclaims the 76 MiB of
    artefact files an installation ships beside the binary
    (`crates/lumit-colour/vendored/`, which the packaging scripts copy into
    `data/colour/`; they are no longer compiled into the executable).

## Next - engine/bridge follow-ups

**Settings pages still unbuilt (docs/07 §15's remainder):** CUDA on/off and the
plugins/decoder page - the two the sidebar still has no row for. Audio, Autosave and
Export defaults all landed; colour management lives in Project settings; the
preview-mode toggle exists, on the Preview and cache page. Both land wired to the
engine through the bridge, not as a Dart-side setting nothing reads.

**The Export drawing's rows are built, both halves** (engine and interface; docs/06
§7.4–§7.5, docs/15-DESIGN.md §12A.4). Audio-only output (`.m4a`/`.wav`), colour
depth, channels and alpha, the output colour space, crop and *use region of interest*,
container metadata, the named preset store, the auto bitrate, the render settings (quality,
disk cache, effects, solo switches) and the *when done* hook are all on
`lumit_render::export::ExportSpec`, all across the seam on `BridgeExportSpec`, and all on
the dialog's one scrolling page, with a per-format capability table refusing what a format
cannot carry. Every face the drawing shows is a live control writing into
`BridgeExportSpec` — the resampler's filter (Fast bilinear, High Lanczos-3), the
colour-space family of five built-ins the container is stamped with, the free width and
height boxes with their aspect lock, and the sound's rate, width and layout — with the
capability row deciding which of them the chosen format may honour. The one control still
drawn dead is the *Managed by* row — which since the OCIO UI landed reads the project's
config path rather than "No OCIO configuration", and stays dead on purpose: colour
management is chosen in Project settings, and an export dialog must not edit the project.
What is left:

- **The *Still* output type is withdrawn**: a still is an image sequence of one
  frame, which the span already says, so the fourth chip the drawing offered is gone rather
  than pending. What is genuinely missing is only the *naming* — a one-frame sequence is
  written `shot.00001.png` rather than `shot.png` — and that is a rule in the encoder's
  file naming, not an output type.
- **A disk-cache policy with something to govern.** The setting exists and defaults to Off,
  which is what happens: the export renderer is a fresh `HeadlessRenderer` with no disk
  tier at all. *Read-only* becomes a real choice the day the export path gains one.

**Proxies — the subsystem landed, the interface has not** (docs/03 §3a, docs/06
§5.7). The engine half is in: a second media reference per footage item
with its own probe, resolved and fingerprinted like the original; a per-item and a
project-wide *use proxies* state on undoable ops; one resolution point the decode planner and
the frame key both go through, so proxy and full-resolution frames can never share a name; a
background transcode that makes one (`name_proxy.mov` beside the original, half size); and
`RenderOptions::use_proxies`, off by default, so delivery reads the originals whatever the
Viewer is working at. A proxy that disagrees with the original about frame count or rate is
refused and falls back. The interface landed too (the seam and the panel: set/clear,
MAKE-PROXY with its progress, the per-item tick, the project-wide switch, the badge).
Still open: **nothing says whether the proxy *file* itself is broken** - `BridgeProxy`
carries a path, this item's tick and whether the document reads it, and nothing else. That
wants a new query over the renderer.

**Two small settings follow-ups** — the "Show shortcut hints" switch is specified
(docs/07 §15's Interface page) and nothing in the frontend either draws it or consumes a
hints flag (the menu bar and tooltips must read it before the switch can honestly exist);
and the Settings drawing's slider face (2px track, primary knob, no fill) disagrees with
the Main drawing's zoom slider that `HouseSlider` was built from — one widget draws both
surfaces today, at a 4px track with an accent fill and a `text_secondary` knob, and each
surface should wear its own manifest's face.

**An autosave does not refresh the welcome picture.** Every *save*
files one, and opening a project that has none draws one, so no row is empty any
more. An autosave is the one write that does not: it runs on the engine's own timer
thread, and the file it would have to write is named by a digest the frontend owns
(`Workspace.thumbnailKey`) in a folder the frontend owns. Teaching the engine that
name would make one filename two sources of truth. The cost is only that a picture
can be up to one editing session stale, which is what it has always been; the fix,
if it is ever wanted, is an "autosaved" event on the change stream that the frontend
answers by drawing a fresh thumbnail — not the engine writing the file.

**A control for an image sequence's frame rate.** The rate is stored,
saved and read by everything that opens the run, but nothing can change it: an
imported sequence plays at the 25 default. It wants a row in the Project panel's
item menu beside *Relink…* — a rate field, one op, one undo step — plus its arb
strings. The engine side is a `SetSequenceRate` op and a bridge setter; the model
already carries the field (`FootageItem::sequence`).

**Tracking a sequence.** `lumit_render::track` still opens footage
by bare path — `MediaLuma::open` probes and decodes one file — so a camera track over a
run of stills analyses its first frame alone. It wants the same `MediaSource` the Viewer's
decode already takes; `crates/lumit-bridge/src/api/track.rs` resolves the path it hands
over.

**A relinked run keeps its old name.** The Project panel names a sequence for its
span — `frame[0001-0050].png` — and relinking rewrites the media reference but not
the name (`FootageReference::relink` emits `SetMediaRef` and nothing else), so a run that
gained or lost frames while it was away shows a stale span. It wants the relink to rename
a sequence item in the same batch.

**Camera tracking, phase 4 stage 3** (docs/impl/tracking.md §5a–§5b).
Stage 1 landed the model half — `ParamKind::Action`, the Camera track effect, the
solve link and Convert to keyframes, all against an injected solve. Stage 2 landed
the engine half: `lumit_render::track` — the analysis job on its own thread with
cancellation between frames and inside the solve, the `track/` sidecar keyed by
(media fingerprint, settings, mask geometry), the real `CameraSolveStore`, the
conversion into `CameraPose`, and the derived camera threaded into the render path
and the frame key.

**Stage 3 largely landed** (the effect's Analyse/Cancel and staged status readout,
the point cloud following the effect and the solve, the analysed-span bar), and the 2D
track exports came with the planar tracker — **Create corner pin** and **Create transform
keys** both write ordinary keyframes onto the named Pin layer. Still owed from §5c's own
list: a **Tracking workspace**; a **picker that links an existing Camera layer**
(`set_camera_solve_link` is the primitive under it, so this is a panel away); the
**layer-transform-aware cloud placement** (points come back as the footage's raster centred
on the comp, which is exact for the ordinary case and wrong by that transform otherwise —
the fix is a change to `CameraSolveStore`, not arithmetic in the overlay); and the cloud's
own affordances — a count, a filter, deleting a point, hiding points behind the shot, and
setting the ground plane and origin from a selection.

One thing stage 2 left, recorded in docs/impl/tracking.md §5b:

- **One analysis at a time.** A second `request` while one runs answers `Busy`
    rather than queueing. Deliberate (two disk-bound jobs halve each other), and a
    queue is a small change if anyone asks for one. The warm pass deliberately does not
    take the slot, or a project's second tracked clip would never be read back.

**A held re-render cannot see footage move** (the re-render work's remainder). Fast
motion blur and Datamosh now measure the composite an adjustment layer or a
Precomp layer actually shows, by building it again at the neighbour time and
measuring between the two textures. What that neighbour composite cannot show is
**footage motion**: a re-render re-decodes nothing (docs/impl/temporal-rerender.md
Traps), so every footage layer beneath the adjustment carries the *same* decoded
frame in both pictures and contributes zero flow. Comp-driven motion — transforms,
effects, cameras, nested animation — measures correctly; an adjustment over
plain playing footage measures nothing, and the effect has to go on the footage
layer instead.

Closing it is the FX-1 shape one level up: `posterize_sample_times` already makes
the decode planner snap *which source frame each covered layer decodes* for a held
re-render, and this wants the sibling — the layers beneath a flow-consuming
adjustment decoding their `±1` neighbours too, and `below_draws_at` handed that
second set of pixels. Planner, decode worker and builder all move, which is why it
was not folded into the re-render work.

**Flow's remaining work.** The engine, the GPU port, the cache and the
controls have landed. What is left:
1. **`PreviewEngine::default` still builds its pool without a GPU**, so that
    path measures flow on a headless device of its own; the headless renderer
    the Flutter frontend drives shares the render device correctly
    (`DecodePool::with_gpu`). Nothing reads the one that gets built — it is a
    `WorkerState` field no code touches — so deleting the path is the cheaper of
    the two answers.
2. **The remaining CPU work in synthesis is the luma conversion and the frame
    uploads** — about 70 ms of the 79 ms a 1080p interpolation costs, against
    8 ms for the flow itself. Both would go if the decoded frame reached the
    card once and stayed there, which is the `DrawSource` change already sketched.
3. **The learned ceiling** — RIFE-class synthesis, WAFT-class flow — now has a
    judge to be measured against: `flow_quality.rs` and `clip_cadence.rs` landed
    and the measurement programme ran through them
    (docs/impl/optical-flow.md §4.5–§4.7, §5.5). A learned synthesiser emits no
    flow field, so Motion blur and Datamosh need DIS vectors regardless.
4. **A second matching cost is measured out, not open.** Census scoring cost
    game capture 0.0073 against a 0.005 allowance; choosing census
    or SSD per patch from the Hessian trace recovered most of it
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
    now measured rather than argued: census matching closed part
    of the gap at a cost elsewhere that the bar refused, and edge-aware
    densification lost on four of five clips because a field-space
    solve cannot add evidence. A third attempt must be **evidence-bearing** —
    something that measures line art better, not something that smooths a
    finished field again.

**Not to be built: a `flow/` disk tier.** docs/06 §5.4 reserves the folder and it
should stay empty. Measuring a 1080p pair on the GPU costs ~8 ms; reading 37 MB
of stored field off an SSD costs more. It would be a cache slower than the thing
it caches. The RAM tier (`DEFAULT_FLOW_CACHE_BYTES`) is the one that pays.

**Localisation follow-ups.** The seam is built and the strings are out of the
code (`flutter_ui/lib/l10n/`); the round trip is the translate page, `scripts/translations.ps1`
and the `translation-state.json` sidecar that records the English each line was translated
from, so "stale" is a fact rather than a guess. What is left is other people's turn and one
gap:

- **The two numbered shortcut labels stay English.** `lumit-keymap` builds "Add marker
  {n} at the playhead" and "Go to marker {n}" with `format!`, so they are not literals
  the lookup table can hold (`lib/l10n/engine_labels.dart`). Give the bridge the number
  separately, or the label a stable id, and they join the rest.

**Lens flare follow-ups ([impl/lens-flare.md](impl/lens-flare.md))** — the
shipped core is docs/08 §3.27; its performance items sit in **Now** above. Still owed:
the **Lights source wiring**; an **image aperture** file parameter; the **lens
designer** (`lens_file` has landed, so its output has a place to go); an
**Occlusion layer** reference; **adaptive grid refinement at vignette folds**, the real
cure for both known limits (six ablations are already ruled out — do not re-chase
them with guards). Panel side: the pair row's dropper on
**Transform's px@comp pairs** (the pick exists, on a depth-of-field focal point alone);
one-op writes for a paired keyframe toggle.

**The stale-fd race on a Linux Viewer resize** (`lumit-render/src/headless.rs`'s
`shared_dmabuf` pool, with `lumit-gpu/src/shared_linux.rs`'s `Drop`). The pool now
keeps one target per view and size and evicts by `shared_pool_evictions`, so a resize
churns fewer handles than it did — but the exported descriptor is still closed when
`SharedDmabuf` drops, and the descriptor *number* travels to Dart asynchronously, so an
eviction can have Dart register a closed fd - or one the OS has since reissued. Either
hold the evicted `SharedDmabuf` for one generation, or `dup()` at export so the number in
flight owns itself.

**Ramp preset shelf rework** - the Linear/Slow/Fast/Smooth/Sharp buttons need a
general rethink (owner, 2026-08-02) before they return on the property path; not
a Vegas-mode concern ([04-RETIMING.md](04-RETIMING.md) §12.2).

**Retime UI wiring** (UI/command affordances - [04-RETIMING.md](04-RETIMING.md);
these return on the **property** path — the segment calls named here
are the reference for behaviour, not wiring targets):
- Freeze-at-playhead (`insert_freeze` built, no caller); Hold preset button;
    RATE/MAP type chips; kink badge; graph overrun band + source-out reference
    line; compensating Alt-drag; copy/paste a retime between clips;
    outward-trim-extends-map; the retime keyboard shortcuts (§12); Blend
    interpolation toggle; the source-rate advisory badge (§10's "holding each
    source frame" guidance, when a segment's sampling ratio strays — the
    Flow-params rows themselves landed, in Effect controls, reading and writing
    `api::retime`'s Flow group).
- The Time-lens **vertical (source-position) boundary drag** has no bridge op -
    `Retime::from_source_keyframes` (`lumit-core/src/retime.rs`) is unexposed, and
    the `SetLayerRetime` op this entry used to name alongside it no longer exists
    at all, since Retime moved onto the property path.

**Bridge reads left outside the read model** - the Source card's text, source-item,
mask and interpolation reads for the selected layer, the Viewer's missing-file probe, and
the **composition's** own marker and work-area reads on a Timeline rebuild. A layer's
markers and a Camera layer's settings both ride `BridgeLayerInfo` now; fold any of the rest
into `BridgeLayerInfo`/`BridgeCompModel` if they show up in the budget ranking.

**Thin-view debts the 2026-08-10 audit left for engine API** - each is Dart
doing the engine's job and each wants one bridge call:
- `viewer_camera.dart` still picks the active camera itself, by walking the held
    layer model, and re-derives the renderer's Ry·Rx·Rz basis in Dart; the *pose*
    crosses properly now (`layer.cameraPoseAt`, so a two-node camera's aim and a
    solve link are composed engine-side). Wants `comp.activeCameraPose(frame)`
    for the half that is left.
- `viewer_type.dart` mirrors the engine's text-width estimate (caret, anchor,
    gizmo all share `estimatedTextWidth`); wants a `layer.textMetrics` read.
- `viewer_gizmo.dart`'s `_pathBeingEdited` parses `<layer>/masks/<mask>/path`
    strings in a widget; wants the selection model to expose the pair.
- The shape tool's Ctrl+Z pops draft points locally (a second undo meaning);
    wants engine-side draft ops so undo stays the document's.
- `FlowRowsFrb.build` (Effect controls) still reads four flow getters in
    build; same class of defect the audit cleared from the Timeline's rows.
- `theme_tokens.dart`'s `_with` restatement wants `LumitTheme.copyWith` in
    `theme.dart` to cover the tokens, whose five-field shape is documented as
    deliberate - an owner call, not a mechanical fold.
- `headless.rs`'s four per-platform present-target-pool bodies share one dance;
    fold them on a machine that compiles the macOS/Linux paths.
- `ExpressionContext::comp_time` is raw `f64` across an engine boundary
    (docs/14 typed time; `lumit-core/src/expression.rs`); rhai's seam is f64
    regardless, so the typed carry is a three-file ripple best taken while
    `fx/resolved.rs` is quiet.

**`LumitAppNew` rebuilds the whole app on any `LumitUiState.notifyListeners`** (a
`ListenableBuilder` above everything), and un-scoped document changes do the same
via `LumitState`. Reads are nearly free; the widget-tree rebuild is not. Scoping
the visible tree remains.

**Playback scheduler - what remains**
([impl/playback-scheduler.md](impl/playback-scheduler.md)): in-render epoch tokens
(composites are serial on one worker thread, so cancellation latency is one
frame's render — 200 ms for the reference comp at 32 animated layers — rather than
§1's 15 ms), and §6's real-window benches (A/V drift over 10 minutes, the underrun
ladder). Re-run `integration_test/playback_bench_test.dart` to price the stack; it needs a
1080p60 fixture and a Windows device, so it is run by hand.

**Engineering-rules tooling still owed** ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)):
fuzz targets for the `.lum` deserialiser and journal replayer (§6 — `lumit-ofx`'s
`handle_fuzz.rs` is a hand-written test, not a cargo-fuzz target, and there is no fuzz
directory); the **edition-2024 move** (§9 - the toolchain pin landed at 1.97.1, the
edition did not: the workspace and every crate still say `edition = "2021"`); the
`indexing_slicing` / `arithmetic_side_effects` clippy denies after a hot-path sweep (§4);
`clippy::pedantic` with curated allows (§7); the golden-frame EXR export corpus (§6 —
today's goldens are in-crate oracles).

**Four unmaintained dependencies are deliberately ignored in `deny.toml`.**
`ttf-parser` (via fontdue, via `lumit-text`) is the one with a real successor: moving
the rasteriser to `skrifa` is its own piece of work with its own glyph-metric tests.
`bincode` 1.x, `paste` and `smartstring` (via rhai, retired 2026-08-11 in favour of
compact_str/smol_str) leave when the dependencies that pull them update.

**A genuinely FFmpeg-free build is not possible yet.** `lumit_bridge
--no-default-features` compiles the bridge's own decode paths out, but `lumit-render` and
`lumit-audio` depend on `lumit-media` unconditionally, so the library is still linked and
the build still needs it installed. Making those two deps optional — and the render/audio
paths that use them — is what "builds without FFmpeg" would actually take.

**The three-tier cache's remaining sharp edge.** The disk tier's write queue was
bounded after it reached 81 GB on an idle Mac, and its depth is reported now
(`DiskIo::pending_parks` → `BridgeDiskCacheStats::park_queue_frames` → the Preview and
cache page); the same shape of question is worth asking of the *other* unbounded `mpsc`
channels the worker owns (the loaded-frame return, the prefetcher's results) — none
carries whole frames as freely as the park queue did, but none counts its depth either.

**What the performance harness still cannot measure** (`crates/lumit-bench`
drives the reference comp headless through B3, B4, B5, B6, B7 and B11, and adds
Particulate's B12–B14 and the puppet's B15–B17, which need no comp and no media; the job
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
budget's actual number. A **stress comp** (4K, 20 layers) is unbuilt, and §7.3's per-effect
cost-class benchmarks are begun rather than done: B12–B14 are the first three and ride the
same baseline file and ratio gate as the rest, and every other built-in effect still wants
its own per-dispatch time and memory measurement against its declared cost class. The
per-node profiler (§7.1) now has its first visible piece - the render-time column - and the
rest of it (continuous timestamp-query collection, the recording mode, the panel) is in the
**Render-time indicator follow-ups** entry above.

**CI coverage the Flutter port left thin:**
- **macOS and Windows CI do not require an adapter.** `LUMIT_REQUIRE_GPU` turns
    a "no adapter" skip into a failure and the Linux job sets it (as does the bench
    job); the other two do not, because nobody has confirmed those runners
    enumerate one. One run with the variable set says whether they can.
- **Nothing in CI proves a Viewer frame arrives.** The Linux job is the only one
    running the Flutter suite and has no GPU, so the six Viewer tests that wait
    for a frame skip there on `LUMIT_NO_ZERO_COPY_VIEWER=1`. They still fail on a
    regression on any machine with a real adapter, so the owner's box is the gate.
    A Linux runner with a GPU, or a Windows job running `flutter test`, closes
    this and verifies the DMA-BUF path at the same time.
- **Registering a texture cannot happen in a widget test**, so
    `integration_test/shared_texture_test.dart`, run by hand on a real window, is
    the only coverage of that path.

**Threading / platform:**
- **Shared-texture producer/consumer fence on Linux and macOS** - both presents
    still say "no fence yet" and settle for `device.poll(Maintain::Wait)`; Windows
    has a real one (a `D3D11_QUERY_EVENT` ended and waited on around the copy).
    Only if a live run shows tearing; verify on the machine first.
- **Export options still to build** ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)
    §7) - export priority and encoder preference order, and §7.5's **reframe**: the
    vertical variant ships as a preset (`ExportPreset::Vertical1080p60`, 1080×1920)
    but only off the 1080p60 landscape preset, and with neither the draggable
    centre-crop nor the pillar-fit alternative the spec asks for.
- **Export status still speaks the old idiom** - `crate::export` replies in JSON
    strings that `api::export::export_poll` re-parses on a timer; follow the
    worker's typed-stream way.

- **Three shipped chords still have nothing answering them.** The keymap binds
    them and Settings ▸ Keymap lists them, so they can be rebound and still do
    nothing: `,` / `.` (**previous / next keyframe** — it needs a rule for what
    "the keyframes" are when no property row is picked), `Ctrl+,` / `Ctrl+.`
    (**previous / next edit point**, which needs an idea of what counts as an
    edit point in a comp), and `K` (**shuttle pause** — the shuttle itself is not
    built, which is why `J`/`L` step a frame instead). Everything else in docs/07
    §15 is dispatched.

- **The menu bar names its own backlog.** Every row marked
    "(Not implemented)" in File/Edit/Composition/Layer/Animation/View/Help is a
    command with a place waiting for it. What is left, now that the rows whose
    engine call already existed have been wired — History, Trim and Crop comp,
    Layer styles, the Animation menu's three Reveal rows, the View menu's grid,
    ruler and snap rows, **Save** and **Apply animation preset** (the same
    `.lumfx` the Effects & presets panel writes), and the Camera family (Layer ▸
    Camera settings and Animation ▸ Track camera): **Preserve transparency**,
    **Auto-outline**, the Layer menu's **Reveal** row, and **Track motion** —
    each of which needs an engine or bridge seam that does not exist yet. One is
    marked for a reason rather than for want of time: **Add text selector** has
    nothing to add while a text animator carries exactly one range selector.
    **Show wireframe** is not marked at all — it is a live toggle wired to the
    layer-controls switch, and stays that switch until the full wireframe display
    mode (docs/07 §2.2 item 5) gives it something of its own. Delete each mark as
    the command lands. No chords are suggested for these; the shipped table is
    docs/07 §15.

- **A Flatpak remote, so `flatpak update` has something to update from.**
    Releases ship a single-file `.flatpak` bundle, which installs perfectly well
    and then never updates: `flatpak update` needs a remote, and `release.yml`
    builds its OSTree repo only to bundle out of it and throw it away. Export it,
    publish it (Cloudflare Pages beside the site) and ship a `.flatpakref`, or
    submit to Flathub and let it host. Until then Lumit tells Flatpak users the
    install command rather than offering a button.

## Later

**AE import, phase 1 (docs/impl/ae-import.md §6) - the walker, the reader, the
structural mapping, both halves of the effect table and the surface all landed
2026-08-20..21.** `tools/ae-bridge/` holds the ExtendScript walker and the fixture
builder; `crates/lumit-import/` the bundle open, the capture types, `map/` and the typed
`ImportReport`; `crates/lumit-bridge/src/api/import.rs` the adoption and
`flutter_ui/lib/shell/ae_report_frb.dart` the report window.
`crates/lumit-import/tests/golden.rs` holds the lot against the golden bundle from one
sitting on a live After Effects 26.0 (`tools/ae-bridge/fixtures/fixture.lum-bundle/`, two
comps, 24 layers, 109 unreadables), beside the two hand-written ones — the schema's
readable documentation, and the awkward half one well-formed AE project does not contain.
What that sitting and the two halves left open:
 - **Roving, and it needs one more AE sitting.** `make-fixture.jsx`'s
   `setRovingAtKey(2, true)` did not take — the capture records `roving: false` on every
   Position key, and the walker reads `keyRoving` correctly, so the fixture never
   contained a roving key to import. The next sitting should set roving on a key whose
   neighbours are bezier, and report what the builder's step harness caught if it still
   refuses.
 - **Material Options' Casts Shadows is dropped without a report row** — the one place
   the mapping loses something silently, against its own standing rule. Lumit has no
   field for it, which is the ordinary case for a row; every other such row is raised,
   and this one is not.
 - **The golden-frame tests §5 requires of every mapped conversion.** The golden
   *bundle* has landed and `tests/golden.rs` checks every converted number against one
   worked out from the fixture's own inputs, which is not the same thing: these need
   After Effects *renders* of `fixture.aep` to compare pictures against. They also
   replace **three undocumented bases that are stated choices, not measurements** —
   Fractal noise's Scale, Advanced Lightning's Turbulence and Add grain's Softness
   convert on the "AE's default lands on Lumit's default" anchor docs/11 §5 records.
 - **A keyframed dropdown in the colour half goes by unremarked.** `fx_colour`'s reader
   for the controls Lumit does not animate — option lists, switches, seeds — reads the
   still value only, so an instance whose Fractal type (say) is keyframed imports at
   Lumit's default with no report row. `fx_distort` reports "the value it starts on" for
   the same case; both halves should. Rare in real projects and behind no docs/11 clause,
   but it is the one place either half changes something silently.
 - **Ten match-name rows are claimed but unaudited** (docs/11 §5's `pending_audit`): the
   five Controls and the five added with the file itself — Invert, Exposure, Apply Color
   LUT, Sharpen, Unsharp Mask. Their match names are the famous ones, but their property
   trees were not in the 2026-08-20 captured set, so the parameter numbering is
   reconstructed rather than read off a live After Effects.
   `tools/ae-audit/claimed-matchnames.txt` carries all ten (60 names to 70). The next
   sitting confirms them; a wrong one costs only the placeholder road §6 already
   specifies.
 - **Turbulent displace's Pinning maps at one index**: the audit records a dropdown's
   default but not its option strings, so only AE's own default (every edge) is pinned
   from evidence and every other index is reported rather than guessed. A second audit
   pass that enumerates option strings closes it, and would also confirm the orders the
   distort half took from Photoshop's published list (Warp's fifteen styles) and from
   AE's own defaults (Wave warp's eight pinnings, the ten-entry channel picker).
 - **The collected `footage/` copy is still owed.** Of docs/11 §2.5's four relink steps,
   the absolute path and the search-folder sweep both run, so a project copied across
   with its media beside it now comes up linked. What is left is the collected copy and
   the hash verification that wants it: write the `footage/` folder, store a genuinely
   relative path beside it, and the re-rooting and fingerprint steps start paying too.
 - **A report row does not lead anywhere** (docs/11 §9's navigation): a row names its
   comp ▸ layer ▸ property and double-clicking it does nothing. `BridgeImportRow` carries
   a path string and no id, so the row has nothing to navigate *to*.
 - **The report is not kept** (docs/11 §9's persistence): it lives as long as its window,
   is not stored in the project's `ae` namespace, is not reopenable from the File menu,
   and is not written beside the bundle as `import-report.json`. The reason-level filter
   §9 asks for (disabled expressions as their own list) belongs with that work; the built
   filter is by outcome.

**The direct `.aep` parser (docs/impl/ae-import.md §7) - phases A, B and C all
landed 2026-08-21; what is left is depth, not surface.** `crates/lumit-import/src/aep/`
reads an After Effects project file itself and fills the same `Capture` the Bridge
writes, so the mapping, the effect table and the report are shared unchanged: `rifx.rs`
is the bounds-checked container walk, `enums.rs` the funnel tables, `mod.rs` the
structure decode and `open_aep`, `props.rs` the property system.
`tests/aep_differential.rs` compares every recovered field against
`fixture.lum-bundle/capture.json` — AE's own account of the same file — and its
**exemption list is part of what it asserts**, which is what makes the items below
readable off the test rather than off this file.
 - **Corpus testing is owed.** One fixture from one After Effects version proves the
   offsets it contains and nothing about the ones it does not. Real community project
   files across several AE versions, run through the parser looking for panics, refusals
   and empty imports, is what turns "measured on one file" into "measured".
 - **One doc debt the phases left behind.** An effect **parameter name** now has a CI
   assertion but an effect parameter *value* in DOM units is asserted only through the
   shared value sweep; that is enough today and worth naming if the units table grows.
 - **Two encodings are still owed**: a text document (`btds`) and a gradient (`GCst`).
   The text document arrives carrying its match name and a note saying the encoding is
   not decoded, so the report already says so. **The gradient is unmeasured**:
   `fixture.aep` holds no `GCst` chunk at all — the shape layer's gradient is at its
   default and the file stores only what is not — so nothing has been proved about it
   either way, and a fixture with a non-default gradient is owed before anything is
   claimed. Decoding both is still owed, alongside **decoding the arbitrary-data blobs** —
   the sixteen-point Curves target is reachable in principle now that the bytes are in
   hand, measured rather than promised — and **shape-layer and text depth**, which arrive
   named and marked rather than drawn.
 - **Property display names are not read, and may never be.** They are After Effects'
   own localised resources rather than data in the file (a property nobody renamed
   carries the `-_0_/-` sentinel), so 1,106 of the golden capture's names have no source
   in the project. The mapper falls back to the match name; effect parameters, effect
   instances and masks do get their real names — 83 of them, every one asserted equal to
   AE's own, so a drifted `pard` offset cannot hand a parameter its neighbour's name
   unnoticed. A name table for the other 1,106 would be a table of Adobe's English
   strings — a separate decision, not an oversight.
 - **The project-level `LIST EfdG` fallback is not read.** It carries every effect's
   parameter definitions and is what tells a real parameter from a topic heading when a
   layer's own `parT` is empty (Gaussian Blur's is). None of the fixture's effects needed
   it; an effect that does simply reads its slots as the plain numbers they are stored as.
 - **A mask path's linear speed is 1.0 per segment in the DOM**, and one sample cannot
   say whether that is a constant or a duration-derived number, so the differential
   exempts it rather than curve-fitting. Nothing downstream reads a linear side's speed.
   A fixture with an animated path over a different duration settles it.
 - **The rest of footage interpretation is not read, and needs a fixture that has some.**
   A footage item's **name**, **path**, **placeholder-ness** and **missing-at-save** flag
   are read — measured against a real production project and against the layouts
   `forticheprod/py-aep` documents, with synthetic byte fixtures in `aep/mod.rs`'s tests
   as the regression. Frame rate, alpha, fields, pulldown and loop are still unread:
   Lumit has no field for any of them, and `fixture.aep` is solids and comps with no file
   footage in it, so not one of those offsets could be checked against AE. One more
   sitting with real footage in the project unblocks the group, and the differential test
   asserts the fixture still has none so the exemption cannot rot.
 - **An effect on a layer that is not the comp's size is owed.** Both layers carrying
   effects in `fixture.aep` are 640 x 360 in a 640 x 360 comp, so the frame an effect's
   stored two-dimensional point is a fraction *of* could not be measured: the parser
   reads it against the layer (the format's own convention — the anchor point and the
   mask path are the only other normalised values and both are the layer's), which is
   what `an_effects_point_is_a_fraction_of_its_layer_not_of_the_composition` in
   `aep::props` pins. A sitting with a Transform effect on a precomp or solid of a
   different size than its comp settles it against After Effects itself.
 - **A dragged layer is owed too, and cannot be forged.** Every layer in `fixture.aep`
   starts at zero, so `ldta`'s start offset — what puts in and out points, keyframe times
   and a stretch's reach back on the comp's clock — is measured against AE at one value
   only, and the 50 % layer sitting at zero cannot tell stretch-about-the-start from
   stretch-about-the-origin. The fixture is authored *by* After Effects
   (`make-fixture.jsx` inside a running AE), so hand-written bytes would be this parser's
   guess compared against this parser: owed is a sitting with a layer dragged along the
   timeline and a second both dragged and stretched. Standing meanwhile:
   `a_layers_in_and_out_are_counted_from_its_own_start` and
   `a_stretched_layer_is_stretched_from_its_start` in `aep::tests`, which prove the parser
   reads the field it was handed and not that the field is where AE puts it. The
   differential test asserts every start is still zero, so the exemption cannot rot.
 - **A reflected layer's ends are one frame loose.** At −100% stretch AE reports its two
   ends 1/3000 s further out than the file's arithmetic gives, as if it reflects
   inclusive indices on an internal grid; with one sample the grid cannot be proved, so
   the differential test compares those two within a frame. A fixture with a second
   negative-stretch layer at a different frame rate settles it.
 - **Every funnel-table row the fixture does not exercise is `reference`, not proved**
   (marked as such in `enums.rs`): most blend modes, two matte types, `WIREFRAME`
   quality, three light types, two auto-orient modes, and the three non-Classic
   renderers. A fixture that uses them turns each into a measurement.

**AE effect parity (docs/impl/ae-effect-parity.md) - waves 1 and 2 both landed in
full, 2026-08-20..21, with one standing exclusion (no particle-world port).** All
eighteen Tier-A effects and all of Tier B, by owner's ruling; docs/11's seed table is
trued for all fifty, with no substitutes left in it. The limits each wave recorded
against its own effects, so they are not re-derived:
 - **A mask-path row names one mask, and three AE controls want a set** (docs/08
   §3.78-§3.79). Scribble, Stroke and Vegas' Mask/Path source are built and the import's
   substitutes are retired; what is still reported against the seam is AE's **All Masks**
   and **Stroke Sequentially**, and Scribble's two multi-mask Fill Types. All three want
   a row naming a *set* of masks — a small extension of `ParamKind::MaskPath`, which
   still carries one optional mask id, and a list rather than a slot in the carriage.
   Nobody has asked for it.
 - **A path drawing is capped at 512 straight pieces** (docs/08 §3.78). The geometry
   rides in a uniform (`MAX_PIECES` in `fx_pathdraw.wgsl`), exactly as Lightning's bolt
   does, and past the cap every consumer coarsens rather than drawing part of a shape:
   the hatch widens its spacing, the dots space out, the chain straightens. A storage
   buffer is the answer the day something wants tens of thousands of pieces; nothing
   does, so none was built.
 - **Lightning ships four of AE's eight types, and no Alpha Obstacle** (docs/08 §3.74).
   Breaking, Bouncey, Anywhere and Vertical map to the nearest of Direction, Strike, Omni
   and Two-way strike and are reported; Alpha Obstacle asks the bolt to route around the
   layer's own alpha, which is a *search* rather than a formula and would change the
   effect's cost class. If it is ever wanted it wants a distance field of the alpha and a
   bolt built against it, both of which the host-side generator could do without touching
   the kernel.
 - **Beam has no 3D perspective** (docs/08 §3.73), for Card wipe's reason: AE's
   foreshortens the beam from a camera of its own, and Lumit keeps cameras on the
   composition (docs/06). The same composition-camera input that would give Card wipe its
   grid would give Beam this; `ParamKind` still has no camera row.
 - **Radio waves ships one Stroke width where AE tapers from a start to an end**, and
   only its Polygon wave type (docs/08 §3.75). A taper needs the *age* to reach the
   stroke's width, which it already does for the fade — so it is a cheap addition
   whenever somebody wants it. Image Contours is Vegas, and so is Mask now (its Mask/Path
   source) — both are reported as suggestions rather than built into Radio waves itself.
 - **Vegas' Segment length is a length, not a count** (docs/08 §3.76). AE traces the
   contour into a path and can therefore count segments *around* it. **On the Mask/Path
   source this is fixed**: there the dashes are spaced by measured distance round the
   mask, so they stay even however hard it curves, and the import converts AE's Segments
   exactly. It is only the contour half that still drifts in phase on a curve, because it
   still never traces one — the machinery that would let it is now sitting next door.
 - **Card wipe has no camera, no back layer, and no Card Scale** (docs/08 §3.72). Each
   card is projected in its own local frame at a fixed viewing distance, because Lumit
   keeps cameras on the composition (docs/06) and has none on an effect. If effects ever
   get a composition-camera input, the grid could be projected through it and AE's Camera
   Position / Corner Pins / Composite Camera would stop being reported. A back layer
   would need a second layer row, which §3.68's test says a card wipe can justify.
 - **Card wipe's Flip order has no Gradient entry** (docs/08 §3.72). AE reads that order
   from a gradient *layer*; Lumit's one layer row is the universal Matte, and a card wipe
   wants to say "only over the sky" as well as "in this order". A Gradient order can
   arrive later on a row of its own without moving anything. Randomness plus Seed covers
   the intent meanwhile. The import cannot read the spread — the capture carries the
   gradient layer's *index*, not its pixels — so an instance using Gradient imports as
   Left to right on AE's own Timing Randomness, and both are reported.
 - **Median's Radius is capped at 3 and cannot be typed past** (docs/08 §3.64), the only
   control in the catalogue for which that is true. The cost is the fourth power of the
   radius, so a larger window needs a different algorithm — a per-tile histogram, or a
   separable approximation that is no longer a median — and either is its own programme
   with its own oracle. The import writes 3 and reports the instance as approximated.
 - **Texturize's Placement cannot honour AE's *native-size* Tile and Centre** (docs/08
   §3.68). The layer carriage renders a referenced layer at this raster, so the texture
   arrives frame-shaped and Scale is what says how big one copy is. If `ParamKind::Layer`
   ever carries its source's own dimensions alongside the texture, the three Placements
   could use them and the import would stop approximating the size.
 - **The Stylise II proof renders on the CPU, and the fixtures are gradients.** Median,
   Find edges and Emboss are the first effects whose picture cannot be judged on the
   smooth clips in `C:/tmp/lumit-shots` at all, and the batch was judged on a screenshot
   instead. A fixture with real high-frequency detail in it — a resolution chart, a page
   of type — would serve every future edge-detecting or despeckling effect.
 - **Shadow highlight has no Auto amounts, and probably never should** (docs/08 §3.63).
   AE's is a whole-frame histogram reduction smoothed across neighbouring frames, which
   makes a grade whose answer at a frame depends on the shot around it. If it is ever
   wanted, it is a *scene analysis* feature with its own cache and its own doc, not a
   checkbox on this effect — and the import already reports it.
 - **Shadow highlight ships one Radius where AE ships two.** The second full-frame
   gaussian is real work for the softness of a mask; if a shot ever needs the shadows'
   mask measured at one scale and the highlights' at another, the kernel takes a second
   bound texture and the uniform grows one float.
 - **The old distort kernels still guard a texture fetch instead of clamping it**:
   `fx_mirror`, `fx_lensdistort`, `fx_dropshadow`, `fx_transform`, `fx_shake_mb` and the
   blur family (`fx_blur`, `fx_chanblur`, `fx_dirblur`, `fx_radialblur`) all carry the
   early-return form of `tap`, which the compiler may hoist above its own bounds check. A
   pixel whose four bilinear taps are *all* outside the frame can come back opaque
   instead of empty. Wave 2's kernels use the clamp-and-`select` form and Tile clamps
   outright, its edge policy being wrap; the rest want the same one-line change, and an
   oracle case that drives every tap outside at once so the fix is held.
 - **Bezier warp's twelve points want on-picture handles.** v1 ships them as twenty-four
   ordinary rows, four corners open and the eight tangents behind their edges' headings
   (docs/08 §3.55). Dragging a Bezier patch in the Viewer is the same overlay job Corner
   pin's four points want and should land with them; the stored form is AE's clockwise
   walk and survives the editor.
 - **Warp has no Warp Axis, and Wave warp no noise wave types.** Both are recorded skips
   (docs/08 §3.56, §3.54) that the import reports rather than approximates. The axis swap
   is six lines whenever someone misses it; the noise wave types are §3.37's field
   wearing a wave's clothes and probably never want building.
 - **Fractal noise is missing five AE controls**, all of them one more scalar through the
   same loop: Sub rotation, Sub offset, Perspective offset, Centre subscale, and the
   Overflow modes beyond Clip (docs/08 §3.37). None changes what the effect is; they land
   when a real project asks.

**Roadmap features not yet built.** Grouped by the phase they belong to in
[16-ROADMAP.md](16-ROADMAP.md). A pointer list, not a re-statement of the roadmap; a
line goes the moment its subsystem ships.

- **Media engine ([05-ARCHITECTURE.md](05-ARCHITECTURE.md) §6).** The one-copy
    D3D11→DX12 interop — hardware decode runs on the card's fixed-function unit and
    then transfers the finished picture back through ordinary memory
    (`lumit-media/src/decode.rs`) — and VideoToolbox, which lands with the macOS pass;
    ProRes/DNxHR intermediate export (v1 writes H.264, HEVC and EXR).
- **Audio** ([07-UI-SPEC.md](07-UI-SPEC.md) §10, [09-AUDIO.md](09-AUDIO.md)): persistent
    waveform peak files — `lumit-bridge/src/peaks.rs` builds the multi-zoom pyramid on
    demand and keeps it for the session, never writing it to the project sidecar, so it
    is rebuilt next time the project opens; §3.4's scrub-audition grain and its Timeline
    toggle (a scrub moves the audio clock and plays nothing); and §5's replace-or-merge
    offer on a re-run (detection replaces).
- **File format ([10-FILE-FORMAT.md](10-FILE-FORMAT.md)).** Embedded `thumbs/` previews
    in the `.lum`; the per-project sidecar `proxies/`, `peaks/` and `flow/` directories
    (`frames/`, `track/`, `roto/` and the global media index exist).
- **Design ([15-DESIGN.md](15-DESIGN.md)).** The missing type-scale steps in the theme
    struct, and identity colour tokens for Shape and Null layers (§6.1 reserves the
    values; `LayerColours` carries six kinds and both borrow). Font bundling is done -
    Hanken Grotesk and Geist Mono ship in `flutter_ui/assets/fonts/`.
- **Platform.** The macOS pass — native menu bar, VideoToolbox, ProRes; it also owes
    `application:openFile:` (a double-clicked `.lum` opening). The Metal/IOSurface Viewer
    path is unverified on real hardware. Developer ID signing and notarisation landed but
    have never run — the repository carries no tag yet, so the first one is their first
    execution, and a pre-release tag is the way to rehearse it. Signing the Windows
    installer is still blocked on buying a certificate, so the installer ships unsigned
    and SmartScreen still warns.
- **Phase 2 - Retime.** Automatic beat snapping across edit/retime points
    ([04-RETIMING.md](04-RETIMING.md) §12's `retime.quantise_boundaries_to_beats`,
    [09-AUDIO.md](09-AUDIO.md)): `lumit_core::markers::snap_time` is written and tested
    and nothing calls it, so no drag snaps to a beat yet.
- **Phase 3 - The look.** Per-layer motion blur polish
    ([08-EFFECTS.md](08-EFFECTS.md)); importing a preset file from outside the presets
    folder is still a manual copy (`list_presets` reads `presets_dir()` and nowhere
    else). A **tone mapping effect** belongs here too (owner, 2026-08-06): the grade that
    actually lands in the export, distinct from the Viewer's preview-only toggle that
    ships today, and it wants [08-EFFECTS.md](08-EFFECTS.md) §3 to gain its entry and a
    curve chosen once, which both then share. This gate is the v1.0 milestone.
- **Phase 4 - Extensibility** ([12-PLUGINS.md](12-PLUGINS.md)). The LFX C ABI and its
    validator: there is no `lumit-lfx` crate, only the out-of-process sandbox and IPC
    substrate `lumit-ofx`/`lumit-ofx-broker` built for OFX that it is meant to share.
    Lottie import ([11-AE-IMPORT.md](11-AE-IMPORT.md) §8) is the other half of this
    phase still unbuilt.
- **Phase 5 - AE parity march.** The **stabiliser** (docs/08 §7's Stabiliser row,
    flow-engine-backed smoothing of unwanted camera motion, warp-stabiliser class) is the
    only member of this phase with nothing behind it.
- **Phase 6 - Beyond parity.** Blender scene import, Lottie export, OpenTimelineIO
    interchange, render-farm/CLI export.

**Tracking (docs/impl/tracking.md) - all four phases landed
(2026-08-20, 2026-08-21).** `crates/lumit-track` holds the track substrate —
Shi-Tomasi detection on a 16x16 bucket grid, pyramidal affine KLT with
forward-backward and NCC verification, exclusion masks, re-detection into
starved buckets — the two-view geometry over it (Hartley-normalised 8-point and
7-point fundamentals inside LO-RANSAC, the GRIC gate that calls a pan a pan,
parallax-driven keyframe selection, epipolar dynamic-track segmentation, the
zoom cut/ramp detector), the planar homography tracker, and the global solve:
`solve_camera` returns a `CameraSolve` with a pose per frame, a focal curve per
segment, the point cloud and the per-frame error. Sixty-seven tests, all
synthetic, no assets.
 - **A nodal-pan product.** `SolveError::RotationOnly` refuses a shot with no baseline,
   and that refusal is right for a *camera* solve — but the rotations are recoverable and
   a Camera layer that only turns is a real deliverable for a locked-off pan. It needs
   its own output shape and its own decision entry.
 - **Nothing the solve notices crosses the bridge.** `SolveNote` — `ZoomRamp`,
   `FocalGuessed`, `ColinearBaselines`, `InterpolatedFrames`, `DisconnectedKeyframes` —
   is produced and correct and is read nowhere outside `lumit-track`, so the panel cannot
   say "the lens moved during this shot" or "the focal was guessed". The same seam owes
   the row that lets the operator type a focal: `SolveSettings::focal_px` is the lever
   and `api/track.rs` has nothing that sets it.
 - **A multi-frame rack can never be a `Cut`** (the note's Open questions). `detect_zoom`
   reserves `ZoomKind::Cut` for an isolated hot pair, so a lens rack over several frames
   is always a `Ramp` — which is the honest answer as long as a ramp's focal is a curve,
   and it now is. Named so the asymmetry is not rediscovered as a bug.
 - **Lens distortion (k1/k2) is not solved.** The note's camera model allows an optional
   pair per segment; the solve fixes the principal point at centre and solves focal
   alone. Two more columns in the same bundle, and the same `ponytail:` ceiling applies.
 - **The Shi-Tomasi response map is a whole-frame pass and dominates when re-detection
   runs** — 24.4 ms/frame against 11.0 with re-detection off, on 100 features over
   640x360 (the note's measured number). `response_map_into` still sums the gradient
   normal matrix over a `(2r+1)²` window per pixel; those box sums are separable, that is
   the cheap win, and it comes before any WGSL port.

**From the Caddis study, parked by the owner (2026-08-25):**
- **B8 - Choke, Inner glow, Inner shadow** as catalogue effects.
- **B9 - Slitscan, Dither, Draw Glass**; and aperture-shape upgrades beyond DOF's.
- **B4 - a details inspector** (read-only per-layer engine state: buffer size, format,
  colour space, cache tier). The bottom strip already reads the cache; the unique
  value is per-layer format/colour-space once OCIO debugging is routine - build it
  then, as a section of the existing Source card rather than a new panel.

**The Hierarchy panel's graph view** (6.46, 7.24): deferred to the tail on the
owner's word. The panel has left the default workspaces; the graph view and the
indent/graph switch wait until something needs them, and 7.24's doc note waits with it.

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
    command-buffer batching that was the real win, and `lumit-gpu` has carried it
    since. Anyone reaching for the pool again should re-run the stopwatch first:
    if the processor half has not grown, this entry still stands.
- **The workspace strip ticks nothing after a restart.** `Workspace.activePreset`
    is session-only on purpose: what persists is the arrangement, which the user
    is free to drag about, so a ticked preset could claim a layout the panels no
    longer match (`state/workspace.dart`).
- **Re-time the flare after its two correctness fixes.** The tent now reaches a full grid
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
    general and the flare's render pass does not yet offer. Named so it is not
    rediscovered as the cache-key bug an earlier fix closed.
- **No progress for the idle cache fill** - it is not a frame anyone is waiting
    for, so the bar stays quiet for it.
- The two recorded behavioural deviations - export queue-snapshot timing (an
    export renders the document as it stood when it was queued, so later edits
    never alter what a queued export writes) and the share-export VBR cap
    (docs/06 §7.5's preset table, YouTube's own bands). Both are recorded in
    [archive/flutter-port/06-REMAINING-WORK.md](archive/flutter-port/06-REMAINING-WORK.md).
