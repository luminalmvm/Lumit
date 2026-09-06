//! Making a proxy: transcoding one footage file into its low-resolution
//! stand-in (docs/06-RENDER-PIPELINE.md §5.7).
//!
//! # In plain terms
//!
//! A proxy is a small copy of a clip that the Viewer decodes while you work
//! (see [`crate::source::effective_media`] for how it is chosen). *Making* one
//! is the dullest job in the application: open the original, read every frame,
//! write each one out half as wide, close the file. It runs on its own thread
//! and reports progress the same way an export does, because to the person
//! waiting it is the same kind of wait.
//!
//! It writes through the same encoder every export writes through
//! ([`lumit_media::encode::Encoder`]), so a proxy is made by the code path that
//! is already exercised on every delivery rather than by a second, quieter one.
//! What it deliberately does **not** reuse is the compositor: a proxy stands in
//! for a *file*, not for a composition, so nothing here builds a draw list,
//! touches the GPU or needs a document.
//!
//! Sound is not copied. The audio path reads the item's original reference and
//! always did (`AudioJobsBuilder`), so a proxy carrying a re-encoded copy of the
//! sound would be a second, worse answer to a question nobody asked.

use lumit_media::encode::{ColourTags, Encoder, Metadata, VideoCodec, VideoSettings};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// How a MAKE-PROXY job reports itself — deliberately the same three-beat shape
/// [`crate::export::ExportEvent`] has, since the interface waits on both the
/// same way.
pub enum ProxyEvent {
    Progress {
        frame: usize,
        total: usize,
    },
    /// The finished file, ready to be attached to the item.
    Done(PathBuf),
    Failed(String),
}

/// A running MAKE-PROXY job.
pub struct ProxyHandle {
    pub events: Receiver<ProxyEvent>,
    cancel: Arc<AtomicBool>,
}

impl ProxyHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The denominator the proxy's width is divided by: half size, which is
/// a quarter of the pixels — enough to be several times cheaper to decode, and
/// still enough picture to judge framing and timing by.
pub const PROXY_DIVISOR: u32 = 2;

/// Where a proxy for `source` is written: beside the original, with
/// `_proxy` before a `.mov` extension — `shot.mp4` makes `shot_proxy.mov`.
///
/// Beside the original rather than in a project-relative folder, because a
/// proxy belongs to the *footage*: the same clip used by three projects wants
/// one proxy, not three, and a folder of clips carries its proxies with it when
/// it is moved. `.mov` for every source whatever the original's container is, so
/// the extension alone says which of the two files in a folder is the stand-in.
#[must_use]
pub fn proxy_path_for(source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "footage".to_string());
    source.with_file_name(format!("{stem}_proxy.mov"))
}

/// The proxy's width for a source `width` pixels wide: half, rounded to an even
/// number and never below two, because every codec here encodes in 2×2 chroma
/// blocks and an odd raster is not expressible.
#[must_use]
pub fn proxy_width(width: u32) -> u32 {
    (width / PROXY_DIVISOR).max(2) & !1
}

/// Start a MAKE-PROXY job on its own thread: transcode `source` to `dest` at
/// half width. Progress streams back through the handle; cancelling removes the
/// half-written file rather than leaving a trap that looks like a finished
/// proxy.
pub fn start(source: PathBuf, dest: PathBuf) -> ProxyHandle {
    let (tx, events) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let result = run(&source, &dest, &tx, &flag);
        let _ = match result {
            Ok(()) if flag.load(Ordering::Relaxed) => {
                let _ = std::fs::remove_file(&dest);
                tx.send(ProxyEvent::Failed("cancelled".into()))
            }
            Ok(()) => tx.send(ProxyEvent::Done(dest)),
            Err(e) => {
                let _ = std::fs::remove_file(&dest);
                tx.send(ProxyEvent::Failed(e))
            }
        };
    });
    ProxyHandle { events, cancel }
}

