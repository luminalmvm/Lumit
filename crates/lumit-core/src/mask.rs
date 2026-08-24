//! Masks: bezier paths that gate a layer's alpha (docs/03-DATA-MODEL.md §7),
//! plus the scanline rasteriser that turns a path into pixel coverage.
//!
//! In plain terms: a mask is a drawn shape; inside the shape the layer shows,
//! outside it doesn't (or the reverse when inverted). The rasteriser walks
//! the shape row by row, finding where each row enters and leaves the shape —
//! with fractional edges and two vertical subsamples so boundaries render
//! smooth, not stair-stepped. Several masks combine top to bottom by their
//! mode (add, subtract, intersect, difference), and each can be softened
//! (feather) or grown/shrunk (expansion) before it joins the stack. The path
//! can be **keyframed** (see [`PathKeyframe`]), and the feather can be given a
//! width **per vertex** rather than one width all the way round (K-445).

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    anim::{Animation, CubicSpan, Keyframe, Property, SideInterp},
    time::Rational,
};

/// One path vertex with cubic tangent handles (layer-pixel coordinates;
/// tangents relative to the vertex).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub pos: (f64, f64),
    pub tan_in: (f64, f64),
    pub tan_out: (f64, f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BezierPath {
    pub vertices: Vec<Vertex>,
    pub closed: bool,
}

/// One keyed shape of a mask path, at a time in the owner's timebase.
///
/// In plain terms: a whole drawn shape pinned to a moment, the way a scalar
/// keyframe pins a number. A path has no single number to plot, so there is no
/// value graph for it and the timeline shows these as diamonds only — but the
/// *timing* is the ordinary keyframe timing, so the same holds and eases work.
/// [`SideInterp`] here shapes how fast the shape crosses from this key to the
/// next (the interpolation parameter, 0 at this key and 1 at the next), not a
/// value curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathKeyframe {
    pub time: Rational,
    pub path: BezierPath,
    /// Approaching this key.
    pub interp_in: SideInterp,
    /// Leaving this key.
    pub interp_out: SideInterp,
}

/// How a mask joins the stack above it (docs/06-RENDER-PIPELINE.md §2, step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MaskMode {
    /// Geometry only: the path is drawn and editable but gates nothing.
    None,
    #[default]
    Add,
    Subtract,
    Intersect,
    /// The greater of this mask and what the stack holds (K-445).
    Lighten,
    /// The lesser of the two (K-445).
    Darken,
    Difference,
}

impl MaskMode {
    fn is_add(&self) -> bool {
        matches!(self, MaskMode::Add)
    }

    /// Whether a lone mask in this mode should build up from an empty frame
    /// rather than cut into a full one. Add and Lighten both take the mask's
    /// own shape from nothing; every other mode needs a picture to work on.
    fn starts_empty(self) -> bool {
        matches!(self, MaskMode::Add | MaskMode::Lighten)
    }
}

/// True when this property is a plain, still zero — the default for feather and
/// expansion, and so the thing that is left out of the file entirely.
fn is_static_zero(p: &Property) -> bool {
    matches!(p.animation, Animation::Static(v) if v == 0.0) && p.extra.is_empty()
}

/// A mask's animatable number, written as a **bare number while it is still**.
///
/// In plain terms: a mask's opacity used to be just `50.0` in the file, and now
/// it can hold keyframes. Anything that can hold keyframes is a [`Property`],
/// and a `Property` normally writes itself as an object. If these three fields
/// started doing that, every `.lum` ever saved would have to be migrated, and —
/// worse — every frame every project has banked would be retired, because the
/// frame cache names a frame partly by the bytes its masks serialise to
/// (K-338, K-339 made a point of not doing that).
///
/// So the encoding stays what it was for the case that is almost always true.
/// A still value writes as the number; only a mask somebody has actually keyed
/// grows the object. Reading accepts either, so a project written by any build
/// opens here, and one written by this build opens in an older one as long as
/// nobody keyed the mask.
mod still_or_keyed {
    use super::{Animation, Property};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(p: &Property, s: S) -> Result<S::Ok, S::Error> {
        match (&p.animation, p.extra.is_empty()) {
            (Animation::Static(v), true) => s.serialize_f64(*v),
            _ => p.serialize(s),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Property, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Still(f64),
            Keyed(Property),
        }
        Ok(match Either::deserialize(d)? {
            Either::Still(v) => Property::fixed(v),
            Either::Keyed(p) => p,
        })
    }
}

/// The same bare-number-while-still encoding as [`still_or_keyed`], for the
/// per-vertex feather list (K-445). A list of plain numbers is what a mask
/// nobody has keyed writes, so a `.lum` stays readable by eye.
mod still_or_keyed_vec {
    use super::{Animation, Property};
    use serde::{ser::SerializeSeq, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[Property], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for p in v {
            match (&p.animation, p.extra.is_empty()) {
                (Animation::Static(x), true) => seq.serialize_element(x)?,
                _ => seq.serialize_element(p)?,
            }
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Property>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Still(f64),
            Keyed(Property),
        }
        Ok(Vec::<Either>::deserialize(d)?
            .into_iter()
            .map(|e| match e {
                Either::Still(v) => Property::fixed(v),
                Either::Keyed(p) => p,
            })
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub id: Uuid,
    pub name: String,
    /// The shape when the path is not animated, and the shape the editing
    /// tools draw into. Ignored while `path_keys` holds any key.
    pub path: BezierPath,
    /// The keyed shapes, sorted by time, unique times (enforced by the editing
    /// ops). Empty — the ordinary case — is absent from the file, so an
    /// unanimated mask writes exactly the bytes it always did and the frame
    /// cache keeps everything it has banked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_keys: Vec<PathKeyframe>,
    pub inverted: bool,
    /// 0..100, and animatable like any transform property (K-340). Written as
    /// a bare number while it is still — see [`still_or_keyed`].
    #[serde(with = "still_or_keyed")]
    pub opacity: Property,
    /// How this mask combines with the ones above it. Absent in projects
    /// written before modes existed, which loaded and rendered as Add.
    #[serde(default, skip_serializing_if = "MaskMode::is_add")]
    pub mode: MaskMode,
    /// Total width of the soft edge, in layer pixels, half either side of the
    /// path (0 = the hard, antialiased edge). Animatable (K-340).
    #[serde(
        default = "Property::zero",
        with = "still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub feather: Property,
    /// A width of its own for each **vertex** of the path, in layer pixels,
    /// running linearly along each segment between them (K-445). Empty — the
    /// ordinary mask — means [`Self::feather`] all the way round, and is
    /// absent from the file, so a mask nobody has varied writes exactly the
    /// bytes it always did. An entry short of the path's vertex count falls
    /// back to [`Self::feather`] for the vertices it does not reach.
    #[serde(
        default,
        with = "still_or_keyed_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub vertex_feather: Vec<Property>,
    /// Grow (+) or shrink (−) the shape, in layer pixels. Animatable (K-340).
    #[serde(
        default = "Property::zero",
        with = "still_or_keyed",
        skip_serializing_if = "is_static_zero"
    )]
    pub expansion: Property,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Mask {
    /// Whether this mask has any say in what the layer looks like at `t`.
    ///
    /// Two switches turn a mask off, and both mean the same thing to the
    /// person using them: mode `None`, and opacity zero. Neither is "combine
    /// nothing into the stack" — a mask that is off is a mask that is not
    /// there, so a layer carrying only switched-off masks is a layer with no
    /// masks at all, whole and visible. Exactly zero, not merely rounding to
    /// zero: 0.1 % is a mask the author can still see the edge of.
    ///
    /// Time matters because opacity animates: a mask keyed from 0 % can be off
    /// for the first half of a shot and on for the second.
    #[must_use]
    pub fn does_something_at(&self, t: f64) -> bool {
        self.mode != MaskMode::None && self.opacity.value_at(t) > 0.0
    }

    /// The feather this mask asks for at `t`, in layer pixels: one width, and
    /// the per-vertex widths **only when they actually differ** (K-445).
    ///
    /// `n` is the vertex count of the path being drawn, which is not always
    /// [`Self::path`]'s: an animated path whose keys hold different point
    /// counts is reconciled upward by [`resample`] before it is rasterised, and
    /// the widths are read against the reconciled vertices. Vertices the list
    /// does not reach take the uniform width.
    ///
    /// Answering "they are all the same" is what keeps the ordinary mask on the
    /// cheap path: a varying feather costs a second distance transform, and a
    /// mask whose widths happen to be equal should not pay for one.
    fn feather_widths_at(&self, n: usize, t: f64) -> (f64, Option<Vec<f64>>) {
        let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
        let uniform = finite(self.feather.value_at(t)).max(0.0);
        if self.vertex_feather.is_empty() || n == 0 {
            return (uniform, None);
        }
        let widths: Vec<f64> = (0..n)
            .map(|i| {
                self.vertex_feather
                    .get(i)
                    .map_or(uniform, |p| finite(p.value_at(t)).max(0.0))
            })
            .collect();
        let Some(&first) = widths.first() else {
            return (uniform, None);
        };
        if widths.iter().all(|w| *w == first) {
            (first, None)
        } else {
            (uniform, Some(widths))
        }
    }

    /// A fresh, default-switched mask around `path`: Add mode, full opacity,
    /// hard-edged, unanimated.
    fn from_path(name: &str, path: BezierPath) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.into(),
            path,
            path_keys: Vec::new(),
            inverted: false,
            opacity: Property::fixed(100.0),
            mode: MaskMode::Add,
            feather: Property::zero(),
            vertex_feather: Vec::new(),
            expansion: Property::zero(),
            extra: serde_json::Map::new(),
        }
    }

    pub fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Self {
        let corner = |px: f64, py: f64| Vertex {
            pos: (px, py),
            tan_in: (0.0, 0.0),
            tan_out: (0.0, 0.0),
        };
        Self::from_path(
            "Rectangle",
            BezierPath {
                vertices: vec![
                    corner(x, y),
                    corner(x + w, y),
                    corner(x + w, y + h),
                    corner(x, y + h),
                ],
                closed: true,
            },
        )
    }

    /// An `n`-point star with straight edges (corner vertices only), outer
    /// radius `outer`, inner radius `inner`. Points start at the top.
    pub fn star(cx: f64, cy: f64, outer: f64, inner: f64, n: usize) -> Self {
        let n = n.max(3);
        let mut vertices = Vec::with_capacity(n * 2);
        for i in 0..n * 2 {
            let r = if i % 2 == 0 { outer } else { inner };
            // -PI/2 puts the first outer point at the top.
            let a = std::f64::consts::PI * f64::from(i as u32) / f64::from(n as u32)
                - std::f64::consts::FRAC_PI_2;
            vertices.push(Vertex {
                pos: (cx + r * a.cos(), cy + r * a.sin()),
                tan_in: (0.0, 0.0),
                tan_out: (0.0, 0.0),
            });
        }
        Self::from_path(
            "Star",
            BezierPath {
                vertices,
                closed: true,
            },
        )
    }

    /// Ellipse via the standard 4-vertex cubic approximation (kappa).
    pub fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> Self {
        const K: f64 = 0.552_284_749_830_793_4;
        let v = |px: f64, py: f64, tin: (f64, f64), tout: (f64, f64)| Vertex {
            pos: (px, py),
            tan_in: tin,
            tan_out: tout,
        };
        Self::from_path(
            "Ellipse",
            BezierPath {
                vertices: vec![
                    v((cx, cy - ry).0, cy - ry, (-rx * K, 0.0), (rx * K, 0.0)),
                    v(cx + rx, cy, (0.0, -ry * K), (0.0, ry * K)),
                    v(cx, cy + ry, (rx * K, 0.0), (-rx * K, 0.0)),
                    v(cx - rx, cy, (0.0, ry * K), (0.0, -ry * K)),
                ],
                closed: true,
            },
        )
    }

    /// The shape this mask has at time `t` (seconds, the owner's timebase —
    /// layer time for a layer's masks, exactly as every other property on the
    /// layer is read).
    ///
    /// Unanimated is the common case and costs nothing: the stored [`Self::path`]
    /// is handed back by reference. With one key, that key's shape holds for
    /// all time; with several, the two keys either side are blended by
    /// [`lerp_paths`] at the eased parameter.
    pub fn path_at(&self, t: f64) -> Cow<'_, BezierPath> {
        let (Some(first), Some(last)) = (self.path_keys.first(), self.path_keys.last()) else {
            return Cow::Borrowed(&self.path);
        };
        if t <= first.time.to_f64() {
            return Cow::Borrowed(&first.path);
        }
        if t >= last.time.to_f64() {
            return Cow::Borrowed(&last.path);
        }
        let idx = self
            .path_keys
            .windows(2)
            .position(|w| match w.get(1) {
                Some(next) => t < next.time.to_f64(),
                None => false,
            })
            .unwrap_or(0);
        let (Some(a), Some(b)) = (self.path_keys.get(idx), self.path_keys.get(idx + 1)) else {
            return Cow::Borrowed(&self.path);
        };
        // The eases are the scalar evaluator's, run on a 0→1 ramp: one keyframe
        // machine, not two. Hold, linear and AE speed/influence all arrive here
        // already correct, and the parameter is exactly 0 and 1 at the keys.
        let ramp = [
            Keyframe {
                time: a.time,
                value: 0.0,
                interp_in: a.interp_in,
                interp_out: a.interp_out,
            },
            Keyframe {
                time: b.time,
                value: 1.0,
                interp_in: b.interp_in,
                interp_out: b.interp_out,
            },
        ];
        let u = crate::anim::evaluate(&ramp, t).unwrap_or(0.0);
        Cow::Owned(lerp_paths(&a.path, &b.path, u))
    }

    /// Whether this mask's shape is keyframed.
    pub fn path_is_animated(&self) -> bool {
        !self.path_keys.is_empty()
    }
}

