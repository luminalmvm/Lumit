//! The mixer's own settings on a layer and a composition: how a fader's
//! automation is written, the level a layer lowers to behind another, and
//! the loudness a mix is made for.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How a fader writes its keys while the comp plays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutomationMode {
    /// The keys play and a fader move writes nothing.
    #[default]
    Read,
    /// Writes while the fader is held, then goes back to the keys.
    Touch,
    /// Writes from the first move until playback stops.
    Latch,
    /// Writes for the whole pass, held or not.
    Write,
    /// A move shifts the keys already there by the change, rather than
    /// replacing them.
    Trim,
}

impl AutomationMode {
    #[must_use]
    pub fn is_read(&self) -> bool {
        *self == Self::Read
    }
}

/// **Lower behind**: the layer's level drops while another layer or a bus is
/// sounding, and comes back when it stops.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Duck {
    /// The layer or Bus layer of the same comp whose sound lowers this one.
    pub behind: Uuid,
    /// How far the level drops, in dB.
    #[serde(default = "Duck::default_depth")]
    pub depth_db: f64,
    /// How loud `behind` has to be before anything drops, in dB.
    #[serde(default = "Duck::default_threshold")]
    pub threshold_db: f64,
    #[serde(default = "Duck::default_attack")]
    pub attack_ms: f64,
    #[serde(default = "Duck::default_hold")]
    pub hold_ms: f64,
    #[serde(default = "Duck::default_release")]
    pub release_ms: f64,
}

impl Duck {
    /// Lower behind `behind` with the usual settings.
    #[must_use]
    pub fn behind(behind: Uuid) -> Self {
        Self {
            behind,
            depth_db: Self::default_depth(),
            threshold_db: Self::default_threshold(),
            attack_ms: Self::default_attack(),
            hold_ms: Self::default_hold(),
            release_ms: Self::default_release(),
        }
    }

    fn default_depth() -> f64 {
        -12.0
    }

    fn default_threshold() -> f64 {
        -36.0
    }

    fn default_attack() -> f64 {
        20.0
    }

    fn default_hold() -> f64 {
        150.0
    }

    fn default_release() -> f64 {
        600.0
    }
}

/// The loudness a comp's mix is made for, and the highest its true peak may
/// reach.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoudnessTarget {
    /// Integrated loudness, in LUFS.
    pub lufs: f64,
    /// In dBTP.
    pub true_peak_db: f64,
}
