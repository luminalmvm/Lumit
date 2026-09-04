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
    /// Comp pixels at which the light has fallen to nothing; 0 means it never
    /// does. The rectangle integral already dims with distance on its own —
    /// this is the artistic dial on top, and the only distance term a point
    /// light has at all.
    pub falloff_px: f32,
    /// True for an area light: use the rectangle integral. False: cosine law.
    pub is_area: bool,
    /// Cosine of a spot's half-angle. Anything below -1 means "not a spot",
    /// which is how a point light says it shines everywhere.
    pub cone_cos: f32,
    /// Unit vector a spot is aimed along.
    pub axis: [f32; 3],
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

/// How much of `light` reaches a point `p` on a surface facing `n`, in 0..1.
#[must_use]
pub fn irradiance(p: [f32; 3], n: [f32; 3], light: &ShadingLight) -> f32 {
    let centre = light.centre();
    let to_light = sub(centre, p);

    // Distance falloff. Squared so it eases out rather than ending on a crease.
    let reach = if light.falloff_px > 0.0 {
        let t = (1.0 - length(to_light) / light.falloff_px).clamp(0.0, 1.0);
        t * t
    } else {
        1.0
    };
    if reach <= 0.0 {
        return 0.0;
    }

    // A spot's cone, softened over the outer tenth so the edge is not a
    // stencil. `cone_cos < -1` is the "not a spot" sentinel.
    let cone = if light.cone_cos < -1.0 {
        1.0
    } else {
        let Some(dir) = normalise(sub(p, centre)) else {
            return 0.0;
        };
        let inner = light.cone_cos + (1.0 - light.cone_cos) * 0.1;
        smoothstep(light.cone_cos, inner, dot(dir, light.axis))
    };
    if cone <= 0.0 {
        return 0.0;
    }

    let e = if light.is_area {
        rect_form_factor(p, n, &light.corners)
    } else {
        // The cosine law: a point light's brightness is how squarely the
        // surface faces it, and nothing else. Distance is the falloff dial
        // above — an inverse square measured in pixels is a number with no
        // meaning, and a compositor would only have to fight it.
        match normalise(to_light) {
            Some(dir) => dot(n, dir).max(0.0),
            None => 0.0,
        }
    };
    e * reach * cone
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
pub fn shade(rgba: &mut [f32], w: u32, h: u32, s: &ShadingSurface, lights: &[ShadingLight]) {
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
            for l in lights.iter().take(MAX_LIT_LIGHTS) {
                let e = irradiance(p, s.normal, l);
                for (g, c) in gain.iter_mut().zip(l.colour) {
                    *g += e * c;
                }
            }
            for (ch, g) in px[..3].iter_mut().zip(gain) {
                *ch *= g;
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

    /// The horizon clip, which is the part most easily got wrong: a light
    /// behind the surface contributes nothing at all, and one straddling the
    /// plane contributes only its front half.
    #[test]
    fn a_light_behind_the_surface_contributes_nothing() {
        let n = [0.0, 0.0, -1.0];
        let p = [0.0, 0.0, 0.0];
        // Entirely behind (positive z is away from the viewer).
        let behind = rect_form_factor(
            p,
            n,
            &[
                [-100.0, -100.0, 200.0],
                [100.0, -100.0, 200.0],
                [100.0, 100.0, 200.0],
                [-100.0, 100.0, 200.0],
            ],
        );
        assert_eq!(behind, 0.0, "a light behind the surface lights nothing");

        // Straddling: half in front, half behind. Must be positive, and must
        // be less than the same light moved wholly in front.
        let straddle = rect_form_factor(
            p,
            n,
            &[
                [-100.0, -100.0, -200.0],
                [100.0, -100.0, -200.0],
                [100.0, 100.0, 200.0],
                [-100.0, 100.0, 200.0],
            ],
        );
        assert!(
            straddle > 0.0,
            "the half in front of the surface still lights it"
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
        shade(&mut untouched, 4, 4, &surface, &[]);
        assert_eq!(untouched, base, "no lights is a bit-exact no-op");

        let light = ShadingLight {
            corners: [
                [-50.0, -50.0, -100.0],
                [50.0, -50.0, -100.0],
                [50.0, 50.0, -100.0],
                [-50.0, 50.0, -100.0],
            ],
            colour: [1.0, 1.0, 1.0],
            falloff_px: 0.0,
            is_area: true,
            cone_cos: -2.0,
            axis: [0.0, 0.0, 1.0],
        };
        let mut lit = base.clone();
        shade(&mut lit, 4, 4, &surface, &[light]);
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
            falloff_px: 0.0,
            is_area: false,
            cone_cos: -2.0,
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
        let under = irradiance([0.0, 0.0, 0.0], n, &spot);
        let aside = irradiance([500.0, 0.0, 0.0], n, &spot);
        assert!(under > 0.0, "under the spot is lit, got {under}");
        assert_eq!(aside, 0.0, "outside the cone is dark, got {aside}");
    }

    /// Falloff reaches nothing at its stated distance rather than merely
    /// getting small — a dial that never quite turns off is a dial nobody can
    /// use.
    #[test]
    fn falloff_reaches_zero_at_the_distance_it_names() {
        let n = [0.0, 0.0, -1.0];
        let light = ShadingLight {
            corners: [[0.0, 0.0, -100.0]; 4],
            colour: [1.0, 1.0, 1.0],
            falloff_px: 200.0,
            is_area: false,
            cone_cos: -2.0,
            axis: [0.0, 0.0, 1.0],
        };
        assert!(irradiance([0.0, 0.0, 0.0], n, &light) > 0.0, "near is lit");
        assert_eq!(
            irradiance([300.0, 0.0, 0.0], n, &light),
            0.0,
            "past the falloff distance is dark"
        );
    }
}
