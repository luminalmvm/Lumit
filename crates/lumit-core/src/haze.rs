//! Volumetric light: a Light layer seen in the air it shines through.
//!
//! Each light has a few rows for how it shows in haze. With Volumetric at
//! nought it shows nothing and the renderer runs nothing new. Above nought,
//! the comp gets one pass after its 3D layers: every pixel's ray is walked
//! from the eye to the nearest 3D layer, and at each step the light falling
//! there is scattered towards the eye. The light is cut by the same casters
//! that throw its shadows, so a 3D layer cuts a shaft through the beam and a
//! coloured one tints it.
//!
//! This module holds the rows and the reference for that walk. The WGSL twin
//! in `lumit-gpu` (`fx_volumetric.wgsl`) mirrors [`march`] step for step.

use crate::anim::Property;
use crate::lighting::{
    arrival, dot, length, normalise, sub, ShadingKind, ShadingLight, ShadowCaster,
};

/// How many steps a ray takes through the haze. A multiple of four, as the
/// kernel takes them four at a time.
pub const STEPS: u32 = 48;

/// How much light the haze scatters towards the eye per comp pixel, at a
/// Volumetric and a Haze density of a hundred percent.
pub const SCATTER_PER_PX: f32 = 0.0025;

/// How much the haze dims what is behind it per comp pixel, at a Haze density
/// of a hundred percent. Kept faint: raising a light should add light, not
/// darken the comp.
pub const EXTINCTION_PER_PX: f32 = 0.000_025;

/// How strongly a parallel light shows beside a beam. It fills all the haze
/// there is, so at the same rows it would wash the picture out.
pub const PARALLEL_SHARE: f32 = 0.02;

/// The same for a point or area light, which glows all round itself where a
/// spot sends its light one way.
pub const POINT_SHARE: f32 = 0.2;

/// How far above the ground the haze halves at a Height falloff of a hundred
/// percent, comp pixels.
pub const FALLOFF_PX: f32 = 100.0;

/// The strongest Forward scatter, short of a beam only seen head on.
const MAX_SCATTER: f32 = 0.95;

/// A Light layer's rows for how it shows in the air.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LightHaze {
    /// Percent. Nought shows nothing.
    pub amount: Property,
    /// Percent: how thick the air is.
    pub density: Property,
    /// Scene-linear RGB the haze tints the light it scatters by.
    pub colour: [Property; 3],
    /// Percent: how fast the haze thins above the ground. Nought is even.
    pub falloff: Property,
    /// Percent, -100 to 100: how much more the light shows looking towards
    /// it than away from it.
    pub scatter: Property,
    /// Percent: how patchy the haze is.
    pub noise: Property,
    /// Comp pixels across one patch.
    pub noise_size: Property,
    /// Comp pixels a second the patches drift.
    pub noise_speed: Property,
}

impl Default for LightHaze {
    fn default() -> Self {
        Self {
            amount: Property::zero(),
            density: Property::fixed(30.0),
            colour: [1.0, 1.0, 1.0].map(Property::fixed),
            falloff: Property::zero(),
            scatter: Property::fixed(30.0),
            noise: Property::fixed(40.0),
            noise_size: Property::fixed(300.0),
            noise_speed: Property::zero(),
        }
    }
}

impl LightHaze {
    /// Left out of a saved light while it is untouched.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Whether the light shows in the air at some moment: Volumetric is
    /// above nought or keyed. What decides whether its other rows show.
    #[must_use]
    pub fn shows(&self) -> bool {
        self.amount.is_animated() || self.amount.value_at(0.0) > 0.0
    }

    /// The same for its Noise, which decides whether Noise size and speed
    /// show.
    #[must_use]
    pub fn noisy(&self) -> bool {
        self.noise.is_animated() || self.noise.value_at(0.0) > 0.0
    }

    /// The row `prop` names, or `None` for one that is not a haze row.
    #[must_use]
    pub fn channel(&self, prop: crate::model::TransformProp) -> Option<&Property> {
        use crate::model::TransformProp as P;
        Some(match prop {
            P::HazeAmount => &self.amount,
            P::HazeDensity => &self.density,
            P::HazeColourR => &self.colour[0],
            P::HazeColourG => &self.colour[1],
            P::HazeColourB => &self.colour[2],
            P::HazeFalloff => &self.falloff,
            P::HazeScatter => &self.scatter,
            P::HazeNoise => &self.noise,
            P::HazeNoiseSize => &self.noise_size,
            P::HazeNoiseSpeed => &self.noise_speed,
            _ => return None,
        })
    }

