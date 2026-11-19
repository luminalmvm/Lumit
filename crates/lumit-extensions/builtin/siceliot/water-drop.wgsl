// Water drop: rings spreading from a drop, as Softimage's Ripple texture. From
// Start time a wave packet Rings crests deep runs out from Centre at Speed;
// its crests are Wavelength apart and it fades by Decay as it spreads. With
// Repeat above nought a new drop falls every Repeat seconds. The surface is
// the Calm colour, raised toward Crest colour and lowered toward Trough
// colour.

struct Params {
    /// @point @default(960, 540) Centre
    centre: vec2<f32>,
    /// @slider(2, 1000) @default(60) @unit(px) Wavelength
    wavelength: f32,
    /// @slider(0, 5000) @default(400) Speed
    speed: f32,
    /// @counter(1, 40) @default(6) Rings
    rings: i32,
    /// @bounded(0, 100) @default(100) Amplitude
    amplitude: f32,
    /// @bounded(0, 100) @default(40) Decay
    decay: f32,
    /// @slider(-60, 60) @default(0) @unit(s) Start time
    start: f32,
    /// @slider(0, 60) @default(0) @unit(s) Repeat
    repeat_every: f32,
    /// @default(0.03, 0.12, 0.22, 1) Calm colour
    calm: vec4<f32>,
    /// @default(0.6, 0.8, 0.9, 1) Crest colour
    crest: vec4<f32>,
    /// @default(0.0, 0.02, 0.05, 1) Trough colour
    trough: vec4<f32>,
    /// @choice("Inside layer", "Whole frame") @default("Inside layer") Paint
    paint: u32,
};

// ---- The SICELIOT texture helpers: the same text in every SICELIOT texture ----

// A point in layer pixels turned by -angle about centre: the pattern turns.
fn sx_turn(px: vec2<f32>, centre: vec2<f32>, degrees: f32) -> vec2<f32> {
    let d = px - centre;
    let a = -radians(degrees);
    return vec2<f32>(d.x * cos(a) - d.y * sin(a), d.x * sin(a) + d.y * cos(a));
}

// The finished colour, kept inside the layer when Paint asks for it.
fn sx_paint(c: vec4<f32>, uv: vec2<f32>) -> vec4<f32> {
    if (p.paint == 0u) {
        return c * lumit_sample(uv).a;
    }
    return c;
}

// One drop's height at distance r, t seconds after it fell: -1 to 1.
fn rp_drop(r: f32, t: f32) -> f32 {
    if (t < 0.0) {
        return 0.0;
    }
    let l = max(p.wavelength, 1.0);
    // How far behind the front r is; the packet is Rings wavelengths deep.
    let x = p.speed * t - r;
    let depth = f32(clamp(p.rings, 1, 40)) * l;
    if (x < 0.0 || x > depth) {
        return 0.0;
    }
    // Eased in at the front and out at the tail.
    let env = smoothstep(0.0, l, x) * (1.0 - smoothstep(depth - l, depth, x));
    // Decay: the wave weakens as it spreads, by a third of the screen at 100.
    let fade = exp(-p.decay / 100.0 * (r + x) / 640.0);
    return sin(6.2831853 * x / l) * env * fade;
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let r = distance(lumit_px(uv), p.centre);
    var t = lumit.time - p.start;
    var h = 0.0;
    if (p.repeat_every > 0.0 && t >= 0.0) {
        // This drop, and the one before it whose tail may still be out.
        let tt = t - floor(t / p.repeat_every) * p.repeat_every;
        h = rp_drop(r, tt) + rp_drop(r, tt + p.repeat_every);
    } else {
        h = rp_drop(r, t);
    }
    h = clamp(h * p.amplitude / 100.0, -1.0, 1.0);
    let calm = lumit_premult(p.calm);
    var c = mix(calm, lumit_premult(p.crest), max(h, 0.0));
    c = mix(c, lumit_premult(p.trough), max(-h, 0.0));
    return sx_paint(c, uv);
}
