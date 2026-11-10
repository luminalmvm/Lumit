//! Writing a caption layer's captions out as a subtitle file, on the comp's
//! clock.

use super::*;

/// Every caption `layer` shows in its comp, on the comp's clock, frame by
/// frame from `from` up to `to` (comp frames), shifted so `from` is nought.
pub(crate) fn comp_cues(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: Uuid,
    from: i64,
    to: i64,
) -> Vec<Cue> {
    let Some(found) = comp.layers.iter().find(|l| l.id == layer) else {
        return Vec::new();
    };
    let LayerKind::Text { document } = &found.kind else {
        return Vec::new();
    };
    let Some(captions) = document.captions.as_ref() else {
        return Vec::new();
    };
    let time = |n: i64| {
        comp.frame_rate
            .time_of_frame(n)
            .map_or(0.0, |t| t.0.to_f64())
    };
    let times: Vec<f64> = (from..to).map(time).collect();
    let context = lumit_core::expression::ExpressionContext {
        document: Arc::clone(doc),
        comp: Some(comp.id),
        layer: Some(layer),
        comp_time: 0.0,
        current_depth: 0,
        inputs: None,
    };
    let start = time(from);
    let shift = Rational::from_f64_on_grid(start, 1000).unwrap_or(Rational::ZERO);
    show::spans(captions, &context, &times, time(to), &|| false)
        .into_iter()
        .map(|mut cue| {
            let back = |t: Rational| {
                t.checked_sub(shift)
                    .unwrap_or(Rational::ZERO)
                    .max(Rational::ZERO)
            };
            cue.start = back(cue.start);
            cue.end = back(cue.end);
            for word in &mut cue.words {
                word.start = back(word.start);
                word.end = back(word.end);
            }
            cue
        })
        .collect()
}

/// `cues` as the text of a file in `format`.
pub(crate) fn file_text(cues: &[Cue], format: BridgeCaptionFormat, language: &str) -> String {
    match format {
        BridgeCaptionFormat::Srt => srt::write(cues),
        BridgeCaptionFormat::Vtt => vtt::write(cues, language),
        BridgeCaptionFormat::Ttml => lumit_core::captions::ttml::write(cues, language),
        BridgeCaptionFormat::Txt => cues
            .iter()
            .map(|c| c.lines().join(" "))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

pub(crate) fn export(
    layer: &LayerReference,
    format: BridgeCaptionFormat,
    path: String,
) -> Result<(), BridgeError> {
    let held = held(layer)?;
    let frames = held
        .comp
        .frame_rate
        .frame_at(lumit_core::time::CompTime(held.comp.duration.0));
    let cues = comp_cues(&held.doc, &held.comp, layer.layer_id, 0, frames);
    let text = file_text(&cues, format, &held.captions.language);
    lumit_ingress::files::write(std::path::Path::new(&path), text.as_bytes())
        .map_err(|_| BridgeError::WriteFailed)
}
