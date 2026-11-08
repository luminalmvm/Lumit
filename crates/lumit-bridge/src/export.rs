//! Export from Flutter — resolving the export spec (preset stamp, VBR-peak and
//! filename rules) and driving the lumit-ui exporter over the headless seam.
//!
//! # In plain terms
//!
//! Exporting writes the composited comp to an `.mp4` on its own thread, exactly
//! as the egui frontend does. The bridge reuses the identical exporter
//! (`lumit_render::export`) through the headless seam: the seam builds the
//! footage/audio inputs and lends a GPU context, and `export::start` spawns the
//! encode thread and streams progress back over a channel. The bridge holds that
//! channel's receiver and drains it on each poll, so Dart can drive a simple
//! `start → poll* → done/failed` loop over the C ABI.
//!
//! One export runs at a time (docs/06 §7.1); the rest wait in the queue kept
//! here — each item holding the document snapshot it will render, so editing
//! the composition afterwards never alters what a queued export writes. The
//! queue is turned by the interface's own polling rather than by a thread of
//! its own: the next item starts when the one before it finishes.
//!
//! Two pieces are pure and always compiled (and unit-tested without a GPU):
//! - the **spec conversion** — `BridgeExportSpec` (the dialogue's flat fields)
//!   into `lumit_render::export::ExportSpec` and back, plus the four questions
//!   the dialogue asks the engine rather than answering itself: what a format
//!   can carry, whether a spec is exportable, what a crop leaves, and what
//!   bitrate it will run at;
//! - the **filename template** — `{comp}`/`{preset}`/`{date}` substitution, the
//!   Windows sanitiser and the `.mp4` guarantee, a faithful port of
//!   `shell::export_default_file_name`/`render_filename_template`/
//!   `sanitise_windows_filename`. A blank template reproduces each preset's own
//!   default file name byte-for-byte (load-bearing).
//!
//! Naming a codec needs the `media` feature; without it the conversion answers
//! a calm "this build has no encoder" and every capability reads false, so the
//! dialogue disables rather than offering choices no file would honour.

use crate::err_json;
use serde_json::{json, Value};

mod names;
#[cfg(target_arch = "wasm32")]
mod web;

/// Audio on all delivery presets: AAC 320 kbps (docs/06 §7.5). The bridge's own
/// copy of `lumit_render::export::PRESET_AUDIO_BPS`, so spec resolution needs no GPU
/// build to know the default.
pub(crate) const PRESET_AUDIO_BPS: i64 = 320_000;

/// The parameter row a delivery preset stamps — the bridge's pure mirror of
/// `lumit_render::export::PresetParams` (kept here so the resolver and its tests
/// build with or without the `render` feature). `codec` is the codec name
/// (`h264`/`hevc`); the bitrates are bits/second.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct PresetParams {
    size: (u32, u32),
    codec: &'static str,
    target_bps: i64,
    peak_bps: i64,
}

/// The parameters a preset stamps (`None` for Custom / an unknown name — the
/// dialogue's own fields apply). A faithful copy of `ExportPreset::params`
/// (docs/06 §7.5), keyed by the bridge's snake_case preset names.
fn preset_params(name: &str) -> Option<PresetParams> {
    match name {
        "youtube_1080p60" => Some(PresetParams {
            size: (1920, 1080),
            codec: "h264",
            target_bps: 16_000_000,
            peak_bps: 24_000_000,
        }),
        "youtube_1440p60" => Some(PresetParams {
            size: (2560, 1440),
            codec: "hevc",
            target_bps: 25_000_000,
            peak_bps: 35_000_000,
        }),
        "youtube_4k60" => Some(PresetParams {
            size: (3840, 2160),
            codec: "hevc",
            target_bps: 45_000_000,
            peak_bps: 60_000_000,
        }),
        "vertical_1080p60" => Some(PresetParams {
            size: (1080, 1920),
            codec: "h264",
            target_bps: 16_000_000,
            peak_bps: 24_000_000,
        }),
        // "custom" and any unknown name stamp nothing.
        _ => None,
    }
}

