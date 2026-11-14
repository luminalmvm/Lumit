// Colorama: After Effects' Colorama. Each pixel's phase is read from one of
// its channels (and, with a layer in Add phase, combined with that layer's),
// repeated Cycle repetitions times and shifted by Phase shift, then looked up
// on the output cycle: a wheel of colours. Positions on the wheel are read
// clockwise from its top, as After Effects draws it, and the phase runs round
// it the other way, as After Effects does: a phase of zero is the top, and a
// rising phase goes on to the colour just before the top. The colour found
// replaces what Modify names, only where Pixel selection matches, and with
// Composite over layer the result is laid over the layer as it was. The work
// is done on display values, as After Effects does in a non-linear project,
// and the result goes back to linear.

struct Params {
    /// @choice("Intensity", "Red", "Green", "Blue", "Hue", "Lightness", "Saturation", "Value", "Alpha", "Zero") @default("Intensity") Get phase from
    get_phase_from: u32,
    /// @choice("Intensity", "Red", "Green", "Blue", "Hue", "Lightness", "Saturation", "Value", "Alpha", "Zero") @default("Intensity") @enabled_if(input) Add phase from
    add_phase_from: u32,
    /// @choice("Wrap", "Clamp", "Average", "Screen") @default("Wrap") @enabled_if(input) Add mode
    add_mode: u32,
    /// @dial @default(0) Phase shift
    phase_shift: f32,
    /// @choice("None", "Alpha ramp", "Hue cycle", "Negative", "Ramp red", "RGB", "Saturation ramp", "Solarize red", "Caribbean", "Clay", "Copper", "Deep ocean", "Earthenware", "Fire", "Golden 1") @default("None") Use preset palette
    palette: u32,
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
    /// @slider(0, 10) @default(1) Cycle repetitions
    repetitions: f32,
    /// @toggle @default(1) Interpolate palette
    interpolate: u32,
    /// @choice("All", "Hue", "Lightness", "Saturation", "Hue and lightness", "Hue and saturation", "Lightness and saturation", "Red", "Green", "Blue", "None") @default("All") Modify
    modify: u32,
    /// @toggle @default(1) Modify alpha
    modify_alpha: u32,
    /// @toggle @default(0) Change empty pixels
    change_empty: u32,
    /// @default(1, 0, 0, 1) @enabled_if(matching_mode!=0) Matching colour
    matching_colour: vec4<f32>,
    /// @bounded(0, 100) @default(50) @enabled_if(matching_mode!=0) Matching tolerance
    matching_tolerance: f32,
    /// @bounded(0, 100) @default(0) @enabled_if(matching_mode!=0) Matching softness
    matching_softness: f32,
    /// @choice("Off", "RGB", "Hue", "Chroma") @default("Off") Matching mode
    matching_mode: u32,
    /// @toggle @default(1) Composite over layer
    composite: u32,
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

fn cr_o(r: f32, g: f32, b: f32) -> vec4<f32> {
    return vec4<f32>(r, g, b, 1.0);
}

// Two colours met at `f`, or the first while Interpolate palette is off.
fn cr_step(a: vec4<f32>, b: vec4<f32>, f: f32) -> vec4<f32> {
    if (p.interpolate == 0u) {
        return a;
    }
    return mix(a, b, clamp(f, 0.0, 1.0));
}

// A wheel of the first `n` colours (display values, straight alpha), each at
// a place 0 to 1 clockwise from the top, read at `x`. Between the last colour
// and the first it wraps round through the top; a colour at the very end of
// the wheel meets the first at a hard edge.
fn cr_wheel(at_in: array<f32, 8>, col_in: array<vec4<f32>, 8>, n: i32, x: f32) -> vec4<f32> {
    var at = at_in;
    var col = col_in;
    for (var i = 1; i < n; i += 1) {
        var j = i;
        loop {
            if (j <= 0 || at[j - 1] <= at[j]) {
                break;
            }
            let ta = at[j];
            at[j] = at[j - 1];
            at[j - 1] = ta;
            let tc = col[j];
            col[j] = col[j - 1];
            col[j - 1] = tc;
            j -= 1;
        }
    }
    let last = n - 1;
    if (x < at[0] || x >= at[last]) {
        let span = max(1.0 - at[last] + at[0], 1e-6);
        let f = fract(x - at[last] + 1.0) / span;
        return cr_step(col[last], col[0], f);
    }
    for (var i = 1; i < n; i += 1) {
        if (x < at[i]) {
            let f = (x - at[i - 1]) / max(at[i] - at[i - 1], 1e-6);
            return cr_step(col[i - 1], col[i], f);
        }
    }
    return col[last];
}

// The output cycle read at `x`: None is the Colour and Position rows, the
// rest are After Effects' presets as its wheel draws them.
fn cr_palette(x: f32) -> vec4<f32> {
    let k = cr_o(0.0, 0.0, 0.0);
    let w = cr_o(1.0, 1.0, 1.0);
    let z = array<f32, 8>(0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0);
    switch p.palette {
        case 1u: {
            return cr_wheel(z, array<vec4<f32>, 8>(vec4<f32>(1.0, 1.0, 1.0, 0.0), w, w, w, w, w, w, w), 2, x);
        }
        case 2u: {
            return cr_wheel(array<f32, 8>(0.0, 1.0 / 6.0, 2.0 / 6.0, 0.5, 4.0 / 6.0, 5.0 / 6.0, 1.0, 1.0),
                array<vec4<f32>, 8>(cr_o(1.0, 0.0, 0.0), cr_o(1.0, 1.0, 0.0), cr_o(0.0, 1.0, 0.0),
                    cr_o(0.0, 1.0, 1.0), cr_o(0.0, 0.0, 1.0), cr_o(1.0, 0.0, 1.0), k, k), 6, x);
        }
        case 3u: { return cr_wheel(z, array<vec4<f32>, 8>(w, k, k, k, k, k, k, k), 2, x); }
        case 4u: {
            return cr_wheel(z, array<vec4<f32>, 8>(k, cr_o(1.0, 0.0, 0.0), k, k, k, k, k, k), 2, x);
        }
        case 5u: {
            return cr_wheel(array<f32, 8>(0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0, 1.0),
                array<vec4<f32>, 8>(cr_o(1.0, 0.0, 0.0), cr_o(0.0, 1.0, 0.0), cr_o(0.0, 0.0, 1.0),
                    k, k, k, k, k), 3, x);
        }
        case 6u: {
            return cr_wheel(z, array<vec4<f32>, 8>(cr_o(0.5, 0.5, 0.5), cr_o(0.0, 1.0, 1.0),
                k, k, k, k, k, k), 2, x);
        }
        case 7u: {
            return cr_wheel(array<f32, 8>(0.0, 0.5, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0),
                array<vec4<f32>, 8>(k, cr_o(1.0, 0.0, 0.0), k, k, k, k, k, k), 3, x);
        }
        case 8u: {
            // Caribbean: four blues, round the wheel four times.
            return cr_wheel(array<f32, 8>(0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0),
                array<vec4<f32>, 8>(cr_o(0.45, 0.85, 0.95), cr_o(0.15, 0.6, 0.88),
                    cr_o(0.08, 0.38, 0.72), cr_o(0.2, 0.68, 0.9), k, k, k, k), 4, fract(x * 4.0));
        }
        case 9u: {
            return cr_wheel(array<f32, 8>(0.0, 0.2, 0.4, 0.6, 0.75, 0.9, 1.0, 1.0),
                array<vec4<f32>, 8>(cr_o(0.47, 0.44, 0.22), cr_o(0.5, 0.4, 0.2), cr_o(0.6, 0.33, 0.15),
                    cr_o(0.66, 0.34, 0.14), cr_o(0.68, 0.36, 0.15), cr_o(0.55, 0.42, 0.2), k, k), 6, x);
        }
        case 10u: {
            // Copper: light and dark, eight times round.
            return cr_wheel(array<f32, 8>(0.0, 0.5, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0),
                array<vec4<f32>, 8>(cr_o(0.93, 0.62, 0.5), cr_o(0.55, 0.3, 0.2), k, k, k, k, k, k),
                2, fract(x * 8.0));
        }
        case 11u: {
            return cr_wheel(array<f32, 8>(0.0, 0.2, 0.33, 0.45, 0.55, 0.7, 0.85, 1.0),
                array<vec4<f32>, 8>(cr_o(0.3, 0.65, 0.6), cr_o(0.05, 0.05, 0.45), cr_o(0.1, 0.1, 0.5),
                    cr_o(0.35, 0.1, 0.45), cr_o(0.05, 0.4, 0.15), cr_o(0.1, 0.5, 0.5),
                    cr_o(0.25, 0.65, 0.6), k), 7, x);
        }
        case 12u: {
            return cr_wheel(array<f32, 8>(0.0, 0.2, 0.35, 0.5, 0.65, 0.8, 0.9, 1.0),
                array<vec4<f32>, 8>(cr_o(0.65, 0.55, 0.4), cr_o(0.85, 0.55, 0.25), cr_o(0.7, 0.45, 0.2),
                    cr_o(0.2, 0.25, 0.2), cr_o(0.4, 0.3, 0.2), cr_o(0.35, 0.25, 0.4),
                    cr_o(0.55, 0.5, 0.45), k), 7, x);
        }
        case 13u: {
            return cr_wheel(array<f32, 8>(0.0, 0.2, 0.45, 0.6, 0.75, 0.9, 1.0, 1.0),
                array<vec4<f32>, 8>(k, cr_o(0.45, 0.0, 0.0), cr_o(0.8, 0.15, 0.0), cr_o(1.0, 0.5, 0.0),
                    cr_o(1.0, 0.75, 0.2), cr_o(1.0, 0.93, 0.75), w, k), 7, x);
        }
        case 14u: {
            return cr_wheel(array<f32, 8>(0.0, 0.15, 0.35, 0.55, 0.75, 0.9, 1.0, 1.0),
                array<vec4<f32>, 8>(cr_o(0.4, 0.18, 0.08), cr_o(0.6, 0.32, 0.12), cr_o(0.85, 0.55, 0.25),
                    cr_o(0.95, 0.72, 0.4), cr_o(0.85, 0.6, 0.3), cr_o(0.95, 0.75, 0.45),
                    cr_o(0.97, 0.8, 0.5), k), 7, x);
        }
        default: {
            let col = array<vec4<f32>, 8>(p.colour_1, p.colour_2, p.colour_3, p.colour_4,
                                          p.colour_5, p.colour_6, p.colour_7, p.colour_8);
            var shown = col;
            for (var i = 0; i < 8; i += 1) {
                shown[i] = vec4<f32>(cr_encode(col[i].rgb), clamp(col[i].a, 0.0, 1.0));
            }
            let at = array<f32, 8>(p.position_1 / 100.0, p.position_2 / 100.0,
                p.position_3 / 100.0, p.position_4 / 100.0, p.position_5 / 100.0,
                p.position_6 / 100.0, p.position_7 / 100.0, p.position_8 / 100.0);
            return cr_wheel(at, shown, clamp(p.stops, 2, 8), x);
        }
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
    let t = phase * p.repetitions + p.phase_shift / 360.0;
    // The phase runs round the wheel anticlockwise.
    let x = fract(1.0 - fract(t));

    // Output cycle, then Modify.
    let pal = cr_palette(x);
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
    if (here.a <= 0.0) {
        a = pal.a;
    } else if (p.modify_alpha == 1u) {
        a = here.a * pal.a;
    }
    var changed = vec4<f32>(cr_decode(out) * a, a);
    // Composite over layer: the result laid over the layer as it was, so
    // what Modify alpha clears shows the layer through.
    if (p.composite == 1u) {
        changed = changed + here * (1.0 - changed.a);
    }
    return mix(here, changed, cr_selected(c));
}
