// Lens dirt: dust, grit, hairs, smudges, haze, scratches and droplets on
// curved glass, coloured, and lit by the bright parts of the picture and by
// Ambient. Prepended with fx_noise_core.wgsl.
//
//   ld_bright  the highlights at a quarter of the size
//   ld_blur_h  ...spread wide, across
//   ld_blur_v  ...and down: the light that reaches each part of the glass
//   ld_dirt    the dirt, out of focus by the f-stop, lit, and the picture
//              bent through the droplets
//
// Sizes are in units of a thousandth of the frame's diagonal, so a preview at
// any resolution shows the same dirt.

struct P {
    w: u32,
    h: u32,
    qw: u32,
    qh: u32,
    seed: u32,
    blades: u32,
    // Always 0, the noise's "never wraps". Read from here rather than written
    // as a literal: FXC folds a literal 0 into the noise's modulo and refuses
    // to compile the branch that guards it.
    no_cycle: i32,
    // 1 draws the lit dirt alone, over black.
    alone: u32,
    // Pixels per unit.
    k: f32,
    // The dirt pattern's own scale.
    scale: f32,
    // How far out of focus the glass is, in pattern units.
    rd: f32,
    rotation: f32,
    roundness: f32,
    dust: f32,
    smudges: f32,
    haze: f32,
    scratches: f32,
    droplets: f32,
    refraction: f32,
    intensity: f32,
    threshold: f32,
    // The spread of the light, quarter-size pixels.
    sigma_q: f32,
    mix_amt: f32,
    ambient: f32,
    grit: f32,
    hairs: f32,
    // How far the glass bulges, -1..1.
    curvature: f32,
    // How far each speck's hue and brightness stray, 0..1.
    variation: f32,
    tint: vec4<f32>,
    tint_2: vec4<f32>,
    // 1 picks each speck's colour between the two tints.
    gradient: u32,
    _g1: u32,
    _g2: u32,
    _g3: u32,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: P;
@group(0) @binding(2) var bright_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var light: texture_2d<f32>;
@group(0) @binding(4) var light_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var dst: texture_storage_2d<rgba16float, write>;

const TAU: f32 = 6.2831853;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@compute @workgroup_size(8, 8)
fn ld_bright(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.qw || gid.y >= p.qh) {
        return;
    }
    var sum = vec3<f32>(0.0);
    var n = 0.0;
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let at = vec2<i32>(i32(min(gid.x * 4u + x, p.w - 1u)), i32(min(gid.y * 4u + y, p.h - 1u)));
            let c = textureLoad(src, at, 0).rgb;
            let l = luma(c);
            if (l > p.threshold) {
                sum += c * ((l - p.threshold) / l);
            }
            n += 1.0;
        }
    }
    textureStore(bright_out, vec2<i32>(gid.xy), vec4<f32>(sum / n, 1.0));
}

// One direction of a wide falloff, its weights summing to 1 over the whole
// reach, in the frame or not, so the glass at the frame's edge is not lit by
// light from past it.
fn blur(gid: vec2<u32>, dir: vec2<i32>) {
    if (gid.x >= p.qw || gid.y >= p.qh) {
        return;
    }
    // Every texel, never a stride: skipping them shows as a grid. At a quarter
    // of the size even the widest spread is a few hundred reads.
    // Four sigmas: the light is raised before it is seen, so at three it can
    // still be bright enough to end in a line.
    let reach = min(i32(ceil(p.sigma_q * 4.0)), 768);
    let size = vec2<i32>(i32(p.qw), i32(p.qh));
    var sum = vec3<f32>(0.0);
    var total = 0.0;
    for (var i = -reach; i <= reach; i++) {
        let t = f32(i) / max(p.sigma_q, 0.5);
        // Eased to nothing at the reach, so it ends without a line.
        let e = 1.0 - (f32(i) / f32(reach + 1)) * (f32(i) / f32(reach + 1));
        let w = exp(-0.5 * t * t) * e * e;
        total += w;
        // Past the frame is dark. Clamping to the edge would read its light
        // over and over and draw a line a reach in from it.
        let at = vec2<i32>(gid) + dir * i;
        if (any(at < vec2<i32>(0)) || any(at >= size)) {
            continue;
        }
        sum += textureLoad(light, at, 0).rgb * w;
    }
    textureStore(light_out, vec2<i32>(gid), vec4<f32>(sum / total, 1.0));
}

