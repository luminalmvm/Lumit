//! Multicam comps, and the clips that play one of their angles.
//!
//! A multicam comp is an ordinary composition whose Sequence layers are the
//! **angles** of one shoot. Each holds one camera's or one recorder's files at
//! the moments they were filmed, so every angle is in step with the others. A
//! **multicam clip** plays that comp and names one angle ([`Clip::angle`]): it
//! shows, or sounds, that angle and nothing else.
//!
//! The render never hears the word. A clip that names an angle plays its comp
//! seen through that angle ([`seen_through`]): the angle's row switched on and
//! every other angle's switched off. So the frame key, the decode plan and the
//! mix all read one ordinary composition, and two angles of one comp at the
//! same moment are two pictures with two names.
//!
//! Runs wherever the document is read.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{AudioStreams, Composition, Document, EffectInstance, Layer, LayerKind};
use crate::sequence::{Clip, ClipSource, Fade};
use crate::time::Rational;

/// What marks a composition as a multicam comp: the angles a multicam clip of
/// it starts on. Kept in the comp's `extra` ([`Composition::multicam`]), so a
/// Lumit that does not know it keeps it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Multicam {
    /// The angle a new multicam clip shows. Unset, or an angle that has gone,
    /// is the first angle with a picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<Uuid>,
    /// The angle a new multicam clip's sound plays. Unset is the picture's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<Uuid>,
    /// Unknown fields from newer Lumit versions (docs/10-FILE-FORMAT.md §1.1).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Composition {
    /// The key a multicam comp's [`Multicam`] is kept under in `extra`.
    pub const MULTICAM_KEY: &'static str = "multicam";

    /// This comp's multicam mark, or `None` for an ordinary comp.
    #[must_use]
    pub fn multicam(&self) -> Option<Multicam> {
        self.extra
            .get(Self::MULTICAM_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Whether this is a multicam comp, without reading the mark.
    #[must_use]
    pub fn is_multicam(&self) -> bool {
        self.extra.contains_key(Self::MULTICAM_KEY)
    }

    /// Set the multicam mark, or take it away with `None`.
    pub fn set_multicam(&mut self, multicam: Option<&Multicam>) {
        match multicam.and_then(|m| serde_json::to_value(m).ok()) {
            Some(v) => {
                self.extra.insert(Self::MULTICAM_KEY.to_owned(), v);
            }
            None => {
                self.extra.remove(Self::MULTICAM_KEY);
            }
        }
    }
}

/// Apply [`crate::ops::Op::SetCompMulticam`], answering its inverse.
pub(crate) fn set_mark(
    doc: &mut Document,
    comp: Uuid,
    mark: &Option<Box<Multicam>>,
) -> Result<crate::ops::Op, crate::ops::OpError> {
    let c = doc.comp_mut(comp).ok_or(crate::ops::OpError::UnknownComp)?;
    let previous = c.multicam().map(Box::new);
    c.set_multicam(mark.as_deref());
    Ok(crate::ops::Op::SetCompMulticam {
        comp,
        multicam: previous,
    })
}

/// The angles of `comp`: its Sequence layers, the top one first.
pub fn angles(comp: &Composition) -> impl Iterator<Item = &Layer> {
    comp.layers
        .iter()
        .filter(|l| matches!(l.kind, LayerKind::Sequence { .. }))
}

/// The angles the angle viewer, its keys and the angle menus offer: those
/// whose row is switched on inside the multicam comp, by its eye, or by its
/// sound for a sound angle. A clip of an angle switched off still plays it.
pub fn offered(comp: &Composition) -> impl Iterator<Item = &Layer> {
    angles(comp).filter(|l| {
        if l.audio_only {
            l.switches.audible
        } else {
            l.switches.visible
        }
    })
}

/// Where `angle` stands among the angles `comp` offers, counting from one.
#[must_use]
pub fn number_of(comp: &Composition, angle: Uuid) -> Option<usize> {
    offered(comp).position(|l| l.id == angle).map(|i| i + 1)
}

/// The first angle offered with a picture, else the first with one: what a
/// multicam clip shows when nothing says otherwise.
#[must_use]
pub fn first_picture(comp: &Composition) -> Option<Uuid> {
    let picture = |l: &&Layer| !l.audio_only;
    offered(comp)
        .find(picture)
        .or_else(|| angles(comp).find(picture))
        .map(|l| l.id)
}

/// The angles a new multicam clip of `comp` starts on: its picture angle, and
/// the angle its sound plays. An angle the mark names that has gone falls
/// back as an unset one does.
#[must_use]
pub fn starting_angles(comp: &Composition) -> (Option<Uuid>, Option<Uuid>) {
    let mark = comp.multicam().unwrap_or_default();
    let here = |id: Option<Uuid>| id.filter(|id| number_of(comp, *id).is_some());
    let picture = here(mark.picture).or_else(|| first_picture(comp));
    (picture, here(mark.sound).or(picture))
}

/// `comp` seen through `angle`: the angle's row on, drawn when it has a
/// picture and heard, and every other angle's row off. Other layers keep
/// their switches, so a title or an adjustment over every angle still works.
///
/// The angle's own switches are set on rather than read: they are for working
/// inside the multicam comp, as soloing an angle to look at it is, and must
/// not change the cuts that use it. An angle that has gone leaves every angle
/// off, so a clip of a deleted angle is black and silent.
#[must_use]
pub fn seen_through(comp: &Composition, angle: Uuid) -> Composition {
    let mut seen = comp.clone();
    for layer in &mut seen.layers {
        if !matches!(layer.kind, LayerKind::Sequence { .. }) {
            continue;
        }
        let on = layer.id == angle;
        layer.switches.visible = on && !layer.audio_only;
        layer.switches.audible = on;
        layer.switches.solo = false;
    }
    seen
}

/// The comp a clip of a comp plays, seen through its angle when it names one.
/// `None` for a clip of footage or a solid, and for a comp that has gone.
///
/// Every reading of a comp clip's comp goes through here (the picture, its
/// frame key, the decode plan and the mix), so they cannot disagree about
/// which angle a clip shows.
#[must_use]
pub fn nested_for<'a>(doc: &'a Document, clip: &Clip) -> Option<Cow<'a, Composition>> {
    let ClipSource::Comp(id) = clip.source else {
        return None;
    };
    let comp = doc.comp(id)?;
    Some(match clip.angle {
        Some(angle) => Cow::Owned(seen_through(comp, angle)),
        None => Cow::Borrowed(comp),
    })
}

