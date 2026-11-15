//! Control surfaces: grading with hardware.
//!
//! Two kinds are spoken to. **Tangent** panels (Element, Ripple, Wave, Arc)
//! through Tangent's own Hub, a separate program the person installs from
//! Tangent, which this crate reaches over a socket on this machine with our
//! own client for its documented protocol ([`tipc`]). And **MIDI**
//! controllers of any make, through MIDI learn.
//!
//! Both turn what a hand does into the same [`Event`]s: move a Correction
//! row by some amount, reset it, press or let go of an action, step a menu,
//! jog or shuttle. What an event does to the grade is the interface's
//! business. The interface pushes back what the panels should show with
//! [`Surfaces::show`].
//!
//! Threads: Tangent runs on `tangent` while panels are on, MIDI on `midi`
//! once anything has asked about it. Events are handed to the `tell` given
//! to [`Surfaces::new`] from those threads. No lock is held across a read, a
//! write or a call to `tell`.

// The browser build has no MIDI ports, so it keeps the decoding unused.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

mod exchange;
pub mod midi;
#[cfg(not(target_arch = "wasm32"))]
mod ports;
pub mod table;
mod tangent;
mod tipc;
mod xml;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use midi::{Encoding, MidiControl, MidiMapping};
use table::Axis;

/// Something a person did on a surface.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Move Correction row `param` by `by`, in the row's own units. For a
    /// wheel, `axis` says which part: X and Y its chroma, Master all three
    /// channels.
    Nudge {
        param: &'static str,
        axis: Option<Axis>,
        by: f64,
    },
    /// Put `param` back where it rests, only `axis` of a wheel when given.
    Reset {
        param: &'static str,
        axis: Option<Axis>,
    },
    /// A button pressed (`down`) or let go.
    Action { action: &'static str, down: bool },
    /// Step `menu` by one either way, or with 0 put it back to its first
    /// item.
    Menu { menu: &'static str, step: i32 },
    /// Move the playhead by `frames`.
    Jog { frames: i32 },
    /// One step along the playback ladder, 1 forwards or -1 back.
    Shuttle { step: i32 },
    /// Where Tangent panels stand now.
    Tangent(TangentStatus),
    /// The MIDI input ports on the system now, by name.
    Ports(Vec<String>),
    /// MIDI learn heard `control`, which most likely reads as `encoding`.
    /// Learning stops.
    Learned {
        control: MidiControl,
        encoding: Encoding,
    },
}

/// Where Tangent panels stand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TangentStatus {
    Off,
    /// On, and the Hub is not there yet.
    Waiting,
    /// Talking to the Hub, which has these panels, by Tangent's names.
    Connected {
        panels: Vec<String>,
    },
}

/// What the panels show: each row's value under its key (`contrast`,
/// `lift.x`), the place of each menu's item under the menu's name
/// (`grade_page`), the node's number from 1 under `node`, and the shot's
/// name for the status line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    pub values: BTreeMap<String, f64>,
    pub shot: String,
}

/// What the threads share. Each lock is held to copy a value in or out.
pub(crate) struct Shared {
    pub(crate) tell: Box<dyn Fn(Event) + Send + Sync>,
    shown: Mutex<(u64, Arc<Shown>)>,
    status: Mutex<TangentStatus>,
    ports: Mutex<Vec<String>>,
}

impl Shared {
    /// The newest values, and a number that changes when they do.
    pub(crate) fn shown(&self) -> (u64, Arc<Shown>) {
        self.shown
            .lock()
            .map(|held| (held.0, held.1.clone()))
            .unwrap_or_default()
    }

    /// Say where Tangent stands, unless the thread saying it is stopping.
    pub(crate) fn set_status(&self, status: TangentStatus, stop: &AtomicBool) {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        self.store_status(status);
    }

    fn store_status(&self, status: TangentStatus) {
        let changed = match self.status.lock() {
            Ok(mut held) if *held != status => {
                *held = status.clone();
                true
            }
            _ => false,
        };
        if changed {
            (self.tell)(Event::Tangent(status));
        }
    }

