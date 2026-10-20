//! Shading layers with the comp's Light layers.
//!
//! **In plain terms.** A Light layer is a shape that emits: a point, a
//! cone, or — the one that matters — a rectangle, a softbox. This module works
//! out how much of that light lands on each pixel of a layer, so a piece of
//! footage can be lit by a softbox you placed and keyframed rather than by a
//! gradient you painted.
//!
//! The whole thing rests on one old and rather lovely piece of geometry. To
//! know how brightly a flat surface is lit by a flat glowing rectangle, you do
//! not need to fire rays at it. There is a closed-form answer: stand at the
//! point being lit, look up, and measure how much of your sky the rectangle
//! covers — weighted for the fact that light arriving at a glancing angle
//! spreads over more surface and so counts for less. That measurement is a sum
//! of one term per edge of the rectangle, and the sum is exact. Four edges,
//! four terms, done — no sampling, no noise, no render time worth the name.
//!
//! (The term of art is the *diffuse form factor*, and the same integral is the
//! identity-matrix case of Linearly Transformed Cosines — Heitz et al. 2016 —
//! which is how one would later add roughness and specular highlights. The
//! matrix tables that would need are deliberately not here: the diffuse case
//! ships alone.)
//!
//! Two decisions worth knowing before reading the code:
//!
//! - **Light adds, it does not replace.** The result multiplies the picture by
//!   `1 + light`, so a layer that no light reaches is untouched and a comp with
//!   no lights renders byte-for-byte as it did before this existed. Physical
//!   shading would multiply by the light alone and plunge everything unlit into
//!   black, which is the correct answer to a question no compositor is asking.
//! - **The surface is the layer's own plane.** A 2.5D compositor has no
//!   per-pixel normals and inventing them from luminance is a quality cliff, so
//!   every pixel of a layer shares one normal: the direction its plane faces.
//!   For a softbox raking across footage that is exactly right, and it is the
//!   honest answer rather than a guess.

/// How many lights one layer can be shaded by in a single pass. Beyond this
/// the nearest are kept — a budget, not a limit on the model (docs/13).
pub const MAX_LIT_LIGHTS: usize = 8;

/// One light reduced to what shading needs: its emitting rectangle's four
/// corners in comp pixels, and how it falls off. Points and spots collapse to
/// a single position, and are shaded by the ordinary cosine law instead of the
/// rectangle integral.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadingLight {
    /// The emitting rectangle in comp space, wound consistently. For a point
    /// or a spot all four are the light's centre.
    pub corners: [[f32; 3]; 4],
    /// Scene-linear RGB, intensity already folded in.
    pub colour: [f32; 3],
    pub kind: ShadingKind,
    /// How the light weakens with distance from its centre. The rectangle
    /// integral already dims with distance on its own; this is the artistic
    /// dial on top, and the only distance term a point light has at all.
    pub falloff: crate::model::LightFalloff,
    /// Comp pixels of full strength before any falloff starts.
    pub radius_px: f32,
    /// How far past the radius a smooth falloff takes to reach nothing.
    pub falloff_px: f32,
    /// A light from before it had rows: its smooth falloff is the old squared
    /// ramp rather than After Effects' S-curve, and a distance of nought is
    /// no falloff.
    pub legacy: bool,
    /// Cosine of a spot's half-angle. Anything below -1 means "not a spot",
    /// which is how a point light says it shines everywhere.
    pub cone_cos: f32,
    /// Cosine of the angle inside which the cone is at full strength: the
    /// feather runs between this and `cone_cos`.
    pub cone_inner: f32,
    /// Unit vector a spot is aimed along, and the way a parallel light's rays
    /// run.
    pub axis: [f32; 3],
    /// How much of its light the shadows it casts take away, 0..1. Nought
    /// casts none.
    pub shadow_darkness: f32,
    /// How soft those shadows are: the comp pixels their edges spread over.
    pub shadow_diffusion: f32,
}

/// Which law shades a [`ShadingLight`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadingKind {
    /// A point or a spot: the cosine law from its centre.
    Point,
    /// A rectangle: the form factor integral.
    Area,
    /// Rays all running along the axis, with no distance to fall off over.
    Parallel,
    /// Light from everywhere: adds its colour whatever way a surface faces.
    Ambient,
}

impl ShadingLight {
    /// The light's centre — the average of its corners, which is the centre
    /// itself for a point or spot.
    #[must_use]
    pub fn centre(&self) -> [f32; 3] {
        let mut c = [0.0f32; 3];
        for v in &self.corners {
            for k in 0..3 {
                c[k] += v[k] * 0.25;
            }
        }
        c
    }
}

/// The plane a layer's pixels live on, in comp pixels. `origin` is where texel
/// (0, 0) sits; `du`/`dv` are how far one texel of movement in x and y carries
/// you. Affine, because a layer placement is affine — the camera's perspective
/// happens later, when the lit layer is projected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadingSurface {
    pub origin: [f32; 3],
    pub du: [f32; 3],
    pub dv: [f32; 3],
    /// Unit normal of the plane, facing the side the light must come from.
    pub normal: [f32; 3],
}

/// How a layer takes light, its Material options as fractions, and where it
/// is seen from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadingMaterial {
    /// What a pixel's light starts from: 1 where light adds to the picture,
    /// 0 where it lights the layer from black, as in After Effects.
    pub base: f32,
    pub ambient: f32,
    /// What the light reaching a pixel is multiplied by. Twice the Diffuse
    /// option where light adds, so its default half lights a layer as these
    /// lights always have.
    pub diffuse: f32,
    pub specular: f32,
    /// The highlight's Blinn-Phong exponent.
    pub exponent: f32,
    pub metal: f32,
    /// The eye, comp pixels.
    pub eye: [f32; 3],
    /// Light the display-encoded picture, as an After Effects project without
    /// Linearize working space does. The lights' colours are then display
    /// encoded too, which is the caller's to do.
    pub encoded: bool,
    /// Whether shadows fall on it, and Only to show nothing but them.
    pub accepts_shadows: crate::model::ShadowMode,
    /// Accepts lights off: the lights leave it as it is, but their shadows
    /// still darken it, as in After Effects.
    pub unlit: bool,
}

