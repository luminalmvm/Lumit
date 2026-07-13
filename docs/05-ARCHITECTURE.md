# Kiriko system architecture

**Status: canonical.** This document defines how Kiriko is structured as a codebase and a
running process. It implements decisions K-010 through K-019 in [02-DECISIONS.md](02-DECISIONS.md).
Terminology follows [01-GLOSSARY.md](01-GLOSSARY.md) exactly. RFC-2119 keywords (MUST, SHOULD,
MAY) are binding. The companion rulebook is [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md);
runtime degradation policy lives in [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md).

The two requirements everything below serves: **the UI stays responsive under any load**, and
**the application never crashes**. Every structural choice is downstream of those.

---

## 1. Workspace layout

Kiriko is a single cargo workspace of small crates. Small crates keep incremental builds in
seconds, force interfaces to be explicit, and make the two load-bearing seams — engine/UI and
engine/media — mechanically enforceable rather than aspirational.

| Crate | Responsibility |
|---|---|
| `kiriko-core` | The document model: project, assets, comps, layers, clips, properties, keyframes, Retime segments, markers. Pure data + command application. No IO, no GPU, no threads. |
| `kiriko-time` | Rational time types (`SourceTime`, `ClipTime`, `LayerTime`, `CompTime`, `FrameRate`) and conversions. Depended on by everything. |
| `kiriko-eval` | The heart of **Togi** (K-067), the render pipeline: compiles a comp's layer stack into the evaluation graph; scheduling, cancellation epochs, the metadata pass, job orchestration for the pixel pass. |
| `kiriko-gpu` | The one wgpu device, WGSL effect kernels, texture pool, readback, device-lost recovery, optional CUDA interop. |
| `kiriko-media` | rsmpeg demux/decode/encode, frame index, persistent decoder instances, hardware decode, proxy generation, image sequences. |
| `kiriko-audio` | **Hibiki** (K-067): cpal output, audio graph evaluation, the audio clock, peak-pyramid waveform generation, beat detection. |
| `kiriko-cache` | **Kura** (K-067): the three-tier cache (VRAM/RAM/disk), content-hash keys, budget accounting, eviction, the resource governor. |
| `kiriko-expr` | QuickJS-ng embedding for expressions (K-063): deterministic runtime, AE-surface library, per-property sandboxing. |
| `kiriko-ofx` | OFX host: out-of-process plugin server, C ABI boundary, shared-memory/shared-texture frame transport. |
| `kiriko-kfx` | KFX host (K-062). Shares the sandbox/IPC substrate with `kiriko-ofx`. |
| `kiriko-project` | Serialisation: `.kir` container read/write, operation journal, autosave, relink, migrations. Spec: [10-FILE-FORMAT.md](10-FILE-FORMAT.md). |
| `kiriko-ui` | The egui shell: a tiling dock (egui_tiles, K-074) with a bare Viewer, timeline/graph-editor/Viewer widgets, theming per [15-DESIGN.md](15-DESIGN.md). |
| `kiriko-app` | The binary: winit event loop, wiring, session lifecycle, crash handler. |

### 1.1 Dependency direction rules

- Dependencies point **downward only**: `kiriko-app` → `kiriko-ui` → engine crates →
  `kiriko-core`/`kiriko-time`. No engine crate may depend on `kiriko-ui` or on egui, winit, or
  any UI crate. This is the K-012 escape hatch: the UI layer MUST be replaceable (GPUI, Qt
  shell) without touching the engine.
- `kiriko-core` and `kiriko-time` MUST have no dependency on wgpu, rsmpeg, cpal, or QuickJS.
  The document model is testable on any machine with no GPU and no codecs.
- `kiriko-eval` depends on `kiriko-core` (reads compiled snapshots) and on `kiriko-gpu`,
  `kiriko-media`, `kiriko-cache` through **trait objects defined in `kiriko-eval` itself**
  (`FrameSource`, `KernelExecutor`, `CacheStore`), so the graph scheduler unit-tests against
  fakes.
