// Lens flare Matte-mode source detection (docs/08 §3.27, docs/impl/
// lens-flare.md §6, K-257): find the brightest sources in the matte layer's
// rendered picture and turn them into the frame's flare lights. Mirrors
// lumit_core::fx::lens_flare::detect_lights op-for-op — the CPU function is
// the oracle — and is deterministic by construction: the tile reduction
// merges fixed-order partials, and the top-K pass runs on a single thread.
//
//   detect_tiles — one workgroup per DETECT_TILE-sided cell: each thread
//                  scans a strided share of the cell's pixels for the
//                  brightest (Rec. 709 luma; ties to the lowest linear
//                  index), thread 0 merges the partials in thread order.
//   detect_pick  — one thread: top-MAX_SOURCES anchor cells by luma (ties
//                  to the lower cell index), Chebyshev suppression radius
//                  2, each gated by the soft threshold; then every gated
//                  tile's flux accumulates onto its nearest anchor
//                  (K-267 area sources); dead slots zeroed.

// What one tile knows about the light inside it — the WGSL twin of
// lumit_core's `TileStat` (K-355). Through K-354 this was the brightest pixel
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
    _pad: f32,
};

// Shares its layout with fx_lens_flare_trace.wgsl: position and half-extent
// as raster fractions, colour already gated. The extent is what the trace
// integrates over per ray (K-367).
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
    // the tint alone (K-259).
    use_source_colour: u32,
    // 1 = read the matte inverted (1 - rgb), the Matte row's Invert (K-395);
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

const DETECT_TILE: u32 = 32u;
// Distinct sources detection may find, and so the light slots the trace
// carries — one per source however large it is (K-367 dropped K-355's
// expansion into up to 5x5 sample slots). Mirrors lumit_core's MAX_SOURCES.
const MAX_SOURCES: u32 = 16u;
const SUPPRESS_TILES: i32 = 2;

// The soft gate (== lens_flare::threshold_gate, K-363): one-sided — closed
// at and below the threshold, fully open a softness above it. Declared
// before its first use, which WGSL requires.
// The Matte row's Invert, at the one place the matte is read (K-395): the
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
            // Every lit pixel contributes, not just the brightest (K-355).
            let g = gate(luma);
            if (g > 0.0) {
                let f = luma * g;
                wsum = wsum + g;
                csum = csum + max(c, vec3<f32>(0.0)) * g;
                fsum = fsum + f;
                fx = fx + f32(x) * f;
                fy = fy + f32(y) * f;
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
    workgroupBarrier();
    if (li == 0u) {
        var luma = -1.0;
        var index = 0u;
        var w_all = 0.0;
        var c_all = vec3<f32>(0.0);
        var f_all = 0.0;
        var fx_all = 0.0;
        var fy_all = 0.0;
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
        t._pad = 0.0;
        tiles[wg.y * dp.tiles_x + wg.x] = t;
    }
}

