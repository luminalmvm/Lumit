//! Where a footage item's pixels are read from.

use lumit_core::model::Document;
use std::path::PathBuf;
use uuid::Uuid;

/// The decode source for one footage item: the file its pixels are read from
/// (already resolved through the proxy rule) and — for a numbered run of
/// stills — the rate to read the run at.
///
/// The rate rides only when the file *is* the item's own media. A proxy is one
/// file standing in for the whole run, so reading it as a sequence would be
/// reading files that are not there. The same goes for a source layer or a
/// render pass: a proxy has no layers or passes to pick from.
pub(super) fn media_source(
    doc: &Document,
    item: Uuid,
    media: &lumit_core::model::MediaRef,
) -> lumit_media::sequence::MediaSource {
    let footage = match doc.item(item) {
        Some(lumit_core::model::ProjectItem::Footage(f)) => Some(f),
        _ => None,
    };
    let (sequence_fps, source_layer, render_pass) = match footage {
        Some(f) if std::ptr::eq(&f.media, media) => (
            f.sequence_fps(),
            f.source_layer,
            f.interpretation.render_pass.clone(),
        ),
        _ => (None, None, None),
    };
    lumit_media::sequence::MediaSource {
        path: PathBuf::from(&media.absolute_path),
        sequence_fps,
        source_layer,
        render_pass,
        // A proxy is the same frames, so it is read the way its original is.
        interpret: footage
            .map(crate::source::interpretation)
            .unwrap_or_default(),
        stamp: None,
    }
    .stamped()
}
