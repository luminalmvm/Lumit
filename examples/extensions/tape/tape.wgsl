// Tape: a worn video tape, drawn in the two passes extension.json lists.
//
// bleed, at half size, smears the colour to the right the way tape does and
// leaves the brightness alone. shade wobbles the lines, puts the smeared
// colour back, then adds the scanlines and the noise. Every row at 0 leaves
// the picture as it came in.
//
// The noise comes from the instance's seed and the layer's time and nothing
// else, so a frame looks the same every time it's drawn.

struct Params {
    /// @slider(0, 20) @default(3) @unit(px) Wobble
    wobble: f32,
    /// @slider(0, 40) @default(12) @unit(px) Bleed
    bleed: f32,
    /// @bounded(0, 100) @default(25) Noise
    noise: f32,
    /// @bounded(0, 100) @default(40) Scanlines
    scanlines: f32,
    /// @choice("PAL", "NTSC") @default("PAL") Style
    style: u32,
};

const PI: f32 = 3.14159265;

// Lines in one field: 288 for PAL, 240 for NTSC.
fn tape_lines() -> f32 {
    return select(288.0, 240.0, p.style == 1u);
}

// Three numbers and the seed, mixed into one.
fn tape_hash(a: u32, b: u32, c: u32) -> u32 {
    var h = a ^ (b * 0x9e3779b9u) ^ (c * 0x85ebca6bu) ^ lumit.seed;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return h;
}

// 0 to 1.
fn tape_rand(a: u32, b: u32, c: u32) -> f32 {
    return f32(tape_hash(a, b, c) >> 8u) / 16777216.0;
}

// The frame as a number. The same time always gives the same noise.
fn tape_frame() -> u32 {
    return bitcast<u32>(lumit.time);
}

// Smooth noise along x, -1 to 1.
fn tape_wave(x: f32, salt: u32) -> f32 {
    let i = floor(x);
    let f = x - i;
    let a = tape_rand(u32(i32(i)), salt, 7u);
    let b = tape_rand(u32(i32(i) + 1), salt, 7u);
    return mix(a, b, f * f * (3.0 - 2.0 * f)) * 2.0 - 1.0;
}

// The colour without its brightness, as tape keeps it (I and Q).
fn tape_iq(c: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(
        dot(c, vec3<f32>(0.596, -0.274, -0.322)),
        dot(c, vec3<f32>(0.211, -0.523, 0.312))
    );
}

// I and Q back to red, green and blue, with no brightness of their own.
fn tape_rgb(iq: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.956 * iq.x + 0.621 * iq.y,
        -0.272 * iq.x - 0.647 * iq.y,
        -1.106 * iq.x + 1.703 * iq.y
    );
}

// The input exactly as it came, at this pixel.
fn tape_here(uv: vec2<f32>) -> vec4<f32> {
    return lumit_load(vec2<i32>(floor(uv * lumit_size())));
}

// Pass one, at half size: the colour, smeared to the right.
fn bleed(uv: vec2<f32>) -> vec4<f32> {
    if (p.bleed == 0.0) {
        return vec4<f32>(0.0);
    }
    // px at comp size, as uv.
    let reach = p.bleed * lumit.comp_scale / lumit_size().x;
    var sum = vec2<f32>(0.0);
    var weight = 0.0;
    for (var i = 0; i < 16; i += 1) {
        let t = f32(i) / 15.0;
        let w = exp(-2.5 * t);
        sum += tape_iq(lumit_sample(uv - vec2<f32>(reach * t, 0.0)).rgb) * w;
        weight += w;
    }
    var iq = sum / weight;
    let k = clamp(abs(p.bleed) / 20.0, 0.0, 1.0);
    if (p.style == 1u) {
        // NTSC: the hue drifts, down the picture and from frame to frame.
        let turn = (0.12 + 0.1 * tape_wave(uv.y * 4.0 + lumit.time * 0.8, 3u)) * k;
        iq = vec2<f32>(
            iq.x * cos(turn) - iq.y * sin(turn),
            iq.x * sin(turn) + iq.y * cos(turn)
        );
    } else {
        // PAL: no drift, but the colour comes out a little paler.
        iq *= 1.0 - 0.15 * k;
    }
    return vec4<f32>(iq, 0.0, 1.0);
}

