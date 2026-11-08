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

use crate::expression::ExpressionContext;
use crate::fx::drivers::{clone_index, per_copy};
use crate::fx::effects::vary_points::{APPLY_GROUP_WHEN, APPLY_THRESHOLD_WHEN};
use crate::fx::points::{self, Column, PointsStream, SpriteFit};
use crate::fx::{
    params::ParamId, params::Params, params::ShortText, params::Value, registry::EffectDef,
    registry::EffectMetadata, registry::ResolveCx, registry::Signature, schema::EffectSchema,
    schema::EnabledCond, schema::EnabledWhen, schema::ParamGroup, schema::Port, schema::PortType,
};
use crate::model::{Composition, Document, EffectInstance, EffectValue, Layer};
use lumit_fx_macros::Effect;
use std::sync::Arc;
use uuid::Uuid;

mod stamps;

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

/// Choose by's Layer index, which reads the Layer index row (or the copy's
/// own value, where a driver is wired into it).
const CHOOSE_INDEX: u32 = 4;

/// Distribution's options, by their place in the dropdown.
pub const DIST_INPUT: u32 = 0;
pub const DIST_GRID: u32 = 1;
pub const DIST_PATH: u32 = 2;
pub const DIST_LAYOUT: u32 = 3;

/// The layer row Path reads its path from, which is not a layer to stamp.
pub const PATH_LAYER: &str = "path_layer";

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
    // Each Distribution's own rows, shown only under it.
    ParamGroup {
        label: "",
        params: &["dist_count"],
        collapsed: false,
        visible_when: Some(("distribution", &[DIST_PATH, DIST_LAYOUT])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &[
            "dist_columns",
            "dist_rows",
            "dist_spacing_x",
            "dist_spacing_y",
        ],
        collapsed: false,
        visible_when: Some(("distribution", &[DIST_GRID])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &[
            "path_layer",
            "dist_align",
            "dist_length",
            "dist_travel",
            "dist_offset",
        ],
        collapsed: false,
        visible_when: Some(("distribution", &[DIST_PATH])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["dist_direction", "dist_spacing"],
        collapsed: false,
        visible_when: Some(("distribution", &[DIST_LAYOUT])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["dist_position_x", "dist_position_y"],
        collapsed: false,
        visible_when: Some(("distribution", &[DIST_GRID, DIST_LAYOUT])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "More layers",
        params: &[
            "clone_layer_2",
            "clone_layer_3",
            "clone_layer_4",
            "clone_layer_5",
            "clone_layer_6",
            "clone_layer_7",
            "clone_layer_8",
            "choose_by",
            "choose_name",
            "layer_index",
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
    category = Points,
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
    /// Where the copies go. Points input stamps the wired stream. Grid, Path
    /// and Layout make the points here, so no producer has to be wired:
    /// Grid in rows and columns, Path spread along a layer's path, Layout in
    /// a line where each copy starts where the one before ends.
    #[choice(
        label = "Distribution",
        options = ["Points input", "Grid", "Path", "Layout"],
        default = 0
    )]
    pub distribution: u32,

    /// How many copies Path and Layout make.
    #[counter(
        label = "Count",
        min = 1,
        max = 200,
        default = 10,
        hard_min = 1,
        hard_max = points::CAP_HARD,
        unit = Raw
    )]
    pub dist_count: i32,

    /// Grid's columns.
    #[counter(
        label = "Columns",
        min = 1,
        max = 100,
        default = 5,
        hard_min = 1,
        hard_max = 10_000,
        unit = Raw
    )]
    pub dist_columns: i32,

    /// Grid's rows.
    #[counter(
        label = "Rows",
        min = 1,
        max = 100,
        default = 5,
        hard_min = 1,
        hard_max = 10_000,
        unit = Raw
    )]
    pub dist_rows: i32,

    /// The gap between Grid's columns, px@comp.
    #[slider(
        label = "Spacing x",
        min = 0.0,
        max = 1000.0,
        default = 120.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub dist_spacing_x: f32,

    /// The gap between Grid's rows, px@comp.
    #[slider(
        label = "Spacing y",
        min = 0.0,
        max = 1000.0,
        default = 120.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub dist_spacing_y: f32,

    /// The layer whose path Path follows: a Shape layer's outlines, or
    /// another layer's first mask. Unset is this layer's own.
    #[layer(label = "Path layer")]
    pub path_layer: bool,

    /// Turns each copy to face the way the path runs.
    #[toggle(label = "Align to path", default = true)]
    pub dist_align: bool,

    /// How much of the path the copies spread over, per cent of its length.
    #[slider(label = "Length", min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub dist_length: f32,

    /// Slides the copies along the path, per cent of its length. Past an end
    /// they come back in at the other.
    #[slider(label = "Travel", min = -100.0, max = 100.0, default = 0.0, unit = Percent)]
    pub dist_travel: f32,

    /// Moves each copy off the path, along the normal where it stands,
    /// px@comp: to the left of the way the path runs, and the other way
    /// below nought. Round a closed path it rings the copies closer in or
    /// further out, as Cavalry's Offset does.
    #[slider(label = "Offset", min = -500.0, max = 500.0, default = 0.0, unit = Px)]
    pub dist_offset: f32,

    /// Which way Layout lines the copies up.
    #[choice(label = "Direction", options = ["Horizontal", "Vertical"], default = 0)]
    pub dist_direction: u32,

    /// The gap Layout leaves between one copy and the next, px@comp.
    /// Below nought they overlap.
    #[slider(
        label = "Spacing",
        min = -500.0,
        max = 1000.0,
        default = 20.0,
        unit = Px
    )]
    pub dist_spacing: f32,

    /// The centre of Grid and Layout, px@comp.
    #[slider(label = "Position x", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub dist_position_x: f32,

    /// px@comp; see [`dist_position_x`](Self::dist_position_x).
    #[slider(label = "Position y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub dist_position_y: f32,

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

    /// A fifth. See [`clone_layer_2`](Self::clone_layer_2).
    #[layer(label = "Clone layer 5")]
    pub clone_layer_5: bool,

    /// A sixth. See [`clone_layer_2`](Self::clone_layer_2).
    #[layer(label = "Clone layer 6")]
    pub clone_layer_6: bool,

    /// A seventh. See [`clone_layer_2`](Self::clone_layer_2).
    #[layer(label = "Clone layer 7")]
    pub clone_layer_7: bool,

    /// An eighth. See [`clone_layer_2`](Self::clone_layer_2).
    #[layer(label = "Clone layer 8")]
    pub clone_layer_8: bool,

    /// How a point picks its layer when more than one is set: each in turn
    /// down the stream, by its own dice, by the number it carries, or by Layer
    /// index. Every layer stamps them all at every point, the first row at
    /// the bottom.
    #[choice(
        label = "Choose by",
        options = ["In order", "Random", "Number", "Every layer", "Layer index"],
        default = 0
    )]
    pub choose_by: u32,

    /// Which of a point's numbers Number chooses by: a name written above,
    /// or one of the `@` names. Empty is the number it carries.
    #[text(label = "Name", default = "")]
    pub choose_name: ShortText,

    /// Which layer every point takes when Choose by is Layer index, counted
    /// from 0 and wrapped round the layers that are set. Wire a Random, a
    /// Stagger or Clone index in to pick a layer for each copy.
    #[slider(label = "Layer index", min = 0.0, max = 7.0, default = 0.0, unit = Raw)]
    pub layer_index: f32,

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
    /// Below nought each step is that much later rather than earlier, as
    /// Cavalry's Stagger runs the copies ahead; Point age reads it as nought.
    #[slider(
        label = "Time step",
        min = 0.0,
        max = 1.0,
        default = 0.067,
        hard_min = -10.0,
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

    /// Multiplies each stamp's width, per cent, on top of Scale and the
    /// point's own stretch. Wire a Stagger in to give every copy its own.
    #[slider(
        label = "Scale x",
        min = 0.0,
        max = 1000.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub scale_x: f32,

    /// Multiplies each stamp's height, per cent. See
    /// [`scale_x`](Self::scale_x).
    #[slider(
        label = "Scale y",
        min = 0.0,
        max = 1000.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub scale_y: f32,

    /// Added to each point's own rotation, degrees.
    #[dial(label = "Rotation", default = 0.0)]
    pub rotation: f32,

    /// Moves each stamp across from its point, px@comp. Wire a driver in to
    /// move every copy by its own amount.
    #[slider(label = "Offset x", min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub offset_x: f32,

    /// Moves each stamp down from its point, px@comp.
    #[slider(label = "Offset y", min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub offset_y: f32,

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

    /// How the stamps combine with each other where they overlap, by the
    /// layer blend modes verbatim, as Particulate's Particle blend. They then
    /// go over the picture together. An index into
    /// [`BlendMode::ALL`](crate::model::BlendMode::ALL).
    #[choice(
        label = "Copy blend",
        options = *crate::model::BlendMode::NAMES,
        default = 0,
        dividers_after = crate::fx::builtins::CHOICE_UNGROUPED
    )]
    pub copy_blend: u32,

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

    /// Which dice a Random choice of layer and a Random time offset roll.
    #[seed]
    pub seed: u32,

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

