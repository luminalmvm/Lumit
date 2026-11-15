//! Reading and writing the puppet across the seam.

use super::*;

impl BridgePuppetPinKind {
    #[frb(ignore)]
    fn read(kind: lumit_core::puppet::PuppetPinKind) -> Self {
        use lumit_core::puppet::PuppetPinKind as K;
        match kind {
            K::Position => Self::Position,
            K::Starch => Self::Starch,
            K::Overlap => Self::Overlap,
            K::Bend => Self::Bend,
        }
    }

    #[frb(ignore)]
    pub(super) fn write(self) -> lumit_core::puppet::PuppetPinKind {
        use lumit_core::puppet::PuppetPinKind as K;
        match self {
            Self::Position => K::Position,
            Self::Starch => K::Starch,
            Self::Overlap => K::Overlap,
            Self::Bend => K::Bend,
        }
    }
}

impl BridgePuppetPin {
    #[frb(ignore)]
    fn read_at(pin: &lumit_core::puppet::PuppetPin, offset: Rational) -> Self {
        Self {
            id: pin.id,
            name: pin.name.clone(),
            kind: BridgePuppetPinKind::read(pin.kind),
            x: BridgeScalar::read_at(&pin.x, offset),
            y: BridgeScalar::read_at(&pin.y, offset),
            rotation: BridgeScalar::read_at(&pin.rotation, offset),
            scale: BridgeScalar::read_at(&pin.scale, offset),
            amount: BridgeScalar::read_at(&pin.amount, offset),
            extent: pin.extent,
        }
    }

    /// The engine's pin this describes. Every channel is converted before any
    /// of them is assigned, so a bad fifth channel cannot leave a pin half
    /// written; the extent is clamped rather than trusted, because a negative
    /// or infinite radius would reach nothing for ever after.
    #[frb(ignore)]
    pub(super) fn write_at(
        &self,
        offset: Rational,
    ) -> Result<lumit_core::puppet::PuppetPin, BridgeError> {
        let x = self.x.animation_at(offset)?;
        let y = self.y.animation_at(offset)?;
        let rotation = self.rotation.animation_at(offset)?;
        let scale = self.scale.animation_at(offset)?;
        let amount = self.amount.animation_at(offset)?;
        let property = |animation| Property {
            animation,
            extra: serde_json::Map::new(),
        };
        Ok(lumit_core::puppet::PuppetPin {
            id: self.id,
            name: self.name.clone(),
            kind: self.kind.write(),
            x: property(x),
            y: property(y),
            rotation: property(rotation),
            scale: property(scale),
            amount: property(amount),
            extent: if self.extent.is_finite() {
                self.extent.clamp(1.0, 10_000.0)
            } else {
                lumit_core::puppet::DEFAULT_EXTENT
            },
            extra: serde_json::Map::new(),
        })
    }
}

impl BridgePuppet {
    #[frb(ignore)]
    pub(super) fn read_at(block: &lumit_core::puppet::PuppetBlock, offset: Rational) -> Self {
        let reference = block
            .reference_time
            .checked_add(offset)
            .unwrap_or(block.reference_time);
        Self {
            reference_time: BridgeRational {
                num: reference.num(),
                den: reference.den(),
            },
            density: block.density,
            expansion: block.expansion,
            pins: block
                .pins
                .iter()
                .map(|p| BridgePuppetPin::read_at(p, offset))
                .collect(),
        }
    }

    /// The engine's block this describes, on the layer's own clock.
    ///
    /// Density and expansion are clamped, not trusted: a density of zero or a
    /// negative expansion is a mesh that refuses to build, and a refusal the
    /// user could not act on is worse than a number quietly kept sane.
    #[frb(ignore)]
    pub(super) fn write_at(
        &self,
        offset: Rational,
    ) -> Result<lumit_core::puppet::PuppetBlock, BridgeError> {
        let reference_time = Rational::new(self.reference_time.num, self.reference_time.den)
            .map_err(|_| BridgeError::InvalidTime)?
            .checked_sub(offset)
            .map_err(|_| BridgeError::InvalidTime)?;
        let mut pins = Vec::with_capacity(self.pins.len());
        for pin in &self.pins {
            pins.push(pin.write_at(offset)?);
        }
        Ok(lumit_core::puppet::PuppetBlock {
            reference_time,
            density: if self.density.is_finite() {
                self.density.clamp(2.0, 500.0)
            } else {
                lumit_core::puppet::DEFAULT_DENSITY
            },
            expansion: if self.expansion.is_finite() {
                self.expansion.clamp(0.0, 100.0)
            } else {
                lumit_core::puppet::DEFAULT_EXPANSION
            },
            pins,
            extra: serde_json::Map::new(),
        })
    }
}
