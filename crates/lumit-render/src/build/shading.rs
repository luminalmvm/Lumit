//! What the lighting pass reads: the comp's lights and a layer's Material
//! options, and the placement maths that puts them in space.

use lumit_core::expression::ExpressionContext;
use std::sync::Arc;

/// The comp's Light layers, reduced to what the lighting pass needs (docs/06):
/// each light's emitting rectangle as four corners in comp pixels,
/// wound the same way for every light so the form-factor integral has a
/// consistent sign.
///
/// Returns empty — the pass's no-op — when the layer's Accepts lights switch
/// is off, when the layer is itself a light or a camera (a light does not light
/// itself, and neither draws pixels to shade), or when the comp holds no
/// lights at all. That last case is the one that matters for compatibility: a
/// project made before lighting existed has no Light layers, so this is empty
/// on every draw, so no lighting pass runs and the frame is unchanged to the
/// byte.
///
/// The nearest `MAX_LIT_LIGHTS` win when there are more, measured from the
/// layer's position — a budget rather than an error, because running out of
/// uniform slots must not make a frame fail (docs/13).
pub fn shading_lights(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: &Arc<ExpressionContext>,
) -> Vec<lumit_gpu::fx::lighting::LightingLight> {
    use lumit_core::model::LayerKind;

    // Accepts lights off still takes the shadows of the lights that cast
    // them, as in After Effects. And After Effects never lights a 2D layer.
    let unlit = !layer.switches.accepts_lights;
    let takes_shadows = layer.switches.three_d
        && layer.transform.material.accepts_shadows != lumit_core::model::ShadowMode::Off
        && comp.layers.iter().any(|l| {
            l.switches.three_d
                && l.transform.material.casts_shadows != lumit_core::model::ShadowMode::Off
        });
    if (unlit && !takes_shadows)
        || matches!(
            layer.kind,
            LayerKind::Light { .. } | LayerKind::Camera { .. }
        )
        || (comp.lighting != lumit_core::model::CompLighting::Adds && !layer.switches.three_d)
    {
        return Vec::new();
    }
    let encoded = comp.lighting == lumit_core::model::CompLighting::AfterEffects { linear: false };
    let read = lumit_core::model::context_reader(context, t_comp);
    let mut lights = comp.lights_at(t_comp, &read, crate::track::solves_for(&context.document));
    if unlit {
        lights.retain(|l| l.shadow_darkness > 0.0);
    }
    if lights.is_empty() {
        return Vec::new();
    }
    if lights.len() > lumit_gpu::fx::lighting::MAX_LIT_LIGHTS {
        let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
        let (lx, ly) = (
            layer.transform.position_x.value_at(lt),
            layer.transform.position_y.value_at(lt),
        );
        // Total order, so two runs pick the same lights: distance first, and
        // the original stacking order breaks any tie.
        let mut keyed: Vec<_> = lights
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let (dx, dy) = (l.position.0 - lx, l.position.1 - ly);
                (dx * dx + dy * dy, i)
            })
            .collect();
        keyed.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        keyed.truncate(lumit_gpu::fx::lighting::MAX_LIT_LIGHTS);
        keyed.sort_by_key(|&(_, i)| i);
        lights = keyed.iter().map(|&(_, i)| lights[i]).collect();
    }

    lights.iter().map(|l| lighting_light(l, encoded)).collect()
}

/// A 3D layer from a collapsed precomp taking `lights` beside its own, up to
/// the slots there are. Unlit, it takes only their shadows, as any layer does.
pub(crate) fn add_lights(
    d: &mut crate::draw::CompLayerDraw,
    lights: &[lumit_core::model::ResolvedLight],
) {
    let room = lumit_gpu::fx::lighting::MAX_LIT_LIGHTS.saturating_sub(d.lights.len());
    let (unlit, encoded) = (d.material.unlit, d.material.encoded);
    d.lights.extend(
        lights
            .iter()
            .filter(|l| !unlit || l.shadow_darkness > 0.0)
            .take(room)
            .map(|l| lighting_light(l, encoded)),
    );
}

/// One resolved light as the lighting kernel reads it, for a picture lit in
/// display encoding when `encoded`.
pub(crate) fn lighting_light(
    l: &lumit_core::model::ResolvedLight,
    encoded: bool,
) -> lumit_gpu::fx::lighting::LightingLight {
    use lumit_core::model::LightKind;
    // The same placement maths a layer gets, so a light and a layer
    // that share a transform occupy the same place in space.
    let place = lumit_gpu::composite::place_matrix(
        (l.position.0 as f32, l.position.1 as f32),
        (0.0, 0.0),
        (100.0, 100.0),
        l.rotation_deg as f32,
        l.z as f32,
        l.rotation_x_deg as f32,
        l.rotation_y_deg as f32,
    );
    let (hw, hh) = (l.half_size.0 as f32, l.half_size.1 as f32);
    let at = |x: f32, y: f32| place_point(&place, x, y, 0.0);
    // A spot is aimed along its own local +z, away from the camera;
    // anything below -1 tells the kernel "not a spot".
    let cone_cos = match l.kind {
        LightKind::Spot => (l.cone_deg as f32).to_radians().cos(),
        _ => -2.0,
    };
    use lumit_gpu::fx::lighting::LightingKind as K;
    lumit_gpu::fx::lighting::LightingLight {
        corners: [at(-hw, -hh), at(hw, -hh), at(hw, hh), at(-hw, hh)],
        // A picture lit in display encoding takes the light's colour
        // encoded too, and its intensity as it is, as After Effects does.
        colour: if encoded {
            l.tint.map(|c| lumit_core::lighting::encode(c) * l.gain)
        } else {
            l.colour
        },
        kind: match l.kind {
            LightKind::Area if hw > 0.0 && hh > 0.0 => K::Area,
            LightKind::Parallel => K::Parallel,
            LightKind::Ambient => K::Ambient,
            _ => K::Point,
        },
        falloff: l.falloff as u32,
        radius_px: l.radius_px as f32,
        falloff_px: l.falloff_px as f32,
        legacy: l.legacy_shape,
        cone_cos,
        // The feather is a share of the cone's angle, as After Effects
        // has it. An old light's is a share of its cosine, which at a
        // tenth is the edge spots had before it was a row.
        cone_inner: if l.legacy_shape {
            cone_cos + (1.0 - cone_cos) * (l.cone_feather / 100.0) as f32
        } else {
            (l.cone_deg * (1.0 - l.cone_feather / 100.0))
                .to_radians()
                .cos() as f32
        },
        axis: unit(place_dir(&place, 0.0, 0.0, 1.0)),
        shadow_darkness: l.shadow_darkness,
        shadow_diffusion: l.shadow_diffusion,
    }
}

