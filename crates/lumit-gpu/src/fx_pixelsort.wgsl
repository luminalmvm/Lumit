// Pixel sort (docs/08-EFFECTS.md §3.99): runs of pixels picked out by a
// threshold, each one sorted along its own line. Mirrors
// lumit_core::fx::cpu::pixel_sort op-for-op (§1.6: the CPU is the oracle); the
// line offsets come from fx_noise_core.wgsl, which is prepended to this file at
// compile time and is the one WGSL twin of lumit_core::fx::maths::lattice_hash.
//
// **One invocation per pixel, and it works out where its own pixel goes.** It
// walks its own span outward from itself until the run of in-band pixels ends,
// counting how many of them belong in front of it on the way. That count is its
// rank, its rank is its place in the span, and it writes itself there. Nothing
// is shared between invocations and nothing waits for anything.
//
// The obvious kernel is the other way round: load the span into workgroup
// memory and run a sorting network over it. That is far cheaper on paper - a
// span of a thousand costs ten passes rather than a thousand reads - and it is
// what the first version of this file did. It is not what ships, because the
// arrays it needs are four bytes a slot and the HLSL compiler's time grows much
// faster than they do. Measured on a software rasteriser, which is what a
// machine with no graphics card falls back to: 2 kB of workgroup memory took
// 95 ms to compile, 6 kB took twelve seconds, and 12 kB - what a span of a
// thousand actually needs - never finished at all. A kernel that cannot be
// compiled without a graphics card is a kernel that cannot be tested, so this
// one reads more and always runs.
//
// The walk is the whole cost: a pixel reads its own span once, so a frame costs
// about width x height x Maximum span length reads. That is what makes Maximum
// span length a dial worth having rather than an apology.
//
// **The spans are kept apart by the walk, not by a sort key.** A pixel stops
// the moment it meets one outside the band, so it can never see past its own
// run, and a pixel that is in no run walks nowhere and cannot move at all. Ties
// fall back to the position, which makes the order total and its answer the
// same on both paths.
//
// Nothing here grades a colour. The whole texel travels, alpha with it, so
// Mix 0 is the bit-exact identity and Mix 100 is a rearrangement of pixels that
// were already in the frame.

struct Params {
    min_v: f32,                 // the bottom of the band that sorts, 0..1
    max_v: f32,                 // the top of it, 0..1
    mix_amt: f32,               // 0..1, blended against the unprocessed input
    matte_on: f32,              // 1 = the matte says where the spans may form
    sort_by: u32,               // 0 R, 1 G, 2 B, 3 Luminance, 4 Hue, 5 Saturation
    span_mode: u32,             // 0 Sort, 1 Stretch, 2 Mirror
    stride: u32,                // the piece of line no span may cross, pixels
    seed: u32,                  // which offsets the pieces take on each line
    vertical: u32,              // 1 = spans run down columns
    reverse: u32,               // 1 = every span ordered the other way round
    pad0: u32,
    pad1: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (docs/08 §2.6), bound for every kernel on this layout and read only
// under `matte_on` - bound to `src` when there is none, since a texture binding
// cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// The value's share of a sort key == cpu::PIXEL_SORT_LEVELS.
const PS_LEVELS: u32 = 0x003fffffu;

fn ps_unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// The HSV hue in degrees 0..360 (== cpu::hsv_hue), spelled as §3.33's kernel
// spells it.
fn ps_hue(u: vec3<f32>, v: f32, c: f32) -> f32 {
    if (c <= 0.0) {
        return 0.0;
    }
    var sixth: f32;
    if (v == u.r) {
        sixth = (u.g - u.b) / c;
    } else if (v == u.g) {
        sixth = (u.b - u.r) / c + 2.0;
    } else {
        sixth = (u.r - u.g) / c + 4.0;
    }
    let h = sixth * 60.0;
    if (h < 0.0) {
        return h + 360.0;
    }
    return h;
}

// The 0..1 value one pixel sorts by (== cpu::pixel_sort_value). `u` is straight
// colour, and a value above 1 reads as 1 so an HDR highlight sits at the top of
// the band rather than outside every band.
fn ps_value(u: vec3<f32>) -> f32 {
    var v: f32;
    if (p.sort_by == 0u) {
        v = u.r;
    } else if (p.sort_by == 1u) {
        v = u.g;
    } else if (p.sort_by == 2u) {
        v = u.b;
    } else if (p.sort_by == 3u) {
        v = u.r * 0.2126 + u.g * 0.7152 + u.b * 0.0722;
    } else if (p.sort_by == 4u) {
        let hi = max(u.r, max(u.g, u.b));
        let lo = min(u.r, min(u.g, u.b));
        v = ps_hue(u, hi, hi - lo) / 360.0;
    } else {
        let hi = max(u.r, max(u.g, u.b));
        let lo = min(u.r, min(u.g, u.b));
        v = 0.0;
        if (hi > 0.0) {
            v = (hi - lo) / hi;
        }
    }
    return clamp(v, 0.0, 1.0);
}

// Truncated, never rounded: WGSL rounds a half to even and Rust rounds it away
// from zero, and one bucket of disagreement is a swapped pixel rather than a
// last-bit difference.
fn ps_quant(v: f32) -> u32 {
    return u32(v * f32(PS_LEVELS));
}

// This pixel's matte answer: a hard half, because a span is a yes or a no and
// there is no half a pixel to give a grey matte. The Channel pick and Invert
// already happened, once, at the seam (fx_matte_prepare.wgsl).
fn ps_matte_ok(xy: vec2<i32>) -> bool {
    if (p.matte_on == 0.0) {
        return true;
    }
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0) >= 0.5;
}

