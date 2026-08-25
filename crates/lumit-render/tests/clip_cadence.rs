//! What a clip's frames actually *are*: how often they repeat, and how much
//! texture they carry.
//!
//! Two questions the flow work kept guessing at. **Cadence** — animation drawn
//! on 2s or 3s holds each drawing, and how often it does decides what the Input
//! rate should be told and whether interpolating between neighbours is even a
//! sensible thing to ask. **Flatness** — the fraction of the picture with
//! essentially no local gradient, which is what decides whether patch matching
//! has anything to work with at all, and is the most likely basis for choosing
//! an engine automatically.
//!
//! Ignored by default: it wants a media file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// Mean absolute luma difference between two frames, 0..255.
fn frame_delta(a: &[u8], b: &[u8]) -> f64 {
    let n = a.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let mut sum = 0f64;
    for (x, y) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let la = 0.2126 * f64::from(x[0]) + 0.7152 * f64::from(x[1]) + 0.0722 * f64::from(x[2]);
        let lb = 0.2126 * f64::from(y[0]) + 0.7152 * f64::from(y[1]) + 0.0722 * f64::from(y[2]);
        sum += (la - lb).abs();
    }
    sum / n as f64
}

/// The fraction of the picture whose local gradient is below `thr` — flat, and
/// therefore invisible to patch matching.
fn flat_fraction(rgba: &[u8], w: usize, h: usize, thr: f64) -> f64 {
    let luma: Vec<f64> = rgba
        .chunks_exact(4)
        .map(|c| 0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2]))
        .collect();
    let (mut flat, mut total) = (0usize, 0usize);
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let i = y * w + x;
            let gx = (luma[i + 1] - luma[i - 1]).abs();
            let gy = (luma[i + w] - luma[i - w]).abs();
            if gx + gy < thr {
                flat += 1;
            }
            total += 1;
        }
    }
    flat as f64 / total.max(1) as f64
}

#[test]
#[ignore = "harness: set LUMIT_FLOW_CLIPS to ;-separated clip paths"]
fn report_cadence_and_flatness() {
    let Ok(clips) = std::env::var("LUMIT_FLOW_CLIPS") else {
        eprintln!("set LUMIT_FLOW_CLIPS");
        return;
    };
    let window: usize = std::env::var("LUMIT_CADENCE_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);

    for clip in clips.split(';').filter(|s| !s.trim().is_empty()) {
        let path = std::path::Path::new(clip.trim());
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let Ok(index) = lumit_media::index::build_frame_index(path) else {
            eprintln!("{name}: could not index");
            continue;
        };
        let Ok(mut dec) = lumit_media::decode::VideoDecoder::open(path, index) else {
            eprintln!("{name}: could not open");
            continue;
        };
        let count = dec.frame_count().min(window);
        if count < 3 {
            continue;
        }
        // Walk consecutively so the decoder never seeks — and so the deltas
        // describe the clip as it plays.
        let mut deltas = Vec::new();
        let mut prev: Option<Vec<u8>> = None;
        let mut flat = 0f64;
        let mut flat_n = 0usize;
        let (mut w, mut h) = (0usize, 0usize);
        for f in 0..count {
            let Ok(fr) = dec.frame_rgba(f, None) else {
                continue;
            };
            w = fr.width as usize;
            h = fr.height as usize;
            if let Some(p) = &prev {
                deltas.push(frame_delta(p, &fr.rgba));
            }
            if f % 20 == 0 {
                flat += flat_fraction(&fr.rgba, w, h, 2.0);
                flat_n += 1;
            }
            prev = Some(fr.rgba);
        }
        if deltas.is_empty() {
            continue;
        }
        // "Held" means the picture barely moved between neighbours. The
        // threshold is generous: compression noise alone moves a frame a
        // little, and a held cel is not bit-identical after encoding.
        let held = deltas.iter().filter(|&&d| d < 0.5).count();
        let mean: f64 = deltas.iter().sum::<f64>() / deltas.len() as f64;
        let mut sorted = deltas.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = sorted[sorted.len() / 2];
        println!(
            "\n{name}  {w}x{h}  {} frame pairs\n  \
             held (delta<0.5): {held} ({:.0}%)\n  \
             mean delta {mean:.2}   median {median:.2}\n  \
             flat fraction {:.1}%  (no local gradient — invisible to patch matching)",
            deltas.len(),
            100.0 * held as f64 / deltas.len() as f64,
            100.0 * flat / flat_n.max(1) as f64,
        );
        // The run length of held frames is the cadence: 1 means every frame is
        // new, 2 means drawn on 2s, and so on.
        let mut runs = Vec::new();
        let mut run = 1usize;
        for &d in &deltas {
            if d < 0.5 {
                run += 1;
            } else {
                runs.push(run);
                run = 1;
            }
        }
        if !runs.is_empty() {
            let mut counts = std::collections::BTreeMap::new();
            for r in &runs {
                *counts.entry(*r).or_insert(0usize) += 1;
            }
            let top: Vec<String> = counts
                .iter()
                .rev()
                .take(4)
                .map(|(k, v)| format!("{k}s x{v}"))
                .collect();
            let mean_run: f64 = runs.iter().sum::<usize>() as f64 / runs.len() as f64;
            println!("  cadence runs: mean {mean_run:.2}  ({})", top.join(", "));
        }
    }
}