/// The footage items a clip's angle plays, or `None` for a clip that names no
/// angle. What a probe ahead of a multicam clip needs, rather than every file
/// of every angle.
#[must_use]
pub fn angle_items(doc: &Document, clip: &Clip) -> Option<Vec<Uuid>> {
    let layer = angle_layer(doc, clip)?;
    let LayerKind::Sequence { clips } = &layer.kind else {
        return None;
    };
    let mut items: Vec<Uuid> = Vec::new();
    for source in clips.iter().flat_map(Clip::sources) {
        if let ClipSource::Footage(item) = source {
            if !items.contains(&item) {
                items.push(item);
            }
        }
    }
    Some(items)
}

/// The layer a multicam clip's angle is.
#[must_use]
pub fn angle_layer<'a>(doc: &'a Document, clip: &Clip) -> Option<&'a Layer> {
    let (ClipSource::Comp(id), Some(angle)) = (clip.source, clip.angle) else {
        return None;
    };
    doc.comp(id)?.layers.iter().find(|l| l.id == angle)
}

/// What a clip shows or sounds at one moment: the footage item, the moment of
/// it, and the streams of it that are heard.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub item: Uuid,
    /// Seconds of the item's own time.
    pub source_time: Rational,
    pub streams: AudioStreams,
}

/// What `clip` shows or sounds at layer time `lt` (the clock of
/// [`Clip::source_at`]). A clip of footage answers for itself. A multicam
/// clip follows its angle into its comp and answers for the angle's clip
/// there. `None` in a gap of the angle, for a clip of a solid or of a comp
/// with no angle named, and on overflow.
#[must_use]
pub fn footage_at(doc: &Document, clip: &Clip, lt: Rational) -> Option<Shown> {
    let at = clip.source_at(lt)?;
    match clip.source {
        ClipSource::Footage(item) => Some(Shown {
            item,
            source_time: at,
            streams: clip.audio_streams.clone(),
        }),
        ClipSource::Comp(_) => {
            let layer = angle_layer(doc, clip)?;
            let LayerKind::Sequence { clips } = &layer.kind else {
                return None;
            };
            let inner_lt = at.checked_sub(layer.start_offset.0).ok()?;
            let inner = clips
                .iter()
                .find(|c| c.place_start <= inner_lt && inner_lt < c.place_end())?;
            let ClipSource::Footage(item) = inner.source else {
                return None;
            };
            Some(Shown {
                item,
                source_time: inner.source_at(inner_lt)?,
                streams: inner.audio_streams.clone(),
            })
        }
        ClipSource::Solid(_) => None,
    }
}

