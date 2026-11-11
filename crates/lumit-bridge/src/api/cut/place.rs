//! Putting things down in the Cut workspace: a stretch of footage, with its
//! sound on rows of its own, and a whole composition as one clip.

use super::*;

impl Edit<'_> {
    /// What a new row is called: Picture or Sound, and the first number no
    /// layer in the composition, or made by this edit, already has with it.
    pub(super) fn row_name(&self, audio_only: bool) -> String {
        let kind = if audio_only { "Sound" } else { "Picture" };
        let made = self.rows.iter().filter_map(|r| r.fresh.as_ref());
        let names: Vec<&str> = self
            .comp
            .layers
            .iter()
            .chain(made)
            .map(|l| l.name.as_str())
            .collect();
        let free = (1..).find(|n| !names.contains(&format!("{kind} {n}").as_str()));
        format!("{kind} {}", free.unwrap_or(1))
    }

    /// `target` is the row's layer. `span`, when given, is where on the row
    /// the stretch goes in place of `at_frame` and its own length, played at
    /// whatever speed fills it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place(
        mut self,
        target: Option<Uuid>,
        footage: &FootageReference,
        source_in: BridgeRational,
        source_out: BridgeRational,
        at_frame: i64,
        insert: bool,
        linked: bool,
        with_picture: bool,
        arrival: BridgeStreamsArrival,
        span: Option<(Rational, Rational)>,
    ) -> Cut {
        self.place_only(
            target,
            footage,
            source_in,
            source_out,
            at_frame,
            insert,
            linked,
            with_picture,
            arrival,
            span,
        )?;
        self.commit()
    }

    /// [`Self::place`] without the commit, so several go in one edit.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_only(
        &mut self,
        target: Option<Uuid>,
        footage: &FootageReference,
        source_in: BridgeRational,
        source_out: BridgeRational,
        at_frame: i64,
        insert: bool,
        linked: bool,
        with_picture: bool,
        arrival: BridgeStreamsArrival,
        span: Option<(Rational, Rational)>,
    ) -> Cut {
        let source_in = Rational::new(source_in.num, source_in.den)?;
        let source_out = Rational::new(source_out.num, source_out.den)?;
        let length = source_out.checked_sub(source_in)?;
        if source_in.is_negative() || length <= Rational::ZERO {
            return Err(BridgeError::InvalidTime.into());
        }
        let (at, end) = match span {
            Some(span) => span,
            None => {
                let at = self.time(at_frame.max(0))?;
                (at, at.checked_add(length)?)
            }
        };
        let fills = end.checked_sub(at)?;
        if fills <= Rational::ZERO {
            return Err(BridgeError::InvalidTime.into());
        }
        let item = footage.id();
        let (name, picture, sound, streams, (_, width, height)) = {
            let proj = self.of.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = state.store.snapshot();
            let Some(ProjectItem::Footage(f)) = doc.item(item) else {
                return Err(BridgeError::InvalidItem.into());
            };
            (
                f.name.clone(),
                CompositionReference::has_picture(&state, f),
                has_sound(&state, f),
                crate::streams::of_item(&state, f),
                CompositionReference::footage_span_and_size(&state, f, &self.comp),
            )
        };
        // What each clip that sounds plays: the first stream as ever, every
        // stream mixed, or a clip to a stream.
        let sounds = arrival.sounds(streams.len());
        let target = match target {
            Some(layer) => Some(self.row_of(layer).ok_or(BridgeError::NotSequence)?),
            None => None,
        };

        // Dropped on a sound row, a file gives its sound and nothing else.
        let onto_sound = target.is_some_and(|r| self.rows[r].audio_only);
        // A file with no picture dropped on a picture row is sound on that
        // row, which is made heard for it.
        let onto_picture = !picture && target.is_some_and(|r| !self.rows[r].audio_only);
        if let (true, Some(r)) = (onto_picture, target) {
            self.hear(r);
        }
        let picture_row = match target {
            _ if !picture || onto_sound => None,
            Some(r) => Some(r),
            None => Some(self.add_row(None, false, width, height)),
        };
        // With `with_picture` the picture clip plays the first of them itself,
        // and with `linked` the rest go down as sound clips beside it.
        let heard = picture_row.is_some_and(|r| self.rows[r].audible);
        let picture_sounds = sound && heard && with_picture;
        // Otherwise the sound goes down as a clip of its own, linked to the
        // picture, and the picture clip is silent. A picture row made silent
        // before rows were all heard is left as it is, its clips' sound with
        // it.
        let silenced = sound && heard && !picture_sounds;
        let wanted = sound
            && match picture_row {
                Some(_) => linked && !picture_sounds,
                None => true,
            };
        let on_sound_rows: &[AudioStreams] = match (wanted, picture_sounds && linked) {
            (true, _) => &sounds,
            (false, true) => sounds.get(1..).unwrap_or_default(),
            (false, false) => &[],
        };
        // A row there already for each, where one will do, and `None` where
        // one has to be made.
        let mut landings: Vec<Option<usize>> = Vec::with_capacity(on_sound_rows.len());
        for n in 0..on_sound_rows.len() {
            // The sound of the file itself goes where it always has. A stream
            // split off beside it goes on a row nothing of this file is on.
            let whole = n == 0 && wanted;
            if whole && (onto_sound || onto_picture) {
                landings.push(target);
                continue;
            }
            // The sound row this picture row already keeps its sound on,
            // else the first one with room, else a new one at the bottom.
            let links: Vec<Uuid> = picture_row
                .into_iter()
                .flat_map(|r| self.rows[r].clips.iter().filter_map(|c| c.link))
                .collect();
            let taken: Vec<Uuid> = landings
                .iter()
                .flatten()
                .map(|r| self.rows[*r].id)
                .collect();
            let open = |row: &&Row| row.audio_only && !row.locked && !taken.contains(&row.id);
            let paired = |row: &&Row| {
                whole
                    && row
                        .clips
                        .iter()
                        .any(|c| c.link.is_some_and(|l| links.contains(&l)))
            };
            // An insert opens its own room, so only a clip the point
            // falls inside is in the way.
            let free = |row: &&Row| {
                !row.clips.iter().any(|c| {
                    if insert {
                        c.place_start < at && at < c.place_end()
                    } else {
                        covers(c, at, end)
                    }
                })
            };
            let found = self.rows.iter().filter(open).find(paired);
            let found = found.or_else(|| self.rows.iter().filter(open).find(free));
            landings.push(match found.map(|row| row.id) {
                Some(id) => Some(self.row_of(id).ok_or(BridgeError::InvalidLayer)?),
                None => None,
            });
        }
        // The rows to make, the last stream's first: each new sound row goes
        // in above the one made before it, so the streams read down the
        // stack in the file's own order.
        for (n, plays) in on_sound_rows.iter().enumerate().rev() {
            if landings[n].is_some() {
                continue;
            }
            let (w, h) = (f64::from(self.comp.width), f64::from(self.comp.height));
            let name = match plays.places() {
                [place] if n > 0 || !wanted => {
                    Some(crate::streams::split_name(&name, *place, &streams))
                }
                _ => None,
            };
            landings[n] = Some(self.add_row(name, true, w, h));
        }
        let sound_rows: Vec<usize> = landings.into_iter().flatten().collect();

        let rows: Vec<usize> = picture_row
            .into_iter()
            .chain(sound_rows.iter().copied())
            .collect();
        if rows.is_empty() {
            return Err(BridgeCutResult::WrongLayer.into());
        }
        if rows.iter().any(|r| self.rows[*r].locked) {
            return Err(BridgeCutResult::Locked.into());
        }
        if insert {
            // A clip the point falls inside is cut there, so its later half
            // moves along with everything after it.
            self.razor(&rows, at, linked)?;
            self.ripple(at, fills)?;
        }
        let link = (rows.len() > 1).then(Uuid::now_v7);
        // The picture clip first where there is one, then a clip to each
        // sound row, in the order the streams were given their rows.
        let picture_plays = picture_row.map(|_| match picture_sounds {
            true => sounds.first().cloned().unwrap_or_default(),
            false => AudioStreams::default(),
        });
        let plays = picture_plays
            .into_iter()
            .chain(on_sound_rows.iter().cloned());
        for (n, (r, plays)) in rows.into_iter().zip(plays).enumerate() {
            let mut clip = Clip::new(ClipSource::Footage(item), source_in, source_out, at, length);
            if fills != length {
                clip = clip.stretched(at, end).ok_or(BridgeCutResult::Limit)?;
            }
            clip.link = link;
            clip.audio_streams = plays;
            clip.audible = !(n == 0 && silenced && picture_row.is_some());
            let dropped = clip.id;
            let row = &mut self.rows[r];
            row.clips.push(clip);
            row.clips = placed(std::mem::take(&mut row.clips), dropped, false);
        }
        Ok(())
    }

    /// Put the whole of a composition down at `at_frame` as one clip, its
    /// sound inside it, on `target` or a new picture Sequence layer at the top.
    pub(super) fn place_comp(
        mut self,
        target: Option<LayerReference>,
        comp: Uuid,
        at_frame: i64,
    ) -> Cut {
        // A composition cannot play itself.
        if comp == self.comp.id {
            return Err(BridgeCutResult::WrongLayer.into());
        }
        let (length, width, height, multicam) = {
            let doc = self.of.document()?;
            let inner = doc.comp(comp).ok_or(BridgeError::InvalidComp)?;
            (
                inner.duration.0,
                f64::from(inner.width),
                f64::from(inner.height),
                inner.is_multicam(),
            )
        };
        // A multicam comp goes down as its picture and its sound, linked.
        if multicam {
            let target = target.map(|layer| layer.id());
            return self.place_multicam(
                target,
                comp,
                Rational::ZERO,
                length,
                at_frame,
                false,
                false,
                None,
                None,
            );
        }
        let at = self.time(at_frame.max(0))?;
        let r = match target {
            Some(layer) => self.row_of(layer.id()).ok_or(BridgeError::NotSequence)?,
            None => self.add_row(None, false, width, height),
        };
        if self.rows[r].audio_only {
            return Err(BridgeCutResult::WrongLayer.into());
        }
        if self.rows[r].locked {
            return Err(BridgeCutResult::Locked.into());
        }
        let clip = Clip::new(ClipSource::Comp(comp), Rational::ZERO, length, at, length);
        let dropped = clip.id;
        let row = &mut self.rows[r];
        row.clips.push(clip);
        row.clips = placed(std::mem::take(&mut row.clips), dropped, false);
        self.commit()
    }

    /// Put one clip of composition `comp` down for each of `copies`, one after
    /// another from `at_frame`, on a new picture Sequence layer at the top.
    /// Each holds the values it is given and runs for the seconds it is
    /// given, or as long as the composition. What placing a graphics
    /// template's data rows does in Cut.
    pub(super) fn place_copies(
        mut self,
        comp: Uuid,
        at_frame: i64,
        copies: Vec<(Option<lumit_core::model::EffectInstance>, Option<f64>)>,
    ) -> Cut {
        if comp == self.comp.id || copies.is_empty() {
            return Err(BridgeCutResult::WrongLayer.into());
        }
        let (length, width, height) = {
            let doc = self.of.document()?;
            let inner = doc.comp(comp).ok_or(BridgeError::InvalidComp)?;
            (
                inner.duration.0,
                f64::from(inner.width),
                f64::from(inner.height),
            )
        };
        let mut at = self.time(at_frame.max(0))?;
        let r = self.add_row(None, false, width, height);
        let fps = self.comp.frame_rate.fps();
        for (inputs, seconds) in copies {
            let long = match seconds {
                Some(seconds) => self.time((seconds * fps).round().max(1.0) as i64)?,
                None => length,
            };
            let mut clip = Clip::new(ClipSource::Comp(comp), Rational::ZERO, long, at, long);
            clip.inputs = inputs;
            self.rows[r].clips.push(clip);
            at = at.checked_add(long)?;
        }
        self.commit()
    }

    /// **Replace**: the source from `source_in` over `clip`, which keeps its
    /// row, its place and its length. `Limit` when the file runs out first.
    pub(super) fn replace(
        self,
        clip: Uuid,
        footage: &FootageReference,
        source_in: BridgeRational,
        linked: bool,
        with_picture: bool,
        arrival: BridgeStreamsArrival,
    ) -> Cut {
        let (r, c) = self.find(clip)?;
        let old = &self.rows[r].clips[c];
        let (row, at, length) = (self.rows[r].id, old.place_start, old.place_duration);
        let out = Rational::new(source_in.num, source_in.den)?.checked_add(length)?;
        if self.media_length(footage)?.is_some_and(|media| out > media) {
            return Err(BridgeCutResult::Limit.into());
        }
        let source_out = BridgeRational {
            num: out.num(),
            den: out.den(),
        };
        let span = Some((at, at.checked_add(length)?));
        let (insert, at_frame) = (false, 0);
        self.place(
            Some(row),
            footage,
            source_in,
            source_out,
            at_frame,
            insert,
            linked,
            with_picture,
            arrival,
            span,
        )
    }

    /// **Fit to fill**: the source from `source_in` to `source_out` over the
    /// work area, at whatever speed fills it, overwriting what is there.
    /// `Nothing` with no work area.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fit_to_fill(
        self,
        target: Option<Uuid>,
        footage: &FootageReference,
        source_in: BridgeRational,
        source_out: BridgeRational,
        linked: bool,
        with_picture: bool,
        arrival: BridgeStreamsArrival,
    ) -> Cut {
        let Some((start, end)) = self.comp.work_area else {
            return Err(BridgeCutResult::Nothing.into());
        };
        let (insert, at_frame) = (false, 0);
        self.place(
            target,
            footage,
            source_in,
            source_out,
            at_frame,
            insert,
            linked,
            with_picture,
            arrival,
            Some((start.0, end.0)),
        )
    }

    /// How long `footage` runs, or `None` for a still or a file that has not
    /// been read, which run as long as they are asked.
    fn media_length(&self, footage: &FootageReference) -> Cut<Option<Rational>> {
        let proj = self.of.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = state.store.snapshot();
        let item = footage.id();
        if let Some(ProjectItem::Footage(f)) = doc.item(item) {
            if is_still(&state, f) {
                return Ok(None);
            }
        }
        Ok(clip_source_duration(
            &state,
            &doc,
            ClipSource::Footage(item),
        ))
    }
}

/// [`Edit::replace`] as one edit on `of`.
#[frb(ignore)]
pub(crate) fn replace_clip(
    of: &CompositionReference,
    clip: Uuid,
    footage: &FootageReference,
    source_in: BridgeRational,
    linked: bool,
    with_picture: bool,
    arrival: BridgeStreamsArrival,
) -> Result<BridgeCutResult, BridgeError> {
    cut(of, |edit| {
        edit.replace(clip, footage, source_in, linked, with_picture, arrival)
    })
}

/// [`Edit::fit_to_fill`] as one edit on `of`.
#[allow(clippy::too_many_arguments)]
#[frb(ignore)]
pub(crate) fn fit_clip(
    of: &CompositionReference,
    target: Option<Uuid>,
    footage: &FootageReference,
    source_in: BridgeRational,
    source_out: BridgeRational,
    linked: bool,
    with_picture: bool,
    arrival: BridgeStreamsArrival,
) -> Result<BridgeCutResult, BridgeError> {
    cut(of, |edit| {
        edit.fit_to_fill(
            target,
            footage,
            source_in,
            source_out,
            linked,
            with_picture,
            arrival,
        )
    })
}
