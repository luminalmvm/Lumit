//! Making caption layers: from a layer's speech, or from a subtitle file.

use super::*;

/// The words a layer plays, per footage item, in source time: each item's
/// transcript cut to the stretches of the file the layer plays.
fn heard_words(doc: &Document, source: &Layer) -> Result<Vec<(Uuid, Vec<CueWord>)>, BridgeError> {
    let mut spans: Vec<(Uuid, f64, f64)> = Vec::new();
    spans_of(doc, source, 0, &mut spans);
    if spans.is_empty() {
        return Err(BridgeError::NoAudio);
    }
    let mut out: Vec<(Uuid, Vec<CueWord>)> = Vec::new();
    for (item, _, _) in &spans {
        if out.iter().any(|(i, _)| i == item) {
            continue;
        }
        let transcript = doc.transcripts.get(item).ok_or(BridgeError::NoTranscript)?;
        let mine: Vec<(f64, f64)> = spans
            .iter()
            .filter(|(i, _, _)| i == item)
            .map(|(_, a, b)| (*a, *b))
            .collect();
        let words = transcript
            .words
            .iter()
            .filter(|w| {
                let (s, e) = (w.start.0.to_f64(), w.end.0.to_f64());
                mine.iter().any(|(a, b)| e > *a && s < *b)
            })
            .map(|w| CueWord {
                text: w.text.clone(),
                start: w.start.0,
                end: w.end.0,
                speaker: w.speaker,
                emphasis: false,
            })
            .collect();
        out.push((*item, words));
    }
    Ok(out)
}

/// Every stretch of footage `layer` plays, as the item and its source times,
/// down through the Precomp layers inside it.
fn spans_of(doc: &Document, layer: &Layer, depth: u32, out: &mut Vec<(Uuid, f64, f64)>) {
    if depth > 8 {
        return;
    }
    match &layer.kind {
        LayerKind::Footage { item } => {
            let lt = |t: f64| layer.source_time_at(t - layer.start_offset.0.to_f64());
            let (a, b) = (
                lt(layer.in_point.0.to_f64()),
                lt(layer.out_point.0.to_f64()),
            );
            out.push((*item, a.min(b), a.max(b)));
        }
        LayerKind::Sequence { clips } => {
            for clip in clips {
                if let lumit_core::sequence::ClipSource::Footage(item) = clip.source {
                    let (a, b) = (clip.source_in.to_f64(), clip.source_out.to_f64());
                    out.push((item, a.min(b), a.max(b)));
                }
            }
        }
        LayerKind::Precomp { comp } => {
            if let Some(nested) = doc.comp(*comp) {
                for inner in nested.layers.iter().filter(|l| l.switches.audible) {
                    spans_of(doc, inner, depth + 1, out);
                }
            }
        }
        _ => {}
    }
}

/// The usual place for captions: a box most of the comp wide and as tall as
/// the lines, sitting near the bottom, the lines centred and growing upwards.
fn placed_look(comp: &Composition, mut document: TextDocument, lines: u32) -> TextDocument {
    let paragraph = &mut document.paragraph;
    if paragraph.box_width <= 0.0 {
        paragraph.box_width = f64::from(comp.width) * 0.84;
    }
    paragraph.align = lumit_core::text::TextAlign::Centre;
    let leading = document.style.leading.unwrap_or(document.size * 1.2);
    #[allow(clippy::cast_possible_truncation)] // a box in pixels
    let height = (leading * f64::from(lines.max(1))) as f32;
    document.paragraph.box_height = height;
    document.paragraph.vertical = lumit_core::text::VerticalAlign::Bottom;
    document
}

/// A Text layer showing `captions`, styled as `look`, at the top of the stack.
fn caption_layer(
    comp_ref: &CompositionReference,
    comp: &Composition,
    name: String,
    look: crate::api::assets::BridgeTextDocument,
    captions: Captions,
) -> Result<LayerReference, BridgeError> {
    use lumit_core::anim::Property;
    use lumit_core::model::TransformGroup;
    let lines = captions.limits.lines;
    let mut document = placed_look(
        comp,
        crate::api::assets::text_document_of(look, Rational::ZERO)?,
        lines,
    );
    document.text = String::new();
    document.expression = None;
    // Measured with a line of the same size, so the anchor is the bottom of
    // the box wherever the words fall in it.
    let sample = captions
        .cues
        .iter()
        .map(Cue::text)
        .max_by_key(|t| t.chars().count())
        .unwrap_or_else(|| "Ag".to_owned());
    let box_ = lumit_text::layout(
        &lumit_text::TextBlock::of(&document, &sample),
        !document.animators.is_empty(),
    );
    let bottom = box_
        .lines
        .last()
        .map_or(box_.height as f32, |l| l.baseline + box_.descent);
    document.captions = Some(Arc::new(captions));
    let layer = crate::edits::base_layer(
        name,
        LayerKind::Text { document },
        comp.duration.0,
        TransformGroup {
            anchor_x: Property::fixed(f64::from(box_.left + box_.right) * 0.5),
            anchor_y: Property::fixed(f64::from(bottom)),
            position_x: Property::fixed(f64::from(comp.width) * 0.5),
            position_y: Property::fixed(f64::from(comp.height) * 0.925),
            ..TransformGroup::default()
        },
    );
    comp_ref.add_at(layer, None)
}

