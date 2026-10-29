//! A clip's audio streams in the Cut workspace: which a clip plays, and the
//! edits that put each on a sound row of its own or back on one clip.

use super::*;

impl Edit<'_> {
    /// The audio streams of the file a footage item names, and the item's
    /// name. None of either for anything else.
    pub(super) fn streams_of(&self, item: Uuid) -> Cut<(String, Vec<crate::streams::Stream>)> {
        let proj = self.of.project()?;
        let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let doc = state.store.snapshot();
        Ok(match doc.item(item) {
            Some(ProjectItem::Footage(f)) => (f.name.clone(), crate::streams::of_item(&state, f)),
            _ => (String::new(), Vec::new()),
        })
    }

    /// The clips an edit of audio streams works on: the named ones and the
    /// clips linked to them, less any that is not heard, on its own or by its
    /// row, which has no sound to split or merge. So the edit can be asked of
    /// a picture clip and lands on the clip that carries its sound.
    pub(super) fn sounding(&self, clips: &[Uuid]) -> Cut<Vec<Uuid>> {
        let mut heard = Vec::new();
        for id in self.with_linked(clips, true)? {
            let (r, c) = self.find(id)?;
            if self.rows[r].audible && self.rows[r].clips[c].audible {
                heard.push(id);
            }
        }
        Ok(heard)
    }

    /// **Split audio streams.** Each clip that sounds several streams of its
    /// file is left sounding the first of them, and each of the others goes
    /// down as a clip of its own, linked to it, on a sound row: the first
    /// with room that holds none of the others, else a new one at the bottom.
    /// A clip that sounds the first stream, as one does until told otherwise,
    /// is split into every stream its file has, less any a clip already
    /// split off beside it sounds: a clip that has been split has no more to
    /// give.
    pub(super) fn split_streams(mut self, clips: &[Uuid]) -> Cut {
        let mut split = false;
        for id in self.sounding(clips)? {
            let (r, c) = self.find(id)?;
            let clip = self.rows[r].clips[c].clone();
            let ClipSource::Footage(item) = clip.source else {
                continue;
            };
            let (name, streams) = self.streams_of(item)?;
            let sound_rows = self.rows.iter().filter(|row| row.audio_only);
            let apart: Vec<u32> = sound_rows
                .flat_map(|row| row.clips.iter().filter(|other| same_stretch(&clip, other)))
                .flat_map(|other| other.audio_streams.places().to_vec())
                .collect();
            // The clip keeps the first of its streams whatever is beside it.
            let places = crate::streams::to_split(&clip.audio_streams, streams.len());
            let Some((first, rest)) = places.split_first() else {
                continue;
            };
            let rest: Vec<u32> = rest
                .iter()
                .copied()
                .filter(|p| !apart.contains(p))
                .collect();
            if rest.is_empty() {
                continue;
            }
            if self.rows[r].locked {
                return Err(BridgeCutResult::Locked.into());
            }
            let link = clip.link.unwrap_or_else(Uuid::now_v7);
            let kept = &mut self.rows[r].clips[c];
            kept.link = Some(link);
            kept.audio_streams = AudioStreams::only(*first);

            let (start, end) = (clip.place_start, clip.place_end());
            // A row with room for each stream, where there is one.
            let mut used = vec![r];
            let mut landings: Vec<Option<usize>> = Vec::with_capacity(rest.len());
            for _ in &rest {
                let room = (0..self.rows.len()).find(|i| {
                    let row = &self.rows[*i];
                    row.audio_only
                        && !row.locked
                        && !used.contains(i)
                        && !row.clips.iter().any(|c| covers(c, start, end))
                });
                used.extend(room);
                landings.push(room);
            }
            // And a new row for each of the rest, the last stream's first:
            // each goes in above the one made before it, so the streams read
            // down the stack in the file's own order.
            for (n, place) in rest.iter().enumerate().rev() {
                if landings[n].is_none() {
                    let (w, h) = (f64::from(self.comp.width), f64::from(self.comp.height));
                    let name = crate::streams::split_name(&name, *place, &streams);
                    landings[n] = Some(self.add_row(name, true, w, h));
                }
            }
            for (place, landing) in rest.iter().zip(landings.into_iter().flatten()) {
                let mut sound = clip.duplicate();
                sound.link = Some(link);
                sound.audio_streams = AudioStreams::only(*place);
                self.rows[landing].clips.push(sound);
            }
            split = true;
        }
        if !split {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.commit()
    }

    /// **Merge audio streams**, the way back from a split. A clip takes the
    /// streams of every clip on a sound row that is the same stretch of the
    /// same file at the same moment, and those clips go. A sound row left
    /// with nothing on it goes too. A clip with no such clip beside it sounds
    /// every stream its file has.
    pub(super) fn merge_streams(mut self, clips: &[Uuid]) -> Cut {
        let mut merged = false;
        let mut folded: Vec<Uuid> = Vec::new();
        for id in self.sounding(clips)? {
            if folded.contains(&id) {
                continue;
            }
            let (r, c) = self.find(id)?;
            let kept = self.rows[r].clips[c].clone();
            let ClipSource::Footage(item) = kept.source else {
                continue;
            };
            let beside = |other: &&Clip| same_stretch(&kept, other);
            let mut places = kept.audio_streams.places().to_vec();
            let mut gone: Vec<Uuid> = Vec::new();
            for (i, row) in self.rows.iter().enumerate() {
                if i == r || !row.audio_only || row.locked {
                    continue;
                }
                for other in row.clips.iter().filter(beside) {
                    places.extend_from_slice(other.audio_streams.places());
                    gone.push(other.id);
                }
            }
            let plays = if gone.is_empty() {
                crate::streams::all(self.streams_of(item)?.1.len())
            } else {
                AudioStreams::of(&places)
            };
            if gone.is_empty() && plays == kept.audio_streams {
                continue;
            }
            if self.rows[r].locked {
                return Err(BridgeCutResult::Locked.into());
            }
            self.rows[r].clips[c].audio_streams = plays;
            for row in &mut self.rows {
                let before = row.clips.len();
                row.clips.retain(|c| !gone.contains(&c.id));
                if row.clips.is_empty() && before > 0 && row.audio_only && row.fresh.is_none() {
                    self.gone.push(row.id);
                }
            }
            folded.extend(gone);
            merged = true;
        }
        if !merged {
            return Err(BridgeCutResult::Nothing.into());
        }
        self.commit()
    }
}
