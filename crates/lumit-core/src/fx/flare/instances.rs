//! A design turned into instances: one small description per quad drawn,
//! made again every frame. Each is relative to a light, so the same list is
//! drawn once for every light, by [`super::raster::place`] on the CPU and by
//! the same steps in the GPU's vertex shader.

use std::f32::consts::TAU;

use super::design::{ColourMode, Design, Element, Kind};
use super::raster::Frame;
use crate::fx::noise::hash01;

/// How many floats an instance packs into, which the WGSL reads as nine
/// `vec4`s.
pub const FLOATS: usize = 36;

/// Most instances a frame draws for each light. Every count control is
/// capped well below this, so only a design with a great many big elements
/// meets it, and then the last elements are dropped.
pub const MAX: usize = 16_384;

/// Where the instance sits before its offset: on the light.
pub const ON_LIGHT: u32 = 0;
/// At `at[0]` along the axis: 0 the light, 1 the centre.
pub const ON_AXIS: u32 = 1;
/// At `at`, raster pixels.
pub const ON_FRAME: u32 = 2;

/// A soft round light.
pub const GLOW: u32 = 0;
/// A line, from the light out or both ways through it.
pub const RAY: u32 = 1;
/// A ring, whole or an arc.
pub const ANNULUS: u32 = 2;
/// A polygon or a circle with a fill and a rim.
pub const IRIS: u32 = 3;

/// Its angle is measured from the axis rather than from the frame.
pub const FOLLOWS_AXIS: u32 = 1;
/// The light fading out past the frame's edge leaves it alone.
pub const NO_EDGE_FADE: u32 = 2;
/// The quad follows the axis but the polygon in it stays put on screen.
pub const STILL_POLYGON: u32 = 4;

/// One quad, before it is put on a light.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Instance {
    /// [`ON_LIGHT`], [`ON_AXIS`] or [`ON_FRAME`].
    pub anchor: u32,
    pub at: [f32; 2],
    pub flags: u32,
    /// Pixels from the anchor to the quad's middle, along the quad's own
    /// turned axes.
    pub offset: [f32; 2],
    /// Half the quad's width and height, pixels.
    pub half: [f32; 2],
    /// Radians, clockwise on screen.
    pub angle: f32,
    pub shape: u32,
    /// How much bigger it gets as the light moves away from the centre, at
    /// the frame's corner.
    pub grow: f32,
    /// Pixels: only lit while the light is nearer than this. 0 is always.
    pub reach: f32,
    pub colour: [f32; 3],
    pub gain: f32,
    /// The colour at the edge, for a gradient.
    pub colour_b: [f32; 3],
    /// How much of a spectrum is laid across it.
    pub rainbow: f32,
    /// Brighter near: the choice, its amount and its range.
    pub response: [f32; 3],
    /// The shape's own numbers, see `shade` in [`super::raster`].
    pub s: [f32; 12],
}

impl Instance {
    /// The layout the GPU reads.
    pub fn packed(&self) -> [f32; FLOATS] {
        let mut out = [0.0; FLOATS];
        let head = [
            self.anchor as f32,
            self.at[0],
            self.at[1],
            self.flags as f32,
            self.offset[0],
            self.offset[1],
            self.half[0],
            self.half[1],
            self.angle,
            self.shape as f32,
            self.grow,
            self.reach,
            self.colour[0],
            self.colour[1],
            self.colour[2],
            self.gain,
            self.colour_b[0],
            self.colour_b[1],
            self.colour_b[2],
            self.rainbow,
            self.response[0],
            self.response[1],
            self.response[2],
            0.0,
        ];
        out[..24].copy_from_slice(&head);
        out[24..].copy_from_slice(&self.s);
        out
    }
}

/// How far out a falloff of `exp(-x^p)` is drawn: where it has dropped to a
/// millionth, which no 8-bit picture shows even after a bright grade.
fn reach_of(p: f32) -> f32 {
    13.8_f32.powf(1.0 / p.max(0.1)).min(14.0)
}

