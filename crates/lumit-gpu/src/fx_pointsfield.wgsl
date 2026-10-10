// Points field: every pixel finds the point nearest to it and draws something
// from that point. Mirrors lumit_core's PointsField::draw, which is the oracle.
//
// The search is jump flooding. Each point writes its own number into the pixel
// it sits on (pf_seed). Then a few passes (pf_jump) let every pixel ask its
// eight neighbours a set distance away which point they know of, and keep the
// nearest. The distance halves each pass, so about log2 of the frame's longer
// side passes reach every pixel, however many points there are. The last pass
// (pf_resolve) turns the winner into a colour.
//
// A pixel compares where the points really are, not the pixels they were
// written into, so the distances that come out are exact for whichever point
// won. Jump flooding does now and then settle on a point that is a shade
// further than the true nearest, along the edge between two points' cells.
// The host asks the pixels next door a second time at the end, which mends
// most of those.

struct Params {
    step: i32,       // how far away this jump asks, raster px
    count: u32,      // how many seeds the buffer holds
    output: u32,     // 0 distance, 1 direction, 2 colour, 3 number
    flags: u32,      // bit 0 invert, bit 1 limit to radius
    radius: f32,     // raster px
    range: f32,      // the number that draws as white
    mix_amt: f32,    // 0..1, blended against the input
    _pad: f32,
};

struct Seed {
    at: vec2<f32>,   // where the point is seen, raster px
    number: f32,
    _pad: f32,
    colour: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> seeds: array<Seed>;
// Which seed each pixel knows of so far, plus one. Nought is none.
@group(0) @binding(2) var near_in: texture_2d<u32>;
@group(0) @binding(3) var near_out: texture_storage_2d<r32uint, write>;
@group(0) @binding(4) var src: texture_2d<f32>;
@group(0) @binding(5) var dst: texture_storage_2d<rgba16float, write>;

// One thread a seed. A point off the frame is written into the nearest pixel
// on its edge, so it still takes part. The host sends one seed a pixel, so no
// two threads write the same place.
@compute @workgroup_size(64)
fn pf_seed(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.count) {
        return;
    }
    let size = vec2<f32>(textureDimensions(near_in));
    let at = clamp(floor(seeds[gid.x].at), vec2<f32>(0.0), size - vec2<f32>(1.0));
    textureStore(near_out, vec2<i32>(at), vec4<u32>(gid.x + 1u, 0u, 0u, 0u));
}

@compute @workgroup_size(8, 8)
fn pf_jump(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(near_in));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let centre = vec2<f32>(xy) + vec2<f32>(0.5);
    var best = 0u;
    var best_d = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            // A neighbour past the edge reads the edge pixel, which is as
            // good a pixel to ask as any.
            let q = clamp(xy + vec2<i32>(i, j) * p.step, vec2<i32>(0), size - vec2<i32>(1));
            let k = textureLoad(near_in, q, 0).r;
            if (k != 0u) {
                let v = seeds[k - 1u].at - centre;
                let d = dot(v, v);
                // The earlier seed wins a tie, as it does in the oracle.
                if (best == 0u || d < best_d || (d == best_d && k < best)) {
                    best = k;
                    best_d = d;
                }
            }
        }
    }
    textureStore(near_out, xy, vec4<u32>(best, 0u, 0u, 0u));
}

// == PointsField::shade.
fn shade(centre: vec2<f32>, s: Seed) -> vec4<f32> {
    let v = s.at - centre;
    let d = sqrt(v.x * v.x + v.y * v.y);
    let radius = max(p.radius, 0.0);
    if (p.output == 1u) {
        // Displacement map's reading: 0.5 is no push.
        var u = vec2<f32>(0.0);
        if (d > 0.0) {
            u = v / d;
        }
        return vec4<f32>(0.5 + 0.5 * u.x, 0.5 + 0.5 * u.y, 0.5, 1.0);
    }
    if (p.output == 2u || p.output == 3u) {
        if ((p.flags & 2u) != 0u && d > radius) {
            return vec4<f32>(0.0);
        }
        if (p.output == 2u) {
            return s.colour;
        }
        let n = s.number / max(p.range, 1e-6);
        return vec4<f32>(n, n, n, 1.0);
    }
    var g = 0.0;
    if (radius > 0.0) {
        g = clamp(1.0 - d / radius, 0.0, 1.0);
    }
    if ((p.flags & 1u) != 0u) {
        g = 1.0 - g;
    }
    return vec4<f32>(g, g, g, 1.0);
}

@compute @workgroup_size(8, 8)
fn pf_resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let k = textureLoad(near_in, xy, 0).r;
    if (k == 0u) {
        textureStore(dst, xy, o);
        return;
    }
    let f = shade(vec2<f32>(xy) + vec2<f32>(0.5), seeds[k - 1u]);
    textureStore(dst, xy, o + (f - o) * p.mix_amt);
}
