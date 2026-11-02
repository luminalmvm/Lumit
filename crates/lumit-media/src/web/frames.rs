//! The frames the page has decoded, and the decoder a render reads them
//! through.
//!
//! A render can't wait for the page: both run on the page's one thread, and
//! the page decodes in between. So a render whose frame is not here yet asks
//! for it and draws the nearest frame of the file that is, and counts that
//! ([`stand_ins`]) so the picture is never kept. When the frame lands,
//! [`landed`] moves and the picture is made again.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::decode::{DecodedFrame, LumaFrame, PixelFormat, VideoDecoder};
use crate::index::FrameIndex;
use crate::interpret::{Interpretation, Pick};
use crate::sequence::MediaSource;
use crate::MediaError;

/// The most the frames held here may weigh: about seventy 720p frames, or
/// thirty 1080p ones.
const BUDGET: usize = 256 << 20;

/// A frame is filed by its file, the file's own frame number and its width.
type Key = (PathBuf, usize, u32);

#[derive(Default)]
struct Store {
    frames: BTreeMap<Key, DecodedFrame>,
    /// Longest since it landed or playback asked for it first, which is the
    /// order they go in when the budget is spent.
    order: VecDeque<Key>,
    bytes: usize,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
    static LANDED: Cell<u64> = const { Cell::new(0) };
    static STAND_INS: Cell<u64> = const { Cell::new(0) };
}

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    /// Ask the page for a frame (flutter_ui/web/media.js).
    #[wasm_bindgen(catch, js_namespace = lumitMedia, js_name = want)]
    fn page_want(
        path: &str,
        frame: usize,
        at: f64,
        width: u32,
        height: u32,
        ahead: bool,
    ) -> Result<(), wasm_bindgen::JsValue>;
}

/// How many frames the page has handed in.
pub(crate) fn landed() -> u64 {
    LANDED.with(Cell::get)
}

/// How many times a render has drawn a stand-in.
pub(crate) fn stand_ins() -> u64 {
    STAND_INS.with(Cell::get)
}

/// File frame `frame` of `path`, decoded by the page at `width` by `height`.
pub fn land(path: &Path, frame: usize, width: u32, height: u32, rgba: Vec<u8>) {
    if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return;
    }
    STORE.with(|store| {
        let mut store = store.borrow_mut();
        let key = (path.to_path_buf(), frame, width);
        store.bytes += rgba.len();
        let frame = DecodedFrame {
            width,
            height,
            rgba,
            format: PixelFormat::Srgb8,
        };
        if let Some(old) = store.frames.insert(key.clone(), frame) {
            store.bytes -= old.rgba.len();
            store.order.retain(|held| held != &key);
        }
        store.order.push_back(key);
        while store.bytes > BUDGET && store.order.len() > 1 {
            let Some(oldest) = store.order.pop_front() else {
                break;
            };
            if let Some(gone) = store.frames.remove(&oldest) {
                store.bytes -= gone.rgba.len();
            }
        }
    });
    LANDED.with(|n| n.set(n.get() + 1));
}

fn copy(frame: &DecodedFrame) -> DecodedFrame {
    DecodedFrame {
        width: frame.width,
        height: frame.height,
        rgba: frame.rgba.clone(),
        format: frame.format,
    }
}

/// One frame as a render wants it: the file's own frame, the size, and the
/// moment in the file to ask the page for.
struct Want {
    raw: usize,
    width: u32,
    height: u32,
    at: f64,
}

fn want_of(
    index: &FrameIndex,
    it: &Interpretation,
    size: (u32, u32),
    n: usize,
    target_width: Option<u32>,
) -> Option<Want> {
    let count = index.entries.len();
    // ponytail: a field or a pulled-down frame is read as the whole frame it
    // comes from. Weave in the page's frames if interlaced footage needs it.
    let raw = match it.pick(n, count) {
        Pick::Whole(raw) | Pick::Field { frame: raw, .. } | Pick::Weave { first: raw, .. } => raw,
    };
    let entry = index.entries.get(raw.min(count.checked_sub(1)?))?;
    let wide = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
    let (width, height) =
        crate::decode::scaled_size(wide(size.0), wide(size.1), target_width, it.pixel_aspect);
    // Half a frame in, so the page lands inside the frame and not on its edge.
    let ticks = entry.pts as f64 + index.median_delta as f64 / 2.0;
    Some(Want {
        raw,
        width: u32::try_from(width).unwrap_or(0),
        height: u32::try_from(height).unwrap_or(0),
        at: ticks * f64::from(index.timebase_num) / f64::from(index.timebase_den.max(1)),
    })
}

fn held(path: &Path, want: &Want) -> Option<DecodedFrame> {
    STORE.with(|store| {
        let key = (path.to_path_buf(), want.raw, want.width);
        store.borrow().frames.get(&key).map(copy)
    })
}

