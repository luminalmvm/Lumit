// Sprite flare (docs/08 §3.29) — the art-directed sibling of the
// physically simulated §3.27, and the twin of
// `lumit_core::fx::cpu::sprite_flare_at`, op for op.
//
// Everything is placed from the light's POSITION, never from the picture's
// brightness. That is the whole difference from §3.27's Matte mode, and the
// reason this one cannot flicker on footage: no threshold to cross, so nothing
// pops in and out as grain moves. The elements march along the line from the
// light through the frame's centre, which is what a real lens does — ghosts are
// reflections about the optical axis, so they swing to the far side of the
// middle as the light crosses frame.
//
// One pass, no textures but the layer itself: every element is procedural.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba16float, write>;

struct Params {
    w: u32,
    h: u32,
    light_x: f32,
    light_y: f32,
    intensity: f32,
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    glow_size: f32,
    glow_intensity: f32,
    ghosts: u32,
    ghost_spacing: f32,
    ghost_size: f32,
    ghost_intensity: f32,
    streak_length: f32,
    streak_intensity: f32,
    streak_angle_deg: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
};
@group(0) @binding(3) var<uniform> p: Params;

const MAX_GHOSTS: u32 = 16u;

fn flare_at(px: f32, py: f32) -> f32 {
    let cx = f32(p.w) * 0.5;
    let cy = f32(p.h) * 0.5;
    var acc = 0.0;

    // The central glow: a soft falloff on the light itself.
    if (p.glow_intensity > 0.0 && p.glow_size > 0.0) {
        let d = length(vec2<f32>(px - p.light_x, py - p.light_y)) / p.glow_size;
        acc = acc + p.glow_intensity * exp(-d * d);
    }

    // The ghosts, mirrored through the centre and shrinking with distance.
    if (p.ghost_intensity > 0.0 && p.ghost_size > 0.0) {
        let ax = p.light_x - cx;
        let ay = p.light_y - cy;
        let n = min(p.ghosts, MAX_GHOSTS);
        for (var i = 1u; i <= n; i = i + 1u) {
            let t = -f32(i) * p.ghost_spacing;
            let gx = cx + ax * t;
            let gy = cy + ay * t;
            let radius = p.ghost_size * (0.35 + 0.65 * abs(t));
            if (radius <= 0.0) {
                continue;
            }
            let d = length(vec2<f32>(px - gx, py - gy)) / radius;
            // A soft-edged disc: flat in the middle, nothing at the rim —
            // what an out-of-focus iris looks like.
            let disc = max(1.0 - d * d, 0.0);
            // Alternate ghosts fall off harder, so the train reads as a train
            // rather than as one smear.
            var shaped = disc;
            if (i % 2u == 0u) {
                shaped = disc * disc;
            }
            acc = acc + p.ghost_intensity * shaped / f32(i);
        }
    }

    // The anamorphic streak: long along its own axis, tight across it.
    if (p.streak_intensity > 0.0 && p.streak_length > 0.0) {
        let a = radians(p.streak_angle_deg);
        let s = sin(a);
        let c = cos(a);
        let dx = px - p.light_x;
        let dy = py - p.light_y;
        let along = (dx * c + dy * s) / p.streak_length;
        let across = (-dx * s + dy * c) / max(p.streak_length * 0.03, 1e-3);
        let d2 = along * along + across * across;
        acc = acc + p.streak_intensity * exp(-d2);
    }

    return max(acc, 0.0);
}

@compute @workgroup_size(8, 8)
fn sprite_flare(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let at = vec2<i32>(i32(gid.x), i32(gid.y));
    let base = textureLoad(src, at, 0);
    if (p.intensity <= 0.0 || p.mix_amt <= 0.0) {
        textureStore(out_tex, at, base);
        return;
    }
    let e = flare_at(f32(gid.x) + 0.5, f32(gid.y) + 0.5) * p.intensity;
    if (e <= 0.0) {
        textureStore(out_tex, at, base);
        return;
    }
    let tint = vec3<f32>(p.tint_r, p.tint_g, p.tint_b);
    // Additive, like every other light in the engine: a flare is light
    // arriving at the sensor, not a grade.
    let lit = base.rgb + e * tint;
    let rgb = base.rgb * (1.0 - p.mix_amt) + lit * p.mix_amt;
    textureStore(out_tex, at, vec4<f32>(rgb, base.a));
}
