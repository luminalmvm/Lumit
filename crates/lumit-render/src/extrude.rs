//! A Text or Shape layer with Geometry options on the way to the mesh pass:
//! its outline now, the solid each part makes, and the scene they come to.
//!
//! A letter or a run of art is meshed once for each outline it ever has, and
//! the scenes built from those are kept too, so a playing type-on meshes only
//! its new letter and a moving layer meshes nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use glam::{Mat3, Mat4, Vec3};
use lumit_budget::Tier;
use lumit_cache::{ByteLru, ByteSized};
use lumit_core::expression::ExpressionContext;
use lumit_core::extrude::mesh::{self, Mesh};
use lumit_core::extrude::Profile;
use lumit_core::model::{EffectInstance, Layer, LayerKind};
use lumit_core::shape::modifier::ShapeEntry;
use lumit_core::text::GlyphXform;
use lumit_scene::{AlphaMode, Bounds, Material, ModelScene, Node, Primitive};
use uuid::Uuid;

use crate::model_material::Look;

/// The most bytes kept meshes and kept scenes hold, in memory. A scene is
/// cheap to make again from its meshes, and its GPU model goes with it.
const MESH_BYTES: usize = 256 << 20;
const SCENE_BYTES: usize = 128 << 20;

struct Kept<T>(Arc<T>);

impl ByteSized for Kept<Mesh> {
    fn byte_size(&self) -> usize {
        self.0.byte_size()
    }
}

impl ByteSized for Kept<ModelScene> {
    fn byte_size(&self) -> usize {
        self.0.byte_size()
    }
}

type Shelf<T> = Mutex<Option<ByteLru<u128, Kept<T>>>>;

static MESHES: Shelf<Mesh> = Mutex::new(None);
static SCENES: Shelf<ModelScene> = Mutex::new(None);
/// Each layer's scene as last drawn, while it lives, which picking and its
/// box read.
static LAST: Mutex<Option<HashMap<Uuid, Weak<ModelScene>>>> = Mutex::new(None);

/// What's on `shelf` under `key`, made and put there if it isn't. The lock is
/// never held while it's made.
fn kept<T>(shelf: &Shelf<T>, bytes: usize, key: u128, make: impl FnOnce() -> T) -> Arc<T>
where
    Kept<T>: ByteSized,
{
    let found = shelf.lock().ok().and_then(|mut slot| {
        let lru = slot.get_or_insert_with(|| ByteLru::new(bytes));
        lru.get(&key).map(|k| Arc::clone(&k.0))
    });
    if let Some(found) = found {
        return found;
    }
    let made = Arc::new(make());
    if let Ok(mut slot) = shelf.lock() {
        let lru = slot.get_or_insert_with(|| ByteLru::new(bytes));
        if lru
            .pressure()
            .is_some_and(lumit_budget::Pressure::should_trim)
        {
            lru.evict_cold_half();
        }
        lru.insert(key, Kept(Arc::clone(&made)));
    }
    made
}

/// Count kept meshes and scenes against the governor's `ledger` from here on.
pub(crate) fn account_against(ledger: &Arc<lumit_budget::Ledger>) {
    fn set<T>(shelf: &Shelf<T>, bytes: usize, ledger: &Arc<lumit_budget::Ledger>)
    where
        Kept<T>: ByteSized,
    {
        if let Ok(mut slot) = shelf.lock() {
            let mut lru = ByteLru::new(bytes);
            lru.account_against(Arc::clone(ledger), Tier::Ram);
            *slot = Some(lru);
        }
    }
    set(&MESHES, MESH_BYTES, ledger);
    set(&SCENES, SCENE_BYTES, ledger);
}

/// One letter, or one run of a shape's art at one of its copies.
struct Piece {
    contours: Vec<Vec<[f64; 2]>>,
    /// Linear, the front's and the back's.
    fill: [f32; 4],
    /// Linear, the bevel's and the side's.
    edge: [f32; 4],
    /// What the text's animators ask of a letter.
    xform: GlyphXform,
    /// How far towards the eye it sits, so art overlapping in one layer stacks
    /// as it paints when the layer turns.
    lift: f32,
    /// Where a letter on a straight line turns and scales: the middle of its
    /// advance on the baseline, as a flat one does. Else the middle of its box.
    pivot: Option<[f32; 2]>,
}