/// A layer's Material options at comp time `t_comp`, as what the lighting
/// pass reads. The eye is the realiser's to fill in. A 2D layer has no
/// Material options to set, so it takes the lights as they always were, with
/// no highlight.
pub fn shading_material(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
) -> lumit_gpu::fx::lighting::LightingMaterial {
    if !layer.switches.three_d {
        return lumit_gpu::fx::lighting::LightingMaterial::default();
    }
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let m = &layer.transform.material;
    let pct = |p: &lumit_core::anim::Property| (p.value_at(lt) / 100.0).max(0.0) as f32;
    // Where light adds to the picture, Diffuse at a half is the light as it
    // always was; After Effects lights from black at the Diffuse given.
    let adds = comp.lighting == lumit_core::model::CompLighting::Adds;
    let encoded = comp.lighting == lumit_core::model::CompLighting::AfterEffects { linear: false };
    let shadows = |s: lumit_core::model::ShadowMode| match s {
        lumit_core::model::ShadowMode::Off => lumit_gpu::fx::lighting::LightingShadows::Off,
        lumit_core::model::ShadowMode::On => lumit_gpu::fx::lighting::LightingShadows::On,
        lumit_core::model::ShadowMode::Only => lumit_gpu::fx::lighting::LightingShadows::Only,
    };
    lumit_gpu::fx::lighting::LightingMaterial {
        base: if adds { 1.0 } else { 0.0 },
        ambient: pct(&m.ambient),
        diffuse: if adds { 2.0 } else { 1.0 } * pct(&m.diffuse),
        specular: pct(&m.specular),
        exponent: lumit_core::lighting::ShadingMaterial::exponent_for(m.shininess.value_at(lt)),
        metal: pct(&m.metal).min(1.0),
        eye: [0.0; 3],
        encoded,
        accepts_shadows: shadows(m.accepts_shadows),
        casts_shadows: shadows(m.casts_shadows),
        transmission: pct(&m.light_transmission).min(1.0),
        unlit: !layer.switches.accepts_lights,
        reflection: pct(&m.reflection).min(1.0),
    }
}

/// A point through a column-major placement matrix, done by hand so the render
/// crate does not take a maths dependency for nine multiplies. Placements are
/// affine — the camera's perspective is applied later — so there is no w to
/// divide by.
pub(crate) fn place_point(m: &[[f32; 4]; 4], x: f32, y: f32, z: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| m[0][k] * x + m[1][k] * y + m[2][k] * z + m[3][k])
}

/// A direction through the same matrix: the translation column is left out,
/// which is what makes it a direction rather than a position.
pub(crate) fn place_dir(m: &[[f32; 4]; 4], x: f32, y: f32, z: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| m[0][k] * x + m[1][k] * y + m[2][k] * z)
}

pub(crate) fn unit(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-6 || !len.is_finite() {
        return [0.0, 0.0, 1.0];
    }
    v.map(|c| c / len)
}

/// A casting 3D layer's plane, as its shadows read it.
pub(crate) fn caster_plane(
    tex: &wgpu::Texture,
    l: &crate::draw::CompLayerDraw,
    g: (f32, f32),
) -> lumit_gpu::fx::lighting::CasterPlane {
    let world = crate::realise::placed_in_3d(tex, l, g).world();
    lumit_gpu::fx::lighting::CasterPlane {
        origin: world.col(3).truncate().to_array(),
        u: world.col(0).truncate().to_array(),
        v: world.col(1).truncate().to_array(),
        transmission: l.material.transmission,
        opacity: (l.opacity / 100.0).clamp(0.0, 1.0),
    }
}

/// A flat 8 x 8 tile at the layer's own size, as wide as the card takes.
pub(crate) fn at_own_size(d: &mut crate::draw::CompLayerDraw) {
    let natural = (d.natural_size.0 as u32, d.natural_size.1 as u32);
    if let crate::draw::DrawSource::Pixels {
        rgba, tex_w, tex_h, ..
    } = &mut d.source
    {
        let flat = rgba.len() == 256 && rgba.chunks(4).all(|p| p == &rgba[..4]);
        if (*tex_w, *tex_h) == (8, 8) && natural != (8, 8) && flat {
            let fit = lumit_core::largest_picture::fit;
            let (w, h) = (fit(natural.0), fit(natural.1));
            let px = [rgba[0], rgba[1], rgba[2], rgba[3]];
            (*rgba, *tex_w, *tex_h) = (Arc::new(super::px_tile(&px, w, h)), w, h);
        }
    }
}
