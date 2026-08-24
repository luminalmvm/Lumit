//! Sequence layers: clips cut back-to-back on one row (docs/03-DATA-MODEL.md
//! §5.3, docs/04-RETIMING.md §1.3). This is Lumit's Vegas-style editing
//! surface.
//!
//! In plain terms: a Sequence layer is one timeline row holding a run of
//! **clips** laid end to end. Each clip points at a source (a footage item or
//! a comp), carries its own trim and its own [`Retime`] ramp, and sits at an
//! exact place on the row. Clips never overlap; a gap between them shows
//! through as transparent. To draw the layer at a given moment you ask "which
//! clip is under the playhead, and which moment of its source does that map
//! to?" — that resolution is all this module does. Turning that source moment
//! into pixels, and the layer's own masks/effects/transform, happen above.
//!
//! Scope note: this is the resolution model and its invariants only. Wiring
//! it into `LayerKind` and the render paths is the next step and lives
//! elsewhere; cutting (§8) and the graph lenses (§9) build on top.

use crate::anim::{Animation, Keyframe, Property, SideInterp};
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
    /// has (K-197, K-249).
    ///
    /// `None` is "not retimed": the clip plays from [`Self::source_in`] at
    /// source rate. That is a different state from a map that happens to be
    /// 1×, exactly as it is on a layer, and only the first skips the map.
    ///
    /// It was a segment store until K-249. Two representations for one job is
    /// what that decision existed to end, and clips were the second half of
    /// it; a document written before then converts on open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retime: Option<Property>,
    /// How fractional source moments become pixels (render policy).
    #[serde(default)]
    pub interpolation: Interpolation,
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
    /// clip — the straight-line speed the Vegas envelope authors (K-247).
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
    /// K-249 moved.
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
        let side = |s: &SideInterp, chord: f64| match s {
            SideInterp::Bezier { speed, .. } => *speed,
            SideInterp::Hold => 0.0,
            SideInterp::Linear => chord,
        };
        let mut out = Vec::with_capacity((keys.len() - 1) * 2);
        for pair in keys.windows(2) {
            let dt = pair[1].time.checked_sub(pair[0].time).ok()?.to_f64();
            if dt <= 0.0 {
                return None;
            }
            let chord = (pair[1].value - pair[0].value) / dt;
            out.push(side(&pair[0].interp_out, chord));
            out.push(side(&pair[1].interp_in, chord));
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
    /// around a trimmed clip (K-441, docs/15-DESIGN.md §12A.1).
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
        let on_grid = |v: f64| Rational::from_f64_on_grid(v, Rational::FLICK_DEN).ok();

        let mut whole = map.clone();
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
    /// exactly partition the original (docs/03-DATA-MODEL.md §5.3, the
    /// beat-sync covenant: `place` never moves, source positions stay exact).
    /// None when `at` is not strictly inside the clip, or the retime can't be
    /// split exactly there ([`Retime::split_at`]).
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

    /// Slip the source under the fixed clip by `delta` (docs/04-RETIMING.md
    /// §8.2): the clip keeps its place and duration, but a different stretch of
    /// the source plays. The trim window and every retime source position shift
    /// by `delta` together, so the retime's shape is untouched; overrun is
    /// re-evaluated at render time. None if the slip would read before the
    /// source start, or on overflow.
    pub fn slip(&self, delta: Rational) -> Option<Clip> {
        let source_in = self.source_in.checked_add(delta).ok()?;
        if source_in.is_negative() {
            return None;
        }
        let source_out = self.source_out.checked_add(delta).ok()?;
        // Every source position moves by the same amount, so the curve's shape
        // — and every keyframe time — is untouched. Tangent speeds are
        // slopes and a constant offset does not change them.
        let shift = delta.to_f64();
        let retime = match &self.retime {
            Some(map) => match &map.animation {
                Animation::Keyframed(keys) => Some(Property {
                    animation: Animation::Keyframed(
                        keys.iter()
                            .map(|k| Keyframe {
                                value: k.value + shift,
                                ..*k
                            })
                            .collect(),
                    ),
                    extra: map.extra.clone(),
                }),
                Animation::Static(v) => Some(Property {
                    animation: Animation::Static(v + shift),
                    extra: map.extra.clone(),
                }),
                // An expression-driven Retime cannot be shifted the way a
                // number or a keyframe can: the source positions it produces
                // are computed, so moving them means rewriting what the user
                // typed — `(expr) + shift`, compounding on every slip. Refused
                // rather than silently rewritten. Unreachable today (only
                // transform and effect properties can be given expressions),
                // and wants deciding properly if Retime ever offers one.
                Animation::Expression(_) => return None,
            },
            None => None,
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

    /// Trim the out point to the last moment still inside the source extent
    /// (docs/04-RETIMING.md §7.4, non-ripple): when the retime runs the clip
    /// past its trimmed source end (tail overrun), crop the clip to the crossing
    /// point. The clip's start never moves and a gap is left after it (gaps are
    /// never auto-closed — the beat-sync covenant K-022). None when there is no
    /// tail overrun, so the command can report "nothing to trim".
    pub fn trim_to_source_end(&self) -> Option<Clip> {
        let crossing = self.overrun_local_time()?;
        let new_end = self.place_start.checked_add(crossing).ok()?;
        self.trim_end(new_end)
    }

    /// The clip-local time at which the map first reaches [`Self::source_out`],
    /// or None when it never does — which is the ordinary case, and what makes
    /// "nothing to trim" reportable rather than a silent no-op.
    ///
    /// Found by walking the clip's own frame-free domain in small steps and
    /// bisecting the step that crosses. The map is monotone in every shape the
    /// editor can author, and the answer only has to be good to a frame: it
    /// decides where a *trim* lands, and rendering clamps per sample either way
    /// (docs/04 §7.2 — "rendering correctness never depends on this solve").
    fn overrun_local_time(&self) -> Option<Rational> {
        let target = self.source_out.to_f64();
        let d = self.place_duration.to_f64();
        if d <= 0.0 || self.source_time(self.place_start.to_f64() + d) <= target {
            return None; // never runs past the source it has
        }
        let at = |t: f64| self.source_time(self.place_start.to_f64() + t);
        if at(0.0) > target {
            return None; // over from the first moment: nothing to keep
        }
        let (mut lo, mut hi) = (0.0, d);
        for _ in 0..60 {
            let mid = (lo + hi) / 2.0;
            if at(mid) <= target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Rational::from_f64_on_grid(lo, Rational::FLICK_DEN).ok()
    }
}

/// The *shape* of a Sequence layer, apart from what it plays: where its cuts
/// fall, where its gaps are, and how each piece is ramped (K-248).
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

/// The last source position a map reaches.
fn map_last_value(map: &Property) -> Option<Rational> {
    let Animation::Keyframed(keys) = &map.animation else {
        return None;
    };
    Rational::from_f64_on_grid(keys.last()?.value, Rational::FLICK_DEN).ok()
}

/// Resolve the overlaps a clip has just been dropped into — the **overwrite**
/// edit every NLE does when one clip lands on another (K-248).
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
/// edit point beyond it moves and nothing ripples (K-022).
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
            if let Some(left) = c.trim_end(start) {
                out.push(left);
            }
            if let Some(mut right) = c.trim_start(end) {
                right.id = Uuid::now_v7();
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

/// The layer-local span the clips occupy: the first clip's start to the last
/// clip's end (K-248). None for a Sequence layer with no clips at all, which
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
/// (transparent) or past the end. Clips must not overlap, so at most one
/// matches; the first match wins defensively.
pub fn active_clip(clips: &[Clip], lt: f64) -> Option<&Clip> {
    clips.iter().find(|c| c.contains(lt))
}

/// Resolve layer-local time `lt` to `(active clip id, source, source time)`,
/// or None in a gap. The one query the renderer needs.
pub fn resolve(clips: &[Clip], lt: f64) -> Option<(Uuid, ClipSource, f64)> {
    active_clip(clips, lt).map(|c| {
        // Render and cache key sample the TRIMMED extent: on overrun the mapped
        // source position holds at the clip's [source_in, source_out] boundary
        // rather than running on into media past the trim (docs/04 §7.2). The
        // raw (unclamped) map stays available via `Clip::source_time` for
        // overrun detection. `.max().min()` avoids `f64::clamp`'s panics on a
        // degenerate window or a NaN map (engine crates never panic).
        let s = c
            .source_time(lt)
            .max(c.source_in.to_f64())
            .min(c.source_out.to_f64());
        (c.id, c.source, s)
    })
}

/// The single source shared by all clips, if they share one — a sequenced
/// layer is single-source (K-071). None when empty or mixed.
pub fn single_source(clips: &[Clip]) -> Option<ClipSource> {
    let first = clips.first()?.source;
    clips.iter().all(|c| c.source == first).then_some(first)
}

/// True when clips never jump backwards in the source as you read the layer
/// left to right — "no mixing footage time" (K-071): `source_in` is
/// non-decreasing by timeline position. Gaps are allowed; reordering is not.
pub fn is_source_ordered(clips: &[Clip]) -> bool {
    let mut by_place: Vec<&Clip> = clips.iter().collect();
    by_place.sort_by_key(|c| c.place_start);
    by_place
        .windows(2)
        .all(|w| w[0].source_in <= w[1].source_in)
}

/// Do any two clips overlap on the layer timeline? (docs/03-DATA-MODEL.md
/// §5.3 invariant: clips MUST NOT overlap — this is the check editors run
/// after a move before committing.)
pub fn has_overlap(clips: &[Clip]) -> bool {
    let mut spans: Vec<(f64, f64)> = clips
        .iter()
        .map(|c| (c.place_start.to_f64(), c.place_end().to_f64()))
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    spans.windows(2).any(|w| w[1].0 < w[0].1)
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
    fn a_flat_ramp_reprices_the_clip_without_moving_it() {
        // A 4 s clip of source [0,4). Play it at 2× → it still occupies 4 s on
        // the layer (place unchanged) but consumes 8 s of source.
        let base = clip(Uuid::now_v7(), 3, 4);
        let fast = base.with_ramp(rat(2, 1), rat(2, 1));
        assert_eq!(fast.place_start, base.place_start); // edit point held
        assert_eq!(fast.place_duration, base.place_duration);
        assert_eq!(fast.source_out, rat(8, 1)); // 4 s × 2×
        assert_eq!(fast.id, base.id); // same clip
        assert_eq!(fast.constant_speed(), Some(2.0));
        // Half speed consumes half the source.
        let slow = base.with_ramp(rat(1, 2), rat(1, 2));
        assert_eq!(slow.source_out, rat(2, 1));
        assert_eq!(slow.constant_speed(), Some(0.5));
        // A plain clip reads as 1×.
        assert_eq!(base.constant_speed(), Some(1.0));
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
        // And its first frame is still its own trim-in (K-070's pinning).
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

    #[test]
    fn source_time_runs_through_the_clip_retime() {
        // A clip at layer [2,6) whose source starts at 10s, played at half
        // speed: at layer time 4 (clip-local 2) the source is 10 + 0.5·2 = 11.
        let src = Uuid::now_v7();
        let mut c = clip(src, 2, 4);
        c.source_in = rat(10, 1);
        c = c.with_ramp(rat(1, 2), rat(1, 2));
        assert!((c.source_time(2.0) - 10.0).abs() < 1e-9); // clip start
        assert!((c.source_time(4.0) - 11.0).abs() < 1e-9); // half speed
        assert!((c.source_time(6.0) - 12.0).abs() < 1e-9); // clip end
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
    fn slipping_changes_the_source_but_not_the_place() {
        // Clip at layer [2,6), source [0,4) at natural rate. Slip +1 shows
        // source [1,5); the place is unchanged and every moment shifts by +1.
        let src = Uuid::now_v7();
        let c = clip(src, 2, 4);
        let s = c.slip(rat(1, 1)).unwrap();
        assert_eq!(s.place_start, c.place_start); // place held
        assert_eq!(s.place_duration, c.place_duration);
        assert_eq!(s.source_in, rat(1, 1)); // window shifted
        assert_eq!(s.source_out, rat(5, 1));
        for &lt in &[2.0, 4.0, 5.9] {
            assert!(
                (s.source_time(lt) - (c.source_time(lt) + 1.0)).abs() < 1e-9,
                "@ {lt}"
            );
        }
        // Slipping before the source start is refused.
        assert!(c.slip(rat(-1, 1)).is_none());
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

    #[test]
    fn trim_to_source_end_crops_a_tail_overrun() {
        let src = Uuid::now_v7();
        // Clip at layer [0,4), source [0,4). Retime it to 2× so f(t) = 2t runs
        // out of the source (out = 4) at local time 2.
        let mut c = clip(src, 0, 4).with_ramp(rat(2, 1), rat(2, 1));
        // Re-speeding re-derives how much source the clip *asks* for (8 s);
        // the media it actually has is still the 4 s it was trimmed to, and
        // that mismatch is exactly what overrun is.
        c.source_out = rat(4, 1);
        let t = c.trim_to_source_end().expect("a tail overrun trims");
        assert_eq!(t.place_start, c.place_start); // non-ripple: start held
        assert!((t.place_duration.to_f64() - 2.0).abs() < 1e-6);
        assert!((t.source_out.to_f64() - 4.0).abs() < 1e-6); // ends at the source end
                                                             // A clip that fits inside its source has nothing to trim.
        assert!(clip(src, 0, 4).trim_to_source_end().is_none());
    }

    /// The overwrite edit (K-248): a clip dropped on others takes its whole
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

    #[test]
    fn overlap_detection() {
        let s = Uuid::now_v7();
        // Back-to-back is fine (end-exclusive touching).
        assert!(!has_overlap(&[clip(s, 0, 2), clip(s, 2, 2)]));
        // A gap is fine.
        assert!(!has_overlap(&[clip(s, 0, 2), clip(s, 5, 2)]));
        // Genuine overlap is caught.
        assert!(has_overlap(&[clip(s, 0, 3), clip(s, 2, 2)]));
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

    #[test]
    fn single_source_and_ordering_invariants() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        // Two clips of the same source in order.
        let c0 = Clip::new(
            ClipSource::Footage(a),
            rat(0, 1),
            rat(2, 1),
            rat(0, 1),
            rat(2, 1),
        );
        let c1 = Clip::new(
            ClipSource::Footage(a),
            rat(2, 1),
            rat(4, 1),
            rat(3, 1),
            rat(2, 1),
        );
        assert_eq!(
            single_source(&[c0.clone(), c1.clone()]),
            Some(ClipSource::Footage(a))
        );
        assert!(is_source_ordered(&[c0.clone(), c1.clone()]));
        // A gap between them is fine (still ordered).
        assert!(is_source_ordered(&[c0.clone(), c1.clone()]));
        // Mixed sources → not single-source.
        let other = Clip::new(
            ClipSource::Footage(b),
            rat(0, 1),
            rat(2, 1),
            rat(5, 1),
            rat(2, 1),
        );
        assert_eq!(single_source(&[c0.clone(), other]), None);
        assert_eq!(single_source(&[]), None);
        // Reordered so a later timeline slot holds an earlier source moment →
        // "mixing footage time", rejected.
        let early_source_late_place = Clip::new(
            ClipSource::Footage(a),
            rat(0, 1),
            rat(1, 1),
            rat(6, 1),
            rat(1, 1),
        );
        assert!(!is_source_ordered(&[c1, early_source_late_place]));
    }

    /// K-441, docs/15-DESIGN.md §12A.1: a trimmed clip draws the faint outline
    /// of the material trimmed away, exactly as a trimmed layer does. The
    /// reach is the whole source laid on the layer's clock — so a clip trimmed
    /// in by 2 s reaches 2 s to the left of where it starts, and on to the end
    /// of its source whatever the clip's own length.
    #[test]
    fn an_untrimmed_clips_reach_is_its_own_span() {
        // A 4 s clip of a 4 s source, trimmed at neither end: the outline sits
        // exactly under the bar.
        let c = clip(Uuid::now_v7(), 3, 4);
        assert_eq!(
            c.source_reach(Some(rat(4, 1))),
            Some((rat(3, 1), rat(7, 1)))
        );
    }

    #[test]
    fn a_trimmed_clips_reach_runs_out_both_sides() {
        // 10 s of source, of which the clip shows [2, 6) placed at 5 s.
        let mut c = clip(Uuid::now_v7(), 5, 4);
        c.source_in = rat(2, 1);
        c.source_out = rat(6, 1);
        // Source moment 0 would sit 2 s before the clip's start, and the
        // source runs 10 s from there.
        assert_eq!(
            c.source_reach(Some(rat(10, 1))),
            Some((rat(3, 1), rat(13, 1)))
        );
    }

    #[test]
    fn a_clip_dragged_past_the_row_origin_reports_a_negative_reach() {
        // Nothing is clamped: the layer-level bounds are not either, and a
        // clamped reach would draw an outline that lies about where the
        // material begins.
        let mut c = clip(Uuid::now_v7(), 1, 4);
        c.source_in = rat(3, 1);
        c.source_out = rat(7, 1);
        assert_eq!(
            c.source_reach(Some(rat(9, 1))),
            Some((rat(-2, 1), rat(7, 1)))
        );
    }

    #[test]
    fn a_retimed_clip_has_no_reach() {
        // Retime frees the ends, exactly as it does on a layer bar
        // (docs/04-RETIMING.md): the map decides which source moment each
        // frame shows, so the source's length stops bounding the clip.
        let c = clip(Uuid::now_v7(), 3, 4).with_ramp(rat(2, 1), rat(2, 1));
        assert!(c.retime.is_some());
        assert_eq!(c.source_reach(Some(rat(4, 1))), None);
    }

    #[test]
    fn a_source_of_unknown_length_has_no_reach() {
        // Missing or unprobed media leaves the outline off rather than drawing
        // one pinned to a guess.
        assert_eq!(clip(Uuid::now_v7(), 3, 4).source_reach(None), None);
    }

    #[test]
    fn clip_round_trips_through_serde() {
        let c = clip(Uuid::now_v7(), 1, 4);
        let json = serde_json::to_string(&c).unwrap();
        let back: Clip = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
