//! A graphics template's clock on a copy: the intro and the outro keep their
//! speed, and the stretch between them takes up whatever the copy has left.
//!
//! One function, asked by the nested clock of a Precomp layer and of a clip,
//! so the picture, the frame key and the decode plan read the same moment.
//!
//! Runs wherever the document is read.

/// Where a template's intro ends and its outro starts, in seconds of the
/// template's own time, with how long the template runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    /// The moment the intro ends. Zero is no intro.
    pub intro: f64,
    /// The moment the outro starts. The template's length is no outro.
    pub outro: f64,
    /// How long the template runs.
    pub duration: f64,
}

/// Which ends of a template a copy plays. A copy cut in half-way through a
/// scene can skip the intro, and one cut away from can skip the outro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ends {
    /// The copy starts on the intro. Off, it starts where the intro ends.
    pub intro: bool,
    /// The copy ends on the outro. Off, it ends where the outro starts.
    pub outro: bool,
}

impl Ends {
    /// Both ends played, which is every copy nobody has changed.
    pub const BOTH: Ends = Ends {
        intro: true,
        outro: true,
    };
}

/// The moment of the template a copy shows `along` seconds into its own
/// `length`.
///
/// The intro plays at its own speed from the copy's start and the outro at
/// its own speed into the copy's end. The stretch between them is spread over
/// what is left, so a template of five seconds placed for twenty holds its
/// middle for fifteen longer, and one whose middle moves plays it slower.
///
/// A copy too short for both ends plays them faster by the same amount, the
/// outro straight after the intro, and none of the middle.
#[must_use]
pub fn copy_time(timing: Timing, ends: Ends, length: f64, along: f64) -> f64 {
    let duration = timing.duration.max(0.0);
    let intro = timing.intro.clamp(0.0, duration);
    let outro = timing.outro.clamp(intro, duration);
    let first = if ends.intro { 0.0 } else { intro };
    if length.is_nan() || length <= 0.0 {
        return first;
    }
    let along = along.clamp(0.0, length);
    // The last moment, said outright: worked out below, rounding can leave it
    // on the end of the intro.
    if along >= length {
        return if ends.outro { duration } else { outro };
    }
    let head = if ends.intro { intro } else { 0.0 };
    let tail = if ends.outro { duration - outro } else { 0.0 };
    if head + tail >= length {
        if head + tail <= 0.0 {
            return first;
        }
        let faster = (head + tail) / length;
        let cut = head / faster;
        return if along < cut {
            along * faster
        } else {
            outro + (along - cut) * faster
        };
    }
    if along < head {
        along
    } else if along >= length - tail {
        outro + (along - (length - tail))
    } else {
        intro + (along - head) * ((outro - intro) / (length - head - tail))
    }
}

/// [`copy_time`] as a Retime map over a copy that starts `start` seconds into
/// its own clock and runs for `length`: what the mix reads a copy's sound
/// through, so it is heard where the picture is seen. `None` when a time is
/// past what a rational holds.
///
/// A copy too short for both ends is heard straight through from its first
/// moment to its last, where the picture cuts from the intro to the outro.
#[must_use]
pub fn copy_map(
    timing: Timing,
    ends: Ends,
    start: f64,
    length: f64,
) -> Option<crate::anim::Property> {
    use crate::time::Rational;
    if length.is_nan() || length <= 0.0 {
        return None;
    }
    let duration = timing.duration.max(0.0);
    let intro = timing.intro.clamp(0.0, duration);
    let outro = timing.outro.clamp(intro, duration);
    let head = if ends.intro { intro } else { 0.0 };
    let tail = if ends.outro { duration - outro } else { 0.0 };
    let mut along = vec![0.0, length];
    if head + tail < length {
        along = vec![0.0, head, length - tail, length];
    }
    let mut keys: Vec<crate::anim::Keyframe> = Vec::new();
    for at in along {
        let time = Rational::from_f64_on_grid(start + at, Rational::FLICK_DEN).ok()?;
        if keys.last().is_some_and(|k| k.time == time) {
            continue;
        }
        keys.push(crate::anim::Keyframe {
            time,
            value: copy_time(timing, ends, length, at),
            interp_in: crate::anim::SideInterp::Linear,
            interp_out: crate::anim::SideInterp::Linear,
        });
    }
    Some(crate::anim::Property {
        animation: crate::anim::Animation::Keyframed(keys),
        extra: serde_json::Map::new(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Whatever the copy's length, the clock never runs backwards, starts
        /// on the first moment it plays, ends on the last, and the two ends
        /// keep their speed whenever the copy has room for both.
        #[test]
        fn the_ends_keep_their_speed_and_the_clock_runs_forwards(
            duration in 0.5f64..120.0,
            intro_at in 0.0f64..1.0,
            outro_at in 0.0f64..1.0,
            length in 0.05f64..600.0,
            play_intro: bool,
            play_outro: bool,
        ) {
            let intro = duration * intro_at.min(outro_at);
            let outro = duration * intro_at.max(outro_at);
            let timing = Timing { intro, outro, duration };
            let ends = Ends { intro: play_intro, outro: play_outro };
            let at = |along: f64| copy_time(timing, ends, length, along);

            let head = if play_intro { intro } else { 0.0 };
            let tail = if play_outro { duration - outro } else { 0.0 };
            let room = head + tail < length;
            // With no room for the middle a copy that skips the intro
            // starts on the outro.
            let first = if play_intro { 0.0 } else if room { intro } else { outro };
            let last = if play_outro { duration } else { outro };
            prop_assert!((at(0.0) - first).abs() < 1e-9);
            prop_assert!((at(length) - last).abs() < 1e-6);

            let steps = 200;
            let mut before = at(0.0);
            for i in 1..=steps {
                let now = at(length * f64::from(i) / f64::from(steps));
                prop_assert!(now >= before - 1e-9, "the clock ran backwards");
                before = now;
            }

            if room {
                if head > 0.0 {
                    let along = head * 0.5;
                    prop_assert!((at(along) - along).abs() < 1e-9, "the intro changed speed");
                }
                if tail > 0.0 {
                    let from_end = tail * 0.5;
                    prop_assert!(
                        (at(length - from_end) - (duration - from_end)).abs() < 1e-6,
                        "the outro changed speed"
                    );
                }
            }
        }
    }
}
