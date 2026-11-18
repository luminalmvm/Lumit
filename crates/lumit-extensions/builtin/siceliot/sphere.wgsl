// Sphere: wraps the layer round a ball and lights it, as the CC Sphere of
// After Effects. The layer is the skin of the ball, left to right round it and
// top to bottom from pole to pole. Rotation turns the ball; Render shows the
// outside, the inside seen through the clear parts of the skin, or both. One
// light gives the ball its shading, and Reflective mirrors the Reflection map
// (the layer itself when none is chosen) in its surface.

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
    /// @bounded(0, 100) @default(0) Reflective
    reflective: f32,
    /// @toggle @default(0) Internal shadows
    internal_shadows: u32,
    /// @toggle @default(0) Transparency falloff
    transparency_falloff: u32,
};

const SP_PI: f32 = 3.14159265;

// v turned about one axis: 0 is X, 1 is Y, 2 is Z.
fn sp_turn(v: vec3<f32>, axis: u32, rad: f32) -> vec3<f32> {
    let c = cos(rad);
    let s = sin(rad);
    switch axis {
        case 0u: { return vec3<f32>(v.x, v.y * c - v.z * s, v.y * s + v.z * c); }
        case 1u: { return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c); }
        default: { return vec3<f32>(v.x * c - v.y * s, v.x * s + v.y * c, v.z); }
    }
}

// The axes in the order the ball is turned about them.
fn sp_order() -> vec3<u32> {
    switch p.rotation_order {
        case 1u: { return vec3<u32>(0u, 2u, 1u); }
        case 2u: { return vec3<u32>(1u, 0u, 2u); }
        case 3u: { return vec3<u32>(1u, 2u, 0u); }
        case 4u: { return vec3<u32>(2u, 0u, 1u); }
        case 5u: { return vec3<u32>(2u, 1u, 0u); }
        default: { return vec3<u32>(0u, 1u, 2u); }
    }
}

fn sp_angle(axis: u32) -> f32 {
    switch axis {
        case 0u: { return radians(p.rotation_x); }
        case 1u: { return radians(p.rotation_y); }
        default: { return radians(p.rotation_z); }
    }
}

// A direction out of the ball as seen, taken back to the ball before it was
// turned: the three turns undone, last first.
fn sp_unturn(n: vec3<f32>) -> vec3<f32> {
    let o = sp_order();
    var v = sp_turn(n, o.z, -sp_angle(o.z));
    v = sp_turn(v, o.y, -sp_angle(o.y));
    return sp_turn(v, o.x, -sp_angle(o.x));
}

// Where on a picture laid round the ball a direction lands.
fn sp_wrap(d: vec3<f32>) -> vec2<f32> {
    let across = atan2(d.x, d.z) / (2.0 * SP_PI) + 0.5;
    let down = asin(clamp(d.y, -1.0, 1.0)) / SP_PI + 0.5;
    return vec2<f32>(across, down);
}

// The skin of the ball where its surface faces n.
fn sp_skin(n: vec3<f32>) -> vec4<f32> {
    return lumit_sample(sp_wrap(sp_unturn(n)));
}

// Where the light comes from, as a direction. Light direction is a turn from
// straight up, clockwise; Light height lifts the light towards the eye.
fn sp_light() -> vec3<f32> {
    let lift = clamp(p.light_height * 0.01, -1.0, 1.0);
    let flat = sqrt(max(1.0 - lift * lift, 0.0));
    let a = radians(p.light_direction);
    return vec3<f32>(sin(a) * flat, -cos(a) * flat, lift);
}

// The skin at a point whose surface faces n, lit. `shadow` is how much of the
// light something in front of it stops.
fn sp_lit(skin: vec4<f32>, n: vec3<f32>, shadow: f32) -> vec4<f32> {
    let light = sp_light();
    let colour = p.light_colour.rgb * p.light_intensity * 0.01 * (1.0 - shadow);
    let facing = max(dot(n, light), 0.0);
    // The eye is straight out of the screen.
    let half_way = normalize(light + vec3<f32>(0.0, 0.0, 1.0));
    let rough = clamp(p.roughness * 0.01, 0.01, 1.0);
    let shine = pow(max(dot(n, half_way), 0.0), 2.0 / (rough * rough));
    var rgb = skin.rgb
        * (vec3<f32>(p.ambient * 0.01) + colour * facing * p.diffuse * 0.01);
    // Metal takes the colour of the skin into its highlight.
    let straight = lumit_unpremult(skin).rgb;
    let highlight = mix(vec3<f32>(1.0), straight, clamp(p.metal * 0.01, 0.0, 1.0));
    rgb += colour * highlight * shine * p.specular * 0.01 * skin.a;
    if (p.reflective > 0.0) {
        // What the eye sees mirrored in the surface.
        let mirrored = 2.0 * n.z * n - vec3<f32>(0.0, 0.0, 1.0);
        let at = sp_wrap(mirrored);
        var seen = lumit_sample(at);
        if (lumit.input2_on > 0.5) {
            seen = lumit_sample2(at);
        }
        rgb += seen.rgb * p.reflective * 0.01 * skin.a;
    }
    return vec4<f32>(rgb, skin.a);
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let frame = lumit_size() / max(lumit.comp_scale, 1e-6);
    let centre = vec2<f32>(p.offset_x, p.offset_y) * 0.01 * frame;
    let q = uv * frame - centre;
    let r = max(p.radius, 1e-3);
    let d = length(q);
    // One raster pixel of soft edge.
    let edge = clamp((r - d) * lumit.comp_scale + 0.5, 0.0, 1.0);
    if (edge <= 0.0) {
        return vec4<f32>(0.0);
    }
    let flat = q / r;
    let z = sqrt(max(1.0 - dot(flat, flat), 0.0));
    let light = sp_light();

    // The near side: its surface faces the eye.
    let near = vec3<f32>(flat, z);
    var outside = sp_lit(sp_skin(near), near, 0.0);
    if (p.transparency_falloff != 0u) {
        // Clearer where the surface turns away from the eye.
        outside *= z;
    }

    // The far side, seen from within: its lit face looks back at the eye.
    let far = vec3<f32>(flat, -z);
    var shadow = 0.0;
    if (p.internal_shadows != 0u) {
        // The skin the light came in through, on its way to this point.
        let entry = far - 2.0 * dot(far, light) * light;
        shadow = sp_skin(entry).a;
    }
    let inside = sp_lit(sp_skin(far), vec3<f32>(-flat, z), shadow);

    var out = vec4<f32>(0.0);
    switch p.render {
        case 1u: { out = outside; }
        case 2u: { out = inside; }
        default: { out = outside + inside * (1.0 - outside.a); }
    }
    return out * edge;
}
