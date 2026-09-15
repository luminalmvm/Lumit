//! How much of each pixel is the subject, guessed from the frame
//! (docs/impl/addons.md §6.1).
//!
//! Two families answer that question and they answer it differently. **Robust
//! Video Matting** reads a shot rather than a frame: it takes the picture at
//! its own size and hands back four small tensors of its own state, which the
//! next frame is fed along with the next picture, so the edge it cut last frame
//! is what it starts from this one. **BiRefNet** knows nothing of time: it
//! takes one frame squashed into a square of its own, hands back a number per
//! pixel that has to be put through a sigmoid to become coverage, and the
//! answer is stretched back to the frame's shape.
//!
//! Either way what comes out is [`Coverage`] at the frame's own raster: one
//! byte a pixel, 0 for none of the subject and 255 for all of it, which is the
//! gray8 a matte is everywhere else in Lumit.
//!
//! # Thread role and contract
//!
//! A [`Matte`] owns its session and is never shared: a run is an FFI call that
//! may hold the GPU, and 14-ENGINEERING-RULES §1.3 forbids holding a lock
//! across one. It also owns the state of the sequence it is part way through,
//! so one is opened at the first frame of a run and dropped at the last; a run
//! that starts anywhere else is a run whose first frames were read with the
//! state of a shot the model never saw (§13).

use crate::{
    error::MlError,
    manifest::{Arch, Birefnet, Rvm, Task},
    runtime,
    session::{Prefer, Session},
    store,
    tensor::{self, Detail},
};

/// One frame's coverage, at the frame's own raster.
///
/// `data` is row-major, one byte a pixel: 0 is none of the subject there and
/// 255 is all of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Which family of matte model a control asked for.
///
/// The effect's Model row names one of these and the pack that answers to it is
/// the pack that runs. Nothing falls back to the other: a person who chose
/// BiRefNet and has not got it is told so, rather than shown Robust Video
/// Matting's answer under BiRefNet's name (§2's no-surprises rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatteArch {
    /// Robust Video Matting: a sequence, its state carried frame to frame.
    #[default]
    Rvm,
    /// BiRefNet: one frame on its own, at a square of its own.
    Birefnet,
}

/// What the pack's manifest calls the tensors, per family.
#[derive(Debug)]
enum Names {
    /// Boxed because it is three times the other's size and this enum is
    /// otherwise as large as its largest arm wherever it is held.
    Rvm(Box<Rvm>),
    Birefnet(Birefnet),
}

/// Robust Video Matting's four recurrent tensors: each a shape and its numbers.
type State = [(Vec<usize>, Vec<f32>); 4];

/// What the model is handed before it has seen a frame: one nought in each,
/// shaped `[1, 1, 1, 1]`, which is how the model itself is told "this is the
/// start of the shot" (§4's tensor table).
fn first_state() -> State {
    std::array::from_fn(|_| (vec![1, 1, 1, 1], vec![0.0]))
}

/// An open matte model, ready to read frames.
#[derive(Debug)]
pub struct Matte {
    session: Session,
    names: Names,
    identity: [u8; 32],
    pack: String,
    /// How much of the frame Robust Video Matting works at, from the effect's
    /// own Detail row. BiRefNet has its own square and ignores it.
    detail: Detail,
    /// What the last frame handed back, and what the next one is fed.
    state: State,
}

