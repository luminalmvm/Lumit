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

/// What the export dialogue is asking for — the whole of
/// `lumit_render::export::ExportSpec`, in the flat shape the seam carries.
///
/// `width`/`height` of zero mean "the composition's own size", which is what the
/// dialogue shows until somebody types over it. `bitrate_mbps` of zero means the
/// encoder's own default — a quality nobody chose is better than a number this
/// layer invented — and is a *different answer* from `bitrate_auto`, which works
/// a delivery-quality rate out from the frame and the rate.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeExportSpec {
    /// A preset name from the store, or empty for a custom export.
    pub preset: String,
    /// The output format key: `h264` / `hevc` for an `.mp4`, `png` / `tiff`
    /// for a numbered image sequence, `m4a` / `wav` for sound alone.
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub bitrate_mbps: u32,
    /// The VBR peak in Mb/s; zero takes the customary 1.5× of the target. A
    /// preset carries its own peak, which is why this crosses rather than being
    /// worked out from the target on the way over.
    pub peak_mbps: u32,
    /// Work the bitrate out from the frame and the rate. Overrides
    /// `bitrate_mbps`; a blank field (zero) with this off means the encoder
    /// chooses its own quality, which is what blank has always meant.
    pub bitrate_auto: bool,
    /// Output frame rate; zero means the composition's own. A different rate
    /// resamples by nearest comp frame over the same wall-clock span.
    pub fps: f64,
    /// Export range start, in comp frames. Negative means the default — the
    /// work area when one is set, else the whole comp.
    pub range_start_frame: i64,
    /// Export range end (exclusive), in comp frames. Negative = the default.
    pub range_end_frame: i64,
    pub include_audio: bool,
    /// Audio bits per second; zero takes the delivery-preset rate.
    pub audio_bit_rate: i64,
    /// The sound's sample rate in hertz — one of [`export_audio_rates`].
    /// Zero means the customary 48 kHz, so a caller that never sets
    /// it writes the file Lumit has always written.
    #[frb(default = 0)]
    pub audio_rate: u32,
    /// Bits a sound sample: 16 or 24. Anything below 24 reads as
    /// sixteen, which is what an unset field has always meant. A format whose
    /// sound cannot carry the choice (AAC stores coefficients, not samples) is
    /// refused rather than handed the identical file either way —
    /// [`BridgeFormatCaps::audio_24_bit`] is the row that says which.
    #[frb(default = 0)]
    pub audio_depth: u32,
    /// Channels in the written sound: `1` folds the composition's stereo mix
    /// down to mono, anything else (zero included) keeps it stereo.
    /// Every format that carries sound carries both, so there is no capability
    /// row for it.
    #[frb(default = 0)]
    pub audio_channels: u32,
    /// Bits per channel in the written file: 8 or 16.
    pub depth: u32,
    /// Write the composite's own coverage as an alpha channel, rather than an
    /// opaque one.
    pub alpha_channel: bool,
    /// Un-multiply the colour on the way out (docs/06 §3.4). Meaningless
    /// without `alpha_channel`, and ignored there.
    pub straight_alpha: bool,
    /// The output colour space, by the stable name
    /// [`BridgeFormatCaps::colour_spaces`] lists: empty for the default sRGB /
    /// Rec. 709 (a genuine pass-through), `linear`, `rec709`, `rec2020`,
    /// `display-p3`, or the name of an OCIO output space (post-v1; an export
    /// that asks for one before OCIO exists is refused). A space the chosen
    /// container cannot *state* is refused too, rather than written unlabelled.
    pub colour_space: String,
    /// The filter a resized frame is resampled with: `high` for
    /// Lanczos-3, anything else — blank included — for the bilinear default
    /// every Lumit export has always used. Blank rather than `fast` as the
    /// unset value, so a caller that never sets it cannot change a byte.
    #[frb(default = "")]
    pub resample: String,
    /// Pixels taken off each edge, at composition size.
    pub crop_top: u32,
    pub crop_left: u32,
    pub crop_bottom: u32,
    pub crop_right: u32,
    /// Take the crop from the Viewer's region of interest instead.
    pub use_region_of_interest: bool,
    /// That region as comp fractions `[x0, y0, x1, y1]`, or empty for
    /// none. Anything that is not four increasing finite numbers is no region.
    pub region: Vec<f64>,
    /// What is written into the container about the file, in the order the
    /// Metadata section lists it — the order lands in the file.
    pub metadata: Vec<BridgeMetadataField>,
    /// The preview-resolution divisor the export renders at: 1 = Full, 2 =
    /// Half, 3 = Third, 4 = Quarter (docs/01 §5).
    pub quality_divisor: u32,
    /// Read frames already banked in the disk cache (nothing is ever written).
    pub disk_cache_read_only: bool,
    /// Run each layer's effect stack.
    pub effects: bool,
    /// Honour solo switches.
    pub honour_solo: bool,
    /// Deliver the guide layers too. Off — the default — is what a
    /// guide layer is: drawn in the Viewer, absent from the file, at every
    /// depth.
    #[frb(default = false)]
    pub render_guides: bool,
    /// Motion blur at export: `0` the compositions' own settings, `1`
    /// on for checked layers, `2` off for all layers. An unknown number is the
    /// compositions' own settings — an answer nobody recognises is not a
    /// reason to refuse an export.
    #[frb(default = 0)]
    pub motion_blur: u32,
    /// Retime blend at export: `0` the compositions' own settings, `1`
    /// off for all layers. There is **no** *on for checked layers*: a layer's
    /// interpolation policy is its own check, so that answer would write the
    /// identical file as the first.
    #[frb(default = 0)]
    pub retime_blend: u32,
    /// Read the proxies instead of the originals. Off by default
    /// whatever the project is set to work at: delivery is the one moment a
    /// proxy must not apply, and a draft for review is the only export it is
    /// right for.
    #[frb(default = false)]
    pub use_proxies: bool,
    /// Play the completion sound when this export lands. Silent — never an
    /// error — when no sound has been supplied.
    pub make_a_noise: bool,
    /// Show the finished file in the desktop's own file manager.
    pub open_folder: bool,
}

