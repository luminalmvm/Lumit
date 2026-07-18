# Layer-input effect parameters + completing DoF — impl note

Feeds docs/08-EFFECTS.md §3.9 (DoF) and docs/03-DATA-MODEL.md (a new effect
parameter type). The GPU kernel already exists (`lumit_gpu::fx::dof` +
`upload_depth_map` + `fx_dof.wgsl`, oracle green); this note is the *how* for the
two things left: an effect parameter that references **another layer** as an
input, and wiring the DoF effect on top of it.

## In plain terms
Some effects need a second picture, not a number — depth-of-field needs a **depth
map** saying how far each pixel is. The cleanest source is *another layer* in the
comp (a depth pass). Lumit already does exactly this for **track mattes**: a layer
names another layer, and the compositor renders that other layer alone and hands
its texture in. A layer-input parameter is the same idea, but the referenced
layer's texture is handed to an **effect** instead of the matte stage.

## 1. The parameter (mirror `MatteRef`)
- `ParamKind::Layer { }` (fx.rs) — declares the effect wants a reference to
  another layer (a depth/aux input).
- `EffectValue::Layer(Option<Uuid>)` (model.rs) — the referenced layer's id, or
  None when unset. Exactly the shape of `MatteRef.layer`, minus channel/invert.
- Inspector: a **Layer picker** arm — a dropdown of the comp's layers by name
  (plus "None"), like the Parent picker in the Effect Controls panel (K-103).
  Selecting sets the id through an undoable op.

## 2. Threading the referenced layer's texture (mirror mattes + the LUT §8)
`run_ops` takes only `&[Resolved]` (Copy scalars), so — exactly as the LUT
texture and the flow field are threaded — the referenced layer's **rendered
texture** travels beside the ops:
- The referenced layer is rendered **alone at comp size** (linear fp16), the same
  render the matte stage already produces (`MatteInput.texture` is "the matte
  layer rendered alone at comp size" — reuse that path / helper).
- `run_ops` gains a parallel input `layer_inputs: &[Option<Tex>]`, one slot per
  effect op that declares a Layer parameter; the k-th such op binds the k-th slot
  (the LUT counter pattern). A `None` slot (unset / missing / cyclic reference) is
  a passthrough — never a fault.
- Preview (`build_comp_draws` → `gpu.rs`) and **export** must render the
  referenced layer and thread it **identically** (K-031) — factor "render layer
  X alone at comp size" into one shared helper both call, as the matte path does.
- Guard cycles: an effect on layer A referencing layer B that (transitively)
  references A must not infinitely recurse — a visited-set like the cache key's,
  or simply "a layer-input renders its target with that target's own layer-input
  effects disabled" for v1. Flag the choice.

## 3. The DoF effect (docs/08 §3.9) on top
- Schema `dof` (Blur & sharpen or a new Camera category — check §3.9): params a
  `Layer` param `depth` + Float `focus` (0–1), `range` (0–1), `aperture` (px@comp)
  and Mix. Traits: cost Moderate, roi Padded(aperture), temporal `{0}`,
  premultiplied true (the gather is over premultiplied colour — confirm against
  `fx_dof.wgsl`).
- `Resolved::Dof { focus, range, aperture, mix }` (scalars only — the depth
  texture is threaded per §2, not carried in Resolved).
- resolve arm reads the floats; the depth layer id is read separately by the
  caller (like the LUT path) to render + thread the depth texture.
- `run_ops` Dof arm: if its `layer_inputs` slot is `Some(depth)`, call
  `fx.dof(ctx, &tex, w, h, depth, focus, range, aperture, mix)`; else passthrough
  (no depth = no blur, a labelled no-op).
- `cpu::apply` Dof arm = passthrough (GPU-only, like the LUT); the §1.6 oracle
  reference is the existing `wgsl_dof_matches_the_cpu_oracle` in lumit-gpu (its
  `dof_reference`), not `cpu::apply` — the depth is a texture, not a number.
- Depth encoding: the depth layer's pixels are read as a single channel (its
  luminance or R) mapped to 0..1 by `upload_depth_map`; document that a brighter
  pixel = nearer/farther (pick one, note it). A pre-rendered comp-size texture is
  already fp16 working format, so extract depth as luma in the DoF kernel's read,
  or upload a converted R32Float — reuse `upload_depth_map`'s contract.

