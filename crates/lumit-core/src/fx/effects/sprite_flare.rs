//! Sprite flare (docs/08 §3.29): the art-directed flare. What it draws is a
//! design kept in the effect, a list of glows, rays, rings, ghosts and so on
//! (see [`crate::fx::flare`]), edited in the Flare window. The rows here are
//! what animates it: the light, brightness, size, rotation, colour,
//! evolution and flicker.

use std::sync::{Arc, Mutex};

use crate::fx::flare::design::{self, Design, Element, Kind};
use crate::fx::flare::presets::default_design;
use crate::fx::flare::raster::{self, Frame, Light};
use crate::fx::noise::value3;
use crate::fx::{
    params::{ParamId, Params, Value},
    points::PointsStream,
    registry::{EffectDef, EffectMetadata, ResolveCx, Signature},
    schema::{EffectSchema, EnabledCond, EnabledWhen, ParamGroup, Port, PortType},
};
use crate::model::{Composition, EffectInstance, EffectParam, EffectValue};
use lumit_fx_macros::Effect;

/// The panel's twirls, and the rows only one way of finding the light uses.
pub const SPRITE_FLARE_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "",
        params: &["light_z"],
        collapsed: false,
        visible_when: Some(("light_from", &[THREE_D_POINT])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["light_mask"],
        collapsed: false,
        visible_when: Some(("light_from", &[MASK_POINTS])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["threshold", "threshold_softness"],
        collapsed: false,
        visible_when: Some(("light_from", &[BRIGHT_SPOTS])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Centre",
        params: &["centre_x", "centre_y"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Flicker",
        params: &["flicker", "flicker_speed", "flicker_seed"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Lights",
        params: &[
            "use_light_colour",
            "use_light_intensity",
            "size_with_distance",
        ],
        collapsed: false,
        visible_when: Some(("light_from", &[THREE_D_POINT, LIGHTS, BRIGHT_SPOTS, POINTS])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Hiding",
        params: &[
            "hide_behind",
            "hide_channel",
            "light_size",
            "depth_m",
            "light_distance_m",
            "edge_fade",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Textures",
        params: &["texture_layer"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// Light from: the Light row.
pub const POINT: u32 = 0;
/// Light from: the Light row and Light z, seen through the comp's camera.
pub const THREE_D_POINT: u32 = 1;
/// Light from: the comp's Light layers.
pub const LIGHTS: u32 = 2;
/// Light from: the bright parts of the layer's own picture.
pub const BRIGHT_SPOTS: u32 = 3;
/// Light from: each vertex of one of the layer's masks.
pub const MASK_POINTS: u32 = 4;
/// Light from: each point of a points stream wired in.
pub const POINTS: u32 = 5;

const POINTS_IN: &[Port] = &[Port::new(
    crate::fx::effects::clone_to_points::POINTS_PORT,
    "Points",
    PortType::Points,
)];

const fn when(param: &'static str, on: &'static str, cond: EnabledCond) -> EnabledWhen {
    EnabledWhen { param, on, cond }
}

/// The Light row is only where the light is while nothing else finds it, and
/// the Lights rows each mean something to some of the ways only.
pub const SPRITE_FLARE_ENABLED_WHEN: &[EnabledWhen] = &[
    when("light_x", "light_from", EnabledCond::ChoiceIsNot(LIGHTS)),
    when(
        "light_x",
        "light_from",
        EnabledCond::ChoiceIsNot(BRIGHT_SPOTS),
    ),
    when("light_y", "light_from", EnabledCond::ChoiceIsNot(LIGHTS)),
    when(
        "light_y",
        "light_from",
        EnabledCond::ChoiceIsNot(BRIGHT_SPOTS),
    ),
    when(
        "light_x",
        "light_from",
        EnabledCond::ChoiceIsNot(MASK_POINTS),
    ),
    when("light_x", "light_from", EnabledCond::ChoiceIsNot(POINTS)),
    when("light_y", "light_from", EnabledCond::ChoiceIsNot(POINTS)),
    when(
        "light_y",
        "light_from",
        EnabledCond::ChoiceIsNot(MASK_POINTS),
    ),
    when(
        "use_light_colour",
        "light_from",
        EnabledCond::ChoiceIsNot(THREE_D_POINT),
    ),
    when(
        "use_light_intensity",
        "light_from",
        EnabledCond::ChoiceIs(LIGHTS),
    ),
    when(
        "size_with_distance",
        "light_from",
        EnabledCond::ChoiceIsNot(BRIGHT_SPOTS),
    ),
    when("hide_channel", "hide_behind", EnabledCond::LayerSet),
    when("light_distance_m", "depth_m", EnabledCond::LayerSet),
];

/// Most lights one frame draws from: sixteen Light layers, or fewer each
/// drawn several times across a motion-blurred shutter.
pub const MAX_LIGHTS: usize = 64;

/// Most Light layers a frame takes, top of the stack first, and most mask
/// vertices.
pub const MAX_LIGHT_LAYERS: usize = 16;

/// `<prefix><n>` for n up to [`MAX_LIGHTS`], as one const array.
macro_rules! ids {
    ($prefix:literal) => {
        ids!($prefix; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
            21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63)
    };
    ($prefix:literal; $($n:literal),*) => {
        [$(ParamId::new(concat!($prefix, $n))),*]
    };
}

/// Sprite flare's controls.
///
/// Every position and distance is px@comp, declared `Px` so the resolve
/// step scales it to the raster. The design's own sizes are px@comp too, and
/// are scaled by the factor pushed beside them.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "sprite_flare",
    label = "Sprite flare",
    // 2: the glow, ghost and streak rows became a design kept in the effect.
    // `migrate_percent_to_px` carries a v1 instance over on load.
    version = 2,
    category = Stylise,
    cost = Cheap,
    roi = FullFrame,
    groups = SPRITE_FLARE_GROUPS,
    enabled_when = SPRITE_FLARE_ENABLED_WHEN,
)]
pub struct SpriteFlare {
    /// Opens the Flare window.
    #[action(label = "Edit flare…")]
    pub edit_flare: (),

    /// Open both sides: a light off the frame still throws ghosts across it.
    #[slider(min = 0.0, max = 3840.0, default = 640.0, unit = Px)]
    pub light_x: f32,

    /// See [`light_x`](Self::light_x).
    #[slider(min = 0.0, max = 2160.0, default = 360.0, unit = Px)]
    pub light_y: f32,

    /// Where the light comes from: the Light row, that row as a point in the
    /// comp's 3D space, the comp's Light layers, the bright parts of the
    /// layer's own picture, or the corners of one of its masks.
    #[choice(
        label = "Light from",
        options = ["Point", "3D point", "Lights", "Bright spots", "Mask points", "Points"],
        default = 0
    )]
    pub light_from: u32,

    /// The 3D point's depth, px@comp in the comp's own space: positive is
    /// away from a camera that has not moved.
    #[slider(label = "Light z", min = -4000.0, max = 4000.0, default = 0.0, unit = Px)]
    pub light_z: f32,

    /// The mask whose vertices are the lights, the layer's first unless
    /// another is picked.
    #[mask_path(label = "Mask")]
    pub light_mask: bool,

    /// The scene-linear luma a spot must pass to be a light, as Lens
    /// flare's Matte source reads it.
    #[slider(min = 0.0, max = 1.0, default = 1.0, hard_min = 0.0, unit = Raw)]
    pub threshold: f32,

    /// How far above Threshold a spot is a full light.
    #[slider(label = "Softness", min = 0.0, max = 1.0, default = 0.25, hard_min = 0.0, unit = Raw)]
    pub threshold_softness: f32,

    /// Gain on everything the effect draws; 0 draws nothing.
    #[slider(
        label = "Brightness",
        min = 0.0,
        max = 4.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub intensity: f32,

    /// The whole design's scale, per cent.
    #[slider(min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub size: f32,

    #[dial(label = "Rotation", default = 0.0)]
    pub rotation: f32,

    /// The light's colour, scene-linear and open above.
    #[colour(label = "Colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub tint: [f32; 4],

    /// What turns rays' shimmer and orbs' drift. A full turn comes back to
    /// the start.
    #[dial(label = "Evolution", default = 0.0)]
    pub evolution: f32,

    /// The optical centre the axis runs through. The schema default is the
    /// 1080p middle; `instantiate_for_raster` puts a fresh one on the comp's.
    #[slider(label = "Centre X", min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub centre_x: f32,

    /// See [`centre_x`](Self::centre_x).
    #[slider(label = "Centre Y", min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub centre_y: f32,

    /// How far the brightness dips at random.
    #[slider(
        label = "Amount",
        min = 0.0,
        max = 1.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 1.0,
        unit = Raw
    )]
    pub flicker: f32,

    /// Dips a second.
    #[slider(
        label = "Speed",
        min = 0.0,
        max = 30.0,
        default = 8.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub flicker_speed: f32,

    /// A counter rather than a seed row, so a flare that doesn't flicker
    /// keeps one frame key across time. See [`moves_with_time`].
    #[counter(label = "Seed", min = 0, max = 999, default = 1, hard_min = 0, unit = Raw)]
    pub flicker_seed: i32,

    /// A Light layer's own colour, or a spot's, tints the flare.
    #[toggle(label = "Use light colour", default = true)]
    pub use_light_colour: bool,

    /// A Light layer's Intensity scales its flare.
    #[toggle(label = "Use light intensity", default = true)]
    pub use_light_intensity: bool,

    /// A light further from the camera draws a smaller flare, by the
    /// camera's zoom over its distance.
    #[toggle(label = "Size with distance", default = false)]
    pub size_with_distance: bool,

    /// A layer that hides the light where it covers it, like a pole in
    /// front of the sun. Decided by the render, so always `false` here.
    #[layer(label = "Hide behind", self_default = false)]
    pub hide_behind: bool,

    /// What of Hide behind covers the light: its alpha or its luma.
    #[choice(label = "Channel", options = ["Alpha", "Luma"], default = 0)]
    pub hide_channel: u32,

    /// How wide the light is, px@comp: a light half behind an edge flares
    /// at about half.
    #[slider(min = 0.0, max = 400.0, default = 20.0, hard_min = 0.0, unit = Px)]
    pub light_size: f32,

    /// Distance in metres per pixel, as Lens flare's Depth reads it. Set, a
    /// light with something nearer in front of it is hidden. Decided by the
    /// render, so always `false` here.
    #[layer(label = "Depth", self_default = false)]
    pub depth_m: bool,

    /// How far away the light is, metres, read against the Depth. Bright
    /// spots sit on the picture, so nothing hides them.
    #[slider(
        label = "Light distance (m)",
        min = 1.0,
        max = 1000.0,
        default = 1000.0,
        hard_min = 0.01,
        unit = Raw
    )]
    pub light_distance_m: f32,

    /// How far past the frame's edge the light fades out. 0 never fades.
    #[slider(min = 0.0, max = 1000.0, default = 200.0, hard_min = 0.0, unit = Px)]
    pub edge_fade: f32,

    /// A layer's picture the design's elements can take as their texture:
    /// footage of a dirty lens, a logo. Decided by the render, so always
    /// `false` here.
    #[layer(label = "Texture layer", self_default = false)]
    pub texture_layer: bool,

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

impl SpriteFlare {
    /// Raster pixels per comp pixel, pushed at resolve for the design's sizes.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");
    /// Layer time, seconds, for Spin and Flicker.
    pub const DERIVED_TIME: ParamId = ParamId::new("derived.time");
    /// How many lights the resolve found, when it found them rather than
    /// the Light row: Light from is 3D point, Lights or Mask points, or the
    /// shutter is open across the frame.
    pub const DERIVED_LIGHT_COUNT: ParamId = ParamId::new("derived.sf_light_count");
    /// Each light found: x and y in comp pixels, its gain and its scale.
    pub const DERIVED_LIGHTS: [ParamId; MAX_LIGHTS] = ids!("derived.sf_light_");
    /// Each light's own colour, before the Colour row.
    pub const DERIVED_LIGHT_RGB: [ParamId; MAX_LIGHTS] = ids!("derived.sf_light_rgb_");

    /// The frame the design is drawn into, out of a resolved bag. No lights
    /// at Brightness 0 or Mix 0, which both render paths take as the
    /// untouched input.
    pub fn frame(p: Params<'_>) -> Frame {
        let s = Self::read(p);
        let px_scale = p.float(Self::DERIVED_PX_SCALE, 1.0).max(1e-6);
        let time = p.float(Self::DERIVED_TIME, 0.0);
        let mix = (s.mix / 100.0).clamp(0.0, 1.0);
        let dip = 0.5
            + 0.5
                * value3(
                    s.flicker_seed.max(0) as u32,
                    0,
                    time * s.flicker_speed,
                    0.0,
                    0.0,
                    0,
                );
        let colour = [s.tint[0].max(0.0), s.tint[1].max(0.0), s.tint[2].max(0.0)];
        let found = p.int(Self::DERIVED_LIGHT_COUNT, -1);
        let lights = if s.intensity <= 0.0 || mix <= 0.0 || s.light_from == BRIGHT_SPOTS {
            Vec::new()
        } else if found >= 0 {
            (0..(found as usize).min(MAX_LIGHTS))
                .map(|i| {
                    let [x, y, gain, scale] = p.vec4(Self::DERIVED_LIGHTS[i], [0.0; 4]);
                    let c = p.colour(Self::DERIVED_LIGHT_RGB[i], [1.0; 4]);
                    Light {
                        pos: [x * px_scale, y * px_scale],
                        colour: [c[0] * colour[0], c[1] * colour[1], c[2] * colour[2]],
                        gain: gain * mix,
                        scale,
                    }
                })
                .collect()
        } else if matches!(s.light_from, LIGHTS | MASK_POINTS | POINTS) {
            // No comp or layer to find them in, or the points arrive at the
            // draw.
            Vec::new()
        } else {
            vec![Light {
                pos: [s.light_x, s.light_y],
                colour,
                // Mix rides on the light, so even an element that ignores
                // Brightness is mixed.
                gain: mix,
                scale: 1.0,
            }]
        };
        Frame {
            lights,
            centre: [s.centre_x, s.centre_y],
            px_scale,
            brightness: s.intensity.max(0.0) * (1.0 - s.flicker.clamp(0.0, 1.0) * dip),
            size: (s.size / 100.0).max(0.0),
            rotation_deg: s.rotation,
            evolution_deg: s.evolution,
            time_s: time,
            // The row arrives at the raster's size, the frame wants comp px.
            edge_fade: s.edge_fade.max(0.0) / px_scale,
        }
    }

    /// One light for each point of the wired stream while Light from is
    /// Points, up to 16: where the point is seen, its colour when Use light
    /// colour is on, its opacity as its brightness, and its foreshortening
    /// as its size when Size with distance is on.
    pub fn points_lights(p: Params<'_>, stream: &PointsStream) -> Option<Vec<Light>> {
        let s = Self::read(p);
        let mix = (s.mix / 100.0).clamp(0.0, 1.0);
        if s.light_from != POINTS || s.intensity <= 0.0 || mix <= 0.0 {
            return None;
        }
        let tint = [s.tint[0].max(0.0), s.tint[1].max(0.0), s.tint[2].max(0.0)];
        let picked = |i: usize| stream.pick.get(i).is_none_or(|&k| k > 0.5);
        let lights = (0..stream.position.len())
            .filter(|&i| picked(i) && stream.depth_scale(i) > 0.0)
            .take(16)
            .map(|i| {
                let [r, g, b, a] = stream.colour.get(i).copied().unwrap_or([1.0; 4]);
                let own = if s.use_light_colour && a > 1e-6 {
                    [r / a, g / a, b / a]
                } else {
                    [1.0; 3]
                };
                Light {
                    pos: stream.projected(i),
                    colour: [own[0] * tint[0], own[1] * tint[1], own[2] * tint[2]],
                    gain: a.clamp(0.0, 1.0) * mix,
                    scale: if s.size_with_distance {
                        stream.depth_scale(i)
                    } else {
                        1.0
                    },
                }
            })
            .collect();
        Some(lights)
    }

    /// How to find the lights in the picture while Light from is Bright
    /// spots and the flare draws at all.
    pub fn spots(p: Params<'_>) -> Option<Spots> {
        let s = Self::read(p);
        let mix = (s.mix / 100.0).clamp(0.0, 1.0);
        (s.light_from == BRIGHT_SPOTS && s.intensity > 0.0 && mix > 0.0).then(|| Spots {
            threshold: s.threshold.max(0.0),
            softness: s.threshold_softness.max(0.0),
            use_colour: s.use_light_colour,
            colour: [s.tint[0].max(0.0), s.tint[1].max(0.0), s.tint[2].max(0.0)],
            gain: mix,
        })
    }

    /// What Hide behind reads, luma rather than alpha, and the light's
    /// radius in raster pixels.
    pub fn hiding(p: Params<'_>) -> (bool, f32) {
        let s = Self::read(p);
        (s.hide_channel == 1, s.light_size.max(0.0) * 0.5)
    }

    /// How far away the light is against the Depth, metres, or `None` for
    /// Bright spots, whose lights sit on the picture.
    pub fn light_distance(p: Params<'_>) -> Option<f32> {
        let s = Self::read(p);
        (s.light_from != BRIGHT_SPOTS).then_some(s.light_distance_m.max(0.01))
    }
}

/// How Bright spots finds its lights: Lens flare's detection, and what each
/// spot it finds becomes. See [`raster::spots`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spots {
    pub threshold: f32,
    pub softness: f32,
    pub use_colour: bool,
    pub colour: [f32; 3],
    pub gain: f32,
}

