//! The GPU effect kernels (docs/05 crate table: "WGSL effect kernels" live
//! here; docs/08-EFFECTS.md §1.1 part 2 — the production path). Each kernel
//! mirrors its CPU reference in `lumit_core::fx::cpu` op-for-op; the §1.6
//! oracle tests at the bottom hold the two to agreement.
//!
//! In plain terms: this is where effects actually run during preview and
//! export — small GPU programs working on the same linear fp16 textures the
//! compositor uses. The engine takes plain numbers (a blur radius in pixels,
//! an edge mode), so it neither knows nor cares about the project model.

use crate::{GpuContext, WORKING_FORMAT};

mod blur;
mod colour;
mod common;
/// The Custom shader's GPU half (K-650): validate, compile, cache, dispatch.
mod custom_shader;
mod distort;
mod dof;
mod engine;
mod generate;
mod lens_flare;
mod lighting;
mod particulate;
mod points_draw;
mod split;
mod stylise;
mod temporal;
mod utility;

pub use blur::*;
pub use colour::*;
pub use common::*;
pub use custom_shader::*;
pub use distort::*;
// `dof` exposes its `impl FxEngine` methods, which are reachable without a
// re-export — but it also houses the `DofOp` parameter struct that carries the
// effect's two dozen scalars, and a public type does need naming.
pub use dof::*;
pub use generate::*;
pub use lens_flare::*;
pub use lighting::*;
pub use particulate::*;
pub use points_draw::*;
pub use split::*;
pub use stylise::*;
pub use temporal::*;
pub use utility::*;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

