// Lens flare (docs/08-EFFECTS.md §3.27, docs/impl/lens-flare.md, K-261,
// K-366). Two compute entry points of the per-frame ghost pipeline:
//   trace        — one thread per pupil-grid ray, each taking its light from
//                  its own point of the source's emitting rectangle (K-367,
//                  `source_jitter`): refract the ray through
//                  the prescription with the FlareSim three-phase walk
//                  (reflecting at the pair's two surfaces), carry eight
//                  spectral throughputs through the per-surface reflectance
//                  (K-364), land on the sensor and band-integrate them into
//                  the ray's rgb; the weight the ray keeps is geometry
//                  alone — housing feather × iris mask. Mirrors lumit_core's
//                  `trace_splat_spectral` op-for-op; the dead sentinel is
//                  weight −1 (the CPU returns None).
//   build_splats — one thread per RAY (K-366): the ray's own footprint, by
//                  central differences over its neighbours' landings
//                  (lumit_core `ray_axes`), and the peak of the tent it
//                  deposits its pupil cell's flux over (lumit_core
//                  `splat_ray`'s arithmetic up to the divide). Rays are
//                  never connected to one another, so a caustic fold is
//                  just splats piling up — the correct integral — and the
//                  sliver/inflate/pull-in machinery of K-261..K-264 has
//                  nothing left to rescue.
// The additive raster lives in fx_lens_flare_draw.wgsl (one instanced quad
// per splat), the box blur in fx_lens_flare_blur.wgsl, detection in
// fx_lens_flare_detect.wgsl and the combine in fx_lens_flare_combine.wgsl.

struct Surface {
    radius_mm: f32,     // 0 = flat
    z_mm: f32,          // vertex z (front vertex at 0, +z toward sensor)
    semi_ap_mm: f32,    // clear semi-aperture (the stop's scales by fstop)
    cauchy_a: f32,      // medium AFTER this surface (1.0 = air)
    cauchy_b: f32,
    coating_layers: f32,
    is_stop: f32,
    _pad: f32,
};

// One (path × wavelength) combo: the bounce surfaces, the wavelength the
// GEOMETRY is traced at, and the band's index into `band_subs` — the
// radiometric sub-samples the energy is carried at (K-364).
//
// `bounce_c`/`bounce_d` are the third and fourth bounces of a four-bounce
// path (K-368); `NO_BOUNCE` in `bounce_c` marks the two-bounce ghosts that
// were all this struct carried before, and they walk exactly the phases they
// always did. The two fields took two of the padding slots, so the layout
// mirrors lumit-gpu's `GpuCombo` at the size it always had.
//
// `ring_fresnel` is this ghost's own Fresnel number (K-369, re-derived
// K-370), which sets how fine the diffraction fringes on its rim are; 0
// leaves the plain analytic polygon. It took the struct's last padding slot,
// so the layout is again unchanged.
struct Combo {
    bounce_a: u32,
    bounce_b: u32,
    lambda_nm: f32,
    _pad: f32,
    band: u32,
    bounce_c: u32,
    bounce_d: u32,
    ring_fresnel: f32,
};

// The empty bounce slot — lumit_core's `NO_BOUNCE`.
const NO_BOUNCE: u32 = 0xffffffffu;

// One radiometric sub-sample of a band (K-364): where in the baked
// reflectance table its wavelength sits, and its RGB weight, already
// multiplied by the exposure gain and Ghost intensity. Eight per band,
// at `band * 8 + k`.
struct BandSub {
    lambda_idx: u32,
    r: f32,
    g: f32,
    b: f32,
};

// One traced corner: raster position (flare-buffer px), the GEOMETRIC weight
// (housing feather × iris mask; weight < 0 = dead) and the band-integrated
// energy the ray carries. Since K-364 the two are separate — the throughput
// is spectral and the weight is not — where one scalar carried both.
struct Ray {
    pos_x: f32,
    pos_y: f32,
    weight: f32,
    _pad: f32,
    r: f32,
    g: f32,
    b: f32,
    _pad2: f32,
};

// One ray's deposit (K-366): where it landed, the two half-axes of the
// parallelogram footprint it spreads over, and the PEAK of the separable
// tent — its flux already divided by the density-capped footprint area, so
// the raster's fragment only has to evaluate (1−|u|)(1−|v|). `live` is 1.0
// for a ray with flux and 0.0 for a dead or unlit one, which the raster
// draws as a degenerate off-screen quad (the slot must still be written:
// the batch's splats are one contiguous instance range).
struct Splat {
    cx: f32,
    cy: f32,
    a1x: f32,
    a1y: f32,
    a2x: f32,
    a2y: f32,
    r: f32,
    g: f32,
    b: f32,
    live: f32,
    _pad0: f32,
    _pad1: f32,
};

// One flare source (see fx_lens_flare_detect.wgsl): position as a raster
// fraction, colour already multiplied by its gate weight, and the half-extent
// of its emitting area as a raster fraction (K-367 — zero is a point source).
// All-zero rgb is a dead slot the passes skip.
struct Light {
    pos_x: f32,
    pos_y: f32,
    r: f32,
    g: f32,
    b: f32,
    ext_x: f32,
    ext_y: f32,
    _pad2: f32,
};

