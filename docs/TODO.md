# TODO

The backlog, most important first. One point per line, with a pointer to where the detail
lives. Delete a point when it lands, its regression test is the record.
[16-ROADMAP.md](16-ROADMAP.md) is the phase plan above it.

---

## 1. Bugs

- Linux Viewer resize can hand Dart a closed fd (`shared_linux.rs` `Drop`,
  `headless.rs` pool). Hold evicted targets one generation or `dup()` at export.

## 2. Performance

- `LumitAppNew` rebuilds the whole app on any `LumitUiState.notifyListeners`.
- Opaque footage and images can't hide the layers under them, because nothing knows
  whether a decoded source has an alpha channel. `lumit_media::VideoInfo` and
  `SourceProbe` would both need an alpha fact, and `occluder_index` the source facts. A
  precomp needs its own contents judged (`occlusion.rs`).
- An in-flight render can't be cancelled, and neither can the idle cache fill. In-render
  epoch tokens, docs/impl/playback-scheduler.md.
- Replace `poll(Maintain::Wait)` in every present with a keyed mutex (`shared.rs`,
  `shared_linux.rs`, `shared_metal.rs`). Measure first, own branch and pull request.
- Roto propagation is 895 ms a 1080p frame against 60 ms. WGSL ports, docs/impl/roto.md.
- The Windows embedder re-records the whole window every frame. File the drafted upstream
  issue and correct §7 item 1's MSAA diagnosis (docs/impl/ui-performance.md §7.1, §7.2).
- Each Flutter upgrade, re-run the backend A/B and flip `ImpellerSwitch::Disabled` in
  `windows/runner/main.cpp` once Impeller holds 60 fps. Standing.
- Flow synthesis spends about 70 of 79 ms on luma conversion and uploads. Keep decoded
  frames on the card.
- The flare's Matte mode dispatches all 16 light slots. Measure the live fraction, then
  dispatch indirectly.
- The Scopes trace is a fixed 256x256 CPU image. Use the shared texture, sized to the panel.
- The matte render-alone pass stays at full comp resolution.
- The audio mix is rebuilt from scratch when its signature changes (`prepare_once`).
- frb's SSE codec encodes `Vec<u8>` a byte at a time (thumbnails, scope traces).
- Bridge reads outside the read model: Source card text, source item, mask and
  interpolation reads, the Viewer's missing-file probe, the comp's marker and work area.
- `FlowRowsFrb.build` reads four flow getters in build.
- The worker's loaded-frame return and prefetcher channels are unbounded and uncounted.

## 3. Features

**Node graph** (docs/impl/node-graph-comp.md §8)
- P4: Save group and the saved groups in the canvas console, the picture Input's preview
  picker, the no-stream word on box rows. The bridge calls exist, only copy uses one.
- P5: placed graph Inputs in Effect controls and a Timeline fold, dimmed collapse and audio
  cells, the Retime clock face. `use/node-graphs.mdx` already describes these.
- More drivers: Colour ramp.
- The effects console rows show a placeholder gradient instead of preview thumbnails.
- A point row can't show it's driven (`EffectPointRowFrb`).
- Spare parameters, and the Sync and Remove row for derived parameters
  (docs/impl/custom-shader.md CS3).

