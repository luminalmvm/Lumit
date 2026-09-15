//! The addons folder, which is the whole registry: scan it, install into it,
//! remove from it (docs/impl/addons.md §2, §5).
//!
//! There is no preference file recording what is installed, because the
//! directory is the truth and a second record could only disagree with it.
//!
//! # Thread role and contract
//!
//! Disk work on whichever thread asked, and one install at a time across the
//! whole process: [`install`] takes a slot and a second caller is told
//! [`MlError::Busy`] rather than queued (14-ENGINEERING-RULES §1.3, §1.4). No
//! lock is held while a file is copied. The folder override is process-wide
//! and not per thread, because an install runs on a worker thread and a
//! thread-local would not follow it there.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, TryLockError},
};

use crate::{
    error::MlError,
    manifest::{self, Manifest, Task, Unpack},
    runtime,
};

/// The file every addon folder holds.
pub const MANIFEST_FILE: &str = "addon.json";

/// Where an install is assembled before it is renamed into place. Swept on
/// every [`scan`], so a build that died halfway leaves nothing behind.
pub const STAGING: &str = ".staging";

/// The one install in flight. A `Mutex<()>` rather than a flag so the slot is
/// released even when the installing thread unwinds.
static SLOT: Mutex<()> = Mutex::new(());

/// Somewhere other than the user's own addons folder, when a test has said so.
static OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// What the last [`scan`] found, so the callers that ask every frame ask
/// memory rather than the disk. Cleared whenever the folder moves.
static SNAPSHOT: RwLock<Option<Arc<Vec<Installed>>>> = RwLock::new(None);

/// Where addons live: what [`with_dir`] was last given, else the platform's
/// own folder.
#[must_use]
pub fn dir() -> Option<PathBuf> {
    OVERRIDE
        .lock()
        .ok()
        .and_then(|over| over.clone())
        .or_else(lumit_project::addons_dir)
}

/// Point [`dir`] at `dir` instead, or at nothing to put it back.
///
/// Compiled into every build rather than hidden behind `cfg(test)` because
/// the tests on the bridge's side of the seam drive install and remove
/// through the generated surface, and they must not write into the user's own
/// folder either. Nothing in the application calls it.
pub fn with_dir(dir: Option<PathBuf>) {
    if let Ok(mut over) = OVERRIDE.lock() {
        *over = dir;
    }
    forget();
}

/// One addon as the folder has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    /// What its `addon.json` says.
    pub manifest: Manifest,
    /// Its folder.
    pub path: PathBuf,
    /// A file the manifest lists is missing or the wrong size. The row still
    /// shows, with a word saying so, because the fix is to install it again.
    pub broken: bool,
}

/// Every addon installed, by id, with the staging leftovers swept first.
///
/// A folder with no readable `addon.json`, or one naming a different id, is
/// not an addon and is not listed: the folder name is the id, by rule.
#[must_use]
pub fn scan() -> Vec<Installed> {
    sweep();
    let listed = walk();
    // Taken after the disk work, never around it: a scan reads a whole folder
    // of manifests and no other thread may be made to wait on that
    // (14-ENGINEERING-RULES §1.3).
    if let Ok(mut held) = SNAPSHOT.write() {
        *held = Some(Arc::new(listed.clone()));
    }
    listed
}

/// Every addon installed, as the last [`scan`] found them, scanning once if
/// nothing has yet.
///
/// This is what the render path asks. A frame that names a model pack asks
/// which pack on every frame it draws, and walking the addons folder that
/// often would put a directory read inside the frame loop for an answer that
/// changes when the user presses a button (docs/impl/addons.md §6.3). Install,
/// remove and any full scan refresh it, so the page and the render path can
/// never hold two different ideas of what is installed.
///
/// The one thing this does not do is sweep the staging leftovers. That is a
/// recursive delete, and the callers here are the Flow group's engine row and
/// the frame key: neither is a place to delete a folder tree from. [`scan`],
/// which the Addons page calls, is where a leftover goes (§9).
#[must_use]
pub fn snapshot() -> Arc<Vec<Installed>> {
    if let Ok(held) = SNAPSHOT.read() {
        if let Some(listed) = held.as_ref() {
            return Arc::clone(listed);
        }
    }
    let listed = Arc::new(walk());
    if let Ok(mut held) = SNAPSHOT.write() {
        *held = Some(Arc::clone(&listed));
    }
    listed
}

