//! The coarse search: where two onset curves meet, to within a few frames.
//!
//! The shorter curve is cut into 30 s pieces and each is slid along the whole
//! of the longer one by FFT. A piece is short enough that even a fast drifting
//! clock moves it only a few milliseconds, so its peak stays sharp; a whole
//! hour slid at once would smear by a third of a second. The pieces that
//! agree on one straight line of offsets are the match, and the line's slope
//! is the drift.

use realfft::RealFftPlanner;

use super::{Onsets, SyncError, FPS};

const PIECE: usize = 30 * FPS;
const PIECE_HOP: usize = 15 * FPS;
/// A peak needs this much overlap, so a sliver at an edge cannot win by luck.
const MIN_OVERLAP: usize = 10 * FPS;
/// Peaks kept per piece, at least a second apart.
const PEAKS: usize = 3;
const PEAK_GAP: usize = FPS;
/// How far from a line a piece's peak may be and still agree with it.
const TOLERANCE: f64 = 0.015;
/// Real clocks drift well under a thousandth; a steeper line is a fluke.
const MAX_SLOPE: f64 = 1e-3;
/// Lines tried through two peaks, besides a level line through each piece.
const TRIES: usize = 2000;
/// Lags sampled for a piece's median and spread: plenty for a steady figure.
const Z_SAMPLES: usize = 20_000;
const SYNCED_Z: f64 = 10.0;
const SYNCED_R: f64 = 0.5;
const SYNCED_RIVAL: f64 = 0.5;
const UNCERTAIN_Z: f64 = 6.0;

/// Offsets along a pair: the long input's time minus the short one's, at
/// short time `t`, is `a + b * t` seconds.
#[derive(Debug, Clone, Copy)]
pub(super) struct Line {
    pub(super) a: f64,
    pub(super) b: f64,
}

impl Line {
    pub(super) fn at(&self, t: f64) -> f64 {
        self.a + self.b * t
    }
}

/// A pair the coarse search accepts.
pub(super) struct Found {
    pub(super) line: Line,
    /// Median standard score of the agreeing peaks.
    pub(super) z: f64,
    /// Share of the pieces that could agree and did.
    pub(super) r: f64,
    pub(super) synced: bool,
    /// Short-input times, the middles of the agreeing pieces.
    pub(super) times: Vec<f64>,
}

/// One peak of one piece.
struct Point {
    piece: usize,
    t: f64,
    offset: f64,
    ncc: f32,
    z: f32,
}

