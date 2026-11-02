//! The path as straight pieces in raster pixels, cut to the lit stretch, and
//! the tiles each piece can reach.

use crate::mask::MaskPolyline;

/// The most pieces a frame draws. A longer path is walked with fewer, longer
/// pieces rather than cut short.
pub const MAX_PIECES: usize = 16_384;

/// The side of a tile, raster pixels.
pub const TILE: u32 = 16;

/// The most tile entries a frame files. Past it the last pieces go unfiled,
/// which only a path far longer than a frame can reach.
pub const MAX_BIN_ITEMS: usize = 1 << 21;

/// The lit pieces, raster pixels.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pieces {
    /// `(ax, ay, bx, by)`.
    pub segs: Vec<[f32; 4]>,
    /// How far along the lit stretch each end sits, 0 at its start and 1 at
    /// its end, for the taper; which moment of the shutter the piece belongs
    /// to; and how present its stretch is, which is less than 1 while the
    /// stretch is shorter than the core is wide, so a line drawing on grows
    /// out of nothing rather than popping in as a dot.
    pub along: Vec<[f32; 4]>,
}

impl Pieces {
    /// Another moment's pieces after these.
    pub fn extend(&mut self, more: Pieces) {
        self.segs.extend(more.segs);
        self.along.extend(more.along);
    }
}

/// Which pieces each tile has to measure.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bins {
    pub cols: u32,
    pub rows: u32,
    /// Tile `i`'s pieces are `items[start[i]..start[i + 1]]`.
    pub start: Vec<u32>,
    pub items: Vec<u32>,
}

impl Bins {
    /// The pieces the pixel at `(x, y)` measures.
    #[must_use]
    pub fn at(&self, x: u32, y: u32) -> &[u32] {
        let (c, r) = (x / TILE, y / TILE);
        if c >= self.cols || r >= self.rows {
            return &[];
        }
        let i = (r * self.cols + c) as usize;
        let (a, b) = (self.start[i] as usize, self.start[i + 1] as usize);
        &self.items[a..b]
    }
}

/// [`pieces`] for each path of a run on its own, so every mask, item or
/// letter draws on at once rather than one after another. The steps between
/// paths have no length, which is how the run is cut back into its paths.
#[must_use]
pub fn pieces_each(
    poly: &MaskPolyline,
    scale: f32,
    trim: [f32; 3],
    sample: u32,
    width: f32,
) -> Pieces {
    let mut out = Pieces::default();
    let n = poly.points.len().min(poly.arc.len());
    let mut from = 0;
    for i in 1..=n {
        let step = i < n && poly.arc[i] <= poly.arc[i - 1] && poly.points[i] != poly.points[i - 1];
        if i < n && !step {
            continue;
        }
        if i - from >= 2 {
            let points = poly.points[from..i].to_vec();
            let base = poly.arc[from];
            // A closed path ends on its first point again.
            let closed = points.first() == points.last();
            let path = MaskPolyline {
                arc: poly.arc[from..i].iter().map(|a| a - base).collect(),
                points,
                closed,
                ..MaskPolyline::default()
            };
            out.extend(pieces(&path, scale, trim, sample, width));
        }
        from = i;
    }
    out.segs.truncate(MAX_PIECES);
    out.along.truncate(MAX_PIECES);
    out
}

