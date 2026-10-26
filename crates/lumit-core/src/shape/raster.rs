//! Drawing one run of a shape layer's art: the taper, the gradient, the
//! combine, the dashes, and the outline an effect walks.

use super::geometry::{densified, dist, flatten_path, polyline_path, transform_path, Affine};
use super::modifier::{self, Lowered, ShapeModifier};
use super::{colour, GradientStop, ShapeItem, Taper, MAX_GRADIENT_STOPS};
use crate::anim::Property;
use crate::mask::BezierPath;

/// A line the outline's brush runs along, and how wide the brush is at each
/// of its points, 0..1 of the full width. No widths is full width.
pub(super) type Pressed = (Vec<(f64, f64)>, Vec<f64>);

/// The narrowest a taper draws, as a share of the width: a taper to nothing
/// ends in a fine point rather than stopping short of where the line does.
pub(super) const TAPER_TIP: f64 = 1e-3;

impl Taper {
    /// `lines` with the brush's width at each point. Each line is cut finely
    /// enough first that the narrowing reads as a curve.
    pub(super) fn applied(&self, lines: Vec<Vec<(f64, f64)>>, t: f64) -> Vec<Pressed> {
        let share = |p: &Property| p.value_at(t).clamp(0.0, 100.0) / 100.0;
        let (start, end, ease) = (share(&self.start), share(&self.end), share(&self.ease));
        let (mut from, mut to) = (share(&self.start_length), share(&self.end_length));
        if from + to > 1.0 {
            let over = from + to;
            from /= over;
            to /= over;
        }
        // How far down a ramp `x` of the way along it is: straight, or
        // rounded by the ease.
        let ramp = |x: f64| {
            let x = x.clamp(0.0, 1.0);
            x + (x * x * (3.0 - 2.0 * x) - x) * ease
        };
        let width = |u: f64| {
            let mut w: f64 = 1.0;
            if from > 0.0 && u < from {
                w = w.min(1.0 - start * (1.0 - ramp(u / from)));
            }
            if to > 0.0 && u > 1.0 - to {
                w = w.min(1.0 - end * (1.0 - ramp((1.0 - u) / to)));
            }
            w.max(TAPER_TIP)
        };
        let lengths: Vec<f64> = lines
            .iter()
            .map(|l| l.windows(2).map(|p| dist(p[0], p[1])).sum())
            .collect();
        let whole: f64 = lengths.iter().sum();
        let mut before = 0.0;
        lines
            .into_iter()
            .zip(lengths)
            .map(|(line, length)| {
                let (mut along, span) = if self.scope == 1 {
                    (before, whole)
                } else {
                    (0.0, length)
                };
                before += length;
                if span <= 0.0 || line.is_empty() {
                    return (line, Vec::new());
                }
                let shortest = [from, to]
                    .into_iter()
                    .filter(|&r| r > 0.0)
                    .fold(1.0f64, f64::min);
                let line = densified(&line, (shortest * span / 32.0).max(0.25));
                let mut last = line[0];
                let pressures = line
                    .iter()
                    .map(|&p| {
                        along += dist(last, p);
                        last = p;
                        width(along / span)
                    })
                    .collect();
                (line, pressures)
            })
            .collect()
    }
}

