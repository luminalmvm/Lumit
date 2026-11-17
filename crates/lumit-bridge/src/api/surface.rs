//! Control surfaces: Tangent panels and MIDI controllers driving a grade.
//!
//! `lumit-surface` talks to the hardware. This is where its events reach
//! the interface, one stream for both, and where the interface says which
//! surfaces to use and what the panels' displays should show. What an event
//! does to the grade is the interface's to decide: it owns the current shot
//! and node.

use std::sync::{Mutex, OnceLock};

use flutter_rust_bridge::frb;
use lumit_surface::midi::{Encoding, MidiControl, MidiKind, MidiMapping, Target};
use lumit_surface::table::{self, Axis};
use lumit_surface::{Event, Shown, Surfaces, TangentStatus};

use crate::frb_generated::StreamSink;

/// Which part of a wheel a control moves: its chroma across or up, or all
/// three channels together.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeSurfaceAxis {
    X,
    Y,
    Master,
}

/// Where Tangent panels stand.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeTangentStatus {
    Off,
    /// On, and Tangent's Hub is not there yet.
    Waiting,
    /// Talking to the Hub, which has these panels, by Tangent's own names
    /// (`Element-Tk`).
    Connected {
        panels: Vec<String>,
    },
}

/// What sort of MIDI control a mapping listens to.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMidiKind {
    Note,
    /// A controller's 7-bit value.
    Cc,
    /// A controller's 14-bit value, sent as a pair 32 numbers apart.
    Cc14,
}

/// One control on one MIDI device.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMidiControl {
    /// The input port, by the name the system gives it.
    pub port: String,
    /// 0 to 15.
    pub channel: u8,
    pub kind: BridgeMidiKind,
    pub number: u8,
}

/// How a MIDI controller's value is read: a knob that says where it is, or
/// an endless encoder in one of the three usual ways of counting steps.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMidiEncoding {
    Absolute,
    TwosComplement,
    SignBit,
    Offset64,
}

/// What a control drives: a Correction row (and for a wheel which part), or
/// an action by its id.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeSurfaceTarget {
    Param {
        param: String,
        axis: Option<BridgeSurfaceAxis>,
    },
    Action {
        id: String,
    },
}

/// A learned MIDI control and what it drives.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMidiMapping {
    pub control: BridgeMidiControl,
    pub encoding: BridgeMidiEncoding,
    pub target: BridgeSurfaceTarget,
}

/// A value the panels show, under its key: `contrast`, `lift.x`, or a
/// menu's name for the place of its item.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSurfaceValue {
    pub key: String,
    pub value: f64,
}

/// Something that happened on a surface.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeSurfaceEvent {
    /// Move Correction row `param` by `by`, in the row's own units. A
    /// wheel's chroma moves on X and Y as the Grade panel's wheel does.
    /// Several in quick succession are one drag.
    Nudge {
        param: String,
        axis: Option<BridgeSurfaceAxis>,
        by: f64,
    },
    /// Put `param` back where it rests: only `axis` of a wheel when given.
    Reset {
        param: String,
        axis: Option<BridgeSurfaceAxis>,
    },
    /// The action `id` pressed (`down`) or let go.
    Action { id: String, down: bool },
    /// Step the menu `id` one item either way, or with 0 back to its first.
    Menu { id: String, step: i32 },
    /// Move the playhead by `frames`.
    Jog { frames: i32 },
    /// One step along the playback ladder, as J and L take.
    Shuttle { step: i32 },
    /// Tangent panels stand somewhere new.
    Tangent { status: BridgeTangentStatus },
    /// The MIDI input ports now, by name.
    Ports { ports: Vec<String> },
    /// MIDI learn heard `control`, most likely read as `encoding`. Learning
    /// has stopped.
    Learned {
        control: BridgeMidiControl,
        encoding: BridgeMidiEncoding,
    },
}

/// Where events go: the stream the interface listens on, if it is.
static SINK: Mutex<Option<StreamSink<BridgeSurfaceEvent>>> = Mutex::new(None);
static SURFACES: OnceLock<Surfaces> = OnceLock::new();

#[frb(ignore)]
fn surfaces() -> &'static Surfaces {
    SURFACES.get_or_init(|| Surfaces::new(tell))
}

/// Hand `event` to the stream. The sink is copied out first, so no lock is
/// held while it crosses to Dart.
#[frb(ignore)]
fn tell(event: Event) {
    let sink = SINK.lock().ok().and_then(|held| held.clone());
    if let Some(sink) = sink {
        let _ = sink.add(event_of(event));
    }
}

/// Hear what happens on every surface, in place of any stream listening
/// before.
pub fn listen_to_surfaces(events: Option<StreamSink<BridgeSurfaceEvent>>) {
    if let Ok(mut held) = SINK.lock() {
        *held = events;
    }
}

/// Talk to Tangent panels, through Tangent's Hub, or stop.
#[frb(sync)]
pub fn set_tangent_panels(enabled: bool) {
    // Lumit's own control and map files, written afresh each time panels
    // are turned on, so a cache folder is the right place for them.
    let folder = lumit_project::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("tangent");
    surfaces().set_tangent(enabled, folder);
}

/// Where Tangent panels stand. Changes come on the stream too.
#[frb(sync)]
pub fn tangent_status() -> BridgeTangentStatus {
    status_of(surfaces().tangent_status())
}

/// The MIDI input ports as last read, and from now on any change to them on
/// the stream.
#[frb(sync)]
pub fn midi_inputs() -> Vec<String> {
    surfaces().midi_ports()
}

/// Keep the MIDI ports called `names` open, each as it appears.
#[frb(sync)]
pub fn open_midi_inputs(names: Vec<String>) {
    surfaces().set_midi_inputs(names);
}

