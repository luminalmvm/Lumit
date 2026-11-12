//! What each export format can carry, as the dialogue reads it.

use crate::api::export::BridgeFormatCaps;

/// What one format can carry, as the dialogue reads it. A build with no encoder
/// can carry nothing, which disables every control rather than offering choices
/// no file will honour.
#[cfg(feature = "media")]
pub(crate) fn format_caps(codec: &str) -> BridgeFormatCaps {
    use lumit_media::encode::BitDepth;
    let Ok(format) = super::export_format(codec) else {
        return BridgeFormatCaps::default();
    };
    #[cfg(target_arch = "wasm32")]
    if !super::web::offers(codec) {
        return BridgeFormatCaps::default();
    }
    let caps = format.caps();
    // The page writes no metadata and states no colour space.
    let web = cfg!(target_arch = "wasm32");
    BridgeFormatCaps {
        video: caps.video,
        audio: caps.audio,
        alpha: caps.alpha,
        alpha_mode: caps.alpha_mode,
        gif: caps.gif,
        depths: caps
            .depths
            .iter()
            .map(|d| match d {
                BitDepth::Eight => 8,
                // Half and Sixteen are both "16" to the dialogue, and never
                // both on offer: OpenEXR carries the float depths and nothing
                // else, every other still format the integer ones. So the
                // number says which within a format, which is the only place
                // it is read.
                BitDepth::Sixteen | BitDepth::Half => 16,
                BitDepth::Float => 32,
            })
            .collect(),
        bit_rate: caps.bit_rate,
        // Uncompressed PCM is exactly what it is; AAC has a rate to choose.
        audio_bit_rate: caps.audio
            && format.audio_codec(lumit_render::export::AudioDepth::Sixteen)
                == lumit_media::encode::AudioCodec::Aac,
        audio_24_bit: caps
            .audio_depths
            .contains(&lumit_render::export::AudioDepth::TwentyFour),
        metadata: caps.metadata && !web,
        colour_spaces: caps
            .colour_spaces
            .iter()
            .take(if web { 1 } else { usize::MAX })
            .map(lumit_render::export::ColourSpace::stored_name)
            .collect(),
        max_fps: caps.max_fps.unwrap_or(0),
        // The page writes no subtitles either.
        subtitles: caps.subtitles && !web,
    }
}

#[cfg(not(feature = "media"))]
pub(crate) fn format_caps(_codec: &str) -> BridgeFormatCaps {
    BridgeFormatCaps::default()
}
