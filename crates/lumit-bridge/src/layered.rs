//! Turning a layered image file into a composition.
//!
//! # In plain terms
//!
//! A Photoshop document comes in as a folder of footage items, one per layer,
//! and a composition the size of the document holding a Footage layer for
//! each. Every layer is read at the document's size, so each one sits where it
//! sat in Photoshop with a centred transform and nothing else. A solid colour
//! fill with no mask becomes a Solid layer instead, and a layer keeps the
//! layer styles that Lumit also has. A vector mask the file has not already
//! drawn into the layer's picture becomes masks on the layer. An adjustment
//! layer becomes an Adjustment layer carrying the nearest effect, switched off.
//!
//! A group becomes a layer group where that changes nothing about the
//! picture: straight inside a composition, passing its layers' blend modes
//! through, at full opacity. Any other group becomes a composition of its own
//! and a Precomp layer, since layer groups do not nest and carry no opacity
//! or blend mode.
//!
//! An Illustrator document comes in the same way, with a Footage layer for
//! each of its top-level layers. A layer's own opacity and blend mode are
//! already in its picture, so every layer arrives plain.
//!
//! Pure, like `edits`: it returns ops and commits nothing.

use std::path::Path;

use lumit_core::anim::Property;
use lumit_core::group::LayerGroup;
use lumit_core::mask::{BezierPath, Mask, MaskMode, Vertex};
use lumit_core::model::{
    BlendMode, Composition, EffectInstance, EffectValue, Folder, FootageItem, Layer, LayerKind,
    LinearColour, MatteChannel, MatteRef, MediaRef, MotionBlur, ProjectItem, SolidDef,
};
use lumit_core::ops::Op;
use lumit_core::time::{Duration, FrameRate, Rational};
use lumit_media::ai::AiDocument;
use lumit_media::psd::{PsdDocument, PsdLayer, Section};
use uuid::Uuid;

