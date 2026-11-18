// Linear colour key: makes the pixels near one colour clear, as the Linear
// Color Key of After Effects. Each pixel is compared with the Key colour, by
// its RGB, its hue or its chroma. Within Matching tolerance it is keyed whole;
// across Matching softness beyond that, it fades back in. Keep colours turns
// the key round: only the pixels near the Key colour stay.

struct Params {
    /// @choice("Final output", "Source only", "Matte only") @default("Final output") View
    view: u32,
    /// @default(0, 0, 1, 1) Key colour
    key_colour: vec4<f32>,
    /// @choice("Using RGB", "Using hue", "Using chroma") @default("Using RGB") Match colours
    match_colours: u32,
    /// @bounded(0, 100) @default(0) Matching tolerance (%)
    tolerance: f32,
    /// @bounded(0, 100) @default(10) Matching softness (%)
    softness: f32,
    /// @choice("Key colours", "Keep colours") @default("Key colours") Key operation
    operation: u32,
};

// A colour as the eye weighs it, so a tolerance is the same step in the
// shadows as in the lights.
fn lk_display(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

// Hue as a turn, 0 to 1, and how far from grey the colour is.
fn lk_hue(c: vec3<f32>) -> vec2<f32> {
    let hi = max(c.r, max(c.g, c.b));
    let lo = min(c.r, min(c.g, c.b));
    let d = hi - lo;
    if (d <= 1e-5) {
        return vec2<f32>(0.0, 0.0);
    }
    var h = 0.0;
    if (hi == c.r) {
        h = (c.g - c.b) / d;
        if (h < 0.0) {
            h += 6.0;
        }
    } else if (hi == c.g) {
        h = (c.b - c.r) / d + 2.0;
    } else {
        h = (c.r - c.g) / d + 4.0;
    }
    return vec2<f32>(h / 6.0, d);
}

// The colour without its brightness: blue and red, each less the luma.
fn lk_chroma(c: vec3<f32>) -> vec2<f32> {
    let y = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec2<f32>(c.b - y, c.r - y);
}

// How far the colour is from the key, 0 to 1.
fn lk_distance(c: vec3<f32>, key: vec3<f32>) -> f32 {
    switch p.match_colours {
        case 1u: {
            let a = lk_hue(c);
            let b = lk_hue(key);
            // A grey has no hue to match: it is as far as a colour can be,
            // unless the key is grey too.
            if (a.y <= 1e-5 || b.y <= 1e-5) {
                if (a.y <= 1e-5 && b.y <= 1e-5) {
                    return 0.0;
                }
                return 1.0;
            }
            let d = abs(a.x - b.x);
            return min(d, 1.0 - d) * 2.0;
        }
        case 2u: {
            return clamp(length(lk_chroma(c) - lk_chroma(key)), 0.0, 1.0);
        }
        default: {
            return length(c - key) / 1.7320508;
        }
    }
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let here = lumit_sample(uv);
    if (p.view == 1u) {
        return here;
    }
    let c = lk_display(lumit_unpremult(here).rgb);
    let d = lk_distance(c, lk_display(p.key_colour.rgb));
    let tolerance = p.tolerance * 0.01;
    let softness = p.softness * 0.01;
    // How much of the pixel a key leaves: none inside the tolerance.
    var kept = 0.0;
    if (softness > 1e-5) {
        kept = clamp((d - tolerance) / softness, 0.0, 1.0);
    } else if (d > tolerance) {
        kept = 1.0;
    }
    if (p.operation == 1u) {
        kept = 1.0 - kept;
    }
    if (p.view == 2u) {
        let m = here.a * kept;
        return vec4<f32>(m, m, m, 1.0);
    }
    return here * kept;
}
