//! Sprite flare (docs/08 §3.29, K-359): the art-directed flare — a glow on the
//! light, a train of iris ghosts through the frame's centre, and an anamorphic
//! streak. Deliberately a separate effect from the physically simulated §3.27
//! rather than a mode of it.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamGroup, Params};
use lumit_fx_macros::Effect;

/// The panel's disclosure groups. Held as a named constant because the derive
/// takes the groups as one expression, and because the same three headings read
/// better here than inlined into the attribute.
pub const SPRITE_FLARE_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Glow",
        params: &["glow_size", "glow_intensity"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Ghosts",
        params: &["ghosts", "ghost_spacing", "ghost_size", "ghost_intensity"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Streak",
        params: &["streak_length", "streak_intensity", "streak_angle"],
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// Sprite flare's controls.
///
/// Every distance is px@comp (K-260), declared `Px` so the resolve step scales
/// each by the §2.3 preview factor and
/// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::rescale_spatial)
/// moves them together if the stack is reused at another size — the light's
/// position included, or the whole flare would slide (K-266). Ghost spacing is a
/// *fraction* of the light→centre distance, not a length, so it follows nothing.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "sprite_flare",
    label = "Sprite flare",
    version = 1,
    category = Stylise,
    cost = Cheap,
    roi = FullFrame,
    groups = SPRITE_FLARE_GROUPS,
)]
pub struct SpriteFlare {
    /// px@comp (K-260), like the physical flare's light. Open both sides: an
    /// off-frame light still throws ghosts across the frame, which is most of
    /// what this effect is for.
    #[slider(min = 0.0, max = 3840.0, default = 640.0, unit = Px)]
    pub light_x: f32,

    /// See [`light_x`](Self::light_x).
    #[slider(min = 0.0, max = 2160.0, default = 360.0, unit = Px)]
    pub light_y: f32,

    /// Master gain on everything the effect draws; 0 is the neutral point.
    #[slider(min = 0.0, max = 4.0, default = 1.0, hard_min = 0.0)]
    pub intensity: f32,

    /// Scene-linear, and open above so an HDR tint can push the flare hotter
    /// than the plate.
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub tint: [f32; 4],

    /// The central glow's radius, px@comp.
    #[slider(min = 0.0, max = 800.0, default = 120.0, hard_min = 0.0, unit = Px)]
    pub glow_size: f32,

    /// The central glow's gain.
    #[slider(min = 0.0, max = 4.0, default = 1.0, hard_min = 0.0)]
    pub glow_intensity: f32,

    /// How many discs march along the axis; 0 is none of them.
    #[counter(min = 0, max = 16, default = 6, hard_min = 0, hard_max = 16)]
    pub ghosts: i32,

    /// A fraction of the light→centre distance, so the train stretches and
    /// gathers as the light moves, exactly as a real one does.
    #[slider(min = 0.0, max = 1.5, default = 0.35, hard_min = 0.0)]
    pub ghost_spacing: f32,

    /// The ghosts' base radius, px@comp.
    #[slider(min = 0.0, max = 400.0, default = 60.0, hard_min = 0.0, unit = Px)]
    pub ghost_size: f32,

    /// The ghosts' gain.
    #[slider(min = 0.0, max = 2.0, default = 0.35, hard_min = 0.0)]
    pub ghost_intensity: f32,

    /// The anamorphic streak's half-length, px@comp.
    #[slider(min = 0.0, max = 2000.0, default = 300.0, hard_min = 0.0, unit = Px)]
    pub streak_length: f32,

    /// The streak's gain.
    #[slider(min = 0.0, max = 2.0, default = 0.5, hard_min = 0.0)]
    pub streak_intensity: f32,

    /// Degrees; 0 is horizontal — the anamorphic look.
    #[slider(min = -180.0, max = 180.0, default = 0.0)]
    pub streak_angle: f32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

impl SpriteFlare {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4),
    /// floored exactly as the old resolve arm floored it: every gain and every
    /// length at zero, the tint's three channels at zero (alpha is unused — a
    /// flare respects the layer's own footprint), the ghost count rounded and
    /// capped at [`cpu::SPRITE_FLARE_MAX_GHOSTS`], and Mix a plain 0..1 fraction.
    /// The distances arrive already scaled by the §2.3 preview factor. Both
    /// render paths read this one method, so the CPU reference and the WGSL
    /// kernel cannot drift apart.
    pub fn packed(self) -> cpu::SpriteFlareParams {
        cpu::SpriteFlareParams {
            light: [self.light_x, self.light_y],
            intensity: self.intensity.max(0.0),
            tint: [
                self.tint[0].max(0.0),
                self.tint[1].max(0.0),
                self.tint[2].max(0.0),
            ],
            glow_size: self.glow_size.max(0.0),
            glow_intensity: self.glow_intensity.max(0.0),
            ghosts: self.ghosts.clamp(0, cpu::SPRITE_FLARE_MAX_GHOSTS as i32) as u32,
            ghost_spacing: self.ghost_spacing.max(0.0),
            ghost_size: self.ghost_size.max(0.0),
            ghost_intensity: self.ghost_intensity.max(0.0),
            streak_length: self.streak_length.max(0.0),
            streak_intensity: self.streak_intensity.max(0.0),
            streak_angle_deg: self.streak_angle,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Sprite flare's behaviour.
pub struct SpriteFlareDef;

impl EffectDef for SpriteFlareDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SpriteFlare as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::sprite_flare(rgba, w, h, &SpriteFlare::read(p).packed());
    }
}
