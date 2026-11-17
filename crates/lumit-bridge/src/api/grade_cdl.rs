//! ASC CDL in and out: a `.cdl`, `.cc` or `.ccc` brought in as a CDL node on
//! a shot, and a CDL node, or a Correction's grade, written out as a `.cdl`
//! or `.cc`.
//!
//! A shot is a layer, or with `clip` one clip on a Sequence layer.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::composition::CompositionReference;
use crate::api::grade_versions::grade_edit;
use crate::api::layer::LayerReference;
use crate::api::BridgeError;
use crate::cdl::{self, CdlValues};
use lumit_core::anim::Property;
use lumit_core::fx::effects::cdl::{Cdl, CDL};
use lumit_core::fx::effects::correction::Correction;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::grade::{self, GradeError};
use lumit_core::grade_solve::cdl_fit;
use lumit_core::model::{EffectInstance, EffectValue};

/// `inst`'s colour row `id` set to `rgb`, alpha 1.
fn set_colour(inst: &mut EffectInstance, id: &str, rgb: [f32; 3]) {
    if let Some(p) = inst.params.iter_mut().find(|p| p.id == id) {
        p.value = EffectValue::Colour([
            Property::fixed(f64::from(rgb[0])),
            Property::fixed(f64::from(rgb[1])),
            Property::fixed(f64::from(rgb[2])),
            Property::fixed(1.0),
        ]);
    }
}

impl LayerReference {
    /// A CDL node holding the grade in the `.cdl`, `.cc` or `.ccc` at `path`:
    /// the ColorCorrection whose id is `id`, or the first. It goes right
    /// after effect `after`, or with `None` after the shot's last grade node,
    /// or at the end of its stack when it has none. One undo step; answers
    /// the new node's id. A file with no such block, or numbers that don't
    /// read, is refused as unreadable.
    pub fn import_cdl(
        &self,
        clip: Option<Uuid>,
        after: Option<Uuid>,
        path: String,
        id: Option<String>,
    ) -> Result<Uuid, BridgeError> {
        let text = std::fs::read_to_string(&path).map_err(|_| BridgeError::ReadFailed)?;
        let v = cdl::parse(&text, id.as_deref()).ok_or(BridgeError::ReadFailed)?;
        let mut inst =
            lumit_core::fx::builtins::instantiate(CDL).ok_or(BridgeError::UnknownEffectName)?;
        set_colour(&mut inst, "slope", v.slope);
        set_colour(&mut inst, "offset", v.offset);
        set_colour(&mut inst, "power", v.power);
        if let Some(p) = inst.params.iter_mut().find(|p| p.id == "saturation") {
            p.value = EffectValue::Float(Property::fixed(f64::from(v.saturation)));
        }
        let new = inst.id;
        let shot = self.shot(clip);
        grade_edit(&*self.project()?, "Edit effects", |doc| {
            let ((), op) = grade::edit(doc, shot, |effects, _| {
                let at = match after {
                    Some(after) => effects
                        .iter()
                        .position(|e| e.id == after)
                        .map(|i| i + 1)
                        .ok_or(GradeError::UnknownShot)?,
                    None => effects
                        .iter()
                        .rposition(grade::is_grade)
                        .map_or(effects.len(), |i| i + 1),
                };
                effects.insert(at, inst);
                Ok(())
            })?;
            Ok((new, op))
        })
    }
}

impl CompositionReference {
    /// Write effect `node` of shot `layer`/`clip`, its rows read at comp
    /// `frame`, to `path` as a CDL: a decision list when `path` ends `.cdl`,
    /// one ColorCorrection otherwise. A CDL node writes its own numbers. A
    /// Correction writes its grade as one: exactly when only its wheels and
    /// saturation have moved, otherwise a least-squares fit, both in the
    /// node's grading space and as if its key took the whole picture. Answers
    /// whether the numbers are exact. Any other effect is refused.
    pub fn export_cdl(
        &self,
        layer: Uuid,
        clip: Option<Uuid>,
        frame: i64,
        node: Uuid,
        path: String,
    ) -> Result<bool, BridgeError> {
        let correction = <Correction as EffectMetadata>::SCHEMA.match_name;
        let (mut values, exact) = self.read_node(layer, clip, frame, node, |name, p| {
            if name == CDL {
                let pk = Cdl::read(p).packed();
                Some((
                    CdlValues {
                        id: None,
                        slope: pk.slope,
                        offset: pk.offset,
                        power: pk.power,
                        saturation: pk.saturation,
                    },
                    true,
                ))
            } else if name == correction {
                let fit = cdl_fit::cdl_of(&Correction::read(p));
                Some((
                    CdlValues {
                        id: None,
                        slope: fit.slope,
                        offset: fit.offset,
                        power: fit.power,
                        saturation: fit.saturation,
                    },
                    fit.exact,
                ))
            } else {
                None
            }
        })?;
        values.id = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_owned);
        let list = path.to_ascii_lowercase().ends_with(".cdl");
        let text = cdl::write(&values, list);
        let part = format!("{path}.part");
        let written = std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, &path));
        if written.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        written.map_err(|_| BridgeError::WriteFailed)?;
        Ok(exact)
    }
}

/// The ids of the ColorCorrections in the CDL file at `path`, in order, for
/// picking one out of a collection. Blank for a block with no id.
#[frb(sync)]
pub fn cdl_ids(path: String) -> Result<Vec<String>, BridgeError> {
    let text = std::fs::read_to_string(&path).map_err(|_| BridgeError::ReadFailed)?;
    cdl::ids(&text).ok_or(BridgeError::ReadFailed)
}
