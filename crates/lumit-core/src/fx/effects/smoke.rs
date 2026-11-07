//! Smoke and fire: gas in a box, worked out on the graphics card.
//!
//! The effect turns a 3D layer into a box of smoke. The layer's transform is the
//! box, so its gizmo, parents and keyframes all carry it. Each frame is the walk
//! from the layer's in point (`lumit-render`'s `sim::smoke`), and the comp's
//! volumetric walk draws the result, lit by the comp's lights, cut off by what is
//! in front of it. The layer's own picture never shows.

use crate::fx::params::Value;
use crate::fx::schema::{EnabledCond, EnabledWhen, ParamGroup};
use crate::fx::{
    params::ParamId, registry::EffectDef, registry::EffectMetadata, registry::ResolveCx,
    schema::EffectSchema,
};
use crate::model::{Composition, Document, EffectInstance, EffectValue, Layer, LayerKind};
use lumit_fx_macros::Effect;

/// The effect's match name.
pub const MATCH_NAME: &str = "smoke";

/// The Resolution row's words, and the cells along the box's longest side each
/// one means.
const RESOLUTION_OPTIONS: &[&str] = &["64", "96", "128"];
const RESOLUTIONS: [u32; 3] = [64, 96, 128];

const EMIT_OPTIONS: &[&str] = &["Layer's box", "Particles from a layer", "Model layer"];

const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

const GROUPS: &[ParamGroup] = &[
    group(
        "Simulation",
        &["simulate", "sub_steps", "bake", "cancel", "resolution"],
    ),
    group(
        "Emitter",
        &[
            "emit_from",
            "emit_layer",
            "emit_size",
            "density",
            "temperature",
            "fuel",
            "expansion",
        ],
    ),
    group(
        "Motion",
        &[
            "buoyancy",
            "turbulence",
            "vorticity",
            "dissipation",
            "wind_across",
            "wind_vertical",
            "wind_depth",
            "trail",
        ],
    ),
    ParamGroup {
        collapsed: true,
        ..group("Collision", &["collide_with", "collide_all"])
    },
    group(
        "Look",
        &[
            "smoke_colour",
            "fire_brightness",
            "fire_colour",
            "fire_core",
            "shadow_strength",
        ],
    ),
];

/// A row only a walk reads, so it greys with Simulate off and on a volume file.
const fn walk_row(param: &'static str) -> EnabledWhen {
    EnabledWhen {
        param,
        on: "simulate",
        cond: EnabledCond::BoolIs(true),
    }
}

const ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "emit_layer",
        on: "emit_from",
        cond: EnabledCond::ChoiceIsNot(0),
    },
    EnabledWhen {
        param: "emit_size",
        on: "emit_from",
        cond: EnabledCond::ChoiceIs(0),
    },
    walk_row("sub_steps"),
    walk_row("resolution"),
    walk_row("emit_from"),
    walk_row("emit_layer"),
    walk_row("emit_size"),
    walk_row("temperature"),
    walk_row("fuel"),
    walk_row("expansion"),
    walk_row("buoyancy"),
    walk_row("vorticity"),
    walk_row("turbulence"),
    walk_row("dissipation"),
    walk_row("wind_across"),
    walk_row("wind_vertical"),
    walk_row("wind_depth"),
    walk_row("trail"),
    walk_row("collide_with"),
    walk_row("collide_all"),
    EnabledWhen {
        param: "bake",
        on: "simulate",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "cancel",
        on: "simulate",
        cond: EnabledCond::BoolIs(true),
    },
];

/// Smoke and fire's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "smoke",
    label = "Smoke and fire",
    version = 1,
    category = Generate,
    cost = Heavy,
    // It draws in the comp's scene, wherever its box is.
    roi = FullFrame,
    premultiplied = true,
    matte = false,
    // The layer's local time joins the frame's name, as every frame differs.
    seeded = true,
    groups = GROUPS,
    enabled_when = ENABLED_WHEN,
)]
pub struct Smoke {
    /// Works each frame out from the frame before. Off, only what a bake or the
    /// card already holds is shown.
    #[toggle(label = "Simulate", default = true)]
    pub simulate: bool,

