//! Tests of the solve link.

use super::*;
use crate::anim::Property;
use crate::model::{
    BlendMode, CameraPose, Composition, LinearColour, ProjectItem, Switches, TransformGroup,
    TransformProp,
};
use crate::ops::apply;

use crate::time::{CompTime, Duration, FrameRate};

/// A written-down solve standing in for a real one: sixty frames at 30 fps,
/// with the pose a plain function of the frame so a test can say what it
/// expects without carrying a table.
///
/// The point of stage 1 is that this is *all* the model needs, and it
/// arrives through the same trait the sidecar will.
struct Synthetic {
    media: Uuid,
}

impl Synthetic {
    const FPS: f64 = 30.0;
    const FIRST: i64 = 0;
    const LAST: i64 = 59;

    fn new(media: Uuid) -> Self {
        Self { media }
    }

    /// The pose at solved frame `n` — distinct in every field per frame, so
    /// a test that lands on the wrong frame cannot accidentally pass.
    fn pose(n: i64) -> CameraPose {
        let f = n as f64;
        CameraPose {
            zoom: 1000.0 + f,
            position: (f, f * 2.0, f * 3.0),
            rotation_deg: (f * 0.5, f * 0.25, f * 0.125),
            dof: None,
        }
    }
}

impl CameraSolveStore for Synthetic {
    fn solved_range(&self, media: Uuid) -> Option<SolvedRange> {
        (media == self.media).then_some(SolvedRange {
            fps: Self::FPS,
            first_frame: Self::FIRST,
            last_frame: Self::LAST,
        })
    }

    fn solved_pose(&self, media: Uuid, frame: i64) -> Option<CameraPose> {
        (media == self.media && (Self::FIRST..=Self::LAST).contains(&frame))
            .then(|| Synthetic::pose(frame))
    }
}

/// A store with nothing in it — never analysed, or the media offline.
struct Empty;

impl CameraSolveStore for Empty {
    fn solved_range(&self, _: Uuid) -> Option<SolvedRange> {
        None
    }
    fn solved_pose(&self, _: Uuid, _: i64) -> Option<CameraPose> {
        None
    }
}

fn secs(n: i64) -> CompTime {
    CompTime(Rational::new(n, 1).unwrap())
}

fn rat(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}

/// A Retime map from `(layer time, source time)` pairs, linear between them.
fn retime(points: &[(i64, f64)]) -> Property {
    Property {
        animation: Animation::Keyframed(
            points
                .iter()
                .map(|(t, v)| super::key(rat(*t, 30), *v))
                .collect(),
        ),
        extra: serde_json::Map::new(),
    }
}

fn layer(name: &str, kind: LayerKind, out_s: i64) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind,
        in_point: secs(0),
        out_point: secs(out_s),
        start_offset: secs(0),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        volume_db: Property::zero(),
        pan: Property::zero(),
        audio_only: false,
        audio_streams: Default::default(),
        adjustment: false,
        pitch_follows_speed: false,
        audio_routes: Default::default(),
        audio_gain_db: 0.0,
        automation: Default::default(),
        duck: None,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        grade_versions: None,
        graph_inputs: None,
        blend: BlendMode::Normal,
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

/// A tracked layer is one carrying the Camera track effect — that is the
/// whole of what marks it.
fn tracked(mut l: Layer) -> Layer {
    l.effects
        .push(crate::fx::builtins::instantiate(CAMERA_TRACK).expect("the effect is registered"));
    l
}

fn camera(link: Option<Uuid>) -> Layer {
    layer(
        "Camera 1",
        LayerKind::Camera {
            zoom: Property::fixed(777.0),
            solve_link: link,
            correction_base: None,
            options: Default::default(),
        },
        4,
    )
}