// Pass two, full size: everything else.
fn shade(uv: vec2<f32>) -> vec4<f32> {
    let lines = tape_lines();
    let frame = tape_frame();
    // The line of the frame this is, two fields to a frame.
    let row = u32(max(uv.y * lines * 2.0, 0.0));

    // Wobble: a slow wave down the picture, a little jitter on every line,
    // and the tear in the last few lines where the heads switch.
    let tear = smoothstep(0.96, 1.0, uv.y);
    let shift = p.wobble * (0.7 * tape_wave(uv.y * 6.0 + lumit.time * 1.7, 1u)
        + 0.3 * (tape_rand(row, frame, 2u) * 2.0 - 1.0)
        + 5.0 * tear * tape_rand(row, frame, 3u));
    let at = uv + vec2<f32>(shift * lumit.comp_scale / lumit_size().x, 0.0);
    var c = select(lumit_sample(at), tape_here(uv), shift == 0.0);

    // Bleed: a softer picture, and the smeared colour in place of its own.
    // PAL also averages each line's colour with the line above.
    let k = clamp(abs(p.bleed) / 2.0, 0.0, 1.0);
    let soft = vec2<f32>(0.12 * p.bleed * lumit.comp_scale / lumit_size().x, 0.0);
    c = mix(c, 0.25 * (lumit_sample(at - soft) + 2.0 * c + lumit_sample(at + soft)), k);
    var iq = lumit_read(pass_bleed, at).xy;
    if (p.style == 0u) {
        iq = 0.5 * (iq + lumit_read(pass_bleed, at - vec2<f32>(0.0, 1.0 / lines)).xy);
    }
    c = vec4<f32>(c.rgb + tape_rgb(iq - tape_iq(c.rgb)) * k * c.a, c.a);

    // Scanlines: a dark gap between the lines of each field. Lines closer
    // than two pixels would shimmer, so they fade to their average shade.
    let gap = pow(abs(cos(PI * uv.y * lines)), 4.0);
    let pitch = f32(lumit.roi_size.y) / lines;
    let even = mix(0.375, gap, clamp(pitch - 1.0, 0.0, 1.0));
    c = vec4<f32>(c.rgb * (1.0 - clamp(p.scanlines / 100.0, 0.0, 1.0) * 0.6 * even), c.a);

    // Noise: grain a little wider than it is tall, about as strong to the
    // eye in the shadows as in the highlights, now and then a white streak
    // where the tape dropped out, and a noisy tear.
    let amount = clamp(p.noise / 100.0, 0.0, 1.0);
    let cell = vec2<u32>(vec2<i32>(floor(lumit_px(uv) / vec2<f32>(2.0, 1.0))));
    let light = 2.0 * sqrt(max(dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0)) + 0.03;
    let grain = (tape_rand(cell.x, cell.y, frame) - 0.5) * 0.2 * amount * light;
    var streak = 0.0;
    if (tape_rand(row, frame, 4u) < amount * 0.02) {
        let begin = tape_rand(row, frame, 5u);
        let along = (uv.x - begin) / (0.05 + 0.3 * tape_rand(row, frame, 6u));
        if (along >= 0.0 && along <= 1.0) {
            streak = (1.0 - along) * amount;
        }
    }
    let torn = tear * clamp(abs(p.wobble) / 3.0, 0.0, 1.0)
        * (tape_rand(cell.x, cell.y, frame ^ 9u) - 0.5) * 0.6 * light;
    return vec4<f32>(c.rgb + (grain + streak + torn) * c.a, c.a);
}
