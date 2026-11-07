//! The built-in presets an effect's menu offers ([`lumit_core::fx::presets`]).

use flutter_rust_bridge::frb;

use crate::api::{effect::BridgeEffectInstance, layer::LayerReference, BridgeError};

impl BridgeEffectInstance {
    /// The names of the presets this effect ships with, in menu order. Engine
    /// labels, so the panel translates them. Empty for most effects.
    #[frb(sync)]
    pub fn preset_names(&self) -> Vec<String> {
        lumit_core::fx::presets::presets(&self.effect.effect.match_name)
            .iter()
            .map(|p| p.name.to_owned())
            .collect()
    }
}

impl LayerReference {
    /// Set `effect` to the `preset`-th of its built-in presets, as one undo
    /// step. Any keyframes start at comp frame `at_frame`, the playhead.
    #[frb(sync)]
    pub fn apply_effect_preset(
        &self,
        effect: &BridgeEffectInstance,
        preset: u32,
        at_frame: i64,
    ) -> Result<(), BridgeError> {
        let id = effect.id();
        let chosen = lumit_core::fx::presets::presets(&effect.effect.effect.match_name)
            .get(preset as usize)
            .copied()
            .ok_or(BridgeError::InvalidEffect)?;
        let comp = self.composition()?;
        let raster = (f64::from(comp.width), f64::from(comp.height));
        let layer = self.item()?;
        // A 3D model's material starts from its file's own values.
        let file = lumit_render::model_material::file_rows(&*self.document()?, &layer, id);
        let at = comp
            .frame_rate
            .time_of_frame(at_frame.max(0))
            .and_then(|t| t.delta(layer.start_offset))
            .map_err(|_| BridgeError::InvalidTime)?;
        self.with_instances(self.instance_home(&effect.effect)?, move |effects| {
            let instance = effects
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or(BridgeError::InvalidEffect)?;
            match &file {
                Some(file) => lumit_core::fx::presets::apply_over(instance, &chosen, at.0, file),
                None => lumit_core::fx::presets::apply(instance, &chosen, at.0, raster),
            }
            Ok(())
        })
    }
}
