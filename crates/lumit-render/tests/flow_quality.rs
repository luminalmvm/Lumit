//! Measuring how good the flow interpolation actually is, on real footage.
//!
//! # In plain terms
//!
//! Up to now "better" has been an opinion. Two people look at a slowed clip and
//! disagree, a change gets made on a hunch, and nobody can say afterwards
//! whether it helped. This turns that into a number, using a trick that gets
//! ground truth out of ordinary footage: **take three consecutive frames, throw
//! the middle one away, rebuild it from its two neighbours, and compare the
//! rebuild against the frame that was actually there.** No synthetic scenes, no
//! hand-labelled motion — the film itself is the answer key.
//!
//! It reports flow against two baselines, and the baselines are the point:
//!
//! - **Nearest** — just show the previous frame. What flow must beat to be
//!   worth running at all.
//! - **Blend** — crossfade the two neighbours. What flow must beat to be worth
//!   running *instead of the cheap thing*. A flow engine that loses to a
//!   crossfade is worse than useless, because it costs far more and the failure
//!   looks like tearing rather than a soft double image.
//!
//! Scores are PSNR (decibels, higher is better; +1 dB is a visible step) and
//! SSIM (0..1, structural agreement, which punishes warping and tearing in a
//! way PSNR is too forgiving of).
//!
//! # Running it
//!
//! ```text
//! LUMIT_FLOW_CLIPS="C:/a.mp4;C:/b.mp4" cargo test -p lumit-render --release \
//!     --test flow_quality -- --ignored --nocapture
//! ```
//!
//! `LUMIT_FLOW_ONLY` keeps only the variants whose label contains it — sweeping
//! one engine constant wants `defaults` and nothing else, and the other five
//! arms are five times the wait for numbers nobody reads.
//!
//! `LUMIT_FLOW_STRIDE` sets the gap between the frames used (default 1).
//! **Animation drawn on 2s or 3s needs it**: consecutive frames there are
//! duplicates, so a triplet is A, A, B and "rebuild the middle" is asking to
//! reproduce a frame identical to one of its inputs — which flatters every
//! method and measures nothing. Stride 2 on a 2s cut steps drawing to drawing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::retime::VectorDetail;
use lumit_flow::FlowSettings;

/// Peak signal-to-noise ratio over RGB bytes, in decibels.
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mut sum = 0f64;
    let mut n = 0usize;
    for (x, y) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for c in 0..3 {
            let d = f64::from(x[c]) - f64::from(y[c]);
            sum += d * d;
            n += 1;
        }
    }
    let mse = sum / n.max(1) as f64;
    if mse <= 0.0 {
        return 99.0; // identical; a finite ceiling reads better than infinity
    }
    10.0 * (255.0f64 * 255.0 / mse).log10()
}

/// Mean SSIM on luma over 8×8 windows.
///
/// Included because PSNR is the wrong instrument on its own here: it scores an
/// error by size and not by shape, so a soft crossfade (many small errors) can
/// out-score a sharp warp (few large ones) even when the warp is the thing that
/// looks broken. SSIM compares local structure, which is where tearing lives.
/// Mean SSIM and the **5th-percentile block** — the worst twentieth of the
/// picture.
///
/// The mean is the wrong instrument on its own for the failure being chased.
/// Optical flow does not go uniformly slightly wrong; it goes badly wrong in a
/// few places — a torn edge, a warped line — and stays right everywhere else.
/// Averaged over a 1080p frame that is a rounding error, which is how a clip a
/// person calls unusable scores level with a crossfade. The worst blocks are
/// where the artefact lives, so they are what has to be measured to be improved.
fn ssim_blocks(a: &[u8], b: &[u8], w: usize, h: usize) -> (f64, f64) {
    let luma = |p: &[u8]| -> Vec<f64> {
        p.chunks_exact(4)
            .map(|c| 0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2]))
            .collect()
    };
    let (x, y) = (luma(a), luma(b));
    let (c1, c2) = (6.5025, 58.5225); // (0.01·255)², (0.03·255)²
    let (mut total, mut count) = (0f64, 0usize);
    let mut scores: Vec<f64> = Vec::new();
    let win = 8;
    for by in (0..h.saturating_sub(win - 1)).step_by(win) {
        for bx in (0..w.saturating_sub(win - 1)).step_by(win) {
            let (mut mx, mut my) = (0f64, 0f64);
            for j in 0..win {
                for i in 0..win {
                    let k = (by + j) * w + bx + i;
                    mx += x[k];
                    my += y[k];
                }
            }
            let n = (win * win) as f64;
            mx /= n;
            my /= n;
            let (mut vx, mut vy, mut cov) = (0f64, 0f64, 0f64);
            for j in 0..win {
                for i in 0..win {
                    let k = (by + j) * w + bx + i;
                    let (dx, dy) = (x[k] - mx, y[k] - my);
                    vx += dx * dx;
                    vy += dy * dy;
                    cov += dx * dy;
                }
            }
            vx /= n - 1.0;
            vy /= n - 1.0;
            cov /= n - 1.0;
            let s = ((2.0 * mx * my + c1) * (2.0 * cov + c2))
                / ((mx * mx + my * my + c1) * (vx + vy + c2));
            total += s;
            scores.push(s);
            count += 1;
        }
    }
    if count == 0 {
        return (1.0, 1.0);
    }
    scores.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
    let p5 = scores[(scores.len() / 20).min(scores.len() - 1)];
    (total / count as f64, p5)
}

