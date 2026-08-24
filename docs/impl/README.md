# Implementation notes

These notes exist for one reason: some parts of Lumit are genuinely hard or fiddly at a
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
| [lut.md](lut.md) | `.cube` LUT parsing, shared trilinear maths, 3D-texture upload with manual shader interpolation, caching by path | 08, 03 |
| [layer-input.md](layer-input.md) | Effect parameters that reference another layer (mirroring track mattes), threading the rendered texture into `run_ops`, and completing the DoF effect on top | 08, 03 |
| [temporal-rerender.md](temporal-rerender.md) | Accumulation motion blur + Posterize Time: re-rendering the below-stack at sub-frame/held times at the frame-orchestration layer, the per-effect don't-sample flag | 08, 06 |
| [gpu-foundation.md](gpu-foundation.md) | wgpu device/texture pool/bind groups, fp16 pipeline, device-lost recovery, colour blit | 05, 06 |
| [media-io.md](media-io.md) | ffmpeg via rsmpeg, frame index, exact seeking, D3D11VA/VideoToolbox hardware decode → wgpu, NV12 WGSL, audio decode | 05 |
| [playback-scheduler.md](playback-scheduler.md) | Epoch cancellation, job pool, bounded pipelines, cpal audio clock, ring buffer, preview modes | 05, 06 |
| [optical-flow.md](optical-flow.md) | The flow engine: DIS optical flow in WGSL, occlusion, frame synthesis, flow motion blur | 04, 08 |
| [ofx-host.md](ofx-host.md) | Hosting OpenFX from Rust: suites, property sets, action dispatch, out-of-process transport | 12 |
| [beat-detection.md](beat-detection.md) | Spectral-flux onset detection, thresholding, BPM grid | 09 |
| [expressions.md](expressions.md) | Expressions on Rhai: the engine pool, what an expression can see, and how far determinism goes | 12 |
| [phase-0-kickoff.md](phase-0-kickoff.md) | The cold-start build order: workspace scaffold and six runnable slices to Gate 0 | 16 |
| [paint.md](paint.md) | Paint strokes: the gesture-not-pixels model, the dab-along-a-polyline rasteriser, the clone-source trap, where paint sits in the render | 03, 06, 07 |
| [anti-aliasing.md](anti-aliasing.md) | Multisampling the composite: why MSAA over supersampling, the four traps in the composite loop, adapter capability checks, the project property | 06, 03 |
| [shape-layers.md](shape-layers.md) | The plan for `LayerKind::Shape`: model, renderer, bridge and tools — a plan, not a spec | 03, 06, 07 |
| [ae-effect-parity.md](ae-effect-parity.md) | The AE default-effect gap audit and the order the missing ones get built (tiers, wave 1 batches) | 08, 11 |
| [effect-registry.md](effect-registry.md) | How an effect is declared, registered, resolved and dispatched: the derive macro, the parameter bag, dynamic and spare parameters (K-381) | 08, 05, 06 |
| [lens-flare.md](lens-flare.md) | The Lens flare effect: lens prescriptions, ghost ray tracing with coating interference, FRFT/FFT bakes, hardware-raster ghosts, the staged oracle | 08 |
| [ae-import.md](ae-import.md) | The AE importer: the Bridge walker, the capture schema, ExtendScript traps, the fixture, and the mapping rules (K-410) | 11, 10, 05 |
| [tracking.md](tracking.md) | Camera and object tracking: affine KLT tracks, global SfM solve, dynamic-track rejection, zoom-cut handling (K-415) | 08, 16, 05 |
| [node-graph.md](node-graph.md) | The layer's driver graph (K-471): model, ops, evaluation, port types, the points stream, and phase 3's ordered work packages | 03, 05, 15 |
| [particulate.md](particulate.md) | The Particulate design (K-446): closed-form points evaluation, the parameter surface, the finalised `PointsStream` layout, budgets, and the simulated exception's contract — design only until built | 08, 06, 13 |
