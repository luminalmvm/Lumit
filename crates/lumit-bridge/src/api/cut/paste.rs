//! Pasting in the Cut workspace: the clips last copied go down again, on the
//! rows they came from or beside a chosen one.

use super::*;

impl Edit<'_> {
    pub(super) fn paste(
        self,
        at_frame: i64,
        target: Option<LayerReference>,
        end_frame: &mut i64,
    ) -> Cut {
        let at = self.time(at_frame.max(0))?;
        let held = self.copied()?;
        self.paste_held(held, at, target, end_frame)
    }

    /// The clips last copied in this project. `Nothing` when there are none.
    pub(super) fn copied(&self) -> Cut<Vec<Held>> {
        let copied = COPIED.lock().map_err(|_| BridgeError::ReadFailed)?;
        match copied.as_ref() {
            Some((project, held)) if *project == self.of.project_id() => Ok(held.clone()),
            _ => Err(BridgeCutResult::Nothing.into()),
        }
    }

    /// Put `held` down with the earliest of them at `at`, as a paste does.
    pub(super) fn paste_held(
        mut self,
        held: Vec<Held>,
        at: Rational,
        target: Option<LayerReference>,
        end_frame: &mut i64,
    ) -> Cut {
        let mut clips: Vec<Clip> = Vec::with_capacity(held.len());
        for h in &held {
            let mut clip = h.clip.clone();
            clip.place_start = clip.place_start.checked_add(at)?;
            clips.push(clip);
        }
        let of_lane = |sound: bool, lane: usize| {
            let pairs = held.iter().zip(&clips);
            pairs.filter_map(move |(h, clip)| ((h.sound, h.lane) == (sound, lane)).then_some(clip))
        };

        // Each lane of the copy, top first with the picture lanes ahead of
        // the sound, beside how big a layer made for it would be, and the row
        // it lands on once that is known.
        type Lane = ((bool, usize), (f64, f64), Option<usize>);
        let mut lanes: Vec<Lane> = Vec::new();
        {
            let proj = self.of.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let doc = state.store.snapshot();
            let mut keys: Vec<(bool, usize)> = held.iter().map(|h| (h.sound, h.lane)).collect();
            keys.sort_unstable();
            keys.dedup();
            for (sound, lane) in keys {
                let mut look = None;
                for clip in of_lane(sound, lane) {
                    // A composition cannot play itself, and an item deleted
                    // since the copy is nothing to paste.
                    let size = match clip.source {
                        ClipSource::Comp(id) if id == self.comp.id => {
                            return Err(BridgeCutResult::WrongLayer.into())
                        }
                        ClipSource::Comp(id) => {
                            let inner = doc.comp(id).ok_or(BridgeCutResult::Nothing)?;
                            (f64::from(inner.width), f64::from(inner.height))
                        }
                        ClipSource::Footage(id) => {
                            let Some(ProjectItem::Footage(f)) = doc.item(id) else {
                                return Err(BridgeCutResult::Nothing.into());
                            };
                            let (_, w, h) =
                                CompositionReference::footage_span_and_size(&state, f, &self.comp);
                            (w, h)
                        }
                        ClipSource::Solid(id) => {
                            let sd = doc.solid(id).ok_or(BridgeCutResult::Nothing)?;
                            (f64::from(sd.width), f64::from(sd.height))
                        }
                    };
                    look.get_or_insert(size);
                }
                lanes.extend(look.map(|look| ((sound, lane), look, None)));
            }
        }
        let Some(&((lead_sound, lead_lane), ..)) = lanes.first() else {
            return Err(BridgeCutResult::Nothing.into());
        };
        let of_kind = |sound: bool| -> Vec<usize> {
            let kind = |r: &usize| self.rows[*r].audio_only == sound;
            (0..self.rows.len()).filter(kind).collect()
        };
        let (pictures, sounds) = (of_kind(false), of_kind(true));
        // The target is for the top-most picture clip, or the top-most sound
        // clip when no picture was copied: its place among rows of that kind.
        let target = match target {
            Some(layer) => {
                let r = self.row_of(layer.id()).ok_or(BridgeError::NotSequence)?;
                if self.rows[r].audio_only != lead_sound {
                    return Err(BridgeCutResult::WrongLayer.into());
                }
                let rows = if lead_sound { &sounds } else { &pictures };
                rows.iter().position(|o| *o == r)
            }
            None => None,
        };
        let mut next_sound = 0;
        for ((sound, lane), _, landing) in &mut lanes {
            let rows = if *sound { &sounds } else { &pictures };
            *landing = match target {
                // As a move chooses rows: the lanes of the lead clip's kind
                // cross as many layers as it does, and the rest keep theirs.
                Some(to) if *sound == lead_sound => rows.get(*lane - lead_lane + to).copied(),
                Some(_) => rows.get(*lane).copied(),
                // As a placed clip chooses them: a sound lane takes the first
                // sound layer with room, below the one the lane above took,
                // and a picture lane a layer of its own.
                None if *sound => {
                    let free = |i: &usize| {
                        let row = &self.rows[rows[*i]];
                        let mut mine = of_lane(*sound, *lane);
                        !row.locked
                            && !mine.any(|m| {
                                let (start, end) = (m.place_start, m.place_end());
                                row.clips.iter().any(|c| covers(c, start, end))
                            })
                    };
                    let found = (next_sound..rows.len()).find(free);
                    next_sound = found.map_or(rows.len(), |i| i + 1);
                    found.map(|i| rows[i])
                }
                None => None,
            };
            if landing.is_some_and(|r| self.rows[r].locked) {
                return Err(BridgeCutResult::Locked.into());
            }
        }
        // A lane with no layer gets one. A picture layer missing below the
        // ones a target counted from goes under the last of them, and the
        // bottom lane is made first, so each lands above the one before it
        // and the lanes keep their order.
        let under = pictures.last().and_then(|r| {
            let id = self.rows[*r].id;
            self.comp.layers.iter().position(|l| l.id == id)
        });
        let (width, height) = (f64::from(self.comp.width), f64::from(self.comp.height));
        for ((sound, _), (w, h), landing) in lanes.iter_mut().rev() {
            if landing.is_some() {
                continue;
            }
            let (w, h) = if *sound { (width, height) } else { (*w, *h) };
            let made = self.add_row(None, *sound, w, h);
            if let (false, Some(_), Some(under)) = (*sound, target, under) {
                self.rows[made].at = under + 1;
            }
            *landing = Some(made);
        }

        let mut lifted: Vec<(usize, Clip)> = Vec::with_capacity(clips.len());
        let mut last = at;
        for (h, clip) in held.iter().zip(clips) {
            let lane = lanes.iter().find(|(key, ..)| *key == (h.sound, h.lane));
            if let Some((.., Some(row))) = lane {
                last = last.max(clip.place_end());
                lifted.push((*row, clip));
            }
        }
        renew(lifted.iter_mut().map(|(_, clip)| clip));
        self.land(lifted, false)?;
        *end_frame = self.frame_up(last)?;
        self.commit()
    }
}
