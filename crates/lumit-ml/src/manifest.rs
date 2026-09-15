//! `addon.json`: what an addon says it is, what files it carries, and which
//! tensors the engine feeds its model (docs/impl/addons.md §4).
//!
//! The same object is the catalogue entry and the installed manifest, so an
//! install copies the text into the folder unchanged and the parser here is
//! the only thing that ever judges it.
//!
//! # Thread role and contract
//!
//! Pure parsing. No IO, no threads, no interior mutability: the text arrives
//! borrowed and a [`Manifest`] goes back (14-ENGINEERING-RULES §1.1). Nothing
//! in a manifest is executed, and nothing in it is trusted until [`parse`] has
//! said so: it arrives over the network, so every file name it carries is
//! checked to be a plain name before anything opens it.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

use crate::error::MlError;

/// The manifest format this build reads. A higher one is refused rather than
/// guessed at.
pub const FORMAT: u32 = 1;

/// The longest an addon id may be. It is also a folder name, so it stays
/// short enough to sit inside a path with room to spare.
pub const ID_MAX: usize = 64;

/// The platform key this build looks for, checked before `any`.
pub const CURRENT_PLATFORM: &str = if cfg!(windows) {
    "windows-x86_64"
} else if cfg!(target_os = "macos") {
    if cfg!(target_arch = "aarch64") {
        "macos-aarch64"
    } else {
        "macos-x86_64"
    }
} else {
    "linux-x86_64"
};

/// Every platform key a manifest may name. Anything else is a typo or a
/// platform this build knows nothing about, and either way it is refused.
const PLATFORM_KEYS: [&str; 5] = [
    "windows-x86_64",
    "macos-aarch64",
    "macos-x86_64",
    "linux-x86_64",
    "any",
];

/// Which of the two things an addon is (§2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// ONNX Runtime and its provider libraries. Exactly one may be installed.
    Runtime,
    /// One analysis model, with its licence and its tensor contract.
    Model,
}

/// What a pack does. This is what Lumit's code binds to; the architecture is
/// only how the tensors are named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Task {
    /// The frame between two real ones, for Retime's Flow.
    Synthesis,
    /// How far away every pixel is.
    Depth,
    /// Coverage: how much of each pixel is the subject.
    Matte,
    /// The thing that was clicked on, traced.
    Segmentation,
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Task::Synthesis => "synthesis",
            Task::Depth => "depth",
            Task::Matte => "matte",
            Task::Segmentation => "segmentation",
        })
    }
}

/// How one download becomes files on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unpack {
    /// The download is the file. It is copied to `dest`.
    File,
    /// The download is a zip, and only the named entries are taken out of it.
    Zip,
}

/// One file the installer fetches and places.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Download {
    /// Where the catalogue says the bytes are. Read by the download side,
    /// never by the engine.
    pub url: String,
    /// The digest the file is verified against before the engine sees it, and
    /// the model's identity in every key a result is filed under (§7).
    pub sha256: String,
    /// The size in bytes, checked with the digest and again on every scan.
    pub size: u64,
    /// Copied whole, or opened as a zip.
    pub unpack: Unpack,
    /// The file name inside the addon folder, for `unpack: file`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dest: String,
    /// Path inside the zip to file name inside the addon folder, for
    /// `unpack: zip`. Only these entries are taken.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entries: BTreeMap<String, String>,
}

/// What one platform needs downloading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    /// Unpacked in the order written, so a later entry may overwrite an
    /// earlier one deliberately.
    pub downloads: Vec<Download>,
}

/// How a depth model's numbers are to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DepthKind {
    /// Larger is nearer, with no unit and no zero. What Depth Anything emits.
    #[default]
    InverseRelative,
    /// Larger is further, in whatever unit the model was trained on.
    Metric,
}

/// The mean and standard deviation a model's input is normalised by, after
/// the picture has been scaled to 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Normalise {
    /// Subtracted, per channel, red first.
    pub mean: [f32; 3],
    /// Divided by, per channel, red first.
    pub std: [f32; 3],
}

