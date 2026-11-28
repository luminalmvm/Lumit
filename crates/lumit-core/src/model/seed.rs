use super::{EffectInstance, EffectParam, EffectValue};
use crate::anim::{Animation, Property};

impl EffectParam {
    #[must_use]
    pub fn new(id: impl Into<String>, value: EffectValue) -> Self {
        Self {
            id: id.into(),
            value,
            seed_keys: None,
            extra: serde_json::Map::new(),
        }
    }

    #[must_use]
    pub fn keyed_seed(&self) -> Option<&Property> {
        self.seed_keys.as_ref().filter(|keys| keys.is_animated())
    }

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
    #[must_use]
    pub fn seed_at(&self, id: &str, lt: f64) -> Option<u32> {
        self.params.iter().find(|p| p.id == id)?.seed_at(lt)
    }
}

fn whole(v: f64) -> u32 {
    v.round() as u32
}
