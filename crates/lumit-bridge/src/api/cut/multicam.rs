//! Multicam clips in the Cut workspace (`lumit_core::multicam`): cutting to an
//! angle at the playhead, switching a clip's angle, stepping through the
//! angles, painting one over a stretch, putting a multicam comp down, and
//! flattening a clip into the footage its angle plays.
//!
//! A multicam clip's picture and its sound are two clips sharing a link, as
//! footage's are, so each edit here reaches the halves its switch names: the
//! picture, the sound, or both.

use super::*;
use lumit_core::model::Document;

/// Which halves of a multicam clip a switch reaches.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAngleSwitch {
    /// The picture and the sound together.
    Both,
    Picture,
    Sound,
}

/// One half of a multicam clip: where it is, and whether it shows the angle,
/// sounds it, or both (a picture clip that is heard).
#[frb(ignore)]
struct Half {
    r: usize,
    c: usize,
    shows: bool,
    sounds: bool,
    /// A sound clip added under the clip to hear another angle as well, which
    /// a switch leaves on its angle: every sounding half after the first.
    added: bool,
}

impl BridgeAngleSwitch {
    /// Whether a switch of this kind reaches `half`.
    #[frb(ignore)]
    fn reaches(self, half: &Half) -> bool {
        match self {
            Self::Both => !half.added,
            Self::Picture => half.shows,
            Self::Sound => half.sounds && !half.added,
        }
    }

    /// Whether a switch of this kind to an angle, a sound angle with `sound`,
    /// changes `half`. A half that shows a picture never takes a sound angle,
    /// which has no picture, even when it is heard as well.
    #[frb(ignore)]
    fn takes(self, half: &Half, sound: bool) -> bool {
        self.reaches(half) && !(sound && half.shows)
    }
}

/// Whether `clip` plays an angle of a multicam comp.
#[frb(ignore)]
fn plays_multicam(doc: &Document, clip: &Clip) -> bool {
    clip.angle.is_some()
        && matches!(clip.source, ClipSource::Comp(id) if doc.comp(id).is_some_and(Composition::is_multicam))
}

