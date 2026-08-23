//! The solve link: a Camera layer driven by a tracked layer's camera solve
//! (K-417, docs/03-DATA-MODEL.md §5.6, docs/impl/tracking.md phase 4).
//!
//! # In plain terms
//!
//! When a shot has been camera-tracked, the engine knows where the real camera
//! was on every frame of that *file*. The obvious thing to do with that is to
//! stamp it onto a Camera layer as several thousand keyframes. Lumit does not,
//! at least not at first: the Camera layer **points at** the tracked layer and
//! works out its placement afresh each frame. Re-solve the shot, trim it,
//! reorder its cuts, ramp its speed — the camera follows, because nothing was
//! ever copied.
//!
//! Working out "which moment of the file is on screen right now" is the whole
//! of the mechanism, and it is the same walk the renderer already does to
//! decide which frame of the file to decode: composition time, less the
//! layer's offset, through the layer's or the clip's Retime, to a moment of the
//! source. That is why a reordered Sequence layer and a speed ramp both come
//! out right for free — the tracker ran on the file, once, exactly as K-248
//! says it must.
//!
//! Two honest failures, and neither is silent (K-417):
//!
//! - The link asks for a moment **outside** what was solved (the layer runs on
//!   past the solved range, or a retime reaches before its start). The camera
//!   **holds** the nearest solved frame — the last derived motion — and the
//!   reading says [`LinkState::Held`].
//! - The link cannot be followed at all: the layer was deleted, its media is
//!   offline, or nothing has been solved for it. The camera falls back to the
//!   properties the document itself holds — which, since a linked camera's
//!   transform is read-only, are the ones it had when the link was made — and
//!   the reading says [`LinkState::Unresolved`]. Never a freeze nobody
//!   mentioned, never a crash.
//!
//! **The store is a trait, not a thing.** What actually holds the solves is the
//! project's `track/` sidecar, and it arrives in stage 2. Everything here talks
//! to [`CameraSolveStore`], so the model half is complete and testable now with
//! a written-down solve standing in for a real one.

use uuid::Uuid;

use crate::anim::{Animation, Keyframe, SideInterp};
use crate::model::{stored_camera_pose, CameraPose, Composition, Document, Layer, LayerKind};
use crate::ops::Op;
use crate::sequence::ClipSource;
use crate::time::Rational;

/// The match name of the Camera track effect — what marks a layer as *the*
/// tracked layer inside a precomp (K-417's parent-comp ruling).
///
/// One spelling, here, because two places compare against it: the walk below
/// and the effect's own declaration
/// ([`crate::fx::effects::camera_track`](crate::fx::effects::camera_track)).
pub const CAMERA_TRACK: &str = "camera_track";

/// What was solved for one piece of media: the frame rate its solved frames are
/// numbered at, and the range that actually has poses.
///
/// Frames rather than seconds because a solve is per *frame* — one pose per
/// decoded picture — and rounding a source time to a frame is the one place
/// that conversion should happen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolvedRange {
    /// The rate the solved frames are numbered at (the media's own rate).
    pub fps: f64,
    /// First solved frame, inclusive.
    pub first_frame: i64,
    /// Last solved frame, inclusive.
    pub last_frame: i64,
}

/// Where the solved camera paths live, as far as the model is concerned.
///
/// The real implementation is the project's `track/` sidecar and arrives in
/// stage 2; tests inject a written-down one. Deliberately two small methods
/// rather than one that hands back a whole solve: the model needs the range to
/// decide whether it is holding, and one pose to place the camera, and nothing
/// else — a trait that handed over the point cloud as well would be a trait
/// the model could accidentally start depending on.
///
/// **The poses are already in Lumit's camera terms**: comp pixels, AE-style
/// position and rotation, exactly what [`CameraPose`] means everywhere else.
/// Turning `lumit_track::CameraSolve`'s world-to-camera rotations, its solve
/// units and its focal in source pixels into these is the *store's* job, and it
/// is stage 2's — putting it here would make the model depend on the tracker,
/// which the crate graph forbids in that direction (docs/05).
pub trait CameraSolveStore {
    /// What has been solved for the footage item `media`, or `None` when
    /// nothing has (never analysed, media offline, sidecar rebuilt away).
    fn solved_range(&self, media: Uuid) -> Option<SolvedRange>;

