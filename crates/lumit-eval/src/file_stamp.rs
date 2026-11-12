//! How a file a row reads is folded into a frame key: by its size and time,
//! and by where it was found when that isn't where the document says.

use crate::SourceStamper;

/// The file a row names, as this machine reads it. A file found elsewhere,
/// such as a LUT kept in the user's library, is named and stamped as found,
/// so editing that copy renames the frames drawn from it.
pub(crate) fn feed_row_file(h: &mut blake3::Hasher, stamper: &dyn SourceStamper, path: &str) {
    match stamper.file_on_this_machine(path) {
        Some(found) => {
            h.update(b"found/");
            h.update(&(found.len() as u64).to_le_bytes());
            h.update(found.as_bytes());
            feed_file_stamp(h, &found);
        }
        None => feed_file_stamp(h, path),
    }
}

/// A file's size and last-modified time, folded into a key so that editing
/// the file on disk renames the frames that read it.
///
/// A path a frame key mentions is a path some loader is about to read, so the
/// stat is warm and costs microseconds. A file the engine cannot stat — gone,
/// on a disconnected share, permission-denied — feeds a distinct marker and
/// nothing else: the loader will fail the same way, and a name that says
/// "this file was unreadable" is as true as any other.
fn feed_file_stamp(h: &mut blake3::Hasher, path: &str) {
    let stamp = std::fs::metadata(path).ok().map(|m| {
        let modified = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        (m.len(), modified)
    });
    match stamp {
        Some((len, modified)) => {
            h.update(&[1]);
            h.update(&len.to_le_bytes());
            h.update(&modified.to_le_bytes());
        }
        None => {
            h.update(&[0]);
        }
    }
}
