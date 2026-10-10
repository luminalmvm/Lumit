//! Clone to points: a layer's picture stamped at every point of a
//! stream.
//!
//! **In plain terms.** Wire a producer's teal Points socket into this effect,
//! pick a layer, and that layer's picture is stamped once per point — at the
//! point's place, turned by the point's own rotation, sized by the point's own
//! size and tinted by its colour. A hundred snowflakes from Particulate, a
//! lattice of thumbnails from Grid, a logo scattered inside a silhouette: the
//! rig people build by hand out of repeaters and expressions, as one wire.
//!
//! **It is Particulate's Sprite mode, pointed at somebody else's particles.**
//! Not a second implementation of it — literally the same instanced quad, the
//! same bilinear tap, the same premultiplied tint, reached through the shared
//! points draw. What changes is only where the points came from.
//!
//! **Painter's order is `id` order**, or furthest first with Depth sort on.
//! The stream arrives ordered by birth index ascending, which is a fact of the
//! evaluation rather than an artefact of how it was scheduled (particulate.md
//! §5), and the stamps are laid down in that order so a later point covers an
//! earlier one. Depth sort reorders them by how far the camera sees each, and
//! keeps `id` order between points the same distance off. Two renders of one
//! frame therefore lay the same picture down in the same order, on any
//! machine.
//!
//! **Nothing wired draws nothing** — the picture passes through unchanged, and
//! the box wears the family's "no stream" mark. So does an unset Clone layer
//! row: this effect exists to stamp a layer, and with none to stamp the honest
//! answer is the identity, not a fallback shape somebody has to notice and
//! undo.
//!
//! **A stamp need not be a square.** Size says what it is: a square of the
//! point's size, the layer at its own size, or a cell the layer is fitted
//! into or fills. Anchor says which place on it sits on the point, and Corner
//! radius rounds it.
//!
//! **Up to four layers.** With more than one set, each point takes one of
//! them: in turn, by its own dice, or by the number it carries. Or every
//! point takes them all, one on top of the other. The stamps still go down
//! in one order.
//!
//! **A stamp can show another moment.** With Time offset on, the layer is
//! rendered at several moments and each stamp takes one, so one animated
//! layer becomes a cascade. Every moment is another render of the layer.
//!
//! **A stamp can be its own copy.** A Clone index driver on the layer being
//! cloned makes the layer render once per stamp, each reading its own number.
//!
//! **Never more than 32 pictures.** [`CloneToPoints::pictures`] is the rule
//! for what gives way, and [`CloneToPoints::picture_of`] chooses a stamp's
//! picture. The planner, the builder and the draw all go through them.

use crate::fx::drivers::clone_index;
use crate::fx::effects::vary_points::{APPLY_GROUP_WHEN, APPLY_THRESHOLD_WHEN};
use crate::fx::points::{self, PointsStream, SpriteFit};
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup, ParamId, Params,
    Port, PortType, ResolveCx, ShortText, Signature, Value,
};
use crate::model::{Composition, Document, EffectInstance, EffectValue};
use lumit_fx_macros::Effect;
use uuid::Uuid;

/// The dice a Random choice of layer rolls, kept off the numbers the
/// producers and Vary points use for theirs.
const PICTURE_ATTR: u32 = 48;

/// The dice a Random time offset rolls.
const MOMENT_ATTR: u32 = 49;

/// The most pictures one Clone to points asks the render for: layers times
/// moments, or one a clone.
pub const MAX_PICTURES: usize = 32;

/// Choose by's Every layer, by its place in the dropdown.
const CHOOSE_EVERY: u32 = 3;

/// Time offset's options, by their place in the dropdown.
const TIME_OFF: u32 = 0;
const TIME_IN_ORDER: u32 = 1;
const TIME_RANDOM: u32 = 2;
const TIME_NUMBER: u32 = 3;
const TIME_POINT_AGE: u32 = 4;

/// How the list of pictures the render hands this effect is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pictures {
    /// How many layers are in it.
    pub layers: usize,
    /// How many moments each layer is rendered at. 1 with Time offset off.
    pub moments: usize,
    /// How many renders have a clone number of their own. 0 when the copies
    /// are not rendered one per clone.
    pub renders: usize,
}

