//! Clone deformer: changes the copies a Clone to points above it drew, as one
//! group.
//!
//! **In plain terms.** Put it under a Clone to points, and Align moves the
//! whole group of copies so its bounds line up on a point, left, centred or
//! right, top, centred or bottom. Boolean cuts the group with another layer:
//! Union adds the layer, Subtract cuts it away, Intersect keeps only where both
//! are, Difference keeps where one is but not both, and Subtract from layer
//! cuts the copies out of the layer instead, in the layer's colours. Round
//! corners rounds corners by Radius, as Cavalry's Bevel. Several of them in a
//! row are a stack, each working on what the one above left; in Vector the
//! whole stack is worked out on the paths while every copy is opaque.
//!
//! **Mode** says what the group is read as. Raster reads the picture: the
//! bounds and the cut follow its pixels, so it works on any layer and after
//! any effect. Vector reads the outlines of Shape layers: the copies' edges,
//! their bounds and the cut follow the paths themselves, and the colours still
//! come from the picture. It wants the copies' outlines, so it needs a Clone to
//! points of Shape layers right above it, with only Clone deformers between.
//! The first Vector deformer under it reads the copies' outlines; one further
//! down reads the edges the first one left. The Boolean layer is read by its
//! outline when it is a Shape layer or has a mask, and by its picture
//! otherwise. Anywhere else Vector reads the picture, as Raster does.
//!
//! The outlines are the paths' own lines, filled: a stroke's width is not in
//! them, so a Shape layer of strokes alone is filled in Vector.

use crate::fx::builtins::MAP_PLACEMENT_OPTIONS;
use crate::fx::points::PointsStream;
use crate::fx::{
    params::ParamId, params::Params, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, schema::EffectSchema, schema::EnabledCond, schema::EnabledWhen,
    schema::ParamGroup,
};
use crate::mask::MaskPolyline;
use lumit_fx_macros::Effect;

use super::clone_to_points::CloneToPoints;

pub mod round;
use round::rounded;

/// The effect's match name.
pub const MATCH_NAME: &str = "clone_deformer";

/// Mode: read the picture.
pub const MODE_RASTER: u32 = 0;
/// Mode: read the Shape layers' outlines.
pub const MODE_VECTOR: u32 = 1;

/// Deformer: line the group's bounds up on Position.
pub const DEFORM_ALIGN: u32 = 0;
/// Deformer: cut the group with a layer.
pub const DEFORM_BOOLEAN: u32 = 1;
/// Deformer: round the group's outward corners.
pub const DEFORM_ROUND: u32 = 2;

/// The Boolean layer row.
pub const BOOLEAN_LAYER: &str = "boolean_layer";

const fn group(
    label: &'static str,
    params: &'static [&'static str],
    visible_when: Option<(&'static str, &'static [u32])>,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when,
        visible_when_lens_elements: None,
    }
}

/// Align's rows show for Align, Boolean's for Boolean.
pub const DEFORMER_GROUPS: &[ParamGroup] = &[
    group(
        "Align",
        &[
            "align_horizontal",
            "align_vertical",
            "position_x",
            "position_y",
        ],
        Some(("deformer", &[DEFORM_ALIGN])),
    ),
    group(
        "Boolean",
        &["boolean_layer", "boolean_layer_placement", "operation"],
        Some(("deformer", &[DEFORM_BOOLEAN])),
    ),
    group(
        "Round corners",
        &["radius"],
        Some(("deformer", &[DEFORM_ROUND])),
    ),
];

/// A Position row is not read while its axis is None.
pub const DEFORMER_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "position_x",
        on: "align_horizontal",
        cond: EnabledCond::ChoiceIsNot(0),
    },
    EnabledWhen {
        param: "position_y",
        on: "align_vertical",
        cond: EnabledCond::ChoiceIsNot(0),
    },
];

/// Clone deformer's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "clone_deformer",
    label = "Clone deformer",
    version = 1,
    category = Points,
    cost = Moderate,
    // Align moves the picture anywhere in the frame.
    roi = FullFrame,
    premultiplied = true,
    seeded = false,
    groups = DEFORMER_GROUPS,
    enabled_when = DEFORMER_ENABLED_WHEN,
)]
pub struct CloneDeformer {
    /// What the group is read as: its picture, or its Shape layers'
    /// outlines. A Choice is stored as its index, so a new one goes on the end.
    #[choice(label = "Mode", options = ["Raster", "Vector"], default = 0)]
    pub mode: u32,