/// The effect-pass engine: compiled kernels plus their layouts, one per
/// device (owned alongside the Compositor by whoever renders).
pub struct FxEngine {
    /// The Custom shader (docs/08 §3.95, K-650): its own seven-entry bind group
    /// layout, and one compiled pipeline per distinct source. Its own layout
    /// rather than the shared fx five, exactly as the LUT's is — the two extra
    /// pictures and the user's own uniform have nowhere to go on that one.
    custom_shader: custom_shader::CustomShaderPipelines,
    blur: wgpu::ComputePipeline,
    dir_blur: wgpu::ComputePipeline,
    radial_blur: wgpu::ComputePipeline,
    sharpen_unpremultiply: wgpu::ComputePipeline,
    sharpen_combine: wgpu::ComputePipeline,
    /// Plain 3×3 sharpen (docs/08 §3.9, K-138): a single-pass high-pass
    /// convolution, the radius-free sibling of the Unsharp mask's two-entry
    /// kernel above.
    sharpen_simple: wgpu::ComputePipeline,
    /// Sprite flare (docs/08 §3.29, K-359): one procedural pass, placed from
    /// a light position rather than from the picture's bright pixels.
    sprite_flare: wgpu::ComputePipeline,
    /// Light wrap (docs/08 §3.28, K-358): the two passes that fold the
    /// background's blur and the foreground's softened matte into the edge.
    light_wrap_pack: wgpu::ComputePipeline,
    light_wrap_combine: wgpu::ComputePipeline,
    rgb_split: wgpu::ComputePipeline,
    spectral_split: wgpu::ComputePipeline,
    chromatic_aberration: wgpu::ComputePipeline,
    flash: wgpu::ComputePipeline,
    colour_balance: wgpu::ComputePipeline,
    saturation: wgpu::ComputePipeline,
    vibrancy: wgpu::ComputePipeline,
    matte_key: wgpu::ComputePipeline,
    /// The spatial keyer's stages (K-546): the screen matte on its own, the
    /// separable shrink/grow, the despot, one garbage mask, and the pass that
    /// spends the finished matte on the original colour.
    matte_key_screen: wgpu::ComputePipeline,
    matte_key_combine: wgpu::ComputePipeline,
    matte_morph: wgpu::ComputePipeline,
    matte_despot: wgpu::ComputePipeline,
    /// Stroke (**style**, K-706): the separable pass that carries the fattened
    /// and thinned copies of the layer's alpha together, and the combine that
    /// cuts the band between them.
    stroke_morph: wgpu::ComputePipeline,
    stroke_combine: wgpu::ComputePipeline,
    matte_mask: wgpu::ComputePipeline,
    vignette: wgpu::ComputePipeline,
    exposure: wgpu::ComputePipeline,
    /// The lighting pass (docs/06, K-361). Not an effect — the realiser calls
    /// it directly, between a layer's effect stack and its composite.
    lighting: wgpu::ComputePipeline,
    temperature: wgpu::ComputePipeline,
    invert: wgpu::ComputePipeline,
    tint: wgpu::ComputePipeline,
    hue_shift: wgpu::ComputePipeline,
    contrast: wgpu::ComputePipeline,
    gamma: wgpu::ComputePipeline,
    /// Curves (docs/08 §3.30, K-396): the per-channel monotone-cubic tone
    /// curve. Shares the ordinary pointwise layout — the knots and their
    /// tangents arrive in the uniform, already fitted host-side.
    curves: wgpu::ComputePipeline,
    /// Levels (docs/08 §3.31): per-channel input/output black and white with
    /// gamma, both reciprocals precomputed host-side.
    levels: wgpu::ComputePipeline,
    /// Brightness (docs/08 §3.32, K-397): AE's Brightness & Contrast pair as
    /// one affine grade about the mid-grey pivot Contrast uses.
    brightness: wgpu::ComputePipeline,
    /// Hue and saturation (docs/08 §3.33): the master adjustment and six
    /// weighted colour ranges, through an HSV round trip.
    hue_saturation: wgpu::ComputePipeline,
    /// Posterize (docs/08 §3.58, K-404): the tone ladder cut into steps, the
    /// rungs spaced in a square root of the light so they land where a person
    /// sees them.
    posterize: wgpu::ComputePipeline,
    /// Threshold (docs/08 §3.59): every pixel to black or to white, across a
    /// crossing that is never a bare step.
    threshold: wgpu::ComputePipeline,
    /// Tritone (docs/08 §3.60): three colours mapped onto the tone range.
    tritone: wgpu::ComputePipeline,
    /// Photo filter (docs/08 §3.61): a coloured glass in front of the lens,
    /// with the exposure optionally put back afterwards.
    photo_filter: wgpu::ComputePipeline,
    /// Black and white (docs/08 §3.62): the exact grey/secondary/primary
    /// decomposition under six weights.
    black_and_white: wgpu::ComputePipeline,
    /// Shadow highlight (docs/08 §3.63), the second pass: the local lift and
    /// pull, steered by the luma of the blurred picture. The blur itself is
    /// [`Self::blur`], reused for the third time — after §3.43's softening and
    /// §3.57's distance field — and here it is a *question*, never a colour.
    shadow_highlight: wgpu::ComputePipeline,
    /// Fill (docs/08 §3.34, K-398): the layer's own coverage flooded with one
    /// colour.
    fill: wgpu::ComputePipeline,
    /// Gradient (docs/08 §3.35): the linear or radial two-colour ramp.
    gradient: wgpu::ComputePipeline,
    /// Noise (docs/08 §3.36): per-pixel uniform or gaussian grain.
    noise: wgpu::ComputePipeline,
    /// Fractal noise (docs/08 §3.37): the seeded multi-octave generator, and
    /// the WGSL twin of the shared `lumit_core::fx::noise` core the
    /// displacement family will reuse.
    fractal_noise: wgpu::ComputePipeline,
    /// Beam (docs/08 §3.73): a tapered shaft of light between two points. One
    /// capsule a pixel.
    beam: wgpu::ComputePipeline,
    /// Lightning (docs/08 §3.74): a forked bolt. The first kernel in the
    /// catalogue whose *geometry* arrives in the uniform, already built — the
    /// randomness does not vary per pixel, so it does not belong in the kernel.
    lightning: wgpu::ComputePipeline,
    /// Radio waves (docs/08 §3.75): shapes emitted from a point and expanding.
    /// §3.71's sector solve, done once for a unit shape and scaled per wave.
    radio_waves: wgpu::ComputePipeline,
    /// Vegas (docs/08 §3.76): marching lights along the picture's contours. The
    /// contour is a level set rather than an edge detector's output, which is
    /// what makes Width a width in pixels.
    vegas: wgpu::ComputePipeline,
    /// The shared path drawing (docs/08 §3.78 Scribble, §3.79 Stroke and §3.76
    /// Vegas' Mask/Path source): a maximum over capsules that arrive already
    /// built, and the fifth reader of `fx_noise_core.wgsl` — Scribble's waver
    /// displaces the paper rather than the geometry.
    ///
    /// One pipeline for three effects, because what differs between them is
    /// where the line goes and that is decided on the CPU (K-408).
    path_draw: wgpu::ComputePipeline,
    /// Add grain (docs/08 §3.77): film grain laid on by tone. The fourth reader
    /// of the shared `fx_noise_core.wgsl`.
    add_grain: wgpu::ComputePipeline,
    /// Turbulent displace (docs/08 §3.38): the fractal-driven warp, and the
    /// second reader of the shared `fx_noise_core.wgsl`. One of the kernels that
    /// claim the K-395 matte inside their own maths — it scales the
    /// displacement.
    turbulent_displace: wgpu::ComputePipeline,
    /// Tile (docs/08 §3.39): one rectangle of the picture stamped across the
    /// frame.
    tile: wgpu::ComputePipeline,
    /// Offset (docs/08 §3.40): the frame slid, wrapping round.
    offset: wgpu::ComputePipeline,
    /// Mirror (docs/08 §3.41): one half reflected onto the other.
    mirror: wgpu::ComputePipeline,
    /// Lens distort (docs/08 §3.42): barrel and pincushion by field of view.
    lens_distort: wgpu::ComputePipeline,
    /// Corner pin (docs/08 §3.48): the picture pulled onto four points, through
    /// the inverse of the homography they define.
    corner_pin: wgpu::ComputePipeline,
    /// Displacement map (docs/08 §3.49): another layer's channels push this one.
    /// The seventh kernel to claim the K-395 matte inside its own maths, and the
    /// second (after Set matte) for which the matte is the effect's subject.
    displacement_map: wgpu::ComputePipeline,
    /// Polar coordinates (docs/08 §3.50): the frame bent into a circle, and the
    /// exact inverse map back.
    polar_coordinates: wgpu::ComputePipeline,
    /// Twirl (docs/08 §3.51): the picture wrung round a point.
    twirl: wgpu::ComputePipeline,
    /// Spherize (docs/08 §3.52): a glass ball held over the picture.
    spherize: wgpu::ComputePipeline,
    /// Ripple (docs/08 §3.53): rings spreading from a point.
    ripple: wgpu::ComputePipeline,
    /// Wave warp (docs/08 §3.54): a travelling wave across the frame.
    wave_warp: wgpu::ComputePipeline,
    /// Bezier warp (docs/08 §3.55): the frame's four edges bent, inverted per
    /// pixel by Newton's method — the first kernel in the catalogue to *solve*
    /// for its sample position rather than compute one.
    bezier_warp: wgpu::ComputePipeline,
    /// Warp (docs/08 §3.56): the thirteen bend presets, one kernel.
    warp: wgpu::ComputePipeline,
    /// Roughen edges (docs/08 §3.57), the second pass: the blurred alpha re-cut
    /// at a threshold the §3.37 fractal field wobbles. The blur itself is
    /// [`Self::blur`], reused exactly as Drop shadow reuses it — and here the
    /// blurred alpha *is* the distance field. The fourth reader of
    /// `fx_noise_core.wgsl`.
    roughen_edges: wgpu::ComputePipeline,
    /// Median (docs/08 §3.64, K-405): the true middle value of a neighbourhood,
    /// selected by a compare-exchange network so that nothing branches on a
    /// value and the four channels come out of one sweep. The catalogue's only
    /// `heavy` single-pass kernel.
    median: wgpu::ComputePipeline,
    /// Mosaic (docs/08 §3.65): the frame in flat blocks, every boundary an
    /// integer division.
    mosaic: wgpu::ComputePipeline,
    /// Find edges (docs/08 §3.66): a Sobel gradient per channel, taken on the
    /// perceptual value so the lines land where a person would draw them.
    find_edges: wgpu::ComputePipeline,
    /// Emboss (docs/08 §3.67): the picture as grey relief, lit from Direction.
    emboss: wgpu::ComputePipeline,
    /// Texturize (docs/08 §3.68): §3.67's relief taken from another layer and
    /// multiplied into this one. The second kernel after Light wrap to read a
    /// layer of its own beside the universal Matte row.
    texturize: wgpu::ComputePipeline,
    /// Broadcast safe (docs/08 §3.69): the composite signal's amplitude measured
    /// and clamped, or the pixels that fail keyed out.
    broadcast_safe: wgpu::ComputePipeline,
    /// Channel blur (docs/08 §3.45): the separable gaussian with a radius per
    /// channel. Its own kernel rather than a fourth mode of [`Self::blur`] —
    /// four weight tables cannot be one table, and widening the shipped blur's
    /// uniform would put its byte-for-byte guarantee at risk for nothing.
    channel_blur: wgpu::ComputePipeline,
    /// Drop shadow (docs/08 §3.43), the combine pass: the softened shape read
    /// at the offset, painted and composited *under* the layer. The softening
    /// itself is [`Self::blur`], reused exactly as the Glow and Light wrap
    /// reuse it.
    drop_shadow: wgpu::ComputePipeline,
    /// Set matte (docs/08 §3.44): another layer's channel becomes this layer's
    /// alpha. The sixth kernel to claim the K-395 matte inside its own maths,
    /// and the only one for which the matte *is* the output rather than a
    /// modifier of it.
    set_matte: wgpu::ComputePipeline,
    set_channels: wgpu::ComputePipeline,
    /// Linear wipe (docs/08 §3.46): a straight edge swept across the frame.
    linear_wipe: wgpu::ComputePipeline,
    /// Radial wipe (docs/08 §3.47): a wedge swept round a centre.
    radial_wipe: wgpu::ComputePipeline,
    /// Venetian blinds (docs/08 §3.70): §3.46's straight edge, folded into one
    /// slat so that one edge becomes a rank of them.
    venetian_blinds: wgpu::ComputePipeline,
    /// Iris wipe (docs/08 §3.71): a polygon or a star opened out of the middle.
    /// The shape is never rasterised — the pixel's angle is folded into one
    /// sector, which reduces the whole boundary to a straight edge.
    iris_wipe: wgpu::ComputePipeline,
    /// Card wipe (docs/08 §3.72): the frame as a grid of cards, turning away.
    /// The first kernel to put a camera in front of a pixel, and it *inverts*
    /// the projection rather than drawing it — Lumit's effects gather.
    card_wipe: wgpu::ComputePipeline,
    transform: wgpu::ComputePipeline,
    /// The shake's own motion blur (docs/08 §3.4, T18/K-165): averages the
    /// shake resampled at its motion-blur sub-frames. Its own kernel rather
    /// than the Transform kernel because it reads the input at several affines
    /// in one pass; it uses the shared two-input layout all the same.
    shake_mb: wgpu::ComputePipeline,
    glow_bright: wgpu::ComputePipeline,
    glow_combine: wgpu::ComputePipeline,
    block_glitch: wgpu::ComputePipeline,
    scanlines: wgpu::ComputePipeline,
    echo_accumulate: wgpu::ComputePipeline,
    /// Accumulation motion blur's per-pixel shutter (docs/08 §3.26,
    /// K-429): one dispatch per sub-frame render, folding it into the
    /// average at a weight the Matte decides. On the shared fx layout,
    /// because it is the ordinary shape — two pictures in, one out, and
    /// a matte.
    accum_shutter: wgpu::ComputePipeline,
    echo_mix: wgpu::ComputePipeline,
    motion_blur: wgpu::ComputePipeline,
    /// The dominant-motion tile reduction Motion blur runs first (K-390,
    /// docs/impl/optical-flow.md §4.5 item 3): one thread per tile, reducing
    /// the flow field to the confidence-weighted longest vector per tile. Its
    /// own [`Self::mb_tile_layout`] because the tile texture is rgba32float —
    /// those vectors are judged bit-for-bit against an f32 oracle.
    mb_tilemax: wgpu::ComputePipeline,
    /// A supplied **Motion vectors** layer turned into a flow field
    /// (K-429, docs/08 §3.2): the same layout as the reduction above,
    /// because it is the same shape of pass — a picture in, an
    /// rgba32float field out. Everything downstream then reads one kind
    /// of field and knows nothing about where it came from.
    mb_vectors: wgpu::ComputePipeline,
    /// Datamosh (docs/08 §3.12, K-104): shares [`Self::mb_layout`]/`mb_pl`
    /// with Motion blur — both need exactly three sampled inputs (the
    /// current frame, one extra neighbour-derived texture, and a flow
    /// field) plus a storage output and a uniform.
    datamosh: wgpu::ComputePipeline,
    adjust: wgpu::ComputePipeline,
    /// The generic Matte dissolve (K-395, docs/08 §2.6): one pass after any
    /// effect that was handed a matte, lerping its output back towards its
    /// input by the matte's luma. Shares [`Self::adjust_layout`] — three
    /// sampled inputs, a storage output, one uniform.
    matte_mix: wgpu::ComputePipeline,
    /// The matte's Channel pick and Invert (K-425), once before any kernel or
    /// the dissolve reads it. Shares [`Self::adjust_layout`].
    matte_prepare: wgpu::ComputePipeline,
    /// The effect Blend and Mix (K-425): one pass after a kernel run at Mix
    /// 100, blending its result onto its input by a layer mode and then
    /// applying the effect's own Mix. Shares [`Self::adjust_layout`].
    blend_mix: wgpu::ComputePipeline,
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
    /// Lens flare (docs/08 §3.27, K-256): the one effect that owns a render
    /// pass — its pipelines, layouts and bake cache live in their own
    /// sub-struct rather than six more fields here.
    lens_flare: lens_flare::LazyFlare,
    /// Particulate (docs/08 §3.86, K-446): the second effect to own a render
    /// pass, and the first to own a compute pipeline that writes a buffer
    /// rather than a picture. Its four passes and two layouts are built with
    /// the rest — there is nothing lazy about it, because a particle system
    /// that stalls on first use stalls in the middle of a scrub.
    particulate_layout: wgpu::BindGroupLayout,
    particulate_draw_layout: wgpu::BindGroupLayout,
    particulate_empty_layout: wgpu::BindGroupLayout,
    particulate_alive: wgpu::ComputePipeline,
    particulate_scan: wgpu::ComputePipeline,
    particulate_blocks: wgpu::ComputePipeline,
    particulate_scatter: wgpu::ComputePipeline,
    particulate_draw: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The adjustment blend's own layout: three sampled inputs (below,
    /// processed, coverage) where every effect kernel takes two.
    adjust_layout: wgpu::BindGroupLayout,
    /// Flow motion blur's own layout: three sampled inputs — the source (which
    /// doubles as the unprocessed original for Mix, this being a single pass),
    /// the dominant-motion tiles and the flow field. Also Datamosh's layout
    /// (see [`Self::datamosh`]): its three sampled inputs (current, previous,
    /// flow) fit the same shape.
    mb_layout: wgpu::BindGroupLayout,
    /// The tile reduction's layout (see [`Self::mb_tilemax`]): the flow field
    /// in (0), the rgba32float tile texture out (1), the uniform (2).
    mb_tile_layout: wgpu::BindGroupLayout,
    /// The LUT lookup's own layout (see [`Self::lut`]): src (0), orig (1),
    /// the storage output (2), the uniform (3) and the 3D cube texture (4).
    lut_layout: wgpu::BindGroupLayout,
}

