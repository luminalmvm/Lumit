//! Set channels (docs/08 §3.94): every output channel is told which channel of
//! which picture it comes from — AE's Set Channels.
//!
//! **In plain terms.** Four dropdowns, one per output channel. Each says where
//! that channel is fetched from: one of this layer's own four channels (or its
//! luminance), the same five off a **Source** layer you pick, or flat on and
//! flat off. It is how a depth pass is moved into the alpha, how a luma pass
//! becomes a red channel, and how a colour is thrown away without a Fill.
//!
//! **The Source row is this effect's own layer input**, on the ordinary
//! auxiliary-layer carriage beside Light wrap's Background and Texturize's
//! Texture (docs/impl/layer-input.md). It is not a matte: a matte answers
//! "how much of me happens here" and this row answers "where do these numbers
//! come from", so the universal Matte row (§2.6) stays beside it and does the
//! usual strength dissolve — which is what "reassign the channels, but only over
//! the sky" means here.
//!
//! **After Effects has four source layers; this has one.** The carriage carries
//! a matte and one auxiliary layer per effect, and four would need four
//! carriages. What the four buy in practice is a single question — is this
//! channel mine or somebody else's — and every instance in the reference project
//! answers it with at most one other layer. The import therefore maps that shape
//! exactly and reports anything wider rather than guessing (docs/11 §5).
//!
//! There is no CPU reference through the single-buffer dispatcher for the
//! *source*, which carries no second picture — but the effect still shuffles
//! this layer's own channels without one, so `apply_cpu` runs the oracle with an
//! empty source rather than defaulting to identity. The §1.6 oracle is
//! [`crate::fx::cpu::set_channels`], exercised with a source from the lumit-gpu
//! test, which can upload one.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Where one output channel is fetched from (docs/08 §3.94).
///
/// The five channel reads appear twice — once for this layer, once for the
/// Source row — because that pairing *is* the effect, and folding them into two
/// dropdowns per output channel would double the rows to say the same thing.
/// **Full on** and **Full off** end the list where After Effects ends its own,
/// so the import maps by position for those two.
///
/// AE's Hue, Lightness and Saturation are deliberately absent, exactly as they
/// are absent from [`CHANNEL_OPTIONS`](super::super::CHANNEL_OPTIONS): nothing
/// encodes a channel as a hue, and the import reports the collapse rather than
/// pretending otherwise.
pub const SET_CHANNELS_OPTIONS: &[&str] = &[
    "Red",
    "Green",
    "Blue",
    "Alpha",
    "Luminance",
    "Source red",
    "Source green",
    "Source blue",
    "Source alpha",
    "Source luminance",
    "Full on",
    "Full off",
];

/// Set channels' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "set_channels",
    label = "Set channels",
    version = 1,
    category = Utility,
    // One texture read a pixel, no neighbourhood.
    cost = Trivial,
    roi = Exact,
    // §2.2: the effect moves coverage and colour about independently, so it runs
    // on straight values — a premultiplied channel read as a colour would carry
    // its own alpha into the answer. The round trip is fused into the one pass,
    // as §2.2 permits.
    premultiplied = false,
)]
pub struct SetChannels {
    /// The other picture the four `Source …` picks read. Unset until the owner
    /// names one, and then every `Source …` pick reads **zero**: a picture
    /// nobody has supplied contributes nothing, which is the only reading that
    /// does not invent one. No `self_default`: this layer is already on
    /// every dropdown, under its own five names.
    #[layer(label = "Source", self_default = false)]
    pub source: bool,

    /// Where the output's red comes from. The four defaults are the identity
    /// assignment, which is After Effects' own and the only honest one: any
    /// other default would scramble a picture the moment the effect was added.
    /// It is the labelled no-op §1.2 sanctions for a layer-input effect, and it
    /// is one for the same reason — what the effect does cannot be guessed
    /// before the owner has said where a channel should come from.
    #[choice(label = "Red from", options = *SET_CHANNELS_OPTIONS, default = 0)]
    pub red_from: u32,

    /// Where the output's green comes from.
    #[choice(label = "Green from", options = *SET_CHANNELS_OPTIONS, default = 1)]
    pub green_from: u32,

    /// Where the output's blue comes from.
    #[choice(label = "Blue from", options = *SET_CHANNELS_OPTIONS, default = 2)]
    pub blue_from: u32,

    /// Where the output's alpha comes from.
    #[choice(label = "Alpha from", options = *SET_CHANNELS_OPTIONS, default = 3)]
    pub alpha_from: u32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
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

impl SetChannels {
    /// The two things both kernels consume (docs/impl/effect-registry.md §2.4):
    /// the four picks, clamped to the option list, and Mix as a fraction.
    #[must_use]
    pub fn packed(self) -> ([u32; 4], f32) {
        let last = SET_CHANNELS_OPTIONS.len() as u32 - 1;
        (
            [
                self.red_from.min(last),
                self.green_from.min(last),
                self.blue_from.min(last),
                self.alpha_from.min(last),
            ],
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Set channels' behaviour. Unlike Set matte and Texturize, this effect does
/// real work without its layer row — the four `This layer` picks are a shuffle
/// of the picture the dispatcher already has — so `apply_cpu` runs the oracle
/// with an empty source rather than keeping the identity default.
pub struct SetChannelsDef;

impl EffectDef for SetChannelsDef {
    fn schema(&self) -> &'static EffectSchema {
        &<SetChannels as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let (picks, mix) = SetChannels::read(p).packed();
        cpu::set_channels(rgba, &[], picks, mix);
    }
}
