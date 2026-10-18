// Depth of field pro. Six passes over one set of bindings, each pass binding only
// what it reads:
//
//   lb_prep    depth to metres to defocus, and which traced kernel a pixel uses
//   lb_tiles   the widest blur in each 16 px tile
//   lb_dilate  ...spread to every tile it can reach
//   lb_gather  every pixel collects the light its neighbours' kernels throw on it
//   lb_sprite  highlights drawn as whole kernels, added on top
//   lb_finish  Mix against the input, or the metres for the Depth view
//
// The kernels come from lumit-core's lens_blur bake. Each is a small picture of
// one point's blur, tangential along x and radially outward along y, each
// channel summing to 1.

struct P {
    w: u32,
    h: u32,
    tiles_w: u32,
    tiles_h: u32,
    field_bins: u32,
    defocus_bins: u32,
    az_bins: u32,
    kernel_res: u32,
    atlas_cols: u32,
    depth_bound: u32,
    depth_channel: u32,
    depth_invert: u32,
    use_focus_point: u32,
    blades: u32,
    depth_metres: u32,
    display: u32,
    focal_mm: f32,
    dz_max_mm: f32,
    field_max_mm: f32,
    px_per_mm: f32,
    focus_m: f32,
    near_m: f32,
    far_m: f32,
    size: f32,
    focus_x: f32,
    focus_y: f32,
    rotation: f32,
    threshold: f32,
    boost: f32,
    mix_amt: f32,
    max_span: f32,
    zebra: f32,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_2d<f32>;
@group(0) @binding(2) var info_out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var<uniform> p: P;
@group(0) @binding(4) var info: texture_2d<f32>;
@group(0) @binding(5) var tiles_out: texture_storage_2d<r32float, write>;
@group(0) @binding(6) var tiles: texture_2d<f32>;
@group(0) @binding(7) var atlas: texture_2d<f32>;
@group(0) @binding(8) var acc_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(9) var acc: texture_2d<f32>;
@group(0) @binding(10) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(11) var<storage, read> spans: array<f32>;
// Where this submission's share of the gather's rows or the sprites starts.
@group(0) @binding(12) var<uniform> part: vec4<u32>;

const TILE: i32 = 16;
// The least blur, px, that is drawn as a layer over what is behind it.
const FRONT_SPAN: f32 = 3.0;
// The least blur, px, whose highlights are drawn as sprites.
const SPRITE_SPAN: f32 = 1.5;
// Block sizes highlights are drawn in, 1 to 32 px. Mirrored in lens_blur.rs.
const SPRITE_LEVELS: u32 = 6u;
const TAU: f32 = 6.2831853;

fn channel_of(c: vec4<f32>) -> f32 {
    switch (p.depth_channel) {
        case 1u: { return c.a; }
        case 2u: { return c.r; }
        case 3u: { return c.g; }
        case 4u: { return c.b; }
        default: { return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b; }
    }
}

// How far away pixel `xy` is, metres: read straight off a depth in metres,
// or a 0..1 pass placed between the nearest and farthest distances. With
// neither, everything is at the far distance.
fn metres_at(xy: vec2<i32>) -> f32 {
    if (p.depth_bound == 0u) {
        return p.far_m;
    }
    let c = textureLoad(depth, xy, 0);
    if (p.depth_metres == 1u) {
        return max(c.r, 0.01);
    }
    var d = clamp(channel_of(c), 0.0, 1.0);
    if (p.depth_invert == 1u) {
        d = 1.0 - d;
    }
    return mix(p.near_m, p.far_m, d);
}

// Where a point `m` metres away comes to focus behind the lens, mm.
fn image_mm(m: f32) -> f32 {
    let f = p.focal_mm;
    let d = max(m * 1000.0, f * 1.001);
    return f * d / (d - f);
}

fn focus_metres() -> f32 {
    if (p.use_focus_point == 1u && p.depth_bound == 1u) {
        let xy = clamp(
            vec2<i32>(vec2<f32>(p.focus_x, p.focus_y)),
            vec2<i32>(0),
            vec2<i32>(i32(p.w) - 1, i32(p.h) - 1),
        );
        return metres_at(xy);
    }
    return p.focus_m;
}

// The bake's defocus planes, mirrored from lumit_core::fx::lens_blur.
fn dz_centre(b: u32) -> f32 {
    let half = p.defocus_bins / 2u;
    if (b < half) {
        let k = f32(half - b) / f32(half);
        return -p.dz_max_mm * k * k;
    }
    let k = f32(b - half + 1u) / f32(half);
    return p.dz_max_mm * k * k;
}

fn dz_bin(dz: f32) -> u32 {
    let half = p.defocus_bins / 2u;
    let s = round(sqrt(abs(dz) / max(p.dz_max_mm, 1e-6)) * f32(half));
    let k = clamp(u32(s), 1u, half);
    if (dz < 0.0) {
        return half - k;
    }
    return half - 1u + k;
}

fn offset_of(j: vec2<i32>) -> vec2<f32> {
    return vec2<f32>(j) + 0.5 - vec2<f32>(f32(p.w), f32(p.h)) * 0.5;
}

// The unit vector from the frame centre out through pixel `j`.
fn radial_of(j: vec2<i32>) -> vec2<f32> {
    let o = offset_of(j);
    let l = length(o);
    if (l < 1e-3) {
        return vec2<f32>(0.0, 1.0);
    }
    return o / l;
}

fn atlas_texel(slice: u32, x: i32, y: i32, level: u32) -> vec3<f32> {
    let k = i32(p.kernel_res >> level);
    if (x < 0 || y < 0 || x >= k || y >= k) {
        return vec3<f32>(0.0);
    }
    let col = i32(slice % p.atlas_cols);
    let row = i32(slice / p.atlas_cols);
    return textureLoad(atlas, vec2<i32>(col * k + x, row * k + y), i32(level)).rgb;
}

// A kernel's light per px² at local (tangential, radial) offset `l` from its
// centre, for a kernel `span` px wide each way. Bilinear between cells of the
// atlas's `level`, each a quarter the cells of the one before.
fn kernel_local(slice: u32, mirror: f32, span: f32, l: vec2<f32>, level: u32) -> vec3<f32> {
    let k = f32(p.kernel_res >> level);
    let u = (l.x * mirror / span * 0.5 + 0.5) * k - 0.5;
    let v = (l.y / span * 0.5 + 0.5) * k - 0.5;
    if (u < -1.0 || v < -1.0 || u > k || v > k) {
        return vec3<f32>(0.0);
    }
    let x0 = floor(u);
    let y0 = floor(v);
    let f = vec2<f32>(u - x0, v - y0);
    let xi = i32(x0);
    let yi = i32(y0);
    let a = mix(atlas_texel(slice, xi, yi, level), atlas_texel(slice, xi + 1, yi, level), f.x);
    let b = mix(atlas_texel(slice, xi, yi + 1, level), atlas_texel(slice, xi + 1, yi + 1, level), f.x);
    let cell = 2.0 * span / k;
    return mix(a, b, f.y) / (cell * cell);
}

// Pixel `j`'s kernel, read at raster offset `d` from `j`.
fn kernel_at(o: vec4<f32>, j: vec2<i32>, d: vec2<f32>) -> vec3<f32> {
    let r = radial_of(j);
    let t = vec2<f32>(-r.y, r.x);
    return kernel_local(u32(o.z), o.w, o.x, vec2<f32>(dot(d, t), dot(d, r)), 0u);
}

// How much of a pixel is over the highlight threshold.
fn hi_frac(c: vec4<f32>) -> f32 {
    let l = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    if (l <= p.threshold || l <= 0.0) {
        return 0.0;
    }
    return (l - p.threshold) / l;
}

// What a pixel puts into the gather. A wide blur's highlight goes to the
// sprites instead. A narrow one stays and is boosted here, eased in from no
// blur, so Blur size can run down to 0 without the highlights jumping.
fn gathered(c: vec4<f32>, span: f32) -> vec4<f32> {
    let k = hi_frac(c);
    if (span >= SPRITE_SPAN) {
        return c * (1.0 - k);
    }
    let b = mix(1.0, p.boost, clamp(span - 0.5, 0.0, 1.0));
    return vec4<f32>(c.rgb * (1.0 + k * (b - 1.0)), c.a);
}

fn hash(xy: vec2<i32>) -> f32 {
    var h = u32(xy.x) * 1664525u + u32(xy.y) * 22695477u + 1013904223u;
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = h ^ (h >> 13u);
    return f32(h & 0xffffu) / 65536.0;
}

@compute @workgroup_size(8, 8)
fn lb_prep(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let xy = vec2<i32>(gid.xy);
    let dist = metres_at(xy);
    let dz = image_mm(focus_metres()) - image_mm(dist);
    let b = dz_bin(dz);
    let scale = abs(dz) / abs(dz_centre(b));
    let o = offset_of(xy);
    let field = length(o) / p.px_per_mm / max(p.field_max_mm, 1e-6);
    let fb = u32(clamp(round(field * f32(p.field_bins - 1u)), 0.0, f32(p.field_bins - 1u)));
    // The iris's turn against this pixel's radial direction, folded into half
    // a blade: the other half is the mirror image.
    let sector = TAU / f32(max(p.blades, 3u));
    var beta = (p.rotation - atan2(o.y, o.x)) % sector;
    if (beta < 0.0) {
        beta += sector;
    }
    var mirror = 1.0;
    if (beta > sector * 0.5) {
        beta = sector - beta;
        mirror = -1.0;
    }
    let ab = min(u32(round(beta / (sector * 0.5) * f32(p.az_bins - 1u))), p.az_bins - 1u);
    let slice = (fb * p.defocus_bins + b) * p.az_bins + ab;
    let span = min(spans[slice] * scale * p.px_per_mm * p.size, p.max_span);
    textureStore(info_out, xy, vec4<f32>(span, dist, f32(slice), mirror));
}

@compute @workgroup_size(8, 8)
fn lb_tiles(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.tiles_w || gid.y >= p.tiles_h) {
        return;
    }
    var m = 0.0;
    let base = vec2<i32>(gid.xy) * TILE;
    for (var y = 0; y < TILE; y++) {
        for (var x = 0; x < TILE; x++) {
            let px = base + vec2<i32>(x, y);
            if (px.x < i32(p.w) && px.y < i32(p.h)) {
                m = max(m, textureLoad(info, px, 0).x);
            }
        }
    }
    textureStore(tiles_out, vec2<i32>(gid.xy), vec4<f32>(m));
}

