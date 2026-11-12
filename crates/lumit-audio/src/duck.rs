//! Lower behind: one node's sound pulled down while another is playing
//! (docs/09-AUDIO.md).
//!
//! The ducked node follows its key node's level after the key's fader. While
//! the key is over the threshold the ducked node's gain moves toward the
//! depth; once the key has been under it for the hold, the gain moves back
//! up. Attack and release are time constants, so the move is smooth and never
//! a step.
//!
//! Runs inside [`crate::plan::MixPlan::mix_block`], so on whichever thread is
//! mixing: the audio callback, the rack thread or an export. No allocation,
//! no locks: the follower is a pair of numbers each mix keeps per node.

/// How many frames the key's level is read over at a time: under a
/// millisecond at any rate, and short enough that a hit is not missed.
pub const DUCK_STEP: usize = 32;

/// A node lowered while another plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Duck {
    /// The node listened to. Earlier in the plan than the ducked node.
    pub key: u16,
    /// The gain the ducked node is brought down to, linear (0.25 is about
    /// -12 dB).
    pub depth: f32,
    /// The key's level, linear, over which the node is lowered.
    pub threshold: f32,
    /// The time constant of the move down, in frames.
    pub attack_frames: u32,
    /// How long the key has to stay under the threshold before the move back
    /// up starts, in frames.
    pub hold_frames: u32,
    /// The time constant of the move back up, in frames.
    pub release_frames: u32,
}

/// Where one ducked node's gain stands between blocks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follower {
    /// The gain now, linear.
    pub gain: f32,
    /// How many frames the key has been under the threshold.
    pub under: u32,
}

impl Default for Follower {
    fn default() -> Self {
        Self {
            gain: 1.0,
            under: u32::MAX,
        }
    }
}

/// Lower `sig` (interleaved stereo) by `duck`, following `key` over the same
/// frames. Answers the smallest gain used, for the meter's gain reduction.
pub fn apply(duck: &Duck, follower: &mut Follower, key: &[f32], sig: &mut [f32]) -> f32 {
    let mut lowest = follower.gain;
    for (k, s) in key.chunks(DUCK_STEP * 2).zip(sig.chunks_mut(DUCK_STEP * 2)) {
        let frames = s.len() / 2;
        let level = k.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let from = follower.gain;
        let target = if level > duck.threshold {
            follower.under = 0;
            duck.depth
        } else {
            follower.under = follower.under.saturating_add(frames as u32);
            if follower.under >= duck.hold_frames {
                1.0
            } else {
                from
            }
        };
        let tau = if target < from {
            duck.attack_frames
        } else {
            duck.release_frames
        };
        let to = if tau == 0 {
            target
        } else {
            from + (target - from) * (1.0 - (-(frames as f32) / tau as f32).exp())
        };
        // Across the step from where it was to where it is going.
        for (n, frame) in s.chunks_exact_mut(2).enumerate() {
            let g = from + (to - from) * (n + 1) as f32 / frames as f32;
            frame[0] *= g;
            frame[1] *= g;
        }
        follower.gain = to;
        lowest = lowest.min(to);
    }
    lowest
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A burst on the key brings a steady tone down to the depth after the
    /// attack, holds it there while the key plays and for the hold after, and
    /// lets it back up over the release. A key under the threshold leaves
    /// the tone alone, sample for sample.
    #[test]
    fn a_key_burst_lowers_a_tone_and_lets_it_back() {
        let duck = Duck {
            key: 0,
            depth: 0.25,
            threshold: 0.1,
            attack_frames: 480,
            hold_frames: 4_800,
            release_frames: 4_800,
        };
        let run = |key_level: f32| {
            let mut follower = Follower::default();
            let mut heard = Vec::new();
            for block in 0..100usize {
                let at = block * 480;
                let key: Vec<f32> = (at..at + 480)
                    .flat_map(|f| {
                        let v = if (4_800..14_400).contains(&f) {
                            key_level
                        } else {
                            0.0
                        };
                        [v, v]
                    })
                    .collect();
                let mut sig = vec![0.5f32; 960];
                apply(&duck, &mut follower, &key, &mut sig);
                heard.extend(sig.chunks_exact(2).map(|f| f[0]));
            }
            heard
        };
        let heard = run(0.5);
        assert_eq!(heard[4_000], 0.5, "nothing before the key");
        let near = |got: f32, want: f32| (got - want).abs() < want * 0.02;
        assert!(
            near(heard[9_600], 0.125),
            "down after the attack: {}",
            heard[9_600]
        );
        assert!(near(heard[19_000], 0.125), "held: {}", heard[19_000]);
        assert!(
            near(heard[43_500], 0.5),
            "back after the release: {}",
            heard[43_500]
        );

        let quiet = run(0.05);
        assert!(
            quiet.iter().all(|v| *v == 0.5),
            "a quiet key lowers nothing"
        );
    }
}
