// Volumetric light (lumit_core::haze, which is the oracle and carries the
// explanation). Compiled after the noise core and fx_lighting.wgsl, whose
// lights, casters and `transmitted` it reads through the same uniform `p`.
//
// haze_march runs at half size: each texel walks its ray from the eye to the
// scene's depth and keeps the light scattered towards the eye and how much of
// what is behind is left. haze_apply lays that over the full-size picture,
// reading the sixteen nearest half-size texels weighted by how close their
// depth is to this pixel's.

const SCATTER_PER_PX: f32 = 0.0025;
const EXTINCTION_PER_PX: f32 = 0.000025;
const PARALLEL_SHARE: f32 = 0.02;
const POINT_SHARE: f32 = 0.2;
const FALLOFF_PX: f32 = 100.0;
const MAX_SCATTER: f32 = 0.95;

struct Haze {
    // xyz = the eye, w = the zoom.
    eye: vec4<f32>,
    // The camera's right axis and the comp's width, its axis down the
    // picture and the comp's height.
    right: vec4<f32>,
    up: vec4<f32>,
    // The way it looks, w = raster pixels per comp pixel.
    forward: vec4<f32>,
    // The sphere the lit haze is kept inside: centre, radius.
    sphere: vec4<f32>,
    // x = the clip w the scene depth is counted from, y = nought, z = how
    // many steps a ray takes.
    view: vec4<f32>,
    // Per light: Volumetric, Haze density, Forward scatter, Noise, percent.
    look: array<vec4<f32>, 8>,
    // Per light: Noise size and how far the noise has drifted, comp pixels,
    // then Height falloff, percent.
    grain: array<vec4<f32>, 8>,
};

// The scene's depth at full size: clip w less `view.x`.
@group(0) @binding(1) var depth: texture_2d<f32>;
@group(0) @binding(6) var<uniform> h: Haze;
// haze_march's half-size result, read by haze_apply.
@group(0) @binding(7) var scatter: texture_2d<f32>;

// As lumit_core::haze::jitter.
fn haze_jitter(x: u32, y: u32) -> f32 {
    return fract(52.982918 * fract(0.06711056 * f32(x) + 0.00583715 * f32(y)));
}

// As lumit_core::haze::extinction.
fn haze_extinction(live: u32) -> f32 {
    var s: f32 = 0.0;
    for (var i: u32 = 0u; i < live; i = i + 1u) {
        let k = h.look[i];
        s = max(s, k.y / 100.0 * EXTINCTION_PER_PX * min(k.x / 100.0, 1.0));
    }
    return s;
}

// As lumit_core::haze::thickness, with the ground through the comp's middle.
fn thickness(pos: vec3<f32>, i: u32) -> f32 {
    let k = h.look[i];
    let q = h.grain[i];
    let above = max(h.up.w * 0.5 - pos.y, 0.0);
    let height = exp2(-q.z / 100.0 * above / FALLOFF_PX);
    if (k.w <= 0.0 || q.x <= 0.0) {
        return height;
    }
    let s = 1.0 / q.x;
    let x = (pos.x + q.y) * s;
    let y = (pos.y - q.y * 0.25) * s;
    let z = (pos.z + q.y * 0.5) * s;
    // No loop. Read from the uniform, as a literal nought lets one
    // compiler fold the `%` the core guards and refuse it.
    let cycle = i32(h.view.y);
    let n = nc_perlin3(0u, 0u, x, y, z, cycle)
        + 0.5 * nc_perlin3(0u, 1u, x * 2.0, y * 2.0, z * 2.0, cycle);
    return height * max(1.0 + k.w / 100.0 * n, 0.0);
}

// One step of lumit_core::haze::march: what every showing light scatters
// towards the eye at step `i`, before the step's length.
fn haze_step(i: u32, jit: f32, t0: f32, dt: f32, d: vec3<f32>, dl: f32, dir: vec3<f32>,
             sigma: f32, live: u32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let t = t0 + (f32(i) + jit) * dt;
    let pos = h.eye.xyz + d * t;
    let left = exp(-sigma * (t - t0) * dl);
    for (var j: u32 = 0u; j < live; j = j + 1u) {
        let k = h.look[j];
        if (k.x <= 0.0) {
            continue;
        }
        let l = p.lights[j];
        var to_light = vec3<f32>(0.0);
        let strength = arrival(pos, l, &to_light);
        if (strength <= 0.0) {
            continue;
        }
        var glow: f32 = 1.0;
        if (u32(l.flags.x + 0.5) == 2u) {
            glow = PARALLEL_SHARE;
        } else {
            if (u32(l.flags.y + 0.5) == 0u) {
                let r = max(l.flags.z, 1.0);
                let far = length(to_light);
                glow = r * r / (r * r + far * far);
            }
            if (l.axis.w < -1.0) {
                glow = glow * glow * POINT_SHARE;
            }
        }
        let g = clamp(k.z / 100.0, -MAX_SCATTER, MAX_SCATTER);
        let cos_t = dot(normalise(to_light), dir);
        let phase = (1.0 - g * g) / pow(max(1.0 + g * g - 2.0 * g * cos_t, 1e-6), 1.5);
        var through = vec3<f32>(1.0);
        if (l.shadow.x > 0.0) {
            through = 1.0 - l.shadow.x * (1.0 - transmitted(pos, l));
        }
        let amount = left * k.x / 100.0 * k.y / 100.0 * SCATTER_PER_PX
            * thickness(pos, j) * strength * glow * phase;
        sum = sum + amount * l.colour.rgb * through;
    }
    return sum;
}

