//! The roto brush's **document** half (docs/impl/roto.md §1): the
//! strokes the user draws, the base frame they start from, and the two hashes
//! that decide which cached mattes an edit throws away.
//!
//! # In plain terms
//!
//! Rotoscoping is cutting a moving thing out of its shot. The user scribbles on
//! one frame — green through the subject, red through the background — and the
//! engine works out a **matte** (a black-and-white picture saying where the
//! subject is) for that frame and then carries it forward and backward through
//! the shot by watching how the picture moved.
//!
//! Two kinds of state, and keeping them apart is the whole design. The
//! **strokes** are the edit: they live in the project file, they undo, they are
//! what the user actually did. The **mattes** are derived — worked out from the
//! strokes, kept in a cache folder, and thrown away without loss. Nothing in
//! this file knows how to compute a matte; it holds the strokes and answers one
//! question about them very precisely: *if this stroke changed, which cached
//! mattes are now wrong?*
//!
//! The answer is the note's one invalidation rule. The matte at frame `F`
//! depends on the media, the settings, the base frame, and **only** the strokes
//! drawn between the base and `F` on `F`'s side of the base. So editing a
//! stroke on frame `n` spoils every frame at least as far from the base as `n`
//! is, on `n`'s side, and nothing else. [`chain_hash`] is that sentence as
//! arithmetic: a frame whose contributing strokes did not change keeps its
//! hash, keeps its cached matte, and keeps the name its rendered frame was
//! filed under.
//!
//! A **prompt** is the second kind of edit, and it keeps the same company: a
//! few taps saying "this thing, not that one", which a segmentation model reads
//! into a first guess at the base frame's matte when the effect's seed row asks
//! for one (docs/impl/addons.md §6.2). The taps are the edit and live here; the
//! model's answer is derived like every other matte, and it goes through the
//! same predicate, so a prompt spoils exactly the frames a stroke in its place
//! would.
//!
//! There is deliberately **no roto-shaped op**: a stroke edit rides
//! `Op::SetLayerEffects`, the coarse-and-exactly-invertible whole-stack commit
//! every parameter edit already uses, so strokes undo, journal and replay with
//! no second mechanism.
//! ponytail: the History row therefore reads "Edit effects" rather than "Draw a
//! roto stroke"; add a `SetRotoStrokes` with its own label the day that wording
//! is worth an op arm, its serde and its tests.
//!
//! **Why the strokes are not effect parameters.** A parameter is a number the
//! timeline animates and the frame key hashes whole. Strokes are neither: they
//! are a growing table where the *position in time* of each entry decides who it
//! affects, and hashing the table whole would rename every cached frame in the
//! shot every time the user corrected one of them. So they ride on the effect
//! instance beside the audio plugin's state blob, and the chain hash is what
//! reaches the frame key instead.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{EffectInstance, EffectValue};

/// The `roto/` sidecar tier's format version, fed into every hash here so a
/// build that changes the meaning of a matte cannot read the old one back.
/// Bumping it orphans every cached matte, which costs one re-propagation.
///
/// Version 2 is the one that can seed the base frame from a segmentation model
/// (docs/impl/addons.md §6.2): what a matte is a function of grew, so the
/// mattes a version 1 build wrote are never asked for again.
pub const TIER_VERSION: u16 = 2;

/// The Roto brush's `match_name`, so the render path and the frame key can find
/// the effect without importing the catalogue's type.
pub const ROTO_BRUSH: &str = "roto_brush";

/// What a stroke claims about the pixels under it.
///
/// The document's own copy of `lumit_roto::StrokeKind`. `lumit-core` is the
/// bottom of the crate graph and may not depend on the arithmetic crate above
/// it (docs/05 §1.1) — the same split the Camera track's density table takes
/// (docs/impl/tracking.md §5a, deviation 1). The render path converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RotoStrokeKind {
    /// These pixels are the subject.
    Foreground,
    /// These pixels are not.
    Background,
    /// These pixels want the refine band, whatever the segmentation decided.
    Refine,
}

impl RotoStrokeKind {
    /// A stable byte for the hashes, so a reordering of this enum never renames
    /// a cached matte.
    const fn tag(self) -> u8 {
        match self {
            RotoStrokeKind::Foreground => 1,
            RotoStrokeKind::Background => 2,
            RotoStrokeKind::Refine => 3,
        }
    }
}

