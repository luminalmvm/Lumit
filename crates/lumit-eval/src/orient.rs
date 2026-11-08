//! What Orient adds to a frame's name. Nothing while it is off, so every other
//! key stays as it was. Towards camera adds only its name, as the camera, the
//! layer's place and its parents are in the key already.

use super::feed_f64;
use lumit_core::model::TransformGroup;
use lumit_core::orient::{direction, Orient};

pub(super) fn feed(h: &mut blake3::Hasher, tr: &TransformGroup, lt: f64) {
    match tr.orient {
        Orient::Off => {}
        Orient::TowardsCamera => {
            h.update(b"orient-camera/");
        }
        Orient::AlongPath => {
            h.update(b"orient-path/");
            let (x, y, z) = direction(tr, lt, false).unwrap_or((0.0, 0.0, 0.0));
            for v in [x, y, z] {
                feed_f64(h, v);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::{comp_frame_key, Quality, SourceStamper};
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    use lumit_core::edits::{base_layer, new_comp_ops, CompSettings};
    use lumit_core::model::{Document, Layer, LayerKind};
    use lumit_core::ops::{apply, Op};
    use lumit_core::orient::Orient;
    use lumit_core::time::Rational;
    use std::sync::Arc;
    use uuid::Uuid;

    struct Nothing;
    impl SourceStamper for Nothing {
        fn stamp(&self, item: Uuid, _lt: f64, _native: bool) -> Option<(String, u64)> {
            Some((item.to_string(), 0))
        }
    }

    fn name(edit: impl Fn(&mut Layer)) -> Option<u128> {
        let mut doc = Document::new();
        let (id, ops) = new_comp_ops(&doc, CompSettings::default(), None);
        for op in &ops {
            apply(&mut doc, op).unwrap();
        }
        let duration = doc.comp(id).unwrap().duration.0;
        let mut layer = base_layer(
            "moving".into(),
            LayerKind::Solid {
                def: Uuid::from_u128(9),
            },
            duration,
            Default::default(),
        );
        layer.id = Uuid::from_u128(7);
        edit(&mut layer);
        let index = 0;
        apply(
            &mut doc,
            &Op::AddLayer {
                comp: id,
                index,
                layer: Box::new(layer),
            },
        )
        .unwrap();
        let doc = Arc::new(doc);
        let comp = doc.comp(id).unwrap().clone();
        comp_frame_key(&doc, &comp, 1.0, Quality::default(), &Nothing).map(|k| k.0)
    }

    fn through_100_at_one_second(from: f64, to: f64) -> Property {
        let key = |sec: i64, value: f64| Keyframe {
            time: Rational::new(sec, 1).unwrap(),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let mut p = Property::fixed(0.0);
        p.animation = Animation::Keyframed(vec![key(0, from), key(2, to)]);
        p
    }

    /// Off names a frame as it always did, and two layers at the same place on
    /// the same frame heading different ways turn differently, so one's frame
    /// can't stand in for the other's.
    #[test]
    fn the_way_a_layer_is_heading_names_the_frame() {
        let climbing = |l: &mut Layer| {
            l.transform.position_x = through_100_at_one_second(0.0, 200.0);
            l.transform.position_y = through_100_at_one_second(0.0, 200.0);
        };
        let falling = |l: &mut Layer| {
            l.transform.position_x = through_100_at_one_second(0.0, 200.0);
            l.transform.position_y = through_100_at_one_second(200.0, 0.0);
        };
        assert_eq!(name(climbing), name(falling));
        let on = |l: &mut Layer| l.transform.orient = Orient::AlongPath;
        let along = |path: &dyn Fn(&mut Layer)| {
            name(|l| {
                path(l);
                on(l);
            })
        };
        assert_ne!(along(&climbing), along(&falling));
        assert_ne!(along(&climbing), name(climbing));
    }
}
