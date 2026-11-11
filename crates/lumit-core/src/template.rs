//! Graphics templates: a composition with **controls**, rows from inside it
//! that every Precomp layer or clip placing it can set for itself.
//!
//! The template is kept in the composition's `extra`
//! ([`Composition::template`]), so every road a composition travels (paste,
//! duplicate, another project, an older Lumit) carries it. A placed copy
//! keeps its own values as one effect instance
//! ([`crate::fx::effects::template`]), on [`crate::model::Layer::graph_inputs`]
//! or [`crate::sequence::Clip::inputs`].
//!
//! The render never hears the word. A copy plays the document
//! [`placed::document`] makes: the template's composition with the copy's
//! values written into the rows they name. So the picture, the frame key, the
//! decode plan, the mix and every expression inside read one ordinary
//! document, and two copies with two values are two pictures with two names.
//!
//! Runs wherever the document is read.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::fx::params::Unit;
use crate::model::{Composition, Document, TransformProp};
use crate::time::Rational;

/// Making and changing a template: controls, the ops that set them, looks.
pub mod author;
/// Reading a table of rows, and each cell as the control it feeds.
pub mod data;
/// Which rows of a table feed which controls, and one row as values.
pub mod feed;
/// The fonts a template's text names, for packing them with it.
pub mod fonts;
/// A copy's document: the template with the copy's values written in.
pub mod placed;
/// Reading and writing the row a control names.
pub mod rows;
/// A template's clock on a copy: the intro and the outro keep their speed.
pub mod time;
/// One row's version of a template: a copy fed from it, and its file's name.
pub mod version;

/// The most controls a template keeps.
pub const MAX_CONTROLS: usize = 256;

/// What makes a composition a graphics template: its controls, in the order
/// a copy shows them, and how it behaves when a copy is longer or shorter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Template {
    /// The library's name for this template, the same through every version.
    /// `None` until it is first saved to the library.
    ///
    /// Not called `id`: a composition copied into another project has every
    /// `id` it holds made new, and this one names something outside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<Uuid>,
    /// Which save of the library's template this composition holds, counting
    /// from 1. Zero is never saved.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub version: u32,
    /// The design as it was when last saved to or taken from the library, as
    /// [`design_stamp`] reads it. A composition whose stamp has moved has been
    /// changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved: Option<String>,
    /// The controls, in the order a copy shows them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<Control>,
    /// The frame the library shows for it. `None` is the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<Rational>,
    /// The moment the intro ends. `None` is no intro.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<Rational>,
    /// The moment the outro starts. `None` is no outro.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outro: Option<Rational>,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

fn raw() -> Unit {
    Unit::Raw
}

fn is_raw(unit: &Unit) -> bool {
    *unit == Unit::Raw
}

impl Template {
    /// The control called `id`.
    #[must_use]
    pub fn control(&self, id: &str) -> Option<&Control> {
        self.controls.iter().find(|c| c.id == id)
    }

    /// An id for a new control labelled `label`: the label in lower case with
    /// underscores, numbered when another control or one of the rows a copy
    /// derives from it already has it.
    #[must_use]
    pub fn free_id(&self, label: &str) -> String {
        let mut stem: String = label
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        while stem.contains("__") {
            stem = stem.replace("__", "_");
        }
        let stem = stem.trim_matches('_');
        let stem = if stem.is_empty() { "control" } else { stem };
        let taken = |id: &str| {
            RESERVED_IDS.contains(&id)
                || self.controls.iter().any(|c| {
                    c.id == id
                        || ROW_ENDINGS
                            .iter()
                            .any(|e| id == format!("{}{e}", c.id) || c.id == format!("{id}{e}"))
                })
        };
        if !taken(stem) {
            return stem.to_owned();
        }
        (2u32..)
            .map(|n| format!("{stem}_{n}"))
            .find(|id| !taken(id))
            .unwrap_or_else(|| stem.to_owned())
    }

