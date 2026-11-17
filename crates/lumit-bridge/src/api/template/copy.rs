//! A placed copy of a graphics template: the Precomp layer or the clip that
//! places it, what it has set for itself, and the edits that set, clear and
//! hand back a value.
//!
//! A copy's rows are an effect instance
//! (`lumit_core::fx::effects::template`), so the rows that key are read and
//! written through the same staged copies every effect's are
//! (`LayerReference::get_graph_inputs`, `get_clip_inputs`, `set_effects`).
//! What is here is the rest: what the interface needs to draw the card, and
//! the values that do not key.
//!
//! Runs on the UI thread, like every other read and edit.

use flutter_rust_bridge::frb;
use lumit_core::fx::effects::template as values;
use lumit_core::model::{Composition, Document, EffectInstance, Layer, LayerKind, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::sequence::{Clip, ClipSource};
use lumit_core::template::{author, rows, ControlKind, Setting, Template, Value};
use uuid::Uuid;

use super::{BridgeControlKind, BridgeControlValue};
use crate::api::{
    composition::CompositionReference, effect::BridgeEffectInstance, layer::LayerReference,
    BridgeError,
};

/// One control of a copy, as its card draws it. The values are in the
/// copy's staged rows, under `rows`.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCopyControl {
    pub id: String,
    pub label: String,
    pub kind: BridgeControlKind,
    pub group: Option<String>,
    /// The ids of the rows it keeps on the copy, the control's own first.
    pub rows: Vec<String>,
    /// A menu's options.
    pub options: Vec<String>,
    /// The copy has set this control itself.
    pub own: bool,
    /// The copy's data row feeds this control.
    pub fed: bool,
}

/// A copy of a graphics template.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCopy {
    /// The template's composition and its name.
    pub comp: Uuid,
    pub name: String,
    /// What the copy's first text control says, which is what the copy is
    /// called until somebody renames it. `None` with no text control.
    pub title: Option<String>,
    pub controls: Vec<BridgeCopyControl>,
    /// The data row the copy follows, counting from 0, and how many rows the
    /// template's data has. Nought is no data.
    pub row: Option<u32>,
    pub data_rows: u32,
    /// Whether the template has an intro and an outro, and so the copy a
    /// tick for playing each.
    pub intro: bool,
    pub outro: bool,
    /// How long the intro and the outro run, in seconds, as a copy's bar
    /// shades them. Nought for one the template has not got, or the copy
    /// does not play.
    pub intro_seconds: f64,
    pub outro_seconds: f64,
}

/// The size in pixels of the picture `item` is: a footage item's own, as its
/// file says, or a composition's. Nought by nought when it cannot be told.
#[frb(ignore)]
pub(crate) fn picture_size(
    project: Uuid,
    doc: &Document,
    comp: &Composition,
    item: Uuid,
) -> Result<(u32, u32), BridgeError> {
    match doc.item(item) {
        Some(ProjectItem::Composition(c)) => Ok((c.width, c.height)),
        Some(ProjectItem::Footage(footage)) => {
            let holder = CompositionReference {
                project,
                id: comp.id,
            };
            let state = holder.project()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            let (_, w, h) = CompositionReference::footage_span_and_size(&state, footage, comp);
            Ok((w.round().max(0.0) as u32, h.round().max(0.0) as u32))
        }
        _ => Ok((0, 0)),
    }
}

/// The template composition `layer` places, when it places one.
#[frb(ignore)]
pub(crate) fn placed_template_of(doc: &Document, layer: &Layer) -> Option<Uuid> {
    let LayerKind::Precomp { comp } = layer.kind else {
        return None;
    };
    doc.comp(comp)?.is_template().then_some(comp)
}

/// The template composition `clip` places, when it places one.
#[frb(ignore)]
pub(crate) fn clip_template_of(doc: &Document, clip: &Clip) -> Option<Uuid> {
    let ClipSource::Comp(comp) = clip.source else {
        return None;
    };
    doc.comp(comp)?.is_template().then_some(comp)
}

