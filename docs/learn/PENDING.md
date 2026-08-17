# Pending: what the open pull requests change

**Delete this file when the three pull requests below merge.** It exists so the area
guides describe only current `main`. One file to delete beats a stale paragraph in
six.

Written 2026-08-13 against PR #92, #97 and #98.

## PR #98 — an effect is declared once (K-373)

Adds the machinery for single-declaration effects. It does not rewire the render
path yet.

| New | What it is |
|---|---|
| `crates/lumit-fx-macros/` | A proc-macro crate. `#[derive(Effect)]` turns one struct into the effect's schema and a typed parameter reader |
| `lumit-core/src/fx/registry.rs` | The `EffectDef` trait and `Catalogue` lookup |
| `lumit-core/src/fx/catalogue.rs` | Registration as a written list. List order is Add-effect menu order (K-137) |
| `lumit-core/src/fx/params.rs` | The parameter bag: a `(ParamId, Value)` arena replacing the closed `Resolved` enum |
| `lumit-core/src/fx/effects/` | Nine colour effects, each declared once in its own file |

Every numeric parameter gains a `Unit` (`Raw`, `PctDiag`, `Px`, `Degrees`,
`Seconds`). That turns `rescale_px` into one generic pass instead of a match that
knows which field is a length.

State after merge: 9 of 35 effects migrated. `builtins.rs` and `resolved.rs` stay
authoritative for the other 26 during the transition. Read
`docs/impl/effect-registry.md` before touching this area.

**Affects:** [01-CORE.md](01-CORE.md).

## PR #97 — the flare, the lights and the Viewer

Touches every layer of the stack. Records K-351 to K-372.

**Light layers.** `lumit-core` gains `LayerKind::Light { light: Box<LightDef> }` with
`LightKind` Point, Spot and Area, animatable colour and intensity, and
`accepts_lights: bool` on every layer. A new `lumit-core/src/lighting.rs` is the CPU
oracle. `fx_lighting.wgsl` is its GPU twin.

Lighting is **not** an effect. It has no schema and no stack entry. `lumit-render`
inserts it as pipeline step 4.5, after effects and before the transform. Light adds
rather than multiplies, so a comp with no lights renders byte-for-byte as before.

**The lens flare** is rewritten. The pipeline becomes: ray-trace compute → splat
build → additive hardware raster of one small quad per ray → Matte-mode source
detection → combine. FFTs never run on the GPU. The CPU bake arrives as textures
cached by parameter hash on its own bake thread (K-350). Bit-stability now comes from
barycentric-derivative coverage rather than 4× MSAA (K-353), which deletes a ~66 MB
multisample texture. Pipelines compile on a background thread, cutting worker start
from 7.7 s to 1.1 s.

Two effects join the catalogue: `light_wrap` and `sprite_flare`. Two shaders join
the GPU crate: `fx_light_wrap.wgsl` and `fx_sprite_flare.wgsl`.

**The Viewer** gains a region of interest, drawn by a new `ViewerRegionLayer` widget
and crossing the bridge as fractions of the picture. The transparency grid shows
through to nothing. Preview resolution becomes per-comp. Auto becomes a real quality
tier. The splash shows an "opening project" state, because `open_project` is now
async.

**At the bridge**, still frames and playback get separate quality decisions
(`still_quality` / `playback_quality`, K-372). Scrubbing after playback reused the
leftover adaptive tier and missed the cache. Layers gain `acceptsLights`. Parameter
groups gain the `visible_when_lens_elements` rule.

Dev builds now compile the four maths-heavy crates at `opt-level = 3`. Without it the
flare bake ran about 16× slower.

One known red test rides along:
`wgsl_lens_flare_padded_anamorphic_matches_and_fills_the_edge` sits at 0.9876 against
a 1% bar. It is recorded in `docs/TODO.md` and is not a new failure.

**Affects:** [01-CORE.md](01-CORE.md), [02-PIXELS.md](02-PIXELS.md),
[03-GPU.md](03-GPU.md), [05-BRIDGE.md](05-BRIDGE.md),
[06-FRONTEND.md](06-FRONTEND.md).

## PR #92 — Crowdin sync

The routine translation round-trip. It updates five machine-owned locale files:
`app_de.arb`, `app_kk.arb`, `app_uk.arb`, `app_zh.arb`, `app_zh_Hant.arb`. It changes
no English source and no code.

PR #97's roughly 30 new English keys upload after #97 merges, not in this one.

**Affects:** [07-BUILD-SHIP.md](07-BUILD-SHIP.md).

## After merging

1. Delete this file.
2. Remove its row from [README.md](README.md).
3. Update the affected guides above where the code no longer matches them.
