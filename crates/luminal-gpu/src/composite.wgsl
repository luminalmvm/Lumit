// Layer compositing (docs/06-RENDER-PIPELINE.md render order, evaluator v0).
//
// Each layer draws as a textured quad. The vertex transform is a full 4×4
// (decision K-023: 4×4 from day one, so 3D bolts on without a rewrite).
// Blending is premultiplied-over in LINEAR light — the whole reason the
// working format exists: light adds correctly here.

struct LayerUniform {
    // comp pixel space → NDC, including the layer's transform.
    matrix: mat4x4<f32>,
    // x: opacity 0..1 · y: use_matte · z: matte luma (else alpha) · w: invert
    params: vec4<f32>,
    // xy: comp target size in pixels (normalises frag position to matte uv)
    // z: snapshot-blend selector (0 screen · 1 overlay · 2 soft light ·
    //    3 hard light · 4 lighten · 5 darken)
    target_size: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> layer: LayerUniform;
// Comp-space matte (a rendered layer); 1×1 white when unused.
@group(0) @binding(3) var matte: texture_2d<f32>;
// Snapshot of the accumulated comp so far (shader-computed blends read the
// destination themselves and write with blending off); 1×1 black when unused.
@group(0) @binding(4) var dst_snapshot: texture_2d<f32>;
// Layer-space mask coverage in the alpha channel (GPU-sourced layers such as
// Precomps, whose pixels never exist CPU-side); 1×1 white when unused.
@group(0) @binding(5) var layer_mask: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_layer(@builtin(vertex_index) i: u32) -> VsOut {
    // Unit quad 0..1 (two triangles, 6 vertices).
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let c = corners[i];
    var out: VsOut;
    out.pos = layer.matrix * vec4<f32>(c, 0.0, 1.0);
    out.uv = c;
    return out;
}

fn srgb_encode_c(v: vec3<f32>) -> vec3<f32> {
    let lo = v * 12.92;
    let hi = 1.055 * pow(max(v, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, v <= vec3<f32>(0.0031308));
}

fn srgb_decode_c(v: vec3<f32>) -> vec3<f32> {
    let lo = v / 12.92;
    let hi = pow((v + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, v <= vec3<f32>(0.04045));
}

// W3C soft-light D(d) helper.
fn soft_light_d(d: vec3<f32>) -> vec3<f32> {
    let poly = ((16.0 * d - 12.0) * d + 4.0) * d;
    return select(sqrt(d), poly, d <= vec3<f32>(0.25));
}

// One encoded-domain formula per snapshot mode (docs/06 §blend domains;
// formulas are the W3C/PDF compositing set editors expect).
fn blend_encoded(mode: f32, s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let one = vec3<f32>(1.0);
    if (mode < 0.5) { // screen
        return one - (one - s) * (one - d);
    } else if (mode < 1.5) { // overlay: hard light with src/dst swapped
        return select(one - 2.0 * (one - s) * (one - d), 2.0 * s * d, d <= vec3<f32>(0.5));
    } else if (mode < 2.5) { // soft light
        let darkened = d - (one - 2.0 * s) * d * (one - d);
        let lightened = d + (2.0 * s - one) * (soft_light_d(d) - d);
        return select(lightened, darkened, s <= vec3<f32>(0.5));
    } else { // hard light
        return select(one - 2.0 * (one - s) * (one - d), 2.0 * s * d, s <= vec3<f32>(0.5));
    }
}

// Snapshot blends (docs/06-RENDER-PIPELINE.md §blend domains): the fragment
// reads the accumulated comp itself and writes the finished value (fixed-
// function blending off). Screen/Overlay/lights run perceptually — encode
// both sides, apply the formula, decode. Lighten/Darken are domain-invariant
// and run directly in linear.
@fragment
fn fs_layer_snapshot(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(src, samp, in.uv);
    var a = texel.a * layer.params.x * textureSample(layer_mask, samp, in.uv).a;
    let comp_uv = in.pos.xy / layer.target_size.xy;
    if (layer.params.y > 0.5) {
        let m = textureSample(matte, samp, comp_uv);
        var strength = m.a;
        if (layer.params.z > 0.5) {
            strength = dot(m.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        }
        if (layer.params.w > 0.5) {
            strength = 1.0 - strength;
        }
        a = a * clamp(strength, 0.0, 1.0);
    }
    let dst = textureSample(dst_snapshot, samp, comp_uv);
    let mode = layer.target_size.z;
    var blended: vec3<f32>;
    if (mode < 3.5) {
        let s_enc = srgb_encode_c(clamp(texel.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
        let d_enc = srgb_encode_c(clamp(dst.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
        blended = srgb_decode_c(blend_encoded(mode, s_enc, d_enc));
    } else if (mode < 4.5) { // lighten: per-channel max, linear
        blended = max(texel.rgb, dst.rgb);
    } else { // darken: per-channel min, linear
        blended = min(texel.rgb, dst.rgb);
    }
    let rgb = mix(dst.rgb, blended, a);
    let out_a = a + dst.a * (1.0 - a);
    return vec4<f32>(rgb, out_a);
}

@fragment
fn fs_layer(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(src, samp, in.uv);
    // Straight-alpha source → premultiplied output, opacity folded in.
    var a = texel.a * layer.params.x * textureSample(layer_mask, samp, in.uv).a;
    if (layer.params.y > 0.5) {
        // Matte lives in comp space: sample at this fragment's comp position.
        let comp_uv = in.pos.xy / layer.target_size.xy;
        let m = textureSample(matte, samp, comp_uv);
        var strength = m.a;
        if (layer.params.z > 0.5) {
            // Luma matte (v0: luma of the premultiplied composite).
            strength = dot(m.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        }
        if (layer.params.w > 0.5) {
            strength = 1.0 - strength;
        }
        a = a * clamp(strength, 0.0, 1.0);
    }
    return vec4<f32>(texel.rgb * a, a);
}
