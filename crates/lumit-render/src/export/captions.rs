//! Captions at export: drawn into the picture, written beside the file, or
//! carried inside it as a subtitle stream.
//!
//! # In plain terms
//!
//! A caption layer is a Text layer, so by default it is drawn into the
//! picture like any other. Asked for a file instead, the export leaves the
//! comp's own caption layers out of the picture and writes what each one
//! shows across the export as an `.srt` or `.vtt` beside the video. Asked to
//! carry them, it writes each as a subtitle stream inside the video file.
//!
//! Only the comp's own layers count: a caption layer inside a precomp is part
//! of that precomp's picture. The captions are read frame by frame over the
//! frames the export writes, so they follow cuts, Retime and the layer's in
//! and out exactly as the picture would have, and their times are the
//! file's: the export's first frame is nought.
//!
//! # Thread role
//!
//! The export's own thread, and its writer for the embedded streams. Reads
//! the export's snapshot. The caption files are written before the picture
//! starts.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lumit_core::captions::{cues::Cue, show, srt, vtt, Captions};
use lumit_core::expression::ExpressionContext;
use lumit_core::model::{Composition, Document, Layer, LayerKind, ProjectItem};
use lumit_core::time::Rational;
use uuid::Uuid;

use super::{apply_render_overrides, frame::Span, ExportSpec, Sink};

/// What an export does with the comp's own caption layers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum CaptionsOut {
    /// Drawn into the picture like any Text layer.
    #[default]
    Burn,
    /// Left out of the picture and written beside the file.
    File(SubtitleFormat),
    /// Left out of the picture and carried inside the file as subtitle
    /// streams, where the format can hold them ([`super::FormatCaps::subtitles`]).
    Embed,
}

/// The kind of caption file written beside an export.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum SubtitleFormat {
    /// SubRip, `.srt`.
    Srt,
    /// WebVTT, `.vtt`.
    Vtt,
}

impl SubtitleFormat {
    /// The file extension it is written with.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            SubtitleFormat::Srt => "srt",
            SubtitleFormat::Vtt => "vtt",
        }
    }
}

/// The captions `layer` carries, when it is a caption layer.
fn captions_of(layer: &Layer) -> Option<&Arc<Captions>> {
    match &layer.kind {
        LayerKind::Text { document } => document.captions.as_ref(),
        _ => None,
    }
}

/// Whether `comp` has a caption layer of its own.
#[must_use]
pub fn has_caption_layer(comp: &Composition) -> bool {
    comp.layers.iter().any(|l| captions_of(l).is_some())
}

/// Refuse captions carried inside a file that cannot hold them.
pub(super) fn check(spec: &ExportSpec) -> Result<(), String> {
    if spec.captions == CaptionsOut::Embed && !spec.format.caps().subtitles {
        return Err(format!("{} cannot carry captions", spec.format.extension()));
    }
    Ok(())
}

/// The export's snapshot with the comp's own caption layers left out of the
/// picture, or `None` when they stay in it. Only their picture goes: a caption
/// layer makes no sound.
pub(super) fn leave_out(
    doc: &Arc<Document>,
    comp: Uuid,
    spec: &ExportSpec,
) -> Option<Arc<Document>> {
    let hides = |l: &Layer| l.switches.visible && captions_of(l).is_some();
    if spec.captions == CaptionsOut::Burn || !doc.comp(comp)?.layers.iter().any(hides) {
        return None;
    }
    let mut copy = Document::clone(doc);
    for item in &mut copy.items {
        if let ProjectItem::Composition(c) = item {
            if c.id == comp {
                for layer in c.layers.iter_mut().filter(|l| hides(l)) {
                    layer.switches.visible = false;
                }
            }
        }
    }
    Some(Arc::new(copy))
}

/// One caption layer as the export writes it.
struct Stream {
    language: String,
    cues: Vec<Cue>,
}

