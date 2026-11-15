//! Graphics templates as the interface reads and edits them
//! (`lumit_core::template`): a composition's controls, the rows they name,
//! and what a menu's options set.
//!
//! A placed copy's own values are in [`copy`]. The library of saved templates
//! is in [`library`], and data and its rows in [`data`].
//!
//! Runs on the UI thread, like every other read and edit.

use flutter_rust_bridge::frb;
use lumit_core::model::{Composition, Document, LayerKind};
use lumit_core::ops::Op;
use lumit_core::template::{
    author, Control, ControlKind, Fit, MaskRow, Picture, Setting, ShapeRow, Target, Template,
    TextRow, Value,
};
use uuid::Uuid;

use crate::api::{
    composition::CompositionReference,
    effect::{bridge_unit, BridgeUnit},
    layer::BridgeTransformProp,
    BridgeError,
};

pub mod copy;
pub mod data;
pub mod library;
pub mod menu;
pub mod rows;

/// What a row is, for [`BridgeRowRef`].
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRowKind {
    /// Transform rows: one, or the rows of a point or a light's colour.
    Transform,
    /// One row of an effect or a layer style.
    Param,
    /// An effect's point, the two rows shown as one.
    ParamPoint,
    /// Whether an effect is on.
    EffectOn,
    /// A Text layer's words.
    Text,
    /// A Text layer's font.
    Font,
    /// A Text layer's size.
    TextSize,
    /// A Text layer's colour.
    TextFill,
    /// A Text layer's outline colour.
    TextStroke,
    /// A Text layer's outline width.
    TextStrokeWidth,
    /// A Text layer's tracking.
    TextTracking,
    /// The colour of the box behind a Text layer's words.
    TextBackground,
    /// A shape's fill colour.
    ShapeFill,
    /// A shape's stroke colour.
    ShapeStroke,
    /// A shape's stroke width.
    ShapeStrokeWidth,
    /// A shape's opacity.
    ShapeOpacity,
    /// A parametric shape's width.
    ShapeWidth,
    /// A parametric shape's height.
    ShapeHeight,
    /// A rectangle's corners.
    ShapeRoundness,
    /// A parametric shape's rotation.
    ShapeRotation,
    /// A mask's opacity.
    MaskOpacity,
    /// A mask's feather.
    MaskFeather,
    /// A mask's expansion.
    MaskExpansion,
    /// Whether the layer is shown.
    Visible,
    /// The layer's Volume.
    Volume,
    /// What a Footage or Precomp layer shows.
    Source,
    /// A control of a template placed inside this one.
    Copy,
}

/// One row of one layer, by ids: what **Add to controls** is handed, and what
/// a control says it sets. Two refs name the same row when everything but
/// `label` matches.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeRowRef {
    pub kind: BridgeRowKind,
    pub layer: Uuid,
    /// Transform rows, in their own order.
    pub props: Vec<BridgeTransformProp>,
    /// The effect or style, the shape, or the mask.
    pub owner: Option<Uuid>,
    /// An effect row's id, a point's row across, or a nested copy's control.
    pub param: Option<String>,
    /// A point's row down.
    pub param_y: Option<String>,
    /// What the row is called where it was picked, for a new control's label.
    /// Not part of which row it is.
    pub label: String,
}

/// What a control is drawn as on a copy.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeControlKind {
    Number,
    Angle,
    Point,
    Colour,
    Checkbox,
    Menu,
    Text,
    Font,
    Footage,
}

/// One value of a control.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeControlValue {
    /// A number or an angle.
    Number(f64),
    /// A point: two numbers, or three.
    Point(Vec<f64>),
    /// A colour as its row keeps it: scene-linear red, green, blue and alpha.
    Colour(Vec<f64>),
    Tick(bool),
    /// A menu's option, counting from 0.
    Choice(u32),
    Text(String),
    Font {
        family: String,
        face: String,
    },
    /// A footage item or a composition, and how it is fitted: 0 fill, 1 fit,
    /// 2 stretch.
    Picture {
        item: Uuid,
        fit: u32,
    },
}