    /// The groups the controls sit in, in the order they are first met.
    #[must_use]
    pub fn groups(&self) -> Vec<&str> {
        let mut groups: Vec<&str> = Vec::new();
        for group in self.controls.iter().filter_map(|c| c.group.as_deref()) {
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        groups
    }
}

/// The row ids a copy keeps that are not a control's: whether it plays the
/// template's intro and its outro.
pub const RESERVED_IDS: &[&str] = &[PLAY_INTRO, PLAY_OUTRO];
/// The row a copy keeps for playing the template's intro.
pub const PLAY_INTRO: &str = "play_intro";
/// The row a copy keeps for playing the template's outro.
pub const PLAY_OUTRO: &str = "play_outro";

/// What a control's further rows on a copy end in: a point's axes, and a text
/// control's font, size and colour, and how a footage control's picture is
/// fitted.
pub const ROW_ENDINGS: &[&str] = &["_x", "_y", "_z", "_font", "_size", "_colour", "_fit"];

/// One control: a row inside the template that a copy can set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Control {
    /// What a copy's value is kept under. Never changes once made, so a
    /// renamed control keeps every copy's value.
    pub id: String,
    /// What a copy shows it as, and what a data column is matched by.
    pub label: String,
    /// What it is drawn as on a copy.
    pub kind: ControlKind,
    /// The row it sets.
    pub target: Target,
    /// The folding heading it sits under on a copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// A number's range: the slider's travel on a copy, which typing stays
    /// inside. `None` leaves the row's own range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<(f64, f64)>,
    /// For a text control: whether a copy may also change the font.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub font: bool,
    /// For a text control: whether a copy may also change the size.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub size: bool,
    /// For a text control: whether a copy may also change the colour.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub colour: bool,
    /// For a menu: what each option sets besides the menu's own row, in the
    /// menu's order. An option with nothing recorded is empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub looks: Vec<Vec<Setting>>,
    /// A number's slider travel on a copy when it has no range of its own:
    /// the row's own, as it was when the control was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slider: Option<(f64, f64)>,
    /// What a number is counted in, written beside it on a copy.
    #[serde(default = "raw", skip_serializing_if = "is_raw")]
    pub unit: Unit,
    /// A menu's options, as the row lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// A point's axes: 2, or 3 for a row in 3D.
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub axes: u8,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Control {
    /// A control with nothing set but what every control has.
    #[must_use]
    pub fn new(id: String, label: String, kind: ControlKind, target: Target) -> Self {
        Self {
            id,
            label,
            kind,
            target,
            group: None,
            range: None,
            font: false,
            size: false,
            colour: false,
            looks: Vec::new(),
            slider: None,
            unit: Unit::Raw,
            options: Vec::new(),
            axes: 0,
            extra: serde_json::Map::new(),
        }
    }
}

/// One row a menu option sets, and what it sets it to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Setting {
    /// The row.
    pub target: Target,
    /// What the option sets it to.
    pub value: Value,
}

/// What a control is drawn as on a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    /// A number with a slider.
    Number,
    /// An angle with a dial.
    Angle,
    /// Two numbers, or three for a row in 3D, with a handle in the Viewer.
    Point,
    /// A colour.
    Colour,
    /// A tick.
    Checkbox,
    /// One of a list of options.
    Menu,
    /// Words.
    Text,
    /// A font: a family and a face.
    Font,
    /// A picture: a footage item or a composition.
    Footage,
}

/// A row of a text layer that is a number or a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextRow {
    /// The size of the letters.
    Size,
    /// The colour of the letters.
    Fill,
    /// The outline's colour.
    Stroke,
    /// The outline's width.
    StrokeWidth,
    /// The space between letters.
    Tracking,
    /// The colour of the box behind the words.
    Background,
}

/// A row of one shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeRow {
    /// The fill colour.
    Fill,
    /// The stroke colour.
    Stroke,
    /// The stroke's width.
    StrokeWidth,
    /// How opaque the shape is.
    Opacity,
    /// A rectangle's or an ellipse's width.
    Width,
    /// A rectangle's or an ellipse's height.
    Height,
    /// A rectangle's corners.
    Roundness,
    /// How far a parametric shape is turned.
    Rotation,
}

/// A row of one mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskRow {
    /// How opaque the mask is.
    Opacity,
    /// How soft its edge is.
    Feather,
    /// How far its edge is pushed out or pulled in.
    Expansion,
}

