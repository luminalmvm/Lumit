//! Hardware decode: the device a decoder is given, the frames it hands back
//! brought to the layout software decoding gives, and the step back to
//! software when either goes wrong. Runs on the decode thread that owns the
//! decoder.

use super::{open_codec_ctx, VideoDecoder};
use crate::MediaError;
use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVCodecParameters};
use rsmpeg::avutil::{AVFrame, AVHWDeviceContext};
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;
use std::sync::{Mutex, PoisonError};

/// Which device a decoder asks for.
#[derive(Clone, Copy)]
pub(super) enum HwKind {
    /// The platform's own: D3D11VA on Windows, CUDA or VA-API on Linux.
    Platform,
    /// CUDA on its own, so the shared path can be tried on a Windows machine.
    #[cfg(test)]
    Cuda,
}

/// Give `ctx` a hardware device when one suits this stream. libav's own
/// format negotiation then picks the hardware path, and falls back to
/// software by itself when the device cannot take the stream. Nothing
/// suitable leaves the context for software; never an error.
pub(super) fn attach(
    codec: &AVCodec,
    par: &AVCodecParameters,
    ctx: &mut AVCodecContext,
    kind: HwKind,
) {
    let fits = |device| suits(codec, par, device);
    let device = match kind {
        HwKind::Platform => platform_device(fits),
        #[cfg(test)]
        HwKind::Cuda => {
            let cuda = ffi::AV_HWDEVICE_TYPE_CUDA;
            fits(cuda)
                .then(|| AVHWDeviceContext::create(cuda, None, None, 0).ok())
                .flatten()
        }
    };
    if let Some(device) = device {
        ctx.set_hw_device_ctx(device);
    }
}

/// A D3D11VA device of its own for each decoder.
#[cfg(windows)]
fn platform_device(fits: impl Fn(ffi::AVHWDeviceType) -> bool) -> Option<AVHWDeviceContext> {
    let d3d11 = ffi::AV_HWDEVICE_TYPE_D3D11VA;
    fits(d3d11)
        .then(|| AVHWDeviceContext::create(d3d11, None, None, 0).ok())
        .flatten()
}

/// One device every decoder shares, found the first time one asks.
#[cfg(target_os = "linux")]
fn platform_device(fits: impl Fn(ffi::AVHWDeviceType) -> bool) -> Option<AVHWDeviceContext> {
    // Found outside the cell, so no lock is held while a driver loads. Two
    // decoders opening at once may each look once.
    let found = match SHARED.get() {
        Some(found) => found,
        None => {
            let _ = SHARED.set(find_device());
            SHARED.get()?
        }
    };
    let Shared(device, kind) = found.as_ref()?;
    // A new reference for the decoder, which frees it when it closes.
    fits(*kind).then(|| device.clone())
}

/// The macOS sibling is VideoToolbox and lands with the macOS pass; until
/// then every other platform decodes in software, on all cores.
#[cfg(not(any(windows, target_os = "linux")))]
fn platform_device(_fits: impl Fn(ffi::AVHWDeviceType) -> bool) -> Option<AVHWDeviceContext> {
    None
}

/// The device found, or `None` for none, kept for the rest of the run.
#[cfg(target_os = "linux")]
static SHARED: std::sync::OnceLock<Option<Shared>> = std::sync::OnceLock::new();

#[cfg(target_os = "linux")]
struct Shared(AVHWDeviceContext, ffi::AVHWDeviceType);

// SAFETY: the device is only reached through `&`, and the one call made
// through it is `av_buffer_ref`, whose count is atomic. libav's CUDA and
// VA-API devices are made to be shared by decoders on many threads.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
unsafe impl Sync for Shared {}