/// The stretch of `poly` from Start to End per cent of its length, slid by
/// Offset per cent (`trim`), as pieces in raster pixels (`scale` raster pixels
/// a comp pixel), tagged with shutter moment `sample`. `width` is the core's
/// widest, raster pixels.
///
/// A run of several paths (every mask, a shape's items, the letters) is one
/// path for this: the stretch is measured along all of them in turn, and the
/// steps between them, which have no length, are never drawn. A closed run
/// comes round again when Offset slides the stretch past its end; an open one
/// is cut off there.
#[must_use]
pub fn pieces(poly: &MaskPolyline, scale: f32, trim: [f32; 3], sample: u32, width: f32) -> Pieces {
    let [start, end, offset] = trim;
    let mut out = Pieces::default();
    let total = poly.length();
    let (a0, b0) = (
        (start / 100.0).clamp(0.0, 1.0),
        (end / 100.0).clamp(0.0, 1.0),
    );
    if poly.is_empty() || total <= 0.0 || b0 <= a0 || !offset.is_finite() {
        return out;
    }
    let shift = offset / 100.0;
    // The lit stretch as one or two windows of arc length, and how far into
    // the stretch each window begins.
    let mut windows: Vec<(f32, f32, f32)> = Vec::with_capacity(2);
    if poly.closed {
        let a = (a0 + shift).rem_euclid(1.0) * total;
        let span = (b0 - a0) * total;
        if a + span <= total {
            windows.push((a, a + span, 0.0));
        } else {
            windows.push((a, total, 0.0));
            windows.push((0.0, a + span - total, total - a));
        }
    } else {
        let (a, b) = (
            ((a0 + shift) * total).clamp(0.0, total),
            ((b0 + shift) * total).clamp(0.0, total),
        );
        if b > a {
            // What slid off the front still counts toward the taper, so a
            // stretch sliding off the end keeps its shape as it goes.
            windows.push((a, b, a - (a0 + shift) * total));
        }
    }
    let span = ((b0 - a0) * total).max(1e-6);
    let present = (span * scale / width.max(1e-6)).clamp(0.0, 1.0);
    let edges = poly.points.len() - 1;
    // ponytail: a path past the cap is walked every `stride` vertices, which
    // cuts corners on a very long path. Simplify by curvature if that shows.
    let drawn = poly.arc.windows(2).filter(|a| a[1] > a[0]).count();
    let stride = drawn.div_ceil(MAX_PIECES).max(1);
    for &(wa, wb, into) in &windows {
        let mut i = 0;
        while i < edges {
            // One piece from vertex i to vertex j, never across a step.
            let mut j = i + 1;
            while j < edges && j - i < stride && poly.arc[j + 1] > poly.arc[j] {
                j += 1;
            }
            let (s0, s1) = (poly.arc[i], poly.arc[j]);
            // A step between two paths has no length.
            let jump = s1 <= s0;
            if !jump && s1 > wa && s0 < wb && out.segs.len() < MAX_PIECES {
                let (c0, c1) = (s0.max(wa), s1.min(wb));
                let at = |s: f32| {
                    let t = (s - s0) / (s1 - s0);
                    let (p, q) = (poly.points[i], poly.points[j]);
                    [
                        (p[0] + (q[0] - p[0]) * t) * scale,
                        (p[1] + (q[1] - p[1]) * t) * scale,
                    ]
                };
                let (p, q) = (at(c0), at(c1));
                out.segs.push([p[0], p[1], q[0], q[1]]);
                out.along.push([
                    (c0 - wa + into) / span,
                    (c1 - wa + into) / span,
                    sample as f32,
                    present,
                ]);
            }
            i = j;
        }
    }
    out
}

/// File each piece into every `TILE`-pixel tile of a `w × h` raster that it
/// comes within `reach` raster pixels of.
#[must_use]
pub fn bins(p: &Pieces, reach: f32, w: u32, h: u32) -> Bins {
    let (cols, rows) = (w.div_ceil(TILE).max(1), h.div_ceil(TILE).max(1));
    let tiles = (cols * rows) as usize;
    let reach = reach.max(0.0);
    let span = |s: &[f32; 4]| {
        let lo = |a: f32, b: f32| ((a.min(b) - reach) / TILE as f32).floor();
        let hi = |a: f32, b: f32| ((a.max(b) + reach) / TILE as f32).floor();
        let x0 = lo(s[0], s[2]).max(0.0) as u32;
        let y0 = lo(s[1], s[3]).max(0.0) as u32;
        let x1 = hi(s[0], s[2]).min(cols as f32 - 1.0);
        let y1 = hi(s[1], s[3]).min(rows as f32 - 1.0);
        if x1 < 0.0 || y1 < 0.0 {
            return None;
        }
        let (x1, y1) = (x1 as u32, y1 as u32);
        (x0 <= x1 && y0 <= y1).then_some((x0, y0, x1, y1))
    };
    let mut count = vec![0u32; tiles + 1];
    let mut filed = 0usize;
    let mut last = p.segs.len();
    for (k, s) in p.segs.iter().enumerate() {
        let Some((x0, y0, x1, y1)) = span(s) else {
            continue;
        };
        let n = ((x1 - x0 + 1) * (y1 - y0 + 1)) as usize;
        if filed + n > MAX_BIN_ITEMS {
            last = k;
            break;
        }
        filed += n;
        for r in y0..=y1 {
            for c in x0..=x1 {
                count[(r * cols + c) as usize + 1] += 1;
            }
        }
    }
    for i in 0..tiles {
        count[i + 1] += count[i];
    }
    let mut items = vec![0u32; filed];
    let mut next = count.clone();
    for (k, s) in p.segs.iter().enumerate().take(last) {
        let Some((x0, y0, x1, y1)) = span(s) else {
            continue;
        };
        for r in y0..=y1 {
            for c in x0..=x1 {
                let t = (r * cols + c) as usize;
                items[next[t] as usize] = k as u32;
                next[t] += 1;
            }
        }
    }
    Bins {
        cols,
        rows,
        start: count,
        items,
    }
}
