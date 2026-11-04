// Particulate worked out step by step on the card (fx/particulate_sim.rs),
// appended to fx_particulate.wgsl. Each step moves every particle, counts who
// lives and who is born, and places them all by a prefix sum, never atomics, so
// a particle's slot follows from the particles before it and not from which
// thread got there first. The state stays in one buffer between steps, in
// px@comp, oldest first.

struct Part {
    // Where it is, and how old.
    at: vec4<f32>,
    // How fast it is going, and how long it lasts. A particle held on a surface
    // keeps its place in that surface's pixels in `.xyz` instead.
    vel: vec4<f32>,
    id: vec2<u32>,
    // Bit 0: held, bit 1: dead, bit 2: hit something this step, bits 8 up: the
    // collider that holds it.
    flags: u32,
    // While it is being moved, how many sparks it throws.
    kids: u32,
};

struct State {
    count: u32,
    _a: u32,
    _b: u32,
    _c: u32,
    parts: array<Part>,
};

struct Step {
    h: f32,
    clock: f32,
    cap: u32,
    blocks: u32,

    // The steady births this step makes: how many, the first one's index, when
    // in the frame the first one happens and how far apart they are, and when
    // in the frame this step ends.
    steady_first: vec2<u32>,
    steady_n: u32,
    off0: f32,

    off_step: f32,
    end: f32,
    burst_n: u32,
    colliders: u32,

    burst_first: vec2<u32>,
    contact: u32,
    keep: f32,

    slip: f32,
    sparks: f32,
    spark_speed: f32,
    spark_life: f32,

    sparks_from: u32,
    axis: u32,
    attract: f32,
    vortex: f32,

    // The field's middle, and how far it reaches.
    field: vec4<f32>,

    sphere: f32,
    curl: f32,
    // How far off another particle still counts as a neighbour, nought for no
    // flock, and how strongly a particle steers each of the three ways.
    flock_radius: f32,
    apart: f32,

    along: f32,
    together: f32,
    // How far through the frame this step starts and ends, for what moves.
    pose0: f32,
    pose1: f32,

    // How much of its parent's speed a spark starts with.
    spark_inherit: f32,
    _d: f32,
    _e: f32,
    _f: f32,
};

const K_WALLS: u32 = 0u;
const K_RECT: u32 = 1u;
const K_SHAPES: u32 = 2u;
const K_FLOOR: u32 = 3u;
const K_MODEL: u32 = 4u;

struct Collider {
    // This layer's pixels to the collider's own, by rows, at the frame's end.
    // For a model, to the cells of its box of distances.
    from0: vec4<f32>,
    from1: vec4<f32>,
    from2: vec4<f32>,
    // And back.
    to0: vec4<f32>,
    to1: vec4<f32>,
    to2: vec4<f32>,
    // The same at the frame's start, so what moves sweeps through the frame.
    was_from0: vec4<f32>,
    was_from1: vec4<f32>,
    was_from2: vec4<f32>,
    was_to0: vec4<f32>,
    was_to1: vec4<f32>,
    was_to2: vec4<f32>,
    // Its box, in the pixels its edges are in.
    lo: vec2<f32>,
    hi: vec2<f32>,
    kind: u32,
    first: u32,
    n: u32,
    // Walls: how wide a cell of their grid is.
    cell: f32,
    // A model's box of distances: where it starts in `sdf`, and its cells. For
    // walls, where their grid's cell starts and items are in `edges`, and its
    // cells across and down.
    grid: vec4<u32>,
};

@group(3) @binding(0) var<uniform> st: Step;
@group(3) @binding(1) var<storage, read_write> sa: State;
@group(3) @binding(2) var<storage, read_write> sb: State;
// Who lives then how many sparks, a number a particle each, each turned into
// its place within its block, then each block's total for both, then the sums.
@group(3) @binding(3) var<storage, read_write> counts: array<u32>;
@group(3) @binding(4) var<storage, read> colliders: array<Collider>;
// Each collider's edges, four numbers each, then the walls' grids as whole
// numbers.
@group(3) @binding(5) var<storage, read> edges: array<u32>;
@group(3) @binding(6) var<storage, read> sdf: array<f32>;
// A flock's cells: how many particles are in each, and the sum of where in the
// cell they are and how fast they go, as whole numbers. Whole numbers add up the
// same in any order, which floats would not.
@group(3) @binding(7) var<storage, read_write> cells: array<atomic<i32>>;

