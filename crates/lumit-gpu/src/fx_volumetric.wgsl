// Volumetric light (lumit_core::haze, which is the oracle and carries the
// explanation). Compiled after the noise core and fx_lighting.wgsl, whose
// lights, casters and `transmitted` it reads through the same uniform `p`,
// and the models' shadow maps through `mapped`, which the oracle has no
// models for.
//
// haze_march runs at half size: each texel walks its ray from the eye to the
// scene's depth and keeps the light scattered towards the eye and how much of
// what is behind is left. haze_apply lays that over the full-size picture,
// reading the sixteen nearest half-size texels weighted by how close their
// depth is to this pixel's.
//
// The same walk takes up to four boxes of smoke, each a 3D field of density in
// red and heat in green with the light reaching it worked out beforehand
// (smoke_lit). Haze behind a box is dimmed by it. A surface the ray stops at is
// dimmed by what the smoke keeps from each light, which is smoke's shadow.

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
    // x = the clip w the scene depth is counted from, y = how many pixels a
    // texel of the walk covers, z = how many steps a ray takes, w = how many
    // boxes of smoke there are.
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

const MAX_VOLUMES: u32 = 4u;
// The most steps a ray takes through boxes in a row, and through one towards a
// light.
const VOLUME_STEPS: u32 = 256u;
const LIGHT_STEPS: u32 = 12u;
const SURFACE_STEPS: u32 = 24u;
// A ray with this little left of what is behind stops walking.
const OPAQUE: f32 = 0.003;
// A depth as far as this is no surface.
const NO_SURFACE: f32 = 60000.0;

struct Volume {
    // Comp pixels to the box's own nought to one, by rows.
    to_box: array<vec4<f32>, 3>,
    // Its cells each way, then how much a density of one dims per comp pixel.
    cells: vec4<f32>,
    // The smoke's colour, then how much its shadows count.
    smoke: vec4<f32>,
    // The fire at its coolest, then the light a flame gives per comp pixel.
    fire_low: vec4<f32>,
    // The fire at its hottest, then the heat a flame starts at.
    fire_high: vec4<f32>,
    // Finer detail than the cells hold: how far gas is pushed about, in the box's
    // nought to one, how many patches fit in the box each way, then how far they
    // have risen, in patches.
    detail: vec4<f32>,
    detail_scale: vec4<f32>,
};

@group(1) @binding(0) var<uniform> volumes: array<Volume, 4>;
@group(1) @binding(1) var fields0: texture_3d<f32>;
@group(1) @binding(2) var fields1: texture_3d<f32>;
@group(1) @binding(3) var fields2: texture_3d<f32>;
@group(1) @binding(4) var fields3: texture_3d<f32>;
@group(1) @binding(5) var lit0: texture_3d<f32>;
@group(1) @binding(6) var lit1: texture_3d<f32>;
@group(1) @binding(7) var lit2: texture_3d<f32>;
@group(1) @binding(8) var lit3: texture_3d<f32>;
// A tiling 3D noise and how to read it.
@group(1) @binding(13) var noise_field: texture_3d<f32>;
@group(1) @binding(14) var noise_wrap: sampler;
// The depth of the layers that take no shadow, far where there are none.
@group(1) @binding(9) var no_shadow: texture_2d<f32>;
// haze_march's result when there are boxes, whatever the project's depth: their
// light runs well over one, which a narrower picture clips.
@group(0) @binding(20) var wide_out: texture_storage_2d<rgba16float, write>;
// smoke_lit's own: the light it works out, in half floats, in rows a 3D texture
// is copied from, which box that is and the rows' pitch, then its cells each way.
@group(1) @binding(10) var<storage, read_write> lit_out: array<vec2<u32>>;
@group(1) @binding(11) var<uniform> lit_which: array<vec4<u32>, 2>;
// The box's nought to one to comp pixels: three axes then its corner.
@group(1) @binding(12) var<uniform> lit_from: array<vec4<f32>, 4>;

fn into_box(i: u32, q: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(dot(volumes[i].to_box[0], q), dot(volumes[i].to_box[1], q),
        dot(volumes[i].to_box[2], q));
}

// Gas thins out before a walked box's sides and top, so the box never shows as a
// cut. The floor stays hard, and a file's box is tight to its gas.
fn volume_fields(i: u32, at: vec3<f32>) -> vec4<f32> {
    let w = volumes[i].detail.z;
    let side = min(min(at.x, 1.0 - at.x), min(at.z, 1.0 - at.z));
    let fade = select(1.0, smoothstep(0.0, w, side) * smoothstep(0.0, w * 5.0 / 3.0, at.y), w > 0.0);
    return volume_raw(i, at) * fade;
}

