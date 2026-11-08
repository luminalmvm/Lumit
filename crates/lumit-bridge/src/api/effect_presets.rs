//! The built-in presets an effect's menu offers ([`lumit_core::fx::presets`]).

use flutter_rust_bridge::frb;

use lumit_core::preset_parts::PresetParts;

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

    /// A `.lumfx` document of the chosen parts of this layer: the effects named
    /// by `effects` with the drivers that feed them, its transform keys, and its
    /// text animators. An empty `effects` takes no effects. Named `name`, and
    /// handed back as text, as [`Self::save_preset`] hands it back.
    #[frb(sync)]
    pub fn save_preset_parts(
        &self,
        name: String,
        effects: Vec<uuid::Uuid>,
        transform_keys: bool,
        text_animators: bool,
    ) -> Result<String, BridgeError> {
        let layer = self.leaving()?;
        let taken: Vec<_> = layer
            .effects
            .iter()
            .filter(|e| effects.contains(&e.id))
            .cloned()
            .collect();
        let wiring = lumit_core::preset::wiring_of(&layer.graph, &layer.effects, &effects);
        let text = lumit_core::preset::to_json_wired(&name, &taken, wiring)
            .map_err(|_| BridgeError::InvalidPreset)?;
        let parts = PresetParts {
            transform: if transform_keys {
                lumit_core::preset_parts::keyed_rows(&layer.transform)
            } else {
                Vec::new()
            },
            animators: match &layer.kind {
                lumit_core::model::LayerKind::Text { document } if text_animators => {
                    document.animators.clone()
                }
                _ => Vec::new(),
            },
        };
        parts
            .written_into(&text)
            .map_err(|_| BridgeError::InvalidPreset)
    }

    /// Lay a copied stack or a preset onto this layer, **timed to the playhead**:
    /// whatever its earliest keyframe was, it lands at `at_frame` and the rest
    /// keep their spacing, across the effects, the transform keys and the text
    /// animators alike.
    ///
    /// An effect copied from a layer that flashes at 4 s and pasted while the
    /// playhead sits at 12 s flashes at 12 s, not off the end of the comp.
    /// Nothing keyed has no timing to place. Each effect arrives with a fresh
    /// instance id. A list the layer can't take is left out: text animators off
    /// a text layer, transform keys on a layer with no picture.
    ///
    /// `at_frame` is a **comp** frame; the shift is worked out in the target
    /// layer's own local time, so a layer that starts later does not
    /// double-count its offset. One commit, so one undo.
    #[frb(sync)]
    pub fn paste_effects(&self, text: String, at_frame: i64) -> Result<(), BridgeError> {
        use lumit_core::model::{EffectInstance, LayerKind};
        use lumit_core::ops::Op;

        let preset =
            lumit_core::preset::from_json(&text).map_err(|_| BridgeError::InvalidPreset)?;
        let mut parts = PresetParts::from_json(&text);

        // Comp frame -> the layer's own clock, the space keyframes live in.
        let comp = self.composition()?;
        let layer = self.item()?;
        if layer.audio_only {
            parts.transform.clear();
        }
        if !matches!(layer.kind, LayerKind::Text { .. }) {
            parts.animators.clear();
        }
        let at_comp = comp
            .frame_rate
            .time_of_frame(at_frame.max(0))
            .map_err(|_| BridgeError::InvalidTime)?;
        let at_local = at_comp
            .delta(layer.start_offset)
            .map_err(|_| BridgeError::InvalidTime)?;

        // Copied with their wiring, the effects come back with their drivers and
        // wires, every id fresh, set down to the right of what the graph holds.
        let mut wired = preset.wiring.as_ref().map(|wiring| {
            let right = layer
                .graph
                .layout
                .iter()
                .map(|(_, xy)| xy[0])
                .fold(f64::MIN, f64::max);
            let at = if right == f64::MIN {
                [0.0, 0.0]
            } else {
                [right + 240.0, 0.0]
            };
            lumit_core::preset::group_instantiated(wiring, at)
        });
        let mut fresh: Vec<EffectInstance> = if wired.is_some() {
            Vec::new()
        } else {
            lumit_core::preset::instantiated(&preset)
        };

        // One amount moves everything, so the lists keep their timing together.
        let first = [
            lumit_core::preset::first_key_time(&fresh),
            wired
                .as_ref()
                .and_then(|w| lumit_core::preset::first_key_time(&w.effects)),
            wired
                .as_ref()
                .and_then(|w| lumit_core::preset::first_key_time(&w.nodes)),
            parts.first_key_time(),
        ]
        .into_iter()
        .flatten()
        .min();
        if let Some(first) = first {
            let delta = at_local
                .0
                .checked_sub(first)
                .map_err(|_| BridgeError::InvalidTime)?;
            lumit_core::preset::shift_keys(&mut fresh, delta);
            if let Some(wired) = &mut wired {
                lumit_core::preset::shift_keys(&mut wired.effects, delta);
                lumit_core::preset::shift_keys(&mut wired.nodes, delta);
            }
            parts.shift_keys(delta);
        }

        let mut ops = Vec::new();
        if let Some(mut added) = wired {
            let mut graph = layer.graph.clone();
            graph.nodes.extend(added.nodes);
            graph.edges.extend(added.edges);
            graph.layout.extend(added.layout);
            self.migrate_in(
                &mut added.effects,
                &lumit_core::fx::builtins::wired_in(&graph),
            )?;
            let mut effects = layer.effects.clone();
            effects.extend(added.effects);
            graph.prune_rows(&effects);
            ops.push(lumit_core::fx::effects::particulate::set_effects(
                self.comp_id,
                &layer,
                effects,
            ));
            ops.push(Op::SetLayerGraph {
                comp: self.comp_id,
                layer: self.layer_id,
                graph: Box::new(graph),
            });
        } else if !fresh.is_empty() {
            self.migrate_in(&mut fresh, &|_, _| false)?;
            let mut effects = layer.effects.clone();
            effects.extend(fresh);
            let doc = self.document()?;
            ops.push(lumit_core::fx::effects::depth_points::set_effects(
                &doc, &comp, &layer, effects,
            ));
        }
        for row in parts.transform {
            ops.push(Op::SetTransformProperty {
                comp: self.comp_id,
                layer: self.layer_id,
                prop: row.prop,
                animation: row.animation,
            });
        }
        if !parts.animators.is_empty() {
            if let LayerKind::Text { document } = &layer.kind {
                let mut document = document.clone();
                document.animators.extend(parts.animators);
                ops.push(Op::SetTextDocument {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    document,
                });
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;
        Ok(())
    }
}