impl ShapeItem {
    /// The gradient this item's fill is drawn with at `t`, placed by `to_box`
    /// (a repeated copy carries its gradient with it) and scaled into the
    /// buffer's pixels — `None` for the flat fill every shape has until
    /// somebody ramps it.
    pub(super) fn ramp_at(&self, t: f64, to_box: &Affine, sx: f64, sy: f64) -> Option<Ramp> {
        if self.gradient == 0 {
            return None;
        }
        let from = colour::at(self.fill.as_ref()?, t);
        let to = self
            .gradient_colour
            .unwrap_or(crate::model::LinearColour::BLACK);
        let place = |x: f64, y: f64| {
            let (x, y) = to_box.apply((x, y));
            (x * sx, y * sy)
        };
        let start = place(
            self.gradient_start_x.value_at(t),
            self.gradient_start_y.value_at(t),
        );
        let end = place(
            self.gradient_end_x.value_at(t),
            self.gradient_end_y.value_at(t),
        );
        let axis = (end.0 - start.0, end.1 - start.1);
        // One epsilon on the squared length, so the linear and the radial
        // reading degenerate at exactly the same point — the Gradient effect's
        // rule, and for the same reason.
        let len2 = (axis.0 * axis.0 + axis.1 * axis.1).max(1e-6);
        // Each end's opacity goes into its colour's alpha, so the table fades
        // as it mixes.
        let faded = |p: &Property| (p.value_at(t).clamp(0.0, 100.0) / 100.0) as f32;
        let from_alpha = from.0[3] * faded(&self.gradient_start_opacity);
        let to_alpha = to.0[3] * faded(&self.gradient_end_opacity);
        // The ramp's colours in order: the fill, the stops between, then the
        // gradient colour.
        let mut between: Vec<(f64, &GradientStop)> = self
            .gradient_stops
            .iter()
            .take(MAX_GRADIENT_STOPS)
            .map(|s| (s.position.value_at(t), s))
            .filter(|(at, _)| at.is_finite())
            .collect();
        between.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut points = Vec::with_capacity(between.len() + 2);
        points.push((0.0f32, [from.0[0], from.0[1], from.0[2], from_alpha]));
        for (at, stop) in between {
            let c = stop.colour.0;
            let alpha = c[3] * faded(&stop.opacity);
            let at = (at.clamp(0.0, 100.0) / 100.0) as f32;
            points.push((at, [c[0], c[1], c[2], alpha]));
        }
        points.push((1.0, [to.0[0], to.0[1], to.0[2], to_alpha]));
        let mut lut = [[0u8; 4]; RAMP_STEPS];
        let mut piece = 0;
        for (i, cell) in lut.iter_mut().enumerate() {
            let u = i as f32 / (RAMP_STEPS - 1) as f32;
            // The piece of the ramp this step falls in. Stepping past a stop
            // as soon as it is reached is what makes two at one place an edge.
            while piece + 2 < points.len() && u >= points[piece + 1].0 {
                piece += 1;
            }
            let (a_at, a) = points[piece];
            let (b_at, b) = points[piece + 1];
            let span = b_at - a_at;
            let v = if span > 0.0 {
                ((u - a_at) / span).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let mix = crate::model::LinearColour([
                a[0] + (b[0] - a[0]) * v,
                a[1] + (b[1] - a[1]) * v,
                a[2] + (b[2] - a[2]) * v,
                a[3] + (b[3] - a[3]) * v,
            ]);
            *cell = crate::pixels::solid_rgba(mix);
        }
        Some(Ramp {
            radial: self.gradient == 2,
            start,
            axis,
            inv_len2: 1.0 / len2,
            inv_len: 1.0 / len2.sqrt(),
            lut,
        })
    }

    /// The dash pattern at `t` in layer pixels, evened out — empty when the
    /// outline is solid, which is the ordinary case.
    ///
    /// An odd-length list is repeated (the SVG rule): `[10]` is ten on, ten off,
    /// not ten on and nothing said about the rest. A pattern whose lengths are
    /// all zero is no pattern at all.
    pub(super) fn dash_pattern_at(&self, t: f64) -> Vec<f64> {
        let mut pattern: Vec<f64> = self.dashes.iter().map(|p| p.value_at(t).max(0.0)).collect();
        if pattern.iter().all(|&d| d <= 0.0) {
            return Vec::new();
        }
        if !pattern.len().is_multiple_of(2) {
            pattern.extend_from_within(..);
        }
        pattern
    }
}

/// The layer's contents split into the pieces that draw as **one**.
///
/// An item whose [`combine`](ShapeItem::combine) is 0 starts a run of its own;
/// each item after it that names a combine joins the run in front of it, when
/// the entry above it is an item (`Lowered::joins`). The first item of a
/// group always starts a run, whatever it says, because there is nothing
/// before it to join. A layer nobody has combined is therefore one run per
/// item, which is the list this crate has always walked.
pub(super) fn runs(items: &[Lowered]) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, art) in items.iter().enumerate() {
        match out.last_mut() {
            Some(run) if art.joins => run.end = i + 1,
            _ => out.push(i..i + 1),
        }
    }
    out
}

