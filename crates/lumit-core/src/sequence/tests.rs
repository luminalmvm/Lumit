use super::transition::dissolve;
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
    let base = clip(Uuid::now_v7(), 0, 4);
    let ramp = base.with_ramp(rat(1, 1), rat(3, 1));
    assert_eq!(ramp.place_duration, base.place_duration);
    assert_eq!(ramp.source_out, rat(8, 1));
    let (v0, v1) = ramp.ramp_view().unwrap();
    assert!((v0 - 1.0).abs() < 1e-9 && (v1 - 3.0).abs() < 1e-9);
    assert_eq!(ramp.constant_speed(), None);
    assert!((ramp.source_time(0.0) - 0.0).abs() < 1e-9);
}

#[test]
fn resolution_picks_the_clip_under_the_playhead() {
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    let clips = vec![clip(a, 0, 2), clip(b, 3, 2)];
    assert_eq!(resolve(&clips, 1.0).unwrap().1, ClipSource::Footage(a));
    assert_eq!(resolve(&clips, 4.0).unwrap().1, ClipSource::Footage(b));
    assert!(resolve(&clips, 2.5).is_none());
    assert!(resolve(&clips, 5.0).is_none());
    assert!(resolve(&clips, 0.0).is_some());
    assert!(resolve(&clips, 2.0).is_none());
    assert!(resolve(&clips, 3.0).is_some());
}

#[test]
fn a_picture_row_dissolves_across_an_overlap_and_fades_at_a_lone_end() {
    let linear = |seconds: i64| Fade {
        seconds: rat(seconds, 1),
        shape: FadeShape::Linear,
    };
    let mut a = clip(Uuid::now_v7(), 0, 4);
    let mut b = clip(Uuid::now_v7(), 2, 4);
    let mut c = clip(Uuid::now_v7(), 6, 2);
    (a.fade_in, b.fade_out, c.fade_in) = (linear(1), linear(1), linear(1));
    (a.fade_out, b.fade_in) = (linear(3), linear(3));
    let clips = vec![b.clone(), c.clone(), a.clone()];
    let one = |clip: &Clip, gain: f64| (None, Some(clip.id), gain);
    let shown = |lt: f64| match shown_at(&clips, lt) {
        Shown::Nothing => (None, None, 0.0),
        Shown::One { clip, gain } => one(clip, gain),
        Shown::Two {
            outgoing,
            incoming,
            gain,
            ..
        } => (Some(outgoing.id), Some(incoming.id), gain),
    };
    assert_eq!(shown(-1.0), (None, None, 0.0));
    assert_eq!(shown(8.0), (None, None, 0.0));
    assert_eq!(shown(0.0), one(&a, 0.0));
    assert_eq!(shown(0.5), one(&a, 0.5));
    assert_eq!(shown(1.5), one(&a, 1.0));
    assert_eq!(shown(2.0), (Some(a.id), Some(b.id), 0.0));
    assert_eq!(shown(3.0), (Some(a.id), Some(b.id), 0.5));
    assert_eq!(shown(4.0), one(&b, 1.0));
    assert_eq!(shown(5.5), one(&b, 0.5));
    assert_eq!(shown(6.5), one(&c, 0.5));
}

#[test]
fn resolve_holds_at_the_trimmed_end_on_overrun() {
    let src = Uuid::now_v7();
    let clips = [Clip::new(
        ClipSource::Footage(src),
        rat(10, 1),
        rat(12, 1),
        rat(2, 1),
        rat(4, 1),
    )];
    let st = |lt: f64| resolve(&clips, lt).unwrap().2;
    assert!((st(2.0) - 10.0).abs() < 1e-9);
    assert!((st(4.0) - 12.0).abs() < 1e-9);
    assert!((st(5.0) - 12.0).abs() < 1e-9);
    assert!((st(5.9) - 12.0).abs() < 1e-9);
    assert!((clips[0].source_time(5.0) - 13.0).abs() < 1e-9);
}

#[test]
fn sliding_moves_the_clip_but_not_its_content() {
    let src = Uuid::now_v7();
    let c = clip(src, 2, 4);
    let s = c.slide(rat(3, 1)).unwrap();
    assert_eq!(s.place_start, rat(5, 1));
    assert_eq!(s.place_duration, c.place_duration);
    assert_eq!(s.source_in, c.source_in);
    assert_eq!(s.source_out, c.source_out);
    assert!((s.source_time(5.0) - c.source_time(2.0)).abs() < 1e-9);
    assert!((s.source_time(7.0) - c.source_time(4.0)).abs() < 1e-9);
    assert!(c.slide(rat(-3, 1)).is_none());
}

