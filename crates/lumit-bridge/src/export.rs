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
//! - the **spec resolver** — the preset stamp plus the VBR-peak-preserved-while-
//!   unedited rule and the 1.5× peak fallback, a faithful port of
//!   `ExportDialogState::apply`/`spec`;
//! - the **filename template** — `{comp}`/`{preset}`/`{date}` substitution, the
//!   Windows sanitiser and the `.mp4` guarantee, a faithful port of
//!   `shell::export_default_file_name`/`render_filename_template`/
//!   `sanitise_windows_filename`. A blank template reproduces each preset's own
//!   default file name byte-for-byte (K-119, load-bearing).
//!
//! The driving surface (start/poll/cancel) is gated behind the `render` feature;
//! without it the pure resolver and filename endpoints still work, and starting
//! an export answers a calm "unavailable in this build".

// The spec resolver and its `ResolvedSpec`/`SpecInputs`/parse/resolve helpers are
// always compiled and unit-tested (unconditionally), but only *wired* into the
// export driver under the `render` feature. Without it — and outside the test
// build — they are dead, so silence the warning there rather than gate the code
// (the tests must run in every feature configuration).

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

/// The resolved export spec — the bridge's pure mirror of
/// `lumit_render::export::ExportSpec` (codec as a name string). Produced by
/// [`resolve_spec`] and, under the `render` feature, converted into the real
/// `ExportSpec` the exporter runs with.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct ResolvedSpec {
    /// `h264` / `hevc` for mp4, `png` / `tiff` for an image sequence (K-201).
    pub codec: String,
    pub target: (u32, u32),
    pub bit_rate: Option<i64>,
    pub max_rate: Option<i64>,
    /// Output frame rate; None = the comp's own.
    pub fps: Option<f64>,
    /// Export range in comp frames, end exclusive; None = work area / whole comp.
    pub range: Option<(usize, usize)>,
    pub include_audio: bool,
    pub audio_bit_rate: i64,
}

/// Whether a codec name writes stills rather than a video container — the one
/// question several rules below hang off (no audio, no bitrates).
pub(crate) fn is_image_codec(codec: &str) -> bool {
    matches!(codec, "png" | "tiff")
}

/// The dialogue-shaped inputs a `start_export` spec_json carries — the final
/// state of the egui export dialogue's fields, so [`resolve_spec`] can reproduce
/// `ExportDialogState::spec` exactly.
struct SpecInputs {
    preset: String,
    codec: String,
    size: Option<(u32, u32)>,
    bitrate_mbps: String,
    /// Output rate; zero or absent = the comp's own.
    fps: f64,
    /// `[start, end)` comp frames; absent/null = work area / whole comp.
    range: Option<(u64, u64)>,
    include_audio: bool,
    audio_bit_rate: i64,
}

/// Parse the spec_json into [`SpecInputs`]. Every field is optional and falls to
/// the dialogue's own defaults: no preset ("custom"), H.264, the comp's own size
/// (`size` absent/null), the encoder's default quality (`bitrate_mbps` blank),
/// audio on, and the delivery-preset audio rate. `bitrate_mbps` accepts a string
/// (the raw dialogue field) or a number, so Dart can send either.
fn parse_inputs(spec_json: &str) -> Result<SpecInputs, String> {
    let v: Value =
        serde_json::from_str(spec_json).map_err(|_| "spec must be a JSON object".to_string())?;
    let Value::Object(m) = v else {
        return Err("spec must be a JSON object".to_string());
    };
    let preset = m
        .get("preset")
        .and_then(|p| p.as_str())
        .unwrap_or("custom")
        .to_owned();
    let codec = m
        .get("codec")
        .and_then(|c| c.as_str())
        .unwrap_or("h264")
        .to_owned();
    // `size`: an explicit [w, h], or null/absent for the comp's own size.
    let size = match m.get("size") {
        Some(Value::Array(a)) if a.len() == 2 => match (a[0].as_u64(), a[1].as_u64()) {
            (Some(w), Some(h)) => Some((w as u32, h as u32)),
            _ => None,
        },
        _ => None,
    };
    let bitrate_mbps = match m.get("bitrate_mbps") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let fps = m.get("fps").and_then(|f| f.as_f64()).unwrap_or(0.0);
    // `range`: an explicit [start, end) in comp frames, or null/absent for the
    // work-area default.
    let range = match m.get("range") {
        Some(Value::Array(a)) if a.len() == 2 => match (a[0].as_u64(), a[1].as_u64()) {
            (Some(s), Some(e)) if e > s => Some((s, e)),
            _ => None,
        },
        _ => None,
    };
    let include_audio = m
        .get("include_audio")
        .and_then(|a| a.as_bool())
        .unwrap_or(true);
    let audio_bit_rate = m
        .get("audio_bit_rate")
        .and_then(|a| a.as_i64())
        .unwrap_or(PRESET_AUDIO_BPS);
    Ok(SpecInputs {
        preset,
        codec,
        size,
        bitrate_mbps,
        fps,
        range,
        include_audio,
        audio_bit_rate,
    })
}