const F_HELD: u32 = 1u;
const F_DEAD: u32 = 2u;
const F_HIT: u32 = 4u;
// A die of its own for a spark's fraction, clear of the others.
const A_SPARK: u32 = 18u;
const SPARK_HI: u32 = 0x80000000u;
const SIM_NUDGE: f32 = 1e-3;
const SIM_HITS: u32 = 4u;
const CTL_OLD: u32 = 0u;
const CTL_KEPT: u32 = 1u;
const CTL_KIDS: u32 = 2u;
const CTL_DROP: u32 = 3u;

fn c_kids() -> u32 { return st.cap; }
fn c_keep_sums() -> u32 { return 2u * st.cap; }
fn c_kid_sums() -> u32 { return 2u * st.cap + st.blocks; }
fn c_ctl() -> u32 { return 2u * st.cap + 2u * st.blocks; }

// The gradient of one channel of value noise at `q`.
fn sim_grad(ch: u32, q: vec3<f32>) -> vec3<f32> {
    let f = floor(q);
    let x = q - f;
    let i = vec3<i32>(f);
    let u = x * x * x * (x * (x * 6.0 - 15.0) + 10.0);
    let du = 30.0 * x * x * (x * (x - 2.0) + 1.0);
    let a = nc_hash01(p.seed, ch, i.x, i.y, i.z);
    let b = nc_hash01(p.seed, ch, i.x + 1, i.y, i.z);
    let c = nc_hash01(p.seed, ch, i.x, i.y + 1, i.z);
    let d = nc_hash01(p.seed, ch, i.x + 1, i.y + 1, i.z);
    let e = nc_hash01(p.seed, ch, i.x, i.y, i.z + 1);
    let g = nc_hash01(p.seed, ch, i.x + 1, i.y, i.z + 1);
    let h = nc_hash01(p.seed, ch, i.x, i.y + 1, i.z + 1);
    let k = nc_hash01(p.seed, ch, i.x + 1, i.y + 1, i.z + 1);
    let kx = b - a;
    let ky = c - a;
    let kz = e - a;
    let kxy = a - b - c + d;
    let kyz = a - c - e + h;
    let kzx = a - b - e + g;
    let kxyz = -a + b + c - d + e - g - h + k;
    return 2.0 * du * vec3<f32>(
        kx + kxy * u.y + kzx * u.z + kxyz * u.y * u.z,
        ky + kyz * u.z + kxy * u.x + kxyz * u.z * u.x,
        kz + kzx * u.x + kyz * u.y + kxyz * u.x * u.y,
    );
}

// The flow at `at`: the curl of a noise that drifts with the clock. It has no
// sources or sinks, so particles swirl and never clump where it points inwards.
fn sim_curl(at: vec3<f32>) -> vec3<f32> {
    let q = at / max(p.turb_scale, 1e-3);
    let s = st.clock * p.turb_speed;
    let a = sim_grad(TURB_X, q + vec3<f32>(s, 0.0, 31.7));
    let b = sim_grad(TURB_Y, q + vec3<f32>(11.3, s, 0.0));
    let c = sim_grad(TURB_Z, q + vec3<f32>(0.0, 47.1, s));
    return vec3<f32>(c.y - b.z, a.z - c.x, b.x - a.y);
}

const CELLS_SIDE: i32 = 64;
const CELL_WORDS: u32 = 7u;
// What one unit of a cell's sums stands for: of its side, and px@comp a second.
const CELL_PLACE: f32 = 1023.0;
const CELL_SPEED: f32 = 0.5;

fn sim_cell(c: vec3<i32>) -> u32 {
    let m = ((c % CELLS_SIDE) + CELLS_SIDE) % CELLS_SIDE;
    return (u32(m.x) + u32(m.y) * 64u + u32(m.z) * 4096u) * CELL_WORDS;
}

// Each free particle adds itself to the cell it is in.
@compute @workgroup_size(64)
fn sim_flock_add(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= sa.count) {
        return;
    }
    let q = sa.parts[i];
    if ((q.flags & F_HELD) != 0u) {
        return;
    }
    let g = q.at.xyz / st.flock_radius;
    let f = floor(g);
    let c = sim_cell(vec3<i32>(f));
    let place = vec3<i32>((g - f) * CELL_PLACE);
    let speed = vec3<i32>(clamp(q.vel.xyz * CELL_SPEED, vec3<f32>(-1023.0), vec3<f32>(1023.0)));
    atomicAdd(&cells[c], 1);
    atomicAdd(&cells[c + 1u], place.x);
    atomicAdd(&cells[c + 2u], place.y);
    atomicAdd(&cells[c + 3u], place.z);
    atomicAdd(&cells[c + 4u], speed.x);
    atomicAdd(&cells[c + 5u], speed.y);
    atomicAdd(&cells[c + 6u], speed.z);
}