- Heavy FFI crates (`rsmpeg`, wgpu, cudarc, QuickJS bindings) live only in their one owning
  crate. No `-sys` crate appears in more than one `[dependencies]` table.
- Circular dependencies are a build error by construction; if two crates want each other, the
  shared piece moves down into a new crate or into `kiriko-core`.

---

## 2. Process and thread model

One main process plus sandbox processes for third-party plugins (§7) and the crash handler.
Inside the main process, threads have fixed roles:

| Thread | Role |
|---|---|
| **UI thread** | winit events, egui, document edits, painting. Per K-017 it MUST NOT evaluate any node, decode any frame, run any expression, or block on any render. It reads results from latest-wins mailboxes and cache-status snapshots. |
| **Worker pool** | Work-stealing pool (`cores − 1` threads, adaptive), running evaluation-graph jobs. Two priority classes: *interactive* (current Viewer frame, scrub, audio-adjacent) and *background* (cache warming, thumbnails, proxy checks). Interactive always pre-empts at job boundaries. |
| **Decode threads** | One per active media stream, owned by `kiriko-media`, feeding bounded frame queues. Decode never runs on pool workers: long-GOP seeks stall unpredictably and would starve the pool. |
| **IO threads** | Disk-cache read/write, project autosave journal appends, proxy/export file IO. |
| **Audio thread pair** | The cpal callback (real-time, lock-free ring-buffer reads only) plus an audio-render thread that evaluates the audio graph ahead of the callback, sample-accurately. |
| **GPU-submit thread** | Sole owner of wgpu queue submission. Interactive work and background work submit through it in separate batches so a scrub pre-empts cache warming. |

### 2.1 Cancellation

Every render request carries an **epoch** (a monotonically increasing generation number per
consumer — one for the Viewer, one per export job, one for background warming). Moving the
playhead bumps the Viewer epoch. Jobs check their epoch at node boundaries and between
macro-tiles; a superseded job aborts before its next node. GPU work is submitted a few nodes
per command buffer so a stale frame wastes at most 1–2 ms of GPU time. Frames that complete
despite being stale still enter the cache — the work is kept, not wasted.

### 2.2 Playback pipelining

Playback is a three-stage pipeline over bounded queues (2–4 frames deep, back-pressure by
construction):

```
decode(N+k)  ─▶  evaluate(N+1…)  ─▶  present(N)
   media threads     worker pool        UI thread blit
```

**The audio clock is master.** The audio-render thread counts samples actually consumed by the
cpal callback; the presented video frame is `f(audio_samples_consumed)`. Video never drives
audio. If evaluation falls behind, frames are dropped at present (and adaptive degradation
engages per [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)); audio never stutters to wait
for pixels.

---

## 3. Document model

`kiriko-core` holds the project as an **immutable snapshot + command journal**:

- Every edit is a **command**: a small, serialisable operation (`SetKeyframe`, `TrimClip`,
  `AddLayer`, …) with a computed inverse. Applying a command produces a **new snapshot**;
  unchanged subtrees are structurally shared (persistent data structures — `im`-style maps/
  vectors keyed by UUID), so a snapshot is cheap to take and cheap to keep.
- **Undo/redo** is the command journal walked backwards/forwards. **Autosave** is the same
  journal appended to disk as edits happen (fsync on a short timer), plus periodic compacted
  snapshots; crash recovery is last snapshot + journal replay. See
  [10-FILE-FORMAT.md](10-FILE-FORMAT.md).
- Every entity (asset, comp, layer, clip, property, keyframe, marker, effect instance) has a
  **stable UUID** assigned at creation and preserved across save/load, copy/paste (remapped on
  paste), and AE import. Nothing in the engine, cache, or file format identifies an entity by
  index or position.

