# Temporal re-render effects: accumulation motion blur + Posterize Time

Feeds docs/08-EFFECTS.md (two Temporal effects) and docs/06-RENDER-PIPELINE.md. These are
NOT per-pixel effects — they change *what time the layers below them render at*, so they live
at the frame-orchestration layer, not in `run_ops`. Owner spec: [[accumulation-mb-design]],
[[posterize-time-design]].

## In plain terms
Per-layer motion blur (K-120) smears one layer along its own transform. **Accumulation motion
blur** is the expensive, correct one: it renders the *whole scene below it* several times at
in-between moments and averages the finished frames — so footage motion, animated effects,
depth passes and everything else are all correct per sample (no blurred-depth artefact).
**Posterize Time** renders the scene below at a *held* time on a coarse grid, for the choppy
stop-motion look. Both re-run the render at a changed time; that is the whole trick.

## 1. Why it cannot be a `run_ops` effect
`fxops::run_ops` receives finished textures (`&[Resolved]` + the composite-below as one
texture). To render the below-stack at a *different* time you must rebuild its draw list
(`draws::build_comp_draws(doc, comp, τ, …)`) and `realise` it — and only the frame
orchestration (which holds `doc`/`comp` and the decoded `pixels_by_layer`) can do that.
`realise` only has textures. So these effects are detected and executed **where
`build_comp_draws` + `realise` are called** (preview: the comp-render entry in `gpu.rs`/
`app_state`; export: `render_comp_linear`), not in the compositor's per-op loop.

Both are **adjustment-layer** effects (docs/08 §1.5): they process everything below. "Apply
to all layers" (the owner's global toggle) is simply putting the effect on a top adjustment
layer spanning the comp — no separate global flag needed.

## 2. The shared capability: render the below-stack at time τ
Factor one helper both effects and both paths (preview + export) use:
`render_below_at(doc, comp, below_layers, τ, …) -> Texture` = `build_comp_draws` at `τ` (reuse
the *same* decoded `pixels_by_layer` — footage frames are held; only transforms/effects/camera
re-resolve) → `realise`. Sub-frame footage *motion* is out of scope (that is the flow
`motion_blur` effect); this blurs comp-driven animation. Preview and export MUST call the one
helper so a preview frame equals an export frame (K-031), exactly as `render_layer_input` and
`motion_blur_average` are shared.

## 3. Accumulation MB
- Params: **Samples** N, **Shutter angle**, **Shutter phase** (reuse `MotionBlur::sample_offsets`
  maths for the centred offsets), Mix. Category Temporal, cost Heavy (≈ N× a full comp render).
- Render below at `τ_k = t + off_k·dt` for each k; **average** the N textures with a
  pure-additive-at-`1/N` pass (colour AND alpha additive, so a static scene is unchanged).
  Composite the average in place of the plain below-composite.

**Landed (K-134).** `lumit_core::fx::stack_accumulation_mb` resolves the effect to an
`AccumulationMbParams { samples, shutter_angle, shutter_phase, mix }`, whose `sample_offsets()`
reuses `MotionBlur::sample_offsets` (empty for N < 2 — no blur). It is an **adjustment** effect,
detected exactly as Posterize is: `accumulation_mb_below(...)` builds one `below_draws_at(τ_k)`
per sample into an `AccumulationBelow { samples, mix }` carried on the adjustment draw. The
combine is a **new** GPU pass — `Compositor::accumulate(&[(&Texture, weight)])` over the
premultiplied-passthrough fragment `fs_accumulate` (the inputs are already-premultiplied comp
composites, so unlike per-layer `motion_blur_average`'s `fs_layer` it must NOT re-premultiply).
Preview (`Realiser::accumulate_below`) and export both render the N sub-frames through the one
`render_below_at`, average at `1/N`, then blend the average against the frame-time below by
`mix` (a second `accumulate` of two weighted layers `1 − mix` and `mix` — a linear interpolation
the additive pass gives exactly). Still-scene bit-identity holds because `1/N` is exact in fp16
for a power-of-two N and the N copies sum back exactly; a moving scene smears. **On real
hardware**, which is the qualification the identity needs: Linux CI runs the pixel tests
against Mesa's lavapipe, a CPU rasteriser, and there the sum-and-divide path lands up to one
8-bit step from the single composite — an implementation may round fp16 intermediates
differently from a GPU, and nothing in the spec forbids it. The test keeps the exact
assertion on hardware adapters and checks "within one step" on software ones
(`GpuContext::software`); a genuinely broken accumulation — wrong weights, dropped samples —
fails both. `sample_temporally`
(K-132) is honoured through the shared `below_draws_at`/`build_comp_draws_at` threading. It takes
precedence over Posterize when an adjustment somehow carries both (one temporal re-render per
adjustment in v1).

## 4. Posterize Time
- Params: **Input frame rate** (e.g. 12), optional **Phase**. No Scope parameter (K-166): the
  reach is implied by the carrier layer's kind — an **adjustment layer** holds everything below
  (its effect input); any **other layer** holds its own source and stack. Category Temporal,
  cost cheap (one render at the held time — often the SAME held time across many frames, so the
  frame cache key collapses them: a big win).
- Held time `τ = floor((t − phase)·rate)/rate + phase`.
- **Adjustment carrier**: `render_below_at(…, τ)` — the below re-render path.
- **Any other carrier**: no re-render of others; the layer the effect sits on evaluates its
  own effect stack at `τ` instead of `t` (a per-layer time substitution feeding its own stack),
  its transform staying live. Simpler; no orchestration re-entry. **Landed (K-133, reach rule
  K-166):** `this_layer_effect_time(effects, fx_on, lt, start_offset)` returns the held layer
  time (the grid computed on comp time `lt + start_offset`, mapped back) whenever a live
  Posterize is present and `lt` unchanged otherwise; both `build_comp_draws_at` (preview) and
  export's `apply_fx` feed it to `resolve_stack_temporal` as the sample time (so
  `sample_temporally == false` still holds at the live `lt`), so the two are identical (K-031).
  The kind split lives at the orchestration sites: `posterize_below`/`posterize_sample_times`
  treat only `LayerKind::Adjustment` carriers as below-holds.
