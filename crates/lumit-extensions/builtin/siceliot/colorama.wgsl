// Colorama: After Effects' Colorama. Each pixel's phase is read from one of
// its channels (and, with a layer in Add phase, combined with that layer's),
// shifted by Phase shift and repeated Cycle repetitions times, then looked up
// on the output cycle: a ring of colours that wraps from the last back to the
// first. The colour found replaces what Modify names, only where Pixel
// selection matches. The work is done on display values, as After Effects
// does in a non-linear project, and the result goes back to linear.

struct Params {
    /// @choice("Intensity", "Red", "Green", "Blue", "Hue", "Lightness", "Saturation", "Value", "Alpha", "Zero") @default("Intensity") Get phase from
    get_phase_from: u32,
    /// @choice("Intensity", "Red", "Green", "Blue", "Hue", "Lightness", "Saturation", "Value", "Alpha", "Zero") @default("Intensity") @enabled_if(input) Add phase from
    add_phase_from: u32,
    /// @choice("Wrap", "Clamp", "Average", "Screen") @default("Wrap") @enabled_if(input) Add mode
    add_mode: u32,
    /// @dial @default(0) Phase shift
    phase_shift: f32,
    /// @choice("Custom", "Rainbow", "Ramp grey", "Ramp red", "Ramp green", "Ramp blue", "Fire", "Ice", "Golden", "Negative") @default("Custom") Preset palette
    palette: u32,
    /// @toggle @default(1) Interpolate palette
    interpolate: u32,
    /// @slider(0, 10) @default(1) Cycle repetitions
    repetitions: f32,
    /// @counter(2, 8) @default(6) @enabled_if(palette=0) Stops
    stops: i32,
    /// @default(1, 0, 0, 1) @enabled_if(palette=0) Colour 1
    colour_1: vec4<f32>,
    /// @bounded(0, 100) @default(0) @enabled_if(palette=0) Position 1
    position_1: f32,
    /// @default(1, 1, 0, 1) @enabled_if(palette=0) Colour 2
    colour_2: vec4<f32>,
    /// @bounded(0, 100) @default(16.667) @enabled_if(palette=0) Position 2
    position_2: f32,
    /// @default(0, 1, 0, 1) @enabled_if(palette=0) Colour 3
    colour_3: vec4<f32>,
    /// @bounded(0, 100) @default(33.333) @enabled_if(palette=0) Position 3
    position_3: f32,
    /// @default(0, 1, 1, 1) @enabled_if(palette=0) Colour 4
    colour_4: vec4<f32>,
    /// @bounded(0, 100) @default(50) @enabled_if(palette=0) Position 4
    position_4: f32,
    /// @default(0, 0, 1, 1) @enabled_if(palette=0) Colour 5
    colour_5: vec4<f32>,
    /// @bounded(0, 100) @default(66.667) @enabled_if(palette=0) Position 5
    position_5: f32,
    /// @default(1, 0, 1, 1) @enabled_if(palette=0) Colour 6
    colour_6: vec4<f32>,
    /// @bounded(0, 100) @default(83.333) @enabled_if(palette=0) Position 6
    position_6: f32,
    /// @default(1, 1, 1, 1) @enabled_if(palette=0) Colour 7
    colour_7: vec4<f32>,
    /// @bounded(0, 100) @default(90) @enabled_if(palette=0) Position 7
    position_7: f32,
    /// @default(0, 0, 0, 1) @enabled_if(palette=0) Colour 8
    colour_8: vec4<f32>,
    /// @bounded(0, 100) @default(95) @enabled_if(palette=0) Position 8
    position_8: f32,
    /// @choice("All", "Hue", "Lightness", "Saturation", "Hue and lightness", "Hue and saturation", "Lightness and saturation", "Red", "Green", "Blue", "None") @default("All") Modify
    modify: u32,
    /// @toggle @default(1) Modify alpha
    modify_alpha: u32,
    /// @toggle @default(0) Change empty pixels
    change_empty: u32,
    /// @choice("Off", "RGB", "Hue", "Chroma") @default("Off") Matching mode
    matching_mode: u32,
    /// @default(1, 1, 1, 1) @enabled_if(matching_mode!=0) Matching colour
    matching_colour: vec4<f32>,
    /// @bounded(0, 100) @default(0) @enabled_if(matching_mode!=0) Matching tolerance
    matching_tolerance: f32,
    /// @bounded(0, 100) @default(0) @enabled_if(matching_mode!=0) Matching softness
    matching_softness: f32,
};