/// Drive grades from these learned MIDI controls. A mapping whose target
/// this Lumit does not have is left out.
#[frb(sync)]
pub fn set_midi_mappings(mappings: Vec<BridgeMidiMapping>) {
    surfaces().set_midi_mappings(mappings.into_iter().map(mapping_from).collect());
}

/// Listen for the next control moved on an open MIDI port, which comes on
/// the stream as `Learned`. False stops listening.
#[frb(sync)]
pub fn midi_learn(listening: bool) {
    surfaces().midi_learn(listening);
}

/// What the panels' displays show: each value under its key, the place of
/// each menu's item under the menu's name, the node's number from 1 under
/// `node`, and the shot's name. Only what changed is sent on.
#[frb(sync)]
pub fn show_on_surfaces(values: Vec<BridgeSurfaceValue>, shot: String) {
    surfaces().show(Shown {
        values: values.into_iter().map(|v| (v.key, v.value)).collect(),
        shot,
    });
}

/// Everything a MIDI control can be learned to drive, in the order a list
/// shows them: the rows, a wheel's X, Y and Master together, then the
/// actions.
#[frb(sync)]
pub fn surface_targets() -> Vec<BridgeSurfaceTarget> {
    table::PARAMS
        .iter()
        .map(|p| BridgeSurfaceTarget::Param {
            param: p.param.to_owned(),
            axis: p.axis.map(axis_of),
        })
        .chain(table::ACTIONS.iter().map(|a| BridgeSurfaceTarget::Action {
            id: a.action.to_owned(),
        }))
        .collect()
}

#[frb(ignore)]
fn axis_of(axis: Axis) -> BridgeSurfaceAxis {
    match axis {
        Axis::X => BridgeSurfaceAxis::X,
        Axis::Y => BridgeSurfaceAxis::Y,
        Axis::Master => BridgeSurfaceAxis::Master,
    }
}

#[frb(ignore)]
fn axis_from(axis: BridgeSurfaceAxis) -> Axis {
    match axis {
        BridgeSurfaceAxis::X => Axis::X,
        BridgeSurfaceAxis::Y => Axis::Y,
        BridgeSurfaceAxis::Master => Axis::Master,
    }
}

#[frb(ignore)]
fn status_of(status: TangentStatus) -> BridgeTangentStatus {
    match status {
        TangentStatus::Off => BridgeTangentStatus::Off,
        TangentStatus::Waiting => BridgeTangentStatus::Waiting,
        TangentStatus::Connected { panels } => BridgeTangentStatus::Connected { panels },
    }
}

#[frb(ignore)]
fn encoding_of(encoding: Encoding) -> BridgeMidiEncoding {
    match encoding {
        Encoding::Absolute => BridgeMidiEncoding::Absolute,
        Encoding::TwosComplement => BridgeMidiEncoding::TwosComplement,
        Encoding::SignBit => BridgeMidiEncoding::SignBit,
        Encoding::Offset64 => BridgeMidiEncoding::Offset64,
    }
}

#[frb(ignore)]
fn control_of(control: MidiControl) -> BridgeMidiControl {
    BridgeMidiControl {
        port: control.port,
        channel: control.channel,
        kind: match control.kind {
            MidiKind::Note => BridgeMidiKind::Note,
            MidiKind::Cc => BridgeMidiKind::Cc,
            MidiKind::Cc14 => BridgeMidiKind::Cc14,
        },
        number: control.number,
    }
}

#[frb(ignore)]
fn mapping_from(mapping: BridgeMidiMapping) -> MidiMapping {
    let control = mapping.control;
    MidiMapping {
        control: MidiControl {
            port: control.port,
            channel: control.channel,
            kind: match control.kind {
                BridgeMidiKind::Note => MidiKind::Note,
                BridgeMidiKind::Cc => MidiKind::Cc,
                BridgeMidiKind::Cc14 => MidiKind::Cc14,
            },
            number: control.number,
        },
        encoding: match mapping.encoding {
            BridgeMidiEncoding::Absolute => Encoding::Absolute,
            BridgeMidiEncoding::TwosComplement => Encoding::TwosComplement,
            BridgeMidiEncoding::SignBit => Encoding::SignBit,
            BridgeMidiEncoding::Offset64 => Encoding::Offset64,
        },
        target: match mapping.target {
            BridgeSurfaceTarget::Param { param, axis } => Target::Param {
                param,
                axis: axis.map(axis_from),
            },
            BridgeSurfaceTarget::Action { id } => Target::Action(id),
        },
    }
}

#[frb(ignore)]
fn event_of(event: Event) -> BridgeSurfaceEvent {
    match event {
        Event::Nudge { param, axis, by } => BridgeSurfaceEvent::Nudge {
            param: param.to_owned(),
            axis: axis.map(axis_of),
            by,
        },
        Event::Reset { param, axis } => BridgeSurfaceEvent::Reset {
            param: param.to_owned(),
            axis: axis.map(axis_of),
        },
        Event::Action { action, down } => BridgeSurfaceEvent::Action {
            id: action.to_owned(),
            down,
        },
        Event::Menu { menu, step } => BridgeSurfaceEvent::Menu {
            id: menu.to_owned(),
            step,
        },
        Event::Jog { frames } => BridgeSurfaceEvent::Jog { frames },
        Event::Shuttle { step } => BridgeSurfaceEvent::Shuttle { step },
        Event::Tangent(status) => BridgeSurfaceEvent::Tangent {
            status: status_of(status),
        },
        Event::Ports(ports) => BridgeSurfaceEvent::Ports { ports },
        Event::Learned { control, encoding } => BridgeSurfaceEvent::Learned {
            control: control_of(control),
            encoding: encoding_of(encoding),
        },
    }
}
