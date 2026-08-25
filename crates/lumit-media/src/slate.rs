//! The missing-footage slate: the picture a layer shows when its media
//! cannot be found (docs/07-UI-SPEC.md §3.3, docs/03 §3).
//!
//! In plain terms: when footage goes missing, a comp must not silently turn
//! black — black looks like a deliberate edit, and the mistake hides until
//! export. Broadcast test bars are the opposite: unmistakably "no signal
//! here", legible at a glance, and universally understood by anyone who has
//! touched video. So a missing layer renders bars instead of nothing.
//!
//! The pattern is *generated*, never loaded: it is a handful of rectangles
//! and two ramps, so shipping it as an image file would mean a binary asset,
//! a fixed resolution and a decode path — for something arithmetic can draw
//! at whatever size the layer happens to be. Pure and deterministic, so it
//! is a plain unit test like the rest of the pixel code.

/// Full-range sRGB, the same encoding a decoded frame arrives in — the slate
/// travels the ordinary footage path (linearise → composite → display), so
/// it must be encoded like footage, not like linear light.
type Rgb = [u8; 3];

const WHITE: Rgb = [255, 255, 255];
const YELLOW: Rgb = [255, 240, 20];
const CYAN: Rgb = [20, 240, 255];
const GREEN: Rgb = [20, 220, 60];
const MAGENTA: Rgb = [220, 20, 120];
const RED: Rgb = [230, 20, 20];
const BLUE: Rgb = [20, 20, 220];
const BLACK: Rgb = [0, 0, 0];

/// The seven main bars, left to right — the classic descending-luminance run.
const BARS: [Rgb; 7] = [WHITE, YELLOW, CYAN, GREEN, MAGENTA, RED, BLUE];
/// The reversed band beneath them, which is what makes the pattern read as
/// test bars rather than a plain gradient.
const UNDER: [Rgb; 7] = [BLUE, MAGENTA, YELLOW, RED, CYAN, BLACK, WHITE];

/// Band boundaries as fractions of the height: main bars, the reversed
/// band, the ramps, then the stepped greys.
const BAND_UNDER: f32 = 0.72;
const BAND_RAMP: f32 = 0.80;
const BAND_STEPS: f32 = 0.86;
/// How much of the width the greyscale ramp and the step wedge occupy; the
/// remainder carries the hue sweep and a black rest field.
const RAMP_SPLIT: f32 = 0.58;
/// Discrete steps in the bottom wedge.
const STEPS: u32 = 12;

/// Interpolated hue sweep (red → yellow → green → cyan → blue → magenta) for
/// the narrow strip beside the greyscale ramp. `t` is 0..1 across the strip.
fn hue_sweep(t: f32) -> Rgb {
    const STOPS: [Rgb; 7] = [RED, YELLOW, GREEN, CYAN, BLUE, MAGENTA, RED];
    let scaled = (t.clamp(0.0, 1.0) * (STOPS.len() - 1) as f32).min((STOPS.len() - 1) as f32);
    let i = scaled.floor() as usize;
    let f = scaled - i as f32;
    let (a, b) = (STOPS[i], STOPS[(i + 1).min(STOPS.len() - 1)]);
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * f) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * f) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * f) as u8,
    ]
}

/// The colour of the slate at pixel `(x, y)` of a `w × h` frame. Pure, so the
/// bands can be asserted directly.
#[must_use]
pub fn sample(x: u32, y: u32, w: u32, h: u32) -> Rgb {
    let (w, h) = (w.max(1), h.max(1));
    let fx = x as f32 / w as f32;
    let fy = y as f32 / h as f32;
    // Which of the seven columns this pixel falls in.
    let col = ((fx * 7.0) as usize).min(6);

    if fy < BAND_UNDER {
        BARS[col]
    } else if fy < BAND_RAMP {
        UNDER[col]
    } else if fy < BAND_STEPS {
        if fx < RAMP_SPLIT {
            // Smooth greyscale ramp, white at the left.
            let v = (255.0 * (1.0 - fx / RAMP_SPLIT)) as u8;
            [v, v, v]
        } else {
            hue_sweep((fx - RAMP_SPLIT) / (1.0 - RAMP_SPLIT))
        }
    } else if fx < RAMP_SPLIT {
        // Stepped wedge, black at the left — the eye reads banding here that
        // a smooth ramp hides.
        let step = ((fx / RAMP_SPLIT) * STEPS as f32) as u32;
        let v = (255 * step.min(STEPS - 1) / (STEPS - 1)) as u8;
        [v, v, v]
    } else {
        BLACK
    }
}

/// The slate as an interleaved RGBA8 buffer (`w × h × 4`, alpha 255) — the
/// same shape a decoded frame arrives in, so a missing layer flows through
/// the ordinary compositing path with nothing else special-cased.
#[must_use]
pub fn colour_bars(w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w.max(1), h.max(1));
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = sample(x, y, w, h);
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn buffer_is_rgba8_of_the_asked_size_and_opaque() {
        let (w, h) = (64u32, 36u32);
        let px = colour_bars(w, h);
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert!(px.chunks_exact(4).all(|p| p[3] == 255), "slate is opaque");
        // Degenerate sizes are safe, never empty or panicking.
        assert_eq!(colour_bars(0, 0).len(), 4);
    }

    #[test]
    fn the_seven_bars_run_white_to_blue_across_the_top() {
        let (w, h) = (700u32, 100u32);
        // Sample the middle of each column in the top band.
        let got: Vec<_> = (0..7).map(|c| sample(c * 100 + 50, 10, w, h)).collect();
        assert_eq!(got, BARS.to_vec());
        // The band beneath is the reversed run — what makes it read as bars.
        let under: Vec<_> = (0..7)
            .map(|c| sample(c * 100 + 50, (h as f32 * 0.75) as u32, w, h))
            .collect();
        assert_eq!(under, UNDER.to_vec());
    }

    #[test]
    fn the_ramps_darken_left_to_right_and_the_wedge_steps() {
        let (w, h) = (1000u32, 1000u32);
        let ramp_y = (h as f32 * 0.82) as u32;
        let left = sample(10, ramp_y, w, h)[0];
        let right = sample((w as f32 * RAMP_SPLIT) as u32 - 10, ramp_y, w, h)[0];
        assert!(left > right, "greyscale ramp runs white → black");

        // The wedge is banded: neighbouring samples inside one step match,
        // and the wedge as a whole climbs from black to white.
        let step_y = (h as f32 * 0.93) as u32;
        let dark = sample(5, step_y, w, h)[0];
        let light = sample((w as f32 * RAMP_SPLIT) as u32 - 5, step_y, w, h)[0];
        assert!(dark < light, "step wedge runs black → white");
        // Right of the split the bottom band is a black rest field.
        assert_eq!(sample(w - 5, step_y, w, h), BLACK);
    }

    #[test]
    fn the_pattern_scales_rather_than_crops() {
        // The same relative position gives the same colour at any size — the
        // point of generating it instead of shipping a fixed image.
        for (w, h) in [(320u32, 180u32), (1920, 1080), (77, 41)] {
            assert_eq!(sample(w / 14, h / 10, w, h), WHITE, "first bar at {w}×{h}");
            assert_eq!(
                sample(w * 13 / 14, h / 10, w, h),
                BLUE,
                "last bar at {w}×{h}"
            );
        }
    }
}
