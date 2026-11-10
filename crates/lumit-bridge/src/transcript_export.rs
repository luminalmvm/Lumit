//! Writing a transcript to a file: plain text, a Word document, rich text,
//! Markdown, a web page, a spreadsheet's CSV, and subtitles.
//!
//! The words arrive gathered, in order, with their times and speakers. Each
//! writer only turns them into bytes, so the same words go out the same way
//! whichever panel asked.
//!
//! Runs on whichever thread asks. The export calls that reach it are not
//! sync, so that is never the UI thread.

use std::fmt::Write as _;
use std::io;

use lumit_core::captions::{cues, srt, vtt};
use lumit_core::interpretation::StartTimecode;
use lumit_core::time::{CompTime, FrameRate, Rational};
use uuid::Uuid;

mod docx;

/// One word to write.
pub(crate) struct Said {
    pub text: String,
    /// When it is said, in the time the transcript counts in.
    pub start: Rational,
    pub end: Rational,
    pub speaker: Option<String>,
    /// Who said it, for subtitles, which name a speaker at each change.
    pub who: Option<Uuid>,
    /// It starts a paragraph or somebody's turn.
    pub paragraph: bool,
}

/// The kinds of file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    Text,
    Word,
    RichText,
    Markdown,
    Html,
    Csv,
    SubRip,
    WebVtt,
}

/// What every writer is told.
pub(crate) struct Options<'a> {
    /// The page's title, where the file has one: the composition or the
    /// item's name.
    pub title: &'a str,
    pub language: &'a str,
    /// The speaker's name at each turn.
    pub speakers: bool,
    /// A timecode at each paragraph, on `rate`'s frames.
    pub timecodes: bool,
    /// For subtitles: the most characters on a line, and lines on screen.
    pub chars_per_line: u32,
    pub lines: u32,
    /// The frames subtitles keep apart and paragraph timecodes count.
    pub rate: FrameRate,
    /// What a paragraph's timecode reads at the first frame: the
    /// composition's Start timecode, or the file's own.
    pub start: StartTimecode,
}

/// The rate a paragraph's timecode counts at, and what it reads at the first
/// frame.
pub(crate) type Clock = (FrameRate, StartTimecode);

/// The words as `form`'s bytes.
pub(crate) fn write(form: Form, options: &Options<'_>, words: &[Said]) -> io::Result<Vec<u8>> {
    let speakers = options.speakers;
    let timecodes = options.timecodes.then_some((options.rate, options.start));
    Ok(match form {
        Form::Text => text(words, speakers, timecodes).into_bytes(),
        Form::Markdown => markdown(words, speakers, timecodes).into_bytes(),
        Form::Html => html(options, words).into_bytes(),
        Form::RichText => rich_text(words, speakers, timecodes).into_bytes(),
        Form::Csv => csv(words).into_bytes(),
        Form::Word => docx::write(words, speakers, timecodes)?,
        Form::SubRip => srt::write(&subtitles(options, words)).into_bytes(),
        Form::WebVtt => vtt::write(&subtitles(options, words), options.language).into_bytes(),
    })
}

/// The words cut into captions: the language's usual limits with the line
/// length and line count asked for, one second to seven on screen, and two
/// frames between one and the next.
fn subtitles(options: &Options<'_>, words: &[Said]) -> Vec<cues::Cue> {
    let mut limits = cues::Limits::for_language(options.language);
    if options.chars_per_line > 0 {
        limits.chars_per_line = options.chars_per_line;
    }
    if options.lines > 0 {
        limits.lines = options.lines;
    }
    limits.min_duration = Rational::ONE;
    limits.max_duration = Rational::new(7, 1).unwrap_or(Rational::ONE);
    limits.gap = options
        .rate
        .time_of_frame(2)
        .map_or(Rational::ZERO, |two| two.0);
    if options.speakers {
        limits.speaker_labels = cues::SpeakerLabels::Names;
    }
    let said: Vec<cues::CueWord> = words
        .iter()
        .map(|w| cues::CueWord {
            text: w.text.clone(),
            start: w.start,
            end: w.end,
            speaker: w.who,
            emphasis: false,
        })
        .collect();
    let names = |id: Uuid| {
        words
            .iter()
            .find(|w| w.who == Some(id))
            .and_then(|w| w.speaker.clone())
    };
    cues::build(&said, &limits, &names)
}

/// A run of words between two turns or paragraph breaks.
struct Paragraph<'a> {
    speaker: Option<&'a str>,
    start: Rational,
    text: String,
}

