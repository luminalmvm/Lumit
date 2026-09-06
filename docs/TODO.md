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

All seven waves landed and their per-wave summaries were pruned as they went; git log
is the record. What the programme's own runs
surfaced and deliberately left open, so it is not re-derived:

- **The physical Lens flare renders nothing on a headless run** - bit-identical to the
    plate at defaults AND with a bright area light placed on the frame. The manual's
    flare page has no figure because of it. Smells adjacent to the MSAA/fp16-additive
    history; wants its own investigation with a non-empty-output assertion.
- **Shots sweep 4 crashes natively about one run in three** ("Lost connection to
    device", no Dart exception, different point each run). All its pictures were
    gathered across runs; the sweep is unreliable and the crash is an engine bug in
    hiding.
- **Six screenshots the manual still wants** (pages ship without them, no placeholders):
    camera-track.png, planar-track.png, project-settings-colour.png,
    viewer-colour-menu.png, text-animators.png, shape-combine.png.
- **Clone to points / Trail / Connect points have no example pictures by nature** -
    the effect-example harness stages one effect on one layer and their stream arrives
    on a wire. An example route through a two-effect stage would fix all three.
- **The audio device-change stream** (a device vanishing mid-playback rebuilds on the
    next open, not live) - the remainder recorded when the Audio settings landed.
- Crowdin at the next push owes: the pre-programme ~360 keys, 53 safe-lane keys, every
    FP key (listed per commit), 63 changed tooltip values, tipBrushPressure, and the
    unused settingsHelpChromeLabels to cull.
- **The Expression driver, deferred from 0.3.0** (owner, 2026-09-01). A driver box whose
    output is what an expression returns. The groundwork landed with the decision to defer
    it, because both halves stand on their own: `expression::evaluate_value` hands back a
    number, a point or a colour - or Rhai's own sentence about why not, where `evaluate`
    turns every refusal into -1 - and `DriverCx` now carries the expression context, as
    `ResolveCx` always has.
    What is left is the box itself, and one decision the owner has not made: its output
    type cannot follow the expression's result, because a port is a fact about the
    catalogue entry rather than the node (one shared `WiggleDef` serves every Wiggle) and
    nothing revalidates a wire when `extra` changes, so editing the text after wiring would
    strand a mismatched one. The recommendation put to the owner and not yet answered:
    four static outputs - Value, Colour, Point x, Point y - with the result deciding which
    carry a value each frame, the shape Split already has. All four labels are in
    fx-labels.txt already. The text belongs in `extra["expression"]["source"]`, as Custom
    shader's does.
- **Syntax highlighting in the shader editor** (Airyz, 2026-09-01: "u can reuse the
    syntax highlighting logic from the expression editor"). Held back from 0.3.0, and not
    for the reason it looks like. `ExpressionTextEditingController` is forty lines over the
    `syntax_highlight` package and takes a language in one string, so pointing it at Rust
    (the nearest grammar shipped; there is no WGSL one) is three edits. What stops it is
    what the package paints unconditionally: six hardcoded bracket colours and a
    `Color(0xFFff0000)` for an unmatched one, with no API to turn them off - hex in the
    frontend is a defect, and the token colours are VS Code's two default themes,
    so six of Lumit's eight schemes would get somebody else's palette. The honest road is
    `Highlighter.addLanguage`, which takes a TextMate grammar as a plain string (no asset,
    no `pub get` - which matters, since pub-cache writes from this machine land in the MSIX
    overlay), plus a theme-driven palette. Also required either way: the controller's
    `buildTextSpan` discards the style it is handed, which would drop the shader well's
    `height: 1.4` and drift the line numbers out of step with the code.
- **A shader could be a project item** (Airyz, 2026-09-01): "i'd like if i could load a
    file in the project view and reference that instead, as it would give the same options
    for 'find missing footage' and stuff to swap the file to something else. It would also
    follow the same settings for relative/absolute file paths". Today Load from file copies
    the text into the instance (`extra["shader"]["source"]`) and the path is a
    memory rather than a link. Making it an item means a project-item kind that is not
    footage, the relink road reaching it, and a decision about what a missing shader
    renders - which is a decision-sized change, not an editor one.
- **The Project panel is not virtualised**, which docs/13-PERFORMANCE-RULES.md §5 names
    it in: `ListView(children: rows)` builds every row of the open tree whatever the
    viewport holds, with no `itemExtent` on a fixed-height list. The click and the probe
    storm behind the owner's "clicking a folder feels very slow" are fixed (the name
    cache and the coalesced rebuild, 2026-09-01); this is what is left, and it is the
    scroll half. `LazyBlocks` in timeline_metrics_frb.dart is the machinery to reuse.
    Note `_visibleIds` must keep the whole filtered tree - Ctrl+A reads it.
- **A click still rebuilds the whole Project panel.** Selection is `setState` on the
    panel, where the Timeline publishes it as row state instead (f137662f). Two rows
    change; every row rebuilds. Cheap now that the names are cached, and the next thing
    to do if the panel still feels heavy on a large project.
- **An undecodable file imports silently.** The dialogue no longer hides formats
    (the filter lists what the engine reads, with All files beside it), so what is left
    is the answer: a file FFmpeg cannot open becomes a footage item with no picture and
    no reason. `thumbnail` returns `None` for undecodable and missing alike, and only
    missing wears a badge. Testers reading that as "it would not import" is what raised
    it. Wants the probe's error carried to the row and said in one line.
- **Some languages went backwards when the Crowdin branch merged.** Its five `.arb`s
    lost to the site tool's own output, which is newer and much fuller, but it had keys
    theirs do not: 152 in Kazakh, 134 in Ukrainian, 42 in German, 37 in each Chinese.
    Those keys are simply untranslated again, so the translation page already lists them
    for whoever wants them - nothing to do here. The community's earlier words for them
    are in c5845a78 if anyone wants to compare.
- **The flare's raster still draws the cells it culled.** A batch now
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
    the deleted read-back transport - the Viewer receives a GPU handle
    and nothing else.
- **Playback's remaining bridge chatter scales with rows on screen** - one
    `sample_scalar` per animated row plus one `time_of_frame`. Batch per frame if
    it ever bites, the way `time_of_frame` already was.
    (`bridge_call_budget_test.dart` is the gate.)

---

## Now - the interface answers at 60/120 (docs/impl/ui-performance.md)

The owner's interface mandate is measured and has an answer. The note is binding: the
gesture table taken in the owner's own conditions (window maximised, live preview —
the small-window empty-preview test trap flatters by 4×), the architecture, and six
ordered work packages, each gated by a probe re-run in those conditions:

- **WP-1 - LANDED, then reversed on shipping**: the unmet gap is the
    Windows embedder's own (4x MSAA whole-window resolve, no partial repaint - ~8
    ms/megapixel) and WP-7 in the note pursues it (the drafted upstream issue, the
    local-engine MSAA test); the runner meanwhile pins Skia.
- **Per Flutter upgrade: re-run the §2.4 backend A/B** in the owner's conditions
    (docs/impl/ui-performance.md §2.1/§6, one run each backend) and flip
    `ImpellerSwitch::Disabled` back to `Default` in
    `flutter_ui/windows/runner/main.cpp` the day Impeller clears the 60 fps mandate.
    Standing, not one-off - delete only when the flip lands.
- **WP-2 — the select click**: layer selection becomes listenable row state; a
    first-visit click's worst build < 8.3 ms (was 39–67 ms).
- **WP-3 — incremental scroll**: `LazyBlocks` reuses identical children across window
    slides + a per-block `RepaintBoundary`. The "8–12 fps in every condition" that
    outlived it was the probe grinding the scroll's stops plus its fps arithmetic
    (note §2.6): on the pinned backend every notch answers inside the 16.6 ms
    floor.
- **WP-4 — zero per-frame document calls during drags**: `animated_mask_paths_at` and
    `time_of_frame` off the scrub path (~1.3 ms a frame today).
- **WP-5 — the edit storm becomes one wave**: the per-revision walks stop being
    per-item sync calls (a switch click today: ~228 calls, 0.7–0.9 s to settled).
- **WP-6 — the repaint matrix becomes CI gates** in `rebuild_budget_test.dart`.

## Now - the effect registry (docs/impl/effect-registry.md §6)

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
    with the size. Before the migration neither moved, so the phase was right and the period wrong;
    now it is the other way about. The Lens flare's `derived.light*` entries are the same
    shape - raster pixels under a derived id - though there the old `rescale_px` match did
    not move them either, so a Lights-mode flare on a resized precomp is no worse than it
    was and no better. Two fixes, both small: give `EffectDef` a
    `derived_spatial()` list the rescale pass consults (keeps the resolve maths bit-identical),
    or derive the roll in *periods* rather than pixels and multiply in `packed()` (no new API,
    but the f64 product rounds once more and so is not bit-identical to the old arm).