@compute @workgroup_size(8, 8)
fn lb_dilate(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.tiles_w || gid.y >= p.tiles_h) {
        return;
    }
    let reach = i32(ceil(p.max_span / f32(TILE)));
    let me = vec2<i32>(gid.xy);
    var m = 0.0;
    for (var dy = -reach; dy <= reach; dy++) {
        for (var dx = -reach; dx <= reach; dx++) {
            let t = me + vec2<i32>(dx, dy);
            if (t.x < 0 || t.y < 0 || t.x >= i32(p.tiles_w) || t.y >= i32(p.tiles_h)) {
                continue;
            }
            let s = textureLoad(tiles, t, 0).x;
            let gap = max(length(vec2<f32>(f32(dx), f32(dy))) * f32(TILE) - f32(TILE) * 1.415, 0.0);
            if (s >= gap) {
                m = max(m, s);
            }
        }
    }
    textureStore(tiles_out, me, vec4<f32>(m));
}

@compute @workgroup_size(8, 8)
fn lb_gather(@builtin(global_invocation_id) gid: vec3<u32>) {
    let y = gid.y + part.x;
    if (gid.x >= p.w || y >= p.h) {
        return;
    }
    let xy = vec2<i32>(i32(gid.x), i32(y));
    let me = textureLoad(info, xy, 0);
    let c_me = textureLoad(src, xy, 0);
    let reach = max(textureLoad(tiles, xy / TILE, 0).x, me.x);
    if (reach < 0.5) {
        textureStore(acc_out, xy, c_me);
        return;
    }
    // Two layers. What is in front of focus and blurred is light spread over
    // what is behind it: it adds up how much of this pixel it covers as well
    // as its colour, and lies over the rest by that much, so its edge is soft
    // on both sides. The rest is averaged as it comes. Where something in
    // front hides it, the pixels behind nearest by fill in, the way the
    // blurred edge of a near thing shows what is behind it.
    let focus = focus_metres();
    var front = vec4<f32>(0.0);
    var front_w = vec3<f32>(0.0);
    var back = vec4<f32>(0.0);
    var back_w = vec3<f32>(0.0);
    // This pixel's own kernel summed over the same samples: whole, it would
    // be 1. A pixel in front reads its cover against it, so what the samples
    // miss inside a blurred thing does not let what is behind show through.
    var own = 0.0;
    // Only a real blur goes in front: one of a pixel or two covers too
    // little to say how much it hides, and would leave sharp things see-through.
    let me_front = me.y < focus && me.x >= FRONT_SPAN;
    {
        var w = vec3<f32>(1.0);
        if (me.x >= 0.5) {
            w = kernel_at(me, xy, vec2<f32>(0.0));
        }
        let cb = gathered(c_me, me.x);
        if (me_front) {
            front += vec4<f32>(cb.rgb * w, cb.a * w.g);
            front_w += w;
            own += w.g;
        } else {
            back += vec4<f32>(cb.rgb * w, cb.a * w.g);
            back_w += w;
        }
    }
    let n = u32(clamp(ceil(3.14159265 * reach * reach / 4.0), 8.0, 768.0));
    // A golden-angle spiral over the reach, turned per pixel so its pattern
    // reads as grain rather than copies. Its rings are evenly spaced rather
    // than its samples, so they crowd in close by: there a neighbour blurred
    // a little still lands many samples, and further out only kernels at
    // least that wide reach, each sample a small share of their light.
    let spin = hash(xy) * TAU;
    let last = vec2<i32>(i32(p.w) - 1, i32(p.h) - 1);
    let ring = 3.14159265 * reach * reach / (f32(n) * f32(n));
    for (var k = 0u; k < n; k++) {
        let rr = reach * (f32(k) + 0.5) / f32(n);
        let a = f32(k) * 2.3999632 + spin;
        let step = vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * rr));
        if (all(step == vec2<i32>(0))) {
            continue;
        }
        let j = clamp(xy + step, vec2<i32>(0), last);
        let o = textureLoad(info, j, 0);
        let d = vec2<f32>(xy - j);
        let area = ring * f32(2u * k + 1u);
        if (me_front) {
            own += kernel_at(vec4<f32>(max(me.x, sqrt(area)), me.yzw), xy, d).g * area;
        }
        let c = textureLoad(src, j, 0);
        let in_front = o.y < focus && o.x >= FRONT_SPAN;
        if (me_front && !in_front) {
            if (o.y <= me.y) {
                continue;
            }
            // Behind this pixel: its light where it reaches, and a faint
            // weight on every one across this pixel's own blur, to fill in
            // smoothly where none does.
            let fill = vec3<f32>(1e-12 * area / (1.0 + dot(d, d) / (me.x * me.x)));
            var w = fill;
            if (o.x >= 0.5) {
                w += kernel_at(vec4<f32>(max(o.x, sqrt(area)), o.yzw), j, d) * area;
            }
            let cb = gathered(c, o.x);
            back += vec4<f32>(cb.rgb * w, cb.a * w.g);
            back_w += w;
            continue;
        }
        if (o.x < 0.5) {
            continue;
        }
        // Something behind this pixel only shows as far as this pixel's own
        // blur reaches, so a sharp edge in front is not washed over.
        if (o.y > me.y && length(d) > me.x + 1.0) {
            continue;
        }
        // The kernel is read as at least as wide as the sample's own patch,
        // keeping its light: narrower, it is dense light in a small disc that
        // samples either miss or count many times over, which shows as
        // speckle.
        let w = kernel_at(vec4<f32>(max(o.x, sqrt(area)), o.yzw), j, d) * area;
        let cb = gathered(c, o.x);
        if (in_front) {
            front += vec4<f32>(cb.rgb * w, cb.a * w.g);
            front_w += w;
        } else {
            back += vec4<f32>(cb.rgb * w, cb.a * w.g);
            back_w += w;
        }
    }
    let tiny = vec3<f32>(1e-30);
    if (front_w.g <= 1e-8) {
        if (back_w.g <= 1e-30) {
            textureStore(acc_out, xy, c_me);
            return;
        }
        textureStore(acc_out, xy, vec4<f32>(back.rgb / max(back_w, tiny), back.a / back_w.g));
        return;
    }
    let f = vec4<f32>(front.rgb / max(front_w, tiny), front.a / front_w.g);
    if (back_w.g <= 1e-30) {
        textureStore(acc_out, xy, f);
        return;
    }
    let b = vec4<f32>(back.rgb / max(back_w, tiny), back.a / back_w.g);
    // The front layer's cover: its kernels' light summed, whole at 1.
    var cover = clamp(front_w, vec3<f32>(0.0), vec3<f32>(1.0));
    if (me_front) {
        cover = clamp(front_w / max(own, 1e-6), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    // Eased so that nearly whole is whole: the samples' shortfall inside a
    // blurred thing would otherwise let what is behind through as speckle.
    cover = smoothstep(vec3<f32>(0.0), vec3<f32>(0.9), cover);
    textureStore(acc_out, xy, vec4<f32>(
        mix(b.rgb, f.rgb, cover),
        mix(b.a, f.a, cover.g),
    ));
}

struct SpriteOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) colour: vec4<f32>,
    @location(2) @interpolate(flat) at: vec4<f32>,
    @location(3) @interpolate(flat) src_px: vec2<f32>,
}