impl Pictures {
    /// One picture a layer, which is the list with no time offset and no
    /// Clone index.
    #[must_use]
    pub fn plain(layers: usize) -> Self {
        Self {
            layers,
            moments: 1,
            renders: 0,
        }
    }

    /// How many pictures the list holds.
    #[must_use]
    pub fn len(self) -> usize {
        if self.renders > 0 {
            self.renders
        } else {
            self.layers * self.moments
        }
    }

    /// Whether there is nothing to stamp.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// How many renders one point takes with Every layer, where the copies
    /// are rendered one per clone: one a layer, or as many as were made.
    fn every_deep(self) -> usize {
        self.layers.min(self.renders).max(1)
    }
}

/// The pictures one point is stamped with, bottom first: `count` of them,
/// each `step` places on from `first` in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamped {
    pub first: usize,
    pub step: usize,
    pub count: usize,
}

/// One picture in the list: which layer row it is of, which moment, and which
/// clone of how many where the copies are rendered one per clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picture {
    pub layer: usize,
    pub moment: usize,
    pub clone: Option<(u32, u32)>,
}

/// One picture as the render is asked for it: the row and the layer it names,
/// the composition time to render that layer at, and its clone number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Planned {
    pub row: &'static str,
    pub layer: Uuid,
    pub time: f64,
    pub clone: Option<(u32, u32)>,
}

