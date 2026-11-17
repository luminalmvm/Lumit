//! The edits a Transcript panel makes, as Cut edits: taking stretches out,
//! copying and inserting them, putting removed words back and laying a
//! source's words down.

use std::collections::BTreeMap;

use super::*;

/// The fade a sound clip's new edge gets so a cut through a word's tail does
/// not click: two milliseconds, too short to hear as a fade.
#[frb(ignore)]
fn anti_click() -> Rational {
    Rational::new(1, 500).unwrap_or(Rational::ZERO)
}

impl Edit<'_> {
    /// Make Sequence layers of the Footage layers an edit is about to cut:
    /// any heard one whose item has a transcript and whose bar meets one of
    /// `spans`, or that runs across one of `points`. Each keeps its id and its
    /// place in the stack, and is a row of this edit from here on.
    pub(super) fn convert_heard(
        &mut self,
        spans: &[(Rational, Rational)],
        points: &[Rational],
    ) -> Cut {
        let doc = self.of.document()?;
        let inside = |a: Rational, b: Rational| spans.iter().any(|(s, e)| a < *e && *s < b);
        let mut converting: Vec<Uuid> = Vec::new();
        for layer in &self.comp.layers {
            let LayerKind::Footage { item } = &layer.kind else {
                continue;
            };
            if layer.retime.is_some() || layer.switches.locked || !layer.switches.audible {
                continue;
            }
            let says =
                doc.transcripts.contains_key(item) && inside(layer.in_point.0, layer.out_point.0);
            let across = points
                .iter()
                .any(|p| layer.in_point.0 < *p && *p < layer.out_point.0);
            if says || across {
                converting.push(layer.id);
            }
        }
        for id in converting {
            let Some(index) = self.comp.layers.iter().position(|l| l.id == id) else {
                continue;
            };
            let converted = crate::api::layer::sequenced(&self.comp.layers[index])?;
            let LayerKind::Sequence { clips } = &converted.kind else {
                continue;
            };
            let offset = converted.start_offset.0;
            let mut clips = clips.clone();
            for c in &mut clips {
                c.place_start = c.place_start.checked_add(offset)?;
            }
            // Rows are in stack order, as `Edit::open` reads them.
            let above = self.comp.layers[..index]
                .iter()
                .filter(|l| matches!(l.kind, LayerKind::Sequence { .. }))
                .count();
            self.rows.insert(
                above.min(self.rows.len()),
                Row {
                    id,
                    audio_only: converted.audio_only,
                    locked: false,
                    audible: converted.switches.audible,
                    offset,
                    clips,
                    fresh: None,
                    at: 0,
                },
            );
            let comp = self.comp.id;
            self.first.push(Op::RemoveLayer { comp, layer: id });
            self.first.push(Op::AddLayer {
                comp,
                index,
                layer: Box::new(converted.clone()),
            });
            self.comp.layers[index] = converted;
        }
        Ok(())
    }

    /// Take `spans` (in order, apart) out of every unlocked row, the latest
    /// first so an earlier one is still where it was read. With `ripple` each
    /// closes up. Each sound clip edge it makes gets the anti-click fade
    /// where it has no fade. `Nothing` when a lift finds no clip.
    pub(super) fn take_spans(&mut self, spans: &[(Rational, Rational)], ripple: bool) -> Cut {
        let rows: Vec<usize> = (0..self.rows.len())
            .filter(|r| !self.rows[*r].locked)
            .collect();
        let mut edges: Vec<Rational> = Vec::new();
        let mut gone = 0;
        for (start, end) in spans.iter().rev() {
            gone += self.take_span(*start, *end, &rows, ripple)?;
            if ripple {
                let by = end.checked_sub(*start)?;
                for edge in &mut edges {
                    *edge = edge.checked_sub(by)?;
                }
                edges.push(*start);
            } else {
                edges.extend([*start, *end]);
            }
        }
        if !ripple && gone == 0 {
            return Err(BridgeCutResult::Nothing.into());
        }
        // Only on sound rows: on a picture row a fade fades the picture.
        let click = anti_click();
        for r in rows {
            let row = &mut self.rows[r];
            if !row.audio_only {
                continue;
            }
            for clip in &mut row.clips {
                if clip.fade_in.seconds.is_zero() && edges.contains(&clip.place_start) {
                    clip.fade_in.seconds = click;
                }
                if clip.fade_out.seconds.is_zero() && edges.contains(&clip.place_end()) {
                    clip.fade_out.seconds = click;
                }
            }
        }
        Ok(())
    }

    /// What every unlocked row holds under `spans`, as copied clips laid one
    /// after another from nought: a span's pieces keep their places against
    /// its start, and the next span starts where it ends. A locked row is left
    /// as it is, as a ripple leaves it, so its pieces neither move nor land.
    /// `Nothing` when no row holds anything there.
    pub(super) fn held_spans(&self, spans: &[(Rational, Rational)]) -> Cut<Vec<Held>> {
        let mut held: Vec<Held> = Vec::new();
        let mut at = Rational::ZERO;
        for (start, end) in spans {
            for (r, row) in self.rows.iter().enumerate().filter(|(_, row)| !row.locked) {
                let lane = self.rows[..r]
                    .iter()
                    .filter(|o| o.audio_only == row.audio_only)
                    .count();
                for clip in row.clips.iter().filter(|c| covers(c, *start, *end)) {
                    let (a, b) = (clip.place_start.max(*start), clip.place_end().min(*end));
                    let mut piece = trimmed(clip, a, b).ok_or(BridgeCutResult::Uncuttable)?;
                    // A cut edge is not the clip's own end, so it has no fade.
                    if a > clip.place_start {
                        piece.fade_in = Default::default();
                    }
                    if b < clip.place_end() {
                        piece.fade_out = Default::default();
                    }
                    piece.place_start = piece.place_start.checked_sub(*start)?.checked_add(at)?;
                    held.push(Held {
                        clip: piece,
                        sound: row.audio_only,
                        lane,
                    });
                }
            }
            at = at.checked_add(end.checked_sub(*start)?)?;
        }
        if held.is_empty() {
            return Err(BridgeCutResult::Nothing.into());
        }
        Ok(held)
    }

    /// Insert `held` at `at`: every unlocked row cut there and opened by its
    /// length, and each piece on the row it came from, counted among rows of
    /// its kind, as a paste with a target lands them.
    pub(super) fn insert_held(mut self, held: Vec<Held>, at: Rational, end_frame: &mut i64) -> Cut {
        let length = held.iter().map(|h| h.clip.place_end()).max();
        let lead = held.iter().map(|h| (h.sound, h.lane)).min();
        let (Some(length), Some((sound, lane))) = (length, lead) else {
            return Err(BridgeCutResult::Nothing.into());
        };
        let open: Vec<usize> = (0..self.rows.len())
            .filter(|r| !self.rows[*r].locked)
            .collect();
        self.razor(&open, at, false)?;
        self.ripple(at, length)?;
        let of_kind: Vec<usize> = (0..self.rows.len())
            .filter(|r| self.rows[*r].audio_only == sound)
            .collect();
        let target = of_kind
            .get(lane)
            .map(|r| LayerReference::new(self.of.project_id(), self.comp.id, self.rows[*r].id));
        self.paste_held(held, at, target, end_frame)
    }

    /// A drag of words to `to_frame`, as
    /// [`CompositionReference::cut_transcript_move`] describes it.
    pub(super) fn move_words(
        mut self,
        spans: &[BridgeFrameSpan],
        to_frame: i64,
        copy: bool,
        end_frame: &mut i64,
    ) -> Cut {
        let spans = times_of(self.comp.frame_rate, spans)?;
        let to = self.time(to_frame.max(0))?;
        // Dropped on the edge of the only span, a move would land where it
        // was.
        let still = !copy && matches!(spans.as_slice(), [(a, b)] if *a <= to && to <= *b);
        if still || spans.is_empty() || spans.iter().any(|(a, b)| *a < to && to < *b) {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.convert_heard(&spans, &[to])?;
        let held = self.held_spans(&spans)?;
        let mut at = to;
        if !copy {
            self.take_spans(&spans, true)?;
            // What left before the drop point moved it earlier by as much.
            for (a, b) in &spans {
                if *b <= to {
                    at = at.checked_sub(b.checked_sub(*a)?)?;
                }
            }
        }
        self.insert_held(held, at, end_frame)
    }

    /// Put back each run of removed words, `(clip, item, from, to)`, as
    /// [`CompositionReference::cut_transcript_restore`] describes it: the
    /// latest edit point first, so an earlier one is still where it was read,
    /// and at one edit point the latest words first, so the earlier ones
    /// still find the stretch they came out of. A run with nothing to put
    /// back is passed over.
    pub(super) fn restore(mut self, runs: &[(Uuid, Uuid, usize, usize)]) -> Cut {
        let doc = self.of.document()?;
        let mut order = Vec::with_capacity(runs.len());
        for run in runs {
            let (r, c) = self.find(run.0)?;
            let clip = &self.rows[r].clips[c];
            // How far past the clip's end its first word was said.
            let words = doc.transcripts.get(&run.1).map(|t| t.words.as_slice());
            let past = match words.and_then(|w| w.get(run.2)) {
                Some(word) => word.start.0.checked_sub(clip.source_out)?,
                None => Rational::ZERO,
            };
            order.push((clip.place_end(), past, *run));
        }
        order.sort_by_key(|(end, past, _)| std::cmp::Reverse((*end, *past)));
        let mut restored = false;
        for (_, _, (clip, item, from, to)) in order {
            match self.restore_run(clip, item, from, to) {
                Ok(()) => restored = true,
                Err(Stop::Refused(BridgeCutResult::Nothing)) => {}
                Err(stop) => return Err(stop),
            }
        }
        if !restored {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.commit()
    }

    /// Put the removed words `from..=to` of `item` back after `clip`.
    fn restore_run(&mut self, clip: Uuid, item: Uuid, from: usize, to: usize) -> Cut {
        let (r, c) = self.find(clip)?;
        let a = self.rows[r].clips[c].clone();
        let edit = a.place_end();
        let b = self.rows[r]
            .clips
            .iter()
            .filter(|o| o.id != clip && o.place_start >= edit)
            .min_by_key(|o| o.place_start)
            .cloned();
        let plain = |c: &Clip| c.retime.is_none() && c.source == ClipSource::Footage(item);
        let Some(b) = b.filter(|b| plain(&a) && plain(b) && b.source_in > a.source_out) else {
            return Err(BridgeCutResult::Nothing.into());
        };
        let doc = self.of.document()?;
        let words = &doc
            .transcripts
            .get(&item)
            .ok_or(BridgeCutResult::Nothing)?
            .words;
        let (Some(first), Some(last)) = (words.get(from), words.get(to)) else {
            return Err(BridgeCutResult::Nothing.into());
        };
        if to < from {
            return Err(BridgeCutResult::Nothing.into());
        }
        // Halfway into the silence either side, kept inside what was taken
        // out, and whole frames from the edit point so what follows stays on
        // frames.
        let (lo, hi) = (a.source_out, b.source_in);
        let before = from.checked_sub(1).and_then(|p| words.get(p));
        let start = before.map_or(first.start.0, |p| spoken::middle(p.end.0, first.start.0));
        let end = words
            .get(to + 1)
            .map_or(last.end.0, |n| spoken::middle(last.end.0, n.start.0));
        let rate = self.comp.frame_rate;
        let taken = hi.checked_sub(lo)?;
        let floor = rate.frame_at(CompTime(start.max(lo).checked_sub(lo)?));
        let ceiling = self
            .frame_up(end.min(hi).checked_sub(lo)?)?
            .min(rate.frame_at(CompTime(taken)));
        let from_out = self.time(floor)?;
        let to_out = self.time(ceiling)?;
        let (start, end) = (lo.checked_add(from_out)?, lo.checked_add(to_out)?);
        let length = end.checked_sub(start)?;
        if length <= Rational::ZERO {
            return Err(BridgeCutResult::Nothing.into());
        }
        // In the gap a lift left, the words go back where they were, so
        // they stay in time with the rows the lift kept. Anywhere else they go
        // in at the edit point.
        let lifted = b.place_start.checked_sub(edit)? == taken;
        let place = if lifted {
            edit.checked_add(start.checked_sub(lo)?)?
        } else {
            edit
        };
        // The clip, and every other clip the same cut split here: one that
        // ends at the edit point, with the same file carrying on after it as
        // much further in. Its picture, a second microphone and the music
        // under it all get their stretch back, so the rows stay in step.
        let mut pieces: Vec<(usize, Clip)> = Vec::new();
        let mut links: BTreeMap<Uuid, Uuid> = BTreeMap::new();
        for (mr, row) in self.rows.iter().enumerate() {
            for mate in &row.clips {
                if mate.place_end() != edit || mate.retime.is_some() {
                    continue;
                }
                // A linked mate on the same stretch of the same file goes
                // back whatever follows it.
                let linked = a.link.is_some()
                    && mate.link == a.link
                    && mate.source == a.source
                    && mate.source_out == a.source_out;
                if mate.id == clip || linked {
                    if row.locked {
                        return Err(BridgeCutResult::Locked.into());
                    }
                } else {
                    let next = row
                        .clips
                        .iter()
                        .filter(|o| o.id != mate.id && o.place_start >= edit)
                        .min_by_key(|o| o.place_start);
                    let split = matches!(mate.source, ClipSource::Footage(_))
                        && next.is_some_and(|n| {
                            n.source == mate.source
                                && n.retime.is_none()
                                && n.place_start == b.place_start
                                && n.source_in.checked_sub(mate.source_out).ok() == Some(taken)
                        });
                    if !split || row.locked {
                        continue;
                    }
                }
                let mut piece = mate.duplicate();
                piece.source_in = mate.source_out.checked_add(from_out)?;
                piece.source_out = mate.source_out.checked_add(to_out)?;
                piece.place_start = place;
                piece.place_duration = length;
                piece.fade_in = Default::default();
                piece.fade_out = Default::default();
                piece.transition = None;
                piece.link = mate
                    .link
                    .map(|old| *links.entry(old).or_insert_with(Uuid::now_v7));
                pieces.push((mr, piece));
            }
        }
        // With no room there, everything after makes room.
        let until = place.checked_add(length)?;
        let room = pieces.iter().all(|(row, _)| {
            !self.rows[*row]
                .clips
                .iter()
                .any(|o| covers(o, place, until))
        });
        if !room {
            let open: Vec<usize> = (0..self.rows.len())
                .filter(|r| !self.rows[*r].locked)
                .collect();
            self.razor(&open, place, false)?;
            self.ripple(place, length)?;
        }
        self.land(pieces, false)
    }

    /// `length` as whole frames of this composition, down to the frame and at
    /// least one, so what follows it starts on a frame.
    pub(super) fn whole_frames(&self, length: Rational) -> Cut<Rational> {
        self.time(self.comp.frame_rate.frame_at(CompTime(length)).max(1))
    }

    /// Lay `spans` of `footage` down one after another from `at`, each as
    /// [`Self::place`] places one, and answer where the last ends. A picture
    /// row made for the first takes the rest.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_spans(
        &mut self,
        target: Option<Uuid>,
        footage: &FootageReference,
        spans: &[BridgeSourceSpan],
        at: Rational,
        insert: bool,
        linked: bool,
        with_picture: bool,
        arrival: BridgeStreamsArrival,
    ) -> Cut<Rational> {
        if spans.is_empty() {
            return Err(BridgeCutResult::Nothing.into());
        }
        let (mut target, mut at) = (target, at);
        for span in spans {
            let start = Rational::new(span.start.num, span.start.den)?;
            let length = Rational::new(span.end.num, span.end.den)?.checked_sub(start)?;
            if length <= Rational::ZERO {
                return Err(BridgeError::InvalidTime.into());
            }
            // Whole frames, so each piece starts on one.
            let length = self.whole_frames(length)?;
            let end = start.checked_add(length)?;
            let until = at.checked_add(length)?;
            let made = self.rows.len();
            self.place_only(
                target,
                footage,
                span.start,
                BridgeRational {
                    num: end.num(),
                    den: end.den(),
                },
                0,
                insert,
                linked,
                with_picture,
                arrival,
                Some((at, until)),
            )?;
            if target.is_none() {
                let fresh = self.rows.get(made..).unwrap_or_default();
                target = fresh.iter().find(|r| !r.audio_only).map(|r| r.id);
            }
            at = until;
        }
        Ok(at)
    }
}
