// Halftone: redraws the picture as a printed screen of dots, lines or
// squares, as Cavalry's Halftone Filter. Each cell reads the picture once at
// its middle; darker means a bigger mark.

struct Params {
    /// @choice("Dots", "Lines", "Squares") @default("Dots") Shape
    shape: u32,
    /// @slider(2, 200) @default(16) @unit(px) Cell size
    cell_size: f32,
    /// @dial @default(45) Angle
    angle: f32,
    /// @slider(0, 10) @default(1) @unit(px) Softness
    softness: f32,
    /// @toggle @default(0) Keep colours
    keep_colours: u32,
    /// @default(0, 0, 0, 1) Ink
    ink: vec4<f32>,
    /// @default(1, 1, 1, 1) Paper
    paper: vec4<f32>,
    /// @toggle @default(0) Reverse
    reverse: u32,
};

fn ht_turn(v: vec2<f32>, deg: f32) -> vec2<f32> {
    let a = radians(deg);
    return vec2<f32>(v.x * cos(a) - v.y * sin(a), v.x * sin(a) + v.y * cos(a));
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let size = max(p.cell_size, 1.0);
    let px = lumit_px(uv);
    let q = ht_turn(px, -p.angle);
    let cell = floor(q / size);
    let local = q - (cell + 0.5) * size;
    // The picture at the cell's middle, turned back into the frame.
    let mid_px = ht_turn((cell + 0.5) * size, p.angle);
    let mid_uv = mid_px / max(lumit_size() / max(lumit.comp_scale, 1e-6), vec2<f32>(1.0));
    let here = lumit_sample(clamp(mid_uv, vec2<f32>(0.0), vec2<f32>(1.0)));
    let straight = lumit_unpremult(here);
    var dark = 1.0 - clamp(dot(straight.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
    if (p.reverse == 1u) {
        dark = 1.0 - dark;
    }
    // How far inside the mark this pixel is, in pixels: positive inside.
    var inside = 0.0;
    switch p.shape {
        case 1u: {
            inside = dark * size * 0.5 - abs(local.y);
        }
        case 2u: {
            inside = sqrt(dark) * size * 0.5 - max(abs(local.x), abs(local.y));
        }
        default: {
            inside = sqrt(dark) * size * 0.7071 - length(local);
        }
    }
    let soft = max(p.softness, 0.5 / max(lumit.comp_scale, 1e-3));
    let cover = clamp(inside / soft + 0.5, 0.0, 1.0);
    var ink = lumit_premult(p.ink);
    if (p.keep_colours == 1u) {
        ink = vec4<f32>(straight.rgb, 1.0);
    }
    let c = mix(lumit_premult(p.paper), ink, cover);
    return c * lumit_sample(uv).a;
}