/// CUDA, or else VA-API on the first render node that opens. Asked only once
/// hardware decode is on, so with it off no driver is touched. Each library
/// is opened by hand first: FFmpeg's Linux build reaches libva and libdrm
/// through stubs that end the process when one is missing, and the video
/// half of the NVIDIA driver is often a package of its own.
#[cfg(target_os = "linux")]
fn find_device() -> Option<Shared> {
    let mut why = Vec::new();
    let cuda = ffi::AV_HWDEVICE_TYPE_CUDA;
    say("hardware decode: trying cuda");
    match crate::vaapi::loads(&["libcuda.so.1", "libnvcuvid.so.1"]) {
        Ok(()) => match AVHWDeviceContext::create(cuda, None, None, 0) {
            Ok(device) => {
                say("hardware decode: cuda");
                return Some(Shared(device, cuda));
            }
            Err(e) => why.push(format!("cuda: {e}")),
        },
        Err(e) => why.push(e),
    }
    let vaapi = ffi::AV_HWDEVICE_TYPE_VAAPI;
    say("hardware decode: trying vaapi");
    match crate::vaapi::libva_loads() {
        Ok(()) => {
            // Only the render node, so FFmpeg never falls back to libva's X11 half.
            let drm = rsmpeg::avutil::AVDictionary::new(c"connection_type", c"drm", 0);
            let found = (128..192).find_map(|n| {
                let node = format!("/dev/dri/renderD{n}");
                if !std::path::Path::new(&node).exists() {
                    return None;
                }
                let path = std::ffi::CString::new(node.clone()).ok()?;
                let device = AVHWDeviceContext::create(vaapi, Some(&path), Some(&drm), 0).ok()?;
                Some((Shared(device, vaapi), node))
            });
            if let Some((shared, node)) = found {
                say(&format!("hardware decode: vaapi ({node})"));
                return Some(shared);
            }
            why.push("vaapi: no render node opened".into());
        }
        Err(e) => why.push(e.to_string()),
    }
    say(&format!("hardware decode: none ({})", why.join("; ")));
    None
}

/// One line to whoever [`super::report_hardware_to`] named.
fn say(line: &str) {
    if let Some(sink) = super::HARDWARE_REPORT.get() {
        sink(line);
    }
}

/// [`say`], once for each line. Capped, since an error can name a frame.
fn say_once(line: String) {
    static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let new = {
        let mut said = SAID.lock().unwrap_or_else(PoisonError::into_inner);
        let new = said.len() < 32 && !said.contains(&line);
        if new {
            said.push(line.clone());
        }
        new
    };
    if new {
        say(&line);
    }
}

/// A kind of stream by name, as "cuda h264 yuv422p10le".
fn kind_name((device, codec, format): Kind) -> String {
    let device = rsmpeg::avutil::hwdevice_get_type_name(device).map(|n| n.to_string_lossy());
    let codec = AVCodec::find_decoder(codec).map(|c| c.name().to_string_lossy().into_owned());
    let format = rsmpeg::avutil::get_pix_fmt_name(format).map(|n| n.to_string_lossy());
    format!(
        "{} {} {}",
        device.unwrap_or_default(),
        codec.unwrap_or_default(),
        format.unwrap_or_default()
    )
}

/// Whether a device of this type is offered the stream: a layout it can hand
/// back exactly as software would, a codec it decodes, and no check this run
/// that found it wrong for this kind of stream.
fn suits(codec: &AVCodec, par: &AVCodecParameters, device: ffi::AVHWDeviceType) -> bool {
    // The enum is an `i32` where C enums are signed and a `u32` where they
    // are not, so the cast is needed on some targets and flagged on the rest.
    #[allow(clippy::unnecessary_cast)]
    let by_device = ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32;
    allowed(device, par.format)
        && verdict((device, par.codec_id, par.format)) != Some(false)
        && (0..)
            .map_while(|i| codec.hw_config(i))
            .any(|c| c.device_type == device && c.methods & by_device != 0)
}

/// The software layouts a device may decode. Only CUDA takes 4:2:2 here:
/// D3D11VA and VA-API hand 4:2:2 back packed, which the repack refuses.
fn allowed(device: ffi::AVHWDeviceType, format: ffi::AVPixelFormat) -> bool {
    use ffi::{
        AV_PIX_FMT_YUV420P, AV_PIX_FMT_YUV420P10LE, AV_PIX_FMT_YUV422P, AV_PIX_FMT_YUV422P10LE,
        AV_PIX_FMT_YUVJ420P, AV_PIX_FMT_YUVJ422P,
    };
    let four_two_two = [
        AV_PIX_FMT_YUV422P,
        AV_PIX_FMT_YUVJ422P,
        AV_PIX_FMT_YUV422P10LE,
    ];
    [
        AV_PIX_FMT_YUV420P,
        AV_PIX_FMT_YUVJ420P,
        AV_PIX_FMT_YUV420P10LE,
    ]
    .contains(&format)
        || (device == ffi::AV_HWDEVICE_TYPE_CUDA && four_two_two.contains(&format))
}

/// A device, a codec and the layout software decoding gives.
type Kind = (ffi::AVHWDeviceType, ffi::AVCodecID, ffi::AVPixelFormat);

/// What the check against software found for each kind of stream, for the
/// rest of the run. A handful of entries: one a device, codec and layout.
static VERDICTS: Mutex<Vec<(Kind, bool)>> = Mutex::new(Vec::new());

