// Grid smoke and fire (fx/smoke.rs). One cell holds a speed in cells a second
// and fields: density, heat, fuel, and how fast it is burning, all as half
// floats in four words. A step is a run of whole-grid passes, each reading one
// buffer and writing another, and a fixed number of them, so it comes out the
// same however it is run.
//
// Buffers rather than 3D textures, as one graphics API only writes the first
// slice of a 3D texture through this version of wgpu.
//
// Between steps the speed's fourth value keeps the pressure, which starts the
// next solve near its answer.

struct Step {
    // Cells each way, then how long the step is, seconds.
    cells: vec4<f32>,
    // The wind in cells a second, then how fast speed settles towards it.
    wind: vec4<f32>,
    // Density a second, heat and fuel where it emits, then what it emits from:
    // 0 the box, 1 particles, 2 a model.
    emit: vec4<f32>,
    // The box emitter's middle in cells, then the clock, seconds.
    emit_middle: vec4<f32>,
    // The box emitter's reach in cells, then the shove gas leaves any emitter
    // with. smoke_drawn reads the fourth as the pitch of its rows instead.
    emit_reach: vec4<f32>,
    // Push per unit of heat, pull per unit of density, curl's push, and how
    // fast density goes.
    motion: vec4<f32>,
    // How fast fuel burns, the heat and the density a unit gives, then how fast
    // heat goes.
    burn: vec4<f32>,
    // How hot fuel must be to catch, how much burning swells the gas, and the
    // skin a model emits from, in its own cells.
    fire: vec4<f32>,
    // Cells of this grid to cells of the model's distances, by rows.
    to_model: array<vec4<f32>, 3>,
    // The model's distances' cells each way, then where they start.
    model_cells: vec4<f32>,
    // The push that breaks gas up, how many cells across its patches are, how
    // many solids there are, and how long a frame is, seconds.
    extra: vec4<f32>,
    // How much faster the hottest gas cools, and how broken up fuel arrives.
    more: vec4<f32>,
    // With Trail on, how fast the box moves through the gas, cells a second.
    drift: vec4<f32>,
};

// A layer or a model the gas goes round.
struct Solid {
    // Cells of this grid to the solid's own space, by rows.
    from_cell: array<vec4<f32>, 3>,
    // Cells of this grid to where that part of the solid was when the frame
    // started, in cells of this grid, by rows.
    was: array<vec4<f32>, 3>,
    // A card's box in its own space.
    lo: vec4<f32>,
    hi: vec4<f32>,
    // A model's distances: where they start, then its cells each way.
    grid: vec4<u32>,
    // 0 a card, 1 a model.
    kind: vec4<u32>,
};

@group(0) @binding(0) var<storage, read> first: array<vec4<u32>>;
@group(0) @binding(1) var<storage, read> second: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read_write> out: array<vec4<u32>>;
@group(0) @binding(3) var<storage, read> pressure: array<f32>;
@group(0) @binding(4) var<storage, read> spread: array<f32>;
@group(0) @binding(5) var<storage, read_write> one_out: array<f32>;
@group(0) @binding(6) var<storage, read_write> two_out: array<f32>;
@group(0) @binding(7) var<storage, read> distances: array<f32>;
@group(0) @binding(8) var<storage, read> emitted: array<u32>;
// smoke_drawn's: density and heat for the marcher, a row every `pitch` cells.
@group(0) @binding(9) var<storage, read_write> drawn: array<u32>;
// Each cell: the speed of the solid in it, then whether there is one.
@group(0) @binding(10) var<storage, read> solid: array<vec2<u32>>;
@group(0) @binding(11) var<storage, read_write> solid_out: array<vec2<u32>>;
@group(0) @binding(12) var<storage, read> solids: array<Solid>;

@group(1) @binding(0) var<uniform> s: Step;

fn size() -> vec3<i32> {
    return vec3<i32>(s.cells.xyz);
}

fn edge(c: vec3<i32>) -> vec3<i32> {
    return clamp(c, vec3<i32>(0), size() - vec3<i32>(1));
}

fn index(c: vec3<i32>) -> u32 {
    let n = size();
    return u32(c.x + c.y * n.x + c.z * n.x * n.y);
}

// A cell's speed from its words.
fn speed_of(w: vec4<u32>) -> vec4<f32> {
    return vec4<f32>(unpack2x16float(w.x), unpack2x16float(w.y));
}

fn fields_of(w: vec4<u32>) -> vec4<f32> {
    return vec4<f32>(unpack2x16float(w.z), unpack2x16float(w.w));
}

