// The lighting pass (docs/06-RENDER-PIPELINE.md). Mirrors
// lumit_core::lighting::shade op-for-op — that module is the oracle and
// carries the explanation of the maths; this file is its twin, not its
// documentation.
//
// One thread per texel: work out where the texel sits in comp space from the
// layer's affine plane, sum the diffuse form factor of each light rectangle
// over the hemisphere above it, and multiply the picture by 1 + that. Alpha is
// untouched, and premultiplied colour scales correctly under a scalar so there
// is no unpremultiply round trip.
//
// A light that casts shadows is dimmed by every caster between it and the
// texel: the ray back towards the light meets each caster's plane, and the
// caster's square is read there, softer from a smaller level for diffusion,
// and each model's shadow map (shadow_core.wgsl, prepended).

const MAX_LIT_LIGHTS: u32 = 8u;

struct Params {
    // xyz = comp-space position of texel (0,0), w = how much of an
    // environment it reflects.
    origin: vec4<f32>,
    // xyz = comp-space step per texel in x / in y.
    du: vec4<f32>,
    dv: vec4<f32>,
    // xyz = the plane's unit normal, w = how many of `lights` are live.
    normal: vec4<f32>,
    // x = ambient, y = diffuse, z = specular, w = highlight exponent.
    material: vec4<f32>,
    // xyz = the eye, w = metal.
    eye: vec4<f32>,
    // x = what the light starts from: 1 where it adds, 0 from black.
    // y = 1 to light the display-encoded picture.
    // z = accepts shadows: 0 off, 1 on, 2 only the shadows.
    // w = this layer's own caster, or -1.
    base: vec4<f32>,
    // x = how many casters, y = the side of their squares, z = 1 for a layer
    // that takes no light, only shadows, w = 1 for particles in the scene.
    casters_head: vec4<f32>,
    lights: array<Light, 8>,
    casters: array<Caster, 16>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var squares: texture_2d_array<f32>;
@group(0) @binding(5) var square_sampler: sampler;
// Particles in the scene: one over clip w at each texel, nought where none is.
// Their place is `origin + (du·x + dv·y + normal) / that`.
@group(0) @binding(6) var points_depth: texture_2d<f32>;

// How much of a light reaches a particle in the scene, lit as a small ball
// seen whole: the share of it lit as the camera sees it, so one lit from
// behind fades to a rim rather than going out.
fn ball(pos: vec3<f32>, l: Light) -> f32 {
    var to_light = light_centre(l) - pos;
    if (u32(l.flags.x + 0.5) == 2u) {
        to_light = -l.axis.xyz;
    }
    let ld = normalise(to_light);
    let seen = 0.5 + 0.5 * dot(normalise(p.eye.xyz - pos), ld);
    return irradiance(pos, ld, l) * seen;
}

// The light reaching `pos`, a plane's or a particle's.
fn lit_by(pos: vec3<f32>, l: Light) -> f32 {
    if (p.casters_head.w > 0.5) {
        return ball(pos, l);
    }
    return irradiance(pos, p.normal.xyz, l);
}
@group(0) @binding(8) var maps: texture_depth_2d_array;
@group(0) @binding(9) var map_sampler: sampler_comparison;
@group(0) @binding(10) var<uniform> shadow_maps: ShadowMaps;
// The comp's environment (environment_core.wgsl, prepended).
@group(0) @binding(11) var env_irradiance_cube: texture_cube<f32>;
@group(0) @binding(12) var env_specular_cube: texture_cube<f32>;
@group(0) @binding(13) var env_sampler: sampler;
@group(0) @binding(14) var<uniform> environment: Environment;

// Blinn-Phong, as lumit_core::lighting::highlight.
fn highlight(pos: vec3<f32>, n: vec3<f32>, l: Light) -> f32 {
    var to_light = vec3<f32>(0.0);
    let strength = arrival(pos, l, &to_light);
    if (strength <= 0.0) {
        return 0.0;
    }
    let ld = normalise(to_light);
    let v = normalise(p.eye.xyz - pos);
    if (all(ld == vec3<f32>(0.0)) || all(v == vec3<f32>(0.0))) {
        return 0.0;
    }
    if (dot(n, ld) <= 0.0 || dot(n, v) <= 0.0) {
        return 0.0;
    }
    let h = normalise(ld + v);
    if (all(h == vec3<f32>(0.0))) {
        return 0.0;
    }
    return pow(max(dot(n, h), 0.0), p.material.w) * strength;
}

@compute @workgroup_size(8, 8)
fn lighting(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let live = u32(p.normal.w);
    let surround = environment.colour.w > 0.5;
    if (live == 0u && !surround) {
        textureStore(dst, xy, o);
        return;
    }
    let f = vec2<f32>(f32(xy.x) + 0.5, f32(xy.y) + 0.5);
    var pos = p.origin.xyz + p.du.xyz * f.x + p.dv.xyz * f.y;
    if (p.casters_head.w > 0.5) {
        let iw = textureLoad(points_depth, xy, 0).r;
        if (iw <= 0.0) {
            textureStore(dst, xy, o);
            return;
        }
        pos = p.origin.xyz + (p.du.xyz * f.x + p.dv.xyz * f.y + p.normal.xyz) / iw;
    }
    let encoded = p.base.y > 0.5;
    let accepts = u32(p.base.z + 0.5);
    let takes = accepts != 0u && (p.casters_head.x > 0.5 || shadow_maps.head.x > 0.5);
    if (p.casters_head.z > 0.5 || accepts == 2u) {
        // Unlit, or showing only its shadows, so only the shadows touch it:
        // each light that falls here dims it by what its casters stopped.
        var dim = vec3<f32>(1.0);
        for (var i: u32 = 0u; i < MAX_LIT_LIGHTS; i = i + 1u) {
            if (i >= live) {
                break;
            }
            let l = p.lights[i];
            // Either face of a layer takes a shadow. A particle has no faces.
            if (!takes || l.shadow.x <= 0.0 || (lit_by(pos, l) <= 0.0
                && (p.casters_head.w > 0.5 || irradiance(pos, -p.normal.xyz, l) <= 0.0))) {
                continue;
            }
            let through = transmitted(pos, l) * mapped(pos, p.normal.xyz, l);
            dim = dim * (1.0 - l.shadow.x * (1.0 - through));
        }
        if (accepts == 2u) {
            // Just the shadow, in the colour a caster lets through.
            let d = select(dim, decode(dim), encoded);
            let lo = clamp(min(d.x, min(d.y, d.z)), 0.0, 1.0);
            textureStore(dst, xy, vec4<f32>((d - lo) * o.a, (1.0 - lo) * o.a));
        } else if (encoded) {
            if (o.a > 0.0) {
                textureStore(dst, xy, vec4<f32>(decode(encode(o.rgb / o.a) * dim) * o.a, o.a));
            } else {
                textureStore(dst, xy, o);
            }
        } else {
            textureStore(dst, xy, vec4<f32>(o.rgb * dim, o.a));
        }
        return;
    }
    // An environment surrounds everything, so with one a layer starts from
    // black, as a model does.
    var gain = vec3<f32>(select(p.base.x, 0.0, surround));
    var shine = vec3<f32>(0.0);
    let diffuse = p.material.y;
    for (var i: u32 = 0u; i < MAX_LIT_LIGHTS; i = i + 1u) {
        if (i >= live) {
            break;
        }
        let l = p.lights[i];
        let colour = l.colour.rgb;
        if (u32(l.flags.x + 0.5) == 3u) {
            gain = gain + p.material.x * colour;
            continue;
        }
        let lit = lit_by(pos, l);
        let e = diffuse * lit;
        var left = vec3<f32>(1.0);
        if (takes && l.shadow.x > 0.0 && lit > 0.0) {
            left = 1.0 - l.shadow.x * (1.0 - transmitted(pos, l) * mapped(pos, p.normal.xyz, l));
        }
        gain = gain + e * colour * left;
        if (p.material.z > 0.0) {
            shine = shine + p.material.z * highlight(pos, p.normal.xyz, l) * colour * left;
        }
    }
    let metal = p.eye.w;
    if (surround) {
        // The environment on a model's shading: Ambient takes its matte
        // light, Reflection is how much of it the layer reflects, tinted by
        // the layer as a metal is, and Shininess how sharply. Seen from
        // either side.
        let v = normalise(p.eye.xyz - pos);
        let n = select(p.normal.xyz, -p.normal.xyz, dot(p.normal.xyz, v) < 0.0);
        let k = clamp(p.origin.w, 0.0, 1.0);
        let roughness = 1.0 - clamp(p.material.w / 100.0, 0.0, 1.0);
        let matte = env_irradiance(env_irradiance_cube, env_sampler, environment, n);
        let mirror = env_radiance(env_specular_cube, env_sampler, environment, reflect(-v, n), roughness);
        let e = matte * p.material.x * (1.0 - k) + mirror * k;
        gain = gain + select(e, encode(e), encoded);
    }
    let lit = max(shine, vec3<f32>(0.0));
    let g = max(gain + metal * lit, vec3<f32>(0.0));
    if (encoded) {
        if (o.a <= 0.0) {
            textureStore(dst, xy, o);
            return;
        }
        // Out of premultiplied linear and into what After Effects would have
        // lit, and back.
        let v = encode(o.rgb / o.a) * g + (1.0 - metal) * lit;
        textureStore(dst, xy, vec4<f32>(decode(v) * o.a, o.a));
        return;
    }
    textureStore(dst, xy, vec4<f32>(o.rgb * g + (1.0 - metal) * lit * o.a, o.a));
}
