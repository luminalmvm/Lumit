//! The browser build's side of footage. The page reads each file with the
//! browser's own decoders and hands in what it found: the file's numbers when
//! it is opened (here), its sound (`sound`), and each frame once it is decoded
//! (`frames`). The bytes stay with the page. The page writes an export's
//! file too (`export`). FFmpeg's encoders are not here, so opening one
//! answers an error and the methods of one can never be reached.

pub mod export;
pub mod frames;
pub mod sound;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::decode::DecodedFrame;
use crate::encode::gif::{GifEncoder, GifOptions};
use crate::encode::{
    AudioSettings, BitDepth, ColourTags, Encoder, ImageFormat, ImageSequenceEncoder, Metadata,
    VideoSettings,
};
use crate::index::{FrameIndex, IndexEntry};
use crate::probe::MediaProbe;
use crate::{no_ffmpeg, Fingerprint, MediaError};

/// What the page found when it opened a file.
pub struct Opened {
    pub probe: MediaProbe,
    pub size: u64,
    /// When the file was last changed, in seconds since 1970.
    pub modified: i64,
    /// The file's first 64 KiB, then its last 64 KiB when it is longer than
    /// twice that: what [`Fingerprint::of`] hashes on the desktop.
    pub ends: Vec<u8>,
    /// The picture's clock, in ticks a second.
    pub ticks: u32,
    /// Each frame's moment on that clock, and whether it is a keyframe.
    pub frames: Vec<(i64, bool)>,
}

/// One file the page holds.
struct Held {
    probe: MediaProbe,
    fingerprint: Fingerprint,
    index: FrameIndex,
    /// The sound at the file's own rate, interleaved stereo.
    sound: Option<(u32, Arc<[f32]>)>,
}

thread_local! {
    static HELD: RefCell<HashMap<PathBuf, Held>> = RefCell::new(HashMap::new());
}

/// File what the page found in `path`, so every question about it is
/// answered from here.
pub fn register(path: PathBuf, opened: Opened) {
    let fingerprint = Fingerprint {
        size: opened.size,
        mtime_unix: opened.modified,
        content_hash: blake3::hash(&opened.ends).to_hex().to_string(),
    };
    let mut entries: Vec<IndexEntry> = opened
        .frames
        .iter()
        .map(|&(pts, keyframe)| IndexEntry { pts, keyframe })
        .collect();
    entries.sort_by_key(|e| e.pts);
    entries.dedup_by_key(|e| e.pts);
    let mut deltas: Vec<i64> = entries.windows(2).map(|w| w[1].pts - w[0].pts).collect();
    deltas.sort_unstable();
    // ponytail: an uneven recording is read frame by frame and not conformed
    // to its average rate as the desktop does, so it can drift against its
    // sound. Share the desktop's conform step if phone footage shows it.
    let index = FrameIndex {
        timebase_num: 1,
        timebase_den: i32::try_from(opened.ticks.max(1)).unwrap_or(i32::MAX),
        entries,
        vfr: false,
        median_delta: deltas.get(deltas.len() / 2).copied().unwrap_or(0),
        fingerprint: fingerprint.clone(),
        interpret: Default::default(),
    };
    let held = Held {
        probe: opened.probe,
        fingerprint,
        index,
        sound: None,
    };
    lumit_ingress::files::hold(&path);
    HELD.with(|files| files.borrow_mut().insert(path, held));
}

/// File `path`'s sound: interleaved stereo at `rate`.
pub fn register_sound(path: &Path, rate: u32, samples: Vec<f32>) {
    HELD.with(|files| {
        if let Some(held) = files.borrow_mut().get_mut(path) {
            held.sound = Some((rate, samples.into()));
        }
    });
}

fn with<T>(path: &Path, read: impl FnOnce(&Held) -> T) -> Option<T> {
    HELD.with(|files| files.borrow().get(path).map(read))
}

pub(crate) fn holds(path: &Path) -> bool {
    with(path, |_| ()).is_some()
}

pub(crate) fn probe(path: &Path) -> Option<MediaProbe> {
    with(path, |held| held.probe.clone())
}

pub(crate) fn fingerprint(path: &Path) -> Option<Fingerprint> {
    with(path, |held| held.fingerprint.clone())
}

/// The frame index, for a file with a picture.
pub(crate) fn index(path: &Path) -> Option<FrameIndex> {
    with(path, |held| {
        held.probe.video.is_some().then(|| held.index.clone())
    })
    .flatten()
}

pub(crate) fn sound(path: &Path) -> Option<(u32, Arc<[f32]>)> {
    with(path, |held| held.sound.clone()).flatten()
}