fn comp(name: &str, layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: name.into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(rat(4, 1)),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

fn document(comps: Vec<Composition>) -> Document {
    let mut doc = Document::new();
    for c in comps {
        doc.items.push(ProjectItem::Composition(c));
    }
    doc
}

/// A retimed clip, freeze included. The mapping is the layer's own Retime —
/// the same one the renderer decodes through — so the camera sees exactly
/// the frame the picture shows.
#[test]
fn a_link_through_a_retimed_clip_follows_the_retime() {
    let media = Uuid::now_v7();
    let mut footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
    // Half speed for the first second, then frozen on source 0.5 s.
    footage.retime = Some(retime(&[(0, 0.0), (30, 0.5), (60, 0.5)]));
    let c = comp("main", vec![camera(Some(footage.id)), footage]);
    let doc = document(vec![c.clone()]);
    let store = Synthetic::new(media);

    // Half speed: comp frame 10 is 1/3 s, source 1/6 s, solved frame 5.
    let got = camera_pose_at(&Arc::new(doc.clone()), &c, 10.0 / 30.0, &store).unwrap();
    assert_eq!(got.state, LinkState::Derived);
    assert_eq!(got.pose, Synthetic::pose(5));

    // The freeze: every moment of the second half reads source 0.5 s,
    // solved frame 15 — and it is a *derived* read, not a held one. The
    // solve has that frame; the retime simply keeps asking for it.
    for n in [31, 40, 59] {
        let got = camera_pose_at(&Arc::new(doc.clone()), &c, f64::from(n) / 30.0, &store).unwrap();
        assert_eq!(got.state, LinkState::Derived, "frame {n}");
        assert_eq!(got.pose, Synthetic::pose(15), "frame {n}");
    }
}

/// The parent-comp rule: a linked camera in the parent comp names the
/// **precomp layer**, and the chain resolves through it to the tracked layer
/// inside — the precomp layer's own Retime included.
#[test]
fn a_link_resolves_through_a_precomp_to_the_tracked_layer_inside() {
    let media = Uuid::now_v7();
    let inner = comp(
        "inner",
        vec![tracked(layer(
            "shot",
            LayerKind::Footage { item: media },
            2,
        ))],
    );

    let mut nested = layer("inner", LayerKind::Precomp { comp: inner.id }, 2);
    // The precomp layer plays at half speed, so comp frame 20 shows the
    // nested comp's 1/3 s, which is the tracked layer's solved frame 10.
    nested.retime = Some(retime(&[(0, 0.0), (60, 1.0)]));
    let outer = comp("outer", vec![camera(Some(nested.id)), nested]);
    let doc = document(vec![inner, outer.clone()]);
    let store = Synthetic::new(media);

    let got = camera_pose_at(&Arc::new(doc.clone()), &outer, 20.0 / 30.0, &store).unwrap();
    assert_eq!(got.state, LinkState::Derived);
    assert_eq!(got.pose, Synthetic::pose(10));

    // A precomp with no tracked layer in it has nothing to resolve to, and
    // says so rather than guessing at the first footage it finds.
    let bare = comp(
        "bare",
        vec![layer("plain", LayerKind::Footage { item: media }, 2)],
    );
    let lone = layer("bare", LayerKind::Precomp { comp: bare.id }, 2);
    let outer = comp("outer", vec![camera(Some(lone.id)), lone]);
    let doc = document(vec![bare, outer.clone()]);
    assert_eq!(
        camera_pose_at(&Arc::new(doc.clone()), &outer, 0.0, &store)
            .unwrap()
            .state,
        LinkState::Unresolved
    );
}

/// The two honest failures, and neither is silent: past the end of the solve
/// the last derived motion **holds**, and a link that cannot be followed at
/// all falls back to the camera's own properties and says which it is.
#[test]
fn an_unresolvable_link_holds_and_says_which_kind_of_unresolvable() {
    let media = Uuid::now_v7();
    let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 4));
    let footage_id = footage.id;
    let c = comp("main", vec![camera(Some(footage_id)), footage]);
    let doc = document(vec![c.clone()]);

    // Three seconds in is past the sixty solved frames: the last one holds.
    let got = camera_pose_at(&Arc::new(doc.clone()), &c, 3.0, &Synthetic::new(media)).unwrap();
    assert_eq!(got.state, LinkState::Held);
    assert_eq!(got.pose, Synthetic::pose(59));

    // Nothing solved at all: the document's own numbers, flagged.
    let got = camera_pose_at(&Arc::new(doc.clone()), &c, 0.0, &Empty).unwrap();
    assert_eq!(got.state, LinkState::Unresolved);
    assert_eq!(got.pose.zoom, 777.0);

    // The linked layer deleted: the same answer, for the same reason.
    let mut orphan = c.clone();
    orphan.layers.retain(|l| l.id != footage_id);
    let doc = document(vec![orphan.clone()]);
    assert_eq!(
        camera_pose_at(&Arc::new(doc.clone()), &orphan, 0.0, &Synthetic::new(media))
            .unwrap()
            .state,
        LinkState::Unresolved
    );

    // A camera with no link is none of those.
    let plain = comp("plain", vec![camera(None)]);
    assert_eq!(
        camera_pose_at(
            &Arc::new(document(vec![plain.clone()])),
            &plain,
            0.0,
            &Empty
        )
        .unwrap()
        .state,
        LinkState::Unlinked
    );
}

