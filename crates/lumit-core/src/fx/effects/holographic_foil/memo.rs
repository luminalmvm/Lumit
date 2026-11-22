//! A few recent results of the packer's slow, pure parts (the spectral sums,
//! the band tables and the renormalisation), kept by the exact bits of what
//! they read, so a frame whose tint, spectra and pose are unchanged copies
//! its rows instead of summing them again. A hit hands back the very rows a
//! miss writes, so nothing drawn depends on what was asked before.

use std::sync::Mutex;

/// How many results each part keeps.
const KEEP: usize = 8;

/// Rows of one part, by the bits of its inputs.
pub(super) type Memo = Mutex<Vec<(Vec<u64>, Vec<[f32; 4]>)>>;

/// An empty memo, for a `static`.
pub(super) const fn memo() -> Memo {
    Mutex::new(Vec::new())
}

/// The bits of `inputs`, as a key.
pub(super) fn key(inputs: &[f64]) -> Vec<u64> {
    inputs.iter().map(|v| v.to_bits()).collect()
}

/// Rows `ranges` of `b` (each from its first row to before its second) as
/// `fill` writes them (it writes nowhere else), copied from `cache` when `key`
/// was seen lately.
pub(super) fn rows(
    cache: &Memo,
    key: Vec<u64>,
    b: &mut [[f32; 4]],
    ranges: &[(usize, usize)],
    fill: impl FnOnce(&mut [[f32; 4]]),
) {
    if let Ok(mut kept) = cache.lock() {
        if let Some(at) = kept.iter().position(|(k, _)| *k == key) {
            let hit = kept.remove(at);
            let mut from = hit.1.iter();
            for &(lo, hi) in ranges {
                let to = b.get_mut(lo..hi).unwrap_or_default();
                for (row, v) in to.iter_mut().zip(&mut from) {
                    *row = *v;
                }
            }
            kept.push(hit);
            return;
        }
    }
    fill(b);
    let made = ranges
        .iter()
        .flat_map(|&(lo, hi)| b.get(lo..hi).unwrap_or_default().iter().copied())
        .collect();
    if let Ok(mut kept) = cache.lock() {
        if kept.len() >= KEEP {
            kept.remove(0);
        }
        kept.push((key, made));
    }
}
