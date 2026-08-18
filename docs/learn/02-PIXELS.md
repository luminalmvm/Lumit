# The picture: lumit-render, lumit-eval, lumit-cache

How a document snapshot becomes pixels. Three crates share the work:

- `lumit-render` — the shipped pixel pass: probe, plan, decode, build, realise,
  export. The bridge and the exporter both drive its `HeadlessRenderer`.
- `lumit-eval` — "Nova": frame naming (content hashes), the graph compiler, epochs,
  the worker pool, and the playback-scheduler decision core.
- `lumit-cache` — "Nebula": the byte-budget stores behind the VRAM, RAM and disk
  frame tiers.

Specs: [06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md),
[05-ARCHITECTURE.md](../05-ARCHITECTURE.md) §4–5. Impl notes:
`playback-scheduler.md`, `media-io.md`, `optical-flow.md`, `temporal-rerender.md`.

> **First pass:** Five steps turn a snapshot into a frame: probe, plan, decode,
> build, realise. Cache keys name a frame by its content, not its timeline position.
> Frames demote and promote through three tiers: VRAM, RAM and disk.
>
> Skip to [Nova's second half](#novas-second-half-built-but-not-yet-wired) for the
> demand-pull executor and the playback decisions.

## The five steps of a frame

`lumit-render/src/lib.rs` names them. Each step has a file.

```mermaid
flowchart LR
    P[probe<br/>source.rs] --> PL[plan<br/>plan.rs]
    PL --> D[decode<br/>decode.rs]
    D --> B[build<br/>build.rs]
    B --> R[realise<br/>realise.rs]
    R --> V[Viewer present /<br/>export sink]
```

1. **Probe** (`source.rs`) — once per footage item per session: is it video, audio-only,
   missing? Missing files plan a colour-bars slate. Unprobed footage makes the frame
   *unnameable* (rendered live, never cached).
2. **Plan** (`plan.rs`) — pure, opens nothing. Walks layers at time t, maps layer time
   through Retime, picks source frames, decides Blend/Flow interpolation, chooses a
   decode width from `Quality`. `same_decode` compares two plans. Identical pixels
   mean the renderer skips decode, which is the value-drag fast path.
3. **Decode** (`decode.rs`) — `DecodePool` owns per-item decoders (opened from the
   sidecar frame index), a decoded-frame `ByteLru` (512 MB default) and the flow
   cache. It decodes jobs plus temporal neighbours and runs optical flow where the
   plan asked for it.
4. **Build** (`build.rs`) — document + decoded pixels → `Vec<CompLayerDraw>`: resolved
   effect stacks, mattes, masks and paint baked in, collapse splicing, held or
   sub-frame below-stacks for Posterize Time and accumulation motion blur.
5. **Realise** (`realise.rs`) — `Realiser` walks the draw list on the GPU inside one
   submit per frame: upload → linearise → `fxops::run_ops` (each `Resolved` op becomes
   a kernel call) → lighting → matte/mask/motion-blur → `Compositor::composite_seeded`.

**Lighting sits between steps 4 and 5** of docs/06's numbering — step 4.5, after a layer's
effects and before it is placed (K-361). It is deliberately *not* an effect: it has no
schema, no stack row and no place in the Add-effect menu, because a light is a layer
elsewhere in the comp rather than a control on this one. `build.rs` resolves the comp's
Light layers and hangs the relevant ones on each draw; `lighting_op` in `realise.rs` turns
that into the pass's parameters, and returns `None` when the list is empty — which it is
unless the comp holds lights and this layer's **Accepts lights** switch is on. Empty means
the pass never runs, which is how a comp with no lights stays byte-for-byte what it was.

Two drivers call this. The bridge's worker thread owns a `HeadlessRenderer`
(`headless.rs`) for preview. `export::start` spawns its own thread with its **own**
renderer on its **own** GPU device. Both run the same walk at the same resolution
rules. A test matrix pins preview == export bit-for-bit (K-031).

## A region of interest

`Realiser::realise` is a thin call onto `realise_region` with no region.
Given one (K-362), the composite writes only that window of the composition: the
comp-pixels-to-NDC mapping shifts and the target is sized to the region, so the returned
texture is the region's size. The camera matrix is untouched — it projects comp space, and
a window on the result cannot change perspective — and pixel density is unchanged, so
every px@comp parameter in the frame still means what it meant.

Preview only. `HeadlessRenderer::set_region` is a Viewer setting; the exporter is never
handed one, the same way the preview scale and the Viewer's exposure never reach a file.

Two things about it are easy to get wrong:

- **Two stagings refuse the window.** An adjustment layer runs its effects on the composite
  of everything below it, and a motion-blurring layer averages sub-frame copies into a
  comp-sized texture. Both are written against the comp raster, so `region_is_safe` drops
  the region and the caller crops the finished frame instead. The picture is identical
  either way; only the work differs. A test asserts that equivalence, because working
  inside a region that quietly renders something else is working on a lie.
- **A region saves the composite, not the effect stack.** Effects run per layer at the
  layer's own size, untouched. Culling layers that miss the region is the upgrade path, and
  is not done, because a layer that appears to miss it can still reach it through an
  adjustment layer's blur.

Frames rendered through a region are still named: `named_under_view` folds the rectangle
into the key alongside the Viewer's look and the transparent-background flag. Refusing to
name them would empty the cache exactly when it is wanted, since scrubbing inside a region
is the whole use case. A region covering the whole comp is refused as no region at all, so
the ordinary case keeps sharing full-frame names.

## Naming frames: the content hash

A cache must never serve a stale frame, so keys describe *content*, not timeline
position. `lumit-render/src/cache.rs` implements `lumit_eval::SourceStamper` and
calls `lumit_eval::comp_frame_key` (`lumit-eval/src/lib.rs`). That key is a blake3
hash, truncated to a `u128`, over everything that can reach a pixel: evaluated
transform values (never raw keyframes), the live effect stack with evaluated
parameters, masks, mattes, camera pose, quality, and source stamps
(`{path}#w{decode_width}` plus source frame index).

Two subtleties carry most of the bugs:

- **Presence is gated, not hashed.** Visibility, in/out span and solo decide whether
  a layer feeds the hash at all. Editing a hidden layer keeps every cached frame.
  Tests pin that keys must NOT change.
- **The key only knows what it was taught.** A new content axis that the key never
  receives serves stale frames silently. The fix is to feed it AND bump
  `ALGO_VERSION` (`ALGO_VERSION` in `lumit-eval/src/lib.rs` documents versions 1→3).

## The cache tiers

| Tier | Store | Notes |
|---|---|---|
| VRAM | `ByteLru<(hash, bgra), FrameTexture>` in `headless.rs` | 512 MiB default. Evictions become non-blocking GPU readbacks (≤4 in flight), collected as `DemotedFrame`s |
| RAM | `ByteLru` (`lumit-cache/src/lib.rs`) | Byte budget, GreedyDual eviction: stale × large ÷ cost |
| Disk | `DiskCache` + `FrameIndex` (`lumit-cache/src/disk.rs`, `index.rs`) | `.kfr` files (LZ4 RGBA8), atomic temp-then-rename, 50 GB cap, owned by the `nebula-disk` thread (`lumit-render/src/diskio.rs`) |

Frames demote down the ladder and promote back up (`upload_frame_texture`). The
disk thread talks over mpsc channels. The render path never waits on IO bookkeeping
(`try_lock` mirrors).

## Cancellation

The engine force-kills nothing. Every task checks for cancellation and steps aside.

- **Epochs** (`lumit-eval/src/epoch.rs`) — an `Arc<AtomicU64>`. `bump()` invalidates,
  and workers call `token.check()` at ~10 ms granularity.
- **Generations** (`decode.rs`) — the decode worker drains its channel latest-wins
  and skips superseded requests.
- **`media_epoch`** — the receiver drops in-flight frames rendered under a stale
  probe state. Deliberately not the generation: background fills bump the
  generation too.
- Export checks an `AtomicBool` every frame. A queued flare bake drops the frame's
  cache name after compositing rather than banking a lie.

## Nova's second half: built but not yet wired

`lumit-eval` also contains the eventual demand-pull executor. `graph.rs` compiles a
comp into an `EvalGraph` DAG (identity folding, source dedup).
`exec::render_frame` walks that DAG against three trait sockets: `FrameSource`,
`KernelExecutor` and `CacheStore`. The executor therefore unit-tests with fakes,
with no GPU and no codecs. Today it drives real kernels only in
`lumit-gpu/tests/exec_skeleton.rs`. The shipped path is the draw-list renderer above.

Live from this crate today: `comp_frame_key` (all cache naming) and `schedule.rs`
(the realtime tier controller).

`schedule.rs` is deliberately pure, with no clocks and no threads, so its one rule is
a table test: `RealtimeController` (EWMA cost vs budget, which drops a preview tier
immediately above 0.9× budget and rises only after 12 consecutive frames under 0.4×).
`lumit-bridge/src/realtime.rs` runs one for the session and feeds it measured render
costs; it is the shipped adaptive-resolution picker.

The rest of the playback loop is in `lumit-bridge`, where the clock and the GPU are:
`playback.rs` has `CostWindow` (p95 over the last 32 renders) and `lookahead_frames`
(`clamp(2 × p95 × fps, 8, 16)`), and the worker's `Playback` owns the render-ahead
ring of `PreparedFrame`s with `present_choice` (front-on-a-grid for every-frame, newest
frame the clock has reached for adaptive) and `pre_roll_done` (audio waits for three
banked frames or 150 ms).

## Traps

- **Decode width and cache key are one policy.** Both round the display scale
  through `keyed_scale` (1% steps). When they diverged, footage got two names per
  frame. The cache bar then drew empty over a fully cached comp.
- **Parked disk writes need two questions**: `contains()` and `is_pending()`.
  Without the second, the idle backup re-offered the same frame forever (the 81 GB
  incident, K-277).
- **The parallel slot lists** (`lut_files`, `dof_inputs`, `flare_mattes`,
  `flare_lens_files`) are 1:1 with the stack's `Resolved` ops. A second filter
  anywhere silently binds LUTs to the wrong ops.
- **Unnameable beats misnamed.** When in doubt (unprobed footage, pending bake) a
  frame renders live and banks nothing. A content-keyed lie outlives every undo.
- Profiled renders bypass the VRAM cache and fence the GPU. Measuring is opt-in
  and never on during playback.
