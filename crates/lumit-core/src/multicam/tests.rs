use super::*;
use crate::edits::{base_layer, centred_transform};
use crate::model::{Composition, Document, Layer, LayerKind, LinearColour, ProjectItem};
use crate::time::{Duration, FrameRate};

fn s(n: i64) -> Rational {
    Rational::new(n, 1).unwrap()
}

fn angle(name: &str, clips: Vec<Clip>, audio_only: bool) -> Layer {
    let mut layer = base_layer(
        name.into(),
        LayerKind::Sequence { clips },
        s(60),
        centred_transform(1920.0, 1080.0, 1920, 1080),
    );
    layer.audio_only = audio_only;
    layer
}

fn comp(layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Multicam".into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(25, 1).unwrap(),
        duration: Duration(s(60)),
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

/// A clip of a multicam comp draws and sounds its own angle and no other,
/// whatever the angles' own switches say inside the comp, and a clip of a
/// deleted angle draws and sounds nothing.
#[test]
fn a_multicam_clip_plays_only_its_angle() {
    let footage = || ClipSource::Footage(Uuid::now_v7());
    let mut a = angle(
        "A",
        vec![Clip::new(footage(), s(0), s(60), s(0), s(60))],
        false,
    );
    let mut b = angle(
        "B",
        vec![Clip::new(footage(), s(0), s(60), s(0), s(60))],
        false,
    );
    let sound = angle(
        "Recorder",
        vec![Clip::new(footage(), s(0), s(60), s(0), s(60))],
        true,
    );
    // Worked on inside the comp: A hidden and B soloed, to look at B.
    a.switches.visible = false;
    b.switches.solo = true;
    let mut mc = comp(vec![a.clone(), b.clone(), sound.clone()]);
    mc.set_multicam(Some(&Multicam::default()));
    let mut doc = Document::new();
    let id = mc.id;
    doc.items.push(ProjectItem::Composition(mc));

    let mut clip = Clip::new(ClipSource::Comp(id), s(0), s(60), s(0), s(60));
    clip.angle = Some(a.id);
    let seen = nested_for(&doc, &clip).unwrap();
    let switches = |l: &Layer| {
        let found = seen.layers.iter().find(|x| x.id == l.id).unwrap();
        (
            found.switches.visible,
            found.switches.audible,
            found.switches.solo,
        )
    };
    assert_eq!(switches(&a), (true, true, false));
    assert_eq!(switches(&b), (false, false, false));
    assert_eq!(switches(&sound), (false, false, false));

    // A sound angle is heard and never drawn.
    clip.angle = Some(sound.id);
    let seen = nested_for(&doc, &clip).unwrap();
    let rec = seen.layers.iter().find(|x| x.id == sound.id).unwrap();
    assert_eq!((rec.switches.visible, rec.switches.audible), (false, true));

    // A deleted angle leaves every angle off.
    clip.angle = Some(Uuid::now_v7());
    let seen = nested_for(&doc, &clip).unwrap();
    assert!(seen
        .layers
        .iter()
        .all(|l| !l.switches.visible && !l.switches.audible));

    // A clip of the comp naming no angle plays the comp as it is.
    clip.angle = None;
    assert!(matches!(nested_for(&doc, &clip), Some(Cow::Borrowed(_))));
}

/// The footage a multicam clip shows follows its angle into the comp: through
/// the clip's trim, the angle row's own start and the angle clip's trim. A gap
/// in the angle shows nothing.
#[test]
fn footage_at_follows_the_angle_through_every_offset() {
    let file = Uuid::now_v7();
    // The camera's file starts 10 s into the multicam comp and is used from
    // 2 s in. The angle row itself starts 1 s late.
    // It carries a grade and fades in over a second.
    let mut inner = Clip::new(ClipSource::Footage(file), s(2), s(30), s(10), s(28));
    inner
        .effects
        .push(crate::fx::builtins::instantiate("blur").unwrap());
    inner.fade_in.seconds = s(1);
    let mut cam = angle("Cam", vec![inner], false);
    cam.start_offset = crate::time::CompTime(s(1));
    let mut mc = comp(vec![cam.clone()]);
    mc.set_multicam(Some(&Multicam::default()));
    let id = mc.id;
    let mut doc = Document::new();
    doc.items.push(ProjectItem::Composition(mc.clone()));

    // A multicam clip at 100 s of its row, trimmed 15 s into the comp.
    let mut clip = Clip::new(ClipSource::Comp(id), s(15), s(25), s(100), s(10));
    clip.angle = Some(cam.id);
    // 3 s into the clip: comp time 18, row time 17, 7 s into the file's clip,
    // which started 2 s into the file.
    let shown = footage_at(&doc, &clip, s(103)).unwrap();
    assert_eq!((shown.item, shown.source_time), (file, s(9)));
    // Before the file starts on its row: a gap.
    let mut early = clip.clone();
    early.source_in = s(0);
    assert!(footage_at(&doc, &early, s(100)).is_none());
    // Its head is still the file's first picture.
    assert_eq!(
        opens_on(&doc, &early).map(|shown| shown.source_time),
        Some(s(2))
    );

    // The parent comp: the picture clip on a picture row and its sound on a
    // sound row, linked, the sound on another angle.
    let link = Some(Uuid::now_v7());
    let mut picture = clip.clone();
    picture.link = link;
    picture.audible = false;
    let mut heard = clip.duplicate();
    heard.link = link;
    heard.angle = Some(Uuid::now_v7());
    let top = angle("Picture 1", vec![picture.clone()], false);
    let low = angle("Sound 1", vec![heard.clone()], true);
    let parent = comp(vec![top.clone(), low]);
    let found = under(&doc, &parent, s(104)).unwrap();
    assert_eq!(found.clip, picture.id);
    assert_eq!(found.layer, top.id);
    assert_eq!(found.comp, id);
    assert_eq!(found.time, s(19));
    assert_eq!((found.picture, found.sound), (picture.angle, heard.angle));
    assert!(under(&doc, &parent, s(99)).is_none());

    // Flattened from before the file starts, the piece keeps the file's own
    // look: its grade ahead of anything else, and its fade at its own start.
    let mut wide = Clip::new(ClipSource::Comp(id), s(5), s(20), s(100), s(15));
    wide.angle = Some(cam.id);
    let flat = flattened(&doc, &wide).unwrap();
    assert_eq!(flat.len(), 1);
    assert_eq!(flat[0].place_start, s(106));
    assert_eq!(flat[0].effects[0].effect.match_name, "blur");
    assert!(flat[0].effects[0].enabled);
    assert_eq!(flat[0].fade_in.seconds, s(1));

    // The file itself in a cut is in step with the comp, but not once slowed.
    let own = Clip::new(ClipSource::Footage(file), s(5), s(15), s(0), s(10));
    let cut = comp(vec![angle("Picture 1", vec![own.clone()], false)]);
    let found = under(&doc, &cut, s(2)).unwrap();
    assert!(found.plain);
    assert_eq!(found.time, s(16));
    let slowed = own.stretched(s(0), s(20)).unwrap();
    let cut = comp(vec![angle("Picture 1", vec![slowed], false)]);
    assert!(under(&doc, &cut, s(2)).is_none());
}
