//! Image sequences: a folder of numbered stills read as one piece of footage
//! (docs/03-DATA-MODEL.md §3, K-439).
//!
//! In plain terms: a 3D application's output arrives as thousands of files —
//! `Depth000000_depth.exr`, `Depth000001_depth.exr`, and so on — which
//! together are one shot. Lumit imports the run as a single footage item: the
//! user picks any one file, this module works out which numbered run that file
//! belongs to, and from then on "frame 12 of that item" means "the twelfth file
//! of the run".
//!
//! **Nothing decodes the files itself.** FFmpeg has read numbered runs since
//! long before Lumit existed — its `image2` demuxer takes a printf pattern such
//! as `Depth%06d_depth.exr` and hands back a video stream — so the whole
//! feature is a naming question: turn one file name into a pattern, a start
//! number and a length, and every existing probe, index, decode and cache path
//! works unchanged.
//!
//! **A gap ends the run** (K-439). Given `0001…0100` with `0050` missing and
//! `0007` picked, the sequence is `0001…0049`: the unbroken block the picked
//! file sits in. Refusing outright would make one deleted frame reject a whole
//! shot, and silently bridging the hole would show the wrong picture at the
//! wrong time without saying so.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::MediaError;

/// How many frames one sequence may hold (docs/14 §5, budgeted work). Well
/// past any real shot — a feature film at 24 fps is about 173 000 frames —
/// and it stops a directory of a million numbered files from being walked into
/// a single item.
pub const MAX_FRAMES: u32 = 1_000_000;

/// The numbered run one file belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The `image2` input: the containing directory joined with the file name,
    /// its number replaced by a printf field (`Depth%06d_depth.exr`).
    pub pattern: PathBuf,
    /// The number of the run's first file.
    pub start: u32,
    /// How many files are in the run. Never zero — the picked file is one.
    pub count: u32,
    /// The run's first file, as it actually is on disk. This is the path that
    /// gets fingerprinted and stat-ed, because the pattern names no file.
    pub first: PathBuf,
}

impl Run {
    /// What the Project panel calls this run: the file name with its span in
    /// place of the number — `Depth[000000-002270]_depth.exr`.
    ///
    /// After Effects' own shape, and the reason it is worth copying is that it
    /// answers, from the panel alone, the two questions a folder of stills
    /// raises: is this one item or two thousand, and where does it stop. It is
    /// built out of file names rather than words, so there is nothing here to
    /// translate.
    #[must_use]
    pub fn display_name(&self) -> String {
        let last = self.start.saturating_add(self.count.saturating_sub(1));
        let name = self
            .first
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        match split(name) {
            Some(p) => {
                let width = p.digits.len();
                format!(
                    "{}[{:0width$}-{:0width$}]{}",
                    p.prefix,
                    self.start,
                    last,
                    p.suffix,
                    width = width
                )
            }
            None => name.to_owned(),
        }
    }
}

/// A file name split at its number: `Depth`, 6, `_depth.exr`.
struct Parts<'a> {
    prefix: &'a str,
    digits: &'a str,
    suffix: &'a str,
}

/// Split `name` at the digit run that numbers it: the longest run of ASCII
/// digits in the file *stem*, latest one winning a tie.
///
/// Longest rather than last because a rendered frame's name often carries both a
/// version and a frame number — `shot_v2_0043.exr` is frame 43 of version 2,
/// not frame 2 — and the frame number is the wider field of the two. The
/// extension is held back so `.mp4`'s digit never wins.
fn split(name: &str) -> Option<Parts<'_>> {
    let dot = name.rfind('.').unwrap_or(name.len());
    let stem = name.get(..dot)?;

    let bytes = stem.as_bytes();
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let len = i - start;
            if best.is_none_or(|(bs, be)| len >= be - bs) {
                best = Some((start, i));
            }
        } else {
            i += 1;
        }
    }
    let (start, end) = best?;
    Some(Parts {
        prefix: stem.get(..start)?,
        digits: stem.get(start..end)?,
        suffix: name.get(end..)?,
    })
}

