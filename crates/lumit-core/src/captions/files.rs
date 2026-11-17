//! What the subtitle file formats share: their time stamps, their tags, and
//! turning a caption's lines back into timed words.
//!
//! # Thread role and contract
//!
//! Pure text in and out. No IO.

use crate::time::Rational;

use super::cues::{Cue, CueWord};

/// Which way a time stamp is written.
#[derive(Clone, Copy)]
pub(super) enum Stamp {
    /// `00:01:02,500`
    Srt,
    /// `00:01:02.500`
    Vtt,
}

/// `at` to the millisecond, as the format writes it. Before nought reads as
/// nought.
pub(super) fn stamp(at: Rational, how: Stamp) -> String {
    let num = i128::from(at.num().max(0));
    let den = i128::from(at.den());
    let ms = (num * 1000 + den / 2) / den;
    let (hours, rest) = (ms / 3_600_000, ms % 3_600_000);
    let (minutes, rest) = (rest / 60_000, rest % 60_000);
    let (seconds, ms) = (rest / 1000, rest % 1000);
    let mark = match how {
        Stamp::Srt => ',',
        Stamp::Vtt => '.',
    };
    format!("{hours:02}:{minutes:02}:{seconds:02}{mark}{ms:03}")
}

/// A time stamp of either format, hours optional, as an exact time.
pub(super) fn parse_stamp(text: &str) -> Option<Rational> {
    let text = text.trim();
    let (clock, fraction) = match text.rfind([',', '.']) {
        Some(at) => (&text[..at], &text[at + 1..]),
        None => (text, ""),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut seconds: i64 = 0;
    for part in &parts {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        seconds = seconds
            .checked_mul(60)?
            .checked_add(part.parse::<i64>().ok()?)?;
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) || fraction.len() > 9 {
        return None;
    }
    let digits = u32::try_from(fraction.len()).ok()?;
    let scale = 10_i64.checked_pow(digits)?;
    let part = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().ok()?
    };
    Rational::new(seconds.checked_mul(scale)?.checked_add(part)?, scale).ok()
}

/// What Windows-1252 puts at 0x80 to 0x9F, where Latin-1 has control codes.
/// The five it leaves empty stay as they are.
const WINDOWS_1252: [u32; 32] = [
    0x20AC, 0x81, 0x201A, 0x192, 0x201E, 0x2026, 0x2020, 0x2021, 0x2C6, 0x2030, 0x160, 0x2039,
    0x152, 0x8D, 0x17D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x2DC,
    0x2122, 0x161, 0x203A, 0x153, 0x9D, 0x17E, 0x178,
];

