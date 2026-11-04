// The mesh pass (mesh.rs): a 3D model's triangles drawn through the comp's
// camera, shaded with its metal-rough materials and the comp's lights. Shaded
// in comp space, where the lights are, so a model and a plane under one light
// take the same light: light_core.wgsl and shadow_core.wgsl are prepended, the
// same maths planes read.
//
// Depth is a constant clip z over clip w, as a run of 3D layers cut by depth
// draws it (composite.rs, CUT_NEAR): nearer is larger, and nothing is far
// enough to be clipped.

// composite.rs's CUT_NEAR, which this has to match.
const CUT_NEAR: f32 = 1e-4;
const PI: f32 = 3.14159265;

struct Frame {
    // Comp pixels to clip space, its z left for the vertex to set.
    clip: mat4x4<f32>,
    // xyz = the eye in comp pixels, w = how many lights.
    eye: vec4<f32>,
    // x = 1 when the layer takes no light, y = how a view shades it (0 as it
    // exports, 1 in a studio light, 2 as its normals), z = 1 to tint its
    // faces by which way they face, w = whether it takes shadows: 0 no, 1 yes,
    // 2 only the shadows.
    flags: vec4<f32>,
    // The lines' colour, where a view draws them.
    wire: vec4<f32>,
    lights: array<Light, 8>,
};

struct Draw {
    // Model space to comp pixels.
    model: mat4x4<f32>,
    // The normals' matrix, its columns. normal0.w = 1 where the matrix
    // mirrors, which turns the faces inside out.
    normal0: vec4<f32>,
    normal1: vec4<f32>,
    normal2: vec4<f32>,
};

// A texture slot: the 2 by 3 transform of its uvs, which uv set it reads in
// row0.w, and whether it has a picture in row1.w.
struct Slot {
    row0: vec4<f32>,
    row1: vec4<f32>,
};

struct Material {
    base_colour: vec4<f32>,
    // rgb = emission with its strength in it.
    emissive: vec4<f32>,
    // metallic, roughness, normal scale, occlusion strength.
    params: vec4<f32>,
    // x = alpha mode (0 opaque, 1 mask, 2 blend), y = cutoff, z = unlit,
    // w = 1 with an opacity picture.
    mode: vec4<f32>,
    // transmission, IOR, clearcoat, clearcoat roughness.
    extra: vec4<f32>,
    // Base colour, metal-rough, normal, occlusion, emissive.
    slots: array<Slot, 5>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<uniform> draw: Draw;
// Each copy of the model: where it goes, in comp pixels, and its tint.
struct MeshCopy {
    place: mat4x4<f32>,
    tint: vec4<f32>,
};
@group(0) @binding(2) var<storage, read> instances: array<MeshCopy>;
@group(1) @binding(0) var<uniform> mat: Material;
@group(1) @binding(1) var base_tex: texture_2d<f32>;
@group(1) @binding(2) var mr_tex: texture_2d<f32>;
@group(1) @binding(3) var normal_tex: texture_2d<f32>;
@group(1) @binding(4) var occlusion_tex: texture_2d<f32>;
@group(1) @binding(5) var emissive_tex: texture_2d<f32>;
@group(1) @binding(6) var base_samp: sampler;
@group(1) @binding(7) var mr_samp: sampler;
@group(1) @binding(8) var normal_samp: sampler;
@group(1) @binding(9) var occlusion_samp: sampler;
@group(1) @binding(10) var emissive_samp: sampler;
// A layer's picture whose brightness is the opacity, on the base colour's uvs.
@group(1) @binding(11) var opacity_tex: texture_2d<f32>;
// The comp's picture under the model's run of 3D layers, for glass to bend.
@group(1) @binding(12) var behind_tex: texture_2d<f32>;
@group(1) @binding(13) var behind_samp: sampler;

// The shadows it takes: the 3D layers' casters and the models' maps.
struct Casters {
    casters_head: vec4<f32>,
    casters: array<Caster, 16>,
};
@group(2) @binding(0) var<uniform> p: Casters;
@group(2) @binding(1) var squares: texture_2d_array<f32>;
@group(2) @binding(2) var square_sampler: sampler;
@group(2) @binding(3) var maps: texture_depth_2d_array;
@group(2) @binding(4) var map_sampler: sampler_comparison;
@group(2) @binding(5) var<uniform> shadow_maps: ShadowMaps;

// The comp's environment (environment_core.wgsl, prepended).
@group(3) @binding(0) var env_irradiance_cube: texture_cube<f32>;
@group(3) @binding(1) var env_specular_cube: texture_cube<f32>;
@group(3) @binding(2) var env_sampler: sampler;
@group(3) @binding(3) var<uniform> environment: Environment;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(5) colour: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) at: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(5) colour: vec4<f32>,
};