- **The effect manual is five pages behind the catalogue.** The catalogue stands at 90 and
    `web-docs/src/content/docs/effects/` holds 85 pages: the Controls category has no
    index and no five pages, and the four wipes' Completion still prints as `float` where
    the schema now says `slider`. `npm run docs:effects` from `web-docs/` creates and
    refreshes all of it inside the `GENERATED` markers; the five new pages then want their
    protected prose written by hand. Nothing in CI gates the manual, which is exactly why
    it is written down here.

---

## Now - Flutter frontend parity and regressions

Flutter is the only frontend; git history is the parity reference.
These are v1-scope surfaces it does not yet match.

**Timeline outline ([07-UI-SPEC.md](07-UI-SPEC.md) §4.2):**
- **A switch cell on a locked layer throws.** `lock_guards` refuses every switch
    but the lock, shy and the label (`lumit-core/src/ops.rs`), and the outline's
    cells call `set_switch` unguarded — so clicking the eye, solo, fx, motion
    blur, 3D or guide on a locked row raises out of a tap handler instead of
    saying no. Pre-existing, and it wants one answer for all six cells rather
    than a guard per cell: either the cells stand down while the row is locked,
    or the refusal becomes a status-line notice.

**Viewer bars ([07-UI-SPEC.md](07-UI-SPEC.md) §2.2):**
- The wireframe/overlay menu's own *separation* (§2.2 item 5) — the view menu
    now carries the layer-controls switch, which turns wireframes, handles and hover
    highlight on and off as one; separating those from motion paths, mask paths and
    gizmo visibility, and the full wireframe display mode, is owed.
- **Rulers, draggable guides and snapping-to-guides** (§2.2 item 6). The menu, the
    grid and the title/action safe areas are built; these three land as further
    entries in the same menu, which is why it is a menu.
- **A comp's overlays are session-only**: the grid and safe-area flags ride
    `LumitUiState` keyed by comp and are forgotten when Lumit closes. They belong in
    the per-project session beside the preview resolution and the region of interest.
- **A zoomed-in snapshot is the panel's worth of detail, not the picture's**
    (§2.2 item 14). The photograph is capped at the panel's resolution so
    pressing Take at 400 % cannot ask for a few hundred million pixels; the
    upgrade is to photograph the *visible region* instead of the whole picture,
    which keeps full detail and wants the boundary moved rather than a number
    changed.
- **Degradation names a tier, not the steps it skipped** (§2.2 item 9). The bar's
    reading says the pixel count a frame was made at, which is the tier; §2.2 also asks
    that the indicator name what was degraded ("glow skipped"), and nothing reports that
    across the bridge yet.
- **Tone mapping's explanation lives in the colour picker's tooltip** and nowhere
    else on screen (owner, 2026-08-08). The row itself is a name, because §13.2 keeps
    a control's label to that; the picker is a readout, which §13.2 does allow a
    sentence, so that is where "what this does" went. If hints of this kind ever get a
    home of their own, this is still a candidate to move.

**Toolbar tools ([07-UI-SPEC.md](07-UI-SPEC.md) §1.7):** what is armed is a
*tool*; what each tool then does is the backlog.
- **Razor** - a Sequence layer's eased ramps refuse a cut (`UncuttableClip`).
- **Shape layers** ([impl/shape-layers.md](impl/shape-layers.md)) - trim paths,
    dashed strokes, the repeater, offset paths and gradient fills have landed.
    Owed: nested groups, the **wiggle** modifier, gradient **stop lists** (there
    are two stops), joins and caps other than round, and animated paths.
- **Path editing on the picture** - mask and shape-layer points drag. Still
    owed: a **paint stroke's** points, which are a stored gesture
    rather than a path and so are their own piece of work; no path's bezier
    **handles** can be dragged, so the `Alt`-drag that re-links a broken tangent
    pair exists only while a point is being *placed* - and the model has no
    linked/broken flag, so adding one is a
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
- **Type** - vertical type (needs `lumit-text` to lay a line downwards); true
    glyph metrics across the bridge (the caret, the anchor and the gizmo all use
    the same half-an-em estimate, and one measured advance width would replace
    all three); multiple lines and a character panel (font, tracking, leading,
    alignment - the document is one styled run, [03-DATA-MODEL.md](03-DATA-MODEL.md)
    §9.1); per-character and per-word animators.
- **Paint** (brush/clone stamp/eraser, [impl/paint.md](impl/paint.md)) - owed:
    **pressure and tilt** from a tablet, **spacing** and **scatter**; a keyed
    Start/End's **curve in the graph editor** (the Timeline lane draws and drags
    its diamonds, but `graphChannels` walks transform, effect and mask paths only);
    painting in **Layer view** rather than on the composite; and **a GPU
    stamping path** (the rasteriser is a CPU loop beside the mask one, and it
    changes the rasteriser, not the stored stroke - it is also what would retire
    the 8-bit read-back a painted Precomp pays).
- **Camera** - a separate point of interest (AE's two-node camera) is an engine
    change; the Unified Camera tool; depth-of-field handles on the picture; a
    keyframed camera cannot be dragged (no single value to add to); a drag
    spanning several layers is one undo step per layer, because no op carries
    edits to more than one.
- **Roto propagation speed** - the tools shipped (geodesic solve,
    flow propagation, guided Refine edge, the `roto/` sidecar tier, both tools
    armed), but a propagated 1080p frame measures 895 ms against §7's 60 ms
    target - the WGSL ports [impl/roto.md](impl/roto.md) names are owed.
    Puppet shipped whole; its recorded upgrades (GPU warp,
    sparse factorisation) have fired triggers and wait on the same quiet day.
**Smooth zooming everywhere else.** The shared helper is built
(`widgets/smooth_zoom.dart`) and the Timeline reads it. Still cutting
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
    changes, rather than patched.

**Retime follow-up after the property-path move.** **The eased ramp shapes are
gone from clips** — `Clip::with_ramp` takes two speeds and runs straight between
them, which is what the envelope authors. Slow/Fast/Smooth/Sharp come back with the
preset-shelf rework below, rebuilt on the property like everything else the move
carried.

**Video memory is only read on Windows.** `video_memory_bytes` answers the
first DXGI adapter's dedicated memory there and 0 everywhere else, so the GPU
cache ceiling falls back to the frontend's documented figure on macOS and
Linux. Wants Metal's `recommendedMaxWorkingSetSize` and the Vulkan adapter's
device-local heap.

**Appearance.** The seven built-in schemes still restate every colour
individually; only the two Timeline tokens default from the mode. Still owed:
a swatch strip per row **inside** the picker's menu (it previews the
selection only), and a place to keep themes other than the workspace file, so an
imported theme travels with the user rather than the machine's settings.

**Shell and onboarding:**
- **The boot splash says only what `boot_log` says.** It is mounted now
    (`BootGate` in main.dart) and streams the engine's own boot log, which is
    all the engine can tell it: there is no notice stream to subscribe to, so a
    module that took a long time coming up, or came up degraded, cannot say so
    on the splash. Wants an engine-side boot event stream before it can.
- **Workspace machinery beyond the presets** ([07-UI-SPEC.md](07-UI-SPEC.md)
    §1.6) - user workspaces (save-as/rename/export), the chrome switcher strip,
    and Alt+Shift+1-9.
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
    deleting a *whole lane selection* is not built (the graph view has both).
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
    during playback to tap a beat, and the bare digits went to the numbered
    markers. Needs its own chord or a modal reading.
- **Edge-follow has one manner, not two** - the lanes flip a page when the
    playhead leaves the viewport (TI-9); [07-UI-SPEC.md](07-UI-SPEC.md) §4.6 also
    wants a *smooth* follow and a setting to choose between them. `Shift+=` (zoom
    to the work area) is unbound for the same reason: neither was in TI-9's own
    sentence list.
- **Volume keyframes draw no lane diamonds and no graph curve** - volume is not
    in the comp read model; fold it into `BridgeLayerInfo` if either matters.

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

The 2026-08-23 redesign lands in four phases, in this order; 15-DESIGN §12A and the
approved mockups govern the layouts. Phase 1 is Now; each later phase becomes Now when
the one before it lands. Multi-window is a fifth phase that waits on Flutter and
does not gate the four. Delete each phase here when it lands, as with everything else.

- **Phase 1 - theme groundwork**: the `animated` token and the
    three-greys-at-rest audit; Hanken Grotesk and Geist Mono bundled (replacing the
    old bundle-the-household-fonts item under *Later - Design*); the icon set drawn
    on its 16 px grammar and embedded as Flutter icons. **The Chrome labels setting is
    built and consuming**: the three-way setting ships as Icons, is read by the
    Timeline's column toggles, and carries its word in every tooltip. What is left is
    converting the rest of the chrome a surface at a time.
