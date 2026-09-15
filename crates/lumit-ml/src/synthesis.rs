//! The frame between two real ones, painted by a model (docs/impl/addons.md
//! §6.3).
//!
//! Retime's Flow already invents that frame by measuring how everything moved
//! and warping the two neighbours along the measurement. A synthesis model
//! paints it instead: two frames and a phase in, one frame out, no motion
//! field anywhere. That is why it slots in at the synthesis level and nowhere
//! else, and why Motion blur and Datamosh keep the built-in engine's vectors
//! whatever a layer's engine row says.
//!
//! # Thread role and contract
//!
//! A [`Synthesis`] owns its session and is never shared: a run is an FFI call
//! that may hold the GPU, and 14-ENGINEERING-RULES §1.3 forbids holding a lock
//! across one. The decode pool that owns it runs one frame at a time on its own
//! thread. Nothing here is cancellable part way; a single run of one frame is
//! the smallest unit, exactly as §9 says.

use crate::{
    error::MlError,
    manifest::{Arch, Rife, Task},
    runtime,
    session::{Prefer, Session},
    store, tensor,
};

/// An open synthesis model, ready to paint in-between frames.
#[derive(Debug)]
pub struct Synthesis {
    session: Session,
    /// What the pack's manifest calls the tensors, and how far the graph
    /// wants the frame grown before it will take it.
    names: Rife,
    identity: [u8; 32],
}

impl Synthesis {
    /// Open the installed synthesis pack.
    ///
    /// # Errors
    ///
    /// [`MlError::RuntimeMissing`] when no model runtime is installed,
    /// [`MlError::PackMissing`] when nothing installed does synthesis, and
    /// whatever [`Session::open`] answers for a pack that is there and will
    /// not open. Each is a calm sentence on the Flow group's engine row, never
    /// a silent fall back to the built-in engine (§9, and docs/08 §3.1's rule
    /// that a chosen engine is never quietly swapped).
    pub fn open() -> Result<Self, MlError> {
        if matches!(runtime::status(), runtime::RuntimeStatus::Missing) {
            return Err(MlError::RuntimeMissing);
        }
        let installed =
            store::find(Task::Synthesis).ok_or(MlError::PackMissing(Task::Synthesis))?;
        let Some(Arch::Rife(names)) = installed.manifest.model.as_ref().map(|model| &model.arch)
        else {
            // The manifest parser holds task and arch to each other, so this
            // is a pack claiming synthesis with a family that does something
            // else, which only a hand-written manifest can be.
            return Err(MlError::PackUnreadable);
        };
        let names = names.clone();
        let identity = store::identity(&installed.manifest);
        let session = Session::open(&installed.path, &names.file, Prefer::Platform)?;
        Ok(Synthesis {
            session,
            names,
            identity,
        })
    }

    /// What this pack is, for the frame key every picture it paints is filed
    /// under (§7). Two packs, or two versions of one, never share it.
    #[must_use]
    pub fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// Which provider took the session: part of the same answer, and what the
    /// Addons page's runtime row shows.
    #[must_use]
    pub fn provider(&self) -> &'static str {
        self.session.provider()
    }

    /// Paint the frame `phi` of the way from `a` to `b`, both RGBA bytes at
    /// `width` by `height`.
    ///
    /// Alpha is dropped on the way in and comes back opaque: the model was
    /// trained on three channels and has nothing to say about a fourth.
    ///
    /// # Errors
    ///
    /// [`MlError::ShapeMismatch`] when the model hands back something that is
    /// not the frame that was asked for, and [`MlError::ModelFailed`] with
    /// ONNX Runtime's own words for a run it refuses.
    pub fn synthesise(
        &mut self,
        a: &[u8],
        b: &[u8],
        width: usize,
        height: usize,
        phi: f32,
    ) -> Result<Vec<u8>, MlError> {
        if let Some(frame) = endpoint(a, b, phi) {
            return Ok(frame.to_vec());
        }
        if width == 0 || height == 0 {
            return Err(MlError::ShapeMismatch);
        }
        // Grown to what the graph's own downsampling divides evenly, by
        // repeating the edge: a black border is an edge the model sees and
        // invents motion along.
        let multiple = self.names.multiple as usize;
        let (first, grown_width, grown_height) = tensor::pad(
            &tensor::pack_u8(a, width, height, None),
            width,
            height,
            3,
            multiple,
        );
        let (second, ..) = tensor::pad(
            &tensor::pack_u8(b, width, height, None),
            width,
            height,
            3,
            multiple,
        );
        let shape = [1, 3, grown_height, grown_width];
        let step = [phi];
        let outputs = self.session.run(&[
            (self.names.img0.as_str(), &shape, first.as_slice()),
            (self.names.img1.as_str(), &shape, second.as_slice()),
            (self.names.timestep.as_str(), &[1], &step),
        ])?;

        let (_, out_shape, painted) = outputs
            .iter()
            .find(|(name, _, _)| *name == self.names.output)
            .ok_or(MlError::ShapeMismatch)?;
        if painted.len() < 3 * grown_width * grown_height || out_shape.len() != 4 {
            return Err(MlError::ShapeMismatch);
        }
        let cropped = tensor::crop(painted, grown_width, grown_height, width, height, 3);
        Ok(tensor::unpack_u8(&cropped, width, height, None))
    }
}

