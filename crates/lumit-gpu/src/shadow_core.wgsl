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

// The models' shadows (mesh/shadow.rs): each light that casts, seen from where
// it shines, keeps how far the nearest model is at every texel of its map.
// Each that reads them declares `shadow_maps`, `maps` and `map_sampler` too.

struct ShadowMap {
    // x = its first view, y = how many (six round a light inside the models),
    // z = a texel's width per comp pixel of the view's w.
    view: vec4<f32>,
    // x = the nearest depth kept, y = how far past that the farthest is.
    range: vec4<f32>,
};

struct ShadowMaps {
    // x = how many lights have maps, y = the maps' side in texels.
    head: vec4<f32>,
    lights: array<ShadowMap, 8>,
    // Comp pixels to a view: xy = where on the map times w, z = depth in comp
    // pixels, w = 1, or the depth for a light that is somewhere.
    views: array<mat4x4<f32>, 48>,
};

const MAP_TAPS: u32 = 16u;
const MAP_MAX_SOFT: f32 = 24.0;
// Past this many texels the soft edge takes twice the taps, so it doesn't band.
const MAP_WIDE: f32 = 8.0;

// How much of light `l` gets past the models to `pos`, 0 to 1. `n` is the
// surface there, which it is read a little off so a model doesn't shade
// itself, nought in the air. A light's Shadow diffusion softens it as it
// does a 3D layer's shadow: wider the farther the model is from `pos`.
fn mapped(pos: vec3<f32>, n: vec3<f32>, l: Light) -> f32 {
    let slot = u32(l.c1.w + 0.5);
    if (slot == 0u || slot > u32(shadow_maps.head.x + 0.5)) {
        return 1.0;
    }
    let m = shadow_maps.lights[slot - 1u];
    var to_light = light_centre(l) - pos;
    if (u32(l.flags.x + 0.5) == 2u) {
        to_light = -l.axis.xyz;
    }
    var view = u32(m.view.x + 0.5);
    if (m.view.y > 1.5) {
        // All round the light: the face the way from it points along most.
        let d = -to_light;
        let a = abs(d);
        if (a.x >= a.y && a.x >= a.z) {
            view = view + select(1u, 0u, d.x > 0.0);
        } else if (a.y >= a.z) {
            view = view + select(3u, 2u, d.y > 0.0);
        } else {
            view = view + select(5u, 4u, d.z > 0.0);
        }
    }
    let seen = shadow_maps.views[min(view, 47u)];
    let w = (seen * vec4<f32>(pos, 1.0)).w;
    if (w <= 0.0) {
        return 1.0;
    }
    let texel = w * m.view.z;
    // A texel and a half off the face that looks at the light.
    let lift = select(-n, n, dot(n, to_light) >= 0.0) * texel * 1.5;
    let q = seen * vec4<f32>(pos + lift, 1.0);
    let uv = q.xy / q.w;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return 1.0;
    }
    let side = shadow_maps.head.y;
    let layer = i32(view);
    let here = q.z - texel;
    // Nothing past the farthest model casts, and an empty texel is there.
    let ref_depth = min((here - m.range.x) / m.range.y, 1.0);
    let soft = l.shadow.y;
    if (soft <= 0.0 || all(n == vec3<f32>(0.0))) {
        return textureSampleCompareLevel(maps, map_sampler, uv, layer, ref_depth);
    }
    // How far the models in the way are, on average, over the widest the
    // soft edge can be.
    let reach = clamp(soft / texel, 1.0, MAP_MAX_SOFT);
    let centre = uv * side;
    var sum: f32 = 0.0;
    var found: f32 = 0.0;
    for (var i: u32 = 0u; i < MAP_TAPS; i = i + 1u) {
        let at = centre + vec2<f32>(f32(i % 4u) - 1.5, f32(i / 4u) - 1.5) * reach / 1.5;
        let xy = clamp(vec2<i32>(at), vec2<i32>(0), vec2<i32>(i32(side) - 1));
        let raw = textureLoad(maps, xy, layer, 0);
        let d = m.range.x + raw * m.range.y;
        if (raw < 1.0 && d < here) {
            sum = sum + d;
            found = found + 1.0;
        }
    }
    if (found == 0.0) {
        return 1.0;
    }
    let blocker = sum / found;
    // As the 3D layers' shadows: the soft edge on the model times how much
    // farther the shadow lands.
    var away = max(blocker, 1.0);
    if (u32(l.flags.x + 0.5) == 2u) {
        away = max(length(light_centre(l) - pos), 1.0);
    }
    let radius = clamp(soft * (here - blocker) / away / texel, 0.5, MAP_MAX_SOFT);
    let taps = select(MAP_TAPS, MAP_TAPS * 2u, radius > MAP_WIDE);
    var lit: f32 = 0.0;
    for (var i: u32 = 0u; i < taps; i = i + 1u) {
        // A sunflower's seeds: even over the disc, no two on one ring.
        let r = sqrt((f32(i) + 0.5) / f32(taps)) * radius / side;
        let a = f32(i) * 2.39996;
        let at = uv + vec2<f32>(cos(a), sin(a)) * r;
        lit = lit + textureSampleCompareLevel(maps, map_sampler, at, layer, ref_depth);
    }
    return lit / f32(taps);
}
