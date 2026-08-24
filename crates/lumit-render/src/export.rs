//! Export (docs/06-RENDER-PIPELINE.md §7): render every work-area frame
//! through the compositor at full resolution and encode to H.264/mp4.
//!
//! In plain terms: the same pixels the Viewer shows, written to a file — the
//! preview-equals-export promise (K-031) holds because this path reuses the
//! identical colour engine and compositor. Precomp layers render recursively:
//! the nested comp becomes a texture the parent composites like any other
//! source. Runs on its own thread with its own decoders (K-017); progress
//! streams back; cancel is checked every frame.

use lumit_core::model::{Document, ProjectItem};
pub use lumit_core::pixels::{px_tile, solid_rgba, srgb_decode, srgb_encode};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use uuid::Uuid;

pub enum ExportEvent {
    /// Which encoder the ladder settled on ("NVENC", "software x264", …),
    /// sent once the file is open.
    Encoder(&'static str),
    Progress {
        frame: usize,
        total: usize,
    },
    Done(PathBuf),
    Failed(String),
}

pub struct ExportHandle {
    pub events: Receiver<ExportEvent>,
    cancel: Arc<AtomicBool>,
}

impl ExportHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Everything the export thread needs about one footage item.
#[derive(Clone)]
pub struct ItemInfo {
    pub path: PathBuf,
    pub fps: f64,
    pub frames: usize,
    /// The file could not be found (docs/07 §3.3): render the test-bar slate
    /// at this size rather than decoding. `Some((w, h))` carries the comp's
    /// dimensions — the preview sizes a missing layer the same way, since a
    /// file we cannot open has no size of its own, and the two must agree or
    /// the layer's geometry would differ between them. Export must match the
    /// preview (K-031): an export that quietly dropped a missing layer to
    /// black while the Viewer showed bars would hide the mistake in the
    /// delivered file, which is precisely what the slate prevents.
    pub missing: Option<(u32, u32)>,
}

/// One audio-bearing layer, as the export thread needs it: where its file
/// is, its comp-timeline span, its start offset, and its Volume (the same
/// set the preview mix uses, so export audio matches playback).
#[derive(Clone, PartialEq)]
pub struct AudioJob {
    /// The footage item the audio comes from — the key the preview path uses
    /// to reuse an already-decoded buffer instead of re-decoding the file.
    pub item: uuid::Uuid,
    pub path: PathBuf,
    pub in_s: f64,
    pub out_s: f64,
    pub offset_s: f64,
    /// The layer's Volume property (dB, docs/09 §6): static values become a
    /// constant gain; keyframed ones bake to a control-rate envelope.
    pub volume: lumit_core::anim::Property,
    /// Enclosing Precomp layers' Volumes (outermost first), each with the
    /// outer-comp time where its layer time 0 sits — a precomp's Volume
    /// scales everything inside it, so the gains multiply through the chain.
    pub carriers: Vec<(lumit_core::anim::Property, f64)>,
}

/// Bake one job's Volume — its own dB property times every carrier's — for
/// its placed span: `(constant gain, envelope)`. All-static chains are
/// exactly their constant product (envelope None); any animated link bakes
/// the whole chain to a ~10 ms control-rate curve, each property sampled in
/// its own layer time (`lt = comp time − its offset`).
pub fn volume_bake(
    job: &AudioJob,
    start_frame: i64,
    len: usize,
    rate: u32,
) -> (f32, Option<lumit_audio::mix::GainEnvelope>) {
    let gain_at = |t: f64| {
        let mut g = lumit_audio::mix::db_to_gain(job.volume.value_at(t - job.offset_s));
        for (prop, off) in &job.carriers {
            g *= lumit_audio::mix::db_to_gain(prop.value_at(t - off));
        }
        g
    };
    let animated = job.volume.is_animated() || job.carriers.iter().any(|(p, _)| p.is_animated());
    if !animated {
        return (gain_at(0.0), None);
    }
    let stride = (rate / 100).max(1);
    let n = len / stride as usize + 2;
    let points = (0..n)
        .map(|p| {
            let t = (start_frame + p as i64 * i64::from(stride)) as f64 / f64::from(rate);
            gain_at(t)
        })
        .collect();
    (1.0, Some(lumit_audio::mix::GainEnvelope { stride, points }))
}

/// Delivery presets (docs/06-RENDER-PIPELINE.md §7.5): frame, codec, and
/// bitrates as data, not code. Custom keeps the comp's own size and the
/// dialogue's choices; it is also the default (Settings → Export, K-119),
/// matching the implicit behaviour every "Export…" action had before that
/// setting existed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum ExportPreset {
    #[default]
    Custom,
    Youtube1080p60,
    Youtube1440p60,
    Youtube4k60,
    Vertical1080p60,
}

/// The parameter row one preset stamps into the export dialogue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PresetParams {
    pub size: (u32, u32),
    pub codec: lumit_media::encode::VideoCodec,
    /// VBR average target, bits/second.
    pub target_bps: i64,
    /// VBR peak, bits/second.
    pub peak_bps: i64,
}

/// Audio on all delivery presets: AAC 320 kbps, 48 kHz (docs/06 §7.5).
pub const PRESET_AUDIO_BPS: i64 = 320_000;
/// Export audio sample rate (docs/06 §7.5: 48 kHz on delivery presets).
pub const EXPORT_AUDIO_RATE: u32 = 48_000;

impl ExportPreset {
    pub const ALL: [ExportPreset; 5] = [
        ExportPreset::Custom,
        ExportPreset::Youtube1080p60,
        ExportPreset::Youtube1440p60,
        ExportPreset::Youtube4k60,
        ExportPreset::Vertical1080p60,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportPreset::Custom => "Custom (comp size)",
            ExportPreset::Youtube1080p60 => "YouTube 1080p60",
            ExportPreset::Youtube1440p60 => "YouTube 1440p60",
            ExportPreset::Youtube4k60 => "YouTube 4K60",
            ExportPreset::Vertical1080p60 => "Vertical 1080×1920p60",
        }
    }

    /// The parameters this preset stamps; None for Custom (the dialogue's
    /// own fields apply).
    pub fn params(self) -> Option<PresetParams> {
        use lumit_media::encode::VideoCodec;
        match self {
            ExportPreset::Custom => None,
            // H.264 high, VBR 16 target / 24 peak (docs/06 §7.5).
            ExportPreset::Youtube1080p60 => Some(PresetParams {
                size: (1920, 1080),
                codec: VideoCodec::H264,
                target_bps: 16_000_000,
                peak_bps: 24_000_000,
            }),
            // HEVC (H.264 fallback), VBR 25 target / 35 peak — YouTube's
            // 1440p60 band (docs/06 §7.5).
            ExportPreset::Youtube1440p60 => Some(PresetParams {
                size: (2560, 1440),
                codec: VideoCodec::Hevc,
                target_bps: 25_000_000,
                peak_bps: 35_000_000,
            }),
            // HEVC (the ladder falls back to x265 when no hardware offers
            // it), VBR 45 target / 60 peak — YouTube's 2160p60 band.
            ExportPreset::Youtube4k60 => Some(PresetParams {
                size: (3840, 2160),
                codec: VideoCodec::Hevc,
                target_bps: 45_000_000,
                peak_bps: 60_000_000,
            }),
            // The vertical variant of the 1080p60 preset (docs/06 §7.5).
            ExportPreset::Vertical1080p60 => Some(PresetParams {
                size: (1080, 1920),
                codec: VideoCodec::H264,
                target_bps: 16_000_000,
                peak_bps: 24_000_000,
            }),
        }
    }

    /// Suggested file name for the save dialogue.
    pub fn default_file_name(self) -> &'static str {
        match self {
            ExportPreset::Custom => "export.mp4",
            ExportPreset::Youtube1080p60 => "youtube-1080p60.mp4",
            ExportPreset::Youtube1440p60 => "youtube-1440p60.mp4",
            ExportPreset::Youtube4k60 => "youtube-4k60.mp4",
            ExportPreset::Vertical1080p60 => "vertical-1080x1920.mp4",
        }
    }
}

/// The sample rates an export can write. Three, and only three: the CD rate,
/// the delivery rate every preset uses, and the high-resolution master rate.
/// A rate outside this list is refused rather than nudged to the nearest one
/// — an export that quietly wrote 48 kHz when 44.1 was asked for would be a
/// file that is not what it says it is.
pub const EXPORT_AUDIO_RATES: &[u32] = &[44_100, EXPORT_AUDIO_RATE, 96_000];

/// How many bits one written sample carries.
///
/// In plain terms: this is the *file's* resolution, not the mix's. Lumit mixes
/// in 32-bit floats whatever is chosen here; the depth decides how finely that
/// mix is written down. It means something only for the uncompressed forms —
/// a lossy codec stores coefficients, not samples, and has no sample width at
/// all (see [`FormatCaps::audio_depths`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum AudioDepth {
    /// Sixteen bits a sample — CD resolution, and what every delivery file
    /// has always carried.
    #[default]
    Sixteen,
    /// Twenty-four bits a sample — the master resolution, where the extra
    /// headroom is wanted for further work.
    TwentyFour,
}

impl AudioDepth {
    pub const ALL: [AudioDepth; 2] = [AudioDepth::Sixteen, AudioDepth::TwentyFour];

    pub fn bits(self) -> u32 {
        match self {
            AudioDepth::Sixteen => 16,
            AudioDepth::TwentyFour => 24,
        }
    }
}

/// How many channels the written file carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum AudioLayout {
    /// One channel: the comp's stereo mix folded down (see
    /// [`lumit_audio::mix::downmix_to_mono`], which states the law).
    Mono,
    /// Two channels — the comp's mix as it is mixed and as it is played back.
    #[default]
    Stereo,
}

impl AudioLayout {
    pub const ALL: [AudioLayout; 2] = [AudioLayout::Mono, AudioLayout::Stereo];

    /// The interleave width every buffer downstream of the fold-down uses.
    pub fn channels(self) -> u16 {
        match self {
            AudioLayout::Mono => 1,
            AudioLayout::Stereo => 2,
        }
    }
}

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

/// What the export writes: a video file, one still image per frame (K-201), or
/// sound with no picture at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum ExportFormat {
    /// An `.mp4`, in the given codec.
    Video(lumit_media::encode::VideoCodec),
    /// A numbered image per frame — `shot.00001.png` beside the chosen path.
    Images(lumit_media::encode::ImageFormat),
    /// An `.m4a` or `.wav` of the comp's mix, with no video stream.
    Audio(AudioFormat),
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
}

use lumit_media::encode::BitDepth;

/// Eight bits only — every video codec Lumit writes in v1 (docs/06 §7.4:
/// ProRes and DNxHR, which is where 4444 and deeper live, are not in v1).
const EIGHT_ONLY: &[BitDepth] = &[BitDepth::Eight];
/// The still formats carry either width.
const EIGHT_OR_SIXTEEN: &[BitDepth] = &[BitDepth::Eight, BitDepth::Sixteen];
/// AAC has no sample width of its own; only the delivery default stands.
const AAC_DEPTH: &[AudioDepth] = &[AudioDepth::Sixteen];
/// Uncompressed PCM in a `.wav` carries either width.
const PCM_DEPTHS: &[AudioDepth] = &[AudioDepth::Sixteen, AudioDepth::TwentyFour];

