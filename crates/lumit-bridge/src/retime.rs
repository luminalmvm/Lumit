//! The bridge v0.4 Retime ops — the simple speed row and the graph editor's
//! segment/boundary edits (docs/04-RETIMING.md; glossary: Retime, speed).
//!
//! # In plain terms
//!
//! A footage layer can play at a different speed than its source: half speed for
//! slow motion, a ramp from fast to slow, and so on. That map lives in a
//! [`Retime`] store on the layer. These ops are the exact edits the egui speed
//! row and graph header commit — enable/disable the retime, set a constant
//! speed, change a segment's ease preset (Lin/Slow/Fast/Smth/Shrp), convert a
//! curved (Map) segment to a plain Rate segment, and drag a boundary to a new
//! frame. Each one rebuilds the store and commits one [`Op::SetLayerRetime`], so
//! undo is one step and the two frontends cannot drift.
//!
//! Only what the egui UI can do today is exposed — no aspirational surface.

use crate::edits::{layer_local_seconds, rational_at};
use crate::err_json;
use crate::snapshot::snapshot_value;
use crate::state::{parse_comp_layer, Bridge};
use lumit_core::model::{Composition, Layer, LayerKind};
use lumit_core::ops::Op;
use lumit_core::retime::{Ease, Retime};
use lumit_core::time::Rational;
use serde_json::json;
use uuid::Uuid;

/// Resolve a comp id and layer id to the comp (cloned), the layer (cloned), and
/// the layer's current Retime (cloned; `None` when it plays at source rate). A
/// non-footage layer, or an unknown comp/layer, is a calm error prefixed `ctx`.
fn resolve_retime(
    bridge: &Bridge,
    comp_id: &str,
    layer_id: &str,
    ctx: &str,
) -> Result<(Uuid, Composition, Layer, Option<Retime>), String> {
    let (comp, layer) = parse_comp_layer(comp_id, layer_id).map_err(|e| format!("{ctx}: {e}"))?;
    let doc = bridge.store.snapshot();
    let c = doc
        .comp(comp)
        .cloned()
        .ok_or_else(|| format!("{ctx}: unknown composition"))?;
    let l = c
        .layers
        .iter()
        .find(|l| l.id == layer)
        .cloned()
        .ok_or_else(|| format!("{ctx}: unknown layer"))?;
    let retime = match &l.kind {
        LayerKind::Footage { retime, .. } => retime.clone(),
        _ => return Err(format!("{ctx}: only footage layers carry a Retime")),
    };
    Ok((comp, c, l, retime))
}

/// The layer's local domain length (seconds) — its out point, mirroring the
/// egui speed row's `dur = ctx.layer.out_point.0`.
fn layer_duration(l: &Layer) -> Rational {
    l.out_point.0
}

/// A percentage as a speed rational on the egui grid (`pct/100`, denominator
/// 1000) — `speed_rows`' `to_speed`, so a bridge-set speed matches.
fn to_speed(pct: f64) -> Rational {
    Rational::from_f64_on_grid(pct / 100.0, 1000).unwrap_or(Rational::ONE)
}

/// Commit a layer's new (or cleared) Retime as one [`Op::SetLayerRetime`].
fn commit_retime(bridge: &mut Bridge, comp: Uuid, layer: Uuid, retime: Option<Retime>) -> String {
    crate::state::commit(
        bridge,
        Op::SetLayerRetime {
            comp,
            layer,
            retime,
        },
        "set retime",
    )
}

