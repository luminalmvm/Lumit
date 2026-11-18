// Colour neutraliser: takes a colour cast out of the shadows, the midtones
// and the highlights, each on its own, as the CC Color Neutralizer of After
// Effects. Pick, as the Unbalance colour of a range, a part of the picture
// that should be grey and is not: the effect pulls that colour to the grey of
// the same brightness, and everything else in that range with it. The Red,
// Green and Blue rows of a range add to or take from a channel by hand.
// Pinning holds pure black and pure white where they are.

struct Params {
    /// @choice("Final result", "Correction black to white", "Correction stair") @default("Final result") View
    view: u32,
    /// @default(0, 0, 0, 1) Shadows unbalance
    shadows_unbalance: vec4<f32>,
    /// @bounded(0, 200) @default(100) Shadows (%)
    shadows: f32,
    /// @slider(-100, 100) @default(0) Red - shadows
    shadows_red: f32,
    /// @slider(-100, 100) @default(0) Green - shadows
    shadows_green: f32,
    /// @slider(-100, 100) @default(0) Blue - shadows
    shadows_blue: f32,
    /// @default(0.214, 0.214, 0.214, 1) Midtones unbalance
    midtones_unbalance: vec4<f32>,
    /// @bounded(0, 200) @default(100) Midtones (%)
    midtones: f32,
    /// @slider(-100, 100) @default(0) Red - midtones
    midtones_red: f32,
    /// @slider(-100, 100) @default(0) Green - midtones
    midtones_green: f32,
    /// @slider(-100, 100) @default(0) Blue - midtones
    midtones_blue: f32,
    /// @default(1, 1, 1, 1) Highlights unbalance
    highlights_unbalance: vec4<f32>,
    /// @bounded(0, 200) @default(100) Highlights (%)
    highlights: f32,
    /// @slider(-100, 100) @default(0) Red - highlights
    highlights_red: f32,
    /// @slider(-100, 100) @default(0) Green - highlights
    highlights_green: f32,
    /// @slider(-100, 100) @default(0) Blue - highlights
    highlights_blue: f32,
    /// @bounded(0, 100) @default(0) Pinning (%)
    pinning: f32,
    /// @bounded(0, 100) @default(0) Blend with original (%)
    blend_original: f32,
    /// @bounded(0, 100) @default(0) Black point (%)
    black_point: f32,
    /// @bounded(0, 100) @default(100) White point (%)
    white_point: f32,
};

const CN_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

fn cn_display(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

fn cn_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// What one range adds to a colour: the way back to grey of its Unbalance
// colour, by the amount of the range, and the three rows set by hand.
fn cn_shift(unbalance: vec4<f32>, amount: f32, by_hand: vec3<f32>) -> vec3<f32> {
    let u = cn_display(unbalance.rgb);
    let grey = dot(u, CN_LUMA);
    return (vec3<f32>(grey) - u) * amount * 0.01 + by_hand * 0.005;
}

// A colour, as the eye weighs it, with the corrections of the three ranges.
fn cn_correct(c: vec3<f32>) -> vec3<f32> {
    let lo = p.black_point * 0.01;
    let hi = max(p.white_point * 0.01, lo + 1e-3);
    let t = clamp((dot(c, CN_LUMA) - lo) / (hi - lo), 0.0, 1.0);
    // How much of each range the brightness is in. The three add up to one.
    let in_shadows = 1.0 - smoothstep(0.0, 0.5, t);
    let in_highlights = smoothstep(0.5, 1.0, t);
    let in_midtones = 1.0 - in_shadows - in_highlights;
    var shift = in_shadows
        * cn_shift(p.shadows_unbalance, p.shadows,
            vec3<f32>(p.shadows_red, p.shadows_green, p.shadows_blue));
    shift += in_midtones
        * cn_shift(p.midtones_unbalance, p.midtones,
            vec3<f32>(p.midtones_red, p.midtones_green, p.midtones_blue));
    shift += in_highlights
        * cn_shift(p.highlights_unbalance, p.highlights,
            vec3<f32>(p.highlights_red, p.highlights_green, p.highlights_blue));
    // Pinned, the correction falls to nothing at black and at white.
    let pinned = sin(t * 3.14159265);
    shift *= mix(1.0, pinned, clamp(p.pinning * 0.01, 0.0, 1.0));
    return max(c + shift, vec3<f32>(0.0));
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    if (p.view != 0u) {
        // The correction on a ramp from black to white, smooth or in steps.
        var v = clamp(uv.x, 0.0, 1.0);
        if (p.view == 2u) {
            v = min(floor(v * 11.0) / 10.0, 1.0);
        }
        return vec4<f32>(cn_linear(cn_correct(vec3<f32>(v))), 1.0);
    }
    let here = lumit_sample(uv);
    if (here.a <= 0.0) {
        return here;
    }
    let straight = lumit_unpremult(here);
    let fixed = cn_linear(cn_correct(cn_display(straight.rgb)));
    let rgb = mix(fixed, straight.rgb, clamp(p.blend_original * 0.01, 0.0, 1.0));
    return lumit_premult(vec4<f32>(rgb, straight.a));
}