impl FxEngine {
    /// Let this engine make a Lens flare's bake **beside** the frame rather
    /// than inside it (K-350).
    ///
    /// Off by default, and that default is the safe one: an engine nobody has
    /// told otherwise bakes inside the frame exactly as it always did, so a
    /// path that has not opted in — the exporter's, which builds its own
    /// engine on its own device — cannot draw a provisional picture by
    /// omission. The Viewer's engine turns it on, and a frame whose lens is
    /// still being baked draws the lens the last frame drew (or, with none
    /// yet, no flare) instead of stopping for half a second of optics.
    pub fn set_deferred_flare_bakes(&self, deferred: bool) {
        self.lens_flare.set_deferred(deferred);
    }

    /// Whether a flare bake is being made right now.
    ///
    /// Answered without waiting for the flare's own pipelines to finish
    /// compiling (see [`lens_flare::LazyFlare`]): nothing can be baking before
    /// there is anything to bake with.
    #[must_use]
    pub fn flare_bake_pending(&self) -> bool {
        self.lens_flare
            .ready()
            .is_some_and(lens_flare::LensFlareFx::bake_pending)
    }

    /// A number that moves whenever a flare bake is queued or lands.
    ///
    /// What a caller compares either side of a render to answer the only
    /// question deferring the bake raises: *did this frame draw the lens its
    /// parameters name?* If the number moved, it may not have, and the frame
    /// must not be filed under a name that says it did — the frame caches are
    /// keyed by what is *in* a frame (K-178), and an entry that lies about
    /// that outlives every edit and undo that might have fixed it.
    #[must_use]
    pub fn flare_bake_generation(&self) -> u64 {
        // Zero until the flare engine exists, and honestly so: a bake cannot
        // have been queued or landed before there is one.
        self.lens_flare.ready().map_or(0, |lf| {
            lf.generation.load(std::sync::atomic::Ordering::Relaxed)
        })
    }

