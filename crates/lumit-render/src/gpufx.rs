//! The GPU half of a migrated effect: name in, texture out
//! (docs/impl/effect-registry.md §2.5).
//!
//! **In plain terms.** An effect that has moved to the registry no longer has a
//! variant in the big enum, so `run_ops` cannot reach its kernel with a `match`
//! arm any more. This module is what it reaches instead: a small list of
//! wrappers, each naming one effect and knowing the single `FxEngine` call that
//! draws it. Looking one up by name gives something callable, so adding an
//! effect adds a line here rather than an arm there.
//!
//! **Why it lives in `lumit-render` and not beside the declaration.**
//! `lumit-gpu` only *dev*-depends on `lumit-core` (docs/05 crate table), so the
//! kernels cannot be named from the same file as the schema. `lumit-render`
//! depends on both, which makes it the only place the two halves can meet. The
//! join is by `match_name` string, and a typo there is a missing effect at run
//! time rather than a compile error — which is why
//! `every_migrated_effect_has_a_gpu_entry` is not optional.
//!
//! The wrappers are deliberately thin. None of them does arithmetic: each reads
//! the effect's own typed struct out of the resolved bag and asks it for the
//! numbers the kernel wants (`packed`), so the CPU reference and the WGSL kernel
//! multiply by values that came from one expression, not two.

use lumit_core::fx::effects;
use lumit_core::fx::{EffectMetadata, Params};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

use crate::fxops::LoadedLut;

pub mod ofx;

type Tex = wgpu::Texture;

/// Which parallel input list an effect consumes a slot of
/// (docs/impl/effect-registry.md §2.5a, K-387).
///
/// **In plain terms.** A few effects need something the *render* prepared, not
/// something the user typed: a parsed `.cube`, another layer's picture, the
/// frames either side of this one, the motion field. Those arrive as lists
/// running alongside the stack, and which entry of a list belongs to which op is
/// settled by counting: the k-th LUT op takes the k-th cube. This says which
/// list an effect counts along, so `run_ops` can advance that counter and hand
/// the slot over without knowing anything else about the effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxKind {
    /// Nothing beside the op — every effect but a handful.
    None,
    /// The parallel LUT list (docs/08 §3.11): one parsed cube per `lut` op.
    Lut,
    /// The Lens flare's custom prescription (K-264): one `.lens` file per flare
    /// op. Its Matte source used to be counted off this same index; since K-395
    /// it is the generic matte, and rides on [`AuxSlot::matte`].
    LensFile,
    /// The layer's decoded neighbour frames — the whole list, never per-op.
    Neighbours,
    /// The layer's decoded motion — the dense field and the neighbour frames
    /// beside it, both whole, never per-op.
    FlowField,
}

/// Everything the render prepared beside one op: the [`AuxData`] its
/// [`AuxKind`] asked for, and — independently of that — the generic **Matte**
/// this op is driven by, when the effect claims it inside its own maths (K-395).
///
/// **Why the matte is a field rather than a seventh [`AuxKind`].** Every effect
/// can take a matte, including the ones that already consume a different aux
/// list: the Lens flare wants its prescription *and* its matte, Depth of field's
/// matte is a whole picture and Blur's is too. Making it a variant would mean a
/// combinatorial variant per (kind, matte) pair, and the first effect that
/// wanted both would add a seam. It rides beside instead, so the schema's
/// [`MatteRole`](lumit_core::fx::MatteRole) is the *only* thing that decides who
/// consumes the matte — one seam, whatever else the effect happens to need.
#[derive(Clone, Copy)]
pub struct AuxSlot<'a> {
    data: AuxData<'a>,
    matte: Option<&'a Tex>,
    layer: Option<&'a Tex>,
    paths: &'a [lumit_core::mask::MaskPolyline],
    schedule: Option<&'a lumit_core::fx::points::PointsSchedule>,
    /// Which effect instance this op is (K-593), and the layer time its
    /// parameters were evaluated at.
    op: (uuid::Uuid, f64),
}

/// The borrowed slot itself, as [`crate::fxops::run_ops`] resolved it.
///
/// A missing input is `None` (or an empty list), which every effect that takes
/// one renders as a passthrough: an unset LUT, a dangling layer reference or a
/// dropped decode degrades the picture, it never faults
/// (14-ENGINEERING-RULES §4).
#[derive(Clone, Copy)]
pub enum AuxData<'a> {
    /// The effect declared [`AuxKind::None`].
    None,
    /// This op's parsed cube, or `None` when the file was unset, missing, 1D or
    /// unreadable.
    Lut(Option<&'a LoadedLut>),
    /// The flare's custom prescription (content hash and text), absent when the
    /// `.lens` row is unset or the file would not parse. Its Matte source is
    /// **not** here: since K-395 that is the generic matte every effect can
    /// take, and it arrives on [`AuxSlot::matte`] like everyone else's.
    LensFile(Option<&'a (u64, String)>),
    /// Every decoded neighbour frame, keyed by offset; empty unless the stack
    /// asked for a temporal window.
    Neighbours(&'a [(i32, Tex)]),
    /// The layer's decoded motion: the dense field at this raster if one was
    /// computed, and the neighbour frames it displaces. Both come off the one
    /// decode, and Datamosh reads both — the field to walk along, the −1 frame
    /// to drag — which is why they arrive together, as the flare's matte and its
    /// prescription do. Fast motion blur ignores the frames.
    FlowField {
        field: Option<&'a Tex>,
        neighbours: &'a [(i32, Tex)],
    },
}

impl<'a> AuxSlot<'a> {
    /// Bundle one op's aux data with its generic matte, its auxiliary layer
    /// and its mask path.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: AuxData<'a>,
        matte: Option<&'a Tex>,
        layer: Option<&'a Tex>,
        paths: &'a [lumit_core::mask::MaskPolyline],
        schedule: Option<&'a lumit_core::fx::points::PointsSchedule>,
        instance: uuid::Uuid,
        lt: f64,
    ) -> Self {
        Self {
            data,
            matte,
            layer,
            paths,
            schedule,
            op: (instance, lt),
        }
    }

    /// **Which effect instance this op is** (K-593), and the layer time its
    /// parameters were evaluated at.
    ///
    /// Every built-in ignores it: a built-in's picture is a function of its bag,
    /// and two instances with the same numbers draw the same thing. A **plugin**
    /// needs both — the time is what its render is told the frame is, and the
    /// instance is which live copy of it renders, and which row wears the badge
    /// when it fails.
    pub fn op(self) -> (uuid::Uuid, f64) {
        self.op
    }

    /// **This op's birth schedule** (points-stream.md §3.3): the births this
    /// frame can still see, and the layer time to evaluate them at.
    ///
    /// A field beside the mask path, for the mask path's own reason and one
    /// more of its own: neither of these is a number anybody typed. The
    /// layer's clock is not a control, and the schedule is the *whole history*
    /// of the Emit rate track rather than its value now — so the resolved bag,
    /// which carries one evaluated number per declared row, has nowhere to put
    /// either. The draw builder holds both and threads them through.
    ///
    ///  for every effect but Particulate, and a default schedule — no
    /// births, time zero — is the documented passthrough.
    pub fn schedule(self) -> Option<&'a lumit_core::fx::points::PointsSchedule> {
        self.schedule
    }

    /// **This op's mask path** (K-408): the layer's chosen mask, flattened at
    /// this frame's time to an arc-length-parameterised polyline in px@comp.
    ///
    /// A field beside the matte rather than an [`AuxKind`] variant, for the
    /// reason the matte is one: an effect that walks a path may want a matte,
    /// a layer input or neighbours as well, and a variant per pair would add a
    /// seam to the first effect that wanted two things.
    ///
    /// `None` unless the effect declares a
    /// [`ParamKind::MaskPath`](lumit_core::fx::ParamKind::MaskPath) row, and
    /// [`MaskPolyline::is_empty`](lumit_core::mask::MaskPolyline::is_empty)
    /// whenever the row comes to no mask. Both mean the same thing to a kernel
    /// — render the input unchanged — which is why an effect need only check
    /// the polyline it got, never why it is missing.
    ///
    /// It arrives as a **CPU slice**, the way [`AuxData::Lut`]'s parsed cube
    /// does: whoever needs it on the GPU uploads it as a storage buffer at its
    /// own layout, since a kernel that walks a curve and a kernel that builds a
    /// distance field over one want different things in the buffer.
    pub fn mask_path(self) -> Option<&'a lumit_core::mask::MaskPolyline> {
        self.paths.first()
    }

    /// This op's **n-th** mask path (K-546), in the order the effect declares
    /// its [`ParamKind::MaskPath`](lumit_core::fx::ParamKind::MaskPath) rows.
    ///
    /// Three effects walk one line and answer with [`Self::mask_path`]; the
    /// Matte key takes two, an inside hold-out and an outside one, so the slot
    /// is a slice. Past the end is `None`, which is the same no-op an unset row
    /// already is.
    pub fn mask_path_n(self, i: usize) -> Option<&'a lumit_core::mask::MaskPolyline> {
        self.paths.get(i)
    }

    /// **This op's Matte** (K-395), already resolved against the picture the
    /// chain is carrying — so a matte pointed at the effect's own layer (K-288)
    /// is a real texture by the time it arrives here.
    ///
    /// `None` unless the effect's [`MatteRole`](lumit_core::fx::MatteRole) says
    /// it consumes the matte itself *and* a layer is bound: an effect on the
    /// generic strength semantic never sees one, because its matte is spent in
    /// the dissolve beside the dispatch instead. An override with an unset row
    /// gets `None` and must render exactly what it rendered before K-395
    /// (K-258) — which for all four of them is the same branch an unset row
    /// always took.
    pub fn matte(self) -> Option<&'a Tex> {
        self.matte
    }

    /// This op's cube. `None` for a missing slot *and* for a slot of the wrong
    /// kind — which cannot happen, since [`GpuEffect::aux`] is what chose the
    /// kind, and which is a passthrough rather than a panic if it ever does.
    pub fn lut(self) -> Option<&'a LoadedLut> {
        match self.data {
            AuxData::Lut(l) => l,
            _ => None,
        }
    }

    /// **This op's auxiliary layer** (K-123, K-429) — Light wrap's background
    /// plate, Texturize's Texture, Fast motion blur's Motion vectors, Set
    /// matte's source — already resolved against the picture the chain is
    /// carrying, so a reference to the effect's own layer (K-288) is a real
    /// texture by the time it arrives here.
    ///
    /// A field beside the matte rather than an [`AuxKind`] variant, for the
    /// reason the matte and the mask path are fields: an effect may want a
    /// layer *and* a flow field, and a variant per pair is a seam the first
    /// effect that wanted two things would have to add. `None` for an unset,
    /// missing or cyclic reference: the labelled no-op.
    pub fn layer_input(self) -> Option<&'a Tex> {
        self.layer
    }

    /// The decoded neighbour frames, empty when there are none — from either of
    /// the two kinds that carry them.
    pub fn neighbours(self) -> &'a [(i32, Tex)] {
        match self.data {
            AuxData::Neighbours(n) => n,
            AuxData::FlowField { neighbours, .. } => neighbours,
            _ => &[],
        }
    }

    /// The dense motion field, `None` when the decode computed none — a plain
    /// layer, or a dropped neighbour. The passthrough, never a fault.
    pub fn flow_field(self) -> Option<&'a Tex> {
        match self.data {
            AuxData::FlowField { field, .. } => field,
            _ => None,
        }
    }

    /// The Lens flare's custom prescription: `None` for an unset, missing or
    /// unparsable `.lens` file, which falls back to the picked library lens.
    pub fn lens_file(self) -> Option<&'a (u64, String)> {
        match self.data {
            AuxData::LensFile(l) => l,
            _ => None,
        }
    }
}

/// One migrated effect's GPU pass.
///
/// The arguments are what [`crate::fxops::run_ops`] already holds when an op
/// comes round: the engine, the device, the picture the chain is carrying, its
/// size, and whatever the render prepared beside the stack for it. `run` returns
/// the picture after the pass, which for most effects is a new texture.
pub trait GpuEffect: Sync + 'static {
    /// The stable name this answers to — the same `match_name` the effect's
    /// declaration carries in `lumit-core`.
    fn match_name(&self) -> &'static str;

    /// Which parallel input list this effect consumes a slot of.
    /// [`crate::fxops::run_ops`] advances the matching counter exactly as the
    /// old match arms did, so the enumeration in `build.rs` and the consumption
    /// here stay in step (K-387).
    fn aux(&self) -> AuxKind {
        AuxKind::None
    }

    /// Draw the effect, with its parameters read from the resolved bag and its
    /// side-table input (if it declared one) already bound.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex;
}

/// Every migrated effect's GPU pass. Order is irrelevant here (the Add-effect
/// menu reads the catalogue, not this), so it follows the catalogue's for the
/// benefit of anyone reading the two side by side.
static GPU_EFFECTS: &[&dyn GpuEffect] = &[
    &Blur,
    &DirectionalBlur,
    &RadialBlur,
    &Sharpen,
    &SharpenSimple,
    &SpriteFlare,
    &LightWrap,
    &RgbSplit,
    &ChromaticAberration,
    &Flash,
    &ColourBalance,
    &Saturation,
    &Vibrancy,
    &Vignette,
    &Exposure,
    &HueShift,
    &Contrast,
    &Gamma,
    &Temperature,
    &Lut,
    &Dof,
    &ChannelBlur,
    &Transform,
    &Shake,
    &Glow,
    &BlockGlitch,
    &Scanlines,
    &Datamosh,
    &TurbulentDisplace,
    &Tile,
    &Offset,
    &Mirror,
    &LensDistort,
    &CornerPin,
    &DisplacementMap,
    &PolarCoordinates,
    &Twirl,
    &Spherize,
    &Ripple,
    &WaveWarp,
    &BezierWarp,
    &Warp,
    &RoughenEdges,
    &Median,
    &Mosaic,
    &FindEdges,
    &Emboss,
    &Texturize,
    &Fill,
    &Gradient,
    &Noise,
    &FractalNoise,
    &Echo,
    &MotionBlur,
    &MatteKey,
    &SetMatte,
    &BroadcastSafe,
    &Invert,
    &Tint,
    &Curves,
    &Levels,
    &Brightness,
    &HueSaturation,
    &Posterize,
    &Threshold,
    &Tritone,
    &PhotoFilter,
    &BlackAndWhite,
    &ShadowHighlight,
    &LensFlare,
    &DropShadow,
    &LinearWipe,
    &RadialWipe,
    &VenetianBlinds,
    &IrisWipe,
    &CardWipe,
    &Beam,
    &Lightning,
    &RadioWaves,
    &Vegas,
    &AddGrain,
    &Scribble,
    &Stroke,
    &Particulate,
    &Grid,
    &Scatter,
    &CloneToPoints,
    &Trail,
    &ConnectPoints,
];

/// The passes registered while the program runs (K-593) — an OFX plugin's,
/// today, and the user's own in time. The built-in table above is searched
/// first, so nothing here can shadow a shipped effect.
static REGISTERED: std::sync::RwLock<Vec<&'static dyn GpuEffect>> =
    std::sync::RwLock::new(Vec::new());

/// Add a pass discovered at run time, beside the effect definition that
/// registered into `lumit-core`'s catalogue (K-593).
///
/// `false` — and nothing added — for a name the table already answers to, which
/// is what makes a rescan idempotent. Additive and never removes, so no frame in
/// flight can find its pass gone.
pub fn register_gpu_effect(effect: &'static dyn GpuEffect) -> bool {
    let name = effect.match_name();
    let Ok(mut registered) = REGISTERED.write() else {
        return false;
    };
    if GPU_EFFECTS.iter().any(|g| g.match_name() == name)
        || registered.iter().any(|g| g.match_name() == name)
    {
        return false;
    }
    registered.push(effect);
    true
}

/// The GPU pass for `match_name`, or `None` when the effect has no image
/// operation of its own — the orchestration-only case, which is a passthrough
/// rather than a fault (docs/impl/effect-registry.md §3).
///
/// A linear scan of a handful of `&'static str`s, called once per effect per
/// frame and never per pixel — the same shape `fx::schema` has always had.
pub fn gpu_effect(match_name: &str) -> Option<&'static dyn GpuEffect> {
    GPU_EFFECTS
        .iter()
        .copied()
        .find(|g| g.match_name() == match_name)
        .or_else(|| {
            registered_effects()
                .into_iter()
                .find(|g| g.match_name() == match_name)
        })
}

/// Every name this table answers to, for the test that holds it against the
/// catalogue. Built-ins first, then whatever registered at run time.
pub fn gpu_effect_names() -> impl Iterator<Item = &'static str> {
    GPU_EFFECTS
        .iter()
        .copied()
        .chain(registered_effects())
        .map(GpuEffect::match_name)
}

/// The run-time half, copied out — a handful of thin pointers, read rather than
/// borrowed so the callers above outlive the guard.
fn registered_effects() -> Vec<&'static dyn GpuEffect> {
    REGISTERED
        .read()
        .map(|list| list.clone())
        .unwrap_or_default()
}

struct Blur;
impl GpuEffect for Blur {
    fn match_name(&self) -> &'static str {
        "blur"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (radius_px, edge, mix) = effects::blur::Blur::read(p).packed();
        // **The blur claims its matte** (K-395): it scales the radius per pixel
        // rather than dissolving a finished blur, so the texture goes into the
        // kernel and no dissolve runs beside this op. With no matte bound the
        // kernel takes the branch it always took, byte for byte.
        fx.blur(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::BlurOp {
                radius_px,
                edge,
                mix,
            },
        )
    }
}

struct DirectionalBlur;
impl GpuEffect for DirectionalBlur {
    fn match_name(&self) -> &'static str {
        "directional_blur"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (length_px, angle_deg, edge, mix) =
            effects::directional_blur::DirectionalBlur::read(p).packed();
        // The unit direction and the tap count are derived here exactly as the
        // old `run_ops` arm derived them, from the same two numbers the CPU
        // reference derives them from.
        let (dx, dy) = lumit_core::fx::rgb_split_offset(1.0, angle_deg);
        fx.dir_blur(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::DirBlurOp {
                dx,
                dy,
                length_px,
                taps: lumit_core::fx::cpu::dir_blur_taps(length_px),
                edge,
                mix,
            },
        )
    }
}

struct RadialBlur;
impl GpuEffect for RadialBlur {
    fn match_name(&self) -> &'static str {
        "radial_blur"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (centre_px, amount_px, spin, edge, mix) =
            effects::radial_blur::RadialBlur::read(p).packed();
        fx.radial_blur(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::RadialBlurOp {
                centre_px,
                amount_px,
                taps: lumit_core::fx::cpu::radial_blur_taps(amount_px),
                spin,
                edge,
                mix,
            },
        )
    }
}

struct Sharpen;
impl GpuEffect for Sharpen {
    fn match_name(&self) -> &'static str {
        "sharpen"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (amount, radius_px, threshold, luma_only, mix) =
            effects::sharpen::Sharpen::read(p).packed();
        fx.sharpen(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::SharpenOp {
                amount,
                radius_px,
                threshold,
                luma_only,
                mix,
            },
        )
    }
}

struct SharpenSimple;
impl GpuEffect for SharpenSimple {
    fn match_name(&self) -> &'static str {
        "sharpen_simple"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (amount, radius, mix) = effects::sharpen_simple::SharpenSimple::read(p).packed();
        fx.sharpen_simple(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::SharpenSimpleOp {
                amount,
                radius,
                mix,
            },
        )
    }
}

struct SpriteFlare;
impl GpuEffect for SpriteFlare {
    fn match_name(&self) -> &'static str {
        "sprite_flare"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let s = effects::sprite_flare::SpriteFlare::read(p).packed();
        fx.sprite_flare(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::SpriteFlareOp {
                light: s.light,
                intensity: s.intensity,
                tint: s.tint,
                glow_size: s.glow_size,
                glow_intensity: s.glow_intensity,
                ghosts: s.ghosts,
                ghost_spacing: s.ghost_spacing,
                ghost_size: s.ghost_size,
                ghost_intensity: s.ghost_intensity,
                streak_length: s.streak_length,
                streak_intensity: s.streak_intensity,
                streak_angle_deg: s.streak_angle_deg,
                mix: s.mix,
            },
        )
    }
}

struct LightWrap;
impl GpuEffect for LightWrap {
    fn match_name(&self) -> &'static str {
        "light_wrap"
    }
    /// The Background plate is another layer, rendered alone at this raster —
    /// the layer-input carriage (K-358, K-387), which since K-429 is a field on
    /// the slot rather than a kind, read straight off `aux.layer_input()`. It is
    /// a *plate*, not a matte: the light in it spills round the foreground's
    /// edge, so it is not the same input as the Matte row this effect also
    /// carries, which dissolves the wrap's strength generically.
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (width_px, intensity, mix) = effects::light_wrap::LightWrap::read(p).packed();
        // An absent Background — unset, missing or cyclic — is the passthrough,
        // and so is any neutral setting: the old arm guarded both, and the CPU
        // reference guards the second internally, so the two paths agree about
        // which inputs draw nothing at all.
        match aux.layer_input() {
            Some(background) if width_px > 0.0 && intensity > 0.0 && mix > 0.0 => fx.light_wrap(
                ctx,
                tex,
                w,
                h,
                background,
                &lumit_gpu::fx::LightWrapOp {
                    width_px,
                    intensity,
                    mix,
                },
            ),
            _ => tex.clone(),
        }
    }
}

struct RgbSplit;
impl GpuEffect for RgbSplit {
    fn match_name(&self) -> &'static str {
        "rgb_split"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // The Wavelength tier runs a different kernel, which is why the effect's
        // `packed` answers with a mode rather than a tuple. The offset vector and
        // the spectral basis are derived here exactly as the old `run_ops` arms
        // derived them, from the same numbers the CPU reference derives them from.
        match effects::rgb_split::RgbSplit::read(p).packed() {
            effects::rgb_split::Split::Classic {
                amount_px,
                angle_deg,
                scale,
                tints,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(amount_px, angle_deg);
                fx.rgb_split(
                    ctx,
                    tex,
                    w,
                    h,
                    aux.matte(),
                    &lumit_gpu::fx::RgbSplitOp {
                        dx,
                        dy,
                        scale,
                        tints,
                        mix,
                    },
                )
            }
            effects::rgb_split::Split::Spectral {
                amount_px,
                angle_deg,
                samples,
                tints,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(amount_px, angle_deg);
                let (basis, count) = lumit_core::fx::spectral_basis_uniform(samples, tints);
                fx.spectral_split(
                    ctx,
                    tex,
                    w,
                    h,
                    aux.matte(),
                    &lumit_gpu::fx::SpectralSplitOp {
                        dx,
                        dy,
                        amount_px,
                        radial: false,
                        basis,
                        count,
                        mix,
                    },
                )
            }
        }
    }
}

struct ChromaticAberration;
impl GpuEffect for ChromaticAberration {
    fn match_name(&self) -> &'static str {
        "chromatic_aberration"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        match effects::chromatic_aberration::ChromaticAberration::read(p).packed() {
            effects::chromatic_aberration::Fringe::Classic {
                amount_px,
                tints,
                mix,
            } => fx.chromatic_aberration(
                ctx,
                tex,
                w,
                h,
                aux.matte(),
                &lumit_gpu::fx::ChromaticAberrationOp {
                    amount_px,
                    tints,
                    mix,
                },
            ),
            // The radial spectral split (K-144): the old arm passed angle 0.0,
            // so the offset vector is the same `(amount_px, 0)` it always was.
            effects::chromatic_aberration::Fringe::Spectral {
                amount_px,
                samples,
                tints,
                mix,
            } => {
                let (dx, dy) = lumit_core::fx::rgb_split_offset(amount_px, 0.0);
                let (basis, count) = lumit_core::fx::spectral_basis_uniform(samples, tints);
                fx.spectral_split(
                    ctx,
                    tex,
                    w,
                    h,
                    aux.matte(),
                    &lumit_gpu::fx::SpectralSplitOp {
                        dx,
                        dy,
                        amount_px,
                        radial: true,
                        basis,
                        count,
                        mix,
                    },
                )
            }
        }
    }
}