/// A preset's own default file name (`ExportPreset::default_file_name`), the
/// byte-for-byte fallback when no filename template is set.
fn preset_default_file_name(name: &str) -> &'static str {
    match name {
        "youtube_1080p60" => "youtube-1080p60.mp4",
        "youtube_1440p60" => "youtube-1440p60.mp4",
        "youtube_4k60" => "youtube-4k60.mp4",
        "vertical_1080p60" => "vertical-1080x1920.mp4",
        _ => "export.mp4",
    }
}

// ---------------------------------------------------------------------------
// The seam's own half of the export spec: `BridgeExportSpec` in, the
// engine's `ExportSpec` out, and the dialogue's four questions — what can this
// format carry, is this spec exportable, what does the crop leave, what bitrate
// will it run at — answered by the engine rather than re-derived in Dart.
// ---------------------------------------------------------------------------

use crate::api::export::{
    BridgeCrop, BridgeExportDefaults, BridgeExportPresetEntry, BridgeExportSpec, BridgeFormatCaps,
};
use lumit_render::colour::ColourState;
// Only the conversion names a metadata field, and only a build with an encoder
// converts anything.
#[cfg(feature = "media")]
use crate::api::export::BridgeMetadataField;

/// Megabits per second as bits per second, which is what the encoder is set in.
#[cfg(feature = "media")]
fn bps(mbps: u32) -> i64 {
    i64::from(mbps) * 1_000_000
}

/// The crop this spec applies to a `comp_w` × `comp_h` frame — the typed
/// insets, or the Viewer's region when *use region of interest* is ticked and a
/// region is set. One engine function decides (`crop_for`), so the reading in
/// the dialogue and the pixels in the file cannot disagree.
fn spec_crop(spec: &BridgeExportSpec, comp_w: u32, comp_h: u32) -> lumit_render::export::Crop {
    let explicit = lumit_render::export::Crop {
        top: spec.crop_top,
        left: spec.crop_left,
        bottom: spec.crop_bottom,
        right: spec.crop_right,
    };
    let region = <[f64; 4]>::try_from(spec.region.as_slice()).ok();
    lumit_render::export::crop_for(
        explicit,
        spec.use_region_of_interest,
        region,
        comp_w,
        comp_h,
    )
}

/// The crop and the frame that survives it, as the dialogue's Crop row reads it.
pub(crate) fn crop_for(spec: &BridgeExportSpec, comp_w: u32, comp_h: u32) -> BridgeCrop {
    let crop = spec_crop(spec, comp_w, comp_h);
    let (width, height) = crop.output_size(comp_w, comp_h);
    BridgeCrop {
        top: crop.top,
        left: crop.left,
        bottom: crop.bottom,
        right: crop.right,
        width,
        height,
    }
}

/// Whatever the engine refuses this spec for, or an empty string.
///
/// `colour` is the project's colour state, so a spec naming one of an OCIO
/// config's own spaces is answered by whether that space can actually be
/// delivered — the config loaded, and the transform to it baked — rather than
/// by a blanket refusal of every OCIO name. A config that has gone missing
/// refuses **by name**, which is the preview-and-delivery asymmetry: the
/// preview degrades to the built-in transform, the delivery does not.
pub(crate) fn spec_check(spec: &BridgeExportSpec, colour: &ColourState) -> String {
    // The crop plays no part in what a format can carry, so the comp's size is
    // not needed to answer this one.
    match to_export_spec(spec, 0, 0) {
        Ok(resolved) => resolved.check_with_colour(colour).err().unwrap_or_default(),
        Err(e) => e,
    }
}

/// The video bitrate this spec runs at, or zero when there is none to choose.
pub(crate) fn resolved_bitrate(spec: &BridgeExportSpec, width: u32, height: u32, fps: f64) -> i64 {
    to_export_spec(spec, 0, 0)
        .ok()
        .and_then(|s| s.resolved_bitrate((width, height), fps))
        .map_or(0, |(target, _)| target)
}

/// Every preset the list shows, built-ins first.
pub(crate) fn preset_list() -> Vec<BridgeExportPresetEntry> {
    lumit_render::export_presets::PresetLibrary::load_default()
        .list()
        .into_iter()
        .map(|(name, read_only)| BridgeExportPresetEntry { name, read_only })
        .collect()
}