/// The transcode itself, so the same work can be run to completion in a test
/// without a thread or a channel. `progress` is called after each frame is
/// written; `cancel` is checked before each.
///
/// **Every frame, in order, and exactly as many as the original has.** The
/// resolution point refuses a proxy whose frame count disagrees with the
/// original's ([`crate::source::effective_media`]), so a transcode that dropped
/// or added one would produce a file that is simply never used — which is the
/// safe failure, but a wasted wait. Reading sequentially is also what the
/// decoder is fastest at: no seeks at all.
pub fn transcode(
    source: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(), String> {
    let probe = lumit_media::probe::probe(source).map_err(|e| e.to_string())?;
    let video = probe
        .video
        .as_ref()
        .ok_or_else(|| "a proxy needs a video stream to stand in for".to_string())?;
    let fps = video.fps();
    let index = crate::media_index::load_or_build_index(source).map_err(|e| e.to_string())?;
    let total = index.frame_count();
    if total == 0 {
        return Err("the source decodes no frames".into());
    }
    let mut decoder =
        lumit_media::decode::VideoDecoder::open(source, index).map_err(|e| e.to_string())?;

    let target_w = proxy_width(video.width);
    // The first frame decides the file's size rather than arithmetic doing it:
    // the scaler picks the height that keeps the aspect (and keeps it even), so
    // asking it is the only way to be sure the encoder is opened at the size the
    // frames actually arrive in.
    let first = decoder
        .frame_rgba(0, Some(target_w))
        .map_err(|e| e.to_string())?;
    let (fps_num, fps_den) = crate::export::fps_rational(fps);
    let settings = VideoSettings {
        codec: VideoCodec::H264,
        width: first.width,
        height: first.height,
        fps_num,
        fps_den,
        // No bitrate: the encoder's own quality choice is right for a stand-in,
        // and a number here would be a guess applied to every clip alike.
        bit_rate: None,
        max_rate: None,
        colour: ColourTags::default(),
    };
    let mut encoder =
        Encoder::open(dest, Some(&settings), None, &Metadata::new()).map_err(|e| e.to_string())?;
    encoder.write_rgba(&first.rgba).map_err(|e| e.to_string())?;
    progress(1, total);
    for n in 1..total {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let frame = decoder
            .frame_rgba(n, Some(target_w))
            .map_err(|e| format!("proxy frame {n}: {e}"))?;
        encoder.write_rgba(&frame.rgba).map_err(|e| e.to_string())?;
        progress(n + 1, total);
    }
    encoder.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn run(
    source: &Path,
    dest: &Path,
    tx: &Sender<ProxyEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut progress = |frame: usize, total: usize| {
        let _ = tx.send(ProxyEvent::Progress { frame, total });
    };
    transcode(source, dest, cancel, &mut progress)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The naming rule, on the shapes a real path takes.
    #[test]
    fn a_proxy_is_named_beside_its_original() {
        assert_eq!(
            proxy_path_for(Path::new("/clips/shot.mp4")),
            PathBuf::from("/clips/shot_proxy.mov")
        );
        // Whatever the container, the proxy is a .mov.
        assert_eq!(
            proxy_path_for(Path::new("/clips/shot.mov")),
            PathBuf::from("/clips/shot_proxy.mov")
        );
        // A dotted name keeps everything before the last dot.
        assert_eq!(
            proxy_path_for(Path::new("/clips/shot.v2.mp4")),
            PathBuf::from("/clips/shot.v2_proxy.mov")
        );
    }

    /// Half, even, never zero — the raster rule every codec here needs.
    #[test]
    fn the_proxy_width_is_half_and_even() {
        assert_eq!(proxy_width(1920), 960);
        assert_eq!(proxy_width(1921), 960);
        // 1080/2 = 540, already even.
        assert_eq!(proxy_width(1080), 540);
        // Odd halves round down to even.
        assert_eq!(proxy_width(1078), 538);
        // Tiny sources still make an expressible raster.
        assert_eq!(proxy_width(3), 2);
        assert_eq!(proxy_width(1), 2);
        assert_eq!(proxy_width(0), 2);
    }

    /// End to end on a real file: encode a small clip, make its proxy, and read
    /// the proxy back with our own probe. Half the width, the same frame count
    /// — which is the agreement `effective_media` insists on before it will use
    /// a proxy at all, so a transcode that failed it would produce a file the
    /// renderer silently ignored.
    #[test]
    fn making_a_proxy_halves_the_picture_and_keeps_every_frame() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("shot.mp4");
        let settings = VideoSettings {
            codec: VideoCodec::H264,
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            bit_rate: None,
            max_rate: None,
            colour: ColourTags::default(),
        };
        let mut enc = Encoder::open(&source, Some(&settings), None, &Metadata::new()).unwrap();
        for n in 0..12u8 {
            // A changing picture, so a transcode that wrote one frame twelve
            // times would not be indistinguishable from a correct one.
            let mut rgba = vec![0u8; 320 * 240 * 4];
            for px in rgba.chunks_exact_mut(4) {
                px[0] = n * 20;
                px[1] = 40;
                px[2] = 200u8.saturating_sub(n * 10);
                px[3] = 255;
            }
            enc.write_rgba(&rgba).unwrap();
        }
        enc.finish().unwrap();

        let dest = proxy_path_for(&source);
        assert_eq!(dest, dir.path().join("shot_proxy.mov"));
        let mut seen = Vec::new();
        transcode(
            &source,
            &dest,
            &AtomicBool::new(false),
            &mut |frame, total| seen.push((frame, total)),
        )
        .unwrap();

        let probe = lumit_media::probe::probe(&dest).unwrap();
        let video = probe.video.unwrap();
        assert_eq!(video.width, 160, "the proxy is half the original's width");
        assert_eq!(video.height, 120);
        let src_frames = crate::media_index::load_or_build_index(&source)
            .unwrap()
            .frame_count();
        let proxy_frames = crate::media_index::load_or_build_index(&dest)
            .unwrap()
            .frame_count();
        assert_eq!(
            proxy_frames, src_frames,
            "a proxy that lost a frame is one the renderer would refuse"
        );
        assert_eq!(
            seen.len(),
            src_frames,
            "progress is reported once per frame written"
        );
        assert_eq!(seen.last().copied(), Some((src_frames, src_frames)));
    }

    /// Cancelling stops the transcode where it stands, and reports no error —
    /// a job that was asked to stop did not fail.
    #[test]
    fn a_cancelled_transcode_stops_and_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("shot.mp4");
        let settings = VideoSettings {
            codec: VideoCodec::H264,
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            bit_rate: None,
            max_rate: None,
            colour: ColourTags::default(),
        };
        let mut enc = Encoder::open(&source, Some(&settings), None, &Metadata::new()).unwrap();
        for _ in 0..12 {
            enc.write_rgba(&vec![90u8; 320 * 240 * 4]).unwrap();
        }
        enc.finish().unwrap();

        let cancel = AtomicBool::new(true);
        let dest = dir.path().join("cancelled_proxy.mov");
        let mut frames = 0usize;
        let out = transcode(&source, &dest, &cancel, &mut |_, _| frames += 1);
        assert!(out.is_ok(), "cancelling is not a failure: {out:?}");
        assert_eq!(frames, 1, "the first frame opens the file, then it stops");
    }
}