fn verdict(kind: Kind) -> Option<bool> {
    let list = VERDICTS.lock().unwrap_or_else(PoisonError::into_inner);
    list.iter().find(|(k, _)| *k == kind).map(|(_, good)| *good)
}

fn judge(kind: Kind, good: bool) {
    let mut list = VERDICTS.lock().unwrap_or_else(PoisonError::into_inner);
    match list.iter_mut().find(|(k, _)| *k == kind) {
        Some(entry) => entry.1 = good,
        None => list.push((kind, good)),
    }
}

/// The layout a hardware frame comes back from the card in.
fn surface_format(frame: &AVFrame) -> Option<ffi::AVPixelFormat> {
    // SAFETY: a hardware frame's frames context is an AVBufferRef whose data
    // is the AVHWFramesContext libav made with it. Both live as long as the
    // frame, which is borrowed here, and only the format is read.
    #[allow(unsafe_code)]
    unsafe {
        frame
            .hw_frames_ctx
            .as_ref()?
            .data
            .cast::<ffi::AVHWFramesContext>()
            .as_ref()
            .map(|c| c.sw_format)
    }
}

/// The type of the device `ctx` decodes on, if it has one.
fn device_type(ctx: &AVCodecContext) -> Option<ffi::AVHWDeviceType> {
    let device = ctx.hw_device_ctx;
    if device.is_null() {
        return None;
    }
    // SAFETY: a device the context holds is an AVBufferRef whose data is the
    // AVHWDeviceContext libav made with it. Both live as long as the context,
    // which is borrowed here, and only the type is read.
    #[allow(unsafe_code)]
    unsafe {
        (*device)
            .data
            .cast::<ffi::AVHWDeviceContext>()
            .as_ref()
            .map(|d| d.type_)
    }
}

impl VideoDecoder {
    /// A decoded frame as it is handed out. One the graphics card decoded is
    /// copied back and repacked into the layout software decoding gives, and
    /// the first of its kind is checked against software.
    pub(super) fn handed_out(&mut self, n: usize, frame: AVFrame) -> Result<AVFrame, MediaError> {
        self.hardware = !frame.hw_frames_ctx.is_null();
        if !self.hardware {
            return Ok(frame);
        }
        // A copy nobody holds any more is filled again: a fresh 4K frame's
        // allocation and first touch were half the transfer's time.
        let format = surface_format(&frame);
        let free = self.spare.iter().position(|f| {
            (f.width, f.height, Some(f.format)) == (frame.width, frame.height, format)
                && f.is_writable().unwrap_or(false)
        });
        let mut sw = free.map_or_else(AVFrame::new, |i| self.spare.swap_remove(i));
        sw.hwframe_transfer_data(&frame)
            .map_err(|e| MediaError::Ffmpeg(format!("hw frame transfer: {e}")))?;
        // The transfer copies the planes only. What the file says about its
        // colour, which the RGBA tap reads, comes from the decoded frame.
        // SAFETY: both frames are valid and ours. The reused frame's side
        // data and metadata are freed first, so they never pile up, and the
        // copy takes plain fields and side data, never the planes.
        #[allow(unsafe_code)]
        unsafe {
            let to = sw.as_mut_ptr();
            ffi::av_frame_side_data_free(&mut (*to).side_data, &mut (*to).nb_side_data);
            ffi::av_dict_free(&mut (*to).metadata);
            ffi::av_frame_copy_props(to, frame.as_ptr());
        }
        let frame = to_software_layout(&sw, self.decoder.sw_pix_fmt)?;
        // Two kept: the last frame handed out is still held by `last`.
        self.spare.push(sw);
        if self.spare.len() > 2 {
            self.spare.remove(0);
        }
        self.check_once(n, &frame)?;
        Ok(frame)
    }