/// Resolve the dialogue inputs into a [`ResolvedSpec`], given the comp's own
/// size — a faithful port of `ExportDialogState::spec` (docs/06 §7.5, K-119):
/// the target defaults to the comp size; the bitrate parses from Mbps (blank =
/// encoder default); and the VBR peak follows the preset's peak while its numbers
/// stand unedited (same codec and same target bitrate), else the customary 1.5×.
fn resolve_spec(inputs: &SpecInputs, comp_w: u32, comp_h: u32) -> ResolvedSpec {
    let stamped = preset_params(&inputs.preset);
    let target = inputs.size.unwrap_or((comp_w, comp_h));
    let bit_rate = inputs
        .bitrate_mbps
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|m| *m > 0.0)
        .map(|m| (m * 1_000_000.0) as i64);
    let max_rate = match (stamped, bit_rate) {
        (Some(p), Some(b)) if b == p.target_bps && inputs.codec == p.codec => Some(p.peak_bps),
        (_, Some(b)) => Some(b.saturating_mul(3) / 2),
        (_, None) => None,
    };
    let images = is_image_codec(&inputs.codec);
    ResolvedSpec {
        codec: inputs.codec.clone(),
        target,
        // Stills are lossless; a bitrate against them is a field the dialogue
        // should not have sent, dropped here so the exporter never sees one.
        bit_rate: if images { None } else { bit_rate },
        max_rate: if images { None } else { max_rate },
        fps: (inputs.fps > 0.0).then_some(inputs.fps),
        range: inputs.range.map(|(s, e)| (s as usize, e as usize)),
        // A folder of stills has nowhere for sound to go (K-201).
        include_audio: inputs.include_audio && !images,
        audio_bit_rate: inputs.audio_bit_rate,
    }
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
    spec_json: &str,
    out_path: &str,
) -> String {
    driving::start_with_document(doc, comp, spec_json, out_path)
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
    spec_json: &str,
    out_path: &str,
    start: bool,
    open_folder: bool,
) -> Result<u32, String> {
    driving::queue_add(
        doc,
        comp,
        comp_name,
        spec_json,
        out_path,
        start,
        open_folder,
    )
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
    use super::{err_json, parse_inputs, resolve_spec, ResolvedSpec};
    use lumit_render::export::{ExportEvent, ExportHandle, ExportSpec};
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
        spec_json: String,
        out_path: String,
        preset: String,
        range: Option<(u64, u64)>,
        /// Show the file when it lands (docs/07 §11's *Reveal in folder*).
        open_folder: bool,
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

    /// Convert the resolved spec into the exporter's `ExportSpec` (codec name →
    /// the real `VideoCodec`; an unknown name is a calm error).
    /// Without a media build there is no encoder to name, so an export cannot
    /// be specified at all — a calm error rather than a spec pointing at
    /// nothing.
    #[cfg(not(feature = "media"))]
    fn to_export_spec(_r: &ResolvedSpec) -> Result<ExportSpec, String> {
        Err("export: this build has no encoder (the media feature is off)".to_owned())
    }

    #[cfg(feature = "media")]
    fn to_export_spec(r: &ResolvedSpec) -> Result<ExportSpec, String> {
        use lumit_media::encode::{ImageFormat, VideoCodec};
        use lumit_render::export::ExportFormat;
        let format = match r.codec.as_str() {
            "h264" => ExportFormat::Video(VideoCodec::H264),
            "hevc" => ExportFormat::Video(VideoCodec::Hevc),
            "png" => ExportFormat::Images(ImageFormat::Png),
            "tiff" => ExportFormat::Images(ImageFormat::Tiff),
            other => return Err(format!("export: unknown format '{other}'")),
        };
        Ok(ExportSpec {
            format,
            target: r.target,
            bit_rate: r.bit_rate,
            max_rate: r.max_rate,
            fps: r.fps,
            range: r.range,
            include_audio: r.include_audio,
            audio_bit_rate: r.audio_bit_rate,
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
        spec_json: &str,
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

        match launch(&mut guard, &doc, comp, spec_json, out_path) {
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
        spec_json: &str,
        out_path: &str,
    ) -> Result<(), String> {
        let parsed = parse_inputs(spec_json).map_err(|e| format!("export: {e}"))?;
        // Resolve the spec against the comp's own size.
        let (cw, ch) = doc
            .comp(comp)
            .map(|c| (c.width, c.height))
            .ok_or("export: unknown composition")?;
        let resolved = resolve_spec(&parsed, cw, ch);
        let spec = to_export_spec(&resolved)?;

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
        spec_json: &str,
        out_path: &str,
        start: bool,
        open_folder: bool,
    ) -> Result<u32, String> {
        if out_path.trim().is_empty() {
            return Err("export: no output path".to_owned());
        }
        // Parsed here rather than at launch time so a spec the resolver cannot
        // read is refused while the user is looking at the dialogue, instead of
        // failing silently minutes later when its turn comes.
        let parsed = parse_inputs(spec_json).map_err(|e| format!("export: {e}"))?;

        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        let id = guard.next_id;
        guard.next_id += 1;
        guard.queue.push(Item {
            id,
            comp_name,
            doc,
            comp,
            spec_json: spec_json.to_owned(),
            out_path: out_path.to_owned(),
            preset: parsed.preset.clone(),
            range: parsed.range,
            open_folder,
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
                preset: item.preset.clone(),
                codec: parse_inputs(&item.spec_json)
                    .map(|i| i.codec)
                    .unwrap_or_default(),
                range: item.range,
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
            let (doc, comp, spec_json, out_path, id) = {
                let item = &run.queue[index];
                (
                    item.doc.clone(),
                    item.comp,
                    item.spec_json.clone(),
                    item.out_path.clone(),
                    item.id,
                )
            };
            match launch(run, &doc, comp, &spec_json, &out_path) {
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
                    // The tick the dialogue set, honoured here rather than by
                    // whatever window happens to be watching: an export that
                    // lands after its dialogue closed still opens its folder.
                    if item.open_folder && matches!(item.state, ItemState::Done) {
                        super::reveal_in_folder(&item.out_path);
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
    use super::{parse_inputs, resolve_spec};

    /// The dialogue's new fields parse with honest defaults: no fps override,
    /// no explicit range, and a nonsense range (end before start) is ignored
    /// rather than clamped into something nobody asked for (K-201).
    #[test]
    fn fps_and_range_parse_with_absent_meaning_default() {
        let inputs = parse_inputs(r#"{"codec":"h264"}"#).unwrap();
        assert_eq!(inputs.fps, 0.0);
        assert_eq!(inputs.range, None);

        let inputs = parse_inputs(r#"{"codec":"h264","fps":29.97,"range":[12,48]}"#).unwrap();
        assert_eq!(inputs.fps, 29.97);
        assert_eq!(inputs.range, Some((12, 48)));

        let inputs = parse_inputs(r#"{"codec":"h264","range":[48,12]}"#).unwrap();
        assert_eq!(inputs.range, None, "a backwards range is no range");

        let resolved = resolve_spec(
            &parse_inputs(r#"{"codec":"h264","fps":0}"#).unwrap(),
            64,
            36,
        );
        assert_eq!(resolved.fps, None, "zero means the comp's own rate");
    }

    /// An image sequence is stills: no audio track and no bitrates, whatever
    /// the dialogue sent — resolution enforces it so the exporter never has to.
    #[test]
    fn image_codecs_shed_audio_and_bitrates_at_resolution() {
        let inputs =
            parse_inputs(r#"{"codec":"png","include_audio":true,"bitrate_mbps":"16"}"#).unwrap();
        let resolved = resolve_spec(&inputs, 64, 36);
        assert!(!resolved.include_audio, "stills carry no sound");
        assert_eq!(resolved.bit_rate, None, "stills are lossless");
        assert_eq!(resolved.max_rate, None);

        // And the same dialogue state as h264 keeps all three.
        let inputs =
            parse_inputs(r#"{"codec":"h264","include_audio":true,"bitrate_mbps":"16"}"#).unwrap();
        let resolved = resolve_spec(&inputs, 64, 36);
        assert!(resolved.include_audio);
        assert_eq!(resolved.bit_rate, Some(16_000_000));
    }
}