impl ImageSequenceEncoder {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        _path: &Path,
        _format: ImageFormat,
        _width: u32,
        _height: u32,
        _fps_num: i32,
        _fps_den: i32,
        _depth: BitDepth,
        _colour: ColourTags,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn write_rgba(&mut self, _rgba: &[u8]) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn finish(&mut self) -> Result<(), MediaError> {
        match self.0 {}
    }
}

impl GifEncoder {
    pub fn open(
        _path: &Path,
        _width: u32,
        _height: u32,
        _fps_num: i32,
        _fps_den: i32,
        _options: GifOptions,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn write_rgba(&mut self, _rgba: &[u8]) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn finish(&mut self) -> Result<(), MediaError> {
        match self.0 {}
    }
}

impl Encoder {
    pub fn open(
        _path: &Path,
        _video: Option<&VideoSettings>,
        _audio: Option<&AudioSettings>,
        _metadata: &Metadata,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn open_with_sound(
        _path: &Path,
        _video: Option<&VideoSettings>,
        _audio: Option<&AudioSettings>,
        _more_sound: usize,
        _metadata: &Metadata,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn open_with_subtitles(
        _path: &Path,
        _video: Option<&VideoSettings>,
        _audio: Option<&AudioSettings>,
        _languages: &[String],
        _metadata: &Metadata,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn write_subtitle(
        &mut self,
        _stream: usize,
        _start_ms: i64,
        _end_ms: i64,
        _lines: &[String],
    ) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn encoder_name(&self) -> &'static str {
        match self.0 {}
    }

    pub fn encoder_label(&self) -> &'static str {
        match self.0 {}
    }

    pub fn has_video(&self) -> bool {
        match self.0 {}
    }

    pub fn write_rgba(&mut self, _rgba: &[u8]) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn write_audio(&mut self, _interleaved: &[f32]) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn write_audio_stream(
        &mut self,
        _stream: usize,
        _interleaved: &[f32],
    ) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn finish(&mut self) -> Result<(), MediaError> {
        match self.0 {}
    }
}

/// Whether this is a still the page reads itself, with no FFmpeg: a PNG, a
/// JPEG, or an EXR by its own reader.
pub(crate) fn reads_still(path: &Path) -> bool {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    ["png", "jpg", "jpeg", "exr"]
        .iter()
        .any(|known| extension.eq_ignore_ascii_case(known))
}

/// The longest still file the page reads.
const STILL_MAX: u64 = 1 << 30;

/// The picture of a still that [`reads_still`], at its own size.
pub(crate) fn read_still(path: &Path) -> Result<DecodedFrame, MediaError> {
    if crate::exr::is_exr(path) {
        return crate::exr::read_pass(path, None);
    }
    let bytes = lumit_ingress::read_capped(path, STILL_MAX)?;
    let unreadable = |e: String| MediaError::Ffmpeg(format!("could not read the picture: {e}"));
    let (width, height, samples) = if bytes.starts_with(b"\x89PNG") {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().map_err(|e| unreadable(e.to_string()))?;
        let size = reader.output_buffer_size().ok_or(MediaError::NoStreams)?;
        let mut samples = vec![0; size];
        let info = reader
            .next_frame(&mut samples)
            .map_err(|e| unreadable(e.to_string()))?;
        samples.truncate(info.buffer_size());
        (info.width, info.height, samples)
    } else {
        let mut decoder = zune_jpeg::JpegDecoder::new(std::io::Cursor::new(&bytes));
        let samples = decoder.decode().map_err(|e| unreadable(e.to_string()))?;
        let info = decoder.info().ok_or(MediaError::NoStreams)?;
        (u32::from(info.width), u32::from(info.height), samples)
    };
    let area = lumit_ingress::checked_area(u64::from(width), u64::from(height))?;
    let channels = samples.len() / lumit_ingress::checked_usize(area.max(1))?;
    let mut budget = lumit_ingress::Budget::new(lumit_ingress::Limits::IMAGE);
    let mut rgba =
        budget.vec_with_capacity(lumit_ingress::checked_usize(area)?.saturating_mul(4))?;
    for px in samples.chunks_exact(channels.max(1)) {
        rgba.extend_from_slice(&match *px {
            [g] => [g, g, g, 255],
            [g, a] => [g, g, g, a],
            [r, g, b] => [r, g, b, 255],
            [r, g, b, a] => [r, g, b, a],
            _ => return Err(MediaError::NoStreams),
        });
    }
    Ok(DecodedFrame {
        width,
        height,
        rgba,
        format: crate::decode::PixelFormat::Srgb8,
    })
}