/// Half a line's quad across it, for a line `width` wide: out to where its
/// gaussian is a ten-millionth.
fn across(width: f32) -> f32 {
    2.0 * width.max(1.0) + 1.0
}

/// Every quad `design` draws in `frame`, `w` by `h` raster pixels.
pub fn expand(design: &Design, frame: &Frame, w: u32, h: u32) -> Vec<Instance> {
    let mut out = Vec::new();
    for e in design.elements.iter().filter(|e| e.on) {
        element(e, frame, [w as f32, h as f32], &mut out);
        if out.len() >= MAX {
            out.truncate(MAX);
            break;
        }
    }
    out
}

/// A shape's numbers, the rest left at 0.
fn pad(head: &[f32]) -> [f32; 12] {
    let mut s = [0.0; 12];
    for (to, from) in s.iter_mut().zip(head) {
        *to = *from;
    }
    s
}

/// A seeded number in 0..1 for copy `i`.
fn rand(seed: u32, channel: u32, i: usize) -> f32 {
    hash01(seed, channel, i as i32, 0, 0)
}

/// The two colours and the rainbow amount of copy `i`.
fn colours(e: &Element, i: usize, seed: u32) -> ([f32; 3], [f32; 3], f32) {
    let white = [1.0; 3];
    match e.colour.mode {
        ColourMode::Light => (white, white, 0.0),
        ColourMode::Solid => (e.colour_at(0), e.colour_at(0), 0.0),
        ColourMode::Mixed => {
            let n = e.colour.colours.len().max(1);
            let pick = ((rand(seed, 9, i) * n as f32) as usize).min(n - 1);
            (e.colour_at(pick), e.colour_at(pick), 0.0)
        }
        ColourMode::Gradient => (e.colour_at(0), e.colour_at(1), 0.0),
        ColourMode::Rainbow => (white, white, 1.0),
    }
}

/// 0..1, moving smoothly with Evolution: a sine at a whole number of cycles
/// per turn, so a full turn of Evolution comes back to where it started.
fn wave(evolution: f32, seed: u32, channel: u32, i: usize) -> f32 {
    let cycles = 1.0 + (rand(seed, channel, i) * 3.0).floor();
    let phase = rand(seed, channel + 1, i) * TAU;
    0.5 + 0.5 * (evolution * cycles + phase).sin()
}