/// The extensions Lumit will offer to read as a sequence.
///
/// Not a decoder list — FFmpeg reads far more than this — but the list of
/// still-image formats where a numbered run *means* a sequence. A folder of
/// `clip0001.mp4`…`clip0100.mp4` is a hundred clips, not one sequence, and the
/// import must not quietly glue them together. Every entry here is decoded by
/// the FFmpeg Lumit links (PNG, TIFF, JPEG, OpenEXR, Targa, DPX, BMP, WebP and
/// the Netpbm family all ship in a default build).
pub const STILL_EXTENSIONS: &[&str] = &[
    "png", "tif", "tiff", "jpg", "jpeg", "exr", "tga", "targa", "dpx", "bmp", "webp", "ppm", "pgm",
    "pnm", "pbm",
];

/// Whether `path`'s extension is a still-image format a numbered run of which
/// should be offered as one sequence.
#[must_use]
pub fn is_still(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| STILL_EXTENSIONS.contains(&e.as_str()))
}

/// The numbered run `file` belongs to, or `None` when it is not part of one.
///
/// `None` covers every honest "this is just a file": a name with no digits in
/// it, a path with no directory, a directory that cannot be read, and a path
/// containing a `%` — which `image2` would read as a field of its own, so a run
/// under `C:\100%%\` is left alone rather than mis-addressed.
///
/// A file with digits that has no numbered neighbours still answers `Some`,
/// with `count` 1. That is the truthful answer — a one-frame run — and it keeps
/// "the user asked for a sequence" and "the files agree" separate questions.
#[must_use]
pub fn detect(file: &Path) -> Option<Run> {
    let dir = file.parent()?;
    let name = file.file_name()?.to_str()?;
    let parts = split(name)?;

    // `image2` scans the whole path for its field, so any other `%` in it —
    // directory included — would be read as one.
    if dir.as_os_str().to_str()?.contains('%')
        || parts.prefix.contains('%')
        || parts.suffix.contains('%')
    {
        return None;
    }

    let width = parts.digits.len();
    let picked: u32 = parts.digits.parse().ok()?;

    // Every neighbour that shares the prefix, the suffix and the field width.
    // A differently-padded name is a different run: `%06d` does not name
    // `frame7.png`, so treating it as part of the run would build a pattern
    // that skips it.
    let mut numbers: HashSet<u32> = HashSet::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let Some(other) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(p) = split(&other) else { continue };
        if p.prefix != parts.prefix || p.suffix != parts.suffix || p.digits.len() != width {
            continue;
        }
        if let Ok(n) = p.digits.parse::<u32>() {
            numbers.insert(n);
        }
    }
    if !numbers.contains(&picked) {
        // The picked file is gone between the pick and the scan, or is a
        // directory. Either way there is no run to speak for.
        return None;
    }

    // Walk out from the picked file to the first hole either side.
    let mut start = picked;
    while start > 0 && numbers.contains(&(start - 1)) {
        start -= 1;
    }
    let mut end = picked;
    while end < u32::MAX && numbers.contains(&(end + 1)) && end - start < MAX_FRAMES - 1 {
        end += 1;
    }

    Some(Run {
        pattern: dir.join(format!("{}%0{width}d{}", parts.prefix, parts.suffix)),
        start,
        count: end - start + 1,
        first: dir.join(format!(
            "{}{:0width$}{}",
            parts.prefix,
            start,
            parts.suffix,
            width = width
        )),
    })
}

