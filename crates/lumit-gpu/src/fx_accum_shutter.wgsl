// Accumulation motion blur's per-pixel shutter (docs/08 §3.26, K-429): fold one
// sub-frame render into the running average, at a weight the Matte decides.
//
// **In plain terms.** The effect above this one has already rendered the whole
// scene N times, at N moments spread across the time the shutter is open. The
// ordinary combine averages them equally. This one asks the matte, at every
// pixel, how far open the shutter is *there* — and where it is less than fully
// open, the average is taken over a shorter slice of those N moments, centred
// on the frame's own instant. So one part of the picture is blurred over half a
// frame and another part is not blurred at all, which is a thing no dissolve
// between the blurred and the sharp frame can produce: this is a genuinely
// shorter exposure, not a shorter one faded in.
//
// The samples are treated as cells: sample k owns the span [k/n, (k+1)/n] of
// the open shutter. The window is that whole span scaled toward `anchor` — the
// point where the frame's own time falls — so at k = 1 it is [0, 1] and every
// cell is fully inside it (equal weights, the ordinary average), and at k = 0
// it has shrunk to the instant of the frame. A cell's weight is how much of it
// the window covers, over the window's own width, so the weights sum to one at
// every strength.
//
// Bindings are the shared five-binding fx shape: 0 the running accumulator
// (ignored on the first sample), 1 this sample, 2 the output, 3 the uniform,
// 4 the matte — already prepared (Channel picked, Invert applied) by the
// caller, since this effect has no dispatch seam to do it at.

struct Params {
    anchor: f32,  // where the frame's own time falls across the open shutter, 0..1
    n: f32,       // how many sub-frame samples there are
    k: f32,       // which one this dispatch is folding in, 0..n-1
    first: f32,   // 1 on the first sample: start the accumulator rather than add to it
};

@group(0) @binding(0) var acc: texture_2d<f32>;
@group(0) @binding(1) var sample_tex: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped.
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

@compute @workgroup_size(8, 8)
fn accum_shutter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(dst));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let m = matte_k(xy);
    // The open span, shrunk toward the frame's own moment. Floored at a
    // thousandth rather than at nothing: `hi - lo` is a subtraction of two
    // numbers about `anchor` apart, and at a span of 1e-6 the cancellation
    // costs more than a per cent of the answer. A thousandth of a shutter is
    // far below anything a frame can show, and the weight is clamped besides,
    // so a wholly black matte lands all of its weight on the one sample
    // nearest the frame and none of it anywhere else.
    let span = max(m, 1e-3);
    let lo = p.anchor * (1.0 - m);
    let hi = lo + span;
    let c0 = p.k / p.n;
    let c1 = (p.k + 1.0) / p.n;
    let overlap = max(0.0, min(hi, c1) - max(lo, c0));
    let w = clamp(overlap / span, 0.0, 1.0);
    var out = textureLoad(sample_tex, xy, 0) * w;
    if (p.first == 0.0) {
        out = out + textureLoad(acc, xy, 0);
    }
    textureStore(dst, xy, out);
}