- **Footage decode snap (FX-1).** The held re-render alone quantises *comp-driven* animation
  (transforms/effects/camera); a scene that is only footage playing back would not visibly step,
  because the decode planner still chose the frame-time source frame. `posterize_sample_times(
  layers, t) -> Vec<f64>` closes that: it walks the stack top-to-bottom composing each live
  Posterize's grid onto a running sample time (an adjustment carrier holds every layer
  beneath it; any other carrier holds only its own layer's source sampling — K-166), and both the
  preview decode planner (`collect_comp_jobs`) and export (`prepare` on the main pass,
  `collect_below_pixels` for the held re-render) read it to snap *which source frame each covered
  layer decodes* to `τ`. So the held re-render's footage frames now match the held grid, footage
  playback steps, and preview equals export (K-031). Only the primary source frame is snapped —
  temporal effects' neighbour frames still degrade to stills — and footage is held everywhere
  below the adjustment (a masked reveal shows held footage outside the mask too, a documented
  boundary; the full-frame adjustment is the intended global pass).

## 5. Per-effect "don't sample" flag (owner request)
`EffectInstance` gains `#[serde(default = "default_true")] sample_temporally: bool`. During a
sub-frame/held render, an effect with it **false** is evaluated at the *frame* time `t`, not
`τ_k` — for particle systems and other stochastic/costly effects the user does not want re-run
per sample. Held effects render once at `t`; only sampled ones move.

**Landed (K-132).** The split lives in `lumit_core::fx::resolve_stack_temporal(effects,
sample_lt, frame_lt, …)`: it shares `resolve_one` with `resolve_stack`, handing each effect
`frame_lt` when its flag is false and `sample_lt` otherwise — so `sample_lt == frame_lt` is
byte-identical to `resolve_stack` and the ordinary render is untouched. `build_comp_draws`
becomes a thin wrapper over `build_comp_draws_at(doc, comp, t_comp, frame_t, …)`, which threads
the true playhead `frame_t` (and `frame_lt = frame_t − start_offset`) through nested Precomps
and into `posterize_below`/`below_draws_at`/`render_below_at`; every layer's own stack resolves
through `resolve_stack_temporal`. The after-effects matte/depth sources keep their own K-125
temporal boundary (they already hold their temporal inputs to stills), so the flag is honoured
on each below-layer's *own* stack, at every depth.

## 6. Cache key + determinism (lumit-eval)
The sampled/held times are a pure function of `t`, the rate/shutter, and `sample_temporally`
flags, so `feed_*` just reflects them: accumulation MB feeds N + the shutter (only when the
effect is present and enabled); Posterize Time feeds the *held* time (so identical held frames
share a key — the deduplication that makes it cheap). No new non-determinism.

## 7. Build order (incremental, each green)
1. `render_below_at` shared helper (preview + export) — the risky orchestration re-entry;
   prove preview == export on a still scene first (a re-render at the same `t` must be
   bit-identical to no re-render).
2. Posterize Time on an adjustment carrier (one render at `τ`) — the simplest consumer.
3. `EffectInstance.sample_temporally` (landed, K-132) + Posterize Time per-layer reach
   (landed, K-133; carrier-kind rule K-166).
4. Accumulation MB (N samples + the additive average) on top of (1) (landed, K-134).
Each step is a K-decision + docs/08 section + oracle/parity test where one applies (these have
no per-pixel oracle; the test is a still-scene identity + a moving-scene coverage check, as
per-layer MB used).

## Traps
- Re-rendering re-decodes nothing — pass the SAME `pixels_by_layer`; re-resolving at `τ` must
  not re-request media (that would thrash the decode planner).
- Recursion: an adjustment layer's below-set excludes itself; nested comps already cycle-guard
  via `visited`.
- Cost: N× render is Heavy — respect docs/13 budgets; degrade N under the preview-draft/scrub
  path (fewer samples while interacting), full N on export.
- Preview == export is the whole risk surface — one shared `render_below_at`, reviewed by hand.
