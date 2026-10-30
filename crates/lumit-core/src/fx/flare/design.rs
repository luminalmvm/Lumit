//! What a flare design is made of.
//!
//! Every number an element holds lives in one map by key, so the window can
//! edit any of them the same way and a missing key reads as its default from
//! [`controls`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::controls::controls;
use crate::model::EffectInstance;

/// Where a design lives in the instance's `extra`.
pub const EXTRA_KEY: &str = "flare";

/// The file format's version, written as `lumit_flare` in every design.
pub const FORMAT: u32 = 1;

/// The kinds of element, in the order the Add menu lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Glow,
    Rays,
    Star,
    Streak,
    Ring,
    Halo,
    Ghost,
    Ghosts,
    Orbs,
    Dirt,
    Picture,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Kind::Glow,
        Kind::Rays,
        Kind::Star,
        Kind::Streak,
        Kind::Ring,
        Kind::Halo,
        Kind::Ghost,
        Kind::Ghosts,
        Kind::Orbs,
        Kind::Dirt,
        Kind::Picture,
    ];

    /// The kind's name as the window shows it.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Glow => "Glow",
            Kind::Rays => "Rays",
            Kind::Star => "Star",
            Kind::Streak => "Streak",
            Kind::Ring => "Ring",
            Kind::Halo => "Halo",
            Kind::Ghost => "Ghost",
            Kind::Ghosts => "Ghosts",
            Kind::Orbs => "Orbs",
            Kind::Dirt => "Dirt",
            Kind::Picture => "Picture",
        }
    }

    /// Which part of the Add menu and the element list it sits in.
    pub fn place(self) -> Place {
        match self {
            Kind::Glow | Kind::Rays | Kind::Star | Kind::Streak | Kind::Ring | Kind::Halo => {
                Place::OnTheLight
            }
            Kind::Ghost | Kind::Ghosts => Place::AlongTheAxis,
            Kind::Orbs | Kind::Dirt => Place::AcrossTheFrame,
            Kind::Picture => Place::Anywhere,
        }
    }
}

/// The groups of the Add menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    OnTheLight,
    AlongTheAxis,
    AcrossTheFrame,
    Anywhere,
}

impl Place {
    pub fn label(self) -> &'static str {
        match self {
            Place::OnTheLight => "On the light",
            Place::AlongTheAxis => "Along the axis",
            Place::AcrossTheFrame => "Across the frame",
            Place::Anywhere => "Anywhere",
        }
    }
}

/// How an element is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColourMode {
    /// The light's own colour, times the effect's Colour row.
    #[default]
    Light,
    /// The first colour.
    Solid,
    /// Each copy picks one of the colours by its seed.
    Mixed,
    /// From the first colour at the middle to the second at the edge.
    Gradient,
    /// A spectrum across the element.
    Rainbow,
}

impl ColourMode {
    pub const ALL: [ColourMode; 5] = [
        ColourMode::Light,
        ColourMode::Solid,
        ColourMode::Mixed,
        ColourMode::Gradient,
        ColourMode::Rainbow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ColourMode::Light => "Light",
            ColourMode::Solid => "Solid",
            ColourMode::Mixed => "Mixed",
            ColourMode::Gradient => "Gradient",
            ColourMode::Rainbow => "Rainbow",
        }
    }
}

/// An element's colour: the mode and the scene-linear colours it uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Colour {
    #[serde(default)]
    pub mode: ColourMode,
    #[serde(default = "white")]
    pub colours: Vec<[f32; 3]>,
}

fn white() -> Vec<[f32; 3]> {
    vec![[1.0, 1.0, 1.0]]
}

impl Default for Colour {
    fn default() -> Self {
        Self {
            mode: ColourMode::Light,
            colours: white(),
        }
    }
}

fn yes() -> bool {
    true
}

/// One element of a design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub kind: Kind,
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes")]
    pub on: bool,
    #[serde(default)]
    pub colour: Colour,
    /// Every number the element holds, by control key. A key that is missing
    /// reads as its default.
    #[serde(default)]
    pub values: BTreeMap<String, f32>,
}

impl Element {
    /// A fresh element of `kind` with every value at its default, named after
    /// its kind.
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            name: kind.label().to_owned(),
            on: true,
            colour: Colour::default(),
            values: BTreeMap::new(),
        }
    }

    /// The value of `key`, its default when it was never set, or 0 for a key
    /// this kind has no control for. Clamped to the control's hard range.
    pub fn num(&self, key: &str) -> f32 {
        let Some(c) = controls(self.kind).find(|c| c.key == key) else {
            return self.values.get(key).copied().unwrap_or(0.0);
        };
        match self.values.get(key) {
            Some(v) if v.is_finite() => v.clamp(c.hard_min, c.hard_max),
            _ => c.default,
        }
    }

    /// [`Self::num`] as a whole number, for counts and choices.
    pub fn int(&self, key: &str) -> u32 {
        self.num(key).round().max(0.0) as u32
    }

    /// [`Self::num`] as a switch.
    pub fn on(&self, key: &str) -> bool {
        self.num(key) >= 0.5
    }

    pub fn set(&mut self, key: &str, value: f32) {
        self.values.insert(key.to_owned(), value);
    }

    /// The `i`th colour, wrapping, or white when there are none.
    pub fn colour_at(&self, i: usize) -> [f32; 3] {
        let list = &self.colour.colours;
        if list.is_empty() {
            [1.0, 1.0, 1.0]
        } else {
            list[i % list.len()]
        }
    }
}

/// A whole flare: its elements, drawn first to last.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Design {
    #[serde(default = "format")]
    pub lumit_flare: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub elements: Vec<Element>,
}

fn format() -> u32 {
    FORMAT
}

impl Design {
    /// Read a design from its JSON, or `None` when it isn't one.
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// The design an instance keeps, or `None` when it keeps none.
pub fn embedded(e: &EffectInstance) -> Option<Design> {
    serde_json::from_value(e.extra.get(EXTRA_KEY)?.clone()).ok()
}

/// The kept design's text exactly as stored, which is what the frame key and
/// the render read.
pub fn embedded_text(e: &EffectInstance) -> Option<String> {
    e.extra.get(EXTRA_KEY).map(|v| v.to_string())
}

/// Keep `design` in the instance, or forget it with `None`.
pub fn set_embedded(e: &mut EffectInstance, design: Option<&Design>) {
    match design.and_then(|d| serde_json::to_value(d).ok()) {
        Some(v) => {
            e.extra.insert(EXTRA_KEY.to_owned(), v);
        }
        None => {
            e.extra.remove(EXTRA_KEY);
        }
    }
}
