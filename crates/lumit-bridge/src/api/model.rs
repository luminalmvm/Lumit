//! 3D models: telling a model item from footage, and a Model layer's settings.

use flutter_rust_bridge::frb;
use lumit_core::model::{LayerKind, ProjectItem};
use lumit_core::model_layer::{ModelSettings, UpAxis};

use crate::api::{
    composition::CompositionReference, footage::FootageReference, layer::LayerReference,
    BridgeError,
};

/// Which of a 3D model file's axes points up.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeUpAxis {
    Y,
    Z,
}

/// How a Viewer view shows its models: lit as they export, in a studio
/// light of the view's own, as lines, both, or as the way they face.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeModelDisplay {
    Final,
    Shaded,
    Wireframe,
    ShadedWire,
    Normals,
}

/// How a Viewer view shows its models, and whether it tints their faces by
/// which way they face.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeModelLook {
    pub display: BridgeModelDisplay,
    pub faces: bool,
}

impl BridgeModelLook {
    /// The engine's own.
    #[frb(ignore)]
    pub(crate) fn engine(self) -> lumit_gpu::mesh::display::ModelDisplay {
        use lumit_gpu::mesh::display::Shading;
        let shading = match self.display {
            BridgeModelDisplay::Final => Shading::Final,
            BridgeModelDisplay::Shaded => Shading::Shaded,
            BridgeModelDisplay::Wireframe => Shading::Wireframe,
            BridgeModelDisplay::ShadedWire => Shading::ShadedWire,
            BridgeModelDisplay::Normals => Shading::Normals,
        };
        lumit_gpu::mesh::display::ModelDisplay {
            shading,
            faces: self.faces,
        }
    }
}

impl CompositionReference {
    /// Whether this comp draws a model, its own or inside a precomp.
    #[frb(sync)]
    pub fn holds_models(&self) -> Result<bool, BridgeError> {
        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        let mut seen = vec![self.id];
        let mut i = 0;
        // Each comp once, so a comp nested twice or in a loop is walked once.
        while let Some(&id) = seen.get(i) {
            i += 1;
            for layer in doc.comp(id).map_or(&[][..], |c| &c.layers) {
                match &layer.kind {
                    LayerKind::Model { .. } => return Ok(true),
                    LayerKind::Precomp { comp } if !seen.contains(comp) => seen.push(*comp),
                    _ => {}
                }
            }
        }
        Ok(false)
    }
}

/// How a 3D model sits in its Model layer, so its Scale stays at 100%.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeModelSettings {
    pub up_axis: BridgeUpAxis,
    /// Comp pixels for each metre of the model.
    pub units: f64,
    /// Sized to fill the comp and centred, in place of `units`.
    pub fit: bool,
}

impl FootageReference {
    /// Whether this item is a 3D model rather than footage that plays.
    #[frb(sync)]
    pub fn is_model(&self) -> Result<bool, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match proj.store.snapshot().item(self.id()) {
            Some(ProjectItem::Footage(f)) => lumit_scene::is_model(f.media.on_disk()),
            _ => false,
        })
    }

    /// The pictures this 3D model names that are missing or won't read, as
    /// it names them. Reads the model if nothing has yet, so never from a
    /// build.
    pub fn get_model_missing(&self) -> Result<Vec<String>, BridgeError> {
        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        Ok(lumit_render::model::missing(&doc, self.id()))
    }
}

impl LayerReference {
    /// A Model layer's settings, or `None` on any other kind of layer.
    #[frb(sync)]
    pub fn get_model_settings(&self) -> Result<Option<BridgeModelSettings>, BridgeError> {
        Ok(match self.item()?.kind {
            LayerKind::Model { settings, .. } => Some(BridgeModelSettings {
                up_axis: match settings.up_axis {
                    UpAxis::Y => BridgeUpAxis::Y,
                    UpAxis::Z => BridgeUpAxis::Z,
                },
                units: settings.units,
                fit: settings.fit,
            }),
            _ => None,
        })
    }