impl ExportFormat {
    /// This format's capability row.
    pub fn caps(self) -> FormatCaps {
        match self {
            // H.264/HEVC in mp4: 4:2:0, eight bits, no alpha, a bitrate to
            // choose, and a container that holds metadata.
            ExportFormat::Video(_) => FormatCaps {
                video: true,
                audio: true,
                alpha: false,
                depths: EIGHT_ONLY,
                audio_rates: EXPORT_AUDIO_RATES,
                audio_depths: AAC_DEPTH,
                bit_rate: true,
                metadata: true,
            },
            // Stills: lossless RGBA, either depth, no sound and no bitrate.
            // The image2 muxer writes one file per frame and has nowhere to
            // put container metadata.
            ExportFormat::Images(_) => FormatCaps {
                video: true,
                audio: false,
                alpha: true,
                depths: EIGHT_OR_SIXTEEN,
                audio_rates: &[],
                audio_depths: &[],
                bit_rate: false,
                metadata: false,
            },
            ExportFormat::Audio(f) => FormatCaps {
                video: false,
                audio: true,
                alpha: false,
                depths: &[],
                audio_rates: EXPORT_AUDIO_RATES,
                audio_depths: match f {
                    AudioFormat::M4a => AAC_DEPTH,
                    AudioFormat::Wav => PCM_DEPTHS,
                },
                // AAC has a bitrate; PCM is exactly what it is.
                bit_rate: f == AudioFormat::M4a,
                metadata: true,
            },
        }
    }

    /// The file extension this format writes.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Video(_) => "mp4",
            ExportFormat::Images(f) => f.extension(),
            ExportFormat::Audio(f) => f.extension(),
        }
    }
}

/// Which channels the written file carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Channels {
    /// Colour only — the alpha channel is written opaque, so what a viewer
    /// sees is the comp over its own background.
    #[default]
    Rgb,
    /// Colour and the composite's own coverage, for a file that will be
    /// layered over something else.
    RgbAlpha,
}

/// How the colour channels relate to the alpha channel in the written file
/// (docs/06 §3.4). The compositor works premultiplied throughout, so
/// premultiplied is a pass-through and straight is a division.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum AlphaMode {
    /// Colour already multiplied by coverage — the working form, and what
    /// most compositors expect back.
    #[default]
    Premultiplied,
    /// Colour un-multiplied, full-strength wherever there is any coverage at
    /// all. What paint programs and some delivery specs ask for.
    Straight,
}

/// The colour space the file is written in — the export's final transform
/// (docs/06 §7.4).
///
/// In plain terms: the compositor works in scene-linear light, which is not
/// what a file should contain. Something has to convert on the way out, and
/// this says what to convert *to*. One transform exists today; the named
/// variant is the shape an OCIO config's output spaces will arrive in, so the
/// setting does not have to change when they do.
#[derive(Clone, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum ColourSpace {
    /// The built-in transform: working space → sRGB / Rec.709, which is the
    /// same display encode the Viewer shows (K-031).
    #[default]
    SrgbRec709,
    /// A named output space from an OCIO config (docs/06 §2, post-v1). Kept in
    /// the model so a project written today names its space the same way it
    /// will then; an export that asks for one before OCIO exists is refused,
    /// because a wrong colour space in a delivered file is worse than an
    /// export that did not run.
    Ocio(String),
}

impl ColourSpace {
    /// Whether this build can actually perform the transform.
    pub fn is_available(&self) -> bool {
        matches!(self, ColourSpace::SrgbRec709)
    }

    pub fn label(&self) -> String {
        match self {
            ColourSpace::SrgbRec709 => "sRGB / Rec.709".to_owned(),
            ColourSpace::Ocio(name) => name.clone(),
        }
    }
}

/// A crop applied on the way out, as pixel insets from each edge of the
/// composition (K-419: distances are pixels at composition size, never a
/// percentage). `Crop::NONE` is no crop.
///
/// In plain terms: the four numbers are how much to take off the top, the
/// left, the bottom and the right — the reading the export drawing shows as
/// `T · L · B · R`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Crop {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

impl Crop {
    pub const NONE: Crop = Crop {
        top: 0,
        left: 0,
        bottom: 0,
        right: 0,
    };

    pub fn is_none(self) -> bool {
        self == Crop::NONE
    }

    /// The size a `w`×`h` frame becomes. Insets that meet or cross are
    /// clamped so at least one pixel survives — a crop that asked for nothing
    /// is a slip of the fingers, not a reason to fail an export.
    pub fn output_size(self, w: u32, h: u32) -> (u32, u32) {
        let out_w = w
            .saturating_sub(self.left.saturating_add(self.right))
            .max(1);
        let out_h = h
            .saturating_sub(self.top.saturating_add(self.bottom))
            .max(1);
        (out_w.min(w.max(1)), out_h.min(h.max(1)))
    }

    /// The window this crop keeps, as `(x, y, width, height)` in source
    /// pixels — clamped to the frame the same way [`Self::output_size`] is, so
    /// the two can never disagree about which pixels are copied.
    pub fn window(self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let (out_w, out_h) = self.output_size(w, h);
        let x = self.left.min(w.saturating_sub(out_w));
        let y = self.top.min(h.saturating_sub(out_h));
        (x, y, out_w, out_h)
    }

    /// Copy the kept window out of a tightly-packed frame of `bytes_per_px`.
    /// A buffer too small for the frame it claims to be comes back unchanged,
    /// which is the calm answer: no panic, and a caller bug shows as a
    /// full-size frame rather than a crash mid-export.
    pub fn apply(self, frame: &[u8], w: u32, h: u32, bytes_per_px: usize) -> Vec<u8> {
        let (x, y, out_w, out_h) = self.window(w, h);
        let src_row = (w as usize).saturating_mul(bytes_per_px);
        // Nothing to crop, an empty frame, or a buffer smaller than the frame
        // it claims to be: hand it back whole. No panics in engine crates
        // (docs/14 §4), and a caller bug must show as a full-size frame rather
        // than a crash halfway through an export.
        if self.is_none() || w == 0 || h == 0 || frame.len() < src_row.saturating_mul(h as usize) {
            return frame.to_vec();
        }
        let dst_row = (out_w as usize) * bytes_per_px;
        let skip = (x as usize) * bytes_per_px;
        let mut out = Vec::with_capacity(dst_row * out_h as usize);
        for row in 0..out_h as usize {
            let start = (y as usize + row) * src_row + skip;
            out.extend_from_slice(&frame[start..start + dst_row]);
        }
        out
    }

    /// The crop equivalent to the Viewer's region of interest — the rectangle
    /// the user swept on the picture, which crosses every boundary as
    /// fractions `[x0, y0, x1, y1]` rather than pixels (K-362: which pixel a
    /// point is depends on the raster, and the raster changes with the preview
    /// resolution).
    ///
    /// Degenerate input answers no crop, exactly as a degenerate region clears
    /// the region: a drag that ended where it began is a gesture, not an
    /// error.
    pub fn from_region(region: [f64; 4], w: u32, h: u32) -> Crop {
        let [x0, y0, x1, y1] = region;
        if !region.iter().all(|v| v.is_finite()) || x1 <= x0 || y1 <= y0 {
            return Crop::NONE;
        }
        let px = |v: f64, size: u32| (v.clamp(0.0, 1.0) * f64::from(size)).round() as u32;
        let (l, t, r, b) = (px(x0, w), px(y0, h), px(x1, w), px(y1, h));
        Crop {
            top: t,
            left: l,
            bottom: h.saturating_sub(b),
            right: w.saturating_sub(r),
        }
    }
}

/// The video bitrate, chosen or worked out (docs/06 §7.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Bitrate {
    /// Work a high default out from the resolution and the rate — what the
    /// dialog's *Auto* face means.
    #[default]
    Auto,
    /// Set no bitrate at all and let the encoder pick its own quality. What a
    /// blank bitrate field has always meant (K-119), kept as its own answer
    /// rather than folded into `Auto`, because the two produce different
    /// files and a preset saved under one must not silently become the other.
    EncoderDefault,
    /// The number the user typed, in bits per second, with an optional VBR
    /// peak (None takes the 1.5× fallback the resolver has always used).
    Manual {
        target_bps: i64,
        peak_bps: Option<i64>,
    },
}

/// Bits per pixel per second a codec needs for a delivery-quality picture —
/// the constants behind [`Bitrate::Auto`]. Taken from the preset table
/// (docs/06 §7.5): 1920×1080 at 60 wants 16 Mbps of H.264, which is 0.13 bits
/// per pixel, and HEVC buys roughly a quarter off that.
const H264_BITS_PER_PIXEL: f64 = 0.13;
const HEVC_BITS_PER_PIXEL: f64 = 0.10;

/// The VBR peak as a multiple of the target, when no peak was given — the
/// same 1.5× the spec resolver has always fallen back to.
pub const PEAK_MULTIPLE: f64 = 1.5;

/// A high-quality bitrate for `w`×`h` at `fps`, as `(target, peak)` in bits
/// per second, rounded to a whole megabit and clamped to something a file can
/// actually hold.
///
/// In plain terms: more pixels and more frames need more bits, in proportion.
/// This is deliberately a straight line rather than a curve fitted to the
/// preset table — a preset stamps its own exact numbers, and *Auto* only has
/// to be a good default for a size no preset covers.
pub fn auto_bitrate(
    w: u32,
    h: u32,
    fps: f64,
    codec: lumit_media::encode::VideoCodec,
) -> (i64, i64) {
    let bits_per_px = match codec {
        lumit_media::encode::VideoCodec::H264 => H264_BITS_PER_PIXEL,
        lumit_media::encode::VideoCodec::Hevc => HEVC_BITS_PER_PIXEL,
    };
    let pixels_per_second = f64::from(w) * f64::from(h) * fps.clamp(1.0, 1000.0);
    let mbps = (pixels_per_second * bits_per_px / 1e6)
        .round()
        .clamp(1.0, 400.0);
    let target = (mbps as i64) * 1_000_000;
    let peak = ((target as f64) * PEAK_MULTIPLE).round() as i64;
    (target, peak)
}

/// Whether the export reads and writes the disk frame cache while it runs.
///
/// In plain terms: the cache of already-rendered frames on disk speeds up
/// scrubbing, but an export is a single pass through the timeline — it would
/// fill the cache with frames nobody is going to ask for again, evicting the
/// ones the user *is* working with. Off is the honest default.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum DiskCachePolicy {
    /// Neither read nor written — the export renderer's own state today.
    #[default]
    Off,
    /// Read frames already banked, but bank nothing new.
    ReadOnly,
}

/// What the export does to the composition on the way through — the render
/// settings the export drawing puts beside the output format.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RenderOptions {
    /// The resolution/quality tier, the same one the preview uses
    /// (docs/01-GLOSSARY.md §5: Full, Half, Third, Quarter). Full is what an
    /// export wants and what it gets unless something else is asked for.
    pub quality: crate::plan::Quality,
    pub disk_cache: DiskCachePolicy,
    /// Run each layer's effect stack. Off exports the layers unaffected —
    /// the export-time twin of the per-layer fx switch (docs/08 §1.5).
    pub effects: bool,
    /// Honour solo switches (K-105). Off exports every visible layer even
    /// when one is soloed for working on — an export of a soloed comp is
    /// almost never what was wanted, but it must be *askable*, not assumed.
    pub honour_solo: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            quality: crate::plan::Quality::default(),
            disk_cache: DiskCachePolicy::default(),
            effects: true,
            honour_solo: true,
        }
    }
}

impl RenderOptions {
    /// Whether these options change anything about the document at all.
    pub fn changes_document(&self) -> bool {
        !self.effects || !self.honour_solo
    }
}

