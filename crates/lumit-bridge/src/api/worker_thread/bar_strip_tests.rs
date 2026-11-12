//! Tests of the bar strip and the audio chase.

use super::{
    audio_chase, bar_byte, refine_bar_strip, sample_bar_strip, AudioChase, AUDIO_REALTIME_FRAMES,
};
use crate::framecache::bar::pack;

/// **The bar says not just "cached" but "cached at what size"**
/// (docs/15-DESIGN.md §6.3). Each case names one frame's holdings and
/// checks the whole strip byte: the storage state the bar has always
/// drawn, and the preview divisor the picture it found was actually made
/// at.
#[test]
fn a_frames_byte_reports_the_finest_picture_there_is_of_it() {
    // Held at the scale asked about: full, and nothing coarser is probed.
    let mut probes = 0;
    let byte = bar_byte(false, |divisor| {
        probes += 1;
        (divisor == 1, false)
    });
    assert_eq!(byte, pack(2, 1));
    assert_eq!(probes, 1, "the common answer costs one probe");

    // Held only at half and at quarter: half is the better picture, and
    // the walk must say so rather than reporting whichever tier it met
    // last.
    assert_eq!(
        bar_byte(false, |divisor| (divisor == 2 || divisor == 4, false)),
        pack(1, 2)
    );
    // Third is a tier the realtime controller genuinely drops to, so a
    // frame held there is found and named, not rounded to half or quarter.
    assert_eq!(bar_byte(false, |d| (d == 3, false)), pack(1, 3));
    assert_eq!(bar_byte(false, |d| (d == 4, false)), pack(1, 4));

    // Parked, not held: the storage state changes, the tier rule does not.
    assert_eq!(bar_byte(false, |d| (false, d == 1)), pack(4, 1));
    assert_eq!(bar_byte(false, |d| (false, d == 3 || d == 4)), pack(3, 3));

    // Playable outranks promotable: parked at this scale but held coarser
    // reads as held, at the tier it is held at.
    assert_eq!(
        bar_byte(false, |d| (d == 2, d == 1)),
        pack(1, 2),
        "a frame you can play now is not a frame you could promote"
    );

    // Nothing anywhere has no size either, so the byte is a plain zero —
    // which is what the sampler's "is this frame held at all?" test reads.
    assert_eq!(bar_byte(false, |_| (false, false)), 0);
}

/// **The sound gives way once it is in front of the picture, and comes back
/// to the picture once the picture has held the rate.**
///
/// Every-frame playback shows every frame however long each takes, so the
/// picture can fall behind the sound. Lumit stops the sound rather than let
/// it run on in front.
///
/// The rules tried before this one, and what was wrong with each:
///
/// * *Never start again.* The clock stops reporting when the sound stops,
///   so the test that stopped it could not run a second time.
/// * *Wait for the picture to reach the sound.* After a long stall it may
///   never reach it, and does not at all if the composition ends first.
/// * *Start when frames are banked.* Frames are usually banked when a
///   picture goes out, even in a slow run, so the sound started again on
///   the very next picture and stuttered.
/// * *Stop on one late picture.* A picture a few milliseconds late is made
///   up by the next, and stopping for it left holes in a span played from
///   the cache. It also never stopped for a picture running a little slow
///   throughout, which is the case that drifts.
#[test]
fn the_sound_gives_way_in_front_of_the_picture_and_returns_on_the_rate() {
    let long = AUDIO_REALTIME_FRAMES;
    let slack = 0.040;
    // Running, and level with the picture: nothing happens. A picture a
    // little late leaves the sound a little in front, and that is level.
    assert_eq!(
        audio_chase(false, Some(0.0), slack, long),
        AudioChase::Leave
    );
    assert_eq!(audio_chase(false, Some(0.015), slack, 0), AudioChase::Leave);
    // Running, and in front by more than the slack: it stops, whether it
    // got there in one stall or a little at a time.
    assert_eq!(audio_chase(false, Some(0.060), slack, 0), AudioChase::Hold);
    assert_eq!(
        audio_chase(false, Some(0.060), slack, long),
        AudioChase::Hold
    );
    // Running, and behind: it is moved up, not stopped.
    assert_eq!(
        audio_chase(false, Some(-0.2), slack, long),
        AudioChase::Catch
    );
    // No clock to read, as just after a seek: left alone.
    assert_eq!(audio_chase(false, None, slack, 0), AudioChase::Leave);

    // Stopped, and the pictures are still late: it stays stopped.
    assert_eq!(audio_chase(true, None, slack, 0), AudioChase::Leave);
    // Stopped, and a few pictures have gone out on time, but not enough.
    assert_eq!(audio_chase(true, None, slack, 1), AudioChase::Leave);
    assert_eq!(audio_chase(true, None, slack, long - 1), AudioChase::Leave);
    // Stopped, and the rate has held: the sound comes back.
    assert_eq!(audio_chase(true, None, slack, long), AudioChase::Start);
    assert_eq!(audio_chase(true, None, slack, long + 40), AudioChase::Start);
}

/// A long one is sampled: one frame in four is named and stands for the four.
/// The whole stripe therefore has an answer immediately — the alternative is a
/// bar that fills in from one end, which reads as the *cache* filling in from
/// one end.
#[test]
fn a_long_composition_is_sampled_then_refined_to_the_truth() {
    // Frame 4 is the only one held, and it is not a sample point (samples are
    // 0, 4, 8, … with stride 4 — so it IS one here; use 5 instead).
    let held = |frame: u64| frame == 5;
    let tier = move |frame: u64| u8::from(held(frame)) * 2;

    let mut sampled = sample_bar_strip(12, 4, &mut { tier });
    assert_eq!(
        sampled.tiers,
        vec![0; 12],
        "frame 5 is not a sample point, so the coarse pass misses it"
    );
    assert_eq!(sampled.refined_to, 0, "and the strip needs refining");

    // Two turns of four frames each: the truth appears where it belongs, and
    // only there.
    let refined = refine_bar_strip(&mut sampled.tiers, 0, 0, 4, &mut { tier });
    assert_eq!(refined, 4);
    assert_eq!(sampled.tiers, vec![0; 12], "the first four hold nothing");
    let refined = refine_bar_strip(&mut sampled.tiers, 0, refined, 4, &mut { tier });
    assert_eq!(refined, 8);
    assert_eq!(
        sampled.tiers,
        vec![0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0],
        "frame 5 alone, not the run of four it sits in"
    );

    // And the sweep finishes rather than running past the end.
    let refined = refine_bar_strip(&mut sampled.tiers, 0, refined, 100, &mut { tier });
    assert_eq!(refined, 12);
}

/// Degenerate spans do nothing rather than panicking — an empty composition
/// and a zero-length turn both reach here from ordinary interface states.
#[test]
fn empty_strips_and_empty_turns_are_calm() {
    let mut none: Vec<u8> = Vec::new();
    assert_eq!(refine_bar_strip(&mut none, 0, 0, 8, &mut |_| 2), 0);
    let mut some = vec![0u8; 4];
    assert_eq!(refine_bar_strip(&mut some, 0, 0, 0, &mut |_| 2), 0);
    assert_eq!(some, vec![0; 4]);
    assert!(sample_bar_strip(0, 4, &mut |_| 2).tiers.is_empty());
    // A stride of zero would divide by nothing; it is floored to one.
    assert_eq!(sample_bar_strip(2, 0, &mut |_| 2).tiers, vec![2, 2]);
}
