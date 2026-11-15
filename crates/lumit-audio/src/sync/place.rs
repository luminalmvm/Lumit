//! Joining the pairs into one placement.
//!
//! The strongest pairs that join every input without a loop (a maximum
//! spanning tree) set each input's speed against the longest input it meets.
//! The starts then come from weighted least squares over every pair, which
//! is what puts A and C right when they only meet through B. A pair that
//! disagrees with the rest by more than 20 ms is dropped, weakest first.

use std::collections::BTreeMap;

use super::{Edge, Input, Placed, State};

/// More disagreement than this between pairs, or overlap within one device,
/// is a conflict.
const CONFLICT: f64 = 0.020;

/// Place `inputs` from the accepted `edges`.
pub(super) fn place(inputs: &[Input], edges: &[Edge]) -> Vec<Placed> {
    let n = inputs.len();
    let seconds: Vec<f64> = inputs.iter().map(|i| i.onsets.seconds).collect();
    let mut out = vec![
        Placed {
            start: None,
            speed: 1.0,
            state: State::Unmatched,
        };
        n
    ];
    let edges: Vec<&Edge> = edges.iter().filter(|e| e.short < n && e.long < n).collect();

    // Pairs with measured drift first, then strongest first; a stable sort
    // keeps ties in the order found.
    let mut strongest = edges.clone();
    strongest.sort_by(|x, y| {
        (y.b != 0.0)
            .cmp(&(x.b != 0.0))
            .then(y.weight.total_cmp(&x.weight))
    });
    let mut root: Vec<usize> = (0..n).collect();
    let mut tree: Vec<Vec<(usize, Option<f64>)>> = vec![Vec::new(); n];
    for e in &strongest {
        let (ru, rv) = (find(&mut root, e.short), find(&mut root, e.long));
        if ru == rv {
            continue;
        }
        root[ru] = rv;
        // The long clock runs 1 + b times the short one's. An unmeasured
        // drift says nothing, so that input keeps its nominal speed.
        let measured = e.b != 0.0;
        tree[e.short].push((e.long, measured.then_some(1.0 + e.b)));
        tree[e.long].push((e.short, measured.then_some(1.0 / (1.0 + e.b))));
    }
    let scene: Vec<usize> = (0..n).map(|i| find(&mut root, i)).collect();

    // Each scene's reference is its longest steady input, else its longest.
    let mut anchors: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, s) in scene.iter().enumerate() {
        let a = anchors.entry(*s).or_insert(i);
        let (steady, was) = (inputs[i].steady, inputs[*a].steady);
        if (steady && !was) || (steady == was && seconds[i] > seconds[*a]) {
            *a = i;
        }
    }
    let mut speed = vec![1.0; n];
    let mut seen = vec![false; n];
    for &a in anchors.values() {
        let mut stack = vec![a];
        seen[a] = true;
        while let Some(u) = stack.pop() {
            for &(v, factor) in &tree[u] {
                if !seen[v] {
                    seen[v] = true;
                    speed[v] = factor.map_or(1.0, |f| speed[u] * f);
                    stack.push(v);
                }
            }
        }
    }

    let fixed: Vec<bool> = (0..n).map(|i| anchors.values().any(|a| *a == i)).collect();
    // Where the pair was measured, both inputs' times land on one moment.
    let target = |e: &Edge| (e.mid + e.offset) / speed[e.long] - e.mid / speed[e.short];
    let mut kept = vec![true; edges.len()];
    let mut doubtful = vec![false; n];
    let start = loop {
        let start = least_squares(&edges, &kept, &fixed, &target);
        let residual = |e: &Edge| (start[e.short] - start[e.long] - target(e)).abs();
        let worst = edges
            .iter()
            .enumerate()
            .filter(|(i, e)| kept[*i] && residual(e) > CONFLICT)
            .min_by(|x, y| x.1.weight.total_cmp(&y.1.weight));
        let Some((i, e)) = worst else {
            break start;
        };
        kept[i] = false;
        doubtful[e.short] = true;
        doubtful[e.long] = true;
    };

    // Each scene starts where the one before it ended, in input order.
    let mut matched = vec![false; n];
    let mut strength = vec![0f64; n];
    let mut synced = vec![false; n];
    for (e, _) in edges.iter().zip(&kept).filter(|(_, k)| **k) {
        for i in [e.short, e.long] {
            matched[i] = true;
            strength[i] = strength[i].max(e.weight);
            synced[i] |= e.synced;
        }
    }
    let end = |i: usize, s: f64| s + seconds[i] / speed[i];
    let mut cursor = 0.0;
    let mut laid: Vec<usize> = Vec::new();
    for i in 0..n {
        if !matched[i] || laid.contains(&scene[i]) {
            continue;
        }
        laid.push(scene[i]);
        let members: Vec<usize> = (0..n).filter(|&m| scene[m] == scene[i]).collect();
        let lo = members
            .iter()
            .map(|&m| start[m])
            .fold(f64::INFINITY, f64::min);
        let mut hi = cursor;
        for &m in &members {
            let s = cursor + start[m] - lo;
            hi = hi.max(end(m, s));
            out[m] = Placed {
                start: Some(s),
                speed: speed[m],
                state: if synced[m] && !doubtful[m] {
                    State::Synced
                } else {
                    State::Uncertain
                },
            };
        }
        cursor = hi;
    }

    // One device's inputs never overlap and keep their order: where two
    // do not, the weaker is in doubt.
    let mut groups: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, input) in inputs.iter().enumerate() {
        if out[i].start.is_some() {
            groups.entry(input.group).or_default().push(i);
        }
    }
    for members in groups.values_mut() {
        members.sort_by_key(|&i| (inputs[i].order, i));
        let at = |i: usize| out[i].start.unwrap_or(0.0);
        let mut doubted = Vec::new();
        let mut keep = members.first().copied();
        for &cur in members.iter().skip(1) {
            let Some(k) = keep else { break };
            if at(cur) + CONFLICT < end(k, at(k)) {
                if strength[cur] > strength[k] {
                    doubted.push(k);
                    keep = Some(cur);
                } else {
                    doubted.push(cur);
                }
            } else {
                keep = Some(cur);
            }
        }
        for i in doubted {
            out[i].state = State::Uncertain;
        }
    }
    out
}

