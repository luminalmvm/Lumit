//! The master's loudness, for the desk to read and to start afresh.

use super::{lock, Device};
use lumit_audio::meter::LoudnessReading;

/// The master's loudness as the callback last published it. With no device
/// there is nothing measured.
pub(crate) fn loudness() -> LoudnessReading {
    let st = lock();
    match &st.device {
        Device::Ready { meters, .. } => meters.loudness(),
        _ => LoudnessReading {
            short_term: f32::NEG_INFINITY,
            integrated: f32::NEG_INFINITY,
            true_peak: 0.0,
        },
    }
}

/// Start the integrated loudness and the true peak afresh.
pub(crate) fn reset_loudness() {
    let st = lock();
    if let Device::Ready { meters, .. } = &st.device {
        meters.reset_loudness();
    }
}
