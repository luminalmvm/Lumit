//! TIPC, the Tangent Hub's wire format, written from Tangent's published
//! description of it (TUBE API, TIPC revision 9).
//!
//! A message is a byte count and then one or more commands back to back.
//! Each command is a 32-bit id and its fields. Integers and floats are 32
//! bits, most significant byte first, whatever the machine. A string is its
//! length and then its bytes, with no terminator. A bool is an integer, 0 or
//! 1.
//!
//! Pure: bytes in, commands out, and back. The socket is
//! [`crate::tangent`]'s.

/// The largest message we accept. The Hub's are a few dozen bytes; anything
/// near this is a stream that has lost its place.
pub(crate) const LARGEST: usize = 64 * 1024;

/// A command from the Hub.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FromHub {
    /// The Hub is there: its protocol revision and the panels it expects,
    /// as (type, id) pairs.
    InitiateComms {
        revision: u32,
        panels: Vec<(u32, u32)>,
    },
    ParameterChange {
        id: u32,
        by: f32,
    },
    ParameterReset {
        id: u32,
    },
    ParameterValueRequest {
        id: u32,
    },
    MenuChange {
        id: u32,
        by: i32,
    },
    MenuReset {
        id: u32,
    },
    MenuStringRequest {
        id: u32,
    },
    ActionOn {
        id: u32,
    },
    ModeChange {
        id: u32,
    },
    /// Never both at once. 32 counts to a turn unless the user changed it.
    Transport {
        jog: i32,
        shuttle: i32,
    },
    ActionOff {
        id: u32,
    },
}

/// A command to the Hub.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ToHub {
    /// Who we are and the folder holding our `controls.xml` and default
    /// maps. `process` is sent only when the Hub's revision has the field.
    ApplicationDefinition {
        app: String,
        sys_dir: String,
        process: bool,
    },
    ParameterValue {
        id: u32,
        value: f32,
        at_default: bool,
    },
    MenuString {
        id: u32,
        text: String,
        at_default: bool,
    },
    AllChange,
    ModeValue {
        id: u32,
    },
    /// The status lines, top first.
    DisplayText {
        lines: Vec<String>,
    },
}

/// The revision from which ApplicationDefinition carries a process name.
pub(crate) const PROCESS_FIELD_FROM: u32 = 7;

/// The commands in one message's body, in order. A command we have no use
/// for ends the read: commands carry no length, so nothing after an unknown
/// one can be found. Nothing we enable in `controls.xml` sends one.
pub(crate) fn decode(body: &[u8]) -> Vec<FromHub> {
    let mut read = Reader { bytes: body, at: 0 };
    let mut out = Vec::new();
    while let Some(command) = read.command() {
        out.push(command);
    }
    out
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn word(&mut self) -> Option<[u8; 4]> {
        let end = self.at.checked_add(4)?;
        let word = self.bytes.get(self.at..end)?;
        self.at = end;
        word.try_into().ok()
    }

    fn u32(&mut self) -> Option<u32> {
        self.word().map(u32::from_be_bytes)
    }

    fn i32(&mut self) -> Option<i32> {
        self.word().map(i32::from_be_bytes)
    }

    fn f32(&mut self) -> Option<f32> {
        self.word().map(f32::from_be_bytes)
    }

    fn command(&mut self) -> Option<FromHub> {
        Some(match self.u32()? {
            0x01 => {
                let revision = self.u32()?;
                let count = self.u32()?;
                let mut panels = Vec::new();
                for _ in 0..count {
                    panels.push((self.u32()?, self.u32()?));
                }
                FromHub::InitiateComms { revision, panels }
            }
            0x02 => FromHub::ParameterChange {
                id: self.u32()?,
                by: self.f32()?,
            },
            0x03 => FromHub::ParameterReset { id: self.u32()? },
            0x04 => FromHub::ParameterValueRequest { id: self.u32()? },
            0x05 => FromHub::MenuChange {
                id: self.u32()?,
                by: self.i32()?,
            },
            0x06 => FromHub::MenuReset { id: self.u32()? },
            0x07 => FromHub::MenuStringRequest { id: self.u32()? },
            0x08 => FromHub::ActionOn { id: self.u32()? },
            0x09 => FromHub::ModeChange { id: self.u32()? },
            0x0A => FromHub::Transport {
                jog: self.i32()?,
                shuttle: self.i32()?,
            },
            0x0B => FromHub::ActionOff { id: self.u32()? },
            _ => return None,
        })
    }
}