    /// What is done to the group.
    #[choice(label = "Deformer", options = ["Align", "Boolean", "Round corners"], default = 0)]
    pub deformer: u32,

    /// Which side of the group's bounds goes on Position x. None leaves the
    /// group where it is across.
    #[choice(
        label = "Horizontal",
        options = ["None", "Left", "Centre", "Right"],
        default = 2
    )]
    pub align_horizontal: u32,

    /// Which side of the group's bounds goes on Position y. None leaves the
    /// group where it is down.
    #[choice(
        label = "Vertical",
        options = ["None", "Top", "Centre", "Bottom"],
        default = 2
    )]
    pub align_vertical: u32,

    /// Where the group is lined up across, px@comp in the layer.
    #[slider(label = "Position x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub position_x: f32,

    /// Where the group is lined up down, px@comp in the layer.
    #[slider(label = "Position y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub position_y: f32,

    /// The layer the group is cut with. Unset cuts nothing.
    #[layer(label = "Boolean layer", self_default = false)]
    pub boolean_layer: bool,

    /// Where the Boolean layer sits on this one: as it sits in the comp,
    /// stretched over the frame, or centred at its own size.
    #[choice(label = "Map placement", options = *MAP_PLACEMENT_OPTIONS, default = 0)]
    pub boolean_layer_placement: u32,

    /// How the layer cuts the group.
    #[choice(
        label = "Operation",
        options = ["Union", "Subtract", "Intersect", "Difference", "Subtract from layer"],
        default = 1
    )]
    pub operation: u32,

    /// How round Round corners makes each outward corner, px@comp: the
    /// radius of the circle that fits in it.
    #[slider(label = "Radius", min = 0.0, max = 200.0, default = 20.0, hard_min = 0.0, unit = Px)]
    pub radius: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

/// Closed contours, filled by winding, px@comp.
pub type Outline = Vec<Vec<[f32; 2]>>;

/// One copy a Clone to points stamped, as outlines: px@comp on the layer,
/// where the camera puts them, and how opaque the copy is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeCopy {
    pub contours: Outline,
    pub opacity: f32,
}

/// The copies the Clone to points above stamped, as the first Vector deformer
/// under it reads them. The draw builder makes them, as it makes the stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CloneShapes {
    pub copies: Vec<ShapeCopy>,
    /// What this deformer leaves, as outlines, when every Vector deformer
    /// above it back to the Clone to points could be worked out on the paths:
    /// the cut and the rounding are then the paths' own, not the pixels'.
    pub result: Option<Outline>,
}

/// The Boolean layer's outline, as Vector reads it: in its own px with its
/// own size, for Stretch to fit and Centre, and carried into this layer's px
/// for As placed in comp.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BooleanShape {
    pub own: Outline,
    pub size: [f32; 2],
    pub placed: Outline,
}

/// A path walk broken back into the contours it was made of: a step with no
/// length is where one path of a run ends and the next begins.
#[must_use]
pub fn contours_of(line: &MaskPolyline) -> Outline {
    let mut out: Outline = Vec::new();
    let mut run: Vec<[f32; 2]> = Vec::new();
    for (i, p) in line.points.iter().enumerate() {
        let joins = i > 0 && line.arc.get(i) <= line.arc.get(i - 1);
        if joins && run.len() >= 3 {
            out.push(std::mem::take(&mut run));
        } else if joins {
            run.clear();
        }
        run.push(*p);
    }
    if run.len() >= 3 {
        out.push(run);
    }
    out
}

