# 03 — Architecture: Flutter over the Rust engine

## In plain terms

Today one program does everything: the Rust process opens a window, egui draws
the interface into it, and the same process decodes, composites and exports.
With Flutter, the window and everything drawn in it belongs to Flutter (which
runs Dart code), while the engine stays Rust. The two halves talk over a
*bridge*: Dart calls Rust functions as if they were Dart functions, and Rust
streams results back. The one place that needs more than function calls is the
Viewer — video frames are far too big to copy through a function call per frame,
so the engine draws them into a piece of GPU memory that Flutter displays
directly.

## The layering

```
flutter_ui/  (Dart)          — widgets, layout, theme, input, dialogs
     │  flutter_rust_bridge (generated FFI)
crates/lumit-bridge  (Rust)  — the API surface: commands in, events/state out
     │  plain Rust calls
crates/lumit-core, -eval, -media, -audio, -cache, -gpu, …  (unchanged)
```

- `lumit-bridge` is a **new** crate (Phase F1): a cdylib the Flutter Windows
  runner loads. It owns the engine-side state that `lumit-ui`'s `AppState`
  owns today; `lumit-ui` remains untouched so the egui frontend keeps building.
- Engine crates never depend on the bridge or on Flutter — the docs/05
  dependency rule holds unchanged.
- Long-running work (decode, export, beat detection) already lives on worker
  threads with channels; the bridge exposes those as Dart `Stream`s
  (flutter_rust_bridge's `StreamSink`), which is the same latest-wins pattern
  K-170 documents.

## Commands and state

The egui frontend mutates `AppState` directly each frame. Flutter cannot — the
document lives in Rust. The port keeps one honest boundary:

- **Commands down:** every user action becomes one bridge call (`op` dispatch
  mirrors `lumit_core::ops`, so undo/redo journalling is untouched).
- **State up:** the bridge publishes coarse-grained, versioned snapshots
  (project tree, comp outline, selection, transport) over streams; Dart holds
  them in `ChangeNotifier`s the widgets watch. Fine-grained per-frame data
  (playhead position during playback) rides its own lightweight stream.
- Rule of thumb from 14-ENGINEERING-RULES: typed rational time crosses the
  bridge as `{num, den}` pairs, never as floating seconds.

### Bridge v0

Phase F1 does **not** start with `flutter_rust_bridge`. It starts with **bridge
v0: a hand-rolled JSON-over-C-ABI seam** — plain `extern "C"` functions in
`lumit-bridge` that Dart calls over `dart:ffi`, exchanging UTF-8 JSON strings.
`flutter_rust_bridge` remains the target once the API surface stabilises; v0
keeps the toolchain simple and testable (no codegen step, no build-runner) while
the shape of the commands and snapshots is still being found. The parity
checklist's codegen row stays open until then.

The contract v0 pins, unchanged when codegen replaces it:

- **No panic crosses the boundary.** Every exported function's body runs inside
  `std::panic::catch_unwind`; a panic becomes an ordinary
  `{"ok":false,"error":"…"}` reply, never an unwind into Dart
  (14-ENGINEERING-RULES). Every reply is either `{"ok":true, …}` or
  `{"ok":false,"error":"…"}`, the error a calm sentence for the status line.
- **Rust owns the strings.** Each function returns a Rust-allocated,
  NUL-terminated UTF-8 pointer; Dart copies the bytes out and immediately hands
  the pointer back to `lumit_bridge_free_string` so Rust frees it. Dart never
  frees Rust memory itself, and Rust never reads a freed pointer.
- **One client, one lock.** The engine-side document and its undo store live
  behind a single process-wide `Mutex` (there is exactly one Flutter window),
  held only for the duration of one state transition, never across re-entry.
- **Absent library ⇒ placeholders.** Dart's `LumitBridge.tryLoad()` returns null
  when the `.dll` cannot be found or bound, and the whole frontend (and every
  Flutter test) keeps its F0 placeholder behaviour. The bridge is an
  enhancement, never a hard dependency of the chrome.

#### Bridge v0.2 — the data the Viewer, Timeline and editors need

v0.2 extends v0 without breaking it (the ABI number rises 1 → 2; every reply
keeps the fields it had):

- **Snapshot v2 is strictly additive.** Each composition item gains a `comp`
  block (`width`, `height`, `fps` as the model's exact `{num, den}`,
  `frame_count`, `layers`, `markers`); each footage item gains a `status`
  (`ok` / `missing` / `unprobed` / `failed`) and, once probed, a `media` block
  (`duration_frames`, `fps`, `width`, `height`, `audio`). Frames are integers
  derived from the composition's *own* frame rate on rational time (a layer's
  `in_frame`/`out_frame` is the frame containing its in/out point; `frame_count`
  is duration ÷ one frame, rounded) — f64 time is never threaded across the
  seam. Layer `kind` and `switches` mirror `LayerKind`/`Switches` name-for-name.
