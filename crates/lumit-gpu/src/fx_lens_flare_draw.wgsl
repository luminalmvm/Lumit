// Lens flare additive raster (docs/08 §3.27, K-261): vertex pulling from
// the built quad buffer, plain additive blend into the fp16 flare buffer.
// The colour is fully computed in build_verts (energy × Fresnel × mask ×
// wavelength × light), so the fragment is a passthrough — the ghost SHAPE
// is the warped grid itself, not a texture.

struct Vertex {
    ndc_x: f32,
    ndc_y: f32,
    r: f32,
    g: f32,
    b: f32,
};

@group(0) @binding(0) var<storage, read> verts: array<Vertex>;

struct DrawDims {
    raster: vec2<f32>,
    pad: vec2<f32>,
};
@group(0) @binding(1) var<uniform> dims: DrawDims;

// How far each vertex is pushed outward, in pixels (K-353).
//
// At one sample the rasteriser only makes a fragment where the pixel CENTRE
// falls inside the triangle, but a cell that covers no centre can still
// cover sample positions — and those are real light. Widening every triangle
// by more than the furthest sample offset from a centre (0.375 in each axis,
// so 0.53 diagonally) guarantees a fragment exists wherever coverage might
// be, and the fragment's own test then throws away the pixels the widening
// added. The CPU twin does the same thing by widening its bounding box.
const EXPAND_PX: f32 = 1.0;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) rgb: vec3<f32>,
    // Barycentric coordinates of this vertex within its own triangle — the
    // input to the analytic coverage test in the fragment (K-353).
    @location(1) bary: vec3<f32>,
};

// The 4x multisample positions inside a pixel, as offsets from the pixel
// CENTRE — the standard Vulkan/D3D locations at count 4, and the same four
// `lumit_core::fx::lens_flare::MSAA_SAMPLES` holds as offsets from the pixel
// origin. The two must stay in step: they are the CPU twin and the GPU of
// one antialiasing model.
const SAMPLES: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2<f32>(-0.125, -0.375),
    vec2<f32>(0.375, -0.125),
    vec2<f32>(-0.375, 0.125),
    vec2<f32>(0.125, 0.375),
);

// Which of the cell's four stored corners each of its six vertices is
// (K-263). build_verts stores the corners once, in order round the cell; the
// two triangles are (0,1,2) and (0,2,3), so this is the index buffer an
// indexed draw would hold, spelled in the shader instead of held in memory.
fn corner_of(k: u32) -> u32 {
    if (k == 1u) {
        return 1u;
    }
    if (k == 2u || k == 4u) {
        return 2u;
    }
    if (k == 5u) {
        return 3u;
    }
    return 0u;
}

fn cross2(u: vec2<f32>, v: vec2<f32>) -> f32 {
    return u.x * v.y - u.y * v.x;
}

// One corner of the triangle, pushed out so that BOTH edges meeting there
// have moved [`EXPAND_PX`] outward along their own normals.
//
// Pushing the corner away from the centroid instead — the obvious thing —
// is wrong for the shape that matters most here. A caustic-folded cell is a
// SLIVER: its three corners are nearly collinear, so the centroid sits on
// that line and "away from the centroid" points ALONG the sliver, leaving
// its thin axis exactly as thin as it was. Those are precisely the cells
// whose coverage the pixel centres miss, so the energy they carry went
// missing with them. Displacing the two edge lines and re-intersecting them
// widens the thin axis by construction, whatever the shape.
fn widen_corner(prev: vec2<f32>, here: vec2<f32>, next: vec2<f32>, s: f32) -> vec2<f32> {
    let d_in = here - prev;
    let d_out = next - here;
    let len_in = length(d_in);
    let len_out = length(d_out);
    // A collapsed edge has no normal to speak of; fall back to pushing the
    // corner away from the far one, which is the only direction there is.
    if (len_in < 1e-6 || len_out < 1e-6) {
        let away = here - (prev + next) * 0.5;
        let d = length(away);
        if (d < 1e-6) {
            return here;
        }
        return here + away / d * EXPAND_PX;
    }
    let n_in = vec2<f32>(d_in.y, -d_in.x) / len_in * s;
    let n_out = vec2<f32>(d_out.y, -d_out.x) / len_out * s;
    let a1 = prev + n_in * EXPAND_PX;
    let a2 = here + n_out * EXPAND_PX;
    let denom = cross2(d_in, d_out);
    // Near-parallel edges put the miter at infinity; offsetting along one
    // normal is the bounded answer and still covers what it must.
    if (abs(denom) < 1e-9) {
        return here + n_out * EXPAND_PX;
    }
    let t = cross2(a2 - a1, d_out) / denom;
    let widened = a1 + d_in * t;
    // A very sharp corner mitres a long way out. The coverage test would
    // still throw the extra away, but the fill it costs is unbounded, so cap
    // the corner's travel at a few pixels.
    let travel = widened - here;
    let dist = length(travel);
    let cap = EXPAND_PX * 4.0;
    if (dist > cap) {
        return here + travel / dist * cap;
    }
    return widened;
}

