//! Our `controls.xml` and the default map for each Tangent panel, written
//! from [`crate::table`] into the folder the Hub is pointed at.
//!
//! Written each time panels are turned on, so they always match the build
//! that is running and nothing has to be installed beside the app. The maps
//! are Lumit's defaults. A user's own maps, made in Tangent's Mapper, are
//! kept by the Hub elsewhere and are never touched.
//!
//! Control numbers are the ones in Tangent's panel layout drawings. Buttons
//! the Hub keeps for itself (the reset under each trackball, each knob's
//! push) are left alone.

use std::fmt::Write as _;
use std::io;
use std::path::Path;

use crate::table::{self, ACTIONS, MENUS, MODE_GRADING, PARAMS};

/// Tangent's own controls, which never reach us.
const ALT: u32 = 0x8000_0001;
const NEXT_KNOB_BANK: u32 = 0x8000_0002;
const PREV_KNOB_BANK: u32 = 0x8000_0003;
const NEXT_BUTTON_BANK: u32 = 0x8000_0004;
const PREV_BUTTON_BANK: u32 = 0x8000_0005;
const NEXT_BALL_BANK: u32 = 0x8000_0006;
const PREV_BALL_BANK: u32 = 0x8000_0007;
/// The transport ring: the Hub turns it into jog and shuttle.
const TRANSPORT_RING: u32 = 0x8100_0001;

/// What a control does: one of ours by name, or one of Tangent's.
#[derive(Clone, Copy)]
enum Key {
    Ours(&'static str),
    Tangent(u32),
}

use Key::{Ours, Tangent};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Button,
    Encoder,
}

/// One physical control's mapping, normally and with Alt held.
#[derive(Clone, Copy)]
struct Control {
    kind: Kind,
    number: u32,
    std: Key,
    alt: Option<Key>,
}

const fn button(number: u32, std: Key) -> Control {
    Control {
        kind: Kind::Button,
        number,
        std,
        alt: None,
    }
}

const fn encoder(number: u32, std: Key) -> Control {
    Control {
        kind: Kind::Encoder,
        number,
        std,
        alt: None,
    }
}

const fn both(kind: Kind, number: u32, std: u32, alt: u32) -> Control {
    Control {
        kind,
        number,
        std: Tangent(std),
        alt: Some(Tangent(alt)),
    }
}

