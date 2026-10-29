// Bend: curls the picture round a circle, as Cavalry's Bend Deformer does
// to a shape, but on pixels. From Centre along Angle, a stretch of Length is
// bent through Bend degrees; what lies past it carries on straight in the new
// direction, and what lies behind Centre is left alone.

struct Params {
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @dial @default(0) Angle
    angle: f32,
    /// @slider(-360, 360) @default(90) @unit(deg) Bend
    bend: f32,
    /// @slider(1, 4000) @default(600) @unit(px) Length
    length: f32,
};

fn bd_turn(v: vec2<f32>, a: f32) -> vec2<f32> {
    return vec2<f32>(v.x * cos(a) - v.y * sin(a), v.x * sin(a) + v.y * cos(a));
}

// Where the unbent picture had the point that bending put at `l`, in the
// bend's own axes: x along the axis from Centre, y across it.
fn bd_source(l: vec2<f32>) -> vec2<f32> {
    let theta = radians(p.bend);
    let len = max(p.length, 1.0);
    if (abs(theta) < 1e-4 || l.x <= 0.0) {
        return l;
    }
    let r = len / theta;
    let c = vec2<f32>(0.0, r);
    let s = sign(r);
    let v = l - c;
    let a = atan2(s * v.x, -s * v.y);
    // On the arc while the turn has the bend's sign and stays within it.
    if (a * s >= 0.0 && abs(a) <= abs(theta)) {
        return vec2<f32>(a * r, r - s * length(v));
    }
    // Past the arc: straight on from its end.
    let e = c + r * vec2<f32>(sin(theta), -cos(theta));
    let t = vec2<f32>(cos(theta), sin(theta));
    let n = vec2<f32>(-sin(theta), cos(theta));
    let u = dot(l - e, t);
    if (u > 0.0) {
        return vec2<f32>(len + u, dot(l - e, n));
    }
    return l;
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let a = radians(p.angle);
    let l = bd_turn(lumit_px(uv) - p.centre, -a);
    let src = p.centre + bd_turn(bd_source(l), a);
    let at = src * lumit.comp_scale / lumit_size();
    if (any(at < vec2<f32>(0.0)) || any(at > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    return lumit_sample(at);
}
