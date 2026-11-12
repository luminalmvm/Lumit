//! The mixer across the bridge: a layer's gain, automation mode, solo safe
//! and lower behind, and a comp's master fader keys, master automation mode
//! and loudness target ([`lumit_core::mixer`]).
//!
//! A layer's settings ride in on [`crate::api::layer::BridgeLayerInfo`].
//! Every setter is one op, one undo step.

use flutter_rust_bridge::frb;
use lumit_core::anim::{Animation, Property};
use lumit_core::mixer::{AutomationMode, Duck, LoudnessTarget};
use lumit_core::ops::Op;
use lumit_core::time::Rational;
use uuid::Uuid;

use crate::api::effect::BridgeScalar;
use crate::api::layer::LayerReference;
use crate::api::{composition::CompositionReference, BridgeError};

/// How a fader writes its keys while the comp plays.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAutomationMode {
    Read,
    Touch,
    Latch,
    Write,
    Trim,
}

impl BridgeAutomationMode {
    #[frb(ignore)]
    pub(crate) fn of(mode: AutomationMode) -> Self {
        match mode {
            AutomationMode::Read => Self::Read,
            AutomationMode::Touch => Self::Touch,
            AutomationMode::Latch => Self::Latch,
            AutomationMode::Write => Self::Write,
            AutomationMode::Trim => Self::Trim,
        }
    }

    #[frb(ignore)]
    fn read(self) -> AutomationMode {
        match self {
            Self::Read => AutomationMode::Read,
            Self::Touch => AutomationMode::Touch,
            Self::Latch => AutomationMode::Latch,
            Self::Write => AutomationMode::Write,
            Self::Trim => AutomationMode::Trim,
        }
    }
}

/// **Lower behind**: the layer or bus whose sound lowers this layer, and how.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeDuck {
    pub behind: Uuid,
    pub depth_db: f64,
    pub threshold_db: f64,
    pub attack_ms: f64,
    pub hold_ms: f64,
    pub release_ms: f64,
}

impl BridgeDuck {
    #[frb(ignore)]
    pub(crate) fn of(duck: &Duck) -> Self {
        Self {
            behind: duck.behind,
            depth_db: duck.depth_db,
            threshold_db: duck.threshold_db,
            attack_ms: duck.attack_ms,
            hold_ms: duck.hold_ms,
            release_ms: duck.release_ms,
        }
    }

    #[frb(ignore)]
    fn read(&self) -> Result<Duck, BridgeError> {
        let all = [
            self.depth_db,
            self.threshold_db,
            self.attack_ms,
            self.hold_ms,
            self.release_ms,
        ];
        if !all.iter().all(|v| v.is_finite()) {
            return Err(BridgeError::ParamKindMismatch);
        }
        Ok(Duck {
            behind: self.behind,
            depth_db: self.depth_db,
            threshold_db: self.threshold_db,
            attack_ms: self.attack_ms,
            hold_ms: self.hold_ms,
            release_ms: self.release_ms,
        })
    }
}

/// Lower behind `behind` with the usual settings, so the interface never
/// holds its own copy of them.
#[frb(sync)]
pub fn duck_behind(behind: Uuid) -> BridgeDuck {
    BridgeDuck::of(&Duck::behind(behind))
}

/// The loudness a comp's mix is made for, in LUFS, and its highest true
/// peak, in dBTP.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeLoudnessTarget {
    pub lufs: f64,
    pub true_peak_db: f64,
}

impl LayerReference {
    fn commit_mixer(&self, op: Op) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store.commit(op).map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// Set the layer's own level before its rack and Volume, in dB.
    #[frb(sync)]
    pub fn set_audio_gain_db(&self, db: f64) -> Result<(), BridgeError> {
        // Refused rather than written, as a clip's gain is: the save writes a
        // level that is not a number as null, which the reopen will not take.
        if !db.is_finite() {
            return Err(BridgeError::InvalidTime);
        }
        self.commit_mixer(Op::SetLayerAudioGain {
            comp: self.comp_id,
            layer: self.layer_id,
            db,
        })
    }

    #[frb(sync)]
    pub fn set_automation_mode(&self, mode: BridgeAutomationMode) -> Result<(), BridgeError> {
        self.commit_mixer(Op::SetLayerAutomation {
            comp: self.comp_id,
            layer: self.layer_id,
            mode: mode.read(),
        })
    }