@compute @workgroup_size(8, 8)
fn ld_blur_h(@builtin(global_invocation_id) gid: vec3<u32>) {
    blur(gid.xy, vec2<i32>(1, 0));
}

@compute @workgroup_size(8, 8)
fn ld_blur_v(@builtin(global_invocation_id) gid: vec3<u32>) {
    blur(gid.xy, vec2<i32>(0, 1));
}

fn h01(c: vec2<i32>, channel: u32, i: i32) -> f32 {
    return nc_hash01(p.seed, channel, c.x, c.y, i);
}

// A colour from four draws in 0..1: where it sits between the tints, which
// way its hue strays and how far, and its brightness. White has no hue to
// turn, so it also takes on a little of the stray hue.
fn tinted(d: vec4<f32>) -> vec3<f32> {
    var col = p.tint.rgb;
    if (p.gradient == 1u) {
        col = mix(p.tint.rgb, p.tint_2.rgb, d.x);
    }
    if (p.variation <= 0.0) {
        return col;
    }
    let amount = p.variation * d.z;
    // Turned about the grey axis, so the brightness holds.
    let turn = (d.y - 0.5) * 2.0 * amount * 3.14159265;
    let g = vec3<f32>(0.57735027);
    let ct = cos(turn);
    col = col * ct + cross(g, col) * sin(turn) + g * dot(g, col) * (1.0 - ct);
    let hue = cos(TAU * (d.y + vec3<f32>(0.0, 0.33333333, 0.66666667)));
    col += hue * luma(col) * amount * 0.6;
    return max(col, vec3<f32>(0.0)) * (1.0 + (d.w - 0.5) * p.variation);
}

// One speck's colour, drawn from its cell.
fn speck_colour(c: vec2<i32>, channel: u32, i: i32) -> vec3<f32> {
    return tinted(vec4<f32>(
        h01(c, channel, i),
        h01(c, channel + 1u, i),
        h01(c, channel + 2u, i),
        h01(c, channel + 3u, i),
    ));
}

// The same for dirt that spreads, varying slowly across the glass.
fn colour_at(q: vec2<f32>, channel: u32) -> vec3<f32> {
    if (p.variation <= 0.0 && p.gradient == 0u) {
        return p.tint.rgb;
    }
    let f = q / 300.0;
    let draw = vec4<f32>(
        nc_perlin3(p.seed, channel, f.x, f.y, 0.0, p.no_cycle),
        nc_perlin3(p.seed, channel + 1u, f.x, f.y, 0.0, p.no_cycle),
        nc_perlin3(p.seed, channel + 2u, f.x, f.y, 0.0, p.no_cycle),
        nc_perlin3(p.seed, channel + 3u, f.x, f.y, 0.0, p.no_cycle),
    );
    return tinted(clamp(vec4<f32>(0.5) + 0.6 * draw, vec4<f32>(0.0), vec4<f32>(1.0)));
}

// Where a point of the frame, in units from its middle (500 at the corners),
// sits on the glass. A bulging front element turns away towards its rim, so
// more of it, and more dirt, crowds into each step out across the frame; a
// dished one spreads it out. Up to 1 that is a sphere seen from in front;
// beyond it bends harder still, the way a fisheye's front glass does.
fn on_glass(v: vec2<f32>) -> vec2<f32> {
    let c = abs(p.curvature);
    let r = min(length(v) / 500.0, 1.0);
    let x = min(c, 0.999) * r;
    if (x < 1e-4) {
        return v;
    }
    let stretch = asin(x) / x * (1.0 + max(c - 1.0, 0.0) * 1.5 * r * r);
    if (p.curvature > 0.0) {
        return v * stretch;
    }
    return v / stretch;
}

