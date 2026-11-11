//! Voice isolation: speech kept and everything else turned down, by
//! DeepFilterNet.
//!
//! The model never makes a sound of its own. It listens to the input's
//! spectrum and answers with two things for every 10 ms frame: a gain for each
//! of 32 bands, and a short complex filter across five frames for each of the
//! lowest 96 bins, where a voice's harmonics sit closer together than a band
//! is wide. Both are applied to the input's own spectrum, which is then turned
//! back into sound. Everything around the three graphs is DeepFilterNet's own
//! signal path, in [`signal`].
//!
//! # Thread role and contract
//!
//! A [`Voice`] owns its three sessions and is never shared, for the reason
//! [`crate::session`] gives. [`Voice::isolate`] runs offline over a whole
//! signal on the calling thread, a minute of it per run of the graphs, so the
//! model's share of the memory stays the same however long the signal is. It
//! is a pure function of its input: the sessions run on the processor with a
//! fixed thread count, the runs fall at fixed frames counted from the start,
//! and two calls on the same samples give the same samples back. A stop flag
//! is read between runs.

mod signal;

use std::sync::atomic::AtomicBool;

use crate::{
    error::MlError,
    manifest::{deep_filter::DeepFilter, Arch, Task},
    runtime,
    session::{Output, Session},
    store,
};

use signal::{Answer, Chunks, Shape, Spectra, CHUNKS};

/// The rate [`Voice::isolate`] takes and gives. The caller resamples.
pub const RATE: u32 = 48_000;

/// Threads each graph runs on. Fixed rather than left to the runtime, which
/// takes however many the machine has, and the answer must not depend on that.
/// One is also nearly the quickest: the graphs are small, and four threads
/// measured only a fifth faster.
pub const THREADS: usize = 1;

/// An open voice model.
#[derive(Debug)]
pub struct Voice {
    enc: Session,
    erb_dec: Session,
    df_dec: Session,
    /// What each decoder takes from the encoder, by the graphs' own names.
    erb_wants: Vec<String>,
    df_wants: Vec<String>,
    names: DeepFilter,
    /// How many bins each band holds, lowest first.
    bands: Vec<usize>,
}

impl Voice {
    /// Open the installed voice pack.
    ///
    /// # Errors
    ///
    /// [`MlError::RuntimeMissing`] when no model runtime is installed,
    /// [`MlError::PackMissing`] when nothing installed does voice,
    /// [`MlError::PackUnreadable`] for a pack whose numbers this code cannot
    /// run, and whatever [`Session::open_on_cpu`] answers for a graph that
    /// will not open.
    pub fn open() -> Result<Self, MlError> {
        if matches!(runtime::status(), runtime::RuntimeStatus::Missing) {
            return Err(MlError::RuntimeMissing);
        }
        let installed = store::find(Task::Voice).ok_or(MlError::PackMissing(Task::Voice))?;
        let Some(Arch::DeepFilter(names)) =
            installed.manifest.model.as_ref().map(|model| &model.arch)
        else {
            return Err(MlError::PackUnreadable);
        };
        let bands = signal::runnable(names).ok_or(MlError::PackUnreadable)?;
        let open = |file: &str| Session::open_on_cpu(&installed.path, file, THREADS);
        let (enc, erb_dec, df_dec) = (
            open(&names.enc)?,
            open(&names.erb_dec)?,
            open(&names.df_dec)?,
        );
        let wants = |session: &Session| -> Vec<String> {
            session.inputs().into_iter().map(|(name, _)| name).collect()
        };
        Ok(Voice {
            erb_wants: wants(&erb_dec),
            df_wants: wants(&df_dec),
            enc,
            erb_dec,
            df_dec,
            names: names.clone(),
            bands,
        })
    }

    /// The voice in `mono`, one channel at [`RATE`], as a signal of the same
    /// length lined up with it sample for sample.
    ///
    /// # Errors
    ///
    /// As [`Voice::isolate`].
    pub fn enhance(&mut self, mono: &[f32]) -> Result<Vec<f32>, MlError> {
        let mut outs = self.isolate(vec![mono.to_vec()], &AtomicBool::new(false))?;
        Ok(outs.pop().unwrap_or_default())
    }

