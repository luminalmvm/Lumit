# Keyframe and Retime evaluation: the cubic solving that must not be fudged

Two places in Lumit evaluate x-monotone parametric cubics: property keyframes
([03-DATA-MODEL.md](../03-DATA-MODEL.md) §6.2) and Retime MapSegments
([04-RETIMING.md](../04-RETIMING.md) §4.2). Both look like "just a bezier" and both have a
trap: the curve is parametric in u, but you are asked for value-at-**time**, so every
evaluation is a root-solve `x(u) = t`. Done sloppily (fixed-iteration Newton, no bracketing)
this produces the exact class of bug users describe as "AE's graph editor feels wrong":
values that jitter near steep handles, non-monotone output from monotone-looking curves.

## 1. The curve, from AE parameters

Between keys `(t1, v1)` and `(t2, v2)`, with out-side (speed s1, influence b1 ∈ (0,1]) and
in-side (s2, b2), Δt = t2 − t1:

```
x(u) = t1 + Δt·( 3b1·u(1−u)² + (3 − 3b2)·u²(1−u)·… )        — expand as standard bezier:
P0 = (t1, v1)
P1 = (t1 + b1·Δt,  v1 + s1·b1·Δt)
P2 = (t2 − b2·Δt,  v2 − s2·b2·Δt)
P3 = (t2, v2)
x(u) = B(u; P0.x..P3.x),  v(u) = B(u; P0.y..P3.y),  B = cubic Bernstein
```