/// What almost every vision model trained on ImageNet expects.
pub const IMAGENET: Normalise = Normalise {
    mean: [0.485, 0.456, 0.406],
    std: [0.229, 0.224, 0.225],
};

impl Default for Normalise {
    fn default() -> Self {
        IMAGENET
    }
}

/// RIFE: two frames and a phase in, one frame out (§4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rife {
    /// The `.onnx` file inside the addon folder.
    pub file: String,
    /// The earlier frame, `[1, 3, H, W]` RGB 0..1.
    pub img0: String,
    /// The later frame, the same shape.
    pub img1: String,
    /// Where between them to land, `[1]`.
    pub timestep: String,
    /// The synthesised frame, `[1, 3, H, W]`.
    pub output: String,
    /// Height and width are padded up to a multiple of this before the run.
    pub multiple: u32,
}

impl Default for Rife {
    fn default() -> Self {
        Rife {
            file: String::new(),
            img0: "img0".into(),
            img1: "img1".into(),
            timestep: "timestep".into(),
            output: "output".into(),
            multiple: 32,
        }
    }
}

/// Depth Anything: one frame in, one plane of depth out (§4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepthAnything {
    /// The `.onnx` file inside the addon folder.
    pub file: String,
    /// The frame, `[1, 3, h, w]`, normalised.
    pub input: String,
    /// The plane, `[1, h, w]`.
    pub output: String,
    /// The long side is resized to this before the run.
    pub size: u32,
    /// Both sides are then rounded to a multiple of this.
    pub multiple: u32,
    /// How the input is normalised.
    #[serde(flatten)]
    pub normalise: Normalise,
    /// How to read what comes back.
    pub output_kind: DepthKind,
}

impl Default for DepthAnything {
    fn default() -> Self {
        DepthAnything {
            file: String::new(),
            input: "pixel_values".into(),
            output: "predicted_depth".into(),
            size: 518,
            multiple: 14,
            normalise: IMAGENET,
            output_kind: DepthKind::InverseRelative,
        }
    }
}

/// Robust Video Matting: a sequence, with its own state carried frame to
/// frame (§4, and §13's warning that this one is not a set of frames).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rvm {
    /// The `.onnx` file inside the addon folder.
    pub file: String,
    /// The frame, `[1, 3, H, W]` 0..1.
    pub src: String,
    /// The four recurrent state inputs, zeros on the first frame.
    pub state_in: [String; 4],
    /// The four recurrent state outputs, fed back on the next frame.
    pub state_out: [String; 4],
    /// How far the model downsamples internally, `[1]`.
    pub downsample: String,
    /// The subject with its background removed, `[1, 3, H, W]`.
    pub foreground: String,
    /// Coverage, `[1, 1, H, W]`.
    pub matte: String,
}

impl Default for Rvm {
    fn default() -> Self {
        Rvm {
            file: String::new(),
            src: "src".into(),
            state_in: ["r1i".into(), "r2i".into(), "r3i".into(), "r4i".into()],
            state_out: ["r1o".into(), "r2o".into(), "r3o".into(), "r4o".into()],
            downsample: "downsample_ratio".into(),
            foreground: "fgr".into(),
            matte: "pha".into(),
        }
    }
}

/// BiRefNet: one square frame in, one plane of logits out (§4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Birefnet {
    /// The `.onnx` file inside the addon folder.
    pub file: String,
    /// The frame, `[1, 3, size, size]`, normalised.
    pub input: String,
    /// Logits, `[1, 1, size, size]`; a sigmoid turns them into coverage.
    pub output: String,
    /// The square the frame is resized to.
    pub size: u32,
    /// How the input is normalised.
    #[serde(flatten)]
    pub normalise: Normalise,
}

impl Default for Birefnet {
    fn default() -> Self {
        Birefnet {
            file: String::new(),
            input: "input_image".into(),
            output: "output_image".into(),
            size: 1024,
            normalise: IMAGENET,
        }
    }
}

