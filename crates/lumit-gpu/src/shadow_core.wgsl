// How much of a light gets past the 3D layers that cast shadows, shared by
// the lighting pass and the haze. Prepended after light_core.wgsl. Each that
// takes it declares what it reads: `p`, with `casters_head` and `casters`,
// then `squares` and `square_sampler`.

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

