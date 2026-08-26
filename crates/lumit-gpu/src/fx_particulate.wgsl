// Particulate (docs/08 §3.86, K-446, K-474, K-475) — the WGSL twin of
// `lumit_core::fx::points`, op for op.
//
// This file is prepended with `fx_noise_core.wgsl`, whose `nc_hash01` and
// `nc_value3` are the very lattice the CPU reference draws its dice and its
// turbulence from. There is one noise family in this engine and this kernel
// does not get a second one.
//
// **Four passes, and why there are four.**
//
//  1. `pt_alive`   — one thread per *candidate* (a birth this frame could still
//                    see). It answers one question, "is this one alive?", which
//                    costs one hash, and writes 1 or 0.
//  2. `pt_scan`    — a workgroup-local exclusive prefix sum over those flags,
//                    256 at a time, leaving each block's own total behind.
//  3. `pt_blocks`  — the block totals scanned in turn, and the indirect draw
//                    arguments written from the grand total.
//  4. `pt_scatter` — one thread per candidate again; the live ones evaluate the
//                    closed forms in full and write themselves to the slot the
//                    prefix sum gave them.
//
// A **prefix sum and never atomics** (particulate.md §5): a compacted slot has
// to be a function of the birth index, not of which workgroup reached the
// counter first, or `id` order — the thing that makes trails possible — would
// be a scheduling artefact and two renders of one frame could disagree.
//
// **The stream is one buffer of attribute-major regions.** particulate.md §4
// names eight arrays; the default WebGPU limit is eight storage buffers for a
// whole stage, and this pass needs five inputs of its own beside them. So the
// eight arrays are suballocated from one allocation at the offsets below — the
// same structure-of-arrays, reached by adding a constant instead of by binding
// a buffer. A consumer reads region by region exactly as it would read eight
// bindings.

struct Params {
    // Emitter.
    em_pos: vec2<f32>,
    em_wh: vec2<f32>,
    em_angle: f32,
    dir: f32,
    spread: f32,
    speed: f32,

    speed_jitter: f32,
    shape: u32,
    seed: u32,
    cap: u32,

    // Particle.
    life: f32,
    life_jitter: f32,
    size: f32,
    size_jitter: f32,

    rotation: f32,
    rotation_jitter: f32,
    spin: f32,
    align: u32,

    colour: vec4<f32>,
    end_colour: vec4<f32>,

    // Forces.
    wind: vec2<f32>,
    gravity: f32,
    drag: f32,

    turb: f32,
    turb_scale: f32,
    turb_speed: f32,
    eps: f32,

    // Schedule and the frame.
    dt: f32,
    first_birth_lo: u32,
    first_birth_hi: u32,
    frames: u32,

    candidates: u32,
    path_len: u32,
    path_total: f32,
    tail: f32,

    // Draw.
    feather: f32,
    mix: f32,
    mode: u32,
    target_w: f32,

    target_h: f32,
    sprite_w: f32,
    sprite_h: f32,
    wind_z: f32,

    // The third axis (K-561). Every one of these is nought on a 2D layer.
    em_z: f32,
    em_depth: f32,
    dir_z: f32,
    spread_z: f32,

    // The composition camera, restricted back onto the layer's own plane:
    // `(m0·v, m1·v, m2·v)` for `v = (x, y, z, 1)` puts the particle at
    // `xy / w` and scales it by `1 / w`. Only read when `project` is set.
    proj0: vec4<f32>,
    proj1: vec4<f32>,
    proj2: vec4<f32>,

