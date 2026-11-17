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

use crate::model::{Composition, EffectInstance, EffectValue, Layer};
use crate::time::CompTime;

/// The `planes/` sidecar tier's version, fed into every hash here so a build
/// that changes the meaning of a plane cannot read the old one back. Bumping it
/// orphans every cached plane, which costs one Analyse.
pub const TIER_VERSION: u16 = 1;

/// The Depth effect's `match_name`, so the render path and the frame key can
/// find it without importing the catalogue's type.
pub const DEPTH: &str = "depth";

/// Depth points' `match_name`: the same analysis, drawn another way.
pub const DEPTH_POINTS: &str = "depth_points";

/// The Remove background effect's, for the same reason.
pub const REMOVE_BACKGROUND: &str = "remove_background";

/// The **Frames** row's options, in index order, on every effect whose
/// analysis reads a whole shot: Depth, Remove background, the Roto brush and
/// the three trackers. **All frames** is the default, so a project made before
/// the row existed reads exactly as it did.
pub const FRAMES_OPTIONS: &[&str] = &["All frames", "Layer in to out", "Work area"];

/// The source times, first and last, in seconds, an analysis on `layer` is
/// limited to by the effect's Frames row, or `None` for all of its source.
///
/// `layer` is `None` for a node graph box. A box is on screen for the whole
/// composition, so Layer in to out reads as the composition's length, and its
/// Read has no offset or Retime, so the source runs on the composition's own
/// clock. A comp with no work area set reads as All frames.
///
/// Seconds rather than frames: this crate knows when a layer is on screen and
/// not how fast its media runs, so the caller picks the frames.
///
/// ponytail: only the two ends are mapped, so a Retime that runs backwards
/// and forwards again inside the span is limited to what its ends show.
#[must_use]
pub fn source_span(
    fx: &EffectInstance,
    layer: Option<&Layer>,
    comp: &Composition,
) -> Option<(f64, f64)> {
    let (from, to) = match fx.param("frames") {
        Some(EffectValue::Choice(1)) => layer
            .map_or((CompTime::ZERO, CompTime(comp.duration.0)), |l| {
                (l.in_point, l.out_point)
            }),
        Some(EffectValue::Choice(2)) => comp.work_area?,
        _ => return None,
    };
    // The first frame starting at or after `from`, and the last one starting
    // before `to`, which is when a layer is on screen.
    let fps = comp.frame_rate.fps();
    let first = (from.0.to_f64() * fps - 1e-9).ceil() as i64;
    let last = ((to.0.to_f64() * fps - 1e-9).ceil() as i64 - 1).max(first);
    let at = |frame: i64| {
        let t = comp.frame_rate.time_of_frame(frame).ok()?.0.to_f64();
        Some(layer.map_or(t, |l| {
            l.source_time_at(crate::time::layer_time(t, l.start_offset.0))
        }))
    };
    let (a, b) = (at(first)?, at(last)?);
    Some((a.min(b), a.max(b)))
}

/// What a planes-tier effect asks a model for.
///
/// The document's own copy of `lumit_ml::manifest::Task`, because `lumit-core` is the
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
        DEPTH | DEPTH_POINTS => Some(PlaneTask::Depth),
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
    use crate::fx::builtins::instantiate;
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

    /// **The Frames row picks the source seconds an analysis reads**, through
    /// the layer's own clock on a layer and the comp's on a node graph box.
    #[test]
    fn the_frames_row_picks_the_source_span() {
        use crate::model::{LinearColour, MotionBlur};
        use crate::time::{Duration, FrameRate, Rational};
        let secs = |n: i64| CompTime(Rational::new(n, 1).unwrap());
        let mut comp = Composition {
            id: uuid::Uuid::now_v7(),
            name: "Comp".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(25, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            groups: Vec::new(),
            markers: Vec::new(),
            motion_blur: MotionBlur::default(),
            master_volume_db: 0.0,
            master_keys: None,
            master_automation: Default::default(),
            loudness_target: None,
            lighting: Default::default(),
            sound_mix: false,
            beat_grid: None,
            graph: None,
            extra: serde_json::Map::new(),
        };
        let mut layer = crate::edits::null_layer(&comp);
        layer.in_point = secs(2);
        layer.out_point = secs(6);
        layer.start_offset = secs(1);
        let mut fx = instantiate(DEPTH).expect("declared");

        assert_eq!(source_span(&fx, Some(&layer), &comp), None, "All frames");

        set(&mut fx, "frames", EffectValue::Choice(1));
        // On screen from 2s to the frame before 6s, which is 1s later in the
        // layer's own time.
        assert_eq!(source_span(&fx, Some(&layer), &comp), Some((1.0, 4.96)));
        assert_eq!(source_span(&fx, None, &comp), Some((0.0, 9.96)), "a box");

        set(&mut fx, "frames", EffectValue::Choice(2));
        assert_eq!(source_span(&fx, None, &comp), None, "no work area set");
        comp.work_area = Some((secs(3), secs(4)));
        assert_eq!(source_span(&fx, None, &comp), Some((3.0, 3.96)));
    }
}