fn volume_raw(i: u32, at: vec3<f32>) -> vec4<f32> {
    switch (i) {
        case 0u: { return textureSampleLevel(fields0, square_sampler, at, 0.0); }
        case 1u: { return textureSampleLevel(fields1, square_sampler, at, 0.0); }
        case 2u: { return textureSampleLevel(fields2, square_sampler, at, 0.0); }
        default: { return textureSampleLevel(fields3, square_sampler, at, 0.0); }
    }
}

fn volume_lit(i: u32, at: vec3<f32>) -> vec3<f32> {
    switch (i) {
        case 0u: { return textureSampleLevel(lit0, square_sampler, at, 0.0).rgb; }
        case 1u: { return textureSampleLevel(lit1, square_sampler, at, 0.0).rgb; }
        case 2u: { return textureSampleLevel(lit2, square_sampler, at, 0.0).rgb; }
        default: { return textureSampleLevel(lit3, square_sampler, at, 0.0).rgb; }
    }
}

// Where the line `o + d t` is inside box `i`: its first and last t.
fn box_span(i: u32, o: vec3<f32>, d: vec3<f32>) -> vec2<f32> {
    let a = into_box(i, vec4<f32>(o, 1.0));
    let b = into_box(i, vec4<f32>(d, 0.0));
    let safe = select(b, vec3<f32>(1e-12), abs(b) < vec3<f32>(1e-12));
    let t0 = -a / safe;
    let t1 = (vec3<f32>(1.0) - a) / safe;
    let lo = min(t0, t1);
    let hi = max(t0, t1);
    return vec2<f32>(max(lo.x, max(lo.y, lo.z)), min(hi.x, min(hi.y, hi.z)));
}

// How much box `i` dims the way from `pos` along the unit `dir`, as far as `far`,
// in `steps` steps starting `jit` of a step in.
fn volume_depth(i: u32, pos: vec3<f32>, dir: vec3<f32>, far: f32, jit: f32, steps: u32) -> f32 {
    let s = box_span(i, pos, dir);
    let a = max(s.x, 0.0);
    let b = min(s.y, far);
    if (b <= a) {
        return 0.0;
    }
    let dt = (b - a) / f32(steps);
    var sum: f32 = 0.0;
    for (var k: u32 = 0u; k < steps; k = k + 1u) {
        let q = pos + dir * (a + (f32(k) + jit) * dt);
        sum = sum + volume_fields(i, into_box(i, vec4<f32>(q, 1.0))).r;
    }
    return sum * volumes[i].cells.w * dt;
}

// How much the boxes dim the way from `pos` to light `l`, each by its Shadow
// strength. `to_light` is where `arrival` said the light is.
fn volume_shadow_depth(pos: vec3<f32>, l: Light, to_light: vec3<f32>, jit: f32, steps: u32) -> f32 {
    let dir = normalise(to_light);
    var far = length(to_light);
    if (u32(l.flags.x + 0.5) == 2u) {
        far = 1e7;
    }
    var depth: f32 = 0.0;
    let count = min(u32(h.view.w), MAX_VOLUMES);
    for (var i: u32 = 0u; i < count; i = i + 1u) {
        depth = depth + volume_depth(i, pos, dir, far, jit, steps) * volumes[i].smoke.a;
    }
    return depth;
}

// A surface at `pos` as much as the boxes leave it of the light it gets, the
// Ambient lights never being in their shadow.
fn volume_shadow(pos: vec3<f32>, jit: f32) -> f32 {
    var direct: f32 = 0.0;
    var kept: f32 = 0.0;
    var flat_light: f32 = 0.0;
    let live = min(u32(p.normal.w), MAX_LIT_LIGHTS);
    for (var j: u32 = 0u; j < live; j = j + 1u) {
        let l = p.lights[j];
        let bright = dot(l.colour.rgb, vec3<f32>(1.0 / 3.0));
        if (u32(l.flags.x + 0.5) == 3u) {
            flat_light = flat_light + bright;
            continue;
        }
        var to_light = vec3<f32>(0.0);
        let strength = arrival(pos, l, &to_light);
        if (strength <= 0.0) {
            continue;
        }
        direct = direct + bright * strength;
        let tau = volume_shadow_depth(pos, l, to_light, jit, SURFACE_STEPS);
        kept = kept + bright * strength * light_past_smoke(tau);
    }
    if (direct <= 0.0) {
        return 1.0;
    }
    return (flat_light + kept) / (flat_light + direct);
}