/// What a probe, an index or a decoder is being pointed at: one media file, or
/// the numbered run one file belongs to.
///
/// Everything in the engine that opens media takes this, and a bare `&Path`
/// converts into it, so the overwhelmingly common "just this file" caller reads
/// exactly as it did before sequences existed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct MediaSource {
    /// The file the project points at. For a sequence this is *a* file of the
    /// run — usually the first — and it is what gets stat-ed, fingerprinted and
    /// relinked, because the pattern names no file on disk.
    pub path: PathBuf,
    /// `Some((num, den))` reads the numbered run `path` belongs to as one piece
    /// of footage at exactly that rate. The rate is the item's, not the files'
    /// — stills carry no frame rate of their own, so somebody has to say
    /// (K-439).
    pub sequence_fps: Option<(u32, u32)>,
}

impl MediaSource {
    /// A plain single file.
    #[must_use]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            sequence_fps: None,
        }
    }

    /// The run this source names, together with its rate — `None` when it is a
    /// plain file. An item marked as a sequence whose run cannot be worked out
    /// (the file vanished, the folder is unreadable) reads as a plain file,
    /// which is what the missing-media path is already built to handle.
    #[must_use]
    pub fn run(&self) -> Option<(Run, (u32, u32))> {
        let fps = self.sequence_fps?;
        Some((detect(&self.path)?, fps))
    }

    /// The path to stat and fingerprint: always a real file, never a pattern.
    #[must_use]
    pub fn on_disk(&self) -> &Path {
        &self.path
    }
}

impl From<&Path> for MediaSource {
    fn from(p: &Path) -> Self {
        Self::file(p)
    }
}

impl From<PathBuf> for MediaSource {
    fn from(p: PathBuf) -> Self {
        Self::file(p)
    }
}

impl From<&PathBuf> for MediaSource {
    fn from(p: &PathBuf) -> Self {
        Self::file(p.clone())
    }
}

impl From<&MediaSource> for MediaSource {
    fn from(s: &MediaSource) -> Self {
        s.clone()
    }
}