/// Cut a contour to the box `lo`–`hi` (Sutherland–Hodgman). A concave one
/// comes back with edges along the box, which fill as nothing.
fn clipped(contour: &[[f32; 2]], lo: [f32; 2], hi: [f32; 2]) -> Vec<[f32; 2]> {
    let mut out = contour.to_vec();
    for axis in 0..2 {
        for (bound, keep_above) in [(lo[axis], true), (hi[axis], false)] {
            let inside = |p: &[f32; 2]| (p[axis] >= bound) == keep_above || p[axis] == bound;
            let input = std::mem::take(&mut out);
            for (i, b) in input.iter().enumerate() {
                let a = input[(i + input.len() - 1) % input.len()];
                let cross = |a: [f32; 2], b: [f32; 2]| {
                    let t = (bound - a[axis]) / (b[axis] - a[axis]);
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                };
                match (inside(&a), inside(b)) {
                    (true, true) => out.push(*b),
                    (true, false) => out.push(cross(a, *b)),
                    (false, true) => {
                        out.push(cross(a, *b));
                        out.push(*b);
                    }
                    (false, false) => {}
                }
            }
            if out.is_empty() {
                return out;
            }
        }
    }
    out
}

/// The copies `clone` stamps at `stamps`, as outlines, the way the draw lays a
/// sprite on its point: `pictures[k]` is picture k's outline in its own px
/// and its own size, and every stamp's picture must have one.
#[must_use]
pub fn shape_copies(
    clone: CloneToPoints,
    stamps: &PointsStream,
    pictures: &[(Outline, [f32; 2])],
) -> Vec<ShapeCopy> {
    let mix = clone.draw_style().mix;
    let mut out = Vec::with_capacity(stamps.len());
    for i in 0..stamps.len() {
        let (Some(&head3), Some(&size), Some(&colour)) = (
            stamps.position.get(i),
            stamps.size.get(i),
            stamps.colour.get(i),
        ) else {
            continue;
        };
        let Some((outline, own)) = pictures.get(stamps.index_of(i) as usize) else {
            continue;
        };
        let (head, depth) = stamps.projection.apply(head3);
        let size = size * depth;
        let opacity = colour[3] * mix;
        if size <= 0.0 || opacity <= 0.0 {
            continue;
        }
        let fit = clone.fit(*own, 1.0);
        let stretch = stamps.stretch_of(i);
        let (s, c) = stamps.rotation.get(i).copied().unwrap_or(0.0).sin_cos();
        let span = [
            size * stretch[0] * fit.unit[0],
            size * stretch[1] * fit.unit[1],
        ];
        // The part of the picture the stamp shows, in its own px.
        let lo = [
            own[0] * (0.5 - 0.5 * fit.uv[0]).max(0.0),
            own[1] * (0.5 - 0.5 * fit.uv[1]).max(0.0),
        ];
        let hi = [
            own[0] * (0.5 + 0.5 * fit.uv[0]).min(1.0),
            own[1] * (0.5 + 0.5 * fit.uv[1]).min(1.0),
        ];
        let place = |q: [f32; 2]| {
            let local = [0, 1].map(|k| {
                let tu = q[k] / own[k].max(1e-6);
                let u = 0.5 + (tu - 0.5) / fit.uv[k].max(1e-6);
                (u - fit.anchor[k]) * span[k]
            });
            [
                head[0] + local[0] * c - local[1] * s,
                head[1] + local[0] * s + local[1] * c,
            ]
        };
        let cut = fit.uv[0] < 1.0 || fit.uv[1] < 1.0;
        let contours = outline
            .iter()
            .map(|contour| match cut {
                true => clipped(contour, lo, hi),
                false => contour.clone(),
            })
            .filter(|contour| contour.len() >= 3)
            .map(|contour| contour.into_iter().map(place).collect())
            .collect();
        out.push(ShapeCopy { contours, opacity });
    }
    out
}

