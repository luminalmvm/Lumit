//! What Lumit says to the Hub, command by command: the handshake, a turned
//! knob passed on as an [`Event`], and the values the panels show.
//!
//! Pure, so the whole conversation can be checked without a Hub. The socket
//! and its thread are [`crate::tangent`]'s.

use std::collections::BTreeMap;

use crate::table::{self, MODE_GRADING};
use crate::tipc::{FromHub, ToHub, PROCESS_FIELD_FROM};
use crate::{Event, Shown};

/// What the Hub calls us. It must never change, or the Hub forgets the
/// user's own maps.
pub(crate) const APP_NAME: &str = "Lumit";

/// Shuttle counts to one step along the playback ladder: a quarter turn at
/// the Hub's usual 32 counts a turn.
const SHUTTLE_STEP: i32 = 8;

/// One connection's worth of conversation.
#[derive(Default)]
pub(crate) struct Exchange {
    /// What each display was last told, so only a change is sent.
    values: BTreeMap<u32, f32>,
    menus: BTreeMap<u32, String>,
    lines: Option<Vec<String>>,
    /// Shuttle counts not yet a whole step.
    shuttle: i32,
}

impl Exchange {
    /// Answer `command`. Replies go in `out` and what the person did in
    /// `events`. The Hub's panels come back from the handshake, by name.
    pub(crate) fn on(
        &mut self,
        command: FromHub,
        shown: &Shown,
        sys_dir: &str,
        out: &mut Vec<ToHub>,
        events: &mut Vec<Event>,
    ) -> Option<Vec<String>> {
        match command {
            FromHub::InitiateComms { revision, panels } => {
                *self = Exchange::default();
                out.push(ToHub::ApplicationDefinition {
                    app: APP_NAME.to_owned(),
                    sys_dir: sys_dir.to_owned(),
                    process: revision >= PROCESS_FIELD_FROM,
                });
                out.push(ToHub::ModeValue { id: MODE_GRADING });
                return Some(
                    panels
                        .iter()
                        .map(|&(kind, _)| panel_name(kind).to_owned())
                        .collect(),
                );
            }
            FromHub::ParameterChange { id, by } => {
                if let Some(p) = table::param(id) {
                    events.push(Event::Nudge {
                        param: p.param,
                        axis: p.axis,
                        by: f64::from(by),
                    });
                }
            }
            FromHub::ParameterReset { id } => {
                if let Some(p) = table::param(id) {
                    events.push(Event::Reset {
                        param: p.param,
                        axis: p.axis,
                    });
                }
            }
            FromHub::ParameterValueRequest { id } => {
                if let Some(p) = table::param(id) {
                    out.push(self.value(p, shown));
                }
            }
            FromHub::MenuChange { id, by } => {
                if let Some(m) = table::menu(id) {
                    // Never 0, which is a reset.
                    let step = if by < 0 { -1 } else { 1 };
                    events.push(Event::Menu { menu: m.menu, step });
                }
            }
            FromHub::MenuReset { id } => {
                if let Some(m) = table::menu(id) {
                    events.push(Event::Menu {
                        menu: m.menu,
                        step: 0,
                    });
                }
            }
            FromHub::MenuStringRequest { id } => {
                if let Some(m) = table::menu(id) {
                    out.push(self.menu(m, shown));
                }
            }
            FromHub::ActionOn { id } | FromHub::ActionOff { id } => {
                if let Some(a) = table::action(id) {
                    let down = matches!(command, FromHub::ActionOn { .. });
                    events.push(Event::Action {
                        action: a.action,
                        down,
                    });
                }
            }
            // One mode, so the panel is put back in it.
            FromHub::ModeChange { .. } => out.push(ToHub::ModeValue { id: MODE_GRADING }),
            FromHub::Transport { jog, shuttle } => {
                if jog != 0 {
                    events.push(Event::Jog { frames: jog });
                }
                self.shuttle += shuttle;
                while self.shuttle.abs() >= SHUTTLE_STEP {
                    let step = self.shuttle.signum();
                    self.shuttle -= step * SHUTTLE_STEP;
                    events.push(Event::Shuttle { step });
                }
            }
        }
        None
    }

    /// Tell the panels whatever in `shown` has changed since they were last
    /// told.
    pub(crate) fn refresh(&mut self, shown: &Shown, out: &mut Vec<ToHub>) {
        for p in table::PARAMS {
            let (value, _) = value_of(p, shown);
            if self.values.get(&p.id) != Some(&value) {
                out.push(self.value(p, shown));
            }
        }
        for m in table::MENUS {
            if self.menus.get(&m.id).map(String::as_str) != Some(item(m, shown).0) {
                out.push(self.menu(m, shown));
            }
        }
        let lines = status_lines(shown);
        if self.lines.as_ref() != Some(&lines) {
            // Another shot is another grade, so the Hub asks again for
            // everything it shows.
            if self
                .lines
                .as_ref()
                .is_some_and(|was| was.first() != lines.first())
            {
                out.push(ToHub::AllChange);
            }
            self.lines = Some(lines.clone());
            out.push(ToHub::DisplayText { lines });
        }
    }

    fn value(&mut self, p: &table::Param, shown: &Shown) -> ToHub {
        let (value, at_default) = value_of(p, shown);
        self.values.insert(p.id, value);
        ToHub::ParameterValue {
            id: p.id,
            value,
            at_default,
        }
    }

    fn menu(&mut self, m: &table::Menu, shown: &Shown) -> ToHub {
        let (text, at_default) = item(m, shown);
        self.menus.insert(m.id, text.to_owned());
        ToHub::MenuString {
            id: m.id,
            text: text.to_owned(),
            at_default,
        }
    }
}

/// The item `m` is on, and whether that is its first, which is where a
/// menu starts and what a reset goes back to.
fn item(m: &table::Menu, shown: &Shown) -> (&'static str, bool) {
    let place = shown
        .values
        .get(m.menu)
        .map(|place| place.max(0.0) as usize)
        .unwrap_or(0);
    let text = m
        .items
        .get(place)
        .or(m.items.first())
        .copied()
        .unwrap_or("");
    (text, place == 0)
}

/// The status lines: the shot, then its node.
fn status_lines(shown: &Shown) -> Vec<String> {
    let mut lines = vec![shown.shot.clone()];
    if let Some(node) = shown.values.get("node") {
        lines.push(format!("Node {}", node.max(0.0) as u32));
    }
    lines
}

/// `p`'s value as the interface last said, or its resting value when the
/// interface has nothing to say (no grade to move), and whether that is the
/// resting value.
fn value_of(p: &table::Param, shown: &Shown) -> (f32, bool) {
    let value = shown
        .values
        .get(&p.key())
        .map(|v| *v as f32)
        .unwrap_or(p.default);
    (value, (value - p.default).abs() < p.step / 2.0)
}

/// A panel's name from its type code, as Tangent names it.
pub(crate) fn panel_name(kind: u32) -> &'static str {
    match kind {
        0x03 => "CP200-BK",
        0x04 => "CP200-K",
        0x05 => "CP200-TS",
        0x09 => "CP200-S",
        0x0A => "Wave",
        0x0C => "Element-Tk",
        0x0D => "Element-Mf",
        0x0E => "Element-Kb",
        0x0F => "Element-Bt",
        0x11 => "Ripple",
        0x12 => "Arc-FCN",
        0x13 => "Arc-GRD",
        0x14 => "Arc-NAV",
        _ => "Tangent",
    }
}
