# Implementation notes

These notes exist for one reason: some parts of Kiriko are genuinely hard or fiddly at a
level the specs deliberately do not descend to, and the implementing model (or human)
should not have to re-derive them. Each note pins down **exactly how** to build one hard
thing: algorithm choice with rationale, data layouts, reference code sketches, the traps,
and how to test it. They were written by the model that designed the system (Fable), for
the models that will build it.

Rules of engagement:

- The **specs in `docs/` remain canonical** for *what* to build; these notes are the
  authoritative *how* for their topics. If a note and a spec conflict, the spec wins and
  the note is a bug — fix it in the same change.
- Code blocks are reference sketches: correct in structure, intent, and the tricky maths,
  but not compiled. Treat variable names and crate APIs as advisory; treat the algorithms,
  invariants, formulas, and traps as binding.
- Every note ends with a **test plan**. Implement the tests with the feature — they encode
  the correctness arguments.
- If you are implementing something hard that has no note and you find yourself making a
  research-level choice, stop and record the choice in the relevant spec's open questions
  (or a new note) rather than burying it in code.

| Note | Covers | Feeds |
|---|---|---|
| [rational-time.md](rational-time.md) | Overflow-safe rational time arithmetic, canonical form, hashing, f64 conversion, grid rounding | everything |
| [keyframe-eval.md](keyframe-eval.md) | AE bezier keyframe evaluation, cubic solving, Retime segment evaluation and inversion | 03, 04 |
| [gpu-foundation.md](gpu-foundation.md) | wgpu device/texture pool/bind groups, fp16 pipeline, device-lost recovery, egui viewport, colour blit | 05, 06 |
| [media-io.md](media-io.md) | ffmpeg via rsmpeg, frame index, exact seeking, D3D11VA/VideoToolbox hardware decode → wgpu, NV12 WGSL, audio decode | 05 |
| [playback-scheduler.md](playback-scheduler.md) | Epoch cancellation, job pool, bounded pipelines, cpal audio clock, ring buffer, preview modes | 05, 06 |
| [optical-flow.md](optical-flow.md) | The flow engine: DIS optical flow in WGSL, occlusion, frame synthesis, flow motion blur | 04, 08 |
| [ofx-host.md](ofx-host.md) | Hosting OpenFX from Rust: suites, property sets, action dispatch, out-of-process transport | 12 |
| [beat-detection.md](beat-detection.md) | Spectral-flux onset detection, thresholding, BPM grid | 09 |
| [expressions.md](expressions.md) | Embedding QuickJS-ng deterministically via rquickjs | 12 |
| [phase-0-kickoff.md](phase-0-kickoff.md) | The cold-start build order: workspace scaffold and six runnable slices to Gate 0 | 16 |
