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

/// The resolution flow is *measured* at (docs/08 §3.1).
///
/// Deliberately independent of the preview quality tier. Flow used to run on
/// whatever the preview scale had shrunk the decode to, which made a draft
/// scrub and an export two different measurements rather than one measurement
/// at two sizes — the field would change shape as you raised the quality. The
/// cost of fixing that is real and accepted: a layer with Flow live decodes at
/// native width even in draft preview, because full-resolution flow cannot be
/// measured on a shrunk decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FlowResolution {
    /// The source's own pixels — the default, and the only setting where
    /// preview and export are the same measurement at every quality tier.
    #[default]
    Native,
    /// Half the source's dimensions: ~4× cheaper, vectors scaled back up.
    Half,
    /// Quarter dimensions — the escape hatch for slow machines on 4K sources.
    Quarter,
}

impl FlowResolution {
    /// The Choice option labels, in code order.
    pub const OPTIONS: &'static [&'static str] = &["Native", "Half", "Quarter"];

    /// How much to divide the source dimensions by before measuring.
    pub const fn divisor(self) -> u32 {
        match self {
            FlowResolution::Native => 1,
            FlowResolution::Half => 2,
            FlowResolution::Quarter => 4,
        }
    }

    /// The mode for a stored Choice index, or `None` for an unknown code.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(FlowResolution::Native),
            1 => Some(FlowResolution::Half),
            2 => Some(FlowResolution::Quarter),
            _ => None,
        }
    }

    pub const fn code(self) -> u32 {
        match self {
            FlowResolution::Native => 0,
            FlowResolution::Half => 1,
            FlowResolution::Quarter => 2,
        }
    }
}

/// How hard the flow search works (docs/08 §3.1 "Vector detail"): pyramid depth
/// and the inverse-search iteration cap. More detail finds smaller and faster
/// motion at proportionally more time; it does not change what the vectors
/// *mean*, so it is safe to preview at Medium and export at Ultra — unlike
/// [`FlowResolution`], which changes the measurement itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VectorDetail {
    Low,
    #[default]
    Medium,
    High,
    Ultra,
}

impl VectorDetail {
    pub const OPTIONS: &'static [&'static str] = &["Low", "Medium", "High", "Ultra"];

    /// Inverse-search iterations per patch per level (docs/impl/optical-flow.md
    /// §1 step 2 pins Medium at the paper's ≤ 12).
    pub const fn iterations(self) -> u32 {
        match self {
            VectorDetail::Low => 6,
            VectorDetail::Medium => 12,
            VectorDetail::High => 20,
            VectorDetail::Ultra => 32,
        }
    }

    /// Variational-refinement fixed-point iterations per pyramid level,
    /// the third part of DIS. This is where most of the quality lives: it is
    /// what fills smoke, sky and darkness with a sensible field instead of
    /// leaving them flagged untrustworthy and crossfaded. Low still runs one
    /// pass — refinement off is not a user-reachable state, because the result
    /// is the artefact this project reported.
    pub const fn refine_iters(self) -> u32 {
        match self {
            VectorDetail::Low => 1,
            VectorDetail::Medium => 1,
            VectorDetail::High => 2,
            VectorDetail::Ultra => 3,
        }
    }

    /// The smallest pyramid dimension to build down to. A shallower pyramid
    /// (larger floor) is cheaper but blind to large motion; deeper than ~24 px
    /// and the 8×8 patches go frame-scale, which the §6.1 occlusion test
    /// measured as whole strips of garbage the finer levels cannot heal.
    pub const fn min_level_dim(self) -> u32 {
        match self {
            VectorDetail::Low => 48,
            VectorDetail::Medium => 24,
            VectorDetail::High => 24,
            VectorDetail::Ultra => 16,
        }
    }

    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(VectorDetail::Low),
            1 => Some(VectorDetail::Medium),
            2 => Some(VectorDetail::High),
            3 => Some(VectorDetail::Ultra),
            _ => None,
        }
    }

    pub const fn code(self) -> u32 {
        match self {
            VectorDetail::Low => 0,
            VectorDetail::Medium => 1,
            VectorDetail::High => 2,
            VectorDetail::Ultra => 3,
        }
    }
}