// What the walk through the boxes came to: the light towards the eye, how much
// of what is behind is left, and the first and last t it was in any.
struct VolumeSeen {
    light: vec3<f32>,
    left: f32,
    first: f32,
    last: f32,
};

// Walks the boxes the ray `eye + d t` passes, in order, as far as `end`, a step
// about a cell long. Boxes that overlap are walked together, so each dims the
// other.
fn volumes_ray(d: vec3<f32>, dl: f32, end: f32, jit: f32) -> VolumeSeen {
    var seen = VolumeSeen(vec3<f32>(0.0), 1.0, 1e30, -1e30);
    let count = min(u32(h.view.w), MAX_VOLUMES);
    var spans: array<vec2<f32>, 4>;
    for (var i: u32 = 0u; i < count; i = i + 1u) {
        let s = box_span(i, h.eye.xyz, d);
        spans[i] = vec2<f32>(max(s.x, 0.0), min(s.y, end));
    }
    var from_t: f32 = 0.0;
    for (var lots: u32 = 0u; lots < count; lots = lots + 1u) {
        // The nearest box still ahead, then as far as those it touches reach.
        var a: f32 = 1e30;
        var b: f32 = -1e30;
        for (var i: u32 = 0u; i < count; i = i + 1u) {
            let lo = max(spans[i].x, from_t);
            if (spans[i].y > lo && lo < a) {
                a = lo;
                b = spans[i].y;
            }
        }
        if (b <= a) {
            break;
        }
        var rate: f32 = 0.0;
        for (var k: u32 = 0u; k < count; k = k + 1u) {
            for (var i: u32 = 0u; i < count; i = i + 1u) {
                if (spans[i].y > spans[i].x && spans[i].x <= b && spans[i].y >= a) {
                    b = max(b, spans[i].y);
                    let along = into_box(i, vec4<f32>(d, 0.0)) * volumes[i].cells.xyz;
                    rate = max(rate, length(along));
                }
            }
        }
        let steps = clamp(u32(ceil((b - a) * rate)), 4u, VOLUME_STEPS);
        let dt = (b - a) / f32(steps);
        seen.first = min(seen.first, a);
        seen.last = max(seen.last, b);
        for (var s: u32 = 0u; s < steps; s = s + 1u) {
            let t = a + (f32(s) + jit) * dt;
            var sigma: f32 = 0.0;
            var made = vec3<f32>(0.0);
            for (var i: u32 = 0u; i < count; i = i + 1u) {
                if (t < spans[i].x || t > spans[i].y) {
                    continue;
                }
                var at = into_box(i, vec4<f32>(h.eye.xyz + d * t, 1.0));
                var f = volume_fields(i, at);
                // Only where there is gas is it worth breaking up: the patches
                // rise with it, and push it about by less than a cell or two.
                if (f.r > 1e-4 || f.g > 0.05) {
                    let n = at * volumes[i].detail_scale.xyz
                        + vec3<f32>(0.0, volumes[i].detail.y, 0.0);
                    let push = textureSampleLevel(noise_field, noise_wrap, n, 0.0).xyz
                        - vec3<f32>(0.5);
                    at = at + push * volumes[i].detail.x / volumes[i].cells.xyz;
                    f = volume_fields(i, at);
                }
                let flame = smoothstep(volumes[i].fire_high.a, 1.0, f.g);
                // Gas that hot burns clear, or smoke would hide its own flames.
                let thick = f.r * volumes[i].cells.w * dl * (1.0 - flame * 0.85);
                sigma = sigma + thick;
                made = made + volumes[i].smoke.rgb * volume_lit(i, at) * thick;
                if (flame > 0.0) {
                    let hot = flame * flame * flame;
                    made = made + (volumes[i].fire_low.rgb * pow(flame, 1.6) + volumes[i].fire_high.rgb * hot)
                        * volumes[i].fire_low.a * dl;
                }
            }
            if (sigma > 1e-7) {
                let through = exp(-sigma * dt);
                seen.light = seen.light + seen.left * made * (1.0 - through) / sigma;
                seen.left = seen.left * through;
            } else {
                seen.light = seen.light + seen.left * made * dt;
            }
            if (seen.left < OPAQUE) {
                break;
            }
        }
        if (seen.left < OPAQUE) {
            break;
        }
        from_t = b;
    }
    return seen;
}

