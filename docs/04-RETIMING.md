# Retime

**Status: canonical.** This document specifies Retime, Luminal's single retiming system
(K-021), end to end: the segment data model and its exact maths, the two graph-editor lenses,
the beat-sync covenant (K-022), cutting behaviour, overrun, frame interpolation policy, and
how Retime composes with the rest of the engine. Terminology follows
[01-GLOSSARY.md](01-GLOSSARY.md) §4 exactly. Object ownership (where a Retime lives on a Clip
or Footage layer) is defined in [03-DATA-MODEL.md](03-DATA-MODEL.md) and is referenced, not
redefined, here.

The key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are to be interpreted as described
in RFC 2119.

---

## 1. Purpose and design principles

Retime is the feature Luminal is built around. The target editor (K-002) spends most of a
montage session ramping clips against beats. Every design choice below traces to five
principles:

1. **One system, two lenses.** There is exactly one retime store per clip or per Footage
   layer. The value graph (AE-style map from local time to source time) and the speed graph
   (Vegas-style rate semantics) are views of that one store, edited in the graph editor.
   The speed view is drawn in the graph editor pane, below or instead of the value view —
   it is NEVER overlaid on the clip in the timeline (K-021).
2. **The beat-sync covenant.** Editing a retime curve MUST NOT move clip boundaries, edit
   points, or layer in/out points, and MUST NOT ripple anything (K-022). A ramped clip is a
   stable rectangle; beats already synced stay synced.
3. **Exact boundaries.** Every segment boundary stores the exact rational source position.
   Cutting, trimming, and repeated editing never accumulate floating-point drift: the frame
   that lands on a beat is the frame that stays on the beat.
4. **Hard cuts are a feature.** Montage editing wants instantaneous rate changes. Continuity
   of source position (C0) is enforced; continuity of speed (C1) is optional and its absence
   is shown, never "fixed" automatically.
5. **The map is not the pixels.** How fractional source positions become frames (nearest,
   blend, flow) is a per-clip render policy, orthogonal to the retime map (§10).

## 2. Domain and timebases

A Retime is a function *f* from **local time** to **source time**:

- On a **clip** (Sequence layer): local time is clip time; source time is seconds in the
  clip's source (footage item or comp).
- On a **Footage layer**: local time is layer time; source time is seconds in the footage
  item (after interpretation, e.g. frame-rate override).
- On a **Precomp layer**: local time is layer time; source time is the nested comp's comp
  time. Precomp Retime follows this spec identically (required for AE import fidelity, §13.1).

*f* MUST be defined over the entire local domain [0, D], where D is the clip duration or
layer duration. Speed is d*f*/dt: 1.0 = normal, 0 = freeze, negative = reverse. The UI shows
speed as a percentage.

All boundary times and source positions are exact rationals (`Rational { num: i64, den: u64 }`,
always normalised — number policy in [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)
§time-types). Per-sample evaluation during rendering is f64; the rational store exists so that
*edits and cuts* are exact, and so that identical inputs always hash identically for the cache
(§11.7).

## 3. Data model

```rust
/// One retime store. Owned by a Clip or by a Footage/Precomp layer (03-DATA-MODEL.md).
pub struct Retime {
    /// n + 1 boundaries for n segments. boundaries[0].t == 0, boundaries[n].t == D.
    /// Strictly increasing in t. Authoritative for evaluation and cutting.
    pub boundaries: Vec<Boundary>,
    /// n segments; segments[i] spans boundaries[i] .. boundaries[i + 1].
    pub segments: Vec<RetimeSegment>,
    /// Reverse gate (§6.2). Default false.
    pub allow_reverse: bool,
    /// Frame interpolation policy (§10). Default Nearest.
    pub interpolation: Interpolation,
}

pub struct Boundary {
    pub t: Rational,      // local time
    pub s: Rational,      // source time — exact, shared by both adjacent segments (C0)
    pub smooth: bool,     // C1 maintenance flag (§6.1). Default false.
}

pub enum RetimeSegment {
    Rate(RateSegment),    // speed-native: constant or eased speed
    Map(MapSegment),      // value-native: cubic source-time curve
}

/// Speed-defined segment. Source advance is a closed-form integral (§4.1).
pub struct RateSegment {
    pub v0: Rational,     // speed at segment start (1 = 100%)
    pub v1: Rational,     // speed at segment end
    pub ease: Ease,       // shape of the speed transition (§4.1)
}

pub enum Ease { Linear, Slow, Fast, Smooth, Sharp }

/// Value-defined segment: x-monotone parametric cubic bezier in (t, s),
/// AE-compatible per K-025 (§4.2). Endpoint values come from the boundaries.
pub struct MapSegment {
    pub m0: Rational,     // outgoing speed (source seconds per local second) at start
    pub m1: Rational,     // incoming speed at end
    pub b0: Rational,     // outgoing influence, (0, 1]
    pub b1: Rational,     // incoming influence, (0, 1]
}

pub enum Interpolation { Nearest, Blend, Flow(FlowParams) }
```

Invariants (all MUST):

- Boundaries are strictly increasing in `t`; first at 0, last at D.
- C0: each interior boundary's `s` is the single source position shared by both neighbouring
  segments. There is no way to represent a source-position jump within one clip — a jump is
  a cut (§8.1).
- For every `RateSegment`, the boundary consistency equation of §4.1 holds exactly; edits
  recompute downstream boundary `s` values transactionally before the edit commits.
