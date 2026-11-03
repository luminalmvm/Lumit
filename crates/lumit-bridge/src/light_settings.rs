//! A Light layer's settings across the bridge, to and from the
//! project's own.

use crate::api::layer::{BridgeLightFalloff, BridgeLightKind, BridgeLightSettings};

impl BridgeLightSettings {
    pub(crate) fn of(s: lumit_core::model::LightSettings) -> Self {
        use lumit_core::model::{LightFalloff as F, LightKind as K};
        Self {
            kind: match s.kind {
                K::Point => BridgeLightKind::Point,
                K::Spot => BridgeLightKind::Spot,
                K::Area => BridgeLightKind::Area,
                K::Parallel => BridgeLightKind::Parallel,
                K::Ambient => BridgeLightKind::Ambient,
                K::Environment => BridgeLightKind::Environment,
            },
            falloff: match s.falloff {
                F::None => BridgeLightFalloff::None,
                F::Smooth => BridgeLightFalloff::Smooth,
                F::InverseSquareClamped => BridgeLightFalloff::InverseSquareClamped,
            },
            two_node: s.two_node,
            casts_shadows: s.casts_shadows,
            in_air: false,
            noisy: false,
            source: s.environment.source,
            background: s.environment.background,
        }
    }

    /// `legacy_shape` is the light's own, which the window never shows.
    pub(crate) fn core(self, legacy_shape: bool) -> lumit_core::model::LightSettings {
        use lumit_core::model::{LightFalloff as F, LightKind as K};
        lumit_core::model::LightSettings {
            kind: match self.kind {
                BridgeLightKind::Point => K::Point,
                BridgeLightKind::Spot => K::Spot,
                BridgeLightKind::Area => K::Area,
                BridgeLightKind::Parallel => K::Parallel,
                BridgeLightKind::Ambient => K::Ambient,
                BridgeLightKind::Environment => K::Environment,
            },
            falloff: match self.falloff {
                BridgeLightFalloff::None => F::None,
                BridgeLightFalloff::Smooth => F::Smooth,
                BridgeLightFalloff::InverseSquareClamped => F::InverseSquareClamped,
            },
            two_node: self.two_node,
            legacy_shape,
            casts_shadows: self.casts_shadows,
            environment: lumit_core::environment::EnvironmentSettings {
                source: self.source,
                background: self.background,
            },
        }
    }
}
