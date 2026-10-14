// Lens flare Matte-mode source detection (docs/08 §3.27, docs/impl/
// lens-flare.md §6): find the brightest sources in the matte layer's
// rendered picture and turn them into the frame's flare lights. Mirrors
// lumit_core::fx::lens_flare::detect_lights op-for-op — the CPU function is
// the oracle — and is deterministic by construction: the tile reduction
// merges fixed-order partials, and the top-K pass runs on a single thread.
//
//   detect_tiles — one workgroup per DETECT_TILE-sided cell: each thread
//                  scans a strided share of the cell's pixels for the
//                  brightest (Rec. 709 luma; ties to the lowest linear
//                  index), thread 0 merges the partials in thread order.
//   detect_pick  — one thread: lit cells whose lit pixels touch across
//                  the edge between them, corners included, join into
//                  patches; each patch's flux is summed, and the
//                  top-MAX_SOURCES patches by flux (ties to the lower first
//                  cell) become the lights, area and all; dead slots zeroed.

// What one tile knows about the light inside it — the WGSL twin of
// lumit_core's `TileStat`. Earlier this was the brightest pixel
// alone, which is what made flares JUMP on footage: which pixel is brightest
// inside a practical changes frame to frame with sensor noise, so the light's
// position hopped about inside a source that had not moved.
struct Tile {
    // Still how anchors are ranked and gated, so a small bright source is
    // still found.
    luma: f32,
    index: u32,
    // Sum of gates, and of colour x gate: their ratio is the mean colour of
    // the light in this tile, which one sparkle cannot define.
    wsum: f32,
    csum_r: f32,
    csum_g: f32,
    csum_b: f32,
    // Sum of luma x gate and its first moments: the tile's flux and where in
    // the tile that flux sits.
    fsum: f32,
    fx: f32,
    fy: f32,
    // The pick pass's own: the cell this one joins (itself for a patch's
    // first cell), and, on a patch's first cell, the whole patch's sums.
    parent: u32,
    acc_r: f32,
    acc_g: f32,
    acc_b: f32,
    cen_x: f32,
    cen_y: f32,
    cen_w: f32,
    spr_x: f32,
    spr_y: f32,
    // Which pixels along the tile's edges are lit, one bit a pixel: the left
    // column low and the right high, then the top row and the bottom.
    edge_lr: u32,
    edge_tb: u32,
};

// Shares its layout with fx_lens_flare_trace.wgsl: position and half-extent
// as raster fractions, colour already gated. The extent is what the trace
// integrates over per ray.
struct Light {
    pos_x: f32,
    pos_y: f32,
    r: f32,
    g: f32,
    b: f32,
    ext_x: f32,
    ext_y: f32,
    _pad2: f32,
};

struct DetectParams {
    w: u32,
    h: u32,
    tiles_x: u32,
    tiles_y: u32,
    threshold: f32,
    softness: f32,
    // 1 = a detected source's own colour tints its flare; 0 = white through
    // the tint alone.
    use_source_colour: u32,
    // 1 = read the matte inverted (1 - rgb), the Matte row's Invert;
    // the twin of `detect_lights`'s `invert`, applied at every load below.
    invert: u32,
    // Scene-linear Light tint, multiplied into every detected light.
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    _pad1: f32,
};

@group(0) @binding(0) var matte_tex: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> tiles: array<Tile>;
@group(0) @binding(2) var<storage, read_write> lights: array<Light>;
@group(0) @binding(3) var<uniform> dp: DetectParams;

const DETECT_TILE: u32 = 16u;
// Distinct sources detection may find, and so the light slots the trace
// carries — one per source however large it is. Mirrors lumit_core's
// MAX_SOURCES.
const MAX_SOURCES: u32 = 16u;

// The soft gate (== lens_flare::threshold_gate): one-sided — closed
// at and below the threshold, fully open a softness above it. Declared
// before its first use, which WGSL requires.
// The Matte row's Invert, at the one place the matte is read: the
// flare owns its matte, so it inverts the raw RGBA itself rather than through
// the dispatch seam's grey `matte_prepare` pass.
fn matte_rgb(xy: vec2<i32>) -> vec3<f32> {
    let c = textureLoad(matte_tex, xy, 0).rgb;
    if (dp.invert != 0u) {
        return vec3<f32>(1.0) - c;
    }
    return c;
}