impl ShadingMaterial {
    /// What every layer had before materials: the lights as they were, and no
    /// highlight.
    pub const PLAIN: Self = Self {
        base: 1.0,
        ambient: 1.0,
        diffuse: 1.0,
        specular: 0.0,
        exponent: 1.0,
        metal: 1.0,
        eye: [0.0, 0.0, 0.0],
        encoded: false,
        accepts_shadows: crate::model::ShadowMode::Off,
        unlit: false,
    };

    /// After Effects' Specular shininess, a percentage, as the exponent. It
    /// is the exponent itself there, so nought spreads the highlight over the
    /// whole layer.
    #[must_use]
    pub fn exponent_for(shininess_pct: f64) -> f32 {
        shininess_pct.max(0.0) as f32
    }
}

const INV_TWO_PI: f32 = 0.159_154_94;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

/// Unit vector, or `None` when the input is too short to have a direction —
/// which is a real case here (a light passing exactly through the point it
/// lights), and one that must not divide by zero.
fn normalise(a: [f32; 3]) -> Option<[f32; 3]> {
    let len = length(a);
    if len < 1e-6 || !len.is_finite() {
        return None;
    }
    Some([a[0] / len, a[1] / len, a[2] / len])
}

/// The fraction of the hemisphere above `p` that the rectangle `corners`
/// covers, cosine-weighted — the diffuse form factor, in 0..1 where 1 would be
/// a light filling the entire sky.
///
/// The rectangle is first clipped to the horizon: the part of a light that has
/// sunk below the surface cannot light it, and including it would give a
/// nonsense (often negative) answer. That clip is the reason for the loop
/// below rather than a flat four-term sum, and it is why the polygon can come
/// out with five corners.
#[must_use]
pub fn rect_form_factor(p: [f32; 3], n: [f32; 3], corners: &[[f32; 3]; 4]) -> f32 {
    // Sutherland–Hodgman against the single plane through `p` facing `n`. A
    // convex quad clipped by one plane keeps at most five corners; the array
    // has room to spare and the write is guarded, because an engine crate does
    // not panic (docs/14).
    let mut poly = [[0.0f32; 3]; 8];
    let mut count = 0usize;
    for i in 0..4 {
        let a = sub(corners[i], p);
        let b = sub(corners[(i + 1) % 4], p);
        let da = dot(a, n);
        let db = dot(b, n);
        if da >= 0.0 && count < poly.len() {
            poly[count] = a;
            count += 1;
        }
        if (da >= 0.0) != (db >= 0.0) && count < poly.len() {
            // Where the edge crosses the horizon. The denominator cannot be
            // zero: the signs differ, so the two are not equal.
            let t = da / (da - db);
            poly[count] = [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ];
            count += 1;
        }
    }
    if count < 3 {
        return 0.0;
    }

    // One term per edge of the clipped polygon: the angle the edge subtends,
    // times how squarely that edge's plane faces the surface.
    let mut sum = 0.0f32;
    for i in 0..count {
        let (Some(a), Some(b)) = (normalise(poly[i]), normalise(poly[(i + 1) % count])) else {
            // A corner sitting on the shaded point itself. No meaningful
            // answer exists; nothing is the safe one.
            return 0.0;
        };
        let Some(edge) = normalise(cross(a, b)) else {
            // Collinear corners subtend no angle — a real case for a light
            // squashed to a line, and it contributes nothing.
            continue;
        };
        sum += dot(a, b).clamp(-1.0, 1.0).acos() * dot(edge, n);
    }
    // The magnitude, not the signed sum: the sign only records which way round
    // the corners were listed, and this model has no back face for a light to
    // hide behind — a rectangle emits from both sides. Taking it as read means
    // no caller has to wind its corners a particular way to get light.
    (sum * INV_TWO_PI).abs().min(1.0)
}

/// Which way `light` arrives at `p` from, and how much of it is left there
/// after its falloff and its cone, as the two factors. `None` where none of it
/// arrives, and for an ambient light, which comes from nowhere.
fn arrival(p: [f32; 3], light: &ShadingLight) -> Option<([f32; 3], f32, f32)> {
    let centre = light.centre();
    let to_light = match light.kind {
        ShadingKind::Ambient => return None,
        // Every ray runs the same way, so the light arrives from straight
        // back up the axis wherever the point is.
        ShadingKind::Parallel => [-light.axis[0], -light.axis[1], -light.axis[2]],
        _ => sub(centre, p),
    };

    let reach = if light.kind == ShadingKind::Parallel {
        1.0
    } else {
        reach(length(to_light), light)
    };
    if reach <= 0.0 {
        return None;
    }

    // A spot's cone, softened over its feather so the edge is not a stencil.
    // `cone_cos < -1` is the "not a spot" sentinel.
    let cone = if light.cone_cos < -1.0 || light.kind == ShadingKind::Parallel {
        1.0
    } else {
        smoothstep(
            light.cone_cos,
            light.cone_inner,
            dot(normalise(sub(p, centre))?, light.axis),
        )
    };
    if cone <= 0.0 {
        return None;
    }
    Some((to_light, reach, cone))
}

/// How much of `light` reaches a point `p` on a surface facing `n`, in 0..1.
/// An ambient light has no direction to arrive from, so it is not counted
/// here; [`shade`] adds it whole.
#[must_use]
pub fn irradiance(p: [f32; 3], n: [f32; 3], light: &ShadingLight) -> f32 {
    let Some((to_light, reach, cone)) = arrival(p, light) else {
        return 0.0;
    };
    let e = if light.kind == ShadingKind::Area {
        rect_form_factor(p, n, &light.corners)
    } else {
        // The cosine law: a point light's brightness is how squarely the
        // surface faces it, and nothing else. Distance is the falloff above.
        match normalise(to_light) {
            Some(dir) => dot(n, dir).max(0.0),
            None => 0.0,
        }
    };
    e * reach * cone
}

