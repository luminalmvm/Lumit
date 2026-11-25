use crate::api::project::ProjectReference;

pub(crate) fn let_go(project: &ProjectReference) {
    crate::api::share::stop(project.id);
    crate::claim::release(project.id);
    crate::api::cut::forget_copied(project.id);
    project.stop_blenders_where(true);
}
