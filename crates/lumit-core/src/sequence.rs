//! Sequence layers: clips cut back-to-back on one row (docs/03-DATA-MODEL.md
//! §5.3, docs/04-RETIMING.md §1.3). This is Lumit's Vegas-style editing
//! surface.
//!
//! In plain terms: a Sequence layer is one timeline row holding a run of
//! **clips** laid end to end. Each clip points at a source (a footage item or
//! a comp), carries its own trim and its own [`Retime`] ramp, and sits at an
//! exact place on the row. Two clips may overlap, and the overlap is a
//! dissolve on a layer that draws a picture and a crossfade on an audio-only
//! one (docs/03-DATA-MODEL.md §5.3). A gap
//! between them shows through as transparent. To draw the layer at a given
//! moment you ask "which
//! clip is under the playhead, and which moment of its source does that map
//! to?" — that resolution is all this module does. Turning that source moment
//! into pixels, and the layer's own masks/effects/transform, happen above.
//!
//! Scope note: this is the resolution model and its invariants only. Wiring
//! it into `LayerKind` and the render paths is the next step and lives
//! elsewhere; cutting (§8) and the graph lenses (§9) build on top.

use crate::anim::{Animation, CubicSpan, Keyframe, Property, SideInterp};
use crate::model::{default_true, is_true, is_zero, Composition, EffectInstance};
use crate::retime::Interpolation;
use crate::time::Rational;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a clip plays: one footage item or one nested composition
/// (docs/03-DATA-MODEL.md §5.3 ClipSource).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipSource {
    Footage(Uuid),
    Comp(Uuid),
}

/// How far a Custom handle may reach past the box in y, in box heights, and
/// the narrowest reach it may have in x. The Easing panel's own two bounds:
/// y is a value and overshoot is the point of it, x is time and holding both
/// handles inside the span is what keeps the curve x-monotone.
const HANDLE_REACH: f64 = 0.5;
const MIN_REACH: f64 = 1e-3;

/// The curve a fade follows, written as the gain of a fade **in**: `u` runs
/// from 0 at silence to 1 at full level, and a fade out reads the same curve
/// backwards (docs/impl/audio-timeline.md §3). So a shape is one curve, named
/// once, and the end of the clip it sits on decides which way it is read.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FadeShape {
    Linear,
    /// A quarter sine. Fast in against Fast out over one overlap keeps the two
    /// gains' squares summing to one, which is the crossfade that holds its
    /// level, so it is what a fresh overlap gets and the default here.
    #[default]
    Fast,
    Slow,
    Smooth,
    /// Smooth turned inside out: steep at both ends, flat through the middle.
    Sharp,
    /// A cubic bezier from (0, 0) to (1, 1) with two handles, read at x = u -
    /// the Easing panel's own four numbers, held to the same bounds.
    Custom {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
}

impl FadeShape {
    /// A Custom shape with its handles already inside the legal box.
    #[must_use]
    pub fn custom(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        let (x1, y1, x2, y2) = clamp_handles(x1, y1, x2, y2);
        FadeShape::Custom { x1, y1, x2, y2 }
    }

    /// The gain of a fade **in** at `u`. A fade out is this read backwards,
    /// `gain(1 - u)`.
    #[must_use]
    pub fn gain(self, u: f64) -> f64 {
        // Every bound in here is a constant, so `clamp` has no reversed pair
        // to panic on (docs/14 §4).
        let u = u.clamp(0.0, 1.0);
        match self {
            FadeShape::Linear => u,
            FadeShape::Fast => (u * std::f64::consts::FRAC_PI_2).sin(),
            FadeShape::Slow => 1.0 - (u * std::f64::consts::FRAC_PI_2).cos(),
            FadeShape::Smooth => u * u * (3.0 - 2.0 * u),
            // The exact inverse of Smooth rather than a steeper curve chosen
            // by eye: Smooth is u²(3 − 2u), and this is the u it came from.
            FadeShape::Sharp => 0.5 - ((1.0 - 2.0 * u).clamp(-1.0, 1.0).asin() / 3.0).sin(),
            // Solved for x, not walked in the bezier's own parameter: the
            // curve is read at a moment, and `CubicSpan` is where that solve
            // already lives (docs/impl/keyframe-eval.md §2).
            FadeShape::Custom { x1, y1, x2, y2 } => {
                let (x1, y1, x2, y2) = clamp_handles(x1, y1, x2, y2);
                CubicSpan::from_points([0.0, x1, x2, 1.0], [0.0, y1, y2, 1.0]).value_at(u)
            }
        }
    }
}

/// Both handles of a Custom shape inside a legal, reachable box.
fn clamp_handles(x1: f64, y1: f64, x2: f64, y2: f64) -> (f64, f64, f64, f64) {
    let y = |v: f64| v.clamp(-HANDLE_REACH, 1.0 + HANDLE_REACH);
    (
        x1.clamp(MIN_REACH, 1.0),
        y(y1),
        x2.clamp(0.0, 1.0 - MIN_REACH),
        y(y2),
    )
}

/// One end of a clip's fade: how long it takes, and the curve it takes
/// (docs/impl/audio-timeline.md §3). Zero seconds is no fade.
///
/// Inside an **overlap** the seconds are not read - the overlap is the length
/// of both fades across it - but the shape still is, so a crossfade takes its
/// two curves from the two clips that make it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fade {
    pub seconds: Rational,
    pub shape: FadeShape,
}

impl Default for Fade {
    fn default() -> Self {
        Self {
            seconds: Rational::ZERO,
            shape: FadeShape::default(),
        }
    }
}