/// One light the resolve found: comp x, y, gain and scale, and its colour.
type Found = ([f32; 4], [f32; 3]);

/// The lights at `d` seconds from the frame `cx` resolves, in `mode`.
fn lights_at(cx: &ResolveCx<'_>, comp: &Composition, mode: u32, d: f64) -> Vec<Found> {
    let ctx = &cx.context;
    let on = |id: &str, default: bool| cx.inst.bool_of(id).unwrap_or(default);
    let at = |id: &str| {
        cx.inst
            .float_at_with_context(id, cx.lt + d, ctx.clone())
            .unwrap_or(0.0)
    };
    let t = ctx.comp_time + d;
    let (w, h) = (f64::from(comp.width), f64::from(comp.height));
    // Seen the way the 3D layers are, through the default camera when the
    // comp has none. Behind the eye is nowhere.
    let pose = crate::track::active_camera_pose(&ctx.document, comp, t)
        .unwrap_or_else(|| crate::camera::default_pose(w, h));
    let m = crate::camera::projection(&pose, w, h);
    let by_distance = on("size_with_distance", false);
    let seen = |p: (f64, f64, f64)| {
        let row = |r: [f64; 4]| r[0] * p.0 + r[1] * p.1 + r[2] * p.2 + r[3];
        let depth = row(m[2]);
        // The plane the camera sees 1:1 is depth 1, so the scale is its
        // inverse.
        let scale = if by_distance { 1.0 / depth } else { 1.0 };
        (depth > 1e-6).then(|| {
            [
                (row(m[0]) / depth) as f32,
                (row(m[1]) / depth) as f32,
                scale as f32,
            ]
        })
    };
    match mode {
        POINT => vec![(
            [at("light_x") as f32, at("light_y") as f32, 1.0, 1.0],
            [1.0; 3],
        )],
        THREE_D_POINT => seen((at("light_x"), at("light_y"), at("light_z")))
            .map(|[x, y, k]| ([x, y, 1.0, k], [1.0; 3]))
            .into_iter()
            .collect(),
        // Mask vertices are layer pixels, the space the Light row is in.
        MASK_POINTS => {
            let masks = ctx
                .layer
                .and_then(|id| comp.layers.iter().find(|l| l.id == id))
                .map_or(&[][..], |l| &l.masks[..]);
            crate::mask::mask_index_for_path_param(masks, cx.inst.mask_ref("light_mask"), true)
                .and_then(|i| masks.get(i))
                .map(|m| {
                    m.path_at(cx.lt + d)
                        .vertices
                        .iter()
                        .take(MAX_LIGHT_LAYERS)
                        .map(|v| ([v.pos.0 as f32, v.pos.1 as f32, 1.0, 1.0], [1.0; 3]))
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => {
            let read = crate::model::context_reader(ctx, t);
            let solves = crate::track::solve_store().map(|store| (&ctx.document, store));
            let colour = on("use_light_colour", true);
            let intensity = on("use_light_intensity", true);
            comp.lights_at(t, &read, solves)
                .into_iter()
                // An ambient light is not anywhere, so it has no place to
                // flare from.
                .filter(|l| l.kind != crate::model::LightKind::Ambient)
                .take(MAX_LIGHT_LAYERS)
                .filter_map(|l| {
                    let [x, y, k] = seen((l.position.0, l.position.1, l.z))?;
                    // A negative light takes light away, which a flare cannot.
                    let gain = if intensity { l.gain.max(0.0) } else { 1.0 };
                    let rgb = if colour { l.tint } else { [1.0; 3] };
                    Some(([x, y, gain, k], rgb))
                })
                .collect()
        }
    }
}

/// Whether the flare changes over time with every row held still: Flicker
/// is on or keyed, or a shown element spins. Only then does the layer time
/// join its frame key, so a still flare is cached once for its whole span.
pub fn moves_with_time(e: &EffectInstance) -> bool {
    let flickers = match e.param("flicker") {
        Some(EffectValue::Float(p)) => p.is_animated() || p.value_at(0.0) > 0.0,
        _ => false,
    };
    flickers
        || design::embedded(e)
            .unwrap_or_else(default_design)
            .elements
            .iter()
            .any(|el| el.on && el.num("spin") != 0.0)
}

/// The design a render draws: the kept one (content hash and text, as the
/// lens carriage brings it) or the default when none is kept or it won't
/// read. Parsed once per text.
pub fn design_of(kept: Option<&(u64, String)>) -> Arc<Design> {
    static PARSED: Mutex<Vec<(u64, Arc<Design>)>> = Mutex::new(Vec::new());
    let Some((hash, text)) = kept else {
        return Arc::new(default_design());
    };
    if let Some(hit) = PARSED
        .lock()
        .ok()
        .and_then(|list| list.iter().find(|(h, _)| h == hash).map(|(_, d)| d.clone()))
    {
        return hit;
    }
    let design = Arc::new(Design::parse(text).unwrap_or_else(default_design));
    if let Ok(mut list) = PARSED.lock() {
        if list.len() >= 8 {
            list.remove(0);
        }
        list.push((*hash, design.clone()));
    }
    design
}

/// The rows version 1 had and version 2 keeps in its design instead.
const V1_ROWS: [&str; 9] = [
    "glow_size",
    "glow_intensity",
    "ghosts",
    "ghost_spacing",
    "ghost_size",
    "ghost_intensity",
    "streak_length",
    "streak_intensity",
    "streak_angle",
];

/// Carry a version 1 instance over: its glow, ghosts and streak become a
/// design, its centre the middle of the `w` by `h` comp it was always drawn
/// about, and it never fades at the edge, as it never did. The glow and the
/// streak come out exactly; the ghosts were a disc profile no iris draws, so
/// they are a near match. A keyframed row is read at its first frame.
pub fn migrate_v1(e: &mut EffectInstance, w: f64, h: f64) {
    let old = |id: &str, default: f32| e.float_at(id, 0.0).map_or(default, |v| v as f32);
    let (glow_size, glow) = (old("glow_size", 120.0), old("glow_intensity", 1.0));
    let ghosts = old("ghosts", 6.0).round().clamp(0.0, 16.0) as u32;
    let spacing = old("ghost_spacing", 0.35).max(0.0);
    let (ghost_size, ghost) = (old("ghost_size", 60.0), old("ghost_intensity", 0.35));
    let (streak_length, streak) = (old("streak_length", 300.0), old("streak_intensity", 0.5));
    let streak_angle = old("streak_angle", 0.0);

    let element = |kind: Kind, values: &[(&str, f32)]| {
        let mut el = Element::new(kind);
        for (key, v) in values {
            el.set(key, *v);
        }
        el
    };
    let mut elements = Vec::new();
    if glow > 0.0 && glow_size > 0.0 {
        elements.push(element(
            Kind::Glow,
            &[
                ("size", glow_size),
                ("brightness", glow),
                ("falloff", 0.5),
                ("core", 0.0),
            ],
        ));
    }
    if ghost > 0.0 && ghost_size > 0.0 {
        for i in 1..=ghosts {
            let t = i as f32 * spacing;
            // Every other ghost fell off harder, so it is drawn smaller.
            let tighter = if i % 2 == 0 { 0.6 } else { 0.75 };
            elements.push(element(
                Kind::Ghost,
                &[
                    ("place", 1.0 + t),
                    ("size", ghost_size * (0.35 + 0.65 * t) * tighter),
                    ("brightness", ghost / i as f32),
                    ("sides", 0.0),
                    ("fill", 1.0),
                    ("edge", 0.0),
                    ("softness", 1.0),
                    ("fringe", 0.0),
                ],
            ));
        }
    }
    if streak > 0.0 && streak_length > 0.0 {
        elements.push(element(
            Kind::Streak,
            &[
                ("size", streak_length),
                ("brightness", streak),
                ("thickness", 0.03 * streak_length),
                ("taper", 0.5),
                ("rotation", streak_angle),
            ],
        ));
    }
    design::set_embedded(
        e,
        Some(&Design {
            lumit_flare: design::FORMAT,
            name: String::new(),
            elements,
        }),
    );
    e.params.retain(|p| !V1_ROWS.contains(&p.id.as_str()));
    for (id, v) in [
        ("centre_x", w * 0.5),
        ("centre_y", h * 0.5),
        ("edge_fade", 0.0),
    ] {
        let value = EffectValue::Float(crate::anim::Property::fixed(v));
        match e.params.iter_mut().find(|p| p.id == id) {
            Some(p) => p.value = value,
            None => e.params.push(EffectParam {
                id: id.to_owned(),
                value,
                extra: serde_json::Map::new(),
            }),
        }
    }
    e.effect.version = 2;
}

/// Sprite flare's behaviour.
pub struct SpriteFlareDef;

impl EffectDef for SpriteFlareDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SpriteFlare as EffectMetadata>::SCHEMA
    }

    fn depth_inputs(&self) -> &'static [&'static str] {
        &["depth_m"]
    }

    fn signature(&self) -> Signature {
        Signature::Image {
            inputs: POINTS_IN,
            extra: &[],
        }
    }

    /// Where the lights are when the Light row alone doesn't say: through
    /// the camera, from the Light layers or a mask, and across the shutter
    /// when the layer blurs. Bright spots are found on the GPU from the picture.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(SpriteFlare::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
        push(SpriteFlare::DERIVED_TIME, Value::Float(cx.lt as f32));
        let mode = match cx.inst.param("light_from") {
            Some(EffectValue::Choice(c)) => *c,
            _ => POINT,
        };
        let ctx = &cx.context;
        let Some(comp) = ctx.comp.and_then(|id| ctx.document.comp(id)) else {
            return;
        };
        if mode == BRIGHT_SPOTS || mode == POINTS {
            return;
        }
        let blurs = ctx
            .layer
            .and_then(|id| comp.layers.iter().find(|l| l.id == id))
            .is_some_and(|l| l.switches.motion_blur);
        let mut shutter = if blurs {
            comp.motion_blur.sample_offsets()
        } else {
            Vec::new()
        };
        if mode == POINT && shutter.is_empty() {
            return;
        }
        let now = lights_at(cx, comp, mode, 0.0);
        // Fewer samples when there are many lights, so the frame stays in
        // budget.
        let room = (MAX_LIGHTS / now.len().max(1)) as u32;
        if shutter.len() > room as usize {
            shutter = crate::model::MotionBlur {
                samples: room,
                ..comp.motion_blur
            }
            .sample_offsets();
        }
        let dt = 1.0 / comp.frame_rate.fps().max(1.0);
        let samples: Vec<Vec<Found>> = shutter
            .iter()
            .map(|off| lights_at(cx, comp, mode, off * dt))
            .collect();
        // A light that holds still across the shutter is drawn once.
        let lights: Vec<Found> = if samples.iter().all(|s| *s == now) {
            now
        } else {
            let share = 1.0 / samples.len() as f32;
            samples
                .into_iter()
                .flatten()
                .map(|([x, y, gain, k], rgb)| ([x, y, gain * share, k], rgb))
                .collect()
        };
        let n = lights.len().min(MAX_LIGHTS);
        for (i, (geometry, [r, g, b])) in lights.into_iter().take(n).enumerate() {
            push(SpriteFlare::DERIVED_LIGHTS[i], Value::Vec4(geometry));
            push(
                SpriteFlare::DERIVED_LIGHT_RGB[i],
                Value::Colour([r, g, b, 0.0]),
            );
        }
        push(SpriteFlare::DERIVED_LIGHT_COUNT, Value::Int(n as i32));
    }

    /// Raster pixels per comp pixel follows the raster as a length does.
    fn derived_spatial(&self) -> &'static [ParamId] {
        &[SpriteFlare::DERIVED_PX_SCALE]
    }

    /// The default design: a kept one rides beside the op, which a bag alone
    /// can't carry. Bright spots are looked for in the picture as it came in.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let mut frame = SpriteFlare::frame(p);
        if let Some(s) = SpriteFlare::spots(p) {
            let found = crate::fx::lens_flare::detect_lights(
                rgba,
                w,
                h,
                s.threshold,
                s.softness,
                s.use_colour,
                [1.0; 3],
                false,
            );
            frame.lights = raster::spots(&found, w, h, s.use_colour, s.colour, s.gain);
        }
        raster::draw(&default_design(), &frame, w, h, rgba);
    }
}

#[cfg(test)]
mod tests;
