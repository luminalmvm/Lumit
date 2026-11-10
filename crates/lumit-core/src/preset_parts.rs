//! What a preset can carry beside its effects: a layer's transform keys and a
//! text layer's animators.
//!
//! In plain terms: these sit in the same `.lumfx` file as two more lists, left
//! out when empty, so a file with neither reads as it always has and a Lumit
//! that doesn't know them reads the effects and ignores the rest.

use serde::{Deserialize, Serialize};

use crate::anim::{Animation, Property};
use crate::model::{TransformGroup, TransformProp};
use crate::text::TextAnimator;
use crate::time::Rational;

/// The rows a preset's transform block can hold: Anchor point, Position, Scale,
/// Rotation and Opacity, each with its 3D axes. Keys on a camera's or a light's
/// own rows stay with that layer.
const ROWS: [TransformProp; 16] = [
    TransformProp::AnchorX,
    TransformProp::AnchorY,
    TransformProp::AnchorZ,
    TransformProp::PositionX,
    TransformProp::PositionY,
    TransformProp::PositionZ,
    TransformProp::ScaleX,
    TransformProp::ScaleY,
    TransformProp::ScaleZ,
    TransformProp::Rotation,
    TransformProp::RotationX,
    TransformProp::RotationY,
    TransformProp::OrientationX,
    TransformProp::OrientationY,
    TransformProp::OrientationZ,
    TransformProp::Opacity,
];

/// One keyed transform row, in layer-local time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyedRow {
    pub prop: TransformProp,
    pub animation: Animation,
}

/// The two lists, as they sit in the file beside `effects`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PresetParts {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transform: Vec<KeyedRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub animators: Vec<TextAnimator>,
}

/// The keyed rows of `group`, and nothing that only holds a value.
#[must_use]
pub fn keyed_rows(group: &TransformGroup) -> Vec<KeyedRow> {
    ROWS.iter()
        .filter_map(|&prop| {
            let property = group.get(prop)?;
            matches!(property.animation, Animation::Keyframed(_)).then(|| KeyedRow {
                prop,
                animation: property.animation.clone(),
            })
        })
        .collect()
}

/// Every animatable number on an animator, so a new row can't be missed by the
/// one thing that moves them.
fn animator_rows(animator: &mut TextAnimator) -> [&mut Property; 19] {
    let TextAnimator {
        selector,
        position_x,
        position_y,
        rotation,
        position_z,
        rotation_x,
        rotation_y,
        scale_x,
        scale_y,
        opacity,
        fill_r,
        fill_g,
        fill_b,
        box_opacity,
        box_r,
        box_g,
        box_b,
        ..
    } = animator;
    [
        &mut selector.start,
        &mut selector.end,
        &mut selector.offset,
        position_x,
        position_y,
        rotation,
        position_z,
        rotation_x,
        rotation_y,
        scale_x,
        scale_y,
        opacity,
        fill_r,
        fill_g,
        fill_b,
        box_opacity,
        box_r,
        box_g,
        box_b,
    ]
}

/// The same numbers, to read.
#[must_use]
pub fn animator_values(a: &TextAnimator) -> [&Property; 19] {
    [
        &a.selector.start,
        &a.selector.end,
        &a.selector.offset,
        &a.position_x,
        &a.position_y,
        &a.rotation,
        &a.position_z,
        &a.rotation_x,
        &a.rotation_y,
        &a.scale_x,
        &a.scale_y,
        &a.opacity,
        &a.fill_r,
        &a.fill_g,
        &a.fill_b,
        &a.box_opacity,
        &a.box_r,
        &a.box_g,
        &a.box_b,
    ]
}

impl PresetParts {
    /// The parts in a preset's text. Text that holds neither reads as empty.
    #[must_use]
    pub fn from_json(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.transform.is_empty() && self.animators.is_empty()
    }

    /// `preset`, a preset's JSON text, with these parts written into it.
    pub fn written_into(&self, preset: &str) -> Result<String, String> {
        if self.is_empty() {
            return Ok(preset.to_owned());
        }
        let mut document: serde_json::Value =
            serde_json::from_str(preset).map_err(|e| e.to_string())?;
        let parts = serde_json::to_value(self).map_err(|e| e.to_string())?;
        if let (Some(document), Some(parts)) = (document.as_object_mut(), parts.as_object()) {
            document.extend(parts.clone());
        }
        serde_json::to_string_pretty(&document).map_err(|e| e.to_string())
    }

    /// The earliest key in either list, in layer-local seconds.
    pub fn first_key_time(&mut self) -> Option<Rational> {
        let mut first: Option<Rational> = None;
        self.for_each_keys(|keys| {
            if let Some(key) = keys.first() {
                first = Some(match first {
                    Some(held) if held <= key.time => held,
                    _ => key.time,
                });
            }
        });
        first
    }

    /// Move every key by `delta` layer-local seconds. A key that can't move
    /// without overflowing stays, as [`crate::preset::shift_keys`] leaves one.
    pub fn shift_keys(&mut self, delta: Rational) {
        self.for_each_keys(|keys| {
            for key in keys {
                if let Ok(moved) = key.time.checked_add(delta) {
                    key.time = moved;
                }
            }
        });
    }

    fn for_each_keys(&mut self, mut visit: impl FnMut(&mut Vec<crate::anim::Keyframe>)) {
        for row in &mut self.transform {
            if let Animation::Keyframed(keys) = &mut row.animation {
                visit(keys);
            }
        }
        for animator in &mut self.animators {
            for row in animator_rows(animator) {
                if let Animation::Keyframed(keys) = &mut row.animation {
                    visit(keys);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A file written with parts must still open as a plain preset, and a plain
    /// preset must read as having none, or saved presets stop loading.
    #[test]
    fn the_parts_sit_beside_the_effects_in_one_file() {
        let plain = crate::preset::to_json("look", &[]).unwrap();
        assert!(PresetParts::from_json(&plain).is_empty());

        let parts = PresetParts {
            transform: vec![KeyedRow {
                prop: TransformProp::Opacity,
                animation: Animation::Keyframed(Vec::new()),
            }],
            animators: vec![TextAnimator::new("Fade")],
        };
        let written = parts.written_into(&plain).unwrap();
        assert_eq!(PresetParts::from_json(&written), parts);
        assert_eq!(crate::preset::from_json(&written).unwrap().name, "look");
    }
}
