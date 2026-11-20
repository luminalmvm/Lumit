// Cylinder: wraps the layer round an open tube and lights it, as the CC
// Cylinder of After Effects. The layer is the skin of the tube: left to right
// round it, top to bottom along it. Rotation turns the tube; Render shows the
// outside, the inside seen through the clear parts of the skin, or both. The
// light, shading and Rotation order are Sphere's, so the two sit together.

struct Params {
    /// @dial @default(0) Rotation X
    rotation_x: f32,
    /// @dial @default(0) Rotation Y
    rotation_y: f32,
    /// @dial @default(0) Rotation Z
    rotation_z: f32,
    /// @choice("XYZ", "XZY", "YXZ", "YZX", "ZXY", "ZYX") @default("XYZ") Rotation order
    rotation_order: u32,
    /// @slider(0, 4000) @default(200) @unit(px) Radius
    radius: f32,
    /// @slider(0, 8000) @default(600) @unit(px) Height
    height: f32,
    /// @bounded(-50, 150) @default(50) Offset X (%)
    offset_x: f32,
    /// @bounded(-50, 150) @default(50) Offset Y (%)
    offset_y: f32,
    /// @choice("Full", "Outside", "Inside") @default("Full") Render
    render: u32,
    /// @slider(0, 400) @default(100) Light intensity
    light_intensity: f32,
    /// @default(1, 1, 1, 1) Light colour
    light_colour: vec4<f32>,
    /// @slider(-100, 100) @default(35) Light height
    light_height: f32,
    /// @dial @default(-60) Light direction
    light_direction: f32,
    /// @bounded(0, 100) @default(25) Ambient
    ambient: f32,
    /// @bounded(0, 100) @default(75) Diffuse
    diffuse: f32,
    /// @bounded(0, 100) @default(30) Specular
    specular: f32,
    /// @bounded(1, 100) @default(20) Roughness
    roughness: f32,
    /// @bounded(0, 100) @default(100) Metal
    metal: f32,
    /// @toggle @default(0) Transparency falloff
    transparency_falloff: u32,
};

const CY_PI: f32 = 3.14159265;

// v turned about one axis: 0 is X, 1 is Y, 2 is Z.
fn cy_turn(v: vec3<f32>, axis: u32, rad: f32) -> vec3<f32> {
    let c = cos(rad);
    let s = sin(rad);
    switch axis {
        case 0u: { return vec3<f32>(v.x, v.y * c - v.z * s, v.y * s + v.z * c); }
        case 1u: { return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c); }
        default: { return vec3<f32>(v.x * c - v.y * s, v.x * s + v.y * c, v.z); }
    }
}

// The axes in the order the tube is turned about them.
fn cy_order() -> vec3<u32> {
    switch p.rotation_order {
        case 1u: { return vec3<u32>(0u, 2u, 1u); }
        case 2u: { return vec3<u32>(1u, 0u, 2u); }
        case 3u: { return vec3<u32>(1u, 2u, 0u); }
        case 4u: { return vec3<u32>(2u, 0u, 1u); }
        case 5u: { return vec3<u32>(2u, 1u, 0u); }
        default: { return vec3<u32>(0u, 1u, 2u); }
    }
}

fn cy_angle(axis: u32) -> f32 {
    switch axis {
        case 0u: { return radians(p.rotation_x); }
        case 1u: { return radians(p.rotation_y); }
        default: { return radians(p.rotation_z); }
    }
}

// A vector as seen, taken back to the tube before it was turned.
fn cy_unturn(v0: vec3<f32>) -> vec3<f32> {
    let o = cy_order();
    var v = cy_turn(v0, o.z, -cy_angle(o.z));
    v = cy_turn(v, o.y, -cy_angle(o.y));
    return cy_turn(v, o.x, -cy_angle(o.x));
}

// A vector on the tube, turned as the tube is.
fn cy_returned(v0: vec3<f32>) -> vec3<f32> {
    let o = cy_order();
    var v = cy_turn(v0, o.x, cy_angle(o.x));
    v = cy_turn(v, o.y, cy_angle(o.y));
    return cy_turn(v, o.z, cy_angle(o.z));
}