- **New ops, one undo step each.** `set_layer_switch`, `edit_layer_span`,
  `set_transform` and `add_marker` each map onto the real, unit-tested
  `lumit-core` op (`SetLayer*`, `SetLayerSpan`/`edit_layer_span`,
  `SetTransformProperty`, `SetCompMarkers`), so undo/redo is one clean step and
  the reply is the full refreshed snapshot. Switch and transform names are the
  model's own field names.
- **The binary frame-buffer contract.** `decode_frame(item_id, frame, out_w,
  out_h, out_len) → *mut u8` is the one call that does **not** return JSON: a
  video frame is far too large to encode as text. It returns a Rust-owned block
  of tightly-packed RGBA8 (null on failure, with the out-pointers zeroed) and
  writes the frame's width/height/length into the out-pointers. Dart copies the
  pixels out and hands the pointer **and its exact length** back to
  `free_buffer` — the mirror of the string contract, one boxed slice freed as a
  whole. This is the F2 CPU path; the shared-texture path stays future work.
- **The `media` feature gate.** Probing and decoding live behind a default-on
  `media` cargo feature that pulls `lumit-media` (FFmpeg). `--no-default-features`
  drops it entirely — the crate still builds and tests without FFmpeg (CI
  parity), footage simply reports `unprobed` and `decode_frame` returns null.
- **Synchronous probe caveat.** At this phase footage is probed *synchronously*
  on import and on open (building/loading the frame index on the calling thread),
  where the egui frontend probes on a background thread. Acceptable while the
  first files are small and imported one at a time; the bridge will move probing
  off-thread once the command surface stabilises.

#### Bridge v0.3 — read-back and the ops that unblock the port

v0.3 extends v0.2 without breaking it (the ABI number rises 2 → 3; every reply
keeps the fields it had). The state transitions split into a second module
(`crate::edits`) so `crate::state` stays under length; the snapshot builder
gains the read-back below.

- **Snapshot v3 read-back (additive).** Each layer gains a `transform` block —
  one entry per property (`anchor_x`…`opacity`) shaped `{value, animated,
  keys?}`, where `value` is the static value or the value evaluated at layer
  time 0, and `keys` (only when animated) is `[{frame, value, interp_in,
  interp_out}]` with the `SideInterp` variant names (`Hold`/`Linear`/`Bezier`)
  and the keyframe's comp frame. The seeded values (e.g. position at the comp
  centre, from `lumit-ui`'s `centred_transform`) are already in the read-back,
  so it *is* the true current value — no separate defaults block. Each layer
  also gains its identity link (`source_item_id` for footage, `source_comp_id`
  for precomp, `colour` for a solid — resolved from the `SolidDef` asset) and an
  `effects` array (`[{id, name, enabled, params:[{name, kind, value}]}]`; scalar
  and colour params carry an evaluated value, exotic kinds a null). Each comp
  gains `work_area` as `[in_frame, out_frame]` or null.
- **Layer lifecycle ops.** `add_solid_layer`, `add_text_layer`,
  `add_camera_layer`, `add_adjustment_layer`, `add_sequence_layer`,
  `delete_layer`, `duplicate_layer` — each mirrors the egui add/duplicate/delete
  path exactly (name, size, colour, span, centred transform), through
  `AddLayer`/`RemoveLayer` (the solid is one `Batch`: Solids folder + asset +
  layer).
- **Comp settings.** `set_comp_settings(comp, name, w, h, fps_num, fps_den,
  duration_frames)` commits one `SetCompSettings` (the background preserved), so
  undo is one step — as `confirm_comp_dialog` does.
