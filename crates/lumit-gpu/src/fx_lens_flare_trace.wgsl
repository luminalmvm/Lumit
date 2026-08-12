// Lens flare (docs/08-EFFECTS.md §3.27, docs/impl/lens-flare.md, K-261,
// K-263/K-264). Three compute entry points of the per-frame ghost pipeline:
//   trace       — one thread per pupil-grid corner: refract the ray through
//                 the prescription with the FlareSim three-phase walk
//                 (reflecting at the pair's two surfaces), carry eight
//                 spectral throughputs through the per-surface reflectance
//                 (K-364), land on the sensor and band-integrate them into
//                 the ray's rgb; the weight the corner keeps is geometry
//                 alone — housing feather × iris mask. Mirrors lumit_core's
//                 `trace_splat_spectral` op-for-op; the dead sentinel is
//                 weight −1 (the CPU returns None).
//   quad_area   — one thread per grid cell: the cell's landed area in
//                 flare-buffer px² (0 = a dead corner). Not folded into
//                 build_verts (K-264), because build_verts reads the areas
//                 of NEIGHBOURING cells too — the vertex-smoothed density —
//                 and neighbours across a workgroup boundary need a pass
//                 boundary to be visible.
//   build_verts — one thread per grid cell: density at each of the cell's
//                 four corners is launch cell area over the MEAN landed
//                 area of the live cells touching that corner ([Hullin
//                 2011]'s per-vertex rule, K-264) — continuous across the
//                 grid where the old per-cell density jumped at every cell
//                 edge and drew the Ultra faceting. Sub-pixel COMPACT quads
//                 inflate about their centroid with flux conserved (K-261);
//                 sub-pixel slivers park; long thin fold cells DRAW (K-264
//                 — parking them cut notches into every caustic rim).
// The additive raster lives in fx_lens_flare_draw.wgsl (4x multisampled
// since K-264), the box blur in fx_lens_flare_blur.wgsl, detection in
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

// One (pair × wavelength) combo: the two bounce surfaces, the wavelength the
// GEOMETRY is traced at, and the band's index into `band_subs` — the
// radiometric sub-samples the energy is carried at (K-364).
struct Combo {
    bounce_a: u32,
    bounce_b: u32,
    lambda_nm: f32,
    _pad: f32,
    band: u32,
    _pad2: f32,
    _pad3: f32,
    _pad4: f32,
};

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

// One drawn corner (K-263): clip position and additive colour, nothing else.
// Through K-262 this carried three pad floats AND was written six times per
// cell (the two triangles' vertex lists spelled out). A cell now stores its
// four corners once at 20 bytes each — 80 bytes a cell where it was 192 — and
// the raster's vertex shader maps its six vertex indices onto them. Same
// triangles, same order, 2.4× less vertex memory to write and to read back.
struct Vertex {
    ndc_x: f32,
    ndc_y: f32,
    r: f32,
    g: f32,
    b: f32,
};

// One flare source (see fx_lens_flare_detect.wgsl): position as a raster
// fraction, colour already multiplied by its gate weight. All-zero rgb is a
// dead slot the passes skip.
struct Light {
    pos_x: f32,
    pos_y: f32,
    r: f32,
    g: f32,
    b: f32,
    _pad0: f32,
    _pad1: f32,
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
    quad_stride: u32,      // quads per slot — THIS batch's (grid-1)² (K-263)
    blades: u32,
    rot_rad: f32,
    roundness: f32,        // effective (wide-open blended)
    softness: f32,
    light_offset: u32,     // first light of this chunk (K-263)
};

