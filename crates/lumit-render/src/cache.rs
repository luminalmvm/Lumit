//! Naming a finished frame, so it need never be rendered twice.
//!
//! # In plain terms
//!
//! Rendering a comp frame is the most expensive thing the application does, and
//! scrubbing back and forth over the same few frames should not pay for it every
//! time. The trick is to give each finished frame an honest **name**: a hash of
//! everything that went into it — every layer's transform, effects, masks, blend
//! and switches, which file each footage layer reads and which frame of it, plus
//! the resolution tier. Two frames with the same name are guaranteed to be the
//! same picture, so one can stand in for the other (docs/06 §5.2; cache entries
//! are keyed by content, never by timeline position).
//!
//! Naming by content is what makes editing feel light. Renaming a layer, moving
//! the work area or selecting something changes the document but not the
//! picture, so every cached frame survives. Nudging one layer's position
//! invalidates only the frames that layer appears in. A cruder scheme — "throw
//! everything away whenever the document changes" — re-renders the whole comp
//! after every keystroke, which is exactly the behaviour this replaces.
//!
//! A frame is only nameable once all its footage is probed: until then the
//! pipeline does not know which source frame a layer will show, so it renders
//! live and banks nothing ([`frame_key`] answers `None`).

use crate::plan::Quality;
use crate::source::{SourceProbe, SourceProbes};
use lumit_core::model::{Composition, Document, ProjectItem};
use uuid::Uuid;

/// A frame's cache-bar tier (docs/06 §5.6): green plays now, blue promotes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CacheTier {
    None,
    /// In RAM at current quality — plays in real time now (green).
    Ram,
    /// On disk only — promotable, not yet playable (blue).
    Disk,
}

/// One display-ready comp frame in the RAM tier (sRGB bytes as shown and as
/// exported — the same pixels, K-031).
pub struct CachedCompFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl lumit_cache::ByteSized for CachedCompFrame {
    fn byte_size(&self) -> usize {
        self.rgba.len() + 16
    }
}

/// Supplies the content identity of footage pixels to [`lumit_eval::
/// comp_frame_key`]: which source, and which source frame, a Footage layer shows
/// at layer time `lt`. Built over any frontend's probe cache.
pub struct Stamper<'a> {
    doc: &'a Document,
    probes: &'a dyn SourceProbes,
    quality: Quality,
}

impl<'a> Stamper<'a> {
    #[must_use]
    pub fn new(doc: &'a Document, probes: &'a dyn SourceProbes, quality: Quality) -> Self {
        Self {
            doc,
            probes,
            quality,
        }
    }
}

impl lumit_eval::SourceStamper for Stamper<'_> {
    fn source_fps(&self, item: Uuid) -> Option<f64> {
        self.probes.probe(item).video().map(|(fps, ..)| fps)
    }

    /// The camera the picture is actually drawn with — the solve link followed
    /// (K-417), through the same store [`crate::build`] and [`crate::headless`]
    /// read. Without this a linked camera's frames would be named by the
    /// transform the document happens to hold, which is not the transform they
    /// were drawn with, and a solve landing would hand back every frame made
    /// before it.
    fn camera(
        &self,
        doc: &Document,
        comp: &lumit_core::model::Composition,
        t: f64,
    ) -> Option<lumit_core::model::CameraPose> {
        crate::track::camera_pose(doc, comp, t)
    }

    fn stamp(&self, item: Uuid, lt: f64, native: bool) -> Option<(String, u64)> {
        let Some(ProjectItem::Footage(f)) = self.doc.item(item) else {
            return None;
        };
        let probe = self.probes.probe(item);
        // Missing media renders the slate (docs/07 §3.3), which is perfectly
        // cacheable: it is a pure function of the size. Key it on the state
        // and the path so relinking retires those frames — returning None
        // here would instead make every frame of the comp unkeyable, so a
        // project with one lost file would cache nothing at all.
        if probe.slates() {
            return Some((format!("missing#{}", f.media.relative_path), 0));
        }
        // Audio-only contributes no picture but is fully known, so it is a
        // stable stamp rather than an unkeyable unknown.
        if probe == SourceProbe::AudioOnly {
            return Some((format!("audio#{}", f.media.relative_path), 0));
        }
        let (fps, width, _height, frames) = probe.video()?;
        let source_frame = ((lt * fps).round().max(0.0) as usize).min(frames.saturating_sub(1));
        // Key at the SPECIFIED resolution: draft frames are never cached, so
        // the content-hash key always represents the settled resolution.
        let settled = Quality {
            draft: false,
            ..self.quality
        };
        // A layer that needs flow decodes at its own width whatever the preview
        // tier says (K-331), so its name must say so too — the plan and this
        // stamp must never disagree about the width the pixels have.
        let target = if native {
            None
        } else {
            settled.target_width(width)
        };
        Some((
            format!("{}#w{}", f.media.absolute_path, target.unwrap_or(0)),
            source_frame as u64,
        ))
    }
}