/// What synthesis does where a pixel is visible in only one of the two frames
/// (docs/08 §3.1 "Occlusion handling").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OcclusionMode {
    /// Take only the frame the pixel is actually visible in — sharp, and the
    /// right answer when the occlusion mask is right.
    #[default]
    VisibleOnly,
    /// Weight both frames anyway: trades ghosting at a revealed edge for fewer
    /// holes when the mask is wrong.
    Blend,
}

impl OcclusionMode {
    pub const OPTIONS: &'static [&'static str] = &["Visible only", "Blend"];

    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(OcclusionMode::VisibleOnly),
            1 => Some(OcclusionMode::Blend),
            _ => None,
        }
    }

    pub const fn code(self) -> u32 {
        match self {
            OcclusionMode::VisibleOnly => 0,
            OcclusionMode::Blend => 1,
        }
    }
}

/// Where confidence is too low to synthesise, this is what shows instead
/// (docs/08 §3.1 "Fallback"). Flow failure degrades to a picture, never to
/// garbage — that requirement is binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FlowFallback {
    /// Crossfade the two frames — soft, identical to the Blend policy.
    #[default]
    Blend,
    /// Show the nearer source frame — crisp, and a cleaner failure on footage
    /// where a ghosted double image would read as a fault.
    Nearest,
}

impl FlowFallback {
    pub const OPTIONS: &'static [&'static str] = &["Blend", "Nearest"];

    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(FlowFallback::Blend),
            1 => Some(FlowFallback::Nearest),
            _ => None,
        }
    }

    pub const fn code(self) -> u32 {
        match self {
            FlowFallback::Blend => 0,
            FlowFallback::Nearest => 1,
        }
    }
}

/// Which engine paints the in-between frame (docs/impl/addons.md §6.3).
///
/// The built-in one measures optical flow and warps the two frames along it,
/// and it is always there. RIFE is a trained model that paints the frame
/// directly, and it is an addon the user installs: a project that names it on
/// a machine without the pack keeps the choice, previews with the built-in
/// engine and says so, and refuses to export rather than quietly exporting
/// something else. Nothing here picks it on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FlowEngineChoice {
    /// The built-in dense inverse search, measured and warped on the card.
    #[default]
    Dis,
    /// The RIFE model pack, which paints the frame and measures nothing.
    Rife,
}

impl FlowEngineChoice {
    /// The Choice option labels, in code order.
    pub const OPTIONS: &'static [&'static str] = &["Built in", "RIFE"];

    /// The engine for a stored Choice index, or `None` for an unknown code.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(FlowEngineChoice::Dis),
            1 => Some(FlowEngineChoice::Rife),
            _ => None,
        }
    }

    pub const fn code(self) -> u32 {
        match self {
            FlowEngineChoice::Dis => 0,
            FlowEngineChoice::Rife => 1,
        }
    }
}