/// Whether the frame is here, moved to the back of the queue to go if it is:
/// playback is about to show it.
fn keep(path: &Path, want: &Want) -> bool {
    STORE.with(|store| {
        let mut store = store.borrow_mut();
        let key = (path.to_path_buf(), want.raw, want.width);
        let here = store.frames.contains_key(&key);
        if here {
            store.order.retain(|held| held != &key);
            store.order.push_back(key);
        }
        here
    })
}

fn ask(path: &Path, want: &Want, ahead: bool) {
    let path = path.to_string_lossy();
    let _ = page_want(&path, want.raw, want.at, want.width, want.height, ahead);
}

/// One file's picture, read from the frames the page decodes.
pub(crate) struct Video {
    path: PathBuf,
    index: FrameIndex,
    /// The picture's size as the page shows it, turned and in square pixels.
    size: (u32, u32),
}

impl Video {
    fn open(src: MediaSource, mut index: FrameIndex) -> Result<Self, MediaError> {
        let video = super::probe(src.on_disk())
            .and_then(|probe| probe.video)
            .ok_or(MediaError::NoStreams)?;
        index.interpret = src.interpret;
        Ok(Self {
            path: src.path,
            index,
            size: (video.width, video.height),
        })
    }

    fn want(&self, n: usize, target_width: Option<u32>) -> Result<Want, MediaError> {
        want_of(
            &self.index,
            &self.index.interpret,
            self.size,
            n,
            target_width,
        )
        .ok_or(MediaError::NoStreams)
    }

    /// The nearest frame of this file that is here, at the size wanted, or a
    /// clear one when none is.
    fn stand_in(&self, want: &Want) -> DecodedFrame {
        let near = STORE.with(|store| {
            let store = store.borrow();
            let from = (self.path.clone(), 0, 0);
            let to = (self.path.clone(), usize::MAX, u32::MAX);
            store
                .frames
                .range(from..=to)
                .min_by_key(|((_, frame, _), _)| frame.abs_diff(want.raw))
                .map(|(_, frame)| copy(frame))
        });
        match near {
            Some(frame) => crate::interpret::resize(frame, want.width, want.height),
            None => DecodedFrame {
                width: want.width,
                height: want.height,
                rgba: vec![0; want.width as usize * want.height as usize * 4],
                format: PixelFormat::Srgb8,
            },
        }
    }
}

/// Ask the page for a frame playback is about to show, behind any a render
/// is waiting on. True when it is here already.
pub fn ahead(src: &MediaSource, n: usize, target_width: Option<u32>) -> bool {
    let want = super::with(src.on_disk(), |held| {
        let video = held.probe.video.as_ref()?;
        want_of(
            &held.index,
            &src.interpret,
            (video.width, video.height),
            n,
            target_width,
        )
    })
    .flatten();
    let Some(want) = want else {
        return true;
    };
    let here = keep(src.on_disk(), &want);
    if !here {
        ask(src.on_disk(), &want, true);
    }
    here
}

impl VideoDecoder {
    pub fn open(src: impl Into<MediaSource>, index: FrameIndex) -> Result<Self, MediaError> {
        Ok(Self(Video::open(src.into(), index)?))
    }

    pub fn open_with(
        src: impl Into<MediaSource>,
        index: FrameIndex,
        _allow_hardware: bool,
    ) -> Result<Self, MediaError> {
        Self::open(src, index)
    }

    pub fn frame_count(&self) -> usize {
        self.0.index.frame_count()
    }

    pub fn seeks(&self) -> usize {
        0
    }

    pub fn is_hardware(&self) -> bool {
        false
    }

    /// Exactly frame `n`, once the page has decoded it. Until then it is
    /// asked for and this answers an error.
    pub fn frame_rgba(
        &mut self,
        n: usize,
        target_width: Option<u32>,
    ) -> Result<DecodedFrame, MediaError> {
        let want = self.0.want(n, target_width)?;
        match held(&self.0.path, &want) {
            Some(frame) => Ok(crate::interpret::finish(frame, &self.0.index.interpret)),
            None => {
                ask(&self.0.path, &want, false);
                Err(MediaError::Ffmpeg(
                    "the page has not decoded this frame yet".into(),
                ))
            }
        }
    }

    /// Frame `n` for a render: the frame itself once the page has decoded it,
    /// and until then the nearest one that is here, counted in [`stand_ins`].
    pub fn frame_or_stand_in(
        &mut self,
        n: usize,
        target_width: Option<u32>,
    ) -> Result<DecodedFrame, MediaError> {
        let want = self.0.want(n, target_width)?;
        let frame = held(&self.0.path, &want).unwrap_or_else(|| {
            ask(&self.0.path, &want, false);
            STAND_INS.with(|n| n.set(n.get() + 1));
            self.0.stand_in(&want)
        });
        Ok(crate::interpret::finish(frame, &self.0.index.interpret))
    }

    pub fn frame_luma(&mut self, n: usize) -> Result<LumaFrame, MediaError> {
        Ok(crate::decode::luma_of(&self.frame_rgba(n, None)?))
    }
}
