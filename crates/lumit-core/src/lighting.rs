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
    /// A light from before it had rows: its smooth falloff eases out rather
    /// than running straight, and a distance of nought is no falloff.
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
    pub ambient: f32,
    pub diffuse: f32,
    pub specular: f32,
    /// The highlight's Blinn-Phong exponent.
    pub exponent: f32,
    pub metal: f32,
    /// The eye, comp pixels.
    pub eye: [f32; 3],
}

impl ShadingMaterial {
    /// What every layer had before materials: the lights as they were, and no
    /// highlight.
    pub const PLAIN: Self = Self {
        ambient: 1.0,
        diffuse: 0.5,
        specular: 0.0,
        exponent: 1.0,
        metal: 1.0,
        eye: [0.0, 0.0, 0.0],
    };

    /// After Effects' Specular shininess, a percentage, as the exponent: one
    /// more than it, so nought is the broadest highlight there is.
    #[must_use]
    pub fn exponent_for(shininess_pct: f64) -> f32 {
        (1.0 + shininess_pct.max(0.0)) as f32
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
                t
            }
        }
        LightFalloff::InverseSquareClamped => (r / d) * (r / d),
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge1 <= edge0 {
        return f32::from(x >= edge1);
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
    if lights.is_empty() {
        return;
    }
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
            let mut gain = [1.0f32; 3];
            let mut shine = [0.0f32; 3];
            // Diffuse at a half lights a layer as these lights always have.
            let diffuse = 2.0 * m.diffuse;
            for l in lights.iter().take(MAX_LIT_LIGHTS) {
                if l.kind == ShadingKind::Ambient {
                    for (g, c) in gain.iter_mut().zip(l.colour) {
                        *g += m.ambient * c;
                    }
                    continue;
                }
                let e = diffuse * irradiance(p, s.normal, l);
                for (g, c) in gain.iter_mut().zip(l.colour) {
                    *g += e * c;
                }
                if m.specular > 0.0 {
                    let k = m.specular * highlight(p, s.normal, l, m.eye, m.exponent);
                    for (sh, c) in shine.iter_mut().zip(l.colour) {
                        *sh += k * c;
                    }
                }
            }
            let a = px[3];
            for ((ch, g), sh) in px[..3].iter_mut().zip(gain).zip(shine) {
                *ch = *ch * (g + m.metal * sh) + (1.0 - m.metal) * sh * a;
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
        // falloff halfway out squares to a quarter, and so does the inverse
        // square from a radius of 50.
        let below = [0.0, 0.0, 0.0];
        let smooth = |radius_px, falloff_px| ShadingLight {
            falloff: LightFalloff::Smooth,
            radius_px,
            falloff_px,
            ..plain
        };
        assert!(close(irradiance(below, n, &smooth(150.0, 50.0)), 1.0));
        assert!(close(irradiance(below, n, &smooth(50.0, 100.0)), 0.5));
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
        };
        let eye = [0.0, 0.0, -1000.0];
        let under = highlight([0.0, 0.0, 0.0], n, &light, eye, 6.0);
        let aside = highlight([150.0, 0.0, 0.0], n, &light, eye, 6.0);
        assert!((under - 1.0).abs() < 1e-4, "dead on is full, got {under}");
        assert!(aside > 0.0 && aside < under * 0.5, "it fades: {aside}");
        assert!(highlight([150.0, 0.0, 0.0], n, &light, eye, 60.0) < aside);

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
                diffuse: 1.0,
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
}
