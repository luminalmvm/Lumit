//! Denoise: camera noise taken out of footage, measured first.
//!
//! **In plain terms.** Learn finds the flattest patch of the frame and measures
//! the noise on it, by size and by brightness. The frames either side are then
//! moved along the measured motion and averaged in where they agree with this
//! one, and what noise is left is shrunk band by band
//! ([`crate::fx::denoise`]). Every amount is relative to the measurement, so
//! the defaults suit any footage once it has been measured.
//!
//! The neighbours are the layer's own source frames, so Denoise belongs first
//! in the stack: effects above it are not run on them. Anywhere else it reads
//! no neighbours and works as it does at Frames 0. Where a neighbour
//! disagrees with the frame (something moved that the motion missed, or the
//! other side of a cut) its patches stand out from the noise and it is left
//! out there.

use crate::anim::Animation;
use crate::fx::denoise::{self, DenoiseParams};
use crate::fx::temporal::FlowWant;
use crate::fx::{
    params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
    schema::ParamGroup,
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

/// Denoise's groups, in the order its rows run.
pub const DENOISE_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Noise by size",
        params: &[
            "luma_fine",
            "luma_medium",
            "luma_coarse",
            "luma_broad",
            "chroma_fine",
            "chroma_medium",
            "chroma_coarse",
            "chroma_broad",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Noise by tone",
        params: &["shadows", "midtones", "highlights"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Temporal",
        params: &["frames", "motion", "temporal"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Spatial",
        params: &["luma", "chroma"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Spatial by size",
        params: &["fine", "medium", "coarse", "broad"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Finish",
        params: &["detail", "sharpen", "grain"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// The most frames either side Denoise reads.
pub const MAX_FRAMES: i32 = 5;

/// Denoise's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "denoise",
    label = "Denoise",
    version = 1,
    category = BlurSharpen,
    // A warp and an average per neighbour, then four bands.
    cost = Heavy,
    // The deepest band reaches 2 + 4 + 8 + 16 px, and its 5x5 energy window two more.
    roi = PaddedPx(32.0),
    // The kernel works on straight colour and leaves alpha as it found it.
    premultiplied = false,
    // Up to five frames either side; `frames_needed` narrows it to the row.
    temporal = &[-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5],
    groups = DENOISE_GROUPS,
    // Inside the kernel, so the Removed noise view shows what is taken.
    matte = (
        "matte",
        "scales the reduction per pixel: white takes all the noise the rows say, grey less, black none",
    ),
)]
pub struct Denoise {
    /// What the effect shows: the result, what it took away, or how many
    /// frames went into each pixel.
    #[choice(options = ["Result", "Removed noise", "Frames blended"], default = 0)]
    pub view: u32,

    /// Find the flattest patch at the playhead, move the sample there and
    /// measure it.
    #[action(label = "Learn")]
    pub learn: (),

    /// The centre of the patch the noise is measured on.
    #[slider(label = "Sample x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub sample_x: f32,

    /// See [`sample_x`](Self::sample_x).
    #[slider(label = "Sample y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub sample_y: f32,

    /// The side of the patch.
    #[slider(
        label = "Sample size",
        min = 16.0,
        max = 512.0,
        default = 96.0,
        hard_min = 16.0,
        hard_max = 512.0,
        unit = Px
    )]
    pub sample_size: f32,

    /// The measured luma noise: the sigma of encoded Y in eight-bit code values.
    #[slider(label = "Luma noise", min = 0.0, max = 50.0, default = 3.0, hard_min = 0.0, unit = Raw)]
    pub luma_noise: f32,

    /// The measured noise of Cb and Cr together, in the same units.
    #[slider(label = "Chroma noise", min = 0.0, max = 50.0, default = 2.0, hard_min = 0.0, unit = Raw)]
    pub chroma_noise: f32,

    /// Each band's luma noise as a per cent of Luma noise, finest first.
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub luma_fine: f32,
    #[slider(min = 0.0, max = 400.0, default = 60.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub luma_medium: f32,
    #[slider(min = 0.0, max = 400.0, default = 35.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub luma_coarse: f32,
    #[slider(min = 0.0, max = 400.0, default = 20.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub luma_broad: f32,

    /// Each band's chroma noise as a per cent of Chroma noise.
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub chroma_fine: f32,
    #[slider(min = 0.0, max = 400.0, default = 80.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub chroma_medium: f32,
    #[slider(min = 0.0, max = 400.0, default = 60.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub chroma_coarse: f32,
    #[slider(min = 0.0, max = 400.0, default = 45.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub chroma_broad: f32,

    /// Luma noise in the shadows, midtones and highlights, as a per cent of
    /// Luma noise.
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub shadows: f32,
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub midtones: f32,
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub highlights: f32,

    /// Frames either side averaged in. 0 is the spatial pass alone.
    #[counter(min = 0, max = 5, default = 2, hard_min = 0, hard_max = 5, unit = Frames)]
    pub frames: i32,

    /// Line the neighbours up along the measured motion first. Off averages
    /// them where they are, for a locked-off shot.
    #[toggle(label = "Follow motion", default = true)]
    pub motion: bool,

    /// How different a neighbour may be and still be averaged in.
    #[slider(
        label = "Temporal strength",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 200.0,
        unit = Percent
    )]
    pub temporal: f32,

    /// How much of the luma noise left after the average is taken.
    #[slider(min = 0.0, max = 100.0, default = 75.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub luma: f32,

    /// How much of the chroma noise is taken.
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub chroma: f32,

    /// How much is taken from each band, finest first.
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub fine: f32,
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub medium: f32,
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub coarse: f32,
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub broad: f32,

    /// Fine texture kept where it stands above the noise.
    #[slider(min = 0.0, max = 100.0, default = 25.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub detail: f32,

    /// The fine bands lifted where they are picture, never where they are noise.
    #[slider(min = 0.0, max = 200.0, default = 0.0, hard_min = 0.0, hard_max = 200.0, unit = Percent)]
    pub sharpen: f32,

    /// This much of the fine luma noise taken away put back.
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub grain: f32,

    /// The host-uniform Mix every effect ends with, per cent.
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub mix: f32,
}

impl Denoise {
    /// What both render paths read. Every row is clamped here, once.
    #[must_use]
    pub fn packed(self) -> DenoiseParams {
        let pc = |v: f32, hi: f32| (v / 100.0).clamp(0.0, hi);
        let strength = pc(self.temporal, 2.0);
        DenoiseParams {
            view: self.view.min(2),
            luma_sigma: self.luma_noise.max(0.0) / 255.0,
            chroma_sigma: self.chroma_noise.max(0.0) / 255.0,
            tone: [
                pc(self.shadows, 4.0),
                pc(self.midtones, 4.0),
                pc(self.highlights, 4.0),
            ],
            luma_bands: [
                pc(self.luma_fine, 4.0),
                pc(self.luma_medium, 4.0),
                pc(self.luma_coarse, 4.0),
                pc(self.luma_broad, 4.0),
            ],
            chroma_bands: [
                pc(self.chroma_fine, 4.0),
                pc(self.chroma_medium, 4.0),
                pc(self.chroma_coarse, 4.0),
                pc(self.chroma_broad, 4.0),
            ],
            frames: self.frames.clamp(0, MAX_FRAMES) as u32,
            motion: self.motion,
            t0: 0.5 + strength,
            t1: 0.5 + 2.0 * strength,
            strength,
            luma_amount: pc(self.luma, 1.0),
            chroma_amount: pc(self.chroma, 1.0),
            band_amounts: [
                pc(self.fine, 1.0),
                pc(self.medium, 1.0),
                pc(self.coarse, 1.0),
                pc(self.broad, 1.0),
            ],
            detail: pc(self.detail, 1.0),
            sharpen: pc(self.sharpen, 2.0),
            grain: pc(self.grain, 1.0),
            mix: pc(self.mix, 1.0),
        }
    }

    /// The most frames either side `inst` can read: its Frames row, or the
    /// whole window when the row is keyframed or driven by an expression,
    /// since the window is asked for without a time.
    #[must_use]
    pub fn reach(inst: &EffectInstance) -> i32 {
        match inst.param("frames") {
            Some(EffectValue::Float(p)) if !matches!(p.animation, Animation::Static(_)) => {
                MAX_FRAMES
            }
            Some(EffectValue::Float(p)) => (p.value_at(0.0).round() as i32).clamp(0, MAX_FRAMES),
            _ => 2,
        }
    }

    /// Every frame `inst` reads either side, each with the motion it wants
    /// to it: measured with Follow motion on, none with it off. Either way
    /// the decode leaves out a neighbour that is another shot or the same
    /// source frame again, and only a neighbour with a field is averaged in.
    #[must_use]
    pub fn flow_wants(inst: &EffectInstance) -> Vec<(i32, FlowWant)> {
        let want = match inst.param("motion") {
            Some(EffectValue::Bool(false)) => FlowWant::Still,
            _ => FlowWant::Measured,
        };
        let r = Self::reach(inst);
        (-r..=r).filter(|&o| o != 0).map(|o| (o, want)).collect()
    }
}

/// Denoise's behaviour.
pub struct DenoiseDef;

impl EffectDef for DenoiseDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Denoise as EffectMetadata>::SCHEMA
    }

    /// The spatial pass alone: the fallback is handed no neighbours.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let out = denoise::run(rgba, w, h, &[], &Denoise::read(p).packed(), &[]);
        if out.len() == rgba.len() {
            rgba.copy_from_slice(&out);
        }
    }

    fn view(&self) -> Option<&'static str> {
        Some("view")
    }

    fn frames_needed(&self, inst: &EffectInstance, _frame: f64) -> Option<Vec<i32>> {
        let r = Denoise::reach(inst);
        Some((-r..=r).collect())
    }
}