- If `allow_reverse` is false, *f* is non-decreasing: RateSegment speeds are clamped ≥ 0 and
  MapSegments satisfy the monotonicity conditions of §6.2.
- `s` values are unclamped: the map may run past the source extent. Clamping to the available
  source happens at evaluation and is what defines overrun (§7).
- A freeze is representable in either primitive: `Rate { v0: 0, v1: 0, ease: Linear }` or a
  Map with `m0 = m1 = 0` and equal boundary `s`. "Hold" in the preset vocabulary (§12.2)
  produces a constant-speed RateSegment; there is no separate hold ease.

Default state: a new clip or a Footage layer with Retime enabled has one
`Rate { v0: 1, v1: 1, ease: Linear }` segment spanning [0, D] with `s` running from the source
in point. This is exactly "no retiming" and MUST render identically to Retime being absent.

## 4. Segment maths

Within segment *i*, write d = tᵢ₊₁ − tᵢ and u = (t − tᵢ)/d ∈ [0, 1].

### 4.1 RateSegment: integration to source time

The speed profile is v(u) = v₀ + (v₁ − v₀)·e(u), where e is the ease function. Source
position is the integral:

```
f(t) = sᵢ + d · [ v₀·u + (v₁ − v₀)·E(u) ]        where E(u) = ∫₀ᵘ e(w) dw
```

Every ease is polynomial (at most piecewise-quadratic) so E is closed-form and, at rational
u, exactly rational:

| Ease | e(u) | E(u) | E(1) | shape |
|---|---|---|---|---|
| Linear | u | u²/2 | 1/2 | straight speed ramp |
| Slow | u² | u³/3 | 1/3 | lingers at v₀, late transition |
| Fast | 2u − u² | u² − u³/3 | 2/3 | early transition, settles at v₁ |
| Smooth | 2u² for u ≤ ½; 1 − 2(1−u)² for u > ½ | 2u³/3 for u ≤ ½; u + 2(1−u)³/3 − ½ for u > ½ | 1/2 | S-curve, C1 at u = ½ |
| Sharp | 2u − 2u² for u ≤ ½; 2u² − 2u + 1 for u > ½ | u² − 2u³/3 for u ≤ ½; 2u³/3 − u² + u − 1/6 for u > ½ | 1/2 | inverse S-curve |

These five names are deliberately the Vegas fade-type vocabulary, and Smooth and Sharp are
deliberately **piecewise-quadratic** rather than cubic: a quadratic speed profile integrates
to a cubic source-time curve, which keeps Rate→Map conversion exact (§5.1). All eases are
monotone in u, so v(u) always lies between v₀ and v₁; clamping the endpoints is sufficient
to clamp the whole profile.

**Boundary consistency (MUST hold exactly):**

```
sᵢ₊₁ = sᵢ + d · [ v₀ + (v₁ − v₀)·E(1) ]
```

Whenever v₀, v₁, ease, or a boundary time changes, this is re-evaluated in rational
arithmetic and the downstream boundary `s` values are updated (edit semantics in §9).

**Precision policy.** Ease coefficients have denominators ≤ 12; speeds entered as percentages
have small denominators; boundary times sit on the comp frame grid (denominators like
60000/1001). Rationals MUST be reduced after every operation. If a numerator or denominator
would overflow i64, the implementation MUST compute in i128 and round the result to the
1/705 600 000 s grid (the flick), emitting a diagnostic; this is a sub-nanosecond rounding and
occurs only under pathological editing.

### 4.2 MapSegment: the cubic value curve

A MapSegment is an x-monotone parametric cubic bezier, exactly AE's temporal keyframe maths
(K-025) so AE import is lossless (§13.1). With endpoints (tᵢ, sᵢ) and (tᵢ₊₁, sᵢ₊₁):

```
P0 = (tᵢ,            sᵢ)
P1 = (tᵢ + b₀·d,     sᵢ + m₀·b₀·d)
P2 = (tᵢ₊₁ − b₁·d,   sᵢ₊₁ − m₁·b₁·d)
P3 = (tᵢ₊₁,          sᵢ₊₁)
x(u), y(u) = cubic beziers over these control points;  f(t) = y(x⁻¹(t))
```

m₀ and m₁ are the endpoint speeds; b₀ and b₁ are AE's influence values as fractions.

- **x-monotonicity (MUST):** x′(u) > 0 on (0, 1). Sufficient condition (used by the editor's
  clamps): b₀ + b₁ ≤ 1 with b₀, b₁ > 0. The exact condition (quadratic minimum of x′) MUST
  be checked when accepting imported data; violations are clamped with a notice.
- **Speed** (for the speed graph and motion blur): v(t) = y′(u) / x′(u), a rational function
  of u.
- **Polynomial subclass.** When b₀ = b₁ = 1/3, x(u) is linear (t = tᵢ + d·u) and f is a plain
  cubic polynomial in u — the cubic Hermite through (sᵢ, sᵢ₊₁) with tangent slopes m₀, m₁.
  This subclass is what conversions (§5.1) and freehand editing produce; general influences
  arise only from AE import and explicit handle edits.

Evaluating a polynomial MapSegment at rational t is exactly rational. Evaluating a
general-influence MapSegment requires inverting x(u), whose root is algebraic; §5.3 defines
the bounded rounding used when such a segment must be split.

### 4.3 Evaluation