/// The settings behind a preset name.
pub(crate) fn preset_get(name: &str) -> Option<BridgeExportSpec> {
    let spec = lumit_render::export_presets::PresetLibrary::load_default().get(name)?;
    let mut bridged = from_export_spec(&spec)?;
    // The row's own name comes back with it, so applying a preset also sets the
    // dropdown that applied it.
    bridged.preset = name.to_owned();
    Some(bridged)
}

/// Save these settings under `name` (replacing a preset of that name in its own
/// row). The library is read and written whole each time: it is a handful of
/// rows in one small file, and a store held in memory would be a second copy to
/// keep in step with the one on disk for no gain.
pub(crate) fn preset_save(name: &str, spec: &BridgeExportSpec) -> Result<(), String> {
    let resolved = to_export_spec(spec, 0, 0)?;
    let mut library = lumit_render::export_presets::PresetLibrary::load_default();
    library.put(name, resolved)?;
    library.save_default()
}

/// What the export dialogue opens on. Read whole each time, for the
/// reason [`preset_save`] gives: five short strings in one small file, and a
/// copy held in memory would be a second thing to keep in step.
pub(crate) fn defaults_get() -> BridgeExportDefaults {
    let stored = lumit_render::export_defaults::ExportDefaults::load_default();
    BridgeExportDefaults {
        preset: stored.preset.clone(),
        codec: stored.codec.clone(),
        // Normalised on the way over, so Dart never has to guess what an answer
        // it does not recognise means.
        destination: stored.policy().to_owned(),
        filename_template: stored.filename_template,
        folder: stored.folder,
    }
}

/// Remember these as what the dialogue opens on.
pub(crate) fn defaults_set(defaults: &BridgeExportDefaults) -> Result<(), String> {
    lumit_render::export_defaults::ExportDefaults {
        preset: defaults.preset.clone(),
        codec: defaults.codec.clone(),
        filename_template: defaults.filename_template.clone(),
        destination: defaults.destination.clone(),
        folder: defaults.folder.clone(),
    }
    .save_default()
}

/// Forget a preset of one's own.
pub(crate) fn preset_delete(name: &str) -> Result<(), String> {
    let mut library = lumit_render::export_presets::PresetLibrary::load_default();
    library.delete(name)?;
    library.save_default()
}

/// The format key an [`ExportFormat`](lumit_render::export::ExportFormat) is
/// named by over the seam, and back again.
#[cfg(feature = "media")]
fn export_format(codec: &str) -> Result<lumit_render::export::ExportFormat, String> {
    use lumit_media::encode::{ImageFormat, VideoCodec};
    use lumit_render::export::{AudioFormat, ExportFormat};
    Ok(match codec {
        "h264" => ExportFormat::Video(VideoCodec::H264),
        #[cfg(target_arch = "wasm32")]
        "vp9" | "av1" => ExportFormat::Video(VideoCodec::H264),
        "hevc" => ExportFormat::Video(VideoCodec::Hevc),
        "prores_proxy" => ExportFormat::Video(VideoCodec::ProRes422Proxy),
        "prores_lt" => ExportFormat::Video(VideoCodec::ProRes422Lt),
        "prores_422" => ExportFormat::Video(VideoCodec::ProRes422),
        "prores_hq" => ExportFormat::Video(VideoCodec::ProRes422Hq),
        "prores_4444" => ExportFormat::Video(VideoCodec::ProRes4444),
        "prores_4444xq" => ExportFormat::Video(VideoCodec::ProRes4444Xq),
        "png" => ExportFormat::Images(ImageFormat::Png),
        "tiff" => ExportFormat::Images(ImageFormat::Tiff),
        "exr" => ExportFormat::Images(ImageFormat::Exr),
        "m4a" => ExportFormat::Audio(AudioFormat::M4a),
        "wav" => ExportFormat::Audio(AudioFormat::Wav),
        "gif" => ExportFormat::Gif,
        other => return Err(format!("export: unknown format '{other}'")),
    })
}

/// The chosen path, ending in its format's extension. FFmpeg picks the
/// container from the file name and the Windows save dialogue hands back a
/// typed name as it is, so a name without the extension would fail to open.
#[cfg(feature = "media")]
pub(crate) fn with_format_extension(out_path: &str, codec: &str) -> String {
    let Ok(format) = export_format(codec) else {
        return out_path.to_owned();
    };
    let ext = format.extension();
    #[cfg(target_arch = "wasm32")]
    let ext = web::extension(codec).unwrap_or(ext);
    // The same test the save dialogue makes, so the two never disagree about
    // a name that is nothing but its extension.
    let has_it = out_path
        .to_lowercase()
        .ends_with(&format!(".{}", ext.to_lowercase()));
    if has_it {
        out_path.to_owned()
    } else {
        format!("{out_path}.{ext}")
    }
}

