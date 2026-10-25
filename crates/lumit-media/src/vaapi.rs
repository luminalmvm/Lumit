//! VAAPI, which is how AMD and Intel cards encode on Linux. Unlike the other
//! hardware encoders it only takes frames that are already on the card, so
//! each frame is copied over after it is converted to NV12.

use crate::MediaError;
use rsmpeg::avcodec::AVCodecContext;
use rsmpeg::avutil::{AVFrame, AVHWDeviceContext};
use rsmpeg::ffi;

/// Give a VAAPI encoder a pool of NV12 frames on the system's first card.
pub(crate) fn attach(ctx: &mut AVCodecContext, width: i32, height: i32) -> Result<(), MediaError> {
    let device = AVHWDeviceContext::create(ffi::AV_HWDEVICE_TYPE_VAAPI, None, None, 0)
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