/// A copy's rows ready to show: what it has stored, or a fresh instance when
/// it has stored nothing, with the template's own values in every row the
/// copy has not set. `host` is the layer's or the clip's id, which a fresh
/// instance's id is made from.
#[frb(ignore)]
pub(crate) fn shown(
    doc: &Document,
    template_comp: Uuid,
    stored: Option<&EffectInstance>,
    host: Uuid,
) -> Option<EffectInstance> {
    let template = doc.comp(template_comp)?.template()?;
    let mut inst = match stored.filter(|i| values::comp_of(i) == Some(template_comp)) {
        Some(stored) => stored.clone(),
        None => {
            let mut fresh = values::instance_for(template_comp, &template);
            fresh.id = values::offered_id(host);
            fresh
        }
    };
    values::refresh(&mut inst, doc);
    values::offer(&mut inst, doc);
    Some(inst)
}

/// An edited copy's rows ready to store: only what the copy has set, and
/// nothing at all for a copy that has set nothing. Any other instance is
/// stored as it is.
#[frb(ignore)]
pub(crate) fn stored(
    mut inst: EffectInstance,
    before: Option<&EffectInstance>,
) -> Option<EffectInstance> {
    if values::comp_of(&inst).is_none() {
        return Some(inst);
    }
    values::settle(&mut inst, before);
    (!values::is_bare(&inst)).then_some(inst)
}

/// The clip of Sequence `layer` a copy's staged rows belong to: by the id it
/// has stored, or the id a clip that has stored nothing is offered.
#[frb(ignore)]
pub(crate) fn clip_of_inputs(layer: &Layer, inst: &EffectInstance) -> Option<Uuid> {
    values::comp_of(inst)?;
    let LayerKind::Sequence { clips } = &layer.kind else {
        return None;
    };
    clips
        .iter()
        .find(|c| match &c.inputs {
            Some(held) => held.id == inst.id,
            None => values::offered_id(c.id) == inst.id,
        })
        .map(|c| c.id)
}

/// What the interface draws a copy from.
#[frb(ignore)]
pub(crate) fn bridge_copy(
    doc: &Document,
    template_comp: Uuid,
    stored: Option<&EffectInstance>,
) -> Option<BridgeCopy> {
    let comp = doc.comp(template_comp)?;
    let template = comp.template()?;
    let kept = stored
        .filter(|i| values::comp_of(i) == Some(template_comp))
        .map(values::held)
        .unwrap_or_default();
    let controls = template
        .controls
        .iter()
        .map(|c| {
            let rows = values::row_ids(c);
            BridgeCopyControl {
                id: c.id.clone(),
                label: c.label.clone(),
                kind: BridgeControlKind::of(c.kind),
                group: c.group.clone(),
                options: c.options.clone(),
                own: rows.iter().any(|r| kept.own.contains(r)),
                fed: rows.iter().any(|r| kept.fed.contains(r)),
                rows,
            }
        })
        .collect();
    let ends = stored.map_or(lumit_core::template::time::Ends::BOTH, values::ends);
    Some(BridgeCopy {
        comp: template_comp,
        name: comp.name.clone(),
        title: title(doc, comp, &template, stored),
        controls,
        row: kept.row,
        data_rows: comp
            .template_data()
            .map_or(0, |d| d.table.rows.len() as u32),
        intro: template.intro.is_some(),
        outro: template.outro.is_some(),
        intro_seconds: template
            .intro
            .filter(|_| ends.intro)
            .map_or(0.0, |t| t.to_f64()),
        outro_seconds: template
            .outro
            .filter(|_| ends.outro)
            .map_or(0.0, |t| (comp.duration.0.to_f64() - t.to_f64()).max(0.0)),
    })
}

/// What a copy's first text control says: the copy's own words, or the
/// template's.
fn title(
    doc: &Document,
    comp: &Composition,
    template: &Template,
    stored: Option<&EffectInstance>,
) -> Option<String> {
    let control = template
        .controls
        .iter()
        .find(|c| c.kind == ControlKind::Text)?;
    let own = stored.and_then(|inst| values::value_of(inst, control, 0.0, None));
    match own.or_else(|| rows::read(doc, comp, &control.target))? {
        Value::Text(words) => {
            let line = words.lines().next().unwrap_or_default().trim().to_owned();
            (!line.is_empty()).then_some(line)
        }
        _ => None,
    }
}

