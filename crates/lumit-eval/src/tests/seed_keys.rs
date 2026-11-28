use super::*;

/// A keyed seed keys each frame by the seed it shows, and nothing else: an
/// unkeyed seed saves and keys exactly as before.
#[test]
fn a_keyed_seed_keys_the_seed_each_frame_shows() {
    let mut doc = Document::new();
    let def = Uuid::now_v7();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: def,
        name: "s".into(),
        colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
        width: 64,
        height: 64,
        extra: serde_json::Map::new(),
    }));
    let mut l = text_layer("x", 0.0, 10.0, 0.0);
    l.kind = LayerKind::Solid { def };
    l.effects
        .push(lumit_core::fx::builtins::instantiate("shake").unwrap());
    let shaken = comp_with(vec![l]);
    let old = f64::from(shaken.layers[0].effects[0].seed_at("seed", 0.0).unwrap());
    let with = |animation| {
        let mut c = shaken.clone();
        let mut rows = c.layers[0].effects[0].params.iter_mut();
        let seed = rows.find(|p| p.id == "seed").unwrap();
        assert!(seed.set_seed(animation));
        c
    };
    let json = |c: &Composition| serde_json::to_string(c).unwrap();
    assert_eq!(json(&with(Animation::Static(old))), json(&shaken));

    let reseeded = with(Animation::Static(old + 1.0));
    // Linear keys, held all the same: the frame between the two shows the first.
    let key_at = |s: i64, value: f64| Keyframe {
        time: Rational::new(s, 1).unwrap(),
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    let keyed = with(Animation::Keyframed(vec![
        key_at(0, old),
        key_at(2, old + 1.0),
    ]));
    assert_eq!(key(&doc, &keyed, 1.0), key(&doc, &shaken, 1.0));
    assert_eq!(key(&doc, &keyed, 3.0), key(&doc, &reseeded, 3.0));
    assert_ne!(key(&doc, &keyed, 1.0), key(&doc, &keyed, 3.0));
}