    /// The first frame of each kind a device other than D3D11VA hands out is
    /// decoded again in software and compared, at a keyframe so the check
    /// costs one frame. A device that disagrees is not offered that kind of
    /// stream again this run, and a decoder already open on it steps back to
    /// software on its next frame. D3D11VA is left unchecked, as it always
    /// was.
    fn check_once(&self, n: usize, frame: &AVFrame) -> Result<(), MediaError> {
        let Some(device) = device_type(&self.decoder) else {
            return Ok(());
        };
        let kind = (device, self.decoder.codec_id, self.decoder.sw_pix_fmt);
        if device == ffi::AV_HWDEVICE_TYPE_D3D11VA {
            return Ok(());
        }
        match verdict(kind) {
            Some(true) => return Ok(()),
            Some(false) => {
                return Err(MediaError::Ffmpeg(
                    "hardware frame differs from software".into(),
                ))
            }
            None => {}
        }
        if self.index.nearest_keyframe_at_or_before(n) != n {
            return Ok(());
        }
        // Software that cannot read the frame either leaves nothing to judge.
        let Ok(mut software) = Self::open_with(self.src.clone(), self.index.clone(), false) else {
            return Ok(());
        };
        let Ok(theirs) = software.decode_exact(n) else {
            return Ok(());
        };
        // A ten-bit frame kept semi-planar is compared in software's layout.
        let planar = (frame.format != theirs.format)
            .then(|| repack(frame, theirs.format).ok())
            .flatten();
        let agrees = same_samples(planar.as_ref().unwrap_or(frame), &theirs);
        judge(kind, agrees);
        let said = if agrees {
            "agrees"
        } else {
            "disagrees, using software"
        };
        say(&format!("hardware decode: {} {said}", kind_name(kind)));
        if agrees {
            Ok(())
        } else {
            Err(MediaError::Ffmpeg(
                "hardware frame differs from software".into(),
            ))
        }
    }

    /// Frame `n` again in software, after `err` on the graphics card. The
    /// decoder stays in software from here on, and a kind of stream that
    /// failed on the card but reads in software is not offered to it again.
    pub(super) fn again_in_software(
        &mut self,
        n: usize,
        err: MediaError,
    ) -> Result<AVFrame, MediaError> {
        // A frame the index does not have is not the card's doing.
        if self.index.pts_of_frame(n).is_none() {
            return Err(err);
        }
        let device = device_type(&self.decoder);
        let kind = (self.decoder.codec_id, self.decoder.sw_pix_fmt);
        self.drop_to_software()?;
        say_once(format!("hardware decode: dropped to software: {err}"));
        let frame = self.decode_exact_once(n)?;
        if let Some(device) = device.filter(|d| *d != ffi::AV_HWDEVICE_TYPE_D3D11VA) {
            let kind = (device, kind.0, kind.1);
            if verdict(kind) != Some(false) {
                judge(kind, false);
            }
        }
        Ok(frame)
    }

    /// Rebuild the decoder in software. The next frame seeks, so it starts
    /// from a keyframe with nothing of the card's left in it.
    fn drop_to_software(&mut self) -> Result<(), MediaError> {
        let par = self
            .input
            .streams()
            .iter()
            .find(|s| s.index == self.stream_index)
            .map(|s| s.codecpar().clone())
            .ok_or(MediaError::NoStreams)?;
        let codec = AVCodec::find_decoder(par.codec_id)
            .ok_or_else(|| MediaError::Ffmpeg("no decoder for codec".into()))?;
        self.decoder = open_codec_ctx(&codec, &par, None)?;
        self.next_sequential = None;
        self.last = None;
        self.spare.clear();
        self.hardware = false;
        Ok(())
    }
}

/// A frame copied back from the graphics card, holding exactly the samples
/// software decoding gives for this stream, `sw_pix_fmt`, so the shared RGBA
/// conversion treats both paths alike. Eight-bit frames are repacked into
/// software's planar layout: swscale's NV12 and yuv420p conversions
/// interpolate chroma differently (measured: 9% of bytes off, up to 161, on a
/// test pattern's edges). Ten-bit P010 and P210 are kept as they are: their
/// RGBA at every size, their luma and their fields measured byte for byte the
/// same as the planar layout's, and the repack cost more than the transfer.
/// Anything else is refused: a 4:2:0 surface for a 4:2:2 stream, eight bits
/// for ten, sixteen, or a packed layout.
pub(super) fn to_software_layout(
    sw: &AVFrame,
    sw_pix_fmt: ffi::AVPixelFormat,
) -> Result<AVFrame, MediaError> {
    // The J layouts are the same samples, labelled full range.
    let plain = match sw_pix_fmt {
        ffi::AV_PIX_FMT_YUVJ420P => ffi::AV_PIX_FMT_YUV420P,
        ffi::AV_PIX_FMT_YUVJ422P => ffi::AV_PIX_FMT_YUV422P,
        other => other,
    };
    let planar = match sw.format {
        ffi::AV_PIX_FMT_NV12 => ffi::AV_PIX_FMT_YUV420P,
        ffi::AV_PIX_FMT_NV16 => ffi::AV_PIX_FMT_YUV422P,
        ffi::AV_PIX_FMT_P010LE => ffi::AV_PIX_FMT_YUV420P10LE,
        ffi::AV_PIX_FMT_P210LE => ffi::AV_PIX_FMT_YUV422P10LE,
        other => other,
    };
    if planar != plain {
        let name = |f| rsmpeg::avutil::get_pix_fmt_name(f).map(|n| n.to_string_lossy());
        return Err(MediaError::Ffmpeg(format!(
            "hardware frame is {:?}, software gives {:?}",
            name(sw.format),
            name(sw_pix_fmt)
        )));
    }
    let mut out = match sw.format {
        ffi::AV_PIX_FMT_P010LE | ffi::AV_PIX_FMT_P210LE => return Ok(sw.clone()),
        f if f == plain => sw.clone(),
        _ => repack(sw, plain)?,
    };
    out.set_format(sw_pix_fmt);
    Ok(out)
}