/// The contours a combined run draws at `t`, in the art's own
/// coordinates and before the first item's offset and trim have had them.
///
/// The run is folded left to right — each item is combined with everything
/// combined so far — so three items read as `(A ∪ B) − C`, which is the order
/// they are written in. Every path is **flattened first**: the boolean works on
/// polygons, so a curve is walked into one exactly as a trim walks it.
///
/// Degenerate input is a **no-op** rather than a refusal: a member with fewer
/// than three points has no area to combine, and dropping the whole run over it
/// would make a half-drawn path erase the art it was being joined to.
fn combined_at(run: &[Lowered], t: f64) -> Vec<Vec<(f64, f64)>> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;

    /// A polygon the boolean can be handed: closed, at least a triangle, and
    /// finite. Anything else is nothing to combine with.
    fn polygon(item: &ShapeItem, t: f64) -> Option<Vec<[f64; 2]>> {
        let points = flatten_path(&item.path_at(t));
        let out: Vec<[f64; 2]> = points
            .iter()
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .map(|&(x, y)| [x, y])
            .collect();
        (out.len() >= 3).then_some(out)
    }

    let Some((first, rest)) = run.split_first() else {
        return Vec::new();
    };
    let Some(subject) = polygon(first.item, t) else {
        return Vec::new();
    };
    let mut acc: Vec<Vec<[f64; 2]>> = vec![subject];
    for art in rest {
        let item = art.item;
        let rule = match item.combine {
            1 => OverlayRule::Union,
            2 => OverlayRule::Difference,
            3 => OverlayRule::Intersect,
            4 => OverlayRule::Xor,
            // A reading nobody wrote down draws the art it was going to
            // combine with, which is the answer that shows something.
            _ => continue,
        };
        let Some(clip) = polygon(item, t) else {
            continue;
        };
        // **Non-zero**, the rule a shape's fill is drawn by, so a combine reads
        // each path the way the picture already shows it: a pentagram drawn
        // in one stroke has a filled middle before the union and after it.
        let shapes = acc.overlay(&vec![clip], rule, FillRule::NonZero);
        acc = shapes.into_iter().flatten().collect();
        if acc.is_empty() {
            return Vec::new();
        }
    }
    acc.into_iter()
        .map(|c| c.into_iter().map(|[x, y]| (x, y)).collect())
        .collect()
}

/// One run of a layer's contents as it is drawn at `t`, before the repeater
/// places it. The picture and an effect walking the outline both start here,
/// so a combine, an offset and a trim mean the same thing to each.
pub(super) struct Art<'a> {
    /// The run's first item, whose paint the whole run takes.
    pub(super) item: &'a ShapeItem,
    /// The modifiers acting on that item, which the whole run is drawn through.
    pub(super) mods: &'a [&'a ShapeModifier],
    /// The item's own shape this instant, which a keyed path moves.
    drawn: std::borrow::Cow<'a, BezierPath>,
    /// What a combine, an offset or a trim made of it. `None` is "nothing
    /// has changed it", which draws the bezier itself.
    lines: Option<Vec<Vec<(f64, f64)>>>,
    /// A combined run, whose contours close on themselves.
    ring: bool,
}

