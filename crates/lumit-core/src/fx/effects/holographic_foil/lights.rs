//! Holographic foil's light rig: three spots and a panel fixed to the viewer,
//! and a soft grey studio around them. The Lighting choice picks one of eight
//! rigs; Custom reads the rows; Auto picks a rig for the Material. Runs
//! wherever the parameters are packed.

use super::{HolographicFoil, GLITTER, SHATTER, SPARKLE, VORTEX};

/// One light. A spot is a small sphere; the panel is a rectangle facing the
/// sheet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Light {
    pub on: bool,
    /// Direction cosines of the way to the light, seen from the sheet.
    pub x: f32,
    pub y: f32,
    /// A spot's angular diameter, or the panel's width along the line out
    /// from the view axis, degrees.
    pub size: f32,
    /// The panel's length across that line, degrees. Unused for a spot.
    pub length: f32,
    pub intensity: f32,
    /// Metres from the sheet's centre.
    pub distance: f32,
    /// 0 equal, 1 daylight, 2 tungsten, 3 LED, 4 fluorescent.
    pub spectrum: u32,
}

/// Every light and the studio's four levels: rims, fill, scrim, room.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Rig {
    pub spots: [Light; 3],
    pub panel: Light,
    pub studio: [f32; 4],
}

const fn spot(on: bool, x: f32, y: f32, size: f32, intensity: f32, distance: f32) -> Light {
    Light {
        on,
        x,
        y,
        size,
        length: 0.0,
        intensity,
        distance,
        spectrum: 0,
    }
}

const fn panel(on: bool, x: f32, y: f32, size: [f32; 2], intensity: f32, distance: f32) -> Light {
    Light {
        on,
        x,
        y,
        size: size[0],
        length: size[1],
        intensity,
        distance,
        spectrum: 0,
    }
}

const OFF: Light = spot(false, 0.0, 0.0, 1.0, 0.0, 1.0);

/// The rigs of the Lighting choice, in its order. Custom is the rows.
/// Studio is holospot's card scene: its four lights stand still while its
/// camera circles the card, so they are placed here to give the same
/// half-vectors at the sheet's centre at the default pose, with a soft
/// studio, so the rainbow fills the card over grey metal. The card there is
/// bowed, which Bow's default copies; flat, the spots' near-mirror bands sit
/// on the card as soft halos. The other rigs sit where the default pose catches them, a quarter
/// as bright, with a brighter studio, so bare foil reads as grey metal.
pub(super) const RIGS: [Rig; 8] = [
    // Studio.
    Rig {
        spots: [
            spot(true, -0.38, 0.57, 1.5, 970.0, 1.0),
            spot(true, -0.12, -0.17, 2.5, 485.0, 1.2),
            spot(true, -0.42, 0.71, 1.0, 970.0, 1.0),
        ],
        panel: panel(true, -0.43, 0.48, [14.0, 50.0], 1.6, 0.9),
        studio: [0.5, 0.25, 0.2, 0.04],
    },
    // Spot.
    Rig {
        spots: [spot(true, -0.5, 0.42, 1.5, 300.0, 1.0), OFF, OFF],
        panel: OFF,
        studio: [0.0, 0.0, 0.0, 0.08],
    },
    // Softbox.
    Rig {
        spots: [OFF, OFF, OFF],
        panel: panel(true, -0.55, 0.3, [40.0, 40.0], 0.6, 0.8),
        studio: [0.0, 0.2, 0.0, 0.15],
    },
    // Strip.
    Rig {
        spots: [OFF, OFF, OFF],
        panel: panel(true, -0.55, 0.3, [6.0, 60.0], 6.0, 0.9),
        studio: [0.0, 0.2, 0.0, 0.15],
    },
    // Dark.
    Rig {
        spots: [OFF, OFF, OFF],
        panel: OFF,
        studio: [3.0, 0.6, 0.0, 0.12],
    },
    // Desk: a daylight window and a tungsten lamp.
    Rig {
        spots: [
            Light {
                spectrum: 2,
                ..spot(true, -0.4, 0.5, 4.0, 80.0, 0.5)
            },
            OFF,
            OFF,
        ],
        panel: Light {
            spectrum: 1,
            ..panel(true, -0.7, 0.2, [30.0, 45.0], 0.6, 1.5)
        },
        studio: [0.0, 0.3, 0.0, 0.2],
    },
    // Office: one fluorescent strip overhead.
    Rig {
        spots: [OFF, OFF, OFF],
        panel: Light {
            spectrum: 4,
            ..panel(true, -0.45, 0.55, [4.0, 70.0], 6.0, 1.6)
        },
        studio: [0.0, 0.4, 0.0, 0.25],
    },
    // Raking.
    Rig {
        spots: [spot(true, -0.83, 0.52, 3.0, 400.0, 1.0), OFF, OFF],
        panel: OFF,
        studio: [0.0, 0.2, 0.3, 0.1],
    },
];

/// The Lighting choice that reads the rows.
pub(super) const CUSTOM: u32 = 8;

/// The Lighting choice that picks a rig for the Material.
const AUTO: u32 = 9;