/// A file's bytes as text: UTF-8 or UTF-16 by the mark at its start, UTF-8
/// without one, and Windows-1252 when it is not UTF-8, which is what older
/// subtitle tools wrote.
pub(super) fn text_of(bytes: &[u8]) -> String {
    let utf16 = |bytes: &[u8], big: bool| {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|p| {
                let pair = [p[0], p[1]];
                if big {
                    u16::from_be_bytes(pair)
                } else {
                    u16::from_le_bytes(pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        _ => match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => bytes
                .iter()
                .map(|&b| match b {
                    0x80..=0x9F => WINDOWS_1252
                        .get(usize::from(b - 0x80))
                        .and_then(|&c| char::from_u32(c))
                        .unwrap_or(char::from(b)),
                    _ => char::from(b),
                })
                .collect(),
        },
    }
}

/// The entities subtitle files use, written out.
pub(super) fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(end) = after.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let name = &after[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "lrm" | "rlm" => None,
            _ => {
                let code = name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .map_or_else(
                        || name.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()),
                        |hex| u32::from_str_radix(hex, 16).ok(),
                    );
                match code.and_then(char::from_u32) {
                    Some(c) => Some(c),
                    None => {
                        out.push('&');
                        rest = &after[1..];
                        continue;
                    }
                }
            }
        };
        if let Some(ch) = ch {
            out.push(ch);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A caption line without its markup: tags in angle brackets, override blocks
/// in braces, entities written out, and runs of space made one.
pub(super) fn clean(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut depth_angle = false;
    let mut depth_brace = false;
    for ch in line.chars() {
        match ch {
            '<' if !depth_brace => depth_angle = true,
            '>' if depth_angle => depth_angle = false,
            '{' if !depth_angle => depth_brace = true,
            '}' if depth_brace => depth_brace = false,
            _ if depth_angle || depth_brace => {}
            _ => plain.push(ch),
        }
    }
    decode_entities(&plain)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `c` is punctuation that leans on the word before it.
fn leans(c: char) -> bool {
    !c.is_alphanumeric() && !c.is_whitespace()
}

/// A line's words: split at spaces, and every character of a script written
/// without spaces made a word of its own, so a caption in Japanese can still be
/// coloured as it is said. Punctuation stays with the word before it.
fn words_of(line: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in line.chars() {
        if ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else if !super::cues::unspaced(ch) {
            current.push(ch);
        } else if leans(ch) && (!current.is_empty() || !words.is_empty()) {
            if current.is_empty() {
                if let Some(last) = words.last_mut() {
                    last.push(ch);
                }
            } else {
                current.push(ch);
            }
        } else {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            words.push(ch.to_string());
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// A caption from its lines and times, its words spread over it by length:
/// a file without word timing still colours them in turn as it plays.
///
/// Lines that all start with a dash are one speaker each, and the dashes are
/// put back when the caption is shown or written. `None` for no words at all.
pub(super) fn cue_of_lines<S: AsRef<str>>(
    lines: &[S],
    start: Rational,
    end: Rational,
) -> Option<Cue> {
    let lines: Vec<&str> = lines
        .iter()
        .map(|l| l.as_ref().trim())
        .filter(|l| !l.is_empty())
        .collect();
    let dashes = lines.len() > 1 && lines.iter().all(|l| l.starts_with('-'));
    let mut texts: Vec<String> = Vec::new();
    let mut breaks = Vec::new();
    for line in &lines {
        let line = if dashes {
            line.trim_start_matches('-').trim_start()
        } else {
            line
        };
        let words = words_of(line);
        if words.is_empty() {
            continue;
        }
        if !texts.is_empty() {
            breaks.push(texts.len());
        }
        texts.extend(words);
    }
    if texts.is_empty() {
        return None;
    }
    let end = end.max(start);
    let weights: Vec<i64> = texts
        .iter()
        .map(|t| i64::try_from(t.chars().count()).unwrap_or(1) + 1)
        .collect();
    let total: i64 = weights.iter().sum();
    let span = end.checked_sub(start).unwrap_or(Rational::ZERO);
    let at = |weight: i64| -> Rational {
        Rational::new(weight, total.max(1))
            .and_then(|share| span.checked_mul(share))
            .and_then(|offset| start.checked_add(offset))
            .unwrap_or(start)
    };
    let mut gone = 0;
    let words = texts
        .into_iter()
        .zip(&weights)
        .map(|(text, weight)| {
            let word = CueWord {
                text,
                start: at(gone),
                end: at(gone + weight),
                speaker: None,
                emphasis: false,
            };
            gone += weight;
            word
        })
        .collect();
    Some(Cue {
        start,
        end,
        words,
        breaks,
        label: None,
        dashes,
        item: None,
        offset: None,
        extra: serde_json::Map::new(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::{srt, vtt};

    /// **A file written and read back says the same thing at the same
    /// times**, word times included for WebVTT. A subtitle file is what
    /// somebody sends on with the video, and one that lost or moved words on
    /// the way would be sent on wrong.
    #[test]
    fn a_file_written_and_read_back_says_the_same_at_the_same_times() {
        let srt_text =
            "1\r\n00:00:01,000 --> 00:00:03,500\r\n<i>Hello</i> there,\r\nhow are you?\r\n\r\n\
                        00:00:04.250 --> 00:00:06,000 X1:10\n- Fine.\n- Good &amp; you?\n";
        let cues = super::super::read_file("Talk.srt", srt_text).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].lines(), vec!["Hello there,", "how are you?"]);
        assert_eq!(cues[1].lines(), vec!["- Fine.", "- Good & you?"]);

        let again = srt::read(&srt::write(&cues)).unwrap();
        assert_eq!(again, cues);

        let web = vtt::write(&cues, "en");
        assert!(web.starts_with("WEBVTT\nLanguage: en\n"), "{web}");
        let back = super::super::read_file("Talk.vtt", &web).unwrap();
        assert_eq!(back.len(), 2);
        for (a, b) in back.iter().zip(&cues) {
            assert_eq!(a.lines(), b.lines());
            assert_eq!((a.start, a.end), (b.start, b.end));
            let times = |c: &super::super::cues::Cue| -> Vec<_> {
                c.words.iter().map(|w| w.start).collect()
            };
            // Written to the millisecond, so read back to it.
            let ms = |c: &super::super::cues::Cue| -> Vec<i64> {
                times(c)
                    .iter()
                    .map(|t| (t.to_f64() * 1000.0).round() as i64)
                    .collect()
            };
            assert_eq!(ms(a), ms(b));
        }

        let timed = super::super::ttml::write(&cues, "en");
        let back = super::super::read_file("Talk.dfxp", &timed).unwrap();
        assert_eq!(
            back.iter()
                .map(super::super::cues::Cue::lines)
                .collect::<Vec<_>>(),
            cues.iter()
                .map(super::super::cues::Cue::lines)
                .collect::<Vec<_>>()
        );
        assert_eq!(back[1].start, cues[1].start);

        let ass = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
                   Dialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\i1}Well,{\\i0} hello\\Nthere\n";
        let read = super::super::read_file("Talk.ass", ass).unwrap();
        assert_eq!(read[0].lines(), vec!["Well, hello", "there"]);
        assert_eq!(read[0].start, crate::time::Rational::new(3, 2).unwrap());

        // Numbered entities and stray ampersands, a line in a script written
        // without spaces, cut into a word a character with its punctuation
        // leaning on the word before, and a timing that is not one.
        let odd = "1\n00:00:01,000 --> 00:00:02,000\n&#x41;&#66; &bogus; R&D\n\n\
                   2\n00:00:02,000 --> 00:00:03,000\n今日は、晴れ。\n";
        let read = super::super::read_file("Odd.srt", odd).unwrap();
        assert_eq!(read[0].text(), "AB &bogus; R&D");
        let words: Vec<&str> = read[1].words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(words, ["今", "日", "は、", "晴", "れ。"]);
        assert_eq!(read[1].text(), "今日は、晴れ。");
        let broken =
            super::super::read_file("Bad.srt", "1\n00:00:xx,000 --> 00:00:02,000\nWords\n");
        assert_eq!(
            broken,
            Err(super::super::CaptionsError::Unreadable { line: 2 })
        );

        // The same file written as UTF-16 with its mark, and an older one in
        // Windows-1252, read as the words they hold.
        let plain = "1\n00:00:01,000 --> 00:00:02,000\nCafé “open”\n";
        let mut wide = vec![0xFF, 0xFE];
        wide.extend(plain.encode_utf16().flat_map(u16::to_le_bytes));
        let read = super::super::read_bytes("Wide.srt", &wide).unwrap();
        assert_eq!(read[0].text(), "Café “open”");
        let mut old = b"1\n00:00:01,000 --> 00:00:02,000\nCaf".to_vec();
        old.extend([0xE9, b' ', 0x93, b'o', b'p', b'e', b'n', 0x94, b'\n']);
        let read = super::super::read_bytes("Old.srt", &old).unwrap();
        assert_eq!(read[0].text(), "Café “open”");

        let lrc = "[ar:Someone]\n[00:12.00]First <00:12.50>line\n[00:15.25]Second\n";
        let read = super::super::read_file("Song.lrc", lrc).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].end, read[1].start);
        assert_eq!(
            read[0].words[1].start,
            crate::time::Rational::new(25, 2).unwrap()
        );
    }
}
