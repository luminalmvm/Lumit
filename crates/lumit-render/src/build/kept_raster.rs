//! A Text or Shape layer's own picture, rasterised, and a solid's.
//!
//! Each is kept while what it is made from is the same, so a layer that does
//! not change is rasterised once rather than on every build. The name is made
//! from the evaluated inputs, never from the document and the time, so an
//! expression or a keyframe that changes the picture changes the name too.
//! The shelf is its own, made like the meshes': 256 MB at most, trimmed under
//! pressure, charged to the governor, and never locked while a raster is made.

use std::borrow::Cow;
use std::sync::Arc;

use lumit_cache::ByteSized;
use lumit_core::expression::ExpressionContext;
use lumit_core::mask::MaskPolyline;
use lumit_core::model::{Composition, Document, Layer, LinearColour, TextDocument};
use lumit_core::shape::modifier::ShapeEntry;
use lumit_core::text::{GlyphAxes, GlyphXform};

use crate::extrude::{name_of, Kept, Shelf};
use crate::placement::world_placement;

use super::{raster_scale, SHAPE_RASTER_PIXELS};

/// A layer's raster: the bytes, the raster size and the natural size.
pub(super) type LayerRaster = (Arc<Vec<u8>>, u32, u32, (f32, f32));

/// The most bytes kept rasters hold, in memory.
pub(crate) const RASTER_BYTES: usize = 256 << 20;

/// The kept rasters, by name.
pub(crate) static RASTERS: Shelf<KeptRaster> = std::sync::Mutex::new(None);

/// One kept raster. A mask or a stroke copies it before writing, so the kept
/// bytes are never changed.
pub(crate) struct KeptRaster {
    rgba: Arc<Vec<u8>>,
    w: u32,
    h: u32,
    /// The layer's own size, which the placement draws the raster at.
    natural: (f32, f32),
}

impl ByteSized for Kept<KeptRaster> {
    fn byte_size(&self) -> usize {
        self.0.rgba.len() + 24
    }
}

/// The raster named `name`, made by `make` if it is not kept.
fn kept(name: u128, make: impl FnOnce() -> KeptRaster) -> Arc<KeptRaster> {
    crate::extrude::kept(&RASTERS, RASTER_BYTES, name, make)
}

/// A solid's colour as a `tw` by `th` raster.
pub(super) fn solid(colour: LinearColour, tw: u32, th: u32) -> Arc<Vec<u8>> {
    let mut h = blake3::Hasher::new();
    h.update(b"solid/1/");
    for ch in colour.0 {
        h.update(&ch.to_le_bytes());
    }
    h.update(&tw.to_le_bytes());
    h.update(&th.to_le_bytes());
    let raster = kept(name_of(&h), || KeptRaster {
        rgba: Arc::new(lumit_core::pixels::px_tile(
            &lumit_core::pixels::solid_rgba(colour),
            tw,
            th,
        )),
        w: tw,
        h: th,
        natural: (tw as f32, th as f32),
    });
    Arc::clone(&raster.rgba)
}

/// The name of a Text layer's raster: what the block holds, what each letter
/// is asked to do, the path it runs along and how far along it, the zoom it
/// is drawn at and the box it is drawn into, and the fonts in force.
fn text_name(
    block: &lumit_text::TextBlock<'_>,
    xforms: &[GlyphXform],
    spine: Option<&MaskPolyline>,
    offset: f32,
    (zoom, own): (f32, (u32, u32)),
) -> u128 {
    let mut h = blake3::Hasher::new();
    h.update(b"text/1/");
    h.update(&lumit_text::fonts::epoch().to_le_bytes());
    h.update(&zoom.to_le_bytes());
    h.update(&own.0.to_le_bytes());
    h.update(&own.1.to_le_bytes());
    h.update(&block.key_bytes());
    h.update(&(xforms.len() as u64).to_le_bytes());
    for x in xforms {
        // Every field by name, so a new one will not build until it is here.
        let GlyphXform {
            position,
            rotation,
            scale,
            opacity,
            fill,
            depth,
            turn,
            plate,
            axes,
        } = *x;
        let pairs = position.into_iter().chain(scale).chain(turn);
        let lone = [rotation, opacity, depth];
        for v in pairs.chain(lone).chain(fill).chain(plate) {
            h.update(&v.to_le_bytes());
        }
        let GlyphAxes {
            tags,
            keep,
            add,
            len,
        } = axes;
        h.update(&tags.concat());
        for v in keep.into_iter().chain(add) {
            h.update(&v.to_le_bytes());
        }
        h.update(&[len]);
    }
    if let Some(path) = spine {
        h.update(b"spine/");
        h.update(&offset.to_le_bytes());
        h.update(&[u8::from(path.closed)]);
        h.update(&(path.points.len() as u64).to_le_bytes());
        for p in &path.points {
            h.update(&p[0].to_le_bytes());
            h.update(&p[1].to_le_bytes());
        }
        for a in &path.arc {
            h.update(&a.to_le_bytes());
        }
    }
    name_of(&h)
}