fn words(u: vec4<f32>, f: vec4<f32>) -> vec4<u32> {
    return vec4<u32>(pack2x16float(u.xy), pack2x16float(u.zw), pack2x16float(f.xy),
        pack2x16float(f.zw));
}

// Speed and fields at a point in cells, between the eight cells round it, from
// the first buffer.
struct Cell {
    speed: vec4<f32>,
    fields: vec4<f32>,
};

fn first_at(q: vec3<f32>) -> Cell {
    let m = q - vec3<f32>(0.5);
    let low = vec3<i32>(floor(m));
    let f = m - floor(m);
    var cell = Cell(vec4<f32>(0.0), vec4<f32>(0.0));
    for (var k: i32 = 0; k < 8; k = k + 1) {
        let o = vec3<i32>(k & 1, (k >> 1u) & 1, (k >> 2u) & 1);
        let w = mix(vec3<f32>(1.0) - f, f, vec3<f32>(o));
        let share = w.x * w.y * w.z;
        let word = first[index(edge(low + o))];
        cell.speed = cell.speed + speed_of(word) * share;
        cell.fields = cell.fields + fields_of(word) * share;
    }
    return cell;
}

// The same from the second.
fn second_at(q: vec3<f32>) -> Cell {
    let m = q - vec3<f32>(0.5);
    let low = vec3<i32>(floor(m));
    let f = m - floor(m);
    var cell = Cell(vec4<f32>(0.0), vec4<f32>(0.0));
    for (var k: i32 = 0; k < 8; k = k + 1) {
        let o = vec3<i32>(k & 1, (k >> 1u) & 1, (k >> 2u) & 1);
        let w = mix(vec3<f32>(1.0) - f, f, vec3<f32>(o));
        let share = w.x * w.y * w.z;
        let word = second[index(edge(low + o))];
        cell.speed = cell.speed + speed_of(word) * share;
        cell.fields = cell.fields + fields_of(word) * share;
    }
    return cell;
}

fn solid_here(i: u32) -> bool {
    return s.extra.z > 0.5 && unpack2x16float(solid[i].y).y > 0.5;
}

// Which cells a solid is in this frame, and how fast it is going there.
@compute @workgroup_size(4, 4, 4)
fn smoke_solids(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = vec3<i32>(gid);
    if (any(c >= size())) {
        return;
    }
    let v = vec4<f32>(vec3<f32>(c) + vec3<f32>(0.5), 1.0);
    var speed = vec3<f32>(0.0);
    var inside = false;
    let count = u32(s.extra.z);
    for (var i: u32 = 0u; i < count && !inside; i = i + 1u) {
        let m = vec3<f32>(dot(solids[i].from_cell[0], v), dot(solids[i].from_cell[1], v),
            dot(solids[i].from_cell[2], v));
        if (solids[i].kind.x == 1u) {
            let n = solids[i].grid.yzw;
            if (all(m >= vec3<f32>(0.0)) && all(m < vec3<f32>(n))) {
                let at = vec3<u32>(m);
                let k = solids[i].grid.x + at.x + at.y * n.x + at.z * n.x * n.y;
                inside = distances[k] < 0.0;
            }
        } else {
            inside = all(m >= solids[i].lo.xyz) && all(m <= solids[i].hi.xyz);
        }
        if (inside) {
            let w = vec3<f32>(dot(solids[i].was[0], v), dot(solids[i].was[1], v),
                dot(solids[i].was[2], v));
            speed = clamp((v.xyz - w) / max(s.extra.w, 1e-4), vec3<f32>(-4000.0), vec3<f32>(4000.0));
        }
    }
    solid_out[index(c)] = vec2<u32>(pack2x16float(speed.xy),
        pack2x16float(vec2<f32>(speed.z, select(0.0, 1.0, inside))));
}

fn first_speed(c: vec3<i32>) -> vec4<f32> {
    return speed_of(first[index(edge(c))]);
}

// The swirl of the speed, then how strong it is.
@compute @workgroup_size(4, 4, 4)
fn smoke_curl(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = vec3<i32>(gid);
    if (any(c >= size())) {
        return;
    }
    let dx = (first_speed(c + vec3<i32>(1, 0, 0)).xyz - first_speed(c - vec3<i32>(1, 0, 0)).xyz) * 0.5;
    let dy = (first_speed(c + vec3<i32>(0, 1, 0)).xyz - first_speed(c - vec3<i32>(0, 1, 0)).xyz) * 0.5;
    let dz = (first_speed(c + vec3<i32>(0, 0, 1)).xyz - first_speed(c - vec3<i32>(0, 0, 1)).xyz) * 0.5;
    let w = vec3<f32>(dy.z - dz.y, dz.x - dx.z, dx.y - dy.x);
    out[index(c)] = words(vec4<f32>(w, length(w)), vec4<f32>(0.0));
}

