//! The CPU drawing of a design: the GPU kernel's twin and test oracle, and
//! what the Flare window's preview is drawn with.

use std::f32::consts::{PI, TAU};

use super::design::Design;
use super::instances::{self, Instance};
use super::textures;
use crate::fx::lens_flare::FlareLight;

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
    /// How much bigger than designed it draws what sits on it or along its
    /// axis: a light far off draws a smaller flare. 1 is as designed.
    pub scale: f32,
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
                scale: 1.0,
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

/// A layer's picture an element can use as its texture: scene-linear RGBA,
/// `w` by `h`.
#[derive(Debug, Clone, Copy)]
pub struct Picture<'a> {
    pub rgba: &'a [f32],
    pub w: u32,
    pub h: u32,
}

impl Picture<'_> {
    /// The picture at `x`, `y` in texture texels, stretched so the texture's
    /// square covers it once, wrapping, bilinear between pixel middles. The
    /// WGSL's `layer_at` is the same.
    fn at(&self, x: f32, y: f32) -> [f32; 3] {
        if self.w == 0 || self.h == 0 || self.rgba.len() < (self.w * self.h * 4) as usize {
            return [0.0; 3];
        }
        let n = textures::SIZE as f32;
        let (x, y) = (x / n * self.w as f32 - 0.5, y / n * self.h as f32 - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let px = |dx: i32, dy: i32, k: usize| {
            let tx = (x0 as i32 + dx).rem_euclid(self.w as i32) as usize;
            let ty = (y0 as i32 + dy).rem_euclid(self.h as i32) as usize;
            self.rgba[(ty * self.w as usize + tx) * 4 + k].max(0.0)
        };
        let mut out = [0.0; 3];
        for (k, o) in out.iter_mut().enumerate() {
            let top = px(0, 0, k) + (px(1, 0, k) - px(0, 0, k)) * fx;
            let bottom = px(0, 1, k) + (px(1, 1, k) - px(0, 1, k)) * fx;
            *o = top + (bottom - top) * fy;
        }
        out
    }
}

