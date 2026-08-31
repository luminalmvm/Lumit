# Effects on a layer group

Status: **PROPOSED** (owner feature ask, 2026-08-31; decision entry text at the
end of this note — the build package appends it to docs/02-DECISIONS.md and
numbers it). Amends one sentence of K-702's promise; reverses nothing else.

**In plain terms.** A layer group today is a label: a named band over a run of
layers with a triangle on it, and the render walk never reads it — grouped or
ungrouped, the picture is identical (K-702). The owner's ask is to let the band
*do* one thing: drop effects on the group's header and have them act like an
adjustment layer for the members only. A blur on the header blurs the lower
third; the background plates behind it stay sharp. An adjustment layer cannot
say that — it processes *everything* below it — and Precompose can, but at the
cost of packing the layers into another comp you now have to open to edit.

The way to get there is not a new kind of adjustment. It is to notice that
"apply effects to these layers composited together, and to nothing else"
already has a name in this engine: it is what a **Precomp layer** does every
frame — realise a list of draws into one comp-sized texture, run an effect
stack on that texture, composite the result as one picture
(`DrawSource::Nested` in lumit-render/src/draw.rs, realised in
realise.rs's `realise_segment`). So an effected group renders as an **implicit,
per-frame precompose**: the member run's draws are built exactly as today, then
wrapped in one Nested draw whose `fx` is the header's stack. When the header
carries no effects, nothing is wrapped and the walk stays group-blind — which
is how K-702's promise survives, reworded: *the picture is identical grouped or
ungrouped **when the header carries no effects***.

Photoshop is the precedent, not After Effects. AE has nothing here — its users
precompose and put an adjustment layer inside, which is this same machine
operated by hand. Photoshop groups blend as **Pass Through** by default (each
member blends against the full backdrop); give the group its own effects or a
blend mode other than Pass Through and the group **isolates** — members
composite against each other first, then the finished slab meets the backdrop.
That isolation is not a bug to engineer around; it is what "apply an effect to
the members composited together" *means*. You cannot blur a slab that does not
exist yet, and the slab only exists once the members have stopped blending
into the backdrop individually. Lumit adopts the same rule, stated in §3.

## 1. The model — the stack lives on the group

```rust
// lumit-core/src/group.rs
pub struct LayerGroup {
    pub id: Uuid,
    pub name: String,
    pub label: u8,
    pub members: Vec<Uuid>,
    /// The header's effect stack (docs/impl/group-effects.md). Same shape as
    /// Layer::effects. Empty = the K-702 group: invisible to the render walk,
    /// byte-identical picture, and every project saved before this field
    /// existed re-saves byte-identical (skipped while empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectInstance>,
}
```

- **Reuse `EffectInstance`, not a hidden carrier layer.** The alternative — a
  concealed adjustment-flavoured layer inserted above the run — was considered
  and declined: it would leak into everything that counts layers (stack order,
  ids, selection, the read model, `drawn_members` itself, AE export one day)
  and each leak needs a "pretend it isn't there" patch. A field on the group
  touches only the paths that choose to read it. `EffectInstance` brings
  keyframeable, expression-drivable, schema-carrying parameters, serde
  forward-compatibility (K-258's degrade rule for unknown match names), and
  the whole existing effect catalogue for free.
- **Resolve time is comp time.** A group has no in point, out point, start
  offset or Retime, so there is no layer clock to convert through: the header's
  stack resolves at `t_comp`, through the same
  `lumit_core::fx::resolve_stack_temporal_named` every layer stack uses
  (it takes a bare `&[EffectInstance]` — no `Layer` required). Marker context
  is the comp's markers; a group has none of its own.
- **No node graph on the group in v1.** Wires live on `layer.graph`
  (`LayerGraph::validate` checks against `layer.effects`); a group carries
  none, so group effect parameters take keyframes and expressions but not
  node-graph drivers. <!-- ponytail: no LayerGraph on LayerGroup; ceiling = a
  wire cannot drive a group effect's parameter. Upgrade: a graph field on the
  group validated against its effects, same pruning rule as
  Op::SetLayerEffects. Trigger: the first owner ask to wire one. -->
- **No fx master switch on the group.** Per-instance `enabled` is the whole
  bypass surface; a group-level switch would be a fifth header switch for a
  list that is usually one entry long.

## 2. The render seam — one wrapped draw

In `build_comp_draws` (lumit-render/src/build.rs), before the layer loop, work
out which groups are **live**: header stack non-empty with at least one enabled
instance, and `group::drawn_members` answering a non-empty run. For each live
group, note the run's index span in `comp.layers`. During the walk, draws for
layers inside a span collect into a side list instead of `draws`; when the span
closes, push **one** draw:

```text
CompLayerDraw {
    layer: group.id,                    // the profiler's row for the group
    source: DrawSource::Nested {
        width:  comp.width,             // comp-sized: members land where
        height: comp.height,            //   they always landed
        background: transparent,
        draws:  the collected member draws, unchanged,
        camera: the comp's own active camera at t   // 3D members keep their pose
        key:    None,                   // uncached in v1 — see §4
        paint:  empty, paint_time: t,
    },
    natural_size: comp size, identity placement (comp centre, scale 100,
    rotation 0), opacity 100, blend Normal, three_d false,
    fx / fx_ids:  the header stack, resolved at t_comp, comp diagonal,
                  px_scale 1.0,
    fx_ref_width: Some(comp.width),     // K-266 rescale under reduced preview,
                                        // exactly the adjustment arm's setting
    mattes / dof_inputs / lut_files / mask_paths / points_schedules /
    flare_lens_files: built from group.effects by the same helpers the layer
                  path uses (mattes_for, dof_inputs_for, lut_files, ...),
    everything else empty/None.
}
```

Nothing downstream is new. `realise_segment` already realises a Nested draw
entire, runs its `fx` on the finished texture (K-266/K-268 rescale included)
and composites it as one picture — the group unit rides the Precomp layer's
code path from the first line to the last. There is no new `DrawSource`, no
new staging in `realise_at_depth`, and no change to `region_is_safe` (a Nested
draw already renders itself entire and takes the window at the composite).

The walk stays group-blind on every other frame: a group whose header stack is
empty, wholly bypassed, or whose drawn run is empty builds exactly the draws
it builds today. The wrap is the *only* reading of `Composition.groups` the
render ever makes, and it is gated on "live".

**An empty run runs nothing.** If every member is gated out (eyes off, solo
elsewhere, all deleted), the group contributes no draw at all — a flare on the
header of an empty group draws no flare. Cheap, predictable, and consistent
with "the effects act on the members": no members, no act.

## 3. What holds, what changes — the honest list

- **K-702's promise, reworded.** "Identical picture grouped or ungrouped"
  becomes "…when the header carries no effects". The build package edits
  group.rs's module comment, docs/03-DATA-MODEL.md §5.4 and docs/07-UI-SPEC.md
  §4.2a ("MUST never change the picture" gains the same clause) in the same
  commit, and the decision entry below records the amendment against K-702.
- **Member blend modes against the backdrop: isolated, by design** (§0's
  Photoshop rule). Inside the unit, members blend against transparency and
  each other; the finished slab meets the layers below as one Normal-blended
  picture. A Multiply member stops multiplying the plate *below the group* the
  moment the header carries a live effect, and starts again the moment it does
  not. This is pinned v1 semantics, not a ceiling with an upgrade: blending the
  members into the backdrop first and *then* applying the effect to "just
  their contribution" has no meaning for any mode but Normal, which is why
  Photoshop refuses the combination too.
- **Mattes cross the boundary intact, both directions — for free.** A Lumit
  matte is never a read of the composite: `MatteDraw` carries the matte
  source's own pixels (or its nested comp's draws) and realise renders it
  *alone* into comp space (`matte_texture` in realise.rs). A member matted by
  a layer outside the group, and an outside layer matted by a member, both
  keep working because each MatteDraw is self-contained inside whichever draw
  list it sits in. The same argument covers layer inputs (depth passes, Light
  wrap backgrounds) and K-710 propagated mattes.
- **Solo, shy, lock, the header's broadcast switches: unchanged.** They gate
  *which member draws exist* (build-side), and the wrap collects whatever
  exists. Shy is outline-only; lock touches no pixel; the K-702 broadcast ops
  stay per-member.
- **An adjustment layer inside the run changes scope — deliberately.** Today
  (group-blind walk) an adjustment inside a group processes everything below
  it, group boundary or not. Inside a live group's unit, `realise_at_depth`
  recurses and the Adjust draw stages *within the unit*: it processes the
  members below it and nothing outside. That is the precomp semantic, it is
  what "the group composites as a unit" means, and it only ever differs on the
  frames where the header is live. Named in the tests so it is a choice, not a
  surprise.
- **3D members keep their pose but the unit composites flat.** The unit's
  Nested draw carries the comp's camera, so members with the 3D switch land
  exactly where they did; what is lost is depth *interleaving* with 3D layers
  outside the group — the slab is one 2D picture in the stack, as a Precomp
  layer is.
- **The header stack cannot see time sideways in v1.** Posterize Time and
  accumulation motion blur resolve to no op and are only wired on the
  adjustment path (docs/08 §3.25–3.26, "adjustment-only capability"), so on a
  header they are inert. Fast motion blur and Datamosh bind no field and pass
  through (K-544's documented degrade), because nothing builds `flow_below`
  for the unit. <!-- ponytail: temporal/flow effects on a header are
  inert/passthrough; ceiling = a Fast motion blur on a group does nothing.
  Upgrade: build the member run's draws at each neighbour time into the
  unit's flow_below — realise_segment already measures a Nested draw's own
  motion that way (K-565's Precomp arm) — and thread temporal_below the same
  way. Trigger: the first project that puts one there. -->

## 4. The frame key and caching

The key must move when the picture can. In `feed_comp` (lumit-eval/src/lib.rs),
after the layer walk, for each **live** group (same definition as §2):

- feed a `b"group-fx/"` tag, the drawn-run member ids **in stack order** (a
  member drifting out of the run changes which pixels the effect reaches, so
  it must retire cached frames), and the header stack exactly the way a
  layer's effects feed — resolved values at `t_comp`, live instances only.
- A group that is not live feeds **nothing**, so every key ever made — and
  K-702's "grouping changes no key" tests — hold bit-for-bit.

Caching of the unit itself: `key: None` in v1 — the unit re-realises every
frame the comp renders, exactly the K-421 stance the adjustment path takes for
the composite below ("unnamed, so uncached"). Unlike that composite, the unit
*could* be named — its content is the member run, which is a subset of the
facts the frame key already hashes — so the upgrade is real and cheap.
<!-- ponytail: unit uncached (key None); ceiling = the member run re-composites
per frame while the header is live, the K-421 doubling. Upgrade: a sub-key
hashed from the run's members alone (the §4 group-fx feed minus the header
stack), handed to realise_nested the way K-422 names a Precomp's frame.
Trigger: a docs/13 trace where the unit's re-composite is the missing budget. -->

## 5. Undo, ungroup, pre-compose

- **One new op**: `Op::SetGroupEffects { comp, group, effects }` — the
  whole-list shape of `Op::SetLayerEffects`, coarse and exactly invertible
  (the inverse carries the previous list). No graph pruning twin, because §1
  gives the group no graph. Add, remove, reorder, toggle and every param edit
  commit through it or through the existing property ops.
- **The lock has nothing to say** about `SetGroupEffects`, matching K-702's
  stance on the other four group ops: a group is not lockable, and its members'
  locks guard the members.
- **Ungroup discards the header stack** — the band is gone, so is its wardrobe
  — and undo restores it, because `UngroupLayers`' inverse already carries the
  whole group and the group now carries its effects. No new machinery.
- **Pre-compose group moves the stack onto the Precomp layer.** The semantics
  are literally identical (that is §2's whole argument), so the heavy fold
  inherits the light one's effects without changing the picture — the one-click
  path K-702 built now carries the wardrobe across. One extra step in the
  existing precompose command.

## 6. The surface

- **Bridge**: `BridgeLayerGroup` gains the same effects listing the layer
  crossing has (the `BridgeEffectInstanceInfo` shape), plus
  `get_group_effects` / the `SetGroupEffects` command on
  `CompositionReference`. The one shared "find instance on layer" lookup that
  K-706 built (effects, then styles) grows a third place to look — the comp's
  groups — so **every existing param command** (set, keyframe, expression,
  enable, remove, bypass) works on a group instance with no second code path,
  exactly as styles did.
- **Effect controls panel**: clicking a group's header name makes the group
  the panel's subject, the way clicking a layer does; the panel draws the
  stack with the effect parameter row widgets it already has (stopwatches,
  scrub, expressions included) and its Add-effect search targets the group.
  This panel is the whole editing surface in v1.
- **Timeline**: the header row (drawn inside its carrier's block, K-702) gains
  only an **fx tick** beside the member count when the stack is non-empty, so
  an effected group is visible in the outline. No fold rows under the header
  in v1: K-702's row list is one entry per visible layer, and growing
  header-owned lanes bends that shape — the panel carries the parameters
  until it cannot. <!-- ponytail: no Timeline lanes for group effect
  keyframes; ceiling = diamonds on group params are edited in the panel and
  the graph editor only. Upgrade: a FoldGroupRow hung off the carrier layer's
  fold, the way Styles joined the layer fold. Trigger: the owner asking where
  the diamonds are. -->
- **Strings**: the fx tick's tooltip and the panel's group-subject header are
  the only new user-facing strings — `app_en.arb` entries in the same commit,
  named in it and in the PR for the translation page (K-005, K-303). Effect
  names and parameter labels already exist.
- **AE import**: nothing. AE has no group, so no group ever arrives with
  effects on it.

## 7. Test plan

Engine (lumit-core, lumit-eval, lumit-render):

1. **Identity** — a group whose header stack is empty (and one wholly
   bypassed) builds byte-identical draws and an identical frame key to the
   same comp ungrouped; K-702's existing regression stays green; a pre-groups
   project re-saves byte-identical (serde skip).
2. **Scoping** — three layers, blur on the middle group of one: the member
   blurs, the layer below and the layer above do not (pixel test on all
   three bands).
3. **Isolation pinned** — a Multiply member over a plate below the group:
   grouped-with-live-header differs from ungrouped exactly as one Normal slab
   predicts (the §3 semantic, asserted so it is chosen, not drifted into).
4. **Mattes cross** — a member matted by an outside layer still gates; an
   outside layer matted by a member still gates; both with the header live.
5. **Gating** — all members' eyes off (and: a solo elsewhere in the comp)
   ⇒ the unit contributes nothing; a generator on the header draws nothing.
6. **Adjustment inside** — an adjustment layer between two members processes
   the member below it and not the plate below the group, while the header is
   live.
7. **Frame key** — adding a header effect changes the key; a param edit
   changes it; bypassing the last instance restores the no-effects key; a
   member dragged out of the run changes it; dragging back restores it.
8. **Undo shapes** — SetGroupEffects round-trips; Ungroup discards and undo
   restores the stack; Pre-compose group lands the stack on the new Precomp
   layer and the picture does not change across the conversion.
9. **K-266 under preview** — a px@comp radius on the header lands at the same
   comp-relative size at half preview resolution (the `fx_ref_width` path).
10. **Determinism and K-031** — same doc, same frame, same bytes; export and
    preview agree on an effected group (both build through the same walk).

UI (only the touched test files, per standing policy):

11. Panel shows the group's stack when its header is picked; a param edit
    round-trips through the shared instance lookup; the fx tick appears
    exactly when the stack is non-empty; the redraw-budget test stays at 0
    bridge calls in build.

## 8. Packages

- **GE1 — the engine whole**: `LayerGroup::effects`, `Op::SetGroupEffects`,
  the §2 wrap in `build_comp_draws`, the §4 key feed, the K-702 rewording in
  group.rs/docs 03/07, the decision entry appended and numbered, GUIDE.md's
  plain-English section, tests 1–10.
- **GE2 — the surface**: bridge crossing and the third arm of the shared
  instance lookup, panel subject + Add-effect targeting, the header's fx
  tick, the two arb keys (named in commit and PR), test 11.
- **GE3 — the inheritances**: Ungroup/Pre-compose carrying (§5), their tests
  (test 8), and the docs/06 §1.5 cross-reference note that a group unit is
  the Nested path, not a third staging.

## Open questions

- **Group opacity and blend mode.** Once the slab exists, giving the header an
  opacity dial and a blend mode is nearly free (the unit's own draw already
  has both fields, pinned at 100/Normal) — and it is Photoshop's full group
  model. Wanted, or is that scope creep on an organisational tool?
- **Timeline lanes for group effect keyframes** (§6's ponytail): is
  panel-plus-graph-editor editing acceptable to ship, or are lanes under the
  header a v1 requirement — accepting the K-702 row-shape change that
  implies?

## Proposed decision entry (for docs/02-DECISIONS.md — appended and numbered by GE1, not by this note)

> ## K-### — Effects on a layer group header scope to the members: an implicit per-frame precompose
>
> **Status: PROPOSED (2026-08-31).** Owner ask: "apply effects onto the group
> head itself, and it acts like an adjustment layer for the effects within it,
> but not outside." Amends K-702's promise; design in
> docs/impl/group-effects.md.
>
> `LayerGroup` gains an `effects: Vec<EffectInstance>` (serde-skipped while
> empty). When the header stack is live and the drawn run non-empty, the build
> walk wraps the run's draws in one comp-sized `DrawSource::Nested` unit whose
> `fx` is the header's stack resolved at comp time — the Precomp layer's
> existing render path, no new staging. K-702's "identical picture grouped or
> ungrouped" is reworded **"…when the header carries no effects"** in
> group.rs, docs/03 §5.4 and docs/07 §4.2a; on every frame where no header is
> live, the walk stays group-blind and every existing key and cached frame
> holds. Members inside a live unit composite in isolation (Photoshop's
> non-Pass-Through group rule): blend modes stop reaching the backdrop below
> the group while the header is live. Mattes and layer inputs cross the
> boundary unchanged in both directions, because matte sources render alone by
> construction. The frame key feeds, per live group only, the drawn-run ids
> and the resolved header stack. One new op, `SetGroupEffects`, the
> `SetLayerEffects` shape; Ungroup discards the stack (undo restores it);
> Pre-compose group moves it onto the Precomp layer. v1 boundaries, each
> named in the note with its upgrade: the unit is uncached (K-421 stance),
> temporal/flow effects on a header are inert or passthrough, group params
> take no node-graph wires, and the Timeline shows an fx tick but no lanes —
> the Effect controls panel is the editing surface.