// Where the light comes from, as a direction (as Sphere's).
fn cy_light() -> vec3<f32> {
    let lift = clamp(p.light_height * 0.01, -1.0, 1.0);
    let flat = sqrt(max(1.0 - lift * lift, 0.0));
    let a = radians(p.light_direction);
    return vec3<f32>(sin(a) * flat, -cos(a) * flat, lift);
}

// The skin at a point whose surface faces n (as seen), lit.
fn cy_lit(skin: vec4<f32>, n: vec3<f32>) -> vec4<f32> {
    let light = cy_light();
    let colour = p.light_colour.rgb * p.light_intensity * 0.01;
    let facing = max(dot(n, light), 0.0);
    let half_way = normalize(light + vec3<f32>(0.0, 0.0, 1.0));
    let rough = clamp(p.roughness * 0.01, 0.01, 1.0);
    let shine = pow(max(dot(n, half_way), 0.0), 2.0 / (rough * rough));
    var rgb = skin.rgb * (vec3<f32>(p.ambient * 0.01) + colour * facing * p.diffuse * 0.01);
    let straight = lumit_unpremult(skin).rgb;
    let highlight = mix(vec3<f32>(1.0), straight, clamp(p.metal * 0.01, 0.0, 1.0));
    rgb += colour * highlight * shine * p.specular * 0.01 * skin.a;
    return vec4<f32>(rgb, skin.a);
}

// One hit on the tube: the skin there, lit, and how much of the pixel the
// tube's top or bottom rim leaves it.
fn cy_hit(o: vec3<f32>, d: vec3<f32>, t: f32, inside: bool) -> vec4<f32> {
    let pt = o + d * t;
    let half_h = max(p.height, 1e-3) * 0.5;
    let rim = clamp((half_h - abs(pt.y)) * lumit.comp_scale + 0.5, 0.0, 1.0);
    if (rim <= 0.0) {
        return vec4<f32>(0.0);
    }
    let r = max(p.radius, 1e-3);
    let across = atan2(pt.x, pt.z) / (2.0 * CY_PI) + 0.5;
    let down = pt.y / (2.0 * half_h) + 0.5;
    let skin = lumit_sample(vec2<f32>(across, down));
    // The surface's way out, as seen; from inside the tube it faces in.
    var n = cy_returned(vec3<f32>(pt.x / r, 0.0, pt.z / r));
    if (inside) {
        n = -n;
    }
    var lit = cy_lit(skin, n);
    if (!inside && p.transparency_falloff != 0u) {
        lit *= abs(n.z);
    }
    return lit * rim;
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let frame = lumit_size() / max(lumit.comp_scale, 1e-6);
    let centre = vec2<f32>(p.offset_x, p.offset_y) * 0.01 * frame;
    let q = uv * frame - centre;
    let r = max(p.radius, 1e-3);
    // The eye looks straight in: a ray from in front of the tube, into the
    // screen, taken into the tube's own frame, where its axis is Y.
    let o = cy_unturn(vec3<f32>(q, 1e5));
    let d = cy_unturn(vec3<f32>(0.0, 0.0, -1.0));
    // x² + z² = r² along the ray.
    let a = d.x * d.x + d.z * d.z;
    if (a < 1e-8) {
        // Looking straight down the tube: its wall is edge-on.
        return vec4<f32>(0.0);
    }
    let b = o.x * d.x + o.z * d.z;
    let c = o.x * o.x + o.z * o.z - r * r;
    let disc = b * b - a * c;
    // How far inside the silhouette the ray passes, in pixels: one pixel of
    // soft edge.
    let gap = (o.x * d.z - o.z * d.x) / sqrt(a);
    let edge = clamp((r - abs(gap)) * lumit.comp_scale + 0.5, 0.0, 1.0);
    if (disc < 0.0 && edge <= 0.0) {
        return vec4<f32>(0.0);
    }
    let root = sqrt(max(disc, 0.0));
    let t_near = (-b - root) / a;
    let t_far = (-b + root) / a;
    let outside = cy_hit(o, d, t_near, false);
    let inside = cy_hit(o, d, t_far, true);
    var out = vec4<f32>(0.0);
    switch p.render {
        case 1u: { out = outside; }
        case 2u: { out = inside; }
        default: { out = outside + inside * (1.0 - outside.a); }
    }
    return out * edge;
}
