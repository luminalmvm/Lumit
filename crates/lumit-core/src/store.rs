//! The document store: immutable snapshots + operation journal
//! (docs/05-ARCHITECTURE.md; docs/impl/playback-scheduler.md §3).
//!
//! The UI thread is the single writer (by convention); readers grab an
//! `Arc<Document>` snapshot at any time, lock-free, and never observe a
//! half-applied edit.

use crate::model::Document;
use crate::ops::{apply, Op, OpError};
use arc_swap::ArcSwap;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One journal entry: the op as applied, and its exact inverse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub op: Op,
    pub inverse: Op,
}

/// The most undo steps kept in memory (docs/14 §5 compaction story, below).
/// Generous enough that no real editing session reaches it, small enough that
/// the history can never grow without bound. Editing software the owner knows
/// keeps far fewer (After Effects defaults to 32); 500 is a comfortable margin.
pub const MAX_UNDO_DEPTH: usize = 500;

/// The in-memory undo/redo history.
///
/// **Compaction story (docs/14 §5, mandatory for long-lived collections):**
/// `undo` is bounded to [`MAX_UNDO_DEPTH`] entries. Each [`DocumentStore::commit`]
/// that pushes past the cap drops the *oldest* entries — you can no longer undo
/// past that point, but the current document is untouched (dropping history
/// never changes state). `redo` needs no separate bound: it only ever holds
/// entries moved off `undo` by [`DocumentStore::undo`], so it can never exceed
/// the undo depth, and any [`DocumentStore::commit`] clears it outright.
/// Crash recovery does not rely on this history — lumit-project appends every
/// op to an on-disk journal as it is committed, independently of the cap.
#[derive(Default)]
struct Journal {
    undo: Vec<JournalEntry>,
    redo: Vec<JournalEntry>,
}

pub struct DocumentStore {
    current: ArcSwap<Document>,
    journal: Mutex<Journal>,
}

impl DocumentStore {
    pub fn new(doc: Document) -> Self {
        Self {
            current: ArcSwap::from_pointee(doc),
            journal: Mutex::new(Journal::default()),
        }
    }

    /// Lock-free snapshot for readers (render jobs capture this at schedule time).
    pub fn snapshot(&self) -> Arc<Document> {
        self.current.load_full()
    }

    /// Apply an operation, journal it, publish the new snapshot.
    pub fn commit(&self, op: Op) -> Result<Arc<Document>, OpError> {
        let mut journal = self.journal.lock();
        let mut doc = Document::clone(&self.snapshot());
        let inverse = apply(&mut doc, &op)?;
        journal.undo.push(JournalEntry { op, inverse });
        journal.redo.clear();
        // Compaction (docs/14 §5): keep the history bounded by dropping the
        // oldest steps once it exceeds the cap. Dropping history never changes
        // the document — only how far back an undo can reach.
        if journal.undo.len() > MAX_UNDO_DEPTH {
            let overflow = journal.undo.len() - MAX_UNDO_DEPTH;
            journal.undo.drain(..overflow);
        }
        let arc = Arc::new(doc);
        self.current.store(arc.clone());
        Ok(arc)
    }

    /// Undo the most recent operation. Ok(None) when there is nothing to undo.
    pub fn undo(&self) -> Result<Option<Arc<Document>>, OpError> {
        let mut journal = self.journal.lock();
        let Some(entry) = journal.undo.pop() else {
            return Ok(None);
        };
        let mut doc = Document::clone(&self.snapshot());
        // Applying the inverse yields the original op again — symmetry by construction.
        let op = apply(&mut doc, &entry.inverse)?;
        journal.redo.push(JournalEntry {
            op,
            inverse: entry.inverse.clone(),
        });
        let arc = Arc::new(doc);
        self.current.store(arc.clone());
        Ok(Some(arc))
    }

