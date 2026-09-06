# Particulate — the particle system as a points pipeline (design)

**Decision:** Particulate is a separate effect emitting a points stream, through the
`Points` port type and the stack-effect-with-a-points-output shape. Two entries
proposed by this note have since been **confirmed**: closed-form evaluation with no
simulation state, and the particle cap as the user's budget dial. **Related:** drivers
and the wiring model, px@comp, the Mix row's Blend, mask-path parameters, layer
references, per-effect temporal sampling, preview equals export, **the particles live in
three axes and the composition's camera sees them**, and the depth rows with the
projection that rides beside the op. **Status: commissioned** (the owner's 2026-08-24
commission) — [points-stream.md](points-stream.md) is the infrastructure design
(consumption, consumers, seam) and holds the ordered work packages (PS1–PS7); this note
remains the effect's own *how*. When PS2 lands the effect whole, docs/08 gains its §3.x
entry derived from this note.

## In plain terms

A particle system makes many small things — sparks, dust, snow, streaks — that are born,
drift about, and fade away. The classical way to build one is a **simulation**: each frame
takes the last frame's particles and nudges them along. That is exactly the kind of state
Lumit's engine forbids, and for good reason — a simulation can only answer "what does
frame 500 look like?" by computing frames 0 to 499 first, so scrubbing stutters, caches
are fragile, and two renders can disagree.

Particulate does it the other way round. Every particle's whole life is decided the
moment it is born — where it starts, which way it flies, how the wind and gravity will
carry it — from a seeded random number that never changes. Asking for frame 500 is then
just arithmetic: for each particle alive at that moment, compute where its formula puts
it, and draw it. No frame depends on any other frame. Scrubbing anywhere is instant,
export matches preview bit for bit, and the same project renders the same pixels forever.
The price is that particles cannot *react* to each other (no collisions, no flocking) —
and for the montage staples this effect exists for, that trade is the right one.

The effect also hands its particles out as data — the **points stream**: positions,
ages, sizes, colours per frame — so the later family (Connect points, Clone to points,
Trail…) can build on the same particles rather than each inventing its own.

## 1. What v1 is, and is not

**Is:** one stack effect, image in → image out (it draws its particles over its input),
plus a declared `Points` output. Emitter shapes including a mask path, the four
analytically integrable forces (gravity, wind, drag, turbulence), three render modes
(disc, sprite, streak), colour/size/opacity over life. Deterministic, random-access,
scrub-safe.

**Is not:** a simulation. No collisions, no per-particle interaction, no state carried
between frames — §8 specs the exception's contract should one ever be needed, and it is
deliberately not built. Also not in v1: emitting from the image's own bright pixels, and
the stack-effect family consuming the points output (Connect points, Clone to points, Trail, Scatter) — deferred
as named packages in [points-stream.md](points-stream.md) §2.3. The one v1 consumer
beyond this effect's own drawing is the **Points sample** driver.

## 2. Parameter surface

Groups follow the Effect-controls convention; every parameter is animatable and drivable
unless noted. Units per docs/08 §2.3; all distances **px@comp**. Defaults are chosen to
satisfy the "drop it on and it already looks right" rule: gently drifting, softly glowing
motes over the footage.

### Emitter

