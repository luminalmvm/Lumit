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
    /// The picture stream's codec as the container names it (`h264`, `png`),
    /// or `None` when there is no picture. The *user's* word for the file, not
    /// a display string of ours, so it crosses untranslated (K-303).
    pub video_codec: Option<String>,
    /// The sound stream's codec, or `None` when the file is silent.
    pub audio_codec: Option<String>,
    /// The sound's channel count and rate in hertz. Both zero when there is no
    /// sound — the panel says nothing rather than "0 channels".
    pub channels: u32,
    pub sample_rate: u32,
    /// Whether the picture is a **still** rather than something that runs
    /// (K-246). A still probes with a video stream too — one frame of it — so
    /// the question is whether the stream lasts, and the engine
    /// ([`lumit_media::MediaProbe::runs_as_video`]) is the one place it is
    /// asked, so the panel cannot call a file a still while the Timeline cuts
    /// it as a clip.
    ///
    /// This is what replaced the panel's zero-picture-width inference: a file
    /// with no picture is `video_codec: None`, which is a different fact from
    /// a picture that does not move.
    pub is_still: bool,
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
    pub(crate) fn project(&self) -> Result<Arc<std::sync::RwLock<LumitBridgeState>>, BridgeError> {
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

    /// Where this footage was **last known** to be, whether or not anything is
    /// there now — which is exactly what [`Self::resolve_path`] cannot say,
    /// because it answers `None` for a file that has moved and moved files are
    /// the whole subject of a relink. The relative path is resolved against the
    /// project's folder without touching the disk (a missing file cannot be
    /// canonicalised).
    fn stored_path(p: &LumitBridgeState, f: &FootageItem) -> Option<PathBuf> {
        if !f.media.absolute_path.is_empty() {
            return Some(PathBuf::from(&f.media.absolute_path));
        }
        if f.media.relative_path.is_empty() {
            return None;
        }
        let dir = p.path.as_deref()?.parent()?;
        Some(dir.join(&f.media.relative_path))
    }

    /// Point this footage item at `path`, and fix every *other* missing item
    /// whose file name turns up in the same folder — one undo step for the lot.
    ///
    /// The sibling sweep is the behaviour that makes relinking a moved project
    /// bearable: footage almost always moves as a folder, so relinking one clip
    /// by hand should not mean relinking forty. A sibling is only touched when it
    /// currently fails to resolve *and* a file of its name exists beside the
    /// picked one, so a healthy item is never repointed.
    ///
    /// It sweeps **two** ways, because footage is not usually one flat folder.
    /// A clip that moved from `old/a/b/clip.mov` to `new/a/b/clip.mov` says
    /// where the whole tree went — that is
    /// [`lumit_project::path_mapping`], docs/10 §2's "relinking one file
    /// automatically relinks siblings that resolve under the same path
    /// mapping" — so every sibling under the old root is looked for at the same
    /// place under the new one, subfolders and all. A sibling the mapping does
    /// not cover (or a rename, which cannot generalise) falls back to the flat
    /// look beside the picked file. Either way the file has to actually be
    /// there.
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

            // Where the picked file used to be, and so where everything else
            // moved to. Read before the sweep because it is a fact about the
            // *target*, and the sweep visits the items in project order.
            let mapping = match doc.item(self.id) {
                Some(lumit_core::model::ProjectItem::Footage(target)) => {
                    Self::stored_path(&p, target)
                        .and_then(|old| lumit_project::path_mapping(&old, &picked))
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
                    // Where the same move would have put this one, then simply
                    // beside the picked file.
                    let moved = mapping
                        .as_ref()
                        .zip(Self::stored_path(&p, other))
                        .and_then(|(mapping, old)| lumit_project::apply_mapping(mapping, &old));
                    let beside = folder.as_ref().map(|folder| {
                        let name = std::path::Path::new(&other.media.relative_path)
                            .file_name()
                            .map(std::ffi::OsString::from)
                            .unwrap_or_else(|| std::ffi::OsString::from(&other.name));
                        folder.join(name)
                    });
                    let Some(candidate) = moved
                        .into_iter()
                        .chain(beside)
                        .find(|candidate| candidate.is_file())
                    else {
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

        let Some(path) = ({
            let snapshot = proj.store.snapshot();
            match snapshot.item(self.id) {
                Some(lumit_core::model::ProjectItem::Footage(footage)) => {
                    Self::resolve_path(&proj, footage)
                }
                _ => None,
            }
        }) else {
            return Ok(None);
        };

        let id = self.id;
        Ok(
            crate::media::thumbnail_from_path(&mut proj.media, id, max_edge, &path, 0).map(
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
        let Some(path) = Self::resolve_path(&proj, footage) else {
            return Ok(None);
        };
        let Some(info) = crate::probe::ensure_probed(&path) else {
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
            video_codec: video.map(|v| v.codec.clone()),
            audio_codec: info.audio.as_ref().map(|a| a.codec.clone()),
            channels: info
                .audio
                .as_ref()
                .and_then(|a| u32::try_from(a.channels).ok())
                .unwrap_or(0),
            sample_rate: info
                .audio
                .as_ref()
                .and_then(|a| u32::try_from(a.sample_rate).ok())
                .unwrap_or(0),
            // Has a picture, but it does not run.
            is_still: info.has_picture() && !info.runs_as_video(),
        }))
    }

    /// Where this item's file is, as the *project* records it: the relative
    /// path a saved project actually carries (K-173), falling back to the
    /// absolute one only when the project has never been saved and there is
    /// nothing to be relative to.
    ///
    /// Display data — the Project panel's Path column. It says where the
    /// reference points, not whether anything is there; `get_status` is the
    /// question about the disk, and this deliberately touches none.
    #[frb(sync)]
    pub fn file_path(&self) -> Result<String, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();
        let Some(lumit_core::model::ProjectItem::Footage(footage)) = snapshot.item(self.id) else {
            return Err(BridgeError::InvalidItem);
        };
        Ok(footage.media.display_path().to_owned())
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
                let Some(path) = Self::resolve_path(&proj, footage_item) else {
                    return Ok(LumitMediaStatus::Missing);
                };
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
                let probed = crate::probe::ensure_probed(&path).is_some();

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

// ---------------------------------------------------------------------------
// Proxies (K-501): the second media reference, its two switches, and the
// MAKE-PROXY job that fills it in.
// ---------------------------------------------------------------------------

/// The stand-in file attached to one footage item, as the Project panel's row
/// reads it (K-501).
///
/// Three row states, all readable from here: no proxy at all (`None` from
/// [`FootageReference::get_proxy`]), one attached and being read (`in_use`), and
/// one attached but switched off — either by this item's own tick or by the
/// project's master switch.
///
/// Whether the file on the end of it is *usable* is a different question, about
/// media rather than about the document, and nothing here answers it: the
/// renderer falls back to the original on its own when a proxy is missing or
/// disagrees about the footage's length, deliberately without reporting it as
/// missing media.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeProxy {
    /// Where the stand-in is, in the same form the Path column shows for the
    /// original: the relative path a saved project carries, else the absolute
    /// one (K-173).
    pub path: String,
    /// This item's own *use proxy* tick.
    pub enabled: bool,
    /// Whether the document actually reads it — this item's tick and the
    /// project's master switch both on.
    pub in_use: bool,
}

/// How a MAKE-PROXY transcode is getting on. One runs at a time, so this is a
/// process-wide reading exactly as `export_poll` is.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeProxyState {
    /// Nothing has run since start-up.
    Idle,
    Running {
        frame: u64,
        /// Zero until the source's length has been read.
        total: u64,
    },
    /// The finished file — already attached to its item by the time this is
    /// read.
    Done {
        path: String,
    },
    Failed {
        error: String,
    },
}

impl FootageReference {
    /// The proxy attached to this item, or `None` when there is none.
    #[frb(sync)]
    pub fn get_proxy(&self) -> Result<Option<BridgeProxy>, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = proj.store.snapshot();
        let Some(proxy) = doc.proxy(self.id) else {
            return Ok(None);
        };
        Ok(Some(BridgeProxy {
            path: proxy.media.display_path().to_owned(),
            enabled: proxy.enabled,
            in_use: doc.proxy_in_use(self.id).is_some(),
        }))
    }

    /// Attach `path` as this item's proxy, replacing any proxy already there,
    /// and switch it on.
    ///
    /// One undo step: the op carries the whole reference, so attaching and
    /// detaching invert each other exactly as a relink does.
    #[frb(sync)]
    pub fn set_proxy(&self, path: String) -> Result<(), BridgeError> {
        if path.trim().is_empty() {
            return Err(BridgeError::MediaPathUnresolved);
        }
        attach_proxy(self.project, self.id, std::path::Path::new(&path))
    }

    /// Detach this item's proxy. The file on disk is left alone — a proxy took
    /// minutes to make, and forgetting it in the project is not a reason to
    /// delete it.
    #[frb(sync)]
    pub fn clear_proxy(&self) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetItemProxy {
                id: self.id,
                proxy: None,
            })
            .map(|_| ())
            .map_err(BridgeError::OpError)
    }

    /// This item's own *use proxy* tick, leaving the proxy attached — how one
    /// clip is checked at full quality without giving up the proxy.
    ///
    /// Refused on an item with no proxy: the panel does not draw the tick on a
    /// row that has nothing to tick.
    #[frb(sync)]
    pub fn set_use_proxy(&self, on: bool) -> Result<(), BridgeError> {
        let proj = self.project()?;
        let proj = proj.write().map_err(|_| BridgeError::WriteFailed)?;
        proj.store
            .commit(lumit_core::Op::SetItemUseProxy {
                id: self.id,
                use_proxy: on,
            })
            .map(|_| ())
            .map_err(BridgeError::OpError)
    }

    /// Where MAKE-PROXY would write this item's proxy: beside the original,
    /// with `_proxy` before a `.mov` extension.
    ///
    /// Shown before the job starts, so a file already there can be pointed out
    /// rather than silently overwritten.
    #[frb(sync)]
    pub fn proxy_path(&self) -> Result<String, BridgeError> {
        let source = self.source_path()?;
        Ok(lumit_render::proxy::proxy_path_for(&source)
            .to_string_lossy()
            .into_owned())
    }

    /// Make this item's proxy: transcode the original half as wide, and attach
    /// the result when it lands.
    ///
    /// Returns as soon as the job is *running*; ask [`proxy_poll`] how it is
    /// getting on. A second one while the first runs is a calm refusal — two
    /// transcodes share one disk.
    #[frb(sync)]
    pub fn make_proxy(&self) -> Result<(), BridgeError> {
        let source = self.source_path()?;
        let dest = lumit_render::proxy::proxy_path_for(&source);
        crate::proxy::start(self.project, self.id, source, dest).map_err(BridgeError::ExportFailed)
    }

    /// The original's own file, which is what a proxy is made from and what the
    /// proxy's name is derived from.
    #[frb(ignore)]
    fn source_path(&self) -> Result<PathBuf, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = proj.store.snapshot();
        let Some(lumit_core::model::ProjectItem::Footage(footage)) = doc.item(self.id) else {
            return Err(BridgeError::InvalidItem);
        };
        Self::resolve_path(&proj, footage).ok_or(BridgeError::MediaPathUnresolved)
    }
}