    /// Redo the most recently undone operation. Ok(None) when nothing to redo.
    pub fn redo(&self) -> Result<Option<Arc<Document>>, OpError> {
        let mut journal = self.journal.lock();
        let Some(entry) = journal.redo.pop() else {
            return Ok(None);
        };
        let mut doc = Document::clone(&self.snapshot());
        let inverse = apply(&mut doc, &entry.op)?;
        journal.undo.push(JournalEntry {
            op: entry.op,
            inverse,
        });
        let arc = Arc::new(doc);
        self.current.store(arc.clone());
        Ok(Some(arc))
    }

    /// The retained undo ops, oldest first (at most [`MAX_UNDO_DEPTH`] after
    /// compaction). Crash recovery does not read this — lumit-project appends
    /// each op to an on-disk journal as it is committed — so the cap dropping
    /// old entries here never loses a recoverable edit.
    pub fn journal_ops(&self) -> Vec<Op> {
        self.journal
            .lock()
            .undo
            .iter()
            .map(|e| e.op.clone())
            .collect()
    }

    pub fn can_undo(&self) -> bool {
        !self.journal.lock().undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.journal.lock().redo.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::ops::Op;
    use crate::time::{CompTime, Duration, FrameRate, Rational};
    use uuid::Uuid;

    fn t(n: i64, d: i64) -> CompTime {
        CompTime(Rational::new(n, d).unwrap())
    }

    fn test_comp() -> Composition {
        Composition {
            id: Uuid::now_v7(),
            name: "Comp 1".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(30, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn test_layer(item: Uuid) -> Layer {
        Layer {
            id: Uuid::now_v7(),
            name: "clip.mp4".into(),
            kind: LayerKind::Footage { item, retime: None },
            in_point: t(0, 1),
            out_point: t(10, 1),
            start_offset: t(0, 1),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: crate::anim::Property::zero(),
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn json(doc: &Document) -> String {
        serde_json::to_string(doc).unwrap()
    }

    /// Build a scripted edit sequence against a fresh store.
    fn scripted_ops(doc: &Document) -> (Vec<Op>, Uuid) {
        let comp = test_comp();
        let comp_id = comp.id;
        let footage = FootageItem {
            id: Uuid::now_v7(),
            name: "capture.mp4".into(),
            extra: serde_json::Map::new(),
            media: MediaRef {
                relative_path: "footage/capture.mp4".into(),
                absolute_path: "/tmp/capture.mp4".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
        };
        let layer = test_layer(footage.id);
        let layer_id = layer.id;
        let _ = doc;
        (
            vec![
                Op::AddItem {
                    index: 0,
                    item: Box::new(ProjectItem::Footage(footage)),
                },
                Op::AddItem {
                    index: 1,
                    item: Box::new(ProjectItem::Composition(comp)),
                },
                Op::AddLayer {
                    comp: comp_id,
                    index: 0,
                    layer: Box::new(layer),
                },
                Op::SetLayerSpan {
                    comp: comp_id,
                    layer: layer_id,
                    in_point: t(1, 2),
                    out_point: t(19, 2),
                    start_offset: t(1, 2),
                },
                Op::RenameLayer {
                    comp: comp_id,
                    layer: layer_id,
                    name: "hero shot".into(),
                },
                Op::RenameItem {
                    id: comp_id,
                    name: "Main edit".into(),
                },
            ],
            comp_id,
        )
    }

    #[test]
    fn undo_all_restores_initial_redo_all_restores_final() {
        let initial = Document::new();
        let initial_json = json(&initial);
        let store = DocumentStore::new(initial);
        let (ops, _) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }
        let final_json = json(&store.snapshot());

        while store.undo().unwrap().is_some() {}
        assert_eq!(json(&store.snapshot()), initial_json, "undo-all == initial");

        while store.redo().unwrap().is_some() {}
        assert_eq!(json(&store.snapshot()), final_json, "redo-all == final");
    }

    /// docs/14 §5: the undo history is compacted to [`MAX_UNDO_DEPTH`], and
    /// compaction never changes the document — it only limits how far back an
    /// undo can reach. Fails without the cap (the history would grow to every
    /// committed op).
    #[test]
    fn undo_history_is_capped_without_changing_the_document() {
        // Store and oracle must share one initial document (Document::new()
        // mints a fresh id each call, so two of them never compare equal).
        let initial = Document::new();
        let mut oracle = initial.clone();
        let store = DocumentStore::new(initial);
        let comp = test_comp();
        let comp_id = comp.id;
        // One AddItem, then well over the cap of cheap renames.
        let ops: Vec<Op> = std::iter::once(Op::AddItem {
            index: 0,
            item: Box::new(ProjectItem::Composition(comp)),
        })
        .chain((0..(MAX_UNDO_DEPTH + 50)).map(|i| Op::RenameItem {
            id: comp_id,
            name: format!("edit {i}"),
        }))
        .collect();

        // Oracle: apply every op straight through, no store, no cap.
        for op in &ops {
            apply(&mut oracle, op).unwrap();
        }
        for op in ops {
            store.commit(op).unwrap();
        }

        // Compaction dropped old history but not state: the store matches the
        // full replay exactly.
        assert_eq!(json(&store.snapshot()), json(&oracle));
        // The history is bounded, not the full run of commits.
        assert_eq!(store.journal_ops().len(), MAX_UNDO_DEPTH);

        // Every retained step undoes cleanly and no more (no underflow/panic).
        let mut undos = 0;
        while store.undo().unwrap().is_some() {
            undos += 1;
        }
        assert_eq!(undos, MAX_UNDO_DEPTH, "exactly the retained steps undo");
        // Redo is transitively bounded — all of it redoes back to the full state.
        let mut redos = 0;
        while store.redo().unwrap().is_some() {
            redos += 1;
        }
        assert_eq!(redos, MAX_UNDO_DEPTH);
        assert_eq!(
            json(&store.snapshot()),
            json(&oracle),
            "redo-all returns to the full state"
        );
    }

    #[test]
    fn journal_replay_reproduces_final_state() {
        let initial = Document::new();
        let mut replayed = initial.clone();
        let store = DocumentStore::new(initial);
        let (ops, _) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }
        for op in store.journal_ops() {
            crate::ops::apply(&mut replayed, &op).unwrap();
        }
        assert_eq!(json(&replayed), json(&store.snapshot()));
    }

    #[test]
    fn snapshots_are_isolated_from_later_edits() {
        let store = DocumentStore::new(Document::new());
        let before = store.snapshot();
        let (ops, _) = scripted_ops(&before);
        for op in ops {
            store.commit(op).unwrap();
        }
        assert!(before.items.is_empty(), "old snapshot unchanged");
        assert_eq!(store.snapshot().items.len(), 2);
    }

    #[test]
    fn commit_clears_redo() {
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }
        store.undo().unwrap();
        assert!(store.can_redo());
        store
            .commit(Op::RenameItem {
                id: comp_id,
                name: "diverged".into(),
            })
            .unwrap();
        assert!(!store.can_redo(), "new edit invalidates the redo branch");
    }

    #[test]
    fn transform_property_op_round_trips_through_undo() {
        use crate::anim::{Animation, Keyframe, SideInterp, EASY_EASE};
        use crate::model::TransformProp;
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        let mut layer_id = None;
        for op in &ops {
            if let Op::AddLayer { layer, .. } = op {
                layer_id = Some(layer.id);
            }
        }
        for op in ops {
            store.commit(op).unwrap();
        }
        let layer_id = layer_id.unwrap();

        let keys = vec![
            Keyframe {
                time: Rational::new(0, 1).unwrap(),
                value: 0.0,
                interp_in: SideInterp::Linear,
                interp_out: EASY_EASE,
            },
            Keyframe {
                time: Rational::new(2, 1).unwrap(),
                value: 100.0,
                interp_in: EASY_EASE,
                interp_out: SideInterp::Linear,
            },
        ];
        store
            .commit(Op::SetTransformProperty {
                comp: comp_id,
                layer: layer_id,
                prop: TransformProp::Opacity,
                animation: Animation::Keyframed(keys),
            })
            .unwrap();

        let doc = store.snapshot();
        let comp = doc.comp(comp_id).unwrap();
        let layer = comp.layers.iter().find(|l| l.id == layer_id).unwrap();
        assert!(layer.transform.opacity.is_animated());
        let mid = layer.transform.opacity.value_at(1.0);
        assert!((mid - 50.0).abs() < 1e-9, "eased midpoint {mid}");
        assert_eq!(layer.transform.opacity.value_at(-1.0), 0.0);
        assert_eq!(layer.transform.opacity.value_at(99.0), 100.0);

        // Undo restores the static default exactly.
        store.undo().unwrap();
        let doc = store.snapshot();
        let layer = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .unwrap();
        assert!(!layer.transform.opacity.is_animated());
        assert_eq!(layer.transform.opacity.value_at(1.0), 100.0);
    }

    #[test]
    fn reorder_layer_moves_and_undoes_exactly() {
        let store = DocumentStore::new(Document::new());
        let comp = test_comp();
        let comp_id = comp.id;
        store
            .commit(Op::AddItem {
                index: 0,
                item: Box::new(ProjectItem::Composition(comp)),
            })
            .unwrap();
        // Stack top-to-bottom: A (index 0), B, C.
        let mut ids = Vec::new();
        for _ in 0..3 {
            let layer = test_layer(Uuid::now_v7());
            ids.push(layer.id);
            store
                .commit(Op::AddLayer {
                    comp: comp_id,
                    index: 0,
                    layer: Box::new(layer),
                })
                .unwrap();
        }
        // Added top-first, so the final order is the reverse of insertion.
        let order = |s: &DocumentStore| -> Vec<Uuid> {
            s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .map(|l| l.id)
                .collect()
        };
        let before = order(&store);
        // Move the bottom layer to the top.
        let bottom = *before.last().unwrap();
        store
            .commit(Op::ReorderLayer {
                comp: comp_id,
                layer: bottom,
                new_index: 0,
            })
            .unwrap();
        let after = order(&store);
        assert_eq!(after.first(), Some(&bottom), "moved layer is now on top");
        assert_eq!(after.len(), 3);
        // Undo restores the exact original order.
        store.undo().unwrap();
        assert_eq!(order(&store), before, "reorder undo == original order");
    }

    /// A camera layer's zoom and a layer's 3D switch both round-trip through
    /// undo — the two ops the 2.5D camera work added.
    /// Retime on a Footage layer round-trips through undo; the op refuses a
    /// SetSequenceClips round-trips through undo (a cut is one such op).
    #[test]
    fn sequence_clips_op_round_trips() {
        use crate::model::{Layer, LayerKind, Switches, TransformGroup};
        use crate::sequence::{Clip, ClipSource};
        use crate::time::{CompTime, Rational};
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }
        let r = |n| Rational::new(n, 1).unwrap();
        let src = Uuid::now_v7();
        let one = Clip::new(ClipSource::Footage(src), r(0), r(4), r(0), r(4));
        let seq_id = Uuid::now_v7();
        store
            .commit(Op::AddLayer {
                comp: comp_id,
                index: 0,
                layer: Box::new(Layer {
                    id: seq_id,
                    name: "Seq".into(),
                    kind: LayerKind::Sequence {
                        clips: vec![one.clone()],
                    },
                    in_point: CompTime(r(0)),
                    out_point: CompTime(r(4)),
                    start_offset: CompTime(r(0)),
                    transform: TransformGroup::default(),
                    matte: None,
                    parent: None,
                    label: 0,
                    volume_db: crate::anim::Property::zero(),
                    blend: Default::default(),
                    masks: Vec::new(),
                    effects: Vec::new(),
                    switches: Switches::default(),
                    extra: serde_json::Map::new(),
                }),
            })
            .unwrap();
        // Cut into two, commit as SetSequenceClips.
        let (l, rc) = one.cut(r(2)).unwrap();
        store
            .commit(Op::SetSequenceClips {
                comp: comp_id,
                layer: seq_id,
                clips: vec![l, rc],
            })
            .unwrap();
        let n = |doc: &Document| match &doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == seq_id)
            .unwrap()
            .kind
        {
            LayerKind::Sequence { clips } => clips.len(),
            _ => 0,
        };
        assert_eq!(n(&store.snapshot()), 2);
        store.undo().unwrap();
        assert_eq!(n(&store.snapshot()), 1);
    }

    /// Retime on a Footage layer round-trips through undo; the op refuses a
    /// non-Footage target.
    #[test]
    fn retime_op_round_trips_and_targets_footage() {
        use crate::retime::Retime;
        use crate::time::Rational;
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        let mut layer_id = None;
        for op in &ops {
            if let Op::AddLayer { layer, .. } = op {
                layer_id = Some(layer.id);
            }
        }
        for op in ops {
            store.commit(op).unwrap();
        }
        let layer_id = layer_id.unwrap();

        let retime = Retime::constant_speed(
            Rational::new(10, 1).unwrap(),
            Rational::ZERO,
            Rational::new(1, 2).unwrap(),
        );
        store
            .commit(Op::SetLayerRetime {
                comp: comp_id,
                layer: layer_id,
                retime: Some(retime),
            })
            .unwrap();
        // Half speed: at local time 4 the source is 2.
        let doc = store.snapshot();
        let l = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .unwrap();
        if let crate::model::LayerKind::Footage { retime, .. } = &l.kind {
            assert!((retime.as_ref().unwrap().evaluate(4.0) - 2.0).abs() < 1e-9);
        } else {
            panic!("expected a footage layer");
        }
        store.undo().unwrap();
        let doc = store.snapshot();
        let l = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .unwrap();
        assert!(matches!(
            &l.kind,
            crate::model::LayerKind::Footage { retime: None, .. }
        ));
    }

    #[test]
    fn camera_zoom_and_three_d_ops_round_trip_through_undo() {
        use crate::anim::Animation;
        use crate::model::{Layer, LayerKind, Switches, TransformGroup};
        use crate::time::CompTime;
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        let mut layer_id = None;
        for op in &ops {
            if let Op::AddLayer { layer, .. } = op {
                layer_id = Some(layer.id);
            }
        }
        for op in ops {
            store.commit(op).unwrap();
        }
        let layer_id = layer_id.unwrap();
        let cam_id = uuid::Uuid::now_v7();
        let duration = store.snapshot().comp(comp_id).unwrap().duration.0;
        store
            .commit(Op::AddLayer {
                comp: comp_id,
                index: 0,
                layer: Box::new(Layer {
                    id: cam_id,
                    name: "Camera".into(),
                    kind: LayerKind::Camera {
                        zoom: crate::anim::Property::fixed(1000.0),
                    },
                    in_point: CompTime(Rational::ZERO),
                    out_point: CompTime(duration),
                    start_offset: CompTime(Rational::ZERO),
                    transform: TransformGroup::default(),
                    matte: None,
                    parent: None,
                    label: 0,
                    volume_db: crate::anim::Property::zero(),
                    blend: Default::default(),
                    masks: Vec::new(),
                    effects: Vec::new(),
                    switches: Switches::default(),
                    extra: serde_json::Map::new(),
                }),
            })
            .unwrap();

        store
            .commit(Op::SetCameraZoom {
                comp: comp_id,
                layer: cam_id,
                animation: Animation::Static(2500.0),
            })
            .unwrap();
        store
            .commit(Op::SetLayerThreeD {
                comp: comp_id,
                layer: layer_id,
                three_d: true,
            })
            .unwrap();

        let doc = store.snapshot();
        let comp = doc.comp(comp_id).unwrap();
        assert_eq!(comp.camera_pose(1.0).unwrap().zoom, 2500.0);
        let layer = comp.layers.iter().find(|l| l.id == layer_id).unwrap();
        assert!(layer.switches.three_d);

        // Mute round-trips the same way (audible defaults true).
        store
            .commit(Op::SetLayerAudible {
                comp: comp_id,
                layer: layer_id,
                audible: false,
            })
            .unwrap();
        assert!(
            !store
                .snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .audible
        );
        store.undo().unwrap();
        assert!(
            store
                .snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .audible
        );

        // Collapse round-trips the same way (defaults false).
        let clp = |s: &DocumentStore| {
            s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .collapse
        };
        store
            .commit(Op::SetLayerCollapse {
                comp: comp_id,
                layer: layer_id,
                collapse: true,
            })
            .unwrap();
        assert!(clp(&store));
        store.undo().unwrap();
        assert!(!clp(&store));

        // The effect stack + fx switch round-trip the same way.
        let stack = vec![crate::model::EffectInstance {
            id: Uuid::now_v7(),
            effect: crate::model::EffectKey {
                namespace: crate::model::EffectNamespace::Builtin,
                match_name: "glow".into(),
                version: 1,
                extra: serde_json::Map::new(),
            },
            enabled: true,
            params: Vec::new(),
            sample_temporally: true,
            extra: serde_json::Map::new(),
        }];
        store
            .commit(Op::SetLayerEffects {
                comp: comp_id,
                layer: layer_id,
                effects: stack.clone(),
            })
            .unwrap();
        let has_fx = |s: &DocumentStore| {
            !s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .effects
                .is_empty()
        };
        assert!(has_fx(&store));
        store.undo().unwrap();
        assert!(!has_fx(&store));
        store
            .commit(Op::SetLayerFx {
                comp: comp_id,
                layer: layer_id,
                fx: false,
            })
            .unwrap();
        store.undo().unwrap();
        assert!(
            store
                .snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .fx
        );

        // Visibility round-trips the same way (visible defaults true).
        let vis = |s: &DocumentStore| {
            s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .switches
                .visible
        };
        store
            .commit(Op::SetLayerVisible {
                comp: comp_id,
                layer: layer_id,
                visible: false,
            })
            .unwrap();
        assert!(!vis(&store));
        store.undo().unwrap();
        assert!(vis(&store));

        // Lock and label (K-168) round-trip the same way.
        let lock_label = |s: &DocumentStore| {
            let doc = s.snapshot();
            let l = doc
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .clone();
            (l.switches.locked, l.label)
        };
        store
            .commit(Op::SetLayerLocked {
                comp: comp_id,
                layer: layer_id,
                locked: true,
            })
            .unwrap();
        store
            .commit(Op::SetLayerLabel {
                comp: comp_id,
                layer: layer_id,
                label: 3,
            })
            .unwrap();
        assert_eq!(lock_label(&store), (true, 3));
        store.undo().unwrap();
        assert_eq!(lock_label(&store), (true, 0));
        store.undo().unwrap();
        assert_eq!(lock_label(&store), (false, 0));

        // Volume (docs/09 §6) round-trips like the transform properties.
        let vol = |s: &DocumentStore| {
            s.snapshot()
                .comp(comp_id)
                .unwrap()
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .unwrap()
                .volume_db
                .value_at(0.0)
        };
        store
            .commit(Op::SetLayerVolume {
                comp: comp_id,
                layer: layer_id,
                animation: Animation::Static(-12.0),
            })
            .unwrap();
        assert_eq!(vol(&store), -12.0);
        store.undo().unwrap();
        assert_eq!(vol(&store), 0.0, "default volume is unity (0 dB)");

        store.undo().unwrap();
        store.undo().unwrap();
        let doc = store.snapshot();
        let comp = doc.comp(comp_id).unwrap();
        assert_eq!(comp.camera_pose(1.0).unwrap().zoom, 1000.0);
        let layer = comp.layers.iter().find(|l| l.id == layer_id).unwrap();
        assert!(!layer.switches.three_d);

        // Zoom on a non-camera layer is an error, not a silent no-op.
        assert!(store
            .commit(Op::SetCameraZoom {
                comp: comp_id,
                layer: layer_id,
                animation: Animation::Static(1.0),
            })
            .is_err());
    }

    /// The asset-organisation ops behave: a batch is one undo step and
    /// all-or-nothing; folder children, auto-folder slots, comp settings and
    /// solid defs all round-trip exactly.
    #[test]
    fn batch_folder_and_settings_ops_round_trip() {
        use crate::model::{Folder, LinearColour, SolidDef};
        use crate::ops::AutoFolderKind;
        use crate::time::{Duration, FrameRate};
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        for op in ops {
            store.commit(op).unwrap();
        }

        // One batch: create the Solids folder, remember it, add a solid to it.
        let folder_id = uuid::Uuid::now_v7();
        let solid_id = uuid::Uuid::now_v7();
        let n_items = store.snapshot().items.len();
        store
            .commit(Op::Batch {
                ops: vec![
                    Op::AddItem {
                        index: n_items,
                        item: Box::new(ProjectItem::Folder(Folder {
                            id: folder_id,
                            name: "Solids".into(),
                            children: Vec::new(),
                            extra: serde_json::Map::new(),
                        })),
                    },
                    Op::SetAutoFolder {
                        kind: AutoFolderKind::Solids,
                        folder: Some(folder_id),
                    },
                    Op::AddItem {
                        index: n_items + 1,
                        item: Box::new(ProjectItem::Solid(SolidDef {
                            id: solid_id,
                            name: "White solid".into(),
                            colour: LinearColour([1.0, 1.0, 1.0, 1.0]),
                            width: 1920,
                            height: 1080,
                            extra: serde_json::Map::new(),
                        })),
                    },
                    Op::SetFolderChildren {
                        folder: folder_id,
                        children: vec![solid_id],
                    },
                ],
            })
            .unwrap();
        let doc = store.snapshot();
        assert_eq!(doc.auto_folders.solids, Some(folder_id));
        assert_eq!(doc.folder(folder_id).unwrap().children, vec![solid_id]);
        assert!(doc.solid(solid_id).is_some());
        assert!(!doc.root_items().contains(&solid_id), "filed, not root");

        // One undo removes the whole batch.
        store.undo().unwrap();
        let doc = store.snapshot();
        assert_eq!(doc.auto_folders.solids, None);
        assert!(doc.solid(solid_id).is_none());
        assert!(doc.folder(folder_id).is_none());
        store.redo().unwrap();

        // A failing member rolls back the whole batch.
        let before = store.snapshot();
        assert!(store
            .commit(Op::Batch {
                ops: vec![
                    Op::RenameItem {
                        id: folder_id,
                        name: "Renamed".into(),
                    },
                    Op::RemoveItem {
                        id: uuid::Uuid::now_v7(), // unknown: fails
                    },
                ],
            })
            .is_err());
        assert_eq!(*store.snapshot(), *before, "all-or-nothing");

        // Comp settings round-trip.
        store
            .commit(Op::SetCompSettings {
                comp: comp_id,
                name: "Retitled".into(),
                width: 1280,
                height: 720,
                frame_rate: FrameRate::new(24, 1).unwrap(),
                duration: Duration(Rational::new(5, 1).unwrap()),
                background: LinearColour([0.1, 0.1, 0.1, 1.0]),
            })
            .unwrap();
        let doc = store.snapshot();
        let comp = doc.comp(comp_id).unwrap();
        assert_eq!((comp.width, comp.height), (1280, 720));
        assert_eq!(comp.name, "Retitled");
        store.undo().unwrap();
        let comp2 = store.snapshot();
        let comp2 = comp2.comp(comp_id).unwrap();
        assert_eq!((comp2.width, comp2.height), (1920, 1080));

        // Solid def edit round-trips and errors on non-solid targets.
        store
            .commit(Op::SetSolidDef {
                def: solid_id,
                name: "Grey solid".into(),
                colour: LinearColour([0.5, 0.5, 0.5, 1.0]),
                width: 640,
                height: 480,
            })
            .unwrap();
        assert_eq!(store.snapshot().solid(solid_id).unwrap().width, 640);
        store.undo().unwrap();
        assert_eq!(store.snapshot().solid(solid_id).unwrap().width, 1920);
        assert!(store
            .commit(Op::SetSolidDef {
                def: comp_id,
                name: "x".into(),
                colour: LinearColour([0.0, 0.0, 0.0, 1.0]),
                width: 1,
                height: 1,
            })
            .is_err());
    }

    #[test]
    fn matte_op_round_trips_and_targets_any_layer() {
        use crate::model::{LayerInputSource, MatteChannel, MatteRef};
        let store = DocumentStore::new(Document::new());
        let (ops, comp_id) = scripted_ops(&store.snapshot());
        let mut layer_id = None;
        for op in &ops {
            if let Op::AddLayer { layer, .. } = op {
                layer_id = Some(layer.id);
            }
        }
        for op in ops {
            store.commit(op).unwrap();
        }
        let layer_id = layer_id.unwrap();
        // A second layer to serve as the matte source.
        let matte_layer = test_layer(Uuid::now_v7());
        let matte_id = matte_layer.id;
        store
            .commit(Op::AddLayer {
                comp: comp_id,
                index: 0,
                layer: Box::new(matte_layer),
            })
            .unwrap();

        let matte = MatteRef {
            layer: matte_id,
            channel: MatteChannel::Luma,
            inverted: true,
            source: LayerInputSource::None,
        };
        store
            .commit(Op::SetLayerMatte {
                comp: comp_id,
                layer: layer_id,
                matte: Some(matte),
            })
            .unwrap();
        let doc = store.snapshot();
        let l = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .unwrap();
        assert_eq!(l.matte, Some(matte));

        store.undo().unwrap();
        let doc = store.snapshot();
        let l = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .unwrap();
        assert_eq!(l.matte, None, "undo clears the matte exactly");
    }

    #[test]
    fn layers_saved_before_transforms_existed_still_load() {
        // A pre-transform Layer JSON (as slice-3 Lumit wrote it).
        let old = r#"{
            "id": "018f0e9a-0000-7000-8000-000000000001",
            "name": "clip.mp4",
            "kind": { "Footage": { "item": "018f0e9a-0000-7000-8000-000000000002" } },
            "in_point": [0, 1],
            "out_point": [10, 1],
            "start_offset": [0, 1],
            "switches": { "visible": true, "audible": true, "locked": false }
        }"#;
        let layer: crate::model::Layer = serde_json::from_str(old).unwrap();
        assert_eq!(layer.transform.opacity.value_at(0.0), 100.0);
        assert_eq!(layer.transform.scale_x.value_at(0.0), 100.0);
    }

    #[test]
    fn invalid_ops_leave_document_untouched() {
        let store = DocumentStore::new(Document::new());
        let before = json(&store.snapshot());
        let bogus = Op::RemoveItem { id: Uuid::now_v7() };
        assert!(store.commit(bogus).is_err());
        assert_eq!(json(&store.snapshot()), before);
        assert!(!store.can_undo());
    }
}