/// One roto stroke: the path the pointer took, and what it claims.
///
/// Points are **source raster pixels** on the full, unaltered footage,
/// so the matte describes the file's frames and survives every comp-side
/// transform, retime and preview tier — and one shot's mattes serve every comp
/// that cuts it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotoStroke {
    pub id: Uuid,
    /// The pointer's path in source raster pixels, in the order it was drawn.
    pub points: Vec<(f32, f32)>,
    /// Half the brush's width, in source raster pixels.
    pub radius: f32,
    pub kind: RotoStrokeKind,
    /// The **source** frame index the stroke was drawn on.
    pub frame: i64,
}

/// One segmentation prompt: the taps the user made on a frame, and what each
/// of them claims.
///
/// Points are **source raster pixels**, as a stroke's are, so the prompt
/// describes the file's frames and survives every comp-side transform. A label
/// is 1 for a tap on the subject and 0 for one against it, one per point; a
/// label list that does not match the points is refused at the seam it arrives
/// through rather than stored.
///
/// The prompt is the edit and lives in the project file. The mask the model
/// makes of it is derived, kept in the cache folder, and thrown away without
/// loss, exactly as a matte is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotoPrompt {
    pub id: Uuid,
    /// The **source** frame index the taps were made on.
    pub frame: i64,
    /// Where the user tapped, in source raster pixels, in the order tapped.
    pub points: Vec<(f32, f32)>,
    /// One per point: 1 for the subject, 0 against it.
    pub labels: Vec<u8>,
}

/// Everything a Roto brush instance carries that is not a parameter: the base
/// frame, the stroke table and the prompt table, in document order.
///
/// Absent on an instance nobody has stroked, and left out of the saved file
/// then, so a project written before roto existed reads and saves back byte for
/// byte.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RotoBlock {
    /// The frame propagation runs outward from — the first frame the user
    /// stroked, re-assignable from the panel. `None` until the first stroke,
    /// which is what makes `NoBaseFrame` a refusal rather than a guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_frame: Option<i64>,
    /// Ordered, undoable, journaled. Later strokes win where two overlap.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<RotoStroke>,
    /// The taps a segmentation model is asked to read, ordered the same way and
    /// left out of the saved file when there are none, so a project written
    /// before this existed reads and saves back byte for byte.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<RotoPrompt>,
}

impl RotoBlock {
    /// Whether there is anything here at all — an empty block is stored as no
    /// block.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.base_frame.is_none() && self.strokes.is_empty() && self.prompts.is_empty()
    }

    /// The frames propagation could reach, as `(first, last)` over the strokes
    /// and the base — what the panel calls the analysed span before a run has
    /// happened. `None` with no base frame.
    #[must_use]
    pub fn stroked_range(&self) -> Option<(i64, i64)> {
        let base = self.base_frame?;
        let mut first = base;
        let mut last = base;
        for s in &self.strokes {
            first = first.min(s.frame);
            last = last.max(s.frame);
        }
        Some((first, last))
    }

    /// The strokes that decide frame `frame`, in document order: those drawn
    /// between the base and `frame` **inclusive, on `frame`'s side of the
    /// base**.
    ///
    /// This is the note's purity sentence, and every hash below is it restated.
    /// A stroke on the far side of the base contributes nothing — influence
    /// flows outward, and a user who wants the base re-decided moves the base.
    #[must_use]
    pub fn contributing(&self, frame: i64) -> Vec<&RotoStroke> {
        let Some(base) = self.base_frame else {
            return Vec::new();
        };
        self.strokes
            .iter()
            .filter(|s| between(base, frame, s.frame))
            .collect()
    }

    /// The prompts that decide frame `frame`, by the same rule the strokes
    /// follow: influence flows outward from the base, so a prompt on the far
    /// side of it decides nothing here.
    #[must_use]
    pub fn contributing_prompts(&self, frame: i64) -> Vec<&RotoPrompt> {
        let Some(base) = self.base_frame else {
            return Vec::new();
        };
        self.prompts
            .iter()
            .filter(|p| between(base, frame, p.frame))
            .collect()
    }
}

