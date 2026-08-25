// Card wipe (docs/08-EFFECTS.md §3.72): the frame as a grid of cards, turning
// away. Mirrors lumit_core::fx::cpu::card_wipe op-for-op (§1.6: the CPU is the
// oracle). fx_noise_core.wgsl is prepended at pipeline build for nc_hash01 —
// the per-card shuffle is the same hash lumit_core::fx::noise draws with.
//
// Lumit's effects GATHER, so the card is never drawn: the pixel solves the
// projection backwards instead. The one-point projection
//
//     f = s·cos θ · D / (D − s·sin θ)
//
// is a Möbius map in s, so its inverse is one division:
//
//     s = f·D / (D·cos θ + f·sin θ)
//
// which is the whole reason this is a single cheap pass rather than a geometry
// pipeline. D is fixed at three card half-widths: Lumit has no 3D camera on an
// effect, so every card is projected in its own local frame (§3.72's fourth
// decision). The literal below is lumit_core::fx::cpu::CARD_VIEW_DISTANCE.
//
// Both ends of the flip are TESTED FOR rather than arrived at through a cosine,
// because cos(π/2) in f32 is 6e-8 and not zero: at t = 0 the pixel passes
// through, at t = 1 it is cleared. Without that, Completion 100 would leave a
// hairline of quarter-strength pixels down each card's spine.
//
// Every card boundary is whole-number arithmetic (§3.65's rule): a division that
// comes out exact would put a pixel in a different card on the two paths, and
// here that is a seam rather than a block.
//
// Mix 0 and Completion 0 are the bit-exact identity; Completion 100 is the
// exactly empty frame.

const CW_PI: f32 = 3.14159265358979323846;
const CW_VIEW_DISTANCE: f32 = 3.0;

struct Params {
    cols: i32,
    rows: i32,
    completion: f32,        // 0..1
    inv_width: f32,         // 100 / Transition width
    one_minus_width: f32,   // 1 − Transition width / 100
    order_axis: u32,        // 0 columns, 1 rows
    order_bias: f32,        // 0 forwards along that axis, 1 backwards
    order_scale: f32,       // +1 forwards, −1 backwards
    axis: u32,              // 0 horizontal, 1 vertical, 2 per card
    direction: u32,         // 0 forwards, 1 backwards, 2 per card
    randomness: f32,        // 0..1
    seed: u32,
    mix_amt: f32,           // 0..1, blended against the unprocessed input
    matte_on: f32,          // 1 = the matte scales Completion per pixel (K-429)
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (K-395, docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl, K-425).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

// == cpu::card_span. The ceilings are the exact inverse of (x·n) / len, which
// cpu::mosaic_span's floors are not — a card is DRAWN to its span, so the last
// pixel of a card has to fall inside the card it was assigned to.
fn card_lo(i: i32, len: i32, n: i32) -> i32 {
    return (i * len + n - 1) / n;
}

// == cpu::bilinear_edge with the repeat-edge policy. A tap never actually
// leaves the frame here (the card is inside it), so this only ever clamps
// against arithmetic dust.
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    return textureLoad(src, clamp(vec2<i32>(x, y), vec2<i32>(0, 0), size - vec2<i32>(1, 1)), 0);
}

