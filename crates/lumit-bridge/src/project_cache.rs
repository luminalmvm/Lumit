//! Where one project's cache goes, for the location the user chose, and the
//! simulation bakes that live there.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::api::layer::LayerReference;
use crate::api::state::LumitBridgeState;
use crate::framecache::disk::Location;
use lumit_core::model::{Document, EffectInstance};
use uuid::Uuid;

/// The folder `project` caches in, for the application's `location`. The
/// project's own answer wins where it has one: a project told to cache on a
/// scratch drive, or beside itself, should do that whatever the application is
/// set to (docs/06 §5.4).
pub(crate) fn root(project: &LumitBridgeState, location: &Location) -> Option<PathBuf> {
    let document = project.store.snapshot();
    let (doc_id, path) = (document.id, project.path.clone());
    let location = match document.cache_location.clone() {
        Some(lumit_core::model::CacheLocation::AppData) => Location::AppData,
        Some(lumit_core::model::CacheLocation::BesideProject) => Location::BesideProject,
        Some(lumit_core::model::CacheLocation::Custom { folder }) if !folder.is_empty() => {
            Location::Custom(PathBuf::from(folder))
        }
        // A custom location with no folder in it is not a location.
        Some(lumit_core::model::CacheLocation::Custom { .. }) => Location::AppData,
        None => location.clone(),
    };
    match location {
        Location::AppData => lumit_project::frame_cache_dir(doc_id),
        Location::BesideProject => match path.as_deref() {
            Some(path) => lumit_cache::disk::sidecar_root(path),
            // Nowhere to sit beside yet.
            None => lumit_project::frame_cache_dir(doc_id),
        },
        Location::Custom(root) => match path.as_deref() {
            Some(path) => lumit_cache::disk::cache_root_for(path, Some(&root)),
            None => Some(root.join(format!("{doc_id}-cache"))),
        },
    }
}

fn point_bakes_at(project: &LumitBridgeState) {
    let (_, location) = crate::framecache::disk::location();
    lumit_render::sim::bake::set_root(root(project, &location));
}

/// The last project's solves and simulations go, this one's solves are read
/// back, in that order, and bakes go to this project's folder from now on.
pub(crate) fn project_changed(id: Uuid, solves: Vec<lumit_render::track::Job>) {
    lumit_render::sim::let_go();
    let project = crate::api::state::PROJECTS
        .read()
        .ok()
        .and_then(|p| p.get(&id).cloned());
    if let Some(project) = project {
        if let Ok(project) = project.read() {
            point_bakes_at(&project);
        }
    }
    lumit_render::track::clear();
    lumit_render::track::warm(solves);
}

/// Bake pressed on `effect`, into wherever the project caches right now.
pub(crate) fn press(
    project: &Arc<RwLock<LumitBridgeState>>,
    doc: Arc<Document>,
    comp: Uuid,
    layer: Uuid,
    effect: Uuid,
) -> lumit_render::track::Requested {
    if let Ok(project) = project.read() {
        point_bakes_at(&project);
    }
    lumit_render::sim::bake::press(doc, comp, layer, effect)
}

/// A bake `inst` may read: its frames and its span.
pub(crate) fn reading(
    layer: &LayerReference,
    item: &lumit_core::model::Layer,
    inst: &EffectInstance,
    effect: Uuid,
) -> Option<(u32, u32)> {
    use lumit_core::fx::effects::bake_points;
    let project = layer.project().ok()?;
    let doc = project.read().ok()?.store.snapshot();
    if inst.effect.match_name == "particulate" {
        return lumit_render::sim::bake::status(&doc, layer.comp_id, layer.layer_id, effect);
    }
    if inst.effect.match_name != bake_points::MATCH_NAME {
        return None;
    }
    let baked = bake_points::usable(&doc, doc.comp(layer.comp_id)?, item, inst)?;
    Some((
        u32::try_from(baked.frames.len()).unwrap_or(u32::MAX),
        baked.span,
    ))
}