/// What `clip` shows first: [`footage_at`] its start, or for a multicam clip
/// that starts in a gap of its angle, the angle's first file inside it. So a
/// piece cut to a camera before it rolled still has a picture to be known by.
#[must_use]
pub fn opens_on(doc: &Document, clip: &Clip) -> Option<Shown> {
    if let Some(shown) = footage_at(doc, clip, clip.place_start) {
        return Some(shown);
    }
    // A retimed clip has no straight run through its angle to look along.
    if clip.retime.is_some() {
        return None;
    }
    let layer = angle_layer(doc, clip)?;
    let LayerKind::Sequence { clips } = &layer.kind else {
        return None;
    };
    let from = clip.source_in.checked_sub(layer.start_offset.0).ok()?;
    let to = clip.source_out.checked_sub(layer.start_offset.0).ok()?;
    let inner = clips
        .iter()
        .filter(|c| c.place_start >= from && c.place_start < to)
        .min_by_key(|c| c.place_start)?;
    let ClipSource::Footage(item) = inner.source else {
        return None;
    };
    Some(Shown {
        item,
        source_time: inner.source_at(inner.place_start)?,
        streams: inner.audio_streams.clone(),
    })
}

/// What a clip sounds at layer time `lt`: [`footage_at`], named for the
/// readers that follow speech through a cut.
#[must_use]
pub fn heard_at(doc: &Document, clip: &Clip, lt: Rational) -> Option<Shown> {
    footage_at(doc, clip, lt)
}

/// The multicam clip under one moment of a composition, as the angle viewer
/// follows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Under {
    /// The Sequence layer the clip is on.
    pub layer: Uuid,
    pub clip: Uuid,
    /// The multicam comp it plays.
    pub comp: Uuid,
    /// The moment of the multicam comp it shows, in seconds.
    pub time: Rational,
    /// The angle shown, from the picture clip, and the angle heard, from the
    /// clip that sounds: the picture clip's linked sound clip, or the clip
    /// itself when it is heard.
    pub picture: Option<Uuid>,
    pub sound: Option<Uuid>,
    /// The angles heard as well: sound clips of the same comp linked to it
    /// besides the one that plays its sound.
    pub also: Vec<Uuid>,
    /// The clip is footage one of the comp's angles holds, not a multicam
    /// clip: the comp is in step with it, and `picture` is that angle.
    pub plain: bool,
}

/// The multicam clip under comp time `t` of `comp`: on the top picture row
/// that has one, else on the top sound row. Where there is none, footage one
/// of a multicam comp's angles holds, on the top picture row with a clip
/// there ([`Under::plain`]). `None` where neither is.
#[must_use]
pub fn under(doc: &Document, comp: &Composition, t: Rational) -> Option<Under> {
    let rows = || {
        comp.layers.iter().filter_map(|l| match &l.kind {
            LayerKind::Sequence { clips } => Some((l, clips)),
            _ => None,
        })
    };
    let at = |layer: &Layer, clips: &'_ [Clip]| -> Option<(Uuid, Clip)> {
        let lt = t.checked_sub(layer.start_offset.0).ok()?;
        clips
            .iter()
            .find(|c| {
                c.angle.is_some()
                    && c.place_start <= lt
                    && lt < c.place_end()
                    && matches!(c.source, ClipSource::Comp(id) if doc.comp(id).is_some_and(Composition::is_multicam))
            })
            .map(|c| (layer.id, c.clone()))
    };
    let found = rows()
        .filter(|(l, _)| !l.audio_only)
        .find_map(|(l, clips)| at(l, clips))
        .or_else(|| {
            rows()
                .filter(|(l, _)| l.audio_only)
                .find_map(|(l, clips)| at(l, clips))
        });
    let Some((layer, clip)) = found else {
        return synced(doc, comp, t);
    };
    let ClipSource::Comp(mc) = clip.source else {
        return None;
    };
    let row = comp.layers.iter().find(|l| l.id == layer)?;
    let time = clip.source_at(t.checked_sub(row.start_offset.0).ok()?)?;
    // The clips that sound, top row first: the clip itself where it is heard,
    // then the clips of the same comp linked to it. The first plays its sound
    // and the rest are heard as well.
    let linked = |c: &Clip| {
        c.id != clip.id && clip.link.is_some() && c.link == clip.link && c.source == clip.source
    };
    let mut heard: Vec<Uuid> = Vec::new();
    if row.audio_only || clip.audible {
        heard.extend(clip.angle);
    }
    for (l, clips) in rows() {
        for c in clips
            .iter()
            .filter(|c| linked(c) && (l.audio_only || c.audible))
        {
            if let Some(angle) = c.angle.filter(|a| !heard.contains(a)) {
                heard.push(angle);
            }
        }
    }
    let picture = if row.audio_only { None } else { clip.angle };
    Some(Under {
        layer,
        clip: clip.id,
        comp: mc,
        time,
        picture,
        sound: heard.first().copied(),
        also: heard.into_iter().skip(1).collect(),
        plain: false,
    })
}