Rendering evaluates f in f64: binary-search the boundary list for the segment containing t,
evaluate the closed form (Rate) or solve x(u) = t by Newton-with-bisection-fallback to ≤ 2⁻⁴⁸
relative tolerance (Map), then clamp to the available source domain (§7). The evaluator is a
pure function of the Retime value and MUST be shared between preview and export.

## 5. Conversions between primitives

### 5.1 Rate → Map: exact

Every RateSegment converts to MapSegments in the polynomial subclass with **zero error**,
because every ease integrates to a piecewise cubic:

| Ease | result |
|---|---|
| Linear | one MapSegment: m₀ = v₀, m₁ = v₁, b₀ = b₁ = 1/3 (f is quadratic ⊂ cubic) |
| Slow, Fast | one MapSegment: m₀ = v₀, m₁ = v₁, b₀ = b₁ = 1/3 (f is exactly cubic) |
| Smooth, Sharp | two MapSegments split at u = ½; the interior boundary gets t = tᵢ + d/2 and the exact rational s from §4.1; interior speeds from e(½) = ½: v(½) = (v₀ + v₁)/2 |

The conversion is invisible to rendering (bit-identical curve) and MUST be reported in the UI
only as the segment's type chip changing (§9.4).

### 5.2 Map → Rate: fitted, with warning

A cubic map's speed is quadratic (or rational, with general influence); a RateSegment has two
free speeds plus a fixed ease shape. Conversion is therefore a constrained fit:

1. The boundary source positions are pinned: the fitted segment MUST reproduce Δs = sᵢ₊₁ − sᵢ
   exactly (C0 and the covenant are non-negotiable).
2. For each ease shape, solve for (v₀, v₁) minimising ∫₀¹ (v_fit − v_map)² du subject to
   v₀ + (v₁ − v₀)·E(1) = Δs/d; pick the shape with least residual.
3. Compute the maximum source-position deviation max|f_fit(t) − f_map(t)|. If it exceeds
   **one quarter of a source frame duration**, the conversion MUST attach a visible warning
   badge to the segment ("fitted — up to N ms drift") and the status line MUST state it.
   Conversion is never blocked on accuracy grounds.
4. If the map is non-monotone and `allow_reverse` is false, or the map contains an interior
   speed-sign change (a single RateSegment cannot change sign mid-segment under any ease),
   the conversion MUST be refused with an explanation, offering to split at the stationary
   points first.

Explicitly setting a segment to a **constant speed** (numeric entry, §9.3) is not a fit: it
replaces the segment with `Rate { v, v, Linear }`, recomputes downstream boundaries, and is
exact by construction. No warning.

### 5.3 Splitting a segment at local time t

Needed by the razor (§8.1), boundary insertion (§9), and freeze insertion (§7.3). The new
boundary is (t, f(t)) with `smooth = true` (the split itself introduces no kink):

- **RateSegment, Linear ease:** both halves remain RateSegments — a linear speed profile
  restricted to a sub-interval is linear. Left = {v₀, v(t), Linear}, right = {v(t), v₁,
  Linear}. Exact.
- **RateSegment, other eases:** the restricted speed profile is a general quadratic, which is
  outside the ease vocabulary. Both halves convert to polynomial MapSegments via §5.1
  (split the §5.1 result). Exact; the curve is unchanged.
- **MapSegment, polynomial subclass:** de Casteljau split at u = (t − tᵢ)/d. Exact rational.
- **MapSegment, general influence:** solve x(u) = t numerically, round u to the nearest
  multiple of 2⁻⁴⁸, de Casteljau at the rounded u, then set the new boundary's t exactly and
  s to the computed y rounded to the flick grid. The bounded rounding (< 1.5 ns of source
  time) is recorded permanently in the new boundary; because boundaries are authoritative,
  there is no subsequent drift. This is the only inexact operation in the system and it MUST
  be confined to AE-imported free-influence segments.

### 5.4 Deleting a boundary (merging two segments)

The outer boundaries are pinned (covenant). Default result: one MapSegment through the outer
boundaries with the outer one-sided speeds and influences preserved — the interior detail is
smoothly discarded. Special case: if both neighbours are RateSegments with Linear ease and
the merged segment `Rate { v₀_left, v₁_right, Linear }` reproduces the outer Δs exactly, it
MAY remain a RateSegment. Deletion never moves outer boundaries and never changes segments
beyond the merged pair.

## 6. Continuity and reverse

### 6.1 C0 always, C1 by choice

C0 is structural: one `s` per boundary. C1 is a per-boundary `smooth` flag:

- `smooth = false` (default): the two one-sided speeds at the boundary are independent. When
  they differ by more than 0.1 percentage points, both graph lenses draw a small **kink
  badge** (a diamond at the boundary) — informational, never an error state, because hard
  rate cuts are a montage feature.
- `smooth = true`: edits maintain equal one-sided speeds. Toggling a boundary to smooth sets
  both sides to the arithmetic mean of the current one-sided speeds, adjusting only the two
  adjacent segments (Rate: endpoint speed; Map: tangent m), then keeps them locked together
  under subsequent edits.

### 6.2 Reverse and the monotone clamp

Reverse playback is gated by the per-clip (or per-layer) `allow_reverse` flag, default
**off**. This is the deliberate fix for AE's speed-graph footgun, where influence-handle
edits on the remapping curve silently create backwards-time dips.