/// **Track once, then nudge**: a linked camera takes a transform
/// edit, and what it takes is a *correction* — added to the solved pose,
/// channel by channel, wherever the link resolves.
#[test]
fn a_correction_rides_on_top_of_the_solve() {
    let media = Uuid::now_v7();
    let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 4));
    let c = comp("main", vec![camera(Some(footage.id)), footage]);
    let (comp_id, cam_id) = (c.id, c.layers[0].id);
    let mut doc = document(vec![c]);
    let store = Synthetic::new(media);

    // The camera was built already linked, so the base it would have been
    // given by `SetCameraSolveLink` is written here the same way the bridge
    // writes it for a camera that arrives linked.
    set_base(&mut doc, comp_id, cam_id);
    let base = base_of(&doc, comp_id, cam_id);
    assert!(
        !has_correction(layer_of(&doc, comp_id, cam_id)),
        "a camera nobody has touched carries no correction"
    );

    // Uncorrected, the derived pose is the solve exactly.
    let at = |doc: &Document, n: i64| {
        let c = doc.comp(comp_id).unwrap();
        camera_pose_at(&Arc::new(doc.clone()), c, n as f64 / 30.0, &store).unwrap()
    };
    assert_eq!(at(&doc, 10).pose, Synthetic::pose(10));

    // Two nudges, in the two shapes an edit arrives in: a plain drag, and a
    // keyed one. The zoom is corrected too, because a solved focal is as
    // capable of being a little wrong as a solved position.
    for (prop, value) in [
        (TransformProp::PositionX, base.position.0 + 40.0),
        (TransformProp::RotationY, base.rotation_deg.1 - 2.5),
    ] {
        apply(
            &mut doc,
            &Op::SetTransformProperty {
                comp: comp_id,
                layer: cam_id,
                prop,
                animation: Animation::Static(value),
            },
        )
        .expect("a linked camera takes a transform edit");
    }
    apply(
        &mut doc,
        &Op::SetCameraZoom {
            comp: comp_id,
            layer: cam_id,
            animation: Animation::Static(base.zoom + 7.0),
        },
    )
    .expect("and a zoom edit");
    assert!(has_correction(layer_of(&doc, comp_id, cam_id)));

    // Derived: the solve, plus the difference, and nothing else moved.
    for n in [0, 10, 59] {
        let got = at(&doc, n);
        assert_eq!(got.state, LinkState::Derived, "frame {n}");
        let want = Synthetic::pose(n);
        assert_eq!(
            got.pose,
            CameraPose {
                zoom: want.zoom + 7.0,
                position: (want.position.0 + 40.0, want.position.1, want.position.2),
                rotation_deg: (
                    want.rotation_deg.0,
                    want.rotation_deg.1 - 2.5,
                    want.rotation_deg.2
                ),
                dof: None,
            },
            "frame {n}"
        );
    }

    // Held: past the solved range the correction rides on the held pose,
    // not on nothing — the last derived motion, corrected.
    let held = at(&doc, 90);
    assert_eq!(held.state, LinkState::Held);
    assert_eq!(held.pose.position.0, Synthetic::pose(59).position.0 + 40.0);

    // Unresolved: nothing is being corrected, so the numbers are read as a
    // pose. The camera keeps drawing from where it was put.
    let c = doc.comp(comp_id).unwrap();
    let lost = camera_pose_at(&Arc::new(doc.clone()), c, 0.0, &Empty).unwrap();
    assert_eq!(lost.state, LinkState::Unresolved);
    assert_eq!(lost.pose.position.0, base.position.0 + 40.0);
    assert_eq!(lost.pose.zoom, base.zoom + 7.0);
}

