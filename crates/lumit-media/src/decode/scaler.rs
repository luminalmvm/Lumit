//! The scalers a decoder keeps from one frame to the next.

use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;

use super::{
    convert_linear_f32, copy_tight_rows, is_float, unsafe_data_slice, DecodedFrame, PixelFormat,
};
use crate::MediaError;

/// What a kept context was made for: the source's size and format, the size
/// asked for, and the matrix the file states. A frame that differs in any of
/// them gets a new context, made exactly as before.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Shape {
    src: (i32, i32, i32),
    dst: (i32, i32),
    colorspace: ffi::AVColorSpace,
}

impl Shape {
    fn of(frame: &AVFrame, dst_w: i32, dst_h: i32) -> Self {
        Self {
            src: (frame.width, frame.height, frame.format),
            dst: (dst_w, dst_h),
            colorspace: frame.colorspace,
        }
    }
}

/// A decoder's scalers, and the frame the RGBA one writes into, kept while
/// its frames keep the same shape. A new context and a fresh output buffer
/// every frame cost time and page faults before any pixel was made.
#[derive(Default)]
pub(super) struct Scalers {
    rgba: Option<(Shape, SwsContext)>,
    /// Let go of while the decoder sits idle, so only decoders in use hold one.
    out: Option<AVFrame>,
    repack: Option<(Shape, ffi::AVPixelFormat, SwsContext)>,
}

impl Scalers {
    /// A frame as RGBA at exactly this size, eight-bit or float as it arrived.
    pub(super) fn rgba(
        &mut self,
        frame: &AVFrame,
        dst_w: i32,
        dst_h: i32,
    ) -> Result<DecodedFrame, MediaError> {
        if is_float(frame.format) {
            return convert_linear_f32(frame, dst_w, dst_h);
        }
        let shape = Shape::of(frame, dst_w, dst_h);
        let kept = match self.rgba.take() {
            Some(kept) if kept.0 == shape => kept,
            _ => (shape, rgba_context(frame, dst_w, dst_h)?),
        };
        let (_, sws) = self.rgba.insert(kept);
        let out = match self.out.take() {
            Some(out) if (out.width, out.height) == (dst_w, dst_h) => out,
            _ => rgba_frame(dst_w, dst_h)?,
        };
        // The kept frame is written whole by every scale and only read
        // through `copy_tight_rows`, so nothing carries from one frame on.
        let out = self.out.insert(out);
        sws.scale_frame(frame, 0, frame.height, out)?;

        // Copy out of the AVFrame's padded rows into a tight RGBA buffer.
        let stride = usize::try_from(out.linesize[0]).unwrap_or(0);
        let width = u32::try_from(dst_w).unwrap_or(0);
        let height = u32::try_from(dst_h).unwrap_or(0);
        let row_bytes = (width as usize).saturating_mul(4);
        let height_usize = height as usize;
        // Checked (not saturating): if this would overflow, the buffer cannot
        // possibly be that large, so treat it as an error rather than reading
        // out of bounds with a silently-truncated length.
        let buf_len = stride
            .checked_mul(height_usize)
            .ok_or_else(|| MediaError::Ffmpeg("scaled frame buffer size overflow".into()))?;
        let data = unsafe_data_slice(out, buf_len)?;
        let rgba = copy_tight_rows(data, stride, row_bytes, height_usize)?;
        Ok(DecodedFrame {
            width,
            height,
            rgba,
            format: PixelFormat::Srgb8,
        })
    }

    /// Let go of the RGBA frame, made again when next wanted. The contexts
    /// are small and stay.
    pub(super) fn let_go_of_rgba(&mut self) {
        self.out = None;
    }

    /// `src` in the planar `format`, by a same-size point "scale": a plane
    /// copy plus the chroma deinterleave, with no value changed.
    ///
    /// The context is kept; the frame is not, because it is handed on and
    /// held as the decoder's last picture.
    pub(super) fn repack(
        &mut self,
        src: &AVFrame,
        format: ffi::AVPixelFormat,
    ) -> Result<AVFrame, MediaError> {
        let shape = Shape::of(src, src.width, src.height);
        let kept = match self.repack.take() {
            Some(kept) if (kept.0, kept.1) == (shape, format) => kept,
            _ => (shape, format, repack_context(src, format)?),
        };
        let (_, _, sws) = self.repack.insert(kept);
        let mut out = AVFrame::new();
        out.set_width(src.width);
        out.set_height(src.height);
        out.set_format(format);
        out.alloc_buffer()
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
        sws.scale_frame(src, 0, src.height, &mut out)?;
        // SAFETY: both frames are valid and ours; this copies plain fields and
        // side data, never the planes.
        #[allow(unsafe_code)]
        unsafe {
            ffi::av_frame_copy_props(out.as_mut_ptr(), src.as_ptr());
        }
        Ok(out)
    }
}

fn rgba_context(frame: &AVFrame, dst_w: i32, dst_h: i32) -> Result<SwsContext, MediaError> {
    let mut sws = SwsContext::get_context(
        frame.width,
        frame.height,
        frame.format,
        dst_w,
        dst_h,
        ffi::AV_PIX_FMT_RGBA,
        ffi::SWS_BILINEAR,
        None,
        None,
        None,
    )
    .ok_or_else(|| MediaError::Ffmpeg("swscale context creation failed".into()))?;
    crate::probe::use_yuv_matrix(&mut sws, frame.colorspace, frame.height, true);
    Ok(sws)
}

fn rgba_frame(dst_w: i32, dst_h: i32) -> Result<AVFrame, MediaError> {
    let mut out = AVFrame::new();
    out.set_width(dst_w);
    out.set_height(dst_h);
    out.set_format(ffi::AV_PIX_FMT_RGBA);
    out.alloc_buffer()
        .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    Ok(out)
}

fn repack_context(src: &AVFrame, format: ffi::AVPixelFormat) -> Result<SwsContext, MediaError> {
    SwsContext::get_context(
        src.width,
        src.height,
        src.format,
        src.width,
        src.height,
        format,
        ffi::SWS_POINT,
        None,
        None,
        None,
    )
    .ok_or_else(|| MediaError::Ffmpeg("hardware frame repack context creation failed".into()))
}
