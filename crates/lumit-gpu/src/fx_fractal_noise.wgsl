// Fractal noise (docs/08-EFFECTS.md §3.37): the seeded multi-octave generator.
// Mirrors lumit_core::fx::cpu::fractal_noise op-for-op (§1.6: the CPU is the
// oracle); the field itself comes from fx_noise_core.wgsl, which is prepended to
// this file at compile time and is the one WGSL twin of
// lumit_core::fx::noise — shared with fx_turbdisplace.wgsl so the generator and
// the displacer cannot drift apart.
//
// A generator: it replaces the frame edge to edge and writes opaque alpha, so
// nothing of the input's colour is read. The rotation arrives as a host-computed
// cosine/sine pair and every size as a reciprocal, so this kernel runs no
// trigonometry (WGSL's is not correctly rounded) and no division. Mix 0 is the
// bit-exact identity.

struct Params {
    cos_sin_offset: vec4<f32>,       // xy = (cos, sin) of Rotation, zw = offset (raster px)
    inv_scale_z_contrast: vec4<f32>, // xy = 1 ÷ cell size, z = depth, w = Contrast ÷ 100
    brightness: f32,                 // Brightness ÷ 100
    mix_amt: f32,                    // 0..1, blended against the unprocessed input
    gain: f32,                       // Sub influence ÷ 100
    lacunarity: f32,                 // 100 ÷ Sub scaling
    seed: u32,
    octaves: u32,                    // 1..10
    cycle: i32,                      // depth loop length in cells; 0 = no loop
    flags: u32,                      // bit 0 Perlin, bit 1 Turbulent, bit 2 Invert
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn fractal_noise(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let px = f32(xy.x) + 0.5 - p.cos_sin_offset.z;
    let py = f32(xy.y) + 0.5 - p.cos_sin_offset.w;
    // R(−rotation) applied to the pixel offset: the field turns, not the frame.
    let qx = px * p.cos_sin_offset.x + py * p.cos_sin_offset.y;
    let qy = py * p.cos_sin_offset.x - px * p.cos_sin_offset.y;
    // Bit 2 is this effect's own Invert and is not the field's business.
    let field = FractalField(p.seed, p.octaves, p.gain, p.lacunarity, p.flags & 3u, p.cycle);
    let n = nc_fractal(field,
                       qx * p.inv_scale_z_contrast.x,
                       qy * p.inv_scale_z_contrast.y,
                       p.inv_scale_z_contrast.z);
    let n01 = n * 0.5 + 0.5;
    var v = clamp((n01 - 0.5) * p.inv_scale_z_contrast.w + 0.5 + p.brightness, 0.0, 1.0);
    if ((p.flags & 4u) != 0u) {
        v = 1.0 - v;
    }
    let shade = vec3<f32>(v, v, v);
    let outv = o.rgb * (1.0 - p.mix_amt) + shade * p.mix_amt;
    let outa = o.a * (1.0 - p.mix_amt) + p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, outa));
}
