# Particle research — Particular, EmberGen, and the GPU particle canon

Research for Particle, Lumit's particle simulator (backlog 4). Date: 2026-08-24.
Sources: Maxon's Particular manual and the archived Particular help, JangaFX docs and CG
Channel's EmberGen coverage, Bridson's SIGGRAPH 2007 curl-noise paper, Simon Green's CUDA
particles whitepaper, the Wicked Engine GPU-particle write-up, Epic's GPU-particle depth
collision documentation, Unity's VFX Graph docs. Not canonical; this feeds a future
design in docs/08 style, not code.

**In plain terms.** A particle system is thousands of tiny sprites born at an emitter,
pushed around by forces, and drawn fading and shrinking until they die. The whole craft
is in three questions: *where are they born* (emission), *what happens across each one's
life* (curves over life), and *what pushes them* (forces). The two reference tools answer
them from opposite ends: **Trapcode Particular** is the motion-graphics answer — an
*effect on a layer* inside After Effects, exactly the shape Lumit needs — and
**EmberGen** is the performance ceiling — a standalone GPU simulator whose numbers show
what a modern GPU can actually do.

---

## 1. The two reference tools in one paragraph each

**Particular** ([Maxon product page](https://www.maxon.net/en/product-detail/red-giant/particles-and-3d/trapcode-particular))
is an AE effect: one emitter per instance (parent/child systems since v4), CPU
simulation with GPU-accelerated rendering, parameters in ordinary AE property groups so
everything keyframes and takes expressions. Its enduring ideas: emitter types that reuse
the host's own objects (lights, layers, text), per-particle behaviour drawn as small
curves over normalised life, and a physics group (Air / Bounce) that is deliberately
shallow — a look tool, not a simulator. Practical counts are tens of thousands to the
low hundreds of thousands; it is not built for millions.

**EmberGen** ([JangaFX](https://jangafx.com/software/embergen),
[CG Channel on 1.0](https://www.cgchannel.com/2023/03/jangafx-releases-embergen-1-0/),
[on 2.0](https://www.cgchannel.com/2025/01/check-out-the-new-features-due-in-embergen-2-0/))
is a standalone real-time volumetric fluid simulator: everything is a node graph, the
whole sim lives on the GPU, and 2.0's rewritten GPU particle system claims **over 500
million particles on a 24 GB card**, with sparse volumes reaching ~200 million active
voxels on 6 GB. Two facts matter for Lumit beyond the raw numbers: EmberGen's
simulations are **deterministic** — same project, same result, with a seed for
variations ([JangaFX FAQ](https://docs.jangafx.com/embergen/pages/FAQ.html)) — and 2.0
adds a **simulation cache** so the user can scrub a stateful sim
([CG Channel](https://www.cgchannel.com/2025/01/check-out-the-new-features-due-in-embergen-2-0/)).
Both are exactly the problems §6 below has to solve under Lumit's rules. The full FLIP /
sparse-volume machinery is *not* relevant: Lumit is a motion-graphics tool, and a
sprite-particle system covers the genre; smoke and fire are EmberGen's job and arrive as
footage.

## 2. Emission

### 2.1 What the references offer

**Particular** emitter types: Point, Box, Sphere, **Light** (an AE light layer's
position drives the emitter — animate the light, the emitter follows), **Layer** (a 3D
layer's surface emits; sampling is uniform over the layer's bounds — masking the layer
does *not* concentrate the set number of particles into the masked part, a known wart),
Layer Grid, and Text/Mask via the Designer
([training series](https://www.youtube.com/watch?v=YkD3a0ZpNtQ),
[layer emitters](https://www.youtube.com/watch?v=eN3MOSqxlqk),
[Creative COW on layer sampling](https://creativecow.net/forums/thread/trapcode-particular-layer-emitter-problem/)).
Emission quantity is **Particles/sec**, animatable — bursts are done by keyframing the
rate to spike, which works but is clumsy against beats. **EmberGen** emits from shape
primitives and imported meshes, with emission masks and per-node injection controls;
quantity and burst logic are graph inputs like everything else
([CG Channel 1.0](https://www.cgchannel.com/2023/03/jangafx-releases-embergen-1-0/)).

### 2.2 What Lumit should offer

Lumit has two assets no plugin gets for free: **shape layers** and **masks**, both
already reachable by effects (the mask-path reference kind, K-408, walks a mask's
geometry today — the Stroke-style effects use it). The natural emitter list:

- **Point** (a 2D point parameter with crosshair pick, px@comp) with a direction and
  spread (degrees);
- **Line** (two points);
- **Ellipse / Rectangle** (centre + size), interior or edge;
- **Mask path** (a mask-path reference), interior or edge — the marquee feature, since
  the user draws the emitter with the pen tool they already know;
- **Layer** (a layer reference, the DOF §3.22 machinery): emit where the layer is
  bright or opaque, weighted by luma or alpha — the honest version of what Particular's
  Layer emitter fails to do.

**Rate vs burst.** Rate is *Particles per second*, integrated in rational time: each
frame owes `rate × dt` particles; carry the fractional part deterministically by
hashing (seed, frame index) against it rather than accumulating a float across frames,
so any frame's births are computable without history. Bursts ride the existing
**marker-trigger** parameter kind (docs/08 §1.4): *Burst on marker* with a count — beat
markers then drive emission with no keyframing, which is precisely the montage use.

### 2.3 The maths

**Uniform inside a bezier shape.** Flatten the path to a polygon (the adaptive
flattening the mask rasteriser already does), triangulate (ear clipping; fan is enough
for convex), then pick a triangle with probability proportional to its area and sample
uniformly inside it with the square-root trick: for triangle (A, B, C) and uniform
r₁, r₂ ∈ [0,1),

```
P = (1 − √r₁)·A + √r₁(1 − r₂)·B + √r₁·r₂·C
```

— the √ undoes the area distortion so points do not clump at A
([worked example](https://blogs.sas.com/content/iml/2020/10/21/random-points-in-polygon.html)).
The triangle pick is an inverse-CDF lookup into a prefix sum of triangle areas, built
once per resolve on the host. Holes (a mask combining subtractively) fall out of the
triangulation. For **self-intersecting** paths, triangulate the even-odd or non-zero
fill the mask already resolves to — same rule as rasterising it.

**Uniform along a bezier edge.** Arc length of a cubic has no closed form; build a
per-segment length table with Gauss–Legendre quadrature (5-point per segment is ample)
or from the flattened polyline, then sample by inverse CDF over cumulative length and
invert within the segment by a few Newton steps
([Levien on bezier arc length](https://raphlinus.github.io/curves/2018/12/28/bezier-arclength.html)).
Both tables are tiny, host-built, and deterministic — the GPU only ever does the final
lookup with a hashed random.

**Weighted by a layer's luma/alpha.** Build a one-frame CDF: downsample the source
layer to a fixed grid (say 256×256 — resolution independence, and the budget is
bounded), prefix-sum the chosen channel, sample the 2D inverse CDF (row CDF then column
CDF), jitter within the cell. All seeded, all per-frame, no history. Note the layer is
sampled *at the particle's birth frame*, which matters for §6.

## 3. Behaviour over life

### 3.1 Particular's model — curves over normalised life

Particular's Size over Life and Opacity over Life are small drawn curves whose x-axis is
**life as a percentage** (birth → death, whatever the actual duration), with presets,
smooth/random/flip operations, and paint-to-draw
([archived help](http://hbenouil.free.fr/Trapcode/help/particular/size_over_life.html)).
The curves themselves are not keyframeable — only the scalar they multiply (Size,
Opacity) animates. Randomisation is a separate scalar per property (Size random %, Life
random %, …): each particle scales its curve by a hashed per-particle factor.

### 3.2 EmberGen's model — the graph

EmberGen has no fixed "over life" slots; ramps and curve nodes are wired to whatever the
user wants, which is strictly more powerful and strictly harder to reach parity with in
an effect UI ([node list](https://docs.jangafx.com/embergen/pages/references/node_list.html)).
Not the right model for Lumit's effect surface — Lumit's graph-shaped power tool is
expressions, which already reach every scalar parameter.

### 3.3 Recommendation

Particular's model maps almost one-to-one onto what Lumit already has. **`ParamKind::Curve`
(K-412)** is 2..16 control points in the unit square with a clamped cubic through them,
baked to a 257-entry table per resolve, static (not keyframed) — which is exactly what
an over-life curve is: x = normalised life, y = multiplier. So:

- *Size over life*, *Opacity over life* — Curve parameters, default a gentle
  fade-in/fade-out (the "drop it on and it already looks right" rule);
- each paired with a *random* percentage: the per-particle factor is
  `1 + random% × (hash(seed, particle_id) × 2 − 1)`, stateless per §2.4;
- *Rotation* — initial angle + spin (degrees/s) + spin random, no curve needed in v1;
- *Wobble* — a positional jitter over life: amplitude (px@comp) × the seeded value-noise
  generator the catalogue already ships (docs/08 §3.37 machinery), sampled at
  (particle_id, age × frequency) — deterministic and hop-free, the Shake precedent;
- *Colour over life* — the one gap: a gradient has no parameter kind. v1: **Birth
  colour** and **Death colour** (two colour parameters) blended along a Curve; a real
  gradient kind is open question 3 in §8.

That the curves do not keyframe is not a loss — Particular's do not either, and nobody
has ever filed that as the missing feature.

## 4. Forces and motion

The canonical force list, all per-frame accelerations unless noted:

- **Gravity** — strength (px/s²@comp) + direction (degrees, default down). Closed form
  under integration: `p(t) = p₀ + v₀t + ½gt²`.
- **Wind** — a constant acceleration vector; same closed form.
- **Drag** (Particular: Physics > Air > Air Resistance) — linear drag `dv/dt = −kv`
  has the closed form `v(t) = v₀e^{−kt}`, `p(t) = p₀ + v₀(1 − e^{−kt})/k`; still
  stateless.
- **Turbulence** — Particular's Turbulence Field is a **displacement**: three channels
  of animated Perlin-fractal noise added to the particle's *position* (with scale,
  complexity, evolution speed), not a force integrated over time
  ([Maxon manual](https://help.maxon.net/rg/en-us/Content/html/Particles-and-3D/Trapcode%20Particular/displace-turbulence-field.html)).
  That is why it scrubs perfectly: displacement is a pure function of (base position,
  age, time). The higher-grade approach is **curl noise** — Bridson, Houriham,
  Nordenstam, SIGGRAPH 2007: take a noise potential ψ and use its curl as the flow
  field; in 2D, `v = (∂ψ/∂y, −∂ψ/∂x)`. The curl is divergence-free by construction, so
  the flow never bunches up or drains away — it *looks like fluid* without simulating
  any ([paper PDF](https://www.cs.ubc.ca/~rbridson/docs/bridson-siggraph2007-curlnoise.pdf),
  [ACM](https://dl.acm.org/doi/10.1145/1275808.1276435)). The catch for Lumit: sampling
  the curl field *at the particle's current position* and integrating is stateful;
  sampling it along the closed-form base trajectory and applying it as displacement
  (Particular's trick, with curl instead of raw fractal) keeps it stateless and covers
  most of the look. Offer **Displacement** (stateless) and note that field-following is
  what §6's simulated mode buys.
- **Attractor** — a point (crosshair pick) with strength (attract/repel) and falloff
  radius; Particular's spherical field, EmberGen's forces nodes. Genuinely
  position-dependent: stateful, simulated-mode only.
- **Inherit motion** — a percentage of the emitter's own motion at birth added to the
  birth motion of new particles (Particular's Motion inheritance): the emitter draws a
  streak of particles behind its movement instead of a dotted trail. Host-side and
  stateless — the emitter's position curve is differentiable at birth time.

## 5. Occlusion and collision

**Depth-map occlusion.** Lumit already has the pattern: Depth of field (§3.22) takes a
depth pass as a layer reference. Particle reuses it — a *Depth layer* input; each
particle carries a depth value (from its 3D position, or a flat per-particle depth in
2D), and the render kernel compares against the sampled depth pass, discarding or fading
sprites behind the scene. Cheap, per-pixel, and stateless — it is a render-time test,
not a simulation feature. Soft fade over a threshold band avoids hard pops.

**Depth-buffer collision.** The standard GPU-engine technique — Wicked Engine
([write-up](https://wickedengine.net/2017/11/07/gpu-based-particle-simulation/comment-page-1/)),
UE4's GPU particles ([Epic content example](https://docs.unrealengine.com/4.27/en-US/Resources/ContentExamples/EffectsGallery/1_E)),
Unity — is: project the particle into screen space during the *simulate* kernel; sample
the depth buffer; if the particle would pass behind the surface, reconstruct the surface
normal from the depth buffer's local gradients and **reflect the particle's motion vector about it**,
scaled by a bounce factor, with a tangential slide/friction term. Its known limits are
inherent to the trick: only surfaces visible in the depth image collide, and a particle
that drifts behind geometry loses its floor. For Lumit the "depth buffer" is the same
user-supplied depth layer as above, so the quality of collision is the quality of the
depth pass — worth saying plainly in the manual when this ships. Collision is
position-dependent and therefore simulated-mode only (§6).

Sprite-vs-sprite collision (Simon Green's uniform-grid spatial hashing: hash particles
to cells, radix-sort by cell id, test the 27 neighbour cells —
[CUDA particles whitepaper](https://developer.download.nvidia.com/compute/cuda/2_2/sdk/website/projects/particles/doc/particles.pdf))
is the classic paper but the wrong feature for a motion-graphics tool: nobody grades a
montage by whether sparks bounce off each other. Skip it; the hashing machinery is
documented here in case flocking/self-avoidance is ever wanted.

## 6. Determinism — what can be stateless, honestly

Lumit's rules (docs/08 §2.4, docs/14): randomness is **seeded and stateless** —
`hash(seed, frame_index, element_id)` generators only; two exports of the same project
are bit-identical; wall clock, thread scheduling and GPU vendor must not influence
output. A particle *simulation* is the textbook counter-example — state that evolves
frame to frame — so the honest split is:

**Stateless-replayable (the "procedural" core).** Everything whose trajectory has a
closed form in (seed, particle_id, birth_time, current_time): birth position and
motion (all of §2 — the sampling tables are per-resolve host data), gravity, wind,
linear drag, turbulence-as-displacement, wobble, every over-life curve, colour, spin,
depth-map occlusion. For any frame t, the kernel enumerates particles alive at t
(birth frames are computable from the rate curve alone), evaluates each trajectory
directly, and draws. **No history, no cache, any frame in any order** — scrubbing,
Retime, and render-order freedom all come free, and §2.4 is satisfied verbatim. This is
Particular's effective architecture and covers the overwhelming majority of
motion-graphics particle work. One caveat: a *moving* emitter means birth positions
depend on emitter parameters at past times — the host resolves the emitter's property
curves at each birth frame into a small per-frame table (bounded by max life × rate),
which is deterministic but is why max life needs a hard cap.

**Needs a simulation cache (the "simulated" extras).** Position-dependent forces —
attractors, field-following curl noise, depth-buffer collision with bounce/slide —
have no closed form; the frame-N state depends on frame-N−1. The precedent is already
in the house: the camera track's sidecar (K-417, K-248) — a background job, keyed by
its inputs, cached in the project sidecar, *rebuildable and deterministic so a rebuild
is byte-identical*. A particle sim cache is the same animal: **fixed-step integration**
(step = the comp frame interval in rational time, never wall clock; substeps a fixed
declared count), f32 state, one kernel in one dispatch order, all randomness still
`hash(seed, id, step_index)`, and any compaction or reordering done with stable keys
(tie-break on particle id) so the buffer contents are reproducible bit-for-bit.
Evaluation at frame N replays steps 0..N from the nearest cached keyframe of state;
scrubbing backwards reads the cache. EmberGen's 2.0 caching and its "all simulations
are deterministic" stance ([FAQ](https://docs.jangafx.com/embergen/pages/FAQ.html))
show this is a solved, shippable combination. The mode switch should not be a user
decision: the effect is procedural until the user enables a stateful force, and the UI
says which forces cost a cache — no punishment, just the fact.

## 7. Performance

**Budgets.** Particular's practical ceiling is ~10⁵ particles; EmberGen 2.0 claims
5×10⁸ on a 24 GB card. Lumit's realistic v1 target sits comfortably between: **cap
1,000,000 particles per effect instance, default a few thousand**. At 48 bytes of state
a particle (below), a full-cap instance is 48 MB of VRAM — inside the governor's ledger
without drama — and a simulate pass over 1M particles is well under a millisecond on
the reference GPU; the frame budget (docs/13 B-series; a whole-frame dispatch should
stay under ~10 ms, individual dispatches < ~4 ms for cancellation) is threatened by
*sorting and overdraw*, not simulation.

**Buffer layout.** The canon (Wicked Engine, and the same shape in Niagara and VFX
Graph) is:

```
particles:   position ×2 (f32), motion ×2 (f32), birth_time (f32), life (f32),
             seed_id (u32), flags (u32)                      → 32 B core
             + rotation, spin, size_scale, colour_index …    → ~48 B total
dead_list:   u32 indices of free slots (starts full)
alive_list:  ×2, u32 indices, ping-pong — emit appends to A; simulate reads A,
             writes survivors to B; render draws B; swap
counters:    alive count, dead count, real emit count (atomics)
indirect:    dispatch args for simulate (built from counters by a tiny
             "kick-off" kernel), draw args for render (instance count = alive)
```

Emit pops indices off the dead list; simulate pushes expired indices back. The CPU
never reads a count — a kick-off kernel writes `DispatchIndirect`/`DrawIndirect`
arguments from the counters, so the whole frame is GPU-driven
([Wicked Engine](https://wickedengine.net/2017/11/07/gpu-based-particle-simulation/comment-page-1/)).
Strict SoA (separate position/motion/attribute buffers) buys bandwidth when kernels
touch subsets; a 16-byte-aligned AoS struct is simpler and fine at 1M — start AoS,
split only if the profiler says so. Note the **stateless mode needs almost none of
this**: no persistent state means no dead/alive lists — one kernel evaluates
trajectories straight into an instance buffer. The full layout is the simulated mode's.

**Render.** Instanced quads (billboards), vertex shader pulls the particle by
`alive_list[instance_index]`; streak mode stretches the quad along the motion
direction. Blending: Add and Screen are order-independent in exact arithmetic, Normal
is not — Normal blending needs a **depth/age sort**. Determinism bites here twice: the
sort must be a stable key sort (key + particle id) so equal keys never swap
run-to-run, and fp16 additive accumulation is order-sensitive anyway — the flare
MSAA lesson — so even Add should composite in the sorted, stable order rather than
"whatever order the rasteriser lands". GPU radix sort is the missing wheel (wgpu has
no built-in; a subgroup-free radix sort is a known ~200-line WGSL exercise). Overdraw
is the other classic cost; GPU Gems 3's half-resolution particle pass is the standard
mitigation if scenes demand it
([GPU Gems 3 ch. 23](https://developer.nvidia.com/gpugems/gpugems3/part-iv-image-effects/chapter-23-high-speed-screen-particles)).

**What the wgpu foundation already provides** (docs/impl/gpu-foundation.md): the
device/queue and submit thread, the fp16 premultiplied working format, the texture
pool and governor ledger (buffer allocations go through the same ledger), the
per-frame uniform arena, 8×8 workgroup convention and the common uniform header
(roi, comp_scale, time, **seed**), timestamp profiling, and the <4 ms dispatch
checkpointing that gives cancellation. Missing and needed: a *buffer* pool sibling to
the texture pool, the radix sort, and indirect-draw plumbing in the effect executor
(effects today are image-in/image-out dispatches; Particle is the first effect that
rasterises its own geometry — the Lens flare's sprite pass is the nearest precedent).

## 8. Proposed v1 parameter surface

Sized against the fx system as it stands: every kind below exists today (float, slider,
int, bool, enum, angle, colour, 2D point, curve K-412, seed, layer reference K-123,
mask-path reference K-408, marker-trigger §1.4), the universal Matte row (§2.6) comes
free, and the depth input reuses DOF's layer-reference pattern.

| Group | Parameter | Kind | Notes |
|---|---|---|---|
| *Emitter* | Type | enum | Point / Line / Ellipse / Rectangle / Mask path / Layer |
| | Position | 2D point | crosshair pick; Line adds a second point |
| | Size | float ×2, px@comp | Ellipse/Rectangle |
| | Mask path | mask-path ref | K-408; greyed unless Type = Mask path |
| | Source layer | layer ref | Type = Layer; Weight by: enum Luma / Alpha |
| | Emit from | enum | Interior / Edge (shape and mask types) |
| | Direction | angle + Spread (degrees) | |
| *Emission* | Particles per second | float | animatable; rational-time integration |
| | Burst on marker | marker-trigger + Burst count (int) | beat-driven emission |
| | Life | float, seconds + Life random % | hard cap (open question 2) |
| | Initial speed | float, px/s@comp + Speed random % | |
| | Inherit motion | slider 0–100 % | emitter speed at birth |
| *Life* | Size over life | curve + Size random % | default fade-in/out |
| | Opacity over life | curve + Opacity random % | |
| | Birth colour / Death colour | colour ×2 + Colour blend curve | gradient kind is open question 3 |
| | Spin | float, degrees/s + Spin random % | |
| | Wobble | float, px@comp + Frequency | seeded value noise, §3.37 machinery |
| *Motion* | Gravity | float, px/s²@comp + angle | |
| | Wind | float, px/s²@comp + angle | |
| | Drag | slider 0–1 | closed-form exponential |
| | Turbulence | float + Scale + Evolution + Type (Displace / Curl displace) | stateless |
| | Attractor | 2D point + Strength + Radius | **simulated mode** |
| *Occlusion* | Depth layer | layer ref + Threshold + Softness | DOF's depth pattern |
| | Collide | bool + Bounce + Slide | **simulated mode**, depth-buffer technique |
| *Render* | Particle | enum | Circle / Soft circle / Streak / Layer (layer ref as sprite) |
| | Size | float, px@comp | the master, ×curve |
| | Blend | enum | Normal / Add / Screen |
| | Streak length | float | Streak only |
| *Simulation* | Seed | seed + reseed | §2.4 standard row |
| | Particle cap | int, ≤ 1,000,000 | governor-honest |

Trait declaration: `expensive` cost, `full-frame` ROI (particles travel), `seeded`,
temporal `{0}` in procedural mode — the honesty of the simulated mode's history is
carried by the sidecar cache, not a temporal window.

**The three hardest open questions for the owner:**

1. **2D or 2.5D?** Lumit now has a camera and depth-aware effects. Do particles live
   flat in comp space (Particular-in-a-2D-comp, ships sooner) or in 3D comp space,
   billboarded to the camera and parallaxing with it (what the camera-track era
   invites)? This decides the position kinds, the depth story, and half the render
   kernel — it should be decided before any code, not migrated later.
2. **Where does the simulated mode's cache live and what invalidates it?** The camera
   track's sidecar is keyed to the *source*; a particle sim is keyed to *every
   parameter of the effect* — any edit rebuilds from frame 0. Is replay-on-edit
   acceptable at the cap, or does v1 ship procedural-only and defer attractors and
   collision to a v1.1 with the cache?
3. **Bit-identical blending at a million sprites.** §2.4 promises bit-identical
   exports; rasterised alpha blending is order-dependent, so determinism demands a
   stable sort and a fixed composite order every frame, cross-vendor. The flare's MSAA
   lesson says this is where nondeterminism actually hides. What tolerance, if any, is
   acceptable here — or does Particle become the effect that forces the docs/14 open
   question about cross-machine fp16 exactness to a decision?