/// Apply the document-shaped render options to the export's own snapshot —
/// effects off clears every layer's fx switch, solo ignored clears every solo
/// switch, and both apply through nested comps, because "no effects" means no
/// effects anywhere in this export.
///
/// Returns `None` when nothing would change, so the common export keeps the
/// snapshot it was given rather than cloning a whole document to alter
/// nothing. The copy is thrown away when the export finishes and never
/// reaches the project (docs/06 §7.2: baking is invisible).
pub fn apply_render_overrides(doc: &Arc<Document>, opts: &RenderOptions) -> Option<Arc<Document>> {
    if !opts.changes_document() {
        return None;
    }
    let mut copy = Document::clone(doc);
    for item in &mut copy.items {
        let ProjectItem::Composition(comp) = item else {
            continue;
        };
        for layer in &mut comp.layers {
            if !opts.effects {
                layer.switches.fx = false;
            }
            if !opts.honour_solo {
                layer.switches.solo = false;
            }
        }
    }
    Some(Arc::new(copy))
}

/// What happens the moment an export finishes (docs/07 §11's *When done*).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum WhenDone {
    #[default]
    Nothing,
    /// Play a short sound, so a long export can be left to run.
    MakeANoise,
    /// Show the finished file in the file browser.
    OpenFolder,
}

/// Where the completion sound lives, if it is there at all: beside the
/// executable first (a shipped build's own copy), then the application's data
/// directory (a user's own). `None` when neither exists — the hook is silent
/// rather than faulty when no sound has been supplied.
pub fn done_sound_path() -> Option<PathBuf> {
    let beside_exe = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|dir| dir.join("sounds").join(lumit_project::EXPORT_DONE_SOUND))
    });
    [beside_exe, lumit_project::export_done_sound_path()]
        .into_iter()
        .flatten()
        .find(|p| p.is_file())
}

/// Play the completion sound, if there is one. Answers whether there was a
/// sound to play, and is silent — never an error — when the file is absent,
/// cannot be decoded, or there is no audio device to play it on: a missing
/// ding must never make a finished export look failed.
///
/// Returns at once. Everything happens on a thread of its own, engine
/// included: the audio device's stream handle cannot cross threads, so it is
/// born and dies on the one that keeps it alive for the sound's length.
pub fn play_done_sound() -> bool {
    let Some(path) = done_sound_path() else {
        return false;
    };
    std::thread::spawn(move || {
        let Ok(engine) = lumit_audio::AudioEngine::new() else {
            return;
        };
        let Ok(buffer) = lumit_media::audio::decode_all(&path, engine.device_rate()) else {
            return;
        };
        // A ding, not a track: ten seconds is generous and stops a wrongly
        // supplied file from holding a thread open all afternoon.
        let seconds = buffer.duration_seconds().clamp(0.0, 10.0);
        engine.load(Arc::new(buffer));
        engine.play();
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds + 0.25));
    });
    true
}

/// The pack stage: the finished display frame turned into the exact bytes the
/// encoder is fed (docs/06 §7.4).
///
/// In plain terms: the compositor's answer is one 8-bit RGBA pixel per pixel,
/// premultiplied and already display-encoded. What a file wants may be
/// narrower (no alpha), differently related (straight alpha) or wider (16 bits
/// a channel). This is where that conversion happens, on the CPU, once per
/// frame, and it is pure — which is why it is the one part of colour handling
/// that can be tested without a graphics card.
///
// ponytail: the input is the 8-bit display read-back, so a 16-bit file today
// carries an exactly-widened 8-bit signal (v × 257 — every 8-bit value maps to
// its own 16-bit one and back without loss, so nothing is invented). The extra
// bits start meaning something the moment the display read-back widens; the
// upgrade path is a 16-bit display target in lumit-gpu and a `readback16`
// beside `readback8`, and nothing else here changes. docs/TODO.md carries it.
pub fn pack_frame(rgba8: &[u8], channels: Channels, alpha: AlphaMode, depth: BitDepth) -> Vec<u8> {
    let straight = channels == Channels::RgbAlpha && alpha == AlphaMode::Straight;
    let opaque = channels == Channels::Rgb;
    let mut out = Vec::with_capacity(rgba8.len() * depth.bytes_per_channel());
    for px in rgba8.chunks_exact(4) {
        let a = px[3];
        let mut rgba = [px[0], px[1], px[2], a];
        if opaque {
            rgba[3] = 255;
        } else if straight && a > 0 && a < 255 {
            // Un-multiply: colour back to full strength wherever there is any
            // coverage. Rounded, and clamped because a premultiplied pixel
            // whose colour exceeds its own coverage (an additive blend can
            // make one) would divide past full scale.
            for c in &mut rgba[..3] {
                *c = ((f32::from(*c) * 255.0 / f32::from(a)).round()).clamp(0.0, 255.0) as u8;
            }
        } else if straight && a == 0 {
            // No coverage: no colour to recover. Zero rather than a division
            // that has no answer.
            rgba[..3].fill(0);
        }
        match depth {
            BitDepth::Eight => out.extend_from_slice(&rgba),
            // × 257 maps 0→0 and 255→65535 exactly, so the widening is
            // reversible and adds nothing that was not there.
            BitDepth::Sixteen => {
                for c in rgba {
                    out.extend_from_slice(&(u16::from(c) * 257).to_le_bytes());
                }
            }
        }
    }
    out
}

/// The crop an export actually applies, from the dialog's two faces: the
/// explicit `T · L · B · R`, or the Viewer's region of interest when *use
/// region of interest* is ticked and a region is set.
///
/// The region wins when it is asked for and exists; otherwise the typed crop
/// stands. A region that is not four finite, increasing fractions is no region
/// (K-362), and answers the typed crop rather than nothing.
pub fn crop_for(
    explicit: Crop,
    use_region: bool,
    region: Option<[f64; 4]>,
    w: u32,
    h: u32,
) -> Crop {
    match (use_region, region) {
        (true, Some(r)) => {
            let from_region = Crop::from_region(r, w, h);
            if from_region.is_none() {
                explicit
            } else {
                from_region
            }
        }
        _ => explicit,
    }
}

/// Everything one queued export needs beyond the document snapshot: the
/// format, resolved output size, rates, range, what the picture carries and
/// what happens when it finishes.
///
/// Every field carries `serde`'s default when a stored preset does not name it
/// (`#[serde(default)]`), so a preset saved by an older Lumit still loads and
/// simply takes today's default for whatever it had never heard of.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExportSpec {
    pub format: ExportFormat,
    /// The delivery frame; None = the composition's own size, exactly as
    /// `fps: None` means the composition's own rate.
    pub target: Option<(u32, u32)>,
    /// The video bitrate: worked out from the size and rate, or the number
    /// that was typed. Meaningless — and unread — for the lossless formats.
    pub bitrate: Bitrate,
    /// Output frame rate; None = the composition's own (K-201). A different
    /// rate resamples by nearest comp frame — the honest thing without optical
    /// flow in the export path — and the file is stamped with the chosen rate.
    pub fps: Option<f64>,
    /// The export range in comp frames, end exclusive; None = the work area
    /// when one is set, else the whole comp (the standing K-037 behaviour).
    pub range: Option<(usize, usize)>,
    pub include_audio: bool,
    pub audio_bit_rate: i64,
    /// The rate the mix is resampled to and the file is written at, in Hz.
    /// Every source is decoded straight to this rate through the same
    /// resampler the preview mix uses, so no second sampling step exists to
    /// disagree with the first.
    pub audio_rate: u32,
    /// Bits per written sample. Meaningful only where the format has samples
    /// to give a width to ([`FormatCaps::audio_depths`]).
    pub audio_depth: AudioDepth,
    /// One channel or two. Mono folds the comp's stereo mix down; the law is
    /// stated on [`lumit_audio::mix::downmix_to_mono`].
    pub audio_layout: AudioLayout,
    /// Bits per channel in the written file.
    pub depth: BitDepth,
    pub channels: Channels,
    pub alpha: AlphaMode,
    pub colour_space: ColourSpace,
    /// Pixels taken off each edge on the way out, already resolved from the
    /// region of interest where that was asked for ([`crop_for`]).
    pub crop: Crop,
    /// What is written into the container about the file.
    pub metadata: lumit_media::encode::Metadata,
    /// How the composition is rendered for this export.
    pub render: RenderOptions,
    pub when_done: WhenDone,
}

impl Default for ExportSpec {
    /// A comp-sized H.264 mp4 with sound — what a plain "Export…" has always
    /// meant (K-119) — at every setting's own default.
    fn default() -> Self {
        Self {
            format: ExportFormat::Video(lumit_media::encode::VideoCodec::H264),
            target: None,
            bitrate: Bitrate::default(),
            fps: None,
            range: None,
            include_audio: true,
            audio_bit_rate: PRESET_AUDIO_BPS,
            audio_rate: EXPORT_AUDIO_RATE,
            audio_depth: AudioDepth::default(),
            audio_layout: AudioLayout::default(),
            depth: BitDepth::default(),
            channels: Channels::default(),
            alpha: AlphaMode::default(),
            colour_space: ColourSpace::default(),
            crop: Crop::NONE,
            metadata: lumit_media::encode::Metadata::new(),
            render: RenderOptions::default(),
            when_done: WhenDone::default(),
        }
    }
}

impl ExportSpec {
    /// Refuse a spec the chosen format cannot honour, before a single frame is
    /// rendered. A setting a format cannot carry is a mistake worth naming —
    /// silently ignoring it would deliver a file that is not what was asked
    /// for, and the user would find out from someone else.
    pub fn check(&self) -> Result<(), String> {
        let caps = self.format.caps();
        if caps.video && !caps.depths.contains(&self.depth) {
            return Err(format!(
                "{} cannot carry {} colour",
                self.format.extension(),
                self.depth.label()
            ));
        }
        if self.channels == Channels::RgbAlpha && caps.video && !caps.alpha {
            return Err(format!(
                "{} cannot carry an alpha channel",
                self.format.extension()
            ));
        }
        if !self.colour_space.is_available() {
            return Err(format!(
                "the colour space \"{}\" is not available in this build",
                self.colour_space.label()
            ));
        }
        if caps.audio && !caps.audio_rates.contains(&self.audio_rate) {
            return Err(format!(
                "{} cannot be written at {} Hz",
                self.format.extension(),
                self.audio_rate
            ));
        }
        if caps.audio && !caps.audio_depths.contains(&self.audio_depth) {
            return Err(format!(
                "{} cannot carry {}-bit sound",
                self.format.extension(),
                self.audio_depth.bits()
            ));
        }
        if !caps.video && !caps.audio {
            return Err("this format can carry neither picture nor sound".to_owned());
        }
        Ok(())
    }

    /// The video bitrate this spec runs with, as `(target, peak)` — the typed
    /// numbers, or the worked-out ones for `size` — and `None` for a format
    /// with no bitrate to choose. `size` is the frame actually being written,
    /// which is the composition's own whenever no target was named.
    pub fn resolved_bitrate(&self, size: (u32, u32), fps: f64) -> Option<(i64, Option<i64>)> {
        if !self.format.caps().bit_rate {
            return None;
        }
        let codec = match self.format {
            ExportFormat::Video(c) => c,
            // Audio-only: the AAC bitrate is its own field; there is no video
            // rate to work out.
            _ => return None,
        };
        Some(match self.bitrate {
            // No bitrate at all: the encoder chooses its own quality.
            Bitrate::EncoderDefault => return None,
            Bitrate::Auto => {
                let (w, h) = self.target.unwrap_or(size);
                let (t, p) = auto_bitrate(w, h, fps, codec);
                (t, Some(p))
            }
            Bitrate::Manual {
                target_bps,
                peak_bps,
            } => (
                target_bps,
                peak_bps.or_else(|| Some((target_bps as f64 * PEAK_MULTIPLE).round() as i64)),
            ),
        })
    }
}

