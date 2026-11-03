// Particulate's particles in the comp's 3D scene (fx/particulate_scene.rs),
// appended to fx_particulate.wgsl. The particles are worked out as ever and
// drawn through the comp's camera onto the whole frame: nearest last when they
// cover each other, each faded where it meets a 3D layer in front of it, and
// each one's distance laid down for the compositor to join the picture by.

struct Scene {
    // The frame drawn into, px.
    frame: vec2<f32>,
    // One over clip w is this times the particle's own foreshortening.
    k: f32,
    // Clip w per raster pixel at a clip w of one: one over the zoom and the
    // render scale.
    soft: f32,
    // x: 1 to draw in `order`'s order, y: 1 when there is a depth to meet, z:
    // what that depth is counted from.
    flags: vec4<f32>,
    // The camera's depth of field: x the blur in raster pixels per unit of
    // `|distance − focus| / clip w`, nought for none, y the zoom, z the focus
    // distance, comp pixels.
    lens: vec4<f32>,
    // x: how much the layer's scale sizes each particle, y: 1 when each is
    // drawn across the shutter.
    look: vec4<f32>,
};

@group(3) @binding(0) var<uniform> sc: Scene;
// The particles sorted far to near, `(key, particle)`.
@group(3) @binding(1) var<storage, read> order: array<vec2<u32>>;
// Clip w of the nearest 3D layer at each pixel, the compositor's scene depth.
@group(3) @binding(2) var occluder: texture_2d<f32>;

struct VsScene {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) head: vec2<f32>,
    @location(1) @interpolate(flat) tail: vec2<f32>,
    @location(2) @interpolate(flat) colour: vec4<f32>,
    @location(3) @interpolate(flat) geom: vec4<f32>,
    @location(4) @interpolate(flat) stretch: vec2<f32>,
    // x: one over clip w, y: the particle's radius as clip w.
    @location(5) @interpolate(flat) seen: vec2<f32>,
};

@vertex
fn pt_vs_scene(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> VsScene {
    var ii = slot;
    if (sc.flags.x > 0.5) {
        ii = order[slot].y;
    }
    let seen = pt_seen(ii);
    let size = bitcast<f32>(dstream[r_size(dp.cap) + ii]) * seen.head.z * sc.look.x;
    let iw = sc.k * seen.head.z;
    // Out of focus a particle spreads over its circle of confusion, its light
    // with it.
    var blur = 0.0;
    if (sc.lens.x > 0.0 && iw > 0.0) {
        blur = sc.lens.x * abs(sc.lens.y / iw - sc.lens.z) * iw;
    }
    var g = pt_geo(ii, seen.head.xy, seen.tail.xy, size + 2.0 * blur);
    if (blur > 0.0) {
        g.geom.y = max(g.geom.y, blur);
        let share = size / max(size + 2.0 * blur, 1e-6);
        g.colour = g.colour * share * share;
    }
    if (sc.look.y > 0.5) {
        // Across the shutter its light is spread along the smear.
        g.colour = g.colour * size / max(size + length(g.head - g.tail), 1e-6);
    }
    var corner = vec2<f32>(-1.0, -1.0);
    if (vi == 1u || vi == 4u) { corner = vec2<f32>(1.0, -1.0); }
    if (vi == 2u || vi == 3u) { corner = vec2<f32>(-1.0, 1.0); }
    if (vi == 5u) { corner = vec2<f32>(1.0, 1.0); }
    let ay = vec2<f32>(-g.ax.y, g.ax.x);
    let px = g.centre + g.ax * (corner.x * (g.half.x + 1.0)) + ay * (corner.y * (g.half.y + 1.0));
    var out: VsScene;
    // Nearer is larger, as the compositor's depth goes.
    out.clip = vec4<f32>(
        px.x / sc.frame.x * 2.0 - 1.0,
        1.0 - px.y / sc.frame.y * 2.0,
        iw / (1.0 + iw),
        1.0,
    );
    out.head = g.head;
    out.tail = g.tail;
    out.colour = g.colour;
    out.geom = g.geom;
    out.stretch = g.stretch;
    out.seen = vec2<f32>(iw, g.geom.x * sc.soft / max(iw, 1e-9));
    return out;
}

// What the particle lays on this pixel, faded across its own size as it goes
// behind the nearest 3D layer there. One centred on a layer is half in front.
fn pt_scene_cover(in: VsScene) -> vec4<f32> {
    let v = VsOut(in.clip, in.head, in.tail, in.colour, in.geom, in.stretch);
    let c = pt_cover(in.clip.xy, v);
    if (sc.flags.y < 0.5 || in.seen.x <= 0.0) {
        return c;
    }
    let behind = textureLoad(occluder, vec2<i32>(in.clip.xy), 0).r + sc.flags.z;
    return c * clamp(0.5 + (behind - 1.0 / in.seen.x) / (2.0 * max(in.seen.y, 1e-6)), 0.0, 1.0);
}

@fragment
fn pt_fs_scene(in: VsScene) -> @location(0) vec4<f32> {
    return pt_scene_cover(in);
}

// One over clip w wherever the particle shows, the nearest kept by the depth
// test.
@fragment
fn pt_fs_scene_depth(in: VsScene) -> @location(0) vec4<f32> {
    if (pt_scene_cover(in).a <= 0.0) {
        discard;
    }
    return vec4<f32>(in.seen.x, 0.0, 0.0, 1.0);
}

// The sort's keys, far first: 4096 steps to each doubling of distance, from
// 256 times nearer than the comp's own plane to 256 times further. Particles
// as far as each other keep their birth order, the sort being stable.
@compute @workgroup_size(64)
fn pt_scene_keys(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let drawn = min(targs[1], dp.cap);
    if (i == 0u) {
        let blocks = (drawn + LANES - 1u) / LANES;
        scan[CTL_PAIRS] = drawn;
        scan[CTL_BLOCKS] = blocks;
        scan[CTL_LEN] = blocks * LANES;
    }
    if (i >= drawn) {
        return;
    }
    let octaves = clamp(8.0 + log2(max(pt_seen(i).head.z, 1e-30)), 0.0, 15.99);
    pairs_src[i] = vec2<u32>(u32(octaves * 4096.0), i);
}
