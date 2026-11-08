//! Energy line: a hot core of light with a glow round it, drawn along a line
//! between two points, a layer's masks, a Shape layer's paths or a Text
//! layer's letters.
//!
//! **In plain terms.** The path is cut down to the stretch between Start and
//! End, slid round by Offset, and drawn as a soft tube whose width can taper
//! from one end to the other. The tube's middle is the core colour and its
//! edge the glow colour. The glow is the tube blurred by five round
//! exponentials, the same halo Glow builds with Falloff up, so it stays bright
//! near the line and fades a long way out. Noise can push the core and the
//! glow about, and the whole light can flicker.
//!
//! The path is built here, on the host, as straight pieces in raster pixels,
//! and handed to both render paths as the same numbers. Each piece is also
//! filed into the 16 px tiles it can reach, so a pixel only measures the
//! pieces near it, which is what lets a whole paragraph of text be one line.

pub mod draw;
pub mod geometry;
mod presets;
#[cfg(test)]
mod tests;

use self::draw::{render, Look};
use self::geometry::{bins, pieces, pieces_each, Bins, Pieces};

use crate::fx::{
    cpu, noise::FractalField, params::ParamId, params::Params, params::Value, registry::EffectDef,
    registry::EffectMetadata, registry::Pressed, registry::ResolveCx, schema::EffectSchema,
    schema::ParamGroup,
};
use crate::mask::MaskPolyline;
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

/// The `source` options, in the order the Choice stores them.
pub const SOURCE_LINE: u32 = 0;
pub const SOURCE_MASK: u32 = 1;
pub const SOURCE_SHAPE: u32 = 2;
pub const SOURCE_TEXT: u32 = 3;

/// The most moments a blurred shutter draws. Each is the whole path again, so
/// this is what keeps a blurred line inside the frame budget.
pub const MAX_SAMPLES: usize = 16;

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

/// The rows only one source uses, then the twirls.
pub const ENERGY_LINE_GROUPS: &[ParamGroup] = &[
    group(
        "",
        &["start_x", "start_y", "end_x", "end_y"],
        false,
        Some(("source", &[SOURCE_LINE])),
    ),
    group(
        "",
        &["mask_path", "all_masks"],
        false,
        Some(("source", &[SOURCE_MASK])),
    ),
    group(
        "",
        &["paths"],
        false,
        Some(("source", &[SOURCE_MASK, SOURCE_SHAPE, SOURCE_TEXT])),
    ),
    group(
        "Core",
        &[
            "core_size",
            "core_softness",
            "core_colour",
            "halo",
            "start_size",
            "end_size",
        ],
        false,
        None,
    ),
    group(
        "Glow",
        &["glow_colour", "glow_intensity", "glow_size", "glow_falloff"],
        false,
        None,
    ),
    group("Flicker", &["flicker", "flicker_speed"], true, None),
    group(
        "Distortion",
        &[
            "distortion_type",
            "core_distortion",
            "glow_distortion",
            "distortion_size",
            "distortion_detail",
            "distortion_speed",
            "wind_direction",
            "wind_speed",
        ],
        true,
        None,
    ),
];