/// A chosen output rate as the exact rational the encoder is stamped with:
/// thousandths, reduced — 29.97 → 2997/100, 60 → 60/1. Millihertz is finer
/// than any delivery rate needs and keeps the arithmetic in integers.
pub fn fps_rational(fps: f64) -> (i32, i32) {
    let clamped = fps.clamp(1.0, 1000.0);
    let mut num = (clamped * 1000.0).round() as i64;
    let mut den = 1000i64;
    let gcd = {
        let (mut a, mut b) = (num, den);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a.max(1)
    };
    num /= gcd;
    den /= gcd;
    (num as i32, den as i32)
}

/// One export waiting its turn. The document and audio jobs are snapshotted
/// at queue time (docs/06 §7.1): later edits never alter a queued item.
pub struct QueuedExport {
    pub doc: Arc<Document>,
    pub comp_id: Uuid,
    pub items: HashMap<Uuid, ItemInfo>,
    pub audio: Vec<AudioJob>,
    pub out_path: PathBuf,
    pub spec: ExportSpec,
}

pub fn start(
    doc: Arc<Document>,
    comp_id: Uuid,
    audio: Vec<AudioJob>,
    out_path: PathBuf,
    spec: ExportSpec,
) -> ExportHandle {
    let (tx, events) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let result = run(&doc, comp_id, &audio, &out_path, &spec, &tx, &flag);
        let _ = match result {
            Ok(()) if flag.load(Ordering::Relaxed) => {
                let _ = std::fs::remove_file(&out_path); // no half files
                tx.send(ExportEvent::Failed("cancelled".into()))
            }
            Ok(()) => tx.send(ExportEvent::Done(out_path)),
            Err(e) => {
                let _ = std::fs::remove_file(&out_path);
                tx.send(ExportEvent::Failed(e))
            }
        };
    });
    ExportHandle { events, cancel }
}

/// Decode every audio job (resampled to `rate`), lay each on the comp strip
/// at its offset and trim, and sum — the one mixdown all comp audio flows
/// through: preview playback, beat detection, and export, so they cannot
/// disagree about what the comp sounds like.
pub fn mixdown(jobs: &[AudioJob], rate: u32, duration_s: f64) -> Vec<f32> {
    let decoded: Vec<(lumit_media::AudioBuffer, &AudioJob)> = jobs
        .iter()
        .filter_map(|job| {
            lumit_media::audio::decode_all(&job.path, rate)
                .ok()
                .map(|buf| (buf, job))
        })
        .collect();
    let borrowed: Vec<(&lumit_media::AudioBuffer, &AudioJob)> =
        decoded.iter().map(|(b, j)| (b, *j)).collect();
    mix_decoded(&borrowed, rate, duration_s)
}

/// As [`mixdown`], but over already-decoded buffers — the preview path's
/// fast re-mix (docs/09 §2 lazy-decode direction): a solo/mute/trim edit
/// re-sums cached buffers in seconds instead of re-decoding whole files.
pub fn mixdown_prepared(
    decoded: &[(std::sync::Arc<lumit_media::AudioBuffer>, AudioJob)],
    rate: u32,
    duration_s: f64,
) -> Vec<f32> {
    let borrowed: Vec<(&lumit_media::AudioBuffer, &AudioJob)> =
        decoded.iter().map(|(b, j)| (b.as_ref(), j)).collect();
    mix_decoded(&borrowed, rate, duration_s)
}

/// The shared placement + sum over decoded buffers (each already at `rate`).
fn mix_decoded(
    decoded: &[(&lumit_media::AudioBuffer, &AudioJob)],
    rate: u32,
    duration_s: f64,
) -> Vec<f32> {
    let total_frames = (duration_s * f64::from(rate)).round().max(0.0) as usize;
    let placements: Vec<lumit_audio::mix::PlacedAudio> = decoded
        .iter()
        .filter_map(|(buf, job)| {
            let (start_frame, src_start, len) = lumit_audio::mix::place_on_timeline(
                job.in_s,
                job.out_s,
                job.offset_s,
                buf.samples.len() / 2,
                rate,
            )?;
            let (gain, envelope) = volume_bake(job, start_frame, len, rate);
            Some(lumit_audio::mix::PlacedAudio {
                start_frame,
                samples: &buf.samples[src_start * 2..(src_start + len) * 2],
                gain,
                envelope,
            })
        })
        .collect();
    lumit_audio::mix::mix_stereo(&placements, total_frames)
}

/// How many audio samples (per channel) belong before the end of video
/// frame `frame_count` — the A/V interleaving rule. Cumulative rounding, so
/// the running total never drifts from `frames / fps × rate`.
pub fn audio_samples_through(frame_count: usize, fps: f64, rate: u32) -> usize {
    if fps <= 0.0 {
        return 0;
    }
    ((frame_count as f64 / fps) * f64::from(rate)).round() as usize
}

fn run(
    doc: &Arc<Document>,
    comp_id: Uuid,
    audio_jobs: &[AudioJob],
    out_path: &std::path::Path,
    spec: &ExportSpec,
    tx: &Sender<ExportEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    spec.check()?;
    // The render settings that change the document do it on this export's own
    // throwaway snapshot, never on the project (docs/06 §7.2).
    let overridden = apply_render_overrides(doc, &spec.render);
    let doc = overridden.as_ref().unwrap_or(doc);
    let comp = doc.comp(comp_id).ok_or("composition missing")?;
    let fps = comp.frame_rate.fps().max(1.0);
    let comp_frames = (comp.duration.0.to_f64() * fps).round().max(1.0) as usize;
    // The range: the dialogue's own when it set one, else the work area, else
    // the whole comp (docs/01-GLOSSARY.md; K-037 relies on the work-area rule).
    let (first, end) = match spec.range {
        Some((a, b)) => {
            let s = a.min(comp_frames.saturating_sub(1));
            let e = b.clamp(s + 1, comp_frames);
            (s, e)
        }
        None => match comp.work_area {
            Some((a, b)) => {
                let s = ((a.0.to_f64() * fps).round() as usize).min(comp_frames.saturating_sub(1));
                let e = ((b.0.to_f64() * fps).round() as usize).clamp(s + 1, comp_frames);
                (s, e)
            }
            None => (0, comp_frames),
        },
    };
    // The output rate. A rate other than the comp's resamples by nearest comp
    // frame over the same wall-clock span, so a 60 fps comp exported at 30
    // shows every other frame and lasts exactly as long.
    let out_fps = spec.fps.unwrap_or(fps).clamp(1.0, 1000.0);
    let span_seconds = (end - first) as f64 / fps;
    let total = ((span_seconds * out_fps).round() as usize).max(1);
    let _ = tx.send(ExportEvent::Progress { frame: 0, total });

    // The comp's audio, mixed exactly as playback mixes it, then cut to the
    // export range and padded so sound and picture end together.
    let rate = spec.audio_rate;
    let chans = usize::from(spec.audio_layout.channels());
    // Sound only joins a container that can hold it: a folder of stills has
    // nowhere to put it, and the dialogue says so rather than silently
    // dropping it. An audio-only export is nothing *but* sound, so the
    // include-audio tick has no say there.
    let caps = spec.format.caps();
    let wants_audio = caps.audio && (spec.include_audio || !caps.video);
    // A silent comp exported as sound still writes silence of the right
    // length — an empty .wav would look like a failure that wasn't one — but
    // a video export of a silent comp carries no audio stream at all rather
    // than a mute one.
    let audio_mix: Option<Vec<f32>> = if wants_audio && (!audio_jobs.is_empty() || !caps.video) {
        let full = mixdown(audio_jobs, rate, comp.duration.0.to_f64());
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        // The cut starts where the range starts on the comp's own clock, and
        // covers the output's duration — total frames at the *output* rate —
        // so sound and picture end together whatever rate was chosen.
        let start = audio_samples_through(first, fps, rate).min(full.len() / 2);
        let expect = audio_samples_through(total, out_fps, rate);
        let mut cut = full[start * 2..(start + expect).min(full.len() / 2) * 2].to_vec();
        cut.resize(expect * 2, 0.0);
        // The mix is stereo throughout — playback's own mix, so preview and
        // export cannot disagree — and folds down once, at the very end,
        // where nothing else reads it.
        Some(match spec.audio_layout {
            AudioLayout::Stereo => cut,
            AudioLayout::Mono => lumit_audio::mix::downmix_to_mono(&cut),
        })
    } else {
        None
    };

    // Sound with no picture needs no compositor and no graphics card at all:
    // the mix is already made, and there is nothing to render.
    if let ExportFormat::Audio(format) = spec.format {
        return run_audio_only(
            out_path,
            format,
            spec,
            audio_mix.as_deref(),
            total,
            tx,
            cancel,
        );
    }

    // The export renders through the SAME walk the Viewer does — the headless
    // preview at full decode quality (K-031: preview == export by
    // construction, gated by the bit-identity matrix in `headless::tests`).
    // Its own renderer on its own device, so an export never contends with the
    // Viewer's GPU work.
    let mut renderer =
        crate::headless::HeadlessRenderer::new().map_err(|e| format!("export renderer: {e}"))?;
    let (out_num, out_den) = fps_rational(out_fps);
    // One sink, two shapes: the mp4 muxer, or one image file per frame. The
    // loop below is shared — a second frame loop would be a second chance to
    // disagree about sampling, cancellation or progress.
    // The crop happens in composition pixels (K-419), so the picture that
    // leaves the compositor is cropped first and sized afterwards. When the
    // delivery size *is* the comp's own — every Custom export — the cropped
    // size becomes the file's size, which is what cropping is for; a preset
    // that asked for a different frame letterboxes the cropped picture into
    // it, exactly as an uncropped one does.
    let (crop_w, crop_h) = spec.crop.output_size(comp.width, comp.height);
    let delivered = match spec.target {
        Some(t) if t != (comp.width, comp.height) => t,
        _ => (crop_w, crop_h),
    };
    let mut sink = match spec.format {
        ExportFormat::Video(codec) => {
            // Encoded frame dimensions must be even for 4:2:0 H.264/HEVC.
            let (tw, th) = (delivered.0 & !1, delivered.1 & !1);
            let (tw, th) = (tw.max(2), th.max(2));
            let audio_settings = audio_mix
                .as_ref()
                .map(|_| lumit_media::encode::AudioSettings {
                    rate,
                    bit_rate: spec.audio_bit_rate,
                    codec: lumit_media::encode::AudioCodec::Aac,
                    channels: spec.audio_layout.channels(),
                });
            let (bit_rate, max_rate) = match spec.resolved_bitrate((tw, th), out_fps) {
                Some((target, peak)) => (Some(target), peak),
                None => (None, None),
            };
            let encoder = lumit_media::Encoder::open(
                out_path,
                Some(&lumit_media::encode::VideoSettings {
                    codec,
                    width: tw,
                    height: th,
                    fps_num: out_num,
                    fps_den: out_den,
                    bit_rate,
                    max_rate,
                }),
                audio_settings.as_ref(),
                &spec.metadata,
            )
            .map_err(|e| e.to_string())?;
            let _ = tx.send(ExportEvent::Encoder(encoder.encoder_label()));
            Sink::Video {
                encoder,
                size: (tw, th),
            }
        }
        ExportFormat::Images(format) => {
            // Stills have no chroma subsampling, so no evenness rule.
            let (tw, th) = (delivered.0.max(1), delivered.1.max(1));
            let encoder = lumit_media::encode::ImageSequenceEncoder::open(
                out_path, format, tw, th, out_num, out_den, spec.depth,
            )
            .map_err(|e| e.to_string())?;
            let _ = tx.send(ExportEvent::Encoder(format.label()));
            Sink::Images {
                encoder,
                size: (tw, th),
                written: 0,
            }
        }
        // Handled above: sound with no picture never reaches the frame loop.
        ExportFormat::Audio(_) => return Err("audio-only export took the picture path".into()),
    };
    let resize = sink.size() != (crop_w, crop_h);

    let mut audio_fed = 0usize;
    for frame_n in 0..total {
        if cancel.load(Ordering::Relaxed) {
            sink.remove_written(out_path);
            return Ok(());
        }
        // The comp frame under this output frame: exact when the rates match
        // (the rounding is then of an integer), nearest otherwise.
        let src = first + ((frame_n as f64) * fps / out_fps).round() as usize;
        let src = src.min(end.saturating_sub(1));
        let (rgba, _, _) =
            renderer.render_preview(doc, comp_id, src as u64, spec.render.quality, 1.0)?;
        // Crop in composition pixels first, then letterbox into the delivery
        // frame when the size was changed, then pack to what the file carries.
        let rgba = spec.crop.apply(&rgba, comp.width, comp.height, 4);
        let (tw, th) = sink.size();
        let rgba = if resize {
            lumit_core::pixels::letterbox_resize(&rgba, crop_w, crop_h, tw, th)
        } else {
            rgba
        };
        let rgba = pack_frame(&rgba, spec.channels, spec.alpha, spec.depth);
        if let Err(e) = sink.write_rgba(&rgba) {
            // A folder of stills that failed half-way is tidied rather than
            // left as a trap that looks like a finished export.
            sink.remove_written(out_path);
            return Err(e);
        }
        // Interleave: after each picture frame, the samples that cover it,
        // so the muxer keeps sound and picture together in the file.
        if let (Some(mix), Sink::Video { encoder, .. }) = (&audio_mix, &mut sink) {
            let upto = audio_samples_through(frame_n + 1, out_fps, rate).min(mix.len() / chans);
            if upto > audio_fed {
                encoder
                    .write_audio(&mix[audio_fed * chans..upto * chans])
                    .map_err(|e| e.to_string())?;
                audio_fed = upto;
            }
        }
        let _ = tx.send(ExportEvent::Progress {
            frame: frame_n + 1,
            total,
        });
    }
    // Any samples the per-frame rounding left behind.
    if let (Some(mix), Sink::Video { encoder, .. }) = (&audio_mix, &mut sink) {
        if mix.len() / chans > audio_fed {
            encoder
                .write_audio(&mix[audio_fed * chans..])
                .map_err(|e| e.to_string())?;
        }
    }
    sink.finish()
}