// How far inside the iris a point is, as a fraction of its radius: the blade
// polygon blended to a circle, as lumit_core's pupil_mask draws it.
fn iris_bound(d: vec2<f32>) -> f32 {
    let blades = f32(max(p.blades, 3u));
    let sector = TAU / blades;
    let apothem = cos(3.14159265 / blades);
    var a = (atan2(d.y, d.x) - p.rotation) % sector;
    if (a < 0.0) {
        a += sector;
    }
    let poly = apothem / cos(a - sector * 0.5);
    return poly + (1.0 - poly) * p.roundness;
}

// Dust: specks on the lens's several surfaces, each out of focus by how far its
// surface sits from the iris, so they come in many sizes. A disc is faint
// inside with a thin bright rim, the rim split a little by colour, and it
// fades by its width rather than its area so wide open it still reads. Near
// the frame's edge the iris is clipped into a cat's eye.
fn disc(t: f32, rr: f32, eye: f32) -> f32 {
    let inside = clamp((1.0 - t) * rr / 0.6, 0.0, 1.0) * clamp((1.0 - eye) * rr / 0.6, 0.0, 1.0);
    return inside * (0.55 + 0.5 * smoothstep(0.85, 0.98, t));
}

fn dust(q: vec2<f32>, frame_q: vec2<f32>) -> vec3<f32> {
    if (p.dust <= 0.0) {
        return vec3<f32>(0.0);
    }
    let cell = 24.0;
    let base = vec2<i32>(floor(q / cell));
    let reach = i32(ceil((p.rd + 4.0) / cell));
    var sum = vec3<f32>(0.0);
    for (var cy = -reach; cy <= reach; cy++) {
        for (var cx = -reach; cx <= reach; cx++) {
            let c = base + vec2<i32>(cx, cy);
            for (var i = 0; i < 4; i++) {
                if (h01(c, 1u, i) > p.dust) {
                    continue;
                }
                let at = (vec2<f32>(c) + vec2<f32>(h01(c, 2u, i), h01(c, 3u, i))) * cell;
                let size = 0.3 + 2.4 * pow(h01(c, 4u, i), 3.0);
                // Most specks sit on the far glass and blur alike; a few sit
                // near the sensor and stay small and bright.
                let near = h01(c, 8u, i) < 0.12;
                let far = select(0.7 + 0.3 * h01(c, 6u, i), 0.04 + 0.1 * h01(c, 6u, i), near);
                let rr = max(size, p.rd * far);
                let d = q - at;
                if (dot(d, d) > rr * rr * 1.1) {
                    continue;
                }
                let bound = iris_bound(d) * rr;
                // The cat's eye: a second circle the size of the iris, pushed
                // out towards the frame's edge.
                let out_dir = frame_q / max(length(frame_q), 1e-3);
                let lean = clamp(length(frame_q) / 500.0, 0.0, 1.0);
                let eye = length(d + out_dir * rr * 0.8 * lean) / (rr * (1.0 + 0.8 * lean));
                let l = length(d);
                let col = vec3<f32>(
                    disc(l / (bound * 0.985), rr, eye),
                    disc(l / bound, rr, eye),
                    disc(l / (bound * 1.015), rr, eye),
                );
                let strength = clamp(size / rr * 2.0, 0.08, 1.0) * (0.06 + 0.94 * pow(h01(c, 5u, i), 3.0));
                sum += col * strength * speck_colour(c, 80u, i);
            }
        }
    }
    return sum;
}

