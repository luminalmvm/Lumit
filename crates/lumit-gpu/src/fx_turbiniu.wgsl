struct Params {
    size: vec2<f32>,
    five_samples: f32,
    mix_amount: f32,
    additive: f32,
    _p0: f32,
    _p1: f32,
    _p2: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var ink: texture_2d<f32>;
@group(0) @binding(3) var output: texture_storage_2d<rgba16float, write>;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn draw_vertex(@location(0) xy: vec2<f32>, @location(1) color: vec4<f32>) -> Vertex {
    let clip = xy / params.size * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    return Vertex(vec4(clip, 0.0, 1.0), color);
}

@fragment
fn draw_fragment(vertex: Vertex) -> @location(0) vec4<f32> {
    return vertex.color;
}

fn sample_ink(low: vec2<i32>) -> vec4<f32> {
    let hi = vec2<i32>(textureDimensions(ink)) - 1;
    let lo = vec2<i32>(0);
    return (textureLoad(ink, clamp(low, lo, hi), 0)
        + textureLoad(ink, clamp(low + vec2(1, 0), lo, hi), 0)
        + textureLoad(ink, clamp(low + vec2(0, 1), lo, hi), 0)
        + textureLoad(ink, clamp(low + vec2(1, 1), lo, hi), 0)) * 0.25;
}

@compute @workgroup_size(8, 8)
fn resolve_ink(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= vec2<u32>(params.size)) {
        return;
    }
    let xy = vec2<i32>(id.xy);
    let original = textureLoad(source, xy, 0);
    var color: vec4<f32>;
    if params.five_samples != 0.0 {
        let at = xy * 2;
        color = (sample_ink(at)
            + sample_ink(at + vec2(1, 1))
            + sample_ink(at + vec2(-1, 1))
            + sample_ink(at + vec2(1, -1))
            + sample_ink(at + vec2(-1, -1))) * 0.2;
    } else {
        color = textureLoad(ink, xy, 0);
    }
    let ink_amount = color * params.mix_amount;
    var out: vec4<f32>;
    if params.additive != 0.0 {
        out = vec4(original.rgb + ink_amount.rgb, ink_amount.a + original.a * (1.0 - ink_amount.a));
    } else {
        out = ink_amount + original * (1.0 - ink_amount.a);
    }
    textureStore(output, xy, out);
}
