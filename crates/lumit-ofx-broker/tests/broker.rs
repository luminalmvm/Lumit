//! What the broker is for: the plugin dies and the session does not.
//!
//! These are the tests that can only be written against a real second process —
//! a plugin that crashes, one that never comes back, one that shouts, and one
//! that wants eleven frames at once. The plugin is told to misbehave through the
//! broker's environment, because a plugin in another process cannot be reached
//! any other way.
//!
//! They live in this crate rather than in `lumit-ofx` for one flat Cargo reason:
//! `CARGO_BIN_EXE_lumit-ofx-broker` exists only inside the package that owns the
//! binary, and Cargo does not build a dependency's binaries.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use lumit_ofx::bundle::BUNDLE_ARCH_DIR;
use lumit_ofx::image::Frame16;
use lumit_ofx::instance::ParamSnapshot;
use lumit_ofx::ipc::shm::Ring;
use lumit_ofx::props::PropValue;
use lumit_ofx::render::RenderRequest;
use lumit_ofx::{Broker, BrokerConfig, BrokerError, Context, RectI};

/// The plugin this whole file is about.
const PASSTHROUGH: &str = "com.lumitlab.testplug.passthrough";

/// The frame size every test works at. Small on purpose: the ring is sized from
/// it, and a test that allocated a 4K ring per case would be measuring the
/// allocator.
const FRAME: (usize, usize) = (8, 8);

// ------------------------------------------------------------ the scaffold --

/// The test plugin's file name on this platform.
fn test_plugin_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "lumit_ofx_testplug.dll"
    } else if cfg!(target_os = "macos") {
        "liblumit_ofx_testplug.dylib"
    } else {
        "liblumit_ofx_testplug.so"
    }
}

/// Where Cargo put the test plugin, if it built it.
fn test_plugin() -> Option<PathBuf> {
    let name = test_plugin_file_name();
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..3 {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        let in_deps = dir.join("deps").join(name);
        if in_deps.is_file() {
            return Some(in_deps);
        }
        dir = dir.parent()?;
    }
    None
}

/// Lay the test plugin out as a real bundle, and answer with the binary inside
/// it. `None` means the plugin was not built and the caller skips.
fn a_bundle_in(root: &Path) -> Option<PathBuf> {
    let source = test_plugin()?;
    let dir = root
        .join("Test.ofx.bundle")
        .join("Contents")
        .join(BUNDLE_ARCH_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    let binary = dir.join("test.ofx");
    std::fs::copy(&source, &binary).ok()?;
    Some(binary)
}

/// Say why a test did nothing, by name, so a skip is never silent.
fn skipped(test: &str) {
    eprintln!(
        "{test}: skipped — {} was not found in the target directory. \
         Build it first: cargo build -p lumit-ofx-testplug",
        test_plugin_file_name()
    );
}

/// A broker over the test bundle, with short deadlines and whatever
/// environment the case wants the plugin to misbehave under.
fn a_broker(root: &Path, env: &[(&str, &str)]) -> Option<(Broker, u32)> {
    let binary = a_bundle_in(root)?;
    let mut config = BrokerConfig::new(binary, FRAME);
    // The shipped deadlines are ten seconds and two (docs/12 §2.3). A test that
    // waited them out three times over would take a minute; these are the same
    // numbers a quirks-table entry writes, which is the point — the override is
    // the mechanism, not a test hook.
    config.quirks.render_timeout = Duration::from_millis(400);
    config.quirks.control_timeout = Duration::from_secs(5);
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-ofx-broker")));
    config.env = env
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();

    let mut broker = Broker::spawn(config).expect("a broker");
    broker.describe().expect("a description");
    let index = broker
        .descriptors()
        .iter()
        .position(|descriptor| descriptor.identifier == PASSTHROUGH)
        .expect("the passthrough plugin");
    Some((broker, index as u32))
}

/// A frame every pixel of which is the same value, so a test can say what came
/// back in one number.
fn a_flat_frame(value: f32) -> Frame16 {
    let pixels = vec![value; FRAME.0 * FRAME.1 * 4];
    Frame16::from_f32(FRAME.0, FRAME.1, &pixels).expect("a frame")
}

/// The first channel of the first pixel.
fn first(frame: &Frame16) -> f32 {
    frame.pixel(0, 0)[0]
}

// ---------------------------------------------------------------- the tests --

#[test]
fn a_bundle_describes_itself_through_a_broker() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((broker, _)) = a_broker(root.path(), &[]) else {
        skipped("a_bundle_describes_itself_through_a_broker");
        return;
    };
    // The whole bundle came back across the pipe, descriptor by descriptor,
    // with nothing of the plugin in this process.
    assert!(broker
        .descriptors()
        .iter()
        .any(|descriptor| descriptor.identifier == PASSTHROUGH));
    assert!(!broker.is_disabled());
}