impl<'a> Art<'a> {
    /// The art of `run`, or `None` when none is left to draw: trimmed away
    /// to nothing (the first frame of a write-on), or a combine that left no
    /// art at all.
    pub(super) fn at(
        items: &'a [Lowered<'a>],
        run: std::ops::Range<usize>,
        t: f64,
    ) -> Option<Self> {
        let members = items.get(run)?;
        let first = members.first()?;
        let (item, mods) = (first.item, first.mods.as_slice());
        let ring = members.len() > 1;
        let drawn = item.path_at(t);
        let lines: Option<Vec<Vec<(f64, f64)>>> = if ring {
            Some(
                combined_at(members, t)
                    .into_iter()
                    .map(|c| modifier::shaped(mods, c, true, &drawn, t))
                    .collect(),
            )
        } else {
            item.trimmed_at(mods, t).map(|p| vec![p])
        };
        let lines = match lines {
            Some(lines) => {
                let lines: Vec<_> = lines.into_iter().filter(|p| p.len() >= 2).collect();
                if lines.is_empty() {
                    return None;
                }
                Some(lines)
            }
            None => None,
        };
        Some(Self {
            item,
            mods,
            drawn,
            lines,
            ring,
        })
    }

    /// The paths one copy draws, placed by `to_box`.
    pub(super) fn placed(&self, to_box: &Affine) -> Vec<BezierPath> {
        // A boolean's contours close on themselves; a trimmed piece has two
        // ends whatever the path it was cut from had.
        let closed = self.ring || self.drawn.closed;
        match &self.lines {
            // A trimmed piece or a boolean's contour is a polyline, and a
            // polyline is a bezier whose handles are all zero — one path
            // type, still. Closed so the fill has something to
            // fill: a half-trimmed circle fills as a half circle, exactly
            // as AE draws it.
            Some(lines) => lines
                .iter()
                .map(|points| {
                    polyline_path(
                        &points.iter().map(|&p| to_box.apply(p)).collect::<Vec<_>>(),
                        closed,
                    )
                })
                .collect(),
            None => vec![transform_path(&self.drawn, to_box)],
        }
    }

    /// The lines the outline's brush runs along for one copy's `placed`
    /// paths, cut into their dashes, each with how wide the brush is along
    /// it — empty for the full width all the way, which is every outline
    /// nobody has tapered. `scale` is how much bigger the copy draws, which
    /// its dashes grow with.
    pub(super) fn stroked(&self, placed: &[BezierPath], scale: f64, t: f64) -> Vec<Pressed> {
        let item = self.item;
        let outlined = item.stroke.is_some() && item.stroke_width.value_at(t) > 0.0;
        // Dashes cut the outline into pieces, each of which is a brush run
        // of its own. A solid outline is one piece, which is the
        // same single run it always was. An item with no outline has no
        // dashes to cut it.
        let pattern: Vec<f64> = if outlined {
            item.dash_pattern_at(t).iter().map(|d| d * scale).collect()
        } else {
            Vec::new()
        };
        let lines: Vec<Vec<(f64, f64)>> = placed
            .iter()
            .map(|path| {
                let mut points: Vec<(f64, f64)> = match &self.lines {
                    Some(_) => path.vertices.iter().map(|v| v.pos).collect(),
                    None => flatten_path(path),
                };
                if let (true, Some(&first)) = (self.ring, points.first()) {
                    points.push(first);
                }
                points
            })
            .collect();
        // The taper is laid along the whole line before the dashes cut it,
        // so each dash is as wide as the outline is where it falls.
        let lines: Vec<Pressed> = match (&item.taper, outlined) {
            (Some(taper), true) => taper.applied(lines, t),
            _ => lines.into_iter().map(|l| (l, Vec::new())).collect(),
        };
        lines
            .into_iter()
            .flat_map(|(points, pressures)| {
                if pattern.is_empty() {
                    vec![(points, pressures)]
                } else {
                    dashed(
                        &points,
                        &pressures,
                        &pattern,
                        item.dash_offset.value_at(t) * scale,
                    )
                }
            })
            .collect()
    }

    /// The item's own vertices where `to_box` places them, sorted by x, while
    /// the outline still runs through them. An offset moves it off them and a
    /// combine cuts it elsewhere, and then there are none to name.
    pub(super) fn own(&self, to_box: &Affine, t: f64) -> Option<Vec<(f64, f64)>> {
        if self.ring || modifier::displaces(self.mods, t) {
            return None;
        }
        let mut own: Vec<(f64, f64)> = self
            .drawn
            .vertices
            .iter()
            .map(|v| to_box.apply(v.pos))
            .collect();
        own.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        Some(own)
    }
}

/// One stroked line as a path for an effect to walk: closed where it comes
/// back to its start, with a vertex at each end and at each of `own`, the
/// item's own vertices. With no `own` to go by a vertex is wherever the line
/// turns. The joints along a straight run are never vertices.
pub(super) fn walked(
    mut points: Vec<(f64, f64)>,
    own: Option<&[(f64, f64)]>,
) -> crate::mask::MaskPolyline {
    points.dedup();
    if points.len() < 2 {
        return crate::mask::MaskPolyline::default();
    }
    let closed = points.len() > 2 && points.first() == points.last();
    // A closed line ends on its first point again, which is no second vertex.
    let n = points.len() - usize::from(closed);
    let at = |j: usize| points.get(j % n).copied().unwrap_or_default();
    let corners = (0..n)
        .filter(|&i| {
            if !closed && (i == 0 || i + 1 == n) {
                return true;
            }
            let b = at(i);
            match own {
                // Cutting a line moves a point by a rounding error, so near
                // enough is the same place.
                Some(own) => {
                    const NEAR: f64 = 1e-6;
                    let from = own.partition_point(|v| v.0 < b.0 - NEAR);
                    own.iter()
                        .skip(from)
                        .take_while(|v| v.0 <= b.0 + NEAR)
                        .any(|v| (v.1 - b.1).abs() <= NEAR)
                }
                None => {
                    let (a, c) = (at(i + n - 1), at(i + 1));
                    let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
                    // The sine of the turn, with a doubling back counted too.
                    let straight = u.0.hypot(u.1) * v.0.hypot(v.1) * 1e-3;
                    (u.0 * v.1 - u.1 * v.0).abs() > straight || u.0 * v.0 + u.1 * v.1 < 0.0
                }
            }
        })
        .collect();
    let points: Vec<[f32; 2]> = points.iter().map(|p| [p.0 as f32, p.1 as f32]).collect();
    let mut arc = Vec::with_capacity(points.len());
    let mut far = 0.0f32;
    arc.push(far);
    for pair in points.windows(2) {
        if let [a, b] = pair {
            far += (b[0] - a[0]).hypot(b[1] - a[1]);
        }
        arc.push(far);
    }
    crate::mask::MaskPolyline {
        points,
        arc,
        closed,
        corners,
        ..Default::default()
    }
}