@vertex
fn vs_mesh(v: VertexIn, @builtin(instance_index) slot: u32) -> VsOut {
    let n = mat3x3<f32>(draw.normal0.xyz, draw.normal1.xyz, draw.normal2.xyz);
    let place = instances[slot].place;
    let model = place * draw.model;
    let at = (model * vec4<f32>(v.position, 1.0)).xyz;
    var out: VsOut;
    out.pos = frame.clip * vec4<f32>(at, 1.0);
    out.pos.z = CUT_NEAR;
    out.at = at;
    out.normal = (place * vec4<f32>(n * v.normal, 0.0)).xyz;
    out.tangent = vec4<f32>((model * vec4<f32>(v.tangent.xyz, 0.0)).xyz, v.tangent.w);
    out.uv0 = v.uv0;
    out.uv1 = v.uv1;
    out.colour = v.colour * instances[slot].tint;
    return out;
}

fn slot_uv(s: Slot, in: VsOut) -> vec2<f32> {
    let uv = vec3<f32>(select(in.uv0, in.uv1, s.row0.w > 0.5), 1.0);
    return vec2<f32>(dot(s.row0.xyz, uv), dot(s.row1.xyz, uv));
}

// What is left of light `l` at `pos` past the shadows in its way, as a 3D
// layer works it out.
fn left_of(pos: vec3<f32>, n: vec3<f32>, l: Light) -> vec3<f32> {
    if (frame.flags.w < 0.5 || l.shadow.x <= 0.0) {
        return vec3<f32>(1.0);
    }
    return 1.0 - l.shadow.x * (1.0 - transmitted(pos, l) * mapped(pos, n, l));
}

// Karis' fit of the split-sum lookup, as a scale and bias on f0: how much a
// uniform light round about reflects off a surface this rough, at this angle.
fn env_fit(roughness: f32, nv: f32) -> vec2<f32> {
    let r = roughness * vec4<f32>(-1.0, -0.0275, -0.572, 0.022) + vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let a = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    return vec2<f32>(a * -1.04 + r.z, a * 1.04 + r.w);
}

// GGX with Smith's height-correlated visibility, in the units the diffuse
// term is in, which leaves out the pi.
fn ggx(a2: f32, nh: f32, nl: f32, nv: f32) -> f32 {
    let k = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * k * k);
    let vis = 0.5 / max(nl * sqrt(nv * nv * (1.0 - a2) + a2) + nv * sqrt(nl * nl * (1.0 - a2) + a2), 1e-6);
    return d * vis * PI * nl;
}

