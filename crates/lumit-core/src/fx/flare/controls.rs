//! The controls each kind of element shows: the one place that says which
//! keys a kind has, their ranges and their defaults.

use super::design::Kind;

/// How a control is edited.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit {
    Slider,
    /// Whole numbers.
    Counter,
    Toggle,
    /// Degrees, drawn as a dial.
    Angle,
    /// One of these options, stored as its index.
    Choice(&'static [&'static str]),
}

/// The unit a value is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    None,
    /// Pixels at composition size.
    Px,
    Degrees,
    /// Degrees a second.
    DegreesPerSecond,
}

/// One control in the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Control {
    pub key: &'static str,
    pub label: &'static str,
    /// The twirl it sits in.
    pub group: &'static str,
    pub edit: Edit,
    pub unit: Unit,
    /// The slider's range.
    pub min: f32,
    pub max: f32,
    /// What typing may reach.
    pub hard_min: f32,
    pub hard_max: f32,
    pub default: f32,
}

const fn c(
    key: &'static str,
    label: &'static str,
    group: &'static str,
    edit: Edit,
    unit: Unit,
    range: (f32, f32, f32, f32),
    default: f32,
) -> Control {
    Control {
        key,
        label,
        group,
        edit,
        unit,
        min: range.0,
        max: range.1,
        hard_min: range.2,
        hard_max: range.3,
        default,
    }
}

const BIG: f32 = 1.0e5;
const UNIT: (f32, f32, f32, f32) = (0.0, 1.0, 0.0, 1.0);
const ANGLE: (f32, f32, f32, f32) = (-180.0, 180.0, -BIG, BIG);
const PLACE: (f32, f32, f32, f32) = (-3.0, 3.0, -10.0, 10.0);

/// The textures Ghost, Dirt and Picture can use, by index. 0 is none.
pub const TEXTURES: &[&str] = &["None", "Dust", "Smudge", "Caustic", "Rings", "Specks"];

/// What the element brightens towards.
pub const RESPONSES: &[&str] = &["Nothing", "The centre", "The edges"];

const LOOK: &str = "Look";
const SHAPE: &str = "Shape";
const RESPONSE: &str = "Response";
const IGNORE: &str = "Ignore";

use Edit::{Angle, Choice, Counter, Slider, Toggle};
use Unit::{Degrees, DegreesPerSecond, Px};

/// Size first, then brightness and rotation: every kind has these, and Size
/// is in each kind's own terms (a glow's radius, a ray's length).
const fn common(size: f32, size_max: f32) -> [Control; 3] {
    [
        c(
            "size",
            "Size",
            LOOK,
            Slider,
            Px,
            (0.0, size_max, 0.0, BIG),
            size,
        ),
        c(
            "brightness",
            "Brightness",
            LOOK,
            Slider,
            Unit::None,
            (0.0, 2.0, 0.0, 100.0),
            1.0,
        ),
        c("rotation", "Rotation", LOOK, Angle, Degrees, ANGLE, 0.0),
    ]
}

const SEED: Control = c(
    "seed",
    "Seed",
    LOOK,
    Counter,
    Unit::None,
    (0.0, 999.0, 0.0, 1.0e6),
    1.0,
);

/// Every kind ends with these.
const TAIL: [Control; 7] = [
    c(
        "response",
        "Brighter near",
        RESPONSE,
        Choice(RESPONSES),
        Unit::None,
        (0.0, 2.0, 0.0, 2.0),
        0.0,
    ),
    c(
        "response_amount",
        "Amount",
        RESPONSE,
        Slider,
        Unit::None,
        UNIT,
        1.0,
    ),
    c(
        "response_range",
        "Range",
        RESPONSE,
        Slider,
        Unit::None,
        UNIT,
        0.5,
    ),
    c("ignore_size", "Size", IGNORE, Toggle, Unit::None, UNIT, 0.0),
    c(
        "ignore_brightness",
        "Brightness",
        IGNORE,
        Toggle,
        Unit::None,
        UNIT,
        0.0,
    ),
    c(
        "ignore_rotation",
        "Rotation",
        IGNORE,
        Toggle,
        Unit::None,
        UNIT,
        0.0,
    ),
    c(
        "ignore_edge_fade",
        "Edge fade",
        IGNORE,
        Toggle,
        Unit::None,
        UNIT,
        0.0,
    ),
];