struct TraceParams {
    surface_count: u32,
    combo_count: u32,
    grid: u32,          // pupil corners per axis
    combo_offset: u32,  // first combo of this batch
    coating: f32,       // 0..1 Coating dial
    aspect: f32,        // frame h/w for the light direction
    focal_mm: f32,
    screen_transform: f32, // flare-buffer px per sensor mm
    raster_w: f32,
    raster_h: f32,
    light_count: u32,
    sensor_shift_mm: f32,  // focus shift (K-260)
    pupil_mm: f32,         // spray radius, already × the f-stop scale
    start_z_mm: f32,
    sensor_z_mm: f32,
    stop_scale: f32,       // scales the stop surface's semi-aperture
    cell_area_px: f32,     // launch cell area in flare-buffer px²
    ray_stride: u32,       // rays per slot — THIS batch's grid² (K-263)
    // Keeps the uniform struct a multiple of 16 bytes; held the per-slot
    // quad count until K-366 dropped quads for per-ray splats.
    _pad_stride: u32,
    blades: u32,
    rot_rad: f32,
    roundness: f32,        // effective (wide-open blended)
    softness: f32,
    light_offset: u32,     // first light of this chunk (K-263)
};

@group(0) @binding(0) var<storage, read> surfaces: array<Surface>;
@group(0) @binding(1) var<storage, read> combos: array<Combo>;
@group(0) @binding(2) var<storage, read_write> rays: array<Ray>;
// Binding 3 held the per-cell landed areas until K-366; the numbering is
// left alone so the rest of the bindings keep their places.
@group(0) @binding(4) var<storage, read_write> splats: array<Splat>;
@group(0) @binding(5) var<uniform> tp: TraceParams;
@group(0) @binding(6) var<storage, read> lights: array<Light>;
// The baked per-surface reflectance table (K-364), laid out
// [surface][direction 0=fwd,1=rev][lambda][cos] — see lumit_core's
// `FlareBaked::reflectance`.
@group(0) @binding(7) var<storage, read> reflectance: array<f32>;
@group(0) @binding(8) var<storage, read> band_subs: array<BandSub>;

// The light direction for a source at raster fraction (px, py) — the exact
// WGSL twin of lumit_core's `light_direction` (sensor y up, so the y
// fraction flips sign; half the 36 mm sensor width is 18; z toward the
// sensor is positive).
fn dir_of(px: f32, py: f32) -> vec3<f32> {
    let half_w = 18.0;
    let x = (px * 2.0 - 1.0) * half_w;
    let y = -(py * 2.0 - 1.0) * tp.aspect * half_w;
    return normalize(vec3<f32>(-x, -y, tp.focal_mm));
}

fn light_dead(l: Light) -> bool {
    return l.r <= 0.0 && l.g <= 0.0 && l.b <= 0.0;
}

// Per-ray source integration (K-367) — the WGSL twin of lumit_core's
// `source_jitter`, op for op, with the two constants pinned against it by
// test. Each ray takes its light from its OWN point of the source's ±extent
// rectangle, so a source of any size costs exactly the rays a point does and
// the per-ray splat footprints (K-366) inflate to cover the spacing between
// neighbours — which is what makes the replicated ghost copies of K-355
// impossible rather than merely rare.
// 1/rho and 1/rho^2 of the plastic constant, at the digits an f32 holds.
const PHI_U: f32 = 0.7548777;
const PHI_V: f32 = 0.5698403;

// A triangle wave, uniform on [-1, 1]. Not `fract`: `fract` jumps the whole
// range at each wrap, and the splat footprints are central differences over
// the very neighbours a jump would separate by the width of the source.
fn tri(x: f32) -> f32 {
    return 2.0 * abs(2.0 * (fract(x) - 0.5)) - 1.0;
}

fn source_jitter(i: u32, j: u32, ext: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        tri((f32(i) + 0.5) * PHI_U) * ext.x,
        tri((f32(j) + 0.5) * PHI_V) * ext.y,
    );
}

fn cauchy_ior(a: f32, b: f32, lambda_nm: f32) -> f32 {
    let um = lambda_nm * 1e-3;
    return a + b / (um * um);
}