/// Texture `k` at `x`, `y` in texels, for a sprite where a pixel spans
/// `footprint` texels: a made one, the Rainbow coloured across its bow, or
/// the layer's picture, which is nothing when there is none.
fn texel(k: u32, x: f32, y: f32, footprint: f32, layer: Option<&Picture<'_>>) -> [f32; 3] {
    if k == textures::LAYER {
        return layer.map_or([0.0; 3], |l| l.at(x, y));
    }
    let t = textures::sample(k, x, y, footprint);
    if k == textures::RAINBOW {
        let c = spectrum(textures::rainbow_at(x, y));
        return [t * c[0], t * c[1], t * c[2]];
    }
    [t; 3]
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
    /// The light it was put on, raster pixels, which dirt is lit by.
    pub light: [f32; 2],
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
    // What is spread across the frame stays its size whatever the light's.
    let scale = if i.anchor == instances::ON_FRAME {
        1.0
    } else {
        l.scale
    };
    let grow = (1.0 + i.grow * rho) * scale;
    let (s, c) = angle.sin_cos();
    let [ox, oy] = [i.offset[0] * scale, i.offset[1] * scale];
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
        light: l.pos,
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
/// `sides` is under 3, rounded towards one by `roundness`. `notches` cuts a
/// small dent into each corner, where an iris's blades overlap.
fn polygon(q: [f32; 2], sides: f32, angle: f32, roundness: f32, notches: f32) -> f32 {
    let r = q[0].hypot(q[1]);
    if sides < 3.0 {
        return r;
    }
    let seg = TAU / sides;
    let a = q[1].atan2(q[0]) - angle + seg * 0.5;
    let a = a - seg * (a / seg).floor() - seg * 0.5;
    // 1 on a corner, 0 a fifth of a side away from it.
    let corner = (1.0 - (seg * 0.5 - a.abs()) / (seg * 0.1)).max(0.0);
    let p = r * a.cos() / (seg * 0.5).cos() * (1.0 + 0.25 * notches * corner * corner);
    p + (r - p) * roundness
}

/// The light `i` adds at `uv`, -1..1 across its quad, once placed as `p`.
/// The fragment shader in `fx_sprite_flare.wgsl` is the same, step for step.
pub fn shade(i: &Instance, p: &Placed, uv: [f32; 2], layer: Option<&Picture<'_>>) -> [f32; 3] {
    let s = &i.s;
    let len = uv[0].hypot(uv[1]);
    let (v, r) = match i.shape {
        instances::GLOW => {
            let d = len * s[0];
            let v = ((-d.powf(s[1])).exp() + s[2] * (-(d / s[3]).powi(2)).exp())
                * (1.0 - ramp(0.9, 1.0, len));
            // Fringe: white in the middle, a spectrum round the edge, and
            // white again in the faint tail.
            let rim = s[4] * ramp(0.3, 0.9, d) * (1.0 - ramp(1.2, 2.0, d));
            let c = spectrum(d - 0.4);
            let tint = |k: usize| v * (1.0 + (c[k] - 1.0) * rim);
            ([tint(0), tint(1), tint(2)], d.min(1.0))
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
            // A streak narrows out to twice its length; a ray along all of it.
            let narrow = if two_way {
                (a * s[1] * 0.5).min(1.0)
            } else {
                a
            };
            let v = along * band(uv[1] * p.half[1], s[3] * (1.0 - s[5] * narrow), s[4]);
            // Fringe: white at the light, splitting into a spectrum outwards.
            let split = s[6] * ramp(0.0, 0.6, a);
            let c = spectrum(a);
            let tint = |k: usize| v * (1.0 + (c[k] - 1.0) * split);
            ([tint(0), tint(1), tint(2)], a)
        }
        instances::ANNULUS => {
            let spread = s[5].max(1.0);
            let x = (len - s[0]) * p.half[0] / spread;
            let mut v = band(x, s[1] * p.half[0], s[2]) / spread.sqrt();
            if s[3] < PI {
                let aa = 1.0 / (len * p.half[0]).max(1.0);
                let phi = uv[1].atan2(uv[0]).abs();
                v *= 1.0 - ramp(s[3] * (1.0 - s[4]) - aa, s[3] + aa, phi);
            }
            ([v; 3], 0.5 + 0.5 * (len - s[0]) / (s[1] * spread).max(1e-6))
        }
        instances::IRIS => {
            let m = s[0];
            let aa = m / p.half[0].min(p.half[1]).max(1e-3);
            let feather = s[7].max(aa);
            let rim_width = s[6].max(aa);
            let angle = s[2] + p.turn;
            let at = |shift: f32| {
                let d = polygon([uv[0] * m - shift, uv[1] * m], s[1], angle, s[3], s[10]);
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
            // A texture marks the glass rather than blacking it out, so a
            // ghost keeps some of its light where the texture is dark.
            let marked = if s[9] >= 0.5 {
                let n = textures::SIZE as f32;
                let t = texel(
                    s[9].round() as u32,
                    (uv[0] * 0.5 + 0.5) * n,
                    (uv[1] * 0.5 + 0.5) * n,
                    footprint(p),
                    layer,
                );
                t.map(|t| 0.3 + 0.7 * t)
            } else {
                [1.0; 3]
            };
            (
                [red * marked[0], g * marked[1], blue * marked[2]],
                d.min(1.0),
            )
        }
        _ => {
            let k = s[0].round() as u32;
            if s[1] > 0.5 {
                // Dirt: the texture fixed to the frame, lit only near the
                // light, as dirt on a lens is.
                let px = [
                    p.centre[0] + uv[0] * p.half[0],
                    p.centre[1] + uv[1] * p.half[1],
                ];
                let rel = [px[0] - p.centre[0], px[1] - p.centre[1]];
                let d = (px[0] - p.light[0]).hypot(px[1] - p.light[1]) / s[6].max(1e-3);
                let lit = (1.0 - d * d).max(0.0);
                let (sn, cs) = s[2].sin_cos();
                let tx = (rel[0] * cs + rel[1] * sn) * s[3] + s[4];
                let ty = (rel[1] * cs - rel[0] * sn) * s[3] + s[5];
                let t = texel(k, tx, ty, s[3], layer);
                (t.map(|t| t * lit * lit), d.min(1.0))
            } else {
                // A picture, fading out before the square's edge.
                let n = textures::SIZE as f32;
                let (x, y) = ((uv[0] * 0.5 + 0.5) * n, (uv[1] * 0.5 + 0.5) * n);
                let t = texel(k, x, y, footprint(p), layer);
                (t.map(|t| t * (1.0 - ramp(0.8, 1.0, len))), len.min(1.0))
            }
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

/// Texels a pixel spans on a sprite whose texture's square fills its quad.
fn footprint(p: &Placed) -> f32 {
    textures::SIZE as f32 * 0.5 / p.half[0].min(p.half[1]).max(1e-3)
}

/// Add the flare `design` draws in `frame` to `rgba`, `w` by `h` scene-linear
/// RGBA. Only RGB is added to, alpha is left alone. An element whose texture
/// is the layer draws nothing there.
pub fn draw(design: &Design, frame: &Frame, w: u32, h: u32, rgba: &mut [f32]) {
    draw_with(design, frame, w, h, rgba, None);
}

/// [`draw`] with the Texture layer's picture.
///
/// Each quad is drawn over every pixel whose middle falls inside it, which
/// is what the GPU's rasteriser does.
pub fn draw_with(
    design: &Design,
    frame: &Frame,
    w: u32,
    h: u32,
    rgba: &mut [f32],
    layer: Option<Picture<'_>>,
) {
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
                splat_row(i, p, w, y as u32, row, layer.as_ref());
            }
        });
}

fn splat_row(
    i: &Instance,
    p: &Placed,
    w: u32,
    y: u32,
    row: &mut [f32],
    layer: Option<&Picture<'_>>,
) {
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
        let add = shade(i, p, uv, layer);
        let at = (x * 4) as usize;
        for k in 0..3 {
            row[at + k] += add[k];
        }
    }
}

/// How many points of the Hiding disc are read.
pub const HIDE_TAPS: u32 = 64;

/// How much of the disc `radius` round `at` the picture covers, 0..1: its
/// alpha, or its luma when `luma`, read at [`HIDE_TAPS`] points spread
/// evenly over the disc. Off the picture covers nothing. The GPU's
/// `sf_lights` reads the same points.
pub fn covered(picture: &[f32], w: u32, h: u32, at: [f32; 2], radius: f32, luma: bool) -> f32 {
    disc(picture, w, h, at, radius, |px| {
        if luma {
            let l = crate::fx::cpu::LUMA;
            l[0] * px[0] + l[1] * px[1] + l[2] * px[2]
        } else {
            px[3]
        }
    })
}

/// How much of the disc `radius` round `at` is nearer than `metres` in a
/// depth (metres in red), 0..1, read as [`covered`] reads. The GPU's
/// `sf_lights` is the same.
pub fn nearer(depth: &[f32], w: u32, h: u32, at: [f32; 2], radius: f32, metres: f32) -> f32 {
    disc(depth, w, h, at, radius, |px| {
        if px[0].max(0.01) < metres * 0.98 {
            1.0
        } else {
            0.0
        }
    })
}

fn disc(
    picture: &[f32],
    w: u32,
    h: u32,
    at: [f32; 2],
    radius: f32,
    read: impl Fn(&[f32]) -> f32,
) -> f32 {
    if picture.len() < (w as usize) * (h as usize) * 4 {
        return 0.0;
    }
    let mut sum = 0.0;
    for k in 0..HIDE_TAPS {
        let r = radius.max(0.0) * ((k as f32 + 0.5) / HIDE_TAPS as f32).sqrt();
        // The golden angle, so no two taps line up.
        let a = k as f32 * 2.399_963;
        let (x, y) = ((at[0] + r * a.cos()).floor(), (at[1] + r * a.sin()).floor());
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
            continue;
        }
        let i = (y as usize * w as usize + x as usize) * 4;
        sum += read(&picture[i..i + 4]).clamp(0.0, 1.0);
    }
    sum / HIDE_TAPS as f32
}

/// The lights Lens flare's detection found in a `w` by `h` picture, as
/// flare lights: each takes the hue of its spot when `use_colour`, times
/// `colour`, and is as bright as the spot is up to `gain`. A spot that only
/// just made the cut is dim, so lights fade in and out rather than pop.
pub fn spots(
    found: &[FlareLight],
    w: u32,
    h: u32,
    use_colour: bool,
    colour: [f32; 3],
    gain: f32,
) -> Vec<Light> {
    found
        .iter()
        .filter_map(|f| {
            let m = f.rgb[0].max(f.rgb[1]).max(f.rgb[2]);
            if m <= 0.0 {
                return None;
            }
            let hue = |k: usize| if use_colour { f.rgb[k] / m } else { 1.0 };
            Some(Light {
                pos: [f.pos[0] * w as f32, f.pos[1] * h as f32],
                colour: [hue(0) * colour[0], hue(1) * colour[1], hue(2) * colour[2]],
                gain: gain * m.min(1.0),
                scale: 1.0,
            })
        })
        .collect()
}