impl Matte {
    /// Open the installed matte pack of the family `arch` names.
    ///
    /// # Errors
    ///
    /// [`MlError::RuntimeMissing`] when no model runtime is installed,
    /// [`MlError::PackMissing`] when nothing installed does matting with that
    /// family, and whatever [`Session::open`] answers for a pack that is there
    /// and will not open. Each is a calm sentence on the effect's badge, never
    /// a picture invented to fill the gap (§9).
    pub fn open(arch: MatteArch, detail: Detail) -> Result<Self, MlError> {
        if matches!(runtime::status(), runtime::RuntimeStatus::Missing) {
            return Err(MlError::RuntimeMissing);
        }
        let installed = installed_pack(arch).ok_or(MlError::PackMissing(Task::Matte))?;
        let names = match installed.manifest.model.as_ref().map(|model| &model.arch) {
            Some(Arch::Rvm(names)) => Names::Rvm(Box::new(names.clone())),
            Some(Arch::Birefnet(names)) => Names::Birefnet(names.clone()),
            // `installed_pack` asked the same question, so this is a folder
            // that changed under us between the two readings.
            _ => return Err(MlError::PackUnreadable),
        };
        let file = match &names {
            Names::Rvm(names) => names.file.clone(),
            Names::Birefnet(names) => names.file.clone(),
        };
        let identity = store::identity(&installed.manifest);
        let pack = installed.manifest.id.clone();
        let session = Session::open(&installed.path, &file, Prefer::Platform)?;
        Ok(Matte {
            session,
            names,
            identity,
            pack,
            detail,
            state: first_state(),
        })
    }

    /// What this pack is, for the key every plane it makes is filed under (§7).
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

    /// Which provider took the session: part of the same answer, and what the
    /// status card shows (§8).
    #[must_use]
    pub fn provider(&self) -> &'static str {
        self.session.provider()
    }

    /// Read one frame of RGBA bytes and hand back its coverage.
    ///
    /// For Robust Video Matting the frames must arrive **in order from the
    /// first**: each run is fed the state the one before it produced.
    ///
    /// # Errors
    ///
    /// [`MlError::ShapeMismatch`] for a frame of no size, and for a model that
    /// hands back something that is not a plane or does not hand its state
    /// back at all; [`MlError::ModelFailed`] with ONNX Runtime's own words for
    /// a run it refuses.
    pub fn run(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<Coverage, MlError> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 {
            return Err(MlError::ShapeMismatch);
        }
        if matches!(self.names, Names::Rvm(_)) {
            self.run_sequence(rgba, w, h)
        } else {
            self.run_still(rgba, w, h)
        }
    }

    /// Robust Video Matting: the frame at its own size, the state of the frame
    /// before it, and how far the model may downsample internally.
    fn run_sequence(&mut self, rgba: &[u8], w: usize, h: usize) -> Result<Coverage, MlError> {
        let Names::Rvm(names) = &self.names else {
            return Err(MlError::PackUnreadable);
        };
        let planes = tensor::pack_u8(rgba, w, h, None);
        let frame = [1, 3, h, w];
        let one = [1usize];
        let ratio = [tensor::downsample_ratio(w, h, self.detail)];
        let mut fed: Vec<(&str, &[usize], &[f32])> = Vec::with_capacity(6);
        fed.push((names.src.as_str(), &frame, planes.as_slice()));
        for (name, (shape, numbers)) in names.state_in.iter().zip(&self.state) {
            fed.push((name.as_str(), shape.as_slice(), numbers.as_slice()));
        }
        fed.push((names.downsample.as_str(), &one, ratio.as_slice()));
        let outputs = self.session.run(&fed)?;

        // The outputs are taken apart rather than read through, so the state
        // moves into its slot instead of being copied out of one: it is a few
        // hundred kilobytes and this happens once a frame.
        let mut coverage = None;
        let mut next: [Option<(Vec<usize>, Vec<f32>)>; 4] = Default::default();
        for (name, shape, numbers) in outputs {
            if name == names.matte {
                coverage = Some((shape, numbers));
            } else if let Some(at) = names.state_out.iter().position(|out| *out == name) {
                if let Some(slot) = next.get_mut(at) {
                    *slot = Some((shape, numbers));
                }
            }
        }
        // A model that answered without its state would start the next frame
        // from the top of the shot, and the matte would flicker with nothing
        // saying why.
        let Some(state) = four(next) else {
            return Err(MlError::ShapeMismatch);
        };
        let (shape, read) = coverage.ok_or(MlError::ShapeMismatch)?;
        let (out_height, out_width) = plane_of(&shape)?;
        let data = coverage_bytes(
            read.get(..out_width * out_height)
                .ok_or(MlError::ShapeMismatch)?,
        );
        self.state = state;
        Ok(Coverage {
            width: u32::try_from(out_width).map_err(|_| MlError::ShapeMismatch)?,
            height: u32::try_from(out_height).map_err(|_| MlError::ShapeMismatch)?,
            data,
        })
    }

    /// BiRefNet: the frame squashed into the model's square, normalised, and
    /// the numbers that come back put through a sigmoid and stretched to the
    /// frame's own shape.
    fn run_still(&mut self, rgba: &[u8], w: usize, h: usize) -> Result<Coverage, MlError> {
        let Names::Birefnet(names) = &self.names else {
            return Err(MlError::PackUnreadable);
        };
        let size = (names.size as usize).max(1);
        let square = tensor::resample_u8(rgba, w, h, size, size);
        let planes = tensor::pack_u8(&square, size, size, Some(names.normalise));
        let outputs =
            self.session
                .run(&[(names.input.as_str(), &[1, 3, size, size], planes.as_slice())])?;

        let (_, shape, read) = outputs
            .iter()
            .find(|(name, _, _)| *name == names.output)
            .ok_or(MlError::ShapeMismatch)?;
        let (out_height, out_width) = plane_of(shape)?;
        let logits = read
            .get(..out_width * out_height)
            .ok_or(MlError::ShapeMismatch)?;
        // Logits, not coverage: the sigmoid is what turns "how sure the model
        // is" into "how much of the pixel", and without it a matte would be
        // hard everywhere it is not exactly on the edge.
        let coverage: Vec<u8> = logits.iter().map(|v| byte(sigmoid(*v))).collect();
        Ok(Coverage {
            width: u32::try_from(w).map_err(|_| MlError::ShapeMismatch)?,
            height: u32::try_from(h).map_err(|_| MlError::ShapeMismatch)?,
            data: tensor::resample_gray(&coverage, out_width, out_height, w, h),
        })
    }
}