/// Optical-flow parameters (docs/08 §3.1). Every knob §3.1 specifies,
/// plus the engagement override and the HUD guard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowParams {
    /// Which engine paints the in-between frame. Defaults to the built-in one,
    /// so a project written before the choice existed reads as it rendered.
    #[serde(default)]
    pub engine: FlowEngineChoice,
    /// The resolution flow is measured at — independent of preview quality.
    /// Defaults to Native.
    #[serde(default)]
    pub resolution: FlowResolution,
    /// Pyramid depth and refinement iterations (docs/08 §3.1 "Vector detail").
    #[serde(default)]
    pub detail: VectorDetail,
    /// Regularisation weight, 0–100 (docs/08 §3.1 "Smoothness"): high means
    /// fewer tears and a gloopier field, low means crisper motion boundaries
    /// and more chance of a vector escaping on its own.
    #[serde(default = "default_smoothness")]
    pub smoothness: f64,
    /// What to do where a pixel exists in only one of the two frames.
    #[serde(default)]
    pub occlusion: OcclusionMode,
    /// What to show where confidence is too low to synthesise at all.
    #[serde(default)]
    pub fallback: FlowFallback,
    /// Bias static, well-textured regions toward pure blending (docs/08 §3.1
    /// step 5) — the guard that stops a game HUD smearing across the frame.
    /// On by default: this project's primary footage is game capture.
    #[serde(default = "default_true")]
    pub hud_guard: bool,
    /// Force flow on even where it cannot help. Flow normally passes
    /// through to Nearest unless the source rate through the retime undershoots
    /// the comp rate — at 100% speed there is no in-between frame to invent, so
    /// measuring one is pure cost. This overrides that gate.
    #[serde(default)]
    pub always: bool,
    /// The rate the footage is *interpreted* at for flow, in fps — a
    /// keyframeable value. `0` (the default) means Native:
    /// interpolate between adjacent source frames, unchanged behaviour. A
    /// positive rate below the native one conforms the clip: flow brackets the
    /// source frames spaced `1/rate` apart and interpolates between *those*, so
    /// high-framerate footage (whose adjacent frames are near-identical) gets
    /// real slow-motion — the standard "interpret footage as N fps" trick.
    /// Animatable so the conform rate can ramp over the clip; read at frame
    /// time through [`FlowParams::input_fps_at`]. Ignored when it reads Native
    /// or a rate at/above the source's own.
    #[serde(
        default = "crate::anim::Property::zero",
        skip_serializing_if = "input_fps_is_native"
    )]
    pub input_fps: crate::anim::Property,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for FlowParams {
    fn default() -> Self {
        Self {
            engine: FlowEngineChoice::Dis,
            resolution: FlowResolution::Native,
            detail: VectorDetail::Medium,
            smoothness: DEFAULT_SMOOTHNESS,
            occlusion: OcclusionMode::VisibleOnly,
            fallback: FlowFallback::Blend,
            hud_guard: true,
            always: false,
            input_fps: crate::anim::Property::zero(),
            extra: serde_json::Map::new(),
        }
    }
}

impl FlowParams {
    /// The conform rate the footage is interpreted at for flow at layer-local
    /// time `lt`, or `None` for the source's native rate. The
    /// rate is a keyframeable [`crate::anim::Property`]; a value below `0.5`
    /// (i.e. one that rounds to 0 fps) reads as Native, so a keyframe ramp from
    /// Native to a real rate resolves cleanly. Callers pass the result straight
    /// to the frame picker, which itself ignores a rate at/above native.
    pub fn input_fps_at(&self, lt: f64) -> Option<f64> {
        let v = self.input_fps.value_at(lt);
        (v >= 0.5).then_some(v)
    }

    /// The rate the clip is actually *read* at for flow at local time `lt`,
    /// given its native rate: the conform rate when one is set and sits below
    /// native, otherwise native itself. The one place that rule lives,
    /// so the frame picker, the engagement gate and the cache key can never
    /// disagree about which frames flow is working between.
    pub fn read_fps_at(&self, lt: f64, native_fps: f64) -> f64 {
        match self.input_fps_at(lt) {
            Some(r) if r > 0.0 && r < native_fps => r,
            _ => native_fps,
        }
    }

    /// Whether flow has anything to do here: it engages only when it can help.
    ///
    /// Flow invents the frame *between* two real ones. At 100% speed on
    /// matched rates there is no such frame — every comp frame lands on a
    /// source frame — so measuring motion is pure cost for a picture that would
    /// be identical. The question is whether one source frame would otherwise
    /// hold across two or more comp frames: the source advances
    /// `|speed| · source_rate / comp_rate` frames per comp frame, and anything
    /// under 1 repeats. `source_fps` is the rate the clip is *read* at, so pass
    /// the conform rate ([`Self::input_fps_at`]) when one is set — conforming
    /// 600 fps footage to 24 is exactly how you make flow engage on material
    /// whose adjacent frames barely move.
    ///
    /// [`Self::always`] overrides this: the user asked for flow, they get flow.
    /// Degenerate rates (either at or below zero) answer `false` — nothing is
    /// known about the timing, and the cheap mistake is not to spend.
    pub fn engages(&self, source_fps: f64, comp_fps: f64, speed: f64) -> bool {
        if self.always {
            return true;
        }
        // NaN included: an unknowable rate declines rather than guesses.
        if !(source_fps.is_finite() && source_fps > 0.0) {
            return false;
        }
        if !(comp_fps.is_finite() && comp_fps > 0.0) {
            return false;
        }
        // A freeze (speed 0) holds one frame indefinitely: nothing to
        // interpolate *towards*, so flow stays out of it too.
        let advance = speed.abs() * source_fps / comp_fps;
        advance > 0.0 && advance < 1.0
    }
}