/// Energy line's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "energy_line",
    label = "Energy line",
    version = 1,
    category = Generate,
    cost = Moderate,
    // The glow is read far out of the picture's own pixels.
    roi = FullFrame,
    // Straight alpha in and out, which is how the compositor reads a layer's
    // picture (draw::combine).
    premultiplied = false,
    groups = ENERGY_LINE_GROUPS,
)]
pub struct EnergyLine {
    /// A look to start from. Picking one writes its values into the rows
    /// below, and editing any of them puts this back to Custom.
    #[choice(
        label = "Preset",
        options = *presets::NAMES,
        default = 0
    )]
    pub preset: u32,

    /// What the light is drawn along.
    #[choice(
        label = "Draw along",
        options = ["Line", "Mask", "Shape", "Text"],
        default = 0
    )]
    pub source: u32,

    /// px@comp. The defaults run across the middle of a nominal 1080p frame.
    #[slider(label = "From x", min = 0.0, max = 3840.0, default = 480.0, unit = Px)]
    pub start_x: f32,

    /// px@comp.
    #[slider(label = "From y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub start_y: f32,

    /// px@comp.
    #[slider(label = "To x", min = 0.0, max = 3840.0, default = 1440.0, unit = Px)]
    pub end_x: f32,

    /// px@comp.
    #[slider(label = "To y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub end_y: f32,

    /// Which mask to follow. Unset is the first one.
    #[mask_path(label = "Mask")]
    pub mask_path: bool,

    /// On, every mask on the layer is drawn and the Mask row is passed over.
    #[toggle(label = "Every mask", default = true)]
    pub all_masks: bool,

    /// Whether several paths (masks, a shape's items, the letters) light one
    /// after another along Start and End, or all at once, each on its own.
    #[choice(
        label = "Paths",
        options = ["One after another", "All at once"],
        default = 0
    )]
    pub paths: u32,

    /// Where the light starts, per cent of the way along the path.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub start: f32,

    /// Where it stops. Keyframe it up from Start and the line draws itself on.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub end: f32,

    /// Slides the lit stretch along the path, per cent of its length. On a
    /// closed path it comes round again.
    #[slider(min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub offset: f32,

    /// The core's width, px@comp.
    #[slider(
        label = "Core size",
        min = 0.0,
        max = 100.0,
        default = 10.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub core_size: f32,

    /// How much of the core's width its edge fades over, per cent.
    #[slider(
        label = "Core softness",
        min = 0.0,
        max = 100.0,
        default = 40.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub core_softness: f32,

    /// The colour in the middle of the core. Scene-linear and open above 1,
    /// because it is a light.
    #[colour(label = "Core colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub core_colour: [f32; 4],

    /// A tight, hot glow hugging the core, per cent.
    #[slider(
        min = 0.0,
        max = 200.0,
        default = 60.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub halo: f32,

    /// The core's width at the start, per cent of Core size.
    #[slider(
        label = "Start size",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub start_size: f32,

    /// The core's width at the end, per cent of Core size.
    #[slider(
        label = "End size",
        min = 0.0,
        max = 200.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub end_size: f32,

    /// The glow's colour, which also tints the core's edge.
    #[colour(label = "Glow colour", default = [0.06, 0.28, 1.0, 1.0], max = 4.0)]
    pub glow_colour: [f32; 4],

    /// How bright the glow is, per cent.
    #[slider(
        label = "Glow intensity",
        min = 0.0,
        max = 400.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub glow_intensity: f32,

    /// How far the glow reaches, px@comp.
    #[slider(
        label = "Glow size",
        min = 0.0,
        max = 400.0,
        default = 60.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub glow_size: f32,

    /// How much of the glow is sent far out rather than kept by the core,
    /// per cent.
    #[slider(
        label = "Glow falloff",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub glow_falloff: f32,

    /// How far the light dips when it flickers, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub flicker: f32,

    /// Dips a second.
    #[slider(
        label = "Flicker speed",
        min = 0.0,
        max = 30.0,
        default = 12.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub flicker_speed: f32,

    /// The noise that pushes the light about: smooth swells, curling smoke,
    /// or a fast jagged crackle.
    #[choice(
        label = "Type",
        options = ["Smooth", "Smoke", "Crackle"],
        default = 0
    )]
    pub distortion_type: u32,

    /// How far the core is pushed, px@comp.
    #[slider(
        label = "Core distortion",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub core_distortion: f32,

    /// How far the glow is pushed, px@comp.
    #[slider(
        label = "Glow distortion",
        min = 0.0,
        max = 200.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub glow_distortion: f32,

    /// The size of the noise's swirls, px@comp.
    #[slider(
        label = "Noise size",
        min = 4.0,
        max = 400.0,
        default = 60.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub distortion_size: f32,

    /// How many layers of finer noise ride on the first.
    #[counter(
        label = "Detail",
        min = 1,
        max = 6,
        default = 3,
        hard_min = 1,
        hard_max = 6,
        unit = Raw
    )]
    pub distortion_detail: i32,

    /// How fast the noise changes, a second.
    #[slider(
        label = "Evolution speed",
        min = 0.0,
        max = 10.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub distortion_speed: f32,

    /// The way the noise is carried, degrees. 0 is up the frame.
    #[slider(
        label = "Wind direction",
        min = -180.0,
        max = 180.0,
        default = 0.0,
        unit = Degrees
    )]
    pub wind_direction: f32,

    /// How fast it is carried, px@comp a second.
    #[slider(
        label = "Wind speed",
        min = 0.0,
        max = 1000.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub wind_speed: f32,

    #[seed]
    pub seed: u32,

    /// On, the layer stays under the light. Off, the light is all there is,
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