**Snapshot isolation** is what makes K-017 workable: the UI thread edits the document and
publishes a new snapshot; renders in flight keep the snapshot (and compiled graph) they
started with. Workers never observe a half-applied edit, so there is no locking between edit
and render paths — publication is a single atomic pointer swap (`arc-swap`). The UI reads its
own latest snapshot; workers read theirs; both are complete, consistent worlds.

---

## 4. Compiling the layer stack to the evaluation graph

Per K-015: **layers in the UI, DAG underneath**. Users never see the evaluation graph.

On every document edit, `kiriko-eval` incrementally recompiles the affected comp:

1. Each layer lowers to: source node → effect-stack nodes → mask/matte nodes → transform node
   → blend node compositing over the accumulated result below. Adjustment layers lower to an
   effect chain applied to the accumulated composite; Precomp layers lower to the nested
   comp's subgraph behind a single boundary node; a matte is a side input to the blend node.
2. A **Sequence layer** lowers to a *time-multiplexed switch*: for any requested layer time,
   exactly one clip is active (clips never overlap), so compilation resolves which clip covers
   that time and emits that clip's subgraph — source node, the clip's Retime mapping, its
   frame-interpolation policy — then the Sequence layer's own effects/masks/transform apply to
   the switch output. Edit points are pure data; no node exists "between" clips.
3. **Retime** compiles into the *time argument* of upstream requests, not into a pixel node:
   a node evaluated under a Retime is asked for a different source time. The request key is
   `(node_id, local_time, quality, roi)`. Frame interpolation (nearest/blend/flow) inserts a
   synthesis node only when the mapped source time is non-integral in source frames — flow
   synthesis is a real (heavy, cacheable) node; nearest is an identity that snaps time.
   Retime maths is specified in [04-RETIMING.md](04-RETIMING.md).

### 4.1 Two-pass evaluation

- **Metadata pass** — cheap, synchronous, runs on edit and on any request: establishes per
  node its output format, frame range, and DoD (bounding box of defined pixels), and
  propagates ROI top-down (each consumer declares what region it needs; effects declare their
  input-expansion function `roi_in = f(roi_out)`). Identity detection happens here: an effect
  at neutral parameters, a disabled effect, opacity 1.0, declares itself a pass-through and is
  folded out.
- **Pixel pass** — expensive, on workers, cancellable: computes textures only inside
  `ROI ∩ DoD`. Full-frame-per-node on GPU; macro-tiling only as the VRAM/TDR fallback (§5).

### 4.2 Content hashing

Every node computes `H(node) = hash(node_type, algorithm_version,
evaluated_params_at_local_time, local_time, quality, H(inputs…))` with a strong short hash
(blake3). Hashes key the cache (K-016) — never timeline position. Consequences that fall out
for free: two layers sharing footage plus the same first effects deduplicate to one subgraph
(common-subexpression elimination by hash); a static subgraph hashes identically for every
frame, so cross-frame reuse is a cache lookup. Effects that sample other frames (echo, flow
retiming, temporal blur) MUST declare their temporal dependencies in the metadata pass so the
sampled frames' hashes fold in. Full pipeline detail: [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md).

---

## 5. GPU architecture

- **One wgpu device** for the whole application (K-011): DX12 backend on Windows, Metal on
  macOS, Vulkan on Windows only when CUDA interop is enabled (see below). `kiriko-gpu` owns
  it; nothing else holds raw device handles.
- **All first-party effects are WGSL compute kernels.** The working format is **fp16
  scene-linear premultiplied RGBA** (fp32 per-comp opt-in, K-026). Unpremultiply happens only
  transiently inside colour ops that must not tint transparent regions.
- **Texture pool**: node outputs are pooled, DoD-sized textures with refcounted lifetimes
  derived from the compiled graph; the pool allocates through the resource governor's VRAM
  budget. Nothing allocates a texture ad hoc.
