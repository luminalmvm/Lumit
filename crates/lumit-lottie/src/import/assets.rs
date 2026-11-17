//! A `.lottie`'s zip, and a Lottie's pictures as footage.
//!
//! # In plain terms
//!
//! A `.lottie` is a zip holding a manifest, one or more animations and their
//! pictures: version 1 keeps them in `animations/` and `images/`, version 2 in
//! `a/` and `i/`. The animation the manifest names first (or the first one
//! there is) is the one imported.
//!
//! A picture may be carried inside the file (a `data:` address, or an entry
//! of the zip) or sit beside it. One carried inside is written out, into a
//! folder named after the file, so the footage item has a file to point at;
//! one beside it is pointed at where it is.
//!
//! Runs on the import's thread. Nothing is written here: the files to write
//! are handed back with the items.

use std::collections::BTreeMap;
use std::io::Read as _;

use base64::Engine as _;
use lumit_core::model::{FootageItem, MediaRef, ProjectItem};
use lumit_import::report::{ItemPath, Outcome, Reason};
use lumit_ingress::{Budget, IngressError};
use serde_json::Value;
use uuid::Uuid;

use super::{read, Cx};
use crate::LottieError;

/// The most entries a `.lottie` may hold.
const MAX_ENTRIES: usize = 10_000;

/// A `.lottie` opened: the animation's JSON, and each picture by its path in
/// the zip, in lower case.
pub(super) type Unzipped = (Vec<u8>, BTreeMap<String, Vec<u8>>);

/// The animation and the pictures of a `.lottie`.
pub(super) fn unzip(bytes: &[u8], budget: &mut Budget) -> Result<Unzipped, LottieError> {
    let zip_err = |e: zip::result::ZipError| LottieError::Zip(e.to_string());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(zip_err)?;
    if zip.len() > MAX_ENTRIES {
        return Err(IngressError::Items {
            needed: zip.len() as u64,
            limit: MAX_ENTRIES as u64,
        }
        .into());
    }
    budget.take_items(zip.len() as u64)?;
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for n in 0..zip.len() {
        let mut entry = zip.by_index(n).map_err(zip_err)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .name()
            .map_err(zip_err)?
            .trim_start_matches('/')
            .to_owned();
        let wanted = name == "manifest.json"
            || ["a/", "animations/", "i/", "images/"]
                .iter()
                .any(|folder| name.starts_with(folder));
        if !wanted {
            continue;
        }
        let size = entry.size();
        budget.take_bytes(size)?;
        let mut out =
            Vec::with_capacity(usize::try_from(size).map_err(|_| IngressError::Overflow)?);
        // One byte past what the entry declares, so one that lies is caught.
        (&mut entry)
            .take(size.saturating_add(1))
            .read_to_end(&mut out)
            .map_err(|e| LottieError::Zip(e.to_string()))?;
        if out.len() as u64 > size {
            return Err(LottieError::Zip(String::from(
                "an entry is longer than it says",
            )));
        }
        entries.insert(name, out);
    }

    let manifest: Value = entries
        .get("manifest.json")
        .and_then(|m| serde_json::from_slice(m).ok())
        .unwrap_or(Value::Null);
    let named = manifest
        .get("initial")
        .and_then(|i| read::text(i, "animation"))
        .or_else(|| read::text(&manifest, "activeAnimationId"))
        .or_else(|| {
            read::list(&manifest, "animations")
                .first()
                .and_then(|a| read::text(a, "id"))
        })
        .map(str::to_owned);
    let is_animation = |name: &str| {
        (name.starts_with("a/") || name.starts_with("animations/")) && name.ends_with(".json")
    };
    let chosen = named
        .iter()
        .flat_map(|id| [format!("a/{id}.json"), format!("animations/{id}.json")])
        .find(|name| entries.contains_key(name))
        .or_else(|| entries.keys().find(|name| is_animation(name)).cloned())
        .ok_or(LottieError::NotLottie)?;
    let json = entries.remove(&chosen).ok_or(LottieError::NotLottie)?;
    let pictures = entries
        .into_iter()
        .filter(|(name, _)| name.starts_with("i/") || name.starts_with("images/"))
        .map(|(name, bytes)| (name.to_lowercase(), bytes))
        .collect();
    Ok((json, pictures))
}

