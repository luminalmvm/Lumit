//! A layer's Material options: what layers had before them, reading and
//! writing their rows, and a new layer's.

use super::{Material, MaterialSettings, ShadowMode, TransformProp};
use crate::anim::Property;

impl Material {
    /// What layers had before materials: no highlight at all.
    #[must_use]
    pub fn legacy() -> Self {
        Self {
            specular: Property::zero(),
            ..Self::default()
        }
    }

    pub(super) fn legacy_boxed() -> Box<Self> {
        Box::new(Self::legacy())
    }

    // serde hands the field over as it is stored, box and all.
    #[allow(clippy::borrowed_box)]
    pub(super) fn is_legacy(m: &Box<Self>) -> bool {
        **m == Self::legacy()
    }

    #[must_use]
    pub fn get(&self, prop: TransformProp) -> Option<&Property> {
        Some(match prop {
            TransformProp::Ambient => &self.ambient,
            TransformProp::Diffuse => &self.diffuse,
            TransformProp::Specular => &self.specular,
            TransformProp::Shininess => &self.shininess,
            TransformProp::Metal => &self.metal,
            TransformProp::LightTransmission => &self.light_transmission,
            TransformProp::Reflection => &self.reflection,
            _ => return None,
        })
    }

    #[must_use]
    pub fn settings(&self) -> MaterialSettings {
        MaterialSettings {
            casts_shadows: self.casts_shadows,
            accepts_shadows: self.accepts_shadows,
            holdout: self.holdout,
        }
    }

    /// Take `s`, handing back what it had.
    pub fn set_settings(&mut self, s: MaterialSettings) -> MaterialSettings {
        let previous = self.settings();
        (self.casts_shadows, self.accepts_shadows) = (s.casts_shadows, s.accepts_shadows);
        self.holdout = s.holdout;
        previous
    }

    pub fn get_mut(&mut self, prop: TransformProp) -> Option<&mut Property> {
        Some(match prop {
            TransformProp::Ambient => &mut self.ambient,
            TransformProp::Diffuse => &mut self.diffuse,
            TransformProp::Specular => &mut self.specular,
            TransformProp::Shininess => &mut self.shininess,
            TransformProp::Metal => &mut self.metal,
            TransformProp::LightTransmission => &mut self.light_transmission,
            TransformProp::Reflection => &mut self.reflection,
            _ => return None,
        })
    }
}

/// A new layer's material, on After Effects' defaults.
impl Default for Material {
    fn default() -> Self {
        Self {
            ambient: Property::fixed(100.0),
            diffuse: Property::fixed(50.0),
            specular: Property::fixed(50.0),
            shininess: Property::fixed(5.0),
            metal: Property::fixed(100.0),
            casts_shadows: ShadowMode::Off,
            accepts_shadows: ShadowMode::On,
            light_transmission: Property::zero(),
            reflection: Property::zero(),
            holdout: false,
            geometry: Vec::new(),
        }
    }
}