/// One control of a template, as the Graphics template panel lists it.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeControl {
    pub id: String,
    pub label: String,
    pub kind: BridgeControlKind,
    pub group: Option<String>,
    /// The row it sets.
    pub row: BridgeRowRef,
    /// The layer the row is on, by name. Empty when the layer has gone.
    pub layer_name: String,
    /// The row has gone: its layer, effect, shape or mask was deleted.
    pub gone: bool,
    /// The row is keyed or has an expression in the template, so the panel
    /// shows it and does not edit it.
    pub animated: bool,
    /// What the row holds in the template.
    pub value: Option<BridgeControlValue>,
    /// Whether a number has a range a copy stays inside.
    pub ranged: bool,
    pub range_min: f64,
    pub range_max: f64,
    /// The slider's travel on a copy.
    pub slider_min: f64,
    pub slider_max: f64,
    pub unit: BridgeUnit,
    /// A menu's options.
    pub options: Vec<String>,
    /// How many rows each of a menu's options sets, in the menu's order.
    pub looks: Vec<u32>,
    /// A point's axes.
    pub axes: u32,
    /// For text: whether a copy may change the font, the size, the colour.
    pub font: bool,
    pub size: bool,
    pub colour: bool,
}

/// A menu option being recorded.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRecording {
    /// The menu, by its control id.
    pub menu: String,
    pub option: u32,
}

/// A composition's graphics template.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTemplate {
    /// The library's id for it, once it has been saved there.
    pub id: Option<Uuid>,
    /// Which save of the library's template this is. Zero is never saved.
    pub version: u32,
    /// The design has changed since it was saved to or taken from the library.
    pub changed: bool,
    pub controls: Vec<BridgeControl>,
    /// The frame the library shows, the frame the intro ends on and the frame
    /// the outro starts on.
    pub poster: Option<i64>,
    pub intro: Option<i64>,
    pub outro: Option<i64>,
    /// The menu option being recorded in this composition, when one is.
    pub recording: Option<BridgeRecording>,
}

/// Every transform row the bridge names, for reading one back.
const TRANSFORM_PROPS: &[BridgeTransformProp] = {
    use BridgeTransformProp as P;
    &[
        P::AnchorX,
        P::AnchorY,
        P::PositionX,
        P::PositionY,
        P::PositionZ,
        P::ScaleX,
        P::ScaleY,
        P::Rotation,
        P::RotationX,
        P::RotationY,
        P::Opacity,
        P::OrientationX,
        P::OrientationY,
        P::OrientationZ,
        P::AnchorZ,
        P::ScaleZ,
        P::PoiX,
        P::PoiY,
        P::PoiZ,
        P::Zoom,
        P::FocusDistance,
        P::Aperture,
        P::BlurLevel,
        P::LightIntensity,
        P::LightColourR,
        P::LightColourG,
        P::LightColourB,
        P::ConeAngle,
        P::ConeFeather,
        P::LightRadius,
        P::FalloffDistance,
        P::LightWidth,
        P::LightHeight,
        P::ShadowDarkness,
        P::ShadowDiffusion,
        P::HazeAmount,
        P::HazeDensity,
        P::HazeColourR,
        P::HazeColourG,
        P::HazeColourB,
        P::HazeFalloff,
        P::HazeScatter,
        P::HazeNoise,
        P::HazeNoiseSize,
        P::HazeNoiseSpeed,
        P::BackgroundBlur,
        P::Ambient,
        P::Diffuse,
        P::Specular,
        P::Shininess,
        P::Metal,
        P::LightTransmission,
        P::Reflection,
    ]
};