With `allow_reverse` off, all editing operations clamp to keep *f* non-decreasing:

- RateSegments: v₀, v₁ clamped to ≥ 0. Since every ease is monotone, this bounds the whole
  profile. Dragging a speed endpoint below the zero line stops at 0 and the zero line shows
  a lock glyph with a tooltip naming the flag.
- MapSegments: boundaries must satisfy sᵢ₊₁ ≥ sᵢ; tangents clamped by the sufficient
  Bernstein conditions m₀ ≥ 0, m₁ ≥ 0, and m₀·b₀ + m₁·b₁ ≤ Δs/d · 1 (for the polynomial
  subclass this is the familiar box: m₀ + m₁ ≤ 3·Δs/d, each in [0, 3·Δs/d]). When a
  requested edit violates the sum condition, both tangents are scaled proportionally.

With `allow_reverse` on, negative speeds are legal in both primitives, the clamps are
removed, and the speed graph's negative region becomes active. Reverse past the source start
clamps and holds the first available frame, visibly, as overrun (§7) — never silently as
Vegas does. Enabling the flag on a clip whose curve is currently monotone changes nothing
until an edit uses it; disabling it while the curve reverses is refused with an offer to
flatten reversing regions to freezes.

## 7. Overrun and the beat-sync covenant

### 7.1 The covenant (K-022, restated as requirements)

- Editing any part of a Retime MUST NOT change the clip's in/out on the Sequence layer, any
  edit point, or the layer's in/out points, and MUST NOT move any other clip or layer.
  There is no auto-ripple and no opt-in ripple on retime edits.
- Consequently a retime edit can leave the map requesting source beyond the trimmed extent
  [src_in, src_out]. This state is **overrun**, it is legal, and it is rendered — never an
  error.

### 7.2 Overrun rendering and indication

At evaluation, the resolved source position is clamped: s_eff = clamp(f(t), src_in, src_out).
Where clamping engages, the boundary frame is held (frozen). Requirements:

- The timeline MUST draw a hatched region on the clip (or layer bar) over the overrun span,
  at both ends where applicable (tail overrun from running out forwards; head overrun from
  reversing past the start). A tick marks the exact exhaustion point.
- Both graph lenses MUST draw the same hatched band over the overrun span, plus a horizontal
  line at s = src_out (and s = src_in when relevant) in the value view so the user can see
  the curve crossing it.
- The overrun span for display is computed to comp-frame precision: for RateSegments by
  closed form (the crossing is a root of a quadratic or cubic with rational coefficients)
  or monotone bisection; for MapSegments by bisection on the bezier. Rendering correctness
  never depends on this solve — clamping is per-sample.
- Overrun frames hold the **boundary frame**, not black, not a loop. Looping is not a retime
  behaviour (it is an interpretation/expression feature, out of scope here).

### 7.3 Freezes and holds at the ends of media

An explicit freeze anywhere is a zero-speed segment (§3). The interaction with media ends:

- A tail overrun is visually distinguished from an authored freeze: authored freezes draw as
  flat curve regions with no hatching; overrun draws hatched. Same pixels, different intent,
  different indication.
- **Insert freeze at playhead** (`retime.freeze_at_playhead`): splits at the playhead (§5.3),
  inserts `Rate { 0, 0, Linear }` of a default 1 s duration (draggable afterwards), shifts
  all downstream boundaries later in local time by the inserted duration, and crops the map
  at the clip out point (segments pushed wholly past D are removed; the segment straddling D
  is truncated by splitting at D). Boundaries of the clip itself do not move; the tail may
  newly overrun, which is indicated as above. The operation is one undo step.
- Extending a clip or layer **out point** (a boundary edit, not a retime edit) extends the
  map by appending a constant-speed RateSegment at the final one-sided speed (so a tail
  freeze stays frozen, a moving tail keeps moving). Extending the **in point** earlier is
  the mirror case with the initial one-sided speed. Trimming inward crops the map exactly
  (§8.2).

### 7.4 Trim to source end

`retime.trim_to_source_end` — MUST be bindable to a single key (proposed default: `E` with a
clip or retime selection; final bindings in [07-UI-SPEC.md](07-UI-SPEC.md)) — performs a
**non-ripple** out-point trim to the last local frame whose resolved source position is
within the source extent. On a Sequence layer this leaves a gap after the clip; the gap is
never closed automatically (K-022). The mirror command `retime.trim_to_source_start` trims
the in point for head overrun. When there is no overrun, the command does nothing and says
so in the status line.

## 8. Cutting and clip operations on Sequence layers

### 8.1 Razor through a ramped clip

Cutting a clip at local time t_c (t_c on the comp frame grid, hence rational):

1. Evaluate s_c = f(t_c) and split the containing segment per §5.3 (exact for native
   segments; bounded rounding only for imported free-influence Maps).
2. The left clip keeps boundaries with t ≤ t_c plus the new terminal boundary (t_c, s_c).
   The right clip takes the rest, re-based: t′ = t − t_c for every boundary (exact rational
   subtraction). Its first boundary is (0, s_c).
3. Both halves keep the **full original source trim window** [src_in, src_out] — the cut
   does not trim source. Each half is thereafter completely independent: editing one half's
   retime never affects the other. This is the Vegas behaviour montage editors expect.
4. `allow_reverse`, interpolation policy, and all flags copy to both halves.