/// Auto for Shatter: one bright spot and a broad tungsten panel whose mirror
/// lights part of the shards gold, so the cracks read as a mosaic of bright
/// and dark gold with a share of shards flashing colour.
const AUTO_SHATTER: Rig = Rig {
    spots: [spot(true, -0.4, 0.53, 1.5, 600.0, 1.0), OFF, OFF],
    panel: Light {
        spectrum: 2,
        ..panel(true, -0.5, 0.4, [45.0, 50.0], 0.7, 0.9)
    },
    studio: [0.5, 0.4, 0.3, 0.2],
};

/// Auto for Glitter: two spots away from the mirror angle over a dim
/// studio, so only the steeper flakes flash over the dark base ink.
const AUTO_GLITTER: Rig = Rig {
    spots: [
        spot(true, -0.3, 0.65, 1.5, 800.0, 1.0),
        spot(true, -0.2, 0.3, 2.5, 500.0, 1.2),
        OFF,
    ],
    panel: OFF,
    studio: [0.15, 0.08, 0.05, 0.04],
};

/// Auto for Sparkle: two wide spots whose rainbow bands cross the dots over
/// pale matte metal.
const AUTO_SPARKLE: Rig = Rig {
    spots: [
        spot(true, -0.55, 0.45, 2.0, 450.0, 1.0),
        spot(true, -0.3, 0.62, 2.0, 250.0, 1.0),
        OFF,
    ],
    panel: OFF,
    studio: [0.35, 0.55, 0.45, 0.16],
};

/// Auto for Vortex: the studio's spots draw the arms, and a broad tungsten
/// panel mirrors over most of the spiral so it reads as gold.
const AUTO_VORTEX: Rig = Rig {
    spots: RIGS[0].spots,
    panel: Light {
        spectrum: 2,
        ..panel(true, -0.5, 0.35, [50.0, 60.0], 0.5, 0.9)
    },
    studio: [1.0, 0.5, 0.4, 0.1],
};

/// The rig in play: a preset's, the Material's under Auto (Studio for
/// Prism), or the rows' under Custom.
pub(super) fn rig_of(f: &HolographicFoil) -> Rig {
    if f.lighting == AUTO {
        return match f.material {
            SHATTER => AUTO_SHATTER,
            GLITTER => AUTO_GLITTER,
            SPARKLE => AUTO_SPARKLE,
            VORTEX => AUTO_VORTEX,
            _ => RIGS[0],
        };
    }
    if let Some(rig) = RIGS.get(f.lighting as usize) {
        return *rig;
    }
    let s = |on, x, y, size, intensity, distance, spectrum| Light {
        on,
        x,
        y,
        size,
        length: 0.0,
        intensity,
        distance,
        spectrum,
    };
    Rig {
        spots: [
            s(
                f.spot_a_on,
                f.spot_a_x,
                f.spot_a_y,
                f.spot_a_size,
                f.spot_a_intensity,
                f.spot_a_distance,
                f.spot_a_spectrum,
            ),
            s(
                f.spot_b_on,
                f.spot_b_x,
                f.spot_b_y,
                f.spot_b_size,
                f.spot_b_intensity,
                f.spot_b_distance,
                f.spot_b_spectrum,
            ),
            s(
                f.spot_c_on,
                f.spot_c_x,
                f.spot_c_y,
                f.spot_c_size,
                f.spot_c_intensity,
                f.spot_c_distance,
                f.spot_c_spectrum,
            ),
        ],
        panel: Light {
            on: f.panel_on,
            x: f.panel_x,
            y: f.panel_y,
            size: f.panel_width,
            length: f.panel_length,
            intensity: f.panel_intensity,
            distance: f.panel_distance,
            spectrum: f.panel_spectrum,
        },
        studio: [f.rims, f.fill, f.scrim, f.room],
    }
}

/// The unit vector toward a pad direction `(x, y)`, turned by `rot` radians
/// about the view axis. A pad point past the rim stays just inside it.
pub(super) fn pad_to_dir(x: f64, y: f64, rot: f64) -> [f64; 3] {
    let len = x.hypot(y);
    let r = len.min(0.996);
    let (dx, dy) = if len > 1e-9 {
        (x * r / len, y * r / len)
    } else {
        (0.0, 0.0)
    };
    let (s, c) = rot.sin_cos();
    [
        dx * c - dy * s,
        dx * s + dy * c,
        (1.0 - r * r).max(0.0).sqrt(),
    ]
}

/// The two unit vectors across a direction `d`: along the line out from the
/// view axis, and around it.
pub(super) fn radial_axes(d: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let rho = d[0].hypot(d[1]);
    let (cp, sp) = if rho > 1e-9 {
        (d[0] / rho, d[1] / rho)
    } else {
        (1.0, 0.0)
    };
    let ct = d[2];
    let st = rho;
    ([ct * cp, ct * sp, -st], [-sp, cp, 0.0])
}

/// The studio's four soft panels: polar angle from the view axis, azimuth
/// counter-clockwise from +x, width along and across the line out, degrees,
/// and which studio level lights it.
pub(super) const STUDIO: [(f64, f64, f64, f64, usize); 4] = [
    (78.0, 0.0, 5.0, 60.0, 0),
    (78.0, 180.0, 5.0, 60.0, 0),
    (62.0, 90.0, 60.0, 60.0, 1),
    (20.0, 90.0, 20.0, 50.0, 2),
];