/// The extra layers tucked behind one header, the time rows and the Clone
/// index rows behind two more, and the cell rows that only the two cell sizes
/// read.
pub const CLONE_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "More layers",
        params: &[
            "clone_layer_2",
            "clone_layer_3",
            "clone_layer_4",
            "choose_by",
            "choose_name",
            "seed",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Time",
        params: &["time_offset", "time_name", "time_step", "time_samples"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Clone index",
        params: &["per_clone", "max_renders"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["cell_width", "cell_height"],
        collapsed: false,
        visible_when: Some(("fit", &[2, 3])),
        visible_when_lens_elements: None,
    },
];

/// Time step and Time samples do nothing with Time offset off, and Max
/// renders does nothing with Per clone off. Seed has no rule now: it rolls
/// for a Random choice of layer and for a Random time offset, and a row can
/// only be greyed on one of them.
pub const CLONE_ENABLED_WHEN: &[EnabledWhen] = &[
    APPLY_GROUP_WHEN,
    APPLY_THRESHOLD_WHEN,
    // Each name is read by the Number option of the row above it.
    EnabledWhen {
        param: "choose_name",
        on: "choose_by",
        cond: EnabledCond::ChoiceIs(2),
    },
    EnabledWhen {
        param: "time_name",
        on: "time_offset",
        cond: EnabledCond::ChoiceIs(TIME_NUMBER),
    },
    EnabledWhen {
        param: "time_step",
        on: "time_offset",
        cond: EnabledCond::ChoiceIsNot(TIME_OFF),
    },
    EnabledWhen {
        param: "time_samples",
        on: "time_offset",
        cond: EnabledCond::ChoiceIsNot(TIME_OFF),
    },
    EnabledWhen {
        param: "max_renders",
        on: "per_clone",
        cond: EnabledCond::ChoiceIs(0),
    },
];

/// The wire-only data input (points-stream.md §4.1): no stored value, nothing
/// to keyframe, no panel row. The **first** such input on a stack effect — the
/// port the note said `Signature::Image` would grow to answer for.
pub const POINTS_PORT: &str = "points";

/// What this effect consumes. Not `three_d`: a stamp is a picture laid on the
/// layer's own flat rectangle, turned by one angle, so what it needs of a point
/// is where the camera puts it and how much it foreshortens — which is exactly
/// what a 2D reading answers.
const POINTS_IN: &[Port] = &[Port::new(POINTS_PORT, "Points", PortType::Points)];

/// Clone to points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "clone_to_points",
    label = "Clone to points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point may be anywhere, and a stamp reaches half its own size past it.
    roi = FullFrame,
    premultiplied = true,
    // Seeded, since a time offset shows the clone layer at other moments, so
    // the picture moves under parameters that hold still at this one.
    seeded = true,
    groups = CLONE_GROUPS,
    enabled_when = CLONE_ENABLED_WHEN,
)]
pub struct CloneToPoints {
    /// Which points get a stamp.
    #[choice(
        label = "Apply to",
        options = ["All points", "Picked", "Not picked"],
        default = 0
    )]
    pub apply_to: u32,

    /// The group Picked and Not picked go by: a name a Pick points or a Vary
    /// points above wrote, or one of the `@` names. Empty is the points a
    /// Pick points picked.
    #[text(label = "Group", default = "")]
    pub apply_group: ShortText,

    /// What a point's Group has to read above to be in it. Only read with a
    /// Group named.
    #[slider(label = "Threshold", min = 0.0, max = 1.0, default = 0.5, unit = Raw)]
    pub apply_threshold: f32,

    /// The layer stamped at every point. **Unset draws
    /// nothing**, the ordinary unset-is-identity reading — deliberately unlike
    /// Particulate's Sprite mode, which falls back to discs because a *render
    /// mode* must always draw something. Here there is no mode, only a source.
    #[layer(label = "Clone layer")]
    pub clone_layer: bool,

    /// A second layer to stamp. With more than one set, each point takes one
    /// of them, as Choose by says.
    #[layer(label = "Clone layer 2")]
    pub clone_layer_2: bool,

    /// A third. See [`clone_layer_2`](Self::clone_layer_2).
    #[layer(label = "Clone layer 3")]
    pub clone_layer_3: bool,

    /// A fourth. See [`clone_layer_2`](Self::clone_layer_2).
    #[layer(label = "Clone layer 4")]
    pub clone_layer_4: bool,

    /// How a point picks its layer when more than one is set: each in turn
    /// down the stream, by its own dice, or by the number it carries. Every
    /// layer stamps them all at every point, the first row at the bottom.
    #[choice(
        label = "Choose by",
        options = ["In order", "Random", "Number", "Every layer"],
        default = 0
    )]
    pub choose_by: u32,

    /// Which of a point's numbers Number chooses by: a name written above,
    /// or one of the `@` names. Empty is the number it carries.
    #[text(label = "Name", default = "")]
    pub choose_name: ShortText,

    /// Which dice a Random choice of layer and a Random time offset roll.
    #[seed]
    pub seed: u32,

    /// Shows each stamp its layer at another moment, so one animated layer
    /// becomes a cascade. In order steps back once a stamp, Random shuffles
    /// the same steps by the Seed, Number steps back by the number the point
    /// carries, and Point age plays the layer from its start as the point
    /// ages.
    #[choice(
        label = "Time offset",
        options = ["Off", "In order", "Random", "Number", "Point age"],
        default = 0
    )]
    pub time_offset: u32,

    /// Which of a point's numbers Number steps back by: a name written
    /// above, or one of the `@` names. Empty is the number it carries.
    #[text(label = "Name", default = "")]
    pub time_name: ShortText,

    /// How far apart two steps are, seconds. The default is about two frames.
    #[slider(
        label = "Time step",
        min = 0.0,
        max = 1.0,
        default = 0.067,
        hard_min = 0.0,
        unit = Seconds
    )]
    pub time_step: f32,

    /// The most moments a layer is rendered at. Steps past the last wrap
    /// round to the first, and a point older than the last holds it. Each
    /// moment is another render of the layer, so this is a budget dial.
    #[counter(
        label = "Time samples",
        min = 1,
        max = 32,
        default = 8,
        hard_min = 1,
        hard_max = 32,
        unit = Raw
    )]
    pub time_samples: i32,

    /// Auto renders a layer once per stamp when it carries a Clone index
    /// driver, so each copy reads its own number. Off renders every copy
    /// alike. Point age takes the renders for itself, so the copies are alike
    /// there too.
    #[choice(label = "Per clone", options = ["Auto", "Off"], default = 0)]
    pub per_clone: u32,

    /// The most copies rendered with a number of their own. Stamps past it
    /// share them in turn. Each is another render of the layer.
    #[counter(
        label = "Max renders",
        min = 1,
        max = 32,
        default = 32,
        hard_min = 1,
        hard_max = 32,
        unit = Raw
    )]
    pub max_renders: i32,

    /// What a stamp's size is. Point size is a square of the point's own
    /// size with the whole layer squeezed into it. Layer size is the layer at
    /// its own size and shape. The two cell sizes scale the layer into a cell
    /// keeping its shape: Fit leaves the rest of the cell empty, Fill cuts off
    /// what overhangs. For the last three a point's size is a per cent, so a
    /// point of size 100 is the layer's or the cell's own size.
    #[choice(
        label = "Size",
        options = ["Point size", "Layer size", "Fit cell", "Fill cell"],
        default = 0
    )]
    pub fit: u32,

    /// The cell's width, px@comp.
    #[slider(
        label = "Cell width",
        min = 1.0,
        max = 2000.0,
        default = 200.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub cell_width: f32,

    /// The cell's height, px@comp.
    #[slider(
        label = "Cell height",
        min = 1.0,
        max = 2000.0,
        default = 200.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub cell_height: f32,

    /// Multiplies each point's own size, per cent. At 100 a stamp is a square
    /// of the point's diameter, which is what Particulate's Sprite mode draws.
    #[slider(min = 0.0, max = 1000.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub scale: f32,

    /// Added to each point's own rotation, degrees.
    #[dial(label = "Rotation", default = 0.0)]
    pub rotation: f32,

    /// Which place on the stamp sits on the point, per cent across it. 0 is
    /// its left edge. The stamp turns about this place.
    #[slider(label = "Anchor x", min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub anchor_x: f32,

    /// Per cent down the stamp, 0 its top edge. See
    /// [`anchor_x`](Self::anchor_x).
    #[slider(label = "Anchor y", min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub anchor_y: f32,

    /// Rounds the stamp's corners, px@comp on the stamp of a point whose size
    /// is 100, so it turns and scales with the stamp.
    #[slider(
        label = "Corner radius",
        min = 0.0,
        max = 500.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub corner_radius: f32,

    /// Multiplies every stamp's opacity, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub opacity: f32,

    /// Tint each stamp by its point's colour, per cent (0 leaves the layer's
    /// own colours alone, 100 multiplies them by the point's).
    ///
    /// A dial rather than a switch because the stream's colour usually carries
    /// the *fade* as well as the hue — Particulate's Opacity over life lives in
    /// that alpha — so "all of it" and "none of it" are both wanted and so is
    /// everything between. At 0 a stamp is opaque wherever the layer is.
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub tint: f32,

    /// Lay the stamps down furthest from the camera first, so a near one
    /// covers a far one. Off lays them in the stream's own order. Only shows
    /// on a 3D layer with a camera.
    #[toggle(label = "Depth sort", default = true)]
    pub depth_sort: bool,

    /// **The budget dial**, the family's row: the most stamps that may
    /// be drawn at once. A stream longer than this is trimmed to its **newest**
    /// by birth index — the producer's own cap rule applied a second time, so
    /// what vanishes is what a smaller cap would have taken. Not animatable: it
    /// is a capacity declaration.
    #[counter(
        label = "Max clones",
        min = 1,
        max = 200_000,
        default = 2_000,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub max_clones: i32,

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