// Where this cell emits and how much, as a share.
fn emission(c: vec3<i32>) -> f32 {
    let kind = u32(s.emit.w + 0.5);
    if (kind == 3u) {
        return 0.0;
    }
    if (kind == 1u) {
        return f32(emitted[index(c)]) / 256.0;
    }
    let q = vec3<f32>(c) + vec3<f32>(0.5);
    if (kind == 2u) {
        let v = vec4<f32>(q, 1.0);
        let m = vec3<f32>(dot(s.to_model[0], v), dot(s.to_model[1], v), dot(s.to_model[2], v));
        if (any(m < vec3<f32>(0.0)) || any(m >= s.model_cells.xyz)) {
            return 0.0;
        }
        let at = vec3<u32>(m);
        let n = vec3<u32>(s.model_cells.xyz);
        let d = distances[u32(s.model_cells.w) + at.x + at.y * n.x + at.z * n.x * n.y];
        return clamp(1.0 - abs(d) / s.fire.z, 0.0, 1.0);
    }
    let r = length((q - s.emit_middle.xyz) / max(s.emit_reach.xyz, vec3<f32>(1e-3)));
    if (r >= 1.0) {
        return 0.0;
    }
    // Soft at its rim, and uneven in a way that moves, so it licks.
    return clamp((1.0 - r) * 3.0, 0.0, 1.0);
}