@compute @workgroup_size(1)
fn detect_pick(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x != 0u) {
        return;
    }
    let tile_count = dp.tiles_x * dp.tiles_y;
    // Suppression flags, one bit per tile, in registers via a fixed array —
    // tile counts stay small (a 4K frame at 32 px tiles is 4080), but WGSL
    // wants a bound: suppress by re-checking distance to already-picked
    // tiles instead of a flag array, which needs only the picks.
    // `var` (not `let`) for every dynamically indexed array: lavapipe's
    // shader compiler crashes on dynamically indexed `let` arrays (K-264).
    var picked_x: array<i32, 16>;
    var picked_y: array<i32, 16>;
    var anchor_px: array<u32, 16>;
    var anchor_py: array<u32, 16>;
    var acc_r: array<f32, 16>;
    var acc_g: array<f32, 16>;
    var acc_b: array<f32, 16>;
    // The flux centroid of each source, as (sum f*x, sum f*y, sum f), and the
    // second moments that give it its EXTENT — see the bottom of this pass.
    var cen_x: array<f32, 16>;
    var cen_y: array<f32, 16>;
    var cen_w: array<f32, 16>;
    var m2_x: array<f32, 16>;
    var m2_y: array<f32, 16>;
    var picked_count = 0u;
    // Anchor picks: top-K by luma with Chebyshev suppression.
    for (var k = 0u; k < MAX_SOURCES; k = k + 1u) {
        var best_tile = 0u;
        var best_luma = -1.0;
        for (var t = 0u; t < tile_count; t = t + 1u) {
            let tl = tiles[t];
            if (tl.luma <= 0.0) {
                continue;
            }
            // Chebyshev suppression against every earlier pick.
            let tx = i32(t % dp.tiles_x);
            let ty = i32(t / dp.tiles_x);
            var suppressed = false;
            for (var p = 0u; p < picked_count; p = p + 1u) {
                if (abs(tx - picked_x[p]) <= SUPPRESS_TILES
                    && abs(ty - picked_y[p]) <= SUPPRESS_TILES) {
                    suppressed = true;
                }
            }
            if (suppressed) {
                continue;
            }
            if (tl.luma > best_luma) {
                best_luma = tl.luma;
                best_tile = t;
            }
        }
        if (best_luma > 0.0 && gate(best_luma) > 0.0) {
            let idx = tiles[best_tile].index;
            anchor_px[picked_count] = idx % dp.w;
            anchor_py[picked_count] = idx / dp.w;
            picked_x[picked_count] = i32(best_tile % dp.tiles_x);
            picked_y[picked_count] = i32(best_tile / dp.tiles_x);
            acc_r[picked_count] = 0.0;
            acc_g[picked_count] = 0.0;
            acc_b[picked_count] = 0.0;
            cen_x[picked_count] = 0.0;
            cen_y[picked_count] = 0.0;
            cen_w[picked_count] = 0.0;
            m2_x[picked_count] = 0.0;
            m2_y[picked_count] = 0.0;
            picked_count = picked_count + 1u;
        }
    }
    // Area sources (K-267): every gated tile's flux — its brightest pixel's
    // colour (or white) times its gate — lands on the NEAREST anchor
    // (Chebyshev; ties to the lowest anchor index), tile order fixed so the
    // sum matches the CPU reference op-for-op. A one-tile point source is
    // its own anchor's only contributor and reads exactly as before.
    if (picked_count > 0u) {
        for (var t = 0u; t < tile_count; t = t + 1u) {
            let tl = tiles[t];
            if (tl.luma <= 0.0) {
                continue;
            }
            let weight = gate(tl.luma);
            if (weight <= 0.0) {
                continue;
            }
            let tx = i32(t % dp.tiles_x);
            let ty = i32(t / dp.tiles_x);
            var nearest = 0u;
            var nearest_d = 2147483647;
            for (var p = 0u; p < picked_count; p = p + 1u) {
                let d = max(abs(tx - picked_x[p]), abs(ty - picked_y[p]));
                if (d < nearest_d) {
                    nearest_d = d;
                    nearest = p;
                }
            }
            // The tile's MEAN colour over its lit pixels, not its brightest
            // pixel's (K-355): one sparkle among a thousand lit pixels now
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
            acc_r[nearest] = acc_r[nearest] + src.r * weight;
            acc_g[nearest] = acc_g[nearest] + src.g * weight;
            acc_b[nearest] = acc_b[nearest] + src.b * weight;
            // The tile's own first moments carry straight over, so no pixel
            // anywhere can move the source's position on its own.
            cen_x[nearest] = cen_x[nearest] + tl.fx;
            cen_y[nearest] = cen_y[nearest] + tl.fy;
            cen_w[nearest] = cen_w[nearest] + tl.fsum;
            if (tl.fsum > 0.0) {
                let mx = tl.fx / tl.fsum;
                let my = tl.fy / tl.fsum;
                m2_x[nearest] = m2_x[nearest] + tl.fsum * mx * mx;
                m2_y[nearest] = m2_y[nearest] + tl.fsum * my * my;
            }
        }
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

    // Each source becomes ONE light carrying its measured half-extent (K-367).
    // Through K-355 this expanded an area source into a grid of up to 5x5
    // point lights and ran the whole ray pipeline once per sample: 25x the
    // rays, and wherever a ghost was smaller than the sample spacing you saw
    // that many separate copies of the aperture. The trace integrates the
    // extent per ray now, so a bar-shaped practical draws one bar-shaped ghost
    // at a point source's cost.
    for (var k = 0u; k < picked_count; k = k + 1u) {
        // Where the light IS, is the centre of its light (K-354, K-355) — not
        // whichever pixel happened to be brightest this frame. On footage
        // that pixel wanders with sensor noise and specular sparkle, and the
        // whole flare jittered with it.
        var px = f32(anchor_px[k]);
        var py = f32(anchor_py[k]);
        var ex = 0.0;
        var ey = 0.0;
        if (cen_w[k] > 0.0) {
            px = cen_x[k] / cen_w[k];
            py = cen_y[k] / cen_w[k];
            // Half-extent as the standard deviation of the flux about that
            // centre: zero for a point, the real width for a practical.
            ex = sqrt(max(m2_x[k] / cen_w[k] - px * px, 0.0)) / f32(dp.w);
            ey = sqrt(max(m2_y[k] / cen_w[k] - py * py, 0.0)) / f32(dp.h);
        }
        var out: Light;
        out.pos_x = (px + 0.5) / f32(dp.w);
        out.pos_y = (py + 0.5) / f32(dp.h);
        out.r = acc_r[k] * dp.tint_r;
        out.g = acc_g[k] * dp.tint_g;
        out.b = acc_b[k] * dp.tint_b;
        out.ext_x = ex;
        out.ext_y = ey;
        out._pad2 = 0.0;
        lights[k] = out;
    }
}
