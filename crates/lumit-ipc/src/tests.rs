//! What the transport promises, pinned.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::hosts::{APLUG, LFX, OFX, RESERVED};
use crate::pipe::{self, PipeError};
use crate::rules::{describe_deadline, HANDSHAKE_TIMEOUT, MAX_MESSAGE_BYTES};

/// A message of the shape a host's protocol enum has: a tag and a payload.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Greeting {
    Ready { nonce: u64 },
    Note(String),
}

/// An endpoint name nothing else in this suite will claim.
fn unique_name() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    pipe::pipe_name("lumit-ipc-test", &format!("{pid:x}{serial:04x}"))
}

// ------------------------------------------------------------- identity --

/// The three strings §3.1 parameterises are a host's identity, and two hosts
/// sharing one is the collision the extraction was meant not to cause: one
/// prefix puts both brokers' endpoints in one namespace, and one environment
/// variable lets either host's override redirect the other's child.
#[test]
fn no_two_hosts_share_an_endpoint_prefix_or_a_broker_environment_variable() {
    for (index, one) in RESERVED.iter().enumerate() {
        for other in RESERVED.iter().skip(index + 1) {
            assert_ne!(
                one.prefix, other.prefix,
                "{} and {} reserve the same endpoint prefix",
                one.host, other.host
            );
            assert_ne!(
                one.broker_exe_env, other.broker_exe_env,
                "{} and {} reserve the same broker environment variable",
                one.host, other.host
            );
            assert_ne!(
                one.broker_exe_stem, other.broker_exe_stem,
                "{} and {} reserve the same broker executable",
                one.host, other.host
            );
        }
    }
}

/// The distinctness above is worth exactly what the list is worth. A host whose
/// entry fell out of [`RESERVED`] would be distinct from the rest by being
/// absent, and nothing else would notice: each host's own suite compares its
/// three strings against that host's constant rather than against the list.
#[test]
fn the_reservation_names_every_host_there_is() {
    for host in [OFX, APLUG, LFX] {
        assert!(
            RESERVED.contains(&host),
            "{} is a host the reservation does not list",
            host.host
        );
    }
    assert_eq!(
        RESERVED.len(),
        3,
        "OFX, audio and LFX, and nothing else yet"
    );
}

#[test]
fn an_endpoint_name_carries_the_callers_own_prefix() {
    let name = pipe::pipe_name("lumit-ofx", "deadbeef");
    assert!(
        name.contains("lumit-ofx-deadbeef"),
        "the prefix and the identifier are the name: {name}"
    );
    let tail = if cfg!(windows) { ".pipe" } else { ".sock" };
    assert!(name.ends_with(tail), "{name} should end {tail}");
}

/// The same identifier under two hosts must not be the same endpoint, which is
/// what a hard-coded prefix would have made it.
#[test]
fn two_hosts_with_one_identifier_still_have_two_endpoints() {
    let ofx = pipe::pipe_name("lumit-ofx", "deadbeef");
    let lfx = pipe::pipe_name("lumit-lfx", "deadbeef");
    assert_ne!(ofx, lfx);
}

// ----------------------------------------------------------------- wire --

#[test]
fn a_message_goes_round_the_pipe_whole() {
    let name = unique_name();
    let listener = pipe::listen(&name).expect("a listener");

    let child_name = name.clone();
    let child = std::thread::spawn(move || {
        let stream = pipe::connect(&child_name).expect("the broker connects");
        let (mut receiver, mut sender) = pipe::split(stream);
        pipe::send(&mut sender, &Greeting::Ready { nonce: 7 }).expect("the broker speaks first");
        pipe::recv::<_, Greeting>(&mut receiver).expect("and hears the answer")
    });

    let stream = pipe::accept(&listener).expect("the one connection");
    let (mut receiver, mut sender) = pipe::split(stream);
    let first: Greeting = pipe::recv(&mut receiver).expect("the greeting");
    assert_eq!(first, Greeting::Ready { nonce: 7 });
    pipe::send(&mut sender, &Greeting::Note("go on".to_owned())).expect("the answer");

    let heard = child.join().expect("the broker thread");
    assert_eq!(heard, Greeting::Note("go on".to_owned()));
}

/// A name that is already taken is refused rather than cleared: removing what
/// is there would let a program that planted something at a predicted path have
/// it quietly deleted.
#[test]
fn a_name_already_taken_is_refused_rather_than_cleared() {
    let name = unique_name();
    let _held = pipe::listen(&name).expect("a listener");
    let second = pipe::listen(&name);
    assert!(
        matches!(second, Err(PipeError::Io(_))),
        "the second claim on one name must fail"
    );
}