#[test]
fn trimming_an_edge_inward_keeps_the_rest_in_place() {
    let src = Uuid::now_v7();
    let c = clip(src, 2, 4);
    let t = c.trim_end(rat(5, 1)).unwrap();
    assert_eq!(t.id, c.id);
    assert_eq!(t.place_start, rat(2, 1));
    assert_eq!(t.place_duration, rat(3, 1));
    assert_eq!(t.source_out, rat(3, 1));
    for &lt in &[2.0, 3.5, 4.9] {
        assert!(
            (t.source_time(lt) - c.source_time(lt)).abs() < 1e-9,
            "tail @ {lt}"
        );
    }
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
    assert!(c.trim_end(rat(7, 1)).is_none());
    assert!(c.trim_start(rat(1, 1)).is_none());
    assert!(c.trim_end(rat(2, 1)).is_none());
}

#[test]
fn dropping_a_clip_overwrites_what_is_under_it() {
    let src = Uuid::now_v7();
    let a = clip(src, 0, 4);
    let b = clip(src, 4, 4);
    let c = clip(src, 8, 4);

    let mut over = clip(src, 4, 4);
    over.id = Uuid::now_v7();
    let out = overwrite_with(&[a.clone(), b.clone(), c.clone(), over.clone()], over.id);
    assert_eq!(out.len(), 3, "the buried clip went");
    assert!(!out.iter().any(|k| k.id == b.id));
    assert!(out.iter().any(|k| k.id == a.id) && out.iter().any(|k| k.id == c.id));

    let mut across = clip(src, 2, 4);
    across.id = Uuid::now_v7();
    let out = overwrite_with(&[a.clone(), b.clone(), across.clone()], across.id);
    let left = out
        .iter()
        .find(|k| k.id == a.id)
        .expect("the first survives");
    let right = out.iter().find(|k| k.id == b.id).expect("the second too");
    assert_eq!(left.place_end(), rat(2, 1), "trimmed back to the drop");
    assert_eq!(right.place_start, rat(6, 1), "and forward from its end");

    let mut inside = clip(src, 5, 2);
    inside.id = Uuid::now_v7();
    let out = overwrite_with(&[b.clone(), inside.clone()], inside.id);
    assert_eq!(out.len(), 3, "the clip under it became two");
    let pieces: Vec<_> = out.iter().filter(|k| k.id != inside.id).collect();
    assert_eq!(pieces[0].place_start, rat(4, 1));
    assert_eq!(pieces[0].place_end(), rat(5, 1));
    assert_eq!(pieces[1].place_start, rat(7, 1));
    assert_eq!(pieces[1].place_end(), rat(8, 1));
    assert_ne!(pieces[0].id, pieces[1].id, "the halves are distinct clips");

    let mut clear = clip(src, 20, 2);
    clear.id = Uuid::now_v7();
    let all = vec![a.clone(), b.clone(), c.clone(), clear.clone()];
    assert_eq!(overwrite_with(&all, clear.id).len(), 4);
}

#[test]
fn a_ripple_shifts_what_follows_and_refuses_to_overlap() {
    let src = Uuid::now_v7();
    let clips = vec![clip(src, 0, 4), clip(src, 4, 4), clip(src, 10, 2)];
    let starts = |c: &[Clip]| c.iter().map(|c| c.place_start).collect::<Vec<_>>();

    let later = shift_from(&clips, rat(4, 1), rat(2, 1)).unwrap();
    assert_eq!(starts(&later), vec![rat(0, 1), rat(6, 1), rat(12, 1)]);
    assert_eq!(later[1].source_in, clips[1].source_in, "the same frames");
    let straddled = shift_from(&clips, rat(5, 1), rat(2, 1)).unwrap();
    assert_eq!(starts(&straddled), vec![rat(0, 1), rat(4, 1), rat(12, 1)]);

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
            cursor = (cursor + next(6) - 1).max(0);
            let len = 1 + next(5);
            clips.push(clip(src, cursor, len));
            cursor += len;
        }
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