    /// The solved camera at source frame `frame` of `media`. `None` for a frame
    /// outside the solved range, which callers here never ask for — they clamp
    /// first, because clamping *is* the hold.
    fn solved_pose(&self, media: Uuid, frame: i64) -> Option<CameraPose>;
}

/// How a Camera layer's placement was arrived at — the flag K-417 requires the
/// interface to be able to read, so a link that stopped working says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// No solve link: an ordinary camera, driven by its own properties.
    Unlinked,
    /// Derived from the linked layer's solve, at the frame it asked for.
    Derived,
    /// Derived, but from the nearest solved frame rather than the one asked
    /// for — the layer has run past the solved range, or a retime reached
    /// before its start. The last derived motion, held.
    Held,
    /// The link could not be followed at all (layer gone, media offline,
    /// nothing solved). The camera's own stored properties are in play, and the
    /// interface says so rather than showing a freeze nobody explained.
    Unresolved,
}

/// A camera's placement plus how it was arrived at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkedPose {
    pub pose: CameraPose,
    pub state: LinkState,
}

/// How many precomp hops the walk will make before giving up. A cycle is
/// already an invalid document (docs/03 §5.2) and is guarded at insertion; this
/// is the belt to that brace, so a malformed file degrades to `Unresolved`
/// rather than recursing until the stack ends (14-ENGINEERING-RULES §4).
const MAX_NESTING: u32 = 16;

/// The **active** camera's placement at comp time `t`, following its solve link
/// where it has one. `None` → the comp renders flat, exactly as
/// [`Composition::camera_pose`] means it.
///
/// This is the reading the render path wants once solves exist; today's callers
/// use `camera_pose`, which is this with no store and therefore
/// [`LinkState::Unresolved`] on any linked camera — the correct degrade, and
/// what stage 2 replaces by threading the real store.
#[must_use]
pub fn camera_pose_at(
    doc: &Document,
    comp: &Composition,
    t: f64,
    store: &dyn CameraSolveStore,
) -> Option<LinkedPose> {
    let layer = comp.active_camera(t)?;
    camera_pose_of(doc, comp, layer, t, store)
}

/// One named Camera layer's placement at comp time `t`, whether or not it is
/// the active one — what the bake walks, frame by frame.
#[must_use]
pub fn camera_pose_of(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    t: f64,
    store: &dyn CameraSolveStore,
) -> Option<LinkedPose> {
    let stored = stored_camera_pose(layer, t)?;
    let LayerKind::Camera { solve_link, .. } = &layer.kind else {
        return None;
    };
    let Some(tracked) = *solve_link else {
        return Some(LinkedPose {
            pose: stored,
            state: LinkState::Unlinked,
        });
    };
    match derived_pose(doc, comp, tracked, t, store, 0) {
        Some((pose, held)) => Some(LinkedPose {
            pose,
            state: if held {
                LinkState::Held
            } else {
                LinkState::Derived
            },
        }),
        None => Some(LinkedPose {
            pose: stored,
            state: LinkState::Unresolved,
        }),
    }
}

/// Which solved frame of which media the tracked layer `tracked` is showing at
/// comp time `t`, clamped into what was actually solved — and whether the clamp
/// bit, which is what [`LinkState::Held`] means.
///
/// The one walk everything downstream shares: the camera link asks it for a
/// pose, and the interface asks it which frame of the point cloud to draw
/// (K-417's overlay). Two readings of the same moment cannot disagree if there
/// is only one walk.
#[must_use]
pub fn tracked_solved_frame(
    doc: &Document,
    comp: &Composition,
    tracked: Uuid,
    t: f64,
    store: &dyn CameraSolveStore,
) -> Option<(Uuid, i64, bool)> {
    solved_frame(doc, comp, tracked, t, store, 0)
}

