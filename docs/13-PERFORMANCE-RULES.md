# 13 · Performance rules

**Status: canonical.** Luminal's defining promise is buttery real-time preview and no crashes,
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

**The reference comp**, used by every budget and checked into the repository for CI: 1080p60,
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
| B9 | GPU device loss → preview resumed | ≤ 5 s | ≤ 5 s |
| B10 | A/V sync error during playback | ≤ ±½ video frame | ≤ ±½ video frame |
| B11 | Background cache fill of the 20 s work area from cold, while idle | ≤ 60 s | ≤ 240 s |

### 2.1 Document-scale budgets (the "thousands of layers" mandate)

After never-crashing, the project's founding grievance is that After Effects becomes
barely responsive in intensive projects. **Luminal's UI MUST remain fully interactive at
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
- B3 is the latest-wins path: epoch bump, degraded-quality request, cache lookup first. A
  cache hit MUST display in the next UI frame.
- B5 is the promise the cache bars make: green means it plays, full stop.
- B8's 2× is deliberate headroom, not a stretch goal: NVENC encodes 1080p60 far faster than
  realtime, so the budget really constrains evaluation throughput; a comp that previews in
  real time (B6) has no excuse exporting slower than 2× with deeper pipelining and no display.
- Budgets marked "reference comp" scale expectations, not guarantees, for other comps: a 4K
  comp with 40 layers may degrade — visibly, per §4 — but B1/B2 hold unconditionally.

## 3. The resource governor

One component owns memory. Nothing render-related allocates outside it.

- **Budgets**: defaults — VRAM: 70% of the DXGI-reported budget for dedicated GPUs, 40% of
  system RAM treated as the ceiling for shared-memory GPUs; RAM: 60% of physical RAM for the
  sum of caches, decode queues, and working buffers. Both user-overridable in preferences. The
  governor subscribes to DXGI video-memory budget-change notifications and to OS memory
  pressure, and shrinks its budgets live — Windows will demote VRAM allocations anyway when
  another application competes; Luminal yields before WDDM forces it.
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
- **Diagnostics**: every device loss is logged locally with the active node list and timing
  breadcrumbs. Dev and beta builds enable DRED (breadcrumbs + page-fault data) to attribute
  the offending dispatch; release builds keep lightweight per-node GPU timing so nodes that
  trend towards the timeout are pre-emptively tiled.
- **Repeated loss** (3 within a minute): the suspect node drops to CPU fallback for the
  session; if loss continues without a suspect, the session falls back to CPU rendering with a
  calm banner. Never a crash, never a dialogue loop.

## 6. Rules for effect authors

Binding for built-in WGSL effects and for plugins (KFX and OFX,
[12-PLUGINS.md](12-PLUGINS.md)); the host enforces what it can and sandboxes the rest.
Full API contract in [08-EFFECTS.md](08-EFFECTS.md).

- **Declare a cost class**: `trivial` (fused per-pixel), `local` (bounded neighbourhood,
  declared radius), `global` (whole-frame reach, e.g. FFT blur), `temporal` (samples other
  frames), `iterative` (cost scales with a parameter). The scheduler uses cost classes for
  concurrency, tiling, and degradation decisions; an undeclared effect is treated as
  `global`+`iterative` — the most pessimistic.
- **Support ROI**: implement `roi_in = f(roi_out)` honestly (06 §2.1). Claiming less reach than
  the kernel uses produces tile seams and is a correctness bug; claiming a whole-frame
  dependency when untrue forfeits the biggest optimisation in the pipeline.
- **Support cancellation checkpoints**: check the epoch token between passes and between tiles;
  a single uninterruptible span SHOULD stay under ~10 ms of GPU work on the reference desktop.
- **Respect memory ceilings**: declare peak scratch memory per dispatch as a function of ROI
  size; allocate scratch only through the host. The governor denies dispatches that exceed the
  declaration; exceeding it at runtime is a validation failure in dev builds.
- **Ship the CPU reference implementation** (K-019): it is the GPU version's test oracle and
  the fallback for §4 step 6. GPU and CPU outputs MUST match within a stated tolerance.
- **Be deterministic**: all randomness from host-provided seeds; no wall-clock, no global
  state. Same inputs, same output, always — the cache (06 §5.2) and deterministic export
  depend on it.
- **Declare thread safety** (KFX/OFX): a non-thread-safe plugin serialises its own node only;
  the host keeps the rest of the graph parallel and out-of-process plugins cannot take the
  application down.

## 7. Instrumentation

### 7.1 Per-node profiler

A built-in profiler, surfaced in the UI — After Effects' composition profiler done properly:

- Per-node CPU spans and GPU timestamp queries collected continuously at negligible cost,
  not only in a special mode.
- Timeline column: per-layer render time for the current frame, sortable, with effect-level
  drill-down in a profiler panel ([07-UI-SPEC.md](07-UI-SPEC.md)).
- Recording mode: capture over a playback or export run, then report per-node totals,
  percentiles, cache hit rates, and time spent per degradation-ladder step — answering "why is
  this comp slow" with names and numbers, not vibes.

### 7.2 Frame-drop and health telemetry — local only

A local ring log records dropped frames, budget breaches, ladder transitions, device losses,
and governor denials, with enough context to reproduce. An explicit "export diagnostics"
action writes it to a file the user can attach to a bug report. **Luminal never phones home**:
no automatic uploads, no analytics endpoints, no crash reporting service by default. This is a
GPLv3 project; diagnostics belong to the user.

### 7.3 Performance regression tests in CI

- The reference comp (§1) and a stress comp (4K, 20 layers, heavy effects) live in the
  repository with pinned media generated by script.
- A headless benchmark harness drives the real engine (no UI) through scripted scenarios:
  cold scrub, warm playback, cold playback, export, background fill — and emits the metrics
  behind budgets B3–B8 and B11.
- CI runs the harness on a reference-desktop-class runner per merge. Failing a budget fails
  the build; regressing more than 10% against the stored baseline on any metric fails the
  build even while still inside budget. Baselines update only by explicit commit.
- Unit-level: every built-in effect has a per-dispatch time and memory benchmark tied to its
  declared cost class; an effect that outgrows its class fails its own test, not just the
  end-to-end one.

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
- **A stress budget for expressions**: per-frame expression evaluation time has no budget yet;
  likely needs one once the expression engine (K-063) lands.
