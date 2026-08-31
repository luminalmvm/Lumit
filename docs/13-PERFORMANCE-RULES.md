# 13 · Performance rules

**Status: canonical.** Lumit's defining promise is buttery real-time preview and no crashes,
ever, even under absurd load. This document makes that promise falsifiable: named reference
hardware, numeric budgets against it, the single resource governor that owns memory, the
ordered degradation ladder, device-loss recovery, obligations on effect authors, and the
instrumentation that catches regressions. The pipeline these rules govern is
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md); threading and process layout are
[05-ARCHITECTURE.md](05-ARCHITECTURE.md). Key words MUST, SHOULD, MAY follow RFC 2119.

Decisions implemented here: K-017, K-018, K-019.

The two rules everything below serves:

1. **The user interface never waits for the engine.** The UI thread never evaluates anything
   (K-017); slowness appears as degraded pictures, never as a frozen application.
2. **Degrade, never crash** (K-018). Every resource exhaustion path ends in a visible quality
   reduction or a calm pause — never an abort, an OOM kill, or a modal error storm.

---

## 1. Reference hardware

Budgets are meaningless without a machine attached. Two named configurations; CI regression
gates run against the mid target (§7.3), and releases MUST meet the floor targets by manual
verification until a floor-class CI runner exists.

| | **Reference desktop** (mid target) | **Reference laptop** (floor) |
|---|---|---|
| CPU | 4 cores / 8 threads, ~3.8 GHz desktop class (i3-12100 class) | 4 cores / 8 threads ultrabook class (i5-1135G7 class) |
| GPU | NVIDIA RTX 3060, 12 GB | Integrated Iris Xe class, shared memory, DX12-capable |
| RAM | 16 GB | 16 GB |
| Storage | NVMe SSD | NVMe SSD |
| Display target | 1080p60 project, 60 Hz UI | 1080p60 project, 60 Hz UI |

Minimum spec remains K-019: Windows 10 20H2+, any DX12-capable GPU, CPU-only operation
functional (slowly) for every built-in effect.

**The reference comp**, used by every budget and built in code by the harness (§7.3, K-389),
over media it generates rather than media committed to the repository: 1080p60,
20 s. Five layers — two 1080p60 H.264 footage layers (one with a Retime ramp to 40% using flow
interpolation), one text layer, one Sequence layer with four clips, one adjustment layer
carrying a grade (3D LUT + curves). A glow on one footage layer. Motion blur enabled on two
layers. One luma matte. Audio layer with volume keyframes.

## 2. Budgets

All figures are 95th percentile unless stated; measured by the harness in §7.

| # | Budget | Reference desktop | Reference laptop |
|---|---|---|---|
| B1 | UI frame time during any interaction (drag, scrub, resize) | ≤ 8 ms | ≤ 8 ms |
| B2 | Input → first visual acknowledgement | next UI frame | next UI frame |
| B3 | Scrub: playhead move → first (possibly degraded) frame displayed | ≤ 50 ms | ≤ 100 ms |
| B4 | Idle → current frame refined to full chosen quality (reference comp) | ≤ 500 ms | ≤ 1500 ms |
| B5 | Warm cache playback (green bar), reference comp | 60 fps, 0 drops over 60 s | 60 fps, 0 drops over 60 s |
| B6 | Cold cache playback, reference comp, adaptive degradation allowed | sustained 60 fps | sustained ≥ 30 fps |
| B7 | Cold cache playback, reference comp, Full resolution, no degradation | ≥ 24 fps | ≥ 10 fps |
| B8 | Export of the reference comp (YouTube 1080p60 preset, hardware encode) | ≥ 2× realtime | ≥ 0.5× realtime |
| B9 | GPU device loss → preview resumed | ≤ 5 s | ≤ 5 s |¹
| B10 | A/V sync error during playback | ≤ ±½ video frame | ≤ ±½ video frame |
| B11 | Background cache fill of the 20 s work area from cold, while idle | ≤ 60 s | ≤ 240 s |
| B12 | Particulate, default parameters (≈ 300 live particles), evaluate + draw **above the pass floor** | ≲ 0.2 ms | ≲ 0.6 ms |
| B13 | Particulate, 20 000 live discs at the default cap, evaluate + draw above the pass floor | ≤ 1 ms | ≤ 4 ms |
| B14 | Particulate at the 1 000 000 hard cap, evaluate + draw above the pass floor, one comp frame | ≤ 16 ms | — |
| B15 | Puppet warp of a **fully covered** 1080p layer, single-threaded (≈ 40 ns a pixel warped) | ≤ 120 ms | ≤ 300 ms |
| B16 | Puppet mesh build at natural 1080p, fully covered, default density | ≤ 100 ms | ≤ 250 ms |
| B17 | Puppet per-frame solve at the 1500-vertex cap, factorisation held | ≤ 12 ms | ≤ 30 ms |