impl Fade {
    /// Nothing stored: no length and the shape a fresh overlap takes. What a
    /// clip written before fades existed reads as, and what is left out of the
    /// file when it is written again.
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// One clip on a Sequence layer (docs/03-DATA-MODEL.md §5.3). Times are exact
/// rationals in seconds; `place_*` are on the layer's timeline, `source_*`
/// index into the clip's source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Uuid,
    pub source: ClipSource,
    /// Trim into the source (seconds).
    pub source_in: Rational,
    /// Exclusive trim end (seconds).
    pub source_out: Rational,
    /// Where the clip starts on the layer's timeline (seconds).
    pub place_start: Rational,
    /// How long the clip occupies on the layer's timeline (seconds).
    pub place_duration: Rational,
    /// The clip's retime map: clip-local time → source time, in seconds, as
    /// an ordinary keyframable [`Property`] — the same shape a layer's Retime
    /// has.
    ///
    /// `None` is "not retimed": the clip plays from [`Self::source_in`] at
    /// source rate. That is a different state from a map that happens to be
    /// 1×, exactly as it is on a layer, and only the first skips the map.
    ///
    /// It was a segment store until the move to this property. Two
    /// representations for one job is what that move existed to end, and clips
    /// were the second half of it. A document written before then converts on
    /// open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retime: Option<Property>,
    /// How fractional source moments become pixels (render policy).
    #[serde(default)]
    pub interpolation: Interpolation,
    /// The fade at each end (docs/impl/audio-timeline.md §3). Both are left
    /// out of the file while nothing is set on them, so a project written
    /// before fades existed writes again byte for byte as it was.
    #[serde(default, skip_serializing_if = "Fade::is_default")]
    pub fade_in: Fade,
    #[serde(default, skip_serializing_if = "Fade::is_default")]
    pub fade_out: Fade,
    /// The clip's own effect stack, the same shape a layer's and a group
    /// header's has: on an audio row it is the rack on the clip, running
    /// ahead of the row's own (docs/impl/audio-timeline.md §4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectInstance>,
    /// The whole stack's bypass, the clip's twin of the layer's fx switch.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub fx: bool,
    /// The clip's own gain in dB, the line drawn across its box
    /// (docs/impl/audio-timeline.md §2). A number and not a [`Property`]: the
    /// line is set, not automated, and automation is the row's own Volume.
    /// It is applied where the fades are, as one more multiplier on the
    /// clip's placed gain, so it rides ahead of nothing and after nothing.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gain_db: f64,
    /// Clips sharing a link belong together: a picture clip and the clip that
    /// carries its sound move, trim and cut as one while linking is on. An id
    /// is shared only by clips that should move together, so a cut through a
    /// linked pair gives the later halves an id of their own. Left out of the
    /// file while unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Uuid>,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Clip {
    /// A plain (un-retimed) clip of `source` placed at `place_start` for
    /// `place_duration`, playing its source from `source_in` at natural rate.
    pub fn new(
        source: ClipSource,
        source_in: Rational,
        source_out: Rational,
        place_start: Rational,
        place_duration: Rational,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            source,
            source_in,
            source_out,
            place_start,
            place_duration,
            retime: None,
            interpolation: Interpolation::default(),
            fade_in: Fade::default(),
            fade_out: Fade::default(),
            effects: Vec::new(),
            fx: true,
            gain_db: 0.0,
            link: None,
            extra: serde_json::Map::new(),
        }
    }

    /// Where the clip ends on the layer timeline (exclusive).
    pub fn place_end(&self) -> Rational {
        self.place_start
            .checked_add(self.place_duration)
            .unwrap_or(self.place_start)
    }

    /// A two-key retime running from `source_in` at `v0` to `v1` across the
    /// clip — the straight-line speed the Vegas envelope authors.
    ///
    /// The keys carry their endpoint speeds as tangents, and the source
    /// position they reach is the area under that straight line: the average
    /// of the two speeds times the duration. That makes the cubic between them
    /// have an exactly linear derivative, so the ramp the envelope draws and
    /// the curve stored here are the same curve rather than two descriptions
    /// of one.
    fn ramp_property(&self, v0: Rational, v1: Rational) -> Option<(Property, Rational)> {
        let d = self.place_duration;
        let mean = v0
            .checked_add(v1)
            .ok()?
            .checked_div(Rational::new(2, 1).ok()?)
            .ok()?;
        let source_out = self.source_in.checked_add(mean.checked_mul(d).ok()?).ok()?;
        let chord = if d > Rational::ZERO {
            source_out
                .checked_sub(self.source_in)
                .ok()?
                .checked_div(d)
                .ok()?
                .to_f64()
        } else {
            0.0
        };
        // A side whose speed is already the chord stays Linear: the two are
        // the same curve, and the bezier form would only change how the key
        // draws (the same rule the envelope editor follows).
        let side = |v: Rational| {
            if (v.to_f64() - chord).abs() < 1e-12 {
                SideInterp::Linear
            } else {
                SideInterp::Bezier {
                    speed: v.to_f64(),
                    influence: 1.0 / 3.0,
                }
            }
        };
        Some((
            Property {
                animation: Animation::Keyframed(vec![
                    Keyframe {
                        time: Rational::ZERO,
                        value: self.source_in.to_f64(),
                        interp_in: SideInterp::Linear,
                        interp_out: side(v0),
                    },
                    Keyframe {
                        time: d,
                        value: source_out.to_f64(),
                        interp_in: side(v1),
                        interp_out: SideInterp::Linear,
                    },
                ]),
                extra: serde_json::Map::new(),
            },
            source_out,
        ))
    }

    /// This clip with a single speed *ramp* — speed running straight from `v0`
    /// to `v1` across the clip — its place on the layer unchanged (beat-sync).
    /// The montage speed gesture; `source_out` follows from the integral.
    ///
    /// The eased shapes the segment store offered (Slow/Fast/Smooth/Sharp) are
    /// not here: they belong to the preset shelf, which is being reworked
    /// (docs/TODO.md) and will be rebuilt on the property like everything else
    /// that moved off the segment store.
    pub fn with_ramp(&self, v0: Rational, v1: Rational) -> Clip {
        match self.ramp_property(v0, v1) {
            Some((retime, source_out)) => Clip {
                retime: Some(retime),
                source_out,
                ..self.clone()
            },
            // Arithmetic that will not fit leaves the clip exactly as it was,
            // which is always a legal clip.
            None => self.clone(),
        }
    }

    /// The map this clip actually plays by: its own, or the identity it is
    /// playing without one.
    ///
    /// The identity runs from [`Self::source_in`] — **not from zero**. That
    /// distinction is the whole reason this exists: every clip after a cut
    /// starts part way into its source, and anything that assumed a clip's
    /// map began at source zero sent it back to the top of the media the
    /// moment it was retimed. Read the effective map and there is nothing to
    /// assume.
    pub fn effective_retime(&self) -> Property {
        if let Some(map) = &self.retime {
            return map.clone();
        }
        let end = self
            .source_in
            .checked_add(self.place_duration)
            .unwrap_or(self.source_in);
        Property {
            animation: Animation::Keyframed(vec![
                Keyframe {
                    time: Rational::ZERO,
                    value: self.source_in.to_f64(),
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                },
                Keyframe {
                    time: self.place_duration,
                    value: end.to_f64(),
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Linear,
                },
            ]),
            extra: serde_json::Map::new(),
        }
    }

    /// The clip's single constant speed (1.0 = source rate), or None when its
    /// map says something one number cannot.
    ///
    /// Read across **every** span, not just a two-key map: extending a clip
    /// adds a key at the same speed, and three keys in a straight line are
    /// still one constant speed however many of them there are.
    pub fn constant_speed(&self) -> Option<f64> {
        // Not retimed is the plainest constant speed there is: source rate.
        let Some(speeds) = self.span_speeds() else {
            return self.retime.is_none().then_some(1.0);
        };
        let first = *speeds.first()?;
        speeds
            .iter()
            .all(|v| (v - first).abs() < 1e-9)
            .then_some(first)
    }

    /// Every speed the map actually reaches: both ends of every span, in
    /// order. None when the clip is not retimed, or its map is not keyframed.
    ///
    /// The *tangents*, not the chords. A chord is a span's average speed, so
    /// reading chords cannot tell a ramp from a constant — 100% into 300% and
    /// a flat 200% have the same chord, and the first is emphatically not one
    /// speed. These are the same numbers the envelope draws its points at.
    fn span_speeds(&self) -> Option<Vec<f64>> {
        let Animation::Keyframed(keys) = &self.retime.as_ref()?.animation else {
            return None;
        };
        if keys.len() < 2 {
            return None;
        }
        // Through `resolved_side`, so an automatic tangent reads as the speed
        // its neighbours give it rather than as the ease it is remembering.
        let side = |s: SideInterp, chord: f64| match s {
            SideInterp::Bezier { speed, .. } => speed,
            SideInterp::Hold => 0.0,
            SideInterp::Linear | SideInterp::Auto { .. } => chord,
        };
        let mut out = Vec::with_capacity((keys.len() - 1) * 2);
        for i in 0..keys.len() - 1 {
            let dt = keys[i + 1].time.checked_sub(keys[i].time).ok()?.to_f64();
            if dt <= 0.0 {
                return None;
            }
            let chord = (keys[i + 1].value - keys[i].value) / dt;
            out.push(side(crate::anim::resolved_side(keys, i, true), chord));
            out.push(side(crate::anim::resolved_side(keys, i + 1, false), chord));
        }
        Some(out)
    }

    /// The speed the clip leaves at, and the one it arrives at — the ends of
    /// [`Self::span_speeds`]. Used when a clip is extended, so the map is
    /// carried on at the speed it was already going rather than at 1×.
    fn end_speeds(&self) -> Option<(f64, f64)> {
        let speeds = self.span_speeds()?;
        Some((*speeds.first()?, *speeds.last()?))
    }

    /// The clip's ramp as `(start speed, end speed)` when its map is two keys
    /// — the shape the envelope authors. None for anything richer, which the
    /// timeline cannot show as a pair of numbers.
    pub fn ramp_view(&self) -> Option<(f64, f64)> {
        let Animation::Keyframed(keys) = &self.retime.as_ref()?.animation else {
            return None;
        };
        if keys.len() != 2 {
            return None;
        }
        // A two-key map has one span, so its end speeds are the ramp.
        self.end_speeds()
    }

    /// How far this clip's source runs either side of its trim, on the
    /// **layer's** own clock: `(first source moment, last source moment)` as
    /// layer-local times, or `None` when the reach is not knowable.
    ///
    /// In plain terms: the clip shows a window onto a longer piece of media,
    /// and this says where that whole piece would sit on the row if none of it
    /// had been trimmed away — which is the faint outline the Timeline draws
    /// around a trimmed clip (docs/15-DESIGN.md §12A.1).
    ///
    /// The clip-level twin of the layer bar's bounds, and it follows the same
    /// three rules:
    ///
    /// * an un-retimed clip plays its source alongside its own clock from
    ///   [`Self::source_in`], so source moment zero sits `source_in` before the
    ///   clip's start and the source's last moment `source_duration` after
    ///   that;
    /// * a **retimed** clip has no reach — its map decides for itself which
    ///   source moment each of its own frames shows, so its length stops being
    ///   the source's business (docs/04-RETIMING.md);
    /// * a source whose length could not be read has no reach either, rather
    ///   than one pinned to a guess.
    ///
    /// `source_duration` is passed in because the document does not hold it: a
    /// nested comp's is on the comp, and a footage item's comes from the media
    /// probe, so only the caller can know it. Nothing is clamped — a clip
    /// dragged so its source would begin before the row's origin reports a
    /// negative first moment, exactly as a layer's bounds do.
    pub fn source_reach(&self, source_duration: Option<Rational>) -> Option<(Rational, Rational)> {
        if self.retime.is_some() {
            return None;
        }
        let start = self.place_start.checked_sub(self.source_in).ok()?;
        let end = start.checked_add(source_duration?).ok()?;
        Some((start, end))
    }

    /// True when layer-local time `lt` (seconds) falls within this clip.
    pub fn contains(&self, lt: f64) -> bool {
        lt >= self.place_start.to_f64() && lt < self.place_end().to_f64()
    }

    /// The source time (seconds) shown at layer-local time `lt`, via the
    /// clip's retime (which maps clip-local time → source time). Only
    /// meaningful when [`Self::contains`] is true.
    pub fn source_time(&self, lt: f64) -> f64 {
        let clip_time = lt - self.place_start.to_f64();
        match &self.retime {
            Some(map) => map.value_at(clip_time),
            // Not retimed: the source runs alongside the clip's own clock,
            // from wherever it was trimmed in.
            None => self.source_in.to_f64() + clip_time,
        }
    }

    /// [`Self::source_time`] held inside the clip's trim, which is the moment
    /// the render draws and the cache key names: on overrun the mapped source
    /// position holds at the clip's [source_in, source_out] boundary rather
    /// than running on into media past the trim (docs/04 §7.2). The raw map
    /// stays available through [`Self::source_time`] for overrun detection.
    /// `.max().min()` avoids `f64::clamp`'s panics on a degenerate window or
    /// a NaN map (engine crates never panic).
    pub fn held_source_time(&self, lt: f64) -> f64 {
        self.source_time(lt)
            .max(self.source_in.to_f64())
            .min(self.source_out.to_f64())
    }

    /// Which moment of `nested` a clip that plays it shows at layer time
    /// `lt`. Composition time is source time, so this is
    /// [`Self::held_source_time`], and a Retime that runs past the comp holds
    /// its last frame, as [`crate::model::nested_source_time`] holds a
    /// Precomp layer's. The one place a clip's composition is given a time,
    /// so the frame key, the decode plan and the draw agree on it.
    pub fn nested_time(&self, nested: &Composition, lt: f64) -> f64 {
        let st = self.held_source_time(lt);
        if self.retime.is_none() {
            return st;
        }
        let last = nested.duration.0.to_f64() - 1.0 / nested.frame_rate.fps().max(1.0);
        st.min(last).max(0.0)
    }

    /// The exact source time shown at layer time `at`, through the clip's
    /// Retime and held at its trim as [`resolve`] holds it. Where
    /// [`Self::source_time`] is the float the renderer samples with, this is
    /// an answer to keep, on the flick grid where a map had to be read. None
    /// on overflow.
    pub fn source_at(&self, at: Rational) -> Option<Rational> {
        let tau = at.checked_sub(self.place_start).ok()?;
        let shown = match &self.retime {
            None => self.source_in.checked_add(tau).ok()?,
            Some(map) => {
                Rational::from_f64_on_grid(map.value_at(tau.to_f64()), Rational::FLICK_DEN).ok()?
            }
        };
        Some(shown.max(self.source_in).min(self.source_out))
    }

    /// How far the clip's end, or its start with `at_start`, can be carried
    /// outward before it asks for source its media does not have, as a length
    /// on the layer. `source_duration` is the media's own length, which only
    /// the caller can know ([`Self::source_reach`]).
    ///
    /// None when nothing holds the edge: a frozen edge uses no source, and a
    /// source whose length could not be read has no end to run past. The top
    /// of the source is always known, so an edge running towards it is held
    /// either way.
    pub fn spare(&self, at_start: bool, source_duration: Option<Rational>) -> Option<Rational> {
        let speed = self
            .end_speeds()
            .map_or(1.0, |(v0, v1)| if at_start { v0 } else { v1 });
        if speed == 0.0 {
            return None;
        }
        // Carried outward, a head playing forwards and a tail playing
        // backwards run towards the top of the source, and the other two
        // towards its end.
        let edge = if at_start {
            self.source_in
        } else {
            self.source_out
        };
        let left = if at_start == (speed > 0.0) {
            edge
        } else {
            source_duration?.checked_sub(edge).ok()?
        }
        .max(Rational::ZERO);
        if speed.abs() == 1.0 {
            return Some(left);
        }
        Rational::from_f64_on_grid(left.to_f64() / speed.abs(), Rational::FLICK_DEN).ok()
    }

    /// A copy of this clip that is a clip of its own: a fresh id, and fresh
    /// ids down its effect stack, since an effect is found by its id alone.
    /// The link is kept, and is the caller's to pair again.
    pub fn duplicate(&self) -> Clip {
        Clip {
            id: Uuid::now_v7(),
            effects: respawn(&self.effects),
            ..self.clone()
        }
    }

    /// The clip's map cut at clip-local time `tau`: the part before, the part
    /// after re-based to start at zero, and the exact source position the two
    /// meet at.
    ///
    /// An un-retimed clip splits trivially — both halves stay un-retimed and
    /// the meeting point is its natural source position — which is the common
    /// case the razor hits on a freshly imported clip.
    fn map_split(&self, tau: Rational) -> Option<(Option<Property>, Option<Property>, Rational)> {
        let Some(map) = self.retime.as_ref() else {
            return Some((None, None, self.source_in.checked_add(tau).ok()?));
        };
        // **An expression cannot be cut in two.** The source positions it
        // produces are computed rather than stored, so splitting one means
        // rewriting what the user typed — `(expr)` on the left and `(expr)`
        // shifted on the right — which is not a cut, it is an edit of their
        // work. Refused, exactly as [`Self::slip`] refuses one. Unreachable
        // today: only transform and effect properties can carry an expression,
        // and this wants deciding properly if Retime ever offers one.
        if matches!(map.animation, Animation::Expression(_)) {
            return None;
        }
        let on_grid = |v: f64| Rational::from_f64_on_grid(v, Rational::FLICK_DEN).ok();

        let mut whole = map.clone();
        freeze_auto_around(&mut whole, tau);
        whole.insert_key_preserving_shape(tau);
        let Animation::Keyframed(keys) = &whole.animation else {
            // A static map holds one source moment throughout, so both halves
            // are the same map and the cut lands on that moment.
            let s = on_grid(map.value_at(tau.to_f64()))?;
            return Some((Some(map.clone()), Some(map.clone()), s));
        };
        let s_cut = on_grid(whole.value_at(tau.to_f64()))?;

        let keyed = |keys: Vec<Keyframe>| {
            Some(Property {
                animation: Animation::Keyframed(keys),
                extra: map.extra.clone(),
            })
        };
        let left = keyed(keys.iter().filter(|k| k.time <= tau).copied().collect())?;
        let mut rebased = Vec::new();
        for k in keys.iter().filter(|k| k.time >= tau) {
            rebased.push(Keyframe {
                time: k.time.checked_sub(tau).ok()?,
                ..*k
            });
        }
        let right = keyed(rebased)?;
        Some((Some(left), Some(right), s_cut))
    }

    /// Cut this clip at layer-local time `at` into two clips whose retimes
    /// exactly partition the original (docs/03-DATA-MODEL.md §5.3,
    /// docs/04-RETIMING.md §8.1, the beat-sync covenant: `place` never moves,
    /// source positions stay exact).
    ///
    /// **An eased speed ramp cuts like anything else**. A span is one
    /// cubic, and a cubic splits into two cubics that *are* the original curve
    /// rather than an approximation of it, so the razor through the middle of a
    /// ramp leaves two clips whose speeds concatenate to the speed that was
    /// there before — sampled at every frame across the span, the halves and
    /// the original agree to the last bit.
    ///
    /// **The fields divide the way the sound does**: the left piece keeps the
    /// fade in and the right the fade out, both keep the fx bypass and the
    /// gain, and each takes its own copy of the effect stack.
    ///
    /// None when `at` is not strictly inside the clip — an end is not a cut —
    /// or when the map is expression-driven, which cannot be split without
    /// rewriting it ([`Self::map_split`]).
    pub fn cut(&self, at: Rational) -> Option<(Clip, Clip)> {
        let tau_clip = at.checked_sub(self.place_start).ok()?;
        if tau_clip <= Rational::ZERO || tau_clip >= self.place_duration {
            return None;
        }
        let (left_retime, right_retime, s_cut) = self.map_split(tau_clip)?;
        let right_duration = self.place_duration.checked_sub(tau_clip).ok()?;
        let left = Clip {
            id: Uuid::now_v7(),
            source: self.source,
            source_in: self.source_in,
            source_out: s_cut,
            place_start: self.place_start,
            place_duration: tau_clip,
            retime: left_retime,
            interpolation: self.interpolation.clone(),
            // The outside ends keep their fades and no fade is put at the cut:
            // the two halves abut sample-exactly and play as the one sound did.
            fade_in: self.fade_in,
            fade_out: Fade::default(),
            effects: respawn(&self.effects),
            fx: self.fx,
            gain_db: self.gain_db,
            link: self.link,
            extra: self.extra.clone(),
        };
        let right = Clip {
            id: Uuid::now_v7(),
            source: self.source,
            source_in: s_cut,
            source_out: self.source_out,
            place_start: at,
            place_duration: right_duration,
            retime: right_retime,
            interpolation: self.interpolation.clone(),
            fade_in: Fade::default(),
            fade_out: self.fade_out,
            effects: respawn(&self.effects),
            fx: self.fx,
            gain_db: self.gain_db,
            link: self.link,
            extra: self.extra.clone(),
        };
        Some((left, right))
    }

    /// Slide the clip along the Sequence layer by `delta` (docs/04-RETIMING.md
    /// §8.2): its position moves, but the source window, local time and retime
    /// are untouched — the same frames play, just earlier or later on the row.
    /// None if the clip would start before the layer origin, or on overflow.
    pub fn slide(&self, delta: Rational) -> Option<Clip> {
        let place_start = self.place_start.checked_add(delta).ok()?;
        if place_start.is_negative() {
            return None;
        }
        Some(Clip {
            place_start,
            ..self.clone()
        })
    }

    /// Slip the clip by `delta` of source time: its place and its length on
    /// the row stay as they are, and what it shows moves, later in the source
    /// for a positive `delta`. A keyframed map moves whole, so a ramp keeps
    /// its shape and only starts from a different frame.
    ///
    /// None if the clip would start before the top of its source, on
    /// overflow, or when the map is expression-driven, for the reason
    /// [`Self::map_split`] gives.
    pub fn slip(&self, delta: Rational) -> Option<Clip> {
        let source_in = self.source_in.checked_add(delta).ok()?;
        if source_in.is_negative() {
            return None;
        }
        let source_out = self.source_out.checked_add(delta).ok()?;
        let by = delta.to_f64();
        let retime = match &self.retime {
            None => None,
            Some(map) => Some(Property {
                animation: match &map.animation {
                    Animation::Static(v) => Animation::Static(v + by),
                    Animation::Keyframed(keys) => Animation::Keyframed(
                        keys.iter()
                            .map(|k| Keyframe {
                                value: k.value + by,
                                ..*k
                            })
                            .collect(),
                    ),
                    Animation::Expression(_) => return None,
                },
                extra: map.extra.clone(),
            }),
        };
        Some(Clip {
            source_in,
            source_out,
            retime,
            ..self.clone()
        })
    }

    /// Trim the clip's tail inward to end at layer time `new_end`
    /// (docs/04-RETIMING.md §8.2, non-ripple): the retime is split at the new
    /// edge and the outside discarded, so the kept portion plays exactly as
    /// before. The clip keeps its identity and its start. None if `new_end` is
    /// not strictly inside the clip (trimming *outward* extends per §7.3, which
    /// needs the source's available length and is a separate op).
    pub fn trim_end(&self, new_end: Rational) -> Option<Clip> {
        let tau = new_end.checked_sub(self.place_start).ok()?;
        if tau <= Rational::ZERO || tau >= self.place_duration {
            return None;
        }
        let (left, _, source_out) = self.map_split(tau)?;
        Some(Clip {
            source_out,
            place_duration: tau,
            retime: left,
            ..self.clone()
        })
    }

    /// Trim the clip's head inward to start at layer time `new_start`
    /// (docs/04-RETIMING.md §8.2, non-ripple): the retime is split at the new
    /// edge, the outside discarded, and the kept portion's local time re-based
    /// to zero — so it still plays exactly as before, just entered later. The
    /// clip keeps its identity. None if `new_start` is not strictly inside the
    /// clip (outward trims extend per §7.3, a separate op).
    pub fn trim_start(&self, new_start: Rational) -> Option<Clip> {
        let tau = new_start.checked_sub(self.place_start).ok()?;
        if tau <= Rational::ZERO || tau >= self.place_duration {
            return None;
        }
        let (_, right, source_in) = self.map_split(tau)?;
        let place_duration = self.place_duration.checked_sub(tau).ok()?;
        Some(Clip {
            source_in,
            place_start: new_start,
            place_duration,
            retime: right,
            ..self.clone()
        })
    }

    /// Extend the clip's tail outward to end at layer time `new_end`
    /// (docs/04-RETIMING.md §7.3): the map is carried on at the speed it was
    /// already going, so a tail that was frozen stays frozen and a moving one
    /// keeps moving. The clip keeps its start; nothing else on the row moves.
    ///
    /// None when `new_end` is not actually past the current end, or on
    /// overflow. Running past the media it has is *legal* — that is overrun,
    /// and it renders as a held frame (§7.2) — so it is not refused here.
    pub fn extend_end(&self, new_end: Rational) -> Option<Clip> {
        let duration = new_end.checked_sub(self.place_start).ok()?;
        if duration <= self.place_duration {
            return None;
        }
        self.extended(duration.checked_sub(self.place_duration).ok()?, false)
    }

    /// Extend the clip's head outward to start at layer time `new_start`, the
    /// mirror of [`Self::extend_end`]: the map is carried *backwards* at the
    /// speed it starts with, so the clip enters earlier showing earlier
    /// source. The clip's end never moves.
    pub fn extend_start(&self, new_start: Rational) -> Option<Clip> {
        if new_start >= self.place_start || new_start.is_negative() {
            return None;
        }
        self.extended(self.place_start.checked_sub(new_start).ok()?, true)
    }

    /// The shared body of the two extends: grow the clip by `added` at one end,
    /// carrying the map on (or back) at the speed that end was already going.
    fn extended(&self, added: Rational, at_start: bool) -> Option<Clip> {
        let duration = self.place_duration.checked_add(added).ok()?;
        let speed = self
            .end_speeds()
            .map_or(1.0, |(v0, v1)| if at_start { v0 } else { v1 });
        // How much source the growth consumes, on the grid.
        let consumed =
            Rational::from_f64_on_grid(speed * added.to_f64(), Rational::FLICK_DEN).ok()?;

        let flat = |time: Rational, value: f64| Keyframe {
            time,
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let retime = match &self.retime {
            None => None,
            Some(map) => {
                let Animation::Keyframed(keys) = &map.animation else {
                    return None;
                };
                let grown = if at_start {
                    // Every key moves later in clip time by what was added at
                    // the front, and a new one opens the map earlier in source.
                    let first = keys.first()?;
                    let mut grown =
                        vec![flat(Rational::ZERO, first.value - speed * added.to_f64())];
                    for k in keys {
                        grown.push(Keyframe {
                            time: k.time.checked_add(added).ok()?,
                            ..*k
                        });
                    }
                    grown
                } else {
                    let last = keys.last()?;
                    let mut grown = keys.clone();
                    grown.push(flat(duration, last.value + speed * added.to_f64()));
                    grown
                };
                Some(Property {
                    animation: Animation::Keyframed(grown),
                    extra: map.extra.clone(),
                })
            }
        };
        Some(if at_start {
            Clip {
                place_start: self.place_start.checked_sub(added).ok()?,
                place_duration: duration,
                source_in: self.source_in.checked_sub(consumed).ok()?,
                retime,
                ..self.clone()
            }
        } else {
            Clip {
                place_duration: duration,
                source_out: self.source_out.checked_add(consumed).ok()?,
                retime,
                ..self.clone()
            }
        })
    }
}

