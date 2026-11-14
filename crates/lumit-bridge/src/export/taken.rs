//! Whether an export would write over something, and the name that would
//! not: the file itself, frame one of a sequence, the caption files beside
//! it, the files written beside it, and what the queue has yet to write.

use super::{captions_out, first_file, queue_list, with_format_extension, QueueRowState};
use crate::api::export::{BridgeCaptionsOut, BridgeExportAlso};
use lumit_render::export::captions::sidecar_taken;

/// `out_path` with its format's ending, or the first free numbered name
/// beside it when that is taken already, by a file or by the queue.
pub(crate) fn untaken_path(
    out_path: &str,
    codec: &str,
    captions: BridgeCaptionsOut,
    also: &[BridgeExportAlso],
) -> String {
    if path_taken(out_path, codec, captions, also) {
        free_path(out_path, codec, captions, also)
    } else {
        with_format_extension(out_path, codec)
    }
}

/// Whether an export to `out_path` would write over something: a file already
/// there, or an export still waiting or running in the queue under that name.
/// The caption files it writes beside it count too, and so do its `also` files.
pub(crate) fn path_taken(
    out_path: &str,
    codec: &str,
    captions: BridgeCaptionsOut,
    also: &[BridgeExportAlso],
) -> bool {
    let path = with_format_extension(out_path, codec);
    let files: Vec<_> = std::iter::once((path.clone(), codec.to_owned()))
        .chain(super::also::paths(&path, also))
        .collect();
    files.iter().any(|(p, c)| first_file(p, c).exists())
        || sidecar_taken(std::path::Path::new(&path), captions_out(captions))
        || queue_list().iter().any(|row| {
            files
                .iter()
                .any(|(p, _)| row.out_path == *p || row.also.contains(p))
                && matches!(
                    row.state,
                    QueueRowState::Waiting | QueueRowState::Running { .. }
                )
        })
}

/// The first of `name_2.ext`, `name_3.ext`, … that writes over nothing, for
/// Keep both. `out_path` itself when every number up to the cap is taken.
pub(crate) fn free_path(
    out_path: &str,
    codec: &str,
    captions: BridgeCaptionsOut,
    also: &[BridgeExportAlso],
) -> String {
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
        .find(|p| !path_taken(p, codec, captions, also))
        .unwrap_or(path)
}