impl EnergyLine {
    /// Raster pixels per comp pixel, for the path's vertices and the wind.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");
    /// The flicker's gain this frame, 0..1.
    pub const DERIVED_FLICKER: ParamId = ParamId::new("derived.flicker");
    /// How far the noise has evolved this frame.
    pub const DERIVED_EVOLUTION: ParamId = ParamId::new("derived.evolution");
    /// How far the wind has carried the noise, px@comp, x and y.
    pub const DERIVED_WIND: ParamId = ParamId::new("derived.wind");
    /// How many moments across a blurred shutter are drawn; 0 when nothing
    /// moves across it, or the layer doesn't blur.
    pub const DERIVED_SAMPLES: ParamId = ParamId::new("derived.samples");
    /// Each moment's From and To, px@comp.
    pub const DERIVED_SAMPLE_LINE: [ParamId; MAX_SAMPLES] = [
        ParamId::new("derived.line0"),
        ParamId::new("derived.line1"),
        ParamId::new("derived.line2"),
        ParamId::new("derived.line3"),
        ParamId::new("derived.line4"),
        ParamId::new("derived.line5"),
        ParamId::new("derived.line6"),
        ParamId::new("derived.line7"),
        ParamId::new("derived.line8"),
        ParamId::new("derived.line9"),
        ParamId::new("derived.line10"),
        ParamId::new("derived.line11"),
        ParamId::new("derived.line12"),
        ParamId::new("derived.line13"),
        ParamId::new("derived.line14"),
        ParamId::new("derived.line15"),
    ];
    /// Each moment's Start, End and Offset.
    pub const DERIVED_SAMPLE_TRIM: [ParamId; MAX_SAMPLES] = [
        ParamId::new("derived.trim0"),
        ParamId::new("derived.trim1"),
        ParamId::new("derived.trim2"),
        ParamId::new("derived.trim3"),
        ParamId::new("derived.trim4"),
        ParamId::new("derived.trim5"),
        ParamId::new("derived.trim6"),
        ParamId::new("derived.trim7"),
        ParamId::new("derived.trim8"),
        ParamId::new("derived.trim9"),
        ParamId::new("derived.trim10"),
        ParamId::new("derived.trim11"),
        ParamId::new("derived.trim12"),
        ParamId::new("derived.trim13"),
        ParamId::new("derived.trim14"),
        ParamId::new("derived.trim15"),
    ];