impl Edit<'_> {
    /// The multicam clip under `at`: on `row` when it has one, else on the top
    /// picture row with one, else on the top sound row with one.
    fn multicam_under(
        &self,
        doc: &Document,
        at: Rational,
        row: Option<Uuid>,
    ) -> Cut<(usize, usize)> {
        let here = |r: usize| {
            let clips = &self.rows[r].clips;
            clips
                .iter()
                .position(|c| c.place_start <= at && at < c.place_end() && plays_multicam(doc, c))
                .map(|c| (r, c))
        };
        if let Some(found) = row.and_then(|l| self.row_of(l)).and_then(here) {
            return Ok(found);
        }
        let pictures = (0..self.rows.len()).filter(|r| !self.rows[*r].audio_only);
        let sounds = (0..self.rows.len()).filter(|r| self.rows[*r].audio_only);
        pictures
            .chain(sounds)
            .find_map(here)
            .ok_or_else(|| BridgeCutResult::Nothing.into())
    }

    /// A multicam clip and the clips linked to it that play the same comp:
    /// the halves of one multicam clip.
    fn halves(&self, r: usize, c: usize) -> Vec<Half> {
        let clip = &self.rows[r].clips[c];
        let mut out = Vec::new();
        for (rr, row) in self.rows.iter().enumerate() {
            for (cc, other) in row.clips.iter().enumerate() {
                let linked = clip.link.is_some() && other.link == clip.link;
                if (rr, cc) != (r, c) && !(linked && other.source == clip.source) {
                    continue;
                }
                out.push(Half {
                    r: rr,
                    c: cc,
                    shows: !row.audio_only,
                    sounds: row.audio_only || (other.audible && row.audible),
                    added: false,
                });
            }
        }
        // The top half that sounds plays the clip's sound; sound clips below
        // it were added to hear other angles as well.
        let mut heard = false;
        for half in &mut out {
            if half.sounds {
                half.added = heard && !half.shows;
                heard = true;
            }
        }
        out
    }

    /// The angle `angle` of the comp the clip at `(r, c)` plays, checked to be
    /// one of its angles: whether it is a sound angle, or `Nothing`.
    fn angle_of(&self, doc: &Document, r: usize, c: usize, angle: Uuid) -> Cut<bool> {
        let ClipSource::Comp(mc) = self.rows[r].clips[c].source else {
            return Err(BridgeCutResult::Nothing.into());
        };
        let comp = doc.comp(mc).ok_or(BridgeError::InvalidComp)?;
        let found = lumit_core::multicam::angles(comp).find(|l| l.id == angle);
        Ok(found.ok_or(BridgeCutResult::Nothing)?.audio_only)
    }

    /// Switch the halves of the multicam clip at `(r, c)` that `switch`
    /// reaches to `angle`: the whole clips, or with `at` inside them the
    /// pieces after a cut there. A picture half never takes a sound angle,
    /// which has no picture. With `linked` every half is cut, so the halves
    /// stay paired; without it only those that switch are.
    #[allow(clippy::too_many_arguments)]
    fn switch_halves(
        &mut self,
        doc: &Document,
        r: usize,
        c: usize,
        angle: Uuid,
        switch: BridgeAngleSwitch,
        at: Option<Rational>,
        linked: bool,
    ) -> Cut {
        let sound_angle = self.angle_of(doc, r, c, angle)?;
        let halves = self.halves(r, c);
        let takes = |h: &Half| switch.takes(h, sound_angle);
        let id = |h: &Half| self.rows[h.r].clips[h.c].id;
        let switching: Vec<Uuid> = halves.iter().filter(|h| takes(h)).map(id).collect();
        let changes = halves
            .iter()
            .any(|h| takes(h) && self.rows[h.r].clips[h.c].angle != Some(angle));
        if !changes {
            return Err(BridgeCutResult::Nothing.into());
        }
        let cutting: Vec<Uuid> = halves
            .iter()
            .filter(|h| linked || takes(h))
            .map(id)
            .collect();
        let rows: Vec<usize> = halves.iter().map(|h| h.r).collect();
        if rows.iter().any(|r| self.rows[*r].locked) {
            return Err(BridgeCutResult::Locked.into());
        }
        let at = at.filter(|at| {
            let clip = &self.rows[r].clips[c];
            clip.place_start < *at && *at < clip.place_end()
        });
        let Some(at) = at else {
            let whole = self.rows.iter_mut().flat_map(|row| &mut row.clips);
            for clip in whole.filter(|clip| switching.contains(&clip.id)) {
                clip.angle = Some(angle);
            }
            return Ok(());
        };
        for (was, later) in self.cut_picked(at, |_, clip| cutting.contains(&clip.id))? {
            if switching.contains(&was) {
                let (r, c) = self.find(later)?;
                self.rows[r].clips[c].angle = Some(angle);
            }
        }
        Ok(())
    }

    /// Cut each clip `at` falls inside that `picks` chooses, by its row and
    /// itself, as [`Self::razor`] does: the later pieces of a group make a
    /// group of their own. Answers each clip cut with its later piece.
    fn cut_picked(
        &mut self,
        at: Rational,
        picks: impl Fn(usize, &Clip) -> bool,
    ) -> Cut<Vec<(Uuid, Uuid)>> {
        let mut regrouped: Vec<(Uuid, Uuid)> = Vec::new();
        let mut cut = Vec::new();
        for (r, row) in self.rows.iter_mut().enumerate() {
            let mut next = Vec::with_capacity(row.clips.len() + 1);
            for clip in &row.clips {
                if !(clip.place_start < at && at < clip.place_end() && picks(r, clip)) {
                    next.push(clip.clone());
                    continue;
                }
                let (left, mut right) = clip.cut(at).ok_or(BridgeCutResult::Uncuttable)?;
                right.group = right.group.map(|g| {
                    let found = regrouped.iter().find(|(old, _)| *old == g);
                    found.map_or_else(Uuid::now_v7, |(_, new)| *new)
                });
                regrouped.extend(clip.group.zip(right.group));
                cut.push((clip.id, right.id));
                next.extend([left, right]);
            }
            row.clips = next;
        }
        Ok(cut)
    }

    /// Cut the multicam clip under `at_frame` there and switch the piece
    /// after the cut to `angle`, or switch the whole clip without `cut`.
    fn cut_to(
        mut self,
        at_frame: i64,
        angle: Uuid,
        switch: BridgeAngleSwitch,
        row: Option<Uuid>,
        cut: bool,
        linked: bool,
    ) -> Cut {
        let doc = self.of.document()?;
        let at = self.time(at_frame)?;
        let (r, c) = self.multicam_under(&doc, at, row)?;
        self.switch_halves(&doc, r, c, angle, switch, cut.then_some(at), linked)?;
        self.commit()
    }

    /// Switch each named multicam clip, and its other half as `switch` says,
    /// to `angle`, whole.
    fn switch_clips(mut self, clips: &[Uuid], angle: Uuid, switch: BridgeAngleSwitch) -> Cut {
        let doc = self.of.document()?;
        let mut switched = 0;
        for id in clips {
            let (r, c) = self.find(*id)?;
            if !plays_multicam(&doc, &self.rows[r].clips[c]) {
                continue;
            }
            match self.switch_halves(&doc, r, c, angle, switch, None, false) {
                Ok(()) => switched += 1,
                Err(Stop::Refused(BridgeCutResult::Nothing)) => {}
                Err(other) => return Err(other),
            }
        }
        if switched == 0 {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.commit()
    }

    /// Switch the multicam clip under `at_frame` to the angle before or after
    /// the one it shows (or sounds, for a switch of the sound alone), round
    /// the angles. A picture skips the sound angles.
    fn step(mut self, at_frame: i64, forward: bool, switch: BridgeAngleSwitch) -> Cut {
        let doc = self.of.document()?;
        let at = self.time(at_frame)?;
        let (r, c) = self.multicam_under(&doc, at, None)?;
        let clip = &self.rows[r].clips[c];
        let ClipSource::Comp(mc) = clip.source else {
            return Err(BridgeCutResult::Nothing.into());
        };
        // The angle the stepping counts from: the half the switch reaches.
        let halves = self.halves(r, c);
        let from = halves
            .iter()
            .find(|h| switch.reaches(h))
            .and_then(|h| self.rows[h.r].clips[h.c].angle);
        let comp = doc.comp(mc).ok_or(BridgeError::InvalidComp)?;
        let pictures = switch != BridgeAngleSwitch::Sound;
        let angles: Vec<Uuid> = lumit_core::multicam::offered(comp)
            .filter(|l| !pictures || !l.audio_only)
            .map(|l| l.id)
            .collect();
        if angles.len() < 2 {
            return Err(BridgeCutResult::Nothing.into());
        }
        let here = from.and_then(|a| angles.iter().position(|x| *x == a));
        let next = match (here, forward) {
            (Some(i), true) => (i + 1) % angles.len(),
            (Some(i), false) => (i + angles.len() - 1) % angles.len(),
            (None, _) => 0,
        };
        self.switch_halves(&doc, r, c, angles[next], switch, None, false)?;
        self.commit()
    }

    /// Show `angle` from `from_frame` to `to_frame` on the multicam clips
    /// there: cut where the stretch starts and ends, and switch every piece of
    /// the same comp between, on the rows of the clip under its start.
    fn paint(
        mut self,
        from_frame: i64,
        to_frame: i64,
        angle: Uuid,
        switch: BridgeAngleSwitch,
        linked: bool,
    ) -> Cut {
        let doc = self.of.document()?;
        let (from, to) = (self.time(from_frame)?, self.time(to_frame)?);
        if to <= from {
            return Err(BridgeCutResult::Nothing.into());
        }
        let (r, c) = self.multicam_under(&doc, from, None)?;
        let source = self.rows[r].clips[c].source;
        let sound_angle = self.angle_of(&doc, r, c, angle)?;
        let halves = self.halves(r, c);
        if halves.iter().any(|h| self.rows[h.r].locked) {
            return Err(BridgeCutResult::Locked.into());
        }
        let rows = |cut: bool| -> Vec<usize> {
            let takes = |h: &&Half| switch.takes(h, sound_angle) || cut;
            halves.iter().filter(takes).map(|h| h.r).collect()
        };
        let (taking, cutting) = (rows(false), rows(linked));
        for at in [from, to] {
            self.cut_picked(at, |r, clip| cutting.contains(&r) && clip.source == source)?;
        }
        let mut changed = false;
        for r in taking {
            for p in &mut self.rows[r].clips {
                let within = p.place_start >= from && p.place_end() <= to;
                if p.source == source && within && p.angle != Some(angle) {
                    p.angle = Some(angle);
                    changed = true;
                }
            }
        }
        if !changed {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.commit()
    }

    /// Hear `angle` under the multicam clip `clip` as well as what it plays: a
    /// sound clip of that angle across the same stretch, linked to it, on the
    /// first sound row below its sound with room there, else a new one. With
    /// `on` off, the sound clip of `angle` added under it goes.
    fn hear_angle(mut self, clip: Uuid, angle: Uuid, on: bool) -> Cut {
        let doc = self.of.document()?;
        let (r, c) = self.find(clip)?;
        if !plays_multicam(&doc, &self.rows[r].clips[c]) {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.angle_of(&doc, r, c, angle)?;
        let halves = self.halves(r, c);
        if halves.iter().any(|h| self.rows[h.r].locked) {
            return Err(BridgeCutResult::Locked.into());
        }
        let plays = |h: &&Half| h.sounds && self.rows[h.r].clips[h.c].angle == Some(angle);
        if !on {
            let gone: Vec<(usize, Uuid)> = halves
                .iter()
                .filter(|h| h.added)
                .filter(plays)
                .map(|h| (h.r, self.rows[h.r].clips[h.c].id))
                .collect();
            if gone.is_empty() {
                return Err(BridgeCutResult::Nothing.into());
            }
            for (r, id) in gone {
                self.rows[r].clips.retain(|c| c.id != id);
            }
            return self.commit();
        }
        if halves.iter().any(|h| plays(&h)) {
            return Err(BridgeCutResult::Nothing.into());
        }
        let main = halves.iter().find(|h| h.sounds && !h.added);
        let below = main.map_or(r, |h| h.r);
        let base = self.rows[r].clips[c].clone();
        let (start, end) = (base.place_start, base.place_end());
        let free = |row: &Row| {
            row.audio_only && !row.locked && !row.clips.iter().any(|o| covers(o, start, end))
        };
        let row = match (below + 1..self.rows.len()).find(|i| free(&self.rows[*i])) {
            Some(i) => i,
            None => {
                let (w, h) = (f64::from(self.comp.width), f64::from(self.comp.height));
                self.add_row(None, true, w, h)
            }
        };
        // Linked to the clip, so it moves and cuts with it.
        let link = base.link.unwrap_or_else(Uuid::now_v7);
        for h in &halves {
            self.rows[h.r].clips[h.c].link = Some(link);
        }
        let mut added = Clip::new(
            base.source,
            base.source_in,
            base.source_out,
            start,
            base.place_duration,
        );
        added.retime = base.retime.clone();
        added.angle = Some(angle);
        added.link = Some(link);
        let clips = &mut self.rows[row].clips;
        clips.push(added);
        clips.sort_by_key(|c| c.place_start);
        self.commit()
    }

    /// Put `angle` of the multicam comp that holds the footage under
    /// `at_frame` in step with it on the picture row above, from there to
    /// `to_frame`, or to the end of that footage's clip when `to_frame` is
    /// not after `at_frame`: a silent multicam clip showing the angle, on the
    /// nearest picture row above with room, else a new one on top.
    fn overwrite_angle(mut self, at_frame: i64, to_frame: i64, angle: Uuid) -> Cut {
        let doc = self.of.document()?;
        let from = self.time(at_frame)?;
        let Some(under) = lumit_core::multicam::under(&doc, &self.comp, from).filter(|u| u.plain)
        else {
            return Err(BridgeCutResult::Nothing.into());
        };
        let to = if to_frame > at_frame {
            self.time(to_frame)?
        } else {
            let (r, c) = self.find(under.clip)?;
            self.rows[r].clips[c].place_end()
        };
        let mc = doc.comp(under.comp).ok_or(BridgeError::InvalidComp)?;
        let picture = lumit_core::multicam::angles(mc).any(|l| l.id == angle && !l.audio_only);
        // The comp runs out where its angles do.
        let left = mc.duration.0.checked_sub(under.time)?;
        let to = to.min(from.checked_add(left)?);
        if !picture || to <= from {
            return Err(BridgeCutResult::Nothing.into());
        }
        let r = self.row_of(under.layer).ok_or(BridgeError::InvalidLayer)?;
        let free = |row: &Row| {
            !row.audio_only && !row.locked && !row.clips.iter().any(|o| covers(o, from, to))
        };
        let row = match (0..r).rev().find(|i| free(&self.rows[*i])) {
            Some(i) => i,
            None => {
                let (w, h) = (f64::from(mc.width), f64::from(mc.height));
                self.add_row(None, false, w, h)
            }
        };
        let length = to.checked_sub(from)?;
        let source_out = under.time.checked_add(length)?;
        let mut clip = Clip::new(
            ClipSource::Comp(under.comp),
            under.time,
            source_out,
            from,
            length,
        );
        clip.angle = Some(angle);
        clip.audible = false;
        let clips = &mut self.rows[row].clips;
        clips.push(clip);
        clips.sort_by_key(|c| c.place_start);
        self.commit()
    }

    /// Put each named multicam clip's own footage in its place: the clips
    /// its angle plays across its span ([`lumit_core::multicam::flattened`]).
    /// With `linked` its other half goes too.
    fn flatten(mut self, clips: &[Uuid], linked: bool) -> Cut {
        let doc = self.of.document()?;
        let ids = self.chosen(clips, linked)?;
        let mut flattened = 0;
        for id in ids {
            let (r, c) = self.find(id)?;
            let clip = &self.rows[r].clips[c];
            if !plays_multicam(&doc, clip) {
                continue;
            }
            let pieces =
                lumit_core::multicam::flattened(&doc, clip).ok_or(BridgeCutResult::Limit)?;
            let row = &mut self.rows[r];
            row.clips.remove(c);
            row.clips.extend(pieces);
            row.clips.sort_by_key(|c| c.place_start);
            flattened += 1;
        }
        if flattened == 0 {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.commit()
    }

    /// Put `comp`'s stretch from `source_in` to `source_out` down at
    /// `at_frame`, a multicam comp as a linked pair: its picture on `target`
    /// or a new picture row, showing `picture`, and its sound on a sound row,
    /// playing `sound`. With `with_picture` the picture clip plays its own
    /// sound instead. An unset angle is the one the comp starts its clips on.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_multicam(
        mut self,
        target: Option<Uuid>,
        comp: Uuid,
        source_in: Rational,
        source_out: Rational,
        at_frame: i64,
        insert: bool,
        with_picture: bool,
        picture: Option<Uuid>,
        sound: Option<Uuid>,
    ) -> Cut {
        if comp == self.comp.id {
            return Err(BridgeCutResult::WrongLayer.into());
        }
        let length = source_out.checked_sub(source_in)?;
        if source_in.is_negative() || length <= Rational::ZERO {
            return Err(BridgeError::InvalidTime.into());
        }
        let (width, height, start_picture, start_sound, has_picture, has_sound) = {
            let proj = self.of.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = state.store.snapshot();
            let mc = doc.comp(comp).ok_or(BridgeError::InvalidComp)?;
            let (p, s) = lumit_core::multicam::starting_angles(mc);
            let sounds = |layer: &Layer| {
                layer.audio_only
                    || match &layer.kind {
                        LayerKind::Sequence { clips } => clips.iter().any(|c| match c.source {
                            ClipSource::Footage(item) => match doc.item(item) {
                                Some(ProjectItem::Footage(f)) => has_sound(&state, f),
                                _ => false,
                            },
                            _ => false,
                        }),
                        _ => false,
                    }
            };
            let angles: Vec<&Layer> = lumit_core::multicam::angles(mc).collect();
            (
                f64::from(mc.width),
                f64::from(mc.height),
                p,
                s,
                angles.iter().any(|l| !l.audio_only),
                angles.iter().any(|l| sounds(l)),
            )
        };
        let picture = picture.or(start_picture);
        let sound = sound.or(start_sound).or(picture);
        let at = self.time(at_frame.max(0))?;
        let end = at.checked_add(length)?;
        let target = match target {
            Some(layer) => Some(self.row_of(layer).ok_or(BridgeError::NotSequence)?),
            None => None,
        };
        let onto_sound = target.is_some_and(|r| self.rows[r].audio_only);
        let picture_row = match target {
            _ if !has_picture || onto_sound => None,
            Some(r) => Some(r),
            None => Some(self.add_row(None, false, width, height)),
        };
        let heard = picture_row.is_some_and(|r| self.rows[r].audible);
        let picture_sounds = has_sound && heard && with_picture;
        let sound_row = if !has_sound || picture_sounds {
            None
        } else if onto_sound {
            target
        } else {
            // The sound row this picture row keeps its sound on, else the
            // first one with room, else a new one at the bottom.
            let links: Vec<Uuid> = picture_row
                .into_iter()
                .flat_map(|r| self.rows[r].clips.iter().filter_map(|c| c.link))
                .collect();
            let open = |row: &&Row| row.audio_only && !row.locked;
            let paired = |row: &&Row| {
                row.clips
                    .iter()
                    .any(|c| c.link.is_some_and(|l| links.contains(&l)))
            };
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
            match found.map(|row| row.id) {
                Some(id) => Some(self.row_of(id).ok_or(BridgeError::InvalidLayer)?),
                None => {
                    let (w, h) = (f64::from(self.comp.width), f64::from(self.comp.height));
                    Some(self.add_row(None, true, w, h))
                }
            }
        };
        let rows: Vec<usize> = picture_row.into_iter().chain(sound_row).collect();
        if rows.is_empty() {
            return Err(BridgeCutResult::WrongLayer.into());
        }
        if rows.iter().any(|r| self.rows[*r].locked) {
            return Err(BridgeCutResult::Locked.into());
        }
        if insert {
            self.razor(&rows, at, true)?;
            self.ripple(at, length)?;
        }
        let link = (rows.len() > 1).then(Uuid::now_v7);
        for r in rows {
            let mut clip = Clip::new(ClipSource::Comp(comp), source_in, source_out, at, length);
            let is_picture = Some(r) == picture_row;
            clip.angle = if is_picture { picture } else { sound };
            clip.link = link;
            clip.audible = !is_picture || picture_sounds;
            let dropped = clip.id;
            let row = &mut self.rows[r];
            row.clips.push(clip);
            row.clips = placed(std::mem::take(&mut row.clips), dropped, false);
        }
        self.commit()
    }
}

impl CompositionReference {
    /// **Hear an angle as well**: a sound clip of `angle` under the multicam
    /// clip `clip`, linked to it, which switching leaves on its angle; with
    /// `on` off, the one added goes. `Nothing` where it is already heard, or
    /// was never added.
    #[frb(sync)]
    pub fn cut_hear_angle(
        &self,
        clip: Uuid,
        angle: Uuid,
        on: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.hear_angle(clip, angle, on))
    }

    /// **Overwrite in step**: `angle` of the multicam comp holding the footage
    /// under `at_frame`, in step with it, on the picture row above, from
    /// there to `to_frame`, or to the end of the footage's clip when
    /// `to_frame` is not after `at_frame`. `Nothing` where no footage a
    /// multicam comp holds is, or for a sound angle.
    #[frb(sync)]
    pub fn cut_overwrite_angle(
        &self,
        at_frame: i64,
        to_frame: i64,
        angle: Uuid,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.overwrite_angle(at_frame, to_frame, angle))
    }

    /// **Cut to an angle**: cut the multicam clip under `at_frame` there, with
    /// its linked halves, and switch the piece after the cut to `angle`. The
    /// clip is the one on `row` when given, else on the top picture row with
    /// one. At a clip's own start nothing is cut and the whole clip switches.
    /// `Nothing` where no multicam clip is, or the angle already shows.
    #[frb(sync)]
    pub fn cut_to_angle(
        &self,
        at_frame: i64,
        angle: Uuid,
        switch: BridgeAngleSwitch,
        row: Option<LayerReference>,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        let row = row.map(|l| l.id());
        cut(self, |edit| {
            edit.cut_to(at_frame, angle, switch, row, true, linked)
        })
    }

    /// **Switch angle**: the named multicam clips switch to `angle` whole, or
    /// with none named the one under `at_frame`. No cut.
    #[frb(sync)]
    pub fn cut_switch_angle(
        &self,
        clips: Vec<Uuid>,
        at_frame: i64,
        angle: Uuid,
        switch: BridgeAngleSwitch,
    ) -> Result<BridgeCutResult, BridgeError> {
        if clips.is_empty() {
            return cut(self, |edit| {
                edit.cut_to(at_frame, angle, switch, None, false, false)
            });
        }
        cut(self, |edit| edit.switch_clips(&clips, angle, switch))
    }

    /// Switch the multicam clip under `at_frame` to the next angle, or the
    /// one before with `forward` off, round the comp's angles. No cut.
    #[frb(sync)]
    pub fn cut_step_angle(
        &self,
        at_frame: i64,
        forward: bool,
        switch: BridgeAngleSwitch,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.step(at_frame, forward, switch))
    }

    /// Show `angle` from `from_frame` to `to_frame` on the multicam clip
    /// there: a cut at each end and the stretch between switched.
    #[frb(sync)]
    pub fn cut_paint_angle(
        &self,
        from_frame: i64,
        to_frame: i64,
        angle: Uuid,
        switch: BridgeAngleSwitch,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| {
            edit.paint(from_frame, to_frame, angle, switch, linked)
        })
    }

    /// **Flatten**: each named multicam clip, and with `linked` its other
    /// half, becomes the footage clips its angle plays across its span.
    #[frb(sync)]
    pub fn cut_flatten(
        &self,
        clips: Vec<Uuid>,
        linked: bool,
    ) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.flatten(&clips, linked))
    }

    /// Put a multicam comp's stretch from `source_in` to `source_out` down at
    /// `at_frame`, overwriting or with `insert` moving everything after it
    /// later: its picture on `target` or a new picture row, its sound on a
    /// sound row, linked. `picture` and `sound` are the angles they start on,
    /// unset for the ones the comp names.
    #[allow(clippy::too_many_arguments)]
    #[frb(sync)]
    pub fn cut_place_multicam(
        &self,
        target: Option<LayerReference>,
        comp: CompositionReference,
        source_in: BridgeRational,
        source_out: BridgeRational,
        at_frame: i64,
        insert: bool,
        with_picture: bool,
        picture: Option<Uuid>,
        sound: Option<Uuid>,
    ) -> Result<BridgeCutResult, BridgeError> {
        let source_in =
            Rational::new(source_in.num, source_in.den).map_err(|_| BridgeError::InvalidTime)?;
        let source_out =
            Rational::new(source_out.num, source_out.den).map_err(|_| BridgeError::InvalidTime)?;
        let target = target.map(|l| l.id());
        cut(self, |edit| {
            edit.place_multicam(
                target,
                comp.id,
                source_in,
                source_out,
                at_frame,
                insert,
                with_picture,
                picture,
                sound,
            )
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