#[test]
fn rolling_an_edit_point_moves_only_the_join() {
    let src = Uuid::now_v7();
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

    let five = dissolve(&clips, a.id, b.id, 5, frame, room).unwrap();
    assert_eq!(five[0].place_end(), rat(42, 10));
    assert_eq!(five[1].place_start, rat(37, 10));
    assert_eq!(five[0].source_out, rat(52, 10));
    assert_eq!(five[1].source_in, rat(17, 10));
    assert_eq!((five[0].id, five[1].id), (a.id, b.id));
    assert_eq!(five[1].source_at(rat(4, 1)), b.source_at(rat(4, 1)));

    let off = dissolve(&five, a.id, b.id, 0, frame, room).unwrap();
    assert_eq!(off, clips);

    let held = dissolve(&clips, a.id, b.id, 40, frame, room).unwrap();
    assert_eq!(held[0].place_end(), rat(5, 1));
    assert_eq!(held[0].source_out, rat(6, 1), "to the end of its media");
    assert_eq!(held[1].place_start, rat(2, 1));

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
    let back = trimmed.with_ramp(rat(-1, 1), rat(-1, 1));
    assert_eq!(back.source_at(rat(3, 1)), Some(rat(9, 1)), "backwards");
    assert!((back.held_source_time(3.0) - 9.0).abs() < 1e-9);
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

    assert!(slide_between(&clips, clips[1].id, rat(4, 1)).is_none());
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
    let c = clip(src, 2, 4);
    let (l, r) = c.cut(rat(4, 1)).unwrap();
    assert_eq!(l.place_start, rat(2, 1));
    assert_eq!(l.place_duration, rat(2, 1));
    assert_eq!(r.place_start, rat(4, 1));
    assert_eq!(r.place_duration, rat(2, 1));
    assert_eq!(l.source_out, rat(2, 1));
    assert_eq!(r.source_in, rat(2, 1));
    assert!((l.source_time(3.0) - c.source_time(3.0)).abs() < 1e-9);
    assert!((r.source_time(5.0) - c.source_time(5.0)).abs() < 1e-9);
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
    let (_left, right) = clip(src, 0, 4).cut(rat(2, 1)).unwrap();
    let start_frame = right.source_in;
    assert!((right.source_time(2.0) - start_frame.to_f64()).abs() < 1e-9);

    let respeed = right.with_ramp(rat(2, 1), rat(1, 1));
    assert!((respeed.source_time(2.0) - start_frame.to_f64()).abs() < 1e-9);
    assert!(respeed.source_time(3.0) > right.source_time(3.0));
    let mut moved = respeed.clone();
    moved.place_start = rat(5, 1);
    assert!((moved.source_time(5.0) - start_frame.to_f64()).abs() < 1e-9);
}

/// One effect instance for a clip's own rack. Any effect will do: nothing
/// here opens it, and the rules under test are about identity.
fn instance() -> EffectInstance {
    crate::fx::builtins::instantiate("blur").expect("a blur exists")
}

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
        assert!(shape.gain(-5.0).abs() < 1e-9);
        assert!((shape.gain(5.0) - 1.0).abs() < 1e-9);
    }
    for n in 0..=20 {
        let u = f64::from(n) / 20.0;
        let there_and_back = FadeShape::Smooth.gain(FadeShape::Sharp.gain(u));
        assert!(
            (there_and_back - u).abs() < 1e-9,
            "Sharp is not the inverse of Smooth at {u}"
        );
    }
}

#[test]
fn a_custom_shape_is_read_at_x_and_keeps_its_handles_in_the_box() {
    let straight = FadeShape::custom(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0);
    for n in 0..=10 {
        let u = f64::from(n) / 10.0;
        assert!(
            (straight.gain(u) - FadeShape::Linear.gain(u)).abs() < 1e-6,
            "a straight bezier is Linear at {u}"
        );
    }
    assert_eq!(
        FadeShape::custom(-9.0, -9.0, 9.0, 9.0),
        FadeShape::Custom {
            x1: MIN_REACH,
            y1: -HANDLE_REACH,
            x2: 1.0 - MIN_REACH,
            y2: 1.0 + HANDLE_REACH,
        }
    );
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

#[test]
fn fast_against_fast_holds_the_power_and_linear_the_amplitude() {
    for n in 0..=20 {
        let u = f64::from(n) / 20.0;
        let (a, b) = (FadeShape::Fast.gain(u), FadeShape::Fast.gain(1.0 - u));
        assert!((a * a + b * b - 1.0).abs() < 1e-9, "u={u}: not equal power");
        let (a, b) = (FadeShape::Linear.gain(u), FadeShape::Linear.gain(1.0 - u));
        assert!((a + b - 1.0).abs() < 1e-9, "u={u}: not equal amplitude");
    }
}

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

    let trimmed = c.trim_end(rat(3, 1)).expect("a trim inside the clip");
    assert_eq!(trimmed.fade_in, c.fade_in);
    assert_eq!(trimmed.fade_out, c.fade_out);
    assert_eq!(trimmed.effects[0].id, c.effects[0].id);
}

