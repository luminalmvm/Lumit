// Particles laid into a grid for smoke to emit from (fx/smoke.rs). Each adds a
// whole number to the eight cells round it, by how near it is. Adding whole
// numbers gives the same sum in any order, which is why nothing here is a float.

struct Splat {
    // A particle's pixels to cells, by rows.
    to_cells: array<vec4<f32>, 3>,
    // The grid's cells each way.
    cells: vec4<u32>,
};

// The state as the simulation left it: the count first, then 12 words a particle.
@group(0) @binding(0) var<storage, read> parts: array<u32>;
@group(0) @binding(1) var<storage, read_write> emitted: array<atomic<u32>>;
@group(0) @binding(2) var<uniform> u: Splat;

const HEAD_WORDS: u32 = 4u;
const PART_WORDS: u32 = 12u;
const DEAD: u32 = 2u;

@compute @workgroup_size(64)
fn smoke_splat(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= parts[0]) {
        return;
    }
    let at = HEAD_WORDS + gid.x * PART_WORDS;
    if ((parts[at + 10u] & DEAD) != 0u) {
        return;
    }
    let q = vec4<f32>(bitcast<f32>(parts[at]), bitcast<f32>(parts[at + 1u]),
        bitcast<f32>(parts[at + 2u]), 1.0);
    let m = vec3<f32>(dot(u.to_cells[0], q), dot(u.to_cells[1], q), dot(u.to_cells[2], q))
        - vec3<f32>(0.5);
    let low = floor(m);
    let f = m - low;
    let n = vec3<i32>(u.cells.xyz);
    let base = vec3<i32>(low);
    for (var k: i32 = 0; k < 8; k = k + 1) {
        let o = vec3<i32>(k & 1, (k >> 1u) & 1, (k >> 2u) & 1);
        let c = base + o;
        if (any(c < vec3<i32>(0)) || any(c >= n)) {
            continue;
        }
        let w = mix(vec3<f32>(1.0) - f, f, vec3<f32>(o));
        let i = u32(c.x + c.y * n.x + c.z * n.x * n.y);
        atomicAdd(&emitted[i], u32(w.x * w.y * w.z * 256.0));
    }
}
