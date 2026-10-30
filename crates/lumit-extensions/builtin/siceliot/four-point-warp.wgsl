// Four point warp: pins the layer's four corners to four points and can bow
// each edge in or out, as Cavalry's Four Point Warp. The defaults match a
// 1920 x 1080 layer, so it starts with no change. Each pixel is traced back
// to the picture with Newton's method.

struct Params {
    /// @point @default(0, 0) Top left
    top_left: vec2<f32>,
    /// @point @default(1920, 0) Top right
    top_right: vec2<f32>,
    /// @point @default(1920, 1080) Bottom right
    bottom_right: vec2<f32>,
    /// @point @default(0, 1080) Bottom left
    bottom_left: vec2<f32>,
    /// @slider(-1000, 1000) @default(0) @unit(px) Curve top
    curve_top: f32,
    /// @slider(-1000, 1000) @default(0) @unit(px) Curve right
    curve_right: f32,
    /// @slider(-1000, 1000) @default(0) @unit(px) Curve bottom
    curve_bottom: f32,
    /// @slider(-1000, 1000) @default(0) @unit(px) Curve left
    curve_left: f32,
};

// The outward side of the edge from a to b, for corners going clockwise.
fn fw_out(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let e = b - a;
    return normalize(vec2<f32>(e.y, -e.x) + vec2<f32>(1e-6));
}

// Where (u, v) of the picture lands: the four corners blended, each edge
// bowed by its Curve along its outward side.
fn fw_map(t: vec2<f32>) -> vec2<f32> {
    let u = t.x;
    let v = t.y;
    var q = (1.0 - v) * ((1.0 - u) * p.top_left + u * p.top_right)
        + v * ((1.0 - u) * p.bottom_left + u * p.bottom_right);
    q += p.curve_top * 4.0 * u * (1.0 - u) * (1.0 - v) * fw_out(p.top_left, p.top_right);
    q += p.curve_right * 4.0 * v * (1.0 - v) * u * fw_out(p.top_right, p.bottom_right);
    q += p.curve_bottom * 4.0 * u * (1.0 - u) * v * fw_out(p.bottom_right, p.bottom_left);
    q += p.curve_left * 4.0 * v * (1.0 - v) * (1.0 - u) * fw_out(p.bottom_left, p.top_left);
    return q;
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let target_px = lumit_px(uv);
    var t = vec2<f32>(0.5);
    let h = 1e-3;
    for (var i = 0; i < 16; i += 1) {
        let f = fw_map(t) - target_px;
        let ju = (fw_map(t + vec2<f32>(h, 0.0)) - fw_map(t)) / h;
        let jv = (fw_map(t + vec2<f32>(0.0, h)) - fw_map(t)) / h;
        let det = ju.x * jv.y - jv.x * ju.y;
        if (abs(det) < 1e-6) {
            break;
        }
        let step = vec2<f32>(jv.y * f.x - jv.x * f.y, -ju.y * f.x + ju.x * f.y) / det;
        t = clamp(t - step, vec2<f32>(-0.5), vec2<f32>(1.5));
    }
    let miss = length(fw_map(t) - target_px);
    if (miss > 0.5 || any(t < vec2<f32>(0.0)) || any(t > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    return lumit_sample(t);
}