/// One key/value pair written into the container.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BridgeMetadataField {
    /// FFmpeg's own key — `title`, `artist`, `copyright`, `comment`,
    /// `creation_time`, or any other the container will take.
    pub key: String,
    pub value: String,
}

impl Default for BridgeExportSpec {
    /// A comp-sized H.264 mp4 with sound — what a plain "Export…" has always
    /// meant — mirroring `ExportSpec::default()` field for field.
    fn default() -> Self {
        Self {
            preset: String::new(),
            codec: "h264".to_owned(),
            width: 0,
            height: 0,
            bitrate_mbps: 0,
            peak_mbps: 0,
            bitrate_auto: true,
            fps: 0.0,
            range_start_frame: -1,
            range_end_frame: -1,
            include_audio: true,
            audio_bit_rate: crate::export::PRESET_AUDIO_BPS,
            audio_rate: 0,
            audio_depth: 0,
            audio_channels: 0,
            depth: 8,
            alpha_channel: false,
            straight_alpha: false,
            colour_space: String::new(),
            resample: String::new(),
            crop_top: 0,
            crop_left: 0,
            crop_bottom: 0,
            crop_right: 0,
            use_region_of_interest: false,
            region: Vec::new(),
            metadata: Vec::new(),
            quality_divisor: 1,
            disk_cache_read_only: false,
            effects: true,
            honour_solo: true,
            render_guides: false,
            motion_blur: 0,
            retime_blend: 0,
            use_proxies: false,
            make_a_noise: false,
            open_folder: false,
        }
    }
}

