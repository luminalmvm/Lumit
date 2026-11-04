//! What a project lets go of as it closes, beside its place in the registries.

use crate::api::project::ProjectReference;

pub(crate) fn let_go(project: &ProjectReference) {
    // A shared project stops being shared when it closes.
    crate::api::share::stop(project.id);
    // And another Lumit may open its file.
    crate::claim::release(project.id);
    // Clips copied in the Cut workspace name this project's items.
    crate::api::cut::forget_copied(project.id);
    // No Blender stays at work on a file nothing is waiting for.
    project.stop_blenders_where(true);
}