/// Coverage of `contours` filled by winding, each point taken to `p * k +
/// offset` in a `w`×`h` raster, laid over `into` at `opacity`: the next copy
/// goes over what is there, as a stamp does.
pub fn cover(
    into: &mut [f32],
    w: u32,
    h: u32,
    contours: &[Vec<[f32; 2]>],
    k: f32,
    offset: [f32; 2],
    opacity: f32,
) {
    let edges: Vec<[[f32; 2]; 2]> = contours
        .iter()
        .filter(|c| c.len() >= 3)
        .flat_map(|c| {
            (0..c.len()).map(move |i| {
                let at = |p: [f32; 2]| [p[0] * k + offset[0], p[1] * k + offset[1]];
                [at(c[i]), at(c[(i + 1) % c.len()])]
            })
        })
        .filter(|[a, b]| a[1] != b[1] && a.iter().chain(b).all(|v| v.is_finite()))
        .collect();
    if edges.is_empty() || w == 0 || h == 0 {
        return;
    }
    let lo_y = edges
        .iter()
        .map(|e| e[0][1].min(e[1][1]))
        .fold(f32::MAX, f32::min);
    let hi_y = edges
        .iter()
        .map(|e| e[0][1].max(e[1][1]))
        .fold(f32::MIN, f32::max);
    let (y0, y1) = (
        (lo_y.floor().max(0.0) as u32).min(h),
        (hi_y.ceil().max(0.0) as u32).min(h),
    );
    // Four lines across each row, each crossing summed with its winding.
    const SUBS: usize = 4;
    let mut row = vec![0.0f32; w as usize];
    let mut xs: Vec<(f32, i32)> = Vec::new();
    for y in y0..y1 {
        let mut touched = false;
        for sub in 0..SUBS {
            let sy = y as f32 + (sub as f32 + 0.5) / SUBS as f32;
            xs.clear();
            for [a, b] in &edges {
                let (top, bottom) = (a[1].min(b[1]), a[1].max(b[1]));
                if sy < top || sy >= bottom {
                    continue;
                }
                let x = a[0] + (sy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                xs.push((x, if b[1] > a[1] { 1 } else { -1 }));
            }
            if xs.len() < 2 {
                continue;
            }
            touched = true;
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for pair in xs.windows(2) {
                winding += pair[0].1;
                if winding == 0 {
                    continue;
                }
                let (start, end) = (pair[0].0.max(0.0), pair[1].0.min(w as f32));
                if end <= start {
                    continue;
                }
                let last = (end.ceil() as usize).min(w as usize);
                for (px, cell) in row
                    .iter_mut()
                    .enumerate()
                    .take(last)
                    .skip(start.floor() as usize)
                {
                    let l = start.max(px as f32);
                    let r = end.min(px as f32 + 1.0);
                    if r > l {
                        *cell += (r - l) / SUBS as f32;
                    }
                }
            }
        }
        if !touched {
            continue;
        }
        let base = (y * w) as usize;
        for (px, c) in row.iter_mut().enumerate() {
            let c = std::mem::take(c).clamp(0.0, 1.0) * opacity;
            if c > 0.0 {
                let to = &mut into[base + px];
                *to += c * (1.0 - *to);
            }
        }
    }
}

/// The box round everything with any alpha, `[x0, y0, x1, y1]` in raster px.
#[must_use]
pub fn alpha_bounds(alpha: impl Fn(usize) -> f32, w: u32, h: u32) -> Option<[f32; 4]> {
    let mut b: Option<[f32; 4]> = None;
    for y in 0..h {
        for x in 0..w {
            if alpha((y * w + x) as usize) > 1.0 / 512.0 {
                let (x, y) = (x as f32, y as f32);
                b = Some(match b {
                    None => [x, y, x + 1.0, y + 1.0],
                    Some(b) => [
                        b[0].min(x),
                        b[1].min(y),
                        b[2].max(x + 1.0),
                        b[3].max(y + 1.0),
                    ],
                });
            }
        }
    }
    b
}

/// The box round outlines taken to `p * k`, `[x0, y0, x1, y1]`.
#[must_use]
pub fn outline_bounds<'a>(
    contours: impl IntoIterator<Item = &'a Vec<[f32; 2]>>,
    k: f32,
) -> Option<[f32; 4]> {
    let mut b: Option<[f32; 4]> = None;
    for p in contours.into_iter().flatten() {
        let (x, y) = (p[0] * k, p[1] * k);
        b = Some(match b {
            None => [x, y, x, y],
            Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
        });
    }
    b
}

