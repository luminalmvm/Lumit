//! Every control a surface can reach, once: the Correction row and axis a
//! knob or trackball moves, the actions a button runs, and the two menus.
//!
//! The Tangent ids here are the ones our `controls.xml` and panel maps are
//! written from ([`crate::xml`]), so they can never drift apart, and they are
//! stable: a user's own Tangent map names them, so an id is never reused.
//! MIDI learn offers the same rows and actions by name.
//!
//! The names are what the Tangent Mapper lists and the panels' displays
//! show. Those displays only draw ASCII, so they are English here rather
//! than in the interface's translations.

/// Which part of a wheel a control moves. A wheel is a colour row: X and Y
/// move its chroma as the Grade panel's wheel does, Master moves all three
/// channels together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    X,
    Y,
    Master,
}

impl Axis {
    /// The axis's part of a value's key: `lift.x`.
    pub fn key(self) -> &'static str {
        match self {
            Axis::X => "x",
            Axis::Y => "y",
            Axis::Master => "master",
        }
    }
}

/// One number a knob, a ring or a trackball axis moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Param {
    /// The Tangent id.
    pub id: u32,
    /// The Correction row it moves.
    pub param: &'static str,
    /// For a wheel, which part of it. None for an ordinary number row.
    pub axis: Option<Axis>,
    /// The name the Mapper lists.
    pub name: &'static str,
    /// The label on a panel display, nine characters at most.
    pub short: &'static str,
    /// The span a surface moves it over, in the row's own units.
    pub min: f32,
    pub max: f32,
    /// Where it sits at rest, which the panels mark.
    pub default: f32,
    /// The smallest step, in the row's own units.
    pub step: f32,
    /// The Mapper's group it is listed under.
    pub group: &'static str,
}

impl Param {
    /// The key its value is pushed back under: `contrast`, or `lift.x`.
    pub fn key(&self) -> String {
        match self.axis {
            Some(axis) => format!("{}.{}", self.param, axis.key()),
            None => self.param.to_owned(),
        }
    }
}

/// One thing a button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: u32,
    /// The action's own name, which the interface acts on.
    pub action: &'static str,
    pub name: &'static str,
    pub short: &'static str,
    pub group: &'static str,
}

/// A list a button or knob steps through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    pub id: u32,
    pub menu: &'static str,
    pub name: &'static str,
    pub short: &'static str,
    /// What the panels show for each item, first to last. The interface
    /// pushes back which one is current, by its place.
    pub items: &'static [&'static str],
}

/// The one mode: everything is live at once while grading.
pub const MODE_GRADING: u32 = 0x0001_0001;

const WHEELS: &str = "Wheels";
const LOG: &str = "Log wheels";
const ADJUST: &str = "Adjust";

#[allow(clippy::too_many_arguments)]
const fn p(
    id: u32,
    param: &'static str,
    axis: Option<Axis>,
    name: &'static str,
    short: &'static str,
    (min, max, default, step): (f32, f32, f32, f32),
    group: &'static str,
) -> Param {
    Param {
        id,
        param,
        axis,
        name,
        short,
        min,
        max,
        default,
        step,
        group,
    }
}

const CHROMA: (f32, f32, f32, f32) = (-1.0, 1.0, 0.0, 0.001);
const X: Option<Axis> = Some(Axis::X);
const Y: Option<Axis> = Some(Axis::Y);
const M: Option<Axis> = Some(Axis::Master);