/// Write down the **automatic** tangents either side of `tau` as the beziers
/// they currently resolve to, so a cut there can keep the curve.
///
/// In plain terms: an automatic tangent does not store a direction, it works
/// one out from the keys on either side of it. A cut changes exactly
/// those neighbours — the key before the cut gains the cut as its new
/// neighbour, and the key after it ends up first in a clip of its own with
/// nothing to its left — so an automatic side would quietly re-aim itself and
/// the two halves would stop playing what the whole clip played. Said out loud
/// as the bezier it already was, the tangent cannot drift.
///
/// Only the pair the cut lands between is touched: every other key goes on
/// aiming itself, because nothing about its neighbours changed. An automatic
/// speed depends on its neighbours' *times and values*, never on their sides,
/// so writing these two down does not disturb the ones left automatic.
///
/// A no-op when the property is not keyframed, or when `tau` is outside the
/// keyed range — there is no span for the cut to land in.
fn freeze_auto_around(map: &mut Property, tau: Rational) {
    let Animation::Keyframed(keys) = &mut map.animation else {
        return;
    };
    let t = tau.to_f64();
    let Some(i) = keys
        .windows(2)
        .position(|w| t > w[0].time.to_f64() && t < w[1].time.to_f64())
    else {
        return;
    };
    // Both keys read before either is written: each side is resolved against
    // the map as it stands, not against a half-frozen one.
    let sides = [
        (
            crate::anim::resolved_side(keys, i, false),
            crate::anim::resolved_side(keys, i, true),
        ),
        (
            crate::anim::resolved_side(keys, i + 1, false),
            crate::anim::resolved_side(keys, i + 1, true),
        ),
    ];
    for (offset, (interp_in, interp_out)) in sides.into_iter().enumerate() {
        keys[i + offset].interp_in = interp_in;
        keys[i + offset].interp_out = interp_out;
    }
}

