// Brightness (docs/08-EFFECTS.md §3.32, K-397: AE's Brightness & Contrast as
// one effect): the affine grade `(u + b - pivot) * k + pivot` per RGB channel
// about the same mid-grey pivot Contrast uses, in linear light on
// unpremultiplied colour (§2.2, the wrap fused into the kernel). Mirrors
// lumit_core::fx::cpu::brightness op-for-op (§1.6: the CPU is the oracle).
// The neutral pair (0, 1) short-circuits, so a neutral Brightness is the
// bit-exact identity. Purely continuous (no round/clamp/quantize).

struct Params {
    b: f32,        // brightness_percent / 100; 0.0 = neutral
    k: f32,        // 1 + contrast_percent / 100; 1.0 = neutral
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    _pad0: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The mid-grey pivot the contrast half expands about (== cpu::CONTRAST_PIVOT).
const PIVOT = 0.5;

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

@compute @workgroup_size(8, 8)
fn brightness(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return).
    if (p.b == 0.0 && p.k == 1.0) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    let v = (u + vec3<f32>(p.b) - vec3<f32>(PIVOT)) * p.k + vec3<f32>(PIVOT);
    let graded = v * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}
