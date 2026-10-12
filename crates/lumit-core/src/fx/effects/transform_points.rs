//! Transform points moves, scales and turns a whole stream about an anchor.
//!
//! Every point is scaled away from the anchor, turned round it, then placed.
//! Place says how: moved by Position, moved so the anchor lands on a share
//! of the composition, or pinned so the anchor lands Position away from one
//! of the composition's corners, edges or its centre. The anchor is a place
//! in the frame, in px or as a share of the composition, or a place on the
//! box round the points. The points' own sizes and rotations stay as they
//! came unless the two switches say otherwise. The changed stream goes out
//! on the Points socket, and is drawn as discs unless Mix is 0. Nothing
//! wired draws nothing.

use crate::fx::effects::vary_points::{
    APPLY_GROUP_WHEN, APPLY_THRESHOLD_WHEN, POINTS_IN, POINTS_OUT,
};
use crate::fx::points::PointsStream;
use crate::fx::{
    EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen, ParamGroup, ParamId, Params,
    ResolveCx, ShortText, Signature, Value,
};
use lumit_fx_macros::Effect;

/// The Place options, in code order. A Choice is stored as its index, so a
/// new one goes on the end.
const SHARE: u32 = 1;
const PINNED: u32 = 2;

const fn group(
    label: &'static str,
    params: &'static [&'static str],
    visible_when: Option<(&'static str, &'static [u32])>,
) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when,
        visible_when_lens_elements: None,
    }
}

/// The anchor, where the stream is placed, then what that does to each
/// point. Each way of placing shows only the rows it reads.
pub const TRANSFORM_POINTS_GROUPS: &[ParamGroup] = &[
    group("Anchor", &["anchor_to_points", "anchor_in"], None),
    group("", &["anchor_x", "anchor_y"], Some(("anchor_in", &[0]))),
    group(
        "",
        &["anchor_frame_x", "anchor_frame_y"],
        Some(("anchor_in", &[1])),
    ),
    group("Transform", &["place"], None),
    group("", &["place_x", "place_y"], Some(("place", &[SHARE]))),
    group("", &["pin_to"], Some(("place", &[PINNED]))),
    group(
        "",
        &["position_x", "position_y"],
        Some(("place", &[0, PINNED])),
    ),
    group("Point", &["scale_sizes", "turn_points", "feather"], None),
];

const fn while_anchor_to_points(param: &'static str, on: bool) -> EnabledWhen {
    EnabledWhen {
        param,
        on: "anchor_to_points",
        cond: EnabledCond::BoolIs(on),
    }
}

/// The rows that place the anchor in the frame do nothing while it follows
/// the points, and the Anchor on points pair nothing until it does.
pub const TRANSFORM_POINTS_ENABLED_WHEN: &[EnabledWhen] = &[
    APPLY_GROUP_WHEN,
    APPLY_THRESHOLD_WHEN,
    while_anchor_to_points("anchor_in", false),
    while_anchor_to_points("anchor_x", false),
    while_anchor_to_points("anchor_y", false),
    while_anchor_to_points("anchor_frame_x", false),
    while_anchor_to_points("anchor_frame_y", false),
    while_anchor_to_points("anchor_points_x", true),
    while_anchor_to_points("anchor_points_y", true),
];

/// Transform points' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "transform_points",
    label = "Transform points",
    version = 1,
    category = Generate,
    cost = Moderate,
    // A point may be moved anywhere.
    roi = FullFrame,
    premultiplied = true,
    groups = TRANSFORM_POINTS_GROUPS,
    enabled_when = TRANSFORM_POINTS_ENABLED_WHEN,
)]
pub struct TransformPoints {
    /// Which points are moved. The rest pass through as they came.
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

    /// The anchor is a place on the box round the points being moved, its
    /// middle unless the Anchor on points pair says otherwise, so a stream
    /// is scaled and turned about itself wherever it is.
    #[toggle(label = "Anchor to points", default = false)]
    pub anchor_to_points: bool,

    /// How an anchor in the frame is given: in px, or as a share of the
    /// composition's size, which stays put when the composition is resized.
    #[choice(label = "Anchor in", options = ["Pixels", "Share of frame"], default = 0)]
    pub anchor_in: u32,

    /// The place the stream is scaled from and turned round, px@comp.
    #[slider(min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub anchor_x: f32,

    /// px@comp. See [`anchor_x`](Self::anchor_x).
    #[slider(min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub anchor_y: f32,

