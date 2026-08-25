# Anti-aliasing the composite — impl note (binding for its topic)

Feeds [06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md) §1 (the composite step) and
[03-DATA-MODEL.md](../03-DATA-MODEL.md) (the project property). The spec says *what*;
this note is the authoritative *how*: which stage aliases, why multisampling rather
than supersampling, the four traps in the composite loop as it stands, and the test
plan. Written from K-274's decision before the code; **now built** — the sections below
describe what is there, and §5's test plan is implemented alongside it.

## In plain terms

A layer is drawn as a rectangle placed by its transform. Rotate that rectangle a
few degrees and its edge crosses a pixel diagonally — but a pixel is either drawn
or not, so the edge comes out as a staircase, and on a slow rotation the steps
crawl. That is *aliasing*: the picture is being sampled once per pixel, at the
pixel's centre, and one sample cannot describe an edge that only covers part of it.

The cure is to ask about coverage more than once per pixel. **Multisampling**
(MSAA) does exactly that and nothing else: the card keeps four (or two, or eight)
coverage samples per pixel, works out the colour **once**, and averages by how many
samples the shape actually covered. It costs memory and a resolve step, not four
times the shading — which is why it is the standard answer for edges.

What it does *not* fix is worth stating, so nobody expects it to: the inside of a
layer's picture is a texture lookup, and its quality is the sampler's business, not
coverage's. A shape's own curves, a mask's edge and a glyph's outline are already
anti-aliased where they are rasterised (`lumit_core::mask::rasterise` takes two
vertical sub-samples and exact horizontal coverage per scanline; `lumit-text` gets
coverage in the glyph's alpha). What stair-steps today is the **quad edge**: the
boundary of the layer's own rectangle, wherever the transform turns it off-axis.

## 1. The decision (K-274, owner)

- **A project property, not a preference.** The sample count changes what a comp
  looks like, so it travels in the `.lum` and matches on another machine.
- **On by default.**
- **One value for preview and export.** A preview that anti-aliased differently
  from the file would break the K-031 preview-equals-export identity, which is the
  promise the whole render path is built around.

Consequences that follow and are therefore also binding: the value is a
[03-DATA-MODEL.md](../03-DATA-MODEL.md) project field with a serde default (so
older `.lum` files load and newer ones round-trip, [10-FILE-FORMAT.md](../10-FILE-FORMAT.md)
§1.1), it crosses the bridge in `BridgeCompModel`/project settings rather than
living in the workspace file, and export reads the same field the Viewer does.

## 2. Why MSAA rather than supersampling

| | MSAA on the composite target | SSAA (render the comp at k× and downsample) |
|---|---|---|
| Fixes quad edges | yes | yes |
| Fixes interior resampling | no | yes |
| Fragment cost | one shade per pixel | k² shades per pixel, over the whole composite |
| Interaction with reduced-resolution preview | none (orthogonal) | fights it — the preview scale and the AA scale multiply |
| Interaction with px@comp rescaling (K-266, K-268) | none (the layer rasters are untouched) | every px parameter needs the AA factor folded in too |

MSAA wins on the two rows that matter here: the artefact being fixed *is* a
coverage problem, and SSAA would multiply against `render_scale` and against the
px@comp factor the realise walk already applies — two corrections that are hard
enough to keep honest once each.

**Sample counts are asked for, never assumed.** `wgpu` reports what a format
supports through `Adapter::get_texture_format_features(Rgba16Float).flags`
(`MULTISAMPLE_X2` / `X4` / `X8`). The working format is `rgba16float`; a device
that will not multisample it at the asked-for count falls back to the highest it
will do — down to 1, which is today's picture — and says so on the calm status
line ([15-DESIGN.md](../15-DESIGN.md): no red-alert states). It never fails a
render: an unsupported count is a machine's limit, not a project's error.

## 3. Where it goes in `lumit-gpu::composite`

The composite loop (`composite_seeded`) is not a single pass, and that is where
every trap lives. As it stands it:

1. optionally **copies a seed texture** into the target (the previous
   adjustment-layer stage's output);
2. walks the layers, opening a **new render pass at each snapshot-blend
   boundary** (Overlay and friends read the backdrop), and between those passes
   **copies the target into a snapshot texture**;
3. leaves the finished single-sample target for the caller to read back, blit or
   hand to the Viewer.

Multisampling that means keeping **one persistent MSAA colour texture** beside the
existing single-sample `target` for the whole composite:

- every render pass attaches the MSAA view with `resolve_target: Some(target_view)`,
  so each pass leaves a resolved copy in `target` — which is what the snapshot copy
  and the caller both need;
- passes after the first use `LoadOp::Load` **on the MSAA attachment**, whose
  contents persist between passes. Loading from the resolve target is not a thing
  the API offers and not what is wanted;
- the resolve is per pass, and passes are one per snapshot-blend boundary — one
  for the common case of no snapshot blends at all.

### The four traps

1. **`copy_texture_to_texture` cannot cross sample counts.** The seed copy in
   step 1 must become a full-screen **draw** of the seed into the MSAA target (its
   own tiny pipeline at the MSAA count, sampling a single-sample source). Copying
   into a multisample texture is invalid, and copying into the *resolve* target
   instead would be silently discarded by the first pass's load.
2. **Pipelines carry their sample count.** Every `create_render_pipeline` in the
   module hard-codes `multisample: Default::default()` (count 1). Pipelines must be
   built for the count in use — either a set per count built on demand and cached,
   or a `Compositor` that is rebuilt when the project's setting changes. Prefer the
   cache: the setting is per project and the compositor outlives it.
3. **Every reader wants the resolved texture.** Read-backs (`readback8`,
   `start_readback8`), the Scopes trace, the shared-texture hand-off and the
   display blit must all take `target`, never the MSAA texture. A multisample
   texture cannot be sampled by an ordinary shader or copied to a buffer.
4. **The sibling passes draw geometry too.** `motion_blur_average` and the
   coverage pass (`coverage_texture`) place quads exactly as the composite does, so
   an anti-aliased composite with an aliased motion-blur average would show the
   seam. They take the same treatment, at the same count, or the setting is a lie
   on any blurring layer.

`accumulate` is the exception: it averages finished textures with no geometry of
its own, so it stays single-sample and reads resolved inputs.

## 4. Preview, export and the identity

Both paths run the same `Realiser`, so the count must reach it the same way
`render_scale` does — a field on the realiser, read from the project. Export builds
with `render_scale` 1.0 and the *same* sample count; that is what makes the K-031
identity hold at full resolution with AA on.

Reduced-resolution preview is orthogonal and must stay so: the count does not
change with the scale. A half-resolution preview is a smaller picture with the same
edge treatment, which is what "the preview is the same picture, only softer" has
always meant.

## 5. Test plan (implement with the feature)

1. **The staircase goes** — a rotated opaque solid on a transparent background.
   At count 1, every pixel's alpha is 0 or 255 along the edge; at count 4 there is
   a run of partial-alpha pixels down it. Count them: the test is "more than a
   handful of intermediate alphas exist" against "exactly none", which is a
   difference no fp16 tolerance argument can blur. Runs on lavapipe.
2. **Straight edges are untouched.** An axis-aligned solid must render
   **bit-identically** at count 1 and count 4 — MSAA may not soften an edge that
   lands exactly on a pixel boundary, and if it does, something is drawing at
   half-pixel offsets.
3. **Preview equals export.** Add an AA row to the K-031 matrix
   (`the_preview_and_export_paths_agree_across_the_matrix`), so the two walks are
   compared with the setting on.
4. **Snapshot blends survive.** A comp with an Overlay layer over a rotated solid:
   the mid-composite resolve and snapshot copy must produce the same picture as the
   single-sample path does today, plus the softened edge. This is trap 1 and trap 3
   in one scene.
5. **Motion blur agrees with the composite.** A blurring rotated layer, checked
   for the same edge treatment as a still one — trap 4.
6. **An unsupported count degrades calmly.** Ask for 8 on an adapter that reports
   only 4: the render succeeds at 4 and reports the count it actually used. Never
   an error, never a panic ([14-ENGINEERING-RULES.md](../14-ENGINEERING-RULES.md)).
7. **The property round-trips.** Save and reload a project with the count set;
   a `.lum` written before the field existed loads at the default.

## 6. What landed, and where

- **The setting.** `Document::anti_aliasing` (`AntiAliasing::{Off,X2,X4,X8}`, default `X8` — K-286)
  in `lumit-core/src/model.rs`, written through `Op::SetAntiAliasing` so it is undoable and
  journalled. Across the bridge as `ProjectReference::{anti_aliasing, set_anti_aliasing}`,
  with `anti_aliasing_in_use` reporting what the adapter will actually give — the two are
  kept apart so a limited card never rewrites the project.
- **The capability check.** `lumit_gpu::supported_sample_count` (adapter in hand) and
  `GpuContext::sample_count` (from the flags the context carries), sharing one rule.
  `lumit_gpu::adapter_sample_count` is the lock-free reporting path the Project settings row
  reads. Both are held to what the *device* will accept, not to what the adapter advertises:
  counts beyond 1 and 4 need `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` enabled at device
  creation, and reading the adapter's answer without it is what let an 8× setting reach
  `create_texture` and fail there.
- **The composite.** `Compositor` now keeps a `PipelineSet` per sample count, built on
  demand and cached (trap 2); count 1 is built up front so an un-anti-aliased frame costs
  exactly what it did. `composite_seeded` takes `samples`, allocates the multisample
  attachment beside the target, resolves per pass, and draws the seed through `fs_seed`
  rather than copying it (trap 1). `motion_blur_average` takes the same count (trap 4).
- **The walk.** `Realiser::samples`, beside `render_scale` — but read from the project by
  both the preview and the export path, which is what makes §4 hold.
- **The cache.** The count is fed into `comp_frame_key`, and `ALGO_VERSION` went to 3: every
  frame banked before this was made without anti-aliasing and may not be served again.
- **The interface.** The **Project settings** window (`File ▸ Project settings…`,
  `Mod+Alt+Shift+K`), which exists to keep the project's own values out of Settings — where
  everything else belongs to this machine and neither travels nor undoes (K-286).

Not built, and deliberately: nothing anti-aliases the *interior* of a layer's picture (that
is the sampler's business, §In plain terms), and there is no per-comp override — the count is
one project-wide value, which is what K-274 decided.

## Feeds

[06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md) §1, [03-DATA-MODEL.md](../03-DATA-MODEL.md),
[10-FILE-FORMAT.md](../10-FILE-FORMAT.md) §1.1, [13-PERFORMANCE-RULES.md](../13-PERFORMANCE-RULES.md)
(the count is a per-frame cost worth a budget line), K-274.
