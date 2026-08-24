//! Export from Flutter — resolving the export spec (preset stamp, VBR-peak and
//! filename rules) and driving the lumit-ui exporter over the headless seam.
//!
//! # In plain terms
//!
//! Exporting writes the composited comp to an `.mp4` on its own thread, exactly
//! as the egui frontend does (K-017). The bridge reuses the identical exporter
//! (`lumit_render::export`) through the headless seam (K-175): the seam builds the
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
//!   bitrate it will run at (K-479, K-485);
//! - the **filename template** — `{comp}`/`{preset}`/`{date}` substitution, the
//!   Windows sanitiser and the `.mp4` guarantee, a faithful port of
//!   `shell::export_default_file_name`/`render_filename_template`/
//!   `sanitise_windows_filename`. A blank template reproduces each preset's own
//!   default file name byte-for-byte (K-119, load-bearing).
//!
//! Naming a codec needs the `media` feature; without it the conversion answers
//! a calm "this build has no encoder" and every capability reads false, so the
//! dialogue disables rather than offering choices no file would honour.

use crate::err_json;
use serde_json::{json, Value};

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
/// byte-for-byte fallback when no filename template is set (K-119).
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
// The seam's own half of the export spec (K-485): `BridgeExportSpec` in, the
// engine's `ExportSpec` out, and the dialogue's four questions — what can this
// format carry, is this spec exportable, what does the crop leave, what bitrate
// will it run at — answered by the engine rather than re-derived in Dart.
// ---------------------------------------------------------------------------

use crate::api::export::{BridgeCrop, BridgeExportPresetEntry, BridgeExportSpec, BridgeFormatCaps};
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
pub(crate) fn spec_check(spec: &BridgeExportSpec) -> String {
    // The crop plays no part in what a format can carry, so the comp's size is
    // not needed to answer this one.
    match to_export_spec(spec, 0, 0) {
        Ok(resolved) => resolved.check().err().unwrap_or_default(),
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
        "hevc" => ExportFormat::Video(VideoCodec::Hevc),
        "png" => ExportFormat::Images(ImageFormat::Png),
        "tiff" => ExportFormat::Images(ImageFormat::Tiff),
        "m4a" => ExportFormat::Audio(AudioFormat::M4a),
        "wav" => ExportFormat::Audio(AudioFormat::Wav),
        other => return Err(format!("export: unknown format '{other}'")),
    })
}

#[cfg(feature = "media")]
fn format_key(format: lumit_render::export::ExportFormat) -> &'static str {
    use lumit_media::encode::{ImageFormat, VideoCodec};
    use lumit_render::export::{AudioFormat, ExportFormat};
    match format {
        ExportFormat::Video(VideoCodec::H264) => "h264",
        ExportFormat::Video(VideoCodec::Hevc) => "hevc",
        ExportFormat::Images(ImageFormat::Png) => "png",
        ExportFormat::Images(ImageFormat::Tiff) => "tiff",
        ExportFormat::Audio(AudioFormat::M4a) => "m4a",
        ExportFormat::Audio(AudioFormat::Wav) => "wav",
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
    let caps = format.caps();
    BridgeFormatCaps {
        video: caps.video,
        audio: caps.audio,
        alpha: caps.alpha,
        depths: caps
            .depths
            .iter()
            .map(|d| match d {
                BitDepth::Eight => 8,
                BitDepth::Sixteen => 16,
            })
            .collect(),
        bit_rate: caps.bit_rate,
        // Uncompressed PCM is exactly what it is; everything else Lumit writes
        // sound into is AAC, which has a rate to choose.
        audio_bit_rate: caps.audio && codec != "wav",
        metadata: caps.metadata,
    }
}