// Grit: fine sharp specks, dust near the back of the lens where it is almost
// in focus.
fn grit(q: vec2<f32>) -> vec3<f32> {
    if (p.grit <= 0.0) {
        return vec3<f32>(0.0);
    }
    let cell = 5.0;
    let base = vec2<i32>(floor(q / cell));
    let gather = 0.5 + nc_perlin3(p.seed, 55u, q.x / 200.0, q.y / 200.0, 0.0, p.no_cycle);
    // At least half a pixel across, so a speck never shimmers away.
    let soft = max(0.12, 0.5 / (p.k * p.scale));
    var sum = vec3<f32>(0.0);
    for (var cy = -1; cy <= 1; cy++) {
        for (var cx = -1; cx <= 1; cx++) {
            let c = base + vec2<i32>(cx, cy);
            if (h01(c, 50u, 0) > p.grit * gather) {
                continue;
            }
            let at = (vec2<f32>(c) + vec2<f32>(h01(c, 51u, 0), h01(c, 52u, 0))) * cell;
            let r = 0.1 + 0.35 * h01(c, 53u, 0) * h01(c, 53u, 0);
            let d = length(q - at);
            sum += clamp((r + soft - d) / soft, 0.0, 1.0) * (0.15 + 0.85 * h01(c, 54u, 0))
                * speck_colour(c, 84u, 0);
        }
    }
    return sum;
}

// Hairs: a few curling fibres, sharp, each a short walk that keeps turning.
fn hairs(q: vec2<f32>) -> vec3<f32> {
    if (p.hairs <= 0.0) {
        return vec3<f32>(0.0);
    }
    let cell = 260.0;
    let base = vec2<i32>(floor(q / cell));
    let width = max(1.2, 0.8 / (p.k * p.scale));
    var best = 1e9;
    var nearest = vec2<i32>(0);
    for (var cy = -1; cy <= 1; cy++) {
        for (var cx = -1; cx <= 1; cx++) {
            let c = base + vec2<i32>(cx, cy);
            if (h01(c, 60u, 0) > p.hairs) {
                continue;
            }
            var a = (vec2<f32>(c) + vec2<f32>(h01(c, 61u, 0), h01(c, 62u, 0))) * cell;
            var turn = h01(c, 63u, 0) * TAU;
            let bend = (h01(c, 64u, 0) - 0.5) * 0.5;
            for (var s = 0; s < 18; s++) {
                turn += bend + (h01(c, 65u, s) - 0.5) * 1.1;
                let b = a + vec2<f32>(cos(turn), sin(turn)) * 3.0;
                let ab = b - a;
                let t = clamp(dot(q - a, ab) / dot(ab, ab), 0.0, 1.0);
                let d = length(q - a - ab * t);
                if (d < best) {
                    best = d;
                    nearest = c;
                }
                a = b;
            }
        }
    }
    return exp(-(best * best) / (width * width))
        * speck_colour(nearest, 88u, 0);
}
// Smudges: streaks that follow a slowly turning direction, and fingerprint
// ridges inside some of them. Detail finer than the defocus is left out, the
// way the glass being out of focus leaves it out.
fn smudges(q: vec2<f32>) -> f32 {
    if (p.smudges <= 0.0) {
        return 0.0;
    }
    let turn = nc_perlin3(p.seed, 10u, q.x / 400.0, q.y / 400.0, 0.0, p.no_cycle) * 3.14159265;
    let along = vec2<f32>(cos(turn), sin(turn));
    let s = vec2<f32>(dot(q, along) * 0.5, dot(q, vec2<f32>(-along.y, along.x)));
    var n = 0.0;
    var amp = 0.6;
    var wave = 90.0;
    for (var o = 0u; o < 4u; o++) {
        if (wave < p.rd * 1.5) {
            break;
        }
        n += amp * nc_perlin3(p.seed, 11u + o, s.x / wave, s.y / wave, 0.0, p.no_cycle);
        amp *= 0.5;
        wave *= 0.5;
    }
    let blob = smoothstep(0.15, 0.65, n);
    // Ridges about 2.5 units apart, only where the glass is near enough in
    // focus to show them.
    let period = 2.5;
    var ridges = 0.0;
    if (p.rd < period) {
        let centre = floor(q / 160.0);
        let c = (centre + vec2<f32>(0.5)) * 160.0;
        let ring = length(q - c) + 3.0 * nc_perlin3(p.seed, 16u, q.x / 30.0, q.y / 30.0, 0.0, p.no_cycle);
        ridges = (0.5 + 0.5 * sin(ring * TAU / period)) * (1.0 - p.rd / period)
            * smoothstep(0.3, 0.7, nc_perlin3(p.seed, 17u, q.x / 120.0, q.y / 120.0, 0.0, p.no_cycle));
    }
    return p.smudges * blob * (0.6 + 0.4 * ridges) + p.smudges * ridges * 0.3;
}