#[cfg(not(feature = "media"))]
pub(crate) fn with_format_extension(out_path: &str, _codec: &str) -> String {
    out_path.to_owned()
}

#[cfg(feature = "media")]
fn format_key(format: lumit_render::export::ExportFormat) -> &'static str {
    use lumit_media::encode::{ImageFormat, VideoCodec};
    use lumit_render::export::{AudioFormat, ExportFormat};
    match format {
        ExportFormat::Video(VideoCodec::H264) => "h264",
        ExportFormat::Video(VideoCodec::Hevc) => "hevc",
        ExportFormat::Video(VideoCodec::ProRes422Proxy) => "prores_proxy",
        ExportFormat::Video(VideoCodec::ProRes422Lt) => "prores_lt",
        ExportFormat::Video(VideoCodec::ProRes422) => "prores_422",
        ExportFormat::Video(VideoCodec::ProRes422Hq) => "prores_hq",
        ExportFormat::Video(VideoCodec::ProRes4444) => "prores_4444",
        ExportFormat::Video(VideoCodec::ProRes4444Xq) => "prores_4444xq",
        ExportFormat::Images(ImageFormat::Png) => "png",
        ExportFormat::Images(ImageFormat::Tiff) => "tiff",
        ExportFormat::Images(ImageFormat::Exr) => "exr",
        ExportFormat::Audio(AudioFormat::M4a) => "m4a",
        ExportFormat::Audio(AudioFormat::Wav) => "wav",
        ExportFormat::Gif => "gif",
    }
}

/// What one format can carry, as the dialogue reads it. A build with no encoder
/// can carry nothing, which disables every control rather than offering choices
/// no file will honour.
#[cfg(feature = "media")]
pub(crate) fn format_caps(codec: &str) -> BridgeFormatCaps {
    use lumit_media::encode::BitDepth;
    let Ok(format) = export_format(codec) else {
        return BridgeFormatCaps::default();
    };
    #[cfg(target_arch = "wasm32")]
    if !web::offers(codec) {
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
    }
}

#[cfg(not(feature = "media"))]
pub(crate) fn format_caps(_codec: &str) -> BridgeFormatCaps {
    BridgeFormatCaps::default()
}

/// Every sample rate an export can write sound at — the engine's own list, so
/// the Sound row offers what the encoder was actually asked for.
pub(crate) fn audio_rates() -> Vec<u32> {
    lumit_render::export::EXPORT_AUDIO_RATES.to_vec()
}

/// Without a media build there is no encoder to name, so an export cannot be
/// specified at all — a calm error rather than a spec pointing at nothing.
#[cfg(not(feature = "media"))]
pub(crate) fn to_export_spec(
    _spec: &BridgeExportSpec,
    _comp_w: u32,
    _comp_h: u32,
) -> Result<lumit_render::export::ExportSpec, String> {
    Err("export: this build has no encoder (the media feature is off)".to_owned())
}

#[cfg(not(feature = "media"))]
pub(crate) fn from_export_spec(
    _spec: &lumit_render::export::ExportSpec,
) -> Option<BridgeExportSpec> {
    None
}