/// **Clear corrections** takes the nudge back and leaves the track alone,
/// in one undo step — and the dot goes out with it.
#[test]
fn clearing_corrections_keeps_the_link_and_undoes_in_one_step() {
    let media = Uuid::now_v7();
    let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
    let footage_id = footage.id;
    let c = comp("main", vec![camera(Some(footage_id)), footage]);
    let (comp_id, cam_id) = (c.id, c.layers[0].id);
    let mut doc = document(vec![c]);
    set_base(&mut doc, comp_id, cam_id);

    assert!(
        clear_corrections(&doc, comp_id, cam_id).is_none(),
        "there is nothing to clear on a camera nobody has nudged"
    );

    apply(
        &mut doc,
        &Op::SetTransformProperty {
            comp: comp_id,
            layer: cam_id,
            prop: TransformProp::PositionZ,
            animation: Animation::Static(120.0),
        },
    )
    .expect("nudged");
    let corrected = layer_of(&doc, comp_id, cam_id).clone();
    assert!(has_correction(&corrected));

    let clear = clear_corrections(&doc, comp_id, cam_id).expect("there is a lane to clear");
    let undo = apply(&mut doc, &clear).expect("clearing applies");
    let after = layer_of(&doc, comp_id, cam_id);
    assert!(!has_correction(after), "the lane is empty again");
    assert!(
        matches!(after.kind, LayerKind::Camera { solve_link: Some(l), .. } if l == footage_id),
        "clearing a correction must not clear the track"
    );

    apply(&mut doc, &undo).expect("the undo applies");
    assert_eq!(
        layer_of(&doc, comp_id, cam_id),
        &corrected,
        "one step puts the nudge back exactly"
    );
}

/// The base is captured when the link is made, kept when it is re-pointed,
/// dropped when it is cleared — and undo puts back the one that was there.
#[test]
fn the_correction_base_follows_the_link() {
    let media = Uuid::now_v7();
    let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
    let other = tracked(layer("shot 2", LayerKind::Footage { item: media }, 2));
    let (footage_id, other_id) = (footage.id, other.id);
    let mut cam = camera(None);
    cam.transform.position_x = Property::fixed(960.0);
    let c = comp("main", vec![cam, footage, other]);
    let (comp_id, cam_id) = (c.id, c.layers[0].id);
    let mut doc = document(vec![c]);

    assert!(
        base_opt(&doc, comp_id, cam_id).is_none(),
        "no link, no lane"
    );

    let link = |to: Option<Uuid>| Op::SetCameraSolveLink {
        comp: comp_id,
        layer: cam_id,
        solve_link: to,
    };
    let undo = apply(&mut doc, &link(Some(footage_id))).expect("linked");
    let base = base_of(&doc, comp_id, cam_id);
    assert_eq!(base.position.0, 960.0, "the pose it was linked at");
    assert!(!has_correction(layer_of(&doc, comp_id, cam_id)));

    // Nudge, then re-point: the nudge is the user's and rides on.
    apply(
        &mut doc,
        &Op::SetTransformProperty {
            comp: comp_id,
            layer: cam_id,
            prop: TransformProp::PositionX,
            animation: Animation::Static(1000.0),
        },
    )
    .expect("nudged");
    apply(&mut doc, &link(Some(other_id))).expect("re-pointed");
    assert_eq!(
        base_of(&doc, comp_id, cam_id).position.0,
        960.0,
        "re-pointing a link must not swallow the correction into the base"
    );
    assert!(has_correction(layer_of(&doc, comp_id, cam_id)));

    // Clearing the link ends the lane.
    apply(&mut doc, &link(None)).expect("unlinked");
    assert!(base_opt(&doc, comp_id, cam_id).is_none());
    assert!(!has_correction(layer_of(&doc, comp_id, cam_id)));

    // And undoing the very first link, from the document it was made in,
    // leaves no lane behind either.
    let mut fresh = doc.clone();
    apply(&mut fresh, &link(Some(footage_id))).expect("relinked");
    apply(&mut fresh, &undo).expect("undone");
    assert!(base_opt(&fresh, comp_id, cam_id).is_none());
}

