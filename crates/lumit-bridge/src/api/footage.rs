use std::{path::PathBuf, sync::Arc};

use lumit_core::model::FootageItem;
use uuid::Uuid;

use flutter_rust_bridge::frb;

use crate::api::{
    state::{LumitBridgeState, PROJECTS},
    BridgeError,
};

// Not feature-gated: `thumbnail` is declared whatever the build, so its return
// type has to exist whatever the build.
use crate::api::state::BridgeRenderedFrame;

#[derive(Debug, PartialEq, Eq)]
#[frb]
pub struct FootageReference {
    #[frb(name = "internalproject")]
    pub project: Uuid,
    #[frb(name = "internalid")]
    pub id: Uuid,
}

pub enum LumitMediaStatus {
    Missing,
    Ready,
}

/// A footage file's own vital statistics, as the container declares them.
///
/// What "a comp matching the footage" means when a clip is dragged onto the New
/// composition button (docs/07 §3.1): the size, the rate and the length come from
/// here. The rate is the exact pair the container carries and the duration is
/// rational seconds — both because a rate that went through a float would not come
/// back as 30000/1001 (docs/14 §2).
///
/// Audio-only media has no picture, so `width`/`height` are zero and the rate is
/// 0/1; the caller keeps its own size rather than making a comp no pixels wide.
#[frb(non_opaque)]
pub struct BridgeMediaInfo {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub duration: crate::api::effect::BridgeRational,
}

impl FootageReference {
    #[frb(ignore)]
    pub fn new(project: Uuid, id: Uuid) -> FootageReference {
        FootageReference { project, id }
    }

    #[frb(ignore)]
    pub fn project_id(&self) -> Uuid {
        self.project
    }

    #[frb(ignore)]
    pub fn id(&self) -> Uuid {
        self.id
    }

    #[frb(ignore)]
    fn project(&self) -> Result<Arc<std::sync::RwLock<LumitBridgeState>>, BridgeError> {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        let project = projects.get(&self.project);

        let p = project.ok_or(BridgeError::InvalidProject)?;
        Ok(p.clone())
    }

    // copy pasted from lumit-ui/src/headless.rs
    // would be good if these could be shared
    //
    /// Where this footage's file actually is. `None` when the path cannot be
    /// resolved at all — a relative path in a project that has never been saved
    /// (so there is no directory to resolve against), or one that no longer
    /// exists on disk. The caller reports that as missing media, which is what
    /// it is; it is not an error worth surfacing separately.
    pub(crate) fn resolve_path(p: &LumitBridgeState, f: &FootageItem) -> Option<PathBuf> {
        if f.media.absolute_path.is_empty() {
            let path = p.path.clone()?;
            let path = path.parent()?;
            let path = path.join(PathBuf::from(&f.media.relative_path));
            path.canonicalize().ok()
        } else {
            Some(PathBuf::from(&f.media.absolute_path))
        }
    }

    /// [`Self::resolve_path`] together with what kind of media the item says it
    /// is: one file, or the numbered run that file belongs to (K-439).
    ///
    /// Everything that probes, indexes or decodes footage asks for this rather
    /// than the bare path, because for an image sequence the two are different
    /// questions — the path is the file that gets stat-ed and relinked, the run
    /// is what actually plays.
    pub(crate) fn resolve_source(
        p: &LumitBridgeState,
        f: &FootageItem,
    ) -> Option<lumit_media::MediaSource> {
        Some(lumit_media::MediaSource {
            path: Self::resolve_path(p, f)?,
            sequence_fps: f
                .sequence
                .as_ref()
                .map(|s| (s.frame_rate.num(), s.frame_rate.den())),
        })
    }