/// Points along path's End for `count` copies over `length` per cent of a
/// path. Round a closed path the copies are a Length / Count apart however
/// much of it is used, as Cavalry spaces them, where Points along path counts
/// one gap fewer short of the whole path.
#[must_use]
pub fn spread_end(length: f32, count: i32, closed: bool) -> f32 {
    if closed && length < 100.0 && count > 0 {
        length * (count - 1) as f32 / count as f32
    } else {
        length
    }
}

/// The way `path` runs `s` px along it, eased from the way it runs at one
/// vertex to the way it runs at the next. At a vertex where the flattening
/// bent the path by less than 30° it runs between its two chords, so a copy
/// on a curve leans as the curve does, not as the chord it sits on; at a
/// sharper corner each edge keeps its own way.
#[must_use]
pub fn smooth_tangent(path: &crate::mask::MaskPolyline, s: f32) -> [f32; 2] {
    let n = path.points.len();
    if n < 2 {
        return path.tangent_at(s);
    }
    let s = s.clamp(0.0, path.length());
    let e = path
        .arc
        .partition_point(|&a| a <= s)
        .saturating_sub(1)
        .min(n - 2);
    let span = path.arc[e + 1] - path.arc[e];
    let u = if span > 0.0 {
        ((s - path.arc[e]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let own = path.edge_tangent(e);
    // The edge on the far side of vertex `v` from edge `e`, round the join
    // of a closed path.
    let beyond = |v: usize| -> Option<[f32; 2]> {
        let other = if v == e {
            e.checked_sub(1).or(path.closed.then_some(n - 2))?
        } else if e + 2 < n {
            e + 1
        } else {
            path.closed.then_some(0)?
        };
        Some(path.edge_tangent(other))
    };
    let at = |v: usize| match beyond(v) {
        Some(t) if t[0] * own[0] + t[1] * own[1] > 30f32.to_radians().cos() => {
            [t[0] + own[0], t[1] + own[1]]
        }
        _ => own,
    };
    let (a, b) = (at(e), at(e + 1));
    let [x, y] = [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u];
    let l = x.hypot(y);
    if l > 0.0 {
        [x / l, y / l]
    } else {
        own
    }
}

/// Offset: every point `px` off `path`, along the normal where it stands, to
/// the left of the way the path runs, as Points along path's Deform moves one.
/// A point that faces the way the path runs faces the eased way of
/// [`smooth_tangent`] instead of its chord's.
pub fn push_off_path(
    stream: &mut PointsStream,
    path: &crate::mask::MaskPolyline,
    px: f32,
    align: bool,
) {
    let length = path.length();
    for ((p, r), far) in stream
        .position
        .iter_mut()
        .zip(&mut stream.rotation)
        .zip(&stream.index)
    {
        let [x, y] = smooth_tangent(path, far * length);
        if align {
            *r = y.atan2(x);
        }
        p[0] += y * px;
        p[1] -= x * px;
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