// ---- Display values: the sRGB curve, both ways ----

fn cr_encode1(v: f32) -> f32 {
    let x = clamp(v, 0.0, 1.0);
    if (x <= 0.0031308) {
        return x * 12.92;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

fn cr_decode1(v: f32) -> f32 {
    if (v <= 0.04045) {
        return v / 12.92;
    }
    return pow((v + 0.055) / 1.055, 2.4);
}

fn cr_encode(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(cr_encode1(c.r), cr_encode1(c.g), cr_encode1(c.b));
}

fn cr_decode(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(cr_decode1(c.r), cr_decode1(c.g), cr_decode1(c.b));
}

// ---- HSL ----

fn cr_hsl(c: vec3<f32>) -> vec3<f32> {
    let hi = max(c.r, max(c.g, c.b));
    let lo = min(c.r, min(c.g, c.b));
    let l = (hi + lo) * 0.5;
    let d = hi - lo;
    if (d <= 1e-6) {
        return vec3<f32>(0.0, 0.0, l);
    }
    let s = d / (1.0 - abs(2.0 * l - 1.0) + 1e-6);
    var h = 0.0;
    if (hi == c.r) {
        h = (c.g - c.b) / d;
    } else if (hi == c.g) {
        h = (c.b - c.r) / d + 2.0;
    } else {
        h = (c.r - c.g) / d + 4.0;
    }
    return vec3<f32>(fract(h / 6.0 + 1.0), clamp(s, 0.0, 1.0), l);
}

fn cr_rgb(hsl: vec3<f32>) -> vec3<f32> {
    let k = vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0);
    let rgb = clamp(abs(fract(hsl.x + k) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    let ch = (1.0 - abs(2.0 * hsl.z - 1.0)) * hsl.y;
    return hsl.z + ch * (rgb.zyx - 0.5);
}

// One pixel's phase, 0 to 1, from the channel `channel` names; `c` is display
// RGB, `a` alpha.
fn cr_phase(channel: u32, c: vec3<f32>, a: f32) -> f32 {
    switch channel {
        case 1u: { return c.r; }
        case 2u: { return c.g; }
        case 3u: { return c.b; }
        case 4u: { return cr_hsl(c).x; }
        case 5u: { return cr_hsl(c).z; }
        case 6u: { return cr_hsl(c).y; }
        case 7u: { return max(c.r, max(c.g, c.b)); }
        case 8u: { return a; }
        case 9u: { return 0.0; }
        default: { return dot(c, vec3<f32>(0.299, 0.587, 0.114)); }
    }
}

// ---- The output cycle ----

// The colour at `t` (0 to 1) between two colours `a` and `b` met at `f`, or
// the first while Interpolate palette is off.
fn cr_step(a: vec4<f32>, b: vec4<f32>, f: f32) -> vec4<f32> {
    if (p.interpolate == 0u) {
        return a;
    }
    return mix(a, b, clamp(f, 0.0, 1.0));
}

// A fixed palette: `n` colours spread evenly from 0 to 1, without wrapping.
fn cr_ramp(cols: array<vec4<f32>, 5>, n: i32, t: f32) -> vec4<f32> {
    let x = t * f32(n - 1);
    let i = min(i32(floor(x)), n - 2);
    var c = cols;
    return cr_step(c[i], c[i + 1], x - f32(i));
}

// The Custom ring: the first Stops colours by Position, wrapping from the
// last back round to the first. Display values, straight alpha.
fn cr_custom(t: f32) -> vec4<f32> {
    let n = clamp(p.stops, 2, 8);
    var pos = array<f32, 8>(p.position_1, p.position_2, p.position_3, p.position_4,
                            p.position_5, p.position_6, p.position_7, p.position_8);
    var col = array<vec4<f32>, 8>(p.colour_1, p.colour_2, p.colour_3, p.colour_4,
                                  p.colour_5, p.colour_6, p.colour_7, p.colour_8);
    for (var i = 0; i < 8; i += 1) {
        pos[i] = clamp(pos[i] / 100.0, 0.0, 1.0);
        col[i] = vec4<f32>(cr_encode(col[i].rgb), clamp(col[i].a, 0.0, 1.0));
    }
    for (var i = 1; i < n; i += 1) {
        var j = i;
        loop {
            if (j <= 0 || pos[j - 1] <= pos[j]) {
                break;
            }
            let tp = pos[j];
            pos[j] = pos[j - 1];
            pos[j - 1] = tp;
            let tc = col[j];
            col[j] = col[j - 1];
            col[j - 1] = tc;
            j -= 1;
        }
    }
    let last = n - 1;
    // Before the first stop or after the last: the stretch that wraps.
    if (t < pos[0] || t >= pos[last]) {
        let span = max(1.0 - pos[last] + pos[0], 1e-6);
        let f = fract(t - pos[last] + 1.0) / span;
        return cr_step(col[last], col[0], f);
    }
    for (var i = 1; i < n; i += 1) {
        if (t < pos[i]) {
            let f = (t - pos[i - 1]) / max(pos[i] - pos[i - 1], 1e-6);
            return cr_step(col[i - 1], col[i], f);
        }
    }
    return col[last];
}

fn cr_palette(t: f32) -> vec4<f32> {
    let k = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    let w = vec4<f32>(1.0);
    switch p.palette {
        case 1u: {
            if (p.interpolate == 0u) {
                return vec4<f32>(cr_rgb(vec3<f32>(floor(t * 6.0) / 6.0, 1.0, 0.5)), 1.0);
            }
            return vec4<f32>(cr_rgb(vec3<f32>(t, 1.0, 0.5)), 1.0);
        }
        case 2u: { return cr_ramp(array<vec4<f32>, 5>(k, w, w, w, w), 2, t); }
        case 3u: { return cr_ramp(array<vec4<f32>, 5>(k, vec4<f32>(1.0, 0.0, 0.0, 1.0), w, w, w), 2, t); }
        case 4u: { return cr_ramp(array<vec4<f32>, 5>(k, vec4<f32>(0.0, 1.0, 0.0, 1.0), w, w, w), 2, t); }
        case 5u: { return cr_ramp(array<vec4<f32>, 5>(k, vec4<f32>(0.0, 0.0, 1.0, 1.0), w, w, w), 2, t); }
        case 6u: {
            return cr_ramp(array<vec4<f32>, 5>(k, vec4<f32>(0.6, 0.0, 0.0, 1.0),
                vec4<f32>(1.0, 0.35, 0.0, 1.0), vec4<f32>(1.0, 0.85, 0.1, 1.0), w), 5, t);
        }
        case 7u: {
            return cr_ramp(array<vec4<f32>, 5>(k, vec4<f32>(0.0, 0.1, 0.45, 1.0),
                vec4<f32>(0.1, 0.5, 0.9, 1.0), vec4<f32>(0.6, 0.9, 1.0, 1.0), w), 5, t);
        }
        case 8u: {
            return cr_ramp(array<vec4<f32>, 5>(vec4<f32>(0.2, 0.1, 0.0, 1.0),
                vec4<f32>(0.6, 0.4, 0.05, 1.0), vec4<f32>(0.95, 0.78, 0.3, 1.0),
                vec4<f32>(1.0, 0.95, 0.75, 1.0), w), 4, t);
        }
        case 9u: { return cr_ramp(array<vec4<f32>, 5>(w, k, k, k, k), 2, t); }
        default: { return cr_custom(t); }
    }
}

// How much of the change a pixel takes under Pixel selection, 0 to 1.
fn cr_selected(c: vec3<f32>) -> f32 {
    if (p.matching_mode == 0u) {
        return 1.0;
    }
    let m = cr_encode(p.matching_colour.rgb);
    var d = 0.0;
    switch p.matching_mode {
        case 2u: {
            let dh = abs(cr_hsl(c).x - cr_hsl(m).x);
            d = min(dh, 1.0 - dh) * 2.0;
        }
        case 3u: {
            let cc = c / max(c.r + c.g + c.b, 1e-6);
            let mc = m / max(m.r + m.g + m.b, 1e-6);
            d = length(cc - mc) / 1.4142;
        }
        default: { d = length(c - m) / 1.7321; }
    }
    let tol = p.matching_tolerance / 100.0;
    let soft = p.matching_softness / 100.0;
    if (d <= tol) {
        return 1.0;
    }
    if (soft <= 0.0) {
        return 0.0;
    }
    return 1.0 - smoothstep(tol, tol + soft, d);
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let here = lumit_sample(uv);
    if (here.a <= 0.0 && p.change_empty == 0u) {
        return here;
    }
    let c = cr_encode(lumit_unpremult(here).rgb);

    // Input phase.
    var phase = cr_phase(p.get_phase_from, c, here.a);
    if (lumit.input2_on > 0.5) {
        let other = lumit_sample2(uv);
        let q = cr_phase(p.add_phase_from, cr_encode(lumit_unpremult(other).rgb), other.a);
        switch p.add_mode {
            case 1u: { phase = min(phase + q, 1.0); }
            case 2u: { phase = (phase + q) * 0.5; }
            case 3u: { phase = 1.0 - (1.0 - phase) * (1.0 - q); }
            default: { phase = phase + q; }
        }
    }
    let t = fract(phase * p.repetitions + p.phase_shift / 360.0 + 1.0);

    // Output cycle, then Modify.
    let pal = cr_palette(t);
    let hsl = cr_hsl(c);
    let ph = cr_hsl(pal.rgb);
    var out = c;
    switch p.modify {
        case 1u: { out = cr_rgb(vec3<f32>(ph.x, hsl.y, hsl.z)); }
        case 2u: { out = cr_rgb(vec3<f32>(hsl.x, hsl.y, ph.z)); }
        case 3u: { out = cr_rgb(vec3<f32>(hsl.x, ph.y, hsl.z)); }
        case 4u: { out = cr_rgb(vec3<f32>(ph.x, hsl.y, ph.z)); }
        case 5u: { out = cr_rgb(vec3<f32>(ph.x, ph.y, hsl.z)); }
        case 6u: { out = cr_rgb(vec3<f32>(hsl.x, ph.y, ph.z)); }
        case 7u: { out = vec3<f32>(pal.r, c.g, c.b); }
        case 8u: { out = vec3<f32>(c.r, pal.g, c.b); }
        case 9u: { out = vec3<f32>(c.r, c.g, pal.b); }
        case 10u: {}
        default: { out = pal.rgb; }
    }
    var a = here.a;
    if (p.modify_alpha == 1u) {
        a = pal.a;
        if (here.a > 0.0 || p.change_empty == 0u) {
            a = here.a * pal.a;
        }
    } else if (here.a <= 0.0) {
        a = pal.a;
    }
    let changed = vec4<f32>(cr_decode(out) * a, a);
    return mix(here, changed, cr_selected(c));
}
