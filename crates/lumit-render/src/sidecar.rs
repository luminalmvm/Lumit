//! The framing every cache sidecar in this crate shares.
//!
//! In plain terms: an analysis that took seconds is written to a small file so
//! the next session reads it instead of doing the work again. Each such file —
//! the tracker's, roto's — starts with the same nine bytes: a seven-byte magic
//! saying "this is one of ours", then a two-byte format version. A reader
//! checks both before it hands a single byte to a deserialiser, so a file that
//! is not ours, or one a newer Lumit wrote in a shape this build does not know,
//! is refused rather than misread (the refuse-newer rule `manifest.json`
//! follows, docs/10 §1). The version sits **outside** the body deliberately: a
//! reader has to be able to say "this was written by a newer Lumit" without
//! first parsing a shape it does not know.
//!
//! Each caller keeps its own magic, its own version constant and its own record
//! type; only the nine bytes around them live here.

use std::path::Path;

/// The length of the header [`frame`] writes and [`unframe`] strips.
const HEAD: usize = 9;

/// Magic, version, then the body.
pub(crate) fn frame(magic: &[u8; 7], version: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + HEAD);
    out.extend_from_slice(magic);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// The inverse: the body, or `None` for a file too short to hold a header, one
/// whose magic is not ours, or one written by a newer build.
pub(crate) fn unframe<'a>(bytes: &'a [u8], magic: &[u8; 7], max_version: u16) -> Option<&'a [u8]> {
    let (head, body) = bytes.split_at_checked(HEAD)?;
    if head.get(..7)? != magic {
        return None;
    }
    let version = u16::from_le_bytes([*head.get(7)?, *head.get(8)?]);
    (version <= max_version).then_some(body)
}

/// Write a sidecar, best-effort. A cache that cannot be written costs the next
/// session a re-analysis; it is never worth failing an answer already in hand.
pub(crate) fn write(dir: &Path, name: &str, bytes: &[u8]) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join(name), bytes);
}
