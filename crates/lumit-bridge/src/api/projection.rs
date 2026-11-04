//! Projecting a picture on a Model layer in one step.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::layer::{InstanceHome, LayerReference};
use crate::api::BridgeError;

impl LayerReference {
    /// The camera and picture Project from camera would use on this Model
    /// layer at comp frame `frame`, `None` while it can't.
    #[frb(ignore)]
    fn projection_source(&self, selected: &[Uuid], frame: i64) -> Option<(Uuid, Uuid)> {
        let (doc, layer) = (self.document().ok()?, self.item().ok()?);
        lumit_render::model_material::rows(&doc, &layer)?;
        let comp = doc.comp(self.comp_id)?;
        let t = comp.frame_rate.time_of_frame(frame).ok()?.0.to_f64();
        lumit_core::model_material::projection_source(comp, self.layer_id, selected, t)
    }

    /// Whether Project from camera has a camera and a picture to use.
    #[frb(sync)]
    pub fn can_project_from_camera(&self, selected: Vec<Uuid>, frame: i64) -> bool {
        self.projection_source(&selected, frame).is_some()
    }

    /// Project a picture on every material of this Model layer, as one undo
    /// step.
    #[frb(sync)]
    pub fn project_from_camera(&self, selected: Vec<Uuid>, frame: i64) -> Result<(), BridgeError> {
        let (camera, picture) = self
            .projection_source(&selected, frame)
            .ok_or(BridgeError::InvalidLayer)?;
        self.with_instances(InstanceHome::Materials, move |list| {
            for rows in list {
                lumit_core::model_material::set_projection(rows, picture, camera);
            }
            Ok(())
        })
    }
}