    pub fn channel_mut(&mut self, prop: crate::model::TransformProp) -> Option<&mut Property> {
        use crate::model::TransformProp as P;
        Some(match prop {
            P::HazeAmount => &mut self.amount,
            P::HazeDensity => &mut self.density,
            P::HazeColourR => &mut self.colour[0],
            P::HazeColourG => &mut self.colour[1],
            P::HazeColourB => &mut self.colour[2],
            P::HazeFalloff => &mut self.falloff,
            P::HazeScatter => &mut self.scatter,
            P::HazeNoise => &mut self.noise,
            P::HazeNoiseSize => &mut self.noise_size,
            P::HazeNoiseSpeed => &mut self.noise_speed,
            _ => return None,
        })
    }

    /// The rows at layer time `lt`.
    #[must_use]
    pub fn at(&self, lt: f64) -> HazeLook {
        let v = |p: &Property| p.value_at(lt) as f32;
        HazeLook {
            amount: v(&self.amount).max(0.0),
            density: v(&self.density).max(0.0),
            colour: self.colour.each_ref().map(|c| v(c).max(0.0)),
            falloff: v(&self.falloff).max(0.0),
            scatter: v(&self.scatter).clamp(-100.0, 100.0),
            noise: v(&self.noise).max(0.0),
            noise_size: v(&self.noise_size).max(0.0),
            drift: (self.noise_speed.value_at(lt) * lt) as f32,
        }
    }
}

/// One light's haze at a moment, as the walk reads it. Percentages as the
/// rows show them; `drift` is how far the patches have moved, comp pixels.
/// `colour` is folded into the light's own colour before the walk.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HazeLook {
    pub amount: f32,
    pub density: f32,
    pub colour: [f32; 3],
    pub falloff: f32,
    pub scatter: f32,
    pub noise: f32,
    pub noise_size: f32,
    pub drift: f32,
}

impl HazeLook {
    /// Hands `feed` what a frame's cache key takes from this light's haze:
    /// nothing while it shows none, and no drift while there is no noise to
    /// move, so a still haze caches across frames.
    pub fn cache_key(&self, feed: impl FnMut(f32)) {
        if self.amount <= 0.0 {
            return;
        }
        let drift = if self.noise > 0.0 { self.drift } else { 0.0 };
        let [r, g, b] = self.colour;
        [
            self.amount,
            self.density,
            r,
            g,
            b,
            self.falloff,
            self.scatter,
            self.noise,
            self.noise_size,
            drift,
        ]
        .into_iter()
        .for_each(feed);
    }
}

/// Where the haze is seen from, in comp pixels: the camera's eye, its right,
/// down-the-picture and forward axes and zoom, the picture's size, and the
/// sphere the lit haze is kept inside, so a view from far off still spends
/// its steps where the lights are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HazeView {
    pub eye: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub zoom: f32,
    pub comp: (f32, f32),
    pub centre: [f32; 3],
    pub radius: f32,
}

/// The step offset a pixel starts its walk at, 0..1. Fixed by where the pixel
/// is, so the same frame always comes out the same.
#[must_use]
pub fn jitter(x: u32, y: u32) -> f32 {
    let f = |v: f32| v - v.floor();
    f(52.982_918 * f(0.067_110_56 * x as f32 + 0.005_837_15 * y as f32))
}

/// How much the haze dims per comp pixel along a ray: the thickest any light
/// asks for, eased in with its Volumetric so the first percent changes little.
#[must_use]
pub fn extinction(lights: &[(ShadingLight, HazeLook)]) -> f32 {
    lights
        .iter()
        .map(|(_, h)| h.density / 100.0 * EXTINCTION_PER_PX * (h.amount / 100.0).min(1.0))
        .fold(0.0, f32::max)
}