/// `n`-channel `pixels` moved by `d` raster px, read bilinearly. What moves
/// in from past the edge is empty.
#[must_use]
pub fn shifted(pixels: &[f32], n: usize, w: u32, h: u32, d: [f32; 2]) -> Vec<f32> {
    let (w, h) = (w as i64, h as i64);
    let mut out = vec![0.0; pixels.len()];
    let (fx, fy) = (d[0].floor(), d[1].floor());
    let (ax, ay) = (d[0] - fx, d[1] - fy);
    let (ix, iy) = (fx as i64, fy as i64);
    let at = |x: i64, y: i64, c: usize| {
        if x < 0 || y < 0 || x >= w || y >= h {
            0.0
        } else {
            pixels[((y * w + x) as usize) * n + c]
        }
    };
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (x - ix, y - iy);
            for c in 0..n {
                let top = at(sx, sy, c) * (1.0 - ax) + at(sx - 1, sy, c) * ax;
                let bottom = at(sx, sy - 1, c) * (1.0 - ax) + at(sx - 1, sy - 1, c) * ax;
                out[((y * w + x) as usize) * n + c] = top * (1.0 - ay) + bottom * ay;
            }
        }
    }
    out
}

/// What a Clone deformer reads beside its picture: the copies' outlines
/// (only the first Vector deformer under a Clone to points of Shape layers
/// has them), the Boolean layer's picture at this raster and its outline.
#[derive(Default)]
pub struct Reads<'a> {
    pub copies: Option<&'a [ShapeCopy]>,
    pub other: Option<&'a [f32]>,
    pub boolean: Option<&'a BooleanShape>,
    pub result: Option<&'a [Vec<[f32; 2]>]>,
}

impl CloneDeformer {
    /// How far Align moves a group whose bounds are `b`, raster px. The
    /// Position rows are raster px already; `grow` is the room round the
    /// layer's art the picture was given.
    #[must_use]
    pub fn align_shift(self, b: [f32; 4], grow: [f32; 2]) -> [f32; 2] {
        let to = [self.position_x + grow[0], self.position_y + grow[1]];
        let side = |choice: u32, lo: f32, hi: f32, to: f32| match choice {
            1 => to - lo,
            2 => to - (lo + hi) * 0.5,
            3 => to - hi,
            _ => 0.0,
        };
        [
            side(self.align_horizontal, b[0], b[2], to[0]),
            side(self.align_vertical, b[1], b[3], to[1]),
        ]
    }

    /// The Boolean layer's outline in this raster, by Map placement. `None`
    /// where the layer has no outline. `k` is the raster factor and `grow`
    /// the room round the art.
    fn boolean_cover(
        self,
        shape: &BooleanShape,
        w: u32,
        h: u32,
        k: f32,
        grow: [f32; 2],
    ) -> Option<Vec<f32>> {
        let mut c = vec![0.0; (w * h) as usize];
        // The frame the layer is laid on, px@comp.
        let frame = [
            (w as f32 - 2.0 * grow[0]) / k.max(1e-6),
            (h as f32 - 2.0 * grow[1]) / k.max(1e-6),
        ];
        let (contours, scale, offset) = match self.boolean_layer_placement {
            0 => (&shape.placed, [1.0; 2], [0.0; 2]),
            1 => (
                &shape.own,
                [0, 1].map(|i| frame[i] / shape.size[i].max(1e-6)),
                [0.0; 2],
            ),
            _ => (
                &shape.own,
                [1.0; 2],
                [0, 1].map(|i| (frame[i] - shape.size[i]) * 0.5),
            ),
        };
        if contours.is_empty() {
            return None;
        }
        let moved: Outline = contours
            .iter()
            .map(|c| {
                c.iter()
                    .map(|p| [p[0] * scale[0] + offset[0], p[1] * scale[1] + offset[1]])
                    .collect()
            })
            .collect();
        cover(&mut c, w, h, &moved, k, grow, 1.0);
        Some(c)
    }

