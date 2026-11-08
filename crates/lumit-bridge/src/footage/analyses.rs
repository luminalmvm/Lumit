//! What analyses made of a shot, in a shared project: depth and mattes, a
//! Roto brush's mattes, and tracks.
//!
//! They go the way footage does. Each machine says which it has, under an id
//! made from what the analysis is filed under, and whoever lacks one asks for
//! it. What arrives is taken in with this machine's own analyses, so everyone
//! sees the same picture and nobody runs it again.
//!
//! Only what Lumit's own effects made of the project's footage crosses. The
//! files an effect reads, such as a LUT, may be somebody else's to give, and
//! are never sent.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use lumit_core::model::Document;
use lumit_share::bulk::{Held, Wanted};
use uuid::Uuid;

use super::{kept_under, Carrier, GIVE};

/// Each analysis `doc` could be holding: the id it is held under, what it is
/// filed under, and where this machine keeps its file, there or not. Held
/// under an id of its own because a camera solve is filed under its footage
/// item, whose id already says who has the original.
fn analyses(doc: &Document) -> Vec<(Uuid, Uuid, PathBuf)> {
    let files = lumit_render::sent::files(doc).into_iter();
    files
        .map(|(filed, path)| (Uuid::new_v5(&filed, b"analysis"), filed, path))
        .collect()
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

impl Carrier {
    /// Where the file for the analysis held under `id` is put as it arrives.
    fn sent_to(&self, id: Uuid) -> Option<PathBuf> {
        let folder = kept_under(self.root.as_deref())?.join("analyses");
        Some(folder.join(id.to_string()))
    }

    /// This machine's file for `id`, to send.
    pub(super) fn analysis_file(&self, doc: &Document, id: Uuid) -> Option<PathBuf> {
        let (_, _, path) = analyses(doc).into_iter().find(|kept| kept.0 == id)?;
        (GIVE.load(Ordering::Relaxed) && path.is_file()).then_some(path)
    }

    pub(super) fn analysis_room(&self, doc: &Document, id: Uuid) -> Option<PathBuf> {
        if !analyses(doc).iter().any(|kept| kept.0 == id) {
            return None;
        }
        self.sent_to(id)
    }

    /// The file for `id` is here at `path`: take the analysis in. A file that
    /// is not what it was said to be is thrown away, and not asked for again
    /// until who has what changes.
    pub(super) fn take_in(&self, doc: &Document, id: Uuid, path: &Path) -> bool {
        let filed = analyses(doc).into_iter().find(|kept| kept.0 == id);
        let taken = filed.is_some_and(|(_, filed, _)| {
            std::fs::read(path).is_ok_and(|bytes| lumit_render::sent::adopt(doc, filed, &bytes))
        });
        if !taken {
            let _ = std::fs::remove_file(path);
            if let Ok(mut refused) = self.refused.lock() {
                refused.insert(Wanted::Analysis { id });
            }
        }
        taken
    }

    /// Note which analyses this machine has in `held`, and answer the ones it
    /// is to ask for. One sent before is taken in again rather than fetched,
    /// unless nobody else has one its size any more, which is an analysis run
    /// again where it came from.
    pub(super) fn settle_analyses(&self, doc: &Document, held: &mut Vec<Held>) -> Vec<Wanted> {
        let me = self.sharing(|sharing| sharing.me());
        let mut wants = Vec::new();
        for (id, _, path) in analyses(doc) {
            let holders = self.sharing(|sharing| sharing.holders(id));
            let others: Vec<u64> = (holders.into_iter().flatten())
                .filter(|(peer, _)| Some(*peer) != me)
                .map(|(_, bytes)| bytes)
                .collect();
            let sent = self.sent_to(id).filter(|path| path.is_file());
            let stale = sent
                .as_deref()
                .is_some_and(|sent| !others.is_empty() && !others.contains(&size(sent)));
            if path.is_file() && !stale {
                held.push(Held {
                    item: id,
                    bytes: size(&path),
                });
                continue;
            }
            match sent {
                Some(sent) if !stale => {
                    self.take_in(doc, id, &sent);
                }
                _ => wants.push(Wanted::Analysis { id }),
            }
        }
        wants
    }
}