/// Search where `short` sits in `long`. None when they share no sound.
pub(super) fn search(
    planner: &mut RealFftPlanner<f32>,
    short: &Onsets,
    long: &Onsets,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Found>, SyncError> {
    let (s, l) = (&short.feature, &long.feature);
    let piece_len = PIECE.min(s.len());
    let n = l.len();
    if piece_len == 0 || n < piece_len {
        return Ok(None);
    }
    let min_overlap = MIN_OVERLAP.min(piece_len);
    // Lag k puts piece frame j against long frame k + j. Every lag with
    // enough overlap is scored, so a short input may start before the long.
    let first = min_overlap as i64 - piece_len as i64;
    let lags = n - min_overlap + piece_len - min_overlap + 1;
    let overlap = |i: usize| {
        let k = first + i as i64;
        let lo = (-k).max(0) as usize;
        let hi = (n as i64 - k).min(piece_len as i64).max(0) as usize;
        (lo, hi)
    };
    // Lags are scored a block at a time (overlap-save), so the FFTs stay
    // small and in cache however long the long input is. Each block of the
    // long curve is transformed once for every piece.
    let block = (4 * piece_len).next_power_of_two();
    let valid = block - piece_len + 1;
    let fwd = planner.plan_fft_forward(block);
    let inv = planner.plan_fft_inverse(block);
    let mut buf = fwd.make_input_vec();
    let mut scratch = fwd.make_scratch_vec();
    let mut inv_scratch = inv.make_scratch_vec();
    let mut spec = fwd.make_output_vec();
    let mut piece_spec = fwd.make_output_vec();
    let mut corr = inv.make_output_vec();
    let mut long_specs = Vec::with_capacity(lags.div_ceil(valid));
    for at in (0..lags).step_by(valid) {
        if cancelled() {
            return Err(SyncError::Cancelled);
        }
        let q = first + at as i64;
        for (j, b) in buf.iter_mut().enumerate() {
            let k = q + j as i64;
            *b = usize::try_from(k)
                .ok()
                .and_then(|k| l.get(k))
                .copied()
                .unwrap_or(0.0);
        }
        let mut out = fwd.make_output_vec();
        fwd.process_with_scratch(&mut buf, &mut out, &mut scratch)?;
        long_specs.push(out);
    }
    // What every piece's score is divided by at each lag, bar the piece's
    // own energy: the long curve's energy under it, and the FFT's gain. A
    // short overlap scores wider by chance alone, so it is also scaled back
    // to the spread of a full one before peaks compete.
    let long_prefix = prefix_squares(l);
    let scale: Vec<f32> = (0..lags)
        .map(|i| {
            let (lo, hi) = overlap(i);
            let k = first + i as i64;
            let at = |j: usize| long_prefix.get((k + j as i64) as usize).copied();
            let energy = match (at(hi), at(lo)) {
                (Some(h), Some(l)) => (h - l) * piece_len as f64 / (hi - lo).max(1) as f64,
                _ => 0.0,
            };
            if energy > 1e-12 {
                (1.0 / (block as f64 * energy.sqrt())) as f32
            } else {
                0.0
            }
        })
        .collect();
    // Lags hanging off either end of the long curve see only part of the piece.
    let partial: Vec<usize> = (0..piece_len - min_overlap)
        .chain(n.saturating_sub(min_overlap).max(piece_len - min_overlap)..lags)
        .collect();

    let mut starts: Vec<usize> = (0..)
        .step_by(PIECE_HOP)
        .take_while(|p| p + piece_len <= s.len())
        .collect();
    if starts.last().is_some_and(|p| p + piece_len < s.len()) {
        starts.push(s.len() - piece_len);
    }
    let mut points = Vec::new();
    let mut live = Vec::new();
    let mut ncc = vec![0f32; lags];
    for (index, &p) in starts.iter().enumerate() {
        if cancelled() {
            return Err(SyncError::Cancelled);
        }
        let Some(piece) = s.get(p..p + piece_len) else {
            continue;
        };
        // A mostly silent piece has nothing to say.
        if piece.iter().filter(|v| **v != 0.0).count() * 2 < piece_len {
            continue;
        }
        live.push(p);
        buf.fill(0.0);
        for (b, v) in buf.iter_mut().zip(piece) {
            *b = *v;
        }
        fwd.process_with_scratch(&mut buf, &mut piece_spec, &mut scratch)?;
        let piece_prefix = prefix_squares(piece);
        let full = piece_prefix.last().copied().unwrap_or(0.0);
        let inv_full = (1.0 / full.sqrt()) as f32;
        for (at, long_spec) in (0..lags).step_by(valid).zip(&long_specs) {
            for ((x, y), z) in spec.iter_mut().zip(long_spec).zip(&piece_spec) {
                *x = y * z.conj();
            }
            // A real signal's two end bins are real; rounding must not say else.
            if let Some(c) = spec.first_mut() {
                c.im = 0.0;
            }
            if let Some(c) = spec.last_mut() {
                c.im = 0.0;
            }
            inv.process_with_scratch(&mut spec, &mut corr, &mut inv_scratch)?;
            let out = ncc.get_mut(at..).unwrap_or_default();
            let sc = scale.get(at..).unwrap_or_default();
            for ((o, r), sc) in out.iter_mut().zip(corr.iter().take(valid)).zip(sc) {
                *o = r * sc * inv_full;
            }
        }
        // Normalise over the overlap only, so a lag hanging off an end is
        // not marked down for loudness in the part that is missing.
        for &i in &partial {
            let (lo, hi) = overlap(i);
            let pe = match (piece_prefix.get(hi), piece_prefix.get(lo)) {
                (Some(h), Some(l)) => h - l,
                _ => 0.0,
            };
            if let Some(o) = ncc.get_mut(i) {
                *o = if pe > 1e-12 {
                    *o * (full / pe).sqrt() as f32
                } else {
                    0.0
                };
            }
        }
        let t = (p as f64 + piece_len as f64 / 2.0) / FPS as f64;
        for (i, value, z) in peaks(&ncc) {
            points.push(Point {
                piece: index,
                t,
                offset: (first + i as i64 - p as i64) as f64 / FPS as f64,
                ncc: value,
                z,
            });
        }
    }
    if points.is_empty() {
        return Ok(None);
    }

    let tries = tries(&points);
    let everything = vec![true; points.len()];
    let (mut line, mut weight) = best_line(&points, &everything, &tries, cancelled)?;
    if let Some(fit) = refit(&points, &agreeing(&points, &everything, line)) {
        let w = score(&points, &everything, fit);
        if w >= weight {
            (line, weight) = (fit, w);
        }
    }
    if weight <= 0.0 {
        return Ok(None);
    }
    let chosen = agreeing(&points, &everything, line);
    // The best line among what the winner leaves: repeated sound shows here.
    let others: Vec<bool> = points
        .iter()
        .map(|p| (p.offset - line.at(p.t)).abs() > TOLERANCE)
        .collect();
    let (_, rival) = best_line(&points, &others, &tries, cancelled)?;

    let inside = live
        .iter()
        .filter(|&&p| {
            let t = (p as f64 + piece_len as f64 / 2.0) / FPS as f64;
            let k = p as f64 + line.at(t) * FPS as f64;
            k >= 0.0 && k + piece_len as f64 <= n as f64
        })
        .count();
    // Three agreeing pieces, or every piece of a short input. Fewer could
    // be one chance peak seen twice, as neighbouring pieces share half.
    if chosen.len() < live.len().min(3) {
        return Ok(None);
    }
    let expected = inside.max(chosen.len()).max(1);
    let r = chosen.len() as f64 / expected as f64;
    let mut zs: Vec<f32> = chosen
        .iter()
        .filter_map(|&i| points.get(i).map(|p| p.z))
        .collect();
    let z = f64::from(median(&mut zs));
    let synced = z >= SYNCED_Z && r >= SYNCED_R && rival / weight <= SYNCED_RIVAL;
    if !synced && z < UNCERTAIN_Z {
        return Ok(None);
    }
    let times = chosen
        .iter()
        .filter_map(|&i| points.get(i).map(|p| p.t))
        .collect();
    Ok(Some(Found {
        line,
        z,
        r,
        synced,
        times,
    }))
}

/// Running sums of squares, one longer than `x`, in f64.
fn prefix_squares(x: &[f32]) -> Vec<f64> {
    let mut out = Vec::with_capacity(x.len() + 1);
    let mut sum = 0f64;
    out.push(sum);
    for v in x {
        sum += f64::from(*v) * f64::from(*v);
        out.push(sum);
    }
    out
}

/// The median of `x`, reordering it. Nought when empty.
fn median(x: &mut [f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    let at = x.len() / 2;
    *x.select_nth_unstable_by(at, f32::total_cmp).1
}

/// The best few peaks of one piece's scores, a second apart, each with its
/// standard score against the piece's median and spread.
fn peaks(ncc: &[f32]) -> Vec<(usize, f32, f32)> {
    let stride = (ncc.len() / Z_SAMPLES).max(1);
    let mut sample: Vec<f32> = ncc.iter().step_by(stride).copied().collect();
    let mid = median(&mut sample);
    let mut deviation: Vec<f32> = sample.iter().map(|v| (v - mid).abs()).collect();
    let spread = 1.4826 * median(&mut deviation);
    let z = |v: f32| {
        if spread > 0.0 {
            (v - mid) / spread
        } else {
            0.0
        }
    };
    // The best of each second, then the best of those a second apart.
    let mut best: Vec<(usize, f32)> = ncc
        .chunks(PEAK_GAP)
        .map(|chunk| chunk.iter().copied().fold(f32::NEG_INFINITY, f32::max))
        .enumerate()
        .collect();
    best.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut picked: Vec<(usize, f32, f32)> = Vec::new();
    for (c, v) in best {
        if picked.len() == PEAKS {
            break;
        }
        // Only the few chunks looked at need their peak found.
        let at = c * PEAK_GAP;
        let Some(i) = ncc
            .get(at..)
            .and_then(|rest| rest.iter().take(PEAK_GAP).position(|x| *x == v))
            .map(|i| at + i)
        else {
            continue;
        };
        if picked.iter().all(|(j, _, _)| i.abs_diff(*j) >= PEAK_GAP) {
            picked.push((i, v, z(v)));
        }
    }
    picked
}

/// The lines to try: level through each piece's best peak, and through
/// pairs of peaks drawn by a fixed sequence, so every run tries the same.
fn tries(points: &[Point]) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let mut last_piece = None;
    for p in points {
        if last_piece != Some(p.piece) {
            out.push(Line {
                a: p.offset,
                b: 0.0,
            });
            last_piece = Some(p.piece);
        }
    }
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut draw = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % points.len().max(1)
    };
    for _ in 0..TRIES {
        let (Some(p), Some(q)) = (points.get(draw()), points.get(draw())) else {
            continue;
        };
        if p.piece == q.piece {
            continue;
        }
        let b = (q.offset - p.offset) / (q.t - p.t);
        if b.abs() <= MAX_SLOPE {
            out.push(Line {
                a: p.offset - b * p.t,
                b,
            });
        }
    }
    out
}

