# Luminal engineering rules

**Status: canonical and binding.** These rules apply to every line of code in this repository,
whether written by a human or by an AI agent. They exist because the two product requirements —
responsive under any load, never crashes — are architectural properties that erode one careless
commit at a time. RFC-2119 keywords (MUST, MUST NOT, SHOULD, MAY) are binding. Terminology
follows [01-GLOSSARY.md](01-GLOSSARY.md); architecture context is
[05-ARCHITECTURE.md](05-ARCHITECTURE.md); runtime degradation policy is
[13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md).

A rule here beats convenience, beats performance micro-wins, and beats "it works on my
machine". Exceptions require a decision entry in [02-DECISIONS.md](02-DECISIONS.md).

---

## 1. Concurrency contracts

### 1.1 What may run where

| Work | Allowed threads |
|---|---|
| Document edits, snapshot publication | UI thread only |
| egui, winit, painting | UI thread only |
| Evaluation-graph pixel jobs | Worker pool only |
| Metadata pass | UI thread (edit-triggered) or workers (request-triggered) |
| Media decode | Dedicated decode threads only |
| Disk IO (cache, journal, proxies, export files) | Dedicated IO threads only |
| wgpu queue submission | GPU-submit thread only |
| Audio graph evaluation | Audio-render thread only |
| cpal callback body | Lock-free ring-buffer reads only; no allocation, no locks, no logging |

- The UI thread MUST NOT evaluate any node, decode any frame, run any expression, perform
  blocking IO, or wait on any render result (K-017). It reads latest-wins mailboxes.
- Workers MUST NOT touch the live document. They read the immutable snapshot their job was
  created with.

### 1.2 Send/Sync discipline

- Types crossing thread boundaries MUST be `Send`; shared read types MUST be `Sync` by
  construction (immutable), not by interior locking. `unsafe impl Send/Sync` is forbidden
  outside `luminal-gpu` and FFI boundary crates, and there requires a safety comment plus a
  test exercising the cross-thread path.
- Prefer message passing and snapshot sharing over shared mutable state. A new `Mutex` or
  `RwLock` in `luminal-eval`, `luminal-core`, or `luminal-cache` hot paths requires review
  sign-off naming who holds it, for how long, and why a channel does not work. Natron died
  of render-path locks; see [05-ARCHITECTURE.md](05-ARCHITECTURE.md) §8.

### 1.3 Locks across boundaries

- A lock MUST NOT be held across: an `.await`, a GPU submit or readback wait, a channel send
  that can block, an FFI call into ffmpeg/OFX/CUDA, or a call into plugin IPC.
- Lock scope SHOULD be a lexical block small enough to read at a glance. Double-lock
  acquisition (holding one while taking another) requires a documented ordering.

### 1.4 Cancellation and progress

- Every loop over frames, pixels rows/tiles, clips, or graph nodes MUST check its epoch token
  at each iteration boundary (`ctx.is_cancelled()`); the idiomatic form returns
  `Err(Cancelled)` which schedulers treat as clean abort, not failure.
- Every operation that can exceed ~100 ms (import, index build, proxy generation, export,
  cache warm, AE import) MUST be cancellable and MUST report progress through the standard
  progress channel so the UI can show it. No fire-and-forget long work.

## 2. Time discipline

- The four timebases in [01-GLOSSARY.md](01-GLOSSARY.md) §4 are **distinct newtypes** in
  `luminal-time`:

  ```rust
  pub struct RationalTime { num: i64, den: i32 }   // seconds as num/den, den > 0
  pub struct SourceTime(RationalTime);
  pub struct ClipTime(RationalTime);
  pub struct LayerTime(RationalTime);
  pub struct CompTime(RationalTime);
  pub struct FrameRate { num: u32, den: u32 }      // e.g. 30000/1001
  ```

- Authoritative time MUST NOT be `f32`/`f64`. Floats appear only at leaves: UI display,
  slider scratch values, and inside numeric kernels — always converted back through rational
  types before storage or comparison. Two frames that should be equal MUST compare equal;
  floats cannot promise that.