// == the CPU's flock in stepped.rs, read from the cells around the particle
// rather than from each neighbour. A particle's own cell leaves it out.
fn sim_steer(at: vec3<f32>, vel: vec3<f32>) -> vec3<f32> {
    let r = st.flock_radius;
    let home = vec3<i32>(floor(at / r));
    var push = vec3<f32>(0.0, 0.0, 0.0);
    var speed = vec3<f32>(0.0, 0.0, 0.0);
    var mid = vec3<f32>(0.0, 0.0, 0.0);
    var total = 0.0;
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let c = home + vec3<i32>(x, y, z);
                let w = sim_cell(c);
                var n = f32(atomicLoad(&cells[w]));
                var place = vec3<f32>(
                    f32(atomicLoad(&cells[w + 1u])),
                    f32(atomicLoad(&cells[w + 2u])),
                    f32(atomicLoad(&cells[w + 3u])),
                );
                var moving = vec3<f32>(
                    f32(atomicLoad(&cells[w + 4u])),
                    f32(atomicLoad(&cells[w + 5u])),
                    f32(atomicLoad(&cells[w + 6u])),
                );
                if (x == 0 && y == 0 && z == 0) {
                    let g = at / r;
                    let own = vec3<f32>(vec3<i32>((g - floor(g)) * CELL_PLACE));
                    n = n - 1.0;
                    place = place - own;
                    moving = moving - vec3<f32>(vec3<i32>(clamp(vel * CELL_SPEED, vec3<f32>(-1023.0), vec3<f32>(1023.0))));
                }
                if (n <= 0.0) {
                    continue;
                }
                let middle = (vec3<f32>(c) + place / (n * CELL_PLACE)) * r;
                let d = at - middle;
                let len = length(d);
                if (len >= r * 1.5) {
                    continue;
                }
                if (len > 1e-3) {
                    push = push + d * n * max(1.0 - len / r, 0.0) / len;
                }
                speed = speed + moving / CELL_SPEED;
                mid = mid + middle * n;
                total = total + n;
            }
        }
    }
    if (total <= 0.0) {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    // Away from close ones, towards their speed and towards their middle, each
    // the mean, the pull to the middle gentle so a flock streams along.
    const STEER: f32 = 4.0;
    return st.apart * STEER * STEER * r / total * push
        + st.along * 2.0 * STEER * (speed / total - vel)
        + st.together * 0.1 * STEER * STEER * (mid / total - at);
}

// How the fields and the flow push a particle at `at` going at `vel`.
fn sim_push(at: vec3<f32>, vel: vec3<f32>) -> vec3<f32> {
    var a = vec3<f32>(0.0, 0.0, 0.0);
    if (p.turb != 0.0) {
        a = a + sim_curl(at) * st.curl;
    }
    let d = at - st.field.xyz;
    let r = length(d);
    if (r > 1e-3) {
        let out = d / r;
        let reach = r / st.field.w;
        let fade = 1.0 / (1.0 + reach * reach);
        a = a - out * st.attract * fade;
        if (st.sphere > 0.0 && reach < 1.0) {
            a = a + out * st.sphere * (1.0 - reach);
        }
        if (st.vortex != 0.0) {
            var axis = vec3<f32>(0.0, 1.0, 0.0);
            if (st.axis == 0u) { axis = vec3<f32>(1.0, 0.0, 0.0); }
            if (st.axis == 2u) { axis = vec3<f32>(0.0, 0.0, 1.0); }
            let side = cross(axis, d);
            let len = length(side);
            if (len > 1e-3) {
                let round = side / len;
                // Eased towards the sweep's speed, and held in on its circle.
                let along = dot(vel, round);
                let wants = st.vortex * fade;
                let inward = cross(round, axis);
                a = a + round * (wants - along) * 4.0 + inward * along * abs(along) / len;
            }
        }
    }
    return a;
}

