//! Reading OpenEXR by channel name (docs/impl/media-io.md §5b).
//!
//! # In plain terms
//!
//! A render leaves an EXR holding far more than a picture. Beside the red,
//! green and blue there may be a `Z` saying how far away each pixel is,
//! normals, an object id, a dozen light groups, a cryptomatte. ffmpeg hands
//! back the RGB and quietly drops the rest, and it has no way to even *say*
//! what the rest was — its decoder can be pointed at a named layer but cannot
//! list the names.
//!
//! So this module does two things ffmpeg cannot: it reads the channel list off
//! a file's header, and it reads four channels the caller names into an
//! ordinary RGBA frame. Everything else about EXR still goes through ffmpeg,
//! which is where the ordinary decode path already is.

use std::path::Path;

use lumit_ingress::{checked_raster_bytes, checked_usize, Budget, IngressError, Limits};

use crate::decode::{DecodedFrame, PixelFormat};
use crate::MediaError;

// ---------------------------------------------------------------------------
// What this reader will believe
// ---------------------------------------------------------------------------
//
// In plain terms: an EXR header is a list of numbers saying how big the picture
// is and how many channels it has, and the reader below allocates from those
// numbers. They are the file's numbers, not Lumit's, and a file is a thing
// somebody sends you. `65535 × 65535 × 32 channels` is a legal header and about
// 550 gigabytes of float; the header costs a few hundred bytes to write.
//
// So the header is read on its own first, checked against the ceilings below,
// and only then is the file opened for its pixels. The alternative — checking
// afterwards — is checking after the machine has already swapped itself to
// death.

/// The largest sample any EXR channel stores: `f32` and `u32` are four bytes,
/// `f16` is two. Used for the pre-read estimate, where guessing high is the
/// safe direction.
const MAX_SAMPLE_BYTES: u64 = 4;

/// What a header may claim before this reader stops believing it.
///
/// A struct rather than three constants so the tests can lower them and watch a
/// real file be refused. A ceiling nothing ever reaches is a ceiling nobody
/// knows works.
#[derive(Debug, Clone, Copy)]
struct Ceilings {
    /// The widest or tallest picture this reader will open.
    ///
    /// 16K footage is 15360 across. A render at 65536 on a side is not a frame
    /// anybody is compositing; it is a number somebody typed.
    dimension: u64,
    /// The most channels this reader will open across every part of a file.
    ///
    /// A heavy render — beauty, depth, normals, a dozen light groups, a
    /// three-layer cryptomatte — lands around a hundred. A thousand is room for
    /// something unusual; beyond that the file is not a render.
    channels: u64,
    /// What the whole file may occupy once decoded, across every part.
    ///
    /// [`Limits::IMAGE`]'s byte ceiling: one 16K RGBA float frame with room to
    /// spare, which is the largest single picture this application has business
    /// decoding in one piece.
    decoded_bytes: u64,
}

impl Ceilings {
    const DEFAULT: Ceilings = Ceilings {
        dimension: 65_536,
        channels: 1_024,
        decoded_bytes: Limits::IMAGE.bytes,
    };
}

/// Check an EXR's header before its pixels are read, and say what the read will
/// cost.
///
/// **This is the load-bearing function of this module.** `read_all_flat_layers`
/// below reads *every* channel of *every* part — that is how the crate's typed
/// selection works, and it is fine for the forty-AOV renders it was written
/// for — which means the allocation the file asks for is the whole file, and
/// the only place to refuse it is before the read starts.
fn weigh_header(path: &Path) -> Result<u64, MediaError> {
    weigh_header_within(path, Ceilings::DEFAULT)
}

