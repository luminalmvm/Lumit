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
use crate::model::{default_true, is_true, is_zero, Composition, EffectInstance, Layer, LayerKind};
use crate::retime::Interpolation;
use crate::time::Rational;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod clip_opacity;
mod freeze_auto;
pub mod spoken;
pub mod takes;
pub mod transition;

/// What a clip plays: one footage item, one nested composition or one solid
/// (docs/03-DATA-MODEL.md §5.3 ClipSource).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipSource {
    Footage(Uuid),
    Comp(Uuid),
    /// A flat colour at the solid's own size, the same at every time.
    Solid(Uuid),
}

impl ClipSource {
    /// The project item it plays.
    #[must_use]
    pub fn id(self) -> Uuid {
        match self {
            Self::Footage(id) | Self::Comp(id) | Self::Solid(id) => id,
        }
    }
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
    /// The angle a multicam clip plays: one Sequence layer of the comp it
    /// plays, by id ([`crate::multicam`]). Unset on every other clip, and left
    /// out of the file then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<Uuid>,
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
    /// The values this clip hands the graphics template it places: what
    /// [`crate::model::Layer::graph_inputs`] is to a Precomp layer. Keyed
    /// from the clip's own start, as its effects are. `None` is a clip that
    /// sets nothing, which is every clip of anything but a template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<EffectInstance>,
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
    /// Which of its file's audio streams this clip sounds, for a footage clip
    /// of a file with more than one. The first unless it has been told
    /// otherwise, and left out of the file until then.
    #[serde(default, skip_serializing_if = "crate::model::AudioStreams::is_first")]
    pub audio_streams: crate::model::AudioStreams,
    /// Whether the clip is heard. Off on a picture clip whose sound has been
    /// detached onto a clip of its own, which is then the one heard. Left out
    /// of the file while on.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub audible: bool,
    /// The clip's own blend mode, used in place of the layer's while this
    /// clip is the one showing. Unset is "the layer's", and is left out of
    /// the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<crate::model::BlendMode>,
    /// The clip's own label colour, worn in place of its layer's. Unset is
    /// "the layer's", and is left out of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<u8>,
    /// The group the clip is in, as Vegas groups events: clips sharing one
    /// are picked, moved and trimmed together. Apart from `link`, which pairs
    /// a picture clip with its sound one to one. Left out of the file while
    /// unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<Uuid>,
    /// How opaque the clip's picture is, nought to one, taken with its fades.
    #[serde(
        default = "clip_opacity::one",
        skip_serializing_if = "clip_opacity::is_one"
    )]
    pub opacity: f64,
    /// The other takes the clip holds ([`takes`]). What it shows is the one
    /// in `source` and `source_in`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub takes: Vec<takes::Take>,
    /// What the overlap at this clip's start draws as, when another clip runs
    /// into it on a picture row ([`transition`]). Unset is the dissolve, and is
    /// left out of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<EffectInstance>,
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
            angle: None,
            source_in,
            source_out,
            place_start,
            place_duration,
            retime: None,
            interpolation: Interpolation::default(),
            fade_in: Fade::default(),
            fade_out: Fade::default(),
            effects: Vec::new(),
            inputs: None,
            fx: true,
            gain_db: 0.0,
            link: None,
            audio_streams: crate::model::AudioStreams::default(),
            audible: true,
            blend: None,
            label: None,
            group: None,
            opacity: 1.0,
            takes: Vec::new(),
            transition: None,
            extra: serde_json::Map::new(),
        }
    }

    /// The effect the overlap at this clip's start draws as, or `None` for the
    /// dissolve, which is also what a bypassed one draws as.
    pub fn drawn_transition(&self) -> Option<&EffectInstance> {
        self.transition.as_ref().filter(|t| t.enabled)
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

    /// This clip placed over `start..end` of its row instead, at whatever
    /// speed that takes: the same stretch of source, with its map scaled to
    /// the new length. `None` for an empty span or on overflow.
    pub fn stretched(&self, start: Rational, end: Rational) -> Option<Clip> {
        let length = end.checked_sub(start).ok()?;
        if length <= Rational::ZERO || self.place_duration <= Rational::ZERO {
            return None;
        }
        let k = length.checked_div(self.place_duration).ok()?;
        let map = self.effective_retime();
        let animation = match &map.animation {
            Animation::Keyframed(keys) => {
                Animation::Keyframed(crate::retime::stretch_keys(keys, Rational::ZERO, k)?)
            }
            // A held frame holds however long it lasts.
            held => held.clone(),
        };
        Some(Clip {
            place_start: start,
            place_duration: length,
            retime: Some(Property {
                animation,
                extra: map.extra,
            }),
            ..self.clone()
        })
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
    /// A clip played backwards ends before it starts, so the window is its
    /// two ends whichever way round, as the sound holds it. `.max().min()`
    /// avoids `f64::clamp`'s panic on a NaN map (engine crates never panic).
    pub fn held_source_time(&self, lt: f64) -> f64 {
        let (a, b) = (self.source_in.to_f64(), self.source_out.to_f64());
        self.source_time(lt).max(a.min(b)).min(a.max(b))
    }

    /// Which moment of `nested` a clip that plays it shows at layer time
    /// `lt`. Composition time is source time, so this is
    /// [`Self::held_source_time`], and a Retime that runs past the comp holds
    /// its last frame, as [`crate::model::nested_source_time`] holds a
    /// Precomp layer's. The one place a clip's composition is given a time,
    /// so the frame key, the decode plan and the draw agree on it.
    pub fn nested_time(&self, nested: &Composition, lt: f64) -> f64 {
        let last = nested.duration.0.to_f64() - 1.0 / nested.frame_rate.fps().max(1.0);
        if self.retime.is_none() {
            // A template with an intro or an outro fits the clip's length:
            // its ends keep their speed and the stretch between them takes
            // what is left. Retime on the clip is the more exact
            // instruction, and turns this off.
            if let Some(timing) = nested.template_timing() {
                let ends = self.inputs.as_ref().map_or(
                    crate::template::time::Ends::BOTH,
                    crate::fx::effects::template::ends,
                );
                let along = lt - self.place_start.to_f64();
                let length = self.place_duration.to_f64();
                return crate::template::time::copy_time(timing, ends, length, along)
                    .min(last)
                    .max(0.0);
            }
            return self.held_source_time(lt);
        }
        self.held_source_time(lt).min(last).max(0.0)
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
        let (a, b) = (self.source_in, self.source_out);
        Some(shown.max(a.min(b)).min(a.max(b)))
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
            inputs: self
                .inputs
                .as_ref()
                .and_then(|i| respawn(std::slice::from_ref(i)).pop()),
            transition: self
                .transition
                .as_ref()
                .and_then(|t| respawn(std::slice::from_ref(t)).pop()),
            // A copy is out of the original's group.
            group: None,
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
        freeze_auto::freeze_auto_around(&mut whole, tau);
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
            angle: self.angle,
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
            audio_streams: self.audio_streams.clone(),
            audible: self.audible,
            blend: self.blend,
            label: self.label,
            group: self.group,
            opacity: self.opacity,
            takes: self.takes.clone(),
            inputs: self.inputs.clone(),
            // What the clip's start draws as, which is the left piece's start.
            transition: self.transition.clone(),
            extra: self.extra.clone(),
        };
        let right = Clip {
            id: Uuid::now_v7(),
            source: self.source,
            angle: self.angle,
            source_in: s_cut,
            source_out: self.source_out,
            place_start: at,
            place_duration: right_duration,
            retime: right_retime,
            interpolation: self.interpolation.clone(),
            fade_in: Fade::default(),
            fade_out: self.fade_out,
            // A clip's effects are keyed from the clip's own start, and the
            // later piece starts at the cut.
            effects: {
                let mut effects = respawn(&self.effects);
                if let Ok(back) = Rational::ZERO.checked_sub(tau_clip) {
                    crate::preset::shift_keys(&mut effects, back);
                }
                effects
            },
            // And so are the values it hands a template.
            inputs: self.inputs.as_ref().and_then(|inputs| {
                let mut inputs = respawn(std::slice::from_ref(inputs));
                if let Ok(back) = Rational::ZERO.checked_sub(tau_clip) {
                    crate::preset::shift_keys(&mut inputs, back);
                }
                inputs.pop()
            }),
            fx: self.fx,
            gain_db: self.gain_db,
            link: self.link,
            audio_streams: self.audio_streams.clone(),
            audible: self.audible,
            blend: self.blend,
            label: self.label,
            group: self.group,
            opacity: self.opacity,
            takes: self.takes.clone(),
            transition: None,
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
pub(crate) fn respawn(effects: &[EffectInstance]) -> Vec<EffectInstance> {
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
    /// and `incoming` at `gain`, added together, and the mix drawn at
    /// `opacity`, which is 1 unless a clip's own opacity is less.
    Two {
        outgoing: &'a Clip,
        incoming: &'a Clip,
        gain: f64,
        opacity: f64,
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
            gain: lone_gain(clips, first, lt) * clip_opacity::opaque(first),
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
    let (gain, opacity) =
        clip_opacity::weighed(outgoing, incoming, opacity(incoming.fade_in.shape, u));
    Shown::Two {
        outgoing,
        incoming,
        gain,
        opacity,
    }
}

/// A Sequence layer as it draws at `lt`: wearing the effects and the blend mode
/// of the clip it is showing. `None` where the layer draws as it is stored,
/// which is every layer that is not a Sequence layer and every moment whose
/// clip sets neither.
///
/// The clip's stack runs ahead of the layer's own, as it does in the sound
/// mix. Its keys are stored from the clip's start and are moved onto the
/// layer's clock here, so everything downstream reads one stack at one time.
///
/// Inside a dissolve the layer wears both clips' stacks, the outgoing clip's
/// first, each with its effects' Mix scaled by that clip's share of the
/// picture. So an effect arrives and leaves with its clip and nothing changes
/// at once. A blend mode cannot be part way between two, so the layer takes
/// the mode of whichever clip is more than half of the picture.
///
/// ponytail: the two stacks run on the picture already mixed, not each on its
/// own clip's, and an effect whose Mix is an expression or which has none is
/// worn whole from the first frame of the dissolve. The upgrade is to run
/// each clip's stack on its own picture ahead of the mix. And the copy takes
/// the row's other clips with it, which a row of thousands would feel. The
/// upgrade there is a copy that carries only the clips near this moment.
#[must_use]
pub fn worn_at(layer: &Layer, lt: f64) -> Option<Layer> {
    let LayerKind::Sequence { clips } = &layer.kind else {
        return None;
    };
    if layer.audio_only {
        return None;
    }
    // Each clip showing with its share of the picture, the earlier first, and
    // the one that is most of it.
    let (shares, lead): (Vec<(&Clip, f64)>, &Clip) = match shown_at(clips, lt) {
        Shown::Nothing => return None,
        Shown::One { clip, .. } => (vec![(clip, 1.0)], clip),
        Shown::Two {
            outgoing,
            incoming,
            gain,
            ..
        } => (
            // A transition draws each clip on its own, each with its own stack.
            if incoming.drawn_transition().is_some() {
                Vec::new()
            } else {
                vec![(outgoing, 1.0 - gain), (incoming, gain)]
            },
            if gain >= 0.5 { incoming } else { outgoing },
        ),
    };
    let stacked = |clip: &Clip| clip.fx && !clip.effects.is_empty();
    if !shares.iter().any(|(clip, _)| stacked(clip)) && lead.blend.is_none() {
        return None;
    }
    let mut worn = layer.clone();
    let mut effects = Vec::new();
    for (clip, share) in shares {
        if !stacked(clip) {
            continue;
        }
        let mut stack = clip.effects.clone();
        crate::preset::shift_keys(&mut stack, clip.place_start);
        if share < 1.0 {
            scale_mix(&mut stack, share);
        }
        effects.extend(stack);
    }
    effects.extend(worn.effects);
    worn.effects = effects;
    if let Some(blend) = lead.blend {
        worn.blend = blend;
    }
    Some(worn)
}

/// Every effect's Mix made `share` of what it was, keys and all.
fn scale_mix(effects: &mut [EffectInstance], share: f64) {
    for effect in effects {
        for param in &mut effect.params {
            if param.id != crate::fx::schema::MIX_PARAM {
                continue;
            }
            if let crate::model::EffectValue::Float(mix) = &mut param.value {
                if let Some(scaled) = crate::ops::remapped(&mix.animation, share, 0.0) {
                    mix.animation = scaled;
                }
            }
        }
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
mod tests;