// What glass shows: the picture behind, where the ray through `at` lands once
// it has bent at the surface, softened by roughness. ponytail: screen-space,
// so it bends only what is under the run of 3D layers and never the model
// itself, and the ray goes as far past the surface as a third of the way back
// to the eye.
fn behind(at: vec3<f32>, eye: vec3<f32>, n: vec3<f32>, ior: f32, roughness: f32) -> vec4<f32> {
    let bent = refract(-eye, n, 1.0 / max(ior, 1.0));
    let far = at + bent * 0.35 * length(frame.eye.xyz - at);
    let clip = frame.clip * vec4<f32>(far, 1.0);
    let uv = vec2<f32>(0.5, -0.5) * clip.xy / max(clip.w, 1e-6) + 0.5;
    let spread = roughness * roughness * 0.12;
    var sum = textureSampleLevel(behind_tex, behind_samp, uv, 0.0);
    if (spread < 1e-4) {
        return sum;
    }
    for (var i = 0u; i < 24u; i = i + 1u) {
        let turn = f32(i) * 2.39996;
        let off = vec2<f32>(cos(turn), sin(turn)) * spread * sqrt((f32(i) + 0.5) / 24.0);
        sum = sum + textureSampleLevel(behind_tex, behind_samp, uv + off, 0.0);
    }
    return sum / 25.0;
}

// A surface no light touches: its own colour, and what glass shows in its
// place, tinted.
fn untouched(at: vec3<f32>, eye: vec3<f32>, n: vec3<f32>, own: vec3<f32>, tint: vec3<f32>, alpha: f32, glass: f32) -> vec4<f32> {
    let seen = behind(at, eye, n, mat.extra.y, 0.0);
    return vec4<f32>(own * alpha * (1.0 - glass) + seen.rgb * tint * glass, mix(alpha, seen.a, glass));
}

@fragment
fn fs_mesh(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = shade(in, front);
    if (frame.flags.z < 0.5) {
        return c;
    }
    // Fronts blue and backs red, the way the face is wound, shaded by how
    // bright the face is so the tint reads the same on any material.
    let tint = select(vec3<f32>(0.6, 0.02, 0.02), vec3<f32>(0.02, 0.1, 0.7), front != (draw.normal0.w > 0.5));
    let light = dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(tint * (0.35 * c.a + 0.65 * light), c.a);
}