fn bilinear_clamped(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = tap(x0i, y0i, size);
    let c10 = tap(x0i + 1, y0i, size);
    let c01 = tap(x0i, y0i + 1, size);
    let c11 = tap(x0i + 1, y0i + 1, size);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn card_wipe(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let cols = clamp(p.cols, 1, 256);
    let rows = clamp(p.rows, 1, 256);

    let i = (xy.x * cols) / size.x;
    let j = (xy.y * rows) / size.y;
    let x0 = card_lo(i, size.x, cols);
    let x1 = card_lo(i + 1, size.x, cols);
    let y0 = card_lo(j, size.y, rows);
    let y1 = card_lo(j + 1, size.y, rows);

    // How far this card has turned: the Flip order ramp, shuffled towards the
    // seed's own value by Randomness, then read against Completion.
    var along = (f32(i) + 0.5) / f32(cols);
    if (p.order_axis != 0u) {
        along = (f32(j) + 0.5) / f32(rows);
    }
    let base = p.order_bias + p.order_scale * along;
    let shuffled = base + (nc_hash01(p.seed, 0u, i, j, 0) - base) * p.randomness;
    // The matte pulls Completion toward 0 per pixel, before the turn is worked
    // out (K-429) — asked per PIXEL and not per card, so a matte can leave one
    // half of a card standing while the other half has flipped away. Read at
    // the destination pixel, where the card's point is standing (K-427).
    var completion = p.completion;
    if (p.matte_on != 0.0) {
        completion = matte_toward(p.completion, 0.0, matte_k(xy));
    }
    let t = clamp((completion - shuffled * p.one_minus_width) * p.inv_width, 0.0, 1.0);

    var v = o;
    if (t >= 1.0) {
        v = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    } else if (t > 0.0) {
        let hx = 0.5 * f32(x1 - x0);
        let hy = 0.5 * f32(y1 - y0);
        let mx = f32(x0) + hx;
        let my = f32(y0) + hy;
        let lx = (f32(xy.x) + 0.5 - mx) / hx;
        let ly = (f32(xy.y) + 0.5 - my) / hy;

        var axis = p.axis;
        if (p.axis == 2u) {
            axis = select(0u, 1u, nc_hash01(p.seed, 1u, i, j, 0) >= 0.5);
        }
        var dir_sign = 1.0;
        if (p.direction == 1u) {
            dir_sign = -1.0;
        } else if (p.direction == 2u) {
            dir_sign = select(-1.0, 1.0, nc_hash01(p.seed, 2u, i, j, 0) < 0.5);
        }

        // The flip coordinate and the one across it, with the card's
        // half-extent on each.
        var f = ly;
        var g = lx;
        var hf = hy;
        var hg = hx;
        if (axis != 0u) {
            f = lx;
            g = ly;
            hf = hx;
            hg = hy;
        }

        let ang = dir_sign * t * (CW_PI * 0.5);
        let sn = sin(ang);
        let cs = cos(ang);
        let s = f * CW_VIEW_DISTANCE / (CW_VIEW_DISTANCE * cs + f * sn);
        let k = CW_VIEW_DISTANCE / (CW_VIEW_DISTANCE - s * sn);
        let across = g / k;

        // The card's own edges in screen units, and the box overlap of this
        // pixel with them — clamp(a) + clamp(b) − 1 rather than a pair of
        // smoothsteps, so a band narrower than a pixel comes out as its width
        // and not as a half-strength line.
        let edge_near = cs * CW_VIEW_DISTANCE / (CW_VIEW_DISTANCE - sn);
        let edge_far = -cs * CW_VIEW_DISTANCE / (CW_VIEW_DISTANCE + sn);
        let cov_f = clamp((edge_near - f) * hf + 0.5, 0.0, 1.0)
            + clamp((f - edge_far) * hf + 0.5, 0.0, 1.0) - 1.0;
        let cov_g = clamp((k - g) * hg + 0.5, 0.0, 1.0)
            + clamp((g + k) * hg + 0.5, 0.0, 1.0) - 1.0;
        let cover = clamp(cov_f, 0.0, 1.0) * clamp(cov_g, 0.0, 1.0);

        // Clamped before sampling so a tap never leaves the card, which is what
        // stops one card bleeding into its neighbour.
        let sc = clamp(s, -1.0, 1.0);
        let ac = clamp(across, -1.0, 1.0);
        var sx = mx + ac * hx;
        var sy = my + sc * hy;
        if (axis != 0u) {
            sx = mx + sc * hx;
            sy = my + ac * hy;
        }
        v = bilinear_clamped(sx, sy, size) * cover;
    }
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}