## 4. Cache key (lumit-eval)
The referenced layer's *content* feeds the effect, so the consumer frame key must
change when the depth layer changes: in `feed_layer`, when an effect has a
`Layer` param, recurse `feed_layer` on the referenced layer (guarded by the same
visited set) so its evaluated transform/effects/source join the key. v1 minimum:
at least hash the referenced layer id; full recursive content-hashing is the
correct form — do it if the visited-set makes it clean.

## 5. Test plan
- The existing `wgsl_dof_matches_the_cpu_oracle` covers the kernel (done).
- A resolve test: a `dof` instance resolves its floats and its `Layer` param
  round-trips a layer id (serde).
- A no-op test: `dof` with an unset depth layer is a passthrough.
- Preview==export: the referenced-layer render + threading go through one shared
  helper (asserted by construction / reviewed by hand, as for the LUT).

## Reference the layer BEFORE or AFTER its own effects (K-125, matte landed)
Both a **track matte** and a **layer-input** (depth) offer a boolean — take the
referenced layer's pixels **as source** (before its own effect stack, the
default and historical behaviour) or **after** the fully processed layer. A
keyed greenscreen matte, or a matte whose edge you softened with a blur, is the
motivating case; a graded depth pass is the layer-input case.

**Matte — landed (K-125).** `MatteRef` gains `after_effects: bool`,
`#[serde(default)]` false (source-only, unchanged for old projects). When set,
the matte source's stack runs on its texture before it gates the consumer:
`shell::gpu` uploads the source pixels, linearises, and — when the carried
`MatteDraw::fx` is non-empty — calls the same `fxops::run_ops` a layer's own
draw uses, *then* composites the matte alone. Export does the identical thing
via `apply_fx` on the freshly `prepare`d source. Preview and export both resolve
the source's stack the same way, so they match (K-031). The frame key folds the
source's stack (shared `feed_effect_stack`) only when the toggle is on.

- This also **corrected a latent K-031 bug**: export was reading the matte
  source's *post-fx* `prepared` texture while preview read source-only, so a
  matte source that happened to have effects diverged. Both are now source-only
  by default; post-fx only when `after_effects` is set.
- **v1 boundary:** temporal inputs — echo neighbours, the flow motion-blur
  field, a nested depth reference — are **not** fed through an after-effects
  matte (empty `neighbours`/`flow`/`layer_inputs` to `run_ops`). An echo or flow
  effect on the matte source therefore degrades to a still. The common cases
  (colour key, blur, levels, curves) are exact. Lifting this needs the matte
  source to flow through the full per-layer draw path (its own decode job for
  neighbours/flow, its own depth inputs) rather than a one-shot `run_ops`.

**Depth layer-input — landed (K-125).** Rides as a companion
`depth_after_effects` `Bool` schema param on the DoF effect (default false), not
an `EffectValue::Layer` model change — cheaper, no serialisation churn, and
per-input nameable by the `<layer-param-id>_after_effects` convention. When set,
`render_dof_inputs` (preview) / `build_dof_inputs` (export) run the depth
layer's own stack on its texture before `render_layer_input` resamples it —
same `run_ops`-before-consume shape as the matte, same v1 temporal boundary
(empty neighbours/flow/nested-depth). The frame key folds the depth layer's
stack through `feed_effect_stack`'s Layer arm when the sibling flag is set,
guarded by `allow_after_effects_refs` so the fold is one level deep (a depth
layer's *own* layer-inputs render as passthrough in v1, so they are not folded
after-effects — matching the render and bounding key recursion). New DoF
instances show the "Depth after effects" checkbox automatically; existing saved
instances gain it on re-add (the instance-driven param list, as for any new
effect param).

