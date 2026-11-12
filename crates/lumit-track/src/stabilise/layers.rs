//! Which layer of the picture each step's motion is measured from.
//!
//! # In plain terms
//!
//! Between two frames the specks often move two or three ways at once: the
//! background one way, a person walking through it another. Each way is found
//! in turn, as the largest set of specks one slide carries, grown by a slide,
//! turn and growth refitted to them until it carries no more, and is then set
//! aside so the search can run again on what is left. A slide to start from,
//! because anything freer can bend: a four-corner fit moves the middle of the
//! frame one way while its edges go another, and even a turn about one edge
//! carries the still background there and a person moving in the middle as
//! one. Which of them is the picture is
//! decided over the whole shot rather than frame by frame. A layer scores for
//! how much of the frame it covers, the edges most, since the background
//! reaches them and a subject rarely does, and for sharing specks with the
//! layer chosen on the step before. The best path through the shot is kept, so
//! the choice neither flips from frame to frame nor stays wrong because the
//! first frame was.

use lumit_core::stabilise::analysis::MIN_KEPT;
use lumit_core::stabilise::motion::AGREE_TRACKED_PX;

use crate::Correspondence;

/// The most ways of moving one step is split into.
pub(super) const MOST: usize = 3;

/// The coarse grid a layer's cover is counted on, cells across and down.
const GRID: (usize, usize) = (8, 6);

/// Specks in one cell past which the cell counts no more, so one textured
/// patch cannot outvote the frame.
const CAP: usize = 3;

/// How much more a cell on the frame's edge counts than one inside it.
const EDGE: f64 = 2.0;

/// What sharing every speck with the layer chosen on the step before is
/// worth, against a layer covering the whole frame, which is worth one.
const SHARE: f64 = 0.5;

/// The most specks whose slide the search for one layer tries.
const DRAWS: usize = 100;

/// The most times a layer's slide, turn and growth is refitted to what it
/// carries.
const GROWTHS: usize = 10;

/// What two layers count as sharing where too few specks live across both
/// steps to say, and on either side of a step with no layer.
const UNSURE: f64 = 0.5;

/// Fewer specks than this alive across two steps say nothing about whether
/// two layers are one.
const MIN_ACROSS: usize = 4;

/// One way the specks of a step move.
pub(super) struct Layer {
    /// The specks moving this way, by id, ascending.
    pub(super) members: Vec<u32>,
    /// How much of the frame they cover, nought to one.
    pub(super) cover: f64,
}

