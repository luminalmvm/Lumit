//! VAAPI, which is how AMD and Intel cards encode on Linux. Unlike the other
//! hardware encoders it only takes frames that are already on the card, so
//! each frame is copied over after it is converted to NV12.

use crate::MediaError;
use rsmpeg::avcodec::AVCodecContext;
use rsmpeg::avutil::{AVDictionary, AVFrame, AVHWDeviceContext};
use rsmpeg::ffi;

/// Give a VAAPI encoder a pool of NV12 frames on the system's first card.
pub(crate) fn attach(ctx: &mut AVCodecContext, width: i32, height: i32) -> Result<(), MediaError> {
    libva_loads()?;
    // Only the render node, so FFmpeg never falls back to libva's X11 half.
    let drm_only =
        cfg!(target_os = "linux").then(|| AVDictionary::new(c"connection_type", c"drm", 0));
    let device = AVHWDeviceContext::create(ffi::AV_HWDEVICE_TYPE_VAAPI, None, drm_only.as_ref(), 0)
        .map_err(|e| MediaError::Ffmpeg(format!("no VAAPI device: {e}")))?;
    let mut frames = device.hwframe_ctx_alloc();
    let pool = frames.data();
    pool.format = ffi::AV_PIX_FMT_VAAPI;
    pool.sw_format = ffi::AV_PIX_FMT_NV12;
    pool.width = width;
    pool.height = height;
    // FFmpeg's own VAAPI encode example uses 20, which covers what the
    // encoder holds on to between frames.
    pool.initial_pool_size = 20;
    frames
        .init()
        .map_err(|e| MediaError::Ffmpeg(format!("VAAPI frame pool: {e}")))?;
    ctx.set_pix_fmt(ffi::AV_PIX_FMT_VAAPI);
    ctx.set_hw_frames_ctx(frames);
    Ok(())
}

/// The frame as the encoder wants it: copied onto the card for VAAPI, handed
/// straight back for everything else.
pub(crate) fn to_device(ctx: &mut AVCodecContext, frame: AVFrame) -> Result<AVFrame, MediaError> {
    let Some(mut frames) = ctx.hw_frames_ctx_mut() else {
        return Ok(frame);
    };
    let mut on_card = AVFrame::new();
    frames.get_buffer(&mut on_card)?;
    on_card.hwframe_transfer_data(&frame)?;
    Ok(on_card)
}

/// Whether libva is installed. The Linux FFmpeg build opens it through
/// stubs that abort the whole process when it is missing, rather than
/// failing the call, so this asks first.
#[cfg(target_os = "linux")]
fn libva_loads() -> Result<(), MediaError> {
    for name in ["libva.so.2", "libva-drm.so.2"] {
        // SAFETY: libva's load-time code only registers itself, and the
        // library is closed again straight away without calling into it.
        #[allow(unsafe_code)]
        let loaded = unsafe { libloading::Library::new(name) };
        loaded.map_err(|e| MediaError::Ffmpeg(format!("no VAAPI: {e}")))?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn libva_loads() -> Result<(), MediaError> {
    Ok(())
}
