// Denoise (lumit_core::fx::denoise): what every pass shares. Pasted ahead of
// each of the three modules, so the curve, the colour space and the tone are
// spelled once.
//
// Bindings 0..5 are the pictures a pass reads, 6 and 7 what it writes (each
// module declares its own), 8 the numbers.

struct P {
    // Picture width and height, then the field's.
    dims: vec4<u32>,
    // Dilation, band, view, flags (1: a field is bound, 2: the first band).
    pass_: vec4<u32>,
    // Luma sigma, chroma sigma, t0, t1.
    sig: vec4<f32>,
    // Shadows, midtones, highlights, temporal strength.
    tone: vec4<f32>,
    lband: vec4<f32>,
    cband: vec4<f32>,
    bamt: vec4<f32>,
    // Luma amount, chroma amount, detail, sharpen.
    fin: vec4<f32>,
    // Grain, mix, frames.
    fin2: vec4<f32>,
};

@group(0) @binding(0) var in0: texture_2d<f32>;
@group(0) @binding(1) var in1: texture_2d<f32>;
@group(0) @binding(2) var in2: texture_2d<f32>;
@group(0) @binding(3) var in3: texture_2d<f32>;
@group(0) @binding(4) var in4: texture_2d<f32>;
@group(0) @binding(5) var in5: texture_2d<f32>;
@group(0) @binding(8) var<uniform> p: P;

// == space::oetf: carried past 0 and 1.
fn oetf1(x: f32) -> f32 {
    let a = abs(x);
    var v = a * 12.92;
    if (a > 0.0031308) {
        v = 1.055 * pow(a, 1.0 / 2.4) - 0.055;
    }
    return select(v, -v, x < 0.0);
}

// == space::eotf.
fn eotf1(x: f32) -> f32 {
    let a = abs(x);
    var v = a / 12.92;
    if (a > 0.04045) {
        v = pow((a + 0.055) / 1.055, 2.4);
    }
    return select(v, -v, x < 0.0);
}

fn to_ycc(c: vec3<f32>) -> vec3<f32> {
    let y = 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
    return vec3<f32>(y, (c.z - y) / 1.8556, (c.x - y) / 1.5748);
}

fn from_ycc(c: vec3<f32>) -> vec3<f32> {
    let r = c.x + 1.5748 * c.z;
    let b = c.x + 1.8556 * c.y;
    let g = (c.x - 0.2126 * r - 0.0722 * b) / 0.7152;
    return vec3<f32>(r, g, b);
}

// == space::pixel_to_ycc.
fn pixel_to_ycc(px: vec4<f32>) -> vec4<f32> {
    var k = 1.0;
    if (px.w > 1e-6) {
        k = 1.0 / px.w;
    }
    let e = vec3<f32>(oetf1(px.x * k), oetf1(px.y * k), oetf1(px.z * k));
    return vec4<f32>(to_ycc(e), px.w);
}

// == space::tone.
fn tone_at(y: f32) -> f32 {
    let s = p.tone.x;
    let m = p.tone.y;
    let h = p.tone.z;
    if (y <= 0.1) {
        return s;
    }
    if (y <= 0.45) {
        return s + (m - s) * (y - 0.1) / 0.35;
    }
    if (y <= 0.85) {
        return m + (h - m) * (y - 0.45) / 0.4;
    }
    return h;
}

fn pic() -> vec2<i32> {
    return vec2<i32>(i32(p.dims.x), i32(p.dims.y));
}

// A texel of a picture-sized input, edges held.
fn ld(t: texture_2d<f32>, x: i32, y: i32) -> vec4<f32> {
    let s = pic();
    return textureLoad(t, vec2<i32>(clamp(x, 0, s.x - 1), clamp(y, 0, s.y - 1)), 0);
}

// == atrous::mirror.
fn mirror(i: i32, n: i32) -> i32 {
    let last = max(n - 1, 0);
    var j = abs(i);
    if (j > last) {
        j = 2 * last - j;
    }
    return clamp(j, 0, last);
}
