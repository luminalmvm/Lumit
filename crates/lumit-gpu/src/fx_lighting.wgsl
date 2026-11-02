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
// caster's square is read there, softer from a smaller level for diffusion.

const MAX_LIT_LIGHTS: u32 = 8u;
const MAX_SHADOW_CASTERS: u32 = 16u;
const SHADOW_GAP: f32 = 0.5;
const SHADOW_OPAQUE: f32 = 1e-3;

struct Caster {
    // xyz = the picture's top-left corner, w = Light transmission.
    origin: vec4<f32>,
    // xyz = its full width and height as edges in comp pixels.
    u: vec4<f32>,
    v: vec4<f32>,
    // Where the picture sits in its square: corner, width and height.
    place: vec4<f32>,
};

struct Params {
    // xyz = comp-space position of texel (0,0).
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
    // that takes no light, only shadows.
    casters_head: vec4<f32>,
    lights: array<Light, 8>,
    casters: array<Caster, 16>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var squares: texture_2d_array<f32>;
@group(0) @binding(5) var square_sampler: sampler;

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

// The square at `at`, its empty edge held beyond it, as the oracle's
// ShadowCaster::sample.
fn square_at(i: u32, at: vec2<f32>, lod: f32) -> vec4<f32> {
    return textureSampleLevel(squares, square_sampler, at, i, lod);
}

// How much of the light gets past the casters to `pos`, per channel, before
// the shadow's darkness, as lumit_core::lighting::transmitted. Only the
// casters in the light's mask can reach this layer.
fn transmitted(pos: vec3<f32>, l: Light) -> vec3<f32> {
    var through = vec3<f32>(1.0);
    let kind = u32(l.flags.x + 0.5);
    if (kind == 3u) {
        return through;
    }
    var dir = light_centre(l) - pos;
    var bounded = true;
    if (kind == 2u) {
        dir = -l.axis.xyz;
        bounded = false;
    }
    let reach = length(dir);
    if (reach < 1e-6) {
        return through;
    }
    let count = min(u32(p.casters_head.x), MAX_SHADOW_CASTERS);
    let mask = u32(l.shadow.z + 0.5);
    let size = p.casters_head.y;
    for (var i: u32 = 0u; i < count; i = i + 1u) {
        if (((mask >> i) & 1u) == 0u) {
            continue;
        }
        let c = p.casters[i];
        let n = cross(c.u.xyz, c.v.xyz);
        let den = dot(dir, n);
        if (abs(den) < 1e-12) {
            continue;
        }
        let t = dot(c.origin.xyz - pos, n) / den;
        if (t * reach <= SHADOW_GAP || (bounded && t >= 1.0)) {
            continue;
        }
        let rel = pos + dir * t - c.origin.xyz;
        let cu = cross(c.v.xyz, n);
        let cv = cross(n, c.u.xyz);
        let du = dot(c.u.xyz, cu);
        let dv = dot(c.v.xyz, cv);
        if (abs(du) < 1e-12 || abs(dv) < 1e-12) {
            continue;
        }
        let uv = vec2<f32>(dot(rel, cu) / du, dot(rel, cv) / dv);
        let at = c.place.xy + uv * c.place.zw;
        // As soft as the caster is far along from here to the light.
        var along = t;
        if (!bounded) {
            along = min(t / max(length(light_centre(l) - pos), 1.0), 1.0);
        }
        let spread = l.shadow.y * along;
        // Stretched the way the caster turns from the light.
        let facing = max(abs(den / (reach * length(n))), 1e-3);
        let ku = dot(dir, c.u.xyz) / (reach * max(length(c.u.xyz), 1e-6) * facing);
        let kv = dot(dir, c.v.xyz) / (reach * max(length(c.v.xyz), 1e-6) * facing);
        let st = min(sqrt(1.0 + vec2<f32>(ku * ku, kv * kv)), vec2<f32>(l.shadow.w));
        let texels = spread * c.place.z * size / max(length(c.u.xyz), 1e-6);
        let edge = texels * st / size;
        if (any(at < -edge) || any(at > 1.0 + edge)) {
            continue;
        }
        var s: vec4<f32>;
        if (texels * max(st.x, st.y) < 0.5) {
            s = square_at(i, at, 0.0);
        } else {
            // Never further apart than a texel of the level read, the level
            // for the radius between the two ways.
            let g = sqrt(st.x * st.y);
            let lod = log2(texels * g);
            let last = f32(textureNumLevels(squares) - 1u);
            let off = min(texels * g, exp2(min(lod, last))) * 0.5 / (g * size) * st;
            s = 0.25 * (square_at(i, at + vec2<f32>(-off.x, -off.y), lod)
                + square_at(i, at + vec2<f32>(off.x, -off.y), lod)
                + square_at(i, at + vec2<f32>(-off.x, off.y), lod)
                + square_at(i, at + vec2<f32>(off.x, off.y), lod));
        }
        through = through * clamp(1.0 - s.a + c.origin.w * s.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        if (all(through < vec3<f32>(SHADOW_OPAQUE))) {
            break;
        }
    }
    return through;
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
    if (live == 0u) {
        textureStore(dst, xy, o);
        return;
    }
    let f = vec2<f32>(f32(xy.x) + 0.5, f32(xy.y) + 0.5);
    let pos = p.origin.xyz + p.du.xyz * f.x + p.dv.xyz * f.y;
    let encoded = p.base.y > 0.5;
    let accepts = u32(p.base.z + 0.5);
    let takes = accepts != 0u && p.casters_head.x > 0.5;
    if (p.casters_head.z > 0.5 || accepts == 2u) {
        // Unlit, or showing only its shadows, so only the shadows touch it:
        // each light that falls here dims it by what its casters stopped.
        var dim = vec3<f32>(1.0);
        for (var i: u32 = 0u; i < MAX_LIT_LIGHTS; i = i + 1u) {
            if (i >= live) {
                break;
            }
            let l = p.lights[i];
            if (!takes || l.shadow.x <= 0.0 || irradiance(pos, p.normal.xyz, l) <= 0.0) {
                continue;
            }
            dim = dim * (1.0 - l.shadow.x * (1.0 - transmitted(pos, l)));
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
    var gain = vec3<f32>(p.base.x);
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
        let lit = irradiance(pos, p.normal.xyz, l);
        let e = diffuse * lit;
        var left = vec3<f32>(1.0);
        if (takes && l.shadow.x > 0.0 && lit > 0.0) {
            left = 1.0 - l.shadow.x * (1.0 - transmitted(pos, l));
        }
        gain = gain + e * colour * left;
        if (p.material.z > 0.0) {
            shine = shine + p.material.z * highlight(pos, p.normal.xyz, l) * colour * left;
        }
    }
    let metal = p.eye.w;
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