    project: u32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

// The mode codes, matching `lumit_core::fx::points::RenderMode::from_code`. An
// unset Sprite arrives as DISC: the host resolves the fallback, so the kernel
// has one less branch and the two paths cannot disagree about when it fires.
const MODE_DISC: u32 = 0u;
const MODE_SPRITE: u32 = 1u;
const MODE_STREAK: u32 = 2u;

// The per-particle dice, matching `points::attr`.
const A_EMIT_U: u32 = 0u;
const A_EMIT_V: u32 = 1u;
const A_DIRECTION: u32 = 2u;
const A_SPEED: u32 = 3u;
const A_LIFE: u32 = 4u;
const A_SIZE: u32 = 5u;
const A_TURB_PHASE: u32 = 6u;
const A_ROTATION: u32 = 7u;
const A_EMIT_W: u32 = 8u;
const A_DIRECTION_Z: u32 = 9u;

// The turbulence lattices, matching `points::TURB_CHANNEL_X/Y/Z`.
const TURB_X: u32 = 64u;
const TURB_Y: u32 = 65u;
const TURB_Z: u32 = 66u;

const CURVE_TABLE: u32 = 257u;
const SCAN_BLOCK: u32 = 256u;

@group(0) @binding(0) var<uniform> p: Params;
// One entry per recorded frame, plus a closing one: `.x` is the birth offset
// the frame starts at, `.y` is `t − that frame's start` in seconds, computed in
// f64 host-side so the ages this kernel works out never lose their small
// difference inside a large clock.
@group(0) @binding(1) var<storage, read> frames_buf: array<vec2<u32>>;
// The two baked over-life curves, size then opacity, 257 entries each.
@group(0) @binding(2) var<storage, read> curves: array<f32>;
// The mask path: `(x, y, arc length, unused)` per vertex (K-408).
@group(0) @binding(3) var<storage, read> path: array<vec4<f32>>;
// Aliveness on the way in, the exclusive prefix sum on the way out — one
// buffer, because a rank is what a flag becomes.
@group(0) @binding(4) var<storage, read_write> ranks: array<u32>;
// Each scan block's own total, then its running offset; the last slot holds the
// grand total.
@group(0) @binding(5) var<storage, read_write> block_sums: array<u32>;
// The compacted stream (see the header).
@group(0) @binding(6) var<storage, read_write> stream: array<u32>;
// `draw_indirect`'s four words, so the draw never waits on a read-back.
@group(0) @binding(7) var<storage, read_write> args: array<u32>;

// The stream's regions, in words, for a capacity of `c` particles. Position,
// speed and the draw's tail are three components each since K-561.
fn r_pos(c: u32) -> u32 { return 0u; }
fn r_speed(c: u32) -> u32 { return 3u * c; }
fn r_age(c: u32) -> u32 { return 6u * c; }
fn r_life(c: u32) -> u32 { return 7u * c; }
fn r_size(c: u32) -> u32 { return 8u * c; }
fn r_rot(c: u32) -> u32 { return 9u * c; }
fn r_colour(c: u32) -> u32 { return 10u * c; }
fn r_id(c: u32) -> u32 { return 12u * c; }
fn r_tail(c: u32) -> u32 { return 14u * c; }

// == lumit_core::fx::points::Projection::apply, given the three rows.
//
// `.xy` is where the camera puts the particle on the layer's plane and `.z` is
// the foreshortening. With `on` clear — a 2D layer — it hands back the same
// bits it was given and a scale of exactly one, which is the K-258 guarantee:
// the flat path is not an approximation of itself.
fn pt_project(m0: vec4<f32>, m1: vec4<f32>, m2: vec4<f32>, on: u32, q: vec3<f32>) -> vec3<f32> {
    if (on == 0u) {
        return vec3<f32>(q.x, q.y, 1.0);
    }
    let v = vec4<f32>(q, 1.0);
    let w = dot(m2, v);
    // At or behind the camera's own plane: scale nought, which draws nothing
    // at all rather than flinging the particle across the frame.
    if (w <= 1e-4) {
        return vec3<f32>(q.x, q.y, 0.0);
    }
    let inv = 1.0 / w;
    return vec3<f32>(dot(m0, v) * inv, dot(m1, v) * inv, inv);
}

// == lumit_core::fx::points::draw.
fn pt_die(birth_lo: u32, birth_hi: u32, attr_id: u32) -> f32 {
    return nc_hash01(p.seed, attr_id, bitcast<i32>(birth_lo), bitcast<i32>(birth_hi), 0);
}

// == lumit_core::fx::points::jitter.
fn pt_jitter(base: f32, amount: f32, u: f32) -> f32 {
    return max(base * (1.0 + amount * (2.0 * u - 1.0)), 0.0);
}

// == lumit_core::fx::cpu::curve_at, on one of the two baked tables.
fn pt_curve(x: f32, base: u32) -> f32 {
    let last = f32(CURVE_TABLE - 1u);
    let s = x * last;
    let fi = floor(clamp(s, 0.0, last - 1.0));
    let i = min(u32(max(fi, 0.0)), CURVE_TABLE - 2u);
    let f = s - fi;
    let a = curves[base + i];
    return a + (curves[base + i + 1u] - a) * f;
}

// == lumit_core::fx::points::drag_terms. The guard sits at 0.1 with one more
// series term, not at particulate.md's 1e−4: in f32 `1 − e^(−x)` has lost three
// of its seven digits by there, and the two branches would part company by
// parts in a thousand — visible in a trajectory and far past the ≤ 2 ULP this
// kernel owes the CPU (particulate.md §5). Neither branch divides by the drag.
fn pt_drag_terms(x: f32) -> vec2<f32> {
    if (x < 0.1) {
        let r = 1.0 - x * 0.5 + x * x / 6.0 - x * x * x / 24.0;
        let s = 0.5 - x / 6.0 + x * x / 24.0 - x * x * x / 120.0;
        return vec2<f32>(r, s);
    }
    let r = (1.0 - exp(-x)) / x;
    return vec2<f32>(r, (1.0 - r) / x);
}

struct Motion {
    pos: vec3<f32>,
    vel: vec3<f32>,
};

// == lumit_core::fx::points::integrate. Three axes, one algebra: the depth
// component takes the same drag and wind terms. Gravity stays down (K-561).
fn pt_integrate(p0: vec3<f32>, v0: vec3<f32>, age: f32) -> Motion {
    let k = max(p.drag, 0.0);
    let x = k * age;
    let rs = pt_drag_terms(x);
    let decay = exp(-x);
    let g = vec3<f32>(0.0, p.gravity, 0.0);
    let wind = vec3<f32>(p.wind.x, p.wind.y, p.wind_z);
    var m: Motion;
    m.pos = p0 + wind * age + (v0 - wind) * age * rs.x + g * age * age * rs.y;
    m.vel = wind + (v0 - wind) * decay + g * age * rs.x;
    return m;
}

// == lumit_core::fx::points::turbulence. The lattice is sampled at the birth
// point's own x and y as it always was; the third axis is a third channel of
// the same lattice, not a third input coordinate.
fn pt_turbulence(p0: vec3<f32>, phase: f32, age: f32) -> vec3<f32> {
    if (p.turb == 0.0) {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    let scale = max(p.turb_scale, 1e-3);
    let q = p0.xy / scale + vec2<f32>(phase, phase);
    let z = age * p.turb_speed;
    return vec3<f32>(
        p.turb * nc_value3(p.seed, TURB_X, q.x, q.y, z, 0),
        p.turb * nc_value3(p.seed, TURB_Y, q.x, q.y, z, 0),
        p.turb * nc_value3(p.seed, TURB_Z, q.x, q.y, z, 0),
    );
}

// == lumit_core::mask::MaskPolyline::point_at, over the uploaded polyline.
fn pt_path_at(s_in: f32) -> vec2<f32> {
    let n = p.path_len;
    if (n < 2u) {
        return vec2<f32>(0.0, 0.0);
    }
    let s = clamp(s_in, 0.0, p.path_total);
    // The last index whose arc is <= s, by binary search — the same answer
    // `partition_point` gives, and the same reason: a walk would make placing
    // n particles quadratic in the polyline.
    var lo = 0u;
    var hi = n;
    loop {
        if (lo >= hi) { break; }
        let mid = (lo + hi) / 2u;
        if (path[mid].z <= s) { lo = mid + 1u; } else { hi = mid; }
    }
    var i = 0u;
    if (lo > 0u) { i = lo - 1u; }
    i = min(i, n - 2u);
    let a = path[i];
    let b = path[i + 1u];
    let span = b.z - a.z;
    var t = 0.0;
    if (span > 0.0) { t = clamp((s - a.z) / span, 0.0, 1.0); }
    return a.xy + (b.xy - a.xy) * t;
}

// == lumit_core::fx::points::birth_point. `.w` of the result is 0 when the
// emitter has nothing to emit from — a Mask path row that came to no mask,
// which is the documented no-op; `.xyz` is the birth point's three axes.
//
// `w_die` is the draw through the emitter's Depth (K-561), filled uniformly: a
// Point becomes a segment, an Ellipse a cylinder, a Rectangle a box. Line and
// Mask path ignore it and stay on the plane.
fn pt_birth_point(u: f32, v: f32, w_die: f32) -> vec4<f32> {
    if (p.shape == 4u) {
        if (p.path_len < 2u) {
            return vec4<f32>(0.0, 0.0, 0.0, 0.0);
        }
        // Already an absolute position: the path is where the user drew it, so
        // the emitter's own place and turn do not move it.
        return vec4<f32>(pt_path_at(u * p.path_total), p.em_z, 1.0);
    }
    let a = radians(p.em_angle);
    let s = sin(a);
    let c = cos(a);
    var local = vec2<f32>(0.0, 0.0);
    var depth = (w_die - 0.5) * p.em_depth;
    if (p.shape >= 5u) {
        // The two outline shapes (K-597). The host flattened the emitter's own
        // outline into the very buffer a mask path uses, in the emitter's local
        // frame, so the walk is the same walk and the turn and the placement
        // below are the ones the filled shape already gets.
        local = pt_path_at(u * p.path_total);
    } else if (p.shape == 1u) {
        local = vec2<f32>((u - 0.5) * p.em_wh.x, 0.0);
        depth = 0.0;
    } else if (p.shape == 2u) {
        // √u for the radius, or the middle of the disc would be crowded.
        let r = sqrt(max(u, 0.0));
        let ang = v * 6.28318530717958647692;
        local = vec2<f32>(0.5 * p.em_wh.x * r * cos(ang), 0.5 * p.em_wh.y * r * sin(ang));
    } else if (p.shape == 3u) {
        local = vec2<f32>((u - 0.5) * p.em_wh.x, (v - 0.5) * p.em_wh.y);
    }
    return vec4<f32>(
        p.em_pos.x + local.x * c - local.y * s,
        p.em_pos.y + local.x * s + local.y * c,
        p.em_z + depth,
        1.0,
    );
}

struct Candidate {
    birth_lo: u32,
    birth_hi: u32,
    age: f32,
    life: f32,
};

// Which birth this candidate is, and how old it is now.
//
// Candidate `c` is birth `first_birth + c`; which frame owed it — and so when
// it was born inside that frame — is a search over the running birth counts.
// Handing the counts over rather than a birth time per candidate is what keeps
// the per-frame upload the size of the window instead of the size of the
// particle set.
fn pt_candidate(c: u32) -> Candidate {
    var out: Candidate;
    out.birth_lo = p.first_birth_lo + c;
    out.birth_hi = p.first_birth_hi;
    if (out.birth_lo < p.first_birth_lo) {
        out.birth_hi = out.birth_hi + 1u;
    }
    out.age = -1.0;
    out.life = 0.0;
    // The last frame whose start is <= c.
    var lo = 0u;
    var hi = p.frames;
    loop {
        if (lo >= hi) { break; }
        let mid = (lo + hi) / 2u;
        if (frames_buf[mid].x <= c) { lo = mid + 1u; } else { hi = mid; }
    }
    if (lo == 0u) {
        return out;
    }
    let i = lo - 1u;
    let start = frames_buf[i].x;
    let n = frames_buf[i + 1u].x - start;
    if (n == 0u) {
        return out;
    }
    let j = c - start;
    // `t − frame start`, from the host's own f64 arithmetic, minus where in the
    // frame this birth fell: births are spread evenly inside the frame that
    // owed them, so a rate of one a frame does not stack them on the boundary.
    let frame_rel = bitcast<f32>(frames_buf[i].y);
    out.age = frame_rel - (f32(j) + 0.5) * p.dt / f32(n);
    out.life = pt_jitter(p.life, p.life_jitter, pt_die(out.birth_lo, out.birth_hi, A_LIFE));
    return out;
}

fn pt_is_alive(c: Candidate) -> bool {
    return c.age >= 0.0 && c.life > 0.0 && c.age < c.life;
}

// ---------------------------------------------------------------- pass 1

@compute @workgroup_size(64)
fn pt_alive(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = gid.x;
    if (c >= p.candidates) {
        return;
    }
    let cand = pt_candidate(c);
    var live = 0u;
    if (pt_is_alive(cand)) {
        // A Mask path emitter with nothing to walk emits nothing at all.
        if (p.shape != 4u || p.path_len >= 2u) {
            live = 1u;
        }
    }
    ranks[c] = live;
}

// ---------------------------------------------------------------- pass 2

var<workgroup> scan_tile: array<u32, SCAN_BLOCK>;

@compute @workgroup_size(SCAN_BLOCK)
fn pt_scan(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(workgroup_id) wid: vec3<u32>,
) {
    let c = gid.x;
    var v = 0u;
    if (c < p.candidates) {
        v = ranks[c];
    }
    scan_tile[lid.x] = v;
    workgroupBarrier();
    // Hillis–Steele inclusive scan: fixed trip count, no data-dependent
    // branching, so every lane walks the same steps in the same order.
    for (var off = 1u; off < SCAN_BLOCK; off = off * 2u) {
        var add = 0u;
        if (lid.x >= off) {
            add = scan_tile[lid.x - off];
        }
        workgroupBarrier();
        scan_tile[lid.x] = scan_tile[lid.x] + add;
        workgroupBarrier();
    }
    let inclusive = scan_tile[lid.x];
    if (c < p.candidates) {
        ranks[c] = inclusive - v;
    }
    if (lid.x == SCAN_BLOCK - 1u) {
        block_sums[wid.x] = inclusive;
    }
}

// ---------------------------------------------------------------- pass 3

@compute @workgroup_size(1)
fn pt_blocks() {
    let blocks = (p.candidates + SCAN_BLOCK - 1u) / SCAN_BLOCK;
    var total = 0u;
    // ponytail: one lane walks the block totals. There is one entry per 256
    // candidates — four thousand of them at the million-particle hard cap, which
    // is microseconds — so a second level of parallel scan would be machinery
    // bought against nothing.
    for (var b = 0u; b < blocks; b = b + 1u) {
        let n = block_sums[b];
        block_sums[b] = total;
        total = total + n;
    }
    block_sums[blocks] = total;
    // The instanced draw's arguments: six vertices, one instance per drawn
    // particle. Written here so the draw never waits on a read-back, which is
    // what would turn one frame's work into two.
    args[0] = 6u;
    args[1] = min(total, p.cap);
    args[2] = 0u;
    args[3] = 0u;
}

// ---------------------------------------------------------------- pass 4

@compute @workgroup_size(64)
fn pt_scatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = gid.x;
    if (c >= p.candidates) {
        return;
    }
    let cand = pt_candidate(c);
    if (!pt_is_alive(cand)) {
        return;
    }
    let blocks = (p.candidates + SCAN_BLOCK - 1u) / SCAN_BLOCK;
    let rank = ranks[c] + block_sums[c / SCAN_BLOCK];
    let total = block_sums[blocks];
    // **The cap rule** (K-474, K-475): over budget the newest `cap` by birth
    // index survive, and the same rule again at half the cap is the degradation
    // rung. Old particles vanishing early under overload is visible,
    // deterministic and the same from any scrub direction.
    if (total > p.cap && rank < total - p.cap) {
        return;
    }
    var dst = rank;
    if (total > p.cap) {
        dst = rank - (total - p.cap);
    }

    let lo = cand.birth_lo;
    let hi = cand.birth_hi;
    let age = cand.age;
    let life = cand.life;
    let b0 = pt_birth_point(
        pt_die(lo, hi, A_EMIT_U),
        pt_die(lo, hi, A_EMIT_V),
        pt_die(lo, hi, A_EMIT_W),
    );
    if (b0.w == 0.0) {
        return;
    }
    let p0 = b0.xyz;
    let dir = radians(p.dir) + (pt_die(lo, hi, A_DIRECTION) - 0.5) * radians(p.spread);
    let dir_z = radians(p.dir_z) + (pt_die(lo, hi, A_DIRECTION_Z) - 0.5) * radians(p.spread_z);
    let speed = pt_jitter(p.speed, p.speed_jitter, pt_die(lo, hi, A_SPEED));
    // The elevation tilts the launch out of the plane; at nought its cosine is
    // exactly one and the two in-plane components are the bits they always were.
    let cz = cos(dir_z);
    let v0 = vec3<f32>(speed * cos(dir) * cz, speed * sin(dir) * cz, speed * sin(dir_z));
    let m = pt_integrate(p0, v0, age);
    let phase = pt_die(lo, hi, A_TURB_PHASE) * 1000.0;
    let d = pt_turbulence(p0, phase, age);
    let d_next = pt_turbulence(p0, phase, age + p.eps);
    let d_prev = pt_turbulence(p0, phase, age - p.eps);
    let u = clamp(age / life, 0.0, 1.0);
    let speed_out = m.vel + (d_next - d_prev) / (2.0 * p.eps);
    let size = pt_jitter(p.size, p.size_jitter, pt_die(lo, hi, A_SIZE)) * pt_curve(u, 0u);
    let alpha = p.colour.a * pt_curve(u, CURVE_TABLE);
    // Premultiplied, and the blend to End colour is in working space.
    let tint = p.colour.rgb + (p.end_colour.rgb - p.colour.rgb) * u;
    let colour = vec4<f32>(tint * alpha, alpha);

    var rot = radians(p.rotation);
    if (p.align != 0u) {
        // In the layer's plane: a rotation is one angle in the picture the
        // sprite is stamped into, which has no depth axis to turn about.
        rot = atan2(speed_out.y, speed_out.x) + rot;
    }
    // The per-particle rotation spread (K-507), from the seed hash like every
    // other die, so two sprites born together do not point the same way.
    rot = rot + (pt_die(lo, hi, A_ROTATION) - 0.5) * radians(p.rotation_jitter)
        + radians(p.spin) * age;

    let head = m.pos + d;
    var tail = head;
    if (p.tail > 0.0) {
        // Where it was a Streak length ago — the same closed form at an earlier
        // age, never a remembered position.
        let back = max(age - p.tail, 0.0);
        let mb = pt_integrate(p0, v0, back);
        tail = mb.pos + pt_turbulence(p0, phase, back);
    }

    let cap = p.cap;
    stream[r_pos(cap) + dst * 3u] = bitcast<u32>(head.x);
    stream[r_pos(cap) + dst * 3u + 1u] = bitcast<u32>(head.y);
    stream[r_pos(cap) + dst * 3u + 2u] = bitcast<u32>(head.z);
    stream[r_speed(cap) + dst * 3u] = bitcast<u32>(speed_out.x);
    stream[r_speed(cap) + dst * 3u + 1u] = bitcast<u32>(speed_out.y);
    stream[r_speed(cap) + dst * 3u + 2u] = bitcast<u32>(speed_out.z);
    stream[r_age(cap) + dst] = bitcast<u32>(age);
    stream[r_life(cap) + dst] = bitcast<u32>(life);
    stream[r_size(cap) + dst] = bitcast<u32>(size);
    stream[r_rot(cap) + dst] = bitcast<u32>(rot);
    // Half precision, as particulate.md §4 declares the colour region.
    stream[r_colour(cap) + dst * 2u] = pack2x16float(colour.rg);
    stream[r_colour(cap) + dst * 2u + 1u] = pack2x16float(colour.ba);
    stream[r_id(cap) + dst * 2u] = lo;
    stream[r_id(cap) + dst * 2u + 1u] = hi;
    stream[r_tail(cap) + dst * 3u] = bitcast<u32>(tail.x);
    stream[r_tail(cap) + dst * 3u + 1u] = bitcast<u32>(tail.y);
    stream[r_tail(cap) + dst * 3u + 2u] = bitcast<u32>(tail.z);
}

// ---------------------------------------------------------------- the draw

@group(1) @binding(0) var<uniform> dp: Params;
@group(1) @binding(1) var<storage, read> dstream: array<u32>;
@group(1) @binding(2) var sprite: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) head: vec2<f32>,
    @location(1) @interpolate(flat) tail: vec2<f32>,
    @location(2) @interpolate(flat) colour: vec4<f32>,
    @location(3) @interpolate(flat) geom: vec4<f32>, // radius, edge, size, rotation
};

