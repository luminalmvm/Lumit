// The ground a 3D viewport draws into its own picture: lines on the floor
// through the comp's middle, every `step` comp pixels and stronger every ten,
// hidden pixel by pixel wherever a 3D layer is nearer the eye. Not an effect
// and never exported: only a viewport's frames carry it.
//
// Each pixel's ray is met with the floor (y = floor), and how far the floor
// moves between this pixel and the next two says how wide a line is there, so
// lines stay a pixel wide and thin out where they close up towards the
// horizon instead of shimmering.

struct Params {
    // The eye, and the floor's height.
    eye: vec4<f32>,
    // The camera's right axis, and the comp's width.
    right: vec4<f32>,
    // Its axis down the picture, and the comp's height.
    up: vec4<f32>,
    // The way it looks, and its zoom.
    forward: vec4<f32>,
    // Linear, straight alpha.
    colour: vec4<f32>,
    // The middle's x and z, the step, and raster pixels per comp pixel.
    grid: vec4<f32>,
    // x: the clip w the scene depth is measured from.
    depth: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
// The scene's depth: clip w less `depth.x`, nearest 3D layer at each pixel.
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// Where the ray through comp pixel `q` meets the floor, as its x and z, and
// how far along the ray that is in clip w. No meeting reads as w <= 0.
fn floor_at(q: vec2<f32>) -> vec3<f32> {
    let d = p.right.xyz * (q.x - p.right.w * 0.5)
        + p.up.xyz * (q.y - p.up.w * 0.5)
        + p.forward.xyz * p.forward.w;
    if (abs(d.y) < 1e-6) {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    let t = (p.eye.w - p.eye.y) / d.y;
    let hit = p.eye.xyz + d * t;
    return vec3<f32>(hit.x, hit.z, t);
}

// How much of a line every `s` along one axis covers the pixel: `c` is where
// the pixel sits along that axis and `dc` how far it moves across one pixel.
// Fades out as the lines close up to a few pixels apart.
fn lines(c: f32, dc: f32, s: f32) -> f32 {
    let du = max(dc / s, 1e-6);
    let off = abs(fract(c / s + 0.5) - 0.5) / du;
    let cover = 1.0 - min(off, 1.0);
    return cover * smoothstep(3.0, 12.0, 1.0 / du);
}

@compute @workgroup_size(8, 8)
fn ground(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let k = max(p.grid.w, 1e-6);
    let q = (vec2<f32>(xy) + vec2<f32>(0.5)) / k;
    let h = floor_at(q);
    let hx = floor_at(q + vec2<f32>(1.0 / k, 0.0));
    let hy = floor_at(q + vec2<f32>(0.0, 1.0 / k));
    let scene = textureLoad(orig, xy, 0).r;
    if (h.z <= 0.0 || hx.z <= 0.0 || hy.z <= 0.0 || h.z - p.depth.x >= scene) {
        textureStore(dst, xy, o);
        return;
    }
    let at = h.xy - p.grid.xy;
    let moves = abs(hx.xy - h.xy) + abs(hy.xy - h.xy);
    let s = p.grid.z;
    let minor = max(lines(at.x, moves.x, s), lines(at.y, moves.y, s));
    let major = max(lines(at.x, moves.x, s * 10.0), lines(at.y, moves.y, s * 10.0));
    let a = clamp(max(minor * 0.5, major), 0.0, 1.0) * p.colour.a;
    textureStore(dst, xy, vec4<f32>(o.rgb * (1.0 - a) + p.colour.rgb * a, o.a + a * (1.0 - o.a)));
}
