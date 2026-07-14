//! Retime: the map from a clip's local time to source time — the evaluation
//! core of docs/04-RETIMING.md §2–§5 (binding), with the cubic solve from
//! docs/impl/keyframe-eval.md §2/§4.
//!
//! In plain terms: a Retime answers one question — "when the clip's own clock
//! reads t, which moment of the source footage is on screen?". The answer is a
//! curve made of segments. A *Rate* segment speaks Vegas: "play at 200%,
//! easing down to 50%". A *Map* segment speaks After Effects: "be at source
//! second 3.2 by clip second 1.0", shaped by tangent handles. Both kinds meet
//! at *boundaries*, and every boundary stores its exact source position as a
//! fraction — never a rounded decimal — so cutting and re-editing a ramp can
//! never nudge a frame off a beat. Rendering evaluates the curve in fast
//! floating point; the exact fractions are the durable truth the floats are
//! recomputed from.
//!
//! Scope note: this module is the maths only. Overrun clamping (§7), the two
//! graph-editor lenses (§9), cutting (§8) and the flow interpolation engine
//! (§10) build on top of it and live elsewhere.

use crate::time::{Rational, TimeError};
use serde::{Deserialize, Serialize};

/// What can go wrong inside retime maths or structure checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RetimeError {
    /// Rational arithmetic failed even after the §4.1 fallback (only
    /// reachable with astronomically out-of-range times).
    #[error("retime arithmetic failed: {0}")]
    Arithmetic(#[from] TimeError),
    /// The store's shape breaks an invariant (docs/04-RETIMING.md §3).
    #[error("invalid retime structure: {0}")]
    InvalidStructure(&'static str),
}

/// The shape of a speed transition inside a [`RateSegment`] — deliberately
/// the Vegas fade-type vocabulary (docs/04-RETIMING.md §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ease {
    /// Straight speed ramp.
    Linear,
    /// Lingers at the starting speed, transitions late.
    Slow,
    /// Transitions early, settles at the ending speed.
    Fast,
    /// S-curve: gentle at both ends.
    Smooth,
    /// Inverse S-curve: brisk at both ends.
    Sharp,
}

impl Ease {
    /// E(1), the exact integral of the ease over the whole segment
    /// (docs/04-RETIMING.md §4.1 table). This is the number that makes
    /// boundary source positions exact: how much of the speed *change*
    /// contributes to the total source advance.
    pub fn e_at_1(self) -> Rational {
        let (num, den) = match self {
            Ease::Linear | Ease::Smooth | Ease::Sharp => (1, 2),
            Ease::Slow => (1, 3),
            Ease::Fast => (2, 3),
        };
        // Fixed single-digit fractions cannot fail to construct; the
        // fallback value is unreachable.
        Rational::new(num, den).unwrap_or(Rational::ZERO)
    }

    /// E(u) = ∫₀ᵘ e(w) dw in f64, for per-sample rendering
    /// (docs/04-RETIMING.md §4.1 table, including the piecewise Smooth and
    /// Sharp forms).
    pub fn big_e(self, u: f64) -> f64 {
        match self {
            Ease::Linear => u * u / 2.0,
            Ease::Slow => u * u * u / 3.0,
            Ease::Fast => u * u - u * u * u / 3.0,
            Ease::Smooth => {
                if u <= 0.5 {
                    2.0 * u * u * u / 3.0
                } else {
                    let w = 1.0 - u;
                    u + 2.0 * w * w * w / 3.0 - 0.5
                }
            }
            Ease::Sharp => {
                if u <= 0.5 {
                    u * u - 2.0 * u * u * u / 3.0
                } else {
                    2.0 * u * u * u / 3.0 - u * u + u - 1.0 / 6.0
                }
            }
        }
    }

    /// e(u), the speed-profile shape itself (0 at the segment start, 1 at
    /// the end). Used for the instantaneous speed readout.
    pub fn small_e(self, u: f64) -> f64 {
        match self {
            Ease::Linear => u,
            Ease::Slow => u * u,
            Ease::Fast => 2.0 * u - u * u,
            Ease::Smooth => {
                if u <= 0.5 {
                    2.0 * u * u
                } else {
                    let w = 1.0 - u;
                    1.0 - 2.0 * w * w
                }
            }
            Ease::Sharp => {
                if u <= 0.5 {
                    2.0 * u - 2.0 * u * u
                } else {
                    2.0 * u * u - 2.0 * u + 1.0
                }
            }
        }
    }
}

/// How fractional source positions become pixels (docs/04-RETIMING.md §10) —
/// a per-clip render policy, orthogonal to the retime map itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum Interpolation {
    /// Round to the nearest source frame — crisp, deterministic, the
    /// gaming-footage default.
    #[default]
    Nearest,
    /// Crossfade the two neighbouring frames.
    Blend,
    /// Optical-flow synthesis of the in-between frame.
    Flow(FlowParams),
}

/// Optical-flow parameters (docs/08-EFFECTS.md). Placeholder for now: the
/// flow engine is future work, but the policy must already round-trip
/// project files, so the shape exists with only the forward-compat map.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FlowParams {
    /// Unknown fields from newer Kiriko versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One point where two segments meet (or the curve starts/ends). Stores the
/// exact local time and the exact source position — the "frame on the beat
/// stays on the beat" guarantee lives in these two fractions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Boundary {
    /// Local time (seconds into the clip).
    pub t: Rational,
    /// Source time — exact, shared by both adjacent segments (C0).
    pub s: Rational,
    /// When true, edits keep the speed equal on both sides of this boundary
    /// (docs/04-RETIMING.md §6.1). Evaluation ignores it.
    #[serde(default)]
    pub smooth: bool,
    /// Unknown fields from newer Kiriko versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Boundary {
    pub fn new(t: Rational, s: Rational) -> Self {
        Self {
            t,
            s,
            smooth: false,
            extra: serde_json::Map::new(),
        }
    }
}

/// One span of the retime curve, in one of the two native vocabularies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RetimeSegment {
    /// Speed-native: constant or eased speed (Vegas semantics).
    Rate(RateSegment),
    /// Value-native: cubic source-time curve (After Effects semantics).
    Map(MapSegment),
}

/// Speed-defined segment. Source advance is a closed-form integral
/// (docs/04-RETIMING.md §4.1): speed runs from `v0` to `v1` along the ease.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateSegment {
    /// Speed at the segment start (1 = 100%, 0 = freeze).
    pub v0: Rational,
    /// Speed at the segment end.
    pub v1: Rational,
    /// Shape of the speed transition between them.
    pub ease: Ease,
    /// Unknown fields from newer Kiriko versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl RateSegment {
    pub fn new(v0: Rational, v1: Rational, ease: Ease) -> Self {
        Self {
            v0,
            v1,
            ease,
            extra: serde_json::Map::new(),
        }
    }
}

/// Value-defined segment: an x-monotone parametric cubic bezier in (t, s),
/// AE-compatible (docs/04-RETIMING.md §4.2, K-025). Endpoint positions come
/// from the two boundaries; this stores only the handle description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapSegment {
    /// Outgoing speed (source seconds per local second) at the start.
    pub m0: Rational,
    /// Incoming speed at the end.
    pub m1: Rational,
    /// Outgoing influence — how far the start handle reaches, in (0, 1].
    pub b0: Rational,
    /// Incoming influence, in (0, 1].
    pub b1: Rational,
    /// Unknown fields from newer Kiriko versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl MapSegment {
    pub fn new(m0: Rational, m1: Rational, b0: Rational, b1: Rational) -> Self {
        Self {
            m0,
            m1,
            b0,
            b1,
            extra: serde_json::Map::new(),
        }
    }
}

