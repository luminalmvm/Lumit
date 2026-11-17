//! Photoshop and Illustrator documents imported as comps of their layers.

use super::*;

// A Photoshop document comes in as a comp of its layers and a folder of layer
// items, in one undo step, and anything else is left to the footage import.
#[cfg(feature = "media")]
#[test]
fn a_photoshop_document_imports_as_a_comp_of_its_layers_in_one_undo_step() {
    use lumit_core::model::{BlendMode, EffectValue, LayerKind};
    use lumit_media::psd::fixture::{described, document, vector_mask, Layer, Value};

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("poster.psd");
    let mut hat = Layer::solid("Hat", [8, 8, 16, 16], [0, 0, 255, 255]);
    hat.opacity = 128;
    hat.blend = *b"mul ";
    // At half Fill as well, behind a vector mask over the document's left
    // half that the file has not drawn into the layer.
    let half = [128, 0, 0, 0];
    let left = [(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (0.0, 1.0)];
    hat.blocks = vec![(*b"iOpa", half.to_vec()), vector_mask(&left)];
    let mut props = Layer::group("Props", true);
    props.visible = false;
    // A drop shadow lit from 120 degrees, which is Photoshop's own default.
    let mut title = Layer::solid("Title", [4, 4, 12, 28], [255, 255, 255, 255]);
    let black = ["Rd  ", "Grn ", "Bl  "].map(|key| (key, Value::Number(0.0)));
    let shadow = Value::Object(vec![
        ("enab", Value::Switch(true)),
        ("Clr ", Value::Object(black.to_vec())),
        ("Opct", Value::Number(40.0)),
        ("lagl", Value::Number(120.0)),
    ]);
    title.blocks = vec![
        described(*b"lfx2", &[("DrSh", shadow)]),
        (*b"iOpa", half.to_vec()),
    ];
    // Threshold at 128 of 255, with a mask over the left half.
    let mut threshold = Layer::solid("Threshold", [0, 0, 0, 0], [0; 4]);
    threshold.blocks = vec![(*b"thrs", vec![0, 128, 0, 0])];
    threshold.mask = Some(([0, 0, 32, 16], 0, vec![255; 32 * 16]));
    // Bottom first, as the file lists them. "Empty" has no picture, "Sky" is
    // a fill layer, and "Brim" is a group inside "Props".
    let layers = [
        Layer::solid("Background", [0, 0, 32, 32], [255, 0, 0, 255]),
        Layer::group("</Layer group>", false),
        hat,
        Layer::group("</Layer group>", false),
        Layer::solid("Feather", [0, 0, 4, 4], [255, 255, 0, 255]),
        Layer::group("Brim", true),
        Layer::solid("Scarf", [0, 0, 8, 8], [0, 255, 0, 255]),
        props,
        Layer::solid("Empty", [0, 0, 0, 0], [0; 4]),
        Layer::fill("Sky", [0.0, 0.0, 255.0]),
        threshold,
        title,
    ];
    std::fs::write(&path, document(32, 32, 8, &layers)).expect("the fixture writes");

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let snapshot = || {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        state.store.snapshot()
    };
    let before = snapshot().items.len();

    let left_out = project
        .import_layers(path.to_string_lossy().into_owned())
        .expect("the document imports");
    assert_eq!(left_out, Some(1), "the layer with no picture is counted");

    let doc = snapshot();
    let comp = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.name == "poster" => Some(c),
            _ => None,
        })
        .expect("a comp named for the file");
    assert_eq!((comp.width, comp.height), (32, 32));
    let names: Vec<&str> = comp.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Title",
            "Threshold",
            "Threshold",
            "Sky",
            "Scarf",
            "Brim",
            "Hat",
            "Background"
        ],
        "top first"
    );

    // Each layer reads its own record of the file, by its place in the list.
    let picks: Vec<Option<u32>> = comp
        .layers
        .iter()
        .map(|l| match &l.kind {
            LayerKind::Footage { item } => match doc.item(*item) {
                Some(ProjectItem::Footage(f)) => f.source_layer,
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        picks,
        [
            Some(11),
            None,
            Some(10),
            None,
            Some(6),
            None,
            Some(2),
            Some(0)
        ]
    );

    // An adjustment layer is an Adjustment layer carrying the nearest effect.
    // Its mask is the record read as a picture, on a hidden layer under it
    // that it takes as its matte.
    let (threshold, mask) = (&comp.layers[1], &comp.layers[2]);
    assert_eq!(threshold.kind, LayerKind::Adjustment);
    assert!(!threshold.switches.visible, "it arrives switched off");
    assert_eq!(threshold.matte.map(|m| m.layer), Some(mask.id));
    assert!(!mask.switches.visible);
    let [effect] = threshold.effects.as_slice() else {
        panic!("the adjustment carries one effect");
    };
    assert_eq!(effect.effect.match_name, "threshold");
    let level = effect.params.iter().find(|p| p.id == "level");
    let Some(EffectValue::Float(level)) = level.map(|p| &p.value) else {
        panic!("Threshold has a level");
    };
    assert!((level.value_at(0.0) - 50.2).abs() < 0.1, "128 of 255");

    let hat = &comp.layers[6];
    assert_eq!(hat.blend, BlendMode::Multiply);
    // Fill is one more opacity on a layer with no style: 128 of 255, twice.
    let opacity = hat.transform.opacity.value_at(0.0);
    assert!((opacity - 25.2).abs() < 0.1, "a quarter is {opacity}");
    // Its vector mask is a mask on the layer, in the document's pixels.
    let [mask] = hat.masks.as_slice() else {
        panic!("the vector mask comes across");
    };
    let corners: Vec<(f64, f64)> = mask.path.vertices.iter().map(|v| v.pos).collect();
    assert_eq!(
        corners,
        [(0.0, 0.0), (16.0, 0.0), (16.0, 32.0), (0.0, 32.0)]
    );
    assert!(mask.path.closed && !mask.inverted);
    assert!(!hat.switches.visible, "a hidden group hides its members");
    assert!(comp.layers[0].switches.visible);

    assert_eq!(comp.groups.len(), 1);
    assert_eq!(comp.groups[0].name, "Props");
    let members: Vec<_> = comp.layers[4..7].iter().map(|l| l.id).collect();
    assert_eq!(comp.groups[0].members, members);

    // A group inside a group is a composition of its own, placed in the outer
    // group as a Precomp layer that lets its layers blend through.
    let brim = &comp.layers[5];
    let LayerKind::Precomp { comp: inner } = brim.kind else {
        panic!("the inner group is a Precomp layer");
    };
    assert!(brim.switches.collapse);
    let inner = doc.comp(inner).expect("the inner group's composition");
    let names: Vec<&str> = inner.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Feather"]);

    // A fill layer is a Solid layer of its colour.
    let LayerKind::Solid { def } = comp.layers[3].kind else {
        panic!("the fill is a Solid layer");
    };
    let Some(ProjectItem::Solid(sky)) = doc.item(def) else {
        panic!("the fill has a solid");
    };
    assert_eq!(sky.colour.0, [0.0, 0.0, 1.0, 1.0]);

    // A layer style is the Lumit style of the same name. Light from 120
    // degrees throws a shadow at 150.
    let [shadow] = comp.layers[0].styles.as_slice() else {
        panic!("the title wears one style");
    };
    assert_eq!(shadow.effect.match_name, "style_drop_shadow");
    // Fill would fade the shadow with the picture, so a layer that wears a
    // style keeps its opacity.
    let opacity = comp.layers[0].transform.opacity.value_at(0.0);
    assert!(
        (opacity - 100.0).abs() < 0.1,
        "the title stays at {opacity}"
    );
    let row = |id: &str| match shadow.params.iter().find(|p| p.id == id) {
        Some(param) => match &param.value {
            EffectValue::Float(value) => value.value_at(0.0),
            _ => f64::NAN,
        },
        None => f64::NAN,
    };
    assert_eq!((row("opacity"), row("direction")), (40.0, 150.0));

    let folder = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Folder(f) if f.name == "poster layers" => Some(f),
            _ => None,
        })
        .expect("a folder for the layer items");
    // Five pictures, the adjustment's mask, the fill's solid and the inner
    // group's composition.
    assert_eq!(folder.children.len(), 8);

    project.undo().expect("one import, one step");
    assert_eq!(snapshot().items.len(), before, "undo takes all of it back");

    // A document with one layer is a still, and a still is not a document.
    let flat = dir.path().join("flat.psd");
    let one = [Layer::solid("Background", [0, 0, 32, 32], [255, 0, 0, 255])];
    std::fs::write(&flat, document(32, 32, 8, &one)).expect("the fixture writes");
    for plain in [flat, dir.path().join("photo.png")] {
        let answer = project.import_layers(plain.to_string_lossy().into_owned());
        assert_eq!(answer.expect("no error"), None);
    }
    assert_eq!(snapshot().items.len(), before, "and nothing was added");

    // The adjustment kinds whose numbers are reshaped on the way in. Over two
    // pictures, bottom first: Photoshop's own Warming Filter (85), a green
    // curve turned upside down, a tinted Black and White, a Vibrance with a
    // Saturation, a negative Vibrance, which is left out, and a Colour
    // Balance.
    let adjust = |name: &str, block: ([u8; 4], Vec<u8>)| {
        let mut layer = Layer::solid(name, [0, 0, 0, 0], [0; 4]);
        layer.blocks = vec![block];
        layer
    };
    let words = |lead: &[u8], words: &[i16], tail: &[u8]| {
        let words = words.iter().flat_map(|w| w.to_be_bytes());
        [lead, &words.collect::<Vec<u8>>(), tail].concat()
    };
    let white = ["Rd  ", "Grn ", "Bl  "].map(|key| (key, Value::Number(255.0)));
    let grey = [
        ("Rd  ", Value::Number(-10.0)),
        ("Yllw", Value::Number(60.0)),
        ("Grn ", Value::Number(40.0)),
        ("Cyn ", Value::Number(60.0)),
        ("Bl  ", Value::Number(20.0)),
        ("Mgnt", Value::Number(80.0)),
        ("useTint", Value::Switch(true)),
        ("tintColor", Value::Object(white.to_vec())),
    ];
    let vibrance = |vibrance: f64| {
        let items = [("vibrance", vibrance), ("Strt", -20.0)].map(|(k, v)| (k, Value::Number(v)));
        described(*b"vibA", &items)
    };
    let curve = words(&[0], &[1, 0, 4, 2, 255, 0, 0, 255], &[0]);
    let balance = words(&[], &[0, 0, 0, 100, 0, -100, 0, 0, 100], &[1, 0, 0, 0]);
    // The older layout, in Lab: 67.06, 32 and 120, then 25 per cent.
    let glass = words(&[], &[2, 7, 6706, 3200, 12000, 0, 0, 25], &[1, 0, 0, 0]);
    let layers = [
        Layer::solid("Background", [0, 0, 32, 32], [255, 0, 0, 255]),
        Layer::solid("Hat", [8, 8, 16, 16], [0, 0, 255, 255]),
        adjust("Glass", (*b"phfl", glass)),
        adjust("Curves", (*b"curv", curve)),
        adjust("Grey", described(*b"blwh", &grey)),
        adjust("Vibrance", vibrance(30.0)),
        adjust("Faded", vibrance(-30.0)),
        adjust("Balance", (*b"blnc", balance)),
    ];
    let grade = dir.path().join("grade.psd");
    std::fs::write(&grade, document(32, 32, 8, &layers)).expect("the fixture writes");
    let left_out = project.import_layers(grade.to_string_lossy().into_owned());
    assert_eq!(left_out.expect("the document imports"), Some(1));
    let doc = snapshot();
    let comp = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.name == "grade" => Some(c),
            _ => None,
        })
        .expect("a comp named for the file");
    let carried: Vec<Vec<&str>> = comp
        .layers
        .iter()
        .map(|l| l.effects.iter().map(|e| &*e.effect.match_name).collect())
        .collect();
    assert_eq!(
        carried,
        [
            vec!["colour_balance"],
            vec!["vibrancy", "saturation"],
            vec!["black_and_white"],
            vec!["srgb_curve", "curves", "srgb_curve"],
            vec!["photo_filter"],
            vec![],
            vec![]
        ]
    );
    let row = |layer: usize, effect: usize, id: &str| -> Vec<f64> {
        let params = &comp.layers[layer].effects[effect].params;
        match params.iter().find(|p| p.id == id).map(|p| &p.value) {
            Some(EffectValue::Float(v)) => vec![v.value_at(0.0)],
            Some(EffectValue::Colour(c)) => c.iter().map(|v| v.value_at(0.0)).collect(),
            Some(EffectValue::Bool(on)) => vec![f64::from(u8::from(*on))],
            Some(EffectValue::Curve(points)) => {
                points.iter().flatten().map(|v| f64::from(*v)).collect()
            }
            _ => Vec::new(),
        }
    };
    assert_eq!(
        row(3, 1, "green"),
        [0.0, 1.0, 1.0, 0.0],
        "input, then output"
    );
    assert_eq!(row(3, 1, "master"), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(
        (row(2, 0, "reds"), row(2, 0, "tint")),
        (vec![-10.0], vec![1.0])
    );
    assert_eq!(row(2, 0, "tint_colour"), [1.0; 4]);
    assert_eq!(
        (row(1, 0, "amount"), row(1, 1, "saturation")),
        (vec![30.0], vec![80.0])
    );
    assert_eq!(row(0, 0, "lift"), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        row(0, 0, "gamma"),
        [2.0, 1.0, 0.5, 1.0],
        "a stop a full slider"
    );
    assert_eq!(row(0, 0, "gain"), [1.0, 1.0, 2.0, 1.0]);
    // That filter is the orange Photoshop shows as 236, 138, 0.
    let glass = row(4, 0, "colour");
    assert!((glass[0] - 0.84).abs() < 0.01 && (glass[1] - 0.255).abs() < 0.01);
    assert_eq!((glass[2], row(4, 0, "density")), (0.0, vec![25.0]));
    assert_eq!(row(4, 0, "preserve_luminosity"), [1.0]);
}

