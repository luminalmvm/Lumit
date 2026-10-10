//! Text to points makes one point for each character, word or line of a
//! Text layer.
//!
//! Put it on the Text layer itself and each point sits on its piece of the
//! text, so a Clone to points or a Connect points below can follow the
//! letters. The points are where the layer draws its letters in its own
//! space, and they move when an animator pushes a letter. A point's number
//! is its count along the text, or the count of the character, word or line
//! it starts, so the effects below can work word by word on points made per
//! character. On a path it turns to face the way the path runs.
//!
//! The letters are laid out by the text engine, which this crate cannot
//! call, so the draw builder lays them out and hands them over beside the op
//! (`PointsSchedule::text`). That makes this a stream that exists only while
//! a frame is drawn, as Scatter's is: an effect below it in the stack gets
//! it, and a driver or another layer reads an empty stream.

use crate::fx::points::{self, DrawStyle, PointsStream, Projection, RenderMode, TextGlyph};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup, ParamId, Params,
    Port, PortType, ResolveCx, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The same Points output every producer declares.
const POINTS_OUT: &[Port] = &[Port::new("points", "Points", PortType::Points)];

const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// Which text and which pieces of it, and what a point looks like.
pub const TEXT_TO_POINTS_GROUPS: &[ParamGroup] = &[
    group(
        "Text",
        &["text_layer", "per", "include_spaces", "place", "number"],
    ),
    group("Point", &["size_from_width", "size", "feather", "colour"]),
];

/// A space only gets a point of its own when each character does, and Size
/// is not read while the piece's own width is the size.
pub const TEXT_TO_POINTS_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "include_spaces",
        on: "per",
        cond: EnabledCond::ChoiceIs(0),
    },
    EnabledWhen {
        param: "size",
        on: "size_from_width",
        cond: EnabledCond::BoolIs(false),
    },
];

/// Text to points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "text_to_points",
    label = "Text to points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // The discs are drawn over the whole picture.
    roi = FullFrame,
    premultiplied = true,
    // Not seeded, as Grid is not: there are no dice in it.
    seeded = false,
    groups = TEXT_TO_POINTS_GROUPS,
    enabled_when = TEXT_TO_POINTS_ENABLED_WHEN,
)]
pub struct TextToPoints {
    /// The Text layer the points are made from. Unset means the layer this
    /// effect is on. A layer that is not text makes no points.
    #[layer(label = "Text layer")]
    pub text_layer: bool,

    /// What gets a point: each character, each word or each line.
    #[choice(label = "Per", options = ["Character", "Word", "Line"], default = 0)]
    pub per: u32,

    /// A space gets a point too, so the points count every character of the
    /// text. Only when Per is Character. A line break never gets one.
    #[toggle(label = "Include spaces", default = false)]
    pub include_spaces: bool,

    /// Where on its piece a point sits: where the piece starts on the
    /// baseline, or the middle of it.
    #[choice(label = "Place", options = ["Origin", "Centre"], default = 0)]
    pub place: u32,

    /// Which count a point's number carries, for the effects below to read.
    /// Same as Per counts the points themselves. The others count the
    /// character, word or line a point's piece starts with, so with Per on
    /// Character and this on Word every letter of a word carries one number.
    #[choice(
        label = "Number",
        options = ["Same as Per", "Character", "Word", "Line"],
        default = 0
    )]
    pub number: u32,

    /// A point's size is the width of its piece instead of Size: how far a
    /// character moves the pen on, or how wide the word or line is.
    #[toggle(label = "Size from width", default = false)]
    pub size_from_width: bool,

    /// The diameter of the disc a point is drawn as, px@comp.
    #[slider(min = 0.0, max = 200.0, default = 8.0, hard_min = 0.0, unit = Px)]
    pub size: f32,

    /// How soft that disc's edge is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The colour a point is drawn in. Scene-linear, and values above 1 are
    /// useful under a glow.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// The Mix every effect ends with, per cent. At 0 the stream is still
    /// emitted and nothing is drawn.
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

