//! The formats an export can write, and what each one can carry.

use super::{
    AudioDepth, ColourSpace, BUILT_IN_COLOUR_SPACES, EXPORT_AUDIO_RATES, UNTAGGED_COLOUR_SPACE,
};
use lumit_media::encode::BitDepth;

/// The sound-only containers an export can write (docs/06 §7.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum AudioFormat {
    /// AAC in an `.m4a` — the delivery form, same codec a video export uses.
    #[default]
    M4a,
    /// Uncompressed 16-bit PCM in a `.wav` — the master form.
    Wav,
}

impl AudioFormat {
    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::M4a => "M4A (AAC)",
            AudioFormat::Wav => "WAV (uncompressed)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::M4a => "m4a",
            AudioFormat::Wav => "wav",
        }
    }

    /// The codec this container is written with. AAC is AAC whatever depth
    /// was asked for — it stores no samples to give a width to — so the depth
    /// only picks between the two PCM widths, and a depth AAC cannot honour
    /// is refused by [`ExportSpec::check`] long before this is called.
    pub fn codec(self, depth: AudioDepth) -> lumit_media::encode::AudioCodec {
        match (self, depth) {
            (AudioFormat::M4a, _) => lumit_media::encode::AudioCodec::Aac,
            (AudioFormat::Wav, AudioDepth::Sixteen) => lumit_media::encode::AudioCodec::PcmS16,
            (AudioFormat::Wav, AudioDepth::TwentyFour) => lumit_media::encode::AudioCodec::PcmS24,
        }
    }
}

/// What the export writes: a video file, an animated GIF, one still image per
/// frame, or sound with no picture at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum ExportFormat {
    /// An `.mp4`, in the given codec.
    Video(lumit_media::encode::VideoCodec),
    /// A numbered image per frame — `shot.00001.png` beside the chosen path.
    Images(lumit_media::encode::ImageFormat),
    /// An `.m4a` or `.wav` of the comp's mix, with no video stream.
    Audio(AudioFormat),
    /// An animated `.gif`.
    Gif,
}

/// What one output format can and cannot carry (docs/06 §7.4).
///
/// In plain terms: the export dialog draws every option, but not every option
/// means anything in every file. A `.png` has no bitrate; an `.mp4` cannot hold
/// an alpha channel; a `.wav` has no picture to set a depth on. This table says
/// which is which, in one place, so the dialog and the exporter cannot disagree
/// — and so a setting that a format cannot honour is refused rather than
/// quietly ignored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FormatCaps {
    /// Carries a picture at all.
    pub video: bool,
    /// Can carry the comp's sound.
    pub audio: bool,
    /// Can carry an alpha channel (so the Channels choice means something).
    pub alpha: bool,
    /// The choice between premultiplied and straight alpha means something.
    /// A GIF's alpha is on or off, so its kept pixels are always straight.
    pub alpha_mode: bool,
    /// Dither, palette and plays apply, which only a GIF has.
    pub gif: bool,
    /// The colour depths this format can write, best last.
    pub depths: &'static [lumit_media::encode::BitDepth],
    /// The sample rates this format's audio stream can be written at, empty
    /// where it carries no sound at all.
    pub audio_rates: &'static [u32],
    /// The sample widths this format's audio stream can be written at, empty
    /// where it carries no sound at all.
    ///
    /// AAC lists **sixteen only**, and that is not a claim about AAC's
    /// precision: a lossy transform codec stores coefficients rather than
    /// samples, so it has no sample width to set. Asking an `.mp4` or an
    /// `.m4a` for twenty-four bits is therefore answered honestly — the
    /// format cannot carry the choice — rather than by accepting the setting
    /// and writing the same file either way.
    pub audio_depths: &'static [AudioDepth],
    /// A video bitrate applies (lossless formats have none to choose).
    pub bit_rate: bool,
    /// The container holds metadata.
    pub metadata: bool,
    /// The colour spaces this format's container can *state* — the nclx/`colr`
    /// box on an mp4, the cICP chunk on a PNG. A space the file cannot name is
    /// refused rather than written unlabelled, because a wide-gamut file that
    /// says nothing is read as sRGB and comes back looking wrong. Empty where
    /// the format carries no picture at all.
    pub colour_spaces: &'static [ColourSpace],
    /// The fastest the file plays, frames a second, or `None` for no limit. A
    /// GIF counts in hundredths of a second, and browsers play a frame of one
    /// hundredth as ten, so a GIF goes no faster than 50, two a frame.
    pub max_fps: Option<u32>,
}