| Parameter | Kind / unit | Slider (hard) | Default | Notes |
|---|---|---|---|---|
| Shape | choice | Point · Line · Ellipse · Rectangle · Mask path · Ellipse outline · Rectangle outline | Point | Area shapes emit uniformly over their interior; Line along its segment; Mask path along the arc-length polyline; the two **outline** shapes uniformly along the perimeter of the area they hollow out, by the same arc-length walk. The outlines are appended rather than slotted in beside their fills, because a Choice is stored as its index. |
| Position | 2D point, px@comp | open | comp centre | The `_x`/`_y` pair convention; pick-on-Viewer dropper. |
| Position z | px@comp | −2000–2000 (open) | 0 | How far in front of or behind the layer's own plane the emitter sits. Nought is the plane, which is what every 2D layer draws. |
| Width / Height | px@comp | 0–2000 (0+) | 400 / 400 | Extents for Line (Width only), Ellipse, Rectangle and the two outlines. Ignored by Point and Mask path. |
| Depth | px@comp | 0–2000 (0+) | 0 | The extent *through* the plane, filled uniformly: Point becomes a segment, Ellipse a cylinder, Rectangle a box, and an outline a tube. Ignored by Line (one dimension by name) and by Mask path, which stays planar at the emitter's own depth — the path is where the user drew it. |
| Emitter angle | degrees | −360–360 (open) | 0 | Rotates Line, Ellipse, Rectangle and the two outlines about Position, in the plane. Depth runs through that plane and is not turned by it. |
| Mask path | mask-path reference | — | First mask | Used when Shape is Mask path; empty polyline emits nothing (the documented no-op). Static in v1, like every mask-path reference. |
| Emit rate | none (per second) | 0–1000 (0+) | 150 | Births per second of layer time; the integral is the birth schedule (§3.1). |
| Direction | degrees | −360–360 (open) | −90 | Launch direction in the layer's plane; −90 is up. |
| Direction z | degrees | −360–360 (open) | 0 | The launch's elevation out of that plane: positive throws particles away from the camera. |
| Spread | degrees | 0–360 | 360 | Cone about Direction. |
| Spread z | degrees | 0–180 (0–360) | 0 | The elevation's own cone — a row of its own, so that a full 360 of in-plane Spread stays a disc of directions rather than quietly becoming a sphere. |
| Initial speed | px@comp per second | 0–2000 (0+) | 90 | |
| Speed jitter | per cent | 0–100 | 50 | Per-particle, from the seed. |

### Particle

| Parameter | Kind / unit | Slider (hard) | Default | Notes |
|---|---|---|---|---|
| Life | seconds | 0.1–10 (0+) | 2.0 | |
| Life jitter | per cent | 0–100 | 30 | |
| Size | px@comp | 0–200 (0+) | 4 | Diameter at birth. |
| Size jitter | per cent | 0–100 | 40 | |
| Size over life | curve | unit square | flat 1.0 | Multiplies Size by normalised age. Static in v1, as all curves are. |
| Opacity over life | curve | unit square | 1 → 0 | Declared default points `[[0,1],[1,0]]` — born solid, dies faded. |
| Colour | colour | — | white | Scene-linear; values above 1.0 are legal and useful over glow. |
| End colour | colour | — | white | Blended to over normalised age, in working space. |
| Rotation | degrees | −360–360 (open) | 0 | The angle every particle starts at. |
| Rotation jitter | degrees | 0–360 | 360 | Per-particle spread about Rotation: a uniform draw of ±half of this, folded into the seed hash. A whole turn by default, because a field of sprites all facing one way reads as a mistake; at 0, Rotation means exactly what it says. |
| Spin | degrees per second | −720–720 (open) | 0 | |
| Align to motion | bool | — | off | Rotation follows the speed direction; Spin adds on top. |

### Forces

The four v1 forces are exactly the set with closed-form integrals (§3.2) — that is the
selection criterion, not a styling choice.

| Parameter | Kind / unit | Slider (hard) | Default | Notes |
|---|---|---|---|---|
| Gravity | px@comp per second² | −2000–2000 (open) | 0 | Positive is down. **Down stays down**: gravity is the one force with a direction of its own, and a depth component would be a control nobody asked for. |
| Wind x / Wind y / Wind z | px@comp per second | −2000–2000 (open) | 0 / 0 / 0 | The air's own speed, on all three axes. Wind acts *through* Drag — with Drag 0 it does nothing, and the rows' descriptions say so. |
| Drag | none (per second) | 0–10 (0+) | 0.5 | Exponential approach of the particle's speed toward the wind's. |
| Turbulence amount | px@comp | 0–500 (0+) | 40 | Displacement magnitude. |
| Turbulence scale | px@comp | 10–1000 (10+) | 200 | Spatial wavelength of the noise. |
| Turbulence speed | none (Hz) | 0–5 (0+) | 0.3 | Evolution rate against age. |