- **Readback paths**: async buffer readback for RAM-cache demotion, disk-cache writes, scopes
  export, and CPU-fallback bridges. Readbacks never block the UI thread or the submit thread;
  they complete on IO/pool threads via mapped-buffer callbacks.
- **Device-lost recovery** (Windows TDR is routine, not exceptional): all GPU objects belong
  to a device **epoch**. On `DeviceLost`, `kiriko-gpu` tears down the epoch, recreates the
  device, and replays the current request; RAM and disk cache tiers are the recovery data —
  VRAM contents are never the only copy of anything the user would miss. No dispatch may
  approach the ~2 s TDR window: kernels that scale with radius/area MUST macro-tile. Repeated
  device loss (≥3/minute) drops the offending node — then the session — to CPU fallback with
  a user-visible notice.
- **CUDA per K-014**: optional per-node accelerators (optical flow first) via
  `wgpu as_hal` → `VK_KHR_external_memory/semaphore` → cudarc. CUDA is never a pipeline;
  **every CUDA-accelerated node MUST have a WGSL or CPU implementation** that produces
  acceptably close output, selected automatically when CUDA is absent or misbehaving.
- **CPU fallback** is per-node, not per-app: the scheduler inserts readback → CPU node →
  upload bridges, batching adjacent CPU nodes to avoid bus ping-pong. Every WGSL effect ships
  a CPU reference implementation (K-019), which is also its test oracle.

---

## 6. Media layer

`kiriko-media` wraps rsmpeg behind a `MediaSource` trait (open → probe → indexed frame
server) so the binding choice stays swappable.

- **Frame index at import**: a background job builds, per footage item, the map of frame
  number ↔ PTS ↔ nearest preceding keyframe offset. Exact long-GOP seeking is then: seek to
  keyframe, decode forward comparing *output* PTS, flush codec buffers after seeks. Variable
  frame rate is normalised to the item's timebase at import, with a user warning.
- **Persistent decoder instances**: 1–2 decoders per active clip hold their GOP position; a
  scrub within the GOP decodes forward, sequential playback never seeks. Decoders live on
  dedicated decode threads feeding bounded queues (§2).
- **Hardware decode** (K-013): ffmpeg hwaccel — D3D11VA/D3D12VA on Windows, VideoToolbox on
  dev Macs — landing NV12/P010 in GPU memory, then **one GPU→GPU copy** into a wgpu texture
  plus a WGSL pass (colour matrix + chroma upsample + linearise) into the working format.
  Zero-copy import is a post-v1 optimisation tracked against wgpu's texture-import API, not a
  v1 requirement.
- **Proxies**: background generation (DNxHR/ProRes proxy or all-intra H.264) on IO/background
  priority; the proxy toggle is global, and proxy level is a dimension of the cache key.
- **Image sequences** are first-class footage items (`name.####.exr` patterns); EXR half maps
  1:1 onto the fp16 working format; per-frame files decode in parallel on the pool.
- **Encode** goes through ffmpeg's NVENC/AMF/QSV/VideoToolbox wrappers; the export pipeline
  (including baking) is specified in [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md).

---

## 7. Plugin isolation

First-party effects are trusted, in-process, GPU-native. **Third-party OFX and KFX effects
run out-of-process** — in-process plugins are the number-one crash source in every host
Kiriko is replacing, and "never crashes" cannot be delegated to third parties.

Architecture level (full protocol in [12-PLUGINS.md](12-PLUGINS.md)):

- A **plugin server process** per vendor bundle loads plugins; frames cross via shared memory
  (CPU path) or shared GPU handles; parameters and UI actions cross via RPC.
- Each server has a **watchdog**: a hung or crashed plugin process is killed and restarted;
  the affected node renders as an errored placeholder (checkerboard, per
  [15-DESIGN.md](15-DESIGN.md)) and the application continues.