fn ndc_to_px(ndc: vec2<f32>) -> vec2<f32> {
    return (ndc * 0.5 + vec2<f32>(0.5, 0.5)) * dims.raster;
}

fn px_to_ndc(px: vec2<f32>) -> vec2<f32> {
    return (px / dims.raster - vec2<f32>(0.5, 0.5)) * 2.0;
}

@vertex
fn vs_flare(@builtin(vertex_index) vi: u32) -> VsOut {
    let k = vi % 6u;
    let cell = (vi / 6u) * 4u;
    let v = verts[cell + corner_of(k)];

    // The two triangles of a cell are vertices 0-2 and 3-5 of its six, so
    // this vertex's siblings are found by rounding down to its triangle's
    // first — the whole triangle is needed to widen any one of its corners.
    let tri0 = (k / 3u) * 3u;
    let a = verts[cell + corner_of(tri0)];
    let b = verts[cell + corner_of(tri0 + 1u)];
    let c = verts[cell + corner_of(tri0 + 2u)];
    let pa = ndc_to_px(vec2<f32>(a.ndc_x, a.ndc_y));
    let pb = ndc_to_px(vec2<f32>(b.ndc_x, b.ndc_y));
    let pc = ndc_to_px(vec2<f32>(c.ndc_x, c.ndc_y));
    let area = cross2(pb - pa, pc - pa);
    let s = select(-1.0, 1.0, area >= 0.0);
    let local = k % 3u;
    var widened = pa;
    if (local == 1u) {
        widened = widen_corner(pa, pb, pc, s);
    } else if (local == 2u) {
        widened = widen_corner(pb, pc, pa, s);
    } else {
        widened = widen_corner(pc, pa, pb, s);
    }

    // **Both varyings describe the ORIGINAL triangle, not the widened one.**
    // Barycentric coordinates and the interpolated colour are both affine in
    // screen position, and an affine function is reproduced exactly by
    // interpolating its values at any three points — so evaluating the
    // original cell's functions at the widened corner and letting the
    // rasteriser interpolate those gives every fragment the original cell's
    // barycentric and the original cell's colour. Emitting a unit barycentric
    // at the widened corner instead would describe the widened triangle, and
    // the coverage test would happily accept the padding it was supposed to
    // throw away.
    var bary = vec3<f32>(1.0, 0.0, 0.0);
    if (abs(area) > 1e-12) {
        let w0 = cross2(pb - widened, pc - widened) / area;
        let w1 = cross2(pc - widened, pa - widened) / area;
        bary = vec3<f32>(w0, w1, 1.0 - w0 - w1);
    }

    var out: VsOut;
    out.pos = vec4<f32>(px_to_ndc(widened), 0.0, 1.0);
    out.rgb = bary.x * vec3<f32>(a.r, a.g, a.b)
        + bary.y * vec3<f32>(b.r, b.g, b.b)
        + bary.z * vec3<f32>(c.r, c.g, c.b);
    out.bary = bary;
    return out;
}

// Coverage the analytic way (K-353), not the hardware's.
//
// **Why not 4x MSAA, which is what this used to be.** Additively blending
// fp16 into a multisample target is not reproducible run to run on this
// hardware: the same frame came back a few fp16 ULPs different each time,
// which is what made the flare fail its own bit-stability assertion
// (docs/14 determinism, impl/lens-flare.md §2.4). Measured, not guessed —
// the trace is bit-identical run to run, and swapping only the multisample
// raster for a single-sampled one makes the whole frame bit-identical too.
//
// The antialiasing is kept rather than lost. Barycentric coordinates are
// linear across a triangle, so `dpdx`/`dpdy` of them are exact and constant,
// and the barycentric at any sub-pixel offset follows without the rasteriser
// being asked: a fragment tests all four standard sample positions itself
// and takes `colour x covered/4`. That is precisely the model the CPU twin
// already spells out in `raster_triangle`, so the oracle now agrees with the
// GPU by construction instead of by resembling the hardware's resolve.
//
// It also gives back the largest allocation the effect made — the 4-sample
// target was ~66 MB at a 1080p flare buffer — and the resolve with it.
@fragment
fn fs_flare(in: VsOut) -> @location(0) vec4<f32> {
    let dx = dpdx(in.bary);
    let dy = dpdy(in.bary);
    var covered = 0u;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let o = SAMPLES[i];
        let b = in.bary + o.x * dx + o.y * dy;
        if (b.x >= 0.0 && b.y >= 0.0 && b.z >= 0.0) {
            covered = covered + 1u;
        }
    }
    if (covered == 0u) {
        discard;
    }
    // The colour stays the one interpolated at the pixel CENTRE, unclamped
    // and extrapolated where the centre falls outside the triangle — the
    // non-centroid interpolation the CPU twin models.
    let rgb = in.rgb * (f32(covered) * 0.25);
    let luma = 0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z;
    return vec4<f32>(rgb, luma);
}