impl LayerReference {
    /// The template this layer, or its clip `clip`, places, with what the
    /// copy has stored. `None` when it places no template.
    #[frb(ignore)]
    fn copy_of(
        &self,
        doc: &Document,
        clip: Option<Uuid>,
    ) -> Result<Option<(Uuid, Option<EffectInstance>)>, BridgeError> {
        let layer = self.item()?;
        Ok(match clip {
            None => placed_template_of(doc, &layer).map(|comp| (comp, layer.graph_inputs.clone())),
            Some(clip) => {
                let (clips, index) = self.clips_and_index(clip)?;
                clip_template_of(doc, &clips[index]).map(|comp| (comp, clips[index].inputs.clone()))
            }
        })
    }

    /// Store a copy's rows: on the layer, or on its clip.
    #[frb(ignore)]
    fn store_copy(
        &self,
        clip: Option<Uuid>,
        inst: Option<EffectInstance>,
    ) -> Result<(), BridgeError> {
        match clip {
            None => {
                let proj = self.project()?;
                let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
                proj.store
                    .commit(Op::SetLayerGraphInputs {
                        comp: self.comp_id,
                        layer: self.layer_id,
                        inputs: inst.map(Box::new),
                    })
                    .map_err(BridgeError::OpError)?;
                Ok(())
            }
            Some(clip) => {
                let (mut clips, index) = self.clips_and_index(clip)?;
                clips[index].inputs = inst;
                self.commit_clips(clips)
            }
        }
    }

