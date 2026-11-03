//! A Light layer's rows: reading and setting them, and a fresh light's.

use super::{LightDef, LightFalloff, LightKind, LightSettings, Property, TransformProp};

impl LightDef {
    pub(super) fn white() -> [Property; 3] {
        [
            Property::fixed(1.0),
            Property::fixed(1.0),
            Property::fixed(1.0),
        ]
    }
    pub(super) fn zero_pair() -> [Property; 2] {
        [Property::zero(), Property::zero()]
    }
    pub(super) fn zero3() -> [Property; 3] {
        [Property::zero(), Property::zero(), Property::zero()]
    }
    pub(super) fn ten() -> Property {
        Property::fixed(10.0)
    }
    pub(super) fn ninety() -> Property {
        Property::fixed(90.0)
    }
    pub(super) fn hundred() -> Property {
        Property::fixed(100.0)
    }

    /// The light's own channel that `prop` names, or `None` for one it does
    /// not have.
    #[must_use]
    pub fn channel(&self, prop: TransformProp) -> Option<&Property> {
        Some(match prop {
            TransformProp::LightIntensity => &self.intensity,
            TransformProp::LightColourR => &self.colour[0],
            TransformProp::LightColourG => &self.colour[1],
            TransformProp::LightColourB => &self.colour[2],
            TransformProp::ConeAngle => &self.cone_angle,
            TransformProp::ConeFeather => &self.cone_feather,
            TransformProp::LightRadius => &self.radius_px,
            TransformProp::FalloffDistance => &self.falloff_px,
            TransformProp::LightWidth => &self.size[0],
            TransformProp::LightHeight => &self.size[1],
            TransformProp::PoiX => &self.point_of_interest[0],
            TransformProp::PoiY => &self.point_of_interest[1],
            TransformProp::PoiZ => &self.point_of_interest[2],
            TransformProp::ShadowDarkness => &self.shadow_darkness,
            TransformProp::ShadowDiffusion => &self.shadow_diffusion,
            _ => return self.environment.channel(prop).or(self.haze.channel(prop)),
        })
    }

    pub fn channel_mut(&mut self, prop: TransformProp) -> Option<&mut Property> {
        Some(match prop {
            TransformProp::LightIntensity => &mut self.intensity,
            TransformProp::LightColourR => &mut self.colour[0],
            TransformProp::LightColourG => &mut self.colour[1],
            TransformProp::LightColourB => &mut self.colour[2],
            TransformProp::ConeAngle => &mut self.cone_angle,
            TransformProp::ConeFeather => &mut self.cone_feather,
            TransformProp::LightRadius => &mut self.radius_px,
            TransformProp::FalloffDistance => &mut self.falloff_px,
            TransformProp::LightWidth => &mut self.size[0],
            TransformProp::LightHeight => &mut self.size[1],
            TransformProp::PoiX => &mut self.point_of_interest[0],
            TransformProp::PoiY => &mut self.point_of_interest[1],
            TransformProp::PoiZ => &mut self.point_of_interest[2],
            TransformProp::ShadowDarkness => &mut self.shadow_darkness,
            TransformProp::ShadowDiffusion => &mut self.shadow_diffusion,
            TransformProp::BackgroundBlur => &mut self.environment.blur,
            _ => return self.haze.channel_mut(prop),
        })
    }

    /// The part the settings dialog edits as one op.
    #[must_use]
    pub fn settings(&self) -> LightSettings {
        LightSettings {
            kind: self.kind,
            falloff: self.falloff,
            two_node: self.two_node,
            legacy_shape: self.legacy_shape,
            casts_shadows: self.casts_shadows,
            environment: self.environment.settings(),
        }
    }

    pub fn set_settings(&mut self, s: LightSettings) {
        self.kind = s.kind;
        self.falloff = s.falloff;
        self.two_node = s.two_node;
        self.legacy_shape = s.legacy_shape;
        self.casts_shadows = s.casts_shadows;
        self.environment.set_settings(s.environment);
    }
}

/// A fresh light, on After Effects' defaults. Not aimed, since a point of
/// interest needs a composition to sit in: whoever places one sets both.
impl Default for LightDef {
    fn default() -> Self {
        Self {
            kind: LightKind::default(),
            colour: Self::white(),
            intensity: Self::hundred(),
            size: Self::zero_pair(),
            cone_angle: Self::ninety(),
            cone_feather: Property::fixed(50.0),
            falloff: LightFalloff::None,
            radius_px: Property::fixed(500.0),
            falloff_px: Property::fixed(500.0),
            two_node: false,
            point_of_interest: Self::zero3(),
            casts_shadows: false,
            shadow_darkness: Self::hundred(),
            shadow_diffusion: Property::zero(),
            legacy_shape: false,
            haze: crate::haze::LightHaze::default(),
            environment: crate::environment::LightEnvironment::default(),
        }
    }
}