    /// How many times a frame the gas moves. More keeps fast flames honest.
    #[counter(
        label = "Sub-steps",
        min = 1,
        max = 8,
        default = 1,
        hard_min = 1,
        hard_max = 8,
        unit = Raw
    )]
    pub sub_steps: i32,

    /// Bake the simulation to the project's cache.
    #[action(label = "Bake")]
    pub bake: (),

    /// Stop a running bake.
    #[action(label = "Cancel")]
    pub cancel: (),

    /// Cells along the box's longest side. More shows finer curls and takes
    /// memory and time by the cube.
    #[choice(label = "Resolution", options = RESOLUTION_OPTIONS, default = 2)]
    pub resolution: u32,

    /// What the gas comes from.
    #[choice(label = "Emit from", options = EMIT_OPTIONS, default = 0)]
    pub emit_from: u32,

    /// The Particulate layer or Model layer that gas comes from.
    #[layer(label = "Emit layer", self_default = false)]
    pub emit_layer: bool,

    /// How wide the patch on the box's floor is, as a share of the floor.
    #[slider(
        label = "Emitter size",
        min = 0.0,
        max = 100.0,
        default = 40.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub emit_size: f32,

    /// How much smoke it gives off.
    #[slider(
        label = "Density",
        min = 0.0,
        max = 200.0,
        default = 4.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub density: f32,

    /// How hot the gas is where it starts. Hot gas rises, and glows past a third.
    #[slider(
        label = "Temperature",
        min = 0.0,
        max = 100.0,
        default = 70.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub temperature: f32,

    /// How much there is to burn where the gas starts. Fuel that is hot burns,
    /// which heats the gas, swells it and gives off smoke.
    #[slider(
        label = "Fuel",
        min = 0.0,
        max = 100.0,
        default = 60.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub fuel: f32,

    /// How hard burning swells the gas. A lot makes a fireball.
    #[slider(
        label = "Expansion",
        min = 0.0,
        max = 400.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub expansion: f32,

    /// How hard heat pushes the gas up. Below nought it sinks.
    #[slider(
        label = "Buoyancy",
        min = -100.0,
        max = 200.0,
        default = 100.0,
        unit = Percent
    )]
    pub buoyancy: f32,

    /// How much the gas is broken up into small swirls.
    #[slider(
        label = "Turbulence",
        min = 0.0,
        max = 100.0,
        default = 30.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub turbulence: f32,

    /// How much curls are kept alive.
    #[slider(
        label = "Vorticity",
        min = 0.0,
        max = 100.0,
        default = 30.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub vorticity: f32,

    /// How fast smoke thins and heat fades.
    #[slider(
        label = "Dissipation",
        min = 0.0,
        max = 100.0,
        default = 35.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub dissipation: f32,

    /// The speed the air moves at along the box's width, comp pixels a second.
    #[slider(label = "Wind x", min = -1000.0, max = 1000.0, default = 0.0, unit = Raw)]
    pub wind_across: f32,

    /// Along its height.
    #[slider(label = "Wind y", min = -1000.0, max = 1000.0, default = 0.0, unit = Raw)]
    pub wind_vertical: f32,

    /// Along its depth.
    #[slider(label = "Wind z", min = -1000.0, max = 1000.0, default = 0.0, unit = Raw)]
    pub wind_depth: f32,

    /// Moving the box leaves the gas behind in the comp. Off, the gas rides in
    /// the box.
    #[toggle(label = "Trail", default = false)]
    pub trail: bool,

    /// A layer the gas goes round. Unset, nothing is in its way but the floor.
    #[layer(label = "Collide with", self_default = false)]
    pub collide_with: bool,

    /// The gas goes round every 3D layer and Model layer in the comp as well.
    #[toggle(label = "All 3D layers", default = false)]
    pub collide_all: bool,

    /// The smoke's colour, as light falling on it shows it.
    #[colour(label = "Smoke colour", default = [0.18, 0.18, 0.19, 1.0], max = 4.0)]
    pub smoke_colour: [f32; 4],

    /// How much light hot gas gives. Nought is no flames.
    #[slider(
        label = "Fire brightness",
        min = 0.0,
        max = 400.0,
        default = 100.0,
        hard_min = 0.0,
        unit = Percent
    )]
    pub fire_brightness: f32,

    /// The fire at its coolest.
    #[colour(label = "Fire colour", default = [1.0, 0.15, 0.008, 1.0], max = 16.0)]
    pub fire_colour: [f32; 4],

    /// The fire at its hottest.
    #[colour(label = "Fire core", default = [3.0, 1.3, 0.12, 1.0], max = 16.0)]
    pub fire_core: [f32; 4],

    /// How dark the smoke's shadows are, on itself and on what is under it.
    #[slider(
        label = "Shadow strength",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub shadow_strength: f32,
}