/// A reference to a file on disk, built the way a relink builds one: the
/// absolute path for this session, the project-relative path where there is a
/// project directory to be relative to (the only one a saved `.lum` carries,
/// K-173), and a content fingerprint so a moved file can be found again.
///
/// Every second file a project can name goes through here — a proxy, and the
/// colour config — so none of them can quietly disagree about what gets written
/// into the file.
#[frb(ignore)]
pub(crate) fn media_ref_at(
    project_dir: Option<&std::path::Path>,
    path: &std::path::Path,
) -> lumit_core::model::MediaRef {
    let mut media = lumit_core::model::MediaRef {
        relative_path: String::new(),
        absolute_path: path.to_string_lossy().into_owned(),
        fingerprint: None,
        extra: serde_json::Map::new(),
    };
    if let Some(dir) = project_dir {
        if let Some(rel) = lumit_project::relative_between(dir, path) {
            media.relative_path = rel;
        }
    }
    media.fingerprint = lumit_project::fingerprint_path(path).ok();
    media
}

/// Point `item`'s proxy at `path`, switched on — the one place a proxy is
/// attached, whether by the picker or by a transcode that has just landed.
#[frb(ignore)]
pub(crate) fn attach_proxy(
    project: Uuid,
    item: Uuid,
    path: &std::path::Path,
) -> Result<(), BridgeError> {
    let proj = {
        let projects = PROJECTS.read().map_err(|_| BridgeError::ReadFailed)?;
        projects
            .get(&project)
            .ok_or(BridgeError::InvalidProject)?
            .clone()
    };

    let media = {
        let p = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = p.store.snapshot();
        if !matches!(
            doc.item(item),
            Some(lumit_core::model::ProjectItem::Footage(_))
        ) {
            return Err(BridgeError::InvalidItem);
        }
        media_ref_at(p.path.as_deref().and_then(|p| p.parent()), path)
    };

    let p = proj.write().map_err(|_| BridgeError::WriteFailed)?;
    p.store
        .commit(lumit_core::Op::SetItemProxy {
            id: item,
            proxy: Some(Box::new(lumit_core::model::ProxyRef {
                media,
                enabled: true,
                extra: serde_json::Map::new(),
            })),
        })
        .map(|_| ())
        .map_err(BridgeError::OpError)
}

/// How the running MAKE-PROXY job is getting on. Safe to call on the
/// interface's own cadence: it drains a channel and reads two numbers.
///
/// Reading is also what *finishes* the job: a transcode that has just landed is
/// attached to its item here, so an item gains its proxy whether or not the
/// panel that asked for it is still on screen.
#[frb(sync)]
pub fn proxy_poll() -> BridgeProxyState {
    match crate::proxy::poll() {
        crate::proxy::State::Idle => BridgeProxyState::Idle,
        crate::proxy::State::Running { frame, total } => BridgeProxyState::Running {
            frame: frame as u64,
            total: total as u64,
        },
        crate::proxy::State::Done { path } => BridgeProxyState::Done { path },
        crate::proxy::State::Failed { error } => BridgeProxyState::Failed { error },
    }
}

/// Ask the running MAKE-PROXY job to stop. The half-written file is removed — a
/// cancelled proxy leaves nothing pretending to be a finished one.
#[frb(sync)]
pub fn proxy_cancel() {
    crate::proxy::cancel();
}
