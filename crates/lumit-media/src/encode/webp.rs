//! Animated WebP through libwebp's own animation encoder.

#[cfg(not(target_arch = "wasm32"))]
use super::planes::{copy_pixels_into, frame_len};
#[cfg(not(target_arch = "wasm32"))]
use super::{at, dict_set, drain_packets, sliced_scaler};
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
use rsmpeg::swscale::SwsContext;
#[cfg(not(target_arch = "wasm32"))]
use std::ffi::CString;
use std::path::Path;

/// The browser build has no FFmpeg, so nothing is ever written this way there.
#[cfg(target_arch = "wasm32")]
pub struct WebpEncoder(std::convert::Infallible);

#[cfg(target_arch = "wasm32")]
impl WebpEncoder {
    pub fn open(
        _path: &Path,
        _width: u32,
        _height: u32,
        _fps_num: i32,
        _fps_den: i32,
        _lossless: bool,
    ) -> Result<Self, MediaError> {
        Err(crate::no_ffmpeg())
    }

    pub fn write_rgba(&mut self, _rgba: &[u8]) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn finish(&mut self) -> Result<(), MediaError> {
        match self.0 {}
    }
}

/// Writes an animated `.webp` with full alpha. Lossy by default, which is
/// far smaller than a GIF of the same frames; lossless keeps every pixel.
#[cfg(not(target_arch = "wasm32"))]
pub struct WebpEncoder {
    output: AVFormatContextOutput,
    video: AVCodecContext,
    sws: SwsContext,
    pix_fmt: i32,
    width: i32,
    height: i32,
    next_pts: i64,
    finished: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl WebpEncoder {
    /// Open `path`, sized `width`×`height`, at `fps_num/fps_den`. Frames are
    /// eight-bit RGBA with straight alpha.
    pub fn open(
        path: &Path,
        width: u32,
        height: u32,
        fps_num: i32,
        fps_den: i32,
        lossless: bool,
    ) -> Result<Self, MediaError> {
        // Named, because plain `libwebp` writes every frame whole.
        let codec = AVCodec::find_encoder_by_name(c"libwebp_anim").ok_or_else(|| {
            MediaError::Ffmpeg("no animated WebP encoder in this FFmpeg build".into())
        })?;
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
        // Lossy WebP is YUV 4:2:0 and lossless is RGB. Both keep alpha whole.
        let pix_fmt = if lossless {
            ffi::AV_PIX_FMT_BGRA
        } else {
            ffi::AV_PIX_FMT_YUVA420P
        };

        let mut output = AVFormatContextOutput::create(&cpath).map_err(at("opening the file"))?;
        let mut ctx = AVCodecContext::new(&codec);
        ctx.set_width(w);
        ctx.set_height(h);
        ctx.set_time_base(time_base);
        ctx.set_pix_fmt(pix_fmt);
        let options = dict_set(None, "lossless", if lossless { "1" } else { "0" });
        let options = dict_set(options, "quality", "80");
        let options = dict_set(options, "compression_level", "6");
        ctx.open(options).map_err(at("opening the WebP encoder"))?;
        {
            let mut stream = output.new_stream();
            stream.set_codecpar(ctx.extract_codecpar());
            stream.set_time_base(time_base);
        }
        // The muxer's own default plays it once. 0 plays it forever.
        output
            .write_header(&mut dict_set(None, "loop", "0"))
            .map_err(at("writing the header"))?;

        Ok(Self {
            output,
            video: ctx,
            sws: sliced_scaler(w, h, ffi::AV_PIX_FMT_RGBA, pix_fmt)?,
            pix_fmt,
            width: w,
            height: h,
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
        let mut src = AVFrame::new();
        src.set_width(self.width);
        src.set_height(self.height);
        src.set_format(ffi::AV_PIX_FMT_RGBA);
        src.alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        copy_pixels_into(&mut src, rgba, self.width, self.height, 4, false)?;

        let mut dst = AVFrame::new();
        dst.set_width(self.width);
        dst.set_height(self.height);
        dst.set_format(self.pix_fmt);
        dst.alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        // SAFETY: both frames were allocated above at the size and formats
        // the scaler was built for, and nothing else is using either.
        #[allow(unsafe_code)]
        let scaled =
            unsafe { ffi::sws_scale_frame(self.sws.as_mut_ptr(), dst.as_mut_ptr(), src.as_ptr()) };
        if scaled < 0 {
            return Err(MediaError::Ffmpeg(format!("swscale failed ({scaled})")));
        }
        dst.set_pts(self.next_pts);
        self.next_pts += 1;

        self.video.send_frame(Some(&dst))?;
        drain_packets(&mut self.video, &mut self.output, 0, false)
    }

    /// Flush the encoder and close the file. The whole animation comes out
    /// of the encoder here, as one packet. Idempotent.
    pub fn finish(&mut self) -> Result<(), MediaError> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.video.send_frame(None)?;
        drain_packets(&mut self.video, &mut self.output, 0, true)?;
        self.output
            .write_trailer()
            .map_err(at("finishing the file"))?;
        Ok(())
    }
}