/// Whether a stroke on frame `n` decides frame `f`, given the base: `n` is on
/// `f`'s side of the base and no further out than `f` is.
///
/// Both ends inclusive, and the base itself always counts — its strokes are
/// what every propagation starts from, in both directions.
#[must_use]
fn between(base: i64, f: i64, n: i64) -> bool {
    if f >= base {
        (base..=f).contains(&n)
    } else {
        (f..=base).contains(&n)
    }
}

/// The settings that change what a matte **is**, read off a Roto brush
/// instance.
///
/// Deliberately not every parameter: the view and the matte mode change how the
/// matte is *shown*, not what it holds, and hashing them would throw the cache
/// away every time the user glanced at the boundary. What is here is what the
/// propagation reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotoSettings {
    /// The guided filter's window radius, in source pixels.
    pub refine_radius: f32,
    /// The flow's measuring resolution as the effect's Choice index: 0 native,
    /// 1 half (the default), 2 quarter.
    pub flow_resolution: u32,
    /// The flow's regularisation, 0–100.
    pub flow_smoothness: f32,
    /// What seeds the base frame, as the effect's Choice index: 0 the strokes
    /// alone (the default), 1 a segmentation model reading the prompt.
    pub seed: u32,
    /// Which segmentation pack, and which version of it, read the prompt
    /// (docs/impl/addons.md §7).
    ///
    /// Zeros unless the seed row says Segment, and zeros here even then: this
    /// is a fact about the machine rather than about the document, and
    /// `lumit-core` is the bottom of the crate graph and may not ask the crate
    /// that opens a model. The render path fills it in when it builds the job,
    /// which is what stops a run solved under one pack lending its frames to a
    /// run under another.
    pub identity: [u8; 32],
}

impl Default for RotoSettings {
    fn default() -> Self {
        RotoSettings {
            refine_radius: 8.0,
            flow_resolution: 1,
            flow_smoothness: 50.0,
            seed: 0,
            identity: [0; 32],
        }
    }
}

impl RotoSettings {
    /// Read the settings off one Roto brush instance. A parameter the instance
    /// does not carry — an older project, a hand-edited file — reads as the
    /// default rather than failing (docs/14 §4).
    ///
    /// **At layer time nought**, not at the playhead: these two numbers name a
    /// cache, and a cache key that moved with the playhead would file every
    /// frame under a different name. A user who keyframes the refine radius
    /// gets the value it holds at the layer's start, which is the honest
    /// reading for a setting a whole run is measured under.
    #[must_use]
    pub fn of(fx: &EffectInstance) -> Self {
        let d = Self::default();
        let float = |id: &str, fallback: f32| match fx.param(id) {
            Some(EffectValue::Float(p)) => p.value_at(0.0) as f32,
            _ => fallback,
        };
        RotoSettings {
            refine_radius: float("refine_radius", d.refine_radius),
            flow_resolution: match fx.param("flow_resolution") {
                Some(EffectValue::Choice(v)) => *v,
                _ => d.flow_resolution,
            },
            flow_smoothness: float("flow_smoothness", d.flow_smoothness),
            seed: match fx.param("seed") {
                Some(EffectValue::Choice(v)) => *v,
                _ => d.seed,
            },
            identity: d.identity,
        }
    }

    /// Whether the base frame is seeded by a model reading the prompt.
    #[must_use]
    pub fn segments(&self) -> bool {
        self.seed == crate::fx::effects::roto_brush::SEED_SEGMENT
    }

    /// Feed the settings into a hash, in a fixed order.
    ///
    /// The model's identity is fed **only** where the seed row asks for a
    /// model, so installing a segmentation pack renames nothing on a brush
    /// that is seeded by its strokes, which is every brush in every project
    /// written before this existed.
    pub fn feed(&self, h: &mut blake3::Hasher) {
        h.update(&self.refine_radius.to_bits().to_le_bytes());
        h.update(&self.flow_resolution.to_le_bytes());
        h.update(&self.flow_smoothness.to_bits().to_le_bytes());
        h.update(&self.seed.to_le_bytes());
        if self.segments() {
            h.update(&self.identity);
        }
    }
}

