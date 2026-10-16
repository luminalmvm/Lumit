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
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let xy = vec2<i32>(gid.xy);
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
    var n = ii;
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
