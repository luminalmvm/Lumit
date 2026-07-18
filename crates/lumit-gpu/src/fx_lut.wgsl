// 3D-LUT lookup (docs/08-EFFECTS.md §3.11; docs/impl/lut.md). Mirrors
// lumit_core::lut::Lut3d::sample op-for-op (§1.6: the CPU is the oracle):
// trilinear interpolation of a colour cube, in linear light on
// **unpremultiplied** colour (§2.2 — a LUT is an arbitrary colour map, so it
// must not see premultiplied values; unpremult -> look up -> re-premult).
//
// The eight integer corners are read with textureLoad and the three lerps are
// done here in f32, NOT via a hardware linear sampler: fixed-function 3D
// filtering is not guaranteed bit-for-bit across GPUs, so it would not hold the
// fp16-ULP oracle against the CPU reference (docs/impl/lut.md §3). The corner
// index order (x=r, y=g, z=b) matches the red-fastest flat index the cube is
// uploaded with (r + g*N + b*N*N), so there is no transpose.
//
// Domain is assumed 0..1 (a domain remap is a documented follow-up,
// docs/impl/lut.md §2). Out-of-domain input clamps to the edge (it does not
// wrap). `mix == 0` reproduces the input bit-exactly.

struct Params {
    size: u32,     // LUT edge length N (the cube holds N³ samples)
    mix: f32,      // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var cube: texture_3d<f32>;

// The unpremultiplied colour of a premultiplied pixel (== the discipline the
// affine grades use; docs/impl/lut.md §3). Guarded at alpha 0.
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// A LUT sample at integer grid cell (r, g, b), red-fastest. Only the RGB is
// meaningful; the alpha channel is padding from the upload.
fn corner(x: i32, y: i32, z: i32) -> vec3<f32> {
    return textureLoad(cube, vec3<i32>(x, y, z), 0).rgb;
}

// Component-wise linear blend `a + (b - a) * t`, matching cpu::lut lerp3 form
// exactly (so CPU and GPU stay within the cheap-class fp16 ULP tolerance).
fn lerp3(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return a + (b - a) * t;
}

@compute @workgroup_size(8, 8)
fn lut_apply(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= dims.x || xy.y >= dims.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let u = unpremult(o);

    // Map each channel onto the grid and clamp (out-of-domain clamps to the
    // edge). Domain 0..1, so the coordinate is `c * (N - 1)` == the CPU
    // reference's `axis()` with lo=0, hi=1 (docs/impl/lut.md §2).
    let maxi = i32(p.size) - 1;
    let maxf = f32(maxi);
    let g = clamp(u * maxf, vec3<f32>(0.0), vec3<f32>(maxf));
    let base = floor(g);
    let f = g - base;
    let x0 = i32(base.x);
    let y0 = i32(base.y);
    let z0 = i32(base.z);
    let x1 = min(x0 + 1, maxi);
    let y1 = min(y0 + 1, maxi);
    let z1 = min(z0 + 1, maxi);

    // The eight surrounding samples, then lerp along r, then g, then b —
    // byte-for-byte the order of Lut3d::sample.
    let c000 = corner(x0, y0, z0);
    let c100 = corner(x1, y0, z0);
    let c010 = corner(x0, y1, z0);
    let c110 = corner(x1, y1, z0);
    let c001 = corner(x0, y0, z1);
    let c101 = corner(x1, y0, z1);
    let c011 = corner(x0, y1, z1);
    let c111 = corner(x1, y1, z1);

    let c00 = lerp3(c000, c100, f.x);
    let c10 = lerp3(c010, c110, f.x);
    let c01 = lerp3(c001, c101, f.x);
    let c11 = lerp3(c011, c111, f.x);
    let c0 = lerp3(c00, c10, f.y);
    let c1 = lerp3(c01, c11, f.y);
    let graded = lerp3(c0, c1, f.z);

    // Re-premultiply, then blend against the untouched original by Mix.
    let pm = graded * o.a;
    let outv = lerp3(o.rgb, pm, p.mix);
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}