impl BridgeControlKind {
    #[frb(ignore)]
    pub(crate) fn of(kind: ControlKind) -> Self {
        match kind {
            ControlKind::Number => Self::Number,
            ControlKind::Angle => Self::Angle,
            ControlKind::Point => Self::Point,
            ControlKind::Colour => Self::Colour,
            ControlKind::Checkbox => Self::Checkbox,
            ControlKind::Menu => Self::Menu,
            ControlKind::Text => Self::Text,
            ControlKind::Font => Self::Font,
            ControlKind::Footage => Self::Footage,
        }
    }
}

impl BridgeControlValue {
    #[frb(ignore)]
    pub(crate) fn of(value: &Value) -> Self {
        match value {
            Value::Number(v) => Self::Number(*v),
            Value::Point(p) => Self::Point(p.clone()),
            Value::Colour(c) => Self::Colour(c.to_vec()),
            Value::Tick(b) => Self::Tick(*b),
            Value::Choice(c) => Self::Choice(*c),
            Value::Text(t) => Self::Text(t.clone()),
            Value::Font { family, face } => Self::Font {
                family: family.clone(),
                face: face.clone(),
            },
            Value::Picture(p) => Self::Picture {
                item: p.item,
                fit: p.fit.to_choice(),
            },
        }
    }

    /// The value in the engine's own shape. A picture takes its `size` from
    /// the caller, which is the one who can measure it.
    #[frb(ignore)]
    pub(crate) fn core(&self, size: (u32, u32)) -> Result<Value, BridgeError> {
        Ok(match self {
            Self::Number(v) => Value::Number(*v),
            Self::Point(p) => Value::Point(p.clone()),
            Self::Colour(c) => Value::Colour([
                *c.first().ok_or(BridgeError::InvalidParam)?,
                *c.get(1).ok_or(BridgeError::InvalidParam)?,
                *c.get(2).ok_or(BridgeError::InvalidParam)?,
                c.get(3).copied().unwrap_or(1.0),
            ]),
            Self::Tick(b) => Value::Tick(*b),
            Self::Choice(c) => Value::Choice(*c),
            Self::Text(t) => Value::Text(t.clone()),
            Self::Font { family, face } => Value::Font {
                family: family.clone(),
                face: face.clone(),
            },
            Self::Picture { item, fit } => Value::Picture(Picture {
                item: *item,
                size,
                fit: Fit::from_choice(*fit),
            }),
        })
    }
}

impl BridgeRowRef {
    /// The row the engine names, or `None` for a ref that names none. A
    /// source's `frame` is the size of the picture the layer shows now.
    #[frb(ignore)]
    pub(crate) fn target(&self, frame: (u32, u32)) -> Option<Target> {
        use BridgeRowKind as K;
        let layer = self.layer;
        let text = |value| Target::TextRow { layer, value };
        let shape = |value| {
            Some(Target::Shape {
                layer,
                item: self.owner?,
                value,
            })
        };
        let mask = |value| {
            Some(Target::Mask {
                layer,
                mask: self.owner?,
                value,
            })
        };
        Some(match self.kind {
            K::Transform => Target::Transform {
                layer,
                props: self.props.iter().map(|p| p.core()).collect(),
            },
            K::Param => Target::Param {
                layer,
                effect: self.owner?,
                param: self.param.clone()?,
            },
            K::ParamPoint => Target::ParamPoint {
                layer,
                effect: self.owner?,
                x: self.param.clone()?,
                y: self.param_y.clone()?,
            },
            K::EffectOn => Target::EffectOn {
                layer,
                effect: self.owner?,
            },
            K::Text => Target::Text { layer },
            K::Font => Target::Font { layer },
            K::TextSize => text(TextRow::Size),
            K::TextFill => text(TextRow::Fill),
            K::TextStroke => text(TextRow::Stroke),
            K::TextStrokeWidth => text(TextRow::StrokeWidth),
            K::TextTracking => text(TextRow::Tracking),
            K::TextBackground => text(TextRow::Background),
            K::ShapeFill => shape(ShapeRow::Fill)?,
            K::ShapeStroke => shape(ShapeRow::Stroke)?,
            K::ShapeStrokeWidth => shape(ShapeRow::StrokeWidth)?,
            K::ShapeOpacity => shape(ShapeRow::Opacity)?,
            K::ShapeWidth => shape(ShapeRow::Width)?,
            K::ShapeHeight => shape(ShapeRow::Height)?,
            K::ShapeRoundness => shape(ShapeRow::Roundness)?,
            K::ShapeRotation => shape(ShapeRow::Rotation)?,
            K::MaskOpacity => mask(MaskRow::Opacity)?,
            K::MaskFeather => mask(MaskRow::Feather)?,
            K::MaskExpansion => mask(MaskRow::Expansion)?,
            K::Visible => Target::Visible { layer },
            K::Volume => Target::Volume { layer },
            K::Source => Target::Source { layer, frame },
            K::Copy => Target::Copy {
                layer,
                control: self.param.clone()?,
            },
        })
    }