fn weigh_header_within(path: &Path, ceilings: Ceilings) -> Result<u64, MediaError> {
    let meta = exr::meta::MetaData::read_from_file(path, false)
        .map_err(|e| MediaError::Ffmpeg(format!("could not read the EXR header: {e}")))?;

    let mut channels: u64 = 0;
    let mut decoded: u64 = 0;
    for header in &meta.headers {
        let size = header.layer_size;
        let (width, height) = (size.width() as u64, size.height() as u64);
        if width > ceilings.dimension || height > ceilings.dimension {
            return Err(MediaError::TooLarge(IngressError::Bytes {
                needed: width.max(height),
                limit: ceilings.dimension,
            }));
        }
        let here = u64::try_from(header.channels.list.len()).unwrap_or(u64::MAX);
        channels = channels.saturating_add(here);
        if channels > ceilings.channels {
            return Err(MediaError::TooLarge(IngressError::Items {
                needed: channels,
                limit: ceilings.channels,
            }));
        }
        // Checked, not saturating: a header whose numbers do not multiply is a
        // header to refuse, and saturating would quietly turn it into "very
        // large" and let the comparison below decide — which is the same answer
        // today and the wrong one the first time a ceiling is raised.
        let part = checked_raster_bytes(width, height, here, MAX_SAMPLE_BYTES)?;
        decoded = decoded.checked_add(part).ok_or(IngressError::Overflow)?;
        if decoded > ceilings.decoded_bytes {
            return Err(MediaError::TooLarge(IngressError::Bytes {
                needed: decoded,
                limit: ceilings.decoded_bytes,
            }));
        }
    }
    Ok(decoded)
}

/// Whether this path names an OpenEXR file, by extension.
///
/// Case-insensitive, because a render farm's naming is nobody's to predict.
#[must_use]
pub fn is_exr(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("exr"))
}