/// True when a flow input rate is a plain, un-keyframed Native (0 fps): the
/// common case, kept out of the serialised file so a Native flow clip writes
/// exactly as it did before the rate became keyframeable.
fn input_fps_is_native(p: &crate::anim::Property) -> bool {
    matches!(&p.animation, crate::anim::Animation::Static(v) if *v < 0.5) && p.extra.is_empty()
}

fn default_true() -> bool {
    true
}

/// How fast a retime *property* is running at local time `lt` — source seconds
/// per local second, 1.0 being 100%.
///
/// Both a Layer's and a Clip's retime are stored as an [`crate::anim::Property`]
/// mapping local time to source time (not as a [`Retime`] store, which has its
/// own closed-form [`Retime::speed_at`]), so the speed is the slope of that
/// curve. A central difference gives it: exact on the linear stretches that
/// dominate, and at a keyframe it averages the two sides, which is the right
/// answer for the one thing this feeds — the flow engagement gate,
/// where being a hair out at the instant a ramp changes gradient decides
/// nothing. `None` is un-retimed: 100%.
pub fn property_speed_at(retime: Option<&crate::anim::Property>, lt: f64) -> f64 {
    let Some(p) = retime else {
        return 1.0;
    };
    // Small enough to be local on a real ramp, large enough that the
    // subtraction keeps its significant digits at ordinary timeline times.
    const H: f64 = 1e-3;
    (p.value_at(lt + H) - p.value_at(lt - H)) / (2.0 * H)
}

/// docs/08 §3.1: Smoothness is 0–100, default 50.
pub const DEFAULT_SMOOTHNESS: f64 = 50.0;

