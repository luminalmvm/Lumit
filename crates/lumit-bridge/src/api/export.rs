//! Writing a composition out to a file.
//!
//! # In plain terms
//!
//! Export is the one long-running job in Lumit. It does not block: you start it,
//! and then you ask how it is getting on. That is why this is three calls rather
//! than one — `start`, `poll`, `cancel` — and why nothing here returns a
//! finished file.
//!
//! Only one export runs at a time, because two exports competing for the same
//! GPU would make both slower and neither predictable. The rest wait in the
//! **queue**: `queue_export` adds one (with the document snapshotted there and
//! then, docs/06 §7.1), `export_queue_list` reads the whole list and turns it
//! over, and `export_queue_cancel`/`export_queue_remove` take one out.

use flutter_rust_bridge::frb;
use serde_json::Value;

use crate::api::{composition::CompositionReference, BridgeError};

/// What the export dialogue is asking for.
///
/// `width`/`height` of zero mean "the composition's own size", which is what the
/// dialogue shows until somebody types over it. `bitrate_mbps` of zero means the
/// encoder's own default — a quality nobody chose is better than a number this
/// layer invented.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeExportSpec {
    /// A delivery preset name, or empty for a custom export.
    pub preset: String,
    /// The output format key: `h264` / `hevc` for an `.mp4`, `png` / `tiff`
    /// for a numbered image sequence (K-201).
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub bitrate_mbps: u32,
    /// Output frame rate; zero means the composition's own. A different rate
    /// resamples by nearest comp frame over the same wall-clock span.
    pub fps: f64,
    /// Export range start, in comp frames. Negative means the default — the
    /// work area when one is set, else the whole comp.
    pub range_start_frame: i64,
    /// Export range end (exclusive), in comp frames. Negative = the default.
    pub range_end_frame: i64,
    pub include_audio: bool,
    /// Audio bits per second; zero takes the preset's own rate.
    pub audio_bit_rate: i64,
}

/// What a delivery preset fills the export dialogue with.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeExportPreset {
    pub codec: String,
    /// Zero means "the composition's own size".
    pub width: u32,
    pub height: u32,
    /// Zero means the encoder's own default.
    pub bitrate_mbps: u32,
    /// The file name to suggest in the picker.
    pub default_name: String,
}

/// How a running export is getting on.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeExportState {
    /// Nothing has run since start-up.
    Idle,
    Running {
        frame: u64,
        /// Zero until the exporter has worked out how many there are.
        total: u64,
        /// The encoder actually chosen, which may not be the one asked for —
        /// a hardware encoder that is not there falls back to software, and the
        /// dialogue should say so rather than claim what was requested.
        encoder: String,
    },
    Done {
        path: String,
    },
    Failed {
        error: String,
    },
}

impl CompositionReference {
    /// Start writing this composition to `path`.
    ///
    /// Returns once the job is *running*, not once it is finished — ask
    /// [`export_poll`] for that. An export already in flight is a calm error.
    #[frb(sync)]
    pub fn start_export(&self, spec: BridgeExportSpec, path: String) -> Result<(), BridgeError> {
        if path.trim().is_empty() {
            return Err(BridgeError::NoProjectPath);
        }
        let document = {
            let state = self.project()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };

        let spec_json = spec_json(&spec);
        let reply = crate::export::start_export_with_document(document, self.id, &spec_json, &path);
        reply_ok(&reply).then_some(()).ok_or_else(|| {
            BridgeError::ExportFailed(reply_error(&reply).unwrap_or_else(|| "export".into()))
        })
    }
}

/// The dialogue's own JSON shape, which is also what the egui frontend sends —
/// one spec parser, so the two frontends cannot export differently.
#[frb(ignore)]
fn spec_json(spec: &BridgeExportSpec) -> String {
    serde_json::json!({
            "preset": spec.preset,
            "codec": spec.codec,
            "size": if spec.width == 0 || spec.height == 0 {
                Value::Null
            } else {
                serde_json::json!([spec.width, spec.height])
            },
            "bitrate_mbps": if spec.bitrate_mbps == 0 {
                String::new()
            } else {
                spec.bitrate_mbps.to_string()
            },
            "fps": spec.fps,
            "range": if spec.range_start_frame < 0
                || spec.range_end_frame <= spec.range_start_frame
            {
                Value::Null
            } else {
                serde_json::json!([spec.range_start_frame, spec.range_end_frame])
            },
            "include_audio": spec.include_audio,
            "audio_bit_rate": spec.audio_bit_rate,
    })
    .to_string()
}

/// What a delivery preset stamps into the dialogue, and what to call the file.
///
/// A blank `preset` gives the custom defaults. `template` drives the
/// `{comp}`/`{preset}`/`{date}` substitution (K-119); blank yields the preset's
/// own suggested name.
#[frb(sync)]
pub fn export_preset(preset: String, comp_name: String, template: String) -> BridgeExportPreset {
    let reply = crate::export::export_preset(&preset, &comp_name, &template);
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&reply) else {
        return BridgeExportPreset {
            codec: "h264".into(),
            width: 0,
            height: 0,
            bitrate_mbps: 0,
            default_name: String::new(),
        };
    };
    let size = map.get("size").and_then(Value::as_array);
    BridgeExportPreset {
        codec: map
            .get("codec")
            .and_then(|v| v.as_str())
            .unwrap_or("h264")
            .to_owned(),
        width: size
            .and_then(|a| a.first())
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        height: size
            .and_then(|a| a.get(1))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        bitrate_mbps: map
            .get("bitrate_mbps")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        default_name: map
            .get("default_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned(),
    }
}