fn shade(in: VsOut, front: bool) -> vec4<f32> {
    // Every picture and slope read first, while every pixel of the quad is
    // still running.
    let base_uv = slot_uv(mat.slots[0], in);
    let normal_uv = slot_uv(mat.slots[2], in);
    let base = mat.base_colour * in.colour * textureSample(base_tex, base_samp, base_uv);
    let mr = textureSample(mr_tex, mr_samp, slot_uv(mat.slots[1], in));
    var mapped = textureSample(normal_tex, normal_samp, normal_uv).xyz;
    // A layer's normals come as light, so back to how a normal map stores them.
    if (mat.slots[2].row1.w > 1.5) {
        mapped = select(1.055 * pow(mapped, vec3<f32>(1.0 / 2.4)) - 0.055, mapped * 12.92, mapped <= vec3<f32>(0.0031308));
    }
    mapped = mapped * 2.0 - 1.0;
    let occlusion = textureSample(occlusion_tex, occlusion_samp, slot_uv(mat.slots[3], in)).r;
    let glow = textureSample(emissive_tex, emissive_samp, slot_uv(mat.slots[4], in)).rgb;
    let shown = textureSample(opacity_tex, base_samp, base_uv).rgb;
    let dx = dpdx(in.at);
    let dy = dpdy(in.at);
    let uv_dx = dpdx(normal_uv);
    let uv_dy = dpdy(normal_uv);

    var alpha = base.a;
    if (mat.mode.w > 0.5) {
        alpha = alpha * clamp(dot(shown, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
    }
    let mode = u32(mat.mode.x + 0.5);
    if (mode == 1u) {
        if (alpha < mat.mode.y) {
            discard;
        }
        alpha = 1.0;
    } else if (mode == 0u) {
        alpha = 1.0;
    }
    let emissive = mat.emissive.rgb * glow;
    let look = u32(frame.flags.y + 0.5);
    let count = min(u32(frame.eye.w), 8u);
    let accepts = u32(frame.flags.w + 0.5);
    let surround = environment.colour.w > 0.5;
    let eye = normalise(frame.eye.xyz - in.at);
    // How much it lets through. It bends the picture behind only as it
    // exports, since the other looks draw none, and a model showing only its
    // shadows has no surface to bend it.
    let clear = mat.extra.x * (1.0 - clamp(mat.params.x * mr.b, 0.0, 1.0));
    let glass = select(0.0, clear, look == 0u && accepts != 2u);
    // Glass covers as much as what it shows does.
    let surface = mix(alpha, 1.0, glass);
    if (look == 0u && (mat.mode.z > 0.5 || (count == 0u && !surround))) {
        // Showing only its shadows, with none to show.
        if (accepts == 2u) {
            return vec4<f32>(0.0);
        }
        return untouched(in.at, eye, normalise(in.normal), base.rgb + emissive, base.rgb, alpha, glass);
    }

    // A mesh with no normals is shaded flat, facing the eye.
    var n = normalise(in.normal);
    if (all(n == vec3<f32>(0.0))) {
        n = normalise(cross(dx, dy));
        n = select(n, -n, dot(n, eye) < 0.0);
    } else if (front == (draw.normal0.w > 0.5)) {
        n = -n;
    }
    if (mat.slots[2].row1.w > 0.5) {
        // Its tangents, or with none the way its uvs run across the screen.
        var t = in.tangent.xyz;
        var w = in.tangent.w;
        if (w == 0.0) {
            let det = uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y;
            t = select(vec3<f32>(0.0), (uv_dy.y * dx - uv_dx.y * dy) / det, abs(det) > 1e-12);
            w = 1.0;
        }
        t = normalise(t - n * dot(n, t));
        if (any(t != vec3<f32>(0.0))) {
            // A mirrored layer reverses the cross product, so it's put back.
            let flip = select(1.0, -1.0, draw.normal0.w > 0.5);
            let b = cross(n, t) * select(1.0, -1.0, w < 0.0) * flip;
            let s = mat.params.z;
            n = normalise(t * mapped.x * s + b * mapped.y * s + n * mapped.z);
        }
    }
    if (look == 0u && (frame.flags.x > 0.5 || accepts == 2u)) {
        // Unlit, or showing only its shadows, so only the shadows touch it.
        var dim = vec3<f32>(1.0);
        for (var i: u32 = 0u; i < count; i = i + 1u) {
            // Faded in past the edge the light turns away at, so a normal
            // map crossing it doesn't stipple the shadow.
            let facing = smoothstep(0.0, 0.1, irradiance(in.at, n, frame.lights[i]));
            dim = dim * mix(vec3<f32>(1.0), left_of(in.at, n, frame.lights[i]), facing);
        }
        if (accepts == 2u) {
            // Just the shadow, in the colour a caster lets through.
            let lo = clamp(min(dim.x, min(dim.y, dim.z)), 0.0, 1.0);
            return vec4<f32>((dim - lo) * alpha, (1.0 - lo) * alpha);
        }
        return untouched(in.at, eye, n, (base.rgb + emissive) * dim, base.rgb, alpha, glass);
    }

    if (look == 2u) {
        // x right, y up and z towards the comp's own camera, as colour.
        let c = vec3<f32>(n.x, -n.y, -n.z) * 0.5 + 0.5;
        return vec4<f32>(pow(c, vec3<f32>(2.2)) * alpha, alpha);
    }

    let metallic = clamp(mat.params.x * mr.b, 0.0, 1.0);
    let roughness = clamp(mat.params.y * mr.g, 0.045, 1.0);
    let ao = 1.0 + mat.params.w * (occlusion - 1.0);
    let albedo = base.rgb;
    if (look == 1u) {
        // The studio light: a key from over the eye's shoulder and a sky
        // that's brighter overhead, whatever lights the comp has.
        let key = normalise(eye + vec3<f32>(-0.4, -0.7, 0.0));
        let sky = mix(0.15, 0.45, 0.5 - 0.5 * n.y);
        let h = normalise(key + eye);
        let shine = pow(max(dot(n, h), 0.0), mix(80.0, 6.0, roughness)) * mix(0.3, 0.03, roughness);
        // Glass keeps a film of its colour and its highlight, and lets the
        // rest through without bending it, as a view has no picture to bend.
        let film = alpha * (1.0 - 0.8 * clear);
        let matte = albedo * (sky + 0.55 * max(dot(n, key), 0.0)) * ao;
        return vec4<f32>(matte * film + (shine + emissive) * alpha, film);
    }
    let diffuse_colour = albedo * (1.0 - metallic);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let a2 = roughness * roughness * roughness * roughness;
    let nv = max(dot(n, eye), 1e-4);
    // A clear varnish over the rest: a second highlight, and what it
    // reflects is light the layer under it doesn't get.
    let coat = clamp(mat.extra.z, 0.0, 1.0);
    let coat_rough = clamp(mat.extra.w, 0.045, 1.0);
    let coat_a2 = coat_rough * coat_rough * coat_rough * coat_rough;
    var ambient = vec3<f32>(0.0);
    var diffuse = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);
    var coat_specular = vec3<f32>(0.0);
    for (var i: u32 = 0u; i < count; i = i + 1u) {
        let l = frame.lights[i];
        if (u32(l.flags.x + 0.5) == 3u) {
            ambient = ambient + l.colour.rgb;
            continue;
        }
        // The planes' own diffuse term: an area light's whole rectangle,
        // every other light from its middle.
        let e = irradiance(in.at, n, l);
        if (e <= 0.0) {
            continue;
        }
        // The highlight comes from the middle of an area light too.
        var to_light = vec3<f32>(0.0);
        let strength = arrival(in.at, l, &to_light);
        let ld = normalise(to_light);
        let h = normalise(ld + eye);
        let nl = max(dot(n, ld), 0.0);
        let nh = max(dot(n, h), 0.0);
        let schlick = pow(1.0 - max(dot(eye, h), 0.0), 5.0);
        let f = f0 + (1.0 - f0) * schlick;
        let left = left_of(in.at, n, l) * l.colour.rgb;
        specular = specular + ggx(a2, nh, nl, nv) * f * strength * left;
        diffuse = diffuse + (1.0 - f) * e * left;
        if (coat > 0.0) {
            let fc = 0.04 + 0.96 * schlick;
            coat_specular = coat_specular + ggx(coat_a2, nh, nl, nv) * fc * coat * strength * left;
        }
    }
    // A uniform ambient reflects too, or metal would stay black under it.
    let ab = env_fit(roughness, nv);
    let env = f0 * ab.x + ab.y;
    // Glass has no matte colour of its own, as that light goes through it.
    let lit = diffuse_colour * (1.0 - glass);
    var rgb = (lit + env) * ambient * ao + lit * diffuse + specular;
    let r = reflect(-eye, n);
    if (surround) {
        // The environment's split sum: what a matte surface takes from every
        // way, and the reflection at this roughness, both darkened by the
        // model's own occlusion.
        let irradiance = env_irradiance(env_irradiance_cube, env_sampler, environment, n);
        let radiance = env_radiance(env_specular_cube, env_sampler, environment, r, roughness);
        // Then the light many bounces between the microfacets keep
        // (Fdez-Aguera, as Bevy has it).
        let fr = max(vec3<f32>(1.0 - roughness), f0) - f0;
        let ks = f0 + fr * pow(1.0 - nv, 5.0);
        let fss_ess = ks * ab.x + ab.y;
        let ems = 1.0 - (ab.x + ab.y);
        let favg = f0 + (1.0 - f0) / 21.0;
        let fms_ems = fss_ess * favg / (1.0 - ems * favg) * ems;
        let kd = lit * (1.0 - (fss_ess + fms_ems));
        rgb = rgb + ((fms_ems + kd) * irradiance + fss_ess * radiance) * ao;
    }
    if (coat > 0.0) {
        let fit = env_fit(coat_rough, nv);
        let coat_env = 0.04 * fit.x + fit.y;
        var about = ambient;
        if (surround) {
            about = about + env_radiance(env_specular_cube, env_sampler, environment, r, coat_rough);
        }
        rgb = rgb * (1.0 - coat * coat_env) + about * coat_env * coat + coat_specular;
    }
    var out = vec4<f32>((rgb + emissive) * surface, alpha);
    // What glass lets through is tinted by its colour, less what it reflects,
    // and over nothing only its reflections cover.
    if (glass > 0.0) {
        let seen = behind(in.at, eye, n, mat.extra.y, roughness);
        let reflects = max(env.r, max(env.g, env.b));
        out = vec4<f32>(out.rgb + seen.rgb * albedo * glass * (1.0 - env), mix(alpha, max(seen.a, reflects), glass));
    }
    return out;
}

// A view's lines along the model's edges, a hair nearer than the faces so
// the faces under them never hide them.
@vertex
fn vs_wire(v: VertexIn, @builtin(instance_index) slot: u32) -> @builtin(position) vec4<f32> {
    var pos = frame.clip * instances[slot].place * draw.model * vec4<f32>(v.position, 1.0);
    pos.z = CUT_NEAR * 1.002;
    return pos;
}

@fragment
fn fs_wire() -> @location(0) vec4<f32> {
    return frame.wire;
}

// The model's depth, as one over clip w at each pixel, its nearest sample's
// where the samples differ, and nought where it has none.
@group(0) @binding(2) var depth_one: texture_depth_2d;
@group(0) @binding(3) var depth_many: texture_depth_multisampled_2d;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_depth_one(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureLoad(depth_one, vec2<i32>(pos.xy), 0) / CUT_NEAR);
}