    /// The row a control names, as the interface names rows.
    #[frb(ignore)]
    pub(crate) fn of(target: &Target, label: &str) -> Self {
        use BridgeRowKind as K;
        let mut row = BridgeRowRef {
            kind: K::Visible,
            layer: target.layer(),
            props: Vec::new(),
            owner: None,
            param: None,
            param_y: None,
            label: label.to_owned(),
        };
        match target {
            Target::Transform { props, .. } => {
                row.kind = K::Transform;
                row.props = props
                    .iter()
                    .filter_map(|p| TRANSFORM_PROPS.iter().copied().find(|b| b.core() == *p))
                    .collect();
            }
            Target::Param { effect, param, .. } => {
                row.kind = K::Param;
                row.owner = Some(*effect);
                row.param = Some(param.clone());
            }
            Target::ParamPoint { effect, x, y, .. } => {
                row.kind = K::ParamPoint;
                row.owner = Some(*effect);
                row.param = Some(x.clone());
                row.param_y = Some(y.clone());
            }
            Target::EffectOn { effect, .. } => {
                row.kind = K::EffectOn;
                row.owner = Some(*effect);
            }
            Target::Text { .. } => row.kind = K::Text,
            Target::Font { .. } => row.kind = K::Font,
            Target::TextRow { value, .. } => {
                row.kind = match value {
                    TextRow::Size => K::TextSize,
                    TextRow::Fill => K::TextFill,
                    TextRow::Stroke => K::TextStroke,
                    TextRow::StrokeWidth => K::TextStrokeWidth,
                    TextRow::Tracking => K::TextTracking,
                    TextRow::Background => K::TextBackground,
                };
            }
            Target::Shape { item, value, .. } => {
                row.owner = Some(*item);
                row.kind = match value {
                    ShapeRow::Fill => K::ShapeFill,
                    ShapeRow::Stroke => K::ShapeStroke,
                    ShapeRow::StrokeWidth => K::ShapeStrokeWidth,
                    ShapeRow::Opacity => K::ShapeOpacity,
                    ShapeRow::Width => K::ShapeWidth,
                    ShapeRow::Height => K::ShapeHeight,
                    ShapeRow::Roundness => K::ShapeRoundness,
                    ShapeRow::Rotation => K::ShapeRotation,
                };
            }
            Target::Mask { mask, value, .. } => {
                row.owner = Some(*mask);
                row.kind = match value {
                    MaskRow::Opacity => K::MaskOpacity,
                    MaskRow::Feather => K::MaskFeather,
                    MaskRow::Expansion => K::MaskExpansion,
                };
            }
            Target::Visible { .. } => row.kind = K::Visible,
            Target::Volume { .. } => row.kind = K::Volume,
            Target::Source { .. } => row.kind = K::Source,
            Target::Copy { control, .. } => {
                row.kind = K::Copy;
                row.param = Some(control.clone());
            }
        }
        row
    }
}

