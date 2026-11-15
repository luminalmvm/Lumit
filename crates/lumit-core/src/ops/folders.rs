//! The two folder helpers behind the folder ops: the cycle guard and the next free name.

use super::*;

/// Whether `folder` sits anywhere inside `ancestor` — the cycle guard
/// [`move_to_folder_ops`] refuses on.
///
/// Walks children rather than parents, and remembers where it has been: a
/// document that already holds a cycle (one written by an older build, or by
/// hand) must make this answer, not spin.
pub(super) fn folder_descends_from(doc: &Document, folder: Uuid, ancestor: Uuid) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut queue = vec![ancestor];
    while let Some(id) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(f) = doc.folder(id) {
            if f.children.contains(&folder) {
                return true;
            }
            queue.extend(f.children.iter().copied());
        }
    }
    false
}

/// The next unused "Folder N" — the default name [`new_folder_ops`] gives.
///
/// Counts up past the names already taken rather than off the number of
/// folders: a project holding "Folder 1" and "Renders" must not offer
/// "Folder 2" only to collide the moment "Folder 1" is renamed away.
#[must_use]
pub(super) fn next_folder_name(doc: &Document) -> String {
    let taken: std::collections::HashSet<&str> = doc
        .items
        .iter()
        .filter(|i| matches!(i, ProjectItem::Folder(_)))
        .map(ProjectItem::name)
        .collect();
    (1..)
        .map(|n| format!("Folder {n}"))
        .find(|candidate| !taken.contains(candidate.as_str()))
        // `1..` is unbounded and `taken` is finite, so this always answers; the
        // fallback is here because nothing in an engine crate may panic
        // (docs/14 §4).
        .unwrap_or_else(|| "Folder".to_owned())
}
