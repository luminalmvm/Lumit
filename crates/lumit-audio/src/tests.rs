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

#[test]
fn a_chosen_device_resolves_and_a_vanished_one_falls_back() {
    let plugged_in = list(&["Speakers", "Headphones"], Some("Speakers"));
    assert_eq!(
        plugged_in.resolve(Some("Headphones")).as_deref(),
        Some("Headphones")
    );
    assert_eq!(plugged_in.resolve(None).as_deref(), Some("Speakers"));

    let unplugged = list(&["Speakers"], Some("Speakers"));
    assert_eq!(
        unplugged.resolve(Some("Headphones")).as_deref(),
        Some("Speakers"),
        "a device that has gone falls back to the default, calmly"
    );

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

#[test]
fn callback_plays_advances_clock_and_stops_at_end() {
    let shared = Shared::new(Some(plan_of(tone(1000))));
    let mut out = vec![1.0f32; 256 * 2];
    let mut scrub = Scrub::default();
    let mut work = block::MixWork::new(48_000);

    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert!(out.iter().all(|s| *s == 0.0));
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 0);

    shared.playing.store(true, Ordering::Relaxed);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 256);
    assert!((out[0] - 0.0).abs() < 1e-6);
    let late = work.latency() as f32;
    let expected = ((255.0 - late) * 0.05).sin() * 0.25;
    assert!((out[510] - expected).abs() < 1e-5);

    for _ in 0..10 {
        fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    }
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 1000);
    assert!(!shared.playing.load(Ordering::Relaxed));
}

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
    assert!(
        shared.meters.read(meter::MASTER).peak[0] > 0.4,
        "and the meters"
    );
    assert_eq!(scrub.now.at, 1256, "read from where it was asked for");
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 0);
    assert!(!shared.playing.load(Ordering::Relaxed));

    shared.scrub.store(pack_scrub(2000, 600), Ordering::Relaxed);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(scrub.now.at, 2256);
    assert_eq!(scrub.was.left, 0, "the old burst has faded out");
    assert!(out.iter().all(|s| *s <= 0.5 + 1e-6));

    for _ in 0..3 {
        fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    }
    assert!(out.iter().all(|s| *s == 0.0), "a burst is not a loop");
    assert_eq!(shared.meters.read(meter::MASTER).peak[0], 0.0);

    shared.scrub.store(pack_scrub(2000, 600), Ordering::Relaxed);
    shared.playing.store(true, Ordering::Relaxed);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(shared.scrub.load(Ordering::Relaxed), 0);
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 256);
}

#[test]
fn swapping_the_plan_keeps_the_clock_and_changes_the_sound() {
    let shared = Shared::new(Some(plan_of(tone(1000))));
    shared.playing.store(true, Ordering::Relaxed);
    let mut out = vec![0.0f32; 128 * 2];
    let mut scrub = Scrub::default();
    let mut work = block::MixWork::new(48_000);
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    assert_eq!(shared.playhead.load(Ordering::Relaxed), 128);

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
    let late = work.latency() as usize;
    assert!(
        out[late * 2..].iter().all(|s| *s == 0.0),
        "the new plan is heard immediately"
    );
}

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

    while fill_ahead(&shared, RATE) {}
    fill(&shared, &mut scrub, &mut work, None, &mut out, 2);
    let late = work.latency() as usize;
    assert!(
        out[late * 2..] == whole.samples[256 * 2..(512 - late) * 2],
        "the file, in place"
    );
    assert_eq!(shared.starved.load(Ordering::Relaxed), 256, "nothing more");

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
