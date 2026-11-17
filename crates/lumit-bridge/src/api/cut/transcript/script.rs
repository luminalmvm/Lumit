//! Putting a cut together from a script: each line found in the footage's
//! words and laid down in the script's order, its other takes kept on the
//! clip.

use lumit_core::sequence::spoken::script as lines;
use lumit_core::sequence::takes::Take;

use super::*;

/// How much a line keeps either side of its words, so the first sound and
/// the last breath are not clipped.
#[frb(ignore)]
fn handle() -> Rational {
    Rational::new(3, 20).unwrap_or(Rational::ZERO)
}

/// The biggest script read: far beyond any film's.
const SCRIPT_BYTES: u64 = 16 * 1024 * 1024;

/// A run of words as the stretch of source a clip of it plays: its words
/// with the handle either side, kept inside the source.
#[frb(ignore)]
fn stretch(
    doc: &Document,
    found: &lines::Match,
    media: Option<Rational>,
) -> Option<(Rational, Rational)> {
    let words = &doc.transcripts.get(&found.item)?.words;
    let first = words.get(found.from)?;
    let last = words.get(found.to.checked_sub(1)?)?;
    let start = first
        .start
        .0
        .checked_sub(handle())
        .ok()?
        .max(Rational::ZERO);
    let end = last.end.0.checked_add(handle()).ok()?;
    let end = media.map_or(end, |m| end.min(m));
    (start < end).then_some((start, end))
}

/// The text of a Final Draft file's dialogue, a paragraph to a line. A note
/// inside a paragraph is not dialogue.
#[frb(ignore)]
fn final_draft_dialogue(xml: &str) -> Result<String, BridgeError> {
    let doc = roxmltree::Document::parse(xml).map_err(|_| BridgeError::ReadFailed)?;
    let mut out = Vec::new();
    for paragraph in doc.descendants().filter(|n| {
        n.has_tag_name("Paragraph")
            && n.attribute("Type") == Some("Dialogue")
            && !n.ancestors().any(|a| a.has_tag_name("ScriptNote"))
    }) {
        let line: String = paragraph
            .children()
            .filter(|n| n.has_tag_name("Text"))
            .filter_map(|n| n.text())
            .collect();
        if !line.trim().is_empty() {
            out.push(line.trim().to_owned());
        }
    }
    Ok(out.join("\n"))
}

/// A file's bytes as text: UTF-16 when it opens with that byte order mark,
/// UTF-8 otherwise.
#[frb(ignore)]
fn decoded(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| -> String {
        let units = rest
            .chunks_exact(2)
            .filter_map(|pair| <[u8; 2]>::try_from(pair).ok())
            .map(unit);
        char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    };
    match bytes {
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => {
            let text = String::from_utf8_lossy(bytes);
            text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
        }
    }
}

/// [`super::read_script_file`].
#[frb(ignore)]
pub(super) fn read(path: &str) -> Result<String, BridgeError> {
    if path.trim().is_empty() {
        return Err(BridgeError::EmptyPath);
    }
    let size = std::fs::metadata(path)
        .map_err(|_| BridgeError::ReadFailed)?
        .len();
    if size > SCRIPT_BYTES {
        return Err(BridgeError::InvalidParam);
    }
    let text = decoded(&std::fs::read(path).map_err(|_| BridgeError::ReadFailed)?);
    let fdx = std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("fdx"));
    if fdx {
        final_draft_dialogue(&text)
    } else {
        Ok(text)
    }
}

impl Edit<'_> {
    /// Lay down the best take of each line one after another from `at`, a
    /// picture clip and its linked sound as a place makes them, with the
    /// line's other takes on both. Answers how many lines went down and the
    /// lines nobody said.
    fn assemble(
        &mut self,
        doc: &Document,
        lines: &[String],
        found: &[Vec<lines::Match>],
        at: Rational,
        insert: bool,
    ) -> Cut<(u32, Vec<String>)> {
        let project = self.of.project_id();
        let (mut at, mut target, mut placed, mut missing) = (at, None, 0u32, Vec::new());
        for (line, matches) in lines.iter().zip(found) {
            let length_of = |item: Uuid| -> Cut<Option<Rational>> {
                let proj = self.of.project()?;
                let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
                Ok(clip_source_duration(&state, doc, ClipSource::Footage(item)))
            };
            let Some(best) = matches.first() else {
                missing.push(line.clone());
                continue;
            };
            let Some((start, end)) = stretch(doc, best, length_of(best.item)?) else {
                missing.push(line.clone());
                continue;
            };
            // The other takes, each from as far before its words as the
            // best one starts before its own.
            let mut takes = Vec::new();
            for other in matches.iter().skip(1) {
                if let Some((from, _)) = stretch(doc, other, length_of(other.item)?) {
                    takes.push(Take {
                        source: ClipSource::Footage(other.item),
                        offset: from.checked_sub(start)?,
                        extra: serde_json::Map::new(),
                    });
                }
            }
            // Whole frames, so each line starts on one.
            let length = self.whole_frames(end.checked_sub(start)?)?;
            let end = start.checked_add(length)?;
            let until = at.checked_add(length)?;
            let before: Vec<Uuid> = self
                .rows
                .iter()
                .flat_map(|r| &r.clips)
                .map(|c| c.id)
                .collect();
            let footage = FootageReference::new(project, best.item);
            let rational = |t: Rational| BridgeRational {
                num: t.num(),
                den: t.den(),
            };
            self.place_only(
                target,
                &footage,
                rational(start),
                rational(end),
                0,
                insert,
                true,
                false,
                BridgeStreamsArrival::First,
                Some((at, until)),
            )?;
            for row in &mut self.rows {
                for clip in row.clips.iter_mut().filter(|c| !before.contains(&c.id)) {
                    clip.takes.extend(takes.iter().cloned());
                }
            }
            if target.is_none() {
                target = self
                    .rows
                    .iter()
                    .find(|r| r.fresh.is_some() && !r.audio_only)
                    .map(|r| r.id);
            }
            placed += 1;
            at = until;
        }
        Ok((placed, missing))
    }
}