/// SAM 2: an encoder run once on the frame, then a decoder run per prompt
/// (§4, and §13's warning that the decoder wants all three encoder outputs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sam2 {
    /// The encoder `.onnx` file inside the addon folder.
    pub encoder: String,
    /// The decoder `.onnx` file inside the addon folder.
    pub decoder: String,
    /// The frame the encoder takes, `[1, 3, size, size]`, normalised.
    pub image: String,
    /// The embedding it makes, `[1, 256, 64, 64]`.
    pub embed: String,
    /// The two finer feature maps the decoder also wants.
    pub high_res: [String; 2],
    /// The prompt's points, `[1, N, 2]`, in the resized frame's pixels.
    pub point_coords: String,
    /// One label per point: 1 for the subject, 0 against it.
    pub point_labels: String,
    /// A matte fed back in, `[1, 1, 256, 256]`.
    pub mask_input: String,
    /// Whether that matte means anything, `[1]`.
    pub has_mask_input: String,
    /// Three candidate mattes as logits, `[1, 3, 256, 256]`.
    pub masks: String,
    /// How good the model thinks each of the three is, `[1, 3]`.
    pub iou: String,
    /// The square the frame is resized to.
    pub size: u32,
    /// How the input is normalised.
    #[serde(flatten)]
    pub normalise: Normalise,
}

impl Default for Sam2 {
    fn default() -> Self {
        Sam2 {
            encoder: String::new(),
            decoder: String::new(),
            image: "image".into(),
            embed: "image_embed".into(),
            high_res: ["high_res_feats_0".into(), "high_res_feats_1".into()],
            point_coords: "point_coords".into(),
            point_labels: "point_labels".into(),
            mask_input: "mask_input".into(),
            has_mask_input: "has_mask_input".into(),
            masks: "masks".into(),
            iou: "iou_predictions".into(),
            size: 1024,
            normalise: IMAGENET,
        }
    }
}

/// The family whose tensor contract the engine speaks, and the names it
/// speaks it with. A re-export of the same architecture under different
/// tensor names is a catalogue edit, not a Lumit release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "arch", rename_all = "kebab-case")]
pub enum Arch {
    /// Frame synthesis, RIFE.
    Rife(Rife),
    /// Depth, Depth Anything.
    DepthAnything(DepthAnything),
    /// Matte, Robust Video Matting.
    Rvm(Rvm),
    /// Matte, BiRefNet.
    Birefnet(Birefnet),
    /// Segmentation, SAM 2.
    Sam2(Sam2),
}

impl Arch {
    /// The one task this family does. A manifest whose `task` says otherwise
    /// is refused, so `task` can never point the engine at the wrong code.
    #[must_use]
    pub fn task(&self) -> Task {
        match self {
            Arch::Rife(_) => Task::Synthesis,
            Arch::DepthAnything(_) => Task::Depth,
            Arch::Rvm(_) | Arch::Birefnet(_) => Task::Matte,
            Arch::Sam2(_) => Task::Segmentation,
        }
    }

    /// The `.onnx` files inside the addon folder this family names.
    #[must_use]
    pub fn files(&self) -> Vec<&str> {
        match self {
            Arch::Rife(m) => vec![m.file.as_str()],
            Arch::DepthAnything(m) => vec![m.file.as_str()],
            Arch::Rvm(m) => vec![m.file.as_str()],
            Arch::Birefnet(m) => vec![m.file.as_str()],
            Arch::Sam2(m) => vec![m.encoder.as_str(), m.decoder.as_str()],
        }
    }
}

/// The `model` block of a pack's manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    /// What the pack does, and what a control binds to.
    pub task: Task,
    /// Which family it is, and its tensors.
    #[serde(flatten)]
    pub arch: Arch,
}