    /// The numbers both render paths draw with. `None` at Mix 0, which both
    /// take as the untouched input. Nothing lit is still a look: with
    /// Composite on original off it leaves nothing at all.
    #[must_use]
    pub fn look(self, p: Params<'_>) -> Option<Look> {
        let mix = (self.mix / 100.0).clamp(0.0, 1.0);
        let core_half = self.core_size.max(0.0) * 0.5;
        if mix <= 0.0 {
            return None;
        }
        let px_scale = p.float(Self::DERIVED_PX_SCALE, 1.0).max(1e-6);
        let size = self.distortion_size.max(1.0);
        let wind = p.vec4(Self::DERIVED_WIND, [0.0; 4]);
        let rgb = |c: [f32; 4]| [c[0].max(0.0), c[1].max(0.0), c[2].max(0.0)];
        let radius = self.glow_size.max(0.0);
        // The halo of a line `w` wide is about `w / radius` as bright as the
        // line, so the gain makes up the difference: the glow beside a thin
        // core is as bright as beside a thick one.
        let gain = if radius > 0.0 && core_half > 0.0 {
            self.glow_intensity.max(0.0) / 100.0 * (radius / (2.0 * core_half)).max(1.0) * 0.6
        } else {
            0.0
        };
        let (perlin, turbulent, lacunarity) = match self.distortion_type {
            1 => (true, true, 2.0),
            2 => (false, false, 2.7),
            _ => (true, false, 2.0),
        };
        Some(Look {
            core_half,
            taper: [
                self.start_size.max(0.0) / 100.0,
                self.end_size.max(0.0) / 100.0,
            ],
            softness: (self.core_softness / 100.0).clamp(0.0, 1.0),
            core_colour: rgb(self.core_colour),
            halo: self.halo.max(0.0) / 100.0,
            glow_colour: rgb(self.glow_colour),
            glow_gain: gain,
            glow_radius: radius,
            glow_falloff: falloff(self.glow_falloff),
            core_warp: self.core_distortion.max(0.0),
            glow_warp: self.glow_distortion.max(0.0),
            field: FractalField {
                seed: self.seed,
                octaves: self.distortion_detail.clamp(1, 6) as u32,
                gain: 0.5,
                lacunarity,
                perlin,
                turbulent,
                cycle: 0,
            },
            inv_size: 1.0 / size,
            evolution: p.float(Self::DERIVED_EVOLUTION, 0.0),
            drift: [wind[0] * px_scale / size, wind[1] * px_scale / size],
            flicker: p.float(Self::DERIVED_FLICKER, 1.0),
            composite: self.composite_on_original,
            mix,
            samples: 1,
        })
    }

    /// The path the Line source draws, px@comp.
    #[must_use]
    pub fn line(self, px_scale: f32) -> MaskPolyline {
        let k = 1.0 / px_scale.max(1e-6);
        line_between(
            [self.start_x * k, self.start_y * k],
            [self.end_x * k, self.end_y * k],
        )
    }

    /// How far past the core the glow and the noise reach, raster pixels.
    #[must_use]
    pub fn reach(look: &Look) -> f32 {
        // ponytail: the widest exponential reads 73 radii out; this keeps up
        // to 20, where it is down to about a hundredth, as Glow does.
        let ratio = 1.0 - 1.0 / (1.0 + look.glow_falloff);
        let glow = if look.glow_gain > 0.0 {
            look.glow_radius * (2.0 + 18.0 * ratio) + look.glow_warp
        } else {
            0.0
        };
        core_reach(look) + glow
    }

    /// The pieces and tiles for one frame, from the path in px@comp. Across
    /// a blurred shutter it is laid out once a moment, each moment's line and
    /// stretch where they were then, and `look.samples` says how many.
    #[must_use]
    pub fn laid_out(
        self,
        look: &mut Look,
        p: Params<'_>,
        poly: &MaskPolyline,
        px_scale: f32,
        w: u32,
        h: u32,
    ) -> (Pieces, Bins) {
        let width = 2.0 * look.core_half * look.taper[0].max(look.taper[1]);
        let lay = if self.paths == 1 { pieces_each } else { pieces };
        let n = (p.int(Self::DERIVED_SAMPLES, 0).max(0) as usize).min(MAX_SAMPLES);
        let mut laid = Pieces::default();
        if n == 0 {
            laid = lay(
                poly,
                px_scale,
                [self.start, self.end, self.offset],
                0,
                width,
            );
            look.samples = 1;
        } else {
            for i in 0..n {
                let [sx, sy, ex, ey] = p.vec4(Self::DERIVED_SAMPLE_LINE[i], [0.0; 4]);
                let [a, b, off, _] = p.vec4(Self::DERIVED_SAMPLE_TRIM[i], [0.0; 4]);
                let line;
                let path = if self.source == SOURCE_LINE {
                    line = line_between([sx, sy], [ex, ey]);
                    &line
                } else {
                    poly
                };
                laid.extend(lay(path, px_scale, [a, b, off], i as u32, width));
            }
            look.samples = n as u32;
        }
        let tiles = bins(&laid, core_reach(look), w, h);
        (laid, tiles)
    }
}

