// Lens flare's lights against a depth in metres. One thread a light, after
// detection and before the trace:
//
//   - a manual light with something nearer than it where it sits is hidden,
//     so it makes no ghosts and no starburst
//   - every light's starburst fades as the light goes out of focus, since a
//     defocused light is a disc, not a point to diffract round the blades
//
// The fade rides in the light's spare last word, 0 sharp and 1 gone, so a
// light no depth pass touched keeps its whole starburst.

struct Light {
    pos_x: f32,
    pos_y: f32,
    r: f32,
    g: f32,
    b: f32,
    ext_x: f32,
    ext_y: f32,
    fade: f32,
};

struct P {
    w: f32,
    h: f32,
    focal_mm: f32,
    fstop: f32,
    focus_m: f32,
    px_per_mm: f32,
    light_m: f32,
    manual: u32,
    count: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
};

@group(0) @binding(0) var depth: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> lights: array<Light>;
@group(0) @binding(2) var<uniform> p: P;

// Where a point `m` metres away comes to focus behind the lens, mm.
fn image_mm(m: f32) -> f32 {
    let f = p.focal_mm;
    let d = max(m * 1000.0, f * 1.001);
    return f * d / (d - f);
}

@compute @workgroup_size(16)
fn flare_depth(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.count) {
        return;
    }
    var l = lights[i];
    if (l.r <= 0.0 && l.g <= 0.0 && l.b <= 0.0) {
        return;
    }
    let size = vec2<i32>(textureDimensions(depth));
    let xy = clamp(
        vec2<i32>(vec2<f32>(l.pos_x * f32(size.x), l.pos_y * f32(size.y))),
        vec2<i32>(0),
        size - 1,
    );
    let scene = max(textureLoad(depth, xy, 0).r, 0.01);
    var dist = scene;
    if (p.manual == 1u) {
        if (scene < p.light_m * 0.98) {
            l.r = 0.0;
            l.g = 0.0;
            l.b = 0.0;
            lights[i] = l;
            return;
        }
        dist = p.light_m;
    }
    // The light's blur radius on the frame, px, and a fade that is half way
    // at 3 px.
    let blur = abs(image_mm(p.focus_m) - image_mm(dist)) / max(p.fstop, 0.5) * 0.5 * p.px_per_mm;
    let k = blur / 3.0;
    l.fade = 1.0 - 1.0 / (1.0 + k * k);
    lights[i] = l;
}
