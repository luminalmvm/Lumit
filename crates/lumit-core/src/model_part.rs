//! A Model layer's Animation rows and its parts' rows, declared the way an
//! effect is, so keyframes, expressions, undo and the panels all come with them.
//!
//! The rows live on the layer and never in the file. A part nobody has changed
//! isn't saved, and its rows show nothing done to it.

use crate::fx::registry::{Catalogue, EffectDef, EffectMetadata};
use crate::fx::schema::EffectSchema;
use crate::model::{EffectInstance, EffectValue, Layer, LayerKind};
use crate::model_layer::Take;
use lumit_fx_macros::Effect;
use uuid::Uuid;

/// What [`ModelPlayback`] and [`ModelPart`] are known by.
pub const PLAYBACK_NAME: &str = "model_playback";
pub const PART_NAME: &str = "model_part";

/// How a Model layer plays its file's takes.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "model_playback",
    label = "Animation",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct ModelPlayback {
    /// Nothing, then the file's takes, listed from the file.
    #[choice(label = "Take", options = ["None"], default = 0)]
    pub take: u32,
    #[choice(label = "Loop", options = ["Off", "Loop", "Mirror", "Freeze"], default = 1)]
    pub looping: u32,
    /// Added to the layer's clock, Retime and all, to say where in the take.
    #[slider(label = "Offset", min = -10.0, max = 10.0, default = 0.0, unit = Seconds)]
    pub offset: f32,
}

/// What the user has done to one node of the file.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "model_part",
    label = "Part",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    matte = false,
)]
pub struct ModelPart {
    /// Shown from a half up, so it can be keyed.
    #[counter(label = "Visible", min = 0, max = 1, default = 1, hard_min = 0, hard_max = 1, unit = Raw)]
    pub visible: i32,
    /// How far it has moved in the layer's pixels, along the layer's axes.
    #[slider(label = "Position x", min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub position_x: f32,
    #[slider(label = "Position y", min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub position_y: f32,
    #[slider(label = "Position z", min = -1000.0, max = 1000.0, default = 0.0, unit = Px)]
    pub position_z: f32,
    /// Turned about its own origin, as the layer's rows turn the layer.
    #[dial(label = "Rotation x", default = 0.0, step = 15.0)]
    pub rotation_x: f32,
    #[dial(label = "Rotation y", default = 0.0, step = 15.0)]
    pub rotation_y: f32,
    #[dial(label = "Rotation z", default = 0.0, step = 15.0)]
    pub rotation_z: f32,
    #[slider(label = "Scale", min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, unit = Percent)]
    pub scale: f32,
}

/// The rows' behaviour: none of their own, the pose reads them.
pub struct ModelPlaybackDef;
pub struct ModelPartDef;

impl EffectDef for ModelPlaybackDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ModelPlayback as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }
}

impl EffectDef for ModelPartDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ModelPart as EffectMetadata>::SCHEMA
    }

    fn is_image_op(&self) -> bool {
        false
    }
}

/// Kept apart from the effect catalogue, so no menu offers them as effects.
/// [`crate::fx::builtins::def`] finds them here.
pub static PART_DEFS: Catalogue = Catalogue::new(&[&ModelPlaybackDef, &ModelPartDef]);

/// The low half of every part's id is its node's number, the rest a fixed mark
/// over its layer's id, so the id says which part of which layer it is.
const MARK: u128 = 0x6d6f_6465_6c20_7061_7274_7320_0000_0000;

/// The id of the rows of node `node` on Model layer `layer`. Playback's rows
/// take the number after every node's, as there is only ever one of them.
#[must_use]
pub fn part_id(layer: Uuid, node: usize) -> Uuid {
    Uuid::from_u128(layer.as_u128() ^ (MARK | (node as u128 + 1)))
}

/// The id of Model layer `layer`'s playback rows.
#[must_use]
pub fn playback_id(layer: Uuid) -> Uuid {
    Uuid::from_u128(layer.as_u128() ^ MARK)
}

/// Which node of Model layer `layer` the rows `id` are, or `None` for rows of
/// anything else. Playback's rows answer nought as they aren't a node's.
#[must_use]
pub fn node_of(layer: Uuid, id: Uuid) -> Option<usize> {
    let x = layer.as_u128() ^ id.as_u128();
    let n = usize::try_from(x & 0xffff_ffff).ok()?;
    (x & !0xffff_ffff == MARK).then(|| n.checked_sub(1))?
}

