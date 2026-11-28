//! Footage carried inside the `.lum` (docs/01-GLOSSARY.md: Packed project):
//! packing it in on a save, reading it back out on an open, and writing it
//! beside the project again on an unpack. Proxies, the colour config and the
//! files effects read ride along as if each were one more footage item
//! (`lumit_core::model::packed_id`, `packed_file_id`).
//!
//! Runs on whichever thread saves or opens the project, never the UI thread.
//! Every copy here is as long as the footage is big, so each one reports how
//! far it has got and stops when it is told to.
//!
//! A packed file is one stored (uncompressed) archive entry,
//! `media/<folder>/<file name>`. Footage is compressed already, so deflating
//! it again would cost minutes and save nothing. An image sequence's files
//! share one folder. The folder is named for the item that was packed first
//! and means nothing beyond keeping two files of one name apart. A colour
//! config's look-up tables keep their own folders below it, because the config
//! finds them by those.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use lumit_core::model::Document;
use lumit_core::model::{Fingerprint, MediaRef, PackedMedia};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{fingerprint_path, relative_between, Manifest, ProjectError};

pub const MEDIA_PREFIX: &str = "media/";

const CHUNK: usize = 1 << 20;

/// Called with the bytes copied so far and the bytes there are to copy.
/// Answering `false` stops the job, which then returns
/// [`ProjectError::Cancelled`] and leaves nothing half-written behind.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64) -> bool;