// As lumit_core::haze::march.
fn haze_ray(q: vec2<f32>, end: f32, jit: f32) -> vec4<f32> {
    var out = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    let d = h.right.xyz * (q.x - h.right.w * 0.5)
        + h.up.xyz * (q.y - h.up.w * 0.5)
        + h.forward.xyz * h.eye.w;
    let dl = length(d);
    let dir = normalise(d);
    if (all(dir == vec3<f32>(0.0))) {
        return out;
    }
    let oc = h.eye.xyz - h.sphere.xyz;
    let a = dl * dl;
    let b = dot(oc, d);
    let c = dot(oc, oc) - h.sphere.w * h.sphere.w;
    let disc = b * b - a * c;
    if (disc <= 0.0) {
        return out;
    }
    let s = sqrt(disc);
    let t0 = max((-b - s) / a, 0.0);
    let t1 = min((-b + s) / a, end);
    if (t1 <= t0) {
        return out;
    }
    let live = min(u32(p.normal.w), MAX_LIT_LIGHTS);
    let sigma = haze_extinction(live);
    // The step count comes in the uniform, a multiple of four. A constant
    // one gets every step unrolled, which takes one compiler most of a
    // minute; none unrolled leaves each shadow read waiting on the last, many
    // times slower. Four at a time is both quick to make and quick to run.
    let steps = u32(h.view.z);
    let dt = (t1 - t0) / f32(steps);
    var sum = vec3<f32>(0.0);
    for (var i: u32 = 0u; i < steps; i = i + 4u) {
        sum = sum + haze_step(i, jit, t0, dt, d, dl, dir, sigma, live)
            + haze_step(i + 1u, jit, t0, dt, d, dl, dir, sigma, live)
            + haze_step(i + 2u, jit, t0, dt, d, dl, dir, sigma, live)
            + haze_step(i + 3u, jit, t0, dt, d, dl, dir, sigma, live);
    }
    out = vec4<f32>(sum * dt * dl, exp(-sigma * (t1 - t0) * dl));
    return out;
}

@compute @workgroup_size(8, 8)
fn haze_march(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(dst);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let full = vec2<i32>(textureDimensions(depth));
    let at = min(vec2<i32>(gid.xy) * 2, full - vec2<i32>(1));
    let q = (vec2<f32>(at) + vec2<f32>(0.5)) / max(h.forward.w, 1e-6);
    let end = textureLoad(depth, at, 0).r + h.view.x;
    textureStore(dst, vec2<i32>(gid.xy), haze_ray(q, end, haze_jitter(gid.x, gid.y)));
}

@compute @workgroup_size(8, 8)
fn haze_apply(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let half = vec2<i32>(textureDimensions(scatter));
    let z = textureLoad(depth, xy, 0).r;
    let tol = 0.02 * (z + h.view.x) + 1e-4;
    let hp = (vec2<f32>(xy) + vec2<f32>(0.5)) * 0.5 - vec2<f32>(0.5);
    let base = vec2<i32>(floor(hp)) - vec2<i32>(1);
    var sum = vec4<f32>(0.0);
    var weight: f32 = 0.0;
    var best = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    var nearest: f32 = 1e30;
    // A tent over the sixteen nearest half-size texels, each weighted by how
    // near its depth is to this pixel's, so the steps' dither blurs into air
    // and an edge keeps its haze on the right side.
    for (var j: i32 = 0; j < 16; j = j + 1) {
        let at = base + vec2<i32>(j & 3, j >> 2u);
        let c = clamp(at, vec2<i32>(0), half - vec2<i32>(1));
        let s = textureLoad(scatter, c, 0);
        let dz = abs(textureLoad(depth, min(c * 2, size - vec2<i32>(1)), 0).r - z);
        let tent = max(vec2<f32>(2.0) - abs(vec2<f32>(at) - hp), vec2<f32>(0.0));
        let w = tent.x * tent.y / (1.0 + (dz / tol) * (dz / tol));
        sum = sum + s * w;
        weight = weight + w;
        if (dz < nearest) {
            nearest = dz;
            best = s;
        }
    }
    var s = best;
    if (weight > 1e-4) {
        s = sum / weight;
    }
    // The light the haze adds covers what is behind as much as it is bright,
    // so it shows over transparency and leaves a precomp's alpha honest.
    let cover = max(s.r, max(s.g, s.b));
    textureStore(dst, xy, vec4<f32>(o.rgb * s.a + s.rgb, min(o.a + cover, 1.0)));
}
