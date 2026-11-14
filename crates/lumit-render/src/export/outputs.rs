//! More than one file from one export: the files written beside the chosen
//! one, from the same rendered frames. Runs on the export's thread.

use std::path::{Path, PathBuf};

use super::captions::CaptionsOut;
use super::format::ExportFormat;
use super::{Bitrate, Channels, ColourSpace, ExportSpec};

/// The most files one export writes beside its own.
pub const MOST_ALSO: usize = 3;

/// One more file an export writes beside its own, from the same frames.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AlsoWrite {
    /// What it is written as: a video or a still sequence.
    pub format: ExportFormat,
    /// Added to the chosen file's name, before its extension.
    pub suffix: String,
}

/// The settings `also` is written with: the chosen file's, in its own
/// format. Its bitrate is worked out for its own codec, and a sound depth, an
/// alpha channel or embedded captions it cannot carry are left out rather than
/// refused.
pub fn spec_for(primary: &ExportSpec, also: &AlsoWrite) -> ExportSpec {
    let caps = also.format.caps();
    let mut spec = primary.clone();
    spec.format = also.format;
    spec.also = Vec::new();
    spec.stems = false;
    spec.bitrate = Bitrate::Auto;
    if !caps.audio_depths.contains(&spec.audio_depth) {
        if let Some(depth) = caps.audio_depths.first() {
            spec.audio_depth = *depth;
        }
    }
    if !caps.alpha {
        spec.channels = Channels::Rgb;
    }
    // The picture is the chosen file's, caption layers left out or not, so
    // this only stops a still sequence asking for subtitle streams.
    if spec.captions == CaptionsOut::Embed && !caps.subtitles {
        spec.captions = CaptionsOut::Burn;
    }
    spec
}

/// Where `also` is written beside a file at `primary`: the same folder and
/// name, the suffix, then its own extension.
pub fn path_for(primary: &Path, also: &AlsoWrite) -> PathBuf {
    let stem = primary
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    primary.with_file_name(format!("{stem}{}.{}", also.suffix, also.format.extension()))
}

/// Refuse files that cannot be written beside the chosen one. They share one
/// render, so each has to take the chosen file's frames, depth and colour as
/// they are.
pub(super) fn check(spec: &ExportSpec) -> Result<(), String> {
    if spec.also.is_empty() {
        return Ok(());
    }
    if spec.also.len() > MOST_ALSO {
        return Err(format!(
            "an export writes at most {MOST_ALSO} other files beside its own"
        ));
    }
    // A GIF's rate is capped, so it would have fewer frames than the rest.
    let picture = |f: ExportFormat| matches!(f, ExportFormat::Video(_) | ExportFormat::Images(_));
    if !picture(spec.format) {
        return Err(format!(
            "other files cannot be written beside {}",
            spec.format.extension()
        ));
    }
    let mut names = vec![(spec.format.extension().to_lowercase(), String::new())];
    for also in &spec.also {
        let ext = also.format.extension();
        if !picture(also.format) {
            return Err(format!("{ext} cannot be written beside another file"));
        }
        if also
            .suffix
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        {
            return Err(format!("\"{}\" cannot be part of a file name", also.suffix));
        }
        let name = (ext.to_lowercase(), also.suffix.to_lowercase());
        if names.contains(&name) {
            return Err(format!("two {ext} files would have the same name"));
        }
        names.push(name);
        // A config's own space is the chosen file's, checked against the
        // config by `check_with_colour`, so it is set aside here as there.
        let mut extra = spec_for(spec, also);
        if extra.colour_space.ocio_name().is_some() {
            extra.colour_space = ColourSpace::default();
        }
        extra.check()?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::tests::{run_now, solid_doc, spec};
    use super::super::{start, ExportEvent};
    use super::*;
    use lumit_media::encode::{sequence_frame_path, ImageFormat, VideoCodec};

    const PNG: ExportFormat = ExportFormat::Images(ImageFormat::Png);

    /// One export writes an mp4 and a PNG sequence from the same frames: as
    /// many frames in the one as stills in the other, and the stills the same
    /// bytes a PNG export alone writes.
    #[test]
    fn one_export_writes_a_video_and_stills_from_the_same_frames() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.mp4");
        let mut both = spec(ExportFormat::Video(VideoCodec::H264), 32, 16);
        both.range = Some((0, 6));
        both.also = vec![AlsoWrite {
            format: PNG,
            suffix: String::new(),
        }];
        let Some(result) = run_now(&doc, comp, &path, &both) else {
            return;
        };
        result.expect("both files are written");

        let alone_dir = dir.path().join("alone");
        std::fs::create_dir(&alone_dir).unwrap();
        let alone = alone_dir.join("shot.png");
        let mut stills_only = spec(PNG, 32, 16);
        stills_only.range = Some((0, 6));
        run_now(&doc, comp, &alone, &stills_only)
            .unwrap()
            .expect("the stills alone are written");

        let stills = dir.path().join("shot.png");
        let count = (1..)
            .take_while(|n| sequence_frame_path(&stills, *n).exists())
            .count();
        assert_eq!(count, 6, "six frames were asked for");
        let frames = lumit_media::index::build_frame_index(&path)
            .unwrap()
            .frame_count();
        assert_eq!(frames, count, "the video has a frame for every still");
        for n in 1..=count {
            assert_eq!(
                std::fs::read(sequence_frame_path(&stills, n)).unwrap(),
                std::fs::read(sequence_frame_path(&alone, n)).unwrap(),
                "still {n} differs from the one a PNG export writes"
            );
        }
    }

    /// A file that cannot be opened stops the export before its first frame,
    /// and takes the files already opened with it.
    #[test]
    fn an_extra_that_cannot_open_leaves_nothing_behind() {
        let (doc, comp) = solid_doc(32, 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.mp4");
        let blocked = dir.path().join("shot_b.mp4");
        std::fs::create_dir(&blocked).unwrap();
        let mut sp = spec(ExportFormat::Video(VideoCodec::H264), 32, 16);
        sp.range = Some((0, 6));
        sp.also = vec![
            AlsoWrite {
                format: PNG,
                suffix: String::new(),
            },
            AlsoWrite {
                format: ExportFormat::Video(VideoCodec::H264),
                suffix: "_b".into(),
            },
        ];
        let handle = start(doc, comp, Vec::new(), path.clone(), sp);
        let error = loop {
            match handle.events.recv().unwrap() {
                ExportEvent::Failed(e) => break e,
                ExportEvent::Done(_) => panic!("a folder is in the way"),
                _ => {}
            }
        };
        if error.starts_with("export renderer:") {
            lumit_gpu::no_adapter();
            return;
        }
        assert!(!path.exists(), "the chosen file was removed");
        assert!(
            !sequence_frame_path(&dir.path().join("shot.png"), 1).exists(),
            "no stills were written"
        );
        assert!(blocked.is_dir(), "the folder in the way is left alone");
    }
}
