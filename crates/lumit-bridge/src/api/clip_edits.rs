//! Holding a frame and an overlap's kind: the calls behind Freeze frame and
//! Hold here, New composition from selection and the Transition menu.

use flutter_rust_bridge::frb;
use lumit_core::anim::{Animation, Keyframe, SideInterp};
use lumit_core::model::LayerKind;
use lumit_core::sequence::transition::{swept_rows, KINDS};
use lumit_core::time::{CompTime, Rational};
use uuid::Uuid;

use crate::api::{
    composition::CompositionReference,
    cut::{self, BridgeCutResult},
    effect::BridgeEffectInstance,
    layer::LayerReference,
    BridgeError,
};

/// One kind of overlap the Transition menu offers, besides the dissolve.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTransitionKind {
    pub match_name: String,
    /// The effect's own label, as Effect controls heads it.
    pub label: String,
    /// The rows the overlap decides for it, which Effect controls leaves out.
    pub swept_rows: Vec<String>,
}

/// Every kind an overlap can be, in menu order.
#[frb(sync)]
pub fn cut_transition_kinds() -> Vec<BridgeTransitionKind> {
    KINDS
        .iter()
        .filter_map(|name| {
            let def = lumit_core::fx::catalogue::BUILTIN_DEFS.get(*name)?;
            Some(BridgeTransitionKind {
                match_name: (*name).to_string(),
                label: def.schema().label.to_string(),
                swept_rows: swept_rows(name).iter().map(|r| (*r).to_string()).collect(),
            })
        })
        .collect()
}

impl CompositionReference {
    /// **Freeze frame** on a clip: hold the frame it shows at `at_frame` for
    /// `hold_frames`, as a clip of its own between its two halves, with what
    /// follows moved later on every unlocked Sequence layer. With `linked` the
    /// clips linked to it are cut and moved too, and silence sits under the
    /// hold. Without it they stay, and play on through.
    #[frb(sync)]
    pub fn cut_freeze(
        &self,
        clip: Uuid,
        at_frame: i64,
        hold_frames: i64,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut::freeze::freeze_clip(self, clip, at_frame, hold_frames, linked)
    }

    /// **New composition from selection**: move `clips` into a new composition
    /// called `name`, on the same rows and in the same places, and put one clip
    /// playing it across the span they took. With `linked` their linked sound
    /// comes along. One undo step. `None` when a layer they are on is locked.
    #[frb(sync)]
    pub fn cut_clips_to_composition(
        &self,
        clips: Vec<Uuid>,
        linked: bool,
        name: String,
    ) -> Result<Option<CompositionReference>, BridgeError> {
        let made = cut::fold::fold_clips(self, &clips, linked, name)?;
        Ok(made.map(|id| CompositionReference::new(self.project_id(), id)))
    }

    /// Make the overlap at this edge of `clip` the kind named by its match
    /// name, or the dissolve with `None`. An edit point that only meets gets an
    /// overlap of `frames` first. Asked for the kind it already is, an overlap
    /// keeps the rows it was given.
    #[frb(sync)]
    pub fn cut_set_transition(
        &self,
        clip: Uuid,
        end_edge: bool,
        frames: i64,
        kind: Option<String>,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut::transition::set_kind(self, clip, end_edge, frames, kind.as_deref(), linked)
    }
}

impl LayerReference {
    /// **Insert a freeze at the playhead** (docs/04 §7.3,
    /// `retime.freeze_at_playhead`): the moment showing at `frame` is held for
    /// `hold_frames`, everything after it is pushed that far later, and the map
    /// is cropped back to the layer's own out point, so the layer's length never
    /// changes and the beat-sync covenant holds. The tail may newly overrun,
    /// which is drawn rather than repaired.
    ///
    /// The hold is two ordinary keyframes afterwards, so it is dragged like
    /// anything else.
    ///
    /// A layer with no map yet gets the identity one first: freezing a frame
    /// is a reasonable first retime to ask for, and refusing it because the
    /// stopwatch had not been touched would be a rule with no purpose.
    ///
    /// Refused on a Sequence layer (its clips carry the maps) and at a moment
    /// on or outside the layer's own ends, where there is nothing to split.
    #[frb(sync)]
    pub fn freeze_at_playhead(&self, frame: i64, hold_frames: i64) -> Result<(), BridgeError> {
        let layer = self.item()?;
        if matches!(layer.kind, LayerKind::Sequence { .. }) {
            return Err(BridgeError::NotRetimeable);
        }
        let bad = |_| BridgeError::InvalidTime;
        if hold_frames < 1 {
            return Err(BridgeError::InvalidTime);
        }
        let rate = self.composition()?.frame_rate;
        let at = rate.time_of_frame(frame).map_err(bad)?;
        let local = |t: CompTime| t.0.checked_sub(layer.start_offset.0).map_err(bad);
        let (in_local, out_local, at_local) =
            (local(layer.in_point)?, local(layer.out_point)?, local(at)?);
        if at_local <= in_local || at_local >= out_local {
            return Err(BridgeError::InvalidTime);
        }
        let hold: Rational = rate.time_of_frame(hold_frames).map_err(bad)?.0;
        let resume = at_local.checked_add(hold).map_err(bad)?;

        let mut map = self.retime_or_identity(&layer, in_local, out_local);
        // Split the curve where the freeze begins without changing it (the
        // razor's own move), so the moment held is exactly the moment that was
        // showing.
        map.insert_key_preserving_shape(at_local);
        let Animation::Keyframed(keys) = &mut map.animation else {
            return Err(BridgeError::NotRetimed);
        };
        let split = at_local.to_f64();
        let Some(i) = keys
            .iter()
            .position(|k| (k.time.to_f64() - split).abs() < 1e-12)
        else {
            return Err(BridgeError::InvalidTime);
        };
        // What the curve did after this moment, handed to the key the freeze
        // ends on: the movement resumes where it left off rather than
        // restarting as a straight line.
        let resumes = keys[i].interp_out;
        let held = keys[i].value;
        keys[i].interp_out = SideInterp::Hold;
        for key in keys.iter_mut().skip(i + 1) {
            key.time = key.time.checked_add(hold).map_err(bad)?;
        }
        keys.insert(
            i + 1,
            Keyframe {
                time: resume,
                value: held,
                interp_in: SideInterp::Hold,
                interp_out: resumes,
            },
        );
        // Crop back to the layer's own out point: what the freeze pushed past
        // the end is gone, and the span straddling the end is split there so
        // the part that survives is the part that was already drawn.
        if keys.last().is_some_and(|k| k.time > out_local) {
            map.insert_key_preserving_shape(out_local);
            if let Animation::Keyframed(keys) = &mut map.animation {
                keys.retain(|k| k.time <= out_local);
            }
        }

        self.commit(lumit_core::ops::Op::SetRetimeProperty {
            comp: self.comp_id(),
            layer: self.id(),
            retime: Some(map),
        })
    }

    /// The instance behind the overlap at `clip`'s start, as a staged copy on
    /// the comp's clock like [`Self::get_clip_effects`] with `on_comp_clock`.
    /// `None` for a dissolve.
    #[frb(sync)]
    pub fn get_clip_transition(
        &self,
        clip: Uuid,
    ) -> Result<Option<BridgeEffectInstance>, BridgeError> {
        let (clips, index) = self.clips_and_index(clip)?;
        let offset = self
            .item()?
            .start_offset
            .0
            .checked_add(clips[index].place_start)
            .map_err(|_| BridgeError::InvalidTime)?;
        Ok(clips[index]
            .transition
            .as_ref()
            .map(|e| BridgeEffectInstance::new(e.clone(), offset)))
    }
}
