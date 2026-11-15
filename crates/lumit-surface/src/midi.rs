//! MIDI controllers: what a message means, learning which control is which,
//! and turning a learned control into a nudge, a reset or an action.
//!
//! Pure. The ports and their callbacks are [`crate::ports`]'s, and everything
//! here runs on its `midi` thread.

use std::collections::{HashMap, HashSet};

use crate::table::{self, Action, Axis, Param};
use crate::Event;

/// What sort of control sent a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MidiKind {
    /// A key or pad: Note on and Note off.
    Note,
    /// A controller sending one 7-bit value.
    Cc,
    /// A controller sending 14 bits as a pair: its number carries the high
    /// seven, and the number 32 above it the low seven.
    Cc14,
}

/// One control on one device, as MIDI names it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MidiControl {
    /// The input port it came in on, by the name the system gives it.
    pub port: String,
    /// 0 to 15, shown to people as 1 to 16.
    pub channel: u8,
    pub kind: MidiKind,
    /// The note or controller number. For [`MidiKind::Cc14`], the high one.
    pub number: u8,
}

/// How a controller's value is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// A knob or fader that says where it is.
    Absolute,
    /// An endless encoder: 1 is one step up, 127 one step down.
    TwosComplement,
    /// An endless encoder: bit 6 is the sign, 1 one step up, 65 one down.
    SignBit,
    /// An endless encoder: 65 is one step up, 63 one step down.
    Offset64,
}

/// What a learned control drives.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A Correction row, and for a wheel which part of it.
    Param { param: String, axis: Option<Axis> },
    /// One of [`crate::table::ACTIONS`], by name.
    Action(String),
}

/// A learned control and what it drives.
#[derive(Clone, Debug, PartialEq)]
pub struct MidiMapping {
    pub control: MidiControl,
    pub encoding: Encoding,
    pub target: Target,
}

/// One step of an endless encoder moves a row this many of its own smallest
/// steps.
const STEPS_PER_TICK: f64 = 10.0;

/// The steps an endless encoder's `value` stands for.
pub fn relative_ticks(value: u8, encoding: Encoding) -> i32 {
    let value = i32::from(value & 0x7F);
    match encoding {
        Encoding::Absolute => 0,
        Encoding::TwosComplement => {
            if value < 64 {
                value
            } else {
                value - 128
            }
        }
        Encoding::SignBit => {
            if value & 0x40 != 0 {
                -(value & 0x3F)
            } else {
                value
            }
        }
        Encoding::Offset64 => value - 64,
    }
}

/// A channel message we act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Message {
    Note { channel: u8, number: u8, on: bool },
    Cc { channel: u8, number: u8, value: u8 },
}

