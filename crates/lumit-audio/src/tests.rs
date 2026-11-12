use super::*;

fn list(names: &[&str], default: Option<&str>) -> OutputDevices {
    OutputDevices {
        devices: names
            .iter()
            .map(|n| OutputDevice {
                id: (*n).to_owned(),
                name: (*n).to_owned(),
            })
            .collect(),
        default_id: default.map(str::to_owned),
    }
}

/// The whole falling-back rule, without a sound card: a choice that is
/// still plugged in wins, one that has vanished lands on the system
/// default, a machine whose host names no default takes the first output,
/// and a machine with nothing at all says so instead of pretending.
#[test]
fn a_chosen_device_resolves_and_a_vanished_one_falls_back() {
    let plugged_in = list(&["Speakers", "Headphones"], Some("Speakers"));
    assert_eq!(
        plugged_in.resolve(Some("Headphones")).as_deref(),
        Some("Headphones")
    );
    assert_eq!(plugged_in.resolve(None).as_deref(), Some("Speakers"));

    // The headphones were unplugged since the choice was made.
    let unplugged = list(&["Speakers"], Some("Speakers"));
    assert_eq!(
        unplugged.resolve(Some("Headphones")).as_deref(),
        Some("Speakers"),
        "a device that has gone falls back to the default, calmly"
    );

    // A host that names no default (Linux/ALSA), and one that names a
    // default which is itself no longer enumerated.
    assert_eq!(
        list(&["Speakers", "HDMI"], None)
            .resolve(Some("HDMI"))
            .as_deref(),
        Some("HDMI")
    );
    assert_eq!(
        list(&["Speakers", "HDMI"], None).resolve(None).as_deref(),
        Some("Speakers")
    );
    assert_eq!(
        list(&["HDMI"], Some("Speakers")).resolve(None).as_deref(),
        Some("HDMI")
    );

    // Nothing to play through at all.
    assert_eq!(list(&[], None).resolve(Some("Speakers")), None);
    assert_eq!(OutputDevices::default().resolve(None), None);
}

fn tone(frames: usize) -> Arc<AudioBuffer> {
    let mut samples = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let v = (n as f32 * 0.05).sin() * 0.25;
        samples.push(v);
        samples.push(v);
    }
    Arc::new(AudioBuffer {
        rate: 48_000,
        samples,
    })
}

/// The callback contract, exercised directly (no device needed in CI):
/// silence when paused, correct samples when playing (late by the limiter's
/// look ahead), auto-stop at end, and the clock advancing by exactly the
/// frames consumed.
#[test]
fn callback_plays_advances_clock_and_stops_at_end() {
    let shared = Shared::new(Some(plan_of(tone(1000))));
    let mut out = vec![1.0f32; 256 * 2];
    let mut scrub = Scrub::default();
    let mut work = block::MixWork::new(48_000);

    // Paused: silence, clock still.
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert!(out.iter().all(|s| *s == 0.0));
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 0);

    // Playing: exact samples, clock advances by frames written.
    shared.playing.store(true, Ordering::Relaxed);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 256);
    assert!((out[0] - 0.0).abs() < 1e-6); // sin(0)·0.25
    let late = work.latency() as f32;
    let expected = ((255.0 - late) * 0.05).sin() * 0.25;
    assert!((out[510] - expected).abs() < 1e-5);

    // Run past the end: stops exactly at the last frame, playing=false.
    for _ in 0..10 {
        fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    }
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 1000);
    assert!(!shared.playing.load(Ordering::Relaxed));
}

/// The scrub, through the callback: a burst sounds from where it was
/// asked for with the transport stopped and the clock unmoved, rises
/// from nothing so that it cannot click, and ends on its own. A second
/// burst takes over from the first, and pressing play drops the lot.
#[test]
fn a_scrub_burst_sounds_while_stopped_and_leaves_the_clock_alone() {
    let steady = Arc::new(AudioBuffer {
        rate: 48_000,
        samples: vec![0.5; 4000 * 2],
    });
    let shared = Shared::new(Some(plan_of(steady)));
    shared.scrub.store(pack_scrub(1000, 600), Ordering::Relaxed);
    let mut scrub = Scrub::default();
    let mut work = block::MixWork::new(48_000);
    let mut out = vec![0.0f32; 256 * 2];

    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(out[0], 0.0, "the burst starts from nothing");
    assert!((out[255 * 2] - 0.5).abs() < 1e-6, "and reaches the mix");
    assert_eq!(scrub.now.at, 1256, "read from where it was asked for");
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 0);
    assert!(!shared.playing.load(Ordering::Relaxed));

    // A second burst before the first is over: the first fades out
    // under it, and the two together are never louder than the mix.
    shared.scrub.store(pack_scrub(2000, 600), Ordering::Relaxed);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(scrub.now.at, 2256);
    assert_eq!(scrub.was.left, 0, "the old burst has faded out");
    assert!(out.iter().all(|s| *s <= 0.5 + 1e-6));

    // Left alone, it ends, and the buffer after that is silent.
    for _ in 0..3 {
        fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    }
    assert!(out.iter().all(|s| *s == 0.0), "a burst is not a loop");

    // Pressing play drops a burst that was asked for and not yet heard.
    shared.scrub.store(pack_scrub(2000, 600), Ordering::Relaxed);
    shared.playing.store(true, Ordering::Relaxed);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(shared.scrub.load(Ordering::Relaxed), 0);
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 256);
}

