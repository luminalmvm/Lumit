# Engineering rules

Binding on every line of code. They exist because "responsive under any load" and "never
crashes" erode one careless commit at a time.

## 1. Concurrency

| Work | Where |
|---|---|
| Document edits, snapshot publication, painting | UI thread only |
| Pixel jobs | Worker pool only |
| Media decode | Decode threads only |
| Disk IO | IO threads only |
| wgpu submission | GPU-submit thread only |
| Audio graph | Audio-render thread only |
| cpal callback | Lock-free ring reads. No allocation, locks or logging |

- The UI thread never evaluates, decodes, runs an expression, does blocking IO or waits on
  a frame. It reads latest-wins mailboxes.
- Workers read the snapshot their job was made with, never the live document.
- Shared types are `Sync` because they're immutable, not because of locks. `unsafe impl
  Send/Sync` only in `lumit-gpu` and FFI crates, with a safety comment and a test.
- A new `Mutex` or `RwLock` on a hot path in `lumit-eval`, `lumit-core` or `lumit-cache`
  needs review: who holds it, how long, why not a channel.
- No lock held across an `.await`, a GPU submit or readback, a blocking send, an FFI call
  or plugin IPC.
- Every loop over frames, rows, tiles, clips or nodes checks its epoch and returns
  `Err(Cancelled)`. Anything that can take over about 100 ms is cancellable and reports
  progress.

## 2. Time

- Authoritative time is an exact rational. Never `f32` or `f64`. Floats only at the
  leaves (display, slider scratch, kernels) and converted back before storage or
  comparison.
- `SourceTime`, `ClipTime`, `LayerTime` and `CompTime` are distinct types. Mixing them
  doesn't compile. Conversions are named functions on the object that owns them.
- A frame index isn't a time. Rounding happens in `FrameRate::frame_at` and
  `FrameRate::time_of_frame` only.
- Frame rates are rational (`30000/1001`, never `29.97`). Overflow is a typed error.

## 3. Determinism

Same project and inputs, same exported pixels, every run.

- No wall clock, `Instant`, thread ids or `HashMap` order anywhere in evaluation.
- Randomness is seeded from node, property, time and user seed.
- Scheduling never changes results. Float reductions use a fixed order.
- Degradation, proxies and preview resolution are preview only. A path that leaks one
  into export is release-blocking.
- GPU and CPU versions of an effect may differ within its tolerance. One version is
  bit-stable against itself.
- Models aren't bit-stable, so they never run inside a render. They run as a baked
  analysis written to a sidecar, keyed by pack hash and provider.

## 4. Errors

- No panics in engine crates. The workspace denies `unwrap_used`, `expect_used`,
  `panic`, `todo` and `unimplemented`. Tests and build scripts may panic.
- Every fallible boundary returns a typed `thiserror` error with enough context to act on.
  No `Box<dyn Error>` across crates.
- Hitting a limit steps down the degradation ladder before anything is refused, and a
  refusal is a message, never an abort.
- GPU device loss is an enum variant that triggers recovery, not an error.
- A failed effect, expression or plugin draws as an error placeholder and the frame
  carries on.
- The bridge's `#[frb]` functions are outside clippy's reach, so a CI grep enforces this
  on `src/api/`.

## 5. Memory

- Frame-sized buffers come from the pools in `lumit-gpu` and `lumit-media`, which count
  against the governor. A frame-sized `Vec::with_capacity` elsewhere is a review reject.
- Every channel between threads is bounded. The sender blocks, drops or overwrites, by a
  policy chosen at the call site.
- Caches evict by governor policy only.
- A long-lived collection keyed by UUID says how it compacts or evicts, in a comment on
  the type.

## 6. Testing

- Every WGSL effect has a CPU twin, and tests hold the two within the effect's declared
  tolerance.
- Retime and rational time get property tests. So does the journal (apply, invert, apply).
- Every bug fix lands with a test that fails without it.
- Every performance budget is a CI gate.
- Not built yet: the golden EXR corpus and the fuzz targets.

## 7. Code

- Clippy runs with warnings as errors.
- `unsafe` only in `lumit-gpu`, `lumit-media`, the plugin hosts, their brokers and test
  plugins, the bridge, and `lumit-core`'s denormal guard. Each block is wrapped in a safe API, has a
  `// SAFETY:` comment and a test.
- FFI: check pointers, turn C return codes into typed errors at the edge, wrap C-owned
  memory in RAII types, `catch_unwind` in every callback, `#[repr(C)]` with layout tests.
- Public items in engine crates have doc comments. Modules say which thread they run on.
- No user-facing string literal in code. See the strings section of [GUIDE.md](GUIDE.md).
- The glossary binds identifiers: `retime_map` not `time_remap`, `speed` not
  `velocity`, `clip` not `event`, `playhead` not `cti`, `export` not `render` when a file
  is written.

## 8. Logging

- Engine crates never use `println!`. A closed console makes it panic. Until `tracing` is
  wired up, use each crate's `note!` macro. `no_panicking_prints.rs` enforces this.
- No logging in per-pixel or per-sample paths.
- Every degradation step and device reset shows a calm status line. Silent degradation is
  a bug.
- Lumit never phones home by default.

## 9. Dependencies

- A new dependency needs a reason in the pull request: what it does, why not std or an
  existing one, its licence (GPLv3-compatible), and whether it's maintained.
- `cargo deny` checks licences, advisories and sources against `deny.toml`.
- Heavy FFI crates stay in their one owning crate.
- `rust-toolchain.toml` pins the compiler. Raising it is deliberate.

## 10. Done means

1. Tests, including a CPU twin if it touches pixels and property tests if it touches time.
2. New long operations cancel and report progress, with a test that cancels one.
3. No performance gate regresses. New allocations use the pools, new channels are bounded.
4. Failures return typed errors. No new panic sites.
5. No glossary violations. New concepts are in the glossary first.
6. Evaluation stays deterministic.
7. The coverage gate still passes.
8. A new crate gets its line in [GUIDE.md](GUIDE.md).