    /// The voice in each of `channels`, all at [`RATE`] and all the same
    /// length, each lined up with its input sample for sample. The channels
    /// are handed back with their voice written over them, so the answer
    /// costs no memory beyond the input.
    ///
    /// The model hears the channels' mean and its answer is applied to each
    /// channel, so a voice off to one side stays there. Offline and whole: the
    /// model looks 20 ms ahead, so the answer for any sample depends on what
    /// comes after it, and the same samples always give the same answer.
    /// `stop` is read about every minute of signal.
    ///
    /// # Errors
    ///
    /// [`MlError::ModelFailed`] with ONNX Runtime's own words for a run it
    /// refuses, [`MlError::ShapeMismatch`] for channels of different lengths
    /// or a graph that answers with tensors other than the pack's manifest
    /// names, and [`MlError::Cancelled`] once `stop` is set.
    pub fn isolate(
        &mut self,
        channels: Vec<Vec<f32>>,
        stop: &AtomicBool,
    ) -> Result<Vec<Vec<f32>>, MlError> {
        self.isolate_in(channels, stop, CHUNKS)
    }

    /// The whole of [`Voice::isolate`], with the runs cut as `chunks` says.
    fn isolate_in(
        &mut self,
        mut channels: Vec<Vec<f32>>,
        stop: &AtomicBool,
        chunks: Chunks,
    ) -> Result<Vec<Vec<f32>>, MlError> {
        let shape = Shape::of(&self.names);
        let bands = self.bands.clone();
        signal::run(
            shape,
            &bands,
            &mut channels,
            stop,
            chunks,
            &mut |spectra, first, end| self.answer(spectra, first, end),
        )?;
        Ok(channels)
    }

    /// Run the three graphs over frames `first..end`, each fed the features
    /// of the frame the encoder's lookahead says it reads.
    fn answer(&mut self, spectra: &Spectra, first: usize, end: usize) -> Result<Answer, MlError> {
        let shape = spectra.shape;
        let n = end - first;
        let mut erb = Vec::with_capacity(n * shape.nb_erb);
        let mut cplx = vec![0.0; 2 * n * shape.nb_df];
        for (row, t) in (first..end).enumerate() {
            match spectra.features(t + shape.ahead) {
                Some((bands, bins)) => {
                    erb.extend_from_slice(bands);
                    for (f, pair) in bins.chunks_exact(2).enumerate() {
                        cplx[row * shape.nb_df + f] = pair[0];
                        cplx[(n + row) * shape.nb_df + f] = pair[1];
                    }
                }
                // Past the end the model hears silence, as DeepFilterNet's
                // own padding has it.
                None => erb.resize(erb.len() + shape.nb_erb, 0.0),
            }
        }
        let encoded = self.enc.run(&[
            (
                self.names.feat_erb.as_str(),
                &[1, 1, n, shape.nb_erb],
                erb.as_slice(),
            ),
            (
                self.names.feat_spec.as_str(),
                &[1, 2, n, shape.nb_df],
                cplx.as_slice(),
            ),
        ])?;
        drop((erb, cplx));
        let gains = decode(
            &mut self.erb_dec,
            &self.erb_wants,
            &encoded,
            &self.names.mask,
            n * shape.nb_erb,
        )?;
        let coefs = decode(
            &mut self.df_dec,
            &self.df_wants,
            &encoded,
            &self.names.coefs,
            n * shape.nb_df * shape.order * 2,
        )?;
        Ok(Answer {
            first,
            gains,
            coefs,
        })
    }
}