### 2.1 Document-scale budgets (the "thousands of layers" mandate)

After never-crashing, the project's founding grievance is that After Effects becomes
barely responsive in intensive projects. **Lumit's UI MUST remain fully interactive at
document scale**, independent of render load. The reference *stress document* for these
budgets: 200 comps, 5,000 layers total (one comp holding 1,000), 250,000 keyframes,
2,000 footage items.

| # | Budget | Both reference machines |
|---|---|---|
| S1 | B1 (8 ms UI frame) holds against the stress document — timeline scroll/zoom, layer select, twirl-down, box-select of 10,000 keyframes | ≤ 8 ms |
| S2 | Committing an edit (one op) with the stress document open | ≤ 16 ms |
| S3 | Undo/redo of any single op, stress document | ≤ 16 ms |
| S4 | Open the stress document (.lum → interactive) | ≤ 5 s |
| S5 | Save the stress document | ≤ 2 s, non-blocking UI |
| S6 | Graph editor open on a property with 50,000 keyframes: pan/zoom/box-select | ≤ 8 ms/frame |

Consequences the architecture must honour (and known debts):

- Timeline, Project panel, and graph editor MUST be **virtualised** — draw only visible
  rows/keys; cost scales with what's on screen, never with document size.
- Property/keyframe lookups MUST be indexed; no O(all-layers) walks inside the UI frame.
- **Known debt, tracked here until paid:** the Phase 0 `DocumentStore` clones the whole
  document per op — O(document) commits. Fine now, fails S2 at stress scale. Before the
  Phase 1 gate, commits move to structural sharing (`im`-style persistent collections or
  per-item copy-on-write via `Arc`) so an edit copies only the touched path. S2/S3 tests
  land with that change and hold the line thereafter.
- The stress document is generated deterministically by a fixture builder in the perf
  harness (§7) so S-budgets run in CI like every other gate.

Notes:

- B1 is the UI thread alone: layout, paint, input. It holds regardless of engine load because
  the UI thread never evaluates, never blocks on a render, and reads results from lock-free
  mailboxes only. Any UI-thread stall > 16 ms is a bug regardless of budget.
- **B1 and B2 are also the owner's recorded mandate** (K-676, 2026-08-30), restated as the
  60/120 rule: interacting with anything answers on the **next frame** (the preview picture
  excluded — it may degrade, §4), the interface **never drops below 60 fps** during an
  interaction (16.6 ms a frame is the floor and any breach a defect), and every interaction
  frame is budgeted at **8.3 ms** so a high-refresh display (120–165 Hz) is actually fed.
  The rule holds on the owner's real documents **in the owner's own conditions** — window
  maximised, the preview showing real frames — because the measured difference between those
  conditions and a small test window over an empty preview is a factor of four in frame
  rate. An explicit 120 fps energy cap is
  the ideal; Flutter exposes no frame-rate cap on Windows today, so the enforceable halves
  are "draw nothing at rest" (an idle editor schedules zero frames — measured, and a
  regression is a defect) and the 8.3 ms budget itself.
  [impl/ui-performance.md](impl/ui-performance.md) is the binding note: the measured
  gesture table, the paint-layer architecture, and the work packages whose gates enforce
  this panel by panel. **What enforces the 60/120 rule in CI is that note's §4.2 repaint
  matrix, pinned gesture by gesture in `rebuild_budget_test.dart`** (K-681): idle, select,
  scroll, zoom, playhead drag, work-area drag and edit each carry a rebuild and paint-count
  budget that fails on regression, because the milliseconds the rule is written in are the
  cost of those counts. The milliseconds themselves stay on the manual list below — a
  widget test has no compositor — and are read with the note's probe in the owner's
  conditions.
