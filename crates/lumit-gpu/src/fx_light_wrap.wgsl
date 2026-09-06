// Light wrap (docs/08 §3.28) — two small passes over blurs the ordinary
// gaussian has already made.
//
// The effect needs three pictures: the foreground, the background softened
// over the wrap's width, and the foreground's softened ALPHA. The last two are
// both just `fx.blur` run once each — blurring the whole foreground gets its
// softened matte for free, which is why this effect owns no blur of its own
// and why the CPU twin (`lumit_core::fx::cpu::light_wrap`) can be the same
// three lines.
//
// Three inputs is one more than a kernel here binds, so `pack` folds the two
// blurs into one texture — the spill in rgb, the softened alpha in a — and
// `combine` reads the foreground against that.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var other: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba16float, write>;

struct Params {
    w: u32,
    h: u32,
    intensity: f32,
    mix_amt: f32,
};
@group(0) @binding(3) var<uniform> p: Params;

// src = the blurred background (spill), other = the blurred foreground, of
// which only the alpha is wanted.
@compute @workgroup_size(8, 8)
fn pack(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let spill = textureLoad(src, at, 0).rgb;
    let soft_a = textureLoad(other, at, 0).a;
    textureStore(out_tex, at, vec4<f32>(spill, soft_a));
}

// src = the foreground, other = `pack`'s output.
//
// The band is where the matte has been softened AWAY from solid. Blurring a
// solid subject's alpha leaves 1 deep inside, about a half right at the
// outline and less beyond it — so `1 − soft.a` is zero in the middle and rises
// toward the edge, which is the wrap. The doubling brings it to full strength
// at the outline rather than a half, and `a` gates it so the wrap never paints
// on transparent pixels, which would grow a halo OUTSIDE the matte. The spill
// is SCREENED on, so a bright plate brightens the edge toward itself rather
// than past white.
@compute @workgroup_size(8, 8)
fn combine(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let base = textureLoad(src, at, 0);
    let a = base.a;
    let packed = textureLoad(other, at, 0);
    let band = clamp((1.0 - packed.a) * 2.0, 0.0, 1.0) * a;
    let k = band * p.intensity;
    if (k <= 0.0) {
        textureStore(out_tex, at, base);
        return;
    }
    let s = max(packed.rgb * k, vec3<f32>(0.0));
    let screened = vec3<f32>(1.0) - (vec3<f32>(1.0) - base.rgb) * (vec3<f32>(1.0) - s);
    let rgb = base.rgb * (1.0 - p.mix_amt) + screened * p.mix_amt;
    textureStore(out_tex, at, vec4<f32>(rgb, a));
}