/// The dialogue's spec as the exporter's own, resolved against the comp's size
/// (which only the crop needs).
#[cfg(feature = "media")]
pub(crate) fn to_export_spec(
    spec: &BridgeExportSpec,
    comp_w: u32,
    comp_h: u32,
) -> Result<lumit_render::export::ExportSpec, String> {
    use lumit_media::encode::{BitDepth, Metadata};
    use lumit_render::export::{
        AlphaMode, AudioDepth, AudioLayout, Bitrate, Channels, ColourSpace, DiskCachePolicy,
        ExportSpec, MotionBlurOverride, RenderOptions, RetimeBlendOverride, WhenDone,
    };
    let default_spec = ExportSpec::default();
    let mut metadata = Metadata::new();
    for field in &spec.metadata {
        metadata.set(&field.key, &field.value);
    }
    Ok(ExportSpec {
        format: export_format(&spec.codec)?,
        target: (spec.width > 0 && spec.height > 0).then_some((spec.width, spec.height)),
        bitrate: match (spec.bitrate_auto, spec.bitrate_mbps) {
            (true, _) => Bitrate::Auto,
            // A blank field sets no bitrate at all and lets the encoder choose
            // its own quality, which is a different answer from Auto.
            (false, 0) => Bitrate::EncoderDefault,
            (false, mbps) => Bitrate::Manual {
                target_bps: bps(mbps),
                peak_bps: (spec.peak_mbps > 0).then(|| bps(spec.peak_mbps)),
            },
        },
        fps: (spec.fps > 0.0).then_some(spec.fps),
        range: (spec.range_start_frame >= 0 && spec.range_end_frame > spec.range_start_frame)
            .then_some((
                spec.range_start_frame as usize,
                spec.range_end_frame as usize,
            )),
        include_audio: spec.include_audio,
        audio_bit_rate: if spec.audio_bit_rate > 0 {
            spec.audio_bit_rate
        } else {
            PRESET_AUDIO_BPS
        },
        // Zero is "whatever Lumit has always written" for all three, so a
        // caller that sets none of them exports the file it always did.
        audio_rate: if spec.audio_rate > 0 {
            spec.audio_rate
        } else {
            default_spec.audio_rate
        },
        audio_depth: if spec.audio_depth >= 24 {
            AudioDepth::TwentyFour
        } else {
            AudioDepth::Sixteen
        },
        audio_layout: if spec.audio_channels == 1 {
            AudioLayout::Mono
        } else {
            AudioLayout::Stereo
        },
        // The number says which depth *within a format*: OpenEXR carries the
        // float pair and nothing else, every other format the integer pair, so
        // 16 means half in one and sixteen-bit codes in the other. A depth a
        // format cannot write is answered with the one it can rather than
        // refused, which is what the caps row already told the dialogue.
        depth: match (export_format(&spec.codec)?, spec.depth) {
            (
                lumit_render::export::ExportFormat::Images(lumit_media::encode::ImageFormat::Exr),
                d,
            ) => {
                if d >= 32 {
                    BitDepth::Float
                } else {
                    BitDepth::Half
                }
            }
            (_, d) if d >= 16 => BitDepth::Sixteen,
            _ => BitDepth::Eight,
        },
        channels: if spec.alpha_channel {
            Channels::RgbAlpha
        } else {
            Channels::Rgb
        },
        alpha: if spec.straight_alpha {
            AlphaMode::Straight
        } else {
            AlphaMode::Premultiplied
        },
        colour_space: ColourSpace::from_stored_name(&spec.colour_space),
        // Anything but "high" is the bilinear default, blank included — an
        // unrecognised filter name must not silently change what is written.
        resample: if spec.resample == "high" {
            lumit_core::pixels::Resample::High
        } else {
            lumit_core::pixels::Resample::Fast
        },
        crop: spec_crop(spec, comp_w, comp_h),
        metadata,
        render: RenderOptions {
            quality: lumit_render::plan::Quality {
                divisor: spec.quality_divisor.clamp(1, 4),
                ..lumit_render::plan::Quality::default()
            },
            disk_cache: if spec.disk_cache_read_only {
                DiskCachePolicy::ReadOnly
            } else {
                DiskCachePolicy::Off
            },
            effects: spec.effects,
            honour_solo: spec.honour_solo,
            render_guides: spec.render_guides,
            // An answer nobody recognises is the composition's own answer:
            // a number out of range must not refuse an export.
            motion_blur: match spec.motion_blur {
                1 => MotionBlurOverride::OnForChecked,
                2 => MotionBlurOverride::OffForAll,
                _ => MotionBlurOverride::CompSetting,
            },
            retime_blend: if spec.retime_blend == 1 {
                RetimeBlendOverride::OffForAll
            } else {
                RetimeBlendOverride::CompSetting
            },
            use_proxies: spec.use_proxies,
        },
        // The two ticks are one enum in the engine, and showing the folder is
        // the louder of the two — the queue honours both flags itself.
        when_done: if spec.open_folder {
            WhenDone::OpenFolder
        } else if spec.make_a_noise {
            WhenDone::MakeANoise
        } else {
            WhenDone::Nothing
        },
        gif: lumit_media::encode::GifOptions {
            dither: match spec.gif_dither {
                1 => lumit_media::encode::GifDither::Pattern,
                2 => lumit_media::encode::GifDither::None,
                _ => lumit_media::encode::GifDither::Diffusion,
            },
            palette_per_frame: spec.gif_palette_per_frame,
            plays: (spec.gif_plays > 0).then_some(spec.gif_plays),
        },
    })
}