/// The footage item for picture asset `id`, made the first time it is asked
/// for. `None` when the file has no such asset.
pub(super) fn image(
    cx: &mut Cx<'_>,
    place: &ItemPath,
    id: &str,
) -> Result<Option<Uuid>, LottieError> {
    if let Some(made) = cx.images.get(id) {
        return Ok(Some(*made));
    }
    let Some(asset) = cx.assets.get(id).copied() else {
        return Ok(None);
    };
    let u = read::text(asset, "u").unwrap_or_default();
    let p = read::text(asset, "p").unwrap_or_default();
    let name = read::text(asset, "nm")
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(if p.starts_with("data:") { id } else { p })
        .to_owned();

    // The zip's entry for this picture, by its whole path or its name alone.
    let entry = |cx: &Cx| {
        let key = format!("{}{}", u.trim_start_matches('/'), p).to_lowercase();
        if cx.zipped.contains_key(&key) {
            return Some(key);
        }
        let file = p.rsplit('/').next().unwrap_or(p).to_lowercase();
        cx.zipped
            .keys()
            .find(|entry| entry.rsplit('/').next() == Some(file.as_str()))
            .cloned()
    };
    let path = if let Some(data) = p.strip_prefix("data:") {
        match decoded(cx, place, id, data)? {
            Some((file, bytes)) => {
                let path = unique(cx, &file);
                cx.files.push((path.clone(), bytes));
                path
            }
            // Unreadable: the footage points where it would have been
            // written, offline.
            None => cx.folder.join(safe_name(id, "png")),
        }
    } else if let Some(entry) = entry(cx) {
        // A zip entry is written once, however many assets name it.
        match cx.written.get(&entry) {
            Some(path) => path.clone(),
            None => {
                let bytes = cx.zipped.get_mut(&entry).map(std::mem::take);
                let path = unique(cx, &safe_name(p, "png"));
                cx.files.push((path.clone(), bytes.unwrap_or_default()));
                cx.written.insert(entry, path.clone());
                path
            }
        }
    } else {
        // Only a path inside the file's own folder is followed: no root, no
        // drive or share, no `..`.
        let linked = std::path::Path::new(u.trim_start_matches('/')).join(p);
        let inside = linked.components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        });
        let path = if inside {
            cx.beside.join(linked)
        } else {
            cx.beside.join(safe_name(p, "png"))
        };
        if !inside || !lumit_ingress::files::is_file(&path) {
            cx.row(place.clone(), Outcome::Adjusted, Reason::MediaNotFound);
        }
        path
    };
    let path = path.to_string_lossy().into_owned();
    let item = Uuid::now_v7();
    cx.made.push(ProjectItem::Footage(FootageItem {
        id: item,
        name,
        media: MediaRef {
            relative_path: path.clone(),
            absolute_path: path,
            fingerprint: None,
            extra: serde_json::Map::new(),
        },
        colour_space: None,
        sequence: None,
        source_layer: None,
        interpretation: lumit_core::interpretation::Interpretation::default(),
        extra: serde_json::Map::new(),
    }));
    cx.images.insert(id.to_owned(), item);
    Ok(Some(item))
}

/// A `data:` address's bytes and the file name to write them under, or `None`
/// (with a row) when it can't be read.
fn decoded(
    cx: &mut Cx<'_>,
    place: &ItemPath,
    id: &str,
    data: &str,
) -> Result<Option<(String, Vec<u8>)>, LottieError> {
    let (head, body) = data.split_once(',').unwrap_or(("", data));
    let ext = match head.split(';').next().unwrap_or_default() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        _ => "png",
    };
    let text: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    cx.budget.take_bytes(text.len() as u64 / 4 * 3 + 3)?;
    let bytes = if head.ends_with(";base64") {
        base64::engine::general_purpose::STANDARD
            .decode(text.as_bytes())
            .ok()
    } else {
        Some(text.into_bytes())
    };
    let Some(bytes) = bytes else {
        cx.row(
            place.clone(),
            Outcome::Adjusted,
            Reason::MediaMissing {
                path: id.to_owned(),
            },
        );
        return Ok(None);
    };
    Ok(Some((safe_name(id, ext), bytes)))
}

/// A file name made safe to write: its last part only, letters, digits, dots,
/// dashes and underscores, with an extension.
fn safe_name(name: &str, ext: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut safe: String = last
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    safe = safe.trim_start_matches('.').to_owned();
    if safe.is_empty() {
        safe = String::from("image");
    }
    if !safe.contains('.') {
        safe = format!("{safe}.{ext}");
    }
    safe
}

/// A path in the pictures folder that no other picture of this import takes.
fn unique(cx: &Cx<'_>, file: &str) -> std::path::PathBuf {
    let (stem, ext) = file.rsplit_once('.').unwrap_or((file, ""));
    let mut n = 1;
    loop {
        let name = if n == 1 {
            file.to_owned()
        } else {
            format!("{stem}_{n}.{ext}")
        };
        let path = cx.folder.join(name);
        if !cx.files.iter().any(|(taken, _)| *taken == path) {
            return path;
        }
        n += 1;
    }
}