/// The footage clip under comp time `t` of `comp`, on the top picture row
/// with a clip there, when one angle of a multicam comp holds the moment it
/// shows: that comp, in step with it ([`Under::plain`]). Footage that plays
/// at any speed but its own is in step with none.
fn synced(doc: &Document, comp: &Composition, t: Rational) -> Option<Under> {
    let (layer, clip) = comp.layers.iter().filter(|l| !l.audio_only).find_map(|l| {
        let LayerKind::Sequence { clips } = &l.kind else {
            return None;
        };
        let lt = t.checked_sub(l.start_offset.0).ok()?;
        let there = |c: &&Clip| c.place_start <= lt && lt < c.place_end();
        Some((l, clips.iter().filter(there).max_by_key(|c| c.place_start)?))
    })?;
    let ClipSource::Footage(item) = clip.source else {
        return None;
    };
    // A multicam clip is laid at its comp's own pace, so only footage playing
    // at its file's own pace stays in step with one.
    if clip.constant_speed() != Some(1.0) {
        return None;
    }
    let shown = clip.source_at(t.checked_sub(layer.start_offset.0).ok()?)?;
    let comps = doc.items.iter().filter_map(|i| match i {
        crate::model::ProjectItem::Composition(c) if c.is_multicam() && c.id != comp.id => Some(c),
        _ => None,
    });
    for mc in comps {
        for angle in angles(mc) {
            let LayerKind::Sequence { clips } = &angle.kind else {
                continue;
            };
            let mut of_item = clips
                .iter()
                .filter(|c| c.source == ClipSource::Footage(item));
            if let Some(lt) = of_item.find_map(|c| place_of(c, shown)) {
                return Some(Under {
                    layer: layer.id,
                    clip: clip.id,
                    comp: mc.id,
                    time: lt.checked_add(angle.start_offset.0).ok()?,
                    picture: Some(angle.id),
                    sound: None,
                    also: Vec::new(),
                    plain: true,
                });
            }
        }
    }
    None
}

/// Where on its row `clip` shows moment `s` of its source: the other way
/// from [`Clip::source_at`], for a clip at one speed. `None` outside what it
/// plays, and for a clip whose speed changes along it.
fn place_of(clip: &Clip, s: Rational) -> Option<Rational> {
    if s < clip.source_in || s >= clip.source_out {
        return None;
    }
    let speed = clip.constant_speed().filter(|v| *v > 0.0)?;
    let speed = Rational::from_f64_on_grid(speed, Rational::FLICK_DEN).ok()?;
    let along = s
        .checked_sub(clip.source_in)
        .ok()?
        .checked_div(speed)
        .ok()?;
    clip.place_start.checked_add(along).ok()
}