struct ColourBalance;
impl GpuEffect for ColourBalance {
    fn match_name(&self) -> &'static str {
        "colour_balance"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (lift, gamma, gain, mix) = effects::colour_balance::ColourBalance::read(p).packed();
        fx.colour_balance(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::ColourBalanceOp {
                lift,
                gamma,
                gain,
                mix,
            },
        )
    }
}

struct Saturation;
impl GpuEffect for Saturation {
    fn match_name(&self) -> &'static str {
        "saturation"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (saturation, mix) = effects::saturation::Saturation::read(p).packed();
        fx.saturation(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::SaturationOp { saturation, mix },
        )
    }
}

struct Vibrancy;
impl GpuEffect for Vibrancy {
    fn match_name(&self) -> &'static str {
        "vibrancy"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (amount, mix) = effects::vibrancy::Vibrancy::read(p).packed();
        fx.vibrancy(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::VibrancyOp { amount, mix },
        )
    }
}

struct Flash;
impl GpuEffect for Flash {
    fn match_name(&self) -> &'static str {
        "flash"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        // Strength is the resolve-time envelope (K-385), already in the bag; the
        // wrapper does no time maths of its own, exactly as it does no arithmetic
        // of its own.
        let f = effects::flash::Flash::read(p);
        let (strength, colour, mix) = f.packed(effects::flash::Flash::strength_of(p));
        fx.flash(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::FlashOp {
                strength,
                colour,
                mix,
            },
        )
    }
}

struct Vignette;
impl GpuEffect for Vignette {
    fn match_name(&self) -> &'static str {
        "vignette"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (amount, radius, softness, roundness, ramp, mix) =
            effects::vignette::Vignette::read(p).packed();
        fx.vignette(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::VignetteOp {
                amount,
                radius,
                softness,
                roundness,
                ramp,
                mix,
            },
        )
    }
}

struct Exposure;
impl GpuEffect for Exposure {
    fn match_name(&self) -> &'static str {
        "exposure"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let e = effects::exposure::Exposure::read(p);
        let (factor, mix) = e.packed();
        fx.exposure(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::ExposureOp {
                stops: e.stops,
                factor,
                mix,
            },
        )
    }
}

struct HueShift;
impl GpuEffect for HueShift {
    fn match_name(&self) -> &'static str {
        "hue_shift"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let e = effects::hue_shift::HueShift::read(p);
        let (m, mix) = e.packed();
        fx.hue_shift(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::HueShiftOp {
                angle_rad: e.angle.to_radians(),
                preserve: e.preserve_luminance,
                m,
                mix,
            },
        )
    }
}

struct Contrast;
impl GpuEffect for Contrast {
    fn match_name(&self) -> &'static str {
        "contrast"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (k, mix) = effects::contrast::Contrast::read(p).packed();
        fx.contrast(ctx, tex, w, h, &lumit_gpu::fx::ContrastOp { k, mix })
    }
}

struct Gamma;
impl GpuEffect for Gamma {
    fn match_name(&self) -> &'static str {
        "gamma"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (gamma, mix) = effects::gamma::Gamma::read(p).packed();
        fx.gamma(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::GammaOp { gamma, mix },
        )
    }
}

struct Fill;
impl GpuEffect for Fill {
    fn match_name(&self) -> &'static str {
        "fill"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (colour, mix) = effects::fill::Fill::read(p).packed();
        fx.fill(ctx, tex, w, h, &lumit_gpu::fx::FillOp { colour, mix })
    }
}

struct Gradient;
impl GpuEffect for Gradient {
    fn match_name(&self) -> &'static str {
        "gradient"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let g = effects::gradient::Gradient::read(p).packed();
        fx.gradient(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::GradientOp {
                radial: g.radial,
                start: g.start,
                axis: g.axis,
                inv_len2: g.inv_len2,
                inv_len: g.inv_len,
                c0: g.c0,
                c1: g.c1,
                scatter: g.scatter,
                seed: g.seed,
                mix: g.mix,
            },
        )
    }
}

struct Noise;
impl GpuEffect for Noise {
    fn match_name(&self) -> &'static str {
        "noise"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let n = effects::noise::Noise::read(p);
        let (amount, gaussian, colour_noise, seed, tick, mix) =
            n.packed(effects::noise::Noise::tick_of(p));
        fx.noise(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::NoiseOp {
                amount,
                gaussian,
                colour_noise,
                seed,
                tick,
                mix,
            },
        )
    }
}

struct FractalNoise;
impl GpuEffect for FractalNoise {
    fn match_name(&self) -> &'static str {
        "fractal_noise"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let f = effects::fractal_noise::FractalNoise::read(p).packed();
        fx.fractal_noise(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::FractalNoiseOp {
                seed: f.field.seed,
                octaves: f.field.octaves,
                gain: f.field.gain,
                lacunarity: f.field.lacunarity,
                perlin: f.field.perlin,
                turbulent: f.field.turbulent,
                cycle: f.field.cycle,
                cos_sin: f.cos_sin,
                offset: f.offset,
                inv_scale: f.inv_scale,
                z: f.z,
                contrast: f.contrast,
                brightness: f.brightness,
                invert: f.invert,
                mix: f.mix,
            },
        )
    }
}

struct TurbulentDisplace;
impl GpuEffect for TurbulentDisplace {
    fn match_name(&self) -> &'static str {
        "turbulent_displace"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let t = effects::turbulent_displace::TurbulentDisplace::read(p).packed();
        // **Turbulent displace claims its matte** (K-395): it scales the
        // displacement vector rather than dissolving a finished warp, so the
        // texture goes into the kernel and no dissolve runs beside this op.
        fx.turbulent_displace(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::TurbulentDisplaceOp {
                seed_x: t.field.seed,
                seed_y: t.seed_y,
                octaves: t.field.octaves,
                gain: t.field.gain,
                lacunarity: t.field.lacunarity,
                cycle: t.field.cycle,
                offset: t.offset,
                inv_size: t.inv_size,
                z: t.z,
                amount: t.amount,
                axes: t.axes,
                pin: t.pin,
                inv_pin_band: t.inv_pin_band,
                mix: t.mix,
            },
        )
    }
}

struct Tile;
impl GpuEffect for Tile {
    fn match_name(&self) -> &'static str {
        "tile"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        // The four sizes are px@comp (K-558), so `packed` takes the raster they
        // are being drawn on and turns them into the fractions both kernels read.
        let t = effects::tile::Tile::read(p).packed(w as f32, h as f32);
        fx.tile(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::TileOp {
                centre: t.centre,
                tile_frac: t.tile_frac,
                output_frac: t.output_frac,
                phase: t.phase,
                mirror_edges: t.mirror_edges,
                horizontal_phase_shift: t.horizontal_phase_shift,
                mix: t.mix,
                // The oracle sizes the raster (K-542): lumit-gpu has no
                // lumit-core to ask, and one rule in one place is what keeps the
                // two paths making the same picture.
                out_raster: lumit_core::fx::cpu::tile_raster(w, h, &t),
            },
        )
    }
}

struct Offset;
impl GpuEffect for Offset {
    fn match_name(&self) -> &'static str {
        "offset"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (shift, mix) = effects::offset::Offset::read(p).packed();
        fx.offset(ctx, tex, w, h, aux.matte(), shift, mix)
    }
}

struct Mirror;
impl GpuEffect for Mirror {
    fn match_name(&self) -> &'static str {
        "mirror"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (centre, normal, mix) = effects::mirror::Mirror::read(p).packed();
        fx.mirror(ctx, tex, w, h, centre, normal, mix)
    }
}

struct LensDistort;
impl GpuEffect for LensDistort {
    fn match_name(&self) -> &'static str {
        "lens_distort"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let l = effects::lens_distort::LensDistort::read(p).packed();
        fx.lens_distort(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::LensDistortOp {
                active: l.active,
                tan_half_fov: l.tan_half_fov,
                reverse: l.reverse,
                half_kind: l.half_kind,
                centre: l.centre,
                edge: l.edge,
                mix: l.mix,
            },
        )
    }
}

struct CornerPin;
impl GpuEffect for CornerPin {
    fn match_name(&self) -> &'static str {
        "corner_pin"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let c = effects::corner_pin::CornerPin::read(p).packed();
        fx.corner_pin(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::CornerPinOp {
                inv: c.inv,
                active: c.active,
                edge: c.edge,
                mix: c.mix,
            },
        )
    }
}

struct DisplacementMap;
impl GpuEffect for DisplacementMap {
    fn match_name(&self) -> &'static str {
        "displacement_map"
    }
    /// **Displacement map's matte IS its map** (K-395, docs/08 §3.49): the
    /// referenced layer rendered alone at this raster, arriving on the one matte
    /// carriage every effect's matte uses. It declares no other aux, so this
    /// stays [`AuxKind::None`].
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let d = effects::displacement_map::DisplacementMap::read(p).packed();
        fx.displacement_map(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::DisplacementMapOp {
                channels: d.channels,
                amount: d.amount,
                edge: d.edge,
                mix: d.mix,
                matte_invert: p.bool(lumit_core::fx::MATTE_INVERT_ID, false),
            },
        )
    }
}

struct PolarCoordinates;
impl GpuEffect for PolarCoordinates {
    fn match_name(&self) -> &'static str {
        "polar_coordinates"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let pc = effects::polar_coordinates::PolarCoordinates::read(p).packed();
        fx.polar_coordinates(ctx, tex, w, h, pc.to_polar, pc.interp, pc.mix)
    }
}

struct Twirl;
impl GpuEffect for Twirl {
    fn match_name(&self) -> &'static str {
        "twirl"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let t = effects::twirl::Twirl::read(p).packed();
        fx.twirl(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::TwirlOp {
                centre: t.centre,
                radius: t.radius,
                inv_radius: t.inv_radius,
                angle: t.angle,
                mix: t.mix,
            },
        )
    }
}

struct Spherize;
impl GpuEffect for Spherize {
    fn match_name(&self) -> &'static str {
        "spherize"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let s = effects::spherize::Spherize::read(p).packed();
        fx.spherize(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::SpherizeOp {
                centre: s.centre,
                radius: s.radius,
                inv_radius: s.inv_radius,
                bulge: s.bulge,
                mix: s.mix,
            },
        )
    }
}

struct Ripple;
impl GpuEffect for Ripple {
    fn match_name(&self) -> &'static str {
        "ripple"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let r = effects::ripple::Ripple::read(p).packed();
        fx.ripple(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::RippleOp {
                centre: r.centre,
                radius: r.radius,
                inv_radius: r.inv_radius,
                amount: r.amount,
                inv_width: r.inv_width,
                turns: r.turns,
                asymmetric: r.asymmetric,
                mix: r.mix,
            },
        )
    }
}

struct WaveWarp;
impl GpuEffect for WaveWarp {
    fn match_name(&self) -> &'static str {
        "wave_warp"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let v = effects::wave_warp::WaveWarp::read(p).packed();
        fx.wave_warp(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::WaveWarpOp {
                dir: v.dir,
                perp: v.perp,
                height: v.height,
                inv_width: v.inv_width,
                turns: v.turns,
                shape: v.shape,
                pin: v.pin,
                inv_pin_band: v.inv_pin_band,
                mix: v.mix,
            },
        )
    }
}

struct BezierWarp;
impl GpuEffect for BezierWarp {
    fn match_name(&self) -> &'static str {
        "bezier_warp"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let b = effects::bezier_warp::BezierWarp::read(p).packed();
        fx.bezier_warp(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::BezierWarpOp {
                pts: b.pts,
                steps: b.steps,
                mix: b.mix,
            },
        )
    }
}

struct Warp;
impl GpuEffect for Warp {
    fn match_name(&self) -> &'static str {
        "warp"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let a = effects::warp::Warp::read(p).packed();
        fx.warp(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::WarpOp {
                style: a.style,
                bend: a.bend,
                h_distort: a.h_distort,
                v_distort: a.v_distort,
                mix: a.mix,
            },
        )
    }
}

struct RoughenEdges;
impl GpuEffect for RoughenEdges {
    fn match_name(&self) -> &'static str {
        "roughen_edges"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let r = effects::roughen_edges::RoughenEdges::read(p).packed();
        // Border 0 is the exact identity (docs/08 §3.57 decision 3): a
        // zero-radius blur followed by a re-threshold would harden the
        // picture's own antialiasing for an effect the user has turned off.
        if !r.active {
            return tex.clone();
        }
        fx.roughen_edges(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::RoughenEdgesOp {
                seed: r.field.seed,
                octaves: r.field.octaves,
                gain: r.field.gain,
                lacunarity: r.field.lacunarity,
                cycle: r.field.cycle,
                flags: u32::from(r.field.perlin) | (u32::from(r.field.turbulent) << 1),
                offset: r.offset,
                inv_scale: r.inv_scale,
                z: r.z,
                border_px: r.border_px,
                influence: r.influence,
                half_width: r.half_width,
                colour: r.colour,
                colour_on: r.colour_on,
                mix: r.mix,
            },
        )
    }
}

struct Curves;
impl GpuEffect for Curves {
    fn match_name(&self) -> &'static str {
        "curves"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        // The five clamped-cubic tables are baked host-side, once, and the
        // CPU reference reads the identical ones (K-412).
        let c = effects::curves::Curves::read(p).packed();
        fx.curves(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::CurvesOp {
                t: c.t,
                neutral: c.neutral,
                mix: c.mix,
            },
        )
    }
}

struct Levels;
impl GpuEffect for Levels {
    fn match_name(&self) -> &'static str {
        "levels"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (r, mix) = effects::levels::Levels::read(p).packed();
        fx.levels(ctx, tex, w, h, &lumit_gpu::fx::LevelsOp { r, mix })
    }
}

struct Brightness;
impl GpuEffect for Brightness {
    fn match_name(&self) -> &'static str {
        "brightness"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (b, k, mix) = effects::brightness::Brightness::read(p).packed();
        fx.brightness(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::BrightnessOp { b, k, mix },
        )
    }
}

struct HueSaturation;
impl GpuEffect for HueSaturation {
    fn match_name(&self) -> &'static str {
        "hue_saturation"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (bands, mix) = effects::hue_saturation::HueSaturation::read(p).packed();
        fx.hue_saturation(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::HueSaturationOp { bands, mix },
        )
    }
}

struct Posterize;
impl GpuEffect for Posterize {
    fn match_name(&self) -> &'static str {
        "posterize"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (n, mix) = effects::posterize::Posterize::read(p).packed();
        fx.posterize(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::PosterizeOp { n, mix },
        )
    }
}

struct Threshold;
impl GpuEffect for Threshold {
    fn match_name(&self) -> &'static str {
        "threshold"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (level, half_width, mix) = effects::threshold::Threshold::read(p).packed();
        fx.threshold(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::ThresholdOp {
                level,
                half_width,
                mix,
            },
        )
    }
}

struct Tritone;
impl GpuEffect for Tritone {
    fn match_name(&self) -> &'static str {
        "tritone"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let t = effects::tritone::Tritone::read(p).packed();
        fx.tritone(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::TritoneOp {
                shadows: t.shadows,
                midtones: t.midtones,
                highlights: t.highlights,
                mix: t.mix,
            },
        )
    }
}

struct PhotoFilter;
impl GpuEffect for PhotoFilter {
    fn match_name(&self) -> &'static str {
        "photo_filter"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let f = effects::photo_filter::PhotoFilter::read(p).packed();
        fx.photo_filter(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::PhotoFilterOp {
                filter: f.filter,
                density: f.density,
                preserve: f.preserve,
                mix: f.mix,
            },
        )
    }
}

struct BlackAndWhite;
impl GpuEffect for BlackAndWhite {
    fn match_name(&self) -> &'static str {
        "black_and_white"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let b = effects::black_and_white::BlackAndWhite::read(p).packed();
        fx.black_and_white(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::BlackAndWhiteOp {
                weights: b.weights,
                tint: b.tint,
                tint_on: b.tint_on,
                mix: b.mix,
            },
        )
    }
}

struct ShadowHighlight;
impl GpuEffect for ShadowHighlight {
    fn match_name(&self) -> &'static str {
        "shadow_highlight"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let s = effects::shadow_highlight::ShadowHighlight::read(p).packed();
        // Nothing to lift, pull or steepen: the identity, and the gaussian is
        // not run (== the CPU reference early return).
        if !s.active {
            return tex.clone();
        }
        fx.shadow_highlight(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::ShadowHighlightOp {
                shadow: s.shadow,
                highlight: s.highlight,
                shadow_width: s.shadow_width,
                highlight_width: s.highlight_width,
                radius_px: s.radius_px,
                contrast: s.contrast,
                colour_correction: s.colour_correction,
                mix: s.mix,
            },
        )
    }
}

struct Temperature;
impl GpuEffect for Temperature {
    fn match_name(&self) -> &'static str {
        "temperature"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let e = effects::temperature::Temperature::read(p);
        let (gain_r, gain_b, mix) = e.packed();
        fx.temperature(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::TemperatureOp {
                t: e.t(),
                gain_r,
                gain_b,
                mix,
            },
        )
    }
}

struct Lut;
impl GpuEffect for Lut {
    fn match_name(&self) -> &'static str {
        "lut"
    }
    /// The k-th `lut` op binds the k-th cube (docs/08 §3.11) — the counter
    /// `run_ops` advances for this kind.
    fn aux(&self) -> AuxKind {
        AuxKind::Lut
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (mix, space) = effects::lut::Lut::read(p).packed();
        // An empty slot — unset, missing, 1D or unreadable file — is the
        // labelled no-op, exactly as the old arm's `if let Some` was; the
        // texture handle is an `Arc`, so passing it back costs nothing.
        match aux.lut() {
            Some(l) => fx.lut(
                ctx,
                tex,
                w,
                h,
                &l.texture,
                l.size,
                mix,
                space.code(),
                l.domain_min,
                l.domain_max,
            ),
            None => tex.clone(),
        }
    }
}

struct Dof;
impl GpuEffect for Dof {
    fn match_name(&self) -> &'static str {
        "dof"
    }
    /// **Depth of field's matte is its depth pass** (K-395): the referenced
    /// layer rendered alone at this raster, arriving on the one matte carriage
    /// every effect's matte uses. It declares no other aux, so this stays
    /// [`AuxKind::None`] — the matte is not an aux kind, it rides beside
    /// whatever kind an effect asks for.
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // An empty slot — unset, missing or cyclic — is the labelled no-op,
        // exactly as the old arm's `if let Some` was.
        let Some(depth) = aux.matte() else {
            return tex.clone();
        };
        // A slot only ever arrives for an op whose matte row names something
        // (`build.rs`'s `mattes_for` predicate), so a depth pass being *here* is
        // precisely what "a depth layer is bound" meant in the old resolve arm —
        // the fact the bag cannot carry, since a Layer row never reaches it.
        let d = effects::dof::Dof::read(p).packed(true, effects::dof::Dof::blades_of(p));
        fx.dof(
            ctx,
            tex,
            w,
            h,
            depth,
            &lumit_gpu::fx::DofOp {
                focus: d.focus,
                range: d.range,
                near_aperture: d.near_aperture,
                far_aperture: d.far_aperture,
                blade_normals: d.blade_normals,
                blade_count: d.blade_count,
                apothem2: d.apothem2,
                roundness: d.roundness,
                rim: d.rim,
                aspect_scale: d.aspect_scale,
                threshold: d.threshold,
                bokeh_power: d.bokeh_power,
                repeat_edge: d.repeat_edge,
                depth_bound: true,
                depth_channel: d.depth_channel,
                depth_invert: d.depth_invert,
                use_focus_point: d.use_focus_point,
                focus_point: d.focus_point,
                gamma: d.gamma,
                remove_edge_leak: d.remove_edge_leak,
                detect_edge_threshold: d.detect_edge_threshold,
                display: d.display,
                mix: d.mix,
            },
        )
    }
}

struct ChannelBlur;
impl GpuEffect for ChannelBlur {
    fn match_name(&self) -> &'static str {
        "channel_blur"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (radii, edge, mix) = effects::channel_blur::ChannelBlur::read(p).packed();
        fx.channel_blur(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::ChannelBlurOp { radii, edge, mix },
        )
    }
}

struct DropShadow;
impl GpuEffect for DropShadow {
    fn match_name(&self) -> &'static str {
        "drop_shadow"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let d = effects::drop_shadow::DropShadow::read(p).packed();
        fx.drop_shadow(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::DropShadowOp {
                colour: d.colour,
                opacity: d.opacity,
                offset: d.offset,
                softness_px: d.softness_px,
                shadow_only: d.shadow_only,
                mix: d.mix,
            },
        )
    }
}

struct SetMatte;
impl GpuEffect for SetMatte {
    fn match_name(&self) -> &'static str {
        "set_matte"
    }
    /// **Set matte's source is not a matte** (K-429): it is this effect's own
    /// Layer row, rendered alone at this raster and arriving on the ordinary
    /// auxiliary-layer carriage beside Light wrap's Background. The effect
    /// carries no universal Matte row at all, so no dissolve stands beside this
    /// kernel and Invert is applied once, inside it.
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (channel, combine, mix) = effects::set_matte::SetMatte::read(p).packed();
        fx.set_matte(
            ctx,
            tex,
            w,
            h,
            aux.layer_input(),
            &lumit_gpu::fx::SetMatteOp {
                channel,
                combine,
                invert: p.bool(lumit_core::fx::MATTE_INVERT_ID, false),
                mix,
            },
        )
    }
}

struct LinearWipe;
impl GpuEffect for LinearWipe {
    fn match_name(&self) -> &'static str {
        "linear_wipe"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let l = effects::linear_wipe::LinearWipe::read(p).packed();
        fx.linear_wipe(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::LinearWipeOp {
                centre: l.centre,
                normal: l.normal,
                completion: l.completion,
                band: l.band,
                mix: l.mix,
            },
        )
    }
}

struct RadialWipe;
impl GpuEffect for RadialWipe {
    fn match_name(&self) -> &'static str {
        "radial_wipe"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let r = effects::radial_wipe::RadialWipe::read(p).packed();
        fx.radial_wipe(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::RadialWipeOp {
                centre: r.centre,
                start: r.start,
                dir: r.dir,
                completion: r.completion,
                feather: r.feather,
                mix: r.mix,
            },
        )
    }
}

struct VenetianBlinds;
impl GpuEffect for VenetianBlinds {
    fn match_name(&self) -> &'static str {
        "venetian_blinds"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let v = effects::venetian_blinds::VenetianBlinds::read(p).packed();
        fx.venetian_blinds(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::VenetianBlindsOp {
                normal: v.normal,
                period: v.period,
                completion: v.completion,
                band: v.band,
                mix: v.mix,
            },
        )
    }
}

struct IrisWipe;
impl GpuEffect for IrisWipe {
    fn match_name(&self) -> &'static str {
        "iris_wipe"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let i = effects::iris_wipe::IrisWipe::read(p).packed();
        fx.iris_wipe(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::IrisWipeOp {
                centre: i.centre,
                vertex: i.vertex,
                normal: i.normal,
                period: i.period,
                rotation: i.rotation,
                band: i.band,
                active: i.active,
                mix: i.mix,
            },
        )
    }
}

