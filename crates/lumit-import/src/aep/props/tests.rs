use super::*;
use crate::aep::rifx::Chunks;

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    #[allow(clippy::cast_possible_truncation)]
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut inner = kind.to_vec();
    inner.extend_from_slice(body);
    chunk(b"LIST", &inner)
}

fn tdmn(name: &str) -> Vec<u8> {
    let mut body = vec![0_u8; 40];
    body.splice(..name.len(), name.bytes());
    chunk(b"tdmn", &body)
}

/// A 124-byte metadata record: dimension count, the colour bit and the
/// animated flag — the three fields the reader keys on.
fn tdb4_record(dimensions: u16, colour: bool, animated: bool) -> Vec<u8> {
    let mut body = vec![0_u8; 124];
    body.splice(2..4, dimensions.to_be_bytes());
    body[tdb4::TYPE_FLAGS] = if colour { 0x01 } else { 0x08 };
    body[tdb4::ANIMATED] = u8::from(animated);
    chunk(b"tdb4", &body)
}

fn cdat(values: &[f64]) -> Vec<u8> {
    let mut body = Vec::new();
    for value in values {
        body.extend_from_slice(&value.to_be_bytes());
    }
    chunk(b"cdat", &body)
}

fn lhd3_record(count: u16, item_size: u16, kind: u8) -> Vec<u8> {
    let mut body = vec![0_u8; 52];
    body.splice(lhd3::COUNT..lhd3::COUNT + 2, count.to_be_bytes());
    body.splice(
        lhd3::ITEM_SIZE..lhd3::ITEM_SIZE + 2,
        item_size.to_be_bytes(),
    );
    body[lhd3::TYPE] = kind;
    chunk(b"lhd3", &body)
}

/// One property inside a group, named and wrapped exactly as the file does.
fn leaf(name: &str, meta: Vec<u8>, tail: Vec<u8>) -> Vec<u8> {
    let mut inside = chunk(b"tdsb", &[0, 0, 0, 1]);
    inside.extend(chunk(b"tdsn", &chunk(b"Utf8", b"")));
    inside.extend(meta);
    inside.extend(tail);
    let mut out = tdmn(name);
    out.extend(list(b"tdbs", &inside));
    out
}

/// A whole `LIST tdgp` around the given runs, closed the way a real one is.
fn group(runs: Vec<u8>) -> Vec<u8> {
    let mut inside = chunk(b"tdsb", &[0, 0, 0, 1]);
    inside.extend(chunk(b"tdsn", &chunk(b"Utf8", b"")));
    inside.extend(runs);
    inside.extend(tdmn("ADBE Group End"));
    list(b"tdgp", &inside)
}

fn ctx() -> Ctx<'static> {
    Ctx {
        params: None,
        timebase: 25600.0,
        comp: (640.0, 360.0),
        layer: (640.0, 360.0),
        has_source: true,
        start: 0.0,
        in_effect: false,
        layers: None,
    }
}

/// Read one synthetic `LIST tdgp` the way [`read_group`] would.
fn read(bytes: &[u8]) -> Read {
    let chunk = Chunks::new(bytes)
        .ok()
        .next()
        .expect("the synthetic group parses");
    read_group(&chunk, ctx())
}

/// **A layer reference is stored as an id and reported as an index.**
///
/// A silent one before this was fixed, and the worst shape of silent: the
/// number the file holds in a `tdpi` is the target's **layer id**, which for
/// a real project is in the hundreds, while the whole mapping layer — Set
/// matte's row, Displacement map's, Texturize's, Set channels' — reads it as
/// a **stacking index**. Every one of those resolved to no layer at all and
/// imported as the effect's documented no-op, which looks exactly like a row
/// the user never filled in. Reading the owner's own project found it: ten
/// Set Channels instances naming layers 90, 167, 229, 463 and 704 in
/// compositions of nine and eighteen layers.
///
/// An id no layer in this composition claims stays zero — After Effects'
/// "None" — rather than becoming somebody else's index.
#[test]
fn a_layer_reference_is_reported_as_a_stacking_index_not_a_layer_id() {
    let ids: std::collections::HashMap<u32, u32> =
        [(167_u32, 2_u32), (704, 3)].into_iter().collect();
    let mut runs = leaf(
        "ADBE Set Channels-0001",
        tdb4_record(1, false, false),
        chunk(b"tdpi", &167_u32.to_be_bytes()),
    );
    runs.extend(leaf(
        "ADBE Set Channels-0003",
        tdb4_record(1, false, false),
        chunk(b"tdpi", &9999_u32.to_be_bytes()),
    ));
    runs.extend(leaf(
        "ADBE Set Channels-0005",
        tdb4_record(1, false, false),
        chunk(b"tdpi", &0_u32.to_be_bytes()),
    ));
    let bytes = group(runs);
    let chunk = Chunks::new(&bytes)
        .ok()
        .next()
        .expect("the synthetic group parses");
    let read = read_group(
        &chunk,
        Ctx {
            layers: Some(&ids),
            ..ctx()
        },
    );

    assert_eq!(read.properties.len(), 3);
    for p in &read.properties {
        assert_eq!(p.value_type.as_deref(), Some("layer"));
    }
    assert_eq!(read.properties[0].value, Some(json!(2)));
    assert_eq!(
        read.properties[1].value,
        Some(json!(0)),
        "an id no layer here claims is None, never another layer's index"
    );
    assert_eq!(read.properties[2].value, Some(json!(0)), "None stays None");
}

