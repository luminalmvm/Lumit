// Skins and morph targets (mesh/deform.rs): each vertex of a part that moves,
// put where a pose has it and written to a buffer the mesh pass, the shadow
// maps and the lines all draw from, so they all see the same shape.

// One part's run of vertices.
struct Job {
    rest_first: u32,
    count: u32,
    out_first: u32,
    influence_first: u32,
    target_first: u32,
    target_count: u32,
    palette_first: u32,
    weight_first: u32,
    skinned: u32,
};

// The joints a vertex hangs from, and its share of each.
struct Influence {
    joints: vec4<u32>,
    weights: vec4<f32>,
};

@group(0) @binding(0) var<uniform> job: Job;
// mesh.rs's Vertex, as plain numbers: position, normal, tangent, two uvs, colour.
@group(0) @binding(1) var<storage, read> rest: array<f32>;
@group(0) @binding(2) var<storage, read> influences: array<Influence>;
// What each target adds to each vertex: its position, normal and tangent.
@group(0) @binding(3) var<storage, read> targets: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> palette: array<mat4x4<f32>>;
@group(0) @binding(5) var<storage, read> weights: array<f32>;
@group(0) @binding(6) var<storage, read_write> posed: array<f32>;

const STRIDE: u32 = 18u;

@compute @workgroup_size(64)
fn cs_deform(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= job.count) {
        return;
    }
    let src = (job.rest_first + i) * STRIDE;
    var p = vec3<f32>(rest[src], rest[src + 1u], rest[src + 2u]);
    var n = vec3<f32>(rest[src + 3u], rest[src + 4u], rest[src + 5u]);
    var t = vec3<f32>(rest[src + 6u], rest[src + 7u], rest[src + 8u]);
    for (var k: u32 = 0u; k < job.target_count; k = k + 1u) {
        let w = weights[job.weight_first + k];
        if (w == 0.0) {
            continue;
        }
        let at = job.target_first + (k * job.count + i) * 3u;
        p = p + targets[at].xyz * w;
        n = n + targets[at + 1u].xyz * w;
        t = t + targets[at + 2u].xyz * w;
    }
    if (job.skinned != 0u) {
        let v = influences[job.influence_first + i];
        let m = palette[job.palette_first + v.joints.x] * v.weights.x
            + palette[job.palette_first + v.joints.y] * v.weights.y
            + palette[job.palette_first + v.joints.z] * v.weights.z
            + palette[job.palette_first + v.joints.w] * v.weights.w;
        p = (m * vec4<f32>(p, 1.0)).xyz;
        // Normals by the cofactors, which hold up where a bone stretches, with
        // the sign kept the way the bone's own hand has it.
        let a = m[0].xyz;
        let b = m[1].xyz;
        let c = m[2].xyz;
        let face = select(1.0, -1.0, dot(a, cross(b, c)) < 0.0);
        n = mat3x3<f32>(cross(b, c), cross(c, a), cross(a, b)) * n * face;
        t = mat3x3<f32>(a, b, c) * t;
    }
    let to = (job.out_first + i) * STRIDE;
    posed[to] = p.x;
    posed[to + 1u] = p.y;
    posed[to + 2u] = p.z;
    posed[to + 3u] = n.x;
    posed[to + 4u] = n.y;
    posed[to + 5u] = n.z;
    posed[to + 6u] = t.x;
    posed[to + 7u] = t.y;
    posed[to + 8u] = t.z;
    for (var k: u32 = 9u; k < STRIDE; k = k + 1u) {
        posed[to + k] = rest[src + k];
    }
}