// An Illustrator document comes in the same way: a comp the artboard's size
// with a layer for each top-level layer, top first, and a hidden layer hidden.
// One with a single layer is footage, and is still read by its own reader.
#[cfg(feature = "media")]
#[test]
fn an_illustrator_document_imports_as_a_comp_of_its_layers() {
    use lumit_core::model::LayerKind;
    use lumit_media::ai::fixture::{document, Layer};

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("logo.ai");
    let mut guide = Layer::solid("Guide", [0, 0, 4, 4], [0, 255, 0]);
    guide.visible = false;
    let layers = [
        Layer::solid("Background", [0, 0, 32, 48], [255, 0, 0]),
        Layer::solid("Mark", [8, 8, 16, 16], [0, 0, 255]),
        guide,
    ];
    std::fs::write(&path, document(48, 32, &layers)).expect("the fixture writes");

    let project = LumitBridgeState::new_project(None).expect("a new project");
    let snapshot = || {
        let state = project.state().expect("state");
        let state = state.read().expect("read");
        state.store.snapshot()
    };
    let before = snapshot().items.len();

    let left_out = project
        .import_layers(path.to_string_lossy().into_owned())
        .expect("the document imports");
    assert_eq!(left_out, Some(0));

    let doc = snapshot();
    let comp = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(c) if c.name == "logo" => Some(c),
            _ => None,
        })
        .expect("a comp named for the file");
    assert_eq!((comp.width, comp.height), (48, 32));
    let layers: Vec<(&str, bool, Option<u32>)> = comp
        .layers
        .iter()
        .map(|l| {
            let pick = match &l.kind {
                LayerKind::Footage { item } => match doc.item(*item) {
                    Some(ProjectItem::Footage(f)) => f.source_layer,
                    _ => None,
                },
                _ => None,
            };
            (l.name.as_str(), l.switches.visible, pick)
        })
        .collect();
    assert_eq!(
        layers,
        [
            ("Guide", false, Some(2)),
            ("Mark", true, Some(1)),
            ("Background", true, Some(0))
        ],
        "top first, each reading its own layer of the file"
    );
    assert!(doc.items.iter().any(|i| match i {
        ProjectItem::Folder(f) => f.name == "logo layers" && f.children.len() == 3,
        _ => false,
    }));

    project.undo().expect("one import, one step");
    assert_eq!(snapshot().items.len(), before, "undo takes all of it back");

    // One layer is a picture, not a document. ffmpeg can't open it, so the
    // footage item it becomes has to probe through the file's own reader.
    let flat = dir.path().join("flat.ai");
    let one = [Layer::solid("Layer 1", [0, 0, 32, 48], [255, 0, 0])];
    std::fs::write(&flat, document(48, 32, &one)).expect("the fixture writes");
    let answer = project.import_layers(flat.to_string_lossy().into_owned());
    assert_eq!(answer.expect("no error"), None);
    let probe = crate::probe::ensure_probed(flat.as_path()).expect("the document probes");
    let video = probe.video.as_ref().expect("a picture");
    assert_eq!((video.width, video.height), (48, 32));
}