/// One retime store. Owned by a Clip or by a Footage/Precomp layer
/// (docs/03-DATA-MODEL.md); this module is only the curve and its maths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Retime {
    /// n + 1 boundaries for n segments. `boundaries[0].t == 0`, the last sits
    /// at the clip duration. Strictly increasing in t. Authoritative for
    /// evaluation and cutting.
    pub boundaries: Vec<Boundary>,
    /// n segments; `segments[i]` spans `boundaries[i] .. boundaries[i + 1]`.
    pub segments: Vec<RetimeSegment>,
    /// Reverse gate (docs/04-RETIMING.md §6.2), default off. While off,
    /// evaluation clamps RateSegment speeds to ≥ 0, so the curve never runs
    /// backwards; MapSegment monotonicity is an editing-time invariant and
    /// is not re-checked per sample.
    #[serde(default)]
    pub allow_reverse: bool,
    /// Frame interpolation policy (§10). Default Nearest.
    #[serde(default)]
    pub interpolation: Interpolation,
    /// Unknown fields from newer Kiriko versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Retime {
    /// The "no retiming" store (docs/04-RETIMING.md §3 default state): one
    /// 100%-speed segment across `[0, duration]`, source running from
    /// `source_in`. Evaluates as `f(t) = source_in + t` — a pure pass-through
    /// that must render identically to Retime being absent.
    pub fn identity(duration: Rational, source_in: Rational) -> Self {
        // source_in + duration is exact for any real media; the fallback
        // chain below only degrades past ~400 years of source time, where a
        // frozen tail beats a panic (engine crates never panic, K-011).
        let s_end = add_with_flick_fallback(source_in, duration).unwrap_or(source_in);
        Self {
            boundaries: vec![
                Boundary::new(Rational::ZERO, source_in),
                Boundary::new(duration, s_end),
            ],
            segments: vec![RetimeSegment::Rate(RateSegment::new(
                Rational::ONE,
                Rational::ONE,
                Ease::Linear,
            ))],
            allow_reverse: false,
            interpolation: Interpolation::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// A single constant-speed retime over [0, `duration`] (local time),
    /// source running from `source_in` at `speed` (1 = 100%). This is the
    /// simple "play this clip faster/slower" case the timeline speed control
    /// produces; the graph-editor lenses build richer stores later.
    pub fn constant_speed(duration: Rational, source_in: Rational, speed: Rational) -> Self {
        Self::single_ramp(duration, source_in, speed, speed, Ease::Linear)
    }

    /// A single ramping segment over [0, `duration`]: speed eases from `v0`
    /// to `v1` (1 = 100%) with the given `ease`. The whole-clip velocity ramp
    /// — the montage gesture — before per-boundary editing arrives. `v0 == v1`
    /// with `Ease::Linear` is exactly [`Self::constant_speed`].
    pub fn single_ramp(
        duration: Rational,
        source_in: Rational,
        v0: Rational,
        v1: Rational,
        ease: Ease,
    ) -> Self {
        let mut r = Self {
            boundaries: vec![
                Boundary::new(Rational::ZERO, source_in),
                Boundary::new(duration, source_in),
            ],
            segments: vec![RetimeSegment::Rate(RateSegment::new(v0, v1, ease))],
            allow_reverse: v0.is_negative() || v1.is_negative(),
            interpolation: Interpolation::default(),
            extra: serde_json::Map::new(),
        };
        // Fill the end boundary's source position exactly from the rate.
        let _ = r.recompute_boundaries();
        r
    }

    /// If this retime is a single ramping segment, its (start speed, end
    /// speed, ease) — for the timeline speed control to display and edit.
    /// None for multi-segment stores (which need the graph editor).
    pub fn single_ramp_view(&self) -> Option<(f64, f64, Ease)> {
        match self.segments.as_slice() {
            [RetimeSegment::Rate(seg)] => Some((seg.v0.to_f64(), seg.v1.to_f64(), seg.ease)),
            _ => None,
        }
    }

    /// Build a retime whose speed is piecewise-linear through `keys` (local
    /// time → speed, 1.0 = 100%): each consecutive pair becomes a Linear-ease
    /// Rate segment, and boundary source positions are integrated from
    /// `source_in` (§4.1). This is the store the timeline's keyframable speed
    /// row produces (K-072). Needs ≥ 2 keys, the first at local time 0, times
    /// strictly increasing; returns None otherwise (caller keeps its store).
    pub fn from_speed_keyframes(
        source_in: Rational,
        keys: &[(Rational, Rational)],
    ) -> Option<Self> {
        if keys.len() < 2 || keys[0].0 != Rational::ZERO {
            return None;
        }
        if keys.windows(2).any(|w| w[1].0 <= w[0].0) {
            return None;
        }
        let boundaries = keys
            .iter()
            .map(|(t, _)| Boundary::new(*t, source_in))
            .collect();
        let segments = keys
            .windows(2)
            .map(|w| RetimeSegment::Rate(RateSegment::new(w[0].1, w[1].1, Ease::Linear)))
            .collect();
        let mut r = Self {
            boundaries,
            segments,
            allow_reverse: keys.iter().any(|(_, v)| v.is_negative()),
            interpolation: Interpolation::default(),
            extra: serde_json::Map::new(),
        };
        r.recompute_boundaries().ok()?;
        Some(r)
    }

    /// Build a retime whose source time is piecewise-linear through value
    /// keyframes `keys` (local time → source time): each consecutive pair
    /// becomes a constant-speed Rate segment whose speed is Δsource / Δlocal.
    /// This is the store the graph editor's value lens (AE Time Remap) makes —
    /// the mirror of [`Self::from_speed_keyframes`], and exact because a
    /// constant-speed segment's advance `d · v` is precisely `Δsource`, so
    /// [`Self::recompute_boundaries`] reproduces every stored `s`. Needs ≥ 2
    /// keys, the first at local time 0, times strictly increasing; returns None
    /// otherwise (caller keeps its store).
    pub fn from_value_keyframes(keys: &[(Rational, Rational)]) -> Option<Self> {
        if keys.len() < 2 || keys[0].0 != Rational::ZERO {
            return None;
        }
        if keys.windows(2).any(|w| w[1].0 <= w[0].0) {
            return None;
        }
        let boundaries = keys.iter().map(|(t, s)| Boundary::new(*t, *s)).collect();
        let mut segments = Vec::with_capacity(keys.len() - 1);
        let mut any_reverse = false;
        for w in keys.windows(2) {
            let dt = w[1].0.checked_sub(w[0].0).ok()?;
            let ds = w[1].1.checked_sub(w[0].1).ok()?;
            let v = ds.checked_div(dt).ok()?;
            any_reverse |= v.is_negative();
            segments.push(RetimeSegment::Rate(RateSegment::new(v, v, Ease::Linear)));
        }
        let mut r = Self {
            boundaries,
            segments,
            allow_reverse: any_reverse,
            interpolation: Interpolation::default(),
            extra: serde_json::Map::new(),
        };
        r.recompute_boundaries().ok()?;
        Some(r)
    }

    /// Build a value-lens retime from AE Time Remap keyframes: local time →
    /// source time, each side carrying the same bezier tangent (`SideInterp`)
    /// the transform graph uses (K-078). Every consecutive pair becomes a
    /// [`MapSegment`] whose control handles are the left key's out-tangent and
    /// the right key's in-tangent — the exact control-point construction of
    /// [`crate::anim::CubicSpan::from_ae`] — so the source curve evaluates
    /// identically to `anim::evaluate` over the same keys. A Linear side lies on
    /// the chord (AE semantics); a Hold side is treated as Linear here (a
    /// stepped Time Remap is future work). Source positions round onto the flick
    /// grid. Needs ≥ 2 keys, the first at local time 0, strictly increasing
    /// times; returns None otherwise (caller keeps its store).
    pub fn from_source_keyframes(keys: &[crate::anim::Keyframe]) -> Option<Self> {
        use crate::anim::SideInterp;
        if keys.len() < 2 || keys[0].time != Rational::ZERO {
            return None;
        }
        if keys.windows(2).any(|w| w[1].time <= w[0].time) {
            return None;
        }
        let grid = Rational::FLICK_DEN;
        let one_third = Rational::new(1, 3).ok()?;
        let on_grid = |x: f64| Rational::from_f64_on_grid(x, grid).unwrap_or(Rational::ZERO);
        let influence_of = |inf: f64| {
            if (inf - 1.0 / 3.0).abs() < 1e-9 {
                one_third // keep the exact 1/3 so the polynomial fast path holds
            } else {
                Rational::from_f64_on_grid(inf.clamp(1e-3, 1.0), grid).unwrap_or(one_third)
            }
        };
        let boundaries = keys
            .iter()
            .map(|k| Boundary::new(k.time, on_grid(k.value)))
            .collect();
        let mut segments = Vec::with_capacity(keys.len() - 1);
        let mut any_reverse = false;
        for w in keys.windows(2) {
            let dt = w[1].time.to_f64() - w[0].time.to_f64();
            let chord = if dt > 0.0 {
                (w[1].value - w[0].value) / dt
            } else {
                0.0
            };
            // A Linear/Hold side sits on the chord with influence ⅓ — the same
            // convention `anim::side_params` uses, so the two curves agree.
            let side = |si: SideInterp| -> (Rational, Rational) {
                match si {
                    SideInterp::Bezier { speed, influence } => {
                        (on_grid(speed), influence_of(influence))
                    }
                    _ => (on_grid(chord), one_third),
                }
            };
            let (m0, b0) = side(w[0].interp_out);
            let (m1, b1) = side(w[1].interp_in);
            any_reverse |= m0.is_negative() || m1.is_negative() || chord < 0.0;
            segments.push(RetimeSegment::Map(MapSegment::new(m0, m1, b0, b1)));
        }
        let r = Self {
            boundaries,
            segments,
            allow_reverse: any_reverse,
            interpolation: Interpolation::default(),
            extra: serde_json::Map::new(),
        };
        r.validate().ok()?;
        Some(r)
    }

    /// The earliest local time (seconds) at which this retime reaches
    /// `source_duration` seconds of source — i.e. the clip runs out of media
    /// and the boundary frame must be held (docs/04-RETIMING.md §7, the
    /// beat-sync covenant: boundaries never move, so overrun is drawn, not
    /// trimmed automatically). None if the source lasts the whole clip.
    ///
    /// Source time advances monotonically for a forward retime, so the crossing
    /// is bisected inside the segment that straddles `source_duration` — exact
    /// enough for the indicator, and robust for every ease.
    pub fn overrun_local_time(&self, source_duration: Rational) -> Option<f64> {
        let dur = source_duration.to_f64();
        for i in 0..self.boundaries.len() {
            if self.boundaries[i].s.to_f64() < dur {
                continue;
            }
            if i == 0 {
                return Some(0.0); // starts already past the source end
            }
            let (mut lo, mut hi) = (
                self.boundaries[i - 1].t.to_f64(),
                self.boundaries[i].t.to_f64(),
            );
            for _ in 0..40 {
                let mid = 0.5 * (lo + hi);
                if self.evaluate(mid) >= dur {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return Some(hi);
        }
        None
    }

    /// The speed keyframes (local time → speed) that reproduce this retime,
    /// when every segment is a Linear-ease Rate segment; None otherwise (eased
    /// or Map stores are edited in the graph editor, not as plain keys).
    pub fn speed_keyframes(&self) -> Option<Vec<(Rational, Rational)>> {
        let mut out = Vec::with_capacity(self.segments.len() + 1);
        for (i, seg) in self.segments.iter().enumerate() {
            let RetimeSegment::Rate(s) = seg else {
                return None;
            };
            if s.ease != Ease::Linear {
                return None;
            }
            out.push((self.boundaries[i].t, s.v0));
            if i + 1 == self.segments.len() {
                out.push((self.boundaries[i + 1].t, s.v1));
            }
        }
        Some(out)
    }

    /// The value keyframes (local time → source time) that describe this retime
    /// in the value lens: simply each boundary's exact `(t, s)`. Always
    /// available — every boundary carries its source position — which is what
    /// lets the value lens keyframe *any* store, unlike [`Self::speed_keyframes`]
    /// (which only speaks for all-linear Rate stores).
    pub fn value_keyframes(&self) -> Vec<(Rational, Rational)> {
        self.boundaries.iter().map(|b| (b.t, b.s)).collect()
    }

    /// The value-lens keyframes with bezier tangents (local time → source time,
    /// each side a `SideInterp`) — the inverse of [`Self::from_source_keyframes`],
    /// so the value lens can draw and edit *any* store with the transform
    /// graph's own handles (K-078). A [`MapSegment`] contributes its stored
    /// tangents exactly; a [`RateSegment`] shows as a straight (Linear) side,
    /// since its eased source advance has no single per-key tangent — dragging a
    /// handle there recommits the whole channel through `from_source_keyframes`.
    pub fn source_keyframes(&self) -> Vec<crate::anim::Keyframe> {
        use crate::anim::{Keyframe, SideInterp};
        let out_side = |seg: &RetimeSegment| match seg {
            RetimeSegment::Map(m) => SideInterp::Bezier {
                speed: m.m0.to_f64(),
                influence: m.b0.to_f64(),
            },
            RetimeSegment::Rate(_) => SideInterp::Linear,
        };
        let in_side = |seg: &RetimeSegment| match seg {
            RetimeSegment::Map(m) => SideInterp::Bezier {
                speed: m.m1.to_f64(),
                influence: m.b1.to_f64(),
            },
            RetimeSegment::Rate(_) => SideInterp::Linear,
        };
        let last = self.boundaries.len().saturating_sub(1);
        self.boundaries
            .iter()
            .enumerate()
            .map(|(i, b)| Keyframe {
                time: b.t,
                value: b.s.to_f64(),
                interp_in: if i == 0 {
                    SideInterp::Linear
                } else {
                    in_side(&self.segments[i - 1])
                },
                interp_out: if i == last {
                    SideInterp::Linear
                } else {
                    out_side(&self.segments[i])
                },
            })
            .collect()
    }

    /// The index of the segment covering local time `t`, or None if `t` is
    /// outside the domain `[0, D]`. `t == D` resolves to the final segment.
    pub fn segment_index_at(&self, t: Rational) -> Option<usize> {
        let last = self.boundaries.last()?;
        if self.segments.is_empty() || t < self.boundaries[0].t || t > last.t {
            return None;
        }
        for i in 0..self.segments.len() {
            if t < self.boundaries[i + 1].t {
                return Some(i);
            }
        }
        Some(self.segments.len() - 1)
    }

    /// A copy with the ease of the RateSegment covering local time `t` set to
    /// `ease`, downstream source positions recomputed (docs/04-RETIMING.md §9.2:
    /// "change a RateSegment's ease — Δs changes per E(1), downstream recomputes").
    /// The segment's start position is pinned (K-070), so only frames after it
    /// move. None if `t` is outside the domain or lands in a MapSegment.
    pub fn with_segment_ease(&self, t: Rational, ease: Ease) -> Option<Retime> {
        let i = self.segment_index_at(t)?;
        let mut r = self.clone();
        let RetimeSegment::Rate(seg) = &mut r.segments[i] else {
            return None;
        };
        seg.ease = ease;
        r.recompute_boundaries().ok()?;
        Some(r)
    }

    /// A copy with the endpoint speeds of the RateSegment covering local time `t`
    /// set to `(v0, v1)`, downstream source positions recomputed (docs/04-RETIMING.md
    /// §9.2: "drag a RateSegment endpoint level — downstream boundary `s` values
    /// recompute exactly"). The segment start is pinned (K-070). Unlike the
    /// speed-keyframe path this works on eased segments too. None for a MapSegment
    /// or out-of-domain `t`.
    pub fn with_segment_speeds(&self, t: Rational, v0: Rational, v1: Rational) -> Option<Retime> {
        let i = self.segment_index_at(t)?;
        let mut r = self.clone();
        let RetimeSegment::Rate(seg) = &mut r.segments[i] else {
            return None;
        };
        seg.v0 = v0;
        seg.v1 = v1;
        r.recompute_boundaries().ok()?;
        Some(r)
    }

    /// Split this retime at local time `t` into two retimes covering [0, t]
    /// and [t, D], each with its own domain starting at 0 (docs/04-RETIMING.md
    /// §5.3, §8: cutting a clip partitions its retime exactly). A Linear-ease
    /// Rate segment splits into two Rate segments (the montage workflow: cut
    /// first, ramp after); an eased Rate segment or a polynomial MapSegment
    /// splits via the §5.1/§5.3 exact conversion — both halves become
    /// polynomial MapSegments and the source curve is unchanged. Returns None
    /// for a cut outside (0, D) or one landing in a general-influence
    /// MapSegment (the §5.3 numeric split of those is confined to AE import,
    /// which is not present yet).
    pub fn split_at(&self, t: Rational) -> Option<(Retime, Retime)> {
        let d = self.boundaries.last()?.t;
        if t <= Rational::ZERO || t >= d {
            return None;
        }
        // The segment [t_i, t_{i+1}) containing the cut.
        let i = (0..self.segments.len())
            .find(|&i| self.boundaries[i].t <= t && t < self.boundaries[i + 1].t)?;
        // Fast path: a Linear-ease Rate segment stays two Rate segments.
        match &self.segments[i] {
            RetimeSegment::Rate(seg) if seg.ease == Ease::Linear => {
                return self.split_linear_rate(i, t, seg.v0, seg.v1);
            }
            _ => {}
        }
        self.split_general(i, t)
    }

    /// §5.3 general case: replace segment `i` with its §5.1 polynomial-Map
    /// decomposition, split the piece containing `t` exactly (both halves are
    /// polynomial MapSegments), and partition at the new boundary. Source
    /// positions are absolute and never shift; only local times shift so the
    /// right half starts at zero.
    fn split_general(&self, i: usize, t: Rational) -> Option<(Retime, Retime)> {
        let pieces = self.split_pieces(i)?;
        let third = Rational::new(1, 3).ok()?;
        let mut eb = self.boundaries[..=i].to_vec();
        let mut es = self.segments[..i].to_vec();
        let mut lo = self.boundaries[i].clone();
        let mut cut_idx = None;
        for (pmap, phi) in pieces {
            if cut_idx.is_none() && lo.t <= t && t < phi.t {
                let (s_cut, v_cut) = hermite_at(&lo, &phi, pmap.m0, pmap.m1, t).ok()?;
                es.push(RetimeSegment::Map(MapSegment::new(
                    pmap.m0, v_cut, third, third,
                )));
                eb.push(Boundary::new(t, s_cut));
                cut_idx = Some(eb.len() - 1);
                es.push(RetimeSegment::Map(MapSegment::new(
                    v_cut, pmap.m1, third, third,
                )));
            } else {
                es.push(RetimeSegment::Map(pmap));
            }
            eb.push(phi.clone());
            lo = phi;
        }
        let cut_idx = cut_idx?;
        es.extend(self.segments[i + 1..].iter().cloned());
        eb.extend(self.boundaries[i + 2..].iter().cloned());

        let mut left = Retime {
            boundaries: eb[..=cut_idx].to_vec(),
            segments: es[..cut_idx].to_vec(),
            allow_reverse: self.allow_reverse,
            interpolation: self.interpolation.clone(),
            extra: serde_json::Map::new(),
        };
        let mut rb = vec![Boundary::new(Rational::ZERO, eb[cut_idx].s)];
        for b in &eb[cut_idx + 1..] {
            rb.push(Boundary::new(b.t.checked_sub(t).ok()?, b.s));
        }
        let mut right = Retime {
            boundaries: rb,
            segments: es[cut_idx..].to_vec(),
            allow_reverse: self.allow_reverse,
            interpolation: self.interpolation.clone(),
            extra: serde_json::Map::new(),
        };
        // recompute keeps any downstream Rate boundaries consistent; MapSegment
        // boundaries (which we set exactly) are authoritative and untouched.
        left.recompute_boundaries().ok()?;
        right.recompute_boundaries().ok()?;
        Some((left, right))
    }

    /// The §5.1 polynomial-Map decomposition of segment `i`, as a list of
    /// (segment, upper boundary); the lower boundary of the first is
    /// `self.boundaries[i]`. One piece for Slow/Fast (and an already-polynomial
    /// Map), two for Smooth/Sharp (split at u = 1/2, exact). None for a Linear
    /// Rate segment (the caller keeps those as Rate) or a general-influence Map.
    fn split_pieces(&self, i: usize) -> Option<Vec<(MapSegment, Boundary)>> {
        let lo = self.boundaries[i].clone();
        let hi = self.boundaries[i + 1].clone();
        let third = Rational::new(1, 3).ok()?;
        match &self.segments[i] {
            RetimeSegment::Rate(seg) => match seg.ease {
                Ease::Linear => None,
                Ease::Slow | Ease::Fast => {
                    Some(vec![(MapSegment::new(seg.v0, seg.v1, third, third), hi)])
                }
                Ease::Smooth | Ease::Sharp => {
                    let half = Rational::new(1, 2).ok()?;
                    let d_seg = hi.t.checked_sub(lo.t).ok()?;
                    let mid_t = lo.t.checked_add(d_seg.checked_mul(half).ok()?).ok()?;
                    // e(1/2) = 1/2 for both, so v(1/2) is the mean of the speeds.
                    let vmid = seg.v0.checked_add(seg.v1).ok()?.checked_mul(half).ok()?;
                    // E(1/2): Smooth 1/12, Sharp 1/6 (docs/04-RETIMING.md §4.1).
                    let e_half = if seg.ease == Ease::Smooth {
                        Rational::new(1, 12)
                    } else {
                        Rational::new(1, 6)
                    }
                    .ok()?;
                    // s(1/2) = s_i + d·[v0·1/2 + (v1 − v0)·E(1/2)]  (§4.1).
                    let s_mid = exact_or_flick(
                        || {
                            let dv = seg.v1.checked_sub(seg.v0)?;
                            let inner = seg
                                .v0
                                .checked_mul(half)?
                                .checked_add(dv.checked_mul(e_half)?)?;
                            lo.s.checked_add(d_seg.checked_mul(inner)?)
                        },
                        lo.s.to_f64()
                            + d_seg.to_f64()
                                * (seg.v0.to_f64() * 0.5
                                    + (seg.v1.to_f64() - seg.v0.to_f64()) * e_half.to_f64()),
                    )
                    .ok()?;
                    let mid = Boundary::new(mid_t, s_mid);
                    Some(vec![
                        (MapSegment::new(seg.v0, vmid, third, third), mid),
                        (MapSegment::new(vmid, seg.v1, third, third), hi),
                    ])
                }
            },
            RetimeSegment::Map(seg) => {
                if is_one_third(seg.b0) && is_one_third(seg.b1) {
                    Some(vec![(seg.clone(), hi)])
                } else {
                    None
                }
            }
        }
    }

    /// The Linear-ease Rate split: both halves stay Rate, exact.
    fn split_linear_rate(
        &self,
        i: usize,
        t: Rational,
        v0: Rational,
        v1: Rational,
    ) -> Option<(Retime, Retime)> {
        let ti = self.boundaries[i].t;
        let ti1 = self.boundaries[i + 1].t;
        let seg_d = ti1.checked_sub(ti).ok()?;
        let d_left = t.checked_sub(ti).ok()?;
        // Speed at the cut: v0 + (v1−v0)·(d_left / seg_d).
        let u = d_left.checked_div(seg_d).ok()?;
        let dv = v1.checked_sub(v0).ok()?;
        let v_at = v0.checked_add(dv.checked_mul(u).ok()?).ok()?;

        // Left: original boundaries/segments up to i, then a cut boundary and
        // the left half of segment i (Rate v0→v_at). recompute fills the s.
        let mut lb = self.boundaries[..=i].to_vec();
        lb.push(Boundary::new(t, self.boundaries[i].s));
        let mut ls = self.segments[..i].to_vec();
        ls.push(RetimeSegment::Rate(RateSegment::new(
            v0,
            v_at,
            Ease::Linear,
        )));
        let mut left = Retime {
            boundaries: lb,
            segments: ls,
            allow_reverse: self.allow_reverse,
            interpolation: self.interpolation.clone(),
            extra: serde_json::Map::new(),
        };
        left.recompute_boundaries().ok()?;

        // Right: a fresh 0 boundary, then the later boundaries shifted by −t;
        // the right half of segment i (Rate v_at→v1) then the rest.
        let mut rb = vec![Boundary::new(Rational::ZERO, left.boundaries.last()?.s)];
        for b in &self.boundaries[i + 1..] {
            rb.push(Boundary::new(b.t.checked_sub(t).ok()?, b.s));
        }
        let mut rs = vec![RetimeSegment::Rate(RateSegment::new(
            v_at,
            v1,
            Ease::Linear,
        ))];
        rs.extend(self.segments[i + 1..].iter().cloned());
        let mut right = Retime {
            boundaries: rb,
            segments: rs,
            allow_reverse: self.allow_reverse,
            interpolation: self.interpolation.clone(),
            extra: serde_json::Map::new(),
        };
        right.recompute_boundaries().ok()?;
        Some((left, right))
    }

    /// Delete the interior boundary at index `j` (1 ≤ j ≤ n − 1), merging its
    /// two neighbouring segments into one that spans the pinned outer
    /// boundaries (docs/04-RETIMING.md §5.4). The merged segment is a
    /// MapSegment carrying the outer one-sided speeds and influences — the
    /// interior detail is smoothly discarded — unless both neighbours are
    /// Linear Rate segments and a single Linear Rate reproduces the outer
    /// source advance exactly, in which case it stays a Rate segment. The outer
    /// boundaries never move and segments beyond them never change. None if `j`
    /// is not an interior boundary.
    pub fn merge_boundary(&self, j: usize) -> Option<Retime> {
        if j == 0 || j + 1 >= self.boundaries.len() {
            return None;
        }
        let third = Rational::new(1, 3).ok()?;
        let (left, right) = (&self.segments[j - 1], &self.segments[j]);
        let (lo, hi) = (&self.boundaries[j - 1], &self.boundaries[j + 1]);
        // Outer one-sided speeds and influences (a Rate segment's ends are
        // 1/3-influence by construction — the polynomial subclass).
        let (m0, b0) = match left {
            RetimeSegment::Rate(s) => (s.v0, third),
            RetimeSegment::Map(s) => (s.m0, s.b0),
        };
        let (m1, b1) = match right {
            RetimeSegment::Rate(s) => (s.v1, third),
            RetimeSegment::Map(s) => (s.m1, s.b1),
        };
        // Special case: two Linear Rate neighbours whose single Linear Rate
        // reproduces the outer Δs exactly may stay Rate.
        let stays_rate = match (left, right) {
            (RetimeSegment::Rate(l), RetimeSegment::Rate(r))
                if l.ease == Ease::Linear && r.ease == Ease::Linear =>
            {
                let d = hi.t.checked_sub(lo.t).ok()?;
                let ds = hi.s.checked_sub(lo.s).ok()?;
                rate_advance(d, l.v0, r.v1, Ease::Linear).ok()? == ds
            }
            _ => false,
        };
        let merged = if stays_rate {
            RetimeSegment::Rate(RateSegment::new(m0, m1, Ease::Linear))
        } else {
            RetimeSegment::Map(MapSegment::new(m0, m1, b0, b1))
        };

        let mut boundaries = self.boundaries.clone();
        boundaries.remove(j);
        let mut segments = self.segments.clone();
        segments.remove(j);
        segments[j - 1] = merged;
        let mut out = Retime {
            boundaries,
            segments,
            allow_reverse: self.allow_reverse,
            interpolation: self.interpolation.clone(),
            extra: serde_json::Map::new(),
        };
        out.recompute_boundaries().ok()?;
        Some(out)
    }

    /// This retime with every source position shifted by `delta` — the slip
    /// edit (docs/04-RETIMING.md §8.2): the source slides under a fixed clip, so
    /// the map from local time to source is translated while its shape is
    /// untouched. Local times, speeds and influences are unchanged; only the
    /// boundary source positions move. None on rational overflow.
    pub fn shift_source(&self, delta: Rational) -> Option<Retime> {
        let mut r = self.clone();
        for b in &mut r.boundaries {
            b.s = b.s.checked_add(delta).ok()?;
        }
        Some(r)
    }

    /// Insert a freeze — a zero-speed hold of `duration` — at local time `at`,
    /// keeping the clip's own duration fixed (docs/04-RETIMING.md §7.3). The
    /// segment covering `at` is split there, a `Rate { 0, 0, Linear }` hold is
    /// inserted, every later boundary shifts that far further along in local
    /// time, and the map is cropped back to the original domain — so the tail
    /// may be pushed off the end and newly overrun (which renders as a hold).
    /// None when `at` is not strictly inside the domain, `duration` is not
    /// positive, or the split at `at` is unsupported (a general-influence Map).
    pub fn insert_freeze(&self, at: Rational, duration: Rational) -> Option<Retime> {
        let d_orig = self.boundaries.last()?.t;
        if at <= Rational::ZERO || at >= d_orig || duration <= Rational::ZERO {
            return None;
        }
        let (left, right) = self.split_at(at)?;
        let s_at = left.boundaries.last()?.s;
        let freeze_end = at.checked_add(duration).ok()?;
        let mut boundaries = left.boundaries.clone();
        boundaries.push(Boundary::new(freeze_end, s_at));
        // Right's local times shift by the freeze length; its first boundary
        // (0, s_at) is the freeze end we just pushed, so skip it.
        for b in right.boundaries.iter().skip(1) {
            boundaries.push(Boundary::new(b.t.checked_add(freeze_end).ok()?, b.s));
        }
        let mut segments = left.segments.clone();
        segments.push(RetimeSegment::Rate(RateSegment::new(
            Rational::ZERO,
            Rational::ZERO,
            Ease::Linear,
        )));
        segments.extend(right.segments.iter().cloned());
        let mut assembled = Retime {
            boundaries,
            segments,
            allow_reverse: self.allow_reverse,
            interpolation: self.interpolation.clone(),
            extra: serde_json::Map::new(),
        };
        assembled.recompute_boundaries().ok()?;
        // Crop back to the original clip duration: keep [0, d_orig].
        let (cropped, _) = assembled.split_at(d_orig)?;
        Some(cropped)
    }

    /// Convert the MapSegment covering local time `t` to a RateSegment by the
    /// §5.2 fit, returning the updated retime and the maximum source-position
    /// drift in seconds — a nonzero drift means the conversion is inexact and
    /// the UI shows the "fitted" warning badge. The source advance Δs is pinned
    /// exactly (C0 is never sacrificed). None when `t` is outside the domain,
    /// the covering segment is already a Rate, or the map's speed cannot be
    /// followed by any single ease (an interior sign change, or going negative
    /// while reverse is disabled — docs/04-RETIMING.md §5.2 refusal).
    pub fn with_segment_as_rate(&self, t: Rational) -> Option<(Retime, f64)> {
        let i = self.segment_index_at(t)?;
        let RetimeSegment::Map(seg) = &self.segments[i] else {
            return None;
        };
        let (rate, drift) = fit_map_to_rate(
            seg,
            &self.boundaries[i],
            &self.boundaries[i + 1],
            self.allow_reverse,
        )?;
        let mut r = self.clone();
        r.segments[i] = RetimeSegment::Rate(rate);
        // The fitted rate reproduces Δs exactly, so recompute leaves every
        // boundary source position where it was.
        r.recompute_boundaries().ok()?;
        Some((r, drift))
    }

    /// Structural sanity (docs/04-RETIMING.md §3 invariants): n + 1
    /// boundaries for n segments, first boundary at local time zero,
    /// boundary times strictly increasing.
    pub fn validate(&self) -> Result<(), RetimeError> {
        if self.segments.is_empty() {
            return Err(RetimeError::InvalidStructure(
                "a retime needs at least one segment",
            ));
        }
        if self.boundaries.len() != self.segments.len() + 1 {
            return Err(RetimeError::InvalidStructure(
                "boundary count must be segment count plus one",
            ));
        }
        if self.boundaries[0].t != Rational::ZERO {
            return Err(RetimeError::InvalidStructure(
                "the first boundary must sit at local time zero",
            ));
        }
        if self
            .boundaries
            .windows(2)
            .any(|pair| pair[1].t <= pair[0].t)
        {
            return Err(RetimeError::InvalidStructure(
                "boundary times must strictly increase",
            ));
        }
        Ok(())
    }

    /// Re-derive every boundary source position downstream of a RateSegment,
    /// exactly (docs/04-RETIMING.md §4.1 boundary consistency):
    ///
    /// ```text
    /// s[i+1] = s[i] + d · [ v0 + (v1 − v0) · E(1) ]
    /// ```
    ///
    /// all in rational arithmetic, so repeated edits never accumulate drift.
    /// MapSegment boundaries are left untouched — a map's endpoints *are*
    /// its boundaries, so the stored `s` is already authoritative. Speeds
    /// respect the reverse gate (negative speeds count as zero while
    /// `allow_reverse` is off), keeping boundaries consistent with what
    /// `evaluate` renders.
    pub fn recompute_boundaries(&mut self) -> Result<(), RetimeError> {
        self.validate()?;
        for i in 0..self.segments.len() {
            if let RetimeSegment::Rate(seg) = &self.segments[i] {
                let (v0, v1) = clamped_speeds(seg, self.allow_reverse);
                let d = self.boundaries[i + 1].t.checked_sub(self.boundaries[i].t)?;
                let advance = rate_advance(d, v0, v1, seg.ease)?;
                self.boundaries[i + 1].s = add_with_flick_fallback(self.boundaries[i].s, advance)?;
            }
        }
        Ok(())
    }

    /// Resolve local time `t` (seconds) to a source time (docs/04-RETIMING.md
    /// §4.3). `t` is clamped into the local domain `[0, D]`; the result is
    /// deliberately *not* clamped to the source extent — that clamp is what
    /// defines overrun (§7) and happens at a later stage.
    ///
    /// Per-sample evaluation is f64 by design; the rational boundaries are
    /// the exact anchors it works from.
    pub fn evaluate(&self, t: f64) -> f64 {
        let Some((i, t)) = self.locate(t) else {
            // Structurally unusable store: hold the first known source
            // position rather than fault (engine crates never panic).
            return self.boundaries.first().map_or(0.0, |b| b.s.to_f64());
        };
        let (lo, hi) = (&self.boundaries[i], &self.boundaries[i + 1]);
        let (t0, t1) = (lo.t.to_f64(), hi.t.to_f64());
        let d = t1 - t0;
        if d <= 0.0 {
            return lo.s.to_f64();
        }
        match &self.segments[i] {
            RetimeSegment::Rate(seg) => {
                let u = ((t - t0) / d).clamp(0.0, 1.0);
                let (v0, v1) = clamped_speeds(seg, self.allow_reverse);
                let (v0, v1) = (v0.to_f64(), v1.to_f64());
                // f(t) = s_i + d·[v0·u + (v1 − v0)·E(u)]  (§4.1)
                lo.s.to_f64() + d * (v0 * u + (v1 - v0) * seg.ease.big_e(u))
            }
            RetimeSegment::Map(seg) => {
                let (x, y) = map_control_points(seg, lo, hi);
                bezier(&y, map_param_at(seg, &x, t))
            }
        }
    }

    /// Instantaneous speed df/dt at local time `t` (1.0 = 100%). For a
    /// RateSegment this is the speed profile itself, v0 + (v1 − v0)·e(u);
    /// for a MapSegment it is y′(u)/x′(u) (docs/04-RETIMING.md §4.2).
    pub fn speed_at(&self, t: f64) -> f64 {
        let Some((i, t)) = self.locate(t) else {
            return 0.0;
        };
        let (lo, hi) = (&self.boundaries[i], &self.boundaries[i + 1]);
        let (t0, t1) = (lo.t.to_f64(), hi.t.to_f64());
        let d = t1 - t0;
        if d <= 0.0 {
            return 0.0;
        }
        match &self.segments[i] {
            RetimeSegment::Rate(seg) => {
                let u = ((t - t0) / d).clamp(0.0, 1.0);
                let (v0, v1) = clamped_speeds(seg, self.allow_reverse);
                let (v0, v1) = (v0.to_f64(), v1.to_f64());
                v0 + (v1 - v0) * seg.ease.small_e(u)
            }
            RetimeSegment::Map(seg) => {
                let (x, y) = map_control_points(seg, lo, hi);
                let u = map_param_at(seg, &x, t);
                // x′ can legitimately touch zero at a 100%-influence handle;
                // the floor keeps the readout finite (speed is then simply
                // "very large", which is the truth of the curve there).
                bezier_deriv(&y, u) / bezier_deriv(&x, u).max(1e-12)
            }
        }
    }

    /// Clamp `t` into the local domain and find the segment containing it
    /// (binary search over the boundary list, §4.3). `None` means the store
    /// is structurally unusable and evaluation should degrade gracefully.
    fn locate(&self, t: f64) -> Option<(usize, f64)> {
        let first = self.boundaries.first()?;
        let last = self.boundaries.last()?;
        if self.segments.is_empty() || self.boundaries.len() != self.segments.len() + 1 {
            return None;
        }
        let t = t.clamp(first.t.to_f64(), last.t.to_f64());
        // Largest segment whose start boundary is ≤ t.
        let idx = self.boundaries.partition_point(|b| b.t.to_f64() <= t);
        Some((idx.saturating_sub(1).min(self.segments.len() - 1), t))
    }
}

/// Endpoint speeds with the reverse gate applied (docs/04-RETIMING.md §6.2):
/// while `allow_reverse` is off, negative speeds evaluate as zero. Every ease
/// is monotone, so clamping the endpoints clamps the whole profile (§4.1).
fn clamped_speeds(seg: &RateSegment, allow_reverse: bool) -> (Rational, Rational) {
    if allow_reverse {
        (seg.v0, seg.v1)
    } else {
        let floor = |v: Rational| if v.is_negative() { Rational::ZERO } else { v };
        (floor(seg.v0), floor(seg.v1))
    }
}

/// `a + b`, exact. On i64 overflow, follow the §4.1 precision policy: redo
/// the sum in i128 and round to the flick grid (1/705 600 000 s) — a
/// sub-nanosecond rounding reachable only under pathological editing.
fn add_with_flick_fallback(a: Rational, b: Rational) -> Result<Rational, RetimeError> {
    match a.checked_add(b) {
        Ok(v) => Ok(v),
        Err(TimeError::Overflow) => {
            let num = i128::from(a.num()) * i128::from(b.den())
                + i128::from(b.num()) * i128::from(a.den());
            let den = i128::from(a.den()) * i128::from(b.den());
            Rational::from_f64_on_grid(num as f64 / den as f64, Rational::FLICK_DEN)
                .map_err(RetimeError::from)
        }
        Err(e) => Err(RetimeError::from(e)),
    }
}

/// The exact source advance of one RateSegment: `d · [v0 + (v1 − v0)·E(1)]`
/// (docs/04-RETIMING.md §4.1). On i64 overflow, fall back to wide floating
/// point rounded onto the flick grid, per the same precision policy.
fn rate_advance(
    d: Rational,
    v0: Rational,
    v1: Rational,
    ease: Ease,
) -> Result<Rational, RetimeError> {
    let e1 = ease.e_at_1();
    let exact = v1
        .checked_sub(v0)
        .and_then(|dv| dv.checked_mul(e1))
        .and_then(|weighted| v0.checked_add(weighted))
        .and_then(|inner| d.checked_mul(inner));
    match exact {
        Ok(v) => Ok(v),
        Err(TimeError::Overflow) => {
            let approx = d.to_f64() * (v0.to_f64() + (v1.to_f64() - v0.to_f64()) * e1.to_f64());
            Rational::from_f64_on_grid(approx, Rational::FLICK_DEN).map_err(RetimeError::from)
        }
        Err(e) => Err(RetimeError::from(e)),
    }
}

/// The §4.2 control points for one MapSegment between its two boundaries,
/// in f64 for evaluation:
///
/// ```text
/// P0 = (t0,          s0)
/// P1 = (t0 + b0·d,   s0 + m0·b0·d)
/// P2 = (t1 − b1·d,   s1 − m1·b1·d)
/// P3 = (t1,          s1)
/// ```
fn map_control_points(seg: &MapSegment, lo: &Boundary, hi: &Boundary) -> ([f64; 4], [f64; 4]) {
    let (t0, s0) = (lo.t.to_f64(), lo.s.to_f64());
    let (t1, s1) = (hi.t.to_f64(), hi.s.to_f64());
    let d = t1 - t0;
    let (m0, m1) = (seg.m0.to_f64(), seg.m1.to_f64());
    let (b0, b1) = (seg.b0.to_f64(), seg.b1.to_f64());
    (
        [t0, t0 + b0 * d, t1 - b1 * d, t1],
        [s0, s0 + m0 * b0 * d, s1 - m1 * b1 * d, s1],
    )
}

/// Find the bezier parameter u with x(u) = t. The polynomial subclass
/// (b0 = b1 = 1/3, §4.2) makes x(u) linear, so u falls straight out; the
/// general case root-solves.
fn map_param_at(seg: &MapSegment, x: &[f64; 4], t: f64) -> f64 {
    if is_one_third(seg.b0) && is_one_third(seg.b1) {
        ((t - x[0]) / (x[3] - x[0])).clamp(0.0, 1.0)
    } else {
        solve_u(x, t)
    }
}

fn is_one_third(r: Rational) -> bool {
    r.num() == 1 && r.den() == 3
}

/// Cubic bezier over four scalar control points (Bernstein form).
fn bezier(p: &[f64; 4], u: f64) -> f64 {
    let w = 1.0 - u;
    w * w * w * p[0] + 3.0 * w * w * u * p[1] + 3.0 * w * u * u * p[2] + u * u * u * p[3]
}

fn bezier_deriv(p: &[f64; 4], u: f64) -> f64 {
    let w = 1.0 - u;
    3.0 * w * w * (p[1] - p[0]) + 6.0 * w * u * (p[2] - p[1]) + 3.0 * u * u * (p[3] - p[2])
}

/// Solve x(u) = t by Newton inside a shrinking bracket — the binding
/// algorithm of docs/impl/keyframe-eval.md §2 (the same solver as
/// `anim::CubicSpan::solve_u`): fast like Newton, and mathematically unable
/// to escape [0, 1] because a bisection bracket always backs it up. Run to
/// the ≤ 2⁻⁴⁸ relative tolerance of docs/04-RETIMING.md §4.3; 48 iterations
/// guarantee that even in the pure-bisection worst case (x′ = 0 flat spots
/// at 100%-influence handles), and Newton normally exits in a handful.
fn solve_u(x: &[f64; 4], t: f64) -> f64 {
    let (x0, x3) = (x[0], x[3]);
    if x3 <= x0 {
        return 0.0;
    }
    let tol = (x3 - x0) * 2.0_f64.powi(-48);
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    let mut u = ((t - x0) / (x3 - x0)).clamp(0.0, 1.0); // x ≈ identity guess
    for _ in 0..48 {
        let xu = bezier(x, u);
        if (xu - t).abs() <= tol {
            break;
        }
        if xu < t {
            lo = u;
        } else {
            hi = u;
        }
        let dxu = bezier_deriv(x, u);
        let newton = u - (xu - t) / dxu;
        u = if dxu > 1e-12 && newton > lo && newton < hi {
            newton
        } else {
            0.5 * (lo + hi)
        };
    }
    u
}

/// Take an exact rational computation, or — only if it overflows i64 — the
/// f64 `approx` rounded onto the flick grid, per the §4.1 precision policy.
/// This keeps splitting exact in every ordinary case and sub-nanosecond in
/// the pathological one, never faulting (engine crates never panic).
fn exact_or_flick(
    exact: impl FnOnce() -> Result<Rational, TimeError>,
    approx: f64,
) -> Result<Rational, RetimeError> {
    match exact() {
        Ok(v) => Ok(v),
        Err(TimeError::Overflow) => {
            Rational::from_f64_on_grid(approx, Rational::FLICK_DEN).map_err(RetimeError::from)
        }
        Err(e) => Err(RetimeError::from(e)),
    }
}

/// `c0 + c1·u + c2·u² + c3·u³` with exact integer coefficients — the basis
/// evaluation behind the rational cubic-Hermite split.
fn hpoly(c: [i64; 4], u: Rational, u2: Rational, u3: Rational) -> Result<Rational, TimeError> {
    Rational::new(c[0], 1)?
        .checked_add(Rational::new(c[1], 1)?.checked_mul(u)?)?
        .checked_add(Rational::new(c[2], 1)?.checked_mul(u2)?)?
        .checked_add(Rational::new(c[3], 1)?.checked_mul(u3)?)
}

/// Exact source position and speed at local time `t` inside a polynomial
/// (b0 = b1 = 1/3) MapSegment spanning `lo`..`hi` with endpoint speeds
/// `m0`, `m1` — the cubic Hermite of docs/04-RETIMING.md §4.2 polynomial
/// subclass. `t` must lie in (lo.t, hi.t). Returns `(s, v)`, each exact or, on
/// i64 overflow, rounded onto the flick grid (§4.1 precision policy).
fn hermite_at(
    lo: &Boundary,
    hi: &Boundary,
    m0: Rational,
    m1: Rational,
    t: Rational,
) -> Result<(Rational, Rational), RetimeError> {
    // Hermite basis in u: h00 = 1 − 3u² + 2u³, h10 = u − 2u² + u³,
    // h01 = 3u² − 2u³, h11 = u³ − u²; and their u-derivatives for the speed.
    let s_exact = || -> Result<Rational, TimeError> {
        let d = hi.t.checked_sub(lo.t)?;
        let u = t.checked_sub(lo.t)?.checked_div(d)?;
        let (u2, u3) = (u.checked_mul(u)?, u.checked_mul(u)?.checked_mul(u)?);
        let (dm0, dm1) = (d.checked_mul(m0)?, d.checked_mul(m1)?);
        hpoly([1, 0, -3, 2], u, u2, u3)?
            .checked_mul(lo.s)?
            .checked_add(hpoly([0, 1, -2, 1], u, u2, u3)?.checked_mul(dm0)?)?
            .checked_add(hpoly([0, 0, 3, -2], u, u2, u3)?.checked_mul(hi.s)?)?
            .checked_add(hpoly([0, 0, -1, 1], u, u2, u3)?.checked_mul(dm1)?)
    };
    let v_exact = || -> Result<Rational, TimeError> {
        let d = hi.t.checked_sub(lo.t)?;
        let u = t.checked_sub(lo.t)?.checked_div(d)?;
        let (u2, u3) = (u.checked_mul(u)?, u.checked_mul(u)?.checked_mul(u)?);
        let (dm0, dm1) = (d.checked_mul(m0)?, d.checked_mul(m1)?);
        // dy/du, then v = (dy/du)/d since t = t0 + d·u.
        let dydu = hpoly([0, -6, 6, 0], u, u2, u3)?
            .checked_mul(lo.s)?
            .checked_add(hpoly([1, -4, 3, 0], u, u2, u3)?.checked_mul(dm0)?)?
            .checked_add(hpoly([0, 6, -6, 0], u, u2, u3)?.checked_mul(hi.s)?)?
            .checked_add(hpoly([0, -2, 3, 0], u, u2, u3)?.checked_mul(dm1)?)?;
        dydu.checked_div(d)
    };
    // f64 fallbacks for the overflow branch.
    let df = hi.t.to_f64() - lo.t.to_f64();
    let uf = (t.to_f64() - lo.t.to_f64()) / df;
    let (s0, s1, m0f, m1f) = (lo.s.to_f64(), hi.s.to_f64(), m0.to_f64(), m1.to_f64());
    let hp = |c: [i64; 4]| {
        c[0] as f64 + c[1] as f64 * uf + c[2] as f64 * uf * uf + c[3] as f64 * uf * uf * uf
    };
    let s_approx = hp([1, 0, -3, 2]) * s0
        + hp([0, 1, -2, 1]) * df * m0f
        + hp([0, 0, 3, -2]) * s1
        + hp([0, 0, -1, 1]) * df * m1f;
    let dydu_approx = hp([0, -6, 6, 0]) * s0
        + hp([1, -4, 3, 0]) * df * m0f
        + hp([0, 6, -6, 0]) * s1
        + hp([0, -2, 3, 0]) * df * m1f;
    Ok((
        exact_or_flick(s_exact, s_approx)?,
        exact_or_flick(v_exact, dydu_approx / df)?,
    ))
}

/// Integrate `f` over [0, 1] by composite Simpson (N = 64). The §5.2 integrands
/// are low-degree polynomials, so this is effectively exact.
fn integrate(f: impl Fn(f64) -> f64) -> f64 {
    const N: usize = 64;
    let h = 1.0 / N as f64;
    let mut sum = f(0.0) + f(1.0);
    for i in 1..N {
        let w = if i % 2 == 0 { 2.0 } else { 4.0 };
        sum += w * f(f64::from(i as u32) * h);
    }
    sum * h / 3.0
}

/// Solve the 3×3 system `m·x = b` by Cramer's rule; None if near-singular.
fn solve_3x3(m: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = |a: &[[f64; 3]; 3]| {
        a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
            - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
    };
    let d = det(&m);
    if d.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for (c, slot) in out.iter_mut().enumerate() {
        let mut a = m;
        for r in 0..3 {
            a[r][c] = b[r];
        }
        *slot = det(&a) / d;
    }
    Some(out)
}

/// Fit a MapSegment to a RateSegment (docs/04-RETIMING.md §5.2): for each ease
/// shape solve the constrained least squares `min ∫(v_fit − v_map)² du` subject
/// to reproducing the mean speed (so Δs is pinned), keep the least-residual
/// shape, then set `v1` rationally from that constraint so the source advance
/// is reproduced *exactly*. Returns the rate and the maximum source drift in
/// seconds (for the warning badge). None when no single ease can follow the
/// map's speed: an interior sign change, or negative speed while reverse is off.
fn fit_map_to_rate(
    seg: &MapSegment,
    lo: &Boundary,
    hi: &Boundary,
    allow_reverse: bool,
) -> Option<(RateSegment, f64)> {
    let (x, y) = map_control_points(seg, lo, hi);
    let (t0, s0) = (lo.t.to_f64(), lo.s.to_f64());
    let d = (hi.t.to_f64() - t0).max(1e-12);
    let vmap = |u: f64| bezier_deriv(&y, u) / bezier_deriv(&x, u).max(1e-12);
    // A single RateSegment cannot change speed sign, nor go negative while the
    // reverse gate is off — refuse those (§5.2 step 4).
    let (mut mn, mut mx) = (f64::INFINITY, f64::NEG_INFINITY);
    for k in 0..=64 {
        let v = vmap(f64::from(k) / 64.0);
        mn = mn.min(v);
        mx = mx.max(v);
    }
    if (mn < 0.0 && mx > 0.0) || (!allow_reverse && mn < 0.0) {
        return None;
    }
    let c_avg = (hi.s.to_f64() - s0) / d; // mean speed = Δs / d
    let mut best: Option<(Ease, f64, f64)> = None; // (ease, v0, drift)
    let mut best_residual = f64::INFINITY;
    for ease in [
        Ease::Linear,
        Ease::Slow,
        Ease::Fast,
        Ease::Smooth,
        Ease::Sharp,
    ] {
        let phi0 = |u: f64| 1.0 - ease.small_e(u);
        let phi1 = |u: f64| ease.small_e(u);
        let (p0, p1) = (integrate(phi0), integrate(phi1));
        let m = [
            [
                integrate(|u| phi0(u) * phi0(u)),
                integrate(|u| phi0(u) * phi1(u)),
                p0,
            ],
            [
                integrate(|u| phi0(u) * phi1(u)),
                integrate(|u| phi1(u) * phi1(u)),
                p1,
            ],
            [p0, p1, 0.0],
        ];
        let b = [
            integrate(|u| phi0(u) * vmap(u)),
            integrate(|u| phi1(u) * vmap(u)),
            c_avg,
        ];
        let Some([v0, v1, _]) = solve_3x3(m, b) else {
            continue;
        };
        let residual = integrate(|u| {
            let vf = v0 + (v1 - v0) * ease.small_e(u);
            (vf - vmap(u)).powi(2)
        });
        let mut drift = 0.0_f64;
        for k in 0..=64 {
            let u = f64::from(k) / 64.0;
            let f_fit = s0 + d * (v0 * u + (v1 - v0) * ease.big_e(u));
            let f_map = bezier(&y, map_param_at(seg, &x, t0 + d * u));
            drift = drift.max((f_fit - f_map).abs());
        }
        if residual < best_residual {
            best_residual = residual;
            best = Some((ease, v0, drift));
        }
    }
    let (ease, v0_f, drift) = best?;
    // Pin Δs exactly: a rational v0 from the fit, v1 solving the C0 constraint
    // v0 + (v1 − v0)·E(1) = Δs/d.
    let v0 = Rational::from_f64_on_grid(v0_f, Rational::FLICK_DEN).ok()?;
    let c_rat =
        hi.s.checked_sub(lo.s)
            .ok()?
            .checked_div(hi.t.checked_sub(lo.t).ok()?)
            .ok()?;
    let v1 = v0
        .checked_add(
            c_rat
                .checked_sub(v0)
                .ok()?
                .checked_div(ease.e_at_1())
                .ok()?,
        )
        .ok()?;
    Some((RateSegment::new(v0, v1, ease), drift))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    #[test]
    fn speed_keyframes_build_and_round_trip() {
        // 100% for [0,2], then ramp up to 200% by t=4.
        let keys = [
            (rat(0, 1), rat(1, 1)),
            (rat(2, 1), rat(1, 1)),
            (rat(4, 1), rat(2, 1)),
        ];
        let r = Retime::from_speed_keyframes(rat(0, 1), &keys).unwrap();
        assert_eq!(r.speed_keyframes().unwrap(), keys.to_vec());
        // 1× for the first 2s ⇒ source time 2 at t = 2.
        assert!((r.evaluate(2.0) - 2.0).abs() < 1e-9);
        // Halfway up the 1→2 ramp at t = 3 ⇒ instantaneous speed 1.5.
        assert!((r.speed_at(3.0) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn from_speed_keyframes_rejects_bad_input() {
        // Fewer than two keys.
        assert!(Retime::from_speed_keyframes(rat(0, 1), &[(rat(0, 1), rat(1, 1))]).is_none());
        // First key not at local time zero.
        assert!(Retime::from_speed_keyframes(
            rat(0, 1),
            &[(rat(1, 1), rat(1, 1)), (rat(2, 1), rat(1, 1))]
        )
        .is_none());
        // Times not strictly increasing.
        assert!(Retime::from_speed_keyframes(
            rat(0, 1),
            &[(rat(0, 1), rat(1, 1)), (rat(0, 1), rat(2, 1))]
        )
        .is_none());
    }

    #[test]
    fn value_keyframes_build_and_round_trip() {
        // Value lens: hold source 0 at t=0, be at source 1 by t=2 (½× so far),
        // then jump to source 5 by t=4 (2× over that span).
        let keys = [
            (rat(0, 1), rat(0, 1)),
            (rat(2, 1), rat(1, 1)),
            (rat(4, 1), rat(5, 1)),
        ];
        let r = Retime::from_value_keyframes(&keys).unwrap();
        // The boundaries reproduce the value keys exactly (no drift).
        assert_eq!(r.value_keyframes(), keys.to_vec());
        // Source time passes through every keyframe.
        assert!((r.evaluate(0.0) - 0.0).abs() < 1e-9);
        assert!((r.evaluate(2.0) - 1.0).abs() < 1e-9);
        assert!((r.evaluate(4.0) - 5.0).abs() < 1e-9);
        // Constant speed within a span: ½× on [0,2], 2× on [2,4].
        assert!((r.speed_at(1.0) - 0.5).abs() < 1e-9);
        assert!((r.speed_at(3.0) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn from_value_keyframes_allows_reverse_and_rejects_bad_input() {
        // A value key that steps source backwards opens the reverse gate.
        let back = Retime::from_value_keyframes(&[(rat(0, 1), rat(2, 1)), (rat(1, 1), rat(0, 1))])
            .unwrap();
        assert!(back.allow_reverse);
        assert!((back.speed_at(0.5) - -2.0).abs() < 1e-9);
        // Fewer than two keys, first key off zero, non-increasing times: all None.
        assert!(Retime::from_value_keyframes(&[(rat(0, 1), rat(0, 1))]).is_none());
        assert!(
            Retime::from_value_keyframes(&[(rat(1, 1), rat(0, 1)), (rat(2, 1), rat(1, 1))])
                .is_none()
        );
        assert!(
            Retime::from_value_keyframes(&[(rat(0, 1), rat(0, 1)), (rat(0, 1), rat(1, 1))])
                .is_none()
        );
    }

    #[test]
    fn source_keyframes_evaluate_like_a_transform_property() {
        // The whole point of K-078: a Time Remap built from bezier keyframes
        // must render bit-for-bit like the same keys on a transform property.
        use crate::anim::{Keyframe, SideInterp};
        let keys = vec![
            Keyframe {
                time: rat(0, 1),
                value: 0.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Bezier {
                    speed: 0.0,
                    influence: 1.0 / 3.0,
                }, // easy-ease out
            },
            Keyframe {
                time: rat(2, 1),
                value: 3.0,
                interp_in: SideInterp::Bezier {
                    speed: 4.0,
                    influence: 0.6,
                },
                interp_out: SideInterp::Bezier {
                    speed: 4.0,
                    influence: 0.4,
                },
            },
            Keyframe {
                time: rat(5, 1),
                value: 1.0,
                interp_in: SideInterp::Bezier {
                    speed: 0.0,
                    influence: 1.0 / 3.0,
                },
                interp_out: SideInterp::Linear,
            },
        ];
        let r = Retime::from_source_keyframes(&keys).unwrap();
        for i in 0..=50 {
            let t = 5.0 * i as f64 / 50.0;
            let want = crate::anim::evaluate(&keys, t).unwrap();
            assert!(
                (r.evaluate(t) - want).abs() < 1e-6,
                "at t={t}: retime {} vs property {want}",
                r.evaluate(t)
            );
        }
    }

    #[test]
    fn source_keyframes_round_trip_their_tangents() {
        // from_source_keyframes → source_keyframes returns the same tangents
        // (within flick-grid rounding), so opening the graph on a curve you just
        // committed shows the handles exactly where you left them.
        use crate::anim::SideInterp;
        let keys = Retime::from_source_keyframes(&[
            crate::anim::Keyframe {
                time: rat(0, 1),
                value: 0.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Bezier {
                    speed: 1.5,
                    influence: 0.5,
                },
            },
            crate::anim::Keyframe {
                time: rat(3, 1),
                value: 2.0,
                interp_in: SideInterp::Bezier {
                    speed: -0.5,
                    influence: 0.25,
                },
                interp_out: SideInterp::Linear,
            },
        ])
        .unwrap()
        .source_keyframes();
        assert_eq!(keys.len(), 2);
        match keys[0].interp_out {
            SideInterp::Bezier { speed, influence } => {
                assert!((speed - 1.5).abs() < 1e-6 && (influence - 0.5).abs() < 1e-6);
            }
            other => panic!("expected bezier out, got {other:?}"),
        }
        match keys[1].interp_in {
            SideInterp::Bezier { speed, influence } => {
                assert!((speed - -0.5).abs() < 1e-6 && (influence - 0.25).abs() < 1e-6);
            }
            other => panic!("expected bezier in, got {other:?}"),
        }
    }

    #[test]
    fn value_keyframes_read_any_store() {
        // Even a store the *speed* lens can't describe (an eased ramp) still
        // yields value keys — every boundary carries an exact source position.
        let eased = Retime::single_ramp(rat(4, 1), rat(0, 1), rat(1, 1), rat(2, 1), Ease::Smooth);
        assert!(eased.speed_keyframes().is_none());
        let vks = eased.value_keyframes();
        assert_eq!(vks.len(), 2);
        assert_eq!(vks[0], (rat(0, 1), rat(0, 1)));
    }

    #[test]
    fn overrun_marks_where_the_clip_runs_out_of_source() {
        // 2× over a 4 s clip from source 0: source runs 0→8, so 4 s of source
        // is used up at local time 2 s.
        let r = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(2, 1));
        let t = r.overrun_local_time(rat(4, 1)).unwrap();
        assert!((t - 2.0).abs() < 1e-3, "overrun at {t}");
        // 1× over 4 s needs only 4 s of source; 10 s of media never runs out.
        let slow = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(1, 1));
        assert!(slow.overrun_local_time(rat(10, 1)).is_none());
        // Source-in already past the end → overruns from the very start.
        let past = Retime::identity(rat(4, 1), rat(5, 1));
        assert_eq!(past.overrun_local_time(rat(3, 1)), Some(0.0));
    }

    #[test]
    fn speed_keyframes_none_for_eased_store() {
        // An eased ramp is a graph-editor store, not plain keys.
        let eased = Retime::single_ramp(rat(4, 1), rat(0, 1), rat(1, 1), rat(2, 1), Ease::Smooth);
        assert!(eased.speed_keyframes().is_none());
    }

    /// Build a store from (t, s) boundary pairs and segments.
    fn store(bounds: &[(Rational, Rational)], segments: Vec<RetimeSegment>) -> Retime {
        Retime {
            boundaries: bounds.iter().map(|&(t, s)| Boundary::new(t, s)).collect(),
            segments,
            allow_reverse: false,
            interpolation: Interpolation::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn rate(v0: Rational, v1: Rational, ease: Ease) -> RetimeSegment {
        RetimeSegment::Rate(RateSegment::new(v0, v1, ease))
    }

    #[test]
    fn identity_is_a_straight_pass_through() {
        let r = Retime::identity(rat(4, 1), rat(3, 2));
        r.validate().unwrap();
        for t in [0.0, 0.25, 1.0, 2.5, 4.0] {
            assert!((r.evaluate(t) - (1.5 + t)).abs() < 1e-9, "t = {t}");
            assert!((r.speed_at(t) - 1.0).abs() < 1e-9, "speed at {t}");
        }
        // Out-of-domain requests clamp to the ends of the local domain.
        assert!((r.evaluate(-1.0) - 1.5).abs() < 1e-9);
        assert!((r.evaluate(10.0) - 5.5).abs() < 1e-9);
        // The end boundary is the exact rational sum.
        assert_eq!(r.boundaries[1].s, rat(11, 2));
    }

    #[test]
    fn double_speed_covers_twice_the_source() {
        let mut r = store(
            &[(rat(0, 1), rat(5, 1)), (rat(2, 1), rat(0, 1))],
            vec![rate(rat(2, 1), rat(2, 1), Ease::Linear)],
        );
        r.recompute_boundaries().unwrap();
        assert_eq!(r.boundaries[1].s, rat(9, 1)); // 5 + 2·2, exactly
        assert!((r.evaluate(0.5) - 6.0).abs() < 1e-9);
        assert!((r.evaluate(1.5) - 8.0).abs() < 1e-9);
        assert!((r.speed_at(1.0) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn constant_speed_evaluates_as_linear_from_source_in() {
        // Half speed from source 2s over a 4s layer: at lt the source is
        // 2 + 0.5·lt; the end boundary is exactly 2 + 0.5·4 = 4.
        let r = Retime::constant_speed(rat(4, 1), rat(2, 1), rat(1, 2));
        assert_eq!(r.boundaries[1].s, rat(4, 1));
        assert!((r.evaluate(0.0) - 2.0).abs() < 1e-9);
        assert!((r.evaluate(2.0) - 3.0).abs() < 1e-9);
        assert!((r.evaluate(4.0) - 4.0).abs() < 1e-9);
        assert!((r.speed_at(1.0) - 0.5).abs() < 1e-9);
        r.validate().unwrap();
    }

    #[test]
    fn split_reproduces_the_curve_for_linear_segments() {
        // Constant 2× from source 5s over [0,4], cut at 1.5.
        let r = Retime::constant_speed(rat(4, 1), rat(5, 1), rat(2, 1));
        let (l, rt) = r.split_at(rat(3, 2)).unwrap();
        // Left [0,1.5] matches the original there.
        assert!((l.evaluate(0.0) - r.evaluate(0.0)).abs() < 1e-9);
        assert!((l.evaluate(1.5) - r.evaluate(1.5)).abs() < 1e-9);
        // Right domain is [0,2.5]; right(x) == original(1.5 + x).
        assert!((rt.evaluate(0.0) - r.evaluate(1.5)).abs() < 1e-9);
        assert!((rt.evaluate(1.0) - r.evaluate(2.5)).abs() < 1e-9);
        assert!((rt.evaluate(2.5) - r.evaluate(4.0)).abs() < 1e-9);
        // The two halves share the cut's source position exactly (C0).
        assert_eq!(l.boundaries.last().unwrap().s, rt.boundaries[0].s);

        // A linear ramp splits and stays exact.
        let ramp = Retime::single_ramp(rat(4, 1), rat(0, 1), rat(1, 1), rat(3, 1), Ease::Linear);
        let (rl, rr) = ramp.split_at(rat(2, 1)).unwrap();
        assert!((rl.evaluate(2.0) - ramp.evaluate(2.0)).abs() < 1e-9);
        assert!((rr.evaluate(0.0) - ramp.evaluate(2.0)).abs() < 1e-9);
        assert!((rr.evaluate(2.0) - ramp.evaluate(4.0)).abs() < 1e-9);

        // Refuses cuts outside the open domain (eased and Map cuts are the
        // §5 tests below).
        assert!(r.split_at(rat(0, 1)).is_none());
        assert!(r.split_at(rat(4, 1)).is_none());
    }

    /// Sample both halves of a split against the original: left on [0, cut],
    /// right on [0, D − cut] compared to the original at cut + τ.
    fn assert_split_preserves_curve(orig: &Retime, cut: Rational, dur: f64) {
        let (l, r) = orig.split_at(cut).expect("split supported");
        l.validate().unwrap();
        r.validate().unwrap();
        let cutf = cut.to_f64();
        for k in 0..=25 {
            let f = f64::from(k) / 25.0;
            let tl = cutf * f;
            assert!(
                (l.evaluate(tl) - orig.evaluate(tl)).abs() < 1e-9,
                "left @ {tl}"
            );
            let tr = (dur - cutf) * f;
            assert!(
                (r.evaluate(tr) - orig.evaluate(cutf + tr)).abs() < 1e-9,
                "right @ {tr}"
            );
        }
        // C0 at the cut: the two halves share the exact source position.
        assert_eq!(l.boundaries.last().unwrap().s, r.boundaries[0].s);
        // Both halves are polynomial MapSegments (§5.3).
        assert!(l
            .segments
            .iter()
            .all(|s| matches!(s, RetimeSegment::Map(_))));
        assert!(r
            .segments
            .iter()
            .all(|s| matches!(s, RetimeSegment::Map(_))));
    }

    #[test]
    fn splitting_an_eased_ramp_preserves_the_curve() {
        // A Slow-ease ramp 100%→300% over [0,4] is one cubic piece (§5.1).
        let r = Retime::single_ramp(rat(4, 1), rat(0, 1), rat(1, 1), rat(3, 1), Ease::Slow);
        assert_split_preserves_curve(&r, rat(3, 2), 4.0);
        // Fast ease too.
        let r = Retime::single_ramp(rat(4, 1), rat(0, 1), rat(1, 1), rat(3, 1), Ease::Fast);
        assert_split_preserves_curve(&r, rat(5, 2), 4.0);
    }

    #[test]
    fn splitting_a_smooth_ramp_works_either_side_of_the_midpoint() {
        // Smooth/Sharp decompose into two pieces at t = D/2 = 2, so cuts at 1
        // and 3 exercise both the first and second piece.
        for ease in [Ease::Smooth, Ease::Sharp] {
            let r = Retime::single_ramp(rat(4, 1), rat(0, 1), rat(1, 2), rat(2, 1), ease);
            assert_split_preserves_curve(&r, rat(1, 1), 4.0);
            assert_split_preserves_curve(&r, rat(3, 1), 4.0);
        }
    }

    #[test]
    fn splitting_a_polynomial_map_segment_preserves_the_curve() {
        // A single polynomial MapSegment (b0 = b1 = 1/3), source 0→3 over [0,2].
        let third = rat(1, 3);
        let rt = store(
            &[(rat(0, 1), rat(0, 1)), (rat(2, 1), rat(3, 1))],
            vec![RetimeSegment::Map(MapSegment::new(
                rat(1, 1),
                rat(2, 1),
                third,
                third,
            ))],
        );
        assert_split_preserves_curve(&rt, rat(1, 1), 2.0);
    }

    #[test]
    fn merging_two_collinear_linear_rates_stays_one_rate() {
        // 100%→200% then 200%→300%: the mid slope matches, so it is one line.
        let mut rt = store(
            &[
                (rat(0, 1), rat(0, 1)),
                (rat(2, 1), rat(0, 1)),
                (rat(4, 1), rat(0, 1)),
            ],
            vec![
                rate(rat(1, 1), rat(2, 1), Ease::Linear),
                rate(rat(2, 1), rat(3, 1), Ease::Linear),
            ],
        );
        rt.recompute_boundaries().unwrap();
        let m = rt.merge_boundary(1).expect("interior boundary merges");
        assert_eq!(m.segments.len(), 1);
        assert!(matches!(m.segments[0], RetimeSegment::Rate(_)));
        // The curve was already a single line, so it is unchanged.
        for k in 0..=20 {
            let t = 4.0 * f64::from(k) / 20.0;
            assert!((m.evaluate(t) - rt.evaluate(t)).abs() < 1e-9, "@ {t}");
        }
    }

    #[test]
    fn merging_a_kink_pins_the_outer_boundaries_and_speeds() {
        // 100%→200% then 200%→100%: a kink, so the merge discards the interior
        // and produces one MapSegment through the pinned outer boundaries.
        let mut rt = store(
            &[
                (rat(0, 1), rat(0, 1)),
                (rat(2, 1), rat(0, 1)),
                (rat(4, 1), rat(0, 1)),
            ],
            vec![
                rate(rat(1, 1), rat(2, 1), Ease::Linear),
                rate(rat(2, 1), rat(1, 1), Ease::Linear),
            ],
        );
        rt.recompute_boundaries().unwrap();
        let m = rt.merge_boundary(1).expect("interior boundary merges");
        assert_eq!(m.segments.len(), 1);
        assert!(matches!(m.segments[0], RetimeSegment::Map(_)));
        // Outer boundaries pinned (source and time both).
        assert_eq!(
            m.boundaries.first().unwrap(),
            rt.boundaries.first().unwrap()
        );
        assert_eq!(m.boundaries.last().unwrap(), rt.boundaries.last().unwrap());
        // Outer one-sided speeds preserved: 100% at each end.
        assert!((m.speed_at(0.0) - 1.0).abs() < 1e-9);
        assert!((m.speed_at(4.0) - 1.0).abs() < 1e-9);
        // Non-interior boundaries cannot be merged.
        assert!(rt.merge_boundary(0).is_none());
        assert!(rt.merge_boundary(2).is_none());
    }

    #[test]
    fn shifting_source_translates_the_curve_without_reshaping_it() {
        let r = Retime::single_ramp(rat(4, 1), rat(0, 1), rat(1, 1), rat(3, 1), Ease::Slow);
        let shifted = r.shift_source(rat(5, 1)).unwrap();
        for k in 0..=10 {
            let t = 4.0 * f64::from(k) / 10.0;
            // Every source position is 5s later.
            assert!(
                (shifted.evaluate(t) - (r.evaluate(t) + 5.0)).abs() < 1e-9,
                "@ {t}"
            );
            // The speed profile (shape) is untouched by a constant shift.
            assert!(
                (shifted.speed_at(t) - r.speed_at(t)).abs() < 1e-9,
                "speed @ {t}"
            );
        }
    }

    #[test]
    fn inserting_a_freeze_holds_the_frame_and_crops_the_tail() {
        // 1x from source 0 over [0,4]. Insert a 1s freeze at local time 1.
        let r = Retime::constant_speed(rat(4, 1), rat(0, 1), rat(1, 1));
        let f = r
            .insert_freeze(rat(1, 1), rat(1, 1))
            .expect("interior freeze");
        f.validate().unwrap();
        // The clip's duration is unchanged — the tail was cropped, not extended.
        assert_eq!(f.boundaries.last().unwrap().t, rat(4, 1));
        // Before the freeze plays as before.
        assert!((f.evaluate(0.5) - 0.5).abs() < 1e-9);
        // Across the freeze [1,2] the source is held at 1.
        assert!((f.evaluate(1.5) - 1.0).abs() < 1e-9);
        assert!((f.evaluate(2.0) - 1.0).abs() < 1e-9);
        // After it, the source resumes at 1 and runs on at 1x.
        assert!((f.evaluate(3.0) - 2.0).abs() < 1e-9);
        // The original tail (source 3→4) was pushed off the end: at D the source
        // is now 3, not 4.
        assert!((f.evaluate(4.0) - 3.0).abs() < 1e-9);
        // Freezes must land strictly inside, with a positive duration.
        assert!(r.insert_freeze(rat(0, 1), rat(1, 1)).is_none());
        assert!(r.insert_freeze(rat(1, 1), rat(0, 1)).is_none());
    }

    #[test]
    fn fitting_a_map_back_to_a_rate_pins_the_endpoints() {
        // Map(1,2,1/3,1/3) over [0,2] with source 0→3 is exactly a Linear rate
        // {1,2} (its speed is v_map(u) = 1 + u), so the §5.2 fit recovers it.
        let third = rat(1, 3);
        let rt = store(
            &[(rat(0, 1), rat(0, 1)), (rat(2, 1), rat(3, 1))],
            vec![RetimeSegment::Map(MapSegment::new(
                rat(1, 1),
                rat(2, 1),
                third,
                third,
            ))],
        );
        let (fitted, drift) = rt
            .with_segment_as_rate(rat(1, 1))
            .expect("map fits to a rate");
        assert!(matches!(fitted.segments[0], RetimeSegment::Rate(_)));
        // C0: the endpoints are pinned exactly.
        assert_eq!(
            fitted.boundaries.first().unwrap().s,
            rt.boundaries.first().unwrap().s
        );
        assert_eq!(
            fitted.boundaries.last().unwrap().s,
            rt.boundaries.last().unwrap().s
        );
        // A map that is exactly a rate's cubic fits with negligible drift, and
        // the curve matches across the whole segment.
        assert!(drift < 1e-6, "drift {drift}");
        for k in 0..=10 {
            let t = 2.0 * f64::from(k) / 10.0;
            assert!((fitted.evaluate(t) - rt.evaluate(t)).abs() < 1e-6, "@ {t}");
        }
        // A Rate segment is not a fit target.
        let ramp = Retime::single_ramp(rat(2, 1), rat(0, 1), rat(1, 1), rat(2, 1), Ease::Linear);
        assert!(ramp.with_segment_as_rate(rat(1, 1)).is_none());
    }

    #[test]
    fn single_ramp_eases_speed_across_the_clip() {
        // 100% → 300% over a 2 s clip, Linear ease: end source advance is
        // d·[v0 + (v1−v0)·E(1)] = 2·[1 + 2·½] = 4, exactly.
        let r = Retime::single_ramp(rat(2, 1), rat(0, 1), rat(1, 1), rat(3, 1), Ease::Linear);
        assert_eq!(r.boundaries[1].s, rat(4, 1));
        // Speed at the ends and middle.
        assert!((r.speed_at(0.0) - 1.0).abs() < 1e-9);
        assert!((r.speed_at(2.0) - 3.0).abs() < 1e-9);
        assert!((r.speed_at(1.0) - 2.0).abs() < 1e-9); // Linear: 2× at midpoint
                                                       // Monotone increasing source position.
        assert!(r.evaluate(0.5) < r.evaluate(1.5));
        r.validate().unwrap();
    }

    #[test]
    fn freeze_holds_one_source_position() {
        let mut r = store(
            &[(rat(0, 1), rat(7, 2)), (rat(3, 1), rat(99, 1))],
            vec![rate(rat(0, 1), rat(0, 1), Ease::Linear)],
        );
        r.recompute_boundaries().unwrap();
        assert_eq!(r.boundaries[1].s, rat(7, 2)); // frozen: no advance at all
        for t in [0.0, 0.6, 1.5, 2.9, 3.0] {
            assert!((r.evaluate(t) - 3.5).abs() < 1e-12, "t = {t}");
        }
        assert!(r.speed_at(1.0).abs() < 1e-12);
    }

    #[test]
    fn linear_ramp_matches_the_hand_integral() {
        // v: 1 → 3 over d = 1 with Linear ease. E(1) = 1/2, so
        // s_end = s0 + d·(v0 + (v1 − v0)/2) = 0 + 1·(1 + 1) = 2, exactly.
        let mut r = store(
            &[(rat(0, 1), rat(0, 1)), (rat(1, 1), rat(0, 1))],
            vec![rate(rat(1, 1), rat(3, 1), Ease::Linear)],
        );
        r.recompute_boundaries().unwrap();
        assert_eq!(r.boundaries[1].s, rat(2, 1));
        // Midpoint by hand: f(½) = v0·u + Δv·u²/2 = ½ + 2·⅛ = ¾.
        assert!((r.evaluate(0.5) - 0.75).abs() < 1e-12);
        assert!((r.evaluate(1.0) - 2.0).abs() < 1e-12);
        // And the speed profile itself: v(½) = 1 + 2·½ = 2.
        assert!((r.speed_at(0.5) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn boundary_consistency_matches_f64_evaluation_for_every_ease() {
        for ease in [
            Ease::Linear,
            Ease::Slow,
            Ease::Fast,
            Ease::Smooth,
            Ease::Sharp,
        ] {
            let mut r = store(
                &[(rat(0, 1), rat(1, 4)), (rat(2, 1), rat(0, 1))],
                vec![rate(rat(1, 2), rat(5, 2), ease)],
            );
            r.recompute_boundaries().unwrap();
            let exact_end = r.boundaries[1].s.to_f64();
            let evaluated_end = r.evaluate(2.0);
            assert!(
                (exact_end - evaluated_end).abs() < 1e-9,
                "{ease:?}: rational end {exact_end} vs evaluated {evaluated_end}"
            );
        }
    }

    #[test]
    fn ease_integrals_agree_with_their_exact_endpoints() {
        for ease in [
            Ease::Linear,
            Ease::Slow,
            Ease::Fast,
            Ease::Smooth,
            Ease::Sharp,
        ] {
            assert_eq!(ease.big_e(0.0), 0.0, "{ease:?} at 0");
            let exact = ease.e_at_1().to_f64();
            assert!((ease.big_e(1.0) - exact).abs() < 1e-12, "{ease:?} at 1");
            // The piecewise Smooth/Sharp forms join without a step at u = ½.
            let below = ease.big_e(0.5 - 1e-9);
            let above = ease.big_e(0.5 + 1e-9);
            assert!(
                (below - above).abs() < 1e-8,
                "{ease:?} join: {below} vs {above}"
            );
            // e(u) ≥ 0 on [0, 1] for every ease, so E must never decrease.
            let mut prev = 0.0;
            for i in 0..=100 {
                let e = ease.big_e(f64::from(i) / 100.0);
                assert!(e >= prev - 1e-12, "{ease:?} decreasing at {i}");
                prev = e;
            }
        }
    }

    #[test]
    fn reverse_speeds_clamp_when_reverse_is_off() {
        // Authored with negative speed but the gate off: behaves as a freeze
        // (speeds clamp to zero — §6.2 monotone clamp) in both the exact
        // boundary maths and evaluation.
        let mut r = store(
            &[(rat(0, 1), rat(5, 1)), (rat(2, 1), rat(0, 1))],
            vec![rate(rat(-1, 1), rat(-1, 1), Ease::Linear)],
        );
        r.recompute_boundaries().unwrap();
        assert_eq!(r.boundaries[1].s, rat(5, 1));
        assert!((r.evaluate(1.0) - 5.0).abs() < 1e-12);
        assert!(r.speed_at(1.0).abs() < 1e-12);

        // Same store with the gate on: genuine reverse.
        r.allow_reverse = true;
        r.recompute_boundaries().unwrap();
        assert_eq!(r.boundaries[1].s, rat(3, 1)); // 5 + 2·(−1)
        assert!((r.evaluate(1.0) - 4.0).abs() < 1e-12);
        assert!((r.speed_at(1.0) + 1.0).abs() < 1e-12);
    }

    #[test]
    fn worked_example_from_the_spec_is_bit_exact() {
        // docs/04-RETIMING.md §12.4: 100% / 850% / 20% / 100% over
        // t = 0, 1.2, 1.45, 2.8, 4 — the boundary rationals are given in the
        // spec and must reproduce bit-for-bit.
        let mut r = store(
            &[
                (rat(0, 1), rat(0, 1)),
                (rat(6, 5), rat(0, 1)),
                (rat(29, 20), rat(0, 1)),
                (rat(14, 5), rat(0, 1)),
                (rat(4, 1), rat(0, 1)),
            ],
            vec![
                rate(rat(1, 1), rat(1, 1), Ease::Linear),
                rate(rat(17, 2), rat(17, 2), Ease::Linear),
                rate(rat(1, 5), rat(1, 5), Ease::Linear),
                rate(rat(1, 1), rat(1, 1), Ease::Linear),
            ],
        );
        r.recompute_boundaries().unwrap();
        let expected = [
            rat(0, 1),
            rat(6, 5),
            rat(133, 40),
            rat(719, 200),
            rat(959, 200),
        ];
        for (i, want) in expected.iter().enumerate() {
            assert_eq!(r.boundaries[i].s, *want, "boundary {i}");
        }
        // Binary search lands mid-ramp correctly: halfway through the 850%
        // segment, f = 6/5 + 0.125·8.5.
        assert!((r.evaluate(1.325) - 2.2625).abs() < 1e-9);
    }

    #[test]
    fn general_influence_map_hits_its_endpoints() {
        // m0 = m1 = 0 with b0 = b1 = 1/4: an S-shaped ease-in/out in the
        // general-influence class (solver path, not the 1/3 fast path).
        let seg = MapSegment::new(rat(0, 1), rat(0, 1), rat(1, 4), rat(1, 4));
        let r = store(
            &[(rat(0, 1), rat(0, 1)), (rat(2, 1), rat(1, 1))],
            vec![RetimeSegment::Map(seg)],
        );
        assert!(r.evaluate(0.0).abs() < 1e-9);
        assert!((r.evaluate(2.0) - 1.0).abs() < 1e-9);
        // Symmetric handles: the midpoint maps to the middle of the source span.
        assert!((r.evaluate(1.0) - 0.5).abs() < 1e-9);
        // x-monotone curve: output is non-decreasing and inside the span.
        let mut prev = -1e-9;
        for i in 0..=200 {
            let v = r.evaluate(f64::from(i) / 100.0);
            assert!(v >= prev - 1e-9, "not monotone at sample {i}");
            assert!(
                (-1e-9..=1.0 + 1e-9).contains(&v),
                "out of range at {i}: {v}"
            );
            prev = v;
        }
    }

    #[test]
    fn full_influence_spike_stays_bracketed() {
        // b0 = b1 = 1: x′(u) = 0 at u = ½ — the case that diverges under
        // plain Newton. The bracketed solver must stay finite and in-range.
        let seg = MapSegment::new(rat(0, 1), rat(0, 1), rat(1, 1), rat(1, 1));
        let r = store(
            &[(rat(0, 1), rat(0, 1)), (rat(1, 1), rat(1, 1))],
            vec![RetimeSegment::Map(seg)],
        );
        for i in 0..=100 {
            let t = f64::from(i) / 100.0;
            let v = r.evaluate(t);
            assert!(v.is_finite());
            assert!((-1e-9..=1.0 + 1e-9).contains(&v), "t = {t}, v = {v}");
        }
        assert!((r.evaluate(0.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn linear_rate_and_its_polynomial_map_agree() {
        // §5.1 exactness, checked from both sides without a conversion API:
        // a Linear rate 1 → 3 equals the hand-built polynomial-subclass map
        // (m0 = v0, m1 = v1, b0 = b1 = 1/3) over the same boundaries.
        let bounds = [(rat(0, 1), rat(0, 1)), (rat(1, 1), rat(2, 1))];
        let as_rate = store(&bounds, vec![rate(rat(1, 1), rat(3, 1), Ease::Linear)]);
        let as_map = store(
            &bounds,
            vec![RetimeSegment::Map(MapSegment::new(
                rat(1, 1),
                rat(3, 1),
                rat(1, 3),
                rat(1, 3),
            ))],
        );
        for i in 0..=100 {
            let t = f64::from(i) / 100.0;
            let (a, b) = (as_rate.evaluate(t), as_map.evaluate(t));
            assert!((a - b).abs() < 1e-12, "t = {t}: rate {a} vs map {b}");
        }
    }

    #[test]
    fn validate_rejects_malformed_stores() {
        let good = Retime::identity(rat(1, 1), rat(0, 1));

        let mut no_segments = good.clone();
        no_segments.segments.clear();
        assert!(no_segments.validate().is_err());

        let mut miscounted = good.clone();
        miscounted
            .boundaries
            .push(Boundary::new(rat(2, 1), rat(2, 1)));
        assert!(miscounted.validate().is_err());

        let mut late_start = good.clone();
        late_start.boundaries[0].t = rat(1, 2);
        assert!(late_start.validate().is_err());

        let mut not_increasing = good.clone();
        not_increasing.boundaries[1].t = rat(0, 1);
        assert!(not_increasing.validate().is_err());

        assert!(good.validate().is_ok());
    }

    #[test]
    fn overflow_falls_back_to_the_flick_grid() {
        // Coprime denominators near 2^32 (nothing cancels): the exact sum's
        // denominator would pass 2^63, so the §4.1 fallback rounds onto the
        // flick grid instead of failing.
        let a = rat(1_000_003, 4_294_967_297); // prime num, den = 2^32 + 1
        let b = rat(1_000_003, 8_589_934_591); // prime num, den = 2^33 − 1
        assert!(a.checked_add(b).is_err(), "expected raw overflow");
        let sum = add_with_flick_fallback(a, b).unwrap();
        let exact = 1_000_003.0 / 4_294_967_297.0 + 1_000_003.0 / 8_589_934_591.0;
        assert!(
            (sum.to_f64() - exact).abs() < 2.0 / Rational::FLICK_DEN as f64,
            "grid-rounded {sum:?} too far from {exact}"
        );
    }

    #[test]
    fn retime_round_trips_through_serde() {
        let mut r = Retime::identity(rat(4, 1), rat(0, 1));
        r.segments.push(RetimeSegment::Map(MapSegment::new(
            rat(1, 1),
            rat(0, 1),
            rat(1, 4),
            rat(1, 2),
        )));
        r.boundaries.push(Boundary::new(rat(6, 1), rat(9, 2)));
        r.interpolation = Interpolation::Flow(FlowParams::default());
        r.allow_reverse = true;
        let json = serde_json::to_value(&r).unwrap();
        let back: Retime = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn changing_segment_ease_pins_start_and_recomputes_downstream() {
        // Ramp 100% → 0% over 2 s. Linear end source = 2·[1 + (0−1)·½] = 1.0.
        let base = Retime::single_ramp(rat(2, 1), rat(0, 1), rat(1, 1), rat(0, 1), Ease::Linear);
        assert!((base.evaluate(2.0 - 1e-9) - 1.0).abs() < 1e-6);
        // Slow ease: E(1) = 1/3, so end source = 2·[1 + (−1)·⅓] = 4/3.
        let slow = base.with_segment_ease(rat(1, 1), Ease::Slow).unwrap();
        assert!(slow.evaluate(0.0).abs() < 1e-9, "start is pinned");
        assert!(
            (slow.evaluate(2.0 - 1e-9) - 4.0 / 3.0).abs() < 1e-3,
            "downstream source recomputed for the new ease"
        );
        slow.validate().unwrap();
    }

    #[test]
    fn setting_segment_speeds_recomputes_and_pins_start() {
        // Constant 100% over 2 s → end source 2.0.
        let base = Retime::constant_speed(rat(2, 1), rat(0, 1), rat(1, 1));
        assert!((base.evaluate(2.0 - 1e-9) - 2.0).abs() < 1e-6);
        // Ramp it to 100% → 50%: Δs = 2·[1 + (½−1)·½] = 1.5, start pinned at 0.
        let ramp = base
            .with_segment_speeds(rat(1, 1), rat(1, 1), rat(1, 2))
            .unwrap();
        assert!(ramp.evaluate(0.0).abs() < 1e-9, "start pinned");
        assert!(
            (ramp.evaluate(2.0 - 1e-9) - 1.5).abs() < 1e-3,
            "downstream source recomputed"
        );
        // Linear ramp reads 75% at the midpoint.
        assert!((ramp.speed_at(1.0) - 0.75).abs() < 1e-3);
        ramp.validate().unwrap();
    }

    #[test]
    fn segment_index_at_locates_and_bounds() {
        // Two segments over [0,2] and [2,4].
        let keys = [
            (rat(0, 1), rat(1, 1)),
            (rat(2, 1), rat(1, 1)),
            (rat(4, 1), rat(1, 1)),
        ];
        let r = Retime::from_speed_keyframes(rat(0, 1), &keys).unwrap();
        assert_eq!(r.segment_index_at(rat(1, 1)), Some(0));
        assert_eq!(r.segment_index_at(rat(3, 1)), Some(1));
        assert_eq!(r.segment_index_at(rat(4, 1)), Some(1)); // t == D → last segment
        assert_eq!(r.segment_index_at(rat(-1, 1)), None);
        assert_eq!(r.segment_index_at(rat(5, 1)), None);
    }

    proptest! {
        // The frame-pinning covenant (K-070): editing a segment's speeds only
        // moves source positions *after* its start — the start itself is pinned,
        // and the store stays valid, for any speeds.
        #[test]
        fn setting_speeds_pins_the_edited_segment_start(
            v0 in 1i64..30,
            v1 in 1i64..30,
            nv0 in 1i64..30,
            nv1 in 1i64..30,
        ) {
            let keys = [
                (rat(0, 1), rat(v0, 10)),
                (rat(1, 1), rat(v1, 10)),
                (rat(2, 1), rat(v0, 10)),
                (rat(3, 1), rat(v1, 10)),
            ];
            let base = Retime::from_speed_keyframes(rat(0, 1), &keys).unwrap();
            let i = base.segment_index_at(rat(3, 2)).unwrap();
            let start_before = base.boundaries[i].s;
            let edited = base
                .with_segment_speeds(rat(3, 2), rat(nv0, 10), rat(nv1, 10))
                .unwrap();
            prop_assert_eq!(edited.boundaries[i].s, start_before);
            edited.validate().unwrap();
        }
    }
}