struct CardWipe;
impl GpuEffect for CardWipe {
    fn match_name(&self) -> &'static str {
        "card_wipe"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let c = effects::card_wipe::CardWipe::read(p).packed(w as f32, h as f32);
        fx.card_wipe(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::CardWipeOp {
                grid: c.grid,
                completion: c.completion,
                inv_width: c.inv_width,
                one_minus_width: c.one_minus_width,
                order_axis: c.order_axis,
                order_bias: c.order_bias,
                order_scale: c.order_scale,
                axis: c.axis,
                direction: c.direction,
                randomness: c.randomness,
                seed: c.seed,
                mix: c.mix,
            },
        )
    }
}

struct Transform;
impl GpuEffect for Transform {
    fn match_name(&self) -> &'static str {
        "transform"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (anchor, position, scale, rotation_deg, opacity, mix) =
            effects::transform::Transform::read(p).packed();
        // The affine is the one lumit-core helper both paths build through, so
        // the CPU reference and the kernel consume identical numbers (K-031).
        let (m, off, opacity) =
            lumit_core::fx::transform_op(anchor, position, scale, rotation_deg, opacity);
        // No matte: the Transform keeps the strength dissolve (scaling a
        // whole-frame move is not a picture a per-pixel matte should draw;
        // the Shake is the one that claims this kernel's matte, K-427).
        fx.transform(
            ctx,
            tex,
            w,
            h,
            None,
            &lumit_gpu::fx::TransformOp {
                m,
                off,
                opacity,
                mix,
                // The Transform effect has no Edges control: a transparent
                // border, its long-standing behaviour.
                edge: 0,
            },
        )
    }
}

struct Shake;
impl GpuEffect for Shake {
    fn match_name(&self) -> &'static str {
        "shake"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use effects::shake::{Shake as ShakeParams, Shaken};
        // Shake dispatches the Transform kernel (docs/08 §3.4: a
        // transform-domain effect — perturb a virtual camera, resample once):
        // the shared affine turns the resolved wobble into the same op the CPU
        // reference builds, so both paths consume bit-identical numbers. With
        // its own motion blur on (T18/K-165) it builds one affine per sub-frame
        // and dispatches the averaging kernel instead, over the same sub-frames
        // `cpu::transform_average` averages. Shake's own Edges control governs
        // the border the wobble reveals, either way.
        let params = ShakeParams::read(p);
        match params.packed(ShakeParams::derived_of(p)) {
            Shaken::Plain { wobble, edge, mix } => {
                let (anchor, position, scale, rot) = lumit_core::fx::shake_affine(
                    w,
                    h,
                    wobble.offset_px,
                    wobble.rotation_deg,
                    wobble.zoom,
                );
                let (m, off, opacity) =
                    lumit_core::fx::transform_op(anchor, position, scale, rot, 1.0);
                // **The shake claims its matte** (K-427): it scales the
                // displacement the wobble gives each pixel, read where the
                // pixel lands, so a soft matte turns the shove into a warp.
                fx.transform(
                    ctx,
                    tex,
                    w,
                    h,
                    aux.matte(),
                    &lumit_gpu::fx::TransformOp {
                        m,
                        off,
                        opacity,
                        mix,
                        edge,
                    },
                )
            }
            Shaken::Blurred { samples, edge, mix } => {
                let mut taps = [lumit_gpu::fx::ShakeMbTap {
                    m: [1.0, 0.0, 0.0, 1.0],
                    off: [0.0, 0.0],
                }; lumit_gpu::fx::SHAKE_MB_SAMPLES];
                for (t, s) in taps.iter_mut().zip(samples.iter()) {
                    let (anchor, position, scale, rot) =
                        lumit_core::fx::shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
                    let (m, off, _opacity) =
                        lumit_core::fx::transform_op(anchor, position, scale, rot, 1.0);
                    *t = lumit_gpu::fx::ShakeMbTap { m, off };
                }
                fx.shake_mb(
                    ctx,
                    tex,
                    w,
                    h,
                    aux.matte(),
                    &lumit_gpu::fx::ShakeMbOp {
                        taps,
                        count: samples.len() as u32,
                        edge,
                        mix,
                    },
                )
            }
        }
    }
}

struct Glow;
impl GpuEffect for Glow {
    fn match_name(&self) -> &'static str {
        "glow"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (radius_px, threshold, knee, intensity, tint, mix) =
            effects::glow::Glow::read(p).packed();
        // **The glow claims its matte** (K-395): it gates the bright pass, so
        // the matte decides which pixels are allowed to seed the halo — light
        // still spills out of them across dark matte, which is the difference
        // from dissolving the finished glow.
        fx.glow(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::GlowOp {
                radius_px,
                threshold,
                knee,
                intensity,
                tint,
                mix,
            },
        )
    }
}

struct BlockGlitch;
impl GpuEffect for BlockGlitch {
    fn match_name(&self) -> &'static str {
        "block_glitch"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // The tick is the resolve-time discretised layer time (K-385), already
        // in the bag; the wrapper does no time maths of its own, exactly as it
        // does no arithmetic of its own.
        let b = effects::block_glitch::BlockGlitch::read(p);
        let (
            intensity,
            seed,
            tick,
            block_size_px,
            jitter_frac,
            amount_px,
            chan_px,
            slice_frac,
            mix,
        ) = b.packed(effects::block_glitch::BlockGlitch::tick_of(p));
        fx.block_glitch(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::BlockGlitchOp {
                intensity,
                seed,
                tick,
                block_size_px,
                jitter_frac,
                amount_px,
                chan_px,
                slice_frac,
                mix,
            },
        )
    }
}

struct Scanlines;
impl GpuEffect for Scanlines {
    fn match_name(&self) -> &'static str {
        "scanlines"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // Intensity carries an old project's folded Darkness and the roll is
        // this frame's offset — both resolve-time derivations (K-385), already
        // in the bag.
        let (i, r) = effects::scanlines::Scanlines::derived_of(p);
        let (intensity, period_px, roll_px, interlace, mix) =
            effects::scanlines::Scanlines::read(p).packed(i, r);
        fx.scanlines(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::ScanlinesOp {
                intensity,
                period_px,
                roll_px,
                interlace,
                mix,
            },
        )
    }
}

struct Datamosh;
impl GpuEffect for Datamosh {
    fn match_name(&self) -> &'static str {
        "datamosh"
    }
    /// Datamosh reads the layer's decoded motion — the current→previous flow
    /// field *and* the −1 neighbour it drags along it. Whole lists, so no
    /// counter advances for this kind.
    fn aux(&self) -> AuxKind {
        AuxKind::FlowField
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // Either input missing — a non-footage layer, or a dropped decode — is a
        // passthrough, never a fault, exactly as the old arm's tuple `if let`
        // was.
        let (Some(flow), Some((_, prev))) = (
            aux.flow_field(),
            aux.neighbours().iter().find(|(o, _)| *o == -1),
        ) else {
            return tex.clone();
        };
        let (ramp, reach) = effects::datamosh::Datamosh::derived_of(p);
        let (intensity, displacement, bloom, steps, mix) =
            effects::datamosh::Datamosh::read(p).packed(ramp, reach);
        fx.datamosh(
            ctx,
            tex,
            prev,
            flow,
            w,
            h,
            &lumit_gpu::fx::DatamoshOp {
                // The blend maths take a single fraction; Mix folds into
                // Intensity here rather than adding a second uniform, since
                // mixing the same two inputs twice collapses to one mix by the
                // product.
                intensity: intensity * mix,
                displacement,
                bloom,
                steps,
            },
        )
    }
}

struct Echo;
impl GpuEffect for Echo {
    fn match_name(&self) -> &'static str {
        "echo"
    }
    /// Echo reads the layer's decoded neighbour frames — the **whole** list, not
    /// a slot of it, so no counter advances for this kind. The render decoded
    /// exactly the offsets the effect's declared temporal window asked for.
    fn aux(&self) -> AuxKind {
        AuxKind::Neighbours
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (weights, mode, mix) = effects::echo::Echo::read(p).packed();
        // The kernel is called whether or not any neighbour arrived, exactly as
        // the old arm called it: an offset with no frame simply contributes
        // nothing, so a layer at its first frame trails off rather than
        // flickering between an echoed and an un-echoed picture.
        let by_offset: Vec<(i32, &Tex)> = aux.neighbours().iter().map(|(o, t)| (*o, t)).collect();
        fx.echo(
            ctx,
            tex,
            &by_offset,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::EchoOp { weights, mode, mix },
        )
    }
}

struct MotionBlur;
impl GpuEffect for MotionBlur {
    fn match_name(&self) -> &'static str {
        "motion_blur"
    }
    /// The streak follows the layer's dense motion field (with a confidence
    /// channel, FX-19), which the decode worker computed from the current and
    /// next source frames. A whole texture, so no counter advances for it.
    fn aux(&self) -> AuxKind {
        AuxKind::FlowField
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let (shutter_frac, samples, mix, view, quality, vector_scale) =
            effects::motion_blur::MotionBlur::read(p).packed();
        // A supplied Motion vectors layer stands in for the measured flow
        // (K-429): it is turned into the same field here, so everything below
        // reads one kind of field. It is also the one way this effect works on
        // a layer that has no measured flow at all.
        let supplied = aux
            .layer_input()
            .map(|v| fx.motion_vectors_field(ctx, v, w, h, vector_scale));
        // With no field of either kind (a plain layer, or a decode that dropped
        // the neighbour) it is a passthrough — never a fault.
        let Some(flow) = supplied.as_ref().or(aux.flow_field()) else {
            return tex.clone();
        };
        fx.motion_blur(
            ctx,
            tex,
            flow,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::MotionBlurOp {
                shutter_frac,
                samples,
                mix,
                view: view.code(),
                quality: quality.code(),
                vector_scale,
            },
        )
    }
}

/// One garbage mask's outline, as the kernel takes it (K-546): the CPU builds
/// the geometry — the same function the oracle calls — and this only re-labels
/// the fields for the GPU crate, exactly as `path_draw_op` does for the three
/// line-drawing effects.
fn mask_fill_op(built: &lumit_core::fx::cpu::MaskFillParams) -> lumit_gpu::fx::MaskFillOp {
    lumit_gpu::fx::MaskFillOp {
        segments: built.segments,
        count: built.count,
        ramp: built.ramp,
        expansion: built.expansion,
    }
}

struct MatteKey;
impl GpuEffect for MatteKey {
    fn match_name(&self) -> &'static str {
        "matte_key"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let k = effects::matte_key::MatteKey::read(p).packed();
        // The two garbage masks, in declaration order: inside then outside.
        // Built here, host-side, exactly once a frame and by the very function
        // the §1.6 oracle calls — neither path generates geometry, so neither
        // can generate it differently (K-408's rule, K-546's second row).
        let px_scale = effects::matte_key::MatteKey::px_scale_of(p);
        let unset = lumit_core::mask::MaskPolyline::default();
        let fill = |i: usize| {
            lumit_core::fx::cpu::mask_fill_params(aux.mask_path_n(i).unwrap_or(&unset), px_scale)
        };
        fx.matte_key(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::MatteKeyOp {
                view: k.view,
                key: k.key,
                gain: k.gain,
                balance: k.balance,
                despill_bias: k.despill_bias,
                alpha_bias: k.alpha_bias,
                spill: k.spill,
                clip_black: k.clip_black,
                clip_white: k.clip_white,
                clip_rollback: k.clip_rollback,
                replace_method: k.replace_method,
                replace_colour: k.replace_colour,
                pre_blur: k.pre_blur,
                shrink_grow: k.shrink_grow,
                softness: k.softness,
                despot_black: k.despot_black,
                despot_white: k.despot_white,
                mix: k.mix,
            },
            &mask_fill_op(&fill(0)),
            &mask_fill_op(&fill(1)),
        )
    }
}

struct Invert;
impl GpuEffect for Invert {
    fn match_name(&self) -> &'static str {
        "invert"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let mix = effects::invert::Invert::read(p).packed();
        fx.invert(ctx, tex, w, h, &lumit_gpu::fx::InvertOp { mix })
    }
}

struct Tint;
impl GpuEffect for Tint {
    fn match_name(&self) -> &'static str {
        "tint"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (black, white, mix) = effects::tint::Tint::read(p).packed();
        fx.tint(ctx, tex, w, h, &lumit_gpu::fx::TintOp { black, white, mix })
    }
}

struct LensFlare;
impl GpuEffect for LensFlare {
    fn match_name(&self) -> &'static str {
        "lens_flare"
    }
    /// The custom `.lens` prescription: the k-th flare op binds the k-th entry
    /// (K-264, K-387).
    ///
    /// **The Matte source is no longer counted here** (K-395). It used to ride
    /// beside the prescription off one index, because both were the flare's
    /// private business; it is now the generic matte every effect can take, and
    /// the flare is simply one of the four that claim it inside their own maths
    /// — it reaches [`AuxSlot::matte`] off the one carriage, not a list of the
    /// flare's own.
    fn aux(&self) -> AuxKind {
        AuxKind::LensFile
    }
    /// The one wrapper that is not thin, and could not be: the flare needs a
    /// **bake** — the prescription parsed, every ghost path ray-probed and ranked,
    /// the starburst transformed — which is far too heavy to redo per frame, so it
    /// is handed over as a closure the GPU side runs only when its parameter-hash
    /// cache misses, and may run beside the frame rather than inside it
    /// (`FxEngine::set_deferred_flare_bakes`, K-350).
    ///
    /// Everything below the bake is still the registry's rule: no arithmetic of
    /// its own. Every frame-time number comes out of the one `lumit-core` module
    /// that owns the formulas (K-031: the CPU reference and the kernels read
    /// identical values), through the effect's `packed`.
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use lumit_core::fx::lens_flare as lf;
        let (matte, custom) = (aux.matte(), aux.lens_file());
        let (lights, light_count) = effects::lens_flare::LensFlare::lights_of(p);
        let params = effects::lens_flare::LensFlare::read(p).packed(lights, light_count);
        let p = &params;

        let (tier_base, tier_lambda, flare_div) = lf::quality_ladder(p.quality);
        // The Detail dial scales the tier's base and wavelength count (K-265) —
        // through the shared helpers, so this equals the CPU reference.
        let grid = lf::detail_base(tier_base, p.detail);
        let lambda_count = lf::detail_lambda(tier_lambda, p.detail);
        let energy = p.ghost_intensity;
        // The traced bands with their eight radiometric sub-samples (K-364),
        // Ghost intensity folded into every sub-weight — the bake's
        // auto-exposure gain joins it GPU-side.
        let bands: Vec<lumit_gpu::fx::FlareBand> = lf::spectral_bands(lambda_count, p.dispersion)
            .into_iter()
            .map(|b| lumit_gpu::fx::FlareBand {
                traced_nm: b.traced_nm,
                sub_idx: b.sub_idx,
                sub_rgb: b
                    .sub_rgb
                    .map(|c| [c[0] * energy, c[1] * energy, c[2] * energy]),
            })
            .collect();
        let op = lumit_gpu::fx::LensFlareOp {
            // Raster pixels → fraction here, where the raster is known (K-260:
            // the parameter is px@comp).
            light_frac: [p.light[0] / w.max(1) as f32, p.light[1] / h.max(1) as f32],
            // Manual mode's lights: ONE entry per light, size and all (K-367). An
            // area source is no longer replicated into point samples — every ray
            // integrates the extent itself, so the extent travels with the light.
            manual_lights: lf::manual_light(p, w, h)
                .iter()
                .map(|l| {
                    [
                        l.pos[0],
                        l.pos[1],
                        l.rgb[0],
                        l.rgb[1],
                        l.rgb[2],
                        l.extent[0],
                        l.extent[1],
                    ]
                })
                .collect(),
            intensity: p.intensity,
            bands,
            max_ghosts: p.max_ghosts,
            coating: p.coating,
            focus_m: p.focus_m,
            fstop: p.fstop,
            blades: p.blades,
            aperture_rotation_deg: p.aperture_rotation_deg,
            roundness: p.roundness,
            aperture_softness: p.aperture_softness,
            ghost_softness: p.ghost_softness,
            grid,
            flare_div,
            screen_transform: lf::screen_transform(w),
            starburst_intensity: p.starburst_intensity,
            scale: p.scale,
            anamorphic: p.anamorphic,
            source: p.source,
            threshold: p.threshold,
            threshold_softness: p.threshold_softness,
            light_tint: p.light_tint,
            use_source_colour: p.use_source_colour,
            matte_invert: p.matte_invert,
            blend: p.blend,
            mix: p.mix,
            bake_key: lf::bake_key_with(p, custom.map(|(h, _)| *h)),
        };
        let custom_text = custom.map(|(_, text)| text.clone());
        // Manual mode's frame-time grid probe (K-267): the GPU hands back its
        // cached bake's tables and this closure runs the one lumit-core probe
        // both twins share, at the frame's actual light direction.
        let light_frac = op.light_frac;
        let aspect = h as f32 / w.max(1) as f32;
        let probe = move |pb: &lumit_gpu::fx::FlareProbeBake| {
            let needs = lf::frame_grid_needs_from_rows(
                pb.surfaces,
                pb.ghosts,
                pb.sensor_z_mm,
                pb.focal_mm,
                pb.pupil_mm,
                pb.start_z_mm,
                pb.pair_count,
                lf::light_direction(light_frac, aspect, pb.focal_mm),
                params.coating,
                lf::fstop_scale(pb.native_fstop, params.fstop),
                lf::focus_shift_mm(params.focus_m, pb.focal_mm),
            );
            lf::plan_frame_grids(grid, pb.spreads, &needs)
        };
        fx.lens_flare(
            ctx,
            tex,
            w,
            h,
            &op,
            matte,
            // The bake as something the bake thread can own and run (K-350): one
            // small `Arc` a flare a frame, beside a pass that traces hundreds of
            // thousands of rays. Whether it is actually run beside the frame or
            // inside it is the engine's policy, not this call's — see
            // `FxEngine::set_deferred_flare_bakes`.
            &(std::sync::Arc::new(move || {
                let b = lf::bake_with(&params, custom_text.as_deref());
                lumit_gpu::fx::FlareBakeData {
                    surfaces: b
                        .surfaces
                        .iter()
                        .map(|s| {
                            [
                                s.radius_mm,
                                s.z_mm,
                                s.semi_ap_mm,
                                s.cauchy_a,
                                s.cauchy_b,
                                s.coating_layers,
                                s.is_stop,
                                0.0,
                            ]
                        })
                        .collect(),
                    ghosts: b.pairs.clone(),
                    spreads: b.spreads.clone(),
                    sensor_z_mm: b.sensor_z_mm,
                    focal_mm: b.focal_mm,
                    native_fstop: b.native_fstop,
                    pupil_mm: b.pupil_mm,
                    start_z_mm: b.start_z_mm,
                    energy_gain: b.energy_gain,
                    reflectance: b.reflectance.clone(),
                    starburst: b.starburst,
                    sb_res: lf::STARBURST_RES,
                    sb_fields: lf::STARBURST_FIELDS as u32,
                }
            }) as lumit_gpu::fx::FlareBake),
            &probe,
        )
    }
}

struct Median;
impl GpuEffect for Median {
    fn match_name(&self) -> &'static str {
        "median"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let m = effects::median::Median::read(p).packed();
        // The network's run length, worked out here rather than in the kernel so
        // the CPU reference and the WGSL twin take the same rank of the same
        // sorted run (docs/08 §3.64 decision 1).
        let n = (2 * m.radius + 1) * (2 * m.radius + 1);
        fx.median(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::MedianOp {
                radius: m.radius,
                keep: (n + 1) / 2,
                alpha_on: f32::from(u8::from(m.alpha)),
                mix: m.mix,
            },
        )
    }
}

struct Mosaic;
impl GpuEffect for Mosaic {
    fn match_name(&self) -> &'static str {
        "mosaic"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let m = effects::mosaic::Mosaic::read(p).packed();
        fx.mosaic(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::MosaicOp {
                blocks: m.blocks,
                sharp: f32::from(u8::from(m.sharp)),
                mix: m.mix,
            },
        )
    }
}

struct FindEdges;
impl GpuEffect for FindEdges {
    fn match_name(&self) -> &'static str {
        "find_edges"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let (invert, mix) = effects::find_edges::FindEdges::read(p).packed();
        fx.find_edges(ctx, tex, w, h, &lumit_gpu::fx::FindEdgesOp { invert, mix })
    }
}

struct Emboss;
impl GpuEffect for Emboss {
    fn match_name(&self) -> &'static str {
        "emboss"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let e = effects::emboss::Emboss::read(p).packed();
        fx.emboss(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::EmbossOp {
                offset: e.offset,
                contrast: e.contrast,
                mix: e.mix,
            },
        )
    }
}

struct Texturize;
impl GpuEffect for Texturize {
    fn match_name(&self) -> &'static str {
        "texturize"
    }
    /// The Texture is another layer, rendered alone at this raster — the
    /// layer-input carriage (K-387, K-429), as Light wrap's Background is. It is
    /// **not** the Matte row this effect also carries: §3.49's map is its matte
    /// because a map has nothing else it could be, and a texture is not, because
    /// "press this canvas in" and "only over the sky" are two statements
    /// (docs/08 §3.68 decision 1).
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let t = effects::texturize::Texturize::read(p).packed();
        fx.texturize(
            ctx,
            tex,
            w,
            h,
            aux.layer_input(),
            aux.matte(),
            &lumit_gpu::fx::TexturizeOp {
                offset: t.offset,
                contrast: t.contrast,
                inv_scale: t.inv_scale,
                placement: t.placement,
                mix: t.mix,
            },
        )
    }
}

struct BroadcastSafe;
impl GpuEffect for BroadcastSafe {
    fn match_name(&self) -> &'static str {
        "broadcast_safe"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let b = effects::broadcast_safe::BroadcastSafe::read(p).packed();
        fx.broadcast_safe(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::BroadcastSafeOp {
                target: b.target,
                mode: b.mode,
                mix: b.mix,
            },
        )
    }
}

struct Beam;
impl GpuEffect for Beam {
    fn match_name(&self) -> &'static str {
        "beam"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        _aux: AuxSlot<'_>,
    ) -> Tex {
        let b = effects::beam::Beam::read(p).packed();
        fx.beam(
            ctx,
            tex,
            w,
            h,
            &lumit_gpu::fx::BeamOp {
                start: b.start,
                axis: b.axis,
                inv_len2: b.inv_len2,
                u0: b.u0,
                u1: b.u1,
                inv_span: b.inv_span,
                half0: b.half0,
                half1: b.half1,
                soft: b.soft,
                inside: b.inside,
                outside: b.outside,
                active: b.active,
                composite: b.composite,
                mix: b.mix,
            },
        )
    }
}

struct Lightning;
impl GpuEffect for Lightning {
    fn match_name(&self) -> &'static str {
        "lightning"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // The bolt is built here, host-side, exactly once a frame (docs/08
        // §3.74's first decision) — the kernel is handed the segments.
        let l = effects::lightning::Lightning::read(p).packed();
        fx.lightning(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::LightningOp {
                segments: l.segments,
                fades: l.fades,
                count: l.count,
                core_radius: l.core_radius,
                glow_radius: l.glow_radius,
                glow_opacity: l.glow_opacity,
                core_colour: l.core_colour,
                glow_colour: l.glow_colour,
                composite: l.composite,
                mix: l.mix,
            },
        )
    }
}