/// `layer`'s letters or art at layer time `lt`, in the pixels of the picture
/// it would draw flat, so its front lands where that picture would.
fn pieces(layer: &Layer, lt: f64, context: &Arc<ExpressionContext>) -> Vec<Piece> {
    match &layer.kind {
        LayerKind::Text { document } => {
            let line = document.resolved_text(context.clone());
            let spine = document
                .path
                .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, lt))
                .filter(|p| !p.is_empty());
            let offset = document.path_offset.value_at(lt) as f32;
            let block = lumit_text::TextBlock::of(document, &line);
            let entries = lumit_text::shape_items(&block, spine.as_ref(), offset);
            let xforms = lumit_core::text::glyph_xforms(&document.animators, &line, lt);
            // A straight line with animators is drawn flat with room round it,
            // which moves its letters in, so it keeps its place here too.
            let roomy =
                (!xforms.is_empty() && spine.is_none()).then(|| lumit_text::layout(&block, true));
            let room = roomy.as_ref().map_or([0.0; 2], |roomy| {
                let tight = lumit_text::layout(&block, false);
                let baseline = |l: &lumit_text::BlockLayout| {
                    l.lines.first().map_or(0.0, |line| f64::from(line.baseline))
                };
                [
                    f64::from(roomy.left - tight.left),
                    baseline(roomy) - baseline(&tight),
                ]
            });
            let pivot_of = |k: usize| {
                let line = roomy.as_ref()?.lines.iter().rev().find(|l| l.start <= k)?;
                let (a, b) = (
                    line.carets.get(k - line.start)?,
                    line.carets.get(k + 1 - line.start)?,
                );
                Some([(a + b) / 2.0, line.baseline])
            };
            let chars: Vec<char> = line.chars().collect();
            let [fill, edge] = text_colours(document);
            let style = &document.style;
            let mut cursor = 0;
            let mut out = Vec::with_capacity(entries.len());
            for entry in &entries {
                let ShapeEntry::Group(group) = entry else {
                    continue;
                };
                // The letter it is, for what its animators ask of it.
                let ch = group.name.chars().next();
                let at = (chars.iter().skip(cursor)).position(|c| Some(*c) == ch);
                let at = at.map(|k| k + cursor);
                cursor = at.map_or(cursor, |k| k + 1);
                let xform = at.and_then(|k| xforms.get(k)).copied().unwrap_or_default();
                if xform.opacity <= 0.0 {
                    continue;
                }
                let items = lumit_core::shape::modifier::items(&group.entries);
                let filled = items.iter().filter(|i| i.fill.is_some());
                let filled: Vec<_> = filled.collect();
                let mut contours: Vec<Vec<[f64; 2]>> = if filled.is_empty() {
                    let width = style.stroke_width;
                    (items.iter())
                        .flat_map(|i| {
                            lumit_core::shape::solid::outline_of(
                                &lumit_core::extrude::flattened(&i.path),
                                width,
                            )
                        })
                        .collect()
                } else {
                    let flat = filled
                        .iter()
                        .map(|i| lumit_core::extrude::flattened(&i.path));
                    let outline = if style.stroke_on {
                        style.stroke_width
                    } else {
                        0.0
                    };
                    lumit_core::shape::solid::with_stroke(&flat.collect::<Vec<_>>(), outline)
                };
                for p in contours.iter_mut().flatten() {
                    (p[0], p[1]) = (p[0] + room[0], p[1] + room[1]);
                }
                let added = |c: [f32; 4]| {
                    [
                        c[0] + xform.fill[0],
                        c[1] + xform.fill[1],
                        c[2] + xform.fill[2],
                        c[3] * xform.opacity,
                    ]
                };
                out.push(Piece {
                    contours,
                    fill: added(fill),
                    edge: added(edge),
                    xform,
                    lift: 0.0,
                    pivot: at.and_then(pivot_of),
                });
            }
            out
        }
        LayerKind::Shape { contents } => {
            let contents = lumit_core::shape::contents_at(contents, lt, context);
            let solids = lumit_core::shape::solid::solids(&contents, lt);
            // A quarter of a pixel each, in all no more than a few pixels.
            let step = (8.0 / solids.len().max(1) as f32).min(0.25);
            solids
                .into_iter()
                .enumerate()
                .map(|(k, s)| Piece {
                    lift: k as f32 * step,
                    pivot: None,
                    edge: s
                        .outline
                        .map_or(s.fill, |[r, g, b, _]| [r, g, b, s.fill[3]]),
                    fill: s.fill,
                    contours: s.contours,
                    xform: GlyphXform::default(),
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// A text's front colour and its edge's: its fill and its outline, or the one
/// it has for both.
fn text_colours(document: &lumit_core::model::TextDocument) -> [[f32; 4]; 2] {
    let style = &document.style;
    let stroked = style.stroke_on && style.stroke_width > 0.0;
    let fill = if style.fill_on || !stroked {
        document.fill.0
    } else {
        style.stroke.0
    };
    [fill, if stroked { style.stroke.0 } else { fill }]
}

/// `layer`'s faces, Front, Bevel, Side and Back, each its default material:
/// the first colours it paints with at its start, so turning up the depth
/// shows the user's own colour, lit, as a matte plastic.
#[must_use]
pub(crate) fn faces(layer: &Layer) -> [Material; 4] {
    let [fill, edge] = match &layer.kind {
        LayerKind::Text { document } => text_colours(document),
        LayerKind::Shape { contents } => shape_colours(contents),
        _ => [[1.0; 4]; 2],
    };
    let tinted = [fill, edge, edge, fill];
    std::array::from_fn(|k| Material {
        name: lumit_core::extrude::FACES[k].to_owned(),
        base_colour: [tinted[k][0], tinted[k][1], tinted[k][2], 1.0],
        metallic: 0.0,
        roughness: 0.5,
        soft_turns: true,
        ..Material::default()
    })
}

fn shape_colours(contents: &[ShapeEntry]) -> [[f32; 4]; 2] {
    use lumit_core::shape::colour;
    let items = lumit_core::shape::modifier::items(contents);
    let Some(item) = items.first() else {
        return [[1.0; 4]; 2];
    };
    let fill = item.fill.as_ref().map(|c| colour::at(c, 0.0).0);
    let outline = item.stroke.as_ref().map(|c| colour::at(c, 0.0).0);
    let fill = fill.or(outline).unwrap_or([1.0; 4]);
    [fill, outline.unwrap_or(fill)]
}

/// The name of an outline made solid by `profile`. Nearby numbers name the
/// same outline, which keeps a mesh from depending on what rounding left
/// behind in the last few bits of a number.
fn mesh_key(contours: &[Vec<[f64; 2]>], profile: &Profile) -> u128 {
    let mut h = blake3::Hasher::new();
    h.update(b"extrude/");
    for v in [profile.depth, profile.bevel, profile.hole] {
        h.update(&v.to_le_bytes());
    }
    h.update(&[profile.style as u8]);
    h.update(&profile.segments.to_le_bytes());
    for contour in contours {
        h.update(&(contour.len() as u32).to_le_bytes());
        for p in contour {
            for v in p {
                h.update(&quantised(*v).to_le_bytes());
            }
        }
    }
    name_of(&h)
}

fn name_of(h: &blake3::Hasher) -> u128 {
    let mut k = [0u8; 16];
    k.copy_from_slice(&h.finalize().as_bytes()[..16]);
    u128::from_le_bytes(k)
}

/// A coordinate to the nearest 256th of a pixel.
fn quantised(v: f64) -> i64 {
    (v * 256.0).round() as i64
}

/// A piece moved to sit by its own corner, so the same letter anywhere on a
/// whole pixel is the same mesh, and the corner it moved from.
fn at_own_corner(contours: &[Vec<[f64; 2]>]) -> (Vec<Vec<[f64; 2]>>, [f64; 2]) {
    let finite = contours
        .iter()
        .flatten()
        .filter(|p| p[0].is_finite() && p[1].is_finite());
    let (mut x, mut y) = (f64::MAX, f64::MAX);
    for p in finite {
        (x, y) = (x.min(p[0]), y.min(p[1]));
    }
    if x == f64::MAX {
        return (contours.to_vec(), [0.0; 2]);
    }
    let corner = [x.floor(), y.floor()];
    let moved = contours
        .iter()
        .map(|c| {
            c.iter()
                .map(|p| {
                    [
                        quantised(p[0] - corner[0]) as f64 / 256.0,
                        quantised(p[1] - corner[1]) as f64 / 256.0,
                    ]
                })
                .collect()
        })
        .collect();
    (moved, corner)
}

/// Where a piece's mesh goes: back to its corner, then as its animators ask,
/// turning and scaling about its pivot at half its depth, and how its
/// directions go with it.
fn placed(piece: &Piece, corner: [f64; 2], total: f32) -> (Mat4, Mat3, Mat3) {
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in piece.contours.iter().flatten() {
        (min[0], min[1]) = (min[0].min(p[0] as f32), min[1].min(p[1] as f32));
        (max[0], max[1]) = (max[0].max(p[0] as f32), max[1].max(p[1] as f32));
    }
    let x = piece.xform;
    let [px, py] = piece
        .pivot
        .unwrap_or([(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0]);
    let middle = (px, py, total / 2.0);
    let at = (
        middle.0 + x.position[0],
        middle.1 + x.position[1],
        middle.2 + x.depth - piece.lift,
    );
    let scale = |v: f32| if v.abs() < 1e-4 { 1e-2 } else { v * 100.0 };
    let m = Mat4::from_cols_array_2d(&lumit_gpu::composite::place_matrix_3d(
        at,
        middle,
        (scale(x.scale[0]), scale(x.scale[1]), 100.0),
        (x.turn[0], x.turn[1], x.rotation),
    )) * Mat4::from_translation(Vec3::new(corner[0] as f32, corner[1] as f32, 0.0));
    let along = Mat3::from_mat4(m);
    (m, along, along.inverse().transpose())
}

fn turned(by: &Mat3, d: [f32; 3]) -> [f32; 3] {
    (*by * Vec3::from(d)).normalize_or_zero().to_array()
}

/// `pieces` made solid by `profile` and put together as one scene: one mesh
/// with a primitive for each face, whose colours are its vertices' where
/// `tinted` says so and white where a face's own rows give it its colour.
fn scene_of(pieces: &[Piece], profile: &Profile, tinted: [bool; 4]) -> Arc<ModelScene> {
    let made: Vec<(Arc<Mesh>, [f64; 2], u128)> = pieces
        .iter()
        .map(|p| {
            let (contours, corner) = at_own_corner(&p.contours);
            let key = mesh_key(&contours, profile);
            let mesh = kept(&MESHES, MESH_BYTES, key, || mesh::solid(&contours, profile));
            (mesh, corner, key)
        })
        .collect();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in pieces.iter().flat_map(|p| &p.contours).flatten() {
        (lo[0], lo[1]) = (lo[0].min(p[0] as f32), lo[1].min(p[1] as f32));
        (hi[0], hi[1]) = (hi[0].max(p[0] as f32), hi[1].max(p[1] as f32));
    }
    let total = (profile.depth + profile.bevel * 2.0).max(1.0) as f32;
    let mut h = blake3::Hasher::new();
    h.update(b"extrude-scene/");
    h.update(&tinted.map(u8::from));
    h.update(&total.to_le_bytes());
    for (piece, (_, corner, key)) in pieces.iter().zip(&made) {
        h.update(&key.to_le_bytes());
        h.update(&corner[0].to_le_bytes());
        h.update(&corner[1].to_le_bytes());
        for v in piece.fill.iter().chain(&piece.edge) {
            h.update(&v.to_le_bytes());
        }
        let x = piece.xform;
        for v in [
            x.position[0],
            x.position[1],
            x.rotation,
            x.scale[0],
            x.scale[1],
            x.depth,
            x.turn[0],
            x.turn[1],
            piece.lift,
            piece.pivot.map_or(f32::MIN, |p| p[0]),
            piece.pivot.map_or(f32::MIN, |p| p[1]),
        ] {
            h.update(&v.to_le_bytes());
        }
    }
    kept(&SCENES, SCENE_BYTES, name_of(&h), || {
        let size = [(hi[0] - lo[0]).max(1.0), (hi[1] - lo[1]).max(1.0)];
        let mut primitives: Vec<Primitive> = (0..4)
            .map(|k| Primitive {
                material: Some(k),
                ..Primitive::default()
            })
            .collect();
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (piece, (m, corner, _)) in pieces.iter().zip(&made) {
            let (place, along, across) = placed(piece, *corner, total);
            let shift = [corner[0] as f32, corner[1] as f32];
            for (k, (face, into)) in m.faces.iter().zip(&mut primitives).enumerate() {
                let Ok(base) = u32::try_from(into.positions.len()) else {
                    continue;
                };
                let cap = k == mesh::FRONT || k == mesh::BACK;
                let colour = match (tinted[k], cap) {
                    (true, true) => piece.fill,
                    (true, false) => piece.edge,
                    (false, _) => [1.0, 1.0, 1.0, piece.fill[3]],
                };
                into.positions.extend(face.positions.iter().map(|p| {
                    let p = place.transform_point3(Vec3::from(*p)).to_array();
                    for a in 0..3 {
                        low[a] = low[a].min(p[a]);
                        high[a] = high[a].max(p[a]);
                    }
                    p
                }));
                into.normals
                    .extend(face.normals.iter().map(|n| turned(&across, *n)));
                into.tangents.extend(face.tangents.iter().map(|t| {
                    let [x, y, z] = turned(&along, [t[0], t[1], t[2]]);
                    [x, y, z, t[3]]
                }));
                into.uv0.extend(face.uv0.iter().map(|uv| {
                    if cap {
                        [
                            (uv[0] + shift[0] - lo[0]) / size[0],
                            (uv[1] + shift[1] - lo[1]) / size[1],
                        ]
                    } else {
                        [uv[0] / size[0], uv[1] / total]
                    }
                }));
                into.colours
                    .extend(std::iter::repeat_n(colour, face.positions.len()));
                into.indices.extend(face.indices.iter().map(|i| base + i));
            }
        }
        if low[0] > high[0] {
            (low, high) = ([0.0; 3], [0.0; 3]);
        }
        ModelScene {
            nodes: vec![Node {
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
                mesh: Some(0),
                ..Node::default()
            }],
            roots: vec![0],
            meshes: vec![lumit_scene::Mesh {
                name: String::new(),
                primitives,
            }],
            // White, as a vertex or a look gives each face its colour.
            materials: std::array::from_fn::<_, 4, _>(|k| Material {
                name: lumit_core::extrude::FACES[k].to_owned(),
                metallic: 0.0,
                roughness: 0.5,
                soft_turns: true,
                ..Material::default()
            })
            .to_vec(),
            bounds: Bounds {
                min: low,
                max: high,
            },
            ..ModelScene::default()
        }
    })
}

/// Whether `layer`'s outline can differ across a shutter, which is worth
/// asking at each sample only when it can: its letters move or its art or
/// rows are keyed or driven.
pub(crate) fn may_change(layer: &Layer, lt: f64, context: &Arc<ExpressionContext>) -> bool {
    let keyed = lumit_core::extrude::kept(layer)
        .first()
        .and_then(Option::as_ref);
    let keyed = keyed.is_some_and(|rows| {
        rows.params
            .iter()
            .any(|p| matches!(&p.value, lumit_core::model::EffectValue::Float(v) if !matches!(v.animation, lumit_core::anim::Animation::Static(_))))
    });
    keyed
        || match &layer.kind {
            LayerKind::Text { document } => {
                !document.animators.is_empty() || document.expression.is_some()
            }
            LayerKind::Shape { contents } => matches!(
                lumit_core::shape::contents_at(contents, lt, context),
                std::borrow::Cow::Owned(_)
            ),
            _ => false,
        }
}

/// `layer`'s scene at layer time `lt`, for a shutter sample, coloured as
/// `made` has it.
pub(crate) fn scene_at(
    layer: &Layer,
    lt: f64,
    context: &Arc<ExpressionContext>,
    tinted: [bool; 4],
) -> Option<Arc<ModelScene>> {
    let options = lumit_core::extrude::kept(layer).first()?.as_ref()?;
    let read = |p: &lumit_core::anim::Property| p.value_at_with_context(lt, Arc::clone(context));
    let profile = Profile::of(options, &read);
    Some(scene_of(&pieces(layer, lt, context), &profile, tinted))
}

/// A solid layer as the mesh pass takes it.
pub(crate) struct Made {
    pub scene: Arc<ModelScene>,
    pub looks: Vec<Look>,
    /// Which faces' colours its vertices carry.
    pub tinted: [bool; 4],
}

/// Layer `layer` made solid at layer time `lt`, `None` when it has no Geometry
/// options to go by. `picture` resolves a row naming a layer.
pub(crate) fn made(
    layer: &Layer,
    lt: f64,
    context: &Arc<ExpressionContext>,
    picture: &dyn Fn(&EffectInstance, &str) -> Option<crate::draw::DofInputDraw>,
) -> Option<Made> {
    let kept_rows = lumit_core::extrude::kept(layer);
    let options = kept_rows.first()?.as_ref()?;
    let read = |p: &lumit_core::anim::Property| p.value_at_with_context(lt, Arc::clone(context));
    let profile = Profile::of(options, &read);
    let pieces = pieces(layer, lt, context);
    let file = faces(layer);
    let rows = kept_rows.get(1..).unwrap_or(&[]);
    let mut looks = crate::model_material::looks_of(rows, &file, lt, context, picture);
    let plain = crate::model_material::plain_faces(layer.id, rows, &file);
    let mut tinted = [true; 4];
    for look in &mut looks {
        match plain.get(look.index) {
            // Its rows don't change its colour, which its vertices carry.
            Some(true) => {
                let a = look.material.base_colour[3];
                look.material.base_colour = [1.0, 1.0, 1.0, a];
            }
            _ => {
                if let Some(t) = tinted.get_mut(look.index) {
                    *t = false;
                }
            }
        }
    }
    // A letter part way through fading in blends.
    let fading = pieces.iter().any(|p| p.fill[3] < 1.0);
    if fading {
        for index in 0..4 {
            if !looks.iter().any(|l| l.index == index) {
                looks.push(Look {
                    index,
                    material: Material {
                        metallic: 0.0,
                        roughness: 0.5,
                        soft_turns: true,
                        ..Material::default()
                    },
                    pictures: Vec::new(),
                    textures: true,
                });
            }
        }
        for look in &mut looks {
            look.material.alpha_mode = AlphaMode::Blend;
        }
    }
    let scene = scene_of(&pieces, &profile, tinted);
    if let Ok(mut last) = LAST.lock() {
        let last = last.get_or_insert_with(HashMap::new);
        if last.len() > 4096 {
            last.retain(|_, s| s.strong_count() > 0);
        }
        last.insert(layer.id, Arc::downgrade(&scene));
    }
    Some(Made {
        scene,
        looks,
        tinted,
    })
}

/// The scene layer `layer` was last drawn as, while it's kept.
#[must_use]
pub fn last_scene(layer: Uuid) -> Option<Arc<ModelScene>> {
    LAST.lock().ok()?.as_ref()?.get(&layer)?.upgrade()
}

/// The eight corners of the box layer `layer` was last drawn in, in its own
/// pixels, numbered by the bits of x, y and z.
#[must_use]
pub fn layer_box(layer: Uuid) -> Option<[[f32; 3]; 8]> {
    let bounds = last_scene(layer)?.bounds;
    Some(std::array::from_fn(|i| {
        std::array::from_fn(|k| match i >> k & 1 {
            0 => bounds.min[k],
            _ => bounds.max[k],
        })
    }))
}

/// How far along the ray from `origin` by `direction`, in its own pixels,
/// layer `layer` as last drawn is first met, in parts of `direction`.
#[must_use]
pub fn hit(layer: Uuid, origin: [f32; 3], direction: [f32; 3]) -> Option<f32> {
    let scene = last_scene(layer)?;
    crate::model::scene_hit(&scene, glam::Mat4::IDENTITY, origin, direction)
}

/// Layer `layer`'s rows: its options, then, once it has a depth or a bevel,
/// its faces'.
#[must_use]
pub fn rows(layer: &Layer) -> Vec<EffectInstance> {
    let kept_rows = lumit_core::extrude::kept(layer);
    let mut rows: Vec<EffectInstance> =
        lumit_core::fx::builtins::instantiate(lumit_core::extrude::MATCH_NAME)
            .map(|rows| options_named(layer.id, kept_rows.first().and_then(Option::as_ref), rows))
            .into_iter()
            .collect();
    if lumit_core::extrude::is_extruded(layer) {
        let tail = kept_rows.get(1..).unwrap_or(&[]);
        rows.extend(crate::model_material::rows_from(
            layer.id,
            tail,
            &faces(layer),
            1,
        ));
    }
    rows
}

/// The options' rows as a layer's list carries them: what it keeps, else the
/// defaults, under this layer's id.
pub(crate) fn options_named(
    layer: Uuid,
    kept: Option<&EffectInstance>,
    defaults: EffectInstance,
) -> EffectInstance {
    let rows = kept.cloned().unwrap_or(defaults);
    EffectInstance {
        id: lumit_core::model_material::material_id(layer, 0),
        custom_name: Some("Geometry options".to_owned()),
        ..rows
    }
}

/// What layer `layer`'s row list says beyond its defaults, for it to keep:
/// each row by its place, `None` where it's as it was.
#[must_use]
pub fn kept_of(layer: &Layer, list: &[EffectInstance]) -> Option<Vec<Option<EffectInstance>>> {
    let file = faces(layer);
    let mut out: Vec<Option<EffectInstance>> = vec![None; 5];
    for rows in list {
        let i = lumit_core::model_material::index_of(layer.id, rows.id)?;
        let baseline = file_rows(layer.id, i, &file)?;
        let slot = out.get_mut(i)?;
        if rows.params != baseline.params || !rows.enabled {
            *slot = Some(rows.clone());
        }
    }
    // Rows the list doesn't carry, as a flat layer's faces, stay as kept.
    for (i, kept) in lumit_core::extrude::kept(layer).iter().enumerate() {
        if let (Some(slot), false) = (out.get_mut(i), list_holds(list, layer.id, i)) {
            slot.clone_from(kept);
        }
    }
    while out.last().is_some_and(Option::is_none) {
        out.pop();
    }
    Some(out)
}

fn list_holds(list: &[EffectInstance], layer: Uuid, i: usize) -> bool {
    list.iter()
        .any(|r| lumit_core::model_material::index_of(layer, r.id) == Some(i))
}

/// Row `index` of layer `layer` as it starts: the options' defaults or a
/// face's own colours.
pub(crate) fn file_rows(layer: Uuid, index: usize, file: &[Material; 4]) -> Option<EffectInstance> {
    match index.checked_sub(1) {
        None => lumit_core::fx::builtins::instantiate(lumit_core::extrude::MATCH_NAME)
            .map(|rows| options_named(layer, None, rows)),
        Some(face) => crate::model_material::offered(layer, index, file.get(face)?),
    }
}
