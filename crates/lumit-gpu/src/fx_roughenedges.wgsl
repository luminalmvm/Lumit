// Roughen edges (docs/08-EFFECTS.md §3.57): the alpha edge chewed by a fractal.
// Mirrors lumit_core::fx::cpu::roughen_edges op-for-op (§1.6: the CPU is the
// oracle). The noise itself comes from fx_noise_core.wgsl, prepended to this
// file at compile time — the same twin fx_fractal_noise.wgsl and
// fx_turbdisplace.wgsl read.
//
// This is the SECOND pass. The first is the shipped §3.8 gaussian at a radius of
// Border, run on the picture and bound here as `orig`: what it gives back is a
// soft alpha whose half-way contour sits exactly where the original edge was and
// whose slope is Border wide — the distance field the roughening needs, for
// nothing. §3.43 reuses the same blur; this is the second time it has paid.
//
// Border 0 never reaches this kernel (the host short-circuits to the identity).

struct Params {
    colour: vec4<f32>,   // scene-linear RGB the chewed band is painted in
    offset: vec2<f32>,   // the field's origin, raster pixels
    inv_scale: f32,      // 1 / Scale, raster pixels
    z: f32,              // depth coordinate
    gain: f32,
    lacunarity: f32,
    influence: f32,      // Fractal influence / 100
    half_width: f32,     // half the cut's width, in alpha
    colour_on: f32,      // 1 to paint the band, 0 to leave it
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    seed: u32,
    octaves: u32,        // 1..10
    cycle: i32,          // depth loop length in cells; 0 = no loop
    flags: u32,          // bit 0 Perlin, bit 1 Turbulent (Spiky)
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
// The blurred picture, not the unprocessed original: this effect is one pass and
// `src` is already its own input, so the second sampled slot carries the soft
// alpha instead (fx_dropshadow.wgsl does the same).
@group(0) @binding(1) var soft: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == cpu::smoothstep_between, written out rather than borrowed so the two paths
// cannot differ on the clamp or the polynomial.
fn smoothstep_between(lo: f32, hi: f32, x: f32) -> f32 {
    let t = clamp((x - lo) / (hi - lo), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

@compute @workgroup_size(8, 8)
fn roughen_edges(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let s = textureLoad(soft, xy, 0);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let bar = s.a;
    let field = FractalField(p.seed, p.octaves, p.gain, p.lacunarity, p.flags, p.cycle);
    let n = nc_fractal(field,
                       (px - p.offset.x) * p.inv_scale,
                       (py - p.offset.y) * p.inv_scale,
                       p.z);
    // 1 on the outline, 0 well inside or well outside it. It weights the noise
    // as well as the edge colour, and that is what confines the chewing to the
    // band: deep inside the shape the shift is exactly zero, so no amount of
    // Fractal influence can punch a hole in the middle of a solid layer.
    let band = 1.0 - abs(2.0 * bar - 1.0);
    let t = bar + n * p.influence * 0.5 * band - 0.5;
    let k = smoothstep_between(-p.half_width, p.half_width, t);
    // The colour is carried STRAIGHT (§2.2): the pixel keeps its own colour and
    // gets a new coverage. A pixel the chewing GREW into has no colour of its
    // own — premultiplied black is what that looks like — so it borrows the
    // blurred neighbourhood's instead of arriving as soot.
    var col = o.rgb / max(o.a, 1e-4);
    if (o.a <= 1e-4) {
        col = s.rgb / max(s.a, 1e-4);
    }
    // The same band paints the chewed border, and nothing else.
    let paint = band * p.colour_on;
    col = col + (p.colour.rgb - col) * paint;
    let out = vec4<f32>(col * k, k);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + out * p.mix_amt);
}
