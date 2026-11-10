//! A Lottie's assets: the pictures of still footage, and the comps precomp
//! layers show. Each is written once however many layers use it.
//!
//! Runs on the export's thread.

use base64::Engine;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::Ctx;
use crate::check::Place;
use crate::settings::{Format, ImageMode};
use crate::time::Timebase;
use crate::{LottieError, Picture};

/// The asset id of footage `item`'s picture, made the first time it is
/// asked for; `None` when it is video or can't be read, which the check
/// says once.
pub fn image(cx: &mut Ctx, item: Uuid, at: &Place) -> Option<String> {
    if let Some((_, id)) = cx.images.iter().find(|(i, _)| *i == item) {
        return id.clone();
    }
    let still = match cx.host.picture(item) {
        Picture::Still(still) => Some(still),
        Picture::Video => {
            cx.report
                .left_out(at, "video_footage", "Video is left out".into(), Vec::new());
            None
        }
        Picture::Missing => {
            cx.report.left_out(
                at,
                "missing_footage",
                "The file is missing".into(),
                Vec::new(),
            );
            None
        }
    };
    let id = still.and_then(|still| {
        let pic = crate::picture::encode(&still, cx.settings.image_quality)?;
        let n = cx.images.iter().filter(|(_, id)| id.is_some()).count() + 1;
        let id = format!("image_{n}");
        let (u, p, e) = match (cx.settings.format, cx.settings.images) {
            (Format::DotLottie, _) => {
                let name = format!("{id}.{}", pic.ext);
                cx.files.push((name.clone(), pic.bytes));
                ("/i/", name, 0)
            }
            (_, ImageMode::Beside) => {
                let name = format!("{}_{n}.{}", cx.settings.file_stem, pic.ext);
                cx.files.push((name.clone(), pic.bytes));
                ("", name, 0)
            }
            (_, ImageMode::Embed) => {
                let data = base64::engine::general_purpose::STANDARD.encode(&pic.bytes);
                ("", format!("data:{};base64,{data}", pic.mime), 1)
            }
        };
        cx.assets.push(json!({
            "id": id,
            "w": still.width,
            "h": still.height,
            "u": u,
            "p": p,
            "e": e,
        }));
        Some(id)
    });
    cx.images.push((item, id.clone()));
    id
}

/// The asset id of comp `id` as a precomp, written the first time it is
/// asked for with every layer it holds. `None` for a comp that is missing
/// or that contains itself.
pub fn precomp(cx: &mut Ctx, id: Uuid, at: &Place) -> Result<Option<String>, LottieError> {
    if let Some((_, asset)) = cx.comps.iter().find(|(c, _)| *c == id) {
        return Ok(Some(asset.clone()));
    }
    let doc = cx.doc.clone();
    let Some(comp) = doc.comp(id) else {
        return Ok(None);
    };
    if cx.building.contains(&id) {
        cx.report.left_out(
            at,
            "precomp_cycle",
            "The composition contains itself".into(),
            Vec::new(),
        );
        return Ok(None);
    }
    // A precomp plays on the output's clock across its own whole length, and
    // past it wherever a layer inside runs on: Lumit draws a layer of an
    // unretimed precomp at any time inside the layer's own span.
    let spans = comp
        .layers
        .iter()
        .map(|l| (l.in_point.0.to_f64(), l.out_point.0.to_f64()));
    let (start, end) = spans.fold((0.0f64, comp.duration.0.to_f64()), |(a, b), (i, o)| {
        (a.min(i), b.max(o))
    });
    let tb = Timebase {
        start,
        end,
        ..cx.root
    };
    cx.building.push(id);
    let layers = super::layers(cx, comp, &tb, &at.inside(id));
    cx.building.pop();
    let layers = layers?;
    let asset = format!("comp_{}", cx.comps.len() + 1);
    cx.comps.push((id, asset.clone()));
    let mut m = Map::new();
    m.insert("id".into(), Value::from(asset.clone()));
    cx.fmt.name(&mut m, &comp.name);
    m.insert("layers".into(), Value::Array(layers));
    cx.assets.push(Value::Object(m));
    Ok(Some(asset))
}
