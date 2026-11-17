//! Cutting a multicam clip: its halves cut together and switch as the switch
//! says, a stretch can be painted with an angle, a clip flattens into its
//! angle's footage, and one undo puts each edit back.

use super::*;
use crate::api::project::ProjectReference;
use crate::api::state::LumitBridgeState;

fn commit(project: &ProjectReference, op: Op) {
    let state = project.state().expect("state");
    let state = state.write().expect("write");
    state.store.commit(op).expect("committed");
}

/// The clips on a row, earliest first.
fn clips(row: &LayerReference) -> Vec<Clip> {
    let LayerKind::Sequence { mut clips } = row.item().expect("row").kind else {
        panic!("a Sequence layer");
    };
    clips.sort_by_key(|c| c.place_start);
    clips
}

/// Each clip's angle on a row, earliest first.
fn angles(row: &LayerReference) -> Vec<Option<Uuid>> {
    clips(row).iter().map(|c| c.angle).collect()
}

/// A multicam comp of two camera angles and a recorder, ten seconds each,
/// placed at the top of a fresh comp: the comp, its picture row and its sound
/// row, the multicam comp and the three angles.
fn placed() -> (
    ProjectReference,
    CompositionReference,
    LayerReference,
    LayerReference,
    CompositionReference,
    [Uuid; 3],
) {
    let project = LumitBridgeState::new_project(None).expect("a new project");
    let mc = project
        .new_composition("Multicam".into(), None)
        .expect("comp");
    let mut ids = [Uuid::nil(); 3];
    for (n, name) in ["A.mov", "B.mov", "Recorder.wav"].iter().enumerate() {
        let footage = project
            .import_footage(format!("C:/shoot/{name}"))
            .expect("imported");
        let ten = Rational::new(10, 1).expect("ten");
        let clip = Clip::new(
            ClipSource::Footage(footage.id()),
            Rational::ZERO,
            ten,
            Rational::ZERO,
            ten,
        );
        let mut layer = crate::edits::base_layer(
            (*name).into(),
            LayerKind::Sequence { clips: vec![clip] },
            ten,
            crate::edits::centred_transform(1920.0, 1080.0, 1920, 1080),
        );
        layer.audio_only = n == 2;
        ids[n] = layer.id;
        commit(
            &project,
            Op::AddLayer {
                comp: mc.id(),
                index: n,
                layer: Box::new(layer),
            },
        );
    }
    commit(
        &project,
        Op::SetCompMulticam {
            comp: mc.id(),
            multicam: Some(Box::new(lumit_core::multicam::Multicam::default())),
        },
    );
    let edit = project.new_composition("Edit".into(), None).expect("comp");
    let done = edit.cut_place_comp(None, &mc, 0).expect("answered");
    assert_eq!(done, BridgeCutResult::Done);
    let layers = edit.get_layers().expect("layers");
    assert_eq!(layers.len(), 2, "a picture row and a sound row");
    let (picture, sound) = (layers[0], layers[1]);
    (project, edit, picture, sound, mc, ids)
}