/// The iris shape Ghost and Ghosts share.
const GHOST_SHAPE: [Control; 10] = [
    c(
        "sides",
        "Sides",
        SHAPE,
        Counter,
        Unit::None,
        (0.0, 12.0, 0.0, 32.0),
        6.0,
    ),
    c(
        "roundness",
        "Roundness",
        SHAPE,
        Slider,
        Unit::None,
        UNIT,
        0.2,
    ),
    c("fill", "Fill", SHAPE, Slider, Unit::None, UNIT, 0.35),
    c("edge", "Edge", SHAPE, Slider, Unit::None, UNIT, 0.4),
    c(
        "edge_width",
        "Edge width",
        SHAPE,
        Slider,
        Unit::None,
        (0.0, 0.5, 0.0, 0.5),
        0.06,
    ),
    c("softness", "Softness", SHAPE, Slider, Unit::None, UNIT, 0.1),
    c("fringe", "Fringe", SHAPE, Slider, Unit::None, UNIT, 0.2),
    c(
        "stretch",
        "Stretch",
        SHAPE,
        Slider,
        Unit::None,
        (0.1, 4.0, 0.01, 100.0),
        1.0,
    ),
    c(
        "face",
        "Faces the light",
        SHAPE,
        Toggle,
        Unit::None,
        UNIT,
        0.0,
    ),
    c(
        "texture",
        "Texture",
        SHAPE,
        Choice(TEXTURES),
        Unit::None,
        (0.0, 5.0, 0.0, 5.0),
        0.0,
    ),
];

const GLOW: &[Control] = &[
    c("falloff", "Falloff", SHAPE, Slider, Unit::None, UNIT, 0.5),
    c(
        "core",
        "Core",
        SHAPE,
        Slider,
        Unit::None,
        (0.0, 4.0, 0.0, 100.0),
        1.0,
    ),
    c(
        "core_size",
        "Core size",
        SHAPE,
        Slider,
        Unit::None,
        UNIT,
        0.1,
    ),
    c(
        "stretch",
        "Stretch",
        SHAPE,
        Slider,
        Unit::None,
        (0.1, 10.0, 0.01, 100.0),
        1.0,
    ),
];

const RAYS: &[Control] = &[
    c(
        "count",
        "Count",
        SHAPE,
        Counter,
        Unit::None,
        (1.0, 400.0, 1.0, 2000.0),
        60.0,
    ),
    c(
        "length_var",
        "Length variation",
        SHAPE,
        Slider,
        Unit::None,
        UNIT,
        0.6,
    ),
    c(
        "thickness",
        "Thickness",
        SHAPE,
        Slider,
        Px,
        (0.2, 20.0, 0.01, 1000.0),
        1.0,
    ),
    c("softness", "Softness", SHAPE, Slider, Unit::None, UNIT, 0.5),
    c("taper", "Taper", SHAPE, Slider, Unit::None, UNIT, 1.0),
    c(
        "spread",
        "Spread",
        SHAPE,
        Slider,
        Degrees,
        (0.0, 360.0, 0.0, 360.0),
        360.0,
    ),
    c("shimmer", "Shimmer", SHAPE, Slider, Unit::None, UNIT, 0.3),
    c(
        "spin",
        "Spin",
        SHAPE,
        Slider,
        DegreesPerSecond,
        (-90.0, 90.0, -BIG, BIG),
        0.0,
    ),
    SEED,
];