/// What one output format can and cannot carry — `ExportFormat::caps()` as the
/// dialogue reads it.
///
/// A control the format cannot honour is **disabled**, not live: the dialogue
/// reads this row to decide, and the engine refuses the same combinations as a
/// backstop, so the two cannot disagree about what a file will hold.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BridgeFormatCaps {
    /// Carries a picture at all.
    pub video: bool,
    /// Can carry the composition's sound.
    pub audio: bool,
    /// Can carry an alpha channel.
    pub alpha: bool,
    /// The colour depths this format writes, best last: `[8]`, `[8, 16]`, or
    /// empty for a format with no picture.
    pub depths: Vec<u32>,
    /// A video bitrate applies (lossless formats have none to choose).
    pub bit_rate: bool,
    /// An audio bitrate applies — AAC has one, uncompressed PCM is exactly
    /// what it is.
    pub audio_bit_rate: bool,
    /// This format's sound can be written twenty-four bits a sample as well as
    /// sixteen — true for uncompressed PCM, false for AAC and for
    /// every format with no sound at all.
    ///
    /// A flag rather than a list, because the engine's list is
    /// `AudioDepth::ALL` and has exactly two members. ponytail: if a third
    /// sample width ever exists this becomes a `Vec<u32>` mirroring `depths`;
    /// `the_caps_row_says_what_the_engine_says` fails the moment it would need
    /// to.
    #[frb(default = false)]
    pub audio_24_bit: bool,
    /// The container holds metadata.
    pub metadata: bool,
    /// The colour spaces this format's container can **state**, by the stable
    /// names `BridgeExportSpec::colour_space` carries: `""` (sRGB / Rec. 709),
    /// `linear`, `rec709`, `rec2020`, `display-p3`. Empty where the
    /// format carries no picture.
    ///
    /// Names, not labels: a space the seam cannot translate is a space whose
    /// wording belongs in `app_en.arb` like every other string,
    /// and a name nobody recognises — an OCIO config's own — is shown as it
    /// arrived, exactly as a codec name is.
    #[frb(default = [])]
    pub colour_spaces: Vec<String>,
}

/// The crop an export actually applies, and the frame it leaves — the answer
/// `crop_for` gives for the typed insets and the Viewer's region together.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BridgeCrop {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
    /// The frame that survives the crop, in pixels.
    pub width: u32,
    pub height: u32,
}

/// One row of the preset list: its name, and whether it is one of the
/// read-only built-ins.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BridgeExportPresetEntry {
    pub name: String,
    pub read_only: bool,
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

        let reply = crate::export::start_export_with_document(document, self.id, &spec, &path);
        reply_ok(&reply).then_some(()).ok_or_else(|| {
            BridgeError::ExportFailed(reply_error(&reply).unwrap_or_else(|| "export".into()))
        })
    }
}

/// What one output format can and cannot carry. The dialogue asks this
/// of every format key it offers and disables what the answer refuses.
#[frb(sync)]
pub fn export_format_caps(codec: String) -> BridgeFormatCaps {
    crate::export::format_caps(&codec)
}

/// Every sample rate an export can write sound at, in hertz and in the order
/// the Sound row lists them.
///
/// Not a capability row, because it does not vary by format: a format either
/// carries sound — `BridgeFormatCaps::audio` — and then carries all of these,
/// or carries none and the whole row is dead. `the_caps_row_says_what_the
/// _engine_says` holds the two in step.
#[frb(sync)]
pub fn export_audio_rates() -> Vec<u32> {
    crate::export::audio_rates()
}

impl CompositionReference {
    /// Refuse a spec the chosen format cannot honour, in the dialogue's own
    /// words — empty when the spec is exportable.
    ///
    /// The engine refuses the same combinations before a frame is rendered;
    /// asking here only means the message arrives while the user is looking at
    /// the fields rather than minutes later from the queue.
    ///
    /// **On the composition rather than free-standing** because a colour space
    /// is one of the settings, and whether a name can be delivered is a
    /// question about *this project's* colour config — one check, so
    /// the footer and the exporter cannot disagree about the same spec.
    #[frb(sync)]
    pub fn export_spec_check(&self, spec: BridgeExportSpec) -> Result<String, BridgeError> {
        let state = self.project()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        crate::api::colour::with_colour(&state, |colour| crate::export::spec_check(&spec, colour))
    }
}

/// The crop this spec actually applies to a `comp_width` × `comp_height` frame,
/// and the frame that survives it — the typed insets, or the Viewer's region of
/// interest when that is asked for and exists.
#[frb(sync)]
pub fn export_crop_for(spec: BridgeExportSpec, comp_width: u32, comp_height: u32) -> BridgeCrop {
    crate::export::crop_for(&spec, comp_width, comp_height)
}

/// The video bitrate this spec runs with, in bits per second, for a frame of
/// `width` × `height` at `fps` — the typed number, or the one *Auto* works out.
/// Zero when the format has no bitrate to choose or the encoder is choosing its
/// own quality, which is when the footer offers no size estimate.
#[frb(sync)]
pub fn export_resolved_bitrate(spec: BridgeExportSpec, width: u32, height: u32, fps: f64) -> i64 {
    crate::export::resolved_bitrate(&spec, width, height, fps)
}