/// Feed one stroke into a hash. Its **id is not in it**: two identical
/// scribbles produce the same matte, and a stroke that was deleted and redrawn
/// deserves the cached answer it already has.
fn feed_stroke(h: &mut blake3::Hasher, s: &RotoStroke) {
    h.update(&[s.kind.tag()]);
    h.update(&s.frame.to_le_bytes());
    h.update(&s.radius.to_bits().to_le_bytes());
    h.update(&(s.points.len() as u64).to_le_bytes());
    for (x, y) in &s.points {
        h.update(&x.to_bits().to_le_bytes());
        h.update(&y.to_bits().to_le_bytes());
    }
}

/// Feed one prompt into a hash. Its **id is not in it**, for the reason a
/// stroke's is not: the same taps ask the model the same question, and a prompt
/// that was cleared and tapped again deserves the answer it already has.
fn feed_prompt(h: &mut blake3::Hasher, p: &RotoPrompt) {
    h.update(&p.frame.to_le_bytes());
    h.update(&(p.points.len() as u64).to_le_bytes());
    for (x, y) in &p.points {
        h.update(&x.to_bits().to_le_bytes());
        h.update(&y.to_bits().to_le_bytes());
    }
    h.update(&(p.labels.len() as u64).to_le_bytes());
    h.update(&p.labels);
}

/// The **chain hash** of frame `frame`: everything the matte at that frame is a
/// function of, apart from the media itself.
///
/// The tier version, the settings, the base frame, and the contributing strokes
/// and prompts in document order ([`RotoBlock::contributing`] and
/// [`RotoBlock::contributing_prompts`]), which together are the whole
/// invalidation rule. Two frames on opposite sides of the base with the same
/// distance from it hash differently, because the side is in the frame number
/// the chain runs to.
///
/// `None` when there is no base frame: nothing has been decided, so there is
/// nothing to name.
#[must_use]
pub fn chain_hash(block: &RotoBlock, settings: RotoSettings, frame: i64) -> Option<[u8; 32]> {
    let base = block.base_frame?;
    let mut h = blake3::Hasher::new();
    h.update(b"lumit-roto/chain/");
    h.update(&TIER_VERSION.to_le_bytes());
    settings.feed(&mut h);
    h.update(&base.to_le_bytes());
    h.update(&frame.to_le_bytes());
    for s in block
        .strokes
        .iter()
        .filter(|s| between(base, frame, s.frame))
    {
        feed_stroke(&mut h, s);
    }
    h.update(b"prompts/");
    for p in block
        .prompts
        .iter()
        .filter(|p| between(base, frame, p.frame))
    {
        feed_prompt(&mut h, p);
    }
    Some(*h.finalize().as_bytes())
}

/// The **run key**: what one `.lrot` sidecar file is filed under, minus the
/// media's own fingerprint, which the render path adds because only it knows
/// one.
///
/// The whole stroke table and the whole prompt table, not a prefix: a file
/// holds a whole run, and a run under a different table is a different run.
/// Which of its *frames* survive an edit is the chain hash's question, asked
/// per record.
#[must_use]
pub fn key_hash(block: &RotoBlock, settings: RotoSettings) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"lumit-roto/key/");
    h.update(&TIER_VERSION.to_le_bytes());
    settings.feed(&mut h);
    h.update(&block.base_frame.unwrap_or(i64::MIN).to_le_bytes());
    h.update(&(block.strokes.len() as u64).to_le_bytes());
    for s in &block.strokes {
        feed_stroke(&mut h, s);
    }
    h.update(&(block.prompts.len() as u64).to_le_bytes());
    for p in &block.prompts {
        feed_prompt(&mut h, p);
    }
    *h.finalize().as_bytes()
}

/// The Roto brush instances on a layer that are switched on, in stack order.
///
/// One layer may wear more than one — a subject and a shadow, cut separately —
/// and each has its own strokes, its own base and its own cached run.
pub fn brushes(effects: &[EffectInstance]) -> impl Iterator<Item = &EffectInstance> {
    effects
        .iter()
        .filter(|e| e.enabled && e.effect.match_name == ROTO_BRUSH)
}