/// Every caption layer of `comp` the picture would have shown, top first,
/// with what each shows across the export on the file's clock. `doc` is the
/// export's snapshot with the render settings already applied. `stop` says
/// when the export has been cancelled, and the reading gives up.
fn streams(
    doc: &Arc<Document>,
    comp_id: Uuid,
    spec: &ExportSpec,
    stop: &dyn Fn() -> bool,
) -> Vec<Stream> {
    let Some(comp) = doc.comp(comp_id) else {
        return Vec::new();
    };
    let span = Span::of(comp, spec);
    let time = |n: usize| {
        i64::try_from(n)
            .ok()
            .and_then(|n| comp.frame_rate.time_of_frame(n).ok())
            .map_or(0.0, |t| t.0.to_f64())
    };
    // The moment each written frame shows, so the captions change where the
    // picture would have.
    let times: Vec<f64> = (0..span.total).map(|n| time(span.source(n))).collect();
    let shift = Rational::from_f64_on_grid(time(span.first), 1000).unwrap_or(Rational::ZERO);
    let back = |t: Rational| {
        t.checked_sub(shift)
            .unwrap_or(Rational::ZERO)
            .max(Rational::ZERO)
    };
    let solo = comp.layers.iter().any(|l| l.switches.solo);
    comp.layers
        .iter()
        .filter(|l| l.switches.visible && (!solo || l.switches.solo))
        .filter_map(|l| Some((l.id, captions_of(l)?)))
        .map(|(layer, captions)| {
            let context = ExpressionContext {
                document: Arc::clone(doc),
                comp: Some(comp_id),
                layer: Some(layer),
                comp_time: 0.0,
                current_depth: 0,
                inputs: None,
            };
            let cues = show::spans(captions, &context, &times, time(span.end), stop)
                .into_iter()
                .map(|mut cue| {
                    cue.start = back(cue.start);
                    cue.end = back(cue.end);
                    for word in &mut cue.words {
                        word.start = back(word.start);
                        word.end = back(word.end);
                    }
                    cue
                })
                .collect();
            Stream {
                language: captions.language.clone(),
                cues,
            }
        })
        .collect()
}

/// Where each of the caption files an export to `out` writes goes: beside it,
/// `<stem>.srt` for one, `<stem>.<language>.srt` for each of several. An image
/// sequence's go beside its frames under the name it was given.
fn sidecar_paths(out: &Path, format: SubtitleFormat, languages: &[&str]) -> Vec<PathBuf> {
    let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
    let ext = format.extension();
    if languages.len() == 1 {
        return vec![out.with_file_name(format!("{stem}.{ext}"))];
    }
    let mut names: Vec<String> = Vec::new();
    for language in languages {
        let base = if language.is_empty() { "und" } else { language };
        let mut name = base.to_owned();
        let mut n = 1;
        while names.contains(&name) {
            n += 1;
            name = format!("{base}-{n}");
        }
        names.push(name);
    }
    names
        .iter()
        .map(|name| out.with_file_name(format!("{stem}.{name}.{ext}")))
        .collect()
}

/// Whether a caption file an export to `out` with `captions` could write is
/// there already, so the question about writing over a file covers it too.
#[must_use]
pub fn sidecar_taken(out: &Path, captions: CaptionsOut) -> bool {
    let CaptionsOut::File(format) = captions else {
        return false;
    };
    let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
    let ext = format!(".{}", format.extension());
    let folder = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let Ok(entries) = lumit_ingress::files::list(folder) else {
        return false;
    };
    entries.iter().any(|(name, _)| {
        // `<stem>.srt`, or `<stem>.<language>.srt`.
        name.strip_prefix(stem)
            .and_then(|rest| rest.strip_suffix(ext.as_str()))
            .is_some_and(|middle| {
                middle.is_empty()
                    || middle
                        .strip_prefix('.')
                        .is_some_and(|m| !m.is_empty() && !m.contains('.'))
            })
    })
}

