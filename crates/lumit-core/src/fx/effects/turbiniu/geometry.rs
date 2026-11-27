use std::f32::consts::TAU;

use crate::fx::{
    cpu::{curve_at, curve_table, CURVE_TABLE},
    params::CurvePoints,
};
use crate::mask::MaskPolyline;

use super::noise::{random as hashed, Perlin};
use super::Turbiniu;

pub type TurbiniuVertex = [f32; 6];

const MAX_VERTICES: usize = 1_500_000;
const MARKER_SALT: u64 = 0x4d41_524b_0000_0000;
type Vec2 = [f32; 2];
type Color = [f32; 4];

pub(super) fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn add(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn scale(a: Vec2, k: f32) -> Vec2 {
    [a[0] * k, a[1] * k]
}

fn length(a: Vec2) -> f32 {
    a[0].hypot(a[1])
}

fn unit(a: Vec2) -> Vec2 {
    let n = length(a);
    if n > 1e-5 {
        scale(a, n.recip())
    } else {
        [1.0, 0.0]
    }
}

fn normal(a: Vec2) -> Vec2 {
    [-a[1], a[0]]
}

fn blend(a: Color, b: Color, t: f32) -> Color {
    std::array::from_fn(|i| mix(a[i], b[i], t))
}

fn wrap(t: f32, looping: bool) -> f32 {
    if looping {
        t.rem_euclid(1.0)
    } else {
        t.clamp(0.0, 1.0)
    }
}

pub(super) struct Ramp {
    table: [f32; CURVE_TABLE],
    offset: f32,
    scale: f32,
    looping: bool,
}

impl Ramp {
    pub(super) fn new(points: CurvePoints, offset: f32, size: f32, looping: bool) -> Self {
        Self {
            table: curve_table(&points),
            offset: offset / 100.0,
            scale: size / 100.0,
            looping,
        }
    }

    fn at(&self, t: f32) -> f32 {
        let from_end = if self.scale < 0.0 { 1.0 } else { 0.0 };
        curve_at(
            wrap(t * self.scale - self.offset + from_end, self.looping),
            &self.table,
        )
    }
}

pub(super) struct Gradient {
    pub(super) ramp: Ramp,
    pub(super) colors: [Color; 5],
    pub(super) count: usize,
}

impl Gradient {
    fn at(&self, t: f32) -> Color {
        let x = self.ramp.at(t) * (self.count - 1) as f32;
        let i = x as usize;
        blend(
            self.colors[i],
            self.colors[(i + 1).min(self.count - 1)],
            x.fract(),
        )
    }
}

pub(super) struct Noise {
    pub(super) coordinates: u32,
    pub(super) amplitude: f32,
    pub(super) frequency: f32,
    pub(super) axes: Vec2,
    pub(super) offset: Vec2,
    pub(super) evolution: f32,
    pub(super) cycle: f32,
    pub(super) line_offset: f32,
    pub(super) line_steps_phase: bool,
    pub(super) field: Perlin,
    pub(super) ramp: Ramp,
}

#[derive(Clone, Copy)]
struct Sample {
    point: Vec2,
    size: Vec2,
    line: usize,
    line_count: usize,
    along: f32,
    segments: usize,
}

impl Noise {
    fn at(&self, at: Sample) -> f32 {
        if self.amplitude == 0.0 {
            return 0.0;
        }
        let step = self.line_offset * at.line as f32;
        let (across, phase) = if self.line_steps_phase {
            (0.0, step)
        } else {
            (step, 0.0)
        };
        let uv = match self.coordinates {
            0 => [at.point[0] / at.size[0], at.point[1] / at.size[0]],
            2 => [at.along * at.segments as f32 * 0.01; 2],
            _ => [at.along, at.line as f32 / at.line_count as f32],
        };
        let [x, y] = std::array::from_fn(|i| {
            let shift = if i == 0 { across } else { 0.0 };
            (uv[i] * self.frequency * self.axes[i] + self.offset[i] + shift).clamp(-1e6, 1e6)
        });
        self.amplitude
            * self.ramp.at(at.along)
            * self
                .field
                .sample(x, y, (self.evolution + phase).clamp(-1e6, 1e6), self.cycle)
    }
}

pub(super) struct Sine {
    pub(super) amplitude: f32,
    pub(super) frequency: f32,
    pub(super) offset: f32,
}

impl Sine {
    fn at(&self, t: f32) -> f32 {
        self.amplitude * ((t * self.frequency + self.offset) * TAU).sin()
    }
}

pub(super) struct Guide {
    pub(super) thickness: f32,
    pub(super) thickness_ramp: Ramp,
    pub(super) feather: f32,
    pub(super) opacity: f32,
    pub(super) opacity_ramp: Ramp,
    pub(super) gradient: Gradient,
    pub(super) gradient_random: f32,
    pub(super) sine_operation: u32,
    pub(super) sines: [Sine; 2],
}

impl Guide {
    fn position(
        &self,
        path: &MaskPolyline,
        density: &Ramp,
        t: f32,
        invert: bool,
        raster_scale: f32,
    ) -> (Vec2, Vec2) {
        let dist = density.at(if invert { 1.0 - t } else { t }) * path.length();
        let mut tangent = path.tangent_at(dist);
        let a = self.sines[0].at(t);
        let b = self.sines[1].at(t);
        let displacement = if self.sine_operation == 1 {
            let (da, db) = (self.sines[0].amplitude, self.sines[1].amplitude);
            if da == 0.0 || db == 0.0 {
                0.0
            } else {
                (da * db).abs().sqrt() * (a / da) * (b / db) * (da * db).signum()
            }
        } else {
            a + b
        };
        let xy = sub(
            scale(path.point_at(dist), raster_scale),
            scale(normal(tangent), displacement),
        );
        if invert {
            tangent = scale(tangent, -1.0);
        }
        (xy, tangent)
    }

    fn appearance(&self, t: f32, random: f32) -> (f32, Color, f32) {
        (
            self.thickness.max(0.0) * self.thickness_ramp.at(t) * 0.5,
            self.gradient
                .at(mix(t, random, self.gradient_random / 100.0)),
            self.opacity * self.opacity_ramp.at(t),
        )
    }
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn grain(field: &Perlin, x: f32, lap: f32, line: usize, salt: f32) -> f32 {
    let y = line as f32 * 3.71 + salt;
    let value = if lap > 0.0 {
        let radius = lap / TAU;
        let angle = x / lap * TAU;
        field.sample(
            (radius * angle.cos()).clamp(-1e6, 1e6),
            y,
            (radius * angle.sin() + salt * 0.5).clamp(-1e6, 1e6),
            0.0,
        )
    } else {
        field.sample(x.clamp(-1e6, 1e6), y, salt * 0.5, 0.0)
    };
    (value * 2.0 + 0.5).clamp(0.0, 1.0)
}

fn centred(s: Station, n: Vec2) -> Station {
    Station {
        xy: add(s.xy, scale(n, s.shift)),
        shift: 0.0,
        ..s
    }
}

#[derive(Clone, Copy)]
struct Station {
    xy: Vec2,
    shift: f32,
    half: f32,
    feather: f32,
    color: Color,
    opacity: f32,
}

struct Mesh {
    vertices: Vec<TurbiniuVertex>,
}

fn vertex(p: Vec2, mut c: Color, opacity: f32) -> TurbiniuVertex {
    c[3] = (c[3] * opacity).clamp(0.0, 1.0);
    [p[0], p[1], c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
}

fn row(s: Station, n: Vec2) -> [TurbiniuVertex; 4] {
    let band = s.feather.max(0.5);
    let inner = (s.half - band * 0.5).max(0.0);
    let outer = s.half + band * 0.5;
    let coverage = s.opacity * (s.half * 2.0 / band).clamp(0.0, 1.0);
    let centre = add(s.xy, scale(n, s.shift));
    [
        vertex(add(centre, scale(n, -outer)), s.color, 0.0),
        vertex(add(centre, scale(n, -inner)), s.color, coverage),
        vertex(add(centre, scale(n, inner)), s.color, coverage),
        vertex(add(centre, scale(n, outer)), s.color, 0.0),
    ]
}

impl Mesh {
    fn triangles(&mut self, vertices: &[TurbiniuVertex]) {
        if self.vertices.len() + vertices.len() <= MAX_VERTICES
            && vertices.iter().flatten().all(|n| n.is_finite())
        {
            self.vertices.extend_from_slice(vertices);
        }
    }

    fn quad(&mut self, a: TurbiniuVertex, b: TurbiniuVertex, c: TurbiniuVertex, d: TurbiniuVertex) {
        self.triangles(&[a, b, c, b, d, c]);
    }

    fn strip(&mut self, stations: &[Station], before_start: Option<Vec2>, after_end: Option<Vec2>) {
        let Some(first) = stations.first() else {
            return;
        };
        let last = stations.len() - 1;
        let before = before_start.unwrap_or(first.xy);
        let mut previous = row(*first, normal(unit(sub(stations[1].xy, before))));
        for i in 1..=last {
            let before = stations[i - 1].xy;
            let after = stations
                .get(i + 1)
                .map_or(after_end.unwrap_or(stations[last].xy), |station| station.xy);
            let current = row(stations[i], normal(unit(sub(after, before))));
            for band in 0..3 {
                self.quad(
                    previous[band],
                    previous[band + 1],
                    current[band],
                    current[band + 1],
                );
            }
            previous = current;
        }
    }

    fn disk(&mut self, s: Station, segments: usize, angle: f32, sweep: f32) {
        let feather = s.feather.max(0.5);
        let outer = s.half + feather * 0.5;
        let inner = (s.half - feather * 0.5).max(0.0);
        let alpha = s.opacity * (s.half * 2.0 / feather).clamp(0.0, 1.0);
        let centre = vertex(s.xy, s.color, alpha);
        let pair = |i| {
            let t = angle + sweep * i as f32 / segments as f32;
            let direction = [t.cos(), t.sin()];
            [
                vertex(add(s.xy, scale(direction, inner)), s.color, alpha),
                vertex(add(s.xy, scale(direction, outer)), s.color, 0.0),
            ]
        };
        let mut previous = pair(0);
        for i in 1..=segments {
            let next = pair(i);
            self.triangles(&[centre, previous[0], next[0]]);
            self.quad(previous[0], previous[1], next[0], next[1]);
            previous = next;
        }
    }

    fn cap(&mut self, s: Station, outward: Vec2, kind: u32, size: f32, angle: f32) {
        match kind {
            1 | 2 if size > 0.0 => {
                let wing = angle.to_radians();
                let tip = add(s.xy, scale(outward, size * wing.cos()));
                let arms = [-1.0, 1.0].map(|sign| {
                    add(
                        tip,
                        add(
                            scale(outward, -size * wing.cos()),
                            scale(normal(outward), sign * size * wing.sin()),
                        ),
                    )
                });
                if kind == 1 {
                    let alpha = s.opacity * (s.half * 2.0 / s.feather.max(0.5)).clamp(0.0, 1.0);
                    if alpha <= 0.0 {
                        return;
                    }
                    self.triangles(&[
                        vertex(tip, s.color, alpha),
                        vertex(arms[0], s.color, alpha),
                        vertex(arms[1], s.color, alpha),
                    ]);
                } else {
                    self.strip(
                        &[
                            Station {
                                xy: arms[0],
                                shift: 0.0,
                                ..s
                            },
                            Station {
                                xy: tip,
                                shift: 0.0,
                                ..s
                            },
                            Station {
                                xy: arms[1],
                                shift: 0.0,
                                ..s
                            },
                        ],
                        None,
                        None,
                    );
                }
            }
            3 => self.disk(s, 12, outward[1].atan2(outward[0]) - TAU * 0.25, TAU * 0.5),
            _ => {}
        }
    }
}

impl Turbiniu {
    pub fn geometry(
        self,
        first: &MaskPolyline,
        second: &MaskPolyline,
        raster_scale: f32,
        dimensions: [u32; 2],
    ) -> Vec<TurbiniuVertex> {
        if [first.length(), second.length(), raster_scale]
            .iter()
            .any(|x| !x.is_finite() || *x <= 0.0)
        {
            return Vec::new();
        }
        let look = self.look();
        let second_look = if self.clone_mask_1 {
            &look.guides[0]
        } else {
            &look.guides[1]
        };
        let count = self.iterations.clamp(1, 10_000) as usize;
        let closed = first.closed && second.closed;
        let points = self.primitives == 1;
        let mut sides = self.point_segments.clamp(3, 128) as usize;
        let requested_steps = self.curve_segments.clamp(1, 10_000) as usize;
        let begin = self.stroke_begin / 100.0;
        let end = if self.stroke_loop {
            (self.stroke_end / 100.0).min(begin + 1.0)
        } else {
            self.stroke_end / 100.0
        };
        if end <= begin || (points && self.visible_points <= 0) {
            return Vec::new();
        }
        let seed = self.random_seed.clamp(0, 32768) as usize;
        let random = |line, index| hashed(seed, line, index);
        let travel = end - begin;
        let full_turn = closed && self.stroke_loop && (travel - 1.0).abs() <= 1e-6;
        let mut plans: Vec<_> = (0..count)
            .filter_map(|line| {
                let shift =
                    (self.stroke_offset + (random(line, 7) - 0.5) * self.offset_random) / 100.0;
                let spans = if full_turn {
                    vec![(0.0, 1.0)]
                } else {
                    stroke_spans(begin + shift, end + shift, self.stroke_loop)
                };
                (!spans.is_empty()).then(|| StrokePlan {
                    line,
                    shift,
                    closed: closed && spans.iter().all(|&span| span == (0.0, 1.0)),
                    wraps: closed && spans.len() > 1,
                    spans,
                })
            })
            .collect();
        let cap_cost = |kind| match kind {
            1 => 3,
            2 => 36,
            3 => 108,
            _ => 0,
        };
        let estimate = |plan: &StrokePlan, steps, sides| {
            plan.spans
                .iter()
                .enumerate()
                .map(|(i, &(start, end))| {
                    let pieces = plan.pieces(start, end, steps);
                    if points {
                        (((end - start) * steps as f32).ceil() as usize + 1) * 9 * sides
                    } else {
                        18 * pieces
                            + if plan.leading(i) {
                                cap_cost(self.cap_begin)
                            } else {
                                0
                            }
                            + if plan.trailing(i) {
                                cap_cost(self.cap_end)
                            } else {
                                0
                            }
                    }
                })
                .sum::<usize>()
        };
        let total = |plans: &[StrokePlan], steps, sides| {
            plans
                .iter()
                .map(|p| estimate(p, steps, sides))
                .fold(0, usize::saturating_add)
        };
        if points && total(&plans, 1, sides) > MAX_VERTICES {
            let unit_cost = total(&plans, 1, 1).max(1);
            sides = sides.min((MAX_VERTICES / unit_cost).max(3));
        }
        if total(&plans, 1, sides) > MAX_VERTICES {
            let max_cost = plans
                .iter()
                .map(|p| estimate(p, 1, sides))
                .max()
                .unwrap_or(1);
            let keep = (MAX_VERTICES / max_cost).max(2);
            let last = plans.len() - 1;
            let mut index = 0;
            plans.retain(|_| {
                let retain =
                    index == 0 || index * (keep - 1) / last != (index - 1) * (keep - 1) / last;
                index += 1;
                retain
            });
        }
        let (mut low, mut high) = (1, requested_steps);
        while low < high {
            let mid = (low + high).div_ceil(2);
            if total(&plans, mid, sides) <= MAX_VERTICES {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        let steps = low;
        let mut mesh = Mesh {
            vertices: Vec::with_capacity(total(&plans, steps, sides)),
        };
        let mut stations = Vec::with_capacity(steps + 1);
        let size = dimensions.map(|x| x.max(1) as f32);
        let path_px = ((first.length() + second.length()) * 0.5 * raster_scale).max(1.0);
        for plan in plans {
            let line = plan.line;
            let random = |i| random(line, i);
            let across = if count == 1 {
                0.0
            } else {
                line as f32 / (count - 1) as f32
            };
            let position =
                look.distribution.at(across) + (random(9) - 0.5) * self.distribution_random / 100.0;
            let between = position.clamp(0.0, 1.0);
            let shade = mix(position, random(1), self.gradient_random / 100.0).clamp(0.0, 1.0);
            let width_gain = mix(1.0, random(4) * 2.0, self.thickness_random / 100.0);
            let opacity_gain = mix(1.0, random(11), self.opacity_random / 100.0);
            let cross = look.cross.at(across);
            let warped_sample = |u: f32| {
                let (a, ta) = look.guides[0].position(
                    first,
                    &look.density[0],
                    u,
                    self.mask_1_invert,
                    raster_scale,
                );
                let (b, tb) = second_look.position(
                    second,
                    &look.density[1],
                    u,
                    self.mask_2_invert,
                    raster_scale,
                );
                let tangent = unit(add(scale(ta, 1.0 - position), scale(tb, position)));
                let sample = Sample {
                    point: add(a, scale(sub(b, a), position)),
                    size,
                    line,
                    line_count: count,
                    along: u,
                    segments: requested_steps,
                };
                Sample {
                    point: sub(
                        sample.point,
                        scale(normal(tangent), look.distortion.at(sample)),
                    ),
                    ..sample
                }
            };
            let station = |u: f32| {
                let sample = warped_sample(u);
                let (half_a, color_a, opacity_a) = look.guides[0].appearance(u, random(3));
                let (half_b, color_b, opacity_b) = second_look.appearance(u, random(5));
                let mut color = blend(color_a, color_b, shade);
                for channel in 0..3 {
                    color[channel] = if self.gradient_blending == 1 {
                        color[channel] * mix(1.0, cross[channel], cross[3])
                    } else {
                        color[channel] + cross[channel] * cross[3]
                    };
                }
                let sinus = self.alpha_sinus_amplitude
                    * ((u * self.alpha_sinus_frequency
                        + self.alpha_sinus_offset
                        + line as f32 * self.alpha_sinus_line_offset)
                        * TAU)
                        .sin();
                let mut alpha = (self.alpha_noise_brightness
                    + look.alpha.at(sample)
                    + sinus
                    + self.alpha_sinus_constant)
                    .clamp(0.0, 1.0);
                let hardness = (self.coverage_hardness / 100.0).clamp(0.0, 1.0);
                let dry = (self.dry_brush / 100.0).clamp(0.0, 1.0) * look.dry_ramp.at(u);
                if dry > 0.0 {
                    let soft = mix(0.15, 0.01, hardness);
                    let per_grain = path_px / self.dry_brush_scale.max(0.5);
                    let n = grain(
                        &look.dry_field,
                        u * per_grain,
                        if closed { per_grain } else { 0.0 },
                        line,
                        0.0,
                    );
                    let level = dry * (1.0 + 2.0 * soft) - soft;
                    alpha *= smoothstep(level - soft, level + soft, n);
                }
                if hardness > 0.0 {
                    let ceiling =
                        (self.alpha_noise_brightness + self.alpha_sinus_constant).clamp(1e-3, 1.0);
                    let t = (alpha / ceiling).clamp(0.0, 1.0);
                    alpha = ceiling * ((t - 0.5) / (1.0 - 0.98 * hardness) + 0.5).clamp(0.0, 1.0);
                }
                let mut half = mix(half_a, half_b, between) * width_gain;
                let mut shift = 0.0;
                let rough = (self.edge_roughness / 100.0).clamp(0.0, 1.0);
                if rough > 0.0 && !points {
                    let depth = (self.edge_roughness_depth / 100.0).clamp(0.01, 1.0);
                    let edge = if count == 1 {
                        1.0
                    } else {
                        (((2.0 * between - 1.0).abs() - (1.0 - depth)) / depth).clamp(0.0, 1.0)
                    };
                    let k = rough * edge;
                    let per_grain = path_px / self.edge_roughness_scale.max(0.5);
                    let lap = if closed { per_grain } else { 0.0 };
                    let x = u * per_grain;
                    if count > 1 {
                        let side = if between < 0.5 { 0.0 } else { 13.0 };
                        let inward = 2.0 * between.min(1.0 - between);
                        let tear =
                            rough * depth * 1.6 * grain(&look.rough_field, x, lap, 0, 40.0 + side);
                        let fibre =
                            (grain(&look.rough_field, x * 3.0, lap * 3.0, line, 70.0) - 0.5) * 0.08;
                        let w = (1.0 / count as f32).max(0.01);
                        alpha *= smoothstep(tear - w, tear + w, inward + fibre + w);
                    }
                    if k > 0.0 {
                        let low = half * (1.0 - k * grain(&look.rough_field, x, lap, line, 0.0));
                        let high = half * (1.0 - k * grain(&look.rough_field, x, lap, line, 17.0));
                        half = (low + high) * 0.5;
                        shift = (high - low) * 0.5;
                    }
                }
                let opacity = mix(opacity_a, opacity_b, between) * opacity_gain * alpha;
                let tint = (self.density_colour / 100.0).clamp(0.0, 1.0);
                if tint > 0.0 {
                    let p = opacity.clamp(0.0, 1.0);
                    let tone = if p < 0.5 {
                        blend(self.density_colour_thin, self.density_colour_mid, p * 2.0)
                    } else {
                        blend(
                            self.density_colour_mid,
                            self.density_colour_dense,
                            p * 2.0 - 1.0,
                        )
                    };
                    for channel in 0..3 {
                        color[channel] = mix(color[channel], tone[channel], tint);
                    }
                }
                (
                    Station {
                        xy: sample.point,
                        shift,
                        color,
                        half,
                        feather: mix(look.guides[0].feather, second_look.feather, between).max(0.0),
                        opacity,
                    },
                    sample,
                )
            };
            let joined =
                closed && length(sub(warped_sample(0.0).point, warped_sample(1.0).point)) < 1e-3;
            if points {
                let looping = self.stroke_loop;
                let n = steps as f32;
                let (lo, hi) = if looping {
                    (begin + plan.shift, end + plan.shift)
                } else {
                    (
                        (begin + plan.shift).clamp(0.0, 1.0),
                        (end + plan.shift).clamp(0.0, 1.0),
                    )
                };
                let first_k = ((lo - plan.shift) * n - 1e-4).ceil() as i64;
                let last_k = ((hi - plan.shift) * n + 1e-4).floor() as i64;
                let mut marker = |u: f32, place: i64| {
                    if hashed(seed, line, place as u64 ^ MARKER_SALT) * 100.0
                        >= self.visible_points as f32
                    {
                        return;
                    }
                    let (s, sample) = station(u);
                    let mut rotation = (self.angle + look.rotation.at(sample)).to_radians();
                    if self.rotation_along_lines != 0.0 {
                        let delta = 0.25 / steps.max(3) as f32;
                        let before = warped_sample(wrap(u - delta, joined)).point;
                        let after = warped_sample(wrap(u + delta, joined)).point;
                        let tangent = sub(after, before);
                        rotation +=
                            tangent[1].atan2(tangent[0]) * self.rotation_along_lines / 100.0;
                    }
                    mesh.disk(s, sides, rotation, TAU);
                };
                for k in first_k..=last_k {
                    if joined && k - first_k >= steps as i64 {
                        break;
                    }
                    let place = if joined || looping {
                        k.rem_euclid(steps as i64)
                    } else {
                        k
                    };
                    let g = k as f32 / n + plan.shift;
                    if !looping {
                        marker(g.clamp(0.0, 1.0), place);
                        continue;
                    }
                    let mut u = g - g.floor();
                    if u > 1.0 - 1e-5 {
                        u = 0.0;
                    }
                    if u < 1e-5 && !joined {
                        if g > lo + 1e-5 {
                            marker(1.0, place);
                        }
                        if g < hi - 1e-5 {
                            marker(0.0, place);
                        }
                    } else {
                        marker(u, place);
                    }
                }
                continue;
            }
            for (span_idx, &(start, end)) in plan.spans.iter().enumerate() {
                stations.clear();
                for u in plan.samples(start, end, steps) {
                    stations.push(station(u).0);
                }
                if stations.len() >= 2 {
                    let last = stations.len() - 1;
                    let mut before = None;
                    let mut after = None;
                    if joined {
                        if plan.closed {
                            before = Some(stations[last - 1].xy);
                            after = Some(stations[1].xy);
                        } else if plan.wraps {
                            if span_idx > 0 {
                                let (a, b) = plan.spans[span_idx - 1];
                                let at = plan.samples(a, b, steps);
                                before = Some(warped_sample(at[at.len() - 2]).point);
                            }
                            if let Some(&(a, b)) = plan.spans.get(span_idx + 1) {
                                let at = plan.samples(a, b, steps);
                                after = Some(warped_sample(at[1]).point);
                            }
                        }
                    }
                    mesh.strip(&stations, before, after);
                    if plan.leading(span_idx) {
                        let outward = unit(sub(stations[0].xy, stations[1].xy));
                        mesh.cap(
                            centred(stations[0], normal(scale(outward, -1.0))),
                            outward,
                            self.cap_begin,
                            self.arrow_length_begin,
                            self.arrow_angle_begin,
                        );
                    }
                    if plan.trailing(span_idx) {
                        let outward = unit(sub(stations[last].xy, stations[last - 1].xy));
                        mesh.cap(
                            centred(stations[last], normal(outward)),
                            outward,
                            self.cap_end,
                            self.arrow_length_end,
                            self.arrow_angle_end,
                        );
                    }
                }
            }
        }
        mesh.vertices
    }
}

struct StrokePlan {
    line: usize,
    shift: f32,
    spans: Vec<(f32, f32)>,
    closed: bool,
    wraps: bool,
}

impl StrokePlan {
    fn pieces(&self, start: f32, end: f32, steps: usize) -> usize {
        let minimum = if self.closed { 3 } else { 1 };
        (((end - start) * steps as f32).ceil() as usize + 1).max(minimum)
    }

    fn samples(&self, start: f32, end: f32, steps: usize) -> Vec<f32> {
        let n = steps as f32;
        let first = (start * n + 1e-3).ceil() as i64;
        let last = (end * n - 1e-3).floor() as i64;
        let mut at = Vec::with_capacity((last - first).max(0) as usize + 2);
        at.push(start);
        at.extend((first..=last).map(|k| k as f32 / n));
        at.push(end);
        let minimum = if self.closed { 3 } else { 1 };
        if at.len() - 1 < minimum {
            at = (0..=minimum)
                .map(|i| mix(start, end, i as f32 / minimum as f32))
                .collect();
        }
        at
    }

    fn leading(&self, span: usize) -> bool {
        !self.closed && (!self.wraps || span == 0)
    }

    fn trailing(&self, span: usize) -> bool {
        !self.closed && (!self.wraps || span + 1 == self.spans.len())
    }
}

fn stroke_spans(begin: f32, end: f32, looping: bool) -> Vec<(f32, f32)> {
    if !looping {
        let (a, b) = (begin.clamp(0.0, 1.0), end.clamp(0.0, 1.0));
        return if b > a { vec![(a, b)] } else { Vec::new() };
    }
    let mut spans = Vec::with_capacity(4);
    let first = begin.floor() as i32;
    let last = end.ceil() as i32;
    for turn in first..last.min(first.saturating_add(4)) {
        let a = (begin - turn as f32).max(0.0);
        let b = (end - turn as f32).min(1.0);
        if a < b {
            spans.push((a, b));
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::{Noise, Perlin, Ramp, Sample};
    use crate::fx::params::CurvePoints;

    fn noise(line_steps_phase: bool, evolution: f32, line_offset: f32) -> Noise {
        Noise {
            coordinates: 1,
            amplitude: 1.0,
            frequency: 3.0,
            axes: [1.0, 1.0],
            offset: [0.0, 0.0],
            evolution,
            cycle: 0.0,
            line_offset,
            line_steps_phase,
            field: Perlin::new(5),
            ramp: Ramp::new(
                CurvePoints::sanitised(&[[0.0, 1.0], [1.0, 1.0]]),
                0.0,
                100.0,
                false,
            ),
        }
    }

    #[test]
    fn a_phase_step_moves_the_phase() {
        let at = |line| Sample {
            point: [0.0, 0.0],
            size: [100.0, 100.0],
            line,
            line_count: 4,
            along: 0.37,
            segments: 10,
        };
        let stepped = noise(true, 0.0, 0.6).at(at(2));
        let phased = noise(false, 1.2, 0.0).at(at(2));
        assert!((stepped - phased).abs() < 1e-6, "{stepped} vs {phased}");
        let across = noise(false, 0.0, 0.6).at(at(2));
        assert!((across - phased).abs() > 1e-4);
    }
}
