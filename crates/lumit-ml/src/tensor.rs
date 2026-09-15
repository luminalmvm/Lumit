//! Turning a picture into what a model takes, and back again
//! (docs/impl/addons.md §4's tensor column, §11 test 5).
//!
//! Every model here wants the same thing in a slightly different dress: the
//! three colour channels one after another rather than interleaved, scaled to
//! 0..1, sometimes normalised, and sometimes with the frame grown to a size
//! the graph's own downsampling divides evenly.
//!
//! # Thread role and contract
//!
//! Pure arithmetic. No IO, no clocks, no threads, no interior mutability:
//! slices in, owned buffers out (14-ENGINEERING-RULES §1.1), so it is the
//! half of the crate that runs and is tested on every machine.

use crate::manifest::Normalise;

/// How much of the frame Robust Video Matting works at, which is the one
/// choice the user makes about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detail {
    /// Head and shoulders, most of the frame.
    #[default]
    Portrait,
    /// A whole person at a distance, where the finer ratio loses the limbs.
    FullBody,
}

/// Pack interleaved RGBA bytes into one `[3, height, width]` plane of f32
/// 0..1, optionally normalised. Alpha is dropped: none of these models takes
/// it.
///
/// A short slice reads as black rather than refusing, because the caller has
/// already agreed the frame's size and a shorter one is a decode fault it
/// reports for itself.
#[must_use]
pub fn pack_u8(rgba: &[u8], width: usize, height: usize, norm: Option<Normalise>) -> Vec<f32> {
    let pixels = width * height;
    let mut out = vec![0.0f32; 3 * pixels];
    for channel in 0..3 {
        let (mean, std) = scale(norm, channel);
        for pixel in 0..pixels {
            let value = rgba
                .get(pixel * 4 + channel)
                .map_or(0.0, |byte| f32::from(*byte) / 255.0);
            out[channel * pixels + pixel] = (value - mean) / std;
        }
    }
    out
}

/// [`pack_u8`], for a frame that is already f32 RGBA in 0..1.
#[must_use]
pub fn pack_f32(rgba: &[f32], width: usize, height: usize, norm: Option<Normalise>) -> Vec<f32> {
    let pixels = width * height;
    let mut out = vec![0.0f32; 3 * pixels];
    for channel in 0..3 {
        let (mean, std) = scale(norm, channel);
        for pixel in 0..pixels {
            let value = rgba.get(pixel * 4 + channel).copied().unwrap_or(0.0);
            out[channel * pixels + pixel] = (value - mean) / std;
        }
    }
    out
}