fn default_smoothness() -> f64 {
    DEFAULT_SMOOTHNESS
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
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
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
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
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
/// AE-compatible (docs/04-RETIMING.md §4.2). Endpoint positions come
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
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
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
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
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
        // frozen tail beats a panic (engine crates never panic).
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

    /// Build a value-lens retime from AE Time Remap keyframes: local time →
    /// source time, each side carrying the same bezier tangent (`SideInterp`)
    /// the transform graph uses. Every consecutive pair becomes a
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

    /// The value-lens keyframes with bezier tangents (local time → source time,
    /// each side a `SideInterp`) — the inverse of [`Self::from_source_keyframes`],
    /// so the value lens can draw and edit *any* store with the transform
    /// graph's own handles. A [`MapSegment`] contributes its stored
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    #[test]
    fn source_keyframes_evaluate_like_a_transform_property() {
        // The whole point: a Time Remap built from bezier keyframes must
        // render bit-for-bit like the same keys on a transform property.
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

    /// Flow engages only where a source frame would otherwise hold across two
    /// or more comp frames.
    #[test]
    fn flow_engages_only_where_it_can_help() {
        let p = FlowParams::default();
        // 30 fps source into a 30 fps comp: at 100% every comp frame lands on
        // a source frame, so there is no in-between frame to invent.
        assert!(!p.engages(30.0, 30.0, 1.0));
        // Half speed holds each source frame for two comp frames — the case
        // flow exists for.
        assert!(p.engages(30.0, 30.0, 0.5));
        // Faster than real time skips frames; nothing to interpolate.
        assert!(!p.engages(30.0, 30.0, 2.0));
        // A freeze has no motion to interpolate towards either.
        assert!(!p.engages(30.0, 30.0, 0.0));
        // Reverse is symmetric: it is the magnitude that decides.
        assert!(p.engages(30.0, 30.0, -0.5));
        assert!(!p.engages(30.0, 30.0, -1.0));
        // 60 fps source into a 30 fps comp already has frames to spare at
        // 100%, and only starts repeating below half speed.
        assert!(!p.engages(60.0, 30.0, 1.0));
        assert!(!p.engages(60.0, 30.0, 0.5));
        assert!(p.engages(60.0, 30.0, 0.25));
        // Degenerate rates decline rather than guess.
        assert!(!p.engages(0.0, 30.0, 0.5));
        assert!(!p.engages(30.0, 0.0, 0.5));
    }

    /// The conform rate is what the gate measures against:
    /// 600 fps footage at 10% speed still advances 60 source frames per comp
    /// frame, so flow would decline — until the clip is conformed to 24, at
    /// which point there is real motion between the bracketing frames.
    #[test]
    fn a_conform_rate_is_what_makes_high_framerate_footage_engage() {
        let native = FlowParams::default();
        assert!(!native.engages(native.read_fps_at(0.0, 600.0), 30.0, 0.1));
        let conformed = FlowParams {
            input_fps: crate::anim::Property::fixed(24.0),
            ..FlowParams::default()
        };
        assert_eq!(conformed.read_fps_at(0.0, 600.0), 24.0);
        assert!(conformed.engages(conformed.read_fps_at(0.0, 600.0), 30.0, 0.1));
        // A conform rate at or above native is ignored: it cannot manufacture
        // frames the source does not have.
        assert_eq!(conformed.read_fps_at(0.0, 24.0), 24.0);
        assert_eq!(conformed.read_fps_at(0.0, 12.0), 12.0);
    }

    /// A project written before the engine choice existed reads as the engine
    /// it was rendered with (docs/impl/addons.md §6.3).
    ///
    /// Its own test rather than a line in the one above, because this is the
    /// promise that matters when an addon is involved: a file that mentions no
    /// engine must never come back naming a model the machine may not even
    /// have, and the whole group has to survive the trip beside it.
    #[test]
    fn a_project_written_before_the_engine_choice_reads_as_built_in() {
        let written = r#"{"resolution":"Half","detail":"High","smoothness":40.0,
                          "occlusion":"Blend","fallback":"Nearest",
                          "hud_guard":false,"always":true}"#;
        let old: FlowParams = serde_json::from_str(written).unwrap();
        assert_eq!(old.engine, FlowEngineChoice::Dis);
        assert_eq!(old.resolution, FlowResolution::Half);
        assert!(old.always, "the rest of the group came through unchanged");

        let chosen = FlowParams {
            engine: FlowEngineChoice::Rife,
            ..FlowParams::default()
        };
        let back: FlowParams =
            serde_json::from_value(serde_json::to_value(&chosen).unwrap()).unwrap();
        assert_eq!(back.engine, FlowEngineChoice::Rife, "and a choice survives");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod keyframe_seam_tests {
    use super::*;
    use crate::time::Rational;

    /// A curve already in the value vocabulary — which is what the envelope
    /// writes, and what `from_source_keyframes` produces — does round-trip
    /// exactly. The seam is lossy in one direction only.
    #[test]
    fn a_map_shaped_curve_round_trips_exactly() {
        let seed = Retime::single_ramp(
            Rational::new(2, 1).unwrap(),
            Rational::ZERO,
            Rational::ONE,
            Rational::new(3, 1).unwrap(),
            Ease::Linear,
        );
        // Once through, so the store holds MapSegments rather than a Rate.
        let mapped = Retime::from_source_keyframes(&seed.source_keyframes()).expect("a curve");
        let again = Retime::from_source_keyframes(&mapped.source_keyframes()).expect("a curve");

        for i in 0..=20 {
            let t = f64::from(i) / 10.0;
            assert!(
                (mapped.evaluate(t) - again.evaluate(t)).abs() < 1e-9,
                "drifted at t={t}"
            );
        }
    }
}
