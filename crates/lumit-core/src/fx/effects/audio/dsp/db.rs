//! Decibels to a multiplier and back, with the fader's silence knee.
//!
//! # In plain terms
//!
//! Every audio row that reads in dB becomes a multiplier here, and one rule
//! decides where silence starts. The rule is the mixer's own: at or below
//! -100 dB the answer is exact zero, not a denormal whisper that costs cycles
//! for ever. `lumit-audio` carries the same knee for the Volume property, and
//! this crate sits under it in the dependency order, so the rule is stated
//! again here rather than reached up for (docs/05-ARCHITECTURE.md).

/// Where a dB row means silence, the same -inf point the Volume property has
/// (docs/09 §6).
pub const SILENCE_FLOOR_DB: f64 = -100.0;

/// dB to a linear gain. 0 dB is unity, +6 dB is about twice, and anything at
/// or under [`SILENCE_FLOOR_DB`] is exact zero.
#[must_use]
pub fn gain_of_db(db: f64) -> f64 {
    if db <= SILENCE_FLOOR_DB {
        0.0
    } else {
        10f64.powf(db / 20.0)
    }
}

/// A linear gain back to dB, floored at [`SILENCE_FLOOR_DB`] so a meter over
/// a silent block reads a number rather than -inf. A negative gain is a
/// polarity flip, and its level is its size.
#[must_use]
pub fn db_of_gain(gain: f64) -> f64 {
    let gain = gain.abs();
    if gain <= 0.0 {
        SILENCE_FLOOR_DB
    } else {
        (20.0 * gain.log10()).max(SILENCE_FLOOR_DB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_maps_to_the_gains_a_fader_promises() {
        assert!((gain_of_db(0.0) - 1.0).abs() < 1e-12);
        assert!((gain_of_db(20.0) - 10.0).abs() < 1e-9);
        assert!((gain_of_db(-6.0) - 0.501_187).abs() < 1e-6);
    }

    #[test]
    fn the_knee_is_exact_silence_and_the_step_above_it_is_not() {
        assert_eq!(gain_of_db(SILENCE_FLOOR_DB), 0.0);
        assert_eq!(gain_of_db(-500.0), 0.0);
        assert!(gain_of_db(SILENCE_FLOOR_DB + 0.1) > 0.0);
    }

    #[test]
    fn a_gain_round_trips_through_db_and_answers_the_same_way_twice() {
        for db in [-40.0, -12.0, -0.5, 0.0, 3.0, 24.0] {
            let there = db_of_gain(gain_of_db(db));
            assert!((there - db).abs() < 1e-9, "{db} came back as {there}");
        }
        assert_eq!(db_of_gain(0.0), SILENCE_FLOOR_DB);
        assert_eq!(db_of_gain(-1.0), db_of_gain(1.0));
        assert_eq!(gain_of_db(-3.0), gain_of_db(-3.0));
    }
}