/// One message holding `commands`, its byte count first.
pub(crate) fn encode(commands: &[ToHub]) -> Vec<u8> {
    let mut body = Vec::new();
    for command in commands {
        command.write(&mut body);
    }
    let mut out = Vec::with_capacity(body.len() + 4);
    put_u32(&mut out, u32::try_from(body.len()).unwrap_or(u32::MAX));
    out.extend_from_slice(&body);
    out
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// A string as its length and its bytes. The panels draw ASCII alone, so
/// anything else shows as `?`, and it is cut to `most` characters.
fn put_text(out: &mut Vec<u8>, text: &str, most: usize) {
    let bytes: Vec<u8> = text
        .chars()
        .take(most)
        .map(|c| match c {
            ' '..='~' => c as u8,
            _ => b'?',
        })
        .collect();
    put_u32(out, bytes.len() as u32);
    out.extend_from_slice(&bytes);
}

/// A path as the Hub reads it: the bytes as they are, since the folder has
/// to be found, not drawn.
fn put_path(out: &mut Vec<u8>, path: &str) {
    let bytes = &path.as_bytes()[..path.len().min(1024)];
    put_u32(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}

impl ToHub {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            ToHub::ApplicationDefinition {
                app,
                sys_dir,
                process,
            } => {
                put_u32(out, 0x81);
                put_text(out, app, 256);
                put_path(out, sys_dir);
                // No folder of the user's own: the Hub keeps their maps
                // where it always does.
                put_u32(out, 0);
                if *process {
                    put_u32(out, 0);
                }
            }
            ToHub::ParameterValue {
                id,
                value,
                at_default,
            } => {
                put_u32(out, 0x82);
                put_u32(out, *id);
                out.extend_from_slice(&value.to_be_bytes());
                put_u32(out, u32::from(*at_default));
            }
            ToHub::MenuString {
                id,
                text,
                at_default,
            } => {
                put_u32(out, 0x83);
                put_u32(out, *id);
                put_text(out, text, 256);
                put_u32(out, u32::from(*at_default));
            }
            ToHub::AllChange => put_u32(out, 0x84),
            ToHub::ModeValue { id } => {
                put_u32(out, 0x85);
                put_u32(out, *id);
            }
            ToHub::DisplayText { lines } => {
                put_u32(out, 0x86);
                let lines = &lines[..lines.len().min(3)];
                put_u32(out, lines.len() as u32);
                for line in lines {
                    put_text(out, line, 32);
                    // Single height.
                    put_u32(out, 0);
                }
            }
        }
    }
}

/// Bytes off the socket, cut into whole messages.
#[derive(Default)]
pub(crate) struct Framer {
    held: Vec<u8>,
}

/// The stream has lost its place: a message longer than any the Hub sends.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Garbled;

impl Framer {
    /// Take `bytes` and hand each whole message's body to `message`.
    pub(crate) fn push(
        &mut self,
        bytes: &[u8],
        mut message: impl FnMut(&[u8]),
    ) -> Result<(), Garbled> {
        self.held.extend_from_slice(bytes);
        let mut at = 0;
        while let Some(head) = self.held.get(at..at + 4) {
            let mut word = [0u8; 4];
            word.copy_from_slice(head);
            let len = u32::from_be_bytes(word) as usize;
            if len > LARGEST {
                self.held.clear();
                return Err(Garbled);
            }
            let Some(body) = self.held.get(at + 4..at + 4 + len) else {
                break;
            };
            message(body);
            at += 4 + len;
        }
        self.held.drain(..at);
        Ok(())
    }
}
