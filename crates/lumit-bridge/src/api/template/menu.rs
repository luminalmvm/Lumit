//! Menu controls of a graphics template: making one, changing its options,
//! and recording what each option sets.
//!
//! Runs on the UI thread, like every other read and edit.

use std::sync::Mutex;

use flutter_rust_bridge::frb;
use lumit_core::fx::effects::custom_controls;
use lumit_core::model::{Composition, EffectInstance};
use lumit_core::ops::Op;
use lumit_core::template::{author, Control, ControlKind, Target};
use uuid::Uuid;

use super::{BridgeControlValue, BridgeRecording};
use crate::api::{composition::CompositionReference, BridgeError};

/// A menu option being recorded: the composition as it stood when recording
/// began. One at a time, as one hand records.
struct Recording {
    project: Uuid,
    comp: Uuid,
    menu: String,
    option: u32,
    was: Composition,
}

static RECORDING: Mutex<Option<Recording>> = Mutex::new(None);

impl CompositionReference {
    /// Make a menu control: a Menu row in a Custom controls effect, on the
    /// layer that already carries one or on a new Null layer called
    /// `layer_name`, and a control for it. Answers the control's id.
    #[frb(sync)]
    pub fn add_menu_control(
        &self,
        label: String,
        options: Vec<String>,
        layer_name: String,
    ) -> Result<String, BridgeError> {
        let label = label.trim().to_owned();
        if label.is_empty() || options.is_empty() {
            return Err(BridgeError::EmptyName);
        }
        let comp = self.composition()?;
        let mut template = self.template_or_new()?;
        let holds = |e: &&EffectInstance| e.effect.match_name == "custom_controls";
        let host = comp
            .layers
            .iter()
            .find(|l| l.effects.iter().any(|e| holds(&e)));
        let mut ops: Vec<Op> = Vec::new();
        let (layer, mut inst, mut effects) = match host {
            Some(layer) => {
                let inst = layer
                    .effects
                    .iter()
                    .find(holds)
                    .cloned()
                    .ok_or(BridgeError::InvalidEffect)?;
                (layer.id, inst, layer.effects.clone())
            }
            None => {
                let mut layer = lumit_core::edits::null_layer(&comp);
                layer.name = layer_name;
                let inst = lumit_core::fx::builtins::instantiate("custom_controls")
                    .ok_or(BridgeError::UnknownEffectName)?;
                let id = layer.id;
                ops.push(Op::AddLayer {
                    comp: self.id,
                    index: 0,
                    layer: Box::new(layer),
                });
                (id, inst, Vec::new())
            }
        };
        let mut controls = custom_controls::controls_of(&inst);
        let stem = template.free_id(&label);
        let row = (1u32..)
            .map(|n| {
                if n == 1 {
                    stem.clone()
                } else {
                    format!("{stem}_{n}")
                }
            })
            .find(|id| !controls.iter().any(|c| &c.id == id))
            .unwrap_or_else(|| stem.clone());
        controls.push(custom_controls::Control {
            id: row.clone(),
            label: label.clone(),
            kind: custom_controls::ControlKind::Menu,
            default: [0.0; 4],
            min: 0.0,
            max: 0.0,
            options: options.clone(),
        });
        custom_controls::set_controls(&mut inst, &controls);
        match effects.iter_mut().find(|e| e.id == inst.id) {
            Some(held) => *held = inst.clone(),
            None => effects.push(inst.clone()),
        }
        ops.push(Op::SetLayerEffects {
            comp: self.id,
            layer,
            effects,
        });
        let mut control = Control::new(
            stem.clone(),
            label,
            ControlKind::Menu,
            Target::Param {
                layer,
                effect: inst.id,
                param: row,
            },
        );
        control.options = options;
        template.controls.push(control);
        ops.push(self.template_op(template));
        self.commit(Op::Batch { ops })?;
        Ok(stem)
    }