impl CloneToPoints {
    /// The raster factor, for the one input the declaration cannot scale: a
    /// stream read off a wire is in px@comp, like a mask path, and has to be
    /// rearranged into the pixels the frame is drawn at.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// How many layer rows are set, which the bag does not otherwise say.
    pub const DERIVED_LAYERS: ParamId = ParamId::new("derived.layers");

    /// Whether the copies are rendered one per clone.
    pub const DERIVED_PER_CLONE: ParamId = ParamId::new("derived.per_clone");

    /// Time step as the builder read it, off its own keyframes.
    pub const DERIVED_TIME_STEP: ParamId = ParamId::new("derived.time_step");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The stream this effect actually stamps: the wired one, with the two
    /// dials and the cap applied.
    ///
    /// **Every decision is here**, in one expression both render paths read, so
    /// the CPU oracle and the instanced draw cannot come to stamp different
    /// squares. `in_stream` is in the units the caller wants out — px@comp for
    /// a reader, raster pixels for a draw ([`PointsStream::rescaled`]).
    ///
    /// `pictures` is how the pictures to choose from are laid out. Which one
    /// each stamp takes is left in the answer's `index` column, as
    /// [`picture_of`](Self::picture_of) chose it.
    #[must_use]
    pub fn stamps(self, in_stream: &PointsStream, pictures: Pictures) -> PointsStream {
        let mut out = in_stream.clone();
        // Chosen before anything is dropped or reordered, so a point keeps
        // its picture when the camera moves or a neighbour is left out.
        out.index = (0..in_stream.len())
            .map(|i| self.picture_of(in_stream, i, pictures).first as f32)
            .collect();
        // How many pictures deep every point is stamped, and how far apart
        // they sit in the list. The same for every point.
        let Stamped { step, count, .. } = self.picture_of(in_stream, 0, pictures);
        if self.apply_to != 0 {
            let group = self.apply_group.as_str();
            out.retain(|i| in_stream.applies(self.apply_to, group, self.apply_threshold, i));
        }
        // The newest by birth index, which is the cap rule the whole family
        // applies — deterministic, and the same from any scrub direction.
        // Max clones counts stamps, so a point stamped several deep counts
        // that many times and keeps all its stamps or none.
        let most = self.max_clones.clamp(0, points::CAP_HARD as i32) as usize;
        out.keep_newest(most / count);
        let scale = (self.scale / 100.0).max(0.0);
        let turn = self.rotation.to_radians();
        let tint = (self.tint / 100.0).clamp(0.0, 1.0);
        let opacity = (self.opacity / 100.0).clamp(0.0, 1.0);
        for s in &mut out.size {
            *s *= scale;
        }
        for r in &mut out.rotation {
            *r += turn;
        }
        for c in &mut out.colour {
            // Towards opaque white, which is the identity of a premultiplied
            // tint: at Tint 0 a stamp is the layer's own picture, at 100 it is
            // the picture times the point's colour and alpha.
            for ch in c.iter_mut() {
                *ch = (1.0 + (*ch - 1.0) * tint) * opacity;
            }
        }
        if self.depth_sort {
            out.sort_far_to_near();
        }
        // Every layer: each point becomes `count` stamps, one after the
        // other, after the sort so the order stays one order. Made with the
        // stream's own append and sort, so no column can be left behind:
        // each copy is numbered by its point's place, and the sort by that
        // number keeps the copies in layer order.
        if count > 1 {
            let places = out.len();
            let ids = std::mem::replace(&mut out.id, (0..places as u64).collect());
            let one = out.clone();
            for j in 1..count {
                let mut next = one.clone();
                for picture in &mut next.index {
                    *picture += (j * step) as f32;
                }
                out.append(&next);
            }
            out.sort_by_id();
            out.id = (0..places * count)
                .filter_map(|k| ids.get(k / count).copied())
                .collect();
        }
        out
    }

