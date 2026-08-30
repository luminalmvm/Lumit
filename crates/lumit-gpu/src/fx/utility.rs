//! The utility and transition kernels (docs/08 §3.44, §3.46–§3.47, §3.94): Set
//! matte, Linear wipe, Radial wipe and Set channels — the effects that decide
//! how much of a pixel there is, or which of its numbers goes where, rather than
//! what colour it is.
//!
//! Each op mirrors its `lumit_core::fx::cpu` parameter struct field-for-field so
//! the kernel and the CPU oracle consume the identical numbers (K-031). Nothing
//! here does arithmetic; every sine and radian conversion was taken once,
//! host-side, in the effect's own `packed`.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved Set matte (docs/08 §3.44). Mirrors the arguments of
/// `lumit_core::fx::cpu::set_matte`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetMatteOp {
    /// `CHANNEL_OPTIONS` index: 0 luminance, 1 alpha, 2 R, 3 G, 4 B.
    pub channel: u32,
    /// Intersect with the layer's own alpha instead of replacing it.
    pub combine: bool,
    /// The Matte's Invert switch (K-395). Read only when a matte is bound.
    pub invert: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SetMatteParams {
    channel: u32,
    combine: u32,
    matte_on: f32,
    invert: f32,
    mix_amt: f32,
    _pad: [f32; 3],
}

/// One resolved Set channels (docs/08 §3.94). Mirrors the arguments of
/// `lumit_core::fx::cpu::set_channels`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetChannelsOp {
    /// `SET_CHANNELS_OPTIONS` indices for R, G, B and A: 0..4 this layer's
    /// R/G/B/A/luminance, 5..9 the Source layer's, 10 full on, 11 full off.
    pub picks: [u32; 4],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SetChannelsParams {
    picks: [u32; 4],
    source_on: f32,
    mix_amt: f32,
    _pad: [f32; 2],
}

/// One resolved Linear wipe (docs/08 §3.46). Mirrors
/// `lumit_core::fx::cpu::LinearWipeParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearWipeOp {
    /// Where the wipe line pivots, raster pixels.
    pub centre: [f32; 2],
    /// The sweep direction, host-computed `(sin θ, −cos θ)`.
    pub normal: [f32; 2],
    /// Completion ÷ 100.
    pub completion: f32,
    /// The feather's width in raster pixels, floored above zero.
    pub band: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LinearWipeParams {
    centre_normal: [f32; 4],
    completion: f32,
    band: f32,
    mix_amt: f32,
    /// 1 = the matte scales Completion per pixel (K-429).
    matte_on: f32,
}

/// One resolved Radial wipe (docs/08 §3.47). Mirrors
/// `lumit_core::fx::cpu::RadialWipeParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialWipeOp {
    /// Where the hand pivots, raster pixels.
    pub centre: [f32; 2],
    /// Start angle in radians, from straight up, clockwise.
    pub start: f32,
    /// Where the wedge's middle sits from `start`: +1 clockwise, −1
    /// anticlockwise, 0 for Both.
    pub dir: f32,
    /// Completion ÷ 100.
    pub completion: f32,
    /// The soft edge's width at the arc, raster pixels, floored above zero.
    pub feather: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RadialWipeParams {
    centre: [f32; 2],
    start: f32,
    dir: f32,
    completion: f32,
    feather: f32,
    mix_amt: f32,
    /// 1 = the matte scales Completion per pixel (K-429).
    matte_on: f32,
}

