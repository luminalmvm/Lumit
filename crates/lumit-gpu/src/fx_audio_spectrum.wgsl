// Audio spectrum. Mirrors lumit_core::fx::effects::audio_spectrum::draw op for op
// (the CPU is the oracle).
//
// THE SHAPE IS NOT BUILT HERE. It arrives as straight pieces in raster pixels,
// three vec4s each (the ends, then the colour at each end), with every piece
// already filed into the 16 px tiles it can reach. A pixel measures only its own
// tile's pieces.

const TILE: u32 = 16u;

struct Params {
    w: u32,
    h: u32,
    cols: u32,
    rows: u32,
    half: f32,
    edge: f32,
    composite: u32,
    mix_amt: f32,
    items_at: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> pieces: array<vec4<f32>>;
// The first cols * rows + 1 words are where each tile's list starts; the lists
// follow from `items_at`.
@group(0) @binding(2) var<storage, read> bins: array<u32>;
@group(0) @binding(3) var src: texture_2d<f32>;
@group(0) @binding(4) var dst: texture_storage_2d<rgba16float, write>;

// == draw::render, and draw::covered inside it.
@compute @workgroup_size(8, 8)
fn audio_spectrum(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let q = vec2<f32>(f32(gid.x) + 0.5, f32(gid.y) + 0.5);
    var best = 0.0;
    var colour = vec3<f32>(0.0);
    let c = gid.x / TILE;
    let r = gid.y / TILE;
    if (c < p.cols && r < p.rows) {
        let tile = r * p.cols + c;
        let first = bins[tile];
        let last = bins[tile + 1u];
        for (var n = first; n < last; n = n + 1u) {
            let k = bins[p.items_at + n] * 3u;
            let s = pieces[k];
            let a = pieces[k + 1u];
            let b = pieces[k + 2u];
            let d = s.zw - s.xy;
            let rv = q - s.xy;
            let len2 = max(d.x * d.x + d.y * d.y, 1e-6);
            let t = clamp((rv.x * d.x + rv.y * d.y) / len2, 0.0, 1.0);
            let o = rv - t * d;
            let dist = sqrt(o.x * o.x + o.y * o.y);
            let cover = clamp((p.half - dist) / p.edge + 0.5, 0.0, 1.0);
            if (cover > best) {
                best = cover;
                colour = a.rgb + (b.rgb - a.rgb) * t;
            }
        }
    }
    let xy = vec2<i32>(gid.xy);
    let was = textureLoad(src, xy, 0);
    var keep = 0.0;
    if (p.composite != 0u) {
        keep = 1.0 - best;
    }
    let drawn = vec4<f32>(colour * best + was.rgb * keep, best + was.a * keep);
    let m = p.mix_amt;
    textureStore(dst, xy, was * (1.0 - m) + drawn * m);
}
