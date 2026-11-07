// An Environment light's picture made ready to light with (environment.rs):
// the panorama wrapped onto a cube, the cube's smaller levels, then each
// level of the specular cube filtered for its roughness and the irradiance
// cube for a matte surface. And the background, the cube seen through the
// camera behind everything.
//
// The filtering follows Bevy 0.17's generated environment maps
// (crates/bevy_pbr/src/light_probe/environment_filter.wgsl and the helpers
// it imports), with the per-pixel noise from its hash and never from the
// time, so a picture filters the same every time.
//
// Copyright (c) 2020 Carter Anderson. Bevy is dual-licensed under MIT or
// Apache-2.0; under MIT:
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the
// "Software"), to deal in the Software without restriction, including
// without limitation the rights to use, copy, modify, merge, publish,
// distribute, sublicense, and/or sell copies of the Software, and to permit
// persons to whom the Software is furnished to do so, subject to the
// following conditions:
//
// The above copyright notice and this permission notice shall be included
// in all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
// MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
// NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
// DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
// OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
// USE OR OTHER DEALINGS IN THE SOFTWARE.

const PI: f32 = 3.14159265;

struct Filter {
    // The level's roughness, how many samples it takes, the cube's side at
    // its first level, unused.
    roughness: f32,
    samples: u32,
    size: f32,
    pad: f32,
};

@group(0) @binding(0) var src: texture_2d_array<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var dst: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(3) var<uniform> params: Filter;

// Bevy's: a direction for a texel of a face, and the face and texel for a
// direction, in the cube's own order (+x, -x, +y, -y, +z, -z).
fn sample_cube_dir(uv: vec2<f32>, face: u32) -> vec3<f32> {
    let c = 2.0 * uv - 1.0;
    var dir: vec3<f32>;
    switch (face) {
        case 0u: { dir = vec3<f32>(1.0, -c.y, -c.x); }
        case 1u: { dir = vec3<f32>(-1.0, -c.y, c.x); }
        case 2u: { dir = vec3<f32>(c.x, 1.0, c.y); }
        case 3u: { dir = vec3<f32>(c.x, -1.0, -c.y); }
        case 4u: { dir = vec3<f32>(c.x, -c.y, 1.0); }
        default: { dir = vec3<f32>(-c.x, -c.y, -1.0); }
    }
    return normalize(dir);
}

struct CubeUv {
    uv: vec2<f32>,
    face: u32,
};

fn dir_to_cube_uv(dir: vec3<f32>) -> CubeUv {
    let a = abs(dir);
    var face = 0u;
    var uv = vec2<f32>(0.0);
    if (a.x >= a.y && a.x >= a.z) {
        if (dir.x > 0.0) {
            face = 0u;
            uv = vec2<f32>(-dir.z, -dir.y) / a.x;
        } else {
            face = 1u;
            uv = vec2<f32>(dir.z, -dir.y) / a.x;
        }
    } else if (a.y >= a.z) {
        if (dir.y > 0.0) {
            face = 2u;
            uv = vec2<f32>(dir.x, dir.z) / a.y;
        } else {
            face = 3u;
            uv = vec2<f32>(dir.x, -dir.z) / a.y;
        }
    } else {
        if (dir.z > 0.0) {
            face = 4u;
            uv = vec2<f32>(dir.x, -dir.y) / a.z;
        } else {
            face = 5u;
            uv = vec2<f32>(-dir.x, -dir.y) / a.z;
        }
    }
    return CubeUv(uv * 0.5 + 0.5, face);
}

fn sample_environment(dir: vec3<f32>, level: f32) -> vec3<f32> {
    let c = dir_to_cube_uv(dir);
    return textureSampleLevel(src, samp, c.uv, c.face, level).rgb;
}

fn hammersley_2d(i: u32, n: u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), f32(reverseBits(i)) * 2.3283064365386963e-10);
}

fn rand_f(state: ptr<function, u32>) -> f32 {
    *state = *state * 747796405u + 2891336453u;
    let word = ((*state >> ((*state >> 28u) + 4u)) ^ *state) * 277803737u;
    return f32((word >> 22u) ^ word) * bitcast<f32>(0x2f800004u);
}

// Where in the sequence a texel starts, from its place alone.
fn texel_noise(coords: vec2<u32>, face: u32) -> vec2<f32> {
    var state = (coords.x * 3966231743u) ^ (coords.y * 3928936651u) ^ (face * 1199786941u);
    let a = rand_f(&state);
    return vec2<f32>(a, rand_f(&state));
}