// One instanced quad per live particle: a capsule's bounding box for a disc or
// a streak, the sprite's own turned square for a sprite. Six vertices, two
// triangles, no vertex buffer — everything is read out of the stream by index,
// which is what makes the draw one call whatever the particle count.
@vertex
fn pt_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let cap = dp.cap;
    let head3 = vec3<f32>(
        bitcast<f32>(dstream[r_pos(cap) + ii * 3u]),
        bitcast<f32>(dstream[r_pos(cap) + ii * 3u + 1u]),
        bitcast<f32>(dstream[r_pos(cap) + ii * 3u + 2u]),
    );
    let tail3 = vec3<f32>(
        bitcast<f32>(dstream[r_tail(cap) + ii * 3u]),
        bitcast<f32>(dstream[r_tail(cap) + ii * 3u + 1u]),
        bitcast<f32>(dstream[r_tail(cap) + ii * 3u + 2u]),
    );
    // Through the composition's camera (K-561): where it is seen, and how much
    // it foreshortens. The tail takes the same camera, so a streak that runs
    // towards the lens really does run towards the lens. Flat on a 2D layer, in
    // which case these are the same bits the stream holds.
    let ph = pt_project(dp.proj0, dp.proj1, dp.proj2, dp.project, head3);
    let pt = pt_project(dp.proj0, dp.proj1, dp.proj2, dp.project, tail3);
    let head = ph.xy;
    let tail = pt.xy;
    let size = bitcast<f32>(dstream[r_size(cap) + ii]) * ph.z;
    let rot = bitcast<f32>(dstream[r_rot(cap) + ii]);
    let rg = unpack2x16float(dstream[r_colour(cap) + ii * 2u]);
    let ba = unpack2x16float(dstream[r_colour(cap) + ii * 2u + 1u]);
    let radius = size * 0.5;
    let edge = max(dp.feather * radius, 0.5);

    // The two triangles' corners in the quad's own frame, −1..1 each way.
    var corner = vec2<f32>(-1.0, -1.0);
    if (vi == 1u || vi == 4u) { corner = vec2<f32>(1.0, -1.0); }
    if (vi == 2u || vi == 3u) { corner = vec2<f32>(-1.0, 1.0); }
    if (vi == 5u) { corner = vec2<f32>(1.0, 1.0); }

    var centre = head;
    var ax = vec2<f32>(1.0, 0.0);
    var half_long = radius;
    var half_wide = radius;
    if (dp.mode == MODE_SPRITE) {
        // A square of the particle's own size, turned by its rotation; the
        // corners reach √2 of the half-side, and the fragment discards what
        // falls outside the sprite's own 0..1 square.
        ax = vec2<f32>(cos(rot), sin(rot));
        half_long = radius;
        half_wide = radius;
    } else {
        let seg = head - tail;
        let len = length(seg);
        if (len > 1e-6) {
            ax = seg / len;
        }
        centre = (head + tail) * 0.5;
        half_long = len * 0.5 + radius;
    }
    let ay = vec2<f32>(-ax.y, ax.x);
    // A pixel of slack all round: coverage is measured at pixel centres, and a
    // quad that stops exactly on the edge would clip the ramp that softens it.
    let px = centre + ax * (corner.x * (half_long + 1.0)) + ay * (corner.y * (half_wide + 1.0));

    var out: VsOut;
    out.clip = vec4<f32>(
        px.x / dp.target_w * 2.0 - 1.0,
        1.0 - px.y / dp.target_h * 2.0,
        0.0,
        1.0,
    );
    out.head = head;
    out.tail = tail;
    // The host Mix, folded into the source's coverage. For a premultiplied
    // `over` that is the dissolve exactly, so no second pass runs (K-425).
    out.colour = vec4<f32>(rg.x, rg.y, ba.x, ba.y) * dp.mix;
    out.geom = vec4<f32>(radius, edge, size, rot);
    return out;
}

