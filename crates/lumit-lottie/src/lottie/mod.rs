//! The Lottie emitter: a comp as Lottie JSON, or as a dotLottie zip.
//!
//! # In plain terms
//!
//! The comp's layers go out top first, each with its transform, masks,
//! matte, blend and the effects a player knows, and its own content: shape
//! art as shapes, a solid as a solid, a still as an image, a precomp as a
//! comp of its own in the file's assets. Keys keep their easing exactly;
//! expressions are baked into keys. What can't go into the file is said in
//! the check ([`crate::check`]) and the export carries on.
//!
//! The JSON is built as a tree and written compact in one go, with every
//! number rounded for what it is ([`value::Fmt`]).
//!
//! Runs on the caller's thread (a bridge worker or the command line's).

use std::sync::Arc;

use lumit_core::model::{Composition, Document};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::check::{Place, Report};
use crate::settings::{Format, Settings};
use crate::time::Timebase;
use crate::{walk, Exported, Host, LottieError};

mod assets;
mod dotlottie;
mod effects;
mod layers;
mod shapes;
mod text;
mod value;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

use value::{Class, Fmt};

/// The Lottie version the file says it is: new enough that lottie-web runs
/// none of its fixes for older files.
const VERSION: &str = "5.12.2";

/// Everything one export carries from layer to layer.
pub(crate) struct Ctx<'a> {
    pub doc: &'a Arc<Document>,
    pub settings: &'a Settings,
    pub host: &'a dyn Host,
    pub fmt: Fmt,
    /// The output's clock, which every comp in the file shares.
    pub root: Timebase,
    pub report: Report,
    /// The file's assets, in the order they were first needed.
    pub assets: Vec<Value>,
    /// Footage items already made assets, with the asset's id (`None` when
    /// it couldn't be).
    pub images: Vec<(Uuid, Option<String>)>,
    /// Comps already made assets, with the asset's id.
    pub comps: Vec<(Uuid, String)>,
    /// The comps being written now, outermost first, so a comp that
    /// contains itself stops.
    pub building: Vec<Uuid>,
    /// Pictures that go beside the file or into the zip, by file name.
    pub files: Vec<(String, Vec<u8>)>,
    /// Fonts a text layer names, as `fonts.list` entries.
    pub fonts: Vec<Value>,
}

/// `comp` as a Lottie or a dotLottie.
pub fn export(
    doc: &Arc<Document>,
    comp: &Composition,
    settings: &Settings,
    host: &dyn Host,
) -> Result<Exported, LottieError> {
    let root = Timebase::of(comp, settings);
    let fmt = Fmt {
        minify: settings.minify,
    };
    let mut cx = Ctx {
        doc,
        settings,
        host,
        fmt,
        root,
        report: Report::new(settings.players),
        assets: Vec::new(),
        images: Vec::new(),
        comps: Vec::new(),
        building: vec![comp.id],
        files: Vec::new(),
        fonts: Vec::new(),
    };
    let at = Place::comp(comp.id, &comp.name);
    let layers = layers(&mut cx, comp, &root, &at)?;

    let mut top = Map::new();
    top.insert("v".into(), Value::from(VERSION));
    top.insert("fr".into(), fmt.num(root.fps, Class::Time));
    top.insert("ip".into(), fmt.num(root.in_frame(), Class::Time));
    top.insert("op".into(), fmt.num(root.out_frame(), Class::Time));
    top.insert("w".into(), Value::from(comp.width));
    top.insert("h".into(), Value::from(comp.height));
    fmt.name(&mut top, &comp.name);
    top.insert("ddd".into(), Value::from(0));
    top.insert(
        "assets".into(),
        Value::Array(std::mem::take(&mut cx.assets)),
    );
    if !cx.fonts.is_empty() {
        top.insert(
            "fonts".into(),
            json!({"list": std::mem::take(&mut cx.fonts)}),
        );
    }
    top.insert("layers".into(), Value::Array(layers));
    top.insert("markers".into(), markers(fmt, comp, &root));
    if !settings.minify {
        top.insert(
            "meta".into(),
            json!({"g": format!("Lumit {}", env!("CARGO_PKG_VERSION"))}),
        );
    }
    let json = serde_json::to_vec(&Ordered(&Value::Object(top)))
        .map_err(|e| LottieError::Json(e.to_string()))?;

    let (bytes, beside) = match settings.format {
        Format::DotLottie => (
            dotlottie::pack(&settings.file_stem, &json, &cx.files)?,
            Vec::new(),
        ),
        _ => (json, cx.files),
    };
    Ok(Exported {
        bytes,
        beside,
        issues: cx.report.finish(),
        frames: root.frame_count(),
        fps: root.fps,
        width: comp.width,
        height: comp.height,
    })
}

/// The layers of `comp` that go out, top first, on the comp's clock `tb`.
pub(crate) fn layers(
    cx: &mut Ctx,
    comp: &Composition,
    tb: &Timebase,
    at: &Place,
) -> Result<Vec<Value>, LottieError> {
    let plan = walk::walk(cx.doc, comp, at, tb, &mut cx.report);
    let mut out = Vec::with_capacity(plan.len());
    let total = plan.len().max(1);
    for (n, layer) in plan.iter().enumerate() {
        if cx.host.cancelled() {
            return Err(LottieError::Cancelled);
        }
        out.push(layers::layer(cx, comp, tb, layer, &plan)?);
        if cx.building.len() == 1 {
            cx.host.progress((n + 1) as f32 / total as f32);
        }
    }
    if cx.host.cancelled() {
        return Err(LottieError::Cancelled);
    }
    Ok(out)
}

/// The keys written first in every object, before the rest in name order.
/// lottie-android and the Flutter player read a shape item, an effect and a
/// gradient in one pass: they skip whatever comes before `ty`, and read a
/// gradient's stops by the count `p` they have already met.
const FIRST: [&str; 2] = ["ty", "p"];

/// A JSON tree written with [`FIRST`] leading each object.
struct Ordered<'a>(&'a Value);

impl serde::Serialize for Ordered<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self.0 {
            Value::Object(m) => {
                let mut map = s.serialize_map(Some(m.len()))?;
                let first = FIRST.iter().filter_map(|k| m.get_key_value(*k));
                let rest = m.iter().filter(|(k, _)| !FIRST.contains(&k.as_str()));
                for (k, v) in first.chain(rest) {
                    map.serialize_entry(k, &Ordered(v))?;
                }
                map.end()
            }
            Value::Array(a) => s.collect_seq(a.iter().map(Ordered)),
            v => serde::Serialize::serialize(v, s),
        }
    }
}

/// The comp's markers, in time order: plain names, times and lengths in
/// frames.
fn markers(fmt: Fmt, comp: &Composition, tb: &Timebase) -> Value {
    let mut list: Vec<&lumit_core::markers::Marker> = comp.markers.iter().collect();
    list.sort_by(|a, b| {
        a.time
            .0
            .to_f64()
            .total_cmp(&b.time.0.to_f64())
            .then_with(|| a.label.cmp(&b.label))
    });
    Value::Array(
        list.into_iter()
            .map(|m| {
                json!({
                    "cm": m.label,
                    "tm": fmt.num(tb.frame(m.time.0.to_f64()), Class::Time),
                    "dr": fmt.num(tb.frame(m.duration.map_or(0.0, |d| d.to_f64())), Class::Time),
                })
            })
            .collect(),
    )
}