    /// How many times a frame has drawn a lens flare with **other** optics
    /// than its parameters name (K-431) — the deferred fallback to the lens
    /// the last frame drew, or no flare at all with none drawn yet.
    ///
    /// Read either side of a render, this is the exact answer to *may this
    /// frame be filed under the name taken before it?* If the number did not
    /// move, every flare in the frame drew the bake its parameters name and
    /// the name describes the pixels; if it moved, it does not, and the frame
    /// is made but not kept (the tiers are keyed by what is *in* a frame,
    /// K-178).
    ///
    /// It replaces [`Self::flare_bake_generation`] for that job. The
    /// generation moves whenever any bake is *queued* — a keyframed aperture
    /// keeps one queued permanently, which made every frame of every comp
    /// unbankable, flare or no flare. It stays for the other job it does:
    /// noticing that a bake has landed and the picture is worth making again.
    #[must_use]
    pub fn flare_substitutions(&self) -> u64 {
        self.lens_flare.ready().map_or(0, |lf| {
            lf.substitutions.load(std::sync::atomic::Ordering::Relaxed)
        })
    }

    /// Start making a lens's bake now, before any frame asks to draw it.
    ///
    /// The same queue a deferred miss uses, offered by name so a caller that
    /// knows a lens is *about* to be wanted can have the optics started
    /// early — and so the rule that a frame made while a bake is in flight is
    /// unnameable can be checked without waiting on a real half-second of it.
    ///
    /// Answers whether it was queued: `false` when the key is already held or
    /// already baking, or when this machine gave us no bake thread. Queueing
    /// is never required — a miss makes the bake either way.
    pub fn warm_flare_bake(&self, key: u64, bake: &lens_flare::FlareBake) -> bool {
        // The one flare call that *does* wait for the pipelines: a caller
        // asking for a bake by name has decided a flare is wanted.
        self.lens_flare.get().warm(key, bake)
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
        self.dispatch_matted(ctx, pipeline, src, orig, None, dst, w, h, params)
    }