- B9¹ **has a mechanism now, not a number** (K-585): loss is noticed, the renderer is rebuilt
  and the picture republished — see §4's "what is built today". The five seconds still cannot
  be *measured* without a real device to lose, so B9 stays on the manual list below with B1,
  B2, B8 and B10. What CI does hold is that the recovery path works at all: the device is
  destroyed for real and a rebuilt renderer draws the identical frame
  (`crates/lumit-render/tests/device_loss.rs`).
- B3 is the latest-wins path: epoch bump, degraded-quality request, cache lookup first. A
  cache hit MUST display in the next UI frame.
- B5 is the promise the cache bars make: green means it plays, full stop.
- B8's 2× is deliberate headroom, not a stretch goal: NVENC encodes 1080p60 far faster than
  realtime, so the budget really constrains evaluation throughput; a comp that previews in
  real time (B6) has no excuse exporting slower than 2× with deeper pipelining and no display.
- Budgets marked "reference comp" scale expectations, not guarantees, for other comps: a 4K
  comp with 40 layers may degrade — visibly, per §4 — but B1/B2 hold unconditionally.
- B12–B14 are the four numbers K-475 makes Particulate's own, and they are per-effect rather
  than per-comp because **Max particles is the user's budget dial**: the cap is the declared
  peak scratch (§6), so an instance's cost is a number the document states rather than one
  the governor has to guess. B12 says the effect is free to drop on. B13 says the default cap
  plays. B14 says the hard cap **degrades rather than stalls** — one comp frame, not real
  time, and the pass checks cancellation between its evaluate and its draw. The fourth of
  K-475's claims is not a millisecond and so is not a row: under governor pressure the effect
  draws the **newest `cap/2`**, halving again as pressure demands, which is the cap rule
  applied a second time — deterministic, identical from any scrub direction, and **never on
  the export path** (docs/06 §6.2). It is gated as a correctness test, not a timing one:
  `particulate_exports_its_whole_declared_field` renders the export walk and the interactive
  walk of one Particulate comp and a second comp declaring half the cap, and holds the first
  two identical and both different from the third. Today nothing *can* set the cap below the
  declared one — `ParticulateOp::cap` is filled in one place, from the schema value — and
  that test is what makes it a guarantee rather than a coincidence.
- **The three rows are measured above the pass floor**, and that is a resolution rather than
  a convenience. The harness times a fourth fixture first — the same call with nothing to
  emit, which is one full-frame copy and one round trip to the queue — and subtracts it. The
  floor is real work, but it is the *frame's*: every effect in the stack pays it, and no
  particle count changes it. Measured whole, B12 came in at 0.266 ms against a ≲ 0.2 ms
  budget with 0.062 ms of that being the copy, and there were only two honest ways to close
  the gap: raise the number, or stop charging the effect for the frame's paperwork. Raising
  it would have been the silent loosening; this is the other one, and it makes B12, B13 and
  B14 comparable with one another and with the closed forms they time. §7.3's other
  convention applies on top and needs no exception: B12 and B13 land under a millisecond, so
  the **ratio gate does not fire on them** — a factor at that size is scheduler noise. What
  holds them is the absolute assertion on the reference desktop, and the floor row beside
  them, which is where the copy itself getting dearer would show.
- **B15–B17 are the puppet's three** (K-704, K-712), measured on the *pathological* fixture
  on purpose: a 1920×1080 layer **fully covered**, which is the shape puppet is not for. A
  cutout — an arm, a character, the thing the tool exists for — covers a fraction of that
  and costs the same fraction, because the warp only touches the pixels the mesh covers.
  Read B15 as its rate, ≈ 40 ns a pixel warped: a 400×600 cutout warps in about 10 ms and
  scrubs inside B3.

  Two of the three are looser than `docs/impl/puppet.md` first wrote them, and the reason
  is arithmetic rather than accident (K-712 records it). The **warp** is one bilinear
  resample and one barycentric per pixel in f64; two million of those do not fit 8 ms on
  one thread, and the note's 8 ms is what the GPU warp it already names as the upgrade path
  is *for*. The **solve** at the vertex cap is a forward and back substitution through a
  dense 3000×3000 factor plus two 1500×1500 ones — about eighteen million multiply-adds,
  which is milliseconds, not the microseconds the note estimated; the sparse factorisation
  it already names is what closes that. Both upgrade paths carry an observable trigger in
  the note, and both triggers have now fired: what these rows do until then is stop the two
  numbers getting *worse*, which is what the ratio gate is for.

## 3. The resource governor

