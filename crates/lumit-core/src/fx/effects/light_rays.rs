use crate::fx::{
    cpu, params::ParamId, params::Params, params::Value, registry::EffectDef,
    registry::EffectMetadata, registry::Pressed, registry::ResolveCx, schema::EffectSchema,
    schema::EnabledCond, schema::EnabledWhen, schema::ParamGroup,
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

pub const PASSES: usize = 4;
pub const TAPS: usize = 8;
pub const FALL: f32 = 1.5;
pub const RAY_GAIN: f32 = 1.5;
pub const TABLE: usize = 1024;

pub const COLOURISE_NONE: u32 = 0;
pub const COLOURISE_ONE: u32 = 1;
pub const COLOURISE_THREE: u32 = 2;
pub const COLOURISE_FIVE: u32 = 3;

pub const COLOURISE_OPTIONS: &[&str] = &[
    "None",
    "One colour",
    "3-colour gradient",
    "5-colour gradient",
    "Ember",
    "Rust",
    "Acid",
    "Lagoon",
    "Arc",
    "Orchid",
    "Plum",
    "Dusk",
    "Rose",
    "Twilight",
    "Dawn",
    "Isotope",
    "Thermal",
];

const FIRST_LOOK: u32 = 4;
const FIRST_FIVE_LOOK: u32 = 14;

const LOOKS: &[&[[u8; 3]]] = &[
    &[[240, 214, 162], [182, 103, 2], [69, 3, 0]],
    &[[255, 205, 161], [181, 85, 1], [57, 14, 0]],
    &[[207, 242, 150], [102, 181, 2], [1, 56, 0]],
    &[[150, 253, 247], [5, 181, 94], [0, 56, 1]],
    &[[196, 243, 245], [65, 129, 168], [0, 1, 55]],
    &[[243, 203, 243], [129, 65, 168], [1, 0, 55]],
    &[[254, 188, 221], [163, 71, 139], [54, 0, 56]],
    &[[229, 220, 219], [136, 98, 129], [39, 19, 39]],
    &[[243, 207, 242], [168, 65, 129], [55, 1, 1]],
    &[[253, 252, 207], [158, 63, 38], [45, 1, 57]],
    &[
        [245, 220, 210],
        [191, 135, 135],
        [145, 88, 125],
        [55, 47, 101],
        [13, 13, 45],
    ],
    &[
        [81, 161, 15],
        [59, 147, 7],
        [2, 103, 8],
        [1, 57, 7],
        [1, 13, 3],
    ],
    &[
        [255, 10, 5],
        [225, 115, 5],
        [80, 179, 7],
        [15, 95, 27],
        [3, 0, 29],
    ],
];

pub const BASED_ON_OPTIONS: &[&str] = &[
    "Lightness",
    "Luminance",
    "Alpha",
    "Alpha edges",
    "Red",
    "Green",
    "Blue",
];
pub const BASED_ON_ALPHA_EDGES: u32 = 3;

pub const BLEND_OPTIONS: &[&str] = &[
    "None",
    "Normal",
    "Add",
    "Screen",
    "Lighten",
    "Lighter colour",
    "Colour dodge",
    "Linear light",
    "Overlay",
    "Soft light",
    "Hard light",
    "Multiply",
    "Difference",
];
const BLEND_MODES: [u32; 13] = [0, 0, 6, 8, 7, 10, 9, 14, 11, 12, 13, 2, 18];
pub const BLEND_NONE: u32 = 0;
pub const BLEND_NORMAL: u32 = 1;
pub const BLEND_ADD: u32 = 2;

const THREE_AND_FIVE: &[u32] = &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
const FIVE: &[u32] = &[3, 14, 15, 16];

pub const LIGHT_RAYS_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Pre-process",
        params: &["threshold", "use_mask", "mask_radius", "mask_feather"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Shimmer",
        params: &[
            "shimmer_amount",
            "shimmer_detail",
            "shimmer_phase",
            "source_affects_shimmer",
            "shimmer_radius",
            "reduce_flicker",
            "use_loop",
            "revolutions",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["colour"],
        collapsed: false,
        visible_when: Some(("colourise", &[COLOURISE_ONE])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["highlights"],
        collapsed: false,
        visible_when: Some(("colourise", THREE_AND_FIVE)),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["mid_high"],
        collapsed: false,
        visible_when: Some(("colourise", FIVE)),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["midtones"],
        collapsed: false,
        visible_when: Some(("colourise", THREE_AND_FIVE)),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["mid_low"],
        collapsed: false,
        visible_when: Some(("colourise", FIVE)),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["shadows"],
        collapsed: false,
        visible_when: Some(("colourise", THREE_AND_FIVE)),
        visible_when_lens_elements: None,
    },
];

pub const LIGHT_RAYS_ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "mask_radius",
        on: "use_mask",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "mask_feather",
        on: "use_mask",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "shimmer_radius",
        on: "source_affects_shimmer",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "reduce_flicker",
        on: "source_affects_shimmer",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "revolutions",
        on: "use_loop",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "based_on",
        on: "colourise",
        cond: EnabledCond::ChoiceIsNot(COLOURISE_NONE),
    },
    EnabledWhen {
        param: "edge_thickness",
        on: "based_on",
        cond: EnabledCond::ChoiceIs(BASED_ON_ALPHA_EDGES),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "light_rays",
    label = "Light rays",
    version = 1,
    category = Stylise,
    cost = Moderate,
    roi = FullFrame,
    groups = LIGHT_RAYS_GROUPS,
    enabled_when = LIGHT_RAYS_ENABLED_WHEN,
)]
pub struct LightRays {
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub threshold: f32,