/// Write the comp's mix with no picture at all (docs/06 §7.4). No compositor,
/// no graphics card: the mixdown above is the whole export, so this feeds it
/// to the muxer in one-second helpings and reports progress against the same
/// frame count a video export would have written.
fn run_audio_only(
    out_path: &std::path::Path,
    format: AudioFormat,
    spec: &ExportSpec,
    mix: Option<&[f32]>,
    total: usize,
    tx: &Sender<ExportEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let rate = spec.audio_rate;
    let chans = usize::from(spec.audio_layout.channels());
    let mut encoder = lumit_media::Encoder::open(
        out_path,
        None,
        Some(&lumit_media::encode::AudioSettings {
            rate,
            bit_rate: spec.audio_bit_rate,
            codec: format.codec(spec.audio_depth),
            channels: spec.audio_layout.channels(),
        }),
        &spec.metadata,
    )
    .map_err(|e| e.to_string())?;
    let _ = tx.send(ExportEvent::Encoder(encoder.encoder_label()));

    // A comp with no audible layer still exports a file — of silence, of the
    // right length. An empty .wav would look like a failure that wasn't one.
    let silence;
    let mix = match mix {
        Some(m) => m,
        None => {
            silence = Vec::new();
            &silence
        }
    };
    let chunk = rate as usize * chans;
    for (n, block) in mix.chunks(chunk).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        encoder.write_audio(block).map_err(|e| e.to_string())?;
        // Progress against the picture's own clock, so the queue row reads
        // the same way whatever an item writes.
        let done = ((n + 1) * chunk / chans).min(mix.len() / chans);
        let frame = if mix.is_empty() {
            total
        } else {
            (done * total / (mix.len() / chans).max(1)).min(total)
        };
        let _ = tx.send(ExportEvent::Progress { frame, total });
    }
    encoder.finish().map_err(|e| e.to_string())?;
    let _ = tx.send(ExportEvent::Progress {
        frame: total,
        total,
    });
    Ok(())
}

/// Where the rendered frames go: the mp4 muxer, or the numbered stills. One
/// type so the frame loop stays single.
enum Sink {
    Video {
        encoder: lumit_media::Encoder,
        size: (u32, u32),
    },
    Images {
        encoder: lumit_media::encode::ImageSequenceEncoder,
        size: (u32, u32),
        /// Frames written so far — exactly the files a cancel removes.
        written: usize,
    },
}

impl Sink {
    fn size(&self) -> (u32, u32) {
        match self {
            Sink::Video { size, .. } | Sink::Images { size, .. } => *size,
        }
    }

