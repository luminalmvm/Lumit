//! Animated GIF: palettes picked and dithered by FFmpeg's own filters.

#[cfg(not(target_arch = "wasm32"))]
use super::planes::{copy_pixels_into, frame_len};
#[cfg(not(target_arch = "wasm32"))]
use super::{at, dict_set, drain_packets};
#[cfg(not(target_arch = "wasm32"))]
use crate::MediaError;
#[cfg(not(target_arch = "wasm32"))]
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
#[cfg(not(target_arch = "wasm32"))]
use rsmpeg::avformat::AVFormatContextOutput;
#[cfg(not(target_arch = "wasm32"))]
use rsmpeg::avutil::{AVFrame, AVRational};
#[cfg(not(target_arch = "wasm32"))]
use rsmpeg::ffi;
#[cfg(not(target_arch = "wasm32"))]
use std::ffi::{CStr, CString};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// How a GIF spreads a picture over its 256 colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum GifDither {
    /// Error diffusion, the smoothest gradients.
    #[default]
    Diffusion,
    /// An ordered pattern, which holds still from frame to frame and makes a
    /// smaller file.
    Pattern,
    /// The nearest colour and nothing else.
    None,
}

#[cfg(not(target_arch = "wasm32"))]
impl GifDither {
    /// `paletteuse`'s own name for it.
    fn filter_value(self) -> &'static str {
        match self {
            GifDither::Diffusion => "sierra2_4a",
            GifDither::Pattern => "bayer",
            GifDither::None => "none",
        }
    }
}

/// The choices a GIF has that no other format does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GifOptions {
    pub dither: GifDither,
    /// A palette for each frame instead of one for the whole GIF. Each frame
    /// gets truer colour, but a still part of the picture can flicker.
    pub palette_per_frame: bool,
    /// How many times it plays. None plays forever.
    pub plays: Option<u32>,
}

/// The browser build has no FFmpeg, so nothing is ever written this way there.
#[cfg(target_arch = "wasm32")]
pub struct GifEncoder(pub(crate) std::convert::Infallible);

/// Writes an animated `.gif`. GIF holds 256 colours a frame, so FFmpeg's
/// `palettegen` picks them and `paletteuse` dithers the picture onto them,
/// the same pair the GIF converters use. A pixel under half alpha is written
/// clear, which is as far as GIF transparency goes.
///
/// One palette for the whole GIF can only be made once every frame has been
/// seen, so the frames wait in a temporary file rather than in memory, and
/// are written out at [`Self::finish`].
#[cfg(not(target_arch = "wasm32"))]
pub struct GifEncoder {
    output: AVFormatContextOutput,
    video: AVCodecContext,
    /// A palette for each frame: the whole graph. One palette: `palettegen`
    /// alone, counting colours as the frames go past.
    graph: rsmpeg::avfilter::AVFilterGraph,
    /// One palette only: every frame as it came, read back at the end.
    spool: Option<std::io::BufWriter<std::fs::File>>,
    dither: GifDither,
    width: i32,
    height: i32,
    time_base: AVRational,
    next_pts: i64,
    finished: bool,
}

/// A `va_list` as a C function receives it. On x86-64 outside Windows it
/// is an array, and an array argument arrives as a pointer to its first item.
#[cfg(all(target_arch = "x86_64", not(windows)))]
type VaList = *mut ffi::__va_list_tag;
#[cfg(not(any(all(target_arch = "x86_64", not(windows)), target_arch = "wasm32")))]
type VaList = ffi::va_list;

/// palettegen says how many colours it made at info level, which with a
/// palette for each frame is a line a frame. Everything else goes through.
///
/// SAFETY: FFmpeg calls this with `avcl` null or a live context whose first
/// field is its `AVClass` pointer, which may be null. `item_name` gives a
/// string that lives at least as long as the context. `fmt` and `vl` are
/// handed on to FFmpeg's own logger untouched, as it would have been called.
#[cfg(not(target_arch = "wasm32"))]
#[allow(unsafe_code)]
unsafe extern "C" fn quiet_palettegen(
    avcl: *mut std::ffi::c_void,
    level: std::ffi::c_int,
    fmt: *const std::ffi::c_char,
    vl: VaList,
) {
    let from_palettegen = || {
        if avcl.is_null() {
            return false;
        }
        let class = unsafe { *(avcl as *const *const ffi::AVClass) };
        let Some(item_name) = unsafe { class.as_ref() }.and_then(|c| c.item_name) else {
            return false;
        };
        let name = unsafe { item_name(avcl) };
        // The GIF graph names its palettegen after the filter.
        !name.is_null() && unsafe { CStr::from_ptr(name) }.to_bytes() == b"palettegen"
    };
    let quiet = level >= ffi::AV_LOG_INFO as std::ffi::c_int
        && std::panic::catch_unwind(from_palettegen).unwrap_or(false);
    if !quiet {
        unsafe { ffi::av_log_default_callback(avcl, level, fmt, vl) };
    }
}