// Which block size a highlight of blur `span` is drawn in: blocks of 2^level
// px, no wider than an eighth of the blur. One sprite per pixel costs the
// square of the blur for every bright pixel, which on a wide blur over a
// bright sky is minutes a frame. A block's highlights are summed into one
// sprite at their centre of light, so a lone highlight still lands where it
// is, and a bright area's edge is a few even steps too faint to see.
fn sprite_level(span: f32) -> u32 {
    return u32(clamp(floor(log2(span / 8.0)), 0.0, f32(SPRITE_LEVELS - 1u)));
}

// One instance per block at every level, pixels first. A block sums the
// blurred highlights in it whose own level is this one, and becomes a quad the
// size of their kernel. A block with none collapses to nothing.
@vertex
fn lb_sprite_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
    var out: SpriteOut;
    out.pos = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.colour = vec4<f32>(0.0);
    out.at = vec4<f32>(0.0);
    out.src_px = vec2<f32>(0.0);
    var n = ii + part.x;
    var level = 0u;
    var bw = p.w;
    for (; level < SPRITE_LEVELS; level++) {
        bw = (p.w + (1u << level) - 1u) >> level;
        let bh = (p.h + (1u << level) - 1u) >> level;
        if (n < bw * bh) {
            break;
        }
        n -= bw * bh;
    }
    let b = i32(1u << level);
    let base = vec2<i32>(i32(n % bw), i32(n / bw)) * b;
    var hi = vec4<f32>(0.0);
    var sum_w = 0.0;
    var at = vec2<f32>(0.0);
    var span = 0.0;
    var dist = 0.0;
    var best = vec4<f32>(0.0);
    var best_w = 0.0;
    for (var y = 0; y < b; y++) {
        for (var x = 0; x < b; x++) {
            let j = base + vec2<i32>(x, y);
            if (j.x >= i32(p.w) || j.y >= i32(p.h)) {
                continue;
            }
            let o = textureLoad(info, j, 0);
            let c = textureLoad(src, j, 0);
            let k = hi_frac(c);
            if (o.x < SPRITE_SPAN || k <= 0.0 || sprite_level(o.x) != level) {
                continue;
            }
            let w = k * (0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b);
            hi += c * k;
            sum_w += w;
            at += vec2<f32>(j) * w;
            span += o.x * w;
            dist += o.y * w;
            if (w > best_w) {
                best = o;
                best_w = w;
            }
        }
    }
    if (sum_w <= 0.0) {
        return out;
    }
    at /= sum_w;
    let o = vec4<f32>(span / sum_w, dist / sum_w, best.z, best.w);
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );
    // A pixel wider, so every pixel the kernel's edge reaches is drawn.
    let local = corners[vi] * (o.x + 1.0);
    let r = radial_of(vec2<i32>(round(at)));
    let t = vec2<f32>(-r.y, r.x);
    let px = at + 0.5 + t * local.x + r * local.y;
    out.pos = vec4<f32>(px.x / f32(p.w) * 2.0 - 1.0, 1.0 - px.y / f32(p.h) * 2.0, 0.0, 1.0);
    out.local = local;
    out.colour = vec4<f32>(hi.rgb * p.boost, hi.a);
    out.at = o;
    out.src_px = at;
    return out;
}

@fragment
fn lb_sprite_fs(in: SpriteOut) -> @location(0) vec4<f32> {
    let here = vec2<i32>(in.pos.xy);
    let d = textureLoad(info, here, 0);
    // Hidden by anything nearer that is sharp where it lands.
    if (d.y < in.at.y && length(vec2<f32>(here) - in.src_px) > d.x + 1.0) {
        discard;
    }
    // Read at pixel centres, a small kernel's cells would be missed or hit
    // whole. Read the copy whose cells are at most a pixel apart.
    let fine = log2(f32(p.kernel_res) / (2.0 * in.at.x));
    let level = u32(clamp(ceil(fine), 0.0, f32(textureNumLevels(atlas) - 1u)));
    let w = kernel_local(u32(in.at.z), in.at.w, in.at.x, in.local, level);
    return vec4<f32>(in.colour.rgb * w, in.colour.a * w.g);
}

@compute @workgroup_size(8, 8)
fn lb_finish(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let xy = vec2<i32>(gid.xy);
    let me = textureLoad(info, xy, 0);
    if (p.display == 1u) {
        textureStore(dst, xy, zebra(xy, vec4<f32>(me.y, me.y, me.y, 1.0), me.x));
        return;
    }
    // The highlights add their coverage on top of the gather's, so an opaque
    // picture comes back above 1 where they land. Coverage stops at whole.
    var blurred = textureLoad(acc, xy, 0);
    blurred.a = min(blurred.a, 1.0);
    textureStore(dst, xy, zebra(xy, mix(textureLoad(src, xy, 0), blurred, p.mix_amt), me.x));
}

