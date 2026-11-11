//! Graphics template: the values a placed copy of a template keeps.
//!
//! **In plain terms.** A graphics template is a composition with controls. A
//! Precomp layer or a clip that places it is a copy, and what the copy has
//! set for itself is kept in one of these: an effect instance that draws
//! nothing and holds a row for each control. So a copy's values keyframe,
//! take expressions and save as any effect's rows do, through the code that
//! already does that.
//!
//! **The rows are derived, never adopted**, as the Node graph effect's are.
//! They come from a copy of the template's control list the instance carries
//! in `extra.template`, read through a cache keyed by that list's hash,
//! because [`EffectDef::derived`] has no document in its hand. The copy is
//! refreshed on the clones the bridge hands out ([`refresh`]).
//!
//! **A copy stores only what it has set.** `own` lists the rows the copy set
//! itself and `fed` the rows a data row feeds. Every other row follows the
//! template: it is offered at the template's own value when the rows are
//! shown ([`offer`]) and dropped again when the instance is stored
//! ([`settle`]), so a change to the template reaches every copy that has not
//! set that row.
//!
//! Not an image operation: the render reads these values to make the copy's
//! document ([`crate::template::placed`]) and never runs this as an effect.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::anim::Property;
use crate::expression::ExpressionContext;
use crate::fx::registry::{EffectDef, EffectMetadata, RowState};
use crate::fx::schema::{EffectSchema, ParamSchema};
use crate::model::{Document, EffectInstance, EffectParam, EffectValue};
use crate::template::time::Ends;
use crate::template::{
    Control, ControlKind, Fit, Picture, Setting, Target, Template, TextRow, Value, PLAY_INTRO,
    PLAY_OUTRO,
};
use lumit_fx_macros::Effect;

/// The rows a copy's control list derives.
mod rows;

/// The effect's match name.
pub const MATCH_NAME: &str = "template";

/// The `extra` key a copy's [`Held`] lives under.
pub const EXTRA_KEY: &str = "template";

/// The effect a copy's values are kept as. It declares nothing: every row it
/// shows is derived from the template it names.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "template",
    label = "Graphics template",
    version = 1,
    category = Controls,
    // It never renders.
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct TemplateValues {}

/// What a copy's instance keeps beside its rows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Held {
    /// The template's composition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comp: Option<Uuid>,
    /// The template's controls as they were last read, without what a menu's
    /// options set. What the rows are derived from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<Control>,
    /// Whether the template has an intro, and so the copy a row for playing it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub intro: bool,
    /// Whether the template has an outro.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outro: bool,
    /// The rows the copy has set itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub own: Vec<String>,
    /// The rows the copy's data row feeds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fed: Vec<String>,
    /// The data row the copy follows, counting from 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<u32>,
    /// The template's own value for each row, while the instance is handed
    /// out to be shown ([`offer`]). Never stored: [`settle`] takes it away.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live: Vec<(String, EffectValue)>,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Held {
    /// Whether the copy has a value of its own for `row`, set or fed.
    #[must_use]
    pub fn has(&self, row: &str) -> bool {
        self.own.iter().chain(&self.fed).any(|r| r == row)
    }
}