/// Every number a surface moves. Wheels come in threes, X, Y then Master,
/// which is the order a trackball binding takes them in.
pub const PARAMS: &[Param] = &[
    p(0x0002_0101, "lift", X, "Lift X", "Lift X", CHROMA, WHEELS),
    p(0x0002_0102, "lift", Y, "Lift Y", "Lift Y", CHROMA, WHEELS),
    p(
        0x0002_0103,
        "lift",
        M,
        "Lift Master",
        "Lift",
        CHROMA,
        WHEELS,
    ),
    p(
        0x0002_0111,
        "gamma",
        X,
        "Gamma X",
        "Gamma X",
        CHROMA,
        WHEELS,
    ),
    p(
        0x0002_0112,
        "gamma",
        Y,
        "Gamma Y",
        "Gamma Y",
        CHROMA,
        WHEELS,
    ),
    p(
        0x0002_0113,
        "gamma",
        M,
        "Gamma Master",
        "Gamma",
        CHROMA,
        WHEELS,
    ),
    p(0x0002_0121, "gain", X, "Gain X", "Gain X", CHROMA, WHEELS),
    p(0x0002_0122, "gain", Y, "Gain Y", "Gain Y", CHROMA, WHEELS),
    p(
        0x0002_0123,
        "gain",
        M,
        "Gain Master",
        "Gain",
        (0.0, 4.0, 1.0, 0.002),
        WHEELS,
    ),
    p(
        0x0002_0131,
        "offset",
        X,
        "Offset X",
        "Offset X",
        CHROMA,
        WHEELS,
    ),
    p(
        0x0002_0132,
        "offset",
        Y,
        "Offset Y",
        "Offset Y",
        CHROMA,
        WHEELS,
    ),
    p(
        0x0002_0133,
        "offset",
        M,
        "Offset Master",
        "Offset",
        CHROMA,
        WHEELS,
    ),
    p(
        0x0002_0141,
        "shadow",
        X,
        "Shadow X",
        "Shadow X",
        CHROMA,
        LOG,
    ),
    p(
        0x0002_0142,
        "shadow",
        Y,
        "Shadow Y",
        "Shadow Y",
        CHROMA,
        LOG,
    ),
    p(
        0x0002_0143,
        "shadow",
        M,
        "Shadow Master",
        "Shadow",
        CHROMA,
        LOG,
    ),
    p(0x0002_0151, "midtone", X, "Midtone X", "Mid X", CHROMA, LOG),
    p(0x0002_0152, "midtone", Y, "Midtone Y", "Mid Y", CHROMA, LOG),
    p(
        0x0002_0153,
        "midtone",
        M,
        "Midtone Master",
        "Midtone",
        CHROMA,
        LOG,
    ),
    p(
        0x0002_0161,
        "highlight",
        X,
        "Highlight X",
        "High X",
        CHROMA,
        LOG,
    ),
    p(
        0x0002_0162,
        "highlight",
        Y,
        "Highlight Y",
        "High Y",
        CHROMA,
        LOG,
    ),
    p(
        0x0002_0163,
        "highlight",
        M,
        "Highlight Master",
        "Highlight",
        CHROMA,
        LOG,
    ),
    p(
        0x0002_0171,
        "low_range",
        None,
        "Low Range",
        "Low rng",
        (0.0, 1.0, 0.333, 0.001),
        LOG,
    ),
    p(
        0x0002_0172,
        "high_range",
        None,
        "High Range",
        "High rng",
        (0.0, 1.0, 0.55, 0.001),
        LOG,
    ),
    p(
        0x0002_0201,
        "contrast",
        None,
        "Contrast",
        "Contrast",
        (0.0, 200.0, 100.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_0202,
        "pivot",
        None,
        "Pivot",
        "Pivot",
        (0.0, 1.0, 0.435, 0.001),
        ADJUST,
    ),
    p(
        0x0002_0203,
        "saturation",
        None,
        "Saturation",
        "Sat",
        (0.0, 200.0, 100.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_0204,
        "hue",
        None,
        "Hue",
        "Hue",
        (-180.0, 180.0, 0.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_0205,
        "lum_mix",
        None,
        "Lum Mix",
        "Lum mix",
        (0.0, 100.0, 100.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_0206,
        "temperature",
        None,
        "Temperature",
        "Temp",
        (-150.0, 150.0, 0.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_0207,
        "tint",
        None,
        "Tint",
        "Tint",
        (-150.0, 150.0, 0.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_0208,
        "colour_boost",
        None,
        "Colour Boost",
        "Boost",
        (-100.0, 100.0, 0.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_0209,
        "shadows",
        None,
        "Shadows",
        "Shadows",
        (-100.0, 100.0, 0.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_020A,
        "highlights",
        None,
        "Highlights",
        "Hilights",
        (-100.0, 100.0, 0.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_020B,
        "midtone_detail",
        None,
        "Midtone Detail",
        "Mid det",
        (-100.0, 100.0, 0.0, 0.1),
        ADJUST,
    ),
    p(
        0x0002_020C,
        "exposure",
        None,
        "Exposure",
        "Exposure",
        (-5.0, 5.0, 0.0, 0.01),
        ADJUST,
    ),
];

const fn a(
    id: u32,
    action: &'static str,
    name: &'static str,
    short: &'static str,
    group: &'static str,
) -> Action {
    Action {
        id,
        action,
        name,
        short,
        group,
    }
}

const SHOTS: &str = "Shots";
const NODES: &str = "Nodes";
const GRADE: &str = "Grade";
const TRANSPORT: &str = "Transport";
const EDIT: &str = "Edit";
const PAGES: &str = "Pages";

/// Every action a button can run.
pub const ACTIONS: &[Action] = &[
    a(
        0x0003_0001,
        "shot_previous",
        "Previous Shot",
        "Prev shot",
        SHOTS,
    ),
    a(0x0003_0002, "shot_next", "Next Shot", "Next shot", SHOTS),
    a(
        0x0003_0011,
        "node_previous",
        "Previous Node",
        "Prev node",
        NODES,
    ),
    a(0x0003_0012, "node_next", "Next Node", "Next node", NODES),
    a(
        0x0003_0013,
        "node_add_serial",
        "Add Serial Node",
        "Serial",
        NODES,
    ),
    a(
        0x0003_0014,
        "node_add_parallel",
        "Add Parallel Node",
        "Parallel",
        NODES,
    ),
    a(
        0x0003_0015,
        "node_add_layer",
        "Add Layer Node",
        "Layer",
        NODES,
    ),
    a(
        0x0003_0016,
        "node_add_outside",
        "Add Outside Node",
        "Outside",
        NODES,
    ),
    a(0x0003_0017, "node_bypass", "Bypass Node", "Byp node", NODES),
    a(0x0003_0018, "node_reset", "Reset Node", "Rst node", NODES),
    a(0x0003_0021, "reset_lift", "Reset Lift", "Rst lift", GRADE),
    a(
        0x0003_0022,
        "reset_gamma",
        "Reset Gamma",
        "Rst gamma",
        GRADE,
    ),
    a(0x0003_0023, "reset_gain", "Reset Gain", "Rst gain", GRADE),
    a(
        0x0003_0024,
        "reset_offset",
        "Reset Offset",
        "Rst offs",
        GRADE,
    ),
    a(
        0x0003_0025,
        "reset_shadow",
        "Reset Shadow",
        "Rst shad",
        GRADE,
    ),
    a(
        0x0003_0026,
        "reset_midtone",
        "Reset Midtone",
        "Rst mid",
        GRADE,
    ),
    a(
        0x0003_0027,
        "reset_highlight",
        "Reset Highlight",
        "Rst high",
        GRADE,
    ),
    a(
        0x0003_0031,
        "grade_bypass",
        "Bypass Grade",
        "Byp grade",
        GRADE,
    ),
    a(0x0003_0032, "still_grab", "Grab Still", "Grab", GRADE),
    a(0x0003_0041, "play_stop", "Play or Stop", "Play", TRANSPORT),
    a(
        0x0003_0042,
        "frame_back",
        "Back a Frame",
        "Frame -",
        TRANSPORT,
    ),
    a(
        0x0003_0043,
        "frame_forward",
        "Forward a Frame",
        "Frame +",
        TRANSPORT,
    ),
    a(0x0003_0051, "undo", "Undo", "Undo", EDIT),
    a(0x0003_0052, "redo", "Redo", "Redo", EDIT),
    a(0x0003_0061, "page_wheels", "Wheels Page", "Wheels", PAGES),
    a(0x0003_0062, "page_bars", "Bars Page", "Bars", PAGES),
    a(0x0003_0063, "page_log", "Log Page", "Log", PAGES),
    a(0x0003_0064, "page_hdr", "HDR Page", "HDR", PAGES),
];

/// The two lists: which page the Grade panel shows, and how its wheels are
/// drawn.
pub const MENUS: &[Menu] = &[
    Menu {
        id: 0x0004_0001,
        menu: "grade_page",
        name: "Grade Page",
        short: "Page",
        items: &[
            "Wheels",
            "Adjust",
            "Curves",
            "Qualifier",
            "Windows",
            "Match",
            "Look",
        ],
    },
    Menu {
        id: 0x0004_0002,
        menu: "wheel_mode",
        name: "Wheel Mode",
        short: "Wheels",
        items: &["Wheels", "Bars", "Log", "HDR"],
    },
];

/// The number with Tangent id `id`.
pub fn param(id: u32) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.id == id)
}

/// The number that moves row `param` on `axis`.
pub fn param_named(param: &str, axis: Option<Axis>) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.param == param && p.axis == axis)
}

/// The action with Tangent id `id`.
pub fn action(id: u32) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.id == id)
}

/// The action called `action`.
pub fn action_named(action: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.action == action)
}

/// The menu with Tangent id `id`.
pub fn menu(id: u32) -> Option<&'static Menu> {
    MENUS.iter().find(|m| m.id == id)
}

/// The Tangent id of the number on `param`/`axis`, of the action or of the
/// menu called `name`. Only for the panel maps, which name every control by
/// what it does, so a wrong name there is a bug caught as the map is
/// written.
pub(crate) fn id_of(name: &str) -> Option<u32> {
    if let Some((param, axis)) = name.split_once('.') {
        let axis = match axis {
            "x" => Axis::X,
            "y" => Axis::Y,
            "master" => Axis::Master,
            _ => return None,
        };
        return param_named(param, Some(axis)).map(|p| p.id);
    }
    param_named(name, None)
        .map(|p| p.id)
        .or_else(|| action_named(name).map(|a| a.id))
        .or_else(|| MENUS.iter().find(|m| m.menu == name).map(|m| m.id))
}