- **Keyframes.** `toggle_property_animated` is the stopwatch (seed a key at the
  playhead on enable, collapse to static on disable); `add_keyframe`,
  `remove_keyframe`, `shift_keyframes(frames_json, delta)` mirror `upsert_key`,
  the collapse-on-last-delete, and the lane's `shift_keys_time`. All route
  through `SetTransformProperty` with the whole animation (coarse + invertible).
  Frames are comp frames; the bridge maps them to layer-local time the way the
  egui frontend does (`frame / fps − start_offset`).
- **Work area.** `set_work_area_edge(comp, frame, is_out)` mirrors the B/N keys
  (`SetWorkArea`), clearing to null when the span covers the whole comp.
- **Effects.** `list_effects()` returns the registry (`[{name, label}]` from
  `lumit_core::fx::BUILTINS`, stateless); `add_effect` (via
  `instantiate_for_raster`), `remove_effect`, `set_effect_enabled`,
  `set_effect_param_scalar`, `set_effect_param_colour` all commit
  `SetLayerEffects`. Point/file/layer param kinds are read-back only in v0.3
  (no setter yet).

#### Bridge v0.4 — export, Retime, and the last timeline columns

v0.4 extends v0.3 without breaking it (the ABI number rises 3 → 4; every reply
keeps the fields it had). Two new modules keep `crate::state`/`crate::edits`
under length: `crate::columns` (blend/matte/parent/motion-blur/mask ops) and
`crate::retime` (the Retime read-back helpers live in `crate::snapshot`, the ops
in `crate::retime`). Export lives in `crate::export`.