- **Phase 2 - panels and windows**: the effect controls
    (fixed columns, square stopwatch, reserved keyframe-nav slot, linked vector
    wells, the crosshair point picker, Mix row with blend mode and matte channel,
    Matte row with invert), the Timeline (Layers/Graph modes, the Animated filter,
    full-width composition tabs, the double-height ruler with its end padding, the
    tier-coloured cache bar, the work-area band, trimmed-extent outlines) and Graph
    mode's surface (edge-to-edge curves, the fixed value gutter, the Value/Speed strip
    - the filtered colour-ticked outline is withdrawn: the graph shows the Layers
    outline), the Project panel, the Viewer bar,
    Settings, the welcome screen, and **the export dialog plus the export queue**.
    Every dialog is built **in-window**, as an ordinary `showDialog` overlay - that
    is the migration prep, not a stopgap: when windowing ships, `showDialog` becomes
    a real child window with no per-dialog rewrite (docs/impl/multi-window.md §5).
    This phase owns, rather than tracking separately: the export dialog and queue
    rework, the easing and graph-surface rework, the switch-column-at-minimum-width
    polish, the editable-value colour treatment (decided in phase 1, built here),
    and the rest of the redesign's visual polish list.
- ~~**The timeline interaction programme**~~ - **landed complete
    2026-08-25**, all ten packages, and verified sentence by sentence against
    **docs/impl/timeline-interaction.md**, which stays as the binding spec for the
    panel's behaviour. What it left deferred is listed above (edge-follow's second
    manner) and in the note's own §8: the razor's scissors pointer, and the scrub
    ladder on the clock.