/// Enable or disable a footage layer's Retime — the Time stopwatch. Enabling
/// seeds an identity store over the layer's domain (source running at 100%);
/// disabling clears it (plays at source rate). Mirrors the egui Time stopwatch,
/// which enables to at least the start/end boundary pair (AE Time Remap).
pub(crate) fn set_retime_enabled(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    enabled: bool,
) -> String {
    let ctx = "set retime enabled";
    let (comp, _c, l, _retime) = match resolve_retime(bridge, comp_id, layer_id, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let retime = enabled.then(|| Retime::identity(layer_duration(&l), Rational::ZERO));
    commit_retime(bridge, comp, l.id, retime)
}

/// Set a constant playback speed (percent) — the speed row's value scrub when
/// the channel is not keyframed. 100% clears the Retime (plays at source rate),
/// exactly as the egui row does; any other speed becomes a single constant-speed
/// segment over the layer's domain.
pub(crate) fn set_retime_speed(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    speed_percent: f64,
) -> String {
    let ctx = "set retime speed";
    let (comp, _c, l, _retime) = match resolve_retime(bridge, comp_id, layer_id, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let retime = if (speed_percent - 100.0).abs() < 1e-6 {
        None
    } else {
        Some(Retime::constant_speed(
            layer_duration(&l),
            Rational::ZERO,
            to_speed(speed_percent),
        ))
    };
    commit_retime(bridge, comp, l.id, retime)
}

/// Parse an ease preset name — the graph header's Lin/Slow/Fast/Smth/Shrp row.
fn parse_ease(name: &str) -> Option<Ease> {
    Some(match name {
        "Lin" | "Linear" => Ease::Linear,
        "Slow" => Ease::Slow,
        "Fast" => Ease::Fast,
        "Smth" | "Smooth" => Ease::Smooth,
        "Shrp" | "Sharp" => Ease::Sharp,
        _ => return None,
    })
}

/// Set the ease of the Rate segment covering the playhead `frame` — the graph
/// header's ease preset row (`with_segment_ease`, §9.2). Downstream boundary
/// source positions recompute exactly. A no-op (no Retime, the playhead lands in
/// a Map segment, or out of domain) is a calm error.
pub(crate) fn set_segment_preset(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    frame: i64,
    ease: &str,
) -> String {
    let ctx = "set segment preset";
    let (comp, c, l, retime) = match resolve_retime(bridge, comp_id, layer_id, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let Some(ease) = parse_ease(ease) else {
        return err_json(format!("set segment preset: unknown ease '{ease}'"));
    };
    let Some(retime) = retime else {
        return err_json("set segment preset: the layer has no Retime");
    };
    let lt = layer_local_seconds(&c, &l, frame).max(0.0);
    let Some(new_rt) = retime.with_segment_ease(rational_at(lt), ease) else {
        return err_json("set segment preset: the playhead is not on a Rate segment");
    };
    commit_retime(bridge, comp, l.id, Some(new_rt))
}

/// Convert the Map segment covering the playhead `frame` to a Rate segment — the
/// graph header's →Rate button (`with_segment_as_rate`, §5.2). The source
/// advance is pinned exactly; the reply carries the fit `drift` in seconds (a
/// non-zero drift means the ease shape could not follow the curve perfectly, the
/// "fitted" badge). A no-op (already a Rate, or an unfollowable curve) is a calm
/// error.
pub(crate) fn segment_to_rate(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    frame: i64,
) -> String {
    let ctx = "segment to rate";
    let (comp, c, l, retime) = match resolve_retime(bridge, comp_id, layer_id, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let Some(retime) = retime else {
        return err_json("segment to rate: the layer has no Retime");
    };
    let lt = layer_local_seconds(&c, &l, frame).max(0.0);
    let Some((new_rt, drift)) = retime.with_segment_as_rate(rational_at(lt)) else {
        return err_json("segment to rate: the playhead is not on a convertible Map segment");
    };
    // Commit, then attach the drift to the refreshed snapshot the reply carries
    // (an additive field an older reader ignores).
    if let Err(e) = bridge.store.commit(Op::SetLayerRetime {
        comp,
        layer: l.id,
        retime: Some(new_rt),
    }) {
        return err_json(format!("segment to rate: {e}"));
    }
    let mut v = snapshot_value(bridge);
    v["drift"] = json!(drift);
    v.to_string()
}

/// Move the value-lens boundary at `index` to comp `frame` — the graph's Time
/// keyframe drag (the value lens keys every boundary). The boundary's source
/// position is kept; only its local time moves, and the store rebuilds through
/// [`Retime::from_value_keyframes`]. The first boundary is pinned at local time
/// 0 (docs/04-RETIMING.md §3, the same pin the egui `retime_drag_time` applies),
/// so a drag on it re-pins the start rather than shifting the domain. A drag that
/// would collide with or cross a neighbour, or land on a layer with no Retime, is
/// a calm error.
pub(crate) fn drag_boundary(
    bridge: &mut Bridge,
    comp_id: &str,
    layer_id: &str,
    index: i64,
    frame: i64,
) -> String {
    let ctx = "drag boundary";
    let (comp, c, l, retime) = match resolve_retime(bridge, comp_id, layer_id, ctx) {
        Ok(t) => t,
        Err(e) => return err_json(e),
    };
    let Some(retime) = retime else {
        return err_json("drag boundary: the layer has no Retime");
    };
    let mut keys = retime.value_keyframes();
    let Ok(index) = usize::try_from(index) else {
        return err_json("drag boundary: index out of range");
    };
    if index >= keys.len() {
        return err_json("drag boundary: index out of range");
    }
    // The first boundary is pinned at local time 0; any other takes the dragged
    // (clamped ≥ 0) local time.
    let new_t = if index == 0 {
        Rational::ZERO
    } else {
        rational_at(layer_local_seconds(&c, &l, frame).max(0.0))
    };
    keys[index].0 = new_t;
    let Some(new_rt) = Retime::from_value_keyframes(&keys) else {
        return err_json("drag boundary: the move would collide with a neighbour");
    };
    commit_retime(bridge, comp, l.id, Some(new_rt))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::state::{snapshot, undo, Bridge};
    use lumit_core::model::{
        Composition, FootageItem, LayerKind, LinearColour, MediaRef, MotionBlur, ProjectItem,
        Switches, TransformGroup,
    };
    use lumit_core::store::DocumentStore;
    use lumit_core::time::{CompTime, Duration, FrameRate};
    use serde_json::Value;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("reply is valid JSON")
    }

    /// A bridge holding one 60 fps comp with a single footage layer spanning
    /// [0, 5] s. Returns the bridge, comp id and layer id.
    fn bridge_with_footage() -> (Bridge, String, String) {
        let item = Uuid::now_v7();
        let layer = Layer {
            id: Uuid::now_v7(),
            name: "clip".into(),
            kind: LayerKind::Footage { item, retime: None },
            in_point: CompTime(Rational::new(0, 1).unwrap()),
            out_point: CompTime(Rational::new(5, 1).unwrap()),
            start_offset: CompTime(Rational::new(0, 1).unwrap()),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        let layer_id = layer.id.to_string();
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Scene".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: MotionBlur::default(),
            extra: serde_json::Map::new(),
        };
        let comp_id = comp.id.to_string();
        let store = DocumentStore::new(lumit_core::model::Document::new());
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Footage(FootageItem {
                    id: item,
                    name: "clip".into(),
                    media: MediaRef {
                        relative_path: "clip".into(),
                        absolute_path: String::new(),
                        fingerprint: None,
                        extra: serde_json::Map::new(),
                    },
                    extra: serde_json::Map::new(),
                })),
            })
            .unwrap();
        store
            .commit(Op::AddItem {
                index: 1,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        let b = Bridge {
            store,
            path: None,
            media: crate::media::MediaCache::default(),
        };
        (b, comp_id, layer_id)
    }

    /// The first (and only) footage layer's retime block, locating the comp item
    /// wherever it sits in the item list (the footage item precedes it here).
    fn layer_retime(snap: &Value) -> Value {
        let comp = snap["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["kind"] == json!("composition"))
            .expect("a composition in the snapshot");
        comp["comp"]["layers"][0]["retime"].clone()
    }

    #[test]
    fn set_speed_creates_and_clears_a_retime() {
        let (mut b, comp, layer) = bridge_with_footage();
        // 50% speed → a single constant-speed rate segment.
        let snap = parse(&set_retime_speed(&mut b, &comp, &layer, 50.0));
        assert_eq!(snap["ok"], json!(true));
        let rt = layer_retime(&snap);
        assert!(rt.is_object(), "a retime now exists");
        assert_eq!(rt["segments"][0]["kind"], json!("rate"));
        assert_eq!(rt["segments"][0]["v0"], json!(0.5));
        assert_eq!(rt["segments"][0]["v1"], json!(0.5));
        // Back to 100% clears it.
        let snap = parse(&set_retime_speed(&mut b, &comp, &layer, 100.0));
        assert_eq!(layer_retime(&snap), Value::Null);
    }

    #[test]
    fn enable_seeds_identity_then_disable_clears() {
        let (mut b, comp, layer) = bridge_with_footage();
        let snap = parse(&set_retime_enabled(&mut b, &comp, &layer, true));
        let rt = layer_retime(&snap);
        assert!(rt.is_object());
        // Identity: two boundaries, one 100% rate segment.
        assert_eq!(rt["boundaries"].as_array().unwrap().len(), 2);
        assert_eq!(rt["segments"][0]["v0"], json!(1.0));
        let snap = parse(&set_retime_enabled(&mut b, &comp, &layer, false));
        assert_eq!(layer_retime(&snap), Value::Null);
    }

    #[test]
    fn segment_preset_sets_the_ease() {
        let (mut b, comp, layer) = bridge_with_footage();
        // A ramp so the segment is a Rate segment; then ease it.
        set_retime_speed(&mut b, &comp, &layer, 50.0);
        let snap = parse(&set_segment_preset(&mut b, &comp, &layer, 30, "Smth"));
        assert_eq!(snap["ok"], json!(true));
        assert_eq!(layer_retime(&snap)["segments"][0]["ease"], json!("Smooth"));
    }

    #[test]
    fn segment_to_rate_reports_drift_on_a_map_store() {
        let (mut b, comp, layer) = bridge_with_footage();
        // Build a genuine Map store (bezier tangents) whose constant source speed
        // (= the chord, 0.6) fits cleanly to a Rate segment, then convert it and
        // read the drift back. The influences are the polynomial 1/3, so the fit
        // is exact (drift ≈ 0) — enough to prove the plumbing and the drift field.
        let third = 1.0 / 3.0;
        let keys = [
            lumit_core::anim::Keyframe {
                time: Rational::ZERO,
                value: 0.0,
                interp_in: lumit_core::anim::SideInterp::Linear,
                interp_out: lumit_core::anim::SideInterp::Bezier {
                    speed: 0.6,
                    influence: third,
                },
            },
            lumit_core::anim::Keyframe {
                time: Rational::new(5, 1).unwrap(),
                value: 3.0,
                interp_in: lumit_core::anim::SideInterp::Bezier {
                    speed: 0.6,
                    influence: third,
                },
                interp_out: lumit_core::anim::SideInterp::Linear,
            },
        ];
        let rt = Retime::from_source_keyframes(&keys).expect("a map store");
        b.store
            .commit(Op::SetLayerRetime {
                comp: Uuid::parse_str(&comp).unwrap(),
                layer: Uuid::parse_str(&layer).unwrap(),
                retime: Some(rt),
            })
            .unwrap();
        let reply = parse(&segment_to_rate(&mut b, &comp, &layer, 30));
        assert_eq!(reply["ok"], json!(true));
        assert!(reply.get("drift").is_some(), "the reply carries drift");
        assert_eq!(layer_retime(&reply)["segments"][0]["kind"], json!("rate"));
    }

    #[test]
    fn drag_boundary_moves_an_interior_boundary() {
        let (mut b, comp, layer) = bridge_with_footage();
        // A three-boundary value store: 0, 2, 5 s → source 0, 1, 3 s.
        let rt = Retime::from_value_keyframes(&[
            (Rational::ZERO, Rational::ZERO),
            (Rational::new(2, 1).unwrap(), Rational::new(1, 1).unwrap()),
            (Rational::new(5, 1).unwrap(), Rational::new(3, 1).unwrap()),
        ])
        .expect("a value store");
        b.store
            .commit(Op::SetLayerRetime {
                comp: Uuid::parse_str(&comp).unwrap(),
                layer: Uuid::parse_str(&layer).unwrap(),
                retime: Some(rt),
            })
            .unwrap();
        // Drag boundary 1 from frame 120 (2 s) to frame 180 (3 s).
        let snap = parse(&drag_boundary(&mut b, &comp, &layer, 1, 180));
        assert_eq!(snap["ok"], json!(true));
        assert_eq!(layer_retime(&snap)["boundaries"][1]["t_frame"], json!(180));
        // Undo restores it.
        let after = parse(&undo(&mut b));
        assert_eq!(layer_retime(&after)["boundaries"][1]["t_frame"], json!(120));
        let _ = snapshot(&b);
    }

    #[test]
    fn retime_ops_reject_a_non_footage_layer() {
        let (mut b, comp, _layer) = bridge_with_footage();
        crate::edits::add_camera_layer(&mut b, &comp);
        let snap = parse(&snapshot(&b));
        let comp_item = snap["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["kind"] == json!("composition"))
            .unwrap();
        let cam = comp_item["comp"]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["kind"] == json!("camera"))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let reply = parse(&set_retime_speed(&mut b, &comp, &cam, 50.0));
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("footage"));
    }
}