/// Eight bits only: H.264 and HEVC are written 4:2:0 at eight bits.
const EIGHT_ONLY: &[BitDepth] = &[BitDepth::Eight];
/// The still formats and ProRes take either width. Sixteen fills a ProRes
/// file's ten bits from the deeper composite.
const EIGHT_OR_SIXTEEN: &[BitDepth] = &[BitDepth::Eight, BitDepth::Sixteen];
/// OpenEXR carries floats and nothing else — writing eight-bit codes into one
/// would be a scene file holding a picture, which is the one thing the format
/// is for not doing. Half first because it is what a render farm writes.
const EXR_DEPTHS: &[BitDepth] = &[BitDepth::Half, BitDepth::Float];
/// AAC has no sample width of its own; only the delivery default stands.
const AAC_DEPTH: &[AudioDepth] = &[AudioDepth::Sixteen];
/// Uncompressed PCM in a `.wav` carries either width.
const PCM_DEPTHS: &[AudioDepth] = &[AudioDepth::Sixteen, AudioDepth::TwentyFour];
/// What a still sequence can state. PNG carries cICP and TIFF carries nothing,
/// and one export writes one kind of file, so the honest common answer is the
/// space that needs no tag; `a_format_refuses_a_colour_space_it_cannot_state`
/// keeps this row and the exporter in step.
const STILL_COLOUR_SPACES: &[ColourSpace] = UNTAGGED_COLOUR_SPACE;

impl ExportFormat {
    /// This format's capability row.
    pub fn caps(self) -> FormatCaps {
        match self {
            // ProRes in mov: no bitrate to choose, PCM sound, and the 4444 kinds
            // hold alpha.
            ExportFormat::Video(c) if c.is_prores() => FormatCaps {
                video: true,
                audio: true,
                alpha: c.has_alpha(),
                alpha_mode: c.has_alpha(),
                gif: false,
                depths: EIGHT_OR_SIXTEEN,
                audio_rates: EXPORT_AUDIO_RATES,
                audio_depths: PCM_DEPTHS,
                bit_rate: false,
                metadata: true,
                colour_spaces: BUILT_IN_COLOUR_SPACES,
                max_fps: None,
            },
            // H.264/HEVC in mp4: 4:2:0, eight bits, no alpha, a bitrate to
            // choose, and a container that holds metadata.
            ExportFormat::Video(_) => FormatCaps {
                video: true,
                audio: true,
                alpha: false,
                alpha_mode: false,
                gif: false,
                depths: EIGHT_ONLY,
                audio_rates: EXPORT_AUDIO_RATES,
                audio_depths: AAC_DEPTH,
                bit_rate: true,
                metadata: true,
                colour_spaces: BUILT_IN_COLOUR_SPACES,
                max_fps: None,
            },
            // Stills: lossless RGBA, either depth, no sound and no bitrate.
            // The image2 muxer writes one file per frame and has nowhere to
            // put container metadata.
            ExportFormat::Images(f) => FormatCaps {
                video: true,
                audio: false,
                alpha: true,
                alpha_mode: true,
                gif: false,
                depths: if f == lumit_media::encode::ImageFormat::Exr {
                    EXR_DEPTHS
                } else {
                    EIGHT_OR_SIXTEEN
                },
                audio_rates: &[],
                audio_depths: &[],
                bit_rate: false,
                metadata: false,
                colour_spaces: STILL_COLOUR_SPACES,
                max_fps: None,
            },
            // A GIF is eight-bit sRGB with no sound, no bitrate and no tags.
            // Its alpha is one bit, but it is still alpha.
            ExportFormat::Gif => FormatCaps {
                video: true,
                audio: false,
                alpha: true,
                alpha_mode: false,
                gif: true,
                depths: EIGHT_ONLY,
                audio_rates: &[],
                audio_depths: &[],
                bit_rate: false,
                metadata: false,
                colour_spaces: UNTAGGED_COLOUR_SPACE,
                max_fps: Some(50),
            },
            ExportFormat::Audio(f) => FormatCaps {
                video: false,
                audio: true,
                alpha: false,
                alpha_mode: false,
                gif: false,
                depths: &[],
                audio_rates: EXPORT_AUDIO_RATES,
                audio_depths: match f {
                    AudioFormat::M4a => AAC_DEPTH,
                    AudioFormat::Wav => PCM_DEPTHS,
                },
                // AAC has a bitrate; PCM is exactly what it is.
                bit_rate: f == AudioFormat::M4a,
                metadata: true,
                colour_spaces: &[],
                max_fps: None,
            },
        }
    }

    /// The file extension this format writes.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Video(c) => c.extension(),
            ExportFormat::Images(f) => f.extension(),
            ExportFormat::Audio(f) => f.extension(),
            ExportFormat::Gif => "gif",
        }
    }

    /// The codec this format's sound is written with: PCM beside ProRes, as
    /// an editor expects of a master, and AAC beside H.264 and HEVC.
    pub fn audio_codec(self, depth: AudioDepth) -> lumit_media::encode::AudioCodec {
        match self {
            ExportFormat::Video(c) if c.is_prores() => AudioFormat::Wav.codec(depth),
            ExportFormat::Audio(f) => f.codec(depth),
            _ => lumit_media::encode::AudioCodec::Aac,
        }
    }
}