/// Smoke's behaviour. What it draws needs the camera and the comp's lights, so
/// it has no CPU reference through the trait.
pub struct SmokeDef;

impl EffectDef for SmokeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Smoke as EffectMetadata>::SCHEMA
    }

    fn presets(&self) -> &'static [crate::fx::presets::EffectPreset] {
        use crate::fx::presets::{EffectPreset, PresetValue as V};
        // Burning gas gives soot, dark in Smoke colour, and a burst of fuel a fireball.
        const SOOT: V = V::Colour([0.1, 0.1, 0.105, 1.0]);
        const PALE: V = V::Colour([0.62, 0.62, 0.64, 1.0]);
        &[
            EffectPreset {
                name: "Fire",
                values: &[
                    ("emit_size", V::Float(40.0)),
                    ("density", V::Float(30.0)),
                    ("temperature", V::Float(90.0)),
                    ("fuel", V::Float(80.0)),
                    ("turbulence", V::Float(40.0)),
                    ("smoke_colour", SOOT),
                ],
            },
            EffectPreset {
                name: "Candle",
                values: &[
                    ("emit_size", V::Float(12.0)),
                    ("density", V::Float(0.0)),
                    ("temperature", V::Float(100.0)),
                    ("fuel", V::Float(100.0)),
                    ("expansion", V::Float(30.0)),
                    ("buoyancy", V::Float(60.0)),
                    ("turbulence", V::Float(5.0)),
                    ("vorticity", V::Float(5.0)),
                    ("smoke_colour", SOOT),
                ],
            },
            EffectPreset {
                name: "Torch",
                values: &[
                    ("emit_size", V::Float(14.0)),
                    ("density", V::Float(20.0)),
                    ("temperature", V::Float(100.0)),
                    ("fuel", V::Float(100.0)),
                    ("turbulence", V::Float(50.0)),
                    ("buoyancy", V::Float(130.0)),
                    ("smoke_colour", SOOT),
                ],
            },
            EffectPreset {
                name: "Explosion",
                values: &[
                    ("emit_size", V::Float(35.0)),
                    (
                        "density",
                        V::KeysOut(&[(0.0, 18.0), (1.0, 18.0), (1.3, 0.0)]),
                    ),
                    (
                        "temperature",
                        V::KeysOut(&[(0.0, 100.0), (1.0, 100.0), (1.3, 0.0)]),
                    ),
                    (
                        "fuel",
                        V::KeysOut(&[(0.0, 100.0), (1.0, 100.0), (1.3, 0.0)]),
                    ),
                    ("expansion", V::Float(130.0)),
                    ("buoyancy", V::Float(80.0)),
                    ("turbulence", V::Float(100.0)),
                    ("vorticity", V::Float(70.0)),
                    ("dissipation", V::Float(80.0)),
                    ("fire_brightness", V::Float(200.0)),
                    ("smoke_colour", V::Colour([0.16, 0.16, 0.17, 1.0])),
                ],
            },
            EffectPreset {
                name: "Smoke",
                values: &[
                    ("emit_size", V::Float(35.0)),
                    ("density", V::Float(120.0)),
                    ("temperature", V::Float(45.0)),
                    ("fuel", V::Float(0.0)),
                    ("turbulence", V::Float(40.0)),
                    ("fire_brightness", V::Float(0.0)),
                    ("smoke_colour", PALE),
                ],
            },
        ]
    }

    /// It draws no picture of its own: the comp's volumetric walk does.
    fn is_image_op(&self) -> bool {
        false
    }

    /// These rows are read for where those layers are, never for a picture.
    fn shape_inputs(&self) -> &'static [&'static str] {
        &["emit_layer", "collide_with"]
    }

    fn resolve_derived(&self, _cx: &ResolveCx<'_>, _push: &mut dyn FnMut(ParamId, Value)) {}
}