    /// The deformer on a premultiplied `w`×`h` picture at raster factor `k`,
    /// `grow` the room round the art.
    pub fn apply(
        self,
        rgba: &mut Vec<f32>,
        w: u32,
        h: u32,
        k: f32,
        grow: [f32; 2],
        reads: &Reads<'_>,
    ) {
        let n = (w * h) as usize;
        if rgba.len() < n * 4 {
            return;
        }
        let before = (self.mix < 100.0).then(|| rgba.clone());
        let vector = self.mode == MODE_VECTOR;
        // The group's coverage as Vector reads it: the copies' outlines where
        // they came, and the edges an earlier deformer left otherwise.
        let copies = reads.copies.filter(|_| vector);
        let mut edge: Option<Vec<f32>> = vector.then(|| {
            let mut c = vec![0.0; n];
            match copies {
                Some(copies) => {
                    for copy in copies {
                        cover(&mut c, w, h, &copy.contours, k, grow, copy.opacity);
                    }
                }
                None => {
                    for (i, a) in c.iter_mut().enumerate() {
                        *a = rgba[i * 4 + 3];
                    }
                }
            }
            c
        });
        match self.deformer {
            DEFORM_ALIGN => {
                let bounds = match copies {
                    Some(copies) => {
                        outline_bounds(copies.iter().flat_map(|c| &c.contours), k).map(|b| {
                            [
                                b[0] + grow[0],
                                b[1] + grow[1],
                                b[2] + grow[0],
                                b[3] + grow[1],
                            ]
                        })
                    }
                    None => alpha_bounds(|i| rgba[i * 4 + 3], w, h),
                };
                if let Some(b) = bounds {
                    let d = self.align_shift(b, grow);
                    if d != [0.0; 2] {
                        *rgba = shifted(rgba, 4, w, h, d);
                        edge = match (copies, edge) {
                            // Drawn again in the new place, so the edge stays sharp.
                            (Some(copies), Some(_)) => {
                                let mut c = vec![0.0; n];
                                for copy in copies {
                                    let at = [d[0] + grow[0], d[1] + grow[1]];
                                    cover(&mut c, w, h, &copy.contours, k, at, copy.opacity);
                                }
                                Some(c)
                            }
                            (_, edge) => edge.map(|e| shifted(&e, 1, w, h, d)),
                        };
                    }
                }
            }
            DEFORM_ROUND => {
                if reads.result.is_some() && vector {
                    // The rounded outline is drawn below; the colours stay.
                    finish(
                        rgba,
                        Some(&outline_cover(reads, w, h, k, grow)),
                        before.as_deref(),
                        self.mix,
                    );
                    return;
                }
                // Vector rounds the sharp edge; Raster the picture's alpha.
                let alpha: Vec<f32> = match &edge {
                    Some(e) => e.clone(),
                    None => (0..n).map(|i| rgba[i * 4 + 3]).collect(),
                };
                let round = rounded(&alpha, w, h, self.radius);
                match edge.as_mut() {
                    Some(e) => *e = round,
                    None => {
                        for (px, (&r, &a)) in rgba.chunks_exact_mut(4).zip(round.iter().zip(&alpha))
                        {
                            let k = if a > 1e-5 { r / a } else { 0.0 };
                            for v in px.iter_mut() {
                                *v *= k;
                            }
                        }
                    }
                }
            }
            _ => {
                let Some(other) = reads.other.filter(|o| o.len() >= n * 4) else {
                    // No layer, no cut: a Vector deformer still sharpens.
                    finish(rgba, edge.as_deref(), before.as_deref(), self.mix);
                    return;
                };
                let b_alpha: Vec<f32> = match reads
                    .boolean
                    .filter(|_| vector)
                    .and_then(|shape| self.boolean_cover(shape, w, h, k, grow))
                {
                    Some(c) => c,
                    None => (0..n).map(|i| other[i * 4 + 3]).collect(),
                };
                for i in 0..n {
                    let a = rgba[i * 4 + 3];
                    let b = other[i * 4 + 3];
                    for ch in 0..4 {
                        let (x, y) = (rgba[i * 4 + ch], other[i * 4 + ch]);
                        rgba[i * 4 + ch] = match self.operation {
                            0 => x + y * (1.0 - a),
                            1 => x * (1.0 - b),
                            2 => x * b,
                            3 => x * (1.0 - b) + y * (1.0 - a),
                            _ => y * (1.0 - a),
                        };
                    }
                    if let Some(e) = edge.as_mut() {
                        let (c, o) = (e[i], b_alpha[i]);
                        e[i] = match self.operation {
                            0 => c + o * (1.0 - c),
                            1 => c * (1.0 - o),
                            2 => c * o,
                            3 => c + o - 2.0 * c * o,
                            _ => o * (1.0 - c),
                        };
                    }
                }
            }
        }
        // Worked out on the paths: the edge is the result's own.
        if vector && reads.result.is_some() {
            edge = Some(outline_cover(reads, w, h, k, grow));
        }
        finish(rgba, edge.as_deref(), before.as_deref(), self.mix);
    }