The cut point becomes an edit point; playback across it is frame-exact: the last frame of the
left clip and the first frame of the right clip resolve to the same source position s_c, and
the right clip's first rendered frame is the first frame *after* the edit point on the comp
grid. There is no duplicated or dropped frame.

### 8.2 Trim, slip, slide, copy, paste

- **Trim (edge drag, non-ripple):** trimming an edge inward splits the map at the new edge
  (§5.3) and discards the outside; trimming outward extends per §7.3. Head trims re-base
  boundary times. All exact.
- **Slip:** slipping source under a fixed clip adds a rational constant to every boundary
  `s` and to MapSegment control values. The curve shape is untouched; overrun is recomputed.
- **Slide:** moves the clip along the Sequence layer; local time and the map are untouched.
- **Copy/paste of a clip:** the Retime travels with the clip verbatim.
- **Paste retime onto another clip** (paste-attributes): by default applied verbatim in
  local time — cropped if the target is shorter, extended per §7.3 if longer. A modifier
  offers time-normalised pasting (boundary times scaled by the duration ratio, speeds scaled
  inversely; exact rational scaling). This is the sharing path for ramp presets in community
  packs (K-065).

## 9. The two graph-editor lenses

Both lenses live in the graph editor panel ([07-UI-SPEC.md](07-UI-SPEC.md)): the value view,
the speed view, or both stacked (speed below value). The timeline clip shows only read-only
indication — a retime badge, boundary tick marks, and overrun hatching. No editable curve is
ever drawn on the clip (K-021).

Common to both lenses: boundary drags snap to beat markers and to comp frames (snapping
toggleable, with a temporary-disable modifier); horizontal boundary drags are clamped between
neighbouring boundaries; every draggable value has a numeric field (double-click or Enter);
kink badges (§6.1) and overrun hatching (§7.2) draw identically in both.

### 9.1 Value lens (local time → source time, AE-style)

The x axis is local time, y is source time; the source extent [src_in, src_out] draws as a
shaded band with the media-end line. Slope 1 renders normal speed; flat is a freeze;
descending (only with `allow_reverse`) is reverse.

| Edit | Behaviour | Type change |
|---|---|---|
| Drag boundary vertically | Changes that boundary's `s`. Adjacent RateSegments rescale both endpoint speeds by the exact ratio newΔs/oldΔs (shape preserved); adjacent MapSegments keep tangent slopes and re-solve through the new endpoint, clamped per §6.2. Nothing outside the two adjacent segments changes. | none |
| Drag boundary horizontally | Changes that boundary's `t`; `s` is kept. Both adjacent segments rescale speeds/tangents to preserve their Δs, so nothing downstream changes. | none |
| Drag a tangent handle | Edits m (and, with a modifier, influence b) of a MapSegment. Grabbing a handle on a RateSegment first converts it per §5.1 — exact, indicated only by the type chip. | Rate → Map |
| Add boundary (pen tool or Alt-click on the curve; or `R` at the playhead) | Split per §5.3 at the clicked/playhead time. | per §5.3 |
| Delete boundary | Merge per §5.4. | per §5.4 |
| Numeric entry on a boundary | Fields for local timecode `t` and source timecode `s`; identical semantics to the corresponding drags. | none |

The value lens is realised as the **ordinary graph editor** running on the source-position
channel (K-078): `Retime::source_keyframes` reads the store into bezier keyframes (each
MapSegment contributing its stored tangents, each RateSegment shown as a straight Linear side)
and `Retime::from_source_keyframes` writes an edited keyframe list back as MapSegments, using
the same control-point construction as `anim::CubicSpan::from_ae` — so a Time curve renders
bit-for-bit like the identical keys on a transform property. Editing the Time lens therefore
recommits the channel in the Map (AE) vocabulary; a Hold side is treated as Linear for now
(a stepped Time Remap is future work, as it can't be one monotone MapSegment with exact C0
boundaries).

### 9.2 Speed lens (derivative, Vegas-style semantics)

The x axis is local time, y is speed in per cent; a reference line at 100%, the zero line
(locked glyph when `allow_reverse` is off), and negative space below it. RateSegments draw in
their native shape (two endpoint levels joined by the ease profile); MapSegments draw their
derivative y′(u)/x′(u).

| Edit | Behaviour | Type change |
|---|---|---|
| Drag a RateSegment endpoint level | Sets v₀ or v₁ (Shift = 5-point steps, Ctrl = 0.1 fine). Downstream boundary `s` values recompute exactly — the rest of the clip shows different source frames, the Vegas feel. With Alt held, a **compensating edit**: the next boundary's `s` is pinned by counter-scaling the next segment's speeds/tangents by the exact inverse ratio; nothing beyond the next boundary changes. | none |
| Drag the body of a RateSegment | Moves v₀ and v₁ together (constant offset). Same downstream/compensating semantics. | none |
| Change a RateSegment's ease | Cycles or picks Linear/Slow/Fast/Smooth/Sharp; Δs changes per E(1), downstream recomputes. | none |
| Drag a MapSegment's endpoint speed | Edits the corresponding tangent m; boundaries and Δs unchanged (that is what tangents are). Clamped per §6.2. | none |
| Type a constant % with a segment selected | Replaces the segment with `Rate { v, v, Linear }`; downstream recomputes. Exact (§5.2 last paragraph). | Map → Rate (exact) |
| "Convert to rate" (context menu on a MapSegment) | Shape-preserving fit per §5.2, with the quarter-frame warning badge when applicable. | Map → Rate (fitted) |
| Add point on the profile | Split per §5.3. | per §5.3 |
| Drag below 0% | Clamped at 0 unless `allow_reverse`; the clamp names the flag. | none |