    /// The pictures point `i` of the wired stream is stamped with, as places
    /// in the list the render hands this effect.
    ///
    /// The one place a point's pictures are chosen. Choose by picks the
    /// layer, or the clone where the copies are rendered one per clone, and
    /// Time offset picks the moment. Every layer takes them all, bottom
    /// first, each at the point's own moment.
    #[must_use]
    pub fn picture_of(self, s: &PointsStream, i: usize, of: Pictures) -> Stamped {
        let one = |first: usize| Stamped {
            first,
            step: 1,
            count: 1,
        };
        let every = self.choose_by == CHOOSE_EVERY;
        let id = s.id.get(i).copied().unwrap_or(0);
        // Which of `n` the point takes, as Choose by says.
        let chosen = |n: usize| -> usize {
            if n < 2 {
                return 0;
            }
            match self.choose_by {
                // Its own dice, from its id, so the same picture every frame.
                1 => {
                    let roll = points::draw(self.seed, id, PICTURE_ATTR);
                    ((roll * n as f32) as usize).min(n - 1)
                }
                // The number it carries, wrapped round the list. A NaN reads 0.
                2 => {
                    let number = s.value_of(self.choose_name.as_str(), i);
                    (number.floor() as i64).rem_euclid(n as i64) as usize
                }
                _ => i % n,
            }
        };
        if of.renders > 0 {
            if every {
                // The renders come in groups of one a layer, and the points
                // take the groups in turn.
                let deep = of.every_deep();
                return Stamped {
                    first: (i % (of.renders / deep).max(1)) * deep,
                    step: 1,
                    count: deep,
                };
            }
            return one(chosen(of.renders));
        }
        let age = s.age.get(i).copied().unwrap_or(0.0);
        let number = s.value_of(self.time_name.as_str(), i);
        let moment = self.moment_of(i, id, number, age, of.moments);
        if every {
            return Stamped {
                first: moment,
                step: of.moments.max(1),
                count: of.layers.max(1),
            };
        }
        one(chosen(of.layers) * of.moments + moment)
    }

