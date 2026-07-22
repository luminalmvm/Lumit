# 06 — Remaining work (delete-on-done ledger)

Every partially finished (◐/◑) or not-started (☐) item extracted from
05-PARITY-CHECKLIST.md on 2026-07-22 (owner request). **Rows are deleted as they
land** — the burn-down is complete: sections A–E landed together, the final
integration sweep (2026-07-22) closed the last cross-agent seams, and the final
UI wave (2026-07-22) built the Dart-side UI the v0.9 engine surface unblocked
(beat markers, sequence sub-bars, the overrun HOLD hatch, asset read-back,
effect-param animation, `.lumfx` presets, mask geometry, the Auto tier). **What
survives below is only genuinely blocked or honestly-deferred work, each row
carrying the evidence for why it cannot land yet.** 05 stays the permanent
record.

Excluded on purpose (not parity work): flutter_rust_bridge codegen (deferred by
design until the API stabilises), the macOS pass, the post-parity design changes
in 05 §post-parity, and the two recorded behavioural deviations (export
queue-snapshot timing; share-export VBR cap).

Closed in the final sweep (2026-07-22), removed from the burn-down:

- **Shell Ctrl+C/V → keyframe clipboard** — the shell key handler now routes
  Ctrl+C/Ctrl+V to `AppStateStub.copySelectedKeyframes`/`pasteKeyframes`
  (`shell/shell.dart`), behind the same text-field focus gate as the other
  shortcuts (egui note 2.2 / UI-7).
- **Resolution picker downsample** — `PreviewSource` threads
  `app.previewScale.factor` through the primary comp render (and the Dart LRU
  key carries the scale, mirroring the engine cache's per-scale keying), so
  Half/Third/Quarter actually render fewer pixels (`preview_source.dart`).
- **Timeline cache bar** — the `cache_stats` Dart binding was already on
  `CacheControlBridge`; `AppStateStub.cacheStats()` exposes it, warm frames are
  tracked as the `PreviewSource` drives them into the engine cache
  (`noteFrameWarmed`, scoped per comp+scale, reset on edit/clear), and
  `panels/timeline/cache_bar.dart` draws the RAM-tier band over the ruler
  (theme.success, 15-DESIGN §6.3), polling on the `cacheBarRevision` cadence,
  never per-paint.
- **Layer context menu final wiring** — Rename opens an in-place outline editor
  (`renameLayer`), Add effect opens the categorised submenu from `listEffects()`
  (`addEffect`), Convert to sequenced calls `convertToSequenced`, and Trim to
  source end calls `trimToSourceEnd`, offered only for a retimed footage clip
  (the egui condition, menu.rs:174-184) — `panels/timeline/layer_menu.dart`,
  `layer_row.dart`.
- **EffectDragData onto timeline rows** — each layer row is now a
  `DragTarget<EffectDragData>`; a dropped effect applies to that row's layer
  through `addEffect` (`layer_row.dart`), the sibling of the Effect controls
  drop target.
- **Project-panel thumbnails** — footage rows render a small decoded thumbnail
  through `app.thumbnail`, decoded asynchronously off the build and cached until
  the document epoch advances (a relink re-decodes), with the type glyph as the
  placeholder (`project_panel.dart`).
- **DragValueField Reset targets** — sensible `resetTo` defaults now flow to the
  transform axes (the property seed), the text size (72 pt), the New-composition
  width/height/duration (1920×1080 / 30 s), the autosave interval/copies (5 / 3)
  and the three cache budgets (`effect_controls_panel.dart`, `dialogs.dart`,
  `settings_window.dart`).

Closed in the v0.9 engine-surface wave (2026-07-22), removed from the burn-down
— every one was *expose-what-exists* or a clean wire, not an engine-model change
(evidence: the model already held clips, marker kinds, `start_offset`, the text/
solid/camera assets, and the full `EffectKey` + parameter `Property` animations;
lumit-eval's `RealtimeController` was built and tested, only unwired):

- **Recovery journal-append wired** — `Bridge` now carries a `JournalFile` armed
  at every document-install point (`new_project`/`open_project`/
  `restore_journal`), and `state::commit`/`journal_append` append every op after
  a successful store commit (the direct-commit ops — `new_composition`,
  `import_footage`, the retime `→Rate` setter — append too), matching egui's
  `AppState::commit`. Save/new clear it. `restore_journal` now recovers THIS
  frontend's unsaved work.
- **Beat markers drawn distinctly** — the snapshot now carries `marker_details`
  (`[{frame, kind, confidence?, label, duration_frames?}]`) alongside the bare
  `markers` frames (additive). `kind` is the model's `MarkerKind`
  (`user`/`beat`/`chapter`); a beat carries its 0..1 confidence. Dart:
  `BridgeMarker` on `BridgeComp.markerDetails`.
- **Sequence sub-bars** — a Sequence layer's `clips` ride the snapshot (stable
  ids, comp-frame placement, source refs, the clip's retime). Dart: `BridgeClip`
  on `BridgeLayer.clips`.
- **Overrun HOLD hatch — the data** — a layer now carries `start_offset_frame`/
  `start_offset_secs` and its `in_secs`/`out_secs`, the ingredients
  `overrun_span_secs` (`speed_rows.rs:68`) needs that the frame-only read-back
  lacked. (Drawing the hatch itself is Dart-side Timeline work; the missing
  engine data — the blocker — is closed.)
- **Property editors — read-back** — text content/size/fill, a solid's size (the
  colour already crossed) and a camera's zoom now read back from the snapshot
  (`BridgeLayer.text`/`solidSize`/`cameraZoom`), off the session-edit map.
- **Viewer mask draw — geometry** — `add_mask_geometry(comp, layer, kind, x, y,
  w, h)` builds a rectangle/ellipse/star from a drawn drag rect exactly as
  egui's Shape tool does (`overlays.rs`), so the drawn size/position is honoured.
  Dart: `AppStateStub.addMaskGeometry`.
- **Resolution picker — realtime-tier readout** — lumit-eval's
  `RealtimeController` (K-171) is wired into the Viewer render path: a genuine
  render reports its measured cost (`realtime::observe`, gated so a manual-scale
  render never corrupts the Auto model), and `playback_tier`/`reset_realtime`
  expose the tier + scale. Dart: `BridgePlaybackTier`, `AppStateStub.playbackTier`.
- **Effects presets — `.lumfx`** — `save_effect_preset` returns the stack as
  `.lumfx` JSON byte-compatible with `lumit-ui`'s `preset.rs` (a round-trip test
  pins the two), `load_effect_preset` appends with fresh ids (K-065); the
  snapshot also now carries each effect's full `EffectKey` (namespace + version)
  and each animatable parameter's animation state. Dart side needs only the file
  dialogs.
