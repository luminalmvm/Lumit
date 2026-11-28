use super::{EffectParam, EffectValue};

impl EffectParam {
    /// A row holding `value` and nothing beside it.
    #[must_use]
    pub fn new(id: impl Into<String>, value: EffectValue) -> Self {
        Self {
            id: id.into(),
            value,
            extra: serde_json::Map::new(),
        }
    }
}