#[cfg(not(feature = "media"))]
pub(crate) fn format_caps(_codec: &str) -> BridgeFormatCaps {
    BridgeFormatCaps::default()
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
fn from_export_spec(_spec: &lumit_render::export::ExportSpec) -> Option<BridgeExportSpec> {
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
        AlphaMode, Bitrate, Channels, ColourSpace, DiskCachePolicy, ExportSpec, RenderOptions,
        WhenDone,
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
            // its own quality, which is a different answer from Auto (K-479).
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
        // The sound rate, width and layout are real in the engine and are not
        // on the seam yet, so they take today's answer — 48 kHz, sixteen bits,
        // stereo — exactly as K-479 left the fields it had not yet exposed.
        // One line to change when the dialogue's three faces come alive.
        audio_rate: default_spec.audio_rate,
        audio_depth: default_spec.audio_depth,
        audio_layout: default_spec.audio_layout,
        depth: if spec.depth >= 16 {
            BitDepth::Sixteen
        } else {
            BitDepth::Eight
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
        // The seam has no resample face yet: the drawing's Resize row still
        // shows one filter, so the engine's own default — bilinear, what every
        // export has always used — stands until the dialog carries the choice.
        resample: lumit_core::pixels::Resample::default(),
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
    })
}

/// A stored preset as the dialogue's own fields — the inverse of
/// [`to_export_spec`], so a preset saved from the dialogue comes back as the
/// settings that saved it.
#[cfg(feature = "media")]
fn from_export_spec(spec: &lumit_render::export::ExportSpec) -> Option<BridgeExportSpec> {
    use lumit_media::encode::BitDepth;
    use lumit_render::export::{AlphaMode, Bitrate, Channels, ColourSpace, DiskCachePolicy};
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
        depth: match spec.depth {
            BitDepth::Eight => 8,
            BitDepth::Sixteen => 16,
        },
        alpha_channel: spec.channels == Channels::RgbAlpha,
        straight_alpha: spec.alpha == AlphaMode::Straight,
        colour_space: spec.colour_space.stored_name(),
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
        // *When done* is what this export does, not what the preset is for.
        make_a_noise: false,
        open_folder: false,
    })
}

/// The `export_preset` reply: the dialogue fields a preset stamps plus its
/// suggested file name — everything Dart needs to fill the export dialogue for
/// `preset_name`, reproducing `ExportDialogState::apply` exactly. `comp_name`
/// and `template` feed the `{comp}`/`{preset}`/`{date}` filename substitution
/// (K-119); a blank template yields the preset's own default file name.
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
    let default_name = export_default_file_name(preset_name, comp_name, template);
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

/// The suggested file name for `preset` (K-119) — a faithful port of
/// `shell::export_default_file_name`: with no (or a blank) template, the preset's
/// own default file name byte-for-byte; otherwise the template with `{comp}`/
/// `{preset}`/`{date}` substituted, sanitised, and forced to end in `.mp4`.
fn export_default_file_name(preset: &str, comp_name: &str, template: Option<&str>) -> String {
    match template.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => {
            let stem = preset_default_file_name(preset).trim_end_matches(".mp4");
            render_filename_template(t, comp_name, stem)
        }
        None => preset_default_file_name(preset).to_string(),
    }
}

/// Substitute `{comp}`/`{preset}`/`{date}` in a filename template (K-119),
/// sanitise against characters Windows forbids, and guarantee a `.mp4` suffix —
/// a faithful port of `shell::render_filename_template`.
fn render_filename_template(template: &str, comp_name: &str, preset_stem: &str) -> String {
    let date = today_utc_date();
    let substituted = template
        .replace("{comp}", comp_name)
        .replace("{preset}", preset_stem)
        .replace("{date}", &date);
    let mut name = sanitise_windows_filename(&substituted);
    if !name.to_ascii_lowercase().ends_with(".mp4") {
        name.push_str(".mp4");
    }
    name
}

/// Replace characters illegal in a Windows file name (and control characters)
/// with `_`, falling back to `export` if nothing usable remains — a faithful
/// port of `shell::sanitise_windows_filename`.
fn sanitise_windows_filename(raw: &str) -> String {
    const ILLEGAL: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if ILLEGAL.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "export".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Today's UTC date as `YYYY-MM-DD` (K-119's `{date}` token) — a faithful port
/// of `shell::today_utc_date` and its `civil_from_days`.
fn today_utc_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since the Unix epoch → (year, month, day), proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Driving the export (render feature): start / poll / cancel over the seam.
// ---------------------------------------------------------------------------

