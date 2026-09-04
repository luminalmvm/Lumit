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
| [effect-registry.md](effect-registry.md) | How an effect is declared, registered, resolved and dispatched: the derive macro, the parameter bag, dynamic and spare parameters | 08, 05, 06 |
| [lens-flare.md](lens-flare.md) | The Lens flare effect: lens prescriptions, ghost ray tracing with coating interference, FRFT/FFT bakes, hardware-raster ghosts, the staged oracle | 08 |
| [ae-import.md](ae-import.md) | The AE importer: the Bridge walker, the capture schema, ExtendScript traps, the fixture, and the mapping rules | 11, 10, 05 |
| [tracking.md](tracking.md) | Camera and object tracking: affine KLT tracks, global SfM solve, dynamic-track rejection, zoom-cut handling | 08, 16, 05 |
| [node-graph.md](node-graph.md) | The layer's driver graph: model, ops, evaluation, port types, the points stream, and phase 3's ordered work packages | 03, 05, 15 |
| [particulate.md](particulate.md) | The Particulate design (commissioned by the points-stream decision): closed-form points evaluation, the parameter surface, the finalised `PointsStream` layout, budgets, and the simulated exception's contract | 08, 06, 13 |
| [points-stream.md](points-stream.md) | The points-stream infrastructure: the `EffectData` wire and its rules, the Points sample driver, the evaluation and carriage contract, the seam, and the ordered work packages PS1–PS7 | 08, 06, 05, 13 |
| [ocio.md](ocio.md) | OCIO colour management: the native transform engine, config parsing and resolution, the deterministic bake, where each edge runs, the seam and UI surfaces, the golden conformance suite, and the ordered work packages — design only until built | 06, 07, 03, 10 |
| [custom-shader.md](custom-shader.md) | The Custom shader effect: the binding contract, the uniform-to-parameter grammar, naga validation and the refusal taxonomy, source-hash pipeline caching, the inner node graph that compiles to WGSL, and the work packages CS1–CS5 — design only until built | 08, 12, 06 |
| [audio-plugins.md](audio-plugins.md) | Audio plugin hosting: CLAP first then VST3, the broker isolation reused from OFX, the insert-ahead-of-Volume mix seam, the block/determinism contract, parameter and state mapping, and the ordered work packages AP1–AP5 — design only until built | 12, 09, 05 |
| [puppet.md](puppet.md) | The Puppet tools: the alpha-conforming triangle mesh (marching squares → Douglas–Peucker → constrained Delaunay via `spade`), the Igarashi two-step as-rigid-as-possible solve with starch/overlap/bend, the CPU warp at the paint/masks seam, storage and caching, and the ordered work packages PU1–PU3 — design only until built | 07, 03, 06, 13 |
| [roto.md](roto.md) | Roto brush and Refine edge: stroke-seeded geodesic segmentation, flow-propagated frame to frame with corrections layered on, the guided-filter edge band, the `roto/` sidecar and its one invalidation rule, and the ordered work packages RB1–RB3 — design only until built | 07, 08, 10, 16 |
| [layer-styles.md](layer-styles.md) | Layer styles: the nine AE styles as a second, order-locked `EffectInstance` list on the layer, Photoshop's pinned compositing order, the appended-ops render seam and its recorded pre-transform deviation, the reusable drop-shadow/fill/gradient kernel cores, the Styles fold and panel rows, the AE import mapping, and the ordered work packages — design only until built | 08, 06, 03, 11 |
| [group-effects.md](group-effects.md) | Effects on a layer group header (PROPOSED): the header's `EffectInstance` stack on `LayerGroup`, the implicit per-frame precompose through the existing `DrawSource::Nested` path, the Photoshop isolation rule for member blends, the frame-key feed, the amended grouped-equals-ungrouped promise, and the ordered work packages GE1–GE3 — design only until built | 03, 06, 07 |
| [ui-performance.md](ui-performance.md) | The 60/120 interface mandate: the gesture table measured in the owner's own conditions (maximised window, live preview) and the four-times delta against the small-window test trap, where each millisecond sits (select, scroll, zoom, scrub, the rendering backend), the paint-layer and listenable-selection architecture, the parked probe, and the gated work packages | 13, 07 |