impl TextToPoints {
    /// The raster factor, since the letters and the camera arrive in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// One point per piece of `glyphs`, which are in px@comp, in a raster
    /// `px_scale` times that. `id` is the piece's count along the text, so it
    /// stays put while the text does, and `index` is the count Number asks
    /// for.
    #[must_use]
    pub fn stream(
        self,
        glyphs: &[TextGlyph],
        px_scale: f32,
        projection: Projection,
    ) -> PointsStream {
        let mut out = PointsStream {
            projection,
            px_scale,
            ..PointsStream::default()
        };
        let size = self.size.max(0.0);
        // Premultiplied, as every colour in the working space is.
        let a = self.colour[3];
        let colour = [
            self.colour[0] * a,
            self.colour[1] * a,
            self.colour[2] * a,
            a,
        ];
        // The letters of one word or one line sit next to each other.
        let pieces = glyphs
            .chunk_by(|a, b| match self.per {
                1 => a.word == b.word,
                2 => a.line == b.line,
                _ => false,
            })
            .take(points::CAP_HARD as usize);
        // How many letters came before this piece.
        let mut letters = 0usize;
        for (n, piece) in pieces.enumerate() {
            let (Some(first), Some(last)) = (piece.first(), piece.last()) else {
                continue;
            };
            let character = letters as f32;
            let number = match self.number {
                1 => letters as f32,
                2 => first.word as f32,
                3 => first.line as f32,
                _ => n as f32,
            };
            letters += piece.len();
            // The letters are in px@comp and the Size row is already in the
            // raster, so only the width takes the factor.
            let size = if self.size_from_width {
                (last.end[0] - first.origin[0]).hypot(last.end[1] - first.origin[1]) * px_scale
            } else {
                size
            };
            let at = if self.place == 1 {
                // ponytail: the middle of the piece's box from the font's
                // ascender to its descender, not of its ink. Take the ink's
                // box from the glyph outlines if the difference shows.
                [
                    (first.origin[0] + last.end[0]) * 0.5 + first.up[0],
                    (first.origin[1] + last.end[1]) * 0.5 + first.up[1],
                ]
            } else {
                first.origin
            };
            out.position.push([at[0] * px_scale, at[1] * px_scale, 0.0]);
            out.speed.push([0.0; 3]);
            out.age.push(0.0);
            // 1 rather than 0, so a consumer dividing by it reads a young point.
            out.life.push(1.0);
            out.size.push(size);
            // The way the baseline runs, which is across unless on a path.
            out.rotation.push(first.up[0].atan2(-first.up[1]));
            out.colour.push(colour);
            out.id.push(n as u64);
            out.index.push(number);
            // Every count under a name as well, and the piece's width in
            // px@comp, whatever Number says.
            let width = (last.end[0] - first.origin[0]).hypot(last.end[1] - first.origin[1]);
            let named = [
                ("character", character),
                ("word", first.word as f32),
                ("line", first.line as f32),
                ("width", width),
            ];
            for (name, v) in named {
                if let Some(last) = out.named_mut(name).and_then(|c| c.last_mut()) {
                    *last = v;
                }
            }
        }
        out
    }

    /// How the stream is drawn: a feathered disc per point, and the Mix.
    #[must_use]
    pub fn draw_style(self) -> DrawStyle {
        DrawStyle {
            mode: RenderMode::Disc,
            feather: (self.feather / 100.0).clamp(0.0, 1.0),
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Text to points' behaviour. No CPU reference through the trait, as with
/// the rest of the family: the letters and the camera aren't in the bag.
pub struct TextToPointsDef;

impl EffectDef for TextToPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<TextToPoints as EffectMetadata>::SCHEMA
    }

    /// The picture and the data, as every producer declares it.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: &[],
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(TextToPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
    }
}