// Where a pixel of this line is. The whole of what Direction changes is this
// one swap.
fn ps_xy(line: i32, pos: i32) -> vec2<i32> {
    if (p.vertical != 0u) {
        return vec2<i32>(line, pos);
    }
    return vec2<i32>(pos, line);
}

// Is this pixel in the band, and what does it sort by.
struct Sample {
    masked: bool,
    key: u32,
};

fn ps_sample(line: i32, pos: i32) -> Sample {
    let xy = ps_xy(line, pos);
    let v = ps_value(ps_unpremult(textureLoad(src, xy, 0)));
    var m = false;
    if (v >= p.min_v && v <= p.max_v && ps_matte_ok(xy)) {
        m = true;
    }
    var q = ps_quant(v);
    if (p.reverse != 0u) {
        // Reverse flips the value and nothing else, so a span turns round
        // without the comparison below knowing anything about it.
        q = PS_LEVELS - q;
    }
    return Sample(m, q);
}

// Does the pixel at `ja` belong in front of the one at `jb`. Equal values fall
// back to the position, which is what keeps equal pixels in the order they
// arrived in, on both paths.
fn ps_before(ka: u32, ja: i32, kb: u32, jb: i32) -> bool {
    if (ka != kb) {
        return ka < kb;
    }
    return ja < jb;
}

// Lay one pixel down, blended against whatever was already in that place by the
// host-uniform Mix (docs/08 §1.5).
fn ps_put(line: i32, pos: i32, taken: vec4<f32>) {
    let d = ps_xy(line, pos);
    let was = textureLoad(src, d, 0);
    textureStore(dst, d, taken * p.mix_amt + was * (1.0 - p.mix_amt));
}

@compute @workgroup_size(8, 8)
fn pixel_sort(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    var len = size.x;
    var line = xy.y;
    var pos = xy.x;
    if (p.vertical != 0u) {
        len = size.y;
        line = xy.x;
        pos = xy.y;
    }

    // The piece of line this pixel belongs to. The line's own offset is what
    // keeps every line from breaking its spans in the same places and drawing
    // the cap as a column down the frame.
    let stride = i32(p.stride);
    let offset = i32(nc_lattice_hash(p.seed, 0u, line, 0, 0) % p.stride);
    let piece = (pos + offset) / stride;
    let lo = max(piece * stride - offset, 0);
    let hi = min(piece * stride - offset + stride - 1, len - 1);

    let here = textureLoad(src, ps_xy(line, pos), 0);
    let me = ps_sample(line, pos);

    // The walk: outward until the run of in-band pixels ends, counting the ones
    // that belong in front on the way.
    var s = pos;
    var e = pos;
    var rank = 0;
    if (me.masked) {
        var j = pos - 1;
        loop {
            if (j < lo) { break; }
            let o = ps_sample(line, j);
            if (!o.masked) { break; }
            if (ps_before(o.key, j, me.key, pos)) { rank = rank + 1; }
            s = j;
            j = j - 1;
        }
        j = pos + 1;
        loop {
            if (j > hi) { break; }
            let o = ps_sample(line, j);
            if (!o.masked) { break; }
            if (ps_before(o.key, j, me.key, pos)) { rank = rank + 1; }
            e = j;
            j = j + 1;
        }
    }
    let span = e - s + 1;

    // Stretch: the whole span takes the pixel the sort put at its far end, so
    // that one pixel writes and the rest write nothing.
    if (p.span_mode == 1u) {
        if (rank != span - 1) {
            return;
        }
        for (var q = s; q <= e; q = q + 1) {
            ps_put(line, q, here);
        }
        return;
    }

    // Mirror: the sorted run laid out from both ends inward, so the span climbs
    // to its middle and falls back. Every pixel is still used exactly once.
    var dest = s + rank;
    if (p.span_mode == 2u) {
        if ((rank & 1) == 0) {
            dest = s + rank / 2;
        } else {
            dest = s + span - 1 - (rank - 1) / 2;
        }
    }
    ps_put(line, dest, here);
}