- **Phase 3 - the node graph and the Nodes workspace**: the Graph panel as a
    second view of the effect stack that can also wire effects together, auto-wire
    and heal toggles, type-coloured wires from `viz_*`-family tokens, the Nodes
    workspace with the small viewer and short timeline, and the picture at a node
    (folded into the Viewer's own "at effect" chip). The
    design step has answered the document-model question: the
    stack stays the spine and each layer gains an additive driver graph -
    **docs/impl/node-graph.md** holds the model and the six ordered work packages
    (engine model, bridge, Graph panel, Nodes workspace, the picture at a node,
    points stream + Particulate design doc). ~~WP6, the Particulate design document~~ —
    landed as **docs/impl/particulate.md**; the design's decisions have since been
    confirmed DECIDED by the owner's commission, below.
    ~~WP1, the engine model and evaluation~~ — **landed 2026-08-24**:
    `LayerGraph` on the layer (drivers, typed edges, canvas positions) with
    `SetLayerGraph` and its refusals, the `Signature` split in the registry, the
    complete `PortType` (Points included), the six v1 drivers, driver resolve as
    parameter evaluation, `SourceMatte`, and the graph folded into the frame key.
    Old projects load to an empty graph and re-save byte for byte.
    ~~WP2, the bridge~~ — **landed 2026-08-24**: `LayerReference::get_graph`
    (the whole structure in one call), `get_graph_drivers`, `new_driver`,
    `set_graph` (one `SetLayerGraph`, one undo step), `list_drivers`,
    `BridgePortType`, the refusals as calm sentences, and the driver property
    path spelled `<layer>/graph/<node>/<param>` (docs/17).
    ~~WP3, the Graph panel~~ — **landed 2026-08-24**: the panel to its drawing,
    the wire colours as the theme's `PortColours`, and the *driven* state on an
    Effect-controls row. Its three gaps closed 2026-08-24:
    `Op::SetLayerEffects` prunes the edges, positions and badges naming a
    removed effect (so deleting a wired box is one op), and
    `BridgeEffectInfo` carries an entry's declared ports — which folds the
    auto-wire into the add's own commit and lets the Tab search offer only the
    entries a dragged wire could land on.
    ~~WP4, the Nodes workspace~~ — **landed 2026-08-24**: the preset (the Graph
    panel large with the ordinary Timeline short beneath it, the small Viewer
    and the new **Node panel** down the right), its tab on the workspace strip,
    and the graph's pick carried through the shell so the Node panel follows
    it. Its gap closed 2026-08-24:
    `render_frame_with_driver_preview` stages the graph's nodes as the stack
    preview stages the effect list, so a driver's number moves the picture
    while it is dragged rather than only on release.
    ~~WP5, the picture at a node~~ — **landed 2026-08-24** as its own panel,
    **folded into the Viewer 2026-08-25**: selecting an effect
    on the graph or in the Effect controls stack offers an "at effect" chip over
    the Viewer's picture, and the panel, its seam and its 256px thumbnail are
    gone. WP1's named gap is closed:
    the `AudioTap` is wired (`lumit_render::audio_tap`), so Audio level reads
    the referenced layer's own footage at a fixed rate, identically in the
    preview and the export — the parity matrix carries an audio-driven row.
- **The points-stream programme** (the owner's commission): Particulate and the
    points stream move from design to implementation.
    **docs/impl/points-stream.md** is the binding plan — the `EffectData` wire (a
    points connection is a graph edge, the first stack-sourced data wire), the
    Points sample driver (Count and Nearest distance driving parameters), the
    evaluation and carriage contract, the seam, and the ordered work packages:
    ~~PS1~~ (landed: `fx/points.rs`, the `Signature::Image { extra }` split and
    Particulate's declaration, closed forms and CPU disc reference),
    ~~PS2~~ (landed: the four GPU passes — count, scan, place, instanced draw —
    the three render modes, the schedule's carriage beside the op, docs/08 §3.86,
    docs/13's B12–B14, the rotation-jitter dial and the stream's real agreement
    bound; the effect draws),
    ~~PS3~~ (landed: `OutputRef::EffectData`, the three new refusals, the reorder
    heal, the seam's half), ~~PS4~~ (landed: the Points sample driver, the walk
    re-entering itself through the effect stack), ~~PS5~~ (landed with PS3-PS4:
    the seam crossed with no codegen left owing),
    ~~PS6~~ (landed: the live teal wire, Points sample's rows, Particulate's
    surface verified, and the no-stream mark),
    ~~PS7~~ (landed: `particulate-golden.txt` and its two gates, B12-B14 as
    `lumit-bench` scenarios with the floor subtracted, the export walk's
    undegraded field pinned, and a driven value clamped to its parameter's hard
    range at the effect's socket).
    **The programme is complete.** docs/impl/particulate.md remains the effect's
    own design; the family landed whole: Grid, Scatter, Emit from image,
    Clone to points, Trail, Connect points and the cross-layer tap.
- ~~**Phase 4 - the website**~~ - **landed 2026-08-24** (the `WebHero` drawing):
    lumitlab.com carries the application's own tokens -
    the three greys, the four text tiers, the two hairlines, clay as the only accent -
    and its two faces, **Hanken Grotesk and Geist Mono**, which won the side-by-side
    against Geist on the two things that decided it: the site and the application then
    read as one product, and it ships as one variable latin+ext file where Geist ships
    static latin-only weights. The wordmark is top-left in the bar and its animation is
    the hero, a band of half the window (never more than the drawing's 520px) on the
    drawing's own three washes and grain. The download button names the visitor's
    platform and links straight at that platform's asset; **no platform is greyed**,
    because `release.yml` builds the Windows `.exe`, the macOS `.dmg` and the Linux
    `.flatpak` on every tag and every job gates the release, so all three exist
    whenever any of them does. Below the hero: the wide screenshot, the two captioned
    screenshots, and the three hover-play slots, every picture a real capture from the
    application (`web/public/shots/`, 124 KB the lot).
    One thing the drawing showed is deliberately **not** built: its closing tab strip
    (Animate / Composite / Retime / Export) is a control with nothing behind it, so it
    is omitted rather than shipped dead. The clips are **content debt**:
    - **Record `workspace.webm`, `retime.webm`, `flare.webm` and `camera.webm`** into
      `web/public/clips/` (see `web/README.md`). The slots are built and working - real
      `<video>` elements - and until the files exist each shows its poster, a crop of a
      real screenshot, and the wide slot drops its "plays on hover" label. Nothing fakes
      motion. `workspace.webm` is the wide slot under "Composite the way you know",
      which was a still until the owner asked for a video there; its poster is that
      same still. The other three are behind the Timeline / Graph / Nodes tabs, one
      open at a time, and the open one plays. The posters for the flare and the camera
      slot are stand-ins from the Viewer; a capture of each feature would be better and
      can replace them in place.
- **Later, gated - the Flutter multi-window upgrade.** Blocked
    upstream: windowing is main-channel-only, flagged, and its API promises breaking
    changes, so Lumit takes no production dependency on it until it reaches the
    stable channel un-flagged - re-check the status line in
    **docs/impl/multi-window.md** §1 before planning any of it. The phase opens with
    that note's cheap spike (§6 step 2): can a second window composite the engine's
    shared Viewer texture at all? Then the `WindowManager` root on the main window
    only, the welcome window, the dialogs (mostly free), the settings/theme/queue
    windows, and last the satellite tear-off panels - which is where the old
    pop-out-panel-windows rebuild item is folded in.

## Next - colour management: OCIO (docs/impl/ocio.md)

The owner has ruled OCIO support in scope; the design step has landed
(**docs/impl/ocio.md** holds the model, the maths, the traps and the test plans;
native-Rust hosting and the v1 scope are decided). Six work packages, in
order, each sized for one pull request, each landing with its tests; WP6's fixture format is
WP1's, so fixtures are authored alongside WP1–2 rather than at the end.

**WP1 and WP2 have landed**: `crates/lumit-colour` holds the op set, the samplers, the
bake, the `.spi1d`/`.spi3d`/CLF readers, the `config.ocio` grammar, resolution, the
interchange bridge and the refusal taxonomy, with its own test suite and
`tests/refusals/` as the taxonomy's corpus. The two things they could not finish —
the reference-library fixtures and the vendored `BuiltinTransform` bakes — landed with
WP6 on 2026-08-25, so both shipped ACES configs now resolve end to end.

**WP3 and WP4 have landed too.** `Document::colour` and the per-item tag carry their two
ops, `lumit-render::colour` loads and bakes with the degrade ladder and the frame-key
folding behind it, and the seam is open: `ProjectReference::{colour_summary,
set_colour_config, can_deliver_colour_space}`, `FootageReference::{colour_space,
set_colour_space}`, the view field on `set_viewer_look`, and
`CompositionReference::export_spec_check` (which replaced the free-standing one).
A refusal crosses as an id plus its facts and `colourProblem` in
`flutter_ui/lib/l10n/engine_labels.dart` writes the sentence.

**WP5 has landed**: the config is chosen in **File ▸ Project settings ▸ Colour** (path
well, *Choose…*, *Clear*, the state line, the fixed working-space reading), the Viewer's
colour picker grows a section per display with its views as rows and says calmly when a
named config is not in force, the export's colour dropdown lists the config's spaces
under their own heading with per-name enable off `can_deliver_colour_space`, and a
footage row's **Colour space** submenu assigns one. One thing it left owed, still owed:

- **The working-space reading has one sentence, not two** (note §2.1, §6.4): the Project
    settings row always says "Linear Rec. 709", because `BridgeColourSummary` carries no
    flag for a legacy config composing through its `scene_linear` role. It wants a field
    on the summary and a second sentence behind it.

**WP6 has landed, whole.** The CLF suite is real: eight documents from the Common LUT
Format specification's own example and implementation-test set, vendored byte for byte in
`crates/lumit-colour/tests/fixtures/clf/`, each gated against values that are
published rather than measured — and they found two reader faults the day they
landed (vendor elements inside an `Info` block read as process nodes; an XML comment
inside an `Array` gluing the numbers either side of it into one token). The parity
row is a colour matrix in `crates/lumit-render/tests/ocio_parity.rs`: no config,
every built-in colour family at export, a config's display/view, a config's space at
export — plus a plain-gamma view that must render differently, without which the rest
pass when nothing is bound.

**And both reference fixtures are in**, from one PyOpenColorIO 2.5.2 session:
`aces-1.2` (128 rows, its five reachable LUTs at 14 MiB of a 444 MiB set) and `aces-cg`
(784 rows over every role edge and all 37 display/view pairs of
`cg-config-v4.0.0_aces-v2.0_ocio-v2.5`), with five vendored bakes at 47 MiB. They were
planned as data drops and turned out to be five reader faults and four new §5.4
measurements — see `tests/fixtures/README.md`.

What OCIO still owes, all of it recorded and none of it blocking:

- **Exact Rust ports of the five vendored styles** (§4.1 tier two → tier one), one at a
    time, each landing against the `aces-cg.fixture` rows that gate the bake it replaces.
    The number they exist to bring down is **0.117 at the Rec.709 blue primary**, which is
    what a 65-point cube costs on the ACES 2.0 rendering; inside the gamut the
    same bake is better than 2 × 10⁻³. It also reclaims the 47 MiB of artefact files an
    installation ships beside the binary.
- **The working-space reading has one sentence, not two** — see WP5 above.

## Next - engine/bridge follow-ups

**Settings pages still unbuilt (docs/07 §15's remainder):** CUDA on/off and the
plugins/decoder page. Audio, Autosave and Export defaults all landed; colour
management lives in Project settings; the preview-mode toggle exists.

**The Export drawing's rows are built, both halves** (engine and interface; docs/06
§7.4–§7.5, docs/15-DESIGN.md §12A.4). Audio-only output (`.m4a`/`.wav`), colour
depth, channels and alpha, the output colour space, crop and *use region of interest*,
container metadata, the named preset store, the auto bitrate, the render settings (quality,
disk cache, effects, solo switches) and the *when done* hook are all on
`lumit_render::export::ExportSpec`, all across the seam on `BridgeExportSpec`, and all on
the dialog's one scrolling page, with a per-format capability table refusing what a format
cannot carry. The picture's two remaining dead rows are backed as well: the resize
picks its filter (Fast bilinear, High Lanczos-3), and the colour space is a family of five
built-ins the container is stamped with. What is left:

- ~~The resampler face and the colour-space list do not cross the seam~~ — **landed
  2026-08-25**, and the dialog's rows came alive the same day: every face the
  drawing shows is a live control writing into `BridgeExportSpec`, with the capability row
  deciding which of them the chosen format may honour. The one control still drawn dead is
  the *Managed by* row — which since the OCIO UI landed reads the project's config path
  rather than "No OCIO configuration", and stays dead on purpose: colour management is
  chosen in Project settings, and an export dialog must not edit the project.

- **The *Still* output type is withdrawn**: a still is an image sequence of one
  frame, which the span already says, so the fourth chip the drawing offered is gone rather
  than pending. What is genuinely missing is only the *naming* — a one-frame sequence is
  written `shot.00001.png` rather than `shot.png` — and that is a rule in the encoder's
  file naming, not an output type.
- ~~Reordering the queue~~ (docs/07 §11: "items are reorderable") — **landed 2026-08-25**:
  `export_queue_move(id, index)`, undo-free like removal because the queue is not
  in the `.lum`, refusing an item that is running or has already run. The window's drag
  landed with it: a waiting row is picked up and carried, on Flutter's own reorderable
  machinery rather than a bare draggable, which loses the gesture to the list it sits in.
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
Still open: **nothing says whether the proxy *file* itself is broken** - that wants a
new query over the renderer.

**Two small settings follow-ups** — the "Show shortcut hints" switch exists in the
drawing but nothing consumes a hints flag yet (the menu bar and tooltips must read it
before the switch can honestly exist); and the Settings drawing's slider face (2px
track, primary knob, no fill) disagrees with the Main drawing's zoom slider that
`HouseSlider` was built from — each surface should wear its own manifest's face.

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
by bare path, so a camera track over a run of stills analyses its first frame
alone. It wants the same `MediaSource` the Viewer's decode already takes;
`crates/lumit-bridge/src/api/track.rs` resolves the path it hands over.

**A relinked run keeps its old name.** The Project panel names a sequence for its
span — `frame[0001-0050].png` — and relinking rewrites the media reference but not
the name, so a run that gained or lost frames while it was away shows a stale
span. It wants the relink to rename a sequence item in the same batch.

**Camera tracking, phase 4 stage 3** (docs/impl/tracking.md §5a–§5b).
Stage 1 landed the model half — `ParamKind::Action`, the Camera track effect, the
solve link and Convert to keyframes, all against an injected solve. Stage 2 landed
the engine half: `lumit_render::track` — the analysis job on its own thread with
cancellation between frames and inside the solve, the `track/` sidecar keyed by
(media fingerprint, settings, mask geometry), the real `CameraSolveStore`, the
conversion into `CameraPose`, and the derived camera threaded into the render path
and the frame key.

**Stage 3 largely landed** (the effect's Analyse/Cancel and staged status readout,
the point cloud following the effect and the solve, the analysed-span bar). Still
owed from its list: **the 2D track exports** (keyframed transform /
corner-pin from a track group, docs/08 §7's Tracker row).

Four smaller things stage 2 left, each recorded in docs/impl/tracking.md §5b:

- **Warm and clear are not wired to a project's life yet.**
  `lumit_render::track::request` with `analyse: false` reads a cached solve without
  decoding anything, and `clear()` empties the store; opening a project should do
  the first for every tracked layer and closing it the second. Until then a link
  resolves only after Analyse is pressed in the session.
- **A Camera track on a Precomp layer does not analyse.** The effect is allowed
  there and the solve link already resolves *through* a precomp to the footage
  inside; what is missing is analysing a nested comp, which means rendering it
  frame by frame rather than decoding a file.
- **Masks are flattened at layer time zero.** The tracker takes one fixed set of
  exclusion regions for a whole run, so a mask keyframed to follow a moving object
  — the obvious thing to want — is honoured only in the shape it starts on.
- **One analysis at a time.** A second `request` while one runs answers `Busy`
  rather than queueing. Deliberate (two disk-bound jobs halve each other), and a
  queue is a small change if anyone asks for one.

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
    the Flutter frontend drives shares the render device correctly. Pass a
    context in, or delete the path if nothing drives it.
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
code (`flutter_ui/lib/l10n/`, `crowdin.yml`); what is left is other people's turn and
three small gaps:

- **Confirm the Crowdin language settings took, on the next pull.** The first
  pull landed five languages and reddened main twice, both from Crowdin settings, both
  since corrected there (the `zh`/`zh_Hant` mapping, en-US off) but not yet synced.
  After the next `crowdin pull translations`, `test/l10n/arb_test.dart` passing is the
  proof; if `@@locale` comes back hyphenated anyway, the fix moves into CI as a
  rewrite step on the sync branch.
- **The two numbered shortcut labels stay English.** `lumit-keymap` builds "Add marker
  {n} at the playhead" and "Go to marker {n}" with `format!`, so they are not literals
  the lookup table can hold (`lib/l10n/engine_labels.dart`). Give the bridge the number
  separately, or the label a stable id, and they join the rest.
- **No CI check that the source file was pushed.** A string added here is invisible to
  translators until somebody runs `crowdin push sources` by hand. Worth a release-time
  step once the project exists.

**Lens flare follow-ups ([impl/lens-flare.md](impl/lens-flare.md))** — the
shipped core is docs/08 §3.27; its performance items sit in **Now** above. Still owed:
the **Lights source wiring**; an **image aperture** file parameter; the **lens
designer** (`lens_file` has landed, so its output has a place to go); an
**Occlusion layer** reference; **adaptive grid refinement at vignette folds**, the real
cure for both known limits (six ablations are already ruled out — do not re-chase
them with guards). Panel side: the pair row's dropper on
**Transform's px@comp pairs** (the pick exists); **Radial blur's centre
migration** to px@comp; one-op writes for a paired keyframe toggle.

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
these return on the **property** path — the segment calls named here
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
    at all, since Retime moved onto the property path.

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
move** (§9 - the toolchain pin landed, the edition did not); the
`indexing_slicing` / `arithmetic_side_effects` clippy denies after a hot-path sweep (§4);
`clippy::pedantic` with curated allows (§7); the golden-frame EXR export corpus (§6).

**Four unmaintained dependencies are deliberately ignored in `deny.toml`.**
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

**A genuinely FFmpeg-free build is not possible yet.** `lumit_bridge
--no-default-features` compiles the bridge's own decode paths out, but `lumit-render` and
`lumit-audio` depend on `lumit-media` unconditionally, so the library is still linked and
the build still needs it installed. Making those two deps optional — and the render/audio
paths that use them — is what "builds without FFmpeg" would actually take.

**The three-tier cache's remaining sharp edges.** The disk tier's write queue was
bounded after it reached 81 GB on an idle Mac; the same shape of question is worth asking of
the *other* unbounded `mpsc` channels the worker owns (the loaded-frame return, the
prefetcher's results) — none carries whole frames as freely as the park queue did, but none
counts its depth either. Also owed from that hunt: nothing reports how deep the park queue
is running, so a machine whose disk cannot keep up degrades silently (frames simply stop
reaching disk).

**What the performance harness still cannot measure** (`crates/lumit-bench`
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
piece - the render-time column - and the rest of it (continuous timestamp-query
collection, the recording mode, the panel) is in the entry above.

**CI coverage the Flutter port left thin:**
- **macOS and Windows CI do not require an adapter.** `LUMIT_REQUIRE_GPU` turns
    a "no adapter" skip into a failure and the Linux job sets it; the
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
    and free width/height boxes (sizes are preset-driven today). The sound
    rate, width and layout are engine-real now and wait only on the seam
    (`BridgeExportSpec` and `BridgeFormatCaps`) and the dialog's three rows.
- **Export status still speaks the old idiom** - `export.rs` replies in JSON
    strings (`err_json`) polled on a timer; follow the worker's typed-stream way.

- **Four shipped chords still have nothing answering them.** The keymap binds
    them and Settings ▸ Keymap lists them, so they can be rebound and still do
    nothing: `,` / `.` (**previous / next keyframe** — it needs a rule for what
    "the keyframes" are when no property row is picked), `Ctrl+,` / `Ctrl+.`
    (**previous / next edit point**, which needs an idea of what counts as an
    edit point in a comp), `` ` `` (**maximise the panel under the pointer**,
    dock work), and `K` (**shuttle pause** — the shuttle itself is not built,
    which is why `J`/`L` step a frame instead). Everything else in docs/07 §15
    is dispatched.

- **The menu bar names its own backlog.** Every row marked
    "(Not implemented)" in File/Edit/Composition/Layer/Animation/View/Help is a
    command with a place waiting for it. What is left, now that the rows whose
    engine call already existed have been wired — History, Trim and Crop comp,
    Layer styles, the Animation menu's three Reveal rows, the View menu's grid,
    ruler and snap rows, and **Save** and **Apply animation preset** (the same
    `.lumfx` the Effects & presets panel writes): **Preserve transparency**,
    **Auto-outline**, the Layer menu's **Camera** family, the Layer menu's
    **Reveal** row, and **Track motion** — each of which needs an engine or
    bridge seam that does not exist yet. Two are marked for a
    reason rather than for want of time: **Add text selector** has nothing to
    add while a text animator carries exactly one range selector, and
    **Show wireframe** is wired to the layer-controls switch until the full
    wireframe display mode (docs/07 §2.2 item 5) gives it something of its own.
    Delete each mark as the command lands. No chords are suggested for these;
    the shipped table is docs/07 §15.

- **A Flatpak remote, so `flatpak update` has something to update from.**
    Releases ship a single-file `.flatpak` bundle, which installs perfectly well
    and then never updates: `flatpak update` needs a remote. Export an OSTree
    repo in `release.yml`, publish it (Cloudflare Pages beside the site)
    and ship a `.flatpakref`, or submit to Flathub and let it host. Until then
    Lumit tells Flatpak users the install command rather than offering a button.

## Later

**AE import, phase 1 (docs/impl/ae-import.md §6) - the walker and the reader
landed 2026-08-21; three things are still open.** `tools/ae-bridge/` holds the
ExtendScript walker and the fixture builder, `crates/lumit-import/` holds the bundle
open and the capture types, tested against the golden bundle and two hand-written
ones (the schema's readable documentation, and the awkward half one well-formed AE
project does not contain).
 - **The golden bundle landed 2026-08-20**, from one sitting on a live After Effects
   26.0: `tools/ae-bridge/fixtures/fixture.lum-bundle/` (two comps, 24 layers, 109
   unreadables) with `crates/lumit-import/tests/golden.rs` asserting every §5
   checklist row through the mapped document, the exact report counts, and the
   unreadables' four known classes. It confirmed the match names and corrected three
   assumptions (null and adjustment layers are backed by solid *items*; a −100%
   stretch arrives with its two ends reversed and before comp zero; AE 26 records the
   modern matte form for both generations). **Two checklist rows are owed against the
   walker, and each needs one more AE sitting:**
   - **Roving.** `make-fixture.jsx`'s `setRovingAtKey(2, true)` did not take — the
     capture records `roving: false` on every Position key, and the walker reads
     `keyRoving` correctly, so the fixture never contained a roving key to import. The
     next sitting should set roving on a key whose neighbours are bezier, and report
     what the builder's step harness caught if it still refuses.
   - **A 3D layer's Orientation and Material Options.** The capture carries
     `ADBE Orientation` (`[0, 30, 0]` on the fixture's 3D card) and Casts Shadows;
     the mapper reads only `ADBE Rotate X/Y/Z`, so both are dropped **without a report
     row** — the one place the mapping loses something silently, against its own
     standing rule. Either map orientation onto the three rotation lanes (they are not
     the same thing: orientation composes before rotation) or raise a row. The camera's
     Point of Interest, which AE stores under `ADBE Anchor Point`, lands on the
     anchor-point lanes for the same reason, and its Depth of Field, Aperture and Focus
     Distance are dropped rowless too.
 - **The structural mapping landed 2026-08-21** (phase 2, first half):
   `crates/lumit-import/src/map/` turns a capture into a whole new
   `lumit_core::Document` — the item tree, comps, layers with their kinds, switches,
   parenting, mattes and masks, the keyframe value copy, blend modes with docs/11 §4's
   documented fallbacks, both of AE's times as one Retime, markers, and the typed
   `ImportReport` — with twenty-three tests across two fixtures (`synthetic.lum-bundle`
   is the ordinary half, `edges.lum-bundle` the awkward one) plus a save-and-reload
   round trip through `lumit-project`.
 - **The effect table's colour / blur / generate / temporal half landed 2026-08-21**
   (phase 2, second half): `crates/lumit-import/src/map/fx_colour.rs` claims
   twenty-seven match names — the Blur & sharpen, Colour, Generate and Temporal rows of
   docs/11 §5, plus the two rows §5 places at a placeholder on purpose (Remove Grain and
   Timewarp, each reporting what does the job instead). Per-parameter unit conversion
   (px@comp), option lists pinned against `tools/ae-audit/
   ae-audit-report.json`'s defaults, mask references on the mask-path row, and thirty-three
   conversion tests. Two things are owed:
   - **The golden-frame tests §5 requires of every mapped conversion** — the golden
     *bundle* has landed and `tests/golden.rs` checks every converted number against
     one worked out from the fixture's own inputs, which is not the same thing: these
     need After Effects *renders* of `fixture.aep` to compare pictures against.
   - **A keyframed dropdown in this half goes by unremarked.** `fx_colour`'s reader for
     the controls Lumit does not animate - option lists, switches, seeds - reads the
     still value only, so an instance whose Fractal type (say) is keyframed imports at
     Lumit's default with no report row. The distort half reports "the value it starts
     on" for the same case; both halves should. Rare in real projects and behind no
     docs/11 clause, but it is the one place either half changes something silently.
   - **Three undocumented bases are stated choices, not measurements**: Fractal noise's
     Scale, Advanced Lightning's Turbulence and Add grain's Softness convert on the
     "AE's default lands on Lumit's default" anchor docs/11 §5 now records. The
     golden-frame tests replace each with a measurement.
 - **The effect table's distort / stylise / transition / utility half landed 2026-08-21**
   (phase 2, second half): `crates/lumit-import/src/map/fx_distort.rs` claims twenty-nine
   match names — the Distortion, Stylise, Transition and Utility rows of docs/11 §5, plus
   Channel blur and Median, which no other half claims. Per-parameter unit conversion
   (px@comp, AE's per cent of the layer, and the two bare factors
   AE reads as decimals), the option collapses and splits §5 names, layer references onto
   the universal matte row, AE's two clock-reading controls as keyframes, and thirty-eight
   conversion tests. Three things are owed:
   - **The golden-frame tests §5 requires of every mapped conversion**, as above.
   - **The five Controls match names are claimed but unaudited**: `ADBE Slider
     Control`, `ADBE Angle Control`, `ADBE Checkbox Control`, `ADBE Color Control` and
     `ADBE Point Control` were added to this half after the 2026-08-20 sitting, so
     docs/11 §5 marks their rows **pending** and `tools/ae-audit/
     claimed-matchnames.txt` carries them (60 names to 65). The next sitting confirms
     the five spellings; a wrong one costs only the placeholder road §6 already
     specifies.
   - **Turbulent displace's Pinning maps at one index**: the audit records a dropdown's
     default but not its option strings, so only AE's own default (every edge) is pinned
     from evidence and every other index is reported rather than guessed. A second audit
     pass that enumerates option strings closes it, and would also confirm the orders
     this half took from Photoshop's published list (Warp's fifteen styles) and from AE's
     own defaults (Wave warp's eight pinnings, the ten-entry channel picker).
 - **The surface landed 2026-08-20** (phase 3): `LumitBridgeState::import_ae_bundle`
   in `crates/lumit-bridge/src/api/import.rs`, adopting the mapped document through
   the `api::state::adopt` road `open_project` now shares, with footage relinked by
   `resolve_all_media` against the bundle's folder; File ▸ Import ▸ Bridge bundle
   folder…; and the report window `flutter_ui/lib/shell/ae_report_frb.dart`. Reasons
   cross as a stable id plus their facts and are written in
   `l10n/engine_labels.dart`, gated by `engine_labels_test.dart` reading the
   `Reason` enum. Three things are owed:
   - **The collected `footage/` copy is still owed.** Of docs/11 §2.5's four relink
     steps, the absolute path and the search-folder sweep both run (2026-08-24:
     whatever the first three resolver steps leave lost is looked for by file name
     under the folder the `.aep` or bundle was picked from, one walk for all of them),
     so a project copied across with its media beside it now comes up linked. What is
     left is the collected copy and the hash verification that wants it: write the
     `footage/` folder, store a genuinely relative path beside it, and the re-rooting
     and fingerprint steps start paying too.
   - **A report row does not lead anywhere** (docs/11 §9's navigation): a row names
     its comp ▸ layer ▸ property and double-clicking it does nothing. It needs the
     row to carry an id, not just a path, which means the bridge row carrying one.
   - **The report is not kept** (docs/11 §9's persistence): it lives as long as its
     window, is not stored in the project's `ae` namespace, is not reopenable from
     the File menu, and is not written beside the bundle as `import-report.json`.
     The reason-level filter §9 asks for (disabled expressions as their own list)
     belongs with that work; the built filter is by outcome.

**The direct `.aep` parser (docs/impl/ae-import.md §7) - phases A, B and C all
landed 2026-08-21; what is left is depth, not surface.** `crates/lumit-import/src/aep/` reads an
After Effects project file itself and fills the same `Capture` the Bridge writes, so
the mapping, the effect table and the report are shared unchanged: `rifx.rs` is the
bounds-checked container walk, `enums.rs` the funnel tables, `mod.rs` the structure
decode and `open_aep`, `props.rs` the property system. `tests/aep_differential.rs`
parses `fixture.aep` and compares the project block, all 22 items, both comps'
settings, all 24 layers and every property tree against
`fixture.lum-bundle/capture.json` - AE's own account of the same file - field for
field; §7.1 and §7.2 are the proved layout maps.
 - **Recovery, asserted in CI** (§7.2 has the table): 684 static property values
   exact with none wrong and none invented - the 646 the file stores, plus the 38
   Position and Anchor Point defaults the parser writes in for the records After
   Effects leaves out, each asserted against AE's own number - 27 of 27 keyframes with their ease and
   spatial tangents, 2 expressions, 13 effect instances, 2 masks with their paths,
   4 markers, 1 separated-dimension property, and the 3 `CUSTOM_VALUE` blobs as raw
   bytes - which the Bridge cannot get at all. `map_capture` on the parsed capture
   and on the golden bundle produce documents with identical counts. The 2,734
   golden leaves the parser does *not* report are the ones the file does not store,
   because they are at their defaults. A sixty-four-case damage sweep (truncations,
   flipped bytes, `0xFFFFFFFF` sizes, zeroed runs, fixed seeds) requires an answer
   from every one and times the lot: no panic, no hang, typed refusals.
 - **Phase C landed** (docs/impl/ae-import.md §7.3): `lumit_import::open_ae` routes by
   the file's magic — folder → bundle, `RIFX`/`RIFF` → `open_aep`, else the zip reader —
   so one bridge call takes both front doors and the picker's only job is to offer both.
   File ▸ Import ▸ After Effects project… is the file picker (`.aep`, `.zip`), Bridge
   bundle folder… the quieter folder one; skipped chunks arrive as
   `Reason::ChunkUnreadable` rows on the same summary, raised only for this route so the
   Bridge's own unreadables are not said twice; an `.aep` this build cannot read posts a
   calm notice naming the Bridge route, project standing. Proved end to end by
   `flutter_ui/test/frb/ae_import_frb_test.dart` on the real `fixture.aep`. **One policy
   line from docs/11 §7 is still owed**: a whole-file failure should fall back to
   "import footage references only" where the footage table is readable, and today
   `parse_capture` simply refuses with `NoItemTree`.
 - **Corpus testing is owed.** One fixture from one After Effects version proves the
   offsets it contains and nothing about the ones it does not. Real community project
   files across several AE versions, run through the parser looking for panics, refusals
   and empty imports, is what turns "measured on one file" into "measured".
 - **One doc debt the phases left behind.** An effect **parameter name** now has a CI
   assertion but an effect parameter *value* in DOM units is asserted only through the
   shared value sweep; that is enough today and worth naming if the units table grows.
 - **Two encodings are still owed**: a text document (`btds`) and a gradient
   (`GCst`). The text document arrives carrying its match name and a note saying the
   encoding is not decoded, so the report already says so. **The gradient is
   unmeasured**: `fixture.aep` holds no `GCst` chunk at all - the shape layer's
   gradient is at its default and the file stores only what is not - so nothing has
   been proved about it either way, and a fixture with a non-default gradient is owed
   before anything is claimed. Decoding both is still owed after phase C, alongside
   **decoding the arbitrary-data blobs** - the sixteen-point Curves target is
   reachable in principle now that the bytes are in hand, measured rather than
   promised - and **shape-layer and text depth**, which arrive named and marked
   rather than drawn.
 - **Property display names are not read, and may never be.** They are After
   Effects' own localised resources rather than data in the file (a property nobody
   renamed carries the `-_0_/-` sentinel), so 1,106 of the golden capture's names
   have no source in the project. The mapper falls back to the match name; effect
   parameters, effect instances and masks do get their real names - 83 of them, every
   one asserted equal to AE's own, so a drifted `pard` offset cannot hand a parameter
   its neighbour's name unnoticed. A name table for the other 1,106 would be a table
   of Adobe's English strings - a separate decision, not an oversight.
 - **The project-level `LIST EfdG` fallback is not read.** It carries every effect's
   parameter definitions and is what tells a real parameter from a topic heading
   when a layer's own `parT` is empty (Gaussian Blur's is). None of the fixture's
   effects needed it; an effect that does simply reads its slots as the plain
   numbers they are stored as.
 - **A mask path's linear speed is 1.0 per segment in the DOM**, and one sample
   cannot say whether that is a constant or a duration-derived number, so the
   differential exempts it rather than curve-fitting. Nothing downstream reads a
   linear side's speed. A fixture with an animated path over a different duration
   settles it.
 - **The rest of footage interpretation is not read, and needs a fixture that has
   some.** A footage item's **name**, **path**, **placeholder-ness** and
   **missing-at-save** flag are read - measured against a real production
   project and against the layouts `forticheprod/py-aep` documents, with synthetic
   byte fixtures in `aep/mod.rs`'s tests as the regression. Frame rate, alpha, fields,
   pulldown and loop are still unread: Lumit has no field for any of them, and
   `fixture.aep` is solids and comps with no file footage in it, so not one of those
   offsets could be checked against AE. One more sitting with real footage in the
   project unblocks the group, and the differential test asserts the fixture still has
   none so the exemption cannot rot.
 - **An effect on a layer that is not the comp's size is owed.** Both layers carrying
   effects in `fixture.aep` are 640 x 360 in a 640 x 360 comp, so the frame an effect's
   stored two-dimensional point is a fraction *of* could not be measured: the parser now
   reads it against the layer (the format's own convention - the anchor point
   and the mask path are the only other normalised values and both are the layer's),
   which is what `an_effects_point_is_a_fraction_of_its_layer_not_of_the_composition`
   in `aep::props` pins. A sitting with a Transform effect on a precomp or solid of a
   different size than its comp settles it against After Effects itself.
 - **A dragged layer is owed too, and cannot be forged.** Every layer in `fixture.aep`
   starts at zero, so `ldta`'s start offset - what puts in and out points, keyframe times
   and a stretch's reach back on the comp's clock - is measured against AE at one value
   only, and the 50 % layer sitting at zero cannot tell stretch-about-the-start from
   stretch-about-the-origin. The fixture is authored *by* After Effects
   (`make-fixture.jsx` inside a running AE), so hand-written bytes would be this parser's
   guess compared against this parser: owed is a sitting with a layer dragged along the
   timeline and a second both dragged and stretched. Standing in meanwhile:
   `a_layers_in_and_out_are_counted_from_its_own_start` and
   `a_stretched_layer_is_stretched_from_its_start` in `aep::tests`, which prove the parser
   reads the field it was handed and not that the field is where AE puts it. The
   differential test asserts every start is still zero, so the exemption cannot rot.
 - **A reflected layer's ends are one frame loose.** At −100% stretch AE reports its
   two ends 1/3000 s further out than the file's arithmetic gives, as if it reflects
   inclusive indices on an internal grid; with one sample the grid cannot be proved,
   so the differential test compares those two within a frame. A fixture with a second
   negative-stretch layer at a different frame rate settles it.
 - **Every funnel-table row the fixture does not exercise is `reference`, not proved**
   (marked as such in `enums.rs`): most blend modes, two matte types, `WIREFRAME`
   quality, three light types, two auto-orient modes, and the three non-Classic
   renderers. A fixture that uses them turns each into a measurement.

**AE effect parity, wave 1 (docs/impl/ae-effect-parity.md) - landed in full 2026-08-20.**
Eighteen Tier-A effects in four family batches: ~~colour (Curves, Levels, Brightness, Hue
and saturation)~~, ~~generate (Fill, Gradient, Noise, Fractal noise)~~,
~~distort (Turbulent displace, Tile, Offset, Mirror, Lens distort)~~,
~~utilities and transitions (Drop shadow, Set matte, Channel blur, Linear wipe, Radial
wipe)~~. docs/11's seed table is trued for all eighteen.

**AE effect parity, wave 2 (docs/impl/ae-effect-parity.md) - landed in full 2026-08-20.**
All of Tier B, by owner's ruling, with one standing exclusion (no particle-world port).
Six batches: ~~Distort I (Corner pin, Displacement map, Polar coordinates, Twirl,
Spherize)~~, ~~Distort II (Ripple, Wave warp, Bezier warp, Warp, Roughen
edges)~~, ~~Stylise I (Posterize, Threshold, Tritone, Photo filter, Black and
white, Shadow highlight)~~, ~~Stylise II (Median, Mosaic, Find edges, Emboss,
Texturize, Broadcast safe)~~ **landed 2026-08-20, catalogue at 75**,
~~Transitions (Venetian blinds, Iris wipe, Card wipe)~~ **landed 2026-08-20,
catalogue at 78**, ~~Draw and grain (Beam, Lightning, Radio waves, Vegas, Add grain)~~
**landed 2026-08-20, catalogue at 83**. Scribble, Stroke and Vegas' Mask/Path
half stopped on the mask seam and landed with it the next day - **landed
2026-08-21, catalogue at 85**. docs/11's seed table is trued for all thirty-two, with no
substitutes left in it.
 - **A mask-path row names one mask, and three AE controls want a set** (docs/08
   §3.78-§3.79). Scribble, Stroke and Vegas' Mask/Path source are built and the import's
   substitutes are retired; what is still reported against the seam is AE's **All Masks**
   and **Stroke Sequentially**, and Scribble's two multi-mask Fill Types. All three want a
   row naming a *set* of masks - a small extension of `ParamKind::MaskPath` and a list
   rather than a slot in the carriage. Nobody has asked for it.
 - **A path drawing is capped at 512 straight pieces** (docs/08 §3.78). The geometry
   rides in a uniform, exactly as Lightning's bolt does, and past the cap every consumer
   coarsens rather than drawing part of a shape: the hatch widens its spacing, the dots
   space out, the chain straightens. A storage buffer is the answer the day something wants
   tens of thousands of pieces; nothing does, so none was built.
 - **Lightning ships four of AE's eight types, and no Alpha Obstacle** (docs/08
   §3.74). Breaking, Bouncey, Anywhere and Vertical map to the nearest of the four and are
   reported; Alpha Obstacle asks the bolt to route around the layer's own alpha, which is a
   *search* rather than a formula and would change the effect's cost class. If it is ever
   wanted it wants a distance field of the alpha and a bolt built against it, both of which
   the host-side generator could do without touching the kernel.
 - **Beam has no 3D perspective** (docs/08 §3.73), for Card wipe's reason: AE's
   foreshortens the beam from a camera of its own, and Lumit keeps cameras on the
   composition (docs/06). The same composition-camera input that would give Card wipe its
   grid would give Beam this.
 - **Radio waves ships one Stroke width where AE tapers from a start to an end**, and only
   its Polygon wave type (docs/08 §3.75). A taper needs the *age* to reach the
   stroke's width, which it already does for the fade — so it is a cheap addition whenever
   somebody wants it. Image Contours is Vegas, and so is Mask now (its Mask/Path
   source) - both are reported as suggestions rather than built into Radio waves itself.
 - **Vegas' Segment length is a length, not a count** (docs/08 §3.76). AE traces the
   contour into a path and can therefore count segments *around* it. **On the Mask/Path
   source this is fixed**: there the dashes are spaced by measured distance round
   the mask, so they stay even however hard it curves, and the import converts AE's Segments
   exactly. It is only the contour half that still drifts in phase on a curve, because it
   still never traces one - the machinery that would let it is now sitting next door.
 - **Card wipe has no camera, no back layer, and no Card Scale** (docs/08 §3.72).
   Each card is projected in its own local frame at a fixed viewing distance, because
   Lumit keeps cameras on the composition (docs/06) and has none on an effect. If effects
   ever get a composition-camera input, the grid could be projected through it and AE's
   Camera Position / Corner Pins / Composite Camera would stop being reported. A back
   layer would need a second layer row, which §3.68's test says a card wipe can justify.
 - **Card wipe's Flip order has no Gradient entry** (docs/08 §3.72). AE reads that
   order from a gradient *layer*; Lumit's one layer row is the universal Matte, and a card
   wipe wants to say "only over the sky" as well as "in this order". A Gradient order can
   arrive later on a row of its own without moving anything. Randomness plus Seed covers
   the intent meanwhile. The import cannot read the spread - the capture carries the
   gradient layer's *index*, not its pixels - so an instance using Gradient imports as
   Left to right on AE's own Timing Randomness, and both are reported.
 - **Median's Radius is capped at 3 and cannot be typed past** (docs/08 §3.64), the
   only control in the catalogue for which that is true. The cost is the fourth power of
   the radius, so a larger window needs a different algorithm - a per-tile histogram, or a
   separable approximation that is no longer a median - and either is its own programme
   with its own oracle. The import writes 3 and reports the instance as approximated.
 - **Texturize's Placement cannot honour AE's *native-size* Tile and Centre** (docs/08
   §3.68). The layer carriage renders a referenced layer at this raster, so the
   texture arrives frame-shaped and Scale is what says how big one copy is. If a layer
   input ever carries its source's own dimensions alongside the texture, the three
   Placements could use them and the import would stop approximating the size.
 - **The Stylise II proof renders on the CPU, and the fixtures are gradients.** Median,
   Find edges and Emboss are the first effects whose picture cannot be judged on the smooth
   clips in `C:/tmp/lumit-shots` at all, and the batch was judged on a screenshot instead.
   A fixture with real high-frequency detail in it - a resolution chart, a page of type -
   would serve every future edge-detecting or despeckling effect.
 - **Shadow highlight has no Auto amounts, and probably never should** (docs/08
   §3.63). AE's is a whole-frame histogram reduction smoothed across neighbouring frames,
   which makes a grade whose answer at a frame depends on the shot around it. If it is
   ever wanted, it is a *scene analysis* feature with its own cache and its own doc, not a
   checkbox on this effect — and the import already reports it.
 - **Shadow highlight ships one Radius where AE ships two.** The second full-frame
   gaussian is real work for the softness of a mask; if a shot ever needs the shadows'
   mask measured at one scale and the highlights' at another, the kernel takes a second
   bound texture and the uniform grows one float.
 - **The old distort kernels still guard a texture fetch instead of clamping it**:
   Mirror, Tile, Lens distort, Drop shadow, Transform, Shake and the blur
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
    D3D11→DX12 interop and VideoToolbox; proxy generation; the resource
    governor; ProRes/DNxHR intermediate export (v1 is
    H.264/HEVC only); the 8-/32-bpc working-depth switch (v1 is fp16 only); OCIO
    v2 colour management and its UI.
- **Audio** ([07-UI-SPEC.md](07-UI-SPEC.md) §10, [09-AUDIO.md](09-AUDIO.md)): the
    Audio panel, the Mixer, the meters and the beat-tuning controls all landed with
    the AudioWorkspace programme. Still owed: persistent
    waveform peak files (the multi-zoom summary is built on demand and cached for
    the session — never written to the project sidecar, so it is rebuilt
    next time the project opens); the §3.4 scrub-audition grain and its Timeline
    toggle; and §5's replace-or-merge offer on a re-run (detection replaces).
- **File format ([10-FILE-FORMAT.md](10-FILE-FORMAT.md)).** Embedded `thumbs/`
    previews in the `.lum`; the per-project sidecar `proxies/`, `peaks/` and
    `flow/` directories (only `frames/` and the global media index exist).
- **Design ([15-DESIGN.md](15-DESIGN.md)).** The font bundling item moved into the
    redesign programme's phase 2 above (Hanken Grotesk and Geist Mono - the
    household faces are no longer wanted); still here: the missing type-scale steps
    in the theme struct, and identity colour tokens for Shape and Null layers
    (§6.1 reserves the values; both kinds borrow today).
- **Platform.** The macOS pass - native menu bar, VideoToolbox, ProRes; it also
    owes `application:openFile:` (a double-clicked `.lum` opening). The document
    `.icns` files now ship inside the bundle, so the icons themselves are done.
    The Metal/IOSurface Viewer path is unverified on real hardware.
    Developer ID signing and notarisation landed but have never run —
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
    [08-EFFECTS.md](08-EFFECTS.md) §3 to gain its entry and a curve chosen once,
    which both then share.
    This gate is the v1.0 milestone.
- **Phase 4 - Extensibility** (whole docs, nothing built -
    [11-AE-IMPORT.md](11-AE-IMPORT.md), [12-PLUGINS.md](12-PLUGINS.md)). AE
    import (Bridge panel, `.aep` parser, Lottie, fidelity report); the OFX host;
    the LFX C ABI + validator; expressions landed on Rhai;
    the OFX host and the LFX C ABI + validator remain. Placeholder
    round-tripping already preserves unknown effects/expressions.
- **Phase 5 - AE parity march.** 2.5D cameras/lights/DOF, tracker/stabiliser,
    keying, particles, tier-2 effects, shape operators. Rotoscoping and Puppet
    landed; text animators landed; the Composer audio
    workspace landed with CLAP/VST3 hosting ([09-AUDIO.md](09-AUDIO.md)).
- **Phase 6 - Beyond parity.** Node view over the evaluation graph, Blender scene
    import, Lottie export, OpenTimelineIO interchange, render-farm/CLI export.

**Tracking (docs/impl/tracking.md) - all four phases landed
(2026-08-20, 2026-08-21).** `crates/lumit-track` holds the track substrate -
Shi-Tomasi detection on a 16x16 bucket grid, pyramidal affine KLT with
forward-backward and NCC verification, exclusion masks, re-detection into
starved buckets - the two-view geometry over it (Hartley-normalised 8-point and
7-point fundamentals inside LO-RANSAC, the GRIC gate that calls a pan a pan,
parallax-driven keyframe selection, epipolar dynamic-track segmentation, the
zoom cut/ramp detector), and now the global solve: `solve_camera` returns a
`CameraSolve` with a pose per frame, a focal per segment, the point cloud and
the per-frame error. Forty-three tests, all synthetic, no assets.
 - **Phase 4 landed** (note §5a-§5c): the Camera track effect, the
   analysis job on its own thread with the `track/` sidecar, the solve-linked
   Camera layer, Convert to keyframes, the point cloud with select → Null/Solid,
   and the cancellation seam inside `solve_camera`. Still open from phase 3's
   hand-off list:
   - **A focal hint.** Every tracker worth using lets the operator type the lens
     in, and self-calibration is the weakest number in the pipeline (note §4's
     deviation 1). `SolveSettings` wants an optional `focal_px` that skips the
     search and pins the first segment; the cut ratios already carry it to the
     rest.
   - **A nodal-pan product.** `SolveError::RotationOnly` refuses a shot with no
     baseline, and that refusal is right for a *camera* solve - but the
     rotations are recoverable and a Camera layer that only turns is a real
     deliverable for a locked-off pan. It needs its own output shape and its own
     decision entry.
   - **2D track exports** (docs/08 §7's Tracker row): keyframed transform and
     corner-pin data from a track group, riding the same store.
 - **A zoom inside a moving shot is not detected, and a multi-frame rack can
   never be a cut** (note's Open questions, measured 2026-08-24 from a real
   7135-frame train POV that went wrong the moment the shot scoped in).
   `detect_zoom` merges every adjacent "hot" pair into one run and only calls an
   **isolated** hot pair a cut, so (a) a lens rack over several frames is always
   a `Ramp`, and (b) forward camera motion — which grows every patch in the
   frame every frame — makes the whole clip one run, inside which a genuine
   1.4x scope-in is one sample in a median of thousands and vanishes. The shot
   then gets one focal for two lens settings and the camera path is wrong from
   the rack onwards, with nothing said. Ordered behind the focal-curve item
   below, because finding the boundary is only half the answer: a multi-frame
   rack is a ramp whichever way it is detected. The cheap partial answer
   meanwhile is to **surface `SolveNote`** — `ZoomRamp` is already produced and
   already right, and nothing carries it across the bridge.
 - **The zoom ramp is one focal, not a curve** (note §4's deviation 7). A
   segment containing a detected zoom ramp is flagged and reported as
   `SolveNote::ZoomRamp`, and its focal is a single number over the whole run
   where the note asks for spline knots. The bundle already treats each
   segment's focal as an independent column of the reduced camera system, so
   knots are more columns in the same solve rather than a rewrite - but nothing
   reads a ramp's shape today, so it waits for phase 4 to have somewhere to show
   it.
 - **Lens distortion (k1/k2) is not solved.** The note's camera model allows an
   optional pair per segment; phase 3 fixes the principal point at centre and
   solves focal alone. Two more columns in the same bundle, and the same
   `ponytail:` ceiling applies.
 - **The coverage gate owes `lumit-track` its `-p` flag.** CI's
   `cargo llvm-cov` line in `.github/workflows/ci.yml` names the engine crates
   one by one and does not yet name this one; measured locally at 95 % lines,
   well clear of the 80 % floor, so adding it cannot turn CI red. Held back only
   because the crate landed while other work held that file, and phases 2 and 3
   landed the same way.
 - **The Shi-Tomasi response map is a whole-frame pass and dominates when
   re-detection runs** - 24.4 ms/frame against 11.0 with re-detection off, on
   100 features over 640x360 (the note's measured number). Its box sums are
   separable; that is the cheap win, and it comes before any WGSL port.


**From the Caddis study, parked by the owner (2026-08-25):**
- **B8 - Choke, Inner glow, Inner shadow** as catalogue effects.
- **B9 - Slitscan, Dither, Draw Glass**; and aperture-shape upgrades beyond DOF's.
- **B4 - a details inspector** (read-only per-layer engine state: buffer size, format,
  colour space, cache tier). The bottom strip already reads the cache; the unique
  value is per-layer format/colour-space once OCIO debugging is routine - build it
  then, as a section of the existing Source card rather than a new panel.
- **7.22 hover-scrub thumbnails** on Project rows - only if they cost nothing at rest.

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
    command-buffer item under *Now*, which is where the win actually is. Anyone
    reaching for the pool again should re-run the stopwatch first: if the
    processor half has not grown, this entry still stands.
- **Rotation gizmo affordance** - the previous frontend never offered one; not a
    regression.
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
- The two recorded behavioural deviations (export queue-snapshot timing;
    share-export VBR cap).