/// The row inside the template a control sets, by ids, so it holds through
/// any renaming.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "row", rename_all = "snake_case")]
pub enum Target {
    /// One transform row, or the rows of a point (position, scale, anchor
    /// point) or of a light's colour, in their own order.
    Transform {
        /// The layer.
        layer: Uuid,
        /// The rows: one for a number or an angle, two or three for a point,
        /// three for a colour.
        props: Vec<TransformProp>,
    },
    /// One row of an effect or a layer style.
    Param {
        /// The layer.
        layer: Uuid,
        /// The effect or the style.
        effect: Uuid,
        /// The row.
        param: String,
    },
    /// An effect's point: the two rows the panel shows as one.
    ParamPoint {
        /// The layer.
        layer: Uuid,
        /// The effect.
        effect: Uuid,
        /// The row across.
        x: String,
        /// The row down.
        y: String,
    },
    /// Whether an effect is on.
    EffectOn {
        /// The layer.
        layer: Uuid,
        /// The effect.
        effect: Uuid,
    },
    /// A Text layer's words.
    Text {
        /// The layer.
        layer: Uuid,
    },
    /// A Text layer's font.
    Font {
        /// The layer.
        layer: Uuid,
    },
    /// A number or a colour of a Text layer.
    TextRow {
        /// The layer.
        layer: Uuid,
        /// The row.
        value: TextRow,
    },
    /// A row of one shape on a Shape layer.
    Shape {
        /// The layer.
        layer: Uuid,
        /// The shape.
        item: Uuid,
        /// The row.
        value: ShapeRow,
    },
    /// A row of one mask.
    Mask {
        /// The layer.
        layer: Uuid,
        /// The mask.
        mask: Uuid,
        /// The row.
        value: MaskRow,
    },
    /// Whether a layer is shown.
    Visible {
        /// The layer.
        layer: Uuid,
    },
    /// A layer's Volume.
    Volume {
        /// The layer.
        layer: Uuid,
    },
    /// What a Footage layer or a Precomp layer shows.
    Source {
        /// The layer.
        layer: Uuid,
        /// The size of the picture the layer showed when the control was
        /// made, which a copy's own picture is fitted into. The document
        /// keeps no size for a footage item, so the control does.
        frame: (u32, u32),
    },
    /// A control of another template, placed inside this one.
    Copy {
        /// The Precomp layer placing it.
        layer: Uuid,
        /// The control, by its id in that template.
        control: String,
    },
}

impl Target {
    /// The layer the row is on.
    #[must_use]
    pub fn layer(&self) -> Uuid {
        match self {
            Target::Transform { layer, .. }
            | Target::Param { layer, .. }
            | Target::ParamPoint { layer, .. }
            | Target::EffectOn { layer, .. }
            | Target::Text { layer }
            | Target::Font { layer }
            | Target::TextRow { layer, .. }
            | Target::Shape { layer, .. }
            | Target::Mask { layer, .. }
            | Target::Visible { layer }
            | Target::Volume { layer }
            | Target::Source { layer, .. }
            | Target::Copy { layer, .. } => *layer,
        }
    }
}

/// How a copy's picture is fitted into the frame of the one it replaces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Covers the frame, cut where it runs over.
    #[default]
    Fill,
    /// All of it inside the frame.
    Fit,
    /// Pulled to the frame's own shape.
    Stretch,
}

impl Fit {
    /// The fit a menu's index says, the first being [`Fit::Fill`].
    #[must_use]
    pub fn from_choice(choice: u32) -> Fit {
        match choice {
            1 => Fit::Fit,
            2 => Fit::Stretch,
            _ => Fit::Fill,
        }
    }

    /// Where this fit sits in a menu.
    #[must_use]
    pub fn to_choice(self) -> u32 {
        match self {
            Fit::Fill => 0,
            Fit::Fit => 1,
            Fit::Stretch => 2,
        }
    }
}

/// A picture a copy puts in a footage control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Picture {
    /// The footage item or the composition.
    pub item: Uuid,
    /// Its size in pixels, as it was when it was put there. The document
    /// keeps no size for a footage item, so the copy does.
    pub size: (u32, u32),
    /// How it is fitted.
    #[serde(default)]
    pub fit: Fit,
}

/// One value of a control: what a row holds at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Value {
    /// A number or an angle.
    Number(f64),
    /// A point, as many numbers as the row has axes.
    Point(Vec<f64>),
    /// A colour as its row keeps it: scene-linear, with alpha.
    Colour([f64; 4]),
    /// A tick.
    Tick(bool),
    /// A menu's option, counting from 0.
    Choice(u32),
    /// Words.
    Text(String),
    /// A font.
    Font {
        /// The family.
        family: String,
        /// The face.
        face: String,
    },
    /// A picture.
    Picture(Picture),
}

/// How many kept templates the parse memo holds. A project shows a handful
/// at a time, and a miss costs one small parse.
const KEPT: usize = 64;

impl Composition {
    /// The key a graphics template is kept under in `extra`.
    pub const TEMPLATE_KEY: &'static str = "template";
    /// The key a template's data is kept under in `extra`: apart from the
    /// template, so reading the template every frame never reads the rows.
    pub const TEMPLATE_DATA_KEY: &'static str = "template_data";

    /// Whether this composition has controls, without reading them.
    #[must_use]
    pub fn is_template(&self) -> bool {
        self.extra.contains_key(Self::TEMPLATE_KEY)
    }