/// A stored preset as the dialogue's own fields — the inverse of
/// [`to_export_spec`], so a preset saved from the dialogue comes back as the
/// settings that saved it.
#[cfg(feature = "media")]
pub(crate) fn from_export_spec(
    spec: &lumit_render::export::ExportSpec,
) -> Option<BridgeExportSpec> {
    use lumit_media::encode::BitDepth;
    use lumit_render::export::{
        AlphaMode, Bitrate, Channels, DiskCachePolicy, MotionBlurOverride, RetimeBlendOverride,
    };
    let (bitrate_auto, bitrate_mbps, peak_mbps) = match spec.bitrate {
        Bitrate::Auto => (true, 0, 0),
        Bitrate::EncoderDefault => (false, 0, 0),
        Bitrate::Manual {
            target_bps,
            peak_bps,
        } => (
            false,
            (target_bps / 1_000_000).clamp(0, i64::from(u32::MAX)) as u32,
            peak_bps.map_or(0, |p| (p / 1_000_000).clamp(0, i64::from(u32::MAX)) as u32),
        ),
    };
    let (width, height) = spec.target.unwrap_or((0, 0));
    let (range_start_frame, range_end_frame) =
        spec.range.map_or((-1, -1), |(s, e)| (s as i64, e as i64));
    Some(BridgeExportSpec {
        preset: String::new(),
        codec: format_key(spec.format).to_owned(),
        width,
        height,
        bitrate_mbps,
        peak_mbps,
        bitrate_auto,
        fps: spec.fps.unwrap_or(0.0),
        range_start_frame,
        range_end_frame,
        include_audio: spec.include_audio,
        audio_bit_rate: spec.audio_bit_rate,
        audio_rate: spec.audio_rate,
        audio_depth: spec.audio_depth.bits(),
        audio_channels: u32::from(spec.audio_layout.channels()),
        depth: match spec.depth {
            BitDepth::Eight => 8,
            BitDepth::Sixteen | BitDepth::Half => 16,
            BitDepth::Float => 32,
        },
        alpha_channel: spec.channels == Channels::RgbAlpha,
        straight_alpha: spec.alpha == AlphaMode::Straight,
        colour_space: spec.colour_space.stored_name(),
        resample: match spec.resample {
            lumit_core::pixels::Resample::High => "high".to_owned(),
            lumit_core::pixels::Resample::Fast => String::new(),
        },
        crop_top: spec.crop.top,
        crop_left: spec.crop.left,
        crop_bottom: spec.crop.bottom,
        crop_right: spec.crop.right,
        // A stored crop is already resolved: the region it may have come from
        // was the Viewer's at the time, and that is not what a preset means.
        use_region_of_interest: false,
        region: Vec::new(),
        metadata: spec
            .metadata
            .iter()
            .map(|(key, value)| BridgeMetadataField {
                key: key.to_owned(),
                value: value.to_owned(),
            })
            .collect(),
        quality_divisor: spec.render.quality.divisor.clamp(1, 4),
        disk_cache_read_only: spec.render.disk_cache == DiskCachePolicy::ReadOnly,
        effects: spec.render.effects,
        honour_solo: spec.render.honour_solo,
        render_guides: spec.render.render_guides,
        motion_blur: match spec.render.motion_blur {
            MotionBlurOverride::CompSetting => 0,
            MotionBlurOverride::OnForChecked => 1,
            MotionBlurOverride::OffForAll => 2,
        },
        retime_blend: match spec.render.retime_blend {
            RetimeBlendOverride::CompSetting => 0,
            RetimeBlendOverride::OffForAll => 1,
        },
        use_proxies: spec.render.use_proxies,
        // *When done* is what this export does, not what the preset is for.
        make_a_noise: false,
        open_folder: false,
        gif_dither: match spec.gif.dither {
            lumit_media::encode::GifDither::Diffusion => 0,
            lumit_media::encode::GifDither::Pattern => 1,
            lumit_media::encode::GifDither::None => 2,
        },
        gif_palette_per_frame: spec.gif.palette_per_frame,
        gif_plays: spec.gif.plays.unwrap_or(0),
    })
}