// The iris mask (lumit_core `pupil_mask`): polygon bound blended toward the
// circle by roundness, feathered by softness.
fn pupil_mask(u: f32, v: f32) -> f32 {
    let r = sqrt(u * u + v * v);
    let blades = f32(clamp(tp.blades, 3u, 16u));
    let tau = 6.283185307179586;
    let sector = tau / blades;
    let apothem = cos(3.141592653589793 / blades);
    let angle = atan2(v, u) - tp.rot_rad;
    var a = angle % sector;
    if (a < 0.0) {
        a = a + sector;
    }
    let poly_bound = apothem / cos(a - sector * 0.5);
    let bound = poly_bound + (1.0 - poly_bound) * clamp(tp.roundness, 0.0, 1.0);
    let soft = max(clamp(tp.softness, 0.0, 1.0) * bound, 1e-4);
    let t = clamp((r - (bound - soft)) / soft, 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

// Ghost-edge diffraction (K-369, re-derived K-370) — the op-for-op twins of
// lumit_core's `fresnel_cs`, `knife_edge_intensity` and `ghost_mask`.
//
// A ghost's rim is not a clean cut: light bends round the blade and lays down
// fine fringes just inside it. At the Fresnel numbers real ghosts have — the
// hundreds to tens of thousands, see `ghost_fresnel_number` — that is a
// straight-edge problem, so it is a closed form of ONE number (the
// perpendicular distance to the blade) rather than a propagated aperture
// image. The interior of a ghost is therefore flat by construction.

// Fresnel integrals C(v), S(v) in the pi/2 convention, by the standard
// auxiliary-function rational approximation. Odd in v, so one evaluation
// serves both sides of the edge.
fn fresnel_cs(v: f32) -> vec2<f32> {
    let x = abs(v);
    let f = (1.0 + 0.926 * x) / (2.0 + 1.792 * x + 3.104 * x * x);
    let g = 1.0 / (2.0 + 4.142 * x + 3.492 * x * x + 6.670 * x * x * x);
    let arg = 1.5707963267948966 * x * x;
    let sc = vec2<f32>(cos(arg), sin(arg));
    let cc = 0.5 + f * sc.y - g * sc.x;
    let ss = 0.5 - f * sc.x - g * sc.y;
    if (v < 0.0) {
        return vec2<f32>(-cc, -ss);
    }
    return vec2<f32>(cc, ss);
}

// The knife-edge intensity: 1 deep inside, 1/4 on the geometric edge, a first
// fringe of about 1.37 just inside it, decaying to 0 outside.
fn knife_edge_intensity(v: f32) -> f32 {
    let cs = fresnel_cs(v) + vec2<f32>(0.5, 0.5);
    return 0.5 * dot(cs, cs);
}

// Where the fringes fade into their own average, in v units — pinned against
// lumit_core's RING_WASH by test.
const RING_WASH_LO: f32 = 0.5;
const RING_WASH_HI: f32 = 2.0;

// The iris mask one ray sees, with this ghost's edge diffraction on it.
// `blur` is the ray grid's step in pupil units: fringes finer than that
// cannot be drawn, only aliased, so they cross over to their average — which
// is the plain iris edge.
fn ghost_mask(u: f32, v: f32, fresnel: f32, blur: f32) -> f32 {
    let analytic = pupil_mask(u, v);
    if (fresnel <= 0.0) {
        return analytic;
    }
    let r = sqrt(u * u + v * v);
    let blades = f32(clamp(tp.blades, 3u, 16u));
    let tau = 6.283185307179586;
    let sector = tau / blades;
    let apothem = cos(3.141592653589793 / blades);
    let angle = atan2(v, u) - tp.rot_rad;
    var a = angle % sector;
    if (a < 0.0) {
        a = a + sector;
    }
    // The radial bound `pupil_mask` uses, and the cosine that turns a radial
    // gap into the perpendicular distance to the blade.
    let cos_a = cos(a - sector * 0.5);
    let poly_bound = apothem / cos_a;
    let roundness = clamp(tp.roundness, 0.0, 1.0);
    let bound = poly_bound + (1.0 - poly_bound) * roundness;
    let cos_fac = cos_a + (1.0 - cos_a) * roundness;
    let scale = sqrt(2.0 * fresnel);
    let ringed = knife_edge_intensity((bound - r) * cos_fac * scale);
    let soft = max(clamp(tp.softness, 0.0, 1.0) * bound, 1e-4);
    let blur_v = max(max(blur, 0.0), soft) * scale;
    let t = clamp((blur_v - RING_WASH_LO) / (RING_WASH_HI - RING_WASH_LO), 0.0, 1.0);
    let wash = t * t * (3.0 - 2.0 * t);
    return ringed + (analytic - ringed) * wash;
}

// Unpolarised Fresnel by incidence cosine (lumit_core `fresnel_cos`).
fn fresnel_cos(cos_i_in: f32, n1: f32, n2: f32) -> f32 {
    let cos_i = abs(cos_i_in);
    let eta = n1 / n2;
    let sin2_t = eta * eta * (1.0 - cos_i * cos_i);
    if (sin2_t >= 1.0) {
        return 1.0;
    }
    let cos_t = sqrt(1.0 - sin2_t);
    let rs = (n1 * cos_i - n2 * cos_t) / (n1 * cos_i + n2 * cos_t);
    let rp = (n2 * cos_i - n1 * cos_t) / (n2 * cos_i + n1 * cos_t);
    return 0.5 * (rs * rs + rp * rp);
}

// Grid of the baked reflectance table (K-364) — must equal lumit_core's
// `REFL_LAMBDA_BINS` / `REFL_COS_BINS`, which a test pins.
const REFL_LAMBDA_BINS: u32 = 69u;
const REFL_COS_BINS: u32 = 16u;

// Radiometric sub-samples per traced band (lumit_core `SPECTRAL_SUB`).
const SPECTRAL_SUB: u32 = 8u;

// Read the baked table at (surface, direction, lambda index, cos theta):
// linear interpolation across the cos bins, exact in lambda (the index is
// already on the grid). lumit_core `refl_lookup`, op for op.
//
// This replaced an inline thin-film stack solved per ray per surface at the
// band's single wavelength (K-356). A stack's reflectance oscillates several
// times across the visible, so one wavelength a band could not see the shape
// it was sampling; baked at 5 nm and read eight times a band, it can — and
// the lookup is cheaper than the transfer matrix it replaced.
fn refl_lookup(surf: u32, reverse: u32, lambda_idx: u32, cos_i: f32) -> f32 {
    let base = ((surf * 2u + reverse) * REFL_LAMBDA_BINS + lambda_idx) * REFL_COS_BINS;
    let c = clamp(cos_i, 0.0, 1.0) * f32(REFL_COS_BINS) - 0.5;
    let j0 = min(u32(max(floor(c), 0.0)), REFL_COS_BINS - 1u);
    let j1 = min(j0 + 1u, REFL_COS_BINS - 1u);
    let f = clamp(c - f32(j0), 0.0, 1.0);
    let n = arrayLength(&reflectance);
    if (base + j1 >= n) {
        return 0.0;
    }
    let a = reflectance[base + j0];
    let b = reflectance[base + j1];
    return a + (b - a) * f;
}

// Ray–surface intersection (lumit_core `intersect`, K-264): flat plane at
// the vertex z, else ray–sphere picking the intersection closest to the
// vertex. A ray that MISSES the sphere (or finds it behind) continues
// VIRTUALLY through the vertex plane instead of dying — physically the
// mount absorbs it, so the caller forces its weight to zero, but killing it
// also killed every grid cell touching it and a ghost bounded by misses
// wore its pupil grid as a staircase. ok=false only for degenerate rays.
struct Isect {
    pos: vec3<f32>,
    normal: vec3<f32>,
    ok: bool,
    missed: bool,
};

fn plane_hit(pos: vec3<f32>, dir: vec3<f32>, z_mm: f32, missed: bool) -> Isect {
    var out: Isect;
    out.ok = false;
    out.missed = missed;
    let t = (z_mm - pos.z) / dir.z;
    if (!(t > 1e-6)) {
        return out;
    }
    out.pos = pos + dir * t;
    out.normal = vec3<f32>(0.0, 0.0, select(1.0, -1.0, dir.z > 0.0));
    out.ok = true;
    return out;
}

fn intersect(pos: vec3<f32>, dir: vec3<f32>, radius: f32, z_mm: f32) -> Isect {
    var out: Isect;
    out.ok = false;
    out.missed = false;
    if (abs(dir.z) < 1e-12) {
        return out;
    }
    if (abs(radius) < 1e-6) {
        return plane_hit(pos, dir, z_mm, false);
    }
    let centre = vec3<f32>(0.0, 0.0, z_mm + radius);
    let oc = pos - centre;
    let a = dot(dir, dir);
    let b = 2.0 * dot(oc, dir);
    let c = dot(oc, oc) - radius * radius;
    let disc = b * b - 4.0 * a * c;
    var t = -1.0;
    if (disc >= 0.0) {
        let sd = sqrt(disc);
        let inv2a = 0.5 / a;
        let t1 = (-b - sd) * inv2a;
        let t2 = (-b + sd) * inv2a;
        if (t1 > 1e-6 && t2 > 1e-6) {
            let z1 = pos.z + t1 * dir.z;
            let z2 = pos.z + t2 * dir.z;
            t = select(t2, t1, abs(z1 - z_mm) < abs(z2 - z_mm));
        } else if (t1 > 1e-6) {
            t = t1;
        } else if (t2 > 1e-6) {
            t = t2;
        }
    }
    if (t <= 0.0) {
        return plane_hit(pos, dir, z_mm, true);
    }
    let hit = pos + dir * t;
    var n = (hit - centre) / abs(radius);
    if (dot(n, dir) > 0.0) {
        n = -n;
    }
    out.pos = hit;
    out.normal = n;
    out.ok = true;
    return out;
}

fn refract_dir(dir: vec3<f32>, n: vec3<f32>, o: f32) -> vec4<f32> {
    // xyz = direction, w = 1 live / 0 dead (TIR or degenerate).
    let cos_i = -dot(dir, n);
    let sin2_t = o * o * (1.0 - cos_i * cos_i);
    if (sin2_t >= 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let k = o * cos_i - sqrt(1.0 - sin2_t);
    let v = dir * o + n * k;
    let sq = dot(v, v);
    if (!(sq > 1e-18)) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return vec4<f32>(normalize(v), 1.0);
}

fn reflect_dir(dir: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return normalize(dir - n * (2.0 * dot(dir, n)));
}

// One surface event's radiometry (K-364, lumit_core `trace_splat_spectral`'s
// inner loop): plain Fresnel at the band's own wavelength is computed once by
// the caller, then each sub-sample blends it toward the baked coating at ITS
// wavelength by the Coating dial and multiplies the throughput it keeps —
// the reflected share at a bounce, the transmitted share at a crossing.
fn surface_event(
    thru: ptr<function, array<f32, 8u>>,
    band: u32,
    surf: u32,
    reverse: u32,
    cos_i: f32,
    plain: f32,
    is_reflection: bool,
) {
    let mix = clamp(tp.coating, 0.0, 1.0);
    for (var k = 0u; k < SPECTRAL_SUB; k = k + 1u) {
        let coated = refl_lookup(surf, reverse, band_subs[band * SPECTRAL_SUB + k].lambda_idx, cos_i);
        let rk = clamp(plain + (coated - plain) * mix, 0.0, 1.0);
        (*thru)[k] = (*thru)[k] * select(1.0 - rk, rk, is_reflection);
    }
}

fn semi_of(s: Surface) -> f32 {
    if (s.is_stop > 0.5) {
        return s.semi_ap_mm * tp.stop_scale;
    }
    return s.semi_ap_mm;
}

@compute @workgroup_size(64)
fn trace(@builtin(global_invocation_id) gid: vec3<u32>) {
    let ray_count = tp.grid * tp.grid;
    if (gid.x >= ray_count || gid.y >= tp.combo_count || gid.z >= tp.light_count) {
        return;
    }
    let slot = (gid.z * tp.combo_count + gid.y) * tp.ray_stride + gid.x;
    var dead: Ray;
    dead.pos_x = 0.0;
    dead.pos_y = 0.0;
    dead.weight = -1.0;
    dead._pad = 0.0;
    dead.r = 0.0;
    dead.g = 0.0;
    dead.b = 0.0;
    dead._pad2 = 0.0;

    let light = lights[tp.light_offset + gid.z];
    if (light_dead(light)) {
        rays[slot] = dead;
        return;
    }
    let combo = combos[tp.combo_offset + gid.y];
    let a_idx = combo.bounce_a;
    let b_idx = combo.bounce_b;
    let c_idx = combo.bounce_c;
    let d_idx = combo.bounce_d;
    let four = c_idx != NO_BOUNCE;
    if (a_idx >= b_idx || b_idx >= tp.surface_count) {
        rays[slot] = dead;
        return;
    }
    if (four && (c_idx >= tp.surface_count || c_idx <= a_idx || d_idx >= c_idx)) {
        rays[slot] = dead;
        return;
    }

    let gi = gid.x % tp.grid;
    let gj = gid.x / tp.grid;
    let g1 = f32(max(tp.grid, 2u) - 1u);
    let u = (f32(gi) / g1) * 2.0 - 1.0;
    let v = (f32(gj) / g1) * 2.0 - 1.0;
    // A masked-out corner still traces (K-264): weight zero, geometry
    // real, so iris edges fade inside their cell instead of quantising to
    // it — unless it sits so far outside the iris (zero beyond radius 1)
    // that no cell touching it can hold any lit corner; the CPU twin's
    // comment tells the story.
    let spacing = 2.0 / g1;
    let lim = 1.0 + 1.5 * spacing;
    if (u * u + v * v > lim * lim) {
        rays[slot] = dead;
        return;
    }
    // The shape of the hole the light comes through, with this ghost's own
    // edge diffraction ringing its rim (K-369, re-derived K-370). `spacing`
    // is the ray grid's step, and it is what decides whether fringes this
    // fine can be drawn at all rather than aliased into a pattern across the
    // whole ghost.
    let mask = ghost_mask(u, v, combo.ring_fresnel, spacing);

    var pos = vec3<f32>(u * tp.pupil_mm, v * tp.pupil_mm, tp.start_z_mm);
    // The ray's own point of the source (K-367): a point source jitters by
    // zero and this is bit-identical to the single direction it always had.
    let jit = source_jitter(gi, gj, vec2<f32>(light.ext_x, light.ext_y));
    var dir = dir_of(light.pos_x + jit.x, light.pos_y + jit.y);
    // Per-sub-sample energy throughput (K-364): the geometry is shared, the
    // radiometry is not — the coating's reflectance swings across a band
    // that a single traced wavelength cannot resolve.
    var thru = array<f32, 8u>(1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0);
    var current = 1.0;
    // Worst relative aperture crossing (K-261): grazing rays fade via the
    // 0.95..1 feather below instead of the hard clip alone. Tracked SQUARED
    // and rooted once at the end (K-263) — `max` and `sqrt` commute for
    // non-negative values, so it is the same number for one square root a ray
    // instead of one per surface crossed, on the effect's hottest loop.
    var rrel2 = 0.0;
    let lambda = combo.lambda_nm;

    // Phase 1: forward through 0..=b, reflecting at b.
    for (var s_idx = 0u; s_idx <= b_idx; s_idx = s_idx + 1u) {
        let s = surfaces[s_idx];
        let hit = intersect(pos, dir, s.radius_mm, s.z_mm);
        if (!hit.ok) {
            rays[slot] = dead;
            return;
        }
        pos = hit.pos;
        if (hit.missed) {
            rrel2 = max(rrel2, 4.0);
        }
        // min(clear aperture, glass extent): see the CPU twin (K-264).
        var semi_r = max(semi_of(s), 1e-6);
        if (abs(s.radius_mm) >= 1e-6) {
            semi_r = max(min(semi_r, abs(s.radius_mm)), 1e-6);
        }
        rrel2 = max(rrel2, (pos.x * pos.x + pos.y * pos.y) / (semi_r * semi_r));
        let n2 = cauchy_ior(s.cauchy_a, s.cauchy_b, lambda);
        let cos_i = abs(dot(hit.normal, dir));
        let plain = fresnel_cos(cos_i, current, n2);
        let is_bounce = s_idx == b_idx;
        surface_event(&thru, combo.band, s_idx, 0u, cos_i, plain, is_bounce);
        if (is_bounce) {
            dir = reflect_dir(dir, hit.normal);
        } else {
            let refr = refract_dir(dir, hit.normal, current / n2);
            if (refr.w < 0.5) {
                // TIR: continue straight, weight zero (K-264; the CPU
                // twin's comment tells the story).
                rrel2 = max(rrel2, 4.0);
            } else {
                dir = refr.xyz;
            }
            current = n2;
        }
    }

    // Phase 2: backward through b-1..=a, reflecting at a. This is the one
    // leg that reads the reflectance table REVERSED (K-364): the ray crosses
    // each surface back to front, so the two media swap.
    for (var k = b_idx; k > a_idx; k = k - 1u) {
        let s_idx = k - 1u;
        let s = surfaces[s_idx];
        let hit = intersect(pos, dir, s.radius_mm, s.z_mm);
        if (!hit.ok) {
            rays[slot] = dead;
            return;
        }
        pos = hit.pos;
        if (hit.missed) {
            rrel2 = max(rrel2, 4.0);
        }
        // min(clear aperture, glass extent): see the CPU twin (K-264).
        var semi_r = max(semi_of(s), 1e-6);
        if (abs(s.radius_mm) >= 1e-6) {
            semi_r = max(min(semi_r, abs(s.radius_mm)), 1e-6);
        }
        rrel2 = max(rrel2, (pos.x * pos.x + pos.y * pos.y) / (semi_r * semi_r));
        var n2 = 1.0;
        if (s_idx > 0u) {
            let before = surfaces[s_idx - 1u];
            n2 = cauchy_ior(before.cauchy_a, before.cauchy_b, lambda);
        }
        let cos_i = abs(dot(hit.normal, dir));
        let plain = fresnel_cos(cos_i, current, n2);
        let is_bounce = s_idx == a_idx;
        surface_event(&thru, combo.band, s_idx, 1u, cos_i, plain, is_bounce);
        if (is_bounce) {
            dir = reflect_dir(dir, hit.normal);
            current = cauchy_ior(s.cauchy_a, s.cauchy_b, lambda);
        } else {
            let refr = refract_dir(dir, hit.normal, current / n2);
            if (refr.w < 0.5) {
                // TIR: continue straight, weight zero (K-264; the CPU
                // twin's comment tells the story).
                rrel2 = max(rrel2, 4.0);
            } else {
                dir = refr.xyz;
            }
            current = n2;
        }
    }

    // Phase 3: forward through a+1.., reflecting at c if the path has a
    // third bounce (K-368). Without one the end is the surface count and
    // `c_idx` the sentinel no index can equal, so a two-bounce ghost runs
    // exactly the phase it always did.
    let end3 = select(tp.surface_count, c_idx + 1u, four);
    for (var s_idx = a_idx + 1u; s_idx < end3; s_idx = s_idx + 1u) {
        let s = surfaces[s_idx];
        let hit = intersect(pos, dir, s.radius_mm, s.z_mm);
        if (!hit.ok) {
            rays[slot] = dead;
            return;
        }
        pos = hit.pos;
        if (hit.missed) {
            rrel2 = max(rrel2, 4.0);
        }
        // min(clear aperture, glass extent): see the CPU twin (K-264).
        var semi_r = max(semi_of(s), 1e-6);
        if (abs(s.radius_mm) >= 1e-6) {
            semi_r = max(min(semi_r, abs(s.radius_mm)), 1e-6);
        }
        rrel2 = max(rrel2, (pos.x * pos.x + pos.y * pos.y) / (semi_r * semi_r));
        let n2 = cauchy_ior(s.cauchy_a, s.cauchy_b, lambda);
        let cos_i = abs(dot(hit.normal, dir));
        let plain = fresnel_cos(cos_i, current, n2);
        let is_bounce = s_idx == c_idx;
        surface_event(&thru, combo.band, s_idx, 0u, cos_i, plain, is_bounce);
        if (is_bounce) {
            dir = reflect_dir(dir, hit.normal);
        } else {
            let refr = refract_dir(dir, hit.normal, current / n2);
            if (refr.w < 0.5) {
                // TIR: continue straight, weight zero (K-264).
                rrel2 = max(rrel2, 4.0);
            } else {
                dir = refr.xyz;
            }
            current = n2;
        }
    }

    if (four) {
        // Phase 4 (K-368): backward through c-1..=d, reflecting at d —
        // phase 2's walk again, one leg further in, table read reversed.
        for (var k = c_idx; k > d_idx; k = k - 1u) {
            let s_idx = k - 1u;
            let s = surfaces[s_idx];
            let hit = intersect(pos, dir, s.radius_mm, s.z_mm);
            if (!hit.ok) {
                rays[slot] = dead;
                return;
            }
            pos = hit.pos;
            if (hit.missed) {
                rrel2 = max(rrel2, 4.0);
            }
            var semi_r = max(semi_of(s), 1e-6);
            if (abs(s.radius_mm) >= 1e-6) {
                semi_r = max(min(semi_r, abs(s.radius_mm)), 1e-6);
            }
            rrel2 = max(rrel2, (pos.x * pos.x + pos.y * pos.y) / (semi_r * semi_r));
            var n2 = 1.0;
            if (s_idx > 0u) {
                let before = surfaces[s_idx - 1u];
                n2 = cauchy_ior(before.cauchy_a, before.cauchy_b, lambda);
            }
            let cos_i = abs(dot(hit.normal, dir));
            let plain = fresnel_cos(cos_i, current, n2);
            let is_bounce = s_idx == d_idx;
            surface_event(&thru, combo.band, s_idx, 1u, cos_i, plain, is_bounce);
            if (is_bounce) {
                dir = reflect_dir(dir, hit.normal);
                current = cauchy_ior(s.cauchy_a, s.cauchy_b, lambda);
            } else {
                let refr = refract_dir(dir, hit.normal, current / n2);
                if (refr.w < 0.5) {
                    rrel2 = max(rrel2, 4.0);
                } else {
                    dir = refr.xyz;
                }
                current = n2;
            }
        }

        // Phase 5: forward through d+1..n.
        for (var s_idx = d_idx + 1u; s_idx < tp.surface_count; s_idx = s_idx + 1u) {
            let s = surfaces[s_idx];
            let hit = intersect(pos, dir, s.radius_mm, s.z_mm);
            if (!hit.ok) {
                rays[slot] = dead;
                return;
            }
            pos = hit.pos;
            if (hit.missed) {
                rrel2 = max(rrel2, 4.0);
            }
            var semi_r = max(semi_of(s), 1e-6);
            if (abs(s.radius_mm) >= 1e-6) {
                semi_r = max(min(semi_r, abs(s.radius_mm)), 1e-6);
            }
            rrel2 = max(rrel2, (pos.x * pos.x + pos.y * pos.y) / (semi_r * semi_r));
            let n2 = cauchy_ior(s.cauchy_a, s.cauchy_b, lambda);
            let cos_i = abs(dot(hit.normal, dir));
            let plain = fresnel_cos(cos_i, current, n2);
            surface_event(&thru, combo.band, s_idx, 0u, cos_i, plain, false);
            let refr = refract_dir(dir, hit.normal, current / n2);
            if (refr.w < 0.5) {
                rrel2 = max(rrel2, 4.0);
            } else {
                dir = refr.xyz;
            }
            current = n2;
        }
    }

    // Propagate to the (focus-shifted) sensor plane.
    if (abs(dir.z) < 1e-12) {
        rays[slot] = dead;
        return;
    }
    let t = (tp.sensor_z_mm + tp.sensor_shift_mm - pos.z) / dir.z;
    if (!(t > 0.0)) {
        rays[slot] = dead;
        return;
    }
    let land = pos + dir * t;
    let px = land.x * tp.screen_transform + tp.raster_w / 2.0;
    let py = tp.raster_h / 2.0 - land.y * tp.screen_transform;
    if (!(abs(px) < 1e9) || !(abs(py) < 1e9)) {
        rays[slot] = dead;
        return;
    }
    // Band-integrate (K-364): rgb = sum over subs of throughput × CIE weight.
    // A throughput that has gone non-finite is dead, as it is on the CPU.
    var rgb = vec3<f32>(0.0);
    for (var k = 0u; k < SPECTRAL_SUB; k = k + 1u) {
        let t = thru[k];
        if (!(abs(t) < 3.4e38)) {
            rays[slot] = dead;
            return;
        }
        let sub = band_subs[combo.band * SPECTRAL_SUB + k];
        rgb = rgb + t * vec3<f32>(sub.r, sub.g, sub.b);
    }
    // Housing feather: full inside 0.95, gone at 1.0 (smoothstep). Since
    // K-364 the weight is geometry ALONE — feather × iris mask, as the CPU
    // twin's caller folds it — and the energy travels in rgb.
    let ft = clamp((1.0 - sqrt(rrel2)) / 0.05, 0.0, 1.0);
    var out: Ray;
    out.pos_x = px;
    out.pos_y = py;
    out.weight = ft * ft * (3.0 - 2.0 * ft) * mask;
    out._pad = 0.0;
    out.r = rgb.x;
    out.g = rgb.y;
    out.b = rgb.z;
    out._pad2 = 0.0;
    rays[slot] = out;
}

// ---------------------------------------------------------------------------
// Per-ray splatting (K-366) — the WGSL twin of lumit_core's `ray_axes` and
// the front half of `splat_ray`, op for op.
// ---------------------------------------------------------------------------

// Shortest half-axis a splat's footprint may have, px — the anti-alias
// floor. Must equal lumit_core's `MIN_SPLAT_AXIS_PX` (pinned by test).
const MIN_SPLAT_AXIS_PX: f32 = 0.75;
// Floor on a ray's landed footprint as a fraction of its launch cell — the
// caustic density cap. Must equal lumit_core's `MIN_AREA_FRAC` (same test).
const MIN_AREA_FRAC: f32 = 0.003;

// A neighbour's landing: xy = position, z = 1 live / 0 dead (weight < 0 is
// the trace's dead sentinel, where the CPU reference holds None).
fn landing_at(base: u32, x: u32, y: u32) -> vec3<f32> {
    let r = rays[base + y * tp.grid + x];
    if (r.weight < 0.0) {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    return vec3<f32>(r.pos_x, r.pos_y, 1.0);
}

// One axis of the footprint by central difference over the two neighbours,
// one-sided where only one of them survives, absent where neither does.
fn axis_diff(lo: vec3<f32>, here: vec2<f32>, hi: vec3<f32>) -> vec3<f32> {
    if (lo.z > 0.5 && hi.z > 0.5) {
        return vec3<f32>((hi.xy - lo.xy) / 2.0, 1.0);
    }
    if (hi.z > 0.5) {
        return vec3<f32>(hi.xy - here, 1.0);
    }
    if (lo.z > 0.5) {
        return vec3<f32>(here - lo.xy, 1.0);
    }
    return vec3<f32>(0.0, 0.0, 0.0);
}

// The anti-alias floor on one axis, direction preserved.
fn floor_axis(a: vec2<f32>, fallback: vec2<f32>) -> vec2<f32> {
    let len = sqrt(a.x * a.x + a.y * a.y);
    if (len < 1e-6) {
        return fallback;
    }
    if (len < MIN_SPLAT_AXIS_PX) {
        return a * (MIN_SPLAT_AXIS_PX / len);
    }
    return a;
}

@compute @workgroup_size(64)
fn build_splats(@builtin(global_invocation_id) gid: vec3<u32>) {
    // Dispatched over THIS batch's rays (K-263): one thread per ray, so the
    // splats index exactly as the rays do.
    if (gid.x >= tp.ray_stride || gid.y >= tp.combo_count || gid.z >= tp.light_count) {
        return;
    }
    let slot = gid.z * tp.combo_count + gid.y;
    let base = slot * tp.ray_stride;
    let out_i = base + gid.x;
    var s: Splat;
    s.cx = 0.0;
    s.cy = 0.0;
    s.a1x = 0.0;
    s.a1y = 0.0;
    s.a2x = 0.0;
    s.a2y = 0.0;
    s.r = 0.0;
    s.g = 0.0;
    s.b = 0.0;
    s.live = 0.0;
    s._pad0 = 0.0;
    s._pad1 = 0.0;

    let ray = rays[out_i];
    // A dead ray, or one the iris masked out entirely, deposits nothing —
    // but its slot is still written, because the draw walks the batch's
    // splats as one contiguous instance range.
    if (!(ray.weight > 0.0)) {
        splats[out_i] = s;
        return;
    }
    let i = gid.x % tp.grid;
    let j = gid.x / tp.grid;
    let here = vec2<f32>(ray.pos_x, ray.pos_y);

    var lo_x = vec3<f32>(0.0);
    if (i > 0u) {
        lo_x = landing_at(base, i - 1u, j);
    }
    var hi_x = vec3<f32>(0.0);
    if (i + 1u < tp.grid) {
        hi_x = landing_at(base, i + 1u, j);
    }
    var lo_y = vec3<f32>(0.0);
    if (j > 0u) {
        lo_y = landing_at(base, i, j - 1u);
    }
    var hi_y = vec3<f32>(0.0);
    if (j + 1u < tp.grid) {
        hi_y = landing_at(base, i, j + 1u);
    }
    let ax = axis_diff(lo_x, here, hi_x);
    let ay = axis_diff(lo_y, here, hi_y);

    // Half-axes: the splat spans centre ± a, so a full grid step is 2a. A
    // dead axis borrows the live one's right angle at the anti-alias floor,
    // and a lone survivor takes the floor in both directions.
    var a1 = vec2<f32>(MIN_SPLAT_AXIS_PX, 0.0);
    var a2 = vec2<f32>(0.0, MIN_SPLAT_AXIS_PX);
    if (ax.z > 0.5 && ay.z > 0.5) {
        a1 = ax.xy / 2.0;
        a2 = ay.xy / 2.0;
    } else if (ax.z > 0.5) {
        let len = max(sqrt(ax.x * ax.x + ax.y * ax.y), 1e-6);
        a1 = ax.xy / 2.0;
        a2 = vec2<f32>(-ax.y / len * MIN_SPLAT_AXIS_PX, ax.x / len * MIN_SPLAT_AXIS_PX);
    } else if (ay.z > 0.5) {
        let len = max(sqrt(ay.x * ay.x + ay.y * ay.y), 1e-6);
        a1 = vec2<f32>(-ay.y / len * MIN_SPLAT_AXIS_PX, ay.x / len * MIN_SPLAT_AXIS_PX);
        a2 = ay.xy / 2.0;
    }

    a1 = floor_axis(a1, vec2<f32>(MIN_SPLAT_AXIS_PX, 0.0));
    a2 = floor_axis(a2, vec2<f32>(0.0, MIN_SPLAT_AXIS_PX));
    // Near-parallel axes are a fold seen edge-on: the footprint collapses
    // even though both axes are long. Push a2 across a1 up to the floor so
    // the deposit is at least a pixel-wide line, not a zero-area
    // parallelogram whose flux vanishes.
    var det = a1.x * a2.y - a1.y * a2.x;
    let a1_len = max(sqrt(a1.x * a1.x + a1.y * a1.y), 1e-6);
    if (abs(det) < MIN_SPLAT_AXIS_PX * a1_len) {
        let n = vec2<f32>(-a1.y / a1_len, a1.x / a1_len);
        let sgn = select(-1.0, 1.0, det >= 0.0);
        a2 = a2 + n * (MIN_SPLAT_AXIS_PX * sgn);
        det = a1.x * a2.y - a1.y * a2.x;
    }
    let area = max(abs(det), 1e-6);
    // The density cap: the divisor never drops below the launch cell's
    // capped fraction, so a fold brightens to 333× and stops (K-262's rule,
    // carried over unchanged).
    let divisor = max(area, MIN_AREA_FRAC * tp.cell_area_px);
    // The ray's flux is its launch cell's, weighted by geometry; its colour
    // is the band-integrated throughput (gain already folded into the band
    // sub-samples) times the light's own. Dividing here leaves the fragment
    // with nothing to do but the tent.
    let light = lights[tp.light_offset + gid.z];
    let flux = ray.weight * tp.cell_area_px;
    let peak = (flux * vec3<f32>(ray.r, ray.g, ray.b) * vec3<f32>(light.r, light.g, light.b))
        / divisor;

    s.cx = here.x;
    s.cy = here.y;
    s.a1x = a1.x;
    s.a1y = a1.y;
    s.a2x = a2.x;
    s.a2y = a2.y;
    s.r = peak.x;
    s.g = peak.y;
    s.b = peak.z;
    s.live = 1.0;
    splats[out_i] = s;
}
