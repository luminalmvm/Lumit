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

use crate::decode::{DecodedFrame, PixelFormat};
use crate::MediaError;

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
    let mut out = Vec::new();
    for header in &meta.headers {
        for channel in &header.channels.list {
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
    let image = exr::prelude::read_all_flat_layers_from_file(path)
        .map_err(|e| MediaError::Ffmpeg(format!("could not read the EXR: {e}")))?;
    let layer = image
        .layer_data
        .first()
        .ok_or_else(|| MediaError::Ffmpeg("the EXR holds no layers".into()))?;
    let width = u32::try_from(layer.size.width()).unwrap_or(0);
    let height = u32::try_from(layer.size.height()).unwrap_or(0);
    let px = layer.size.width().saturating_mul(layer.size.height());

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
        let mut plane = Vec::with_capacity(px);
        for i in 0..px {
            plane.push(sample_at(&channel.sample_data, i));
        }
        planes[slot] = Some(plane);
    }

    let mut rgba = Vec::with_capacity(px.saturating_mul(16));
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