@group(0) @binding(0) var<storage, read> surfaces: array<Surface>;
@group(0) @binding(1) var<storage, read> combos: array<Combo>;
@group(0) @binding(2) var<storage, read_write> rays: array<Ray>;
// Landed area per grid cell, px²; 0 = dead (K-264).
@group(0) @binding(3) var<storage, read_write> areas: array<f32>;
@group(0) @binding(4) var<storage, read_write> verts: array<Vertex>;
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
    if (a_idx >= b_idx || b_idx >= tp.surface_count) {
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
    let mask = pupil_mask(u, v);

    var pos = vec3<f32>(u * tp.pupil_mm, v * tp.pupil_mm, tp.start_z_mm);
    var dir = dir_of(light.pos_x, light.pos_y);
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

    // Phase 3: forward through a+1..n.
    for (var s_idx = a_idx + 1u; s_idx < tp.surface_count; s_idx = s_idx + 1u) {
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
        surface_event(&thru, combo.band, s_idx, 0u, cos_i, plain, false);
        let refr = refract_dir(dir, hit.normal, current / n2);
        if (refr.w < 0.5) {
            // TIR: continue straight, weight zero (K-264).
            rrel2 = max(rrel2, 4.0);
        } else {
            dir = refr.xyz;
        }
        current = n2;
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

fn edge_px(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    return (a.x - b.x) * (c.y - a.y) - (a.y - b.y) * (c.x - a.x);
}

// Park a culled cell: four degenerate, black, off-screen corners. The cell
// still occupies its slot in the batch's contiguous draw range, so it must be
// written rather than skipped.
fn park_cell(out_base: u32) {
    var park: Vertex;
    park.ndc_x = -4.0;
    park.ndc_y = -4.0;
    park.r = 0.0;
    park.g = 0.0;
    park.b = 0.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        verts[out_base + i] = park;
    }
}

// The cell's landed area from its four rays; negative-weight (dead)
// corners make it 0. Shared by quad_area and, for the cell's own guard,
// build_verts.
fn cell_area_of(base: u32, qx: u32, qy: u32) -> f32 {
    let c0 = rays[base + qy * tp.grid + qx];
    let c1 = rays[base + qy * tp.grid + qx + 1u];
    let c2 = rays[base + (qy + 1u) * tp.grid + qx + 1u];
    let c3 = rays[base + (qy + 1u) * tp.grid + qx];
    if (c0.weight < 0.0 || c1.weight < 0.0 || c2.weight < 0.0 || c3.weight < 0.0) {
        return 0.0;
    }
    let p0 = vec2<f32>(c0.pos_x, c0.pos_y);
    let p1 = vec2<f32>(c1.pos_x, c1.pos_y);
    let p2 = vec2<f32>(c2.pos_x, c2.pos_y);
    let p3 = vec2<f32>(c3.pos_x, c3.pos_y);
    let a0 = edge_px(p0, p1, p2);
    let a1 = edge_px(p0, p2, p3);
    return abs((a0 + a1) / 2.0);
}

@compute @workgroup_size(64)
fn quad_area(@builtin(global_invocation_id) gid: vec3<u32>) {
    let side = tp.grid - 1u;
    if (gid.x >= tp.quad_stride || gid.y >= tp.combo_count || gid.z >= tp.light_count) {
        return;
    }
    let slot = gid.z * tp.combo_count + gid.y;
    let qx = gid.x % side;
    let qy = gid.x / side;
    areas[slot * tp.quad_stride + gid.x] = cell_area_of(slot * tp.ray_stride, qx, qy);
}

// Density at grid corner (cx, cy): launch cell area over the MEAN landed
// area of the live cells touching the corner (K-264, [Hullin 2011]'s
// per-vertex rule; the CPU twin is `corner_density` in cpu_flare). The
// MIN_AREA_FRAC floor on the mean is still the caustic density cap.
fn corner_mean_area(area_base: u32, side: u32, cx: u32, cy: u32) -> f32 {
    var sum = 0.0;
    var n = 0.0;
    let qx0 = max(cx, 1u) - 1u;
    let qy0 = max(cy, 1u) - 1u;
    let qx1 = min(cx, side - 1u);
    let qy1 = min(cy, side - 1u);
    for (var qy = qy0; qy <= qy1; qy = qy + 1u) {
        for (var qx = qx0; qx <= qx1; qx = qx + 1u) {
            let a = areas[area_base + qy * side + qx];
            if (a > 0.0) {
                sum = sum + a;
                n = n + 1.0;
            }
        }
    }
    if (n > 0.0) {
        return sum / n;
    }
    return 0.0;
}

fn density_of(mean: f32) -> f32 {
    return tp.cell_area_px / max(mean, 3e-3 * tp.cell_area_px);
}

// A corner's colour: the mean of weight × rgb over its 3x3 ray
// neighbourhood, dead rays as zero (K-266, spectral since K-364). The raw
// per-ray weight cliffs — the housing feather compressed into less than a
// cell, a vignette cut — land inside one cell and drew every wash ghost's
// edge as chunky facets; smoothed one lattice step, a cliff becomes a
// two-cell ramp and the raster's interpolation does the rest. Geometry
// decisions (lit corners, pull-in) stay on RAW weights: smearing light onto
// a virtual continuation's far-flung corner would draw the K-264 fan lines
// again.
//
// Weight and colour smooth TOGETHER rather than the weight alone times a
// per-combo tint: a ray's rgb is its own now, and with a constant rgb this
// is exactly the old smooth × that rgb, so the K-266 cliff-smoothing did
// not change shape.
fn smooth_rgb(base: u32, cx: u32, cy: u32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let x = u32(clamp(i32(cx) + dx, 0, i32(tp.grid) - 1));
            let y = u32(clamp(i32(cy) + dy, 0, i32(tp.grid) - 1));
            let ray = rays[base + y * tp.grid + x];
            let w = max(ray.weight, 0.0);
            sum = sum + w * vec3<f32>(ray.r, ray.g, ray.b);
        }
    }
    return sum / 9.0;
}

