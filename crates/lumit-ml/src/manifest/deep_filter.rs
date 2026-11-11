//! The voice model's block of `addon.json`: DeepFilterNet's three graphs,
//! the names of their tensors and the numbers its signal path is built from.
//!
//! # Thread role and contract
//!
//! Pure data, parsed with the rest of the manifest in [`super::parse`].

use serde::{Deserialize, Serialize};

/// DeepFilterNet: one spectrum in, three graphs, and 32 band gains plus a
/// short complex filter on the low bins out (§4). The numbers are the
/// export's own `config.ini`, because the spectrum fed to the graphs has to be
/// made exactly the way the model was trained on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepFilter {
    /// The encoder `.onnx` file inside the addon folder.
    pub enc: String,
    /// The band-gain decoder `.onnx` file.
    pub erb_dec: String,
    /// The deep-filter decoder `.onnx` file.
    pub df_dec: String,
    /// The encoder's band input, `[1, 1, T, nb_erb]`.
    pub feat_erb: String,
    /// The encoder's low-bin input, `[1, 2, T, nb_df]`, real then imaginary.
    pub feat_spec: String,
    /// The band gains, `[1, 1, T, nb_erb]`, 0 to 1.
    pub mask: String,
    /// The filter, `[1, T, nb_df, df_order * 2]`.
    pub coefs: String,
    /// Samples per second the model was trained at.
    pub sr: u32,
    /// The transform length, which is also the window.
    pub fft_size: u32,
    /// Samples between frames. Half the window: nothing else is supported.
    pub hop_size: u32,
    /// How many bands the gains come in.
    pub nb_erb: u32,
    /// The fewest bins a band may have.
    pub min_nb_erb_freqs: u32,
    /// How many of the lowest bins the filter replaces.
    pub nb_df: u32,
    /// How many frames the filter spans.
    pub df_order: u32,
    /// How many of those frames are ahead of the one it makes.
    pub df_lookahead: u32,
    /// How many frames ahead the encoder reads.
    pub conv_lookahead: u32,
    /// How quickly the running feature averages follow the signal, per frame.
    /// DeepFilterNet derives it from `norm_tau` and rounds it: tau 1 is 0.99.
    pub norm_alpha: f32,
}

impl Default for DeepFilter {
    fn default() -> Self {
        DeepFilter {
            enc: "enc.onnx".into(),
            erb_dec: "erb_dec.onnx".into(),
            df_dec: "df_dec.onnx".into(),
            feat_erb: "feat_erb".into(),
            feat_spec: "feat_spec".into(),
            mask: "m".into(),
            coefs: "coefs".into(),
            sr: 48_000,
            fft_size: 960,
            hop_size: 480,
            nb_erb: 32,
            min_nb_erb_freqs: 2,
            nb_df: 96,
            df_order: 5,
            df_lookahead: 2,
            conv_lookahead: 2,
            norm_alpha: 0.99,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod tests {
    use super::super::{parse, Arch, Task, Unpack};
    use super::DeepFilter;

    /// The voice pack's catalogue entry, as `lumit-addons` is to publish it:
    /// upstream's own `.tar.gz`, three graphs taken out of it, and the
    /// numbers from the export's `config.ini`.
    pub(crate) const CATALOGUE_ENTRY: &str = r#"{
  "format": 1,
  "id": "deepfilternet3",
  "kind": "model",
  "name": "DeepFilterNet3",
  "version": "3.0",
  "summary": "Keeps speech and turns everything else down",
  "licence": "MIT OR Apache-2.0",
  "licence_url": "https://github.com/Rikorose/DeepFilterNet#license",
  "homepage": "https://github.com/Rikorose/DeepFilterNet",
  "size": 7983136,
  "requires": [
    "runtime"
  ],
  "platforms": {
    "any": {
      "downloads": [
        {
          "url": "https://raw.githubusercontent.com/Rikorose/DeepFilterNet/978576aa8400552a4ce9730838c635aa30db5e61/models/DeepFilterNet3_onnx.tar.gz",
          "sha256": "c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616",
          "size": 7983136,
          "unpack": "tar.gz",
          "entries": {
            "tmp/export/enc.onnx": "enc.onnx",
            "tmp/export/erb_dec.onnx": "erb_dec.onnx",
            "tmp/export/df_dec.onnx": "df_dec.onnx"
          }
        }
      ]
    }
  },
  "model": {
    "task": "voice",
    "arch": "deep-filter",
    "enc": "enc.onnx",
    "erb_dec": "erb_dec.onnx",
    "df_dec": "df_dec.onnx",
    "feat_erb": "feat_erb",
    "feat_spec": "feat_spec",
    "mask": "m",
    "coefs": "coefs",
    "sr": 48000,
    "fft_size": 960,
    "hop_size": 480,
    "nb_erb": 32,
    "min_nb_erb_freqs": 2,
    "nb_df": 96,
    "df_order": 5,
    "df_lookahead": 2,
    "conv_lookahead": 2,
    "norm_alpha": 0.99
  }
}"#;

    /// **The voice pack's catalogue entry parses, as a voice pack, with the
    /// three graphs it names coming out of the tarball.** The entry lives in
    /// a second repository, so this is the one place it meets the struct it
    /// is read into: a key spelt differently on either side is a pack that
    /// downloads in full and then refuses to install.
    #[test]
    fn the_voice_catalogue_entry_parses() {
        let manifest = parse(CATALOGUE_ENTRY).expect("the entry parses");
        assert_eq!(manifest.task(), Some(Task::Voice));
        let model = manifest.model.as_ref().unwrap();
        assert_eq!(model.arch, Arch::DeepFilter(DeepFilter::default()));
        let download = &manifest.platform().unwrap().downloads[0];
        assert_eq!(download.unpack, Unpack::TarGz);
        let mut placed: Vec<String> = manifest
            .expected()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        placed.sort();
        assert_eq!(placed, ["df_dec.onnx", "enc.onnx", "erb_dec.onnx"]);
    }
}