/// Whether the row `target` names is keyed or runs an expression in `comp`,
/// which the panel shows and leaves alone.
fn animated(comp: &Composition, target: &Target) -> bool {
    let Some(layer) = comp.layers.iter().find(|l| l.id == target.layer()) else {
        return false;
    };
    match target {
        Target::Transform { props, .. } => props
            .iter()
            .any(|p| layer.prop(*p).is_some_and(|row| row.is_animated())),
        Target::Param { effect, param, .. } => layer
            .effects
            .iter()
            .chain(&layer.styles)
            .find(|e| e.id == *effect)
            .and_then(|e| e.param(param))
            .is_some_and(|value| match value {
                lumit_core::model::EffectValue::Float(p) => p.is_animated(),
                lumit_core::model::EffectValue::Point(x, y) => x.is_animated() || y.is_animated(),
                lumit_core::model::EffectValue::Colour(c) => c.iter().any(|p| p.is_animated()),
                _ => false,
            }),
        Target::Text { .. } => match &layer.kind {
            LayerKind::Text { document } => document.expression.is_some(),
            _ => false,
        },
        Target::Volume { .. } => layer.volume_db.is_animated(),
        _ => false,
    }
}

/// One control as the panel lists it.
#[frb(ignore)]
pub(crate) fn bridge_control(
    doc: &Document,
    comp: &Composition,
    control: &Control,
) -> BridgeControl {
    let value = lumit_core::template::rows::read(doc, comp, &control.target);
    let layer = comp.layers.iter().find(|l| l.id == control.target.layer());
    let (slider_min, slider_max) = control.range.or(control.slider).unwrap_or((0.0, 100.0));
    let (range_min, range_max) = control.range.unwrap_or((0.0, 0.0));
    BridgeControl {
        id: control.id.clone(),
        label: control.label.clone(),
        kind: BridgeControlKind::of(control.kind),
        group: control.group.clone(),
        row: BridgeRowRef::of(&control.target, &control.label),
        layer_name: layer.map(|l| l.name.clone()).unwrap_or_default(),
        gone: value.is_none(),
        animated: animated(comp, &control.target),
        value: value.as_ref().map(BridgeControlValue::of),
        ranged: control.range.is_some(),
        range_min,
        range_max,
        slider_min,
        slider_max,
        unit: bridge_unit(control.unit),
        options: control.options.clone(),
        looks: control.looks.iter().map(|l| l.len() as u32).collect(),
        axes: u32::from(control.axes),
        font: control.font,
        size: control.size,
        colour: control.colour,
    }
}

impl CompositionReference {
    /// The template this composition holds, or an empty one to build on.
    #[frb(ignore)]
    pub(crate) fn template_or_new(&self) -> Result<Template, BridgeError> {
        Ok(self
            .composition()?
            .template()
            .map(|t| (*t).clone())
            .unwrap_or_default())
    }

    /// The op that sets this composition's template. One with no controls and
    /// nothing else set is no template at all.
    #[frb(ignore)]
    pub(crate) fn template_op(&self, template: Template) -> Op {
        let bare = template.controls.is_empty()
            && template.library.is_none()
            && template.poster.is_none()
            && template.intro.is_none()
            && template.outro.is_none();
        Op::SetCompTemplate {
            comp: self.id,
            template: (!bare).then(|| Box::new(template)),
        }
    }

    #[frb(ignore)]
    pub(crate) fn edit_template(
        &self,
        edit: impl FnOnce(&mut Template) -> Result<(), BridgeError>,
    ) -> Result<(), BridgeError> {
        let mut template = self.template_or_new()?;
        edit(&mut template)?;
        self.commit(self.template_op(template))
    }

