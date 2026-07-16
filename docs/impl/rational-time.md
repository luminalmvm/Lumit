# Rational time: exact arithmetic without overflow

Authoritative time in Luminal is rational ([14-ENGINEERING-RULES.md](../14-ENGINEERING-RULES.md)).
This note pins down the arithmetic, because naive rational code fails in exactly two ways:
silent i64 overflow when denominators multiply, and non-canonical forms breaking equality
and cache hashes. Both have shipped bugs in real NLEs.

## 1. Representation

```rust
/// Always normalised: den > 0, gcd(|num|, den) == 1, and 0 is (0, 1).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational { num: i64, den: i64 }
```

- `den` is i64, not u32: intermediate maths is cleaner with one signed type, and the
  normalisation invariant (den > 0) carries the sign discipline.
- **Construction is the only place normalisation happens**, and every arithmetic op ends in
  construction. Private fields; `Rational::new(num, den)` normalises (divide both by gcd,
  move sign to num, panic-free error on den == 0 at the typed-error boundary).
- `PartialEq`/`Hash` derive correctly **only because** of the canonical-form invariant.
  This is what makes rational times usable in cache keys (K-016): equal times hash equal.

## 2. The overflow problem, and the rule that solves it

`a/b + c/d = (ad + cb) / bd`. With 48 kHz audio (den 48000), NTSC rates (den 1001), flicks
and user-entered times all mixing, `bd` exceeds i64 after a handful of chained ops if you
never reduce. The rule:

**Do every intermediate multiplication in i128, reduce, then check the result fits i64.**

```rust
fn add(a: Rational, b: Rational) -> Rational {
    // lcm-based: keeps intermediates smaller than the naive ad+cb/bd
    let g = gcd(a.den, b.den);                       // i64
    let l = a.den / g;                               // a.den = g*l
    let num = (a.num as i128) * (b.den / g) as i128
            + (b.num as i128) * l as i128;
    let den = (a.den as i128) * (b.den / g) as i128; // = lcm(a.den, b.den)
    Rational::from_i128(num, den)                    // reduce in i128, then try_into i64
}
```

`from_i128` reduces by the i128 gcd first, then converts; if it still does not fit,
that is a **typed error surfaced to the caller**, not a panic — but see §3: with the domain
rules below it is unreachable in practice, so the internal API may also offer an
infallible variant that saturates to the flick grid with a debug assertion.

Multiplication: cross-reduce **before** multiplying (`gcd(a.num, b.den)`,
`gcd(b.num, a.den)`), then multiply in i128. Comparison: never `a.num * b.den <
b.num * a.den` in i64 — do it in i128 (two multiplications always fit: |i64|·|i64| < i128).

## 3. Domain rules that keep denominators tame

- Media timestamps enter as `pts × timebase` — already rational with the container's
  timebase denominator (90000, 48000, 1001-family). Keep them; do not "simplify" to floats
  ever, or to a fixed grid at import.
- User-interactive edits (dragging a boundary, razor at playhead) snap to either the comp's
  frame grid (`n / frame_rate`) or, for sub-frame features, the **flick grid**
  (1 flick = 1/705,600,000 s; divides every common rate: 24, 25, 30, 48, 60, 90, 120,
  and the /1.001 rates when paired with num scaling, plus 44.1/48/96 kHz audio).
- Derived values that would otherwise grow (retime conversions in
  [04-RETIMING.md](../04-RETIMING.md), which involve E(u) with denominators 2, 3, 6) stay
  bounded because inputs are grid-aligned and the ease integrals have tiny fixed
  denominators. The one documented exception — splitting an AE-imported free-influence
  MapSegment — **rounds to the flick grid by spec** (04 §cutting), which is precisely the
  release valve that keeps the invariant "boundaries are exact and small".

Property test to enforce this: generate 10⁶ random edit sequences from grid-aligned inputs;
assert every stored boundary's `den` stays below 2⁴⁰.

## 4. Conversion to f64 (evaluation only)

Per-sample evaluation is f64 by spec. Convert as `num as f64 / den as f64` — both exact up
to 2⁵³, and den stays far below that (§3). Never convert f64 → Rational except through an
explicit grid-quantisation function (`Rational::from_f64_on_grid(x, grid_den)`), which
rounds half-to-even on the grid. There must be **no** general `From<f64>`.

## 5. Timebase newtypes

```rust
macro_rules! timebase { ($T:ident) => {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct $T(pub Rational);
    // arithmetic within a timebase: T ± Duration -> T, T - T -> Duration
}}
timebase!(SourceTime); timebase!(ClipTime); timebase!(LayerTime); timebase!(CompTime);
pub struct Duration(pub Rational); // shared, unsigned-by-convention
```

Cross-timebase conversion functions live in one module (`luminal-core::time::convert`) and
nowhere else: `comp_to_layer(t, layer) -> Option<LayerTime>` (None outside in/out),
`layer_to_source` goes **through the Retime evaluator only**. No `Deref` to Rational —
the friction is the feature.

Ordering derives are safe **only** because PartialOrd on Rational is implemented via the
i128 cross-multiply, not the derived lexicographic one — write it manually, test it.

## 6. Test plan

1. Property tests (proptest): for random rationals within domain bounds — associativity of
   add within exactness, `a + b - b == a`, comparison total order agrees with f64 order
   when both are exact, canonical form after every op (`gcd == 1`, `den > 0`).
2. Overflow adversarials: 90000-den pts plus 1/705600000 flicks chained 10⁵ times; NTSC
   `30000/1001` frame walks over 3 hours of comp time.
3. Hash/equality: `1/2 == 2/4` after construction; HashMap round-trip.
4. Grid: `from_f64_on_grid` round-trips every frame index at 23.976/29.97/59.94 over
   3 hours exactly.
5. Fuzz `Rational::new` and the convert module (cargo-fuzz) — no panics, only typed errors.