/// The name of a Shape layer's raster: the art as it stands at this time, the
/// raster size and the box it fills, and the time `at` for art that still
/// moves. `None` when the art will not serialise, and then nothing is kept.
fn shape_name(
    contents: &[ShapeEntry],
    (w, h): (u32, u32),
    bounds: [f64; 4],
    at: Option<f64>,
) -> Option<u128> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"shape/1/");
    hasher.update(&serde_json::to_vec(contents).ok()?);
    if let Some(lt) = at {
        hasher.update(&lt.to_le_bytes());
    }
    hasher.update(&w.to_le_bytes());
    hasher.update(&h.to_le_bytes());
    for b in bounds {
        hasher.update(&b.to_le_bytes());
    }
    Some(name_of(&hasher))
}

/// Whether art that `contents_at` handed back as it was still moves: a keyed
/// path, or a number keyed or driven by an expression, which it leaves when
/// the frozen art will not read back (an expression giving NaN).
fn still_moves(contents: &[ShapeEntry]) -> bool {
    use serde_json::Value;
    fn moves(v: &Value) -> bool {
        match v {
            Value::Object(map) => map.iter().any(|(k, v)| match k.as_str() {
                "animation" => v.as_object().is_some_and(|a| !a.contains_key("Static")),
                _ => moves(v),
            }),
            Value::Array(list) => list.iter().any(moves),
            _ => false,
        }
    }
    let items = lumit_core::shape::modifier::items(contents);
    items.iter().any(|item| item.path_is_animated())
        || serde_json::to_value(contents).map_or(true, |v| moves(&v))
}

