//! Taking named files out of a `.tar.gz` download.
//!
//! Some model packs are only published upstream as a gzipped tar, so the
//! installer reads one the way it reads a zip: the download's size is checked,
//! then exactly the entries the manifest names come out under the plain names
//! it gives them, and nothing else is written. A tar is a run of 512-byte
//! headers each followed by its file, so the reader is a short loop rather
//! than a crate.
//!
//! # Thread role and contract
//!
//! Disk work on the installing thread, inside [`crate::store::install`]'s slot.
//! The archive is streamed: nothing is held in memory but one header, however
//! large the files inside are. No file it writes is larger than
//! [`store::MOST_UNPACKED`], and the archive is read to its end, so the gzip
//! checksum is checked and nothing may follow it.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufReader, Read, Write},
    path::Path,
};

use flate2::bufread::GzDecoder;

use crate::{error::MlError, manifest, store};

/// A tar header, and the unit every file inside is padded to.
const BLOCK: u64 = 512;

/// Take exactly the named entries out of a `.tar.gz` and no others. An entry
/// the archive does not have, one cut short or one too large, and an archive
/// whose checksum fails or that has anything after its end, are refusals,
/// not a smaller install. Only regular files are ever taken; links, folders
/// and the extended headers some tools write are stepped over.
pub(crate) fn take(
    from: &Path,
    size: u64,
    entries: &BTreeMap<String, String>,
    into: &Path,
) -> Result<(), MlError> {
    // The manifest parser has already held every name to this, and the check
    // is repeated because this is the code that writes the files.
    if let Some(dest) = entries.values().find(|dest| !manifest::is_plain_name(dest)) {
        return Err(MlError::Invalid(format!(
            "\"{dest}\" is not a file name inside the addon"
        )));
    }
    let file = fs::File::open(from).map_err(invalid)?;
    let held = file.metadata().map_err(invalid)?.len();
    if held != size {
        return Err(MlError::Invalid(format!(
            "{} is {held} bytes and the manifest says {size}",
            from.display()
        )));
    }

    // The decoder that reads from a buffer of ours rather than its own, so
    // whatever it has read past the gzip trailer is still there to be found.
    let mut tar = GzDecoder::new(BufReader::new(file));
    let mut taken = Vec::with_capacity(entries.len());
    let mut header = [0u8; BLOCK as usize];
    loop {
        tar.read_exact(&mut header).map_err(|_| cut_short())?;
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let name = path_of(&header);
        let length = size_of(&header).ok_or_else(cut_short)?;
        let regular = matches!(header[156], b'0' | 0);
        let padded = length
            .div_ceil(BLOCK)
            .checked_mul(BLOCK)
            .ok_or_else(cut_short)?;
        match entries.get(&name).filter(|_| regular) {
            Some(dest) => {
                // The header is the only word on the length, and the copy
                // below takes no more than it, so this bounds what is written.
                if length > store::MOST_UNPACKED {
                    return Err(store::too_large(&name));
                }
                let mut out = fs::File::create(into.join(dest)).map_err(invalid)?;
                let copied = io::copy(&mut (&mut tar).take(length), &mut out).map_err(invalid)?;
                out.flush().map_err(invalid)?;
                if copied != length {
                    return Err(cut_short());
                }
                skip(&mut tar, padded - length)?;
                taken.push(name);
            }
            None => skip(&mut tar, padded)?,
        }
    }
    to_the_end(tar)?;
    match entries.keys().find(|inside| !taken.contains(inside)) {
        Some(inside) => Err(MlError::Invalid(format!(
            "the download has no \"{inside}\" in it"
        ))),
        None => Ok(()),
    }
}

/// The entry's path: the ustar prefix, when there is one, then the name.
fn path_of(header: &[u8; BLOCK as usize]) -> String {
    let field = |from: usize, to: usize| {
        let bytes = &header[from..to];
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    };
    let name = field(0, 100);
    if &header[257..262] == b"ustar" {
        let prefix = field(345, 500);
        if !prefix.is_empty() {
            return format!("{prefix}/{name}");
        }
    }
    name
}