// A model's distance at cell `g`, by its eight neighbours. Out of its box, far.
fn sim_distance(c: u32, g: vec3<f32>) -> f32 {
    let size = colliders[c].grid.yzw;
    let top = vec3<f32>(size) - vec3<f32>(1.0, 1.0, 1.0);
    if (any(g < vec3<f32>(0.0, 0.0, 0.0)) || any(g >= top)) {
        return 1e9;
    }
    let f = floor(g);
    let x = g - f;
    let i = vec3<u32>(f);
    let base = colliders[c].grid.x + i.x + i.y * size.x + i.z * size.x * size.y;
    let dy = size.x;
    let dz = size.x * size.y;
    let y0 = mix(mix(sdf[base], sdf[base + 1u], x.x), mix(sdf[base + dy], sdf[base + dy + 1u], x.x), x.y);
    let y1 = mix(mix(sdf[base + dz], sdf[base + dz + 1u], x.x), mix(sdf[base + dz + dy], sdf[base + dz + dy + 1u], x.x), x.y);
    return mix(y0, y1, x.z);
}

struct SimHit {
    // How far along the move, nought when it started inside, above one for no hit.
    s: f32,
    // The normal on the side it came from.
    n: vec3<f32>,
    which: u32,
    // Where it hit, on the collider where this step leaves it, and how fast the
    // collider is going there.
    at: vec3<f32>,
    moving: vec3<f32>,
    // How far out to go, when it started inside.
    out: f32,
};

fn sim_edge(e: u32) -> vec4<f32> {
    return vec4<f32>(
        bitcast<f32>(edges[e * 4u]),
        bitcast<f32>(edges[e * 4u + 1u]),
        bitcast<f32>(edges[e * 4u + 2u]),
        bitcast<f32>(edges[e * 4u + 3u]),
    );
}

fn sim_inside(c: u32, x: f32, y: f32) -> bool {
    if (x < colliders[c].lo.x || x > colliders[c].hi.x || y < colliders[c].lo.y || y > colliders[c].hi.y) {
        return false;
    }
    if (colliders[c].kind != K_SHAPES) {
        return true;
    }
    // Even-odd, as the CPU reads a surface's shapes.
    var odd = false;
    let first = colliders[c].first;
    for (var e = first; e < first + colliders[c].n; e = e + 1u) {
        let v = sim_edge(e);
        if ((v.y > y) != (v.w > y) && x < v.x + (y - v.y) / (v.w - v.y) * (v.z - v.x)) {
            odd = !odd;
        }
    }
    return odd;
}

// The collider's own place of a layer point, at `pose` through the frame.
fn sim_own(c: u32, pose: f32, q: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(mix(colliders[c].was_from0, colliders[c].from0, pose), q),
        dot(mix(colliders[c].was_from1, colliders[c].from1, pose), q),
        dot(mix(colliders[c].was_from2, colliders[c].from2, pose), q),
    );
}

// And a place on the collider back in this layer's pixels.
fn sim_back(c: u32, pose: f32, own: vec3<f32>) -> vec3<f32> {
    let q = vec4<f32>(own, 1.0);
    return vec3<f32>(
        dot(mix(colliders[c].was_to0, colliders[c].to0, pose), q),
        dot(mix(colliders[c].was_to1, colliders[c].to1, pose), q),
        dot(mix(colliders[c].was_to2, colliders[c].to2, pose), q),
    );
}

