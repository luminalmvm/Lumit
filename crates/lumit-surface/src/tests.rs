use crate::exchange::Exchange;
use crate::midi::{relative_ticks, Encoding, Midi, MidiControl, MidiKind, MidiMapping, Target};
use crate::table::{self, Axis, MODE_GRADING};
use crate::tipc::{self, Framer, FromHub, ToHub};
use crate::{Event, Shown};

fn be(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// The Hub's side of a conversation built byte by byte from Tangent's
/// documented layout, and our replies checked the same way. A wrong byte
/// order here would break every panel.
#[test]
fn a_hub_conversation_round_trips_in_big_endian() {
    let lift_x = table::param_named("lift", Some(Axis::X))
        .map(|p| p.id)
        .unwrap();
    let contrast = table::param_named("contrast", None).map(|p| p.id).unwrap();
    // InitiateComms: revision 9, an Element-Tk and an Element-Mf. Then a
    // trackball's X by a quarter, and the jog wheel three frames on.
    let mut body = be(&[0x01, 9, 2, 0x0C, 1, 0x0D, 2, 0x02, lift_x]);
    body.extend_from_slice(&0.25f32.to_be_bytes());
    body.extend(be(&[0x0A, 3, 0, 0x04, contrast]));
    let mut wire = be(&[body.len() as u32]);
    wire.extend(&body);

    // Split mid-word, as a socket may hand it over.
    let mut framer = Framer::default();
    let mut commands = Vec::new();
    for half in [&wire[..7], &wire[7..]] {
        framer
            .push(half, |message| commands.extend(tipc::decode(message)))
            .unwrap();
    }
    assert_eq!(
        commands,
        vec![
            FromHub::InitiateComms {
                revision: 9,
                panels: vec![(0x0C, 1), (0x0D, 2)],
            },
            FromHub::ParameterChange {
                id: lift_x,
                by: 0.25
            },
            FromHub::Transport { jog: 3, shuttle: 0 },
            FromHub::ParameterValueRequest { id: contrast },
        ]
    );

    let mut shown = Shown::default();
    shown.values.insert("contrast".into(), 120.0);
    let mut exchange = Exchange::default();
    let (mut out, mut events, mut panels) = (Vec::new(), Vec::new(), None);
    for command in commands {
        panels = exchange
            .on(command, &shown, "C:/x", &mut out, &mut events)
            .or(panels);
    }
    assert_eq!(
        panels,
        Some(vec!["Element-Tk".to_owned(), "Element-Mf".to_owned()])
    );
    assert_eq!(
        events,
        vec![
            Event::Nudge {
                param: "lift",
                axis: Some(Axis::X),
                by: 0.25
            },
            Event::Jog { frames: 3 },
        ]
    );

    // ApplicationDefinition with the process field revision 9 carries,
    // ModeValue, then contrast at 120 and not at rest.
    let mut want = be(&[0x81, 5]);
    want.extend(b"Lumit");
    want.extend(be(&[4]));
    want.extend(b"C:/x");
    want.extend(be(&[0, 0, 0x85, MODE_GRADING, 0x82, contrast]));
    want.extend(120f32.to_be_bytes());
    want.extend(be(&[0]));
    let mut message = be(&[want.len() as u32]);
    message.extend(want);
    assert_eq!(tipc::encode(&out), message);
    assert_eq!(
        out.last(),
        Some(&ToHub::ParameterValue {
            id: contrast,
            value: 120.0,
            at_default: false
        })
    );
}

/// Endless encoders in the three common encodings, read straight and after
/// MIDI learn has guessed which one a controller speaks.
#[test]
fn relative_encoders_read_their_steps() {
    for (encoding, up, down) in [
        (Encoding::TwosComplement, 1, 127),
        (Encoding::SignBit, 1, 65),
        (Encoding::Offset64, 65, 63),
    ] {
        assert_eq!(relative_ticks(up, encoding), 1, "{encoding:?}");
        assert_eq!(relative_ticks(down, encoding), -1, "{encoding:?}");
    }
    assert_eq!(relative_ticks(5, Encoding::TwosComplement), 5);
    assert_eq!(relative_ticks(123, Encoding::TwosComplement), -5);
    assert_eq!(relative_ticks(0x45, Encoding::SignBit), -5);
    assert_eq!(relative_ticks(59, Encoding::Offset64), -5);

    let mut midi = Midi::default();
    let mut events = Vec::new();
    midi.learn(true);
    // An encoder turned one step up twice on CC 16, channel 2.
    midi.take("Knobs", &[0xB1, 16, 65], &mut events);
    midi.take("Knobs", &[0xB1, 16, 65], &mut events);
    let control = MidiControl {
        port: "Knobs".into(),
        channel: 1,
        kind: MidiKind::Cc,
        number: 16,
    };
    assert_eq!(
        events,
        vec![Event::Learned {
            control: control.clone(),
            encoding: Encoding::Offset64
        }]
    );

    events.clear();
    midi.set_mappings(vec![MidiMapping {
        control,
        encoding: Encoding::Offset64,
        target: Target::Param {
            param: "saturation".into(),
            axis: None,
        },
    }]);
    midi.take("Knobs", &[0xB1, 16, 62], &mut events);
    let step = f64::from(table::param_named("saturation", None).unwrap().step);
    let [Event::Nudge {
        param: "saturation",
        axis: None,
        by,
    }] = events.as_slice()
    else {
        panic!("{events:?}");
    };
    assert!((by + 2.0 * step * 10.0).abs() < 1e-9, "{by}");
}
