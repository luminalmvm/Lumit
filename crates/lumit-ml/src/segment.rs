//! The thing that was tapped on, traced (docs/impl/addons.md §6.2).
//!
//! SAM 2 is two graphs rather than one. The **encoder** reads a frame and hands
//! back three tensors describing it, which is nearly all of the cost; the
//! **decoder** reads those three plus a handful of taps and hands back three
//! candidate masks with a score each, which is cheap enough to run again every
//! time the user adds a tap. So a frame is encoded once and kept for as long as
//! the run needs it, and a second tap on the same frame pays the decoder alone.
//!
//! What comes back from [`Segment::mask`] is a **probability** per pixel at the
//! frame's own raster: 1 where the model is sure the tapped thing is, 0 where
//! it is sure it is not, and the soft band between the two where it is not
//! sure. That is the shape the Roto brush's seeding reads, and the band is
//! exactly what it declines to seed.
//!
//! The word mask is the model's own and stays inside the engine. What the user
//! reads says matte.
//!
//! # Thread role and contract
//!
//! A [`Segment`] owns its two sessions and is never shared: a run is an FFI
//! call that may hold the GPU, and 14-ENGINEERING-RULES §1.3 forbids holding a
//! lock across one. The propagation opens one on its own thread and drops it
//! when the run ends. A single run is not interruptible; cancellation happens
//! between frames, in the caller's own loop.

use crate::{
    error::MlError,
    manifest::{Arch, Sam2, Task},
    runtime,
    session::{Prefer, Session},
    store, tensor,
};

/// One frame as the encoder describes it, ready for as many taps as the user
/// cares to make.
///
/// The three tensors are kept whole: the decoder wants the embedding **and**
/// both finer feature maps, and handing it the embedding alone is the mistake
/// §13 writes down.
#[derive(Debug, Clone)]
pub struct Embedding {
    /// The encoder's three outputs, in the order the decoder is fed them:
    /// the embedding first, then the two high-resolution feature maps.
    feats: [(Vec<usize>, Vec<f32>); 3],
    /// The frame this was made of.
    width: usize,
    height: usize,
    /// How much of the model's square the frame was fitted into, the rest of
    /// it being padding a mask must be cropped back off.
    fed: (usize, usize),
}

/// An open segmentation model: the encoder, the decoder, and what the pack is.
#[derive(Debug)]
pub struct Segment {
    encoder: Session,
    decoder: Session,
    names: Sam2,
    identity: [u8; 32],
    pack: String,
}

impl Segment {
    /// Open the installed segmentation pack, both graphs.
    ///
    /// # Errors
    ///
    /// [`MlError::RuntimeMissing`] when no model runtime is installed,
    /// [`MlError::PackMissing`] when nothing installed does segmentation, and
    /// whatever [`Session::open`] answers for a pack that is there and will not
    /// open. Each is a calm sentence on the effect's card, never a matte
    /// invented to fill the gap (§9).
    pub fn open() -> Result<Self, MlError> {
        if matches!(runtime::status(), runtime::RuntimeStatus::Missing) {
            return Err(MlError::RuntimeMissing);
        }
        let installed =
            store::find(Task::Segmentation).ok_or(MlError::PackMissing(Task::Segmentation))?;
        let Some(Arch::Sam2(names)) = installed.manifest.model.as_ref().map(|model| &model.arch)
        else {
            // The manifest parser holds task and arch to each other, so this is
            // a pack claiming segmentation with a family that does something
            // else, which only a hand-written manifest can be.
            return Err(MlError::PackUnreadable);
        };
        let names = names.clone();
        let identity = store::identity(&installed.manifest);
        let pack = installed.manifest.id.clone();
        let encoder = Session::open(&installed.path, &names.encoder, Prefer::Platform)?;
        let decoder = Session::open(&installed.path, &names.decoder, Prefer::Platform)?;
        Ok(Segment {
            encoder,
            decoder,
            names,
            identity,
            pack,
        })
    }

    /// What this pack is, for the key every matte it seeds is filed under (§7).
    /// Two packs, or two versions of one, never share it.
    #[must_use]
    pub fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// Which pack this is, by the id its folder carries, for the provenance a
    /// sidecar keeps (§7).
    #[must_use]
    pub fn pack(&self) -> &str {
        &self.pack
    }