/// A straight path from `a` to `b`, px@comp.
fn line_between(a: [f32; 2], b: [f32; 2]) -> MaskPolyline {
    let len = (b[0] - a[0]).hypot(b[1] - a[1]);
    MaskPolyline {
        points: vec![a, b],
        arc: vec![0.0, len],
        corners: vec![0, 1],
        ..MaskPolyline::default()
    }
}

/// How far from a piece the core pass reads it: the core, its halo and the
/// noise's push, raster pixels.
fn core_reach(look: &Look) -> f32 {
    let r = look.core_half * look.taper[0].max(look.taper[1]);
    let halo = if look.halo > 0.0 {
        r * draw::HALO_REACH
    } else {
        0.0
    };
    r + halo + look.core_warp + 1.0
}

/// Glow falloff's per cent onto Glow's Falloff, where 0 keeps the light in
/// the first exponential and higher sends more of it out.
fn falloff(pct: f32) -> f32 {
    (pct / 100.0).clamp(0.0, 1.0) * 1.5
}

/// Whether the light changes from frame to frame with every row held still.
/// Only then does the layer time join its frame key.
#[must_use]
pub fn moves_with_time(e: &EffectInstance) -> bool {
    let on = |id: &str| match e.param(id) {
        Some(EffectValue::Float(p)) => p.is_animated() || p.value_at(0.0) != 0.0,
        _ => false,
    };
    let keyed = |id: &str| match e.param(id) {
        Some(EffectValue::Float(p)) => p.is_animated(),
        _ => false,
    };
    // Across a blurred shutter a moving line is drawn where it is either side
    // of the frame, which this frame's rows alone don't say.
    let travels = [
        "start_x", "start_y", "end_x", "end_y", "start", "end", "offset",
    ]
    .iter()
    .any(|id| keyed(id));
    let flickers = on("flicker") && on("flicker_speed");
    let pushed = on("core_distortion") || on("glow_distortion");
    travels || flickers || (pushed && (on("distortion_speed") || on("wind_speed")))
}

/// The line and the stretch at each moment of a blurred shutter, when the
/// layer blurs and something moves across it.
fn shutter(cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
    let (e, ctx) = (cx.inst, &cx.context);
    let Some(comp) = ctx.comp.and_then(|id| ctx.document.comp(id)) else {
        return;
    };
    let blurs = comp.motion_blur.enabled
        && ctx
            .layer
            .and_then(|id| comp.layers.iter().find(|l| l.id == id))
            .is_some_and(|l| l.switches.motion_blur);
    if !blurs {
        return;
    }
    let mut offsets = comp.motion_blur.sample_offsets();
    if offsets.len() > MAX_SAMPLES {
        offsets = crate::model::MotionBlur {
            samples: MAX_SAMPLES as u32,
            ..comp.motion_blur
        }
        .sample_offsets();
    }
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    let at = |t: f64| {
        let f =
            |id: &str, or: f64| e.float_at_with_context(id, t, ctx.clone()).unwrap_or(or) as f32;
        (
            [
                f("start_x", 480.0),
                f("start_y", 540.0),
                f("end_x", 1440.0),
                f("end_y", 540.0),
            ],
            [f("start", 0.0), f("end", 100.0), f("offset", 0.0), 0.0],
        )
    };
    let now = at(cx.lt);
    let moments: Vec<_> = offsets
        .iter()
        .take(MAX_SAMPLES)
        .map(|off| at(cx.lt + off * dt))
        .collect();
    if moments.iter().all(|m| *m == now) {
        return;
    }
    for (i, (line, trim)) in moments.iter().enumerate() {
        push(EnergyLine::DERIVED_SAMPLE_LINE[i], Value::Vec4(*line));
        push(EnergyLine::DERIVED_SAMPLE_TRIM[i], Value::Vec4(*trim));
    }
    push(
        EnergyLine::DERIVED_SAMPLES,
        Value::Int(moments.len() as i32),
    );
}

