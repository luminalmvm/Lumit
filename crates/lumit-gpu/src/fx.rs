//! The GPU effect kernels (docs/05 crate table: "WGSL effect kernels" live
//! here; docs/08-EFFECTS.md §1.1 part 2 — the production path). Each kernel
//! mirrors its CPU reference in `lumit_core::fx::cpu` op-for-op; the §1.6
//! oracle tests at the bottom hold the two to agreement.
//!
//! In plain terms: this is where effects actually run during preview and
//! export — small GPU programs working on the same linear fp16 textures the
//! compositor uses. The engine takes plain numbers (a blur radius in pixels,
//! an edge mode), so it neither knows nor cares about the project model.

use crate::{GpuContext, GpuError, WORKING_FORMAT};

/// One resolved blur, in raster pixels (the caller converts from the
/// spec's %-of-diagonal units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurOp {
    pub radius_px: f32,
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

/// One resolved directional blur (docs/08 §3.8): a line integral along a
/// host-computed unit direction. `taps` must equal
/// `lumit_core::fx::cpu::dir_blur_taps(length_px)` so the GPU dispatches
/// the oracle's exact kernel size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirBlurOp {
    /// Unit streak direction (host-computed cos/sin).
    pub dx: f32,
    pub dy: f32,
    /// Full streak length, raster pixels.
    pub length_px: f32,
    /// Evenly spaced bilinear taps across the streak.
    pub taps: i32,
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DirBlurParams {
    dx: f32,
    dy: f32,
    length: f32,
    taps: i32,
    edge: u32,
    mix_amt: f32,
    _pad: [f32; 2],
}

/// One resolved radial blur — Blur's Radial mode (docs/08 §3.8, schema
/// status note). `taps` must equal
/// `lumit_core::fx::cpu::radial_blur_taps(amount_px)` so the GPU dispatches
/// the oracle's exact kernel size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialBlurOp {
    /// Centre as a *fraction* of the raster (not raster pixels) — the
    /// kernel scales it by its own `textureDimensions`, exactly like the
    /// CPU reference scales it by the `w`/`h` it is handed.
    pub centre_frac: [f32; 2],
    /// Peak tap spread in raster pixels, reached at the frame's farthest
    /// corner from Centre.
    pub amount_px: f32,
    /// Evenly spaced taps along the ray (Zoom) or its perpendicular (Spin).
    pub taps: i32,
    /// True = Spin (tangent direction), false = Zoom (radial direction).
    pub spin: bool,
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RadialBlurParams {
    centre: [f32; 2],
    amount: f32,
    taps: i32,
    spin: u32,
    edge: u32,
    mix_amt: f32,
    _pad: f32,
}