fn d_ggx(roughness: f32, nh: f32) -> f32 {
    let a = nh * roughness;
    let k = roughness / (1.0 - nh * nh + a * a);
    return k * k / PI;
}

fn g_smith(nv: f32, nl: f32, roughness: f32) -> f32 {
    let k = roughness / 2.0;
    return nl / (nl * (1.0 - k) + k) * (nv / (nv * (1.0 - k) + k));
}

// The bounded VNDF's pdf and sample (Bevy's ggx_vndf_pdf and
// sample_visible_ggx), for a view along the normal.
fn ggx_vndf_pdf(i: vec3<f32>, nh: f32, roughness: f32) -> f32 {
    let ndf = d_ggx(roughness, nh);
    let ai = roughness * i.xy;
    let len2 = dot(ai, ai);
    let t = sqrt(len2 + i.z * i.z);
    if (i.z >= 0.0) {
        let a2 = roughness * roughness;
        let s = 1.0 + length(i.xy);
        let s2 = s * s;
        let k = (1.0 - a2) * s2 / (s2 + a2 * i.z * i.z);
        return ndf / (2.0 * (k * i.z + t));
    }
    return ndf * (t - i.z) / (2.0 * len2);
}

fn sample_visible_ggx(xi: vec2<f32>, roughness: f32, n: vec3<f32>, view: vec3<f32>) -> vec3<f32> {
    let alpha = roughness;
    let wi_n = dot(view, n);
    let wi_z = -n * wi_n;
    let wi_xy = view + wi_z;
    let wi_std = -normalize(alpha * wi_xy + wi_z);
    let wi_std_z = dot(wi_std, n);
    let s = 1.0 + length(wi_xy);
    let a = clamp(alpha, 0.0, 1.0);
    let a2 = a * a;
    let s2 = s * s;
    let k = (1.0 - a2) * s2 / (s2 + a2 * wi_n * wi_n);
    let b = select(wi_std_z, k * wi_std_z, wi_n > 0.0);
    let z = 1.0 - xi.y * (1.0 + b);
    let sin_theta = sqrt(max(0.0, 1.0 - z * z));
    let phi = 2.0 * PI * xi.x - PI;
    let c_std = vec3<f32>(sin_theta * cos(phi), sin_theta * sin(phi), z);
    let wr = n + vec3<f32>(0.0, 0.0, 1.0);
    let c = dot(wr, c_std) * wr / wr.z - c_std;
    let wm_std = c + wi_std;
    let wm_std_z = n * dot(n, wm_std);
    let wm_std_xy = wm_std_z - wm_std;
    let h = normalize(alpha * wm_std_xy + wm_std_z);
    return reflect(-view, h);
}

// Filtered importance sampling: the level whose texels are as wide as a
// sample's share of the sphere. Measured against the first level's texels,
// which the levels are counted from.
fn level_for(pdf: f32, samples: f32) -> f32 {
    let omega_s = 1.0 / (samples * pdf);
    let omega_p = 4.0 * PI / (6.0 * params.size * params.size);
    return max(0.5 * log2(omega_s / omega_p), 0.0);
}

// The panorama onto the cube's first level.
@compute @workgroup_size(8, 8, 1)
fn to_cube(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst).xy;
    if (any(id.xy >= size)) {
        return;
    }
    let d = sample_cube_dir((vec2<f32>(id.xy) + 0.5) / vec2<f32>(size), id.z);
    let rgb = max(textureSampleLevel(src, samp, panorama_uv(d), 0, 0.0).rgb, vec3<f32>(0.0));
    textureStore(dst, id.xy, id.z, vec4<f32>(rgb, 1.0));
}

// The panorama kept as it is, for the background to read at its own detail.
@compute @workgroup_size(8, 8, 1)
fn keep(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst).xy;
    if (any(id.xy >= size)) {
        return;
    }
    textureStore(dst, id.xy, 0, textureLoad(src, id.xy, 0, 0));
}

// A level from the one above, each texel the four above it.
@compute @workgroup_size(8, 8, 1)
fn down(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst).xy;
    if (any(id.xy >= size)) {
        return;
    }
    let p = vec2<i32>(id.xy) * 2;
    let f = i32(id.z);
    let sum = textureLoad(src, p, f, 0) + textureLoad(src, p + vec2<i32>(1, 0), f, 0)
        + textureLoad(src, p + vec2<i32>(0, 1), f, 0) + textureLoad(src, p + vec2<i32>(1, 1), f, 0);
    textureStore(dst, id.xy, id.z, sum * 0.25);
}