/// One addon's `addon.json`, whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// The manifest format. [`FORMAT`] is what this build reads.
    pub format: u32,
    /// `[a-z0-9-]+`, and the folder name.
    pub id: String,
    /// Runtime or model.
    pub kind: Kind,
    /// What the page calls it.
    pub name: String,
    /// The addon's own version, shown beside the name.
    pub version: String,
    /// One line saying what it does.
    #[serde(default)]
    pub summary: String,
    /// An SPDX expression, or a short name with [`Manifest::licence_url`]
    /// beside it. Both are shown.
    #[serde(default)]
    pub licence: String,
    /// Where the licence can be read.
    #[serde(default)]
    pub licence_url: String,
    /// Where the model itself came from.
    #[serde(default)]
    pub homepage: String,
    /// The download size the page shows, in bytes.
    #[serde(default)]
    pub size: u64,
    /// Addon ids this one needs; today always the runtime.
    #[serde(default)]
    pub requires: Vec<String>,
    /// What to fetch, per platform key.
    pub platforms: BTreeMap<String, Platform>,
    /// The model block. Present for a pack, absent for the runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,
}

impl Manifest {
    /// The downloads for this machine: the exact platform key if the manifest
    /// has one, then `any`.
    #[must_use]
    pub fn platform(&self) -> Option<&Platform> {
        self.platforms
            .get(CURRENT_PLATFORM)
            .or_else(|| self.platforms.get("any"))
    }

    /// What the addon folder should hold once installed, with the size of
    /// each file that is a whole download. A zip entry's unpacked size is not
    /// in the manifest, so only its presence can be checked.
    #[must_use]
    pub fn expected(&self) -> Vec<(String, Option<u64>)> {
        let Some(platform) = self.platform() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for download in &platform.downloads {
            match download.unpack {
                Unpack::File => out.push((download.dest.clone(), Some(download.size))),
                Unpack::Zip => out.extend(
                    download
                        .entries
                        .values()
                        .map(|dest| (dest.clone(), None::<u64>)),
                ),
            }
        }
        out
    }

    /// The task this pack does, or `None` for the runtime.
    #[must_use]
    pub fn task(&self) -> Option<Task> {
        self.model.as_ref().map(|model| model.task)
    }
}

/// Read an `addon.json` and judge it whole (§4). Every rule the format has is
/// here, and nothing downstream re-checks any of them.
///
/// # Errors
///
/// [`MlError::Invalid`] with a sentence naming what was wrong, for every way
/// a manifest can be one Lumit will not install.
pub fn parse(text: &str) -> Result<Manifest, MlError> {
    let manifest: Manifest =
        serde_json::from_str(text).map_err(|e| MlError::Invalid(e.to_string()))?;

    if manifest.format > FORMAT {
        return Err(MlError::Invalid(format!(
            "this addon needs a newer build of Lumit: it is format {}, and this build reads {FORMAT}",
            manifest.format
        )));
    }
    if manifest.format == 0 {
        return Err(MlError::Invalid("the manifest has no format".into()));
    }
    if !is_id(&manifest.id) {
        return Err(MlError::Invalid(format!(
            "\"{}\" is not an addon id: lower-case letters, digits and hyphens, at most {ID_MAX} of them",
            manifest.id
        )));
    }
    if manifest.name.trim().is_empty() {
        return Err(MlError::Invalid("the addon has no name".into()));
    }
    for id in &manifest.requires {
        if !is_id(id) {
            return Err(MlError::Invalid(format!("\"{id}\" is not an addon id")));
        }
    }

    if manifest.platforms.is_empty() {
        return Err(MlError::Invalid(
            "the manifest lists nothing to download".into(),
        ));
    }
    for (key, platform) in &manifest.platforms {
        if !PLATFORM_KEYS.contains(&key.as_str()) {
            return Err(MlError::Invalid(format!("\"{key}\" is not a platform")));
        }
        if platform.downloads.is_empty() {
            return Err(MlError::Invalid(format!(
                "the {key} block lists nothing to download"
            )));
        }
        for download in &platform.downloads {
            check_download(download)?;
        }
    }
    if manifest.platform().is_none() {
        return Err(MlError::Invalid(
            "this addon has nothing for this machine".into(),
        ));
    }

    match (manifest.kind, &manifest.model) {
        (Kind::Runtime, Some(_)) => {
            return Err(MlError::Invalid("the runtime carries no model".into()))
        }
        (Kind::Model, None) => {
            return Err(MlError::Invalid("a model pack needs a model block".into()))
        }
        (Kind::Model, Some(model)) => {
            if model.task != model.arch.task() {
                return Err(MlError::Invalid(format!(
                    "this pack says it does {}, and its architecture does {}",
                    model.task,
                    model.arch.task()
                )));
            }
            for file in model.arch.files() {
                if !is_plain_name(file) {
                    return Err(MlError::Invalid(format!(
                        "\"{file}\" is not a file name inside the addon"
                    )));
                }
            }
        }
        (Kind::Runtime, None) => {}
    }

    Ok(manifest)
}

