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
const INV_TWO_PI: f32 = 0.15915494;
const SHADOW_GAP: f32 = 0.5;
const SHADOW_OPAQUE: f32 = 1e-3;

struct Light {
    // Four corners of the emitting rectangle in comp pixels. c0.w = 1 for a
    // light from before it had rows; the other three w are unused.
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
    c3: vec4<f32>,
    // rgb = scene-linear colour with intensity folded in, w = falloff px.
    colour: vec4<f32>,
    // xyz = spot axis, w = cos(half-angle) or < -1 for "not a spot".
    axis: vec4<f32>,
    // x = kind (0 point or spot, 1 area, 2 parallel, 3 ambient),
    // y = falloff (0 none, 1 smooth, 2 inverse square clamped),
    // z = radius px, w = cos of the feather's inner angle.
    flags: vec4<f32>,
    // x = shadow darkness 0..1, y = shadow diffusion px, z = which casters
    // can shade this layer, one bit each.
    shadow: vec4<f32>,
};

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

// Unit vector, or the zero vector when the input is too short to have a
// direction — the caller checks for zero rather than dividing by nothing.
fn normalise(a: vec3<f32>) -> vec3<f32> {
    let len = length(a);
    if (len < 1e-6) {
        return vec3<f32>(0.0);
    }
    return a / len;
}

fn corner(l: Light, i: u32) -> vec3<f32> {
    if (i == 0u) { return l.c0.xyz; }
    if (i == 1u) { return l.c1.xyz; }
    if (i == 2u) { return l.c2.xyz; }
    return l.c3.xyz;
}

// The cosine-weighted fraction of the hemisphere above `pos` that the
// rectangle covers, clipped to the horizon first (Sutherland-Hodgman against
// one plane; a convex quad keeps at most five corners).
fn rect_form_factor(pos: vec3<f32>, n: vec3<f32>, l: Light) -> f32 {
    var poly: array<vec3<f32>, 8>;
    var count: u32 = 0u;
    for (var i: u32 = 0u; i < 4u; i = i + 1u) {
        let a = corner(l, i) - pos;
        let b = corner(l, (i + 1u) % 4u) - pos;
        let da = dot(a, n);
        let db = dot(b, n);
        if (da >= 0.0 && count < 8u) {
            poly[count] = a;
            count = count + 1u;
        }
        if ((da >= 0.0) != (db >= 0.0) && count < 8u) {
            let t = da / (da - db);
            poly[count] = a + (b - a) * t;
            count = count + 1u;
        }
    }
    if (count < 3u) {
        return 0.0;
    }
    var sum: f32 = 0.0;
    for (var i: u32 = 0u; i < count; i = i + 1u) {
        let a = normalise(poly[i]);
        let b = normalise(poly[(i + 1u) % count]);
        if (all(a == vec3<f32>(0.0)) || all(b == vec3<f32>(0.0))) {
            return 0.0;
        }
        let edge = normalise(cross(a, b));
        if (all(edge == vec3<f32>(0.0))) {
            continue;
        }
        sum = sum + acos(clamp(dot(a, b), -1.0, 1.0)) * dot(edge, n);
    }
    // Magnitude, not the signed sum — see the oracle: the sign is only the
    // corner winding, and a light here emits from both faces.
    return min(abs(sum * INV_TWO_PI), 1.0);
}

fn light_centre(l: Light) -> vec3<f32> {
    return (l.c0.xyz + l.c1.xyz + l.c2.xyz + l.c3.xyz) * 0.25;
}