// Bevy's generate_radiance_map: one level of the specular cube.
@compute @workgroup_size(8, 8, 1)
fn radiance(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst).xy;
    if (any(id.xy >= size)) {
        return;
    }
    let n = sample_cube_dir((vec2<f32>(id.xy) + 0.5) / vec2<f32>(size), id.z);
    let roughness = max(params.roughness, 0.089) * max(params.roughness, 0.089);
    if (params.roughness < 0.01) {
        textureStore(dst, id.xy, id.z, vec4<f32>(sample_environment(n, 0.0), 1.0));
        return;
    }
    let noise = texel_noise(id.xy, id.z);
    // Bevy's sample divides by the normal's z plus one, nought facing down -z,
    // so the sampling is done mirrored in z there and turned back.
    let flip = select(vec3<f32>(1.0), vec3<f32>(1.0, 1.0, -1.0), n.z < 0.0);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0u; i < params.samples; i = i + 1u) {
        let xi = fract(hammersley_2d(i, params.samples) + noise);
        let l = flip * sample_visible_ggx(xi, roughness, n * flip, n * flip);
        let nl = dot(n, l);
        if (nl > 0.0) {
            let nh = dot(n, normalize(n + l));
            let g = g_smith(1.0, nl, roughness);
            let pdf = ggx_vndf_pdf(vec3<f32>(0.0, 0.0, 1.0), nh, roughness);
            sum = sum + sample_environment(l, level_for(pdf, f32(params.samples))) * nl * g;
            weight = weight + nl * g;
        }
    }
    textureStore(dst, id.xy, id.z, vec4<f32>(sum / max(weight, 1e-6), 1.0));
}

// Bevy's generate_irradiance_map, its samples spread by the sequence and
// read from the level their share of the sphere wants.
@compute @workgroup_size(8, 8, 1)
fn diffuse(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst).xy;
    if (any(id.xy >= size)) {
        return;
    }
    let n = sample_cube_dir((vec2<f32>(id.xy) + 0.5) / vec2<f32>(size), id.z);
    let noise = texel_noise(id.xy, id.z);
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < params.samples; i = i + 1u) {
        let xi = fract(hammersley_2d(i, params.samples) + noise);
        let cos_theta = 1.0 - 2.0 * xi.x;
        let phi = 2.0 * PI * xi.y;
        let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
        let l = normalize(n + vec3<f32>(sin_theta * cos(phi), sin_theta * sin(phi), cos_theta));
        let pdf = max(dot(n, l), 1e-4) / PI;
        sum = sum + sample_environment(l, level_for(pdf, f32(params.samples)) + 1.0);
    }
    textureStore(dst, id.xy, id.z, vec4<f32>(sum / f32(params.samples), 1.0));
}

// The background: the specular cube through the camera, at a level as soft
// as Background blur asks.
struct Background {
    env: Environment,
    // The camera's own directions in comp space, a column each.
    ray0: vec4<f32>,
    ray1: vec4<f32>,
    ray2: vec4<f32>,
    // xy = the comp's middle, z = the camera's zoom, w = the blur, 0 to 1.
    view: vec4<f32>,
    // xy = comp pixels a texel of the picture.
    raster: vec4<f32>,
};

@group(0) @binding(4) var cube: texture_cube<f32>;
@group(0) @binding(5) var cube_samp: sampler;
@group(0) @binding(6) var picture: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var<uniform> bg: Background;
@group(0) @binding(8) var panorama: texture_2d<f32>;
@group(0) @binding(9) var panorama_samp: sampler;

@compute @workgroup_size(8, 8, 1)
fn background(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(picture);
    if (any(id.xy >= size)) {
        return;
    }
    let at = (vec2<f32>(id.xy) + 0.5) * bg.raster.xy - bg.view.xy;
    let ray = mat3x3<f32>(bg.ray0.xyz, bg.ray1.xyz, bg.ray2.xyz) * vec3<f32>(at, bg.view.z);
    // Sharp, it is the panorama at its own detail, which the cube's first
    // level is smaller than, then the cube's softer levels.
    let d = env_turn(bg.env, normalize(ray));
    let level = bg.view.w * bg.env.turn0.w;
    let soft = textureSampleLevel(cube, cube_samp, d, level).rgb;
    let sharp = textureSampleLevel(panorama, panorama_samp, panorama_uv(d), 0.0).rgb;
    let rgb = mix(max(sharp, vec3<f32>(0.0)), soft, clamp(level, 0.0, 1.0)) * bg.env.colour.rgb;
    textureStore(picture, id.xy, vec4<f32>(rgb, 1.0));
}