/// One panel's default map: its Tangent name, its file, and its control
/// banks, each a `ControlBank` id with one list of controls per bank.
struct PanelMap {
    kind: &'static str,
    file: &'static str,
    banks: Vec<(&'static str, Vec<Vec<Control>>)>,
}

/// Each run of `numbers` given `keys` in turn.
fn row(kind: Kind, numbers: &[u32], keys: &[&'static str]) -> Vec<Control> {
    numbers
        .iter()
        .zip(keys)
        .map(|(&number, &key)| Control {
            kind,
            number,
            std: Ours(key),
            alt: None,
        })
        .collect()
}

const PRIMARY_BALLS: [&str; 9] = [
    "lift.x",
    "lift.y",
    "lift.master",
    "gamma.x",
    "gamma.y",
    "gamma.master",
    "gain.x",
    "gain.y",
    "gain.master",
];
const LOG_BALLS: [&str; 9] = [
    "shadow.x",
    "shadow.y",
    "shadow.master",
    "midtone.x",
    "midtone.y",
    "midtone.master",
    "highlight.x",
    "highlight.y",
    "highlight.master",
];
const BUTTONS_A: [&str; 12] = [
    "shot_previous",
    "shot_next",
    "still_grab",
    "node_previous",
    "node_next",
    "node_bypass",
    "node_add_serial",
    "node_add_parallel",
    "node_add_layer",
    "undo",
    "redo",
    "node_reset",
];
const BUTTONS_B: [&str; 12] = [
    "page_wheels",
    "page_bars",
    "page_log",
    "node_add_outside",
    "grade_bypass",
    "grade_page",
    "wheel_mode",
    "reset_lift",
    "reset_gamma",
    "reset_gain",
    "reset_offset",
    "page_hdr",
];
const KNOBS_A: [&str; 12] = [
    "contrast",
    "pivot",
    "saturation",
    "hue",
    "lum_mix",
    "temperature",
    "tint",
    "colour_boost",
    "shadows",
    "highlights",
    "midtone_detail",
    "exposure",
];
const KNOBS_B: [&str; 11] = [
    "shadow.x",
    "shadow.y",
    "shadow.master",
    "midtone.x",
    "midtone.y",
    "midtone.master",
    "highlight.x",
    "highlight.y",
    "highlight.master",
    "low_range",
    "high_range",
];

const TWELVE: [u32; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

/// Element-Tk and Ripple are numbered alike: A and B, then each trackball's
/// X, Y and ring.
fn trackballs(kind: &'static str, file: &'static str) -> PanelMap {
    PanelMap {
        kind,
        file,
        banks: vec![
            (
                "Standard",
                vec![vec![
                    both(Kind::Button, 0, ALT, ALT),
                    both(Kind::Button, 7, NEXT_BALL_BANK, PREV_BALL_BANK),
                ]],
            ),
            (
                "Trackerball",
                vec![
                    row(Kind::Encoder, &[0, 1, 2, 3, 4, 5, 6, 7, 8], &PRIMARY_BALLS),
                    row(Kind::Encoder, &[0, 1, 2, 3, 4, 5, 6, 7, 8], &LOG_BALLS),
                ],
            ),
        ],
    }
}

fn panels() -> Vec<PanelMap> {
    let offset = || {
        let mut ball = row(
            Kind::Encoder,
            &[0, 1, 2],
            &["offset.x", "offset.y", "offset.master"],
        );
        // Alt and the ring jogs and shuttles.
        if let Some(ring) = ball.get_mut(2) {
            ring.alt = Some(Tangent(TRANSPORT_RING));
        }
        ball
    };
    let wave_buttons = [10, 11, 12, 15, 16, 17, 20, 21, 22];
    vec![
        trackballs("Element-Tk", "element-tk-map.xml"),
        trackballs("Ripple", "ripple-map.xml"),
        PanelMap {
            kind: "Element-Mf",
            file: "element-mf-map.xml",
            banks: vec![
                (
                    "Standard",
                    vec![vec![
                        both(Kind::Button, 12, ALT, ALT),
                        button(14, Ours("play_stop")),
                        button(15, Ours("play_stop")),
                        button(16, Ours("frame_back")),
                        button(17, Ours("frame_forward")),
                        both(Kind::Button, 18, NEXT_BUTTON_BANK, PREV_BUTTON_BANK),
                    ]],
                ),
                // Two banks alike, so stepping the Tk's banks leaves Offset
                // where it is.
                ("Trackerball", vec![offset(), offset()]),
                (
                    "Button",
                    vec![
                        row(Kind::Button, &TWELVE, &BUTTONS_A),
                        row(Kind::Button, &TWELVE, &BUTTONS_B),
                    ],
                ),
            ],
        },
        PanelMap {
            kind: "Element-Kb",
            file: "element-kb-map.xml",
            banks: vec![
                (
                    "Standard",
                    vec![vec![
                        both(Kind::Button, 12, ALT, ALT),
                        both(Kind::Button, 13, NEXT_KNOB_BANK, PREV_KNOB_BANK),
                    ]],
                ),
                (
                    "Encoder",
                    vec![
                        row(Kind::Encoder, &TWELVE, &KNOBS_A),
                        row(Kind::Encoder, &TWELVE, &KNOBS_B),
                    ],
                ),
            ],
        },
        PanelMap {
            kind: "Element-Bt",
            file: "element-bt-map.xml",
            banks: vec![
                (
                    "Standard",
                    vec![vec![
                        both(Kind::Button, 12, ALT, ALT),
                        both(Kind::Button, 13, NEXT_BUTTON_BANK, PREV_BUTTON_BANK),
                    ]],
                ),
                (
                    "Button",
                    vec![
                        row(Kind::Button, &TWELVE, &BUTTONS_A),
                        row(Kind::Button, &TWELVE, &BUTTONS_B),
                    ],
                ),
            ],
        },
        PanelMap {
            kind: "Wave",
            file: "wave-map.xml",
            banks: vec![
                (
                    "Standard",
                    vec![vec![
                        both(Kind::Button, 9, ALT, ALT),
                        both(Kind::Button, 25, NEXT_KNOB_BANK, PREV_KNOB_BANK),
                        both(Kind::Button, 26, NEXT_BUTTON_BANK, PREV_BUTTON_BANK),
                        both(Kind::Button, 27, NEXT_BALL_BANK, PREV_BALL_BANK),
                        button(28, Ours("grade_bypass")),
                        button(29, Ours("node_bypass")),
                        button(30, Ours("shot_previous")),
                        button(31, Ours("shot_next")),
                        button(32, Ours("still_grab")),
                        button(33, Ours("undo")),
                        button(34, Ours("redo")),
                        button(35, Ours("node_reset")),
                        button(36, Ours("frame_back")),
                        button(37, Ours("frame_forward")),
                        button(39, Ours("play_stop")),
                        button(40, Ours("play_stop")),
                        encoder(12, Tangent(TRANSPORT_RING)),
                    ]],
                ),
                (
                    "Trackerball",
                    vec![
                        row(
                            Kind::Encoder,
                            &[13, 14, 9, 15, 16, 10, 17, 18, 11],
                            &PRIMARY_BALLS,
                        ),
                        row(
                            Kind::Encoder,
                            &[13, 14, 9, 15, 16, 10, 17, 18, 11],
                            &LOG_BALLS,
                        ),
                    ],
                ),
                (
                    "Encoder",
                    vec![
                        row(
                            Kind::Encoder,
                            &[0, 1, 2, 3, 4, 5, 6, 7, 8],
                            &[
                                "contrast",
                                "pivot",
                                "saturation",
                                "hue",
                                "lum_mix",
                                "temperature",
                                "tint",
                                "colour_boost",
                                "exposure",
                            ],
                        ),
                        row(
                            Kind::Encoder,
                            &[0, 1, 2, 3, 4, 5, 6, 7],
                            &[
                                "shadows",
                                "highlights",
                                "midtone_detail",
                                "low_range",
                                "high_range",
                                "offset.x",
                                "offset.y",
                                "offset.master",
                            ],
                        ),
                    ],
                ),
                (
                    "Button",
                    vec![
                        row(
                            Kind::Button,
                            &wave_buttons,
                            &[
                                "shot_previous",
                                "shot_next",
                                "still_grab",
                                "node_previous",
                                "node_next",
                                "node_add_serial",
                                "node_add_parallel",
                                "node_add_layer",
                                "node_add_outside",
                            ],
                        ),
                        row(
                            Kind::Button,
                            &wave_buttons,
                            &[
                                "page_wheels",
                                "page_bars",
                                "page_log",
                                "grade_page",
                                "wheel_mode",
                                "reset_lift",
                                "reset_gamma",
                                "reset_gain",
                                "reset_offset",
                            ],
                        ),
                    ],
                ),
            ],
        },
    ]
}

/// Write `controls.xml` and every panel's default map into `folder`.
pub(crate) fn write_all(folder: &Path) -> io::Result<()> {
    std::fs::create_dir_all(folder)?;
    std::fs::write(folder.join("controls.xml"), controls())?;
    for panel in panels() {
        std::fs::write(folder.join(panel.file), map(&panel))?;
    }
    Ok(())
}

const HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// `controls.xml`: the mode, then every control in the Mapper's groups.
fn controls() -> String {
    let mut x = String::from(HEAD);
    x.push_str("<TangentWave fileType=\"ControlSystem\" fileVersion=\"3.0\">\n");
    x.push_str("  <Capabilities>\n");
    x.push_str("    <Jog enabled=\"true\"/>\n");
    x.push_str("    <Shuttle enabled=\"true\"/>\n");
    x.push_str("    <StatusDisplay lineCount=\"3\"/>\n");
    x.push_str("  </Capabilities>\n");
    let _ = writeln!(
        x,
        "  <Modes>\n    <Mode id=\"{MODE_GRADING:#010X}\">\n      <Name>Grading</Name>\n    </Mode>\n  </Modes>"
    );
    x.push_str("  <Controls>\n");
    let mut groups: Vec<&str> = Vec::new();
    for group in PARAMS
        .iter()
        .map(|p| p.group)
        .chain(ACTIONS.iter().map(|a| a.group))
    {
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    for group in groups {
        let _ = writeln!(x, "    <Group name=\"{group}\">");
        for p in PARAMS.iter().filter(|p| p.group == group) {
            let _ = writeln!(
                x,
                "      <Parameter id=\"{:#010X}\">\n        <Name>{}</Name>\n        <Name9>{}</Name9>\n        <MinValue>{}</MinValue>\n        <MaxValue>{}</MaxValue>\n        <StepSize>{}</StepSize>\n        <DefaultDisplay>Bar</DefaultDisplay>\n      </Parameter>",
                p.id, p.name, p.short, p.min, p.max, p.step
            );
        }
        // A trackball takes a wheel's X, Y and Master together.
        for p in PARAMS
            .iter()
            .filter(|p| p.group == group && p.axis == Some(table::Axis::X))
        {
            let members = [table::Axis::X, table::Axis::Y, table::Axis::Master]
                .iter()
                .filter_map(|&axis| table::param_named(p.param, Some(axis)))
                .map(|m| format!("        <Member id=\"{:#010X}\"/>\n", m.id))
                .collect::<String>();
            let name = p.name.trim_end_matches(" X");
            let _ = write!(
                x,
                "      <Binding name=\"{name}\">\n{members}      </Binding>\n"
            );
        }
        for a in ACTIONS.iter().filter(|a| a.group == group) {
            let _ = writeln!(
                x,
                "      <Action id=\"{:#010X}\">\n        <Name>{}</Name>\n        <Name9>{}</Name9>\n      </Action>",
                a.id, a.name, a.short
            );
        }
        // The two menus sit with the pages they choose between.
        if group == "Pages" {
            for m in MENUS {
                let _ = writeln!(
                    x,
                    "      <Menu id=\"{:#010X}\">\n        <Name>{}</Name>\n        <Name9>{}</Name9>\n      </Menu>",
                    m.id, m.name, m.short
                );
            }
        }
        x.push_str("    </Group>\n");
    }
    x.push_str("  </Controls>\n</TangentWave>\n");
    x
}

/// One panel's default map file.
fn map(panel: &PanelMap) -> String {
    let mut x = String::from(HEAD);
    x.push_str("<TangentWave fileType=\"PanelMap\" fileVersion=\"3.0\">\n  <Panels>\n");
    let _ = writeln!(x, "    <Panel type=\"{}\">", panel.kind);
    let _ = writeln!(x, "      <Mode id=\"{MODE_GRADING:#010X}\">");
    for (bank_id, banks) in &panel.banks {
        let _ = writeln!(x, "        <ControlBank id=\"{bank_id}\">");
        for bank in banks {
            x.push_str("          <Bank>\n");
            for control in bank {
                write_control(&mut x, control);
            }
            x.push_str("          </Bank>\n");
        }
        x.push_str("        </ControlBank>\n");
    }
    x.push_str("      </Mode>\n    </Panel>\n  </Panels>\n</TangentWave>\n");
    x
}

fn write_control(x: &mut String, control: &Control) {
    let kind = match control.kind {
        Kind::Button => "Button",
        Kind::Encoder => "Encoder",
    };
    let _ = writeln!(
        x,
        "            <Control type=\"{kind}\" number=\"{}\">",
        control.number
    );
    for (mode, key) in [("Std", Some(control.std)), ("Alt", control.alt)] {
        let Some(key) = key else { continue };
        let (id, extra) = match key {
            Tangent(id) => (Some(id), ""),
            Ours(name) => {
                let id = table::id_of(name);
                let extra = if MENUS.iter().any(|m| m.menu == name) {
                    // A button steps a menu forwards.
                    "                <Argument>0</Argument>\n"
                } else if control.kind == Kind::Encoder {
                    "                <Display>Bar</Display>\n"
                } else {
                    ""
                };
                (id, extra)
            }
        };
        // Every name in the maps is in the table; one that is not is left
        // out rather than written as a control that does nothing.
        let Some(id) = id else { continue };
        let _ = write!(
            x,
            "              <Mapping mode=\"{mode}\">\n                <Key>{id:#010X}</Key>\n{extra}              </Mapping>\n"
        );
    }
    x.push_str("            </Control>\n");
}