// The SMALLEST live cell touching a corner — the pull-in's length scale
// (K-265; the CPU twin's comment tells the story).
fn corner_min_area(area_base: u32, side: u32, cx: u32, cy: u32) -> f32 {
    var m = 1e30;
    let qx0 = max(cx, 1u) - 1u;
    let qy0 = max(cy, 1u) - 1u;
    let qx1 = min(cx, side - 1u);
    let qy1 = min(cy, side - 1u);
    for (var qy = qy0; qy <= qy1; qy = qy + 1u) {
        for (var qx = qx0; qx <= qx1; qx = qx + 1u) {
            let a = areas[area_base + qy * side + qx];
            if (a > 0.0 && a < m) {
                m = a;
            }
        }
    }
    if (m == 1e30) {
        return 0.0;
    }
    return m;
}

@compute @workgroup_size(64)
fn build_verts(@builtin(global_invocation_id) gid: vec3<u32>) {
    let side = tp.grid - 1u;
    // Dispatched over THIS batch's cells (K-263): a batch is a run of
    // combos at ONE grid, so the stride is its own and its cells are
    // contiguous — nothing outside them is dispatched, written, or drawn.
    if (gid.x >= tp.quad_stride || gid.y >= tp.combo_count || gid.z >= tp.light_count) {
        return;
    }
    let slot = gid.z * tp.combo_count + gid.y;
    let out_base = (slot * tp.quad_stride + gid.x) * 4u;
    let qx = gid.x % side;
    let qy = gid.x / side;
    let area_px = areas[slot * tp.quad_stride + gid.x];
    if (area_px <= 0.0) {
        park_cell(out_base);
        return;
    }
    let base = slot * tp.ray_stride;
    let c0 = rays[base + qy * tp.grid + qx];
    let c1 = rays[base + qy * tp.grid + qx + 1u];
    let c2 = rays[base + (qy + 1u) * tp.grid + qx + 1u];
    let c3 = rays[base + (qy + 1u) * tp.grid + qx];
    var p = array<vec2<f32>, 4>(
        vec2<f32>(c0.pos_x, c0.pos_y),
        vec2<f32>(c1.pos_x, c1.pos_y),
        vec2<f32>(c2.pos_x, c2.pos_y),
        vec2<f32>(c3.pos_x, c3.pos_y),
    );
    let area_base = slot * tp.quad_stride;
    // Vertex-smoothed density (K-264): each corner's own neighbourhood
    // mean, interpolated by the raster — continuous across cells, where the
    // per-cell constant of K-263 jumped at every cell edge (the faceting).

    var means = array<f32, 4>(
        corner_mean_area(area_base, side, qx, qy),
        corner_mean_area(area_base, side, qx + 1u, qy),
        corner_mean_area(area_base, side, qx + 1u, qy + 1u),
        corner_mean_area(area_base, side, qx, qy + 1u),
    );
    var d = array<f32, 4>(
        density_of(means[0]),
        density_of(means[1]),
        density_of(means[2]),
        density_of(means[3]),
    );
    let light = lights[tp.light_offset + gid.z];
    // The combo carries no colour since K-364 — the ray does, and the gain
    // rides the band's sub-sample weights.
    let tint = vec3<f32>(light.r, light.g, light.b);
    var col = array<vec3<f32>, 4>(
        tint * (d[0] * smooth_rgb(base, qx, qy)),
        tint * (d[1] * smooth_rgb(base, qx + 1u, qy)),
        tint * (d[2] * smooth_rgb(base, qx + 1u, qy + 1u)),
        tint * (d[3] * smooth_rgb(base, qx, qy + 1u)),
    );
    // Flux-conserving sub-pixel inflation (K-261, refined K-262/K-264 — the
    // CPU `inflate_quad` twin). A sub-pixel COMPACT quad inflates about its
    // centroid, colour scaled by true ÷ inflated area, so the raster cannot
    // drop its caustic flux; a sub-pixel SLIVER parks (inflating one is the
    // K-261 streak artefact; un-inflated it covers nothing). A long thin
    // cell at drawable size DRAWS since K-264: its brightness is now its
    // neighbourhood's, and parking it cut notches into every caustic rim.
    let min_quad_px = 1.0;
    let max_inflate_edge_px = 6.0;
    // Rein in the unlit corners (K-264, the CPU cell loop's twin). A cell
    // spanning from lit geometry to a mount-absorbed virtual continuation
    // can be hundreds of px long; drawn it fans a faint line out of the
    // ghost's bore, dropped it notches the bore's edge. The zero-weight
    // corner carries no light — its only job is geometry — so it is pulled
    // toward the lit corners' centroid to within a few cell-widths, and
    // the fade to zero lands where the boundary is.
    var w4 = array<f32, 4>(c0.weight, c1.weight, c2.weight, c3.weight);
    var lit_n = 0.0;
    var lit_c = vec2<f32>(0.0);
    for (var i = 0; i < 4; i = i + 1) {
        if (w4[i] > 0.0) {
            lit_n = lit_n + 1.0;
            lit_c = lit_c + p[i];
        }
    }
    if (lit_n == 0.0) {
        // No light anywhere in the cell: nothing to draw.
        park_cell(out_base);
        return;
    }
    if (lit_n < 4.0) {
        lit_c = lit_c / lit_n;
        // Smallest lit neighbour, not the mean: fold-stretched neighbours
        // grew the reach into sawteeth (K-265).
        var min_area = 1e30;
        for (var i = 0; i < 4; i = i + 1) {
            if (w4[i] <= 0.0) {
                continue;
            }
            var cx = qx;
            var cy = qy;
            if (i == 1 || i == 2) {
                cx = qx + 1u;
            }
            if (i == 2 || i == 3) {
                cy = qy + 1u;
            }
            let m = corner_min_area(area_base, side, cx, cy);
            if (m > 0.0) {
                min_area = min(min_area, m);
            }
        }
        let reach = max(sqrt(min(min_area, 1e12)), 1.0);
        for (var i = 0; i < 4; i = i + 1) {
            if (w4[i] > 0.0) {
                continue;
            }
            let dd = p[i] - lit_c;
            let dist = length(dd);
            if (dist > reach) {
                p[i] = lit_c + dd * (reach / dist);
            }
        }
    }
    if (area_px < min_quad_px) {
        var longest = 0.0;
        for (var i = 0; i < 4; i = i + 1) {
            let dd = p[i] - p[(i + 1) % 4];
            longest = max(longest, length(dd));
        }
        if (longest > max_inflate_edge_px) {
            park_cell(out_base);
            return;
        }
        let eps = min_quad_px * 1e-4;
        let scl = sqrt(min_quad_px / max(area_px, eps));
        let scale = max(area_px, eps) / min_quad_px;
        let centre = (p[0] + p[1] + p[2] + p[3]) / 4.0;
        for (var i = 0; i < 4; i = i + 1) {
            p[i] = centre + (p[i] - centre) * scl;
            col[i] = col[i] * scale;
        }
    }
    // Corner order 0,1,2,3 around the cell; the raster expands it into the
    // two triangles (0,1,2) and (0,2,3) — the same winding and the same
    // primitive order the six spelled-out vertices had.
    for (var i = 0; i < 4; i = i + 1) {
        var vert: Vertex;
        vert.ndc_x = p[i].x / tp.raster_w * 2.0 - 1.0;
        vert.ndc_y = 1.0 - p[i].y / tp.raster_h * 2.0;
        vert.r = col[i].x;
        vert.g = col[i].y;
        vert.b = col[i].z;
        verts[out_base + u32(i)] = vert;
    }
}