/// How many times the addons folder has changed under this process.
///
/// Anything that keeps something *opened* out of that folder, rather than
/// merely read from it, keeps this number beside it and starts again when it
/// moves: a model opened before a pack was installed would otherwise stay the
/// refusal it was until Lumit restarted, which is not what pressing Install
/// promises.
#[must_use]
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// What [`generation`] counts: an install, a remove, or the folder moving.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Throw the snapshot away, so the next ask walks the folder again, and say
/// that the folder has changed.
fn forget() {
    if let Ok(mut held) = SNAPSHOT.write() {
        *held = None;
    }
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Throw away what an install that died halfway left behind.
///
/// Only when nothing is installing: the slot is what tells a leftover from a
/// job in progress, and the page lists while an install runs.
fn sweep() {
    let Some(root) = dir() else {
        return;
    };
    if let Ok(_slot) = SLOT.try_lock() {
        let _ = fs::remove_dir_all(root.join(STAGING));
    }
}

/// The scan itself, without the snapshot and without the sweep.
fn walk() -> Vec<Installed> {
    let Some(root) = dir() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out: Vec<Installed> = entries
        .flatten()
        .filter_map(|entry| read(&entry.path()))
        .collect();
    out.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    out
}

/// One addon by id, or `None` when it is not installed.
#[must_use]
pub fn pack(id: &str) -> Option<Installed> {
    if !manifest::is_id(id) {
        return None;
    }
    snapshot()
        .iter()
        .find(|installed| installed.manifest.id == id)
        .cloned()
}

/// The first working pack installed for `task`, which is what an effect asks
/// for when its own model row says "whatever is installed".
#[must_use]
pub fn find(task: Task) -> Option<Installed> {
    snapshot()
        .iter()
        .find(|installed| !installed.broken && installed.manifest.task() == Some(task))
        .cloned()
}

/// A stable 32-byte name for one pack on one machine (§7).
///
/// Not a cryptographic digest, and it does not need to be: nothing trusts this
/// number, it only has to differ whenever the picture would. There is no
/// hashing crate in `lumit-ml` and adding one for this would be a dependency
/// bought for a name. What there is instead is the manifest's own SHA-256,
/// which is already thirty-two bytes of the model file's identity, so it is
/// the seed; the pack's id, its version and the provider that will run it are
/// folded over the tail the FNV way.
///
/// The provider folded in is [`runtime::provider`], which is the one a session
/// really took once one has been opened and the one this platform asks for
/// first before that. A machine whose accelerator will not register paints a
/// slightly different picture, and §7 says the key carries the provider that
/// ran, so the name moves the moment the machine's real answer is known: one
/// cache miss, and the truth afterwards.
#[must_use]
pub fn identity(manifest: &Manifest) -> [u8; 32] {
    let mut out = [0u8; 32];
    if let Some(platform) = manifest.platform() {
        for download in &platform.downloads {
            for (slot, pair) in out.iter_mut().zip(download.sha256.as_bytes().chunks(2)) {
                *slot ^= hex_byte(pair);
            }
        }
    }
    let mut fold = 0xcbf2_9ce4_8422_2325_u64;
    for part in [
        manifest.id.as_str(),
        manifest.version.as_str(),
        runtime::provider(),
    ] {
        for byte in part.bytes().chain(std::iter::once(0)) {
            fold = (fold ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    for (slot, byte) in out.iter_mut().skip(24).zip(fold.to_le_bytes()) {
        *slot ^= byte;
    }
    out
}

/// Two hex characters as the byte they spell. The manifest parser has already
/// refused anything that is not sixty-four of them, so a character that is not
/// one cannot get here and reads as zero if it does.
fn hex_byte(pair: &[u8]) -> u8 {
    let nibble = |at: usize| match pair.get(at) {
        Some(c @ b'0'..=b'9') => c - b'0',
        Some(c @ b'a'..=b'f') => c - b'a' + 10,
        Some(c @ b'A'..=b'F') => c - b'A' + 10,
        _ => 0,
    };
    nibble(0) * 16 + nibble(1)
}

/// What the installed pack for `task` is, without opening it, or `None` when
/// none is installed or nothing on this machine could run one.
///
/// Read off the snapshot and then remembered until the addons folder moves,
/// because the frame key asks this on every frame it names: a directory walk
/// per frame is not a thing to put in a render loop, and neither is the whole
/// manifest [`find`] copies out of the snapshot to answer, which is twenty
/// allocations thrown away for a thirty-two byte number (13-PERFORMANCE-RULES,
/// and 14-ENGINEERING-RULES §5 on budgeted allocations).
#[must_use]
pub fn installed_identity(task: Task) -> Option<[u8; 32]> {
    let now = generation();
    let provider = runtime::provider();
    if let Ok(known) = KNOWN.lock() {
        if let Some((at, was, answers)) = known.as_ref() {
            if *at == now && *was == provider {
                if let Some((_, answer)) = answers.iter().find(|(t, _)| *t == task) {
                    return *answer;
                }
            }
        }
    }
    let answer = named_pack(task);
    // Taken again rather than held across the reading above: the first reading
    // of a run walks the addons folder, and no other thread may be made to
    // wait on that (14-ENGINEERING-RULES §1.3).
    if let Ok(mut known) = KNOWN.lock() {
        if known
            .as_ref()
            .is_none_or(|(at, was, _)| *at != now || *was != provider)
        {
            *known = Some((now, provider, Vec::new()));
        }
        if let Some((_, _, answers)) = known.as_mut() {
            answers.retain(|(t, _)| *t != task);
            answers.push((task, answer));
        }
    }
    answer
}

/// What [`installed_identity`] worked out per task, the addons-folder
/// generation it was true of, and the provider it folded in: that one can move
/// once, the first time a session is built, and the answer moves with it.
type Known = Option<(u64, &'static str, Vec<(Task, Option<[u8; 32]>)>)>;

/// The last such answer.
static KNOWN: Mutex<Known> = Mutex::new(None);

/// The reading itself, off the snapshot.
fn named_pack(task: Task) -> Option<[u8; 32]> {
    // A pack with nothing to run it paints nothing, so those frames are the
    // built-in engine's and have to be named as the built-in engine's. The
    // moment the runtime arrives the folder's generation moves and the key
    // flips to the pack, which is what pressing Install promises (§7).
    if matches!(runtime::status(), runtime::RuntimeStatus::Missing) {
        return None;
    }
    find(task).map(|installed| identity(&installed.manifest))
}

/// Read one folder as an addon, or `None` when it is not one.
fn read(path: &Path) -> Option<Installed> {
    let id = path.file_name()?.to_str()?;
    if !manifest::is_id(id) {
        return None;
    }
    let text = fs::read_to_string(path.join(MANIFEST_FILE)).ok()?;
    let manifest = manifest::parse(&text).ok()?;
    if manifest.id != id {
        return None;
    }
    let broken = manifest
        .expected()
        .iter()
        .any(|(name, size)| !file_is_there(&path.join(name), *size));
    Some(Installed {
        manifest,
        path: path.to_path_buf(),
        broken,
    })
}

/// Whether a file the manifest lists is there and, where the manifest knows
/// how big it should be, that big. A whole re-hash of a two-hundred-megabyte
/// file on every scan is what this deliberately does not do (§4).
fn file_is_there(path: &Path, size: Option<u64>) -> bool {
    match fs::metadata(path) {
        Ok(meta) => meta.is_file() && size.is_none_or(|want| meta.len() == want),
        Err(_) => false,
    }
}

/// Install an addon from a manifest and the files somebody already fetched
/// and verified, one file per download of this platform's block, in order.
///
/// Nothing is written into the addon's own folder directly: the files are
/// assembled under [`STAGING`] and renamed into place only once every one of
/// them is there, and the install that was there is moved aside rather than
/// deleted, so a failure part way leaves the previous install exactly as it
/// was (§9).
///
/// # Errors
///
/// [`MlError::Busy`] when another install is running, [`MlError::RuntimeInUse`]
/// when the runtime being replaced has already loaded, and
/// [`MlError::Invalid`] with a sentence for a manifest this build will not
/// take, a file list that does not match it, a file that is not the size the
/// manifest says, a zip missing an entry, or a folder that will not be written.
pub fn install(manifest_text: &str, files: &[PathBuf]) -> Result<Installed, MlError> {
    let _slot = match SLOT.try_lock() {
        Ok(slot) => slot,
        Err(TryLockError::Poisoned(held)) => held.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(MlError::Busy),
    };

    let manifest = manifest::parse(manifest_text)?;
    if manifest.id == runtime::RUNTIME_ID && runtime::loaded() {
        return Err(MlError::RuntimeInUse);
    }
    let root = dir().ok_or_else(|| {
        MlError::Invalid("there is nowhere on this machine to keep an addon".into())
    })?;
    let platform = manifest
        .platform()
        .ok_or_else(|| MlError::Invalid("this addon has nothing for this platform".into()))?;
    if files.len() != platform.downloads.len() {
        return Err(MlError::Invalid(format!(
            "this addon needs {} files and {} were handed over",
            platform.downloads.len(),
            files.len()
        )));
    }

    let staging = root
        .join(STAGING)
        .join(format!("{}-{}", manifest.id, nonce()));
    fs::create_dir_all(&staging).map_err(|e| MlError::Invalid(e.to_string()))?;

    for (download, file) in platform.downloads.iter().zip(files) {
        let placed = match download.unpack {
            Unpack::File => copy_file(file, download.size, &staging.join(&download.dest)),
            Unpack::Zip => take_from_zip(file, download.size, &download.entries, &staging),
        };
        if let Err(why) = placed {
            return Err(abandon(&staging, why));
        }
    }

    if let Err(why) = write_manifest(&staging, manifest_text) {
        return Err(abandon(&staging, why));
    }

    // The install that is there is moved aside rather than deleted, or a
    // rename that will not go through would leave the user with nothing at
    // all. Windows turns a rename down while any file under the tree is open,
    // and a scanner walking a freshly written library holds one open.
    let home = root.join(&manifest.id);
    let aside = home.exists().then(|| {
        root.join(STAGING)
            .join(format!("{}-old-{}", manifest.id, nonce()))
    });
    if let Some(aside) = &aside {
        if let Err(e) = fs::rename(&home, aside) {
            return Err(abandon(&staging, MlError::Invalid(e.to_string())));
        }
    }
    if let Err(e) = fs::rename(&staging, &home) {
        if let Some(aside) = &aside {
            let _ = fs::rename(aside, &home);
        }
        return Err(abandon(&staging, MlError::Invalid(e.to_string())));
    }
    if let Some(aside) = &aside {
        let _ = fs::remove_dir_all(aside);
    }
    let _ = fs::remove_dir(root.join(STAGING));

    // The folder has changed under everything that remembers it, so the
    // snapshot is thrown away and taken again here, on the thread that changed
    // it: the next reader is the engine row or the frame key, and neither of
    // those is a place to walk a folder from.
    forget();
    let _ = scan();
    read(&home).ok_or(MlError::PackUnreadable)
}

/// Delete an addon's folder. The row goes; nothing else is touched.
///
/// # Errors
///
/// [`MlError::NotInstalled`] when there is no addon of that id,
/// [`MlError::RuntimeInUse`] when the runtime has already loaded, and
/// [`MlError::Invalid`] when the folder will not delete.
pub fn remove(id: &str) -> Result<(), MlError> {
    if !manifest::is_id(id) {
        return Err(MlError::NotInstalled);
    }
    // Turned down before a single file goes. ONNX Runtime keeps its handle for
    // the life of the process, so a delete would take the licence files and
    // the provider library and then stop at the one that is locked, leaving a
    // folder that scans as broken and cannot be mended until Lumit restarts.
    if id == runtime::RUNTIME_ID && runtime::loaded() {
        return Err(MlError::RuntimeInUse);
    }
    let home = dir().ok_or(MlError::NotInstalled)?.join(id);
    if !home.is_dir() {
        return Err(MlError::NotInstalled);
    }
    let gone = fs::remove_dir_all(&home).map_err(|e| MlError::Invalid(e.to_string()));
    // Whether or not it went: a half-deleted folder is not the one the
    // snapshot remembers either. Taken again here for the reason install takes
    // it again, on the thread that pressed the button.
    forget();
    let _ = scan();
    gone
}

/// Copy one whole download into the staging folder, refusing a file that is
/// not the size the manifest says it is.
fn copy_file(from: &Path, size: u64, to: &Path) -> Result<(), MlError> {
    let meta = fs::metadata(from).map_err(|e| MlError::Invalid(e.to_string()))?;
    if meta.len() != size {
        return Err(MlError::Invalid(format!(
            "{} is {} bytes and the manifest says {size}",
            from.display(),
            meta.len()
        )));
    }
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| MlError::Invalid(e.to_string()))
}

/// Take exactly the named entries out of a zip and no others. An entry the
/// zip does not have is a refusal, not a smaller install.
fn take_from_zip(
    from: &Path,
    size: u64,
    entries: &std::collections::BTreeMap<String, String>,
    into: &Path,
) -> Result<(), MlError> {
    let file = fs::File::open(from).map_err(|e| MlError::Invalid(e.to_string()))?;
    let meta = file
        .metadata()
        .map_err(|e| MlError::Invalid(e.to_string()))?;
    if meta.len() != size {
        return Err(MlError::Invalid(format!(
            "{} is {} bytes and the manifest says {size}",
            from.display(),
            meta.len()
        )));
    }
    let mut zip = zip::ZipArchive::new(file).map_err(|e| MlError::Invalid(e.to_string()))?;
    for (inside, dest) in entries {
        let mut entry = zip
            .by_name(inside)
            .map_err(|_| MlError::Invalid(format!("the download has no \"{inside}\" in it")))?;
        let mut out =
            fs::File::create(into.join(dest)).map_err(|e| MlError::Invalid(e.to_string()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| MlError::Invalid(e.to_string()))?;
        out.flush().map_err(|e| MlError::Invalid(e.to_string()))?;
    }
    Ok(())
}

/// Write the catalogue's own text into the folder, unchanged, so the
/// installed manifest and the catalogue entry are the same object (§4).
fn write_manifest(into: &Path, text: &str) -> Result<(), MlError> {
    fs::write(into.join(MANIFEST_FILE), text).map_err(|e| MlError::Invalid(e.to_string()))
}

/// Throw the half-built install away and keep the refusal that caused it. The
/// staging folder itself goes too when nothing else is using it, so a refusal
/// leaves the addons folder exactly as it found it.
fn abandon(staging: &Path, why: MlError) -> MlError {
    let _ = fs::remove_dir_all(staging);
    if let Some(parent) = staging.parent() {
        let _ = fs::remove_dir(parent);
    }
    why
}

/// Enough to tell two staging folders apart, including one a dead build left.
fn nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    use crate::test_support::serially;

    /// A manifest for a pack with one plain-file download of `bytes` bytes.
    fn one_file(id: &str, bytes: u64) -> String {
        format!(
            r#"{{"format":1,"id":"{id}","kind":"model","name":"Test pack","version":"1",
               "licence":"MIT","platforms":{{"any":{{"downloads":[
                 {{"url":"https://example.invalid/m.onnx",
                   "sha256":"0000000000000000000000000000000000000000000000000000000000000001",
                   "size":{bytes},"unpack":"file","dest":"model.onnx"}}]}}}},
               "model":{{"task":"depth","arch":"depth-anything","file":"model.onnx"}}}}"#
        )
    }

    /// A manifest for a pack whose one download is a zip with two entries.
    fn one_zip(id: &str, bytes: u64) -> String {
        format!(
            r#"{{"format":1,"id":"{id}","kind":"model","name":"Test pack","version":"1",
               "licence":"Apache-2.0","platforms":{{"any":{{"downloads":[
                 {{"url":"https://example.invalid/p.zip",
                   "sha256":"0000000000000000000000000000000000000000000000000000000000000002",
                   "size":{bytes},"unpack":"zip",
                   "entries":{{"inner/encoder.onnx":"encoder.onnx",
                               "inner/decoder.onnx":"decoder.onnx"}}}}]}}}},
               "model":{{"task":"segmentation","arch":"sam2",
                         "encoder":"encoder.onnx","decoder":"decoder.onnx"}}}}"#
        )
    }

    /// Write `bytes` bytes to a file and hand back its path.
    fn a_file(dir: &Path, name: &str, bytes: usize) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, vec![7u8; bytes]).unwrap();
        path
    }

    /// A zip holding each of `entries`, with a third file beside them that no
    /// manifest names.
    fn a_zip(dir: &Path, name: &str, entries: &[&str]) -> PathBuf {
        let path = dir.join(name);
        let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for entry in entries.iter().chain(["inner/README.txt"].iter()) {
            writer.start_file(*entry, options).unwrap();
            writer.write_all(entry.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    /// **An empty folder lists nothing, and a stale staging folder is swept by
    /// the scan and by nothing else.** The sweep is the other half of the
    /// staging rename: a build that died halfway leaves a folder, and the next
    /// scan is what clears it. The snapshot must not be that: it is read from
    /// the Flow group's engine row and from the frame key, and a recursive
    /// delete belongs in neither.
    #[test]
    fn the_scan_lists_nothing_and_sweeps_the_staging_folder() {
        let _serial = serially();
        let root = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));

        let stale = root.path().join(STAGING).join("rife-1");
        fs::create_dir_all(&stale).unwrap();
        assert!(snapshot().is_empty(), "an empty folder holds no addons");
        assert!(stale.exists(), "and the snapshot deleted nothing to say so");

        assert!(scan().is_empty(), "the scan reads the same empty folder");
        assert!(
            !root.path().join(STAGING).exists(),
            "the staging folder is swept"
        );
        with_dir(None);
    }

    /// **Two packs list as two, and a pack missing its file lists as broken.**
    /// Broken rather than absent, because the row is how the user is told to
    /// install it again.
    #[test]
    fn the_scan_lists_two_packs_and_marks_a_missing_file_broken() {
        let _serial = serially();
        let root = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));

        for (id, size) in [("alpha", 8u64), ("beta", 8)] {
            let home = root.path().join(id);
            fs::create_dir_all(&home).unwrap();
            fs::write(home.join(MANIFEST_FILE), one_file(id, size)).unwrap();
            fs::write(home.join("model.onnx"), vec![0u8; 8]).unwrap();
        }
        let listed = scan();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|one| !one.broken));
        assert_eq!(listed[0].manifest.id, "alpha", "listed by id");

        fs::remove_file(root.path().join("alpha").join("model.onnx")).unwrap();
        fs::write(root.path().join("beta").join("model.onnx"), vec![0u8; 9]).unwrap();
        let listed = scan();
        assert!(listed[0].broken, "a missing file is broken");
        assert!(listed[1].broken, "a file of the wrong size is broken");
        with_dir(None);
    }

    /// **An install from a plain file and from a zip puts exactly the named
    /// files in place, and nothing else.** The zip's own paths are thrown
    /// away: only the entries the manifest names come out, under the names it
    /// gives them.
    #[test]
    fn an_install_places_the_named_files_and_no_others() {
        let _serial = serially();
        let root = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));

        let plain = a_file(downloads.path(), "m.onnx", 12);
        let installed = install(&one_file("plain", 12), &[plain]).unwrap();
        assert!(!installed.broken);
        assert_eq!(
            held(&root.path().join("plain")),
            vec!["addon.json", "model.onnx"]
        );

        let zipped = a_zip(
            downloads.path(),
            "p.zip",
            &["inner/encoder.onnx", "inner/decoder.onnx"],
        );
        let size = fs::metadata(&zipped).unwrap().len();
        install(&one_zip("zipped", size), &[zipped]).unwrap();
        assert_eq!(
            held(&root.path().join("zipped")),
            vec!["addon.json", "decoder.onnx", "encoder.onnx"]
        );
        assert_eq!(scan().len(), 2);
        with_dir(None);
    }

    /// **A second install of the same id replaces the first, and a remove
    /// deletes it.** The folder is the registry, so replacing is two renames:
    /// the old one aside, the new one into place, and only then is the old one
    /// thrown away. What is left afterwards is one folder and no staging.
    #[test]
    fn an_install_replaces_and_a_remove_deletes() {
        let _serial = serially();
        let root = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));

        install(
            &one_file("twice", 12),
            &[a_file(downloads.path(), "first.onnx", 12)],
        )
        .unwrap();
        install(
            &one_file("twice", 20),
            &[a_file(downloads.path(), "second.onnx", 20)],
        )
        .unwrap();
        assert_eq!(scan().len(), 1, "one folder, not two");
        assert_eq!(
            fs::metadata(root.path().join("twice").join("model.onnx"))
                .unwrap()
                .len(),
            20,
            "the second install won"
        );
        assert!(
            !root.path().join(STAGING).exists(),
            "the one that was moved aside went with it"
        );

        remove("twice").unwrap();
        assert!(scan().is_empty());
        assert_eq!(remove("twice").unwrap_err(), MlError::NotInstalled);
        assert_eq!(remove("no such addon").unwrap_err(), MlError::NotInstalled);
        with_dir(None);
    }

    /// **A failure after the first file leaves no folder and the old install
    /// untouched.** This is what the staging rename is for: the second
    /// download of the second install is a zip missing an entry, and the pack
    /// that was already there is still the one that was already there.
    #[test]
    fn a_failure_part_way_leaves_the_old_install_alone() {
        let _serial = serially();
        let root = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));

        install(
            &one_file("sam2", 12),
            &[a_file(downloads.path(), "good.onnx", 12)],
        )
        .unwrap();

        // The decoder is taken first (the entries are read in order) and the
        // encoder is not in the zip at all, so the install fails having
        // already placed a file.
        let half = a_zip(downloads.path(), "half.zip", &["inner/decoder.onnx"]);
        let size = fs::metadata(&half).unwrap().len();

        let refusal = install(&one_zip("sam2", size), &[half]).unwrap_err();
        let MlError::Invalid(why) = &refusal else {
            panic!("the wrong refusal: {refusal:?}");
        };
        assert!(why.contains("encoder.onnx"), "{why}");
        assert!(
            !root.path().join(STAGING).exists(),
            "the staging folder went with it"
        );
        let listed = scan();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].manifest.task(), Some(Task::Depth), "the old one");
        with_dir(None);
    }

    /// **A file that is not the size the manifest says is refused.** The
    /// download side verifies the digest; this is the engine's own check that
    /// the file it was handed is the file the manifest describes.
    #[test]
    fn a_file_of_the_wrong_size_is_refused() {
        let _serial = serially();
        let root = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));

        let refusal = install(
            &one_file("wrong", 12),
            &[a_file(downloads.path(), "short.onnx", 5)],
        )
        .unwrap_err();
        assert!(matches!(refusal, MlError::Invalid(_)), "{refusal:?}");
        assert!(scan().is_empty());
        with_dir(None);
    }

    /// **A second install while one is running is refused, never queued.**
    /// Holding the slot is exactly the state a running install leaves the
    /// process in, so this is the same refusal a second press meets.
    #[test]
    fn a_second_install_is_refused_rather_than_queued() {
        let _serial = serially();
        let root = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));

        let held = SLOT.lock().unwrap_or_else(|taken| taken.into_inner());
        let refusal = install(
            &one_file("busy", 12),
            &[a_file(downloads.path(), "m.onnx", 12)],
        )
        .unwrap_err();
        assert_eq!(refusal, MlError::Busy);
        drop(held);

        assert!(scan().is_empty(), "the refused install wrote nothing");
        with_dir(None);
    }

    /// **A pack is found by task, and a broken one is not.** This is what an
    /// effect asks before it decides whether to wear the missing-addon badge.
    #[test]
    fn a_pack_is_found_by_task_unless_it_is_broken() {
        let _serial = serially();
        let root = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));

        install(
            &one_file("depth-pack", 12),
            &[a_file(downloads.path(), "m.onnx", 12)],
        )
        .unwrap();
        assert_eq!(
            find(Task::Depth).map(|one| one.manifest.id),
            Some("depth-pack".into())
        );
        assert!(find(Task::Matte).is_none());
        assert!(pack("depth-pack").is_some());
        assert!(pack("nothing-here").is_none());

        fs::remove_file(root.path().join("depth-pack").join("model.onnx")).unwrap();
        // A file that goes behind the store's back is noticed by the next
        // scan, which is what the page does whenever it comes forward; the
        // render path reads that scan rather than the folder.
        assert!(scan()[0].broken);
        assert!(find(Task::Depth).is_none(), "a broken pack is not offered");
        with_dir(None);
    }

    /// **The snapshot answers without touching the disk, and every change to
    /// the folder refreshes it.** A frame that names a model pack asks which
    /// pack on every frame it draws, so the answer has to come from memory;
    /// an install or a remove that left it stale would draw with a pack that
    /// is no longer there (docs/impl/addons.md §6.3).
    #[test]
    fn the_snapshot_is_refreshed_by_an_install_and_a_remove() {
        let _serial = serially();
        let root = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));

        assert!(snapshot().is_empty(), "a folder with nothing in it");
        install(
            &one_file("snap", 12),
            &[a_file(downloads.path(), "m.onnx", 12)],
        )
        .unwrap();
        assert_eq!(snapshot().len(), 1, "the install refreshed it");

        // Written straight into the folder, which is exactly what the snapshot
        // is allowed not to see until something scans.
        let home = root.path().join("unseen");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join(MANIFEST_FILE), one_file("unseen", 8)).unwrap();
        fs::write(home.join("model.onnx"), vec![0u8; 8]).unwrap();
        assert_eq!(snapshot().len(), 1, "no disk read per ask");
        assert_eq!(scan().len(), 2, "and a scan is what notices");
        assert_eq!(snapshot().len(), 2);

        let was = generation();
        remove("snap").unwrap();
        assert_eq!(snapshot().len(), 1, "the remove refreshed it");
        assert!(
            generation() != was,
            "and said so, for whatever has a pack open"
        );
        with_dir(None);
        assert!(
            !snapshot()
                .iter()
                .any(|one| one.path.starts_with(root.path())),
            "moving the folder forgets what was in the old one"
        );
    }

    /// The file names one folder holds, sorted, so a test can say exactly
    /// what an install put there.
    fn held(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        names
    }

    /// The published catalogue, from `LUMIT_ADDONS_INDEX`, on a machine that
    /// has the `lumit-addons` repository checked out: each addon's id and its
    /// manifest as text.
    fn catalogue() -> Option<Vec<(String, String)>> {
        let path = PathBuf::from(std::env::var_os("LUMIT_ADDONS_INDEX")?);
        let text = fs::read_to_string(path).ok()?;
        let index: serde_json::Value = serde_json::from_str(&text).ok()?;
        let entries = index.get("addons")?.as_array()?;
        Some(
            entries
                .iter()
                .map(|entry| {
                    let id = entry["id"].as_str().unwrap_or_default().to_owned();
                    (id, entry.to_string())
                })
                .collect(),
        )
    }

    /// **Every manifest the catalogue publishes parses here and has a block
    /// for this machine.** The catalogue lives in another repository, so this
    /// is the one place the two are held to each other; it skips where that
    /// repository is not checked out.
    #[test]
    fn the_published_catalogue_parses_and_serves_this_machine() {
        let Some(entries) = catalogue() else {
            eprintln!("skipping: LUMIT_ADDONS_INDEX is not set");
            return;
        };
        assert!(
            entries.len() >= 6,
            "the catalogue lists the six first addons"
        );
        for (id, text) in &entries {
            let parsed = manifest::parse(text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(parsed.id, *id);
            assert!(
                parsed.platform().is_some(),
                "{id} has no block for this machine"
            );
        }
    }

    /// **The runtime installs from the real packages the catalogue names.**
    /// The NuGet packages sit in the scratch cache under fixed names, so the
    /// unpack runs on the archives users will actually download rather than
    /// on a zip a test wrote.
    #[test]
    fn the_runtime_installs_from_the_real_packages() {
        let _serial = serially();
        let (Some(entries), Some(cache)) = (catalogue(), crate::test_support::packs_dir()) else {
            eprintln!("skipping: LUMIT_ADDONS_INDEX or LUMIT_ML_PACKS_DIR is not set");
            return;
        };
        let Some((_, text)) = entries.iter().find(|(id, _)| id == runtime::RUNTIME_ID) else {
            panic!("the catalogue has no runtime");
        };
        let parsed = manifest::parse(text).unwrap();
        let platform = parsed.platform().expect("a runtime block for this machine");
        let files: Vec<PathBuf> = platform
            .downloads
            .iter()
            .map(|download| {
                let name = if download.url.contains("OnnxRuntime.DirectML") {
                    "onnxruntime-directml-1.24.4.nupkg"
                } else if download.url.contains("AI.DirectML") {
                    "directml-1.15.4.nupkg"
                } else {
                    "onnxruntime-cpu-1.24.4.nupkg"
                };
                cache.join(name)
            })
            .collect();
        if !files.iter().all(|file| file.is_file()) {
            eprintln!("skipping: the packages are not in the cache");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        with_dir(Some(root.path().to_path_buf()));
        let installed = install(text, &files).unwrap();
        for (name, _) in parsed.expected() {
            assert!(
                installed.path.join(&name).is_file(),
                "{name} was not placed"
            );
        }
        let library = installed
            .path
            .join(runtime::LIBRARY)
            .metadata()
            .unwrap()
            .len();
        assert!(
            library > 1_000_000,
            "the library is the real one, not a stub"
        );
        with_dir(None);
    }
}