    /// Which provider took the sessions: part of the same answer (§8).
    #[must_use]
    pub fn provider(&self) -> &'static str {
        self.encoder.provider()
    }

    /// Read one frame of RGBA bytes into the description the decoder works
    /// from.
    ///
    /// The frame is fitted to the model's square on its long side, padded out
    /// to the square by repeating its edge, and normalised. Padding rather than
    /// stretching keeps a tap where the user put it: the same scale on both
    /// axes means the point arithmetic below is one multiplication and not a
    /// guess about which way the picture was squashed.
    ///
    /// # Errors
    ///
    /// [`MlError::ShapeMismatch`] for a frame of no size and for an encoder
    /// that hands back something other than its three tensors;
    /// [`MlError::ModelFailed`] with ONNX Runtime's own words for a run it
    /// refuses.
    pub fn embed(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<Embedding, MlError> {
        let (width, height) = (width as usize, height as usize);
        if width == 0 || height == 0 {
            return Err(MlError::ShapeMismatch);
        }
        let size = (self.names.size as usize).max(1);
        let (fed_width, fed_height) = tensor::fit(width, height, size, 1);
        let small = tensor::resample_u8(rgba, width, height, fed_width, fed_height);
        let planes = tensor::pack_u8(&small, fed_width, fed_height, Some(self.names.normalise));
        let (square, _, _) = tensor::pad(&planes, fed_width, fed_height, 3, size);
        let outputs = self.encoder.run(&[(
            self.names.image.as_str(),
            &[1, 3, size, size],
            square.as_slice(),
        )])?;

        let wanted = [
            self.names.embed.as_str(),
            self.names.high_res[0].as_str(),
            self.names.high_res[1].as_str(),
        ];
        let mut feats: [Option<(Vec<usize>, Vec<f32>)>; 3] = Default::default();
        for (name, shape, numbers) in outputs {
            if let Some(at) = wanted.iter().position(|want| *want == name) {
                if let Some(slot) = feats.get_mut(at) {
                    *slot = Some((shape, numbers));
                }
            }
        }
        let [embed, high_res_0, high_res_1] = feats;
        let (Some(embed), Some(high_res_0), Some(high_res_1)) = (embed, high_res_0, high_res_1)
        else {
            return Err(MlError::ShapeMismatch);
        };
        Ok(Embedding {
            feats: [embed, high_res_0, high_res_1],
            width,
            height,
            fed: (fed_width, fed_height),
        })
    }

    /// Ask the decoder what the taps point at, and hand back its answer at the
    /// frame's own raster, as a probability per pixel.
    ///
    /// No mask is fed back in: this is the first ask about these taps, so the
    /// slot is a square of noughts and the flag beside it says to ignore it.
    /// Of the three candidates the model offers, the one it scores highest is
    /// the answer, which is what its own score is for.
    ///
    /// # Errors
    ///
    /// [`MlError::ShapeMismatch`] for no taps at all, for a label per tap that
    /// is not there, and for a decoder that hands back something that is not a
    /// square of numbers; [`MlError::ModelFailed`] with ONNX Runtime's own
    /// words for a run it refuses.
    pub fn mask(
        &mut self,
        frame: &Embedding,
        points: &[(f32, f32)],
        labels: &[u8],
    ) -> Result<Vec<f32>, MlError> {
        if points.is_empty() || points.len() != labels.len() {
            return Err(MlError::ShapeMismatch);
        }
        let size = (self.names.size as usize).max(1);
        let (fed_width, fed_height) = frame.fed;
        // The taps travel with the frame: the same fit, so a tap lands on the
        // pixel of the square the pixel it was made on became.
        let to_x = fed_width as f32 / frame.width as f32;
        let to_y = fed_height as f32 / frame.height as f32;
        let coords: Vec<f32> = points
            .iter()
            .flat_map(|(x, y)| [x * to_x, y * to_y])
            .collect();
        let marks: Vec<f32> = labels
            .iter()
            .map(|label| if *label == 0 { 0.0 } else { 1.0 })
            .collect();
        let empty = vec![0.0f32; MASK_SIDE * MASK_SIDE];
        let none = [0.0f32];

        let [embed, high_res_0, high_res_1] = &frame.feats;
        let outputs = self.decoder.run(&[
            (self.names.embed.as_str(), &embed.0, &embed.1),
            (
                self.names.high_res[0].as_str(),
                &high_res_0.0,
                &high_res_0.1,
            ),
            (
                self.names.high_res[1].as_str(),
                &high_res_1.0,
                &high_res_1.1,
            ),
            (
                self.names.point_coords.as_str(),
                &[1, points.len(), 2],
                coords.as_slice(),
            ),
            (
                self.names.point_labels.as_str(),
                &[1, points.len()],
                marks.as_slice(),
            ),
            (
                self.names.mask_input.as_str(),
                &[1, 1, MASK_SIDE, MASK_SIDE],
                empty.as_slice(),
            ),
            (self.names.has_mask_input.as_str(), &[1], &none),
        ])?;

        let (_, shape, logits) = outputs
            .iter()
            .find(|(name, _, _)| *name == self.names.masks)
            .ok_or(MlError::ShapeMismatch)?;
        let (side_height, side_width) = match shape.as_slice() {
            [.., h, w] if *h > 0 && *w > 0 => (*h, *w),
            _ => return Err(MlError::ShapeMismatch),
        };
        let candidates = logits.len() / (side_width * side_height).max(1);
        let scores = outputs
            .iter()
            .find(|(name, _, _)| *name == self.names.iou)
            .map(|(_, _, numbers)| numbers.as_slice())
            .unwrap_or(&[]);
        let best = best_of(candidates, scores);
        let chosen = logits
            .get(best * side_width * side_height..(best + 1) * side_width * side_height)
            .ok_or(MlError::ShapeMismatch)?;

        // The candidate covers the whole padded square, so a pixel of the frame
        // is read at where it sits inside the part of that square the frame was
        // fitted into. Growing the candidate to the square and cutting the
        // padding back off is the same arithmetic, done in one pass rather than
        // through a four-megabyte intermediate the caller would never see
        // (14-ENGINEERING-RULES §5).
        // Frame pixels to square pixels, then square pixels to the candidate's
        // own grid, with the half-pixel offset a bilinear read wants.
        let across = fed_width as f32 / frame.width as f32 * (side_width as f32 / size as f32);
        let down = fed_height as f32 / frame.height as f32 * (side_height as f32 / size as f32);
        let mut out = vec![0.0f32; frame.width * frame.height];
        for y in 0..frame.height {
            let source_y = (y as f32 + 0.5) * down - 0.5;
            for x in 0..frame.width {
                let source_x = (x as f32 + 0.5) * across - 0.5;
                let value = sample(chosen, side_width, side_height, source_x, source_y);
                if let Some(slot) = out.get_mut(y * frame.width + x) {
                    *slot = sigmoid(value);
                }
            }
        }
        Ok(out)
    }
}