// How much of the haze at `t` is seen past the boxes: all of it before them,
// what they leave after them, and in between as far along as it is.
fn past_volumes(t: f32, seen: VolumeSeen) -> f32 {
    if (seen.left >= 1.0) {
        return 1.0;
    }
    return mix(1.0, seen.left, clamp((t - seen.first) / max(seen.last - seen.first, 1e-6), 0.0, 1.0));
}

// A step offset for walking boxes, repeating every four pixels, which the tent
// over four half-size texels in haze_apply smooths away. Smoke is thick enough
// for the haze's own offset to show as grain.
fn box_jitter(x: u32, y: u32) -> f32 {
    let a = (x ^ y) & 3u;
    let b = y & 3u;
    let v = ((a & 1u) << 3u) | ((b & 1u) << 2u) | ((a & 2u)) | ((b & 2u) >> 1u);
    return (f32(v) + 0.5) / 16.0;
}

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
    let cycle = i32(h.grain[0].w);
    let n = nc_perlin3(0u, 0u, x, y, z, cycle)
        + 0.5 * nc_perlin3(0u, 1u, x * 2.0, y * 2.0, z * 2.0, cycle);
    return height * max(1.0 + k.w / 100.0 * n, 0.0);
}

// One step of lumit_core::haze::march: what every showing light scatters
// towards the eye at step `i`, before the step's length.
fn haze_step(i: u32, jit: f32, t0: f32, dt: f32, d: vec3<f32>, dl: f32, dir: vec3<f32>,
             sigma: f32, live: u32, seen: VolumeSeen) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let t = t0 + (f32(i) + jit) * dt;
    let pos = h.eye.xyz + d * t;
    let left = exp(-sigma * (t - t0) * dl) * past_volumes(t, seen);
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
            let past = transmitted(pos, l) * mapped(pos, vec3<f32>(0.0), l);
            through = 1.0 - l.shadow.x * (1.0 - past);
        }
        let amount = left * k.x / 100.0 * k.y / 100.0 * SCATTER_PER_PX
            * thickness(pos, j) * strength * glow * phase;
        sum = sum + amount * l.colour.rgb * through;
    }
    return sum;
}

// As lumit_core::haze::march, past whatever `seen` found on the way.
fn haze_ray(d: vec3<f32>, dl: f32, dir: vec3<f32>, end: f32, jit: f32, seen: VolumeSeen) -> vec4<f32> {
    var out = vec4<f32>(0.0, 0.0, 0.0, 1.0);
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
        sum = sum + haze_step(i, jit, t0, dt, d, dl, dir, sigma, live, seen)
            + haze_step(i + 1u, jit, t0, dt, d, dl, dir, sigma, live, seen)
            + haze_step(i + 2u, jit, t0, dt, d, dl, dir, sigma, live, seen)
            + haze_step(i + 3u, jit, t0, dt, d, dl, dir, sigma, live, seen);
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
    textureStore(dst, vec2<i32>(gid.xy), march_pixel(gid));
}

@compute @workgroup_size(8, 8)
fn boxes_march(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(wide_out);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    textureStore(wide_out, vec2<i32>(gid.xy), march_pixel(gid));
}

// How many pixels of the picture a texel of the walk covers: 2 for haze, 1 for
// boxes on all but the largest pictures, as flames need the sharpness.
fn march_step() -> i32 {
    return i32(h.view.y);
}

fn march_pixel(gid: vec3<u32>) -> vec4<f32> {
    let full = vec2<i32>(textureDimensions(depth));
    let at = min(vec2<i32>(gid.xy) * march_step(), full - vec2<i32>(1));
    let q = (vec2<f32>(at) + vec2<f32>(0.5)) / max(h.forward.w, 1e-6);
    let z = textureLoad(depth, at, 0).r;
    let end = z + h.view.x;
    let jit = haze_jitter(gid.x, gid.y);
    let d = h.right.xyz * (q.x - h.right.w * 0.5)
        + h.up.xyz * (q.y - h.up.w * 0.5)
        + h.forward.xyz * h.eye.w;
    let dl = length(d);
    let dir = normalise(d);
    var out = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (any(dir != vec3<f32>(0.0))) {
        var seen = VolumeSeen(vec3<f32>(0.0), 1.0, 1e30, -1e30);
        if (h.view.w > 0.5) {
            seen = volumes_ray(d, dl, end, box_jitter(gid.x, gid.y));
        }
        out = haze_ray(d, dl, dir, end, jit, seen);
        out = vec4<f32>(out.rgb + seen.light, out.a * seen.left);
        if (h.view.w > 0.5 && z < NO_SURFACE) {
            let refuses = textureDimensions(no_shadow).x > 1u
                && abs(textureLoad(no_shadow, at, 0).r - z) < 0.01;
            if (!refuses) {
                out.a = out.a * volume_shadow(h.eye.xyz + d * end, box_jitter(gid.x, gid.y));
            }
        }
    }
    return out;
}

