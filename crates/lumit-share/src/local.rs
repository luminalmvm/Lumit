//! Making what a peer sent fit this machine: refusing what it will not hold,
//! and pointing footage at its own copy. Any thread.

use lumit_core::model::{MediaRef, ProjectItem};
use lumit_core::{Document, Op};
use lumit_project::{comp_size_is_sane, resolve_all_media, resolve_media, Resolved};
use std::path::Path;
use uuid::Uuid;

/// More edits than this in one batch and it is not one a person made: the
/// largest real one is a paste of a few thousand layers.
const MAX_BATCHED: usize = 100_000;

/// How many edits `op` is, batches opened out, counted no further than `room`.
fn weight(op: &Op, room: usize) -> usize {
    match op {
        Op::Batch { ops } => ops.iter().fold(1, |so_far, op| {
            if so_far > room {
                so_far
            } else {
                so_far + weight(op, room - so_far)
            }
        }),
        _ => 1,
    }
}

/// Whether this machine will hold an edit from a peer. What opening a file
/// checks is checked here: a composition's size, which every raster made for
/// it is multiplied by. And how many edits it is, since each is work done
/// while everyone else's edits wait.
pub(crate) fn sane(op: &Op) -> bool {
    weight(op, MAX_BATCHED) <= MAX_BATCHED && sized(op)
}

fn sized(op: &Op) -> bool {
    match op {
        Op::Batch { ops } => ops.iter().all(sized),
        Op::AddItem { item, .. } => match &**item {
            ProjectItem::Composition(comp) => comp_size_is_sane(comp.width, comp.height),
            _ => true,
        },
        Op::SetCompSettings { width, height, .. } | Op::CropCompToRegion { width, height, .. } => {
            comp_size_is_sane(*width, *height)
        }
        _ => true,
    }
}

/// [`sane`] for a whole document.
pub(crate) fn sane_document(doc: &Document) -> bool {
    doc.items.iter().all(|item| match item {
        ProjectItem::Composition(comp) => comp_size_is_sane(comp.width, comp.height),
        _ => true,
    })
}

/// Point the footage a peer's edit names, or the proxy of some, at this
/// machine's copy of it. `have` is the document here and `root` the folder
/// this machine keeps the project's footage under.
pub(crate) fn place(op: &mut Op, have: &Document, root: Option<&Path>) {
    match op {
        Op::Batch { ops } => ops.iter_mut().for_each(|op| place(op, have, root)),
        Op::AddItem { item, .. } => {
            if let ProjectItem::Footage(footage) = &mut **item {
                find(&mut footage.media, have.packed_ref(footage.id), root);
            }
        }
        Op::SetMediaRef { id, media } => find(media, have.packed_ref(*id), root),
        Op::SetItemProxy {
            id,
            proxy: Some(proxy),
        } => find(&mut proxy.media, proxy_of(have, *id), root),
        _ => {}
    }
}

/// Keep the path this machine already has when the peer means the same file,
/// which is every relink a peer makes to its own copy. Otherwise look for the
/// file under `root`, by name and then by fingerprint. Not found is not an
/// error: the item shows as missing here until it is relinked.
fn find(media: &mut MediaRef, had: Option<&MediaRef>, root: Option<&Path>) {
    if let Some(had) = had {
        let same = match (&had.fingerprint, &media.fingerprint) {
            (Some(a), Some(b)) => a.likely_same_content(b),
            _ => false,
        };
        if same && !had.absolute_path.is_empty() {
            media.absolute_path.clone_from(&had.absolute_path);
            return;
        }
    }
    // A name with no fingerprint beside it is not enough. Whatever this
    // machine holds the original of it will send to whoever asks, and a peer
    // that could name any file kept beside the project would be sent it.
    if media.fingerprint.is_none() {
        return;
    }
    if let Some(root) = root {
        if let Resolved::Found { path, .. } = resolve_media(media, root, &[]) {
            media.absolute_path = path.to_string_lossy().into_owned();
        }
    }
}

/// The file `have` reads the proxy of the footage item `id` from.
fn proxy_of(have: &Document, id: Uuid) -> Option<&MediaRef> {
    have.proxy(id).map(|proxy| &proxy.media)
}

/// Make the host's document usable on a guest. Nothing in the host's file is
/// in a guest's, so nothing is marked as packed. What is this machine's own
/// carries over from the document it had, `have`: where its footage is, its
/// cache folder and its panel layout.
///
/// A guest's copy has an id of its own. The crash journal is filed by it, so
/// two Lumits on one machine would otherwise write the same one.
pub(crate) fn settle(doc: &mut Document, have: Option<&Document>, root: Option<&Path>) {
    doc.id = have.map_or_else(Uuid::now_v7, |have| have.id);
    doc.packed.clear();
    doc.ui_state = have.and_then(|have| have.ui_state.clone());
    doc.cache_location = have.and_then(|have| have.cache_location.clone());
    if let Some(have) = have {
        carry(doc, have);
    }
    if let Some(root) = root {
        resolve_all_media(doc, root, &[]);
    }
    // The same rule as [`find`]: what the host named without a fingerprint
    // is not this machine's file on the strength of its name.
    for item in &mut doc.items {
        if let ProjectItem::Footage(footage) = item {
            if footage.media.fingerprint.is_none() {
                footage.media.absolute_path.clear();
            }
        }
    }
}

/// Point `doc`'s footage and proxies at the files `have` reads the same
/// ones from.
pub(crate) fn carry(doc: &mut Document, have: &Document) {
    for item in &mut doc.items {
        if let ProjectItem::Footage(footage) = item {
            find(&mut footage.media, have.packed_ref(footage.id), None);
        }
    }
    for (id, proxy) in &mut doc.proxies {
        find(&mut proxy.media, proxy_of(have, *id), None);
    }
}