/// The instant-edit path: swapping the plan mid-play keeps the clock and
/// the play state, and the very next callback reads the new plan's
/// samples — this is what makes solo/mute/move audible immediately.
#[test]
fn swapping_the_plan_keeps_the_clock_and_changes_the_sound() {
    let shared = Shared::new(Some(plan_of(tone(1000))));
    shared.playing.store(true, Ordering::Relaxed);
    let mut out = vec![0.0f32; 128 * 2];
    let mut scrub = Scrub::default();
    let mut work = block::MixWork::new(48_000);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 128);

    // "Mute": swap in a silent plan (no clips) of the same length.
    *shared.plan.write() = Some(Arc::new(plan::MixPlan {
        total_frames: 1000,
        ..plan::MixPlan::default()
    }));
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(
        shared.playhead.load(Ordering::Relaxed),
        256,
        "the clock kept running across the swap"
    );
    assert!(shared.playing.load(Ordering::Relaxed));
    // Past the old plan's last frames, still in the limiter's look ahead.
    let late = work.latency() as usize;
    assert!(
        out[late * 2..].iter().all(|s| *s == 0.0),
        "the new plan is heard immediately"
    );
}

/// **The callback never waits for a file.** A clip whose block is not
/// decoded plays as silence with the clock running, the shortfall is
/// counted, and once the thread that fills ahead has been round the same
/// callback plays the file. A scrub is handed to the callback only after
/// what it will sound has been decoded.
#[test]
fn a_missing_block_is_silence_that_is_counted_and_then_filled() {
    use stream::tests::{ramp_wav, Scratch};
    const RATE: u32 = 8_000;
    let dir = Scratch::new("engine");
    let path = ramp_wav(&dir.0, "ramp.wav", RATE, 5);
    let whole = lumit_media::audio::decode_all(&path, RATE).unwrap();
    let pool = stream::Pool::new(64 * 1024 * 1024);
    let frames = whole.frames();
    let shared = Shared::new(Some(Arc::new(plan::MixPlan::single(
        vec![plan::PlacedClip {
            source: pool.file(&path, RATE),
            start_frame: 0,
            src_start: 0,
            len: frames,
            gain: [1.0, 1.0],
            envelope: None,
            node: 0,
            silent: false,
        }],
        frames,
    ))));
    shared.playing.store(true, Ordering::Relaxed);
    let mut scrub = Scrub::default();
    let mut work = block::MixWork::new(48_000);
    let mut out = vec![1.0f32; 256 * 2];

    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert!(out.iter().all(|s| *s == 0.0), "nothing decoded: silence");
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 256, "and no wait");
    assert_eq!(shared.starved.load(Ordering::Relaxed), 256);
    assert_eq!(pool.resident_bytes(), 0, "the callback decoded nothing");

    // The filling thread's own loop, stepped by hand.
    while fill_ahead(&shared, RATE) {}
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    // Late by the limiter's look ahead, behind the silence before it.
    let late = work.latency() as usize;
    assert!(
        out[late * 2..] == whole.samples[256 * 2..(512 - late) * 2],
        "the file, in place"
    );
    assert_eq!(shared.starved.load(Ordering::Relaxed), 256, "nothing more");

    // A scrub asked for is not the callback's until it has been filled.
    shared.playing.store(false, Ordering::Relaxed);
    shared
        .scrub_wanted
        .store(pack_scrub(30_000, 400), Ordering::Relaxed);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert!(out.iter().all(|s| *s == 0.0));
    fill_ahead(&shared, RATE);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(
        scrub.now.at, 30_256,
        "the burst sounds from where it was asked"
    );
    assert!((out[255 * 2] - whole.samples[30_255 * 2]).abs() < 1e-6);
}