    /// This composition's template, or `None` for one with no controls.
    ///
    /// Asked on every frame for every copy, so a template already read is
    /// handed back: the same bytes are the same template.
    #[must_use]
    pub fn template(&self) -> Option<Arc<Template>> {
        type Kept = (Vec<u8>, Arc<Template>);
        static MEMO: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
        let value = self.extra.get(Self::TEMPLATE_KEY)?;
        let bytes = serde_json::to_vec(value).ok()?;
        if let Ok(memo) = MEMO.lock() {
            if let Some(kept) = memo.iter().find(|k| k.0 == bytes) {
                return Some(kept.1.clone());
            }
        }
        let mut template: Template = serde_json::from_value(value.clone()).ok()?;
        template.controls.truncate(MAX_CONTROLS);
        let template = Arc::new(template);
        if let Ok(mut memo) = MEMO.lock() {
            if memo.len() >= KEPT {
                memo.remove(0);
            }
            memo.push((bytes, template.clone()));
        }
        Some(template)
    }

    /// Set the template, or take it away with `None`.
    pub fn set_template(&mut self, template: Option<&Template>) {
        match template.and_then(|t| serde_json::to_value(t).ok()) {
            Some(v) => {
                self.extra.insert(Self::TEMPLATE_KEY.to_owned(), v);
            }
            None => {
                self.extra.remove(Self::TEMPLATE_KEY);
            }
        }
    }

    /// Where the template's intro ends and its outro starts, or `None` for a
    /// composition that has neither, which is every composition but a few.
    ///
    /// Read straight from `extra`, as the nested clock asks on every frame
    /// for every Precomp layer and clip.
    #[must_use]
    pub fn template_timing(&self) -> Option<time::Timing> {
        let template = self.extra.get(Self::TEMPLATE_KEY)?;
        let seconds = |key: &str| {
            let pair = template.get(key)?.as_array()?;
            let (num, den) = (pair.first()?.as_i64()?, pair.get(1)?.as_i64()?);
            Rational::new(num, den).ok().map(Rational::to_f64)
        };
        let (intro, outro) = (seconds("intro"), seconds("outro"));
        if intro.is_none() && outro.is_none() {
            return None;
        }
        let duration = self.duration.0.to_f64();
        Some(time::Timing {
            intro: intro.unwrap_or(0.0),
            outro: outro.unwrap_or(duration),
            duration,
        })
    }

    /// The data linked to this template, or `None` when none is.
    #[must_use]
    pub fn template_data(&self) -> Option<feed::Data> {
        self.extra
            .get(Self::TEMPLATE_DATA_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Link data to this template, or take it away with `None`.
    pub fn set_template_data(&mut self, data: Option<&feed::Data>) {
        match data.and_then(|d| serde_json::to_value(d).ok()) {
            Some(v) => {
                self.extra.insert(Self::TEMPLATE_DATA_KEY.to_owned(), v);
            }
            None => {
                self.extra.remove(Self::TEMPLATE_DATA_KEY);
            }
        }
    }
}

/// Apply [`crate::ops::Op::SetCompTemplate`], answering its inverse.
pub(crate) fn set_template(
    doc: &mut Document,
    comp: Uuid,
    template: &Option<Box<Template>>,
) -> Result<crate::ops::Op, crate::ops::OpError> {
    let c = doc.comp_mut(comp).ok_or(crate::ops::OpError::UnknownComp)?;
    let previous = c.template().map(|t| Box::new((*t).clone()));
    c.set_template(template.as_deref());
    Ok(crate::ops::Op::SetCompTemplate {
        comp,
        template: previous,
    })
}

/// Apply [`crate::ops::Op::SetCompTemplateData`], answering its inverse.
pub(crate) fn set_data(
    doc: &mut Document,
    comp: Uuid,
    data: &Option<Box<feed::Data>>,
) -> Result<crate::ops::Op, crate::ops::OpError> {
    let c = doc.comp_mut(comp).ok_or(crate::ops::OpError::UnknownComp)?;
    let previous = c.template_data().map(Box::new);
    c.set_template_data(data.as_deref());
    Ok(crate::ops::Op::SetCompTemplateData {
        comp,
        data: previous,
    })
}

/// A stamp of a template's design: its composition and every composition it
/// places, as they are saved, without the marks a save itself writes. Two
/// stamps that match are one design.
#[must_use]
pub fn design_stamp(doc: &Document, comp: Uuid) -> Option<String> {
    let mut seen: Vec<Uuid> = Vec::new();
    let mut hasher = blake3::Hasher::new();
    let mut queue = vec![comp];
    while let Some(id) = queue.pop() {
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let c = doc.comp(id)?;
        let mut bare = c.clone();
        if let Some(t) = bare.template() {
            let mut t = (*t).clone();
            t.library = None;
            t.version = 0;
            t.saved = None;
            bare.set_template(Some(&t));
        }
        bare.set_template_data(None);
        hasher.update(&serde_json::to_vec(&bare).ok()?);
        queue.extend(crate::edits::nested_comps(doc, c));
    }
    Some(hasher.finalize().to_hex().to_string())
}