/// Rows that only change how smoke is drawn, so a change to them keeps every
/// state and bake. A row not named here renames the walk, which is the safe way
/// round for a new one.
pub const LOOK_ROWS: &[&str] = &[
    "smoke_colour",
    "fire_brightness",
    "fire_colour",
    "fire_core",
    "shadow_strength",
    "simulate",
];

/// Whether the effect called `match_name` has a Bake that walks its frames.
#[must_use]
pub fn bakes_a_walk(match_name: &str) -> bool {
    matches!(match_name, "bake_points" | "particulate" | MATCH_NAME)
}

/// Whether `e` is a Smoke effect that is on.
#[must_use]
pub fn is_smoke(e: &EffectInstance) -> bool {
    e.enabled && e.effect.match_name == MATCH_NAME
}

/// The Smoke effect `layer` draws as a box of smoke, if it has one and is 3D.
#[must_use]
pub fn on_layer(layer: &Layer) -> Option<&EffectInstance> {
    if !layer.switches.three_d || !layer.switches.fx {
        return None;
    }
    layer.effects.iter().find(|e| is_smoke(e))
}

/// Whether `l` stops the comp it's in being spliced into its parent: an adjustment
/// layer works on the comp beneath it, and a box of smoke draws in its comp's walk.
#[must_use]
pub fn keeps_own_comp(l: &Layer) -> bool {
    (l.is_adjustment() && l.switches.fx && l.effects.iter().any(|e| e.enabled))
        || on_layer(l).is_some()
}

/// The effect a volume file's layer starts with: the look of a box of smoke, with
/// nothing to simulate.
#[must_use]
pub fn for_volume() -> Option<EffectInstance> {
    let mut e = crate::fx::builtins::instantiate(MATCH_NAME)?;
    let simulate = e.params.iter_mut().find(|p| p.id == "simulate")?;
    simulate.value = EffectValue::Bool(false);
    // A file's gas fills more of its box than a walk's, and its flames too.
    for (id, v) in [("density", 25.0), ("fire_brightness", 30.0)] {
        let row = e.params.iter_mut().find(|p| p.id == id)?;
        row.value = EffectValue::Float(crate::anim::Property::fixed(v));
    }
    Some(e)
}

/// A layer's box before its Scale: its picture's width and height, or the comp's
/// where it has none, and as deep as the shorter of the two.
#[must_use]
pub fn box_size(doc: &Document, comp: &Composition, layer: &Layer) -> [f32; 3] {
    let (w, h) = match &layer.kind {
        LayerKind::Solid { def } => doc
            .solid(*def)
            .map_or((comp.width, comp.height), |s| (s.width, s.height)),
        _ => (comp.width, comp.height),
    };
    let (w, h) = (w as f32, h as f32);
    [w, h, w.min(h)]
}

/// A box's grid: how many cells each way, and how wide a cell is in comp pixels
/// at the layer's first frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub cells: [u32; 3],
    pub cell_px: f32,
}

/// The fewest cells a box has along any side.
const FEWEST_CELLS: u32 = 8;

/// `e`'s grid on `layer`. Scale after the first frame stretches the box and
/// leaves the grid alone.
#[must_use]
pub fn grid(doc: &Document, comp: &Composition, layer: &Layer, e: &EffectInstance) -> Grid {
    let t = &layer.transform;
    let size = box_size(doc, comp, layer);
    let scale =
        [&t.scale_x, &t.scale_y, &t.scale_z].map(|p| (p.value_at(0.0).abs() as f32) / 100.0);
    let px = [0, 1, 2].map(|k| (size[k] * scale[k]).max(1.0));
    let longest = px[0].max(px[1]).max(px[2]);
    let code = match e.param("resolution") {
        Some(EffectValue::Choice(c)) => *c as usize,
        _ => 2,
    };
    let res = RESOLUTIONS[code.min(RESOLUTIONS.len() - 1)];
    Grid {
        cells: px.map(|v| ((v / longest * res as f32).round() as u32).max(FEWEST_CELLS)),
        cell_px: longest / res as f32,
    }
}