- Conversions between timebases exist only as named functions on the mapping objects that own
  them (a clip's Retime maps `ClipTime → SourceTime`; a layer's in point maps
  `CompTime → LayerTime`). Arithmetic mixing two timebases without an explicit conversion
  MUST NOT compile.
- Frame counts and seconds are different quantities: `FrameIndex(i64)` is not a time. Rounding
  time → frame happens in exactly one function per direction (`FrameRate::frame_at`,
  `FrameRate::time_of`), with documented rounding (floor to frame start), used everywhere.
- Frame rates are rational (`30000/1001`), never `29.97`. Sums of durations MUST be exact:
  rational arithmetic normalises and checks overflow (`i64` numerator overflows are a typed
  error, not a wrap).

## 3. Determinism

**Same project + same inputs = same pixels on export, on every machine, every run.**

- No wall-clock, no `SystemTime`, no `Instant`, no thread IDs, no iteration-order-sensitive
  hashing (`HashMap` iteration MUST NOT influence output; use ordered structures where order
  reaches pixels) anywhere in evaluation.
- All randomness in effects and expressions is seeded from
  `(node_uuid, property, local_time, user_seed)`. `wiggle`/`seedRandom` reproduce exactly
  across runs and machines (K-063). No `Date`, no IO, no locale access in the expression
  runtime.
- Scheduling MUST NOT change results: whichever thread, order, or tile split evaluates a
  node, the output hash is identical. Reductions with float accumulation MUST use a fixed
  association order (tree reduction), not "whatever order jobs finish".
- Adaptive degradation, proxies, and preview resolution affect **preview only**; export
  always evaluates at full quality (glossary §5). Any code path that could let a degradation
  flag leak into export is a release-blocking bug.
- GPU/CPU/CUDA implementations of one effect MAY differ within a documented tolerance
  (§6 golden tests); a single implementation MUST be bit-stable against itself.

## 4. Error policy

- **No panics in the engine.** Workspace lints deny `unwrap`, `expect`, `panic!`,
  `todo!`/`unimplemented!`, indexing that can panic in hot paths, and arithmetic that can
  overflow-panic, in all non-test code of engine crates (clippy: `unwrap_used`,
  `expect_used`, `panic`, `indexing_slicing`, `arithmetic_side_effects` — allow-listed per
  crate only with a comment). Tests and build scripts may panic freely.
- Every fallible boundary returns a **typed error** (`thiserror` enums per crate); errors
  carry enough context (asset UUID, node id, file path) to be actionable in the UI. No
  `Box<dyn Error>` across crate boundaries; no stringly-typed errors.
- **Degradation over failure**: when a resource limit is hit, the resource governor's ladder
  in [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) applies before any operation is
  refused, and refusal is a message, never an abort.
- **GPU device-lost is a recoverable event, not an error.** Code touching `luminal-gpu` MUST
  treat `DeviceLost` as a normal enum variant that triggers epoch recovery
  ([05-ARCHITECTURE.md](05-ARCHITECTURE.md) §5); it never propagates to the user as a crash
  or dialog on first occurrence.
- A failed node renders as an errored placeholder and the graph continues; one bad effect,
  expression, or plugin MUST NOT take down a frame, a comp, or the application.

## 5. Memory rules

- **All frame-sized allocations go through the pooled allocators** (texture pool in
  `luminal-gpu`, frame arena in `luminal-media`/CPU path), which account against the resource
  governor's RAM/VRAM budgets. `Vec::with_capacity(width * height * …)` outside the pools is
  a review reject.
- **No unbounded queues.** Every channel between threads is bounded; senders block, drop, or
  degrade per an explicit policy chosen at the call site (decode queues block — that is the
  back-pressure; mailbox channels overwrite — latest wins; progress channels drop
  intermediate updates). `unbounded()` in any crate requires a decision-log entry.
- Caches evict by governor policy only; nothing pins cache entries outside the documented
  pin set (playhead neighbourhood, current Viewer result).