- The evaluation graph treats a plugin node like any other node: it declares ROI expansion,
  temporal needs, and a thread-safety capability flag; non-reentrant plugins serialise on
  their own server without stalling the rest of the graph.

Expressions (`kiriko-expr`) are in-process but sandboxed per K-063: no IO, no `Date`, no JIT
variance, seeded random only — an expression can be wrong, never non-deterministic and never
fatal.

---

## 8. Appendix: anti-lessons

Each failure below is somebody else's decade; each maps to a binding Kiriko rule.

| Anti-lesson | What happened | Kiriko rule |
|---|---|---|
| **Olive's unshipped rewrite** | The 0.2 ground-up rewrite (nodes-as-document, float, OCIO, disk cache — the right shopping list) consumed six-plus years and never shipped stable; development halted, then restarted again in another stack. | Ship a usable editing loop early and grow the engine underneath it. No big-bang rewrites: the crate seams (§1) exist so any layer is replaced incrementally. Nodes stay internal; layers are the document (K-015, K-020). |
| **Natron's CPU-only stall** | A credible Nuke-alike whose performance reputation died on CPU-only rendering, with GPU retrofit never achieved before the maintainers left. | GPU-first from day one: every first-party effect is WGSL compute (K-011); CPU is the fallback and oracle, never the plan (K-019). |
| **Natron's deadlocks** | Their own docs: render-path deadlocks were the hardest bugs they had. Fine-grained locking between edit, cache, and render paths. | No shared mutable state between edit and render: immutable snapshots, atomic publication, message passing, bounded queues, epoch cancellation (§2, §3). The lock-across-boundary rules in [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md) §2 are load-bearing. |
| **AE's thread-safety retrofit** | A 1993 single-threaded codebase took Adobe a multi-year campaign to make Multi-Frame Rendering possible, including retrofitting a plugin thread-safety flag. | Concurrency contracts are day-one architecture: what runs where is fixed (§2), and the effect/plugin API carries a thread-safety capability flag from its first version (§7). |
| **MLT's GPU afterthought** | An elegant CPU frame pipeline where GPU residency was bolted on and stayed fragile, forcing CPU↔GPU ping-pong. | Texture residency is the default frame contract; CPU excursions are explicit bridge nodes inserted by the scheduler (§5). |
| **Blender's execution-model migrations** | Tiled → full-frame → GPU: three multi-year compositor rewrites to land where it should have started. | Full-frame-per-node on GPU now, ROI/DoD as metadata, tiling only as fallback (§4.1, §5). Execution models are the most expensive thing to retrofit; Kiriko picks the endpoint. |

---

## Open questions

- **Multi-queue GPU scheduling**: wgpu has no multi-queue today; background submissions share
  the queue with interactive ones. If pre-emption at batch granularity proves too coarse on
  low-end GPUs, do we need `as_hal` async-compute plumbing, and on which backends?
- **Snapshot memory ceiling**: persistent structures make snapshots cheap, but a long session
  with heavy undo history plus in-flight renders pins many snapshots. Journal compaction
  policy needs a measured budget — entry in [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)?
- **OFX GPU suites**: which OFX GPU render suites (CUDA/OpenCL/Metal images) do the target
  plugins (Twixtor, RSMB, Sapphire) actually require, and does the shared-texture transport
  cover them, or do some force a CPU staging path at v1? Needs a plugin-by-plugin audit in
  [12-PLUGINS.md](12-PLUGINS.md).
- **HDR/wide-gamut swapchain**: scRGB output on Windows through wgpu needs a spike; may
  require hal access. Affects Viewer colour accuracy claims in [15-DESIGN.md](15-DESIGN.md).
- **Sequence layer transitions**: a cross-fade at an edit point means two clips are briefly
  live, which breaks the "exactly one clip active" compilation rule. Decide the lowering
  (overlap window with a dedicated transition node?) before transitions enter
  [03-DATA-MODEL.md](03-DATA-MODEL.md).