struct RadioWaves;
impl GpuEffect for RadioWaves {
    fn match_name(&self) -> &'static str {
        "radio_waves"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let r = effects::radio_waves::RadioWaves::read(p).packed();
        fx.radio_waves(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::RadioWavesOp {
                centre: r.centre,
                vertex: r.vertex,
                normal: r.normal,
                period: r.period,
                rotation: r.rotation,
                spin: r.spin,
                newest: r.newest,
                count: r.count,
                time: r.time,
                period_s: r.period_s,
                expansion: r.expansion,
                lifespan: r.lifespan,
                half_width: r.half_width,
                fade_in: r.fade_in,
                fade_out: r.fade_out,
                colour: r.colour,
                opacity: r.opacity,
                composite: r.composite,
                mix: r.mix,
            },
        )
    }
}

struct Vegas;
impl GpuEffect for Vegas {
    fn match_name(&self) -> &'static str {
        "vegas"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let vegas = effects::vegas::Vegas::read(p);
        // The Mask/Path half is the shared path drawing, not the level set: the
        // line is a mask's own, so there is no picture to take a gradient of
        // (K-408, docs/08 §3.76).
        if vegas.on_a_path() {
            let built = vegas.path_packed(&path_of(aux), effects::vegas::Vegas::px_scale_of(p));
            return fx.path_draw(ctx, tex, w, h, aux.matte(), &path_draw_op(&built));
        }
        let v = vegas.packed();
        fx.vegas(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::VegasOp {
                from_alpha: v.from_alpha,
                level: v.level,
                half_width: v.half_width,
                band: v.band,
                inv_segment: v.inv_segment,
                duty: v.duty,
                phase: v.phase,
                colour: v.colour,
                opacity: v.opacity,
                composite: v.composite,
                mix: v.mix,
            },
        )
    }
}

/// The three effects that draw a mask's own line share one pass, so they share
/// one way of filling its op (K-408). `p` is the resolved bag and `aux` the slot
/// the render prepared; an absent path is an empty polyline, which packs to a
/// count of zero and draws nothing.
fn path_draw_op(built: &lumit_core::fx::cpu::PathDrawParams) -> lumit_gpu::fx::PathDrawOp {
    lumit_gpu::fx::PathDrawOp {
        segments: built.segments,
        arcs: built.arcs,
        count: built.count,
        half_width: built.half_width,
        band: built.band,
        inv_segment: built.inv_segment,
        duty: built.duty,
        phase: built.phase,
        wiggle_amp: built.wiggle_amp,
        wiggle_freq: built.wiggle_freq,
        wiggle_tick: built.wiggle_tick,
        seed: built.seed,
        colour: built.colour,
        opacity: built.opacity,
        style: built.style,
        mix: built.mix,
    }
}

/// The polyline this op was handed, or an empty one — the same picture either
/// way, which is why no effect has to ask *why* it is missing (docs/08 §1.2).
fn path_of(aux: AuxSlot<'_>) -> lumit_core::mask::MaskPolyline {
    aux.mask_path().cloned().unwrap_or_default()
}

struct Scribble;
impl GpuEffect for Scribble {
    fn match_name(&self) -> &'static str {
        "scribble"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        // The hatch is laid out here, host-side, exactly once a frame — the
        // kernel is handed the strokes (docs/08 §3.78, §3.74's decision).
        use lumit_core::fx::effects::scribble::Scribble as S;
        let (px_scale, tick) = S::derived_of(p);
        let built = S::read(p).packed(&path_of(aux), px_scale, tick);
        fx.path_draw(ctx, tex, w, h, aux.matte(), &path_draw_op(&built))
    }
}

struct Stroke;
impl GpuEffect for Stroke {
    fn match_name(&self) -> &'static str {
        "stroke"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use lumit_core::fx::effects::stroke::Stroke as S;
        let built = S::read(p).packed(&path_of(aux), S::px_scale_of(p));
        fx.path_draw(ctx, tex, w, h, aux.matte(), &path_draw_op(&built))
    }
}

/// Particulate's GPU op, built from the resolved bag and the schedule threaded
/// beside it (docs/08 §3.86, points-stream.md §3.3).
///
/// Nothing here decides anything: `Particulate::points` and
/// `Particulate::draw_style` did the reducing, in the one expression the CPU
/// reference also reads, and this only lays the numbers out the way the kernel
/// wants them. The two things it *does* work out are the two the bag cannot
/// carry — the candidate window, and each recorded frame's distance from the
/// evaluation time, subtracted in `f64` so a two-second age never has to be
/// recovered from inside an hour-long clock.
#[allow(clippy::too_many_arguments)]
fn particulate_op<'a>(
    packed: &'a lumit_core::fx::points::PointsParams,
    style: lumit_core::fx::points::DrawStyle,
    sched: &lumit_core::fx::points::PointsSchedule,
    frames: &'a [[u32; 2]],
    curves: &'a [f32],
    path: &'a [[f32; 4]],
    path_total: f32,
    mode: u32,
    projection: Option<[[f32; 4]; 3]>,
) -> lumit_gpu::fx::ParticulateOp<'a> {
    let e = &packed.emitter;
    let look = &packed.particle;
    let f = &packed.forces;
    let dt = sched.schedule.dt();
    lumit_gpu::fx::ParticulateOp {
        emitter_pos: e.position,
        emitter_wh: [e.width, e.height],
        emitter_depth: e.depth,
        emitter_angle_deg: e.angle_deg,
        direction_deg: e.direction_deg,
        direction_z_deg: e.direction_z_deg,
        spread_deg: e.spread_deg,
        spread_z_deg: e.spread_z_deg,
        speed: e.speed,
        speed_jitter: e.speed_jitter,
        shape: e.shape as u32,
        seed: packed.seed,
        cap: packed.cap,
        life: look.life,
        life_jitter: look.life_jitter,
        size: look.size,
        size_jitter: look.size_jitter,
        rotation_deg: look.rotation_deg,
        rotation_jitter_deg: look.rotation_jitter_deg,
        spin_deg: look.spin_deg,
        align_to_motion: look.align_to_motion,
        colour: look.colour,
        end_colour: look.end_colour,
        wind: f.wind,
        gravity: f.gravity,
        drag: f.drag,
        turbulence: f.turbulence,
        turbulence_scale: f.turbulence_scale,
        turbulence_speed: f.turbulence_speed,
        // The **fixed** central difference the CPU reference takes, so one
        // frame key names one picture whatever raster is previewing.
        eps: ((dt * 0.5) as f32).max(1e-6),
        dt: dt as f32,
        first_birth: sched.schedule.first_birth(),
        frames,
        candidates: frames.last().map_or(0, |f| f[0]),
        curves,
        path,
        path_total,
        tail_seconds: style.streak_seconds,
        feather: style.feather,
        mix: style.mix,
        mode,
        projection,
    }
}

/// The per-frame table the kernel searches: `[birth offset, bits of
/// (t − that frame's start)]`, oldest first, with a closing offset that is the
/// candidate count.
fn particulate_frames(sched: &lumit_core::fx::points::PointsSchedule) -> Vec<[u32; 2]> {
    let s = &sched.schedule;
    let (dt, first) = (s.dt(), s.first_frame());
    let mut out: Vec<[u32; 2]> = Vec::with_capacity(s.counts().len() + 1);
    let mut offset = 0u32;
    for (i, n) in s.counts().iter().enumerate() {
        let rel = (sched.t - (first + i as i64) as f64 * dt) as f32;
        out.push([offset, rel.to_bits()]);
        offset = offset.saturating_add(*n);
    }
    out.push([offset, 0]);
    out
}

/// Particulate (docs/08 §3.86, K-446, K-474, K-475): evaluate, compact, draw.
///
/// **The Sprite fallback is decided here** rather than in the kernel: an unset
/// Sprite layer means the mode arrives as Disc, because a render mode must
/// always draw something (particulate.md §2) and one branch resolved host-side
/// cannot come to mean two things in two places.
///
/// **The degradation rung is [`lumit_gpu::fx::ParticulateOp::cap`]** (K-475):
/// halve it and the newest half is what draws, by the same rule the cap itself
/// applies. This pass always hands over the *declared* cap, which is why the
/// rung is never on the export path — there is nothing on this path that could
/// turn it on.
struct Particulate;
impl GpuEffect for Particulate {
    fn match_name(&self) -> &'static str {
        "particulate"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use lumit_core::fx::effects::particulate::Particulate as P;
        use lumit_core::fx::points::RenderMode;
        let inst = P::read(p);
        let packed = inst.points();
        let style = inst.draw_style();
        let sprite = aux.layer_input();
        let mode = match style.mode {
            RenderMode::Disc => 0,
            // Unset draws discs — the documented deviation from
            // unset-is-identity, because a mode must draw something.
            RenderMode::Sprite if sprite.is_some() => 1,
            RenderMode::Sprite => 0,
            RenderMode::Streak => 2,
        };
        let default_schedule = lumit_core::fx::points::PointsSchedule::default();
        let sched = aux.schedule().unwrap_or(&default_schedule);
        let frames = particulate_frames(sched);
        let mut curves = Vec::with_capacity(packed.particle.size_curve.len() * 2);
        curves.extend_from_slice(&packed.particle.size_curve);
        curves.extend_from_slice(&packed.particle.opacity_curve);
        // The camera, in the raster this frame is drawn at (K-561): the
        // projection was built in px@comp, like the mask path beside it, and
        // takes the same factor for the same reason (K-266, K-385).
        let projection = sched
            .projection
            .map(|proj| proj.rescaled(P::px_scale_of(p)).m);
        // The polyline the emitter walks. A Mask path's arrives in px@comp and
        // takes the raster factor like every other distance (K-385); an
        // outline's is built **from the packed emitter**, whose extents the
        // resolve step already rescaled, so scaling it again would draw the
        // ring at twice the size on a half-resolution preview (K-597).
        let poly = if packed.emitter.shape.is_outline() {
            lumit_core::fx::points::outline_polyline(&packed.emitter)
        } else {
            lumit_core::fx::points::scale_path(&path_of(aux), P::px_scale_of(p))
        };
        let path: Vec<[f32; 4]> = poly
            .points
            .iter()
            .zip(poly.arc.iter())
            .map(|(pt, a)| [pt[0], pt[1], *a, 0.0])
            .collect();
        let op = particulate_op(
            &packed,
            style,
            sched,
            &frames,
            &curves,
            &path,
            poly.length(),
            mode,
            projection,
        );
        fx.particulate(ctx, tex, w, h, sprite.filter(|_| mode == 1), &op)
    }
}

/// Every point of a host-built stream, as the generic draw reads them
/// (K-598): one conversion, called by Grid and by Scatter, so a generator
/// cannot come to describe its own points differently from the CPU reference
/// that made them.
fn draw_points_of(s: &lumit_core::fx::points::PointsStream) -> Vec<lumit_gpu::fx::DrawPoint> {
    draw_points_tailed(s, &[])
}

/// [`draw_points_of`], with each point's tail beside it (K-601) — where its
/// capsule runs back to, in the same three axes and the same order. A shorter
/// list than the stream, or an empty one, leaves the tail at the head, which is
/// the plain dot.
fn draw_points_tailed(
    s: &lumit_core::fx::points::PointsStream,
    tails: &[[f32; 3]],
) -> Vec<lumit_gpu::fx::DrawPoint> {
    (0..s.len())
        .map(|i| {
            let position = s.position.get(i).copied().unwrap_or([0.0; 3]);
            lumit_gpu::fx::DrawPoint {
                position,
                size: s.size.get(i).copied().unwrap_or(0.0),
                rotation: s.rotation.get(i).copied().unwrap_or(0.0),
                colour: s.colour.get(i).copied().unwrap_or([0.0; 4]),
                // The stream's `id` is the generator's own index, and it fits a
                // word: a lattice and a candidate set are both bounded by the
                // cap.
                id: u32::try_from(s.id.get(i).copied().unwrap_or(0)).unwrap_or(u32::MAX),
                tail: tails.get(i).copied().unwrap_or(position),
            }
        })
        .collect()
}

/// The stream a points **consumer** is drawing this frame (K-600): the wire's
/// own, sample `k` back, rearranged into the raster the frame is being drawn at.
///
/// Empty for a consumer with nothing wired — which every consumer renders as a
/// passthrough, the documented calm (K-509).
fn points_input_at(
    aux: AuxSlot<'_>,
    k: usize,
    px_scale: f32,
) -> Option<lumit_core::fx::points::PointsStream> {
    aux.schedule()
        .and_then(|c| c.input.get(k))
        .map(|s| s.rescaled(px_scale))
}

/// Grid (docs/08 §3.88, K-598): a lattice worked out on the host and drawn
/// through Particulate's own instanced quad.
///
/// Nothing here decides anything — `Grid::stream` and `Grid::draw_style` did
/// the reducing, in the one expression the CPU reference also reads. The
/// camera arrives on the carriage the birth schedule rides (points-stream.md
/// §3.3): a generator declares a Points output, so the draw builder fills a
/// slot for it, and the projection is what this one reads out of it.
struct Grid;
impl GpuEffect for Grid {
    fn match_name(&self) -> &'static str {
        "grid"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use lumit_core::fx::effects::grid::Grid as G;
        let inst = G::read(p);
        let style = inst.draw_style();
        // The camera, in the raster this frame is drawn at (K-561, K-266).
        let projection = aux
            .schedule()
            .and_then(|s| s.projection)
            .map(|proj| proj.rescaled(G::px_scale_of(p)));
        let stream = inst.stream(projection.unwrap_or_default());
        let points = draw_points_of(&stream);
        fx.points_draw(
            ctx,
            tex,
            w,
            h,
            None,
            &lumit_gpu::fx::PointsDrawOp {
                points: &points,
                feather: style.feather,
                mix: style.mix,
                projection: projection.map(|proj| proj.m),
                alpha_test: false,
                alpha_invert: false,
                seed: inst.seed,
                mode: 0,
                sprite: None,
            },
        )
    }
}

/// Scatter (docs/08 §3.89, K-599): candidates thrown on the host, kept on the
/// card where the alpha under them beats their own die.
///
/// **The rejection is the draw's** rather than a pass of its own: the field is
/// a picture that exists only on the card, and the candidates are a set that
/// exists only on the host, so the vertex stage is where the two meet. What is
/// posted is therefore the whole candidate set, and a refused point is given no
/// size — a disc of no radius covers no pixel.
struct Scatter;
impl GpuEffect for Scatter {
    fn match_name(&self) -> &'static str {
        "scatter"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use lumit_core::fx::effects::scatter::Scatter as S;
        let inst = S::read(p);
        let style = inst.draw_style();
        let px_scale = S::px_scale_of(p);
        let projection = aux
            .schedule()
            .and_then(|sched| sched.projection)
            .map(|proj| proj.rescaled(px_scale));
        // The K-395 override: the matte is *where the points go*, so it arrives
        // as the kernel's own input rather than as a dissolve afterwards. Unset
        // reads this effect's own picture, which is the documented default.
        let matte = aux.matte().cloned();
        let invert = p.bool(lumit_core::fx::MATTE_INVERT_ID, false);
        let candidates = inst.candidates(w, h, px_scale, projection.unwrap_or_default());
        let points = draw_points_of(&candidates);
        fx.points_draw(
            ctx,
            tex,
            w,
            h,
            matte.as_ref(),
            &lumit_gpu::fx::PointsDrawOp {
                points: &points,
                feather: style.feather,
                mix: style.mix,
                projection: projection.map(|proj| proj.m),
                alpha_test: true,
                alpha_invert: invert,
                seed: inst.seed,
                mode: 0,
                sprite: None,
            },
        )
    }
}

/// Clone to points (docs/08 §3.90, K-600): a layer's picture stamped at every
/// point of a wired stream.
///
/// Nothing here decides anything — `CloneToPoints::stamps` did the reducing, in
/// the one expression the CPU reference also reads. What this settles is the two
/// things the bag cannot carry: the stream, which arrives on the carriage in
/// px@comp and is rearranged into this raster, and the camera beside it.
///
/// **An unset Layer row renders the input unchanged**, decided here rather than
/// in the kernel for Particulate's reason inverted: a source, unlike a mode, is
/// allowed to be absent, and one branch resolved host-side cannot come to mean
/// two things in two places.
struct CloneToPoints;
impl GpuEffect for CloneToPoints {
    fn match_name(&self) -> &'static str {
        "clone_to_points"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use lumit_core::fx::effects::clone_to_points::CloneToPoints as C;
        let inst = C::read(p);
        let px_scale = C::px_scale_of(p);
        let style = inst.draw_style();
        // Nothing to stamp, or nothing to stamp it at: the picture passes
        // through, which is what an unset row and an unwired socket both mean.
        let (Some(sprite), Some(stream)) = (aux.layer_input(), points_input_at(aux, 0, px_scale))
        else {
            return tex.clone();
        };
        let stamps = inst.stamps(&stream);
        let points = draw_points_of(&stamps);
        // The camera, in the raster this frame is drawn at (K-561, K-266) —
        // `None` on a 2D layer, where it is not the identity matrix but no
        // matrix at all, so the positions' bits are left alone (K-258).
        let projection = aux
            .schedule()
            .and_then(|c| c.projection)
            .map(|proj| proj.rescaled(px_scale));
        fx.points_draw(
            ctx,
            tex,
            w,
            h,
            None,
            &lumit_gpu::fx::PointsDrawOp {
                points: &points,
                feather: style.feather,
                mix: style.mix,
                projection: projection.map(|proj| proj.m),
                alpha_test: false,
                alpha_invert: false,
                seed: 0,
                mode: 1,
                sprite: Some(sprite),
            },
        )
    }
}

/// Trail (docs/08 §3.91, K-601): where every point of a wired stream has been,
/// drawn from the producer evaluated again rather than from a history.
///
/// Nothing here decides anything — `Trail::tail` did the reducing, in the one
/// expression the CPU reference also reads. The samples arrive on the carriage,
/// newest first, each in px@comp and each rearranged into this raster; what this
/// settles is only that rearrangement and the camera.
struct Trail;
impl GpuEffect for Trail {
    fn match_name(&self) -> &'static str {
        "trail"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use lumit_core::fx::effects::trail::Trail as T;
        let inst = T::read(p);
        let px_scale = T::px_scale_of(p);
        let style = inst.draw_style();
        let samples: Vec<lumit_core::fx::points::PointsStream> = aux
            .schedule()
            .map(|c| c.input.iter().map(|s| s.rescaled(px_scale)).collect())
            .unwrap_or_default();
        // Nothing wired: the picture passes through, the documented calm.
        if samples.is_empty() {
            return tex.clone();
        }
        let (stream, tails) = inst.tail(&samples);
        let points = draw_points_tailed(&stream, &tails);
        // The camera, in the raster this frame is drawn at (K-561, K-266) —
        // `None` on a 2D layer, where it is no matrix at all (K-258).
        let projection = aux
            .schedule()
            .and_then(|c| c.projection)
            .map(|proj| proj.rescaled(px_scale));
        fx.points_draw(
            ctx,
            tex,
            w,
            h,
            None,
            &lumit_gpu::fx::PointsDrawOp {
                points: &points,
                feather: style.feather,
                mix: style.mix,
                projection: projection.map(|proj| proj.m),
                alpha_test: false,
                alpha_invert: false,
                seed: 0,
                mode: 0,
                sprite: None,
            },
        )
    }
}

/// Connect points (docs/08 §3.92, K-602): the lines between the points of a
/// wired stream that are near enough to each other.
///
/// Nothing here decides anything — `ConnectPoints::links` did the pairing and
/// the reducing, in the one expression the CPU reference also reads. What this
/// settles is the rearrangement into this raster and the camera, exactly as its
/// two sibling consumers do.
struct ConnectPoints;
impl GpuEffect for ConnectPoints {
    fn match_name(&self) -> &'static str {
        "connect_points"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        use lumit_core::fx::effects::connect_points::ConnectPoints as C;
        let inst = C::read(p);
        let px_scale = C::px_scale_of(p);
        let style = inst.draw_style();
        // Nothing wired: the picture passes through, the documented calm.
        let Some(stream) = points_input_at(aux, 0, px_scale) else {
            return tex.clone();
        };
        let (segments, tails) = inst.links(&stream);
        let points = draw_points_tailed(&segments, &tails);
        // The camera, in the raster this frame is drawn at (K-561, K-266) —
        // `None` on a 2D layer, where it is no matrix at all (K-258).
        let projection = aux
            .schedule()
            .and_then(|c| c.projection)
            .map(|proj| proj.rescaled(px_scale));
        fx.points_draw(
            ctx,
            tex,
            w,
            h,
            None,
            &lumit_gpu::fx::PointsDrawOp {
                points: &points,
                feather: style.feather,
                mix: style.mix,
                projection: projection.map(|proj| proj.m),
                alpha_test: false,
                alpha_invert: false,
                seed: 0,
                mode: 0,
                sprite: None,
            },
        )
    }
}

