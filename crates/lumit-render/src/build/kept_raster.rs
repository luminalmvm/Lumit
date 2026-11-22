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
use lumit_core::mask::{BezierPath, Mask, MaskPolyline};
use lumit_core::model::{Composition, Document, Layer, LinearColour, TextDocument};
use lumit_core::paint::PaintStroke;
use lumit_core::shape::modifier::ShapeEntry;
use lumit_core::text::{GlyphAxes, GlyphXform};
use lumit_gpu::composite::MbSample;

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
    /// How far up and left of the layer's corner the raster starts, for
    /// animated letters thrown further than their room.
    origin: (f32, f32),
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
        origin: (0.0, 0.0),
    });
    Arc::clone(&raster.rgba)
}

/// The name of a Text layer's raster: what the block holds, what each letter
/// is asked to do, the path it runs along and how far along it, the zoom it
/// is drawn at, the box it is drawn into and how far it grows, and the fonts
/// in force.
fn text_name(
    block: &lumit_text::TextBlock<'_>,
    xforms: &[GlyphXform],
    spine: Option<&MaskPolyline>,
    offset: f32,
    (zoom, own, grow): (f32, (u32, u32), [u32; 4]),
) -> u128 {
    let mut h = blake3::Hasher::new();
    h.update(b"text/1/");
    h.update(&lumit_text::fonts::epoch().to_le_bytes());
    h.update(&zoom.to_le_bytes());
    h.update(&own.0.to_le_bytes());
    h.update(&own.1.to_le_bytes());
    // Only a grown box names it, so every other raster keeps its name.
    if grow != [0; 4] {
        h.update(b"grow/");
        for g in grow {
            h.update(&g.to_le_bytes());
        }
    }
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

/// A Text layer's words at layer time `lt`, comp time `t_comp`, rasterised,
/// the raster's name, and how far up and left of the layer's corner it starts.
pub(super) fn text(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    document: &TextDocument,
    (lt, t_comp): (f64, f64),
    context: &Arc<ExpressionContext>,
) -> ((LayerRaster, Option<u128>), (f32, f32)) {
    // The words at *this* layer time — a plain document hands back
    // what was typed; an expression-driven one is evaluated here,
    // which is what makes a caption able to print a live value.

    let (line, spoken, _) = lumit_core::captions::show::spoken(document, context.clone());
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
    let offset = document.path_offset.value_at(lt) as f32;
    // **Continuously rasterised**, as a shape layer is: the words
    // are drawn as large as the layer and its parents show them,
    // so a small text scaled up never blurs. The placement keeps
    // the layer's own size, and a raster too big to hold is drawn
    // at the most it can be.
    let shown =
        raster_scale::shape_raster_scale(world_placement(comp, layer, t_comp, context.clone()));
    let spine = spine.as_ref();
    let at = (doc, comp, layer, document, t_comp, spine, shown);
    let words = laid(at, &line, &spoken);
    let Some((other, w)) = document.resolved_blend(context) else {
        let name = text_name(
            &words.block,
            &words.xforms,
            spine,
            offset,
            (words.zoom, words.own, words.grow),
        );
        let r = kept(name, || {
            let drawn = draw(&words, spine, offset, words.zoom, words.grow);
            kept_of(drawn, words.grow)
        });
        let raster = (Arc::clone(&r.rgba), r.w, r.h, r.natural);
        return ((raster, Some(name)), r.origin);
    };
    // **A fade between keyed words**: each word laid out and drawn on its
    // own, at the same zoom and grown the same, then mixed. Named by both and
    // the weight, so a kept raster of one frame of the fade is never served
    // for another.
    let fade = laid(at, &other, &[]);
    let zoom = words.zoom.min(fade.zoom);
    let wider = (words.own.0.max(fade.own.0), words.own.1.max(fade.own.1));
    let most = std::array::from_fn(|i| words.grow[i].max(fade.grow[i]));
    let grow = lumit_text::animated::trimmed(most, wider, zoom);
    let mut h = blake3::Hasher::new();
    h.update(b"text/fade/1/");
    for one in [&words, &fade] {
        let name = text_name(
            &one.block,
            &one.xforms,
            spine,
            offset,
            (zoom, one.own, grow),
        );
        h.update(&name.to_le_bytes());
    }
    h.update(&w.to_le_bytes());
    let name = name_of(&h);
    let r = kept(name, || {
        let (a, a_natural) = draw(&words, spine, offset, zoom, grow);
        let (b, b_natural) = draw(&fade, spine, offset, zoom, grow);
        let mixed = lumit_text::blend::crossfade(&a, &b, w);
        let natural = (a_natural.0.max(b_natural.0), a_natural.1.max(b_natural.1));
        kept_of((mixed, natural), grow)
    });
    let raster = (Arc::clone(&r.rgba), r.w, r.h, r.natural);
    ((raster, Some(name)), r.origin)
}

/// What one set of words is laid out from: the document and layer, the comp
/// time, the path and how large the layer is shown.
type LaidAt<'a> = (
    &'a Arc<Document>,
    &'a Composition,
    &'a Layer,
    &'a TextDocument,
    f64,
    Option<&'a MaskPolyline>,
    f32,
);