    /// A solo safe layer keeps sounding while another layer is soloed.
    #[frb(sync)]
    pub fn set_solo_safe(&self, enabled: bool) -> Result<(), BridgeError> {
        self.commit_mixer(Op::SetLayerSoloSafe {
            comp: self.comp_id,
            layer: self.layer_id,
            solo_safe: enabled,
        })
    }

    /// Lower this layer behind another layer or a bus, or stop, as one
    /// undoable step. Refused, with nothing changed, when the sound would go
    /// round in a circle.
    #[frb(sync)]
    pub fn set_duck(&self, duck: Option<BridgeDuck>) -> Result<(), BridgeError> {
        let duck = duck.map(|d| d.read()).transpose()?;
        self.commit_mixer(Op::SetLayerDuck {
            comp: self.comp_id,
            layer: self.layer_id,
            duck,
        })
    }
}

impl CompositionReference {
    fn commit_mixer(&self, op: Op) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store.commit(op).map_err(BridgeError::OpError)?;
        Ok(())
    }

    /// The master fader: its keys when it has them, in comp time, else its
    /// static level in dB.
    #[frb(sync)]
    pub fn master_volume(&self) -> Result<BridgeScalar, BridgeError> {
        let comp = self.composition()?;
        Ok(match comp.master_keys {
            Some(animation) => BridgeScalar::read_at(
                &Property {
                    animation,
                    ..Property::zero()
                },
                Rational::ZERO,
            ),
            None => BridgeScalar::Static(comp.master_volume_db),
        })
    }

    /// Set the master fader, as one undoable step. A static level clears any
    /// keys; keys leave the static level as it is, for an older Lumit to read.
    #[frb(sync)]
    pub fn set_master_volume(&self, value: BridgeScalar) -> Result<(), BridgeError> {
        let op = match value.animation_at(Rational::ZERO)? {
            Animation::Static(db) if !db.is_finite() => return Err(BridgeError::InvalidTime),
            Animation::Static(db) => {
                let level = Op::SetMasterVolume { comp: self.id, db };
                if self.composition()?.master_keys.is_some() {
                    Op::Batch {
                        ops: vec![
                            Op::SetMasterAutomation {
                                comp: self.id,
                                keys: None,
                            },
                            level,
                        ],
                    }
                } else {
                    level
                }
            }
            keys @ Animation::Keyframed(_) => Op::SetMasterAutomation {
                comp: self.id,
                keys: Some(keys),
            },
            // The master is keyed or still, never driven by an expression.
            Animation::Expression(_) => return Err(BridgeError::ParamKindMismatch),
        };
        self.commit_mixer(op)
    }

    #[frb(sync)]
    pub fn master_automation_mode(&self) -> Result<BridgeAutomationMode, BridgeError> {
        Ok(BridgeAutomationMode::of(
            self.composition()?.master_automation,
        ))
    }

    #[frb(sync)]
    pub fn set_master_automation_mode(
        &self,
        mode: BridgeAutomationMode,
    ) -> Result<(), BridgeError> {
        self.commit_mixer(Op::SetMasterAutomationMode {
            comp: self.id,
            mode: mode.read(),
        })
    }

    #[frb(sync)]
    pub fn loudness_target(&self) -> Result<Option<BridgeLoudnessTarget>, BridgeError> {
        Ok(self
            .composition()?
            .loudness_target
            .map(|t| BridgeLoudnessTarget {
                lufs: t.lufs,
                true_peak_db: t.true_peak_db,
            }))
    }

    #[frb(sync)]
    pub fn set_loudness_target(
        &self,
        target: Option<BridgeLoudnessTarget>,
    ) -> Result<(), BridgeError> {
        if target.is_some_and(|t| !t.lufs.is_finite() || !t.true_peak_db.is_finite()) {
            return Err(BridgeError::ParamKindMismatch);
        }
        self.commit_mixer(Op::SetLoudnessTarget {
            comp: self.id,
            target: target.map(|t| LoudnessTarget {
                lufs: t.lufs,
                true_peak_db: t.true_peak_db,
            }),
        })
    }

    /// Whether `layer` may send or output to `bus` without the sound going
    /// round in a circle, for a menu to ask before it offers the bus.
    #[frb(sync)]
    pub fn route_allowed(&self, layer: Uuid, bus: Uuid) -> bool {
        self.composition()
            .is_ok_and(|comp| lumit_core::routes::route_allowed(&comp, layer, bus))
    }
}