fn sim_moving(c: u32, own: vec3<f32>) -> vec3<f32> {
    if (st.h <= 0.0) {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    return (sim_back(c, st.pose1, own) - sim_back(c, st.pose0, own)) / st.h;
}

// The first thing a move meets from a surface: `pose_a` is where the collider
// was as the move began, which a later bounce of one step takes from its end.
fn sim_model(c: u32, pose_a: f32, a1: vec4<f32>, b1: vec4<f32>, best: ptr<function, SimHit>) {
    let ga = sim_own(c, pose_a, a1);
    let gb = sim_own(c, st.pose1, b1);
    let seg = gb - ga;
    let size = vec3<f32>(colliders[c].grid.yzw) - vec3<f32>(1.0, 1.0, 1.0);
    // Only the part of the move inside its box.
    var t0 = 0.0;
    var t1 = 1.0;
    for (var k = 0u; k < 3u; k = k + 1u) {
        if (abs(seg[k]) < 1e-9) {
            if (ga[k] < 0.0 || ga[k] >= size[k]) {
                return;
            }
        } else {
            let p = (0.0 - ga[k]) / seg[k];
            let q = (size[k] - ga[k]) / seg[k];
            t0 = max(t0, min(p, q));
            t1 = min(t1, max(p, q));
        }
    }
    if (t0 > t1) {
        return;
    }
    // Each step as long as the distance says is safe, so a thin wall stops a
    // fast particle.
    let len = max(length(seg), 1e-6);
    var t = t0;
    var d = 1.0;
    for (var k = 0u; k < 8u; k = k + 1u) {
        d = sim_distance(c, ga + seg * t);
        if (d <= 0.0 || t >= t1) {
            break;
        }
        t = min(t + max(d, 0.5) / len, t1);
    }
    if (d > 0.0 || t >= (*best).s) {
        return;
    }
    let at = ga + seg * t;
    let e = 0.5;
    let g = vec3<f32>(
        sim_distance(c, at + vec3<f32>(e, 0.0, 0.0)) - sim_distance(c, at - vec3<f32>(e, 0.0, 0.0)),
        sim_distance(c, at + vec3<f32>(0.0, e, 0.0)) - sim_distance(c, at - vec3<f32>(0.0, e, 0.0)),
        sim_distance(c, at + vec3<f32>(0.0, 0.0, e)) - sim_distance(c, at - vec3<f32>(0.0, 0.0, e)),
    );
    // A gradient in cells comes back through the transpose.
    let n = colliders[c].from0.xyz * g.x + colliders[c].from1.xyz * g.y + colliders[c].from2.xyz * g.z;
    let n_len = length(n);
    if (n_len <= 1e-9 || n_len >= 1e6) {
        return;
    }
    (*best).s = t;
    (*best).n = n / n_len;
    (*best).which = c;
    (*best).at = sim_back(c, st.pose1, at);
    (*best).moving = sim_moving(c, at);
    // Started inside: out by as far as it had gone in.
    (*best).out = select(0.0, -d * length(colliders[c].to0.xyz), t <= t0 && t0 == 0.0);
}

// The first thing the move from `a` to `b` meets.
fn sim_first(a: vec3<f32>, b: vec3<f32>, pose_a: f32) -> SimHit {
    var best: SimHit;
    best.s = 2.0;
    best.n = vec3<f32>(0.0, 0.0, 0.0);
    best.which = 0u;
    best.at = b;
    best.moving = vec3<f32>(0.0, 0.0, 0.0);
    best.out = 0.0;
    let a1 = vec4<f32>(a, 1.0);
    let b1 = vec4<f32>(b, 1.0);
    for (var c = 0u; c < st.colliders; c = c + 1u) {
        let kind = colliders[c].kind;
        if (kind == K_MODEL) {
            sim_model(c, pose_a, a1, b1, &best);
            continue;
        }
        if (kind == K_WALLS) {
            // Walls are across and down only, and found through their grid.
            let lo = min(a.xy, b.xy);
            let hi = max(a.xy, b.xy);
            if (hi.x < colliders[c].lo.x || lo.x > colliders[c].hi.x || hi.y < colliders[c].lo.y || lo.y > colliders[c].hi.y) {
                continue;
            }
            let r = b.xy - a.xy;
            let g = colliders[c].grid;
            let last = vec2<i32>(i32(g.z) - 1, i32(g.w) - 1);
            let from_cell = clamp(vec2<i32>(floor((lo - colliders[c].lo) / colliders[c].cell)), vec2<i32>(0, 0), last);
            let to_cell = clamp(vec2<i32>(floor((hi - colliders[c].lo) / colliders[c].cell)), vec2<i32>(0, 0), last);
            for (var cy = from_cell.y; cy <= to_cell.y; cy = cy + 1) {
                for (var cx = from_cell.x; cx <= to_cell.x; cx = cx + 1) {
                    let cell = u32(cy) * g.z + u32(cx);
                    let end = edges[g.x + cell + 1u];
                    for (var it = edges[g.x + cell]; it < end; it = it + 1u) {
                        let v = sim_edge(colliders[c].first + edges[g.y + it]);
                        let w = v.zw - v.xy;
                        let den = r.x * w.y - r.y * w.x;
                        if (abs(den) < 1e-12) {
                            continue;
                        }
                        let qp = v.xy - a.xy;
                        let s = (qp.x * w.y - qp.y * w.x) / den;
                        let u = (qp.x * r.y - qp.y * r.x) / den;
                        if (s >= 0.0 && s <= 1.0 && u >= 0.0 && u <= 1.0 && s < best.s) {
                            var n = normalize(vec2<f32>(-w.y, w.x));
                            if (dot(a.xy - v.xy, n) < 0.0) {
                                n = -n;
                            }
                            best.s = s;
                            best.n = vec3<f32>(n, 0.0);
                            best.which = c;
                            best.at = a + (b - a) * s;
                            best.moving = vec3<f32>(0.0, 0.0, 0.0);
                            best.out = 0.0;
                        }
                    }
                }
            }
            continue;
        }
        // A surface: its card is where its own z is nought. The move starts
        // where the card was and ends where the card is, so it can't step past
        // one rising through it.
        let pz = dot(mix(colliders[c].was_from2, colliders[c].from2, pose_a), a1);
        let qz = dot(mix(colliders[c].was_from2, colliders[c].from2, st.pose1), b1);
        if (!((pz > 0.0 && qz <= 0.0) || (pz < 0.0 && qz >= 0.0))) {
            continue;
        }
        let s = pz / (pz - qz);
        if (s >= best.s) {
            continue;
        }
        let own = mix(sim_own(c, pose_a, a1), sim_own(c, st.pose1, b1), s);
        if (kind != K_FLOOR && !sim_inside(c, own.x, own.y)) {
            continue;
        }
        let z = mix(colliders[c].was_from2, colliders[c].from2, st.pose1).xyz;
        let on = vec3<f32>(own.xy, 0.0);
        best.s = s;
        best.n = z / (length(z) * sign(pz));
        best.which = c;
        best.at = sim_back(c, st.pose1, on);
        best.moving = sim_moving(c, on);
        best.out = 0.0;
    }
    return best;
}

@compute @workgroup_size(64)
fn sim_move(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= sa.count) {
        return;
    }
    var q = sa.parts[i];
    q.kids = 0u;
    let age = q.at.w + st.h;
    let life = q.vel.w;
    let spark = (q.id.y & SPARK_HI) != 0u;
    var hits = 0u;
    if (age >= life) {
        q.flags = q.flags | F_DEAD;
    } else if ((q.flags & F_HELD) != 0u) {
        // A particle that stuck goes where the surface goes.
        let c = q.flags >> 8u;
        if (c < st.colliders) {
            q.at = vec4<f32>(sim_back(c, st.pose1, q.vel.xyz), age);
        } else {
            q.at.w = age;
        }
    } else {
        var v0 = q.vel.xyz + sim_push(q.at.xyz, q.vel.xyz) * st.h;
        if (st.flock_radius > 0.0) {
            // A flock turns a particle and leaves its speed as it was.
            let was = length(v0);
            let turned = v0 + sim_steer(q.at.xyz, q.vel.xyz) * st.h;
            let now = length(turned);
            if (now > 1e-6) {
                v0 = turned * (was / now);
            }
        }
        let m = pt_integrate(q.at.xyz, v0, st.h);
        var a = q.at.xyz;
        var b = m.pos;
        var vel = m.vel;
        if (st.colliders > 0u) {
            for (var k = 0u; k < SIM_HITS; k = k + 1u) {
                // After a bounce the move goes on from the end of the step.
                let hit = sim_first(a, b, select(st.pose1, st.pose0, k == 0u));
                if (hit.s > 1.0) {
                    break;
                }
                hits = hits + 1u;
                let n = hit.n;
                let at = hit.at + n * max(SIM_NUDGE, hit.out);
                // Everything is worked out as the collider sees it, so a card
                // coming up throws what it meets.
                let meets = vel - hit.moving;
                let vn = dot(meets, n);
                if (st.contact == 3u) {
                    q.flags = q.flags | F_DEAD;
                    // Its sparks leave the way it would have bounced.
                    vel = hit.moving + meets - n * vn * 2.0;
                    b = at;
                    break;
                }
                if (st.contact == 2u) {
                    // Its place in the collider's own pixels stays with it.
                    let v = vec4<f32>(at, 1.0);
                    q.flags = (q.flags & 0xffu) | F_HELD | (hit.which << 8u);
                    vel = sim_own(hit.which, st.pose1, v);
                    b = at;
                    break;
                }
                var keep = st.keep;
                if (st.contact == 1u) {
                    keep = 0.0;
                }
                // Along the surface it keeps all but the friction, and off it
                // the bounciness, turned round.
                vel = hit.moving + (meets - n * vn) * st.slip - n * vn * keep;
                let left = (1.0 - hit.s) * st.h;
                let rest = b - (a + (b - a) * hit.s) - hit.moving * left;
                let rn = dot(rest, n);
                let turned = (rest - n * rn) * st.slip - n * rn * keep + hit.moving * left;
                a = at;
                // Still hitting things: it stops where it last came off.
                b = select(at + turned, at, k + 1u == SIM_HITS);
            }
        }
        if (p.edge == EDGE_KILL) {
            let e = pt_bounds();
            if (b.x < e.x || b.x > e.z || b.y < e.y || b.y > e.w) {
                q.flags = q.flags | F_DEAD;
            }
        } else if (p.edge == EDGE_WRAP) {
            b = pt_wrapped(pt_bounds(), b);
        }
        q.at = vec4<f32>(b, age);
        q.vel = vec4<f32>(vel, life);
    }
    let dead = (q.flags & F_DEAD) != 0u;
    if (hits > 0u) {
        q.flags = q.flags | F_HIT;
    }
    // Sparks throw out none. A fraction of a spark is settled by a die, named by
    // the moment, so each death and each hit has its own.
    if (!spark && st.sparks > 0.0) {
        let some = u32(st.sparks + pt_die(q.id.x, q.id.y ^ u32(age * 1024.0), A_SPARK));
        if (st.sparks_from == 0u && dead) {
            q.kids = some;
        } else if (st.sparks_from == 1u && hits > 0u) {
            q.kids = some;
        } else if (st.sparks_from == 2u && !dead) {
            q.kids = u32(floor(age * st.sparks) - floor((age - st.h) * st.sparks));
        }
    }
    sb.parts[i] = q;
    counts[i] = select(1u, 0u, dead);
    counts[c_kids() + i] = q.kids;
}