fn layer_of(doc: &Document, comp: Uuid, layer: Uuid) -> &Layer {
    doc.comp(comp)
        .unwrap()
        .layers
        .iter()
        .find(|l| l.id == layer)
        .unwrap()
}

fn base_opt(doc: &Document, comp: Uuid, layer: Uuid) -> Option<CameraPose> {
    match &layer_of(doc, comp, layer).kind {
        LayerKind::Camera {
            correction_base, ..
        } => correction_base.as_deref().copied(),
        _ => None,
    }
}

fn base_of(doc: &Document, comp: Uuid, layer: Uuid) -> CameraPose {
    base_opt(doc, comp, layer).expect("a linked camera has a base")
}

/// What `Op::SetCameraSolveLink` does for a camera that is linked by an
/// edit, done by hand for the fixtures that are built linked.
fn set_base(doc: &mut Document, comp: Uuid, layer: Uuid) {
    let l = doc
        .comp_mut(comp)
        .unwrap()
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .unwrap();
    let pose = crate::model::stored_camera_pose_lt(l, 0.0);
    if let LayerKind::Camera {
        correction_base, ..
    } = &mut l.kind
    {
        *correction_base = pose.map(Box::new);
    }
}

/// **Convert to keyframes**: the baked keys reproduce the derived path
/// exactly, at every comp frame of the layer's span, and the link is gone.
/// One undo step puts both back.
#[test]
fn the_bake_reproduces_the_derived_path_and_undo_restores_the_link() {
    let media = Uuid::now_v7();
    let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 4));
    let c = comp("main", vec![camera(Some(footage.id)), footage]);
    let (comp_id, cam_id) = (c.id, c.layers[0].id);
    let mut doc = document(vec![c]);
    set_base(&mut doc, comp_id, cam_id);
    let store = Synthetic::new(media);
    let before = doc.comp(comp_id).unwrap().layers[0].clone();

    // What the link says, frame by frame, before anything is baked. The
    // camera's span is 0..4 s at 30 fps, so 120 frames — sixty derived and
    // sixty held, which means the bake is checked on both readings.
    let want: Vec<CameraPose> = (0..120)
        .map(|n| {
            let c = doc.comp(comp_id).unwrap();
            camera_pose_at(&Arc::new(doc.clone()), c, f64::from(n) / 30.0, &store)
                .unwrap()
                .pose
        })
        .collect();
    // The comparison below is against the derived path itself, so it would
    // pass just as happily on a path that never resolved and never moved.
    // These two say the path under test is the real one: sixty distinct
    // solved poses, then sixty held on the last of them.
    assert_eq!(want[..60], (0..60).map(Synthetic::pose).collect::<Vec<_>>());
    assert!(want[60..].iter().all(|p| *p == Synthetic::pose(59)));

    let bake = bake_solve_link(&Arc::new(doc.clone()), comp_id, cam_id, &store)
        .expect("there is a link to bake");
    assert!(
        matches!(&bake, Op::Batch { ops } if ops.len() == 8),
        "one batch: the link, six transform tracks and the zoom"
    );
    // One undo step, not a hundred and twenty.
    let undo = apply(&mut doc, &bake).expect("the bake applies");

    let baked = doc.comp(comp_id).unwrap();
    assert!(
        matches!(
            baked.layers[0].kind,
            LayerKind::Camera {
                solve_link: None,
                ..
            }
        ),
        "the bake severs the link"
    );
    for (n, expected) in want.iter().enumerate() {
        assert_eq!(
            baked.camera_pose(n as f64 / 30.0).as_ref(),
            Some(expected),
            "baked frame {n} does not match the derived path"
        );
    }
    // Undo restores the link — and restores it *last*, after the transforms
    // are home, so the read-only refusal does not trip on the way out
    // either. One step, and the layer is byte-for-byte what it was.
    apply(&mut doc, &undo).expect("the undo applies");
    assert_eq!(
        doc.comp(comp_id).unwrap().layers[0],
        before,
        "undo puts the link and the properties back exactly"
    );
}