One component owns memory. Nothing render-related allocates outside it.

- **Budgets**: defaults — VRAM: 70% of the reported card memory for dedicated GPUs, 40% of
  system RAM treated as the ceiling for shared-memory GPUs; RAM: 60% of physical RAM for the
  sum of caches, decode queues, and working buffers. Both user-overridable in preferences. The
  governor subscribes to DXGI video-memory budget-change notifications and to OS memory
  pressure, and shrinks its budgets live — Windows will demote VRAM allocations anyway when
  another application competes; Lumit yields before WDDM forces it.
- **Where "the reported card memory" comes from** (K-582): each platform is asked through
  the graphics API the engine already links, and each answers a *ceiling*, never a live
  free-space figure. Windows: the first DXGI adapter's `DedicatedVideoMemory`. macOS:
  Metal's `recommendedMaxWorkingSetSize`, which on Apple Silicon is a share of the unified
  memory rather than a separate pool — which is exactly the ceiling wanted. Linux: the
  largest **device-local** Vulkan memory heap, read through the adapter the renderer opens.
  Largest, not the sum: a discrete card commonly reports a second, small device-local heap
  (the host-visible BAR window) that is a view of the same memory, and adding it would count
  part of the card twice. Every one of them errs low, which is the safe direction for a
  budget. A platform or build that cannot be asked answers **0**, and the frontend falls
  back to its own documented ceiling rather than pretending — Linux answers 0 until a
  renderer has opened an adapter, for the same reason.
- **Accounting**: every frame-sized allocation (cache entries, node output textures, decode
  buffers, ring buffers, staging) is registered with size, tier, and owner. The governor's
  ledger MUST equal reality; an unaccounted frame allocation fails code review
  ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)). Allocation requests are grant/deny —
  a deny triggers the ladder (§4), never an OS-level OOM.
- **Bounded queues everywhere**: decode queues (2–4 frames per stream), the render-ahead ring,
  GPU submission batches, IO write-behind queues, mailboxes. No unbounded channel exists in
  the render path; back-pressure is structural, not advisory.
- **Pools**: texture and buffer allocations come from governor-owned pools with aliasing;
  per-node lifetimes derive from the compiled graph's refcounts
  ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §2.2).

## 4. The degradation ladder

When the governor denies an allocation, a budget is breached, or measured throughput falls
below the playback requirement, the engine steps down this ladder **in order**, one step at a
time, taking the cheapest step that resolves the pressure. Every active step is user-visible in
the **status readout** — a small chip in the Viewer corner plus a line in the status bar (e.g.
"Half resolution · background caching paused"). Silent degradation is a bug.

1. **Pause background cache fill.** Idle-time work yields first; interactive work is untouched.
2. **Evict cold cache.** Cost-aware eviction (06 §5.3) beyond its steady-state rate: distant
   frames, cheap intermediates, VRAM→RAM demotions.
3. **Drop the preview resolution tier.** Auto/current tier steps down (Full→Half→Quarter),
   during interaction and playback only. The chosen tier in the Viewer is not changed; the
   readout shows the effective tier.
4. **Macro-tile the frame.** Split evaluation into 2–4 tiles (06 §2.2), trading latency for
   peak VRAM.
5. **Swap flow interpolation to blend during interaction.** Retimed clips using flow synthesis
   temporarily render with blend interpolation; export and idle refinement still use flow.
6. **CPU fallback per node.** The scheduler moves the offending node(s) to their CPU reference
   implementations with readback/upload bridges; the rest of the graph stays on the GPU.
7. **Pause playback with a calm banner.** "Playback paused — this composition needs more memory
   than is available. Lower the preview resolution or close other applications." One banner,
   dismissible, no modal, no error storm, project intact, editing still live.