// The light reaching each cell of a box, for the walk to read: every light as
// the casters and the models' shadow maps let it through, and as much as the
// smoke on the way leaves of it. With no lights at all, smoke is lit from above.
@compute @workgroup_size(4, 4, 4)
fn smoke_lit(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = lit_which[1].xyz;
    if (any(gid >= size)) {
        return;
    }
    let i = lit_which[0].x;
    let at = (vec3<f32>(gid) + vec3<f32>(0.5)) / vec3<f32>(size);
    let pos = lit_from[0].xyz * at.x + lit_from[1].xyz * at.y + lit_from[2].xyz * at.z
        + lit_from[3].xyz;
    let live = min(u32(p.normal.w), MAX_LIT_LIGHTS);
    var sum = vec3<f32>(0.0);
    if (live == 0u) {
        let tau = volume_depth(i, pos, vec3<f32>(0.0, -1.0, 0.0), 1e7, 0.5, LIGHT_STEPS)
            * volumes[i].smoke.a;
        sum = vec3<f32>(light_past_smoke(tau));
    }
    for (var j: u32 = 0u; j < live; j = j + 1u) {
        let l = p.lights[j];
        if (u32(l.flags.x + 0.5) == 3u) {
            sum = sum + l.colour.rgb;
            continue;
        }
        var to_light = vec3<f32>(0.0);
        let strength = arrival(pos, l, &to_light);
        if (strength <= 0.0) {
            continue;
        }
        var through = vec3<f32>(1.0);
        if (l.shadow.x > 0.0) {
            let past = transmitted(pos, l) * mapped(pos, vec3<f32>(0.0), l);
            through = 1.0 - l.shadow.x * (1.0 - past);
        }
        let tau = volume_shadow_depth(pos, l, to_light, 0.5, LIGHT_STEPS);
        sum = sum + l.colour.rgb * strength * through * light_past_smoke(tau);
    }
    sum = sum + fire_glow(i, at);
    let pitch = lit_which[0].y;
    lit_out[gid.x + gid.y * pitch + gid.z * pitch * size.y] =
        vec2<u32>(pack2x16float(sum.rg), pack2x16float(vec2<f32>(sum.b, 1.0)));
}

// The light the box's own flames throw on the gas round them: their heat summed
// along the three axes both ways, nearer counting more.
fn fire_glow(i: u32, at: vec3<f32>) -> vec3<f32> {
    let gain = bitcast<f32>(lit_which[0].z);
    if (gain <= 0.0) {
        return vec3<f32>(0.0);
    }
    var sum: f32 = 0.0;
    for (var a: u32 = 0u; a < 6u; a = a + 1u) {
        var way = vec3<f32>(0.0);
        way[a / 2u] = select(-1.0, 1.0, (a & 1u) == 0u);
        for (var k: u32 = 0u; k < 8u; k = k + 1u) {
            let q = at + way * (f32(k) + 0.5) * 0.04;
            let f = volume_fields(i, q);
            sum = sum + smoothstep(volumes[i].fire_high.a, 1.0, f.g) / (1.0 + f32(k));
        }
    }
    return volumes[i].fire_low.rgb * sum * gain;
}

// How much light gets through smoke this thick, with a little of what the smoke
// scatters on the way in, so a thick plume is never black inside.
fn light_past_smoke(tau: f32) -> f32 {
    return max(exp(-tau), 0.25 * exp(-tau * 0.25));
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
    let k = 1.0 / f32(march_step());
    let hp = (vec2<f32>(xy) + vec2<f32>(0.5)) * k - vec2<f32>(0.5);
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
        let dz = abs(textureLoad(depth, min(c * march_step(), size - vec2<i32>(1)), 0).r - z);
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
    // so it shows over transparency and leaves a precomp's alpha honest. Smoke
    // covers as much as it dims too, or a dark plume would be no plume.
    let cover = max(s.r, max(s.g, s.b));
    var a = min(o.a + cover, 1.0);
    // Smoke's alpha already says how much it dims, so adding its light on top
    // would take what is behind away twice.
    if (h.view.w > 0.5) {
        a = max(1.0 - (1.0 - o.a) * s.a, min(cover, 1.0));
    }
    textureStore(dst, xy, vec4<f32>(o.rgb * s.a + s.rgb, a));
}
