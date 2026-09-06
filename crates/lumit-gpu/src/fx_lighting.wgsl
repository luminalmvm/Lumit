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

const MAX_LIT_LIGHTS: u32 = 8u;
const INV_TWO_PI: f32 = 0.15915494;

struct Light {
    // Four corners of the emitting rectangle in comp pixels, w unused.
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
    c3: vec4<f32>,
    // rgb = scene-linear colour with intensity folded in, w = falloff px.
    colour: vec4<f32>,
    // xyz = spot axis, w = cos(half-angle) or < -1 for "not a spot".
    axis: vec4<f32>,
    // x = 1 for an area light, 0 for a point or spot.
    flags: vec4<f32>,
};

struct Params {
    // xyz = comp-space position of texel (0,0).
    origin: vec4<f32>,
    // xyz = comp-space step per texel in x / in y.
    du: vec4<f32>,
    dv: vec4<f32>,
    // xyz = the plane's unit normal, w = how many of `lights` are live.
    normal: vec4<f32>,
    lights: array<Light, 8>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

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

// Matches lumit_core::lighting::smoothstep, including its degenerate branch —
// WGSL's own smoothstep is undefined when the edges are equal.
fn soft_step(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 <= edge0) {
        return select(0.0, 1.0, x >= edge1);
    }
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn irradiance(pos: vec3<f32>, n: vec3<f32>, l: Light) -> f32 {
    let centre = light_centre(l);
    let to_light = centre - pos;

    let falloff_px = l.colour.w;
    var reach: f32 = 1.0;
    if (falloff_px > 0.0) {
        let t = clamp(1.0 - length(to_light) / falloff_px, 0.0, 1.0);
        reach = t * t;
    }
    if (reach <= 0.0) {
        return 0.0;
    }

    let cone_cos = l.axis.w;
    var cone: f32 = 1.0;
    if (cone_cos >= -1.0) {
        let dir = normalise(pos - centre);
        if (all(dir == vec3<f32>(0.0))) {
            return 0.0;
        }
        let inner = cone_cos + (1.0 - cone_cos) * 0.1;
        cone = soft_step(cone_cos, inner, dot(dir, l.axis.xyz));
    }
    if (cone <= 0.0) {
        return 0.0;
    }

    var e: f32;
    if (l.flags.x > 0.5) {
        e = rect_form_factor(pos, n, l);
    } else {
        let dir = normalise(to_light);
        if (all(dir == vec3<f32>(0.0))) {
            e = 0.0;
        } else {
            e = max(dot(n, dir), 0.0);
        }
    }
    return e * reach * cone;
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
    var gain = vec3<f32>(1.0);
    for (var i: u32 = 0u; i < MAX_LIT_LIGHTS; i = i + 1u) {
        if (i >= live) {
            break;
        }
        let l = p.lights[i];
        gain = gain + irradiance(pos, p.normal.xyz, l) * l.colour.rgb;
    }
    textureStore(dst, xy, vec4<f32>(o.rgb * gain, o.a));
}