    /// Which of `moments` moments a stamp shows, as Time offset says. `place`
    /// is where it comes in the stream, `number` the number it carries.
    fn moment_of(self, place: usize, id: u64, number: f32, age: f32, moments: usize) -> usize {
        if moments < 2 {
            return 0;
        }
        match self.time_offset {
            // Wrapped round the moments, so the builder need not know how
            // many points there are.
            TIME_IN_ORDER => place % moments,
            TIME_RANDOM => {
                let roll = points::draw(self.seed, id, MOMENT_ATTR);
                ((roll * moments as f32) as usize).min(moments - 1)
            }
            TIME_NUMBER => (number.floor() as i64).rem_euclid(moments as i64) as usize,
            // A point older than every moment holds the last. A NaN reads 0.
            TIME_POINT_AGE if self.time_step > 0.0 => {
                ((age.max(0.0) / self.time_step) as usize).min(moments - 1)
            }
            _ => 0,
        }
    }

    /// The pictures the render makes for this effect. **The cap rule**, in
    /// one place: never more than [`MAX_PICTURES`].
    ///
    /// `layers` is how many layer rows are set, `per_clone` what
    /// [`per_clone_in`](Self::per_clone_in) answered, and `points` how many
    /// points the wire brings where that is known before the render.
    ///
    /// Layers come first. Every set layer keeps a picture and Time samples
    /// gives way, so four layers get eight moments each. Rendered one per
    /// clone there is a render a point up to Max renders, each at its own
    /// clone's moment, and layers past the number of renders are not shown.
    #[must_use]
    pub fn pictures(self, layers: usize, per_clone: bool, points: Option<usize>) -> Pictures {
        let layers = layers.min(MAX_PICTURES);
        if layers == 0 {
            return Pictures::plain(0);
        }
        let most = MAX_PICTURES as i32;
        let samples = match self.time_offset {
            TIME_OFF => 1,
            _ => self.time_samples.clamp(1, most) as usize,
        };
        if per_clone {
            let cap = self.max_renders.clamp(1, most) as usize;
            // Every layer wants a render for each layer of each point.
            let each = match self.choose_by {
                CHOOSE_EVERY => layers,
                _ => 1,
            };
            // A stream that cannot be counted yet gets the whole cap.
            let mut renders = points.map_or(cap, |n| n.saturating_mul(each).clamp(1, cap));
            // Whole points only, so no render is made that no stamp shows.
            if each > 1 && renders > each {
                renders -= renders % each;
            }
            Pictures {
                layers,
                moments: samples,
                renders,
            }
        } else {
            Pictures {
                layers,
                moments: samples.min(MAX_PICTURES / layers).max(1),
                renders: 0,
            }
        }
    }

    /// What picture `k` of the list is.
    #[must_use]
    pub fn picture(self, of: Pictures, k: usize) -> Picture {
        if of.renders == 0 {
            let moments = of.moments.max(1);
            return Picture {
                layer: k / moments,
                moment: k % moments,
                clone: None,
            };
        }
        // A clone's moment is its own number's, so each has both. Every
        // layer stamps one point with a whole group of renders, and they
        // share that point's moment.
        let place = match self.choose_by {
            CHOOSE_EVERY => k / of.every_deep(),
            _ => k,
        };
        Picture {
            layer: k % of.layers.max(1),
            moment: self.moment_of(place, place as u64, place as f32, 0.0, of.moments),
            clone: Some((k as u32, of.renders as u32)),
        }
    }