    /// [`Self::dispatch`] with a **Matte** bound (K-395, docs/08 §2.6) — for
    /// the kernels that claim the matte inside their own maths, scaling the
    /// control the effect names (a blur's radius, a colour amount, a
    /// distortion's displacement: K-426, K-427) instead of taking the generic
    /// dissolve.
    ///
    /// `None` binds `src` in the matte's place. A texture binding cannot be left
    /// empty and a dummy 1×1 would be a second allocation per dispatch; the
    /// kernels never read it, because the uniform they were handed says the
    /// matte is off. That is the same "bound but not read" convention the `orig`
    /// slot already uses on the passes that pass `src` twice.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_matted(
        &self,
        ctx: &GpuContext,
        pipeline: &wgpu::ComputePipeline,
        src: &wgpu::Texture,
        orig: &wgpu::Texture,
        matte: Option<&wgpu::Texture>,
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
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        &matte.unwrap_or(src).create_view(&Default::default()),
                    ),
                },
            ],
        });
        let mut enc = ctx.encoder("fx-enc");
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fx-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, &bind, &[]);
            cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(enc);
    }
}

/// Fit a working texture into an `nw × nh` raster, centred: a larger target is
/// padded with transparent, a smaller one takes the middle out.
///
/// **In plain terms.** One effect — Tile (docs/08 §3.39, K-542) — can hand back
/// a bigger picture than it was given. Everything downstream of it works texel
/// by texel against pictures that must agree on their size, and a few places
/// cannot take a bigger picture at all (an adjustment layer blends against the
/// composite beneath it, which is comp-sized by definition). This is the one
/// function both cases go through: it grows a picture into its new margin, or
/// crops it back to the frame, and it costs *nothing at all* when the size
/// already matches — which is every effect but that one.
///
/// A plain `copy_texture_to_texture` of the overlap, after a clearing render
/// pass, so no pipeline and no sampling is involved: a pixel that survives is
/// the same pixel, bit for bit.
#[must_use]
pub fn fit_centred(ctx: &GpuContext, tex: wgpu::Texture, nw: u32, nh: u32) -> wgpu::Texture {
    if tex.width() == nw && tex.height() == nh {
        return tex;
    }
    let out = work_texture(ctx, nw, nh, "fx-fit-centred");
    let (cw, ch) = (tex.width().min(nw), tex.height().min(nh));
    let src_origin = wgpu::Origin3d {
        x: (tex.width() - cw) / 2,
        y: (tex.height() - ch) / 2,
        z: 0,
    };
    let dst_origin = wgpu::Origin3d {
        x: (nw - cw) / 2,
        y: (nh - ch) / 2,
        z: 0,
    };
    let mut enc = ctx.encoder("fx-fit-centred");
    {
        // Clear first and explicitly: the copy below writes only the overlap,
        // and a margin that was never written is a margin whose contents are
        // the driver's business, not ours.
        let view = out.create_view(&Default::default());
        let _ = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fx-fit-clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    enc.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: src_origin,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: &out,
            mip_level: 0,
            origin: dst_origin,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: cw,
            height: ch,
            depth_or_array_layers: 1,
        },
    );
    drop(enc);
    out
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