    /// Change a menu control's options: their names, in order. An option
    /// taken off the end takes its look with it. Only for a menu made by
    /// [`Self::add_menu_control`], whose row lists the control's own words.
    #[frb(sync)]
    pub fn set_menu_options(&self, id: String, options: Vec<String>) -> Result<(), BridgeError> {
        if options.is_empty() {
            return Err(BridgeError::EmptyName);
        }
        let comp = self.composition()?;
        let mut template = self.template_or_new()?;
        let control = template
            .controls
            .iter_mut()
            .find(|c| c.id == id && c.kind == ControlKind::Menu)
            .ok_or(BridgeError::InvalidParam)?;
        let Target::Param {
            layer,
            effect,
            param,
        } = control.target.clone()
        else {
            return Err(BridgeError::InvalidParam);
        };
        let host = comp
            .layers
            .iter()
            .find(|l| l.id == layer)
            .ok_or(BridgeError::InvalidLayer)?;
        let mut effects = host.effects.clone();
        let inst = effects
            .iter_mut()
            .find(|e| e.id == effect && e.effect.match_name == "custom_controls")
            .ok_or(BridgeError::InvalidEffect)?;
        let mut controls = custom_controls::controls_of(inst);
        let row = controls
            .iter_mut()
            .find(|c| c.id == param)
            .ok_or(BridgeError::InvalidParam)?;
        row.options.clone_from(&options);
        custom_controls::set_controls(inst, &controls);
        // A choice past the new end falls back to the last option.
        let last = options.len() as u32 - 1;
        for p in inst.params.iter_mut().filter(|p| p.id == param) {
            if let lumit_core::model::EffectValue::Choice(c) = &mut p.value {
                *c = (*c).min(last);
            }
        }
        control.looks.truncate(options.len());
        control.options = options;
        self.commit(Op::Batch {
            ops: vec![
                Op::SetLayerEffects {
                    comp: self.id,
                    layer,
                    effects,
                },
                self.template_op(template),
            ],
        })
    }

    /// Start recording what option `option` of menu `id` sets: the option is
    /// picked, so the composition shows its look, and every row changed from
    /// here until [`Self::stop_recording`] becomes part of it.
    #[frb(sync)]
    pub fn start_recording(&self, id: String, option: u32) -> Result<(), BridgeError> {
        self.set_control_value(id.clone(), BridgeControlValue::Choice(option))?;
        let was = self.composition()?;
        let mut recording = RECORDING.lock().map_err(|_| BridgeError::WriteFailed)?;
        *recording = Some(Recording {
            project: self.project,
            comp: self.id,
            menu: id,
            option,
            was,
        });
        Ok(())
    }

    /// Stop recording, keeping what was changed as the option's look, or
    /// with `keep` off dropping it. The rows stay as they were left.
    #[frb(sync)]
    pub fn stop_recording(&self, keep: bool) -> Result<(), BridgeError> {
        let taken = RECORDING
            .lock()
            .map_err(|_| BridgeError::WriteFailed)?
            .take();
        let Some(recording) = taken.filter(|r| r.project == self.project && r.comp == self.id)
        else {
            return Ok(());
        };
        if !keep {
            return Ok(());
        }
        let doc = self.document()?;
        let now = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let template = now.template().ok_or(BridgeError::InvalidParam)?;
        let Some(next) = author::recorded(
            &doc,
            &template,
            &recording.menu,
            recording.option as usize,
            &recording.was,
            now,
        ) else {
            return Err(BridgeError::InvalidParam);
        };
        if next == *template {
            return Ok(());
        }
        self.commit(self.template_op(next))
    }

    /// Take away what option `option` of menu `id` sets.
    #[frb(sync)]
    pub fn clear_look(&self, id: String, option: u32) -> Result<(), BridgeError> {
        self.edit_template(|t| {
            let control = t
                .controls
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or(BridgeError::InvalidParam)?;
            if let Some(look) = control.looks.get_mut(option as usize) {
                look.clear();
            }
            Ok(())
        })
    }
}

/// The menu option being recorded in composition `comp` of `project`, when
/// one is.
#[frb(ignore)]
pub(crate) fn recording_of(project: Uuid, comp: Uuid) -> Option<BridgeRecording> {
    RECORDING.lock().ok().and_then(|r| {
        r.as_ref()
            .filter(|r| r.project == project && r.comp == comp)
            .map(|r| BridgeRecording {
                menu: r.menu.clone(),
                option: r.option,
            })
    })
}