// -----------------------------------------------------------------------
// The planar track's corner pin
// -----------------------------------------------------------------------

/// A written-down planar track: sixty frames at 30 fps, the quad a plain
/// function of the frame so a test can say what it expects without carrying
/// a table, and every corner distinct so landing on the wrong frame or the
/// wrong corner cannot accidentally pass.
struct SyntheticPlane {
    track: Uuid,
}

impl SyntheticPlane {
    const FPS: f64 = 30.0;
    const FIRST: i64 = 0;
    const LAST: i64 = 59;

    fn quad(n: i64) -> Quad {
        let f = n as f64;
        [
            [100.0 + f, 200.0 + f * 2.0],
            [300.0 + f * 3.0, 210.0 + f * 4.0],
            [110.0 + f * 5.0, 400.0 + f * 6.0],
            [320.0 + f * 7.0, 410.0 + f * 8.0],
        ]
    }
}

impl PlanarTrackStore for SyntheticPlane {
    fn planar_range(&self, track: Uuid) -> Option<SolvedRange> {
        (track == self.track).then_some(SolvedRange {
            fps: Self::FPS,
            first_frame: Self::FIRST,
            last_frame: Self::LAST,
        })
    }

    fn planar_corners(&self, track: Uuid, frame: i64) -> Option<Quad> {
        (track == self.track && (Self::FIRST..=Self::LAST).contains(&frame))
            .then(|| SyntheticPlane::quad(frame))
    }
}

/// A layer wearing the Planar track effect — the handle, exactly as the
/// Camera track is one.
fn planar(mut l: Layer) -> Layer {
    l.effects
        .push(crate::fx::builtins::instantiate(PLANAR_TRACK).expect("the effect is registered"));
    l
}

/// The keyframed value of one Corner pin parameter, at a layer time.
fn pin_value(effects: &[crate::model::EffectInstance], id: &str, t: f64) -> f64 {
    let pin = effects
        .iter()
        .rev()
        .find(|e| e.effect.match_name == "corner_pin")
        .expect("a corner pin was added");
    let param = pin
        .params
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("corner pin has no {id}"));
    match &param.value {
        crate::model::EffectValue::Float(p) => p.value_at(t),
        other => panic!("{id} is {other:?}, not a float"),
    }
}

/// The plain case: comp frame `n` is source frame `n`, so the pin's eight
/// numbers are the tracked quad's eight, frame for frame.
#[test]
fn a_corner_pin_from_a_track_lands_the_quad_frame_for_frame() {
    let media = Uuid::now_v7();
    let shot = planar(layer("shot", LayerKind::Footage { item: media }, 2));
    let effect = shot.effects[0].id;
    let target = layer("screen", LayerKind::Null, 2);
    let (tracked_id, target_id) = (shot.id, target.id);
    let c = comp("main", vec![target, shot]);
    let comp_id = c.id;
    let mut doc = document(vec![c]);
    let store = SyntheticPlane { track: effect };

    let op = corner_pin_from_track(&doc, comp_id, tracked_id, effect, target_id, &store)
        .expect("a track with an answer in it writes a pin");
    let inverse = apply(&mut doc, &op).unwrap();

    let effects = &doc
        .comp(comp_id)
        .unwrap()
        .layers
        .iter()
        .find(|l| l.id == target_id)
        .unwrap()
        .effects;
    for n in [0i64, 7, 29, 59] {
        let t = n as f64 / 30.0;
        let want = SyntheticPlane::quad(n);
        for (i, (x, y)) in CORNER_PIN_POINTS.into_iter().enumerate() {
            assert!(
                (pin_value(effects, x, t) - want[i][0]).abs() < 1e-6,
                "corner {i} x at frame {n}"
            );
            assert!(
                (pin_value(effects, y, t) - want[i][1]).abs() < 1e-6,
                "corner {i} y at frame {n}"
            );
        }
    }

    // One undo step, and it takes the whole pin back — the stack is
    // committed whole, as every effect edit is.
    apply(&mut doc, &inverse).unwrap();
    assert!(doc
        .comp(comp_id)
        .unwrap()
        .layers
        .iter()
        .find(|l| l.id == target_id)
        .unwrap()
        .effects
        .is_empty());
}