    /// The corners of a Model layer's box in its own pixels, three numbers
    /// each, once its file has been read. Empty before that and on any other
    /// kind of layer.
    #[frb(sync)]
    pub fn get_model_box(&self) -> Result<Vec<f64>, BridgeError> {
        let LayerKind::Model { item, settings } = self.item()?.kind else {
            return Ok(Vec::new());
        };
        let doc = self.snapshot()?;
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let size = (comp.width as f32, comp.height as f32);
        let corners = lumit_render::model::layer_box(&doc, item, &settings, size);
        Ok(corners
            .iter()
            .flatten()
            .flatten()
            .map(|v| f64::from(*v))
            .collect())
    }

    /// The comp pixels a metre Fit to comp gives a Model layer's model with
    /// `up_axis` up, once its file has been read. `None` before that and on
    /// any other kind of layer.
    #[frb(sync)]
    pub fn get_model_fitted_units(
        &self,
        up_axis: BridgeUpAxis,
    ) -> Result<Option<f64>, BridgeError> {
        let LayerKind::Model { item, .. } = self.item()?.kind else {
            return Ok(None);
        };
        let doc = self.snapshot()?;
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let up = match up_axis {
            BridgeUpAxis::Y => UpAxis::Y,
            BridgeUpAxis::Z => UpAxis::Z,
        };
        let size = (comp.width as f32, comp.height as f32);
        Ok(lumit_render::model::fitted_units(&doc, item, up, size).map(f64::from))
    }

    /// How far along the ray from `origin` by `direction`, three numbers
    /// each in the layer's own pixels, it first meets a Model layer's model,
    /// in parts of `direction`. `None` where it misses, before the file has
    /// been read and on any other kind of layer. Asked once a click.
    #[frb(sync)]
    pub fn get_model_hit(
        &self,
        origin: Vec<f64>,
        direction: Vec<f64>,
    ) -> Result<Option<f64>, BridgeError> {
        let LayerKind::Model { item, settings } = self.item()?.kind else {
            return Ok(None);
        };
        let (Ok([ox, oy, oz]), Ok([dx, dy, dz])) = (
            <[f64; 3]>::try_from(origin),
            <[f64; 3]>::try_from(direction),
        ) else {
            return Ok(None);
        };
        let doc = self.snapshot()?;
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        Ok(lumit_render::model::hit(
            &doc,
            item,
            &settings,
            (comp.width as f32, comp.height as f32),
            [ox, oy, oz].map(|v| v as f32),
            [dx, dy, dz].map(|v| v as f32),
        )
        .map(f64::from))
    }

    /// The document as it is now.
    #[frb(ignore)]
    fn snapshot(&self) -> Result<std::sync::Arc<lumit_core::model::Document>, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(proj.store.snapshot())
    }

    /// Replace a Model layer's settings, as one undo step. Refused on any
    /// other kind of layer.
    #[frb(sync)]
    pub fn set_model_settings(&self, settings: BridgeModelSettings) -> Result<(), BridgeError> {
        let LayerKind::Model {
            item,
            settings: kept,
        } = self.item()?.kind
        else {
            return Err(BridgeError::InvalidLayer);
        };
        let settings = ModelSettings {
            up_axis: match settings.up_axis {
                BridgeUpAxis::Y => UpAxis::Y,
                BridgeUpAxis::Z => UpAxis::Z,
            },
            // Nought would shrink the model to nothing and no further.
            units: settings.units.max(f64::EPSILON),
            fit: settings.fit,
            materials: kept.materials,
        };
        self.commit(lumit_core::ops::Op::SetLayerKind {
            comp: self.comp_id,
            layer: self.layer_id,
            kind: Box::new(LayerKind::Model {
                item,
                settings: Box::new(settings),
            }),
        })
    }
}