/// Blend two paths, `u` = 0 giving `from` and 1 giving `to`.
///
/// Vertex counts are reconciled first ([`resample`]): the sparser path is cut
/// into as many pieces as the denser one has, without moving the curve, and
/// then the two run vertex for vertex — position and both tangent handles are
/// straight-line blended. That is what After Effects does, and it is why adding
/// a point to a shape halfway through an animation does not throw the
/// animation away.
///
/// **Open against closed.** Whether a path is closed is not a quantity, so it
/// cannot be halfway blended: it is *held* across the span, taking `from`'s
/// flag until the next key's time, where that key's own flag takes over. The
/// geometry still interpolates normally; only the closing segment appears or
/// disappears, and it does so on a frame boundary rather than smearing.
pub fn lerp_paths(from: &BezierPath, to: &BezierPath, u: f64) -> BezierPath {
    let n = from.vertices.len().max(to.vertices.len());
    let a = resample(from, n);
    let b = resample(to, n);
    let mix = |p: (f64, f64), q: (f64, f64)| (p.0 + (q.0 - p.0) * u, p.1 + (q.1 - p.1) * u);
    BezierPath {
        vertices: a
            .vertices
            .iter()
            .zip(&b.vertices)
            .map(|(p, q)| Vertex {
                pos: mix(p.pos, q.pos),
                tan_in: mix(p.tan_in, q.tan_in),
                tan_out: mix(p.tan_out, q.tan_out),
            })
            .collect(),
        closed: from.closed,
    }
}

/// The same path drawn with `target` vertices instead of its own.
///
/// The added vertices are placed by **splitting** the existing curve segments,
/// not by dropping points near them: de Casteljau at a parameter gives two
/// cubics whose union *is* the original cubic (the same exactness
/// [`crate::anim::Property::insert_key_preserving_shape`] relies on), so the
/// path that comes back is geometrically the path that went in — nothing
/// bulges, nothing flattens. Which segments get the extra points is fixed and
/// arithmetic (spread as evenly as the count allows, earliest segments first),
/// so the same two paths always reconcile the same way and playback is
/// deterministic.
///
/// A path already at or above `target`, or with no segments at all, is returned
/// unchanged.
pub fn resample(path: &BezierPath, target: usize) -> BezierPath {
    let n = path.vertices.len();
    let segs = if path.closed { n } else { n.saturating_sub(1) };
    if target <= n || segs == 0 {
        return path.clone();
    }
    let extra = target - n;

    // Every segment of the resampled path, as raw control points.
    let mut pieces: Vec<([f64; 4], [f64; 4])> = Vec::with_capacity(segs + extra);
    for i in 0..segs {
        let (Some(a), Some(b)) = (path.vertices.get(i), path.vertices.get((i + 1) % n.max(1)))
        else {
            continue;
        };
        let mut cur = (
            [
                a.pos.0,
                a.pos.0 + a.tan_out.0,
                b.pos.0 + b.tan_in.0,
                b.pos.0,
            ],
            [
                a.pos.1,
                a.pos.1 + a.tan_out.1,
                b.pos.1 + b.tan_in.1,
                b.pos.1,
            ],
        );
        // extra / segs each, and the remainder to the earliest segments.
        let k = extra / segs + usize::from(i < extra % segs);
        let mut done = 0.0f64;
        for j in 1..=k {
            let global = j as f64 / (k + 1) as f64;
            // The tail is a fresh cubic reparametrised to 0..1, so the next cut
            // has to be expressed in *its* parameter, not the original's.
            let u = (global - done) / (1.0 - done);
            let (left, right) = CubicSpan::from_points(cur.0, cur.1).split_at(u);
            pieces.push(left.control_points());
            cur = right.control_points();
            done = global;
        }
        pieces.push(cur);
    }

    // Each piece contributes the vertex it starts from: its own first handle is
    // that vertex's out-tangent, and the previous piece's last handle is its
    // in-tangent (wrapping on a closed path). An open path keeps its two ends'
    // outer handles, which no segment describes.
    let mut vertices = Vec::with_capacity(pieces.len() + 1);
    for (i, (x, y)) in pieces.iter().enumerate() {
        let prev = match i {
            0 if path.closed => pieces.last(),
            0 => None,
            _ => pieces.get(i - 1),
        };
        let tan_in = match prev {
            Some((px, py)) => (px[2] - px[3], py[2] - py[3]),
            None => path.vertices.first().map_or((0.0, 0.0), |v| v.tan_in),
        };
        vertices.push(Vertex {
            pos: (x[0], y[0]),
            tan_in,
            tan_out: (x[1] - x[0], y[1] - y[0]),
        });
    }
    if !path.closed {
        if let Some((x, y)) = pieces.last() {
            vertices.push(Vertex {
                pos: (x[3], y[3]),
                tan_in: (x[2] - x[3], y[2] - y[3]),
                tan_out: path.vertices.last().map_or((0.0, 0.0), |v| v.tan_out),
            });
        }
    }
    BezierPath {
        vertices,
        closed: path.closed,
    }
}

