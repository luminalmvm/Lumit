//! Audio spectrum: a layer's sound drawn as bars, a line, a ring of bars or a
//! waveform.
//!
//! **In plain terms.** The sound around the frame is measured into a row of
//! numbers, one per band, and those numbers become geometry: a bar for each band
//! along a line, a drawn mask or a circle, or one line through the tips.
//! Waveform reads the raw samples instead of measuring them. Every piece gets
//! its colours here, on the host, so both render paths draw the same numbers.
//!
//! The sound can't reach an effect's own row (a frame's reading is too long a
//! list to sit in the bag), so the host reads it and hands it over beside the op
//! as a list of points, down the same road a mask's path takes ([`levels_of`]).

pub mod draw;
pub mod sound;
#[cfg(test)]
mod tests;

use self::draw::{laid_out, render};
use self::sound::silence;

use std::sync::Arc;

use crate::expression::ExpressionContext;
use crate::fx::drivers::audio_level::{SOURCE_CLIP, SOURCE_LAYER, SOURCE_OPTIONS};
use crate::fx::points::{PointsStream, Projection};
use crate::fx::registry::AudioTap;
use crate::fx::schema::{ParamGroup, Port, PortType};
use crate::fx::{
    params::ParamId, params::Params, params::Value, registry::EffectDef, registry::EffectMetadata,
    registry::ResolveCx, registry::RowState, registry::Signature, schema::EffectSchema,
};
use crate::mask::MaskPolyline;
use crate::model::{EffectInstance, Layer};
use lumit_fx_macros::Effect;

/// The name the render's own carriage asks for.
pub const MATCH_NAME: &str = "audio_spectrum";

/// The row whose slot beside the op holds the reading rather than a path.
pub const LEVELS_ROW: &str = "levels";

/// The `shape` options, in the order the Choice stores them.
pub const SHAPE_BARS: u32 = 0;
pub const SHAPE_LINE: u32 = 1;
pub const SHAPE_RING: u32 = 2;
pub const SHAPE_WAVEFORM: u32 = 3;

const POINTS_OUT: &[Port] = &[Port::new("points", "Points", PortType::Points)];

const fn group(
    label: &'static str,
    params: &'static [&'static str],
    collapsed: bool,
    visible_when: Option<(&'static str, &'static [u32])>,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed,
        visible_when,
        visible_when_lens_elements: None,
    }
}

/// The `along` options, in the order the Choice stores them.
pub const ALONG_LINE: u32 = 0;
pub const ALONG_MASK: u32 = 1;

/// The rows one choice or another has no use for. Waveform reads samples, so it
/// has no frequencies to choose.
pub const AUDIO_SPECTRUM_GROUPS: &[ParamGroup] = &[
    group(
        "",
        &["audio"],
        false,
        Some(("source", &[SOURCE_LAYER, SOURCE_CLIP])),
    ),
    group("", &["clip"], false, Some(("source", &[SOURCE_CLIP]))),
    group(
        "",
        &["start_frequency", "end_frequency"],
        false,
        Some(("shape", &[SHAPE_BARS, SHAPE_LINE, SHAPE_RING])),
    ),
    group(
        "",
        &["from_x", "from_y", "to_x", "to_y"],
        false,
        Some(("along", &[ALONG_LINE])),
    ),
    group("", &["path"], false, Some(("along", &[ALONG_MASK]))),
];

/// Audio spectrum's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "audio_spectrum",
    label = "Audio spectrum",
    version = 1,
    category = Generate,
    cost = Moderate,
    roi = FullFrame,
    groups = AUDIO_SPECTRUM_GROUPS,
)]
pub struct AudioSpectrum {
    /// Which sound is measured, as Audio level asks it: the comp's own mix, the
    /// layer the Audio row names, or one clip of that layer.
    #[choice(options = SOURCE_OPTIONS, default = 0)]
    pub source: u32,

    /// The layer *Layer* and *Clip* measure.
    #[layer(label = "Audio", self_default = false)]
    pub audio: bool,

    /// The clip *Clip* measures, from the clips of the layer above.
    #[clip]
    pub clip: (),

    /// How the sound is drawn. Ring lays the bars round a circle half as wide as
    /// From is from To, centred between them.
    #[choice(options = ["Bars", "Line", "Ring", "Waveform"], default = 0)]
    pub shape: u32,

