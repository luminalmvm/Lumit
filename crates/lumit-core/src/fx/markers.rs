use crate::anim::{Animation, Property};
use crate::model::{Composition, EffectInstance, EffectNamespace, EffectValue, Layer};

/// The Flash trigger envelope (docs/08 §3.7, manual form). A static Trigger
/// is a constant flash. A keyframed Trigger reads each keyframe as a hit:
/// the key's value (0..1) is the hit strength, decaying exponentially to 1/e
/// over `decay_s`; overlapping hits take the loudest. The curve between keys
/// is deliberately not interpolated — one keyframe per beat is the authoring
/// unit, exactly what the §1.4 marker binding will automate. Pure function
/// of the property and time, so determinism (§2.4) holds.
pub fn flash_envelope(trigger: &Property, t: f64, decay_s: f64) -> f64 {
    match &trigger.animation {
        Animation::Static(v) => v.clamp(0.0, 1.0),
        Animation::Keyframed(keys) => {
            let mut env: f64 = 0.0;
            for k in keys {
                let kt = k.time.to_f64();
                if kt > t {
                    break; // keys are sorted; later hits cannot contribute
                }
                let fall = if decay_s > 0.0 {
                    (-(t - kt) / decay_s).exp()
                } else if t == kt {
                    1.0
                } else {
                    0.0
                };
                env = env.max(k.value.clamp(0.0, 1.0) * fall);
            }
            env
        }
        Animation::Expression(_) => 0.0,
    }
}

/// The trigger times a marker-driven Flash reads (docs/08 §3.7): every
/// `nth`-th beat of the ordered §1.4 context — indices 0, n, 2n, … of the
/// beat list, the comp's first beat being index 0 — shifted by
/// `phase_frames` comp frames. Yields layer-local seconds, ascending. One
/// iterator shared by the envelope and the frame-key window
/// ([`marker_window`]) so cache invalidation can never drift from what
/// resolution computes.
fn flash_trigger_times<'a>(
    markers: &'a MarkerContext,
    nth: u32,
    phase_frames: f64,
) -> impl Iterator<Item = f64> + 'a {
    let dt = if markers.fps > 0.0 {
        phase_frames / markers.fps
    } else {
        0.0
    };
    markers
        .beats
        .iter()
        .step_by(nth.max(1) as usize)
        .map(move |b| b + dt)
}

/// The Flash beat envelope (docs/08 §3.7 Trigger and Strobe modes), pinned
/// once for resolution, its unit tests and the frame key alike. From the
/// nearest trigger at/before the frame ([`flash_trigger_times`]), with
/// `elapsed = (lt − trigger) · fps` in comp frames: Hard holds 1 while
/// `0 ≤ elapsed < duration_frames`, Fade ramps `1 − elapsed/duration_frames`
/// over the same span; past it — and before the first trigger — the
/// envelope is 0. No markers, a non-positive frame rate (the [`MarkerContext::NONE`]
/// caller) or a non-positive duration all yield 0: the §1.4 graceful
/// fallback. Pure function of its inputs, so determinism (§2.4) holds.
pub fn flash_beat_envelope(
    markers: &MarkerContext,
    lt: f64,
    duration_frames: f64,
    fade: bool,
    nth: u32,
    phase_frames: f64,
) -> f64 {
    if markers.fps <= 0.0 || duration_frames <= 0.0 {
        return 0.0;
    }
    let mut env = 0.0;
    for tt in flash_trigger_times(markers, nth, phase_frames) {
        let elapsed = (lt - tt) * markers.fps;
        if elapsed < 0.0 {
            break; // ascending: every later trigger is in the future too
        }
        env = if elapsed < duration_frames {
            if fade {
                1.0 - elapsed / duration_frames
            } else {
                1.0
            }
        } else {
            0.0 // the nearest trigger at/before wins, even once spent
        };
    }
    env
}

/// Strobe's Every Nth beat parameter, read as the spec's integer ≥ 1
/// (docs/08 §3.7): rounded to the nearest whole beat count, clamped at 1,
/// non-finite values degrading to 1.
pub(super) fn flash_nth(e: &EffectInstance, lt: f64) -> u32 {
    let n = e.float_at("every_nth", lt).unwrap_or(1.0);
    if n.is_finite() && n >= 1.0 {
        n.round() as u32
    } else {
        1
    }
}

/// What one marker-driven effect instance sees of the §1.4 context at a
/// frame — the nearest trigger either side of it, exactly as its envelope
/// consumes them (Nth-filtered and phase-shifted for a Strobe flash), plus
/// the comp frame rate its frame-authored parameters convert through. Fed
/// into the frame key (lumit-eval) so a cached frame is retired exactly
/// when a marker edit can change what this instance computes, and left
/// alone otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MarkerWindow {
    /// Comp frames per second.
    pub fps: f64,
    /// The nearest trigger at/before the frame, layer-local seconds.
    pub before: Option<f64>,
    /// The nearest trigger strictly after, layer-local seconds.
    pub after: Option<f64>,
}

