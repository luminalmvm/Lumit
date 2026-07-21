//! Export from Flutter — resolving the export spec (preset stamp, VBR-peak and
//! filename rules) and driving the lumit-ui exporter over the headless seam.
//!
//! # In plain terms
//!
//! Exporting writes the composited comp to an `.mp4` on its own thread, exactly
//! as the egui frontend does (K-017). The bridge reuses the identical exporter
//! (`lumit_ui::export`) through the headless seam (K-175): the seam builds the
//! footage/audio inputs and lends a GPU context, and `export::start` spawns the
//! encode thread and streams progress back over a channel. The bridge holds that
//! channel's receiver and drains it on each poll, so Dart can drive a simple
//! `start → poll* → done/failed` loop over the C ABI.
//!
//! One export runs at a time (docs/06 §7.1); a second `start` while one is
//! running returns a calm `ok:false` and the Dart side queues.
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
#![cfg_attr(all(not(feature = "render"), not(test)), allow(dead_code))]

use crate::err_json;
use serde_json::{json, Value};

/// Audio on all delivery presets: AAC 320 kbps (docs/06 §7.5). The bridge's own
/// copy of `lumit_ui::export::PRESET_AUDIO_BPS`, so spec resolution needs no GPU
/// build to know the default.
pub(crate) const PRESET_AUDIO_BPS: i64 = 320_000;

/// The parameter row a delivery preset stamps — the bridge's pure mirror of
/// `lumit_ui::export::PresetParams` (kept here so the resolver and its tests
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
/// `lumit_ui::export::ExportSpec` (codec as a name string). Produced by
/// [`resolve_spec`] and, under the `render` feature, converted into the real
/// `ExportSpec` the exporter runs with.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct ResolvedSpec {
    pub codec: String,
    pub target: (u32, u32),
    pub bit_rate: Option<i64>,
    pub max_rate: Option<i64>,
    pub include_audio: bool,
    pub audio_bit_rate: i64,
}