    #[toggle(label = "Use mask", default = false)]
    pub use_mask: bool,

    #[slider(
        label = "Mask radius",
        min = 0.0,
        max = 2000.0,
        default = 200.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub mask_radius: f32,

    #[slider(
        label = "Mask feather",
        min = 0.0,
        max = 1000.0,
        default = 50.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub mask_feather: f32,

    #[slider(
        label = "Source point X",
        min = 0.0,
        max = 3840.0,
        default = 960.0,
        unit = Px
    )]
    pub source_x: f32,

    #[slider(
        label = "Source point Y",
        min = 0.0,
        max = 2160.0,
        default = 540.0,
        unit = Px
    )]
    pub source_y: f32,

    #[slider(
        label = "Ray length",
        min = 0.0,
        max = 10.0,
        default = 4.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Raw
    )]
    pub ray_length: f32,

    #[toggle(label = "Extend beyond layer", default = false)]
    pub extend_beyond_layer: bool,

    #[slider(
        label = "Amount",
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 200.0,
        unit = Percent
    )]
    pub shimmer_amount: f32,

    #[slider(
        label = "Detail",
        min = 1.0,
        max = 100.0,
        default = 10.0,
        hard_min = 1.0,
        hard_max = 400.0,
        unit = Raw
    )]
    pub shimmer_detail: f32,

    #[dial(label = "Phase", default = 0.0, step = 15.0)]
    pub shimmer_phase: f32,

    #[toggle(label = "Source point affects shimmer", default = false)]
    pub source_affects_shimmer: bool,

    #[slider(
        label = "Radius",
        min = 1.0,
        max = 1000.0,
        default = 100.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub shimmer_radius: f32,

    #[toggle(label = "Reduce flicker", default = false)]
    pub reduce_flicker: bool,

    #[toggle(label = "Use loop", default = false)]
    pub use_loop: bool,

    #[counter(
        label = "Revolutions in loop",
        min = 1,
        max = 20,
        default = 1,
        hard_min = 1,
        hard_max = 1000,
        unit = Raw
    )]
    pub revolutions: i32,

    #[slider(
        label = "Boost light",
        min = 0.0,
        max = 10.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 1000.0,
        unit = Raw
    )]
    pub boost_light: f32,

    #[choice(label = "Colourise", options = *COLOURISE_OPTIONS, default = 0)]
    pub colourise: u32,

    #[colour(label = "Colour", default = [1.0, 0.8, 0.45, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    #[colour(label = "Highlights", default = [0.871, 0.672, 0.361, 1.0], max = 4.0)]
    pub highlights: [f32; 4],

    #[colour(label = "Mid-high", default = [0.521, 0.242, 0.242, 1.0], max = 4.0)]
    pub mid_high: [f32; 4],

    #[colour(label = "Midtones", default = [0.468, 0.136, 0.001, 1.0], max = 4.0)]
    pub midtones: [f32; 4],

    #[colour(label = "Mid-low", default = [0.038, 0.028, 0.130, 1.0], max = 4.0)]
    pub mid_low: [f32; 4],

    #[colour(label = "Shadows", default = [0.060, 0.001, 0.0, 1.0], max = 4.0)]
    pub shadows: [f32; 4],

    #[choice(label = "Based on", options = *BASED_ON_OPTIONS, default = 0)]
    pub based_on: u32,

    #[slider(
        label = "Edge thickness",
        min = 0.0,
        max = 50.0,
        default = 4.0,
        hard_min = 0.0,
        hard_max = 500.0,
        unit = Px
    )]
    pub edge_thickness: f32,

    #[slider(
        label = "Source opacity",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub source_opacity: f32,

    #[slider(
        label = "Rays opacity",
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 400.0,
        unit = Percent
    )]
    pub rays_opacity: f32,

    #[choice(label = "Blend mode", options = *BLEND_OPTIONS, default = 2)]
    pub rays_blend: u32,

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

#[derive(Debug, Clone, PartialEq)]
pub struct LightRaysParams {
    pub source: [f32; 2],
    pub lengths: Vec<f32>,
    pub threshold: f32,
    pub mask: [f32; 2],
    pub colourise: u32,
    pub based_on: u32,
    pub edge: f32,
    pub stops: [[f32; 3]; 7],
    pub at: [f32; 7],
    pub count: u32,
    pub gain: f32,
    pub source_opacity: f32,
    pub rays_opacity: f32,
    pub blend: u32,
    pub mix: f32,
}

