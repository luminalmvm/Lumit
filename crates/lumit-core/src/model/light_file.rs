//! A Light layer as a file holds it, old keys and new.

use super::{LightDef, LightFalloff, LightKind};
use crate::anim::Property;

/// [`LightDef`] as a file holds it, old keys and new.
#[derive(serde::Deserialize)]
pub(super) struct LightDefFile {
    #[serde(default)]
    kind: LightKind,
    #[serde(default = "LightDef::white")]
    colour: [Property; 3],
    intensity_pct: Option<Property>,
    size: Option<[Property; 2]>,
    cone_angle: Option<Property>,
    /// A tenth was the fixed edge every spot had before the row.
    #[serde(default = "LightDef::ten")]
    cone_feather: Property,
    falloff: Option<LightFalloff>,
    #[serde(default = "Property::zero")]
    radius_px: Property,
    #[serde(default = "Property::zero")]
    falloff_px: Property,
    #[serde(default)]
    two_node: bool,
    #[serde(default = "LightDef::zero3")]
    point_of_interest: [Property; 3],
    #[serde(default)]
    casts_shadows: bool,
    #[serde(default = "LightDef::hundred")]
    shadow_darkness: Property,
    #[serde(default = "Property::zero")]
    shadow_diffusion: Property,
    #[serde(default)]
    legacy_shape: bool,
    #[serde(default)]
    haze: crate::haze::LightHaze,
    #[serde(default)]
    environment: crate::environment::LightEnvironment,
    // What a light held before it had rows.
    intensity: Option<Property>,
    half_size: Option<[Property; 2]>,
    cone_deg: Option<Property>,
}

impl From<LightDefFile> for LightDef {
    fn from(f: LightDefFile) -> Self {
        let falloff = f.falloff.unwrap_or_else(|| {
            // A distance used to mean a smooth fade with no radius, and none
            // meant no falloff at all.
            if f.falloff_px.is_animated() || f.falloff_px.value_at(0.0) > 0.0 {
                LightFalloff::Smooth
            } else {
                LightFalloff::None
            }
        });
        // A file with none of the new keys was written before them.
        let legacy_shape = f.legacy_shape || (f.cone_angle.is_none() && f.intensity_pct.is_none());
        Self {
            kind: f.kind,
            colour: f.colour,
            intensity: f
                .intensity_pct
                .or_else(|| f.intensity.map(|p| scaled(p, 100.0)))
                .unwrap_or_else(LightDef::hundred),
            size: f
                .size
                .or_else(|| f.half_size.map(|h| h.map(|p| scaled(p, 2.0))))
                .unwrap_or_else(LightDef::zero_pair),
            cone_angle: f
                .cone_angle
                .or_else(|| f.cone_deg.map(|p| scaled(p, 2.0)))
                .unwrap_or_else(LightDef::ninety),
            cone_feather: f.cone_feather,
            falloff,
            radius_px: f.radius_px,
            falloff_px: f.falloff_px,
            two_node: f.two_node,
            point_of_interest: f.point_of_interest,
            casts_shadows: f.casts_shadows,
            shadow_darkness: f.shadow_darkness,
            shadow_diffusion: f.shadow_diffusion,
            legacy_shape,
            haze: f.haze,
            environment: f.environment,
        }
    }
}

/// Every value of `p` times `by`, keys and their speeds with them.
fn scaled(mut p: Property, by: f64) -> Property {
    use crate::anim::{Animation, SideInterp};
    let side = |s: SideInterp| match s {
        SideInterp::Bezier { speed, influence } => SideInterp::Bezier {
            speed: speed * by,
            influence,
        },
        other => other,
    };
    match &mut p.animation {
        Animation::Static(v) => *v *= by,
        Animation::Keyframed(keys) => {
            for k in keys {
                k.value *= by;
                k.interp_in = side(k.interp_in);
                k.interp_out = side(k.interp_out);
            }
        }
        Animation::Expression(_) => {}
    }
    p
}
