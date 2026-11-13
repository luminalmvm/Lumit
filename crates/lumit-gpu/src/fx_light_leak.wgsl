// Light leak: a soft band of light, broken up by two layers of drifting value
// noise, screened onto the premultiplied picture. Mirrors
// lumit_core::fx::effects::light_leak op-for-op (the CPU is the oracle). The
// noise core is prepended.

struct Params {
    normal: vec2<f32>,    // the band's normal over its width
    centre: vec2<f32>,    // the picture's centre, pixels
    z: vec2<f32>,         // the depth of the coarse and the fine noise
    inv_h: f32,           // one over the picture's height
    seed: u32,
    colour: vec4<f32>,    // the heart's light times Amount; .w is the mix
    colour_2: vec4<f32>,  // the fringe's light times Amount
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// == light_leak_light.
fn light(xy: vec2<i32>) -> f32 {
    let u = (f32(xy.x) + 0.5 - p.centre.x) * p.inv_h;
    let v = (f32(xy.y) + 0.5 - p.centre.y) * p.inv_h;
    let g = u * p.normal.x + v * p.normal.y;
    let band = exp(-(g * g));
    let n1 = nc_value3(p.seed, 0u, u * 2.0, v * 2.0, p.z.x, 0);
    let n2 = nc_value3(p.seed, 1u, u * 5.0, v * 5.0, p.z.y, 0);
    return clamp(band * (0.55 + 0.9 * n1) + 0.35 * n2 * band, 0.0, 1.0);
}

@compute @workgroup_size(8, 8)
fn light_leak(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let px = textureLoad(src, xy, 0);
    let l = light(xy);
    let leak = (p.colour_2.rgb + (p.colour.rgb - p.colour_2.rgb) * l) * l;
    let rgb = px.rgb + p.colour.w * leak * max(vec3<f32>(px.a) - px.rgb, vec3<f32>(0.0));
    textureStore(dst, xy, vec4<f32>(rgb, px.a));
}