/// The square the decoder answers in, and the square of nothing it is handed
/// when there is no earlier mask to refine.
const MASK_SIDE: usize = 256;

/// What the installed segmentation pack is, without opening it, or `None` when
/// none is installed or nothing on this machine could run one.
#[must_use]
pub fn installed_identity() -> Option<[u8; 32]> {
    store::installed_identity(Task::Segmentation)
}

/// Which of the candidates the model thinks best, by its own score. With no
/// scores at all the first is taken, which is the order the model emits them
/// in anyway.
fn best_of(candidates: usize, scores: &[f32]) -> usize {
    let mut best = 0usize;
    let mut top = f32::NEG_INFINITY;
    for at in 0..candidates {
        let score = scores.get(at).copied().unwrap_or(0.0);
        if score.is_finite() && score > top {
            top = score;
            best = at;
        }
    }
    best
}

/// Bilinear read of one candidate, clamped at its edges.
fn sample(plane: &[f32], width: usize, height: usize, x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, (width.saturating_sub(1)) as f32);
    let y = y.clamp(0.0, (height.saturating_sub(1)) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width.saturating_sub(1));
    let y1 = (y0 + 1).min(height.saturating_sub(1));
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let at = |xi: usize, yi: usize| plane.get(yi * width + xi).copied().unwrap_or(0.0);
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
    top + (bottom - top) * ty
}