/// Energy line's behaviour.
pub struct EnergyLineDef;

impl EffectDef for EnergyLineDef {
    fn schema(&self) -> &'static EffectSchema {
        &<EnergyLine as EffectMetadata>::SCHEMA
    }

    /// The Line source has its path in the rows, so it draws here too. The
    /// others are handed their path beside the op, which this never sees, and
    /// leave the picture as it was.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let s = EnergyLine::read(p);
        if s.source != SOURCE_LINE {
            return;
        }
        let Some(mut look) = s.look(p) else {
            return;
        };
        let px_scale = p.float(EnergyLine::DERIVED_PX_SCALE, 1.0);
        let (laid, tiles) = s.laid_out(&mut look, p, &s.line(px_scale), px_scale, w, h);
        render(rgba, w, h, &look, &laid, &tiles);
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let e = cx.inst;
        let fl = |id: &str, or: f64| {
            e.float_at_with_context(id, cx.lt, cx.context.clone())
                .unwrap_or(or)
        };
        let seed = match e.param("seed") {
            Some(EffectValue::Seed(s)) => *s,
            _ => 0,
        };
        push(EnergyLine::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
        // A smooth wander between 0 and 1, so the light dips and recovers
        // rather than blinking.
        let amount = (fl("flicker", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
        let turns = (cx.lt * fl("flicker_speed", 12.0).max(0.0)) as f32;
        let dip = 0.5 + 0.5 * crate::fx::noise::value3(seed, 7, turns, 0.0, 0.0, 0);
        push(
            EnergyLine::DERIVED_FLICKER,
            Value::Float(1.0 - amount * dip),
        );
        let evolution = cx.lt * fl("distortion_speed", 1.0).max(0.0);
        push(
            EnergyLine::DERIVED_EVOLUTION,
            Value::Float(evolution as f32),
        );
        // The noise is carried with the wind, so the field reads from behind.
        let travelled = cx.lt * fl("wind_speed", 0.0).max(0.0);
        let angle = fl("wind_direction", 0.0).to_radians();
        let wind = [
            (-travelled * angle.sin()) as f32,
            (travelled * angle.cos()) as f32,
            0.0,
            0.0,
        ];
        push(EnergyLine::DERIVED_WIND, Value::Vec4(wind));
        shutter(cx, push);
    }

    /// Raster pixels per comp pixel follows the raster as a length does.
    fn derived_spatial(&self) -> &'static [ParamId] {
        &[EnergyLine::DERIVED_PX_SCALE]
    }

    /// Text and shape pictures end at their ink, so the glow is given room.
    /// A line or a mask is usually drawn on a frame-sized layer, where the
    /// room would only cost.
    fn spread_px(&self, p: Params<'_>) -> [f32; 2] {
        let s = EnergyLine::read(p);
        if s.source != SOURCE_SHAPE && s.source != SOURCE_TEXT {
            return [0.0, 0.0];
        }
        let reach = s.look(p).map_or(0.0, |look| EnergyLine::reach(&look));
        [reach, reach]
    }

    fn moves_with_time(&self, e: &EffectInstance) -> bool {
        moves_with_time(e)
    }

    fn settle(&self, before: Option<&EffectInstance>, after: &EffectInstance) -> Option<Pressed> {
        presets::settle(before, after)
    }
}

/// The halo both paths build, so the GPU walks the CPU's numbers.
#[must_use]
pub fn octaves(look: &Look, w: u32, h: u32) -> [cpu::GlowOctave; cpu::GLOW_OCTAVES] {
    cpu::glow_octaves(look.glow_radius, look.glow_falloff, w, h)
}