/// The caption files `spec` asks for beside `out`: where each goes and what
/// it says. `doc` is the export's own snapshot, before its caption layers are
/// left out of the picture.
pub(super) fn files(
    doc: &Arc<Document>,
    comp: Uuid,
    out: &Path,
    spec: &ExportSpec,
    stop: &dyn Fn() -> bool,
) -> Vec<(PathBuf, String)> {
    let CaptionsOut::File(format) = spec.captions else {
        return Vec::new();
    };
    let delivery = apply_render_overrides(doc, &spec.render_options());
    let streams = streams(delivery.as_ref().unwrap_or(doc), comp, spec, stop);
    let languages: Vec<&str> = streams.iter().map(|s| s.language.as_str()).collect();
    streams
        .iter()
        .zip(sidecar_paths(out, format, &languages))
        .map(|(stream, path)| {
            let text = match format {
                SubtitleFormat::Srt => srt::write(&stream.cues),
                SubtitleFormat::Vtt => vtt::write(&stream.cues, &stream.language),
            };
            (path, text)
        })
        .collect()
}

/// Write the caption files `spec` asks for beside `out`, before the picture
/// starts. Answers the files written, so a cancel can take them away again. A
/// file that fails takes the ones before it with it.
pub(super) fn write_files(
    doc: &Arc<Document>,
    comp: Uuid,
    out: &Path,
    spec: &ExportSpec,
    stop: &dyn Fn() -> bool,
) -> Result<Vec<PathBuf>, String> {
    let mut written: Vec<PathBuf> = Vec::new();
    let files = files(doc, comp, out, spec, stop);
    // Cancelled while they were read: nothing is written.
    if stop() {
        return Ok(written);
    }
    for (path, text) in files {
        if let Err(e) = lumit_ingress::files::write(&path, text.as_bytes()) {
            for done in &written {
                let _ = lumit_ingress::files::remove_file(done);
            }
            return Err(format!("{}: {e}", path.display()));
        }
        written.push(path);
    }
    Ok(written)
}

/// The caption layers an export carries inside its video file, and how far
/// into each it has written.
pub(super) struct Embedded {
    streams: Vec<Stream>,
    /// The next cue of each stream to write.
    next: Vec<usize>,
}

impl Embedded {
    /// The streams `spec` carries, read from the export's snapshot before its
    /// caption layers leave the picture. None unless it asks for them.
    pub(super) fn of(
        doc: &Arc<Document>,
        comp: Uuid,
        spec: &ExportSpec,
        stop: &dyn Fn() -> bool,
    ) -> Self {
        let streams = if spec.captions == CaptionsOut::Embed && spec.format.caps().subtitles {
            streams(doc, comp, spec, stop)
        } else {
            Vec::new()
        };
        let next = vec![0; streams.len()];
        Self { streams, next }
    }

    /// Each stream's language, in order.
    pub(super) fn languages(&self) -> Vec<String> {
        self.streams.iter().map(|s| s.language.clone()).collect()
    }