/// The inverse of [`pack_u8`]: one `[3, height, width]` plane back to
/// interleaved RGBA bytes, opaque.
#[must_use]
pub fn unpack_u8(planes: &[f32], width: usize, height: usize, norm: Option<Normalise>) -> Vec<u8> {
    let pixels = width * height;
    let mut out = vec![255u8; 4 * pixels];
    for channel in 0..3 {
        let (mean, std) = scale(norm, channel);
        for pixel in 0..pixels {
            let value = planes
                .get(channel * pixels + pixel)
                .map_or(0.0, |v| v * std + mean);
            out[pixel * 4 + channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    out
}

/// The inverse of [`pack_f32`].
#[must_use]
pub fn unpack_f32(
    planes: &[f32],
    width: usize,
    height: usize,
    norm: Option<Normalise>,
) -> Vec<f32> {
    let pixels = width * height;
    let mut out = vec![1.0f32; 4 * pixels];
    for channel in 0..3 {
        let (mean, std) = scale(norm, channel);
        for pixel in 0..pixels {
            out[pixel * 4 + channel] = planes
                .get(channel * pixels + pixel)
                .map_or(0.0, |v| v * std + mean);
        }
    }
    out
}

/// Grow a `[channels, height, width]` plane so both sides are a multiple of
/// `multiple`, repeating the edge pixel outward.
///
/// Repeating the edge rather than filling with black, because a black border
/// is an edge the model can see and invents motion or depth along. Returns
/// the grown plane and its new width and height; [`crop`] takes it back.
#[must_use]
pub fn pad(
    planes: &[f32],
    width: usize,
    height: usize,
    channels: usize,
    multiple: usize,
) -> (Vec<f32>, usize, usize) {
    let up = |n: usize| {
        if multiple == 0 {
            n
        } else {
            n.div_ceil(multiple) * multiple
        }
    };
    let (grown_width, grown_height) = (up(width), up(height));
    if grown_width == width && grown_height == height {
        return (planes.to_vec(), width, height);
    }

    let mut out = vec![0.0f32; channels * grown_width * grown_height];
    for channel in 0..channels {
        for y in 0..grown_height {
            let from_y = y.min(height.saturating_sub(1));
            for x in 0..grown_width {
                let from_x = x.min(width.saturating_sub(1));
                out[channel * grown_width * grown_height + y * grown_width + x] = planes
                    .get(channel * width * height + from_y * width + from_x)
                    .copied()
                    .unwrap_or(0.0);
            }
        }
    }
    (out, grown_width, grown_height)
}

/// Take the top-left `width` by `height` of every plane back out of a grown
/// one. The exact inverse of [`pad`] for the frame itself.
#[must_use]
pub fn crop(
    planes: &[f32],
    grown_width: usize,
    grown_height: usize,
    width: usize,
    height: usize,
    channels: usize,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(channels * width * height);
    for channel in 0..channels {
        for y in 0..height {
            let row = channel * grown_width * grown_height + y * grown_width;
            out.extend_from_slice(planes.get(row..row + width).unwrap_or(&[]));
        }
    }
    out
}

/// The size a frame is resized to for a model that wants `size` on its long
/// side and both sides a multiple of `multiple`.
///
/// The aspect ratio is kept to whatever the rounding allows, so a 1920 by 1080
/// frame going to 518 on the long side comes out 518 by 294 rather than
/// stretched square. A zero either way, or a model that asks for nothing, is
/// answered with at least one whole tile so nothing downstream divides by
/// nought.
#[must_use]
pub fn fit(width: usize, height: usize, size: usize, multiple: usize) -> (usize, usize) {
    let step = multiple.max(1);
    let long = width.max(height);
    let (mut out_width, mut out_height) = if long == 0 || size == 0 {
        (width, height)
    } else if width >= height {
        (size, (size * height).div_ceil(long))
    } else {
        ((size * width).div_ceil(long), size)
    };
    out_width = out_width.div_ceil(step) * step;
    out_height = out_height.div_ceil(step) * step;
    (out_width.max(step), out_height.max(step))
}

/// Resample interleaved RGBA bytes to another size, averaging the source
/// pixels each destination pixel covers.
///
/// An area average rather than a bilinear tap because this is used to shrink a
/// whole frame down to the few hundred pixels a model takes, and a bilinear
/// tap at that ratio reads one pixel in six and aliases everything it skips.
/// Magnifying, every destination pixel covers less than one source pixel and
/// the average is that pixel, which is the nearest-neighbour answer.
#[must_use]
pub fn resample_u8(
    rgba: &[u8],
    width: usize,
    height: usize,
    out_width: usize,
    out_height: usize,
) -> Vec<u8> {
    resample(rgba, Pixel::RGBA, (width, height), (out_width, out_height))
}

/// The same, for a plane of one byte a pixel: coverage rather than colour.
///
/// A frame of nothing resamples to nothing, which for coverage means none of
/// the subject is there.
#[must_use]
pub fn resample_gray(
    plane: &[u8],
    width: usize,
    height: usize,
    out_width: usize,
    out_height: usize,
) -> Vec<u8> {
    resample(plane, Pixel::GRAY, (width, height), (out_width, out_height))
}

/// What one pixel of a buffer [`resample`] walks looks like: how many bytes it
/// takes, how many of them are averaged, and what anything left over is filled
/// with.
struct Pixel {
    stride: usize,
    channels: usize,
    fill: u8,
}

impl Pixel {
    /// Interleaved colour, alpha left opaque.
    const RGBA: Pixel = Pixel {
        stride: 4,
        channels: 3,
        fill: 255,
    };
    /// One byte of coverage, and nothing is none of the subject.
    const GRAY: Pixel = Pixel {
        stride: 1,
        channels: 1,
        fill: 0,
    };
}

/// The resample both of the two above are.
fn resample(src: &[u8], pixel: Pixel, from: (usize, usize), to: (usize, usize)) -> Vec<u8> {
    let Pixel {
        stride,
        channels,
        fill,
    } = pixel;
    let ((width, height), (out_width, out_height)) = (from, to);
    let mut out = vec![fill; stride * out_width * out_height];
    if width == 0 || height == 0 || out_width == 0 || out_height == 0 {
        return out;
    }
    for y in 0..out_height {
        let y0 = y * height / out_height;
        let y1 = (((y + 1) * height).div_ceil(out_height))
            .max(y0 + 1)
            .min(height);
        for x in 0..out_width {
            let x0 = x * width / out_width;
            let x1 = (((x + 1) * width).div_ceil(out_width))
                .max(x0 + 1)
                .min(width);
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for row in y0..y1 {
                for column in x0..x1 {
                    let at = stride * (row * width + column);
                    for (channel, total) in sum.iter_mut().take(channels).enumerate() {
                        *total += u32::from(src.get(at + channel).copied().unwrap_or(0));
                    }
                    count += 1;
                }
            }
            let at = stride * (y * out_width + x);
            for (channel, total) in sum.iter().take(channels).enumerate() {
                if let Some(slot) = out.get_mut(at + channel) {
                    *slot = (total / count.max(1)) as u8;
                }
            }
        }
    }
    out
}

/// How far Robust Video Matting downsamples internally for a frame this size
/// (docs/impl/addons.md §4, the model's own table).
///
/// Keyed on the longer side, so a portrait frame and a landscape one of the
/// same footage get the same answer.
#[must_use]
pub fn downsample_ratio(width: usize, height: usize, detail: Detail) -> f32 {
    match (width.max(height), detail) {
        (0..=512, _) => 1.0,
        (513..=1280, Detail::Portrait) => 0.375,
        (513..=1280, Detail::FullBody) => 0.6,
        (1281..=1920, Detail::Portrait) => 0.25,
        (1281..=1920, Detail::FullBody) => 0.4,
        (_, Detail::Portrait) => 0.125,
        (_, Detail::FullBody) => 0.2,
    }
}

/// What to subtract and what to divide by for one channel, or leave it alone.
fn scale(norm: Option<Normalise>, channel: usize) -> (f32, f32) {
    norm.map_or((0.0, 1.0), |norm| {
        (
            norm.mean.get(channel).copied().unwrap_or(0.0),
            norm.std.get(channel).copied().unwrap_or(1.0),
        )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::manifest::IMAGENET;

    /// A small frame where every byte is different, so a packing that
    /// transposes two channels or two rows cannot pass.
    fn frame(width: usize, height: usize) -> Vec<u8> {
        (0..width * height)
            .flat_map(|pixel| {
                let base = (pixel * 7) as u8;
                [base, base.wrapping_add(29), base.wrapping_add(113), 255]
            })
            .collect()
    }

    /// **Eight-bit packing and unpacking round trip exactly.** A byte through
    /// 0..1 and back is the same byte, so nothing a plane carries is lost to
    /// the trip itself.
    #[test]
    fn eight_bit_packing_round_trips() {
        let (width, height) = (5, 3);
        let rgba = frame(width, height);
        let planes = pack_u8(&rgba, width, height, None);
        assert_eq!(planes.len(), 3 * width * height);
        assert!((planes[0] - f32::from(rgba[0]) / 255.0).abs() < 1e-6);
        assert!(
            (planes[width * height] - f32::from(rgba[1]) / 255.0).abs() < 1e-6,
            "green starts one plane in"
        );
        assert_eq!(unpack_u8(&planes, width, height, None), rgba);
    }

    /// **ImageNet normalisation inverts.** The mean and standard deviation
    /// the manifest carries are undone on the way back out, so a model's
    /// picture output lands where it started.
    #[test]
    fn imagenet_normalisation_inverts() {
        let (width, height) = (4, 4);
        let rgba = frame(width, height);
        let planes = pack_u8(&rgba, width, height, Some(IMAGENET));
        assert!(
            planes.iter().any(|v| *v < 0.0),
            "normalised values leave 0..1"
        );
        assert_eq!(unpack_u8(&planes, width, height, Some(IMAGENET)), rgba);

        let floats: Vec<f32> = rgba.iter().map(|b| f32::from(*b) / 255.0).collect();
        let planes = pack_f32(&floats, width, height, Some(IMAGENET));
        let back = unpack_f32(&planes, width, height, Some(IMAGENET));
        for (was, now) in floats.iter().zip(&back) {
            assert!((was - now).abs() < 1e-5, "{was} came back as {now}");
        }
    }

    /// **The padding to a multiple and the crop back are exact.** RIFE wants
    /// a multiple of 32 and Depth Anything a multiple of 14; both are grown
    /// by repeating the edge and cut back to the byte.
    #[test]
    fn padding_to_a_multiple_crops_back_exactly() {
        for multiple in [32usize, 14] {
            let (width, height) = (37, 21);
            let planes: Vec<f32> = (0..3 * width * height).map(|n| n as f32).collect();
            let (grown, grown_width, grown_height) = pad(&planes, width, height, 3, multiple);
            assert_eq!(grown_width % multiple, 0);
            assert_eq!(grown_height % multiple, 0);
            assert!(grown_width >= width && grown_height >= height);
            assert_eq!(grown.len(), 3 * grown_width * grown_height);
            assert_eq!(
                crop(&grown, grown_width, grown_height, width, height, 3),
                planes
            );
        }
    }

    /// **A frame already a multiple of the number is not touched.** The
    /// common case costs a copy and no arithmetic.
    #[test]
    fn a_frame_already_a_multiple_is_left_alone() {
        let (width, height) = (64, 32);
        let planes: Vec<f32> = (0..3 * width * height).map(|n| n as f32).collect();
        let (grown, grown_width, grown_height) = pad(&planes, width, height, 3, 32);
        assert_eq!((grown_width, grown_height), (width, height));
        assert_eq!(grown, planes);
    }

    /// **The fit keeps the long side at the size asked for and both sides on
    /// the tile.** Depth Anything wants 518 on the long side and multiples of
    /// 14; a frame fitted to anything else is a frame the graph refuses.
    #[test]
    fn the_fit_lands_on_the_long_side_and_the_tile() {
        for (width, height) in [(1920usize, 1080usize), (1080, 1920), (640, 640), (37, 21)] {
            let (out_width, out_height) = fit(width, height, 518, 14);
            assert_eq!(out_width % 14, 0, "{width}x{height} width off the tile");
            assert_eq!(out_height % 14, 0, "{width}x{height} height off the tile");
            assert_eq!(out_width.max(out_height), 518, "{width}x{height} long side");
            // The shape is kept: the short side is within one tile of the
            // ratio the frame came in at.
            let wanted = 518.0 * (width.min(height) as f64) / (width.max(height) as f64);
            assert!(
                (out_width.min(out_height) as f64 - wanted).abs() <= 14.0,
                "{width}x{height} came out {out_width}x{out_height}"
            );
        }
        assert_eq!(fit(0, 0, 518, 14), (14, 14), "nothing still fills one tile");
    }

    /// **Shrinking averages every source pixel, and growing takes the nearest
    /// one.** A frame of one colour comes back that colour whatever the ratio,
    /// and a half-and-half frame keeps its halves rather than smearing them
    /// across the join.
    #[test]
    fn the_resample_averages_down_and_holds_the_edge() {
        let (width, height) = (8usize, 8usize);
        let flat: Vec<u8> = (0..width * height)
            .flat_map(|_| [40u8, 90, 200, 255])
            .collect();
        let small = resample_u8(&flat, width, height, 2, 2);
        assert_eq!(small.len(), 4 * 2 * 2);
        assert!(
            small.chunks_exact(4).all(|p| p[..3] == [40, 90, 200]),
            "one colour shrinks to itself: {small:?}"
        );

        // Left half black, right half white.
        let split: Vec<u8> = (0..width * height)
            .flat_map(|pixel| {
                let v = if pixel % width < width / 2 { 0u8 } else { 255 };
                [v, v, v, 255]
            })
            .collect();
        let halves = resample_u8(&split, width, height, 2, 1);
        assert_eq!(halves[0], 0, "the left half is still black");
        assert_eq!(halves[4], 255, "and the right half still white");

        // Growing: every destination pixel reads one source pixel.
        let big = resample_u8(&split, width, height, 16, 16);
        assert_eq!(big.len(), 4 * 16 * 16);
        assert_eq!(big[0], 0);
        assert_eq!(big[4 * 15], 255);
        assert!(
            big.chunks_exact(4).all(|p| p[0] == 0 || p[0] == 255),
            "a magnification invents no in-between value"
        );
    }

    /// **A one-byte plane resamples the same way.** A matte comes back at the
    /// square the model works in and has to be taken to the frame's own shape,
    /// and a coverage that averaged the wrong bytes would be a matte with the
    /// subject in the wrong place.
    #[test]
    fn a_coverage_resamples_by_the_same_arithmetic() {
        let (width, height) = (8usize, 8usize);
        let split: Vec<u8> = (0..width * height)
            .map(|pixel| if pixel % width < width / 2 { 0u8 } else { 255 })
            .collect();
        let halves = resample_gray(&split, width, height, 2, 1);
        assert_eq!(halves, [0, 255], "the halves stayed where they were");

        let big = resample_gray(&split, width, height, 16, 16);
        assert_eq!(big.len(), 16 * 16);
        assert!(
            big.iter().all(|v| *v == 0 || *v == 255),
            "a magnification invents no in-between coverage"
        );
        assert_eq!(
            resample_gray(&[], 0, 0, 4, 4),
            [0u8; 16],
            "nothing covers nothing"
        );
    }

    /// **The downsample table picks the documented ratio per resolution.**
    /// The numbers are the model's own, and a wrong one is a matte that is
    /// soft everywhere or ragged everywhere.
    #[test]
    fn the_downsample_table_picks_the_documented_ratio() {
        assert!((downsample_ratio(512, 512, Detail::Portrait) - 1.0).abs() < f32::EPSILON);
        assert!((downsample_ratio(512, 288, Detail::FullBody) - 1.0).abs() < f32::EPSILON);
        assert!((downsample_ratio(1280, 720, Detail::Portrait) - 0.375).abs() < f32::EPSILON);
        assert!((downsample_ratio(1280, 720, Detail::FullBody) - 0.6).abs() < f32::EPSILON);
        assert!((downsample_ratio(1920, 1080, Detail::Portrait) - 0.25).abs() < f32::EPSILON);
        assert!((downsample_ratio(1920, 1080, Detail::FullBody) - 0.4).abs() < f32::EPSILON);
        assert!((downsample_ratio(3840, 2160, Detail::Portrait) - 0.125).abs() < f32::EPSILON);
        assert!((downsample_ratio(3840, 2160, Detail::FullBody) - 0.2).abs() < f32::EPSILON);
        assert!(
            (downsample_ratio(720, 1280, Detail::Portrait) - 0.375).abs() < f32::EPSILON,
            "a portrait frame reads its longer side"
        );
    }
}