// == lumit_core::fx::points::seg_distance.
fn pt_seg_distance(q: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let e = b - a;
    let len2 = dot(e, e);
    var t = 0.0;
    if (len2 > 0.0) {
        t = clamp(dot(q - a, e) / len2, 0.0, 1.0);
    }
    return length(q - a - e * t);
}

// == lumit_core::fx::points::sprite_tap. Four loads and three lerps rather than
// a sampler call: a hardware sampler's filtering precision is the driver's
// business, and this is compared against the CPU's own arithmetic.
fn pt_sprite_tap(u: f32, v: f32) -> vec4<f32> {
    let last = vec2<f32>(max(dp.sprite_w - 1.0, 0.0), max(dp.sprite_h - 1.0, 0.0));
    let xy = clamp(vec2<f32>(u * dp.sprite_w - 0.5, v * dp.sprite_h - 0.5), vec2<f32>(0.0), last);
    let base = floor(xy);
    let f = xy - base;
    let i0 = vec2<i32>(base);
    let i1 = vec2<i32>(min(base + vec2<f32>(1.0), last));
    let a = textureLoad(sprite, vec2<i32>(i0.x, i0.y), 0);
    let b = textureLoad(sprite, vec2<i32>(i1.x, i0.y), 0);
    let c = textureLoad(sprite, vec2<i32>(i0.x, i1.y), 0);
    let d = textureLoad(sprite, vec2<i32>(i1.x, i1.y), 0);
    let top = a + (b - a) * f.x;
    let bot = c + (d - c) * f.x;
    return top + (bot - top) * f.y;
}