Steps reverse in the opposite order once pressure clears, with hysteresis (a step must be
clear for ~2 s before reversing) so the ladder never flaps. Export ignores steps 3 and 5
entirely — under pressure export slows down; it never changes output
([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §7.3).

## 5. GPU device loss and TDR

Device loss is routine, not exceptional. Windows resets the driver on any GPU packet exceeding
~2 s (TDR); other causes include driver updates and physical GPU removal. Rules:

- **No dispatch may approach the timeout.** Effect dispatches are sized so the expected worst
  case on minimum-spec hardware stays under ~500 ms; macro-tiling (§4 step 4) doubles as the
  enforcement mechanism for pathological parameter values.
- **Recovery path**: all GPU objects belong to a device-epoch object. On loss: tear down the
  epoch, recreate the device, re-upload from RAM/disk cache — the lower cache tiers are the
  recovery data by design — recompile pipelines from the shader cache, replay the current
  request. The user sees a hiccup and a status readout entry, within budget B9. In-flight
  export items resume from the last completed frame.
- **What is built today** (K-585). The "device-epoch object" is the renderer itself: a
  `HeadlessRenderer` owns its device and everything made from it, so tearing down the epoch
  *is* dropping the renderer. wgpu's device-lost callback raises a flag on the context
  (`GpuContext::device_lost`); the render worker reads it at the top of every turn, before
  anything else asks the renderer for anything, and rebuilds on the K-434 build road — the
  same turn-taking and the same session settings, with the Viewer's look put back on the new
  renderer so a reset is not also a *view* reset. The VRAM tier went with the device, so its
  budget is applied again and its published figures return to zero; the RAM and disk tiers are
  untouched and are what refills it. The worker then republishes the frame the Viewer was
  showing, because from the frontend's side nothing happened and nothing will be asked for,
  and sends `WorkerResponse::DeviceReset` — one calm status line. The export path's own
  renderer recovers by the same rule, by returning its slot to "not yet built". Still owed
  from the paragraph above: the shader-cache recompile, the DRED diagnostics, the repeated-loss
  CPU fallback, and export items resuming mid-item.
- **Diagnostics**: every device loss is logged locally with the active node list and timing
  breadcrumbs. Dev and beta builds enable DRED (breadcrumbs + page-fault data) to attribute
  the offending dispatch; release builds keep lightweight per-node GPU timing so nodes that
  trend towards the timeout are pre-emptively tiled.
- **Repeated loss** (3 within a minute): the suspect node drops to CPU fallback for the
  session; if loss continues without a suspect, the session falls back to CPU rendering with a
  calm banner. Never a crash, never a dialogue loop.

## 6. Rules for effect authors

Binding for built-in WGSL effects and for plugins (LFX and OFX,
[12-PLUGINS.md](12-PLUGINS.md)); the host enforces what it can and sandboxes the rest.
Full API contract in [08-EFFECTS.md](08-EFFECTS.md).

- **Declare your traits honestly** — cost class, ROI expansion, temporal window, alpha mode,
  cancellation points, randomness ([08-EFFECTS.md](08-EFFECTS.md) §1.3, which owns the
  vocabulary). The scheduler plans concurrency, tiling and degradation from them; an
  undeclared effect is treated as the most pessimistic case. Claiming less reach than the
  kernel uses produces tile seams and is a correctness bug; claiming a whole-frame
  dependency when untrue forfeits the biggest optimisation in the pipeline.
- **Support cancellation checkpoints**: check the epoch token between passes and between tiles;
  a single uninterruptible span SHOULD stay under ~10 ms of GPU work on the reference desktop.
- **Respect memory ceilings**: declare peak scratch memory per dispatch as a function of ROI
  size; allocate scratch only through the host. The governor denies dispatches that exceed the
  declaration; exceeding it at runtime is a validation failure in dev builds.
- **Ship the CPU reference implementation** (K-019): it is the GPU version's test oracle and
  the fallback for §4 step 6. GPU and CPU outputs MUST match within the tolerance
  [08-EFFECTS.md](08-EFFECTS.md) §1.6 states for it.
- **Be deterministic** ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md) §3): the content
  cache (06 §5.2) and deterministic export both depend on it, so a breach here is a
  performance defect as well as a correctness one.
- **Declare thread safety** (LFX/OFX): a non-thread-safe plugin serialises its own node only;
  the host keeps the rest of the graph parallel and out-of-process plugins cannot take the
  application down.

## 7. Instrumentation

**v1 status:** this section is the target. What runs in CI today is fmt, clippy, the full
test suites on macOS/Windows/Linux, the engine-crate coverage gate and the no-hex lint
(`.github/workflows/ci.yml`); the per-node profiler's first visible piece is built (§7.1's
per-layer and per-effect indicators, K-276) and the rest of it — continuous collection, the
recording mode, the profiler panel — and the headless benchmark harness with budget gates
(§7.3) are not — [TODO.md](TODO.md) tracks them.

### 7.0 Submissions per frame (K-290)