/// Lumit's blend mode for one of Photoshop's four-letter keys. Dissolve, and
/// any key newer than this table, comes in as Normal.
fn blend_of(key: [u8; 4]) -> BlendMode {
    match &key {
        b"dark" => BlendMode::Darken,
        b"mul " => BlendMode::Multiply,
        b"idiv" => BlendMode::ColourBurn,
        b"lbrn" => BlendMode::LinearBurn,
        b"dkCl" => BlendMode::DarkerColour,
        b"lddg" => BlendMode::Add,
        b"lite" => BlendMode::Lighten,
        b"scrn" => BlendMode::Screen,
        b"div " => BlendMode::ColourDodge,
        b"lgCl" => BlendMode::LighterColour,
        b"over" => BlendMode::Overlay,
        b"sLit" => BlendMode::SoftLight,
        b"hLit" => BlendMode::HardLight,
        b"lLit" => BlendMode::LinearLight,
        b"vLit" => BlendMode::VividLight,
        b"pLit" => BlendMode::PinLight,
        b"hMix" => BlendMode::HardMix,
        b"diff" => BlendMode::Difference,
        b"smud" => BlendMode::Exclusion,
        b"fsub" => BlendMode::Subtract,
        b"fdiv" => BlendMode::Divide,
        b"hue " => BlendMode::Hue,
        b"sat " => BlendMode::Saturation,
        b"colr" => BlendMode::Colour,
        b"lum " => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

/// The same for the codes a layer style names its blend mode by.
fn blend_of_code(code: &str) -> BlendMode {
    match code {
        "Drkn" => BlendMode::Darken,
        "Mltp" => BlendMode::Multiply,
        "CBrn" => BlendMode::ColourBurn,
        "linearBurn" => BlendMode::LinearBurn,
        "darkerColor" => BlendMode::DarkerColour,
        "linearDodge" => BlendMode::Add,
        "Lghn" => BlendMode::Lighten,
        "Scrn" => BlendMode::Screen,
        "CDdg" => BlendMode::ColourDodge,
        "lighterColor" => BlendMode::LighterColour,
        "Ovrl" => BlendMode::Overlay,
        "SftL" => BlendMode::SoftLight,
        "HrdL" => BlendMode::HardLight,
        "linearLight" => BlendMode::LinearLight,
        "vividLight" => BlendMode::VividLight,
        "pinLight" => BlendMode::PinLight,
        "hardMix" => BlendMode::HardMix,
        "Dfrn" => BlendMode::Difference,
        "Xclu" => BlendMode::Exclusion,
        "blendSubtraction" => BlendMode::Subtract,
        "blendDivide" => BlendMode::Divide,
        "H   " => BlendMode::Hue,
        "Strt" => BlendMode::Saturation,
        "Clr " => BlendMode::Colour,
        "Lmns" => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

/// Photoshop's key for each layer style that comes across, and the style it
/// becomes.
const STYLES: [(&str, &str); 6] = [
    ("DrSh", "style_drop_shadow"),
    ("IrSh", "style_inner_shadow"),
    ("OrGl", "style_outer_glow"),
    ("IrGl", "style_inner_glow"),
    ("SoFi", "style_colour_overlay"),
    ("FrFX", "style_stroke"),
];

/// A style's rows that are plain numbers, and Photoshop's key for each. Sizes
/// are in pixels and the rest in per cent on both sides.
const ROWS: [(&str, &str); 6] = [
    ("opacity", "Opct"),
    ("distance", "Dstn"),
    ("softness", "blur"),
    ("spread", "Ckmt"),
    ("choke", "Ckmt"),
    ("size", "Sz  "),
];

/// An sRGB colour from 0 to 255 as the linear one Lumit keeps.
fn linear(colour: [f64; 3]) -> [f32; 3] {
    colour.map(|c| lumit_core::pixels::srgb_decode(c.round() as u8))
}

/// A Lab colour as Photoshop writes one, under D50 light, as scene-linear
/// sRGB. A colour sRGB can't hold is cut to the nearest it can.
fn lab_to_linear(l: f64, a: f64, b: f64) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let unbend = |f: f64| {
        if f > 6.0 / 29.0 {
            f * f * f
        } else {
            3.0 * (6.0_f64 / 29.0).powi(2) * (f - 4.0 / 29.0)
        }
    };
    let (x, y, z) = (
        0.964_22 * unbend(fy + a / 500.0),
        unbend(fy),
        0.825_21 * unbend(fy - b / 200.0),
    );
    // From D50 to sRGB's own white and primaries, in one matrix.
    [
        3.133_856_1 * x - 1.616_866_7 * y - 0.490_614_6 * z,
        -0.978_768_4 * x + 1.916_141_5 * y + 0.033_454 * z,
        0.071_945_3 * x - 0.228_991_4 * y + 1.405_242_7 * z,
    ]
    .map(|c| c.clamp(0.0, 1.0) as f32)
}

/// The layer styles `record` wears, as Lumit's own.
// ponytail: one of each style, one colour each. A second shadow, a gradient,
// a contour, noise and an outer style's blend mode are not carried, and
// gradient overlay, pattern overlay, satin and bevel are left out. The
// upgrade is a row for each on the Lumit style.
fn styles(psd: &PsdDocument, record: &PsdLayer) -> Vec<EffectInstance> {
    let mut out = Vec::new();
    if record.number("lfx2/masterFXSwitch") == Some(0.0) {
        return out;
    }
    for (key, name) in STYLES {
        let number = |leaf: &str| record.number(&format!("lfx2/{key}/{leaf}"));
        let code = |leaf: &str| record.code(&format!("lfx2/{key}/{leaf}"));
        // A glow or a stroke painted with a gradient has no one colour.
        let (Some(red), Some(green), Some(blue)) = (
            number("Clr /Rd  "),
            number("Clr /Grn "),
            number("Clr /Bl  "),
        ) else {
            continue;
        };
        if number("enab") != Some(1.0) || code("PntT").is_some_and(|paint| paint != "SClr") {
            continue;
        }
        let Some(mut style) = lumit_core::fx::instantiate(name) else {
            continue;
        };
        // Photoshop measures the light anticlockwise from the right, and a
        // shadow falls away from it. Lumit measures where the shadow falls,
        // clockwise from straight up.
        let light = match number("uglg") {
            Some(shared) if shared != 0.0 => psd.global_angle.or(number("lagl")),
            _ => number("lagl"),
        };
        for param in &mut style.params {
            param.value = match (param.id.as_str(), &param.value) {
                (_, EffectValue::Colour(_)) => {
                    let [r, g, b] = linear([red, green, blue]);
                    EffectValue::Colour([r, g, b, 1.0].map(|c| Property::fixed(f64::from(c))))
                }
                ("direction", _) => match light {
                    Some(light) => EffectValue::Float(Property::fixed(270.0 - light)),
                    None => continue,
                },
                ("knockout", _) => EffectValue::Bool(number("layerConceals") != Some(0.0)),
                ("position", _) => EffectValue::Choice(match code("Styl") {
                    Some("InsF") => 1,
                    Some("CtrF") => 2,
                    _ => 0,
                }),
                ("source", _) => EffectValue::Choice(u32::from(code("glwS") == Some("SrcC"))),
                // A style drawn under the layer has nothing to blend with.
                (lumit_core::fx::BLEND_PARAM, _) if !lumit_core::fx::style_is_outer(name) => {
                    let mode = blend_of_code(code("Md  ").unwrap_or_default());
                    let index = BlendMode::ALL.iter().position(|b| *b == mode);
                    EffectValue::Choice(index.unwrap_or(0) as u32)
                }
                // An overlay has no opacity of its own. Its Mix row is it.
                ("mix", _) if key == "SoFi" => match number("Opct") {
                    Some(opacity) => EffectValue::Float(Property::fixed(opacity)),
                    None => continue,
                },
                (id, _) => {
                    let leaf = ROWS.iter().find(|(row, _)| *row == id);
                    match leaf.and_then(|(_, leaf)| number(leaf)) {
                        Some(value) => EffectValue::Float(Property::fixed(value)),
                        None => continue,
                    }
                }
            };
        }
        out.push(style);
    }
    lumit_core::fx::normalise_styles(&mut out);
    out
}

/// The masks `record`'s vector mask becomes: one for each subpath, in the
/// file's order. A layer is read at the document's size, so a point's place in
/// the document is its place on the layer.
// ponytail: the mask's feather and density are not carried, and a subpath
// that crosses itself fills the way Lumit fills any path. The upgrade is
// reading the first two from the layer's mask data.
fn masks(psd: &PsdDocument, record: &PsdLayer) -> Vec<Mask> {
    let Some(vector) = record.vector_mask() else {
        return Vec::new();
    };
    let place = |(x, y): (f64, f64)| (x * f64::from(psd.width), y * f64::from(psd.height));
    let subpaths = vector.paths.iter().enumerate();
    subpaths
        .map(|(index, subpath)| {
            let vertices = subpath.knots.iter().map(|[before, point, after]| {
                let (pos, before, after) = (place(*point), place(*before), place(*after));
                Vertex {
                    pos,
                    tan_in: (before.0 - pos.0, before.1 - pos.1),
                    tan_out: (after.0 - pos.0, after.1 - pos.1),
                }
            });
            // The first subpath is the shape itself, unless it is cut out of
            // the whole layer.
            let mode = match (index, subpath.operation) {
                (_, 2) => MaskMode::Subtract,
                (0, _) => MaskMode::Add,
                (_, 3) => MaskMode::Intersect,
                (_, 0) => MaskMode::Difference,
                _ => MaskMode::Add,
            };
            // An inverted mask is the same list with every step turned over.
            let (mode, inverted) = match mode {
                _ if !vector.inverted => (mode, false),
                MaskMode::Add => (MaskMode::Intersect, true),
                MaskMode::Intersect => (MaskMode::Add, true),
                MaskMode::Subtract => (MaskMode::Add, false),
                _ => (mode, false),
            };
            Mask {
                id: Uuid::now_v7(),
                name: record.name.clone(),
                path: BezierPath {
                    vertices: vertices.collect(),
                    closed: true,
                },
                path_keys: Vec::new(),
                inverted,
                opacity: Property::fixed(100.0),
                mode,
                feather: Property::zero(),
                vertex_feather: Vec::new(),
                expansion: Property::zero(),
                extra: serde_json::Map::new(),
            }
        })
        .collect()
}

/// The Lumit effects a Photoshop adjustment layer becomes, or `None` for a
/// kind that is not mapped.
///
/// Each number is carried the way the After Effects import carries the same
/// effect's, where it has one. Photoshop adjusts the encoded picture and Lumit
/// works in linear light, so the result is near Photoshop's and not the same.
// ponytail: Invert, Posterize, Threshold, Brightness/Contrast, Exposure,
// Hue/Saturation, Levels, Curves, Black and White, Vibrance and Colour
// Balance. Exposure's offset and gamma, a colourised Hue/Saturation, a range's
// own edges, a curve of more than sixteen points, a negative Vibrance and
// Colour Balance's preserve luminosity are not carried. Every other kind is
// left out, and the upgrade is an arm here for each.
fn adjustment(record: &PsdLayer) -> Option<Vec<EffectInstance>> {
    let word = |path: &str| record.number(path);
    let colour = |[r, g, b]: [f32; 3]| {
        EffectValue::Colour([r, g, b, 1.0].map(|c| Property::fixed(f64::from(c))))
    };
    let mut rows: Vec<(String, f64)> = Vec::new();
    // The rows that are not plain numbers, and a second effect to carry.
    let mut others: Vec<(&str, EffectValue)> = Vec::new();
    let mut second = None;
    let (vibrance, saturation) = (word("vibA/vibrance"), word("vibA/Strt"));
    let name = if word("nvrt").is_some() {
        "invert"
    } else if let Some(levels) = word("post/0") {
        rows.push(("levels".into(), levels));
        "posterize"
    } else if let Some(level) = word("thrs/0") {
        // Photoshop counts the level from 0 to 255 and Lumit in per cent.
        rows.push(("level".into(), level * 100.0 / 255.0));
        "threshold"
    } else if word("brit").is_some() {
        // A newer Photoshop keeps the real numbers in a block beside the old
        // one, and leaves the old one at zero.
        rows.push(("brightness".into(), word("CgEd/Brgh").or(word("brit/0"))?));
        rows.push(("contrast".into(), word("CgEd/Cntr").or(word("brit/1"))?));
        "brightness"
    } else if let Some(stops) = word("expA/0") {
        rows.push(("stops".into(), stops));
        "exposure"
    } else if word("hue2/0") == Some(2.0) {
        // Colourise throws the picture's own hue away, which no row here does.
        if word("hue2/1") != Some(0.0) {
            return None;
        }
        let ranges = [
            "master", "reds", "yellows", "greens", "cyans", "blues", "magentas",
        ];
        for (place, range) in ranges.into_iter().enumerate() {
            // Master's three numbers start at the sixth. Each range is seven
            // numbers: its four edges, then its three.
            let at = 5 + place * 7;
            for (i, row) in ["hue", "saturation", "lightness"].into_iter().enumerate() {
                rows.push((format!("{range}_{row}"), word(&format!("hue2/{}", at + i))?));
            }
        }
        "hue_saturation"
    } else if word("levl/0") == Some(2.0) {
        // Five numbers a channel: the four levels from 0 to 255, then gamma
        // in hundredths.
        let levels = [
            ("in_black", 255.0),
            ("in_white", 255.0),
            ("out_black", 255.0),
            ("out_white", 255.0),
            ("gamma", 100.0),
        ];
        for (place, channel) in ["master", "red", "green", "blue"].into_iter().enumerate() {
            for (i, (row, full)) in levels.into_iter().enumerate() {
                let stored = word(&format!("levl/{}", 1 + place * 5 + i))?;
                rows.push((format!("{channel}_{row}"), stored / full));
            }
        }
        "levels"
    } else if word("curv/0") == Some(1.0) {
        // A bit for each curve that is there: composite, red, green, blue.
        // Each is a count and that many points from 0 to 255, the output
        // before the input.
        let there = word("curv/2")? as u32;
        let mut at = 3;
        for (bit, channel) in ["master", "red", "green", "blue"].into_iter().enumerate() {
            if there & (1 << bit) == 0 {
                continue;
            }
            // Photoshop's curve holds up to nineteen points and Lumit's
            // sixteen.
            let count = word(&format!("curv/{at}"))? as usize;
            if !(2..=lumit_core::fx::CURVE_MAX_POINTS).contains(&count) {
                return None;
            }
            let mut points = Vec::new();
            for i in (at + 1..).step_by(2).take(count) {
                let point = [
                    word(&format!("curv/{}", i + 1))?,
                    word(&format!("curv/{i}"))?,
                ];
                if point.iter().any(|v| !(0.0..=255.0).contains(v)) {
                    return None;
                }
                points.push(point.map(|v| (v / 255.0) as f32));
            }
            at += 1 + count * 2;
            others.push((channel, EffectValue::Curve(points)));
        }
        "curves"
    } else if word("blwh/Rd  ").is_some() {
        let weights = [
            ("reds", "Rd  "),
            ("yellows", "Yllw"),
            ("greens", "Grn "),
            ("cyans", "Cyn "),
            ("blues", "Bl  "),
            ("magentas", "Mgnt"),
        ];
        for (row, key) in weights {
            rows.push((row.into(), word(&format!("blwh/{key}"))?));
        }
        let tint = word("blwh/useTint") == Some(1.0);
        others.push(("tint", EffectValue::Bool(tint)));
        match ["Rd  ", "Grn ", "Bl  "].map(|key| word(&format!("blwh/tintColor/{key}"))) {
            [Some(red), Some(green), Some(blue)] => {
                others.push(("tint_colour", colour(linear([red, green, blue]))));
            }
            // A tint whose colour is not red, green and blue is not read.
            _ if tint => return None,
            _ => {}
        }
        "black_and_white"
    } else if vibrance.is_some() || saturation.is_some() {
        // Vibrancy only lifts, so a negative Vibrance has no row to land on.
        let vibrance = vibrance.unwrap_or(0.0);
        if vibrance < 0.0 {
            return None;
        }
        rows.push(("amount".into(), vibrance));
        // Photoshop counts Saturation from no change and Lumit from 100.
        if let Some(saturation) = saturation.filter(|s| *s != 0.0) {
            rows.push(("saturation".into(), 100.0 + saturation));
            second = Some("saturation");
        }
        "vibrancy"
    } else if word("blnc").is_some() {
        // Three numbers each for shadows, midtones and highlights, from -100
        // to 100, towards red, green and blue. Lumit grades with lift, gamma
        // and gain, so a full slider is taken as a tenth of lift, or a stop of
        // gamma or gain.
        for (place, row) in ["lift", "gamma", "gain"].into_iter().enumerate() {
            let mut rgb = [0.0; 3];
            for (i, channel) in rgb.iter_mut().enumerate() {
                let slider = word(&format!("blnc/{}", place * 3 + i))?;
                if !(-100.0..=100.0).contains(&slider) {
                    return None;
                }
                *channel = match row {
                    "lift" => slider / 1000.0,
                    _ => (slider / 100.0).exp2(),
                } as f32;
            }
            others.push((row, colour(rgb)));
        }
        "colour_balance"
    } else if word("phfl").is_some() {
        // A version, the glass's colour, the density in per cent as two
        // words, and a byte for Preserve luminosity. Only the older layout
        // says how its colour is written, as a colour space and four numbers,
        // and Photoshop's own filters are in Lab there. The newer layout is
        // three numbers with no scale written down anywhere, so it is left
        // out.
        let part = |i: usize| word(&format!("phfl/{i}"));
        if part(0)? != 2.0 || part(1)? != 7.0 {
            return None;
        }
        let density = part(6)? * 65536.0 + part(7)?.rem_euclid(65536.0);
        if !(0.0..=100.0).contains(&density) {
            return None;
        }
        rows.push(("density".into(), density));
        let glass = lab_to_linear(part(2)? / 100.0, part(3)? / 100.0, part(4)? / 100.0);
        others.push(("colour", colour(glass)));
        others.push((
            "filter",
            EffectValue::Choice(lumit_core::fx::effects::photo_filter::PhotoFilter::CUSTOM),
        ));
        // The byte is the top half of a ninth word. A block too short to hold
        // it keeps Photoshop's default, which is on.
        let keep = part(8).is_none_or(|byte| byte != 0.0);
        others.push(("preserve_luminosity", EffectValue::Bool(keep)));
        "photo_filter"
    } else {
        return None;
    };
    let mut effects = vec![lumit_core::fx::instantiate(name)?];
    if let Some(second) = second {
        effects.push(lumit_core::fx::instantiate(second)?);
    }
    let rows = rows
        .into_iter()
        .map(|(id, value)| (id, EffectValue::Float(Property::fixed(value))))
        .chain(others.into_iter().map(|(id, value)| (id.to_owned(), value)));
    for (id, value) in rows {
        let mut params = effects.iter_mut().flat_map(|e| &mut e.params);
        if let Some(param) = params.find(|p| p.id == id) {
            param.value = value;
        }
    }
    Some(effects)
}

/// A layered file that imports as a composition.
pub(crate) enum Layered {
    Psd(PsdDocument),
    Ai(AiDocument),
}

impl Layered {
    /// `None` unless `path` is a layered file of a kind that is read, holding
    /// two or more layers with a picture.
    pub(crate) fn open(path: &Path) -> Option<Self> {
        if lumit_media::psd::is_psd(path) {
            let psd = lumit_media::psd::open(path).ok()?;
            let pictures = psd.layers.iter().filter(|l| l.has_pixels()).count();
            return (pictures >= 2).then_some(Self::Psd(psd));
        }
        if lumit_media::ai::is_ai(path) {
            let ai = lumit_media::ai::open(path).ok()?;
            return (ai.layers.len() >= 2).then_some(Self::Ai(ai));
        }
        None
    }

    /// The document's size in pixels.
    pub(crate) fn size(&self) -> (u32, u32) {
        match self {
            Self::Psd(psd) => (psd.width, psd.height),
            Self::Ai(ai) => (ai.width, ai.height),
        }
    }

    /// The ops that fill `comp` with the file's layers, and how many layers
    /// were left out because they hold no picture.
    ///
    /// `comp` is new and empty. `comp_size`, `rate` and `out` are its size,
    /// frame rate and length, and `first_index` is where the next project
    /// item goes.
    pub(crate) fn ops(
        &self,
        path: &Path,
        comp: Uuid,
        comp_size: (u32, u32),
        rate: FrameRate,
        out: Rational,
        first_index: usize,
    ) -> (Vec<Op>, u32) {
        match self {
            Self::Psd(psd) => psd_ops(psd, path, comp, comp_size, rate, out, first_index),
            Self::Ai(ai) => (ai_ops(ai, path, comp, comp_size, out, first_index), 0),
        }
    }
}

/// The item that reads one layer of the file at `path` as a picture.
fn layer_item(path: &Path, id: Uuid, index: usize, name: &str) -> ProjectItem {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    ProjectItem::Footage(FootageItem {
        id,
        name: format!("{name}/{file_name}"),
        media: MediaRef {
            relative_path: file_name,
            absolute_path: path.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        colour_space: None,
        sequence: None,
        source_layer: u32::try_from(index).ok(),
        extra: serde_json::Map::new(),
    })
}

/// The ops that add `items`, file them in a folder named for the file at
/// `path`, and fill `comp` with `layers` and `groups`.
fn filed(
    path: &Path,
    items: Vec<ProjectItem>,
    layers: Vec<Layer>,
    groups: Vec<LayerGroup>,
    comp: Uuid,
    first_index: usize,
) -> Vec<Op> {
    let stem = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut ops = Vec::new();
    let folder = Uuid::now_v7();
    let children: Vec<Uuid> = items.iter().map(ProjectItem::id).collect();
    for (at, item) in items.into_iter().enumerate() {
        ops.push(Op::AddItem {
            index: first_index + at,
            item: Box::new(item),
        });
    }
    ops.push(Op::AddItem {
        index: first_index + children.len(),
        item: Box::new(ProjectItem::Folder(Folder {
            id: folder,
            name: format!("{stem} layers"),
            children: Vec::new(),
            extra: serde_json::Map::new(),
        })),
    });
    ops.push(Op::SetFolderChildren { folder, children });
    for (index, layer) in layers.into_iter().enumerate() {
        ops.push(Op::AddLayer {
            comp,
            index,
            layer: Box::new(layer),
        });
    }
    for (index, group) in groups.into_iter().enumerate() {
        ops.push(Op::GroupLayers {
            comp,
            index,
            group: Box::new(group),
        });
    }
    ops
}

/// The ops that fill `comp` with `ai`'s layers: a Footage layer for each
/// top-level layer, hidden where Illustrator had it switched off.
// ponytail: a layer is read as pixels at the artboard's size, so one scaled
// past 100% goes soft. The upgrade is drawing it at the size the comp shows it.
fn ai_ops(
    ai: &AiDocument,
    path: &Path,
    comp: Uuid,
    comp_size: (u32, u32),
    out: Rational,
    first_index: usize,
) -> Vec<Op> {
    let mut items = Vec::new();
    let mut layers = Vec::new();
    // The file lists its layers bottom first, and a comp lists them top first.
    for (index, record) in ai.layers.iter().enumerate().rev() {
        let id = Uuid::now_v7();
        items.push(layer_item(path, id, index, &record.name));
        let (width, height) = (f64::from(ai.width), f64::from(ai.height));
        let mut layer = crate::edits::base_layer(
            record.name.clone(),
            LayerKind::Footage { item: id },
            out,
            crate::edits::centred_transform(width, height, comp_size.0, comp_size.1),
        );
        layer.switches.visible = record.visible;
        layers.push(layer);
    }
    filed(path, items, layers, Vec::new(), comp, first_index)
}

/// Somewhere layers are being gathered: the document, or a group that becomes
/// a composition of its own.
struct Scope<'a> {
    /// The group's header. `None` for the document.
    header: Option<&'a PsdLayer>,
    layers: Vec<Layer>,
    groups: Vec<LayerGroup>,
    /// The layer group being filled, and whether Photoshop had it switched on.
    fold: Option<(LayerGroup, bool)>,
}

impl Scope<'_> {
    fn place(&mut self, mut layer: Layer) {
        if let Some((group, visible)) = &mut self.fold {
            group.members.push(layer.id);
            layer.switches.visible &= *visible;
        }
        self.layers.push(layer);
    }
}

/// How many groups may sit inside each other and each still become a
/// composition of its own. A group past that is read as plain layers of the
/// one it is in. Every composition inside another is a level the renderer
/// walks down by calling itself, and a file can hold thousands of groups.
const MAX_GROUP_DEPTH: usize = 16;

/// The ops that fill `comp` with `psd`'s layers, and how many records were
/// left out because they hold no picture: an adjustment layer of a kind that
/// is not mapped, a gradient or pattern fill, an empty layer.
///
/// `comp` is new and empty. `comp_size`, `rate` and `out` are its size, frame
/// rate and length, and `first_index` is where the next project item goes.
// ponytail: a group's own mask and layer styles are dropped, and a layer
// clipped to a group is not clipped. A matte on the Lumit layer is the upgrade
// for the mask and the clip.
fn psd_ops(
    psd: &PsdDocument,
    path: &Path,
    comp: Uuid,
    comp_size: (u32, u32),
    rate: FrameRate,
    out: Rational,
    first_index: usize,
) -> (Vec<Op>, u32) {
    // The item that reads one record of the file as a picture.
    let footage =
        |id: Uuid, index: usize, record: &PsdLayer| layer_item(path, id, index, &record.name);
    let centred = |width: u32, height: u32| {
        let (width, height) = (f64::from(width), f64::from(height));
        crate::edits::centred_transform(width, height, comp_size.0, comp_size.1)
    };

    let mut items = Vec::new();
    let mut scopes = vec![Scope {
        header: None,
        layers: Vec::new(),
        groups: Vec::new(),
        fold: None,
    }];
    let mut left_out = 0u32;
    // Groups opened past [`MAX_GROUP_DEPTH`], which are read as plain layers.
    let mut flat = 0u32;

    // The file lists its layers bottom first, and a comp lists them top first.
    for (index, record) in psd.layers.iter().enumerate().rev() {
        let depth = scopes.len();
        let Some(scope) = scopes.last_mut() else {
            break;
        };
        match record.section {
            // A layer group never changes the picture, so it stands in for a
            // group only where the group did not either.
            Section::Group
                if scope.fold.is_none()
                    && record.blend == *b"pass"
                    && record.opacity == 255
                    && record.fill_opacity == 255 =>
            {
                let group = LayerGroup {
                    id: Uuid::now_v7(),
                    name: record.name.clone(),
                    label: 0,
                    members: Vec::new(),
                    effects: Vec::new(),
                };
                scope.fold = Some((group, record.visible));
            }
            Section::Group if depth > MAX_GROUP_DEPTH => flat += 1,
            Section::Group => scopes.push(Scope {
                header: Some(record),
                layers: Vec::new(),
                groups: Vec::new(),
                fold: None,
            }),
            Section::GroupEnd if flat > 0 => flat -= 1,
            Section::GroupEnd => {
                close(&mut scopes, &mut items, comp_size, rate, out);
            }
            Section::Layer if !record.has_pixels() => {
                let Some(effects) = adjustment(record) else {
                    left_out += 1;
                    continue;
                };
                let mut layer = crate::edits::base_layer(
                    record.name.clone(),
                    LayerKind::Adjustment,
                    out,
                    centred(comp_size.0, comp_size.1),
                );
                dress(&mut layer, record);
                layer.effects = effects;
                // It arrives switched off. Photoshop adjusts the encoded
                // picture and Lumit works in linear light, so the same numbers
                // give a different picture, and the real file drew further
                // from Photoshop's with them on than with them left out.
                layer.switches.visible = false;
                if !record.clipped && !record.has_mask() {
                    scope.place(layer);
                    continue;
                }
                // A mask or a clip says where the adjustment acts. The record
                // reads as that picture, on a hidden layer the adjustment
                // takes as its matte.
                let id = Uuid::now_v7();
                items.push(footage(id, index, record));
                let mut matte = crate::edits::base_layer(
                    record.name.clone(),
                    LayerKind::Footage { item: id },
                    out,
                    centred(psd.width, psd.height),
                );
                matte.switches.visible = false;
                layer.matte = Some(MatteRef {
                    layer: matte.id,
                    channel: MatteChannel::Alpha,
                    inverted: false,
                    source: Default::default(),
                });
                scope.place(layer);
                scope.place(matte);
            }
            Section::Layer => {
                let id = Uuid::now_v7();
                let kind = match record.fill() {
                    // A fill with nothing hiding any of it stays a colour that
                    // can be changed. Any other is read as a picture.
                    Some(colour) if !record.clipped && !record.has_mask() => {
                        let [r, g, b] = linear(colour);
                        items.push(ProjectItem::Solid(SolidDef {
                            id,
                            name: record.name.clone(),
                            colour: LinearColour([r, g, b, 1.0]),
                            width: psd.width,
                            height: psd.height,
                            extra: serde_json::Map::new(),
                        }));
                        LayerKind::Solid { def: id }
                    }
                    _ => {
                        items.push(footage(id, index, record));
                        LayerKind::Footage { item: id }
                    }
                };
                let mut layer = crate::edits::base_layer(
                    record.name.clone(),
                    kind,
                    out,
                    centred(psd.width, psd.height),
                );
                layer.styles = styles(psd, record);
                layer.masks = masks(psd, record);
                dress(&mut layer, record);
                scope.place(layer);
            }
        }
    }
    // A file that ends inside a group still closes it.
    while close(&mut scopes, &mut items, comp_size, rate, out) {}
    let (layers, groups) = scopes
        .pop()
        .map(|root| (root.layers, root.groups))
        .unwrap_or_default();
    let ops = filed(path, items, layers, groups, comp, first_index);
    (ops, left_out)
}

/// A record's opacity, blend mode and visibility, onto the layer made for it.
fn dress(layer: &mut Layer, record: &PsdLayer) {
    // Photoshop's Fill fades a layer's picture and leaves its styles alone.
    // Lumit has one opacity for both, so Fill is carried where no style came
    // across, and there it is one more opacity.
    // ponytail: a layer that wears a style keeps its picture at full Fill.
    // The upgrade is a Fill row on the layer.
    let fill = if layer.styles.is_empty() {
        f64::from(record.fill_opacity) / 255.0
    } else {
        1.0
    };
    layer.transform.opacity = Property::fixed(f64::from(record.opacity) / 2.55 * fill);
    layer.blend = blend_of(record.blend);
    layer.switches.visible = record.visible;
}

/// Close the innermost open group: finish the layer group being filled, or
/// turn the scope into a composition and place a Precomp layer of it in the
/// scope around it. False when no group is open.
fn close(
    scopes: &mut Vec<Scope<'_>>,
    items: &mut Vec<ProjectItem>,
    size: (u32, u32),
    rate: FrameRate,
    out: Rational,
) -> bool {
    let Some(top) = scopes.last_mut() else {
        return false;
    };
    if let Some((group, _)) = top.fold.take() {
        // A group whose every member was left out has nothing to hold.
        if !group.members.is_empty() {
            top.groups.push(group);
        }
        return true;
    }
    let Some(header) = top.header else {
        return false;
    };
    let (Some(scope), Some(around)) = (scopes.pop(), scopes.last_mut()) else {
        return false;
    };
    if scope.layers.is_empty() {
        return true;
    }
    let inner = Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: scope.groups,
        beat_grid: None,
        id: Uuid::now_v7(),
        name: header.name.clone(),
        width: size.0,
        height: size.1,
        frame_rate: rate,
        duration: Duration(out),
        background: LinearColour::BLACK,
        work_area: None,
        layers: scope.layers,
        markers: Vec::new(),
        motion_blur: MotionBlur::default(),
        extra: serde_json::Map::new(),
    };
    let mut layer = crate::edits::base_layer(
        header.name.clone(),
        LayerKind::Precomp { comp: inner.id },
        out,
        crate::edits::centred_transform(f64::from(size.0), f64::from(size.1), size.0, size.1),
    );
    dress(&mut layer, header);
    // A collapsed Precomp layer lets its layers blend with what is under it,
    // which is what a group with no blend mode of its own does.
    layer.switches.collapse = header.blend == *b"pass";
    items.push(ProjectItem::Composition(inner));
    around.place(layer);
    true
}