/// Every channel this file holds, in the order the file lists them.
///
/// Names are the file's own, layer prefix and all — `R`, `Z`, `N.X`,
/// `diffuse.R`, `crypto_object00.a`. They are what the Extract channels effect
/// offers in its dropdowns, so they are passed through exactly as written
/// rather than tidied: a name the user recognises from their render settings is
/// worth more than a name that sorts nicely.
///
/// A file with more than one part contributes every part's channels. A file
/// that will not open, or is not an EXR at all, is an error rather than an
/// empty list — "this file has no channels" and "this file could not be read"
/// are different answers and the caller shows them differently.
pub fn channels(path: &Path) -> Result<Vec<String>, MediaError> {
    let meta = exr::meta::MetaData::read_from_file(path, false)
        .map_err(|e| MediaError::Ffmpeg(format!("could not read the EXR header: {e}")))?;
    let mut out: Vec<String> = Vec::new();
    let mut seen: u64 = 0;
    for header in &meta.headers {
        for channel in &header.channels.list {
            // Counted across every part, not per part: a file with a thousand
            // parts of one channel each fills a dropdown just as effectively as
            // one part with a thousand channels.
            seen = seen.saturating_add(1);
            if seen > Ceilings::DEFAULT.channels {
                return Err(MediaError::TooLarge(IngressError::Items {
                    needed: seen,
                    limit: Ceilings::DEFAULT.channels,
                }));
            }
            let name = channel.name.to_string();
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    Ok(out)
}

/// Read four named channels into one RGBA frame, scene-linear float.
///
/// `wanted` is red, green, blue and alpha in that order; `None` in a slot
/// leaves it at zero, except alpha, which is left fully opaque — a depth pass
/// routed into red has no alpha of its own and an invisible layer is not what
/// anybody meant by that. A name the file does not hold is treated as `None`
/// rather than as a fault, so a project whose EXRs changed shape still opens
/// and shows which slot went quiet.
///
/// **This reads the whole file.** The crate's typed channel selection is
/// compile-time shaped, and the names here are chosen at runtime, so the way to
/// pick by name is to read the flat layers and index them. An EXR with forty
/// AOVs costs forty AOVs of memory for the moment it takes to pull four of
/// them, which is the same order as what opening the file costs anyway.
pub fn read_channels(
    path: &Path,
    wanted: &[Option<String>; 4],
) -> Result<DecodedFrame, MediaError> {
    // The header, on its own, before a single pixel is allocated. What it says
    // the file will cost is charged against this read's budget, so the four
    // planes and the interleaved frame below are spending what is left of a
    // ceiling the file has already been measured against rather than starting
    // again from nothing.
    let mut budget = Budget::new(Limits::IMAGE);
    budget.take_bytes(weigh_header(path)?)?;

    let image = exr::prelude::read_all_flat_layers_from_file(path)
        .map_err(|e| MediaError::Ffmpeg(format!("could not read the EXR: {e}")))?;
    let layer = image
        .layer_data
        .first()
        .ok_or_else(|| MediaError::Ffmpeg("the EXR holds no layers".into()))?;
    let width = u32::try_from(layer.size.width()).unwrap_or(0);
    let height = u32::try_from(layer.size.height()).unwrap_or(0);
    let px = checked_usize(lumit_ingress::checked_area(
        layer.size.width() as u64,
        layer.size.height() as u64,
    )?)?;

    // One pass per slot, so a channel named twice is read once per slot and a
    // slot naming nothing costs nothing.
    let mut planes: [Option<Vec<f32>>; 4] = [None, None, None, None];
    for (slot, name) in wanted.iter().enumerate() {
        let Some(name) = name else { continue };
        let Some(channel) = layer
            .channel_data
            .list
            .iter()
            .find(|c| c.name.to_string() == *name)
        else {
            continue;
        };
        let mut plane = budget.vec_with_capacity::<f32>(px)?;
        for i in 0..px {
            plane.push(sample_at(&channel.sample_data, i));
        }
        planes[slot] = Some(plane);
    }

    // Four bytes a channel, four channels a pixel.
    let interleaved = checked_usize(checked_raster_bytes(
        layer.size.width() as u64,
        layer.size.height() as u64,
        4,
        4,
    )?)?;
    let mut rgba = budget.vec_with_capacity::<u8>(interleaved)?;
    for i in 0..px {
        for (slot, plane) in planes.iter().enumerate() {
            // An unfilled alpha is opaque; an unfilled colour is black.
            let default = if slot == 3 { 1.0f32 } else { 0.0 };
            let v = plane
                .as_ref()
                .and_then(|p| p.get(i).copied())
                .unwrap_or(default);
            rgba.extend_from_slice(&v.to_le_bytes());
        }
    }
    Ok(DecodedFrame {
        width,
        height,
        rgba,
        format: PixelFormat::LinearF32,
    })
}

/// The file frame `n` of this source is, when that file is an OpenEXR — a run
/// of stills resolves to its own numbered file, a plain source to itself.
///
/// `None` for anything that is not an EXR, which is how the caller decides
/// between this reader and the ordinary ffmpeg decode.
#[must_use]
pub fn file_for(source: &crate::MediaSource, frame: usize) -> Option<std::path::PathBuf> {
    let path = match source.run() {
        Some((run, _)) => run.file_at(frame),
        None => source.path.clone(),
    };
    is_exr(&path).then_some(path)
}

/// Box-average a float RGBA frame down to `target_width`, keeping its aspect.
///
/// The same reason the ordinary decode scales: a preview at a third of the size
/// has no use for a 4K texture, and the upload and the video memory are what it
/// costs. Averaging rather than dropping samples, so a fine pattern reads as
/// grey instead of as moiré — and so a depth pass reads as the distance across
/// the pixel rather than as whichever corner was sampled.
///
/// A target at or above the frame's own width hands the frame back untouched.
#[must_use]
pub fn downsample(frame: DecodedFrame, target_width: Option<u32>) -> DecodedFrame {
    let Some(dst_w) = target_width.filter(|w| *w < frame.width && *w >= 1) else {
        return frame;
    };
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let dw = dst_w as usize;
    let dh = ((sh * dw) / sw.max(1)).max(1);
    let mut out = Vec::with_capacity(dw * dh * 16);
    for y in 0..dh {
        let (y0, y1) = ((y * sh) / dh, (((y + 1) * sh) / dh).max((y * sh) / dh + 1));
        for x in 0..dw {
            let (x0, x1) = ((x * sw) / dw, (((x + 1) * sw) / dw).max((x * sw) / dw + 1));
            let count = ((y1 - y0) * (x1 - x0)) as f32;
            let mut sum = [0.0f32; 4];
            for sy in y0..y1.min(sh) {
                for sx in x0..x1.min(sw) {
                    let base = (sy * sw + sx) * 16;
                    for (c, slot) in sum.iter_mut().enumerate() {
                        let at = base + c * 4;
                        *slot += frame
                            .rgba
                            .get(at..at + 4)
                            .and_then(|b| <[u8; 4]>::try_from(b).ok())
                            .map_or(0.0, f32::from_le_bytes);
                    }
                }
            }
            for v in sum {
                out.extend_from_slice(&(v / count.max(1.0)).to_le_bytes());
            }
        }
    }
    DecodedFrame {
        width: dw as u32,
        height: dh as u32,
        rgba: out,
        format: PixelFormat::LinearF32,
    }
}

/// One sample as a float, whichever of the three ways EXR stored it.
///
/// EXR channels are half, float or unsigned int, and a file mixes them freely —
/// colour in half, `Z` in float, an object id in uint. The compositor works in
/// float, so all three arrive as one, and a uint id keeps its exact value up to
/// the point floats stop counting integers, which is well past any id a
/// renderer writes.
fn sample_at(samples: &exr::prelude::FlatSamples, i: usize) -> f32 {
    match samples {
        exr::prelude::FlatSamples::F16(v) => v.get(i).map_or(0.0, |s| s.to_f32()),
        exr::prelude::FlatSamples::F32(v) => v.get(i).copied().unwrap_or(0.0),
        exr::prelude::FlatSamples::U32(v) => v.get(i).map_or(0.0, |s| *s as f32),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn is_exr_ignores_case_and_other_extensions() {
        assert!(is_exr(Path::new("a/b.exr")));
        assert!(is_exr(Path::new("a/b.EXR")));
        assert!(!is_exr(Path::new("a/b.png")));
        assert!(!is_exr(Path::new("a/b")));
    }

    /// A file that is not there is an error, not an empty channel list: the
    /// two mean different things to whoever is looking at the dropdown.
    #[test]
    fn a_missing_file_errors_rather_than_reading_as_channelless() {
        assert!(channels(Path::new("Z:/definitely/not/here.exr")).is_err());
        assert!(read_channels(
            Path::new("Z:/definitely/not/here.exr"),
            &[None, None, None, None]
        )
        .is_err());
    }

    /// The thing ffmpeg cannot do: say what is in the file. Every channel a
    /// render wrote comes back, layer prefix and all, so the effect's dropdowns
    /// offer the names the user set in their render settings.
    #[test]
    fn every_channel_of_a_render_is_listed() {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::index::tests_support::multichannel_exr(dir.path());
        let mut found = channels(&file).unwrap();
        found.sort();
        assert_eq!(found, ["A", "B", "G", "N.X", "R", "Z"]);
    }

    /// The other thing ffmpeg cannot do: hand back a channel that is not part
    /// of the picture. A depth pass routed into red arrives at its own values,
    /// which run to 1600 here — past what a half float counts in whole numbers,
    /// which is the whole reason the carrier is a full float.
    #[test]
    fn a_named_depth_channel_arrives_at_its_own_values() {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::index::tests_support::multichannel_exr(dir.path());
        let frame =
            read_channels(&file, &[Some("Z".into()), None, None, Some("A".into())]).unwrap();

        assert_eq!(frame.format, PixelFormat::LinearF32);
        assert_eq!((frame.width, frame.height), (4, 4));
        for i in [0usize, 7, 15] {
            let want = crate::index::tests_support::multichannel_depth_at(i);
            let got =
                f32::from_le_bytes(<[u8; 4]>::try_from(&frame.rgba[i * 16..i * 16 + 4]).unwrap());
            assert_eq!(got, want, "pixel {i} read as {got}, wanted {want}");
        }
    }

    /// An empty slot is black, an empty alpha is opaque. A depth pass has no
    /// alpha of its own, and an invisible layer is not what anybody meant by
    /// routing one into red.
    #[test]
    fn an_unfilled_slot_is_black_and_an_unfilled_alpha_is_opaque() {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::index::tests_support::multichannel_exr(dir.path());
        let frame = read_channels(&file, &[Some("R".into()), None, None, None]).unwrap();

        let px: Vec<f32> = frame.rgba[..16]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(<[u8; 4]>::try_from(b).unwrap_or([0; 4])))
            .collect();
        assert_eq!(px, [0.75, 0.0, 0.0, 1.0]);
    }

    /// A render's own file spends a small share of the ceilings, which is the
    /// check that they are not in an honest file's way.
    #[test]
    fn an_ordinary_render_is_well_inside_the_ceilings() {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::index::tests_support::multichannel_exr(dir.path());
        let decoded = weigh_header(&file).unwrap();
        // 4 × 4 × 6 channels × 4 bytes.
        assert_eq!(decoded, 384);
        assert!(decoded < Ceilings::DEFAULT.decoded_bytes / 1000);
    }

    /// The header is the whole attack surface: it is a few hundred bytes that
    /// say how many gigabytes to allocate, and it is read before any of them
    /// are. A file whose parts add up past the ceiling is refused there, with
    /// no pixels read.
    #[test]
    fn every_part_of_a_file_counts_towards_the_ceilings() {
        use exr::prelude::*;

        // The shape that matters: many parts, each ordinary. A reader that
        // measured only the first part would see a 4×4 picture and allocate the
        // whole file. Written small and checked against ceilings lowered to
        // match, because a fixture that actually reached four gigabytes would
        // be a test that needs four gigabytes.
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("many.exr");
        let px = 16usize;
        let layer = |name: &str| {
            Layer::new(
                (4, 4),
                LayerAttributes::named(name),
                Encoding::FAST_LOSSLESS,
                AnyChannels::sort(
                    ["R", "G", "B", "A"]
                        .into_iter()
                        .map(|c| AnyChannel::new(c, FlatSamples::F32(vec![0.5; px])))
                        .collect::<Vec<_>>()
                        .into_iter()
                        .collect(),
                ),
            )
        };
        let layers: Vec<_> = (0..8).map(|i| layer(&format!("part{i}"))).collect();
        Image::from_layers(ImageAttributes::with_size((4, 4)), layers)
            .write()
            .to_file(&out)
            .unwrap();

        // Every part counted, not just the first: 8 × 4 × 4 × 4 channels × 4.
        assert_eq!(weigh_header(&out).unwrap(), 2_048);

        // A byte ceiling between one part and all eight refuses the file, and
        // does it from the header — no pixels were read to find out.
        let tight = weigh_header_within(
            &out,
            Ceilings {
                decoded_bytes: 1_024,
                ..Ceilings::DEFAULT
            },
        );
        assert!(
            matches!(
                tight,
                Err(MediaError::TooLarge(IngressError::Bytes {
                    limit: 1_024,
                    ..
                }))
            ),
            "a file past the byte ceiling must be refused: {tight:?}"
        );

        // The channel count is an aggregate too — thirty-two channels spread
        // over eight parts cost the same as thirty-two in one.
        let tight = weigh_header_within(
            &out,
            Ceilings {
                channels: 16,
                ..Ceilings::DEFAULT
            },
        );
        assert!(
            matches!(
                tight,
                Err(MediaError::TooLarge(IngressError::Items { limit: 16, .. }))
            ),
            "channels must be counted across parts: {tight:?}"
        );

        // And a dimension no picture has is refused before anything is
        // multiplied by it.
        let tight = weigh_header_within(
            &out,
            Ceilings {
                dimension: 3,
                ..Ceilings::DEFAULT
            },
        );
        assert!(
            matches!(
                tight,
                Err(MediaError::TooLarge(IngressError::Bytes { limit: 3, .. }))
            ),
            "an oversized picture must be refused: {tight:?}"
        );

        // The listing walks the same parts and de-duplicates the names.
        let listed = channels(&out).unwrap();
        assert_eq!(listed.len(), 4, "the names repeat across parts: {listed:?}");
    }

    /// A header that claims a picture no machine holds is refused rather than
    /// believed, and refused by arithmetic that cannot itself wrap: this is the
    /// `65535 × 65535 × 32` shape, where every number is plausible and the
    /// product is not.
    #[test]
    fn raster_arithmetic_on_header_numbers_cannot_wrap() {
        use lumit_ingress::{checked_raster_bytes, IngressError};
        let c = Ceilings::DEFAULT;
        assert_eq!(
            checked_raster_bytes(c.dimension, c.dimension, c.channels, MAX_SAMPLE_BYTES).unwrap(),
            65_536 * 65_536 * 1_024 * 4,
        );
        assert_eq!(
            checked_raster_bytes(u64::MAX, u64::MAX, 4, 4).unwrap_err(),
            IngressError::Overflow
        );
        // The ceiling itself is the real gate: the largest header this reader
        // will accept at all is still far past the byte budget, so a file has
        // to pass both.
        assert!(
            (c.dimension * c.dimension * 4 * 4) > c.decoded_bytes,
            "a single full-size four-channel picture must already exceed the byte ceiling"
        );
    }

    /// A name the file does not hold reads as an empty slot rather than a
    /// fault: a project whose EXRs changed shape still opens, and the slot that
    /// went quiet is visible rather than fatal.
    #[test]
    fn a_channel_the_file_lost_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::index::tests_support::multichannel_exr(dir.path());
        let frame =
            read_channels(&file, &[Some("nosuchchannel".into()), None, None, None]).unwrap();
        assert_eq!(&frame.rgba[..4], &0.0f32.to_le_bytes());
    }
}