/// The *shape* of a Sequence layer, apart from what it plays: where its cuts
/// fall, where its gaps are, and how each piece is ramped.
///
/// **This is what a depth pass needs.** Cutting one layer to a beat and then
/// cutting a second — a depth render, a mask pass, a duplicate with different
/// effects — to exactly the same beats is work nobody should do twice by hand,
/// and doing it by eye guarantees they drift. Copying the shape and applying
/// it elsewhere makes the second layer follow the first exactly.
///
/// It deliberately carries no *source*: the clips it is applied to keep their
/// own media, which is the entire point — the depth pass is not the footage.
/// Times are the layer's own, so the shape is independent of where either
/// layer sits in the composition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequenceShape {
    pub pieces: Vec<ShapePiece>,
}

/// One piece of a [`SequenceShape`]: where it sits on the row, how far into
/// its own source it starts, and its retime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapePiece {
    pub place_start: Rational,
    pub place_duration: Rational,
    pub source_in: Rational,
    pub retime: Option<Property>,
}

impl SequenceShape {
    /// Read the shape of `clips` — all of them, or one, for the two things
    /// the row's menu offers.
    pub fn of(clips: &[Clip]) -> Self {
        Self {
            pieces: clips
                .iter()
                .map(|c| ShapePiece {
                    place_start: c.place_start,
                    place_duration: c.place_duration,
                    source_in: c.source_in,
                    retime: c.retime.clone(),
                })
                .collect(),
        }
    }

    /// Rebuild `clips` in this shape, keeping their own source.
    ///
    /// `limit` is how far the target row reaches — the extent it already
    /// occupied. A shape longer than that is applied as far as it goes and no
    /// further: the piece straddling the end is trimmed to it and anything
    /// wholly beyond is dropped, so a shape taken from a long clip lands
    /// sensibly on a short one rather than inventing a row that runs past its
    /// media.
    ///
    /// Every piece plays `source`, taking the shape's own trim-in and map, so
    /// the two rows show the same moments of their respective media at the
    /// same times — which is what makes a depth pass line up.
    pub fn apply(&self, source: ClipSource, limit: Rational) -> Vec<Clip> {
        let mut out = Vec::with_capacity(self.pieces.len());
        for piece in &self.pieces {
            if piece.place_start >= limit {
                continue; // wholly past what this row reaches
            }
            let end = piece
                .place_start
                .checked_add(piece.place_duration)
                .unwrap_or(limit);
            let duration = if end > limit {
                match limit.checked_sub(piece.place_start) {
                    Ok(d) => d,
                    Err(_) => continue,
                }
            } else {
                piece.place_duration
            };
            if duration <= Rational::ZERO {
                continue;
            }
            let mut clip = Clip::new(
                source,
                piece.source_in,
                piece.source_in.checked_add(duration).unwrap_or(duration),
                piece.place_start,
                duration,
            );
            clip.retime = piece.retime.clone();
            // A trimmed piece keeps only the part of its map it still has
            // room for, exactly as trimming that clip by hand would.
            if end > limit {
                if let Some(shorter) =
                    clip.trim_end(piece.place_start.checked_add(duration).unwrap_or(limit))
                {
                    clip = shorter;
                }
            }
            if let Some(map) = &clip.retime {
                if let Some(last) = map_last_value(map) {
                    clip.source_out = last;
                }
            }
            out.push(clip);
        }
        out
    }
}

/// A copy of a clip's stack with fresh instance ids, for the piece of a split
/// that is a new clip. An effect is found by its id alone, so two pieces
/// sharing one would answer for each other.
fn respawn(effects: &[EffectInstance]) -> Vec<EffectInstance> {
    effects
        .iter()
        .map(|e| EffectInstance {
            id: Uuid::now_v7(),
            ..e.clone()
        })
        .collect()
}

/// The last source position a map reaches.
fn map_last_value(map: &Property) -> Option<Rational> {
    let Animation::Keyframed(keys) = &map.animation else {
        return None;
    };
    Rational::from_f64_on_grid(keys.last()?.value, Rational::FLICK_DEN).ok()
}

/// Resolve the overlaps a clip has just been dropped into — the **overwrite**
/// edit every NLE does when one clip lands on another.
///
/// The dropped clip wins its whole span, and each clip already under it is
/// dealt with by how much of it is covered:
///
/// * covered entirely — it goes;
/// * covered at one end — that end is trimmed back to the dropped clip's edge;
/// * covered in the middle — it becomes two clips, one either side.
///
/// Everything outside the dropped span is untouched, which is the point: an
/// overwrite is destructive exactly where it lands and nowhere else, so no
/// edit point beyond it moves and nothing ripples.
///
/// The surviving pieces keep playing the frames they played — the trims and
/// the split go through [`Clip::trim_end`], [`Clip::trim_start`] and the same
/// map arithmetic a razor uses, so the half of a clip left beside a dropped
/// one shows exactly what it showed before.
pub fn overwrite_with(clips: &[Clip], dropped: Uuid) -> Vec<Clip> {
    let Some(over) = clips.iter().find(|c| c.id == dropped) else {
        return clips.to_vec();
    };
    let (start, end) = (over.place_start, over.place_end());
    let mut out = Vec::with_capacity(clips.len() + 1);
    for c in clips {
        if c.id == dropped {
            out.push(c.clone());
            continue;
        }
        // Clear of it on either side: nothing to do.
        if c.place_end() <= start || c.place_start >= end {
            out.push(c.clone());
            continue;
        }
        // Buried: it goes.
        if c.place_start >= start && c.place_end() <= end {
            continue;
        }
        // Straddling: one clip either side, and the later piece needs an
        // identity of its own — it is a new clip, not the one that was there.
        if c.place_start < start && c.place_end() > end {
            if let Some(mut left) = c.trim_end(start) {
                // The split divides the fades the way a razor does: the
                // outside ends keep theirs, and nothing is put at the new edge.
                left.fade_out = Fade::default();
                out.push(left);
            }
            if let Some(mut right) = c.trim_start(end) {
                right.id = Uuid::now_v7();
                right.fade_in = Fade::default();
                right.effects = respawn(&right.effects);
                out.push(right);
            }
            continue;
        }
        // Covered at one end.
        let trimmed = if c.place_start < start {
            c.trim_end(start)
        } else {
            c.trim_start(end)
        };
        if let Some(trimmed) = trimmed {
            out.push(trimmed);
        }
    }
    out.sort_by_key(|c| c.place_start);
    out
}

/// Ripple: move every clip that starts at or after `at` by `delta`, and leave
/// the rest where they are. A clip that only straddles `at` started before
/// it, so it stays.
///
/// None if a moved clip would start before the row's zero, or would land on a
/// clip that stayed. Only a move earlier can do the second: a move later
/// opens room and never closes it.
pub fn shift_from(clips: &[Clip], at: Rational, delta: Rational) -> Option<Vec<Clip>> {
    let mut out = Vec::with_capacity(clips.len());
    for c in clips {
        out.push(if c.place_start >= at {
            c.slide(delta)?
        } else {
            c.clone()
        });
    }
    if delta.is_negative() {
        // The clips that stayed, in order of start, each beside the latest
        // end of any up to and including it. A moved clip lands on one of
        // them exactly when some clip starting before the moved one ends also
        // ends after it starts, and that is one search and one comparison.
        // Trying every moved clip against every one that stayed took
        // milliseconds a row on a long cut.
        let mut stayed: Vec<(Rational, Rational)> = clips
            .iter()
            .filter(|c| c.place_start < at)
            .map(|c| (c.place_start, c.place_end()))
            .collect();
        stayed.sort_by_key(|(start, _)| *start);
        let mut latest: Option<Rational> = None;
        for (_, end) in &mut stayed {
            let so_far = latest.map_or(*end, |l| l.max(*end));
            *end = so_far;
            latest = Some(so_far);
        }
        let moved = clips
            .iter()
            .zip(&out)
            .filter(|(was, _)| was.place_start >= at);
        for (_, now) in moved {
            let before = stayed.partition_point(|(start, _)| *start < now.place_end());
            let reaches = before.checked_sub(1).and_then(|last| stayed.get(last));
            if reaches.is_some_and(|(_, end)| *end > now.place_start) {
                return None;
            }
        }
    }
    Some(out)
}

/// Roll the edit point between two abutting clips to layer time `to`: `left`
/// trims as `right` extends, or the other way round, and nothing else on the
/// row moves. Each keeps playing the frames it played wherever it still is.
///
/// None when the two do not meet end to start, when `to` leaves either of
/// them with no length, or when a map cannot be trimmed or carried on.
pub fn roll(clips: &[Clip], left: Uuid, right: Uuid, to: Rational) -> Option<Vec<Clip>> {
    let l = clips.iter().find(|c| c.id == left)?;
    let r = clips.iter().find(|c| c.id == right)?;
    let edit = l.place_end();
    if r.place_start != edit {
        return None;
    }
    let (l, r) = match to.cmp(&edit) {
        std::cmp::Ordering::Less => (l.trim_end(to)?, r.extend_start(to)?),
        std::cmp::Ordering::Greater => (l.extend_end(to)?, r.trim_start(to)?),
        std::cmp::Ordering::Equal => return Some(clips.to_vec()),
    };
    Some(with(clips, &[l, r]))
}