#[test]
fn a_crash_on_a_frame_restarts_the_broker_and_the_session_carries_on() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((mut broker, plugin)) =
        a_broker(root.path(), &[("LUMIT_TESTPLUG_CRASH_ON_FRAME", "100")])
    else {
        skipped("a_crash_on_a_frame_restarts_the_broker_and_the_session_carries_on");
        return;
    };
    let instance = broker
        .create_instance(plugin, Context::Filter, ParamSnapshot::new())
        .expect("an instance");

    let source = a_flat_frame(0.25);
    let request = RenderRequest::filter(100.0, source.clone());
    let rendered = broker
        .render(instance, &request, &|_, _| None)
        .expect("a frame back");

    // The Gate-4 demo (docs/16): the frame is lost, and nothing else is.
    assert!(rendered.errored, "the dead frame must say so");
    assert_eq!(
        first(&rendered.frame),
        first(&source),
        "a dead render answers with its own input"
    );
    assert!(!broker.is_disabled(), "one crash is not three");
    assert!(
        broker.restarts() >= 1,
        "the broker must have been restarted"
    );

    // And the session carries on: the next frame renders for real, through a
    // process that did not exist a moment ago, with an instance nobody had to
    // rebuild by hand.
    let next = RenderRequest::filter(101.0, a_flat_frame(0.5));
    let rendered = broker
        .render(instance, &next, &|_, _| None)
        .expect("a frame back");
    assert!(!rendered.errored, "the frame after the crash is a real one");
    assert!((first(&rendered.frame) - 0.5).abs() < 1e-2);
}

#[test]
fn a_hang_trips_the_deadline_and_the_third_strike_disables_the_plugin() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((mut broker, plugin)) = a_broker(root.path(), &[("LUMIT_TESTPLUG_HANG", "1")]) else {
        skipped("a_hang_trips_the_deadline_and_the_third_strike_disables_the_plugin");
        return;
    };
    let instance = broker
        .create_instance(plugin, Context::Filter, ParamSnapshot::new())
        .expect("an instance");

    let source = a_flat_frame(0.75);
    for attempt in 1..=3 {
        let request = RenderRequest::filter(f64::from(attempt), source.clone());
        let rendered = broker
            .render(instance, &request, &|_, _| None)
            .expect("a frame back");
        assert!(rendered.errored, "a render that never came back is errored");
        assert_eq!(first(&rendered.frame), first(&source));
    }

    assert!(
        broker.is_disabled(),
        "three consecutive failures put the plugin away for the session"
    );
    // And it stays away: the fourth attempt does not start a fourth process.
    let restarts = broker.restarts();
    let rendered = broker
        .render(
            instance,
            &RenderRequest::filter(4.0, source.clone()),
            &|_, _| None,
        )
        .expect("a frame back");
    assert!(rendered.errored);
    assert_eq!(
        broker.restarts(),
        restarts,
        "a disabled plugin is not retried"
    );
}

#[test]
fn eleven_frames_cross_in_one_shipment() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((mut broker, plugin)) = a_broker(root.path(), &[("LUMIT_TESTPLUG_TEMPORAL", "5")])
    else {
        skipped("eleven_frames_cross_in_one_shipment");
        return;
    };
    let instance = broker
        .create_instance(plugin, Context::Filter, ParamSnapshot::new())
        .expect("an instance");

    // Every frame carries its own time, divided down into the working range, so
    // the mean of eleven of them is a number only all eleven can produce.
    let value_at = |time: f64| (time / 100.0) as f32;
    let request = RenderRequest::filter(20.0, a_flat_frame(value_at(20.0)));
    let rendered = broker
        .render(instance, &request, &|clip, time| {
            (clip == "Source").then(|| a_flat_frame(value_at(time)))
        })
        .expect("a frame back");

    assert!(!rendered.errored);
    assert_eq!(
        broker.shipments(),
        1,
        "a retimer's eleven frames go across in one shipment, not eleven"
    );
    // The mean of 0.15 … 0.25 is 0.20, and it is only 0.20 if every one of the
    // eleven arrived: the plugin divides by what it asked for, not by what it
    // was given.
    assert!(
        (first(&rendered.frame) - 0.20).abs() < 1e-2,
        "expected the mean of eleven frames, got {}",
        first(&rendered.frame)
    );
    assert_eq!(
        rendered.frames_needed.get("Source").copied(),
        Some((15.0, 25.0)),
        "and the declaration itself comes back for the graph's temporal edges"
    );
}

