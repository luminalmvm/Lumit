//! The property system: the `tdgp`/`tdbs`/`tdb4` trees, their static values,
//! their keyframes, and the specialised nodes hanging off them — effects,
//! masks, shapes, markers, expressions (K-418 phase B,
//! docs/impl/ae-import.md §7.2).
//!
//! # In plain terms
//!
//! Everything a layer *is*, past its name and its timing, lives in one tree of
//! boxes. A box named `tdgp` is a group (Transform, Masks, Effects); a box
//! named `tdbs` is a single property (Opacity, a blur radius). They alternate
//! with little 40-byte `tdmn` labels: a label, then the thing it names, a
//! label, then the thing it names, ending at the label `ADBE Group End`. So
//! reading a layer's properties is reading that alternation, and recursing
//! wherever the thing named is another group.
//!
//! Three things about it are worth knowing before the code makes sense.
//!
//! - **The file only stores what is not at its default.** A layer nobody moved
//!   has no Position box at all. That is not damage and it is not something to
//!   guess around: the property is simply absent from the capture, exactly as
//!   it is absent from the file, and the mapping layer already treats an absent
//!   property as "use the default" — the same thing After Effects does.
//! - **The stored numbers are not the numbers After Effects reports.** Opacity
//!   is a fraction on disk and a percentage in the DOM; a colour is A,R,G,B in
//!   0–255 on disk and R,G,B,A in 0–1 in the DOM; an effect's point is a
//!   fraction of the layer on disk and pixels in the DOM. Every one of those
//!   conversions is here, in [`resolve`], and is proven against the golden
//!   capture rather than assumed.
//! - **A keyframe's layout depends on what kind of property it belongs to.**
//!   The record size is written in the file (`lhd3`), never assumed, and the
//!   shape of the record is chosen from that size — a one-dimensional key is 48
//!   bytes, a three-dimensional spatial one is 128. A size no table knows makes
//!   the property fall back to its static value *with* a note naming the class,
//!   which is the whole discipline of this route: never a wrong curve.
//!
//! Reimplemented in Rust from `forticheprod/aep_parser` (MIT, licence checked
//! 2026-08-21), read as documentation — `binary/property_chunks.py`,
//! `binary/ldat_chunks.py`, `binary/misc_chunks.py` and `parsers/*.py` are the
//! map. No code is vendored.

use serde_json::json;

use super::enums;
use super::rifx::{bit, f32_at, i32_at, text_of, u16_at, u32_at, u8_at, Chunk};
use crate::capture::{Ease, Keyframe, Marker, Mask, Property, Unreadable};

/// `tdb4` field offsets. The record is 124 bytes of fixed layout; only the
/// fields below are read, and everything else is deliberately untouched.
mod tdb4 {
    /// Dimension count (u16).
    pub const DIMENSIONS: usize = 2;
    /// Spatial/static flag byte — bit 3 is "this property is spatial".
    pub const SPATIAL_FLAGS: usize = 5;
    /// "This property carries no numeric value" flag byte, bit 0.
    pub const NO_VALUE_FLAGS: usize = 57;
    /// Type flag byte — bit 3 vector, bit 2 integer, bit 0 colour.
    pub const TYPE_FLAGS: usize = 59;
    /// Whether the property is animated (u8) — what scripting reports as
    /// `timeRemapEnabled` for `ADBE Time Remapping`.
    pub const ANIMATED: usize = 68;
    /// Expression flag byte, bit 0 = the expression is switched *off*.
    pub const EXPRESSION_FLAGS: usize = 119;
}

/// `lhd3` field offsets — the header of a keyframe (or shape-point) list.
mod lhd3 {
    /// Number of records (u16).
    pub const COUNT: usize = 10;
    /// Bytes per record (u16). Read, never assumed: it is what says which
    /// class of keyframe the records are.
    pub const ITEM_SIZE: usize = 18;
    /// The list's own type code (u8), which the size is paired with.
    pub const TYPE: usize = 23;
}

/// The properties After Effects stores as a fraction and reports as a
/// percentage. Proven against the golden capture: the file holds Opacity 1.0
/// where the DOM says 100.
const PERCENT: &[&str] = &["ADBE Opacity", "ADBE Scale", "ADBE Mask Opacity"];

/// The display name After Effects writes for a property nobody renamed. It is
/// a sentinel, not a name, so it never reaches the capture.
const UNNAMED: &str = "-_0_/-";

/// The SDK parameter types that are not parameters: a topic heading and its
/// end, and a slot the plug-in declared as carrying nothing. Scripting reports
/// all three as unreadable groups, which is what the capture must say too —
/// the file stores a zero in each, and a zero is not what they mean.
const PARAM_TOPIC: &[u8] = &[13, 14, 15];

/// The SDK parameter type for arbitrary data: Curves' point list, Levels'
/// histogram, Hue/Saturation's channel ranges. The DOM cannot read these at
/// all (K-410); the file can, and does, so the bytes come through.
const PARAM_ARBITRARY: u8 = 11;

/// One effect's parameter definitions, by match name — the `pard` records in
/// the effect's own `parT` list. Each holds the SDK parameter type and the
/// plug-in's own name for the slot, which is the name scripting reports: a
/// parameter's `tdsn` is sometimes the neighbouring slot's name instead.
pub(crate) type ParamTypes = std::collections::HashMap<String, (u8, String)>;

/// What a group needs to know about where it sits, so the values inside it can
/// be put back into the DOM's units.
///
/// All four of the sizes matter: an effect's point, an anchor point and a
/// mask's path are each a fraction of the *layer's* frame. Getting the wrong
/// one puts the mask in the wrong place on a layer that is not comp-sized.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ctx<'a> {
    /// The containing effect's parameter definitions, when there is one, read
    /// from its own `parT`. They are the only way to tell a real parameter
    /// from a topic heading, and they carry the plug-in's own parameter names.
    pub params: Option<&'a ParamTypes>,
    /// `cdta`'s internal timebase: keyframe times are a count of these per
    /// second.
    pub timebase: f64,
    /// The comp's width and height in pixels.
    pub comp: (f64, f64),
    /// The layer's source width and height, where it has a source.
    pub layer: (f64, f64),
    /// Whether the layer has a source item at all. A shape, text or null layer
    /// stores its anchor in raw pixels; a footage layer stores a fraction.
    pub has_source: bool,
    /// The layer's start time in seconds — keyframe times are stored relative
    /// to it and reported by scripting in comp time.
    pub start: f64,
    /// Whether the group being read sits inside an effect, which changes how a
    /// two-dimensional value is read.
    pub in_effect: bool,
    /// The comp's layer ids to stacking indices. A property that points at
    /// another layer stores that layer's **id**, and the capture's vocabulary
    /// is the *index* scripting reports — so the one is turned into the other
    /// here, where the map is known, rather than left for a reader that has
    /// no way to tell the two apart.
    pub layers: Option<&'a std::collections::HashMap<u32, u32>>,
}

