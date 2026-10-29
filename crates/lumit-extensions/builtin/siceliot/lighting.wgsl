// Lighting: long shadows and a light, as Cavalry's Lighting Filter. Every
// clear pixel looks towards the light for something opaque in the way; if it
// finds it, it is in shadow. The layer's own pixels are brightened by the
// light, more the closer they are to it.
//
// The shadow can only fall inside the layer's picture, so use it on a layer
// as big as the comp (a precomp, or a full-size shape layer).

struct Params {
    /// @choice("Point", "Directional") @default("Point") Light
    light: u32,
    /// @point @default(400, 200) Light position
    light_position: vec2<f32>,
    /// @dial @default(45) Angle
    angle: f32,
    /// @slider(0, 2000) @default(400) @unit(px) Length
    length: f32,
    /// @bounded(0, 100) @default(50) Fade
    fade: f32,
    /// @default(0, 0, 0, 0.6) Shadow colour
    shadow_colour: vec4<f32>,
    /// @default(1, 0.95, 0.85, 1) Light colour
    light_colour: vec4<f32>,
    /// @slider(0, 400) @default(50) Light intensity
    light_intensity: f32,
    /// @slider(0, 4000) @default(1200) @unit(px) Light radius
    light_radius: f32,
};

const LI_STEPS: i32 = 48;

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let here = lumit_sample(uv);
    let px = lumit_px(uv);
    let to_px = lumit.comp_scale / lumit_size();
    var toward = -vec2<f32>(cos(radians(p.angle)), sin(radians(p.angle)));
    var reach = p.length;
    var dist = 0.0;
    if (p.light == 0u) {
        let v = p.light_position - px;
        dist = length(v);
        toward = v / max(dist, 1e-3);
        reach = min(p.length, dist);
    }
    // The most opaque thing between this pixel and the light, weaker the
    // further away it is when Fade is up.
    var block = 0.0;
    for (var i = 1; i <= LI_STEPS; i += 1) {
        let s = reach * f32(i) / f32(LI_STEPS);
        let q = (px + toward * s) * to_px;
        if (any(q < vec2<f32>(0.0)) || any(q > vec2<f32>(1.0))) {
            break;
        }
        let f = 1.0 - p.fade / 100.0 * s / max(p.length, 1e-3);
        block = max(block, lumit_sample(q).a * f);
    }
    let shadow = block * p.shadow_colour.a * (1.0 - here.a);
    var k = 1.0;
    if (p.light == 0u) {
        k = 1.0 - smoothstep(0.0, max(p.light_radius, 1e-3), dist);
    }
    let lit = here.rgb * (vec3<f32>(1.0) + p.light_colour.rgb * p.light_intensity / 100.0 * k);
    return vec4<f32>(lit + p.shadow_colour.rgb * shadow, here.a + shadow);
}
