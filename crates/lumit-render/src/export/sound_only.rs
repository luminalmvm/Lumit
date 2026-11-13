//! An export of sound alone: the comp's mix written with no picture, and the
//! stems, one file per bus, beside any export that carries sound.
//!
//! # Thread role
//!
//! The export's own thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use lumit_core::model::Document;
use uuid::Uuid;

use super::frame::{carries_sound, cut, desk, Span};
use super::{AudioFormat, ExportEvent, ExportFormat, ExportSpec};
use crate::mix::jobs::AudioJob;

/// Write the comp's mix with no picture at all (docs/06 §7.4). No compositor,
/// no graphics card: the comp's mixdown is the whole export, so this feeds it
/// to the muxer in one-second helpings and reports progress from `progress.0`
/// to `progress.1`, the frame count a video export would have written.
pub(super) fn run_audio_only(
    out_path: &Path,
    format: AudioFormat,
    spec: &ExportSpec,
    mix: Option<&[f32]>,
    (from, total): (usize, usize),
    tx: &Sender<ExportEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    // A comp with no audible layer still exports a file — of silence, of the
    // right length. An empty .wav would look like a failure that wasn't one.
    let mix = mix.unwrap_or(&[]);
    write_sound(
        out_path,
        format,
        spec,
        mix,
        cancel,
        &mut |label| {
            let _ = tx.send(ExportEvent::Encoder(label));
        },
        &mut |done| {
            // Progress against the picture's own clock, so the queue row
            // reads the same way whatever an item writes.
            let frame = from + ((total - from) as f64 * done) as usize;
            let _ = tx.send(ExportEvent::Progress {
                frame: frame.min(total),
                total,
            });
        },
    )?;
    let _ = tx.send(ExportEvent::Progress {
        frame: total,
        total,
    });
    Ok(())
}

/// Write `mix` (interleaved, in the spec's layout) to `out_path` in one-second
/// helpings, naming the encoder to `on_encoder` and handing `on_progress` how
/// much is written, 0 to 1. A stop leaves the file unfinished for the caller.
fn write_sound(
    out_path: &Path,
    format: AudioFormat,
    spec: &ExportSpec,
    mix: &[f32],
    cancel: &AtomicBool,
    on_encoder: &mut dyn FnMut(&'static str),
    on_progress: &mut dyn FnMut(f64),
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
    on_encoder(encoder.encoder_label());
    let chunk = rate as usize * chans;
    for (n, block) in mix.chunks(chunk).enumerate() {
        super::held::wait(cancel);
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        encoder.write_audio(block).map_err(|e| e.to_string())?;
        let done = ((n + 1) * chunk).min(mix.len());
        on_progress(done as f64 / mix.len().max(1) as f64);
    }
    encoder.finish().map_err(|e| e.to_string())
}

/// **Stems**: each top-level bus of the comp written to a file of its own
/// beside `out_path`, named `<file stem> - <bus name>.<ext>`: the bus after
/// its own fader, with the master's fader and limiter left out, cut to the
/// export range like the main file's sound. Nothing when the spec asks for no
/// stems or the file carries no sound. A stem this wrote is removed again if
/// a later one fails or the export is stopped.
pub(super) fn write_stems(
    doc: &Arc<Document>,
    comp_id: Uuid,
    jobs: &[AudioJob],
    out_path: &Path,
    spec: &ExportSpec,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if !spec.stems || !carries_sound(spec) {
        return Ok(());
    }
    let comp = doc.comp(comp_id).ok_or("composition missing")?;
    let span = Span::of(comp, spec);
    let Some((plan, info)) = desk(spec, doc, comp, jobs, cancel) else {
        return Ok(());
    };
    let format = stem_format(spec.format);
    let mut written: Vec<PathBuf> = Vec::new();
    let tidy = |written: &[PathBuf]| {
        for path in written {
            let _ = std::fs::remove_file(path);
        }
    };
    for (i, node) in info.nodes.iter().enumerate().filter(|(_, n)| n.bus) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Some(stem) = crate::mix::render::stem(&plan, i as u16) else {
            continue;
        };
        let full = crate::mix::render::render(Arc::new(stem), spec.audio_rate, false, &mut |_| {});
        let name = comp
            .layers
            .iter()
            .find(|l| l.id == node.layer)
            .map_or("Bus", |l| l.name.as_str());
        let path = stem_path(out_path, name, format.extension());
        written.push(path.clone());
        let result = write_sound(
            &path,
            format,
            spec,
            &cut(spec, &span, &full),
            cancel,
            &mut |_| {},
            &mut |_| {},
        );
        if let Err(e) = result {
            tidy(&written);
            return Err(e);
        }
    }
    if cancel.load(Ordering::Relaxed) {
        tidy(&written);
    }
    Ok(())
}

/// What a stem is written as: the export's own sound format, PCM in a `.wav`
/// beside a picture whose sound is PCM, AAC in an `.m4a` beside the rest.
fn stem_format(format: ExportFormat) -> AudioFormat {
    match format {
        ExportFormat::Audio(f) => f,
        ExportFormat::Video(c) if c.is_prores() => AudioFormat::Wav,
        _ => AudioFormat::M4a,
    }
}

/// `<file stem> - <bus name>.<ext>` beside `out_path`, with anything a file
/// name cannot hold in the bus's name made an underscore.
fn stem_path(out_path: &Path, bus: &str, ext: &str) -> PathBuf {
    let stem = out_path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let bus: String = bus
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    out_path.with_file_name(format!("{stem} - {}.{ext}", bus.trim()))
}