/// Crossfade, the Blend baseline.
fn blend(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.iter()
        .zip(b)
        .map(|(x, y)| ((u16::from(*x) + u16::from(*y)) / 2) as u8)
        .collect()
}

struct Score {
    psnr: f64,
    ssim: f64,
    worst: f64,
}

impl Score {
    fn of(got: &[u8], truth: &[u8], w: usize, h: usize) -> Self {
        let (ssim, worst) = ssim_blocks(got, truth, w, h);
        Score {
            psnr: psnr(got, truth),
            ssim,
            worst,
        }
    }
}

fn mean(v: &[Score]) -> (f64, f64, f64) {
    let n = v.len().max(1) as f64;
    (
        v.iter().map(|s| s.psnr).sum::<f64>() / n,
        v.iter().map(|s| s.ssim).sum::<f64>() / n,
        v.iter().map(|s| s.worst).sum::<f64>() / n,
    )
}

/// The variants measured. Each is a question worth an answer: does looking
/// harder help, does measuring smaller help, does the guard cost anything.
///
/// `LUMIT_FLOW_ONLY` keeps just the arms whose label contains it — a sweep of
/// one engine constant only ever wants the `flow (defaults)` row, and paying
/// for the other five arms per triplet makes the sweep six times as long for
/// numbers nobody reads.
fn variants() -> Vec<(&'static str, FlowSettings)> {
    let base = FlowSettings::default();
    let only = std::env::var("LUMIT_FLOW_ONLY").unwrap_or_default();
    let all: Vec<(&'static str, FlowSettings)> = vec![
        ("flow (defaults)", base),
        (
            "flow, detail=Ultra",
            FlowSettings {
                iterations: VectorDetail::Ultra.iterations(),
                min_level_dim: VectorDetail::Ultra.min_level_dim(),
                refine_iters: VectorDetail::Ultra.refine_iters(),
                ..base
            },
        ),
        ("flow, half res", FlowSettings { divisor: 2, ..base }),
        (
            "flow, no refinement",
            FlowSettings {
                refine_iters: 0,
                ..base
            },
        ),
        (
            "flow, smoothness=90",
            FlowSettings {
                smoothness: 90.0,
                ..base
            },
        ),
        (
            "flow, no HUD guard",
            FlowSettings {
                hud_guard: false,
                ..base
            },
        ),
    ];
    all.into_iter()
        .filter(|(label, _)| only.is_empty() || label.contains(&only))
        .collect()
}