### Render

| Parameter | Kind / unit | Slider (hard) | Default | Notes |
|---|---|---|---|---|
| Mode | choice | Disc · Sprite · Streak | Disc | |
| Feather | per cent | 0–100 | 100 | Disc edge softness. |
| Sprite layer | layer reference | — | unset | With the standard source combobox. Rendered once per frame, instanced per particle. **Unset draws discs** — the mode falls back rather than the effect going no-op, because a render mode must always draw something (deviation from the unset-is-identity convention, documented here). |
| Streak length | seconds | 0–0.1 (0+) | 0.02 | Streak mode: a line from `p(t − length)` to `p(t)` — closed form again, no history needed. |
| Max particles | integer | 1–200 000 (1–1 000 000) | 20 000 | **The budget dial** (§7). Not animatable — it is a capacity declaration, like the flare's ray budget, and animating a capacity would re-key the governor per frame. |
| Seed | seed | — | 0 | With the standard reseed button (docs/08 §2.4). |

Plus the host-provided Mix row with its Blend choice — Add on the Blend combo is
how sparks glow over footage, and it costs this effect nothing to support.

**Traits** (docs/08 §1.3): cost `moderate`; ROI `full-frame` (a particle may travel
anywhere); temporal window `{0}` — the payoff of §3, no other frames are ever read;
alpha `premultiplied`; cancellation `per-pass`; randomness `seeded` (so the layer's local
time joins the cache key, the standard rule); marker input `none` in v1 (beat-triggered
bursts are a natural v1.x, riding §1.4's existing plumbing).

`sample_temporally` defaults **on**, like every effect: closed form makes
sub-frame evaluation exact and cheap, so the accumulation Motion blur effect gets true
particle motion blur for free. The flag remains the user's pin if they want the pinned
look.

## 3. Evaluation — closed form, no state

### 3.1 The birth schedule

Births are the one place a walk exists, and it is a walk over *one scalar*, not over
particle state. Per comp frame `f` from the layer's in point, with `Δt = 1/comp_rate`
(rational time throughout, [rational-time.md](rational-time.md)):

```
carry += rate(f) · Δt          // rate sampled at the frame, keyframes and drivers applied
n_f    = floor(carry)          // particles born this frame
carry -= n_f
```

Each birth gets a global **birth index** `b` (monotone from the in point) and a birth time
`t_b` spread evenly inside its frame: `t_b = frame_start + (j + ½)·Δt/n_f`. The schedule
is a pure function of the rate curve, the in point and the comp rate; it is O(frames since
in point) of scalar work (a 60 s comp at 60 fps is 3 600 iterations — microseconds),
computed on demand and cacheable keyed by a hash of exactly those inputs. This is what
keeps random access honest: frame 500's candidate set is enumerable without evaluating
any other frame's *pixels*.

Every per-particle random draw — position within the emitter, direction within the
spread, the speed/life/size/rotation jitters, the turbulence phase — is
`hash(seed, b, attribute_id)`, the stateless generator docs/08 §2.4 mandates. A particle
is a pure function of its birth index.

### 3.2 The closed forms

At frame time `t` (layer time — the same base driver evaluation uses, node-graph.md
§2.1), the candidates are the births in `[t − max_life, t]`; each is alive if
`age = t − t_b < life_b`. With gravity `g` (px/s², vector), wind `w` (px/s, vector), drag
`k` (1/s), initial position `p0` and initial speed vector `v0`:

```
k > 0:   v(age) = w + g/k + (v0 − w − g/k) · e^(−k·age)
         p(age) = p0 + (w + g/k)·age + (v0 − w − g/k) · (1 − e^(−k·age)) / k
k → 0:   p(age) = p0 + v0·age + ½·g·age²          // series guard below k·age < 1e−4
```

**Three components, one algebra**: `p0`, `v0` and `w` are three-vectors and the
formulas above apply component by component, the depth axis included. `g` is
`[0, gravity, 0]` — the one force with a direction of its own. The implementation
rearranges both branches so neither divides by `k`, and moves the series guard to
`k·age = 0.1`, which is where the two genuinely meet in `f32` (`fx/points.rs::drag_terms`).

**Turbulence** is a displacement, not an integrated force — the standard trick that keeps
it closed-form:

```
Δp = amount · noise3(p0 / scale + phase_b, age · turb_speed)
```

where `noise3` is **three channels of** the same deterministic value-noise family Wiggle
and Fractal noise use — the third added with the depth axis, on the rule that a jitter
with an x and a y gains a z. The lattice is sampled at the birth point's own x and y as
it always was: a third *input* coordinate would move every existing sample and repaint
every project. It follows that on a 2D layer with Turbulence above nought the stream's
`z` is **not** nought and cannot be seen, because the flat projection drops it — the
guarantee to old projects is about the picture, which is what the tests hold. `noise3`
(one lattice, pinned by the test plan — do not invent a second noise), and `phase_b` is
the particle's hashed phase. The drawn position is `p(age) + Δp`.

The **speed** written to the points stream is the analytic `v(age)` plus the turbulence
derivative by central difference at a fixed `ε = Δt/2` — fixed, so one frame key names
one picture regardless of preview raster or refresh.

**Force parameters and animation.** The closed forms treat `g`, `w`, `k` as constants
over a particle's life, **sampled at the current frame** — when the user keyframes
gravity, every live particle's whole trajectory re-solves under the new value. That is
physically wrong (the true answer integrates the changing force) and visually right (the
whole system leans when the keyframe lands, which is what motion designers expect from
retiming a force), and it is what keeps the formula closed. The alternative — integrating
keyframed forces — is precisely the simulation this design excludes, written down so
nobody "fixes" it into statefulness.

**The cap rule** must also be pure per-frame: when live candidates exceed Max particles,
keep the **newest** `cap` by birth index. Old particles vanish early under overload —
visible, deterministic, and identical from any scrub direction.

### 3.3 What the cap rule and the schedule together guarantee

For any frame, the drawn set is a pure function of (parameters at that frame, the rate
curve's history, seed, layer time). Two evaluations agree bit for bit; evaluation order
across frames is irrelevant; there is nothing to invalidate on a scrub because there is
nothing retained.

## 4. The points output and the driver graph

Three decisions, each held against the driver wiring model:

- **Forces are parameters, not nodes.** A force is a per-particle field, and the model's
  drivers produce *scalar/colour values* evaluated as pure CPU work at resolve time — a
  force-node species would need per-particle evaluation inside the driver pass, a new
  contract, and Points-typed wires between driver nodes, which the model does not have.
  As parameters, every force is keyframable, expression-readable and **drivable** through
  the existing machinery: Audio level → Emit rate is one wire, and "sparks burst on the
  beat" needs no new node kind.
- **Emission is parameters too** — the emitter's position, rate, direction and spread are
  ordinary drivable rows. There is no Points *input* in v1; nothing upstream produces one.
- **The Points output is a declared port on the stack node**: Particulate's
  registry entry declares it, and the Graph panel draws it type-coloured (teal, the
  geometry group). This note first shipped it drawn-but-unwirable; the family design
  step has since decided how it is consumed — **a graph wire**, the `EffectData` edge,
  whose first consumer is the Points sample driver.
  [points-stream.md](points-stream.md) §1–§2 carries the rules; the later stack-effect
  family (Connect points, Clone to points, Trail…) consumes the same edge under the
  downstream-only rule recorded there.

**Registry shape.** node-graph.md §1.3's `Signature::Image` grows an optional extra-output
list — `Image { extra: &'static [(&'static str, PortType)] }`, empty for every existing
effect — so a stack effect can declare data outputs without becoming a driver. This is an
implementation-time detail of the registry, not a new decision; the port itself is
already decided.

### The `PointsStream` layout, finalised

node-graph.md §6.2's shape was the starting point; WP6 finalises it with **one addition**,
`life`, so consumers can compute normalised age without re-deriving per-particle jitter:

```rust
/// One frame's particles, structure-of-arrays, GPU-resident, never in the document.
struct PointsStream {
    count: u32,                 // live particles this frame (≤ the declared cap)
    position: Buffer<Vec3>,     // px@comp, the layer's three axes, UNPROJECTED
    speed: Buffer<Vec3>,        // px@comp per second
    age: Buffer<f32>,           // seconds since birth
    life: Buffer<f32>,          // this particle's total lifetime, seconds  ← added
    size: Buffer<f32>,          // px@comp, after Size over life
    rotation: Buffer<f32>,      // radians, after Spin and Align to motion
    colour: Buffer<[f16; 4]>,   // premultiplied, working space, after over-life blends
    id: Buffer<u64>,            // the birth index — stable across frames; what makes trails possible
}
```

`id` **is** the birth index: no separate id space, and stability across frames falls out
of §3.1 for free.

**Positions and speeds are unprojected**: the layer's own three axes. Where the
composition's camera puts a particle is one small 3×4 table carried beside the stream and
applied at the *read* — `projected(i)` for a consumer that thinks in 2D, `position[i]` for
one that declared `Port::three_d`. points-stream.md §3.1 is the contract. On the card the
stream is 17 words a particle: position 3, speed 3, age, life, size, rotation, colour 2
(half pairs), id 2, and the draw's own tail 3.

## 5. Determinism and caching

- **Frame key**: the standard formula (docs/06 §5.2) with no new terms — parameters hash
  as usual, and `seeded` already folds the layer's local time in (docs/08 §1.3). Temporal
  window `{0}` means the prefetcher plans nothing.
- **Export equals preview** by construction: no state, no wall clock, no
  evaluation-order dependence. The GPU path's only reduction is the live-particle
  compaction, done by a deterministic prefix sum in birth-index order — never atomics
  racing for slots, which would make `id` order a scheduling artefact.
- **CPU/GPU agreement**: `moderate` class, so the §1.6 perceptual epsilon governs the
  *pixels*; the *points stream* values themselves (position, age, …) must agree to
  **10⁻⁵ of each attribute's own range** over the frame's live set (corrected from
  this note's original ≤ 2 ULP), because downstream consumers will read them as data, and
  data has no perceptual tolerance. Two ULP is not a bound a GPU can meet against libm —
  `sin`, `cos` and `exp` are a part in 10⁶ before a speed multiplies them — and half these
  quantities pass through zero, where a ULP count means nothing at all. `id` is exempt and
  exact (it is the birth index, not a measurement); colour is exempt because §4 declares
  that region at half precision.

## 6. GPU/CPU split under the pipeline

An ordinary stack effect in docs/06 §1.2's per-layer order. Per frame:

1. **Schedule** (CPU, cached): the birth-schedule scan (§3.1) and the candidate range for
   this frame. Scalar work, microseconds.
2. **Evaluate** (GPU compute): one thread per candidate computes aliveness and the closed
   forms, writes the SoA attributes; a prefix-sum compaction packs live particles and
   sets `count`. This *is* the points stream — the render pass and future consumers read
   the same buffers.
3. **Draw** (GPU raster, instanced): one quad per live particle — feathered disc, sprite
   texture (the referenced layer's texture, threaded exactly as a matte's is), or streak
   segment — blended over the input; the host's Blend/Mix seam applies as everywhere.
4. **CPU reference**: the same maths into a `Vec`, software dabs for the draw —
   the oracle and the ladder's fallback rung. Same cap, same schedule, just slower.

Buffers come from the governor's pools, declared as a function of the cap — the cap is
*the* peak-scratch declaration (docs/13 §6), which is why it is a parameter and not a
guess.

## 7. Budget and degradation

Particulate is a **playback-class** effect, not a member of the physical flare's
owner-set ~2 s-a-frame simulation class: a particle system that cannot play back
in real time is not a montage tool. Mirroring the flare's rule — *the budget is the
user's dial* — the dial here is **Max particles**:

- **Defaults must play**: the default parameter set (≈300 live particles) costs ≲ 0.2 ms
  GPU on the reference desktop — noise against B6.
- **The default cap must play**: 20 000 live particles in Disc mode ≤ 1 ms GPU on the
  reference desktop, ≤ 4 ms on the reference laptop.
- **The hard cap must not hang**: 1 000 000 live particles must clear evaluate + draw in
  ≤ 16 ms on the reference desktop — one comp frame, degraded playback rather than a
  stall — and check cancellation between the evaluate and draw passes.
- **Degradation rung**: under governor pressure or a missed budget, the effect draws the
  newest `count/2` particles (halving again as pressure demands) — deterministic, the
  same rule as the cap, interaction-only, shown in the status readout like every ladder
  step, and **never on export** (docs/06 §6.2's standing rule). It slots beside ladder
  step 5 (the flow→blend swap) as an effect-declared cheapening.
- CPU fallback (ladder step 6) renders the same particles at the same cap — slower is
  allowed, different is not.
- **The candidate ceiling is a dispatch limit, not a memory budget.** Max particles bounds
  what is *drawn*; what is *evaluated* is the candidate set — every birth in the window
  `[t − max_life, t]` — and Emit rate and Life are both open-ended rows, so that set needs
  a ceiling of its own. The schedule is trimmed to it, dropping the **oldest** candidates,
  which changes nothing the cap rule would not already have dropped (§3.2). The number is
  `MAX_CANDIDATES` in `lumit-gpu`, and it is derived rather than chosen: the evaluate pass
  dispatches one workgroup per 64 candidates against a
  `max_compute_workgroups_per_dimension` of 65 535, so the ceiling is 65 535 × 64 =
  4 194 240. It was first set at 8 000 000 as a memory budget alone, which asks the device
  for 125 000 workgroups — a validation error that invalidates the encoder and takes the
  frame's draw down with it, reached by typing a large Emit rate. Any future change to the
  workgroup width moves the ceiling with it; the two constants sit together for that reason.

These numbers become CI gates in the perf harness when the effect lands (docs/13 §7.3),
per the verification-beats-assertion rule.

## 8. The simulated exception — specified, not built

If a future need is real (collisions with layer alpha, flocking), it lands as a separate
**Simulate** mode with this contract, and only with an appended decision:

- **Fixed-step**: state steps at comp frames from the layer in point, one fixed
  integration order, seeded — deterministic, but sequential.
- **The sim cache**: frame states cached keyed by `(hash of every sim-affecting
  parameter + seed + in point, frame index)`. Any sim-affecting edit invalidates the
  whole run — no partial invalidation, because frame N's state embeds every earlier
  frame's parameters. Non-sim parameters (colour, render mode) stay out of the sim hash
  so a recolour does not re-simulate.
- **Cold scrub**: scrubbing to an uncached frame steps forward from the newest cached
  ancestor, cancellable per step, with the standard progress affordance; playback waits,
  the UI never does.
- **Export** always steps from frame 0 of the run; the sim cache is a preview
  convenience, never an export input (preview equals export survives because stepping
  is itself deterministic).

Everything else in this note — closed-form default, the points layout, the cap, the
render modes — is unchanged by the exception existing. Its absence from v1 is deliberate.

## 9. Test plan

Landed with the implementation. lumit-core unless noted.

1. **Determinism**: one comp, two evaluations of the same frame — pixels bit-identical,
   points stream bit-identical. Export-equals-preview on a Particulate fixture joins the
   standing gate.
2. **Random access**: evaluate frames `{500, 3, 250, 3}` in that order; each equals the
   same frame evaluated in ascending order. The scrub-safety property, as a test.
3. **Birth schedule**: against a closed-form count for constant rate (`floor(rate·t)`
   ± the carry) and a hand-computed table for a keyframed ramp; schedule cache hit
   equals cold computation.
4. **Closed forms**: position/speed against the analytic solutions at `k = 0`,
   `k = 0.5`, and across the series guard boundary; wind-with-zero-drag is exactly
   motionless wind (the documented behaviour).
5. **Turbulence pinning**: fixed seed, fixed lattice — golden values for `noise3` at
   pinned sample points, so no one swaps the noise family silently.
6. **Id stability**: a particle's `id`, sampled at three frames of its life, is constant;
   ids are strictly increasing in birth order after compaction (the prefix-sum
   determinism test).
7. **Cap rule**: over-budget fixture — live set is exactly the newest `cap` by birth
   index; halving degradation produces the newest `cap/2`; degradation never active on
   the export path (render test).
8. **CPU/GPU twins**: pixels within the `moderate` perceptual epsilon; points-stream
   attributes within 10⁻⁵ of their own range (§5's stricter data bound).
9. **Mask-path emitter**: empty polyline (no masks, deleted mask, nothing named) emits
   nothing and the effect passes its input through — the documented no-op.
10. **Sprite fallback**: Sprite mode with an unset layer draws discs, not nothing.
11. **Frame-key sensitivity**: seed change ⇒ key changes; scrubbing time ⇒ key changes
    (seeded rule); an edit to an unrelated layer ⇒ key stable.
12. **Budgets** (perf harness, docs/13 §7.3): the four numbers of §7 as gates on the
    reference-desktop runner. Measured **flat**, deliberately: B12–B14 are the numbers
    docs/13 §2 states against a checked-in baseline, and the third axis costs one dot
    product and one divide per particle in the vertex stage — inside the noise of a row
    already reported to three decimal places, so it earns no row of its own.
14. **Border emission.** *On the ring, not in it*: every particle from an
    Ellipse outline satisfies the ellipse's own equation, and every one from a
    Rectangle outline sits on a side. *Uniformly along the perimeter*: eight equal
    lengths of arc take an eighth of the particles each — a walk parameterised by
    angle instead would crowd the ends of a 2:1 ellipse's long axis by a factor of
    two. *The two paths walk one polyline*: the flattening is one function the CPU
    reference and the GPU host both call, held as a stream twin. *Nothing moved*:
    the five codes a saved document can name mean exactly what they meant,
    which is what appending the two new ones buys. *A degenerate outline* emits at
    the emitter's centre and never faults.

13. **The third axis.** *The flat projection is the identity*, bit for bit, at any
    `z` — the arithmetic the whole 2D guarantee rests on, and the one bit it does not
    carry (the sign of a zero) named rather than skipped. *The camera puts depth where
    perspective puts it*: the plane unmoved, a particle further off drawn smaller and
    nearer the centre, one at or behind the camera drawn not at all. *A projected particle
    lands where the camera would have put it* — the restriction and its inverse held
    against the compositor's own matrices (`lumit-render`, `points_projection_tests`).
    *The old-file gate*: an instance with the five depth rows absent altogether — which is
    what an old file loads as — evaluates to the identical stream and draws the identical
    picture. *The depth rows do something*, each one separately, turbulence's third
    lattice included. *Determinism, random access and the cap rule hold in 3D*: items 1, 2
    and 7 re-run on a stream that is off the plane and seen through a camera. *The GPU
    twins in 3D*: item 8 again with a projection, and the drawn picture in all three modes
    — the streak's tail taking the same camera as its head. *End to end*: a 3D layer under
    a comp camera draws a different field from the same layer flat, and a 2D layer's
    picture is byte-identical whether the comp has a camera or not.

## Open questions

Two of this note's original questions are resolved by the family design step
([points-stream.md](points-stream.md)): **how the family consumes the stream** — a graph
wire, the `EffectData` edge — and **Vec2 vs Vec3** — positions are `Vec3`, and on
a 3D layer the composition's camera sees them. What stays open:

- **Whether the simulated exception is ever needed.** §8 is its contract; building it
  waits on an owner ruling driven by real demand (collisions with layer alpha is the
  likeliest first ask).
- **Emitting from the image** (births weighted by the input's luminance) — wants an
  analysis pass over the input and a different birth-schedule story; a named package in
  points-stream.md §2.3, which also records its constraint on driver-side sampling of an
  image-dependent stream, since Scatter wants the identical machinery.
