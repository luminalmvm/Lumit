//! How far away every pixel is, guessed from one frame (docs/impl/addons.md
//! §6.1).
//!
//! The model takes a picture and hands back a plane of numbers with no unit
//! and no zero: larger is nearer, and that is the whole contract. Nothing here
//! turns it into metres, because the model cannot. What comes back is quantised
//! over the frame's own smallest and largest number, so one frame's plane says
//! what is near and far *in that frame*, which is what a defocus or a fog
//! reads it for.
//!
//! The plane comes back at the model's own output resolution rather than the
//! frame's. The model produced nothing finer, so growing it here would be
//! inventing detail and storing it; the draw resamples on the card, where every
//! other differently-sized input is fitted.
//!
//! # Thread role and contract
//!
//! A [`Depth`] owns its session and is never shared: a run is an FFI call that
//! may hold the GPU, and 14-ENGINEERING-RULES §1.3 forbids holding a lock
//! across one. The analysis job builds one on its own thread and drops it when
//! the run ends. A single run of one frame is not interruptible; cancellation
//! happens between frames, in the caller's own loop.

use crate::{
    error::MlError,
    manifest::{Arch, DepthAnything, DepthKind, Task},
    runtime,
    session::{Prefer, Session},
    store, tensor,
};

/// One frame's depth, at the model's own output resolution.
///
/// `data` is row-major, one number per pixel, quantised over the frame's own
/// range: 0 is the furthest thing in this frame and 65535 the nearest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u16>,
}

/// An open depth model, ready to read frames.
#[derive(Debug)]
pub struct Depth {
    session: Session,
    /// What the pack's manifest calls the tensors, how big a frame it wants,
    /// and how to read what comes back.
    names: DepthAnything,
    identity: [u8; 32],
    pack: String,
}

impl Depth {
    /// Open the installed depth pack.
    ///
    /// # Errors
    ///
    /// [`MlError::RuntimeMissing`] when no model runtime is installed,
    /// [`MlError::PackMissing`] when nothing installed does depth, and whatever
    /// [`Session::open`] answers for a pack that is there and will not open.
    /// Each is a calm sentence on the effect's badge, never a picture invented
    /// to fill the gap (§9).
    pub fn open() -> Result<Self, MlError> {
        if matches!(runtime::status(), runtime::RuntimeStatus::Missing) {
            return Err(MlError::RuntimeMissing);
        }
        let installed = store::find(Task::Depth).ok_or(MlError::PackMissing(Task::Depth))?;
        let Some(Arch::DepthAnything(names)) =
            installed.manifest.model.as_ref().map(|model| &model.arch)
        else {
            // The manifest parser holds task and arch to each other, so this
            // is a pack claiming depth with a family that does something else,
            // which only a hand-written manifest can be.
            return Err(MlError::PackUnreadable);
        };
        let names = names.clone();
        let identity = store::identity(&installed.manifest);
        let pack = installed.manifest.id.clone();
        let session = Session::open(&installed.path, &names.file, Prefer::Platform)?;
        Ok(Depth {
            session,
            names,
            identity,
            pack,
        })
    }

    /// What this pack is, for the key every plane it makes is filed under (§7).
    /// Two packs, or two versions of one, never share it.
    #[must_use]
    pub fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// Which pack this is, by the id its folder carries. The provenance a
    /// sidecar keeps names it (§7), so a plane made by one pack can be told
    /// from a plane made by another after the fact.
    #[must_use]
    pub fn pack(&self) -> &str {
        &self.pack
    }

    /// Which provider took the session: part of the same answer, and what the
    /// status card shows (§8).
    #[must_use]
    pub fn provider(&self) -> &'static str {
        self.session.provider()
    }

    /// Read one frame of RGBA bytes and hand back its depth.
    ///
    /// # Errors
    ///
    /// [`MlError::ShapeMismatch`] for a frame of no size, and for a model that
    /// hands back something that is not a plane; [`MlError::ModelFailed`] with
    /// ONNX Runtime's own words for a run it refuses.
    pub fn run(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<Plane, MlError> {
        let (width, height) = (width as usize, height as usize);
        if width == 0 || height == 0 {
            return Err(MlError::ShapeMismatch);
        }
        let (fed_width, fed_height) = tensor::fit(
            width,
            height,
            self.names.size as usize,
            self.names.multiple as usize,
        );
        let small = tensor::resample_u8(rgba, width, height, fed_width, fed_height);
        let planes = tensor::pack_u8(&small, fed_width, fed_height, Some(self.names.normalise));
        let outputs = self.session.run(&[(
            self.names.input.as_str(),
            &[1, 3, fed_height, fed_width],
            planes.as_slice(),
        )])?;

        let (_, shape, read) = outputs
            .iter()
            .find(|(name, _, _)| *name == self.names.output)
            .ok_or(MlError::ShapeMismatch)?;
        // `[1, h, w]` is what Depth Anything emits and `[1, 1, h, w]` is what
        // a re-export of it sometimes does, so the plane is read off the last
        // two dimensions rather than off a fixed rank.
        let (out_height, out_width) = match shape.as_slice() {
            [.., h, w] if *h > 0 && *w > 0 => (*h, *w),
            _ => return Err(MlError::ShapeMismatch),
        };
        if read.len() < out_width * out_height {
            return Err(MlError::ShapeMismatch);
        }
        Ok(Plane {
            width: u32::try_from(out_width).map_err(|_| MlError::ShapeMismatch)?,
            height: u32::try_from(out_height).map_err(|_| MlError::ShapeMismatch)?,
            data: quantise(
                read.get(..out_width * out_height).unwrap_or(&[]),
                self.names.output_kind,
            ),
        })
    }
}