/// The footage clips a multicam clip's angle plays across the clip's span,
/// placed where the clip sits on its own row's clock: what Flatten puts in its
/// place, so the row plays what it played. A gap in the angle stays a gap.
///
/// Each piece carries the clip's speed, gain, opacity, label, link and group,
/// and keeps the angle clip's own trim, speed and gain. Its effects are the
/// angle clip's own, then the angle row's, then the clip's, keys moved onto the
/// piece's clock. A stack whose fx switch was off comes across switched off,
/// effect by effect.
///
/// A piece fades as the clip did at the clip's outer ends, and as the angle
/// clip did at the angle clip's own ends. Where both fade one end, the longer
/// fade is kept. Its blend mode is the clip's, else the angle clip's. Its
/// transition is the clip's at the clip's start, and the angle clip's at the
/// angle clip's own start, so a dissolve between two files of one angle stays.
///
/// `None` when the clip names no angle, or plays at a speed that changes along
/// it, is frozen or runs backwards, or on overflow.
///
/// ponytail: keyed effects from inside the comp are moved, not scaled, onto a
/// piece that plays faster or slower than the comp, and an angle row's
/// transform is not carried. An angle clip's fade that the clip's trim cuts
/// through is dropped, not shortened. The upgrade is a Transform effect on each
/// piece from the row's own placement, and a fade cut to the part kept.
#[must_use]
pub fn flattened(doc: &Document, clip: &Clip) -> Option<Vec<Clip>> {
    let layer = angle_layer(doc, clip)?;
    let LayerKind::Sequence { clips: inner } = &layer.kind else {
        return None;
    };
    let speed = clip.constant_speed().filter(|s| *s > 0.0)?;
    let speed = Rational::from_f64_on_grid(speed, Rational::FLICK_DEN).ok()?;
    let offset = layer.start_offset.0;
    // The stretch of the comp the clip plays, and where its first moment sits.
    let (s0, s1) = (clip.source_in, clip.source_out);
    let mut order: Vec<&Clip> = inner.iter().collect();
    order.sort_by_key(|c| c.place_start);
    let mut out = Vec::new();
    for a in order {
        let (a0, a1) = (
            a.place_start.checked_add(offset).ok()?,
            a.place_end().checked_add(offset).ok()?,
        );
        let (lo, hi) = (s0.max(a0), s1.min(a1));
        if hi <= lo {
            continue;
        }
        let mut piece = a.clone();
        if lo > a0 {
            piece = piece.trim_start(lo.checked_sub(offset).ok()?)?;
        }
        if hi < a1 {
            piece = piece.trim_end(hi.checked_sub(offset).ok()?)?;
        }
        let at = clip
            .place_start
            .checked_add(lo.checked_sub(s0).ok()?.checked_div(speed).ok()?)
            .ok()?;
        let length = hi.checked_sub(lo).ok()?.checked_div(speed).ok()?;
        piece = piece.slide(at.checked_sub(piece.place_start).ok()?)?;
        if speed != Rational::ONE {
            piece = piece.stretched(at, at.checked_add(length).ok()?)?;
        }
        // The angle clip's effects ran on the footage first, on its own clock,
        // then the angle row's on the row's, then the clip's on its own.
        let mut effects = stack(&a.effects, a.fx, lo.checked_sub(a0).ok()?)?;
        effects.extend(stack(
            &layer.effects,
            layer.switches.fx,
            lo.checked_sub(offset).ok()?,
        )?);
        effects.extend(stack(
            &clip.effects,
            clip.fx,
            at.checked_sub(clip.place_start).ok()?,
        )?);
        let (first, last) = (lo == s0, hi == s1);
        let (own_start, own_end) = (lo == a0, hi == a1);
        // The angle clip's fades, timed on the piece's clock.
        let own = |f: Fade| -> Option<Fade> {
            Some(Fade {
                seconds: f.seconds.checked_div(speed).ok()?,
                ..f
            })
        };
        let fade_in = fade(
            first.then_some(clip.fade_in),
            if own_start { own(a.fade_in) } else { None },
        );
        let fade_out = fade(
            last.then_some(clip.fade_out),
            if own_end { own(a.fade_out) } else { None },
        );
        let transition = if first {
            clip.transition.clone()
        } else if own_start {
            a.transition.clone()
        } else {
            None
        };
        piece = Clip {
            id: Uuid::now_v7(),
            angle: None,
            effects,
            fx: true,
            gain_db: clip.gain_db + a.gain_db,
            opacity: clip.opacity * a.opacity,
            fade_in,
            fade_out,
            link: clip.link,
            group: clip.group,
            label: clip.label,
            blend: clip.blend.or(a.blend),
            audible: clip.audible,
            takes: Vec::new(),
            transition,
            ..piece
        };
        out.push(piece);
    }
    Some(out)
}

/// A stack of effects carried onto a flattened piece that starts `from` into
/// the clock its keys are on. A stack whose fx switch is off comes across
/// with each effect switched off, so it neither starts to apply nor is lost.
fn stack(effects: &[EffectInstance], on: bool, from: Rational) -> Option<Vec<EffectInstance>> {
    let mut out = crate::sequence::respawn(effects);
    crate::preset::shift_keys(&mut out, Rational::ZERO.checked_sub(from).ok()?);
    if !on {
        for effect in &mut out {
            effect.enabled = false;
        }
    }
    Some(out)
}

/// The fade at one end of a flattened piece: the multicam clip's, the angle
/// clip's, or where both fade there, the longer.
fn fade(outer: Option<Fade>, own: Option<Fade>) -> Fade {
    match (outer, own) {
        (Some(o), Some(i)) if i.seconds > o.seconds => i,
        (o, i) => o.or(i).unwrap_or_default(),
    }
}

/// How the angle viewer lays out `count` angles on a page: columns and rows.
/// One across for one, two across for two, then the smallest square that
/// holds them, up to four by four.
#[must_use]
pub fn grid(count: usize) -> (usize, usize) {
    match count {
        0 | 1 => (1, 1),
        2 => (2, 1),
        3 | 4 => (2, 2),
        5..=9 => (3, 3),
        _ => (4, 4),
    }
}

/// The most angles one page of the angle viewer holds.
pub const PAGE: usize = 16;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
