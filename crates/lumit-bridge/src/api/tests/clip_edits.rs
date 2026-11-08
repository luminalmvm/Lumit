//! The Cut edits that move clips somewhere new: nothing is lost on the way, and
//! one undo brings all of it back.

use super::*;
use crate::api::cut::{BridgeCutMove, BridgeCutResult};

/// Clips folded into a composition land there in the places they had, one clip
/// plays it across their span, and one undo puts the row back as it was.
#[test]
fn folded_clips_keep_their_places_and_come_back_whole() {
    let (project, comp, top, length) = cut_row(3);
    let before = row_frames(&top);
    let ids: Vec<Uuid> = row_clips(&top).iter().map(|c| c.id).collect();

    let made = comp
        .cut_clips_to_composition(vec![ids[1], ids[2]], false, "Folded".into())
        .expect("answered")
        .expect("made");

    assert_eq!(
        row_frames(&top),
        [(0, length), (length, 3 * length)],
        "one clip stands where the two did"
    );
    let inner = made.get_layers().expect("layers");
    assert_eq!(inner.len(), 1, "one row came across");
    assert_eq!(
        row_frames(&inner[0]),
        [(0, length), (length, 2 * length)],
        "they start from the composition's start"
    );
    assert_eq!(
        made.get_settings().expect("settings").duration,
        BridgeRational { num: 4, den: 1 },
        "as long as their span"
    );

    project.undo().expect("undone");
    assert_eq!(row_frames(&top), before);
}

/// A freeze cuts the clip at the playhead, holds that frame between the halves,
/// and moves what follows later by the hold, with no source lost either side.
#[test]
fn a_freeze_goes_between_two_halves_and_moves_what_follows() {
    let (project, comp, top, length) = cut_row(2);
    let before = row_frames(&top);
    let ids: Vec<Uuid> = row_clips(&top).iter().map(|c| c.id).collect();
    let (at, hold) = (length / 2, length / 4);
    let whole = row_clips(&top)[0].clone();

    let done = comp.cut_freeze(ids[0], at, hold, true);
    assert_eq!(done.expect("answered"), BridgeCutResult::Done);

    assert_eq!(
        row_frames(&top),
        [
            (0, at),
            (at, at + hold),
            (at + hold, length + hold),
            (length + hold, 2 * length + hold),
        ]
    );
    let clips = row_clips(&top);
    let moment = clips[0].source_out;
    assert_eq!(clips[0].source_in, whole.source_in);
    assert_eq!(clips[2].source_in, moment, "the next half picks up there");
    assert_eq!(clips[2].source_out, whole.source_out);
    assert_eq!(
        (clips[1].source_in, clips[1].source_out),
        (moment, moment),
        "the hold shows that one moment"
    );
    assert_eq!(clips[1].constant_speed(), Some(0.0), "and its line is at 0");

    project.undo().expect("undone");
    assert_eq!(row_frames(&top), before);
}

/// An overlap a drag makes under Auto crossfade takes the kind asked for, and
/// the kind goes when the overlap does, so a later one can't pick it back up.
#[test]
fn an_overlap_takes_the_kind_it_arrives_with_and_loses_it_when_it_goes() {
    let (_project, comp, top, length) = cut_row(2);
    let ids: Vec<Uuid> = row_clips(&top).iter().map(|c| c.id).collect();
    let how = BridgeCutMove {
        overlap: true,
        ripple: false,
        nudge_ticks: 0,
        transition: Some("iris_wipe".into()),
    };
    let moved = comp.cut_move(
        vec![ids[1]],
        ids[1],
        -(length / 4),
        None,
        false,
        how.clone(),
    );
    assert_eq!(moved.expect("answered"), BridgeCutResult::Done);
    let kind = row_clips(&top)[1].transition.clone();
    assert_eq!(kind.expect("a kind").effect.match_name, "iris_wipe");

    let moved = comp.cut_move(vec![ids[1]], ids[1], length / 4, None, false, how);
    assert_eq!(moved.expect("answered"), BridgeCutResult::Done);
    assert_eq!(row_clips(&top)[1].transition, None);
}

/// Asked for the kind it already is, an overlap keeps the rows it was given, and
/// taking the overlap off takes the kind with it.
#[test]
fn a_kind_keeps_its_rows_and_goes_with_its_overlap() {
    let (_project, comp, top, _) = cut_row(2);
    let ids: Vec<Uuid> = row_clips(&top).iter().map(|c| c.id).collect();
    let set = |kind: Option<&str>| {
        comp.cut_set_transition(ids[0], true, 12, kind.map(str::to_string), false)
            .expect("answered")
    };
    assert_eq!(set(Some("linear_wipe")), BridgeCutResult::Done);
    let first = row_clips(&top)[1].transition.clone().expect("a kind");
    assert_eq!(set(Some("linear_wipe")), BridgeCutResult::Done);
    assert_eq!(
        row_clips(&top)[1].transition.as_ref().map(|t| t.id),
        Some(first.id)
    );
    assert_eq!(set(Some("not_a_kind")), BridgeCutResult::Limit);

    assert_eq!(set(None), BridgeCutResult::Done);
    assert_eq!(row_clips(&top)[1].transition, None, "a dissolve again");
    assert_eq!(set(Some("radial_wipe")), BridgeCutResult::Done);
    let off = comp.cut_transition(ids[0], true, 0, false);
    assert_eq!(off.expect("answered"), BridgeCutResult::Done);
    assert_eq!(row_clips(&top)[1].transition, None, "gone with the overlap");
}