// The focus zebra, as Depth of field draws it: diagonal bands of white and
// mid-grey over pixels whose blur is under a pixel across, premultiplied.
fn zebra(xy: vec2<i32>, c: vec4<f32>, span: f32) -> vec4<f32> {
    if (p.zebra == 0.0 || span >= 0.5) {
        return c;
    }
    let band = ((xy.x + xy.y) / 6) % 4;
    if (band == 0) {
        return vec4<f32>(c.a, c.a, c.a, c.a);
    }
    if (band == 2) {
        return vec4<f32>(0.18 * c.a, 0.18 * c.a, 0.18 * c.a, c.a);
    }
    return c;
}

// ---- Ray traced ----
//
// Each pixel is a point on the sensor. Its rays go back through every surface
// of the lens, refracting at each one, dying on an element's edge or the iris,
// and leave the front glass into the scene. Each is then stepped through the
// depth until it passes behind it, and takes the colour there. A pixel is the
// average of what its rays find, each ray carrying one of three wavelengths.
//
//   lb_ray_min1    the nearest depth in each 8 px block, and the frame's range
//   lb_ray_min2    the nearest in each 64 px block
//   lb_ray_setup   where the sensor sits for the focus distance, and the chief
//                  ray of each height on the sensor
//   lb_ray_invert  the chief rays turned round: sensor height by slope
//   lb_ray_trace   one band of rows, one run of rays, added up per pixel
//   lb_ray_resolve the band's sums as a picture
//
// The scene's distances are measured from the entrance pupil, along the axis,
// and a point in it is found on the sensor through the lens's own chief rays,
// so what is in focus lands on the pixel it came from.

// The lens: four header words, then two per surface.
//   0: surfaces, stop radius, sensor at infinity focus, focal length
//   1: entrance pupil radius and place, roundness, blades
//   2: infinity's axis crossing, focus ray height, front glass on, iris turn
//   3: colour fringing, the half-diagonal of the sensor
//   per surface: radius, vertex z, clear semi-aperture, stop; Cauchy A, B
@group(0) @binding(13) var<storage, read> lens: array<vec4<f32>>;
// Worked out per frame: 0 the sensor, 1 the largest chief slope, 2 the on-axis
// aim radius, then the chief slope and aim per sensor height, then the sensor
// height per slope.
@group(0) @binding(14) var<storage, read_write> rt: array<f32>;
// The frame's nearest and farthest depth, as float bits.
@group(0) @binding(15) var<storage, read_write> zr: array<atomic<u32>, 2>;
@group(0) @binding(16) var minz1_out: texture_storage_2d<r32float, write>;
@group(0) @binding(17) var minz1: texture_2d<f32>;
@group(0) @binding(18) var minz2_out: texture_storage_2d<r32float, write>;
@group(0) @binding(19) var minz2: texture_2d<f32>;
@group(0) @binding(20) var glass: texture_2d<f32>;
// Per pixel of the band: colour sums with alpha, then the weights.
@group(0) @binding(21) var<storage, read_write> sums: array<vec4<f32>>;
// What is likely behind each pixel where something nearer hides it: the
// farthest of what is seen around it, as far as its blur reaches.
@group(0) @binding(22) var back_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(23) var back: texture_2d<f32>;
@group(0) @binding(24) var backz_out: texture_storage_2d<r32float, write>;
@group(0) @binding(25) var backz: texture_2d<f32>;

const RT_N: u32 = 256u;
const RT_SLOPE: u32 = 3u;
const RT_AIM: u32 = 3u + 256u;
const RT_HEIGHT: u32 = 3u + 512u;
const LAMBDA_D: f32 = 0.58756;
const STEPS: u32 = 160u;
// How deep a surface is taken to be, as a share of its distance. A ray further
// behind it than this has gone round its edge.
const THICK: f32 = 0.05;

struct Hit {
    p: vec3<f32>,
    n: vec3<f32>,
    ok: bool,
}

struct Ray {
    o: vec3<f32>,
    d: vec3<f32>,
    w: f32,
}

fn surfaces() -> u32 {
    return u32(lens[0].x);
}

fn surf(i: u32) -> vec4<f32> {
    return lens[4u + 2u * i];
}

// The index of the glass after surface `i` at `lambda` µm, as the bake has it.
fn ior_after(i: u32, lambda: f32) -> f32 {
    let g = lens[5u + 2u * i];
    let d = 1.0 / (LAMBDA_D * LAMBDA_D);
    let l = 1.0 / (lambda * lambda);
    return g.x + g.y * (d + lens[3].x * (l - d));
}

// Where a ray meets a sphere of `radius` with its vertex at `z`, the root
// nearest the vertex, and the normal facing the ray. Mirrors lens_blur.rs.
fn sphere(o: vec3<f32>, d: vec3<f32>, radius: f32, z: f32) -> Hit {
    var out: Hit;
    out.ok = false;
    if (abs(d.z) < 1e-9) {
        return out;
    }
    if (abs(radius) < 1e-6) {
        let t = (z - o.z) / d.z;
        if (t > 1e-6) {
            out.p = o + d * t;
            out.n = vec3<f32>(0.0, 0.0, -sign(d.z));
            out.ok = true;
        }
        return out;
    }
    let c = vec3<f32>(0.0, 0.0, z + radius);
    let oc = o - c;
    let b = dot(oc, d);
    let cc = dot(oc, oc) - radius * radius;
    let disc = b * b - cc;
    if (disc < 0.0) {
        return out;
    }
    let sd = sqrt(disc);
    let t1 = -b - sd;
    let t2 = -b + sd;
    var t = 0.0;
    if (t1 > 1e-6 && t2 > 1e-6) {
        t = select(t2, t1, abs((o + d * t1).z - z) < abs((o + d * t2).z - z));
    } else if (t1 > 1e-6) {
        t = t1;
    } else if (t2 > 1e-6) {
        t = t2;
    } else {
        return out;
    }
    out.p = o + d * t;
    var n = (out.p - c) / abs(radius);
    if (dot(n, d) > 0.0) {
        n = -n;
    }
    out.n = n;
    out.ok = true;
    return out;
}

// Snell's law, `o` the ratio of the indices. A zero vector where the ray is
// turned back.
fn refract_ray(i: vec3<f32>, n: vec3<f32>, o: f32) -> vec3<f32> {
    let cos_i = -dot(i, n);
    let sin2_t = o * o * (1.0 - cos_i * cos_i);
    if (sin2_t >= 1.0) {
        return vec3<f32>(0.0);
    }
    let v = o * i + (o * cos_i - sqrt(1.0 - sin2_t)) * n;
    let sq = dot(v, v);
    if (!(sq > 1e-18)) {
        return vec3<f32>(0.0);
    }
    return v / sqrt(sq);
}

