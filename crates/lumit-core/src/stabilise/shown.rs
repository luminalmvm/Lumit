//! Which analysed frames a layer shows at a moment, and the rate its source
//! plays at.
//!
//! In plain terms: the picture a footage layer shows is one whole source
//! frame, the one the decode picks, or two of them mixed when the layer blends
//! frames. The map, Fill's neighbours, the rolling shutter and the Viewer's
//! points all read the same pick, so they agree with the picture.

use super::analysis::Analysis;
use crate::model::{Document, Layer, LayerKind, ProjectItem};
use crate::retime::Interpolation;

/// The analysed frames a layer shows at one moment, counted from the
/// analysis's first: one, or two the layer blends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    /// The frame shown, or the first of the two blended.
    pub at: usize,
    /// The second of two blended frames, and how far towards it the picture
    /// is, above nought and below one.
    pub blend: Option<(usize, f64)>,
}

impl Shown {
    /// One frame, alone.
    #[must_use]
    pub fn single(at: usize) -> Shown {
        Shown { at, blend: None }
    }

    /// The one frame nearest what is shown.
    #[must_use]
    pub fn nearest(&self) -> usize {
        match self.blend {
            Some((b, w)) if w >= 0.5 => b,
            _ => self.at,
        }
    }

    /// `f` of the shown frame, or of the two mixed by the same weight.
    pub fn mix<T>(&self, f: impl Fn(usize) -> T, lerp: impl Fn(T, T, f64) -> T) -> T {
        match self.blend {
            Some((b, w)) => lerp(f(self.at), f(b), w),
            None => f(self.at),
        }
    }
}

/// The rate `layer`'s source frames play at now: the item's Interpret footage
/// rate, or its image sequence's, where one is set, and the rate the analysis
/// read the file at otherwise.
#[must_use]
pub fn rate(doc: &Document, layer: &Layer, a: &Analysis) -> f64 {
    let own = if a.fps.is_finite() && a.fps > 0.0 {
        a.fps
    } else {
        1.0
    };
    let LayerKind::Footage { item } = layer.kind else {
        return own;
    };
    let Some(f) = doc.items.iter().find_map(|i| match i {
        ProjectItem::Footage(f) if f.id == item => Some(f),
        _ => None,
    }) else {
        return own;
    };
    // The same limits the decode holds a set rate to.
    let set = match &f.sequence {
        Some(s) => Some(s.frame_rate.fps()),
        None => f
            .interpretation
            .frame_rate
            .map(|r| r.fps())
            .filter(|r| (0.01..=999.0).contains(r)),
    };
    match set {
        Some(r) if r.is_finite() && r > 0.0 && a.rate_scale > 0.0 => r * a.rate_scale,
        _ => own,
    }
}

/// The analysed frames `layer` shows at layer time `lt`, its source playing
/// at `rate`, in a comp at `comp_fps`: picked as the decode picks a footage
/// layer's frame, blending two only where the layer blends frames (Blend, or
/// Flow where it engages). A Precomp layer shows its nested comp at the
/// moment itself, read as the nearest frame. Held to the analysed span.
#[must_use]
pub fn shown(layer: &Layer, comp_fps: f64, lt: f64, rate: f64, a: &Analysis) -> Option<Shown> {
    let frames = usize::try_from(a.frames).ok().filter(|&n| n > 0)?;
    let st = layer.source_time_at(lt);
    if !st.is_finite() {
        return None;
    }
    let (blend_on, sample_fps) = match layer.kind {
        LayerKind::Footage { .. } => {
            let speed = crate::retime::property_speed_at(layer.retime.as_ref(), lt);
            let flow = match &layer.interpolation {
                Interpolation::Flow(p) => p
                    .engages(p.read_fps_at(lt, rate), comp_fps, speed)
                    .then_some(p),
                _ => None,
            };
            (
                matches!(layer.interpolation, Interpolation::Blend) || flow.is_some(),
                flow.and_then(|p| p.input_fps_at(lt)),
            )
        }
        _ => (false, None),
    };
    let end = usize::try_from(a.first.max(0)).unwrap_or(0) + frames;
    let (pick, blend) = crate::pixels::frame_pick(st, rate, end, blend_on, sample_fps);
    let index = |n: usize| {
        let i = n as i64 - a.first;
        i.clamp(0, frames as i64 - 1) as usize
    };
    let at = index(pick);
    let blend = blend
        .map(|(b, w)| (index(b), f64::from(w)))
        .filter(|&(b, w)| b != at && w > 0.0 && w < 1.0);
    Some(Shown { at, blend })
}

/// The source frame a whole-frame decode shows of `layer` at layer time `lt`,
/// counted from `a`'s first and not held to the span: how a temporal effect's
/// neighbour frames are picked.
#[must_use]
pub fn nearest(layer: &Layer, lt: f64, rate: f64, a: &Analysis) -> f64 {
    (layer.source_time_at(lt) * rate).max(0.0).round() - a.first as f64
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::model::TransformGroup;
    use crate::stabilise::analysis::Step;
    use crate::stabilise::frame::Framed;
    use crate::stabilise::plan::Plan;
    use crate::time::Rational;
    use std::sync::Arc;
    use uuid::Uuid;

    /// A 30 fps clip in a 24 fps comp shows whole source frames, and each is
    /// drawn with its own map, never a mix of two. Only a layer that blends
    /// frames mixes two maps, by the decode's own weight.
    #[test]
    fn the_shown_frame_is_drawn_with_its_own_map_at_another_comp_rate() {
        let mut layer = crate::edits::base_layer(
            "clip".into(),
            LayerKind::Footage { item: Uuid::nil() },
            Rational::new(10, 1).unwrap(),
            TransformGroup::default(),
        );
        let a = Analysis {
            fps: 30.0,
            rate_scale: 1.0,
            width: 64,
            height: 36,
            scale: 1.0,
            first: 0,
            frames: 6,
            asked: 6,
            steps: vec![Step::STILL; 5],
            runs: Vec::new(),
            readout: [0.0; 2],
        };
        let slide = |i: usize| [[1.0, 0.0, i as f64], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let plan = Plan {
            id: 0,
            first: 0,
            frames: 6,
            size: [64.0, 36.0],
            maps: (0..6).map(slide).collect(),
            framed: Framed {
                scale: vec![1.0; 6],
                crop: vec![1.0; 6],
                needed: 1.0,
                short: false,
            },
            steps: Arc::new(a.steps.clone()),
            motion: vec![[0.0; 4]; 6],
        };
        // Comp frame 2 is source frame 2.5, and the decode shows frame 3.
        let lt = 2.0 / 24.0;
        let s = shown(&layer, 24.0, lt, 30.0, &a).unwrap();
        assert_eq!(s, Shown::single(3));
        assert_eq!(plan.at(s).unwrap().map, slide(3));
        // Blended, the two frames either side, half and half.
        layer.interpolation = Interpolation::Blend;
        let s = shown(&layer, 24.0, lt, 30.0, &a).unwrap();
        assert_eq!(s.at, 2);
        let (b, w) = s.blend.unwrap();
        assert_eq!(b, 3);
        assert!((w - 0.5).abs() < 1e-6);
        assert!((plan.at(s).unwrap().map[0][2] - 2.5).abs() < 1e-6);
    }
}
