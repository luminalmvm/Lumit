//! The write-behind disk cache tier for comp frames (moved verbatim from
//! app_state.rs).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Default disk budget (docs/06 §5.4; user-set cap arrives with settings).
pub const DEFAULT_CAP_BYTES: u64 = 50 * 1024 * 1024 * 1024;

pub enum Cmd {
    /// Point the cache at a project's sidecar (None = unsaved: disabled).
    SetRoot(Option<PathBuf>),
    /// Park a rendered frame (write-behind).
    Store(u128, u32, u32, Vec<u8>),
    /// Bring a frame back for the RAM tier.
    Load(u128),
    /// Set the byte cap (Settings → Performance). Remembered so it also
    /// applies to the cache opened on the next `SetRoot`.
    SetCap(u64),
}

pub struct DiskIo {
    pub tx: Sender<Cmd>,
    pub loaded: Receiver<(u128, lumit_cache::disk::DiskFrame)>,
    /// Hashes present on disk, mirrored by the worker.
    pub known: Arc<Mutex<HashSet<u128>>>,
}

/// Spawn the worker. It exits when the sender side drops.
pub fn spawn() -> DiskIo {
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    let (loaded_tx, loaded) = std::sync::mpsc::channel();
    let known: Arc<Mutex<HashSet<u128>>> = Arc::default();
    let known_worker = known.clone();
    std::thread::Builder::new()
        .name("nebula-disk".into())
        .spawn(move || {
            let mut cache: Option<lumit_cache::disk::DiskCache> = None;
            // The desired cap, so it survives project switches (a fresh
            // cache is opened per `SetRoot`) rather than resetting.
            let mut cap = DEFAULT_CAP_BYTES;
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    Cmd::SetRoot(root) => {
                        cache = root.map(|r| lumit_cache::disk::DiskCache::open(r, cap));
                        let hashes = cache.as_ref().map(|c| c.known_hashes()).unwrap_or_default();
                        if let Ok(mut k) = known_worker.lock() {
                            k.clear();
                            k.extend(hashes);
                        }
                    }
                    Cmd::SetCap(bytes) => {
                        cap = bytes;
                        if let Some(c) = &mut cache {
                            c.set_cap(bytes);
                        }
                    }
                    Cmd::Store(hash, w, h, rgba) => {
                        if let Some(c) = &mut cache {
                            c.store(hash, w, h, &rgba);
                            if c.contains(hash) {
                                if let Ok(mut k) = known_worker.lock() {
                                    k.insert(hash);
                                }
                            }
                        }
                    }
                    Cmd::Load(hash) => {
                        let frame = cache.as_mut().and_then(|c| c.load(hash));
                        match frame {
                            Some(f) => {
                                let _ = loaded_tx.send((hash, f));
                            }
                            None => {
                                // Missing or corrupt-discarded: unmirror it
                                // so the fill falls back to rendering.
                                if let Ok(mut k) = known_worker.lock() {
                                    k.remove(&hash);
                                }
                            }
                        }
                    }
                }
            }
        })
        .ok();
    DiskIo { tx, loaded, known }
}
