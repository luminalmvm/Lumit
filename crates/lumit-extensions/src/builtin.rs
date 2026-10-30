//! The extensions that come with Lumit. Their effects are always there,
//! filed under Lumit's own headings, and are never in a folder, so they can't
//! be removed or go missing. An installed copy of one, registered after,
//! takes its place.
//!
//! Siceliot's picture effects and Reaizzi's Glass, by andrea68.

use crate::{lfx_effect, parse};
use lumit_core::fx::lfx::LfxEffect;

/// A shader's file name and its text, from `builtin/<extension>/`.
macro_rules! shader {
    ($extension:literal, $name:literal) => {
        (
            concat!($name, ".wgsl"),
            include_str!(concat!("../builtin/", $extension, "/", $name, ".wgsl")),
        )
    };
}

/// Each manifest, and its shaders by file name.
const BUILTIN: &[(&str, &[(&str, &str)])] = &[
    (
        include_str!("../builtin/siceliot/extension.json"),
        &[
            shader!("siceliot", "falloff"),
            shader!("siceliot", "conical-gradient"),
            shader!("siceliot", "sweep-gradient"),
            shader!("siceliot", "multi-point-gradient"),
            shader!("siceliot", "gradient-map"),
            shader!("siceliot", "voronoi"),
            shader!("siceliot", "checkerboard"),
            shader!("siceliot", "halftone"),
            shader!("siceliot", "dithering"),
            shader!("siceliot", "inner-shadow"),
            shader!("siceliot", "erosion"),
            shader!("siceliot", "light-sweep"),
            shader!("siceliot", "lighting"),
            shader!("siceliot", "stripes"),
            shader!("siceliot", "scrape"),
            shader!("siceliot", "bilateral-blur"),
            shader!("siceliot", "bend"),
            shader!("siceliot", "four-point-warp"),
        ],
    ),
    (
        include_str!("../builtin/reaizzi-glass/extension.json"),
        &[shader!("reaizzi-glass", "glass")],
    ),
];

/// The effects of the extensions that come with Lumit, each with its shader,
/// for the engine to register before any installed ones.
#[must_use]
pub fn effects() -> Vec<LfxEffect> {
    let mut all = Vec::new();
    for (manifest, shaders) in BUILTIN {
        let Ok(manifest) = parse(manifest) else {
            continue;
        };
        for effect in &manifest.effects {
            let shader = effect
                .shader
                .as_deref()
                .and_then(|file| shaders.iter().find(|(name, _)| *name == file));
            if let Some((_, text)) = shader {
                all.push(lfx_effect(&manifest.id, effect, (*text).to_owned()));
            }
        }
    }
    all
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    /// Every effect that comes with Lumit reads, finds its shader, files
    /// under one of Lumit's headings and compiles. Guards projects that use
    /// them opening with the effects missing.
    #[test]
    fn every_built_in_effect_loads_and_compiles() {
        let effects = super::effects();
        assert_eq!(effects.len(), 19);
        for effect in effects {
            lumit_core::fx::lfx::check(&effect)
                .unwrap_or_else(|why| panic!("{}: {why}", effect.id));
            let passes: Vec<&str> = effect.passes.iter().map(|p| p.name.as_str()).collect();
            let program = lumit_core::fx::shader::build_passes(&effect.shader, &passes)
                .unwrap_or_else(|why| panic!("{}: {why:?}", effect.id));
            for (_, text) in &program.passes {
                if let Err(why) = lumit_gpu::fx::custom_shader::validate(text) {
                    panic!("{}: {}", effect.id, program.remap_error(&why));
                }
            }
        }
    }
}
