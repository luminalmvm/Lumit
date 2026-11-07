//! A text animator on its way between the document and the panel.

use flutter_rust_bridge::frb;

use super::{BridgeRangeSelector, BridgeSelectorBasis, BridgeSelectorShape, BridgeTextAnimator};
use crate::api::{effect::BridgeScalar, BridgeError};

/// One animator on its way out to the panel, its keys on the comp's clock.
#[frb(ignore)]
pub(crate) fn read_animator(
    animator: &lumit_core::text::TextAnimator,
    offset: lumit_core::time::Rational,
) -> BridgeTextAnimator {
    let read = |p| BridgeScalar::read_at(p, offset);
    BridgeTextAnimator {
        name: animator.name.clone(),
        selector: BridgeRangeSelector {
            start: read(&animator.selector.start),
            end: read(&animator.selector.end),
            offset: read(&animator.selector.offset),
            basis: match animator.selector.basis {
                lumit_core::text::SelectorBasis::Characters => BridgeSelectorBasis::Characters,
                lumit_core::text::SelectorBasis::Words => BridgeSelectorBasis::Words,
            },
            shape: match animator.selector.shape {
                lumit_core::text::SelectorShape::Square => BridgeSelectorShape::Square,
                lumit_core::text::SelectorShape::Ramp => BridgeSelectorShape::Ramp,
            },
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
    }
}

/// And back: the panel's animator returned to the layer's own clock.
#[frb(ignore)]
pub(super) fn animator_from(
    animator: BridgeTextAnimator,
    offset: lumit_core::time::Rational,
) -> Result<lumit_core::text::TextAnimator, BridgeError> {
    let write = |s: &BridgeScalar| -> Result<lumit_core::anim::Property, BridgeError> {
        Ok(lumit_core::anim::Property {
            animation: s.animation_at(offset)?,
            extra: serde_json::Map::new(),
        })
    };
    Ok(lumit_core::text::TextAnimator {
        name: animator.name,
        selector: lumit_core::text::RangeSelector {
            start: write(&animator.selector.start)?,
            end: write(&animator.selector.end)?,
            offset: write(&animator.selector.offset)?,
            basis: match animator.selector.basis {
                BridgeSelectorBasis::Characters => lumit_core::text::SelectorBasis::Characters,
                BridgeSelectorBasis::Words => lumit_core::text::SelectorBasis::Words,
            },
            shape: match animator.selector.shape {
                BridgeSelectorShape::Square => lumit_core::text::SelectorShape::Square,
                BridgeSelectorShape::Ramp => lumit_core::text::SelectorShape::Ramp,
            },
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
        extra: serde_json::Map::new(),
    })
}
