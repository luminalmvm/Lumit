//! The CPU drawing of a design: the GPU kernel's twin and test oracle, and
//! what the Flare window's preview is drawn with.

use std::f32::consts::{PI, TAU};

use super::design::Design;
use super::instances::{self, Instance};

/// One light the flare is drawn from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    /// Raster pixels. May be off the frame.
    pub pos: [f32; 2],
    /// Scene-linear, already times the effect's Colour row.
    pub colour: [f32; 3],
    /// Gain on everything this light draws: its own intensity, how much of it
    /// is hidden, its share of a motion-blurred frame.
    pub gain: f32,
}

/// Everything about one frame the design is drawn into.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub lights: Vec<Light>,
    /// The optical centre, raster pixels. The axis runs from each light
    /// through it.
    pub centre: [f32; 2],
    /// Raster pixels per comp pixel, which every px@comp size is scaled by.
    pub px_scale: f32,
    /// The effect's Brightness, times flicker.
    pub brightness: f32,
    /// The effect's Size, 1 is the design as drawn.
    pub size: f32,
    pub rotation_deg: f32,
    /// The effect's Evolution, degrees: what moves shimmer and drift.
    pub evolution_deg: f32,
    /// Layer time, seconds, for Spin.
    pub time_s: f32,
    /// How far past the frame's edge a light fades out, comp pixels. 0 is
    /// no fade.
    pub edge_fade: f32,
}

impl Frame {
    /// One white light at `light` with the centre in the middle of a `w` by
    /// `h` frame, everything else neutral: the window's preview.
    pub fn preview(w: u32, h: u32, light: [f32; 2], px_scale: f32) -> Self {
        Self {
            lights: vec![Light {
                pos: light,
                colour: [1.0, 1.0, 1.0],
                gain: 1.0,
            }],
            centre: [w as f32 * 0.5, h as f32 * 0.5],
            px_scale,
            brightness: 1.0,
            size: 1.0,
            rotation_deg: 0.0,
            evolution_deg: 0.0,
            time_s: 0.0,
            edge_fade: 0.0,
        }
    }
}

/// An instance put on one light: where its quad is on screen and how bright.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub centre: [f32; 2],
    pub angle: f32,
    pub half: [f32; 2],
    pub colour: [f32; 3],
    pub colour_b: [f32; 3],
    pub gain: f32,
    /// What the polygon in an iris is turned by on top of its own rotation.
    pub turn: f32,
}