/// The entry's length: octal digits, or the base-256 form GNU tar writes for
/// a file too large for them. `None` for a field that is neither.
fn size_of(header: &[u8; BLOCK as usize]) -> Option<u64> {
    let field = &header[124..136];
    if field[0] & 0x80 != 0 {
        return field[1..]
            .iter()
            .try_fold(u64::from(field[0] & 0x7f), |sum, byte| {
                sum.checked_mul(256)?.checked_add(u64::from(*byte))
            });
    }
    let digits = std::str::from_utf8(field).ok()?;
    let digits = digits.trim_matches(|c: char| c == '\0' || c == ' ');
    if digits.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(digits, 8).ok()
}

/// Step over `count` bytes of the archive.
fn skip(tar: &mut impl Read, count: u64) -> Result<(), MlError> {
    let skipped = io::copy(&mut tar.take(count), &mut io::sink()).map_err(invalid)?;
    if skipped == count {
        Ok(())
    } else {
        Err(cut_short())
    }
}

/// Read what follows the end marker: zeros to the end of the tar's last
/// record, then the gzip trailer, whose checksum the decoder checks as it
/// reaches it. Anything else there, a checksum that fails, or a byte after
/// the trailer, is a refusal.
fn to_the_end(mut tar: GzDecoder<BufReader<fs::File>>) -> Result<(), MlError> {
    let mut rest = [0u8; 8192];
    loop {
        let read = match tar.read(&mut rest) {
            Ok(0) => break,
            Ok(read) => read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(damaged()),
        };
        if rest.iter().take(read).any(|byte| *byte != 0) {
            return Err(damaged());
        }
    }
    let mut after = [0u8; 1];
    match tar.into_inner().read(&mut after) {
        Ok(0) => Ok(()),
        Ok(_) => Err(damaged()),
        Err(e) => Err(invalid(e)),
    }
}

/// The refusal for an archive whose checksum fails or that has something
/// after its end.
fn damaged() -> MlError {
    MlError::Invalid("the download is damaged".into())
}

/// The refusal for an archive that stops part way through.
fn cut_short() -> MlError {
    MlError::Invalid("the download ends part way through".into())
}