    /// The composition time a layer is rendered at for moment `m`. `t` is the
    /// frame's own time and `start` the layer's in point.
    #[must_use]
    pub fn moment_time(self, m: usize, t: f64, start: f64) -> f64 {
        let step = m as f64 * f64::from(self.time_step.max(0.0));
        match self.time_offset {
            TIME_POINT_AGE => start + step,
            // The frame's own time to the bit, which needs no second render.
            _ if m == 0 => t,
            _ => t - step,
        }
    }

    /// The rows that decide which pictures are rendered, read off the stored
    /// instance at layer time `lt`. Every other row is at its default.
    ///
    /// The planner, the builder and the draw all read them through here, so
    /// they agree. A wire into Time step, Time samples or Max renders is not
    /// followed: they keep to their own keyframes, as Trail's Samples does.
    #[must_use]
    pub fn stored(e: &EffectInstance, lt: f64) -> Self {
        let mut c = Self::read(Params::EMPTY);
        let choice = |id: &str, or: u32| match e.param(id) {
            Some(EffectValue::Choice(v)) => *v,
            _ => or,
        };
        let number = |id: &str| e.float_at(id, lt).filter(|v| v.is_finite());
        c.choose_by = choice("choose_by", c.choose_by);
        c.time_offset = choice("time_offset", c.time_offset);
        c.per_clone = choice("per_clone", c.per_clone);
        if let Some(EffectValue::Seed(seed)) = e.param("seed") {
            c.seed = *seed;
        }
        c.time_step = number("time_step").map_or(c.time_step, |v| v as f32);
        c.time_samples = number("time_samples").map_or(c.time_samples, |v| v as i32);
        c.max_renders = number("max_renders").map_or(c.max_renders, |v| v as i32);
        c
    }

    /// The clone layer rows that name a layer, in row order, each with the
    /// layer it names.
    #[must_use]
    pub fn rows(e: &EffectInstance) -> Vec<(&'static str, Uuid)> {
        let schema: &'static EffectSchema = &<Self as EffectMetadata>::SCHEMA;
        schema
            .layer_inputs()
            .filter_map(|row| Some((row, e.layer_ref(row)?)))
            .collect()
    }

    /// Whether the copies are rendered one per clone: Per clone is on Auto
    /// and a Clone index driver is somewhere a render of a layer being cloned
    /// reads it, inside a precomp or a node graph included
    /// ([`clone_index::reaches`]).
    ///
    /// Point age needs every stamp free to take any moment, so it wins and
    /// the copies are rendered alike.
    #[must_use]
    pub fn per_clone_in(self, e: &EffectInstance, doc: &Document, comp: &Composition) -> bool {
        self.per_clone == 0
            && self.time_offset != TIME_POINT_AGE
            && Self::rows(e).iter().any(|(_, id)| {
                comp.layers
                    .iter()
                    .any(|l| l.id == *id && clone_index::reaches(doc, l))
            })
    }

    /// Every picture this effect asks the render for at composition time `t`,
    /// in list order. `None` is a hole: a row naming a layer that is gone.
    ///
    /// The planner and the builder both ask here, so the footage one fetches
    /// is the footage the other draws. `lt` is the time of the layer the
    /// effect is on. `points` is as [`pictures`](Self::pictures) takes it,
    /// and the planner, which has no stream, passes `None` and so fetches for
    /// every render there could be.
    #[must_use]
    pub fn planned(
        e: &EffectInstance,
        doc: &Document,
        comp: &Composition,
        t: f64,
        lt: f64,
        points: Option<usize>,
    ) -> Vec<Option<Planned>> {
        let stored = Self::stored(e, lt);
        let rows = Self::rows(e);
        let of = stored.pictures(rows.len(), stored.per_clone_in(e, doc, comp), points);
        (0..of.len())
            .map(|k| {
                let picture = stored.picture(of, k);
                let (row, layer) = *rows.get(picture.layer)?;
                let start = comp.layers.iter().find(|l| l.id == layer)?.in_point;
                Some(Planned {
                    row,
                    layer,
                    time: stored.moment_time(picture.moment, t, start.0.to_f64()),
                    clone: picture.clone,
                })
            })
            .collect()
    }