// The sRGB curve, carried through nought, as lumit_core::lighting::encode.
fn encode(x: vec3<f32>) -> vec3<f32> {
    let v = abs(x);
    let e = select(1.055 * pow(v, vec3<f32>(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3<f32>(0.0031308));
    return sign(x) * e;
}

fn decode(x: vec3<f32>) -> vec3<f32> {
    let v = abs(x);
    let d = select(pow((v + 0.055) / 1.055, vec3<f32>(2.4)), v / 12.92, v <= vec3<f32>(0.04045));
    return sign(x) * d;
}

// Matches lumit_core::lighting::smoothstep, including its degenerate branch —
// WGSL's own smoothstep is undefined when the edges are equal.
fn soft_step(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 <= edge0) {
        return select(0.0, 1.0, x >= edge1);
    }
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Full strength out to the radius, then the chosen falloff.
fn reach(d: f32, l: Light) -> f32 {
    let mode = u32(l.flags.y + 0.5);
    let r = l.flags.z;
    if (mode == 0u || d <= r) {
        return 1.0;
    }
    if (mode == 1u) {
        let legacy = l.c0.w > 0.5;
        let falloff_px = l.colour.w;
        if (falloff_px <= 0.0) {
            return select(0.0, 1.0, legacy && r <= 0.0);
        }
        let t = clamp(1.0 - (d - r) / falloff_px, 0.0, 1.0);
        return select(t, t * t, legacy);
    }
    return (r / d) * (r / d);
}

// How much of the light is left at `pos` after its falloff and its cone, or
// zero where none arrives; `to_light` is set to the way it arrives from.
fn arrival(pos: vec3<f32>, l: Light, to_light: ptr<function, vec3<f32>>) -> f32 {
    let kind = u32(l.flags.x + 0.5);
    if (kind == 3u) {
        return 0.0;
    }
    let centre = light_centre(l);
    *to_light = centre - pos;
    var r: f32 = 1.0;
    if (kind == 2u) {
        *to_light = -l.axis.xyz;
    } else {
        r = reach(length(*to_light), l);
    }
    if (r <= 0.0) {
        return 0.0;
    }

    let cone_cos = l.axis.w;
    var cone: f32 = 1.0;
    if (cone_cos >= -1.0 && kind != 2u) {
        let dir = normalise(pos - centre);
        if (all(dir == vec3<f32>(0.0))) {
            return 0.0;
        }
        cone = soft_step(cone_cos, l.flags.w, dot(dir, l.axis.xyz));
    }
    if (cone <= 0.0) {
        return 0.0;
    }
    return r * cone;
}

fn irradiance(pos: vec3<f32>, n: vec3<f32>, l: Light) -> f32 {
    var to_light = vec3<f32>(0.0);
    let strength = arrival(pos, l, &to_light);
    if (strength <= 0.0) {
        return 0.0;
    }
    var e: f32;
    if (u32(l.flags.x + 0.5) == 1u) {
        e = rect_form_factor(pos, n, l);
    } else {
        let dir = normalise(to_light);
        if (all(dir == vec3<f32>(0.0))) {
            e = 0.0;
        } else {
            e = max(dot(n, dir), 0.0);
        }
    }
    return e * strength;
}

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
        var spread = l.shadow.y;
        if (bounded) {
            spread = spread * (1.0 - t);
        }
        let texels = spread * c.place.z * size / max(length(c.u.xyz), 1e-6);
        let reach = texels / size;
        if (any(at < vec2<f32>(-reach)) || any(at > vec2<f32>(1.0 + reach))) {
            continue;
        }
        var s: vec4<f32>;
        if (texels < 0.5) {
            s = square_at(i, at, 0.0);
        } else {
            // Never further apart than a texel of the level read.
            let lod = log2(texels);
            let last = f32(textureNumLevels(squares) - 1u);
            let off = min(texels, exp2(min(lod, last))) * 0.5 / size;
            s = 0.25 * (square_at(i, at + vec2<f32>(-off, -off), lod)
                + square_at(i, at + vec2<f32>(off, -off), lod)
                + square_at(i, at + vec2<f32>(-off, off), lod)
                + square_at(i, at + vec2<f32>(off, off), lod));
        }
        through = through * clamp(1.0 - s.a + c.origin.w * s.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        if (all(through < vec3<f32>(SHADOW_OPAQUE))) {
            break;
        }
    }
    return through;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
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
    if (p.casters_head.z > 0.5) {
        // Unlit, so only the shadows touch it: each light that falls here
        // dims it by what its casters stopped.
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
            textureStore(dst, xy, vec4<f32>(0.0, 0.0, 0.0, o.a * clamp(1.0 - luma(dim), 0.0, 1.0)));
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
    var unshadowed = gain;
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
            unshadowed = unshadowed + p.material.x * colour;
            continue;
        }
        let lit = irradiance(pos, p.normal.xyz, l);
        let e = diffuse * lit;
        var left = vec3<f32>(1.0);
        if (takes && l.shadow.x > 0.0 && lit > 0.0) {
            left = 1.0 - l.shadow.x * (1.0 - transmitted(pos, l));
        }
        gain = gain + e * colour * left;
        unshadowed = unshadowed + e * colour;
        if (p.material.z > 0.0) {
            shine = shine + p.material.z * highlight(pos, p.normal.xyz, l) * colour * left;
        }
    }
    if (accepts == 2u) {
        // Black, as opaque as the share of light the shadows took.
        let full = luma(unshadowed);
        var taken = 0.0;
        if (full > 1e-6) {
            taken = clamp(1.0 - luma(gain) / full, 0.0, 1.0);
        }
        textureStore(dst, xy, vec4<f32>(0.0, 0.0, 0.0, o.a * taken));
        return;
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