/// The ways the pairs `pts` of the step out of frame `from` move, largest
/// first, on a raster of `size`. Each is the largest set of the pairs no
/// earlier one took that one similarity carries.
pub(super) fn split(pts: &[Correspondence], size: (usize, usize), from: i64) -> Vec<Layer> {
    let mut out: Vec<Layer> = Vec::new();
    let mut left: Vec<Correspondence> = pts.to_vec();
    while out.len() < MOST && left.len() >= MIN_KEPT as usize {
        let seed = (from as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (left.len() as u64);
        let found = consensus(&left, seed);
        if found.len() < MIN_KEPT as usize {
            break;
        }
        // `left` is in id order, as every set of correspondences is, and
        // `found` ascends, so the members do too.
        let members: Vec<u32> = found
            .iter()
            .filter_map(|&i| left.get(i).map(|c| c.id))
            .collect();
        left.retain(|c| members.binary_search(&c.id).is_err());
        out.push(Layer {
            cover: cover(pts, &members, size),
            members,
        });
    }
    out
}

/// A similarity as a complex scale and turn `(a, b)` and a slide `t`:
/// `x' = a·x − b·y + tx`, `y' = b·x + a·y + ty`.
type Similar = ([f64; 2], [f64; 2]);

/// How far `s` misses taking `c.from` to `c.to`, squared.
fn miss(s: &Similar, c: &Correspondence) -> f64 {
    let ([a, b], [tx, ty]) = *s;
    let (x, y) = (c.from[0], c.from[1]);
    let (dx, dy) = (a * x - b * y + tx - c.to[0], b * x + a * y + ty - c.to[1]);
    dx * dx + dy * dy
}

/// The similarity with its scale and turn `(a, b)` that takes `p` (a pair's
/// start) to `q` (its end).
fn anchored(a: f64, b: f64, p: [f64; 2], q: [f64; 2]) -> Similar {
    (
        [a, b],
        [q[0] - (a * p[0] - b * p[1]), q[1] - (b * p[0] + a * p[1])],
    )
}

/// The least-squares similarity over the pairs of `pts` that `which` names.
fn least_squares(pts: &[Correspondence], which: &[usize]) -> Option<Similar> {
    if which.len() < 2 {
        return None;
    }
    let n = which.len() as f64;
    let (mut p, mut q) = ([0.0f64; 2], [0.0f64; 2]);
    for c in which.iter().filter_map(|&i| pts.get(i)) {
        p = [p[0] + c.from[0] / n, p[1] + c.from[1] / n];
        q = [q[0] + c.to[0] / n, q[1] + c.to[1] / n];
    }
    let (mut a, mut b, mut d) = (0.0f64, 0.0f64, 0.0f64);
    for c in which.iter().filter_map(|&i| pts.get(i)) {
        let (ux, uy) = (c.from[0] - p[0], c.from[1] - p[1]);
        let (vx, vy) = (c.to[0] - q[0], c.to[1] - q[1]);
        a += vx * ux + vy * uy;
        b += vy * ux - vx * uy;
        d += ux * ux + uy * uy;
    }
    if d.is_nan() || d < 1e-9 {
        return None;
    }
    Some(anchored(a / d, b / d, p, q))
}

/// The indices of the pairs `s` carries within [`AGREE_TRACKED_PX`], ascending.
fn carried(pts: &[Correspondence], s: &Similar) -> Vec<usize> {
    let limit = AGREE_TRACKED_PX * AGREE_TRACKED_PX;
    pts.iter()
        .enumerate()
        .filter(|(_, c)| miss(s, c) < limit)
        .map(|(i, _)| i)
        .collect()
}

/// The largest set of `pts` one similarity carries, by index, ascending: the
/// best of [`DRAWS`] slides, each the move of one pair drawn from a fixed
/// sequence seeded by `seed`, then a similarity refitted to what it carries
/// while that grows.
fn consensus(pts: &[Correspondence], seed: u64) -> Vec<usize> {
    let n = pts.len() as u64;
    if n < 2 {
        return Vec::new();
    }
    let mut state = seed;
    let mut draw = || {
        // SplitMix64: a fixed sequence, so the same pairs give the same layers.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n) as usize
    };
    let limit = AGREE_TRACKED_PX * AGREE_TRACKED_PX;
    let mut best: Option<Similar> = None;
    let mut most = 0usize;
    for _ in 0..DRAWS {
        let Some(c) = pts.get(draw()) else {
            continue;
        };
        let s: Similar = ([1.0, 0.0], [c.to[0] - c.from[0], c.to[1] - c.from[1]]);
        let count = pts.iter().filter(|c| miss(&s, c) < limit).count();
        if count > most {
            most = count;
            best = Some(s);
        }
    }
    let Some(s) = best else {
        return Vec::new();
    };
    let mut found = carried(pts, &s);
    for _ in 0..GROWTHS {
        let Some(s) = least_squares(pts, &found) else {
            break;
        };
        let again = carried(pts, &s);
        if again.len() < found.len() {
            break;
        }
        let done = again.len() == found.len();
        found = again;
        if done {
            break;
        }
    }
    found
}

/// How much of a `size` frame the pairs of `pts` named in `members` cover,
/// where they start: occupied cells of a coarse grid, each counting up to
/// [`CAP`] specks, the edge cells [`EDGE`] times over.
fn cover(pts: &[Correspondence], members: &[u32], size: (usize, usize)) -> f64 {
    let (gx, gy) = GRID;
    let mut cells = [0usize; GRID.0 * GRID.1];
    let (w, h) = (size.0.max(1) as f64, size.1.max(1) as f64);
    for c in pts.iter().filter(|c| members.binary_search(&c.id).is_ok()) {
        let x = (c.from[0] / w * gx as f64).floor();
        let y = (c.from[1] / h * gy as f64).floor();
        if !(x >= 0.0 && y >= 0.0 && x < gx as f64 && y < gy as f64) {
            continue;
        }
        if let Some(n) = cells.get_mut(y as usize * gx + x as usize) {
            *n += 1;
        }
    }
    let (mut got, mut all) = (0.0, 0.0);
    for (i, &n) in cells.iter().enumerate() {
        let (x, y) = (i % gx, i / gx);
        let weight = if x == 0 || y == 0 || x + 1 == gx || y + 1 == gy {
            EDGE
        } else {
            1.0
        };
        got += weight * n.min(CAP) as f64 / CAP as f64;
        all += weight;
    }
    got / all
}

/// How far two layers of neighbouring steps are one: of the specks alive
/// across both steps, the share of the smaller layer's that the other holds
/// too, so a layer that splits off a larger one, or joins it, still counts as
/// the same. `now` is a layer of the later step and `then` one of the earlier;
/// `seen_now` and `seen_then` are the ids each step saw, ascending. `None`
/// where too few live across both to say.
pub(super) fn shared(
    now: &[u32],
    then: &[u32],
    seen_now: &[u32],
    seen_then: &[u32],
) -> Option<f64> {
    let count = |x: &[u32], y: &[u32]| x.iter().filter(|id| y.binary_search(id).is_ok()).count();
    let smaller = count(now, seen_then).min(count(then, seen_now));
    (smaller >= MIN_ACROSS).then(|| count(now, then) as f64 / smaller as f64)
}

/// The layer to measure each step from: the path through the shot that covers
/// the most frame and changes layer least, as a dynamic programme over the
/// steps. `layers[t]` are step `t`'s ways of moving and `share[t][i][j]` how
/// far its way `i` is the step before's way `j`. `None` only for a step with
/// no layer at all. `None` overall when `stop` said so.
pub(super) fn choose(
    layers: &[Vec<Layer>],
    share: &[[[Option<f64>; MOST]; MOST]],
    stop: &dyn Fn() -> bool,
) -> Option<Vec<Option<usize>>> {
    // The best score of a path ending in each of the step's layers, or in
    // nothing for a step that had none.
    let mut score: Vec<f64> = vec![0.0];
    let mut back: Vec<[u8; MOST]> = Vec::with_capacity(layers.len());
    for (t, here) in layers.iter().enumerate() {
        if stop() {
            return None;
        }
        let n = here.len().min(MOST);
        let mut next = vec![0.0f64; n.max(1)];
        let mut from = [0u8; MOST];
        let before = score.len();
        for (i, slot) in next.iter_mut().enumerate() {
            let node = here.get(i).filter(|_| i < n).map_or(0.0, |l| l.cover);
            let mut best = f64::NEG_INFINITY;
            for (j, &s) in score.iter().enumerate() {
                let link = share
                    .get(t)
                    .and_then(|m| m.get(i)?.get(j).copied().flatten())
                    .filter(|_| i < n && j < before)
                    .unwrap_or(UNSURE);
                // Strictly better only, so a tie keeps the larger layer.
                let v = s + SHARE * link;
                if v > best {
                    best = v;
                    from[i] = j as u8;
                }
            }
            *slot = node + if t == 0 { 0.0 } else { best };
        }
        back.push(from);
        score = next;
    }
    let mut at = 0usize;
    for (i, &s) in score.iter().enumerate() {
        if score.get(at).is_some_and(|&b| s > b) {
            at = i;
        }
    }
    let mut out = vec![None; layers.len()];
    for (t, here) in layers.iter().enumerate().rev() {
        out[t] = (at < here.len().min(MOST)).then_some(at);
        at = back.get(t).map_or(0, |b| usize::from(b[at.min(MOST - 1)]));
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// One step of a 960 by 540 raster: forty specks of background round the
    /// frame's edges sliding one way, and a hundred and twenty on a subject in
    /// the middle, reaching the bottom edge, moving its own way.
    fn step(t: i64) -> Vec<Correspondence> {
        let mut out = Vec::new();
        let bg = [1.0 + 0.3 * (t as f64).sin(), 0.5];
        let subject = [12.0 * (0.9 * t as f64).cos(), -8.0];
        for i in 0..40u32 {
            let k = f64::from(i);
            let p = match i % 4 {
                0 => [10.0 + k, 20.0 + 12.0 * k],
                1 => [930.0 - k, 20.0 + 12.0 * k],
                2 => [30.0 + 22.0 * k, 12.0],
                _ => [30.0 + 22.0 * k, 520.0 - (k % 3.0)],
            };
            out.push(Correspondence {
                id: i,
                from: p,
                to: [p[0] + bg[0], p[1] + bg[1]],
            });
        }
        for i in 0..120u32 {
            let p = [
                400.0 + f64::from(i % 12) * 14.0,
                150.0 + f64::from(i / 12) * 39.0,
            ];
            out.push(Correspondence {
                id: 100 + i,
                from: p,
                to: [p[0] + subject[0], p[1] + subject[1]],
            });
        }
        out
    }

    #[test]
    fn the_background_is_followed_over_a_larger_subject() {
        let size = (960, 540);
        let mut found = Vec::new();
        let mut share = Vec::new();
        let mut then: Vec<u32> = Vec::new();
        for t in 0..8 {
            let pts = step(t);
            let here = split(&pts, size, t);
            let seen: Vec<u32> = pts.iter().map(|c| c.id).collect();
            let mut links = [[None; MOST]; MOST];
            if let Some(before) = found.last() {
                let before: &Vec<Layer> = before;
                for (a, now) in here.iter().enumerate() {
                    for (b, was) in before.iter().enumerate() {
                        links[a][b] = shared(&now.members, &was.members, &seen, &then);
                    }
                }
            }
            found.push(here);
            share.push(links);
            then = seen;
        }
        let chosen = choose(&found, &share, &|| false).unwrap();
        for (t, (layers, pick)) in found.iter().zip(&chosen).enumerate() {
            let members = &layers[pick.unwrap()].members;
            assert!(
                members.len() >= 36 && members.iter().all(|&id| id < 100),
                "step {t}: {members:?}"
            );
        }
    }
}