/// [`tracked_solved_frame`] with the precomp depth the walk is already at.
fn solved_frame(
    doc: &Document,
    comp: &Composition,
    tracked: Uuid,
    t: f64,
    store: &dyn CameraSolveStore,
    depth: u32,
) -> Option<(Uuid, i64, bool)> {
    let (media, st) = tracked_source_at(doc, comp, tracked, t, depth)?;
    if !st.is_finite() {
        return None;
    }
    let range = store.solved_range(media)?;
    if !range.fps.is_finite() || range.fps <= 0.0 || range.last_frame < range.first_frame {
        return None;
    }
    let asked = (st * range.fps).round();
    // Past `i64`'s reach is past any solve's reach, and saturating is the same
    // answer as clamping would have been.
    #[allow(clippy::cast_possible_truncation)]
    let asked = asked.clamp(i64::MIN as f64, i64::MAX as f64) as i64;
    let frame = asked.clamp(range.first_frame, range.last_frame);
    Some((media, frame, frame != asked))
}

/// The solved pose the link comes to at comp time `t`, and whether it was
/// **held** (clamped into the solved range) to get there.
fn derived_pose(
    doc: &Document,
    comp: &Composition,
    tracked: Uuid,
    t: f64,
    store: &dyn CameraSolveStore,
    depth: u32,
) -> Option<(CameraPose, bool)> {
    let (media, frame, held) = solved_frame(doc, comp, tracked, t, store, depth)?;
    let pose = store.solved_pose(media, frame)?;
    Some((pose, held))
}

/// Walk comp time `t` down `layer`'s time chain to `(footage item, source
/// time)` — the one question the link asks, and the same mapping the renderer
/// uses to choose which frame to decode.
///
/// Through a **Precomp** layer (or a comp-sourced clip) the walk continues
/// inside: the nested comp's own tracked layer is found by the Camera track
/// effect on it, which is what makes K-417's parent-comp workflow resolve — a
/// linked camera in the parent comp names the precomp layer, and the chain
/// finds the footage inside.
///
/// `None` for a layer that has no source to track (a solid, a null, a camera),
/// a gap in a Sequence layer, a precomp naming a comp that is gone, or a
/// precomp with no tracked layer in it.
fn tracked_source_at(
    doc: &Document,
    comp: &Composition,
    layer: Uuid,
    t: f64,
    depth: u32,
) -> Option<(Uuid, f64)> {
    if depth > MAX_NESTING {
        return None;
    }
    let l = comp.layers.iter().find(|l| l.id == layer)?;
    let lt = crate::time::layer_time(t, l.start_offset.0);
    match &l.kind {
        LayerKind::Footage { item } => Some((*item, l.source_time_at(lt))),
        LayerKind::Precomp { comp: nested } => descend(doc, *nested, l.source_time_at(lt), depth),
        LayerKind::Sequence { clips } => {
            // The clip under the playhead and the moment of its source it
            // shows — trimmed extent and retime included, which is exactly why
            // reordering the cuts or ramping one of them changes nothing about
            // which solved frame comes back.
            let (_, source, st) = crate::sequence::resolve(clips, lt)?;
            match source {
                ClipSource::Footage(item) => Some((item, st)),
                ClipSource::Comp(nested) => descend(doc, nested, st, depth),
            }
        }
        _ => None,
    }
}

/// One hop into a nested composition at *its* comp time `t`, continuing from
/// the tracked layer inside it.
fn descend(doc: &Document, nested: Uuid, t: f64, depth: u32) -> Option<(Uuid, f64)> {
    let nc = doc.comp(nested)?;
    let inner = tracked_layer(nc)?;
    tracked_source_at(doc, nc, inner, t, depth + 1)
}