fn element(e: &Element, frame: &Frame, frame_px: [f32; 2], out: &mut Vec<Instance>) {
    let k = frame.px_scale * if e.on("ignore_size") { 1.0 } else { frame.size };
    let gain = e.num("brightness")
        * if e.on("ignore_brightness") {
            1.0
        } else {
            frame.brightness
        };
    let rotation = (e.num("rotation")
        + if e.on("ignore_rotation") {
            0.0
        } else {
            frame.rotation_deg
        })
    .to_radians();
    let size = e.num("size") * k;
    if gain <= 0.0 || size <= 0.0 {
        return;
    }
    let seed = e.int("seed");
    let evolution = frame.evolution_deg.to_radians();
    let (colour, colour_b, rainbow) = colours(e, 0, seed);
    let base = Instance {
        flags: if e.on("ignore_edge_fade") {
            NO_EDGE_FADE
        } else {
            0
        },
        angle: rotation,
        colour,
        colour_b,
        rainbow,
        gain,
        response: [
            e.num("response"),
            e.num("response_amount"),
            e.num("response_range"),
        ],
        ..Instance::default()
    };
    let fringe = e.num("fringe");
    match e.kind {
        Kind::Glow => {
            let falloff = e.num("falloff");
            // 1 is an exponential tail, 2 a gaussian, 16 nearly a disc.
            let p = if falloff <= 0.5 {
                1.0 + 2.0 * falloff
            } else {
                2.0 * 8.0_f32.powf(2.0 * falloff - 1.0)
            };
            let core_size = e.num("core_size").max(1e-3);
            let extent = reach_of(p).max(3.72 * core_size);
            let stretch = e.num("stretch").max(0.01);
            out.push(Instance {
                shape: GLOW,
                half: [size * extent * stretch, size * extent],
                s: pad(&[extent, p, e.num("core"), core_size]),
                ..base
            });
        }
        Kind::Rays => {
            let n = e.int("count").clamp(1, 2000) as usize;
            let all_round = e.num("spread") >= 360.0;
            let spread = e.num("spread").to_radians();
            let spin = (e.num("spin") * frame.time_s).to_radians();
            let thickness = e.num("thickness") * k;
            let (var, shimmer) = (e.num("length_var"), e.num("shimmer"));
            for i in 0..n {
                // Spread evenly, each nudged within its own share, so they
                // neither clump nor look combed.
                let share = (i as f32 + rand(seed, 0, i)) / n as f32;
                let angle = rotation
                    + spin
                    + if all_round {
                        share * TAU
                    } else {
                        (share - 0.5) * spread
                    };
                let length = size
                    * (1.0 - var * rand(seed, 1, i))
                    * (1.0 - shimmer * wave(evolution, seed, 2, i));
                let (colour, colour_b, rainbow) = colours(e, i, seed);
                let copy = Instance {
                    colour,
                    colour_b,
                    rainbow,
                    ..base
                };
                ray(
                    out,
                    &copy,
                    angle,
                    length,
                    thickness,
                    e.num("softness"),
                    e.num("taper"),
                );
            }
        }
        Kind::Star => {
            let n = e.int("points").max(1) as usize;
            let thickness = e.num("thickness") * k;
            let (twin, twin_length) = (e.num("twin"), e.num("twin_length"));
            let step = TAU / n as f32;
            let star = Instance {
                rainbow: rainbow.max(fringe),
                ..base
            };
            let second = Instance {
                gain: gain * twin,
                ..star
            };
            for i in 0..n {
                let angle = rotation + i as f32 * step;
                ray(out, &star, angle, size, thickness, 1.0, e.num("taper"));
                if twin > 0.0 {
                    let length = size * twin_length;
                    ray(
                        out,
                        &second,
                        angle + step * 0.5,
                        length,
                        thickness,
                        1.0,
                        e.num("taper"),
                    );
                }
            }
        }
        Kind::Streak => {
            let lines = e.int("lines").clamp(1, 64) as usize;
            let spacing = e.num("spacing") * k;
            // Taper 0.5 is a gaussian along it, 0 a flat bar, 1 a sharp point.
            let p = 2.0_f32.powf(2.0 - 2.0 * e.num("taper"));
            let extent = reach_of(p);
            // Thickness is where it has dropped to 1/e, so the full width of
            // the line's profile is twice it.
            let width = 2.0 * e.num("thickness") * k;
            let middle = (lines as f32 - 1.0) * 0.5;
            for j in 0..lines {
                let from_middle = j as f32 - middle;
                out.push(Instance {
                    shape: RAY,
                    flags: base.flags | if e.on("face") { FOLLOWS_AXIS } else { 0 },
                    offset: [0.0, from_middle * spacing],
                    half: [size * extent, across(width)],
                    gain: gain / (1.0 + from_middle.abs()),
                    rainbow: rainbow.max(fringe),
                    s: pad(&[1.0, extent, p, width, 1.0]),
                    ..base
                });
            }
        }
        Kind::Ring | Kind::Halo => {
            let halo = e.kind == Kind::Halo;
            let thickness = e.num("thickness") * k;
            let half = size + across(thickness);
            let feather = if halo { 1.0 } else { e.num("arc_feather") };
            out.push(Instance {
                shape: ANNULUS,
                flags: base.flags | if halo { FOLLOWS_AXIS } else { 0 },
                half: [half, half],
                grow: if halo { e.num("grow") } else { 0.0 },
                rainbow: rainbow.max(fringe),
                s: pad(&[
                    size / half,
                    thickness / half,
                    e.num("softness"),
                    e.num("arc").to_radians() * 0.5,
                    feather,
                ]),
                ..base
            });
        }
        Kind::Ghost => {
            iris(
                out,
                e,
                &base,
                rotation,
                ON_AXIS,
                [e.num("place"), 0.0],
                size,
            );
        }
        Kind::Ghosts => {
            let n = e.int("count").clamp(1, 500) as usize;
            let (from, to) = (e.num("from"), e.num("to"));
            let scatter = e.num("scatter") * k;
            let (size_var, bright_var) = (e.num("size_var"), e.num("bright_var"));
            for i in 0..n {
                // Each in its own share of the way, so they string out
                // rather than bunch.
                let t = from + (to - from) * (i as f32 + rand(seed, 0, i)) / n as f32;
                let s = size * (1.0 - size_var * rand(seed, 1, i));
                iris(out, e, &base, rotation, ON_AXIS, [t, 0.0], s);
                if let Some(last) = out.last_mut() {
                    last.offset[1] = scatter * (2.0 * rand(seed, 2, i) - 1.0);
                    last.gain *= 1.0 - bright_var * rand(seed, 3, i);
                    (last.colour, last.colour_b, last.rainbow) = colours(e, i, seed);
                }
            }
        }
        Kind::Orbs => {
            let n = e.int("count").clamp(1, 2000) as usize;
            let size_var = e.num("size_var");
            let drift = e.num("drift") * k;
            for i in 0..n {
                let at = [
                    rand(seed, 0, i) * frame_px[0],
                    rand(seed, 1, i) * frame_px[1],
                ];
                let s = size * (1.0 - size_var * rand(seed, 2, i));
                iris(out, e, &base, 0.0, ON_FRAME, at, s);
                if let Some(last) = out.last_mut() {
                    let a = wave(evolution, seed, 3, i) * TAU;
                    last.offset = [drift * a.cos(), drift * a.sin()];
                    last.reach = e.num("reach") * frame.px_scale;
                    (last.colour, last.colour_b, last.rainbow) = colours(e, i, seed);
                }
            }
        }
        // Textures come later.
        Kind::Dirt | Kind::Picture => {}
    }
}