    /// What this deformer does to `input`, outlines in px@comp on the layer,
    /// worked out on the paths: Align moves them, Boolean cuts them with the
    /// layer's outline (`frame` is the layer's size, for Map placement), and
    /// Round corners rounds every corner. `None` where the paths cannot say,
    /// a Boolean whose layer has no outline.
    #[must_use]
    pub fn on_paths(
        self,
        input: &Outline,
        boolean: Option<&BooleanShape>,
        frame: [f32; 2],
    ) -> Option<Outline> {
        match self.deformer {
            DEFORM_ALIGN => {
                let Some(b) = outline_bounds(input, 1.0) else {
                    return Some(input.clone());
                };
                let d = self.align_shift(b, [0.0; 2]);
                Some(
                    input
                        .iter()
                        .map(|c| c.iter().map(|p| [p[0] + d[0], p[1] + d[1]]).collect())
                        .collect(),
                )
            }
            DEFORM_ROUND => Some(input.iter().map(|c| filleted(c, self.radius)).collect()),
            _ => {
                let Some(shape) = boolean else {
                    // No layer, no cut.
                    return Some(input.clone());
                };
                let clip = placed(shape, self.boolean_layer_placement, frame);
                if clip.is_empty() {
                    return None;
                }
                Some(overlay(input, &clip, self.operation))
            }
        }
    }
}

/// The result's outline as coverage at raster factor `k`.
fn outline_cover(reads: &Reads<'_>, w: u32, h: u32, k: f32, grow: [f32; 2]) -> Vec<f32> {
    let mut c = vec![0.0; (w * h) as usize];
    if let Some(result) = reads.result {
        cover(&mut c, w, h, result, k, grow, 1.0);
    }
    c
}

/// The Boolean layer's outline where Map placement puts it on a layer
/// `frame` px@comp in size.
fn placed(shape: &BooleanShape, placement: u32, frame: [f32; 2]) -> Outline {
    let (contours, scale, offset) = match placement {
        0 => (&shape.placed, [1.0; 2], [0.0; 2]),
        1 => (
            &shape.own,
            [0, 1].map(|i| frame[i] / shape.size[i].max(1e-6)),
            [0.0; 2],
        ),
        _ => (
            &shape.own,
            [1.0; 2],
            [0, 1].map(|i| (frame[i] - shape.size[i]) * 0.5),
        ),
    };
    contours
        .iter()
        .map(|c| {
            c.iter()
                .map(|p| [p[0] * scale[0] + offset[0], p[1] * scale[1] + offset[1]])
                .collect()
        })
        .collect()
}

/// `a` cut with `b` by Operation, each filled by winding: Union, Subtract,
/// Intersect, Difference, Subtract from layer.
#[must_use]
pub fn overlay(a: &Outline, b: &Outline, operation: u32) -> Outline {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    let rule = match operation {
        0 => OverlayRule::Union,
        1 => OverlayRule::Difference,
        2 => OverlayRule::Intersect,
        3 => OverlayRule::Xor,
        _ => OverlayRule::InverseDifference,
    };
    let wide = |o: &Outline| -> Vec<Vec<[f64; 2]>> {
        o.iter()
            .filter(|c| c.len() >= 3)
            .map(|c| {
                c.iter()
                    .map(|p| [f64::from(p[0]), f64::from(p[1])])
                    .collect()
            })
            .collect()
    };
    let (a, b) = (wide(a), wide(b));
    a.overlay(&b, rule, FillRule::NonZero)
        .into_iter()
        .flatten()
        .map(|c| c.into_iter().map(|p| [p[0] as f32, p[1] as f32]).collect())
        .collect()
}