/// The dialogue-shaped inputs a `start_export` spec_json carries — the final
/// state of the egui export dialogue's fields, so [`resolve_spec`] can reproduce
/// `ExportDialogState::spec` exactly.
struct SpecInputs {
    preset: String,
    codec: String,
    size: Option<(u32, u32)>,
    bitrate_mbps: String,
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
    ResolvedSpec {
        codec: inputs.codec.clone(),
        target,
        bit_rate,
        max_rate,
        include_audio: inputs.include_audio,
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

/// Start an export of `comp_id` to `out_path` with the dialogue-shaped
/// `spec_json`. Returns `{"ok":true}` on a clean start, or a calm `ok:false` —
/// "an export is already running" while one is in flight (the Dart side queues),
/// or a resolution/build error. Without the `render` feature this always answers
/// "export is unavailable in this build".
pub(crate) fn start_export(comp_id: &str, spec_json: &str, out_path: &str) -> String {
    #[cfg(feature = "render")]
    {
        driving::start(comp_id, spec_json, out_path)
    }
    #[cfg(not(feature = "render"))]
    {
        let _ = (comp_id, spec_json, out_path);
        err_json("export: this build has no exporter (the render feature is off)")
    }
}

/// Poll the running export, draining the exporter's event channel. Reply:
/// `{"ok":true,"state":"idle|running|done|failed","frame":…,"total":…,
/// "encoder":…,"path"/"error":…}`. `idle` when nothing has run since start-up.
pub(crate) fn export_poll() -> String {
    #[cfg(feature = "render")]
    {
        driving::poll()
    }
    #[cfg(not(feature = "render"))]
    {
        json!({ "ok": true, "state": "idle" }).to_string()
    }
}

/// Ask the running export to cancel (no-op when none is running). The export
/// stops at the next frame and poll then reports `failed` with "cancelled".
pub(crate) fn export_cancel() -> String {
    #[cfg(feature = "render")]
    {
        driving::cancel()
    }
    #[cfg(not(feature = "render"))]
    {
        json!({ "ok": true }).to_string()
    }
}

#[cfg(feature = "render")]
mod driving {
    use super::{err_json, parse_inputs, resolve_spec, ResolvedSpec};
    use lumit_ui::export::{ExportEvent, ExportHandle, ExportSpec};
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

    /// The one in-flight export: its state plus the handle whose receiver a poll
    /// drains. The handle is dropped once a terminal event arrives.
    struct Run {
        state: State,
        handle: Option<ExportHandle>,
    }

    static EXPORT: OnceLock<Mutex<Run>> = OnceLock::new();

    fn slot() -> &'static Mutex<Run> {
        EXPORT.get_or_init(|| {
            Mutex::new(Run {
                state: State::Idle,
                handle: None,
            })
        })
    }

    /// Convert the resolved spec into the exporter's `ExportSpec` (codec name →
    /// the real `VideoCodec`; an unknown name is a calm error).
    fn to_export_spec(r: &ResolvedSpec) -> Result<ExportSpec, String> {
        use lumit_media::encode::VideoCodec;
        let codec = match r.codec.as_str() {
            "h264" => VideoCodec::H264,
            "hevc" => VideoCodec::Hevc,
            other => return Err(format!("export: unknown codec '{other}'")),
        };
        Ok(ExportSpec {
            codec,
            target: r.target,
            bit_rate: r.bit_rate,
            max_rate: r.max_rate,
            include_audio: r.include_audio,
            audio_bit_rate: r.audio_bit_rate,
        })
    }

    pub(super) fn start(comp_id: &str, spec_json: &str, out_path: &str) -> String {
        let comp = match Uuid::parse_str(comp_id) {
            Ok(id) => id,
            Err(_) => return err_json("export: composition id is not a valid UUID"),
        };
        let inputs = match parse_inputs(spec_json) {
            Ok(i) => i,
            Err(e) => return err_json(format!("export: {e}")),
        };
        if out_path.trim().is_empty() {
            return err_json("export: no output path");
        }

        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        // Drain first so a just-finished export frees the slot for a new one.
        drain(&mut guard);
        if matches!(guard.state, State::Running { .. }) {
            return err_json("an export is already running");
        }

        // Resolve the spec against the comp's own size (read under the doc lock).
        let (doc, comp_size) = crate::state::with_bridge(|b| {
            let doc = b.store.snapshot();
            let size = doc.comp(comp).map(|c| (c.width, c.height));
            (doc, size)
        });
        let Some((cw, ch)) = comp_size else {
            return err_json("export: unknown composition");
        };
        let resolved = resolve_spec(&inputs, cw, ch);
        let spec = match to_export_spec(&resolved) {
            Ok(s) => s,
            Err(e) => return err_json(e),
        };

        // Build the footage/audio inputs and a GPU context through the headless
        // seam (K-175), then hand off to the exact egui exporter (K-017, K-031).
        let Some(inputs) = crate::render::with_export_inputs(&doc, comp) else {
            return err_json("export: the GPU pipeline is unavailable");
        };
        let handle = lumit_ui::export::start(
            doc.clone(),
            comp,
            inputs.items,
            inputs.audio,
            inputs.gpu,
            std::path::PathBuf::from(out_path),
            spec,
        );

        guard.state = State::Running {
            frame: 0,
            total: 0,
            encoder: None,
        };
        guard.handle = Some(handle);
        json!({ "ok": true }).to_string()
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
            run.state = state;
            run.handle = None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("reply is valid JSON")
    }

    /// A blank template reproduces each preset's own default file name
    /// byte-for-byte (K-119, load-bearing), and the custom preset defaults to
    /// `export.mp4`.
    #[test]
    fn default_file_name_is_the_preset_name_with_no_template() {
        for (preset, name) in [
            ("custom", "export.mp4"),
            ("youtube_1080p60", "youtube-1080p60.mp4"),
            ("youtube_1440p60", "youtube-1440p60.mp4"),
            ("youtube_4k60", "youtube-4k60.mp4"),
            ("vertical_1080p60", "vertical-1080x1920.mp4"),
        ] {
            assert_eq!(export_default_file_name(preset, "My comp", None), name);
            assert_eq!(
                export_default_file_name(preset, "My comp", Some("   ")),
                name
            );
        }
    }

    /// The filename template substitutes the tokens, sanitises Windows-illegal
    /// characters, and forces `.mp4`.
    #[test]
    fn filename_template_substitutes_sanitises_and_suffixes() {
        // `{comp}` and `{preset}` fill in; the illegal `:` and `/` become `_`.
        let out = render_filename_template("{comp}-{preset}", "A: B/C", "youtube-1080p60");
        assert_eq!(out, "A_ B_C-youtube-1080p60.mp4");
        // A template that already ends in .mp4 is not doubled.
        let out = render_filename_template("{preset}.mp4", "x", "export");
        assert_eq!(out, "export.mp4");
        // Illegal characters become underscores (not dropped), so `///` is a
        // real `___.mp4`, not a fallback.
        let out = render_filename_template("///", "x", "s");
        assert_eq!(out, "___.mp4");
        // Whitespace-only trims to empty and falls back to `export`.
        let out = render_filename_template("   ", "x", "s");
        assert_eq!(out, "export.mp4");
    }

    /// The date token expands to a valid `YYYY-MM-DD` (exact value depends on the
    /// clock; assert the shape).
    #[test]
    fn date_token_expands_to_iso_date() {
        let out = render_filename_template("{date}", "x", "s");
        // "YYYY-MM-DD.mp4" — 14 characters.
        assert_eq!(out.len(), "0000-00-00.mp4".len());
        assert_eq!(&out[out.len() - 4..], ".mp4");
        assert_eq!(&out[4..5], "-");
        assert_eq!(&out[7..8], "-");
    }

    /// The unedited preset keeps its own peak (VBR peak preserved); an edited
    /// bitrate takes the 1.5× fallback; a custom export with no bitrate has no
    /// cap.
    #[test]
    fn spec_resolution_follows_the_peak_rules() {
        // Unedited 1080p60: 16 Mbps target, its own 24 Mbps peak, HEVC-vs-H264
        // codec match preserved.
        let unedited = SpecInputs {
            preset: "youtube_1080p60".into(),
            codec: "h264".into(),
            size: Some((1920, 1080)),
            bitrate_mbps: "16".into(),
            include_audio: true,
            audio_bit_rate: PRESET_AUDIO_BPS,
        };
        let r = resolve_spec(&unedited, 1280, 720);
        assert_eq!(r.target, (1920, 1080));
        assert_eq!(r.bit_rate, Some(16_000_000));
        assert_eq!(r.max_rate, Some(24_000_000), "unedited peak preserved");

        // Edited bitrate (20 Mbps) → 1.5× peak, no longer the preset's 24.
        let edited = SpecInputs {
            preset: "youtube_1080p60".into(),
            codec: "h264".into(),
            size: Some((1920, 1080)),
            bitrate_mbps: "20".into(),
            include_audio: true,
            audio_bit_rate: PRESET_AUDIO_BPS,
        };
        let r = resolve_spec(&edited, 1280, 720);
        assert_eq!(r.bit_rate, Some(20_000_000));
        assert_eq!(r.max_rate, Some(30_000_000), "1.5x fallback");

        // A changed codec breaks the "unedited" match, so the 1.5× fallback
        // applies even at the preset's own target bitrate — and the codec carries
        // through.
        let recodec = SpecInputs {
            preset: "youtube_1080p60".into(),
            codec: "hevc".into(),
            size: Some((1920, 1080)),
            bitrate_mbps: "16".into(),
            include_audio: true,
            audio_bit_rate: PRESET_AUDIO_BPS,
        };
        let r = resolve_spec(&recodec, 1280, 720);
        assert_eq!(r.codec, "hevc");
        assert_eq!(
            r.max_rate,
            Some(24_000_000),
            "16 Mbps × 1.5 via the fallback"
        );

        // Custom, blank bitrate → comp size, no bitrate, no cap.
        let custom = SpecInputs {
            preset: "custom".into(),
            codec: "h264".into(),
            size: None,
            bitrate_mbps: String::new(),
            include_audio: false,
            audio_bit_rate: PRESET_AUDIO_BPS,
        };
        let r = resolve_spec(&custom, 1280, 720);
        assert_eq!(r.target, (1280, 720), "custom uses the comp size");
        assert_eq!(r.bit_rate, None);
        assert_eq!(r.max_rate, None);
        assert!(!r.include_audio);
    }

    /// `parse_inputs` accepts a string or a number bitrate and defaults every
    /// absent field.
    #[test]
    fn parse_inputs_defaults_and_accepts_both_bitrate_forms() {
        let a = parse_inputs(r#"{"preset":"youtube_4k60","bitrate_mbps":45}"#).unwrap();
        assert_eq!(a.preset, "youtube_4k60");
        assert_eq!(a.bitrate_mbps, "45");
        assert!(a.include_audio, "audio defaults on");
        let b = parse_inputs(r#"{"bitrate_mbps":"12","size":[640,480]}"#).unwrap();
        assert_eq!(b.preset, "custom");
        assert_eq!(b.codec, "h264");
        assert_eq!(b.size, Some((640, 480)));
        // A non-object is a calm error.
        assert!(parse_inputs("42").is_err());
    }

    /// A full export of a tiny solid comp to a temp `.mp4` actually completes
    /// (media + render). The encode step (FFmpeg encoder ladder) can be flaky on
    /// a headless machine, so this drives the real global bridge and is gated
    /// behind the `LUMIT_BRIDGE_EXPORT_TEST` env var — the spec/plumbing tests
    /// above stay unconditional. Run with the var set to exercise the whole
    /// start → poll → done loop over a real encoder.
    #[cfg(feature = "render")]
    #[test]
    fn full_export_of_a_solid_comp_completes() {
        if std::env::var("LUMIT_BRIDGE_EXPORT_TEST").is_err() {
            eprintln!("skipping full export (set LUMIT_BRIDGE_EXPORT_TEST to run)");
            return;
        }
        use crate::state::with_bridge;
        // Build a one-solid comp in the global bridge.
        with_bridge(crate::state::new_project);
        with_bridge(|b| crate::state::new_composition(b, "ExportScene"));
        let comp_id = with_bridge(|b| {
            b.store
                .snapshot()
                .items
                .iter()
                .find_map(|i| match i {
                    lumit_core::model::ProjectItem::Composition(c) => Some(c.id),
                    _ => None,
                })
                .map(|id| id.to_string())
        })
        .expect("a comp exists");
        with_bridge(|b| crate::edits::add_solid_layer(b, &comp_id));

        let dir = tempfile::tempdir().expect("temp dir");
        let out = dir.path().join("solid.mp4");
        let out_str = out.to_string_lossy().into_owned();
        // Custom preset, small H.264, no audio: whatever the ladder picks.
        let spec = r#"{"preset":"custom","codec":"h264","include_audio":false}"#;
        let reply = parse(&start_export(&comp_id, spec, &out_str));
        assert_eq!(reply["ok"], json!(true), "export started: {reply}");

        // Poll until done or failed (up to ~30 s for a few frames).
        let mut last = json!({});
        for _ in 0..600 {
            last = parse(&export_poll());
            match last["state"].as_str() {
                Some("done") | Some("failed") => break,
                _ => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
        assert_eq!(last["state"], json!("done"), "export finished: {last}");
        assert!(out.is_file(), "the mp4 was written");
    }

    /// The `export_preset` endpoint stamps a preset's dialogue fields and its
    /// default file name.
    #[test]
    fn export_preset_stamps_the_dialogue_fields() {
        let v = parse(&export_preset("youtube_1440p60", "Scene", ""));
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["codec"], json!("hevc"));
        assert_eq!(v["size"], json!([2560, 1440]));
        assert_eq!(v["bitrate_mbps"], json!("25"));
        assert_eq!(v["default_name"], json!("youtube-1440p60.mp4"));
        // Custom stamps nothing (comp size, blank bitrate).
        let v = parse(&export_preset("custom", "Scene", ""));
        assert_eq!(v["size"], Value::Null);
        assert_eq!(v["bitrate_mbps"], json!(""));
        assert_eq!(v["default_name"], json!("export.mp4"));
        // A template drives the name.
        let v = parse(&export_preset(
            "youtube_1080p60",
            "My scene",
            "{comp}-{preset}",
        ));
        assert_eq!(v["default_name"], json!("My scene-youtube-1080p60.mp4"));
    }
}
