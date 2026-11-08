//! Fields lifted out of a frame's planes, and two frames woven back into
//! one, before either is converted to RGBA.

use rsmpeg::avutil::AVFrame;
use rsmpeg::ffi;

/// How many rows each plane of a `height`-row picture in `format` has, and
/// how many bytes one of its rows is. `None` for a format whose planes are
/// not plain rows of samples: a palette, a bitstream or a hardware surface.
fn plane_rows(format: i32, width: i32, height: i32) -> Option<Vec<(usize, usize)>> {
    // SAFETY: `av_pix_fmt_desc_get` takes a format value and returns a pointer
    // to a static descriptor libav owns for the life of the process, or null
    // for a value it does not know, which is checked before it is read.
    // `av_pix_fmt_count_planes` takes the same value and returns a count.
    #[allow(unsafe_code)]
    let (shift, planes, flags) = unsafe {
        let desc = ffi::av_pix_fmt_desc_get(format);
        if desc.is_null() {
            return None;
        }
        (
            i32::from((*desc).log2_chroma_h),
            ffi::av_pix_fmt_count_planes(format),
            (*desc).flags,
        )
    };
    let not_rows =
        ffi::AV_PIX_FMT_FLAG_PAL | ffi::AV_PIX_FMT_FLAG_HWACCEL | ffi::AV_PIX_FMT_FLAG_BITSTREAM;
    if flags & u64::from(not_rows) != 0 || !(1..=4).contains(&planes) || shift > 4 {
        return None;
    }
    (0..planes)
        .map(|plane| {
            // The two colour planes are the ones stored at reduced height.
            let rows = if plane == 1 || plane == 2 {
                (height + (1 << shift) - 1) >> shift
            } else {
                height
            };
            // SAFETY: plain values in, a byte count or a negative error out.
            #[allow(unsafe_code)]
            let bytes = unsafe { ffi::av_image_get_linesize(format, width, plane) };
            Some((usize::try_from(rows).ok()?, usize::try_from(bytes).ok()?))
                .filter(|(rows, bytes)| *rows > 0 && *bytes > 0)
        })
        .collect()
}

/// A new frame of `height` rows in `like`'s format, row `r` of every plane
/// copied from the frame and the row `from(r)` names. Every frame it names
/// must be `like`'s size and format. A row past the end of a plane is its
/// last row.
fn rows_from<'a>(
    like: &AVFrame,
    height: i32,
    from: impl Fn(usize) -> (&'a AVFrame, usize),
) -> Option<AVFrame> {
    let source = plane_rows(like.format, like.width, like.height)?;
    let rows = plane_rows(like.format, like.width, height)?;
    let mut out = AVFrame::new();
    out.set_width(like.width);
    out.set_height(height);
    out.set_format(like.format);
    out.alloc_buffer().ok()?;
    // SAFETY: both are live frames, and this copies colour and timing
    // properties only, never picture data.
    #[allow(unsafe_code)]
    unsafe {
        ffi::av_frame_copy_props(out.as_mut_ptr(), like.as_ptr());
    }
    for (plane, ((count, bytes), (source_rows, _))) in rows.iter().zip(&source).enumerate() {
        for r in 0..*count {
            let (src, at) = from(r);
            let at = at.min(source_rows.saturating_sub(1));
            let (from_ptr, from_stride) = (*src.data.get(plane)?, *src.linesize.get(plane)?);
            let (to_ptr, to_stride) = (*out.data.get(plane)?, *out.linesize.get(plane)?);
            let from_stride = usize::try_from(from_stride).ok()?;
            let to_stride = usize::try_from(to_stride).ok()?;
            if from_ptr.is_null() || to_ptr.is_null() || from_stride < *bytes || to_stride < *bytes
            {
                return None;
            }
            // SAFETY: `at` is below the plane's row count for a frame of the
            // source's size and format, and `r` below the count for the new
            // frame's, both worked out from libav's own description of the
            // format. Each row is at least `bytes` long, checked above
            // against the stride. The two frames are separate allocations.
            #[allow(unsafe_code)]
            unsafe {
                std::ptr::copy_nonoverlapping(
                    from_ptr.add(at * from_stride),
                    to_ptr.add(r * to_stride),
                    *bytes,
                );
            }
        }
    }
    Some(out)
}

/// One field of a frame as a picture of its own, half as tall: the rows of
/// `parity` from every plane. `None` where the planes can't be read as rows,
/// and the caller separates the converted picture instead.
pub(super) fn field_planes(frame: &AVFrame, parity: usize) -> Option<AVFrame> {
    let rows = usize::try_from(frame.height).ok()?;
    let height = i32::try_from((rows + 1).saturating_sub(parity & 1) / 2).ok()?;
    if height < 1 {
        return None;
    }
    rows_from(frame, height, |r| (frame, 2 * r + (parity & 1)))
}

/// The rows of `parity` from `first` with the others from `second`, plane by
/// plane: the two halves of a film frame that pulldown split across two.
pub(super) fn weave_planes(first: &AVFrame, second: &AVFrame, parity: usize) -> Option<AVFrame> {
    let same =
        (first.format, first.width, first.height) == (second.format, second.width, second.height);
    if !same {
        return None;
    }
    rows_from(first, first.height, |r| {
        (if r % 2 == parity & 1 { first } else { second }, r)
    })
}