/// One ray from the light out at `angle`.
fn ray(
    out: &mut Vec<Instance>,
    base: &Instance,
    angle: f32,
    length: f32,
    thickness: f32,
    softness: f32,
    taper: f32,
) {
    if length <= 0.0 {
        return;
    }
    out.push(Instance {
        shape: RAY,
        angle,
        offset: [length * 0.5, 0.0],
        half: [length * 0.5, across(thickness)],
        s: pad(&[0.0, 0.0, 2.0, thickness, softness, taper]),
        ..*base
    });
}

/// One ghost or orb of radius `size`, from the shape controls they share.
fn iris(
    out: &mut Vec<Instance>,
    e: &Element,
    base: &Instance,
    rotation: f32,
    anchor: u32,
    at: [f32; 2],
    size: f32,
) {
    if size <= 0.0 {
        return;
    }
    let ghost = anchor == ON_AXIS;
    let softness = e.num("softness");
    // Red and blue each pulled a tenth of the radius apart at Fringe 1.
    let shift = if ghost { e.num("fringe") * 0.1 } else { 0.0 };
    let stretch = if ghost {
        e.num("stretch").max(0.01)
    } else {
        1.0
    };
    // Room for the feather, the fringe and a pixel either side.
    let margin = 1.0 + 0.5 * softness + shift + 2.0 / size;
    let sides = if ghost { e.int("sides") } else { 0 };
    let edge_width = if ghost { e.num("edge_width") } else { 0.08 };
    let mut flags = base.flags;
    if ghost {
        flags |= FOLLOWS_AXIS;
        if !e.on("face") {
            flags |= STILL_POLYGON;
        }
    }
    out.push(Instance {
        anchor,
        at,
        flags,
        shape: IRIS,
        angle: 0.0,
        half: [size * margin * stretch, size * margin],
        s: pad(&[
            margin,
            if sides >= 3 { sides as f32 } else { 0.0 },
            rotation,
            if ghost { e.num("roundness") } else { 1.0 },
            e.num("fill"),
            e.num("edge"),
            edge_width,
            softness,
            shift,
        ]),
        ..*base
    });
}
