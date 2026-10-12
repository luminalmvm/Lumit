//! The OCIO effects (docs/08 §3.97, docs/impl/ocio.md §6.6): the project's
//! colour config applied to one layer by hand, in the middle of a stack,
//! rather than at the edges the project settings manage.
//!
//! **In plain terms.** Colour management normally happens at the borders:
//! footage is read through its colour space, the Viewer shows through a view,
//! the export writes a named space. These four effects put the same recipes
//! on a layer where you want them - convert this layer between two of the
//! config's spaces, show it through a display and view, run a look over it,
//! or apply a LUT or CDL file straight from disk - which is the flexibility a
//! grade sometimes needs and a project setting cannot give.
//!
//! The declarations are the whole of what lives here. A name row is a
//! [`ParamKind::ColourName`](crate::fx::ParamKind::ColourName): the config's
//! own spelling, kept in the document, never in the arena. The render resolves
//! the names against the loaded config, bakes the chain once, and threads the
//! baked table beside the op exactly as a LUT's cube is threaded (docs/impl/
//! effect-registry.md §2.5a); the effect itself resolves to its Mix alone.
//!
//! There is no CPU reference, for the reason the LUT has none: the table
//! never reaches the single-buffer CPU dispatcher, so the degradation rung
//! renders these as identity. The §1.6 oracle is `lumit_colour::Artefact::eval`,
//! held against the kernel in `lumit-render`'s `ocio_parity` test.

use crate::fx::{EffectDef, EffectMetadata, EffectSchema};
use lumit_fx_macros::Effect;

/// The per cent Mix every one of these ends with, as the kernel's 0-1.
fn mix_of(mix: f32) -> f32 {
    (mix / 100.0).clamp(0.0, 1.0)
}

/// OCIO colour space transform: from one of the config's spaces to another.
/// A row left unset is the working space, so a fresh instance with only
/// Output set is an output transform and one with only Input set is an
/// input transform.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "ocio_colour_space",
    label = "OCIO colour space transform",
    version = 1,
    category = Colour,
    cost = Moderate,
    roi = Exact,
    // §2.2: a colour transform must see straight colour.
    premultiplied = false,
)]
pub struct OcioColourSpace {
    #[colour_name(role = Space)]
    pub input_colour_space: (),
    #[colour_name(role = Space)]
    pub output_colour_space: (),
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
    /// The Information row: what the loaded config calls itself, read-only.
    #[colour_name(role = Config, label = "OCIO configuration")]
    pub ocio_configuration: (),
}

impl OcioColourSpace {
    pub fn packed(self) -> f32 {
        mix_of(self.mix)
    }
}

pub struct OcioColourSpaceDef;

impl EffectDef for OcioColourSpaceDef {
    fn schema(&self) -> &'static EffectSchema {
        &<OcioColourSpace as EffectMetadata>::SCHEMA
    }
}

/// OCIO display transform: the layer shown through a display and view, from a
/// named input space. Inverse undoes a view instead, which is how a
/// display-referred picture is brought back to scene-linear.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "ocio_display",
    label = "OCIO display transform",
    version = 1,
    category = Colour,
    cost = Moderate,
    roi = Exact,
    premultiplied = false,
)]
pub struct OcioDisplay {
    #[colour_name(role = Space)]
    pub input_colour_space: (),
    #[colour_name(role = Display)]
    pub display: (),
    #[colour_name(role = View)]
    pub view: (),
    #[toggle(default = false)]
    pub inverse: bool,
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
    /// The Information row: what the loaded config calls itself, read-only.
    #[colour_name(role = Config, label = "OCIO configuration")]
    pub ocio_configuration: (),
}

impl OcioDisplay {
    pub fn packed(self) -> f32 {
        mix_of(self.mix)
    }
}

pub struct OcioDisplayDef;

impl EffectDef for OcioDisplayDef {
    fn schema(&self) -> &'static EffectSchema {
        &<OcioDisplay as EffectMetadata>::SCHEMA
    }
}

/// OCIO look transform: one of the config's looks, applied between two of its
/// spaces. Inverse runs the look backwards.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "ocio_look",
    label = "OCIO look transform",
    version = 1,
    category = Colour,
    cost = Moderate,
    roi = Exact,
    premultiplied = false,
)]
pub struct OcioLook {
    #[colour_name(role = Space)]
    pub input_colour_space: (),
    #[colour_name(role = Look)]
    pub look: (),
    #[colour_name(role = Space)]
    pub output_colour_space: (),
    #[toggle(default = false)]
    pub inverse: bool,
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
    /// The Information row: what the loaded config calls itself, read-only.
    #[colour_name(role = Config, label = "OCIO configuration")]
    pub ocio_configuration: (),
}

impl OcioLook {
    pub fn packed(self) -> f32 {
        mix_of(self.mix)
    }
}

pub struct OcioLookDef;

impl EffectDef for OcioLookDef {
    fn schema(&self) -> &'static EffectSchema {
        &<OcioLook as EffectMetadata>::SCHEMA
    }
}

/// OCIO file transform: a LUT or CDL file applied as it is, through the same
/// readers a config's own `FileTransform` uses, so it needs no config at all.
/// Tetrahedral for a cube, unlike the LUT effect's trilinear.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "ocio_file",
    label = "OCIO file transform",
    version = 1,
    category = Colour,
    cost = Moderate,
    roi = Exact,
    premultiplied = false,
)]
pub struct OcioFile {
    /// Always `None` here, as the LUT's is: the render decides the slot.
    #[file(
        filter = ["cube", "spi1d", "spi3d", "clf", "ctf", "cc", "ccc", "cdl"],
        filter_name = "LUT or CDL"
    )]
    pub file: Option<u32>,
    #[toggle(default = false)]
    pub inverse: bool,
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

impl OcioFile {
    pub fn packed(self) -> f32 {
        mix_of(self.mix)
    }
}

pub struct OcioFileDef;

impl EffectDef for OcioFileDef {
    fn schema(&self) -> &'static EffectSchema {
        &<OcioFile as EffectMetadata>::SCHEMA
    }
}
