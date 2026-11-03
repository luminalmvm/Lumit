//! Shadows between 3D layers: which layers cast on which, and the squares
//! their pictures are shrunk into for the lighting pass.

use super::lighting::lighting_op;
use super::{scene, Realiser};
use crate::draw::{CompLayerDraw, DrawSource};

impl Realiser<'_> {
    /// [`Self::shadow_casters`] from the pictures drawn so far in `order`,
    /// which [`models_last`] made: every layer's but the models', which cast
    /// none of these.
    #[allow(clippy::type_complexity)]
    pub(super) fn casters_so_far(
        &self,
        layers: &[CompLayerDraw],
        (order, drawn, grown): (&[usize], &[wgpu::Texture], &[(f32, f32)]),
        (width, height): (u32, u32),
        eye: [f32; 3],
    ) -> (
        Vec<lumit_gpu::fx::lighting::ShadowCasters>,
        Vec<(Option<usize>, Option<usize>)>,
    ) {
        let none = self.transparent(1, 1);
        let mut textures = vec![none; layers.len()];
        let mut grows = vec![(1.0, 1.0); layers.len()];
        for ((&i, t), g) in order.iter().zip(drawn).zip(grown) {
            textures[i] = t.clone();
            grows[i] = *g;
        }
        self.shadow_casters(layers, &textures, &grows, width, height, eye)
    }

    /// Each layer's shadows: which run of 3D layers' casters it takes, and its
    /// own place among them. A 2D layer ends a run, as in After Effects, so
    /// the layers either side of one don't shade each other. The runs share
    /// the frame's budget for their squares.
    #[allow(clippy::type_complexity)]
    pub(super) fn shadow_casters(
        &self,
        layers: &[CompLayerDraw],
        textures: &[wgpu::Texture],
        grown: &[(f32, f32)],
        width: u32,
        height: u32,
        eye: [f32; 3],
    ) -> (
        Vec<lumit_gpu::fx::lighting::ShadowCasters>,
        Vec<(Option<usize>, Option<usize>)>,
    ) {
        let mut plans = Vec::new();
        let mut start = 0;
        while start < layers.len() {
            let end = (start..layers.len())
                .find(|&i| !layers[i].three_d || layers[i].ends_run)
                .unwrap_or(layers.len());
            let run = start..end;
            if let Some(plan) = run_plan(
                &layers[run.clone()],
                &textures[run.clone()],
                &grown[run],
                eye,
            ) {
                plans.push((start, plan));
            }
            start = end + 1;
        }
        let share = SHADOW_BUDGET / plans.len().max(1) as u64;
        let frame = width.max(height).max(64).next_power_of_two();
        let mut sets = Vec::new();
        let mut each = vec![(None, None); layers.len()];
        // ponytail: made again every frame. Keep a run's squares while its
        // casters' pictures and places hold still if they show in a profile.
        for (start, plan) in plans {
            let casters: Vec<(&wgpu::Texture, lumit_gpu::fx::lighting::CasterPlane)> = plan
                .kept
                .iter()
                .map(|&(i, p)| (&textures[start + i], p))
                .collect();
            let longest = casters
                .iter()
                .map(|(t, _)| t.width().max(t.height()))
                .max()
                .unwrap_or(0);
            // The picture fills three quarters of its square at most.
            let mut size = (longest.saturating_mul(4) / 3)
                .max(64)
                .next_power_of_two()
                .min(frame);
            while size > 64 && u64::from(size).pow(2) * 4 * 4 / 3 * casters.len() as u64 > share {
                size /= 2;
            }
            let Some(set) =
                self.fx
                    .shadow_casters(&self.ctx, &casters, size, plan.encoded, plan.spread)
            else {
                continue;
            };
            for (k, own) in plan.own.into_iter().enumerate() {
                each[start + k] = (Some(sets.len()), own);
            }
            sets.push(set);
        }
        (sets, each)
    }
}

/// How much video memory one frame's shadow casters may take between them,
/// shared out between the runs of 3D layers that cast.
const SHADOW_BUDGET: u64 = 96 << 20;

/// Which of a run's layers cast, chosen before any square is made.
struct RunPlan {
    /// Each kept caster's place in the run and its plane, in stack order.
    kept: Vec<(usize, lumit_gpu::fx::lighting::CasterPlane)>,
    /// Each layer's place among the kept casters.
    own: Vec<Option<usize>>,
    /// Whether the comp lights the display-encoded picture.
    encoded: bool,
    /// The widest soft edge any light here gives a caster, comp pixels.
    spread: f32,
}