/// Read one message. Everything but notes and controllers is left alone.
pub(crate) fn parse(bytes: &[u8]) -> Option<Message> {
    let (&status, data) = bytes.split_first()?;
    let channel = status & 0x0F;
    let (&number, rest) = data.split_first()?;
    let &value = rest.first()?;
    let (number, value) = (number & 0x7F, value & 0x7F);
    Some(match status & 0xF0 {
        0x80 => Message::Note {
            channel,
            number,
            on: false,
        },
        // A Note on with no velocity is a Note off.
        0x90 => Message::Note {
            channel,
            number,
            on: value > 0,
        },
        0xB0 => Message::Cc {
            channel,
            number,
            value,
        },
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum To {
    Param(&'static Param),
    Action(&'static Action),
}

struct Bound {
    control: MidiControl,
    encoding: Encoding,
    to: To,
}

/// A controller on one channel of one port.
type Place = (String, u8, u8);

/// The learned controls, and what the last messages left behind.
#[derive(Default)]
pub(crate) struct Midi {
    bound: Vec<Bound>,
    /// Some while learning: the first controller message seen, if any. A
    /// controller is told apart from a 14-bit pair or an encoder by the
    /// message after it.
    learning: Option<Option<(Place, u8)>>,
    /// Where each absolute control last was, so a move is a change from
    /// there. Keyed by port, channel and number, so it holds one entry for
    /// each control that has moved since Lumit started.
    last: HashMap<Place, u16>,
    /// The high half of each 14-bit pair, waiting for its low half.
    high: HashMap<Place, u8>,
    /// Absolute controllers driving an action that are held down.
    held: HashSet<Place>,
}

impl Midi {
    /// Use `mappings` from now on. One whose target is unknown is skipped.
    pub(crate) fn set_mappings(&mut self, mappings: Vec<MidiMapping>) {
        self.bound = mappings
            .into_iter()
            .filter_map(|m| {
                let to = match &m.target {
                    Target::Param { param, axis } => To::Param(table::param_named(param, *axis)?),
                    Target::Action(name) => To::Action(table::action_named(name)?),
                };
                Some(Bound {
                    control: m.control,
                    encoding: m.encoding,
                    to,
                })
            })
            .collect();
    }

    /// Start or stop listening for the next control moved.
    pub(crate) fn learn(&mut self, on: bool) {
        self.learning = on.then_some(None);
    }

    /// Act on one message from `port`.
    pub(crate) fn take(&mut self, port: &str, bytes: &[u8], events: &mut Vec<Event>) {
        let Some(message) = parse(bytes) else {
            return;
        };
        if self.learning.is_some() {
            self.learn_from(port, message, events);
            return;
        }
        match message {
            Message::Note {
                channel,
                number,
                on,
            } => {
                for (_, to) in self.matching(port, channel, MidiKind::Note, number) {
                    match to {
                        To::Param(p) if on => events.push(Event::Reset {
                            param: p.param,
                            axis: p.axis,
                        }),
                        To::Param(_) => {}
                        To::Action(a) => events.push(Event::Action {
                            action: a.action,
                            down: on,
                        }),
                    }
                }
            }
            Message::Cc {
                channel,
                number,
                value,
            } => self.controller(port, channel, number, value, events),
        }
    }

    fn matching(&self, port: &str, channel: u8, kind: MidiKind, number: u8) -> Vec<(Encoding, To)> {
        self.bound
            .iter()
            .filter(|b| {
                b.control.port == port
                    && b.control.channel == channel
                    && b.control.kind == kind
                    && b.control.number == number
            })
            .map(|b| (b.encoding, b.to))
            .collect()
    }

    fn controller(
        &mut self,
        port: &str,
        channel: u8,
        number: u8,
        value: u8,
        events: &mut Vec<Event>,
    ) {
        let place = (port.to_owned(), channel, number);
        if number < 32 {
            self.high.insert(place.clone(), value);
        }
        for (encoding, to) in self.matching(port, channel, MidiKind::Cc, number) {
            match (encoding, to) {
                (Encoding::Absolute, To::Param(p)) => {
                    self.absolute(place.clone(), p, u16::from(value), 127.0, events);
                }
                (Encoding::Absolute, To::Action(a)) => {
                    let down = value >= 64;
                    let was = self.held.contains(&place);
                    if down != was {
                        if down {
                            self.held.insert(place.clone());
                        } else {
                            self.held.remove(&place);
                        }
                        events.push(Event::Action {
                            action: a.action,
                            down,
                        });
                    }
                }
                (_, To::Param(p)) => {
                    let ticks = relative_ticks(value, encoding);
                    if ticks != 0 {
                        events.push(Event::Nudge {
                            param: p.param,
                            axis: p.axis,
                            by: f64::from(ticks) * f64::from(p.step) * STEPS_PER_TICK,
                        });
                    }
                }
                // A step of an encoder is a press.
                (_, To::Action(a)) => {
                    if relative_ticks(value, encoding) != 0 {
                        events.push(Event::Action {
                            action: a.action,
                            down: true,
                        });
                        events.push(Event::Action {
                            action: a.action,
                            down: false,
                        });
                    }
                }
            }
        }
        // The low half of a 14-bit pair finishes it.
        if (32..64).contains(&number) {
            let high_place = (port.to_owned(), channel, number - 32);
            let high = self.high.get(&high_place).copied().unwrap_or(0);
            let full = (u16::from(high) << 7) | u16::from(value);
            for (_, to) in self.matching(port, channel, MidiKind::Cc14, number - 32) {
                if let To::Param(p) = to {
                    self.absolute(high_place.clone(), p, full, 16383.0, events);
                }
            }
        }
    }

    /// An absolute control at `value` of `full`: the change since it was
    /// last seen, over the row's span. Its first message only says where it
    /// is, so picking a fader up never jumps the grade.
    fn absolute(
        &mut self,
        place: Place,
        p: &Param,
        value: u16,
        full: f64,
        events: &mut Vec<Event>,
    ) {
        if let Some(was) = self.last.insert(place, value) {
            let moved = f64::from(value) - f64::from(was);
            if moved != 0.0 {
                events.push(Event::Nudge {
                    param: p.param,
                    axis: p.axis,
                    by: moved / full * f64::from(p.max - p.min),
                });
            }
        }
    }

    fn learn_from(&mut self, port: &str, message: Message, events: &mut Vec<Event>) {
        let learned = match message {
            Message::Note {
                channel,
                number,
                on: true,
            } => Some((
                MidiControl {
                    port: port.to_owned(),
                    channel,
                    kind: MidiKind::Note,
                    number,
                },
                Encoding::Absolute,
            )),
            Message::Note { .. } => None,
            Message::Cc {
                channel,
                number,
                value,
            } => {
                let first = self.learning.take().flatten();
                let place = (port.to_owned(), channel, number);
                let control = |kind, number| MidiControl {
                    port: port.to_owned(),
                    channel,
                    kind,
                    number,
                };
                match first {
                    Some(((p, c, n), _))
                        if p == port && c == channel && n < 32 && number == n + 32 =>
                    {
                        Some((control(MidiKind::Cc14, n), Encoding::Absolute))
                    }
                    Some(((p, c, n), was)) if p == port && c == channel && n == number => {
                        Some((control(MidiKind::Cc, number), guess(was, value)))
                    }
                    _ => {
                        self.learning = Some(Some((place, value)));
                        None
                    }
                }
            }
        };
        if let Some((control, encoding)) = learned {
            self.learning = None;
            events.push(Event::Learned { control, encoding });
        }
    }
}

/// How a controller that sent `a` then `b` is most likely read. An endless
/// encoder turned one way sends the same small step again and again; a knob
/// says where it is, which changes. Sign-bit encoders read like one of the
/// other two, so they are only ever chosen by hand.
fn guess(a: u8, b: u8) -> Encoding {
    match (a, b) {
        (1..=3, 1..=3) | (125..=127, 125..=127) if a == b => Encoding::TwosComplement,
        (61..=67, 61..=67) if a == b && a != 64 => Encoding::Offset64,
        _ => Encoding::Absolute,
    }
}
