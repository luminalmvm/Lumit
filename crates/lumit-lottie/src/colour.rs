//! Colour in a vector export: Lumit's scene-linear values as the display
//! sRGB a Lottie or an SVG states.
//!
//! A Lottie colour is three numbers from 0 to 1 in display sRGB, and players
//! blend in that encoding. Lumit holds colours in scene-linear light and
//! blends there. So a still colour is converted once; a colour that animates
//! is sampled per frame in linear light and converted, so the file shows the
//! colours Lumit passes through rather than a straight line between two
//! encoded ends (`fit` then keeps only the frames it needs). A gradient gets
//! stops between its own for the same reason ([`between`]).

/// The sRGB transfer: a linear channel as the encoded value a file states.
#[must_use]
pub fn srgb_encode(linear: f64) -> f64 {
    let c = linear.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// The inverse of [`srgb_encode`], for colours read from a file.
#[must_use]
pub fn srgb_decode(encoded: f64) -> f64 {
    let c = encoded.clamp(0.0, 1.0);
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Lumit's own working space (scene-linear Rec.709) as display sRGB. What a
/// project with no colour config exports through, and the host's default.
#[must_use]
pub fn rec709_display(linear: [f64; 3]) -> [f64; 3] {
    linear.map(srgb_encode)
}

/// How far a player's straight mix of two display colours may stray from
/// Lumit's mix in linear light, per channel: half an eight-bit step.
const MIX_TOLERANCE: f64 = 0.5 / 255.0;

/// The finest a span between two stops is cut: a 1024th of it.
const MAX_DEPTH: u32 = 10;

/// The most stops placed between two of a gradient's own.
const MAX_BETWEEN: usize = 64;

/// Where to put stops between two of a gradient's colours, as fractions of
/// the way from one to the other, so that a player mixing the display
/// colours in a straight line stays within [`MIX_TOLERANCE`] of the colours
/// Lumit mixes in linear light. Each pair in `pairs` is the same span at a
/// different moment, and all of them are held to it, so a moving gradient
/// keeps one set of stops. Near black, where the encoding bends most, the
/// stops crowd in; elsewhere a few do.
#[must_use]
pub fn between(pairs: &[([f64; 3], [f64; 3])], display: &dyn Fn([f64; 3]) -> [f64; 3]) -> Vec<f64> {
    let shown = |a: &[f64; 3], b: &[f64; 3], u: f64| {
        display(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * u))
    };
    let straight = |u0: f64, u1: f64| {
        pairs.iter().all(|(a, b)| {
            let (d0, d1) = (shown(a, b, u0), shown(a, b, u1));
            [0.25, 0.5, 0.75].iter().all(|s| {
                let want = shown(a, b, u0 + (u1 - u0) * s);
                (0..3).all(|i| (d0[i] + (d1[i] - d0[i]) * s - want[i]).abs() <= MIX_TOLERANCE)
            })
        })
    };
    let mut out = Vec::new();
    let mut stack = vec![(0.0, 1.0, 0)];
    while let Some((u0, u1, depth)) = stack.pop() {
        if depth >= MAX_DEPTH || out.len() >= MAX_BETWEEN || straight(u0, u1) {
            continue;
        }
        let mid = (u0 + u1) / 2.0;
        out.push(mid);
        stack.push((u0, mid, depth + 1));
        stack.push((mid, u1, depth + 1));
    }
    out.sort_by(f64::total_cmp);
    out
}

/// [`between`] for each span of a gradient: `moments` holds its own stops'
/// linear colours at each moment written, and every moment is held to it.
#[must_use]
pub fn splits(moments: &[Vec<[f64; 3]>], display: &dyn Fn([f64; 3]) -> [f64; 3]) -> Vec<Vec<f64>> {
    let spans = moments
        .iter()
        .map(|m| m.len().saturating_sub(1))
        .min()
        .unwrap_or(0);
    (0..spans)
        .map(|i| {
            let pairs: Vec<([f64; 3], [f64; 3])> = moments
                .iter()
                .filter_map(|m| Some((*m.get(i)?, *m.get(i + 1)?)))
                .collect();
            between(&pairs, display)
        })
        .collect()
}

/// A display colour as `#rrggbb`, how a solid layer names its colour.
#[must_use]
pub fn hex(rgb: [f64; 3]) -> String {
    let [r, g, b] = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}