// -----------------------------------------------------------------------
// The planar track's transform keys
// -----------------------------------------------------------------------

/// A written-down planar track of a **rigid** rectangle, so the pose the
/// bake is supposed to recover is an exact formula rather than a
/// measurement: the quad slides by (2, 3) px a frame, turns 7° a frame and
/// grows 1 % a frame, all about its own centre.
///
/// Seven degrees a frame is deliberate: over sixty frames it passes a whole
/// turn, so a bake that read `atan2` straight would snap back to −53° at the
/// end instead of carrying on to 413°.
struct MovingPlane {
    track: Uuid,
}

impl MovingPlane {
    const FPS: f64 = 30.0;
    const FIRST: i64 = 0;
    const LAST: i64 = 59;
    /// Half the reference rectangle's width and height.
    const HALF: (f64, f64) = (100.0, 50.0);

    fn centre(n: i64) -> [f64; 2] {
        [500.0 + 2.0 * n as f64, 400.0 + 3.0 * n as f64]
    }

    fn degrees(n: i64) -> f64 {
        7.0 * n as f64
    }

    fn scale(n: i64) -> f64 {
        1.0 + n as f64 / 100.0
    }

    fn quad(n: i64) -> Quad {
        let (a, s, c) = (
            Self::degrees(n).to_radians(),
            Self::scale(n),
            Self::centre(n),
        );
        let (sin, cos) = a.sin_cos();
        let (hw, hh) = Self::HALF;
        // Corner pin order: upper left, upper right, lower left, lower
        // right.
        let mut out = [[0.0f64; 2]; 4];
        for (slot, (x, y)) in out
            .iter_mut()
            .zip([(-hw, -hh), (hw, -hh), (-hw, hh), (hw, hh)])
        {
            *slot = [
                c[0] + s * (x * cos - y * sin),
                c[1] + s * (x * sin + y * cos),
            ];
        }
        out
    }
}

impl PlanarTrackStore for MovingPlane {
    fn planar_range(&self, track: Uuid) -> Option<SolvedRange> {
        (track == self.track).then_some(SolvedRange {
            fps: Self::FPS,
            first_frame: Self::FIRST,
            last_frame: Self::LAST,
        })
    }

    fn planar_corners(&self, track: Uuid, frame: i64) -> Option<Quad> {
        (track == self.track && (Self::FIRST..=Self::LAST).contains(&frame))
            .then(|| MovingPlane::quad(frame))
    }
}

/// One transform property of a layer, at a layer time.
fn tr_value(
    doc: &Document,
    comp: Uuid,
    layer: Uuid,
    prop: crate::model::TransformProp,
    t: f64,
) -> f64 {
    layer_of(doc, comp, layer)
        .transform
        .get(prop)
        .unwrap()
        .value_at(t)
}

/// Whether a transform property was left as the constant it was.
fn is_static(doc: &Document, comp: Uuid, layer: Uuid, prop: crate::model::TransformProp) -> bool {
    matches!(
        layer_of(doc, comp, layer)
            .transform
            .get(prop)
            .unwrap()
            .animation,
        Animation::Static(_)
    )
}