/// Make the two clips at an edit point overlap by `frames` steps of `frame`,
/// centred on it. On a layer that draws, the overlap is a dissolve, and on an
/// audio-only one a crossfade.
///
/// `outgoing` ends where `incoming` starts, or the two already overlap and
/// the edit point is the middle of that. Each is carried on by half the
/// length into its own source, the outgoing clip's end later and the incoming
/// clip's start earlier, and the odd step goes to the incoming clip. `room`
/// is how far each edge may be carried from where it stands, the outgoing
/// clip's first, with None for no limit ([`Clip::spare`]). A clip with less
/// than its half gives what it has, in whole steps, so the overlap is the
/// longest the two allow. Neither edge passes the far end of the other clip
/// or reaches a third, so two clips at most cover any moment.
///
/// No `frames` trims an overlap back to its middle, and the clips abut again.
///
/// None when the two neither meet nor overlap, when a length was asked for
/// and neither clip has any to give, or when a map cannot be trimmed or
/// carried on.
pub fn dissolve(
    clips: &[Clip],
    outgoing: Uuid,
    incoming: Uuid,
    frames: i64,
    frame: Rational,
    room: (Option<Rational>, Option<Rational>),
) -> Option<Vec<Clip>> {
    let out = clips.iter().find(|c| c.id == outgoing)?;
    let inc = clips.iter().find(|c| c.id == incoming)?;
    let (start, end) = (inc.place_start, out.place_end());
    if frames < 0
        || frame <= Rational::ZERO
        || out.place_start > start
        || start > end
        || end > inc.place_end()
    {
        return None;
    }
    // The whole steps in a length, and the length of that many steps.
    let steps = |length: Rational| {
        let count = length.checked_div(frame).ok()?;
        Some(count.num().div_euclid(count.den()))
    };
    let span = |steps: i64| frame.checked_mul(Rational::new(steps, 1).ok()?).ok();
    let overlap = steps(end.checked_sub(start).ok()?)?;
    let point = end.checked_sub(span(overlap / 2)?).ok()?;

    // How far each edge may go: to the other clip's far end, to a third
    // clip, and as far as its own source runs.
    let others = || {
        clips
            .iter()
            .filter(|c| c.id != outgoing && c.id != incoming)
    };
    let later = others().map(|c| c.place_start).filter(|s| *s >= point);
    let mut latest = later.chain([inc.place_end()]).min()?;
    let earlier = others().map(Clip::place_end).filter(|e| *e <= point);
    let mut earliest = earlier.chain([out.place_start]).max()?;
    if let Some(room) = room.0 {
        latest = latest.min(end.checked_add(room).ok()?);
    }
    if let Some(room) = room.1 {
        earliest = earliest.max(start.checked_sub(room).ok()?);
    }
    let after = (frames / 2).min(steps(latest.checked_sub(point).ok()?)?);
    let before = (frames - frames / 2).min(steps(point.checked_sub(earliest).ok()?)?);
    if frames > 0 && after + before == 0 {
        return None;
    }
    let new_end = point.checked_add(span(after)?).ok()?;
    let new_start = point.checked_sub(span(before)?).ok()?;
    let out = match new_end.cmp(&end) {
        std::cmp::Ordering::Less => out.trim_end(new_end)?,
        std::cmp::Ordering::Greater => out.extend_end(new_end)?,
        std::cmp::Ordering::Equal => out.clone(),
    };
    let inc = match new_start.cmp(&start) {
        std::cmp::Ordering::Less => inc.extend_start(new_start)?,
        std::cmp::Ordering::Greater => inc.trim_start(new_start)?,
        std::cmp::Ordering::Equal => inc.clone(),
    };
    Some(with(clips, &[out, inc]))
}

/// Slide a clip between its neighbours by `delta`, the editor's slide: the
/// clip keeps its length and its frames, the clip that ends where it starts
/// follows its head and the clip that starts where it ends follows its tail,
/// so the three stay abutting and the rest of the row never moves.
/// [`Clip::slide`] is the plain move along the row.
///
/// A side with no neighbour is open row, and the clip may move into it. None
/// when a neighbour has no length left to give, when the clip would land on
/// anything else, or when it would start before the row's zero.
pub fn slide_between(clips: &[Clip], clip: Uuid, delta: Rational) -> Option<Vec<Clip>> {
    let c = clips.iter().find(|c| c.id == clip)?;
    if delta == Rational::ZERO {
        return Some(clips.to_vec());
    }
    let moved = c.slide(delta)?;
    let (start, end) = (moved.place_start, moved.place_end());
    let later = !delta.is_negative();
    let mut changed = Vec::with_capacity(3);
    if let Some(before) = clips
        .iter()
        .find(|n| n.id != clip && n.place_end() == c.place_start)
    {
        changed.push(if later {
            before.extend_end(start)?
        } else {
            before.trim_end(start)?
        });
    }
    if let Some(after) = clips
        .iter()
        .find(|n| n.id != clip && n.place_start == c.place_end())
    {
        changed.push(if later {
            after.trim_start(end)?
        } else {
            after.extend_start(end)?
        });
    }
    changed.push(moved);
    let out = with(clips, &changed);
    if out.iter().any(|o| o.id != clip && overlaps(o, start, end)) {
        return None;
    }
    Some(out)
}

/// Whether `clip` shares any of the row with the span `start..end`.
fn overlaps(clip: &Clip, start: Rational, end: Rational) -> bool {
    clip.place_start < end && start < clip.place_end()
}

/// `clips` with each of `changed` standing in for the clip of the same id.
fn with(clips: &[Clip], changed: &[Clip]) -> Vec<Clip> {
    clips
        .iter()
        .map(|c| changed.iter().find(|n| n.id == c.id).unwrap_or(c).clone())
        .collect()
}

/// The layer-local span the clips occupy: the first clip's start to the last
/// clip's end. None for a Sequence layer with no clips at all, which
/// has no length of its own to take.
///
/// Clips are not required to be in order in the list, so both ends are found
/// by scanning rather than by reading the ends — reordering a Sequence layer
/// is exactly the operation this has to survive.
pub fn clips_span(clips: &[Clip]) -> Option<(Rational, Rational)> {
    let start = clips.iter().map(|c| c.place_start).min()?;
    let end = clips.iter().map(Clip::place_end).max()?;
    Some((start, end))
}

/// The clip active at layer-local time `lt`, or None if `lt` is in a gap
/// (transparent) or past the end.
///
/// The plain answer: one clip. Two clips on a row may overlap, and there the
/// first as the clips are stored is the one given. A picture asks
/// [`shown_at`], which answers with both and the dissolve between them, and
/// the mixer asks for every clip that sounds at `lt`, so it walks the list
/// itself and never comes here.
pub fn active_clip(clips: &[Clip], lt: f64) -> Option<&Clip> {
    clips.iter().find(|c| c.contains(lt))
}

/// Resolve layer-local time `lt` to `(active clip id, source, source time)`,
/// or None in a gap. One clip and no fades: see [`shown_at`] for the picture.
pub fn resolve(clips: &[Clip], lt: f64) -> Option<(Uuid, ClipSource, f64)> {
    active_clip(clips, lt).map(|c| (c.id, c.source, c.held_source_time(lt)))
}

/// What a Sequence layer that draws a picture shows at one layer time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shown<'a> {
    /// A gap, or a time off either end of the row: transparent.
    Nothing,
    /// One clip. `gain` is 1 where it is whole, and inside a fade at an end
    /// no other clip overlaps it is how opaque the clip is.
    One { clip: &'a Clip, gain: f64 },
    /// Two clips overlapping, which is a dissolve: `outgoing` at `1 - gain`
    /// and `incoming` at `gain`, added together.
    Two {
        outgoing: &'a Clip,
        incoming: &'a Clip,
        gain: f64,
    },
}

/// What a picture layer shows at layer-local time `lt`: nothing, one clip, or
/// two under a dissolve.
///
/// An overlap is a dissolve, and is as long as the overlap. The clip that
/// starts later comes in as the other goes out, at `fade_in.shape.gain(u)`
/// against one less that, where `u`
/// runs from 0 at the start of the overlap to 1 at its end, and neither
/// clip's stored seconds are read inside it: the reading the sound mix makes
/// of a crossfade. Outside an overlap a clip comes up over its own
/// `fade_in.seconds` from its start and goes down over its `fade_out.seconds`
/// before its end, each through its own shape. An end another clip overlaps
/// has no fade of its own, because the dissolve is the fade there.
///
/// At most two clips cover a moment on a row the edit commands made. Where
/// more do, the first two as the clips are stored are the ones read.
///
/// ponytail: inside an overlap only the dissolve is read, so a fade at a
/// clip's other end that reaches into the overlap is not multiplied in. It
/// takes a clip shorter than its fade and its dissolve together to see it.
/// The upgrade is a gain for each of the two clips here.
pub fn shown_at(clips: &[Clip], lt: f64) -> Shown<'_> {
    let mut live = clips.iter().filter(|c| c.contains(lt));
    let Some(first) = live.next() else {
        return Shown::Nothing;
    };
    let Some(second) = live.next() else {
        return Shown::One {
            clip: first,
            gain: lone_gain(clips, first, lt),
        };
    };
    // Two that start together keep the order they are stored in.
    let (outgoing, incoming) = if second.place_start < first.place_start {
        (second, first)
    } else {
        (first, second)
    };
    let start = incoming.place_start.to_f64();
    let end = outgoing.place_end().min(incoming.place_end()).to_f64();
    let u = if end > start {
        (lt - start) / (end - start)
    } else {
        1.0
    };
    Shown::Two {
        outgoing,
        incoming,
        gain: opacity(incoming.fade_in.shape, u),
    }
}

/// A fade's gain as an opacity. A Custom shape may overshoot, which a level
/// can follow and an opacity cannot. The bounds are constants, so `clamp` has
/// no reversed pair to panic on (docs/14 §4).
fn opacity(shape: FadeShape, u: f64) -> f64 {
    shape.gain(u).clamp(0.0, 1.0)
}