// Everything that adds to the cell: what it emits, what burns, the push of heat,
// the curl and the wind.
@compute @workgroup_size(4, 4, 4)
fn smoke_force(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = vec3<i32>(gid);
    let n = size();
    if (any(c >= n)) {
        return;
    }
    let dt = s.cells.w;
    let word = first[index(c)];
    var u = speed_of(word);
    var f = fields_of(word);

    if (s.motion.z > 0.0) {
        let swirl = vec3<f32>(
            speed_of(second[index(edge(c + vec3<i32>(1, 0, 0)))]).w
                - speed_of(second[index(edge(c - vec3<i32>(1, 0, 0)))]).w,
            speed_of(second[index(edge(c + vec3<i32>(0, 1, 0)))]).w
                - speed_of(second[index(edge(c - vec3<i32>(0, 1, 0)))]).w,
            speed_of(second[index(edge(c + vec3<i32>(0, 0, 1)))]).w
                - speed_of(second[index(edge(c - vec3<i32>(0, 0, 1)))]).w,
        );
        let g = length(swirl);
        if (g > 1e-4) {
            let w = speed_of(second[index(c)]).xyz;
            u = vec4<f32>(u.xyz + cross(swirl / g, w) * s.motion.z * dt, u.w);
        }
    }

    // Patches of push that shift with place and time, only where there is gas.
    if (s.extra.x > 0.0 && (f.r > 1e-3 || f.g > 1e-3)) {
        let k = 1.0 / max(s.extra.y, 1.0);
        let q = vec3<f32>(c) * k;
        let t = s.emit_middle.w * 1.7;
        let cycle = i32(s.fire.w);
        let shove = vec3<f32>(
            nc_perlin3(1u, 0u, q.x, q.y + t, q.z, cycle),
            nc_perlin3(1u, 1u, q.x + t, q.y, q.z, cycle),
            nc_perlin3(1u, 2u, q.x, q.y, q.z + t, cycle),
        );
        u = vec4<f32>(u.xyz + shove * s.extra.x * dt, u.w);
    }

    let share = emission(c);
    if (share > 0.0) {
        let held = min(share, 1.0);
        // Fuel arrives through a moving pattern, in tongues, never as a sheet.
        let p = (vec3<f32>(c) + vec3<f32>(0.5)) * 12.0 / max(s.cells.x, 1.0);
        let t2 = s.emit_middle.w * 2.2;
        // Scrolled on a slant: along one axis it crosses the noise's zero lattice
        // regularly, which reads as ribs up the flame.
        let pattern = nc_perlin3(2u, 0u, p.x + t2 * 0.7, p.y + t2 * 1.1, p.z - t2 * 0.9, i32(s.fire.w))
            + 0.5 * nc_perlin3(2u, 1u, p.x * 2.1 - t2 * 1.3, p.y * 2.1 + t2 * 1.7, p.z * 2.1 + t2 * 1.1, i32(s.fire.w));
        let gate = mix(1.0, smoothstep(-0.1, 0.3, pattern), s.more.y);
        f.r = f.r + s.emit.x * share * gate * dt;
        f.g = max(f.g, s.emit.y * held * gate);
        f.b = max(f.b, s.emit.z * held * gate);
        // A kick that shifts with place and time, so the gas leaves unevenly.
        let scale = 24.0 / max(s.cells.x, 1.0);
        let q = vec3<f32>(c) * scale;
        let t = s.emit_middle.w * 2.5;
        let cycle = i32(s.fire.w);
        let kick = vec3<f32>(
            nc_perlin3(0u, 0u, q.x, q.y + t, q.z, cycle),
            nc_perlin3(0u, 1u, q.x + t, q.y, q.z, cycle),
            nc_perlin3(0u, 2u, q.x, q.y, q.z + t, cycle),
        );
        u = vec4<f32>(u.xyz + kick * s.emit_reach.w * held * dt, u.w);
    }

    var burnt: f32 = 0.0;
    if (f.g >= s.fire.x) {
        // Hotter burns faster, so flame is a thin hot sheet that runs to its end.
        burnt = min(f.b, s.burn.x * (0.2 + f.g * f.g) * dt);
    }
    f.b = f.b - burnt;
    f.g = min(f.g + burnt * s.burn.y, 1.6);
    f.r = f.r + burnt * s.burn.z;
    f.a = burnt / dt;
    // The hotter it is the faster it cools, so flames stay low and warm smoke
    // goes on rising.
    f.g = f.g * exp(-(s.burn.w + s.more.x * f.g * f.g * f.g) * dt);
    f.r = f.r * exp(-s.motion.w * dt);

    // Up is the way the comp's y falls.
    let push = s.motion.x * f.g - s.motion.y * f.r;
    u.y = u.y - push * dt;
    let settle = 1.0 - exp(-s.wind.w * dt);
    u = vec4<f32>(u.xyz + (s.wind.xyz - u.xyz) * settle, u.w);
    // Half floats run out, and a cell moving this fast has left the box anyway.
    u = vec4<f32>(clamp(u.xyz, vec3<f32>(-4000.0), vec3<f32>(4000.0)), u.w);
    f.r = min(f.r, 64.0);

    // The box's floor is solid. Smoke at its other faces fades, so the box
    // never shows as a cut.
    if (c.y >= n.y - 1) {
        u.y = min(u.y, 0.0);
    }
    if (solid_here(index(c))) {
        let w = solid[index(c)];
        u = vec4<f32>(vec3<f32>(unpack2x16float(w.x), unpack2x16float(w.y).x), u.w);
        f = vec4<f32>(0.0);
    }
    let room = min(vec3<f32>(c) + vec3<f32>(0.5), vec3<f32>(n - c) - vec3<f32>(0.5)) / s.cells.xyz;
    let side = 1.0 - smoothstep(0.0, 0.1, min(room.x, room.z));
    let top = 1.0 - smoothstep(0.0, 0.35, (f32(c.y) + 0.5) / s.cells.y);
    let edge_near = max(side, top);
    let gone = exp(-16.0 * edge_near * edge_near * dt);
    f.r = f.r * gone;
    f.g = f.g * gone;
    if (f.r < 1e-4) {
        f.r = 0.0;
    }
    out[index(c)] = words(u, f);
}

// How much each cell's gas spreads, and the pressure the last step ended on to
// start from. Burning swells the gas, which is what makes a fireball.
@compute @workgroup_size(4, 4, 4)
fn smoke_spread(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = vec3<i32>(gid);
    if (any(c >= size())) {
        return;
    }
    let x = first_speed(c + vec3<i32>(1, 0, 0)).x - first_speed(c - vec3<i32>(1, 0, 0)).x;
    let y = first_speed(c + vec3<i32>(0, 1, 0)).y - first_speed(c - vec3<i32>(0, 1, 0)).y;
    let z = first_speed(c + vec3<i32>(0, 0, 1)).z - first_speed(c - vec3<i32>(0, 0, 1)).z;
    let word = first[index(c)];
    one_out[index(c)] = (x + y + z) * 0.5 - fields_of(word).a * s.fire.y;
    two_out[index(c)] = speed_of(word).w;
}

// The pressure at a cell past the edge: nothing out in the open, the same as
// next door at the solid floor.
fn pressure_at(c: vec3<i32>, from_cell: f32) -> f32 {
    let n = size();
    if (c.y >= n.y) {
        return from_cell;
    }
    if (any(c < vec3<i32>(0)) || c.x >= n.x || c.z >= n.z) {
        return 0.0;
    }
    let i = index(c);
    if (solid_here(i)) {
        return from_cell;
    }
    return pressure[i];
}