// The iris's opening at (u, v), its radius 1. Mirrors lens_flare::pupil_mask.
fn iris_at(u: f32, v: f32) -> f32 {
    let r = sqrt(u * u + v * v);
    let blades = clamp(u32(lens[1].w), 3u, 16u);
    let sector = TAU / f32(blades);
    let apothem = cos(3.14159265 / f32(blades));
    var a = (atan2(v, u) - lens[2].w) % sector;
    if (a < 0.0) {
        a += sector;
    }
    let poly = apothem / cos(a - sector * 0.5);
    let bound = poly + (1.0 - poly) * clamp(lens[1].z, 0.0, 1.0);
    let soft = max(0.04 * bound, 1e-4);
    let t = clamp((r - (bound - soft)) / soft, 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

fn front_glass(u: vec2<f32>) -> f32 {
    if (lens[2].z < 0.5) {
        return 1.0;
    }
    let n = i32(textureDimensions(glass).x);
    let c = clamp(vec2<i32>((u * 0.5 + 0.5) * f32(n)), vec2<i32>(0), vec2<i32>(n - 1));
    return textureLoad(glass, c, 0).r;
}

// A ray from sensor point `s` towards `aim` on the last surface's vertex plane,
// back through the lens. `clip` false ignores every edge and the iris, which
// is what finding a chief ray wants.
fn trace_back(s: vec3<f32>, aim: vec2<f32>, lambda: f32, clip: bool) -> Ray {
    var out: Ray;
    out.w = 0.0;
    let n = surfaces();
    let last = surf(n - 1u);
    var o = s;
    var d = normalize(vec3<f32>(aim, last.y) - s);
    var n1 = ior_after(n - 1u, lambda);
    var w = 1.0;
    for (var k = 0u; k < n; k++) {
        let i = n - 1u - k;
        let sf = surf(i);
        let h = sphere(o, d, sf.x, sf.y);
        if (!h.ok) {
            return out;
        }
        if (clip) {
            if (sf.w > 0.5) {
                w *= iris_at(h.p.x / lens[0].y, h.p.y / lens[0].y);
                if (w <= 0.0) {
                    return out;
                }
            } else if (dot(h.p.xy, h.p.xy) > sf.z * sf.z) {
                return out;
            }
            if (i == 0u) {
                w *= front_glass(h.p.xy / max(sf.z, 1e-3));
            }
        }
        var n2 = 1.0;
        if (i > 0u) {
            n2 = ior_after(i - 1u, lambda);
        }
        d = refract_ray(d, h.n, n1 / n2);
        if (dot(d, d) < 0.5) {
            return out;
        }
        n1 = n2;
        o = h.p;
    }
    if (d.z >= -1e-6) {
        return out;
    }
    out.o = o;
    out.d = d;
    out.w = w;
    return out;
}

// The height a ray from `s` towards `aim` crosses the stop at, edges ignored.
fn stop_height(s: vec3<f32>, aim: vec2<f32>) -> f32 {
    let n = surfaces();
    let last = surf(n - 1u);
    var o = s;
    var d = normalize(vec3<f32>(aim, last.y) - s);
    var n1 = ior_after(n - 1u, 0.55);
    for (var k = 0u; k < n; k++) {
        let i = n - 1u - k;
        let sf = surf(i);
        let h = sphere(o, d, sf.x, sf.y);
        if (!h.ok) {
            return 1e9;
        }
        if (sf.w > 0.5) {
            return h.p.y;
        }
        var n2 = 1.0;
        if (i > 0u) {
            n2 = ior_after(i - 1u, 0.55);
        }
        d = refract_ray(d, h.n, n1 / n2);
        if (dot(d, d) < 0.5) {
            return 1e9;
        }
        n1 = n2;
        o = h.p;
    }
    return 1e9;
}

// Where the sensor sits for the focus distance: a green ray from the axis at
// that distance crosses the axis behind the lens by so much more than one
// from infinity, and the sensor moves by the same.
fn sensor_z() -> f32 {
    let dist_mm = focus_metres() * 1000.0;
    let n = surfaces();
    let first = surf(0u);
    let obj = vec3<f32>(0.0, 0.0, lens[1].y - dist_mm);
    // Aimed at the focus ray's height on the entrance pupil.
    var d = normalize(vec3<f32>(0.0, lens[2].y, lens[1].y) - obj);
    var o = obj;
    var n1 = 1.0;
    for (var i = 0u; i < n; i++) {
        let sf = surf(i);
        let h = sphere(o, d, sf.x, sf.y);
        if (!h.ok) {
            return lens[0].z;
        }
        let n2 = ior_after(i, 0.55);
        d = refract_ray(d, h.n, n1 / n2);
        if (dot(d, d) < 0.5) {
            return lens[0].z;
        }
        n1 = n2;
        o = h.p;
    }
    if (abs(d.y) < 1e-9 || d.z <= 0.0) {
        return lens[0].z;
    }
    let cross = o.z - o.y * d.z / d.y;
    return lens[0].z + (cross - lens[2].x);
}

fn sensor_height(k: u32) -> f32 {
    return lens[3].y * 1.1 * f32(k) / f32(RT_N - 1u);
}

@compute @workgroup_size(64)
fn lb_ray_setup(@builtin(global_invocation_id) gid: vec3<u32>) {
    let k = gid.x;
    if (k >= RT_N) {
        return;
    }
    let sz = sensor_z();
    let s = vec3<f32>(0.0, sensor_height(k), sz);
    // The chief ray: aimed so it crosses the middle of the stop. Secant steps
    // on its aim height.
    var a = 0.0;
    var b = s.y * 0.5;
    var fa = stop_height(s, vec2<f32>(0.0, a));
    var fb = stop_height(s, vec2<f32>(0.0, b));
    for (var it = 0; it < 16; it++) {
        if (abs(fb - fa) < 1e-9 || abs(fb) < 1e-5) {
            break;
        }
        let c = b - fb * (b - a) / (fb - fa);
        a = b;
        fa = fb;
        b = c;
        fb = stop_height(s, vec2<f32>(0.0, b));
    }
    let chief = trace_back(s, vec2<f32>(0.0, b), 0.55, false);
    var slope = 0.0;
    if (chief.w > 0.0) {
        slope = length(chief.d.xy) / max(-chief.d.z, 1e-6);
    }
    rt[RT_SLOPE + k] = slope;
    rt[RT_AIM + k] = b;
    if (k == 0u) {
        rt[0] = sz;
        // How far from the axis on the last surface a ray from the middle of
        // the sensor can be aimed and still get through.
        var lo = 0.0;
        var hi = surf(surfaces() - 1u).z * 1.5;
        for (var it = 0; it < 24; it++) {
            let mid = 0.5 * (lo + hi);
            let r = trace_back(vec3<f32>(0.0, 0.0, sz), vec2<f32>(0.0, mid), 0.55, true);
            if (r.w > 0.0) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        rt[2] = lo;
    }
}

@compute @workgroup_size(64)
fn lb_ray_invert(@builtin(global_invocation_id) gid: vec3<u32>) {
    let j = gid.x;
    if (j >= RT_N) {
        return;
    }
    let top = rt[RT_SLOPE + RT_N - 1u];
    if (j == 0u) {
        rt[1] = top;
    }
    let want = top * f32(j) / f32(RT_N - 1u);
    // The sensor height whose chief slope is `want`: slopes rise with height.
    var lo = 0u;
    var hi = RT_N - 1u;
    while (hi - lo > 1u) {
        let mid = (lo + hi) / 2u;
        if (rt[RT_SLOPE + mid] < want) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let s0 = rt[RT_SLOPE + lo];
    let s1 = rt[RT_SLOPE + hi];
    let f = clamp((want - s0) / max(s1 - s0, 1e-9), 0.0, 1.0);
    rt[RT_HEIGHT + j] = mix(sensor_height(lo), sensor_height(hi), f);
}

fn frame_mm(px: vec2<f32>) -> vec2<f32> {
    return (px - vec2<f32>(f32(p.w), f32(p.h)) * 0.5) / p.px_per_mm;
}

fn frame_px(mm: vec2<f32>) -> vec2<f32> {
    return mm * p.px_per_mm + vec2<f32>(f32(p.w), f32(p.h)) * 0.5;
}

// The pixel a point in front of the lens is seen at, mm on the lens's axis.
fn project(q: vec3<f32>) -> vec2<f32> {
    let dist = max(lens[1].y - q.z, 1e-3);
    let lat = length(q.xy);
    let slope = lat / dist;
    let top = max(rt[1], 1e-9);
    let x = slope / top * f32(RT_N - 1u);
    let i0 = u32(clamp(floor(x), 0.0, f32(RT_N - 2u)));
    let h0 = rt[RT_HEIGHT + i0];
    let h1 = rt[RT_HEIGHT + i0 + 1u];
    let r = h0 + (h1 - h0) * (x - f32(i0));
    var dir = vec2<f32>(0.0, 0.0);
    if (lat > 1e-6) {
        dir = -q.xy / lat;
    }
    return frame_px(dir * r);
}

fn depth_px(px: vec2<f32>) -> f32 {
    let c = clamp(vec2<i32>(floor(px)), vec2<i32>(0), vec2<i32>(i32(p.w) - 1, i32(p.h) - 1));
    return textureLoad(info, c, 0).y;
}

fn min_at(px: vec2<f32>, level: u32) -> f32 {
    let c = vec2<i32>(floor(px));
    if (level == 0u) {
        return depth_px(px);
    }
    if (level == 1u) {
        let n = vec2<i32>(textureDimensions(minz1)) - 1;
        return textureLoad(minz1, clamp(c / 8, vec2<i32>(0), n), 0).r;
    }
    let n = vec2<i32>(textureDimensions(minz2)) - 1;
    return textureLoad(minz2, clamp(c / 64, vec2<i32>(0), n), 0).r;
}

// The source at a pixel position, bilinear, edges held.
fn source_at(px: vec2<f32>) -> vec4<f32> {
    let q = px - 0.5;
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
fn lb_ray_min1(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = vec2<u32>(textureDimensions(minz1_out));
    if (gid.x >= n.x || gid.y >= n.y) {
        return;
    }
    var m = 1e30;
    var far = 0.0;
    for (var y = 0; y < 8; y++) {
        for (var x = 0; x < 8; x++) {
            let px = vec2<i32>(gid.xy) * 8 + vec2<i32>(x, y);
            if (px.x < i32(p.w) && px.y < i32(p.h)) {
                let z = textureLoad(info, px, 0).y;
                m = min(m, z);
                far = max(far, z);
            }
        }
    }
    textureStore(minz1_out, vec2<i32>(gid.xy), vec4<f32>(m));
    atomicMin(&zr[0], bitcast<u32>(m));
    atomicMax(&zr[1], bitcast<u32>(far));
}

@compute @workgroup_size(8, 8)
fn lb_ray_min2(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = vec2<u32>(textureDimensions(minz2_out));
    if (gid.x >= n.x || gid.y >= n.y) {
        return;
    }
    let lim = vec2<i32>(textureDimensions(minz1)) - 1;
    var m = 1e30;
    for (var y = 0; y < 8; y++) {
        for (var x = 0; x < 8; x++) {
            let c = vec2<i32>(gid.xy) * 8 + vec2<i32>(x, y);
            if (c.x <= lim.x && c.y <= lim.y) {
                m = min(m, textureLoad(minz1, c, 0).r);
            }
        }
    }
    textureStore(minz2_out, vec2<i32>(gid.xy), vec4<f32>(m));
}

@compute @workgroup_size(8, 8)
fn lb_ray_back(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let xy = vec2<i32>(gid.xy);
    let me = textureLoad(info, xy, 0);
    // As far as any blur that reaches this pixel does: rays from a blurred
    // neighbour pass behind a sharp thing here too.
    let reach = max(textureLoad(tiles, xy / TILE, 0).x, me.x) * 1.2 + 2.0;
    let last = vec2<i32>(i32(p.w) - 1, i32(p.h) - 1);
    let spin = hash(xy) * TAU;
    let k_n = 48u;
    var far = me.y;
    for (var k = 0u; k < k_n; k++) {
        let rr = reach * sqrt((f32(k) + 0.5) / f32(k_n));
        let a = f32(k) * 2.3999632 + spin;
        let j = clamp(xy + vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * rr)), vec2<i32>(0), last);
        far = max(far, textureLoad(info, j, 0).y);
    }
    // The colour of what sits at that distance, the nearest of it counting
    // most.
    var col = vec4<f32>(0.0);
    var n = 0.0;
    if (me.y >= far * 0.97) {
        col = textureLoad(src, xy, 0);
        n = 1.0;
    }
    for (var k = 0u; k < k_n; k++) {
        let rr = reach * sqrt((f32(k) + 0.5) / f32(k_n));
        let a = f32(k) * 2.3999632 + spin;
        let j = clamp(xy + vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * rr)), vec2<i32>(0), last);
        if (textureLoad(info, j, 0).y >= far * 0.97) {
            let f = rr / (0.25 * reach);
            let wk = 1.0 / (1.0 + f * f);
            col += textureLoad(src, j, 0) * wk;
            n += wk;
        }
    }
    textureStore(back_out, xy, col / max(n, 1.0));
    textureStore(backz_out, xy, vec4<f32>(far));
}

fn back_at(px: vec2<f32>) -> vec2<f32> {
    let c = clamp(vec2<i32>(floor(px)), vec2<i32>(0), vec2<i32>(i32(p.w) - 1, i32(p.h) - 1));
    return vec2<f32>(textureLoad(backz, c, 0).r, 0.0);
}

// Where a ray leaving the lens first meets the depth, as a pixel position and
// 0, or 1 where it went round a nearer thing's edge and met what is behind,
// or the last place it was seen if it meets neither.
fn march(r: Ray) -> vec3<f32> {
    let near = bitcast<f32>(atomicLoad(&zr[0]));
    let far = bitcast<f32>(atomicLoad(&zr[1]));
    // Distance from the entrance pupil, metres, at ray length t, and back.
    let z0 = (lens[1].y - r.o.z) / 1000.0;
    let dz = -r.d.z / 1000.0;
    var z = max(near * 0.999, z0 + 1e-4);
    let z_end = far * 1.001 + 1e-4;
    var level = 2u;
    var last = project(r.o + r.d * ((z - z0) / dz));
    var behind_lo = z;
    for (var i = 0u; i < STEPS; i++) {
        let q = r.o + r.d * ((z - z0) / dz);
        let px = project(q);
        last = px;
        let m = min_at(px, level);
        if (z < m) {
            // In front of everything in this block: move on about the
            // block's width on screen, no further than its nearest depth.
            let blk = f32(1u << (3u * level));
            let z2 = z * 1.01 + 1e-4;
            let px2 = project(r.o + r.d * ((z2 - z0) / dz));
            let rate = length(px2 - px) / (z2 - z);
            var adv = 0.7 * max(blk, 1.0) / max(rate, 1e-6);
            adv = max(adv, z * 1e-4 + 1e-5);
            behind_lo = z;
            z = min(z + adv, max(m, z + z * 1e-5));
            if (level < 2u) {
                level += 1u;
            }
            if (z >= z_end) {
                return vec3<f32>(px, 0.0);
            }
            continue;
        }
        if (level > 0u) {
            level -= 1u;
            continue;
        }
        let front = depth_px(px);
        if (z - front <= THICK * front) {
            // Into the surface here: close in on where it crossed.
            var lo = behind_lo;
            var hi = z;
            for (var b = 0; b < 8; b++) {
                let mid = 0.5 * (lo + hi);
                let mq = r.o + r.d * ((mid - z0) / dz);
                let mp = project(mq);
                if (mid >= depth_px(mp)) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return vec3<f32>(project(r.o + r.d * ((hi - z0) / dz)), 0.0);
        }
        // Behind it by more than its depth: round its edge, to what is behind.
        if (z >= back_at(px).x) {
            return vec3<f32>(px, 1.0);
        }
        let z2 = z * 1.01 + 1e-4;
        let px2 = project(r.o + r.d * ((z2 - z0) / dz));
        let rate = length(px2 - px) / (z2 - z);
        z = z + max(2.0 / max(rate, 1e-6), z * 1e-4 + 1e-5);
        if (z >= z_end) {
            return vec3<f32>(px, 1.0);
        }
    }
    return vec3<f32>(last, 0.0);
}

// The source as rays see it: what is over the highlight threshold boosted.
fn lit(c: vec4<f32>) -> vec4<f32> {
    let k = hi_frac(c);
    return vec4<f32>(c.rgb * (1.0 + k * (p.boost - 1.0)), c.a);
}

@compute @workgroup_size(8, 8)
fn lb_ray_trace(@builtin(global_invocation_id) gid: vec3<u32>) {
    let y = gid.y + part.x;
    if (gid.x >= p.w || y >= p.h) {
        return;
    }
    let xy = vec2<i32>(i32(gid.x), i32(y));
    let slot = (gid.y * p.w + gid.x) * 2u;
    var c = vec4<f32>(0.0);
    var wt = vec4<f32>(0.0);
    if (part.y > 0u) {
        c = sums[slot];
        wt = sums[slot + 1u];
    }
    let total = max(part.w, 3u);
    let per = (total + 2u) / 3u;
    let spin = hash(xy) * TAU;
    let sz = rt[0];
    let aim_r = rt[2] * 1.25;
    for (var i = part.y; i < part.z; i++) {
        let ch = i % 3u;
        let j = i / 3u;
        // From the pixel's middle: the source has already summed its own
        // area, so spreading the start over the pixel would soften it twice.
        let s2 = frame_mm(vec2<f32>(xy) + 0.5);
        let s = vec3<f32>(s2, sz);
        let h = length(s2);
        var e = vec2<f32>(0.0, 1.0);
        if (h > 1e-6) {
            e = s2 / h;
        }
        let hk = clamp(h / max(lens[3].y * 1.1, 1e-6) * f32(RT_N - 1u), 0.0, f32(RT_N - 1u));
        let k0 = u32(floor(hk));
        let k1 = min(k0 + 1u, RT_N - 1u);
        let chief = mix(rt[RT_AIM + k0], rt[RT_AIM + k1], hk - f32(k0));
        let rr = sqrt((f32(j) + 0.5) / f32(per)) * aim_r;
        let ang = f32(j) * 2.3999632 + spin + f32(ch) * 2.0943951;
        let aim = e * chief + vec2<f32>(cos(ang), sin(ang)) * rr;
        var lambda = 0.55;
        if (ch == 0u) {
            lambda = 0.62;
        } else if (ch == 2u) {
            lambda = 0.46;
        }
        let ray = trace_back(s, aim, lambda, true);
        if (ray.w <= 0.0) {
            continue;
        }
        let hit = march(ray);
        var col = source_at(hit.xy);
        if (hit.z > 0.5) {
            let bc = clamp(vec2<i32>(floor(hit.xy)), vec2<i32>(0), vec2<i32>(i32(p.w) - 1, i32(p.h) - 1));
            col = textureLoad(back, bc, 0);
        }
        col = lit(col);
        // This ray's channel, and alpha with green.
        let pick = vec4<f32>(f32(ch == 0u), f32(ch == 1u), f32(ch == 2u), f32(ch == 1u));
        c += pick * ray.w * col;
        wt += vec4<f32>(pick.xyz, 0.0) * ray.w;
    }
    sums[slot] = c;
    sums[slot + 1u] = wt;
}

@compute @workgroup_size(8, 8)
fn lb_ray_resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let y = gid.y + part.x;
    if (gid.x >= p.w || y >= p.h) {
        return;
    }
    let xy = vec2<i32>(i32(gid.x), i32(y));
    let slot = (gid.y * p.w + gid.x) * 2u;
    let c = sums[slot];
    let wt = sums[slot + 1u];
    let me = textureLoad(src, xy, 0);
    // A pixel none of whose rays got through keeps its own colour.
    let rgb = select(me.rgb, c.rgb / max(wt.xyz, vec3<f32>(1e-6)), wt.xyz > vec3<f32>(1e-6));
    let a = select(me.a, c.a / max(wt.y, 1e-6), wt.y > 1e-6);
    textureStore(acc_out, xy, vec4<f32>(rgb, a));
}

// ---- Ray traced through a 3D precomp ----
//
// When the Depth row is the layer's own precomp and it has 3D layers, rays
// meet those layers rather than a depth picture, so what a near layer hides is
// still there to be seen round its edge. Each ray leaves the lens, is turned
// into the comp camera's frame by the lens's own chief rays (so what is in
// focus lands where the camera drew it), and passes through every layer plane
// nearest first, picking up each one's colour by how much of it is left.
// What it misses is the precomp's 2D layers, seen as if far away.

// Header: layer count, comp width and height, zoom; then effect pixels per
// comp pixel, comp pixels per lens mm, 1 with a backdrop, 0. Per layer: its
// plane's u axis with opacity, v axis, and origin, in the camera's frame.
@group(0) @binding(26) var<storage, read> scene: array<vec4<f32>>;
@group(0) @binding(27) var layer0: texture_2d<f32>;
@group(0) @binding(28) var layer1: texture_2d<f32>;
@group(0) @binding(29) var layer2: texture_2d<f32>;
@group(0) @binding(30) var layer3: texture_2d<f32>;
@group(0) @binding(31) var layer4: texture_2d<f32>;
@group(0) @binding(32) var layer5: texture_2d<f32>;
@group(0) @binding(33) var layer6: texture_2d<f32>;
@group(0) @binding(34) var layer7: texture_2d<f32>;
@group(0) @binding(35) var layer8: texture_2d<f32>;
@group(0) @binding(36) var layer9: texture_2d<f32>;
@group(0) @binding(37) var layer10: texture_2d<f32>;
@group(0) @binding(38) var layer11: texture_2d<f32>;
@group(0) @binding(39) var backdrop: texture_2d<f32>;

const SCENE_MAX: u32 = 12u;

fn layer_dims(i: u32) -> vec2<i32> {
    switch (i) {
        case 0u: { return vec2<i32>(textureDimensions(layer0)); }
        case 1u: { return vec2<i32>(textureDimensions(layer1)); }
        case 2u: { return vec2<i32>(textureDimensions(layer2)); }
        case 3u: { return vec2<i32>(textureDimensions(layer3)); }
        case 4u: { return vec2<i32>(textureDimensions(layer4)); }
        case 5u: { return vec2<i32>(textureDimensions(layer5)); }
        case 6u: { return vec2<i32>(textureDimensions(layer6)); }
        case 7u: { return vec2<i32>(textureDimensions(layer7)); }
        case 8u: { return vec2<i32>(textureDimensions(layer8)); }
        case 9u: { return vec2<i32>(textureDimensions(layer9)); }
        case 10u: { return vec2<i32>(textureDimensions(layer10)); }
        default: { return vec2<i32>(textureDimensions(layer11)); }
    }
}

fn layer_texel(i: u32, c: vec2<i32>) -> vec4<f32> {
    switch (i) {
        case 0u: { return textureLoad(layer0, c, 0); }
        case 1u: { return textureLoad(layer1, c, 0); }
        case 2u: { return textureLoad(layer2, c, 0); }
        case 3u: { return textureLoad(layer3, c, 0); }
        case 4u: { return textureLoad(layer4, c, 0); }
        case 5u: { return textureLoad(layer5, c, 0); }
        case 6u: { return textureLoad(layer6, c, 0); }
        case 7u: { return textureLoad(layer7, c, 0); }
        case 8u: { return textureLoad(layer8, c, 0); }
        case 9u: { return textureLoad(layer9, c, 0); }
        case 10u: { return textureLoad(layer10, c, 0); }
        default: { return textureLoad(layer11, c, 0); }
    }
}

// Layer `i` at `uv` across its quad, bilinear, nothing outside it.
fn layer_at(i: u32, uv: vec2<f32>) -> vec4<f32> {
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    let n = layer_dims(i);
    let q = uv * vec2<f32>(n) - 0.5;
    let f = fract(q);
    let b = vec2<i32>(floor(q));
    let hi = n - 1;
    let t00 = layer_texel(i, clamp(b, vec2<i32>(0), hi));
    let t10 = layer_texel(i, clamp(b + vec2<i32>(1, 0), vec2<i32>(0), hi));
    let t01 = layer_texel(i, clamp(b + vec2<i32>(0, 1), vec2<i32>(0), hi));
    let t11 = layer_texel(i, clamp(b + vec2<i32>(1, 1), vec2<i32>(0), hi));
    return mix(mix(t00, t10, f.x), mix(t01, t11, f.x), f.y);
}

// The sensor height, mm, whose chief ray leaves the lens at `slope`.
fn height_by_slope(slope: f32) -> f32 {
    let top = max(rt[1], 1e-9);
    let x = slope / top * f32(RT_N - 1u);
    let i0 = u32(clamp(floor(x), 0.0, f32(RT_N - 2u)));
    let h0 = rt[RT_HEIGHT + i0];
    let h1 = rt[RT_HEIGHT + i0 + 1u];
    return h0 + (h1 - h0) * (x - f32(i0));
}

// What a ray leaving the lens finds among the layers, premultiplied.
fn scene_colour(r: Ray) -> vec4<f32> {
    let head = scene[1];
    let size = scene[0].yz;
    let zoom = scene[0].w;
    let centre = size * 0.5;
    // Where it crosses the entrance pupil, and its slope there.
    let t_ep = (lens[1].y - r.o.z) / r.d.z;
    let a = r.o.xy + r.d.xy * t_ep;
    let s = r.d.xy / (-r.d.z);
    let ts = length(s);
    // Its slope in the camera's frame: the comp pixel the lens's chief ray at
    // this slope lands on, seen through the camera. The image is upside down
    // behind a lens, so both turn half round.
    var dir = vec3<f32>(0.0, 0.0, 1.0);
    if (ts > 1e-9) {
        let comp_px = height_by_slope(ts) * p.px_per_mm / head.x;
        dir = vec3<f32>(-s / ts * (comp_px / zoom), 1.0);
    }
    let origin = vec3<f32>(centre - a * head.y, -zoom);
    let count = min(u32(scene[0].x), SCENE_MAX);
    var col = vec4<f32>(0.0);
    var done = 0u;
    var last_t = 0.0;
    // Nearest first: each pass takes the nearest plane past the last one.
    for (var seen = 0u; seen < count; seen++) {
        var best_t = 1e30;
        var best_i = SCENE_MAX;
        var best_uv = vec2<f32>(0.0);
        for (var i = 0u; i < count; i++) {
            let u_ax = scene[2u + 3u * i].xyz;
            let v_ax = scene[3u + 3u * i].xyz;
            let o = scene[4u + 3u * i].xyz;
            let nrm = cross(u_ax, v_ax);
            let den = dot(dir, nrm);
            if (abs(den) < 1e-12) {
                continue;
            }
            let t = dot(o - origin, nrm) / den;
            if (t <= last_t + 1e-6 || t >= best_t) {
                continue;
            }
            let rel = origin + dir * t - o;
            let cu = cross(v_ax, nrm);
            let cv = cross(nrm, u_ax);
            let uv = vec2<f32>(dot(rel, cu) / dot(u_ax, cu), dot(rel, cv) / dot(v_ax, cv));
            if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
                continue;
            }
            best_t = t;
            best_i = i;
            best_uv = uv;
        }
        if (best_i >= SCENE_MAX) {
            break;
        }
        last_t = best_t;
        let c = layer_at(best_i, best_uv) * scene[2u + 3u * best_i].w;
        col += (1.0 - col.a) * c;
        if (col.a >= 0.999) {
            return col;
        }
    }
    if (head.z > 0.5) {
        let n = vec2<i32>(textureDimensions(backdrop));
        let at = (centre + dir.xy * zoom) / size * vec2<f32>(n);
        let bc = clamp(vec2<i32>(floor(at)), vec2<i32>(0), n - 1);
        col += (1.0 - col.a) * textureLoad(backdrop, bc, 0);
    }
    return col;
}