/// The chain hash that names frame `frame` of the matte `fx` holds, or `None`
/// when `fx` is not a stroked Roto brush.
///
/// The one call the frame key makes (docs/impl/roto.md §5): a stroke edit
/// renames exactly the frames it invalidated, and a frame whose strokes did not
/// move keeps the name its picture was banked under.
#[must_use]
pub fn frame_stamp(fx: &EffectInstance, frame: i64) -> Option<[u8; 32]> {
    let block = fx.roto.as_ref()?;
    chain_hash(block, RotoSettings::of(fx), frame)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn stroke(frame: i64, x: f32) -> RotoStroke {
        RotoStroke {
            id: Uuid::now_v7(),
            points: vec![(x, 1.0), (x + 2.0, 1.0)],
            radius: 4.0,
            kind: RotoStrokeKind::Foreground,
            frame,
        }
    }

    fn block(base: i64, frames: &[i64]) -> RotoBlock {
        RotoBlock {
            base_frame: Some(base),
            strokes: frames.iter().map(|&f| stroke(f, 10.0)).collect(),
            prompts: Vec::new(),
        }
    }

    fn prompt(frame: i64, x: f32) -> RotoPrompt {
        RotoPrompt {
            id: Uuid::now_v7(),
            frame,
            points: vec![(x, 6.0)],
            labels: vec![1],
        }
    }

    /// A block whose base frame is decided by a prompt rather than by a
    /// stroke, with the seed row set to match.
    fn prompted(base: i64, frames: &[i64]) -> (RotoBlock, RotoSettings) {
        (
            RotoBlock {
                base_frame: Some(base),
                strokes: Vec::new(),
                prompts: frames.iter().map(|&f| prompt(f, 20.0)).collect(),
            },
            RotoSettings {
                seed: crate::fx::effects::roto_brush::SEED_SEGMENT,
                ..RotoSettings::default()
            },
        )
    }

    #[test]
    fn influence_flows_outward_from_the_base_and_never_back() {
        let b = block(10, &[10, 5, 15]);
        // Forward of the base: the base's stroke and the one at 15, never the
        // correction at 5 on the other side.
        let f = b.contributing(20);
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|s| s.frame == 10 || s.frame == 15));
        // Backward: the base and the one at 5.
        let back = b.contributing(4);
        assert_eq!(back.len(), 2);
        assert!(back.iter().all(|s| s.frame == 10 || s.frame == 5));
        // The base itself is decided by its own stroke alone.
        assert_eq!(b.contributing(10).len(), 1);
    }

    #[test]
    fn a_correction_renames_exactly_the_frames_past_it() {
        let s = RotoSettings::default();
        let before = block(0, &[0]);
        let mut after = before.clone();
        after.strokes.push(stroke(20, 40.0));

        // Every frame before the correction keeps its name...
        for f in 0..20 {
            assert_eq!(
                chain_hash(&before, s, f),
                chain_hash(&after, s, f),
                "frame {f} was renamed by a correction it does not depend on"
            );
        }
        // ...and every frame from it onward is renamed.
        for f in 20..30 {
            assert_ne!(chain_hash(&before, s, f), chain_hash(&after, s, f));
        }
        // The other side of the base is untouched: influence flows outward.
        for f in -10..0 {
            assert_eq!(chain_hash(&before, s, f), chain_hash(&after, s, f));
        }
    }

    #[test]
    fn a_setting_or_a_base_move_renames_everything() {
        let b = block(10, &[10]);
        let d = RotoSettings::default();
        let wider = RotoSettings {
            refine_radius: 16.0,
            ..d
        };
        assert_ne!(chain_hash(&b, d, 12), chain_hash(&b, wider, 12));
        assert_ne!(key_hash(&b, d), key_hash(&b, wider));

        let moved = block(11, &[10]);
        assert_ne!(chain_hash(&b, d, 12), chain_hash(&moved, d, 12));
        assert_ne!(key_hash(&b, d), key_hash(&moved, d));
    }

    #[test]
    fn an_unstroked_block_names_nothing() {
        let b = RotoBlock::default();
        assert!(b.is_empty());
        assert!(chain_hash(&b, RotoSettings::default(), 0).is_none());
        assert!(b.stroked_range().is_none());
        assert!(b.contributing(0).is_empty());
    }

    /// **A prompt obeys the purity sentence the strokes do.** The taps decide
    /// the frames outward from the base on their own side and nothing else, so
    /// a prompt is a contributor rather than a fact about the shot, and every
    /// cache guarantee in the note holds for it unchanged.
    #[test]
    fn a_prompt_flows_outward_from_the_base_and_never_back() {
        let (b, _) = prompted(10, &[10, 5, 15]);
        let forward = b.contributing_prompts(20);
        assert_eq!(forward.len(), 2);
        assert!(forward.iter().all(|p| p.frame == 10 || p.frame == 15));
        let back = b.contributing_prompts(4);
        assert_eq!(back.len(), 2);
        assert!(back.iter().all(|p| p.frame == 10 || p.frame == 5));
        assert_eq!(b.contributing_prompts(10).len(), 1);
        assert!(RotoBlock::default().contributing_prompts(0).is_empty());
    }

    /// **A prompt edit renames exactly the frames past it**, in both
    /// directions, and leaves the far side of the base alone.
    #[test]
    fn a_prompt_renames_exactly_the_frames_past_it() {
        let (before, s) = prompted(0, &[0]);
        let mut after = before.clone();
        after.prompts.push(prompt(20, 44.0));

        for f in 0..20 {
            assert_eq!(
                chain_hash(&before, s, f),
                chain_hash(&after, s, f),
                "frame {f} was renamed by a prompt it does not depend on"
            );
        }
        for f in 20..30 {
            assert_ne!(chain_hash(&before, s, f), chain_hash(&after, s, f));
        }
        for f in -10..0 {
            assert_eq!(chain_hash(&before, s, f), chain_hash(&after, s, f));
        }
        // Moving a tap is an edit, and it retires the run's whole file.
        let mut moved = before.clone();
        moved.prompts[0].points[0].0 += 1.0;
        assert_ne!(key_hash(&before, s), key_hash(&moved, s));
        let mut negative = before.clone();
        negative.prompts[0].labels[0] = 0;
        assert_ne!(
            key_hash(&before, s),
            key_hash(&negative, s),
            "a tap against the subject asks a different question"
        );
    }

    /// **A prompt cleared and tapped again keeps the cache it earned.** The id
    /// is not in either hash, for the reason a stroke's is not.
    #[test]
    fn a_retapped_prompt_keeps_the_cache_it_earned() {
        let (one, s) = prompted(0, &[3]);
        let mut two = one.clone();
        two.prompts[0].id = Uuid::now_v7();
        assert_eq!(chain_hash(&one, s, 9), chain_hash(&two, s, 9));
        assert_eq!(key_hash(&one, s), key_hash(&two, s));
    }

    /// **The seed row renames everything, and so does the pack behind it.**
    ///
    /// The identity is what fences `lendable`, which reads every sidecar of the
    /// same media and trusts the chain hash alone: without this a run seeded by
    /// one pack would lend its frames to a run seeded by another. And a brush
    /// seeded by its strokes is renamed by neither, so installing a pack costs
    /// every existing project nothing.
    #[test]
    fn the_seed_choice_and_the_pack_behind_it_rename_everything() {
        let b = block(0, &[0]);
        let strokes = RotoSettings::default();
        let segment = RotoSettings {
            seed: crate::fx::effects::roto_brush::SEED_SEGMENT,
            ..strokes
        };
        assert_ne!(chain_hash(&b, strokes, 4), chain_hash(&b, segment, 4));
        assert_ne!(key_hash(&b, strokes), key_hash(&b, segment));

        let other_pack = RotoSettings {
            identity: [9; 32],
            ..segment
        };
        assert_ne!(chain_hash(&b, segment, 4), chain_hash(&b, other_pack, 4));
        assert_ne!(key_hash(&b, segment), key_hash(&b, other_pack));

        // The same pack installed beside a brush that reads its strokes is not
        // an edit to that brush.
        let installed = RotoSettings {
            identity: [9; 32],
            ..strokes
        };
        assert_eq!(chain_hash(&b, strokes, 4), chain_hash(&b, installed, 4));
        assert_eq!(key_hash(&b, strokes), key_hash(&b, installed));
    }

    #[test]
    fn a_redrawn_stroke_keeps_the_cache_it_earned() {
        let d = RotoSettings::default();
        let one = block(0, &[3]);
        let mut two = one.clone();
        // Same geometry, new identity — a delete and a redraw.
        two.strokes[0].id = Uuid::now_v7();
        assert_eq!(chain_hash(&one, d, 9), chain_hash(&two, d, 9));
        assert_eq!(key_hash(&one, d), key_hash(&two, d));
    }
}