/// One set of words laid out: the block, what each letter is asked to do, the
/// zoom it can be drawn at, the box it is drawn into and how far it grows.
struct Laid<'a> {
    block: lumit_text::TextBlock<'a>,
    xforms: Vec<GlyphXform>,
    zoom: f32,
    own: (u32, u32),
    grow: [u32; 4],
}

/// `line` laid out as the layer's words.
fn laid<'a>(
    (doc, comp, layer, document, t_comp, spine, shown): LaidAt<'a>,
    line: &'a str,
    spoken: &[lumit_core::text::SpokenWord],
) -> Laid<'a> {
    // **The letters move separately**. What each one is
    // asked to do is worked out here, from the animators' own
    // keyframed numbers at this layer time; a layer with none hands
    // back an empty list and the rasteriser takes the path it
    // always took, byte for byte.
    let xforms = lumit_core::text::ports::layer_glyph_xforms(
        doc, comp, layer, document, line, t_comp, spoken,
    );
    // An unstyled single line draws the way it always has. A
    // styled one, or one with a line break, is set by the shaper.
    let block = lumit_text::TextBlock::of(document, line);
    let room = |w: u32, h: u32| {
        (SHAPE_RASTER_PIXELS / (f64::from(w.max(1)) * f64::from(h.max(1))))
            .sqrt()
            .max(1.0)
    };
    // Animated letters thrown further than the room round the words grow the
    // box that far, so they are not clipped.
    let reach = match spine {
        Some(_) => [0; 4],
        None => lumit_text::animated::overhang(&block, &xforms),
    };
    // The zoom it is drawn at, the box it is drawn into and how far it grows.
    // Shown at its own size or smaller and not grown, the layer is drawn as it
    // always was, without laying it out twice, and the box is the raster's own.
    let (zoom, own, grow) = match spine {
        Some(path) => {
            let (w, h) = lumit_text::path_box(path, document.size as f32);
            (
                f64::from(shown).clamp(1.0, room(w, h)) as f32,
                (w, h),
                reach,
            )
        }
        None if shown <= 1.0 && reach == [0; 4] => (1.0, (0, 0), reach),
        None => {
            let laid = lumit_text::layout(&block, !xforms.is_empty());
            let own = (laid.width, laid.height);
            let [l, t, r, b] = reach;
            let (w, h) = (own.0.saturating_add(l + r), own.1.saturating_add(t + b));
            let zoom = f64::from(shown).clamp(1.0, room(w, h)) as f32;
            (zoom, own, lumit_text::animated::trimmed(reach, own, zoom))
        }
    };
    Laid {
        block,
        xforms,
        zoom,
        own,
        grow,
    }
}

/// Laid-out words drawn at `zoom`, grown by `grow`, and the layer's own size.
fn draw(
    words: &Laid<'_>,
    spine: Option<&MaskPolyline>,
    offset: f32,
    zoom: f32,
    grow: [u32; 4],
) -> (lumit_text::RasterText, (u32, u32)) {
    let (block, xforms) = (&words.block, &words.xforms);
    match spine {
        Some(path) => {
            let (w, h) = words.own;
            let r = lumit_text::rasterise_along_at(block, path, offset, w, h, xforms, zoom);
            (r, (w.max(1), h.max(1)))
        }
        None => lumit_text::rasterise_at(block, xforms, zoom, grow),
    }
}

fn kept_of((r, natural): (lumit_text::RasterText, (u32, u32)), grow: [u32; 4]) -> KeptRaster {
    KeptRaster {
        rgba: Arc::new(r.rgba),
        w: r.width,
        h: r.height,
        natural: (natural.0 as f32, natural.1 as f32),
        origin: (grow[0] as f32, grow[1] as f32),
    }
}

/// A Shape layer's art at layer time `lt`, rasterised, and the raster's name
/// when it has one. `None` when it has nothing to draw.
pub(super) fn shape(
    comp: &Composition,
    layer: &Layer,
    contents: &[ShapeEntry],
    (lt, t_comp): (f64, f64),
    context: &Arc<ExpressionContext>,
) -> Option<(LayerRaster, Option<u128>)> {
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
    let name = shape_name(&contents, (w, h), [x0, y0, x1, y1], at);
    let rgba = match name {
        Some(name) => {
            let r = kept(name, || KeptRaster {
                rgba: Arc::new(make()),
                w,
                h,
                natural: (natural_w as f32, natural_h as f32),
                origin: (0.0, 0.0),
            });
            Arc::clone(&r.rgba)
        }
        None => Arc::new(make()),
    };
    Some(((rgba, w, h, (natural_w as f32, natural_h as f32)), name))
}