/// The `image2` input for `run` at `fps`, ready to open.
pub(crate) fn open_run(
    run: &Run,
    fps: (u32, u32),
) -> Result<rsmpeg::avformat::AVFormatContextInput, MediaError> {
    use rsmpeg::avformat::{AVFormatContextInput, AVInputFormat};
    use rsmpeg::avutil::AVDictionary;
    use std::ffi::CString;

    let url = CString::new(run.pattern.to_str().ok_or(MediaError::BadPath)?)
        .map_err(|_| MediaError::BadPath)?;
    let (num, den) = (fps.0.max(1), fps.1.max(1));

    // `start_number` pins the first file: left to itself `image2` searches
    // upwards from zero and gives up after five misses, so a run starting at
    // 1000 would not be found at all.
    let start = CString::new(run.start.to_string()).map_err(|_| MediaError::BadPath)?;
    let rate = CString::new(format!("{num}/{den}")).map_err(|_| MediaError::BadPath)?;
    let mut options =
        Some(AVDictionary::new(c"start_number", &start, 0).set(c"framerate", &rate, 0));

    // Named rather than probed: the demuxer for a pattern is never in doubt,
    // and probing a path that names no file is a guess waiting to go wrong.
    let format = AVInputFormat::find(c"image2")
        .ok_or_else(|| MediaError::Ffmpeg("no image2 demuxer in this FFmpeg".into()))?;

    AVFormatContextInput::builder()
        .url(&url)
        .format(&format)
        .options(&mut options)
        .open()
        .map_err(MediaError::from)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A 2×2 binary PPM of one colour. Netpbm is the one still format that can
    /// be written by hand in three lines — no checksum, no compression — which
    /// is what makes it the fixture for "does the decoder read file N as frame
    /// N" without pulling in an image encoder to find out.
    pub(crate) fn ppm(rgb: [u8; 3]) -> Vec<u8> {
        let mut out = b"P6\n2 2\n255\n".to_vec();
        for _ in 0..4 {
            out.extend_from_slice(&rgb);
        }
        out
    }

    /// Write `count` frames named `<prefix><n padded to width><suffix>` into
    /// `dir`, the n-th one red = n, so a decoded frame names its own number.
    pub(crate) fn write_run(dir: &Path, prefix: &str, width: usize, suffix: &str, ns: &[u32]) {
        for &n in ns {
            let name = format!("{prefix}{n:0width$}{suffix}", width = width);
            std::fs::write(dir.join(name), ppm([n as u8, 0, 0])).unwrap();
        }
    }

    #[test]
    fn the_frame_number_is_the_widest_digit_run_before_the_extension() {
        let p = split("Depth000000_depth.exr").unwrap();
        assert_eq!(
            (p.prefix, p.digits, p.suffix),
            ("Depth", "000000", "_depth.exr")
        );

        // A version tag must not win over the frame field.
        let p = split("shot_v2_0043.exr").unwrap();
        assert_eq!((p.prefix, p.digits, p.suffix), ("shot_v2_", "0043", ".exr"));

        // The extension's own digits are out of the running.
        let p = split("take7.mp4").unwrap();
        assert_eq!((p.prefix, p.digits, p.suffix), ("take", "7", ".mp4"));

        assert!(split("nodigits.png").is_none());
    }

    #[test]
    fn a_numbered_run_is_found_from_any_file_in_it() {
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), "name", 4, ".ppm", &(1..=10).collect::<Vec<_>>());

        for pick in [1u32, 4, 10] {
            let run = detect(&dir.path().join(format!("name{pick:04}.ppm")))
                .unwrap_or_else(|| panic!("picking {pick} finds the run"));
            assert_eq!(run.start, 1, "picking {pick}");
            assert_eq!(run.count, 10, "picking {pick}");
            assert_eq!(run.first, dir.path().join("name0001.ppm"));
            assert_eq!(run.pattern, dir.path().join("name%04d.ppm"));
        }
    }

    #[test]
    fn a_gap_ends_the_run_on_the_side_it_is_on() {
        // 1..=4, hole at 5, 6..=9 (K-439: clamp, never bridge).
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), "f", 3, ".ppm", &[1, 2, 3, 4, 6, 7, 8, 9]);

        let below = detect(&dir.path().join("f002.ppm")).unwrap();
        assert_eq!(
            (below.start, below.count),
            (1, 4),
            "the run stops at the hole"
        );

        let above = detect(&dir.path().join("f007.ppm")).unwrap();
        assert_eq!(
            (above.start, above.count),
            (6, 4),
            "the far side is its own run"
        );
    }

    #[test]
    fn a_differently_padded_neighbour_is_a_different_run() {
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), "f", 4, ".ppm", &[1, 2, 3]);
        // `f4.ppm` would be frame 4 by number, but `%04d` does not name it.
        std::fs::write(dir.path().join("f4.ppm"), ppm([4, 0, 0])).unwrap();

        let run = detect(&dir.path().join("f0002.ppm")).unwrap();
        assert_eq!((run.start, run.count), (1, 3));
    }

    #[test]
    fn a_lone_still_is_a_run_of_one_and_a_nameless_one_is_no_run() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("still0001.ppm"), ppm([1, 2, 3])).unwrap();
        std::fs::write(dir.path().join("plain.ppm"), ppm([1, 2, 3])).unwrap();

        let run = detect(&dir.path().join("still0001.ppm")).unwrap();
        assert_eq!((run.start, run.count), (1, 1));
        assert!(detect(&dir.path().join("plain.ppm")).is_none());
    }

    #[test]
    fn a_per_cent_anywhere_in_the_path_refuses_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("100%");
        std::fs::create_dir(&odd).unwrap();
        write_run(&odd, "f", 4, ".ppm", &[1, 2]);
        assert!(
            detect(&odd.join("f0001.ppm")).is_none(),
            "image2 would read the directory's per-cent as a field of its own"
        );

        write_run(dir.path(), "50%off_", 4, ".ppm", &[1, 2]);
        assert!(detect(&dir.path().join("50%off_0001.ppm")).is_none());
    }

    #[test]
    fn only_still_formats_are_offered_as_sequences() {
        assert!(is_still(Path::new("a/b0001.EXR")));
        assert!(is_still(Path::new("a/b0001.png")));
        assert!(
            !is_still(Path::new("a/clip0001.mp4")),
            "a folder of numbered clips is a hundred clips, not one sequence"
        );
        assert!(!is_still(Path::new("a/noext")));
    }

    /// The whole point of the feature, end to end: frame N of a sequence item
    /// is file N of the run, at the rate the item asked for.
    #[test]
    fn frame_n_of_a_sequence_is_file_n_of_the_run() {
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), "shot", 4, ".ppm", &(1..=6).collect::<Vec<_>>());

        let src = MediaSource {
            path: dir.path().join("shot0003.ppm"),
            sequence_fps: Some((30, 1)),
        };

        let probe = crate::probe::probe(&src).unwrap();
        let video = probe.video.expect("a run of stills is picture");
        assert_eq!((video.width, video.height), (2, 2));
        assert_eq!(
            (video.fps_num, video.fps_den),
            (30, 1),
            "stills carry no rate of their own, so the item's rate is the rate"
        );

        let index = crate::index::build_frame_index(&src).unwrap();
        assert_eq!(index.frame_count(), 6, "six files, six frames");

        let mut decoder = crate::VideoDecoder::open(&src, index).unwrap();
        // Out of order on purpose: a sequence must seek as well as run forward.
        for (frame, file) in [(0usize, 1u8), (4, 5), (2, 3), (5, 6), (1, 2)] {
            let out = decoder.frame_rgba(frame, None).unwrap();
            assert_eq!((out.width, out.height), (2, 2));
            assert_eq!(
                out.rgba.first().copied(),
                Some(file),
                "frame {frame} should be the file numbered {file}"
            );
        }
    }

    /// The clamp is not merely a detection rule: the run that decodes is the
    /// run that was detected, so the frames past the hole are unreachable
    /// rather than silently shifted into place.
    #[test]
    fn a_clamped_run_decodes_only_its_own_side_of_the_hole() {
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), "g", 4, ".ppm", &[1, 2, 3, 7, 8, 9]);

        let src = MediaSource {
            path: dir.path().join("g0002.ppm"),
            sequence_fps: Some((25, 1)),
        };
        let index = crate::index::build_frame_index(&src).unwrap();
        assert_eq!(index.frame_count(), 3, "the run stops at the hole");

        let mut decoder = crate::VideoDecoder::open(&src, index).unwrap();
        assert_eq!(decoder.frame_rgba(2, None).unwrap().rgba.first(), Some(&3));
        assert!(
            decoder.frame_rgba(3, None).is_err(),
            "frame 7 is past the hole and must not answer as frame 3"
        );
    }

    /// What Lumit exports, Lumit re-imports. The image-sequence export writes
    /// `shot.%05d.png` (`crate::encode::sequence_pattern`); the detector has to
    /// read that name back as the run it is, or the one sequence a user is
    /// guaranteed to own is the one that will not import.
    #[test]
    fn lumits_own_exported_sequence_reads_back_as_a_run() {
        let dir = tempfile::tempdir().unwrap();
        let chosen = dir.path().join("shot.png");
        for n in 1..=3 {
            std::fs::write(
                crate::encode::sequence_frame_path(&chosen, n),
                ppm([n as u8, 0, 0]),
            )
            .unwrap();
        }
        let run = detect(&crate::encode::sequence_frame_path(&chosen, 2)).unwrap();
        assert_eq!((run.start, run.count), (1, 3));
        assert_eq!(run.pattern, crate::encode::sequence_pattern(&chosen));
    }

    #[test]
    fn a_bare_path_converts_into_a_plain_source() {
        let src: MediaSource = Path::new("a/b.mp4").into();
        assert_eq!(src.sequence_fps, None);
        assert!(src.run().is_none());
        assert_eq!(src.on_disk(), Path::new("a/b.mp4"));
    }
}