#[derive(Debug, Clone)]
pub struct PackSource {
    pub item: Uuid,
    /// The file the item's reference names, then the rest of its numbered run
    /// when the item is an image sequence, the files a 3D model pulls in, or
    /// the look-up tables a colour config names. Never empty. A file below the first one's folder keeps
    /// its path from there.
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct Packed {
    /// The packed map as the file holds it: the items whose bytes are in the
    /// archive, and nothing else.
    pub packed: BTreeMap<Uuid, PackedMedia>,
    pub files: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Unpacked {
    pub moved: Vec<(Uuid, MediaRef)>,
    /// The items whose bytes could not be written out. They stay packed, so
    /// the next save still carries them.
    pub kept: BTreeMap<Uuid, PackedMedia>,
    pub written: u64,
}

#[derive(Clone)]
enum Held {
    Archive { archive: usize, index: usize },
    Disk(PathBuf),
}

#[derive(Clone)]
struct HeldFile {
    name: String,
    size: u64,
    from: Held,
}

/// The archives a job reads from, in the caller's order. `None` stands for
/// one that is missing or would not open.
type Zips = Vec<Option<ZipArchive<File>>>;

type HeldFolders = HashMap<String, Vec<HeldFile>>;

/// `media/<folder>/<name>` taken apart, or `None` for an entry that is not
/// one. Every part has to be a plain name: an archive is somebody else's
/// file, and a name with a `..` or a drive in it would be written wherever it
/// pointed. The name may sit in folders of its own below `<folder>`.
fn split_entry(entry: &str) -> Option<(&str, &str)> {
    let (folder, name) = entry.strip_prefix(MEDIA_PREFIX)?.split_once('/')?;
    (plain_name(folder) && plain_path(name)).then_some((folder, name))
}

pub(crate) fn plain_name(name: &str) -> bool {
    // Windows drops a dot or a space off the end of a name, which would make
    // `.. ` the folder above.
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name.contains(['/', '\\', ':', '\0'])
}

fn plain_path(name: &str) -> bool {
    name.split('/').all(plain_name)
}

/// What `file` is called in a packed folder whose first file sits in `dir`:
/// its path from there with forward slashes, or its bare name when it is
/// somewhere else.
fn name_under(dir: &Path, file: &Path) -> Option<String> {
    let below = file
        .strip_prefix(dir)
        .ok()
        .or(file.file_name().map(Path::new))?;
    let parts: Vec<_> = below.iter().map(|part| part.to_string_lossy()).collect();
    Some(parts.join("/"))
}

/// The stored media files of an archive, by folder.
///
/// Only stored entries count. That is all a save writes, and a stored entry
/// cannot expand past its own size in the file. Together they cannot come to
/// more than the archive's own `length` either, unless the same bytes are
/// listed under many names, which no save writes. So the listing stops there,
/// and reading it back never writes more than the archive holds.
fn media_index(archive: usize, zip: &mut ZipArchive<File>, length: u64, into: &mut HeldFolders) {
    let mut found = HeldFolders::new();
    let mut listed = 0u64;
    for index in 0..zip.len() {
        let Ok(entry) = zip.by_index_raw(index) else {
            continue;
        };
        if entry.is_dir() || entry.compression() != CompressionMethod::Stored {
            continue;
        }
        let Ok(entry_name) = entry.name() else {
            continue;
        };
        let Some((folder, name)) = split_entry(&entry_name) else {
            continue;
        };
        listed = listed.saturating_add(entry.size());
        if listed > length {
            break;
        }
        found.entry(folder.to_owned()).or_default().push(HeldFile {
            name: name.to_owned(),
            size: entry.size(),
            from: Held::Archive { archive, index },
        });
    }
    // The first archive that has a folder answers for it.
    for (folder, files) in found {
        into.entry(folder).or_insert(files);
    }
}

/// Open each archive that is there and index its media. A missing or
/// unreadable archive simply holds nothing.
fn open_archives(paths: &[PathBuf]) -> (Zips, HeldFolders) {
    let mut held = HeldFolders::new();
    let mut zips = Vec::with_capacity(paths.len());
    for (n, path) in paths.iter().enumerate() {
        let length = fs::metadata(path).map_or(0, |meta| meta.len());
        let mut zip = File::open(path).ok().and_then(|f| ZipArchive::new(f).ok());
        if let Some(zip) = zip.as_mut() {
            media_index(n, zip, length, &mut held);
        }
        zips.push(zip);
    }
    (zips, held)
}

/// Why a copy stopped. The two ends are told apart because they mean
/// different things to a save: a source that will not read costs one item its
/// place in the archive, and a destination that will not write is the save
/// failing.
enum CopyError {
    Read(std::io::Error),
    Write(std::io::Error),
    Cancelled,
}

impl From<CopyError> for ProjectError {
    fn from(e: CopyError) -> Self {
        match e {
            CopyError::Read(e) | CopyError::Write(e) => ProjectError::Io(e),
            CopyError::Cancelled => ProjectError::Cancelled,
        }
    }
}

fn copy_chunks(
    from: &mut dyn Read,
    to: &mut dyn Write,
    size: u64,
    done: &mut u64,
    total: u64,
    progress: Progress<'_>,
) -> Result<(), CopyError> {
    let mut buf = vec![0u8; CHUNK];
    let mut left = size;
    while left > 0 {
        let want = usize::try_from(left).unwrap_or(CHUNK).min(CHUNK);
        let got = from.read(&mut buf[..want]).map_err(CopyError::Read)?;
        if got == 0 {
            return Err(CopyError::Read(std::io::ErrorKind::UnexpectedEof.into()));
        }
        to.write_all(&buf[..got]).map_err(CopyError::Write)?;
        left -= got as u64;
        *done += got as u64;
        if !progress(*done, total) {
            return Err(CopyError::Cancelled);
        }
    }
    Ok(())
}

/// Whether the files of a source on disk are the files a packed folder holds:
/// the same names at the same sizes. The first file's fingerprint is checked
/// by the caller. The rest of a run is compared this way because hashing two
/// thousand frames on every save is the cost packing is meant to avoid.
fn same_files(files: &[PathBuf], held: &[HeldFile]) -> bool {
    let dir = files.first().and_then(|first| first.parent());
    files.len() == held.len()
        && files.iter().all(|file| {
            let name = dir.and_then(|dir| name_under(dir, file));
            let size = fs::metadata(file).map(|m| m.len()).ok();
            held.iter()
                .any(|h| Some(h.name.as_str()) == name.as_deref() && Some(h.size) == size)
        })
}

fn free_folder(item: Uuid, claimed: &mut HashSet<String>) -> String {
    let base = item.to_string();
    if claimed.insert(base.clone()) {
        return base;
    }
    let mut n = 1u32;
    loop {
        let candidate = format!("{base}-{n}");
        if claimed.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Save `doc` to `path` with footage inside the archive.
///
/// Two sets of items are packed. Every item `doc.packed` names is carried
/// forward, and every item `sources` offers is packed whether it was before
/// or not. `previous` is the project's file as it stands, which is where an
/// item's bytes are copied from when the file on disk still matches them or
/// is gone. A file on disk that has changed since it was packed is packed
/// again from disk, so the archive holds what the project is showing.
///
/// An item that can be found nowhere is left out and reads as unpacked from
/// then on. The answer says what the file ended up holding, and the caller
/// records it in the open document.
///
/// Atomic like [`crate::save`]: a failure or a cancel leaves `path` as it
/// was, and `previous` may be `path` itself.
pub fn save_packed(
    doc: &Document,
    path: &Path,
    previous: Option<&Path>,
    sources: &[PackSource],
    progress: Progress<'_>,
) -> Result<Packed, ProjectError> {
    let manifest = Manifest::current();
    save_packed_with(
        doc,
        path,
        previous,
        sources,
        &manifest,
        &mut |_, _| Ok(()),
        progress,
    )
}

pub(crate) type MoreEntries<'a> = &'a mut dyn FnMut(
    &mut ZipWriter<File>,
    &BTreeMap<Uuid, PackedMedia>,
) -> Result<(), ProjectError>;

pub(crate) fn save_packed_with(
    doc: &Document,
    path: &Path,
    previous: Option<&Path>,
    sources: &[PackSource],
    manifest: &Manifest,
    more: MoreEntries<'_>,
    progress: Progress<'_>,
) -> Result<Packed, ProjectError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_name().map(|n| n.to_string_lossy().into_owned());
    // A number of its own as well as the process's, so two saves started
    // close together never write through the same file.
    static SAVES: AtomicU32 = AtomicU32::new(0);
    let tmp = dir.join(format!(
        ".{}.tmp-{}-{}",
        stem.unwrap_or_else(|| "project.lum".into()),
        std::process::id(),
        SAVES.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| -> Result<Packed, ProjectError> {
        let previous: Vec<PathBuf> = previous.map(Path::to_path_buf).into_iter().collect();
        let (mut zips, held) = open_archives(&previous);
        let offered: HashMap<Uuid, &PackSource> = sources
            .iter()
            .filter(|s| !s.files.is_empty())
            .map(|s| (s.item, s))
            .collect();
        let live = |id: Uuid| doc.packed_path(id).is_some();

        let mut plan: Vec<(String, Vec<HeldFile>)> = Vec::new();
        let mut packed: BTreeMap<Uuid, PackedMedia> = BTreeMap::new();
        let mut claimed: HashSet<String> = HashSet::new();
        // Several items can read one file (the layers of a PSD), and it is
        // packed once.
        let mut by_path: HashMap<PathBuf, PackedMedia> = HashMap::new();

        for (id, was) in &doc.packed {
            if !live(*id) {
                continue;
            }
            let Some((folder, name)) = split_entry(&was.entry) else {
                continue;
            };
            let source = offered.get(id);
            let carried = held
                .get(folder)
                .filter(|files| files.iter().any(|f| f.name == name));
            let main = source.and_then(|s| s.files.first());
            let Some(files) = carried else {
                continue;
            };
            let unchanged = match (source, main) {
                (Some(source), Some(main)) => {
                    fingerprint_path(main).is_ok_and(|fp| fp.likely_same_content(&was.fingerprint))
                        && same_files(&source.files, files)
                }
                _ => true,
            };
            if !unchanged {
                continue;
            }
            if claimed.insert(folder.to_owned()) {
                plan.push((folder.to_owned(), files.clone()));
            }
            packed.insert(*id, was.clone());
            if let Some(main) = main {
                by_path.insert(main.clone(), was.clone());
            }
        }

        for source in sources {
            if packed.contains_key(&source.item) || !live(source.item) {
                continue;
            }
            let Some(main) = source.files.first() else {
                continue;
            };
            if let Some(shared) = by_path.get(main) {
                packed.insert(source.item, shared.clone());
                continue;
            }
            let Ok(fingerprint) = fingerprint_path(main) else {
                continue; // gone or unreadable: it stays unpacked
            };
            let mut files = Vec::with_capacity(source.files.len());
            let mut names = HashSet::new();
            for file in &source.files {
                let name = main.parent().and_then(|dir| name_under(dir, file));
                let size = fs::metadata(file).map(|m| m.len()).ok();
                let (Some(name), Some(size)) = (name, size) else {
                    continue;
                };
                if plain_path(&name) && names.insert(name.clone()) {
                    files.push(HeldFile {
                        name,
                        size,
                        from: Held::Disk(file.clone()),
                    });
                }
            }
            // The file the item names has to be the one that leads the
            // folder, or the entry would name a different frame.
            let Some(name) = files
                .first()
                .filter(|f| matches!(&f.from, Held::Disk(path) if path == main))
                .map(|f| f.name.clone())
            else {
                continue;
            };
            let folder = free_folder(source.item, &mut claimed);
            let media = PackedMedia {
                entry: format!("{MEDIA_PREFIX}{folder}/{name}"),
                fingerprint,
                // Packed again from a copy, it is still the same file's.
                original: doc
                    .packed
                    .get(&source.item)
                    .and_then(|was| was.original.clone()),
                extra: serde_json::Map::new(),
            };
            plan.push((folder, files));
            packed.insert(source.item, media.clone());
            by_path.insert(main.clone(), media);
        }

        let total: u64 = plan
            .iter()
            .flat_map(|(_, files)| files.iter().map(|f| f.size))
            .sum();
        let mut done = 0u64;
        let mut count = 0u64;

        let mut zip = ZipWriter::new(File::create(&tmp)?);
        let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        // Manifest MUST be the first entry.
        zip.start_file("manifest.json", text)?;
        zip.write_all(serde_json::to_string_pretty(manifest)?.as_bytes())?;

        for (folder, files) in &plan {
            let mut failed = false;
            for file in files {
                if failed {
                    break;
                }
                let stored = SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Stored)
                    // Past four gigabytes an entry needs the wide size fields,
                    // and the writer has to be told before the first byte.
                    .large_file(file.size >= u64::from(u32::MAX) - 0xFFFF);
                zip.start_file(format!("{MEDIA_PREFIX}{folder}/{}", file.name), stored)?;
                match &file.from {
                    // The project's own file failing to read back is the save
                    // failing: this may be the only copy, so the file it is
                    // in is left alone.
                    Held::Archive { archive, index } => {
                        let Some(source) = zips.get_mut(*archive).and_then(Option::as_mut) else {
                            return Err(ProjectError::NotALumitProject);
                        };
                        let mut entry = source.by_index(*index)?;
                        copy_chunks(&mut entry, &mut zip, file.size, &mut done, total, progress)?;
                    }
                    // A file on disk that will not read is still on disk. It
                    // is left out, and the items reading it stay unpacked.
                    Held::Disk(path) => {
                        let before = done;
                        let copied = match File::open(path) {
                            Ok(mut source) => copy_chunks(
                                &mut source,
                                &mut zip,
                                file.size,
                                &mut done,
                                total,
                                progress,
                            ),
                            Err(e) => Err(CopyError::Read(e)),
                        };
                        match copied {
                            Ok(()) => {}
                            Err(CopyError::Read(_)) => {
                                zip.abort_file()?;
                                done = before;
                                failed = true;
                                continue;
                            }
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
                count += 1;
            }
            if failed {
                let prefix = format!("{MEDIA_PREFIX}{folder}/");
                packed.retain(|_, media| !media.entry.starts_with(&prefix));
            }
        }

        more(&mut zip, &packed)?;
        // Last, because only now is it known what the archive really holds.
        let mut written = doc.clone();
        written.packed.clone_from(&packed);
        zip.start_file("project.json", text)?;
        zip.write_all(serde_json::to_string_pretty(&written)?.as_bytes())?;
        let mut file = zip.finish()?;
        // An entry given up part-way leaves its bytes past the end of the
        // archive when what followed was shorter. They are cut off here.
        let end = file.stream_position()?;
        file.set_len(end)?;
        file.sync_all()?;
        // The old file may be the one being replaced, and Windows will not
        // rename over a file somebody has open.
        drop(zips);
        fs::rename(&tmp, path)?;
        Ok(Packed {
            packed,
            files: count,
        })
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_out(
    zips: &mut [Option<ZipArchive<File>>],
    files: &[HeldFile],
    dest_dir: &Path,
    main: (&str, &Fingerprint),
    done: &mut u64,
    total: u64,
    progress: Progress<'_>,
) -> Result<u64, ProjectError> {
    let mut written = 0;
    // The file the item names says whose folder this is. When it is not the
    // one that was packed, the folder is an older pack's and nothing in it is
    // kept: a frame the same size as the one wanted is still another frame.
    let ours = files
        .iter()
        .any(|file| file.name == main.0 && already_there(&dest_dir.join(&file.name), file, main));
    for file in files {
        let out = dest_dir.join(&file.name);
        if ours && already_there(&out, file, main) {
            *done += file.size;
            continue;
        }
        // Written under another name and renamed, so a copy cut short is
        // never taken for the file.
        let leaf = out.file_name().unwrap_or_default().to_string_lossy();
        let part = out.with_file_name(format!(".{leaf}.part"));
        let copied = (|| -> Result<(), ProjectError> {
            fs::create_dir_all(out.parent().unwrap_or(dest_dir))?;
            let mut to = File::create(&part)?;
            match &file.from {
                Held::Archive { archive, index } => {
                    let Some(zip) = zips.get_mut(*archive).and_then(Option::as_mut) else {
                        return Err(ProjectError::NotALumitProject);
                    };
                    let mut entry = zip.by_index(*index)?;
                    copy_chunks(&mut entry, &mut to, file.size, done, total, progress)?;
                }
                Held::Disk(path) => {
                    let mut from = File::open(path)?;
                    copy_chunks(&mut from, &mut to, file.size, done, total, progress)?;
                }
            }
            to.sync_all()?;
            Ok(())
        })();
        if let Err(e) = copied {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
        fs::rename(&part, &out)?;
        written += 1;
    }
    Ok(written)
}

/// Whether `out` is already the file `file` would be written as. Size answers
/// for most of them. The file the item names is also fingerprinted, because
/// that one decides whether the folder is this pack's or an older one's.
fn already_there(out: &Path, file: &HeldFile, main: (&str, &Fingerprint)) -> bool {
    if fs::metadata(out).map(|m| m.len()).ok() != Some(file.size) {
        return false;
    }
    file.name != main.0 || fingerprint_path(out).is_ok_and(|fp| fp.likely_same_content(main.1))
}

/// Point every packed item of a just-opened `doc` at a file to read: the one
/// on disk when it is still the one that was packed, otherwise a copy read
/// out of the archive into `dest_root`. Returns how many items read a copy,
/// and the items still on the packed list that it could not place.
///
/// `archives` are tried in order. The project's own file comes first, and an
/// autosave is followed by the project it was written beside
/// ([`autosave_owner`]), because an autosave carries the document and not the
/// footage.
///
/// Call it before [`crate::resolve_all_media`], which leaves an item this has
/// placed alone. An item it cannot place is left as it was for the resolver
/// and the relink slate, and comes off the packed list when no archive holds
/// it.
pub fn restore_packed(
    doc: &mut Document,
    archives: &[PathBuf],
    project_dir: &Path,
    dest_root: &Path,
    progress: Progress<'_>,
) -> Result<(usize, Vec<Uuid>), ProjectError> {
    let mut wanted: Vec<(Uuid, PackedMedia)> = Vec::new();
    for (id, media) in doc.packed.clone() {
        // A file an effect reads has the one path and nothing relative.
        let relative = doc
            .packed_ref(id)
            .map(|r| project_dir.join(&r.relative_path));
        let Some(stored) = doc.packed_path_mut(id) else {
            continue;
        };
        let in_place = [Some(PathBuf::from(&*stored)), relative]
            .into_iter()
            .flatten()
            .find(|path| {
                !path.as_os_str().is_empty()
                    && path.is_file()
                    && fingerprint_path(path)
                        .is_ok_and(|fp| fp.likely_same_content(&media.fingerprint))
            });
        match in_place {
            Some(path) => *stored = path.to_string_lossy().into_owned(),
            None => wanted.push((id, media)),
        }
    }
    if wanted.is_empty() {
        return Ok((0, Vec::new()));
    }

    let (mut zips, held) = open_archives(archives);
    let mut unplaced = Vec::new();
    let folders: HashSet<&str> = wanted
        .iter()
        .filter_map(|(_, media)| split_entry(&media.entry).map(|(folder, _)| folder))
        .collect();
    let total = folders
        .iter()
        .filter_map(|folder| held.get(*folder))
        .flat_map(|files| files.iter().map(|f| f.size))
        .fold(0u64, u64::saturating_add);
    let mut done = 0u64;
    let mut read: HashSet<&str> = HashSet::new();
    let mut restored = 0;
    for (id, media) in &wanted {
        let Some((folder, name)) = split_entry(&media.entry) else {
            doc.packed.remove(id);
            continue;
        };
        let dest_dir = dest_root.join(folder);
        if read.insert(folder) {
            if let Some(files) = held.get(folder) {
                match write_out(
                    &mut zips,
                    files,
                    &dest_dir,
                    (name, &media.fingerprint),
                    &mut done,
                    total,
                    progress,
                ) {
                    Ok(_) => {}
                    Err(ProjectError::Cancelled) => return Err(ProjectError::Cancelled),
                    // A full disk or a damaged entry: this item is missing,
                    // and the rest of the project still opens.
                    Err(_) => {
                        unplaced.push(*id);
                        continue;
                    }
                }
            }
        }
        // Whether it was read just now or on an earlier open, it has to be
        // the file that was packed.
        let main = dest_dir.join(name);
        if fingerprint_path(&main).is_ok_and(|fp| fp.likely_same_content(&media.fingerprint)) {
            // An effect's file has only the one path, so the one it held is
            // kept on the entry for the next save to write back.
            let file = doc.packed_ref(*id).is_none();
            if let Some(stored) = doc.packed_path_mut(*id) {
                let was = std::mem::replace(stored, main.to_string_lossy().into_owned());
                restored += 1;
                if file && !Path::new(&was).starts_with(dest_root) {
                    if let Some(entry) = doc.packed.get_mut(id) {
                        entry.original = Some(was);
                    }
                }
            }
        } else if !held.contains_key(folder) {
            // The list is the file's own word. An item no archive holds was
            // never packed, and left on the list the next save would pack
            // whatever file its path names on this machine. One an archive
            // holds stays on it even when the copy would not read out, or
            // that save would leave its bytes behind.
            doc.packed.remove(id);
        } else {
            unplaced.push(*id);
        }
    }
    Ok((restored, unplaced))
}

/// Put back the path each effect's file had before an open pointed its
/// parameter at a copy under `copies`. Every save does this to the document it
/// writes, so a file never keeps a path inside one machine's cache.
pub(crate) fn keep_original_paths(doc: &mut Document, copies: &Path) {
    // By the copy's path and not by whose file it is: a layer split or
    // duplicated since the open has effects with ids of their own, reading
    // the same copy.
    let originals: HashMap<String, String> = doc
        .packed
        .iter()
        .filter_map(|(id, entry)| {
            let copy = doc
                .packed_path(*id)
                .filter(|path| Path::new(path).starts_with(copies))?;
            Some((copy.to_owned(), entry.original.clone()?))
        })
        .collect();
    if originals.is_empty() {
        return;
    }
    for path in doc.effect_file_paths_mut() {
        if let Some(original) = originals.get(path.as_str()) {
            path.clone_from(original);
        }
    }
}

/// Write the packed footage of `doc` back out as files beside the project, in
/// `<project_dir>/media/`.
///
/// An item whose file on disk is still there is left reading it and nothing
/// is written for it. Anything else is copied out of the archive, or out of
/// `dest_root` where an open already read it. A single file lands in `media/`
/// under its own name, and an image sequence in a folder of its own there.
///
/// Changes nothing in `doc` or in the archive. The answer says which
/// references to repoint and which items could not be written, and the caller
/// records that and saves, which is what takes the bytes out of the `.lum`.
pub fn unpack(
    doc: &Document,
    archives: &[PathBuf],
    project_dir: &Path,
    dest_root: &Path,
    progress: Progress<'_>,
) -> Result<Unpacked, ProjectError> {
    let (mut zips, mut held) = open_archives(archives);
    let mut out = Unpacked::default();

    let mut folders: Vec<(String, String, Fingerprint, Vec<Uuid>)> = Vec::new();
    for (id, media) in &doc.packed {
        let Some(now) = doc.packed_path(*id).map(Path::new) else {
            continue;
        };
        if now.is_file() && !now.starts_with(dest_root) {
            continue;
        }
        let Some((folder, name)) = split_entry(&media.entry) else {
            out.kept.insert(*id, media.clone());
            continue;
        };
        match folders.iter_mut().find(|(known, ..)| known == folder) {
            Some((.., items)) => items.push(*id),
            None => folders.push((
                folder.to_owned(),
                name.to_owned(),
                media.fingerprint.clone(),
                vec![*id],
            )),
        }
    }

    for (folder, ..) in &folders {
        if held.contains_key(folder) {
            continue;
        }
        let mut files = Vec::new();
        read_out_files(&dest_root.join(folder), "", &mut files);
        if !files.is_empty() {
            held.insert(folder.clone(), files);
        }
    }

    let total: u64 = folders
        .iter()
        .filter_map(|(folder, ..)| held.get(folder))
        .flat_map(|files| files.iter().map(|f| f.size))
        .sum();
    let mut done = 0u64;
    let media_dir = project_dir.join("media");
    for (folder, name, fingerprint, items) in &folders {
        let keep = |out: &mut Unpacked| {
            for id in items {
                if let Some(media) = doc.packed.get(id) {
                    out.kept.insert(*id, media.clone());
                }
            }
        };
        let Some(files) = held
            .get(folder)
            .filter(|f| f.iter().any(|f| &f.name == name))
        else {
            keep(&mut out);
            continue;
        };
        let (dest_dir, as_name) = unpack_place(&media_dir, name, fingerprint, files);
        // A lone file may have had to take another name to sit beside one
        // already there.
        let renamed: Vec<HeldFile>;
        let files = if as_name == *name {
            files
        } else {
            renamed = files
                .iter()
                .cloned()
                .map(|mut f| {
                    f.name.clone_from(&as_name);
                    f
                })
                .collect();
            &renamed
        };
        match write_out(
            &mut zips,
            files,
            &dest_dir,
            (&as_name, fingerprint),
            &mut done,
            total,
            progress,
        ) {
            Ok(written) => out.written += written,
            Err(ProjectError::Cancelled) => return Err(ProjectError::Cancelled),
            Err(_) => {
                keep(&mut out);
                continue;
            }
        }
        let main = dest_dir.join(&as_name);
        for id in items {
            // A file an effect reads has no reference to keep anything of.
            let mut media = doc.packed_ref(*id).cloned().unwrap_or(MediaRef {
                relative_path: String::new(),
                absolute_path: String::new(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            });
            media.relative_path = relative_between(project_dir, &main)
                .unwrap_or_else(|| main.to_string_lossy().into_owned());
            media.absolute_path = main.to_string_lossy().into_owned();
            media.fingerprint = Some(fingerprint.clone());
            out.moved.push((*id, media));
        }
    }
    Ok(out)
}

fn read_out_files(dir: &Path, under: &str, into: &mut Vec<HeldFile>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let leaf = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if leaf.starts_with('.') {
            continue;
        }
        let name = if under.is_empty() {
            leaf
        } else {
            format!("{under}/{leaf}")
        };
        if meta.is_dir() {
            read_out_files(&entry.path(), &name, into);
        } else if meta.is_file() {
            into.push(HeldFile {
                name,
                size: meta.len(),
                from: Held::Disk(entry.path()),
            });
        }
    }
}

/// Where under `media_dir` a packed folder is written, and what its main file
/// is called there.
///
/// A place that already holds this very file is used again, so unpacking
/// twice writes once. A place holding something else is stepped past with
/// `-1`, `-2` and so on.
fn unpack_place(
    media_dir: &Path,
    name: &str,
    fingerprint: &Fingerprint,
    files: &[HeldFile],
) -> (PathBuf, String) {
    let same =
        |path: &Path| fingerprint_path(path).is_ok_and(|fp| fp.likely_same_content(fingerprint));
    let named = Path::new(name);
    let stem = named
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_owned());
    let ext = named.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 0u32.. {
        let numbered = if n == 0 {
            stem.clone()
        } else {
            format!("{stem}-{n}")
        };
        if files.len() > 1 {
            // A run keeps its file names and gets a folder to itself.
            let dir = media_dir.join(&numbered);
            if !dir.exists() || same(&dir.join(name)) {
                return (dir, name.to_owned());
            }
        } else {
            let candidate = match &ext {
                Some(ext) => format!("{numbered}.{ext}"),
                None => numbered,
            };
            let path = media_dir.join(&candidate);
            if !path.exists() || same(&path) {
                return (media_dir.to_path_buf(), candidate);
            }
        }
    }
    (media_dir.to_path_buf(), name.to_owned())
}

/// The file a read-out folder holds, locked, for as long as a project is
/// reading from it.
const IN_USE: &str = ".in-use";

/// What a folder is renamed to start with when it is being cleared away, so
/// it stops being a document's at once and is emptied afterwards.
const GONE: &str = ".gone-";

/// How long a folder has to sit unopened before it counts as unused. Sooner,
/// and someone working on two packed projects would read each one out of its
/// file again on every open.
const IDLE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Take the folder `doc_id`'s packed files are read out to under `root`, and
/// remove the other documents' folders there that nobody has used for a week.
///
/// A folder is taken by a lock on a marker file in it, which lasts until the
/// answer is dropped or the process ends, however it ends. Taking it also
/// stamps the marker with the time. A folder goes only when nobody holds its
/// marker and the stamp is a week old. A folder with no marker was written by
/// a Lumit from before there were any, and goes once nothing in it has
/// changed for a week.
///
/// One Lumit at a time is in here, by a lock on a file of `root`'s own, so no
/// folder is taken while another Lumit is deciding it is free.
///
/// `wanted` makes the folder if it is not there. `None` when there is no
/// folder to take or the lock was refused. Nothing outside `root` is removed,
/// and no link is followed out of it.
pub fn hold_read_out(root: &Path, doc_id: Uuid, wanted: bool) -> Option<File> {
    if wanted {
        fs::create_dir_all(root).ok()?;
    }
    if !fs::symlink_metadata(root).is_ok_and(|meta| meta.is_dir()) {
        return None;
    }
    let gate = lock_file(&root.join(".lock"))?;
    gate.lock().ok()?;

    let mine = doc_id.to_string();
    if wanted {
        let _ = fs::create_dir_all(root.join(&mine));
    }
    let held = lock_file(&root.join(&mine).join(IN_USE)).filter(|m| m.lock_shared().is_ok());
    if let Some(marker) = &held {
        let _ = marker.set_modified(SystemTime::now());
    }

    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        // Only the folders this module makes, each named for a document. A
        // link to a folder is not one.
        let name = entry.file_name();
        // One on its way out that a Lumit did not live to finish with.
        if name.to_str().is_some_and(|name| name.starts_with(GONE))
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            let gone = entry.path();
            std::thread::spawn(move || fs::remove_dir_all(gone));
            continue;
        }
        let theirs = name
            .to_str()
            .is_some_and(|name| name != mine && Uuid::parse_str(name).is_ok());
        if !theirs || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let dir = entry.path();
        let marker = dir.join(IN_USE);
        let unused = match fs::symlink_metadata(&marker) {
            Ok(meta) if meta.is_file() => {
                idle(&marker) && lock_file(&marker).is_some_and(|m| m.try_lock().is_ok())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                idle(&dir)
                    && fs::read_dir(&dir)
                        .is_ok_and(|mut inside| inside.all(|e| e.is_ok_and(|e| idle(&e.path()))))
            }
            _ => false,
        };
        if unused {
            // Out of the way under a name no document has, which is quick,
            // and then gone on a thread of its own: taking out a few thousand
            // frames can take seconds, and an open is waiting on this.
            let gone = root.join(format!("{GONE}{}", name.to_string_lossy()));
            if fs::rename(&dir, &gone).is_ok() {
                std::thread::spawn(move || fs::remove_dir_all(gone));
            }
        }
    }
    held
}

/// Open the file at `path` to lock it, making it if it is not there. A link
/// is refused, because it could point anywhere.
fn lock_file(path: &Path) -> Option<File> {
    if fs::symlink_metadata(path).is_ok_and(|meta| !meta.is_file()) {
        return None;
    }
    let mut open = OpenOptions::new();
    open.read(true).write(true).create(true).truncate(false);
    open.open(path).ok()
}

/// Whether `path` was last changed more than a week ago. Anything that will
/// not say counts as fresh.
fn idle(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .and_then(|meta| meta.modified())
        .is_ok_and(|at| at.elapsed().is_ok_and(|age| age >= IDLE))
}

/// The project an autosave was written beside:
/// `<dir>/autosaves/<stem>.autosave-<n>.lum` answers `<dir>/<stem>.lum`.
/// `None` for a path that is not an autosave's.
#[must_use]
pub fn autosave_owner(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    if dir.file_name()? != "autosaves" {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let (project, slot) = stem.rsplit_once(".autosave-")?;
    if slot.is_empty() || !slot.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(dir.parent()?.join(format!("{project}.lum")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
