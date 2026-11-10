//! One model, open and ready to run (docs/impl/addons.md §3).
//!
//! Most models here take f32 tensors and hand f32 tensors back, and
//! [`Session::run`] is all they need. A decoder that writes one word piece at
//! a time needs more: whole numbers and flags going in, and a cache that goes
//! straight back in on the next step without being copied out into a Rust
//! buffer and back again. [`Session::bound`] is that door, and a [`Held`] is
//! a tensor the runtime keeps, on the graphics card where there is one.
//!
//! # Thread role and contract
//!
//! A session is owned by the thread that runs the analysis and is never put
//! behind a shared lock: a run is an FFI call that may hold the GPU, and
//! 14-ENGINEERING-RULES §1.3 forbids holding a lock across one. One session
//! per pack per job; the job drops it when it finishes, so nothing is held
//! between analyses. A single run of one frame is not interruptible;
//! cancellation happens between frames, in the caller's own loop.

#[cfg(not(target_arch = "wasm32"))]
use std::borrow::Cow;
use std::path::Path;

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

/// One named input to [`Session::bound`].
#[derive(Debug, Clone, Copy)]
pub enum Feed<'a> {
    /// Numbers, with their shape.
    F32(&'a [usize], &'a [f32]),
    /// Whole numbers, such as word piece ids, with their shape. An empty
    /// shape is a scalar.
    I64(&'a [usize], &'a [i64]),
    /// Flags, with their shape.
    Bool(&'a [usize], &'a [bool]),
    /// Something an earlier run handed back, fed in where it already is.
    Held(&'a Held),
}

/// Where an output of [`Session::bound`] is left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// In ordinary memory, so [`Held::read`] can read it.
    Host,
    /// On the device the session runs on, for something that only ever goes
    /// back into the model. The same as [`Place::Host`] on the processor.
    Device,
}

/// A tensor the runtime keeps between runs.
#[derive(Debug)]
pub struct Held {
    #[cfg(not(target_arch = "wasm32"))]
    value: ort::value::DynValue,
    #[cfg(target_arch = "wasm32")]
    value: std::convert::Infallible,
}

impl Held {
    /// The shape and the numbers of an f32 tensor left in ordinary memory.
    ///
    /// # Errors
    ///
    /// [`MlError::ShapeMismatch`] for a tensor that is not f32 or was left on
    /// the device.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read(&self) -> Result<(Vec<usize>, &[f32]), MlError> {
        let (shape, data) = self
            .value
            .try_extract_tensor::<f32>()
            .map_err(|_| MlError::ShapeMismatch)?;
        Ok((
            shape
                .iter()
                .map(|dim| usize::try_from(*dim).unwrap_or(0))
                .collect(),
            data,
        ))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn read(&self) -> Result<(Vec<usize>, &[f32]), MlError> {
        match self.value {}
    }
}