impl LightRaysParams {
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.mix == 0.0
    }

    #[must_use]
    pub fn weight(k: usize, j: usize) -> f32 {
        let step = FALL * (TAPS.pow(k as u32) as f32) / (TAPS.pow(PASSES as u32) as f32);
        (-(j as f32) * step).exp()
    }

    #[must_use]
    pub fn step(k: usize, len: f32) -> f32 {
        len * (TAPS.pow(k as u32) as f32) / (TAPS.pow(PASSES as u32) as f32)
    }
}

fn linear(v: u8) -> f32 {
    let c = f32::from(v) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[must_use]
pub fn look(choice: u32) -> Option<Vec<[f32; 4]>> {
    let at = choice.checked_sub(FIRST_LOOK)? as usize;
    Some(
        LOOKS
            .get(at)?
            .iter()
            .map(|c| [linear(c[0]), linear(c[1]), linear(c[2]), 1.0])
            .collect(),
    )
}

const FIVE_ROWS: [&str; 5] = ["highlights", "mid_high", "midtones", "mid_low", "shadows"];
const THREE_ROWS: [&str; 3] = ["highlights", "midtones", "shadows"];

#[must_use]
pub fn mode_of(choice: u32) -> u32 {
    match choice {
        0..=3 => choice,
        c if c < FIRST_FIVE_LOOK => COLOURISE_THREE,
        _ => COLOURISE_FIVE,
    }
}

fn hash(mut v: u32) -> u32 {
    v = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let w = ((v >> ((v >> 28) + 4)) ^ v).wrapping_mul(277_803_737);
    (w >> 22) ^ w
}

fn corner(c: [i64; 4], period: [i64; 4], octave: u32) -> f32 {
    let mut h = hash(octave.wrapping_mul(0x9E37_79B9));
    for (v, p) in c.iter().zip(period) {
        let v = if p > 0 { v.rem_euclid(p) } else { *v };
        h = hash(h ^ (v as u32));
    }
    h as f32 / u32::MAX as f32
}

fn noise4(x: [f64; 4], period: [i64; 4], octave: u32) -> f32 {
    let base = x.map(|v| v.floor() as i64);
    let t = [0, 1, 2, 3].map(|i| {
        let f = (x[i] - x[i].floor()) as f32;
        f * f * f * (f * (f * 6.0 - 15.0) + 10.0)
    });
    let mut sum = 0.0;
    for k in 0..16u32 {
        let mut weight = 1.0;
        let mut c = base;
        for (d, cd) in c.iter_mut().enumerate() {
            if k >> d & 1 == 1 {
                *cd += 1;
                weight *= t[d];
            } else {
                weight *= 1.0 - t[d];
            }
        }
        sum += weight * corner(c, period, octave);
    }
    sum
}

impl LightRays {
    pub const DERIVED_DIAG: ParamId = ParamId::new("derived.diag");

    #[must_use]
    pub fn reach(self, diag: f32) -> f32 {
        if !self.extend_beyond_layer || self.mix <= 0.0 || diag <= 0.0 {
            return 0.0;
        }
        let shimmer = 1.0 + self.shimmer_amount.max(0.0) / 100.0;
        let len = (1.0 + self.ray_length.max(0.0)).ln() * shimmer;
        let far = self.source_x.hypot(self.source_y) + diag;
        (far * len.exp_m1()).min(diag)
    }

    #[must_use]
    pub fn reach_within(self, diag: f32, picture: [f32; 4]) -> f32 {
        let loose = self.reach(diag);
        if loose <= 0.0 {
            return 0.0;
        }
        let shimmer = 1.0 + self.shimmer_amount.max(0.0) / 100.0;
        let len = (1.0 + self.ray_length.max(0.0)).ln() * shimmer;
        let [left, top, right, bottom] = picture;
        let far = [(left, top), (right, top), (right, bottom), (left, bottom)]
            .iter()
            .map(|&(x, y)| (x - self.source_x).hypot(y - self.source_y))
            .fold(0.0f32, f32::max);
        ((far * len.exp_m1()).ceil() + PASSES as f32 + 1.0).min(loose)
    }

    #[must_use]
    pub fn packed(self, w: u32, h: u32) -> LightRaysParams {
        let (wf, hf) = (w as f32, h as f32);
        let source = [
            self.source_x.clamp(-wf, 2.0 * wf),
            self.source_y.clamp(-hf, 2.0 * hf),
        ];
        let len = (1.0 + self.ray_length.max(0.0)).ln();
        let lengths = self.lengths(len);
        let colourise = mode_of(self.colourise.min(COLOURISE_OPTIONS.len() as u32 - 1));
        let rgb = |c: [f32; 4]| [c[0].max(0.0), c[1].max(0.0), c[2].max(0.0)];
        let (stops, at, count) = match colourise {
            COLOURISE_ONE => {
                let mut stops = [[0.0; 3]; 7];
                stops[1] = rgb(self.colour);
                let mut at = [1.0; 7];
                at[0] = 0.0;
                (stops, at, 2)
            }
            COLOURISE_FIVE => (
                [
                    [0.0; 3],
                    rgb(self.shadows),
                    rgb(self.mid_low),
                    rgb(self.midtones),
                    rgb(self.mid_high),
                    rgb(self.highlights),
                    [1.0; 3],
                ],
                [0.0, 0.1, 0.25, 0.4, 0.6, 0.8, 1.0],
                7,
            ),
            _ => (
                [
                    [0.0; 3],
                    rgb(self.shadows),
                    rgb(self.midtones),
                    rgb(self.highlights),
                    [1.0; 3],
                    [1.0; 3],
                    [1.0; 3],
                ],
                [0.0, 0.1, 0.4, 0.8, 1.0, 1.0, 1.0],
                5,
            ),
        };
        let blend = self.rays_blend.min(BLEND_OPTIONS.len() as u32 - 1);
        LightRaysParams {
            source,
            lengths,
            threshold: (self.threshold / 100.0).clamp(0.0, 1.0),
            mask: if self.use_mask {
                [self.mask_radius.max(0.0), self.mask_feather.max(0.0)]
            } else {
                [-1.0, 0.0]
            },
            colourise,
            based_on: self.based_on.min(BASED_ON_OPTIONS.len() as u32 - 1),
            edge: self.edge_thickness.max(0.0),
            stops,
            at,
            count,
            gain: RAY_GAIN * (1.0 + self.boost_light.max(0.0)),
            source_opacity: (self.source_opacity / 100.0).clamp(0.0, 1.0),
            rays_opacity: (self.rays_opacity / 100.0).max(0.0),
            blend: match blend {
                BLEND_NONE | BLEND_NORMAL => blend,
                m => BLEND_MODES[m as usize] + 2,
            },
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }

    fn lengths(self, len: f32) -> Vec<f32> {
        let amount = self.shimmer_amount.max(0.0) / 100.0;
        if amount == 0.0 {
            return vec![len; TABLE];
        }
        let cells = self.shimmer_detail.max(1.0).round() as i64;
        let phase = f64::from(self.shimmer_phase) / 360.0 * 2.0;
        let looped = self
            .use_loop
            .then(|| i64::from(self.revolutions.max(1)) * 2);
        let flicker = self.source_affects_shimmer && self.reduce_flicker;
        let travel = if self.source_affects_shimmer {
            let r = f64::from(self.shimmer_radius.max(1.0)) * if flicker { 2.0 } else { 1.0 };
            [f64::from(self.source_x) / r, f64::from(self.source_y) / r]
        } else {
            [0.0; 2]
        };
        let octaves = if flicker { 1 } else { 3 };
        (0..TABLE)
            .map(|i| {
                let turn = i as f64 / TABLE as f64;
                let (mut sum, mut total, mut amp) = (0.0f32, 0.0f32, 1.0f32);
                for o in 0..octaves {
                    let f = 1i64 << o;
                    let x = [
                        turn * (cells * f) as f64,
                        phase * f as f64,
                        travel[0] * f as f64,
                        travel[1] * f as f64,
                    ];
                    let period = [cells * f, looped.map_or(0, |p| p * f), 0, 0];
                    sum += amp * noise4(x, period, o);
                    total += amp;
                    amp *= 0.5;
                }
                let n = sum / total;
                len * (1.0 + amount * (2.0 * n - 1.0)).max(0.0)
            })
            .collect()
    }
}

#[must_use]
pub fn length_at(lengths: &[f32], d: [f32; 2]) -> f32 {
    let turn = d[1].atan2(d[0]) / std::f32::consts::TAU;
    let x = (turn - turn.floor()) * TABLE as f32;
    let i = (x.floor() as usize).min(TABLE - 1);
    let f = x - x.floor();
    let a = lengths[i];
    let b = lengths[(i + 1) % TABLE];
    a + (b - a) * f
}

fn lightness(c: [f32; 3]) -> f32 {
    0.5 * (c[0].max(c[1]).max(c[2]) + c[0].min(c[1]).min(c[2]))
}

#[must_use]
pub fn light_at(src: &[f32], w: u32, h: u32, x: u32, y: u32, p: &LightRaysParams) -> [f32; 4] {
    let i = ((y * w + x) * 4) as usize;
    let c = [src[i], src[i + 1], src[i + 2], src[i + 3]];
    let pos = [x as f32 + 0.5, y as f32 + 0.5];
    let mut k = 1.0f32;
    if p.threshold > 0.0 {
        let l = lightness([c[0], c[1], c[2]]).max(0.0).powf(1.0 / 2.2);
        k = (l - p.threshold).max(0.0) / l.max(1e-5);
    }
    if p.mask[0] >= 0.0 {
        let d = (pos[0] - p.source[0]).hypot(pos[1] - p.source[1]);
        let r = p.mask[0];
        let f = p.mask[1];
        k *= if f > 0.0 {
            let t = ((d - r) / f).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        } else if d <= r {
            1.0
        } else {
            0.0
        };
    }
    if p.colourise == COLOURISE_NONE {
        return c.map(|v| v * k);
    }
    let a = c[3];
    let u = if a > 0.0 {
        [c[0] / a, c[1] / a, c[2] / a]
    } else {
        [0.0; 3]
    };
    let (v, alpha) = match p.based_on {
        0 => (lightness(u) * a, a),
        1 => ((0.2126 * u[0] + 0.7152 * u[1] + 0.0722 * u[2]) * a, a),
        2 => (a, a),
        BASED_ON_ALPHA_EDGES => {
            let mut low = a;
            for t in 0..8 {
                let angle = t as f32 * std::f32::consts::FRAC_PI_4;
                let at = cpu::bilinear_edge(
                    src,
                    w,
                    h,
                    pos[0] + p.edge * angle.cos(),
                    pos[1] + p.edge * angle.sin(),
                    0,
                );
                low = low.min(at[3]);
            }
            let e = (a - low).clamp(0.0, 1.0);
            (e, e)
        }
        4 => (u[0] * a, a),
        5 => (u[1] * a, a),
        _ => (u[2] * a, a),
    };
    let v = v * k;
    [v, v, v, alpha * k]
}

#[must_use]
pub fn gradient(p: &LightRaysParams, t: f32) -> [f32; 3] {
    let n = p.count as usize;
    for s in 1..n {
        if t <= p.at[s] {
            let span = p.at[s] - p.at[s - 1];
            let f = if span > 0.0 {
                ((t - p.at[s - 1]) / span).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let (a, b) = (p.stops[s - 1], p.stops[s]);
            return [
                a[0] + (b[0] - a[0]) * f,
                a[1] + (b[1] - a[1]) * f,
                a[2] + (b[2] - a[2]) * f,
            ];
        }
    }
    p.stops[n - 1]
}

#[must_use]
pub fn rays_of(r: [f32; 4], p: &LightRaysParams) -> [f32; 4] {
    let g = p.gain;
    if p.colourise == COLOURISE_NONE {
        return [r[0] * g, r[1] * g, r[2] * g, (r[3] * g).min(1.0)];
    }
    let v = (r[0] * g).max(0.0);
    let a = (r[3] * g).clamp(0.0, 1.0);
    let t = v.min(1.0).powf(1.0 / 2.2);
    let c = gradient(p, t);
    let k = v.max(1.0) * a;
    [c[0] * k, c[1] * k, c[2] * k, a]
}

#[must_use]
pub fn combine(d: [f32; 4], s: [f32; 4], blend: u32) -> [f32; 4] {
    match blend {
        BLEND_NONE => s,
        BLEND_NORMAL => {
            let k = 1.0 - s[3].min(1.0);
            [
                s[0] + d[0] * k,
                s[1] + d[1] * k,
                s[2] + d[2] * k,
                s[3].min(1.0) + d[3] * k,
            ]
        }
        m => {
            let sa = s[3].min(1.0);
            let un = |c: [f32; 4], a: f32| {
                if a > 1e-6 {
                    [c[0] / a, c[1] / a, c[2] / a, 1.0]
                } else {
                    [0.0, 0.0, 0.0, 1.0]
                }
            };
            let b = cpu::blend_pixel(m - 2, un(d, d[3]), un(s, sa));
            let mut o = [0.0f32; 4];
            for c in 0..3 {
                o[c] = s[c] * (1.0 - d[3]) + d[c] * (1.0 - sa) + sa * d[3] * b[c];
            }
            o[3] = sa + d[3] - sa * d[3];
            o
        }
    }
}

fn gather(src: &[f32], w: u32, h: u32, k: usize, p: &LightRaysParams) -> Vec<f32> {
    let mut out = vec![0.0f32; src.len()];
    let weights: Vec<f32> = (0..TAPS).map(|j| LightRaysParams::weight(k, j)).collect();
    let total: f32 = weights.iter().sum();
    for y in 0..h {
        for x in 0..w {
            let pos = [x as f32 + 0.5, y as f32 + 0.5];
            let d = [pos[0] - p.source[0], pos[1] - p.source[1]];
            let step = LightRaysParams::step(k, length_at(&p.lengths, d));
            let mut sum = [0.0f32; 4];
            for (j, wt) in weights.iter().enumerate() {
                let s = (-(j as f32) * step).exp();
                let at = cpu::bilinear_edge(
                    src,
                    w,
                    h,
                    p.source[0] + d[0] * s,
                    p.source[1] + d[1] * s,
                    0,
                );
                for c in 0..4 {
                    sum[c] += at[c] * wt;
                }
            }
            let i = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                out[i + c] = half(sum[c] / total);
            }
        }
    }
    out
}

fn half(v: f32) -> f32 {
    half::f16::from_f32(v).to_f32()
}

pub fn light_rays(rgba: &mut [f32], w: u32, h: u32, p: &LightRaysParams) {
    if p.is_identity() || w == 0 || h == 0 {
        return;
    }
    let original = rgba.to_vec();
    let mut light = vec![0.0f32; rgba.len()];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let l = light_at(&original, w, h, x, y, p);
            for c in 0..4 {
                light[i + c] = half(l[c]);
            }
        }
    }
    for k in 0..PASSES {
        light = gather(&light, w, h, k, p);
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let o = [
            original[i * 4],
            original[i * 4 + 1],
            original[i * 4 + 2],
            original[i * 4 + 3],
        ];
        let r = rays_of(
            [
                light[i * 4],
                light[i * 4 + 1],
                light[i * 4 + 2],
                light[i * 4 + 3],
            ],
            p,
        );
        let mut s = r.map(|v| v * p.rays_opacity);
        s[3] = s[3].min(1.0);
        let d = o.map(|v| v * p.source_opacity);
        let out = combine(d, s, p.blend);
        for c in 0..4 {
            px[c] = o[c] + (out[c] - o[c]) * p.mix;
        }
    }
}

