//! A Seed row's keys. A seed is a whole number, and two seeds have nothing
//! between them, so every key holds until the next.
//!
//! The keys sit beside the value rather than in it, so a project with none
//! saves the same bytes it always did. With keys, the value is the first
//! key's seed, which is what an older build reads and renders throughout.

use super::{EffectInstance, EffectParam, EffectValue};
use crate::anim::{Animation, Property};

impl EffectParam {
    /// A row holding `value` and nothing beside it.
    #[must_use]
    pub fn new(id: impl Into<String>, value: EffectValue) -> Self {
        Self {
            id: id.into(),
            value,
            seed_keys: None,
            extra: serde_json::Map::new(),
        }
    }

    /// The keys a Seed row holds, when it has any.
    #[must_use]
    pub fn keyed_seed(&self) -> Option<&Property> {
        self.seed_keys.as_ref().filter(|keys| keys.is_animated())
    }

    /// A Seed row's seed at layer time `lt`, or `None` for any other row.
    #[must_use]
    pub fn seed_at(&self, lt: f64) -> Option<u32> {
        let EffectValue::Seed(seed) = self.value else {
            return None;
        };
        Some(
            self.keyed_seed()
                .map_or(seed, |keys| whole(keys.value_at(lt))),
        )
    }

    /// Put `animation` on a Seed row: a seed, or keys that all hold. An
    /// expression, or any other row, is refused with `false`.
    pub fn set_seed(&mut self, animation: Animation) -> bool {
        let EffectValue::Seed(seed) = &mut self.value else {
            return false;
        };
        let keys = match animation {
            Animation::Static(v) => {
                *seed = whole(v);
                Vec::new()
            }
            Animation::Keyframed(keys) => keys,
            Animation::Expression(_) => return false,
        };
        let keys: Vec<_> = keys
            .into_iter()
            .map(|k| crate::anim::Keyframe {
                value: f64::from(whole(k.value)),
                ..k.to_hold()
            })
            .collect();
        if let Some(first) = keys.first() {
            *seed = whole(first.value);
        }
        self.seed_keys = (!keys.is_empty()).then(|| Property {
            animation: Animation::Keyframed(keys),
            extra: serde_json::Map::new(),
        });
        true
    }
}

impl EffectInstance {
    /// The seed row `id` holds at layer time `lt`.
    #[must_use]
    pub fn seed_at(&self, id: &str, lt: f64) -> Option<u32> {
        self.params.iter().find(|p| p.id == id)?.seed_at(lt)
    }
}

/// The nearest seed: the cast saturates, so a negative or NaN gives 0.
fn whole(v: f64) -> u32 {
    v.round() as u32
}