The speed lens never performs hidden integration adjustments elsewhere in the curve: every
edit's region of effect is exactly the dragged segment plus (for non-compensated Rate edits)
the downstream boundary source positions — visible as the value-lens curve translating
vertically after the edited segment.

### 9.3 Numeric entry and readouts

- Graph editor header: speed at the playhead (per cent, one decimal), resolved source
  timecode, and the segment's type chip (RATE / MAP) with its ease name.
- Typing digits with a segment selected opens constant-speed entry (§9.2). Typing with a
  boundary selected opens the t/s fields.
- Timeline clip badge: a mono label with the speed range, e.g. `20–850%`, shown whenever the
  clip's Retime is non-unity ([15-DESIGN.md](15-DESIGN.md) for style).

### 9.4 Type chips and conversion visibility

Every segment shows a small RATE or MAP chip in the graph editor. Conversions triggered by
editing (§9.1, §9.2) change the chip immediately; exact conversions carry no further notice,
fitted conversions carry the warning badge (§5.2). There is no modal dialogue for any
conversion.

## 10. Frame interpolation policy

Per-clip and per-Footage-layer render policy, orthogonal to the map (glossary §4). The map
resolves a fractional source position; the policy decides the pixels:

- **Nearest** (default): round to the nearest source sample instant, ties toward the earlier
  frame. Deterministic, crisp, zero ghosting — the gaming-footage default; matches the
  montage community's universal "Disable Resample" habit in Vegas.
- **Blend:** crossfade the two neighbouring frames weighted by the fractional phase.
  Cheap smoothness; visible ghosting on fast motion.
- **Flow:** optical-flow synthesis of the intermediate frame (engine and parameters in
  [08-EFFECTS.md](08-EFFECTS.md)). Twixtor-class quality is the bar (K-064).

Requirements and expectations for Flow:

- Quality expectation: clean, well-lit footage at 2× slow-down SHOULD show no obvious
  artefacts; below ~30% speed artefacts become likely on 24–30 fps sources.
- Known artefact cases (document in-app help, do not pretend otherwise): tearing and
  stretching at occlusion boundaries (objects crossing, limbs against background); edge
  warping on pans as unseen content enters frame; smearing of motion-blurred elements;
  confusion on repetitive or low-contrast textures; breakdown at very large per-frame
  displacement.
- Fallbacks: during preview, adaptive degradation MAY substitute Blend for Flow under load
  (never at export — glossary §5). Per-frame, when the flow solver's confidence falls below
  threshold, the engine SHOULD fall back to Blend for that frame and MAY mark the frame in
  the cache bar diagnostics. Export MUST honour the requested policy, using the CPU
  reference implementation when no capable GPU is present (K-019); export never silently
  downgrades.
- Source-rate guidance surfaced in the UI: when any segment's effective sampling ratio
  r = |speed| × source_fps / comp_fps falls below 0.3 and the policy is Nearest or Blend,
  the clip and the graph editor SHOULD show an advisory badge — "holding each source frame
   3+ comp frames; consider Flow or higher-rate source". In-app guidance follows the
  community rule of thumb: 60 fps source for 30–40% playback, 120 fps+ below 20%, with Flow
  bridging the remainder.
- When the clip's source is a **comp**, the engine SHOULD evaluate the nested comp at the
  exact fractional retimed time instead of synthesising between its frame-grid renders
  (procedural content stays perfectly smooth in slow motion — a genuine advantage over AE's
  grid-stepped nested sampling). Footage inside the nested comp still steps at its own rate;
  the clip's interpolation policy then applies to the nested comp's *output* only when exact
  evaluation is disabled (per-clip toggle, default on).

## 11. Composition with the rest of the system

Time resolution order for a rendered frame (full chain in
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)):

```
comp time → (layer in point, parent comp) → layer time → [clip offset] → local time
          → Retime f → source time → interpretation (fps override) → sample request(s)
```

### 11.1 Layer in/out points

In/out edits are boundary edits, not retime edits; they crop or extend the map per §7.3 and
§8.2. Moving a whole layer or clip in comp time changes nothing in the Retime.

### 11.2 Stretch

Stretch (glossary §4) rescales the layer's keyframes — including Retime. Applying stretch
factor k rewrites the store: every boundary t multiplies by k, every speed (v, m) divides by
k, influences unchanged. Exact rational operations; no hidden runtime multiplier, so the
graph editor always shows the true curve. Stretch on a Sequence layer applies to the layer's
output as a whole and does not rewrite per-clip Retimes ([03-DATA-MODEL.md](03-DATA-MODEL.md)).

### 11.3 Precomps

Retiming a Precomp layer (or a clip sourcing a comp) maps into the nested comp's time: the
entire nested comp — all its animation — retimes together, as in AE. Overrun beyond the
nested comp's duration holds the boundary frame of the nested comp's render. Nested comps'
own Retimes compose by function composition, resolved outer-first in the evaluation graph.

### 11.4 Motion blur