/// The content-hash name of `comp`'s frame at integer `frame`, or `None` while
/// some footage is still unprobed (rendered live, never banked).
#[must_use]
pub fn frame_key(
    doc: &std::sync::Arc<Document>,
    comp: &Composition,
    frame: usize,
    quality: Quality,
    probes: &dyn SourceProbes,
) -> Option<u128> {
    let t = frame as f64 / comp.frame_rate.fps().max(1.0);
    frame_key_at(doc, comp, t, quality, probes)
}

/// [`frame_key`] at a comp time rather than an integer frame — the form a
/// nested comp is named in, since a Precomp layer's time is its layer time,
/// which need not land on the nested comp's own frame grid.
#[must_use]
pub fn frame_key_at(
    doc: &std::sync::Arc<Document>,
    comp: &Composition,
    t: f64,
    quality: Quality,
    probes: &dyn SourceProbes,
) -> Option<u128> {
    let stamper = Stamper::new(doc, probes, quality);
    lumit_eval::comp_frame_key(
        doc,
        comp,
        t,
        lumit_eval::Quality {
            divisor: quality.tag(),
        },
        &stamper,
    )
    .map(|k| k.0)
}

/// Names a nested comp's frame for the draw builder and the decode planner
/// (K-422, docs/06 §5.2): the key [`frame_key_at`] gives that comp at that
/// layer time, the same whichever parent asks for it. `None` when the frame
/// must not be cached — some footage unprobed, or a draft render, whose
/// decode is narrower than the settled name would claim.
pub trait NestedKeyer {
    fn nested_key(&self, nested: &Composition, lt: f64) -> Option<u128>;
}

/// The renderer's [`NestedKeyer`]: the document, the probes a render already
/// gathered, and the quality it renders at.
pub struct NestedKeys<'a> {
    pub doc: &'a std::sync::Arc<Document>,
    pub probes: &'a dyn SourceProbes,
    pub quality: Quality,
}

impl NestedKeyer for NestedKeys<'_> {
    fn nested_key(&self, nested: &Composition, lt: f64) -> Option<u128> {
        if self.quality.draft {
            return None;
        }
        frame_key_at(self.doc, nested, lt, self.quality, self.probes)
    }
}

/// Frame count of a comp preview (comp duration × comp rate).
#[must_use]
pub fn comp_frame_count(comp: &Composition) -> usize {
    let dur = comp.duration.0.to_f64();
    (dur * comp.frame_rate.fps()).round().max(1.0) as usize
}

/// Work-area frame span of a comp (start, end-exclusive); full when unset.
#[must_use]
pub fn work_area_frames(comp: &Composition) -> (usize, usize) {
    let total = comp_frame_count(comp);
    let fps = comp.frame_rate.fps().max(1.0);
    match comp.work_area {
        Some((a, b)) => {
            let s = ((a.0.to_f64() * fps).round() as usize).min(total.saturating_sub(1));
            let e = ((b.0.to_f64() * fps).round() as usize).clamp(s + 1, total);
            (s, e)
        }
        None => (0, total),
    }
}

/// Frame visit order for the idle background cache fill: the playhead first,
/// then a forward-biased walk — roughly three frames ahead of the playhead for
/// every one behind — because playback and scrubbing usually head forwards, so
/// the frames most likely to be viewed next should cache first. Every work-area
/// frame appears exactly once.
#[must_use]
pub fn fill_walk_order(playhead: usize, start: usize, end: usize) -> Vec<usize> {
    let mut order = Vec::new();
    if end <= start || playhead < start || playhead >= end {
        return order;
    }
    let span = end - start;
    order.push(playhead);
    let (mut ahead, mut behind) = (1usize, 1usize);
    let mut k = 0usize;
    while order.len() < span && k < span * 2 + 8 {
        // One behind for every three ahead; when a side is exhausted the other
        // takes over so every frame is still visited.
        let want_behind = k % 4 == 3;
        let forward = playhead + ahead;
        if !want_behind && forward < end {
            order.push(forward);
            ahead += 1;
        } else if let Some(f) = playhead.checked_sub(behind).filter(|f| *f >= start) {
            order.push(f);
            behind += 1;
        } else if forward < end {
            order.push(forward);
            ahead += 1;
        }
        k += 1;
    }
    order
}

