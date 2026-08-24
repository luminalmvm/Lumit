// The colour pipeline's one shader (docs/impl/gpu-foundation.md §2).
//
// Both passes draw a fullscreen triangle and copy texels; the colour maths
// lives in the TEXTURE FORMATS, which is the whole trick:
//  - linearise pass: source view is Rgba8UnormSrgb, so hardware decodes
//    sRGB → linear on sample; the render target is Rgba16Float, so linear
//    values land in the working format untouched.
//  - display pass: source is the linear Rgba16Float; the render target is
//    Rgba8UnormSrgb, so hardware encodes linear → sRGB on write.
// One shader, zero hand-rolled gamma curves, no chance of drift between
// decode and encode — the auditable single place the design doc demands.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> VsOut {
    // One triangle covering the screen: (-1,-1) (3,-1) (-1,3).
    var out: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> view: ViewParams;

// The two viewer-only controls (docs/07-UI-SPEC.md §2.2, docs/06 §3.3, K-314).
// Neither may ever reach an export: every export path passes the neutral value,
// which this shader short-circuits on so a neutral pass is bit-identical to the
// plain copy it used to be.
struct ViewParams {
    /// 2^stops, computed host-side so the Viewer's number and the Exposure
    /// effect's multiply by the identical float (K-106). 1.0 is neutral.
    gain: f32,
    /// 0 off, 1 on. A fixed curve, no measurement, no carried state — the
    /// picture at a frame never depends on which frame preceded it.
    tone_map: u32,
    _pad0: f32,
    _pad1: f32,
};

/// Rec. 709 luminance, the working space's own primaries (docs/06 §3).
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// Where the shoulder starts. Below this the curve is the identity, exactly —
/// which is the whole promise of the control: on an ordinary composite whose
/// values never pass 1, turning it on changes nothing at all.
const KNEE: f32 = 0.8;

/// Fold everything above the knee into the room left below 1.
///
/// `knee + room * (1 - exp(-(L - knee) / room))` has slope exactly 1 at the
/// knee, so the join is smooth, and approaches 1 without ever reaching it, so
/// no highlight — however bright — clips flat. It is a rolloff, not a grade:
/// scaling RGB by the luminance ratio keeps hue and saturation where the
/// author put them.
fn tone_map_rgb(c: vec3<f32>) -> vec3<f32> {
    let l = luma(c);
    if (l <= KNEE) {
        return c;
    }
    let room = 1.0 - KNEE;
    let mapped = KNEE + room * (1.0 - exp(-(l - KNEE) / room));
    return c * (mapped / l);
}

@fragment
fn fs_copy(in: VsOut) -> @location(0) vec4<f32> {
    return shade(in);
}

/// The display pass for a SIXTEEN-bit export target.
//
// The trick at the top of this file — let the texture format do the gamma —
// has one gap: there is no sixteen-bit sRGB format for the hardware to encode
// into. So the deep display target is Rgba16Float and this applies the same
// curve the hardware applies on an Rgba8UnormSrgb write, in the one place it
// can be compared against it (`the_deep_display_agrees_with_the_eight_bit_one`
// does exactly that, to within a code). Alpha is not encoded, matching the
// hardware, and the clamp matches what a unorm write does to a value past
// full scale.
@fragment
fn fs_display16(in: VsOut) -> @location(0) vec4<f32> {
    let s = shade(in);
    return vec4<f32>(srgb_encode(s.r), srgb_encode(s.g), srgb_encode(s.b), s.a);
}

/// Linear → sRGB, the IEC 61966-2-1 transfer the hardware writes.
fn srgb_encode(c: f32) -> f32 {
    let v = clamp(c, 0.0, 1.0);
    if (v <= 0.0031308) {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

/// What both fragment entry points draw, before any encoding.
fn shade(in: VsOut) -> vec4<f32> {
    let s = textureSample(src, samp, in.uv);
    // The neutral point, bit-exact: the linearise pass and every export take
    // this branch, so they are the copy they always were.
    if (view.gain == 1.0 && view.tone_map == 0u) {
        return s;
    }
    // A scene-linear gain is a scalar, so premultiplied alpha rides through it
    // untouched (K-106). The curve is not linear, so it has to see the colour
    // the author authored: unpremultiply, map, put it back.
    var rgb = s.rgb * view.gain;
    if (view.tone_map != 0u && s.a > 0.0) {
        rgb = tone_map_rgb(rgb / s.a) * s.a;
    } else if (view.tone_map != 0u) {
        rgb = tone_map_rgb(rgb);
    }
    return vec4<f32>(rgb, s.a);
}