/// How the running export is getting on. Safe to call on the interface's own
/// cadence: it drains a channel and reads a few numbers.
#[frb(sync)]
pub fn export_poll() -> BridgeExportState {
    let reply = crate::export::export_poll();
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&reply) else {
        return BridgeExportState::Idle;
    };
    let string = |key: &str| {
        map.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let number = |key: &str| map.get(key).and_then(Value::as_u64).unwrap_or(0);

    match map.get("state").and_then(|v| v.as_str()) {
        Some("running") => BridgeExportState::Running {
            frame: number("frame"),
            total: number("total"),
            encoder: string("encoder"),
        },
        Some("done") => BridgeExportState::Done {
            path: string("path"),
        },
        Some("failed") => BridgeExportState::Failed {
            error: string("error"),
        },
        _ => BridgeExportState::Idle,
    }
}

/// Ask the running export to stop. It finishes the frame it is on and then
/// reports `Failed` with "cancelled" — a cancelled export leaves no half-file
/// pretending to be a finished one.
#[frb(sync)]
pub fn export_cancel() {
    let _ = crate::export::export_cancel();
}

/// One item in the export queue.
///
/// Everything here was true when the item was *added*: the document it renders
/// was snapshotted then (docs/06 §7.1), and so was the comp's name. Editing the
/// composition afterwards changes nothing about a queued export.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeExportQueueItem {
    pub id: u32,
    pub comp_name: String,
    /// Where it writes. The interface shows the file's own name and keeps the
    /// whole path for the tooltip.
    pub path: String,
    /// The delivery preset, empty for a custom export.
    pub preset: String,
    /// The format key — `h264`/`hevc`, or `png`/`tiff` for a sequence.
    pub codec: String,
    /// The range in comp frames, end exclusive. Both −1 when the item takes the
    /// default: the work area as it stood at queue time, else the whole comp.
    pub range_start_frame: i64,
    pub range_end_frame: i64,
    pub state: BridgeExportQueueState,
}

/// Where one queued item has got to.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeExportQueueState {
    /// Waiting its turn — nothing runs until the queue is started.
    Waiting,
    Running {
        frame: u64,
        /// Zero until the exporter has worked out how many there are.
        total: u64,
        /// The encoder actually chosen, which may not be the one asked for.
        encoder: String,
    },
    Done,
    Failed {
        error: String,
    },
}

impl CompositionReference {
    /// Add this composition to the export queue, and start the queue when
    /// `start` is set.
    ///
    /// The two footer actions are this one call: *Add to queue* leaves the
    /// item waiting, *Export* sets it running. Either way the document is
    /// snapshotted here, so the export renders what the composition was when
    /// it was queued. `open_folder` is the dialogue's *Open folder* tick,
    /// honoured as the item lands rather than by whatever window is watching.
    #[frb(sync)]
    pub fn queue_export(
        &self,
        spec: BridgeExportSpec,
        path: String,
        start: bool,
        open_folder: bool,
    ) -> Result<u32, BridgeError> {
        if path.trim().is_empty() {
            return Err(BridgeError::NoProjectPath);
        }
        let document = {
            let state = self.project()?;
            let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
            state.store.snapshot()
        };
        let comp_name = self.get_settings()?.name;
        crate::export::queue_add(
            document,
            self.id,
            comp_name,
            &spec_json(&spec),
            &path,
            start,
            open_folder,
        )
        .map_err(BridgeError::ExportFailed)
    }
}

/// Every item in the queue, oldest first.
///
/// Asking is also what turns the queue over: the next item starts when the one
/// before it finishes, on the same cadence the progress is read.
#[frb(sync)]
pub fn export_queue_list() -> Vec<BridgeExportQueueItem> {
    crate::export::queue_list()
        .into_iter()
        .map(|row| BridgeExportQueueItem {
            id: row.id,
            comp_name: row.comp_name,
            path: row.out_path,
            preset: row.preset,
            codec: row.codec,
            range_start_frame: row.range.map_or(-1, |(a, _)| a as i64),
            range_end_frame: row.range.map_or(-1, |(_, b)| b as i64),
            state: match row.state {
                crate::export::QueueRowState::Waiting => BridgeExportQueueState::Waiting,
                crate::export::QueueRowState::Running {
                    frame,
                    total,
                    encoder,
                } => BridgeExportQueueState::Running {
                    frame,
                    total,
                    encoder,
                },
                crate::export::QueueRowState::Done => BridgeExportQueueState::Done,
                crate::export::QueueRowState::Failed(error) => {
                    BridgeExportQueueState::Failed { error }
                }
            },
        })
        .collect()
}

/// Let the queue run: the next waiting item starts, and the rest follow it.
#[frb(sync)]
pub fn export_queue_start() {
    crate::export::queue_start();
}

/// Cancel one item. A running export stops at its next frame and leaves no
/// half-file; an item still waiting simply leaves the list.
#[frb(sync)]
pub fn export_queue_cancel(id: u32) {
    crate::export::queue_cancel(id);
}

/// Forget one item, cancelling it first if it is the one running.
#[frb(sync)]
pub fn export_queue_remove(id: u32) {
    crate::export::queue_remove(id);
}

#[frb(ignore)]
fn reply_ok(reply: &str) -> bool {
    serde_json::from_str::<Value>(reply)
        .ok()
        .and_then(|v| v.get("ok").and_then(Value::as_bool))
        .unwrap_or(false)
}

#[frb(ignore)]
fn reply_error(reply: &str) -> Option<String> {
    serde_json::from_str::<Value>(reply)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str().map(str::to_owned)))
}