@fragment
fn fs_depth_many(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    var d = 0.0;
    for (var s: u32 = 0u; s < textureNumSamples(depth_many); s = s + 1u) {
        d = max(d, textureLoad(depth_many, vec2<i32>(pos.xy), i32(s)));
    }
    return vec4<f32>(d / CUT_NEAR);
}

// A picture's next level down: each pixel sits where four of the level
// above meet, so one linear read is their average.
@group(0) @binding(4) var down_from: texture_2d<f32>;
@group(0) @binding(5) var down_samp: sampler;

@fragment
fn fs_down(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(down_from));
    return textureSampleLevel(down_from, down_samp, pos.xy * 2.0 / size, 0.0);
}

// A shutter sample's picture, added to the others at the pass's share.
@fragment
fn fs_add(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(down_from, vec2<i32>(pos.xy), 0);
}

// A shadow map's view (mesh/shadow.rs): frame.clip is comp pixels to the
// view, xy where on the map times w, z the depth in comp pixels, and
// frame.eye.xy the nearest depth kept and how far past it the farthest is.
// Kept in at.x on the way.
@vertex
fn vs_shadow(v: VertexIn) -> VsOut {
    let c = frame.clip * (draw.model * vec4<f32>(v.position, 1.0));
    var out: VsOut;
    out.pos = vec4<f32>(2.0 * c.x - c.w, c.w - 2.0 * c.y, 0.5 * c.w, c.w);
    out.at = vec3<f32>(c.z, 0.0, 0.0);
    out.uv0 = v.uv0;
    out.uv1 = v.uv1;
    out.colour = v.colour;
    return out;
}

// Where a part is cut out by its Mask, or mostly see-through when it blends,
// it casts nothing.
@fragment
fn fs_shadow(in: VsOut) -> @builtin(frag_depth) f32 {
    let mode = u32(mat.mode.x + 0.5);
    if (mode != 0u) {
        let a = (mat.base_colour * in.colour
            * textureSample(base_tex, base_samp, slot_uv(mat.slots[0], in))).a;
        if (a < select(0.5, mat.mode.y, mode == 1u)) {
            discard;
        }
    }
    return clamp((in.at.x - frame.eye.x) / frame.eye.y, 0.0, 1.0);
}