/// What `inst` on layer `owner` goes round: what its Collision rows name, but
/// never another box of smoke, which is only gas.
pub fn solid_layers<'a>(
    comp: &'a Composition,
    inst: &EffectInstance,
    owner: uuid::Uuid,
) -> impl Iterator<Item = &'a Layer> {
    crate::fx::points::stepped::sim::hit_layers(comp, inst, owner).filter(|l| on_layer(l).is_none())
}

/// The layer `e`'s Emit layer row names, if it emits from one.
#[must_use]
pub fn emit_layer<'a>(comp: &'a Composition, e: &EffectInstance) -> Option<&'a Layer> {
    if !matches!(e.param("emit_from"), Some(EffectValue::Choice(1 | 2))) {
        return None;
    }
    let named = e.layer_ref("emit_layer")?;
    comp.layers.iter().find(|l| l.id == named)
}

/// Everything a walk reads, as one name: its kept states and its bake are found
/// by it. What only changes how it looks is left out, and so is the box's place,
/// since the gas lives in the box, so moving a box draws again rather than walks
/// again. What it emits from is what ties it to the rest of the comp.
#[must_use]
pub fn definition(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
) -> [u8; 32] {
    use crate::fx::points::stepped::sim::{feed_place, feed_shape};
    let mut h = blake3::Hasher::new_derive_key("lumit smoke walk 2");
    let _ = serde_json::to_writer(&mut h, &comp.frame_rate);
    let g = grid(doc, comp, layer, inst);
    for c in g.cells {
        h.update(&c.to_le_bytes());
    }
    h.update(&g.cell_px.to_le_bytes());
    // A Fit to comp model an emitter or solid reads goes by the comp's size.
    h.update(&comp.width.to_le_bytes());
    h.update(&comp.height.to_le_bytes());
    for p in inst
        .params
        .iter()
        .filter(|p| !LOOK_ROWS.contains(&p.id.as_str()))
    {
        let _ = serde_json::to_writer(&mut h, p);
    }
    let _ = serde_json::to_writer(&mut h, &layer.start_offset);
    // A wire into any of its rows, and what feeds it.
    let _ = serde_json::to_writer(&mut h, &layer.graph);
    feed_shape(doc, layer, &mut h);
    let mut hits = solid_layers(comp, inst, layer.id).peekable();
    let trails = matches!(inst.param("trail"), Some(EffectValue::Bool(true)));
    if hits.peek().is_some() || trails {
        feed_place(comp, layer, &mut h);
    }
    for l in hits {
        feed_shape(doc, l, &mut h);
        feed_place(comp, l, &mut h);
    }
    if let Some(source) = emit_layer(comp, inst) {
        feed_place(comp, layer, &mut h);
        feed_shape(doc, source, &mut h);
        feed_place(comp, source, &mut h);
        // Particles' own walk, by what names it.
        for e in source
            .effects
            .iter()
            .filter(|e| e.effect.match_name == "particulate")
        {
            h.update(&crate::fx::points::stepped::sim::definition(
                doc, comp, source, e,
            ));
        }
    }
    *h.finalize().as_bytes()
}

/// What a step reads, each row reduced once, in cells and seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Physics {
    pub steps: u32,
    /// The air's speed, cells a second.
    pub wind: [f32; 3],
    /// How fast speed settles to it, each second.
    pub settle: f32,
    pub emit_kind: u32,
    /// Density each second, the heat and the fuel where it emits.
    pub emit: [f32; 3],
    /// The floor patch's middle and reach, cells.
    pub patch_middle: [f32; 3],
    pub patch_reach: [f32; 3],
    /// The upward push of heat, the pull of density, the curl's push, and the
    /// uneven shove gas leaves the emitter with.
    pub push: f32,
    pub pull: f32,
    pub curl: f32,
    pub kick: f32,
    /// The push that breaks gas up, and how many cells across its patches are.
    pub turbulence: f32,
    pub patch: f32,
    /// How much faster than that the hottest gas cools, and how broken up fuel
    /// arrives, nought to one.
    pub radiate: f32,
    pub tongues: f32,
    /// How fast density goes and heat fades, each second.
    pub thin: f32,
    pub cool: f32,
    /// How fast fuel burns each second, and the heat and density a unit gives.
    pub burn_rate: f32,
    pub burn_heat: f32,
    pub burn_soot: f32,
    /// The heat fuel catches at, and how much burning swells the gas.
    pub ignite: f32,
    pub swell: f32,
}