    /// The port list as read now, told only when it changed.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn set_ports(&self, ports: Vec<String>) {
        let changed = match self.ports.lock() {
            Ok(mut held) if *held != ports => {
                *held = ports.clone();
                true
            }
            _ => false,
        };
        if changed {
            (self.tell)(Event::Ports(ports));
        }
    }
}

/// Every surface this Lumit talks to. One for the whole run.
pub struct Surfaces {
    shared: Arc<Shared>,
    /// The Tangent thread's stop flag while panels are on.
    tangent: Mutex<Option<Arc<AtomicBool>>>,
    #[cfg(not(target_arch = "wasm32"))]
    midi: Mutex<Option<std::sync::mpsc::SyncSender<ports::Msg>>>,
}

impl Surfaces {
    /// Surfaces that tell `tell` what happens on them, from their own
    /// threads.
    pub fn new(tell: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Surfaces {
            shared: Arc::new(Shared {
                tell: Box::new(tell),
                shown: Mutex::default(),
                status: Mutex::new(TangentStatus::Off),
                ports: Mutex::default(),
            }),
            tangent: Mutex::default(),
            #[cfg(not(target_arch = "wasm32"))]
            midi: Mutex::default(),
        }
    }

    /// Turn Tangent panels on or off. On writes our control and map files
    /// into `folder`, which the Hub is told to read, and keeps a connection
    /// to the Hub until turned off.
    pub fn set_tangent(&self, on: bool, folder: PathBuf) {
        let Ok(mut running) = self.tangent.lock() else {
            return;
        };
        if let Some(stop) = running.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if !on {
            drop(running);
            self.shared.store_status(TangentStatus::Off);
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, flag) = (self.shared.clone(), stop.clone());
        let spawned = std::thread::Builder::new()
            .name("tangent".into())
            .spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    tangent::run(&shared, &flag, &folder);
                }));
            });
        if spawned.is_ok() {
            *running = Some(stop);
        }
    }

    /// Where Tangent panels stand.
    pub fn tangent_status(&self) -> TangentStatus {
        self.shared
            .status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(TangentStatus::Off)
    }

    /// What the panels should show from now on. Only what changed is sent.
    pub fn show(&self, shown: Shown) {
        if let Ok(mut held) = self.shared.shown.lock() {
            *held = (held.0.wrapping_add(1), Arc::new(shown));
        }
    }

    /// The MIDI input ports, as last read. Starts watching them, and the
    /// list then comes again as [`Event::Ports`] whenever it changes.
    pub fn midi_ports(&self) -> Vec<String> {
        self.midi_send(None);
        self.shared
            .ports
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    /// Keep the ports called `names` open, opening each when it appears.
    pub fn set_midi_inputs(&self, names: Vec<String>) {
        self.midi_send(Some(MidiOrder::Want(names)));
    }

    /// Drive targets from these learned controls.
    pub fn set_midi_mappings(&self, mappings: Vec<MidiMapping>) {
        self.midi_send(Some(MidiOrder::Mappings(mappings)));
    }

    /// Listen for the next control moved on any open port, which comes as
    /// [`Event::Learned`]. False stops listening.
    pub fn midi_learn(&self, on: bool) {
        self.midi_send(Some(MidiOrder::Learn(on)));
    }

    /// Start the MIDI thread if it is not running, and give it `order`.
    #[cfg(not(target_arch = "wasm32"))]
    fn midi_send(&self, order: Option<MidiOrder>) {
        let Ok(mut midi) = self.midi.lock() else {
            return;
        };
        if midi.is_none() {
            *midi = ports::start(self.shared.clone());
        }
        let Some(tx) = midi.clone() else {
            return;
        };
        drop(midi);
        let msg = match order {
            None => return,
            Some(MidiOrder::Want(names)) => ports::Msg::Want(names),
            Some(MidiOrder::Mappings(m)) => ports::Msg::Mappings(m),
            Some(MidiOrder::Learn(on)) => ports::Msg::Learn(on),
        };
        // Waits if the thread is behind: an order must not be lost.
        let _ = tx.send(msg);
    }

    /// The browser build has no MIDI ports.
    #[cfg(target_arch = "wasm32")]
    fn midi_send(&self, _order: Option<MidiOrder>) {}
}

enum MidiOrder {
    Want(Vec<String>),
    Mappings(Vec<MidiMapping>),
    Learn(bool),
}