- Long-lived collections keyed by UUID (undo journal, cache indices, thumbnails) MUST have a
  compaction or eviction story stated in a comment at the type definition.

## 6. Testing

- **CPU oracle per effect (K-019):** every WGSL effect ships a CPU reference implementation;
  CI renders both against a corpus of inputs and asserts agreement within the effect's
  declared tolerance (default: max component error ≤ 2/1024 in working space; effects
  needing looser bounds document why). The CPU path is also the runtime fallback, so the
  oracle is always shipping code, never a test-only sketch.
- **Golden-frame tests:** a corpus of small projects renders to reference EXRs; CI compares
  export output per platform. Golden updates are explicit, reviewed diffs (with visual
  side-by-sides in the PR), never regenerated silently.
- **Property tests** (proptest) for retime maths per [04-RETIMING.md](04-RETIMING.md):
  integrate(speed) ↔ differentiate(map) round-trips, monotone-segment invariants, overrun
  boundary behaviour (K-022: retime never moves edit points); for rational time (associativity,
  no drift over hour-long sums); for the command journal (apply → invert → apply = identity).
- **Fuzzing** (cargo-fuzz, in CI on a schedule): the `.lum` deserialiser and journal
  replayer (arbitrary bytes MUST produce a typed error, never a panic or hang) and the OFX
  host boundary (malformed plugin responses, wrong-size frames, dead processes).
- **Performance regression gates in CI**, on the reference machine (defined in
  [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)); regressions beyond 10% fail the build:
  - timeline interaction (drag a layer in a 200-layer comp): < 8 ms/frame UI cost;
  - scrub response (input → first draft frame presented, cached comp): < 50 ms;
  - snapshot publication after a keyframe edit: < 1 ms;
  - project open, 1000-asset synthetic project: < 2 s to interactive.
- Every bug fix lands with a test that fails before the fix. Deadlock-class bugs get a loom
  or stress test where feasible.

## 7. Code style and boundaries

- **Workspace lints** (`[workspace.lints]`): `rust_2024_idioms`, `clippy::all`,
  `clippy::pedantic` (curated allows), plus the §4 panic lints. Warnings are errors in CI.
- **Unsafe policy:** `unsafe` is permitted only in `luminal-gpu`, `luminal-media`,
  `luminal-expr` FFI edges, and the plugin hosts — each block wrapped in a safe API within
  its crate, carrying a `// SAFETY:` comment stating the invariant and who upholds it, and
  covered by a test (miri where the code is miri-able). `#![deny(unsafe_code)]` in every
  other crate.
- **FFI rules** (ffmpeg, OFX, CUDA, QuickJS): all pointers checked before deref; all C
  return codes converted to typed errors at the boundary; C-owned memory wrapped in RAII
  types with documented ownership; callbacks into Rust catch unwinds
  (`catch_unwind` → error code, never unwind across FFI); struct layouts pinned with
  `#[repr(C)]` and layout tests.
- **Public API docs:** every public item in engine crates has a doc comment; modules state
  their thread-role contract (§1.1) at the top. Doc examples compile (`cargo test --doc`).
- **User-facing strings** go through the i18n table from day one (K-005); en-GB, sentence
  case, calm, no exclamation marks. No string literal shown to a user lives in code.
- **Glossary compliance** extends to identifiers: `retime_map`, not `time_remap`; `speed`,
  not `velocity`; `clip`, not `event`; `playhead`, not `cti`; `export`, not `render` when a
  file is written. CI greps for the banned list in [01-GLOSSARY.md](01-GLOSSARY.md) §9
  across code, comments, and UI strings (allow-listed only in AE-import and
  other-app-comparison contexts).

## 8. Observability

- Structured logging via `tracing` throughout; engine crates emit spans, never `println!`.
  Log levels are meaningful: `error` is reserved for events a user would want reported,
  `warn` for degradation-ladder activations and recoveries, `info` for lifecycle, `debug`
  and below for everything else. Logging in per-pixel or per-sample paths is forbidden;
  per-frame paths log at `trace` behind a compile-time feature.