/// Run one decoder on what the encoder made and keep the tensor named
/// `output`, which must be `len` numbers long.
fn decode(
    session: &mut Session,
    wants: &[String],
    encoded: &[Output],
    output: &str,
    len: usize,
) -> Result<Vec<f32>, MlError> {
    let mut fed = Vec::with_capacity(wants.len());
    for want in wants {
        let (name, shape, data) = encoded
            .iter()
            .find(|(name, ..)| name == want)
            .ok_or(MlError::ShapeMismatch)?;
        fed.push((name.as_str(), shape.as_slice(), data.as_slice()));
    }
    session
        .run(&fed)?
        .into_iter()
        .find(|(name, ..)| name == output)
        .map(|(.., data)| data)
        .filter(|data| data.len() == len)
        .ok_or(MlError::ShapeMismatch)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The voice pack as upstream publishes it, from `LUMIT_ML_PACKS_DIR`.
    fn real_tarball() -> Option<std::path::PathBuf> {
        let file = crate::test_support::packs_dir()?.join("DeepFilterNet3_onnx.tar.gz");
        file.is_file().then_some(file)
    }

    /// Something a voice model takes for a voice: a buzz at a wavering pitch
    /// with three formants, spoken in 300 ms syllables with gaps between, and
    /// whether each sample is inside a syllable.
    fn syllables(seconds: f32) -> (Vec<f32>, Vec<bool>) {
        let rate = RATE as f32;
        let count = (seconds * rate) as usize;
        let (mut phase, mut voice, mut inside) = (0.0f32, Vec::new(), Vec::new());
        for i in 0..count {
            let time = i as f32 / rate;
            let pitch = 130.0 + 15.0 * (std::f32::consts::TAU * 3.0 * time).sin();
            phase = (phase + std::f32::consts::TAU * pitch / rate) % std::f32::consts::TAU;
            let within = (time % 0.5) / 0.3;
            let envelope = if within < 1.0 {
                (std::f32::consts::PI * within).sin().powi(2)
            } else {
                0.0
            };
            let mut sample = 0.0;
            for harmonic in 1..60 {
                let hz = pitch * harmonic as f32;
                let formant = [(600.0f32, 1.0f32), (1700.0, 0.5), (2600.0, 0.3)]
                    .iter()
                    .map(|(at, level)| level * (-((hz - at) / 200.0).powi(2)).exp())
                    .sum::<f32>();
                sample += formant * (phase * harmonic as f32).sin();
            }
            voice.push(0.1 * envelope * sample);
            inside.push(within < 1.0);
        }
        (voice, inside)
    }

    /// The level in decibels of the samples where `pick` says so.
    fn level(signal: &[f32], marks: &[bool], pick: bool) -> f64 {
        let (sum, count) = signal
            .iter()
            .zip(marks)
            .filter(|(_, mark)| **mark == pick)
            .fold((0.0f64, 0usize), |(sum, count), (x, _)| {
                (sum + f64::from(*x).powi(2), count + 1)
            });
        10.0 * (sum / count.max(1) as f64 + 1e-20).log10()
    }

    /// **On the reference machine, the real pack takes the hiss out from
    /// between the syllables, keeps the syllables, lines up with its input
    /// and gives the same samples twice, across run seams too.**
    ///
    /// The pack is installed from the upstream `.tar.gz` with the catalogue's
    /// own entry, so this is also the install path that file takes. Skipped
    /// politely wherever the runtime and the tarball are not, which is every
    /// CI runner.
    #[test]
    fn the_real_model_keeps_the_voice_and_takes_out_the_hiss() {
        let (Some(runtime_dir), Some(tarball)) =
            (crate::test_support::runtime_dir(), real_tarball())
        else {
            runtime::no_runtime();
            return;
        };
        runtime::load(&runtime_dir).expect("the runtime would not load");

        let _serial = crate::test_support::serially();
        let root = tempfile::tempdir().unwrap();
        store::with_dir(Some(root.path().to_path_buf()));
        let entry = crate::manifest::deep_filter::tests::CATALOGUE_ENTRY;
        store::install(entry, &[tarball]).expect("the pack would not install");
        let mut voice = Voice::open().expect("the pack would not open");
        store::with_dir(None);

        let (clean, inside) = syllables(6.0);
        let mut seed = 0x2545_f491_u32;
        let noisy: Vec<f32> = clean
            .iter()
            .map(|x| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                x + 0.02 * (seed as f32 / u32::MAX as f32 - 0.5)
            })
            .collect();

        let heard = voice.enhance(&noisy).unwrap();
        assert_eq!(heard.len(), noisy.len(), "as long as its input");
        assert_eq!(voice.enhance(&noisy).unwrap(), heard, "the same twice");

        let hiss_before = level(&noisy, &inside, false);
        let hiss_after = level(&heard, &inside, false);
        assert!(
            hiss_after < hiss_before - 10.0,
            "the gaps went from {hiss_before:.1} dB to {hiss_after:.1} dB"
        );
        let voice_before = level(&noisy, &inside, true);
        let voice_after = level(&heard, &inside, true);
        assert!(
            voice_after > voice_before - 6.0,
            "the syllables went from {voice_before:.1} dB to {voice_after:.1} dB"
        );

        // Runs of two seconds, each settled from the very start, so every
        // handover lands inside the signal and joins two runs that heard the
        // same thing: anything but the same answer is the seams' bookkeeping.
        let short = Chunks {
            size: 200,
            warm: 400,
            fade: 20,
        };
        let seamed = voice
            .isolate_in(vec![noisy.clone()], &AtomicBool::new(false), short)
            .unwrap()
            .pop()
            .unwrap();
        let error: f64 = seamed
            .iter()
            .zip(&heard)
            .map(|(a, b)| f64::from(a - b).powi(2))
            .sum();
        let signal: f64 = heard.iter().map(|x| f64::from(*x).powi(2)).sum();
        let apart = 10.0 * (signal / error.max(1e-30)).log10();
        assert!(apart > 60.0, "the seams are {apart:.1} dB below the voice");
    }
}