    /// The size of the picture `layer` shows now: a footage item's own, or a
    /// composition's. Nought by nought when it cannot be told.
    #[frb(ignore)]
    fn shown_size(&self, layer: Uuid) -> Result<(u32, u32), BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer) else {
            return Ok((0, 0));
        };
        match &layer.kind {
            LayerKind::Footage { item } | LayerKind::Precomp { comp: item } => {
                copy::picture_size(self.project, &doc, comp, *item)
            }
            _ => Ok((0, 0)),
        }
    }

    /// This composition's graphics template: its controls with what each row
    /// holds, or `None` for a composition with no controls.
    #[frb(sync)]
    pub fn get_template(&self) -> Result<Option<BridgeTemplate>, BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let Some(template) = comp.template() else {
            return Ok(None);
        };
        let frame = |t: Option<lumit_core::time::Rational>| {
            t.map(|t| {
                comp.frame_rate
                    .nearest_frame_at(lumit_core::time::CompTime(t))
            })
        };
        let recording = menu::recording_of(self.project, self.id);
        Ok(Some(BridgeTemplate {
            id: template.library,
            version: template.version,
            changed: template.saved.is_some()
                && lumit_core::template::design_stamp(&doc, self.id) != template.saved,
            controls: template
                .controls
                .iter()
                .map(|c| bridge_control(&doc, comp, c))
                .collect(),
            poster: frame(template.poster),
            intro: frame(template.intro),
            outro: frame(template.outro),
            recording,
        }))
    }

    /// **Add to controls**: make a control for each of `rows` that is not one
    /// already, after the controls there are. Answers the new controls' ids.
    /// A row no control can set is passed over.
    #[frb(sync)]
    pub fn add_controls(&self, rows: Vec<BridgeRowRef>) -> Result<Vec<String>, BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let mut template = self.template_or_new()?;
        let mut made = Vec::new();
        for row in rows {
            let frame = if row.kind == BridgeRowKind::Source {
                self.shown_size(row.layer)?
            } else {
                (0, 0)
            };
            let Some(target) = row.target(frame) else {
                continue;
            };
            if template.controls.iter().any(|c| c.target == target)
                || template.controls.len() >= lumit_core::template::MAX_CONTROLS
            {
                continue;
            }
            // A layer's words and its picture are best known by the layer.
            let label = match target {
                Target::Text { layer } | Target::Source { layer, .. } => comp
                    .layers
                    .iter()
                    .find(|l| l.id == layer)
                    .map_or(row.label.clone(), |l| l.name.clone()),
                _ => row.label.clone(),
            };
            let Some(mut control) = author::control_for(&doc, comp, target, &label) else {
                continue;
            };
            control.id = template.free_id(&label);
            made.push(control.id.clone());
            template.controls.push(control);
        }
        if !made.is_empty() {
            self.commit(self.template_op(template))?;
        }
        Ok(made)
    }

    /// Take controls away. Copies keep what they had set for them, unread, so
    /// undoing this brings every value back.
    #[frb(sync)]
    pub fn remove_controls(&self, ids: Vec<String>) -> Result<(), BridgeError> {
        self.edit_template(|t| {
            t.controls.retain(|c| !ids.contains(&c.id));
            Ok(())
        })
    }

    /// Rename a control. Its id, and so every copy's value, stays.
    #[frb(sync)]
    pub fn rename_control(&self, id: String, label: String) -> Result<(), BridgeError> {
        let label = label.trim().to_owned();
        if label.is_empty() {
            return Err(BridgeError::EmptyName);
        }
        self.edit_template(|t| {
            let control = t
                .controls
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or(BridgeError::InvalidParam)?;
            control.label = label;
            Ok(())
        })
    }

    /// Move controls to sit before the control `before`, or last with `None`,
    /// keeping their own order, and into `group` (`None` is no group).
    #[frb(sync)]
    pub fn move_controls(
        &self,
        ids: Vec<String>,
        before: Option<String>,
        group: Option<String>,
    ) -> Result<(), BridgeError> {
        self.edit_template(|t| {
            let (mut moved, mut rest): (Vec<Control>, Vec<Control>) =
                t.controls.drain(..).partition(|c| ids.contains(&c.id));
            for control in &mut moved {
                control.group = group.clone().filter(|g| !g.trim().is_empty());
            }
            let at = before
                .as_ref()
                .and_then(|b| rest.iter().position(|c| &c.id == b))
                .unwrap_or(rest.len());
            rest.splice(at..at, moved);
            t.controls = rest;
            Ok(())
        })
    }

    /// Rename a group of controls.
    #[frb(sync)]
    pub fn rename_control_group(&self, group: String, name: String) -> Result<(), BridgeError> {
        let name = name.trim().to_owned();
        self.edit_template(|t| {
            for control in t
                .controls
                .iter_mut()
                .filter(|c| c.group.as_ref() == Some(&group))
            {
                control.group = (!name.is_empty()).then(|| name.clone());
            }
            Ok(())
        })
    }

    /// Set a number control's range, which a copy stays inside, or take it
    /// away with `ranged` off.
    #[frb(sync)]
    pub fn set_control_range(
        &self,
        id: String,
        ranged: bool,
        min: f64,
        max: f64,
    ) -> Result<(), BridgeError> {
        if ranged && !(min.is_finite() && max.is_finite() && min < max) {
            return Err(BridgeError::InvalidParam);
        }
        self.edit_template(|t| {
            let control = t
                .controls
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or(BridgeError::InvalidParam)?;
            control.range = ranged.then_some((min, max));
            Ok(())
        })
    }

    /// Say whether a copy may also change a text control's font, size and
    /// colour.
    #[frb(sync)]
    pub fn set_control_text_rows(
        &self,
        id: String,
        font: bool,
        size: bool,
        colour: bool,
    ) -> Result<(), BridgeError> {
        self.edit_template(|t| {
            let control = t
                .controls
                .iter_mut()
                .find(|c| c.id == id && c.kind == ControlKind::Text)
                .ok_or(BridgeError::InvalidParam)?;
            (control.font, control.size, control.colour) = (font, size, colour);
            Ok(())
        })
    }

    /// Set what a control's row holds in the template itself, fixed. Picking
    /// a menu's option also sets every row that option's look names. One undo
    /// step.
    #[frb(sync)]
    pub fn set_control_value(
        &self,
        id: String,
        value: BridgeControlValue,
    ) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let template = comp.template().ok_or(BridgeError::InvalidParam)?;
        let control = template.control(&id).ok_or(BridgeError::InvalidParam)?;
        let size = match &value {
            BridgeControlValue::Picture { item, .. } => {
                copy::picture_size(self.project, &doc, comp, *item)?
            }
            _ => (0, 0),
        };
        let value = value.core(size)?;
        let mut settings: Vec<Setting> = match &value {
            Value::Choice(option) => control
                .looks
                .get(*option as usize)
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        settings.push(Setting {
            target: control.target.clone(),
            value,
        });
        let ops = author::ops(&doc, self.id, &settings);
        if ops.is_empty() {
            return Ok(());
        }
        self.commit(Op::Batch { ops })
    }

    /// Set the frame the library shows for the template, where its intro ends
    /// and where its outro starts. `None` clears one.
    #[frb(sync)]
    pub fn set_template_marks(
        &self,
        poster: Option<i64>,
        intro: Option<i64>,
        outro: Option<i64>,
    ) -> Result<(), BridgeError> {
        let comp = self.composition()?;
        let time = |frame: Option<i64>| -> Result<Option<lumit_core::time::Rational>, BridgeError> {
            frame
                .map(|f| {
                    comp.frame_rate
                        .time_of_frame(f.max(0))
                        .map(|t| t.0)
                        .map_err(|_| BridgeError::InvalidTime)
                })
                .transpose()
        };
        let (poster, intro, outro) = (time(poster)?, time(intro)?, time(outro)?);
        // An outro that starts before the intro ends leaves nothing between
        // them to stretch, and is refused.
        if let (Some(i), Some(o)) = (intro, outro) {
            if o < i {
                return Err(BridgeError::InvalidTime);
            }
        }
        self.edit_template(|t| {
            (t.poster, t.intro, t.outro) = (poster, intro, outro);
            Ok(())
        })
    }
}