- Per-node GPU and CPU timings are collected in release builds (cheap counters, no
  allocation) — they feed the scheduler's adaptive concurrency and the pre-emptive tiling of
  nodes that trend towards the TDR window ([05-ARCHITECTURE.md](05-ARCHITECTURE.md) §5).
- Crash capture is out-of-process (Crashpad-style minidumps) and opt-in for telemetry;
  Luminal never phones home by default. DRED breadcrumbs are enabled in dev and beta builds
  only.
- Every degradation-ladder step and device-lost recovery emits a user-visible, calm status
  line (per [15-DESIGN.md](15-DESIGN.md) — no red-alert states); silent degradation is a
  bug because it makes performance reports undiagnosable.

## 9. Dependency hygiene

- New workspace dependencies require justification in the PR description: what it does, why
  not std/an existing dep, licence (GPLv3-compatible), maintenance signal. `cargo deny`
  runs in CI for licences, advisories, and duplicate versions.
- FFI-heavy and slow-to-compile crates (wgpu, rsmpeg, cudarc, QuickJS bindings) stay in
  their one owning leaf crate ([05-ARCHITECTURE.md](05-ARCHITECTURE.md) §1.1) so incremental
  builds of app-level crates stay in seconds.
- Pinned toolchain via `rust-toolchain.toml`; edition 2024; MSRV bumps are deliberate,
  logged in the changelog, never incidental.

## 10. Definition of done

A feature is done when all of the following hold; PRs state each explicitly:

1. **Spec reference** — the PR links the governing doc section (or adds one); behaviour not
   in a doc is not done, it is improvised.
2. **Tests** — unit tests; CPU-oracle + golden coverage if it touches pixels; property tests
   if it touches time or retiming; a fuzz corpus entry if it touches deserialisation or IPC.
3. **Cancellation and progress** — any new long operation checks epochs and reports progress
   (§1.4), demonstrated by a test that cancels it mid-flight.
4. **Budget compliance** — no CI performance gate regresses; new allocations go through
   pools; new channels are bounded.
5. **Error paths** — failure modes return typed errors and degrade per §4; no new panic
   sites; device-lost handled if GPU-adjacent.
6. **No new glossary violations** — identifiers, comments, UI strings, and docs pass the
   banned-term check; new concepts are named in [01-GLOSSARY.md](01-GLOSSARY.md) first.
7. **Determinism** — if it touches evaluation: no clocks, seeded randomness only, and the
   golden frames still match across two consecutive CI runs.
8. **Regression coverage (K-007)** — a bug fix MUST include a regression test that fails
   without the fix; the engine-crate line-coverage gate in CI MUST still pass, and its
   threshold may be raised but never lowered. The suite is the museum of every bug ever
   fixed; none may return unnoticed.
9. **Owner readability (K-007)** — if the change introduces a new concept, crate, or
   mechanism, [GUIDE.md](GUIDE.md) gains its plain-English section in the same commit, and
   any new impl note opens with an "in plain terms" framing.

---

## Open questions

- **Tolerance per effect class:** the default golden tolerance (≤ 2/1024) is a guess; flow
  interpolation and iterative effects will need per-effect bounds. Who owns the tolerance
  table — [08-EFFECTS.md](08-EFFECTS.md) per effect, or a test-owned manifest?
- **Reference hardware definition:** the CI performance gates need a pinned machine spec
  (and a macOS mirror?) in [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) before the
  numbers above are enforceable.
- **Loom coverage:** which hand-offs justify loom's state-space cost — snapshot publication,
  epoch cancellation, mailbox overwrite — and which settle for stress tests?
- **fp16 determinism across GPU vendors:** WGSL fp16 kernels may not be bit-identical
  between vendors. Is the export determinism promise per-machine bit-exact and cross-machine
  tolerance-exact, or do accumulation-sensitive nodes force fp32 on export?
- **Clippy pedantic drift:** the curated allow-list will grow; decide a cadence (per
  release?) for re-auditing allows so the lint wall stays meaningful.