/// A gradient fill worked out for one drawn copy: the ramp's geometry in the
/// **buffer's** own pixels, and the colours it runs through as a lookup table.
///
/// A table rather than a lerp per pixel because the two ends are scene-linear
/// and the buffer is 8-bit: mixing has to happen in linear and be encoded
/// after, which is a transcendental per pixel done honestly and a table lookup
/// done sensibly. 256 steps is finer than the 8-bit result can show.
pub(super) struct Ramp {
    radial: bool,
    start: (f64, f64),
    axis: (f64, f64),
    inv_len2: f64,
    inv_len: f64,
    lut: [[u8; 4]; RAMP_STEPS],
}

/// How many colours a gradient is worked out at. One per 8-bit level, so the
/// table cannot be the thing you can see.
const RAMP_STEPS: usize = 256;

impl Ramp {
    /// The colour at a pixel centre, `[r, g, b]` encoded and the alpha 0..1.
    pub(super) fn at(&self, x: f64, y: f64) -> ([u8; 3], f32) {
        let (dx, dy) = (x - self.start.0, y - self.start.1);
        // The Gradient effect's two readings (docs/08 §3.35): linear projects
        // onto the axis, radial measures how far out you are with the end
        // point on the outer edge. Both reciprocals were floored against one
        // epsilon when the ramp was built, so a zero-length axis is one flat
        // colour rather than a division by zero (docs/14 §4).
        let along = if self.radial {
            (dx * dx + dy * dy).sqrt() * self.inv_len
        } else {
            (dx * self.axis.0 + dy * self.axis.1) * self.inv_len2
        };
        let i = (along.clamp(0.0, 1.0) * (RAMP_STEPS - 1) as f64).round() as usize;
        let c = self.lut[i.min(RAMP_STEPS - 1)];
        ([c[0], c[1], c[2]], f32::from(c[3]) / 255.0)
    }
}

/// The most pieces one dashed outline is cut into.
///
/// A pattern fine enough to need more than this is, at any size a stroke is
/// drawn at, a solid line to the eye — and cutting it into a hundred thousand
/// pieces would cost a frame to draw something indistinguishable from one
/// piece. Past the ceiling the outline is drawn **solid** rather than truncated,
/// because a stroke that stopped half way along would be a visible wrong answer
/// where a solid one is an invisible one.
const MAX_DASHES: usize = 4096;

/// `points` cut into its dashes: the "on" pieces of `pattern` (dash, gap, dash,
/// gap, in the polyline's own units), started `offset` along the path.
///
/// The cutting is the same length-measured cut a trim makes, so a dash and a
/// trim agree about where "ten units along" is. `pressures` is cut with the
/// points, so a dash keeps the width the line had where it falls.
pub(super) fn dashed(
    points: &[(f64, f64)],
    pressures: &[f64],
    pattern: &[f64],
    offset: f64,
) -> Vec<Pressed> {
    let whole = || vec![(points.to_vec(), pressures.to_vec())];
    let cycle: f64 = pattern.iter().sum();
    if pattern.is_empty() || cycle <= 0.0 || points.len() < 2 {
        return whole();
    }
    let total: f64 = points
        .windows(2)
        .map(|p| ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt())
        .sum();
    if total <= 0.0 || (total / cycle) * pattern.len() as f64 > MAX_DASHES as f64 {
        return whole();
    }

    let mut out = Vec::new();
    // Start inside the cycle *before* the path, so the first dash can be part
    // way through when the offset says so.
    let mut at = -offset.rem_euclid(cycle);
    let mut i = 0usize;
    while at < total {
        let length = pattern[i % pattern.len()];
        let end = at + length;
        // Even entries are dashes, odd ones gaps — the SVG and AE convention.
        if i.is_multiple_of(2) && length > 0.0 && end > 0.0 && at < total {
            let piece = crate::paint::trimmed_with(
                points,
                pressures,
                at.max(0.0) / total * 100.0,
                end.min(total) / total * 100.0,
            );
            if piece.0.len() >= 2 {
                out.push(piece);
            }
        }
        at = end;
        i += 1;
    }
    out
}
