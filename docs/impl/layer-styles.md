# Layer styles

Decision: K-706. Status: §10's first package has landed — the model, all nine
declarations, the render seam and two rendering styles. Packages 2 and 3 are
still to build.

**In plain terms.** Photoshop lets you hang a wardrobe on a layer — a shadow
behind it, a glow around it, a colour or gradient painted across its face, a
stroke around its edge — without adding a single effect. After Effects carries
the same wardrobe over as **layer styles**: a fixed little family that lives on
the layer itself, below Transform, and dresses whatever the layer's alpha says
the layer *is*. They are not effects in a stack you reorder; they are nine named
slots in an order Photoshop fixed twenty years ago, and everyone's muscle memory
expects that order. This note pins how Lumit models them, where they render,
which existing kernels do the work, how the Timeline and panel show them, and
what an `.aep` import keeps.

The trick that keeps this small: a style *is* an effect instance wearing a
uniform. Lumit already has typed, keyframeable, schema-carrying effect structs
(`#[derive(Effect)]`), a resolve walk that turns them into GPU ops, a panel that
draws their rows, and an importer that maps AE properties onto them. Styles
reuse every one of those mechanisms; what is new is a second, order-locked list
on the layer and a handful of small kernels.

## 1. The model

```rust
// lumit-core/src/model.rs
pub struct Layer {
    ...
    /// The layer's style stack (docs/impl/layer-styles.md). Same shape as
    /// `effects` — each entry an EffectInstance whose match name is one of the
    /// nine style definitions — but order-locked to §2's table and capped at
    /// one instance per style. Empty = no styles, byte-identical rendering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub styles: Vec<EffectInstance>,
}
```