/// Indices of the peaks in `mask` that agree with `line`. Peaks of one
/// piece are a second apart, so at most one of each can.
fn agreeing(points: &[Point], mask: &[bool], line: Line) -> Vec<usize> {
    points
        .iter()
        .zip(mask)
        .enumerate()
        .filter(|(_, (p, m))| **m && (p.offset - line.at(p.t)).abs() <= TOLERANCE)
        .map(|(i, _)| i)
        .collect()
}

/// How strongly the peaks in `mask` back `line`: their summed scores.
fn score(points: &[Point], mask: &[bool], line: Line) -> f64 {
    agreeing(points, mask, line)
        .iter()
        .filter_map(|&i| points.get(i))
        .map(|p| f64::from(p.ncc.max(0.0)))
        .sum()
}

/// The best backed of `tries`, first found winning a tie.
fn best_line(
    points: &[Point],
    mask: &[bool],
    tries: &[Line],
    cancelled: &dyn Fn() -> bool,
) -> Result<(Line, f64), SyncError> {
    let mut best = (Line { a: 0.0, b: 0.0 }, 0.0);
    for (n, line) in tries.iter().enumerate() {
        if n % 256 == 0 && cancelled() {
            return Err(SyncError::Cancelled);
        }
        let w = score(points, mask, *line);
        if w > best.1 {
            best = (*line, w);
        }
    }
    Ok(best)
}

/// The least squares line through the chosen peaks, when it is a real
/// clock's slope.
fn refit(points: &[Point], chosen: &[usize]) -> Option<Line> {
    let picked: Vec<&Point> = chosen.iter().filter_map(|&i| points.get(i)).collect();
    if picked.len() < 2 {
        return None;
    }
    let n = picked.len() as f64;
    let mt = picked.iter().map(|p| p.t).sum::<f64>() / n;
    let mo = picked.iter().map(|p| p.offset).sum::<f64>() / n;
    let var: f64 = picked.iter().map(|p| (p.t - mt).powi(2)).sum();
    let cov: f64 = picked.iter().map(|p| (p.t - mt) * (p.offset - mo)).sum();
    if var <= 0.0 {
        return None;
    }
    let b = cov / var;
    (b.abs() <= MAX_SLOPE).then_some(Line { a: mo - b * mt, b })
}
