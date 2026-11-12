//! Hardware decode: the device a decoder is given, and the frames it hands
//! back brought to the layout software decoding gives. Runs on the decode
//! thread that owns the decoder.

use crate::MediaError;
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;

/// Attach a D3D11VA hardware device to `ctx` when this codec supports
/// device-context hardware decode (docs/impl/media-io.md §4, v1 baseline).
/// libav's default format negotiation then selects the hardware path on its
/// own. False — leaving the context untouched for software — when there is no
/// support or no device; never an error.
#[cfg(windows)]
pub(super) fn attach_d3d11va(codec: &AVCodec, ctx: &mut AVCodecContext) -> bool {
    let supported = (0..).map_while(|i| codec.hw_config(i)).any(|c| {
        c.device_type == ffi::AV_HWDEVICE_TYPE_D3D11VA
            && (c.methods & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX) != 0
    });
    if !supported {
        return false;
    }
    match rsmpeg::avutil::AVHWDeviceContext::create(ffi::AV_HWDEVICE_TYPE_D3D11VA, None, None, 0) {
        Ok(device) => {
            ctx.set_hw_device_ctx(device);
            true
        }
        Err(_) => false,
    }
}

/// The macOS sibling is VideoToolbox and lands with the macOS pass;
/// until then every other platform decodes in software, on all cores.
#[cfg(not(windows))]
pub(super) fn attach_d3d11va(_codec: &AVCodec, _ctx: &mut AVCodecContext) -> bool {
    false
}

/// Repack a hardware-transferred NV12 frame as planar yuv420p. Same-size
/// point "scale" through swscale is a lossless plane copy plus chroma
/// deinterleave — no values change, only the memory layout, so the shared
/// RGBA conversion behaves identically to the software decoder's output.
pub(super) fn deinterleave_to_yuv420p(src: &AVFrame) -> Result<AVFrame, MediaError> {
    let mut sws = SwsContext::get_context(
        src.width,
        src.height,
        src.format,
        src.width,
        src.height,
        ffi::AV_PIX_FMT_YUV420P,
        ffi::SWS_POINT,
        None,
        None,
        None,
    )
    .ok_or_else(|| MediaError::Ffmpeg("nv12 repack context creation failed".into()))?;
    let mut out = AVFrame::new();
    out.set_width(src.width);
    out.set_height(src.height);
    out.set_format(ffi::AV_PIX_FMT_YUV420P);
    out.alloc_buffer()
        .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    sws.scale_frame(src, 0, src.height, &mut out)?;
    // Keep what the file says about its colour, which the RGBA tap reads.
    // SAFETY: both frames are valid and ours; this copies plain fields and
    // side data, never the planes.
    #[allow(unsafe_code)]
    unsafe {
        ffi::av_frame_copy_props(out.as_mut_ptr(), src.as_ptr());
    }
    Ok(out)
}