/// Whether the pack of that family is installed and whole.
///
/// What the effect's badge asks before it says a pack is missing, so the answer
/// there and the answer [`Matte::open`] gives are one reading rather than two.
#[must_use]
pub fn installed(arch: MatteArch) -> bool {
    installed_pack(arch).is_some()
}

/// What the pack of that family is, without opening it, or `None` when none is
/// installed or nothing on this machine could run one.
///
/// Per family rather than per task, which is the whole of why it exists: two
/// packs do matting, the Model row named one of them, and a key made from the
/// other one's digest neither moves when the named pack is updated nor stays
/// put when a second pack is installed beside it (§7).
///
/// `None` while the runtime is missing, which is
/// [`store::installed_identity`]'s own rule: a pack with nothing to run it
/// paints nothing, so those frames are named as the frames of a machine with no
/// pack at all, and installing the runtime renames them.
#[must_use]
pub fn identity(arch: MatteArch) -> Option<[u8; 32]> {
    if matches!(runtime::status(), runtime::RuntimeStatus::Missing) {
        return None;
    }
    installed_pack(arch).map(|installed| store::identity(&installed.manifest))
}

/// The installed matte pack of that family, or `None` when none is installed.
///
/// Read off the snapshot the store keeps rather than by walking the folder, and
/// filtered by family rather than by task alone, because the two matte packs do
/// the same task and the Model row picked one of them.
fn installed_pack(arch: MatteArch) -> Option<store::Installed> {
    store::snapshot()
        .iter()
        .find(|installed| {
            !installed.broken && family(installed.manifest.model.as_ref()) == Some(arch)
        })
        .cloned()
}