/// The system's own sentence, for a file that will not open or write.
fn invalid(e: io::Error) -> MlError {
    MlError::Invalid(e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A `.tar.gz` holding each named file with its bytes, as ustar writes it.
    fn a_tarball(dir: &Path, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let mut tar = Vec::new();
        for (name, bytes) in files {
            let mut header = [0u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[124..135].copy_from_slice(format!("{:011o}", bytes.len()).as_bytes());
            header[156] = b'0';
            header[257..263].copy_from_slice(b"ustar\0");
            tar.extend_from_slice(&header);
            tar.extend_from_slice(bytes);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        tar.extend_from_slice(&[0u8; 1024]);
        gzipped(dir, "pack.tar.gz", &tar)
    }

    /// `tar` gzipped into `dir` as `name`.
    fn gzipped(dir: &Path, name: &str, tar: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut gz = flate2::write::GzEncoder::new(
            fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        gz.write_all(tar).unwrap();
        gz.finish().unwrap();
        path
    }

    /// What `take` answers for the archive at `path`, which must be a refusal.
    fn refused(path: &Path, wanted: &BTreeMap<String, String>, into: &Path) -> MlError {
        let size = fs::metadata(path).unwrap().len();
        take(path, size, wanted, into).unwrap_err()
    }

    /// **A tar.gz gives up exactly the named files, and refuses a name that
    /// leaves the folder, a download of the wrong size, an archive cut short,
    /// a size field that overflows, an entry past the ceiling, and a damaged
    /// gzip or bytes after the end.** The download arrives over the network
    /// or from disk, so each of these is a file written where it should not
    /// be, a disk filled, or a pack that installs broken.
    #[test]
    fn a_tarball_gives_up_the_named_files_and_refuses_the_rest() {
        let downloads = tempfile::tempdir().unwrap();
        let into = tempfile::tempdir().unwrap();
        let archive = a_tarball(
            downloads.path(),
            &[
                ("tmp/export/enc.onnx", b"encoder"),
                ("tmp/export/config.ini", b"[df]"),
                ("tmp/export/df_dec.onnx", &[7u8; 700]),
            ],
        );
        let size = fs::metadata(&archive).unwrap().len();
        let wanted = BTreeMap::from([
            ("tmp/export/enc.onnx".to_string(), "enc.onnx".to_string()),
            (
                "tmp/export/df_dec.onnx".to_string(),
                "df_dec.onnx".to_string(),
            ),
        ]);

        take(&archive, size, &wanted, into.path()).unwrap();
        let mut names: Vec<String> = fs::read_dir(into.path())
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        assert_eq!(names, ["df_dec.onnx", "enc.onnx"], "and not config.ini");
        assert_eq!(fs::read(into.path().join("enc.onnx")).unwrap(), b"encoder");
        assert_eq!(
            fs::read(into.path().join("df_dec.onnx")).unwrap(),
            [7u8; 700]
        );

        let escaping =
            BTreeMap::from([("tmp/export/enc.onnx".to_string(), "../enc.onnx".to_string())]);
        let refusal = take(&archive, size, &escaping, into.path()).unwrap_err();
        assert!(matches!(refusal, MlError::Invalid(_)), "{refusal:?}");

        let refusal = take(&archive, size + 1, &wanted, into.path()).unwrap_err();
        let MlError::Invalid(why) = &refusal else {
            panic!("the wrong refusal: {refusal:?}");
        };
        assert!(why.contains("the manifest says"), "{why}");

        // The same archive with its last file's bytes cut off before the end.
        let bytes = fs::read(&archive).unwrap();
        let mut tar = Vec::new();
        flate2::read::GzDecoder::new(bytes.as_slice())
            .read_to_end(&mut tar)
            .unwrap();
        let whole = tar.clone();
        tar.truncate(512 * 6 + 100);
        let short = gzipped(downloads.path(), "short.tar.gz", &tar);
        assert_eq!(refused(&short, &wanted, into.path()), cut_short());

        // A file nobody asked for whose base-256 size is all ones: padded to
        // a whole block it passes the top of a u64.
        let mut huge = [0u8; 512];
        huge[..9].copy_from_slice(b"stray.bin");
        huge[124] = 0x80;
        huge[128..136].fill(0xff);
        huge[156] = b'0';
        let mut tar = huge.to_vec();
        tar.extend_from_slice(&[0u8; 4096]);
        let overflowing = gzipped(downloads.path(), "overflow.tar.gz", &tar);
        assert_eq!(refused(&overflowing, &wanted, into.path()), cut_short());

        // A wanted file whose header claims a byte past the ceiling. It is
        // refused before anything is written, so the bytes need not be there.
        let fresh = tempfile::tempdir().unwrap();
        let mut big = [0u8; 512];
        big[..19].copy_from_slice(b"tmp/export/enc.onnx");
        big[124..135]
            .copy_from_slice(format!("{:011o}", crate::store::MOST_UNPACKED + 1).as_bytes());
        big[156] = b'0';
        let mut tar = big.to_vec();
        tar.extend_from_slice(&[0u8; 4096]);
        let bomb = gzipped(downloads.path(), "bomb.tar.gz", &tar);
        let MlError::Invalid(why) = refused(&bomb, &wanted, fresh.path()) else {
            panic!("the wrong refusal");
        };
        assert!(why.contains("more than"), "{why}");
        assert_eq!(
            fs::read_dir(fresh.path()).unwrap().count(),
            0,
            "nothing written"
        );

        // The whole archive with something after its end marker.
        let mut tar = whole.clone();
        tar.extend_from_slice(b"and then some");
        let trailing = gzipped(downloads.path(), "trailing.tar.gz", &tar);
        assert_eq!(refused(&trailing, &wanted, into.path()), damaged());

        // The whole archive with bytes after the gzip trailer.
        let mut bytes = fs::read(gzipped(downloads.path(), "after.tar.gz", &whole)).unwrap();
        bytes.extend_from_slice(b"junk");
        let after = downloads.path().join("after.tar.gz");
        fs::write(&after, &bytes).unwrap();
        assert_eq!(refused(&after, &wanted, into.path()), damaged());

        // The whole archive with its checksum wrong: the trailer's first byte.
        let mut bytes = fs::read(gzipped(downloads.path(), "crc.tar.gz", &whole)).unwrap();
        let at = bytes.len() - 8;
        bytes[at] ^= 0xff;
        let bad = downloads.path().join("crc.tar.gz");
        fs::write(&bad, &bytes).unwrap();
        assert_eq!(refused(&bad, &wanted, into.path()), damaged());
    }
}