    /// How far across the composition the anchor sits, per cent. 0 is its
    /// left edge and 100 its right.
    #[slider(
        label = "Anchor x",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        unit = Percent
    )]
    pub anchor_frame_x: f32,

    /// Per cent, down, from the top edge. See
    /// [`anchor_frame_x`](Self::anchor_frame_x).
    #[slider(
        label = "Anchor y",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        unit = Percent
    )]
    pub anchor_frame_y: f32,

    /// Where across the box round the points the anchor sits while Anchor to
    /// points is on, per cent. 0 is its left edge and 100 its right.
    #[slider(
        label = "Anchor on points x",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        unit = Percent
    )]
    pub anchor_points_x: f32,

    /// Per cent, down, from the top edge. See
    /// [`anchor_points_x`](Self::anchor_points_x).
    #[slider(
        label = "Anchor on points y",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        unit = Percent
    )]
    pub anchor_points_y: f32,

    /// How the stream is placed. Offset moves it by Position from where it
    /// came. Share of frame moves it so the anchor lands on a share of the
    /// composition. Pinned moves it so the anchor lands Position away from
    /// the corner, edge or centre of the composition that Pin to names.
    #[choice(
        label = "Place",
        options = ["Offset", "Share of frame", "Pinned"],
        default = 0
    )]
    pub place: u32,

    /// How far across the composition the anchor lands, per cent. 0 is its
    /// left edge and 100 its right.
    #[slider(
        label = "Place x",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        unit = Percent
    )]
    pub place_x: f32,

    /// Per cent, down, from the top edge. See [`place_x`](Self::place_x).
    #[slider(
        label = "Place y",
        min = 0.0,
        max = 100.0,
        default = 50.0,
        unit = Percent
    )]
    pub place_y: f32,

    /// The place in the composition that Position is measured from when the
    /// stream is pinned. Bottom right with an anchor at the points' own
    /// bottom right and a Position of -32 and -32 sits them 32 px in from
    /// that corner.
    #[choice(
        label = "Pin to",
        options = [
            "Top left",
            "Top",
            "Top right",
            "Left",
            "Centre",
            "Right",
            "Bottom left",
            "Bottom",
            "Bottom right",
        ],
        default = 4
    )]
    pub pin_to: u32,

    /// How far the whole stream is moved across, px@comp. Pinned, how far
    /// across from the place Pin to names the anchor lands.
    #[slider(min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub position_x: f32,

    /// px@comp, down. See [`position_x`](Self::position_x).
    #[slider(min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub position_y: f32,

    /// How far the whole stream is moved through the layer's plane, px@comp,
    /// however it is placed.
    #[slider(min = -2000.0, max = 2000.0, default = 0.0, unit = Px)]
    pub position_z: f32,

    /// How far the stream is spread across from the anchor, per cent. A
    /// negative number flips it.
    #[slider(label = "Scale x %", min = 0.0, max = 400.0, default = 100.0, unit = Percent)]
    pub scale_x: f32,

    /// Per cent, down. See [`scale_x`](Self::scale_x).
    #[slider(label = "Scale y %", min = 0.0, max = 400.0, default = 100.0, unit = Percent)]
    pub scale_y: f32,

    /// How far the stream is turned round the anchor, degrees.
    #[dial(default = 0.0, step = 15.0)]
    pub rotation: f32,

    /// The points grow and shrink with the stream.
    #[toggle(label = "Scale sizes", default = false)]
    pub scale_sizes: bool,

    /// The points turn with the stream.
    #[toggle(label = "Turn points", default = false)]
    pub turn_points: bool,

    /// How soft the disc a point is drawn as is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub feather: f32,

    /// The Mix every effect ends with, per cent. At 0 the stream is still
    /// handed on and nothing is drawn.
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

impl TransformPoints {
    /// The raster factor, since a stream off a wire arrives in px@comp.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// The composition's width and height in the units the bag is in, which
    /// the shares and the pins are measured on. 0 where no composition is in
    /// play.
    pub const DERIVED_FRAME: [ParamId; 2] = [
        ParamId::new("derived.frame_w"),
        ParamId::new("derived.frame_h"),
    ];

