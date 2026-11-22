//! Holographic foil's motion: the slow built-in wobble, the turn of the layer
//! and the comp camera when Follow layer rotation is on, and the pose at the
//! shutter's start, which the colours' motion blur is measured against.

use std::sync::Arc;

use super::HolographicFoil;
use crate::anim::Animation;
use crate::expression::ExpressionContext;
use crate::fx::params::{ParamId, Params, Value};
use crate::fx::registry::ResolveCx;
use crate::model::{EffectInstance, EffectValue};

/// A rotation by rows.
pub(super) type Mat = [[f64; 3]; 3];

/// The layer's and the camera's turn as the sheet sees it: degrees about x,
/// y and z (`Rx Ry Rz`), and 1 when it is in play.
const DERIVED_TURN: ParamId = ParamId::new("derived.turn");
/// How long the shutter is open, seconds. Absent or 0 is no blur.
const DERIVED_SHUTTER: ParamId = ParamId::new("derived.shutter");
/// At the shutter's start: tilt x, tilt y, rig rotation, spin.
const DERIVED_PREV_TILT: ParamId = ParamId::new("derived.prev_tilt");
/// At the shutter's start: spot A and spot B's directions.
const DERIVED_PREV_AB: ParamId = ParamId::new("derived.prev_ab");
/// At the shutter's start: spot C and the panel's directions.
const DERIVED_PREV_CP: ParamId = ParamId::new("derived.prev_cp");
/// At the shutter's start: the layer's and the camera's turn.
const DERIVED_PREV_TURN: ParamId = ParamId::new("derived.prev_turn");

/// The pose at the shutter's start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Prev {
    /// The controls with their values then.
    pub f: HolographicFoil,
    pub time: f64,
    pub turn: Option<[f64; 3]>,
}

/// What moves the sheet this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Motion {
    /// Layer time, seconds.
    pub time: f64,
    pub turn: Option<[f64; 3]>,
    pub prev: Option<Prev>,
}

fn turn_of(v: [f32; 4]) -> Option<[f64; 3]> {
    (v[3] > 0.5).then(|| [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])])
}

/// What the resolve pushed for `f`, read back from `p`.
pub(crate) fn read(f: &HolographicFoil, p: Params<'_>, time: f64) -> Motion {
    let turn = turn_of(p.vec4(DERIVED_TURN, [0.0; 4]));
    let dt = f64::from(p.float(DERIVED_SHUTTER, 0.0));
    let prev = (dt > 0.0).then(|| {
        let t = p.vec4(
            DERIVED_PREV_TILT,
            [f.tilt_x, f.tilt_y, f.rig_rotation, f.spin],
        );
        let ab = p.vec4(
            DERIVED_PREV_AB,
            [f.spot_a_x, f.spot_a_y, f.spot_b_x, f.spot_b_y],
        );
        let cp = p.vec4(
            DERIVED_PREV_CP,
            [f.spot_c_x, f.spot_c_y, f.panel_x, f.panel_y],
        );
        let then = HolographicFoil {
            tilt_x: t[0],
            tilt_y: t[1],
            rig_rotation: t[2],
            spin: t[3],
            spot_a_x: ab[0],
            spot_a_y: ab[1],
            spot_b_x: ab[2],
            spot_b_y: ab[3],
            spot_c_x: cp[0],
            spot_c_y: cp[1],
            panel_x: cp[2],
            panel_y: cp[3],
            ..*f
        };
        Prev {
            f: then,
            time: time - dt,
            turn: turn_of(p.vec4(DERIVED_PREV_TURN, [0.0; 4])),
        }
    });
    Motion { time, turn, prev }
}

fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