fn paragraphs(words: &[Said]) -> Vec<Paragraph<'_>> {
    let mut out: Vec<Paragraph<'_>> = Vec::new();
    for word in words {
        match out.last_mut() {
            Some(last) if !word.paragraph => {
                // A space only where the scripts want one: none between
                // Chinese, Japanese or Thai words.
                if cues::spaced(&last.text, &word.text) {
                    last.text.push(' ');
                }
                last.text.push_str(&word.text);
            }
            _ => out.push(Paragraph {
                speaker: word.speaker.as_deref(),
                start: word.start,
                text: word.text.clone(),
            }),
        }
    }
    out
}

/// `HH:MM:SS:FF` on `rate`'s frames, as the timeline reads the frame `t`
/// falls in: counted on from `start`.
fn timecode(t: Rational, (rate, start): Clock) -> String {
    let frame = u64::try_from(rate.frame_at(CompTime(t))).unwrap_or(0);
    StartTimecode {
        frame: start.frame.saturating_add(frame),
        drop_frame: start.drop_frame,
    }
    .text(rate)
}

/// [`timecode`] to the millisecond, the one it is in.
fn timecode_ms(t: Rational) -> String {
    let ms = FrameRate::new(1000, 1)
        .map_or(0, |rate| rate.frame_at(CompTime(t)))
        .max(0);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// The line that heads a paragraph, or `None` with nothing asked for.
/// `timecodes` is the clock a paragraph's timecode reads, when it has one.
fn heading(
    p: &Paragraph<'_>,
    speakers: bool,
    timecodes: Option<Clock>,
) -> Option<(Option<String>, Option<String>)> {
    let name = p.speaker.filter(|_| speakers).map(str::to_owned);
    let time = timecodes.map(|clock| timecode(p.start, clock));
    (name.is_some() || time.is_some()).then_some((name, time))
}

/// Plain text: paragraphs on blank lines, each under its speaker's name and
/// its timecode in brackets when asked.
pub(crate) fn text(words: &[Said], speakers: bool, timecodes: Option<Clock>) -> String {
    let mut out = String::new();
    for p in paragraphs(words) {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        if let Some((name, time)) = heading(&p, speakers, timecodes) {
            let parts: Vec<String> = name
                .into_iter()
                .chain(time.map(|t| format!("[{t}]")))
                .collect();
            out.push_str(&parts.join(" "));
            out.push('\n');
        }
        out.push_str(&p.text);
    }
    out
}

fn markdown(words: &[Said], speakers: bool, timecodes: Option<Clock>) -> String {
    let mut out = String::new();
    for p in paragraphs(words) {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        if let Some((name, time)) = heading(&p, speakers, timecodes) {
            let parts: Vec<String> = name
                .map(|n| format!("**{}**", md_escaped(&n)))
                .into_iter()
                .chain(time.map(|t| format!("*{t}*")))
                .collect();
            out.push_str(&parts.join(" "));
            out.push_str("  \n");
        }
        out.push_str(&md_escaped(&p.text));
    }
    out.push('\n');
    out
}

/// `text` as Markdown shows it written: each character that marks text up
/// escaped, and a line's start that would make a list or a heading.
fn md_escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|' | '~'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    // "- ", "+ " and "=" open a list or underline a heading, and "1. " or
    // "1) " a numbered list.
    let number = out.len() - out.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if out.starts_with(['-', '+', '=']) {
        out.insert(0, '\\');
    } else if number > 0 && out.get(number..).is_some_and(|r| r.starts_with(['.', ')'])) {
        out.insert(number, '\\');
    }
    out
}

/// `text` safe inside HTML or XML.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A page that stands on its own: no fonts, scripts or sheets from anywhere
/// else, and its look in its own attributes.
fn html(options: &Options<'_>, words: &[Said]) -> String {
    let mut out = format!(
        "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n</head>\n\
         <body style=\"font-family: system-ui, sans-serif; max-width: 42em; margin: 2em auto; padding: 0 1em; line-height: 1.6\">\n",
        escaped(options.language),
        escaped(options.title)
    );
    let timecodes = options.timecodes.then_some((options.rate, options.start));
    for p in paragraphs(words) {
        out.push_str("<p>");
        if let Some((name, time)) = heading(&p, options.speakers, timecodes) {
            if let Some(name) = name {
                let _ = write!(out, "<strong>{}</strong> ", escaped(&name));
            }
            if let Some(time) = time {
                let _ = write!(out, "<em style=\"opacity: 0.7\">{time}</em>");
            }
            out.push_str("<br>");
        }
        out.push_str(&escaped(&p.text));
        out.push_str("</p>\n");
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// `text` safe inside RTF: its three special characters escaped, and
/// anything past ASCII as a Unicode escape with a question mark for readers
/// that cannot show it.
fn rtf_escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_ascii() => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{}?", *unit as i16);
                }
            }
        }
    }
    out
}

