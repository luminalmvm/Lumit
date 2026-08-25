# Kiriko engine research — high-performance video compositing architecture

Research notes for a native Windows-first After Effects alternative. Goal: responsive and crash-free under heavy load (many effects, 4K, long comps), GPU compute wherever possible. Sources: Foundry NDK docs, Adobe MFR docs/SDK guides, Blender developer docs, Natron/Olive/MLT project history, Khronos/NVIDIA/Microsoft docs, Resolve manual. Compiled 2026-07-12.

---

## 1. Render graph design

### 1.1 The DAG as the universal substrate (Nuke, Fusion, Natron)

- All serious compositors evaluate a **directed acyclic graph** of image operations via a **demand-driven pull model**: nothing renders until an output (viewer, writer, exporter) *requests* a result for a `(node, frame, quality, roi)` tuple; the request recurses upstream ([Foundry NDK 2D architecture](https://learn.foundry.com/nuke/developers/90/ndkdevguide/2d/architecture.html)).
- Nuke's evaluation is phased, and this phasing is the single most copied idea in the field:
  1. `validate()` — main thread, cheap, top-down/bottom-up metadata pass: establishes each node's **format, channels, frame range, and DoD (bounding box of defined pixels)**.
  2. `request(roi)` — main thread: the downstream consumer tells each input *which region and channels it will actually need*. Upstream nodes intersect this with their DoD.
  3. `engine()` — worker threads compute pixels (in Nuke's case per **scanline Row**, 32-bit float, per-channel planar arrays), only inside `ROI ∩ DoD`.
- Keeping the metadata pass (cheap, main-thread, synchronous) strictly separate from the pixel pass (expensive, worker threads, cancellable) is what makes the UI feel instant: param edits re-run validate immediately (updating bounding boxes, comp duration, etc.) while pixel evaluation is queued asynchronously.
- Fusion and Natron follow the same request/render split via **OpenFX**: OFX formalises `getRegionOfDefinition`, `getRegionsOfInterest`, `render(renderWindow)` actions plus `isIdentity` (node can declare itself a pass-through for the current params/frame — free optimisation, always implement an equivalent).
- Nuke is scanline-based mainly for CPU memory reasons (never hold whole frames for a long chain). For a GPU-first engine, per-node **full-tile/full-frame textures** are the right unit instead (see §4), but the request/validate phasing carries over unchanged.

### 1.2 Layer UI compiling to a graph

- AE's layer stack **is** a graph: each layer = source node → chain of effect nodes → transform node → blend node that composites over the accumulated result below it. Track mattes are side inputs to the blend; precomps are subgraphs with a single output; adjustment layers are effect chains applied to the accumulated composite so far.
- Recommended design: the document model is layers/keyframes (user-facing, undoable); a **compiler** lowers it to an immutable evaluation graph. Recompile is incremental — only the affected comp — and is cheap enough to run on every edit. Blender's sequencer/compositor and Olive both converged on this "UI model ≠ evaluation model" split; Olive 0.2 made the node graph the *document* model and paid for it in UX complexity — keep nodes internal, layers external (optionally expose the graph read-only for power users later).
- **Flattening / graph optimisation** at compile time:
  - Constant-fold identity nodes (opacity 1.0, 0-value blur, disabled effect) via `isIdentity`-style checks.
  - **Common subexpression elimination**: two layers using the same footage + same first N effects share one subgraph node. Because nodes are content-hashed (§2), CSE falls out naturally — dedupe by hash.
  - Fuse adjacent simple ops (transform→transform, LUT→LUT, per-pixel colour ops) into single GPU passes; Blender's realtime compositor evaluates simple pixel-wise node runs as fused shader chains ([Blender realtime compositor](https://code.blender.org/2022/07/real-time-compositor/)).
  - Time-remapping means a node may be evaluated at a *different* frame than the comp frame; the request key must be `(node_id, local_time, quality, roi)`, not just comp time.
- **Across frames**: static subgraphs (no animated params, still source) hash identically for every frame → the cache turns cross-frame CSE into a lookup, no special machinery needed.

---

## 2. Caching

### 2.1 Hash-based invalidation

- Universal pattern (Nuke `Op::hash()`, Natron, modern engines): every node computes
  `H(node) = hash(node_type, node_version, param_values_at_eval_time, local_frame_time, quality/proxy_level, H(input_1), H(input_2), …)`
  Cache key = that hash (plus ROI if partial results are cached). Any upstream change ripples down automatically; anything untouched keeps its hash and its cache entries stay valid.
- Details that matter:
  - Hash **evaluated param values** (post-expression, at the node's local time), not raw keyframe data — this makes time-invariant animated params cache correctly per frame.
  - Include a per-node-type **algorithm version** so shipping a bug-fixed effect invalidates old caches.
  - Effects that sample other frames (echo, temporal blur, optical-flow retime) must fold the hashes of *all* sampled frames in — the request phase should declare temporal dependencies just like spatial ROI.
  - Use a strong-enough hash (xxHash3-128 / Blake3 short) that collisions are ignorable; never compare params structurally at lookup time.

### 2.2 RAM cache

- Central **budgeted pool** (user-set, default ~50–70% of RAM; Natron and Nuke both expose this) with LRU eviction, but weighted:
  - Pin the frames around the playhead and the current viewer result.
  - Prefer evicting *intermediate* node results over final composited frames (finals are what playback needs; intermediates are cheap to recompute if their inputs are cached).
  - Track entry cost (bytes) and recompute cost (measured ms) — evict high-byte/low-cost first ("GreedyDual"-style beats plain LRU here).
- **GPU (VRAM) cache is a separate tier**: textures for recently used node outputs, budgeted against queried VRAM with headroom for the compositor/OS (Windows WDDM will demote you anyway — stay under ~75–80% of dedicated VRAM). Eviction demotes GPU→RAM (readback) only if the entry is expensive; otherwise just drop.

### 2.3 Disk cache

- Tier 3, persistent across sessions (Nuke viewer cache, Natron disk cache, Olive's design goal of a "highly efficient automated disk cache", Resolve's render cache).
- Format guidance: **don't invent a container** — a directory of per-entry files named by hash, plus an index (SQLite or a compact journal) mapping hash → file/offset/metadata. Store frames as:
  - fp16 uncompressed or LZ4-compressed planes for exactness and speed (disk caches are bandwidth-bound; LZ4/ZSTD-1 on fp16 planes typically hits multi-GB/s), or
  - DNxHR/ProRes for *conform-level* caches where visually-lossless is fine (what Resolve does — its render cache literally encodes to a user-chosen intermediate codec).
- Write-behind: disk-cache writes happen on background IO threads from the RAM copy; never block the render for a disk write. Cap total size, LRU by file atime/own index.

### 2.4 Background pre-rendering

- **AE 2022 Multi-Frame Rendering** ([Adobe MFR docs](https://helpx.adobe.com/after-effects/using/multi-frame-rendering.html)): after making the whole codebase thread-safe, AE renders many frames **concurrently in one process** (shared project state, no per-instance duplication like the old "Render Multiple Frames Simultaneously" hack). It continuously measures per-frame cost and memory and **dynamically adjusts concurrency** — the license for Kiriko: schedule N frame jobs where N adapts to measured frame cost and RAM/VRAM pressure, don't fix thread counts. AE also added *Speculative Preview*: rendering the comp in the background while the app is idle.
  - Plugin lesson from the AE SDK: effects must declare thread-safety (`AE_Effect_Global_Refcon`-style flags); non-thread-safe effects force serialisation of that node while the rest of the graph stays parallel. Design the same capability flag into Kiriko's effect API from day one.
- **Resolve Smart Cache** ([Resolve manual](https://www.steakunderwater.com/VFXPedia/__man/Resolve18-6/DaVinciResolve18_Manual_files/part246.htm)): automatically decides *what* to cache (codecs too slow to decode live — H.264/H.265/RAW; Fusion comps; speed effects), caches **at the source level vs sequence level** as appropriate, and renders during **idle time**. Red/blue indicators in the timeline ruler show uncached/cached.
- Kiriko plan: an idle-priority background scheduler walks outward from the playhead (then over marked work areas) rendering final frames into RAM→disk cache; it yields instantly to interactive requests (cancellation, §6) and to memory pressure (§7).

### 2.5 Cache visualisation

- AE: green bar (RAM-cached final frames) + blue (disk-cached) across the timeline. Resolve: red→blue line in the ruler. This is essential UX, cheap to build: the cache keeps a per-frame bitmap of "final frame present at current quality" per comp; timeline draws it as a 2px strip. Also show a subtle second strip for "cached at preview res only".

---

## 3. Precision, colour, alpha

- **Working pixel format**: float pipelines are non-negotiable for compositing quality (Nuke is 32-bit float end-to-end; Natron 32-bit float linear; Olive 0.2 rebuilt around half/float + OCIO from the outset). Pragmatic GPU choice: **fp16 (RGBA16F) as the default working/cache format, fp32 for accumulation-sensitive ops** (big blurs, iterative effects, scopes accumulation) and as an opt-in per-comp quality switch. fp16 halves bandwidth/VRAM — the actual bottleneck — and its ~11-bit mantissa is fine for display-referred and most HDR work when intermediates stay premultiplied and near [0, 64k) range.
- **Linear light**: decode footage to scene-linear (or at minimum linearised display space) before compositing; filtering, resampling, motion blur, and `over` are only correct in linear. Keep sRGB/gamma encode as the *last* step in the viewer and at export.
- **Premultiplied alpha everywhere internally**: `over = A + B*(1−a_A)` only composes associatively with premult; filtering unpremultiplied images bleeds background colour into edges. Unpremultiply *only* transiently inside colour-correction ops that must not tint transparent regions, then re-premultiply (this is exactly the Nuke Merge/Grade convention). Document the convention in the effect API and provide helpers.
- **OCIO** ([OpenColorIO shaders API](https://opencolorio.readthedocs.io/en/latest/api/shaders.html)): adopt OpenColorIO v2 as the colour-management backbone (Natron, Nuke, Blender, Olive all do). OCIO v2's **GPUProcessor generates shader source** for a target language (GLSL/HLSL/Metal — request the language you need, or transpile the GLSL) plus required 1D/3D LUT textures; the "generic" v2 path evaluates ops exactly like the CPU renderer, no baking error. Wire: input transform (per-footage IDT) → working space (scene-linear, e.g. ACEScg or linear-sRGB default config) → display/view transform in the viewer only.
- **Scopes on GPU**: waveform/vectorscope/histogram are scatter/accumulate jobs — one compute pass over the (already-on-GPU) viewer image doing `InterlockedAdd` into a small UAV (e.g. 256×N bins for waveform columns, 2D CbCr histogram for vectorscope), one tiny pass to normalise/draw. Run them on the *display-referred* or explicitly chosen signal, at most once per displayed frame, on the same async compute queue as the viewer blit. Cost is <0.5 ms at 4K; never compute scopes on CPU.

---

## 4. Tiled vs full-frame; ROI and DoD

- **Nuke (scanline/tile, CPU)**: fundamental unit is a Row; ROI × DoD intersection means a Blur asked for a 200px crop touches only ~200px+kernel of upstream image regardless of source size. Wins: bounded memory, work proportional to what's viewed. Costs: per-row virtual-call overhead, poor fit for GPU dispatch, complex operator authoring.
- **Blender's arc is the decisive evidence**: the old tiled/per-pixel compositor was replaced by the **full-frame** design (buffers per operation, freed as soon as the last reader finishes) and then the **GPU realtime compositor**; the 4.2 rewrite made final renders "often several times faster" ([Blender 4.2 notes](https://developer.blender.org/docs/release_notes/4.2/compositor/), [T88150](https://projects.blender.org/blender/blender/issues/88150)). Tiling per-pixel graphs on CPU has high interpretive overhead; full-frame + buffer lifetime analysis is simpler and faster, and maps directly to GPU textures.
- **Recommended hybrid for Kiriko**:
  - **Full-frame-per-node on GPU** as the execution model (each node output = one texture, lifetime-managed by refcount from the compiled graph; aliased/pooled allocations).
  - **Keep ROI/DoD as metadata even in full-frame execution.** DoD (the bounding box of non-transparent pixels — a small title layer in a 4K comp has a tiny DoD) bounds the *allocated* texture and the dispatch grid; ROI from downstream crops what upstream must produce. This routinely cuts memory and work by 10–100× on motion-graphics-style comps and is what allows "adjustment layer over one small region" to be cheap.
  - **Tile only as a fallback**: when a requested full-frame allocation exceeds the VRAM budget (8K comps, deep chains), split the request into 2–4 macro-tiles and run the subgraph per tile. Effects with non-local reach (blur radius, distortion max displacement) must declare their **input-expansion function** (`roi_in = f(roi_out)`) — this is the OFX `getRegionsOfInterest` contract and it's needed for correct tile overlap anyway.
  - DoD must propagate through transforms (transform the box), grows through blurs/glows (pad by radius), clamps at crops/comp bounds. Blend node DoD = union of inputs.

---

## 5. GPU strategy

### 5.1 API choice

- Field survey: **Resolve** uses CUDA on NVIDIA / OpenCL on AMD-Windows / Metal on macOS, auto-selected ([BMD forum guidance](https://forum.blackmagicdesign.com/viewtopic.php?f=21&t=83494)); CUDA is measurably fastest on NVIDIA. **Blender's compositor** uses portable GPU compute (its GPU module over Vulkan/Metal/GL). **Blender Cycles** carries CUDA+OptiX+HIP+oneAPI+Metal backends — a huge maintenance surface that a small team must avoid. **Natron is CPU-only** — and its performance reputation suffered for it.
- Verdict for Kiriko (Windows-first, small team): **one portable compute path as the required baseline, vendor APIs only where they buy something compute shaders can't.**
  - Baseline: **Vulkan compute or D3D12 compute** (or wgpu over both). Every effect ships in this path. Modern compute shaders (subgroup ops, fp16 arithmetic, push descriptors) reach within ~10–30% of CUDA for image-processing workloads; the gap is not worth a per-vendor effect codebase.
  - What CUDA uniquely buys: cuFFT/NPP libraries, OptiX denoising, easier kernel authoring, and some vendor-tuned wins. Treat CUDA/HIP as an **optional acceleration backend for specific heavy nodes** (optical flow, denoise, FFT-based blur) behind the same node interface — never a second full pipeline. DirectML/ONNX Runtime covers ML-based effects (rotoscoping, upscaling) portably across NVIDIA/AMD/Intel on Windows.
  - Interop is a solved problem if needed: `VK_KHR_external_memory(_win32)` + `VK_KHR_external_semaphore` ↔ `cudaImportExternalMemory` / external semaphores lets CUDA kernels write Vulkan-visible memory zero-copy ([Vulkan external memory guide](https://docs.vulkan.org/guide/latest/extensions/external.html), [CUDA driver interop API](https://docs.nvidia.com/cuda/cuda-driver-api/group__CUDA__EXTRES__INTEROP.html)). It works but adds real complexity (handle types, layout/queue-family transitions, sync bugs) — another reason to keep vendor kernels rare.

### 5.2 Zero-copy hardware decode

- Decode must land **directly in GPU memory** and stay there:
  - **NVDEC** (via Video Codec SDK or ffmpeg `hwaccel=cuda`) → CUDA device frames → export to Vulkan/D3D12 via external memory; or
  - **D3D11VA/D3D12VA** (ffmpeg `d3d11va`/`d3d12va` hwaccels) → NV12 `ID3D11Texture2D`/D3D12 resource — the *vendor-neutral Windows path* (NVIDIA, AMD, Intel all implement it); share into Vulkan if the engine is Vulkan-based, or consume natively if D3D12-based; or
  - **Vulkan Video** (`VK_KHR_video_decode_*`): decode straight into `VkImage`s inside your own Vulkan device — cleanest single-API story, now shipping on NVIDIA (Pascal+), AMD RADV, and ffmpeg (`-hwaccel vulkan`), but **only H.264/HEVC/AV1(/VP9)**; older/odd codecs still need D3D11VA or CPU ([mpv Vulkan Video guide](https://github.com/mpv-player/mpv/discussions/13909)).
- The decoded frame is NV12/P010 — run one tiny compute pass (colour-matrix + chroma upsample + linearise via OCIO) NV12→fp16 RGBA working texture. That pass is where "zero-copy" pays: no CPU round trip, no staging upload for camera footage.
- Recommended: **D3D12-centred engine on Windows** (D3D12 compute + D3D12VA decode + DXGI present) gives the least-interop path on the target platform; if portability is desired later, wgpu/Vulkan with D3D11VA-sharing is the alternative. Either way, ffmpeg supplies demux + bitstream, hardware does pixel decode.
- **Encode**: NVENC / AMD **AMF** / Intel **QSV**, all reachable uniformly through ffmpeg's encoder wrappers (`h264_nvenc`, `hevc_amf`, `av1_qsv`, …) — use ffmpeg as the abstraction rather than three vendor SDKs. Hardware-encode H.264/HEVC/AV1 for previews/proxies/delivery; CPU (x264/x265, or licensed ProRes/DNxHR) for mastering quality.

### 5.3 CPU fallback

- Every effect's reference implementation is CPU (also serves as the test oracle for the GPU version). Fallback triggers: no capable GPU, device-lost storm (§7), VRAM exhaustion after degradation, or an effect without a GPU port. Fall back **per-node**, not per-app: the scheduler inserts readback→CPU-node→upload bridges. Keep CPU images in the same fp16/fp32 planar layout to make bridging trivial. Batch adjacent CPU nodes to avoid ping-ponging across the bus.

---

## 6. Threading model

- **Process layout**:
  - **UI thread**: owns the document model, input, drawing. **Never** evaluates a node, never blocks on a render, never takes a lock the render side holds for unbounded time. It reads render *results* from lock-free mailboxes ("latest completed viewer frame", cache-status bitmap snapshots).
  - **Graph/compile thread** (or short main-thread tasks): document edit → incremental recompile → publish new immutable graph snapshot (epoch/arena; renders in flight keep the old snapshot — this is what makes cancellation and data races tractable).
  - **Worker pool**: work-stealing job system (one pool, cores−1..cores threads; Nuke, AE MFR, and every modern game engine converge here). Two job priorities minimum: *interactive* (current viewer frame, scrub) and *background* (pre-render, disk-cache writes, thumbnails, waveform generation). AE MFR's lesson: adapt concurrency to measured per-frame cost and memory headroom rather than fixed counts.
  - **Dedicated IO threads**: decode threads per open media stream (feeding bounded frame queues), disk-cache read/write threads. Never decode on pool workers — long-GOP seeks stall unpredictably.
  - **GPU submission thread**: sole owner of queue submission; interactive work on the main compute queue, background pre-render on a second/async queue so scrubbing pre-empts batch work at the hardware level.
- **Frame pipelining for playback**: decode(N+k) ∥ process(N+1..) ∥ display(N) — classic three-stage pipeline with bounded queues (2–4 frames) providing natural back-pressure. Audio clock is the playback master (§8).
- **Cancellation**: every render request carries a generation/epoch token; a scrub bumps the epoch. Jobs check `is_cancelled()` at node boundaries (and between macro-tiles); superseded jobs abort before their next node. GPU work is submitted in small batches (a few nodes per command buffer) so an obsolete frame wastes ≤1–2 ms of GPU time rather than a whole comp. Completed-but-stale results still go into the cache — the work isn't wasted.
- **Progressive preview**:
  - During scrub: render at **half/quarter res** (proxy scale is part of the cache key, so proxy caches are first-class), skip expensive flagged nodes if the user enables "draft", and show the newest completed frame — never queue up stale scrub positions (latest-wins mailbox).
  - On idle (no input for ~100–200 ms): re-render the current frame at full res and quality, then resume background pre-render outward from the playhead. This is AE's adaptive-resolution + Resolve's idle-cache pattern combined.
  - Optionally progressive within a frame (render centre-out macro-tiles) — nice-to-have, not core.

---

## 7. Robustness

- **Memory budgets + back-pressure (degrade, never die)**: a central resource governor tracks RAM and VRAM commitments (all pools allocate through it). Ordered degradation ladder when nearing budget:
  1. stop background pre-render; 2. evict background-tier caches (thumbnails, distant frames); 3. demote GPU cache entries to RAM, then drop; 4. drop preview to half res; 5. switch working format fp32→fp16 if elevated; 6. tile the frame (§4); 7. per-node CPU fallback with streaming; 8. only then refuse the specific operation with a message — never OOM-crash. Windows specifics: respond to `QueryMemoryResourceNotification`/job memory pressure, and to DXGI budget-change notifications (`IDXGIAdapter3::RegisterVideoMemoryBudgetChangeNotificationEvent`) since WDDM shrinks your VRAM budget when other apps compete.
- **GPU device-lost / TDR (Windows-critical)**: any >2 s GPU packet triggers Windows TDR — driver reset, `DXGI_ERROR_DEVICE_REMOVED` ([Microsoft DRED spec](https://microsoft.github.io/DirectX-Specs/d3d/DeviceRemovedExtendedData.html)). Design rules:
  - Split work so no single dispatch approaches the timeout (macro-tile huge blurs/particle sims; the tile fallback of §4 doubles as TDR insurance).
  - Treat device-removed as *expected*: all GPU objects are owned by a `GpuDevice` epoch object; on device-lost, tear down, recreate device, re-upload from RAM-side sources (caches are the recovery data — RAM/disk cache entries survive device loss by design), replay the current request. The user sees a hiccup, not a crash.
  - Enable **DRED** breadcrumbs + page-fault data in dev/beta builds to attribute which node's kernel caused resets; keep per-node GPU timing stats in release to pre-emptively tile nodes that trend long.
  - Repeated device-loss (≥2–3 in a minute) → drop that node (or the whole session) to CPU fallback and tell the user.
- **Autosave + crash recovery**: document model = immutable-ish state with an **append-only operation journal** (every edit serialised as it happens, fsynced periodically) + periodic compacted snapshots. Recovery = last snapshot + journal replay; this is stronger than timer autosave (loses ≤ seconds, not minutes) and gives unlimited undo persistence for free. Write snapshots atomically (temp + rename). Keep a crash-marker file; on unclean start offer recovery. Out-of-process crash handler (Crashpad-style) to capture minidumps.
- **Plugin isolation**: in-process third-party plugins are the #1 crash source in AE/Premiere/OFX hosts. Precedents: browsers (site/process isolation), Bitwig Studio (per-plugin audio plugin processes), Reaper (optional dedicated plugin process). Design:
  - First-party effects: in-process (trusted, tested, GPU-native).
  - Third-party effects: **out-of-process host** — plugin server process loads the plugin; frames pass via shared memory (CPU path) or shared GPU handles (`ID3D12Device::CreateSharedHandle` cross-process resource sharing) for GPU-capable plugins; RPC for params/UI. A hung/crashed plugin process is killed and restarted; the node renders as errored (magenta/checkerboard) instead of taking the app down. Batch multiple plugins into one sandbox process per vendor to amortise IPC, with per-process watchdog timers.
  - Adopt/bridge **OpenFX** for ecosystem compatibility (Natron/Resolve/Nuke plugins) rather than inventing a plugin ABI.

---

## 8. Media I/O

- **ffmpeg (libav) integration**: use libavformat/libavcodec directly (demux, decode, encode, mux) — the MLT/Shotcut, Olive, Blender pattern. Isolate it behind a `MediaSource` interface (open → probe streams → indexed, seekable frame server). Build with hardware hwaccels enabled (d3d11va/d3d12va, cuda, qsv, vulkan). Keep ffmpeg usage in the media process/threads only; its error handling is C-style and its abort paths are a crash risk worth containing (consider an out-of-process media server for maximum robustness — also solves LGPL dynamic-linking hygiene).
- **Frame-accurate seeking in long-GOP** (H.264/HEVC/AV1 camera files) — the classic pitfalls ([ffmpeg seeking wiki](https://fftrac-bg.ffmpeg.org/wiki/Seeking)):
  - `av_seek_frame`/`avformat_seek_file` land on a keyframe *before* the target (or occasionally wrong — known container bugs); you must then **decode forward** discarding frames until PTS ≥ target. Compare PTS in stream timebase, handle B-frame reordering (compare *output* PTS), flush codec buffers after seek (`avcodec_flush_buffers`).
  - Build a **frame index on import** (background job): map frame number ↔ PTS ↔ nearest preceding keyframe byte offset. Makes scrubbing deterministic and enables "which GOP does frame N live in" for decode scheduling.
  - Scrub optimisation: keep 1–2 decoder instances per active clip with their current GOP position; a scrub within the same GOP decodes forward, a backward scrub seeks to the GOP start; sequential playback never seeks. For heavy long-GOP material, offer background **proxy generation** (ProRes Proxy/DNxHR LB or all-intra H.264) — universally what makes editors feel fast; proxies slot into the cache-key quality dimension.
  - Variable frame rate: normalise to the project timebase at import (index by PTS, define frame boundaries), warn the user.
- **Audio pipeline**: audio is the sync master. Pull-model audio callback (WASAPI shared/exclusive) reads from a lock-free ring buffer filled by a dedicated audio-render thread that evaluates the audio graph (per-clip resample to project rate via swr/soxr, volume/pan, mix) **sample-accurately** — positions tracked in samples, never frames. Video frame selection = f(audio clock). Waveform display: on import, background-generate a multi-resolution min/max peak pyramid (e.g. peaks per 256/4096/65536 samples) cached to disk; timeline draws from the appropriate mip. Never decode audio on the UI thread.
- **Image sequences**: EXR/PNG/TIFF/DPX sequences are first-class sources (pattern-matched `name.####.exr`); per-frame files parallelise decode trivially and are the interchange norm for VFX. Use OpenImageIO or tinyexr+libpng directly; EXR half-float reads map 1:1 onto the fp16 working format. Sequence + embedded/ sidecar audio pairing for review movies.
- **ProRes/DNxHR intermediates**: ffmpeg decodes both; encoding — `prores_ks` (unofficial but production-accepted) and `dnxhd` encoders exist in ffmpeg; Apple licensing only matters if you ship Apple's own encoder. All-intra, fixed-bitrate-per-frame → constant-time seeks, light decode: use them for (a) render/export masters, (b) the disk render-cache codec option (Resolve's approach), (c) proxies. DNxHR HQX / ProRes 422 HQ ≈ visually lossless 10-bit; fp16 EXR or 12-bit ProRes 4444 XQ where alpha/HDR must survive.

---

## 9. Lessons from open-source strugglers

- **Olive** ([history](https://en.wikipedia.org/wiki/Olive_(software))): 0.1 (traditional, QPainter/GL, 8-bit) got popular; 2019 began the 0.2 **ground-up rewrite** (node compositor, OCIO, float pipeline, disk cache). The rewrite consumed *six-plus years*: 0.2 never shipped stable, commits stopped Sept 2023, and in 2025 the developer restarted *again* (C#, Godot rendering). Lessons:
  1. **Don't big-bang rewrite; don't let architecture purity block shipping.** Olive had the right technical shopping list (float, OCIO, nodes, disk cache) and still failed to ship it. Kiriko must reach a usable editing loop early and grow the engine underneath it.
  2. **Nodes-as-the-document scared users** and multiplied UI work. Layers in front, DAG behind (§1.2).
  3. Single-maintainer bus factor: architecture docs and boring, testable components matter.
- **Natron** ([status thread](https://discuss.pixls.us/t/natron-development-status/9505)): proved an OFX-based Nuke-alike is buildable, then stalled when the lead (INRIA-funded) left; community maintenance since. Performance never matched Nuke: CPU-only rendering, OpenGL used only for the viewer, cache/threading bugs ("deadlocks in the rendering code are the hardest bugs to fix" — their own docs), notoriously slow video (vs image-sequence) I/O because decode wasn't pipelined per §8. Lessons:
  1. **CPU-only is a dead end for the "fast" positioning** — GPU-first from day one, not retrofitted.
  2. **Threading + caching is where compositors die**: Natron's worst bugs were render-path deadlocks. Favour immutable graph snapshots, message passing, and lock-free hand-off over fine-grained locking; make cancellation cooperative and structural.
  3. Video decode must be an engineered pipeline, not "call ffmpeg per frame".
- **Blender compositor**: shipped tiled → learned it was slow → full-frame rewrite → GPU rewrite, each a multi-year migration. Lesson: **pick full-frame + GPU now**; retrofitting execution models is the most expensive rewrite there is.
- **MLT/Shotcut** ([MLT docs](https://www.mltframework.org/docs/framework/)): lazy producer→filter→consumer frame pipeline is elegant and stable, but its mostly-CPU, per-frame-object design caps effect complexity, and GPU (Movit) integration remained fragile/optional for years. Lesson: an abstraction where GPU residency is an afterthought forces CPU↔GPU ping-pong; make *texture residency the default* in the frame contract.
- **AE itself** is a cautionary tale from the other side: single-threaded legacy took Adobe a decade-plus to unwind (MFR shipped 2021 only after a multi-year thread-safety campaign across a 1993 codebase). Lesson: **concurrency and thread-safety contracts (including for plugins) are day-one architecture**, unaffordable to retrofit.

---

## Cross-cutting recommended architecture (summary for the design doc)

1. **Layers compile to an immutable, content-hashed DAG**; validate/request metadata pass (main thread, instant) separate from pixel evaluation (workers, cancellable); OFX-style ROI/DoD/identity contracts in the node API.
2. **Three-tier hash-keyed cache** (VRAM → RAM → disk) with cost-aware eviction, idle-time background pre-render adapting concurrency to measured cost (AE MFR) and idle scheduling (Resolve), timeline cache bars.
3. **fp16 premultiplied scene-linear working format** (fp32 opt-in), OCIO v2 with GPU-generated shaders, GPU compute scopes.
4. **Full-frame-per-node GPU execution with ROI/DoD-bounded allocations**; macro-tiling only as VRAM/TDR fallback (Blender's endpoint, skipping its detours).
5. **One portable GPU compute baseline (D3D12-first on Windows; wgpu/Vulkan if portability is a goal), CUDA/HIP only as optional per-node accelerators**; D3D11/12VA (+NVDEC where it wins) zero-copy decode into GPU textures; ffmpeg-wrapped NVENC/AMF/QSV encode; per-node CPU fallback with the CPU implementation as test oracle.
6. **Work-stealing pool + dedicated decode/IO/audio/GPU-submit threads; UI thread never evaluates**; epoch-based cancellation; latest-wins scrub with half-res preview, full-res refine on idle; audio clock masters playback.
7. **Resource governor with an explicit degradation ladder** (drop caches → drop res → tile → CPU) instead of OOM; device-lost treated as routine and recovered from caches; operation-journal autosave; out-of-process third-party (OFX) plugins over shared-memory/shared-GPU-handle IPC.
8. **Frame-indexed ffmpeg media layer** (keyframe index at import, per-clip persistent decoders, background proxies in DNxHR/ProRes), sample-accurate audio, image sequences first-class.
9. **Ship a usable vertical slice early; never big-bang rewrite** (Olive); GPU-first and threading-contracts-first (Natron, AE).

### Key sources
- Foundry NDK 2D architecture: https://learn.foundry.com/nuke/developers/90/ndkdevguide/2d/architecture.html
- Adobe MFR: https://helpx.adobe.com/after-effects/using/multi-frame-rendering.html · plugin threading: https://ae-plugins.docsforadobe.dev/effect-details/multi-frame-rendering-in-ae/
- Blender realtime compositor: https://code.blender.org/2022/07/real-time-compositor/ · full-frame project: https://projects.blender.org/blender/blender/issues/88150 · 4.2 notes: https://developer.blender.org/docs/release_notes/4.2/compositor/
- Resolve render cache manual: https://www.steakunderwater.com/VFXPedia/__man/Resolve18-6/DaVinciResolve18_Manual_files/part246.htm · GPU API choice: https://forum.blackmagicdesign.com/viewtopic.php?f=21&t=83494
- OCIO GPU shaders: https://opencolorio.readthedocs.io/en/latest/api/shaders.html
- Vulkan external memory/semaphore interop: https://docs.vulkan.org/guide/latest/extensions/external.html · CUDA interop API: https://docs.nvidia.com/cuda/cuda-driver-api/group__CUDA__EXTRES__INTEROP.html
- Vulkan Video status/FAQ (mpv): https://github.com/mpv-player/mpv/discussions/13909 · NVIDIA Video Codec SDK: https://developer.nvidia.com/video-codec-sdk
- DRED / device-removed: https://microsoft.github.io/DirectX-Specs/d3d/DeviceRemovedExtendedData.html
- ffmpeg seeking: https://fftrac-bg.ffmpeg.org/wiki/Seeking
- MLT framework design: https://www.mltframework.org/docs/framework/
- Olive history: https://en.wikipedia.org/wiki/Olive_(software) · Natron status: https://discuss.pixls.us/t/natron-development-status/9505 · Natron cache/deadlock notes: https://natron.readthedocs.io/en/rb-2.5/guide/getstarted-troubleshooting.html