/// The `export_preset` reply: the dialogue fields a preset stamps plus its
/// suggested file name — everything Dart needs to fill the export dialogue for
/// `preset_name`, reproducing `ExportDialogState::apply` exactly. `comp_name`
/// and `template` feed the `{comp}`/`{preset}`/`{date}` filename substitution;
/// a blank template yields the preset's own default file name.
pub(crate) fn export_preset(preset_name: &str, comp_name: &str, template: &str) -> String {
    let stamped = preset_params(preset_name);
    let (codec, size, bitrate_mbps) = match stamped {
        Some(p) => (
            p.codec.to_string(),
            Some(p.size),
            (p.target_bps / 1_000_000).to_string(),
        ),
        None => ("h264".to_string(), None, String::new()),
    };
    let template = if template.trim().is_empty() {
        None
    } else {
        Some(template)
    };
    let default_name = names::export_default_file_name(preset_name, comp_name, template);
    json!({
        "ok": true,
        "preset": preset_name,
        "codec": codec,
        "size": size.map(|(w, h)| json!([w, h])).unwrap_or(Value::Null),
        "bitrate_mbps": bitrate_mbps,
        "include_audio": true,
        "default_name": default_name,
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Driving the export (render feature): start / poll / cancel over the seam.
// ---------------------------------------------------------------------------

/// Start an export of `comp` in `doc` — the frb entry, which brings its own
/// document rather than reading the process-wide v0 bridge.
pub(crate) fn start_export_with_document(
    doc: std::sync::Arc<lumit_core::model::Document>,
    comp: uuid::Uuid,
    spec: &BridgeExportSpec,
    out_path: &str,
) -> String {
    driving::start_with_document(doc, comp, spec, out_path)
}

/// Poll the running export, draining the exporter's event channel. Reply:
/// `{"ok":true,"state":"idle|running|done|failed","frame":…,"total":…,
/// "encoder":…,"path"/"error":…}`. `idle` when nothing has run since start-up.
pub(crate) fn export_poll() -> String {
    driving::poll()
}

/// Ask the running export to cancel (no-op when none is running). The export
/// stops at the next frame and poll then reports `failed` with "cancelled".
pub(crate) fn export_cancel() -> String {
    driving::cancel()
}

/// One row of the export queue, as the interface reads it.
///
/// The document each item renders from was snapshotted when it was added
/// (docs/06 §7.1), so everything here is what was true at *queue* time — the
/// comp's name included, which is why it is stored rather than looked up.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QueueRow {
    pub id: u32,
    /// The composition it was made from, so its settings can be opened again.
    pub comp: uuid::Uuid,
    pub comp_name: String,
    pub out_path: String,
    /// The delivery preset's name, empty for a custom export.
    pub preset: String,
    pub codec: String,
    /// The range this item exports, end exclusive; `None` = the work area at
    /// queue time, else the whole comp.
    pub range: Option<(u64, u64)>,
    pub state: QueueRowState,
}

/// Where one queued item has got to.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum QueueRowState {
    /// Waiting its turn. Nothing starts until the queue is told to run.
    Waiting,
    Running {
        frame: u64,
        total: u64,
        encoder: String,
    },
    /// How long it took in seconds, and the file's size, zero for a sequence.
    Done {
        seconds: u32,
        bytes: u64,
    },
    Failed(String),
}

/// Add an export to the queue, and start the queue when `start` is set.
///
/// The document is snapshotted by the caller, so later edits never alter a
/// queued item. Returns the item's id.
pub(crate) fn queue_add(
    doc: std::sync::Arc<lumit_core::model::Document>,
    comp: uuid::Uuid,
    comp_name: String,
    spec: &BridgeExportSpec,
    out_path: &str,
    start: bool,
) -> Result<u32, String> {
    driving::queue_add(doc, comp, comp_name, spec, out_path, start)
}

/// Show a file in the desktop's own file manager — the *Open folder* tick, and
/// what a finished item with that tick set does for itself.
///
/// Here rather than in the api layer because the queue calls it as an item
/// lands, which happens whether or not any window is still up to notice.
pub(crate) fn reveal_in_folder(path: &str) -> bool {
    let path = std::path::Path::new(path);
    if !path.exists() {
        return false;
    }
    // Quoted by hand. Explorer splits its switch on commas, and an argument is
    // only quoted for it when it holds a space, so a comma in a name with no
    // space in it opened the wrong folder. A Windows path cannot hold a quote.
    #[cfg(target_os = "windows")]
    let launched = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let launched = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    // Everything else opens the containing folder: `xdg-open` has no way to
    // ask for one file inside it, and an open folder is the point.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let launched = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
    launched.is_ok()
}

/// Every queued item, oldest first — and the pump that starts the next one
/// when the queue is running and nothing is in flight.
pub(crate) fn queue_list() -> Vec<QueueRow> {
    driving::queue_list()
}

/// Let the queue run: the next waiting item starts, and each one after it
/// follows as the one before finishes.
pub(crate) fn queue_start() {
    driving::queue_start();
}

/// Let the export in flight finish and start nothing after it, until the
/// queue is told to run again.
pub(crate) fn queue_stop_after() {
    driving::queue_stop_after();
}

/// Whether the queue starts the next waiting item when the one in flight
/// lands.
pub(crate) fn queue_goes_on() -> bool {
    driving::queue_goes_on()
}

/// Cancel one item — the running export stops at its next frame, a waiting one
/// simply never starts.
pub(crate) fn queue_cancel(id: u32) {
    driving::queue_cancel(id);
}

/// Forget one item. A running item is cancelled first: a row cannot leave the
/// list while the encoder it named is still writing.
pub(crate) fn queue_remove(id: u32) {
    driving::queue_remove(id);
}

/// Move one waiting item to `index`, oldest-first, so the order the list is
/// read in is the order the work runs in.
///
/// Transient state, like removal: the queue is not in the `.lum`, so this
/// commits no op and there is nothing to undo. Refuses an item that is running
/// or has already run, in the engine's own words.
pub(crate) fn queue_move(id: u32, index: usize) -> Result<(), String> {
    driving::queue_move(id, index)
}

/// The settings one item was queued with, or `None` once it has gone.
pub(crate) fn queue_spec(id: u32) -> Option<BridgeExportSpec> {
    driving::queue_spec(id)
}

/// Give a waiting item new settings and a new file. The snapshot it renders
/// stays the one taken when it was added. Refuses the same items a move does.
pub(crate) fn queue_update(id: u32, spec: &BridgeExportSpec, out_path: &str) -> Result<(), String> {
    driving::queue_update(id, spec, out_path)
}

/// The first file an export to `path` writes: the file itself, or frame one
/// of an image sequence, which is the one to ask about for the whole run.
fn first_file(path: &str, codec: &str) -> std::path::PathBuf {
    #[cfg(feature = "media")]
    if let Ok(lumit_render::export::ExportFormat::Images(_)) = export_format(codec) {
        return lumit_media::encode::sequence_frame_path(std::path::Path::new(path), 1);
    }
    let _ = codec;
    std::path::PathBuf::from(path)
}

/// Whether an export to `out_path` would write over something: a file already
/// there, or an export still waiting or running in the queue under that name.
pub(crate) fn path_taken(out_path: &str, codec: &str) -> bool {
    let path = with_format_extension(out_path, codec);
    first_file(&path, codec).exists()
        || queue_list().iter().any(|row| {
            row.out_path == path
                && matches!(
                    row.state,
                    QueueRowState::Waiting | QueueRowState::Running { .. }
                )
        })
}

/// The first of `name_2.ext`, `name_3.ext`, … that writes over nothing, for
/// Keep both. `out_path` itself when every number up to the cap is taken.
pub(crate) fn free_path(out_path: &str, codec: &str) -> String {
    let path = with_format_extension(out_path, codec);
    let file = std::path::Path::new(&path);
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("export");
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    (2..10_000)
        .map(|n| file.with_file_name(format!("{stem}_{n}{ext}")))
        .map(|p| p.to_string_lossy().into_owned())
        .find(|p| !path_taken(p, codec))
        .unwrap_or(path)
}

mod driving;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