/// The highlight `light` puts on `p` for an eye at `eye`, in 0..1: Blinn-Phong,
/// the way After Effects' Classic 3D draws it, with `exponent` for how tight
/// it is. An area light is taken from its centre. Nothing where the light or
/// the eye is behind the surface.
#[must_use]
pub fn highlight(
    p: [f32; 3],
    n: [f32; 3],
    light: &ShadingLight,
    eye: [f32; 3],
    exponent: f32,
) -> f32 {
    let Some((to_light, reach, cone)) = arrival(p, light) else {
        return 0.0;
    };
    let (Some(l), Some(v)) = (normalise(to_light), normalise(sub(eye, p))) else {
        return 0.0;
    };
    if dot(n, l) <= 0.0 || dot(n, v) <= 0.0 {
        return 0.0;
    }
    let Some(half) = normalise([l[0] + v[0], l[1] + v[1], l[2] + v[2]]) else {
        return 0.0;
    };
    dot(n, half).max(0.0).powf(exponent) * reach * cone
}

/// How much of a light is left at `d` comp pixels from it, After Effects'
/// way: full strength out to the radius, then the chosen falloff.
fn reach(d: f32, light: &ShadingLight) -> f32 {
    use crate::model::LightFalloff;
    let r = light.radius_px;
    match light.falloff {
        LightFalloff::None => 1.0,
        _ if d <= r => 1.0,
        LightFalloff::Smooth if light.falloff_px <= 0.0 => {
            if light.legacy && r <= 0.0 {
                1.0
            } else {
                0.0
            }
        }
        LightFalloff::Smooth => {
            let t = (1.0 - (d - r) / light.falloff_px).clamp(0.0, 1.0);
            if light.legacy {
                t * t
            } else {
                // After Effects eases both ends: an S from the radius out.
                t * t * (3.0 - 2.0 * t)
            }
        }
        LightFalloff::InverseSquareClamped => (r / d) * (r / d),
    }
}

/// The sRGB curve, carried through nought so a negative light stays negative.
#[must_use]
pub fn encode(x: f32) -> f32 {
    let v = x.abs();
    let e = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    e.copysign(x)
}

fn decode(x: f32) -> f32 {
    let v = x.abs();
    let d = if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    };
    d.copysign(x)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge1 <= edge0 {
        return f32::from(x >= edge1);
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How many layers cast shadows in one frame. Beyond this the rest cast
/// none: a budget, as the lights' is.
pub const MAX_SHADOW_CASTERS: usize = 16;

/// Rays closer to the surface than this many comp pixels meet nothing, so a
/// card lying on a floor does not shade the floor it sits in.
const SHADOW_GAP: f32 = 0.5;

/// Once this little of a light is left, the casters further on change nothing.
const SHADOW_OPAQUE: f32 = 1e-3;

/// A layer that casts shadows, as the lights see it: its plane in comp
/// pixels, how much light passes through it, and its picture shrunk into a
/// square with each smaller level averaged from the one before.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowCaster {
    /// Where the picture's top-left corner sits.
    pub origin: [f32; 3],
    /// Its full width and height as edges in comp space.
    pub u: [f32; 3],
    pub v: [f32; 3],
    /// After Effects' Light transmission, 0..1: how much of the light passing
    /// through takes the layer's colour rather than stopping.
    pub transmission: f32,
    /// Where the picture sits in its square, as shares of the side: the
    /// corner, then the width and height. See [`caster_box`].
    pub place: [f32; 4],
    /// Premultiplied RGBA, the first `size` texels square, each next half.
    pub levels: Vec<Vec<f32>>,
    pub size: u32,
}

/// How many levels a caster square of `size` has: down to a single texel.
#[must_use]
pub fn caster_levels(size: u32) -> u32 {
    size.max(1).ilog2() + 1
}

/// Where a caster's picture sits in its square of `size`, in texels: the
/// corner, then the width and height. The longer side fills three quarters,
/// less where `spread`, the widest soft edge any light gives it in comp
/// pixels, needs a wider empty margin to spread into. The shorter side keeps
/// the plane's shape, so a texel is square in comp pixels.
#[must_use]
pub fn caster_box(size: u32, u_len: f32, v_len: f32, spread: f32) -> [u32; 4] {
    let long = u_len.max(v_len).max(1e-6);
    let share = (1.0 / (1.0 + 2.5 * spread.max(0.0) / long)).min(0.75);
    let fill = size as f32 * share;
    let w = ((fill * u_len / long).round() as u32).clamp(1, size);
    let h = ((fill * v_len / long).round() as u32).clamp(1, size);
    [(size - w) / 2, (size - h) / 2, w, h]
}

/// A caster's picture shrunk into `place` in its square, as the GPU stores
/// it: eight bits a channel, `opacity` already applied, the colour display
/// encoded where the comp lights an encoded picture, and the smaller levels.
#[must_use]
pub fn caster_square(
    rgba: &[f32],
    w: u32,
    h: u32,
    opacity: f32,
    size: u32,
    place: [u32; 4],
    encoded: bool,
) -> Vec<Vec<f32>> {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0;
    let [x0, y0, cw, ch] = place;
    let texel = |x: i64, y: i64| -> [f32; 4] {
        let x = x.clamp(0, i64::from(w) - 1) as usize;
        let y = y.clamp(0, i64::from(h) - 1) as usize;
        let i = (y * w as usize + x) * 4;
        rgba.get(i..i + 4)
            .map_or([0.0; 4], |p| [p[0], p[1], p[2], p[3]])
    };
    let mut first = vec![0.0f32; (size * size * 4) as usize];
    if w > 0 && h > 0 {
        for y in y0..(y0 + ch).min(size) {
            for x in x0..(x0 + cw).min(size) {
                let u = (x - x0) as f32 + 0.5;
                let v = (y - y0) as f32 + 0.5;
                let mut s = bilinear(&texel, u / cw as f32 * w as f32, v / ch as f32 * h as f32);
                for c in &mut s {
                    *c *= opacity;
                }
                if encoded && s[3] > 0.0 {
                    for k in 0..3 {
                        s[k] = encode(s[k] / s[3]) * s[3];
                    }
                }
                let i = ((y * size + x) * 4) as usize;
                for k in 0..4 {
                    first[i + k] = q(s[k]);
                }
            }
        }
    }
    let mut levels = vec![first];
    let mut n = size;
    for _ in 1..caster_levels(size) {
        let half = (n / 2).max(1);
        let Some(prev) = levels.last() else { break };
        let mut next = vec![0.0f32; (half * half * 4) as usize];
        for y in 0..half {
            for x in 0..half {
                for k in 0..4 {
                    let at = |dx: u32, dy: u32| {
                        prev.get((((y * 2 + dy) * n + x * 2 + dx) * 4) as usize + k)
                            .copied()
                            .unwrap_or(0.0)
                    };
                    next[((y * half + x) * 4) as usize + k] =
                        q((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) * 0.25);
                }
            }
        }
        levels.push(next);
        n = half;
    }
    levels
}

