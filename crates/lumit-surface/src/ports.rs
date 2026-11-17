//! The `midi` thread: the system's MIDI input ports, the ones the person
//! chose kept open, and every message from them read in one place.
//!
//! There is no word from the system when a device is plugged in or pulled
//! out, so the port list is read again every second. A chosen port that
//! appears is opened, and one that has gone is closed, so a controller can
//! come and go while Lumit runs.
//!
//! Each open port's callback runs on the MIDI backend's own thread. It only
//! copies the message into this thread's bounded channel, dropping it if the
//! channel is full, and everything else happens here.

use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use midir::{Ignore, MidiInput, MidiInputConnection, MidiInputPort};

use crate::midi::{Midi, MidiMapping};
use crate::Shared;

/// How often the port list is read.
const POLL: Duration = Duration::from_secs(1);
/// Messages waiting for the thread. A controller sends a few hundred a
/// second at most.
const QUEUE: usize = 1024;
/// What the system's MIDI lists call Lumit.
const CLIENT: &str = "Lumit";

/// What the thread is told.
pub(crate) enum Msg {
    /// A message from the port called `port`.
    Raw {
        port: Arc<str>,
        bytes: [u8; 3],
        len: usize,
    },
    /// Keep these ports open, by name.
    Want(Vec<String>),
    Mappings(Vec<MidiMapping>),
    Learn(bool),
}

/// Start the thread. None when the system has no MIDI or no thread could be
/// made.
pub(crate) fn start(shared: Arc<Shared>) -> Option<SyncSender<Msg>> {
    let (tx, rx) = sync_channel(QUEUE);
    let own = tx.clone();
    std::thread::Builder::new()
        .name("midi".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(&shared, &rx, &own);
            }));
        })
        .ok()?;
    Some(tx)
}

fn run(shared: &Shared, rx: &Receiver<Msg>, tx: &SyncSender<Msg>) {
    let Ok(lister) = MidiInput::new(CLIENT) else {
        return;
    };
    let mut midi = Midi::default();
    let mut want: Vec<String> = Vec::new();
    let mut open: Vec<(String, MidiInputConnection<()>)> = Vec::new();
    let mut next_poll = Instant::now();
    let mut events = Vec::new();
    loop {
        let wait = next_poll.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Msg::Raw { port, bytes, len }) => {
                midi.take(&port, bytes.get(..len).unwrap_or_default(), &mut events);
            }
            Ok(Msg::Want(names)) => {
                want = names;
                next_poll = Instant::now();
            }
            Ok(Msg::Mappings(mappings)) => midi.set_mappings(mappings),
            Ok(Msg::Learn(on)) => midi.learn(on),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        for event in events.drain(..) {
            (shared.tell)(event);
        }
        if Instant::now() >= next_poll {
            next_poll = Instant::now() + POLL;
            let ports: Vec<(MidiInputPort, String)> = lister
                .ports()
                .into_iter()
                .filter_map(|port| lister.port_name(&port).ok().map(|name| (port, name)))
                .collect();
            shared.set_ports(ports.iter().map(|(_, name)| name.clone()).collect());
            // Dropping a connection closes its port.
            open.retain(|(name, _)| {
                want.contains(name) && ports.iter().any(|(_, there)| there == name)
            });
            for (port, name) in &ports {
                if want.contains(name) && !open.iter().any(|(held, _)| held == name) {
                    if let Some(connection) = connect(port, name, tx) {
                        open.push((name.clone(), connection));
                    }
                }
            }
        }
    }
}

/// Open `port`. Its messages go to the thread through `tx`.
fn connect(
    port: &MidiInputPort,
    name: &str,
    tx: &SyncSender<Msg>,
) -> Option<MidiInputConnection<()>> {
    let mut input = MidiInput::new(CLIENT).ok()?;
    // System exclusive, clock and active sensing are never a control.
    input.ignore(Ignore::All);
    let tag: Arc<str> = Arc::from(name);
    let tx = tx.clone();
    input
        .connect(
            port,
            CLIENT,
            move |_stamp, message, _| {
                if message.len() > 3 {
                    return;
                }
                let mut bytes = [0u8; 3];
                bytes
                    .iter_mut()
                    .zip(message)
                    .for_each(|(to, from)| *to = *from);
                // Full means the thread is behind; a lost tick is better
                // than a stalled MIDI driver.
                let _ = tx.try_send(Msg::Raw {
                    port: tag.clone(),
                    bytes,
                    len: message.len(),
                });
            },
            (),
        )
        .ok()
}