**A frame hands the graphics driver one command buffer, and the number does not grow with the
layer count.** A submit is a round trip through the driver whose cost does not depend on the
card, so this is a budget that means the same on every machine — and one that can be *gated*
rather than benchmarked: `GpuContext::submits_so_far` counts every submission, and the
regression test asserts the shape rather than a magic number ("adding thirty-one layers adds
no submissions"). It is checkable on the software rasteriser CI runs, where a timing would
prove nothing.

The exceptions are deliberate and each is followed by a fence, which is the one thing batching
cannot defer: the read-backs, the scope trace, and the shared-texture present paths. A
**measured** frame is the other exception and gives the batching up on purpose — see §7.1.

**The count belongs to the context, not to the process.** It began as one global atomic, and
that made the gate report a *shared* number: the suite runs its cases in parallel, each with a
renderer of its own, so any other test rendering between the two reads was counted as this
render's work. The gate went red on CI — where there are cores enough for the overlap — while
passing on a quieter machine, which is the worst way for a test to be wrong. The counter now
lives on `GpuContext` and is shared only with the handles of that same device, so what a
measurement sees is one renderer's own submissions. Any future budget counted this way MUST be
scoped the same: a number two tests can both write is not a measurement.

### 7.0.1 The memory report (K-294)

**Every tier that holds memory MUST report its bytes, and the process MUST report its
total, in one place the user can read.** Settings ▸ Performance ▸ Memory shows what the
operating system says Lumit is holding, what each byte-budgeted tier admits to, how many
media decoders are open, and — the figure the section exists for — **what is left over**.

This is a diagnostic obligation, not a nicety. Lumit has twice been reported holding tens
of gigabytes (K-277, and again after it), and both times the first question — *is a cache
doing what it was told, or is something holding memory nobody counts?* — took days to
answer from outside the process. It is one syscall and five atomics from inside. A report
whose tiers sit at their budgets while the process is a hundred times larger says the
search is not in this list, which is the most valuable thing it can say.

Rules the report keeps, so its arithmetic can be trusted:

- **VRAM is reported, never subtracted.** On unified memory (every Apple Silicon Mac) the
  card's frames are part of the process; on a discrete card they are not. Folding them in
  either way would be wrong on half the machines Lumit runs on.
- **The graphics driver reports what it holds.** Two ways, because one of them does not
  exist everywhere. **Live objects** — how many textures and buffers the driver still has
  — are kept by every backend, Metal included, and a handful at rest against thousands is
  the difference between a cache doing its job and frames the engine dropped never being
  destroyed. **Bytes in use and reserved** come from the allocator report, which is Vulkan
  and D3D12 only: on macOS it answers nothing at all, so that row is not drawn there
  rather than printing zeroes somebody might reason from. The first draft of this reported
  only the bytes, and on the one platform the question had been asked on it read *"not
  reported by this driver"* — a hole is worth knowing about, but a report that only works
  where there is no problem is not an instrument.
- **Nothing is counted twice.** A frame waiting in the write-behind queue shares its
  allocation with the frame cache (one `Arc`, both tiers), so the queue reports a *count*
  of frames rather than bytes.
- **What cannot be weighed is counted.** What an open media decoder holds is FFmpeg's and
  the driver's business; the report says how many are open rather than inventing a size.
- **A platform that cannot answer says zero**, and the interface says "not known here"
  rather than printing a guess.

### 7.0.2 Reclaiming what has been dropped (K-295)

**An engine that renders without presenting MUST maintain its graphics device on a
schedule of its own.** Dropping a texture or a buffer only *marks* it destroyed; the
driver hands the memory back on the device's next maintain. A renderer that draws to a
window gets those for free from presenting — Lumit renders into caches, on a worker
thread, and idles, so it gets none.

The worker calls `GpuContext::reclaim` (a non-blocking `Maintain::Poll`) once per turn.
It is cheap when there is nothing to drain, and it makes reclamation a property of time
passing rather than of the user happening to open a panel — which is exactly what was
observed before it: 5 500 live buffers and 6 GB held, then 8 buffers and 2.9 GB the moment
something else polled.

Anything that frees memory only as a side effect of an unrelated call is not freeing
memory. The regression gate is `what_the_engine_drops_the_driver_gets_back`, which renders
many times the cache's capacity and then asks the driver how many objects it still holds —
**twice, a batch apart**, and compares the two.

Both readings are taken through `GpuContext::settle`, the blocking sibling of `reclaim`,
and that is not a detail. Work is submitted and runs later, so a CPU that has run ahead of
the card is still holding every frame the card has not reached; a non-blocking poll cannot
free those, however many times it is called. Reading there reads the backlog, and the
backlog grows with the frame count — which is what this gate did when it was first written,
reporting 113 live textures on Metal and 577 on D3D12 against 18 on the software
rasteriser, where the CPU never gets ahead and nothing looked wrong. What is held once the
queue is empty is what is genuinely held.

The comparison is the assertion, not a ceiling. How many objects a backend rests on is a
fact about that backend; memory that is dropped and never handed back grows by one object
per frame on every backend there is.

### 7.1 Per-node profiler

A built-in profiler, surfaced in the UI — After Effects' composition profiler done properly:

- Per-node CPU spans and GPU timestamp queries collected continuously at negligible cost,
  not only in a special mode. **Not what is built (K-276):** the shipped measurement fences —
  it waits for the graphics card at each node before reading the clock, because GPU work is
  *submitted* rather than performed and an unfenced span would time the paperwork. That is a
  true per-node number at the cost of the processor/card overlap for the frame measured, so
  it is never on during playback, and it is **on by default** (K-276 revision 8) with the
  clock in the bottom strip as the switch for the session — only the frame the user is
  waiting on is measured. Measuring also gives up the one-command-buffer-per-frame batching
  (§7.0, K-290) and hands work over layer by layer, because a fence over a queue that has
  not been submitted waits for nothing — the same processor/card overlap, paid in a second
  place. Only a **composited** frame yields numbers, and a frame served from a tier costs
  a copy and so has none to give — but the cache ladder is not stepped over for it:
  **the held frame is served at once and composited again, measured, on the next idle
  turn** (K-420), so the picture is never made to wait for its own numbers. Timestamp
  queries are what would make it continuous and free; TODO tracks the upgrade, and until
  then the honest description of this rung is "measured when asked".
- Timeline column: per-layer render time for the current frame, sortable, with effect-level
  drill-down in a profiler panel ([07-UI-SPEC.md](07-UI-SPEC.md)). **Built (K-276):** the
  column shows each layer's own picture (its source — a Precomp's whole comp included — and
  its effect stack), and each effect's cost on its heading row in the fold-out and on its
  title row in the Effect controls panel. Only the top-level layers of the composition being
  rendered are timed, and the final composite — one pass over the whole stack, not a
  per-layer act — lands in the frame total rather than on a row. **The total explains
  itself by stage**: the profile carries wall-clock per render stage (plan, decode, build,
  composite, present — `BridgeFrameProfile`'s five stage fields), so time no layer owns —
  a heavy draw-list build, a slow decode — is named in the header's readout instead of
  hanging unattributed over the rows. No budget row changed meaning: the total and the
  per-layer numbers are what they were, the stages only partition the total. Sorting by
  the column, and the profiler panel proper, are not built.
- Recording mode: capture over a playback or export run, then report per-node totals,
  percentiles, cache hit rates, and time spent per degradation-ladder step — answering "why is
  this comp slow" with names and numbers, not vibes.

### 7.2 Frame-drop and health telemetry — local only

A local ring log records dropped frames, budget breaches, ladder transitions, device losses,
and governor denials, with enough context to reproduce. An explicit "export diagnostics"
action writes it to a file the user can attach to a bug report. **Lumit never phones home**:
no automatic uploads, no analytics endpoints, no crash reporting service by default. This is a
GPLv3 project; diagnostics belong to the user.

### 7.3 Performance regression tests in CI

**Built (K-389).** `crates/lumit-bench` is the harness: a development crate nothing in the
application depends on, run by the CI job **`performance gates (ratio vs baseline)`**.

- **The reference comp of §1 is built in code**, over synthetic clips the harness asks
  ffmpeg for when it starts, rather than committed as media. It is assembled through
  `lumit-core` and `lumit-render` directly — no bridge, no Flutter.
- **Six scenarios, each one timed measurement** emitting one JSON line
  (`{"budget":"B3","value_ms":…,"frames":…}`): B3 scrub latency, B4 refine-to-full, B5 warm
  playback, B6 cold adaptive playback, B7 cold full-resolution playback, B11 idle fill of
  the twenty-second work area. Latency budgets are reported at the 95th percentile, playback
  budgets as milliseconds per frame. Beside them, **three per-effect scenarios** — B12, B13
  and B14, which time one Particulate pass rather than a comp and so need neither media nor
  the reference comp — and **three per-feature ones**: B15, B16 and B17, which time the
  puppet's warp, mesh build and per-frame solve. Those three are pure processor work, so
  unlike every other row they need no graphics adapter either and measure on any machine
  the suite runs on. The binary runs all twelve and writes the file; each is also an
  `#[ignore]`d test, for measuring one budget while working on it.
- **The harness measures; CI gates on the ratio to a baseline.** A GitHub runner is not §1's
  reference desktop — its graphics card is a software rasteriser — so a run is compared with
  a checked-in baseline **for that runner's operating system**
  (`crates/lumit-bench/baselines/<os>.json`, a previous run's own output) and fails at 1.6x
  worse. Baselines are regenerated exactly like `crates/lumit-core/fx-labels.txt`: run the
  harness, replace the file, commit on purpose. On a software rasteriser the span-scaled
  scenarios (B6, B7, B11) measure a fraction of the work area (`BENCH_SPAN_FRACTION`,
  stamped into every results file): the full span burned CI's whole time limit on the fill
  alone. A run and its baseline must carry the same fraction — the compare refuses a
  mismatch outright, exactly as it refuses a foreign operating system. A measurement under 1 ms is not gated on
  ratio (a warm frame costs microseconds, where a factor is noise), and a baseline from
  another operating system is refused rather than used. Where a baseline does not exist yet
  the job prints the fresh numbers, warns, and does not pretend to be a gate.
- **The absolute budgets of §2 are asserted only under `LUMIT_REFERENCE_HW=1`** — §1's
  reference desktop, or a future self-hosted runner. The budgets above remain the truth; the
  ratio gate is the enforcement an ordinary runner can honestly carry. K-389 records the
  split, and setting that variable on a pinned machine is the whole of switching this
  paragraph back to the original design.
- **Five budgets are out of a headless harness's reach** and stay manual or real-window
  checks ([TODO.md](TODO.md) keeps them): B1 and B2 need the UI thread and a real window —
  the parked probe of [impl/ui-performance.md](impl/ui-performance.md) §6 is their named
  manual instrument until a real-window harness exists — B8 needs the encoder, B9 is device
  loss, B10 is A/V drift.

**Still owed.** A stress comp (4K, 20 layers, heavy effects) beside the reference one. A
reference-desktop-class runner, on which the 10%-against-baseline rule replaces the interim
1.6x and the budgets themselves gate. Unit-level benchmarks: every built-in effect with a
per-dispatch time and memory measurement tied to its declared cost class, so an effect that
outgrows its class fails its own test rather than only the end-to-end one. **B12–B14 are the
first three of those and they are built** (PS7): `scenarios::particulate` times one pass at
1080p rather than a whole comp, needs no media and no reference comp, and emits its three
lines beside the other six — so they ride the same baseline file, the same ratio gate and
the same `LUMIT_REFERENCE_HW` switch as everything else. §2's note records why they are
measured above the pass floor and why the ratio gate is quiet on the two that sit under a
millisecond.

## Open questions

- **Floor-class CI**: budgets for the reference laptop are currently manual release checks;
  find or build an Iris Xe-class runner so B-column two is CI-enforced too.
- **Export parallelism cap**: how many export items may run concurrently with interactive
  editing before the governor should refuse to start another rather than degrade both.
- **fp32 comps on the floor machine**: whether fp32 opt-in (K-026) carries relaxed budgets or
  simply engages the ladder earlier; needs measurement.
- **Ladder step 5 scope**: swapping flow→blend during interaction is per-clip; whether it
  should also apply to flow-based effect nodes (RSMB-class blur) or only Retime interpolation.
- **Thermal throttling on laptops**: sustained-playback budgets assume steady clocks; decide
  whether B5/B6 on the floor machine are measured after a 10-minute soak.
- **A stress budget for expressions**: the engine has landed (Rhai, K-305) and per-frame
  expression evaluation time still has no budget. Two separate things are missing — a
  *performance* budget gating merges, and a *runtime* interrupt so one expression cannot
  stall a render thread ([12-PLUGINS.md](12-PLUGINS.md) §4.4). For scale: a pooled engine
  evaluates a typical expression in about 1µs, so a 60fps frame holds roughly fifteen
  thousand.