fn choice_of(e: &EffectInstance) -> u32 {
    match e.param("colourise") {
        Some(EffectValue::Choice(v)) => *v,
        _ => COLOURISE_NONE,
    }
}

fn keyed(e: &EffectInstance, id: &str) -> bool {
    match e.param(id) {
        Some(EffectValue::Colour(c)) => c.iter().any(|p| p.is_animated()),
        _ => false,
    }
}

#[must_use]
pub fn settle(before: Option<&EffectInstance>, after: &EffectInstance) -> Option<Pressed> {
    let chosen = choice_of(after);
    if before.map(choice_of) != Some(chosen) {
        let colours = look(chosen)?;
        let rows: &[&'static str] = if colours.len() == 5 {
            &FIVE_ROWS
        } else {
            &THREE_ROWS
        };
        let rows = rows
            .iter()
            .zip(colours)
            .filter(|(id, _)| !keyed(after, id))
            .map(|(id, c)| (*id, Value::Colour(c)))
            .collect();
        return Some(Pressed { rows, memory: None });
    }
    let before = before?;
    if chosen < FIRST_LOOK {
        return None;
    }
    let edited = FIVE_ROWS
        .iter()
        .any(|id| before.param(id) != after.param(id));
    edited.then(|| Pressed {
        rows: vec![("colourise", Value::Choice(mode_of(chosen)))],
        memory: None,
    })
}