/// The layer in `comp` carrying an enabled Camera track effect — the tracked
/// layer, by K-417's definition that the effect *is* the handle. The first one,
/// in stack order, so the answer never depends on the playhead; `None` when the
/// comp has none.
#[must_use]
pub fn tracked_layer(comp: &Composition) -> Option<Uuid> {
    comp.layers
        .iter()
        .find(|l| {
            l.effects
                .iter()
                .any(|e| e.enabled && e.effect.match_name == CAMERA_TRACK)
        })
        .map(|l| l.id)
}

/// **Convert to keyframes** (K-417): bake the derived motion into ordinary
/// keyframes and sever the link, as one undoable step.
///
/// One key per composition frame across the camera layer's own span, linear on
/// both sides, on the six transform properties and on the camera's zoom — real,
/// editable keyframes the graph editor draws like any others, which is the
/// bargain the ruling makes for being honest that there are a lot of them.
///
/// **The link is cleared first**, inside the batch, because a linked camera's
/// transform is read-only ([`crate::ops::OpError::CameraLinked`]) and a batch
/// is applied in order. Undo reverses the members, so the link is restored last
/// — after the transforms are back — and the refusal never trips on the way out
/// either.
///
/// Deterministic: the frames come from the comp's own rate, the poses from the
/// same [`camera_pose_of`] the render reads, and a frame the link cannot
/// resolve bakes the same fallback the render would have shown. `None` when
/// `camera` is not a Camera layer in `comp`, when it carries no link, or when
/// its span holds no frames.
#[must_use]
pub fn bake_solve_link(
    doc: &Document,
    comp: Uuid,
    camera: Uuid,
    store: &dyn CameraSolveStore,
) -> Option<Op> {
    use crate::model::TransformProp;

    let c = doc.comp(comp)?;
    let layer = c.layers.iter().find(|l| l.id == camera)?;
    let LayerKind::Camera { solve_link, .. } = &layer.kind else {
        return None;
    };
    solve_link.as_ref()?;

    let rate = c.frame_rate;
    let first = rate.frame_at(layer.in_point);
    let last = rate.frame_at(layer.out_point);
    if last <= first {
        return None;
    }

    // Six transform tracks plus the zoom, filled in one walk of the frames so
    // the derived pose is asked for once per frame rather than seven times.
    let mut keys: [Vec<Keyframe>; 7] = Default::default();
    for n in first..last {
        let ct = rate.time_of_frame(n).ok()?;
        let pose = camera_pose_of(doc, c, layer, ct.0.to_f64(), store)?.pose;
        // Keyframe times are LAYER time, which is what every property is
        // evaluated at — the same subtraction `stored_camera_pose` makes.
        let time = ct.0.checked_sub(layer.start_offset.0).ok()?;
        for (slot, value) in keys.iter_mut().zip([
            pose.position.0,
            pose.position.1,
            pose.position.2,
            pose.rotation_deg.0,
            pose.rotation_deg.1,
            pose.rotation_deg.2,
            pose.zoom,
        ]) {
            slot.push(key(time, value));
        }
    }

    let props = [
        TransformProp::PositionX,
        TransformProp::PositionY,
        TransformProp::PositionZ,
        TransformProp::RotationX,
        TransformProp::RotationY,
        TransformProp::Rotation,
    ];
    let mut ops = Vec::with_capacity(props.len() + 2);
    // First, so the read-only refusal below does not stop the very edit that
    // ends the link.
    ops.push(Op::SetCameraSolveLink {
        comp,
        layer: camera,
        solve_link: None,
    });
    let mut track = keys.into_iter();
    for prop in props {
        ops.push(Op::SetTransformProperty {
            comp,
            layer: camera,
            prop,
            animation: Animation::Keyframed(track.next()?),
        });
    }
    ops.push(Op::SetCameraZoom {
        comp,
        layer: camera,
        animation: Animation::Keyframed(track.next()?),
    });
    Some(Op::Batch { ops })
}

