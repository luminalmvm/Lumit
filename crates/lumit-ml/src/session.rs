//! One model, open and ready to run, speaking f32 tensors and nothing else
//! (docs/impl/addons.md §3).
//!
//! # Thread role and contract
//!
//! A session is owned by the thread that runs the analysis and is never put
//! behind a shared lock: a run is an FFI call that may hold the GPU, and
//! 14-ENGINEERING-RULES §1.3 forbids holding a lock across one. One session
//! per pack per job; the job drops it when it finishes, so nothing is held
//! between analyses. A single run of one frame is not interruptible;
//! cancellation happens between frames, in the caller's own loop.

use std::{borrow::Cow, path::Path};

use crate::{error::MlError, runtime};

/// Which execution provider a session asks for first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Prefer {
    /// The platform's accelerator, with ONNX Runtime's own CPU fallback
    /// behind it.
    #[default]
    Platform,
    /// The CPU, chosen deliberately.
    Cpu,
}

/// One tensor a run hands back: its name, its shape and its numbers.
pub type Output = (String, Vec<usize>, Vec<f32>);

/// An open model.
#[derive(Debug)]
pub struct Session {
    inner: ort::session::Session,
    provider: &'static str,
}

impl Session {
    /// Open `file` inside an installed pack's folder.
    ///
    /// A provider that will not register is not a failure: the session is
    /// built again with none and [`Session::provider`] then says `CPU`, which
    /// is what the status card shows. Nothing here downgrades silently.
    ///
    /// # Errors
    ///
    /// [`MlError::PackUnreadable`] when the file is not in the pack's folder,
    /// [`MlError::RuntimeMissing`] when no runtime is installed, and
    /// [`MlError::ModelFailed`] with ONNX Runtime's own words when the graph
    /// will not load.
    pub fn open(pack_dir: &Path, file: &str, prefer: Prefer) -> Result<Self, MlError> {
        let path = pack_dir.join(file);
        if !path.is_file() {
            return Err(MlError::PackUnreadable);
        }
        // Nothing below may run before the library is open: `ort` goes looking
        // for it beside the executable on its own and `expect`s when it is not
        // there, and that is a panic inside a library rather than a refusal
        // this side can answer with (§13).
        if !runtime::loaded() {
            runtime::load_installed()?;
        }
        if prefer == Prefer::Platform {
            if let Some(inner) = accelerated(&path) {
                runtime::took(runtime::PROVIDER);
                return Ok(Session {
                    inner,
                    provider: runtime::PROVIDER,
                });
            }
        }
        let mut builder = ort::session::Session::builder().map_err(failed)?;
        let inner = builder.commit_from_file(&path).map_err(failed)?;
        if prefer == Prefer::Platform {
            // The accelerator would not register, so this machine's answer is
            // the processor and every key made from here on says so (§7).
            runtime::took(runtime::CPU);
        }
        Ok(Session {
            inner,
            provider: runtime::CPU,
        })
    }

    /// Which provider took this session. Part of every key a result made by
    /// it is filed under (§7).
    #[must_use]
    pub fn provider(&self) -> &'static str {
        self.provider
    }

    /// The model's own input names and shapes, so a caller can check them
    /// against what its manifest claims before it feeds anything. A dimension
    /// the graph leaves open reads `-1`.
    #[must_use]
    pub fn inputs(&self) -> Vec<(String, Vec<i64>)> {
        self.inner
            .inputs()
            .iter()
            .map(|outlet| {
                let shape = match outlet.dtype() {
                    ort::value::ValueType::Tensor { shape, .. } => shape.to_vec(),
                    _ => Vec::new(),
                };
                (outlet.name().to_owned(), shape)
            })
            .collect()
    }

    /// Run the graph once: named tensors in, named tensors out.
    ///
    /// # Errors
    ///
    /// [`MlError::ShapeMismatch`] when a shape and its data disagree or an
    /// output is not f32, and [`MlError::ModelFailed`] with ONNX Runtime's own
    /// words for everything the run itself refuses, a tensor name the graph
    /// does not have included.
    pub fn run(&mut self, inputs: &[(&str, &[usize], &[f32])]) -> Result<Vec<Output>, MlError> {
        let mut fed: Vec<(Cow<'_, str>, ort::session::SessionInputValue<'_>)> =
            Vec::with_capacity(inputs.len());
        for (name, shape, data) in inputs {
            if shape.iter().product::<usize>() != data.len() {
                return Err(MlError::ShapeMismatch);
            }
            let tensor = ort::value::Tensor::from_array((*shape, data.to_vec()))
                .map_err(|_| MlError::ShapeMismatch)?;
            fed.push((Cow::Borrowed(*name), tensor.into()));
        }

        let outputs = self.inner.run(fed).map_err(failed)?;
        let mut out = Vec::with_capacity(outputs.len());
        for (name, value) in outputs.iter() {
            let (shape, data) = value
                .try_extract_tensor::<f32>()
                .map_err(|_| MlError::ShapeMismatch)?;
            out.push((
                name.to_owned(),
                shape
                    .iter()
                    .map(|dim| usize::try_from(*dim).unwrap_or(0))
                    .collect(),
                data.to_vec(),
            ));
        }
        Ok(out)
    }
}