/// How thick the haze is at `p` for `h`, around 1: thinner above the ground
/// at `ground` (the 3D views' floor), and patchy with its drifting noise.
fn thickness(p: [f32; 3], h: &HazeLook, ground: f32) -> f32 {
    let above = (ground - p[1]).max(0.0);
    let height = (-h.falloff / 100.0 * above / FALLOFF_PX).exp2();
    if h.noise <= 0.0 || h.noise_size <= 0.0 {
        return height;
    }
    let s = 1.0 / h.noise_size;
    let (x, y, z) = (
        (p[0] + h.drift) * s,
        (p[1] - h.drift * 0.25) * s,
        (p[2] + h.drift * 0.5) * s,
    );
    let n = crate::fx::noise::perlin3(0, 0, x, y, z, 0)
        + 0.5 * crate::fx::noise::perlin3(0, 1, x * 2.0, y * 2.0, z * 2.0, 0);
    height * (1.0 + h.noise / 100.0 * n).max(0.0)
}

/// The light scattered towards the eye along the ray through comp pixel `q`,
/// as far as the ray parameter `end` (where it meets the scene), and how much
/// of what is behind is left: premultiplied RGB light, then transmittance.
/// `jitter` is the pixel's step offset. A ray that misses the haze's sphere
/// adds nothing and leaves everything.
#[must_use]
pub fn march(
    view: &HazeView,
    q: [f32; 2],
    end: f32,
    jitter: f32,
    lights: &[(ShadingLight, HazeLook)],
    casters: &[ShadowCaster],
) -> [f32; 4] {
    let mut out = [0.0, 0.0, 0.0, 1.0];
    let (cx, cy) = (q[0] - view.comp.0 * 0.5, q[1] - view.comp.1 * 0.5);
    let d = [0, 1, 2].map(|k| view.right[k] * cx + view.up[k] * cy + view.forward[k] * view.zoom);
    let dl = length(d);
    let Some(dir) = normalise(d) else {
        return out;
    };
    let oc = sub(view.eye, view.centre);
    let (a, b, c) = (dl * dl, dot(oc, d), dot(oc, oc) - view.radius * view.radius);
    let disc = b * b - a * c;
    if disc <= 0.0 {
        return out;
    }
    let s = disc.sqrt();
    let t0 = ((-b - s) / a).max(0.0);
    let t1 = ((-b + s) / a).min(end);
    if t1 <= t0 {
        return out;
    }
    let sigma = extinction(lights);
    let dt = (t1 - t0) / STEPS as f32;
    let seg = dt * dl;
    for i in 0..STEPS {
        let t = t0 + (i as f32 + jitter) * dt;
        let p = [0, 1, 2].map(|k| view.eye[k] + d[k] * t);
        let left = (-sigma * (t - t0) * dl).exp();
        for (l, h) in lights.iter().take(crate::lighting::MAX_LIT_LIGHTS) {
            if h.amount <= 0.0 {
                continue;
            }
            let Some((to_light, reach, cone)) = arrival(p, l) else {
                continue;
            };
            // A light with no falloff of its own still glows round itself in
            // haze, out to about its radius: a spot along its beam, the
            // others in a tighter ball, as they shine every way.
            let mut glow = 1.0;
            if l.kind == ShadingKind::Parallel {
                glow = PARALLEL_SHARE;
            } else {
                if l.falloff == crate::model::LightFalloff::None {
                    let r = l.radius_px.max(1.0);
                    let far = length(to_light);
                    glow = r * r / (r * r + far * far);
                }
                if l.cone_cos < -1.0 {
                    glow = glow * glow * POINT_SHARE;
                }
            }
            let g = (h.scatter / 100.0).clamp(-MAX_SCATTER, MAX_SCATTER);
            let cos = normalise(to_light).map_or(0.0, |w| dot(w, dir));
            let phase = (1.0 - g * g) / (1.0 + g * g - 2.0 * g * cos).max(1e-6).powf(1.5);
            let mut through = [1.0f32; 3];
            if l.shadow_darkness > 0.0 {
                // On the card, models' shadow maps dim `through` here too
                // (`mapped`), which this has no models for.
                through = crate::lighting::transmitted(p, l, casters, None)
                    .map(|v| 1.0 - l.shadow_darkness * (1.0 - v));
            }
            let k = left * h.amount / 100.0 * h.density / 100.0
                * SCATTER_PER_PX
                * thickness(p, h, view.comp.1 * 0.5)
                * seg
                * reach
                * cone
                * glow
                * phase;
            for c in 0..3 {
                out[c] += k * l.colour[c] * through[c];
            }
        }
    }
    out[3] = (-sigma * (t1 - t0) * dl).exp();
    out
}