## Status / follow-ups (landed, K-123/K-124)

**What shipped, and the choices §2's "render alone" pinned in practice.** The
effect stack runs on the *consuming layer's own working raster* `(w, h)` (the
decoded size, which shrinks under reduced-resolution preview), and the DoF
kernel reads the depth at that same pixel grid — so the depth input must be
exactly `(w, h)` and aligned with the layer texture, **not** a comp-sized
render (a comp-sized depth would misalign under reduced preview and for
non-full-frame layers). v1 therefore renders the referenced layer's **source**
(effects not applied) and **resamples it to fill `(w, h)`** through the one
shared helper `fxops::render_layer_input`, which preview and export both call
(K-031). Consequences, all documented in docs/08 §3.22:
- **Cycle guard = source-only.** Because the depth render never re-enters an
  effect stack, a layer-input can never recurse — the strongest form of "render
  the target with its own layer-input effects disabled".
- **Framing.** The depth pass is expected to share the footage's framing (it is
  stretched to the working raster; the depth layer's own transform is not
  applied). A placement-aware / effects-aware depth is a follow-up.
- **Visibility gate.** Preview only decodes visible in-span layers (plus matte
  sources), so both preview and export gate the depth reference on *visible +
  in-span*; a hidden reference is a passthrough in both, never a disagreement.
  Extending `app_state::collect_comp_jobs` to decode a hidden depth reference
  (as it already does a matte source) is the recorded follow-up that lifts this.
- **Cache key.** `feed_layer` hashes the referenced layer's source + transform
  (the matte block's shape — matching the source-only render), guarded by the
  precomp visited set.

This unblocks **DoF v1** (a depth layer + focus/range/aperture/mix). Remaining:
the inspector **Layer picker** and the set-param op (the owner's follow-up — an
unpicked Layer renders as nothing for now); the preview decode planner gate
above; a placement/effects-aware depth; and the fuller "DOF PRO" second effect
with shaped bokeh highlights and the deferred bright-rim "Highlight bloom" param.
Logged as K-123 (Layer-input parameter kind) and K-124 (DoF effect).

## DoF lens controls — landed (K-128)

Three additions to the DoF effect that read the same threaded depth pass, none
touching the layer-input plumbing above (they are plain scalar params on the
effect instance, carried in `Resolved::Dof`, which stays `Copy`):

- **Depth invert** (`depth_invert` bool). Applied to the depth *after* it is
  read (`d' = 1 - d`) and *before* the circle-of-confusion, the near/far select
  and the Depth-map view — so it swaps near and far consistently everywhere. It
  acts on the resampled depth the kernel reads, so it is orthogonal to
  `depth_after_effects` (which changes *what* the depth pass is): a graded depth
  can still be inverted. Continuous, so the §1.6 ULP oracle holds.
- **Near/Far blur** (`near_aperture`/`far_aperture`, px@comp). Per-side maximum
  CoC; the pre-existing `aperture` becomes a master scaling both about its
  default 8. Absent on pre-feature projects, so the resolve arm falls them back
  to `aperture` (`float_at(..).unwrap_or(8)` under the master, which reproduces
  the old single-aperture radius). The near/far select flips only where the
  smoothstep `s` is zero (`d = focus`), so the radius stays continuous.
- **Display** (`display` choice: Rendered / Depth map / Focus map). Diagnostic
  views computed from the same depth read: Depth map writes the post-invert `d`
  as greyscale, Focus map writes `1 - s`; both short-circuit before the disc
  gather and ignore Mix. All shipped modes are continuous, so the oracle covers
  them with no exclusion.

No cache-key change: the new Bool/Float/Choice params hash automatically through
`feed_effect_stack`'s per-param arms (they sit on the effect instance).