/// One run's casters and each layer's place among them. Only the casters
/// that can shade a layer in the run are kept, the 16 nearest the layers
/// they shade when there are more. `None` when nothing in the run casts on
/// anything.
fn run_plan(
    layers: &[CompLayerDraw],
    textures: &[wgpu::Texture],
    grown: &[(f32, f32)],
    eye: [f32; 3],
) -> Option<RunPlan> {
    use lumit_gpu::fx::lighting::{
        CasterPlane, LightingShadows, DIFFUSION_SPREAD, MAX_SHADOW_CASTERS, MAX_STRETCH,
    };
    let centre =
        |c: &[[f32; 3]; 4]| [0, 1, 2].map(|k| (c[0][k] + c[1][k] + c[2][k] + c[3][k]) * 0.25);
    let receivers: Vec<(usize, &CompLayerDraw, [[f32; 3]; 4])> = layers
        .iter()
        .zip(textures)
        .enumerate()
        // A model isn't a plane: it casts none of these, and takes them by
        // its middle, below.
        .filter(|(_, (l, _))| l.material.accepts_shadows != LightingShadows::Off)
        .filter(|(_, (l, _))| !matches!(l.source, DrawSource::Model(_)))
        .filter_map(|(i, (l, tex))| {
            let op = lighting_op(l, tex.width(), tex.height(), eye, false)?;
            Some((i, l, op.corners(tex.width(), tex.height())))
        })
        .collect();
    let models: Vec<(&CompLayerDraw, [f32; 3])> = layers
        .iter()
        .filter(|l| l.material.accepts_shadows != LightingShadows::Off)
        .filter(|_| crate::model::display().takes_shadows())
        .filter_map(|l| Some((l, crate::model_layer::middle(l)?.to_array())))
        .collect();
    let mut kept: Vec<(f32, usize, CasterPlane)> = Vec::new();
    for (i, ((l, tex), g)) in layers.iter().zip(textures).zip(grown).enumerate() {
        let model = matches!(l.source, DrawSource::Model(_)) || scene::draws_in_scene(l);
        if l.material.casts_shadows == LightingShadows::Off || l.opacity <= 0.0 || model {
            continue;
        }
        let plane = crate::build::shading::caster_plane(tex, l, *g);
        let (o, u, v) = (plane.origin, plane.u, plane.v);
        let at = |a: f32, b: f32| [0, 1, 2].map(|k| o[k] + u[k] * a + v[k] * b);
        let corners = [o, at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0)];
        let mid = centre(&corners);
        // How near it is to the nearest layer it can shade, if any.
        let near = receivers
            .iter()
            .filter(|(r, l, corners)| {
                *r != i
                    && (scene::draws_in_scene(l)
                        || l.lights
                            .iter()
                            .any(|g| lumit_gpu::fx::lighting::may_shade(corners, g, &plane)))
            })
            .map(|(_, _, corners)| centre(corners))
            .chain(models.iter().map(|(_, c)| *c))
            .map(|c| (0..3).map(|k| (c[k] - mid[k]).powi(2)).sum::<f32>())
            .min_by(f32::total_cmp);
        if let Some(near) = near {
            kept.push((near, i, plane));
        }
    }
    if kept.is_empty() {
        return None;
    }
    // ponytail: the farthest past 16 drop without a word. Say so once the
    // render has a list of notes to show, beside lights past eight.
    kept.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    kept.truncate(MAX_SHADOW_CASTERS);
    kept.sort_by_key(|k| k.1);
    let mut own = vec![None; layers.len()];
    for (k, (_, i, _)) in kept.iter().enumerate() {
        own[*i] = Some(k);
    }
    let lights = (receivers.iter().map(|(_, l, _)| *l))
        .chain(models.iter().map(|(l, _)| *l))
        .flat_map(|l| &l.lights);
    Some(RunPlan {
        kept: kept.into_iter().map(|(_, i, p)| (i, p)).collect(),
        own,
        // The squares keep their colour the way the comp lights it.
        encoded: receivers.iter().any(|(_, l, _)| l.material.encoded),
        spread: lights
            .filter(|g| g.shadow_darkness > 0.0)
            .map(|g| g.shadow_diffusion * DIFFUSION_SPREAD * MAX_STRETCH)
            .fold(0.0, f32::max),
    })
}

/// Whether any layer in `layers` casts a shadow that a light carries onto
/// another layer taking them.
pub(super) fn casts_shadows(layers: &[CompLayerDraw]) -> bool {
    use lumit_gpu::fx::lighting::LightingShadows as S;
    // A view showing the models themselves leaves them out of the shadows.
    let plain = !crate::model::display().takes_shadows();
    let mut layers = layers
        .iter()
        .filter(|l| !(plain && matches!(l.source, DrawSource::Model(_))));
    layers
        .clone()
        .any(|l| l.three_d && l.material.casts_shadows != S::Off)
        && layers.any(|l| {
            l.material.accepts_shadows != S::Off && l.lights.iter().any(|g| g.shadow_darkness > 0.0)
        })
}

/// A layer that is there only for its shadows, After Effects' Only: one that
/// casts them without being seen, or one showing only those that fall on it
/// when no light reaches it at all.
pub(super) fn shadow_only(l: &CompLayerDraw) -> bool {
    use lumit_gpu::fx::lighting::LightingShadows as S;
    // A holdout is only hidden behind its shadows, and a view showing the
    // models themselves shows them all.
    let plain = !crate::model::display().takes_shadows();
    let seen = matches!(&l.source, DrawSource::Model(m) if m.holdout || plain);
    !seen
        && (l.material.casts_shadows == S::Only
            || (l.material.accepts_shadows == S::Only && l.lights.is_empty()))
}

/// The order a segment's layers are drawn in: as they stack, but with
/// shadows the models last, so each has the casters of the 3D layers drawn
/// before it.
pub(super) fn models_last(layers: &[CompLayerDraw], shadowed: bool) -> Vec<usize> {
    let model = |i: &usize| shadowed && matches!(layers[*i].source, DrawSource::Model(_));
    let all = 0..layers.len();
    all.clone()
        .filter(|i| !model(i))
        .chain(all.filter(model))
        .collect()
}

/// `drawn`, made in `order`, put back in the order the layers stack.
pub(super) fn unpermute<T>(drawn: Vec<T>, order: &[usize]) -> Vec<T> {
    let mut back: Vec<Option<T>> = std::iter::repeat_with(|| None).take(drawn.len()).collect();
    for (t, &i) in drawn.into_iter().zip(order) {
        if let Some(slot) = back.get_mut(i) {
            *slot = Some(t);
        }
    }
    back.into_iter().flatten().collect()
}