/// Rasterise a closed path to 0..255 coverage at `w`×`h`, with the path's
/// layer-pixel coordinates scaled by (`sx`, `sy`) — pass the texture/natural
/// ratio so reduced-resolution decodes mask correctly. Even-odd fill,
/// fractional-span horizontal AA, two vertical subsamples.
pub fn rasterise(path: &BezierPath, w: u32, h: u32, sx: f64, sy: f64) -> Vec<u8> {
    let mut coverage = vec![0u8; (w * h) as usize];
    if path.vertices.len() < 3 || !path.closed {
        return coverage;
    }

    // Flatten cubics to polyline edges (fixed subdivision — paths are UI-drawn
    // and small; adaptive flattening arrives with the pen tool if needed).
    const SEGS: usize = 24;
    let n = path.vertices.len();
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(n * SEGS);
    for i in 0..n {
        let a = &path.vertices[i];
        let b = &path.vertices[(i + 1) % n];
        let p0 = (a.pos.0 * sx, a.pos.1 * sy);
        let p1 = ((a.pos.0 + a.tan_out.0) * sx, (a.pos.1 + a.tan_out.1) * sy);
        let p2 = ((b.pos.0 + b.tan_in.0) * sx, (b.pos.1 + b.tan_in.1) * sy);
        let p3 = (b.pos.0 * sx, b.pos.1 * sy);
        for s in 0..SEGS {
            let t = s as f64 / SEGS as f64;
            let u = 1.0 - t;
            let x = u * u * u * p0.0
                + 3.0 * u * u * t * p1.0
                + 3.0 * u * t * t * p2.0
                + t * t * t * p3.0;
            let y = u * u * u * p0.1
                + 3.0 * u * u * t * p1.1
                + 3.0 * u * t * t * p2.1
                + t * t * t * p3.1;
            points.push((x, y));
        }
    }

    // Scanline with two vertical subsamples per row.
    let mut xs: Vec<f64> = Vec::with_capacity(16);
    for row in 0..h {
        let mut row_cov = vec![0.0f32; w as usize];
        for sub in 0..2 {
            let y = f64::from(row) + 0.25 + 0.5 * f64::from(sub);
            xs.clear();
            for e in 0..points.len() {
                let (x0, y0) = points[e];
                let (x1, y1) = points[(e + 1) % points.len()];
                if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
                    xs.push(x0 + (y - y0) / (y1 - y0) * (x1 - x0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in xs.chunks_exact(2) {
                let (start, end) = (pair[0].max(0.0), pair[1].min(f64::from(w)));
                if end <= start {
                    continue;
                }
                let (first, last) = (
                    start.floor() as usize,
                    (end.ceil() as usize).min(w as usize),
                );
                for (px, cell) in row_cov.iter_mut().enumerate().take(last).skip(first) {
                    let l = start.max(px as f64);
                    let r = end.min(px as f64 + 1.0);
                    if r > l {
                        *cell += ((r - l) * 0.5) as f32;
                    }
                }
            }
        }
        let base = (row * w) as usize;
        for (px, c) in row_cov.iter().enumerate() {
            coverage[base + px] = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    coverage
}

/// One mask's own 0..255 coverage: rasterised, then expanded and feathered.
///
/// Feather and expansion are two readings of one signed distance field rather
/// than a blur and a separate grow/shrink pass: build the distance from the
/// path once, and expansion moves where the edge sits while feather sets how
/// wide the ramp across it is. Both are layer pixels, so they are multiplied
/// by the preview scale — a feather keeps its real width when the Viewer
/// drops to half resolution. `sx` and `sy` are averaged: a mask softens the
/// same amount in both directions, and the two differ only when a decode is
/// non-square.
fn mask_coverage(mask: &Mask, w: u32, h: u32, sx: f64, sy: f64, t: f64) -> Vec<u8> {
    let path = mask.path_at(t);
    let cov = rasterise(&path, w, h, sx, sy);
    let scale = (sx + sy) * 0.5;
    let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
    let (uniform, varying) = mask.feather_widths_at(path.vertices.len(), t);
    let feather = (uniform * scale) as f32;
    let expansion = (finite(mask.expansion.value_at(t)) * scale) as f32;
    // Fast path: the overwhelmingly common mask is hard-edged and unexpanded,
    // and must come back byte for byte as the rasteriser drew it.
    if feather == 0.0 && expansion == 0.0 && varying.is_none() {
        return cov;
    }

    let dist = signed_distance(&cov, w as usize, h as usize);
    // A zero feather still gets a one-pixel ramp: that is exactly the
    // antialiased edge the rasteriser would have drawn (coverage 0.5 at the
    // edge, falling linearly over the pixel it crosses), so expanding alone
    // slides a smooth edge rather than stamping a jagged one.
    let ramp = feather.max(1.0);
    let widths = varying.map(|at| feather_map(&path, &at, w, h, sx, sy, scale));
    let mut out = cov;
    for (i, (o, d)) in out.iter_mut().zip(dist).enumerate() {
        // A pixel the boundary walk never reached - a path entirely off the
        // raster - has no nearest width, and takes the uniform one.
        let ramp = match widths.as_ref().and_then(|m| m.get(i)) {
            Some(v) if v.is_finite() => v.max(1.0),
            _ => ramp,
        };
        *o = (((0.5 + (d + expansion) / ramp).clamp(0.0, 1.0)) * 255.0).round() as u8;
    }
    out
}

/// The feather width, in **raster** pixels, at every pixel of a mask whose
/// vertices carry widths of their own (K-445).
///
/// In plain terms: the soft edge can be wide in one place and narrow in
/// another, so "how wide is the ramp here" stops being one number and becomes
/// a picture. This builds that picture.
///
/// Widths live at the vertices and run straight-line along each segment
/// between them. The same fixed flattening [`rasterise`] uses walks the
/// boundary and stamps the interpolated width into whichever pixel it lands
/// in; every other pixel then takes the width of the **nearest** stamped one
/// ([`spread_seeds`]). That pairing is the point: the distance field measures
/// each pixel against the nearest piece of edge, so the width it is scaled by
/// should come from that same piece.
fn feather_map(
    path: &BezierPath,
    widths: &[f64],
    w: u32,
    h: u32,
    sx: f64,
    sy: f64,
    scale: f64,
) -> Vec<f32> {
    const SEGS: usize = 24;
    let mut seed = vec![f32::NAN; (w * h) as usize];
    let n = path.vertices.len();
    for i in 0..n {
        let (Some(a), Some(b)) = (path.vertices.get(i), path.vertices.get((i + 1) % n)) else {
            continue;
        };
        let wa = widths.get(i).copied().unwrap_or(0.0);
        let wb = widths.get((i + 1) % n).copied().unwrap_or(0.0);
        let p0 = (a.pos.0 * sx, a.pos.1 * sy);
        let p1 = ((a.pos.0 + a.tan_out.0) * sx, (a.pos.1 + a.tan_out.1) * sy);
        let p2 = ((b.pos.0 + b.tan_in.0) * sx, (b.pos.1 + b.tan_in.1) * sy);
        let p3 = (b.pos.0 * sx, b.pos.1 * sy);
        // `..=SEGS` so the segment's far end is stamped too: the next segment
        // starts there and stamps it again with the same width, and an open
        // run would otherwise leave its final vertex bare.
        for step in 0..=SEGS {
            let t = step as f64 / SEGS as f64;
            let u = 1.0 - t;
            let x = u * u * u * p0.0
                + 3.0 * u * u * t * p1.0
                + 3.0 * u * t * t * p2.0
                + t * t * t * p3.0;
            let y = u * u * u * p0.1
                + 3.0 * u * u * t * p1.1
                + 3.0 * u * t * t * p2.1
                + t * t * t * p3.1;
            if !(x >= 0.0 && y >= 0.0 && x < f64::from(w) && y < f64::from(h)) {
                continue;
            }
            if let Some(cell) = seed.get_mut(y as usize * w as usize + x as usize) {
                *cell = ((wa + (wb - wa) * t) * scale) as f32;
            }
        }
    }
    spread_seeds(&seed, w as usize, h as usize)
}

/// Give every pixel the value of the nearest seeded one. `NaN` marks a pixel
/// with no seed, on the way in and - when nothing was seeded at all - on the
/// way out.
///
/// The **feature** half of the same Felzenszwalb and Huttenlocher transform
/// [`signed_distance`] uses for the distance: the column pass records which row
/// of each column holds its nearest seed, and the row pass records which column
/// won, so the two together name the seeding pixel and not merely how far away
/// it is.
fn spread_seeds(seed: &[f32], w: usize, h: usize) -> Vec<f32> {
    const FAR: f32 = 1e20;
    if w == 0 || h == 0 || seed.len() < w * h {
        return seed.to_vec();
    }
    let mut f: Vec<f32> = seed
        .iter()
        .map(|v| if v.is_nan() { FAR } else { 0.0 })
        .collect();
    let n = w.max(h);
    let mut line = vec![0.0f32; n];
    let mut out = vec![0.0f32; n];
    let mut arg = vec![0u32; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n + 1];
    let mut src_y = vec![0u32; w * h];
    for x in 0..w {
        for y in 0..h {
            line[y] = f[y * w + x];
        }
        edt_1d(
            &line[..h],
            &mut out[..h],
            &mut arg[..h],
            &mut v[..h],
            &mut z[..=h],
        );
        for y in 0..h {
            f[y * w + x] = out[y];
            src_y[y * w + x] = arg[y];
        }
    }
    let mut vals = vec![f32::NAN; w * h];
    for y in 0..h {
        let row = y * w;
        line[..w].copy_from_slice(&f[row..row + w]);
        edt_1d(
            &line[..w],
            &mut out[..w],
            &mut arg[..w],
            &mut v[..w],
            &mut z[..=w],
        );
        for x in 0..w {
            let sx = (arg[x] as usize).min(w - 1);
            let sy = (src_y[row + sx] as usize).min(h - 1);
            vals[row + x] = seed[sy * w + sx];
        }
    }
    vals
}

/// Signed distance in pixels from each pixel centre to the mask edge —
/// positive inside the shape, negative outside.
///
/// Seeded from the antialiased coverage rather than a hard threshold: where a
/// pixel is partly covered its centre is about `0.5 − coverage` from the edge
/// (a straight edge crossing a pixel covers it in proportion to how far past
/// the centre it is), which puts the seeds at sub-pixel positions and keeps
/// feathered edges as smooth as the raster they came from. Everything else
/// starts unknown and gets its distance from the exact Euclidean transform.
fn signed_distance(cov: &[u8], w: usize, h: usize) -> Vec<f32> {
    // Not `f32::INFINITY`: the parabola intersections below would go NaN
    // subtracting one infinity from another. Far beyond any real image.
    const FAR: f32 = 1e20;
    let mut f: Vec<f32> = cov
        .iter()
        .map(|&c| {
            if c == 0 || c == 255 {
                FAR
            } else {
                let t = f32::from(c) / 255.0 - 0.5;
                t * t
            }
        })
        .collect();
    if w == 0 || h == 0 {
        return f;
    }

    // An edge that lands exactly on a pixel boundary leaves no partly covered
    // pixel to seed from — a rectangle on whole coordinates is the common
    // case, not a corner one. Where two saturated neighbours disagree, the
    // edge runs half a pixel from each of their centres.
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let c = cov[i];
            if c != 0 && c != 255 {
                continue;
            }
            let mut neighbour = |j: usize| {
                let n = cov[j];
                if (n == 0 || n == 255) && n != c {
                    f[i] = f[i].min(0.25);
                    f[j] = f[j].min(0.25);
                }
            };
            if x + 1 < w {
                neighbour(i + 1);
            }
            if y + 1 < h {
                neighbour(i + w);
            }
        }
    }

    // Felzenszwalb & Huttenlocher: the 2D squared Euclidean transform is the
    // 1D transform down every column, then along every row. Scratch buffers
    // are allocated once for the whole image, never per line.
    let n = w.max(h);
    let mut line = vec![0.0f32; n];
    let mut out = vec![0.0f32; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n + 1];
    // Which parabola won is [`spread_seeds`]'s business, not this one's.
    let mut arg = vec![0u32; n];
    for x in 0..w {
        for y in 0..h {
            line[y] = f[y * w + x];
        }
        edt_1d(
            &line[..h],
            &mut out[..h],
            &mut arg[..h],
            &mut v[..h],
            &mut z[..=h],
        );
        for y in 0..h {
            f[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        let row = y * w;
        line[..w].copy_from_slice(&f[row..row + w]);
        edt_1d(
            &line[..w],
            &mut out[..w],
            &mut arg[..w],
            &mut v[..w],
            &mut z[..=w],
        );
        f[row..row + w].copy_from_slice(&out[..w]);
    }

    for (d, &c) in f.iter_mut().zip(cov) {
        let inside = c >= 128;
        *d = d.max(0.0).sqrt() * if inside { 1.0 } else { -1.0 };
    }
    f
}

/// The 1D squared distance transform of a sampled function: `out[q]` is the
/// lowest `f[p] + (q − p)²` over all `p`, and `arg[q]` is the `p` that won it.
/// `v` and `z` are scratch (the indices of the parabolas forming the lower
/// envelope, and where they cross).
///
/// The `arg` half costs one store per sample and is what [`spread_seeds`] needs
/// to carry a *value* out from the nearest seed rather than only a distance;
/// [`signed_distance`] hands it a scratch buffer and ignores it.
fn edt_1d(f: &[f32], out: &mut [f32], arg: &mut [u32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 || out.len() < n || arg.len() < n || v.len() < n || z.len() < n + 1 {
        return;
    }
    let sq = |i: usize| (i as f32) * (i as f32);
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        let mut s;
        loop {
            let p = v[k];
            s = ((f[q] + sq(q)) - (f[p] + sq(p))) / (2.0 * (q as f32 - p as f32));
            if k > 0 && s <= z[k] {
                k -= 1;
            } else {
                break;
            }
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        *o = (q as f32 - p as f32).powi(2) + f[p];
        arg[q] = p as u32;
    }
}

/// Apply a layer's masks to straight-alpha RGBA8 pixels in place.
/// Masks combine top to bottom by mode; invert, feather, expansion and
/// opacity apply to each mask before it joins the stack.
///
/// `t` is the owner's time in seconds — layer time for a layer's masks — and
/// only matters when a path is keyframed.
pub fn apply_masks(
    rgba: &mut [u8],
    w: u32,
    h: u32,
    natural_w: f64,
    natural_h: f64,
    masks: &[Mask],
    t: f64,
) {
    if masks.is_empty() {
        return;
    }
    let total = combined_coverage(masks, w, h, natural_w, natural_h, t);
    for (px, t) in rgba.chunks_exact_mut(4).zip(total) {
        px[3] = ((u16::from(px[3]) * u16::from(t)) / 255) as u8;
    }
}

/// The combined 0..255 coverage of a mask stack at `w`×`h` (path coordinates
/// in `natural` space) — the same maths [`apply_masks`] uses, exposed so
/// GPU-sourced layers (Precomps) can upload it as a texture instead of
/// editing pixels they don't have.
///
/// Masks fold in list order, top to bottom, each one's own coverage
/// (feathered, expanded, inverted, then faded by its opacity) combined into
/// the running total by its mode. Order therefore matters: subtracting B from
/// A is not subtracting A from B.
///
/// **What the first mask combines with.** The fold has to start somewhere, and
/// zero is only the right start for Add — a lone Subtract mask against an
/// empty total would cut a hole in nothing and leave the layer invisible,
/// where every editor (and every user) expects a hole in the picture. So the
/// stack starts empty when the topmost mask that does anything is Add, and
/// full-frame otherwise: Subtract cuts its hole, Intersect first shows just
/// itself, Difference first shows its inverse. That matches After Effects.
pub fn combined_coverage(
    masks: &[Mask],
    w: u32,
    h: u32,
    natural_w: f64,
    natural_h: f64,
    t: f64,
) -> Vec<u8> {
    let sx = f64::from(w) / natural_w.max(1.0);
    let sy = f64::from(h) / natural_h.max(1.0);
    let base: u16 = match masks
        .iter()
        .find(|m| m.does_something_at(t))
        .map(|m| m.mode)
    {
        Some(mode) if mode.starts_empty() => 0,
        // Including `None`: no mask does anything, so nothing is masked and the
        // layer is whole. Starting at zero here would hide a layer because it
        // carries one switched-off mask, which reads as the mask doing the
        // exact opposite of nothing.
        _ => 255,
    };
    let mut total = vec![base; (w * h) as usize];
    for mask in masks {
        if !mask.does_something_at(t) {
            continue;
        }
        let cov = mask_coverage(mask, w, h, sx, sy, t);
        let op = (mask.opacity.value_at(t).clamp(0.0, 100.0) / 100.0 * 255.0) as u16;
        for (t, c) in total.iter_mut().zip(cov) {
            let c = if mask.inverted {
                255 - u16::from(c)
            } else {
                u16::from(c)
            };
            let c = c * op / 255;
            *t = match mask.mode {
                MaskMode::None => *t,
                MaskMode::Add => (*t + c).min(255),
                MaskMode::Subtract => t.saturating_sub(c),
                MaskMode::Intersect => (*t).min(c),
                // Max and min against what the stack holds, which is what
                // After Effects means by these two (K-445).
                MaskMode::Lighten => (*t).max(c),
                MaskMode::Darken => (*t).min(c),
                MaskMode::Difference => t.abs_diff(c),
            };
        }
    }
    total.into_iter().map(|t| t as u8).collect()
}

// ---------------------------------------------------------------------------
// The mask-path carriage (K-408): a mask's geometry, on its way to an effect.
// ---------------------------------------------------------------------------

/// How closely a flattened mask path follows the curve it came from, in
/// **pixels at composition scale** (K-408, docs/08 §2.3's unit).
///
/// A constant on purpose, and this is the whole reason it is one: the polyline
/// is part of what an effect renders, so if the tolerance could vary — with the
/// preview raster, with a setting, with the machine — the same document would
/// name the same frame twice and draw it differently. Half a pixel is under the
/// threshold of a visible kink at 100 % zoom and cheap: a full-frame 1080p
/// ellipse comes to a few hundred points.
pub const MASK_PATH_TOLERANCE_PX: f64 = 0.5;

/// A safety ceiling on the subdivisions of ONE cubic segment, so a path with an
/// absurd tangent (a handle dragged a hundred thousand pixels out) costs a
/// bounded amount rather than an unbounded one. Well above anything the drawing
/// tools produce.
const MAX_SEGMENT_STEPS: usize = 1024;

/// One mask path flattened for an effect: an **arc-length-parameterised
/// polyline** in layer pixels at composition scale (K-408).
///
/// # In plain terms
///
/// A mask is stored as a handful of points with curve handles. An effect that
/// walks the shape — a brush travelling from 20 % to 80 % along it, segments
/// marching round it — cannot use that directly: it needs to know *how far
/// along* it is, and a curve handle says nothing about distance. So the render
/// straightens the curve into many short straight pieces and writes down how
/// far along each corner sits. Then "60 % of the way round" is a lookup.
///
/// [`Self::points`] and [`Self::arc`] are the same length. `arc[0]` is 0 and
/// `arc.last()` is the total length, so a distance `s` in `0..=length` locates a
/// point by searching `arc` — no consumer has to re-measure the curve, and every
/// consumer measures it the same way.
///
/// **Empty is the documented no-op**: a row naming nothing, a mask since
/// deleted, a layer with no masks at all, or a path with fewer than two
/// vertices all arrive here as an empty polyline, and an effect handed one
/// renders its input unchanged (14-ENGINEERING-RULES §4 — degrade, never
/// fault).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaskPolyline {
    /// The vertices, in **layer pixels at composition scale** — deliberately
    /// not the raster the frame happens to be previewing at, so the same
    /// document flattens to the same numbers whatever the preview divisor is.
    /// A consumer scales them by its own raster factor, as it does any px@comp
    /// quantity (docs/08 §2.3).
    pub points: Vec<[f32; 2]>,
    /// Cumulative distance along the polyline at each point. Same length as
    /// [`Self::points`]; starts at 0; ends at the total length.
    pub arc: Vec<f32>,
    /// Whether the path closes back to its first point. A closed path's last
    /// point is the first one repeated, so the final `arc` entry is the full
    /// perimeter and a consumer never has to special-case the join.
    pub closed: bool,
    /// The mask's own soft-edge width at this frame, px@comp — total width,
    /// half either side of the curve, exactly as the mask itself draws it
    /// (K-446). Zero for a hard edge, and for every way of naming nothing.
    ///
    /// It rides here because an effect that *fills* the shape has to feather
    /// its fill the way the mask feathers its coverage; an effect that merely
    /// walks the curve ignores it. One number, not the K-445 per-vertex
    /// widths: a varying width would have to ride per segment, and the only
    /// consumer so far is a garbage matte, where the mask's own uniform width
    /// is what the eye is comparing against.
    pub feather: f32,
    /// The mask's own grow (+) / shrink (−) at this frame, px@comp — where the
    /// edge sits relative to the drawn curve (K-446). Zero for an unexpanded
    /// mask.
    pub expansion: f32,
}

impl MaskPolyline {
    /// Nothing to walk — the effect's no-op.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.len() < 2
    }

    /// The total length in px@comp, 0 for an empty polyline.
    #[must_use]
    pub fn length(&self) -> f32 {
        self.arc.last().copied().unwrap_or(0.0)
    }

    /// The point `s` px along the path, `s` clamped into `0..=length()`
    /// (K-408). The lookup [`Self::arc`] exists for: a consumer asking "where
    /// is 60 % of the way round" gets an answer without re-measuring the curve,
    /// and every consumer gets the *same* answer.
    ///
    /// A binary search rather than a walk, because Stroke asks it once per
    /// brush stamp and a walk would make placing `n` stamps quadratic in the
    /// polyline. `[0.0, 0.0]` for an empty polyline — the no-op's coordinate,
    /// never a panic (docs/14 §4).
    #[must_use]
    pub fn point_at(&self, s: f32) -> [f32; 2] {
        if self.is_empty() {
            return [0.0, 0.0];
        }
        let s = s.clamp(0.0, self.length());
        // The last index whose arc is <= s: `partition_point` counts the
        // entries strictly below, so subtracting one lands on it, and the
        // final point can never be the *start* of an edge.
        let i = self
            .arc
            .partition_point(|&a| a <= s)
            .saturating_sub(1)
            .min(self.points.len() - 2);
        let (a, b) = (self.points[i], self.points[i + 1]);
        let span = self.arc[i + 1] - self.arc[i];
        // A zero-length edge (two coincident vertices) hands back its start
        // rather than dividing by nothing.
        let t = if span > 0.0 {
            ((s - self.arc[i]) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }
}

/// Flatten one bezier path to an arc-length-parameterised polyline within
/// `tolerance_px` (K-408). Deterministic: the subdivision count of each cubic
/// comes from the control points and the tolerance alone, so the same path
/// always flattens to the same bytes.
#[must_use]
pub fn flatten_path(path: &BezierPath, tolerance_px: f64) -> MaskPolyline {
    let n = path.vertices.len();
    if n < 2 {
        return MaskPolyline::default();
    }
    let tol = if tolerance_px.is_finite() && tolerance_px > 0.0 {
        tolerance_px
    } else {
        MASK_PATH_TOLERANCE_PX
    };
    // An open path has n-1 segments; a closed one has n, the last joining back.
    let segments = if path.closed { n } else { n - 1 };
    let mut points: Vec<[f32; 2]> = Vec::with_capacity(segments * 8);
    for i in 0..segments {
        let a = &path.vertices[i];
        let b = &path.vertices[(i + 1) % n];
        let p0 = a.pos;
        let p1 = (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1);
        let p2 = (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1);
        let p3 = b.pos;
        // How many straight pieces this cubic needs. The standard bound: a
        // cubic split into `k` equal pieces sits within |B''|max / (8 k²) of
        // the chords, and |B''|max is 6 × the larger of the two second
        // differences of the control points. Solve for k.
        let d1 = (p0.0 - 2.0 * p1.0 + p2.0, p0.1 - 2.0 * p1.1 + p2.1);
        let d2 = (p1.0 - 2.0 * p2.0 + p3.0, p1.1 - 2.0 * p2.1 + p3.1);
        let m = 6.0 * (d1.0.hypot(d1.1)).max(d2.0.hypot(d2.1));
        let steps = if m.is_finite() && m > 0.0 {
            ((m / (8.0 * tol)).sqrt().ceil() as usize).clamp(1, MAX_SEGMENT_STEPS)
        } else {
            1
        };
        // Each segment emits its start and its interior; the next segment's
        // start is this one's end, so the join is written once.
        for s in 0..steps {
            let t = s as f64 / steps as f64;
            let u = 1.0 - t;
            let x = u * u * u * p0.0
                + 3.0 * u * u * t * p1.0
                + 3.0 * u * t * t * p2.0
                + t * t * t * p3.0;
            let y = u * u * u * p0.1
                + 3.0 * u * u * t * p1.1
                + 3.0 * u * t * t * p2.1
                + t * t * t * p3.1;
            points.push([x as f32, y as f32]);
        }
    }
    // The final point: the last vertex for an open path, the first one again
    // for a closed one — so `arc` covers the closing edge too and a consumer
    // walking to `length()` ends where it started.
    let last = if path.closed {
        path.vertices[0].pos
    } else {
        path.vertices[n - 1].pos
    };
    points.push([last.0 as f32, last.1 as f32]);

    let mut arc = Vec::with_capacity(points.len());
    let mut total = 0.0f32;
    arc.push(0.0);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        total += (b[0] - a[0]).hypot(b[1] - a[1]);
        arc.push(total);
    }
    MaskPolyline {
        points,
        arc,
        closed: path.closed,
        // The curve alone. Whoever knows which *mask* this came from fills the
        // widths in (see `mask_path_at`); flattening a bare path does not.
        feather: 0.0,
        expansion: 0.0,
    }
}

/// Which of `masks` a [`ParamKind::MaskPath`](crate::fx::ParamKind::MaskPath)
/// row names: the one it names, else the first when the schema's `self_default`
/// says an unset row means "First mask" (K-408).
///
/// **One place**, because two answers would be two different pictures: the
/// frame key hashes what this returns and the render flattens what this
/// returns, and a key that disagrees with the picture is a stale frame nobody
/// can explain. A named mask that is no longer on the layer is `None` — the
/// no-op — and deliberately does *not* fall back to the first, because
/// quietly walking a different shape is worse than walking none.
///
/// A mask in [`MaskMode::None`] is offered like any other: that mode is
/// "geometry only, gates nothing", which is precisely the mask somebody draws
/// *for* an effect to walk.
/// Answers with the mask's **position** rather than the mask, because that is
/// what both callers need: the render takes `masks[i]`, and the frame key
/// hashes `i` — never the id, since identity never feeds a key (a duplicated
/// comp shares its original's cache).
#[must_use]
pub fn mask_index_for_path_param(
    masks: &[Mask],
    named: Option<Uuid>,
    self_default: bool,
) -> Option<usize> {
    match named {
        Some(id) => masks.iter().position(|m| m.id == id),
        None if self_default => (!masks.is_empty()).then_some(0),
        None => None,
    }
}

/// The polyline a mask-path row comes to at time `t`, in layer pixels at
/// composition scale — the whole carriage in one call (K-408). Empty for every
/// way of naming nothing.
#[must_use]
pub fn mask_path_at(
    masks: &[Mask],
    named: Option<Uuid>,
    self_default: bool,
    t: f64,
) -> MaskPolyline {
    match mask_index_for_path_param(masks, named, self_default).and_then(|i| masks.get(i)) {
        Some(mask) => {
            let path = mask.path_at(t);
            let mut poly = flatten_path(&path, MASK_PATH_TOLERANCE_PX);
            // The soft edge and the expansion travel with the curve (K-446):
            // an effect filling the shape has to soften and slide its fill
            // exactly where the mask itself would. Read through the same
            // accessors `mask_coverage` reads, so a garbage matte and the
            // mask's own coverage never disagree about how wide the edge is.
            let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
            poly.feather = mask.feather_widths_at(path.vertices.len(), t).0 as f32;
            poly.expansion = finite(mask.expansion.value_at(t)) as f32;
            poly
        }
        None => MaskPolyline::default(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **The same document at the same frame flattens to the same bytes**
    /// (K-408). The whole reason the tolerance is a constant: the polyline is
    /// part of what an effect draws, so a flattening that could vary — with the
    /// preview raster, with a run, with the order the masks happen to be walked
    /// in — would let one frame key name two pictures.
    #[test]
    fn a_mask_path_flattens_deterministically() {
        let mut layer_a = vec![Mask::ellipse(60.0, 40.0, 30.0, 18.0)];
        layer_a[0].name = "Ellipse 1".into();
        // A second layer with the same shape drawn independently: different
        // ids, same geometry. Identity must not reach the vertices.
        let mut layer_b = vec![Mask::ellipse(60.0, 40.0, 30.0, 18.0)];
        layer_b[0].name = "somebody else's name".into();
        assert_ne!(layer_a[0].id, layer_b[0].id, "two masks, two ids");

        let one = mask_path_at(&layer_a, None, true, 0.0);
        let again = mask_path_at(&layer_a, None, true, 0.0);
        let other = mask_path_at(&layer_b, None, true, 0.0);
        assert_eq!(one, again, "two calls, two answers");
        assert_eq!(one, other, "the mask's id reached its vertices");
        assert!(!one.is_empty(), "an ellipse flattens to something");

        // Arc length is monotone, starts at zero, and its last entry is the
        // perimeter — the contract every consumer walks by.
        assert_eq!(one.arc.len(), one.points.len());
        assert_eq!(one.arc.first().copied(), Some(0.0));
        assert!(
            one.arc.windows(2).all(|w| w[1] >= w[0]),
            "arc goes backwards"
        );
        // 2πab-ish: Ramanujan's perimeter for a 30×18 ellipse is ~152.5.
        let (a, b) = (30.0f32, 18.0f32);
        let want = std::f32::consts::PI * (3.0 * (a + b) - ((3.0 * a + b) * (a + 3.0 * b)).sqrt());
        assert!(
            (one.length() - want).abs() < want * 0.01,
            "perimeter {} vs ~{want}",
            one.length()
        );
        // A closed path ends where it started, so a consumer walking to
        // `length()` needs no special case for the join.
        assert!(one.closed);
        let (first, last) = (one.points[0], one.points[one.points.len() - 1]);
        assert!((first[0] - last[0]).abs() < 1e-4 && (first[1] - last[1]).abs() < 1e-4);
    }

    /// The tolerance means what it says: the shipped flattening is already
    /// close enough that flattening 64× finer barely moves the outline.
    ///
    /// Measured as perimeter rather than as a point-to-curve distance, because
    /// every flattened point sits *exactly* on the curve by construction — what
    /// the tolerance actually bounds is how far the straight bits cut the
    /// corners, and that shows up as a shorter total. A chord never overshoots,
    /// so the coarse perimeter must also be the shorter of the two.
    #[test]
    fn a_finer_tolerance_barely_moves_the_outline() {
        let m = Mask::ellipse(0.0, 0.0, 100.0, 60.0);
        let coarse = flatten_path(&m.path, MASK_PATH_TOLERANCE_PX);
        let fine = flatten_path(&m.path, MASK_PATH_TOLERANCE_PX / 64.0);
        assert!(
            fine.points.len() > coarse.points.len(),
            "finer means more points"
        );
        assert!(
            coarse.length() <= fine.length(),
            "a chord overshot its curve: {} vs {}",
            coarse.length(),
            fine.length()
        );
        assert!(
            fine.length() - coarse.length() < fine.length() * 0.005,
            "the coarse perimeter {} strays from the fine one {}",
            coarse.length(),
            fine.length()
        );
    }

    /// Which mask a path row comes to (K-408) — and every way of coming to
    /// none, all of which are the effect's documented no-op rather than a fault.
    #[test]
    fn a_mask_path_row_resolves_or_is_a_no_op() {
        let masks = vec![
            Mask::rectangle(0.0, 0.0, 10.0, 10.0),
            Mask::ellipse(50.0, 50.0, 8.0, 8.0),
        ];
        let (first, second) = (masks[0].id, masks[1].id);

        assert_eq!(
            mask_index_for_path_param(&masks, Some(second), true),
            Some(1)
        );
        assert_eq!(
            mask_index_for_path_param(&masks, Some(first), false),
            Some(0)
        );
        // "First mask": unset means the first one where the schema says so.
        assert_eq!(mask_index_for_path_param(&masks, None, true), Some(0));
        // …and means nothing where it does not.
        assert_eq!(mask_index_for_path_param(&masks, None, false), None);
        // A mask since deleted does NOT fall back to the first: walking a
        // different shape than the one named is worse than walking none.
        assert_eq!(
            mask_index_for_path_param(&masks, Some(Uuid::now_v7()), true),
            None
        );
        // A layer with no masks at all, on the self-default.
        assert_eq!(mask_index_for_path_param(&[], None, true), None);

        // Each of those comes out as an empty polyline, never a panic.
        for named in [Some(Uuid::now_v7()), None] {
            let p = mask_path_at(&masks, named, false, 0.0);
            assert!(p.is_empty() && p.length() == 0.0, "not the no-op");
        }
        assert!(mask_path_at(&[], None, true, 0.0).is_empty());
        // A path of one vertex is a shape being drawn, not a curve to walk.
        let stub = BezierPath {
            vertices: vec![masks[0].path.vertices[0]],
            closed: false,
        };
        assert!(flatten_path(&stub, MASK_PATH_TOLERANCE_PX).is_empty());
    }

    /// An animated mask hands over the shape at the *frame's* time — the same
    /// `path_at` the rasteriser reads, so an effect walking a mask and the mask
    /// gating the layer can never disagree about where the shape is.
    #[test]
    fn a_keyed_mask_path_follows_its_keys() {
        let mut m = Mask::rectangle(0.0, 0.0, 10.0, 10.0);
        let wide = Mask::rectangle(0.0, 0.0, 100.0, 10.0);
        m.path_keys = vec![
            PathKeyframe {
                time: crate::time::Rational::new(0, 1).expect("0"),
                path: m.path.clone(),
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
            PathKeyframe {
                time: crate::time::Rational::new(1, 1).expect("1"),
                path: wide.path.clone(),
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            },
        ];
        let masks = vec![m];
        let at_zero = mask_path_at(&masks, None, true, 0.0);
        let at_one = mask_path_at(&masks, None, true, 1.0);
        assert!(
            at_one.length() > at_zero.length() * 2.0,
            "{} then {} — the key never moved the shape",
            at_zero.length(),
            at_one.length()
        );
    }

    #[test]
    fn rectangle_covers_exactly_its_area() {
        let m = Mask::rectangle(4.0, 4.0, 8.0, 8.0);
        let cov = rasterise(&m.path, 16, 16, 1.0, 1.0);
        assert_eq!(cov[(8 * 16 + 8) as usize], 255, "inside");
        assert_eq!(cov[(2 * 16 + 2) as usize], 0, "outside");
        let sum: f64 = cov.iter().map(|c| f64::from(*c) / 255.0).sum();
        assert!((sum - 64.0).abs() < 1.5, "area {sum} vs 64");
    }

    #[test]
    fn star_has_alternating_radii_and_closes() {
        let m = Mask::star(50.0, 50.0, 40.0, 16.0, 5);
        assert_eq!(m.path.vertices.len(), 10);
        assert!(m.path.closed);
        // Outer points sit ~40 from centre, inner ~16 — alternating.
        for (i, v) in m.path.vertices.iter().enumerate() {
            let r = ((v.pos.0 - 50.0).powi(2) + (v.pos.1 - 50.0).powi(2)).sqrt();
            let want = if i % 2 == 0 { 40.0 } else { 16.0 };
            assert!((r - want).abs() < 1e-9, "vertex {i} radius {r} vs {want}");
        }
        // First outer point is at the top (y < centre).
        assert!(m.path.vertices[0].pos.1 < 50.0);
        // Rasterises to a sensible non-zero, sub-bounding-box area.
        let cov = rasterise(&m.path, 100, 100, 1.0, 1.0);
        let sum: f64 = cov.iter().map(|c| f64::from(*c) / 255.0).sum();
        assert!(sum > 500.0 && sum < 5000.0, "star area {sum}");
    }

    #[test]
    fn ellipse_area_matches_pi_r_squared() {
        let m = Mask::ellipse(32.0, 32.0, 20.0, 20.0);
        let cov = rasterise(&m.path, 64, 64, 1.0, 1.0);
        let sum: f64 = cov.iter().map(|c| f64::from(*c) / 255.0).sum();
        let expect = std::f64::consts::PI * 20.0 * 20.0;
        assert!(
            (sum - expect).abs() / expect < 0.01,
            "area {sum} vs {expect}"
        );
    }

    #[test]
    fn scaled_rasterisation_masks_reduced_decodes_correctly() {
        // Path in natural 100×100 space, rasterised for a 50×50 decode.
        let m = Mask::rectangle(0.0, 0.0, 50.0, 100.0); // left half
        let cov = rasterise(&m.path, 50, 50, 0.5, 0.5);
        assert_eq!(cov[(25 * 50 + 10) as usize], 255, "left in");
        assert_eq!(cov[(25 * 50 + 40) as usize], 0, "right out");
    }

    #[test]
    fn apply_masks_gates_alpha_with_invert_and_opacity() {
        let m = Mask::rectangle(0.0, 0.0, 2.0, 4.0); // left half of 4×4
        let mut rgba = vec![255u8; 4 * 4 * 4];
        apply_masks(&mut rgba, 4, 4, 4.0, 4.0, std::slice::from_ref(&m), 0.0);
        assert_eq!(rgba[4 * 4 + 3], 255, "left opaque");
        assert_eq!(rgba[(4 + 3) * 4 + 3], 0, "right transparent");

        let mut inv = m.clone();
        inv.inverted = true;
        inv.opacity = Property::fixed(50.0);
        let mut rgba = vec![255u8; 4 * 4 * 4];
        apply_masks(&mut rgba, 4, 4, 4.0, 4.0, &[inv], 0.0);
        assert_eq!(rgba[4 * 4 + 3], 0, "inverted left transparent");
        let right = rgba[(4 + 3) * 4 + 3];
        assert!((i16::from(right) - 127).abs() <= 2, "half opacity {right}");
    }

    /// Two 16-wide-tall rectangles: A covers x 0..10, B covers x 6..16, so
    /// x=2 is A only, x=8 is both, x=12 is B only.
    fn overlapping(mode: MaskMode) -> Vec<u8> {
        let a = Mask::rectangle(0.0, 0.0, 10.0, 16.0);
        let mut b = Mask::rectangle(6.0, 0.0, 10.0, 16.0);
        b.mode = mode;
        combined_coverage(&[a, b], 16, 16, 16.0, 16.0, 0.0)
    }

    fn at(cov: &[u8], x: usize) -> u8 {
        cov[8 * 16 + x]
    }

    #[test]
    fn each_mode_combines_as_specified() {
        let add = overlapping(MaskMode::Add);
        assert_eq!((at(&add, 2), at(&add, 8), at(&add, 12)), (255, 255, 255));

        let sub = overlapping(MaskMode::Subtract);
        assert_eq!((at(&sub, 2), at(&sub, 8), at(&sub, 12)), (255, 0, 0));

        let int = overlapping(MaskMode::Intersect);
        assert_eq!((at(&int, 2), at(&int, 8), at(&int, 12)), (0, 255, 0));

        let dif = overlapping(MaskMode::Difference);
        assert_eq!((at(&dif, 2), at(&dif, 8), at(&dif, 12)), (255, 0, 255));
    }

    #[test]
    fn none_mode_contributes_nothing() {
        let none = overlapping(MaskMode::None);
        let alone = combined_coverage(
            std::slice::from_ref(&Mask::rectangle(0.0, 0.0, 10.0, 16.0)),
            16,
            16,
            16.0,
            16.0,
            0.0,
        );
        assert_eq!(none, alone, "a None mask is geometry only");
    }

    /// **A mask that is switched off leaves the layer whole.** Both switches —
    /// mode `None` and opacity zero — used to hide the layer completely when
    /// the mask was the only one on it: the fold started from nothing and then
    /// skipped the very mask it had started from, so the layer ended up masked
    /// by an empty shape. The opposite of doing nothing.
    #[test]
    fn a_switched_off_mask_leaves_the_layer_whole() {
        for off in [
            {
                let mut m = Mask::rectangle(0.0, 0.0, 8.0, 8.0);
                m.mode = MaskMode::None;
                m
            },
            {
                let mut m = Mask::rectangle(0.0, 0.0, 8.0, 8.0);
                m.opacity = Property::zero();
                m
            },
            {
                // Off by opacity while asking to subtract: still off.
                let mut m = Mask::rectangle(0.0, 0.0, 8.0, 8.0);
                m.mode = MaskMode::Subtract;
                m.opacity = Property::zero();
                m
            },
        ] {
            let cov = combined_coverage(std::slice::from_ref(&off), 16, 16, 16.0, 16.0, 0.0);
            assert!(
                cov.iter().all(|c| *c == 255),
                "mode {:?} at {} % hid the layer",
                off.mode,
                off.opacity.value_at(0.0),
            );
        }
    }

    #[test]
    fn a_lone_subtract_mask_cuts_a_hole() {
        let mut m = Mask::rectangle(0.0, 0.0, 8.0, 16.0);
        m.mode = MaskMode::Subtract;
        let cov = combined_coverage(std::slice::from_ref(&m), 16, 16, 16.0, 16.0, 0.0);
        assert_eq!(at(&cov, 4), 0, "inside the subtracted shape");
        assert_eq!(at(&cov, 12), 255, "the rest of the frame stays");
    }

    #[test]
    fn subtract_order_matters() {
        let a = Mask::rectangle(0.0, 0.0, 10.0, 16.0);
        let b = Mask::rectangle(6.0, 0.0, 10.0, 16.0);
        let mut b_sub = b.clone();
        b_sub.mode = MaskMode::Subtract;
        let mut a_sub = a.clone();
        a_sub.mode = MaskMode::Subtract;

        let a_then_b = combined_coverage(&[a, b_sub], 16, 16, 16.0, 16.0, 0.0);
        let b_then_a = combined_coverage(&[b, a_sub], 16, 16, 16.0, 16.0, 0.0);
        assert_ne!(a_then_b, b_then_a);
        assert_eq!((at(&a_then_b, 2), at(&a_then_b, 12)), (255, 0));
        assert_eq!((at(&b_then_a, 2), at(&b_then_a, 12)), (0, 255));
    }

    #[test]
    fn zero_feather_and_expansion_leave_the_raster_untouched() {
        let m = Mask::ellipse(32.0, 32.0, 20.0, 12.0);
        assert_eq!(
            mask_coverage(&m, 64, 64, 1.0, 1.0, 0.0),
            rasterise(&m.path, 64, 64, 1.0, 1.0),
            "the fast path must return the rasteriser's own bytes"
        );
    }

    fn area(cov: &[u8]) -> f64 {
        cov.iter().map(|c| f64::from(*c) / 255.0).sum()
    }

    #[test]
    fn expansion_grows_and_shrinks_the_shape() {
        let base = Mask::rectangle(30.0, 30.0, 40.0, 40.0);
        let with = |e: f64| {
            let mut m = base.clone();
            m.expansion = Property::fixed(e);
            area(&mask_coverage(&m, 100, 100, 1.0, 1.0, 0.0))
        };
        // Growing a square by r gives a square with rounded corners:
        // (40+2r)² minus the four corner squares, plus the quarter-discs.
        let grown = 50.0 * 50.0 - 4.0 * 25.0 + std::f64::consts::PI * 25.0;
        assert!(
            (with(5.0) - grown).abs() / grown < 0.05,
            "grown {} vs {grown}",
            with(5.0)
        );
        // Shrinking gives the inset square, its corners a little rounded off
        // (distance is measured to the nearest point of the path, so corners
        // round both ways — as they do in After Effects).
        assert!(
            (with(-5.0) - 900.0).abs() / 900.0 < 0.08,
            "shrunk {}",
            with(-5.0)
        );
        assert_eq!(with(-40.0), 0.0, "a big negative expansion erases the mask");
    }

    #[test]
    fn feather_ramps_monotonically_across_the_edge() {
        let mut m = Mask::rectangle(0.0, 0.0, 50.0, 100.0); // left half
        m.feather = Property::fixed(12.0);
        let cov = mask_coverage(&m, 100, 100, 1.0, 1.0, 0.0);
        let row: Vec<u8> = (0..100).map(|x| cov[50 * 100 + x]).collect();
        for pair in row.windows(2) {
            assert!(pair[0] >= pair[1], "ramp goes back up: {row:?}");
        }
        assert_eq!(row[0], 255, "deep inside");
        assert_eq!(row[99], 0, "well outside");
        let soft = row.iter().filter(|c| **c > 0 && **c < 255).count();
        assert!(
            soft >= 8,
            "a 12px feather should span several pixels: {soft}"
        );
    }

    /// **Lighten and Darken are max and min against what the stack holds**
    /// (K-445). Read at half opacity, where they are visibly not Add and not
    /// Subtract: Add would saturate the overlap and Lighten must not.
    #[test]
    fn lighten_and_darken_take_the_greater_and_the_lesser() {
        // A at 50 %, B at 80 %, overlapping down the middle as `overlapping`
        // lays them out: x=2 is A only, x=8 is both, x=12 is B only.
        let combine = |mode: MaskMode| {
            let mut a = Mask::rectangle(0.0, 0.0, 10.0, 16.0);
            a.opacity = Property::fixed(50.0);
            let mut b = Mask::rectangle(6.0, 0.0, 10.0, 16.0);
            b.opacity = Property::fixed(80.0);
            b.mode = mode;
            combined_coverage(&[a, b], 16, 16, 16.0, 16.0, 0.0)
        };
        let (half, most) = (127u8, 204u8);
        let near = |got: u8, want: u8| (i16::from(got) - i16::from(want)).abs() <= 2;

        let light = combine(MaskMode::Lighten);
        assert!(near(at(&light, 2), half), "A alone {}", at(&light, 2));
        assert!(
            near(at(&light, 8), most),
            "the greater of the two, not their sum: {}",
            at(&light, 8)
        );
        assert!(near(at(&light, 12), most), "B alone {}", at(&light, 12));

        let dark = combine(MaskMode::Darken);
        assert_eq!(at(&dark, 2), 0, "outside B, the lesser is nothing");
        assert!(
            near(at(&dark, 8), half),
            "the lesser of the two: {}",
            at(&dark, 8)
        );
        assert_eq!(at(&dark, 12), 0, "outside A, the lesser is nothing");

        // Add saturates the overlap; Lighten is the mode that does not, which
        // is the whole reason it exists beside Add.
        let added = combine(MaskMode::Add);
        assert!(at(&added, 8) > at(&light, 8), "Add did not add");
    }

    /// A lone mask has to build from somewhere, and Lighten builds from
    /// nothing exactly as Add does — max against a full frame would leave the
    /// layer untouched, which is a mask doing the opposite of anything.
    #[test]
    fn a_lone_lighten_mask_shows_its_own_shape() {
        let mut m = Mask::rectangle(0.0, 0.0, 8.0, 16.0);
        m.mode = MaskMode::Lighten;
        let cov = combined_coverage(std::slice::from_ref(&m), 16, 16, 16.0, 16.0, 0.0);
        assert_eq!(at(&cov, 4), 255, "inside the shape");
        assert_eq!(at(&cov, 12), 0, "outside it");

        // Darken is the other way round: it cuts a full frame down to itself.
        let mut d = m.clone();
        d.mode = MaskMode::Darken;
        let cov = combined_coverage(std::slice::from_ref(&d), 16, 16, 16.0, 16.0, 0.0);
        assert_eq!(at(&cov, 4), 255);
        assert_eq!(at(&cov, 12), 0);
    }

    /// **The soft edge is as wide as the vertices near it say** (K-445): a
    /// rectangle sharp along its left edge and soft along its right one.
    ///
    /// Measured as the count of partly covered pixels on a row, which is what
    /// "how wide is the ramp here" means in a raster.
    #[test]
    fn feather_varies_along_the_path() {
        // 100 tall, x from 20 to 80. Vertices run (20,20) (80,20) (80,80)
        // (20,80), so 1 and 2 are the right-hand edge.
        let mut m = Mask::rectangle(20.0, 20.0, 60.0, 60.0);
        m.vertex_feather = vec![
            Property::fixed(0.0),
            Property::fixed(20.0),
            Property::fixed(20.0),
            Property::fixed(0.0),
        ];
        let cov = mask_coverage(&m, 100, 100, 1.0, 1.0, 0.0);
        let row: Vec<u8> = (0..100).map(|x| cov[50 * 100 + x]).collect();
        let soft = |from: usize, to: usize| {
            row[from..to]
                .iter()
                .filter(|c| **c > 0 && **c < 255)
                .count()
        };
        let (left, right) = (soft(0, 50), soft(50, 100));
        assert!(
            right >= 12,
            "a 20 px feather should span most of a dozen pixels: {right}"
        );
        assert!(
            left <= 2,
            "the sharp edge softened too: {left} soft pixels, row {row:?}"
        );
        assert!(right > left * 4, "{right} vs {left}");

        // And it is still a ramp, not a staircase: monotone out of the shape.
        assert!(
            row[50..].windows(2).all(|w| w[0] >= w[1]),
            "the soft edge goes back up: {:?}",
            &row[50..]
        );
    }

    /// A per-vertex list whose widths are all the same is the uniform feather,
    /// down to the byte — otherwise switching the feature on and changing
    /// nothing would quietly re-render every frame a project has banked.
    #[test]
    fn equal_vertex_feathers_are_the_uniform_feather() {
        let mut plain = Mask::rectangle(20.0, 20.0, 60.0, 60.0);
        plain.feather = Property::fixed(9.0);
        let mut listed = plain.clone();
        listed.vertex_feather = vec![Property::fixed(9.0); 4];
        assert_eq!(
            mask_coverage(&plain, 100, 100, 1.0, 1.0, 0.0),
            mask_coverage(&listed, 100, 100, 1.0, 1.0, 0.0),
        );

        // Including all-zero, which must still take the untouched-raster fast
        // path the ordinary hard-edged mask takes.
        let hard = Mask::rectangle(20.0, 20.0, 60.0, 60.0);
        let mut listed_zero = hard.clone();
        listed_zero.vertex_feather = vec![Property::zero(); 4];
        assert_eq!(
            mask_coverage(&listed_zero, 100, 100, 1.0, 1.0, 0.0),
            rasterise(&hard.path, 100, 100, 1.0, 1.0),
        );
    }

    /// A list shorter than the path falls back to the uniform width for the
    /// vertices it does not reach, rather than treating them as zero — a
    /// half-filled list is what a path with a point added to it leaves behind.
    #[test]
    fn a_short_vertex_feather_list_falls_back_to_the_uniform_width() {
        let mut m = Mask::rectangle(20.0, 20.0, 60.0, 60.0);
        m.feather = Property::fixed(16.0);
        // Only the first vertex named, and named sharp.
        m.vertex_feather = vec![Property::fixed(0.0)];
        let (uniform, widths) = m.feather_widths_at(4, 0.0);
        assert_eq!(uniform, 16.0);
        assert_eq!(widths, Some(vec![0.0, 16.0, 16.0, 16.0]));
    }

    /// The widths keyframe like the uniform feather does.
    #[test]
    fn a_vertex_feather_animates() {
        let mut m = Mask::rectangle(20.0, 20.0, 60.0, 60.0);
        m.vertex_feather = vec![
            Property::zero(),
            Property {
                animation: Animation::Keyframed(vec![
                    Keyframe {
                        time: Rational::new(0, 1).expect("0"),
                        value: 0.0,
                        interp_in: SideInterp::Linear,
                        interp_out: SideInterp::Linear,
                    },
                    Keyframe {
                        time: Rational::new(1, 1).expect("1"),
                        value: 24.0,
                        interp_in: SideInterp::Linear,
                        interp_out: SideInterp::Linear,
                    },
                ]),
                extra: serde_json::Map::new(),
            },
            Property::zero(),
            Property::zero(),
        ];
        assert_eq!(m.feather_widths_at(4, 0.0).1, None, "still sharp at 0 s");
        let at_half = m.feather_widths_at(4, 0.5).1.expect("varying by now");
        assert!((at_half[1] - 12.0).abs() < 1e-9, "{at_half:?}");
    }

    /// The list is absent from the file until somebody uses it, so every mask
    /// ever saved writes the bytes it always did — which is what keeps the
    /// frame cache's banked frames (K-338's promise, kept).
    #[test]
    fn an_unvaried_mask_serialises_as_it_always_did() {
        let m = Mask::rectangle(0.0, 0.0, 8.0, 8.0);
        let json = serde_json::to_string(&m).expect("a mask serialises");
        assert!(!json.contains("vertex_feather"), "{json}");

        let mut varied = m.clone();
        varied.vertex_feather = vec![Property::fixed(3.0), Property::fixed(7.5)];
        let json = serde_json::to_string(&varied).expect("a mask serialises");
        assert!(
            json.contains("\"vertex_feather\":[3.0,7.5]"),
            "still widths write as bare numbers: {json}"
        );
        let back: Mask = serde_json::from_str(&json).expect("and reads back");
        assert_eq!(back, varied);
    }

    #[test]
    fn feather_and_expansion_keep_their_shape_at_half_preview_scale() {
        let mut m = Mask::rectangle(25.0, 25.0, 50.0, 50.0);
        m.feather = Property::fixed(10.0);
        m.expansion = Property::fixed(4.0);
        // Path coordinates are natural 100×100 in both cases.
        let full = area(&mask_coverage(&m, 100, 100, 1.0, 1.0, 0.0)) / (100.0 * 100.0);
        let half = area(&mask_coverage(&m, 50, 50, 0.5, 0.5, 0.0)) / (50.0 * 50.0);
        assert!(
            (full - half).abs() < 0.02,
            "covered fraction {full} full vs {half} half"
        );
    }

    #[test]
    fn inverting_a_feathered_mask_is_the_complement_of_its_feather() {
        let mut m = Mask::ellipse(32.0, 32.0, 16.0, 16.0);
        m.feather = Property::fixed(9.0);
        let plain = combined_coverage(std::slice::from_ref(&m), 64, 64, 64.0, 64.0, 0.0);
        let mut inv = m.clone();
        inv.inverted = true;
        // Inverted alone would start the fold at zero for Add, so compare the
        // mask's own contribution: Add onto an empty stack is the coverage.
        let inverted = combined_coverage(std::slice::from_ref(&inv), 64, 64, 64.0, 64.0, 0.0);
        for (i, (p, q)) in plain.iter().zip(&inverted).enumerate() {
            assert_eq!(255 - p, *q, "pixel {i}: {p} then {q}");
        }
    }

    #[test]
    fn masks_without_the_new_fields_load_as_add() {
        let json = r#"{"id":"018f0000-0000-7000-8000-000000000000","name":"M",
            "path":{"vertices":[],"closed":true},"inverted":false,"opacity":100.0}"#;
        let m: Mask = serde_json::from_str(json).unwrap();
        assert_eq!(m.mode, MaskMode::Add);
        assert_eq!(m.feather, Property::zero());
        assert_eq!(m.expansion, Property::zero());
        assert_eq!(m.opacity, Property::fixed(100.0));
        // …and an untouched mask serialises exactly as it did before, so the
        // frame cache keeps the frames it already banked.
        let round = serde_json::to_string(&m).unwrap();
        assert!(!round.contains("mode"), "{round}");
        assert!(!round.contains("feather"), "{round}");
        assert!(round.contains(r#""opacity":100.0"#), "{round}");
    }

    /// **A still mask still writes bare numbers; only a keyed one grows.**
    ///
    /// The three values became animatable in K-340, and animatable normally
    /// means an object in the file. That would have retired every frame every
    /// project has banked, because the frame key names a mask by the bytes it
    /// serialises to — so the encoding stays a bare number until somebody
    /// actually keys the property.
    #[test]
    fn a_still_mask_writes_bare_numbers_and_a_keyed_one_does_not() {
        let mut m = Mask::rectangle(0.0, 0.0, 8.0, 8.0);
        m.feather = Property::fixed(4.0);
        let still = serde_json::to_string(&m).unwrap();
        assert!(still.contains(r#""opacity":100.0"#), "{still}");
        assert!(still.contains(r#""feather":4.0"#), "{still}");
        assert!(!still.contains("animation"), "{still}");
        assert_eq!(serde_json::from_str::<Mask>(&still).unwrap(), m);

        m.opacity = Property {
            animation: Animation::Keyframed(vec![Keyframe {
                time: Rational::new(0, 1).unwrap(),
                value: 20.0,
                interp_in: SideInterp::Linear,
                interp_out: SideInterp::Linear,
            }]),
            extra: serde_json::Map::new(),
        };
        let keyed = serde_json::to_string(&m).unwrap();
        assert!(keyed.contains("animation"), "{keyed}");
        // And it comes back as what it was, so a keyed mask survives a save.
        assert_eq!(serde_json::from_str::<Mask>(&keyed).unwrap(), m);
    }

    // ---- Animated paths -------------------------------------------------

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    fn pkey(t: (i64, i64), path: &BezierPath, side: SideInterp) -> PathKeyframe {
        PathKeyframe {
            time: rat(t.0, t.1),
            path: path.clone(),
            interp_in: side,
            interp_out: side,
        }
    }

    /// Move every vertex of a path by (dx, dy), tangents untouched.
    fn shifted(path: &BezierPath, dx: f64, dy: f64) -> BezierPath {
        BezierPath {
            vertices: path
                .vertices
                .iter()
                .map(|v| Vertex {
                    pos: (v.pos.0 + dx, v.pos.1 + dy),
                    ..*v
                })
                .collect(),
            closed: path.closed,
        }
    }

    /// Points along the curve itself, `per_seg` per segment.
    fn curve_points(p: &BezierPath, per_seg: usize) -> Vec<(f64, f64)> {
        let n = p.vertices.len();
        let segs = if p.closed { n } else { n.saturating_sub(1) };
        let mut out = Vec::with_capacity(segs * per_seg);
        for i in 0..segs {
            let (a, b) = (p.vertices[i], p.vertices[(i + 1) % n]);
            let c = [
                a.pos,
                (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1),
                (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1),
                b.pos,
            ];
            for s in 0..per_seg {
                let t = s as f64 / per_seg as f64;
                let u = 1.0 - t;
                let mix = |f: fn(&(f64, f64)) -> f64| {
                    u * u * u * f(&c[0])
                        + 3.0 * u * u * t * f(&c[1])
                        + 3.0 * u * t * t * f(&c[2])
                        + t * t * t * f(&c[3])
                };
                out.push((mix(|q| q.0), mix(|q| q.1)));
            }
        }
        out
    }

    /// The furthest any point of `a` sits from the nearest point of `b` — a
    /// one-sided Hausdorff distance, which is how "the same curve" is checked.
    fn deviation(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
        a.iter()
            .map(|p| {
                b.iter()
                    .map(|q| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt())
                    .fold(f64::MAX, f64::min)
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn an_unanimated_mask_is_byte_identical_on_disk() {
        // The frame-cache guarantee: adding path keys to the model must not
        // change one byte of a mask that has none, or every frame every
        // existing project has banked is retired.
        let m = Mask::ellipse(10.0, 10.0, 5.0, 5.0);
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("path_keys"), "{json}");
        // And a file written before path keys existed still loads.
        let old = r#"{"id":"018f0000-0000-7000-8000-000000000000","name":"M",
            "path":{"vertices":[],"closed":true},"inverted":false,"opacity":100.0}"#;
        let loaded: Mask = serde_json::from_str(old).unwrap();
        assert!(loaded.path_keys.is_empty());
        assert!(!loaded.path_is_animated());
        assert!(!serde_json::to_string(&loaded)
            .unwrap()
            .contains("path_keys"));
    }

    #[test]
    fn a_mask_without_keys_is_its_static_path_at_every_time() {
        let m = Mask::rectangle(1.0, 2.0, 3.0, 4.0);
        for t in [-100.0, 0.0, 0.5, 1e6] {
            assert_eq!(*m.path_at(t), m.path, "at t={t}");
        }
        // One key holds for all time, the static path ignored.
        let mut one = m.clone();
        let other = Mask::rectangle(50.0, 50.0, 3.0, 4.0).path;
        one.path_keys = vec![pkey((1, 1), &other, SideInterp::Linear)];
        for t in [-5.0, 1.0, 9.0] {
            assert_eq!(*one.path_at(t), other, "at t={t}");
        }
    }

    #[test]
    fn equal_vertex_counts_blend_position_and_both_tangents() {
        let a = Mask::ellipse(0.0, 0.0, 10.0, 10.0).path;
        let b = BezierPath {
            vertices: a
                .vertices
                .iter()
                .map(|v| Vertex {
                    pos: (v.pos.0 + 20.0, v.pos.1 + 4.0),
                    tan_in: (v.tan_in.0 * 3.0, v.tan_in.1 * 3.0),
                    tan_out: (v.tan_out.0 * 3.0, v.tan_out.1 * 3.0),
                })
                .collect(),
            closed: true,
        };
        let mut m = Mask::rectangle(0.0, 0.0, 1.0, 1.0);
        m.path_keys = vec![
            pkey((0, 1), &a, SideInterp::Linear),
            pkey((2, 1), &b, SideInterp::Linear),
        ];

        // Exactly each key at its own time.
        assert_eq!(*m.path_at(0.0), a);
        assert_eq!(*m.path_at(2.0), b);

        let mid = m.path_at(1.0);
        assert_eq!(mid.vertices.len(), a.vertices.len());
        for (i, v) in mid.vertices.iter().enumerate() {
            let (p, q) = (a.vertices[i], b.vertices[i]);
            let half = |x: f64, y: f64| (x + y) * 0.5;
            assert!(
                (v.pos.0 - half(p.pos.0, q.pos.0)).abs() < 1e-12,
                "pos.x {i}"
            );
            assert!(
                (v.pos.1 - half(p.pos.1, q.pos.1)).abs() < 1e-12,
                "pos.y {i}"
            );
            assert!(
                (v.tan_in.0 - half(p.tan_in.0, q.tan_in.0)).abs() < 1e-12,
                "tan_in {i}"
            );
            assert!(
                (v.tan_out.1 - half(p.tan_out.1, q.tan_out.1)).abs() < 1e-12,
                "tan_out {i}"
            );
        }
    }

    #[test]
    fn resampling_keeps_the_curve_it_was() {
        for base in [
            Mask::ellipse(30.0, 30.0, 20.0, 12.0).path,
            Mask::star(50.0, 50.0, 40.0, 16.0, 5).path,
            BezierPath {
                // An open path: three vertices, two segments, real handles.
                vertices: vec![
                    Vertex {
                        pos: (0.0, 0.0),
                        tan_in: (0.0, 0.0),
                        tan_out: (10.0, 20.0),
                    },
                    Vertex {
                        pos: (30.0, 0.0),
                        tan_in: (-8.0, 15.0),
                        tan_out: (8.0, -15.0),
                    },
                    Vertex {
                        pos: (60.0, 10.0),
                        tan_in: (-10.0, -20.0),
                        tan_out: (0.0, 0.0),
                    },
                ],
                closed: false,
            },
        ] {
            let n = base.vertices.len();
            for target in [n + 1, n + 3, n * 2, n * 3 + 1] {
                let r = resample(&base, target);
                assert_eq!(r.vertices.len(), target, "count for target {target}");
                assert_eq!(r.closed, base.closed);
                let dense = curve_points(&base, 2000);
                let probe = curve_points(&r, 200);
                let d = deviation(&probe, &dense);
                assert!(d < 0.02, "resample to {target} moved the curve by {d}");
            }
        }
    }

    #[test]
    fn resampling_is_deterministic() {
        let p = Mask::ellipse(3.0, -7.0, 11.0, 4.0).path;
        assert_eq!(resample(&p, 9), resample(&p, 9));
        // …and so is a whole interpolation built on it.
        let q = shifted(&p, 5.0, 5.0);
        let once = lerp_paths(&p, &resample(&q, 9), 0.37);
        let twice = lerp_paths(&p, &resample(&q, 9), 0.37);
        assert_eq!(once, twice);
    }

    #[test]
    fn mismatched_vertex_counts_still_land_on_each_key() {
        let sparse = Mask::ellipse(20.0, 20.0, 10.0, 10.0).path; // 4 vertices
        let dense = resample(&shifted(&sparse, 40.0, 0.0), 7); // 7 vertices
        let mut m = Mask::rectangle(0.0, 0.0, 1.0, 1.0);
        m.path_keys = vec![
            pkey((0, 1), &sparse, SideInterp::Linear),
            pkey((1, 1), &dense, SideInterp::Linear),
        ];

        // At each key the shape is that key's own curve, exactly — a key's own
        // time never goes near the blend.
        assert_eq!(*m.path_at(0.0), sparse);
        // Just inside the span the counts are reconciled upward, and the shape
        // is still the sparse key's curve.
        let just_after = m.path_at(1e-9);
        assert_eq!(just_after.vertices.len(), 7, "reconciled upward");
        let d = deviation(
            &curve_points(&just_after, 200),
            &curve_points(&sparse, 2000),
        );
        assert!(d < 0.02, "the first key's curve moved by {d}");
        let at_end = m.path_at(1.0);
        assert_eq!(*at_end, dense, "the denser key is used as it stands");

        // And halfway across, halfway along.
        let mid = m.path_at(0.5);
        assert_eq!(mid.vertices.len(), 7);
        assert!((mid.vertices[0].pos.0 - (sparse.vertices[0].pos.0 + 20.0)).abs() < 1e-9);
    }

    #[test]
    fn hold_holds_and_a_bezier_ease_is_not_linear_in_the_middle() {
        let a = Mask::rectangle(0.0, 0.0, 10.0, 10.0).path;
        let b = shifted(&a, 100.0, 0.0);
        let mut m = Mask::rectangle(0.0, 0.0, 1.0, 1.0);

        m.path_keys = vec![
            pkey((0, 1), &a, SideInterp::Hold),
            pkey((1, 1), &b, SideInterp::Hold),
        ];
        assert_eq!(*m.path_at(0.5), a, "a held span does not move");
        assert_eq!(*m.path_at(0.999), a);
        assert_eq!(*m.path_at(1.0), b, "and steps at the next key");

        m.path_keys = vec![
            pkey((0, 1), &a, crate::anim::EASY_EASE),
            pkey((1, 1), &b, crate::anim::EASY_EASE),
        ];
        assert_eq!(*m.path_at(0.0), a, "exact at the first key");
        assert_eq!(*m.path_at(1.0), b, "exact at the last key");
        // Linear would put the shape at x=25 a quarter of the way through; an
        // ease is still gathering pace.
        let quarter = m.path_at(0.25).vertices[0].pos.0;
        assert!(
            quarter < 20.0 && quarter > 0.0,
            "eased quarter at x={quarter}, linear would be 25"
        );
    }

    #[test]
    fn a_closed_path_stays_closed_and_closedness_is_held_not_blended() {
        let closed = Mask::ellipse(20.0, 20.0, 10.0, 10.0).path;
        let mut open = shifted(&closed, 30.0, 0.0);
        open.closed = false;
        let mut m = Mask::rectangle(0.0, 0.0, 1.0, 1.0);

        m.path_keys = vec![
            pkey((0, 1), &closed, SideInterp::Linear),
            pkey((1, 1), &shifted(&closed, 30.0, 0.0), SideInterp::Linear),
        ];
        for t in [0.0, 0.3, 0.5, 1.0] {
            assert!(m.path_at(t).closed, "closed at t={t}");
        }

        // Closed → open: the flag is not a quantity, so it holds across the
        // span and flips at the second key, exactly like a Hold keyframe. The
        // geometry interpolates normally throughout.
        m.path_keys = vec![
            pkey((0, 1), &closed, SideInterp::Linear),
            pkey((1, 1), &open, SideInterp::Linear),
        ];
        assert!(m.path_at(0.0).closed);
        assert!(m.path_at(0.99).closed, "held until the next key");
        assert!(!m.path_at(1.0).closed, "and the open key is exactly itself");
        assert!(
            (m.path_at(0.5).vertices[0].pos.0 - (closed.vertices[0].pos.0 + 15.0)).abs() < 1e-9
        );
    }

    #[test]
    fn an_animated_mask_gates_different_pixels_at_different_times() {
        // The end-to-end proof: a 4-wide square crossing a 16-wide frame.
        let left = Mask::rectangle(0.0, 0.0, 4.0, 16.0).path;
        let right = shifted(&left, 12.0, 0.0);
        let mut m = Mask::rectangle(0.0, 0.0, 1.0, 1.0);
        m.path_keys = vec![
            pkey((0, 1), &left, SideInterp::Linear),
            pkey((1, 1), &right, SideInterp::Linear),
        ];

        let alpha_at = |t: f64| {
            let mut rgba = vec![255u8; 16 * 16 * 4];
            apply_masks(&mut rgba, 16, 16, 16.0, 16.0, std::slice::from_ref(&m), t);
            let px = |x: usize| rgba[(8 * 16 + x) * 4 + 3];
            (px(2), px(14))
        };
        assert_eq!(alpha_at(0.0), (255, 0), "the square starts at the left");
        assert_eq!(alpha_at(1.0), (0, 255), "and ends at the right");
    }
}
