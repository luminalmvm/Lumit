// Denoise's last pass (spatial::finish): the bands put back on the coarsest
// level, Grain, the view asked for, back to premultiplied linear and mixed
// with the input. Reads the input (in0), the current frame in Y'CbCr (in1),
// the shrunk bands (in2), the coarsest level (in3), the average (in4) and,
// when flag 4 says one is bound, the matte (in5). The common part is pasted
// ahead of this text.

@group(0) @binding(6) var dst: texture_storage_2d<rgba16float, write>;

fn result(x: i32, y: i32) -> vec3<f32> {
    return ld(in3, x, y).xyz + ld(in2, x, y).xyz;
}

fn removed(x: i32, y: i32) -> f32 {
    return ld(in1, x, y).x - result(x, y).x;
}

@compute @workgroup_size(8, 8)
fn finish(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.dims.x || g.y >= p.dims.y) {
        return;
    }
    let q = vec2<i32>(g.xy);
    let x = i32(g.x);
    let y = i32(g.y);
    let src = textureLoad(in0, q, 0);
    var r = result(x, y);
    let grain = p.fin2.x;
    if (grain > 0.0) {
        var box5 = 0.0;
        for (var dy = -2; dy <= 2; dy = dy + 1) {
            for (var dx = -2; dx <= 2; dx = dx + 1) {
                box5 = box5 + removed(x + dx, y + dy);
            }
        }
        r.x = r.x + grain * (removed(x, y) - box5 / 25.0);
    }
    // == spatial::matte_strength: the matte's share of the reduction.
    let cur = textureLoad(in1, q, 0).xyz;
    if ((p.pass_.w & 4u) != 0u) {
        let m = textureLoad(in5, q, 0);
        let k = clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
        r = cur + (r - cur) * k;
    }
    var made = src;
    let view = p.pass_.z;
    if (view == 1u) {
        let was = from_ycc(cur);
        let now = from_ycc(r);
        made = vec4<f32>(
            eotf1(0.5 + 4.0 * (was.x - now.x)),
            eotf1(0.5 + 4.0 * (was.y - now.y)),
            eotf1(0.5 + 4.0 * (was.z - now.z)),
            1.0,
        );
    } else if (view == 2u) {
        var grey = 0.0;
        if (p.fin2.z > 0.0) {
            grey = clamp((textureLoad(in4, q, 0).w - 1.0) / (2.0 * p.fin2.z), 0.0, 1.0);
        }
        grey = eotf1(grey);
        made = vec4<f32>(grey, grey, grey, 1.0);
    } else if (src.w > 1e-6) {
        let e = from_ycc(r);
        let lin = vec3<f32>(eotf1(e.x), eotf1(e.y), eotf1(e.z));
        made = vec4<f32>(lin * src.w, src.w);
    }
    textureStore(dst, q, src + (made - src) * p.fin2.y);
}
