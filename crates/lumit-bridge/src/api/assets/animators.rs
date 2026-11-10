//! A text animator on its way between the document and the panel.

use flutter_rust_bridge::frb;

use super::{
    BridgeAnimatorAxis, BridgeRangeSelector, BridgeSelectorBasis, BridgeSelectorKind,
    BridgeSelectorShape, BridgeTextAnimator,
};
use crate::api::{effect::BridgeScalar, BridgeError};

/// One animator on its way out to the panel, its keys on the comp's clock.
#[frb(ignore)]
pub(crate) fn read_animator(
    animator: &lumit_core::text::TextAnimator,
    offset: lumit_core::time::Rational,
) -> BridgeTextAnimator {
    use lumit_core::text::{SelectorBasis, SelectorKind, SelectorShape};
    let read = |p| BridgeScalar::read_at(p, offset);
    let s = &animator.selector;
    BridgeTextAnimator {
        name: animator.name.clone(),
        selector: BridgeRangeSelector {
            start: read(&s.start),
            end: read(&s.end),
            offset: read(&s.offset),
            basis: match s.basis {
                SelectorBasis::Characters => BridgeSelectorBasis::Characters,
                SelectorBasis::Words => BridgeSelectorBasis::Words,
            },
            shape: match s.shape {
                SelectorShape::Square => BridgeSelectorShape::Square,
                SelectorShape::Ramp => BridgeSelectorShape::Ramp,
                SelectorShape::Smooth => BridgeSelectorShape::Smooth,
            },
            kind: match s.kind {
                SelectorKind::Range => BridgeSelectorKind::Range,
                SelectorKind::Proximity => BridgeSelectorKind::Proximity,
                SelectorKind::Wiggly => BridgeSelectorKind::Wiggly,
            },
            target: s.target,
            radius: read(&s.radius),
            invert: s.invert,
            min_amount: read(&s.min_amount),
            max_amount: read(&s.max_amount),
            wiggles: read(&s.wiggles),
            correlation: read(&s.correlation),
            seed: s.seed,
        },
        position_x: read(&animator.position_x),
        position_y: read(&animator.position_y),
        rotation: read(&animator.rotation),
        position_z: read(&animator.position_z),
        rotation_x: read(&animator.rotation_x),
        rotation_y: read(&animator.rotation_y),
        scale_x: read(&animator.scale_x),
        scale_y: read(&animator.scale_y),
        opacity: read(&animator.opacity),
        fill_r: read(&animator.fill_r),
        fill_g: read(&animator.fill_g),
        fill_b: read(&animator.fill_b),
        axes: animator
            .axes
            .iter()
            .map(|a| BridgeAnimatorAxis {
                tag: a.tag.clone(),
                value: read(&a.value),
            })
            .collect(),
        properties: animator.properties.clone(),
    }
}

/// And back: the panel's animator returned to the layer's own clock.
#[frb(ignore)]
pub(super) fn animator_from(
    animator: BridgeTextAnimator,
    offset: lumit_core::time::Rational,
) -> Result<lumit_core::text::TextAnimator, BridgeError> {
    use lumit_core::text::{SelectorBasis, SelectorKind, SelectorShape};
    let write = |s: &BridgeScalar| -> Result<lumit_core::anim::Property, BridgeError> {
        Ok(lumit_core::anim::Property {
            animation: s.animation_at(offset)?,
            extra: serde_json::Map::new(),
        })
    };
    let s = &animator.selector;
    // An axis named twice is one axis, the later setting winning, and a tag
    // that isn't one is dropped.
    let mut axes: Vec<lumit_core::text::AnimatorAxis> = Vec::new();
    for a in &animator.axes {
        if lumit_core::text::axis_tag(&a.tag).is_none() {
            continue;
        }
        let value = write(&a.value)?;
        match axes.iter_mut().find(|b| b.tag == a.tag) {
            Some(b) => b.value = value,
            None => axes.push(lumit_core::text::AnimatorAxis {
                tag: a.tag.clone(),
                value,
            }),
        }
    }
    Ok(lumit_core::text::TextAnimator {
        name: animator.name.clone(),
        selector: lumit_core::text::RangeSelector {
            start: write(&s.start)?,
            end: write(&s.end)?,
            offset: write(&s.offset)?,
            basis: match s.basis {
                BridgeSelectorBasis::Characters => SelectorBasis::Characters,
                BridgeSelectorBasis::Words => SelectorBasis::Words,
            },
            shape: match s.shape {
                BridgeSelectorShape::Square => SelectorShape::Square,
                BridgeSelectorShape::Ramp => SelectorShape::Ramp,
                BridgeSelectorShape::Smooth => SelectorShape::Smooth,
            },
            kind: match s.kind {
                BridgeSelectorKind::Range => SelectorKind::Range,
                BridgeSelectorKind::Proximity => SelectorKind::Proximity,
                BridgeSelectorKind::Wiggly => SelectorKind::Wiggly,
            },
            target: s.target,
            radius: write(&s.radius)?,
            invert: s.invert,
            min_amount: write(&s.min_amount)?,
            max_amount: write(&s.max_amount)?,
            wiggles: write(&s.wiggles)?,
            correlation: write(&s.correlation)?,
            seed: s.seed,
        },
        position_x: write(&animator.position_x)?,
        position_y: write(&animator.position_y)?,
        rotation: write(&animator.rotation)?,
        position_z: write(&animator.position_z)?,
        rotation_x: write(&animator.rotation_x)?,
        rotation_y: write(&animator.rotation_y)?,
        scale_x: write(&animator.scale_x)?,
        scale_y: write(&animator.scale_y)?,
        opacity: write(&animator.opacity)?,
        fill_r: write(&animator.fill_r)?,
        fill_g: write(&animator.fill_g)?,
        fill_b: write(&animator.fill_b)?,
        axes,
        // Each group once, in the order added.
        properties: animator.properties.map(|list| {
            let mut out: Vec<String> = Vec::new();
            for p in list {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
            out
        }),
        extra: serde_json::Map::new(),
    })
}