/// [`ProjectReference::find_script`].
#[frb(ignore)]
pub(super) fn find(
    project: &ProjectReference,
    script: &str,
    footage: &[FootageReference],
) -> Result<BridgeScriptFound, BridgeError> {
    let doc = project
        .state()?
        .read()
        .map_err(|_| BridgeError::ReadFailed)?
        .store
        .snapshot();
    let items: Vec<Uuid> = footage.iter().map(FootageReference::id).collect();
    let lines = lines::lines(script);
    let found = lines::find(&doc, &items, &lines);
    let settings = BridgeCompSettings::defaults();
    let mut out = BridgeScriptFound {
        lines: Vec::with_capacity(lines.len()),
        width: settings.width,
        height: settings.height,
        fps_num: settings.fps_num,
        fps_den: settings.fps_den,
    };
    #[cfg(feature = "media")]
    if let Some(first) = found.iter().find_map(|m| m.first()) {
        let probed = FootageReference::new(project.id, first.item).probe();
        if let Some(video) = probed.as_ref().and_then(|p| p.video.as_ref()) {
            if video.width > 0 && video.height > 0 {
                (out.width, out.height) = (video.width, video.height);
            }
            let rate = (u32::try_from(video.fps_num), u32::try_from(video.fps_den));
            if let (Ok(num @ 1..), Ok(den @ 1..)) = rate {
                (out.fps_num, out.fps_den) = (num, den);
            }
        }
    }
    for (text, matches) in lines.into_iter().zip(found) {
        let takes = matches
            .iter()
            .filter_map(|m| {
                Some(BridgeWordRun {
                    item: FootageReference::new(project.id, m.item),
                    from: u32::try_from(m.from).ok()?,
                    to: u32::try_from(m.to.checked_sub(1)?).ok()?,
                })
            })
            .collect();
        out.lines.push(BridgeScriptLine { text, takes });
    }
    Ok(out)
}

/// [`ProjectReference::assemble_from_script`].
#[frb(ignore)]
pub(super) fn assemble(
    project: &ProjectReference,
    found: BridgeScriptFound,
    into: Option<CompositionReference>,
    at_frame: i64,
    name: String,
) -> Result<BridgeAssembled, BridgeError> {
    let mut lines = Vec::with_capacity(found.lines.len());
    let mut matches = Vec::with_capacity(found.lines.len());
    for line in found.lines {
        lines.push(line.text);
        matches.push(
            line.takes
                .iter()
                .filter_map(|t| {
                    Some(lines::Match {
                        item: t.item.id(),
                        from: t.from as usize,
                        to: (t.to as usize).checked_add(1)?,
                        score: 0,
                    })
                })
                .collect::<Vec<_>>(),
        );
    }
    if matches.iter().all(Vec::is_empty) {
        return Ok(BridgeAssembled {
            result: BridgeCutResult::Nothing,
            placed: 0,
            missing: lines,
            comp: None,
        });
    }
    let state = project.state()?;
    let own = {
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        let own = state.store.begin_own_undo_group();
        if own {
            state.store.name_undo_group("Assemble from script");
        }
        own
    };
    let made = into.is_none();
    // Every exit from here reaches the end, so the group is closed.
    let out = (|| {
        let (comp, at_frame) = match into {
            Some(comp) => (comp, at_frame),
            None => {
                let mut settings = BridgeCompSettings::defaults();
                (settings.width, settings.height) = (found.width, found.height);
                (settings.fps_num, settings.fps_den) = (found.fps_num, found.fps_den);
                // One frame long: it grows to the cut as the clips go down.
                settings.duration = BridgeRational {
                    num: i64::from(settings.fps_den),
                    den: i64::from(settings.fps_num.max(1)),
                };
                (project.new_composition(name, Some(settings))?, 0)
            }
        };
        let doc = comp.document()?;
        let mut tally = (0, Vec::new());
        let result = cut(&comp, |mut edit| {
            let at = edit.time(at_frame.max(0))?;
            tally = edit.assemble(&doc, &lines, &matches, at, !made)?;
            edit.commit()
        })?;
        Ok::<_, BridgeError>((result, tally, comp))
    })();
    let done = matches!(out, Ok((BridgeCutResult::Done, ..)));
    if own {
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        if done {
            state.store.end_undo_group();
        } else {
            state
                .store
                .abandon_undo_group()
                .map_err(BridgeError::OpError)?;
        }
    }
    let (result, (placed, missing), comp) = out?;
    Ok(BridgeAssembled {
        result,
        placed,
        missing,
        comp: (made && done).then_some(comp),
    })
}