/// The whole movement, added to what the layer already held: the quad's
/// slide becomes Position, its turn becomes Rotation past a full circle, and
/// its growth becomes Scale — with the reference frame's keys landing on the
/// layer's own untouched numbers, which is what "added, not stamped" means.
#[test]
fn transform_keys_add_the_tracks_movement_to_what_the_layer_had() {
    use crate::model::TransformProp as P;

    let media = Uuid::now_v7();
    let shot = planar(layer("shot", LayerKind::Footage { item: media }, 2));
    let effect = shot.effects[0].id;
    let target = layer("logo", LayerKind::Null, 2);
    let (tracked_id, target_id) = (shot.id, target.id);
    let c = comp("main", vec![target, shot]);
    let comp_id = c.id;
    let mut doc = document(vec![c]);
    let store = MovingPlane { track: effect };

    let base: Vec<f64> = [
        P::PositionX,
        P::PositionY,
        P::Rotation,
        P::ScaleX,
        P::ScaleY,
    ]
    .into_iter()
    .map(|p| tr_value(&doc, comp_id, target_id, p, 0.0))
    .collect();

    let op = transform_from_track(&doc, comp_id, tracked_id, effect, target_id, true, &store)
        .expect("a track with an answer in it writes transform keys");
    let inverse = apply(&mut doc, &op).unwrap();

    for n in [0i64, 1, 20, 59] {
        let t = n as f64 / 30.0;
        let want = [
            base[0] + 2.0 * n as f64,
            base[1] + 3.0 * n as f64,
            base[2] + MovingPlane::degrees(n),
            base[3] * MovingPlane::scale(n),
            base[4] * MovingPlane::scale(n),
        ];
        for (prop, want) in [
            P::PositionX,
            P::PositionY,
            P::Rotation,
            P::ScaleX,
            P::ScaleY,
        ]
        .into_iter()
        .zip(want)
        {
            let got = tr_value(&doc, comp_id, target_id, prop, t);
            assert!(
                (got - want).abs() < 1e-6,
                "{prop:?} at frame {n}: wanted {want}, got {got}"
            );
        }
    }
    // The claim the unwrap exists for, said on its own: 413°, not −53°.
    let far = tr_value(&doc, comp_id, target_id, P::Rotation, 59.0 / 30.0) - base[2];
    assert!(
        (far - 413.0).abs() < 1e-6,
        "the turn should carry past a full circle, got {far}"
    );

    // One undo step, and every property goes back to the constant it was.
    apply(&mut doc, &inverse).unwrap();
    for prop in [
        P::PositionX,
        P::PositionY,
        P::Rotation,
        P::ScaleX,
        P::ScaleY,
    ] {
        assert!(is_static(&doc, comp_id, target_id, prop), "{prop:?} undone");
    }
}

/// *Position* alone writes two properties and touches nothing else. The
/// point of the row: a small, plain patch gives a position worth having long
/// before it gives a rotation worth having, and a rotation of nought written
/// over one the user animated would be the button deleting their work.
#[test]
fn position_alone_leaves_rotation_and_scale_as_they_were() {
    use crate::model::TransformProp as P;

    let media = Uuid::now_v7();
    let shot = planar(layer("shot", LayerKind::Footage { item: media }, 2));
    let effect = shot.effects[0].id;
    let target = layer("logo", LayerKind::Null, 2);
    let (tracked_id, target_id) = (shot.id, target.id);
    let c = comp("main", vec![target, shot]);
    let comp_id = c.id;
    let mut doc = document(vec![c]);
    let store = MovingPlane { track: effect };

    let base_x = tr_value(&doc, comp_id, target_id, P::PositionX, 0.0);
    let op = transform_from_track(&doc, comp_id, tracked_id, effect, target_id, false, &store)
        .expect("position alone is still a track worth writing");
    apply(&mut doc, &op).unwrap();

    let moved = tr_value(&doc, comp_id, target_id, P::PositionX, 20.0 / 30.0);
    assert!(
        (moved - (base_x + 40.0)).abs() < 1e-6,
        "position still follows the track, got {moved}"
    );
    for prop in [P::Rotation, P::ScaleX, P::ScaleY] {
        assert!(
            is_static(&doc, comp_id, target_id, prop),
            "{prop:?} should not have been written"
        );
    }
}