x-monotonicity holds because b1, b2 ∈ (0,1] keeps P1.x, P2.x inside [t1, t2] — **validate
at construction** (import clamps AE's 0.1–100% influence into this range; equality at the
ends is legal and makes x'(u) = 0 at an endpoint, see §2 trap).

## 2. Solving x(u) = t: Newton inside a shrinking bracket

Binding algorithm (do not substitute plain Newton or plain bisection):

```rust
fn solve_u(t: f64, x: &CubicX) -> f64 {          // x normalised to [0,1] domain
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let mut u = t;                                // good initial guess: x ≈ identity
    for _ in 0..16 {
        let (xu, dxu) = x.eval_with_deriv(u);
        if xu < t { lo = u } else { hi = u }      // maintain bracket (x is monotone)
        let newton = u - (xu - t) / dxu;
        u = if dxu > 1e-12 && newton > lo && newton < hi
            { newton }                            // Newton step, only if it stays inside
            else { 0.5 * (lo + hi) };             // else bisect
        if (xu - t).abs() < 1e-12 { break }
    }
    u
}
```

- The bracket update relies on monotone x — which §1 guarantees. Newton alone diverges
  when `dxu → 0`, which **legitimately happens** at endpoints with influence 1.0 (AE's
  100%: the "spike" case). The bisection fallback makes those exact, not explosive.
- 16 iterations of bracketed Newton reaches < 1e-12 for every representable input; do not
  early-out on iteration count without the residual check in tests.
- Cache per (segment, frame) is unnecessary — this is ~50 flops; do not complicate.

Hold: value is v1 over [t1, t2). Linear: lerp. Mixed sides (bezier-out into hold-in etc.)
follow AE: each *pair* of adjacent sides defines the span curve; hold-out wins the span.

## 3. Spatial properties and roving

Spatial values (Vec2/Vec3 position) use **two** curves: the spatial path (bezier in value
space with the stored tangents) and the temporal curve above applied to **arc length**.
Implementation: at edit time, arc-length-parameterise the spatial span (Gauss–Legendre
16-point per span, cached table of 64 cumulative samples, invalidated with the span);
evaluation maps temporal output → arc fraction → path point via the table + one Newton
refine. Roving keyframes: on any neighbour edit, redistribute the roving keys' times so
cumulative arc length is proportional to time across the roving run — solve on the same
tables, then **write the times back as grid-quantised rationals**
([rational-time.md](rational-time.md) §4).

## 4. Retime segments

RateSegments: closed-form — evaluate `f(t)` directly from the E(u) table in
[04-RETIMING.md](../04-RETIMING.md) §4.1 in f64; boundary source positions come from the
stored rationals, never re-integrated at render time. MapSegments: exactly §2 above with
(t, s) in place of (t, v).

Inversion (source → local time, needed for "which local times show source frame N" in the
overrun UI and flow prefetch): only defined where monotone; implement as the same bracketed
solve against the inverse relation per segment, walking segments in order. For freezes
(speed 0 spans) return the span start by convention and document it in the caller.

Splitting a MapSegment at local time tc (razor): de Casteljau at the solved u — both halves
get the subdivided control points, converted back to (speed, influence) form:
`b = (P1.x − P0.x)/Δt`, `s = (P1.y − P0.y)/(P1.x − P0.x)` per side (guard the b → 0
degenerate: falls back to linear side). Native (polynomial, b = ⅓) segments split exactly
in rationals; free-influence ones round the new boundary to the flick grid **by spec**.

## 5. Test plan

1. Golden values against AE: export a comp from AE (Bridge JSON) containing every
   interpolation combination (linear/bezier/hold × influences 0.1%, 33.33%, 100%, easy
   ease, spikes) and assert Lumit's sampled values match AE's rendered motion within
   1e-4 of value range at every frame. This one test kills the whole class of "feels off".
2. Property tests: solve_u(x(u)) == u to 1e-10 over random monotone cubics including
   dx = 0 endpoints; monotone in, monotone out.
3. Roving: three-key path with middle roving — equal speed segments to 1e-6; times remain
   grid-rational after redistribution.
4. Split: razor at 1000 random points → piecewise evaluation of halves equals original to
   1e-12; boundary s values exact for native segments.
5. Bench gate: 10⁶ scalar evaluations < 20 ms on the reference CPU (it is ~50 flops; if
   this fails something is allocating).
6. Tangent modes: the six sentences in §6 below, asserted on both ports of the evaluator
   (`crates/lumit-core/src/anim.rs`, `flutter_ui/lib/panels/graph_maths.dart`) against the
   same hand-computed numbers, so the two cannot drift.

## 6. Tangent modes: Auto, Clamp and Free

The graph's tangent strip (docs/impl/timeline-interaction.md §6.3) offers three modes per
**key side**. Free is everything above: the side stores a speed and an influence, and they
are what the curve uses. Auto and Clamp store the influence but **compute the speed from
the key's neighbours on every read**, so the shape stays smooth as the curve is edited
around it.

**In plain terms.** An automatic tangent points the way the curve is already going. The
plain one aims straight from the key before to the key after. The clamped one is the same
aim with the swing taken out of it: where the key is a peak or a trough it lies flat,
because any tilt would send the curve past a neighbour on one side or the other, and
elsewhere it is held to a slope the span cannot overshoot with.

### 6.1 The arithmetic (binding)

For key `i` with neighbours `i−1` and `i+1`, times `tp < tk < tn` and values `vp, vk, vn`:

```
smooth  = (vn − vp) / (tn − tp)                    # Catmull-Rom, non-uniform
before  = (vk − vp) / (tk − tp)
after   = (vn − vk) / (tn − tk)

Auto  :  smooth
Clamp :  0                                   if before·after ≤ 0     (peak or trough)
         clamp(smooth, ±3·min(|before|,|after|))  otherwise
```

- The **±3·min** bound is the Fritsch–Carlson condition for a monotone cubic: inside it a
  span whose two ends are both within the bound cannot leave the box its keys make, which
  is exactly "no overshoot". Do not substitute a smaller constant to be safe — 3 is the
  tight bound, and anything less flattens curves that were never going to overshoot.
- **An end key's automatic tangent is 0.** It has no pair to aim between, so the curve
  arrives and leaves level, which is what an automatic ease means there.
- Clamp is defined for the *value* being clamped, not the tangent's length: the influence
  is untouched. An automatic tangent decides which way the handle points; it does not
  lengthen it.
- Resolution happens at **read**, never at write: `resolved_side(keys, i, out)` turns an
  automatic side into the bezier its neighbours dictate, and every reader — the evaluator,
  the speed lens, the handle geometry, the range fit — goes through it. Nothing recomputes
  a stored value, so there is no "recompute after every edit" hook to forget.

### 6.2 Where the custom ease lives (binding)

The mode is stored **inside the side**, as a fourth `SideInterp` arm:

```rust
SideInterp::Auto { clamped: bool, speed: f64, influence: f64 }
```

`speed` and `influence` there are **not evaluated**. They are the ease the side carried
when it was last Free, and they are what makes the study's bar — *switching Free → Auto →
Free keeps the custom ease* — true without any merge step: the memory travels inside the
thing that owns it, so a key list can cross the bridge, be rebuilt by the interface and
come back, and the ease is still there. The alternative designs both cost more: a separate
mode field beside the side needs the write path to consult what was there before (and
every write path to remember to), and recomputing-and-storing needs a second pair of
fields to remember the ease with anyway.

Two consequences, both deliberate:

- **A side that was straight or held returns from Auto as an easy ease** (speed 0,
  influence ⅓). It had no ease of its own to keep, and it has had a tangent all the time
  it was automatic; handing back a straight side would take the handle away under the
  user's hand.
- **Shaping a handle takes its side back to Free.** The handle drag, the influence wells
  and the ease presets all write a plain bezier side, which is a Free side by definition.
  That is the honest answer to "the neighbours choose this tangent" meeting "no, *this*
  is the tangent", and it needs no extra rule: it falls out of what the modes are.