    /// How the list the render handed over is laid out, read back from the
    /// resolved bag and the list's own length. What the builder decided is
    /// what the draw reads, so a stamp's picture is the one made for it.
    #[must_use]
    pub fn handed(p: Params<'_>, handed: usize) -> Pictures {
        // A bag nobody resolved has no layer count, and the list is then one
        // picture a layer.
        let layers = usize::try_from(p.int(Self::DERIVED_LAYERS, handed as i32)).unwrap_or(0);
        if layers == 0 || handed == 0 {
            return Pictures::plain(0);
        }
        if p.bool(Self::DERIVED_PER_CLONE, false) {
            Pictures {
                layers,
                moments: 1,
                renders: handed,
            }
        } else {
            Pictures {
                layers,
                moments: (handed / layers).max(1),
                renders: 0,
            }
        }
    }

    /// How a stamp of a picture `picture` px@comp across and down sits on its
    /// point: the Size, Anchor and Corner radius rows as the draw reads them.
    ///
    /// `px_scale` is the raster factor the bag's px rows were scaled by. A
    /// stamp's shape is worked out at composition size, since a point's size
    /// carries the raster factor already.
    #[must_use]
    pub fn fit(self, picture: [f32; 2], px_scale: f32) -> SpriteFit {
        let k = px_scale.max(1e-6);
        let layer = [picture[0].max(1.0), picture[1].max(1.0)];
        let cell = [
            (self.cell_width / k).max(1e-3),
            (self.cell_height / k).max(1e-3),
        ];
        // How many times the layer's width and height the cell is.
        let (across, down) = (cell[0] / layer[0], cell[1] / layer[1]);
        // The stamp's own size at a point size of 100, and how much of it the
        // picture spans.
        let (own, uv) = match self.fit {
            1 => (layer, [1.0; 2]),
            // One scale for both axes: the smaller fits the layer inside the
            // cell, the larger covers it.
            2 => (cell, [across / across.min(down), down / across.min(down)]),
            3 => (cell, [across / across.max(down), down / across.max(down)]),
            _ => ([100.0; 2], [1.0; 2]),
        };
        SpriteFit {
            unit: [own[0] / 100.0, own[1] / 100.0],
            anchor: [self.anchor_x / 100.0, self.anchor_y / 100.0],
            uv,
            corner: (self.corner_radius / k).max(0.0),
        }
    }

    /// How the stamps are drawn — Sprite mode, and the host Mix.
    #[must_use]
    pub fn draw_style(self) -> points::DrawStyle {
        points::DrawStyle {
            mode: points::RenderMode::Sprite,
            feather: 0.0,
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Clone to points' behaviour.
///
/// **No CPU reference through the trait**, for the reason every points effect
/// has one fewer than it looks: what it draws is a stream and a camera, neither
/// of which is a number in the bag [`apply_cpu`](EffectDef::apply_cpu) is
/// handed. Both ride the carriage beside the op. The §1.6 oracle is
/// [`CloneToPoints::stamps`] with [`points::draw_stream`], exercised directly
/// from the test suite — and it is the very stream the GPU draw is handed.
pub struct CloneToPointsDef;

impl EffectDef for CloneToPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<CloneToPoints as EffectMetadata>::SCHEMA
    }

    /// A picture in, a picture out, and a **stream in** beside it — the first
    /// stack effect to declare a data input.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: &[],
        }
    }

    /// The raster factor, so a px@comp stream reaches the pixels this frame is
    /// drawn at.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(CloneToPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
        // What the draw needs to read its list the way the builder laid it
        // out, from the same code the builder asked.
        let stored = CloneToPoints::stored(cx.inst, cx.lt);
        let doc = &cx.context.document;
        let per_clone = cx
            .context
            .comp
            .and_then(|id| doc.comp(id))
            .is_some_and(|comp| stored.per_clone_in(cx.inst, doc, comp));
        let layers = CloneToPoints::rows(cx.inst).len();
        push(CloneToPoints::DERIVED_LAYERS, Value::Int(layers as i32));
        push(CloneToPoints::DERIVED_PER_CLONE, Value::Bool(per_clone));
        push(
            CloneToPoints::DERIVED_TIME_STEP,
            Value::Float(stored.time_step),
        );
    }
}
