// Grid's Lines type: a line grid on its own plane, seen through its own
// camera. Mirrors lumit_core::fx::effects::grid_lines op for op (the CPU is
// the oracle).
//
// Each pixel's ray is followed onto the grid's plane, and so are its right
// and lower neighbours', which says how far one pixel reaches on the plane.
// A line's share of the pixel is then a band against a one-pixel box, so
// lines stay smooth at any tilt, and cells only a pixel or two across fade to
// the share of a cell the lines cover instead of shimmering.
//
// floor(x + 0.5), not round(): Rust rounds halves away from nought and WGSL
// rounds them to even.

struct Params {
    centre_focal_dist: vec4<f32>, // Position.xy, focal length, camera distance
    axis_u: vec4<f32>,            // the grid's across, in view space
    axis_v: vec4<f32>,            // its down
    axis_n: vec4<f32>,            // its outward normal
    u: vec4<f32>,                 // up and down lines: cell, origin, half width, reach
    v: vec4<f32>,                 // across lines: the same
    ends: vec4<i32>,              // u last, u middle, v last, v middle; last -1 is endless
    colour: vec4<f32>,            // premultiplied
    major_width: f32,
    mix_amt: f32,
    major_every: i32,
    _pad: i32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The share of a pixel at x the band -half..half covers, both in pixels.
fn span(x: f32, half: f32) -> f32 {
    return clamp(min(x + 0.5, half) - max(x - 0.5, -half), 0.0, 1.0);
}

// Where the ray through raster point s meets the plane, as (across, down,
// 1), or z = 0 past the horizon.
fn plane_at(s: vec2<f32>) -> vec3<f32> {
    let ray = vec3<f32>(s - p.centre_focal_dist.xy, p.centre_focal_dist.z);
    let dist = p.centre_focal_dist.w;
    let n = p.axis_n.xyz;
    let t = dist * n.z / dot(n, ray);
    if (!(t > 0.0) || t > 3.0e38) {
        return vec3<f32>(0.0);
    }
    let hit = vec3<f32>(t * ray.x, t * ray.y, t * ray.z - dist);
    return vec3<f32>(dot(p.axis_u.xyz, hit), dot(p.axis_v.xyz, hit), 1.0);
}

fn family(axis: vec4<f32>, last: i32, middle: i32, along: f32, step: f32, across: f32, step_across: f32) -> f32 {
    let cell = axis.x;
    let origin = axis.y;
    let half0 = axis.z;
    let reach = axis.w;
    if (!(cell > 0.0 && half0 > 0.0 && step > 0.0)) {
        return 0.0;
    }
    var j = clamp(floor((along - origin) / cell + 0.5), -1.0e6, 1.0e6);
    if (last >= 0) {
        j = clamp(j, 0.0, f32(last));
    }
    let major = p.major_every > 0 && (i32(j) - middle) % p.major_every == 0;
    var half = half0;
    if (major) {
        half = half0 * p.major_width;
    }
    let near = span((along - origin - j * cell) / step, half / step);
    var thicker = 1.0;
    if (p.major_every > 0) {
        thicker = (f32(p.major_every) - 1.0 + p.major_width) / f32(p.major_every);
    }
    var share = min(2.0 * half0 * thicker / cell, 1.0);
    // A grid with ends only has that tone between them.
    if (last >= 0) {
        let run = 0.5 * f32(last) * cell;
        let s = min(step, cell);
        share = share * span((along - origin - run) / s, (run + half0) / s);
    }
    let t = clamp((cell / step - 1.0) / 2.0, 0.0, 1.0);
    let cover = share + (near - share) * (t * t * (3.0 - 2.0 * t));
    if (reach >= 0.0 && step_across > 0.0) {
        return cover * span(across / step_across, reach / step_across);
    }
    return cover;
}

fn end_step(step: f32, cell: f32) -> f32 {
    if (cell > 0.0) {
        return min(step, cell);
    }
    return step;
}

@compute @workgroup_size(8, 8)
fn grid_lines(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let s = vec2<f32>(xy) + vec2<f32>(0.5);
    let here = plane_at(s);
    let right = plane_at(s + vec2<f32>(1.0, 0.0));
    let below = plane_at(s + vec2<f32>(0.0, 1.0));
    var k = 0.0;
    if (here.z > 0.0 && right.z > 0.0 && below.z > 0.0) {
        let du = length(vec2<f32>(right.x - here.x, below.x - here.x));
        let dv = length(vec2<f32>(right.y - here.y, below.y - here.y));
        // An end is softened over a cell at most, or the horizon smears it.
        let a = family(p.u, p.ends.x, p.ends.y, here.x, du, here.y, end_step(dv, p.v.x));
        let b = family(p.v, p.ends.z, p.ends.w, here.y, dv, here.x, end_step(du, p.u.x));
        k = max(a, b);
    }
    let over = p.colour * k + o * (1.0 - p.colour.a * k);
    textureStore(dst, xy, o + (over - o) * p.mix_amt);
}