/// **An effect's point is a fraction of the layer, not of the composition.**
///
/// The Transform effect is where this is felt: its Anchor Point sits at the
/// layer's centre by default and is therefore *absent* from the file, so
/// the mapping layer writes it in at the layer's centre, while its Position
/// comes out of the file. Read the file's fraction against the composition
/// and the two land in different places on any layer that is not the comp's
/// size — a Transform nobody touched shifts the picture, and one whose
/// Position was moved moves it by the wrong distance. Here: a
/// 2560 × 1088 layer in a 1920 × 816 comp, both points at the layer's own
/// centre.
#[test]
fn an_effects_point_is_a_fraction_of_its_layer_not_of_the_composition() {
    let bytes = group(leaf(
        "ADBE Geometry2-0002",
        tdb4_record(2, false, false),
        cdat(&[0.5, 0.5, 0.0, 0.0]),
    ));
    let chunk = Chunks::new(&bytes)
        .ok()
        .next()
        .expect("the synthetic group parses");
    let read = read_group(
        &chunk,
        Ctx {
            comp: (1920.0, 816.0),
            layer: (2560.0, 1088.0),
            in_effect: true,
            ..ctx()
        },
    );

    assert_eq!(read.properties.len(), 1);
    assert_eq!(read.properties[0].value, Some(json!([1280.0, 544.0])));
}

/// **A keyframe class no table knows falls back to the static value, and
/// says which class it was.**
///
/// The discipline the whole route rests on. A record size this reader does
/// not recognise could be decoded *nearly* right and produce a curve that is
/// subtly wrong for the rest of the project's life; instead the property
/// keeps its still value, the note names the class, and the import report
/// carries a row a person can act on.
#[test]
fn an_unknown_keyframe_class_falls_back_rather_than_guessing_a_curve() {
    let mut animation = lhd3_record(1, 999, 4);
    animation.extend(chunk(b"ldat", &[0_u8; 999]));
    let mut tail = cdat(&[7.0; 5]);
    tail.extend(list(b"list", &animation));

    let read = read(&group(leaf(
        "ADBE Rotate Z",
        tdb4_record(1, false, true),
        tail,
    )));
    let node = &read.properties[0];

    assert!(node.keyframes.is_none(), "no curve is invented");
    assert_eq!(node.value, Some(json!(7.0)), "the still value stands in");
    assert!(node
        .unreadable
        .as_deref()
        .is_some_and(|why| why.contains("4/999")));
    assert_eq!(read.skipped.len(), 1, "and the report is told");
}

/// **A keyframe list that claims more records than the bytes hold reads the
/// ones that are there.**
///
/// `lhd3` says how many records follow and how long each is, and both
/// numbers come out of an untrusted file. Trusting them is how a parser
/// reads past the end of a buffer; the count is clamped to what `ldat`
/// actually carries, so a truncated project loses the tail of an animation
/// rather than crashing.
#[test]
fn a_keyframe_list_that_overstates_its_length_reads_only_what_is_there() {
    let mut animation = lhd3_record(50, 48, 4);
    // Two whole 48-byte records, and no more.
    let mut records = Vec::new();
    for (index, value) in [10.0_f64, 20.0].into_iter().enumerate() {
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let time = (index as i32) * 25600;
        records.extend_from_slice(&time.to_be_bytes());
        records.extend_from_slice(&[1, 1, 0, 0]);
        records.extend_from_slice(&value.to_be_bytes());
        records.extend_from_slice(&[0_u8; 32]);
    }
    animation.extend(chunk(b"ldat", &records));
    let tail = list(b"list", &animation);

    let read = read(&group(leaf(
        "ADBE Rotate Z",
        tdb4_record(1, false, true),
        tail,
    )));
    let keys = read.properties[0]
        .keyframes
        .as_deref()
        .expect("the readable records are keys");
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].t, Some(0.0));
    assert_eq!(keys[1].t, Some(1.0));
    assert_eq!(keys[1].v, Some(json!(20.0)));
    // Linear on both sides, so the speed is the segment's own slope and the
    // influence After Effects' default — neither number is in the file.
    let out = keys[0].out_ease.as_deref().unwrap_or_default();
    assert_eq!(out[0].speed, Some(10.0));
    assert_eq!(out[0].influence, Some(DEFAULT_INFLUENCE));
}