#[test]
#[ignore = "harness: set LUMIT_FLOW_CLIPS to ;-separated clip paths"]
fn score_flow_against_its_baselines_on_real_clips() {
    let Ok(clips) = std::env::var("LUMIT_FLOW_CLIPS") else {
        eprintln!(
            "set LUMIT_FLOW_CLIPS to ;-separated paths.\n\
             LUMIT_FLOW_STRIDE sets the frame gap (2 for animation on 2s)."
        );
        return;
    };
    let stride: usize = std::env::var("LUMIT_FLOW_STRIDE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1);
    let samples: usize = std::env::var("LUMIT_FLOW_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);

    for clip in clips.split(';').filter(|s| !s.trim().is_empty()) {
        let path = std::path::Path::new(clip.trim());
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let index = match lumit_media::index::build_frame_index(path) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("{name}: could not index: {e}");
                continue;
            }
        };
        let mut dec = match lumit_media::decode::VideoDecoder::open(path, index) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{name}: could not open: {e}");
                continue;
            }
        };
        let count = dec.frame_count();
        // Triplets spread across the clip, so one busy or one still passage
        // cannot stand in for the whole thing.
        let span = count.saturating_sub(2 * stride);
        if span == 0 {
            eprintln!("{name}: too short for stride {stride}");
            continue;
        }
        let step = (span / samples.max(1)).max(1);

        // The GPU is the shipping path and the default here. `LUMIT_FLOW_CPU=1`
        // pins the oracle instead, which is how a change is measured in the
        // window between the CPU reference landing and its WGSL twin: without
        // it the harness would score the *old* shader and report no effect.
        let mut engine = if std::env::var("LUMIT_FLOW_CPU").is_ok() {
            lumit_flow::FlowEngine::cpu()
        } else {
            lumit_flow::FlowEngine::new_auto()
        };
        let mut nearest = Vec::new();
        let mut blended = Vec::new();
        let mut flows: Vec<Vec<Score>> = (0..variants().len()).map(|_| Vec::new()).collect();
        let (mut w, mut h) = (0usize, 0usize);
        let mut identical = 0usize;

        for start in (0..span).step_by(step).take(samples) {
            let Ok(a) = dec.frame_rgba(start, None) else {
                continue;
            };
            let Ok(mid) = dec.frame_rgba(start + stride, None) else {
                continue;
            };
            let Ok(b) = dec.frame_rgba(start + 2 * stride, None) else {
                continue;
            };
            w = a.width as usize;
            h = a.height as usize;
            // A triplet measures nothing unless all three pictures differ.
            //
            // Equal *ends* is the obvious case — every method reproduces it.
            // The subtle one is a middle that duplicates an end, which is the
            // norm in animation drawn on 2s and 3s: there, "hold the previous
            // frame" reproduces the answer exactly and every other method is
            // scored against a target one of its inputs already is. Measured on
            // the owner's anime clip, 78% of neighbouring pairs are held, so
            // leaving these in is not a rounding error — it is most of the
            // sample, and it is what made holding look like the best method on
            // that clip in an earlier run of this harness.
            //
            // Compared loosely: a held cel is not bit-identical after encoding.
            let held = |x: &[u8], y: &[u8]| -> bool {
                let n = x.len() / 4;
                if n == 0 {
                    return true;
                }
                let sum: f64 = x
                    .chunks_exact(4)
                    .zip(y.chunks_exact(4))
                    .map(|(p, q)| {
                        let lp = 0.2126 * f64::from(p[0])
                            + 0.7152 * f64::from(p[1])
                            + 0.0722 * f64::from(p[2]);
                        let lq = 0.2126 * f64::from(q[0])
                            + 0.7152 * f64::from(q[1])
                            + 0.0722 * f64::from(q[2]);
                        (lp - lq).abs()
                    })
                    .sum();
                sum / (n as f64) < 0.5
            };
            if held(&a.rgba, &b.rgba) || held(&a.rgba, &mid.rgba) || held(&mid.rgba, &b.rgba) {
                identical += 1;
                continue;
            }
            nearest.push(Score::of(&a.rgba, &mid.rgba, w, h));
            blended.push(Score::of(&blend(&a.rgba, &b.rgba), &mid.rgba, w, h));
            for (vi, (_, set)) in variants().iter().enumerate() {
                let got = engine.interpolate_at(&a.rgba, &b.rgba, w, h, 0.5, set);
                flows[vi].push(Score::of(&got, &mid.rgba, w, h));
            }
        }

        if nearest.is_empty() {
            eprintln!("{name}: no usable triplets ({identical} were duplicates)");
            continue;
        }
        let (np, ns, nw) = mean(&nearest);
        let (bp, bs, bw) = mean(&blended);
        println!(
            "\n=== {name}  {w}x{h}  stride {stride}  {} triplets{} ===",
            nearest.len(),
            if identical > 0 {
                format!(", {identical} skipped as duplicates")
            } else {
                String::new()
            }
        );
        println!(
            "  {:<22} {:>8} {:>8} {:>9}   {:>8} {:>9}",
            "method", "PSNR dB", "SSIM", "worst 5%", "vs blend", "worst d"
        );
        println!("  {:<22} {np:>8.2} {ns:>8.4} {nw:>9.4}", "nearest (hold)");
        println!(
            "  {:<22} {bp:>8.2} {bs:>8.4} {bw:>9.4}",
            "blend (crossfade)"
        );
        for (vi, (label, _)) in variants().iter().enumerate() {
            let (fp, fs, fw) = mean(&flows[vi]);
            println!(
                "  {label:<22} {fp:>8.2} {fs:>8.4} {fw:>9.4}   {:>+8.2} {:>+9.4}",
                fp - bp,
                fw - bw
            );
        }
        println!(
            "  backend: {}   (a flow row below blend is a net loss: dearer AND worse)",
            engine.backend()
        );
    }
}
