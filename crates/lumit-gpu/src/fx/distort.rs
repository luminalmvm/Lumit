//! The distort kernels (docs/08 §3.38–§3.42 and §3.48–§3.52): Turbulent
//! displace, Tile, Offset, Mirror, Lens distort, and Wave 2's Corner pin,
//! Displacement map, Polar coordinates, Twirl and Spherize — the effects that
//! move pixels rather than recolour them.
//!
//! Each op mirrors its `lumit_core::fx::cpu` parameter struct field-for-field so
//! the kernel and the CPU oracle consume the identical numbers (K-031). Nothing
//! here does arithmetic; every reciprocal, cosine and tangent that *could* be
//! taken once was taken host-side, in the effect's own `packed`.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved Turbulent displace (docs/08 §3.38). Mirrors
/// `lumit_core::fx::cpu::TurbulentDisplaceParams` with its `FractalField`
/// flattened, which is the shape the uniform wants anyway.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurbulentDisplaceOp {
    /// The x field's seed; the y field's is the salted sibling.
    pub seed_x: u32,
    pub seed_y: u32,
    /// 1..=10.
    pub octaves: u32,
    pub gain: f32,
    pub lacunarity: f32,
    /// Depth loop length in cells; 0 for a field that never repeats.
    pub cycle: i32,
    /// Field origin in raster pixels.
    pub offset: [f32; 2],
    /// `1 ÷ Size`, raster pixels.
    pub inv_size: f32,
    /// Depth coordinate (Evolution ÷ 360, folded into the cycle).
    pub z: f32,
    /// Amount, raster pixels; signed.
    pub amount: f32,
    /// Which components survive: `[1,1]`, `[1,0]` or `[0,1]`.
    pub axes: [f32; 2],
    /// Per axis, 1 when that axis's pair of edges is pinned.
    pub pin: [f32; 2],
    /// `1 ÷ |Amount|` — the pin ramp's width, reciprocated.
    pub inv_pin_band: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TurbulentDisplaceParams {
    offset_axes: [f32; 4],
    pin_amount: [f32; 4],
    inv_size: f32,
    z: f32,
    gain: f32,
    lacunarity: f32,
    seed_x: u32,
    seed_y: u32,
    octaves: u32,
    cycle: i32,
    mix_amt: f32,
    matte_on: f32,
    /// Was Invert; the seam applies it once since K-425. Always 0.
    _pad1: f32,
    _pad0: f32,
}

