// Scene passes (mesh/passes.rs): what each pixel of a 3D scene is, rather than
// how it looks. Models and 3D layers meet in one depth here, the nearest
// surface at each sample winning, as they do in the picture.
//
// Depth is a constant clip z over clip w, as in mesh.wgsl.

const CUT_NEAR: f32 = 1e-4;

struct Frame {
    // Comp pixels to clip space, its z left for the vertex to set.
    clip: mat4x4<f32>,
    // Comp pixels to metres in the space the pass is read in.
    to_space: mat4x4<f32>,
    // xyz = the eye in comp pixels. w = what to write: 0 normal, 1 position,
    // 2 part id, 3 a matte with its coverage from its alpha, 4 a matte without.
    eye: vec4<f32>,
};

struct Draw {
    // Model space, or a 3D layer's unit square, to comp pixels.
    model: mat4x4<f32>,
    // The normals' matrix, its columns. normal0.w = 1 where the matrix
    // mirrors or, on a 3D layer, its opacity. normal1.w = the part's number.
    // normal2.w = 1 where a matte shows this part.
    normal0: vec4<f32>,
    normal1: vec4<f32>,
    normal2: vec4<f32>,
};

struct Slot {
    row0: vec4<f32>,
    row1: vec4<f32>,
};

struct Material {
    base_colour: vec4<f32>,
    emissive: vec4<f32>,
    // metallic, roughness, normal scale, occlusion strength.
    params: vec4<f32>,
    // x = alpha mode (0 opaque, 1 mask, 2 blend), y = cutoff, w = 1 with an
    // opacity picture.
    mode: vec4<f32>,
    // transmission, IOR, clearcoat, clearcoat roughness.
    extra: vec4<f32>,
    slots: array<Slot, 5>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<uniform> draw: Draw;
@group(1) @binding(0) var<uniform> mat: Material;
@group(1) @binding(1) var base_tex: texture_2d<f32>;
@group(1) @binding(3) var normal_tex: texture_2d<f32>;
// The samplers a material reads with, in a group of their own.
@group(2) @binding(0) var base_samp: sampler;
@group(2) @binding(2) var normal_samp: sampler;
@group(1) @binding(11) var opacity_tex: texture_2d<f32>;
// A 3D layer's picture, which only the layers' pipeline binds.
@group(1) @binding(20) var plane_tex: texture_2d<f32>;
@group(1) @binding(21) var plane_samp: sampler;

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

fn safe_unit(v: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    return select(vec3<f32>(0.0), v / l, l > 0.0);
}

// Linear light that shows as these bytes, so the picture reads back as the
// number it was made from.
fn from_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// A surface is there at this pixel, facing `n`, seen through `alpha`.
fn paint(n: vec3<f32>, at: vec3<f32>, alpha: f32) -> vec4<f32> {
    let what = u32(frame.eye.w + 0.5);
    switch (what) {
        case 0u: {
            let m = mat3x3<f32>(frame.to_space[0].xyz, frame.to_space[1].xyz, frame.to_space[2].xyz);
            return vec4<f32>(safe_unit(m * n) * 0.5 + 0.5, 1.0);
        }
        case 1u: {
            return vec4<f32>((frame.to_space * vec4<f32>(at, 1.0)).xyz, 1.0);
        }
        case 2u: {
            // Whole already, and a half added would round it away.
            let id = u32(draw.normal1.w);
            let bytes = vec3<u32>((id >> 16u) & 255u, (id >> 8u) & 255u, id & 255u);
            return vec4<f32>(from_srgb(vec3<f32>(bytes) / 255.0), 1.0);
        }
        default: {
            return vec4<f32>(vec3<f32>(draw.normal2.w), alpha);
        }
    }
}

// Whether a surface this see-through still counts. A matte with coverage takes
// it as coverage, and the rest keep what is at least half there, as the depth
// pass does.
fn counts(alpha: f32) -> bool {
    return select(alpha >= 0.5, alpha > 0.0, u32(frame.eye.w + 0.5) == 3u);
}

fn slot_uv(s: Slot, in: VsOut) -> vec2<f32> {
    let uv = vec3<f32>(select(in.uv0, in.uv1, s.row0.w > 0.5), 1.0);
    return vec2<f32>(dot(s.row0.xyz, uv), dot(s.row1.xyz, uv));
}

@vertex
fn vs_model(v: VertexIn) -> VsOut {
    let n = mat3x3<f32>(draw.normal0.xyz, draw.normal1.xyz, draw.normal2.xyz);
    let at = (draw.model * vec4<f32>(v.position, 1.0)).xyz;
    var out: VsOut;
    out.pos = frame.clip * vec4<f32>(at, 1.0);
    out.pos.z = CUT_NEAR;
    out.at = at;
    out.normal = n * v.normal;
    out.tangent = vec4<f32>((draw.model * vec4<f32>(v.tangent.xyz, 0.0)).xyz, v.tangent.w);
    out.uv0 = v.uv0;
    out.uv1 = v.uv1;
    out.colour = v.colour;
    return out;
}

@fragment
fn fs_model(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Every picture and slope read first, while every pixel of the quad is
    // still running.
    let base_uv = slot_uv(mat.slots[0], in);
    let normal_uv = slot_uv(mat.slots[2], in);
    let base = mat.base_colour.a * in.colour.a * textureSample(base_tex, base_samp, base_uv).a;
    let shown = textureSample(opacity_tex, base_samp, base_uv).rgb;
    var mapped = textureSample(normal_tex, normal_samp, normal_uv).xyz;
    // A layer's normals come as light, so back to how a normal map stores them.
    if (mat.slots[2].row1.w > 1.5) {
        mapped = select(1.055 * pow(mapped, vec3<f32>(1.0 / 2.4)) - 0.055, mapped * 12.92, mapped <= vec3<f32>(0.0031308));
    }
    mapped = mapped * 2.0 - 1.0;
    let dx = dpdx(in.at);
    let dy = dpdy(in.at);
    let uv_dx = dpdx(normal_uv);
    let uv_dy = dpdy(normal_uv);

    var alpha = base;
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
    } else if (!counts(alpha)) {
        discard;
    }

