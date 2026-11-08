//! The FFmpeg calls under the sound reader: the resampler, whether a jump
//! lands where it says, and the raw pointer touches, each one small.

use crate::MediaError;
use rsmpeg::avcodec::AVCodecContext;
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::{AVChannelLayout, AVFrame, AVSamples};
use rsmpeg::ffi;
use rsmpeg::swresample::SwrContext;

/// A resampler from what `decoder` hands out to interleaved stereo f32 at
/// `target_rate`, fresh: no history, and in phase with its first sample.
pub(super) fn resampler(
    decoder: &AVCodecContext,
    target_rate: u32,
) -> Result<SwrContext, MediaError> {
    let out_layout = AVChannelLayout::from_nb_channels(2);
    let mut swr = SwrContext::new(
        &out_layout,
        ffi::AV_SAMPLE_FMT_FLT,
        i32::try_from(target_rate).unwrap_or(48_000),
        &decoder.ch_layout,
        decoder.sample_fmt,
        decoder.sample_rate,
    )
    .map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    cap_downmix(&mut swr)?;
    swr.init().map_err(|e| MediaError::Ffmpeg(e.to_string()))?;
    Ok(swr)
}

/// Uncompressed sound, or FLAC: codecs whose every packet decodes to the same
/// samples wherever the decoder came from.
pub(super) fn is_lossless(par: &rsmpeg::avcodec::AVCodecParameters) -> bool {
    rsmpeg::avcodec::AVCodec::find_decoder(par.codec_id).is_some_and(|c| {
        let name = c.name().to_string_lossy();
        name.starts_with("pcm_") || name == "flac"
    })
}

/// Whether a jump in this file can be trusted to land where it says.
///
/// Two things have to hold. The container has to place every packet exactly
/// (QuickTime and MP4 keep a table of them, and uncompressed sound in a WAV or
/// AIFF is arithmetic). And the sound has to be one unbroken run, because
/// [`super::decode_all`] counts samples and a landing reads timestamps: a file with a
/// hole in its sound has the two disagree after the hole. The table is read
/// here to rule that out, which costs no file access since the demuxer already
/// holds it.
pub(super) fn lands_exactly(
    input: &AVFormatContextInput,
    stream: &rsmpeg::avformat::AVStreamRef<'_>,
    par: &rsmpeg::avcodec::AVCodecParameters,
) -> bool {
    let container = input.iformat().name().to_string_lossy().into_owned();
    let is = |name: &str| container.split(',').any(|n| n == name);
    let pcm = rsmpeg::avcodec::AVCodec::find_decoder(par.codec_id)
        .is_some_and(|c| c.name().to_string_lossy().starts_with("pcm_"));
    if is("wav") || is("aiff") || is("w64") {
        return pcm;
    }
    if is("flac") {
        return true;
    }
    if !is("mov") {
        return false;
    }
    let (num, den) = (
        i128::from(stream.time_base.num),
        i128::from(stream.time_base.den),
    );
    let rate = i128::from(par.sample_rate);
    let entries = index_entries(stream);
    if entries.is_empty() || num <= 0 || den <= 0 || rate <= 0 {
        return false;
    }
    // Bytes to one frame of uncompressed sound. QuickTime leaves the block
    // size unset, so it is worked out from the sample's own width.
    let pcm_frame = if par.block_align > 0 {
        i128::from(par.block_align)
    } else {
        i128::from(bits_per_sample(par.codec_id) / 8) * i128::from(par.ch_layout.nb_channels)
    };
    entries.windows(2).all(|pair| {
        let (at, size) = pair[0];
        // How many samples this packet holds: a fixed frame for a codec that
        // has one, and bytes over bytes-per-frame for uncompressed sound.
        let samples = if par.frame_size > 0 {
            i128::from(par.frame_size)
        } else if pcm && pcm_frame > 0 {
            i128::from(size) / pcm_frame
        } else {
            return false;
        };
        (i128::from(pair[1].0) - i128::from(at)) * num * rate == samples * den
    })
}

/// How many bits one sample of this codec takes, or nought for a codec with
/// no fixed width.
///
/// SAFETY: a plain lookup by codec id with no pointers either way.
#[allow(unsafe_code)]
fn bits_per_sample(codec: ffi::AVCodecID) -> i32 {
    unsafe { ffi::av_get_bits_per_sample(codec) }
}

/// The demuxer's table of this stream's packets: each one's timestamp and its
/// size in bytes. Empty for a container that keeps none.
///
/// SAFETY: `stream` is a live stream of an open input for the whole call, the
/// count comes from the same stream, and each entry pointer is read at once
/// and not kept: FFmpeg only promises it until the next call that takes the
/// stream. A null entry is skipped.
#[allow(unsafe_code)]
fn index_entries(stream: &rsmpeg::avformat::AVStreamRef<'_>) -> Vec<(i64, i32)> {
    let raw = stream.as_ptr();
    let count = unsafe { ffi::avformat_index_get_entries_count(raw) };
    (0..count.max(0))
        .filter_map(|i| {
            let entry = unsafe { ffi::avformat_index_get_entry(raw.cast_mut(), i) };
            if entry.is_null() {
                return None;
            }
            Some(unsafe { ((*entry).timestamp, (*entry).size()) })
        })
        .collect()
}

