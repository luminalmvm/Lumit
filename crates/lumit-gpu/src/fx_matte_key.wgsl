// Matte key (docs/08-EFFECTS.md §3.21, K-121/K-154): a Keylight-style
// colour-difference keyer. Mirrors lumit_core::fx::cpu::matte_key op-for-op
// (§1.6: the CPU is the oracle). Straight (unpremultiplied) colour (§2.2, the
// wrap fused into the kernel): unpremultiply -> key + despill -> re-premultiply.
//
// The screen colour's largest channel is the primary screen axis; a pixel's
// primary-minus-(balance-weighted)-secondary difference, normalised by the
// screen colour's own, drives the screen matte (gain scales the fall-off). Clip
// black/white/rollback tidy the matte's ends, despill drains screen tint from
// kept pixels, and the Replace method recolours where spill was removed. Every
// step is clamp/min/max/mix -- continuous -- so there is no hard step and the
// effect is safe under the ULP oracle. Mix 0 is the bit-exact identity.

struct Params {
    key: vec4<f32>,            // scene-linear screen colour; alpha ignored
    despill_bias: vec4<f32>,   // despill reference bias; grey is a no-op
    alpha_bias: vec4<f32>,     // matte neutral bias; grey is a no-op
    replace_colour: vec4<f32>, // Hard/Soft replace colour
    gain: f32,                 // screen gain (matte fall-off strength)
    balance: f32,              // 0..1 secondary-channel weighting
    spill: f32,                // 0..1 despill amount
    clip_black: f32,           // 0..1
    clip_white: f32,           // 0..1
    clip_rollback: f32,        // 0..1
    view: u32,                 // 0 Final, 1 Screen matte, 2 Status
    replace_method: u32,       // 0 Source, 1 Hard, 2 Soft, 3 None
    mix_amt: f32,              // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Rec. 709 luma weights, in linear light (== cpu::LUMA).
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// Screen axis split from the screen colour (== cpu::matte_key_axis): returns
// (primary, secondary_a, secondary_b). Ties resolve green > red > blue. The
// booleans are computed from `key` (a uniform), so every pixel takes the same
// branch and the CPU picks the same axis.
fn axis_split(c: vec3<f32>) -> vec3<f32> {
    let green_primary = p.key.g >= p.key.r && p.key.g >= p.key.b;
    let red_primary = p.key.r >= p.key.g && p.key.r >= p.key.b;
    if (green_primary) {
        return vec3<f32>(c.g, c.r, c.b);
    }
    if (red_primary) {
        return vec3<f32>(c.r, c.g, c.b);
    }
    return vec3<f32>(c.b, c.r, c.g);
}

// Put a modified primary value back in the screen channel, secondaries kept
// (== cpu's `despilled[pi] = ...`).
fn recompose(primary: f32, c: vec3<f32>) -> vec3<f32> {
    let green_primary = p.key.g >= p.key.r && p.key.g >= p.key.b;
    let red_primary = p.key.r >= p.key.g && p.key.r >= p.key.b;
    if (green_primary) {
        return vec3<f32>(c.r, primary, c.b);
    }
    if (red_primary) {
        return vec3<f32>(primary, c.g, c.b);
    }
    return vec3<f32>(c.r, c.g, primary);
}

// Balance-weighted secondary reference of a colour (== cpu::matte_key_secref).
fn secref(c: vec3<f32>) -> f32 {
    let s = axis_split(c);
    let lo = min(s.y, s.z);
    let hi = max(s.y, s.z);
    return p.balance * hi + (1.0 - p.balance) * lo;
}

// A colour's primary channel value (the screen axis).
fn primary_of(c: vec3<f32>) -> f32 {
    return axis_split(c).x;
}

@compute @workgroup_size(8, 8)
fn matte_key(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let a = o.a;
    let u = unpremult(o);

    // Alpha-bias neutral, the screen's own biased difference (floored), and the
    // despill-bias offset -- all derived from uniforms, exactly as the CPU does.
    let ab_off = primary_of(p.alpha_bias.rgb) - secref(p.alpha_bias.rgb);
    let sd = max((primary_of(p.key.rgb) - secref(p.key.rgb)) - ab_off, 1e-6);
    let db_off = primary_of(p.despill_bias.rgb) - secref(p.despill_bias.rgb);

    // Screen matte, before clips: 1 on the neutral, 0 on the screen colour.
    let pd = (primary_of(u) - secref(u)) - ab_off;
    let raw = pd / sd;
    let m0 = clamp(1.0 - p.gain * raw, 0.0, 1.0);
    // Clip black/white, then rollback recovers detail toward the pre-clip matte.
    let den = max(p.clip_white - p.clip_black, 1e-6);
    let mc = clamp((m0 - p.clip_black) / den, 0.0, 1.0);
    let m = mc + p.clip_rollback * (m0 - mc);

    // Unspill: pull the primary down toward the (bias-shifted) reference.
    let target = secref(u) + db_off;
    let removed = max(primary_of(u) - target, 0.0);
    let despill = p.spill * removed;
    let despilled = recompose(primary_of(u) - despill, u);

    // Replace method: recolour where spill was removed (`t_repl` = how much).
    let t_repl = clamp(despill / sd, 0.0, 1.0);
    let dl = dot(despilled, LUMA);
    var rgb = despilled; // None
    if (p.replace_method == 0u) {
        rgb = u; // Source
    } else if (p.replace_method == 1u) {
        rgb = mix(despilled, p.replace_colour.rgb, t_repl); // Hard colour
    } else if (p.replace_method == 2u) {
        rgb = mix(despilled, p.replace_colour.rgb * dl, t_repl); // Soft colour
    }

    // View select (all continuous in `m`, so the oracle holds).
    var proc_rgb = vec3<f32>(0.0);
    var proc_a = 0.0;
    if (p.view == 1u) {
        proc_rgb = vec3<f32>(m); // Screen matte
        proc_a = 1.0;
    } else if (p.view == 2u) {
        // Status: greyscale matte tinted where the matte is uncertain.
        let warn = 4.0 * m * (1.0 - m) * 0.5;
        proc_rgb = vec3<f32>(m) + warn * (vec3<f32>(1.0, 0.3, 0.3) - vec3<f32>(m));
        proc_a = 1.0;
    } else {
        // Final: re-premultiply the keyed colour by the new alpha.
        let out_a = a * m;
        proc_rgb = rgb * out_a;
        proc_a = out_a;
    }

    // Mix against the untouched premultiplied input; Mix 0 is the identity.
    let outv = o.rgb * (1.0 - p.mix_amt) + proc_rgb * p.mix_amt;
    let outa = a * (1.0 - p.mix_amt) + proc_a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, outa));
}