pub struct LightRaysDef;

impl EffectDef for LightRaysDef {
    fn schema(&self) -> &'static EffectSchema {
        &<LightRays as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        light_rays(rgba, w, h, &LightRays::read(p).packed(w, h));
    }

    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(LightRays::DERIVED_DIAG, Value::Float(cx.diag_px));
    }

    fn spread_px(&self, p: Params<'_>) -> [f32; 2] {
        let reach = LightRays::read(p).reach(p.float(LightRays::DERIVED_DIAG, 0.0));
        [reach, reach]
    }

    fn spread_px_within(&self, p: Params<'_>, picture: [f32; 4]) -> [f32; 2] {
        let reach = LightRays::read(p).reach_within(p.float(LightRays::DERIVED_DIAG, 0.0), picture);
        [reach, reach]
    }

    fn settle(&self, before: Option<&EffectInstance>, after: &EffectInstance) -> Option<Pressed> {
        settle(before, after)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn every_look_has_three_or_five_colours_and_a_name() {
        assert_eq!(
            COLOURISE_OPTIONS.len() as u32,
            FIRST_LOOK + LOOKS.len() as u32
        );
        for (i, look) in LOOKS.iter().enumerate() {
            let n = if (i as u32) + FIRST_LOOK < FIRST_FIVE_LOOK {
                3
            } else {
                5
            };
            assert_eq!(
                look.len(),
                n,
                "{}",
                COLOURISE_OPTIONS[i + FIRST_LOOK as usize]
            );
        }
        assert_eq!(mode_of(5), COLOURISE_THREE);
        assert_eq!(mode_of(15), COLOURISE_FIVE);
        assert_eq!(mode_of(1), COLOURISE_ONE);
    }

    #[test]
    fn the_fade_splits_into_one_set_of_weights_a_pass() {
        let fine = FALL / (TAPS.pow(PASSES as u32) as f32);
        for k in 0..PASSES {
            for j in 0..TAPS {
                let at = (j * TAPS.pow(k as u32)) as f32;
                let want = (-at * fine).exp();
                assert!((LightRaysParams::weight(k, j) - want).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn the_shimmer_wraps_round_the_circle_and_loops_in_phase() {
        let mut fx = LightRays::read(Params::EMPTY);
        fx.shimmer_amount = 80.0;
        fx.use_loop = true;
        fx.revolutions = 2;
        let a = fx.packed(64, 48).lengths;
        let step = (a[0] - a[TABLE - 1]).abs();
        assert!(step < 0.05, "seam step {step}");
        fx.shimmer_phase = 720.0;
        let b = fx.packed(64, 48).lengths;
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() < 1e-5);
        }
        fx.shimmer_phase = 300.0;
        let c = fx.packed(64, 48).lengths;
        assert!(a.iter().zip(&c).any(|(x, y)| (x - y).abs() > 0.01));
    }

    #[test]
    fn no_ray_length_leaves_the_light_where_it_is() {
        let (w, h) = (8u32, 6u32);
        let img: Vec<f32> = (0..w * h * 4).map(|i| (i % 7) as f32 / 7.0).collect();
        let mut fx = LightRays::read(Params::EMPTY);
        fx.ray_length = 0.0;
        fx.rays_blend = BLEND_NONE;
        let mut out = img.clone();
        light_rays(&mut out, w, h, &fx.packed(w, h));
        for (i, (a, b)) in img.iter().zip(&out).enumerate() {
            let want = if i % 4 == 3 {
                (a * RAY_GAIN).min(1.0)
            } else {
                a * RAY_GAIN
            };
            assert!((want - b).abs() < 4e-3, "{want} {b}");
        }
    }

    #[test]
    fn a_bright_dot_throws_a_ray_away_from_the_source() {
        let (w, h) = (64u32, 16u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 7..9 {
            for x in 20..22 {
                let i = ((y * w + x) * 4) as usize;
                img[i..i + 4].copy_from_slice(&[1.0; 4]);
            }
        }
        let mut fx = LightRays::read(Params::EMPTY);
        fx.source_x = 0.0;
        fx.source_y = 8.0;
        fx.rays_blend = BLEND_NONE;
        let mut out = img.clone();
        light_rays(&mut out, w, h, &fx.packed(w, h));
        let at = |x: u32| out[((8 * w + x) * 4) as usize];
        assert!(at(30) > 0.0, "the ray reaches past the dot");
        assert!(at(30) > at(50), "and fades as it goes");
        assert_eq!(at(10), 0.0, "nothing towards the source");
    }

    #[test]
    fn the_gradient_runs_black_to_shadows_to_highlights() {
        let mut fx = LightRays::read(Params::EMPTY);
        fx.colourise = COLOURISE_THREE;
        let p = fx.packed(64, 48);
        assert_eq!(gradient(&p, 0.0), [0.0; 3]);
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(near(gradient(&p, 0.1), rgb3(fx.shadows)));
        assert!(near(gradient(&p, 0.8), rgb3(fx.highlights)));
        assert_eq!(gradient(&p, 1.0), [1.0; 3]);
    }

    #[test]
    fn extend_beyond_layer_gives_the_rays_room_up_to_a_diagonal() {
        let mut fx = LightRays::read(Params::EMPTY);
        assert_eq!(fx.reach(2203.0), 0.0, "off by default: no room");
        fx.extend_beyond_layer = true;
        assert_eq!(fx.reach(2203.0), 2203.0, "long rays: one diagonal");
        fx.ray_length = 0.05;
        let short = fx.reach(2203.0);
        assert!(
            short > 0.0 && short < 2203.0,
            "short rays: less room ({short})"
        );
        fx.ray_length = 0.0;
        assert_eq!(fx.reach(2203.0), 0.0, "no rays: no room");
    }

    #[test]
    fn the_rays_never_light_past_the_room_reach_within_gives() {
        let (w, h) = (40u32, 24u32);
        let diag = (w as f32).hypot(h as f32);
        let picture = [0.0, 0.0, w as f32, h as f32];
        for (sx, sy) in [(20.0, 12.0), (3.0, 20.0), (-15.0, 8.0), (60.0, -10.0)] {
            for (length, shimmer) in [(0.3, 0.0), (0.3, 100.0), (2.0, 0.0), (0.05, 50.0)] {
                let mut fx = LightRays::read(Params::EMPTY);
                fx.extend_beyond_layer = true;
                fx.threshold = 0.0;
                fx.ray_length = length;
                fx.shimmer_amount = shimmer;
                fx.source_x = sx;
                fx.source_y = sy;
                let loose = fx.reach(diag);
                let tight = fx.reach_within(diag, picture);
                let case = format!("source ({sx}, {sy}), length {length}, shimmer {shimmer}");
                assert!(tight <= loose, "{case}: {tight} > {loose}");
                assert!(tight > 0.0, "{case}: rays with no room");
                let pad = loose.ceil() as u32;
                let (pw, ph) = (w + 2 * pad, h + 2 * pad);
                let mut rgba = vec![0.0f32; (pw * ph * 4) as usize];
                for y in pad..pad + h {
                    for x in pad..pad + w {
                        let i = ((y * pw + x) * 4) as usize;
                        rgba[i..i + 4].copy_from_slice(&[3.0, 3.0, 3.0, 1.0]);
                    }
                }
                let mut moved = fx;
                moved.source_x += pad as f32;
                moved.source_y += pad as f32;
                light_rays(&mut rgba, pw, ph, &moved.packed(pw, ph));
                let room = tight.ceil();
                let (mut lit_outside, mut past) = (0, Vec::new());
                for y in 0..ph {
                    for x in 0..pw {
                        let i = ((y * pw + x) * 4) as usize;
                        if rgba[i..i + 4].iter().all(|v| *v <= 1e-6) {
                            continue;
                        }
                        let (px, py) = (x as f32 - pad as f32, y as f32 - pad as f32);
                        let dx = (-px).max(px + 1.0 - w as f32).max(0.0);
                        let dy = (-py).max(py + 1.0 - h as f32).max(0.0);
                        if dx > 0.0 || dy > 0.0 {
                            lit_outside += 1;
                        }
                        if dx > room || dy > room {
                            past.push((x, y));
                        }
                    }
                }
                assert!(lit_outside > 0, "{case}: the rays should leave the picture");
                assert!(
                    past.is_empty(),
                    "{case}: {} lit past the room of {room}, first {:?}",
                    past.len(),
                    past.first()
                );
                let small = room as u32;
                let (sw, sh) = (w + 2 * small, h + 2 * small);
                let mut tight_rgba = vec![0.0f32; (sw * sh * 4) as usize];
                for y in small..small + h {
                    for x in small..small + w {
                        let i = ((y * sw + x) * 4) as usize;
                        tight_rgba[i..i + 4].copy_from_slice(&[3.0, 3.0, 3.0, 1.0]);
                    }
                }
                let mut placed = fx;
                placed.source_x += small as f32;
                placed.source_y += small as f32;
                light_rays(&mut tight_rgba, sw, sh, &placed.packed(sw, sh));
                let mut worst = 0.0f32;
                for y in 0..sh {
                    for x in 0..sw {
                        let i = ((y * sw + x) * 4) as usize;
                        let j = (((y + pad - small) * pw + x + pad - small) * 4) as usize;
                        for c in 0..4 {
                            worst = worst.max((tight_rgba[i + c] - rgba[j + c]).abs());
                        }
                    }
                }
                assert!(
                    worst < 2e-3,
                    "{case}: the tight room changed a pixel by {worst}"
                );
            }
        }
        let mut fx = LightRays::read(Params::EMPTY);
        fx.extend_beyond_layer = true;
        fx.ray_length = 0.3;
        fx.source_x = 1700.0;
        fx.source_y = 1050.0;
        let (loose, tight) = (
            fx.reach(4061.0),
            fx.reach_within(4061.0, [0.0, 0.0, 3478.0, 2096.0]),
        );
        assert!(tight < loose * 0.4, "{tight} against {loose}");
    }

    #[test]
    fn a_look_fills_the_colours_and_an_edit_puts_the_menu_back() {
        let before = crate::fx::builtins::instantiate("light_rays").unwrap();
        let set = |e: &mut EffectInstance, id: &str, v: EffectValue| {
            if let Some(p) = e.params.iter_mut().find(|p| p.id == id) {
                p.value = v;
            }
        };
        let mut after = before.clone();
        set(&mut after, "colourise", EffectValue::Choice(16));
        let pressed = settle(Some(&before), &after).unwrap();
        let ids: Vec<_> = pressed.rows.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, FIVE_ROWS);
        let mut plain = before.clone();
        set(
            &mut plain,
            "colourise",
            EffectValue::Choice(COLOURISE_THREE),
        );
        assert_eq!(settle(Some(&before), &plain), None);
        let mut ember = before.clone();
        set(&mut ember, "colourise", EffectValue::Choice(4));
        let mut edited = ember.clone();
        let other = edited.param("colour").cloned().unwrap();
        set(&mut edited, "midtones", other);
        let pressed = settle(Some(&ember), &edited).unwrap();
        assert_eq!(
            pressed.rows,
            vec![("colourise", Value::Choice(COLOURISE_THREE))]
        );
        let mut moved = ember.clone();
        set(&mut moved, "use_mask", EffectValue::Bool(true));
        assert_eq!(settle(Some(&ember), &moved), None);
    }

    fn rgb3(c: [f32; 4]) -> [f32; 3] {
        [c[0], c[1], c[2]]
    }

    #[test]
    fn normal_puts_the_rays_over_the_picture_and_add_adds_them() {
        let d = [0.2, 0.3, 0.4, 1.0];
        let s = [0.5, 0.25, 0.0, 0.5];
        assert_eq!(combine(d, s, BLEND_NONE), s);
        let n = combine(d, s, BLEND_NORMAL);
        assert!((n[0] - 0.6).abs() < 1e-6 && (n[3] - 1.0).abs() < 1e-6);
        let a = combine(d, s, BLEND_MODES[BLEND_ADD as usize] + 2);
        for c in 0..3 {
            assert!((a[c] - (d[c] + s[c])).abs() < 1e-5);
        }
    }
}