/// A closed contour with every corner rounded to `radius`, as a fillet:
/// the arc meets both sides, and stops half way along the shorter one, so a
/// thin piece is rounded by what it has room for and no piece goes.
#[must_use]
pub fn filleted(contour: &[[f32; 2]], radius: f32) -> Vec<[f32; 2]> {
    let n = contour.len();
    if n < 3 || radius <= 0.0 {
        return contour.to_vec();
    }
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let p = contour[i];
        let (a, b) = (contour[(i + n - 1) % n], contour[(i + 1) % n]);
        let (la, lb) = (
            (a[0] - p[0]).hypot(a[1] - p[1]),
            (b[0] - p[0]).hypot(b[1] - p[1]),
        );
        if la < 1e-6 || lb < 1e-6 {
            continue;
        }
        let (ua, ub) = (
            [(a[0] - p[0]) / la, (a[1] - p[1]) / la],
            [(b[0] - p[0]) / lb, (b[1] - p[1]) / lb],
        );
        // Half the angle between the two sides.
        let half = (ua[0] * ub[0] + ua[1] * ub[1]).clamp(-1.0, 1.0).acos() * 0.5;
        if !(1e-4..=std::f32::consts::FRAC_PI_2 - 1e-3).contains(&half) {
            out.push(p);
            continue;
        }
        let reach = (radius / half.tan()).min(la.min(lb) * 0.5);
        let r = reach * half.tan();
        let bis = [ua[0] + ub[0], ua[1] + ub[1]];
        let bl = bis[0].hypot(bis[1]);
        let c = [
            p[0] + bis[0] / bl * r / half.sin(),
            p[1] + bis[1] / bl * r / half.sin(),
        ];
        let (s, e) = (
            [p[0] + ua[0] * reach, p[1] + ua[1] * reach],
            [p[0] + ub[0] * reach, p[1] + ub[1] * reach],
        );
        let a0 = (s[1] - c[1]).atan2(s[0] - c[0]);
        let mut sweep = (e[1] - c[1]).atan2(e[0] - c[0]) - a0;
        if sweep > std::f32::consts::PI {
            sweep -= std::f32::consts::TAU;
        } else if sweep < -std::f32::consts::PI {
            sweep += std::f32::consts::TAU;
        }
        // About a segment every 3 px of arc, at least one a 12th of a turn.
        let steps = ((sweep.abs() * r / 3.0).ceil() as usize)
            .max((sweep.abs() / 0.52).ceil() as usize)
            .clamp(1, 64);
        for k in 0..=steps {
            let t = a0 + sweep * k as f32 / steps as f32;
            out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
        }
    }
    out
}

/// The picture's colour with Vector's edge as its alpha, then the Mix.
fn finish(rgba: &mut [f32], edge: Option<&[f32]>, before: Option<&[f32]>, mix: f32) {
    if let Some(edge) = edge {
        for (px, &c) in rgba.chunks_exact_mut(4).zip(edge) {
            let a = px[3];
            let k = if a > 1e-5 { c.clamp(0.0, 1.0) / a } else { 0.0 };
            for v in px.iter_mut().take(3) {
                *v *= k;
            }
            px[3] = if a > 1e-5 { c.clamp(0.0, 1.0) } else { 0.0 };
        }
    }
    if let Some(before) = before {
        let m = (mix / 100.0).clamp(0.0, 1.0);
        for (v, b) in rgba.iter_mut().zip(before) {
            *v = b + (*v - b) * m;
        }
    }
}

/// Clone deformer's behaviour. The picture and the outlines it reads ride
/// beside the op, so the CPU reference is [`CloneDeformer::apply`], called
/// directly by the tests.
pub struct CloneDeformerDef;

impl EffectDef for CloneDeformerDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CloneDeformer as EffectMetadata>::SCHEMA
    }

    fn reads_clone_shapes(&self) -> bool {
        true
    }

    /// The raster factor, so px@comp outlines reach the pixels this frame is
    /// drawn at.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(CloneDeformer::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}

impl CloneDeformer {
    /// The raster factor the bag's px rows were scaled by.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// [`DERIVED_PX_SCALE`](Self::DERIVED_PX_SCALE) out of a resolved bag, 1
    /// where nobody resolved it.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0).max(1e-6)
    }
}