/// The resampler call needs raw pointers on both sides; isolated here so the
/// unsafety is one auditable function (engineering rules §unsafe policy).
///
/// SAFETY: `out.audio_data[0]` was just allocated by `AVSamples::new` for at
/// least `max_out` samples, and `swr_convert`/`swr.convert` only ever writes
/// up to `max_out` samples into it. `frame`'s `extended_data`, when present,
/// is owned by the decoder-produced `AVFrame` for the lifetime of this call
/// and is read-only from swresample's side. With a `skip`, each plane pointer
/// is moved on by that many samples: the caller passes `skip < nb_samples`
/// and `in_count = nb_samples - skip`, so the read stays inside the plane, and
/// a frame has one plane per channel when its format is planar and one
/// otherwise, which is how many pointers are read here.
#[allow(unsafe_code)]
pub(super) fn convert_samples(
    swr: &mut SwrContext,
    out: &mut AVSamples,
    max_out: i32,
    frame: Option<&AVFrame>,
    skip: usize,
    in_count: i32,
) -> Result<i32, MediaError> {
    let moved: Vec<*const u8> = match frame {
        Some(f) if skip > 0 => {
            let planar = unsafe { ffi::av_sample_fmt_is_planar(f.format) } != 0;
            let bytes =
                usize::try_from(unsafe { ffi::av_get_bytes_per_sample(f.format) }).unwrap_or(0);
            let channels = usize::try_from(f.ch_layout.nb_channels).unwrap_or(0);
            let (planes, stride) = if planar {
                (channels, bytes)
            } else {
                (1, bytes * channels)
            };
            if f.extended_data.is_null() || stride == 0 {
                return Err(MediaError::Ffmpeg("a decoded frame with no samples".into()));
            }
            (0..planes)
                .map(|p| unsafe { (*f.extended_data.add(p)).add(skip * stride) as *const u8 })
                .collect()
        }
        _ => Vec::new(),
    };
    let input = match frame {
        Some(_) if !moved.is_empty() => moved.as_ptr(),
        Some(f) => f.extended_data as *const *const u8,
        None => std::ptr::null(),
    };
    unsafe { swr.convert(&mut out.audio_data[0], max_out, input, in_count) }
        .map_err(|e| MediaError::Ffmpeg(e.to_string()))
}

/// The two raw-pointer touches in audio decode, kept small and auditable.
/// Returns an error rather than dereferencing a null plane pointer — a
/// defensive check against exotic sample formats/channel layouts where the
/// resampler could in principle leave a plane unset.
pub(super) fn plane_slice<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], MediaError> {
    if ptr.is_null() {
        return Err(MediaError::Ffmpeg(
            "resampler returned a null output buffer".into(),
        ));
    }
    // SAFETY: caller (`push`) only reaches here after `convert_samples`
    // reported `converted > 0` samples written into this same plane by
    // `AVSamples::new`'s allocation, and `len` is derived from that count,
    // so the read stays within the allocation.
    #[allow(unsafe_code)]
    unsafe {
        Ok(std::slice::from_raw_parts(ptr, len))
    }
}

/// Hold a fold-down to stereo at full scale.
///
/// **In plain terms.** A 5.1 or 7.1 recording has a centre and surrounds, and
/// folding it to two speakers adds those onto the left and right. Left alone
/// the sum runs well past full scale, by 7.7 dB for 5.1 and 9.9 dB for 7.1
/// with every channel busy, and the master then clips it: the clip plays loud
/// and distorted. FFmpeg scales the fold-down back by itself for whole-number
/// samples and not for the floats Lumit asks for, so it is asked to here.
/// A stereo source has nothing folded into it and is not changed.
///
/// Wrapped so the unsafety is one auditable function, as the two beside it
/// are.
#[allow(unsafe_code)]
fn cap_downmix(swr: &mut SwrContext) -> Result<(), MediaError> {
    // SAFETY: `swr` is a live `SwrContext`, whose first field is its
    // `AVClass`, which is all `av_opt_set_double` asks of the pointer. The
    // name is a NUL-terminated literal that outlives the call.
    unsafe {
        rsmpeg::avutil::opt_set_double(swr.as_mut_ptr().cast(), c"rematrix_maxval", 1.0, 0)
            .map_err(|e| MediaError::Ffmpeg(e.to_string()))
    }
}

pub(super) fn byte_to_f32(bytes: &[u8], out: &mut [f32]) {
    for (i, chunk) in bytes.chunks_exact(4).enumerate().take(out.len()) {
        out[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
}