const STAR: &[Control] = &[
    c(
        "points",
        "Points",
        SHAPE,
        Counter,
        Unit::None,
        (2.0, 16.0, 1.0, 64.0),
        6.0,
    ),
    c(
        "thickness",
        "Thickness",
        SHAPE,
        Slider,
        Px,
        (0.2, 20.0, 0.01, 1000.0),
        1.5,
    ),
    c("taper", "Taper", SHAPE, Slider, Unit::None, UNIT, 1.0),
    c("fringe", "Fringe", SHAPE, Slider, Unit::None, UNIT, 0.0),
    c("twin", "Second set", SHAPE, Slider, Unit::None, UNIT, 0.0),
    c(
        "twin_length",
        "Second set length",
        SHAPE,
        Slider,
        Unit::None,
        (0.0, 2.0, 0.0, 10.0),
        0.5,
    ),
];

const STREAK: &[Control] = &[
    c(
        "thickness",
        "Thickness",
        SHAPE,
        Slider,
        Px,
        (0.2, 40.0, 0.01, 1000.0),
        2.0,
    ),
    c(
        "lines",
        "Lines",
        SHAPE,
        Counter,
        Unit::None,
        (1.0, 15.0, 1.0, 64.0),
        1.0,
    ),
    c(
        "spacing",
        "Spacing",
        SHAPE,
        Slider,
        Px,
        (0.0, 100.0, 0.0, BIG),
        8.0,
    ),
    c("taper", "Taper", SHAPE, Slider, Unit::None, UNIT, 0.5),
    c("fringe", "Fringe", SHAPE, Slider, Unit::None, UNIT, 0.0),
    c(
        "face",
        "Points to the centre",
        SHAPE,
        Toggle,
        Unit::None,
        UNIT,
        0.0,
    ),
];

const RING: &[Control] = &[
    c(
        "thickness",
        "Thickness",
        SHAPE,
        Slider,
        Px,
        (0.5, 100.0, 0.01, BIG),
        4.0,
    ),
    c("softness", "Softness", SHAPE, Slider, Unit::None, UNIT, 0.5),
    c(
        "arc",
        "Arc",
        SHAPE,
        Slider,
        Degrees,
        (0.0, 360.0, 0.0, 360.0),
        360.0,
    ),
    c(
        "arc_feather",
        "Arc feather",
        SHAPE,
        Slider,
        Unit::None,
        UNIT,
        0.5,
    ),
    c("fringe", "Fringe", SHAPE, Slider, Unit::None, UNIT, 0.0),
];

const HALO: &[Control] = &[
    c(
        "thickness",
        "Thickness",
        SHAPE,
        Slider,
        Px,
        (0.5, 100.0, 0.01, BIG),
        6.0,
    ),
    c("softness", "Softness", SHAPE, Slider, Unit::None, UNIT, 0.5),
    c(
        "arc",
        "Arc",
        SHAPE,
        Slider,
        Degrees,
        (0.0, 360.0, 0.0, 360.0),
        140.0,
    ),
    c(
        "grow",
        "Grows away from the centre",
        SHAPE,
        Slider,
        Unit::None,
        (0.0, 2.0, 0.0, 10.0),
        0.5,
    ),
    c("fringe", "Fringe", SHAPE, Slider, Unit::None, UNIT, 1.0),
];

const GHOST: &[Control] = &[c("place", "Place", SHAPE, Slider, Unit::None, PLACE, 1.5)];

const GHOSTS: &[Control] = &[
    c(
        "count",
        "Count",
        SHAPE,
        Counter,
        Unit::None,
        (1.0, 60.0, 1.0, 500.0),
        8.0,
    ),
    c("from", "From", SHAPE, Slider, Unit::None, PLACE, 0.4),
    c("to", "To", SHAPE, Slider, Unit::None, PLACE, 2.2),
    c(
        "scatter",
        "Scatter",
        SHAPE,
        Slider,
        Px,
        (0.0, 200.0, 0.0, BIG),
        0.0,
    ),
    c(
        "size_var",
        "Size variation",
        SHAPE,
        Slider,
        Unit::None,
        UNIT,
        0.6,
    ),
    c(
        "bright_var",
        "Brightness variation",
        SHAPE,
        Slider,
        Unit::None,
        UNIT,
        0.6,
    ),
    SEED,
];