fn rich_text(words: &[Said], speakers: bool, timecodes: Option<Clock>) -> String {
    let mut out = String::from("{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Arial;}}\\f0\\fs24\n");
    for p in paragraphs(words) {
        if let Some((name, time)) = heading(&p, speakers, timecodes) {
            if let Some(name) = name {
                let _ = write!(out, "{{\\b {}}} ", rtf_escaped(&name));
            }
            if let Some(time) = time {
                let _ = write!(out, "{{\\i {time}}}");
            }
            out.push_str("\\line\n");
        }
        out.push_str(&rtf_escaped(&p.text));
        out.push_str("\\par\\par\n");
    }
    out.push('}');
    out
}

/// A field for a CSV row, quoted when it has to be. One a spreadsheet would
/// read as a formula starts with an apostrophe.
fn field(text: &str) -> String {
    let text = if text.starts_with(['=', '+', '-', '@']) {
        format!("'{text}")
    } else {
        text.to_owned()
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// A row to a word: its start, its end, who said it and what. The byte order
/// mark tells a spreadsheet the file is UTF-8.
fn csv(words: &[Said]) -> String {
    let mut out = String::from("\u{feff}start,end,speaker,text\n");
    for w in words {
        let _ = writeln!(
            out,
            "{},{},{},{}",
            timecode_ms(w.start),
            timecode_ms(w.end),
            field(w.speaker.as_deref().unwrap_or_default()),
            field(&w.text)
        );
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn said(text: &str, ms: i64, speaker: Option<&str>, paragraph: bool) -> Said {
        Said {
            text: text.into(),
            start: Rational::new(ms, 1000).unwrap(),
            end: Rational::new(ms + 300, 1000).unwrap(),
            speaker: speaker.map(str::to_owned),
            who: None,
            paragraph,
        }
    }

    /// Every format writes every word, in order, and a Word document opens
    /// as a zip holding the three parts Word needs. A writer that dropped a
    /// word would hand over a transcript with a hole in it.
    #[test]
    fn every_format_writes_every_word() {
        let words = [
            said("Hello,", 61_000, Some("Ann"), true),
            said("you & me", 61_400, Some("Ann"), false),
            said("Café.", 3_723_000, Some("Bo"), true),
        ];
        let options = Options {
            title: "Scene",
            language: "en",
            speakers: true,
            timecodes: true,
            chars_per_line: 42,
            lines: 2,
            rate: FrameRate::new(25, 1).unwrap(),
            start: StartTimecode::default(),
        };
        let as_text = |form| String::from_utf8(write(form, &options, &words).unwrap()).unwrap();
        assert_eq!(
            as_text(Form::Text),
            "Ann [00:01:01:00]\nHello, you & me\n\nBo [01:02:03:00]\nCafé."
        );
        assert!(as_text(Form::Markdown).starts_with("**Ann** *00:01:01:00*  \nHello, you & me"));
        assert!(as_text(Form::Html).contains("<strong>Bo</strong> "));
        assert!(as_text(Form::Html).contains("Hello, you &amp; me"));
        assert!(as_text(Form::RichText).contains("Caf\\u233?."));
        assert_eq!(
            as_text(Form::Csv).lines().nth(1),
            Some("00:01:01.000,00:01:01.300,Ann,\"Hello,\"")
        );
        let srt = as_text(Form::SubRip);
        assert!(
            srt.starts_with(
                "1
00:01:01,000 --> "
            ) && srt.contains("Café."),
            "{srt}"
        );
        assert!(as_text(Form::WebVtt).starts_with("WEBVTT"));
        let docx = write(Form::Word, &options, &words).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
        for part in ["[Content_Types].xml", "_rels/.rels", "word/document.xml"] {
            assert!(zip.by_name(part).is_ok(), "{part}");
        }
        let mut body = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut body)
            .unwrap();
        assert!(body.contains("you &amp; me") && body.contains("<w:b/>"));
    }
}