fn gate(luma: f32) -> f32 {
    if (dp.softness <= 0.0) {
        return f32(luma > dp.threshold);
    }
    let t = clamp((luma - dp.threshold) / dp.softness, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

var<workgroup> partial_luma: array<f32, 64>;
var<workgroup> partial_index: array<u32, 64>;
var<workgroup> partial_w: array<f32, 64>;
var<workgroup> partial_c: array<vec3<f32>, 64>;
var<workgroup> partial_f: array<f32, 64>;
var<workgroup> partial_fx: array<f32, 64>;
var<workgroup> partial_fy: array<f32, 64>;
var<workgroup> partial_lr: array<u32, 64>;
var<workgroup> partial_tb: array<u32, 64>;

@compute @workgroup_size(8, 8)
fn detect_tiles(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    let tile_x0 = wg.x * DETECT_TILE;
    let tile_y0 = wg.y * DETECT_TILE;
    var best_luma = -1.0;
    var best_index = 0u;
    var wsum = 0.0;
    var csum = vec3<f32>(0.0);
    var fsum = 0.0;
    var fx = 0.0;
    var fy = 0.0;
    var lr = 0u;
    var tb = 0u;
    // Each thread strides the tile 8 apart in both axes: 4×4 pixels each.
    // Row-major visit order per thread keeps its own tie-break at the
    // lowest linear index; the merge below keeps the global one.
    for (var oy = lid.y; oy < DETECT_TILE; oy = oy + 8u) {
        let y = tile_y0 + oy;
        if (y >= dp.h) {
            continue;
        }
        for (var ox = lid.x; ox < DETECT_TILE; ox = ox + 8u) {
            let x = tile_x0 + ox;
            if (x >= dp.w) {
                continue;
            }
            let c = matte_rgb(vec2<i32>(i32(x), i32(y)));
            let luma = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
            let index = y * dp.w + x;
            if (luma > best_luma || (luma == best_luma && index < best_index)) {
                best_luma = luma;
                best_index = index;
            }
            // Every lit pixel contributes, not just the brightest.
            let g = gate(luma);
            if (g > 0.0) {
                let f = luma * g;
                wsum = wsum + g;
                csum = csum + max(c, vec3<f32>(0.0)) * g;
                fsum = fsum + f;
                fx = fx + f32(x) * f;
                fy = fy + f32(y) * f;
                if (ox == 0u) {
                    lr = lr | (1u << oy);
                }
                if (ox == DETECT_TILE - 1u) {
                    lr = lr | (1u << (16u + oy));
                }
                if (oy == 0u) {
                    tb = tb | (1u << ox);
                }
                if (oy == DETECT_TILE - 1u) {
                    tb = tb | (1u << (16u + ox));
                }
            }
        }
    }
    partial_luma[li] = best_luma;
    partial_index[li] = best_index;
    partial_w[li] = wsum;
    partial_c[li] = csum;
    partial_f[li] = fsum;
    partial_fx[li] = fx;
    partial_fy[li] = fy;
    partial_lr[li] = lr;
    partial_tb[li] = tb;
    workgroupBarrier();
    if (li == 0u) {
        var luma = -1.0;
        var index = 0u;
        var w_all = 0.0;
        var c_all = vec3<f32>(0.0);
        var f_all = 0.0;
        var fx_all = 0.0;
        var fy_all = 0.0;
        var lr_all = 0u;
        var tb_all = 0u;
        // Thread order, so the sums are the same numbers in the same order
        // every run — the determinism docs/14 requires.
        for (var i = 0u; i < 64u; i = i + 1u) {
            let pl = partial_luma[i];
            let pi = partial_index[i];
            if (pl > luma || (pl == luma && pi < index)) {
                luma = pl;
                index = pi;
            }
            w_all = w_all + partial_w[i];
            c_all = c_all + partial_c[i];
            f_all = f_all + partial_f[i];
            fx_all = fx_all + partial_fx[i];
            fy_all = fy_all + partial_fy[i];
            lr_all = lr_all | partial_lr[i];
            tb_all = tb_all | partial_tb[i];
        }
        var t: Tile;
        t.luma = luma;
        t.index = index;
        t.wsum = w_all;
        t.csum_r = c_all.r;
        t.csum_g = c_all.g;
        t.csum_b = c_all.b;
        t.fsum = f_all;
        t.fx = fx_all;
        t.fy = fy_all;
        t.parent = 0u;
        t.acc_r = 0.0;
        t.acc_g = 0.0;
        t.acc_b = 0.0;
        t.cen_x = 0.0;
        t.cen_y = 0.0;
        t.cen_w = 0.0;
        t.spr_x = 0.0;
        t.spr_y = 0.0;
        t.edge_lr = lr_all;
        t.edge_tb = tb_all;
        tiles[wg.y * dp.tiles_x + wg.x] = t;
    }
}

// lumit_core's `cut_fade`: none of a kept light's strength at the cut,
// all of it at half as much again, smoothly between.
fn cut_fade(flux: f32, cut: f32) -> f32 {
    if (cut <= 0.0) {
        return 1.0;
    }
    let t = clamp((flux - cut) / (0.5 * cut), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn lit(t: u32) -> bool {
    let l = tiles[t].luma;
    return l > 0.0 && gate(l) > 0.0;
}

// The first cell of t's patch, halving the path on the way as lumit_core's
// `detect_lights` does, so both sides leave the same table behind.
fn find(t_in: u32) -> u32 {
    var t = t_in;
    loop {
        let p = tiles[t].parent;
        if (p == t) {
            break;
        }
        let gp = tiles[p].parent;
        tiles[t].parent = gp;
        t = gp;
    }
    return t;
}

// Whether a lit edge pixel in `a` touches one of the three facing pixels
// in `b`, both 16-bit edge masks.
fn meets(a: u32, b: u32) -> bool {
    return (a & (b | (b << 1u) | (b >> 1u)) & 0xffffu) != 0u;
}

fn join(a_in: u32, b_in: u32, touching: bool) {
    if (!touching || !lit(b_in)) {
        return;
    }
    let a = find(a_in);
    let b = find(b_in);
    tiles[max(a, b)].parent = min(a, b);
}

@compute @workgroup_size(1)
fn detect_pick(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x != 0u) {
        return;
    }
    let tile_count = dp.tiles_x * dp.tiles_y;
    let tx = dp.tiles_x;
    // Patches: lit cells whose lit pixels touch across the shared edge or
    // corner, joined in reading order against the four neighbours already
    // visited.
    for (var t = 0u; t < tile_count; t = t + 1u) {
        tiles[t].parent = t;
        if (!lit(t)) {
            continue;
        }
        let x = t % tx;
        let y = t / tx;
        let lr = tiles[t].edge_lr;
        let tb = tiles[t].edge_tb;
        if (x > 0u) {
            join(t, t - 1u, meets(lr, tiles[t - 1u].edge_lr >> 16u));
        }
        if (x > 0u && y > 0u) {
            let n = t - tx - 1u;
            join(t, n, (tb & 1u) != 0u && (tiles[n].edge_tb >> 31u) != 0u);
        }
        if (y > 0u) {
            join(t, t - tx, meets(tb, tiles[t - tx].edge_tb >> 16u));
        }
        if (x + 1u < tx && y > 0u) {
            let n = t - tx + 1u;
            join(t, n, ((tb >> 15u) & 1u) != 0u && ((tiles[n].edge_tb >> 16u) & 1u) != 0u);
        }
    }
    // Each patch's light, summed onto its first cell in reading order.
    for (var t = 0u; t < tile_count; t = t + 1u) {
        if (!lit(t)) {
            continue;
        }
        let root = find(t);
        let tl = tiles[t];
        let weight = gate(tl.luma);
        // The tile's MEAN colour over its lit pixels, not its brightest
        // pixel's: one sparkle among a thousand lit pixels now
        // shifts the colour by a thousandth instead of defining it.
        var src = vec3<f32>(1.0, 1.0, 1.0);
        if (dp.use_source_colour == 1u) {
            if (tl.wsum > 0.0) {
                src = vec3<f32>(tl.csum_r, tl.csum_g, tl.csum_b) / tl.wsum;
            } else {
                let px = tl.index % dp.w;
                let py = tl.index / dp.w;
                let c = matte_rgb(vec2<i32>(i32(px), i32(py)));
                src = max(c, vec3<f32>(0.0));
            }
        }
        tiles[root].acc_r = tiles[root].acc_r + src.r * weight;
        tiles[root].acc_g = tiles[root].acc_g + src.g * weight;
        tiles[root].acc_b = tiles[root].acc_b + src.b * weight;
        // The tile's own first moments carry straight over, so no pixel
        // anywhere can move the source's position on its own.
        tiles[root].cen_x = tiles[root].cen_x + tl.fx;
        tiles[root].cen_y = tiles[root].cen_y + tl.fy;
        tiles[root].cen_w = tiles[root].cen_w + tl.fsum;
        if (tl.fsum > 0.0) {
            let mx = tl.fx / tl.fsum;
            let my = tl.fy / tl.fsum;
            tiles[root].spr_x = tiles[root].spr_x + tl.fsum * mx * mx;
            tiles[root].spr_y = tiles[root].spr_y + tl.fsum * my * my;
        }
    }
    // The patches with the most light, brightest first, ties to the lower
    // first cell, in one pass: each patch slides into a running list one
    // longer than the lights kept, whose last entry is then the strongest
    // patch that missed the cut, which every kept light fades against
    // (lumit_core `cut_fade`). `var` (not `let`) for the dynamically indexed
    // arrays: lavapipe's shader compiler crashes on dynamically indexed `let`
    // arrays.
    var top_t: array<u32, 17>;
    var top_f: array<f32, 17>;
    var count = 0u;
    for (var t = 0u; t < tile_count; t = t + 1u) {
        if (tiles[t].parent != t || !lit(t)) {
            continue;
        }
        let flux = tiles[t].cen_w;
        if (count == MAX_SOURCES + 1u && !(flux > top_f[MAX_SOURCES])) {
            continue;
        }
        // Strictly brighter moves ahead, so an equal patch found later stays
        // behind the one found first.
        var at = min(count, MAX_SOURCES);
        loop {
            if (at == 0u || !(flux > top_f[at - 1u])) {
                break;
            }
            if (at < MAX_SOURCES + 1u) {
                top_t[at] = top_t[at - 1u];
                top_f[at] = top_f[at - 1u];
            }
            at = at - 1u;
        }
        top_t[at] = t;
        top_f[at] = flux;
        count = min(count + 1u, MAX_SOURCES + 1u);
    }
    let picked_count = min(count, MAX_SOURCES);
    var cut = 0.0;
    if (count > MAX_SOURCES) {
        cut = top_f[MAX_SOURCES];
    }

    // Zero every slot first: the trace dispatches all MAX_SOURCES of them and
    // a dead slot must carry no weight.
    for (var k = 0u; k < MAX_SOURCES; k = k + 1u) {
        var dead: Light;
        dead.pos_x = 0.0;
        dead.pos_y = 0.0;
        dead.r = 0.0;
        dead.g = 0.0;
        dead.b = 0.0;
        dead.ext_x = 0.0;
        dead.ext_y = 0.0;
        dead._pad2 = 0.0;
        lights[k] = dead;
    }

    // Each source becomes ONE light carrying its measured half-extent.
    // Earlier this expanded an area source into a grid of up to 5x5
    // point lights and ran the whole ray pipeline once per sample: 25x the
    // rays, and wherever a ghost was smaller than the sample spacing you saw
    // that many separate copies of the aperture. The trace integrates the
    // extent per ray now, so a bar-shaped practical draws one bar-shaped ghost
    // at a point source's cost.
    for (var k = 0u; k < picked_count; k = k + 1u) {
        // Where the light IS, is the centre of its light — not
        // whichever pixel happened to be brightest this frame. On footage
        // that pixel wanders with sensor noise and specular sparkle, and the
        // whole flare jittered with it.
        let src = tiles[top_t[k]];
        var px = f32(src.index % dp.w);
        var py = f32(src.index / dp.w);
        var ex = 0.0;
        var ey = 0.0;
        if (src.cen_w > 0.0) {
            px = src.cen_x / src.cen_w;
            py = src.cen_y / src.cen_w;
            // Half-extent as the standard deviation of the flux about that
            // centre: zero for a point, the real width for a practical.
            ex = sqrt(max(src.spr_x / src.cen_w - px * px, 0.0)) / f32(dp.w);
            ey = sqrt(max(src.spr_y / src.cen_w - py * py, 0.0)) / f32(dp.h);
        }
        var out: Light;
        out.pos_x = (px + 0.5) / f32(dp.w);
        out.pos_y = (py + 0.5) / f32(dp.h);
        let fade = cut_fade(src.cen_w, cut);
        out.r = src.acc_r * fade * dp.tint_r;
        out.g = src.acc_g * fade * dp.tint_g;
        out.b = src.acc_b * fade * dp.tint_b;
        out.ext_x = ex;
        out.ext_y = ey;
        out._pad2 = 0.0;
        lights[k] = out;
    }
}
