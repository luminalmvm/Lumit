//! An export of sound alone: the comp's mix written with no picture.
//!
//! # Thread role
//!
//! The export's own thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use super::{AudioFormat, ExportEvent, ExportSpec};

/// Write the comp's mix with no picture at all (docs/06 §7.4). No compositor,
/// no graphics card: the comp's mixdown is the whole export, so this feeds it
/// to the muxer in one-second helpings and reports progress against the same
/// frame count a video export would have written.
pub(super) fn run_audio_only(
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
    let mut encoder = lumit_media::encode::Encoder::open(
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