#[test]
fn the_new_fields_are_absent_until_set_and_round_trip_when_they_are() {
    let bare = clip(Uuid::now_v7(), 1, 4);
    let before = serde_json::to_string(&bare).unwrap();
    let keys = ["fade_in", "fade_out", "effects", "fx", "gain_db", "link"];
    for key in keys.into_iter().chain(["opacity", "takes"]) {
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
    set.opacity = 0.5;
    set = set
        .with_take(ClipSource::Comp(Uuid::now_v7()), rat(0, 1))
        .unwrap();
    let text = serde_json::to_string(&set).unwrap();
    assert_eq!(serde_json::from_str::<Clip>(&text).unwrap(), set);
}

#[test]
fn a_peak_row_is_nothing_at_both_ends_and_stored_in_the_middle() {
    use super::transition::{fresh, keyed, swept, Side};
    let blur = fresh("blur", None, 1920.0, 1080.0).unwrap();
    let row = |e: &crate::model::EffectInstance, id: &str| match e.param(id) {
        Some(crate::model::EffectValue::Float(p)) => p.value_at(0.0),
        _ => f64::NAN,
    };
    let stored = row(&blur, "radius");
    assert!(stored > 0.0);
    for (u, radius, mix) in [(0.0, 0.0, 0.0), (0.5, stored, 100.0), (1.0, 0.0, 0.0)] {
        for side in [Side::Out, Side::In] {
            let at = swept(&blur, u, side, 1920.0, 1080.0, 0.0);
            assert!((row(&at, "radius") - radius).abs() < 1e-6, "radius at {u}");
            assert!((row(&at, "mix") - mix).abs() < 1e-6, "mix at {u}");
        }
    }

    let span = (rat(1, 1), rat(2, 1));
    let value = |e: &crate::model::EffectInstance, id: &str, t: f64| match e.param(id) {
        Some(crate::model::EffectValue::Float(p)) => p.value_at(t),
        _ => f64::NAN,
    };
    let frame = rat(1, 20);
    let hit = keyed(&blur, Side::Out, span, frame, 1920.0, 1080.0);
    for (t, radius, mix) in [(1.0, 0.0, 0.0), (1.5, stored, 100.0), (2.0, 0.0, 0.0)] {
        assert!(
            (value(&hit, "radius", t) - radius).abs() < 1e-6,
            "keyed radius at {t}"
        );
        assert!(
            (value(&hit, "mix", t) - mix).abs() < 1e-6,
            "keyed mix at {t}"
        );
    }
    let push = keyed(
        &fresh("move", None, 1920.0, 1080.0).unwrap(),
        Side::In,
        span,
        frame,
        1920.0,
        1080.0,
    );
    for (t, completion) in [(1.0, 0.0), (1.5, 50.0), (2.0, 100.0)] {
        assert!(
            (value(&push, "completion", t) - completion).abs() < 1e-6,
            "completion at {t}"
        );
    }
    assert_eq!(
        push.param("side"),
        Some(&crate::model::EffectValue::Choice(1))
    );
    assert_eq!(
        push.param("fade"),
        Some(&crate::model::EffectValue::Bool(false))
    );

    let mosaic = fresh("mosaic", None, 1920.0, 1080.0).unwrap();
    let pixelate = keyed(&mosaic, Side::In, span, frame, 1920.0, 1080.0);
    let Some(crate::model::EffectValue::Float(blocks)) = pixelate.param("horizontal_blocks") else {
        panic!("no block count");
    };
    let crate::anim::Animation::Keyframed(keys) = &blocks.animation else {
        panic!("block count not keyed");
    };
    let steps = [0, 1, 2, 3, 4, 5, 6, 8, 10, 12, 14, 15, 16, 17, 18, 19, 20];
    assert_eq!(keys.len(), steps.len());
    for (k, step) in keys.iter().zip(steps) {
        let u = f64::from(step) / 20.0;
        let want = row(
            &swept(&mosaic, u, Side::In, 1920.0, 1080.0, 0.0),
            "horizontal_blocks",
        );
        assert_eq!(k.time, rat(20 + i64::from(step), 20), "key time at {u}");
        assert!((k.value - want).abs() < 1e-6, "blocks at {u}");
    }
}
