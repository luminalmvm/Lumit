//! Joining clips back together across a cut that changed nothing: two clips
//! that meet edge to edge and play straight on through one source, as a cut
//! with nothing done to either piece leaves them.

use super::*;
use lumit_core::model::EffectInstance;
use lumit_core::sequence::Fade;

/// `a` and `b` as one clip, where `b` carries straight on from `a`: the same
/// source, on from the moment `a` stops, neither retimed, nothing fading or
/// dissolving at the join, and alike in everything else, effects keyed on
/// one clock. A multicam clip takes `a`'s angle. `None` otherwise.
///
/// ponytail: retimed clips stay apart; the upgrade is joining their maps.
#[frb(ignore)]
fn joined(a: &Clip, b: &Clip) -> Option<Clip> {
    let near = |x: f64, y: f64| (x - y).abs() < 1e-9;
    let meet = a.place_end() == b.place_start && a.source_out == b.source_in;
    let plain = a.retime.is_none() && b.retime.is_none();
    let open =
        a.fade_out == Fade::default() && b.fade_in == Fade::default() && b.transition.is_none();
    let alike = a.interpolation == b.interpolation
        && a.fx == b.fx
        && near(a.gain_db, b.gain_db)
        && near(a.opacity, b.opacity)
        && a.audio_streams == b.audio_streams
        && a.audible == b.audible
        && a.blend == b.blend
        && a.label == b.label
        && a.takes == b.takes
        && a.extra == b.extra;
    if a.source != b.source || !meet || !plain || !open || !alike {
        return None;
    }
    // The later piece's keys sit on its own clock, which starts where `a`'s
    // ends.
    let mut later = b.effects.clone();
    lumit_core::preset::shift_keys(&mut later, a.place_duration);
    let same = a.effects.len() == later.len()
        && a.effects.iter().zip(&later).all(|(x, y)| {
            *x == EffectInstance {
                id: x.id,
                ..y.clone()
            }
        });
    if !same {
        return None;
    }
    Some(Clip {
        source_out: b.source_out,
        place_duration: a.place_duration.checked_add(b.place_duration).ok()?,
        fade_out: b.fade_out,
        ..a.clone()
    })
}

impl Edit<'_> {
    /// Join the named clips wherever two of them meet and the later carries
    /// straight on from the earlier. With `linked` the clips linked to them
    /// join too, and a pair whose linked clips cannot join stays apart, so
    /// linked clips stay paired.
    fn join(mut self, clips: &[Uuid], linked: bool) -> Cut {
        let ids = self.chosen(clips, linked)?;
        // Every join between two named clips: its row, and the two.
        let mut seams: Vec<(usize, Uuid, Uuid)> = Vec::new();
        for (r, row) in self.rows.iter().enumerate() {
            let mut order: Vec<&Clip> = row.clips.iter().collect();
            order.sort_by_key(|c| c.place_start);
            for pair in order.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if ids.contains(&a.id) && ids.contains(&b.id) && joined(a, b).is_some() {
                    seams.push((r, a.id, b.id));
                }
            }
        }
        if linked {
            let link_of = |id: Uuid| {
                self.rows
                    .iter()
                    .flat_map(|row| &row.clips)
                    .find(|c| c.id == id)
                    .and_then(|c| c.link)
            };
            let links: Vec<(usize, Option<Uuid>, Option<Uuid>)> = seams
                .iter()
                .map(|(r, a, b)| (*r, link_of(*a), link_of(*b)))
                .collect();
            // A join stays where every other row holding a clip linked to
            // either side has a join between the same two links.
            seams.retain(|(r, a, b)| {
                let (la, lb) = (link_of(*a), link_of(*b));
                self.rows.iter().enumerate().all(|(rr, row)| {
                    let holds = row
                        .clips
                        .iter()
                        .any(|c| c.link.is_some() && (c.link == la || c.link == lb));
                    rr == *r || !holds || links.contains(&(rr, la, lb))
                })
            });
        }
        if seams.is_empty() {
            return Err(BridgeCutResult::Nothing.into());
        }
        for (r, row) in self.rows.iter_mut().enumerate() {
            if !seams.iter().any(|(sr, _, _)| *sr == r) {
                continue;
            }
            let mut order = std::mem::take(&mut row.clips);
            order.sort_by_key(|c| c.place_start);
            let mut out: Vec<Clip> = Vec::with_capacity(order.len());
            // The id the last clip out ends with, which a join is found by.
            let mut last: Option<Uuid> = None;
            for clip in order {
                let at_seam = last.is_some_and(|l| seams.contains(&(r, l, clip.id)));
                let id = clip.id;
                match out.last_mut() {
                    Some(prev) if at_seam => match joined(prev, &clip) {
                        Some(both) => *prev = both,
                        None => out.push(clip),
                    },
                    _ => out.push(clip),
                }
                last = Some(id);
            }
            row.clips = out;
        }
        self.commit()
    }
}

impl CompositionReference {
    /// **Join**: wherever two named clips meet and the later carries
    /// straight on from the earlier, they become one clip, as before a cut
    /// that changed nothing. A multicam clip takes the earlier piece's angle.
    /// With `linked` their linked clips join too. `Nothing` where none can.
    #[frb(sync)]
    pub fn cut_join(&self, clips: Vec<Uuid>, linked: bool) -> Result<BridgeCutResult, BridgeError> {
        cut(self, |edit| edit.join(&clips, linked))
    }
}
