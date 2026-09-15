//! The planes tier's **document** half (docs/impl/addons.md §6.1): which
//! effects ask a model for a plane, what they ask for, and the per-frame stamp
//! that decides which cached pictures an edit throws away.
//!
//! # In plain terms
//!
//! Some effects here hand a frame to a trained model and get a plane back: how
//! far away every pixel is, or how much of each pixel is the subject. The plane
//! is not in the project file. It is worked out by a background analysis, kept
//! in a cache folder, and thrown away without loss, exactly as a roto matte is.
//!
//! What this file holds is the document's side of that: the rows the analysis
//! reads, and one hash saying what a frame's plane is a function of, so a frame
//! drawn through a plane is *named* by that plane. Change the model row and
//! every frame is renamed; change the view row and none of them is, because the
//! view decides how the plane is shown and not what it holds.
//!
//! The model's own identity - which pack, which version, which provider - is
//! deliberately **not** here. It is not document state: it is a fact about the
//! machine, and two machines with two different packs are two machines with the
//! same project. It reaches the frame key through
//! `lumit_eval::SourceStamper::planes_identity`, the way the synthesis pack's
//! does (§7).

use crate::model::{EffectInstance, EffectValue};

/// The `planes/` sidecar tier's version, fed into every hash here so a build
/// that changes the meaning of a plane cannot read the old one back. Bumping it
/// orphans every cached plane, which costs one Analyse.
pub const TIER_VERSION: u16 = 1;

/// The Depth effect's `match_name`, so the render path and the frame key can
/// find it without importing the catalogue's type.
pub const DEPTH: &str = "depth";

/// The Remove background effect's, for the same reason.
pub const REMOVE_BACKGROUND: &str = "remove_background";

/// What a planes-tier effect asks a model for.
///
/// The document's own copy of `lumit_ml::Task`, because `lumit-core` is the
/// bottom of the crate graph and may not depend on the crate that opens a model
/// (docs/05 §1.1) - the same split the Camera track's density table takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneTask {
    /// How far away every pixel is.
    Depth,
    /// How much of each pixel is the subject.
    Matte,
}

impl PlaneTask {
    /// A stable byte for a hash. Never reuse a value.
    #[must_use]
    pub fn tag(self) -> u8 {
        match self {
            PlaneTask::Depth => 1,
            PlaneTask::Matte => 2,
        }
    }
}

/// What an effect of this `match_name` asks for, or `None` when it is not a
/// planes-tier effect at all.
///
/// The one predicate: the render carriage, the frame key, the badge and the
/// bridge all ask it rather than each keeping a list of names.
#[must_use]
pub fn task_of_name(match_name: &str) -> Option<PlaneTask> {
    match match_name {
        DEPTH => Some(PlaneTask::Depth),
        REMOVE_BACKGROUND => Some(PlaneTask::Matte),
        _ => None,
    }
}

/// What one instance asks for, or `None` for anything that is not a
/// planes-tier effect.
#[must_use]
pub fn task_of(fx: &EffectInstance) -> Option<PlaneTask> {
    (fx.effect.namespace == crate::model::EffectNamespace::Builtin)
        .then(|| task_of_name(&fx.effect.match_name))
        .flatten()
}

/// The planes-tier effects on a layer that are switched on, in stack order.
pub fn analyses(effects: &[EffectInstance]) -> impl Iterator<Item = &EffectInstance> {
    effects.iter().filter(|e| e.enabled && task_of(e).is_some())
}

/// The settings that change what a plane **is**, read off an instance.
///
/// Deliberately not every row: the view and the invert change how the plane is
/// *shown*, not what the model produced, and hashing them would throw a whole
/// analysis away every time somebody glanced at the depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaneSettings {
    /// Which model the effect names, as its Choice index.
    pub model: u32,
    /// Robust Video Matting's detail row, as its Choice index. Zero for every
    /// effect that has no such row, which is every one but Remove background.
    pub detail: u32,
}

impl PlaneSettings {
    /// Read the settings off one instance. A row the instance does not carry
    /// (an older project, a hand-edited file) reads as the default rather than
    /// failing (docs/14 §4).
    ///
    /// **At layer time nought**, not at the playhead, for the Roto brush's
    /// reason: these rows name a cache, and a cache key that moved with the
    /// playhead would file every frame under a different name.
    #[must_use]
    pub fn of(fx: &EffectInstance) -> Self {
        let choice = |id: &str| match fx.param(id) {
            Some(EffectValue::Choice(v)) => *v,
            _ => 0,
        };
        PlaneSettings {
            model: choice("model"),
            detail: choice("detail"),
        }
    }

    /// Feed the settings into a hash, in a fixed order.
    pub fn feed(&self, h: &mut blake3::Hasher) {
        h.update(&self.model.to_le_bytes());
        h.update(&self.detail.to_le_bytes());
    }
}