Shutter sampling MUST happen in retimed time: for each of the N shutter sub-samples at comp
times t + kΔ, the full chain including f resolves a source position. Blur therefore scales
naturally with speed — an 850% segment sweeps 8.5× the source span per shutter interval,
a freeze produces zero retime-induced blur. When the interpolation policy is Flow, the flow
vectors computed for frame synthesis SHOULD be reused for flow-based motion blur
([08-EFFECTS.md](08-EFFECTS.md), RSMB-class) rather than re-estimated.

### 11.5 Audio

v1 Retime is video-only (K-050 scope). Audio layers have no Retime; a retimed clip or layer
contributes no speed-matched audio, and Luminal MUST NOT attempt naive resampled audio under
a ramp. Pitch-preserving audio retime is a Composer-era feature
([09-AUDIO.md](09-AUDIO.md)). Montage practice — game audio muted, music driving the edit —
makes this the right v1 cut.

### 11.6 Expressions

Expressions ([12-PLUGINS.md](12-PLUGINS.md) §scripting) observe:

- `time` remains layer time; property keyframes live in layer time and are **not** remapped
  by Retime (transform and effect animation stays put when the footage under it is ramped —
  deliberately unlike Premiere's coupling, which is a top reported pain).
- `thisLayer.sourceTime(t = time)` returns the resolved source time (post-clamp);
  `thisLayer.retimeSpeed(t = time)` returns the signed speed. Both are deterministic pure
  reads and participate in dependency hashing.
- Expressions cannot drive Retime in v1 (no expression slot on the retime store) — see open
  questions.

### 11.7 Evaluation graph and cache keys

Retime resolves during the cheap metadata pass (K-015): for each requested output frame it
emits a **sample request tuple** — for Nearest `(footage id, interp settings, frame index)`;
for Blend `(…, frameA, frameB, phase)` with phase quantised to 1/1024; for Flow
`(…, frameA, frameB, phase, flow params, algorithm version)`. Motion blur multiplies
requests by shutter sub-samples.

Cache entries are keyed by content hash of the resolved tuple, never by timeline position or
by a hash of the whole curve (K-016). Consequences (MUST):

- A retime edit invalidates exactly the output frames whose resolved tuples changed. Frames
  before the edited segment are untouched; frames after it invalidate only because their
  resolved source positions actually moved (§9.2 downstream recompute) — and a compensated
  edit (Alt) invalidates nothing beyond the next boundary.
- Decoded source frames cache independently of the map, so scrubbing a re-edited ramp re-uses
  every decode.
- Two clips with different curves that resolve a frame to the same tuple share the cache
  entry.

Because boundary maths is rational, undoing an edit restores bit-identical tuples and the
cache re-validates for free.

## 12. UI affordances and keyboard workflow

Final bindings and panel layout belong to [07-UI-SPEC.md](07-UI-SPEC.md); the commands, their
semantics, and proposed defaults are normative here.

### 12.1 Commands

| Command | Effect | Proposed default |
|---|---|---|
| `retime.add_boundary` | Split at the playhead (§5.3) on the selected clip/layer | `R` |
| `retime.set_segment_speed` | Constant-speed numeric entry for the selected segment (§9.2) | type digits |
| `retime.trim_to_source_end` / `…_start` | §7.4 | `E` / `Shift+E` |
| `retime.freeze_at_playhead` | §7.3 | `F` (retime context) |
| `retime.toggle_reverse_allowed` | §6.2, per clip | none (header toggle) |
| `retime.apply_preset <shape>` | Set selected segment's ease / apply preset (§12.2) | `1`–`6` (retime context) |
| `retime.select_prev/next_segment` | Walk segments of the focused clip | `[` / `]` |
| `retime.quantise_boundaries_to_beats` | Snap every boundary of the selection to the nearest beat marker (bounded by neighbours) | none |
| `retime.toggle_lens` | Value / speed / stacked | `Tab` (graph editor) |

### 12.2 Preset ramp shapes

The preset row offers **Linear, Fast, Slow, Smooth, Sharp, Hold** — deliberately the Vegas
fade vocabulary so the target audience needs no relearning. Applied to a selected segment,
each sets the RateSegment ease (converting a MapSegment via §5.2's exact constant path first
when needed); Hold sets `Rate { v₀, v₀, Linear }` (sustain the entry speed to the next
boundary — the kink at the next boundary is expected and badged). Preset application is
one undo step.

### 12.3 Beat markers

Boundary drags snap to beat markers (glossary §3) in both lenses. The
`retime.quantise_boundaries_to_beats` command bulk-snaps. Beat markers also drive playhead
navigation (jump to next/previous beat), which combined with `R` makes ramp placement a
two-key loop. Nothing about beat snapping ever moves a clip edge — snapping applies to
retime boundaries and the playhead only (K-022).

### 12.4 The three-point kill ramp

The canonical montage gesture (Vegas tutorials call it "velocity sync"): normal into the
moment, a fast rush, then a slow hold on the impact. Keyboard flow, entirely without the
mouse:

1. Playhead on the beat before the kill (beat-jump keys) — `R` (boundary 1).
2. Beat-jump to the kill — `R` (boundary 2). `[` to select the segment between them, type
   `850`, Enter.
3. `]` to select the following segment, type `20`, Enter. The hold on the impact is now a
   20% crawl; the kink badges at both boundaries confirm the hard cuts.
4. If the tail hatches (overrun), `E` trims to source end — or leave the hold, which is
   often the desired freeze-into-transition.

Worked example (comp 60 fps, clip D = 4 s, source 60 fps, 10 s available): boundaries at
t = 0, 1.2, 1.45, 2.8, 4 with segments 100%, 850%, 20%, 100% (all `Rate { v, v, Linear }`).
Boundary source positions, exactly: s = 0, 6/5, 6/5 + 0.25·8.5 = 133/40, 133/40 + 1.35·0.2
= 719/200, 719/200 + 1.2 = 959/200 (= 4.795 s ≤ 10 s, no overrun). Cutting at any of these
boundaries later reproduces these rationals bit-for-bit.

## 13. Import mappings

### 13.1 After Effects → Luminal (via the exporter panel, K-060; pipeline in [11-AE-IMPORT.md](11-AE-IMPORT.md))

AE's **Time Remap** property (AE's name for the value-graph view of retiming) maps to Retime
losslessly:

- Each adjacent pair of Time Remap keyframes → one **MapSegment**: keyframe values become
  boundary `s`, per-side speed becomes m, per-side influence becomes b — the identical
  parametric-bezier maths (K-025), so the curve is reproduced exactly, including
  free-influence shapes outside the polynomial subclass.
- AE hold keyframes → a MapSegment with m₀ = m₁ = 0 and equal boundary values (an exact
  freeze), or equivalently `Rate { 0, 0, Linear }` — the importer SHOULD emit the Rate form
  for legibility.
- AE linear keyframes → the polynomial subclass with matching endpoint slopes; the importer
  SHOULD recognise constant-slope runs and emit `Rate { v, v, Linear }`.
- Non-monotonic AE curves (reverses, palindromes) import intact with `allow_reverse`
  enabled automatically, plus an import notice on the clip.
- The layer's extension past the last keyframe (AE's held tail) becomes explicit: the map is
  extended per §7.3 with a zero-speed segment, so what AE leaves implicit is visible.
- An expression on AE's Time Remap imports as preserved-but-disabled expression text attached
  to the layer notes, with the keyframe curve imported as above (expressions cannot drive
  Retime in v1, §11.6). AE Timewarp (the effect) does not map to Retime; it imports as an
  inert placeholder (K-060). AE frame blending switches map to the interpolation policy:
  off → Nearest, Frame Mix → Blend, Pixel Motion → Flow.

### 13.2 Vegas → Luminal (mapping documented now; a Vegas importer is future work)

Vegas expresses retiming as a per-event **velocity envelope** (Vegas's terms) plus two
constant-factor mechanisms. The mapping, for when an importer exists:

- Envelope points (event time, percentage) with per-segment fade types → **RateSegments**:
  consecutive points (tᵢ, pᵢ), (tᵢ₊₁, pᵢ₊₁) become `Rate { pᵢ/100, pᵢ₊₁/100, ease }`. Fade
  types map by name: Linear→Linear, Fast→Fast, Slow→Slow, Smooth→Smooth, Sharp→Sharp,
  Hold→`Rate { pᵢ/100, pᵢ/100, Linear }`. Vegas's Fast/Slow are logarithmic and its
  Smooth/Sharp are its own cubics, while Luminal's ease family is (piecewise-)quadratic: the
  shapes differ slightly mid-segment (endpoint speeds and boundary source positions are
  matched exactly; interior deviation is small and MUST be noted once per import, not per
  segment).
- Vegas's event playback rate (0.25–4×) and Ctrl-drag stretch multiply into the envelope:
  fold the constant factor into every RateSegment's speeds — exact.
- Vegas's silent behaviours become visible Luminal states: media exhaustion (Vegas's "tiny V"
  notch, plus its default looping) imports as overrun with hatching and **no looping**;
  reverse-past-start's silent first-frame hold imports as head overrun.
- Vegas resample modes map to interpolation policy: Disable Resample → Nearest, Smart/Force
  Resample → Blend. Vegas has no flow mode. Vegas project supersampling does not map to
  anything in Retime (it never synthesised source frames) and is ignored with a notice.

## Open questions

1. **Expression-driven Retime.** Should a later version allow an expression slot on the
   retime store (AE parity: expressions on Time Remap, `loopOut` source loops)? Requires
   defining exactness and cache semantics for a scripted map; deliberately excluded from v1
   (§11.6).
2. **Loop-past-end as an opt-in.** Overrun always holds (§7.2). Community Vegas habits
   include deliberate loop-past-end; is an explicit per-clip "loop source" interpretation
   toggle (visibly distinct from overrun) worth adding, or is `loopOut`-style scripting the
   right home?
3. **Compensated-edit affordance.** Alt-drag pins the next boundary (§9.2). Should there be
   a "pin all downstream boundaries" mode (solving across multiple segments), or does that
   reintroduce the AE speed-graph opacity this design removes?
4. **Fitted-conversion threshold.** The quarter-source-frame warning threshold (§5.2) is a
   first guess; validate against real Twixtor-era projects during the prototype.
5. **Blend phase quantisation.** 1/1024 phase steps (§11.7) trade cache hits against banding
   in long slow ramps; confirm with perceptual testing, and whether Flow needs finer phase.
6. **Nested-comp exact evaluation by default.** §10 defaults continuous sampling of nested
   comps to on; confirm there is no pathological interaction with nested caches before
   locking, since it multiplies distinct evaluation times.
7. **Audio preview under ramps.** v1 mutes nothing automatically (audio simply is not
   retimed); should a clip with non-unity Retime offer a one-click "mute linked audio"
   affordance when the Composer lands, or is that already moot given montage practice?