/// The root of `i`'s set, halving the path on the way.
fn find(root: &mut [usize], mut i: usize) -> usize {
    while root[i] != i {
        root[i] = root[root[i]];
        i = root[i];
    }
    i
}

/// Starts that best meet every kept edge, `start[short] - start[long] =
/// target`, weighted by edge strength, with each fixed input at 0.
fn least_squares(
    edges: &[&Edge],
    kept: &[bool],
    fixed: &[bool],
    target: &dyn Fn(&Edge) -> f64,
) -> Vec<f64> {
    let n = fixed.len();
    let mut a = vec![0f64; n * n];
    let mut rhs = vec![0f64; n];
    for (e, _) in edges.iter().zip(kept).filter(|(_, k)| **k) {
        let (u, v, w, c) = (e.short, e.long, e.weight, target(e));
        a[u * n + u] += w;
        a[v * n + v] += w;
        a[u * n + v] -= w;
        a[v * n + u] -= w;
        rhs[u] += w * c;
        rhs[v] -= w * c;
    }
    for (i, _) in fixed.iter().enumerate().filter(|(_, f)| **f) {
        a[i * n..(i + 1) * n].fill(0.0);
        a[i * n + i] = 1.0;
        rhs[i] = 0.0;
    }
    // Gaussian elimination with partial pivoting; n is the number of files.
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&x, &y| a[x * n + col].abs().total_cmp(&a[y * n + col].abs()))
            .unwrap_or(col);
        if a[pivot * n + col].abs() < 1e-12 {
            continue;
        }
        if pivot != col {
            for j in 0..n {
                a.swap(col * n + j, pivot * n + j);
            }
            rhs.swap(col, pivot);
        }
        for row in col + 1..n {
            let f = a[row * n + col] / a[col * n + col];
            if f != 0.0 {
                for j in col..n {
                    a[row * n + j] -= f * a[col * n + j];
                }
                rhs[row] -= f * rhs[col];
            }
        }
    }
    let mut x = vec![0f64; n];
    for row in (0..n).rev() {
        let d = a[row * n + row];
        if d.abs() < 1e-12 {
            continue;
        }
        let sum: f64 = (row + 1..n).map(|j| a[row * n + j] * x[j]).sum();
        x[row] = (rhs[row] - sum) / d;
    }
    x
}