/// A Text layer's words at layer time `lt`, comp time `t_comp`, rasterised.
pub(super) fn text(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    document: &TextDocument,
    (lt, t_comp): (f64, f64),
    context: &Arc<ExpressionContext>,
) -> LayerRaster {
    // The words at *this* layer time — a plain document hands back
    // what was typed; an expression-driven one is evaluated here,
    // which is what makes a caption able to print a live value.

    let (line, spoken, _) = lumit_core::captions::show::spoken(document, context.clone());
    let size = document.size as f32;
    // **Text on a path**. The named mask's own flattened
    // curve, read through the carriage every path-walking effect
    // reads — so the glyphs and a Stroke on the same mask
    // agree about where the curve is. A layer whose row names
    // nothing, or a mask since deleted, flattens to an empty
    // polyline and the line lays straight: refuse calmly, never
    // an empty layer (docs/14 §4).
    let spine = document
        .path
        .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, lt))
        .filter(|p| !p.is_empty());
    // **The letters move separately**. What each one is
    // asked to do is worked out here, from the animators' own
    // keyframed numbers at this layer time; a layer with none hands
    // back an empty list and the rasteriser takes the path it
    // always took, byte for byte.
    let xforms = lumit_core::text::ports::layer_glyph_xforms(
        doc, comp, layer, document, &line, t_comp, &spoken,
    );
    // An unstyled single line draws the way it always has. A
    // styled one, or one with a line break, is set by the shaper.
    let block = lumit_text::TextBlock::of(document, &line);
    let offset = document.path_offset.value_at(lt) as f32;
    // **Continuously rasterised**, as a shape layer is: the words
    // are drawn as large as the layer and its parents show them,
    // so a small text scaled up never blurs. The placement keeps
    // the layer's own size, and a raster too big to hold is drawn
    // at the most it can be.
    let shown =
        raster_scale::shape_raster_scale(world_placement(comp, layer, t_comp, context.clone()));
    let room = |w: u32, h: u32| {
        (SHAPE_RASTER_PIXELS / (f64::from(w.max(1)) * f64::from(h.max(1))))
            .sqrt()
            .max(1.0)
    };
    // The zoom it is drawn at and the box it is drawn into. Shown at its own
    // size or smaller, the layer is drawn as it always was, without laying it
    // out twice, and the box is the raster's own.
    let (zoom, own) = match &spine {
        Some(path) => {
            let (w, h) = lumit_text::path_box(path, size);
            (f64::from(shown).clamp(1.0, room(w, h)) as f32, (w, h))
        }
        None if shown <= 1.0 => (1.0, (0, 0)),
        None => {
            let own = lumit_text::layout(&block, !xforms.is_empty());
            let zoom = f64::from(shown).clamp(1.0, room(own.width, own.height)) as f32;
            (zoom, (own.width, own.height))
        }
    };
    let name = text_name(&block, &xforms, spine.as_ref(), offset, (zoom, own));
    let r = kept(name, || {
        let (r, natural) = match &spine {
            Some(path) => {
                let (w, h) = own;
                let r = lumit_text::rasterise_along_at(&block, path, offset, w, h, &xforms, zoom);
                (r, (w.max(1), h.max(1)))
            }
            None if shown <= 1.0 => {
                let r = lumit_text::rasterise(&block, &xforms);
                let size = (r.width, r.height);
                (r, size)
            }
            None => lumit_text::rasterise_at(&block, &xforms, zoom),
        };
        KeptRaster {
            rgba: Arc::new(r.rgba),
            w: r.width,
            h: r.height,
            natural: (natural.0 as f32, natural.1 as f32),
        }
    });
    (Arc::clone(&r.rgba), r.w, r.h, r.natural)
}

/// A Shape layer's art at layer time `lt`, rasterised. `None` when it has
/// nothing to draw.
pub(super) fn shape(
    comp: &Composition,
    layer: &Layer,
    contents: &[ShapeEntry],
    (lt, t_comp): (f64, f64),
    context: &Arc<ExpressionContext>,
) -> Option<LayerRaster> {
    let contents = lumit_core::shape::contents_at(contents, lt, context);
    let (x0, y0, x1, y1) = lumit_core::shape::contents_bounds(&contents, lt)?;
    let natural_w = (x1 - x0).max(1.0);
    let natural_h = (y1 - y0).max(1.0);
    // The same reduced-resolution rule the other rasterised
    // kinds follow: draw at the working scale, and let the
    // placement matrix carry the natural size.
    //
    // **Continuously rasterised**, as After Effects draws a
    // shape layer: the art is drawn as large as the layer and
    // its parents show it, so scaling it up never blurs it.
    // The effects read the raster factor off the texture, so
    // they keep their size.
    let shown =
        raster_scale::shape_raster_scale(world_placement(comp, layer, t_comp, context.clone()));
    let most = (SHAPE_RASTER_PIXELS / (natural_w * natural_h))
        .sqrt()
        .max(1.0);
    let k = f64::from(shown).clamp(1.0, most);
    let w = (natural_w * k).round().max(1.0) as u32;
    let h = (natural_h * k).round().max(1.0) as u32;
    let make = || lumit_core::shape::rasterise_contents(&contents, w, h, x0, y0, x1, y1, lt);
    // Art that came back unfrozen draws differently at each time, so its
    // name carries the time.
    let at = (matches!(contents, Cow::Borrowed(_)) && still_moves(&contents)).then_some(lt);
    let rgba = match shape_name(&contents, (w, h), [x0, y0, x1, y1], at) {
        Some(name) => {
            let r = kept(name, || KeptRaster {
                rgba: Arc::new(make()),
                w,
                h,
                natural: (natural_w as f32, natural_h as f32),
            });
            Arc::clone(&r.rgba)
        }
        None => Arc::new(make()),
    };
    Some((rgba, w, h, (natural_w as f32, natural_h as f32)))
}