    /// Write every caption that starts before `upto` seconds into the file,
    /// so they go in beside the frames they belong with.
    pub(super) fn feed(&mut self, sink: &mut Sink, upto: f64) -> Result<(), String> {
        let Sink::Video { encoder, .. } = sink else {
            return Ok(());
        };
        let ms = |t: Rational| t.num().saturating_mul(1000) / t.den().max(1);
        for (at, (stream, next)) in self.streams.iter().zip(&mut self.next).enumerate() {
            while let Some(cue) = stream.cues.get(*next).filter(|c| c.start.to_f64() < upto) {
                encoder
                    .write_subtitle(at, ms(cue.start), ms(cue.end), &cue.lines())
                    .map_err(|e| e.to_string())?;
                *next += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::export::{start, ExportEvent, ExportFormat};
    use lumit_core::captions::cues::CueWord;

    /// A 32 x 16 solid comp at 30 fps with a caption layer over it saying
    /// "Hello" from one second to two, and the id of that layer.
    fn caption_doc() -> (Arc<Document>, Uuid, Uuid) {
        let (doc, comp_id) = super::super::tests::solid_doc(32, 16);
        let mut doc = Document::clone(&doc);
        let at = |s: i64| Rational::new(s, 1).unwrap();
        let captions = Captions {
            source: None,
            cues: vec![Cue {
                start: at(1),
                end: at(2),
                words: vec![CueWord {
                    text: "Hello".into(),
                    start: at(1),
                    end: at(2),
                    speaker: None,
                    emphasis: false,
                }],
                breaks: Vec::new(),
                label: None,
                dashes: false,
                item: None,
                offset: None,
                extra: serde_json::Map::new(),
            }],
            language: "en".into(),
            limits: Default::default(),
            speakers: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let caption_id = Uuid::now_v7();
        for item in &mut doc.items {
            if let ProjectItem::Composition(c) = item {
                let mut layer = c.layers[0].clone();
                layer.id = caption_id;
                layer.kind = LayerKind::Text {
                    document: lumit_core::model::TextDocument {
                        text: String::new(),
                        expression: None,
                        size: 24.0,
                        fill: lumit_core::model::LinearColour([1.0, 1.0, 1.0, 1.0]),
                        path: None,
                        path_offset: lumit_core::anim::Property::zero(),
                        animators: Vec::new(),
                        style: Default::default(),
                        paragraph: Default::default(),
                        captions: Some(Arc::new(captions.clone())),
                        extra: serde_json::Map::new(),
                    },
                };
                c.layers.insert(0, layer);
            }
        }
        (Arc::new(doc), comp_id, caption_id)
    }

    /// Run an export through to its end, or `None` on a machine with no
    /// graphics card.
    fn export(doc: &Arc<Document>, comp: Uuid, out: &Path, spec: ExportSpec) -> Option<()> {
        let handle = start(Arc::clone(doc), comp, Vec::new(), out.to_path_buf(), spec);
        for event in handle.events.iter() {
            match event {
                ExportEvent::Done(_) => return Some(()),
                ExportEvent::Failed(e) if e.starts_with("export renderer:") => {
                    lumit_gpu::no_adapter();
                    return None;
                }
                ExportEvent::Failed(e) => panic!("the export failed: {e}"),
                _ => {}
            }
        }
        panic!("the export ended without a word");
    }

    /// Captions as a file: the `.srt` holds the caption on the file's clock,
    /// and the picture is the one an export with no caption layer at all
    /// would have written, where burning it in would not have been.
    #[test]
    fn an_export_with_captions_as_a_file_writes_them_and_leaves_them_out_of_the_picture() {
        let (doc, comp, caption) = caption_doc();
        let mut without = Document::clone(&doc);
        for item in &mut without.items {
            if let ProjectItem::Composition(c) = item {
                c.layers.retain(|l| l.id != caption);
            }
        }
        let without = Arc::new(without);
        let dir = tempfile::tempdir().unwrap();
        // Comp frames 27 to 32: the caption comes in at frame 30, a tenth of
        // a second into the file, and is still up when it ends.
        let spec = |captions| ExportSpec {
            format: ExportFormat::Images(lumit_media::encode::ImageFormat::Png),
            include_audio: false,
            range: Some((27, 33)),
            captions,
            ..ExportSpec::default()
        };
        let file = CaptionsOut::File(SubtitleFormat::Srt);
        let (one, two, three) = (
            dir.path().join("with.png"),
            dir.path().join("without.png"),
            dir.path().join("burnt.png"),
        );
        let Some(()) = export(&doc, comp, &one, spec(file)) else {
            return;
        };
        export(&without, comp, &two, spec(file)).expect("an adapter was there a moment ago");
        export(&doc, comp, &three, spec(CaptionsOut::Burn)).expect("and still is");

        let srt = std::fs::read_to_string(dir.path().join("with.srt")).unwrap();
        assert_eq!(
            srt,
            "1
00:00:00,100 --> 00:00:00,200
Hello

"
        );
        assert!(
            !dir.path().join("without.srt").exists(),
            "no caption layer, no file"
        );
        assert!(!dir.path().join("burnt.srt").exists(), "burnt in, no file");
        let read =
            |p: &Path| std::fs::read(lumit_media::encode::sequence_frame_path(p, 6)).unwrap();
        assert_eq!(read(&one), read(&two), "the caption is not in the picture");
        assert_ne!(read(&three), read(&two), "burnt in, it is");
    }
}