/// The logistic curve: what turns "how sure the model is" into "how much of the
/// pixel", the same step BiRefNet's coverage goes through.
fn sigmoid(value: f32) -> f32 {
    if value.is_finite() {
        1.0 / (1.0 + (-value).exp())
    } else {
        0.0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// **The highest score wins, and a model that scores nothing still
    /// answers.** Picking the wrong candidate is the difference between the
    /// thing that was tapped and the whole object it is part of.
    #[test]
    fn the_candidate_the_model_thinks_best_is_the_one_taken() {
        assert_eq!(best_of(3, &[0.1, 0.9, 0.4]), 1);
        assert_eq!(best_of(3, &[0.9, 0.1, 0.4]), 0);
        assert_eq!(best_of(3, &[]), 0, "no scores is the first candidate");
        assert_eq!(best_of(3, &[f32::NAN, 0.2, 0.1]), 1, "a score that is not");
        assert_eq!(best_of(0, &[0.5]), 0);
    }

    /// **The probability a seed threshold reads means what it says.** Nought is
    /// half, sure is all, and a number that is not one covers nothing.
    #[test]
    fn a_logit_becomes_the_coverage_it_means() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
        assert!(sigmoid(20.0) > 0.99, "certain is nearly all of the pixel");
        assert!(sigmoid(-20.0) < 0.01, "and certainly not is nearly none");
        assert_eq!(sigmoid(f32::NAN), 0.0);
    }

    /// **With nothing installed, opening the pack is a refusal that says which
    /// is missing.** The Roto brush's card reads one of these, so "install the
    /// runtime" and "install a segmentation pack" must not be one answer.
    #[test]
    fn opening_with_nothing_installed_refuses_by_name() {
        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        store::with_dir(Some(root.path().to_path_buf()));
        let refusal = Segment::open().unwrap_err();
        assert!(
            matches!(
                refusal,
                MlError::RuntimeMissing | MlError::PackMissing(Task::Segmentation)
            ),
            "{refusal:?}"
        );
        assert!(
            installed_identity().is_none(),
            "and there is nothing to name"
        );
        store::with_dir(None);
    }

    /// A manifest for a segmentation pack whose two files are that many bytes.
    fn a_manifest(encoder: u64, decoder: u64) -> String {
        let one = "5c0f6e4bd0ee1f9dfbee4e8e5ba75d84bd3d5b93bd0e9e0e4e1c8f4b0a2d6e11";
        let two = "9a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";
        format!(
            r#"{{"format":1,"id":"sam2-tiny","kind":"model",
               "name":"Segment Anything 2.1 Tiny","version":"1.0",
               "licence":"Apache-2.0","size":{total},
               "platforms":{{"any":{{"downloads":[
                 {{"url":"https://example.invalid/encoder.onnx","sha256":"{one}",
                   "size":{encoder},"unpack":"file","dest":"encoder.onnx"}},
                 {{"url":"https://example.invalid/decoder.onnx","sha256":"{two}",
                   "size":{decoder},"unpack":"file","dest":"decoder.onnx"}}]}}}},
               "model":{{"task":"segmentation","arch":"sam2",
                         "encoder":"encoder.onnx","decoder":"decoder.onnx"}}}}"#,
            total = encoder + decoder
        )
    }

    /// Where the real SAM 2 files sit on the reference machine: a folder of
    /// their own under the packs folder, since the pack is two files rather
    /// than one.
    fn real_pack() -> Option<(PathBuf, PathBuf)> {
        let home = crate::test_support::packs_dir()?.join("sam2");
        let named = |part: &str| {
            std::fs::read_dir(&home).ok()?.flatten().find_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?.to_owned();
                (name.contains(part) && name.ends_with(".onnx")).then_some(path)
            })
        };
        Some((named("encoder")?, named("decoder")?))
    }

    /// Write the pack into `root` as the store expects to find it.
    fn a_pack(root: &Path, encoder: &Path, decoder: &Path) {
        let home = root.join("sam2-tiny");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::copy(encoder, home.join("encoder.onnx")).unwrap();
        std::fs::copy(decoder, home.join("decoder.onnx")).unwrap();
        let size = |name: &str| std::fs::metadata(home.join(name)).unwrap().len();
        std::fs::write(
            home.join(store::MANIFEST_FILE),
            a_manifest(size("encoder.onnx"), size("decoder.onnx")),
        )
        .unwrap();
        store::with_dir(Some(root.to_path_buf()));
    }

    /// A bright disc on a dark ground, and whether each pixel is inside it.
    fn disc(width: usize, height: usize) -> (Vec<u8>, Vec<bool>) {
        let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
        let radius = (width.min(height) as f32) * 0.3;
        let mut rgba = vec![0u8; 4 * width * height];
        let mut inside = vec![false; width * height];
        for y in 0..height {
            for x in 0..width {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let here = dx * dx + dy * dy <= radius * radius;
                inside[y * width + x] = here;
                let at = 4 * (y * width + x);
                let colour: [u8; 4] = if here {
                    [235, 225, 205, 255]
                } else {
                    [24, 26, 34, 255]
                };
                rgba[at..at + 4].copy_from_slice(&colour);
            }
        }
        (rgba, inside)
    }

    /// **On the reference machine, one tap in the middle of a disc traces the
    /// disc.**
    ///
    /// The model is held to a shape rather than to a picture: what SAM 2 makes
    /// of a shot is its own business and pinning it would be pinning the
    /// weights. What is pinned is that the tap reached the right place, that
    /// the answer comes back at the frame's own raster, and that it covers the
    /// thing that was tapped and not the ground around it, which is the whole
    /// of what the seeding reads. The frame is deliberately not square, so a
    /// padding or a point scale that lost an axis cannot pass.
    ///
    /// Skipped politely wherever the runtime and the pack are not installed,
    /// which is every CI runner.
    #[test]
    fn the_real_model_traces_the_thing_that_was_tapped() {
        let (Some(runtime_dir), Some((encoder, decoder))) =
            (crate::test_support::runtime_dir(), real_pack())
        else {
            runtime::no_runtime();
            return;
        };
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        a_pack(root.path(), &encoder, &decoder);

        let mut segment = Segment::open().expect("the pack would not open");
        assert_eq!(segment.pack(), "sam2-tiny");
        assert_eq!(
            segment.identity(),
            installed_identity().expect("the pack is installed"),
            "the open pack and the key disagree about which pack it is"
        );

        let (width, height) = (512usize, 288usize);
        let (rgba, truth) = disc(width, height);
        let frame = segment
            .embed(&rgba, width as u32, height as u32)
            .expect("the encoder");
        let mask = segment
            .mask(&frame, &[(width as f32 / 2.0, height as f32 / 2.0)], &[1])
            .expect("the decoder");
        assert_eq!(mask.len(), width * height, "one number per frame pixel");

        let (mut both, mut either) = (0usize, 0usize);
        for (at, want) in truth.iter().enumerate() {
            let got = mask.get(at).copied().unwrap_or(0.0) > 0.5;
            if *want && got {
                both += 1;
            }
            if *want || got {
                either += 1;
            }
        }
        let iou = both as f64 / either.max(1) as f64;
        assert!(iou > 0.7, "the tapped disc came back as {iou:.3} of itself");

        // The second ask about the same frame pays the decoder alone, which is
        // the whole reason the embedding is a value the caller holds.
        let again = segment
            .mask(&frame, &[(width as f32 / 2.0, height as f32 / 2.0)], &[1])
            .expect("the decoder again");
        assert_eq!(again, mask, "the same taps on the same frame, twice");

        // A tap list nobody can read is refused rather than guessed at.
        assert_eq!(
            segment.mask(&frame, &[], &[]).unwrap_err(),
            MlError::ShapeMismatch
        );
        assert_eq!(
            segment.mask(&frame, &[(1.0, 1.0)], &[1, 0]).unwrap_err(),
            MlError::ShapeMismatch
        );
        store::with_dir(None);
    }

    /// **What a segmentation of a 1080p frame costs**, printed and never gated
    /// (docs/impl/addons.md §8). It is in no row of 13's table, because a
    /// segmentation is one frame of one run rather than a per-frame budget.
    ///
    /// Ignored, so it runs when somebody asks for it: it needs the runtime and
    /// the pack. The encoder and the decoder are reported apart, because the
    /// first is paid once a prompted frame and the second once a tap.
    #[test]
    #[ignore = "needs the model runtime and the segmentation pack"]
    fn what_a_segmentation_costs_at_1080p() {
        let (Some(runtime_dir), Some((encoder, decoder))) =
            (crate::test_support::runtime_dir(), real_pack())
        else {
            runtime::no_runtime();
            return;
        };
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        a_pack(root.path(), &encoder, &decoder);

        let (width, height) = (1920usize, 1080usize);
        let (rgba, _) = disc(width, height);
        let taps = [(width as f32 / 2.0, height as f32 / 2.0)];
        let mut segment = Segment::open().expect("the pack would not open");
        let warm = segment
            .embed(&rgba, width as u32, height as u32)
            .expect("the warm-up frame");
        segment.mask(&warm, &taps, &[1]).expect("the warm-up tap");

        let started = std::time::Instant::now();
        let frame = segment
            .embed(&rgba, width as u32, height as u32)
            .expect("a frame");
        let encoded = started.elapsed().as_secs_f64() * 1000.0;
        let started = std::time::Instant::now();
        segment.mask(&frame, &taps, &[1]).expect("a tap");
        let decoded = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "segmentation, {width} by {height}, {}: {encoded:.1} ms a frame, \
             {decoded:.1} ms a tap",
            segment.provider()
        );
        store::with_dir(None);
    }
}