// Each block of 256 on one thread: its numbers become their places within it.
@compute @workgroup_size(1)
fn sim_scan(@builtin(workgroup_id) wid: vec3<u32>) {
    let n = sa.count;
    var keep = 0u;
    var kids = 0u;
    for (var j = 0u; j < SCAN_BLOCK; j = j + 1u) {
        let i = wid.x * SCAN_BLOCK + j;
        if (i >= n) {
            break;
        }
        let a = counts[i];
        counts[i] = keep;
        keep = keep + a;
        let b = counts[c_kids() + i];
        counts[c_kids() + i] = kids;
        kids = kids + b;
    }
    counts[c_keep_sums() + wid.x] = keep;
    counts[c_kid_sums() + wid.x] = kids;
}

// The block totals become running sums, and the whole step's numbers are set.
@compute @workgroup_size(1)
fn sim_sums() {
    var keep = 0u;
    var kids = 0u;
    for (var b = 0u; b < st.blocks; b = b + 1u) {
        let k = counts[c_keep_sums() + b];
        counts[c_keep_sums() + b] = keep;
        keep = keep + k;
        let s = counts[c_kid_sums() + b];
        counts[c_kid_sums() + b] = kids;
        kids = kids + s;
    }
    let total = keep + st.steady_n + st.burst_n + kids;
    counts[c_ctl() + CTL_OLD] = sa.count;
    counts[c_ctl() + CTL_KEPT] = keep;
    counts[c_ctl() + CTL_KIDS] = kids;
    // Over budget the newest survive, the first in line going first.
    counts[c_ctl() + CTL_DROP] = total - min(total, st.cap);
    sa.count = min(total, st.cap);
}