/// Everything one property subtree produced.
pub(crate) struct Read {
    /// The children of the group, in file order.
    pub properties: Vec<Property>,
    /// Rows for anything skipped along the way.
    pub skipped: Vec<Unreadable>,
}

/// Read one `LIST tdgp`'s children into capture [`Property`] nodes.
///
/// The recursion is bounded for free: [`Chunk::children`] refuses past the
/// container walk's depth cap, so a hostile file that nests groups without end
/// runs out of children rather than out of stack.
pub(crate) fn read_group(group: &Chunk<'_>, ctx: Ctx<'_>) -> Read {
    let mut out = Read {
        properties: Vec::new(),
        skipped: Vec::new(),
    };
    let mut separated: Vec<String> = Vec::new();
    for (match_name, chunks) in runs(group) {
        if let Some((node, is_separated)) = read_node(&match_name, &chunks, ctx, &mut out.skipped) {
            if is_separated {
                separated.push(match_name);
            }
            out.properties.push(node);
        }
    }
    fill_separated(&mut out.properties, &separated);
    out
}

/// The markers of one layer, out of its `ADBE Marker` property.
///
/// Markers are keyframes whose value is a little record rather than a number,
/// and the capture carries them as their own list rather than in the property
/// tree — which is exactly what the Bridge's walker does with them
/// (docs/impl/ae-import.md §2). A comp's markers live on the hidden `SecL`
/// layer and come through this same function.
pub(crate) fn read_markers(group: &Chunk<'_>, ctx: Ctx<'_>) -> Vec<Marker> {
    let Some((_, chunks)) = runs(group)
        .into_iter()
        .find(|(name, _)| name == "ADBE Marker")
    else {
        return Vec::new();
    };
    let Some(mrst) = chunks.iter().find(|chunk| chunk.is_list(b"mrst")) else {
        return Vec::new();
    };
    let inside: Vec<Chunk<'_>> = mrst.children().ok().collect();

    // The times sit in the ordinary keyframe list; the comments and durations
    // sit beside it in `mrky`, one `Nmrd` per key, in the same order.
    let times: Vec<f64> = inside
        .iter()
        .find(|chunk| chunk.is_list(b"tdbs"))
        .map(|tdbs| {
            records(&tdbs.children().ok().collect::<Vec<_>>())
                .map(|(_, _, list)| list.map(|record| time_of(&record, ctx)).collect::<Vec<_>>())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let values = inside
        .iter()
        .find(|chunk| chunk.is_list(b"mrky"))
        .map(|mrky| {
            mrky.children()
                .ok()
                .filter(|chunk| chunk.is_list(b"Nmrd"))
                .map(|nmrd| marker_value(&nmrd))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    values
        .into_iter()
        .enumerate()
        .map(|(index, (duration, label, comment, chapter))| Marker {
            t: times.get(index).copied(),
            duration: Some(duration),
            comment: Some(comment),
            chapter: Some(chapter),
            label: Some(label),
        })
        .collect()
}

/// One `Nmrd`: the marker's duration in seconds, its label, its comment and its
/// chapter text. `NmHd` is 20 bytes with the duration in 600ths of a second at
/// offset 8 and the label at 16; the `Utf8` chunks that follow are, in order,
/// comment, chapter, URL, frame target and cue-point name.
fn marker_value(nmrd: &Chunk<'_>) -> (f64, u32, String, String) {
    let inside: Vec<Chunk<'_>> = nmrd.children().ok().collect();
    let head = inside.iter().find(|chunk| chunk.id == *b"NmHd");
    let duration = head
        .and_then(|chunk| u32_at(chunk.body, 8))
        .map_or(0.0, |ticks| f64::from(ticks) / 600.0);
    let label = head
        .and_then(|chunk| u8_at(chunk.body, 16))
        .map_or(0, u32::from);
    let mut texts = inside.iter().filter(|chunk| chunk.id == *b"Utf8");
    let comment = texts.next().map(Chunk::text).unwrap_or_default();
    let chapter = texts.next().map(Chunk::text).unwrap_or_default();
    (duration, label, comment, chapter)
}

/// Split a group's children into `(match name, the chunks it names)` runs.
///
/// A group reads as a `tdsb`, a `tdsn`, then alternating `tdmn` labels and the
/// nodes they name, closed by the label `ADBE Group End`. A run can hold more
/// than one chunk — a mask carries `mkif` beside its group, an arbitrary-data
/// effect parameter carries `aRbs` beside its `tdbs` — so the run keeps them
/// all and the reader picks.
fn runs<'a>(group: &Chunk<'a>) -> Vec<(String, Vec<Chunk<'a>>)> {
    let mut out: Vec<(String, Vec<Chunk<'a>>)> = Vec::new();
    let mut open = false;
    for child in group.children().ok() {
        if child.id == *b"tdmn" {
            let name = child.text();
            open = name != "ADBE Group End";
            if open {
                out.push((name, Vec::new()));
            }
        } else if open {
            if let Some(run) = out.last_mut() {
                run.1.push(child);
            }
        }
    }
    out
}

/// One run, as the capture node it describes plus whether it is a
/// dimension-separated leader — or nothing, when the run holds no node that
/// belongs in the capture's property tree.
fn read_node(
    match_name: &str,
    chunks: &[Chunk<'_>],
    ctx: Ctx<'_>,
    skipped: &mut Vec<Unreadable>,
) -> Option<(Property, bool)> {
    // An effect's index-0 parameter is After Effects' own internal slot and is
    // not exposed by scripting at all, so it must not reach the capture: the
    // effect table is keyed by match name and would see a parameter the DOM
    // never mentions.
    if ctx.in_effect && match_name.ends_with("-0000") {
        return None;
    }
    // The one match name the tree deliberately skips: its keys are markers,
    // and the capture carries those as their own list (docs/impl §2).
    if match_name == "ADBE Marker" {
        return None;
    }

    let list = |kind: &[u8; 4]| chunks.iter().find(|chunk| chunk.is_list(kind));

    if let Some(sspc) = list(b"sspc") {
        return Some((read_effect(match_name, sspc, ctx, skipped), false));
    }
    if let Some(group) = list(b"tdgp") {
        return Some((
            read_subgroup(match_name, group, chunks, ctx, skipped),
            false,
        ));
    }
    if let Some(shape) = list(b"om-s") {
        return Some((read_shape(match_name, shape, chunks, ctx, skipped), false));
    }
    if let Some(orientation) = list(b"otst") {
        return Some((
            read_orientation(match_name, orientation, ctx, skipped),
            false,
        ));
    }
    if let Some(tdbs) = list(b"tdbs") {
        let separated = tdbs
            .children()
            .ok()
            .find(|chunk| chunk.id == *b"tdsb")
            .and_then(|chunk| u8_at(chunk.body, 3))
            .is_some_and(|flags| bit(flags, 1));
        return Some((read_leaf(match_name, tdbs, chunks, ctx, skipped), separated));
    }
    // A text document (`btds`) and a gradient (`GCst`) are their own encodings
    // and are phase C; a node type from a newer After Effects lands here too.
    // Either way the property is named and marked rather than dropped, which is
    // what lets the import report say what was lost.
    let kind = chunks.first().and_then(|chunk| chunk.list_type)?;
    Some((
        unreadable_node(
            match_name,
            &format!("a {} property is not decoded yet", text_of(&kind)),
        ),
        false,
    ))
}

/// A group node — Transform, Masks, a mask atom, an effect's parameters.
fn read_subgroup(
    match_name: &str,
    group: &Chunk<'_>,
    run: &[Chunk<'_>],
    ctx: Ctx<'_>,
    skipped: &mut Vec<Unreadable>,
) -> Property {
    let inside: Vec<Chunk<'_>> = group.children().ok().collect();
    let mut read = read_group(group, ctx);
    skipped.append(&mut read.skipped);

    // The Layer Styles group has no switch of its own: After Effects shows it
    // as on when *any* style below it is on, and its Blending Options row
    // mirrors that. Reading the group's own `tdsb` here says "on" for every
    // layer in the project, which is not what the DOM reports.
    let mut enabled = enabled_of(&inside);
    if match_name == "ADBE Layer Styles" {
        enabled = read.properties.iter().any(|style| {
            style.match_name.as_deref() != Some("ADBE Blend Options Group")
                && style.enabled == Some(true)
        });
        for style in &mut read.properties {
            if style.match_name.as_deref() == Some("ADBE Blend Options Group") {
                style.enabled = Some(enabled);
            }
        }
    }

    Property {
        match_name: Some(match_name.to_string()),
        name: display_name(&inside),
        enabled: Some(enabled),
        mask: run
            .iter()
            .find(|chunk| chunk.id == *b"mkif")
            .map(|chunk| read_mask(chunk.body, &inside)),
        group: Some(read.properties),
        ..Property::default()
    }
}

/// An effect instance: its parameters are an ordinary group, its display name
/// is the `fnam` chunk, and its on/off switch is the group's own `tdsb`.
fn read_effect(
    match_name: &str,
    sspc: &Chunk<'_>,
    ctx: Ctx<'_>,
    skipped: &mut Vec<Unreadable>,
) -> Property {
    let inside: Vec<Chunk<'_>> = sspc.children().ok().collect();
    let name = inside
        .iter()
        .find(|chunk| chunk.id == *b"fnam")
        .and_then(|chunk| chunk.children().ok().find(|c| c.id == *b"Utf8"))
        .map(|chunk| chunk.text());

    // The effect's own parameter definitions say which of its slots are real
    // parameters and which are topic headings or arbitrary-data blocks. When
    // the layer's copy is empty the project's `LIST EfdG` carries the same
    // table for every effect in use — reading that fallback is owed
    // (docs/TODO.md); an effect without it simply reads its slots as the plain
    // numbers they are stored as.
    let param_types = inside
        .iter()
        .find(|chunk| chunk.is_list(b"parT"))
        .map(read_param_types)
        .unwrap_or_default();
    let inner = Ctx {
        in_effect: true,
        params: Some(&param_types),
        ..ctx
    };

    let (children, enabled) = match inside.iter().find(|chunk| chunk.is_list(b"tdgp")) {
        Some(group) => {
            let params: Vec<Chunk<'_>> = group.children().ok().collect();
            let mut read = read_group(group, inner);
            skipped.append(&mut read.skipped);
            (read.properties, enabled_of(&params))
        }
        None => (Vec::new(), true),
    };

    Property {
        match_name: Some(match_name.to_string()),
        name,
        enabled: Some(enabled),
        group: Some(children),
        ..Property::default()
    }
}

/// One effect's `parT` list: alternating `tdmn` match names and 148-byte
/// `pard` definitions, of which the SDK parameter type at byte 15 and the
/// 32-byte name at 16 are read.
fn read_param_types(part: &Chunk<'_>) -> ParamTypes {
    let mut out = ParamTypes::new();
    let mut named: Option<String> = None;
    for child in part.children().ok() {
        if child.id == *b"tdmn" {
            named = Some(child.text());
        } else if child.id == *b"pard" {
            if let (Some(name), Some(kind)) = (named.take(), u8_at(child.body, 15)) {
                let label = child.body.get(16..48).map(text_of).unwrap_or_default();
                out.insert(name, (kind, label));
            }
        }
    }
    out
}

/// A mask's own facts, out of the 48-byte `mkif` record beside its group. The
/// RotoBezier flag is not in there — it is the Mask Shape property's `tdsb`.
fn read_mask(body: &[u8], inside: &[Chunk<'_>]) -> Mask {
    let roto = inside
        .iter()
        .position(|chunk| chunk.id == *b"tdmn" && chunk.text() == "ADBE Mask Shape")
        .and_then(|at| inside.get(at + 1))
        .and_then(|shape| shape.children().ok().find(|chunk| chunk.is_list(b"tdbs")))
        .and_then(|tdbs| tdbs.children().ok().find(|chunk| chunk.id == *b"tdsb"))
        .and_then(|tdsb| u8_at(tdsb.body, 0))
        .is_some_and(|flag| flag != 0);

    Mask {
        mode: u16_at(body, 6).map(|code| enums::mask_mode(u32::from(code))),
        inverted: u8_at(body, 0).map(|flag| flag != 0),
        roto_bezier: Some(roto),
        locked: u8_at(body, 1).map(|flag| flag != 0),
        colour: match (u8_at(body, 45), u8_at(body, 46), u8_at(body, 47)) {
            (Some(r), Some(g), Some(b)) => Some(vec![
                f64::from(r) / 255.0,
                f64::from(g) / 255.0,
                f64::from(b) / 255.0,
            ]),
            _ => None,
        },
    }
}

/// A bezier path property — a mask's outline, or a shape layer's path.
///
/// The `om-s` container holds the ordinary property metadata in a `tdbs` and
/// the path values beside it in `omks`, one `shap` per keyframe (or one for a
/// still path). The two are joined by position: key *n*'s time and easing come
/// from the `tdbs`, its value from the *n*th `shap`.
fn read_shape(
    match_name: &str,
    oms: &Chunk<'_>,
    run: &[Chunk<'_>],
    ctx: Ctx<'_>,
    skipped: &mut Vec<Unreadable>,
) -> Property {
    let inside: Vec<Chunk<'_>> = oms.children().ok().collect();
    let paths: Vec<serde_json::Value> = inside
        .iter()
        .find(|chunk| chunk.is_list(b"omks"))
        .map(|omks| {
            omks.children()
                .ok()
                .filter(|chunk| chunk.is_list(b"shap"))
                .map(|shap| read_path(&shap, ctx))
                .collect()
        })
        .unwrap_or_default();

    let Some(tdbs) = inside.iter().find(|chunk| chunk.is_list(b"tdbs")) else {
        return unreadable_node(match_name, "a path property has no metadata record");
    };

    let mut leaf = read_leaf(match_name, tdbs, run, ctx, skipped);
    leaf.value_type = Some("shape".to_string());
    leaf.unreadable = None;
    match leaf.keyframes.as_mut() {
        // An animated path: the keys carry their times and easing already, and
        // the values arrive here.
        Some(keys) => {
            for (key, path) in keys.iter_mut().zip(paths.iter()) {
                key.v = Some(path.clone());
            }
            leaf.value = None;
        }
        None => leaf.value = paths.first().cloned(),
    }
    leaf
}

/// One `shap`: a bounding box in `shph` and a run of normalised points.
///
/// Every three points are one cycle — vertex, the vertex's *out* tangent, then
/// the *in* tangent of the next vertex — and each point is a fraction of the
/// bounding box. A mask's box is itself a fraction of the layer, so its points
/// are scaled twice; a shape layer's box is already in pixels.
fn read_path(shap: &Chunk<'_>, ctx: Ctx<'_>) -> serde_json::Value {
    let inside: Vec<Chunk<'_>> = shap.children().ok().collect();
    let Some(header) = inside.iter().find(|chunk| chunk.id == *b"shph") else {
        return json!({ "vertices": [], "in_tangents": [], "out_tangents": [], "closed": false });
    };
    let closed = !bit(u8_at(header.body, 3).unwrap_or_default(), 3);
    let box_of = |at: usize| f64::from(f32_at(header.body, at).unwrap_or_default());
    let (left, top, right, bottom) = (box_of(4), box_of(8), box_of(12), box_of(16));

    let points: Vec<(f64, f64)> = records(&inside)
        .and_then(|(_, size, items)| {
            // A shape point is a pair of big-endian f32s; anything else is a
            // layout this reader does not know, and an empty path is honest.
            (size == 8).then(|| {
                items
                    .map(|point| {
                        (
                            f64::from(f32_at(&point, 0).unwrap_or_default()),
                            f64::from(f32_at(&point, 4).unwrap_or_default()),
                        )
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Mask space is the layer's, not the comp's — a mask on a layer smaller
    // than its comp reads at the wrong scale otherwise.
    let (width, height) = if ctx.has_source { ctx.layer } else { ctx.comp };
    let at = |index: usize| {
        let (x, y) = points
            .get(index % points.len().max(1))
            .copied()
            .unwrap_or((0.0, 0.0));
        [
            (left * (1.0 - x) + right * x) * width,
            (top * (1.0 - y) + bottom * y) * height,
        ]
    };

    let mut vertices = Vec::new();
    let mut in_tangents = Vec::new();
    let mut out_tangents = Vec::new();
    for index in (0..points.len()).step_by(3) {
        let vertex = at(index);
        let out = at(index + 1);
        let incoming = at((index + points.len() - 1) % points.len());
        vertices.push(vertex);
        out_tangents.push([out[0] - vertex[0], out[1] - vertex[1]]);
        in_tangents.push([incoming[0] - vertex[0], incoming[1] - vertex[1]]);
    }

    json!({
        "vertices": vertices,
        "in_tangents": in_tangents,
        "out_tangents": out_tangents,
        "closed": closed,
    })
}

/// A 3D Orientation, which After Effects wraps in its own `otst` container so
/// that the three angles can be stored as one quantity. The value is the
/// `otda` record inside `otky`; the `tdbs` beside it carries the metadata.
fn read_orientation(
    match_name: &str,
    otst: &Chunk<'_>,
    ctx: Ctx<'_>,
    skipped: &mut Vec<Unreadable>,
) -> Property {
    let inside: Vec<Chunk<'_>> = otst.children().ok().collect();
    let Some(tdbs) = inside.iter().find(|chunk| chunk.is_list(b"tdbs")) else {
        return unreadable_node(match_name, "an orientation property has no metadata record");
    };
    let mut leaf = read_leaf(match_name, tdbs, &[], ctx, skipped);
    // One `otda` per keyframe, in key order, and a single one when the
    // property is still. The `tdbs` beside them carries the times and the
    // eases but *not* the angles, so a keyframed orientation read from `tdbs`
    // alone is a row of zeroes — the angles only exist here.
    let angles: Vec<Vec<f64>> = inside
        .iter()
        .find(|chunk| chunk.is_list(b"otky"))
        .into_iter()
        .flat_map(|otky| otky.children().flatten())
        .filter(|chunk| chunk.id == *b"otda")
        .map(|otda| doubles(otda.body))
        .filter(|a| a.len() >= 3)
        .collect();
    if let Some(first) = angles.first() {
        leaf.value = Some(json!(first.get(..3).unwrap_or_default()));
        leaf.value_type = Some("point3".to_string());
        leaf.unreadable = None;
    }
    if let Some(keys) = leaf.keyframes.as_mut() {
        if keys.len() == angles.len() {
            for (key, angle) in keys.iter_mut().zip(&angles) {
                key.v = Some(json!(angle.get(..3).unwrap_or_default()));
            }
        }
    }
    leaf
}

/// One leaf property, out of its `LIST tdbs`.
fn read_leaf(
    match_name: &str,
    tdbs: &Chunk<'_>,
    run: &[Chunk<'_>],
    ctx: Ctx<'_>,
    skipped: &mut Vec<Unreadable>,
) -> Property {
    let inside: Vec<Chunk<'_>> = tdbs.children().ok().collect();
    let meta = inside.iter().find(|chunk| chunk.id == *b"tdb4");
    let m = meta.map(|chunk| chunk.body).unwrap_or_default();

    let dimensions = usize::from(u16_at(m, tdb4::DIMENSIONS).unwrap_or(1)).clamp(1, 16);
    let flags = u8_at(m, tdb4::TYPE_FLAGS).unwrap_or_default();
    let colour = bit(flags, 0);
    let spatial = bit(u8_at(m, tdb4::SPATIAL_FLAGS).unwrap_or_default(), 3);
    let no_value = bit(u8_at(m, tdb4::NO_VALUE_FLAGS).unwrap_or_default(), 0);

    let mut node = Property {
        match_name: Some(match_name.to_string()),
        name: display_name(&inside),
        ..Property::default()
    };

    // A property that points at another layer, or at a mask, stores the
    // reference rather than a number; scripting reports it as an index and the
    // capture's vocabulary has its own word for each.
    if let Some(reference) = inside.iter().find(|chunk| chunk.id == *b"tdli") {
        node.value_type = Some("mask".to_string());
        node.value = Some(json!(i32_at(reference.body, 0).unwrap_or_default()));
        return node;
    }
    if let Some(reference) = inside.iter().find(|chunk| chunk.id == *b"tdpi") {
        // Stored as the target's layer id; reported as its stacking index. A
        // zero is After Effects' "None", and an id no layer in this comp
        // claims stays zero rather than becoming somebody else's index.
        let id = u32_at(reference.body, 0).unwrap_or_default();
        let index = match (id, ctx.layers) {
            (0, _) => 0,
            (id, Some(map)) => map.get(&id).copied().unwrap_or_default(),
            (id, None) => id,
        };
        node.value_type = Some("layer".to_string());
        node.value = Some(json!(index));
        return node;
    }

    // What the effect said this slot is. A topic heading and a declared-empty
    // slot are not parameters at all, and the zero the file stores in each is
    // not a value — scripting reports both as unreadable, and so does this.
    let declared = ctx.params.and_then(|table| table.get(match_name));
    if let Some(label) = declared
        .map(|(_, label)| label)
        .filter(|l| !l.trim().is_empty())
    {
        node.name = Some(label.clone());
    }
    let declared = declared.map(|(kind, _)| *kind);
    if declared.is_some_and(|kind| PARAM_TOPIC.contains(&kind)) {
        node.value_type = Some("group".to_string());
        node.unreadable =
            Some("an effect topic heading, which carries no value of its own".to_string());
        return node;
    }

    // An arbitrary-data parameter — Curves' point list, Levels' histogram. The
    // DOM cannot read it at all (K-410), but the bytes *are* in the file, so
    // they are carried as hex beside a note saying what they are. Decoding them
    // is a stretch goal (K-412), not a promise.
    let animated = u8_at(m, tdb4::ANIMATED).is_some_and(|flag| flag != 0);
    if no_value || declared == Some(PARAM_ARBITRARY) {
        if let Some(blob) = run.iter().find(|chunk| chunk.is_list(b"aRbs")) {
            let bytes: Vec<u8> = blob
                .children()
                .ok()
                .find(|chunk| chunk.id == *b"aRbp")
                .map_or_else(Vec::new, |chunk| chunk.body.to_vec());
            node.value_type = Some("custom_blob".to_string());
            node.value = Some(json!({ "bytes": hex(&bytes) }));
            node.unreadable = Some(format!(
                "arbitrary data: {} bytes carried undecoded",
                bytes.len()
            ));
            return node;
        }
        // Declared as carrying nothing, and carrying nothing: a media
        // replacement slot, a plug-in's own bookkeeping. Scripting reports
        // these as unreadable groups, and the zero the file stores is not a
        // value to hand on in their place. An *animated* one is a different
        // thing — a mask path's own keys are recorded this way, and its values
        // live in a container beside it — so it falls through.
        if !animated {
            node.value_type = Some("group".to_string());
            node.unreadable = Some("the property carries no value of its own".to_string());
            return node;
        }
    }

    node.value_type = Some(
        match (colour, dimensions) {
            (true, _) => "colour",
            (false, 1) => "float",
            (false, 2) => "point",
            (false, 3) => "point3",
            _ => "other",
        }
        .to_string(),
    );

    // The expression is a plain `Utf8` beside the value, and whether it is
    // *on* is a flag in the metadata — a disabled expression is still stored,
    // which is exactly why the DOM reports both.
    if let Some(source) = inside.iter().find(|chunk| chunk.id == *b"Utf8") {
        let text = source.text();
        if !text.is_empty() {
            node.expression = Some(text);
            node.expression_enabled = Some(!bit(
                u8_at(m, tdb4::EXPRESSION_FLAGS).unwrap_or_default(),
                0,
            ));
        }
    }

    let scale = scale_of(match_name, colour, dimensions, ctx);

    if animated {
        match keyframes(&inside, dimensions, colour, spatial, &scale, ctx) {
            Ok(mut keys) => {
                resolve_ease(&mut keys, spatial);
                node.keyframes = Some(keys);
                return node;
            }
            Err(why) => {
                node.unreadable = Some(why.clone());
                skipped.push(Unreadable {
                    path: Some(match_name.to_string()),
                    match_name: Some(match_name.to_string()),
                    error: Some(why),
                    ..Unreadable::default()
                });
            }
        }
    }

    node.value = inside
        .iter()
        .find(|chunk| chunk.id == *b"cdat")
        .map(|chunk| doubles(chunk.body))
        .and_then(|raw| resolve(&raw, dimensions, colour, &scale));
    node
}

/// The per-axis multiplier that turns the file's stored numbers into the
/// numbers scripting reports.
///
/// Three separate conventions land here, and each of them was measured against
/// the golden capture rather than assumed:
///
/// - a **percent** property (Opacity, Scale, Mask Opacity) is a fraction on
///   disk;
/// - an **effect's point** is a fraction of the *layer's* frame (K-636). After
///   Effects runs an effect on the layer, so an effect's point is a point in
///   the layer's own raster and the file normalises it against that raster —
///   the same rule the anchor point and the mask path below already follow,
///   they being the only other normalised values in the format. Reading it
///   against the *composition* agrees exactly while a layer is the comp's size,
///   which is why it went unseen: put a 2560 × 1088 precomp in a 1920 × 816
///   comp and a Transform effect's Position arrives at 0.75 of where After
///   Effects has it while its Anchor Point — absent from the file, so written
///   in at the layer's centre by the mapping layer — arrives at the right
///   place, which shifts a picture nobody moved.
/// - an **anchor point** is a fraction of the *layer's source* — but only when
///   the layer has one. A shape, text or null layer stores it in raw pixels,
///   and scaling that by anything at all moves the layer's pivot.
fn scale_of(match_name: &str, colour: bool, dimensions: usize, ctx: Ctx<'_>) -> Vec<f64> {
    if PERCENT.contains(&match_name) {
        return vec![100.0; dimensions];
    }
    if match_name == "ADBE Anchor Point" && ctx.has_source {
        return vec![ctx.layer.0, ctx.layer.1, 1.0];
    }
    if ctx.in_effect && !colour && dimensions == 2 {
        // `layer` is the comp's size for a layer with no source of its own —
        // a shape, a text, a null — which is the frame After Effects draws
        // those at and exactly what `map::Conv::size` falls back to.
        return vec![ctx.layer.0, ctx.layer.1];
    }
    Vec::new()
}

/// One stored value, as the DOM reports it.
///
/// A colour is the odd one out: the file writes alpha first and every channel
/// in 0–255, while scripting hands back red first in 0–1.
fn resolve(
    raw: &[f64],
    dimensions: usize,
    colour: bool,
    scale: &[f64],
) -> Option<serde_json::Value> {
    let taken = raw.get(..dimensions)?;
    if colour && taken.len() == 4 {
        let channel = |at: usize| taken.get(at).copied().unwrap_or_default() / 255.0;
        return Some(json!([channel(1), channel(2), channel(3), channel(0)]));
    }
    let scaled: Vec<f64> = taken
        .iter()
        .enumerate()
        .map(|(axis, value)| value * scale.get(axis).copied().unwrap_or(1.0))
        .collect();
    if dimensions == 1 {
        return scaled.first().map(|value| json!(value));
    }
    Some(json!(scaled))
}

/// The `lhd3` header and the `ldat` records it describes, as an iterator over
/// borrowed record slices. `None` when there is no keyframe list at all.
fn records(inside: &[Chunk<'_>]) -> Option<(usize, usize, impl Iterator<Item = Vec<u8>>)> {
    let list = inside.iter().find(|chunk| chunk.is_list(b"list"))?;
    let inner: Vec<Chunk<'_>> = list.children().ok().collect();
    let header = inner.iter().find(|chunk| chunk.id == *b"lhd3")?;
    let count = usize::from(u16_at(header.body, lhd3::COUNT)?);
    let size = usize::from(u16_at(header.body, lhd3::ITEM_SIZE)?);
    let kind = usize::from(u8_at(header.body, lhd3::TYPE)?);
    let data: Vec<u8> = inner
        .iter()
        .find(|chunk| chunk.id == *b"ldat")
        .map_or_else(Vec::new, |chunk| chunk.body.to_vec());
    // The count and the size both come out of the file, so the product is
    // checked against the bytes that are actually there rather than trusted.
    let count = count.min(data.len().checked_div(size).unwrap_or_default());
    Some((
        kind,
        size,
        (0..count).map(move |index| {
            data.get(index * size..(index + 1) * size)
                .unwrap_or_default()
                .to_vec()
        }),
    ))
}

/// A keyframe's time, in comp seconds. The file counts internal timebase units
/// from the layer's start; scripting reports seconds from the comp's.
fn time_of(record: &[u8], ctx: Ctx<'_>) -> f64 {
    let units = f64::from(i32_at(record, 0).unwrap_or_default());
    let base = if ctx.timebase == 0.0 {
        1.0
    } else {
        ctx.timebase
    };
    ctx.start + units / base
}

/// Every keyframe of one property.
///
/// The record size is read from `lhd3` and paired with the list's type code to
/// choose the layout — never assumed, because the size *is* the class. A pair
/// no table knows is an error rather than a guess, and the caller falls the
/// property back to its static value with the class in the note.
fn keyframes(
    inside: &[Chunk<'_>],
    dimensions: usize,
    colour: bool,
    spatial: bool,
    scale: &[f64],
    ctx: Ctx<'_>,
) -> Result<Vec<Keyframe>, String> {
    let Some((kind, size, items)) = records(inside) else {
        return Err("the property is animated but carries no keyframe list".to_string());
    };

    let layout = match (kind, size) {
        (4, 152) => Layout::Colour,
        (4, 128) if spatial => Layout::Spatial(3),
        (4, 128) => Layout::Plain(3),
        (4, 104) => Layout::Spatial(2),
        (4, 88) => Layout::Plain(2),
        (4, 80) => Layout::Plain(1),
        (4, 64) => Layout::Valueless,
        (4, 48) => Layout::Plain(1),
        _ => {
            return Err(format!(
                "keyframe class {kind}/{size} is not decoded yet; the static value is used instead"
            ))
        }
    };

    Ok(items
        .map(|record| {
            let mut key = Keyframe {
                t: Some(time_of(&record, ctx)),
                in_interp: Some(enums::interpolation(u8_at(&record, 4).unwrap_or_default())),
                out_interp: Some(enums::interpolation(u8_at(&record, 5).unwrap_or_default())),
                ..Keyframe::default()
            };
            let temporal = u8_at(&record, 7).unwrap_or_default();
            key.roving = Some(bit(temporal, 5));
            key.auto_bezier = Some(bit(temporal, 4));
            key.continuous = Some(bit(temporal, 3));

            let payload = record.get(8..).unwrap_or_default();
            match layout {
                // Both of these begin with two doubles this reader does not
                // claim to understand, so the ease starts one double in.
                Layout::Colour => {
                    let values = doubles(payload);
                    ease_into(&mut key, values.get(1..).unwrap_or_default(), 1, &[]);
                    key.v = values
                        .get(6..10)
                        .and_then(|rgba| resolve(rgba, 4, true, &[]));
                }
                Layout::Valueless => {
                    let values = doubles(payload);
                    ease_into(&mut key, values.get(1..).unwrap_or_default(), 1, &[]);
                }
                Layout::Spatial(axes) => {
                    let values = doubles(payload.get(8..).unwrap_or_default());
                    // One ease for the whole property, which is what the DOM
                    // returns for a spatial one.
                    ease_into(&mut key, &values, 1, &[]);
                    let take = |from: usize| -> Option<Vec<f64>> {
                        Some(values.get(5 + from * axes..5 + (from + 1) * axes)?.to_vec())
                    };
                    key.v = take(0).and_then(|v| resolve(&v, axes, false, scale));
                    key.in_tangent = take(1);
                    key.out_tangent = take(2);
                    let flags = u8_at(payload, 3).unwrap_or_default();
                    key.spatial_auto_bezier = Some(bit(flags, 1));
                    key.spatial_continuous = Some(bit(flags, 0));
                }
                Layout::Plain(axes) => {
                    let values = doubles(payload);
                    key.v = values
                        .get(..axes)
                        .and_then(|v| resolve(v, dimensions.min(axes), colour, scale));
                    ease_into(&mut key, &values, axes, scale);
                }
            }
            key
        })
        .collect())
}

/// After Effects' default keyframe influence: a sixth of the segment.
const DEFAULT_INFLUENCE: f64 = 100.0 / 6.0;

/// Finish the ease the file only half-stores.
///
/// This is the one place the parser *computes* rather than reads, and it has to
/// be: After Effects writes zeros into the ease of a linear or held key and
/// works the numbers out when asked, so a straight copy of the bytes gives
/// every linear key a speed of nought — a curve that stands still where the
/// real one moves. The rule is the DOM's own, in three cases:
///
/// - **held**: no speed, the default influence;
/// - **linear**: the constant speed of the segment it joins — the slope
///   between this key and its neighbour, or nought at the ends and against a
///   held neighbour — with the default influence;
/// - **bezier**: the stored numbers, except at the two ends, where the outward
///   side has no segment and so no speed.
///
/// A spatial property's speed is the *length* of the velocity, one number for
/// the whole property; every other multi-dimensional one gets a number per
/// axis.
fn resolve_ease(keys: &mut [Keyframe], spatial: bool) {
    let times: Vec<f64> = keys.iter().map(|key| key.t.unwrap_or_default()).collect();
    let values: Vec<Vec<f64>> = keys.iter().map(|key| axes_of(key.v.as_ref())).collect();
    let interps: Vec<(String, String)> = keys
        .iter()
        .map(|key| {
            (
                key.in_interp.clone().unwrap_or_default(),
                key.out_interp.clone().unwrap_or_default(),
            )
        })
        .collect();

    for index in 0..keys.len() {
        for outgoing in [false, true] {
            let (mine, theirs) = if outgoing {
                (interps[index].1.as_str(), index.checked_add(1))
            } else {
                (interps[index].0.as_str(), index.checked_sub(1))
            };
            let neighbour = theirs.filter(|other| *other < keys.len());
            let width = |key: &Keyframe| {
                let side = if outgoing {
                    &key.out_ease
                } else {
                    &key.in_ease
                };
                side.as_deref().map_or(1, <[Ease]>::len).max(1)
            };
            let default = |speeds: Vec<f64>, count: usize| -> Vec<Ease> {
                (0..count.max(speeds.len()))
                    .map(|axis| Ease {
                        speed: Some(speeds.get(axis).copied().unwrap_or_default()),
                        influence: Some(DEFAULT_INFLUENCE),
                    })
                    .collect()
            };

            let replacement = match mine {
                "HOLD" => Some(default(Vec::new(), width(&keys[index]))),
                "LINEAR" => {
                    let held = neighbour.is_some_and(|other| {
                        let side = if outgoing {
                            &interps[other].0
                        } else {
                            &interps[other].1
                        };
                        side == "HOLD"
                    });
                    let speeds = match neighbour.filter(|_| !held) {
                        Some(other) => {
                            let (a, b) = if outgoing {
                                (index, other)
                            } else {
                                (other, index)
                            };
                            let axes = |at: usize| values.get(at).map(Vec::as_slice).unwrap_or(&[]);
                            let tangent = |at: usize, out: bool| {
                                let side = if out {
                                    &keys[at].out_tangent
                                } else {
                                    &keys[at].in_tangent
                                };
                                side.as_deref().unwrap_or_default()
                            };
                            segment_speed(
                                times[b] - times[a],
                                (axes(a), tangent(a, true)),
                                (axes(b), tangent(b, false)),
                                spatial,
                            )
                        }
                        None => Vec::new(),
                    };
                    Some(default(speeds, width(&keys[index])))
                }
                // A bezier key at either end of the run has no segment on its
                // outward side, so the stored speed there is not a speed.
                "BEZIER" if neighbour.is_none() => {
                    let side = if outgoing {
                        &keys[index].out_ease
                    } else {
                        &keys[index].in_ease
                    };
                    Some(
                        side.as_deref()
                            .unwrap_or_default()
                            .iter()
                            .map(|ease| Ease {
                                speed: Some(0.0),
                                influence: ease.influence,
                            })
                            .collect(),
                    )
                }
                _ => None,
            };

            if let Some(replacement) = replacement {
                if outgoing {
                    keys[index].out_ease = Some(replacement);
                } else {
                    keys[index].in_ease = Some(replacement);
                }
            }
        }
    }
}

/// The constant speed across one segment: per axis, or — for a spatial
/// property — the length of the path travelled, once, for the whole property.
///
/// The spatial case is the interesting one. After Effects does not measure the
/// straight line between two keys, it measures the **motion path**: the cubic
/// through the two keys' own spatial handles, which bows away from the straight
/// line whenever the handles are not flat. Measuring the chord instead reports a
/// layer as moving slower than it does — on the golden fixture, by up to 2.5%.
/// The curve is walked rather than solved, which is exact enough for a number
/// the DOM itself prints to fourteen figures and cheap enough for the handful of
/// keys a property has.
fn segment_speed(
    seconds: f64,
    from: (&[f64], &[f64]),
    to: (&[f64], &[f64]),
    spatial: bool,
) -> Vec<f64> {
    let (from, from_handle) = from;
    let (to, to_handle) = to;
    if seconds == 0.0 || from.is_empty() || from.len() != to.len() {
        return Vec::new();
    }
    if from.len() == 1 {
        let a = from.first().copied().unwrap_or_default();
        let b = to.first().copied().unwrap_or_default();
        return vec![(b - a) / seconds];
    }
    if spatial {
        return vec![path_length(from, from_handle, to, to_handle) / seconds];
    }
    from.iter()
        .zip(to)
        .map(|(a, b)| (b - a) / seconds)
        .collect()
}

/// How many straight pieces the motion path is measured in. Enough that the
/// answer agrees with After Effects' own to six significant figures on the
/// golden fixture, and fixed so the number is the same on every machine.
const PATH_STEPS: usize = 1024;

/// The length of the cubic between two keys, through their spatial handles. A
/// missing or mismatched handle falls back to the straight line, which is what
/// the curve becomes when both handles are flat anyway.
fn path_length(from: &[f64], from_handle: &[f64], to: &[f64], to_handle: &[f64]) -> f64 {
    let axes = from.len();
    let handles = from_handle.len() == axes && to_handle.len() == axes;
    let control = |base: &[f64], handle: &[f64], axis: usize| {
        base.get(axis).copied().unwrap_or_default()
            + if handles {
                handle.get(axis).copied().unwrap_or_default()
            } else {
                0.0
            }
    };
    let at = |t: f64, axis: usize| {
        let u = 1.0 - t;
        let p0 = from.get(axis).copied().unwrap_or_default();
        let p3 = to.get(axis).copied().unwrap_or_default();
        let p1 = control(from, from_handle, axis);
        let p2 = control(to, to_handle, axis);
        u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
    };

    let mut length = 0.0;
    let mut previous: Vec<f64> = (0..axes).map(|axis| at(0.0, axis)).collect();
    for step in 1..=PATH_STEPS {
        #[allow(clippy::cast_precision_loss)]
        let t = step as f64 / PATH_STEPS as f64;
        let point: Vec<f64> = (0..axes).map(|axis| at(t, axis)).collect();
        length += previous
            .iter()
            .zip(&point)
            .map(|(a, b)| (b - a) * (b - a))
            .sum::<f64>()
            .sqrt();
        previous = point;
    }
    length
}

/// A keyframe value read as a run of numbers — one for a scalar, several for a
/// vector, and none for a value that is not a number at all (a path).
fn axes_of(value: Option<&serde_json::Value>) -> Vec<f64> {
    match value {
        Some(serde_json::Value::Array(items)) => {
            items.iter().filter_map(serde_json::Value::as_f64).collect()
        }
        Some(other) => other.as_f64().into_iter().collect(),
        None => Vec::new(),
    }
}

/// Which shape a keyframe record takes, once its size has said so.
#[derive(Debug, Clone, Copy)]
enum Layout {
    /// R, G, B, A after the ease.
    Colour,
    /// Position and its friends: one ease for the property, then the value and
    /// two spatial tangents, `axes` wide each.
    Spatial(usize),
    /// The ordinary case: `axes` values, then `axes` of each ease number.
    Plain(usize),
    /// A key with no value of its own — a mask path's, whose value lives
    /// elsewhere, or a marker's.
    Valueless,
}

/// Fill in a key's temporal ease from a run of doubles laid out as
/// `value…, in speed…, in influence…, out speed…, out influence…`.
///
/// Influence is a fraction on disk and a percentage in the DOM. Speed is in the
/// property's own units per second on both sides, so it carries the same
/// multiplier the value does — a percent property's speed is percent per
/// second.
fn ease_into(key: &mut Keyframe, values: &[f64], axes: usize, scale: &[f64]) {
    let side = |from: usize| -> Vec<Ease> {
        (0..axes)
            .map(|axis| {
                let speed = values.get(axes * from + axis).copied().unwrap_or_default();
                let influence = values
                    .get(axes * (from + 1) + axis)
                    .copied()
                    .unwrap_or_default();
                Ease {
                    speed: Some(speed * scale.get(axis).copied().unwrap_or(1.0)),
                    influence: Some(influence * 100.0),
                }
            })
            .collect()
    };
    key.in_ease = Some(side(1));
    key.out_ease = Some(side(3));
}

/// A dimension-separated property keeps its animation on one follower per axis,
/// and the DOM reports the followers under the leader as well as beside it. So
/// once a group is read, a separated leader collects its followers.
fn fill_separated(properties: &mut [Property], separated: &[String]) {
    if separated.is_empty() {
        return;
    }
    let followers: Vec<(String, Property)> = properties
        .iter()
        .filter_map(|node| {
            let name = node.match_name.clone()?;
            name.rsplit_once('_')
                .filter(|(_, axis)| axis.len() == 1 && axis.chars().all(|c| c.is_ascii_digit()))
                .map(|(leader, _)| (leader.to_string(), node.clone()))
        })
        .collect();

    for node in properties.iter_mut() {
        let Some(name) = node.match_name.clone() else {
            continue;
        };
        if !separated.contains(&name) {
            continue;
        }
        let mine: Vec<Property> = followers
            .iter()
            .filter(|(leader, _)| *leader == name)
            .map(|(_, follower)| follower.clone())
            .collect();
        if mine.is_empty() {
            continue;
        }
        // Scripting does not report the leader's own keys once the dimensions
        // are separated: the followers are the animation, and reading the
        // leader is how a moving layer imports as a still one.
        node.keyframes = None;
        node.separated = Some(mine);
    }
}

/// A property's display name, or nothing when After Effects wrote its
/// "nobody renamed this" sentinel.
fn display_name(inside: &[Chunk<'_>]) -> Option<String> {
    let name = inside
        .iter()
        .find(|chunk| chunk.id == *b"tdsn")
        .and_then(|tdsn| tdsn.children().ok().find(|chunk| chunk.id == *b"Utf8"))
        .map(|chunk| chunk.text())?;
    (name != UNNAMED).then_some(name)
}

/// A group's on/off switch: `tdsb`'s fourth byte, bit 0. This is what carries
/// an effect instance's enabled state (docs/11 §2.2 item 9).
fn enabled_of(inside: &[Chunk<'_>]) -> bool {
    inside
        .iter()
        .find(|chunk| chunk.id == *b"tdsb")
        .and_then(|chunk| u8_at(chunk.body, 3))
        .is_some_and(|flags| bit(flags, 0))
}

/// A node this reader can name but not read, said the way the report says it.
fn unreadable_node(match_name: &str, why: &str) -> Property {
    Property {
        match_name: Some(match_name.to_string()),
        value_type: Some("other".to_string()),
        unreadable: Some(why.to_string()),
        ..Property::default()
    }
}

/// A run of big-endian f64s, as many as fit.
fn doubles(body: &[u8]) -> Vec<f64> {
    body.chunks_exact(8)
        .map(|raw| {
            let mut bytes = [0_u8; 8];
            bytes.copy_from_slice(raw);
            f64::from_be_bytes(bytes)
        })
        .collect()
}

/// Bytes as lower-case hex — how an undecoded blob reaches the capture.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0F)]));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
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

    /// **An effect's point is a fraction of the layer, not of the composition**
    /// (K-636).
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

    /// **A percentage is a fraction on disk, and a colour is A,R,G,B in
    /// 0–255.**
    ///
    /// The two conversions that go wrong in *plausible* ways rather than
    /// obvious ones: an opacity that imports as 1 instead of 100 looks like a
    /// nearly-invisible layer, and a colour read in the file's own channel
    /// order comes out with the alpha in the red channel — a picture that is
    /// wrong without ever looking broken.
    #[test]
    fn a_percentage_and_a_colour_arrive_in_the_units_the_dom_reports() {
        let mut runs = leaf(
            "ADBE Opacity",
            tdb4_record(1, false, false),
            cdat(&[0.5; 5]),
        );
        runs.extend(leaf(
            "ADBE Shadow Color",
            tdb4_record(4, true, false),
            cdat(&[255.0, 51.0, 102.0, 204.0, 0.0, 0.0]),
        ));
        let read = read(&group(runs));

        assert_eq!(read.properties.len(), 2);
        assert_eq!(read.properties[0].value_type.as_deref(), Some("float"));
        assert_eq!(read.properties[0].value, Some(json!(50.0)));
        assert_eq!(read.properties[1].value_type.as_deref(), Some("colour"));
        assert_eq!(
            read.properties[1].value,
            Some(json!([51.0 / 255.0, 102.0 / 255.0, 204.0 / 255.0, 1.0]))
        );
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

    /// **A group with no closing label still gives up its properties.**
    ///
    /// Damage at the end of a group must cost the tail, not the group: a
    /// truncated `tdgp` that swallowed its `ADBE Group End` still holds every
    /// property before the cut, and those are worth importing.
    #[test]
    fn a_group_missing_its_closing_label_keeps_the_properties_before_the_cut() {
        let mut inside = chunk(b"tdsb", &[0, 0, 0, 1]);
        inside.extend(leaf(
            "ADBE Rotate Z",
            tdb4_record(1, false, false),
            cdat(&[45.0; 5]),
        ));
        let read = read(&list(b"tdgp", &inside));

        assert_eq!(read.properties.len(), 1);
        assert_eq!(read.properties[0].value, Some(json!(45.0)));
    }

    /// **The same bytes read to the same tree, twice.**
    #[test]
    fn reading_a_property_tree_is_deterministic() {
        let mut runs = leaf(
            "ADBE Opacity",
            tdb4_record(1, false, false),
            cdat(&[0.25; 5]),
        );
        runs.extend(leaf(
            "ADBE Position",
            tdb4_record(3, false, false),
            cdat(&[1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        ));
        let bytes = group(runs);
        assert_eq!(read(&bytes).properties, read(&bytes).properties);
    }
}