/// Send FFmpeg's log through [`quiet_palettegen`], once per process.
///
/// SAFETY: the callback is a plain function that lives for the program.
#[cfg(not(target_arch = "wasm32"))]
#[allow(unsafe_code)]
fn quieten_palettegen() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe { ffi::av_log_set_callback(Some(quiet_palettegen)) });
}
/// Add one filter to `graph`, named `name`.
#[cfg(not(target_arch = "wasm32"))]
fn add_filter<'g>(
    graph: &'g rsmpeg::avfilter::AVFilterGraph,
    filter: &CStr,
    name: &CStr,
    args: Option<&str>,
) -> Result<rsmpeg::avfilter::AVFilterContextMut<'g>, MediaError> {
    let what = || filter.to_string_lossy().into_owned();
    let found = rsmpeg::avfilter::AVFilter::get_by_name(filter)
        .ok_or_else(|| MediaError::Ffmpeg(format!("no {} filter in this FFmpeg build", what())))?;
    let args = args
        .map(CString::new)
        .transpose()
        .map_err(|_| MediaError::BadPath)?;
    graph
        .create_filter_context(&found, name, args.as_deref())
        .map_err(|e| MediaError::Ffmpeg(format!("setting up {}: {e}", what())))
}

/// The arguments for a `buffer` source taking `w`×`h` frames of `pix_fmt`.
#[cfg(not(target_arch = "wasm32"))]
fn buffer_args(w: i32, h: i32, pix_fmt: i32, time_base: AVRational) -> String {
    format!(
        "video_size={w}x{h}:pix_fmt={pix_fmt}:time_base={}/{}:pixel_aspect=1/1",
        time_base.num, time_base.den
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn link(
    from: &mut rsmpeg::avfilter::AVFilterContextMut,
    from_pad: u32,
    to: &mut rsmpeg::avfilter::AVFilterContextMut,
    to_pad: u32,
) -> Result<(), MediaError> {
    from.link(from_pad, to, to_pad)
        .map_err(at("setting up the GIF palette"))
}

/// Hand a frame (or the end, for `None`) to the source named `source`, and
/// collect everything the sink lets out.
#[cfg(not(target_arch = "wasm32"))]
fn push(
    graph: &rsmpeg::avfilter::AVFilterGraph,
    source: &CStr,
    frame: Option<AVFrame>,
    out: &mut Vec<AVFrame>,
) -> Result<(), MediaError> {
    let lost = || MediaError::Ffmpeg("GIF palette filter lost".into());
    graph
        .get_filter(source)
        .ok_or_else(lost)?
        .buffersrc_add_frame(frame, None)
        .map_err(at("making the GIF palette"))?;
    let mut sink = graph.get_filter(c"out").ok_or_else(lost)?;
    loop {
        match sink.buffersink_get_frame(None) {
            Ok(frame) => out.push(frame),
            Err(rsmpeg::error::RsmpegError::BufferSinkDrainError)
            | Err(rsmpeg::error::RsmpegError::BufferSinkEofError) => return Ok(()),
            Err(e) => return Err(at("making the GIF palette")(e)),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl GifEncoder {
    /// Open `path`, sized `width`×`height`, at `fps_num/fps_den`. Frames are
    /// eight-bit RGBA.
    pub fn open(
        path: &Path,
        width: u32,
        height: u32,
        fps_num: i32,
        fps_den: i32,
        options: GifOptions,
    ) -> Result<Self, MediaError> {
        quieten_palettegen();

        let cpath = CString::new(path.to_str().ok_or(MediaError::BadPath)?)
            .map_err(|_| MediaError::BadPath)?;
        let w =
            i32::try_from(width).map_err(|_| MediaError::Ffmpeg("frame width overflows".into()))?;
        let h = i32::try_from(height)
            .map_err(|_| MediaError::Ffmpeg("frame height overflows".into()))?;
        let time_base = AVRational {
            num: fps_den,
            den: fps_num,
        };

        let graph = rsmpeg::avfilter::AVFilterGraph::new();
        {
            let source = buffer_args(w, h, ffi::AV_PIX_FMT_RGBA, time_base);
            let mut src = add_filter(&graph, c"buffer", c"in", Some(&source))?;
            let mut sink = add_filter(&graph, c"buffersink", c"out", None)?;
            if options.palette_per_frame {
                let mut split = add_filter(&graph, c"split", c"split", None)?;
                let mut gen = add_filter(
                    &graph,
                    c"palettegen",
                    c"palettegen",
                    Some("stats_mode=single"),
                )?;
                let using = format!("new=1:dither={}", options.dither.filter_value());
                let mut using = add_filter(&graph, c"paletteuse", c"use", Some(&using))?;
                link(&mut src, 0, &mut split, 0)?;
                link(&mut split, 0, &mut using, 0)?;
                link(&mut split, 1, &mut gen, 0)?;
                link(&mut gen, 0, &mut using, 1)?;
                link(&mut using, 0, &mut sink, 0)?;
            } else {
                let mut gen = add_filter(
                    &graph,
                    c"palettegen",
                    c"palettegen",
                    Some("stats_mode=full"),
                )?;
                link(&mut src, 0, &mut gen, 0)?;
                link(&mut gen, 0, &mut sink, 0)?;
            }
        }
        graph.config().map_err(at("setting up the GIF palette"))?;
        let spool = if options.palette_per_frame {
            None
        } else {
            let file = tempfile::tempfile().map_err(|e| {
                MediaError::Ffmpeg(format!("making room for the GIF's frames: {e}"))
            })?;
            Some(std::io::BufWriter::new(file))
        };

        let mut output = AVFormatContextOutput::create(&cpath).map_err(at("opening the file"))?;
        let codec = AVCodec::find_encoder(ffi::AV_CODEC_ID_GIF)
            .ok_or_else(|| MediaError::Ffmpeg("no GIF encoder in this FFmpeg build".into()))?;
        let mut ctx = AVCodecContext::new(&codec);
        ctx.set_width(w);
        ctx.set_height(h);
        ctx.set_time_base(time_base);
        ctx.set_pix_fmt(ffi::AV_PIX_FMT_PAL8);
        ctx.open(None).map_err(at("opening the GIF encoder"))?;
        {
            let mut stream = output.new_stream();
            stream.set_codecpar(ctx.extract_codecpar());
            stream.set_time_base(time_base);
        }
        // The muxer counts the repeats after the first play: 0 is forever and
        // -1 is no repeat at all.
        let repeats = match options.plays {
            None => 0,
            Some(n) if n <= 1 => -1,
            Some(n) => i64::from(n) - 1,
        };
        output
            .write_header(&mut dict_set(None, "loop", &repeats.to_string()))
            .map_err(at("writing the header"))?;

        Ok(Self {
            output,
            video: ctx,
            graph,
            spool,
            dither: options.dither,
            width: w,
            height: h,
            time_base,
            next_pts: 0,
            finished: false,
        })
    }

    /// Encode one tightly-packed eight-bit RGBA frame.
    pub fn write_rgba(&mut self, rgba: &[u8]) -> Result<(), MediaError> {
        let expect = frame_len(self.width, self.height, 4)?;
        if rgba.len() != expect {
            return Err(MediaError::Ffmpeg(format!(
                "frame size {} != expected {expect}",
                rgba.len()
            )));
        }
        if let Some(spool) = &mut self.spool {
            use std::io::Write;
            spool
                .write_all(rgba)
                .map_err(|e| MediaError::Ffmpeg(format!("keeping a GIF frame: {e}")))?;
        }
        let frame = self.rgba_frame(rgba)?;
        let mut out = Vec::new();
        push(&self.graph, c"in", Some(frame), &mut out)?;
        // With one palette nothing comes out until the end.
        for frame in out {
            self.encode(Some(&frame))?;
        }
        Ok(())
    }

    fn rgba_frame(&mut self, rgba: &[u8]) -> Result<AVFrame, MediaError> {
        let mut frame = AVFrame::new();
        frame.set_width(self.width);
        frame.set_height(self.height);
        frame.set_format(ffi::AV_PIX_FMT_RGBA);
        frame
            .alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        copy_pixels_into(&mut frame, rgba, self.width, self.height, 4, false)?;
        frame.set_pts(self.next_pts);
        self.next_pts += 1;
        Ok(frame)
    }

    fn encode(&mut self, frame: Option<&AVFrame>) -> Result<(), MediaError> {
        self.video.send_frame(frame)?;
        drain_packets(&mut self.video, &mut self.output, 0, frame.is_none())
    }

    /// Flush the filters and the encoder and close the file. Idempotent.
    pub fn finish(&mut self) -> Result<(), MediaError> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        let mut out = Vec::new();
        push(&self.graph, c"in", None, &mut out)?;
        match self.spool.take() {
            None => {
                for frame in out {
                    self.encode(Some(&frame))?;
                }
            }
            // No frames, no palette, and nothing to write.
            Some(spool) => {
                if let Some(palette) = out.pop() {
                    self.write_spooled(spool, &palette)?;
                }
            }
        }
        self.encode(None)?;
        self.output
            .write_trailer()
            .map_err(at("finishing the file"))?;
        Ok(())
    }

    /// Dither every kept frame onto the one palette and encode it.
    fn write_spooled(
        &mut self,
        spool: std::io::BufWriter<std::fs::File>,
        palette: &AVFrame,
    ) -> Result<(), MediaError> {
        use std::io::{Read, Seek};
        let lost = |e: std::io::Error| MediaError::Ffmpeg(format!("reading a GIF frame back: {e}"));
        let mut file = spool.into_inner().map_err(|e| lost(e.into_error()))?;
        file.rewind().map_err(lost)?;
        let mut file = std::io::BufReader::new(file);

        let graph = rsmpeg::avfilter::AVFilterGraph::new();
        {
            let source = buffer_args(
                self.width,
                self.height,
                ffi::AV_PIX_FMT_RGBA,
                self.time_base,
            );
            // Stated as the palette states itself, or the source warns.
            let pal = format!(
                "{}:colorspace={}:range={}",
                buffer_args(
                    palette.width,
                    palette.height,
                    palette.format,
                    self.time_base
                ),
                palette.colorspace,
                palette.color_range
            );
            let mut src = add_filter(&graph, c"buffer", c"in", Some(&source))?;
            let mut pal = add_filter(&graph, c"buffer", c"pal", Some(&pal))?;
            let using = format!("dither={}", self.dither.filter_value());
            let mut using = add_filter(&graph, c"paletteuse", c"use", Some(&using))?;
            let mut sink = add_filter(&graph, c"buffersink", c"out", None)?;
            link(&mut src, 0, &mut using, 0)?;
            link(&mut pal, 0, &mut using, 1)?;
            link(&mut using, 0, &mut sink, 0)?;
        }
        graph.config().map_err(at("setting up the GIF palette"))?;
        // The palette stands for every frame after it.
        let mut out = Vec::new();
        let mut palette = palette.clone();
        palette.set_pts(0);
        push(&graph, c"pal", Some(palette), &mut out)?;
        push(&graph, c"pal", None, &mut out)?;

        let frames = std::mem::replace(&mut self.next_pts, 0);
        let mut rgba = vec![0u8; frame_len(self.width, self.height, 4)?];
        for _ in 0..frames {
            file.read_exact(&mut rgba).map_err(lost)?;
            let frame = self.rgba_frame(&rgba)?;
            push(&graph, c"in", Some(frame), &mut out)?;
            for frame in out.drain(..) {
                self.encode(Some(&frame))?;
            }
        }
        push(&graph, c"in", None, &mut out)?;
        for frame in out {
            self.encode(Some(&frame))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A GIF opens with every frame, keeps its colour and its clear pixels,
    /// and a frame does not show the one before it through its clear part.
    /// Both palette kinds, as they take different roads to the file.
    #[test]
    fn a_gif_keeps_its_frames_colour_and_clear_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let (w, h) = (32usize, 16usize);
        for palette_per_frame in [false, true] {
            let path = dir.path().join(format!("loop-{palette_per_frame}.gif"));
            let options = GifOptions {
                palette_per_frame,
                ..GifOptions::default()
            };
            let mut enc = GifEncoder::open(&path, w as u32, h as u32, 10, 1, options).unwrap();
            // A red square over nothing, moving right eight pixels a frame.
            for f in 0..3 {
                let mut rgba = vec![0u8; w * h * 4];
                for y in 4..12 {
                    for x in f * 8..f * 8 + 8 {
                        rgba[(y * w + x) * 4..][..4].copy_from_slice(&[220, 30, 30, 255]);
                    }
                }
                enc.write_rgba(&rgba).unwrap();
            }
            enc.finish().unwrap();

            let index = crate::index::build_frame_index(&path).unwrap();
            let mut dec = crate::decode::VideoDecoder::open(&path, index).unwrap();
            assert_eq!(dec.frame_count(), 3);
            let frame = dec.frame_rgba(1, None).unwrap();
            let px = |x: usize, y: usize| &frame.rgba[(y * w + x) * 4..][..4];
            let red = px(12, 8);
            assert!(
                red[3] == 255 && red[0] > 200 && red[1] < 60 && red[2] < 60,
                "square is {red:?}"
            );
            assert_eq!(px(4, 8)[3], 0, "the first frame's square shows through");
            assert_eq!(px(28, 1)[3], 0, "the background is not clear");
        }
    }
}