@compute @workgroup_size(8, 8)
fn lb_ray_scene(@builtin(global_invocation_id) gid: vec3<u32>) {
    let y = gid.y + part.x;
    if (gid.x >= p.w || y >= p.h) {
        return;
    }
    let xy = vec2<i32>(i32(gid.x), i32(y));
    let slot = (gid.y * p.w + gid.x) * 2u;
    var c = vec4<f32>(0.0);
    var wt = vec4<f32>(0.0);
    if (part.y > 0u) {
        c = sums[slot];
        wt = sums[slot + 1u];
    }
    let total = max(part.w, 3u);
    let per = (total + 2u) / 3u;
    let spin = hash(xy) * TAU;
    let sz = rt[0];
    let aim_r = rt[2] * 1.25;
    let s2 = frame_mm(vec2<f32>(xy) + 0.5);
    let s = vec3<f32>(s2, sz);
    let h = length(s2);
    var e = vec2<f32>(0.0, 1.0);
    if (h > 1e-6) {
        e = s2 / h;
    }
    let hk = clamp(h / max(lens[3].y * 1.1, 1e-6) * f32(RT_N - 1u), 0.0, f32(RT_N - 1u));
    let k0 = u32(floor(hk));
    let k1 = min(k0 + 1u, RT_N - 1u);
    let chief = mix(rt[RT_AIM + k0], rt[RT_AIM + k1], hk - f32(k0));
    for (var i = part.y; i < part.z; i++) {
        let ch = i % 3u;
        let j = i / 3u;
        let rr = sqrt((f32(j) + 0.5) / f32(per)) * aim_r;
        let ang = f32(j) * 2.3999632 + spin + f32(ch) * 2.0943951;
        let aim = e * chief + vec2<f32>(cos(ang), sin(ang)) * rr;
        var lambda = 0.55;
        if (ch == 0u) {
            lambda = 0.62;
        } else if (ch == 2u) {
            lambda = 0.46;
        }
        let ray = trace_back(s, aim, lambda, true);
        if (ray.w <= 0.0) {
            continue;
        }
        let col = lit(scene_colour(ray));
        let pick = vec4<f32>(f32(ch == 0u), f32(ch == 1u), f32(ch == 2u), f32(ch == 1u));
        c += pick * ray.w * col;
        wt += vec4<f32>(pick.xyz, 0.0) * ray.w;
    }
    sums[slot] = c;
    sums[slot + 1u] = wt;
}
