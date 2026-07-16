// The colour pipeline's one shader (docs/impl/gpu-foundation.md §2).
//
// Both passes draw a fullscreen triangle and copy texels; the colour maths
// lives in the TEXTURE FORMATS, which is the whole trick:
//  - linearise pass: source view is Rgba8UnormSrgb, so hardware decodes
//    sRGB → linear on sample; the render target is Rgba16Float, so linear
//    values land in the working format untouched.
//  - display pass: source is the linear Rgba16Float; the render target is
//    Rgba8UnormSrgb, so hardware encodes linear → sRGB on write.
// One shader, zero hand-rolled gamma curves, no chance of drift between
// decode and encode — the auditable single place the design doc demands.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> VsOut {
    // One triangle covering the screen: (-1,-1) (3,-1) (-1,3).
    var out: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_copy(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src, samp, in.uv);
}
