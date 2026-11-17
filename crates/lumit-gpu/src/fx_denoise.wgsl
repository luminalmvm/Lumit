// Denoise's passes that write a working picture (lumit_core::fx::denoise):
// into Y'CbCr, a neighbour lined up, the average resolved, the a-trous
// levels and the band shrink. Each mirrors the CPU function named beside it.
// The common part is pasted ahead of this text.

@group(0) @binding(6) var out_mid: texture_storage_2d<rgba16float, write>;

// The current frame in Y'CbCr, alpha in .w (space::picture_to_ycc).
@compute @workgroup_size(8, 8)
fn prep(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let q = vec2<i32>(g.xy);
    textureStore(out_mid, q, pixel_to_ycc(textureLoad(in0, q, 0)));
}

// == temporal::Field::sample: the field (in1, its own size) at field pixel
// (fx, fy), bilinear, edges held.
fn field_at(fx: f32, fy: f32) -> vec3<f32> {
    let fw = i32(p.dims.z);
    let fh = i32(p.dims.w);
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let xi = i32(x0);
    let yi = i32(y0);
    let a = textureLoad(in1, vec2<i32>(clamp(xi, 0, fw - 1), clamp(yi, 0, fh - 1)), 0).xyz;
    let b = textureLoad(in1, vec2<i32>(clamp(xi + 1, 0, fw - 1), clamp(yi, 0, fh - 1)), 0).xyz;
    let c = textureLoad(in1, vec2<i32>(clamp(xi, 0, fw - 1), clamp(yi + 1, 0, fh - 1)), 0).xyz;
    let d = textureLoad(in1, vec2<i32>(clamp(xi + 1, 0, fw - 1), clamp(yi + 1, 0, fh - 1)), 0).xyz;
    let top = a + (b - a) * tx;
    let bottom = c + (d - c) * tx;
    return top + (bottom - top) * ty;
}

// == temporal::catmull_rom.
fn cr(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t * t * t;
    return vec4<f32>(
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    );
}

// == temporal::bicubic over in0.
fn bicubic(px: f32, py: f32) -> vec4<f32> {
    let fx = floor(px);
    let fy = floor(py);
    let wx = cr(px - fx);
    let wy = cr(py - fy);
    let ix = i32(fx);
    let iy = i32(fy);
    var out = vec4<f32>(0.0);
    for (var j = 0; j < 4; j = j + 1) {
        var row = vec4<f32>(0.0);
        for (var i = 0; i < 4; i = i + 1) {
            row = row + wx[i] * ld(in0, ix - 1 + i, iy - 1 + j);
        }
        out = out + wy[j] * row;
    }
    let a = ld(in0, ix, iy);
    let b = ld(in0, ix + 1, iy);
    let c = ld(in0, ix, iy + 1);
    let d = ld(in0, ix + 1, iy + 1);
    return clamp(out, min(min(a, b), min(c, d)), max(max(a, b), max(c, d)));
}

// A neighbour (in0) moved onto this frame along the field (in1), in Y'CbCr
// with the field's confidence in .w (temporal::warp).
@compute @workgroup_size(8, 8)
fn warp(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let x = f32(g.x);
    let y = f32(g.y);
    var px = x;
    var py = y;
    var conf = 1.0;
    if ((p.pass_.w & 1u) != 0u && p.dims.z > 0u && p.dims.w > 0u) {
        let sx = f32(p.dims.z) / f32(p.dims.x);
        let sy = f32(p.dims.w) / f32(p.dims.y);
        let f = field_at((x + 0.5) * sx - 0.5, (y + 0.5) * sy - 0.5);
        px = x + f.x / sx;
        py = y + f.y / sy;
        conf = f.z;
    }
    let c = pixel_to_ycc(bicubic(px, py));
    textureStore(out_mid, vec2<i32>(g.xy), vec4<f32>(c.xyz, conf));
}

// The running sums (in0) and squared weights (in1) as an average and the
// frames it stands for.
@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let q = vec2<i32>(g.xy);
    let s = textureLoad(in0, q, 0);
    let w2 = textureLoad(in1, q, 0).x;
    textureStore(out_mid, q, vec4<f32>(s.xyz / s.w, s.w * s.w / w2));
}