    /// How many bars, or points along a line.
    #[counter(
        min = 2,
        max = 256,
        default = 64,
        hard_min = 2,
        hard_max = 512,
        unit = Raw
    )]
    pub bands: i32,

    /// The lowest pitch drawn, in hertz.
    #[slider(
        label = "Start frequency",
        min = 20.0,
        max = 20000.0,
        default = 40.0,
        hard_min = 1.0,
        unit = Raw
    )]
    pub start_frequency: f32,

    /// The highest pitch drawn, in hertz.
    #[slider(
        label = "End frequency",
        min = 20.0,
        max = 20000.0,
        default = 16000.0,
        hard_min = 1.0,
        unit = Raw
    )]
    pub end_frequency: f32,

    /// What the bands are laid along: From and To, or a mask.
    #[choice(label = "Draw along", options = ["Line", "Mask"], default = 0)]
    pub along: u32,

    /// px@comp. The defaults run across the middle of a nominal 1080p frame.
    #[slider(label = "From x", min = 0.0, max = 3840.0, default = 360.0, unit = Px)]
    pub from_x: f32,

    /// px@comp.
    #[slider(label = "From y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub from_y: f32,

    /// px@comp.
    #[slider(label = "To x", min = 0.0, max = 3840.0, default = 1560.0, unit = Px)]
    pub to_x: f32,

    /// px@comp.
    #[slider(label = "To y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub to_y: f32,

    /// Which mask to follow. Unset is the first one.
    #[mask_path(label = "Path")]
    pub path: bool,

    /// How far a band at full level reaches, px@comp.
    #[slider(min = 0.0, max = 1080.0, default = 320.0, hard_min = 0.0, unit = Px)]
    pub height: f32,

    /// How wide a bar or the line is, px@comp.
    #[slider(min = 0.5, max = 100.0, default = 10.0, hard_min = 0.1, unit = Px)]
    pub thickness: f32,

    /// How much of the width its edge fades over, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub softness: f32,

    /// The colour where a bar starts. Scene-linear and open above 1.
    #[colour(label = "Inside colour", default = [0.05, 0.3, 1.0, 1.0], max = 4.0)]
    pub inside_colour: [f32; 4],

    /// The colour a band at full level reaches.
    #[colour(label = "Outside colour", default = [1.0, 0.15, 0.5, 1.0], max = 4.0)]
    pub outside_colour: [f32; 4],

    /// How far the colours turn round the wheel from the first band to the last.
    #[slider(
        label = "Hue spread",
        min = 0.0,
        max = 360.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 720.0,
        unit = Degrees
    )]
    pub hue_spread: f32,

    /// On, each band reaches both ways from the line.
    #[toggle(default = false)]
    pub mirror: bool,

    /// How many frames before this one are averaged in, so the bars fall away
    /// rather than flicker. On Waveform, how much sound one sweep shows.
    #[counter(
        min = 0,
        max = 30,
        default = 2,
        hard_min = 0,
        hard_max = 120,
        unit = Frames
    )]
    pub smoothing: i32,

    /// Never a row anybody sees: the slot beside the op this frame's reading
    /// arrives in.
    #[mask_path(label = "Levels", self_default = false)]
    pub levels: bool,

    /// On, the layer stays under the bars. Off, the bars are all there is,
    /// which is what a new solid wants: it starts white.
    #[toggle(label = "Composite on original", default = false)]
    pub composite_on_original: bool,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl AudioSpectrum {
    /// Raster pixels per comp pixel, for the path and the rows in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");
}

/// The numbers a reading's slot holds, the first coordinate of each point.
#[must_use]
pub fn levels_of(slot: &MaskPolyline) -> Vec<f32> {
    slot.points.iter().map(|p| p[0]).collect()
}

/// The stream the Points socket hands on: one point at each band's tip, sized
/// and coloured as it is drawn, so a Particulate emitting from it sprays off the
/// bars. In the layer's own pixels, as every stream is on the walk.
#[must_use]
pub fn stream(
    p: Params<'_>,
    inst: &EffectInstance,
    layer: Option<&Layer>,
    t: f64,
    (fps, context, audio): (f64, &Arc<ExpressionContext>, Option<&dyn AudioTap>),
    projection: Projection,
) -> PointsStream {
    let s = AudioSpectrum::read(p);
    let path = crate::mask::effect_path_at(inst, "path", true, layer, t);
    let levels = sound::reading(inst, t, fps, context, audio);
    draw::tips(&s, &levels, &path, projection)
}

/// Audio spectrum's behaviour.
pub struct AudioSpectrumDef;

impl EffectDef for AudioSpectrumDef {
    fn schema(&self) -> &'static EffectSchema {
        &<AudioSpectrum as EffectMetadata>::SCHEMA
    }

    /// The picture, and the bars' tips as points for Particulate to emit from.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    fn row_state(&self, _inst: &EffectInstance) -> RowState {
        RowState {
            hidden: vec![LEVELS_ROW],
            ..RowState::default()
        }
    }

    /// The reading arrives beside the op, which this never sees, so this draws
    /// the shape at silence. The card draws what it was handed.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let s = AudioSpectrum::read(p);
        let px_scale = p.float(AudioSpectrum::DERIVED_PX_SCALE, 1.0);
        let levels = silence(&s);
        let laid = laid_out(&s, px_scale, &levels, &MaskPolyline::default(), w, h);
        render(rgba, w, h, &laid);
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(AudioSpectrum::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }

    /// Raster pixels per comp pixel follows the raster as a length does.
    fn derived_spatial(&self) -> &'static [ParamId] {
        &[AudioSpectrum::DERIVED_PX_SCALE]
    }

    /// The sound is different on every frame, whatever the rows are doing.
    fn moves_with_time(&self, _e: &EffectInstance) -> bool {
        true
    }
}