/// Whether `id` names rows of Model layer `layer`.
#[must_use]
pub fn owns(layer: Uuid, id: Uuid) -> bool {
    (layer.as_u128() ^ id.as_u128()) & !0xffff_ffff == MARK
}

/// The rows Model layer `layer` keeps: its playback rows, then its parts'.
/// Empty on any other layer.
#[must_use]
pub fn stored(layer: &Layer) -> Vec<EffectInstance> {
    match &layer.kind {
        LayerKind::Model { settings, .. } => {
            let playback = settings.playback.iter().cloned();
            playback.chain(settings.parts.iter().cloned()).collect()
        }
        _ => Vec::new(),
    }
}

/// What `take` is when the playback rows' Animation row reads `choice`, with
/// the takes `names` lists.
#[must_use]
pub fn take_of(choice: u32, names: &[String]) -> Take {
    match choice.checked_sub(1).and_then(|i| names.get(i as usize)) {
        Some(name) => Take::Named(name.clone()),
        None => Take::Rest,
    }
}

/// What the playback rows' Animation row reads for `take`: nought for none, and
/// for a take the file has lost the place after its own, where the row says so.
#[must_use]
pub fn choice_of(take: &Take, names: &[String]) -> u32 {
    match take {
        Take::Rest => 0,
        Take::First => u32::from(!names.is_empty()),
        Take::Named(name) => names
            .iter()
            .position(|n| n == name)
            .map_or(names.len() as u32 + 1, |i| i as u32 + 1),
    }
}

/// Whether `rows` do nothing, every one of them still as declared: such rows
/// aren't kept. The Animation row is kept as the layer's take, not here.
#[must_use]
pub fn untouched(rows: &EffectInstance) -> bool {
    let Some(schema) = crate::fx::builtins::schema(&rows.effect.match_name) else {
        return false;
    };
    rows.params.iter().filter(|p| p.id != "take").all(|p| {
        let Some(row) = schema.params.iter().find(|s| s.id == p.id) else {
            return false;
        };
        crate::fx::builtins::default_param_value(&row.kind).as_ref() == Some(&p.value)
    })
}

/// What Model layer rows read at `lt`: a number row, falling back to `default`
/// where it isn't one.
#[must_use]
pub fn number(
    rows: &EffectInstance,
    id: &str,
    lt: f64,
    context: &std::sync::Arc<crate::expression::ExpressionContext>,
    default: f64,
) -> f64 {
    match rows.param(id) {
        Some(EffectValue::Float(p)) => p.value_at_with_context(lt, std::sync::Arc::clone(context)),
        _ => default,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::fx::builtins::instantiate;

    /// Ids find their way back, a take is kept by name however the file's list
    /// moves, one the file has lost reads past the list, and only rows somebody
    /// changed count as changed.
    #[test]
    fn rows_and_takes_read_back() {
        let layer = Uuid::now_v7();
        assert_eq!(node_of(layer, part_id(layer, 7)), Some(7));
        assert_eq!(node_of(layer, playback_id(layer)), None);
        assert!(owns(layer, playback_id(layer)) && owns(layer, part_id(layer, 7)));
        assert!(!owns(layer, Uuid::now_v7()));

        let names = ["Walk".to_owned(), "Run".to_owned()];
        assert_eq!(take_of(2, &names), Take::Named("Run".into()));
        assert_eq!(take_of(0, &names), Take::Rest);
        assert_eq!(choice_of(&Take::Named("Run".into()), &names), 2);
        assert_eq!(choice_of(&Take::First, &names), 1);
        assert_eq!(choice_of(&Take::First, &[]), 0);
        assert_eq!(choice_of(&Take::Named("Gone".into()), &names), 3);

        let mut rows = instantiate(PART_NAME).unwrap();
        assert!(untouched(&rows));
        for p in &mut rows.params {
            if p.id == "scale" {
                p.value = EffectValue::Float(crate::anim::Property::fixed(50.0));
            }
        }
        assert!(!untouched(&rows));
    }
}