/// The heat a flame starts to glow at.
pub const FLAME_STARTS: f32 = 0.33;

/// `rows` on a grid reduced to what a step reads. Nought for what is not a
/// number.
#[must_use]
pub fn physics(rows: &Smoke, g: &Grid) -> Physics {
    let real = |v: f32| if v.is_finite() { v } else { 0.0 };
    let share = |v: f32| real(v).max(0.0) / 100.0;
    let [nx, ny, nz] = g.cells.map(|c| c as f32);
    let longest = nx.max(ny).max(nz);
    let per_px = 1.0 / g.cell_px.max(1e-3);
    let across = share(rows.emit_size).min(1.0) * nx.min(nz) * 0.5;
    let up = (across * 0.6).clamp(1.5, ny * 0.08);
    let fades = share(rows.dissipation);
    Physics {
        steps: rows.sub_steps.clamp(1, 8) as u32,
        wind: [
            real(rows.wind_across) * per_px,
            real(rows.wind_vertical) * per_px,
            real(rows.wind_depth) * per_px,
        ],
        settle: 1.0,
        emit_kind: rows.emit_from.min(2),
        emit: [
            share(rows.density) * 3.0,
            share(rows.temperature) * 0.8,
            share(rows.fuel),
        ],
        patch_middle: [nx * 0.5, ny - up * 0.6 - 1.0, nz * 0.5],
        patch_reach: [across.max(1e-3), up, across.max(1e-3)],
        // Warm smoke crosses a box in a few seconds, whatever its cells.
        push: (real(rows.buoyancy) / 100.0).clamp(-4.0, 16.0) * longest * 1.2,
        pull: longest * 0.01,
        curl: share(rows.vorticity) * longest * 0.16,
        kick: longest * 0.6,
        turbulence: share(rows.turbulence).min(4.0) * longest * 2.6,
        patch: (longest / 25.0).max(3.0),
        radiate: 3.5,
        tongues: 1.0,
        thin: fades * 1.5,
        cool: 0.7 + fades * 0.7,
        burn_rate: 4.0,
        burn_heat: 1.6,
        burn_soot: 0.03,
        ignite: 0.2,
        swell: share(rows.expansion),
    }
}

/// How `rows` look on a box whose longest side is `longest` comp pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub thickness: f32,
    pub smoke: [f32; 3],
    pub shadow: f32,
    pub fire_low: [f32; 3],
    pub fire_high: [f32; 3],
    pub fire_gain: f32,
    pub fire_glow: f32,
    pub detail: f32,
}

/// How thick a density of one is across the box's longest side, and how much
/// light a full flame gives across it.
const THICK_ACROSS: f32 = 15.0;
const FLAME_ACROSS: f32 = 6.0;

/// How strongly flames light the gas round them.
const GLOW: f32 = 0.05;

/// How many cells the march pushes gas about by, for detail finer than a cell.
const DETAIL: f32 = 2.5;