fn rx(deg: f64) -> Mat {
    let (s, c) = deg.to_radians().sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn ry(deg: f64) -> Mat {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rz(deg: f64) -> Mat {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// A fixed share of a turn from the seed and a salt, through splitmix64's
/// finaliser (Sebastiano Vigna, public domain, prng.di.unimi.it).
fn phase(seed: u32, salt: u32) -> f64 {
    let mut z = u64::from(seed) ^ (u64::from(salt) << 32);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    std::f64::consts::TAU * (z >> 11) as f64 / (1u64 << 53) as f64
}

/// The built-in wobble at `time`, degrees about x and y. Straight grooves
/// swing mostly about their own axis, which sweeps the hue, with a smaller,
/// slower swing across it; every other pattern traces a slow ellipse.
pub(super) fn wobble(f: &HolographicFoil, time: f64) -> [f64; 2] {
    let amp = f64::from(f.auto_tilt.max(0.0));
    if amp <= 0.0 {
        return [0.0; 2];
    }
    let w = std::f64::consts::TAU * f64::from(f.auto_tilt_speed.max(0.0)) * time;
    let p1 = phase(f.seed, 1);
    if f.pattern == 0 && f.material != super::VORTEX {
        let (s, c) = f64::from(f.groove_angle + f.spin).to_radians().sin_cos();
        let main = amp * (w + p1).sin();
        let minor = 0.3 * amp * (0.71 * w + phase(f.seed, 2)).sin();
        // Along the grooves (c, s), and across them (s, -c).
        [main * c + minor * s, main * s - minor * c]
    } else {
        [amp * (w + p1).sin(), 0.7 * amp * (w + p1).cos()]
    }
}

/// The sheet's rotation at `time`: the layer's turn, then the tilt with the
/// wobble. Its columns are the sheet's x axis, y axis and normal.
pub(super) fn frame(f: &HolographicFoil, time: f64, turn: Option<[f64; 3]>) -> Mat {
    let [wx, wy] = wobble(f, time);
    let tilt = mul(&rx(f64::from(f.tilt_x) + wx), &ry(f64::from(f.tilt_y) + wy));
    match turn {
        Some([a, b, c]) => mul(&mul(&mul(&rx(a), &ry(b)), &rz(c)), &tilt),
        None => tilt,
    }
}

/// Column `c` of `m`.
pub(super) fn column(m: &Mat, c: usize) -> [f64; 3] {
    [m[0][c], m[1][c], m[2][c]]
}

/// The owning 3D layer's turn, its parents' folded in and the active
/// camera's taken out, at comp time `t`, as the sheet's `Rx Ry Rz` degrees.
/// `None` off a layer or on a 2D one, whose rotation only turns the picture
/// on screen.
fn layer_turn(context: &Arc<ExpressionContext>, t: f64) -> Option<[f64; 3]> {
    let doc = &context.document;
    let comp = doc.comp(context.comp?)?;
    let layer = comp.layers.iter().find(|l| Some(l.id) == context.layer)?;
    if !layer.switches.three_d {
        return None;
    }
    let read = crate::model::context_reader(context, t);
    let turn = |l: &crate::model::Layer| {
        crate::camera::rotation(crate::model::placement(l, t, &read).rotation_deg)
    };
    let mut m = turn(layer);
    for id in crate::model::layer_parent_chain(comp, layer.id) {
        if let Some(parent) = comp.layers.iter().find(|l| l.id == id) {
            m = mul(&turn(parent), &m);
        }
    }
    if let Some(pose) = crate::track::active_camera_pose(doc, comp, t) {
        let c = crate::camera::rotation(pose.rotation_deg);
        let ct = [
            [c[0][0], c[1][0], c[2][0]],
            [c[0][1], c[1][1], c[2][1]],
            [c[0][2], c[1][2], c[2][2]],
        ];
        m = mul(&ct, &m);
    }
    // The comp's y runs down and its z away from the viewer; the sheet's y
    // runs up and its z toward the viewer.
    let sign = [1.0, -1.0, -1.0];
    let s = |r: usize, c: usize| m[r][c] * sign[r] * sign[c];
    let b = s(0, 2).clamp(-1.0, 1.0).asin();
    let a = (-s(1, 2)).atan2(s(2, 2));
    let c = (-s(0, 1)).atan2(s(0, 0));
    Some([a.to_degrees(), b.to_degrees(), c.to_degrees()])
}

/// The rows the shutter reads again at its start.
const SHUTTER_ROWS: [&str; 12] = [
    "tilt_x",
    "tilt_y",
    "rig_rotation",
    "spin",
    "spot_a_x",
    "spot_a_y",
    "spot_b_x",
    "spot_b_y",
    "spot_c_x",
    "spot_c_y",
    "panel_x",
    "panel_y",
];

/// Whether the picture changes with layer time while the frame's own rows
/// hold still: the wobble swings (Auto tilt above 0, keyed or driven), or
/// Motion blur reads a pose from the shutter's start that the frame's rows do
/// not name (a keyed or driven row it reads, or Follow layer rotation). Only
/// then does layer time join the frame's key, so a still sheet is drawn once.
pub(crate) fn moves_with_time(inst: &EffectInstance) -> bool {
    let float = |id: &str| match inst.param(id) {
        Some(EffectValue::Float(p)) => Some(p),
        _ => None,
    };
    let driven = |id: &str| float(id).is_some_and(|p| !matches!(p.animation, Animation::Static(_)));
    let live = |id: &str| driven(id) || float(id).is_some_and(|p| p.value_at(0.0) > 0.0);
    let follow = matches!(inst.param("follow_rotation"), Some(EffectValue::Choice(1)));
    live("auto_tilt") || (live("motion_blur") && (follow || SHUTTER_ROWS.into_iter().any(driven)))
}

/// Pushes layer time when the picture moves with it, the turn when Follow
/// layer rotation is on, and the shutter with the controls' values at its
/// start when Motion blur is on.
pub(crate) fn resolve(cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
    if moves_with_time(cx.inst) {
        push(HolographicFoil::DERIVED_TIME, Value::Float(cx.lt as f32));
    }
    let follow = matches!(
        cx.inst.param("follow_rotation"),
        Some(EffectValue::Choice(1))
    );
    let now = cx.context.comp_time;
    let push_turn = |push: &mut dyn FnMut(ParamId, Value), id, turn: Option<[f64; 3]>| {
        if let Some([a, b, c]) = turn {
            push(id, Value::Vec4([a as f32, b as f32, c as f32, 1.0]));
        }
    };
    if follow {
        push_turn(push, DERIVED_TURN, layer_turn(&cx.context, now));
    }
    let share = cx
        .inst
        .float_at_with_context("motion_blur", cx.lt, cx.context.clone())
        .unwrap_or(0.0)
        / 100.0;
    let doc = &cx.context.document;
    let fps = cx
        .context
        .comp
        .and_then(|id| doc.comp(id))
        .map_or(24.0, |c| c.frame_rate.fps());
    let dt = share.clamp(0.0, 1.0) / fps.max(1.0);
    if dt <= 0.0 {
        return;
    }
    push(DERIVED_SHUTTER, Value::Float(dt as f32));
    let then = Arc::new(ExpressionContext {
        comp_time: now - dt,
        ..(*cx.context).clone()
    });
    let at = |id: &str| {
        cx.inst
            .float_at_with_context(id, cx.lt - dt, then.clone())
            .map(|v| v as f32)
    };
    let four = |ids: [&str; 4]| -> Option<[f32; 4]> {
        Some([at(ids[0])?, at(ids[1])?, at(ids[2])?, at(ids[3])?])
    };
    for (id, ids) in [
        (
            DERIVED_PREV_TILT,
            ["tilt_x", "tilt_y", "rig_rotation", "spin"],
        ),
        (
            DERIVED_PREV_AB,
            ["spot_a_x", "spot_a_y", "spot_b_x", "spot_b_y"],
        ),
        (
            DERIVED_PREV_CP,
            ["spot_c_x", "spot_c_y", "panel_x", "panel_y"],
        ),
    ] {
        if let Some(v) = four(ids) {
            push(id, Value::Vec4(v));
        }
    }
    if follow {
        push_turn(push, DERIVED_PREV_TURN, layer_turn(&then, now - dt));
    }
}
