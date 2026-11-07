// The light maths a plane's lighting pass and a model's mesh pass share, so
// one light means the same to both. Prepended to fx_lighting.wgsl and to
// mesh.wgsl, as WGSL has no modules.

const INV_TWO_PI: f32 = 0.15915494;

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
    // x = shadow darkness 0..1, y = the widest soft edge on a caster px, z =
    // which casters can shade this layer, one bit each, w = how far that
    // edge can stretch on a caster turned from the light.
    shadow: vec4<f32>,
};

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
        return select(t * t * (3.0 - 2.0 * t), t * t, legacy);
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