    /// The wired stream moved. `in_stream`, `frame` and the bag must be in
    /// the same units, and the answer is in those units too. `frame` is the
    /// composition's width and height.
    #[must_use]
    pub fn apply(self, in_stream: &PointsStream, frame: [f32; 2]) -> PointsStream {
        let mut out = in_stream.clone();
        let (sx, sy) = (self.scale_x / 100.0, self.scale_y / 100.0);
        let turn = self.rotation.to_radians();
        let (sin, cos) = turn.sin_cos();
        // Scaled first, then turned, so Scale x is always across the stream
        // as it came.
        let turned = |x: f32, y: f32| {
            let (x, y) = (x * sx, y * sy);
            [x * cos - y * sin, x * sin + y * cos]
        };
        // A point is round, so an uneven scale grows it by the middle of the
        // two.
        let grow = (sx * sy).abs().sqrt();
        let group = in_stream.group(self.apply_group.as_str());
        let moved = |i: usize| in_stream.applies(self.apply_to, group, self.apply_threshold, i);
        let share = |of: f32, per_cent: f32| of * per_cent / 100.0;
        let mut anchor = if self.anchor_in == 1 {
            [
                share(frame[0], self.anchor_frame_x),
                share(frame[1], self.anchor_frame_y),
            ]
        } else {
            [self.anchor_x, self.anchor_y]
        };
        if self.anchor_to_points {
            let (mut low, mut high) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
            for (_, p) in in_stream
                .position
                .iter()
                .enumerate()
                .filter(|(i, _)| moved(*i))
            {
                low = [low[0].min(p[0]), low[1].min(p[1])];
                high = [high[0].max(p[0]), high[1].max(p[1])];
            }
            // No points to measure leaves the anchor where the rows put it.
            if low[0] <= high[0] && low[1] <= high[1] {
                anchor = [
                    low[0] + share(high[0] - low[0], self.anchor_points_x),
                    low[1] + share(high[1] - low[1], self.anchor_points_y),
                ];
            }
        }
        // How far everything is moved once it is scaled and turned. A share
        // or a pin takes the anchor to its place and not just some way off.
        let shift = match self.place {
            SHARE => [
                share(frame[0], self.place_x) - anchor[0],
                share(frame[1], self.place_y) - anchor[1],
            ],
            PINNED => {
                // Three across and three down: none, half or all of the way.
                let pin = self.pin_to.min(8);
                let (across, down) = ((pin % 3) as f32 * 0.5, (pin / 3) as f32 * 0.5);
                [
                    frame[0] * across + self.position_x - anchor[0],
                    frame[1] * down + self.position_y - anchor[1],
                ]
            }
            _ => [self.position_x, self.position_y],
        };
        for i in (0..out.len()).filter(|i| moved(*i)) {
            if let Some(p) = out.position.get_mut(i) {
                let [x, y] = turned(p[0] - anchor[0], p[1] - anchor[1]);
                p[0] = anchor[0] + x + shift[0];
                p[1] = anchor[1] + y + shift[1];
                p[2] += self.position_z;
            }
            // The way it is travelling turns with it.
            if let Some(s) = out.speed.get_mut(i) {
                let [x, y] = turned(s[0], s[1]);
                (s[0], s[1]) = (x, y);
            }
            if self.scale_sizes {
                if let Some(s) = out.size.get_mut(i) {
                    *s *= grow;
                }
            }
            if self.turn_points {
                if let Some(r) = out.rotation.get_mut(i) {
                    *r += turn;
                }
            }
        }
        out
    }
}

/// Transform points' behaviour, the same shape as Vary points'.
pub struct TransformPointsDef;

impl EffectDef for TransformPointsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<TransformPoints as EffectMetadata>::SCHEMA
    }

    /// A stream in and a stream out, beside the picture.
    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: POINTS_OUT,
        }
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(TransformPoints::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
        // ponytail: the composition's size, not the layer's own, so a share
        // or a pin names a place in the frame only on a layer that fills it
        // and has not been moved. Hand the layer's size in here if that ever
        // matters.
        let context = &cx.context;
        let comp = context.comp.and_then(|id| context.document.comp(id));
        let [w, h] = comp.map_or([0.0; 2], |c| [c.width as f32, c.height as f32]);
        push(
            TransformPoints::DERIVED_FRAME[0],
            Value::Float(w * cx.px_scale),
        );
        push(
            TransformPoints::DERIVED_FRAME[1],
            Value::Float(h * cx.px_scale),
        );
    }

    /// The composition's size is a length, so it follows the raster as the
    /// px rows do.
    fn derived_spatial(&self) -> &'static [ParamId] {
        &TransformPoints::DERIVED_FRAME
    }

    fn modify_points(&self, p: Params<'_>, cx: &crate::fx::ModifyCx<'_>) -> Option<PointsStream> {
        let frame = TransformPoints::DERIVED_FRAME.map(|id| p.float(id, 0.0));
        Some(TransformPoints::read(p).apply(cx.input()?, frame))
    }
}