    /// Point this footage item at `path`, and fix every *other* missing item
    /// that moved the same way — one undo step for the lot.
    ///
    /// The sibling sweep is the behaviour that makes relinking a moved project
    /// bearable: footage almost always moves as a folder, so relinking one clip
    /// by hand should not mean relinking forty. It works two ways, in order.
    ///
    /// **The path rewrite** (docs/10 §2) is the one that carries a whole tree.
    /// Where the file went tells you where its folder went: everything the old
    /// path and the new one share at the end did not move, and the prefix in
    /// front of it did. Every other lost item whose stored path begins with that
    /// old prefix is looked for under the new one — so relinking one clip four
    /// folders deep brings back the forty-seven others in forty-seven *different*
    /// subfolders, which is exactly the shape of an edit's footage.
    ///
    /// **Beside the picked file**, by name, is the fallback for the flatter case
    /// — a folder of clips whose paths share nothing useful.
    ///
    /// A sibling is only touched when it currently fails to resolve *and* the
    /// file the rewrite predicts actually exists, so a healthy item is never
    /// repointed and a guess is never saved.
    #[frb(sync)]
    pub fn relink(&self, path: String) -> Result<(), BridgeError> {
        if path.trim().is_empty() {
            return Err(BridgeError::MediaPathUnresolved);
        }
        let picked = PathBuf::from(&path);
        let proj = self.project()?;

        // The files this relink points at, so the probe worker can be reading
        // them while the user is still looking at the picker's afterglow: the
        // panel asks every repointed item for its status the moment the change
        // lands.
        let mut repointed: Vec<PathBuf> = Vec::new();

        let ops = {
            let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = p.store.snapshot();

            // Refuse early if this reference does not name footage at all.
            match doc.item(self.id).ok_or(BridgeError::InvalidItem)? {
                lumit_core::model::ProjectItem::Footage(_) => {}
                _ => return Err(BridgeError::InvalidItem),
            }

            let folder = picked.parent().map(std::path::Path::to_path_buf);
            let project_dir = p.path.as_deref().and_then(|p| p.parent());

            // Where a reference says its file was, whether or not it is there:
            // the session's absolute path when the project carries one, and
            // otherwise the saved relative path against the project's folder.
            // This is the *old* side of the rewrite, so it must be the stored
            // path rather than a resolved one — a resolved path is by definition
            // one that was not lost.
            let stored = |media: &lumit_core::model::MediaRef| -> Option<PathBuf> {
                if !media.absolute_path.is_empty() {
                    return Some(PathBuf::from(&media.absolute_path));
                }
                project_dir.map(|dir| dir.join(&media.relative_path))
            };
            let mapping = match doc.item(self.id) {
                Some(lumit_core::model::ProjectItem::Footage(target)) => {
                    stored(&target.media).and_then(|old| lumit_project::path_mapping(&old, &picked))
                }
                _ => None,
            };

            let mut ops = Vec::new();
            for item in &doc.items {
                let lumit_core::model::ProjectItem::Footage(other) = item else {
                    continue;
                };
                let is_target = other.id == self.id;

                let candidate = if is_target {
                    picked.clone()
                } else {
                    // Only sweep items that are actually broken, and only to a
                    // file that really exists beside the picked one.
                    if Self::resolve_path(&p, other).is_some_and(|p| p.is_file()) {
                        continue;
                    }
                    let moved_with_it = mapping
                        .as_ref()
                        .zip(stored(&other.media))
                        .and_then(|(mapping, old)| lumit_project::apply_mapping(mapping, &old))
                        .filter(|candidate| candidate.is_file());
                    let beside_it = || {
                        let folder = folder.as_ref()?;
                        // Split on both separators: a path written on the other
                        // sort of machine comes back whole from `file_name`.
                        let name = lumit_project::file_name_of(&other.media.relative_path)
                            .unwrap_or(other.name.as_str());
                        let candidate = folder.join(name);
                        candidate.is_file().then_some(candidate)
                    };
                    let Some(candidate) = moved_with_it.or_else(beside_it) else {
                        continue;
                    };
                    candidate
                };

                let mut media = other.media.clone();
                media.absolute_path = candidate.to_string_lossy().into_owned();
                if let Some(dir) = project_dir {
                    if let Some(rel) = lumit_project::relative_between(dir, &candidate) {
                        media.relative_path = rel;
                    }
                }
                media.fingerprint = lumit_project::fingerprint_path(&candidate).ok();
                repointed.push(candidate);
                ops.push(lumit_core::Op::SetMediaRef {
                    id: other.id,
                    media: Box::new(media),
                });
            }
            ops
        };

        if ops.is_empty() {
            return Err(BridgeError::MediaPathUnresolved);
        }

        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        let op = if ops.len() == 1 {
            ops.into_iter().next().ok_or(BridgeError::InvalidItem)?
        } else {
            lumit_core::Op::Batch { ops }
        };
        proj.store.commit(op).map_err(BridgeError::OpError)?;
        drop(proj);

        // After the commit and outside the lock: queueing is a channel send,
        // but the rule is the rule (docs/14 §3).
        for path in &repointed {
            crate::probe::request(path);
        }
        Ok(())
    }

    /// A small decoded picture of this footage's first frame, for the Project
    /// panel row. `None` when the file cannot be resolved or decoded — a missing
    /// or unsupported item shows its type glyph instead.
    ///
    /// Deliberately **not** `#[frb(sync)]`: a cold video decode is FFmpeg work
    /// measured in tens of milliseconds, so it must not run on Dart's UI isolate.
    /// frb puts an async call on its own worker pool and Dart simply awaits it —
    /// which is the whole of what v0 needed a hand-rolled isolate, a wire
    /// protocol, a `TransferableTypedData` hand-off and a generation map to
    /// achieve. Memoised per (item, size) in the project's media cache, so a
    /// rebuild costs nothing.
    ///
    /// The pixels are small enough that frb's per-byte `Vec<u8>` encoding does not
    /// matter here: at the panel's 56 px longer edge this is a few kilobytes, not
    /// the megabytes a Viewer frame carries.
    ///
    /// Declared whatever the features are, so the generated Dart is one shape:
    /// a build with no decoder answers `None` rather than the method being
    /// absent and the Dart side failing to compile against it.
    #[cfg(not(feature = "media"))]
    pub fn thumbnail(&self, max_edge: u32) -> Result<Option<BridgeRenderedFrame>, BridgeError> {
        let _ = max_edge;
        Ok(None)
    }