/// One download's own rules: it is verifiable, and everything it places is a
/// plain file name inside the addon folder.
fn check_download(download: &Download) -> Result<(), MlError> {
    if download.sha256.len() != 64 || !download.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(MlError::Invalid(
            "a download without a SHA-256 cannot be verified".into(),
        ));
    }
    if download.size == 0 {
        return Err(MlError::Invalid("a download with no size".into()));
    }
    match download.unpack {
        Unpack::File => {
            if !is_plain_name(&download.dest) {
                return Err(MlError::Invalid(format!(
                    "\"{}\" is not a file name inside the addon",
                    download.dest
                )));
            }
        }
        Unpack::Zip => {
            if download.entries.is_empty() {
                return Err(MlError::Invalid(
                    "a zip download that takes nothing out of the zip".into(),
                ));
            }
            for (inside, dest) in &download.entries {
                if inside.is_empty() {
                    return Err(MlError::Invalid("a zip entry with no path".into()));
                }
                if !is_plain_name(dest) {
                    return Err(MlError::Invalid(format!(
                        "\"{dest}\" is not a file name inside the addon"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Whether `id` is one: lower-case letters, digits and hyphens, and short
/// enough to be a folder name.
#[must_use]
pub fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Whether `name` is a plain file name and not a path. A manifest arrives
/// over the network, and this is what stops one writing outside the addon's
/// own folder.
#[must_use]
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.starts_with(' ')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A depth pack exactly as docs/impl/addons.md §4 writes it.
    fn good() -> String {
        r#"{
          "format": 1,
          "id": "depth-anything-v2-small",
          "kind": "model",
          "name": "Depth Anything V2 Small",
          "version": "1.0",
          "summary": "A depth map from a single frame",
          "licence": "Apache-2.0",
          "licence_url": "https://example.invalid/licence",
          "size": 99060839,
          "requires": ["runtime"],
          "platforms": {
            "any": {
              "downloads": [
                { "url": "https://example.invalid/model.onnx",
                  "sha256": "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c",
                  "size": 99060839,
                  "unpack": "file",
                  "dest": "model.onnx" }
              ]
            }
          },
          "model": {
            "task": "depth",
            "arch": "depth-anything",
            "file": "model.onnx",
            "input": "pixel_values",
            "output": "predicted_depth",
            "size": 518,
            "multiple": 14,
            "mean": [0.485, 0.456, 0.406],
            "std": [0.229, 0.224, 0.225],
            "output_kind": "inverse-relative"
          }
        }"#
        .to_string()
    }

    /// Swap one value in the written manifest for another, so each refusal
    /// below differs from the good one in exactly one place.
    fn with(from: &str, to: &str) -> String {
        let text = good();
        assert!(text.contains(from), "the fixture has no {from}");
        text.replace(from, to)
    }

    /// **A good manifest round trips.** Parsed, written back out and parsed
    /// again gives the same thing, which is what lets an install copy the
    /// catalogue's text into the folder unchanged.
    #[test]
    fn a_good_manifest_round_trips() {
        let parsed = parse(&good()).unwrap();
        assert_eq!(parsed.id, "depth-anything-v2-small");
        assert_eq!(parsed.kind, Kind::Model);
        assert_eq!(parsed.task(), Some(Task::Depth));
        assert_eq!(
            parsed.expected(),
            vec![("model.onnx".into(), Some(99_060_839))]
        );

        let written = serde_json::to_string(&parsed).unwrap();
        assert_eq!(parse(&written).unwrap(), parsed);
    }

    /// **The tensor names come from the table when the manifest is quiet.**
    /// A pack that names none of them still gets the ones the engine speaks,
    /// so the catalogue only writes a name when it differs.
    #[test]
    fn a_quiet_model_block_takes_the_defaults() {
        let text = with(
            r#""input": "pixel_values",
            "output": "predicted_depth",
            "size": 518,
            "multiple": 14,
            "mean": [0.485, 0.456, 0.406],
            "std": [0.229, 0.224, 0.225],
            "output_kind": "inverse-relative""#,
            r#""_": 0"#,
        );
        let model = parse(&text).unwrap().model.unwrap();
        let Arch::DepthAnything(depth) = model.arch else {
            panic!("the arch changed");
        };
        assert_eq!(depth.file, "model.onnx");
        assert_eq!(depth.input, "pixel_values");
        assert_eq!(depth.output, "predicted_depth");
        assert_eq!(depth.size, 518);
        assert_eq!(depth.multiple, 14);
        assert_eq!(depth.normalise, IMAGENET);
        assert_eq!(depth.output_kind, DepthKind::InverseRelative);
    }

    /// **A newer format is refused as newer, not as broken.** The sentence is
    /// the one thing the user sees, so it has to say to update Lumit rather
    /// than to redownload the pack.
    #[test]
    fn a_newer_format_is_refused_as_newer() {
        let refusal = parse(&with(r#""format": 1"#, r#""format": 2"#)).unwrap_err();
        let MlError::Invalid(why) = &refusal else {
            panic!("the wrong refusal: {refusal:?}");
        };
        assert!(why.contains("newer build"), "{why}");
    }

    /// **Everything else §4 rules out is refused by name.** One case per
    /// rule, each differing from the good manifest in one place.
    #[test]
    fn every_rule_in_the_format_refuses_by_name() {
        let cases = [
            (with(r#""kind": "model""#, r#""kind": "plugin""#), "kind"),
            (with(r#""task": "depth""#, r#""task": "painting""#), "task"),
            (
                with(
                    r#""id": "depth-anything-v2-small""#,
                    r#""id": "Depth Anything""#,
                ),
                "id",
            ),
            (
                with(r#""arch": "depth-anything""#, r#""arch": "midas""#),
                "arch",
            ),
            (
                with(r#""task": "depth""#, r#""task": "matte""#),
                "task against arch",
            ),
            (with(r#""any""#, r#""solaris-m68k""#), "platform key"),
            (
                with(
                    r#""sha256": "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c","#,
                    "",
                ),
                "no digest",
            ),
            (
                with(r#""dest": "model.onnx""#, r#""dest": "../model.onnx""#),
                "dest outside the folder",
            ),
            (
                with(r#""unpack": "file""#, r#""unpack": "zip""#),
                "zip with no entries",
            ),
            (
                with(r#""kind": "model""#, r#""kind": "runtime""#),
                "runtime with a model block",
            ),
        ];
        for (text, what) in cases {
            let refusal = parse(&text).unwrap_err();
            assert!(
                matches!(refusal, MlError::Invalid(_)),
                "{what} was not refused: {refusal:?}"
            );
        }
    }

    /// **The exact platform key wins over `any`.** A pack may carry a Windows
    /// build and a portable one, and the Windows machine must take the first.
    #[test]
    fn the_platform_pick_prefers_the_exact_key() {
        let text = with(
            r#""any": {"#,
            &format!(
                r#""{CURRENT_PLATFORM}": {{
                    "downloads": [
                      {{ "url": "https://example.invalid/exact.onnx",
                         "sha256": "0000000000000000000000000000000000000000000000000000000000000001",
                         "size": 10, "unpack": "file", "dest": "exact.onnx" }}
                    ]
                }},
                "any": {{"#
            ),
        );
        let manifest = parse(&text).unwrap();
        assert_eq!(manifest.platforms.len(), 2);
        assert_eq!(
            manifest.expected(),
            vec![("exact.onnx".into(), Some(10))],
            "the exact key is taken, not any"
        );
    }

    /// **A pack with neither key is refused.** §4's rule is that the app takes
    /// its own key, then `any`, and turns down a manifest with neither.
    /// Refused here rather than only at the install, because a folder holding
    /// one would otherwise list as a healthy addon that expects no files at
    /// all.
    #[test]
    fn a_manifest_for_another_platform_is_refused() {
        let other = if CURRENT_PLATFORM == "linux-x86_64" {
            "macos-aarch64"
        } else {
            "linux-x86_64"
        };
        let refusal = parse(&with(r#""any""#, &format!(r#""{other}""#))).unwrap_err();
        let MlError::Invalid(why) = &refusal else {
            panic!("the wrong refusal: {refusal:?}");
        };
        assert!(why.contains("this machine"), "{why}");
    }

    /// The five model blocks the catalogue publishes, copied from
    /// `lumit-addons/addons/<id>/addon.json`, with the task each one claims.
    ///
    /// The catalogue is a second repository and nothing in either tree reads
    /// the other's files, so this is the only place a block written one way
    /// and a struct written another are ever put together.
    const PUBLISHED: [(&str, Task); 5] = [
        (
            r#""task": "synthesis", "arch": "rife", "file": "rife.onnx",
               "img0": "img0", "img1": "img1", "timestep": "timestep",
               "output": "output", "multiple": 32"#,
            Task::Synthesis,
        ),
        (
            r#""task": "depth", "arch": "depth-anything", "file": "model.onnx",
               "input": "pixel_values", "output": "predicted_depth",
               "size": 518, "multiple": 14,
               "mean": [0.485, 0.456, 0.406], "std": [0.229, 0.224, 0.225],
               "output_kind": "inverse-relative""#,
            Task::Depth,
        ),
        (
            r#""task": "matte", "arch": "rvm", "file": "rvm.onnx",
               "src": "src",
               "state_in": ["r1i", "r2i", "r3i", "r4i"],
               "state_out": ["r1o", "r2o", "r3o", "r4o"],
               "downsample": "downsample_ratio",
               "foreground": "fgr", "matte": "pha""#,
            Task::Matte,
        ),
        (
            r#""task": "matte", "arch": "birefnet", "file": "model.onnx",
               "input": "input_image", "output": "output_image", "size": 1024,
               "mean": [0.485, 0.456, 0.406], "std": [0.229, 0.224, 0.225]"#,
            Task::Matte,
        ),
        (
            r#""task": "segmentation", "arch": "sam2",
               "encoder": "encoder.onnx", "decoder": "decoder.onnx",
               "image": "image", "embed": "image_embed",
               "high_res": ["high_res_feats_0", "high_res_feats_1"],
               "point_coords": "point_coords", "point_labels": "point_labels",
               "mask_input": "mask_input", "has_mask_input": "has_mask_input",
               "masks": "masks", "iou": "iou_predictions", "size": 1024,
               "mean": [0.485, 0.456, 0.406], "std": [0.229, 0.224, 0.225]"#,
            Task::Segmentation,
        ),
    ];

    /// **Every model block the catalogue publishes parses.** A key named one
    /// way on one side and another on the other is how two of the five packs
    /// once downloaded in full and then refused to install, and nothing in
    /// either repository would have caught it.
    #[test]
    fn every_model_block_the_catalogue_publishes_parses() {
        for (model, task) in PUBLISHED {
            let text = format!(
                r#"{{"format":1,"id":"pack","kind":"model","name":"A pack",
                    "version":"1","licence":"MIT","platforms":{{"any":{{"downloads":[
                      {{"url":"https://example.invalid/f",
                        "sha256":"0000000000000000000000000000000000000000000000000000000000000003",
                        "size":1,"unpack":"file","dest":"model.onnx"}}]}}}},
                    "model":{{{model}}}}}"#
            );
            match parse(&text) {
                Ok(parsed) => assert_eq!(parsed.task(), Some(task), "{model}"),
                Err(why) => {
                    panic!("the catalogue publishes a block this build refuses: {why}\n{model}")
                }
            }
        }
    }
}
