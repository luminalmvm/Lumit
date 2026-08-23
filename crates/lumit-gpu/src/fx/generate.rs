//! The Generate kernels (docs/08 §3.34–§3.37, K-398): Fill, Gradient, Noise and
//! Fractal noise — the effects that make pixels rather than change them.
//!
//! Each op mirrors its `lumit_core::fx::cpu` parameter struct field-for-field so
//! the kernel and the CPU oracle consume the identical numbers (K-031). Nothing
//! here does arithmetic; every reciprocal, cosine and fraction was taken once,
//! host-side, in the effect's own `packed`.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved Fill (docs/08 §3.34). Mirrors the arguments of
/// `lumit_core::fx::cpu::fill`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FillOp {
    /// Scene-linear RGB; the layer's own alpha supplies the coverage.
    pub colour: [f32; 3],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FillParams {
    colour: [f32; 4],
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved Gradient (docs/08 §3.35). Mirrors
/// `lumit_core::fx::cpu::GradientParams` field-for-field; both reciprocals
/// arrive floored, so a zero-length axis collapses the ramp to one flat colour
/// rather than faulting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientOp {
    pub radial: bool,
    /// Start point in raster pixels.
    pub start: [f32; 2],
    /// `end − start`, raster pixels.
    pub axis: [f32; 2],
    pub inv_len2: f32,
    pub inv_len: f32,
    /// Scene-linear start and end colours.
    pub c0: [f32; 3],
    pub c1: [f32; 3],
    /// 0..1 dither of the ramp position.
    pub scatter: f32,
    pub seed: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GradientParams {
    start_axis: [f32; 4],
    c0: [f32; 4],
    c1: [f32; 4],
    inv_len2: f32,
    inv_len: f32,
    scatter: f32,
    mix_amt: f32,
    seed: u32,
    radial: u32,
    _pad0: u32,
    _pad1: u32,
}

/// One resolved Noise (docs/08 §3.36). `tick` arrives already discretised from
/// layer time (and pinned to zero when Animate is off), so the kernel never sees
/// a clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseOp {
    /// Amount ÷ 100.
    pub amount: f32,
    pub gaussian: bool,
    pub colour_noise: bool,
    pub seed: u32,
    pub tick: i32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct NoiseParams {
    amount: f32,
    mix_amt: f32,
    seed: u32,
    tick: i32,
    gaussian: u32,
    colour_noise: u32,
    _pad0: u32,
    _pad1: u32,
}

/// One resolved Fractal noise (docs/08 §3.37). Mirrors
/// `lumit_core::fx::cpu::FractalNoiseParams` with its `FractalField` flattened,
/// which is the shape the uniform wants anyway.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FractalNoiseOp {
    pub seed: u32,
    /// 1..=10.
    pub octaves: u32,
    /// Sub influence ÷ 100.
    pub gain: f32,
    /// 100 ÷ Sub scaling.
    pub lacunarity: f32,
    pub perlin: bool,
    pub turbulent: bool,
    /// Depth loop length in cells; 0 for a field that never repeats.
    pub cycle: i32,
    /// `(cos, sin)` of the Rotation control, host-computed.
    pub cos_sin: [f32; 2],
    /// Field origin in raster pixels.
    pub offset: [f32; 2],
    /// `1 ÷ cell size` per axis, raster pixels.
    pub inv_scale: [f32; 2],
    /// Depth coordinate (Evolution ÷ 360, folded into the cycle).
    pub z: f32,
    /// Contrast ÷ 100.
    pub contrast: f32,
    /// Brightness ÷ 100.
    pub brightness: f32,
    pub invert: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FractalNoiseParams {
    cos_sin_offset: [f32; 4],
    inv_scale_z_contrast: [f32; 4],
    brightness: f32,
    mix_amt: f32,
    gain: f32,
    lacunarity: f32,
    seed: u32,
    octaves: u32,
    cycle: i32,
    flags: u32,
}

impl FxEngine {
    /// Apply one Fill (docs/08 §3.34) to a linear working texture, returning a
    /// new texture of the same size. One pointwise pass on premultiplied colour;
    /// the source colour is never read and alpha passes through.
    pub fn fill(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &FillOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-fill-out");
        self.dispatch(
            ctx,
            &self.fill,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&FillParams {
                colour: [op.colour[0], op.colour[1], op.colour[2], 0.0],
                mix_amt: op.mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one Gradient (docs/08 §3.35) to a linear working texture, returning
    /// a new texture of the same size. One pointwise pass: the ramp position,
    /// the optional scatter, and the two-colour interpolation in the working
    /// space.
    pub fn gradient(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &GradientOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-gradient-out");
        self.dispatch(
            ctx,
            &self.gradient,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&GradientParams {
                start_axis: [op.start[0], op.start[1], op.axis[0], op.axis[1]],
                c0: [op.c0[0], op.c0[1], op.c0[2], 0.0],
                c1: [op.c1[0], op.c1[1], op.c1[2], 0.0],
                inv_len2: op.inv_len2,
                inv_len: op.inv_len,
                scatter: op.scatter,
                mix_amt: op.mix,
                seed: op.seed,
                radial: u32::from(op.radial),
                _pad0: 0,
                _pad1: 0,
            }),
        );
        out
    }

    /// Apply one Noise (docs/08 §3.36) to a linear working texture, returning a
    /// new texture of the same size. One pointwise pass; the §2.2 unpremultiply
    /// wrap is fused into the kernel, and Amount 0 short-circuits inside it.
    pub fn noise(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &NoiseOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-noise-out");
        self.dispatch(
            ctx,
            &self.noise,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&NoiseParams {
                amount: op.amount,
                mix_amt: op.mix,
                seed: op.seed,
                tick: op.tick,
                gaussian: u32::from(op.gaussian),
                colour_noise: u32::from(op.colour_noise),
                _pad0: 0,
                _pad1: 0,
            }),
        );
        out
    }

    /// Apply one Fractal noise (docs/08 §3.37) to a linear working texture,
    /// returning a new texture of the same size. One pass of up to ten octaves
    /// of 3-D value or Perlin noise per pixel — no neighbour taps, so the ROI is
    /// exact despite the cost.
    pub fn fractal_noise(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &FractalNoiseOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-fractal-noise-out");
        // bit 0 Perlin, bit 1 Turbulent, bit 2 Invert — three switches in one
        // lane rather than three u32s and a pad.
        let flags =
            u32::from(op.perlin) | (u32::from(op.turbulent) << 1) | (u32::from(op.invert) << 2);
        self.dispatch(
            ctx,
            &self.fractal_noise,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&FractalNoiseParams {
                cos_sin_offset: [op.cos_sin[0], op.cos_sin[1], op.offset[0], op.offset[1]],
                inv_scale_z_contrast: [op.inv_scale[0], op.inv_scale[1], op.z, op.contrast],
                brightness: op.brightness,
                mix_amt: op.mix,
                gain: op.gain,
                lacunarity: op.lacunarity,
                seed: op.seed,
                octaves: op.octaves,
                cycle: op.cycle,
                flags,
            }),
        );
        out
    }
}

/// The most segments a Lightning bolt may occupy (docs/08 §3.74's first
/// decision). Declared here as well as in `lumit_core::fx::cpu` because this
/// crate does not depend on that one at build time; the two are pinned equal by
/// `the_lightning_segment_cap_matches_the_core` in the fx tests.
pub const LIGHTNING_SEGMENTS: usize = 192;

/// One resolved Beam (docs/08 §3.73). Mirrors `lumit_core::fx::cpu::BeamParams`
/// field-for-field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeamOp {
    /// Where the beam starts, raster pixels.
    pub start: [f32; 2],
    /// `End − Start`, raster pixels.
    pub axis: [f32; 2],
    /// `1 ÷ |axis|²`, floored.
    pub inv_len2: f32,
    /// The tail and the head, as fractions of the axis.
    pub u0: f32,
    /// See [`u0`](Self::u0).
    pub u1: f32,
    /// `1 ÷ (u1 − u0)`, floored.
    pub inv_span: f32,
    /// The half-thickness at the tail, raster pixels.
    pub half0: f32,
    /// And at the head.
    pub half1: f32,
    /// Softness ÷ 100, floored above zero.
    pub soft: f32,
    /// The core's colour, scene-linear RGB.
    pub inside: [f32; 3],
    /// The rim's colour, scene-linear RGB.
    pub outside: [f32; 3],
    /// False when the drawn interval is empty (Time 0).
    pub active: bool,
    /// Whether the layer that arrived is kept under the beam.
    pub composite: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BeamParams {
    start_axis: [f32; 4],
    inside: [f32; 4],
    outside: [f32; 4],
    inv_len2: f32,
    u0: f32,
    u1: f32,
    inv_span: f32,
    half0: f32,
    half1: f32,
    soft: f32,
    mix_amt: f32,
    is_active: f32,
    composite: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One resolved Lightning (docs/08 §3.74). Mirrors
/// `lumit_core::fx::cpu::LightningParams` — **the bolt is already built**, which
/// is the whole of that section's first decision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightningOp {
    /// The bolt, as `(ax, ay, bx, by)` in raster pixels.
    pub segments: [[f32; 4]; LIGHTNING_SEGMENTS],
    /// Each segment's brightness, 0..1.
    pub fades: [f32; LIGHTNING_SEGMENTS],
    /// How many of the above are real.
    pub count: u32,
    /// The core's half-width in raster pixels.
    pub core_radius: f32,
    /// The glow's reach in raster pixels.
    pub glow_radius: f32,
    /// Glow opacity ÷ 100.
    pub glow_opacity: f32,
    /// The core's colour, scene-linear RGB.
    pub core_colour: [f32; 3],
    /// The glow's colour, scene-linear RGB.
    pub glow_colour: [f32; 3],
    /// Whether the layer that arrived is kept under the bolt.
    pub composite: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightningParams {
    core_colour: [f32; 4],
    glow_colour: [f32; 4],
    core_radius: f32,
    glow_radius: f32,
    glow_opacity: f32,
    mix_amt: f32,
    count: u32,
    composite: u32,
    /// 1 = the matte scales the bolt's opacity per pixel (K-428).
    matte_on: f32,
    _pad1: u32,
    segs: [[f32; 4]; LIGHTNING_SEGMENTS],
    // Four fades to an element, which is what a uniform array's 16-byte stride
    // costs if they are stored one to an element instead.
    fades: [[f32; 4]; LIGHTNING_SEGMENTS / 4],
}

/// One resolved Radio waves (docs/08 §3.75). Mirrors
/// `lumit_core::fx::cpu::RadioWavesParams` — and note the polygon is already
/// solved into one sector for a **unit** radius, host-side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadioWavesOp {
    /// Where the waves are emitted, raster pixels.
    pub centre: [f32; 2],
    /// The unit shape's first vertex, in the sector's own frame.
    pub vertex: [f32; 2],
    /// The outward unit normal of the edge leaving it.
    pub normal: [f32; 2],
    /// One sector, radians.
    pub period: f32,
    /// Rotation in radians, from straight up, clockwise.
    pub rotation: f32,
    /// Spin in radians per second.
    pub spin: f32,
    /// The newest wave's index, taken host-side (K-399).
    pub newest: i32,
    /// How many waves to walk back from it.
    pub count: i32,
    /// The Time control, seconds.
    pub time: f32,
    /// `1 ÷ Frequency`, seconds between waves.
    pub period_s: f32,
    /// Expansion in raster pixels per second.
    pub expansion: f32,
    /// Lifespan in seconds, floored above zero.
    pub lifespan: f32,
    /// The stroke's half-width in raster pixels.
    pub half_width: f32,
    /// Fade in as a share of the lifespan, floored above zero.
    pub fade_in: f32,
    /// Fade out as a share of the lifespan, floored above zero.
    pub fade_out: f32,
    /// The stroke's colour, scene-linear RGB.
    pub colour: [f32; 3],
    /// Opacity ÷ 100.
    pub opacity: f32,
    /// Whether the layer that arrived is kept under the waves.
    pub composite: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RadioWavesParams {
    centre_vertex: [f32; 4],
    normal_period_rot: [f32; 4],
    spin_time_step_exp: [f32; 4],
    life_half_fades: [f32; 4],
    colour: [f32; 4],
    opacity: f32,
    mix_amt: f32,
    newest: i32,
    count: i32,
    composite: u32,
    /// 1 = the matte scales Opacity per pixel (K-428).
    matte_on: f32,
    _pad1: u32,
    _pad2: u32,
}

/// One resolved Vegas (docs/08 §3.76). Mirrors
/// `lumit_core::fx::cpu::VegasParams` field-for-field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VegasOp {
    /// True to read the alpha rather than the perceptual luma.
    pub from_alpha: bool,
    /// The contour's level, 0..1 in the read value.
    pub level: f32,
    /// The stroke's half-width in raster pixels.
    pub half_width: f32,
    /// The soft band either side of it, raster pixels, floored.
    pub band: f32,
    /// `1 ÷ Segment length`, raster pixels.
    pub inv_segment: f32,
    /// The lit share of one segment; 2 for a continuous outline.
    pub duty: f32,
    /// Rotation in turns.
    pub phase: f32,
    /// The stroke's colour, scene-linear RGB.
    pub colour: [f32; 3],
    /// Opacity ÷ 100.
    pub opacity: f32,
    /// Whether the layer that arrived is kept under the stroke.
    pub composite: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VegasParams {
    colour: [f32; 4],
    level: f32,
    half_width: f32,
    band: f32,
    inv_segment: f32,
    duty: f32,
    phase: f32,
    opacity: f32,
    mix_amt: f32,
    from_alpha: u32,
    composite: u32,
    /// 1 = the matte scales Opacity per pixel (K-428).
    matte_on: f32,
    _pad1: u32,
}

/// The most straight pieces one path-drawn effect may occupy (docs/08 §3.78).
/// Declared here as well as in `lumit_core::fx::cpu` because this crate does not
/// depend on that one at build time; the two are pinned equal by
/// `the_path_piece_cap_matches_the_core` in the fx tests.
pub const PATH_PRIMITIVES: usize = 512;

/// One resolved path drawing (docs/08 §3.78 Scribble, §3.79 Stroke, §3.76
/// Vegas' Mask/Path source). Mirrors `lumit_core::fx::cpu::PathDrawParams`
/// field-for-field — and note **the drawing is already built**, which is what
/// lets one kernel serve all three.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathDrawOp {
    /// The drawing, as `(ax, ay, bx, by)` in raster pixels.
    pub segments: [[f32; 4]; PATH_PRIMITIVES],
    /// How far along the whole drawing each piece's `a` end sits, raster pixels.
    pub arcs: [f32; PATH_PRIMITIVES],
    /// How many of the above are real. Zero draws nothing — the no-op.
    pub count: u32,
    /// Half the drawn width, raster pixels.
    pub half_width: f32,
    /// The soft band either side of it, raster pixels, floored.
    pub band: f32,
    /// `1 ÷ dash length`, raster pixels; 0 for a continuous line.
    pub inv_segment: f32,
    /// The lit share of one dash; 2 for a continuous line.
    pub duty: f32,
    /// The dash's phase in turns.
    pub phase: f32,
    /// How far the paper is displaced, raster pixels; 0 skips the noise.
    pub wiggle_amp: f32,
    /// The wobble's frequency, cells per raster pixel.
    pub wiggle_freq: f32,
    /// Where in the wobble's evolution this frame sits, taken host-side.
    pub wiggle_tick: f32,
    /// The wobble's seed.
    pub seed: u32,
    /// The drawing's colour, scene-linear RGB.
    pub colour: [f32; 3],
    /// Opacity ÷ 100.
    pub opacity: f32,
    /// 0 on the original, 1 on transparent, 2 revealing the original.
    pub style: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PathDrawParams {
    colour: [f32; 4],
    half_width: f32,
    band: f32,
    inv_segment: f32,
    duty: f32,
    phase: f32,
    wiggle_amp: f32,
    wiggle_freq: f32,
    wiggle_tick: f32,
    opacity: f32,
    mix_amt: f32,
    seed: u32,
    count: u32,
    style: u32,
    /// 1 = the matte scales Opacity per pixel (K-428).
    matte_on: f32,
    _pad1: u32,
    _pad2: u32,
    segs: [[f32; 4]; PATH_PRIMITIVES],
    // Four distances-along to an element, which is what a uniform array's
    // 16-byte stride costs if they are stored one to an element instead.
    arcs: [[f32; 4]; PATH_PRIMITIVES / 4],
}

/// One resolved Add grain (docs/08 §3.77). Mirrors
/// `lumit_core::fx::cpu::AddGrainParams` field-for-field; `tick` arrives already
/// discretised from layer time (and pinned to zero when Animate is off), so the
/// kernel never sees a clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AddGrainOp {
    /// The grain's amplitude in perceptual units, per channel.
    pub amplitude: [f32; 3],
    /// `1 ÷ Size`, raster pixels.
    pub inv_size: f32,
    /// Softness ÷ 100.
    pub softness: f32,
    /// The three tonal weights, each ÷ 100.
    pub tonal: [f32; 3],
    /// True to read one lane for all three channels.
    pub monochrome: bool,
    pub seed: u32,
    /// The frame's draw, zero when Animate is off.
    pub tick: i32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct AddGrainParams {
    amplitude: [f32; 4],
    tonal: [f32; 4],
    inv_size: f32,
    softness: f32,
    mix_amt: f32,
    _pad0: f32,
    seed: u32,
    tick: i32,
    monochrome: u32,
    /// 1 = the matte scales Intensity per pixel (K-428).
    matte_on: f32,
}

impl FxEngine {
    /// Apply one Beam (docs/08 §3.73) to a linear working texture, returning a
    /// new texture of the same size. One pass: one capsule distance a pixel.
    pub fn beam(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &BeamOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-beam-out");
        self.dispatch(
            ctx,
            &self.beam,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BeamParams {
                start_axis: [op.start[0], op.start[1], op.axis[0], op.axis[1]],
                inside: [op.inside[0], op.inside[1], op.inside[2], 0.0],
                outside: [op.outside[0], op.outside[1], op.outside[2], 0.0],
                inv_len2: op.inv_len2,
                u0: op.u0,
                u1: op.u1,
                inv_span: op.inv_span,
                half0: op.half0,
                half1: op.half1,
                soft: op.soft,
                mix_amt: op.mix,
                is_active: f32::from(u8::from(op.active)),
                composite: f32::from(u8::from(op.composite)),
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one Lightning (docs/08 §3.74) to a linear working texture,
    /// returning a new texture of the same size. One pass of up to 192 capsule
    /// distances a pixel; the bolt itself was built host-side and arrives in the
    /// uniform.
    pub fn lightning(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &LightningOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-lightning-out");
        let mut fades = [[0.0f32; 4]; LIGHTNING_SEGMENTS / 4];
        for (i, f) in op.fades.iter().enumerate() {
            fades[i / 4][i % 4] = *f;
        }
        self.dispatch_matted(
            ctx,
            &self.lightning,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&LightningParams {
                core_colour: [op.core_colour[0], op.core_colour[1], op.core_colour[2], 0.0],
                glow_colour: [op.glow_colour[0], op.glow_colour[1], op.glow_colour[2], 0.0],
                core_radius: op.core_radius,
                glow_radius: op.glow_radius,
                glow_opacity: op.glow_opacity,
                mix_amt: op.mix,
                count: op.count,
                composite: u32::from(op.composite),
                matte_on: f32::from(matte.is_some()),
                _pad1: 0,
                segs: op.segments,
                fades,
            }),
        );
        out
    }

    /// Apply one Radio waves (docs/08 §3.75) to a linear working texture,
    /// returning a new texture of the same size. One pass: one `atan2` and up to
    /// 32 rings a pixel.
    pub fn radio_waves(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &RadioWavesOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-radio-waves-out");
        self.dispatch_matted(
            ctx,
            &self.radio_waves,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&RadioWavesParams {
                centre_vertex: [op.centre[0], op.centre[1], op.vertex[0], op.vertex[1]],
                normal_period_rot: [op.normal[0], op.normal[1], op.period, op.rotation],
                spin_time_step_exp: [op.spin, op.time, op.period_s, op.expansion],
                life_half_fades: [op.lifespan, op.half_width, op.fade_in, op.fade_out],
                colour: [op.colour[0], op.colour[1], op.colour[2], 0.0],
                opacity: op.opacity,
                mix_amt: op.mix,
                newest: op.newest,
                count: op.count,
                composite: u32::from(op.composite),
                matte_on: f32::from(matte.is_some()),
                _pad1: 0,
                _pad2: 0,
            }),
        );
        out
    }

    /// Apply one Vegas (docs/08 §3.76) to a linear working texture, returning a
    /// new texture of the same size. One pass of a separable 5×5 Sobel and the
    /// stroke it decides.
    pub fn vegas(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &VegasOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-vegas-out");
        self.dispatch_matted(
            ctx,
            &self.vegas,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&VegasParams {
                colour: [op.colour[0], op.colour[1], op.colour[2], 0.0],
                level: op.level,
                half_width: op.half_width,
                band: op.band,
                inv_segment: op.inv_segment,
                duty: op.duty,
                phase: op.phase,
                opacity: op.opacity,
                mix_amt: op.mix,
                from_alpha: u32::from(op.from_alpha),
                composite: u32::from(op.composite),
                matte_on: f32::from(matte.is_some()),
                _pad1: 0,
            }),
        );
        out
    }

    /// Draw one path drawing (docs/08 §3.78, §3.79, §3.76's Mask/Path source) on
    /// a linear working texture, returning a new texture of the same size.
    ///
    /// One pass, and one kernel for the three effects that own one: the pieces
    /// arrive built, so all that differs between a scribble, a brush trail and a
    /// dashed mask stroke is the list.
    pub fn path_draw(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &PathDrawOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-path-draw-out");
        let mut arcs = [[0.0f32; 4]; PATH_PRIMITIVES / 4];
        for (i, a) in op.arcs.iter().enumerate() {
            arcs[i / 4][i % 4] = *a;
        }
        self.dispatch_matted(
            ctx,
            &self.path_draw,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&PathDrawParams {
                colour: [op.colour[0], op.colour[1], op.colour[2], 0.0],
                half_width: op.half_width,
                band: op.band,
                inv_segment: op.inv_segment,
                duty: op.duty,
                phase: op.phase,
                wiggle_amp: op.wiggle_amp,
                wiggle_freq: op.wiggle_freq,
                wiggle_tick: op.wiggle_tick,
                opacity: op.opacity,
                mix_amt: op.mix,
                seed: op.seed,
                count: op.count,
                style: op.style,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0,
                _pad2: 0,
                segs: op.segments,
                arcs,
            }),
        );
        out
    }

    /// Apply one Add grain (docs/08 §3.77) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; the §2.2
    /// unpremultiply wrap is fused into the kernel, and Intensity 0
    /// short-circuits inside it.
    pub fn add_grain(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &AddGrainOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-add-grain-out");
        self.dispatch_matted(
            ctx,
            &self.add_grain,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&AddGrainParams {
                amplitude: [op.amplitude[0], op.amplitude[1], op.amplitude[2], 0.0],
                tonal: [op.tonal[0], op.tonal[1], op.tonal[2], 0.0],
                inv_size: op.inv_size,
                softness: op.softness,
                mix_amt: op.mix,
                _pad0: 0.0,
                seed: op.seed,
                tick: op.tick,
                monochrome: u32::from(op.monochrome),
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }
}