const ORBS: &[Control] = &[
    c(
        "count",
        "Count",
        SHAPE,
        Counter,
        Unit::None,
        (1.0, 300.0, 1.0, 2000.0),
        60.0,
    ),
    c(
        "size_var",
        "Size variation",
        SHAPE,
        Slider,
        Unit::None,
        UNIT,
        0.6,
    ),
    c(
        "reach",
        "Reach",
        SHAPE,
        Slider,
        Px,
        (10.0, 4000.0, 1.0, BIG),
        600.0,
    ),
    c("fill", "Fill", SHAPE, Slider, Unit::None, UNIT, 0.6),
    c("edge", "Edge", SHAPE, Slider, Unit::None, UNIT, 0.3),
    c("softness", "Softness", SHAPE, Slider, Unit::None, UNIT, 0.2),
    c(
        "drift",
        "Drift",
        SHAPE,
        Slider,
        Px,
        (0.0, 200.0, 0.0, BIG),
        20.0,
    ),
    SEED,
];

const DIRT: &[Control] = &[
    c(
        "texture",
        "Texture",
        SHAPE,
        Choice(TEXTURES),
        Unit::None,
        (1.0, 5.0, 1.0, 5.0),
        1.0,
    ),
    c(
        "reach",
        "Reach",
        SHAPE,
        Slider,
        Px,
        (10.0, 4000.0, 1.0, BIG),
        800.0,
    ),
    SEED,
];

const PICTURE: &[Control] = &[
    c(
        "texture",
        "Texture",
        SHAPE,
        Choice(TEXTURES),
        Unit::None,
        (1.0, 5.0, 1.0, 5.0),
        3.0,
    ),
    c("place", "Place", SHAPE, Slider, Unit::None, PLACE, 0.0),
    c(
        "stretch",
        "Stretch",
        SHAPE,
        Slider,
        Unit::None,
        (0.1, 4.0, 0.01, 100.0),
        1.0,
    ),
    c(
        "face",
        "Faces the light",
        SHAPE,
        Toggle,
        Unit::None,
        UNIT,
        0.0,
    ),
];

const fn size_of(kind: Kind) -> (f32, f32) {
    match kind {
        Kind::Glow => (120.0, 1000.0),
        Kind::Rays => (300.0, 2000.0),
        Kind::Star => (200.0, 2000.0),
        Kind::Streak => (600.0, 4000.0),
        Kind::Ring => (150.0, 1000.0),
        Kind::Halo => (250.0, 1500.0),
        Kind::Ghost => (60.0, 600.0),
        Kind::Ghosts => (40.0, 400.0),
        Kind::Orbs => (40.0, 400.0),
        Kind::Dirt => (1000.0, 4000.0),
        Kind::Picture => (100.0, 1000.0),
    }
}

const fn own(kind: Kind) -> &'static [Control] {
    match kind {
        Kind::Glow => GLOW,
        Kind::Rays => RAYS,
        Kind::Star => STAR,
        Kind::Streak => STREAK,
        Kind::Ring => RING,
        Kind::Halo => HALO,
        Kind::Ghost => GHOST,
        Kind::Ghosts => GHOSTS,
        Kind::Orbs => ORBS,
        Kind::Dirt => DIRT,
        Kind::Picture => PICTURE,
    }
}

/// Every control `kind` shows, in the window's order: its look, its shape,
/// then Response and Ignore.
pub fn controls(kind: Kind) -> impl Iterator<Item = Control> {
    let (size, size_max) = size_of(kind);
    let ghost_shape: &'static [Control] = match kind {
        Kind::Ghost | Kind::Ghosts => &GHOST_SHAPE,
        _ => &[],
    };
    common(size, size_max)
        .into_iter()
        .chain(own(kind).iter().copied())
        .chain(ghost_shape.iter().copied())
        .chain(TAIL)
}