/// `rows`' look. The layer's Opacity thins the smoke.
#[must_use]
pub fn look(rows: &Smoke, longest: f32, opacity: f32) -> Look {
    let rgb =
        |c: [f32; 4]| [c[0], c[1], c[2]].map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    let per = 1.0 / longest.max(1.0);
    let bright = if rows.fire_brightness.is_finite() {
        rows.fire_brightness.max(0.0) / 100.0
    } else {
        0.0
    };
    Look {
        thickness: THICK_ACROSS * per * opacity.clamp(0.0, 1.0),
        smoke: rgb(rows.smoke_colour),
        shadow: (rows.shadow_strength / 100.0).clamp(0.0, 1.0),
        fire_low: rgb(rows.fire_colour),
        fire_high: rgb(rows.fire_core),
        fire_gain: FLAME_ACROSS * per * bright * opacity.clamp(0.0, 1.0),
        fire_glow: GLOW * bright,
        detail: DETAIL,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn comp() -> Composition {
        use crate::time::{Duration, FrameRate, Rational};
        Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: uuid::Uuid::now_v7(),
            name: "Smoke".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(30, 1).unwrap()),
            background: crate::model::LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// What it looks like keeps every state, and moving the box too, while what
    /// the gas does renames the walk.
    #[test]
    fn looks_and_moves_keep_the_walk_and_physics_rename_it() {
        let e = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
        for id in LOOK_ROWS {
            assert!(e.params.iter().any(|p| p.id == *id), "no row {id}");
        }
        let layer = crate::edits::base_layer(
            "Smoke".into(),
            LayerKind::Null,
            crate::time::Rational::new(1, 1).unwrap(),
            Default::default(),
        );
        let (mut c, doc) = (comp(), Document::new());
        c.layers = vec![layer.clone()];
        let name = |e: &EffectInstance, l: &Layer| definition(&doc, &c, l, e);
        let walk = name(&e, &layer);
        let set = |id: &str, v: f64| {
            let mut e = e.clone();
            e.params.iter_mut().find(|p| p.id == id).unwrap().value =
                EffectValue::Float(crate::anim::Property::fixed(v));
            e
        };
        assert_eq!(walk, name(&set("fire_brightness", 9.0), &layer));
        assert_ne!(walk, name(&set("buoyancy", 9.0), &layer));
        let mut moved = layer.clone();
        moved.transform.position_x = crate::anim::Property::fixed(400.0);
        assert_eq!(walk, name(&e, &moved));
        let mut scaled = layer.clone();
        scaled.transform.scale_x = crate::anim::Property::fixed(50.0);
        assert_ne!(walk, name(&e, &scaled), "a different shape of grid");
    }

    /// A preset's value off its row's range, or one that reaches the walk as
    /// not a number, is wrong pixels in every frame after.
    #[test]
    fn every_preset_stays_in_range_and_gives_finite_numbers() {
        use crate::fx::presets::{apply, PresetValue};
        use crate::fx::schema::ParamKind;
        let def = crate::fx::builtins::def(MATCH_NAME).unwrap();
        let (c, doc) = (comp(), Document::new());
        for preset in def.presets() {
            for (id, value) in preset.values {
                let row = def.schema().params.iter().find(|p| p.id == *id).unwrap();
                let ParamKind::Slider { range, .. } = row.kind else {
                    continue;
                };
                let inside = |v: f64| v >= range.0 && v <= range.1;
                match value {
                    PresetValue::Float(v) => assert!(inside(*v), "{id}"),
                    PresetValue::KeysOut(k) | PresetValue::Keys(k) => {
                        assert!(k.iter().all(|(_, v)| inside(*v)), "{id}");
                    }
                    _ => {}
                }
            }
            let mut e = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
            let zero = crate::time::Rational::new(0, 1).unwrap();
            apply(&mut e, preset, zero, (1920.0, 1080.0));
            let context = std::sync::Arc::new(crate::expression::ExpressionContext {
                document: std::sync::Arc::new(doc.clone()),
                comp: None,
                layer: None,
                comp_time: 0.0,
                current_depth: 0,
                inputs: None,
            });
            let stack = crate::fx::resolved::resolve_instance(
                &e,
                &Default::default(),
                0.0,
                2000.0,
                1.0,
                &crate::fx::markers::MarkerContext::NONE,
                context,
            );
            let rows = Smoke::read(stack.get(0).unwrap().params);
            let layer = crate::edits::base_layer(
                "Smoke".into(),
                LayerKind::Null,
                crate::time::Rational::new(1, 1).unwrap(),
                Default::default(),
            );
            let g = grid(&doc, &c, &layer, &e);
            let p = physics(&rows, &g);
            let l = look(&rows, 512.0, 1.0);
            let numbers = [
                p.push,
                p.curl,
                p.turbulence,
                p.thin,
                p.cool,
                p.swell,
                l.thickness,
                l.fire_gain,
            ];
            assert!(numbers.iter().all(|n| n.is_finite()), "{}", preset.name);
        }
    }

    /// The longest side gets the row's cells and the rest keep the box's shape.
    #[test]
    fn a_grid_keeps_the_shape_of_its_box() {
        let e = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
        let layer = crate::edits::base_layer(
            "Smoke".into(),
            LayerKind::Null,
            crate::time::Rational::new(1, 1).unwrap(),
            Default::default(),
        );
        let g = grid(&Document::new(), &comp(), &layer, &e);
        assert_eq!(g.cells, [128, 72, 72]);
        assert_eq!(g.cell_px, 15.0);
    }
}