#[test]
fn a_message_past_the_cap_is_refused_before_it_is_written() {
    let mut written: Vec<u8> = Vec::new();
    let huge = Greeting::Note("x".repeat(MAX_MESSAGE_BYTES + 1));
    let refused = pipe::send(&mut written, &huge);
    assert!(matches!(refused, Err(PipeError::TooLarge(_))));
    assert!(written.is_empty(), "nothing may reach the pipe");
}

/// The length is checked before the buffer is made, so a liar cannot make the
/// host reserve a gigabyte by claiming a gigabyte is coming.
#[test]
fn a_length_prefix_past_the_cap_is_refused_before_a_byte_is_allocated() {
    let claimed = u32::try_from(MAX_MESSAGE_BYTES).expect("the cap fits a prefix") + 1;
    let prefix = claimed.to_le_bytes();
    let mut reader: &[u8] = &prefix;
    let refused = pipe::recv::<_, Greeting>(&mut reader);
    match refused {
        Err(PipeError::TooLarge(length)) => {
            assert_eq!(length, MAX_MESSAGE_BYTES + 1);
        }
        other => panic!("expected the cap to refuse it, got {other:?}"),
    }
}

#[test]
fn a_pipe_that_went_away_is_closed_rather_than_an_unexpected_eof() {
    let mut empty: &[u8] = &[];
    assert!(matches!(
        pipe::recv::<_, Greeting>(&mut empty),
        Err(PipeError::Closed)
    ));

    let half: &[u8] = &[4, 0];
    let mut half = half;
    assert!(
        matches!(pipe::recv::<_, Greeting>(&mut half), Err(PipeError::Closed)),
        "a prefix that stops half way is the other side going, not a parse error"
    );
}

#[test]
fn a_body_that_is_not_the_message_is_an_encoding_failure() {
    let body = [0xff_u8; 6];
    let mut wire = Vec::new();
    wire.extend_from_slice(&u32::try_from(body.len()).expect("small").to_le_bytes());
    wire.extend_from_slice(&body);
    let mut reader: &[u8] = &wire;
    assert!(matches!(
        pipe::recv::<_, Greeting>(&mut reader),
        Err(PipeError::Encoding(_))
    ));
}

// ---------------------------------------------------------------- spawn --
//
// `broker_exe`'s own two tests are not here: they set an environment variable,
// and this binary reads the environment on nearly every call - `pipe_name` asks
// for the temporary directory, `broker_exe` asks for its own variable - so a
// writer among the readers is a data race rather than a test. They are in
// `tests/broker_exe_env.rs`, in a process of their own.

/// Nothing in this process can observe whether a child was given a console, so
/// the guard is that the spawn helper still asks for none - and asks for
/// nothing else, because a creation flag is a thing that gets added to.
#[test]
fn no_console_is_create_no_window_and_nothing_else() {
    let source = include_str!("spawn.rs");
    assert!(
        source.contains("command.creation_flags(CREATE_NO_WINDOW);"),
        "no_console must be CREATE_NO_WINDOW and nothing else"
    );
    assert_eq!(
        source.matches("creation_flags").count(),
        1,
        "one creation flag, set once"
    );
}

// ---------------------------------------------------------------- rules --

#[test]
fn a_describe_waits_the_longer_of_the_handshake_and_the_control_timeout() {
    assert_eq!(
        describe_deadline(Duration::from_secs(2)),
        HANDSHAKE_TIMEOUT,
        "a short control timeout does not shorten a program starting"
    );
    assert_eq!(
        describe_deadline(Duration::from_secs(30)),
        Duration::from_secs(30),
        "a quirks table that asks for longer gets longer"
    );
}

// --------------------------------------------------------------- addons --

/// The staging folder is beside the searched one, never inside it.
///
/// §11 item 17: every host's walk descends into every directory it finds, dot
/// prefixed ones included, and the start-up scan fires at every launch - so a
/// staging folder under `addons/` is a half-written bundle a scan can open, and
/// an install killed before the rename leaves one there for ever. The rename is
/// not what delivers "never looks installed"; keeping the staged tree out of the
/// searched directory is.
#[test]
fn the_staging_folder_is_not_inside_the_folder_every_host_searches() {
    let (Some(addons), Some(staging)) = (crate::addons_dir(), crate::staging_dir()) else {
        // A platform with no home directory installs nothing, which is the same
        // answer both functions give and not a failure.
        return;
    };
    assert_ne!(addons, staging);
    assert!(
        !staging.starts_with(&addons),
        "{staging:?} is inside {addons:?}, where a scan would walk into it"
    );
    assert_eq!(
        staging.parent(),
        addons.parent(),
        "staging must be a sibling, so landing an install is one rename"
    );
}