struct AddGrain;
impl GpuEffect for AddGrain {
    fn match_name(&self) -> &'static str {
        "add_grain"
    }
    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &Tex,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> Tex {
        let g =
            effects::add_grain::AddGrain::read(p).packed(effects::add_grain::AddGrain::tick_of(p));
        fx.add_grain(
            ctx,
            tex,
            w,
            h,
            aux.matte(),
            &lumit_gpu::fx::AddGrainOp {
                amplitude: g.amplitude,
                inv_size: g.inv_size,
                softness: g.softness,
                tonal: g.tonal,
                monochrome: g.monochrome,
                seed: g.seed,
                tick: g.tick,
                mix: g.mix,
            },
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::fx::BUILTIN_DEFS;

    // ------------------------------------------------------ Particulate (PS2)

    /// One Particulate fixture: a busy, non-degenerate parameter set with every
    /// force on, so a kernel that quietly skipped the drag terms or the
    /// turbulence could not pass.
    ///
    /// The numbers go through the effect's own `points`/`draw_style`, which is
    /// the whole point of the §1.6 arrangement: the CPU reference and the
    /// kernel read one expression's output, not two.
    fn particulate_fixture(
        mode: u32,
        cap: i32,
    ) -> (
        lumit_core::fx::effects::particulate::Particulate,
        lumit_core::fx::points::Schedule,
        lumit_core::fx::points::PointsSchedule,
    ) {
        particulate_fixture_3d(mode, cap, None)
    }

    /// The same fixture, with the third axis alive (K-561): every new row set
    /// to something that matters, and the camera the stream is seen through.
    /// `None` is the flat fixture above, which is the one every pre-K-561 test
    /// reads and the one whose picture must not have moved.
    fn particulate_fixture_3d(
        mode: u32,
        cap: i32,
        projection: Option<lumit_core::fx::points::Projection>,
    ) -> (
        lumit_core::fx::effects::particulate::Particulate,
        lumit_core::fx::points::Schedule,
        lumit_core::fx::points::PointsSchedule,
    ) {
        use lumit_core::fx::effects::particulate::Particulate as P;
        let deep = projection.is_some();
        let inst = P {
            shape: 2,
            position_x: 64.0,
            position_y: 48.0,
            position_z: if deep { -30.0 } else { 0.0 },
            depth: if deep { 90.0 } else { 0.0 },
            direction_z: if deep { 25.0 } else { 0.0 },
            spread_z: if deep { 70.0 } else { 0.0 },
            wind_z: if deep { 35.0 } else { 0.0 },
            width: 60.0,
            height: 40.0,
            emitter_angle: 20.0,
            mask_path: false,
            emit_rate: 220.0,
            direction: -90.0,
            spread: 200.0,
            initial_speed: 70.0,
            speed_jitter: 40.0,
            life: 1.5,
            life_jitter: 35.0,
            size: 9.0,
            size_jitter: 45.0,
            size_over_life: lumit_core::fx::CurvePoints::default(),
            opacity_over_life: lumit_core::fx::CurvePoints::default(),
            colour: [0.9, 0.6, 0.3, 1.0],
            end_colour: [0.2, 0.4, 1.0, 1.0],
            rotation: 15.0,
            rotation_jitter: 240.0,
            spin: 90.0,
            align_to_motion: false,
            gravity: 120.0,
            wind_x: 40.0,
            wind_y: -10.0,
            drag: 0.8,
            turbulence_amount: 18.0,
            turbulence_scale: 70.0,
            turbulence_speed: 0.4,
            mode,
            feather: 60.0,
            sprite_layer: false,
            streak_length: 0.05,
            max_particles: cap,
            seed: 7,
            mix: 100.0,
        };
        let dt = 1.0 / 24.0;
        let t = 24.0 * dt;
        let rate = f64::from(inst.emit_rate);
        let sched = lumit_core::fx::points::Schedule::scan(
            dt,
            (t / dt).floor() as i64,
            inst.window_frames(dt),
            &|_| rate,
        );
        let carriage = lumit_core::fx::points::PointsSchedule {
            schedule: sched.clone(),
            t,
            projection,
            ..Default::default()
        };
        (inst, sched, carriage)
    }

    /// The GPU op for a fixture, through the very function the shipping pass
    /// uses — so a drift between the two conversions is impossible rather than
    /// merely unlikely.
    #[allow(clippy::type_complexity)]
    fn particulate_pieces(
        inst: lumit_core::fx::effects::particulate::Particulate,
        carriage: &lumit_core::fx::points::PointsSchedule,
    ) -> (
        lumit_core::fx::points::PointsParams,
        lumit_core::fx::points::DrawStyle,
        Vec<[u32; 2]>,
        Vec<f32>,
    ) {
        let packed = inst
            .points()
            .projected(carriage.projection.unwrap_or_default());
        let style = inst.draw_style();
        let frames = particulate_frames(carriage);
        let mut curves = Vec::new();
        curves.extend_from_slice(&packed.particle.size_curve);
        curves.extend_from_slice(&packed.particle.opacity_curve);
        (packed, style, frames, curves)
    }

    /// **The centre of PS2** (points-stream.md §3.2, particulate.md §9 item 8):
    /// the compacted GPU stream and the CPU reference describe the same
    /// particles, attribute by attribute, to **one part in 10⁵ of each
    /// attribute's own range** (K-508).
    ///
    /// Pixels get a perceptual tolerance; these numbers get a numerical one,
    /// because a consumer reads them as *data*. The measure is relative to the
    /// attribute's range and not to each value, because half of these
    /// quantities pass through zero — a speed reversing has no meaningful ULP
    /// count, and asking for one is asking a metric a question it cannot
    /// answer. The colour region is compared nowhere here: particulate.md §4
    /// declares it at half precision, which is a storage width rather than an
    /// agreement, and the picture test below is what holds it.
    /// A head-on camera over the fixture's little raster, as the restriction
    /// comes out: a particle `z` deep scales by `zoom/(zoom + z)` about the
    /// frame's centre. The renderer's own construction is held to the
    /// compositor's matrices in `build.rs`; what this one is for is giving the
    /// kernel a projection that is definitely not the identity.
    fn test_camera(centre: [f32; 2], zoom: f32) -> lumit_core::fx::points::Projection {
        lumit_core::fx::points::Projection {
            m: [
                [1.0, 0.0, centre[0] / zoom, 0.0],
                [0.0, 1.0, centre[1] / zoom, 0.0],
                [0.0, 0.0, 1.0 / zoom, 1.0],
            ],
        }
    }

    #[test]
    fn the_particulate_stream_agrees_with_the_cpu_reference() {
        stream_agrees_with_the_cpu_reference(None);
    }

    /// **Border emission, in both paths** (K-597): the kernel walks the very
    /// polyline the CPU reference walks, because the host flattens it once and
    /// hands the same numbers to both. A twin test rather than a second
    /// picture: an outline the two paths disagreed about would be a ring drawn
    /// in one place and sampled in another.
    #[test]
    fn the_outline_emitters_agree_between_the_two_paths() {
        stream_agrees_with_the_cpu_reference_of(5, None);
        stream_agrees_with_the_cpu_reference_of(6, None);
    }

    /// **The same agreement, in three axes** (K-561): the four passes carry the
    /// third component, and the depth attributes are held to the same one part
    /// in 10⁵ of their range as the two that were always there.
    ///
    /// The compacted stream is *unprojected* on both sides — it is the layer's
    /// three axes, which is what a 3D-aware consumer reads — so this is the
    /// closed forms agreeing about depth, and the draw test below is what says
    /// the camera agrees about where that depth is seen.
    #[test]
    fn the_particulate_stream_agrees_with_the_cpu_reference_in_three_dimensions() {
        stream_agrees_with_the_cpu_reference(Some(test_camera([64.0, 48.0], 300.0)));
    }

    fn stream_agrees_with_the_cpu_reference(
        projection: Option<lumit_core::fx::points::Projection>,
    ) {
        stream_agrees_with_the_cpu_reference_of(2, projection);
    }

    fn stream_agrees_with_the_cpu_reference_of(
        shape: u32,
        projection: Option<lumit_core::fx::points::Projection>,
    ) {
        let Ok(ctx) = GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (128u32, 96u32);
        let (mut inst, sched, carriage) = particulate_fixture_3d(0, 20_000, projection);
        inst.shape = shape;
        if lumit_core::fx::points::EmitterShape::from_code(shape).is_outline() {
            // **Turbulence off for the outline twin, on purpose.** What this
            // run measures is that the two paths walk the polyline to the same
            // point, and turbulence is a magnifier sitting on top of it: the
            // phase runs to a thousand, where an f32 step is 6·10⁻⁵ of a
            // lattice cell, and the noise turns one such step into a couple of
            // thousandths of a pixel. That is the shipped fixture's own
            // tolerance (the test above measures it at a part in 10⁶ and passes
            // on it), and letting it ride here would answer a question about
            // the noise rather than the one this test asks.
            inst.turbulence_amount = 0.0;
        }
        let (packed, style, frames, curves) = particulate_pieces(inst, &carriage);
        // The outline the emitter walks, flattened by the one function both
        // paths call — the shipping pass does exactly this (K-597).
        let outline = lumit_core::fx::points::outline_polyline(&packed.emitter);
        let path: Vec<[f32; 4]> = outline
            .points
            .iter()
            .zip(outline.arc.iter())
            .map(|(pt, a)| [pt[0], pt[1], *a, 0.0])
            .collect();
        let cpu = lumit_core::fx::points::evaluate(
            &packed,
            &sched,
            carriage.t,
            &lumit_core::mask::MaskPolyline::default(),
        );
        assert!(
            cpu.len() > 50,
            "the fixture made only {} particles",
            cpu.len()
        );

        let op = particulate_op(
            &packed,
            style,
            &carriage,
            &frames,
            &curves,
            &path,
            outline.length(),
            0,
            carriage.projection.map(|proj| proj.m),
        );
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &vec![0.0; (w * h * 4) as usize], w, h);
        let (count, words) = fx
            .particulate_stream(&ctx, &tex, w, h, &op)
            .expect("the stream reads back");
        assert_eq!(count as usize, cpu.len(), "live counts differ");

        let cap = op.cap as usize;
        let f = |i: usize| f32::from_bits(words[i]);
        // `id` is not a measurement — it is the birth index, and it agrees
        // exactly or the compaction has put a particle in the wrong slot.
        for i in 0..cpu.len() {
            let id =
                u64::from(words[12 * cap + i * 2]) | (u64::from(words[12 * cap + i * 2 + 1]) << 32);
            assert_eq!(
                id, cpu.id[i],
                "id {i} — the compaction is not in birth order"
            );
        }
        let names = [
            "position x",
            "position y",
            "position z",
            "speed x",
            "speed y",
            "speed z",
            "age",
            "life",
            "size",
            "rotation",
        ];
        let mut worst_gap = [0.0f32; 10];
        let mut range = [0.0f32; 10];
        for i in 0..cpu.len() {
            for (k, (a, b)) in [
                (f(i * 3), cpu.position[i][0]),
                (f(i * 3 + 1), cpu.position[i][1]),
                (f(i * 3 + 2), cpu.position[i][2]),
                (f(3 * cap + i * 3), cpu.speed[i][0]),
                (f(3 * cap + i * 3 + 1), cpu.speed[i][1]),
                (f(3 * cap + i * 3 + 2), cpu.speed[i][2]),
                (f(6 * cap + i), cpu.age[i]),
                (f(7 * cap + i), cpu.life[i]),
                (f(8 * cap + i), cpu.size[i]),
                (f(9 * cap + i), cpu.rotation[i]),
            ]
            .into_iter()
            .enumerate()
            {
                worst_gap[k] = worst_gap[k].max((a - b).abs());
                range[k] = range[k].max(b.abs());
            }
        }
        let mut worst = 0.0f32;
        for k in 0..8 {
            let rel = worst_gap[k] / range[k].max(1e-3);
            eprintln!(
                "particulate stream: {} worst |Δ| {:.2e} over a range of {:.2e} — {rel:.2e}",
                names[k], worst_gap[k], range[k]
            );
            worst = worst.max(rel);
        }
        assert!(
            worst < 1e-5,
            "worst relative gap {worst:.2e} — the closed forms have parted"
        );
    }

    /// The picture, in all three render modes (particulate.md §9 item 8): the
    /// instanced quads land where the software dabs land, inside the
    /// `moderate` class's perceptual epsilon.
    #[test]
    fn the_particulate_draw_matches_the_cpu_reference_in_every_mode() {
        draw_matches_the_cpu_reference(None);
    }

    /// **The camera draws the same picture on both paths** (K-561): the CPU
    /// reference projects each particle before it stamps it, the vertex stage
    /// projects each instance before it places its quad, and the two agree
    /// inside the `moderate` class's epsilon in every render mode — streaks
    /// included, whose tails take the same camera as their heads.
    #[test]
    fn the_particulate_draw_matches_the_cpu_reference_through_a_camera() {
        draw_matches_the_cpu_reference(Some(test_camera([64.0, 48.0], 300.0)));
    }

    fn draw_matches_the_cpu_reference(projection: Option<lumit_core::fx::points::Projection>) {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (128u32, 96u32);
        // A flat sprite, so the comparison is of *placement* rather than of two
        // bilinear filters: a solid square samples identically either side.
        let sprite_px: Vec<f32> = vec![0.5; (8 * 8 * 4) as usize];
        let sprite_tex = lumit_gpu::fx::upload_linear_f32(&ctx, &sprite_px, 8, 8);
        for mode in [0u32, 1, 2] {
            let (inst, sched, carriage) = particulate_fixture_3d(mode, 4_000, projection);
            let (packed, style, frames, curves) = particulate_pieces(inst, &carriage);
            let (cpu_stream, tails) = lumit_core::fx::points::evaluate_with_tail(
                &packed,
                &sched,
                carriage.t,
                &lumit_core::mask::MaskPolyline::default(),
                style.streak_seconds,
            );
            let mut cpu = vec![0.0f32; (w * h * 4) as usize];
            lumit_core::fx::points::draw_stream(
                &mut cpu,
                w,
                h,
                &cpu_stream,
                &tails,
                &style,
                (mode == 1).then_some(lumit_core::fx::points::Sprite {
                    rgba: &sprite_px,
                    w: 8,
                    h: 8,
                }),
            );

            // A camera that changed nothing would make every comparison below
            // pass by comparing two identical flat pictures.
            if projection.is_some() {
                let mut flat_stream = cpu_stream.clone();
                flat_stream.projection = lumit_core::fx::points::Projection::FLAT;
                let mut flat = vec![0.0f32; (w * h * 4) as usize];
                lumit_core::fx::points::draw_stream(
                    &mut flat,
                    w,
                    h,
                    &flat_stream,
                    &tails,
                    &style,
                    (mode == 1).then_some(lumit_core::fx::points::Sprite {
                        rgba: &sprite_px,
                        w: 8,
                        h: 8,
                    }),
                );
                assert_ne!(cpu, flat, "mode {mode}: the camera moved nothing at all");
            }

            let op = particulate_op(
                &packed,
                style,
                &carriage,
                &frames,
                &curves,
                &[],
                0.0,
                mode,
                carriage.projection.map(|proj| proj.m),
            );
            let tex =
                lumit_gpu::fx::upload_linear_f32(&ctx, &vec![0.0; (w * h * 4) as usize], w, h);
            let out = fx.particulate(&ctx, &tex, w, h, (mode == 1).then_some(&sprite_tex), &op);
            let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

            let drawn: f32 = gpu.iter().sum();
            assert!(drawn > 1.0, "mode {mode} drew nothing ({drawn})");
            let worst = cpu
                .iter()
                .zip(&gpu)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            eprintln!("particulate mode {mode}: worst |Δ| {worst:.3e}");
            assert!(worst < 2e-2, "mode {mode}: worst |Δ| {worst}");

            // Bit-stable against itself (docs/08 §2.4): two evaluations of one
            // frame are one picture.
            let again = fx.particulate(&ctx, &tex, w, h, (mode == 1).then_some(&sprite_tex), &op);
            let twice = lumit_gpu::fx::readback_linear_f32(&ctx, &again, w, h).expect("readback");
            assert_eq!(gpu, twice, "mode {mode} is not bit-stable");
        }
    }

    // ------------------------------------------------- Grid (K-598)

    /// A Grid over the little raster, off-centre and jittered, so nothing in
    /// the comparison below can pass by symmetry.
    fn grid_fixture(planes: i32) -> lumit_core::fx::effects::grid::Grid {
        lumit_core::fx::effects::grid::Grid {
            columns: 7,
            rows: 5,
            planes,
            spacing_x: 15.0,
            spacing_y: 17.0,
            spacing_z: 40.0,
            position_x: 60.0,
            position_y: 44.0,
            position_z: if planes > 1 { -20.0 } else { 0.0 },
            jitter_x: 6.0,
            jitter_y: 6.0,
            jitter_z: 0.0,
            seed: 3,
            size: 7.0,
            feather: 55.0,
            colour: [0.8, 0.4, 0.9, 1.0],
            max_points: 20_000,
            mix: 100.0,
        }
    }

    /// **The generic draw draws the CPU reference's lattice** (K-598): the
    /// instanced quads land where the software dabs land, inside the
    /// `moderate` class's perceptual epsilon — and they are drawing the *same*
    /// stream, uploaded, so a disagreement here is the rasteriser's and
    /// nothing else's.
    #[test]
    fn a_grids_draw_matches_the_cpu_reference() {
        grid_draw_matches(None);
    }

    /// The same, through a camera (K-561): a lattice of planes seen in
    /// perspective, projected in the vertex stage on one path and before the
    /// dab on the other.
    #[test]
    fn a_grids_draw_matches_the_cpu_reference_through_a_camera() {
        grid_draw_matches(Some(test_camera([64.0, 48.0], 300.0)));
    }

    fn grid_draw_matches(projection: Option<lumit_core::fx::points::Projection>) {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (128u32, 96u32);
        let inst = grid_fixture(if projection.is_some() { 3 } else { 1 });
        let style = inst.draw_style();
        let stream = inst.stream(projection.unwrap_or_default());
        assert!(
            stream.len() > 20,
            "the fixture made {} points",
            stream.len()
        );

        let mut cpu = vec![0.0f32; (w * h * 4) as usize];
        lumit_core::fx::points::draw_stream(&mut cpu, w, h, &stream, &[], &style, None);
        // A camera that changed nothing would make the comparison pass by
        // comparing two identical flat pictures.
        if projection.is_some() {
            let mut flat_stream = stream.clone();
            flat_stream.projection = lumit_core::fx::points::Projection::FLAT;
            let mut flat = vec![0.0f32; (w * h * 4) as usize];
            lumit_core::fx::points::draw_stream(&mut flat, w, h, &flat_stream, &[], &style, None);
            assert_ne!(cpu, flat, "the camera moved nothing at all");
        }

        let points = draw_points_of(&stream);
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &vec![0.0; (w * h * 4) as usize], w, h);
        let op = lumit_gpu::fx::PointsDrawOp {
            points: &points,
            feather: style.feather,
            mix: style.mix,
            projection: projection.map(|proj| proj.m),
            alpha_test: false,
            alpha_invert: false,
            seed: inst.seed,
            mode: 0,
            sprite: None,
        };
        let out = fx.points_draw(&ctx, &tex, w, h, None, &op);
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        let drawn: f32 = gpu.iter().sum();
        assert!(drawn > 1.0, "the lattice drew nothing ({drawn})");
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        eprintln!("grid draw: worst |Δ| {worst:.3e}");
        assert!(worst < 2e-2, "worst |Δ| {worst}");

        // Bit-stable against itself (docs/08 §2.4).
        let again = fx.points_draw(&ctx, &tex, w, h, None, &op);
        let twice = lumit_gpu::fx::readback_linear_f32(&ctx, &again, w, h).expect("readback");
        assert_eq!(gpu, twice, "the generic draw is not bit-stable");
    }

    /// **Mix at nought is the emit-only mode** (K-598): the stream is still
    /// made, and the picture is the input untouched.
    #[test]
    fn a_grid_at_no_mix_leaves_the_picture_alone() {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (64u32, 48u32);
        let mut inst = grid_fixture(1);
        inst.mix = 0.0;
        let stream = inst.stream(Default::default());
        assert!(!stream.is_empty(), "the stream is emitted regardless");
        let input: Vec<f32> = (0..(w * h * 4)).map(|i| (i % 7) as f32 * 0.1).collect();
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &input, w, h);
        let points = draw_points_of(&stream);
        let out = fx.points_draw(
            &ctx,
            &tex,
            w,
            h,
            None,
            &lumit_gpu::fx::PointsDrawOp {
                points: &points,
                feather: inst.draw_style().feather,
                mix: 0.0,
                projection: None,
                alpha_test: false,
                alpha_invert: false,
                seed: inst.seed,
                mode: 0,
                sprite: None,
            },
        );
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");
        // Against the *texture*, not the array it was uploaded from: the
        // working format is fp16, so the picture has already been rounded once
        // on its way onto the card and this is the pass-through it must equal.
        let before = lumit_gpu::fx::readback_linear_f32(&ctx, &tex, w, h).expect("readback");
        assert_eq!(gpu, before, "Mix at nought drew something");
    }

    // ---------------------------------------------- Scatter (K-599)

    /// **The rejection agrees, path for path** (K-599): the vertex stage keeps
    /// the candidates the CPU reference keeps, and refuses the ones it refuses.
    ///
    /// The field is a hard-edged half-opaque picture on purpose. A soft edge
    /// would put candidates within a rounding of their own die, and what this
    /// test is about is the *rule* — the alpha under the point against the
    /// candidate's own hash — not the last bit of an fp16 texture.
    #[test]
    fn a_scatters_rejection_agrees_with_the_cpu_reference() {
        for invert in [false, true] {
            let Ok(ctx) = GpuContext::headless() else {
                return;
            };
            let fx = FxEngine::new(&ctx);
            let (w, h) = (128u32, 96u32);
            let inst = lumit_core::fx::effects::scatter::Scatter {
                density: 40.0,
                seed: 11,
                size: 5.0,
                feather: 60.0,
                colour: [0.9, 0.5, 0.2, 1.0],
                max_points: 20_000,
                mix: 100.0,
            };
            // Left half opaque, right half clear — the same field both paths
            // read, uploaded once.
            let mut field = vec![0.0f32; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let d = ((y * w + x) * 4) as usize;
                    field[d + 3] = f32::from(x < w / 2);
                }
            }
            let style = inst.draw_style();
            let kept = inst.stream(w, h, 1.0, &field, invert, Default::default());
            assert!(kept.len() > 10, "the fixture kept {}", kept.len());
            let all = inst.candidates(w, h, 1.0, Default::default());
            assert!(kept.len() < all.len(), "nothing was refused");

            let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &field, w, h);
            // The reference draws over the field as the card holds it — the
            // working format is fp16, so this is the picture the kernel is
            // compositing onto and the comparison is of the discs alone.
            let mut cpu = lumit_gpu::fx::readback_linear_f32(&ctx, &tex, w, h).expect("readback");
            lumit_core::fx::points::draw_stream(&mut cpu, w, h, &kept, &[], &style, None);

            let points = draw_points_of(&all);
            let out = fx.points_draw(
                &ctx,
                &tex,
                w,
                h,
                None,
                &lumit_gpu::fx::PointsDrawOp {
                    points: &points,
                    feather: style.feather,
                    mix: style.mix,
                    projection: None,
                    alpha_test: true,
                    alpha_invert: invert,
                    seed: inst.seed,
                    mode: 0,
                    sprite: None,
                },
            );
            let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");
            let worst = cpu
                .iter()
                .zip(&gpu)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            eprintln!("scatter draw (invert {invert}): worst |Δ| {worst:.3e}");
            assert!(worst < 2e-2, "invert {invert}: worst |Δ| {worst}");
        }
    }

    // ---------------------------------------- Clone to points (K-600)

    /// **A stamp laid by a consumer is a stamp Particulate lays** (K-600): the
    /// instanced draw and the CPU reference put the same layer in the same
    /// squares, inside the `moderate` class's perceptual epsilon — and they are
    /// handed the *same* stream, so a disagreement here is the rasteriser's and
    /// nothing else's.
    #[test]
    fn a_clone_to_points_draw_matches_the_cpu_reference() {
        clone_draw_matches(None);
    }

    /// The same, through a camera (K-561): the stamps foreshorten with depth,
    /// projected in the vertex stage on one path and before the stamp on the
    /// other.
    #[test]
    fn a_clone_to_points_draw_matches_the_cpu_reference_through_a_camera() {
        clone_draw_matches(Some(test_camera([64.0, 48.0], 300.0)));
    }

    fn clone_draw_matches(projection: Option<lumit_core::fx::points::Projection>) {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (128u32, 96u32);
        // A flat sprite, so what is compared is *placement* rather than two
        // bilinear filters: a solid square samples identically either side.
        let sprite_px: Vec<f32> = vec![0.5; (8 * 8 * 4) as usize];
        let sprite_tex = lumit_gpu::fx::upload_linear_f32(&ctx, &sprite_px, 8, 8);

        // The wire's own stream, made by a producer that is already tested.
        let stream = grid_fixture(if projection.is_some() { 3 } else { 1 })
            .stream(projection.unwrap_or_default());
        let inst = lumit_core::fx::effects::clone_to_points::CloneToPoints {
            clone_layer: true,
            scale: 160.0,
            rotation: 25.0,
            tint: 100.0,
            max_clones: 20_000,
            mix: 100.0,
        };
        let stamps = inst.stamps(&stream);
        assert!(stamps.len() > 20, "the fixture stamped {}", stamps.len());
        let style = inst.draw_style();

        let mut cpu = vec![0.0f32; (w * h * 4) as usize];
        lumit_core::fx::points::draw_stream(
            &mut cpu,
            w,
            h,
            &stamps,
            &[],
            &style,
            Some(lumit_core::fx::points::Sprite {
                rgba: &sprite_px,
                w: 8,
                h: 8,
            }),
        );
        // A camera that changed nothing would make the comparison pass by
        // comparing two identical flat pictures.
        if projection.is_some() {
            let mut flat_stamps = stamps.clone();
            flat_stamps.projection = lumit_core::fx::points::Projection::FLAT;
            let mut flat = vec![0.0f32; (w * h * 4) as usize];
            lumit_core::fx::points::draw_stream(
                &mut flat,
                w,
                h,
                &flat_stamps,
                &[],
                &style,
                Some(lumit_core::fx::points::Sprite {
                    rgba: &sprite_px,
                    w: 8,
                    h: 8,
                }),
            );
            assert_ne!(cpu, flat, "the camera moved nothing at all");
        }

        let points = draw_points_of(&stamps);
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &vec![0.0; (w * h * 4) as usize], w, h);
        let op = lumit_gpu::fx::PointsDrawOp {
            points: &points,
            feather: style.feather,
            mix: style.mix,
            projection: projection.map(|proj| proj.m),
            alpha_test: false,
            alpha_invert: false,
            seed: 0,
            mode: 1,
            sprite: Some(&sprite_tex),
        };
        let out = fx.points_draw(&ctx, &tex, w, h, None, &op);
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        let drawn: f32 = gpu.iter().sum();
        assert!(drawn > 1.0, "the stamps drew nothing ({drawn})");
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        eprintln!("clone to points draw: worst |Δ| {worst:.3e}");
        assert!(worst < 2e-2, "worst |Δ| {worst}");

        // Bit-stable against itself (docs/08 §2.4).
        let again = fx.points_draw(&ctx, &tex, w, h, None, &op);
        let twice = lumit_gpu::fx::readback_linear_f32(&ctx, &again, w, h).expect("readback");
        assert_eq!(gpu, twice, "the stamped draw is not bit-stable");
    }

    // ---------------------------------------------------- Trail (K-601)

    /// **The tail the card draws is the tail the reference drew** (K-601): the
    /// instanced quads land where the software dabs land, inside the `moderate`
    /// class's perceptual epsilon — and both are handed the *same* dabs and the
    /// *same* capsules, so a disagreement here is the rasteriser's alone.
    ///
    /// Segments rather than Dots, because a capsule exercises the geometry a
    /// dab does not: the quad is swept along the segment, and the coverage is a
    /// distance to a line rather than to a point.
    #[test]
    fn a_trails_draw_matches_the_cpu_reference() {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (128u32, 96u32);

        // Four samples of a moving lattice — a Grid does not move, so the
        // producer here is a Particulate fixture read at four moments.
        let (inst, _, carriage) = particulate_fixture(0, 4_000);
        let samples: Vec<lumit_core::fx::points::PointsStream> = (0..4)
            .map(|k| {
                let t = carriage.t - f64::from(k) * 0.05;
                let dt = carriage.schedule.dt();
                let rate = f64::from(inst.emit_rate);
                let sched = lumit_core::fx::points::Schedule::scan(
                    dt,
                    (t / dt).floor() as i64,
                    inst.window_frames(dt),
                    &|_| rate,
                );
                lumit_core::fx::points::evaluate(
                    &inst.points(),
                    &sched,
                    t,
                    &lumit_core::mask::MaskPolyline::default(),
                )
            })
            .collect();
        assert!(!samples[0].is_empty(), "the fixture emitted nothing");

        let trail = lumit_core::fx::effects::trail::Trail {
            back_samples: 4,
            back_step: 0.05,
            style: 1,
            scale: 80.0,
            feather: 60.0,
            fade: 100.0,
            max_trails: 400,
            mix: 100.0,
        };
        let (stream, tails) = trail.tail(&samples);
        assert!(stream.len() > 20, "the fixture drew {} dabs", stream.len());
        let style = trail.draw_style();

        let mut cpu = vec![0.0f32; (w * h * 4) as usize];
        lumit_core::fx::points::draw_stream(&mut cpu, w, h, &stream, &tails, &style, None);

        let points = draw_points_tailed(&stream, &tails);
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &vec![0.0; (w * h * 4) as usize], w, h);
        let op = lumit_gpu::fx::PointsDrawOp {
            points: &points,
            feather: style.feather,
            mix: style.mix,
            projection: None,
            alpha_test: false,
            alpha_invert: false,
            seed: 0,
            mode: 0,
            sprite: None,
        };
        let out = fx.points_draw(&ctx, &tex, w, h, None, &op);
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        let drawn: f32 = gpu.iter().sum();
        assert!(drawn > 1.0, "the tail drew nothing ({drawn})");
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        eprintln!("trail draw: worst |Δ| {worst:.3e}");
        assert!(worst < 2e-2, "worst |Δ| {worst}");

        // A capsule that was really a dab would make the comparison pass by
        // comparing two pictures of dots.
        let dots = draw_points_tailed(&stream, &[]);
        let flat = fx.points_draw(
            &ctx,
            &tex,
            w,
            h,
            None,
            &lumit_gpu::fx::PointsDrawOp {
                points: &dots,
                ..op
            },
        );
        let flat = lumit_gpu::fx::readback_linear_f32(&ctx, &flat, w, h).expect("readback");
        assert_ne!(gpu, flat, "the capsules joined nothing up");

        // Bit-stable against itself (docs/08 §2.4).
        let again = fx.points_draw(&ctx, &tex, w, h, None, &op);
        let twice = lumit_gpu::fx::readback_linear_f32(&ctx, &again, w, h).expect("readback");
        assert_eq!(gpu, twice, "the tail draw is not bit-stable");
    }

    // ------------------------------------------- Connect points (K-602)

    /// **The web the card draws is the web the reference drew** (K-602): the
    /// instanced capsules land where the software dabs land, inside the
    /// `moderate` class's perceptual epsilon — and both are handed the *same*
    /// segments, so a disagreement here is the rasteriser's alone.
    #[test]
    fn a_connect_points_draw_matches_the_cpu_reference() {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (128u32, 96u32);

        let stream = lumit_core::fx::effects::grid::Grid {
            columns: 7,
            rows: 5,
            planes: 1,
            spacing_x: 18.0,
            spacing_y: 18.0,
            spacing_z: 0.0,
            position_x: 64.0,
            position_y: 48.0,
            position_z: 0.0,
            jitter_x: 6.0,
            jitter_y: 6.0,
            jitter_z: 0.0,
            seed: 7,
            size: 4.0,
            feather: 100.0,
            colour: [0.9, 0.7, 0.4, 1.0],
            max_points: 4_000,
            mix: 100.0,
        }
        .stream(lumit_core::fx::points::Projection::FLAT);
        assert!(stream.len() > 20, "the fixture emitted nothing");

        let connect = lumit_core::fx::effects::connect_points::ConnectPoints {
            max_distance: 26.0,
            max_links: 3,
            taper: 40.0,
            fade: 80.0,
            width: 3.0,
            feather: 60.0,
            colour: [1.0, 1.0, 1.0, 1.0],
            max_points: 4_000,
            mix: 100.0,
        };
        let (segments, tails) = connect.links(&stream);
        assert!(segments.len() > 10, "the web has {} lines", segments.len());
        let style = connect.draw_style();

        let mut cpu = vec![0.0f32; (w * h * 4) as usize];
        lumit_core::fx::points::draw_stream(&mut cpu, w, h, &segments, &tails, &style, None);

        let points = draw_points_tailed(&segments, &tails);
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &vec![0.0; (w * h * 4) as usize], w, h);
        let op = lumit_gpu::fx::PointsDrawOp {
            points: &points,
            feather: style.feather,
            mix: style.mix,
            projection: None,
            alpha_test: false,
            alpha_invert: false,
            seed: 0,
            mode: 0,
            sprite: None,
        };
        let out = fx.points_draw(&ctx, &tex, w, h, None, &op);
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        let drawn: f32 = gpu.iter().sum();
        assert!(drawn > 1.0, "the web drew nothing ({drawn})");
        let worst = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        eprintln!("connect points draw: worst |Δ| {worst:.3e}");
        assert!(worst < 2e-2, "worst |Δ| {worst}");

        // A line that was really a dab would make the comparison pass by
        // comparing two pictures of dots.
        let dots = draw_points_tailed(&segments, &[]);
        let flat = fx.points_draw(
            &ctx,
            &tex,
            w,
            h,
            None,
            &lumit_gpu::fx::PointsDrawOp {
                points: &dots,
                ..op
            },
        );
        let flat = lumit_gpu::fx::readback_linear_f32(&ctx, &flat, w, h).expect("readback");
        assert_ne!(gpu, flat, "the lines joined nothing up");

        // Bit-stable against itself (docs/08 §2.4).
        let again = fx.points_draw(&ctx, &tex, w, h, None, &op);
        let twice = lumit_gpu::fx::readback_linear_f32(&ctx, &again, w, h).expect("readback");
        assert_eq!(gpu, twice, "the web draw is not bit-stable");
    }

    // ---------------------------------------------------------------
    // The goldens (PS7; points-stream.md §5, particulate.md §9)
    // ---------------------------------------------------------------

    /// Where the checked-in expectation lives: the crate root, following the
    /// `fx-labels.txt`/`fx-reference.json` convention it is regenerated by.
    fn golden_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("particulate-golden.txt")
    }

    /// The golden raster — small on purpose. Big enough that a particle lands
    /// on a dozen pixels and a one-pixel shift shows, small enough that the
    /// whole expectation is a file a person can diff.
    const GOLDEN_W: u32 = 32;
    const GOLDEN_H: u32 = 24;

    /// The golden fixture: [`particulate_fixture`]'s shape with the numbers
    /// turned down, so the live set is a few dozen particles rather than a few
    /// hundred and all of them are on the raster.
    fn particulate_golden_fixture(
        mode: u32,
    ) -> (
        lumit_core::fx::points::PointsParams,
        lumit_core::fx::points::DrawStyle,
        lumit_core::fx::points::Schedule,
        lumit_core::fx::points::PointsSchedule,
    ) {
        let (mut inst, _, _) = particulate_fixture(mode, 20_000);
        inst.position_x = 16.0;
        inst.position_y = 12.0;
        inst.width = 18.0;
        inst.height = 12.0;
        inst.emit_rate = 30.0;
        inst.life = 0.9;
        inst.size = 4.0;
        inst.initial_speed = 14.0;
        inst.gravity = 20.0;
        inst.wind_x = 6.0;
        inst.wind_y = -2.0;
        inst.turbulence_amount = 4.0;
        inst.streak_length = 0.08;
        let dt = 1.0 / 24.0;
        let t = 24.0 * dt;
        let rate = f64::from(inst.emit_rate);
        let sched = lumit_core::fx::points::Schedule::scan(
            dt,
            (t / dt).floor() as i64,
            inst.window_frames(dt),
            &|_| rate,
        );
        let carriage = lumit_core::fx::points::PointsSchedule {
            schedule: sched.clone(),
            t,
            projection: None,
            ..Default::default()
        };
        (inst.points(), inst.draw_style(), sched, carriage)
    }

    /// The flat sprite Sprite mode's golden draws: solid, so what is pinned is
    /// *placement* rather than two bilinear filters.
    fn golden_sprite() -> Vec<f32> {
        vec![0.5; 8 * 8 * 4]
    }

    /// The three modes, in the order the golden file writes them.
    const GOLDEN_MODES: [(&str, u32); 3] = [("disc", 0), ("sprite", 1), ("streak", 2)];

    /// The eight attributes the golden pins, in order. `id` is exempt (a birth
    /// index, compared exactly) and colour is exempt (particulate.md §4
    /// declares it at half precision — the frames are what hold it).
    const STREAM_ATTRS: usize = 10;

    /// Particle `i`'s ten attributes, in the golden's order — three axes of
    /// position and speed since K-561.
    fn stream_attrs(s: &lumit_core::fx::points::PointsStream, i: usize) -> [f32; STREAM_ATTRS] {
        [
            s.position[i][0],
            s.position[i][1],
            s.position[i][2],
            s.speed[i][0],
            s.speed[i][1],
            s.speed[i][2],
            s.age[i],
            s.life[i],
            s.size[i],
            s.rotation[i],
        ]
    }

    /// The CPU reference's stream for the golden fixture.
    fn golden_stream() -> lumit_core::fx::points::PointsStream {
        let (packed, _, sched, carriage) = particulate_golden_fixture(0);
        lumit_core::fx::points::evaluate(
            &packed,
            &sched,
            carriage.t,
            &lumit_core::mask::MaskPolyline::default(),
        )
    }

    /// The CPU reference's picture for one mode, on the golden raster.
    fn golden_frame(mode: u32, sprite: &[f32]) -> Vec<f32> {
        let (packed, style, sched, carriage) = particulate_golden_fixture(mode);
        let (stream, tails) = lumit_core::fx::points::evaluate_with_tail(
            &packed,
            &sched,
            carriage.t,
            &lumit_core::mask::MaskPolyline::default(),
            style.streak_seconds,
        );
        let mut px = vec![0.0f32; (GOLDEN_W * GOLDEN_H * 4) as usize];
        lumit_core::fx::points::draw_stream(
            &mut px,
            GOLDEN_W,
            GOLDEN_H,
            &stream,
            &tails,
            &style,
            (mode == 1).then_some(lumit_core::fx::points::Sprite {
                rgba: sprite,
                w: 8,
                h: 8,
            }),
        );
        px
    }

    /// This run's expectation, in the golden file's format: a header line per
    /// block, then one number a line, so a failure diffs to the pixel.
    fn particulate_golden_text() -> String {
        use std::fmt::Write as _;
        let mut out = String::from(
            "# Particulate goldens (PS7, points-stream.md §5). Regenerate:\n\
             #   cargo test -p lumit-render --lib regenerate_particulate_goldens -- --ignored\n\
             # The stream is the CPU reference (K-019); the card's read-back and the\n\
             # three pictures are held to it within 10^-5 of range (K-508).\n",
        );
        let stream = golden_stream();
        let _ = writeln!(out, "ids {}", stream.len());
        for id in &stream.id {
            let _ = writeln!(out, "{id}");
        }
        let _ = writeln!(out, "stream {} {STREAM_ATTRS}", stream.len());
        for i in 0..stream.len() {
            for v in stream_attrs(&stream, i) {
                let _ = writeln!(out, "{v:.9e}");
            }
        }
        let sprite = golden_sprite();
        for (name, mode) in GOLDEN_MODES {
            let _ = writeln!(out, "frame {name} {GOLDEN_W} {GOLDEN_H}");
            for v in golden_frame(mode, &sprite) {
                let _ = writeln!(out, "{v:.9e}");
            }
        }
        out
    }

    /// The golden file as blocks: each header's words, then its numbers.
    fn golden_blocks() -> Vec<(Vec<String>, Vec<f64>)> {
        let text = std::fs::read_to_string(golden_path()).unwrap_or_default();
        let mut blocks: Vec<(Vec<String>, Vec<f64>)> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line.parse::<f64>() {
                Ok(v) => {
                    if let Some(b) = blocks.last_mut() {
                        b.1.push(v);
                    }
                }
                Err(_) => blocks.push((
                    line.split_whitespace().map(str::to_owned).collect(),
                    Vec::new(),
                )),
            }
        }
        blocks
    }

    /// The numbers of the block whose header starts with these words.
    fn golden_block(name: &str, second: Option<&str>) -> Vec<f64> {
        golden_blocks()
            .into_iter()
            .find(|(head, _)| {
                head.first().map(String::as_str) == Some(name)
                    && second.is_none_or(|s| head.get(1).map(String::as_str) == Some(s))
            })
            .map_or_else(Vec::new, |(_, v)| v)
    }

    /// K-508's measure over a whole block: every value agrees with its
    /// expectation to within 10⁻⁵ of the block's own range.
    fn assert_within_range(what: &str, got: &[f32], want: &[f64]) {
        assert_eq!(
            got.len(),
            want.len(),
            "{what}: {} numbers against the golden's {} — the fixture moved, not the maths. \
             Regenerate on purpose:\n  cargo test -p lumit-render --lib \
             regenerate_particulate_goldens -- --ignored",
            got.len(),
            want.len()
        );
        assert!(
            !want.is_empty(),
            "{what}: the golden file has no block for it"
        );
        let range = want.iter().fold(0.0f64, |a, v| a.max(v.abs())).max(1e-3);
        let mut worst = (0.0f64, 0usize);
        for (i, (a, b)) in got.iter().zip(want).enumerate() {
            let gap = (f64::from(*a) - *b).abs();
            if gap > worst.0 {
                worst = (gap, i);
            }
        }
        let rel = worst.0 / range;
        assert!(
            rel < 1e-5,
            "{what}: worst |Δ| {:.3e} at {} over a range of {range:.3e} — {rel:.2e} of range, \
             past K-508's 10⁻⁵. Either the closed forms moved or the golden is stale.",
            worst.0,
            worst.1
        );
    }

    /// **The goldens** (points-stream.md §5, PS7): the CPU reference's stream
    /// and its picture in all three render modes, against numbers checked into
    /// the repository.
    ///
    /// The two tests above hold the card to the CPU. Nothing held the CPU to
    /// anything — it *is* the oracle (K-019) — so a change that moved both
    /// together (a different noise lattice, a rearranged closed form, a drift
    /// in the birth schedule) would have passed every test in this file while
    /// drawing a different picture. This is what notices.
    ///
    /// It needs no graphics adapter, which is the point: this is the part of
    /// Particulate's conformance that gates on every runner there is.
    #[test]
    fn the_particulate_goldens_hold() {
        let stream = golden_stream();
        assert!(
            stream.len() > 8,
            "the golden fixture made only {} particles — it is not pinning anything",
            stream.len()
        );

        // `id` is a birth index, not a measurement: exact, or the compaction
        // has put a particle in the wrong slot.
        let ids = golden_block("ids", None);
        assert_eq!(
            ids.len(),
            stream.len(),
            "the golden pins {} ids and the stream has {}",
            ids.len(),
            stream.len()
        );
        for (i, (got, want)) in stream.id.iter().zip(&ids).enumerate() {
            assert_eq!(*got as f64, *want, "id {i} moved");
        }

        let flat: Vec<f32> = (0..stream.len())
            .flat_map(|i| stream_attrs(&stream, i))
            .collect();
        assert_within_range("the stream", &flat, &golden_block("stream", None));

        let sprite = golden_sprite();
        for (name, mode) in GOLDEN_MODES {
            let frame = golden_frame(mode, &sprite);
            let drawn: f32 = frame.iter().sum();
            assert!(drawn > 1.0, "the {name} golden draws nothing ({drawn})");
            assert_within_range(
                &format!("the {name} frame"),
                &frame,
                &golden_block("frame", Some(name)),
            );
        }
    }

    /// The **card's** half of the goldens: the compacted stream read back off
    /// the GPU, held to the same checked-in numbers at K-508's bound — the
    /// read-back hook `particulate_stream` was built for.
    ///
    /// The twin test says the two paths agree with each other; this says which
    /// numbers they agree *on*. Both drifting together is the one failure a
    /// twin test cannot see.
    #[test]
    fn the_particulate_gpu_stream_matches_the_goldens() {
        let Ok(ctx) = GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (GOLDEN_W, GOLDEN_H);
        let (packed, style, _, carriage) = particulate_golden_fixture(0);
        let frames = particulate_frames(&carriage);
        let mut curves = Vec::new();
        curves.extend_from_slice(&packed.particle.size_curve);
        curves.extend_from_slice(&packed.particle.opacity_curve);
        let op = particulate_op(
            &packed,
            style,
            &carriage,
            &frames,
            &curves,
            &[],
            0.0,
            0,
            carriage.projection.map(|proj| proj.m),
        );
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &vec![0.0; (w * h * 4) as usize], w, h);
        let (count, words) = fx
            .particulate_stream(&ctx, &tex, w, h, &op)
            .expect("the stream reads back");

        let ids = golden_block("ids", None);
        assert_eq!(
            count as usize,
            ids.len(),
            "the card made a different live set"
        );
        let cap = op.cap as usize;
        for (i, want) in ids.iter().enumerate() {
            let id =
                u64::from(words[12 * cap + i * 2]) | (u64::from(words[12 * cap + i * 2 + 1]) << 32);
            assert_eq!(id as f64, *want, "id {i} moved on the card");
        }
        let f = |i: usize| f32::from_bits(words[i]);
        let flat: Vec<f32> = (0..count as usize)
            .flat_map(|i| {
                [
                    f(i * 3),
                    f(i * 3 + 1),
                    f(i * 3 + 2),
                    f(3 * cap + i * 3),
                    f(3 * cap + i * 3 + 1),
                    f(3 * cap + i * 3 + 2),
                    f(6 * cap + i),
                    f(7 * cap + i),
                    f(8 * cap + i),
                    f(9 * cap + i),
                ]
            })
            .collect();
        assert_within_range("the GPU stream", &flat, &golden_block("stream", None));
    }

    /// Writes the golden file. Run it on purpose, after a deliberate change to
    /// the closed forms, and read the diff before committing — exactly the way
    /// `fx-reference.json` is regenerated.
    #[test]
    #[ignore = "writes the golden file; run after a deliberate change"]
    fn regenerate_particulate_goldens() {
        std::fs::write(golden_path(), particulate_golden_text())
            .expect("write particulate-golden.txt");
    }

    /// **The K-475 numbers** (docs/13 §2, rows B12–B14), measured rather than
    /// asserted.
    ///
    /// Ignored by default and printed with `--nocapture`, because a timing
    /// gate on whatever machine happens to be running CI teaches nobody
    /// anything: the gates themselves belong to the perf harness on the
    /// reference-desktop runner (PS7). What this is for is the number in the
    /// commit message and the number the next person checks against it.
    ///
    /// `cargo test -p lumit-render --lib particulate_budget -- --ignored --nocapture`
    #[test]
    #[ignore = "a measurement, not a gate — PS7 owns the harness"]
    fn particulate_budget_numbers() {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (1920u32, 1080u32);
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &vec![0.0; (w * h * 4) as usize], w, h);
        let dt = 1.0 / 60.0;
        for (name, rate, cap, size) in [
            // The floor first: the same call with nothing to draw, which is one
            // full-frame copy and one round trip to the queue. Every number
            // below carries it, and none of them is the effect.
            ("the floor (no particles)", 0.0f64, 20_000i32, 4.0f32),
            ("B12 default look", 150.0, 20_000, 4.0),
            ("B13 20 000 discs", 10_000.0, 20_000, 4.0),
            ("B14 the 1 000 000 hard cap", 500_000.0, 1_000_000, 2.0),
        ] {
            let (mut inst, _, _) = particulate_fixture(0, cap);
            inst.shape = 3;
            inst.position_x = 960.0;
            inst.position_y = 540.0;
            inst.width = 1600.0;
            inst.height = 900.0;
            inst.emit_rate = rate as f32;
            inst.size = size;
            inst.life = 2.0;
            let t = 4.0;
            let mut schedule = lumit_core::fx::points::Schedule::scan(
                dt,
                (t / dt).floor() as i64,
                inst.window_frames(dt),
                &|_| rate,
            );
            schedule.trim_to_newest(lumit_gpu::fx::MAX_CANDIDATES);
            let carriage = lumit_core::fx::points::PointsSchedule {
                schedule,
                t,
                projection: None,
                ..Default::default()
            };
            let (packed, style, frames, curves) = particulate_pieces(inst, &carriage);
            let op = particulate_op(
                &packed,
                style,
                &carriage,
                &frames,
                &curves,
                &[],
                0.0,
                0,
                carriage.projection.map(|proj| proj.m),
            );
            // A few runs to warm the driver and let the card clock up, then
            // twenty timed **without waiting between them**: a flush and a poll
            // per iteration measures the round trip to the queue, which is
            // latency the effect does not own and a real frame does not pay
            // once per effect.
            for _ in 0..4 {
                let _ = fx.particulate(&ctx, &tex, w, h, None, &op);
            }
            ctx.flush();
            ctx.device.poll(wgpu::Maintain::Wait);
            let runs = 20;
            let started = std::time::Instant::now();
            for _ in 0..runs {
                let _ = fx.particulate(&ctx, &tex, w, h, None, &op);
            }
            ctx.flush();
            ctx.device.poll(wgpu::Maintain::Wait);
            let each = started.elapsed().as_secs_f64() / f64::from(runs);
            // Nothing to draw answers `None` — the honest passthrough, and
            // the floor row's whole point.
            let count = fx
                .particulate_stream(&ctx, &tex, w, h, &op)
                .map_or(0, |(c, _)| c);
            println!(
                "{name}: {count} live of {} candidates, {:.3} ms evaluate + draw at {w}x{h}",
                op.candidates,
                each * 1000.0
            );
        }
    }

    /// The whole seam, end to end: an instance in a stack, a schedule threaded
    /// beside its op, and a picture that moved — plus the claim that the
    /// **degradation rung is never on the export path** (K-475).
    ///
    /// The rung is `ParticulateOp::cap`, and there is exactly one place that
    /// fills it. This asserts that place hands over the *declared* Max
    /// particles and nothing smaller, which is why no render — preview or
    /// export — can be quietly drawing half the field. When a governor signal
    /// reaches an effect kernel, this is the assertion that will have to be
    /// rewritten to say "except under pressure, and never on export".
    #[test]
    fn particulate_renders_through_run_ops_at_its_declared_cap() {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (96u32, 96u32);
        let mut inst =
            lumit_core::fx::instantiate("particulate").expect("particulate is a built-in");
        for p in &mut inst.params {
            let v = match p.id.as_str() {
                "position_x" | "position_y" => 48.0,
                "size" => 7.0,
                "emit_rate" => 400.0,
                _ => continue,
            };
            p.value = lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(v));
        }
        let t = 1.0;
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            t,
            136.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        let dt = 1.0 / 24.0;
        let carriage = lumit_core::fx::points::PointsSchedule {
            schedule: lumit_core::fx::points::Schedule::scan(
                dt,
                (t / dt).floor() as i64,
                200,
                &|_| 400.0,
            ),
            t,
            projection: None,
            ..Default::default()
        };

        let source = vec![0.0f32; (w * h * 4) as usize];
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            std::slice::from_ref(&carriage),
            None,
            None,
        );
        let got = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");
        let drawn: f32 = got.iter().sum();
        assert!(
            drawn > 1.0,
            "the stack drew no particles ({drawn}) — the schedule did not reach the pass"
        );

        // The declared cap, unreduced: nothing on this path halves it.
        use lumit_core::fx::effects::particulate::Particulate as P;
        let read = P::read(ops.iter().next().expect("one op").params);
        let packed = read.points();
        let frames = particulate_frames(&carriage);
        let mut curves = Vec::new();
        curves.extend_from_slice(&packed.particle.size_curve);
        curves.extend_from_slice(&packed.particle.opacity_curve);
        let op = particulate_op(
            &packed,
            read.draw_style(),
            &carriage,
            &frames,
            &curves,
            &[],
            0.0,
            0,
            None,
        );
        assert_eq!(
            op.cap,
            lumit_core::fx::points::CAP_DEFAULT as u32,
            "the pass asked for a reduced cap — the degradation rung is on a render path"
        );
    }

    /// **The cap rule and its degradation rung** (K-475, particulate.md §9
    /// item 7): over budget, what survives is the newest `cap` by birth index —
    /// and halving the cap keeps the newest half of *that*, which is the same
    /// rule applied twice rather than a second rule.
    #[test]
    fn particulate_keeps_the_newest_cap_and_halves_deterministically() {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (64u32, 64u32);
        let (inst, sched, carriage) = particulate_fixture(0, 20_000);
        let (packed, style, frames, curves) = particulate_pieces(inst, &carriage);
        let full = lumit_core::fx::points::evaluate(
            &packed,
            &sched,
            carriage.t,
            &lumit_core::mask::MaskPolyline::default(),
        );
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &vec![0.0; (w * h * 4) as usize], w, h);

        for divisor in [1u32, 2, 4] {
            let cap = (full.len() as u32) / divisor;
            let mut expected = full.clone();
            expected.keep_newest(cap as usize);
            let mut op = particulate_op(
                &packed,
                style,
                &carriage,
                &frames,
                &curves,
                &[],
                0.0,
                0,
                carriage.projection.map(|proj| proj.m),
            );
            op.cap = cap;
            let (count, words) = fx
                .particulate_stream(&ctx, &tex, w, h, &op)
                .expect("the stream reads back");
            assert_eq!(count, cap, "cap {cap}: drew {count}");
            let ids: Vec<u64> = (0..count as usize)
                .map(|i| {
                    u64::from(words[12 * cap as usize + i * 2])
                        | (u64::from(words[12 * cap as usize + i * 2 + 1]) << 32)
                })
                .collect();
            assert_eq!(ids, expected.id, "cap {cap}: the wrong particles survived");
        }
    }

    /// The two registries must agree, and nothing but a test can make them:
    /// they are joined by a string. Every migrated effect with an image
    /// operation needs exactly one GPU pass, and every GPU pass must name an
    /// effect that exists (docs/impl/effect-registry.md §7, test 3).
    ///
    /// **The compile-time halves only** (K-593). The run-time pair is registered
    /// by one call — [`ofx::register`] — which does both tables or neither, so
    /// there is no typo left for a test to catch; walking them here would only
    /// mean this test could see a registration another test was halfway through.
    #[test]
    fn every_migrated_effect_has_a_gpu_entry() {
        for def in BUILTIN_DEFS.builtins() {
            let name = def.schema().match_name;
            if def.is_image_op() {
                assert!(
                    gpu_effect(name).is_some(),
                    "{name} is migrated and draws pixels, but has no GPU pass"
                );
            } else {
                assert!(
                    gpu_effect(name).is_none(),
                    "{name} is orchestration-only and must not have a GPU pass"
                );
            }
        }
        for pass in GPU_EFFECTS {
            let name = pass.match_name();
            assert!(
                BUILTIN_DEFS.get(name).is_some(),
                "the GPU table names {name}, which no effect declares"
            );
        }
    }

    /// An effect whose real input is a file or another layer must say which
    /// list that input arrives on (K-387), because `resolve_into_arena`
    /// deliberately drops those parameter kinds: only the render knows which
    /// cube loaded or which layer was rendered, so nothing reaches the bag.
    ///
    /// This is the gate that makes the silence safe, and it lives here because
    /// it is the only place both halves are visible — the declaration in
    /// `lumit-core`, the consumption in this table. Without it, migrating an
    /// effect and forgetting its `aux()` is a picture that renders perfectly and
    /// quietly ignores its grade.
    #[test]
    fn a_side_table_effect_declares_the_list_it_consumes() {
        use lumit_core::fx::ParamKind;
        for def in BUILTIN_DEFS.builtins() {
            let name = def.schema().match_name;
            // The Matte row (K-395) is deliberately not counted: it is carried
            // by the one matte list beside `run_ops`'s dispatch, whoever
            // consumes it, and never by the effect's own `aux()`. Every effect
            // has one, so counting it would require all thirty-odd to declare a
            // list none of them reads. Its Invert goes with it — including the
            // older `depth_invert`, which is the same switch under K-065's
            // stored id.
            let matte_row = def.schema().matte.param();
            let has_file = def
                .schema()
                .params
                .iter()
                .any(|p| matches!(p.kind, ParamKind::File { .. }));
            // A Layer row that is not the matte is the **auxiliary-layer**
            // carriage, and since K-429 the schema is its own predicate: both
            // `build.rs` and `run_ops` walk `EffectSchema::layer_input`, so
            // what has to hold is that the schema finds the row — not that the
            // effect names a kind. That is what lets Fast motion blur take a
            // Motion vectors layer while its `aux()` is still the flow field.
            let extra_layer = def.schema().params.iter().any(|p| {
                let is_matte = matte_row.is_some_and(|m| p.id == m || p.id.starts_with(m));
                !is_matte
                    && p.id != lumit_core::fx::MATTE_INVERT_PARAM
                    && matches!(p.kind, ParamKind::Layer { .. })
            });
            if !has_file && !extra_layer {
                continue;
            }
            // A **driver** reads its layer reference on the CPU, at resolve
            // time (K-471): Audio level measures the referenced layer's sound
            // and hands out a number, so there is no pass to receive a picture
            // and no side table to fill. Anything that draws nothing is excused
            // for the same reason `every_migrated_effect_has_a_gpu_entry`
            // excuses it.
            if !def.is_image_op() {
                continue;
            }
            let gpu = gpu_effect(name).unwrap_or_else(|| {
                panic!("{name} takes a file or layer input but has no GPU pass to receive it")
            });
            if has_file {
                assert_ne!(
                    gpu.aux(),
                    AuxKind::None,
                    "{name} declares a file row, but its GPU pass claims no list — \
                     the input it was given would never arrive"
                );
            }
            if extra_layer {
                assert!(
                    def.schema().layer_input().is_some(),
                    "{name} declares a layer row that is not its matte, but \
                     EffectSchema::layer_input does not find it — the render would \
                     fill no slot for it and the picture would quietly ignore it"
                );
            }
        }
    }

    /// One name, one pass. Two wrappers answering to the same string would make
    /// which kernel runs depend on the order of this file.
    #[test]
    fn no_two_gpu_passes_share_a_name() {
        let mut seen: Vec<&str> = Vec::new();
        for name in gpu_effect_names() {
            assert!(!seen.contains(&name), "two GPU passes answer to {name}");
            seen.push(name);
        }
    }

    /// The whole path, end to end: a real effect instance resolves into the
    /// arena, `run_ops` finds this table by the effect's name, and the kernel
    /// draws what the CPU reference draws.
    ///
    /// This is the one link no compiler checks. Every other failure mode here is
    /// silent and looks like a picture: a lookup that misses leaves the texture
    /// untouched, and a bag read wrongly (250 where 2.5 was meant) still
    /// renders — just not the right thing. So the test pins both ends: the
    /// output must have *moved* from the input, and it must land where the CPU
    /// reference lands. The fp16 tolerance is the oracles' business
    /// (`wgsl_saturation_matches_the_cpu_oracle`); this only asks whether the
    /// right numbers reached the right kernel.
    #[test]
    fn a_migrated_effect_renders_through_run_ops() {
        let Ok(ctx) = GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (8u32, 8u32);
        let source: Vec<f32> = (0..(w * h * 4))
            .map(|i| match i % 4 {
                3 => 1.0,
                _ => (i % 13) as f32 / 13.0,
            })
            .collect();

        // A heavily desaturating instance, so a passthrough cannot pass for a
        // render: Saturation 20 % visibly greys the corpus.
        let mut inst = lumit_core::fx::instantiate("saturation").expect("saturation is a built-in");
        for p in &mut inst.params {
            if p.id == "saturation" {
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(20.0));
            }
        }
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
            None,
        );
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        let mut cpu = source.clone();
        lumit_core::fx::cpu::apply_stack(&mut cpu, w, h, &ops);

        assert_ne!(
            gpu, source,
            "the op passed the texture through — the GPU table was never reached"
        );
        for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
            assert!(
                (g - c).abs() < 1e-2,
                "pixel {i}: GPU {g} vs CPU reference {c} — the bag reached the \
                 kernel with the wrong numbers"
            );
        }
    }

    /// A definition that arrived at run time, as a plugin's does (K-593).
    ///
    /// Declared here rather than driven by a real OFX bundle because what is
    /// under test is the *seam*: `lumit-render` depends on no plugin host
    /// (docs/05), and what it can be shown is that an effect nobody compiled in
    /// resolves, dispatches and draws in the middle of a stack of built-ins.
    /// `lumit-ofx` proves the other half — that a real plugin makes one of
    /// these.
    struct RunTimeDef {
        schema: &'static lumit_core::fx::EffectSchema,
        /// Whether this definition's render fails, as a disabled plugin's does.
        fails: std::sync::atomic::AtomicBool,
    }

    impl lumit_core::fx::EffectDef for RunTimeDef {
        fn schema(&self) -> &'static lumit_core::fx::EffectSchema {
            self.schema
        }
        fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, _p: Params<'_>) {
            if self.fails.load(std::sync::atomic::Ordering::SeqCst) {
                return; // identity, byte for byte
            }
            for (i, v) in rgba.iter_mut().enumerate() {
                if i % 4 != 3 {
                    *v *= 0.5;
                }
            }
        }
        fn last_error(&self) -> Option<String> {
            self.fails
                .load(std::sync::atomic::Ordering::SeqCst)
                .then(|| "the plugin is disabled for this session".to_owned())
        }
    }

    /// A leaked declaration under a plugin-shaped name, as the OFX host leaks
    /// one for a plugin it has just described.
    fn a_run_time_def(match_name: &'static str) -> &'static RunTimeDef {
        Box::leak(Box::new(RunTimeDef {
            schema: Box::leak(Box::new(lumit_core::fx::EffectSchema {
                match_name,
                label: "Run-time effect",
                version: 1,
                category: lumit_core::fx::FxCategory::Utility,
                traits: lumit_core::fx::EffectTraits {
                    cost: lumit_core::fx::CostClass::Heavy,
                    roi: lumit_core::fx::Roi::FullFrame,
                    temporal: &[0],
                    premultiplied: true,
                    seeded: false,
                    beat_input: false,
                },
                params: &[],
                groups: &[],
                enabled_when: &[],
                matte: lumit_core::fx::MatteRole::None,
            })),
            fails: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    /// An instance of a registered effect, in the namespace a plugin's carries.
    fn a_run_time_instance(match_name: &str) -> lumit_core::model::EffectInstance {
        lumit_core::fx::instantiate(match_name).expect("the catalogue knows it")
    }

    /// **Built-in, plugin, built-in** (K-593): a stack with a run-time effect in
    /// the middle of it renders the picture the whole stack describes, through
    /// the same `run_ops` walk every built-in goes through.
    ///
    /// The middle op is the read-back wrapper: the picture comes off the card,
    /// through the definition, and back on again. What it must not do is get
    /// skipped — which is what would happen if the GPU table were built-in-only
    /// — and what it must not do is come back in the wrong order, which is what
    /// would happen if the arena had put it anywhere but second.
    #[test]
    fn a_run_time_effect_renders_between_two_builtins() {
        let Ok(ctx) = GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let def = a_run_time_def("ofx:test.render.stack");
        assert!(crate::gpufx::ofx::register(def), "it registered");
        // Both halves arrived, and the pass is the definition's own.
        assert!(gpu_effect("ofx:test.render.stack").is_some());
        assert!(BUILTIN_DEFS.get("ofx:test.render.stack").is_some());
        assert!(
            !crate::gpufx::ofx::register(def),
            "a second registration is a rescan, not a second effect"
        );

        let fx = FxEngine::new(&ctx);
        let (w, h) = (4u32, 4u32);
        let source: Vec<f32> = (0..(w * h * 4))
            .map(|i| match i % 4 {
                3 => 1.0,
                _ => (i % 7) as f32 / 7.0,
            })
            .collect();

        let stack = vec![
            lumit_core::fx::instantiate("invert").expect("a built-in"),
            a_run_time_instance("ofx:test.render.stack"),
            lumit_core::fx::instantiate("exposure").expect("a built-in"),
        ];
        let ops = lumit_core::fx::resolve_stack(
            &stack,
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        assert_eq!(
            ops.len(),
            3,
            "the plugin resolved into the middle of the stack"
        );

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
            None,
        );
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        let mut cpu = source.clone();
        lumit_core::fx::cpu::apply_stack(&mut cpu, w, h, &ops);
        assert_ne!(gpu, source, "the stack passed the texture through");
        for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
            assert!(
                (g - c).abs() < 1e-2,
                "pixel {i}: GPU {g} vs the same stack on the CPU {c}"
            );
        }
        assert!(
            crate::gpufx::ofx::errored_ops().is_empty(),
            "a render that worked left a badge behind"
        );
    }

    /// **A failed plugin renders identity and badges its own row** (docs/12
    /// §2.3, K-258's shape): the comp keeps compositing, the picture the middle
    /// op was given comes out of it unchanged, and the instance is named so the
    /// frontend can mark exactly that row.
    #[test]
    fn a_failed_run_time_effect_renders_identity_and_reports_it() {
        let Ok(ctx) = GpuContext::headless() else {
            return;
        };
        let def = a_run_time_def("ofx:test.render.errored");
        assert!(crate::gpufx::ofx::register(def));
        def.fails.store(true, std::sync::atomic::Ordering::SeqCst);

        let fx = FxEngine::new(&ctx);
        let (w, h) = (4u32, 4u32);
        let source: Vec<f32> = (0..(w * h * 4)).map(|i| (i % 5) as f32 / 5.0).collect();
        let inst = a_run_time_instance("ofx:test.render.errored");
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
            None,
        );
        let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");
        // The picture only ever went to fp16 and back, which is what the working
        // texture already is — so it is the same picture, exactly.
        let expected = lumit_gpu::fx::readback_linear_f32(
            &ctx,
            &lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h),
            w,
            h,
        )
        .expect("readback");
        assert_eq!(gpu, expected, "a failed plugin changed the picture");

        let badged = crate::gpufx::ofx::errored_ops();
        assert!(
            badged
                .iter()
                .any(|(id, why)| *id == inst.id && why.contains("disabled")),
            "the failure was not filed under the row it happened on: {badged:?}"
        );
        // And it is forgotten once the row is gone, or once it works again.
        crate::gpufx::ofx::clear_errored(inst.id);
        assert!(!crate::gpufx::ofx::errored_ops()
            .iter()
            .any(|(id, _)| *id == inst.id));
    }

    /// **Shake picks its own kernel** (docs/08 §3.4, T18/K-165, K-388).
    ///
    /// Shake is the one migrated effect whose dispatch forks: plain, it is the
    /// Transform kernel; with its own motion blur on, it is the averaging one,
    /// fed nine affines. Nothing but this test joins the fork to the bag — a
    /// wrapper that read the sub-frames and still called `transform` would
    /// render a picture, just not a smeared one — so both modes run end to end
    /// against the CPU reference, and the two must differ from each other.
    #[test]
    fn shake_renders_through_run_ops_in_both_modes() {
        let Ok(ctx) = GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (16u32, 16u32);
        let source: Vec<f32> = (0..(w * h * 4))
            .map(|i| match i % 4 {
                3 => 1.0,
                _ => (i % 13) as f32 / 13.0,
            })
            .collect();

        // A big wobble, so a passthrough cannot pass for a render: 8 % of this
        // raster's diagonal is a shift of a pixel or two, with a twist and a
        // depth pump on top.
        let shaken = |motion_blur: bool| {
            let mut inst = lumit_core::fx::instantiate("shake").expect("shake is a built-in");
            for p in &mut inst.params {
                let v = match p.id.as_str() {
                    "amplitude" => 8.0,
                    "rotation" => 6.0,
                    "z_amp" => 5.0,
                    "mb_amount" => 0.9,
                    "motion_blur" => {
                        p.value = lumit_core::model::EffectValue::Bool(motion_blur);
                        continue;
                    }
                    _ => continue,
                };
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(v));
            }
            lumit_core::fx::resolve_stack(
                std::slice::from_ref(&inst),
                0.4,
                ((w * w + h * h) as f32).sqrt(),
                1.0,
                &lumit_core::fx::MarkerContext::NONE,
                std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
            )
        };

        let rendered = |ops: &lumit_core::fx::ResolvedStack| {
            let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
            let out = crate::fxops::run_ops(
                &fx,
                &ctx,
                tex,
                w,
                h,
                ops,
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                None,
                None,
            );
            let gpu = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");
            let mut cpu = source.clone();
            lumit_core::fx::cpu::apply_stack(&mut cpu, w, h, ops);
            (gpu, cpu)
        };

        let (plain_gpu, plain_cpu) = rendered(&shaken(false));
        let (smeared_gpu, smeared_cpu) = rendered(&shaken(true));
        for (name, gpu, cpu) in [
            ("plain", &plain_gpu, &plain_cpu),
            ("smeared", &smeared_gpu, &smeared_cpu),
        ] {
            assert_ne!(
                gpu, &source,
                "{name}: the op passed the texture through — the GPU table was never reached"
            );
            for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
                assert!(
                    (g - c).abs() < 1e-2,
                    "{name} pixel {i}: GPU {g} vs CPU reference {c} — the bag reached the                      kernel with the wrong numbers"
                );
            }
        }
        assert_ne!(
            plain_gpu, smeared_gpu,
            "the motion-blur toggle must pick the other kernel"
        );
    }

    /// **The k-th LUT op binds the k-th cube** (docs/08 §3.11, K-387).
    ///
    /// The whole threading contract in one picture. `build.rs` enumerates a
    /// layer's enabled `lut` effects in stack order, and `run_ops` walks a
    /// counter down the ops in the same order; nothing but the counting joins
    /// the two, so a slot that is skipped or double-counted grades the wrong
    /// effect — a project where dragging one LUT above another moves the grade
    /// to a layer nobody touched.
    ///
    /// The failure this pins is the tempting one: advancing the counter only
    /// when a cube is actually there. The first slot here is deliberately
    /// **empty** (an unset or unreadable file — the passthrough every list
    /// allows), so an implementation that skips it hands the second op the first
    /// slot and renders no grade at all.
    #[test]
    fn the_kth_lut_op_binds_the_kth_slot() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (4u32, 4u32);
        // Opaque yellow: full red and green, no blue. Every channel sits on a
        // corner of the two-entry cube below, so the answer is the cube's own
        // value rather than an interpolation of it.
        let source: Vec<f32> = (0..(w * h)).flat_map(|_| [1.0f32, 1.0, 0.0, 1.0]).collect();

        // A grade that takes green to zero and leaves red and blue alone.
        let cube: Vec<[f32; 3]> = (0..8u32)
            .map(|i| [(i & 1) as f32, 0.0, ((i >> 2) & 1) as f32])
            .collect();
        let kill_green = crate::fxops::LoadedLut {
            texture: lumit_gpu::fx::upload_lut_3d(&ctx, 2, &cube),
            size: 2,
            path: String::new(),
            mtime: None,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
        };

        let inst = lumit_core::fx::instantiate("lut").expect("lut is a built-in");
        let ops = lumit_core::fx::resolve_stack(
            &[inst.clone(), inst],
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        assert_eq!(ops.len(), 2, "two LUT ops, two slots to bind");

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            &[],
            // Slot 0 empty, slot 1 the grade: only the *second* op grades.
            &[None, Some(kill_green)],
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
            None,
        );
        let got = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        assert!(
            got[1] < 1e-2,
            "green is {} — the second op did not bind the second slot",
            got[1]
        );
        assert!(
            (got[0] - 1.0).abs() < 1e-2 && got[2] < 1e-2,
            "the grade changed a channel it was told to leave alone: {got:?}"
        );
    }

    /// **A plate and a matte are two lists, and neither eats the other's slot**
    /// (docs/impl/layer-input.md §2, K-358, K-387, K-395).
    ///
    /// Depth of field and Light wrap used to share the layer-input list, off one
    /// predicate and one counter. K-395 split them — not arbitrarily: Light
    /// wrap's Background is a *plate* whose light spills round an edge, while
    /// Depth of field's depth pass is that effect's **matte**, and belongs on the
    /// one carriage every effect's matte uses. So the two now count along
    /// different lists, and the risk this test exists for is exactly that: an
    /// implementation where the Depth of field still advances the layer-input
    /// counter hands the wrap a slot that is not there, and the wrap silently
    /// stops wrapping.
    ///
    /// The stack is Depth of field then Light wrap, and the plate is the layer
    /// input list's **only** entry. If the two lists are properly separate, the
    /// wrap reads it and lights the foreground's edge; if the Depth of field is
    /// still counted, the wrap reads past the end and draws nothing at all.
    #[test]
    fn a_background_plate_and_a_matte_do_not_share_a_counter() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 24u32);

        // A dim opaque square in an empty frame: a real matte edge for the wrap
        // to find, and dark enough that a spill is unmistakable.
        let mut fg = vec![0.0f32; (w * h * 4) as usize];
        for y in 6..18u32 {
            for x in 8..24u32 {
                let i = ((y * w + x) * 4) as usize;
                fg[i] = 0.05;
                fg[i + 1] = 0.05;
                fg[i + 2] = 0.05;
                fg[i + 3] = 1.0;
            }
        }
        let plate: Vec<f32> = (0..(w * h) as usize)
            .flat_map(|_| [2.0f32, 2.0, 2.0, 1.0])
            .collect();

        // Stack order: Depth of field (its Matte row unset — a passthrough, and
        // no layer-input slot of its own since K-395), then Light wrap with a
        // real Width so it has something to draw.
        let dof = lumit_core::fx::instantiate("dof").expect("dof is a built-in");
        let mut wrap = lumit_core::fx::instantiate("light_wrap").expect("light_wrap is a built-in");
        for p in &mut wrap.params {
            if p.id == "width" {
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(5.0));
            }
        }
        let ops = lumit_core::fx::resolve_stack(
            &[dof, wrap],
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        assert_eq!(ops.len(), 2, "two ops: one plate to bind, two mattes");

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &fg, w, h);
        let plate_tex = lumit_gpu::fx::upload_linear_f32(&ctx, &plate, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            &[],
            &[],
            // The layer-input list carries Light wrap's Background plate and
            // nothing else: one consuming op, one slot, at index 0.
            &[crate::fxops::LayerInput::Texture(plate_tex)],
            &[],
            // Two matte slots, one per op — the Depth of field's unset (its
            // passthrough) and the wrap's unset too.
            &[
                crate::fxops::LayerInput::Absent,
                crate::fxops::LayerInput::Absent,
            ],
            &[],
            &[],
            None,
            None,
        );
        let got = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        // Just inside the square's left edge: the band the wrap paints.
        let at = |x: u32, y: u32| got[((y * w + x) * 4) as usize];
        assert!(
            at(9, 12) > 0.05 + 1e-2,
            "the edge band is {} — the wrap never saw the second slot",
            at(9, 12)
        );
        // And nowhere outside the matte: an empty pixel stays empty, which is
        // also what proves the Depth of field passed through rather than
        // gathering the plate it was never given.
        for i in (0..got.len()).step_by(4) {
            if fg[i + 3] == 0.0 {
                assert_eq!(
                    got[i + 3],
                    0.0,
                    "pixel {} gained coverage outside the matte",
                    i / 4
                );
            }
        }
    }

    /// **Echo is handed the neighbour frames themselves** (docs/08 §3.13,
    /// K-387). The whole-list kinds take no counter, which makes them look like
    /// the easy case — but an effect that receives an empty list where the
    /// render decoded four frames renders a perfectly ordinary picture with no
    /// trail on it, and nothing else in the pipeline notices. So the trail is
    /// asserted, not the plumbing: a dark frame with a bright neighbour behind
    /// it must come out brighter than it went in.
    #[test]
    fn echo_receives_the_decoded_neighbours() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (4u32, 4u32);
        let source: Vec<f32> = (0..(w * h)).flat_map(|_| [0.2f32, 0.2, 0.2, 1.0]).collect();
        let previous: Vec<f32> = (0..(w * h)).flat_map(|_| [0.8f32, 0.8, 0.8, 1.0]).collect();

        let inst = lumit_core::fx::instantiate("echo").expect("echo is a built-in");
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );

        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let neighbours = [(-1, lumit_gpu::fx::upload_linear_f32(&ctx, &previous, w, h))];
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &neighbours,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
            None,
        );
        let got = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        assert!(
            got[0] > 0.2 + 1e-2,
            "red is {} — the neighbour list never reached the kernel",
            got[0]
        );
    }

    /// **An unbound Matte renders byte-for-byte what it rendered before K-395**
    /// (K-258, the campaign's hardest invariant).
    ///
    /// Every effect gained two parameters. A project saved yesterday carries
    /// neither, and must draw exactly the same pixels today — not "within a
    /// tolerance", the same bits. Both halves are checked here because both can
    /// break on their own: a *legacy* instance stripped of the pair renders what
    /// a fresh instance renders, and a fresh instance with its Matte row unset
    /// renders what it rendered before the row existed — which is what the empty
    /// matte list stands for, since `build.rs` fills `Absent` for an unset row
    /// and `run_ops` treats a missing slot and an absent one alike.
    ///
    /// The failure this catches is the tempting implementation: running the
    /// dissolve unconditionally with `k = 1`. That is *nearly* free and *nearly*
    /// identity — and it is neither, because it costs a full-frame pass per
    /// effect and quantises the result through another fp16 store.
    #[test]
    fn an_unbound_matte_is_byte_identical() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (16u32, 16u32);
        let source: Vec<f32> = (0..(w * h * 4))
            .map(|i| match i % 4 {
                3 => 1.0,
                _ => (i % 13) as f32 / 13.0,
            })
            .collect();

        // A real stack, not one effect: a blur (a gather), a saturation (a
        // pointwise colour op) and a glow (a multi-pass) — three shapes of
        // kernel, so a stray pass anywhere in the chain shows up.
        let stack = |strip_matte: bool| {
            let mut insts = Vec::new();
            for (name, param, value) in [
                ("blur", "radius", 2.0f64),
                ("saturation", "saturation", 30.0),
                ("glow", "intensity", 1.5),
            ] {
                let mut inst = lumit_core::fx::instantiate(name).expect("a built-in");
                for p in &mut inst.params {
                    if p.id == param {
                        p.value = lumit_core::model::EffectValue::Float(
                            lumit_core::anim::Property::fixed(value),
                        );
                    }
                }
                if strip_matte {
                    inst.params.retain(|p| {
                        p.id != lumit_core::fx::MATTE_PARAM
                            && p.id != lumit_core::fx::MATTE_INVERT_PARAM
                    });
                }
                insts.push(inst);
            }
            lumit_core::fx::resolve_stack(
                &insts,
                0.0,
                ((w * w + h * h) as f32).sqrt(),
                1.0,
                &lumit_core::fx::MarkerContext::NONE,
                std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
            )
        };

        let rendered = |ops: &lumit_core::fx::ResolvedStack,
                        mattes: &[crate::fxops::LayerInput]| {
            let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
            let out = crate::fxops::run_ops(
                &fx,
                &ctx,
                tex,
                w,
                h,
                ops,
                &[],
                &[],
                &[],
                &[],
                &[],
                mattes,
                &[],
                &[],
                None,
                None,
            );
            lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback")
        };

        let fresh = rendered(&stack(false), &[]);
        assert_ne!(
            fresh, source,
            "the stack must actually have drawn something"
        );
        assert_eq!(
            rendered(&stack(true), &[]),
            fresh,
            "a project saved before the Matte row existed renders different pixels"
        );
        // And an explicitly Absent slot per op — what `build.rs` fills for an
        // unset row — is the same nothing as no list at all.
        let absent: Vec<crate::fxops::LayerInput> =
            (0..3).map(|_| crate::fxops::LayerInput::Absent).collect();
        assert_eq!(
            rendered(&stack(false), &absent),
            fresh,
            "an unset Matte row must not be a dissolve by one"
        );
    }

    /// **Set matte's source arrives on the layer-input carriage** (K-429).
    ///
    /// This one has a real way to fail silently. The effect used to take its
    /// picture off the Matte list and now takes it off the layer-input list;
    /// both are `Vec<LayerInput>` of the same shape, so a wrong wiring compiles,
    /// renders, and simply hands the kernel nothing — which is the documented
    /// no-op, and looks exactly like a project whose row is unset. So the test
    /// pins the whole route: a texture bound on the layer-input list must reach
    /// the kernel and cut the picture to it, the same texture on the *matte*
    /// list must reach nothing at all, and the schema must agree that this
    /// effect takes no matte.
    #[test]
    fn set_matte_reads_the_layer_input_carriage_and_no_matte() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (16u32, 16u32);
        // Opaque white, so any change in the picture is the coverage changing.
        let source: Vec<f32> = (0..(w * h)).flat_map(|_| [1.0f32, 1.0, 1.0, 1.0]).collect();
        // A left half that shows and a right half that does not.
        let shape: Vec<f32> = (0..(w * h))
            .flat_map(|i| {
                let lit = f32::from(i % w < w / 2);
                [lit, lit, lit, 1.0]
            })
            .collect();
        let shape_tex = lumit_gpu::fx::upload_linear_f32(&ctx, &shape, w, h);

        let inst = lumit_core::fx::instantiate("set_matte").expect("a built-in");
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            ((w * w + h * h) as f32).sqrt(),
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        let schema = ops.iter().next().expect("one op").def.schema();
        assert_eq!(
            schema.matte,
            lumit_core::fx::MatteRole::None,
            "Set matte carries no Matte row (K-429)"
        );
        assert_eq!(
            schema.layer_input(),
            Some(lumit_core::fx::MATTE_PARAM),
            "its source is the layer-input carriage, under its own stored id"
        );

        let rendered = |layers: &[crate::fxops::LayerInput],
                        mattes: &[crate::fxops::LayerInput]| {
            let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
            let out = crate::fxops::run_ops(
                &fx,
                &ctx,
                tex,
                w,
                h,
                &ops,
                &[],
                &[],
                &[],
                layers,
                &[],
                mattes,
                &[],
                &[],
                None,
                None,
            );
            lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback")
        };

        // Bound on the layer-input list: the picture wears the shape.
        let cut = rendered(&[crate::fxops::LayerInput::Texture(shape_tex.clone())], &[]);
        for y in 0..h {
            let left = ((y * w) * 4 + 3) as usize;
            let right = ((y * w + w - 1) * 4 + 3) as usize;
            assert_eq!(cut[left], 1.0, "the lit half must stay opaque");
            assert_eq!(cut[right], 0.0, "the dark half must be cut away");
        }

        // Unset: the labelled no-op, and the picture back untouched.
        let absent = rendered(&[crate::fxops::LayerInput::Absent], &[]);
        assert_eq!(absent, source, "an unset source must be the identity");

        // The same texture on the *matte* list reaches nothing: this effect
        // takes no slot off it, so a wiring that read it there would be reading
        // whichever effect's matte happened to sit above.
        assert_eq!(
            rendered(
                &[crate::fxops::LayerInput::Absent],
                &[crate::fxops::LayerInput::Texture(shape_tex)],
            ),
            source,
            "Set matte must consume nothing from the matte carriage"
        );
    }

    /// **Fast motion blur reads a flow field, a Motion vectors layer and a
    /// matte, all three** (K-429). The reason the auxiliary layer became a field
    /// on the slot rather than a sixth `AuxKind`: this effect already names the
    /// flow field, so a kind could not also carry its layer.
    ///
    /// Run it through `run_ops` on a layer with **no measured flow at all** and
    /// a vectors layer bound, which is the case the row exists for. It must
    /// smear; with the row unset and no field either it must be the documented
    /// passthrough.
    #[test]
    fn fast_motion_blur_takes_a_vectors_layer_beside_its_flow_field() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 16u32);
        // A hard vertical edge, so a sideways smear is unmistakable.
        let source: Vec<f32> = (0..(w * h))
            .flat_map(|i| {
                let lit = f32::from(i % w < w / 2);
                [lit, lit, lit, 1.0]
            })
            .collect();
        // Red well above the standing-still mid-grey: everything moves sideways.
        let vectors: Vec<f32> = (0..(w * h)).flat_map(|_| [0.9f32, 0.5, 0.5, 1.0]).collect();
        let vec_tex = lumit_gpu::fx::upload_linear_f32(&ctx, &vectors, w, h);

        let inst = lumit_core::fx::instantiate("motion_blur").expect("a built-in");
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            ((w * w + h * h) as f32).sqrt(),
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        let schema = ops.iter().next().expect("one op").def.schema();
        assert_eq!(
            schema.layer_input(),
            Some("motion_vectors"),
            "the vectors row is the auxiliary layer, not the matte"
        );
        assert_eq!(
            schema.matte.param(),
            Some(lumit_core::fx::MATTE_PARAM),
            "and the Matte row is still there beside it"
        );

        let rendered = |layers: &[crate::fxops::LayerInput]| {
            let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
            let out = crate::fxops::run_ops(
                &fx,
                &ctx,
                tex,
                w,
                h,
                &ops,
                &[],
                &[],
                &[],
                layers,
                &[],
                &[],
                &[],
                &[],
                None,
                None,
            );
            lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback")
        };

        // No field of either kind: the documented passthrough, never a fault.
        assert_eq!(
            rendered(&[crate::fxops::LayerInput::Absent]),
            source,
            "with no field at all the effect must pass the picture through"
        );
        // The layer alone is a field, on a layer that has no measured flow.
        let smeared = rendered(&[crate::fxops::LayerInput::Texture(vec_tex)]);
        assert_ne!(
            smeared, source,
            "a supplied vectors layer must smear a layer with no measured flow"
        );
        // And the smear is sideways: the edge has spread across columns that
        // were flat before.
        let row = (h / 2) as usize;
        let at = |x: usize| smeared[(row * w as usize + x) * 4];
        let mid = (w / 2) as usize;
        assert!(
            (at(mid) - at(mid + 3)).abs() > 1e-3,
            "the edge did not spread along the vector"
        );
    }

    /// **Both flow consumers on one layer are served** (K-544).
    ///
    /// Fast motion blur measures forward to the next frame, Datamosh measures
    /// back to the previous one. The layer used to carry a single field and the
    /// first of the two in stack order took it, so the other read nothing and
    /// silently rendered its passthrough. They are separate measurements now,
    /// keyed by the offset each asked for.
    ///
    /// The proof is a stack carrying both, run three ways over the same picture:
    /// with both fields bound, with only the `+1` field, and with only the `-1`
    /// field. If each effect is reading its own measurement, the full run
    /// differs from *both* one-field runs — the missing one having degraded that
    /// effect to a passthrough. Under the old single-slot routing one of those
    /// comparisons was an equality, because the second effect never saw a field
    /// whichever way round it was asked.
    #[test]
    fn both_flow_consumers_on_one_layer_read_their_own_measurement() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (32u32, 16u32);
        let n = (w * h) as usize;
        // A hard vertical edge: a sideways smear or a melt is unmistakable.
        let source: Vec<f32> = (0..n)
            .flat_map(|i| {
                let lit = f32::from(i as u32 % w < w / 2);
                [lit, lit, lit, 1.0]
            })
            .collect();
        // The -1 neighbour Datamosh drags along its field: the same edge shifted,
        // so dragging it in is visible.
        let prev: Vec<f32> = (0..n)
            .flat_map(|i| {
                let lit = f32::from((i as u32 % w + 6) < w / 2);
                [0.2 * lit, lit, 0.2 * lit, 1.0]
            })
            .collect();
        let neighbours = vec![(-1i32, lumit_gpu::fx::upload_linear_f32(&ctx, &prev, w, h))];

        // Two different measurements, one per offset. Sideways for the +1
        // (forward) field, downwards for the -1 (backward) one, so a field
        // handed to the wrong consumer would not merely be wrong, it would move
        // the picture the wrong way.
        let field = |u: f32, v: f32| {
            lumit_gpu::fx::upload_flow_field(&ctx, &vec![u; n], &vec![v; n], &vec![1.0f32; n], w, h)
        };

        let mb = lumit_core::fx::instantiate("motion_blur").expect("a built-in");
        let dm = lumit_core::fx::instantiate("datamosh").expect("a built-in");
        assert_eq!(
            lumit_core::fx::stack_flow_neighbours(&[mb.clone(), dm.clone()], true),
            vec![-1, 1],
            "the stack must ask for both measurements"
        );
        // Datamosh first, then the blur: at Intensity 1 the melt replaces the
        // picture outright, so with it on top nothing upstream of it could show
        // and the test would prove nothing about the blur.
        let ops = lumit_core::fx::resolve_stack(
            &[dm, mb],
            0.0,
            ((w * w + h * h) as f32).sqrt(),
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );

        let rendered = |fields: &[(i32, wgpu::Texture)]| {
            let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
            let out = crate::fxops::run_ops(
                &fx,
                &ctx,
                tex,
                w,
                h,
                &ops,
                &neighbours,
                fields,
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                None,
                None,
            );
            lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback")
        };

        let both = rendered(&[(-1, field(0.0, 5.0)), (1, field(7.0, 0.0))]);
        let blur_only = rendered(&[(1, field(7.0, 0.0))]);
        let mosh_only = rendered(&[(-1, field(0.0, 5.0))]);

        assert_ne!(
            both, source,
            "the stack must do something to the picture at all"
        );
        assert_ne!(
            both, blur_only,
            "Datamosh must read its own -1 field, not silently do nothing              because Fast motion blur was asked first"
        );
        assert_ne!(
            both, mosh_only,
            "Fast motion blur must read its own +1 field, not silently do              nothing because Datamosh was asked first"
        );
        // Neither one-field run is the passthrough either: each effect on its
        // own field really is changing the picture, so the comparisons above
        // are about routing rather than about one of them being inert.
        assert_ne!(blur_only, source, "the +1 field alone must still blur");
        assert_ne!(mosh_only, source, "the -1 field alone must still melt");
    }

    /// **An override's matte is spent ONCE** (K-395).
    ///
    /// The hook's whole risk in one test. An effect that claims the matte inside
    /// its maths must not *also* be dissolved by it beside the dispatch — the
    /// matte would then be applied twice, and the give-away is that the picture
    /// would be neither the kernel's answer nor the dissolve's but a blend of
    /// the two. At a mid-grey matte the three are all different, which is why
    /// the matte here is grey rather than the black or white that would let a
    /// double application hide.
    ///
    /// So: run a Gaussian blur through `run_ops` with a grey matte bound, and
    /// demand the result is **bit-identical** to calling the blur kernel with
    /// that matte directly. Any extra pass — a lerp, an fp16 round-trip, a
    /// dissolve by one — shows up immediately.
    #[test]
    fn an_override_is_not_also_dissolved() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (16u32, 16u32);
        // A hard edge, so a blur has something to do and a dissolve of one would
        // be visible in the numbers.
        let source: Vec<f32> = (0..(w * h))
            .flat_map(|i| {
                let lit = f32::from(i % w < 8);
                [lit, lit, lit, 1.0]
            })
            .collect();
        let grey: Vec<f32> = (0..(w * h)).flat_map(|_| [0.5f32, 0.5, 0.5, 1.0]).collect();
        let matte_tex = lumit_gpu::fx::upload_linear_f32(&ctx, &grey, w, h);

        let mut inst = lumit_core::fx::instantiate("blur").expect("blur is a built-in");
        for p in &mut inst.params {
            if p.id == "radius" {
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(20.0));
            }
        }
        let ops = lumit_core::fx::resolve_stack(
            std::slice::from_ref(&inst),
            0.0,
            ((w * w + h * h) as f32).sqrt(),
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        let (radius_px, edge, mix) =
            lumit_core::fx::effects::blur::Blur::read(ops.iter().next().expect("one op").params)
                .packed();

        let through_run_ops = {
            let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
            let out = crate::fxops::run_ops(
                &fx,
                &ctx,
                tex,
                w,
                h,
                &ops,
                &[],
                &[],
                &[],
                &[],
                &[],
                &[crate::fxops::LayerInput::Texture(matte_tex.clone())],
                &[],
                &[],
                None,
                None,
            );
            lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback")
        };
        let kernel_alone = {
            let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
            let out = fx.blur(
                &ctx,
                &tex,
                w,
                h,
                Some(&matte_tex),
                &lumit_gpu::fx::BlurOp {
                    radius_px,
                    edge,
                    mix,
                },
            );
            lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback")
        };
        assert_ne!(
            kernel_alone, source,
            "the blur must actually have blurred, or this proves nothing"
        );
        assert_eq!(
            through_run_ops, kernel_alone,
            "an effect that claims the matte was ALSO dissolved by it — the \
             matte applied twice"
        );
    }

    /// **The k-th matte-carrying op binds the k-th matte slot** (K-395, the
    /// K-387 contract with its second predicate).
    ///
    /// The mirror of `the_kth_lut_op_binds_the_kth_slot`, and it exists for the
    /// same reason: nothing but the counting joins `build.rs`'s enumeration to
    /// `run_ops`'s walk, so a slot skipped or double-counted drives the wrong
    /// effect — a project where reordering the stack moves which effect a matte
    /// controls.
    ///
    /// The first slot is deliberately **absent** (the unset row every list
    /// allows), so an implementation that advances its counter only when a matte
    /// is really there hands the second op the first op's slot and dissolves
    /// nothing. The second slot is black, which switches its op off entirely —
    /// so the picture must be "the first effect applied, the second not".
    #[test]
    fn the_kth_matte_op_binds_the_kth_slot() {
        let Ok(ctx) = GpuContext::headless() else {
            lumit_gpu::no_adapter();
            return;
        };
        let fx = FxEngine::new(&ctx);
        let (w, h) = (4u32, 4u32);
        // Opaque mid grey, so both an exposure up and an exposure down are
        // visible and neither clips.
        let source: Vec<f32> = (0..(w * h))
            .flat_map(|_| [0.25f32, 0.25, 0.25, 1.0])
            .collect();

        let exposure = |stops: f64| {
            let mut inst = lumit_core::fx::instantiate("exposure").expect("exposure is a built-in");
            for p in &mut inst.params {
                if p.id == "stops" {
                    p.value = lumit_core::model::EffectValue::Float(
                        lumit_core::anim::Property::fixed(stops),
                    );
                }
            }
            inst
        };
        let ops = lumit_core::fx::resolve_stack(
            &[exposure(2.0), exposure(-2.0)],
            0.0,
            1000.0,
            1.0,
            &lumit_core::fx::MarkerContext::NONE,
            std::sync::Arc::new(lumit_core::expression::ExpressionContext::detached()),
        );
        assert_eq!(ops.len(), 2, "two ops, two matte slots to bind");

        let black: Vec<f32> = (0..(w * h)).flat_map(|_| [0.0f32, 0.0, 0.0, 1.0]).collect();
        let off =
            crate::fxops::LayerInput::Texture(lumit_gpu::fx::upload_linear_f32(&ctx, &black, w, h));
        let tex = lumit_gpu::fx::upload_linear_f32(&ctx, &source, w, h);
        let out = crate::fxops::run_ops(
            &fx,
            &ctx,
            tex,
            w,
            h,
            &ops,
            &[],
            &[],
            &[],
            &[],
            &[],
            // Slot 0 absent (the first exposure applies in full), slot 1 black
            // (the second is switched off).
            &[crate::fxops::LayerInput::Absent, off],
            &[],
            &[],
            None,
            None,
        );
        let got = lumit_gpu::fx::readback_linear_f32(&ctx, &out, w, h).expect("readback");

        assert!(
            (got[0] - 1.0).abs() < 1e-2,
            "red is {} — expected 0.25 lifted two stops and NOT pulled back \
             down, i.e. the second op bound the second slot",
            got[0]
        );
    }
}