/// Which family a pack's model block is, or `None` for a pack that does
/// something else entirely.
fn family(model: Option<&crate::manifest::Model>) -> Option<MatteArch> {
    match model?.arch {
        Arch::Rvm(_) => Some(MatteArch::Rvm),
        Arch::Birefnet(_) => Some(MatteArch::Birefnet),
        _ => None,
    }
}

/// The height and width of a plane, off the last two dimensions of whatever
/// shape it came back with: `[1, 1, h, w]` is what both families emit and
/// `[1, h, w]` is what a re-export of one sometimes does.
fn plane_of(shape: &[usize]) -> Result<(usize, usize), MlError> {
    match shape {
        [.., h, w] if *h > 0 && *w > 0 => Ok((*h, *w)),
        _ => Err(MlError::ShapeMismatch),
    }
}

/// Four filled slots, or `None` if any of them is empty.
fn four(slots: [Option<(Vec<usize>, Vec<f32>)>; 4]) -> Option<State> {
    let [a, b, c, d] = slots;
    Some([a?, b?, c?, d?])
}

/// One coverage number as the byte a matte is kept as.
fn byte(value: f32) -> u8 {
    if value.is_finite() {
        (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    } else {
        0
    }
}

/// Coverage bytes for a whole plane.
fn coverage_bytes(read: &[f32]) -> Vec<u8> {
    read.iter().copied().map(byte).collect()
}

/// The logistic curve, which is what turns BiRefNet's numbers into coverage.
fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// **The sigmoid and the byte agree about what coverage is.** Nought is
    /// half, a large number is all of it, and a small one is none: a matte read
    /// the wrong way round is a subject cut out of the wrong side of the frame.
    #[test]
    fn a_logit_becomes_the_coverage_it_means() {
        assert_eq!(byte(sigmoid(0.0)), 128, "no opinion is half coverage");
        assert_eq!(byte(sigmoid(20.0)), 255, "certain is all of the pixel");
        assert_eq!(byte(sigmoid(-20.0)), 0, "and certainly not is none of it");
        assert!(byte(sigmoid(2.0)) > byte(sigmoid(1.0)), "and it climbs");
        assert_eq!(byte(f32::NAN), 0, "a number that is not one covers nothing");
    }

    /// **The state a run starts from is the model's own "this is the start".**
    /// A single nought in each of the four, which is what Robust Video Matting
    /// reads as an empty memory; anything else would be the state of a shot it
    /// never saw (§13).
    #[test]
    fn a_run_starts_with_the_state_that_means_nothing_yet() {
        let state = first_state();
        assert_eq!(state.len(), 4);
        for (shape, numbers) in &state {
            assert_eq!(shape, &vec![1, 1, 1, 1]);
            assert_eq!(numbers, &vec![0.0]);
        }
        assert!(four([None, None, None, None]).is_none());
        assert!(four(first_state().map(Some)).is_some());
    }

    /// **With nothing installed, opening a pack is a refusal that says which is
    /// missing.** The badge's detail is chosen off this, so "install the
    /// runtime" and "install a matte pack" must not be the same answer.
    #[test]
    fn opening_with_nothing_installed_refuses_by_name() {
        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        store::with_dir(Some(root.path().to_path_buf()));
        for arch in [MatteArch::Rvm, MatteArch::Birefnet] {
            let refusal = Matte::open(arch, Detail::Portrait).unwrap_err();
            assert!(
                matches!(
                    refusal,
                    MlError::RuntimeMissing | MlError::PackMissing(Task::Matte)
                ),
                "{arch:?}: {refusal:?}"
            );
        }
        assert!(store::installed_identity(Task::Matte).is_none());
        assert!(identity(MatteArch::Rvm).is_none());
        assert!(identity(MatteArch::Birefnet).is_none());
        store::with_dir(None);
    }

    /// A manifest for a pack of `bytes` bytes sitting in a folder of its own.
    fn a_manifest(arch: MatteArch, bytes: u64) -> String {
        let digest = "88d4531297118f595bf2fd60f6f566aec2e559393802d1f436c380f0cbbd2828";
        let (id, name, model) = match arch {
            MatteArch::Rvm => (
                "rvm",
                "Robust Video Matting",
                r#""task":"matte","arch":"rvm","file":"model.onnx",
                   "src":"src","state_in":["r1i","r2i","r3i","r4i"],
                   "state_out":["r1o","r2o","r3o","r4o"],
                   "downsample":"downsample_ratio","foreground":"fgr","matte":"pha""#,
            ),
            MatteArch::Birefnet => (
                "birefnet-lite",
                "BiRefNet Lite",
                r#""task":"matte","arch":"birefnet","file":"model.onnx",
                   "input":"input_image","output":"output_image","size":1024"#,
            ),
        };
        format!(
            r#"{{"format":1,"id":"{id}","kind":"model","name":"{name}","version":"1.0",
               "licence":"MIT","size":{bytes},
               "platforms":{{"any":{{"downloads":[
                 {{"url":"https://example.invalid/model.onnx","sha256":"{digest}",
                   "size":{bytes},"unpack":"file","dest":"model.onnx"}}]}}}},
               "model":{{{model}}}}}"#
        )
    }

    /// Where the real files sit on the reference machine.
    fn real_pack(arch: MatteArch) -> Option<PathBuf> {
        let packs = crate::test_support::packs_dir()?;
        let file = packs.join(match arch {
            MatteArch::Rvm => "rvm_mobilenetv3_fp32.onnx",
            MatteArch::Birefnet => "birefnet_lite.onnx",
        });
        file.is_file().then_some(file)
    }

    /// Write one pack into `root` as the store expects to find it.
    fn a_pack(root: &Path, arch: MatteArch, model: &Path) {
        let id = match arch {
            MatteArch::Rvm => "rvm",
            MatteArch::Birefnet => "birefnet-lite",
        };
        let home = root.join(id);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::copy(model, home.join("model.onnx")).unwrap();
        let size = std::fs::metadata(home.join("model.onnx")).unwrap().len();
        std::fs::write(home.join(store::MANIFEST_FILE), a_manifest(arch, size)).unwrap();
        store::with_dir(Some(root.to_path_buf()));
    }

    /// A frame with a bright blob in the middle of a dark ground.
    fn blob(width: usize, height: usize) -> Vec<u8> {
        let mut rgba = vec![0u8; 4 * width * height];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        for y in height / 4..height * 3 / 4 {
            for x in width / 3..width * 2 / 3 {
                let at = 4 * (y * width + x);
                rgba[at..at + 3].copy_from_slice(&[220, 200, 180]);
            }
        }
        rgba
    }

    /// **On the reference machine, Robust Video Matting reads two frames and
    /// carries its state from the first to the second.**
    ///
    /// The model is not held to a picture: what it makes of a blob on a dark
    /// ground is its own business, and pinning it would be pinning the weights.
    /// What is pinned is the shape of the answer, that every byte of it is a
    /// coverage, and that the state moved, which is the whole of what makes
    /// this model a sequence rather than a set (§13). Skipped politely wherever
    /// the runtime and the pack are not installed, which is every CI runner.
    #[test]
    fn the_real_sequence_model_carries_its_state_between_frames() {
        let (Some(runtime_dir), Some(model)) = (
            crate::test_support::runtime_dir(),
            real_pack(MatteArch::Rvm),
        ) else {
            runtime::no_runtime();
            return;
        };
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        a_pack(root.path(), MatteArch::Rvm, &model);

        let mut matte =
            Matte::open(MatteArch::Rvm, Detail::Portrait).expect("the pack would not open");
        assert_eq!(matte.pack(), "rvm", "the provenance names the wrong pack");
        assert_eq!(
            matte.identity(),
            identity(MatteArch::Rvm).expect("the pack is installed"),
            "the open pack and the key disagree about which pack it is"
        );

        let (w, h) = (192u32, 128u32);
        let first_state = matte.state.clone();
        let first = matte
            .run(&blob(w as usize, h as usize), w, h)
            .expect("the first frame");
        assert_eq!(
            (first.width, first.height),
            (w, h),
            "the coverage is at the frame's own raster"
        );
        assert_eq!(first.data.len(), (w as usize) * (h as usize));
        assert_ne!(
            matte.state, first_state,
            "the state the next frame is fed is the state it started with"
        );

        let second = matte
            .run(&blob(w as usize, h as usize), w, h)
            .expect("the second frame");
        assert_eq!(second.data.len(), first.data.len());
        store::with_dir(None);
    }

    /// **On the reference machine, BiRefNet reads one frame into a coverage at
    /// the frame's own raster.**
    ///
    /// Shape and range only, for the reason above. What this one pins that the
    /// sequence model's test cannot is the trip through the model's square and
    /// back: the frame is not square, so a resample that lost the aspect would
    /// hand back a plane of the wrong size.
    #[test]
    fn the_real_still_model_reads_a_frame_into_a_coverage() {
        let (Some(runtime_dir), Some(model)) = (
            crate::test_support::runtime_dir(),
            real_pack(MatteArch::Birefnet),
        ) else {
            runtime::no_runtime();
            return;
        };
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        a_pack(root.path(), MatteArch::Birefnet, &model);

        let mut matte =
            Matte::open(MatteArch::Birefnet, Detail::Portrait).expect("the pack would not open");
        assert_eq!(matte.pack(), "birefnet-lite");

        let (w, h) = (320u32, 180u32);
        let coverage = matte
            .run(&blob(w as usize, h as usize), w, h)
            .expect("the frame");
        assert_eq!((coverage.width, coverage.height), (w, h));
        assert_eq!(coverage.data.len(), (w as usize) * (h as usize));

        // Asking for the other family with only this one installed is a
        // refusal, never the pack that happens to be there.
        assert_eq!(
            Matte::open(MatteArch::Rvm, Detail::Portrait).unwrap_err(),
            MlError::PackMissing(Task::Matte),
            "the model row was answered with somebody else's model"
        );
        store::with_dir(None);
    }

    /// **What a matte frame costs at 1080p**, printed and never gated
    /// (13-PERFORMANCE-RULES B18, docs/impl/addons.md §8).
    ///
    /// Ignored, so it runs when somebody asks for it: it needs the runtime and
    /// a pack, and a number measured on a runner with neither would be a number
    /// about nothing. The first run of a session compiles the graph and costs
    /// about a second, so it is thrown away and the warm frames are what is
    /// reported.
    #[test]
    #[ignore = "needs the model runtime and a matte pack"]
    fn what_a_matte_frame_costs_at_1080p() {
        let Some(runtime_dir) = crate::test_support::runtime_dir() else {
            runtime::no_runtime();
            return;
        };
        runtime::load(&runtime_dir).expect("the runtime would not load");
        let _serial = crate::test_support::serially();

        let (w, h) = (1920u32, 1080u32);
        let frame = blob(w as usize, h as usize);
        for arch in [MatteArch::Rvm, MatteArch::Birefnet] {
            let Some(model) = real_pack(arch) else {
                continue;
            };
            let root = tempfile::tempdir().unwrap();
            a_pack(root.path(), arch, &model);
            let mut matte = Matte::open(arch, Detail::Portrait).expect("the pack would not open");
            matte.run(&frame, w, h).expect("the warm-up frame");

            let runs = 5;
            let started = std::time::Instant::now();
            for _ in 0..runs {
                matte.run(&frame, w, h).expect("a frame");
            }
            let each = started.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
            eprintln!(
                "matte {arch:?}, {w} by {h}, {}: {each:.1} ms a frame",
                matte.provider()
            );
            store::with_dir(None);
        }
    }
}