- **Effect controls — per-parameter stopwatch/navigator** — the effect-param
  keyframe ops (`toggle`/`add`/`remove`/`shift`/`set_interp`, with a `channel`
  selector for point/colour) mirror the transform keyframe ops exactly, driving
  each parameter's `Property` animation. Dart:
  `AppStateStub.toggleEffectParamAnimated` and kin.

Closed in the final UI wave (2026-07-22) — the Dart-side UI the v0.9 surface
unblocked, each built against its egui source and covered by
`test/final_ui_wave_test.dart` (plus the existing `edit_ops_test.dart` v0.9
pass-throughs):

- **Timeline beat markers drawn distinctly** — the ruler now takes
  `BridgeComp.markerDetails`; a beat draws as a faint accent tick fading by
  confidence (`0.25 + 0.55·confidence`) from a quarter down the band, a
  user/chapter marker full-height with its flag — mirroring egui `panel.rs:252`.
  Falls back to the bare `markers` frames (all user) on an older library
  (`panels/timeline/ruler.dart`).
- **Sequence sub-bars** — a Sequence layer's clip bar draws its `BridgeClip`
  boundaries as interior hairline dividers (the razor's cut points),
  `panels/timeline/layer_row.dart` `_LaneBarPainter`.
- **Overrun HOLD hatch** — a retimed footage layer that outruns its probed
  source washes + 45° hatches the held span in warning kraft with the exhaustion
  tick and a HOLD tag (panel.rs:994-1076). The `overrun_span_secs`/
  `overrun_local_time`/`evaluate` maths (speed_rows.rs:68, retime.rs:533/1124)
  are ported into `graph_maths.dart` and unit-tested; the span shifts by the
  live move delta like egui's `move_dx`.
- **Asset editors adopt read-back** — the Text/Solid/Camera groups seed from the
  snapshot (`layer.text`/`solidSize`/`cameraZoom`), dropping the session-map
  fallback where read-back exists (`effect_controls_panel.dart`,
  `AppStateStub.textContentFor`/`solidSizeFor`/`cameraZoomFor`).
- **Effect-param animation** — every animatable effect-param row (scalar +
  per-channel for point/colour) carries the stopwatch + ◄◆► navigator, driving
  the v0.9 keyframe ops (`effect_controls_panel.dart` `_FxKeyframeControls`).
- **.lumfx preset UI** — Effects & presets gains Save/Load preset, serialising
  through `save_effect_preset` (byte-compatible with `preset.rs`) to a file the
  user picks and appending a chosen `.lumfx` via `load_effect_preset`; the
  placeholder is gone (`effects_presets_panel.dart`, `file_dialogs.dart` preset
  seams). *Named remainder:* egui also LISTS saved presets above the categories
  (scanning `lumit_project::presets_dir()`); the bridge exposes save/load but no
  listing, so the browser row awaits a `list_presets`/`presets_dir` op.
- **Mask drawing with real geometry** — the Shape-tool drag maps its rect into
  comp pixels and commits `add_mask_geometry`, so the drawn size/position is
  honoured; the default-mask fallback is gone (`viewer_overlays.dart`).
- **Auto resolution tier** — the resolution picker gains Auto (egui's option
  set, overlays.rs:603); under Auto the preview renders at the realtime
  controller's live tier (`effectivePreviewScale`) and the transport reads the
  tier back, polled on the playback cadence (`viewer_panel.dart`,
  `AppStateStub.setPreviewAuto`/`pollPlaybackTier`).
- **Comp-strip popout wording** — the "pop out timeline" entry now explains the
  Timeline stays docked (it owns the transport + preview cache the panel split
  keeps in-window, 06 §E) rather than promising a future popout
  (`panels/timeline/comp_tabs.dart`).

## Blocked — awaiting engine/bridge capability, with evidence

Each row states the specific missing capability. None can land Dart-side without
it; landing a half-built version would drift the engine's behaviour, so they are
annotated honestly rather than faked.

**Section A — bridge caveats (landed with a named follow-up):**

- **Beat detection runs synchronously** in the bridge (`detect_beats` mixes the
  comp audio through the headless input builder and analyses in one blocking
  call the Dart side awaits off its UI isolate), where egui runs it off-thread
  (`detect_beats`/`poll_beats`). If long-audio latency bites, a start/poll pair
  like the export ops is the follow-up — the maths is identical, only the
  threading differs. **Not converted in the v0.9 wave** (it functions today; the
  conversion is a threading refactor, not a missing capability).

**Section B — performance follow-ups:**

- **Fence/keyed-mutex handshake for the shared texture** — only if the owner's
  live run shows tearing. **Verify on the owner's machine first**; not built
  speculatively. The shared texture presents without a producer/consumer fence
  today.
- **Footage probing off-thread** — the thumbnail half of this landed; the
  off-thread probe move did not. The bridge's synchronous probe cache is read
  *synchronously* by several ops — `convert_to_sequenced` and
  `trim_to_source_end` (source duration, `items.rs`), `add_footage_layer`
  sizing, and relink's sibling-missing check — so moving probing onto a worker
  needs those consumers to probe-on-demand or the ops silently degrade to their
  unprobed fallback. Named follow-up: a probe worker drained on
  `lumit_bridge_snapshot` polls (mirroring egui's `MediaRegistry::poll`) **plus**
  a synchronous `ensure_probed` fallback for the consumers above. **Not done in
  the v0.9 wave** (functions synchronously today; the worker + fallbacks are a
  threading refactor, not a missing capability).

**Section C — timeline and graph:**

- **Graph editor — the transform value graph and the Retime Time
  (source-position) lens** (`graph.rs:86-94`, K-078). The Flutter graph editor
  ports the Retime *speed* lens; the value graph for an animated transform
  property (curves from keys with draggable bezier value handles) and the Time
  (source-position) lens for Map segments remain a substantial unbuilt
  graph-editor build — deliberately NOT half-built in this wave, since a
  low-fidelity value curve (bezier segments drawn as straight lines) would drift
  from `graph.rs`'s real shapes. Its dependents ride that same build — the
  **lens picker in the header**, the **Vegas default-lens preference**
  (`graph.rs:164`, an `egui::Checkbox` the shell persists — verified it persists
  one, so it lands with the value lens), **boundary beat/frame snapping** on
  graph drags (`graph.rs:1616-1628`), and the **graph-key right-click interp
  menu** applying to value keys. The `evaluate`/`overrun_local_time` retime maths
  this build shares were ported to `graph_maths.dart` in the final UI wave (for
  the HOLD hatch), so the value-lens build inherits them.
  *egui-gap verdicts (04-RETIMING spec-only — egui never built them, verified in
  graph.rs and excluded from parity):* RATE/MAP **type chips** + ease-name labels
  (§9.4); **kink badges** (§6.1); **numeric % and t·s entry fields** (§9.3); the
  graph's **own overrun hatching** (§7.2 — egui hatches overrun only on the clip
  bar, `panel.rs:992`, which the clip-bar HOLD hatch now draws).

- **Effect-param keyframe lanes in the Timeline outline** — egui shows each
  layer's effect stack as an "Effects" group in the timeline outline with
  per-parameter rows and their keyframe lanes (`panel.rs:1602`, `effects_rows`).
  The Flutter timeline outline shows only the Transform group; effect keyframing
  currently lives in the Effect controls panel (stopwatch + navigator, landed
  this wave). Porting the effect group + parameter rows + lanes into the timeline
  outline is a larger outline build (the `PropertyRow`/lane machinery is
  transform-shaped) — a named remainder, not faked.

**Section D — editors, viewer and panels:**

- **Viewer transform gizmo — full manipulator.** The selected 2D layer draws an
  anchor crosshair, draggable to move its Position. The pan-behind anchor maths,
  the bounding box and the scale handles await the `LayerMap` (layer↔screen
  transform) port from `overlays.rs`.

**Section E — chrome and shell:**

- **Pop out a panel into its own OS window (multi-window)** — BUILT behind
  seams, pending on-machine verification (2026-07-22, re-attempt). The earlier
  block rested on two findings; the **second was wrong**, so the row reopened.
  - *SDK finding (stands).* The pinned stable SDK ships multi-window only as
    `_window.dart` — every symbol `@internal` (importing it fails `flutter
    analyze`), each API throwing unless `isWindowingEnabled` (a build-time flag
    OFF by default). Its own API is therefore still not used. Not fought.
  - *Community finding (corrected).* The old note said each window runs in its
    own engine/isolate with a **separate Dart heap** and concluded a popout
    could not reach the document. The heap fact is true; the conclusion is not.
    `desktop_multi_window` (0.3.0, Apache-2.0, MixinNetwork; no third-party deps,
    SDK `>=3.5.0` — compatible with `^3.12.2`) runs each secondary window as a
    second Flutter engine in the **same OS process** (engine-per-window;
    verified against the package's `window_controller.dart` + pub metadata, and
    the same in-process model `multi_window_native` uses). A popout does not
    need the main window's Dart objects — it needs the DOCUMENT, and that is
    process-wide: the popout opens its OWN `LumitBridge.tryLoad()` handle to the
    one already-loaded `lumit_bridge.dll`, reaching the same
    `static BRIDGE: OnceLock<Mutex<Bridge>>` (the exact fact `bridge.dart`
    already records for the render isolate: "same process, so the same engine
    state behind the bridge's process-wide `Mutex`").
  - *Built.* `lib/popout/`: `popout_arguments.dart` (panel + theme snapshot
    serialised across the window boundary, panel-split gate), `popout_app_state.dart`
    (`PopoutAppState extends AppStateStub` adding a public `resync` from the
    shared surface only — the file it extends is another agent's, untouched),
    `popout_host.dart` (theme from the snapshot, the panel body over the popout
    state, ~2 Hz snapshot poll, clean disposal), `popout_windows.dart` (the
    fake-injectable opener seam), `desktop_window_opener.dart` (the one file that
    touches the plugin; close detected by diffing `WindowController.getAll()` on
    `onWindowsChanged`), `popout_main.dart` (the sub-window entrypoint). Wired at
    `shell/shell.dart onPopOut` (float on open, re-dock on close) +
    `dock_widget.dart` (offer gated to hostable panels), `main.dart` (popout
    dispatch), and `windows/runner/flutter_window.cpp` (sub-window plugin
    callback).
  - *Panel split.* Offered for the read-mostly panels a second engine hosts
    honestly — Project, Hierarchy, Effect controls, Effects & presets, Scopes
    (Scopes renders pixels via the CPU render path, which works from any engine).
    The **Viewer and Timeline stay in-window**: the Viewer owns the shared-texture
    registrar (a per-view concern) and the Timeline owns the playhead/transport
    and the cache-bar warm set tied to the main preview — a second engine would
    fork that state.
  - *Staleness model (caveat).* The popout sees a main-window edit via its own
    ~2 Hz `resync` poll; its own edits reach the shared journal and self-refresh.
    The **main window** sees a popout's edit only on its next interaction —
    `AppStateStub` has no public resync and this agent does not own that file, so
    no main-window polling was added (documented, not faked).
  - *Verification (caveat).* The native plugin, the `main.dart` dispatch and the
    runner callback compile only in a real `flutter build windows` on the owner's
    machine. The `flutter analyze` / `flutter test` / `flutter pub get` gates
    could not be run in the implementing environment (no Dart/Flutter toolchain);
    tests are behind seams (a fake opener, a fake bridge) and never open a real
    window (`test/popout_test.dart`). Window sizing/title is a `window_manager`
    follow-up. Close the row once the gates run green on the owner's machine.

## Deferred, not blocked

- **Tooltip breadth pass — the remaining `on_hover_text` surfaces.** The shell +
  widgets tooltips landed; the remaining egui hover surfaces (layer switches,
  transport step/loop, the ruler, the scopes header) are optional cosmetic
  polish, not parity-blocking, and are unbuilt only by choice — deliberately none
  on menu-bar items, the splash, command-palette rows and dock tab pills (egui
  parity).

## Reconciled in 05

- (2026-07-22, section-A burn-down): the graph-lens "→Rate drift figure dropped
  by BridgeReply" remainder was stale — `driftSeconds` is threaded and the notice
  reads "fitted, N ms drift"; 05's F3 graph-lens named-remainder dropped the
  drift-figure caveat.