/// One resolved Tile (docs/08 §3.39). Mirrors `lumit_core::fx::cpu::TileParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileOp {
    /// The stamped rectangle's centre, raster pixels.
    pub centre: [f32; 2],
    /// Tile width and height as fractions of the frame.
    pub tile_frac: [f32; 2],
    /// Output width and height as fractions of the frame.
    pub output_frac: [f32; 2],
    /// Phase ÷ 360, in tiles.
    pub phase: f32,
    pub mirror_edges: bool,
    pub horizontal_phase_shift: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileParams {
    centre_tile: [f32; 4],
    output_frac: [f32; 2],
    phase: f32,
    mix_amt: f32,
    mirror_edges: u32,
    horizontal_phase_shift: u32,
    _pad0: u32,
    _pad1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OffsetParams {
    shift: [f32; 2],
    mix_amt: f32,
    /// 1 = scale the shift by the matte (K-427).
    matte_on: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MirrorParams {
    centre_normal: [f32; 4],
    mix_amt: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// One resolved Lens distort (docs/08 §3.42). Mirrors
/// `lumit_core::fx::cpu::LensDistortParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensDistortOp {
    /// False below the effect's minimum field of view: the exact identity.
    pub active: bool,
    /// `tan(Field of view ÷ 2)`, host-computed.
    pub tan_half_fov: f32,
    /// Remove the fisheye rather than add it — the exact inverse mapping.
    pub reverse: bool,
    /// Which half-extent the field of view spans: 0 width, 1 height, 2 diagonal.
    pub half_kind: u32,
    /// The optical centre, raster pixels.
    pub centre: [f32; 2],
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LensDistortParams {
    centre: [f32; 2],
    tan_half_fov: f32,
    mix_amt: f32,
    half_kind: u32,
    edge: u32,
    /// `active` in the op and the CPU reference; **`enabled` here**, because
    /// `active` is a reserved keyword in WGSL and a uniform's fields must be
    /// spellable in the kernel.
    enabled: u32,
    reverse: u32,
    /// 1 = scale the displacement by the matte (K-427).
    matte_on: f32,
    _pad: [f32; 3],
}

/// One resolved Corner pin (docs/08 §3.48). Mirrors
/// `lumit_core::fx::cpu::CornerPinParams` with its 3×3 flattened to three rows,
/// which is the shape the uniform wants anyway — a `array<f32, 9>` in a uniform
/// has a 16-byte stride per element in WGSL, so the rows are vec4s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerPinOp {
    /// The inverse homography, row-major, taking a raster pixel to the unit
    /// square. Defined only up to a scale, sign-normalised host-side.
    pub inv: [[f32; 3]; 3],
    /// False for a degenerate quad: the exact identity.
    pub active: bool,
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CornerPinParams {
    n0: [f32; 4],
    n1: [f32; 4],
    n2: [f32; 4],
    mix_amt: f32,
    edge: u32,
    /// `active` in the op and the CPU reference; **`enabled` here**, because
    /// `active` is a reserved keyword in WGSL (fx_lensdistort.wgsl's note).
    enabled: u32,
    /// 1 = scale the pull from the corners by the matte (K-427).
    matte_on: f32,
}

/// One resolved Displacement map (docs/08 §3.49). Mirrors
/// `lumit_core::fx::cpu::DisplacementMapParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplacementMapOp {
    /// `CHANNEL_OPTIONS` indices: which channel of the map steers x, and which y.
    pub channels: [u32; 2],
    /// The farthest push per axis, raster pixels; signed.
    pub amount: [f32; 2],
    /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
    pub edge: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
    /// The Matte's Invert switch (K-395): with it on the map is read the other
    /// way round, so every push reverses. Read only when a map is bound.
    pub matte_invert: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DisplacementMapParams {
    amount: [f32; 2],
    mix_amt: f32,
    matte_on: f32,
    chan_x: u32,
    chan_y: u32,
    edge: u32,
    invert: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PolarParams {
    interp: f32,
    mix_amt: f32,
    to_polar: u32,
    _pad0: u32,
}

/// One resolved Twirl (docs/08 §3.51). Mirrors
/// `lumit_core::fx::cpu::TwirlParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwirlOp {
    /// The twirl's middle, raster pixels.
    pub centre: [f32; 2],
    /// The twirled circle's radius, raster pixels.
    pub radius: f32,
    /// `1 ÷ radius`, floored host-side.
    pub inv_radius: f32,
    /// Radians; positive turns the picture clockwise on screen.
    pub angle: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TwirlParams {
    centre: [f32; 2],
    radius: f32,
    inv_radius: f32,
    angle: f32,
    mix_amt: f32,
    /// 1 = scale Angle by the matte (K-427).
    matte_on: f32,
    _pad1: f32,
}

/// One resolved Spherize (docs/08 §3.52). Mirrors
/// `lumit_core::fx::cpu::SpherizeParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpherizeOp {
    /// The ball's middle, raster pixels.
    pub centre: [f32; 2],
    /// The ball's radius, raster pixels.
    pub radius: f32,
    /// `1 ÷ radius`, floored host-side.
    pub inv_radius: f32,
    /// Bulge ÷ 100, −1..1: the sign chooses the map, the magnitude blends.
    pub bulge: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpherizeParams {
    centre: [f32; 2],
    radius: f32,
    inv_radius: f32,
    bulge: f32,
    mix_amt: f32,
    /// 1 = scale Bulge by the matte (K-427).
    matte_on: f32,
    _pad1: f32,
}

/// One resolved Ripple (docs/08 §3.53). Mirrors
/// `lumit_core::fx::cpu::RippleParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RippleOp {
    /// The rings' middle, raster pixels.
    pub centre: [f32; 2],
    /// How far the rings reach, raster pixels.
    pub radius: f32,
    /// `1 ÷ radius`, floored host-side.
    pub inv_radius: f32,
    /// The farthest a pixel moves, raster pixels (Wave height times `27⁄4`).
    pub amount: f32,
    /// `1 ÷ Wave width`, raster pixels.
    pub inv_width: f32,
    /// Evolution ÷ 360, in whole waves.
    pub turns: f32,
    /// Add the tangential half of the wave, so a pixel walks a small circle.
    pub asymmetric: bool,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RippleParams {
    centre: [f32; 2],
    radius: f32,
    inv_radius: f32,
    amount: f32,
    inv_width: f32,
    turns: f32,
    mix_amt: f32,
    asymmetric: u32,
    /// 1 = scale Wave height by the matte (K-427).
    matte_on: f32,
    _pad1: u32,
    _pad2: u32,
}

/// One resolved Wave warp (docs/08 §3.54). Mirrors
/// `lumit_core::fx::cpu::WaveWarpParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveWarpOp {
    /// The direction the wave travels, host-computed `(sin θ, −cos θ)`.
    pub dir: [f32; 2],
    /// That vector turned a quarter-turn clockwise on screen.
    pub perp: [f32; 2],
    /// How far the picture slides at a crest, raster pixels; signed.
    pub height: f32,
    /// `1 ÷ Wave width`, raster pixels.
    pub inv_width: f32,
    /// Phase ÷ 360, in whole waves.
    pub turns: f32,
    /// 0 Sine, 1 Square, 2 Triangle, 3 Sawtooth, 4 Circle.
    pub shape: u32,
    /// Per edge — left, right, top, bottom — 1 when that edge is pinned.
    pub pin: [f32; 4],
    /// `1 ÷ |Wave height|` — the pin ramp's width, reciprocated.
    pub inv_pin_band: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WaveWarpParams {
    dir_perp: [f32; 4],
    pin: [f32; 4],
    height: f32,
    inv_width: f32,
    turns: f32,
    inv_pin_band: f32,
    mix_amt: f32,
    shape: u32,
    /// 1 = scale Wave height by the matte (K-427).
    matte_on: f32,
    _pad1: u32,
}

/// One resolved Bezier warp (docs/08 §3.55). Mirrors
/// `lumit_core::fx::cpu::BezierWarpParams`; the twelve points travel two to a
/// `vec4`, because a `vec2` in a WGSL uniform array has a 16-byte stride and
/// half the uniform would be padding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BezierWarpOp {
    /// The twelve points in AE's clockwise walk from the upper left, raster
    /// pixels.
    pub pts: [[f32; 2]; 12],
    /// Newton steps a pixel, 1..=12.
    pub steps: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BezierWarpParams {
    q: [[f32; 4]; 6],
    mix_amt: f32,
    steps: u32,
    /// 1 = scale the bend from the straight frame by the matte (K-427).
    matte_on: f32,
    _pad1: u32,
}

/// One resolved Warp (docs/08 §3.56). Mirrors
/// `lumit_core::fx::cpu::WarpParams`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpOp {
    /// Which of the thirteen bends, in §3.56's table order.
    pub style: u32,
    /// Bend ÷ 100, −1..1.
    pub bend: f32,
    /// Horizontal distortion ÷ 100, clamped host-side to ±0.9.
    pub h_distort: f32,
    /// Vertical distortion ÷ 100; see [`h_distort`](Self::h_distort).
    pub v_distort: f32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WarpParams {
    bend: f32,
    h_distort: f32,
    v_distort: f32,
    mix_amt: f32,
    style: u32,
    /// 1 = scale Bend and both distortions by the matte (K-427).
    matte_on: f32,
    _pad1: u32,
    _pad2: u32,
}

impl FxEngine {
    /// Apply one Ripple (docs/08 §3.53) to a linear working texture, returning a
    /// new texture of the same size. One sine and cosine and one bilinear tap a
    /// pixel.
    pub fn ripple(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &RippleOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-ripple-out");
        self.dispatch_matted(
            ctx,
            &self.ripple,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&RippleParams {
                centre: op.centre,
                radius: op.radius,
                inv_radius: op.inv_radius,
                amount: op.amount,
                inv_width: op.inv_width,
                turns: op.turns,
                mix_amt: op.mix,
                asymmetric: u32::from(op.asymmetric),
                matte_on: f32::from(matte.is_some()),
                _pad1: 0,
                _pad2: 0,
            }),
        );
        out
    }

    /// Apply one Wave warp (docs/08 §3.54) to a linear working texture,
    /// returning a new texture of the same size. One wave shape and one bilinear
    /// tap a pixel.
    pub fn wave_warp(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &WaveWarpOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-wave-warp-out");
        self.dispatch_matted(
            ctx,
            &self.wave_warp,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&WaveWarpParams {
                dir_perp: [op.dir[0], op.dir[1], op.perp[0], op.perp[1]],
                pin: op.pin,
                height: op.height,
                inv_width: op.inv_width,
                turns: op.turns,
                inv_pin_band: op.inv_pin_band,
                mix_amt: op.mix,
                shape: op.shape,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0,
            }),
        );
        out
    }

    /// Apply one Bezier warp (docs/08 §3.55) to a linear working texture,
    /// returning a new texture of the same size. Up to twelve Newton steps a
    /// pixel, each a Coons patch evaluation and its Jacobian.
    pub fn bezier_warp(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &BezierWarpOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-bezier-warp-out");
        let mut q = [[0.0f32; 4]; 6];
        for (i, pair) in q.iter_mut().enumerate() {
            *pair = [
                op.pts[i * 2][0],
                op.pts[i * 2][1],
                op.pts[i * 2 + 1][0],
                op.pts[i * 2 + 1][1],
            ];
        }
        self.dispatch_matted(
            ctx,
            &self.bezier_warp,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&BezierWarpParams {
                q,
                mix_amt: op.mix,
                steps: op.steps,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0,
            }),
        );
        out
    }

    /// Apply one Warp (docs/08 §3.56) to a linear working texture, returning a
    /// new texture of the same size. One style evaluation and one bilinear tap a
    /// pixel — thirteen styles, one kernel.
    pub fn warp(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &WarpOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-warp-out");
        self.dispatch_matted(
            ctx,
            &self.warp,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&WarpParams {
                bend: op.bend,
                h_distort: op.h_distort,
                v_distort: op.v_distort,
                mix_amt: op.mix,
                style: op.style,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0,
                _pad2: 0,
            }),
        );
        out
    }

    /// Apply one Corner pin (docs/08 §3.48) to a linear working texture,
    /// returning a new texture of the same size. One matrix multiply, one divide
    /// and one bilinear tap a pixel — the projective derivation happened
    /// host-side.
    pub fn corner_pin(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &CornerPinOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-corner-pin-out");
        let row = |r: [f32; 3]| [r[0], r[1], r[2], 0.0];
        self.dispatch_matted(
            ctx,
            &self.corner_pin,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&CornerPinParams {
                n0: row(op.inv[0]),
                n1: row(op.inv[1]),
                n2: row(op.inv[2]),
                mix_amt: op.mix,
                edge: op.edge,
                enabled: u32::from(op.active),
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }

    /// Apply one Displacement map (docs/08 §3.49) to a linear working texture,
    /// returning a new texture of the same size. One map read and one bilinear
    /// tap a pixel.
    ///
    /// **The matte IS the map** (K-395), so it goes into the kernel and no
    /// dissolve runs beside this op. With none bound the kernel is a
    /// passthrough — the labelled no-op every layer-input effect follows.
    pub fn displacement_map(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &DisplacementMapOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-dispmap-out");
        self.dispatch_matted(
            ctx,
            &self.displacement_map,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&DisplacementMapParams {
                amount: op.amount,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                chan_x: op.channels[0],
                chan_y: op.channels[1],
                edge: op.edge,
                invert: u32::from(op.matte_invert),
            }),
        );
        out
    }

    /// Apply one Polar coordinates (docs/08 §3.50) to a linear working texture,
    /// returning a new texture of the same size. Three transcendentals and one
    /// bilinear tap a pixel.
    ///
    /// Three scalars rather than an op struct, like [`Self::mirror`]: an effect
    /// with a direction and two fractions does not need a named bundle.
    #[allow(clippy::too_many_arguments)]
    pub fn polar_coordinates(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        to_polar: bool,
        interp: f32,
        mix: f32,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-polar-out");
        self.dispatch(
            ctx,
            &self.polar_coordinates,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&PolarParams {
                interp,
                mix_amt: mix,
                to_polar: u32::from(to_polar),
                _pad0: 0,
            }),
        );
        out
    }

    /// Apply one Twirl (docs/08 §3.51) to a linear working texture, returning a
    /// new texture of the same size. One sine/cosine pair and one bilinear tap a
    /// pixel.
    pub fn twirl(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &TwirlOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-twirl-out");
        self.dispatch_matted(
            ctx,
            &self.twirl,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TwirlParams {
                centre: op.centre,
                radius: op.radius,
                inv_radius: op.inv_radius,
                angle: op.angle,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one Spherize (docs/08 §3.52) to a linear working texture, returning
    /// a new texture of the same size. One arc sine or sine and one bilinear tap
    /// a pixel.
    pub fn spherize(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &SpherizeOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-spherize-out");
        self.dispatch_matted(
            ctx,
            &self.spherize,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SpherizeParams {
                centre: op.centre,
                radius: op.radius,
                inv_radius: op.inv_radius,
                bulge: op.bulge,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one Turbulent displace (docs/08 §3.38) to a linear working texture,
    /// returning a new texture of the same size. One pass: two fractal sums and
    /// one bilinear tap a pixel.
    ///
    /// **The matte scales the displacement** (K-395), so it goes into the kernel
    /// and no dissolve runs beside this op. With none bound the kernel takes the
    /// branch it always takes and the vector is used exactly as the field gave it.
    pub fn turbulent_displace(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &TurbulentDisplaceOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-turbdisplace-out");
        self.dispatch_matted(
            ctx,
            &self.turbulent_displace,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TurbulentDisplaceParams {
                offset_axes: [op.offset[0], op.offset[1], op.axes[0], op.axes[1]],
                pin_amount: [op.pin[0], op.pin[1], op.amount, op.inv_pin_band],
                inv_size: op.inv_size,
                z: op.z,
                gain: op.gain,
                lacunarity: op.lacunarity,
                seed_x: op.seed_x,
                seed_y: op.seed_y,
                octaves: op.octaves,
                cycle: op.cycle,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
                _pad0: 0.0,
            }),
        );
        out
    }

    /// Apply one Tile (docs/08 §3.39) to a linear working texture, returning a
    /// new texture of the same size. One bilinear tap a pixel, or transparent
    /// outside the output window.
    pub fn tile(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        op: &TileOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-tile-out");
        self.dispatch(
            ctx,
            &self.tile,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&TileParams {
                centre_tile: [op.centre[0], op.centre[1], op.tile_frac[0], op.tile_frac[1]],
                output_frac: op.output_frac,
                phase: op.phase,
                mix_amt: op.mix,
                mirror_edges: u32::from(op.mirror_edges),
                horizontal_phase_shift: u32::from(op.horizontal_phase_shift),
                _pad0: 0,
                _pad1: 0,
            }),
        );
        out
    }

    /// Apply one Offset (docs/08 §3.40) to a linear working texture, returning a
    /// new texture of the same size. One wrapped bilinear tap a pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn offset(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        shift: [f32; 2],
        mix: f32,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-offset-out");
        self.dispatch_matted(
            ctx,
            &self.offset,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&OffsetParams {
                shift,
                mix_amt: mix,
                matte_on: f32::from(matte.is_some()),
            }),
        );
        out
    }

    /// Apply one Mirror (docs/08 §3.41) to a linear working texture, returning a
    /// new texture of the same size. `normal` is the host-computed `(cos, sin)`
    /// of Angle — the kernel runs no trigonometry.
    ///
    /// Three scalars rather than an op struct, like [`Self::offset`]: an effect
    /// with two points and a mix does not need a named bundle to be read.
    #[allow(clippy::too_many_arguments)]
    pub fn mirror(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        centre: [f32; 2],
        normal: [f32; 2],
        mix: f32,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-mirror-out");
        self.dispatch(
            ctx,
            &self.mirror,
            src,
            src,
            &out,
            w,
            h,
            bytemuck::bytes_of(&MirrorParams {
                centre_normal: [centre[0], centre[1], normal[0], normal[1]],
                mix_amt: mix,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );
        out
    }

    /// Apply one Lens distort (docs/08 §3.42) to a linear working texture,
    /// returning a new texture of the same size. One tangent (or arc tangent)
    /// and one bilinear tap a pixel.
    pub fn lens_distort(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &LensDistortOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-lens-distort-out");
        self.dispatch_matted(
            ctx,
            &self.lens_distort,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&LensDistortParams {
                centre: op.centre,
                tan_half_fov: op.tan_half_fov,
                mix_amt: op.mix,
                half_kind: op.half_kind,
                edge: op.edge,
                enabled: u32::from(op.active),
                reverse: u32::from(op.reverse),
                matte_on: f32::from(matte.is_some()),
                _pad: [0.0; 3],
            }),
        );
        out
    }
}