/// One baked key: linear on both sides, because the samples are one per frame
/// and there is nothing between two of them to shape.
fn key(time: Rational, value: f64) -> Keyframe {
    Keyframe {
        time,
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::anim::Property;
    use crate::model::{
        BlendMode, CameraPose, Composition, LinearColour, ProjectItem, Switches, TransformGroup,
        TransformProp,
    };
    use crate::ops::{apply, OpError};
    use crate::sequence::Clip;
    use crate::time::{CompTime, Duration, FrameRate};

    /// A written-down solve standing in for a real one: sixty frames at 30 fps,
    /// with the pose a plain function of the frame so a test can say what it
    /// expects without carrying a table.
    ///
    /// The point of stage 1 is that this is *all* the model needs, and it
    /// arrives through the same trait the sidecar will.
    struct Synthetic {
        media: Uuid,
    }

    impl Synthetic {
        const FPS: f64 = 30.0;
        const FIRST: i64 = 0;
        const LAST: i64 = 59;

        fn new(media: Uuid) -> Self {
            Self { media }
        }

        /// The pose at solved frame `n` — distinct in every field per frame, so
        /// a test that lands on the wrong frame cannot accidentally pass.
        fn pose(n: i64) -> CameraPose {
            let f = n as f64;
            CameraPose {
                zoom: 1000.0 + f,
                position: (f, f * 2.0, f * 3.0),
                rotation_deg: (f * 0.5, f * 0.25, f * 0.125),
            }
        }
    }

    impl CameraSolveStore for Synthetic {
        fn solved_range(&self, media: Uuid) -> Option<SolvedRange> {
            (media == self.media).then_some(SolvedRange {
                fps: Self::FPS,
                first_frame: Self::FIRST,
                last_frame: Self::LAST,
            })
        }

        fn solved_pose(&self, media: Uuid, frame: i64) -> Option<CameraPose> {
            (media == self.media && (Self::FIRST..=Self::LAST).contains(&frame))
                .then(|| Synthetic::pose(frame))
        }
    }

    /// A store with nothing in it — never analysed, or the media offline.
    struct Empty;

    impl CameraSolveStore for Empty {
        fn solved_range(&self, _: Uuid) -> Option<SolvedRange> {
            None
        }
        fn solved_pose(&self, _: Uuid, _: i64) -> Option<CameraPose> {
            None
        }
    }

    fn secs(n: i64) -> CompTime {
        CompTime(Rational::new(n, 1).unwrap())
    }

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    /// A Retime map from `(layer time, source time)` pairs, linear between them.
    fn retime(points: &[(i64, f64)]) -> Property {
        Property {
            animation: Animation::Keyframed(
                points
                    .iter()
                    .map(|(t, v)| super::key(rat(*t, 30), *v))
                    .collect(),
            ),
            extra: serde_json::Map::new(),
        }
    }

    fn layer(name: &str, kind: LayerKind, out_s: i64) -> Layer {
        Layer {
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: name.into(),
            kind,
            in_point: secs(0),
            out_point: secs(out_s),
            start_offset: secs(0),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: Property::zero(),
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// A tracked layer is one carrying the Camera track effect — that is the
    /// whole of what marks it (K-417).
    fn tracked(mut l: Layer) -> Layer {
        l.effects
            .push(crate::fx::instantiate(CAMERA_TRACK).expect("the effect is registered"));
        l
    }

    fn camera(link: Option<Uuid>) -> Layer {
        layer(
            "Camera 1",
            LayerKind::Camera {
                zoom: Property::fixed(777.0),
                solve_link: link,
            },
            4,
        )
    }

    fn comp(name: &str, layers: Vec<Layer>) -> Composition {
        Composition {
            id: Uuid::now_v7(),
            name: name.into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(rat(4, 1)),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn document(comps: Vec<Composition>) -> Document {
        let mut doc = Document::new();
        for c in comps {
            doc.items.push(ProjectItem::Composition(c));
        }
        doc
    }

    /// The plain case: a footage layer at source rate, so comp frame `n` is
    /// solved frame `n` and the camera wears the solve exactly.
    #[test]
    fn a_link_through_a_plain_clip_reads_the_frame_under_the_playhead() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let doc = document(vec![c.clone()]);
        let store = Synthetic::new(media);

        for n in 0..60 {
            let got = camera_pose_at(&doc, &c, f64::from(n) / 30.0, &store).unwrap();
            assert_eq!(got.state, LinkState::Derived, "frame {n}");
            assert_eq!(got.pose, Synthetic::pose(i64::from(n)), "frame {n}");
        }
    }

    /// A retimed clip, freeze included. The mapping is the layer's own Retime —
    /// the same one the renderer decodes through — so the camera sees exactly
    /// the frame the picture shows.
    #[test]
    fn a_link_through_a_retimed_clip_follows_the_retime() {
        let media = Uuid::now_v7();
        let mut footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        // Half speed for the first second, then frozen on source 0.5 s.
        footage.retime = Some(retime(&[(0, 0.0), (30, 0.5), (60, 0.5)]));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let doc = document(vec![c.clone()]);
        let store = Synthetic::new(media);

        // Half speed: comp frame 10 is 1/3 s, source 1/6 s, solved frame 5.
        let got = camera_pose_at(&doc, &c, 10.0 / 30.0, &store).unwrap();
        assert_eq!(got.state, LinkState::Derived);
        assert_eq!(got.pose, Synthetic::pose(5));

        // The freeze: every moment of the second half reads source 0.5 s,
        // solved frame 15 — and it is a *derived* read, not a held one. The
        // solve has that frame; the retime simply keeps asking for it.
        for n in [31, 40, 59] {
            let got = camera_pose_at(&doc, &c, f64::from(n) / 30.0, &store).unwrap();
            assert_eq!(got.state, LinkState::Derived, "frame {n}");
            assert_eq!(got.pose, Synthetic::pose(15), "frame {n}");
        }
    }

    /// K-248 made testable: the tracker ran on the file, so reordering the cuts
    /// changes which solved frame is on screen and nothing else. The same source
    /// moment gives the same pose whichever order the clips sit in.
    #[test]
    fn reordering_a_sequence_layer_moves_the_pose_with_the_source_frame() {
        let media = Uuid::now_v7();
        let source = ClipSource::Footage(media);
        // Two half-second clips: one showing source 0.0–0.5, one showing
        // 1.0–1.5. `ab` plays them in that order, `ba` the other way round.
        let clip = |source_in: i64, place: i64| {
            Clip::new(
                source,
                rat(source_in, 30),
                rat(source_in + 15, 30),
                rat(place, 30),
                rat(15, 30),
            )
        };
        let ab = vec![clip(0, 0), clip(30, 15)];
        let ba = vec![clip(30, 0), clip(0, 15)];

        for (clips, first_half, second_half) in [(ab, 0, 30), (ba, 30, 0)] {
            let seq = tracked(layer("cut", LayerKind::Sequence { clips }, 1));
            let c = comp("main", vec![camera(Some(seq.id)), seq]);
            let doc = document(vec![c.clone()]);
            let store = Synthetic::new(media);

            for n in 0..15 {
                let got = camera_pose_at(&doc, &c, f64::from(n) / 30.0, &store).unwrap();
                assert_eq!(got.pose, Synthetic::pose(first_half + i64::from(n)));
            }
            for n in 15..30 {
                let got = camera_pose_at(&doc, &c, f64::from(n) / 30.0, &store).unwrap();
                assert_eq!(got.pose, Synthetic::pose(second_half + i64::from(n) - 15));
            }
        }
    }

    /// K-417's parent-comp ruling: a linked camera in the parent comp names the
    /// **precomp layer**, and the chain resolves through it to the tracked layer
    /// inside — the precomp layer's own Retime included.
    #[test]
    fn a_link_resolves_through_a_precomp_to_the_tracked_layer_inside() {
        let media = Uuid::now_v7();
        let inner = comp(
            "inner",
            vec![tracked(layer(
                "shot",
                LayerKind::Footage { item: media },
                2,
            ))],
        );

        let mut nested = layer("inner", LayerKind::Precomp { comp: inner.id }, 2);
        // The precomp layer plays at half speed, so comp frame 20 shows the
        // nested comp's 1/3 s, which is the tracked layer's solved frame 10.
        nested.retime = Some(retime(&[(0, 0.0), (60, 1.0)]));
        let outer = comp("outer", vec![camera(Some(nested.id)), nested]);
        let doc = document(vec![inner, outer.clone()]);
        let store = Synthetic::new(media);

        let got = camera_pose_at(&doc, &outer, 20.0 / 30.0, &store).unwrap();
        assert_eq!(got.state, LinkState::Derived);
        assert_eq!(got.pose, Synthetic::pose(10));

        // A precomp with no tracked layer in it has nothing to resolve to, and
        // says so rather than guessing at the first footage it finds.
        let bare = comp(
            "bare",
            vec![layer("plain", LayerKind::Footage { item: media }, 2)],
        );
        let lone = layer("bare", LayerKind::Precomp { comp: bare.id }, 2);
        let outer = comp("outer", vec![camera(Some(lone.id)), lone]);
        let doc = document(vec![bare, outer.clone()]);
        assert_eq!(
            camera_pose_at(&doc, &outer, 0.0, &store).unwrap().state,
            LinkState::Unresolved
        );
    }

    /// The two honest failures, and neither is silent: past the end of the solve
    /// the last derived motion **holds**, and a link that cannot be followed at
    /// all falls back to the camera's own properties and says which it is.
    #[test]
    fn an_unresolvable_link_holds_and_says_which_kind_of_unresolvable() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 4));
        let footage_id = footage.id;
        let c = comp("main", vec![camera(Some(footage_id)), footage]);
        let doc = document(vec![c.clone()]);

        // Three seconds in is past the sixty solved frames: the last one holds.
        let got = camera_pose_at(&doc, &c, 3.0, &Synthetic::new(media)).unwrap();
        assert_eq!(got.state, LinkState::Held);
        assert_eq!(got.pose, Synthetic::pose(59));

        // Nothing solved at all: the document's own numbers, flagged.
        let got = camera_pose_at(&doc, &c, 0.0, &Empty).unwrap();
        assert_eq!(got.state, LinkState::Unresolved);
        assert_eq!(got.pose.zoom, 777.0);

        // The linked layer deleted: the same answer, for the same reason.
        let mut orphan = c.clone();
        orphan.layers.retain(|l| l.id != footage_id);
        let doc = document(vec![orphan.clone()]);
        assert_eq!(
            camera_pose_at(&doc, &orphan, 0.0, &Synthetic::new(media))
                .unwrap()
                .state,
            LinkState::Unresolved
        );

        // A camera with no link is none of those.
        let plain = comp("plain", vec![camera(None)]);
        assert_eq!(
            camera_pose_at(&document(vec![plain.clone()]), &plain, 0.0, &Empty)
                .unwrap()
                .state,
            LinkState::Unlinked
        );
    }

    /// While the link is set, the camera's placement is not the document's to
    /// edit (K-417). The refusal is typed, and it is in `apply` rather than in
    /// the interface, so a preset or an expression cannot go round it.
    #[test]
    fn a_linked_cameras_transform_is_read_only() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let (comp_id, cam_id) = (c.id, c.layers[0].id);
        let mut doc = document(vec![c]);

        let moved = Op::SetTransformProperty {
            comp: comp_id,
            layer: cam_id,
            prop: TransformProp::PositionX,
            animation: Animation::Static(5.0),
        };
        assert_eq!(apply(&mut doc, &moved), Err(OpError::CameraLinked));
        assert_eq!(
            apply(
                &mut doc,
                &Op::SetCameraZoom {
                    comp: comp_id,
                    layer: cam_id,
                    animation: Animation::Static(5.0),
                }
            ),
            Err(OpError::CameraLinked)
        );

        // Clearing the link is always allowed — a link that could not be undone
        // would be a trap — and afterwards the camera is ordinary again.
        apply(
            &mut doc,
            &Op::SetCameraSolveLink {
                comp: comp_id,
                layer: cam_id,
                solve_link: None,
            },
        )
        .expect("clearing a link is never refused");
        assert!(apply(&mut doc, &moved).is_ok());
    }

    /// **Convert to keyframes**: the baked keys reproduce the derived path
    /// exactly, at every comp frame of the layer's span, and the link is gone.
    /// One undo step puts both back.
    #[test]
    fn the_bake_reproduces_the_derived_path_and_undo_restores_the_link() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 4));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let (comp_id, cam_id) = (c.id, c.layers[0].id);
        let mut doc = document(vec![c]);
        let store = Synthetic::new(media);
        let before = doc.comp(comp_id).unwrap().layers[0].clone();

        // What the link says, frame by frame, before anything is baked. The
        // camera's span is 0..4 s at 30 fps, so 120 frames — sixty derived and
        // sixty held, which means the bake is checked on both readings.
        let want: Vec<CameraPose> = (0..120)
            .map(|n| {
                let c = doc.comp(comp_id).unwrap();
                camera_pose_at(&doc, c, f64::from(n) / 30.0, &store)
                    .unwrap()
                    .pose
            })
            .collect();
        // The comparison below is against the derived path itself, so it would
        // pass just as happily on a path that never resolved and never moved.
        // These two say the path under test is the real one: sixty distinct
        // solved poses, then sixty held on the last of them.
        assert_eq!(want[..60], (0..60).map(Synthetic::pose).collect::<Vec<_>>());
        assert!(want[60..].iter().all(|p| *p == Synthetic::pose(59)));

        let bake = bake_solve_link(&doc, comp_id, cam_id, &store).expect("there is a link to bake");
        assert!(
            matches!(&bake, Op::Batch { ops } if ops.len() == 8),
            "one batch: the link, six transform tracks and the zoom"
        );
        // One undo step, not a hundred and twenty.
        let undo = apply(&mut doc, &bake).expect("the bake applies");

        let baked = doc.comp(comp_id).unwrap();
        assert!(
            matches!(
                baked.layers[0].kind,
                LayerKind::Camera {
                    solve_link: None,
                    ..
                }
            ),
            "the bake severs the link"
        );
        for (n, expected) in want.iter().enumerate() {
            assert_eq!(
                baked.camera_pose(n as f64 / 30.0).as_ref(),
                Some(expected),
                "baked frame {n} does not match the derived path"
            );
        }
        // Undo restores the link — and restores it *last*, after the transforms
        // are home, so the read-only refusal does not trip on the way out
        // either. One step, and the layer is byte-for-byte what it was.
        apply(&mut doc, &undo).expect("the undo applies");
        assert_eq!(
            doc.comp(comp_id).unwrap().layers[0],
            before,
            "undo puts the link and the properties back exactly"
        );
    }

    /// The bake is deterministic, and refuses what it cannot do.
    #[test]
    fn the_bake_is_deterministic_and_refuses_an_unlinked_camera() {
        let media = Uuid::now_v7();
        let footage = tracked(layer("shot", LayerKind::Footage { item: media }, 2));
        let c = comp("main", vec![camera(Some(footage.id)), footage]);
        let (comp_id, cam_id) = (c.id, c.layers[0].id);
        let doc = document(vec![c]);
        let store = Synthetic::new(media);

        let a = bake_solve_link(&doc, comp_id, cam_id, &store).unwrap();
        let b = bake_solve_link(&doc, comp_id, cam_id, &store).unwrap();
        assert_eq!(a, b, "two bakes of one document differ");

        let plain = comp("plain", vec![camera(None)]);
        let (plain_id, plain_cam) = (plain.id, plain.layers[0].id);
        assert!(
            bake_solve_link(&document(vec![plain]), plain_id, plain_cam, &store).is_none(),
            "there is nothing to bake without a link"
        );
    }
}