/// A layer's masks and paint, moved into a raster that starts `at` up and left
/// of the layer's corner. As they are for every other raster.
pub(super) fn moved_art(
    layer: &Layer,
    at: Option<(f32, f32)>,
) -> (Cow<'_, [Mask]>, Cow<'_, [PaintStroke]>) {
    let Some((dx, dy)) = at.filter(|&a| a != (0.0, 0.0)) else {
        return (Cow::Borrowed(&layer.masks), Cow::Borrowed(&layer.paint));
    };
    let (dx, dy) = (f64::from(dx), f64::from(dy));
    let moved = |path: &mut BezierPath| {
        for v in &mut path.vertices {
            v.pos = (v.pos.0 + dx, v.pos.1 + dy);
        }
    };
    let mut masks = layer.masks.clone();
    for mask in &mut masks {
        moved(&mut mask.path);
        for key in &mut mask.path_keys {
            moved(&mut key.path);
        }
    }
    let mut paint = layer.paint.clone();
    for stroke in &mut paint {
        for p in &mut stroke.points {
            *p = (p.0 + dx, p.1 + dy);
        }
    }
    (Cow::Owned(masks), Cow::Owned(paint))
}

/// A placement for a raster that starts `at` up and left of the layer's
/// corner: the anchor is read that much further in, so the layer's own
/// pixels land where they always did.
pub(super) fn lifted(mut placed: MbSample, at: Option<(f32, f32)>) -> MbSample {
    if let Some((dx, dy)) = at {
        placed.anchor = (placed.anchor.0 + dx, placed.anchor.1 + dy);
    }
    placed
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    use lumit_core::model::{LayerKind, ProjectItem};
    use lumit_core::text::keys::TextKeys;
    use lumit_core::time::{Duration, FrameRate, Rational};

    type Drawn = (Vec<u8>, u32, u32, u128);

    /// `document` on a layer drawn at layer time 1.
    fn drawn(document: &TextDocument) -> Drawn {
        let mut layer = crate::build::render_below_at_tests::text_layer(0.0);
        layer.kind = LayerKind::Text {
            document: document.clone(),
        };
        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            master_keys: None,
            master_automation: Default::default(),
            loudness_target: None,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: uuid::Uuid::now_v7(),
            name: "c".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![layer.clone()],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(comp.clone()));
        let doc = Arc::new(doc);
        let context = Arc::new(ExpressionContext {
            document: doc.clone(),
            comp: Some(comp.id),
            layer: Some(layer.id),
            comp_time: 1.0,
            current_depth: 0,
            inputs: None,
        });
        let (((rgba, w, h, _), name), _) =
            text(&doc, &comp, &layer, document, (1.0, 1.0), &context);
        (rgba.to_vec(), w, h, name.unwrap())
    }

    /// "hello" keyed at 0 and "goodbye" at 2, the first key leaving `out`.
    fn keyed(plain: &TextDocument, out: SideInterp) -> TextDocument {
        let key = |t: i64, value: f64, interp_out: SideInterp| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value,
            interp_in: SideInterp::Linear,
            interp_out,
        };
        let mut document = plain.clone();
        document.keys = Some(Box::new(TextKeys {
            words: vec!["hello".into(), "goodbye".into()],
            index: Property {
                animation: Animation::Keyframed(vec![
                    key(0, 0.0, out),
                    key(2, 1.0, SideInterp::Hold),
                ]),
                extra: serde_json::Map::new(),
            },
        }));
        document
    }

    /// A held span draws the plain raster of its word, under the same name. A
    /// fade halfway draws both words mixed half and half, from the top left.
    #[test]
    fn a_fade_between_keyed_words_draws_both_mixed_and_a_hold_draws_one() {
        let layer = crate::build::render_below_at_tests::text_layer(0.0);
        let LayerKind::Text { document: plain } = layer.kind else {
            panic!("a text layer");
        };
        let words = |text: &str| {
            let mut document = plain.clone();
            document.text = text.into();
            drawn(&document)
        };
        let (hello, goodbye) = (words("hello"), words("goodbye"));
        assert_eq!(drawn(&keyed(&plain, SideInterp::Hold)), hello);

        let mixed = drawn(&keyed(&plain, SideInterp::Linear));
        let (w, h) = (hello.1.max(goodbye.1), hello.2.max(goodbye.2));
        assert_eq!((mixed.1, mixed.2), (w, h), "the union of the two sizes");
        assert_ne!(mixed.3, hello.3, "a fade frame has a name of its own");
        let px = |r: &Drawn, x: u32, y: u32| -> [u8; 4] {
            if x >= r.1 || y >= r.2 {
                return [0; 4];
            }
            let i = ((y * r.1 + x) * 4) as usize;
            [r.0[i], r.0[i + 1], r.0[i + 2], r.0[i + 3]]
        };
        let mut alone = 0;
        for y in 0..h {
            for x in 0..w {
                let (a, b, m) = (px(&hello, x, y), px(&goodbye, x, y), px(&mixed, x, y));
                if a[3] == 255 && b[3] == 0 {
                    assert_eq!(m, [a[0], a[1], a[2], 128], "half of one word at {x},{y}");
                    alone += 1;
                }
                if a[3] == 0 && b[3] == 0 {
                    assert_eq!(m, [0; 4], "nothing where neither word is, at {x},{y}");
                }
            }
        }
        assert!(alone > 0, "some pixel is covered by one word only");
    }
}