    /// Change what a copy has stored. `edit` is handed the template and the
    /// copy's stored rows, or a fresh instance when it has stored none.
    #[frb(ignore)]
    pub(crate) fn edit_copy(
        &self,
        clip: Option<Uuid>,
        edit: impl FnOnce(&Document, &Template, &mut EffectInstance) -> Result<(), BridgeError>,
    ) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let (comp, held) = self.copy_of(&doc, clip)?.ok_or(BridgeError::InvalidItem)?;
        let template = doc
            .comp(comp)
            .and_then(|c| c.template())
            .ok_or(BridgeError::InvalidItem)?;
        let mut inst = match held.filter(|i| values::comp_of(i) == Some(comp)) {
            Some(held) => held,
            None => {
                // Under the id the copy's rows were handed out with, so
                // what the interface holds still names them.
                let mut fresh = values::instance_for(comp, &template);
                fresh.id = values::offered_id(clip.unwrap_or(self.layer_id));
                fresh
            }
        };
        values::refresh(&mut inst, &doc);
        edit(&doc, &template, &mut inst)?;
        let keep = (!values::is_bare(&inst)).then_some(inst);
        self.store_copy(clip, keep)
    }

    /// What this layer, or its clip `clip`, is a copy of, as its card draws
    /// it. `None` when it places no graphics template.
    #[frb(sync)]
    pub fn get_copy(&self, clip: Option<Uuid>) -> Result<Option<BridgeCopy>, BridgeError> {
        let doc = self.document()?;
        Ok(self
            .copy_of(&doc, clip)?
            .and_then(|(comp, held)| bridge_copy(&doc, comp, held.as_ref())))
    }

    /// The rows of the template this clip places, as a staged copy to drag,
    /// key and commit through [`Self::set_effects`], as
    /// [`Self::get_graph_inputs`] hands a layer's. On the clip's own clock, or
    /// with `on_comp_clock` on the composition's, as [`Self::get_clip_effects`]
    /// crosses a clip's effects.
    #[frb(sync)]
    pub fn get_clip_inputs(
        &self,
        clip: Uuid,
        on_comp_clock: bool,
    ) -> Result<Option<BridgeEffectInstance>, BridgeError> {
        let doc = self.document()?;
        let (clips, index) = self.clips_and_index(clip)?;
        let Some(comp) = clip_template_of(&doc, &clips[index]) else {
            return Ok(None);
        };
        let offset = if on_comp_clock {
            self.item()?
                .start_offset
                .0
                .checked_add(clips[index].place_start)
                .map_err(|_| BridgeError::InvalidTime)?
        } else {
            lumit_core::time::Rational::ZERO
        };
        Ok(shown(&doc, comp, clips[index].inputs.as_ref(), clip)
            .map(|inst| BridgeEffectInstance::new(inst, offset)))
    }

    /// Set a control on this copy to a value that does not key: words, a
    /// font, a picture, a tick, a menu's option. A number, a point or a
    /// colour set this way is fixed, in place of any keys the copy had.
    #[frb(sync)]
    pub fn set_copy_value(
        &self,
        control: String,
        value: BridgeControlValue,
        clip: Option<Uuid>,
    ) -> Result<(), BridgeError> {
        let size = match &value {
            BridgeControlValue::Picture { item, .. } => {
                let doc = self.document()?;
                let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
                picture_size(self.project_id, &doc, comp, *item)?
            }
            _ => (0, 0),
        };
        let value = value.core(size)?;
        self.edit_copy(clip, |_, template, inst| {
            let control = template
                .control(&control)
                .ok_or(BridgeError::InvalidParam)?;
            if !values::set_value(inst, control, &value) {
                return Err(BridgeError::ParamKindMismatch);
            }
            Ok(())
        })
    }

    /// Set one of a text control's further rows on this copy: its font.
    #[frb(sync)]
    pub fn set_copy_font(
        &self,
        row: String,
        family: String,
        face: String,
        clip: Option<Uuid>,
    ) -> Result<(), BridgeError> {
        self.edit_copy(clip, |_, _, inst| {
            values::set_row(
                inst,
                &row,
                lumit_core::model::EffectValue::Text(format!("{family}\n{face}")),
            );
            Ok(())
        })
    }

    /// Let rows of this copy follow the template again. An empty list is
    /// every row, which is the card's **Reset**.
    #[frb(sync)]
    pub fn clear_copy_rows(
        &self,
        rows: Vec<String>,
        clip: Option<Uuid>,
    ) -> Result<(), BridgeError> {
        self.edit_copy(clip, |_, template, inst| {
            let rows = if rows.is_empty() {
                let mut all: Vec<String> =
                    template.controls.iter().flat_map(values::row_ids).collect();
                all.extend(
                    lumit_core::template::RESERVED_IDS
                        .iter()
                        .map(|r| (*r).to_owned()),
                );
                all
            } else {
                rows
            };
            values::clear_rows(inst, &rows);
            Ok(())
        })
    }

    /// **Set as default**: write this copy's value for `control` into the
    /// template, for every copy that has not set its own, and let this copy
    /// follow the template again. One undo step.
    #[frb(sync)]
    pub fn set_copy_as_default(
        &self,
        control: String,
        clip: Option<Uuid>,
    ) -> Result<(), BridgeError> {
        let doc = self.document()?;
        let (comp, held) = self.copy_of(&doc, clip)?.ok_or(BridgeError::InvalidItem)?;
        let template = doc
            .comp(comp)
            .and_then(|c| c.template())
            .ok_or(BridgeError::InvalidItem)?;
        let named = template
            .control(&control)
            .ok_or(BridgeError::InvalidParam)?;
        let Some(mut inst) = held else {
            return Ok(());
        };
        // A picture is fitted inside a composition made for the copy, which
        // the template itself has no use for.
        let Some(value) = values::value_of(&inst, named, 0.0, None)
            .filter(|_| named.kind != ControlKind::Footage)
        else {
            return Ok(());
        };
        let mut ops = author::ops(
            &doc,
            comp,
            &[Setting {
                target: named.target.clone(),
                value,
            }],
        );
        values::clear_rows(&mut inst, &values::row_ids(named));
        let keep = (!values::is_bare(&inst)).then_some(inst);
        match clip {
            None => ops.push(Op::SetLayerGraphInputs {
                comp: self.comp_id,
                layer: self.layer_id,
                inputs: keep.map(Box::new),
            }),
            Some(clip) => {
                let (mut clips, index) = self.clips_and_index(clip)?;
                clips[index].inputs = keep;
                ops.push(Op::SetSequenceClips {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    clips,
                });
            }
        }
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }
}