/// The §1.4 window `e` consumes at layer time `lt` — None when the
/// instance is not marker-driven right now (an effect without marker
/// input, or a Flash in Manual mode), which is what keeps such instances'
/// frame keys time-free. v1: Flash is the only marker consumer; a new
/// marker-driven effect adds its arm here, so the frame key learns it in
/// the same place resolution does.
pub fn marker_window(e: &EffectInstance, lt: f64, markers: &MarkerContext) -> Option<MarkerWindow> {
    if e.effect.namespace != EffectNamespace::Builtin || e.effect.match_name != "flash" {
        return None;
    }
    let mode = match e.param("mode") {
        Some(EffectValue::Choice(c)) => *c,
        _ => 0,
    };
    if mode != 1 && mode != 2 {
        return None; // Manual: no marker input, no time in the key
    }
    let nth = if mode == 2 { flash_nth(e, lt) } else { 1 };
    let phase = e.float_at("phase", lt).unwrap_or(0.0);
    let mut w = MarkerWindow {
        fps: markers.fps,
        before: None,
        after: None,
    };
    for tt in flash_trigger_times(markers, nth, phase) {
        if tt <= lt {
            w.before = Some(tt);
        } else {
            w.after = Some(tt);
            break;
        }
    }
    Some(w)
}

/// The §1.4 marker resolve context: what marker-driven effects see at
/// resolution time. It carries the comp's beat-marker times **translated
/// into the layer's local time** — comp marker time minus the layer's start
/// offset, the same one f64 subtraction that produces the `lt` handed to
/// [`resolve_stack`], so a beat and a frame at the same comp moment compare
/// exactly equal and the envelope maths lives in a single time base — plus
/// the comp frame rate, because duration-class parameters are authored in
/// comp frames (§2.3). Built by [`MarkerContext::for_layer`], the one
/// constructor preview and export both call (K-031), so the two can never
/// drift. A caller with no comp to hand passes [`MarkerContext::NONE`];
/// marker-driven effects MUST fall back gracefully on it (§1.4).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkerContext {
    /// Beat-marker times in the layer's local time base, seconds, sorted
    /// ascending.
    pub beats: Vec<f64>,
    /// Comp frames per second; 0 in the no-comp default (guarded wherever
    /// frames convert to seconds).
    pub fps: f64,
}

impl MarkerContext {
    /// The obvious empty context — no beats, no frame rate — for callers
    /// without markers. Every marker-driven effect resolves to its
    /// graceful fallback on it (§1.4).
    pub const NONE: MarkerContext = MarkerContext {
        beats: Vec::new(),
        fps: 0.0,
    };

    /// The context for one layer of `comp`: the comp's beat markers only
    /// (the v1 §1.4 scope — named-layer binding and label filters follow),
    /// each translated into the layer's local time, sorted ascending.
    pub fn for_layer(comp: &Composition, layer: &Layer) -> Self {
        Self::at_offset(comp, layer.start_offset.0)
    }

    /// The context at comp time itself — for an effect stack with no layer
    /// clock to convert through: a group header's (docs/impl/group-effects.md
    /// §1, K-731). The same beats `for_layer` reads, unshifted.
    pub fn for_comp(comp: &Composition) -> Self {
        Self::at_offset(comp, crate::time::Rational::ZERO)
    }

    fn at_offset(comp: &Composition, off: crate::time::Rational) -> Self {
        let mut beats: Vec<f64> = comp
            .markers
            .iter()
            .filter(|m| m.is_beat())
            .map(|m| crate::time::layer_time(m.time.0.to_f64(), off))
            .collect();
        beats.sort_by(f64::total_cmp);
        Self {
            beats,
            fps: comp.frame_rate.fps(),
        }
    }

    /// The ordered beat times within `[from_s, to_s]` local seconds — the
    /// §1.4 "inside the effect's temporal window" view.
    pub fn window(&self, from_s: f64, to_s: f64) -> &[f64] {
        let a = self.beats.partition_point(|b| *b < from_s);
        let z = self.beats.partition_point(|b| *b <= to_s).max(a);
        &self.beats[a..z]
    }

    /// The nearest beat at/before `lt` and the nearest strictly after —
    /// the §1.4 "either side of the current frame" pair.
    pub fn nearest(&self, lt: f64) -> (Option<f64>, Option<f64>) {
        let i = self.beats.partition_point(|b| *b <= lt);
        (
            i.checked_sub(1).map(|j| self.beats[j]),
            self.beats.get(i).copied(),
        )
    }
}