    fn write_rgba(&mut self, rgba: &[u8]) -> Result<(), String> {
        match self {
            Sink::Video { encoder, .. } => encoder.write_rgba(rgba).map_err(|e| e.to_string()),
            Sink::Images {
                encoder, written, ..
            } => {
                encoder.write_rgba(rgba).map_err(|e| e.to_string())?;
                *written += 1;
                Ok(())
            }
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        match self {
            Sink::Video { encoder, .. } => encoder.finish().map_err(|e| e.to_string()),
            Sink::Images { encoder, .. } => encoder.finish().map_err(|e| e.to_string()),
        }
    }

    /// Remove what a cancelled or failed image export left behind. The mp4
    /// path needs nothing here — its half file is removed by the caller, which
    /// cannot know a sequence's file names; this does.
    fn remove_written(&self, chosen_path: &std::path::Path) {
        if let Sink::Images { written, .. } = self {
            for n in 1..=*written {
                let _ =
                    std::fs::remove_file(lumit_media::encode::sequence_frame_path(chosen_path, n));
            }
        }
    }
}

/// Coverage bytes → white RGBA whose alpha is the coverage (the layer-mask
/// texture format the compositor samples).
pub fn mask_rgba(coverage: &[u8]) -> Vec<u8> {
    coverage.iter().flat_map(|c| [255, 255, 255, *c]).collect()
}

/// CameraPose (core model) -> GPU camera matrix: the single conversion both
/// the preview and the export path share, so they cannot disagree (K-031).
pub fn camera_mat(
    comp_w: u32,
    comp_h: u32,
    pose: lumit_core::model::CameraPose,
) -> lumit_gpu::Mat4 {
    lumit_gpu::camera_matrix(
        comp_w as f32,
        comp_h as f32,
        pose.zoom as f32,
        (
            pose.position.0 as f32,
            pose.position.1 as f32,
            pose.position.2 as f32,
        ),
        (
            pose.rotation_deg.0 as f32,
            pose.rotation_deg.1 as f32,
            pose.rotation_deg.2 as f32,
        ),
    )
}

/// Collect the ItemInfo map from probed media (cheap — it only reads the
/// frontend's probe cache, never touches disk). `slate_size` is the exported
/// comp's dimensions, used to size the missing-footage slate exactly as the
/// preview does.
pub fn item_infos(
    doc: &Document,
    probes: &dyn crate::source::SourceProbes,
    slate_size: (u32, u32),
) -> HashMap<Uuid, ItemInfo> {
    let mut map = HashMap::new();
    for item in &doc.items {
        let ProjectItem::Footage(f) = item else {
            continue;
        };
        let probe = probes.probe(f.id);
        if let Some((fps, _w, _h, frames)) = probe.video() {
            map.insert(
                f.id,
                ItemInfo {
                    path: PathBuf::from(&f.media.absolute_path),
                    fps,
                    frames,
                    missing: None,
                },
            );
        } else if probe.slates() {
            // Missing/unreadable media is carried, not skipped, so export
            // renders the same slate the Viewer shows (K-031). Audio-only and
            // unprobed items are simply absent: no picture, and — crucially —
            // no slate over a perfectly healthy sound file.
            map.insert(
                f.id,
                ItemInfo {
                    path: PathBuf::from(&f.media.absolute_path),
                    fps: 1.0,
                    frames: 1,
                    missing: Some(slate_size),
                },
            );
        }
    }
    map
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_media::encode::VideoCodec;

    /// A 30 fps, 5 s solid comp — the smallest document a real export can run
    /// against (mirrors the headless tests' builder; modules cannot share test
    /// helpers without exporting them, and exporting a test helper is worse).
    fn solid_doc(w: u32, h: u32) -> (Arc<Document>, Uuid) {
        use lumit_core::model::{
            Composition, LayerKind, LinearColour, ProjectItem, SolidDef, Switches,
        };
        use lumit_core::time::{CompTime, Duration as CompDuration, FrameRate, Rational};
        let mut doc = Document::new();
        let solid_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: solid_id,
            name: "Solid".into(),
            colour: LinearColour([0.9, 0.2, 0.1, 1.0]),
            width: w,
            height: h,
            extra: serde_json::Map::new(),
        }));
        let comp_id = Uuid::now_v7();
        doc.items.push(ProjectItem::Composition(Composition {
            id: comp_id,
            name: "Scene".into(),
            width: w,
            height: h,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: CompDuration(Rational::new(5, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![lumit_core::model::Layer {
                graph: Default::default(),
                markers: Vec::new(),
                id: Uuid::now_v7(),
                name: "Solid".into(),
                kind: LayerKind::Solid { def: solid_id },
                in_point: CompTime(Rational::new(0, 1).unwrap()),
                out_point: CompTime(Rational::new(5, 1).unwrap()),
                start_offset: CompTime(Rational::new(0, 1).unwrap()),
                transform: Default::default(),
                matte: None,
                parent: None,
                label: 0,
                volume_db: lumit_core::anim::Property::zero(),
                audio_only: false,
                retime: None,
                interpolation: Default::default(),
                parked_flow: None,
                blend: Default::default(),
                masks: Vec::new(),
                paint: Vec::new(),
                effects: Vec::new(),
                switches: Switches::default(),
                extra: serde_json::Map::new(),
            }],
            markers: Vec::new(),
            motion_blur: lumit_core::model::MotionBlur::default(),
            extra: serde_json::Map::new(),
        }));
        (Arc::new(doc), comp_id)
    }

    fn spec(format: ExportFormat, w: u32, h: u32) -> ExportSpec {
        ExportSpec {
            format,
            target: Some((w, h)),
            include_audio: false,
            ..ExportSpec::default()
        }
    }

    /// Run an export to completion on this thread, skipping (Ok(None)) on a
    /// machine with no GPU adapter — the lavapipe convention.
    fn run_now(
        doc: &Arc<Document>,
        comp: Uuid,
        path: &std::path::Path,
        spec: &ExportSpec,
    ) -> Option<Result<(), String>> {
        let (tx, _rx) = channel();
        let cancel = AtomicBool::new(false);
        match run(doc, comp, &[], path, spec, &tx, &cancel) {
            Err(e) if e.starts_with("export renderer:") => {
                lumit_gpu::no_adapter();
                None
            }
            other => Some(other),
        }
    }

    /// The chosen rate is stamped as an exact rational, never a rounded whole
    /// number — 29.97 must not quietly become 30 (docs/impl/rational-time).
    #[test]
    fn fps_rational_keeps_fractional_rates_exact() {
        assert_eq!(fps_rational(60.0), (60, 1));
        assert_eq!(fps_rational(29.97), (2997, 100));
        assert_eq!(fps_rational(23.976), (2997, 125));
        assert_eq!(fps_rational(0.0), (1, 1), "clamped, never zero");
    }

    /// An explicit range exports exactly its frames — here comp frames 10..20
    /// as a PNG sequence, so the file count *is* the assertion (K-201).
    #[test]
    fn an_explicit_range_exports_exactly_its_frames_as_stills() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let mut sp = spec(
            ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            32,
            16,
        );
        sp.range = Some((10, 20));
        let Some(result) = run_now(&doc, comp, &path, &sp) else {
            return;
        };
        result.expect("export runs");
        for n in 1..=10 {
            assert!(
                lumit_media::encode::sequence_frame_path(&path, n).exists(),
                "frame {n} missing"
            );
        }
        assert!(
            !lumit_media::encode::sequence_frame_path(&path, 11).exists(),
            "ten frames were asked for, ten written"
        );
    }

    /// A different output rate keeps the wall-clock span: one second of a
    /// 30 fps comp at 10 fps is ten frames, not thirty.
    #[test]
    fn an_fps_override_resamples_over_the_same_span() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slow.png");
        let mut sp = spec(
            ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            32,
            16,
        );
        sp.range = Some((0, 30));
        sp.fps = Some(10.0);
        let Some(result) = run_now(&doc, comp, &path, &sp) else {
            return;
        };
        result.expect("export runs");
        assert!(lumit_media::encode::sequence_frame_path(&path, 10).exists());
        assert!(
            !lumit_media::encode::sequence_frame_path(&path, 11).exists(),
            "one second at 10 fps is ten frames"
        );
    }

    /// The mp4 path takes the same range and rate machinery and writes a real
    /// file — the smoke that the Sink split did not orphan the video half.
    #[test]
    fn a_ranged_mp4_export_still_writes_a_file() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.mp4");
        let mut sp = spec(ExportFormat::Video(VideoCodec::H264), 32, 16);
        sp.range = Some((0, 15));
        sp.fps = Some(15.0);
        let Some(result) = run_now(&doc, comp, &path, &sp) else {
            return;
        };
        result.expect("export runs");
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        assert!(len > 0, "the mp4 has bytes in it");
    }

    /// Volume baking (docs/09 §6): a static Volume is exactly its constant
    /// gain; a keyframed fade becomes a control-rate envelope sampled in
    /// layer time (comp time − start offset), falling to true zero at the
    /// −inf knee.
    #[test]
    fn volume_bake_static_gain_and_animated_envelope() {
        use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
        use lumit_core::Rational;
        let job = |volume: Property, offset_s: f64| AudioJob {
            item: uuid::Uuid::nil(),
            path: PathBuf::new(),
            in_s: 0.0,
            out_s: 10.0,
            offset_s,
            volume,
            carriers: Vec::new(),
        };
        let (g, env) = volume_bake(&job(Property::fixed(-6.0), 0.0), 0, 48_000, 48_000);
        assert!(env.is_none(), "static volume needs no envelope");
        assert!((g - 0.501_19).abs() < 1e-3);

        // A 1 s fade 0 dB → −inf, on a layer whose time 0 sits at comp 1 s.
        let key = |t: i64, v: f64| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value: v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let fade = Property {
            animation: Animation::Keyframed(vec![key(0, 0.0), key(1, -100.0)]),
            extra: serde_json::Map::new(),
        };
        // Placed at comp 1 s (start_frame 48000), offset 1 s: layer time 0..1.
        let (g, env) = volume_bake(&job(fade.clone(), 1.0), 48_000, 48_000, 48_000);
        assert_eq!(g, 1.0);
        let env = env.unwrap();
        assert!((env.gain_at(0) - 1.0).abs() < 1e-6, "fade starts at unity");
        assert!(
            env.gain_at(0) > env.gain_at(24_000) && env.gain_at(24_000) > env.gain_at(47_500),
            "the fade descends"
        );
        assert_eq!(env.gain_at(48_000), 0.0, "the −inf knee lands at silence");

        // Carrier chain (precomp audio): the precomp layer's −6 dB multiplies
        // the inner layer's −6 dB — two static links stay a constant product.
        let mut carried = job(Property::fixed(-6.0), 0.0);
        carried.carriers = vec![(Property::fixed(-6.0), 0.0)];
        let (g, env) = volume_bake(&carried, 0, 48_000, 48_000);
        assert!(env.is_none());
        assert!(
            (g - 0.251_19).abs() < 1e-3,
            "gains multiply through the chain"
        );
        // An animated carrier envelopes the whole chain.
        let mut fading_carrier = job(Property::fixed(0.0), 0.0);
        fading_carrier.carriers = vec![(fade, 0.0)];
        let (_, env) = volume_bake(&fading_carrier, 0, 48_000, 48_000);
        assert!(env.is_some(), "an animated carrier forces the envelope");
    }

    /// The delivery-preset table is spec (docs/06 §7.5): frame, codec, and
    /// bitrates are pinned here so a stray edit can't silently change what
    /// "YouTube 1080p60" means.
    #[test]
    fn preset_table_matches_the_spec() {
        let p = ExportPreset::Youtube1080p60.params().unwrap();
        assert_eq!(p.size, (1920, 1080));
        assert_eq!(p.codec, VideoCodec::H264);
        assert_eq!(p.target_bps, 16_000_000);
        assert_eq!(p.peak_bps, 24_000_000);

        let p = ExportPreset::Youtube4k60.params().unwrap();
        assert_eq!(p.size, (3840, 2160));
        assert_eq!(p.codec, VideoCodec::Hevc);
        assert_eq!(p.target_bps, 45_000_000);
        assert_eq!(p.peak_bps, 60_000_000);

        let p = ExportPreset::Vertical1080p60.params().unwrap();
        assert_eq!(p.size, (1080, 1920));
        assert_eq!(p.codec, VideoCodec::H264);
        assert_eq!(p.target_bps, 16_000_000);
        assert_eq!(p.peak_bps, 24_000_000);

        assert!(ExportPreset::Custom.params().is_none());
        assert_eq!(PRESET_AUDIO_BPS, 320_000);
        assert_eq!(EXPORT_AUDIO_RATE, 48_000);
    }

    #[test]
    fn every_preset_has_a_label_and_file_name() {
        for preset in ExportPreset::ALL {
            assert!(!preset.label().is_empty());
            assert!(preset.default_file_name().ends_with(".mp4"));
        }
    }

    /// K-119: `ExportPreset::default()` must be Custom, so a fresh Settings →
    /// Export default-preset field reproduces today's implicit behaviour
    /// (every generic "Export…" action stamping Custom) until the user
    /// changes it. Also proves the type round-trips through JSON, which
    /// `ExportSettings` (settings.rs) relies on to persist the pick.
    #[test]
    fn export_preset_defaults_to_custom_and_round_trips_through_json() {
        assert_eq!(ExportPreset::default(), ExportPreset::Custom);
        for preset in ExportPreset::ALL {
            let json = serde_json::to_string(&preset).unwrap();
            let back: ExportPreset = serde_json::from_str(&json).unwrap();
            assert_eq!(back, preset);
        }
    }

    /// The A/V interleave rule: cumulative rounding never drifts, and the
    /// total after all frames equals the whole soundtrack.
    #[test]
    fn audio_samples_through_never_drifts() {
        let (fps, rate) = (60.0, 48_000u32);
        // 60 fps at 48 kHz is exactly 800 samples per frame.
        assert_eq!(audio_samples_through(1, fps, rate), 800);
        assert_eq!(audio_samples_through(300, fps, rate), 240_000);
        // An awkward rate: 29.97 fps. Per-frame chunks vary by ±1 sample but
        // the cumulative total stays glued to the exact value.
        let fps = 30_000.0 / 1001.0;
        let mut prev = 0;
        for n in 1..=1000 {
            let now = audio_samples_through(n, fps, rate);
            let chunk = now - prev;
            assert!((1601..=1602).contains(&chunk), "frame {n} chunk {chunk}");
            let exact = n as f64 / fps * 48_000.0;
            assert!((now as f64 - exact).abs() <= 0.5, "frame {n} drifted");
            prev = now;
        }
        // Degenerate input answers zero, never panics.
        assert_eq!(audio_samples_through(100, 0.0, rate), 0);
    }

    /// A silent comp exports video-only; the padding rule keeps sound and
    /// picture the same length when there is audio.
    #[test]
    fn mixdown_of_no_jobs_is_silence_of_the_right_length() {
        let mix = mixdown(&[], 48_000, 2.0);
        assert_eq!(mix.len(), 96_000 * 2);
        assert!(mix.iter().all(|s| *s == 0.0));
    }

    /// The capability table is the one place a format's limits are written
    /// down. It is spec (docs/06 §7.4), so it is pinned here rather than
    /// discovered by an export that fails halfway.
    #[test]
    fn the_capability_table_says_what_each_format_can_carry() {
        use lumit_media::encode::ImageFormat;

        let mp4 = ExportFormat::Video(VideoCodec::H264).caps();
        assert!(mp4.video && mp4.audio && mp4.metadata);
        assert!(!mp4.alpha, "4:2:0 H.264 has no alpha channel");
        assert_eq!(mp4.depths, [BitDepth::Eight]);
        assert!(mp4.bit_rate);

        let png = ExportFormat::Images(ImageFormat::Png).caps();
        assert!(png.video && png.alpha);
        assert!(!png.audio, "a folder of stills has nowhere to put sound");
        assert!(!png.bit_rate, "lossless has no bitrate to choose");
        assert!(!png.metadata, "the image2 muxer has no container to tag");
        assert_eq!(png.depths, [BitDepth::Eight, BitDepth::Sixteen]);
        assert_eq!(ExportFormat::Images(ImageFormat::Tiff).caps(), png);

        let m4a = ExportFormat::Audio(AudioFormat::M4a).caps();
        assert!(!m4a.video && m4a.audio && m4a.metadata);
        assert!(m4a.bit_rate, "AAC has a bitrate");
        let wav = ExportFormat::Audio(AudioFormat::Wav).caps();
        assert!(!wav.bit_rate, "PCM is exactly what it is");

        // Extensions match the formats, so a filename and its contents agree.
        assert_eq!(ExportFormat::Video(VideoCodec::Hevc).extension(), "mp4");
        assert_eq!(ExportFormat::Images(ImageFormat::Tiff).extension(), "tiff");
        assert_eq!(ExportFormat::Audio(AudioFormat::Wav).extension(), "wav");
    }

    /// A setting the chosen format cannot honour is refused before a frame is
    /// rendered — never silently dropped, which would deliver a file that is
    /// not what was asked for.
    #[test]
    fn a_spec_the_format_cannot_honour_is_refused() {
        use lumit_media::encode::ImageFormat;
        let base = spec(ExportFormat::Video(VideoCodec::H264), 320, 240);
        base.check().expect("the plain case runs");

        let deep = ExportSpec {
            depth: BitDepth::Sixteen,
            ..base.clone()
        };
        assert!(deep.check().is_err(), "mp4 cannot carry 16-bit");

        let transparent = ExportSpec {
            channels: Channels::RgbAlpha,
            ..base.clone()
        };
        assert!(transparent.check().is_err(), "mp4 cannot carry alpha");

        // The same two settings are fine on a PNG sequence.
        let stills = ExportSpec {
            format: ExportFormat::Images(ImageFormat::Png),
            depth: BitDepth::Sixteen,
            channels: Channels::RgbAlpha,
            ..base.clone()
        };
        stills.check().expect("a PNG carries both");

        // An OCIO space is refused until OCIO exists: a wrong colour space in
        // a delivered file is worse than an export that did not run.
        let ocio = ExportSpec {
            colour_space: ColourSpace::Ocio("ACES - ACEScg".into()),
            ..base
        };
        assert!(ocio.check().is_err());
        assert!(ColourSpace::SrgbRec709.is_available());
        assert!(!ColourSpace::Ocio("anything".into()).is_available());
    }

    /// Crop arithmetic, in pixels at composition size (K-419): the size it
    /// leaves, the window it keeps, and the pixels it actually copies.
    #[test]
    fn crop_maths_keeps_the_window_it_says_it_keeps() {
        let crop = Crop {
            top: 1,
            left: 2,
            bottom: 3,
            right: 4,
        };
        assert_eq!(crop.output_size(10, 10), (4, 6));
        assert_eq!(crop.window(10, 10), (2, 1, 4, 6));
        assert!(Crop::NONE.is_none());
        assert_eq!(Crop::NONE.output_size(10, 10), (10, 10));

        // Insets that meet leave one pixel rather than none — a slip of the
        // fingers is not a reason to fail an export.
        let silly = Crop {
            top: 99,
            left: 99,
            bottom: 99,
            right: 99,
        };
        assert_eq!(silly.output_size(10, 10), (1, 1));
        let (x, y, w, h) = silly.window(10, 10);
        assert_eq!((w, h), (1, 1));
        assert!(x < 10 && y < 10, "the window stays inside the frame");

        // The pixels: a 4×3 frame of one byte per pixel, numbered by position.
        let frame: Vec<u8> = (0..12).collect();
        let one_off_each_side = Crop {
            top: 1,
            left: 1,
            bottom: 1,
            right: 1,
        };
        assert_eq!(one_off_each_side.apply(&frame, 4, 3, 1), vec![5, 6]);
        // Four bytes a pixel, the real shape.
        let rgba: Vec<u8> = (0..(4 * 2 * 2)).collect();
        let right_half = Crop {
            top: 0,
            left: 1,
            bottom: 0,
            right: 0,
        };
        assert_eq!(
            right_half.apply(&rgba, 2, 2, 4),
            vec![4, 5, 6, 7, 12, 13, 14, 15]
        );
        // No crop is the frame itself, and a buffer too small comes back
        // whole rather than panicking mid-export.
        assert_eq!(Crop::NONE.apply(&frame, 4, 3, 1), frame);
        assert_eq!(one_off_each_side.apply(&[1, 2, 3], 4, 3, 1), vec![1, 2, 3]);
        // Regression: a zero-sized frame used to index past an empty buffer.
        assert!(one_off_each_side.apply(&[], 0, 0, 4).is_empty());
        assert!(one_off_each_side.apply(&[], 4, 0, 4).is_empty());
    }

    /// The region of interest crosses as fractions (K-362) and becomes pixel
    /// insets here; degenerate input is a gesture, not an error.
    #[test]
    fn a_region_of_interest_becomes_pixel_insets() {
        // The middle half of a 100×100 comp.
        assert_eq!(
            Crop::from_region([0.25, 0.25, 0.75, 0.75], 100, 100),
            Crop {
                top: 25,
                left: 25,
                bottom: 25,
                right: 25
            }
        );
        // The whole frame is no crop at all.
        assert!(Crop::from_region([0.0, 0.0, 1.0, 1.0], 100, 100).is_none());
        // Inside-out, empty and non-finite all answer no crop.
        assert!(Crop::from_region([0.8, 0.1, 0.2, 0.9], 100, 100).is_none());
        assert!(Crop::from_region([0.5, 0.5, 0.5, 0.5], 100, 100).is_none());
        assert!(Crop::from_region([f64::NAN, 0.0, 1.0, 1.0], 100, 100).is_none());

        // The two faces of the dialog: the region wins when asked for, the
        // typed crop stands otherwise — and when the region is no region.
        let typed = Crop {
            top: 5,
            left: 5,
            bottom: 5,
            right: 5,
        };
        let region = Some([0.25, 0.25, 0.75, 0.75]);
        assert_eq!(crop_for(typed, false, region, 100, 100), typed);
        assert_eq!(crop_for(typed, true, None, 100, 100), typed);
        assert_eq!(crop_for(typed, true, Some([0.0; 4]), 100, 100), typed);
        assert_eq!(
            crop_for(typed, true, region, 100, 100).left,
            25,
            "the region wins when it is asked for and real"
        );
    }

    /// The pack stage: what each channel/alpha/depth choice does to the
    /// finished pixels, on the CPU, without a graphics card in sight.
    #[test]
    fn the_pack_stage_writes_what_each_choice_asks_for() {
        // One half-covered premultiplied pixel and one opaque one.
        let src = [100u8, 50, 0, 128, 10, 20, 30, 255];

        // RGB: alpha forced opaque, colour untouched.
        let rgb = pack_frame(
            &src,
            Channels::Rgb,
            AlphaMode::Premultiplied,
            BitDepth::Eight,
        );
        assert_eq!(rgb, [100, 50, 0, 255, 10, 20, 30, 255]);

        // Premultiplied RGBA: exactly what the compositor produced.
        let pre = pack_frame(
            &src,
            Channels::RgbAlpha,
            AlphaMode::Premultiplied,
            BitDepth::Eight,
        );
        assert_eq!(pre, src);

        // Straight RGBA: colour divided back up by its coverage.
        let straight = pack_frame(
            &src,
            Channels::RgbAlpha,
            AlphaMode::Straight,
            BitDepth::Eight,
        );
        assert_eq!(straight[3], 128, "coverage itself is unchanged");
        assert_eq!(straight[0], 199, "100 / (128/255) rounds to 199");
        assert_eq!(straight[1], 100);
        assert_eq!(&straight[4..], &src[4..], "an opaque pixel is untouched");

        // A colour beyond its own coverage clamps rather than overflowing,
        // and zero coverage has no colour to recover.
        let odd = [200u8, 0, 0, 100, 9, 9, 9, 0];
        let straight = pack_frame(
            &odd,
            Channels::RgbAlpha,
            AlphaMode::Straight,
            BitDepth::Eight,
        );
        assert_eq!(straight[0], 255);
        assert_eq!(&straight[4..], &[0, 0, 0, 0]);

        // Sixteen bits: × 257, little-endian, exactly reversible.
        let wide = pack_frame(
            &src,
            Channels::RgbAlpha,
            AlphaMode::Premultiplied,
            BitDepth::Sixteen,
        );
        assert_eq!(wide.len(), src.len() * 2);
        let samples: Vec<u16> = wide
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(samples[0], 100 * 257);
        assert_eq!(samples[7], 65_535, "255 widens to full scale");
        assert!(
            samples.iter().zip(src).all(|(w, n)| (w / 257) as u8 == n),
            "the widening is reversible: nothing is invented"
        );
    }

    /// The auto bitrate is a straight line through the preset table's own
    /// 1080p60 point, and it moves the way more pixels and more frames should.
    #[test]
    fn the_auto_bitrate_lands_on_the_preset_tables_own_numbers() {
        let (target, peak) = auto_bitrate(1920, 1080, 60.0, VideoCodec::H264);
        assert_eq!(target, 16_000_000, "docs/06 §7.5's 1080p60 target");
        assert_eq!(peak, 24_000_000, "and its peak, at the 1.5× rule");

        // Half the frames, half the bits.
        let (half, _) = auto_bitrate(1920, 1080, 30.0, VideoCodec::H264);
        assert_eq!(half, 8_000_000);
        // HEVC buys a quarter off.
        let (hevc, _) = auto_bitrate(1920, 1080, 60.0, VideoCodec::Hevc);
        assert!(hevc < target, "hevc {hevc} < h264 {target}");
        // More pixels, more bits — monotonic in every direction.
        let (uhd, _) = auto_bitrate(3840, 2160, 60.0, VideoCodec::Hevc);
        assert!(uhd > hevc);
        // Absurd input clamps rather than overflowing or answering zero.
        let (tiny, _) = auto_bitrate(1, 1, 1.0, VideoCodec::H264);
        assert_eq!(tiny, 1_000_000);
        let (huge, huge_peak) = auto_bitrate(60_000, 40_000, 1000.0, VideoCodec::H264);
        assert_eq!(huge, 400_000_000);
        assert!(huge_peak > huge);
    }

    /// Auto versus manual is stored in the settings and resolved at the last
    /// moment, against the frame actually being written.
    #[test]
    fn the_bitrate_choice_resolves_auto_manual_and_neither() {
        use lumit_media::encode::ImageFormat;
        let auto = spec(ExportFormat::Video(VideoCodec::H264), 1920, 1080);
        assert_eq!(auto.bitrate, Bitrate::Auto, "auto is the default");
        assert_eq!(
            auto.resolved_bitrate((1920, 1080), 60.0),
            Some((16_000_000, Some(24_000_000)))
        );

        // A typed number with no peak takes the 1.5× fallback.
        let manual = ExportSpec {
            bitrate: Bitrate::Manual {
                target_bps: 10_000_000,
                peak_bps: None,
            },
            ..auto.clone()
        };
        assert_eq!(
            manual.resolved_bitrate((1920, 1080), 60.0),
            Some((10_000_000, Some(15_000_000)))
        );

        // No target named: the composition's own size decides the auto rate.
        let comp_sized = ExportSpec {
            target: None,
            ..auto.clone()
        };
        assert_eq!(
            comp_sized.resolved_bitrate((1280, 720), 30.0),
            Some((4_000_000, Some(6_000_000)))
        );

        // Lossless and audio-only formats have no video bitrate at all.
        let stills = ExportSpec {
            format: ExportFormat::Images(ImageFormat::Png),
            ..auto.clone()
        };
        assert_eq!(stills.resolved_bitrate((1920, 1080), 60.0), None);
        let sound = ExportSpec {
            format: ExportFormat::Audio(AudioFormat::M4a),
            ..auto
        };
        assert_eq!(sound.resolved_bitrate((1920, 1080), 60.0), None);
    }

    /// The document-shaped render options act on the export's own snapshot,
    /// through nested comps, and leave the original untouched.
    #[test]
    fn render_overrides_clear_fx_and_solo_everywhere_or_nothing_at_all() {
        let (doc, comp_id) = solid_doc(32, 16);
        // Give the one layer both switches something to clear.
        let mut seeded = Document::clone(&doc);
        for item in &mut seeded.items {
            if let ProjectItem::Composition(c) = item {
                for l in &mut c.layers {
                    l.switches.fx = true;
                    l.switches.solo = true;
                }
            }
        }
        let seeded = Arc::new(seeded);

        // Defaults change nothing, and say so by answering None rather than
        // cloning a whole document to alter nothing.
        assert!(apply_render_overrides(&seeded, &RenderOptions::default()).is_none());

        let off = RenderOptions {
            effects: false,
            honour_solo: false,
            ..RenderOptions::default()
        };
        let patched = apply_render_overrides(&seeded, &off).expect("something changed");
        let layer = &patched.comp(comp_id).unwrap().layers[0];
        assert!(!layer.switches.fx, "effects off clears the fx switch");
        assert!(!layer.switches.solo, "solo ignored clears the solo switch");
        // The snapshot the export was handed is untouched.
        let original = &seeded.comp(comp_id).unwrap().layers[0];
        assert!(original.switches.fx && original.switches.solo);

        // Each half acts on its own.
        let fx_only = RenderOptions {
            effects: false,
            ..RenderOptions::default()
        };
        let patched = apply_render_overrides(&seeded, &fx_only).unwrap();
        let layer = &patched.comp(comp_id).unwrap().layers[0];
        assert!(!layer.switches.fx && layer.switches.solo);
    }

    /// The render settings' own defaults: an export renders at full quality
    /// with everything on and no disk cache (docs/06 §7.3 — export never
    /// degrades), and the export tier is the preview's own machinery.
    #[test]
    fn export_renders_at_full_quality_with_everything_on() {
        let opts = RenderOptions::default();
        assert_eq!(opts.quality, crate::plan::Quality::default());
        assert_eq!(opts.quality.divisor, 1);
        assert!(!opts.quality.draft, "an export never drafts");
        assert_eq!(opts.disk_cache, DiskCachePolicy::Off);
        assert!(opts.effects && opts.honour_solo);
        assert!(!opts.changes_document());

        // A half-resolution export is the preview's own tier, not a new one.
        let half = RenderOptions {
            quality: crate::plan::Quality {
                divisor: 2,
                ..crate::plan::Quality::default()
            },
            ..RenderOptions::default()
        };
        assert!(!half.changes_document(), "a tier is not a document change");
    }

    /// The when-done hook tolerates a missing sound file in silence — the
    /// owner supplies one later, and until then a finished export must not
    /// look failed.
    #[test]
    fn the_when_done_hook_tolerates_a_missing_sound() {
        assert_eq!(WhenDone::default(), WhenDone::Nothing);
        // Whatever this machine has (probably nothing), it answers rather
        // than panicking, and the answer agrees with the path it resolved.
        assert_eq!(play_done_sound(), done_sound_path().is_some());
        // The whole settings payload round-trips, hook included.
        let spec = ExportSpec {
            when_done: WhenDone::MakeANoise,
            ..ExportSpec::default()
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: ExportSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec);
        // A payload missing every field is today's defaults, so an older
        // preset still loads.
        let bare: ExportSpec = serde_json::from_str("{}").unwrap();
        assert_eq!(bare, ExportSpec::default());
    }

    /// Audio-only export, end to end through `run`: no compositor, no
    /// graphics card, a real `.wav` of the range's own length.
    #[test]
    fn an_audio_only_export_writes_the_range_as_a_wav() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mix.wav");
        let mut sp = spec(ExportFormat::Audio(AudioFormat::Wav), 32, 16);
        sp.include_audio = true;
        // Comp frames 0..60 of a 30 fps comp — two seconds.
        sp.range = Some((0, 60));

        let (tx, _rx) = channel();
        let cancel = AtomicBool::new(false);
        run(&doc, comp, &[], &path, &sp, &tx, &cancel).expect("audio-only export runs");

        let probe = lumit_media::probe::probe(&path).unwrap();
        assert!(probe.video.is_none(), "no picture in an audio-only export");
        let audio = probe.audio.expect("it is all sound");
        assert_eq!((audio.sample_rate, audio.channels), (48_000, 2));
        assert!(
            (probe.duration_seconds - 2.0).abs() < 0.05,
            "two seconds of silence, not {}",
            probe.duration_seconds
        );
    }

    /// Every sample rate, sample width and channel layout the dialog offers,
    /// end to end through `run` and probed back off disk. The engine's answer
    /// and the file's own header have to be the same answer.
    #[test]
    fn the_audio_options_reach_the_written_file() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        for (format, ext) in [(AudioFormat::Wav, "wav"), (AudioFormat::M4a, "m4a")] {
            for rate in EXPORT_AUDIO_RATES.iter().copied() {
                for depth in AudioDepth::ALL {
                    for layout in AudioLayout::ALL {
                        let mut sp = spec(ExportFormat::Audio(format), 32, 16);
                        sp.include_audio = true;
                        sp.range = Some((0, 30)); // one second of a 30 fps comp
                        sp.audio_rate = rate;
                        sp.audio_depth = depth;
                        sp.audio_layout = layout;
                        // AAC has no sample width, so only the default stands
                        // there — that refusal is its own test below.
                        if sp.check().is_err() {
                            continue;
                        }
                        let path = dir.path().join(format!(
                            "mix-{ext}-{rate}-{}-{}.{ext}",
                            depth.bits(),
                            layout.channels()
                        ));
                        let (tx, _rx) = channel();
                        let cancel = AtomicBool::new(false);
                        run(&doc, comp, &[], &path, &sp, &tx, &cancel)
                            .unwrap_or_else(|e| panic!("{ext} {rate} Hz: {e}"));

                        let probe = lumit_media::probe::probe(&path).unwrap();
                        let audio = probe.audio.expect("it is all sound");
                        assert_eq!(
                            (audio.sample_rate as u32, audio.channels as u16),
                            (rate, layout.channels()),
                            "{ext} at {rate} Hz, {:?}",
                            layout
                        );
                        if format == AudioFormat::Wav {
                            let want = match depth {
                                AudioDepth::Sixteen => "pcm_s16le",
                                AudioDepth::TwentyFour => "pcm_s24le",
                            };
                            assert_eq!(audio.codec, want);
                        }
                        assert!(
                            (probe.duration_seconds - 1.0).abs() < 0.05,
                            "one second, not {}",
                            probe.duration_seconds
                        );
                    }
                }
            }
        }
    }

    /// The capability table refuses rather than approximates: a width or a
    /// rate the format cannot carry is an error before a frame is rendered,
    /// and every offered combination the format *can* carry passes.
    #[test]
    fn the_audio_capability_table_refuses_what_the_format_cannot_carry() {
        let aac = |f| {
            let mut sp = ExportSpec {
                format: f,
                ..ExportSpec::default()
            };
            sp.audio_depth = AudioDepth::TwentyFour;
            sp
        };
        // AAC stores coefficients, not samples: there is no width to set.
        for f in [
            ExportFormat::Video(VideoCodec::H264),
            ExportFormat::Audio(AudioFormat::M4a),
        ] {
            let err = aac(f).check().expect_err("24-bit AAC is refused");
            assert!(err.contains("24-bit sound"), "{err}");
        }
        // The uncompressed master carries both widths.
        for depth in AudioDepth::ALL {
            let sp = ExportSpec {
                format: ExportFormat::Audio(AudioFormat::Wav),
                audio_depth: depth,
                ..ExportSpec::default()
            };
            assert!(sp.check().is_ok(), "{depth:?} in a wav");
        }
        // A rate off the list is refused, never nudged to the nearest one.
        let odd = ExportSpec {
            audio_rate: 22_050,
            ..ExportSpec::default()
        };
        let err = odd.check().expect_err("22 050 Hz is not offered");
        assert!(err.contains("22050"), "{err}");
        for rate in EXPORT_AUDIO_RATES.iter().copied() {
            for f in [
                ExportFormat::Video(VideoCodec::H264),
                ExportFormat::Audio(AudioFormat::M4a),
                ExportFormat::Audio(AudioFormat::Wav),
            ] {
                let sp = ExportSpec {
                    format: f,
                    audio_rate: rate,
                    ..ExportSpec::default()
                };
                assert!(sp.check().is_ok(), "{rate} Hz in {}", f.extension());
            }
        }
        // A still sequence carries no sound at all, so the audio settings are
        // unread there rather than a reason to refuse a picture.
        let stills = ExportSpec {
            format: ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            audio_rate: 22_050,
            audio_depth: AudioDepth::TwentyFour,
            ..ExportSpec::default()
        };
        assert!(stills.check().is_ok());
    }

    /// Two runs of the same audio export write the same bytes — the standing
    /// determinism rule (docs/06 §7.3), asserted on the new options rather
    /// than only on the old default.
    #[test]
    fn the_same_audio_spec_writes_the_same_bytes_twice() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let mut sp = spec(ExportFormat::Audio(AudioFormat::Wav), 32, 16);
        sp.include_audio = true;
        sp.range = Some((0, 30));
        sp.audio_rate = 44_100;
        sp.audio_depth = AudioDepth::TwentyFour;
        sp.audio_layout = AudioLayout::Mono;

        let mut written = Vec::new();
        for n in 0..2 {
            let path = dir.path().join(format!("twice-{n}.wav"));
            let (tx, _rx) = channel();
            let cancel = AtomicBool::new(false);
            run(&doc, comp, &[], &path, &sp, &tx, &cancel).unwrap();
            written.push(std::fs::read(&path).unwrap());
        }
        assert!(!written[0].is_empty());
        assert_eq!(written[0], written[1], "two runs, two different files");
    }

    /// A spec stored before the sound options existed loads as the behaviour
    /// it was saved under: 48 kHz, sixteen bits, stereo.
    #[test]
    fn a_preset_written_before_the_audio_options_loads_unchanged() {
        let default = ExportSpec::default();
        let mut json: serde_json::Value = serde_json::to_value(&default).unwrap();
        let object = json.as_object_mut().unwrap();
        for key in ["audio_rate", "audio_depth", "audio_layout"] {
            assert!(object.remove(key).is_some(), "{key} is a spec field");
        }
        let old: ExportSpec = serde_json::from_value(json).unwrap();
        assert_eq!(old, default);
        assert_eq!(old.audio_rate, EXPORT_AUDIO_RATE);
        assert_eq!(old.audio_depth, AudioDepth::Sixteen);
        assert_eq!(old.audio_layout, AudioLayout::Stereo);
    }

    /// A crop really crops: the same comp exported with and without one
    /// differs by exactly the pixels the crop took off, and the still's own
    /// size is the assertion.
    #[test]
    fn a_crop_decides_the_exported_frame_size() {
        let (doc, comp) = solid_doc(64, 32);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cropped.png");
        let mut sp = spec(
            ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            64,
            32,
        );
        sp.range = Some((0, 1));
        sp.crop = Crop {
            top: 4,
            left: 8,
            bottom: 4,
            right: 8,
        };
        let Some(result) = run_now(&doc, comp, &path, &sp) else {
            return;
        };
        result.expect("export runs");
        let frame = lumit_media::encode::sequence_frame_path(&path, 1);
        let probe = lumit_media::probe::probe(&frame).unwrap();
        let video = probe.video.expect("a still is a one-frame video");
        assert_eq!(
            (video.width, video.height),
            (48, 24),
            "64−8−8 by 32−4−4, in composition pixels"
        );
    }

    /// A sixteen-bit, alpha-carrying still export runs the whole way through
    /// — the pack stage, the wide encoder, and a file our own probe reads.
    #[test]
    fn a_sixteen_bit_still_export_runs_end_to_end() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        let mut sp = spec(
            ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            32,
            16,
        );
        sp.range = Some((0, 2));
        sp.depth = BitDepth::Sixteen;
        sp.channels = Channels::RgbAlpha;
        sp.alpha = AlphaMode::Straight;
        let Some(result) = run_now(&doc, comp, &path, &sp) else {
            return;
        };
        result.expect("export runs");
        for n in 1..=2 {
            let frame = lumit_media::encode::sequence_frame_path(&path, n);
            let probe = lumit_media::probe::probe(&frame).unwrap();
            let video = probe.video.expect("frame {n} reads back");
            assert_eq!((video.width, video.height), (32, 16));
        }
    }

    /// Metadata reaches the file an export writes, not just the encoder that
    /// was handed it.
    #[test]
    fn export_metadata_reaches_the_written_file() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tagged.mp4");
        let mut sp = spec(ExportFormat::Video(VideoCodec::H264), 32, 16);
        sp.range = Some((0, 5));
        sp.metadata
            .set(lumit_media::encode::Metadata::TITLE, "Scene 1");
        sp.metadata
            .set(lumit_media::encode::Metadata::AUTHOR, "A Person");
        let Some(result) = run_now(&doc, comp, &path, &sp) else {
            return;
        };
        result.expect("export runs");
        // Read it back the way any player would.
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
        assert!(text.contains("Scene 1"), "the title is in the container");
        assert!(text.contains("A Person"), "and so is the author");
    }
}