// Haze: a thin film over the whole glass, cast cool and purple in places the
// way coated glass is.
fn haze(q: vec2<f32>, tint: vec3<f32>) -> vec3<f32> {
    if (p.haze <= 0.0) {
        return vec3<f32>(0.0);
    }
    let n = nc_perlin3(p.seed, 20u, q.x / 500.0, q.y / 500.0, 0.0, p.no_cycle)
        + 0.5 * nc_perlin3(p.seed, 21u, q.x / 250.0, q.y / 250.0, 0.0, p.no_cycle);
    let coat = 0.5 + 0.5 * nc_perlin3(p.seed, 22u, q.x / 700.0, q.y / 700.0, 0.0, p.no_cycle);
    let colour = mix(vec3<f32>(1.0), vec3<f32>(0.78, 0.8, 1.18), coat);
    return p.haze * clamp(0.5 + 0.5 * n, 0.0, 1.0) * colour * tint;
}
// Scratches: fine straight lines, widened and dimmed by the defocus.
fn scratches(q: vec2<f32>) -> vec3<f32> {
    if (p.scratches <= 0.0) {
        return vec3<f32>(0.0);
    }
    let cell = 120.0;
    let base = vec2<i32>(floor(q / cell));
    let width = 0.15;
    let wide = max(width, p.rd * 0.5);
    var sum = vec3<f32>(0.0);
    for (var cy = -1; cy <= 1; cy++) {
        for (var cx = -1; cx <= 1; cx++) {
            let c = base + vec2<i32>(cx, cy);
            if (h01(c, 30u, 0) > p.scratches) {
                continue;
            }
            let a = (vec2<f32>(c) + vec2<f32>(h01(c, 31u, 0), h01(c, 32u, 0))) * cell;
            let turn = h01(c, 33u, 0) * TAU;
            let b = a + vec2<f32>(cos(turn), sin(turn)) * (20.0 + 90.0 * h01(c, 34u, 0));
            let ab = b - a;
            let t = clamp(dot(q - a, ab) / dot(ab, ab), 0.0, 1.0);
            let d = length(q - a - ab * t);
            sum += exp(-(d * d) / (wide * wide)) * (width / wide)
                * speck_colour(c, 92u, 0);
        }
    }
    return sum;
}

// Droplets: how much of this point is under one, and which way its surface
// leans there (pointing out from its middle, 1 at the edge).
fn droplet(q: vec2<f32>) -> vec3<f32> {
    if (p.droplets <= 0.0) {
        return vec3<f32>(0.0);
    }
    let cell = 40.0;
    let base = vec2<i32>(floor(q / cell));
    var best = vec3<f32>(0.0);
    for (var cy = -1; cy <= 1; cy++) {
        for (var cx = -1; cx <= 1; cx++) {
            let c = base + vec2<i32>(cx, cy);
            if (h01(c, 40u, 0) > p.droplets) {
                continue;
            }
            let at = (vec2<f32>(c) + vec2<f32>(h01(c, 41u, 0), h01(c, 42u, 0))) * cell;
            let r = 3.0 + 11.0 * h01(c, 43u, 0);
            let d = (q - at) / r;
            let l = length(d);
            let soft = max(p.rd * 0.5 / r, 0.05);
            let cover = clamp((1.0 - l) / soft, 0.0, 1.0);
            if (cover > best.z) {
                best = vec3<f32>(d, cover);
            }
        }
    }
    return best;
}