// The current frame (in0) alone (temporal::plain).
@compute @workgroup_size(8, 8)
fn plain(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let q = vec2<i32>(g.xy);
    textureStore(out_mid, q, vec4<f32>(textureLoad(in0, q, 0).xyz, 1.0));
}

// == atrous::TAPS, by tap number. Spelled out rather than as an array, which
// one of the shader compilers cannot hand back from a function.
fn tap(k: i32) -> f32 {
    let d = abs(k - 2);
    if (d == 0) {
        return 6.0 / 16.0;
    }
    if (d == 1) {
        return 4.0 / 16.0;
    }
    return 1.0 / 16.0;
}

// One a-trous pass across (atrous::smooth, across).
@compute @workgroup_size(8, 8)
fn atrous_h(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let dil = i32(p.pass_.x);
    let s = pic();
    let x = i32(g.x);
    let y = i32(g.y);
    var acc = vec4<f32>(0.0);
    for (var k = 0; k < 5; k = k + 1) {
        acc = acc + tap(k) * textureLoad(in0, vec2<i32>(mirror(x + (k - 2) * dil, s.x), y), 0);
    }
    textureStore(out_mid, vec2<i32>(g.xy), acc);
}

// And down (atrous::smooth, down).
@compute @workgroup_size(8, 8)
fn atrous_v(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let dil = i32(p.pass_.x);
    let s = pic();
    let x = i32(g.x);
    let y = i32(g.y);
    var acc = vec4<f32>(0.0);
    for (var k = 0; k < 5; k = k + 1) {
        acc = acc + tap(k) * textureLoad(in0, vec2<i32>(x, mirror(y + (k - 2) * dil, s.y)), 0);
    }
    textureStore(out_mid, vec2<i32>(g.xy), acc);
}

// One band shrunk and added to the sum so far (spatial::shrink): the level
// above (in0), the level below (in1), the average for the frames blended
// (in2), the current frame for its luma (in3), the sum so far (in4).
@compute @workgroup_size(8, 8)
fn shrink(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let q = vec2<i32>(g.xy);
    let x = i32(g.x);
    let y = i32(g.y);
    let band = p.pass_.y;
    let blended = max(textureLoad(in2, q, 0).w, 1.0);
    let luma = textureLoad(in3, q, 0).x;
    // == spatial::local_energy: the 5x5 window less the pixel itself.
    var e = vec3<f32>(0.0);
    for (var dy = -2; dy <= 2; dy = dy + 1) {
        for (var dx = -2; dx <= 2; dx = dx + 1) {
            if (dx == 0 && dy == 0) {
                continue;
            }
            let d = ld(in0, x + dx, y + dy).xyz - ld(in1, x + dx, y + dy).xyz;
            e = e + d * d;
        }
    }
    e = e / 24.0;
    let d = textureLoad(in0, q, 0).xyz - textureLoad(in1, q, 0).xyz;
    var r = vec3<f32>(0.0);
    if ((p.pass_.w & 2u) == 0u) {
        r = textureLoad(in4, q, 0).xyz;
    }
    for (var c = 0; c < 3; c = c + 1) {
        var s = p.sig.y * p.cband[band];
        var amount = p.fin.y * p.bamt[band];
        if (c == 0) {
            s = p.sig.x * tone_at(luma) * p.lband[band];
            amount = p.fin.x * p.bamt[band];
        }
        let v = s * s / blended;
        var gain = max(1.0 - v / (e[c] + 1e-12), 0.0);
        if (band < 2u && v > 0.0) {
            gain = gain + p.fin.z * (1.0 - gain) * clamp(e[c] / v - 1.0, 0.0, 1.0);
        }
        var dv = d[c] * (1.0 - amount * (1.0 - gain));
        if (band < 2u && c == 0) {
            dv = dv * (1.0 + p.fin.w * 0.5 * gain);
        }
        r[c] = r[c] + dv;
    }
    textureStore(out_mid, q, vec4<f32>(r, 0.0));
}