/// `smoothstep` that also takes `a == b`, as a step at `a`.
fn ramp(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Put `i` on light `l` in a `w` by `h` frame, or `None` when it would add
/// nothing. The vertex shader in `fx_sprite_flare.wgsl` does the same, step
/// for step.
pub fn place(i: &Instance, l: &Light, frame: &Frame, w: u32, h: u32) -> Option<Placed> {
    let (w, h) = (w as f32, h as f32);
    let axis = [frame.centre[0] - l.pos[0], frame.centre[1] - l.pos[1]];
    let axis_angle = axis[1].atan2(axis[0]);
    // How far the light is from the centre: 1 at the frame's corner.
    let rho = axis[0].hypot(axis[1]) / (0.5 * w.hypot(h)).max(1.0);
    let at = match i.anchor {
        instances::ON_LIGHT => l.pos,
        instances::ON_AXIS => [l.pos[0] + axis[0] * i.at[0], l.pos[1] + axis[1] * i.at[0]],
        _ => i.at,
    };
    let mut gain = i.gain * l.gain;
    let [mode, amount, range] = i.response;
    let near = if mode > 1.5 {
        ramp(1.0 - range, 1.0, rho)
    } else if mode > 0.5 {
        1.0 - ramp(0.0, range, rho)
    } else {
        1.0
    };
    gain *= 1.0 - amount * (1.0 - near);
    let fade = frame.edge_fade * frame.px_scale;
    if i.flags & instances::NO_EDGE_FADE == 0 && fade > 0.0 {
        let out_x = (-l.pos[0]).max(l.pos[0] - w).max(0.0);
        let out_y = (-l.pos[1]).max(l.pos[1] - h).max(0.0);
        gain *= 1.0 - ramp(0.0, fade, out_x.hypot(out_y));
    }
    if i.reach > 0.0 {
        let d = (at[0] - l.pos[0]).hypot(at[1] - l.pos[1]) / i.reach;
        let lit = (1.0 - d * d).max(0.0);
        gain *= lit * lit;
    }
    if gain <= 0.0 {
        return None;
    }
    let follows = i.flags & instances::FOLLOWS_AXIS != 0;
    let angle = i.angle + if follows { axis_angle } else { 0.0 };
    let grow = 1.0 + i.grow * rho;
    let (s, c) = angle.sin_cos();
    let [ox, oy] = i.offset;
    let tint = |c: [f32; 3]| [c[0] * l.colour[0], c[1] * l.colour[1], c[2] * l.colour[2]];
    Some(Placed {
        centre: [at[0] + ox * c - oy * s, at[1] + ox * s + oy * c],
        angle,
        half: [i.half[0] * grow, i.half[1] * grow],
        colour: tint(i.colour),
        colour_b: tint(i.colour_b),
        gain,
        turn: if i.flags & instances::STILL_POLYGON != 0 {
            -axis_angle
        } else {
            0.0
        },
    })
}

/// A rainbow across 0..1, blue inside and red outside.
fn spectrum(x: f32) -> [f32; 3] {
    let x = x.clamp(0.0, 1.0);
    let band = |at: f32| (1.5 - (x * 4.0 - at).abs()).clamp(0.0, 1.0);
    [band(3.2), band(2.0), band(0.8)]
}

/// A line's profile across it, `x` pixels from its middle and `width` wide:
/// a hard edge one pixel soft at softness 0, a gaussian at 1. Thinner than a
/// pixel it is drawn a pixel wide and dimmer, so fine lines stay smooth.
fn band(x: f32, width: f32, softness: f32) -> f32 {
    let wide = width.max(1.0);
    let hard = (wide * 0.5 - x.abs() + 0.5).clamp(0.0, 1.0);
    let soft = (-(2.0 * x / wide).powi(2)).exp();
    (hard + (soft - hard) * softness) * (width.max(0.0) / wide)
}

/// How far `q` is out to a polygon's edge, 1 on the edge: a circle when
/// `sides` is under 3, rounded towards one by `roundness`.
fn polygon(q: [f32; 2], sides: f32, angle: f32, roundness: f32) -> f32 {
    let r = q[0].hypot(q[1]);
    if sides < 3.0 {
        return r;
    }
    let seg = TAU / sides;
    let a = q[1].atan2(q[0]) - angle + seg * 0.5;
    let a = a - seg * (a / seg).floor() - seg * 0.5;
    let p = r * a.cos() / (seg * 0.5).cos();
    p + (r - p) * roundness
}

/// The light `i` adds at `uv`, -1..1 across its quad, once placed as `p`.
/// The fragment shader in `fx_sprite_flare.wgsl` is the same, step for step.
pub fn shade(i: &Instance, p: &Placed, uv: [f32; 2]) -> [f32; 3] {
    let s = &i.s;
    let len = uv[0].hypot(uv[1]);
    let (v, r) = match i.shape {
        instances::GLOW => {
            let d = len * s[0];
            let v = (-d.powf(s[1])).exp() + s[2] * (-(d / s[3]).powi(2)).exp();
            ([v * (1.0 - ramp(0.9, 1.0, len)); 3], d.min(1.0))
        }
        instances::RAY => {
            let two_way = s[0] > 0.5;
            let a = if two_way {
                uv[0].abs()
            } else {
                (uv[0] + 1.0) * 0.5
            };
            let along = if two_way {
                (-(a * s[1]).powf(s[2])).exp() * (1.0 - ramp(0.9, 1.0, a))
            } else {
                (1.0 - a).max(0.0).powf(s[2])
            };
            let v = along * band(uv[1] * p.half[1], s[3] * (1.0 - s[5] * a), s[4]);
            ([v; 3], a)
        }
        instances::ANNULUS => {
            let x = (len - s[0]) * p.half[0];
            let mut v = band(x, s[1] * p.half[0], s[2]);
            if s[3] < PI {
                let aa = 1.0 / (len * p.half[0]).max(1.0);
                let phi = uv[1].atan2(uv[0]).abs();
                v *= 1.0 - ramp(s[3] * (1.0 - s[4]) - aa, s[3] + aa, phi);
            }
            ([v; 3], 0.5 + 0.5 * (len - s[0]) / s[1].max(1e-6))
        }
        _ => {
            let m = s[0];
            let aa = m / p.half[0].min(p.half[1]).max(1e-3);
            let feather = s[7].max(aa);
            let rim_width = s[6].max(aa);
            let angle = s[2] + p.turn;
            let at = |shift: f32| {
                let d = polygon([uv[0] * m - shift, uv[1] * m], s[1], angle, s[3]);
                let e = 1.0 - d;
                let cover = ramp(-feather * 0.5, feather * 0.5, e);
                let rim = (-((e - rim_width) / rim_width).powi(2)).exp() * cover;
                (s[4] * cover + s[5] * rim, d)
            };
            let (g, d) = at(0.0);
            let (red, blue) = if s[8] > 0.0 {
                (at(s[8]).0, at(-s[8]).0)
            } else {
                (g, g)
            };
            ([red, g, blue], d.min(1.0))
        }
    };
    let rainbow = spectrum(r);
    let mut c = [0.0; 3];
    for k in 0..3 {
        let base = p.colour[k] + (p.colour_b[k] - p.colour[k]) * r.clamp(0.0, 1.0);
        let tint = 1.0 + (rainbow[k] - 1.0) * i.rainbow;
        c[k] = p.gain * base * tint * v[k];
    }
    c
}

/// Add the flare `design` draws in `frame` to `rgba`, `w` by `h` scene-linear
/// RGBA. Only RGB is added to, alpha is left alone.
///
/// Each quad is drawn over every pixel whose middle falls inside it, which
/// is what the GPU's rasteriser does.
pub fn draw(design: &Design, frame: &Frame, w: u32, h: u32, rgba: &mut [f32]) {
    use rayon::prelude::*;
    if frame.lights.is_empty() || w == 0 || rgba.len() < (w as usize) * (h as usize) * 4 {
        return;
    }
    let instances = instances::expand(design, frame, w, h);
    let placed: Vec<(&Instance, Placed)> = instances
        .iter()
        .flat_map(|i| {
            frame
                .lights
                .iter()
                .filter_map(move |l| place(i, l, frame, w, h).map(|p| (i, p)))
        })
        .collect();
    // A row at a time across threads, each adding the quads in the same
    // order, so every pixel sums exactly as it would on one thread.
    rgba.par_chunks_mut(w as usize * 4)
        .take(h as usize)
        .enumerate()
        .for_each(|(y, row)| {
            for (i, p) in &placed {
                splat_row(i, p, w, y as u32, row);
            }
        });
}

fn splat_row(i: &Instance, p: &Placed, w: u32, y: u32, row: &mut [f32]) {
    let (s, c) = p.angle.sin_cos();
    let ex = c.abs() * p.half[0] + s.abs() * p.half[1];
    let ey = s.abs() * p.half[0] + c.abs() * p.half[1];
    let yc = y as f32 + 0.5;
    if yc < p.centre[1] - ey - 0.5 || yc > p.centre[1] + ey + 0.5 {
        return;
    }
    let first = (p.centre[0] - ex - 0.5).floor().max(0.0) as u32;
    let last = ((p.centre[0] + ex + 0.5).ceil().max(0.0) as u32).min(w);
    let dy = yc - p.centre[1];
    for x in first..last {
        let dx = x as f32 + 0.5 - p.centre[0];
        let uv = [
            (dx * c + dy * s) / p.half[0],
            (-dx * s + dy * c) / p.half[1],
        ];
        if uv[0].abs() > 1.0 || uv[1].abs() > 1.0 {
            continue;
        }
        let add = shade(i, p, uv);
        let at = (x * 4) as usize;
        for k in 0..3 {
            row[at + k] += add[k];
        }
    }
}