- **Reuse `EffectInstance`, not a new struct family.** Every style property is
  then a `Property` (keyframeable, expression-drivable, undo-covered) for free,
  and serialisation is the shape every tool already reads. Forward compatibility
  is the effect system's own: an unknown match name in a newer file loads,
  reports, and renders as identity (K-258's degrade rule).
- **Each style is a `#[derive(Effect)]` struct** in a new
  `lumit-core/src/fx/styles/` module, registered in its own `STYLE_DEFS` table
  beside `BUILTIN_DEFS` — *not* in the effect catalogue, so the Add-effect
  search never offers "Drop shadow (style)" beside the Drop shadow effect.
  Match names are prefixed: `style_drop_shadow`, `style_inner_shadow`,
  `style_outer_glow`, `style_inner_glow`, `style_bevel_emboss`, `style_satin`,
  `style_colour_overlay`, `style_gradient_overlay`, `style_stroke`.
- **Invariants** (enforced by the edit commands, restored on load): at most one
  instance of each style per layer; the list is always sorted by §2's order.
  Adding a style inserts at its fixed position; there is no reorder command.
- **Blend mode per style**: AE's `mode2` maps onto the Mix row's Blend choice
  that every Lumit effect instance already carries (`fx_blend_mix.wgsl`, same
  mode table as the compositor). No new field.
- **Global light is baked, not modelled.** AE's `useGlobalAngle` is a comp-wide
  shared angle; v1 has no comp-level light. Import bakes the resolved angle into
  the style's own angle and reports the link (§7). If a global light ever lands,
  it is a new comp field plus a per-style toggle — additive, no migration.
- **Cache key**: wherever the frame key hashes `layer.effects`, it hashes
  `layer.styles` immediately after — styles are picture, not metadata.

### The nine structs, v1 parameter sets

Angles below follow the catalogue convention drop_shadow.rs pinned (measured
from straight up, clockwise); import converts (§7). All distances/sizes are
px@comp (`unit = Px`), opacities per cent.

| Style | Parameters (beyond the shared Mix + blend mode) |
|---|---|
| Drop shadow | colour, opacity, direction (dial), distance, softness, spread %, layer knocks out shadow (toggle) |
| Inner shadow | colour, opacity, direction, distance, softness, choke % |
| Outer glow | colour, opacity, softness, spread % |
| Inner glow | colour, opacity, softness, choke %, source (Edge/Centre choice) |
| Colour overlay | colour, opacity |
| Gradient overlay | colour A, colour B, opacity, angle, type (Linear/Radial), scale %, reverse (toggle) |
| Stroke | colour, opacity, size, position (Outside/Inside/Centre choice) |
| Satin | *(modelled, reported, not rendered in v1 — §8)* colour, opacity, direction, distance, softness, invert |
| Bevel and emboss | *(modelled, reported, not rendered in v1 — §8)* style, technique, depth %, direction (up/down), size, softness, angle, altitude, highlight colour/opacity, shadow colour/opacity |

Satin and Bevel get structs (so import preserves their data losslessly and the
file format never migrates) but no kernel and no panel "add" entry in v1.

AE's per-style `noise` sliders and Outer/Inner glow's `glowTechnique`,
`gradient` ramp and `inputRange` are not modelled: the DOM refuses the ramps
outright (the golden bundle's report already counts them), and noise is a
Photoshop dither nobody keyframes. Import reports non-default values (§7).

Gradient overlay is two colour stops in v1, matching the Gradient effect's own
two-stop model (`gradient.rs`) — a ramp editor is that effect's upgrade to
inherit, not ours.

**Where the two overlays' Opacity lives.** Colour overlay and Gradient overlay
have no Opacity row of their own: their **Mix row is labelled "Opacity" and is
it**. That is not a saving, it is the only place the number can sit and mean what
Photoshop means. The K-425 seam applies Mix *after* the style's Blend mode —
"blend the overlay in, then take this much of the result" — whereas a separate
opacity inside the kernel would fade the overlay *before* it was blended, which
is a different picture on every mode but Normal. The styles that draw new pixels
rather than recolouring existing ones — the shadows, the glows, the stroke —
keep their own Opacity, because there it says how dark the shadow is and not how
much of the style you take. Import maps AE's `opacity` onto whichever of the two
the style carries.

## 2. The order — pinned

Photoshop composites styles in a fixed order and AE inherits it (AE's style
rendering is Photoshop's, per Adobe's own layer-styles documentation; the panel
lists them in this order top-to-bottom and paints bottom-of-list furthest
back). Painting order, first = furthest behind:

1. **Drop shadow** — behind everything
2. **Outer glow** — behind the layer, in front of its shadow
3. *(the layer's own pixels, post-effect-stack)*
4. **Gradient overlay** — interior, clipped to alpha
5. **Colour overlay** — interior, covers the gradient overlay
6. **Satin** — interior
7. **Inner glow** — interior
8. **Inner shadow** — interior
9. **Stroke** — straddles the edge, over the interiors
10. **Bevel and emboss** — topmost (its highlights sit on everything, stroke included)

This is the stored order of `Layer::styles`, the order the panel and the
Timeline list them in, and the order the pixels end up in. Interior styles (4–8)
read and write only where the layer's alpha is; 1–2 add premultiplied pixels
underneath; 9–10 may touch both sides of the edge.

**It is not, quite, the order the ops run** — the one place the implementation
corrected the design. The ops run one after another on a single raster, and an
interior style floods whatever alpha it finds: run the Drop shadow first and the
Colour overlay above it would paint the shadow too. So the seam emits the
interiors first, on the layer's own alpha, and then the outer styles — which
composite *underneath* — in reverse, so that the last one run ends up furthest
back and the Drop shadow is where §2 puts it. Because the stored list is sorted,
the outers are exactly its leading run and the split costs a `take_while`. The
picture is §2's; only the arithmetic order differs.

## 3. The render seam

**Where:** in `build_comp_draws` (lumit-render/src/build.rs), the layer's
styles resolve exactly as its effects do — a second `resolve_stack` walk over
`layer.styles` at the same layer time — and the resulting ops are **appended
after the effect stack's ops** in `CompLayerDraw::fx` (`fx_ids` appended 1:1
alongside, so the profiler lands style milliseconds on the style's row). From
there nothing downstream changes: `run_ops` runs them on the layer's own linear
raster after masks and the effect stack, the compositor then transforms, matte-
gates and blends the styled raster as one picture. Styles therefore render
**after the layer's effect stack and before the transform photograph, the track
matte, and the blend into the comp** — one seam, no new pass.

The parallel per-op lists stay 1:1 **by construction, and with no change at
all**: `run_ops` advances each of those counters on the *op's own schema* — a
matte slot for an op whose schema names a matte parameter, a polyline per
declared mask-path row, a schedule for an op with a points port — and a style
declares none of them. So the `mattes` / `dof_inputs` / `mask_paths` /
`points_schedules` builders go on walking `layer.effects` alone, the style ops
consume no slot, and because styles are appended *after* the effect stack no
earlier index moves. (The K-395 injected Matte row is suppressed on style
schemas: a style dresses the layer's own alpha; gating a shadow by another layer
is an effect's job.) The rule that makes this safe is a test rather than a
convention — `no_style_declares_a_row_the_render_would_have_to_fill`, in
`lumit-core/src/fx/styles/tests.rs`, fails the build for the first style that
grows one of those rows.

**The deviation, stated honestly:** AE renders layer styles *after*
transformations — rotate an AE layer and its style shadow keeps its screen
direction; scale it and the shadow distance does not scale. Lumit v1 runs
styles pre-transform on the layer raster, so they inherit the layer's
transform, exactly as Lumit's Drop shadow *effect* (and AE's) already does.
This is the whole cost of getting styles for one appended resolve walk instead
of a new comp-space compositor pass, and it is invisible on the unrotated,
unscaled layers styles overwhelmingly sit on (text, shapes, lower-thirds).
<!-- ponytail: pre-transform styles; ceiling = a rotated/scaled layer's shadow
turns/scales with it where AE's stays screen-fixed. Upgrade: composite the
styled layer alone onto a comp-sized intermediate (the seeded-intermediate
machinery adjustment layers use in realise.rs) and run style ops there. Trigger:
an import report row or user report where the difference is visible. -->
The importer writes a report row when a styled layer is rotated or has
non-uniform scale (§7), so the difference is never silent.

**Alpha input:** every style reads the working raster's own alpha at its point
in the appended chain — which is the post-effect-stack alpha, matching AE
(effects run before styles there too). Drop shadow the *effect* already proves
this works: same raster, same premultiplied maths (drop_shadow.rs's
`premultiplied = true` reasoning carries over verbatim).

**ROI/padding:** outer styles reach `distance + softness` (or `size`) beyond
every edge, with no honest static bound — `roi = FullFrame`, the same
declaration and padding path the Drop shadow effect uses today.

**Adjustment layers:** allowed, nothing special — the styles see the
composite's alpha like any other op in the adjustment's stack. Usually
full-frame opaque, so outer styles add nothing; that is AE's behaviour too.

## 4. Which kernels do the work

| Style | Core | New code |
|---|---|---|
| Drop shadow | `cpu::drop_shadow` + `fx_dropshadow.wgsl`, whole | spread: threshold-remap of the blurred alpha inside the same kernel; knockout: one branch |
| Outer glow | the drop-shadow kernel **with zero offset** (blur alpha, tint, composite under). Not the Glow effect — that is a bright-pass bloom on colour, a different machine | spread, as above |
| Inner shadow | drop-shadow kernel on **inverted** alpha, result clipped inside the shape and composited over | the invert + clip wrapper |
| Inner glow | inner shadow with zero offset; Centre source inverts the distance sense | shared wrapper |
| Colour overlay | `fx_fill.wgsl` (`fill.rs`) — flat colour preserving alpha | mode/opacity via the existing Mix blend stage |
| Gradient overlay | `fx_gradient.wgsl` (`gradient.rs`) two-stop ramp | clip to alpha (one multiply) |
| Stroke | none reusable — this is *alpha-contour* stroke, not `stroke.rs` (that is AE's paint-a-path Stroke effect) | one new separable dilate/erode (running max/min over ±size, two passes), edge band = dilated − eroded per position, tinted |
| Satin, Bevel | — | none in v1 |

<!-- ponytail: separable two-pass max/min dilate gives slightly square corners
where Photoshop's stroke is round; ceiling visible on strokes > ~20 px around
sharp corners. Upgrade: two-pass chamfer distance transform in the same kernel
slots. Trigger: side-by-side import comparison flagging corner shape. -->

Four of the seven shipped styles are one gaussian-on-alpha kernel in four
configurations — the drop-shadow core generalises with three uniforms (invert
alpha in/out, offset, spread) rather than four shaders.

## 5. The Timeline fold

`layer_fold_frb.dart` already builds the fold as one typed row list (Transform
always; Effects, Audio, Retime when present). **Styles** joins as a sibling
group directly after Effects (AE lists Layer Styles beside Effects too, and
after-Effects is also their render position): a `FoldGroupRow` labelled
"Styles", present only when `layer.styles` is non-empty, one subgroup per
style in §2's order, each
opening to its parameter rows. Because styles are `EffectInstance`s carrying
ordinary schemas, the rows are `FoldEffectParamRow`s unchanged — stopwatches,
lane diamonds, value wells, drivers, expressions all arrive with them. The only
new row type is none at all.

Bridge: the effect-stack accessors gain a styles twin (same
`BridgeEffectInstanceInfo` shape, separate list), and the param-edit commands
route through one shared "find instance on layer" lookup that searches
`effects` then `styles` — one helper, so every existing param command (set,
keyframe, driver, enable) works on style rows without a second code path.
Add/remove/toggle style are three small new commands (fixed-position insert,
per §1's invariants).

## 6. The panel rows

The Effect controls panel shows a **Styles** section under the effect stack
(AE shows styles in the Timeline only; giving them panel rows is one reused
widget and spares the Timeline round-trip). Section header, then per-style
blocks rendered by the existing effect parameter row widgets — again no new
row widgets. "Add style" lives in the Layer menu (`Layer > Layer styles >` —
the menu slot already exists as a "(Not implemented)" row) and on the section
header: the seven shipped styles, each greyed once present.

Strings: nine style labels plus the handful of new parameter labels are engine
labels → `engine_labels.dart` + `app_en.arb` entries in the same commit
(`engine_labels_test.dart` gates this).

## 7. AE import

The parser already captures everything (`aep/props.rs` reads the
`ADBE Layer Styles` group generically — match names, values, keyframes, enabled
flags, and the derived group switch the DOM lies about; the golden capture
holds full property sets for all nine styles). What is missing is only the
**map** stage: `docs/11` currently says "placeholder".

Mapping, per style, reusing the effects table machinery (`ae-effect-map.toml`
entries with conversion fns, targeting `style_*` match names into
`layer.styles` instead of the effect stack):

- **Opacity** `0..255 → %` — the Drop shadow effect's rebase, already written.
- **Angle**: AE/Photoshop measures the *light* counter-clockwise from +x;
  Lumit's style direction is from-up clockwise, and the shadow slides opposite
  the light: `θ_lumit = 270° − a_ae (mod 360)`. (Check: AE's 120° default →
  150°, down-and-right — correct.) The same formula, minus the opposition,
  maps overlay/satin angles.
- `blur` → softness, `distance` → distance, `chokeMatte` → spread/choke %,
  `size` → stroke size, `layerConceals` → knockout, `mode2` → the Mix blend
  mode, `frameFX/style` → stroke position, `innerGlowSource` → source,
  `gradientFill/type|scale|reverse|angle` → their twins.
- `useGlobalAngle = true`: bake the comp's global-light angle (the DOM hands
  the resolved `localLightingAngle` regardless), report row naming the link.
- **Reported, value preserved in the raw capture**: Satin and Bevel groups
  (imported into their modelled structs, flagged "not rendered in this
  version"), per-style `noise` ≠ 0, glow `glowTechnique`/`inputRange`, the
  gradient ramps the DOM refuses (already report rows today), any `mode2` on an
  *outer* style other than Normal/Multiply where the raster seam cannot honour
  it, and a styled layer whose transform makes §3's pre-transform deviation
  visible (rotation ≠ 0 or non-uniform scale).
- The Blending Options subgroup (`ADBE Blend Options Group`) imports as report
  rows only in v1.

`docs/11-AE-IMPORT.md`'s Layer styles row moves from "placeholder" to "mapped"
with the boundary list, in the import package's commit.

## 8. v1 ships vs reports

**Ships rendering:** Drop shadow, Inner shadow, Outer glow, Inner glow, Colour
overlay, Gradient overlay (two-stop), Stroke.

**Modelled + imported + reported, not rendered:** Satin (offset-alpha
intersection shading — a fiddly kernel for a style almost nobody uses) and
Bevel and emboss (a lighting model with five techniques and an altitude — the
one genuinely expensive style). Their structs exist so no file migrates when
their kernels land; the panel does not offer them; an import that carries one
says so in the report. Shipping seven well beats shipping nine where two are
wrong.

## 9. Test plan

Engine (lumit-core / lumit-render / lumit-gpu):
- **Identity**: empty `styles` renders byte-identical to before the field
  existed (K-258 regression, the field's serde default included).
- **Order**: two styles enabled compose in §2's order — colour overlay over
  gradient overlay pinned by pixel test; drop shadow under outer glow.
- **One instance per style / fixed position**: the add command's invariants,
  plus load-time restore of a hand-shuffled file.
- **Kernel cores**: outer glow == drop shadow at distance 0 (bit test); inner
  shadow clips to alpha (no pixels outside the shape); stroke Outside adds no
  pixels inside, Inside none outside; spread at 100 % is a hard-edged shadow.
- **CPU/GPU agreement** for each shipped style within the battery's existing
  tolerance harness; determinism (same doc, same frame, same bytes).
- **Cache key**: editing a style property changes the frame key; toggling the
  last style off restores the unstyled key.
- Padding: a shadow's reach beyond the layer rect survives at reduced preview
  resolution (the px@comp resolve path).

Import (lumit-import):
- Golden bundle: the 22 styled layers map — count of mapped styles, angle
  formula spot-checks (120° → 150°), opacity rebase, report rows for Satin/
  Bevel/noise/ramps exactly as §7 lists (the existing refused-property counts
  stay green).

UI (only the touched test files):
- Fold: a styled layer shows the Styles group in order; an unstyled one shows
  none; param row edit round-trips through the shared instance lookup.
- `engine_labels_test.dart` covers the new labels by construction.
- Redraw budget: the Styles rows follow WP-2's listenable patterns; the
  bridge-calls-in-build test stays at 0.

## 10. Packages, in order

1. ~~**Engine model + two anchor styles.**~~ **Landed.** `Layer::styles`, the
   `STYLE_DEFS` module with all nine structs (two rendered: Drop shadow and
   Colour overlay — one outer, one interior, proving both seam halves), the
   build.rs appended resolve walk (`resolve_layer_fx`), cache-key inclusion, the
   generalised drop-shadow kernel uniforms (`spread_scale`, `knockout`), and the
   identity/order/cache/parity tests in `fx/styles/tests.rs` and
   `lumit-render/tests/layer_styles.rs`.
2. **The remaining five kernels + order pinning.** Inner shadow, Outer glow,
   Inner glow, Gradient overlay, Stroke (the new dilate), the full §2 order
   pixel tests, CPU/GPU agreement, padding tests.
3. **Fold + panel + menu + import.** Bridge styles accessors and the shared
   instance lookup, add/remove/toggle commands, Timeline Styles group, panel
   section, Layer-menu wiring, engine labels + arb keys, the §7 map stage,
   docs/11 row update, golden-bundle assertions.

## Open questions

- Does a comp-level global light (and per-style "use global light" toggles)
  ever earn its place, or is the baked import angle the permanent answer?
- Whether Satin and Bevel land as package 4 or wait for demand — their structs
  and import rows make either a non-breaking choice.
- The post-transform (comp-space) style pass, if the §3 deviation ever bites a
  real project — the seeded-intermediate route is named in the ponytail note.
