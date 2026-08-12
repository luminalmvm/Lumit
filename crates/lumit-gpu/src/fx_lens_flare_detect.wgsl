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
//   detect_pick  — one thread: top-MAX_LIGHTS anchor cells by luma (ties
//                  to the lower cell index), Chebyshev suppression radius
//                  2, each gated by the soft threshold; then every gated
//                  tile's flux accumulates onto its nearest anchor
//                  (K-267 area sources); dead slots zeroed.

struct Tile {
    luma: f32,
    index: u32,
};

struct Light {
    pos_x: f32,
    pos_y: f32,
    r: f32,
    g: f32,
    b: f32,
    _pad0: f32,
    _pad1: f32,
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
    _pad0: f32,
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
const MAX_LIGHTS: u32 = 16u;
const SUPPRESS_TILES: i32 = 2;

var<workgroup> partial_luma: array<f32, 64>;
var<workgroup> partial_index: array<u32, 64>;

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
            let c = textureLoad(matte_tex, vec2<i32>(i32(x), i32(y)), 0);
            let luma = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
            let index = y * dp.w + x;
            if (luma > best_luma || (luma == best_luma && index < best_index)) {
                best_luma = luma;
                best_index = index;
            }
        }
    }
    partial_luma[li] = best_luma;
    partial_index[li] = best_index;
    workgroupBarrier();
    if (li == 0u) {
        var luma = -1.0;
        var index = 0u;
        for (var i = 0u; i < 64u; i = i + 1u) {
            let pl = partial_luma[i];
            let pi = partial_index[i];
            if (pl > luma || (pl == luma && pi < index)) {
                luma = pl;
                index = pi;
            }
        }
        var t: Tile;
        t.luma = luma;
        t.index = index;
        tiles[wg.y * dp.tiles_x + wg.x] = t;
    }
}

// The soft gate (== lens_flare::threshold_gate).
fn gate(luma: f32) -> f32 {
    if (dp.softness <= 0.0) {
        return f32(luma >= dp.threshold);
    }
    let t = clamp(
        (luma - (dp.threshold - dp.softness)) / (2.0 * dp.softness),
        0.0,
        1.0,
    );
    return t * t * (3.0 - 2.0 * t);
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
    // The flux-weighted centroid of each anchor's contributing tiles, as
    // (sum w*x, sum w*y, sum w) — see where it is applied at the bottom.
    var cen_x: array<f32, 16>;
    var cen_y: array<f32, 16>;
    var cen_w: array<f32, 16>;
    var picked_count = 0u;
    // Anchor picks: top-K by luma with Chebyshev suppression.
    for (var k = 0u; k < MAX_LIGHTS; k = k + 1u) {
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
            let px = tl.index % dp.w;
            let py = tl.index / dp.w;
            let c = textureLoad(matte_tex, vec2<i32>(i32(px), i32(py)), 0);
            var src = vec3<f32>(1.0, 1.0, 1.0);
            if (dp.use_source_colour == 1u) {
                src = max(c.rgb, vec3<f32>(0.0));
            }
            acc_r[nearest] = acc_r[nearest] + src.r * weight;
            acc_g[nearest] = acc_g[nearest] + src.g * weight;
            acc_b[nearest] = acc_b[nearest] + src.b * weight;
            let flux = tl.luma * weight;
            cen_x[nearest] = cen_x[nearest] + f32(px) * flux;
            cen_y[nearest] = cen_y[nearest] + f32(py) * flux;
            cen_w[nearest] = cen_w[nearest] + flux;
        }
    }
    for (var k = 0u; k < MAX_LIGHTS; k = k + 1u) {
        var out: Light;
        out.pos_x = 0.0;
        out.pos_y = 0.0;
        out.r = 0.0;
        out.g = 0.0;
        out.b = 0.0;
        out._pad0 = 0.0;
        out._pad1 = 0.0;
        out._pad2 = 0.0;
        if (k < picked_count) {
            // Where the light IS, is the centre of its light (K-354) — not
            // whichever pixel happened to be brightest this frame. On footage
            // that pixel wanders with sensor noise and specular sparkle, and
            // the whole flare jitters with it. A one-tile point source has
            // only its own pixel to average, so point lights are unchanged.
            var px = f32(anchor_px[k]);
            var py = f32(anchor_py[k]);
            if (cen_w[k] > 0.0) {
                px = cen_x[k] / cen_w[k];
                py = cen_y[k] / cen_w[k];
            }
            out.pos_x = (px + 0.5) / f32(dp.w);
            out.pos_y = (py + 0.5) / f32(dp.h);
            out.r = acc_r[k] * dp.tint_r;
            out.g = acc_g[k] * dp.tint_g;
            out.b = acc_b[k] * dp.tint_b;
        }
        lights[k] = out;
    }
}