/// What `inst` keeps beside its rows. Empty for an instance that is not a
/// copy's, and for one written in a shape this build cannot read.
#[must_use]
pub fn held(inst: &EffectInstance) -> Held {
    inst.extra
        .get(EXTRA_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// Write what `inst` keeps beside its rows.
pub fn set_held(inst: &mut EffectInstance, held: &Held) {
    if let Ok(v) = serde_json::to_value(held) {
        inst.extra.insert(EXTRA_KEY.to_owned(), v);
    }
}

/// The template composition `inst` holds values for, or `None` for an
/// instance that is not a copy's.
#[must_use]
pub fn comp_of(inst: &EffectInstance) -> Option<Uuid> {
    if inst.effect.match_name != MATCH_NAME {
        return None;
    }
    Uuid::parse_str(inst.extra.get(EXTRA_KEY)?.get("comp")?.as_str()?).ok()
}

/// Point a copy's values at composition `comp`, when they are a copy's. What
/// follows a template that is given a new id, as a duplicate's is.
pub fn rebind(inst: &mut EffectInstance, comp: Uuid) {
    if comp_of(inst).is_some_and(|was| was != comp) {
        let mut kept = held(inst);
        kept.comp = Some(comp);
        set_held(inst, &kept);
    }
}

/// The controls as a copy carries them: without what a menu's options set,
/// which only the template needs.
fn carried(template: &Template) -> Vec<Control> {
    template
        .controls
        .iter()
        .map(|c| Control {
            looks: Vec::new(),
            ..c.clone()
        })
        .collect()
}

/// A fresh instance for a copy of the template in composition `comp`, with
/// nothing set.
#[must_use]
pub fn instance_for(comp: Uuid, template: &Template) -> EffectInstance {
    let mut inst = EffectInstance {
        id: Uuid::now_v7(),
        effect: crate::model::EffectKey {
            namespace: crate::model::EffectNamespace::Builtin,
            match_name: MATCH_NAME.to_owned(),
            version: 1,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: Vec::new(),
        sample_temporally: true,
        custom_name: None,
        linked_pairs: Vec::new(),
        plugin_state: None,
        roto: None,
        extra: serde_json::Map::new(),
    };
    set_held(
        &mut inst,
        &Held {
            comp: Some(comp),
            controls: carried(template),
            intro: template.intro.is_some(),
            outro: template.outro.is_some(),
            ..Held::default()
        },
    );
    inst
}

/// Rewrite the instance's copy of the controls from the live template, and
/// say whether that changed anything. A template that has gone leaves the
/// copy as it was.
pub fn refresh(inst: &mut EffectInstance, doc: &Document) -> bool {
    let Some(template) = comp_of(inst).and_then(|id| doc.comp(id)?.template()) else {
        return false;
    };
    let mut kept = held(inst);
    let live = carried(&template);
    let (intro, outro) = (template.intro.is_some(), template.outro.is_some());
    if kept.controls == live && kept.intro == intro && kept.outro == outro {
        return false;
    }
    kept.controls = live;
    kept.intro = intro;
    kept.outro = outro;
    set_held(inst, &kept);
    true
}

/// The rows a control keeps on a copy, each with the kind of value it holds.
pub(crate) fn rows_of_control(control: &Control) -> Vec<(String, RowKind)> {
    let id = &control.id;
    match control.kind {
        ControlKind::Number => vec![(id.clone(), RowKind::Number)],
        ControlKind::Angle => vec![(id.clone(), RowKind::Angle)],
        ControlKind::Point => {
            let mut rows = vec![
                (format!("{id}_x"), RowKind::Axis),
                (format!("{id}_y"), RowKind::Axis),
            ];
            if control.axes >= 3 {
                rows.push((format!("{id}_z"), RowKind::Axis));
            }
            rows
        }
        ControlKind::Colour => vec![(id.clone(), RowKind::Colour)],
        ControlKind::Checkbox => vec![(id.clone(), RowKind::Tick)],
        ControlKind::Menu => vec![(id.clone(), RowKind::Menu)],
        ControlKind::Text => {
            let mut rows = vec![(id.clone(), RowKind::Words)];
            if control.font {
                rows.push((format!("{id}_font"), RowKind::Font));
            }
            if control.size {
                rows.push((format!("{id}_size"), RowKind::Size));
            }
            if control.colour {
                rows.push((format!("{id}_colour"), RowKind::TextColour));
            }
            rows
        }
        ControlKind::Font => vec![(id.clone(), RowKind::Font)],
        ControlKind::Footage => vec![
            (id.clone(), RowKind::Picture),
            (format!("{id}_fit"), RowKind::Fit),
        ],
    }
}

/// What one row of a copy holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowKind {
    Number,
    Angle,
    Axis,
    Colour,
    Tick,
    Menu,
    Words,
    Font,
    Size,
    TextColour,
    Picture,
    Fit,
}

/// The ids of every row `control` keeps on a copy.
#[must_use]
pub fn row_ids(control: &Control) -> Vec<String> {
    rows_of_control(control)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// A font as one row keeps it: the family, a line break, the face.
fn font_text(family: &str, face: &str) -> String {
    format!("{family}\n{face}")
}

fn font_of(text: &str) -> (String, String) {
    let (family, face) = text.split_once('\n').unwrap_or((text, ""));
    (family.to_owned(), face.to_owned())
}

fn param<'a>(inst: &'a EffectInstance, id: &str) -> Option<&'a EffectParam> {
    inst.params.iter().find(|p| p.id == id)
}

fn put(inst: &mut EffectInstance, id: String, value: EffectValue) -> &mut EffectParam {
    let at = match inst.params.iter().position(|p| p.id == id) {
        Some(at) => {
            inst.params[at].value = value;
            at
        }
        None => {
            inst.params.push(EffectParam {
                id,
                value,
                extra: serde_json::Map::new(),
            });
            inst.params.len() - 1
        }
    };
    &mut inst.params[at]
}

/// A number row at layer time `lt`, with a driver wire's number in front of
/// the stored one.
fn number(
    inst: &EffectInstance,
    id: &str,
    lt: f64,
    context: Option<&Arc<ExpressionContext>>,
    drivers: Option<&crate::fx::drivers::ResolvedDrivers>,
) -> Option<f64> {
    if let Some(driven) = drivers.filter(|d| !d.is_empty()).and_then(|d| {
        d.param(
            crate::graph::NodeRef::Effect(inst.id),
            crate::fx::params::ParamId::new(id),
        )
    }) {
        return Some(f64::from(driven.as_f32()));
    }
    match &param(inst, id)?.value {
        EffectValue::Float(p) => Some(match context {
            Some(c) => p.value_at_with_context(lt, c.clone()),
            None => p.value_at(lt),
        }),
        _ => None,
    }
}

fn colour(
    inst: &EffectInstance,
    id: &str,
    lt: f64,
    context: Option<&Arc<ExpressionContext>>,
) -> Option<[f64; 4]> {
    match context {
        Some(c) => inst.colour_at_with_context(id, lt, c.clone()),
        None => inst.colour_at(id, lt),
    }
}

/// The picture a footage control's rows hold.
fn picture(inst: &EffectInstance, id: &str) -> Option<Picture> {
    let row = param(inst, id)?;
    let EffectValue::Text(item) = &row.value else {
        return None;
    };
    let item = Uuid::parse_str(item).ok()?;
    let size = row.extra.get("size")?.as_array()?;
    let side = |n: usize| u32::try_from(size.get(n)?.as_u64()?).ok();
    let fit = match param(inst, &format!("{id}_fit")).map(|p| &p.value) {
        Some(EffectValue::Choice(c)) => Fit::from_choice(*c),
        _ => Fit::default(),
    };
    Some(Picture {
        item,
        size: (side(0)?, side(1)?),
        fit,
    })
}

/// The copy's own value for `control` at layer time `lt`, or `None` when the
/// copy follows the template there.
#[must_use]
pub fn value_of(
    inst: &EffectInstance,
    control: &Control,
    lt: f64,
    context: Option<&Arc<ExpressionContext>>,
) -> Option<Value> {
    value_with(inst, &held(inst), control, lt, context, None)
}

fn value_with(
    inst: &EffectInstance,
    kept: &Held,
    control: &Control,
    lt: f64,
    context: Option<&Arc<ExpressionContext>>,
    drivers: Option<&crate::fx::drivers::ResolvedDrivers>,
) -> Option<Value> {
    let id = control.id.as_str();
    Some(match control.kind {
        ControlKind::Number | ControlKind::Angle => {
            if !kept.has(id) {
                return None;
            }
            Value::Number(number(inst, id, lt, context, drivers)?)
        }
        ControlKind::Point => {
            let rows = row_ids(control);
            if !rows.iter().any(|r| kept.has(r)) {
                return None;
            }
            let axes: Option<Vec<f64>> = rows
                .iter()
                .map(|r| number(inst, r, lt, context, drivers))
                .collect();
            Value::Point(axes?)
        }
        ControlKind::Colour => {
            if !kept.has(id) {
                return None;
            }
            Value::Colour(colour(inst, id, lt, context)?)
        }
        ControlKind::Checkbox => match &param(inst, id).filter(|_| kept.has(id))?.value {
            EffectValue::Bool(b) => Value::Tick(*b),
            _ => return None,
        },
        ControlKind::Menu => match &param(inst, id).filter(|_| kept.has(id))?.value {
            EffectValue::Choice(c) => Value::Choice(*c),
            _ => return None,
        },
        ControlKind::Text => match &param(inst, id).filter(|_| kept.has(id))?.value {
            EffectValue::Text(t) => Value::Text(t.clone()),
            _ => return None,
        },
        ControlKind::Font => match &param(inst, id).filter(|_| kept.has(id))?.value {
            EffectValue::Text(t) => {
                let (family, face) = font_of(t);
                Value::Font { family, face }
            }
            _ => return None,
        },
        ControlKind::Footage => {
            if !kept.has(id) {
                return None;
            }
            Value::Picture(picture(inst, id)?)
        }
    })
}

/// Everything a copy sets in its template at layer time `lt`: each row and
/// the value to write there.
///
/// What a menu's chosen option sets comes first and the copy's own values
/// after, so a control set by hand wins over a look that sets the same row.
/// A menu the copy has not set applies nothing: the template already looks
/// the way its own option left it.
#[must_use]
pub fn settings_at(
    inst: &EffectInstance,
    template: &Template,
    lt: f64,
    context: Option<&Arc<ExpressionContext>>,
    drivers: Option<&crate::fx::drivers::ResolvedDrivers>,
) -> Vec<Setting> {
    let kept = held(inst);
    let mut looks: Vec<Setting> = Vec::new();
    let mut own: Vec<Setting> = Vec::new();
    for control in &template.controls {
        let id = control.id.as_str();
        if let Some(value) = value_with(inst, &kept, control, lt, context, drivers) {
            if let Value::Choice(option) = value {
                if let Some(look) = control.looks.get(option as usize) {
                    looks.extend(look.iter().cloned());
                }
            }
            own.push(Setting {
                target: control.target.clone(),
                value,
            });
        }
        if control.kind != ControlKind::Text {
            continue;
        }
        // A text control's further rows, each a row of the same layer.
        let layer = control.target.layer();
        let font = format!("{id}_font");
        if control.font && kept.has(&font) {
            if let Some(EffectValue::Text(t)) = param(inst, &font).map(|p| &p.value) {
                let (family, face) = font_of(t);
                own.push(Setting {
                    target: Target::Font { layer },
                    value: Value::Font { family, face },
                });
            }
        }
        let size = format!("{id}_size");
        if control.size && kept.has(&size) {
            if let Some(v) = number(inst, &size, lt, context, drivers) {
                own.push(Setting {
                    target: Target::TextRow {
                        layer,
                        value: TextRow::Size,
                    },
                    value: Value::Number(v),
                });
            }
        }
        let fill = format!("{id}_colour");
        if control.colour && kept.has(&fill) {
            if let Some(c) = colour(inst, &fill, lt, context) {
                own.push(Setting {
                    target: Target::TextRow {
                        layer,
                        value: TextRow::Fill,
                    },
                    value: Value::Colour(c),
                });
            }
        }
    }
    looks.extend(own);
    looks
}

/// Which ends of the template a copy plays. Both, unless it says otherwise.
#[must_use]
pub fn ends(inst: &EffectInstance) -> Ends {
    let kept = held(inst);
    let plays = |row: &str| {
        !kept.has(row)
            || !matches!(
                param(inst, row).map(|p| &p.value),
                Some(EffectValue::Bool(false))
            )
    };
    Ends {
        intro: plays(PLAY_INTRO),
        outro: plays(PLAY_OUTRO),
    }
}

/// One row a value is kept as: its id, what it holds, and a picture's size.
type KeptRow = (String, EffectValue, Option<(u32, u32)>);

/// The rows `value` is kept as for `control`, with what each holds.
fn rows_for(control: &Control, value: &Value) -> Option<Vec<KeptRow>> {
    let id = control.id.clone();
    let fixed = |v: f64| EffectValue::Float(Property::fixed(v));
    Some(match (control.kind, value) {
        (ControlKind::Number | ControlKind::Angle, Value::Number(v)) => {
            vec![(id, fixed(*v), None)]
        }
        (ControlKind::Point, Value::Point(axes)) => row_ids(control)
            .into_iter()
            .zip(axes)
            .map(|(row, v)| (row, fixed(*v), None))
            .collect(),
        (ControlKind::Colour, Value::Colour(c)) => {
            vec![(id, EffectValue::Colour(c.map(Property::fixed)), None)]
        }
        (ControlKind::Checkbox, Value::Tick(b)) => vec![(id, EffectValue::Bool(*b), None)],
        (ControlKind::Menu, Value::Choice(c)) => vec![(id, EffectValue::Choice(*c), None)],
        (ControlKind::Text, Value::Text(t)) => vec![(id, EffectValue::Text(t.clone()), None)],
        (ControlKind::Font, Value::Font { family, face }) => {
            vec![(id, EffectValue::Text(font_text(family, face)), None)]
        }
        (ControlKind::Footage, Value::Picture(p)) => vec![
            (
                id.clone(),
                EffectValue::Text(p.item.to_string()),
                Some(p.size),
            ),
            (
                format!("{id}_fit"),
                EffectValue::Choice(p.fit.to_choice()),
                None,
            ),
        ],
        _ => return None,
    })
}

/// Set `control` to `value` on a copy, as the copy's own. Answers whether the
/// value was the control's kind.
pub fn set_value(inst: &mut EffectInstance, control: &Control, value: &Value) -> bool {
    write(inst, control, value, false)
}

/// Set `control` to `value` on a copy as its data row's, unless the copy has
/// set that control itself. Answers whether the value was the control's kind.
pub fn feed_value(inst: &mut EffectInstance, control: &Control, value: &Value) -> bool {
    write(inst, control, value, true)
}

fn write(inst: &mut EffectInstance, control: &Control, value: &Value, fed: bool) -> bool {
    let Some(rows) = rows_for(control, value) else {
        return false;
    };
    let mut kept = held(inst);
    for (row, value, size) in rows {
        if fed && kept.own.contains(&row) {
            continue;
        }
        let stored = put(inst, row.clone(), value);
        if let Some((w, h)) = size {
            stored
                .extra
                .insert("size".to_owned(), serde_json::json!([w, h]));
        }
        mark(&mut kept, &row, fed);
    }
    set_held(inst, &kept);
    true
}

/// Feed one text control's font, size or colour on a copy from its data row,
/// by the row's id, unless the copy has set that row itself.
pub fn feed_row(inst: &mut EffectInstance, row: &str, value: &Value) {
    let value = match value {
        Value::Number(v) => EffectValue::Float(Property::fixed(*v)),
        Value::Colour(c) => EffectValue::Colour(c.map(Property::fixed)),
        Value::Font { family, face } => EffectValue::Text(font_text(family, face)),
        _ => return,
    };
    let mut kept = held(inst);
    if kept.own.iter().any(|r| r == row) {
        return;
    }
    put(inst, row.to_owned(), value);
    mark(&mut kept, row, true);
    set_held(inst, &kept);
}

/// Take away everything a copy's data row fed it, leaving what it set itself.
pub fn unfeed(inst: &mut EffectInstance) {
    let mut kept = held(inst);
    let fed = std::mem::take(&mut kept.fed);
    inst.params.retain(|p| !fed.contains(&p.id));
    set_held(inst, &kept);
}

/// Say which data row a copy follows, counting from 0, or that it follows
/// none.
pub fn follow(inst: &mut EffectInstance, row: Option<u32>) {
    let mut kept = held(inst);
    kept.row = row;
    set_held(inst, &kept);
}

/// Mark `row` as the copy's own, or as its data row's.
fn mark(kept: &mut Held, row: &str, fed: bool) {
    kept.own.retain(|r| r != row);
    kept.fed.retain(|r| r != row);
    if fed {
        kept.fed.push(row.to_owned());
    } else {
        kept.own.push(row.to_owned());
    }
}

/// Set one text control's font, size or colour on a copy, by the row's id.
pub fn set_row(inst: &mut EffectInstance, row: &str, value: EffectValue) {
    let mut kept = held(inst);
    put(inst, row.to_owned(), value);
    mark(&mut kept, row, false);
    set_held(inst, &kept);
}

/// Let `rows` follow the template again: the copy's own value and its data
/// row's are both dropped.
pub fn clear_rows(inst: &mut EffectInstance, rows: &[String]) {
    let mut kept = held(inst);
    kept.own.retain(|r| !rows.contains(r));
    kept.fed.retain(|r| !rows.contains(r));
    inst.params.retain(|p| !rows.contains(&p.id));
    set_held(inst, &kept);
}

/// The value a row is offered at when the copy follows the template: the
/// template's own, as one row holds it.
fn offered(
    doc: &Document,
    comp: &crate::model::Composition,
    control: &Control,
) -> Vec<(String, EffectValue)> {
    let mut rows: Vec<(String, EffectValue)> = Vec::new();
    if let Some(value) = crate::template::rows::read(doc, comp, &control.target) {
        if let Some(made) = rows_for(control, &value) {
            rows.extend(made.into_iter().map(|(row, value, _)| (row, value)));
        }
    }
    if control.kind == ControlKind::Text {
        let layer = control.target.layer();
        let id = &control.id;
        let read = |target: Target| crate::template::rows::read(doc, comp, &target);
        if control.font {
            if let Some(Value::Font { family, face }) = read(Target::Font { layer }) {
                rows.push((
                    format!("{id}_font"),
                    EffectValue::Text(font_text(&family, &face)),
                ));
            }
        }
        if control.size {
            if let Some(Value::Number(v)) = read(Target::TextRow {
                layer,
                value: TextRow::Size,
            }) {
                rows.push((format!("{id}_size"), EffectValue::Float(Property::fixed(v))));
            }
        }
        if control.colour {
            if let Some(Value::Colour(c)) = read(Target::TextRow {
                layer,
                value: TextRow::Fill,
            }) {
                rows.push((
                    format!("{id}_colour"),
                    EffectValue::Colour(c.map(Property::fixed)),
                ));
            }
        }
    }
    rows
}

/// Hand a copy's rows out ready to show: every row the copy has not set is
/// filled with the template's own value, and the template's values are noted
/// beside the rows ([`Held::live`]) so each row's declared default is what
/// the template has. A row reset to its default is then a row that follows
/// the template again. For the clones the bridge hands out: what is stored
/// keeps only the copy's own ([`settle`]).
pub fn offer(inst: &mut EffectInstance, doc: &Document) {
    let Some(comp) = comp_of(inst).and_then(|id| doc.comp(id)) else {
        return;
    };
    let Some(template) = comp.template() else {
        return;
    };
    let mut kept = held(inst);
    let mut live: Vec<(String, EffectValue)> = template
        .controls
        .iter()
        .flat_map(|control| offered(doc, comp, control))
        .collect();
    for (row, there) in [(PLAY_INTRO, kept.intro), (PLAY_OUTRO, kept.outro)] {
        if there {
            live.push((row.to_owned(), EffectValue::Bool(true)));
        }
    }
    for (row, value) in &live {
        if !kept.has(row) {
            put(inst, row.clone(), value.clone());
        }
    }
    kept.live = live;
    set_held(inst, &kept);
}

/// Make an edited instance ready to store: decide which rows are the copy's
/// own, and drop every row that follows the template, so the copy stores only
/// what it has set.
///
/// A row holding what the template holds follows it. A row holding anything
/// else is the copy's own, unless a data row fed it and it has not moved
/// since `before`, the instance as it is stored now. An instance that was
/// never handed out through [`offer`] keeps the marks it has.
pub fn settle(inst: &mut EffectInstance, before: Option<&EffectInstance>) {
    let mut kept = held(inst);
    let live = std::mem::take(&mut kept.live);
    if !live.is_empty() {
        let follows = |p: &EffectParam| {
            live.iter()
                .any(|(row, value)| *row == p.id && *value == p.value)
        };
        let still_fed = |p: &EffectParam| {
            kept.fed.contains(&p.id)
                && before
                    .and_then(|b| param(b, &p.id))
                    .is_some_and(|was| was.value == p.value && was.extra == p.extra)
        };
        let mut own: Vec<String> = Vec::new();
        let mut fed: Vec<String> = Vec::new();
        for p in &inst.params {
            if still_fed(p) {
                fed.push(p.id.clone());
            } else if !follows(p) {
                own.push(p.id.clone());
            }
        }
        // A point's axes are set together, and so are a picture and its fit.
        for control in &kept.controls {
            if !matches!(control.kind, ControlKind::Point | ControlKind::Footage) {
                continue;
            }
            let rows = row_ids(control);
            if rows.iter().any(|r| own.contains(r)) {
                for row in rows {
                    if !own.contains(&row) && inst.params.iter().any(|p| p.id == row) {
                        fed.retain(|r| *r != row);
                        own.push(row);
                    }
                }
            }
        }
        kept.own = own;
        kept.fed = fed;
    }
    inst.params.retain(|p| kept.has(&p.id));
    kept.own.retain(|r| inst.params.iter().any(|p| &p.id == r));
    kept.fed.retain(|r| inst.params.iter().any(|p| &p.id == r));
    set_held(inst, &kept);
}

/// Whether a copy keeps nothing of its own: no value set, none fed and no
/// data row followed. Such an instance need not be stored at all.
#[must_use]
pub fn is_bare(inst: &EffectInstance) -> bool {
    let kept = held(inst);
    kept.own.is_empty() && kept.fed.is_empty() && kept.row.is_none()
}

/// The id a copy's instance is handed out under before the copy has stored
/// one, made from the layer's or the clip's own id. The same every time, so
/// a drag's preview and its commit find the copy the instance belongs to.
#[must_use]
pub fn offered_id(host: Uuid) -> Uuid {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"template-values/");
    hasher.update(host.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    Uuid::from_bytes(bytes)
}

/// The Graphics template effect's behaviour.
pub struct TemplateValuesDef;

impl EffectDef for TemplateValuesDef {
    fn schema(&self) -> &'static EffectSchema {
        &<TemplateValues as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn derived(&self, inst: &EffectInstance) -> &'static [ParamSchema] {
        rows::rows_of(inst)
    }

    /// A menu's options are the template's own words, so they are handed over
    /// here and not written into the declaration.
    fn row_state(&self, inst: &EffectInstance) -> RowState {
        let kept = held(inst);
        let rows = rows::rows_of(inst);
        let options = kept
            .controls
            .iter()
            .filter(|c| c.kind == ControlKind::Menu)
            .filter_map(|c| {
                let row = rows.iter().find(|r| r.id == c.id)?;
                Some((row.id, c.options.clone()))
            })
            .collect();
        RowState {
            options,
            ..RowState::default()
        }
    }
}