/// Bilinear between texel centres at `(x, y)` in texels, the edges held.
fn bilinear(texel: &impl Fn(i64, i64) -> [f32; 4], x: f32, y: f32) -> [f32; 4] {
    let (x, y) = (x - 0.5, y - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (a, b, c, d) = (
        texel(x0, y0),
        texel(x0 + 1, y0),
        texel(x0, y0 + 1),
        texel(x0 + 1, y0 + 1),
    );
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (d[k] - c[k]) * fx;
        out[k] = top + (bottom - top) * fy;
    }
    out
}

impl ShadowCaster {
    /// The square at `uv` (0..1 across it) and level `lod`, blending the two
    /// levels either side and holding the edges, as a GPU's trilinear
    /// sampler does. The edges are the empty margin, so off the square is
    /// nothing.
    fn sample(&self, uv: [f32; 2], lod: f32) -> [f32; 4] {
        let last = self.levels.len().saturating_sub(1);
        let lod = lod.clamp(0.0, last as f32);
        let k0 = lod.floor() as usize;
        let k1 = (k0 + 1).min(last);
        let at = |k: usize| {
            let n = (self.size >> k).max(1);
            let level = self.levels.get(k);
            let texel = |x: i64, y: i64| -> [f32; 4] {
                let x = x.clamp(0, i64::from(n) - 1) as usize;
                let y = y.clamp(0, i64::from(n) - 1) as usize;
                let i = (y * n as usize + x) * 4;
                level
                    .and_then(|l| l.get(i..i + 4))
                    .map_or([0.0; 4], |p| [p[0], p[1], p[2], p[3]])
            };
            bilinear(&texel, uv[0] * n as f32, uv[1] * n as f32)
        };
        let (a, b) = (at(k0), at(k1));
        let f = lod - k0 as f32;
        [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * f)
    }
}

/// How much of `light` gets past the casters to `p`, per channel, before the
/// shadow's darkness. `own` is the layer being shaded, which never shades
/// itself.
fn transmitted(
    p: [f32; 3],
    light: &ShadingLight,
    casters: &[ShadowCaster],
    own: Option<usize>,
) -> [f32; 3] {
    let mut through = [1.0f32; 3];
    // Towards the light: as far as its centre, or for a parallel light back
    // up its rays with no end.
    let (dir, bounded) = match light.kind {
        ShadingKind::Ambient => return through,
        ShadingKind::Parallel => ([-light.axis[0], -light.axis[1], -light.axis[2]], false),
        _ => (sub(light.centre(), p), true),
    };
    let reach = length(dir);
    if reach < 1e-6 {
        return through;
    }
    for (i, c) in casters.iter().enumerate().take(MAX_SHADOW_CASTERS) {
        if own == Some(i) {
            continue;
        }
        let n = cross(c.u, c.v);
        let den = dot(dir, n);
        if den.abs() < 1e-12 {
            continue;
        }
        let t = dot(sub(c.origin, p), n) / den;
        if t * reach <= SHADOW_GAP || (bounded && t >= 1.0) {
            continue;
        }
        let rel = sub(
            [p[0] + dir[0] * t, p[1] + dir[1] * t, p[2] + dir[2] * t],
            c.origin,
        );
        let (cu, cv) = (cross(c.v, n), cross(n, c.u));
        let (du, dv) = (dot(c.u, cu), dot(c.v, cv));
        if du.abs() < 1e-12 || dv.abs() < 1e-12 {
            continue;
        }
        let uv = [dot(rel, cu) / du, dot(rel, cv) / dv];
        let at = [
            c.place[0] + uv[0] * c.place[2],
            c.place[1] + uv[1] * c.place[3],
        ];
        // The diffusion is how far the shadow's edge spreads where it lands,
        // and on the caster it is less by how much nearer the light it is.
        let spread = if bounded {
            light.shadow_diffusion * (1.0 - t)
        } else {
            light.shadow_diffusion
        };
        // A texel is square in comp pixels, so one count serves both ways.
        let size = c.size as f32;
        let texels = spread * c.place[2] * size / length(c.u).max(1e-6);
        let reach = texels / size;
        if at.iter().any(|x| *x < -reach || *x > 1.0 + reach) {
            continue;
        }
        let s = if texels < 0.5 {
            c.sample(at, 0.0)
        } else {
            // Four taps a radius apart, each from the level that averages a
            // radius' worth, so the edge is soft without being blocky. Never
            // further apart than a texel of that level, or each would be a
            // copy of the shadow of its own.
            let lod = texels.log2();
            let last = c.levels.len().saturating_sub(1) as f32;
            let off = texels.min(lod.min(last).exp2()) * 0.5 / size;
            let mut sum = [0.0f32; 4];
            for (ox, oy) in [(-off, -off), (off, -off), (-off, off), (off, off)] {
                let tap = c.sample([at[0] + ox, at[1] + oy], lod);
                for k in 0..4 {
                    sum[k] += tap[k] * 0.25;
                }
            }
            sum
        };
        for k in 0..3 {
            through[k] *= (1.0 - s[3] + c.transmission * s[k]).clamp(0.0, 1.0);
        }
        if through.iter().all(|t| *t < SHADOW_OPAQUE) {
            break;
        }
    }
    through
}