    #[cfg(feature = "media")]
    pub fn thumbnail(&self, max_edge: u32) -> Result<Option<BridgeRenderedFrame>, BridgeError> {
        let proj = self.project()?;
        let mut proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;

        let Some(src) = ({
            let snapshot = proj.store.snapshot();
            match snapshot.item(self.id) {
                Some(lumit_core::model::ProjectItem::Footage(footage)) => {
                    Self::resolve_source(&proj, footage)
                }
                _ => None,
            }
        }) else {
            return Ok(None);
        };

        let id = self.id;
        Ok(
            crate::media::thumbnail_from_path(&mut proj.media, id, max_edge, &src, 0).map(
                |(width, height, rgba)| BridgeRenderedFrame {
                    // A thumbnail is of the media's own first frame, not of a
                    // composition — there is no playhead behind it to report.
                    frame: 0,
                    width,
                    height,
                    rgba,
                },
            ),
        )
    }

    /// This footage's declared size, rate and length, or `None` when the file
    /// cannot be resolved or does not probe.
    ///
    /// Async for the same reason `thumbnail` is: probing opens the container with
    /// FFmpeg, which is not work for Dart's UI isolate. Declared whatever the
    /// features are, so a build with no decoder answers `None` rather than the
    /// method being absent and the Dart side failing to compile against it.
    #[cfg(not(feature = "media"))]
    pub fn media_info(&self) -> Result<Option<BridgeMediaInfo>, BridgeError> {
        Ok(None)
    }

    #[cfg(feature = "media")]
    pub fn media_info(&self) -> Result<Option<BridgeMediaInfo>, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;

        let snapshot = proj.store.snapshot();
        let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(self.id) else {
            return Err(BridgeError::InvalidItem);
        };
        let Some(src) = Self::resolve_source(&proj, footage) else {
            return Ok(None);
        };
        let Some(info) = crate::probe::ensure_probed(&src) else {
            return Ok(None);
        };

        // The only sanctioned route back from the container's floating-point
        // duration is an explicit grid (docs/impl/rational-time.md §4); the
        // millisecond grid is the resolution the Duration field edits in anyway.
        let duration = lumit_core::time::Rational::from_f64_on_grid(info.duration_seconds, 1000)
            .unwrap_or(lumit_core::time::Rational::ZERO);
        let video = info.video.as_ref();
        Ok(Some(BridgeMediaInfo {
            width: video.map_or(0, |v| v.width),
            height: video.map_or(0, |v| v.height),
            fps_num: video
                .and_then(|v| u32::try_from(v.fps_num).ok())
                .unwrap_or(0),
            fps_den: video
                .and_then(|v| u32::try_from(v.fps_den).ok())
                .unwrap_or(1),
            duration: crate::api::effect::BridgeRational {
                num: duration.num(),
                den: duration.den(),
            },
        }))
    }

    pub fn get_status(&self) -> Result<LumitMediaStatus, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;

        let snapshot = proj.store.snapshot();
        let item = snapshot.item(self.id).ok_or(BridgeError::InvalidItem)?;

        match item {
            lumit_core::model::ProjectItem::Footage(footage_item) => {
                // An unresolvable path is missing media, same as one that
                // resolves but no longer decodes.
                let Some(src) = Self::resolve_source(&proj, footage_item) else {
                    return Ok(LumitMediaStatus::Missing);
                };
                let path = src.path.clone();
                // Whether a file is *there* is not a question for the decoder,
                // and a media-less build used to answer "ready" for a path that
                // plainly was not on disk (K-273). Asking the filesystem costs
                // one stat and gives both builds the same answer.
                if !path.exists() {
                    return Ok(LumitMediaStatus::Missing);
                }

                // It is there; whether it *decodes* takes a prober. A build
                // without one answers that it resolved, because reporting
                // "missing" for a file plainly on disk would send the user to
                // relink something that is not lost.
                //
                // Through the probe worker's cache, which is keyed by the
                // file's own size and modification time — so this still asks
                // *this* file, and a file that has been replaced or has gone
                // away since the last question is read again rather than
                // answered from memory.
                #[cfg(not(feature = "media"))]
                let probed = true;
                #[cfg(feature = "media")]
                let probed = crate::probe::ensure_probed(&src).is_some();

                if probed {
                    Ok(LumitMediaStatus::Ready)
                } else {
                    Ok(LumitMediaStatus::Missing)
                }
            }
            _ => Err(BridgeError::InvalidItem),
        }
    }
}
