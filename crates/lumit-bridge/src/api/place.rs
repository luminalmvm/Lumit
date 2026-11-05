//! Several footage items placed in a composition at once: what a drop of a
//! Project panel selection on the Timeline makes.

use flutter_rust_bridge::frb;
use lumit_core::anim::Property;
use lumit_core::model::{Layer, LayerKind, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::time::{CompTime, Duration, Rational};

use crate::api::{
    composition::{insert_row, CompositionReference},
    footage::FootageReference,
    layer::LayerReference,
    streams::BridgeStreamsArrival,
    BridgeError,
};

/// What marks the Key light a first 3D model brought.
pub(crate) const AUTO_KEY: &str = "auto_key_light";

/// How long a still lasts in a run of dropped files, in seconds: what Cut's
/// drop gives one.
const STILL_SECONDS: i64 = 5;

impl CompositionReference {
    /// Place `footage` in this composition as new layers, the first given on
    /// top, as one undo step.
    ///
    /// `end_to_end` lays them one after another, each starting where the one
    /// before it ends, as Premiere and Vegas lay a multi-item drop. The
    /// composition grows to the last one's end if it runs past it, as a cut
    /// does. Without it they all start together, as After Effects stacks them.
    ///
    /// Each layer is what [`Self::add_footage_layer`] makes of its item, with
    /// `as_sequence` and `arrival` as there. The first 3D model in a comp with
    /// no lights brings an Ambient light and a Key light with it, in the same
    /// undo step, named by `light_names` (ambient, then key) when given.
    /// Answers the layers made from the items.
    #[frb(sync)]
    pub fn add_footage_layers(
        &self,
        footage: Vec<FootageReference>,
        as_sequence: bool,
        row: Option<u32>,
        arrival: Option<BridgeStreamsArrival>,
        end_to_end: bool,
        light_names: Option<Vec<String>>,
    ) -> Result<Vec<LayerReference>, BridgeError> {
        let arrival = arrival.unwrap_or(BridgeStreamsArrival::First);
        let proj = self.project()?;
        let comp = self.composition()?;
        let mut index = insert_row(row, comp.layers.len());
        let mut start = Rational::ZERO;
        let five = Rational::new(STILL_SECONDS, 1).map_err(|_| BridgeError::InvalidTime)?;
        let still = comp
            .frame_rate
            .time_of_frame(comp.frame_rate.nearest_frame_at(CompTime(five)))
            .map_err(|_| BridgeError::InvalidTime)?
            .0;
        let mut ops = Vec::new();
        let mut placed = Vec::new();
        {
            let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = p.store.snapshot();
            for item in &footage {
                let (layer, beside) =
                    Self::footage_layers(&p, &doc, &comp, item.id(), as_sequence, false, arrival)?;
                // A still, or a file that will not probe, has no length of its
                // own and would last the whole comp, pushing the next one past
                // its end. In a run it takes what a still dropped in Cut takes.
                let held = end_to_end
                    && footage.len() > 1
                    && match doc.item(item.id()) {
                        Some(ProjectItem::Footage(f)) => {
                            Self::has_picture(&p, f) && !Self::runs_as_video(&p, f)
                        }
                        _ => false,
                    };
                let length = if held {
                    still
                } else {
                    layer
                        .out_point
                        .0
                        .checked_sub(layer.in_point.0)
                        .map_err(|_| BridgeError::InvalidTime)?
                };
                for mut layer in std::iter::once(layer).chain(beside) {
                    if held {
                        layer.out_point = layer
                            .in_point
                            .add_dur(Duration(length))
                            .map_err(|_| BridgeError::InvalidTime)?;
                    }
                    if end_to_end {
                        shift(&mut layer, Duration(start))?;
                    }
                    placed.push(LayerReference::new(self.project, self.id, layer.id));
                    ops.push(Op::AddLayer {
                        comp: self.id,
                        index,
                        layer: Box::new(layer),
                    });
                    index += 1;
                }
                start = start
                    .checked_add(length)
                    .map_err(|_| BridgeError::InvalidTime)?;
            }
        }
        if ops.is_empty() {
            return Ok(placed);
        }
        let model = ops.iter().any(|op| {
            matches!(op, Op::AddLayer { layer, .. } if matches!(layer.kind, LayerKind::Model { .. }))
        });
        if model {
            for light in Self::first_lights(&comp, light_names.as_deref()) {
                ops.push(Op::AddLayer {
                    comp: self.id,
                    index: 0,
                    layer: Box::new(light),
                });
            }
        }
        if end_to_end && start > comp.duration.0 {
            ops.push(Op::SetCompSettings {
                comp: self.id,
                name: comp.name.clone(),
                width: comp.width,
                height: comp.height,
                frame_rate: comp.frame_rate,
                duration: Duration(start),
                background: comp.background,
            });
        }
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(Op::Batch { ops })
            .map_err(BridgeError::OpError)?;
        Ok(placed)
    }

    /// Add 3D shape `entry` of the menu (cube, sphere, cylinder, cone, torus,
    /// plane, then pyramid) at comp point `at`, or the comp's centre. In a
    /// comp with no lights it brings the lights a 3D model does, named by
    /// `light_names`, in the same undo step.
    #[frb(sync)]
    pub fn add_mesh_layer(
        &self,
        entry: u32,
        row: Option<u32>,
        at: Option<Vec<f64>>,
        light_names: Option<Vec<String>>,
    ) -> Result<LayerReference, BridgeError> {
        let comp = self.composition()?;
        let mut layer = lumit_core::mesh_shape::mesh_layer(&comp, entry as usize)
            .ok_or(BridgeError::InvalidParam)?;
        if let Some([x, y]) = at.as_deref() {
            layer.transform.position_x = Property::fixed(*x);
            layer.transform.position_y = Property::fixed(*y);
        }
        crate::edits::solo_on_arrival(&mut layer, comp.layers.iter());
        let id = layer.id;
        let mut ops = vec![Op::AddLayer {
            comp: self.id,
            index: insert_row(row, comp.layers.len()),
            layer: Box::new(layer),
        }];
        for light in Self::first_lights(&comp, light_names.as_deref()) {
            ops.push(Op::AddLayer {
                comp: self.id,
                index: 0,
                layer: Box::new(light),
            });
        }
        self.commit(Op::Batch { ops })?;
        Ok(LayerReference::new(self.project, self.id, id))
    }

    /// The Key and Ambient lights a comp with no lights gets from its first
    /// model or solid layer, so that isn't drawn as a flat silhouette. Nothing
    /// for a comp that has a light. Named by `names` (ambient, then key) when
    /// given.
    #[frb(ignore)]
    pub(crate) fn first_lights(
        comp: &lumit_core::model::Composition,
        names: Option<&[String]>,
    ) -> Vec<Layer> {
        if comp
            .layers
            .iter()
            .any(|l| matches!(l.kind, LayerKind::Light { .. }))
        {
            return Vec::new();
        }
        // Ambient (4) and Parallel (3), as Layer > New > Light makes them.
        let mut ambient = Self::light_layer(comp, 4);
        let mut key = Self::key_light(comp);
        if let Some([a, k]) = names {
            ambient.name.clone_from(a);
            key.name.clone_from(k);
        }
        if let LayerKind::Light { light } = &mut ambient.kind {
            light.intensity = Property::fixed(25.0);
        }
        let mut lights = [key, ambient];
        for light in &mut lights {
            crate::edits::solo_on_arrival(light, comp.layers.iter());
        }
        lights.into()
    }

    /// The Key light the first 3D model in a comp brings, marked so that lights
    /// made from a model's file can take its place if nobody has touched it.
    #[frb(ignore)]
    pub(crate) fn key_light(comp: &lumit_core::model::Composition) -> Layer {
        let mut key = Self::light_layer(comp, 3);
        key.name = "Key light".into();
        // Up and to the left, about 45 degrees each way from the lens, so the
        // model shows its shape.
        let w = f64::from(comp.width);
        key.transform.position_x = Property::fixed(w * 0.15);
        key.transform.position_y = Property::fixed(f64::from(comp.height) * 0.5 - w * 0.35);
        key.transform.position_z = Property::fixed(-w * 0.5);
        if let LayerKind::Light { light } = &mut key.kind {
            light.casts_shadows = true;
            light.shadow_diffusion = Property::fixed(f64::from(comp.width) * 0.01);
        }
        key.extra.insert(AUTO_KEY.into(), true.into());
        key
    }

    /// The Light layer [`Self::add_light_layer`] makes in `comp`.
    #[frb(ignore)]
    pub(crate) fn light_layer(
        comp: &lumit_core::model::Composition,
        kind: u32,
    ) -> lumit_core::model::Layer {
        use lumit_core::anim::Property;
        use lumit_core::model::{LightDef, LightKind, TransformGroup};

        let kind = match kind {
            1 => LightKind::Spot,
            2 => LightKind::Area,
            3 => LightKind::Parallel,
            4 => LightKind::Ambient,
            5 => LightKind::Environment,
            _ => LightKind::Point,
        };
        let (w, h) = (f64::from(comp.width), f64::from(comp.height));
        let light = Box::new(LightDef {
            kind,
            size: [Property::fixed(w * 0.2), Property::fixed(h * 0.2)],
            two_node: true,
            point_of_interest: [
                Property::fixed(w * 0.5),
                Property::fixed(h * 0.5),
                Property::zero(),
            ],
            ..LightDef::default()
        });
        let name = match kind {
            LightKind::Point => "Point light",
            LightKind::Spot => "Spot light",
            LightKind::Area => "Area light",
            LightKind::Parallel => "Parallel light",
            LightKind::Ambient => "Ambient light",
            LightKind::Environment => "Environment light",
        };
        crate::edits::base_layer(
            name.into(),
            lumit_core::model::LayerKind::Light { light },
            comp.duration.0,
            TransformGroup {
                position_x: Property::fixed(w * 0.4),
                position_y: Property::fixed(h * 0.35),
                position_z: Property::fixed(-w * 0.35),
                ..TransformGroup::default()
            },
        )
    }
}

/// Lay staged `effects` over layer `index` in `comp`, a copy made for a
/// preview. A depth being dragged up from nought shows lit as it will land.
#[frb(ignore)]
pub(crate) fn stage_rows(
    comp: &mut lumit_core::model::Composition,
    index: usize,
    effects: Vec<lumit_core::model::EffectInstance>,
) {
    let Some(layer) = comp.layers.get_mut(index) else {
        return;
    };
    let was = lumit_core::extrude::is_extruded(layer);
    lumit_render::model_material::stage(layer, effects);
    if !was && lumit_core::extrude::is_extruded(layer) {
        // At the end, so the layer's own place in the stack doesn't move.
        let lights = CompositionReference::first_lights(comp, None);
        comp.layers.extend(lights);
    }
}

/// Move a layer and what it holds later by `by`.
fn shift(layer: &mut Layer, by: Duration) -> Result<(), BridgeError> {
    let at = |t: CompTime| t.add_dur(by).map_err(|_| BridgeError::InvalidTime);
    layer.in_point = at(layer.in_point)?;
    layer.out_point = at(layer.out_point)?;
    layer.start_offset = at(layer.start_offset)?;
    Ok(())
}
