//! The words an export or a copy as text writes: gathered from the tokens
//! under the frames asked for, and written out in the format asked for.

use super::*;

/// The words of `tokens` whose stretch lies in one of `spans`, as the
/// writers take them. `fillers` keeps the filler words.
#[frb(ignore)]
pub(super) fn said_in(
    doc: &Document,
    tokens: &[Token],
    spans: &[(Rational, Rational)],
    fillers: bool,
) -> Vec<Said> {
    const ENDS: [char; 7] = ['.', '?', '!', '…', '。', '？', '！'];
    let mut out: Vec<Said> = Vec::new();
    let mut last_speaker: Option<Uuid> = None;
    // What the filler words left out since the last word kept owe the next
    // one: their paragraph break, and a capital when they opened a sentence.
    let mut owed_paragraph = false;
    let mut owed_capital = false;
    // The last word read ended a sentence, so the next one opens one.
    let mut opens = true;
    for token in tokens.iter().filter(|t| t.kind == TokenKind::Word) {
        let mid = spoken::middle(token.start, token.end);
        let inside = spans.iter().any(|(a, b)| *a <= mid && mid < *b);
        let Some(at) = token.word else {
            continue;
        };
        let Some(transcript) = doc.transcripts.get(&at.item) else {
            continue;
        };
        let Some(word) = transcript.words.get(at.index) else {
            continue;
        };
        let opened = opens || word.paragraph;
        opens = word.text.ends_with(ENDS);
        if !inside {
            (owed_paragraph, owed_capital) = (false, false);
            continue;
        }
        if !fillers && token.filler.is_some() {
            owed_paragraph |= word.paragraph;
            owed_capital |= opened;
            // "about, um, building" reads "about building": the comma
            // before a filler word belonged to it. A filler word that
            // closed the sentence hands its full stop back.
            if let Some(last) = out.last_mut() {
                if last.text.ends_with(',') {
                    last.text.pop();
                }
                if let Some(end) = word.text.chars().last().filter(|c| ENDS.contains(c)) {
                    if !last.text.ends_with(ENDS) {
                        last.text.push(end);
                    }
                }
            }
            continue;
        }
        let mut text = word.text.clone();
        if std::mem::take(&mut owed_capital) {
            let mut letters = text.chars();
            if let Some(first) = letters.next() {
                text = first.to_uppercase().chain(letters).collect();
            }
        }
        let paragraph = word.paragraph || std::mem::take(&mut owed_paragraph);
        let (start, end) = token.heard.unwrap_or((token.start, token.end));
        let turn = word.speaker.is_some() && word.speaker != last_speaker;
        last_speaker = word.speaker.or(last_speaker);
        out.push(Said {
            text,
            start,
            end,
            speaker: word
                .speaker
                .and_then(|id| transcript.speaker_name(id))
                .map(str::to_owned),
            who: word.speaker,
            paragraph: paragraph || turn,
        });
    }
    out
}

/// Write `words` to `path` as `how` asks. `title` names a page that has one,
/// and `start` is what its clock reads at the first frame.
#[frb(ignore)]
pub(super) fn write_out(
    path: &str,
    title: &str,
    language: &str,
    words: &[Said],
    how: &BridgeTranscriptExport,
    rate: FrameRate,
    start: StartTimecode,
) -> Result<(), BridgeError> {
    use transcript_export::Form;
    if path.trim().is_empty() {
        return Err(BridgeError::EmptyPath);
    }
    let form = match how.format {
        BridgeTranscriptFormat::Text => Form::Text,
        BridgeTranscriptFormat::Word => Form::Word,
        BridgeTranscriptFormat::RichText => Form::RichText,
        BridgeTranscriptFormat::Markdown => Form::Markdown,
        BridgeTranscriptFormat::Html => Form::Html,
        BridgeTranscriptFormat::Csv => Form::Csv,
        BridgeTranscriptFormat::SubRip => Form::SubRip,
        BridgeTranscriptFormat::WebVtt => Form::WebVtt,
    };
    let bytes = transcript_export::write(
        form,
        &transcript_export::Options {
            title,
            language,
            speakers: how.speakers,
            timecodes: how.timecodes,
            chars_per_line: how.chars_per_line,
            lines: how.lines,
            rate,
            start,
        },
        words,
    )
    .map_err(|e| BridgeError::ExportFailed(e.to_string()))?;
    std::fs::write(path, bytes).map_err(|e| BridgeError::ExportFailed(e.to_string()))
}
