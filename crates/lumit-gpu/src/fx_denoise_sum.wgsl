// Denoise's running sums (temporal::accumulate), kept in 32-bit float so a
// long average loses nothing: .xyz the weighted Y'CbCr, .w the weights, and
// the squared weights beside them. One pass a neighbour, so the memory is the
// same at any Frames. The common part is pasted ahead of this text.

@group(0) @binding(6) var out_sum: texture_storage_2d<rgba32float, write>;
@group(0) @binding(7) var out_sq: texture_storage_2d<r32float, write>;

// The current frame (in0) as the first term.
@compute @workgroup_size(8, 8)
fn init(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let q = vec2<i32>(g.xy);
    textureStore(out_sum, q, vec4<f32>(textureLoad(in0, q, 0).xyz, 1.0));
    textureStore(out_sq, q, vec4<f32>(1.0, 0.0, 0.0, 0.0));
}

// One aligned neighbour (in1) added by how well its 3x3 patch agrees with the
// current frame's (in0), onto the sums so far (in2, in3).
@compute @workgroup_size(8, 8)
fn accumulate(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let q = vec2<i32>(g.xy);
    let x = i32(g.x);
    let y = i32(g.y);
    let cur = textureLoad(in0, q, 0);
    // == temporal::inverse_noise.
    let sl = p.sig.x * tone_at(cur.x);
    let sc = p.sig.y;
    let il = 1.0 / (2.0 * max(sl * sl, 1e-12));
    let ic = 1.0 / (2.0 * max(sc * sc, 1e-12));
    // == temporal::patch_distance.
    var d = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = ld(in0, x + dx, y + dy);
            let o = ld(in1, x + dx, y + dy);
            let dl = c.x - o.x;
            let db = c.y - o.y;
            let dr = c.z - o.z;
            d = d + dl * dl * il + (db * db + dr * dr) * ic;
        }
    }
    d = d / 27.0;
    // == temporal::agreement.
    var wgt = 0.0;
    if (p.tone.w > 0.0 && p.sig.w > 0.0) {
        let k = clamp(1.0 - (d - p.sig.z) / p.sig.w, 0.0, 1.0);
        wgt = k * k;
    }
    let a = textureLoad(in1, q, 0);
    wgt = a.w * wgt;
    let s = textureLoad(in2, q, 0);
    let s2 = textureLoad(in3, q, 0).x;
    textureStore(out_sum, q, vec4<f32>(s.xyz + wgt * a.xyz, s.w + wgt));
    textureStore(out_sq, q, vec4<f32>(s2 + wgt * wgt, 0.0, 0.0, 0.0));
}
