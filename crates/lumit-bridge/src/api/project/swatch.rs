//! Turning a shelf colour into the model's swatch and back.

use flutter_rust_bridge::frb;

use super::BridgeSwatch;

impl BridgeSwatch {
    #[frb(ignore)]
    pub(super) fn of(swatch: &lumit_core::model::Swatch) -> Self {
        let [r, g, b, a] = swatch.colour.0;
        Self {
            r,
            g,
            b,
            a,
            name: swatch.name.clone().unwrap_or_default(),
        }
    }

    #[frb(ignore)]
    pub(super) fn to_model(&self) -> lumit_core::model::Swatch {
        lumit_core::model::Swatch {
            colour: lumit_core::model::LinearColour([self.r, self.g, self.b, self.a]),
            name: (!self.name.is_empty()).then(|| self.name.clone()),
        }
    }
}