@fragment
fn pt_fs(v: VsOut) -> @location(0) vec4<f32> {
    // `@builtin(position)` with no multisampling is the pixel's own centre,
    // which is exactly where the CPU reference measures coverage.
    let q = v.clip.xy;
    let radius = v.geom.x;
    if (dp.mode == MODE_SPRITE) {
        let size = v.geom.z;
        let rot = v.geom.w;
        let dxy = q - v.head;
        let s = sin(rot);
        let c = cos(rot);
        let local = vec2<f32>(dxy.x * c + dxy.y * s, -dxy.x * s + dxy.y * c);
        let uv = local / size + vec2<f32>(0.5, 0.5);
        // Outside the sprite's own square contributes nothing — and nothing,
        // under a premultiplied `over`, is a transparent black fragment. No
        // `discard`: the blend already leaves the picture untouched, and a
        // discarded fragment and a zero one are the same pixel.
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            return vec4<f32>(0.0, 0.0, 0.0, 0.0);
        }
        // Both are premultiplied, so the tint is the plain product.
        return pt_sprite_tap(uv.x, uv.y) * v.colour;
    }
    let cov = clamp((radius - pt_seg_distance(q, v.tail, v.head)) / v.geom.y, 0.0, 1.0);
    return v.colour * cov;
}