/// What the installed depth pack is, without opening it, or `None` when none
/// is installed or nothing on this machine could run one.
#[must_use]
pub fn installed_identity() -> Option<[u8; 32]> {
    store::installed_identity(Task::Depth)
}

/// Squash a model's own numbers into the frame's own 0..65535, nearer larger.
///
/// The model's output has no unit and no zero, so the only honest scale is the
/// frame's own: its smallest number is 0 and its largest 65535. A frame where
/// everything is the same distance, and one the model answered with nothing at
/// all, both come back flat rather than as a full-range picture of rounding
/// noise.
fn quantise(read: &[f32], kind: DepthKind) -> Vec<u16> {
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for value in read.iter().copied().filter(|v| v.is_finite()) {
        low = low.min(value);
        high = high.max(value);
    }
    let span = high - low;
    if !span.is_finite() || span <= 0.0 {
        return vec![0u16; read.len()];
    }
    read.iter()
        .map(|value| {
            let at = if value.is_finite() {
                (value - low) / span
            } else {
                0.0
            };
            // Inverse relative is already nearer-larger; a metric model
            // measures the other way and is turned round here so a plane
            // always means the same thing to the draw.
            let at = match kind {
                DepthKind::InverseRelative => at,
                DepthKind::Metric => 1.0 - at,
            };
            (at.clamp(0.0, 1.0) * f32::from(u16::MAX) + 0.5) as u16
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A manifest for a depth pack of `bytes` bytes sitting in a folder.
    fn a_depth_manifest(bytes: u64) -> String {
        let digest = "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c";
        format!(
            r#"{{"format":1,"id":"depth-anything-v2-small","kind":"model",
               "name":"Depth Anything V2 Small","version":"1.0",
               "licence":"Apache-2.0","size":{bytes},
               "platforms":{{"any":{{"downloads":[
                 {{"url":"https://example.invalid/model.onnx","sha256":"{digest}",
                   "size":{bytes},"unpack":"file","dest":"model.onnx"}}]}}}},
               "model":{{"task":"depth","arch":"depth-anything","file":"model.onnx",
                         "input":"pixel_values","output":"predicted_depth",
                         "size":518,"multiple":14}}}}"#
        )
    }

    /// **The quantise puts the nearest thing at the top and the furthest at
    /// the bottom.** The model's numbers mean nothing on their own, so the one
    /// promise the plane makes is this one, and the draw reads it as brightness.
    #[test]
    fn the_plane_puts_the_nearest_thing_highest() {
        let read = [0.5f32, 2.0, 1.0, 4.0];
        let near = quantise(&read, DepthKind::InverseRelative);
        assert_eq!(near, [0, 28086, 9362, 65535]);

        // A metric model measures the other way round and is turned over.
        let far = quantise(&read, DepthKind::Metric);
        assert_eq!(far, [65535, 37449, 56173, 0]);
    }

    /// **A flat frame is flat, and so is a frame of nonsense.** Stretching
    /// either over the whole range would paint rounding noise as depth.
    #[test]
    fn a_frame_with_no_range_comes_back_flat() {
        assert_eq!(
            quantise(&[3.0, 3.0, 3.0], DepthKind::InverseRelative),
            [0; 3]
        );
        assert_eq!(quantise(&[], DepthKind::InverseRelative), Vec::<u16>::new());
        assert_eq!(
            quantise(&[f32::NAN, f32::NAN], DepthKind::InverseRelative),
            [0; 2]
        );
    }

    /// **With nothing installed, opening the pack is a refusal that says which
    /// is missing.** The badge's detail is chosen off this, so "install the
    /// runtime" and "install a depth pack" must not be the same answer.
    #[test]
    fn opening_with_nothing_installed_refuses_by_name() {
        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        store::with_dir(Some(root.path().to_path_buf()));
        let refusal = Depth::open().unwrap_err();
        assert!(
            matches!(
                refusal,
                MlError::RuntimeMissing | MlError::PackMissing(Task::Depth)
            ),
            "{refusal:?}"
        );
        assert!(
            installed_identity().is_none(),
            "and there is nothing to name"
        );
        store::with_dir(None);
    }

    /// Where the real Depth Anything file sits on the reference machine.
    fn real_depth() -> Option<PathBuf> {
        let packs = crate::test_support::packs_dir()?;
        let file = packs.join("depth-anything-v2-small.onnx");
        file.is_file().then_some(file)
    }

    /// A dark frame with a bright square in the middle of it.
    fn square(side: usize) -> Vec<u8> {
        let mut rgba = vec![0u8; 4 * side * side];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        for y in side / 4..side * 3 / 4 {
            for x in side / 4..side * 3 / 4 {
                let at = 4 * (y * side + x);
                rgba[at..at + 3].copy_from_slice(&[230, 230, 230]);
            }
        }
        rgba
    }

    /// **On the reference machine, the real model reads a frame and hands back
    /// a plane that is not all one number.**
    ///
    /// A model is not held to a picture here: what a depth model makes of a
    /// bright square on a dark ground is its own business, and pinning it
    /// would be pinning the weights. What is pinned is the shape of the answer
    /// and that the frame reached the graph at all. Skipped politely wherever
    /// the runtime and the pack are not installed, which is every CI runner.
    #[test]
    fn the_real_model_reads_a_frame_into_a_plane() {
        let (Some(runtime_dir), Some(model)) = (crate::test_support::runtime_dir(), real_depth())
        else {
            runtime::no_runtime();
            return;
        };
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let _serial = crate::test_support::serially();
        // A pack of its own, so nothing here reads or writes the user's.
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("depth-anything-v2-small");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::copy(&model, home.join("model.onnx")).unwrap();
        let size = std::fs::metadata(home.join("model.onnx")).unwrap().len();
        std::fs::write(home.join(store::MANIFEST_FILE), a_depth_manifest(size)).unwrap();
        store::with_dir(Some(root.path().to_path_buf()));

        let mut depth = Depth::open().expect("the pack would not open");
        assert_eq!(
            depth.identity(),
            installed_identity().expect("the pack is installed"),
            "the open pack and the key agree about which pack it is"
        );
        assert_eq!(
            depth.pack(),
            "depth-anything-v2-small",
            "the provenance a sidecar keeps cannot name the pack that made it"
        );

        let side = 256u32;
        let plane = depth.run(&square(side as usize), side, side).unwrap();
        assert_eq!(
            plane.data.len(),
            (plane.width as usize) * (plane.height as usize),
            "one number per pixel of the plane's own raster"
        );
        assert_eq!(plane.width % 14, 0, "the model's own tile");
        assert_eq!(plane.height % 14, 0);
        assert!(
            plane.data.iter().any(|v| *v != plane.data[0]),
            "the whole frame came back at one distance"
        );
        assert_eq!(
            plane.data.iter().copied().max(),
            Some(u16::MAX),
            "the nearest pixel is the top of the range"
        );
        assert_eq!(
            plane.data.iter().copied().min(),
            Some(0),
            "and the furthest the bottom"
        );
        store::with_dir(None);
    }

    /// **What a depth frame costs at 1080p**, printed and never gated
    /// (13-PERFORMANCE-RULES B18, docs/impl/addons.md §8).
    ///
    /// Ignored, so it runs when somebody asks for it: it needs the runtime and
    /// the pack, and a number measured on a runner with neither would be a
    /// number about nothing. The first run of a session compiles the graph and
    /// costs about a second, so it is thrown away and the warm frames are what
    /// is reported.
    #[test]
    #[ignore = "needs the model runtime and the depth pack"]
    fn what_a_depth_frame_costs_at_1080p() {
        let (Some(runtime_dir), Some(model)) = (crate::test_support::runtime_dir(), real_depth())
        else {
            runtime::no_runtime();
            return;
        };
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("depth-anything-v2-small");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::copy(&model, home.join("model.onnx")).unwrap();
        let size = std::fs::metadata(home.join("model.onnx")).unwrap().len();
        std::fs::write(home.join(store::MANIFEST_FILE), a_depth_manifest(size)).unwrap();
        store::with_dir(Some(root.path().to_path_buf()));

        let (w, h) = (1920u32, 1080u32);
        let frame = {
            let mut rgba = vec![0u8; 4 * (w as usize) * (h as usize)];
            for (at, pixel) in rgba.chunks_exact_mut(4).enumerate() {
                let v = (at % 251) as u8;
                pixel.copy_from_slice(&[v, 255 - v, v / 2, 255]);
            }
            rgba
        };
        let mut depth = Depth::open().expect("the pack would not open");
        depth.run(&frame, w, h).expect("the warm-up frame");

        let runs = 5;
        let started = std::time::Instant::now();
        for _ in 0..runs {
            depth.run(&frame, w, h).expect("a frame");
        }
        let each = started.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
        eprintln!(
            "depth, {w} by {h}, {}: {each:.1} ms a frame",
            depth.provider()
        );
        store::with_dir(None);
    }
}