/// Start an export of `comp` in `doc` — the frb entry, which brings its own
/// document rather than reading the process-wide v0 bridge.
pub(crate) fn start_export_with_document(
    doc: std::sync::Arc<lumit_core::Document>,
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
    Done,
    Failed(String),
}

/// Add an export to the queue, and start the queue when `start` is set.
///
/// The document is snapshotted by the caller, so later edits never alter a
/// queued item. Returns the item's id.
pub(crate) fn queue_add(
    doc: std::sync::Arc<lumit_core::Document>,
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
    #[cfg(target_os = "windows")]
    let launched = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
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

mod driving {
    use super::{err_json, to_export_spec, BridgeExportSpec};
    use lumit_render::export::{ExportEvent, ExportHandle};
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};
    use uuid::Uuid;

    /// The terminal/progress state a poll reports, held between polls.
    enum State {
        Idle,
        Running {
            frame: usize,
            total: usize,
            encoder: Option<String>,
        },
        Done {
            path: String,
        },
        Failed {
            error: String,
        },
    }

    /// One item waiting its turn. The document was snapshotted when it was
    /// added (docs/06 §7.1), so later edits never alter what it renders.
    struct Item {
        id: u32,
        comp_name: String,
        doc: std::sync::Arc<lumit_core::Document>,
        comp: Uuid,
        /// The dialogue's fields as they stood when this was queued — the
        /// *when done* ticks included, which is why the queue can honour them
        /// after the dialogue that set them has closed.
        spec: BridgeExportSpec,
        out_path: String,
        state: ItemState,
    }

    /// Where an item is in its life. Progress is not held here: the in-flight
    /// item's frame count is [`State::Running`], one place rather than two that
    /// could disagree.
    enum ItemState {
        Waiting,
        Running,
        Done,
        Failed(String),
    }

    /// The one in-flight export: its state plus the handle whose receiver a poll
    /// drains. The handle is dropped once a terminal event arrives.
    ///
    /// The queue lives here too rather than in a slot of its own, because "is
    /// anything running" is the question both halves ask: one lock answers it
    /// for the export in flight and for the items waiting behind it.
    struct Run {
        state: State,
        handle: Option<ExportHandle>,
        queue: Vec<Item>,
        next_id: u32,
        /// Whether the queue may start things. Adding holds by default —
        /// "Add to queue" is a list of work, not a start button — and Export
        /// (and the queue window's own action) sets it.
        running: bool,
        /// The queued item the handle belongs to, when the export in flight
        /// came from the queue rather than from a direct start.
        current: Option<u32>,
    }

    static EXPORT: OnceLock<Mutex<Run>> = OnceLock::new();

    fn slot() -> &'static Mutex<Run> {
        EXPORT.get_or_init(|| {
            Mutex::new(Run {
                state: State::Idle,
                handle: None,
                queue: Vec::new(),
                next_id: 1,
                running: false,
                current: None,
            })
        })
    }

    /// The export itself, given the document to render.
    ///
    /// Split out from [`start`] so the frb path can drive the same exporter: v0
    /// reads its document from the process-wide bridge, and an frb project is
    /// not in it. Everything after this point is shared.
    pub(super) fn start_with_document(
        doc: std::sync::Arc<lumit_core::Document>,
        comp: Uuid,
        spec: &BridgeExportSpec,
        out_path: &str,
    ) -> String {
        if out_path.trim().is_empty() {
            return err_json("export: no output path");
        }

        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        // Drain first so a just-finished export frees the slot for a new one.
        drain(&mut guard);
        if matches!(guard.state, State::Running { .. }) {
            return err_json("an export is already running");
        }

        match launch(&mut guard, &doc, comp, spec, out_path) {
            Ok(()) => {
                guard.current = None;
                json!({ "ok": true }).to_string()
            }
            Err(e) => err_json(e),
        }
    }

    /// Start one export against the slot: resolve its spec, build its inputs
    /// over the headless seam, and hand it to the exporter. The caller has
    /// already established that nothing is in flight.
    ///
    /// Split out so the queue and a direct start cannot drift apart — there is
    /// one way to begin an export, whoever asked for it.
    fn launch(
        run: &mut Run,
        doc: &std::sync::Arc<lumit_core::Document>,
        comp: Uuid,
        spec: &BridgeExportSpec,
        out_path: &str,
    ) -> Result<(), String> {
        // The comp's own size, which the crop is resolved against.
        let (cw, ch) = doc
            .comp(comp)
            .map(|c| (c.width, c.height))
            .ok_or("export: unknown composition")?;
        let spec = to_export_spec(spec, cw, ch)?;

        // Build the audio inputs through the headless seam (K-175), then hand
        // off to the exporter, which drives the same render walk the Viewer
        // uses on its own thread and device (K-017, K-031).
        let inputs = crate::render::with_export_inputs(doc, comp)
            .ok_or("export: the GPU pipeline is unavailable")?;
        let handle = lumit_render::export::start(
            doc.clone(),
            comp,
            inputs.audio,
            std::path::PathBuf::from(out_path),
            spec,
        );

        run.state = State::Running {
            frame: 0,
            total: 0,
            encoder: None,
        };
        run.handle = Some(handle);
        Ok(())
    }

    pub(super) fn queue_add(
        doc: std::sync::Arc<lumit_core::Document>,
        comp: Uuid,
        comp_name: String,
        spec: &BridgeExportSpec,
        out_path: &str,
        start: bool,
    ) -> Result<u32, String> {
        if out_path.trim().is_empty() {
            return Err("export: no output path".to_owned());
        }

        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        let id = guard.next_id;
        guard.next_id += 1;
        guard.queue.push(Item {
            id,
            comp_name,
            doc,
            comp,
            spec: spec.clone(),
            out_path: out_path.to_owned(),
            state: ItemState::Waiting,
        });
        if start {
            guard.running = true;
        }
        pump(&mut guard);
        Ok(id)
    }

    pub(super) fn queue_list() -> Vec<super::QueueRow> {
        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        drain(&mut guard);
        pump(&mut guard);
        let running = match &guard.state {
            State::Running {
                frame,
                total,
                encoder,
            } => Some((*frame as u64, *total as u64, encoder.clone())),
            _ => None,
        };
        guard
            .queue
            .iter()
            .map(|item| super::QueueRow {
                id: item.id,
                comp_name: item.comp_name.clone(),
                out_path: item.out_path.clone(),
                preset: item.spec.preset.clone(),
                codec: item.spec.codec.clone(),
                range: (item.spec.range_start_frame >= 0
                    && item.spec.range_end_frame > item.spec.range_start_frame)
                    .then_some((
                        item.spec.range_start_frame as u64,
                        item.spec.range_end_frame as u64,
                    )),
                state: match &item.state {
                    ItemState::Waiting => super::QueueRowState::Waiting,
                    ItemState::Running => {
                        let (frame, total, encoder) = running.clone().unwrap_or_default();
                        super::QueueRowState::Running {
                            frame,
                            total,
                            encoder: encoder.unwrap_or_default(),
                        }
                    }
                    ItemState::Done => super::QueueRowState::Done,
                    ItemState::Failed(error) => super::QueueRowState::Failed(error.clone()),
                },
            })
            .collect()
    }

    pub(super) fn queue_start() {
        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        guard.running = true;
        drain(&mut guard);
        pump(&mut guard);
    }

    pub(super) fn queue_cancel(id: u32) {
        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        if guard.current == Some(id) {
            if let Some(handle) = &guard.handle {
                handle.cancel();
            }
            return;
        }
        // A waiting item has nothing to stop and nothing to report, so it
        // simply leaves the list rather than sitting there as a failure.
        guard
            .queue
            .retain(|item| item.id != id || !matches!(item.state, ItemState::Waiting));
    }

    pub(super) fn queue_remove(id: u32) {
        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        if guard.current == Some(id) {
            if let Some(handle) = &guard.handle {
                handle.cancel();
            }
            guard.current = None;
        }
        guard.queue.retain(|item| item.id != id);
    }

    /// Start the next waiting item when the queue is running and nothing is in
    /// flight. An item that cannot start at all is marked failed and the one
    /// behind it is tried, so a bad path cannot stall the whole queue.
    ///
    /// The pump is turned by the interface's own polling rather than by a
    /// thread of its own: every caller here already holds the lock, and the
    /// queue moves on the same 250 ms tick the progress does.
    fn pump(run: &mut Run) {
        while run.handle.is_none() && run.running {
            let Some(index) = run
                .queue
                .iter()
                .position(|item| matches!(item.state, ItemState::Waiting))
            else {
                return;
            };
            let (doc, comp, spec, out_path, id) = {
                let item = &run.queue[index];
                (
                    item.doc.clone(),
                    item.comp,
                    item.spec.clone(),
                    item.out_path.clone(),
                    item.id,
                )
            };
            match launch(run, &doc, comp, &spec, &out_path) {
                Ok(()) => {
                    run.queue[index].state = ItemState::Running;
                    run.current = Some(id);
                }
                Err(error) => run.queue[index].state = ItemState::Failed(error),
            }
        }
    }

    pub(super) fn poll() -> String {
        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        drain(&mut guard);
        match &guard.state {
            State::Idle => json!({ "ok": true, "state": "idle" }).to_string(),
            State::Running {
                frame,
                total,
                encoder,
            } => json!({
                "ok": true,
                "state": "running",
                "frame": frame,
                "total": total,
                "encoder": encoder,
            })
            .to_string(),
            State::Done { path } => {
                json!({ "ok": true, "state": "done", "path": path }).to_string()
            }
            State::Failed { error } => {
                json!({ "ok": true, "state": "failed", "error": error }).to_string()
            }
        }
    }

    pub(super) fn cancel() -> String {
        let guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(handle) = &guard.handle {
            handle.cancel();
        }
        json!({ "ok": true }).to_string()
    }

    /// Drain every pending exporter event into the held state. A terminal event
    /// (Done/Failed) drops the handle so the slot is free for the next export.
    fn drain(run: &mut Run) {
        let Some(handle) = &run.handle else {
            return;
        };
        let mut terminal: Option<State> = None;
        while let Ok(ev) = handle.events.try_recv() {
            match ev {
                ExportEvent::Encoder(label) => {
                    if let State::Running { encoder, .. } = &mut run.state {
                        *encoder = Some(label.to_string());
                    }
                }
                ExportEvent::Progress { frame, total } => {
                    if let State::Running {
                        frame: f, total: t, ..
                    } = &mut run.state
                    {
                        *f = frame;
                        *t = total;
                    }
                }
                ExportEvent::Done(path) => {
                    terminal = Some(State::Done {
                        path: path.to_string_lossy().into_owned(),
                    });
                }
                ExportEvent::Failed(error) => {
                    terminal = Some(State::Failed { error });
                }
            }
        }
        if let Some(state) = terminal {
            // The queued item this handle belonged to takes the same outcome:
            // the row in the queue window and the progress line in the status
            // strip are two readings of one export, never two answers.
            if let Some(id) = run.current.take() {
                if let Some(item) = run.queue.iter_mut().find(|item| item.id == id) {
                    item.state = match &state {
                        State::Failed { error } => ItemState::Failed(error.clone()),
                        _ => ItemState::Done,
                    };
                    // The ticks the dialogue set, honoured here rather than by
                    // whatever window happens to be watching: an export that
                    // lands after its dialogue closed still makes its noise and
                    // opens its folder. Both are independent — a long export
                    // left running wants the sound *and* the folder — and the
                    // noise is silent, never an error, when no sound has been
                    // supplied.
                    if matches!(item.state, ItemState::Done) {
                        if item.spec.make_a_noise {
                            lumit_render::export::play_done_sound();
                        }
                        if item.spec.open_folder {
                            super::reveal_in_folder(&item.out_path);
                        }
                    }
                }
            }
            run.state = state;
            run.handle = None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A zero size, a zero rate and a negative range all mean "the
    /// composition's own", which is what the dialogue's untouched fields say
    /// (K-201) — never a frame of 0×0 or a range of nothing.
    #[test]
    #[cfg(feature = "media")]
    fn the_untouched_fields_mean_the_composition_s_own() {
        let resolved = to_export_spec(&BridgeExportSpec::default(), 640, 360).unwrap();
        assert_eq!(resolved.target, None, "zero size is the comp's own frame");
        assert_eq!(resolved.fps, None, "zero rate is the comp's own");
        assert_eq!(resolved.range, None, "a negative range is the work area");

        let spec = BridgeExportSpec {
            width: 1920,
            height: 1080,
            fps: 29.97,
            range_start_frame: 12,
            range_end_frame: 48,
            ..BridgeExportSpec::default()
        };
        let resolved = to_export_spec(&spec, 640, 360).unwrap();
        assert_eq!(resolved.target, Some((1920, 1080)));
        assert_eq!(resolved.fps, Some(29.97));
        assert_eq!(resolved.range, Some((12, 48)));

        // A backwards range is no range rather than a clamped one: it is a slip
        // of the fingers, not an instruction.
        let backwards = BridgeExportSpec {
            range_start_frame: 48,
            range_end_frame: 12,
            ..BridgeExportSpec::default()
        };
        assert_eq!(to_export_spec(&backwards, 640, 360).unwrap().range, None);
    }

    /// *Auto* and a blank field are two answers, not one (K-479): Auto works a
    /// rate out from the frame, a blank field sets none at all, and a typed one
    /// keeps its own peak when a preset gave it one.
    #[test]
    #[cfg(feature = "media")]
    fn auto_a_blank_field_and_a_typed_rate_are_three_answers() {
        use lumit_render::export::Bitrate;
        let auto = BridgeExportSpec::default();
        assert_eq!(
            to_export_spec(&auto, 64, 36).unwrap().bitrate,
            Bitrate::Auto
        );

        let blank = BridgeExportSpec {
            bitrate_auto: false,
            ..BridgeExportSpec::default()
        };
        assert_eq!(
            to_export_spec(&blank, 64, 36).unwrap().bitrate,
            Bitrate::EncoderDefault,
            "a blank field lets the encoder choose its own quality (K-119)"
        );

        let typed = BridgeExportSpec {
            bitrate_auto: false,
            bitrate_mbps: 25,
            peak_mbps: 35,
            ..BridgeExportSpec::default()
        };
        assert_eq!(
            to_export_spec(&typed, 64, 36).unwrap().bitrate,
            Bitrate::Manual {
                target_bps: 25_000_000,
                peak_bps: Some(35_000_000),
            },
            "a preset's own peak crosses rather than being re-derived at 1.5×"
        );

        // And the footer's number is the engine's, not the dialogue's: Auto
        // works out a rate for the frame it is actually writing.
        assert!(resolved_bitrate(&auto, 1920, 1080, 60.0) > 0);
        assert_eq!(
            resolved_bitrate(&blank, 1920, 1080, 60.0),
            0,
            "a quality nobody chose is a size nobody can estimate"
        );
    }

    /// The capability table is the engine's, read through the seam: an mp4
    /// carries sound and a bitrate but no alpha and only eight bits; a PNG
    /// sequence carries alpha and either depth but no sound; a WAV carries
    /// neither picture nor a rate to choose.
    #[test]
    #[cfg(feature = "media")]
    fn every_format_answers_for_what_it_can_carry() {
        let mp4 = format_caps("h264");
        assert!(mp4.video && mp4.audio && mp4.bit_rate && mp4.metadata);
        assert!(!mp4.alpha, "no v1 codec in an mp4 carries alpha (K-479)");
        assert_eq!(mp4.depths, vec![8]);

        let png = format_caps("png");
        assert!(png.video && png.alpha);
        assert!(!png.audio && !png.bit_rate && !png.metadata);
        assert_eq!(png.depths, vec![8, 16]);

        let wav = format_caps("wav");
        assert!(wav.audio && !wav.video);
        assert!(!wav.audio_bit_rate, "uncompressed PCM has no rate to pick");
        assert!(format_caps("m4a").audio_bit_rate, "AAC does");

        assert_eq!(
            format_caps("nonsense"),
            BridgeFormatCaps::default(),
            "an unknown format carries nothing rather than everything"
        );
    }

    /// A spec the format cannot honour is refused in the engine's own words,
    /// before anything is queued — and an exportable one says nothing.
    #[test]
    #[cfg(feature = "media")]
    fn a_format_refuses_what_it_cannot_carry() {
        assert_eq!(spec_check(&BridgeExportSpec::default()), "");

        let deep_mp4 = BridgeExportSpec {
            depth: 16,
            ..BridgeExportSpec::default()
        };
        assert!(
            spec_check(&deep_mp4).contains("16-bit"),
            "an mp4 says it cannot carry 16-bit colour rather than writing 8"
        );

        let alpha_mp4 = BridgeExportSpec {
            alpha_channel: true,
            ..BridgeExportSpec::default()
        };
        assert!(spec_check(&alpha_mp4).contains("alpha"));

        // The same two settings in a PNG sequence are perfectly ordinary.
        let stills = BridgeExportSpec {
            codec: "png".to_owned(),
            depth: 16,
            alpha_channel: true,
            ..BridgeExportSpec::default()
        };
        assert_eq!(spec_check(&stills), "");
    }

    /// The crop is the typed insets, or the Viewer's region when that is asked
    /// for — and the reading is the frame that survives it (K-362, K-419).
    #[test]
    fn the_crop_answers_the_typed_insets_or_the_region() {
        let typed = BridgeExportSpec {
            crop_top: 10,
            crop_left: 20,
            crop_bottom: 30,
            crop_right: 40,
            ..BridgeExportSpec::default()
        };
        let crop = crop_for(&typed, 1000, 500);
        assert_eq!(
            (crop.top, crop.left, crop.bottom, crop.right),
            (10, 20, 30, 40)
        );
        assert_eq!((crop.width, crop.height), (940, 460));

        // The region wins when it is ticked and set.
        let region = BridgeExportSpec {
            use_region_of_interest: true,
            region: vec![0.25, 0.0, 0.75, 1.0],
            ..typed.clone()
        };
        let crop = crop_for(&region, 1000, 500);
        assert_eq!((crop.left, crop.right), (250, 250));
        assert_eq!((crop.width, crop.height), (500, 500));

        // A degenerate region is no region, and the typed crop stands.
        let degenerate = BridgeExportSpec {
            region: vec![0.5, 0.5, 0.5, 0.5],
            ..region
        };
        assert_eq!(crop_for(&degenerate, 1000, 500).left, 20);
    }

    /// A preset saved from the dialogue comes back as the settings that saved
    /// it — every field, not the eight the seam used to carry. The engine's own
    /// store is exercised by `export_presets`; this is the conversion round
    /// trip either side of it.
    #[test]
    #[cfg(feature = "media")]
    fn a_spec_survives_the_round_trip_through_the_engine_s_own() {
        let spec = BridgeExportSpec {
            preset: String::new(),
            codec: "tiff".to_owned(),
            width: 1280,
            height: 720,
            bitrate_mbps: 0,
            peak_mbps: 0,
            bitrate_auto: false,
            fps: 24.0,
            range_start_frame: 5,
            range_end_frame: 25,
            include_audio: false,
            audio_bit_rate: 192_000,
            depth: 16,
            alpha_channel: true,
            straight_alpha: true,
            colour_space: String::new(),
            crop_top: 1,
            crop_left: 2,
            crop_bottom: 3,
            crop_right: 4,
            use_region_of_interest: false,
            region: Vec::new(),
            metadata: vec![
                BridgeMetadataField {
                    key: "title".to_owned(),
                    value: "Scene 1".to_owned(),
                },
                BridgeMetadataField {
                    key: "artist".to_owned(),
                    value: "Nobody".to_owned(),
                },
            ],
            quality_divisor: 2,
            disk_cache_read_only: true,
            effects: false,
            honour_solo: false,
            make_a_noise: false,
            open_folder: false,
        };
        let resolved = to_export_spec(&spec, 1920, 1080).unwrap();
        let back = from_export_spec(&resolved).unwrap();
        assert_eq!(back, spec);
        assert_eq!(
            resolved.metadata.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            ["title", "artist"],
            "metadata keeps the order it was given — the order lands in the file"
        );
    }
}
