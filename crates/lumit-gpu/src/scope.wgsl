// The GPU scope pass (docs/07-UI-SPEC.md §8, K-096 v1): waveform, vectorscope
// and histogram traces computed on the graphics card instead of the CPU.
//
// In plain terms: the Scopes panel plots the picture's brightness and colour.
// Doing that meant the CPU walking a quarter-million pixels every frame — the
// "scopes are super laggy" report. This shader does that walk on the GPU: pass
// `bin` drops each sampled pixel into a counting bin (an atomic add, so many
// threads count into the same bin safely), pass `peak` finds the tallest bin,
// and pass `colourise` paints the 256×256 trace texture from the bins. The maths
// mirrors `crates/lumit-ui/src/shell/scopes.rs` op-for-op (the CPU is the
// oracle), and the trace colours ride in as a uniform (no hex in the shader —
// docs/15-DESIGN.md; the caller passes the fixed ScopeColours as bytes).
//
// Rounding note: WGSL `round()` is round-to-nearest-EVEN, but the CPU/Dart
// reference uses `.round()`/`toInt()` = round-half-AWAY / truncate. Every value
// here is non-negative, so `floor(x + 0.5)` reproduces round-half-away and a
// bare `u32(x)` reproduces truncation — used deliberately in place of `round()`.

const GRID: u32 = 256u;

struct Params {
    kind: u32,       // 0 luma, 1 rgb, 2 vectorscope, 3 histogram
    grid: u32,       // always GRID; carried so the shader reads no literal size
    n_cols: u32,     // sampled columns = ceil(width / sx)
    n_rows: u32,     // sampled rows    = ceil(height / sy)
    sx: u32,         // x sample stride
    sy: u32,         // y sample stride
    width: u32,      // source width in pixels
    height: u32,     // source height in pixels
    count_len: u32,  // populated length of `counts` for this kind
    _p0: u32,
    _p1: u32,
    _p2: u32,
    bg: vec4<f32>,    // trace-texture colours, each channel a 0..255 byte value
    trace: vec4<f32>,
    red: vec4<f32>,
    green: vec4<f32>,
    blue: vec4<f32>,
};

// ---- bin + peak bindings (read-write atomics) -----------------------------
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(2) var<storage, read_write> peak: atomic<u32>;
@group(0) @binding(3) var<uniform> p: Params;

// A source pixel's 0..255 channel bytes. `src` is Rgba8Unorm holding the exact
// display bytes the CPU reads back, so `v * 255` is already integral.
fn channel_bytes(x: u32, y: u32) -> vec3<u32> {
    let c = textureLoad(src, vec2<i32>(i32(x), i32(y)), 0);
    return vec3<u32>(
        u32(floor(c.r * 255.0 + 0.5)),
        u32(floor(c.g * 255.0 + 0.5)),
        u32(floor(c.b * 255.0 + 0.5)),
    );
}

// Rec.709 luma of an sRGB (gamma) pixel, bytes → 0..1 (== cpu::luma8).
fn luma8(rgb: vec3<u32>) -> f32 {
    return (0.2126 * f32(rgb.x) + 0.7152 * f32(rgb.y) + 0.0722 * f32(rgb.z)) / 255.0;
}

// Map a 0..1 value to a grid row: 1.0 (bright) at row 0, 0.0 at the bottom.
fn value_row(v: f32) -> u32 {
    let clamped = clamp(v, 0.0, 1.0);
    let row = u32(floor((1.0 - clamped) * (f32(GRID) - 1.0) + 0.5));
    return min(row, GRID - 1u);
}

// The binning pass: one invocation per SAMPLED pixel (the strided grid the CPU
// walks), each incrementing its bin. `gid.xy` indexes the sampled grid, so the
// real pixel is (col*sx, row*sy) — the exact positions `while x < width` visits.
@compute @workgroup_size(8, 8)
fn bin(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.n_cols || gid.y >= p.n_rows) {
        return;
    }
    let x = gid.x * p.sx;
    let y = gid.y * p.sy;
    if (x >= p.width || y >= p.height) {
        return;
    }
    let rgb = channel_bytes(x, y);
    let bx = min((x * GRID) / p.width, GRID - 1u);

    switch (p.kind) {
        case 0u: { // luma waveform
            let by = value_row(luma8(rgb));
            atomicAdd(&counts[by * GRID + bx], 1u);
        }
        case 1u: { // rgb waveform: one grid per channel
            for (var c = 0u; c < 3u; c = c + 1u) {
                let by = value_row(f32(rgb[c]) / 255.0);
                atomicAdd(&counts[c * GRID * GRID + by * GRID + bx], 1u);
            }
        }
        case 2u: { // vectorscope (Rec.601 Cb/Cr)
            let rf = f32(rgb.x) / 255.0;
            let gf = f32(rgb.y) / 255.0;
            let bf = f32(rgb.z) / 255.0;
            let cb = -0.168736 * rf - 0.331264 * gf + 0.5 * bf;
            let cr = 0.5 * rf - 0.418688 * gf - 0.081312 * bf;
            let centre = (f32(GRID) - 1.0) * 0.5;
            let scale = f32(GRID) * 0.9;
            let px = centre + cb * scale;
            let py = centre - cr * scale; // screen y down; Cr up
            if (px >= 0.0 && px < f32(GRID) && py >= 0.0 && py < f32(GRID)) {
                atomicAdd(&counts[u32(py) * GRID + u32(px)], 1u);
            }
        }
        default: { // histogram: per-channel byte counts
            for (var c = 0u; c < 3u; c = c + 1u) {
                let bin_idx = (rgb[c] * (GRID - 1u)) / 255u;
                atomicAdd(&counts[c * GRID + bin_idx], 1u);
            }
        }
    }
}

