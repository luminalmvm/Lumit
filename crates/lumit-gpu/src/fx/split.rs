//! Channel-offset kernels (docs/08 §3.6, §3.15): the classic RGB split, its
//! wavelength-dispersion sibling, and the always-radial chromatic aberration.

use crate::GpuContext;

use super::{work_texture, FxEngine};

/// One resolved RGB split (docs/08 §3.6, T17): a linear, three-tinted-tap
/// fringe. The offset vector is host-computed
/// (`lumit_core::fx::rgb_split_offset`) so the kernel never runs its own
/// trigonometry; the always-radial variant is chromatic aberration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbSplitOp {
    /// Channel offset, raster pixels.
    pub dx: f32,
    pub dy: f32,
    /// Per-tap displacement scale `[t0, t1, t2]` (FX-9): taps 0/1 shift along
    /// −offset·scale, tap 2 along +offset·scale; `[1, 0, 1]` is the classic split.
    pub scale: [f32; 3],
    /// The three taps' tints `[[r, g, b]; 3]` (T17): each tap is sampled in
    /// full colour and multiplied by its tint, then summed. Defaults red /
    /// green / blue reproduce the classic channel-separated split.
    pub tints: [[f32; 3]; 3],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RgbSplitParams {
    /// Row-major vec4 tints (w unused), one per tap.
    tints: [[f32; 4]; 3],
    dx: f32,
    dy: f32,
    scale_r: f32,
    scale_g: f32,
    scale_b: f32,
    mix_amt: f32,
    /// 1 = scale Amount by the matte.
    matte_on: f32,
    _pad1: f32,
}

/// One resolved spectral split — the RGB split's Wavelength mode (docs/08
/// §3.6), its own kernel so the classic mode stays byte-identical.
/// The offset vector and the wavelength basis both arrive host-computed
/// (`lumit_core::fx::rgb_split_offset` / `spectral_basis_uniform`), so the
/// kernel consumes exactly the CPU reference's numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralSplitOp {
    /// Linear-mode peak offset, raster pixels.
    pub dx: f32,
    pub dy: f32,
    /// Radial-mode peak offset (reached at the corner distance), raster px.
    pub amount_px: f32,
    pub radial: bool,
    /// The spectral taps (FX-9): each row is `[r, g, b, fraction]` — the
    /// column-normalised weight and the tap's offset fraction in `[-1, +1]`.
    /// The first `count` rows are active; the rest are zero. From
    /// `lumit_core::fx::spectral_basis_uniform`.
    pub basis: [[f32; 4]; 64],
    /// The number of active taps (`2..=64`).
    pub count: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpectralSplitParams {
    basis: [[f32; 4]; 64],
    dx: f32,
    dy: f32,
    amount: f32,
    radial: u32,
    count: u32,
    mix_amt: f32,
    /// 1 = scale Amount by the matte.
    matte_on: f32,
    _pad1: f32,
}

/// One resolved chromatic aberration (docs/08 §3.15): a dedicated,
/// always-radial sibling of [`RgbSplitOp`]'s own radial mode — three tinted
/// radial taps, no linear offset or wavelength dispersion of its own (the
/// Wavelength mode resolves to [`SpectralSplitOp`] instead).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromaticAberrationOp {
    /// Peak channel offset, raster pixels (reached at the corner distance).
    pub amount_px: f32,
    /// The three radial taps' tints `[[r, g, b]; 3]` (P2), at fractions
    /// −1 / 0 / +1. Defaults red / green / blue reproduce the classic split.
    pub tints: [[f32; 3]; 3],
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ChromaticAberrationParams {
    /// Row-major vec4 tints (w unused), one per tap.
    tints: [[f32; 4]; 3],
    amount: f32,
    mix_amt: f32,
    /// 1 = scale Amount by the matte.
    matte_on: f32,
    _pad1: f32,
}

impl FxEngine {
    /// Apply one RGB split (docs/08 §3.6) to a linear working texture,
    /// returning a new texture of the same size. Single pointwise pass with
    /// offset bilinear taps.
    pub fn rgb_split(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &RgbSplitOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-rgb-split-out");
        self.dispatch_matted(
            ctx,
            &self.rgb_split,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&RgbSplitParams {
                tints: [
                    [op.tints[0][0], op.tints[0][1], op.tints[0][2], 0.0],
                    [op.tints[1][0], op.tints[1][1], op.tints[1][2], 0.0],
                    [op.tints[2][0], op.tints[2][1], op.tints[2][2], 0.0],
                ],
                dx: op.dx,
                dy: op.dy,
                scale_r: op.scale[0],
                scale_g: op.scale[1],
                scale_b: op.scale[2],
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
            }),
        );
        out
    }

    /// Apply one spectral split — the RGB split's Wavelength mode (docs/08
    /// §3.6) — to a linear working texture, returning a new texture
    /// of the same size. Single pointwise pass, nine offset bilinear taps
    /// weighted by the host-supplied wavelength basis.
    pub fn spectral_split(
        &self,
        ctx: &GpuContext,
        src: &wgpu::Texture,
        w: u32,
        h: u32,
        matte: Option<&wgpu::Texture>,
        op: &SpectralSplitOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-spectral-split-out");
        self.dispatch_matted(
            ctx,
            &self.spectral_split,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&SpectralSplitParams {
                basis: op.basis,
                dx: op.dx,
                dy: op.dy,
                amount: op.amount_px,
                radial: u32::from(op.radial),
                count: op.count,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
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
        matte: Option<&wgpu::Texture>,
        op: &ChromaticAberrationOp,
    ) -> wgpu::Texture {
        let out = work_texture(ctx, w, h, "fx-chromatic-aberration-out");
        self.dispatch_matted(
            ctx,
            &self.chromatic_aberration,
            src,
            src,
            matte,
            &out,
            w,
            h,
            bytemuck::bytes_of(&ChromaticAberrationParams {
                tints: [
                    [op.tints[0][0], op.tints[0][1], op.tints[0][2], 0.0],
                    [op.tints[1][0], op.tints[1][1], op.tints[1][2], 0.0],
                    [op.tints[2][0], op.tints[2][1], op.tints[2][2], 0.0],
                ],
                amount: op.amount_px,
                mix_amt: op.mix,
                matte_on: f32::from(matte.is_some()),
                _pad1: 0.0,
            }),
        );
        out
    }
}