/// `src` in the planar `format`, by a same-size point "scale": a plane copy
/// plus the chroma deinterleave, with no value changed.
fn repack(src: &AVFrame, format: ffi::AVPixelFormat) -> Result<AVFrame, MediaError> {
    let mut sws = SwsContext::get_context(
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
    .ok_or_else(|| MediaError::Ffmpeg("hardware frame repack context creation failed".into()))?;
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

/// Whether two frames hold the same samples, give or take one: the slack a
/// driver's rounding is allowed.
fn same_samples(a: &AVFrame, b: &AVFrame) -> bool {
    if (a.format, a.width, a.height) != (b.format, b.width, b.height) {
        return false;
    }
    let (Some(x), Some(y)) = (flat(a), flat(b)) else {
        return false;
    };
    let ten_bit = [ffi::AV_PIX_FMT_YUV420P10LE, ffi::AV_PIX_FMT_YUV422P10LE].contains(&a.format);
    if ten_bit {
        let sample = |s: &[u8]| <[u8; 2]>::try_from(s).map_or(0, u16::from_le_bytes);
        x.chunks_exact(2)
            .zip(y.chunks_exact(2))
            .all(|(p, q)| sample(p).abs_diff(sample(q)) <= 1)
    } else {
        x.iter().zip(&y).all(|(p, q)| p.abs_diff(*q) <= 1)
    }
}

/// A frame's planes end to end, with no row padding.
fn flat(frame: &AVFrame) -> Option<Vec<u8>> {
    let mut out = vec![0; frame.image_get_buffer_size(1).ok()?];
    frame.image_copy_to_buffer(&mut out, 1).ok()?;
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::index::build_frame_index;
    use crate::index::tests_support::fixture_as;

    /// Hardware decode is an implementation detail, never a look: the frames
    /// a device hands back must match the software decoder's. H.264 decoding
    /// is bit-exact by spec, so the two only differ if the transfer or the
    /// repack is wrong; a byte of slack per channel forgives a driver's
    /// chroma rounding without letting a real defect through. Tried on the
    /// platform's device and on CUDA, in eight-bit 4:2:0 and ten-bit 4:2:2.
    /// Skips where there is no hardware or no ffmpeg CLI, where the
    /// fallback is the software path.
    #[test]
    fn hardware_and_software_decode_agree_on_the_pixels() {
        for pix_fmt in ["yuv420p", "yuv422p10le"] {
            let dir = tempfile::tempdir().unwrap();
            let Some(file) = fixture_as(dir.path(), pix_fmt) else {
                eprintln!("skipping: no ffmpeg CLI available");
                return;
            };
            let index = build_frame_index(&file).unwrap();
            for kind in [HwKind::Platform, HwKind::Cuda] {
                let mut hw = VideoDecoder::open_on(&file, index.clone(), Some(kind)).unwrap();
                let mut sw = VideoDecoder::open_with(&file, index.clone(), false).unwrap();
                hw.frame_rgba(0, None).unwrap();
                if !hw.is_hardware() {
                    eprintln!("skipping {pix_fmt}: no hardware decoder for it here");
                    continue;
                }
                for n in [0usize, 7, 63, 119] {
                    let a = hw.frame_rgba(n, None).unwrap();
                    let b = sw.frame_rgba(n, None).unwrap();
                    assert!(
                        hw.is_hardware(),
                        "{pix_fmt} frame {n} fell back to software"
                    );
                    assert!(!sw.is_hardware());
                    assert_eq!((a.width, a.height), (b.width, b.height));
                    let worst = a
                        .rgba
                        .iter()
                        .zip(&b.rgba)
                        .map(|(x, y)| x.abs_diff(*y))
                        .max()
                        .unwrap_or(0);
                    assert!(
                        worst <= 1,
                        "{pix_fmt} frame {n}: hw and sw differ by {worst}"
                    );
                }
            }
        }
    }
}