#[test]
fn a_frame_crosses_the_ring_unchanged() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let path = root.path().join("ring");
    let mut written = Ring::create(&path, FRAME.0, FRAME.1).expect("a ring");
    assert!(
        written.slots() >= 3,
        "the note's triple buffering is the floor"
    );

    // A frame with every pixel different, so a slot written half way or a row
    // written twice cannot pass.
    let pixels: Vec<f32> = (0..FRAME.0 * FRAME.1 * 4)
        .map(|index| index as f32 / 64.0)
        .collect();
    let frame = Frame16::from_f32(FRAME.0, FRAME.1, &pixels).expect("a frame");
    let bounds = RectI::sized(FRAME.0 as i32, FRAME.1 as i32);

    let header = written
        .write_frame(2, &frame, bounds, true)
        .expect("written");
    assert_eq!(header.bounds, bounds);
    assert!(header.premultiplied);

    // Read through a *second* mapping of the same block, which is what the
    // broker process has.
    let read = Ring::open(written.spec()).expect("the same ring");
    let (seen, back) = read.read_frame(2).expect("read back");
    assert_eq!(seen, header, "the header crosses whole");
    assert_eq!(back, frame, "and so do the pixels");

    // A slot nobody wrote is empty rather than plausible.
    assert!(read.read_frame(1).is_err());
}

#[test]
fn a_broker_that_speaks_another_protocol_is_refused() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some(binary) = a_bundle_in(root.path()) else {
        skipped("a_broker_that_speaks_another_protocol_is_refused");
        return;
    };
    let mut config = BrokerConfig::new(binary, FRAME);
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-ofx-broker")));
    config.env = vec![("LUMIT_OFX_BROKER_PROTOCOL".to_owned(), "99".to_owned())];

    match Broker::spawn(config) {
        Err(BrokerError::ProtocolMismatch { theirs, ours }) => {
            assert_eq!(theirs, 99);
            assert_eq!(ours, lumit_ofx::ipc::proto::PROTOCOL_VERSION);
        }
        Err(other) => panic!("expected a version refusal, got {other}"),
        Ok(_) => panic!("a broker of another version must be refused, not believed"),
    }
}

#[test]
fn a_plugin_that_will_not_stop_talking_does_not_fill_the_host() {
    let Ok(root) = tempfile::tempdir() else {
        return;
    };
    let Some((mut broker, plugin)) =
        a_broker(root.path(), &[("LUMIT_TESTPLUG_MESSAGE_SPAM", "500")])
    else {
        skipped("a_plugin_that_will_not_stop_talking_does_not_fill_the_host");
        return;
    };
    let instance = broker
        .create_instance(plugin, Context::Filter, ParamSnapshot::new())
        .expect("an instance");

    let source = a_flat_frame(0.5);
    let rendered = broker
        .render(
            instance,
            &RenderRequest::filter(1.0, source.clone()),
            &|_, _| None,
        )
        .expect("a frame back");

    // Five hundred messages, and the frame still rendered.
    assert!(!rendered.errored);
    assert!((first(&rendered.frame) - 0.5).abs() < 1e-2);
    assert!(
        broker.notes().len() <= lumit_ofx::ipc::broker::MAX_NOTES,
        "the host keeps the last few messages, not all of them"
    );
    assert!(
        !broker.notes().is_empty(),
        "and it does keep some: the message suite has to carry"
    );
}

/// A press crosses the pipe with its frame, waits for the plugin however long
/// it takes, and comes back with every value the plugin holds, its own writes
/// included. That is the whole road a look built in a plugin's window takes.
#[test]
fn a_press_comes_back_with_what_the_plugin_wrote() {
    let root = tempfile::tempdir().expect("a temp dir");
    let Some((mut broker, _)) = a_broker(root.path(), &[]) else {
        skipped("a_press_comes_back_with_what_the_plugin_wrote");
        return;
    };
    let plugin = broker
        .descriptors()
        .iter()
        .position(|descriptor| descriptor.identifier == "com.lumitlab.testplug")
        .expect("the test plugin");
    let instance = broker
        .create_instance(plugin as u32, Context::Filter, ParamSnapshot::new())
        .expect("an instance");

    let params = broker
        .press(instance, "trigger", 0.0, &a_flat_frame(0.5))
        .expect("the press came back");
    assert_eq!(
        params.get("gain"),
        Some(&PropValue::double(lumit_ofx_testplug::TRIGGERED_GAIN))
    );
    assert_eq!(
        params.get("vendorBlob"),
        Some(&PropValue::String(vec![
            lumit_ofx_testplug::TRIGGERED_BLOB.to_owned()
        ]))
    );
    assert_eq!(
        broker.strikes(),
        0,
        "a press that came back is not a strike"
    );
}