/// Rec. 709 luminance, for how much of a pixel's light a shadow took.
fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The reference implementation of the lighting pass (docs/06): shade one
/// layer's premultiplied linear RGBA in place. The WGSL twin mirrors this
/// op-for-op, and the GPU test compares them (docs/08 §1.6's rule, applied to
/// a pass that is not an effect).
///
/// An empty `lights` returns without touching a byte — the no-op that keeps a
/// comp without lights rendering exactly as it always did.
///
/// The highlight is added on top of the light: a metal one tints by the
/// picture's own colour, the rest of it by the light's alone, held to the
/// pixel's coverage so it stays premultiplied.
pub fn shade(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    s: &ShadingSurface,
    m: &ShadingMaterial,
    lights: &[ShadingLight],
) {
    shade_with_shadows(rgba, w, h, s, m, lights, &[], None);
}

/// [`shade`] with the frame's shadow casters, `own` being this layer's place
/// among them if it casts too. A light that casts shadows is dimmed by what
/// stands between it and each pixel, and a layer that shows only shadows
/// comes out black where light was taken and clear everywhere else.
#[allow(clippy::too_many_arguments)]
pub fn shade_with_shadows(
    rgba: &mut [f32],
    w: u32,
    h: u32,
    s: &ShadingSurface,
    m: &ShadingMaterial,
    lights: &[ShadingLight],
    casters: &[ShadowCaster],
    own: Option<usize>,
) {
    use crate::model::ShadowMode;
    if lights.is_empty() {
        return;
    }
    let takes = m.accepts_shadows != ShadowMode::Off && !casters.is_empty();
    for y in 0..h {
        for x in 0..w {
            let Some(px) = rgba
                .get_mut((y as usize * w as usize + x as usize) * 4..)
                .and_then(|r| r.get_mut(..4))
            else {
                return;
            };
            // Texel centres, so the shading agrees with where the sample is.
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let p = [
                s.origin[0] + s.du[0] * fx + s.dv[0] * fy,
                s.origin[1] + s.du[1] * fx + s.dv[1] * fy,
                s.origin[2] + s.du[2] * fx + s.dv[2] * fy,
            ];
            let a = px[3];
            if m.unlit {
                // Unlit, so only the shadows touch it: each light that falls
                // here dims it by what its casters stopped.
                let mut dim = [1.0f32; 3];
                for l in lights.iter().take(MAX_LIT_LIGHTS) {
                    if !takes || l.shadow_darkness <= 0.0 || irradiance(p, s.normal, l) <= 0.0 {
                        continue;
                    }
                    let t = transmitted(p, l, casters, own);
                    for (d, t) in dim.iter_mut().zip(t) {
                        *d *= 1.0 - l.shadow_darkness * (1.0 - t);
                    }
                }
                if m.accepts_shadows == ShadowMode::Only {
                    let taken = (1.0 - luma(dim)).clamp(0.0, 1.0);
                    px.copy_from_slice(&[0.0, 0.0, 0.0, a * taken]);
                } else if m.encoded && a > 0.0 {
                    for (ch, d) in px[..3].iter_mut().zip(dim) {
                        *ch = decode(encode(*ch / a) * d) * a;
                    }
                } else if !m.encoded {
                    for (ch, d) in px[..3].iter_mut().zip(dim) {
                        *ch *= d;
                    }
                }
                continue;
            }
            let mut gain = [m.base; 3];
            let mut unshadowed = [m.base; 3];
            let mut shine = [0.0f32; 3];
            let diffuse = m.diffuse;
            for l in lights.iter().take(MAX_LIT_LIGHTS) {
                let colour = l.colour;
                if l.kind == ShadingKind::Ambient {
                    for ((g, u), c) in gain.iter_mut().zip(&mut unshadowed).zip(colour) {
                        *g += m.ambient * c;
                        *u += m.ambient * c;
                    }
                    continue;
                }
                let lit = irradiance(p, s.normal, l);
                let e = diffuse * lit;
                // What the shadow leaves of this light: all of it, less the
                // darkness's share of what the casters stopped.
                let left = if takes && l.shadow_darkness > 0.0 && lit > 0.0 {
                    transmitted(p, l, casters, own).map(|t| 1.0 - l.shadow_darkness * (1.0 - t))
                } else {
                    [1.0; 3]
                };
                for (((g, u), c), k) in gain.iter_mut().zip(&mut unshadowed).zip(colour).zip(left) {
                    *g += e * c * k;
                    *u += e * c;
                }
                if m.specular > 0.0 {
                    let k = m.specular * highlight(p, s.normal, l, m.eye, m.exponent);
                    for ((sh, c), f) in shine.iter_mut().zip(colour).zip(left) {
                        *sh += k * c * f;
                    }
                }
            }
            if m.accepts_shadows == ShadowMode::Only {
                // Black, as opaque as the share of light the shadows took.
                let lit = luma(unshadowed);
                let taken = if lit > 1e-6 {
                    (1.0 - luma(gain) / lit).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                px.copy_from_slice(&[0.0, 0.0, 0.0, a * taken]);
                continue;
            }
            if m.encoded && a <= 0.0 {
                continue;
            }
            for ((ch, g), sh) in px[..3].iter_mut().zip(gain).zip(shine) {
                // A negative light can take a pixel to black but no further.
                let sh = sh.max(0.0);
                if m.encoded {
                    // Out of premultiplied linear and into what After Effects
                    // would have lit, and back.
                    let v = encode(*ch / a) * (g + m.metal * sh).max(0.0) + (1.0 - m.metal) * sh;
                    *ch = decode(v) * a;
                } else {
                    *ch = *ch * (g + m.metal * sh).max(0.0) + (1.0 - m.metal) * sh * a;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square light directly overhead, at a distance, as the classic sanity
    /// check on any form-factor implementation: it must land between 0 and 1,
    /// grow as the light grows, and shrink as the light retreats. Any sign
    /// error or missing normalisation breaks one of the three.
    #[test]
    fn a_rectangle_overhead_covers_more_sky_as_it_grows_and_less_as_it_retreats() {
        let n = [0.0, 0.0, -1.0];
        let p = [0.0, 0.0, 0.0];
        let rect = |half: f32, z: f32| {
            [
                [-half, -half, z],
                [half, -half, z],
                [half, half, z],
                [-half, half, z],
            ]
        };
        let small = rect_form_factor(p, n, &rect(50.0, -200.0));
        let big = rect_form_factor(p, n, &rect(200.0, -200.0));
        let far = rect_form_factor(p, n, &rect(50.0, -800.0));

        for (name, v) in [("small", small), ("big", big), ("far", far)] {
            assert!(
                (0.0..=1.0).contains(&v),
                "{name} form factor {v} is outside the 0..1 a fraction of the sky can be"
            );
        }
        assert!(
            big > small,
            "a wider light covers more sky: {big} vs {small}"
        );
        assert!(far < small, "a further light covers less: {far} vs {small}");

        // A light that fills nearly the whole sky approaches 1, and never
        // exceeds it — the property that keeps the exposure sane.
        let huge = rect_form_factor(p, n, &rect(100_000.0, -1.0));
        assert!(
            huge > 0.98 && huge <= 1.0,
            "a light filling the sky is ~1, got {huge}"
        );
    }

    /// Light adds, it does not replace: no lights leaves
    /// the picture untouched to the bit, and a light can only brighten.
    #[test]
    fn shading_without_lights_changes_nothing_and_a_light_only_brightens() {
        let base: Vec<f32> = (0..16 * 4).map(|i| (i % 7) as f32 * 0.1).collect();
        let surface = ShadingSurface {
            origin: [0.0, 0.0, 0.0],
            du: [1.0, 0.0, 0.0],
            dv: [0.0, 1.0, 0.0],
            normal: [0.0, 0.0, -1.0],
        };

        let mut untouched = base.clone();
        shade(&mut untouched, 4, 4, &surface, &ShadingMaterial::PLAIN, &[]);
        assert_eq!(untouched, base, "no lights is a bit-exact no-op");

        let light = ShadingLight {
            corners: [
                [-50.0, -50.0, -100.0],
                [50.0, -50.0, -100.0],
                [50.0, 50.0, -100.0],
                [-50.0, 50.0, -100.0],
            ],
            colour: [1.0, 1.0, 1.0],
            kind: ShadingKind::Area,
            falloff: crate::model::LightFalloff::None,
            radius_px: 0.0,
            falloff_px: 0.0,
            legacy: false,
            cone_cos: -2.0,
            cone_inner: -2.0,
            axis: [0.0, 0.0, 1.0],
            shadow_darkness: 0.0,
            shadow_diffusion: 0.0,
        };
        let mut lit = base.clone();
        shade(&mut lit, 4, 4, &surface, &ShadingMaterial::PLAIN, &[light]);
        for (i, (a, b)) in base.iter().zip(&lit).enumerate() {
            if i % 4 == 3 {
                assert_eq!(a, b, "alpha is untouched at {i}");
            } else {
                assert!(b >= a, "light never darkens: {b} < {a} at {i}");
            }
        }
        assert!(lit != base, "a light overhead actually did something");
    }

    /// A point light obeys the cosine law and nothing else, and a spot cuts
    /// off outside its cone — the two non-area kinds, which would otherwise
    /// silently do nothing at all.
    #[test]
    fn a_point_light_follows_the_cosine_law_and_a_spot_respects_its_cone() {
        let n = [0.0, 0.0, -1.0];
        let at = |x: f32, y: f32, z: f32| ShadingLight {
            corners: [[x, y, z]; 4],
            colour: [1.0, 1.0, 1.0],
            kind: ShadingKind::Point,
            falloff: crate::model::LightFalloff::None,
            radius_px: 0.0,
            falloff_px: 0.0,
            legacy: false,
            cone_cos: -2.0,
            cone_inner: -2.0,
            axis: [0.0, 0.0, 1.0],
            shadow_darkness: 0.0,
            shadow_diffusion: 0.0,
        };
        let overhead = irradiance([0.0, 0.0, 0.0], n, &at(0.0, 0.0, -100.0));
        let oblique = irradiance([0.0, 0.0, 0.0], n, &at(100.0, 0.0, -100.0));
        assert!(
            (overhead - 1.0).abs() < 1e-5,
            "straight on is full strength, got {overhead}"
        );
        assert!(
            oblique < overhead && oblique > 0.0,
            "45 degrees off is dimmer but not dark, got {oblique}"
        );
        assert_eq!(
            irradiance([0.0, 0.0, 0.0], n, &at(0.0, 0.0, 100.0)),
            0.0,
            "a light behind the surface lights nothing"
        );

        // A narrow spot aimed straight down the +z axis: lit under it, dark
        // well outside it.
        let mut spot = at(0.0, 0.0, -100.0);
        spot.cone_cos = 20f32.to_radians().cos();
        spot.cone_inner = spot.cone_cos + (1.0 - spot.cone_cos) * 0.1;
        let under = irradiance([0.0, 0.0, 0.0], n, &spot);
        let aside = irradiance([500.0, 0.0, 0.0], n, &spot);
        assert!(under > 0.0, "under the spot is lit, got {under}");
        assert_eq!(aside, 0.0, "outside the cone is dark, got {aside}");
    }

    /// The terms the light rows brought, each against the plain light it
    /// refines: the feather, the two falloffs with their radius, the parallel
    /// light and the ambient one.
    #[test]
    fn feather_falloff_parallel_and_ambient_shade_as_after_effects_does() {
        use crate::model::LightFalloff;
        let n = [0.0, 0.0, -1.0];
        let plain = ShadingLight {
            corners: [[0.0, 0.0, -100.0]; 4],
            colour: [1.0, 1.0, 1.0],
            kind: ShadingKind::Point,
            falloff: LightFalloff::None,
            radius_px: 0.0,
            falloff_px: 0.0,
            legacy: false,
            cone_cos: -2.0,
            cone_inner: -2.0,
            axis: [0.0, 0.0, 1.0],
            shadow_darkness: 0.0,
            shadow_diffusion: 0.0,
        };
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;

        // A 30 degree spot seen 25 degrees off its axis: no feather is full
        // strength there, all feather has started to fade.
        let cos30 = 30f32.to_radians().cos();
        let off_axis = [100.0 * 25f32.to_radians().tan(), 0.0, 0.0];
        let hard = ShadingLight {
            cone_cos: cos30,
            cone_inner: cos30,
            ..plain
        };
        let soft = ShadingLight {
            cone_inner: 1.0,
            ..hard
        };
        let (h, s) = (
            irradiance(off_axis, n, &hard),
            irradiance(off_axis, n, &soft),
        );
        assert!(s > 0.0 && s < h * 0.5, "the feather fades: {s} vs {h}");

        // 100 pixels away: inside a 150 radius is full strength, a smooth
        // falloff halfway out is a half, and the inverse square from a radius
        // of 50 is a quarter.
        let below = [0.0, 0.0, 0.0];
        let smooth = |radius_px, falloff_px| ShadingLight {
            falloff: LightFalloff::Smooth,
            radius_px,
            falloff_px,
            ..plain
        };
        assert!(close(irradiance(below, n, &smooth(150.0, 50.0)), 1.0));
        assert!(close(irradiance(below, n, &smooth(50.0, 100.0)), 0.5));
        // After Effects' own, read off its render of a spot falling off from
        // 350 over 350, at 362, 512, 637 and 687 pixels from the light.
        for (d, ae) in [(362.0, 0.995), (512.0, 0.554), (637.0, 0.086), (687.0, 0.0)] {
            let reach = irradiance(
                below,
                n,
                &ShadingLight {
                    corners: [[0.0, 0.0, -d]; 4],
                    ..smooth(350.0, 350.0)
                },
            );
            assert!(
                (reach - ae).abs() < 0.01,
                "{d} px: {reach}, After Effects {ae}"
            );
        }
        // An old light keeps the eased ramp it had, and a distance of nought
        // there is no falloff at all.
        let old = |radius_px, falloff_px| ShadingLight {
            legacy: true,
            ..smooth(radius_px, falloff_px)
        };
        assert!(close(irradiance(below, n, &old(50.0, 100.0)), 0.25));
        assert!(close(irradiance(below, n, &old(0.0, 0.0)), 1.0));
        let inverse = ShadingLight {
            falloff: LightFalloff::InverseSquareClamped,
            radius_px: 50.0,
            ..plain
        };
        assert!(close(irradiance(below, n, &inverse), 0.25));

        // A parallel light shining straight in lights every point fully,
        // however far from its layer.
        let sun = ShadingLight {
            kind: ShadingKind::Parallel,
            ..plain
        };
        assert!(close(irradiance([5000.0, -3000.0, 0.0], n, &sun), 1.0));

        // An ambient light lifts a surface facing anywhere by its colour.
        let ambient = ShadingLight {
            kind: ShadingKind::Ambient,
            colour: [0.5, 0.25, 0.0],
            ..plain
        };
        let surface = ShadingSurface {
            origin: [0.0, 0.0, 0.0],
            du: [1.0, 0.0, 0.0],
            dv: [0.0, 1.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        };
        let mut px = vec![0.4, 0.4, 0.4, 1.0];
        shade(&mut px, 1, 1, &surface, &ShadingMaterial::PLAIN, &[ambient]);
        assert!(close(px[0], 0.6) && close(px[1], 0.5) && close(px[2], 0.4));

        // A negative light takes light away, down to black and no further.
        let dark = ShadingLight {
            colour: [-0.5, -3.0, 0.0],
            ..ambient
        };
        let mut px = vec![0.4, 0.4, 0.4, 1.0];
        shade(&mut px, 1, 1, &surface, &ShadingMaterial::PLAIN, &[dark]);
        assert!(close(px[0], 0.2) && px[1] == 0.0 && close(px[2], 0.4));
    }

    /// The Material options: Diffuse scales the light, with a half the light
    /// as it always was, and Specular adds a highlight under a light and an eye
    /// straight overhead that fades away from them, tinted by the picture when
    /// metal and by the light when not.
    #[test]
    fn diffuse_scales_the_light_and_specular_adds_a_highlight() {
        use crate::model::LightFalloff;
        let n = [0.0, 0.0, -1.0];
        let light = ShadingLight {
            corners: [[0.0, 0.0, -100.0]; 4],
            colour: [1.0, 1.0, 1.0],
            kind: ShadingKind::Point,
            falloff: LightFalloff::None,
            radius_px: 0.0,
            falloff_px: 0.0,
            legacy: false,
            cone_cos: -2.0,
            cone_inner: -2.0,
            axis: [0.0, 0.0, 1.0],
            shadow_darkness: 0.0,
            shadow_diffusion: 0.0,
        };
        let eye = [0.0, 0.0, -1000.0];
        let under = highlight([0.0, 0.0, 0.0], n, &light, eye, 6.0);
        let aside = highlight([150.0, 0.0, 0.0], n, &light, eye, 6.0);
        assert!((under - 1.0).abs() < 1e-4, "dead on is full, got {under}");
        assert!(aside > 0.0 && aside < under * 0.5, "it fades: {aside}");
        assert!(highlight([150.0, 0.0, 0.0], n, &light, eye, 60.0) < aside);
        // After Effects' own, from its render of a light at (420, 120, -300)
        // on a card at Shininess 0 seen from (320, 180, -888.89). Specular and
        // Diffuse are both 50 there, so a pixel over the same pixel with no
        // Specular is 1 + highlight / irradiance.
        let lamp = ShadingLight {
            corners: [[420.0, 120.0, -300.0]; 4],
            ..light
        };
        let ae_eye = [320.0, 180.0, -888.89];
        let exponent = ShadingMaterial::exponent_for(0.0);
        for (x, y, ae) in [(0.5, 359.5, 1.882), (60.5, 300.5, 1.684)] {
            let p = [x, y, 0.0];
            let ratio = highlight(p, n, &lamp, ae_eye, exponent) / irradiance(p, n, &lamp);
            assert!(
                (ratio - ae).abs() < 0.02,
                "({x}, {y}): {ratio}, After Effects {ae}"
            );
        }

        let surface = ShadingSurface {
            origin: [0.0, 0.0, 0.0],
            du: [1.0, 0.0, 0.0],
            dv: [0.0, 1.0, 0.0],
            normal: n,
        };
        let shaded = |m: ShadingMaterial, px: [f32; 4]| {
            let mut px = px.to_vec();
            shade(&mut px, 1, 1, &surface, &m, &[light]);
            px
        };
        let grey = [0.25, 0.25, 0.25, 1.0];
        let plain = ShadingMaterial {
            eye,
            ..ShadingMaterial::PLAIN
        };
        let full = shaded(
            ShadingMaterial {
                diffuse: 2.0,
                ..plain
            },
            grey,
        );
        let half = shaded(plain, grey);
        assert!(full[0] > half[0], "more diffuse, more light");
        assert!(
            shaded(
                ShadingMaterial {
                    diffuse: 0.0,
                    ..plain
                },
                grey
            ) == grey,
            "no diffuse takes no light"
        );
        // After Effects' way lights the layer from black: a light straight
        // on at a Diffuse of a half gives half the picture.
        let ae = shaded(
            ShadingMaterial {
                base: 0.0,
                diffuse: 0.5,
                ..plain
            },
            grey,
        );
        assert!((ae[0] - 0.125).abs() < 1e-4, "{ae:?}");
        // And without Linearize working space it lights what the screen
        // shows: half the encoded grey, decoded again.
        let shown = shaded(
            ShadingMaterial {
                base: 0.0,
                diffuse: 0.5,
                encoded: true,
                ..plain
            },
            grey,
        );
        let want = decode(encode(0.25) * 0.5);
        assert!((shown[0] - want).abs() < 1e-4, "{shown:?} vs {want}");

        // A black pixel has no colour of its own for a metal highlight to
        // take, so only the light's own highlight shows on it.
        let black = [0.0, 0.0, 0.0, 1.0];
        let shiny = ShadingMaterial {
            specular: 0.5,
            exponent: 6.0,
            ..plain
        };
        assert_eq!(shaded(shiny, black)[0], 0.0);
        let plastic = shaded(
            ShadingMaterial {
                metal: 0.0,
                ..shiny
            },
            black,
        );
        assert!(plastic[0] > 0.0 && plastic[3] == 1.0, "{plastic:?}");
        assert!(shaded(shiny, grey)[0] > half[0], "the highlight adds");
    }

    /// A card halfway between a light and a floor shades the floor under it
    /// and nowhere else, by its darkness. A red card letting light through
    /// tints it red, the card never shades itself, a floor showing only
    /// shadows is black under the card and clear beside it, and an unlit one
    /// is only darkened.
    #[test]
    fn a_card_casts_its_shadow_on_the_floor_beneath_it() {
        use crate::model::{LightFalloff, ShadowMode};
        let light = ShadingLight {
            corners: [[0.0, 0.0, -200.0]; 4],
            colour: [1.0, 1.0, 1.0],
            kind: ShadingKind::Point,
            falloff: LightFalloff::None,
            radius_px: 0.0,
            falloff_px: 0.0,
            legacy: false,
            cone_cos: -2.0,
            cone_inner: -2.0,
            axis: [0.0, 0.0, 1.0],
            shadow_darkness: 1.0,
            shadow_diffusion: 0.0,
        };
        let place = caster_box(64, 40.0, 40.0, 0.0);
        let card = |rgb: [f32; 3], transmission: f32| ShadowCaster {
            origin: [-20.0, -20.0, -100.0],
            u: [40.0, 0.0, 0.0],
            v: [0.0, 40.0, 0.0],
            transmission,
            place: place.map(|t| t as f32 / 64.0),
            levels: caster_square(
                &[rgb[0], rgb[1], rgb[2], 1.0].repeat(16),
                4,
                4,
                1.0,
                64,
                place,
                false,
            ),
            size: 64,
        };
        let white = card([1.0, 1.0, 1.0], 0.0);
        // Two floor texels: one under the card, one well to the side.
        let floor = ShadingSurface {
            origin: [-50.0, -0.5, 0.0],
            du: [100.0, 0.0, 0.0],
            dv: [0.0, 1.0, 0.0],
            normal: [0.0, 0.0, -1.0],
        };
        let m = ShadingMaterial {
            base: 0.0,
            accepts_shadows: ShadowMode::On,
            ..ShadingMaterial::PLAIN
        };
        let lit = |m: &ShadingMaterial, l: &ShadingLight, c: &[ShadowCaster], own| {
            let mut px = [0.5, 0.5, 0.5, 1.0].repeat(2);
            shade_with_shadows(&mut px, 2, 1, &floor, m, &[*l], c, own);
            px
        };
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let px = lit(&m, &light, std::slice::from_ref(&white), None);
        assert!(close(px[0], 0.0), "under the card is dark: {px:?}");
        assert!(px[4] > 0.4, "beside it is lit: {px:?}");

        let half = ShadingLight {
            shadow_darkness: 0.5,
            ..light
        };
        let px = lit(&m, &half, std::slice::from_ref(&white), None);
        assert!(close(px[0], 0.25), "half as dark: {px:?}");

        let red = card([1.0, 0.0, 0.0], 1.0);
        let px = lit(&m, &light, &[red], None);
        assert!(
            close(px[0], 0.5) && close(px[1], 0.0),
            "red gets through: {px:?}"
        );

        let px = lit(&m, &light, std::slice::from_ref(&white), Some(0));
        assert!(close(px[0], 0.5), "its own card is not in its way: {px:?}");

        let only = ShadingMaterial {
            accepts_shadows: ShadowMode::Only,
            ..m
        };
        let px = lit(&only, &light, std::slice::from_ref(&white), None);
        assert_eq!(&px[..3], &[0.0; 3]);
        assert!(close(px[3], 1.0) && px[7] < 1e-3, "only the shadow: {px:?}");

        // Accepts lights off: left as it is, but still in the shadow.
        let unlit = ShadingMaterial { unlit: true, ..m };
        let px = lit(&unlit, &light, &[white], None);
        assert!(
            close(px[0], 0.0) && px[4] == 0.5,
            "shadowed, not lit: {px:?}"
        );
    }
}