/// The stamp naming frame `frame` of the plane `fx` holds, or `None` when `fx`
/// is not a planes-tier effect.
///
/// The one call the frame key makes. Constant across the clip, since these
/// effects have no strokes and nothing about one frame decides another, and
/// still asked per frame so the shape stays the one [`crate::roto::frame_stamp`]
/// set: the day a prompt lands on one frame, this is where it goes.
///
/// The pack's own identity is not in here and cannot be: see the module note.
#[must_use]
pub fn frame_stamp(fx: &EffectInstance, frame: i64) -> Option<[u8; 32]> {
    let task = task_of(fx)?;
    let mut h = blake3::Hasher::new();
    h.update(b"lumit-planes/stamp/");
    h.update(&TIER_VERSION.to_le_bytes());
    h.update(&[task.tag()]);
    PlaneSettings::of(fx).feed(&mut h);
    h.update(&frame.to_le_bytes());
    Some(*h.finalize().as_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::fx::instantiate;
    use crate::model::EffectParam;

    /// Write one row on an instance, adding it when the declaration's default
    /// left it out.
    fn set(fx: &mut EffectInstance, id: &str, value: EffectValue) {
        match fx.params.iter_mut().find(|p| p.id == id) {
            Some(param) => param.value = value,
            None => fx.params.push(EffectParam {
                id: id.to_owned(),
                value,
                extra: serde_json::Map::new(),
            }),
        }
    }

    /// **A Depth instance names its task, and nothing else does.** Every
    /// carriage in the render, the badge and the frame key ask this one
    /// question, so an effect answering it by accident would be handed
    /// somebody else's plane.
    #[test]
    fn the_one_predicate_answers_for_the_planes_effects_alone() {
        let depth = instantiate(DEPTH).expect("declared");
        assert_eq!(task_of(&depth), Some(PlaneTask::Depth));
        let matte = instantiate(REMOVE_BACKGROUND).expect("declared");
        assert_eq!(task_of(&matte), Some(PlaneTask::Matte));
        for other in ["roto_brush", "camera_track", "blur"] {
            let fx = instantiate(other).expect("declared");
            assert_eq!(task_of(&fx), None, "{other}");
        }
        assert_eq!(task_of_name("nothing_of_the_sort"), None);
    }

    /// **The model row renames every frame and the view row renames none.**
    /// The first decides what the model produced; the second decides how it is
    /// drawn, and a cache thrown away for a glance is the mirror mistake the
    /// per-frame stamp exists to avoid (§13).
    #[test]
    fn a_settings_change_renames_everything_and_a_view_change_nothing() {
        let mut fx = instantiate(DEPTH).expect("declared");
        let before: Vec<[u8; 32]> = (0..4).map(|f| frame_stamp(&fx, f).unwrap()).collect();

        set(&mut fx, "view", EffectValue::Choice(1));
        let viewed: Vec<[u8; 32]> = (0..4).map(|f| frame_stamp(&fx, f).unwrap()).collect();
        assert_eq!(before, viewed, "the view is not part of the answer");

        set(&mut fx, "invert", EffectValue::Bool(true));
        let inverted: Vec<[u8; 32]> = (0..4).map(|f| frame_stamp(&fx, f).unwrap()).collect();
        assert_eq!(before, inverted, "nor is the invert");

        set(&mut fx, "model", EffectValue::Choice(1));
        let modelled: Vec<[u8; 32]> = (0..4).map(|f| frame_stamp(&fx, f).unwrap()).collect();
        for (was, is) in before.iter().zip(&modelled) {
            assert_ne!(was, is, "a different model is a different plane");
        }
        // And each frame is named apart from its neighbours, so the shape is
        // ready for the day one frame carries something of its own.
        assert_eq!(
            before
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            before.len()
        );
    }

    /// **The detail row renames every frame and the view row renames none.**
    /// Detail is how much of the frame the model works at, so it decides what
    /// the model produced; the view decides how the matte is drawn.
    #[test]
    fn the_detail_row_is_part_of_what_a_matte_is() {
        let mut fx = instantiate(REMOVE_BACKGROUND).expect("declared");
        let before: Vec<[u8; 32]> = (0..4).map(|f| frame_stamp(&fx, f).unwrap()).collect();

        set(&mut fx, "view", EffectValue::Choice(1));
        set(&mut fx, "invert", EffectValue::Bool(true));
        let shown: Vec<[u8; 32]> = (0..4).map(|f| frame_stamp(&fx, f).unwrap()).collect();
        assert_eq!(before, shown, "how a matte is drawn is not what it is");

        set(&mut fx, "detail", EffectValue::Choice(1));
        let coarser: Vec<[u8; 32]> = (0..4).map(|f| frame_stamp(&fx, f).unwrap()).collect();
        for (was, is) in before.iter().zip(&coarser) {
            assert_ne!(was, is, "another detail is another matte");
        }
    }

    /// **Nothing is stamped for a layer with no such effect.** Otherwise every
    /// cached frame of every project written before this existed would be
    /// renamed the moment the effect shipped.
    #[test]
    fn an_effect_of_another_kind_stamps_nothing() {
        let fx = instantiate("roto_brush").expect("declared");
        assert!(frame_stamp(&fx, 0).is_none());
        assert_eq!(analyses(&[fx]).count(), 0);
    }
}
