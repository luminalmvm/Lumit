# Keyframe and Retime evaluation: the cubic solving that must not be fudged

Two places in Luminal evaluate x-monotone parametric cubics: property keyframes
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
   ease, spikes) and assert Luminal's sampled values match AE's rendered motion within
   1e-4 of value range at every frame. This one test kills the whole class of "feels off".
2. Property tests: solve_u(x(u)) == u to 1e-10 over random monotone cubics including
   dx = 0 endpoints; monotone in, monotone out.
3. Roving: three-key path with middle roving — equal speed segments to 1e-6; times remain
   grid-rational after redistribution.
4. Split: razor at 1000 random points → piecewise evaluation of halves equals original to
   1e-12; boundary s values exact for native segments.
5. Bench gate: 10⁶ scalar evaluations < 20 ms on the reference CPU (it is ~50 flops; if
   this fails something is allocating).
