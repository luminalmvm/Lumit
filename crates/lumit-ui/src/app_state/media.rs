//! Probe/index results for footage items, filled by background threads
//! (moved verbatim from app_state.rs).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use uuid::Uuid;

pub enum MediaStatus {
    Probing,
    Ready {
        probe: lumit_media::MediaProbe,
        frames: usize,
        vfr: bool,
    },
    Failed(String),
}

pub struct MediaRegistry {
    pub map: HashMap<Uuid, MediaStatus>,
    tx: Sender<(Uuid, MediaStatus)>,
    rx: Receiver<(Uuid, MediaStatus)>,
}

impl Default for MediaRegistry {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            map: HashMap::new(),
            tx,
            rx,
        }
    }
}

impl MediaRegistry {
    /// Drain background results into the map. Called once per UI frame.
    pub fn poll(&mut self) {
        while let Ok((id, status)) = self.rx.try_recv() {
            self.map.insert(id, status);
        }
    }

    pub fn any_probing(&self) -> bool {
        self.map.values().any(|s| matches!(s, MediaStatus::Probing))
    }

    /// Probe + build/load the frame index on a background thread
    /// (docs/impl/media-io.md §2 — never on the UI thread, K-017).
    pub fn spawn_probe(&mut self, id: Uuid, path: PathBuf) {
        self.map.insert(id, MediaStatus::Probing);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let status = probe_and_index(&path);
            let _ = tx.send((id, status));
        });
    }
}

fn probe_and_index(path: &std::path::Path) -> MediaStatus {
    let probe = match lumit_media::probe::probe(path) {
        Ok(p) => p,
        Err(e) => return MediaStatus::Failed(e.to_string()),
    };
    // Audio-only items need no frame index.
    if probe.video.is_none() {
        return MediaStatus::Ready {
            probe,
            frames: 0,
            vfr: false,
        };
    }
    let cache_dir = lumit_project::media_index_dir();
    let cached = match (&cache_dir, lumit_media::Fingerprint::of(path)) {
        (Some(dir), Ok(fp)) => lumit_media::FrameIndex::load_cached(dir, &fp),
        _ => None,
    };
    let index = match cached {
        Some(index) => index,
        None => match lumit_media::index::build_frame_index(path) {
            Ok(index) => {
                if let Some(dir) = &cache_dir {
                    let _ = index.save_to(dir);
                }
                index
            }
            Err(e) => return MediaStatus::Failed(e.to_string()),
        },
    };
    MediaStatus::Ready {
        probe,
        frames: index.frame_count(),
        vfr: index.vfr,
    }
}