/// An open model.
#[derive(Debug)]
pub struct Session {
    #[cfg(not(target_arch = "wasm32"))]
    inner: ort::session::Session,
    #[cfg(target_arch = "wasm32")]
    inner: std::convert::Infallible,
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
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(not(target_arch = "wasm32"))]
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

    /// Run the graph once through a binding: any of [`Feed`]'s kinds in, and
    /// only the outputs asked for out, each left where `outputs` says, in the
    /// order asked.
    ///
    /// An output nobody asks for is not fetched, so a decoder step that wants
    /// the next scores and the new cache never copies out the rest.
    ///
    /// # Errors
    ///
    /// [`MlError::ShapeMismatch`] when a shape and its data disagree or an
    /// output asked for did not come back, and [`MlError::ModelFailed`] with
    /// ONNX Runtime's own words for everything the run itself refuses.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn bound(
        &mut self,
        inputs: &[(&str, Feed<'_>)],
        outputs: &[(&str, Place)],
    ) -> Result<Vec<Held>, MlError> {
        use ort::memory::{AllocationDevice, AllocatorType, MemoryInfo, MemoryType};

        let mut binding = self.inner.create_binding().map_err(failed)?;
        for (name, feed) in inputs {
            let made = match feed {
                Feed::F32(shape, data) => Some(tensor(shape, data)?),
                Feed::I64(shape, data) => Some(tensor(shape, data)?),
                Feed::Bool(shape, data) => Some(tensor(shape, data)?),
                Feed::Held(held) => {
                    binding.bind_input(*name, &held.value).map_err(failed)?;
                    None
                }
            };
            if let Some(value) = made {
                binding.bind_input(*name, &value).map_err(failed)?;
            }
        }
        let host = MemoryInfo::new(
            AllocationDevice::CPU,
            0,
            AllocatorType::Device,
            MemoryType::Default,
        )
        .map_err(failed)?;
        // DirectML's own allocator is the one its outputs can stay in. Every
        // other provider here either is the processor or shares its memory.
        let device = if self.provider == "DirectML" {
            Some(
                MemoryInfo::new(
                    AllocationDevice::DIRECTML,
                    0,
                    AllocatorType::Device,
                    MemoryType::Default,
                )
                .map_err(failed)?,
            )
        } else {
            None
        };
        for (name, place) in outputs {
            let info = match (place, &device) {
                (Place::Device, Some(device)) => device,
                _ => &host,
            };
            binding.bind_output_to_device(*name, info).map_err(failed)?;
        }

        let mut ran = self.inner.run_binding(&binding).map_err(failed)?;
        // A device output may go next into another session, which has a queue
        // of its own on the card and would otherwise read it before this run
        // has finished writing it.
        if device.is_some() && outputs.iter().any(|(_, place)| *place == Place::Device) {
            binding.synchronize_outputs().map_err(failed)?;
        }
        outputs
            .iter()
            .map(|(name, _)| {
                ran.remove(*name)
                    .map(|value| Held { value })
                    .ok_or(MlError::ShapeMismatch)
            })
            .collect()
    }
}

/// One input made into a tensor the runtime owns, with its shape checked
/// against its data first.
#[cfg(not(target_arch = "wasm32"))]
fn tensor<T>(shape: &[usize], data: &[T]) -> Result<ort::value::DynValue, MlError>
where
    T: ort::value::PrimitiveTensorElementType + Clone + std::fmt::Debug + 'static,
{
    if shape.iter().product::<usize>() != data.len() {
        return Err(MlError::ShapeMismatch);
    }
    ort::value::Tensor::from_array((shape.to_vec(), data.to_vec()))
        .map(ort::value::Tensor::into_dyn)
        .map_err(|_| MlError::ShapeMismatch)
}

/// The browser build has no ONNX Runtime, so a session never opens there.
#[cfg(target_arch = "wasm32")]
impl Session {
    pub fn open(_pack_dir: &Path, _file: &str, _prefer: Prefer) -> Result<Self, MlError> {
        Err(MlError::RuntimeFailed(runtime::NOT_HERE.into()))
    }

    #[must_use]
    pub fn inputs(&self) -> Vec<(String, Vec<i64>)> {
        match self.inner {}
    }

    pub fn run(&mut self, _inputs: &[(&str, &[usize], &[f32])]) -> Result<Vec<Output>, MlError> {
        match self.inner {}
    }

    pub fn bound(
        &mut self,
        _inputs: &[(&str, Feed<'_>)],
        _outputs: &[(&str, Place)],
    ) -> Result<Vec<Held>, MlError> {
        match self.inner {}
    }
}

/// A session on the platform's accelerator, or `None` when it would not
/// register. The provider is asked to say so rather than fall back quietly,
/// so the answer here is honest about which one ran.
///
/// DirectML wants the memory pattern off and the graph run in order, as ONNX
/// Runtime's own notes on the provider say, so both are set here.
#[cfg(windows)]
fn accelerated(path: &Path) -> Option<ort::session::Session> {
    let mut builder = ort::session::Session::builder()
        .ok()?
        .with_memory_pattern(false)
        .ok()?
        .with_parallel_execution(false)
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
#[cfg(not(any(windows, target_os = "macos", target_arch = "wasm32")))]
fn accelerated(_path: &Path) -> Option<ort::session::Session> {
    None
}

/// ONNX Runtime's own sentence, kept for the badge's detail slot.
#[cfg(not(target_arch = "wasm32"))]
fn failed<E: std::fmt::Display>(e: E) -> MlError {
    MlError::ModelFailed(e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

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