/// The frame already in hand when `phi` is at either end.
///
/// At or below zero the answer is the first frame and at or above one it is
/// the second, bit for bit, and the model is never asked for a picture that is
/// already in hand. The built-in engine promises exactly this, so both engines
/// answer the same at the ends whatever else they disagree about
/// (docs/impl/addons.md §6.3). A phase that is not a number is not a phase at
/// all, and it reads as the first frame.
#[must_use]
pub fn endpoint<'f>(a: &'f [u8], b: &'f [u8], phi: f32) -> Option<&'f [u8]> {
    if phi.is_nan() || phi <= 0.0 {
        return Some(a);
    }
    if phi >= 1.0 {
        return Some(b);
    }
    None
}

/// What the installed synthesis pack is, without opening it, or `None` when
/// none is installed or nothing on this machine could run one.
///
/// [`store::installed_identity`] does the work and remembers the answer; this
/// is the synthesis tier's door to it, and the one the frame key calls.
#[must_use]
pub fn installed_identity() -> Option<[u8; 32]> {
    store::installed_identity(Task::Synthesis)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A manifest for a RIFE pack of `bytes` bytes sitting in a folder.
    fn a_rife_manifest(id: &str, version: &str, bytes: u64, digest: &str) -> String {
        format!(
            r#"{{"format":1,"id":"{id}","kind":"model","name":"RIFE",
               "version":"{version}","licence":"MIT","size":{bytes},
               "platforms":{{"any":{{"downloads":[
                 {{"url":"https://example.invalid/rife.onnx","sha256":"{digest}",
                   "size":{bytes},"unpack":"file","dest":"rife.onnx"}}]}}}},
               "model":{{"task":"synthesis","arch":"rife","file":"rife.onnx",
                         "img0":"img0","img1":"img1","timestep":"timestep",
                         "output":"output","multiple":32}}}}"#
        )
    }

    /// **A phase at either end never reaches the model.** The two frames are
    /// already in hand, so asking for one of them is a copy, and it is the one
    /// promise the built-in engine and a model pack both have to keep or the
    /// ends of a ramp would flicker as the engine row changed.
    #[test]
    fn a_phase_at_either_end_is_the_frame_in_hand() {
        let a = [1u8, 2, 3, 255];
        let b = [9u8, 8, 7, 255];
        for at in [0.0f32, -0.0, -1.0, f32::NAN] {
            assert_eq!(endpoint(&a, &b, at), Some(a.as_slice()), "{at}");
        }
        for at in [1.0f32, 1.5, f32::INFINITY] {
            assert_eq!(endpoint(&a, &b, at), Some(b.as_slice()), "{at}");
        }
        for at in [0.001f32, 0.5, 0.999] {
            assert_eq!(endpoint(&a, &b, at), None, "{at}");
        }
    }

    /// **Two packs, two versions and two digests each name themselves
    /// differently, and the same pack names itself the same way twice.**
    /// Every frame a model paints is filed under this, so a collision serves
    /// one pack's picture for another's (§7).
    #[test]
    fn a_pack_names_itself_stably_and_apart_from_every_other() {
        let one = "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c";
        let two = "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10d";
        let of = |id: &str, version: &str, digest: &str| {
            store::identity(
                &crate::manifest::parse(&a_rife_manifest(id, version, 12, digest)).unwrap(),
            )
        };
        let base = of("rife", "1.0", one);
        assert_eq!(base, of("rife", "1.0", one), "the same pack twice");
        assert_ne!(base, of("rife", "1.1", one), "a newer version");
        assert_ne!(base, of("rife-alt", "1.0", one), "another pack");
        assert_ne!(base, of("rife", "1.0", two), "another file");
        assert_ne!(base, [0u8; 32], "and it is not nothing");
    }

    /// **With nothing installed, opening the pack is a refusal that says which
    /// is missing.** The row's sentence is chosen off this, so "install the
    /// runtime" and "install RIFE" must not be the same answer.
    #[test]
    fn opening_with_nothing_installed_refuses_by_name() {
        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        store::with_dir(Some(root.path().to_path_buf()));
        let refusal = Synthesis::open().unwrap_err();
        assert!(
            matches!(
                refusal,
                MlError::RuntimeMissing | MlError::PackMissing(Task::Synthesis)
            ),
            "{refusal:?}"
        );
        assert!(
            installed_identity().is_none(),
            "and there is nothing to name"
        );
        store::with_dir(None);
    }

    /// **A pack with nothing to run it names nothing.** The frame key is what
    /// this answer becomes, so naming the pack while the built-in engine is the
    /// thing actually painting files those frames under a model that never
    /// touched them, and installing the runtime then serves them straight back
    /// (§7).
    #[test]
    fn a_pack_with_no_runtime_to_run_it_names_nothing() {
        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("rife");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("rife.onnx"), [0u8; 12]).unwrap();
        let digest = "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c";
        std::fs::write(
            home.join(store::MANIFEST_FILE),
            a_rife_manifest("rife", "4.9", 12, digest),
        )
        .unwrap();
        store::with_dir(Some(root.path().to_path_buf()));

        let named = installed_identity();
        let runtime = runtime::status();
        // And twice, because the answer is remembered between the two.
        let again = installed_identity();
        store::with_dir(None);

        assert_eq!(named, again, "the same folder is the same answer");
        // Both endings are written down, so the test is true on a machine
        // carrying the runtime as well as on one without it.
        if matches!(runtime, runtime::RuntimeStatus::Missing) {
            assert!(
                named.is_none(),
                "nothing on this machine can run the pack, so nothing is named"
            );
        } else {
            assert!(named.is_some(), "the pack is installed and it can run");
        }
    }

    /// Where the real RIFE file sits on the reference machine.
    fn real_rife() -> Option<PathBuf> {
        let packs = crate::test_support::packs_dir()?;
        let file = packs.join("rife49_ensemble_True_scale_1_sim.onnx");
        file.is_file().then_some(file)
    }

    /// A frame with a white square whose top-left corner is at `x`.
    fn square(x: usize, side: usize) -> Vec<u8> {
        let mut rgba = vec![0u8; 4 * side * side];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        for y in 100..140 {
            for column in x..x + 40 {
                let at = 4 * (y * side + column);
                rgba[at..at + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        rgba
    }

    /// Where the bright pixels of a frame have their middle, horizontally.
    fn centre(rgba: &[u8], side: usize) -> f64 {
        let mut sum = 0.0;
        let mut count = 0.0;
        for (index, pixel) in rgba.chunks_exact(4).enumerate() {
            if pixel[0] > 128 {
                sum += (index % side) as f64;
                count += 1.0;
            }
        }
        if count == 0.0 {
            return -1.0;
        }
        sum / count
    }

    /// **On the reference machine, RIFE paints the frame in the middle.**
    ///
    /// A square moved eight pixels between two frames: at half way the model
    /// has to put it four pixels along, which no copy of either input and no
    /// crossfade of the two does. Skipped politely wherever the runtime and
    /// the pack are not installed, which is every CI runner.
    #[test]
    fn the_real_model_paints_the_square_between_the_two_frames() {
        let (Some(runtime_dir), Some(model)) = (crate::test_support::runtime_dir(), real_rife())
        else {
            runtime::no_runtime();
            return;
        };
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let _serial = crate::test_support::serially();
        // A pack of its own, so nothing here reads or writes the user's.
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("rife");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::copy(&model, home.join("rife.onnx")).unwrap();
        let size = std::fs::metadata(home.join("rife.onnx")).unwrap().len();
        let digest = "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c";
        std::fs::write(
            home.join(store::MANIFEST_FILE),
            a_rife_manifest("rife", "4.9", size, digest),
        )
        .unwrap();
        store::with_dir(Some(root.path().to_path_buf()));

        let mut synthesis = Synthesis::open().expect("the pack would not open");
        assert_eq!(
            synthesis.identity(),
            installed_identity().expect("the pack is installed"),
            "the open pack and the key agree about which pack it is"
        );

        let side = 256;
        let (first, second) = (square(60, side), square(68, side));
        let painted = synthesis
            .synthesise(&first, &second, side, side, 0.5)
            .unwrap();
        assert_eq!(painted.len(), first.len(), "a frame of the size asked for");
        assert!(
            painted.chunks_exact(4).all(|pixel| pixel[3] == 255),
            "opaque, because the model has nothing to say about alpha"
        );

        let (was, is, now) = (
            centre(&first, side),
            centre(&painted, side),
            centre(&second, side),
        );
        assert!(
            is > was + 1.0 && is < now - 1.0,
            "the square landed at {is}, and the two frames have it at {was} and {now}"
        );

        // The ends are the frames themselves, through the same door.
        assert_eq!(
            synthesis
                .synthesise(&first, &second, side, side, 0.0)
                .unwrap(),
            first
        );
        assert_eq!(
            synthesis
                .synthesise(&first, &second, side, side, 1.0)
                .unwrap(),
            second
        );
        store::with_dir(None);
    }
}