/// One resolved sharpen (docs/08 §3.9), amounts already fractional and the
/// gaussian radius already in raster pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenOp {
    /// Fraction of the detail signal added back (0..3 = 0–300%).
    pub amount: f32,
    pub radius_px: f32,
    /// Linear-light soft gate under which detail is left alone.
    pub threshold: f32,
    /// True: sharpen the Rec. 709 luma only.
    pub luma_only: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurParams {
    dir: [f32; 2],
    radius: f32,
    sigma: f32,
    edge: u32,
    mix_amt: f32,
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SharpenParams {
    amount: f32,
    threshold: f32,
    luma_only: u32,
    mix_amt: f32,
}

/// One resolved RGB split (docs/08 §3.6). The linear-mode offset vector is
/// host-computed (`lumit_core::fx::rgb_split_offset`) so the kernel never
/// runs its own trigonometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbSplitOp {
    /// Linear-mode channel offset, raster pixels.
    pub dx: f32,
    pub dy: f32,
    /// Radial-mode peak offset (reached at the corner distance), raster px.
    pub amount_px: f32,
    pub radial: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RgbSplitParams {
    dx: f32,
    dy: f32,
    amount: f32,
    radial: u32,
    mix_amt: f32,
    _pad: [f32; 3],
}

/// One resolved spectral split — the RGB split's Wavelength mode (docs/08
/// §3.6, K-090), its own kernel so the classic mode stays byte-identical.
/// The offset vector and the wavelength basis both arrive host-computed
/// (`lumit_core::fx::rgb_split_offset` / `spectral_basis_vec4`), so the
/// kernel consumes exactly the CPU reference's numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralSplitOp {
    /// Linear-mode peak offset, raster pixels.
    pub dx: f32,
    pub dy: f32,
    /// Radial-mode peak offset (reached at the corner distance), raster px.
    pub amount_px: f32,
    pub radial: bool,
    /// Wavelength → linear-RGB basis rows (w unused), columns normalised.
    pub basis: [[f32; 4]; 9],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpectralSplitParams {
    basis: [[f32; 4]; 9],
    dx: f32,
    dy: f32,
    amount: f32,
    radial: u32,
    mix_amt: f32,
    _pad: [f32; 3],
}

/// One resolved chromatic aberration (docs/08 §3.15): a dedicated,
/// always-radial sibling of [`RgbSplitOp`]'s own radial mode — no linear
/// offset or wavelength dispersion of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromaticAberrationOp {
    /// Peak channel offset, raster pixels (reached at the corner distance).
    pub amount_px: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ChromaticAberrationParams {
    amount: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved flash (docs/08 §3.7, manual form): the trigger envelope is
/// already evaluated host-side into a plain strength.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlashOp {
    /// 0..1 — envelope × intensity, clamped.
    pub strength: f32,
    /// Scene-linear RGBA flash colour (alpha unused).
    pub colour: [f32; 4],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlashParams {
    colour: [f32; 4],
    strength: f32,
    mix_amt: f32,
    _pad: [f32; 2],
}

/// One resolved colour balance (docs/08 §3.10 as amended by K-090): gain →
/// lift → gamma per channel, in linear on unpremultiplied colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourBalanceOp {
    pub lift: [f32; 3],
    /// Per-channel, > 0 (the resolver clamps).
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ColourBalanceParams {
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
    mix_amt: f32,
    _pad: [f32; 3],
}

/// One resolved saturation (docs/08 §3.10 as amended by K-090): scale about
/// Rec. 709 luma, in linear on unpremultiplied colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SaturationOp {
    /// 0 = greyscale, 1 = neutral, 2 = doubled.
    pub saturation: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SaturationParams {
    saturation: f32,
    mix_amt: f32,
    _pad: [f32; 2],
}

/// One resolved matte key (docs/08 §3.21): a soft chroma key on straight
/// (unpremultiplied) colour. `key` is the scene-linear RGBA key colour (alpha
/// ignored); `tol`/`soft`/`spill` are 0..1 fractions. The kernel derives the
/// key's chroma and hue direction from `key`, exactly as the CPU reference
/// does, so both paths use the same numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatteKeyOp {
    /// Scene-linear RGBA key colour (alpha ignored).
    pub key: [f32; 4],
    /// Chroma-distance threshold, 0..1: at/below it a pixel is fully keyed.
    pub tol: f32,
    /// Soft-edge width above `tol`, 0..1: the smoothstep transition span.
    pub soft: f32,
    /// Key-hue spill removal, 0..1.
    pub spill: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MatteKeyParams {
    key: [f32; 4],
    tol: f32,
    soft: f32,
    spill: f32,
    mix_amt: f32,
}

/// One resolved vignette (docs/08 §3.14): darkens toward black away from
/// the frame centre. Radius/Softness/Roundness are already-clamped
/// fractions; the kernel derives the distance metric from its own
/// `textureDimensions`, exactly like the CPU reference derives it from
/// `w`/`h` — no raster conversion happens host-side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VignetteOp {
    /// 0..1: darkening strength; 0 is the neutral point.
    pub amount: f32,
    /// 0..1: the clear centre's reach.
    pub radius: f32,
    /// 0..1: feather width beyond radius.
    pub softness: f32,
    /// 0..1: 1 = circular, 0 = follows the frame's aspect.
    pub roundness: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VignetteParams {
    amount: f32,
    radius: f32,
    softness: f32,
    roundness: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved exposure (docs/08 §3.16): a single scene-linear gain on the
/// RGB channels. `factor` is `2^stops`, computed host-side so the CPU
/// reference and the kernel multiply by the identical number; alpha is
/// untouched. `factor == 1.0` (0 stops) is the bit-exact neutral point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposureOp {
    /// The linear gain, `2^stops`. 1.0 is the neutral point.
    pub factor: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ExposureParams {
    factor: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved temperature (docs/08 §3.20): a warm/cool white-balance shift as
/// a per-channel gain in scene-linear light. `gain_r`/`gain_b` are computed
/// host-side (`gain_r = 1 + 0.5·k`, `gain_b = 1 − 0.5·k` for `k = temperature /
/// 100`), so the CPU reference and the kernel multiply by byte-identical
/// numbers; green and alpha are untouched. Gains `(1.0, 1.0)` (temperature 0)
/// are the bit-exact neutral point. Premultiplied, exactly like [`ExposureOp`]:
/// a per-channel scalar scales premultiplied colour consistently, so no
/// unpremultiply round trip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemperatureOp {
    /// The scene-linear red gain. 1.0 (with `gain_b` 1.0) is the neutral point.
    pub gain_r: f32,
    /// The scene-linear blue gain. 1.0 (with `gain_r` 1.0) is the neutral point.
    pub gain_b: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TemperatureParams {
    gain_r: f32,
    gain_b: f32,
    mix_amt: f32,
    _pad0: f32,
}

/// One resolved invert (docs/08 §3.23): the colour inverse `out.rgb = 1 − u`
/// per RGB channel, on unpremultiplied colour (`1 − c` is affine, so it does
/// not commute with premultiplied alpha), alpha untouched. There is no neutral
/// value — invert always inverts — so only Mix 0 is the identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InvertOp {
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InvertParams {
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved tint (docs/08 §3.24): a luminance duotone
/// `out.rgb = black + (white − black)·luma(u)` with Rec.709 luma on
/// unpremultiplied colour (a colour remap does not commute with premultiplied
/// alpha), alpha untouched. `black`/`white` are the scene-linear RGB the darkest
/// and brightest input map to; Mix 0 is the identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TintOp {
    /// Scene-linear RGB the darkest input maps to.
    pub black: [f32; 3],
    /// Scene-linear RGB the brightest input maps to.
    pub white: [f32; 3],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TintParams {
    black: [f32; 4],
    white: [f32; 4],
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved contrast (docs/08 §3.18): the affine grade
/// `(u − 0.5) × k + 0.5` per RGB channel about a fixed mid-grey pivot, on
/// unpremultiplied colour (an affine grade does not commute with premultiplied
/// alpha), alpha untouched. `k == 1.0` (Contrast 100 %) is the bit-exact
/// neutral point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastOp {
    /// The contrast factor, `contrast_percent / 100`. 1.0 is the neutral point.
    pub k: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ContrastParams {
    k: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved gamma (docs/08 §3.19): the per-channel power curve
/// `out = pow(max(u, 0), 1/gamma)` on unpremultiplied colour (a non-linear
/// curve does not commute with premultiplied alpha), alpha untouched.
/// `gamma == 1.0` is the bit-exact neutral point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GammaOp {
    /// The gamma value; the kernel raises to `1/gamma`. 1.0 is the neutral
    /// point (clamped ≥ 0.01 host-side so the reciprocal stays finite).
    pub gamma: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GammaParams {
    gamma: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved hue shift (docs/08 §3.17): a row-major linear 3×3 colour
/// matrix, computed host-side (`lumit_core::fx::hue_matrix`) so the CPU
/// reference and the kernel multiply by identical coefficients. The identity
/// matrix is the neutral point; alpha is untouched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HueShiftOp {
    /// Row-major 3×3: `[m00,m01,m02, m10,m11,m12, m20,m21,m22]`.
    pub m: [f32; 9],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HueParams {
    m: [f32; 9],
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved transform (docs/08 §3.5, K-090): the inverse affine arrives
/// host-computed (`lumit_core::fx::transform_op`) so the kernel never runs
/// its own trigonometry and the CPU reference consumes bit-identical
/// numbers. A degenerate (zero-scale) transform arrives as opacity 0 with
/// an identity matrix — fully transparent, exactly like the reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformOp {
    /// Row-major inverse linear 2×2: (m00, m01, m10, m11).
    pub m: [f32; 4],
    /// Inverse translation: sample q = m·p + off.
    pub off: [f32; 2],
    /// 0..1, multiplied into premultiplied RGBA.
    pub opacity: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TransformParams {
    m: [f32; 4],
    off: [f32; 2],
    opacity: f32,
    mix_amt: f32,
}

/// One resolved glow (docs/08 §3.3, v1 core): bright-pass with a soft knee,
/// the shared gaussian on the leftover light, additive recombine. The
/// radius is already in raster pixels; intensity 0 is the neutral point
/// (bit-exact passthrough, matching the CPU reference's short-circuit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlowOp {
    /// The halo gaussian's half-width, raster pixels.
    pub radius_px: f32,
    /// Linear-light bright threshold, ≥ 0 (unbounded above, K-090).
    pub threshold: f32,
    /// Soft-knee width around the threshold, 0..1.
    pub knee: f32,
    /// Gain on the added halo.
    pub intensity: f32,
    /// Scene-linear RGBA halo tint (alpha unused).
    pub tint: [f32; 4],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlowParams {
    tint: [f32; 4],
    threshold: f32,
    knee: f32,
    intensity: f32,
    mix_amt: f32,
}

/// One resolved Block glitch (docs/08 §3.12, split out of the old combined
/// Glitch effect by K-107). `tick` arrives already computed from local time
/// (`lumit_core::fx::GLITCH_TICK_HZ`), so the kernel never sees raw time or
/// does its own time maths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockGlitchOp {
    /// The master 0..1 dial; scales every hashed quantity.
    pub intensity: f32,
    pub seed: u32,
    pub tick: i32,
    /// Raster pixels (px@comp × the §2.3 preview factor).
    pub block_size_px: f32,
    /// 0..1, fraction of block_size_px.
    pub jitter_frac: f32,
    /// Peak per-block displacement, raster pixels.
    pub amount_px: f32,
    /// Peak per-block R/B split, raster pixels.
    pub chan_px: f32,
    /// 0..1: odds (before the Intensity scale) a block slice-repeats.
    pub slice_frac: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlockGlitchParams {
    intensity: f32,
    seed: u32,
    tick: i32,
    block_size: f32,
    jitter_frac: f32,
    amount: f32,
    chan: f32,
    slice_frac: f32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved Scanlines (docs/08 §3.12, split out of the old combined
/// Glitch effect by K-107). `roll_px` arrives already computed from local
/// time (roll speed × time × period), so the kernel never sees raw time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScanlinesOp {
    /// The master 0..1 dial; scales the darken strength.
    pub intensity: f32,
    /// Raster pixels (px@comp × the §2.3 preview factor).
    pub period_px: f32,
    /// 0..1.
    pub darkness: f32,
    /// The scanline pattern's pixel offset at this frame, host-computed.
    pub roll_px: f32,
    pub interlace: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScanlinesParams {
    intensity: f32,
    period: f32,
    darkness: f32,
    roll_px: f32,
    interlace: u32,
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved echo (docs/08 §3.13). The neighbour frames arrive as
/// textures keyed by offset; `weights[i]` is the tap intensity for the echo
/// at offset `-(i+1)` (0 = skip). `mode`: 0 = Add, 1 = Behind, 2 = Max.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoOp {
    pub weights: [f32; 8],
    pub mode: u32,
    /// 0..1, blended against the leading (current) frame.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EchoParams {
    weight: f32,
    mode: u32,
    _pad: [f32; 2],
}

/// One resolved flow motion blur (docs/08 §3.2). The per-pixel motion is a
/// dense flow field passed as its own texture (see [`upload_flow_field`] and
/// [`FxEngine::motion_blur`]); this op carries only the scalars the kernel
/// turns a vector into a streak with. `samples` must equal the resolved
/// `Resolved::MotionBlur::samples` so the GPU integrates the CPU oracle's
/// exact tap count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionBlurOp {
    /// Shutter ÷ 360: streak length as a fraction of the inter-frame motion.
    pub shutter_frac: f32,
    /// Evenly spaced bilinear taps along the streak.
    pub samples: i32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionBlurParams {
    shutter_frac: f32,
    samples: i32,
    mix_amt: f32,
    _pad0: f32,
}

/// One resolved Datamosh pass (docs/08 §3.12, K-104; its own effect since
/// K-107). The raw -1 source neighbour and the dense current→previous flow
/// field arrive as their own textures (see [`FxEngine::datamosh`]); this op
/// carries only the scalar the kernel blends by. Callers fold the schema's
/// Intensity and host Mix into this one field before calling (mixing the
/// same two inputs twice collapses to one mix by the product), so this
/// kernel and its CPU oracle need no second blend knob.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DatamoshOp {
    /// 0..1, blended against the current frame.
    pub intensity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DatamoshParams {
    intensity: f32,
    _pad: [f32; 3],
}

/// One resolved depth-of-field pass (foundation for the planned DoF effects).
/// The per-pixel depth arrives as its own single-channel texture (see
/// [`upload_depth_map`] and [`FxEngine::dof`]); this uniform carries only the
/// scalars the kernel turns a depth into a circle-of-confusion radius with,
/// plus the host Mix. The near side (`d < focus`) uses `near_aperture`, the far
/// side `far_aperture`; both zero (or every pixel inside the sharp band) is a
/// bit-exact passthrough. `depth_invert` and `display` are u32 flags (to match
/// the WGSL uniform's scalar packing). 32 bytes: seven scalars plus one word of
/// tail padding to the 16-byte uniform stride.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DofParams {
    focus: f32,
    range: f32,
    /// Near-side max CoC radius (depths in front of focus), raster px.
    near_aperture: f32,
    /// Far-side max CoC radius (depths behind focus), raster px.
    far_aperture: f32,
    mix_amt: f32,
    /// 0 = read the depth as-is, 1 = invert it (`d' = 1 - d`) before the CoC.
    depth_invert: u32,
    /// Diagnostic view: 0 = Rendered, 1 = Depth map, 2 = Focus map.
    display: u32,
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct AdjustParams {
    opacity: f32,
    _pad: [f32; 3],
}

/// One resolved 3D-LUT lookup (docs/08 §3.11; docs/impl/lut.md). The cube
/// itself arrives as its own 3D texture (see [`upload_lut_3d`] and
/// [`FxEngine::lut`]); this uniform carries only the edge length the shader
/// needs to turn a colour into grid coordinates and the host Mix. Domain is
/// assumed 0..1 (a domain remap is a documented follow-up, docs/impl/lut.md).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LutParams {
    /// LUT edge length N (the cube holds `N³` samples).
    size: u32,
    /// 0..1, blended against the unprocessed input.
    mix: f32,
    _pad: [f32; 2],
}

/// The effect-pass engine: compiled kernels plus their layouts, one per
/// device (owned alongside the Compositor by whoever renders).
pub struct FxEngine {
    blur: wgpu::ComputePipeline,
    dir_blur: wgpu::ComputePipeline,
    radial_blur: wgpu::ComputePipeline,
    sharpen_unpremultiply: wgpu::ComputePipeline,
    sharpen_combine: wgpu::ComputePipeline,
    rgb_split: wgpu::ComputePipeline,
    spectral_split: wgpu::ComputePipeline,
    chromatic_aberration: wgpu::ComputePipeline,
    flash: wgpu::ComputePipeline,
    colour_balance: wgpu::ComputePipeline,
    saturation: wgpu::ComputePipeline,
    matte_key: wgpu::ComputePipeline,
    vignette: wgpu::ComputePipeline,
    exposure: wgpu::ComputePipeline,
    temperature: wgpu::ComputePipeline,
    invert: wgpu::ComputePipeline,
    tint: wgpu::ComputePipeline,
    hue_shift: wgpu::ComputePipeline,
    contrast: wgpu::ComputePipeline,
    gamma: wgpu::ComputePipeline,
    transform: wgpu::ComputePipeline,
    glow_bright: wgpu::ComputePipeline,
    glow_combine: wgpu::ComputePipeline,
    block_glitch: wgpu::ComputePipeline,
    scanlines: wgpu::ComputePipeline,
    echo_accumulate: wgpu::ComputePipeline,
    echo_mix: wgpu::ComputePipeline,
    motion_blur: wgpu::ComputePipeline,
    /// Datamosh (docs/08 §3.12, K-104): shares [`Self::mb_layout`]/`mb_pl`
    /// with Motion blur — both need exactly three sampled inputs (the
    /// current frame, one extra neighbour-derived texture, and a flow
    /// field) plus a storage output and a uniform.
    datamosh: wgpu::ComputePipeline,
    adjust: wgpu::ComputePipeline,
    /// 3D-LUT lookup (docs/08 §3.11; docs/impl/lut.md). Its own pipeline and
    /// [`Self::lut_layout`]: the shared two sampled inputs (src, orig) plus
    /// the cube as a fifth binding — a 3D texture, the first effect to need
    /// one.
    lut: wgpu::ComputePipeline,
    /// Depth-of-field lens blur (foundation for the planned DoF effects).
    /// Shares [`Self::mb_layout`]/`mb_pl` with Motion blur and Datamosh —
    /// its three sampled inputs (source, unprocessed original, depth field)
    /// plus a storage output and a uniform fit the same shape.
    dof: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// The adjustment blend's own layout: three sampled inputs (below,
    /// processed, coverage) where every effect kernel takes two.
    adjust_layout: wgpu::BindGroupLayout,
    /// Flow motion blur's own layout: the shared two inputs (src, orig) plus
    /// the flow-field texture — the one extra sampled input this kernel
    /// needs. Also Datamosh's layout (see [`Self::datamosh`]): its three
    /// sampled inputs (current, previous, flow) fit the same shape.
    mb_layout: wgpu::BindGroupLayout,
    /// The LUT lookup's own layout (see [`Self::lut`]): src (0), orig (1),
    /// the storage output (2), the uniform (3) and the 3D cube texture (4).
    lut_layout: wgpu::BindGroupLayout,
}

impl FxEngine {
    pub fn new(ctx: &GpuContext) -> Self {
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-pl"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let adjust_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-adjust-layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    texture_entry(2),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let adjust_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-adjust-pl"),
                bind_group_layouts: &[&adjust_layout],
                push_constant_ranges: &[],
            });
        // Motion blur's layout: src (0), orig-for-mix (1), the flow field (2),
        // the storage output (3) and the uniform (4) — the shared two-input
        // shape plus the one extra sampled texture (modelled on adjust_layout).
        let mb_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-mb-layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    texture_entry(2),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let mb_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-mb-pl"),
                bind_group_layouts: &[&mb_layout],
                push_constant_ranges: &[],
            });
        // The LUT lookup's layout: src (0), orig-for-mix (1), the storage
        // output (2), the uniform (3) and — the one thing no other kernel has —
        // the cube as a 3D texture at binding 4 (filterable:false; the shader
        // does its own trilinear via textureLoad, docs/impl/lut.md §3).
        let lut_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-lut-layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D3,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let lut_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-lut-pl"),
                bind_group_layouts: &[&lut_layout],
                push_constant_ranges: &[],
            });
        let module = |wgsl: &str, name: &str| {
            ctx.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(name),
                    source: wgpu::ShaderSource::Wgsl(wgsl.into()),
                })
        };
        let pipeline = |shader: &wgpu::ShaderModule, name: &str, entry: &str| {
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(name),
                    layout: Some(&pipeline_layout),
                    module: shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let blur_mod = module(include_str!("fx_blur.wgsl"), "fx-blur");
        let dir_blur_mod = module(include_str!("fx_dirblur.wgsl"), "fx-dir-blur");
        let radial_blur_mod = module(include_str!("fx_radialblur.wgsl"), "fx-radial-blur");
        let sharpen_mod = module(include_str!("fx_sharpen.wgsl"), "fx-sharpen");
        let rgb_split_mod = module(include_str!("fx_rgbsplit.wgsl"), "fx-rgb-split");
        let spectral_mod = module(include_str!("fx_spectral.wgsl"), "fx-spectral-split");
        let chromatic_mod = module(include_str!("fx_chromatic.wgsl"), "fx-chromatic-aberration");
        let flash_mod = module(include_str!("fx_flash.wgsl"), "fx-flash");
        let balance_mod = module(include_str!("fx_colourbalance.wgsl"), "fx-colour-balance");
        let saturation_mod = module(include_str!("fx_saturation.wgsl"), "fx-saturation");
        let matte_key_mod = module(include_str!("fx_matte_key.wgsl"), "fx-matte-key");
        let vignette_mod = module(include_str!("fx_vignette.wgsl"), "fx-vignette");
        let exposure_mod = module(include_str!("fx_exposure.wgsl"), "fx-exposure");
        let temperature_mod = module(include_str!("fx_temperature.wgsl"), "fx-temperature");
        let invert_mod = module(include_str!("fx_invert.wgsl"), "fx-invert");
        let tint_mod = module(include_str!("fx_tint.wgsl"), "fx-tint");
        let hue_mod = module(include_str!("fx_hue.wgsl"), "fx-hue");
        let contrast_mod = module(include_str!("fx_contrast.wgsl"), "fx-contrast");
        let gamma_mod = module(include_str!("fx_gamma.wgsl"), "fx-gamma");
        let transform_mod = module(include_str!("fx_transform.wgsl"), "fx-transform");
        let glow_mod = module(include_str!("fx_glow.wgsl"), "fx-glow");
        let block_glitch_mod = module(include_str!("fx_block_glitch.wgsl"), "fx-block-glitch");
        let scanlines_mod = module(include_str!("fx_scanlines.wgsl"), "fx-scanlines");
        let echo_mod = module(include_str!("fx_echo.wgsl"), "fx-echo");
        let motion_blur_mod = module(include_str!("fx_motionblur.wgsl"), "fx-motion-blur");
        let datamosh_mod = module(include_str!("fx_datamosh.wgsl"), "fx-datamosh");
        let dof_mod = module(include_str!("fx_dof.wgsl"), "fx-dof");
        let adjust_mod = module(include_str!("fx_adjust.wgsl"), "fx-adjust");
        let lut_mod = module(include_str!("fx_lut.wgsl"), "fx-lut");
        let blur = pipeline(&blur_mod, "fx-blur", "blur_pass");
        let dir_blur = pipeline(&dir_blur_mod, "fx-dir-blur", "dir_blur");
        let radial_blur = pipeline(&radial_blur_mod, "fx-radial-blur", "radial_blur");
        let sharpen_unpremultiply = pipeline(&sharpen_mod, "fx-sharpen-un", "unpremultiply");
        let sharpen_combine = pipeline(&sharpen_mod, "fx-sharpen", "sharpen_combine");
        let rgb_split = pipeline(&rgb_split_mod, "fx-rgb-split", "rgb_split");
        let spectral_split = pipeline(&spectral_mod, "fx-spectral-split", "spectral_split");
        let chromatic_aberration = pipeline(
            &chromatic_mod,
            "fx-chromatic-aberration",
            "chromatic_aberration",
        );
        let flash = pipeline(&flash_mod, "fx-flash", "flash");
        let colour_balance = pipeline(&balance_mod, "fx-colour-balance", "colour_balance");
        let saturation = pipeline(&saturation_mod, "fx-saturation", "saturate_fx");
        let matte_key = pipeline(&matte_key_mod, "fx-matte-key", "matte_key");
        let vignette = pipeline(&vignette_mod, "fx-vignette", "vignette");
        let exposure = pipeline(&exposure_mod, "fx-exposure", "exposure");
        let temperature = pipeline(&temperature_mod, "fx-temperature", "temperature");
        let invert = pipeline(&invert_mod, "fx-invert", "invert");
        let tint = pipeline(&tint_mod, "fx-tint", "tint");
        let hue_shift = pipeline(&hue_mod, "fx-hue", "hue_shift");
        let contrast = pipeline(&contrast_mod, "fx-contrast", "contrast");
        let gamma = pipeline(&gamma_mod, "fx-gamma", "gamma");
        let transform = pipeline(&transform_mod, "fx-transform", "transform");
        let glow_bright = pipeline(&glow_mod, "fx-glow-bright", "glow_bright");
        let glow_combine = pipeline(&glow_mod, "fx-glow", "glow_combine");
        let block_glitch = pipeline(&block_glitch_mod, "fx-block-glitch", "block_glitch");
        let scanlines = pipeline(&scanlines_mod, "fx-scanlines", "scanlines");
        let echo_accumulate = pipeline(&echo_mod, "fx-echo-accumulate", "echo_accumulate");
        let echo_mix = pipeline(&echo_mod, "fx-echo-mix", "echo_mix");
        let motion_blur = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-motion-blur"),
                layout: Some(&mb_pl),
                module: &motion_blur_mod,
                entry_point: Some("motion_blur"),
                compilation_options: Default::default(),
                cache: None,
            });
        let datamosh = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-datamosh"),
                layout: Some(&mb_pl),
                module: &datamosh_mod,
                entry_point: Some("datamosh"),
                compilation_options: Default::default(),
                cache: None,
            });
        let dof = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-dof"),
                layout: Some(&mb_pl),
                module: &dof_mod,
                entry_point: Some("dof"),
                compilation_options: Default::default(),
                cache: None,
            });
        let adjust = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-adjust"),
                layout: Some(&adjust_pl),
                module: &adjust_mod,
                entry_point: Some("adjust_blend"),
                compilation_options: Default::default(),
                cache: None,
            });
        let lut = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-lut"),
                layout: Some(&lut_pl),
                module: &lut_mod,
                entry_point: Some("lut_apply"),
                compilation_options: Default::default(),
                cache: None,
            });
        Self {
            blur,
            dir_blur,
            radial_blur,
            sharpen_unpremultiply,
            sharpen_combine,
            rgb_split,
            spectral_split,
            chromatic_aberration,
            flash,
            colour_balance,
            saturation,
            matte_key,
            vignette,
            exposure,
            temperature,
            invert,
            tint,
            hue_shift,
            contrast,
            gamma,
            transform,
            glow_bright,
            glow_combine,
            block_glitch,
            scanlines,
            echo_accumulate,
            echo_mix,
            motion_blur,
            datamosh,
            dof,
            adjust,
            lut,
            layout,
            adjust_layout,
            mb_layout,
            lut_layout,
        }
    }

    /// Apply one echo/trails (docs/08 §3.13) to a linear working texture,
    /// returning a new texture of the same size. Starts the accumulator as
    /// the current frame (an `echo_accumulate` with weight 0 copies it), folds
    /// in each live tap's neighbour (looked up by offset `-(i+1)`), then mixes
    /// the trail back toward the current frame. A missing neighbour or a zero
    /// weight is skipped, so the pass cost tracks the live tap count.
    pub fn echo(
        &self,
        ctx: &GpuContext,
        current: &wgpu::Texture,
        neighbours: &[(i32, &wgpu::Texture)],
        w: u32,
        h: u32,
        op: &EchoOp,
    ) -> wgpu::Texture {
        let params = |weight: f32, mode: u32| EchoParams {
            weight,
            mode,
            _pad: [0.0; 2],
        };
        // acc := current (weight 0 add = a + n*0 = a).
        let mut acc = work_texture(ctx, w, h, "fx-echo-acc");
        self.dispatch(
            ctx,
            &self.echo_accumulate,
            current,
            current,
            &acc,
            w,
            h,
            bytemuck::bytes_of(&params(0.0, 0)),
        );
        for (i, &weight) in op.weights.iter().enumerate() {
            if weight <= 0.0 {
                continue;
            }
            let offset = -(i as i32 + 1);
            let Some((_, tex)) = neighbours.iter().find(|(o, _)| *o == offset) else {
                continue;
            };
            let next = work_texture(ctx, w, h, "fx-echo-acc");
            self.dispatch(
                ctx,
                &self.echo_accumulate,
                &acc,
                tex,
                &next,
                w,
                h,
                bytemuck::bytes_of(&params(weight, op.mode)),
            );
            acc = next;
        }
        let out = work_texture(ctx, w, h, "fx-echo-out");
        self.dispatch(
            ctx,
            &self.echo_mix,
            &acc,
            current,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params(op.mix, 0)),
        );
        out
    }

    /// Apply one flow motion blur (docs/08 §3.2) to a linear working texture,
    /// returning a new texture of the same size. One pass: per output pixel,
    /// read its motion vector from `flow` (a two-channel field the same size
    /// as `src`, in raster pixels) and integrate `op.samples` box-weighted
    /// bilinear taps along the centred streak `± motion × shutter_frac`, then
    /// blend against the input by the host Mix. `flow`'s vectors are consumed
    /// exactly as `lumit_core::fx::cpu::motion_blur` reads its `u`/`v` slices,
    /// so the two agree (§1.6). Its own bind group (the flow field is the one
    /// extra input over the shared two-input shape).
    pub fn motion_blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        flow: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &MotionBlurOp,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-mb-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-mb-params"),
                contents: bytemuck::bytes_of(&MotionBlurParams {
                    shutter_frac: op.shutter_frac,
                    samples: op.samples.max(1),
                    mix_amt: op.mix,
                    _pad0: 0.0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-mb-bind"),
            layout: &self.mb_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                // orig-for-mix: a single pass, so the unprocessed original is
                // the source itself.
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(flow)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fx-mb-enc"),
            });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-mb-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.motion_blur);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        ctx.queue.submit([enc.finish()]);
        out
    }

    /// Apply Datamosh (docs/08 §3.12, K-104; its own effect since K-107)
    /// to a linear working texture, returning a new texture of the
    /// same size. One pass: per output pixel, read its current→previous
    /// motion vector from `flow` and take a single bilinear tap of `prev`
    /// at the displaced position — a motion-compensated prediction, not a
    /// streak integral — then blend against `cur` by Intensity. Shares
    /// [`Self::mb_layout`]/its pipeline layout with Motion blur (same
    /// three-sampled-input shape); its own pipeline and shader.
    #[allow(clippy::too_many_arguments)]
    pub fn datamosh(
        &self,
        ctx: &GpuContext,
        cur: &wgpu::Texture,
        prev: &wgpu::Texture,
        flow: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &DatamoshOp,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-dm-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-dm-params"),
                contents: bytemuck::bytes_of(&DatamoshParams {
                    intensity: op.intensity,
                    _pad: [0.0; 3],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-dm-bind"),
            layout: &self.mb_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(cur)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(prev)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(flow)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fx-dm-enc"),
            });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-dm-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.datamosh);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        ctx.queue.submit([enc.finish()]);
        out
    }

    /// Apply one depth-of-field lens blur to a linear working texture,
    /// returning a new texture of the same size. Backs the `dof` effect
    /// (docs/08 §3.22, docs/impl/layer-input.md): one pass where each output
    /// pixel reads its depth from the **red channel** of `depth` (values in
    /// `[0, 1]` by convention; the shader reads `.x`), optionally inverts it
    /// (`depth_invert`: `d' = 1 - d`, swapping near and far), turns it into a
    /// circle-of-confusion radius — zero inside `range` of `focus`, ramping smoothstep
    /// to `near_aperture` raster pixels on the near side (`d < focus`) or
    /// `far_aperture` on the far side at the depth extreme — and averages a
    /// box-weighted integer disc of that radius from `src`, edges clamped,
    /// then blends against the input by the host Mix. `display` selects the
    /// output view: 0 = Rendered (the blur above), 1 = Depth map (the
    /// post-invert depth as greyscale), 2 = Focus map (the smooth `1 - s`
    /// in-focus mask); the diagnostic views ignore the blur and Mix and are
    /// continuous, so the oracle covers them. `depth` must be the same
    /// size as `src`; because only its red is read (via `textureLoad`, not a
    /// sampler), it may be **any float texture** — the referenced depth layer
    /// rendered in the working `rgba16float` format (the effect's real depth
    /// input), or the exact R32Float map the §1.6 oracle uploads; both read the
    /// same red. `depth` is consumed exactly as `dof_reference` (the CPU
    /// oracle) reads it and the tap disc is byte-identical, so the two agree.
    /// Shares [`Self::mb_layout`] with Motion blur — the depth field is the one
    /// extra sampled input over the two-input convention. Both apertures zero,
    /// or a Mix of 0, is a bit-exact passthrough.
    #[allow(clippy::too_many_arguments)]
    pub fn dof(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        depth: &wgpu::Texture,
        focus: f32,
        range: f32,
        near_aperture: f32,
        far_aperture: f32,
        depth_invert: bool,
        display: u32,
        mix: f32,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-dof-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-dof-params"),
                contents: bytemuck::bytes_of(&DofParams {
                    focus,
                    range,
                    near_aperture,
                    far_aperture,
                    mix_amt: mix,
                    depth_invert: u32::from(depth_invert),
                    display,
                    _pad: 0.0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-dof-bind"),
            layout: &self.mb_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                // orig-for-mix: a single pass, so the unprocessed original is
                // the source itself.
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(depth)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fx-dof-enc"),
            });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-dof-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.dof);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        ctx.queue.submit([enc.finish()]);
        out
    }

    /// Apply one 3D-LUT lookup (docs/08 §3.11; docs/impl/lut.md) to a linear
    /// working texture, returning a new texture of the same size. One pass on
    /// **unpremultiplied** colour (§2.2 — a LUT is an arbitrary colour map):
    /// per output pixel, unpremultiply, map each channel to a grid coordinate
    /// in `[0, size-1]` (domain assumed 0..1, clamped), `textureLoad` the eight
    /// integer corners of `lut_tex` and trilinearly interpolate in f32 — **not**
    /// the hardware sampler, whose precision is not guaranteed bit-for-bit
    /// across GPUs (docs/impl/lut.md §3) — re-premultiply, then blend against
    /// the input by the host Mix. The cube is consumed exactly as
    /// `lumit_core::lut::Lut3d::sample` reads its red-fastest data, so the two
    /// agree (§1.6). Its own bind group (the cube is a 3D texture, the one
    /// binding no other kernel has). `mix == 0` is the bit-exact input.
    #[allow(clippy::too_many_arguments)]
    pub fn lut(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        lut_tex: &wgpu::Texture,
        size: u32,
        mix: f32,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-lut-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-lut-params"),
                contents: bytemuck::bytes_of(&LutParams {
                    size,
                    mix,
                    _pad: [0.0; 2],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        // The cube is a 3D texture; name its view dimension explicitly so the
        // binding matches the layout's `D3` regardless of the default.
        let lut_view = lut_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-lut-bind"),
            layout: &self.lut_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                // orig-for-mix: a single pass, so the unprocessed original is
                // the source itself.
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(src)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(&out)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
            ],
        });
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fx-lut-enc"),
            });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-lut-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.lut);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        ctx.queue.submit([enc.finish()]);
        out
    }

    /// The adjustment-layer blend (docs/06 §1.5): per-channel lerp between
    /// the accumulated composite `below` and its effected copy `processed`,
    /// by `coverage`'s alpha (the layer's comp-space mask raster) times
    /// `opacity` (the layer opacity, 0..1). All three textures are comp
    /// sized; returns a new comp-sized working texture.
    #[allow(clippy::too_many_arguments)]
    pub fn adjust_blend(
        &self,
        ctx: &GpuContext,
        below: &wgpu::Texture,
        processed: &wgpu::Texture,
        coverage: &wgpu::Texture,
        w: u32,
        h: u32,
        opacity: f32,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let out = work_texture(ctx, w, h, "fx-adjust-out");
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-adjust-params"),
                contents: bytemuck::bytes_of(&AdjustParams {
                    opacity,
                    _pad: [0.0; 3],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-adjust-bind"),
            layout: &self.adjust_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &below.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &processed.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &coverage.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        &out.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fx-adjust-enc"),
            });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-adjust-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.adjust);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        ctx.queue.submit([enc.finish()]);
        out
    }

    /// Apply one gaussian blur to a linear working texture, returning a new
    /// texture of the same size (two separable passes; the host Mix blends
    /// the final pass against the untouched input, docs/08 §1.5).
    pub fn blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &BlurOp,
    ) -> wgpu::Texture {
        let tmp = work_texture(ctx, w, h, "fx-blur-tmp");
        let out = work_texture(ctx, w, h, "fx-blur-out");
        let sigma = (op.radius_px * 0.5).max(1e-3);
        // Horizontal into tmp (mix 1: the blend happens once, at the end).
        self.dispatch(
            ctx,
            &self.blur,
            src,
            src,
            &tmp,
            w,
            h,
            bytemuck::bytes_of(&BlurParams {
                dir: [1.0, 0.0],
                radius: op.radius_px,
                sigma,
                edge: op.edge,
                mix_amt: 1.0,
                _pad: [0.0; 2],
            }),
        );
        // Vertical into out, blending against the original input.
        self.dispatch(
            ctx,
            &self.blur,
            &tmp,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BlurParams {
                dir: [0.0, 1.0],
                radius: op.radius_px,
                sigma,
                edge: op.edge,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one directional blur (docs/08 §3.8) to a linear working
    /// texture, returning a new texture of the same size. One pass: a
    /// box-weighted line integral of bilinear taps along the unit direction.
    pub fn dir_blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &DirBlurOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-dir-blur-out");
        self.dispatch(
            ctx,
            &self.dir_blur,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&DirBlurParams {
                dx: op.dx,
                dy: op.dy,
                length: op.length_px,
                taps: op.taps,
                edge: op.edge,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one radial blur — Blur's Radial mode (docs/08 §3.8) — to a
    /// linear working texture, returning a new texture of the same size.
    /// One pass: box-weighted taps along a ray (Zoom) or its perpendicular
    /// (Spin), the shared schema-status-note maths.
    pub fn radial_blur(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &RadialBlurOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-radial-blur-out");
        self.dispatch(
            ctx,
            &self.radial_blur,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&RadialBlurParams {
                centre: op.centre_frac,
                amount: op.amount_px,
                taps: op.taps,
                spin: u32::from(op.spin),
                edge: op.edge,
                mix_amt: op.mix,
                _pad: 0.0,
            }),
        );
        out
    }

    /// Apply one unsharp mask (docs/08 §3.9) to a linear working texture,
    /// returning a new texture of the same size. Four passes: unpremultiply
    /// (§2.2, fused into the kernel chain), a separable gaussian on the
    /// unpremultiplied colour (reusing the blur kernel, Repeat edges — the
    /// CPU reference blurs with the same fixed policy), then the combine
    /// pass that gates, re-premultiplies and applies the host Mix.
    pub fn sharpen(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SharpenOp,
    ) -> wgpu::Texture {
        let un = work_texture(ctx, w, h, "fx-sharpen-un");
        let tmp = work_texture(ctx, w, h, "fx-sharpen-tmp");
        let blurred = work_texture(ctx, w, h, "fx-sharpen-blur");
        let out = work_texture(ctx, w, h, "fx-sharpen-out");
        let params = SharpenParams {
            amount: op.amount,
            threshold: op.threshold,
            luma_only: u32::from(op.luma_only),
            mix_amt: op.mix,
        };
        self.dispatch(
            ctx,
            &self.sharpen_unpremultiply,
            src,
            src,
            &un,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        let sigma = (op.radius_px * 0.5).max(1e-3);
        for (pass_src, pass_dst, dir) in [(&un, &tmp, [1.0, 0.0]), (&tmp, &blurred, [0.0, 1.0])] {
            self.dispatch(
                ctx,
                &self.blur,
                pass_src,
                pass_src,
                pass_dst,
                w,
                h,
                bytemuck::bytes_of(&BlurParams {
                    dir,
                    radius: op.radius_px,
                    sigma,
                    edge: 1, // Repeat, always (see the schema comment)
                    mix_amt: 1.0,
                    _pad: [0.0; 2],
                }),
            );
        }
        self.dispatch(
            ctx,
            &self.sharpen_combine,
            &blurred,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        out
    }

    /// Apply one RGB split (docs/08 §3.6) to a linear working texture,
    /// returning a new texture of the same size. Single pointwise pass with
    /// offset bilinear taps.
    pub fn rgb_split(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &RgbSplitOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-rgb-split-out");
        self.dispatch(
            ctx,
            &self.rgb_split,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&RgbSplitParams {
                dx: op.dx,
                dy: op.dy,
                amount: op.amount_px,
                radial: u32::from(op.radial),
                mix_amt: op.mix,
                _pad: [0.0; 3],
            }),
        );
        out
    }

    /// Apply one spectral split — the RGB split's Wavelength mode (docs/08
    /// §3.6, K-090) — to a linear working texture, returning a new texture
    /// of the same size. Single pointwise pass, nine offset bilinear taps
    /// weighted by the host-supplied wavelength basis.
    pub fn spectral_split(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SpectralSplitOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-spectral-split-out");
        self.dispatch(
            ctx,
            &self.spectral_split,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SpectralSplitParams {
                basis: op.basis,
                dx: op.dx,
                dy: op.dy,
                amount: op.amount_px,
                radial: u32::from(op.radial),
                mix_amt: op.mix,
                _pad: [0.0; 3],
            }),
        );
        out
    }

    /// Apply one chromatic aberration (docs/08 §3.15) to a linear working
    /// texture, returning a new texture of the same size. Single pointwise
    /// pass with offset bilinear taps — a dedicated, always-radial sibling
    /// of [`FxEngine::rgb_split`]'s own radial mode.
    pub fn chromatic_aberration(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ChromaticAberrationOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-chromatic-aberration-out");
        self.dispatch(
            ctx,
            &self.chromatic_aberration,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ChromaticAberrationParams {
                amount: op.amount_px,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one flash (docs/08 §3.7, manual form) to a linear working
    /// texture, returning a new texture of the same size. One pointwise
    /// pass; the trigger envelope arrives pre-evaluated in the op.
    pub fn flash(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &FlashOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-flash-out");
        self.dispatch(
            ctx,
            &self.flash,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&FlashParams {
                colour: op.colour,
                strength: op.strength,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one colour balance (docs/08 §3.10 as amended by K-090) to a
    /// linear working texture, returning a new texture of the same size.
    /// One pointwise pass; the §2.2 unpremultiply wrap is fused into the
    /// kernel, and fully neutral parameters short-circuit inside it.
    pub fn colour_balance(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ColourBalanceOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-colour-balance-out");
        let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        self.dispatch(
            ctx,
            &self.colour_balance,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ColourBalanceParams {
                lift: v4(op.lift),
                gamma: v4(op.gamma),
                gain: v4(op.gain),
                mix_amt: op.mix,
                _pad: [0.0; 3],
            }),
        );
        out
    }

    /// Apply one saturation (docs/08 §3.10 as amended by K-090) to a linear
    /// working texture, returning a new texture of the same size. One
    /// pointwise pass; the §2.2 unpremultiply wrap is fused into the
    /// kernel, and saturation 1 short-circuits inside it.
    pub fn saturation(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &SaturationOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-saturation-out");
        self.dispatch(
            ctx,
            &self.saturation,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SaturationParams {
                saturation: op.saturation,
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one matte key (docs/08 §3.21) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; the §2.2
    /// unpremultiply wrap is fused into the kernel, which derives the key's
    /// chroma/hue direction from `key` exactly as the CPU reference does. There
    /// is no neutral short-circuit (the default keys); Mix 0 is the identity.
    pub fn matte_key(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &MatteKeyOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-matte-key-out");
        self.dispatch(
            ctx,
            &self.matte_key,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&MatteKeyParams {
                key: op.key,
                tol: op.tol,
                soft: op.soft,
                spill: op.spill,
                mix_amt: op.mix,
            }),
        );
        out
    }

    /// Apply one vignette (docs/08 §3.14) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; the
    /// kernel derives the distance metric from its own texture size, and
    /// Amount 0 short-circuits inside it.
    pub fn vignette(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &VignetteOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-vignette-out");
        self.dispatch(
            ctx,
            &self.vignette,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&VignetteParams {
                amount: op.amount,
                radius: op.radius,
                softness: op.softness,
                roundness: op.roundness,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one exposure (docs/08 §3.16) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass: RGB × the
    /// host-computed `factor`, alpha untouched; `factor == 1.0` short-circuits
    /// to the input inside the kernel.
    pub fn exposure(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ExposureOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-exposure-out");
        self.dispatch(
            ctx,
            &self.exposure,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ExposureParams {
                factor: op.factor,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one temperature (docs/08 §3.20) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass: R × the
    /// host-computed `gain_r` and B × `gain_b`, green and alpha untouched;
    /// `gain_r == 1.0 && gain_b == 1.0` (temperature 0) short-circuits to the
    /// input inside the kernel. Premultiplied, exactly like [`Self::exposure`].
    pub fn temperature(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &TemperatureOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-temperature-out");
        self.dispatch(
            ctx,
            &self.temperature,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TemperatureParams {
                gain_r: op.gain_r,
                gain_b: op.gain_b,
                mix_amt: op.mix,
                _pad0: 0.0,
            }),
        );
        out
    }

    /// Apply one invert (docs/08 §3.23) to a linear working texture, returning a
    /// new texture of the same size. One pointwise pass: `1 − u` per channel, the
    /// §2.2 unpremultiply wrap fused into the kernel. There is no neutral
    /// short-circuit (invert always inverts); Mix 0 is the identity.
    pub fn invert(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &InvertOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-invert-out");
        self.dispatch(
            ctx,
            &self.invert,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&InvertParams {
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one tint (docs/08 §3.24) to a linear working texture, returning a
    /// new texture of the same size. One pointwise pass: the luma-driven lerp
    /// between the two mapped colours, the §2.2 unpremultiply wrap fused into the
    /// kernel; Mix 0 is the identity.
    pub fn tint(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &TintOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-tint-out");
        let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        self.dispatch(
            ctx,
            &self.tint,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TintParams {
                black: v4(op.black),
                white: v4(op.white),
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one contrast (docs/08 §3.18) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass: the
    /// affine grade about mid-grey, the §2.2 unpremultiply wrap fused into the
    /// kernel; `k == 1.0` short-circuits to the input inside the kernel.
    pub fn contrast(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ContrastOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-contrast-out");
        self.dispatch(
            ctx,
            &self.contrast,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ContrastParams {
                k: op.k,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one gamma (docs/08 §3.19) to a linear working texture, returning a
    /// new texture of the same size. One pointwise pass: the per-channel power
    /// curve `pow(max(u, 0), 1/gamma)`, the §2.2 unpremultiply wrap fused into
    /// the kernel; `gamma == 1.0` short-circuits to the input inside the kernel.
    pub fn gamma(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &GammaOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-gamma-out");
        self.dispatch(
            ctx,
            &self.gamma,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&GammaParams {
                gamma: op.gamma,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one hue shift (docs/08 §3.17) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass: RGB × the
    /// host-computed colour matrix, alpha untouched.
    pub fn hue_shift(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &HueShiftOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-hue-out");
        self.dispatch(
            ctx,
            &self.hue_shift,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&HueParams {
                m: op.m,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one transform (docs/08 §3.5, K-090) to a linear working
    /// texture, returning a new texture of the same size. One pass: each
    /// output pixel takes a single bilinear tap through the host-computed
    /// inverse affine, transparent outside the frame, opacity folded in.
    /// Identity parameters reproduce the input bit-exactly.
    pub fn transform(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &TransformOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-transform-out");
        self.dispatch(
            ctx,
            &self.transform,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TransformParams {
                m: op.m,
                off: op.off,
                opacity: op.opacity,
                mix_amt: op.mix,
            }),
        );
        out
    }

    /// Apply one glow (docs/08 §3.3, v1 core) to a linear working texture,
    /// returning a new texture of the same size. Four passes: the bright
    /// pass keeps only the light above the threshold (soft knee, all four
    /// premultiplied channels — the halo carries alpha), the shared
    /// separable gaussian widens it (Repeat edges, fixed: the halo holds
    /// its strength along frame borders), and the combine pass adds
    /// `intensity · tint · halo` back onto the untouched input in linear,
    /// alpha saturating at 1. Intensity 0 short-circuits inside the combine
    /// kernel to the bit-exact identity.
    pub fn glow(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &GlowOp,
    ) -> wgpu::Texture {
        let bright = work_texture(ctx, w, h, "fx-glow-bright");
        let tmp = work_texture(ctx, w, h, "fx-glow-tmp");
        let blurred = work_texture(ctx, w, h, "fx-glow-blur");
        let out = work_texture(ctx, w, h, "fx-glow-out");
        let params = GlowParams {
            tint: op.tint,
            threshold: op.threshold,
            knee: op.knee,
            intensity: op.intensity,
            mix_amt: op.mix,
        };
        self.dispatch(
            ctx,
            &self.glow_bright,
            src,
            src,
            &bright,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        let sigma = (op.radius_px * 0.5).max(1e-3);
        for (pass_src, pass_dst, dir) in [(&bright, &tmp, [1.0, 0.0]), (&tmp, &blurred, [0.0, 1.0])]
        {
            self.dispatch(
                ctx,
                &self.blur,
                pass_src,
                pass_src,
                pass_dst,
                w,
                h,
                bytemuck::bytes_of(&BlurParams {
                    dir,
                    radius: op.radius_px,
                    sigma,
                    edge: 1, // Repeat, always (see the CPU reference)
                    mix_amt: 1.0,
                    _pad: [0.0; 2],
                }),
            );
        }
        self.dispatch(
            ctx,
            &self.glow_combine,
            &blurred,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&params),
        );
        out
    }

    /// Apply one Block glitch (docs/08 §3.12, split out by K-107) to a
    /// linear working texture, returning a new texture of the same size.
    /// One pointwise-with-taps pass: block UV displacement and channel
    /// offset.
    pub fn block_glitch(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &BlockGlitchOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-block-glitch-out");
        self.dispatch(
            ctx,
            &self.block_glitch,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BlockGlitchParams {
                intensity: op.intensity,
                seed: op.seed,
                tick: op.tick,
                block_size: op.block_size_px,
                jitter_frac: op.jitter_frac,
                amount: op.amount_px,
                chan: op.chan_px,
                slice_frac: op.slice_frac,
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one Scanlines (docs/08 §3.12, split out by K-107) to a linear
    /// working texture, returning a new texture of the same size. One
    /// pointwise pass: periodic darkening in raster Y, no neighbour taps.
    pub fn scanlines(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &ScanlinesOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-scanlines-out");
        self.dispatch(
            ctx,
            &self.scanlines,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ScanlinesParams {
                intensity: op.intensity,
                period: op.period_px,
                darkness: op.darkness,
                roll_px: op.roll_px,
                interlace: u32::from(op.interlace),
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// One compute pass: `src` and `orig` sampled, `dst` written, `params`
    /// as the uniform — the shared plumbing every kernel dispatch uses.
    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &self,
        ctx: &GpuContext,
        pipeline: &wgpu::ComputePipeline,
        src: &wgpu::Texture,
        orig: &wgpu::Texture,
        dst: &wgpu::Texture,
        w: u32,
        h: u32,
        params: &[u8],
    ) {
        use wgpu::util::DeviceExt;
        let ubuf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx-params"),
                contents: params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &src.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &orig.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &dst.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fx-enc"),
            });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        ctx.queue.submit([enc.finish()]);
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn work_texture(ctx: &GpuContext, w: u32, h: u32, label: &str) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: WORKING_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// Upload a linear f32 RGBA image as a working (fp16) texture — test and
/// tooling support for effect kernels.
pub fn upload_linear_f32(ctx: &GpuContext, rgba: &[f32], w: u32, h: u32) -> wgpu::Texture {
    let tex = work_texture(ctx, w, h, "fx-upload");
    let halfs: Vec<u16> = rgba.iter().map(|v| f16_bits(*v)).collect();
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&halfs),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 8),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    tex
}

/// Upload a 3D colour LUT cube as an `rgba32float` 3D texture for
/// [`FxEngine::lut`]. `data` is `size³` RGB triplets, **red-fastest** (flat
/// index `r + g*size + b*size*size`, the layout `lumit_core::lut::Lut3d`
/// stores). Each triplet is padded to RGBA (alpha 1.0, unused) and written at
/// full f32 precision, so the shader's manual trilinear lookup reads the exact
/// samples the CPU oracle interpolates — the only fp16 rounding is then the
/// colour output at the working texture, matching the other tap-based kernels.
/// The `textureLoad` axis order `(x=r, y=g, z=b)` mirrors the red-fastest flat
/// index, so no transpose. `bytes_per_row = size*16` (four f32 channels),
/// `rows_per_image = size`, depth = size.
pub fn upload_lut_3d(ctx: &GpuContext, size: u32, data: &[[f32; 3]]) -> wgpu::Texture {
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fx-lut-3d"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: size,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let count = (size as usize)
        .saturating_mul(size as usize)
        .saturating_mul(size as usize);
    let mut rgba = vec![0f32; count * 4];
    for (i, c) in data.iter().take(count).enumerate() {
        rgba[i * 4] = c[0];
        rgba[i * 4 + 1] = c[1];
        rgba[i * 4 + 2] = c[2];
        rgba[i * 4 + 3] = 1.0;
    }
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&rgba),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * 16),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: size,
        },
    );
    tex
}

/// Upload a dense flow field (per-pixel `(u, v)` motion, raster pixels) as a
/// two-channel `rg32float` texture for [`FxEngine::motion_blur`]. `u` and `v`
/// are row-major, one entry per pixel (`w × h`). rg32float, not the working
/// fp16 format, so the kernel reads the exact f32 vectors the CPU oracle
/// integrates — the only fp16 rounding then is the colour taps, matching the
/// other tap-based kernels. Interleaved [u, v] per texel; `textureLoad` in the
/// kernel reads `.xy`.
pub fn upload_flow_field(ctx: &GpuContext, u: &[f32], v: &[f32], w: u32, h: u32) -> wgpu::Texture {
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fx-flow-field"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let n = (w * h) as usize;
    let mut interleaved = vec![0f32; n * 2];
    for i in 0..n.min(u.len()).min(v.len()) {
        interleaved[i * 2] = u[i];
        interleaved[i * 2 + 1] = v[i];
    }
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&interleaved),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 8),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    tex
}

/// Upload a per-pixel depth map (one value per pixel, row-major, `w × h`) as a
/// single-channel `r32float` texture for [`FxEngine::dof`]. r32float, not the
/// working fp16 format, so the kernel reads the exact f32 depths the CPU oracle
/// turns into circle-of-confusion radii — the only fp16 rounding is then the
/// colour taps, matching the flow-field and other tap-based kernels. Values are
/// depth in `[0, 1]` by convention (near..far), but the kernel clamps its ramp
/// so any finite input is defined. `textureLoad` in the kernel reads `.x`;
/// `bytes_per_row = w*4`.
pub fn upload_depth_map(ctx: &GpuContext, depth: &[f32], w: u32, h: u32) -> wgpu::Texture {
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fx-depth-map"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let n = (w * h) as usize;
    let mut data = vec![0f32; n];
    data[..n.min(depth.len())].copy_from_slice(&depth[..n.min(depth.len())]);
    ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    tex
}

/// Read a working (fp16) texture back as linear f32 RGBA — the exact-linear
/// counterpart of `ColourEngine::readback8`, for oracle tests.
pub fn readback_linear_f32(
    ctx: &GpuContext,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Result<Vec<f32>, GpuError> {
    let row_bytes = w * 8;
    let padded = row_bytes.div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fx-readback"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fx-readback-enc"),
        });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|e| GpuError::Readback(e.to_string()))?
        .map_err(|e| GpuError::Readback(e.to_string()))?;
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let row = &data[(y * padded) as usize..(y * padded + row_bytes) as usize];
        for c in row.chunks_exact(2) {
            out.push(f16_to_f32(u16::from_le_bytes([c[0], c[1]])));
        }
    }
    Ok(out)
}

/// f32 → IEEE 754 half bits (the working format's texel channel).
pub fn f16_bits(v: f32) -> u16 {
    half::f16::from_f32(v).to_bits()
}

/// IEEE 754 half bits → f32.
pub fn f16_to_f32(bits: u16) -> f32 {
    half::f16::from_bits(bits).to_f32()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn f16_round_trips_representative_values() {
        for v in [0.0f32, 1.0, -1.0, 0.5, 4.0, 1.5e-5, 65504.0] {
            let rt = f16_to_f32(f16_bits(v));
            assert!((rt - v).abs() <= (v.abs() * 1e-3).max(1e-6), "{v} → {rt}");
        }
    }

    /// The §1.6 oracle corpus: a diagonal gradient, a hard alpha edge down
    /// the middle, and an HDR spike — already fp16-quantised, so comparisons
    /// isolate the kernel maths from upload rounding.
    fn corpus(w: u32, h: u32) -> Vec<f32> {
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let g = (x + y) as f32 / (w + h) as f32;
                let a = if x < w / 2 { 1.0 } else { 0.0 };
                img[i] = g * a;
                img[i + 1] = (1.0 - g) * a;
                img[i + 2] = 0.25 * a;
                img[i + 3] = a;
            }
        }
        let spike = ((10 * w + 20) * 4) as usize;
        img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);
        img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect()
    }

    /// Worst absolute difference between two images.
    fn worst_diff(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max)
    }

    /// Worst distance between two images in fp16 ULPs — the §1.6 metric for
    /// `trivial`/`cheap` effects. Bits are remapped so consecutive integers
    /// are consecutive representable halves (±0 coincide).
    fn worst_f16_ulp(a: &[f32], b: &[f32]) -> i32 {
        fn key(v: f32) -> i32 {
            let bits = i32::from(f16_bits(v));
            if bits & 0x8000 != 0 {
                -(bits & 0x7fff)
            } else {
                bits
            }
        }
        a.iter()
            .zip(b)
            .map(|(x, y)| (key(*x) - key(*y)).abs())
            .fold(0, i32::max)
    }

    /// The §1.6 oracle: the WGSL blur agrees with the CPU reference on a
    /// corpus of gradient + alpha edge + HDR spike, per edge policy — and is
    /// bit-stable against itself (§2.4 determinism).
    #[test]
    fn wgsl_blur_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        // Corpus (docs/08 §1.6): a diagonal gradient, a hard alpha edge down
        // the middle, and an HDR spike.
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let g = (x + y) as f32 / (w + h) as f32;
                let a = if x < w / 2 { 1.0 } else { 0.0 };
                img[i] = g * a;
                img[i + 1] = (1.0 - g) * a;
                img[i + 2] = 0.25 * a;
                img[i + 3] = a;
            }
        }
        let spike = ((10 * w + 20) * 4) as usize;
        img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);

        for edge in [0u32, 1, 2] {
            for (radius, mix) in [(3.0f32, 1.0f32), (7.5, 0.6), (0.0, 1.0)] {
                // fp16 quantise the input exactly as the GPU sees it, so the
                // comparison isolates the blur maths from upload rounding.
                let quantised: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();
                let mut cpu = quantised.clone();
                lumit_core::fx::cpu::blur_gaussian(&mut cpu, w, h, radius, edge, mix);

                let tex = upload_linear_f32(&ctx, &img, w, h);
                let op = BlurOp {
                    radius_px: radius,
                    edge,
                    mix,
                };
                let out = fx.blur(&ctx, &tex, w, h, &op);
                let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

                let worst = cpu
                    .iter()
                    .zip(&gpu)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0f32, f32::max);
                // Moderate-class perceptual epsilon (§1.6), scaled for the
                // HDR corpus: fp16 has ~2^-11 relative steps, and the spike
                // sits at 6.0.
                assert!(
                    worst < 2e-2,
                    "edge {edge} radius {radius} mix {mix}: worst diff {worst}"
                );

                // Determinism: a second run is bit-identical to the first.
                let out2 = fx.blur(&ctx, &tex, w, h, &op);
                let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
                assert_eq!(gpu, gpu2, "GPU blur must be bit-stable");
            }
        }
    }

    /// The §1.6 oracle for sharpen: WGSL agrees with the CPU reference on
    /// the corpus across parameter sweeps, and is bit-stable (§2.4). The
    /// internal gaussian's intermediates round through fp16 textures on the
    /// GPU and stay f32 on the CPU, so the bound is an absolute epsilon:
    /// 5e-3 ≈ 1–2 fp16 ULP at the corpus's HDR peak of 6.0 (measured worst
    /// on NVIDIA: 2.9e-3).
    #[test]
    fn wgsl_sharpen_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (amount, radius, threshold, luma_only, mix) in [
            (0.6f32, 3.0f32, 0.05f32, true, 1.0f32),
            (1.5, 6.0, 0.0, false, 0.7),
            (3.0, 2.0, 0.2, true, 1.0),
            (0.0, 3.0, 0.0, true, 1.0),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::sharpen(&mut cpu, w, h, amount, radius, threshold, luma_only, mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = SharpenOp {
                amount,
                radius_px: radius,
                threshold,
                luma_only,
                mix,
            };
            let out = fx.sharpen(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_diff(&cpu, &gpu);
            // Logged so real cross-vendor deltas accumulate (docs/08 open
            // question 5: the class tolerances are placeholders until then).
            eprintln!("sharpen a={amount} r={radius} t={threshold}: worst {worst:.2e}");
            assert!(
                worst < 5e-3,
                "amount {amount} radius {radius} threshold {threshold} \
                 luma {luma_only} mix {mix}: worst diff {worst}"
            );

            let out2 = fx.sharpen(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU sharpen must be bit-stable");
        }
    }

    /// The §1.6 oracle for RGB split: a cheap pointwise effect, so the CPU
    /// and GPU must agree to ≤ 2 fp16 ULP, and the GPU is bit-stable (§2.4).
    #[test]
    fn wgsl_rgb_split_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (amount, angle, radial, mix) in [
            (3.0f32, 0.0f32, false, 1.0f32),
            (2.5, 33.0, false, 0.6),
            (4.0, 0.0, true, 1.0),
            (0.0, 90.0, false, 1.0),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::rgb_split(&mut cpu, w, h, amount, angle, radial, mix);

            let (dx, dy) = lumit_core::fx::rgb_split_offset(amount, angle);
            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = RgbSplitOp {
                dx,
                dy,
                amount_px: amount,
                radial,
                mix,
            };
            let out = fx.rgb_split(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("rgb split a={amount} ang={angle} radial={radial}: worst {worst} ulp");
            assert!(
                worst <= 2,
                "amount {amount} angle {angle} radial {radial} mix {mix}: \
                 worst {worst} fp16 ULP"
            );

            let out2 = fx.rgb_split(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU rgb split must be bit-stable");
        }
    }

    /// The §1.6 oracle for the RGB split's Wavelength mode (docs/08 §3.6,
    /// K-090): both sides accumulate the same nine host-supplied basis
    /// weights over the same fp16-quantised taps in f32, in the same order,
    /// so the cheap-class ≤ 2 fp16 ULP bound holds despite the longer sum;
    /// the GPU is bit-stable (§2.4). The classic mode's oracle above is
    /// untouched — separate kernel, separate maths.
    #[test]
    fn wgsl_spectral_split_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (amount, angle, radial, mix) in [
            (3.0f32, 0.0f32, false, 1.0f32),
            (2.5, 33.0, false, 0.6),
            (4.0, 0.0, true, 1.0),
            (0.0, 90.0, false, 1.0),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::spectral_split(&mut cpu, w, h, amount, angle, radial, mix);

            let (dx, dy) = lumit_core::fx::rgb_split_offset(amount, angle);
            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = SpectralSplitOp {
                dx,
                dy,
                amount_px: amount,
                radial,
                basis: lumit_core::fx::spectral_basis_vec4(),
                mix,
            };
            let out = fx.spectral_split(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("spectral split a={amount} ang={angle} radial={radial}: worst {worst} ulp");
            assert!(
                worst <= 2,
                "amount {amount} angle {angle} radial {radial} mix {mix}: \
                 worst {worst} fp16 ULP"
            );

            let out2 = fx.spectral_split(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU spectral split must be bit-stable");
        }
    }

    /// The §1.6 oracle for chromatic aberration: a cheap pointwise effect
    /// (a dedicated, always-radial sibling of RGB split's own radial mode),
    /// so the CPU and GPU must agree to ≤ 2 fp16 ULP, and the GPU is
    /// bit-stable (§2.4). Amount 0 is a bit-exact passthrough through the
    /// general formula — no explicit short-circuit, mirroring RGB split's
    /// own un-guarded style (asserted here as it is for RGB split above).
    #[test]
    fn wgsl_chromatic_aberration_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (amount, mix) in [
            (3.0f32, 1.0f32),
            (8.0, 0.6),
            (12.5, 1.0),
            (0.0, 1.0),
            (6.0, 0.0),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::chromatic_aberration(&mut cpu, w, h, amount, mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = ChromaticAberrationOp {
                amount_px: amount,
                mix,
            };
            let out = fx.chromatic_aberration(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("chromatic aberration a={amount} mix={mix}: worst {worst} ulp");
            assert!(
                worst <= 2,
                "amount {amount} mix {mix}: worst {worst} fp16 ULP"
            );
            if amount == 0.0 || mix == 0.0 {
                assert_eq!(
                    gpu, img,
                    "amount 0 or mix 0 must be the bit-exact passthrough"
                );
            }

            let out2 = fx.chromatic_aberration(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU chromatic aberration must be bit-stable");
        }
    }

    /// The §1.6 oracle for flash: a trivial pointwise effect, so the CPU
    /// and GPU must agree to ≤ 2 fp16 ULP, and the GPU is bit-stable (§2.4).
    #[test]
    fn wgsl_flash_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (strength, colour, mix) in [
            (1.0f32, [1.0f32, 1.0, 1.0, 1.0], 1.0f32),
            (0.35, [4.0, 2.0, 1.0, 1.0], 1.0), // HDR flash colour
            (0.8, [1.0, 0.9, 0.7, 1.0], 0.6),
            (0.0, [1.0, 1.0, 1.0, 1.0], 1.0),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::flash(&mut cpu, strength, colour, mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = FlashOp {
                strength,
                colour,
                mix,
            };
            let out = fx.flash(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("flash s={strength} mix={mix}: worst {worst} ulp");
            assert!(
                worst <= 2,
                "strength {strength} mix {mix}: worst {worst} fp16 ULP"
            );

            let out2 = fx.flash(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU flash must be bit-stable");
        }
    }

    /// The §1.6 oracle for colour balance: a cheap pointwise effect, so the
    /// CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4),
    /// and — the K-090 split's promise — a fully neutral balance is the
    /// bit-exact identity on both paths.
    #[test]
    fn wgsl_colour_balance_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        let neutral = ColourBalanceOp {
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
            mix: 1.0,
        };
        let teal_orange = ColourBalanceOp {
            lift: [-0.02, 0.0, 0.02],
            gamma: [1.1, 1.0, 0.9],
            gain: [1.2, 1.0, 0.8],
            mix: 1.0,
        };
        let extreme = ColourBalanceOp {
            lift: [0.1; 3],
            gamma: [2.2, 0.6, 1.7],
            gain: [2.0, 0.5, 1.5],
            mix: 0.7,
        };
        for (name, op) in [
            ("neutral", neutral),
            ("teal-orange", teal_orange),
            ("extreme", extreme),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::colour_balance(&mut cpu, op.lift, op.gamma, op.gain, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.colour_balance(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("colour balance {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "neutral" {
                assert_eq!(gpu, img, "neutral balance must be the bit-exact identity");
            }

            let out2 = fx.colour_balance(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU colour balance must be bit-stable");
        }
    }

    /// The §1.6 oracle for saturation: a cheap pointwise effect, so the CPU
    /// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4),
    /// and saturation 1 is the bit-exact identity on both paths.
    #[test]
    fn wgsl_saturation_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (name, op) in [
            (
                "neutral",
                SaturationOp {
                    saturation: 1.0,
                    mix: 1.0,
                },
            ),
            (
                "greyscale",
                SaturationOp {
                    saturation: 0.0,
                    mix: 1.0,
                },
            ),
            (
                "boosted",
                SaturationOp {
                    saturation: 1.6,
                    mix: 1.0,
                },
            ),
            (
                "mixed",
                SaturationOp {
                    saturation: 0.3,
                    mix: 0.6,
                },
            ),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::saturate(&mut cpu, op.saturation, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.saturation(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("saturation {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "neutral" {
                assert_eq!(
                    gpu, img,
                    "neutral saturation must be the bit-exact identity"
                );
            }

            let out2 = fx.saturation(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU saturation must be bit-stable");
        }
    }

    /// The §1.6 oracle for matte key: a cheap pointwise chroma key, so the CPU
    /// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4), and
    /// Mix 0 is the bit-exact identity on both paths. The corpus mixes
    /// near-key greens, far-from-key colours, partial-alpha (premultiplied)
    /// pixels and an HDR spike, and the settings sweep Tolerance / Softness /
    /// Spill so the smoothstep transition and the despill path are exercised.
    #[test]
    fn wgsl_matte_key_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        // Corpus (§1.6): a green field on the left sliding to red/magenta on
        // the right, brightness rising down the frame, alpha in bands 0.25..1
        // so the unpremultiply round trip is load-bearing, plus an HDR
        // partial-alpha spike.
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let fx_ = x as f32 / (w - 1) as f32;
                let fy = y as f32 / (h - 1) as f32;
                let r = fx_;
                let g = (1.0 - fx_) * (0.4 + 0.6 * fy);
                let b = 0.25 * fx_;
                let a = 0.25 + 0.75 * fy;
                img[i] = r * a;
                img[i + 1] = g * a;
                img[i + 2] = b * a;
                img[i + 3] = a;
            }
        }
        let spike = ((10 * w + 20) * 4) as usize;
        img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 0.5]);
        let img: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();

        let green = [0.0f32, 0.6, 0.0, 1.0];
        for (name, op) in [
            (
                "default",
                MatteKeyOp {
                    key: green,
                    tol: 0.2,
                    soft: 0.1,
                    spill: 0.0,
                    mix: 1.0,
                },
            ),
            (
                "wide+spill",
                MatteKeyOp {
                    key: green,
                    tol: 0.35,
                    soft: 0.2,
                    spill: 0.5,
                    mix: 1.0,
                },
            ),
            (
                "tight+despill",
                MatteKeyOp {
                    key: green,
                    tol: 0.1,
                    soft: 0.05,
                    spill: 1.0,
                    mix: 0.8,
                },
            ),
            (
                "identity_mix0",
                MatteKeyOp {
                    key: green,
                    tol: 0.2,
                    soft: 0.1,
                    spill: 0.3,
                    mix: 0.0,
                },
            ),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::matte_key(&mut cpu, op.key, op.tol, op.soft, op.spill, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.matte_key(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("matte key {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "identity_mix0" {
                assert_eq!(gpu, img, "Mix 0 must be the bit-exact identity");
            }

            let out2 = fx.matte_key(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU matte key must be bit-stable");
        }
    }

    /// The §1.6 oracle for vignette: a cheap pointwise effect, so the CPU
    /// and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable (§2.4), and
    /// Amount 0 (or Mix 0) is the bit-exact identity on both paths.
    #[test]
    fn wgsl_vignette_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (name, op) in [
            (
                "neutral",
                VignetteOp {
                    amount: 0.0,
                    radius: 0.75,
                    softness: 0.5,
                    roundness: 1.0,
                    mix: 1.0,
                },
            ),
            (
                "tight-circular",
                VignetteOp {
                    amount: 1.0,
                    radius: 0.3,
                    softness: 0.1,
                    roundness: 1.0,
                    mix: 1.0,
                },
            ),
            (
                "soft-elliptical",
                VignetteOp {
                    amount: 0.6,
                    radius: 0.5,
                    softness: 0.4,
                    roundness: 0.0,
                    mix: 1.0,
                },
            ),
            (
                "mixed",
                VignetteOp {
                    amount: 0.8,
                    radius: 0.6,
                    softness: 0.3,
                    roundness: 0.5,
                    mix: 0.5,
                },
            ),
            (
                "mix-zero",
                VignetteOp {
                    amount: 0.9,
                    radius: 0.2,
                    softness: 0.05,
                    roundness: 1.0,
                    mix: 0.0,
                },
            ),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::vignette(
                &mut cpu,
                w,
                h,
                op.amount,
                op.radius,
                op.softness,
                op.roundness,
                op.mix,
            );

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.vignette(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("vignette {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "neutral" || name == "mix-zero" {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }

            let out2 = fx.vignette(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU vignette must be bit-stable");
        }
    }

    /// The §1.6 oracle for exposure: a cheap pointwise gain, so CPU and GPU
    /// must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and 0 stops
    /// (`factor` 1.0) or Mix 0 is the bit-exact identity on both paths.
    #[test]
    fn wgsl_exposure_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (name, op) in [
            (
                "neutral",
                ExposureOp {
                    factor: 1.0,
                    mix: 1.0,
                },
            ),
            (
                "brighten",
                ExposureOp {
                    factor: 2.0,
                    mix: 1.0,
                },
            ),
            (
                "darken",
                ExposureOp {
                    factor: 0.5,
                    mix: 1.0,
                },
            ),
            (
                "mixed",
                ExposureOp {
                    factor: 1.7,
                    mix: 0.5,
                },
            ),
            (
                "mix-zero",
                ExposureOp {
                    factor: 3.0,
                    mix: 0.0,
                },
            ),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::exposure(&mut cpu, op.factor, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.exposure(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("exposure {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "neutral" || name == "mix-zero" {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }

            let out2 = fx.exposure(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU exposure must be bit-stable");
        }
    }

    /// The §1.6 oracle for temperature: a cheap pointwise per-channel R/B gain,
    /// so CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and
    /// temperature 0 (gains `(1.0, 1.0)`) or Mix 0 is the bit-exact identity on
    /// both paths. The gains are the host-computed `1 ± 0.5·k` for `k =
    /// temperature / 100`, so the CPU and kernel multiply by identical numbers.
    /// The corpus is seeded with partial-alpha pixels too — unlike Contrast the
    /// multiply commutes with premultiplied alpha (no unpremultiply wrap), and
    /// this pins that: a fractional-alpha pixel comes out identical on both.
    #[test]
    fn wgsl_temperature_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        // Start from the shared corpus (gradient + alpha edge + HDR spike),
        // then inject partial-alpha pixels: straight colour stored
        // premultiplied, quantised to f16 so both paths begin identical.
        let mut img = corpus(w, h);
        let q = |v: f32| f16_to_f32(f16_bits(v));
        let partials = [
            // (straight rgb, alpha)
            ([0.7_f32, 0.3, 0.5], 0.5_f32),
            ([0.2, 0.8, 0.6], 0.25),
            ([0.9, 0.1, 0.4], 0.75),
            ([2.0, 1.0, 0.5], 0.5), // partial-alpha HDR
        ];
        for (n, (rgb, a)) in partials.iter().enumerate() {
            let i = n * 4; // the first four pixels of row 0
            img[i] = q(rgb[0] * a);
            img[i + 1] = q(rgb[1] * a);
            img[i + 2] = q(rgb[2] * a);
            img[i + 3] = q(*a);
        }
        // Host-compute the gains exactly as the resolve step does, over a
        // spread of temperatures (incl. the ±80 the task calls for).
        let gains = |temperature: f32| {
            let k = (temperature / 100.0).clamp(-1.0, 1.0);
            (1.0 + 0.5 * k, 1.0 - 0.5 * k)
        };
        for (name, temp, mix) in [
            ("neutral", 0.0, 1.0),
            ("warm", 80.0, 1.0),
            ("cool", -80.0, 1.0),
            ("mixed", 60.0, 0.5),
            ("mix-zero", 100.0, 0.0),
        ] {
            let (gain_r, gain_b) = gains(temp);
            let op = TemperatureOp {
                gain_r,
                gain_b,
                mix,
            };
            let mut cpu = img.clone();
            lumit_core::fx::cpu::temperature(&mut cpu, op.gain_r, op.gain_b, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.temperature(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("temperature {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "neutral" || name == "mix-zero" {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }

            let out2 = fx.temperature(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU temperature must be bit-stable");
        }
    }

    /// A corpus (§1.6) that seeds the shared gradient + alpha edge + HDR spike
    /// with partial-alpha pixels: straight colour stored premultiplied, quantised
    /// to f16 so both paths begin identical. The unpremultiply round trip is
    /// load-bearing for the affine colour effects (Invert, Tint), so a naive pass
    /// on premultiplied colour would diverge exactly on these pixels.
    fn corpus_with_partials(w: u32, h: u32) -> Vec<f32> {
        let mut img = corpus(w, h);
        let q = |v: f32| f16_to_f32(f16_bits(v));
        let partials = [
            // (straight rgb, alpha)
            ([0.7_f32, 0.3, 0.5], 0.5_f32),
            ([0.2, 0.8, 0.6], 0.25),
            ([0.9, 0.1, 0.4], 0.75),
            ([2.0, 1.0, 0.5], 0.5), // partial-alpha HDR
        ];
        for (n, (rgb, a)) in partials.iter().enumerate() {
            let i = n * 4; // the first four pixels of row 0
            img[i] = q(rgb[0] * a);
            img[i + 1] = q(rgb[1] * a);
            img[i + 2] = q(rgb[2] * a);
            img[i + 3] = q(*a);
        }
        img
    }

    /// The §1.6 oracle for invert: a cheap pointwise colour inverse, so CPU and
    /// GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and Mix 0 is the
    /// bit-exact identity on both paths. The corpus carries partial-alpha pixels
    /// (invert runs on unpremultiplied colour, so the premultiply round trip is
    /// load-bearing) and the HDR spike (which inverts to honest negatives, never
    /// clipped). There is no neutral value, so the only identity case is Mix 0.
    #[test]
    fn wgsl_invert_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus_with_partials(w, h);
        for (name, op) in [
            ("full", InvertOp { mix: 1.0 }),
            ("mixed", InvertOp { mix: 0.5 }),
            ("mix-zero", InvertOp { mix: 0.0 }),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::invert(&mut cpu, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.invert(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("invert {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "mix-zero" {
                assert_eq!(gpu, img, "Mix 0 must be the bit-exact identity");
            }

            let out2 = fx.invert(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU invert must be bit-stable");
        }
    }

    /// The §1.6 oracle for tint: a cheap pointwise luminance duotone, so CPU and
    /// GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and Mix 0 is the
    /// bit-exact identity on both paths. The corpus carries partial-alpha pixels
    /// (the luma-driven remap runs on unpremultiplied colour, so the premultiply
    /// round trip is load-bearing). Settings sweep the default greyscale
    /// (black→black, white→white) and a coloured duotone; the lerp is the
    /// `black + (white − black)·luma` form on both paths so they reduce alike.
    #[test]
    fn wgsl_tint_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus_with_partials(w, h);
        for (name, op) in [
            (
                "greyscale",
                TintOp {
                    black: [0.0, 0.0, 0.0],
                    white: [1.0, 1.0, 1.0],
                    mix: 1.0,
                },
            ),
            (
                "duotone",
                TintOp {
                    black: [0.1, 0.05, 0.3],
                    white: [1.0, 0.9, 0.6],
                    mix: 1.0,
                },
            ),
            (
                "mixed",
                TintOp {
                    black: [0.2, 0.0, 0.4],
                    white: [0.8, 1.0, 0.5],
                    mix: 0.5,
                },
            ),
            (
                "mix-zero",
                TintOp {
                    black: [0.1, 0.05, 0.3],
                    white: [1.0, 0.9, 0.6],
                    mix: 0.0,
                },
            ),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::tint(&mut cpu, op.black, op.white, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.tint(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("tint {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "mix-zero" {
                assert_eq!(gpu, img, "Mix 0 must be the bit-exact identity");
            }

            let out2 = fx.tint(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU tint must be bit-stable");
        }
    }

    /// The §1.6 oracle for contrast: a cheap pointwise affine grade about
    /// mid-grey, so CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is
    /// bit-stable, and Contrast 100 % (`k` 1.0) or Mix 0 is the bit-exact
    /// identity on both paths. The corpus is seeded with partial-alpha pixels
    /// (straight colour × alpha), since the affine grade runs on
    /// unpremultiplied colour and the − pivot offset makes the premultiply
    /// round trip load-bearing — a naive grade on premultiplied colour would
    /// diverge exactly there.
    #[test]
    fn wgsl_contrast_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        // Start from the shared corpus (gradient + alpha edge + HDR spike),
        // then inject partial-alpha pixels: straight colour graded, stored
        // premultiplied, quantised to f16 so both paths begin identical.
        let mut img = corpus(w, h);
        let q = |v: f32| f16_to_f32(f16_bits(v));
        let partials = [
            // (straight rgb, alpha)
            ([0.7_f32, 0.3, 0.5], 0.5_f32),
            ([0.2, 0.8, 0.6], 0.25),
            ([0.9, 0.1, 0.4], 0.75),
            ([2.0, 1.0, 0.5], 0.5), // partial-alpha HDR
        ];
        for (n, (rgb, a)) in partials.iter().enumerate() {
            let i = n * 4; // the first four pixels of row 0
            img[i] = q(rgb[0] * a);
            img[i + 1] = q(rgb[1] * a);
            img[i + 2] = q(rgb[2] * a);
            img[i + 3] = q(*a);
        }
        for (name, op) in [
            ("neutral", ContrastOp { k: 1.0, mix: 1.0 }),
            ("boosted", ContrastOp { k: 1.8, mix: 1.0 }),
            ("flattened", ContrastOp { k: 0.4, mix: 1.0 }),
            ("mixed", ContrastOp { k: 1.5, mix: 0.6 }),
            ("mix-zero", ContrastOp { k: 2.0, mix: 0.0 }),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::contrast(&mut cpu, op.k, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.contrast(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("contrast {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "neutral" || name == "mix-zero" {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }

            let out2 = fx.contrast(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU contrast must be bit-stable");
        }
    }

    /// The §1.6 oracle for gamma: a cheap pointwise power curve, so CPU and GPU
    /// must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and gamma 1.0 or Mix 0
    /// is the bit-exact identity on both paths. Like Contrast, the corpus is
    /// seeded with partial-alpha pixels (straight colour × alpha), since the
    /// curve runs on unpremultiplied colour and the premultiply round trip is
    /// load-bearing — a naive curve on premultiplied colour would diverge there.
    #[test]
    fn wgsl_gamma_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        // Start from the shared corpus (gradient + alpha edge + HDR spike),
        // then inject partial-alpha pixels: straight colour curved, stored
        // premultiplied, quantised to f16 so both paths begin identical.
        let mut img = corpus(w, h);
        let q = |v: f32| f16_to_f32(f16_bits(v));
        let partials = [
            // (straight rgb, alpha)
            ([0.7_f32, 0.3, 0.5], 0.5_f32),
            ([0.2, 0.8, 0.6], 0.25),
            ([0.9, 0.1, 0.4], 0.75),
            ([2.0, 1.0, 0.5], 0.5), // partial-alpha HDR
        ];
        for (n, (rgb, a)) in partials.iter().enumerate() {
            let i = n * 4; // the first four pixels of row 0
            img[i] = q(rgb[0] * a);
            img[i + 1] = q(rgb[1] * a);
            img[i + 2] = q(rgb[2] * a);
            img[i + 3] = q(*a);
        }
        for (name, op) in [
            (
                "neutral",
                GammaOp {
                    gamma: 1.0,
                    mix: 1.0,
                },
            ),
            (
                "encode",
                GammaOp {
                    gamma: 0.45,
                    mix: 1.0,
                },
            ),
            (
                "decode",
                GammaOp {
                    gamma: 2.2,
                    mix: 1.0,
                },
            ),
            (
                "mixed",
                GammaOp {
                    gamma: 2.2,
                    mix: 0.6,
                },
            ),
            (
                "mix-zero",
                GammaOp {
                    gamma: 2.2,
                    mix: 0.0,
                },
            ),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::gamma(&mut cpu, op.gamma, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.gamma(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("gamma {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "neutral" || name == "mix-zero" {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }

            let out2 = fx.gamma(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU gamma must be bit-stable");
        }
    }

    /// The §1.6 oracle for hue shift: a cheap pointwise colour-matrix product,
    /// so CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable, and
    /// 0° (the identity matrix) or Mix 0 is the bit-exact identity on both.
    #[test]
    fn wgsl_hue_shift_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (name, deg, mix) in [
            ("neutral", 0.0, 1.0),
            ("quarter", 90.0, 1.0),
            ("half", 180.0, 1.0),
            ("mixed", 45.0, 0.5),
            ("mix-zero", 120.0, 0.0),
        ] {
            let op = HueShiftOp {
                m: lumit_core::fx::hue_matrix(deg),
                mix,
            };
            let mut cpu = img.clone();
            lumit_core::fx::cpu::hue_shift(&mut cpu, op.m, op.mix);

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let out = fx.hue_shift(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("hue_shift {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "neutral" || name == "mix-zero" {
                assert_eq!(gpu, img, "{name}: must be the bit-exact identity");
            }

            let out2 = fx.hue_shift(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU hue shift must be bit-stable");
        }
    }

    /// The §1.6 oracle for the transform effect: a trivial one-tap resample,
    /// so the CPU and GPU must agree to ≤ 2 fp16 ULP, the GPU is bit-stable
    /// (§2.4), and — the docs/08 §3.5 pin — identity parameters reproduce
    /// the input bit-exactly.
    #[test]
    fn wgsl_transform_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        let centre = [w as f32 * 0.5, h as f32 * 0.5];
        for (name, anchor, position, scale, rotation, opacity, mix) in [
            ("identity", [0.0; 2], [0.0; 2], [1.0; 2], 0.0, 1.0, 1.0),
            ("shift", [0.0; 2], [2.5, -1.5], [1.0; 2], 0.0, 1.0, 1.0),
            ("punch-in", centre, centre, [1.4, 1.4], 12.0, 1.0, 1.0),
            ("flip-fade", centre, centre, [-1.0, 1.0], 0.0, 0.5, 0.8),
            ("collapsed", centre, centre, [0.0, 1.0], 0.0, 1.0, 0.6),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::transform(
                &mut cpu, w, h, anchor, position, scale, rotation, opacity, mix,
            );

            let (m, off, opacity) =
                lumit_core::fx::transform_op(anchor, position, scale, rotation, opacity);
            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = TransformOp {
                m,
                off,
                opacity,
                mix,
            };
            let out = fx.transform(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("transform {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "identity" {
                assert_eq!(
                    gpu, img,
                    "identity transform must be the bit-exact passthrough"
                );
            }

            let out2 = fx.transform(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU transform must be bit-stable");
        }
    }

    /// The §1.6 oracle for shake (docs/08 §3.4): a transform-domain effect
    /// with no kernel of its own — the resolved wobble maps through the
    /// shared `shake_affine` to the Transform kernel, exactly as `run_ops`
    /// dispatches it, and the CPU reference walks the same affine. One-tap
    /// resample, so the cheap-class ≤ 2 fp16 ULP bound holds; the GPU is
    /// bit-stable (§2.4); the neutral wobble (zero amplitude, rotation and
    /// pump — the effect's §1.2 trigger-adjacent neutral) is the bit-exact
    /// passthrough even with auto-scale on, because the cover is exactly 1.
    #[test]
    fn wgsl_shake_matches_the_cpu_oracle_through_the_transform_kernel() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (name, offset, rot, zoom, amp, rot_max, zoom_min, auto_scale, mix) in [
            (
                "neutral",
                [0.0f32, 0.0f32],
                0.0f32,
                1.0f32,
                0.0f32,
                0.0f32,
                1.0f32,
                true,
                1.0f32,
            ),
            ("offset", [2.5, -1.5], 0.0, 1.0, 3.0, 0.0, 1.0, false, 1.0),
            ("twist", [1.0, 0.5], 4.0, 1.0, 1.5, 5.0, 1.0, true, 1.0),
            ("pumped", [0.0, 2.0], -2.0, 0.95, 2.0, 3.0, 0.9, true, 0.7),
        ] {
            let shake = lumit_core::fx::Resolved::Shake {
                offset_px: offset,
                rotation_deg: rot,
                zoom,
                amp_px: amp,
                rotation_max_deg: rot_max,
                zoom_min,
                auto_scale,
                mix,
            };
            let mut cpu = img.clone();
            lumit_core::fx::cpu::apply(&mut cpu, w, h, &shake);

            // The exact run_ops mapping: shared affine → transform op →
            // the Transform kernel.
            let (anchor, position, scale, rotation) = lumit_core::fx::shake_affine(
                w, h, offset, rot, zoom, amp, rot_max, zoom_min, auto_scale,
            );
            let (m, off, opacity) =
                lumit_core::fx::transform_op(anchor, position, scale, rotation, 1.0);
            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = TransformOp {
                m,
                off,
                opacity,
                mix,
            };
            let out = fx.transform(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("shake {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            if name == "neutral" {
                assert_eq!(
                    gpu, img,
                    "a neutral shake must be the bit-exact passthrough"
                );
            }

            let out2 = fx.transform(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU shake must be bit-stable");
        }
    }

    /// The §1.6 oracle for glow: WGSL agrees with the CPU reference on the
    /// corpus across parameter sweeps, is bit-stable (§2.4), and — the
    /// effect's neutral pin — intensity 0 is the bit-exact identity. Like
    /// sharpen, the internal gaussian's intermediates round through fp16
    /// textures on the GPU and stay f32 on the CPU, so the bound is an
    /// absolute epsilon rather than a ULP count: 5e-3 ≈ 1–2 fp16 ULP at the
    /// corpus's HDR peak of 6.0 (measured worst on NVIDIA: 1.5e-3, on the
    /// hard-knee case where the bright stage passes the most energy).
    #[test]
    fn wgsl_glow_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for (name, radius, threshold, knee, intensity, tint, mix) in [
            (
                "default",
                6.0f32,
                1.0f32,
                0.5f32,
                1.0f32,
                [1.0f32; 4],
                1.0f32,
            ),
            ("hard-knee", 4.0, 0.5, 0.0, 2.0, [1.0; 4], 1.0),
            ("threshold-0", 8.0, 0.0, 0.0, 1.0, [1.0; 4], 1.0),
            (
                "tinted-mixed",
                5.0,
                0.3,
                0.2,
                1.5,
                [2.0, 0.5, 0.25, 1.0],
                0.6,
            ),
            ("neutral", 6.0, 1.0, 0.5, 0.0, [1.0; 4], 1.0),
        ] {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::glow(
                &mut cpu, w, h, radius, threshold, knee, intensity, tint, mix,
            );

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = GlowOp {
                radius_px: radius,
                threshold,
                knee,
                intensity,
                tint,
                mix,
            };
            let out = fx.glow(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_diff(&cpu, &gpu);
            // Logged so real cross-vendor deltas accumulate (docs/08 open
            // question 5: the class tolerances are placeholders until then).
            eprintln!("glow {name}: worst {worst:.2e}");
            assert!(worst < 5e-3, "{name}: worst diff {worst}");
            if name == "neutral" {
                assert_eq!(gpu, img, "intensity 0 must be the bit-exact identity");
            }

            let out2 = fx.glow(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU glow must be bit-stable");
        }
    }

    /// The §1.6 oracle for Block glitch (docs/08 §3.12, split out by K-107):
    /// WGSL agrees with the CPU reference across intensity, seed, tick and
    /// the full parameter set, and is bit-stable (§2.4). Mirrors the old
    /// combined Glitch oracle's structure — same maths, just without the
    /// scanline section and its toggle. The per-block hash is exact integer
    /// maths on both sides (`splitmix32`), so the bound stays as tight as
    /// the other hash/tap-based kernels; intensity 0 is asserted bit-exact
    /// against the untouched corpus regardless of Mix, matching the CPU
    /// reference's early return.
    #[test]
    fn wgsl_block_glitch_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);

        struct Case {
            name: &'static str,
            intensity: f32,
            seed: u32,
            tick: i32,
            block_size_px: f32,
            jitter_frac: f32,
            amount_px: f32,
            chan_px: f32,
            slice_frac: f32,
            mix: f32,
        }
        let cases = [
            Case {
                name: "neutral-intensity0",
                intensity: 0.0,
                seed: 7,
                tick: 3,
                block_size_px: 6.0,
                jitter_frac: 0.5,
                amount_px: 5.0,
                chan_px: 2.0,
                slice_frac: 0.5,
                mix: 0.4,
            },
            Case {
                name: "moderate",
                intensity: 0.7,
                seed: 11,
                tick: 4,
                block_size_px: 6.0,
                jitter_frac: 0.3,
                amount_px: 4.0,
                chan_px: 1.5,
                slice_frac: 0.4,
                mix: 1.0,
            },
            Case {
                name: "full-partial-mix",
                intensity: 1.0,
                seed: 99,
                tick: 12,
                block_size_px: 5.0,
                jitter_frac: 1.0,
                amount_px: 8.0,
                chan_px: 3.0,
                slice_frac: 1.0,
                mix: 0.6,
            },
        ];

        for case in cases {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::block_glitch(
                &mut cpu,
                w,
                h,
                case.intensity,
                case.seed,
                case.tick,
                case.block_size_px,
                case.jitter_frac,
                case.amount_px,
                case.chan_px,
                case.slice_frac,
                case.mix,
            );

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = BlockGlitchOp {
                intensity: case.intensity,
                seed: case.seed,
                tick: case.tick,
                block_size_px: case.block_size_px,
                jitter_frac: case.jitter_frac,
                amount_px: case.amount_px,
                chan_px: case.chan_px,
                slice_frac: case.slice_frac,
                mix: case.mix,
            };
            let out = fx.block_glitch(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("block_glitch {}: worst {worst} ulp", case.name);
            assert!(worst <= 2, "{}: worst {worst} fp16 ULP", case.name);
            if case.name == "neutral-intensity0" {
                assert_eq!(gpu, img, "{}: must be the bit-exact passthrough", case.name);
            }

            let out2 = fx.block_glitch(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU block_glitch must be bit-stable");
        }
    }

    /// The §1.6 oracle for Scanlines (docs/08 §3.12, split out by K-107):
    /// WGSL agrees with the CPU reference across intensity, period,
    /// darkness, roll and interlace, and is bit-stable (§2.4). Mirrors the
    /// old combined Glitch oracle's scanline cases — same maths, now a
    /// standalone pointwise pass with no block resample. Intensity 0 is
    /// asserted bit-exact against the untouched corpus regardless of Mix,
    /// matching the CPU reference's early return.
    #[test]
    fn wgsl_scanlines_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);

        struct Case {
            name: &'static str,
            intensity: f32,
            period_px: f32,
            darkness: f32,
            roll_px: f32,
            interlace: bool,
            mix: f32,
        }
        let cases = [
            Case {
                name: "neutral-intensity0",
                intensity: 0.0,
                period_px: 3.0,
                darkness: 0.6,
                roll_px: 1.0,
                interlace: true,
                mix: 0.4,
            },
            Case {
                name: "moderate",
                intensity: 0.8,
                period_px: 4.0,
                darkness: 0.5,
                roll_px: 2.5,
                interlace: true,
                mix: 1.0,
            },
            Case {
                name: "full-partial-mix-no-interlace",
                intensity: 1.0,
                period_px: 2.5,
                darkness: 0.8,
                roll_px: -1.5,
                interlace: false,
                mix: 0.6,
            },
        ];

        for case in cases {
            let mut cpu = img.clone();
            lumit_core::fx::cpu::scanlines(
                &mut cpu,
                w,
                h,
                case.intensity,
                case.period_px,
                case.darkness,
                case.roll_px,
                case.interlace,
                case.mix,
            );

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let op = ScanlinesOp {
                intensity: case.intensity,
                period_px: case.period_px,
                darkness: case.darkness,
                roll_px: case.roll_px,
                interlace: case.interlace,
                mix: case.mix,
            };
            let out = fx.scanlines(&ctx, &tex, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("scanlines {}: worst {worst} ulp", case.name);
            assert!(worst <= 2, "{}: worst {worst} fp16 ULP", case.name);
            if case.name == "neutral-intensity0" {
                assert_eq!(gpu, img, "{}: must be the bit-exact passthrough", case.name);
            }

            let out2 = fx.scanlines(&ctx, &tex, w, h, &op);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU scanlines must be bit-stable");
        }
    }

    /// The §1.6 oracle for the directional blur mode: WGSL agrees with the
    /// CPU reference on the corpus per edge policy, and is bit-stable
    /// (§2.4). Both sides accumulate the same taps in f32 from the same
    /// fp16-quantised input, so the bound is tight even for this
    /// moderate-class kernel; the gaussian mode's own oracle is untouched
    /// above (same kernel, byte-identical maths).
    #[test]
    fn wgsl_dir_blur_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for edge in [0u32, 1, 2] {
            for (length, angle, mix) in
                [(6.0f32, 0.0f32, 1.0f32), (9.5, 33.0, 0.6), (0.0, 90.0, 1.0)]
            {
                let mut cpu = img.clone();
                lumit_core::fx::cpu::blur_directional(&mut cpu, w, h, length, angle, edge, mix);

                let (dx, dy) = lumit_core::fx::rgb_split_offset(1.0, angle);
                let tex = upload_linear_f32(&ctx, &img, w, h);
                let op = DirBlurOp {
                    dx,
                    dy,
                    length_px: length,
                    taps: lumit_core::fx::cpu::dir_blur_taps(length),
                    edge,
                    mix,
                };
                let out = fx.dir_blur(&ctx, &tex, w, h, &op);
                let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

                let worst = worst_f16_ulp(&cpu, &gpu);
                eprintln!("dir blur e={edge} l={length} a={angle}: worst {worst} ulp");
                assert!(
                    worst <= 2,
                    "edge {edge} length {length} angle {angle} mix {mix}: \
                     worst {worst} fp16 ULP"
                );

                let out2 = fx.dir_blur(&ctx, &tex, w, h, &op);
                let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
                assert_eq!(gpu, gpu2, "GPU directional blur must be bit-stable");
            }
        }
    }

    /// The §1.6 oracle for Blur's Radial mode (docs/08 §3.8, schema status
    /// note): WGSL agrees with the CPU reference across Spin and Zoom,
    /// off-centre Centres, several amounts and edge policies, and is
    /// bit-stable (§2.4). Neither side runs a per-tap trig call or a
    /// division (the schema note's whole point), so the bound stays as
    /// tight as the directional blur's; amount 0 is asserted bit-exact
    /// against the untouched corpus (mirroring the directional blur's own
    /// zero-length case) — the gaussian and directional oracles above are
    /// untouched (separate kernels, separate maths, same version).
    #[test]
    fn wgsl_radial_blur_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        for edge in [0u32, 1, 2] {
            for (centre, amount, spin, mix) in [
                ([0.5f32, 0.5f32], 6.0f32, true, 1.0f32),
                ([0.5, 0.5], 6.0, false, 1.0),
                ([0.3, 0.7], 9.5, true, 0.6),
                ([0.3, 0.7], 9.5, false, 0.6),
                ([0.5, 0.5], 0.0, true, 1.0),
            ] {
                let mut cpu = img.clone();
                lumit_core::fx::cpu::blur_radial(&mut cpu, w, h, centre, amount, spin, edge, mix);

                let tex = upload_linear_f32(&ctx, &img, w, h);
                let op = RadialBlurOp {
                    centre_frac: centre,
                    amount_px: amount,
                    taps: lumit_core::fx::cpu::radial_blur_taps(amount),
                    spin,
                    edge,
                    mix,
                };
                let out = fx.radial_blur(&ctx, &tex, w, h, &op);
                let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

                let worst = worst_f16_ulp(&cpu, &gpu);
                eprintln!(
                    "radial blur e={edge} c={centre:?} a={amount} spin={spin}: worst {worst} ulp"
                );
                assert!(
                    worst <= 2,
                    "edge {edge} centre {centre:?} amount {amount} spin {spin} mix {mix}: \
                     worst {worst} fp16 ULP"
                );
                if amount == 0.0 && mix == 1.0 {
                    assert_eq!(gpu, img, "amount 0 must be the bit-exact passthrough");
                }

                let out2 = fx.radial_blur(&ctx, &tex, w, h, &op);
                let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
                assert_eq!(gpu, gpu2, "GPU radial blur must be bit-stable");
            }
        }
    }

    /// The adjustment blend (docs/06 §1.5): out = mix(below, processed,
    /// coverage·opacity) per channel, alpha included — pinned against a CPU
    /// lerp on the corpus, with the end stops bit-exact: zero coverage
    /// returns `below` untouched, full coverage at opacity 1 returns
    /// `processed` untouched.
    #[test]
    fn adjust_blend_lerps_by_coverage_times_opacity() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (48u32, 32u32);
        let below = corpus(w, h);
        // A visibly different "effected" copy (any distinct image works).
        let processed: Vec<f32> = below
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if i % 4 == 3 {
                    *v
                } else {
                    f16_to_f32(f16_bits(1.0 - v * 0.5))
                }
            })
            .collect();
        // Coverage ramps left to right in the alpha channel — the mask
        // raster's shape; colour channels are ignored by the kernel.
        let mut cov = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                cov[i + 3] = f16_to_f32(f16_bits(x as f32 / (w - 1) as f32));
            }
        }
        let tb = upload_linear_f32(&ctx, &below, w, h);
        let tp = upload_linear_f32(&ctx, &processed, w, h);
        let tc = upload_linear_f32(&ctx, &cov, w, h);
        for opacity in [1.0f32, 0.35] {
            let out = fx.adjust_blend(&ctx, &tb, &tp, &tc, w, h, opacity);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let want: Vec<f32> = below
                .iter()
                .zip(&processed)
                .enumerate()
                .map(|(i, (b, p))| {
                    let c = (cov[(i / 4) * 4 + 3] * opacity).clamp(0.0, 1.0);
                    f16_to_f32(f16_bits(b * (1.0 - c) + p * c))
                })
                .collect();
            let worst = worst_f16_ulp(&gpu, &want);
            eprintln!("adjust blend opacity={opacity}: worst {worst} ulp");
            assert!(worst <= 1, "opacity {opacity}: worst {worst} fp16 ULP");

            let out2 = fx.adjust_blend(&ctx, &tb, &tp, &tc, w, h, opacity);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "GPU adjust blend must be bit-stable");
        }
        // End stops: no coverage passes `below` through bit-exactly; full
        // coverage at opacity 1 is `processed` bit-exactly.
        let clear = vec![0.0f32; (w * h * 4) as usize];
        let t0 = upload_linear_f32(&ctx, &clear, w, h);
        let out = fx.adjust_blend(&ctx, &tb, &tp, &t0, w, h, 1.0);
        assert_eq!(
            readback_linear_f32(&ctx, &out, w, h).unwrap(),
            below,
            "zero coverage must be a bit-exact passthrough"
        );
        let full: Vec<f32> = clear
            .iter()
            .enumerate()
            .map(|(i, _)| if i % 4 == 3 { 1.0 } else { 0.0 })
            .collect();
        let t1 = upload_linear_f32(&ctx, &full, w, h);
        let out = fx.adjust_blend(&ctx, &tb, &tp, &t1, w, h, 1.0);
        assert_eq!(
            readback_linear_f32(&ctx, &out, w, h).unwrap(),
            processed,
            "full coverage at opacity 1 must be the processed image bit-exactly"
        );
    }

    /// The §1.6 oracle for Echo (docs/08 §3.13): the GPU chain (an
    /// `echo_accumulate` per tap plus a final `echo_mix`) matches
    /// `lumit_core::fx::cpu::echo` across the three combine modes. Each
    /// accumulate stores an fp16 intermediate where the CPU keeps f32, so a
    /// two-tap sum can drift a little past the pointwise ≤2 ULP — the bound
    /// is stated at 4 ULP with that reason (measured well under it). The GPU
    /// is bit-stable (§2.4); no taps with Mix 1 is a bit-exact passthrough.
    #[test]
    fn wgsl_echo_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let current = corpus(w, h);
        // Two distinct neighbour frames, at offsets -1 and -2.
        let neigh = |scale: f32| -> Vec<f32> {
            current
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if i % 4 == 3 {
                        *v
                    } else {
                        f16_to_f32(f16_bits((v * scale).min(6.0)))
                    }
                })
                .collect()
        };
        let n1 = neigh(0.8);
        let n2 = neigh(0.5);
        let cur_t = upload_linear_f32(&ctx, &current, w, h);
        let n1_t = upload_linear_f32(&ctx, &n1, w, h);
        let n2_t = upload_linear_f32(&ctx, &n2, w, h);
        let gpu_neighbours: [(i32, &wgpu::Texture); 2] = [(-1, &n1_t), (-2, &n2_t)];
        let cpu_neighbours: [(i32, &[f32]); 2] = [(-1, &n1), (-2, &n2)];

        for (weights, mode, mix) in [
            ([0.6f32, 0.3, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0u32, 1.0f32),
            ([0.7, 0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 1, 0.8),
            ([0.9, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 2, 1.0),
        ] {
            let cpu = lumit_core::fx::cpu::echo(&current, &cpu_neighbours, weights, mode, mix);
            let op = EchoOp { weights, mode, mix };
            let out = fx.echo(&ctx, &cur_t, &gpu_neighbours, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("echo mode={mode} mix={mix}: worst {worst} ulp");
            assert!(worst <= 4, "mode {mode} mix {mix}: worst {worst} fp16 ULP");
            let out2 = fx.echo(&ctx, &cur_t, &gpu_neighbours, w, h, &op);
            assert_eq!(
                gpu,
                readback_linear_f32(&ctx, &out2, w, h).unwrap(),
                "GPU echo must be bit-stable"
            );
        }
        // No taps, Mix 1: the accumulator is the current frame and the mix is
        // identity, so the output is the current frame bit-exactly.
        let out = fx.echo(
            &ctx,
            &cur_t,
            &gpu_neighbours,
            w,
            h,
            &EchoOp {
                weights: [0.0; 8],
                mode: 0,
                mix: 1.0,
            },
        );
        assert_eq!(
            readback_linear_f32(&ctx, &out, w, h).unwrap(),
            current,
            "no taps at Mix 1 must be a bit-exact passthrough"
        );
    }

    /// The §1.6 oracle for Flow motion blur (docs/08 §3.2): the GPU smear
    /// matches `lumit_core::fx::cpu::motion_blur` given the same flow field,
    /// on a constant-motion field and a varying one. Both accumulate the taps
    /// in f32 and read the same fp16 source and the same exact (rg32float)
    /// flow vectors, so — exactly like the Directional/Radial blur oracles it
    /// shares its tap-integral shape with — it holds to the cheap-class ≤ 2
    /// fp16 ULP bound despite the multi-tap sum (measured worst: 1 ULP). The
    /// GPU is bit-stable (§2.4); a zero flow and a zero shutter are both
    /// bit-exact passthroughs.
    #[test]
    fn wgsl_motion_blur_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        let src = upload_linear_f32(&ctx, &img, w, h);
        let n = (w * h) as usize;

        // A constant horizontal motion, and a smoothly varying field (per-pixel
        // direction and magnitude) — the two shapes the kernel must handle.
        let constant: (Vec<f32>, Vec<f32>) = (vec![5.0; n], vec![0.0; n]);
        let mut vary_u = vec![0f32; n];
        let mut vary_v = vec![0f32; n];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                vary_u[i] = (y as f32 - h as f32 / 2.0) * 0.25;
                vary_v[i] = (x as f32 - w as f32 / 2.0) * 0.2;
            }
        }
        let varying = (vary_u, vary_v);

        let cases = [
            (&constant, 0.5f32, 16i32, 1.0f32, "constant"),
            (&varying, 1.0, 12, 0.7, "varying"),
            (&constant, 0.25, 8, 1.0, "short"),
        ];
        for (field, shutter_frac, samples, mix, name) in cases {
            let (u, v) = field;
            let mut cpu = img.clone();
            lumit_core::fx::cpu::motion_blur(&mut cpu, w, h, u, v, shutter_frac, samples, mix);
            let flow_t = upload_flow_field(&ctx, u, v, w, h);
            let op = MotionBlurOp {
                shutter_frac,
                samples,
                mix,
            };
            let out = fx.motion_blur(&ctx, &src, &flow_t, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("motion blur {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            let out2 = fx.motion_blur(&ctx, &src, &flow_t, w, h, &op);
            assert_eq!(
                gpu,
                readback_linear_f32(&ctx, &out2, w, h).unwrap(),
                "GPU motion blur must be bit-stable"
            );
        }

        // A zero flow, and a real motion with a closed shutter, are both
        // bit-exact passthroughs (every tap collapses onto the pixel itself).
        let zero = upload_flow_field(&ctx, &vec![0.0; n], &vec![0.0; n], w, h);
        let out = fx.motion_blur(
            &ctx,
            &src,
            &zero,
            w,
            h,
            &MotionBlurOp {
                shutter_frac: 0.5,
                samples: 16,
                mix: 1.0,
            },
        );
        assert_eq!(
            readback_linear_f32(&ctx, &out, w, h).unwrap(),
            img,
            "zero flow must be a bit-exact passthrough"
        );
        let moving = upload_flow_field(&ctx, &constant.0, &constant.1, w, h);
        let out = fx.motion_blur(
            &ctx,
            &src,
            &moving,
            w,
            h,
            &MotionBlurOp {
                shutter_frac: 0.0,
                samples: 16,
                mix: 1.0,
            },
        );
        assert_eq!(
            readback_linear_f32(&ctx, &out, w, h).unwrap(),
            img,
            "a closed shutter must be a bit-exact passthrough"
        );
    }

    /// The §1.6 oracle for Datamosh (docs/08 §3.12, K-104; its own effect
    /// since K-107): the GPU single-tap warp matches
    /// `lumit_core::fx::cpu::datamosh` given the same -1 neighbour and flow
    /// field, on a constant field and a varying one — the same two shapes
    /// [`wgsl_motion_blur_matches_the_cpu_oracle`] exercises, since both
    /// kernels read flow the same way. One bilinear tap, no multi-tap sum,
    /// so it holds to the same ≤ 2 fp16 ULP cheap-class bound. The GPU is
    /// bit-stable (§2.4); Intensity 0 is a bit-exact passthrough.
    #[test]
    fn wgsl_datamosh_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let current = corpus(w, h);
        // A distinct -1 neighbour: the alpha channel carried through (as Echo's
        // oracle does), colour channels scaled and requantised to fp16.
        let prev: Vec<f32> = current
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if i % 4 == 3 {
                    *v
                } else {
                    f16_to_f32(f16_bits((v * 0.6 + 0.05).min(6.0)))
                }
            })
            .collect();
        let cur_t = upload_linear_f32(&ctx, &current, w, h);
        let prev_t = upload_linear_f32(&ctx, &prev, w, h);
        let n = (w * h) as usize;

        let constant: (Vec<f32>, Vec<f32>) = (vec![-4.0; n], vec![2.0; n]);
        let mut vary_u = vec![0f32; n];
        let mut vary_v = vec![0f32; n];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                vary_u[i] = (x as f32 - w as f32 / 2.0) * 0.3;
                vary_v[i] = (y as f32 - h as f32 / 2.0) * 0.25;
            }
        }
        let varying = (vary_u, vary_v);

        for (field, intensity, name) in [
            (&constant, 1.0f32, "constant"),
            (&varying, 0.6, "varying"),
            (&constant, 0.35, "partial mix"),
        ] {
            let (u, v) = field;
            let cpu = lumit_core::fx::cpu::datamosh(&current, &prev, w, h, u, v, intensity);
            let flow_t = upload_flow_field(&ctx, u, v, w, h);
            let op = DatamoshOp { intensity };
            let out = fx.datamosh(&ctx, &cur_t, &prev_t, &flow_t, w, h, &op);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("datamosh {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            let out2 = fx.datamosh(&ctx, &cur_t, &prev_t, &flow_t, w, h, &op);
            assert_eq!(
                gpu,
                readback_linear_f32(&ctx, &out2, w, h).unwrap(),
                "GPU datamosh must be bit-stable"
            );
        }

        // Intensity 0 must be a bit-exact passthrough regardless of motion.
        let moving = upload_flow_field(&ctx, &constant.0, &constant.1, w, h);
        let out = fx.datamosh(
            &ctx,
            &cur_t,
            &prev_t,
            &moving,
            w,
            h,
            &DatamoshOp { intensity: 0.0 },
        );
        assert_eq!(
            readback_linear_f32(&ctx, &out, w, h).unwrap(),
            current,
            "intensity 0 must be a bit-exact passthrough"
        );
    }

    /// Build a `Lut3d` (domain 0..1) by mapping each grid point through `f`,
    /// pushed **red-fastest** (index `r + g*size + b*size*size`) — the layout
    /// `upload_lut_3d` and the shader assume.
    fn build_lut(size: usize, f: impl Fn([f32; 3]) -> [f32; 3]) -> lumit_core::lut::Lut3d {
        let maxf = (size - 1) as f32;
        let mut data = Vec::with_capacity(size * size * size);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    data.push(f([r as f32 / maxf, g as f32 / maxf, b as f32 / maxf]));
                }
            }
        }
        lumit_core::lut::Lut3d {
            size,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            data,
        }
    }

    /// The §1.6 oracle for the 3D LUT (docs/08 §3.11; docs/impl/lut.md): the
    /// WGSL manual-trilinear lookup matches `lumit_core::lut::Lut3d::sample`
    /// wrapped as unpremultiply -> sample -> re-premultiply -> Mix, on a spread
    /// of RGBA pixels **including partial-alpha and out-of-domain HDR ones** and
    /// several cubes (identity, a per-channel gamma, an R/B swap). A cheap
    /// pointwise effect, so CPU and GPU agree to ≤ 2 fp16 ULP; the GPU is
    /// bit-stable (§2.4); Mix 0 is the bit-exact input; and the identity cube
    /// round-trips every in-domain pixel to itself (a strong end-to-end check
    /// that the red-fastest indexing, the domain scale and the premult handling
    /// are all right — if it did not, one of those three is wrong).
    #[test]
    fn wgsl_lut_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);

        // A premultiplied corpus built from a known *straight* colour and an
        // alpha that cycles through 0, partial and 1, so unpremultiply -> look
        // up -> re-premultiply is exercised at every alpha. A couple of pixels
        // carry straight colour > 1.0 to hit the out-of-domain edge clamp.
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let s = [
                    x as f32 / (w - 1) as f32,
                    y as f32 / (h - 1) as f32,
                    (x + y) as f32 / (w + h) as f32,
                ];
                let a = match (x + y) % 4 {
                    0 => 0.0,
                    1 => 0.25,
                    2 => 0.5,
                    _ => 1.0,
                };
                img[i] = s[0] * a;
                img[i + 1] = s[1] * a;
                img[i + 2] = s[2] * a;
                img[i + 3] = a;
            }
        }
        // Out-of-domain straight colours (alpha 1): must clamp on both paths.
        img[((5 * w + 7) * 4) as usize..((5 * w + 7) * 4 + 4) as usize]
            .copy_from_slice(&[1.5, 0.2, 2.0, 1.0]);
        img[((9 * w + 3) * 4) as usize..((9 * w + 3) * 4 + 4) as usize]
            .copy_from_slice(&[3.0, 4.0, 0.1, 1.0]);
        // fp16-quantise exactly as the GPU sees it, so the comparison isolates
        // the LUT maths from upload rounding.
        let img: Vec<f32> = img.iter().map(|v| f16_to_f32(f16_bits(*v))).collect();

        let unpremult = |c: [f32; 4]| -> [f32; 3] {
            if c[3] > 0.0 {
                [c[0] / c[3], c[1] / c[3], c[2] / c[3]]
            } else {
                [0.0; 3]
            }
        };

        let identity = build_lut(3, |c| c);
        // A per-channel gamma (a real, non-linear "film" curve); trilinear is
        // approximate for it, but both paths use the *same* cube, so they still
        // agree — the point is the interpolation maths, not the cube's fidelity.
        let gamma = build_lut(5, |c| [c[0].powf(2.0), c[1].powf(0.5), c[2].powf(1.5)]);
        // A non-separable swap of red and blue: out = [b, g, r].
        let swap = build_lut(2, |c| [c[2], c[1], c[0]]);

        let cases: [(&str, &lumit_core::lut::Lut3d, f32); 5] = [
            ("identity-full", &identity, 1.0),
            ("identity-mix0", &identity, 0.0),
            ("gamma-full", &gamma, 1.0),
            ("gamma-mixed", &gamma, 0.5),
            ("swap-rb", &swap, 1.0),
        ];

        for (name, lut, mix) in cases {
            // CPU expected: unpremultiply -> Lut3d::sample -> re-premultiply ->
            // Mix, using the same lerp form the shader uses for the final blend.
            let mut cpu = vec![0.0f32; img.len()];
            for px in 0..(w * h) as usize {
                let i = px * 4;
                let o = [img[i], img[i + 1], img[i + 2], img[i + 3]];
                let graded = lut.sample(unpremult(o));
                let pm = [graded[0] * o[3], graded[1] * o[3], graded[2] * o[3]];
                cpu[i] = o[0] + (pm[0] - o[0]) * mix;
                cpu[i + 1] = o[1] + (pm[1] - o[1]) * mix;
                cpu[i + 2] = o[2] + (pm[2] - o[2]) * mix;
                cpu[i + 3] = o[3];
            }

            let tex = upload_linear_f32(&ctx, &img, w, h);
            let lut_tex = upload_lut_3d(&ctx, lut.size as u32, &lut.data);
            let out = fx.lut(&ctx, &tex, w, h, &lut_tex, lut.size as u32, mix);
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();

            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("lut {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");

            if name == "identity-mix0" {
                // Mix 0 is the bit-exact input on the GPU path.
                assert_eq!(gpu, img, "{name}: Mix 0 must be the bit-exact input");
            }

            // Determinism: a second run is bit-identical to the first (§2.4).
            let out2 = fx.lut(&ctx, &tex, w, h, &lut_tex, lut.size as u32, mix);
            let gpu2 = readback_linear_f32(&ctx, &out2, w, h).unwrap();
            assert_eq!(gpu, gpu2, "{name}: GPU LUT must be bit-stable");
        }

        // End-to-end: the identity cube at Mix 1.0 returns every *in-domain*
        // pixel to itself (out-of-domain HDR pixels legitimately clamp, so they
        // are excluded). A transposed cube or a broken premult round-trip would
        // fail this loudly.
        let lut_tex = upload_lut_3d(&ctx, identity.size as u32, &identity.data);
        let tex = upload_linear_f32(&ctx, &img, w, h);
        let gpu = readback_linear_f32(
            &ctx,
            &fx.lut(&ctx, &tex, w, h, &lut_tex, identity.size as u32, 1.0),
            w,
            h,
        )
        .unwrap();
        for px in 0..(w * h) as usize {
            let i = px * 4;
            let o = [img[i], img[i + 1], img[i + 2], img[i + 3]];
            let s = unpremult(o);
            if s.iter().all(|v| (0.0..=1.0).contains(v)) {
                for c in 0..4 {
                    assert!(
                        (gpu[i + c] - img[i + c]).abs() < 5e-3,
                        "identity must round-trip in-domain pixel {px} chan {c}: \
                         {} vs {}",
                        gpu[i + c],
                        img[i + c]
                    );
                }
            }
        }
    }

    /// The CPU oracle for [`FxEngine::dof`]: byte-for-byte the WGSL kernel's
    /// maths (the same CoC ramp with explicit min/max/mul, the same integer
    /// disc taps in the same row-major order, box weighted, edges clamped,
    /// the same `o*(1-mix)+v*mix` final blend). Consumes the fp16-quantised
    /// image and the exact-f32 depth the GPU reads, so the two agree.
    #[allow(clippy::too_many_arguments)]
    fn dof_reference(
        img: &[f32],
        depth: &[f32],
        w: u32,
        h: u32,
        focus: f32,
        range: f32,
        near_aperture: f32,
        far_aperture: f32,
        depth_invert: bool,
        display: u32,
        mix: f32,
    ) -> Vec<f32> {
        let wi = w as i32;
        let hi = h as i32;
        let mut out = vec![0.0f32; img.len()];
        for y in 0..hi {
            for x in 0..wi {
                let pi = (y * wi + x) as usize;
                let oi = pi * 4;
                let raw = depth[pi];
                // Depth invert (swap near and far): the shader's
                // `select(raw, 1 - raw, invert)`, bit-identical here.
                let d = if depth_invert { 1.0 - raw } else { raw };
                let dist = (d - focus).abs();
                let denom = (1.0f32 - range).max(1e-4);
                // clamp(0,1) is bit-identical to the shader's min(max(·,0),1)
                // for every finite input (the ±0 corner collapses to the same
                // smoothstep zero and coincides in fp16), so parity holds.
                let e = ((dist - range) / denom).clamp(0.0, 1.0);
                let s = e * e * (3.0 - 2.0 * e);
                // Diagnostic views (mirror the kernel): write the view directly,
                // ignoring the disc gather and Mix.
                if display == 1 {
                    // Depth map: post-invert depth as opaque greyscale.
                    out[oi] = d;
                    out[oi + 1] = d;
                    out[oi + 2] = d;
                    out[oi + 3] = 1.0;
                    continue;
                }
                if display == 2 {
                    // Focus map: 1 - s, white where sharp.
                    let m = 1.0 - s;
                    out[oi] = m;
                    out[oi + 1] = m;
                    out[oi + 2] = m;
                    out[oi + 3] = 1.0;
                    continue;
                }
                // Per-side aperture: the shader's
                // `select(far, near, d < focus)`, far at equality.
                let ap = if d < focus {
                    near_aperture
                } else {
                    far_aperture
                };
                let coc = ap * s;
                let coc2 = coc * coc;
                let ri = coc.ceil() as i32;
                let mut acc = [0.0f32; 4];
                let mut wsum = 0.0f32;
                for dy in -ri..=ri {
                    for dx in -ri..=ri {
                        let r2 = (dx * dx + dy * dy) as f32;
                        if r2 <= coc2 {
                            let sx = (x + dx).clamp(0, wi - 1);
                            let sy = (y + dy).clamp(0, hi - 1);
                            let si = ((sy * wi + sx) * 4) as usize;
                            acc[0] += img[si];
                            acc[1] += img[si + 1];
                            acc[2] += img[si + 2];
                            acc[3] += img[si + 3];
                            wsum += 1.0;
                        }
                    }
                }
                for c in 0..4 {
                    let v = acc[c] / wsum;
                    let o = img[oi + c];
                    out[oi + c] = o * (1.0 - mix) + v * mix;
                }
            }
        }
        out
    }

    /// The §1.6 oracle for the depth-of-field lens blur (foundation for the
    /// planned DoF effects): the WGSL variable-radius disc blur matches
    /// [`dof_reference`] over a depth ramp and several focus/aperture/mix
    /// settings. A tap-summing gather like Motion blur, reading exact
    /// (r32float) depth and the same fp16 source, so it holds to the cheap-
    /// class ≤ 2 fp16 ULP bound; the GPU is bit-stable (§2.4); Mix 0, a zero
    /// aperture, and a depth that sits everywhere inside the sharp band are
    /// all bit-exact passthroughs.
    #[test]
    fn wgsl_dof_matches_the_cpu_oracle() {
        let Ok(ctx) = GpuContext::headless() else {
            eprintln!("no GPU adapter; skipping WGSL parity test");
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);
        let img = corpus(w, h);
        let src = upload_linear_f32(&ctx, &img, w, h);
        let n = (w * h) as usize;

        // A left-to-right depth ramp: 0 at the left edge, 1 at the right, so
        // the CoC sweeps its whole range across the frame. r32float, uploaded
        // exact — the depth is not fp16-quantised.
        let mut ramp = vec![0f32; n];
        for y in 0..h {
            for x in 0..w {
                ramp[(y * w + x) as usize] = x as f32 / (w - 1) as f32;
            }
        }
        let depth_t = upload_depth_map(&ctx, &ramp, w, h);

        // (focus, range, near, far, invert, display, mix, name). Invert, an
        // asymmetric near/far pair, and every shipped Display mode all stay
        // continuous (the aperture select flips only where s == 0; Depth/Focus
        // maps are smooth in depth), so the cheap-class ≤ 2 fp16 ULP bound holds
        // across modes — none is excluded.
        let cases = [
            (
                0.5f32,
                0.1f32,
                6.0f32,
                6.0f32,
                false,
                0u32,
                1.0f32,
                "centre-focus",
            ),
            (0.0, 0.05, 8.0, 8.0, false, 0, 1.0, "near-focus"),
            (0.5, 0.1, 6.0, 6.0, false, 0, 0.5, "partial mix"),
            (0.5, 0.2, 10.0, 10.0, false, 0, 1.0, "wide aperture"),
            (0.2, 0.1, 8.0, 8.0, true, 0, 1.0, "inverted near-focus"),
            (0.5, 0.1, 6.0, 6.0, true, 0, 1.0, "inverted centre-focus"),
            (0.5, 0.05, 12.0, 3.0, false, 0, 1.0, "asymmetric near>far"),
            (0.5, 0.05, 3.0, 12.0, false, 0, 1.0, "asymmetric far>near"),
            (0.5, 0.05, 12.0, 3.0, true, 0, 1.0, "asymmetric inverted"),
            (0.5, 0.1, 8.0, 8.0, false, 1, 1.0, "depth map"),
            (0.5, 0.1, 8.0, 8.0, true, 1, 1.0, "depth map inverted"),
            (0.5, 0.1, 8.0, 8.0, false, 2, 1.0, "focus map"),
            (0.3, 0.15, 12.0, 4.0, false, 2, 1.0, "focus map asymmetric"),
        ];
        for (focus, range, near, far, invert, display, mix, name) in cases {
            let cpu = dof_reference(
                &img, &ramp, w, h, focus, range, near, far, invert, display, mix,
            );
            let out = fx.dof(
                &ctx, &src, w, h, &depth_t, focus, range, near, far, invert, display, mix,
            );
            let gpu = readback_linear_f32(&ctx, &out, w, h).unwrap();
            let worst = worst_f16_ulp(&cpu, &gpu);
            eprintln!("dof {name}: worst {worst} ulp");
            assert!(worst <= 2, "{name}: worst {worst} fp16 ULP");
            // Determinism (§2.4): a second run is bit-identical to the first.
            let out2 = fx.dof(
                &ctx, &src, w, h, &depth_t, focus, range, near, far, invert, display, mix,
            );
            assert_eq!(
                gpu,
                readback_linear_f32(&ctx, &out2, w, h).unwrap(),
                "{name}: GPU DoF must be bit-stable"
            );
        }

        // Mix 0 is the bit-exact input regardless of depth or aperture (Rendered
        // mode).
        let out = fx.dof(
            &ctx, &src, w, h, &depth_t, 0.5, 0.1, 10.0, 10.0, false, 0, 0.0,
        );
        assert_eq!(
            readback_linear_f32(&ctx, &out, w, h).unwrap(),
            img,
            "Mix 0 must be the bit-exact input"
        );

        // Both apertures zero collapses every disc to the centre tap — a
        // bit-exact passthrough at full Mix, whatever the depth (invert cannot
        // change a zero radius).
        let out = fx.dof(&ctx, &src, w, h, &depth_t, 0.5, 0.1, 0.0, 0.0, true, 0, 1.0);
        assert_eq!(
            readback_linear_f32(&ctx, &out, w, h).unwrap(),
            img,
            "a zero aperture must be a bit-exact passthrough"
        );

        // A depth that sits everywhere inside the sharp band leaves the CoC at
        // zero for every pixel — also a bit-exact passthrough at full Mix,
        // even with large apertures. Inverting a flat 0.5 leaves it in-band.
        let flat = upload_depth_map(&ctx, &vec![0.5f32; n], w, h);
        let out = fx.dof(&ctx, &src, w, h, &flat, 0.5, 0.1, 10.0, 10.0, false, 0, 1.0);
        assert_eq!(
            readback_linear_f32(&ctx, &out, w, h).unwrap(),
            img,
            "an in-band depth must be a bit-exact passthrough"
        );
    }
}