- **Export over the headless seam (K-175, K-017).** The bridge reuses
  `lumit_ui::export` — the *identical* exporter the egui app runs — through the
  headless seam: `HeadlessRenderer::export_inputs(doc, comp)` builds the footage
  `ItemInfo` map (reusing the renderer's probe cache), collects the comp's audio
  jobs (the headless twin of `AppState::comp_audio_jobs` — solo gate, precomp
  carriers), and lends a GPU context sharing the renderer's device. The bridge
  hands those to `lumit_ui::export::start`, which spawns its **own encode
  thread** (K-017) and streams `ExportEvent`s over an mpsc channel. The bridge
  holds the `ExportHandle` (its receiver) behind a session static and drains it
  on each poll.
  - `lumit_bridge_start_export(comp_id, spec_json, out_path)` → `{ok:true}` on a
    clean start, or `ok:false "an export is already running"` while one is in
    flight (the Dart side queues). `spec_json` mirrors the export dialogue:
    `{preset, codec, size, bitrate_mbps, include_audio, audio_bit_rate}`.
  - `lumit_bridge_export_poll()` → `{ok, state:"idle|running|done|failed",
    frame, total, encoder, path/error}` — the drained progress.
  - `lumit_bridge_export_cancel()` asks the running export to stop (checked every
    frame); poll then reports `failed` with "cancelled".
  - `lumit_bridge_export_preset(preset, comp_name, template)` is the **pure**
    preset resolver, a faithful port of `ExportDialogState::apply`/`spec` and the
    filename helpers: it stamps the preset's codec/size/bitrate, applies the
    VBR-peak-preserved-while-unedited rule and the 1.5× fallback, and renders the
    `{comp}`/`{preset}`/`{date}` filename template (Windows-sanitised, `.mp4`
    forced; a blank template reproduces the preset's own default byte-for-byte,
    K-119). The pure resolver and filename logic are always compiled and
    unit-tested; the driving surface is gated behind the `render` feature (a
    `--no-default-features` build answers "export is unavailable").
- **Keyframe interpolation.** Snapshot keyframes gain `bezier_in`/`bezier_out`
  (`{speed, influence}`) on a `Bezier` side; `set_keyframe_interp(comp, layer,
  property, frame, interp_in, interp_out, +bezier params)` sets the interp of the
  keyframe nearest the playhead through `SetTransformProperty` (whole animation).
- **Retime read-back + ops.** A footage layer's `retime` block serialises the
  store: `{reverse, interpolation, boundaries:[{t_frame, t_seconds, s_seconds,
  smooth}], segments:[{kind:"rate", v0, v1, ease} | {kind:"map", m0, m1, b0,
  b1}]}` (boundary local times as comp frames, source positions in seconds,
  ease/kind names mirroring `lumit_core::retime`). Ops (all `SetLayerRetime`):
  `set_retime_enabled` (identity store / clear), `set_retime_speed` (constant
  speed; 100% clears — the simple speed row), `set_segment_preset` (the
  Lin/Slow/Fast/Smth/Shrp row, `with_segment_ease`), `segment_to_rate` (the
  →Rate button, `with_segment_as_rate`, with the fit `drift` added to the reply
  snapshot), `drag_boundary(index, frame)` (move a value-lens boundary,
  `from_value_keyframes`). Only what the egui speed row / graph header commit
  today.
- **The last columns.** Each layer gains `blend_mode` (serde variant name),
  `matte` (`{source, channel, inverted, source_mode}` or null), and `parent` (a
  layer id or null); each comp gains `motion_blur` (`{enabled, shutter_angle,
  shutter_phase, samples}`). Ops: `list_blend_modes` (registry, stateless),
  `set_blend_mode`, `set_matte` (empty source clears), `set_parent` (empty
  clears; a cycle is a calm error), `set_motion_blur` (`SetCompMotionBlur`), and
  `add_mask(comp, layer, kind)` (`rectangle`/`ellipse`/`star`, the "Add mask"
  menu's centred starter shape).
- **Session restore.** Nothing engine-side is needed: which comps are open and
  where the playhead sits are Dart-owned state, so `SavedSession` stays a
  frontend concern (confirmed for this wave).

## The Viewer texture path (Phase F2) — implemented, K-177

Windows first, matching the project's priorities. This closes the recorded top
performance gap (K-176): today's Viewer makes a per-frame round trip — render on
the GPU, read the pixels down to the CPU, copy them across FFI, upload them back
to the GPU. The zero-copy path removes it. It is an **opt-in `shared-texture`
feature**, off by default so every existing build and CI gate is unchanged; the
owner builds the shipped `.dll` with `--features shared-texture` (see below).
The friend's diagnosis was exactly right — the readback/re-upload was the cost.

The route actually shipped (the D3D12-direct one, not a separate D3D11 device):

1. The engine renders the composited frame with wgpu exactly as today
   (preview == export == Flutter unchanged, K-031).
2. wgpu runs over **D3D12**. The headless renderer reaches through wgpu to its
   D3D12 device (`Device::as_hal`), creates the display target in a **shared
   heap** — a `DXGI_FORMAT_R8G8B8A8_UNORM` texture with `D3D12_HEAP_FLAG_SHARED`
   and `ALLOW_SIMULTANEOUS_ACCESS` — and exports an **NT handle**
   (`ID3D12Device::CreateSharedHandle`). The same D3D12 resource is wrapped back
   as a `wgpu::Texture` (`create_texture_from_hal`), so the normal render path
   copies the finished, display-encoded frame into it (a valid srgb-differing
   `copy_texture_to_texture` — the bytes are the identical sRGB-encoded pixels
   the read-back path produced). `crates/lumit-gpu/src/shared.rs`.
3. The bridge hands the handle across (`lumit_bridge_render_to_shared` →
   `{handle, width, height}`, no bytes). The Windows runner registers it with
   Flutter's texture registrar as a
   **`kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle`** external texture — the
   embedder opens the NT handle itself on its own ANGLE/D3D11 device, so the
   runner holds no D3D device of its own. `windows/runner/viewer_texture_bridge.
   {h,cpp}`, over the `lumit/viewer_texture` method channel; the Dart lifecycle
   owner is `ViewerTextureController`.
4. Dart shows it with a `Texture(textureId: …)` widget inside the Viewer panel;
   the pasteboard around it is `viewer_surround`, drawn by Flutter. Frame pacing
   stays engine-side; Flutter just marks the texture frame available (`frameReady`).

Why D3D12-direct rather than a dedicated D3D11 device (the documented
alternative): it is self-contained (no second device, no D3D11-on-12, no
cross-API copy) and it was verified end to end on the dev machine — the
`solid_comp_renders_to_a_stable_shared_handle` test creates the shared resource,
exports a non-zero handle, and re-uses it across frames on the real adapter. Under
the feature the renderer pins the D3D12 backend (the interop needs it); every
other build keeps the all-backends instance.

**Synchronisation.** After the copy the renderer `poll(Wait)`s so Flutter never
samples a half-written frame — zero CPU pixel work still, the bytes never leave
the card. The texture is re-used each frame (a stable handle; a comp resize makes
a new one), so a keyed-mutex / shared-fence handshake is the recorded follow-up,
worth adding only if tearing shows in practice (D3D12 uses fences, not keyed
mutexes, so a cross-API handshake is non-trivial and deferred until observed).

**No new dependency.** The embedder plumbing (descriptor shape, the DXGI
shared-handle surface type, the register / mark-frame-available dance) follows
the MIT-licensed `flutter_wgpu_texture` package as a *reference* — pattern
borrowed with a code-comment credit, not added as a dependency (it owns its own
renderer/scene architecture, and is young). The `windows` crate is pinned to
0.58 so its D3D12 types unify with the ones wgpu-hal already uses.

**The read-back path REMAINS as the airtight, automatic fallback.**
`lumit_bridge_shared_supported()` is false for an old `.dll`, a non-Windows
build, or a feature-less build; `render_to_shared` returns false (Dart falls back
for that frame) on no D3D12 adapter or any interop error; a missing platform
channel (an unwired runner) latches the controller off for the session. Every
seam degrades to today's RGBA-readback path (the bridge hands RGBA bytes and Dart
blits them through a `ui.Image`), each covered by a fake in the Dart suite. The
**Scopes** still need CPU pixels (the texture path moves none): a throttled
read-back render (~10 Hz, `PreviewSource._maybeScopeRender`) feeds them while the
texture drives the Viewer.

**Building it.** The shipped `.dll`:
`cargo build -p lumit-bridge --release --features shared-texture`. The runner
plugin (`viewer_texture_bridge.cpp`) compiles only under `flutter build windows`
on a real machine; it was written against the actual `flutter_windows` /
texture-registrar headers but cannot be compiled in the docs-first sandbox.

**Linux has the same path via DMA-BUF (K-177).** The zero-copy Viewer is no
longer Windows-only. Behind the sibling opt-in feature `shared-texture-linux`, the
engine (on Vulkan through wgpu-hal) creates an exportable `VkImage`, exports a
DMA-BUF fd (`vkGetMemoryFdKHR`), and the GTK runner
(`linux/runner/viewer_texture_bridge.{h,cc}`) imports it into a GL external
texture via `EGLImage`/`EGL_EXT_image_dma_buf_import` inside an `FlTextureGL`
subclass. It uses the **same `lumit/viewer_texture` channel protocol** as Windows,
but `register` carries `{fd, width, height, stride, offset, fourcc, modifier}`
instead of an NT handle; the bridge exposes it through a separate export
(`lumit_bridge_render_to_shared_dmabuf`) so the Windows ABI is untouched, and the
Dart controller branches the `register` payload by platform. DRM story: RGBA8,
linear tiling, `DRM_FORMAT_ABGR8888`, `DRM_FORMAT_MOD_LINEAR`. Like the Windows
plugin, the GTK plugin compiles only under `flutter build linux` on a real
machine; CI compiles both halves (`cargo check --features shared-texture-linux` +
`flutter build linux`), and runtime verification is the Linux collaborator's
(GUIDE §9).

This is the only part of the port with real platform-specific plumbing; it is
why the Viewer is its own phase.

### The composited-comp seam (implemented, CPU path) — K-175

The Viewer's first working picture decoded ONE footage layer
(`decode_frame`) — no transforms, blends or effects, because the real
compositor lived only inside `lumit-ui` (the offscreen render `export.rs`
drives, the pixels the egui Viewer shows). That was the port's biggest missing
piece. It is now closed on the CPU path:

- **The seam.** `crate::export`'s `Renderer` (in `lumit-ui`) is already
  window-free and egui-free — it needs only a `lumit_gpu::GpuContext`,
  `lumit-media` decoders and `lumit-core`; it composites a comp at time `t` into
  a linear texture (`render_comp_linear`), which `ColourEngine::display` +
  `readback8` turn into RGBA. `lumit-ui` gains a small `pub mod headless`
  (`src/headless.rs`) wrapping that path in a reusable `HeadlessRenderer` that
  **owns** the GPU context (adapter acquired once), the compiled shader engines,
  a decoder pool and a probe cache, and lends them to a fresh `Renderer` per
  call: `render_rgba(&Document, comp, frame, scale) -> (Vec<u8>, w, h)`. It is
  the same code export runs, so **preview == export == Flutter** (K-031). The
  only change to `export.rs` is visibility (the `Renderer` and its fields became
  `pub(crate)`); no behaviour moved.
- **The bridge.** `lumit-bridge` gains a default-on `render` feature that pulls
  `lumit-ui` and holds ONE session-lifetime `HeadlessRenderer` behind its own
  lock (separate from the document lock, so a slow render never blocks an edit).
  `lumit_bridge_render_comp_frame(comp_id, frame, scale, out_w, out_h, out_len)
  -> *mut u8` returns a Rust-owned RGBA block with the exact
  `decode_frame`/`free_buffer` ownership contract (null + zeroed outs on
  failure, `catch_unwind` at the boundary). A machine with **no GPU adapter**
  resolves the renderer to a calm terminal `Failed` state on the first call and
  returns null on that and every later call — never a crash, never a retry storm.
  Without the `render` feature the symbol is present but always returns null.
- **The dependency edge.** `lumit-bridge` depending on `lumit-ui` is the
  deliberate, temporary architecture **K-175**: *the bridge borrows lumit-ui's
  renderer through the headless seam until the pixel pass moves into an engine
  crate.* The docs/05 rule (engine crates never depend on a frontend) is
  unbroken — the bridge is a leaf, not an engine crate.
- **`scale`.** 1.0 is the comp's own resolution; a smaller positive value
  downsamples the OUTPUT buffer (a cheaper blit) but not the internal render —
  the export compositor has no cheap reduced-resolution target, so the GPU cost
  is unchanged. A future reduced-resolution preview render would change that.
- **Dart.** `bridge.dart` adds a separate `CompRenderBridge` capability
  interface (kept off `DocumentBridge` so the many `implements DocumentBridge`
  fakes need no change) with `supportsCompRender` + `renderCompFrame`. The render
  symbol is bound *defensively*: an older `.dll` lacking it leaves the capability
  false rather than failing the whole load. `preview_source.dart` prefers the
  comp path when the bridge advertises it, rendering the WHOLE comp via
  `renderCompFrame` and falling back — per frame — to the single-layer decode
  when a render returns null (no adapter, transient failure). A missing layer
  inside a comp is slated as colour bars **inside** the engine-rendered frame
  (the compositor draws `slate::colour_bars` for missing footage and composites
  it like any source), so the Viewer shows no separate slate on the comp path;
  its placeholder wording drops the "single-layer" caveat there.
- **CPU and zero-copy.** This is the RGBA-readback path; the zero-copy
  shared-texture path (K-177, the "Viewer texture path" section above) is now
  built on Windows behind the opt-in `shared-texture` feature and renders through
  the same `HeadlessRenderer` (`render_to_shared`, a D3D12 shared NT handle). The
  readback path remains as the automatic fallback and for the Scopes.

## Text, fonts, icons

- Inter Medium ships as a bundled Flutter font family, same OFL licence file.
- Iconoir arrives via the maintained `iconoir_flutter` package (same set the
  Rust side embeds); the drawn motion-blur mark becomes a `CustomPainter`
  reproducing the 24×24 artwork.

## Persistence

The egui frontend persists the workspace through eframe's storage and the
project session through egui's data map. The Flutter side writes one JSON file
(`workspace.json`) in the platform config directory carrying the §9 inventory
state, and per-project session files next to it. Migration from the eframe
store is not attempted — the alternative frontend starts with defaults, which
is acceptable for an experiment (logged in the checklist).

## Testing strategy

- Pure Dart logic (dock tree, settings, theme tables, palette filtering,
  shortcut routing) — plain unit tests, the bulk of coverage.
- Widgets — `flutter_test` widget tests: settings pages render every control,
  menu items dispatch, tab pills switch, shortcuts fire actions.
- Bridge (F1+) — Rust-side integration tests on `lumit-bridge` (no Flutter
  needed), plus one Dart smoke test loading the real cdylib.
- Golden-image tests are deferred until the theme stabilises — they are brittle
  while chrome is still landing.