#[test]
fn a_multicam_clip_cuts_and_switches_its_halves() {
    let (project, edit, picture, sound, mc, [a, b, recorder]) = placed();
    let (pic, snd) = (clips(&picture), clips(&sound));
    assert_eq!((pic.len(), snd.len()), (1, 1));
    assert!(
        pic[0].link.is_some() && pic[0].link == snd[0].link,
        "linked"
    );
    assert_eq!((pic[0].angle, snd[0].angle), (Some(a), Some(a)));
    assert!(!pic[0].audible, "its sound is the sound clip's");

    // Picture and sound together, at frame 30.
    let cut = |frame, angle, switch| {
        edit.cut_to_angle(frame, angle, switch, None, true)
            .expect("answered")
    };
    assert_eq!(cut(30, b, BridgeAngleSwitch::Both), BridgeCutResult::Done);
    assert_eq!(angles(&picture), [Some(a), Some(b)]);
    assert_eq!(angles(&sound), [Some(a), Some(b)]);
    // The same angle again cuts nothing.
    assert_eq!(
        cut(40, b, BridgeAngleSwitch::Both),
        BridgeCutResult::Nothing
    );

    // The picture alone: the sound is cut with it and stays where it was.
    assert_eq!(
        cut(60, a, BridgeAngleSwitch::Picture),
        BridgeCutResult::Done
    );
    assert_eq!(angles(&picture), [Some(a), Some(b), Some(a)]);
    assert_eq!(angles(&sound), [Some(a), Some(b), Some(b)]);
    // The pieces are still in step with the comp: each picks up where the
    // one before it stopped.
    let pic = clips(&picture);
    assert_eq!(pic[1].source_in, pic[0].source_out);
    assert_eq!(pic[2].source_in, pic[1].source_out);

    // A sound angle reaches the sound only, whatever the switch says.
    assert_eq!(
        edit.cut_switch_angle(Vec::new(), 10, recorder, BridgeAngleSwitch::Both)
            .expect("answered"),
        BridgeCutResult::Done
    );
    assert_eq!(angles(&picture), [Some(a), Some(b), Some(a)]);
    assert_eq!(angles(&sound), [Some(recorder), Some(b), Some(b)]);

    // Painting angle B over a stretch of the last piece.
    let painted = edit
        .cut_paint_angle(70, 80, b, BridgeAngleSwitch::Picture, true)
        .expect("answered");
    assert_eq!(painted, BridgeCutResult::Done);
    assert_eq!(
        angles(&picture),
        [Some(a), Some(b), Some(a), Some(b), Some(a)]
    );

    // Stepping on from B is A again, round the two picture angles.
    let step = edit
        .cut_step_angle(75, true, BridgeAngleSwitch::Picture)
        .expect("answered");
    assert_eq!(step, BridgeCutResult::Done);
    assert_eq!(angles(&picture)[3], Some(a));

    // One undo puts the last edit back, and the comp is untouched by any.
    project.undo().expect("undone");
    assert_eq!(angles(&picture)[3], Some(b));
    assert!(mc.is_multicam().expect("read"));

    // Joining the first piece takes its cut away, picture and sound alike,
    // and the later piece takes the earlier one's angle.
    let pieces = clips(&picture);
    let joined = edit
        .cut_join(vec![pieces[0].id, pieces[1].id], true)
        .expect("answered");
    assert_eq!(joined, BridgeCutResult::Done);
    assert_eq!(angles(&picture), [Some(a), Some(a), Some(b), Some(a)]);
    assert_eq!(angles(&sound)[0], Some(recorder));
    assert_eq!(clips(&sound).len(), 4);

    // A picture clip that plays its own sound never takes a sound angle.
    let (zero, one) = (
        BridgeRational { num: 0, den: 1 },
        BridgeRational { num: 1, den: 1 },
    );
    let placed = edit
        .cut_place_multicam(None, mc, zero, one, 600, false, true, None, None)
        .expect("answered");
    assert_eq!(placed, BridgeCutResult::Done);
    let heard = edit.get_layers().expect("layers")[0];
    assert!(clips(&heard)[0].audible, "heard");
    assert_eq!(
        edit.cut_to_angle(610, recorder, BridgeAngleSwitch::Sound, Some(heard), true)
            .expect("answered"),
        BridgeCutResult::Nothing
    );
}

#[test]
fn a_flattened_multicam_clip_is_its_angles_footage() {
    let (_project, edit, picture, _sound, mc, [_, b, _]) = placed();
    let id = clips(&picture)[0].id;
    let switched = edit
        .cut_switch_angle(vec![id], 0, b, BridgeAngleSwitch::Picture)
        .expect("answered");
    assert_eq!(switched, BridgeCutResult::Done);
    let whole = clips(&picture)[0].clone();
    let flat = edit.cut_flatten(vec![whole.id], false).expect("answered");
    assert_eq!(flat, BridgeCutResult::Done);
    let after = clips(&picture);
    assert_eq!(after.len(), 1);
    let doc = mc.document().expect("doc");
    let angle = doc
        .comp(mc.id())
        .and_then(|c| c.layers.iter().find(|l| l.id == b).cloned())
        .expect("angle");
    let LayerKind::Sequence { clips: inner } = angle.kind else {
        panic!("an angle is a Sequence layer");
    };
    assert_eq!(after[0].source, inner[0].source, "the angle's own file");
    assert_eq!(after[0].angle, None);
    // The comp runs on past the angle's ten seconds, and that gap in the
    // angle stays a gap.
    assert!(
        inner[0].place_end() < whole.place_end(),
        "the comp outlasts the file"
    );
    assert_eq!(
        (after[0].place_start, after[0].place_end()),
        (whole.place_start, inner[0].place_end()),
        "where the clip was, as far as the angle's file runs"
    );
}