fn sim_spark(q: Part, j: u32) -> Part {
    var s: Part;
    let burst = q.id.y & BURST_HI;
    s.id = vec2<u32>(q.id.x, SPARK_HI | burst | ((j & 0x3ffu) << 20u) | (q.id.y & 0xfffffu));
    // Evenly over a sphere, or off the side it came from when it hit something.
    let z = 2.0 * pt_die(s.id.x, s.id.y, A_DIRECTION_Z) - 1.0;
    let a = pt_die(s.id.x, s.id.y, A_DIRECTION) * 6.28318530717958647692;
    let r = sqrt(max(1.0 - z * z, 0.0));
    var way = vec3<f32>(r * cos(a), r * sin(a), z);
    let held = (q.flags & F_HELD) != 0u;
    var parent = q.vel.xyz;
    if (held) {
        parent = vec3<f32>(0.0, 0.0, 0.0);
    }
    if ((q.flags & F_HIT) != 0u && dot(way, parent) < 0.0) {
        way = -way;
    }
    s.at = vec4<f32>(q.at.xyz, 0.0);
    s.vel = vec4<f32>(way * st.spark_speed + parent * st.spark_inherit, st.spark_life);
    s.flags = 0u;
    s.kids = 0u;
    return s;
}

// Everyone who lived in line, then the births, then the sparks.
@compute @workgroup_size(64)
fn sim_place(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= counts[c_ctl() + CTL_OLD]) {
        return;
    }
    let drop = counts[c_ctl() + CTL_DROP];
    let q = sb.parts[i];
    if ((q.flags & F_DEAD) == 0u) {
        let slot = counts[i] + counts[c_keep_sums() + i / SCAN_BLOCK];
        if (slot >= drop) {
            var kept = q;
            kept.flags = kept.flags & ~F_HIT;
            sa.parts[slot - drop] = kept;
        }
    }
    if (q.kids > 0u) {
        let start = counts[c_ctl() + CTL_KEPT] + st.steady_n + st.burst_n
            + counts[c_kids() + i] + counts[c_kid_sums() + i / SCAN_BLOCK];
        // The numbering runs on through its life, so a second hit doesn't throw
        // the first one's sparks again.
        let base = u32(max(q.at.w * 1024.0, 0.0));
        for (var k = 0u; k < q.kids; k = k + 1u) {
            let slot = start + k;
            if (slot >= drop) {
                sa.parts[slot - drop] = sim_spark(q, base + k);
            }
        }
    }
}