/// Frames to warm ahead of the playhead during playback: the bounded forward
/// window `[playhead + 1, playhead + lookahead]`, clamped to the work-area end
/// (`end` exclusive). Playback presentation chases the audio clock, so warming
/// a little ahead of it keeps the work-area loop smooth once frames are cached
/// (docs/impl/playback-scheduler.md §5). Empty once the playhead reaches the end.
#[must_use]
pub fn playback_lookahead(playhead: usize, end: usize, lookahead: usize) -> Vec<usize> {
    let first = playhead.saturating_add(1);
    let stop = first.saturating_add(lookahead).min(end);
    (first..stop).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::model::{
        Composition, Layer, LayerKind, LinearColour, MediaRef, Switches, TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
    use std::collections::HashMap;

    fn footage_comp() -> (std::sync::Arc<Document>, Composition, Uuid) {
        let mut doc = Document::new();
        let item = Uuid::now_v7();
        doc.items
            .push(ProjectItem::Footage(lumit_core::model::FootageItem {
                sequence: None,
                id: item,
                name: "clip.mp4".into(),
                media: MediaRef {
                    relative_path: "clip.mp4".into(),
                    absolute_path: "/media/clip.mp4".into(),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
                extra: serde_json::Map::new(),
            }));
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "Scene".into(),
            width: 64,
            height: 64,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(4, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![Layer {
                markers: Vec::new(),
                id: Uuid::now_v7(),
                name: "clip".into(),
                kind: LayerKind::Footage { item },
                in_point: CompTime(Rational::ZERO),
                out_point: CompTime(Rational::new(4, 1).unwrap()),
                start_offset: CompTime(Rational::ZERO),
                transform: TransformGroup::default(),
                matte: None,
                parent: None,
                label: 0,
                volume_db: lumit_core::anim::Property::zero(),
                audio_only: false,
                retime: None,
                interpolation: Default::default(),
                parked_flow: None,
                blend: Default::default(),
                masks: Vec::new(),
                paint: Vec::new(),
                effects: Vec::new(),
                switches: Switches::default(),
                extra: serde_json::Map::new(),
            }],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        doc.items.push(ProjectItem::Composition(comp.clone()));
        (std::sync::Arc::new(doc), comp, item)
    }

    fn probed(item: Uuid) -> HashMap<Uuid, SourceProbe> {
        let mut probes = HashMap::new();
        probes.insert(
            item,
            SourceProbe::Video {
                fps: 30.0,
                width: 64,
                height: 64,
                frames: 120,
                audio: false,
            },
        );
        probes
    }

    /// A frame with unprobed footage has no name at all — it renders live and
    /// is never banked, so it can never be filed under a promise it did not
    /// keep. Once probed it names cleanly.
    #[test]
    fn an_unprobed_source_makes_the_frame_unkeyable() {
        let (doc, comp, item) = footage_comp();
        let q = Quality::default();
        assert!(
            frame_key(&doc, &comp, 0, q, &crate::source::NoProbes).is_none(),
            "unprobed footage must make the frame unkeyable"
        );
        assert!(frame_key(&doc, &comp, 0, q, &probed(item)).is_some());
    }

    /// Different frames of the same comp get different names, and the same
    /// frame asked for twice gets the same name — the minimum a cache needs.
    #[test]
    fn frames_are_named_by_content_and_are_stable() {
        let (doc, comp, item) = footage_comp();
        let (q, probes) = (Quality::default(), probed(item));
        let a = frame_key(&doc, &comp, 0, q, &probes).unwrap();
        let b = frame_key(&doc, &comp, 10, q, &probes).unwrap();
        assert_ne!(a, b, "different frames are different pictures");
        assert_eq!(a, frame_key(&doc, &comp, 0, q, &probes).unwrap());
    }

    /// The same frame at a different preview resolution is a DIFFERENT entry,
    /// so a half-resolution scrub frame is never served as the full-resolution
    /// one (docs/06 §5.2 quality axis).
    #[test]
    fn each_resolution_tier_names_separately() {
        let (doc, comp, item) = footage_comp();
        let probes = probed(item);
        let full = frame_key(&doc, &comp, 0, Quality::default(), &probes).unwrap();
        let half = frame_key(
            &doc,
            &comp,
            0,
            Quality {
                divisor: 2,
                ..Quality::default()
            },
            &probes,
        )
        .unwrap();
        assert_ne!(full, half);
    }

    /// An edit that cannot change the picture must not invalidate the cache —
    /// this is the whole reason the key hashes content rather than document
    /// identity. Renaming a layer keeps every frame; moving it does not.
    #[test]
    fn a_picture_free_edit_keeps_the_frames_and_a_move_retires_them() {
        let (doc, comp, item) = footage_comp();
        let (q, probes) = (Quality::default(), probed(item));
        let before = frame_key(&doc, &comp, 0, q, &probes).unwrap();

        let mut renamed = comp.clone();
        renamed.layers[0].name = "a much better name".into();
        assert_eq!(
            frame_key(&doc, &renamed, 0, q, &probes).unwrap(),
            before,
            "renaming a layer cannot change the picture, so the frame survives"
        );

        let mut moved = comp.clone();
        moved.layers[0].transform.position_x = lumit_core::anim::Property::fixed(17.0);
        assert_ne!(
            frame_key(&doc, &moved, 0, q, &probes).unwrap(),
            before,
            "moving a layer does change the picture"
        );
    }

    /// A missing file still names its frames (the slate is a pure function of
    /// the size), so one lost clip does not stop a whole project caching.
    #[test]
    fn missing_footage_is_still_cacheable() {
        let (doc, comp, item) = footage_comp();
        let mut probes = HashMap::new();
        probes.insert(item, SourceProbe::Missing);
        assert!(frame_key(&doc, &comp, 0, Quality::default(), &probes).is_some());
    }

    /// The fill walk visits every work-area frame exactly once, starting at the
    /// playhead and leaning forwards.
    #[test]
    fn the_fill_walk_covers_the_work_area_once_leaning_forward() {
        let order = fill_walk_order(5, 0, 12);
        assert_eq!(order.len(), 12, "every work-area frame appears");
        let mut sorted = order.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 12, "no frame appears twice");
        assert_eq!(order[0], 5, "the playhead's own frame comes first");
        let ahead = order.iter().take(5).filter(|f| **f > 5).count();
        assert!(ahead >= 3, "the walk leans forward, got {order:?}");
        // Playhead at the work-area start: everything is ahead, no panic.
        assert_eq!(fill_walk_order(0, 0, 4), vec![0, 1, 2, 3]);
        assert_eq!(fill_walk_order(0, 0, 1), vec![0]);
        // Degenerate spans are empty rather than a panic.
        assert!(fill_walk_order(5, 0, 0).is_empty());
        assert!(fill_walk_order(0, 0, 0).is_empty());
        assert!(fill_walk_order(99, 0, 12).is_empty());
    }

    /// Playback warms a bounded window strictly ahead of the playhead, clamped
    /// to the work-area end — never behind, never past the end.
    #[test]
    fn playback_warms_only_forward_within_the_work_area() {
        assert_eq!(playback_lookahead(3, 20, 4), vec![4, 5, 6, 7]);
        assert_eq!(playback_lookahead(18, 20, 4), vec![19]);
        assert!(playback_lookahead(19, 20, 4).is_empty());
        assert!(playback_lookahead(20, 20, 4).is_empty());
        assert!(
            playback_lookahead(5, 100, 0).is_empty(),
            "a zero window warms nothing"
        );
    }

    /// The work area defaults to the whole comp and is otherwise read in
    /// A paint stroke is part of the layer's picture, so it must retire the
    /// frames that were named before it existed.
    ///
    /// It did not. The key hashed a layer's masks but never its paint, so a
    /// brush drag changed no name, every cached frame stayed valid, and the
    /// stroke was invisible until something else in the comp moved — which is
    /// exactly the report: "after letting go the line you drew disappears and
    /// nothing makes it reappear".
    #[test]
    fn a_paint_stroke_retires_the_frames_it_changes() {
        use lumit_core::paint::{PaintMode, PaintStroke};
        let (doc, comp, item) = footage_comp();
        let (q, probes) = (Quality::default(), probed(item));
        let before = frame_key(&doc, &comp, 0, q, &probes).unwrap();

        let stroke = |width: f64| PaintStroke {
            id: Uuid::from_u128(7),
            name: "Brush 1".into(),
            points: vec![(1.0, 2.0), (30.0, 40.0)],
            colour: lumit_core::model::LinearColour([1.0, 0.0, 0.0, 1.0]),
            width,
            hardness: 1.0,
            shape: lumit_core::paint::BrushShape::Round,
            opacity: 100.0,
            start: lumit_core::anim::Property::zero(),
            end: lumit_core::anim::Property::fixed(100.0),
            mode: PaintMode::Paint,
            clone_offset: (0.0, 0.0),
            extra: serde_json::Map::new(),
        };

        let mut painted = comp.clone();
        painted.layers[0].paint = vec![stroke(12.0)];
        let with_paint = frame_key(&doc, &painted, 0, q, &probes).unwrap();
        assert_ne!(
            with_paint, before,
            "a stroke changes the picture, so it must change the frame's name"
        );

        // And the stroke's own settings are content too: widening a brush
        // repaints, so it cannot land on the name the thinner one holds.
        let mut wider = comp.clone();
        wider.layers[0].paint = vec![stroke(24.0)];
        assert_ne!(
            frame_key(&doc, &wider, 0, q, &probes).unwrap(),
            with_paint,
            "a stroke's width is part of what it draws"
        );

        // A layer with no paint must hash exactly as it always did, so adding
        // this did not throw away every frame banked before it.
        let mut unpainted = comp.clone();
        unpainted.layers[0].paint = Vec::new();
        assert_eq!(
            frame_key(&doc, &unpainted, 0, q, &probes).unwrap(),
            before,
            "an unpainted layer keeps the name it had"
        );
    }

    /// A shape layer's art is its whole picture, so editing it must retire the
    /// frames drawn from the old art. Nothing hashed `contents`, so recolouring
    /// or reshaping a shape layer showed the frame it had before.
    #[test]
    fn editing_a_shape_layers_art_retires_its_frames() {
        use lumit_core::shape::ShapeItem;
        let (doc, comp, item) = footage_comp();
        let (q, probes) = (Quality::default(), probed(item));

        let art = |red: f32| ShapeItem {
            id: Uuid::from_u128(9),
            name: "Rectangle".into(),
            path: lumit_core::mask::BezierPath {
                vertices: vec![
                    vertex(0.0, 0.0),
                    vertex(60.0, 0.0),
                    vertex(60.0, 40.0),
                    vertex(0.0, 40.0),
                ],
                closed: true,
            },
            fill: Some(lumit_core::model::LinearColour([red, 0.0, 0.0, 1.0])),
            stroke: None,
            stroke_width: 0.0,
            opacity: 100.0,
            extra: serde_json::Map::new(),
        };

        let mut shaped = comp.clone();
        shaped.layers[0].kind = lumit_core::model::LayerKind::Shape {
            contents: vec![art(1.0)],
        };
        let red = frame_key(&doc, &shaped, 0, q, &probes).unwrap();

        let mut recoloured = shaped.clone();
        recoloured.layers[0].kind = lumit_core::model::LayerKind::Shape {
            contents: vec![art(0.25)],
        };
        assert_ne!(
            frame_key(&doc, &recoloured, 0, q, &probes).unwrap(),
            red,
            "a shape's fill colour is its picture"
        );

        let mut emptied = shaped.clone();
        emptied.layers[0].kind = lumit_core::model::LayerKind::Shape {
            contents: Vec::new(),
        };
        assert_ne!(
            frame_key(&doc, &emptied, 0, q, &probes).unwrap(),
            red,
            "deleting the art changes the picture"
        );
    }

    fn vertex(x: f64, y: f64) -> lumit_core::mask::Vertex {
        lumit_core::mask::Vertex {
            pos: (x, y),
            tan_in: (0.0, 0.0),
            tan_out: (0.0, 0.0),
        }
    }

    /// frames, with the end always after the start.
    #[test]
    fn the_work_area_defaults_to_the_whole_comp() {
        let (_doc, mut comp, _item) = footage_comp();
        assert_eq!(work_area_frames(&comp), (0, 120));
        comp.work_area = Some((
            CompTime(Rational::new(1, 1).unwrap()),
            CompTime(Rational::new(2, 1).unwrap()),
        ));
        assert_eq!(work_area_frames(&comp), (30, 60));
    }
}