impl FxEngine {
    /// Apply one Set matte (docs/08 §3.44) to a linear working texture,
    /// returning a new texture of the same size. One pass.
    ///
    /// **The matte is the effect** (K-395/K-400), so it goes into the kernel and
    /// no dissolve runs beside this op. With none bound the kernel is a
    /// passthrough — the labelled no-op every layer-input effect follows — and
    /// the pass still runs, because a stack that skipped it would have to know
    /// what this effect is.
    pub fn set_matte(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &SetMatteOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-set-matte-out");
        self.dispatch_matted(
            ctx,
            &self.set_matte,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SetMatteParams {
                channel: op.channel,
                combine: u32::from(op.combine),
                matte_on: f32::from(matte.is_some()),
                invert: f32::from(op.invert),
                mix_amt: op.mix,
                _pad: [0.0; 3],
            }),
        );
        out
    }

    /// Apply one Set channels (docs/08 §3.94) to a linear working texture,
    /// returning a new texture of the same size. One pass.
    ///
    /// `source` is this effect's **own** layer row (K-429), not a matte: it
    /// rides the ordinary auxiliary-layer carriage and reaches the kernel
    /// through the same optional-second-texture seam a matte does. With none
    /// bound every `Source …` pick reads zero, and the four `This layer` picks
    /// still shuffle the picture, so the pass is not a passthrough merely
    /// because no layer was named.
    pub fn set_channels(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        source: Option<&wgpu::Texture>,
        op: &SetChannelsOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-set-channels-out");
        self.dispatch_matted(
            ctx,
            &self.set_channels,
            src,
            src,
            source,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SetChannelsParams {
                picks: op.picks,
                source_on: f32::from(source.is_some()),
                mix_amt: op.mix,
                _pad: [0.0; 2],
            }),
        );
        out
    }

    /// Apply one Linear wipe (docs/08 §3.46) to a linear working texture,
    /// returning a new texture of the same size. One pass, one multiply a pixel.
    pub fn linear_wipe(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &LinearWipeOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-linear-wipe-out");
        self.dispatch_matted(
            ctx,
            &self.linear_wipe,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&LinearWipeParams {
                centre_normal: [op.centre[0], op.centre[1], op.normal[0], op.normal[1]],
                completion: op.completion,
                band: op.band,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }

    /// Apply one Radial wipe (docs/08 §3.47) to a linear working texture,
    /// returning a new texture of the same size. One pass: one `atan2` and one
    /// multiply a pixel.
    pub fn radial_wipe(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &RadialWipeOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-radial-wipe-out");
        self.dispatch_matted(
            ctx,
            &self.radial_wipe,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&RadialWipeParams {
                centre: op.centre,
                start: op.start,
                dir: op.dir,
                completion: op.completion,
                feather: op.feather,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }
}

/// One resolved Venetian blinds (docs/08 §3.70). Mirrors
/// `lumit_core::fx::cpu::VenetianBlindsParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VenetianBlindsOp {
    /// The direction the slats close along, host-computed `(sin θ, −cos θ)`.
    pub normal: [f32; 2],
    /// One slat's width in raster pixels, floored at one.
    pub period: f32,
    /// Completion ÷ 100.
    pub completion: f32,
    /// The feather's width in raster pixels, floored above zero.
    pub band: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VenetianBlindsParams {
    normal: [f32; 2],
    period: f32,
    completion: f32,
    band: f32,
    mix_amt: f32,
    /// 1 = the matte scales Completion per pixel (K-429).
    matte_on: f32,
    _pad0: f32,
}

/// One resolved Iris wipe (docs/08 §3.71). Mirrors
/// `lumit_core::fx::cpu::IrisWipeParams` — and note the polygon is already
/// solved into one sector: two vertices became a point and a normal, host-side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrisWipeOp {
    /// Where the iris opens, raster pixels.
    pub centre: [f32; 2],
    /// The sector's first vertex, in the sector's own frame.
    pub vertex: [f32; 2],
    /// The outward unit normal of the edge leaving it.
    pub normal: [f32; 2],
    /// One sector, radians.
    pub period: f32,
    /// Rotation in radians, from straight up, clockwise.
    pub rotation: f32,
    /// The feather's width in raster pixels, floored above zero.
    pub band: f32,
    /// False when Outer radius is 0 — there is no polygon.
    pub active: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct IrisWipeParams {
    centre: [f32; 2],
    vertex: [f32; 2],
    normal: [f32; 2],
    period: f32,
    rotation: f32,
    band: f32,
    active: f32,
    mix_amt: f32,
    /// 1 = the matte scales the polygon's radius per pixel (K-429).
    matte_on: f32,
}

/// One resolved Card wipe (docs/08 §3.72). Mirrors
/// `lumit_core::fx::cpu::CardWipeParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardWipeOp {
    /// Columns then rows.
    pub grid: [i32; 2],
    /// Completion ÷ 100.
    pub completion: f32,
    /// `100 ÷ Transition width`.
    pub inv_width: f32,
    /// `1 − Transition width ÷ 100`.
    pub one_minus_width: f32,
    /// Which grid axis the Flip order ramp runs along: 0 columns, 1 rows.
    pub order_axis: u32,
    /// The ramp's offset and slope.
    pub order_bias: f32,
    /// See [`order_bias`](Self::order_bias).
    pub order_scale: f32,
    /// 0 horizontal, 1 vertical, 2 per card.
    pub axis: u32,
    /// 0 forwards, 1 backwards, 2 per card.
    pub direction: u32,
    /// Randomness ÷ 100.
    pub randomness: f32,
    /// Which shuffle this instance gets.
    pub seed: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CardWipeParams {
    cols: i32,
    rows: i32,
    completion: f32,
    inv_width: f32,
    one_minus_width: f32,
    order_axis: u32,
    order_bias: f32,
    order_scale: f32,
    axis: u32,
    direction: u32,
    randomness: f32,
    seed: u32,
    mix_amt: f32,
    /// 1 = the matte scales Completion per pixel (K-429).
    matte_on: f32,
    _pad: [f32; 2],
}

impl FxEngine {
    /// Apply one Venetian blinds (docs/08 §3.70) to a linear working texture,
    /// returning a new texture of the same size. One pass, one multiply a pixel.
    pub fn venetian_blinds(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &VenetianBlindsOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-venetian-blinds-out");
        self.dispatch_matted(
            ctx,
            &self.venetian_blinds,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&VenetianBlindsParams {
                normal: op.normal,
                period: op.period,
                completion: op.completion,
                band: op.band,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad0: 0.0,
            }),
        );
        out
    }

    /// Apply one Iris wipe (docs/08 §3.71) to a linear working texture,
    /// returning a new texture of the same size. One pass: one `atan2`, one
    /// dot product and one multiply a pixel.
    pub fn iris_wipe(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &IrisWipeOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-iris-wipe-out");
        self.dispatch_matted(
            ctx,
            &self.iris_wipe,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&IrisWipeParams {
                centre: op.centre,
                vertex: op.vertex,
                normal: op.normal,
                period: op.period,
                rotation: op.rotation,
                band: op.band,
                active: f32::from(op.active),
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }

    /// Apply one Card wipe (docs/08 §3.72) to a linear working texture,
    /// returning a new texture of the same size. One pass: one hash, one divide
    /// and one bilinear tap a pixel.
    pub fn card_wipe(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &CardWipeOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-card-wipe-out");
        self.dispatch_matted(
            ctx,
            &self.card_wipe,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&CardWipeParams {
                cols: op.grid[0],
                rows: op.grid[1],
                completion: op.completion,
                inv_width: op.inv_width,
                one_minus_width: op.one_minus_width,
                order_axis: op.order_axis,
                order_bias: op.order_bias,
                order_scale: op.order_scale,
                axis: op.axis,
                direction: op.direction,
                randomness: op.randomness,
                seed: op.seed,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad: [0.0; 2],
            }),
        );
        out
    }
}

/// One resolved Broadcast safe (docs/08 §3.69). Mirrors
/// `lumit_core::fx::cpu::BroadcastSafeParams` — and note there is no NTSC/PAL
/// field: the whole of the standard's difference is its setup pedestal, folded
/// into `target` host-side, so the kernel never branches on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BroadcastSafeOp {
    /// The largest `Y + C` a pixel may carry.
    pub target: f32,
    /// 0 Reduce brightness, 1 Reduce saturation, 2 Key out unsafe, 3 Key out
    /// safe.
    pub mode: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BroadcastSafeParams {
    target: f32,
    mode: u32,
    mix_amt: f32,
    _pad0: f32,
}

impl FxEngine {
    /// Apply one Broadcast safe (docs/08 §3.69) to a linear working texture,
    /// returning a new texture of the same size. One pointwise pass; a pixel
    /// already under the target comes back unchanged by construction, so there
    /// is nothing to short-circuit.
    pub fn broadcast_safe(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &BroadcastSafeOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-broadcast-safe-out");
        self.dispatch(
            ctx,
            &self.broadcast_safe,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BroadcastSafeParams {
                target: op.target,
                mode: op.mode,
                mix_amt: op.mix,
                _pad0: 0.0,
            }),
        );
        out
    }
}