/// How opaque `clip` is at `lt` where it is the only clip there: its own two
/// fades, each read only at an end no other clip overlaps. Exactly 1 outside
/// both, whatever the shape, so a moment no fade touches is a plain one.
fn lone_gain(clips: &[Clip], clip: &Clip, lt: f64) -> f64 {
    let (start, end) = (clip.place_start, clip.place_end());
    // A stored fade is never longer than its clip, as the sound mix reads it.
    let span = clip.place_duration.to_f64();
    let stored = |fade: &Fade| fade.seconds.to_f64().max(0.0).min(span);
    let mut gain = 1.0;
    let head = stored(&clip.fade_in);
    let since = lt - start.to_f64();
    if since < head {
        let joined = |o: &Clip| o.id != clip.id && o.place_start <= start && start < o.place_end();
        if !clips.iter().any(joined) {
            gain *= opacity(clip.fade_in.shape, since / head);
        }
    }
    let tail = stored(&clip.fade_out);
    let left = end.to_f64() - lt;
    if left < tail {
        let joined = |o: &Clip| o.id != clip.id && o.place_start < end && end <= o.place_end();
        if !clips.iter().any(joined) {
            gain *= opacity(clip.fade_out.shape, left / tail);
        }
    }
    gain
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    fn clip(src: Uuid, place_start: i64, place_dur: i64) -> Clip {
        Clip::new(
            ClipSource::Footage(src),
            rat(0, 1),
            rat(place_dur, 1),
            rat(place_start, 1),
            rat(place_dur, 1),
        )
    }

    #[test]
    fn with_ramp_sets_a_speed_ramp() {
        // 4 s clip from source 0, speed running straight 1× → 3×: the source
        // used is the area under that line, 4 · (1 + 3)/2 = 8.
        let base = clip(Uuid::now_v7(), 0, 4);
        let ramp = base.with_ramp(rat(1, 1), rat(3, 1));
        assert_eq!(ramp.place_duration, base.place_duration); // place held
        assert_eq!(ramp.source_out, rat(8, 1));
        let (v0, v1) = ramp.ramp_view().unwrap();
        assert!((v0 - 1.0).abs() < 1e-9 && (v1 - 3.0).abs() < 1e-9);
        // A ramp has no single constant speed.
        assert_eq!(ramp.constant_speed(), None);
        // And its first frame is still its own trim-in (the frame pinning).
        assert!((ramp.source_time(0.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn resolution_picks_the_clip_under_the_playhead() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        // Clip A [0,2), then a gap [2,3), then clip B [3,5).
        let clips = vec![clip(a, 0, 2), clip(b, 3, 2)];
        assert_eq!(resolve(&clips, 1.0).unwrap().1, ClipSource::Footage(a));
        assert_eq!(resolve(&clips, 4.0).unwrap().1, ClipSource::Footage(b));
        // The gap and past-the-end render transparent (None).
        assert!(resolve(&clips, 2.5).is_none());
        assert!(resolve(&clips, 5.0).is_none());
        // Boundaries: start inclusive, end exclusive.
        assert!(resolve(&clips, 0.0).is_some());
        assert!(resolve(&clips, 2.0).is_none());
        assert!(resolve(&clips, 3.0).is_some());
    }

    /// An overlap on a picture row is a dissolve as long as the overlap, read
    /// through the incoming clip's shape, and a stored fade is read only at
    /// an end no other clip overlaps. A butt cut overlaps nothing.
    #[test]
    fn a_picture_row_dissolves_across_an_overlap_and_fades_at_a_lone_end() {
        let linear = |seconds: i64| Fade {
            seconds: rat(seconds, 1),
            shape: FadeShape::Linear,
        };
        // A on [0, 4) and B on [2, 6) share [2, 4). C butts B at 6.
        let mut a = clip(Uuid::now_v7(), 0, 4);
        let mut b = clip(Uuid::now_v7(), 2, 4);
        let mut c = clip(Uuid::now_v7(), 6, 2);
        (a.fade_in, b.fade_out, c.fade_in) = (linear(1), linear(1), linear(1));
        // Seconds stored at the two ends inside the overlap, longer than it
        // is: the overlap is the length there, so these are never read.
        (a.fade_out, b.fade_in) = (linear(3), linear(3));
        // Stored out of order, as a row may be.
        let clips = vec![b.clone(), c.clone(), a.clone()];
        let one = |clip: &Clip, gain: f64| (None, Some(clip.id), gain);
        let shown = |lt: f64| match shown_at(&clips, lt) {
            Shown::Nothing => (None, None, 0.0),
            Shown::One { clip, gain } => one(clip, gain),
            Shown::Two {
                outgoing,
                incoming,
                gain,
            } => (Some(outgoing.id), Some(incoming.id), gain),
        };
        assert_eq!(shown(-1.0), (None, None, 0.0));
        assert_eq!(shown(8.0), (None, None, 0.0));
        // A comes up over its own second, then is whole until the overlap.
        assert_eq!(shown(0.0), one(&a, 0.0));
        assert_eq!(shown(0.5), one(&a, 0.5));
        assert_eq!(shown(1.5), one(&a, 1.0));
        // B comes in over A across the two seconds they share.
        assert_eq!(shown(2.0), (Some(a.id), Some(b.id), 0.0));
        assert_eq!(shown(3.0), (Some(a.id), Some(b.id), 0.5));
        // B is whole the moment the overlap ends, and goes down to the cut.
        assert_eq!(shown(4.0), one(&b, 1.0));
        assert_eq!(shown(5.5), one(&b, 0.5));
        assert_eq!(shown(6.5), one(&c, 0.5));
    }

    #[test]
    fn resolve_holds_at_the_trimmed_end_on_overrun() {
        // A clip on layer [2,6) trimmed to source [10,12) whose identity retime
        // maps the whole 4s span to source [10,14): once the map passes
        // source_out=12, the render/key sample must HOLD at 12 (the boundary
        // frame of the trimmed extent, docs/04 §7.2), never run on to 13/14 —
        // media past the trim. The raw source_time stays unclamped so overrun
        // detection still sees past the boundary.
        let src = Uuid::now_v7();
        let clips = [Clip::new(
            ClipSource::Footage(src),
            rat(10, 1),
            rat(12, 1),
            rat(2, 1),
            rat(4, 1),
        )];
        let st = |lt: f64| resolve(&clips, lt).unwrap().2;
        assert!((st(2.0) - 10.0).abs() < 1e-9); // clip start → source_in
        assert!((st(4.0) - 12.0).abs() < 1e-9); // reaches source_out
        assert!((st(5.0) - 12.0).abs() < 1e-9); // overrun holds at the trim
        assert!((st(5.9) - 12.0).abs() < 1e-9); // still held, not ~13.9
                                                // The raw map is unclamped (overrun detection still sees past the trim).
        assert!((clips[0].source_time(5.0) - 13.0).abs() < 1e-9);
    }

    #[test]
    fn sliding_moves_the_clip_but_not_its_content() {
        // Clip at layer [2,6), source [0,4). Slide +3 → layer [5,9), same source.
        let src = Uuid::now_v7();
        let c = clip(src, 2, 4);
        let s = c.slide(rat(3, 1)).unwrap();
        assert_eq!(s.place_start, rat(5, 1));
        assert_eq!(s.place_duration, c.place_duration); // duration unchanged
        assert_eq!(s.source_in, c.source_in); // source window untouched
        assert_eq!(s.source_out, c.source_out);
        // The same source moments play, just later on the row (map untouched).
        assert!((s.source_time(5.0) - c.source_time(2.0)).abs() < 1e-9);
        assert!((s.source_time(7.0) - c.source_time(4.0)).abs() < 1e-9);
        // Sliding before the layer origin is refused.
        assert!(c.slide(rat(-3, 1)).is_none());
    }

    #[test]
    fn trimming_an_edge_inward_keeps_the_rest_in_place() {
        let src = Uuid::now_v7();
        // Clip at layer [2,6), source [0,4) at natural rate.
        let c = clip(src, 2, 4);
        // Trim the tail to end at 5 → layer [2,5), source [0,3).
        let t = c.trim_end(rat(5, 1)).unwrap();
        assert_eq!(t.id, c.id); // same clip identity
        assert_eq!(t.place_start, rat(2, 1));
        assert_eq!(t.place_duration, rat(3, 1));
        assert_eq!(t.source_out, rat(3, 1));
        for &lt in &[2.0, 3.5, 4.9] {
            assert!(
                (t.source_time(lt) - c.source_time(lt)).abs() < 1e-9,
                "tail @ {lt}"
            );
        }
        // Trim the head to start at 4 → layer [4,6), source [2,4), re-based.
        let h = c.trim_start(rat(4, 1)).unwrap();
        assert_eq!(h.id, c.id);
        assert_eq!(h.place_start, rat(4, 1));
        assert_eq!(h.place_duration, rat(2, 1));
        assert_eq!(h.source_in, rat(2, 1));
        for &lt in &[4.0, 5.0, 5.9] {
            assert!(
                (h.source_time(lt) - c.source_time(lt)).abs() < 1e-9,
                "head @ {lt}"
            );
        }
        // Outward trims (need §7.3 extend) and out-of-range edges are refused.
        assert!(c.trim_end(rat(7, 1)).is_none());
        assert!(c.trim_start(rat(1, 1)).is_none());
        assert!(c.trim_end(rat(2, 1)).is_none()); // zero length
    }

    /// The overwrite edit: a clip dropped on others takes its whole
    /// span, and each clip under it is trimmed, split, or removed.
    #[test]
    fn dropping_a_clip_overwrites_what_is_under_it() {
        let src = Uuid::now_v7();
        // Three in a row: [0,4) [4,8) [8,12).
        let a = clip(src, 0, 4);
        let b = clip(src, 4, 4);
        let c = clip(src, 8, 4);

        // A clip covering the whole middle one and nothing else: it goes.
        let mut over = clip(src, 4, 4);
        over.id = Uuid::now_v7();
        let out = overwrite_with(&[a.clone(), b.clone(), c.clone(), over.clone()], over.id);
        assert_eq!(out.len(), 3, "the buried clip went");
        assert!(!out.iter().any(|k| k.id == b.id));
        assert!(out.iter().any(|k| k.id == a.id) && out.iter().any(|k| k.id == c.id));

        // Landing across the join of the first two: the first is trimmed back
        // and the second trimmed forward, both keeping their identities.
        let mut across = clip(src, 2, 4); // [2,6)
        across.id = Uuid::now_v7();
        let out = overwrite_with(&[a.clone(), b.clone(), across.clone()], across.id);
        let left = out
            .iter()
            .find(|k| k.id == a.id)
            .expect("the first survives");
        let right = out.iter().find(|k| k.id == b.id).expect("the second too");
        assert_eq!(left.place_end(), rat(2, 1), "trimmed back to the drop");
        assert_eq!(right.place_start, rat(6, 1), "and forward from its end");

        // Landing inside one clip: it becomes two, one either side.
        let mut inside = clip(src, 5, 2); // [5,7) inside b's [4,8)
        inside.id = Uuid::now_v7();
        let out = overwrite_with(&[b.clone(), inside.clone()], inside.id);
        assert_eq!(out.len(), 3, "the clip under it became two");
        let pieces: Vec<_> = out.iter().filter(|k| k.id != inside.id).collect();
        assert_eq!(pieces[0].place_start, rat(4, 1));
        assert_eq!(pieces[0].place_end(), rat(5, 1));
        assert_eq!(pieces[1].place_start, rat(7, 1));
        assert_eq!(pieces[1].place_end(), rat(8, 1));
        assert_ne!(pieces[0].id, pieces[1].id, "the halves are distinct clips");

        // A clip clear of everything disturbs nothing.
        let mut clear = clip(src, 20, 2);
        clear.id = Uuid::now_v7();
        let all = vec![a.clone(), b.clone(), c.clone(), clear.clone()];
        assert_eq!(overwrite_with(&all, clear.id).len(), 4);
    }

    /// A ripple moves what starts at or after the point and nothing else, and
    /// refuses rather than land a moved clip on one that stayed.
    #[test]
    fn a_ripple_shifts_what_follows_and_refuses_to_overlap() {
        let src = Uuid::now_v7();
        // [0,4) [4,8), a gap, then [10,12).
        let clips = vec![clip(src, 0, 4), clip(src, 4, 4), clip(src, 10, 2)];
        let starts = |c: &[Clip]| c.iter().map(|c| c.place_start).collect::<Vec<_>>();

        let later = shift_from(&clips, rat(4, 1), rat(2, 1)).unwrap();
        assert_eq!(starts(&later), vec![rat(0, 1), rat(6, 1), rat(12, 1)]);
        assert_eq!(later[1].source_in, clips[1].source_in, "the same frames");
        // A clip that straddles the point started before it, so it stays.
        let straddled = shift_from(&clips, rat(5, 1), rat(2, 1)).unwrap();
        assert_eq!(starts(&straddled), vec![rat(0, 1), rat(4, 1), rat(12, 1)]);

        // Closing the gap exactly is fine. Any further lands on the clip that
        // stayed, and nothing moves before the row's zero.
        let closed = shift_from(&clips, rat(10, 1), rat(-2, 1)).unwrap();
        assert_eq!(starts(&closed), vec![rat(0, 1), rat(4, 1), rat(8, 1)]);
        assert!(shift_from(&clips, rat(10, 1), rat(-3, 1)).is_none());
        assert!(shift_from(&clips, rat(0, 1), rat(-1, 1)).is_none());
    }

    /// The ripple's refusal is the same answer as trying every moved clip
    /// against every clip that stayed, on rows in any order, with gaps and
    /// with overlaps. A ripple wrongly allowed leaves two clips on one frame.
    #[test]
    fn a_ripple_refuses_exactly_when_a_moved_clip_lands_on_one_that_stayed() {
        let src = Uuid::now_v7();
        // A small generator with a fixed seed: the same rows every run.
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |below: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % below) as i64
        };
        let (mut refused, mut allowed) = (0, 0);
        for _ in 0..300 {
            let mut clips: Vec<Clip> = Vec::new();
            let mut cursor = 0;
            for _ in 0..(2 + next(12)) {
                // Mostly after the last clip, sometimes back over it.
                cursor = (cursor + next(6) - 1).max(0);
                let len = 1 + next(5);
                clips.push(clip(src, cursor, len));
                cursor += len;
            }
            // Out of order: the list is stored however it was last written.
            for i in (1..clips.len()).rev() {
                clips.swap(i, next(i as u64 + 1) as usize);
            }
            let at = rat(next(cursor as u64 + 2), 1);
            let delta = rat(-1 - next(4), 1);

            let lands = clips
                .iter()
                .filter(|m| m.place_start >= at)
                .filter_map(|m| m.slide(delta))
                .any(|m| {
                    clips
                        .iter()
                        .filter(|s| s.place_start < at)
                        .any(|s| overlaps(&m, s.place_start, s.place_end()))
                });
            let before_zero = clips
                .iter()
                .any(|m| m.place_start >= at && m.slide(delta).is_none());
            let moved = shift_from(&clips, at, delta);
            assert_eq!(moved.is_none(), lands || before_zero, "{clips:?} {at:?}");
            if moved.is_none() {
                refused += 1;
            } else {
                allowed += 1;
            }
        }
        assert!(
            refused > 30 && allowed > 30,
            "{refused} refused, {allowed} allowed"
        );
    }

    /// A roll moves the join and nothing else, and both clips go on playing
    /// the frames they played.
    #[test]
    fn rolling_an_edit_point_moves_only_the_join() {
        let src = Uuid::now_v7();
        // [0,4), then [4,8) entered two seconds into its source, then [8,10).
        let left = clip(src, 0, 4);
        let right = Clip::new(
            ClipSource::Footage(src),
            rat(2, 1),
            rat(6, 1),
            rat(4, 1),
            rat(4, 1),
        );
        let clips = vec![left.clone(), right.clone(), clip(src, 8, 2)];

        for to in [rat(3, 1), rat(5, 1)] {
            let out = roll(&clips, left.id, right.id, to).unwrap();
            assert_eq!(out[0].place_start, rat(0, 1));
            assert_eq!(out[0].place_end(), to);
            assert_eq!(out[1].place_start, to);
            assert_eq!(out[1].place_end(), rat(8, 1));
            assert_eq!(out[2], clips[2], "the rest of the row is untouched");
            assert!((out[0].source_time(2.0) - left.source_time(2.0)).abs() < 1e-9);
            assert!((out[1].source_time(6.0) - right.source_time(6.0)).abs() < 1e-9);
        }
        assert_eq!(
            roll(&clips, left.id, right.id, rat(3, 1)).unwrap()[1].source_in,
            rat(1, 1),
            "the right clip opens a second earlier in its source"
        );

        // Past either clip's far end, or across a pair that does not meet.
        assert!(roll(&clips, left.id, right.id, rat(8, 1)).is_none());
        assert!(roll(&clips, left.id, right.id, rat(0, 1)).is_none());
        assert!(roll(&clips, left.id, clips[2].id, rat(5, 1)).is_none());
    }

    /// A dissolve is centred on the edit point with the odd frame on the
    /// incoming clip, is held to the source each clip has left, and comes off
    /// leaving both clips exactly as they were. One made wrongly shows frames
    /// the media does not have, or moves the cut it was put on.
    #[test]
    fn a_dissolve_is_centred_held_to_the_media_and_comes_off_clean() {
        let src = Uuid::now_v7();
        let (frame, media) = (rat(1, 10), Some(rat(6, 1)));
        // Two clips of a six second file meeting at 4 s: the first has one
        // second of it left after its end, the second two before its start.
        let a = Clip::new(
            ClipSource::Footage(src),
            rat(1, 1),
            rat(5, 1),
            rat(0, 1),
            rat(4, 1),
        );
        let b = Clip::new(
            ClipSource::Footage(src),
            rat(2, 1),
            rat(6, 1),
            rat(4, 1),
            rat(4, 1),
        );
        let clips = vec![a.clone(), b.clone()];
        let room = (a.spare(false, media), b.spare(true, media));
        assert_eq!(room, (Some(rat(1, 1)), Some(rat(2, 1))));

        // Five frames: two after the edit point and three before it.
        let five = dissolve(&clips, a.id, b.id, 5, frame, room).unwrap();
        assert_eq!(five[0].place_end(), rat(42, 10));
        assert_eq!(five[1].place_start, rat(37, 10));
        assert_eq!(five[0].source_out, rat(52, 10));
        assert_eq!(five[1].source_in, rat(17, 10));
        assert_eq!((five[0].id, five[1].id), (a.id, b.id));
        // Each still shows at the edit point what it showed there.
        assert_eq!(five[1].source_at(rat(4, 1)), b.source_at(rat(4, 1)));

        // Removed, from the odd overlap, both are as they were.
        let off = dissolve(&five, a.id, b.id, 0, frame, room).unwrap();
        assert_eq!(off, clips);

        // Forty frames asks twenty of each. The first clip has ten.
        let held = dissolve(&clips, a.id, b.id, 40, frame, room).unwrap();
        assert_eq!(held[0].place_end(), rat(5, 1));
        assert_eq!(held[0].source_out, rat(6, 1), "to the end of its media");
        assert_eq!(held[1].place_start, rat(2, 1));

        // Neither has any to give, and at twice the speed a clip gives half.
        let none = (Some(rat(0, 1)), Some(rat(0, 1)));
        assert!(dissolve(&clips, a.id, b.id, 10, frame, none).is_none());
        let fast = clip(src, 0, 4).with_ramp(rat(2, 1), rat(2, 1));
        assert_eq!(fast.spare(false, Some(rat(10, 1))), Some(rat(1, 1)));
    }

    /// The exact source moment a clip shows: from its trim, through its map,
    /// and held at the trim as the picture is. Match frame opens the source
    /// on this.
    #[test]
    fn source_at_is_exact_follows_the_map_and_holds_at_the_trim() {
        let src = Uuid::now_v7();
        let trimmed = Clip::new(
            ClipSource::Footage(src),
            rat(10, 1),
            rat(12, 1),
            rat(2, 1),
            rat(4, 1),
        );
        assert_eq!(trimmed.source_at(rat(7, 2)), Some(rat(23, 2)));
        assert_eq!(trimmed.source_at(rat(5, 1)), Some(rat(12, 1)), "held");
        // Speed running 1x to 3x over four seconds has used three by two.
        let ramp = clip(src, 0, 4).with_ramp(rat(1, 1), rat(3, 1));
        let shown = ramp.source_at(rat(2, 1)).unwrap().to_f64();
        assert!((shown - 3.0).abs() < 1e-6, "{shown}");
    }

    /// A duplicate is a clip of its own all the way down. Two clips sharing
    /// an effect id would answer for each other.
    #[test]
    fn a_duplicate_has_ids_of_its_own() {
        let mut c = clip(Uuid::now_v7(), 0, 4);
        c.effects = vec![instance()];
        let copy = c.duplicate();
        assert_ne!(copy.id, c.id);
        assert_ne!(copy.effects[0].id, c.effects[0].id);
        assert_eq!(copy.place_start, c.place_start);
    }

    /// A slip changes the frames and never the place, a ramp slips whole, and
    /// a clip cannot be slipped off the top of its source.
    #[test]
    fn slipping_changes_the_frames_and_not_the_place() {
        let c = Clip::new(
            ClipSource::Footage(Uuid::now_v7()),
            rat(2, 1),
            rat(6, 1),
            rat(10, 1),
            rat(4, 1),
        );
        let s = c.slip(rat(1, 1)).unwrap();
        assert_eq!((s.place_start, s.place_duration), (rat(10, 1), rat(4, 1)));
        assert_eq!((s.source_in, s.source_out), (rat(3, 1), rat(7, 1)));
        assert!((s.source_time(10.0) - 3.0).abs() < 1e-9);

        let ramp = c.with_ramp(rat(1, 1), rat(3, 1));
        let slipped = ramp.slip(rat(-1, 1)).unwrap();
        for lt in [10.0, 11.5, 13.9] {
            let moved = slipped.source_time(lt) - ramp.source_time(lt);
            assert!((moved + 1.0).abs() < 1e-9, "the whole map moved at {lt}");
        }
        assert_eq!(slipped.ramp_view(), ramp.ramp_view(), "at the same speeds");

        assert!(c.slip(rat(-3, 1)).is_none());
        let mut typed = c.clone();
        typed.retime = Some(Property {
            animation: Animation::Expression("time".into()),
            extra: serde_json::Map::new(),
        });
        assert!(
            typed.slip(rat(1, 1)).is_none(),
            "an expression is not rewritten"
        );
    }

    /// The editor's slide: the clip moves with its frames, and its neighbours
    /// give and take the difference so the three stay abutting.
    #[test]
    fn sliding_between_neighbours_trims_one_and_extends_the_other() {
        let src = Uuid::now_v7();
        let clips = vec![clip(src, 0, 4), clip(src, 4, 4), clip(src, 8, 4)];
        let spans = |c: &[Clip]| {
            c.iter()
                .map(|c| (c.place_start, c.place_end()))
                .collect::<Vec<_>>()
        };

        let out = slide_between(&clips, clips[1].id, rat(1, 1)).unwrap();
        assert_eq!(
            spans(&out),
            vec![
                (rat(0, 1), rat(5, 1)),
                (rat(5, 1), rat(9, 1)),
                (rat(9, 1), rat(12, 1))
            ]
        );
        assert_eq!(out[1].source_in, clips[1].source_in, "the same frames");
        assert!((out[2].source_time(10.0) - clips[2].source_time(10.0)).abs() < 1e-9);

        // A neighbour has only its own length to give.
        assert!(slide_between(&clips, clips[1].id, rat(4, 1)).is_none());
        // The last clip has open row after it, and may move into it.
        let out = slide_between(&clips, clips[2].id, rat(2, 1)).unwrap();
        assert_eq!(
            spans(&out),
            vec![
                (rat(0, 1), rat(4, 1)),
                (rat(4, 1), rat(10, 1)),
                (rat(10, 1), rat(14, 1))
            ]
        );
    }

    #[test]
    fn cutting_partitions_a_clip_without_moving_it() {
        let src = Uuid::now_v7();
        // A clip at layer [2,6), source 0→4 at natural rate. Cut at layer 4.
        let c = clip(src, 2, 4);
        let (l, r) = c.cut(rat(4, 1)).unwrap();
        // Places abut exactly and don't move (beat-sync).
        assert_eq!(l.place_start, rat(2, 1));
        assert_eq!(l.place_duration, rat(2, 1));
        assert_eq!(r.place_start, rat(4, 1));
        assert_eq!(r.place_duration, rat(2, 1));
        // Source trims partition at the cut (source time 2 at layer time 4).
        assert_eq!(l.source_out, rat(2, 1));
        assert_eq!(r.source_in, rat(2, 1));
        // Each half plays the same source moment as the original did.
        assert!((l.source_time(3.0) - c.source_time(3.0)).abs() < 1e-9);
        assert!((r.source_time(5.0) - c.source_time(5.0)).abs() < 1e-9);
        // A cut outside the clip refuses.
        assert!(c.cut(rat(2, 1)).is_none());
        assert!(c.cut(rat(6, 1)).is_none());
    }

    /// The same, for a map whose middle key **aims itself**. This is
    /// the case that was silently wrong: an automatic tangent is a function of
    /// its neighbours, and a cut changes the neighbours, so both halves drifted
    /// — by a sixth of a second of source on the map below, which is four
    /// frames of the wrong picture either side of the edit point.
    #[test]
    fn cutting_a_self_aiming_ramp_keeps_the_speed_curve_too() {
        let auto = || SideInterp::Auto {
            clamped: false,
            speed: 0.0,
            influence: 1.0 / 3.0,
        };
        let mut clip = Clip::new(
            ClipSource::Footage(Uuid::now_v7()),
            rat(0, 1),
            rat(4, 1),
            rat(2, 1),
            rat(4, 1),
        );
        clip.retime = Some(Property {
            animation: Animation::Keyframed(vec![
                Keyframe {
                    time: Rational::ZERO,
                    value: 0.0,
                    interp_in: SideInterp::Linear,
                    interp_out: SideInterp::Bezier {
                        speed: 0.0,
                        influence: 0.8,
                    },
                },
                Keyframe {
                    time: rat(2, 1),
                    value: 1.0,
                    interp_in: auto(),
                    interp_out: auto(),
                },
                Keyframe {
                    time: rat(4, 1),
                    value: 4.0,
                    interp_in: SideInterp::Bezier {
                        speed: 0.0,
                        influence: 0.8,
                    },
                    interp_out: SideInterp::Linear,
                },
            ]),
            extra: serde_json::Map::new(),
        });

        // Either side of the automatic key, so both spans get a turn.
        for cut in [rat(3, 1), rat(5, 1)] {
            let (left, right) = clip.cut(cut).expect("cuts");
            let mut worst: f64 = 0.0;
            for step in 0..=4000 {
                let lt = 2.0 + 4.0 * f64::from(step) / 4000.0;
                let half = if lt < cut.to_f64() { &left } else { &right };
                worst = worst.max((clip.source_time(lt) - half.source_time(lt)).abs());
            }
            assert!(worst < 1e-9, "the halves are the original curve: {worst:e}");
        }
    }

    /// The frame-pinning invariant (Mack's note): a clip's first frame is its
    /// `source_in`, whatever its speed. So splitting a clip and re-speeding the
    /// second half (e.g. 200% → 100%) leaves the second clip's *starting*
    /// frame exactly where it was — the speed change ripples forward only.
    #[test]
    fn re_speeding_a_cut_clip_keeps_its_start_frame() {
        let src = Uuid::now_v7();
        // Clip [0,4), source 0→4 natural. Cut at layer 2 → right clip [2,4).
        let (_left, right) = clip(src, 0, 4).cut(rat(2, 1)).unwrap();
        let start_frame = right.source_in; // the source moment at the cut
        assert!((right.source_time(2.0) - start_frame.to_f64()).abs() < 1e-9);

        // Re-speed the right clip: 200% ramping to 100% over its 2 s, pinned at
        // its own source_in (this is exactly what per-clip speed editing must
        // build). Its first frame must NOT move.
        let respeed = right.with_ramp(rat(2, 1), rat(1, 1));
        // First frame unchanged; only later frames advance faster.
        assert!((respeed.source_time(2.0) - start_frame.to_f64()).abs() < 1e-9);
        assert!(respeed.source_time(3.0) > right.source_time(3.0));
        // And it holds after moving the whole clip later on the layer (the
        // place shifts, the retime domain is unchanged, so the start frame is
        // still source_in).
        let mut moved = respeed.clone();
        moved.place_start = rat(5, 1);
        assert!((moved.source_time(5.0) - start_frame.to_f64()).abs() < 1e-9);
    }

    // ------------------------------------------------- fades and shapes --

    /// One effect instance for a clip's own rack. Any effect will do: nothing
    /// here opens it, and the rules under test are about identity.
    fn instance() -> EffectInstance {
        crate::fx::instantiate("blur").expect("a blur exists")
    }

    /// The five preset shapes are all fades: silent at the start, full at the
    /// end, and never dipping on the way (plan 1).
    #[test]
    fn every_preset_shape_runs_from_silence_to_full_without_dipping() {
        for shape in [
            FadeShape::Linear,
            FadeShape::Fast,
            FadeShape::Slow,
            FadeShape::Smooth,
            FadeShape::Sharp,
        ] {
            assert!(shape.gain(0.0).abs() < 1e-9, "{shape:?} starts at silence");
            assert!(
                (shape.gain(1.0) - 1.0).abs() < 1e-9,
                "{shape:?} reaches full"
            );
            let mut last = -1.0;
            for n in 0..=100 {
                let g = shape.gain(f64::from(n) / 100.0);
                assert!(g >= last - 1e-9, "{shape:?} dips at {n}: {g} after {last}");
                last = g;
            }
            // Off either end the answer is the end it is nearest, never a
            // number from off the curve.
            assert!(shape.gain(-5.0).abs() < 1e-9);
            assert!((shape.gain(5.0) - 1.0).abs() < 1e-9);
        }
        // Sharp is Smooth read the other way round, exactly.
        for n in 0..=20 {
            let u = f64::from(n) / 20.0;
            let there_and_back = FadeShape::Smooth.gain(FadeShape::Sharp.gain(u));
            assert!(
                (there_and_back - u).abs() < 1e-9,
                "Sharp is not the inverse of Smooth at {u}"
            );
        }
    }

    /// A Custom shape is its bezier read at x = u, and its handles are held
    /// inside the box whatever the file says (plan 1).
    #[test]
    fn a_custom_shape_is_read_at_x_and_keeps_its_handles_in_the_box() {
        // The handles of a straight line give back the straight line.
        let straight = FadeShape::custom(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0);
        for n in 0..=10 {
            let u = f64::from(n) / 10.0;
            assert!(
                (straight.gain(u) - FadeShape::Linear.gain(u)).abs() < 1e-6,
                "a straight bezier is Linear at {u}"
            );
        }
        // A handle dragged far outside is pulled back rather than making a
        // curve that runs backwards in time.
        assert_eq!(
            FadeShape::custom(-9.0, -9.0, 9.0, 9.0),
            FadeShape::Custom {
                x1: MIN_REACH,
                y1: -HANDLE_REACH,
                x2: 1.0 - MIN_REACH,
                y2: 1.0 + HANDLE_REACH,
            }
        );
        // A shape stored unclamped is still read clamped: the file is not a
        // way in past the bound.
        let wild = FadeShape::Custom {
            x1: -9.0,
            y1: -9.0,
            x2: 9.0,
            y2: 9.0,
        };
        let tamed = FadeShape::custom(-9.0, -9.0, 9.0, 9.0);
        for n in 0..=10 {
            let u = f64::from(n) / 10.0;
            assert!((wild.gain(u) - tamed.gain(u)).abs() < 1e-12);
        }
        assert!(wild.gain(0.0).abs() < 1e-6 && (wild.gain(1.0) - 1.0).abs() < 1e-6);
    }

    /// What each default pair holds across a join: Fast against Fast keeps the
    /// power, Linear against Linear keeps the amplitude (plan 1).
    #[test]
    fn fast_against_fast_holds_the_power_and_linear_the_amplitude() {
        for n in 0..=20 {
            let u = f64::from(n) / 20.0;
            // The incoming clip reads g(u), the outgoing one g(1 − u).
            let (a, b) = (FadeShape::Fast.gain(u), FadeShape::Fast.gain(1.0 - u));
            assert!((a * a + b * b - 1.0).abs() < 1e-9, "u={u}: not equal power");
            let (a, b) = (FadeShape::Linear.gain(u), FadeShape::Linear.gain(1.0 - u));
            assert!((a + b - 1.0).abs() < 1e-9, "u={u}: not equal amplitude");
        }
    }

    /// The razor divides the new fields the way docs/impl/audio-timeline.md §2
    /// says: the outside ends keep their fades, both halves keep the bypass,
    /// and neither half's effects answer for the other's (plan 6).
    #[test]
    fn a_cut_divides_the_fades_and_freshens_the_effects() {
        let mut c = clip(Uuid::now_v7(), 0, 4);
        c.fade_in = Fade {
            seconds: rat(1, 2),
            shape: FadeShape::Slow,
        };
        c.fade_out = Fade {
            seconds: rat(1, 1),
            shape: FadeShape::Sharp,
        };
        c.fx = false;
        c.gain_db = -6.0;
        c.effects = vec![instance()];
        let (left, right) = c.cut(rat(2, 1)).expect("a cut inside the clip");

        assert_eq!(left.fade_in, c.fade_in, "the left keeps the fade in");
        assert_eq!(left.fade_out, Fade::default(), "and no fade at the cut");
        assert_eq!(right.fade_in, Fade::default());
        assert_eq!(right.fade_out, c.fade_out, "the right keeps the fade out");
        assert!(!left.fx && !right.fx, "both halves keep the bypass");
        assert_eq!(
            (left.gain_db, right.gain_db),
            (-6.0, -6.0),
            "and the gain, which is one level for the whole sound"
        );
        assert_eq!(left.effects.len(), 1);
        assert_eq!(right.effects.len(), 1);
        assert_ne!(
            left.effects[0].id, right.effects[0].id,
            "two halves sharing an instance id would answer for each other"
        );
        assert_ne!(left.effects[0].id, c.effects[0].id);
        assert_eq!(
            left.effects[0].effect, c.effects[0].effect,
            "the effect itself is the same effect"
        );

        // A trim keeps every field: only a cut divides them.
        let trimmed = c.trim_end(rat(3, 1)).expect("a trim inside the clip");
        assert_eq!(trimmed.fade_in, c.fade_in);
        assert_eq!(trimmed.fade_out, c.fade_out);
        assert_eq!(trimmed.effects[0].id, c.effects[0].id);
    }

    /// **A project written before the fades writes again byte for byte**
    /// (plan 6): nothing is added to the file until something is set, and a
    /// clip that does carry the fields comes back as it went in.
    #[test]
    fn the_new_fields_are_absent_until_set_and_round_trip_when_they_are() {
        let bare = clip(Uuid::now_v7(), 1, 4);
        let before = serde_json::to_string(&bare).unwrap();
        for key in ["fade_in", "fade_out", "effects", "fx", "gain_db", "link"] {
            assert!(!before.contains(key), "an untouched clip writes no {key}");
        }
        let reopened: Clip = serde_json::from_str(&before).unwrap();
        assert_eq!(reopened, bare, "and reads back as the clip it was");
        assert_eq!(
            serde_json::to_string(&reopened).unwrap(),
            before,
            "so it writes again unchanged"
        );

        let mut set = bare.clone();
        set.fade_in = Fade {
            seconds: rat(3, 4),
            shape: FadeShape::custom(0.2, 0.8, 0.6, 0.4),
        };
        set.fade_out = Fade {
            seconds: rat(1, 2),
            shape: FadeShape::Slow,
        };
        set.effects = vec![instance()];
        set.fx = false;
        set.gain_db = -6.0;
        set.link = Some(Uuid::now_v7());
        let text = serde_json::to_string(&set).unwrap();
        assert_eq!(serde_json::from_str::<Clip>(&text).unwrap(), set);
    }
}