/// A session on the platform's accelerator, or `None` when it would not
/// register. The provider is asked to say so rather than fall back quietly,
/// so the answer here is honest about which one ran.
#[cfg(windows)]
fn accelerated(path: &Path) -> Option<ort::session::Session> {
    let mut builder = ort::session::Session::builder()
        .ok()?
        .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])
        .ok()?;
    builder.commit_from_file(path).ok()
}

/// The same, on Apple silicon and Intel Macs.
#[cfg(target_os = "macos")]
fn accelerated(path: &Path) -> Option<ort::session::Session> {
    let mut builder = ort::session::Session::builder()
        .ok()?
        .with_execution_providers([ort::ep::CoreML::default().build().error_on_failure()])
        .ok()?;
    builder.commit_from_file(path).ok()
}

/// Linux has no accelerator here: the CPU provider is the whole story, and
/// the runtime row says so.
#[cfg(not(any(windows, target_os = "macos")))]
fn accelerated(_path: &Path) -> Option<ort::session::Session> {
    None
}

/// ONNX Runtime's own sentence, kept for the badge's detail slot.
fn failed<E: std::fmt::Display>(e: E) -> MlError {
    MlError::ModelFailed(e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **A real model opens, reports its provider, and runs.**
    ///
    /// It opens `rvm_mobilenetv3_fp32.onnx` out of the pack cache rather than
    /// a graph the test writes, because the smallest honest ONNX file is a
    /// hand-assembled protobuf and a hand-assembled protobuf proves only that
    /// the test can write protobuf. This one has the shape the matte tier
    /// actually feeds: a frame, four recurrent state tensors and a ratio in,
    /// a foreground and a matte out. Skipped politely wherever the runtime
    /// and the packs are not installed, which is every CI runner.
    #[test]
    fn a_real_model_opens_and_runs_a_frame_through() {
        let (Some(runtime_dir), Some(packs)) = (
            crate::test_support::runtime_dir(),
            crate::test_support::packs_dir(),
        ) else {
            runtime::no_runtime();
            return;
        };
        let model = "rvm_mobilenetv3_fp32.onnx";
        if !packs.join(model).is_file() {
            eprintln!("skipping: no {model} in LUMIT_ML_PACKS_DIR");
            return;
        }
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let mut session = Session::open(&packs, model, Prefer::Platform).unwrap();
        assert!(
            session.provider() == runtime::PROVIDER || session.provider() == runtime::CPU,
            "an unexpected provider: {}",
            session.provider()
        );
        let named: Vec<String> = session.inputs().into_iter().map(|(name, _)| name).collect();
        assert!(named.iter().any(|name| name == "src"), "{named:?}");

        let (width, height) = (64usize, 64usize);
        let frame = vec![0.5f32; 3 * width * height];
        let zero = [0.0f32];
        let ratio = [0.25f32];
        let outputs = session
            .run(&[
                ("src", &[1, 3, height, width], frame.as_slice()),
                ("r1i", &[1, 1, 1, 1], &zero),
                ("r2i", &[1, 1, 1, 1], &zero),
                ("r3i", &[1, 1, 1, 1], &zero),
                ("r4i", &[1, 1, 1, 1], &zero),
                ("downsample_ratio", &[1], &ratio),
            ])
            .unwrap();

        let (_, shape, data) = outputs
            .iter()
            .find(|(name, _, _)| name == "pha")
            .expect("the matte came back under another name");
        assert_eq!(shape, &vec![1, 1, height, width]);
        assert_eq!(data.len(), width * height);
    }

    /// **A file that is not in the pack is refused before ONNX Runtime is
    /// asked.** A broken install must not reach the library at all, because
    /// what comes back from it is a sentence about a path and not about an
    /// addon.
    #[test]
    fn a_missing_model_file_is_refused_as_unreadable() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            Session::open(empty.path(), "model.onnx", Prefer::Platform).unwrap_err(),
            MlError::PackUnreadable
        );
    }

    /// **A file that is there is answered with a refusal, never a panic.**
    ///
    /// The one test here that runs on every machine, including the CI runners
    /// that have no runtime at all. Without the guard at the top of
    /// [`Session::open`] this is a process that dies inside `ort` rather than
    /// a sentence the page can show, which is what §13 calls the lazy path.
    /// Which refusal comes back depends on what the machine has: no runtime
    /// installed is `RuntimeMissing`, and a runtime that opens then turns the
    /// bytes down itself.
    #[test]
    fn a_session_on_a_machine_with_no_runtime_refuses_rather_than_panicking() {
        let pack = tempfile::tempdir().unwrap();
        std::fs::write(pack.path().join("model.onnx"), b"not a graph").unwrap();

        let refusal = Session::open(pack.path(), "model.onnx", Prefer::Platform).unwrap_err();
        assert!(
            matches!(
                refusal,
                MlError::RuntimeMissing | MlError::RuntimeFailed(_) | MlError::ModelFailed(_)
            ),
            "{refusal:?}"
        );
    }
}