pub(crate) fn create(
    comp_ref: &CompositionReference,
    source: Uuid,
    options: BridgeCaptionOptions,
    look: crate::api::assets::BridgeTextDocument,
) -> Result<LayerReference, BridgeError> {
    let doc = comp_ref.document()?;
    let comp = doc.comp(comp_ref.id).ok_or(BridgeError::InvalidComp)?;
    let layer = comp
        .layers
        .iter()
        .find(|l| l.id == source)
        .ok_or(BridgeError::InvalidLayer)?;
    let heard = heard_words(&doc, layer)?;
    let limits = limits_in(comp, &options.limits);
    let mut speakers: Vec<lumit_core::transcript::Speaker> = Vec::new();
    let mut language = String::new();
    for (item, _) in &heard {
        if let Some(t) = doc.transcripts.get(item) {
            if language.is_empty() {
                language = t.language.clone();
            }
            for s in &t.speakers {
                if !speakers.iter().any(|k| k.id == s.id) {
                    speakers.push(s.clone());
                }
            }
        }
    }
    let names = |id: Uuid| speakers.iter().find(|s| s.id == id).map(|s| s.name.clone());
    let mut cues_out = Vec::new();
    for (item, words) in &heard {
        let words: Vec<CueWord> = if options.fillers {
            words.clone()
        } else {
            words
                .iter()
                .filter(|w| !is_filler(&w.text))
                .cloned()
                .collect()
        };
        for mut cue in cues::build(&words, &limits, &names) {
            cue.item = Some(*item);
            cues_out.push(cue);
        }
    }
    if cues_out.is_empty() {
        return Err(BridgeError::NoSpeech);
    }
    let captions = Captions {
        source: Some(source),
        cues: cues_out,
        language,
        limits,
        speakers,
        extra: serde_json::Map::new(),
    };
    caption_layer(comp_ref, comp, options.name, look, captions)
}

/// Whether a word is a filler: a sound like "um" more than a word.
fn is_filler(text: &str) -> bool {
    let bare: String = text
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect::<String>()
        .to_lowercase();
    matches!(
        bare.as_str(),
        "um" | "umm" | "uh" | "uhh" | "er" | "erm" | "ah" | "hmm" | "mm" | "mhm" | "eh"
    )
}

pub(crate) fn import(
    comp_ref: &CompositionReference,
    path: String,
    name: String,
    look: crate::api::assets::BridgeTextDocument,
) -> Result<LayerReference, BridgeError> {
    let bytes = lumit_ingress::files::read(std::path::Path::new(&path))
        .map_err(|_| BridgeError::CaptionsUnreadable(0))?;
    let cues_in = lumit_core::captions::read_bytes(&path, &bytes).map_err(|e| match e {
        lumit_core::captions::CaptionsError::Unreadable { line } => {
            BridgeError::CaptionsUnreadable(u32::try_from(line).unwrap_or(u32::MAX))
        }
        lumit_core::captions::CaptionsError::Empty => BridgeError::CaptionsUnreadable(0),
    })?;
    let doc = comp_ref.document()?;
    let comp = doc.comp(comp_ref.id).ok_or(BridgeError::InvalidComp)?;
    let captions = Captions {
        source: None,
        cues: lumit_core::captions::tidy(cues_in),
        language: String::new(),
        limits: Limits::default(),
        speakers: Vec::new(),
        extra: serde_json::Map::new(),
    };
    caption_layer(comp_ref, comp, name, look, captions)
}

pub(crate) fn heard_items(
    layer: &LayerReference,
) -> Result<Vec<crate::api::footage::FootageReference>, BridgeError> {
    let doc = layer.document()?;
    let comp = doc.comp(layer.comp_id).ok_or(BridgeError::InvalidComp)?;
    let found = comp
        .layers
        .iter()
        .find(|l| l.id == layer.layer_id)
        .ok_or(BridgeError::InvalidLayer)?;
    let mut spans = Vec::new();
    spans_of(&doc, found, 0, &mut spans);
    let mut items: Vec<Uuid> = Vec::new();
    for (item, _, _) in spans {
        if !items.contains(&item) {
            items.push(item);
        }
    }
    Ok(items
        .into_iter()
        .map(|item| crate::api::footage::FootageReference::new(layer.project_id, item))
        .collect())
}