    let eye = safe_unit(frame.eye.xyz - in.at);
    // A mesh with no normals faces the eye, flat.
    var n = safe_unit(in.normal);
    if (all(n == vec3<f32>(0.0))) {
        n = safe_unit(cross(dx, dy));
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
        t = safe_unit(t - n * dot(n, t));
        if (any(t != vec3<f32>(0.0))) {
            let b = cross(n, t) * select(1.0, -1.0, w < 0.0);
            let s = mat.params.z;
            n = safe_unit(t * mapped.x * s + b * mapped.y * s + n * mapped.z);
        }
    }
    return paint(n, in.at, alpha);
}

@vertex
fn vs_plane(@builtin(vertex_index) i: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let uv = corners[i];
    let at = (draw.model * vec4<f32>(uv, 0.0, 1.0)).xyz;
    var out: VsOut;
    out.pos = frame.clip * vec4<f32>(at, 1.0);
    out.pos.z = CUT_NEAR;
    out.at = at;
    out.uv0 = uv;
    return out;
}

@fragment
fn fs_plane(in: VsOut) -> @location(0) vec4<f32> {
    let alpha = textureSample(plane_tex, plane_samp, in.uv0).a * draw.normal0.w;
    if (!counts(alpha)) {
        discard;
    }
    // A plane faces whoever is looking at it.
    let flat_n = safe_unit(cross(draw.model[1].xyz, draw.model[0].xyz));
    let eye = frame.eye.xyz - in.at;
    return paint(select(flat_n, -flat_n, dot(flat_n, eye) < 0.0), in.at, alpha);
}