@compute @workgroup_size(4, 4, 4)
fn smoke_solve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = vec3<i32>(gid);
    if (any(c >= size())) {
        return;
    }
    let i = index(c);
    let p0 = pressure[i];
    let sum = pressure_at(c + vec3<i32>(1, 0, 0), p0) + pressure_at(c - vec3<i32>(1, 0, 0), p0)
        + pressure_at(c + vec3<i32>(0, 1, 0), p0) + pressure_at(c - vec3<i32>(0, 1, 0), p0)
        + pressure_at(c + vec3<i32>(0, 0, 1), p0) + pressure_at(c - vec3<i32>(0, 0, 1), p0);
    one_out[i] = (sum - spread[i]) / 6.0;
}

// The gas made not to spread: the pressure's slope taken off its speed.
@compute @workgroup_size(4, 4, 4)
fn smoke_settle(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = vec3<i32>(gid);
    if (any(c >= size())) {
        return;
    }
    let i = index(c);
    let p0 = pressure[i];
    let slope = vec3<f32>(
        pressure_at(c + vec3<i32>(1, 0, 0), p0) - pressure_at(c - vec3<i32>(1, 0, 0), p0),
        pressure_at(c + vec3<i32>(0, 1, 0), p0) - pressure_at(c - vec3<i32>(0, 1, 0), p0),
        pressure_at(c + vec3<i32>(0, 0, 1), p0) - pressure_at(c - vec3<i32>(0, 0, 1), p0),
    ) * 0.5;
    let word = first[i];
    if (solid_here(i)) {
        out[i] = word;
        return;
    }
    out[i] = words(vec4<f32>(speed_of(word).xyz - slope, p0), fields_of(word));
}

// Everything carried along by the speed, taking each cell from where it came.
@compute @workgroup_size(4, 4, 4)
fn smoke_carry(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = vec3<i32>(gid);
    if (any(c >= size())) {
        return;
    }
    let u = speed_of(first[index(c)]);
    let from_at = first_at(vec3<f32>(c) + vec3<f32>(0.5) - (u.xyz + s.drift.xyz) * s.cells.w);
    out[index(c)] = words(from_at.speed, from_at.fields);
}

// Carrying blurs. This carries the result back again, and what didn't come back
// to where it started is the blur, which it puts right. Kept to what was in the
// cells it came from, so it can't ring.
@compute @workgroup_size(4, 4, 4)
fn smoke_sharpen(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = vec3<i32>(gid);
    if (any(c >= size())) {
        return;
    }
    let i = index(c);
    let was = first[i];
    let u0 = speed_of(was);
    let f0 = fields_of(was);
    let on = second[i];
    let here = vec3<f32>(c) + vec3<f32>(0.5);
    let moved = (u0.xyz + s.drift.xyz) * s.cells.w;
    let back = second_at(here + moved);
    var u2 = speed_of(on) + (u0 - back.speed) * 0.25;
    var f2 = fields_of(on) + (f0 - back.fields) * 0.25;

    let source = here - moved - vec3<f32>(0.5);
    let low = vec3<i32>(floor(source));
    var u_lo = vec4<f32>(1e30);
    var u_hi = vec4<f32>(-1e30);
    var f_lo = vec4<f32>(1e30);
    var f_hi = vec4<f32>(-1e30);
    for (var k: i32 = 0; k < 8; k = k + 1) {
        let word = first[index(edge(low + vec3<i32>(k & 1, (k >> 1u) & 1, (k >> 2u) & 1)))];
        let ua = speed_of(word);
        let fa = fields_of(word);
        u_lo = min(u_lo, ua);
        u_hi = max(u_hi, ua);
        f_lo = min(f_lo, fa);
        f_hi = max(f_hi, fa);
    }
    if (solid_here(i)) {
        out[i] = words(u0, vec4<f32>(0.0));
        return;
    }
    out[i] = words(clamp(u2, u_lo, u_hi), clamp(f2, f_lo, f_hi));
}

// Density and heat laid out for a 3D texture to be copied from.
@compute @workgroup_size(4, 4, 4)
fn smoke_drawn(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = vec3<i32>(gid);
    let n = size();
    if (any(c >= n)) {
        return;
    }
    let pitch = i32(s.emit_reach.w);
    let f = fields_of(first[index(c)]);
    drawn[u32(c.x + c.y * pitch + c.z * pitch * n.y)] = pack2x16float(f.xy);
}