fn bilinear_light(at: vec2<f32>) -> vec3<f32> {
    let q = at / 4.0 - 0.5;
    let f = fract(q);
    let i = vec2<i32>(floor(q));
    let last = vec2<i32>(i32(p.qw) - 1, i32(p.qh) - 1);
    let a = textureLoad(light, clamp(i, vec2<i32>(0), last), 0).rgb;
    let b = textureLoad(light, clamp(i + vec2<i32>(1, 0), vec2<i32>(0), last), 0).rgb;
    let c = textureLoad(light, clamp(i + vec2<i32>(0, 1), vec2<i32>(0), last), 0).rgb;
    let d = textureLoad(light, clamp(i + vec2<i32>(1, 1), vec2<i32>(0), last), 0).rgb;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn bilinear_src(at: vec2<f32>) -> vec4<f32> {
    let q = at - 0.5;
    let f = fract(q);
    let i = vec2<i32>(floor(q));
    let last = vec2<i32>(i32(p.w) - 1, i32(p.h) - 1);
    let a = textureLoad(src, clamp(i, vec2<i32>(0), last), 0);
    let b = textureLoad(src, clamp(i + vec2<i32>(1, 0), vec2<i32>(0), last), 0);
    let c = textureLoad(src, clamp(i + vec2<i32>(0, 1), vec2<i32>(0), last), 0);
    let d = textureLoad(src, clamp(i + vec2<i32>(1, 1), vec2<i32>(0), last), 0);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

@compute @workgroup_size(8, 8)
fn ld_dirt(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let xy = vec2<i32>(gid.xy);
    let px = vec2<f32>(xy) + 0.5;
    let centre = vec2<f32>(f32(p.w), f32(p.h)) * 0.5;
    let frame_q = (px - centre) / p.k;
    let q = (on_glass(frame_q) + centre / p.k) / p.scale;
    let o = textureLoad(src, xy, 0);

    // Through a droplet the picture is bent the other way round, as through a
    // small lens.
    let drop = droplet(q);
    var base = o;
    if (drop.z > 0.0) {
        let bent = bilinear_src(px - drop.xy * p.refraction * 14.0 * p.k * p.scale);
        base = mix(o, bent, drop.z * min(p.refraction * 2.0, 1.0));
    }

    let rim = drop.z * pow(clamp(length(drop.xy), 0.0, 1.0), 4.0);
    let spread = colour_at(q, 96u);
    let scatter = dust(q, frame_q) + grit(q) + hairs(q) + scratches(q) + haze(q, spread)
        + (smudges(q) + rim * 0.6) * spread;
    // The spread light is the average over a wide patch, so a lamp is a small
    // share of it: raised by a square root, which lifts a little light a long
    // way yet still fades to nothing at the spread's edge, then rolled off
    // towards a ceiling, so a lamp lights the glass round it while a whole
    // bright wall cannot blow it out. Ambient lights it all.
    let raw = 8.0 * sqrt(max(bilinear_light(px), vec3<f32>(0.0)));
    let reached = 1.5 * (vec3<f32>(1.0) - exp(-raw / 1.5)) + vec3<f32>(p.ambient);
    let add = scatter * reached * p.intensity;
    if (p.alone == 1u) {
        textureStore(dst, xy, vec4<f32>(add, 1.0));
        return;
    }
    let lit = vec4<f32>(base.rgb + add, min(base.a + luma(add), 1.0));
    textureStore(dst, xy, mix(o, lit, p.mix_amt));
}