// The peak pass: fold the populated bins to their maximum (one atomicMax into a
// single slot), so `colourise` can normalise every trace to the tallest bin.
@compute @workgroup_size(256)
fn peak_reduce(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.count_len) {
        return;
    }
    let v = atomicLoad(&counts[gid.x]);
    atomicMax(&peak, v);
}

// ---- colourise bindings (read-only) ---------------------------------------
// Distinct binding numbers from the bin/peak group above: all eight globals
// live in one module at group(0), so each (group, binding) slot maps to exactly
// one variable. The colourise pipeline's bind-group layout carries only 4..7.
@group(0) @binding(4) var<storage, read> counts_ro: array<u32>;
@group(0) @binding(5) var<storage, read> peak_ro: array<u32>;
@group(0) @binding(6) var<uniform> cp: Params;
@group(0) @binding(7) var dst: texture_storage_2d<rgba8unorm, write>;

// A soft, saturating count → 0..1 intensity: the square root lifts faint traces
// without blowing out dense ones (a scope phosphor's falloff). == cpu::intensity.
fn intensity(count: u32, peak_v: u32) -> f32 {
    if (peak_v == 0u) {
        return 0.0;
    }
    let v = f32(count) / f32(peak_v);
    if (v <= 0.0) {
        return 0.0;
    }
    return min(1.0, sqrt(v));
}

// Additive trace over the backdrop, clamped — overlapping channels brighten
// toward white like a real scope. `colour` channels are 0..255 byte values, and
// the byte add truncates toward zero (== cpu::add_trace / Dart `_addTrace`).
fn add_trace(px: ptr<function, vec3<u32>>, colour: vec4<f32>, frac: f32) {
    let f = clamp(frac, 0.0, 1.0);
    (*px).x = min((*px).x + u32(colour.x * f), 255u);
    (*px).y = min((*px).y + u32(colour.y * f), 255u);
    (*px).z = min((*px).z + u32(colour.z * f), 255u);
}

@compute @workgroup_size(8, 8)
fn colourise(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= GRID || gid.y >= GRID) {
        return;
    }
    let cx = gid.x; // column / histogram bin
    let cy = gid.y; // row (0 = top)
    let cell = cy * GRID + cx;
    let peak_v = peak_ro[0];

    var px = vec3<u32>(u32(cp.bg.x), u32(cp.bg.y), u32(cp.bg.z));

    switch (cp.kind) {
        case 0u: { // luma waveform
            let f = intensity(counts_ro[cell], peak_v);
            if (f > 0.0) { add_trace(&px, cp.trace, f); }
        }
        case 1u: { // rgb waveform
            let cols = array<vec4<f32>, 3>(cp.red, cp.green, cp.blue);
            for (var c = 0u; c < 3u; c = c + 1u) {
                let f = intensity(counts_ro[c * GRID * GRID + cell], peak_v);
                if (f > 0.0) { add_trace(&px, cols[c], f); }
            }
        }
        case 2u: { // vectorscope
            let f = intensity(counts_ro[cell], peak_v);
            if (f > 0.0) { add_trace(&px, cp.trace, f); }
        }
        default: { // histogram: column height ∝ count, filled bottom-up
            let cols = array<vec4<f32>, 3>(cp.red, cp.green, cp.blue);
            for (var c = 0u; c < 3u; c = c + 1u) {
                let count = counts_ro[c * GRID + cx];
                let h = u32(floor(intensity(count, peak_v) * (f32(GRID) - 1.0) + 0.5));
                if (cy + h >= GRID - 1u) { // cy >= (GRID-1) - h
                    add_trace(&px, cols[c], 0.7);
                }
            }
        }
    }

    textureStore(
        dst,
        vec2<i32>(i32(cx), i32(cy)),
        vec4<f32>(f32(px.x) / 255.0, f32(px.y) / 255.0, f32(px.z) / 255.0, 1.0),
    );
}