@compute @workgroup_size(64)
fn sim_birth(@builtin(global_invocation_id) gid: vec3<u32>) {
    let j = gid.x;
    if (j >= st.steady_n + st.burst_n) {
        return;
    }
    let slot = counts[c_ctl() + CTL_KEPT] + j;
    let drop = counts[c_ctl() + CTL_DROP];
    if (slot < drop) {
        return;
    }
    var base = st.steady_first;
    var at = j;
    var age = 0.0;
    if (j >= st.steady_n) {
        base = st.burst_first;
        at = j - st.steady_n;
    } else {
        age = max(st.end - (st.off0 + f32(j) * st.off_step), 0.0);
    }
    let lo = base.x + at;
    var hi = base.y;
    if (lo < base.x) {
        hi = hi + 1u;
    }
    let life = pt_jitter(p.life, p.life_jitter, pt_die(lo, hi, A_LIFE));
    let l = pt_launch(lo, hi);
    var q: Part;
    q.id = vec2<u32>(lo, hi);
    q.flags = 0u;
    q.kids = 0u;
    let m = pt_integrate(l.p0, l.v0, age);
    q.at = vec4<f32>(m.pos, age);
    q.vel = vec4<f32>(m.vel, life);
    // Nowhere to be born: it goes on the next step, and draws nothing.
    if (!l.ok) {
        q.at.w = life;
    }
    sa.parts[slot - drop] = q;
}

// ---------------------------------------------------------------- the stream

struct SimOut {
    // Raster pixels per px@comp, and a spark's size as a share of its parent's.
    px: f32,
    spark_size: f32,
    _a: f32,
    _b: f32,
};

@group(3) @binding(10) var<storage, read> out_state: State;
@group(3) @binding(11) var<uniform> so: SimOut;

// The state as the closed forms' stream, in this raster, so every draw that
// reads the stream reads this one too.
@compute @workgroup_size(64)
fn sim_out(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let n = min(out_state.count, p.cap);
    if (i == 0u) {
        args[0] = 6u;
        args[1] = n;
        args[2] = 0u;
        args[3] = 0u;
    }
    if (i >= n) {
        return;
    }
    let q = out_state.parts[i];
    let age = q.at.w;
    let life = q.vel.w;
    var speed = q.vel.xyz * so.px;
    if ((q.flags & F_HELD) != 0u) {
        speed = vec3<f32>(0.0, 0.0, 0.0);
    }
    let head = q.at.xyz * so.px + speed * p.lead;
    var tail = head;
    if (p.tail > 0.0) {
        tail = head - speed * p.tail;
    }
    // A spark looks like the particle it came from, and smaller.
    var look = q.id;
    var scale = 1.0;
    if ((q.id.y & SPARK_HI) != 0u) {
        look = vec2<u32>(q.id.x, (q.id.y & BURST_HI) | (q.id.y & 0xfffffu));
        scale = so.spark_size;
    }
    // Born with nowhere to be, or out of time: nothing to draw.
    if (age >= life) {
        scale = 0.0;
    }
    pt_write(i, q.id, look, age, life, speed, head, tail, scale);
}