/// Every preset the dialogue's list shows — the read-only built-ins first, then
/// the user's own in the order they were saved.
#[frb(sync)]
pub fn export_preset_list() -> Vec<BridgeExportPresetEntry> {
    crate::export::preset_list()
}

/// The settings behind a preset name, or `None` when there is no such preset.
#[frb(sync)]
pub fn export_preset_get(name: String) -> Option<BridgeExportSpec> {
    crate::export::preset_get(&name)
}

/// Save the current settings under `name`, replacing a preset of that name in
/// its own row. A built-in's name is refused rather than shadowed.
#[frb(sync)]
pub fn export_preset_save(name: String, spec: BridgeExportSpec) -> Result<(), BridgeError> {
    crate::export::preset_save(&name, &spec).map_err(BridgeError::ExportFailed)
}

/// Forget a preset of one's own. A built-in and an unknown name both answer an
/// error rather than a silent no-op, so the dialogue can say why.
#[frb(sync)]
pub fn export_preset_delete(name: String) -> Result<(), BridgeError> {
    crate::export::preset_delete(&name).map_err(BridgeError::ExportFailed)
}

/// What the export dialogue opens on when nothing else has been said —
/// the subset of a spec worth remembering between sessions, kept beside the
/// preset library in the application's own data area and never in a `.lum`.
///
/// Every field is a string, and every empty string means "nothing has been
/// said", so a store that has never been written asks for exactly the export
/// Lumit has always opened on.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BridgeExportDefaults {
    /// A preset name from the store, or empty for the dialogue's own choice.
    pub preset: String,
    /// The output format key, or empty for the preset's own.
    pub codec: String,
    /// The filename pattern in the tokens the exporter already substitutes —
    /// `{comp}`, `{preset}`, `{date}`. Empty gives each preset's own
    /// suggested name.
    pub filename_template: String,
    /// Where a finished file is written: `ask` every time, `project` for
    /// beside the project file, or `folder` for [`Self::folder`]. Always one
    /// of the three on the way out — an answer a newer Lumit wrote reads as
    /// `ask`, which is the one that cannot write somewhere surprising.
    pub destination: String,
    /// The folder `destination: "folder"` means; empty is the same as `ask`.
    pub folder: String,
}

/// The stored export defaults, or the built-in answers when none were ever
/// saved. Read as the dialogue and the Settings page open, never per rebuild.
#[frb(sync)]
pub fn export_defaults_get() -> BridgeExportDefaults {
    crate::export::defaults_get()
}

/// Remember these as what the export dialogue opens on — the *Set as default*
/// action in the preset strip, and every row of Settings ▸ Export.
#[frb(sync)]
pub fn export_defaults_set(defaults: BridgeExportDefaults) -> Result<(), BridgeError> {
    crate::export::defaults_set(&defaults).map_err(BridgeError::ExportFailed)
}

/// What a delivery preset stamps into the dialogue, and what to call the file.
///
/// A blank `preset` gives the custom defaults. `template` drives the
/// `{comp}`/`{preset}`/`{date}` substitution; blank yields the preset's
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
    /// it was queued. The spec's *when done* ticks — the noise and the folder —
    /// are honoured as the item lands rather than by whatever window is
    /// watching, so an export that finishes after its dialogue closed still
    /// does what it was asked to.
    #[frb(sync)]
    pub fn queue_export(
        &self,
        spec: BridgeExportSpec,
        path: String,
        start: bool,
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
        crate::export::queue_add(document, self.id, comp_name, &spec, &path, start)
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

/// Move one waiting item to `index` in the queue — the drag the queue window's
/// list offers, so the order work runs in is the order it is looked at in.
///
/// The queue's order is **transient state, not document state**: it is not in
/// the `.lum`, it does not survive a restart, and it is no more undoable than
/// removing an item is. So this commits no op and journals nothing, exactly as
/// [`export_queue_remove`] does.
///
/// Three calm refusals rather than a silent no-op, because a row that will not
/// move should say why: an item that is running, one that has already run, and
/// an id no longer in the list. An `index` past the end lands it last.
#[frb(sync)]
pub fn export_queue_move(id: u32, index: u32) -> Result<(), BridgeError> {
    crate::export::queue_move(id, index as usize).map_err(BridgeError::ExportFailed)
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
