// The camera's depth of field on a model's picture (mesh/lens.rs). Each pass
// reads the textures in `a` and `b` at their own pixels.

struct Lens {
    // A depth's blur, in texels: `reach` times its distance off `focus` over
    // its distance from the eye.
    reach: f32,
    focus: f32,
    // The widest blur in the picture, in texels.
    cap: f32,
    // Distance from the eye for each unit of clip w.
    z_per_w: f32,
};

@group(0) @binding(0) var a: texture_2d<f32>;
@group(0) @binding(1) var b: texture_2d<f32>;
@group(0) @binding(2) var<uniform> p: Lens;

@vertex
fn vs_lens(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let c = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(c * 2.0 - 1.0, 0.0, 1.0);
}

// `a` is one over clip w. How far this pixel spreads, as a share of the cap.
@fragment
fn fs_spread(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let iw = textureLoad(a, vec2<i32>(pos.xy), 0).r;
    if (iw <= 0.0) {
        return vec4<f32>(0.0);
    }
    let z = p.z_per_w / iw;
    let s = clamp(p.reach * abs(z - p.focus) / z / p.cap, 0.0, 1.0);
    return vec4<f32>(s, s, s, 1.0);
}

// `a` is that, `b` it blurred. Empty pixels beside the model take the blur of
// what spreads onto them, or its outline would stay as sharp as it was drawn.
@fragment
fn fs_wider(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let own = textureLoad(a, vec2<i32>(pos.xy), 0).rgb;
    let near = textureLoad(b, vec2<i32>(pos.xy), 0).rgb;
    return vec4<f32>(max(own, min(near * 2.0, vec3<f32>(1.0))), 1.0);
}

// `a` is one over clip w again, as a picture a blur can spread.
@fragment
fn fs_depth_picture(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let iw = textureLoad(a, vec2<i32>(pos.xy), 0).r;
    return select(vec4<f32>(0.0), vec4<f32>(iw, iw, iw, 1.0), iw > 0.0);
}

// `a` is one over clip w, `b` its picture blurred. A pixel keeps its own depth,
// exactly, and the fringe round the model takes the depth spread onto it.
@fragment
fn fs_depth(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let own = textureLoad(a, vec2<i32>(pos.xy), 0).r;
    let near = textureLoad(b, vec2<i32>(pos.xy), 0);
    let spread = select(0.0, near.r / max(near.a, 1e-6), near.a > 0.001);
    return vec4<f32>(select(spread, own, own > 0.0), 0.0, 0.0, 1.0);
}