**Viewer and Timeline**
- Making a layer 3D should show a 3D options group with Accepts lights in it (After
  Effects' Material Options).
- Path editing: drag bezier handles, a linked/broken tangent flag (docs/03 change), Pen
  add/delete/convert vertex, drag a path by a segment, a paint stroke's points.
- Mask paths need a per-key op instead of `SetLayerMasks` rewriting the list.
- Mask Feather tool (`penMaskFeather` is a stub), with feather points by arc length.
- Motion-path handles can't be dragged, the model has no spatial tangent (docs/03 §6.5).
- Scale and rotate a multiple selection about one shared box.
- Snap to other layers' edges and centres (`viewer_snap.dart`).
- Gizmos ignore a layer's parent and the camera.
- On-Viewer handles for point parameters, Bezier warp and Corner pin.
- Camera: depth-of-field handles, the iris, dragging a keyframed camera, one undo step for
  a drag across layers (docs/impl/camera.md).
- Separate the overlay menu (motion paths, mask paths, gizmos) and add the full wireframe
  mode (docs/07 §2.2 item 5).
- The degradation reading should name what it skipped (docs/07 §2.2 item 9).
- Graph editor zoom and auto-fit should use `SmoothZoom`.
- Smooth edge-follow with a setting, and `Shift+=` zoom to the work area (docs/07 §4.6).
- Beat tap: `/` during playback drops a beat marker at the playhead (docs/07 §10). It sits
  beside numpad `*`, which adds a marker, and is free on a laptop too.
- Shape layers: nested groups, wiggle, gradient stop lists, joins and caps other than round.
- Type: vertical type, real glyph metrics across the bridge, multiple lines, a character
  panel.
- Paint: tilt, spacing and scatter, Start/End curve in the graph editor, painting in Layer
  view, GPU stamping (docs/impl/paint.md).
- Unbound chords: `,` `.` previous/next keyframe, `Ctrl+,` `Ctrl+.` edit points, `K`
  shuttle pause.
- Menu rows still marked Not implemented: Preserve transparency, Auto-outline, Layer ▸
  Reveal, Track motion.

**Panels and dialogues**
- Dragging on the Curves graph lags.
- Double-click still waits in 11 places. Move them to `DoubleTap` (Hierarchy rows etc).
- Nothing says when a proxy file itself is broken (`BridgeProxy`).
- The opening card sits at 0% while the file is read. Count bytes over the unzip.
- Tone mapping has no explanation anywhere on screen.
- Settings: CUDA on/off, the plugins and decoder page, a Show shortcut hints switch.
- Settings sliders should wear their own face (2px track, primary knob, no fill).
- The Chrome labels setting is only read by the Timeline toggles. Icons everywhere has no
  reader (docs/15 §5.1).
- Themes: swatches on each row of the picker menu, since the strip beside it shows only
  the selection. And somewhere to keep themes besides the workspace file.
- The boot splash needs an engine boot event stream.
- First-run setup's four-card version (docs/07 §13.1).
- A Welcome screen redesign, done with an optional first-run tutorial that explains
  Lumit. Needs a proper design first.
- Timeline column widths and the property selection should live in the workspace.
- An autosave doesn't refresh the welcome picture. Send an autosaved event, the frontend
  draws it.

**Export**
- List the codecs installed on the machine instead of a fixed format list
  (`lumit-media/src/encode.rs`).
- The export queue has no last frame preview, and closing Lumit mid-export doesn't warn.
- A one-frame image sequence should be `shot.png`, not `shot.00001.png`.
- Export priority, encoder preference order, and reframe with a draggable centre crop or
  pillar fit (docs/06 §7).
- Export status is JSON over a poll. Move it to a typed stream.
- Export progress should share the preview progress path, and those fractions should use
  measured costs.

**Audio**
- docs/09 says audio must never pause for video, but Every frame holds the sound when a
  picture runs late, which is what it should do. The spec is the side that changes.
- Scrub audition and its Timeline toggle, the replace-or-merge offer when beats are
  detected again, and peak files on disk (docs/09).
- A device change isn't picked up live (`audio::devices`, `set_device`).
- The Sound mix waveform strides past 256 frames a bucket, a transient can drop out.
- Retime boundary drags should snap to beats, and
  `retime.quantise_boundaries_to_beats` has no caller (docs/04 §12.3). Timeline drags
  already snap to beat markers.

**Retime** (docs/04-RETIMING.md)
- Retime UI: Hold preset, kink badge, overrun band and source-out line, compensating
  Alt-drag, outward trim extends the map, retime shortcuts, source-rate advisory badge.
- `Clip::trim_to_source_end` has no bridge call, and the clip menu has no Trim to source
  end (docs/07 §4.4).
- Flow under an adjustment layer can't see footage move, a re-render decodes nothing
  (docs/impl/temporal-rerender.md).

**Tracking** (docs/impl/tracking.md)
- Tracking a sequence only reads its first frame (`MediaLuma::open`).
- Tracking workspace, linking an existing Camera layer, cloud placement that follows the
  layer's transform, and cloud tools (count, filter, delete, ground plane, origin).
- Solve notes and a typed focal should cross the bridge (`SolveNote`, `focal_px`).
- A rotation-only product for a nodal pan.
- Lens distortion (k1, k2).

**Effects**
- Lens flare: image aperture, lens designer, Occlusion layer, grid refinement at vignette
  folds. A point row's keyframe toggle still costs two undo steps.
- Particulate depth occlusion (docs/research/particles.md §5).
- A tone mapping effect for export (docs/08 §3).
- The shader editor uses VS Code's palette, hardcoded bracket colours, and `buildTextSpan`
  drops the style.
- A shader as a project item, so it relinks like footage (needs a non-footage item kind).

**Translations and manual**
- Translations owed: English 3,013 keys, German 864 short, Kazakh 1,762, Ukrainian 1,557,
  both Chinese 801. Spanish, Polish, Arabic and Portuguese are empty.
- Fill technical names (sRGB, H.264, LUFS) and number separators for every language.
- The two numbered marker shortcut labels stay English (`format!` in `lumit-keymap`).
- Cull the unused `settingsHelpChromeLabels`.
- 16 effect pages missing: the 15 Audio effects and Extract channels. `npm run
  docs:effects`, then place the Audio section and write its prose by hand.
- Six screenshots: camera-track, planar-track, project-settings-colour,
  viewer-colour-menu, text-animators, shape-combine.
- The Viewer pictures may be stale after the 2026-09-10 camera rework. Check the export
  page's picture too.
- Lens flare's example picture: lift its skip in `effect_examples.rs` and re-run.
- Clone to points, Trail, Connect points and Node graph have no example pictures. One
  graph in the example project fixes all four.
- `use/effects.mdx` says plugin hosting isn't built.
- Proofread the pages outside effects. Ask before changing wording.

## 4. Engineering, CI and platform

- The rebuild budget tests' shared mount asks for an effect called `sharpen`, which the
  engine doesn't register, so it silently mounts four effects where the comment says five.
- Big files: 103 over 1000 lines, 14 over 4000. Split where there's no reason not to.
- docs/14 tooling: fuzz targets for the `.lum` reader and journal replay, edition 2024,
  `indexing_slicing` and `arithmetic_side_effects` denies, `clippy::pedantic`, a
  golden-frame EXR corpus.
- Thin-view debts: `comp.activeCameraPose`, `layer.textMetrics`, the mask path pair on the
  selection, engine-side draft ops for the shape tool's Ctrl+Z, `LumitTheme.copyWith` for
  tokens, one present-pool body in `headless.rs`, typed `ExpressionContext::comp_time`.
- Bridge: a panic throws instead of reporting, clippy can't see `#[frb]` functions, and
  `ProjectReference::state()` hands out the raw lock.
- `deny.toml` ignores: move `ttf-parser` to `skrifa`. bincode, paste and smartstring
  leave with their parents.
- An FFmpeg-free build needs `lumit-media` optional in render and audio.
- Performance gates: B1 needs a reference-hardware runner, the ui-budget job is
  continue-on-error until it goes green, B12 to B14 aren't in the baselines, and B8, B9,
  B10 stay manual (docs/13).
- Also a floor-class runner, a 4K 20-layer stress comp, and per-effect cost-class
  benchmarks.
- Render-time column: GPU timestamp queries, sorting, a profiler panel, per-layer numbers
  inside a Precomp (docs/13 §7.1).
- CI: macOS and Windows don't set `LUMIT_REQUIRE_GPU`, nine Viewer tests skip on Linux so
  nothing proves a frame arrives, and texture registration is only tested by hand.
- Shared textures: no keyed mutex, no fence on Linux and macOS, and the D3D12 to D3D11 hop
  isn't in docs/06.
- macOS: the FFmpeg 8 action is a stopgap until Homebrew ships `ffmpeg@8`, the build is
  single architecture, and the IOSurface and Metal paths are unproven.
- macOS pass: VideoToolbox, ProRes, `application:openFile:`.
- The iOS podspec is misnamed (`rust_lib_lumit_flutter`).
- A Flatpak remote so `flatpak update` works.
- One-copy D3D11 to DX12 decode interop, and ProRes/DNxHR export (docs/05 §6).
- File format: embedded `thumbs/`, sidecar `proxies/` and `peaks/` (docs/10).
- Design tokens: the missing type-scale steps, Shape and Null layer colours (docs/15 §6.1).
- The export disk-cache setting has no disk tier to govern.
- Exact Rust ports of the eight vendored OCIO styles, to drop the 0.117 Rec.709 blue error
  and the 74 MiB of artefacts (docs/impl/ocio.md §4.1).

## 5. AE import (docs/impl/ae-import.md)

- Orientation and rotation on one layer are only reported. Spatial tangents on Position
  flatten.
- Ten match-name rows are unaudited (`pending_audit`), and Turbulent displace's Pinning
  needs the option strings.
- The collected `footage/` copy and its hash check (docs/11 §2.5).
- Report rows should lead to their property, and the report should be kept (docs/11 §9).
- Parser: corpus testing across AE versions, `btds` text and `GCst` gradients, the other
  arbitrary-data blobs, shape and text depth.
- Parser: the project-level `LIST EfdG` fallback, a mask path's linear speed.

## 6. Later

- Multi-window, blocked upstream until windowing reaches stable
  (docs/impl/multi-window.md). Welcome, dialogues, export queue and tear-off panels follow.
- The Hierarchy panel's graph view and an indent/graph switch.
- LFX plugins, Lottie import and export, the stabiliser, Blender scene import,
  OpenTimelineIO, a render CLI (docs/16).
- Copying a preset into the library folder so the listing finds it (applying one from
  anywhere already works). Per-layer motion blur polish.
- Parked: Choke, Inner glow and Inner shadow as effects. Slitscan, Dither,
  Draw Glass, aperture shapes. A details inspector in the Source card.
- Flow research: WAFT-class learned flow, a blended census and SSD cost, line art on the
  worst blocks (docs/impl/optical-flow.md §5.5).
- Effect parity limits (docs/impl/ae-effect-parity.md): mask sets, 512 path pieces, four
  Lightning types and no Alpha Obstacle, Beam in 3D, Radio waves taper, Vegas contour
  segments, Card wipe camera, back layer and gradient order, Median past 3, Texturize
  native size, a high-detail proof fixture, Shadow highlight auto and two radii, Warp Axis,
  five Fractal noise controls.

## 7. Needs a decision

- `UU` opens every modified row. Keep it, or go back to the old reveal of animated groups.
- A node's "at" toggle forgets itself when Lumit closes. Saving it would dirty the file.
- What a point pair with only one half driven looks like.
- Can you send `lumit-diagnostics.log` from Temp after an idle device loss, two Particulate
  effects freezing the preview, and a freeze on changing the Viewer fit? Does the
  Particulate freeze still happen after the retime curve fix?
- Is dragging the Timeline panel's height still laggy on your machine now it ships on Skia?
- Does a panel seam drag feel slow enough to be worth profiling?
- The drag multiplier popup wears the hint pill’s face. What should its own look be, and
  is there a drawing for it?
- What should the ramp preset shelf be on the Retime property, and where does it sit,
  before Slow, Fast, Smooth and Sharp come back?
- Now an OCIO colour space transform can sit either side of a LUT, does the LUT still need
  log input spaces of its own?

- AE import questions are parked for now: roving keys, golden-frame renders, the four
  owed fixture rows, and whether a table of After Effects' property names ships.