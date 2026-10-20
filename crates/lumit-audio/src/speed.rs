//! Sound played at a speed other than its source's, read through a retime.
//!
//! Two ways to play it. [`tape`] reads the source faster or slower, so the
//! pitch rises and falls with the speed. [`keep_pitch`] lays short overlapping
//! grains of the source end to end at the speed the retime asks for, each one
//! nudged to line up with the last, so the pitch stays where it was.
//!
//! Both take a position table: where in the source (in frames, from the start
//! of the slice handed in) each output frame reads, one entry every [`STEP`]
//! output frames and straight lines between. A speed ramp, a freeze or a run
//! backwards are all just a table.

/// Output frames between two entries of a position table.
pub const STEP: usize = 64;

/// Where output frame `i` reads the source, from the table.
fn position(table: &[f64], i: usize) -> f64 {
    let c = i / STEP;
    let along = (i % STEP) as f64 / STEP as f64;
    let a = table.get(c).copied().unwrap_or(f64::NAN);
    let b = table.get(c + 1).copied().unwrap_or(a);
    a + (b - a) * along
}

/// One stereo frame of interleaved `src`, silent outside it.
fn frame_at(src: &[f32], at: i64) -> (f32, f32) {
    let Ok(i) = usize::try_from(at) else {
        return (0.0, 0.0);
    };
    match (src.get(i * 2), src.get(i * 2 + 1)) {
        (Some(l), Some(r)) => (*l, *r),
        _ => (0.0, 0.0),
    }
}

/// The source read at the table's positions, between samples by a straight
/// line: faster is higher, slower is lower, the way tape plays.
#[must_use]
pub fn tape(src: &[f32], table: &[f64], frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    let reach = (src.len() / 2) as f64 + 1.0;
    for (i, pair) in out.chunks_exact_mut(2).enumerate() {
        let p = position(table, i);
        if !(p > -1.0 && p < reach) {
            continue;
        }
        let at = p.floor();
        let f = (p - at) as f32;
        let at = at as i64;
        let (l0, r0) = frame_at(src, at);
        let (l1, r1) = frame_at(src, at + 1);
        pair[0] = l0 + (l1 - l0) * f;
        pair[1] = r0 + (r1 - r0) * f;
    }
    out
}

/// The source played at the table's speed with its pitch kept.
///
/// Grains 40 ms long overlap by half under a Hann window, which sums to one,
/// so at 100% the source comes back as it was. Each grain starts near where
/// the table says, moved by up to 10 ms to where it best matches what the
/// last grain would have carried on into, so the joins don't beat.
#[must_use]
pub fn keep_pitch(src: &[f32], table: &[f64], frames: usize, rate: u32) -> Vec<f32> {
    let grain = ((rate as usize / 25) & !1).max(64);
    let hop = grain / 2;
    let seek = (rate as i64 / 100).max(1);
    let window: Vec<f32> = (0..grain)
        .map(|j| {
            let x = std::f32::consts::TAU * j as f32 / grain as f32;
            0.5 - 0.5 * x.cos()
        })
        .collect();
    let mono = |at: i64| {
        let (l, r) = frame_at(src, at);
        l + r
    };

    // A grain is laid every `hop` frames, starting half a grain early so the
    // first frames get the same two grains as every other.
    let mut out = vec![0.0f32; (frames + grain) * 2];
    let mut last: Option<i64> = None;
    let mut at = 0usize;
    while at < frames + hop {
        let o = at as i64 - hop as i64;
        let want = position(table, o.max(0) as usize) + o.min(0) as f64;
        if want.is_finite() {
            let want = want.round() as i64;
            let start = match last {
                None => want,
                Some(prev) => {
                    // What the last grain would carry on into, and the start
                    // near `want` whose first half sounds most like it.
                    // `want` itself unless another start is clearly better,
                    // so a steady speed is never shuffled by a cycle.
                    let natural = prev + hop as i64;
                    let score = |d: i64| {
                        let (mut dot, mut a, mut b) = (0.0f32, 0.0f32, 0.0f32);
                        for j in (0..hop as i64).step_by(4) {
                            let (x, y) = (mono(natural + j), mono(d + j));
                            dot += x * y;
                            a += x * x;
                            b += y * y;
                        }
                        if a > 0.0 && b > 0.0 {
                            dot / (a * b).sqrt()
                        } else {
                            0.0
                        }
                    };
                    let mut best = want;
                    let mut best_score = score(want) + 1e-3;
                    let mut d = want - seek;
                    while d <= want + seek {
                        let s = score(d);
                        if s > best_score {
                            best_score = s;
                            best = d;
                        }
                        d += 2;
                    }
                    best
                }
            };
            for (j, w) in window.iter().enumerate() {
                let Some(k) = (o + j as i64)
                    .try_into()
                    .ok()
                    .filter(|k: &usize| *k < frames)
                else {
                    continue;
                };
                let (l, r) = frame_at(src, start + j as i64);
                out[k * 2] += l * w;
                out[k * 2 + 1] += r * w;
            }
            last = Some(start);
        } else {
            last = None;
        }
        at += hop;
    }
    out.truncate(frames * 2);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine at `hz`, stereo, `frames` long.
    fn sine(hz: f32, frames: usize, rate: u32) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin();
                [v, v]
            })
            .collect()
    }

    /// Zero crossings a second, which is twice the pitch.
    fn crossings(s: &[f32], rate: u32) -> f32 {
        let left: Vec<f32> = s.iter().step_by(2).copied().collect();
        let n = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        n as f32 * rate as f32 / left.len() as f32
    }

    /// Half speed: tape halves the pitch, keep-pitch holds it, and both run
    /// twice as long through the same stretch of source.
    #[test]
    fn half_speed_tape_drops_an_octave_and_keep_pitch_holds() {
        let rate = 48_000;
        let src = sine(440.0, rate as usize, rate);
        let frames = rate as usize * 2;
        let table: Vec<f64> = (0..=frames / STEP + 1)
            .map(|c| (c * STEP) as f64 * 0.5)
            .collect();
        let tape = tape(&src, &table, frames);
        let kept = keep_pitch(&src, &table, frames, rate);
        let body = |s: &[f32]| s[rate as usize / 2..rate as usize * 3].to_vec();
        assert!((crossings(&body(&tape), rate) - 220.0).abs() < 5.0);
        assert!((crossings(&body(&kept), rate) - 440.0).abs() < 10.0);
    }

    /// At 100% keep-pitch gives the source back.
    #[test]
    fn keep_pitch_at_full_speed_is_the_source() {
        let rate = 48_000;
        let src = sine(440.0, rate as usize, rate);
        let frames = rate as usize / 2;
        let table: Vec<f64> = (0..=frames / STEP + 1).map(|c| (c * STEP) as f64).collect();
        let kept = keep_pitch(&src, &table, frames, rate);
        let worst = kept
            .iter()
            .zip(&src)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "{worst}");
    }
}
