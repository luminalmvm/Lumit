//! Audio streams: which of a file's streams a layer sounds, and the two edits
//! that take them apart and put them back.
//!
//! # In plain terms
//!
//! A recording can carry several sounds side by side, a screen capture with
//! the game on one stream and the microphone on another being the usual one.
//! A layer of such a file sounds the first stream until it is told otherwise.
//! **Splitting** gives every stream a row of its own, so each has its own
//! Volume, its own effects and its own mute. **Merging** is the way back: one
//! row sounding the streams mixed, and the rows that were split off gone.
//!
//! This is the half for a Footage layer. A Sequence layer keeps the choice on
//! each clip, and its split and merge are edits of clips and rows, so they
//! live with the other edits of that kind (`crate::api::cut`).
//!
//! Nothing here opens a file: the callers probe and pass in what they found.
//! Runs on the UI thread, like every other document edit.

use lumit_core::model::{AudioStreams, Composition, FootageItem, Layer, LayerKind};
use lumit_core::ops::Op;
use uuid::Uuid;

use crate::api::state::LumitBridgeState;

/// What a file says of one of its audio streams.
pub(crate) struct Stream {
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
    pub title: Option<String>,
    pub language: Option<String>,
}

/// The audio streams of this item's file, in the file's own order. Empty for
/// a file with no sound, a file that will not probe, and a build with no
/// media feature.
pub(crate) fn of_item(state: &LumitBridgeState, footage: &FootageItem) -> Vec<Stream> {
    #[cfg(feature = "media")]
    {
        let probed = crate::api::footage::FootageReference::resolve_source(state, footage)
            .and_then(|src| crate::probe::ensure_probed(&src));
        let Some(info) = probed else {
            return Vec::new();
        };
        info.audio_streams()
            .map(|a| Stream {
                codec: a.codec.clone(),
                channels: u32::try_from(a.channels).unwrap_or(0),
                sample_rate: u32::try_from(a.sample_rate).unwrap_or(0),
                title: a.title.clone(),
                language: a.language.clone(),
            })
            .collect()
    }

    #[cfg(not(feature = "media"))]
    {
        let _ = (state, footage);
        Vec::new()
    }
}

/// Every stream of a file with `count` of them, mixed.
pub(crate) fn all(count: usize) -> AudioStreams {
    AudioStreams::of(&(0..count as u32).collect::<Vec<_>>())
}

/// The streams a split gives rows of their own: the ones `chosen` sounds
/// where it sounds several, and every stream the file has where it sounds the
/// first as a layer does until told otherwise. Fewer than two is nothing to
/// split.
pub(crate) fn to_split(chosen: &AudioStreams, count: usize) -> Vec<u32> {
    let there = |place: &u32| (*place as usize) < count;
    let sounded: Vec<u32> = chosen.places().iter().copied().filter(there).collect();
    if sounded.len() < 2 && chosen.is_first() {
        return (0..count as u32).collect();
    }
    sounded
}

/// What a row split off for the stream at `place` is called: the row it came
/// off, and the stream's own name where the file gives it one, else its
/// number as the interface counts them. No word of ours is in it, so there is
/// nothing to translate.
pub(crate) fn split_name(base: &str, place: u32, streams: &[Stream]) -> String {
    let title = streams.get(place as usize).and_then(|s| s.title.as_deref());
    match title {
        Some(title) => format!("{base} \u{b7} {title}"),
        None => format!("{base} \u{b7} {}", place + 1),
    }
}

/// **Split audio streams**, for a Footage layer: the layer is left sounding
/// the first of its streams and each of the others goes on an Audio layer of
/// its own directly below, a copy of this one as *Detach audio* makes, so
/// every row starts out with the span, Volume and effects the one row had.
///
/// A stream an Audio layer beside this one already sounds is left where it
/// is, so a layer that has been split has nothing more to give: the first
/// stream alone is what a layer sounds before it is split and after.
///
/// Asked of a layer that is not heard while an Audio layer beside it is, as
/// *Detach audio* leaves a picture and its sound, it is the one that is heard
/// that is split.
///
/// The ops, and the ids of the layers they add. `None` when there is nothing
/// to split.
pub(crate) fn split_layer(
    comp: &Composition,
    layer: &Layer,
    streams: &[Stream],
) -> Option<(Vec<Op>, Vec<Uuid>)> {
    if !matches!(layer.kind, LayerKind::Footage { .. }) {
        return None;
    }
    let split_off =
        |other: &&Layer| other.id != layer.id && other.audio_only && beside(layer, other);
    let heard = || {
        let mut beside = comp.layers.iter().filter(split_off);
        beside.find(|other| other.switches.audible)
    };
    let layer = match layer.switches.audible {
        true => layer,
        false => heard().unwrap_or(layer),
    };
    let apart: Vec<u32> = comp
        .layers
        .iter()
        .filter(|other| other.id != layer.id && other.audio_only && beside(layer, other))
        .flat_map(|other| other.audio_streams.places().to_vec())
        .collect();
    // The layer keeps the first of its streams whatever is beside it.
    let places = to_split(&layer.audio_streams, streams.len());
    let (first, rest) = places.split_first()?;
    let rest: Vec<u32> = rest
        .iter()
        .copied()
        .filter(|p| !apart.contains(p))
        .collect();
    if rest.is_empty() {
        return None;
    }
    let index = comp.layers.iter().position(|l| l.id == layer.id)?;
    // The layer's own change goes first, so the history names the edit after
    // it and a locked layer refuses the lot.
    let mut ops = vec![Op::SetLayerAudioStreams {
        comp: comp.id,
        layer: layer.id,
        streams: AudioStreams::only(*first),
    }];
    let mut made = Vec::with_capacity(rest.len());
    for (n, place) in rest.iter().enumerate() {
        // Effect ids are kept, as a detached layer's are: the layer's own
        // driver graph finds its effects by them.
        let mut sound = layer.clone();
        sound.id = Uuid::now_v7();
        sound.name = split_name(&layer.name, *place, streams);
        sound.audio_only = true;
        sound.audio_streams = AudioStreams::only(*place);
        made.push(sound.id);
        ops.push(Op::AddLayer {
            comp: comp.id,
            index: index + 1 + n,
            layer: Box::new(sound),
        });
    }
    Some((ops, made))
}

/// Whether two Footage layers are the same file started at the same moment:
/// what a split leaves side by side, and what a merge can fold together
/// without moving any sound in time.
fn beside(layer: &Layer, other: &Layer) -> bool {
    match (&layer.kind, &other.kind) {
        (LayerKind::Footage { item }, LayerKind::Footage { item: theirs }) => {
            item == theirs && layer.start_offset == other.start_offset
        }
        _ => false,
    }
}

/// **Merge audio streams**, for a Footage layer: one row sounds the streams
/// mixed and the Audio layers split off beside it go.
///
/// The rows that belong together are the layers of the same file that start
/// it at the same moment ([`beside`]). The one kept is the layer asked,
/// unless that is an Audio layer and a layer with the picture is sounding
/// beside it, which is then the one to keep. A layer that is not heard hands
/// over to one beside it that is, so a merge never ends on a silent row. The
/// one kept takes the streams of every Audio layer folded into it. A locked
/// one stays as it is.
///
/// With nothing beside it to fold, the layer sounds every stream its file
/// has: merging a layer that was never split.
///
/// `None` when that changes nothing.
pub(crate) fn merge_layer(comp: &Composition, layer: &Layer, count: usize) -> Option<Vec<Op>> {
    if !matches!(layer.kind, LayerKind::Footage { .. }) {
        return None;
    }
    let beside = |other: &&Layer| beside(layer, other);
    let pictured = || {
        comp.layers
            .iter()
            .filter(beside)
            .find(|l| !l.audio_only && l.switches.audible)
    };
    // An Audio layer beside this one that is heard, for a layer that is not.
    let sounding = || {
        let mut heard = comp.layers.iter().filter(beside);
        heard.find(|l| l.audio_only && l.switches.audible)
    };
    let kept = match (layer.audio_only, layer.switches.audible) {
        (false, true) => layer,
        (true, true) => pictured().unwrap_or(layer),
        (_, false) => pictured().or_else(sounding).unwrap_or(layer),
    };
    let folded: Vec<&Layer> = comp
        .layers
        .iter()
        .filter(beside)
        .filter(|l| l.id != kept.id && l.audio_only && !l.switches.locked)
        .collect();

    let mut places = kept.audio_streams.places().to_vec();
    for gone in &folded {
        places.extend_from_slice(gone.audio_streams.places());
    }
    let mut merged = AudioStreams::of(&places);
    if folded.is_empty() {
        merged = all(count);
    }
    if folded.is_empty() && merged == kept.audio_streams {
        return None;
    }

    let mut ops = Vec::with_capacity(folded.len() + 1);
    if merged != kept.audio_streams {
        ops.push(Op::SetLayerAudioStreams {
            comp: comp.id,
            layer: kept.id,
            streams: merged,
        });
    }
    ops.extend(folded.iter().map(|gone| Op::RemoveLayer {
        comp: comp.id,
        layer: gone.id,
    }));
    Some(ops)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::model::{Document, ProjectItem};
    use lumit_core::time::Rational;

    fn stream(title: Option<&str>) -> Stream {
        Stream {
            codec: "aac".into(),
            channels: 2,
            sample_rate: 48_000,
            title: title.map(str::to_string),
            language: None,
        }
    }

    /// A comp holding one footage layer of a file with three audio streams.
    fn recording() -> (Document, Uuid, Uuid) {
        let mut doc = Document::new();
        let item = Uuid::now_v7();
        let layer = crate::edits::base_layer(
            "take.mkv".into(),
            LayerKind::Footage { item },
            Rational::new(4, 1).unwrap(),
            Default::default(),
        );
        let layer_id = layer.id;
        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "cut".into(),
            width: 64,
            height: 64,
            frame_rate: lumit_core::time::FrameRate::FPS_25,
            duration: lumit_core::time::Duration(Rational::new(10, 1).unwrap()),
            background: lumit_core::model::LinearColour::BLACK,
            work_area: None,
            layers: vec![layer],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let comp_id = comp.id;
        doc.items.push(ProjectItem::Composition(comp));
        (doc, comp_id, layer_id)
    }

    fn apply(doc: &mut Document, ops: Vec<Op>) {
        lumit_core::ops::apply(doc, &Op::Batch { ops }).unwrap();
    }

    /// **A split and a merge are each other's way back.** Three streams become
    /// three rows, one stream each and none twice, named for what the file
    /// calls them; merging from any of the rows leaves one row sounding all
    /// three, and it is the row with the picture.
    #[test]
    fn splitting_gives_each_stream_a_row_and_merging_folds_them_back() {
        let streams = [stream(None), stream(Some("Microphone")), stream(None)];
        let (mut doc, comp_id, layer_id) = recording();
        let comp = doc.comp(comp_id).unwrap().clone();

        let (ops, made) = split_layer(&comp, &comp.layers[0], &streams).unwrap();
        apply(&mut doc, ops);
        let comp = doc.comp(comp_id).unwrap().clone();
        let rows: Vec<_> = comp
            .layers
            .iter()
            .map(|l| {
                (
                    l.name.as_str(),
                    l.audio_only,
                    l.audio_streams.places().to_vec(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("take.mkv", false, vec![0]),
                ("take.mkv \u{b7} Microphone", true, vec![1]),
                ("take.mkv \u{b7} 3", true, vec![2]),
            ]
        );
        assert_eq!(made, [comp.layers[1].id, comp.layers[2].id]);
        // One stream to a row is nothing more to split.
        assert!(split_layer(&comp, &comp.layers[1], &streams).is_none());

        // Asked of a row that was split off, the picture's row is the one kept.
        let ops = merge_layer(&comp, &comp.layers[2], streams.len()).unwrap();
        apply(&mut doc, ops);
        let comp = doc.comp(comp_id).unwrap().clone();
        assert_eq!(comp.layers.len(), 1);
        assert_eq!(comp.layers[0].id, layer_id);
        assert_eq!(comp.layers[0].audio_streams.places(), [0, 1, 2]);
        assert!(merge_layer(&comp, &comp.layers[0], streams.len()).is_none());

        // And a row sounding all three splits into the same three again.
        let (ops, _) = split_layer(&comp, &comp.layers[0], &streams).unwrap();
        apply(&mut doc, ops);
        assert_eq!(doc.comp(comp_id).unwrap().layers.len(), 3);
    }

    /// After *Detach audio* the picture's row is silent and its sound is on
    /// the Audio layer below. A split asked of either is a split of the one
    /// that is heard, and neither changes which stream it had.
    #[test]
    fn a_detached_picture_is_split_through_its_sound() {
        let streams = [stream(None), stream(None), stream(None)];
        let (mut doc, comp_id, picture) = recording();
        {
            let comp = doc.comp_mut(comp_id).unwrap();
            let mut sound = comp.layers[0].clone();
            sound.id = Uuid::now_v7();
            sound.audio_only = true;
            comp.layers[0].switches.audible = false;
            comp.layers.push(sound);
        }
        let comp = doc.comp(comp_id).unwrap().clone();
        let (ops, _) = split_layer(&comp, &comp.layers[0], &streams).unwrap();
        apply(&mut doc, ops);
        let rows: Vec<_> = doc
            .comp(comp_id)
            .unwrap()
            .layers
            .iter()
            .map(|l| {
                (
                    l.id == picture,
                    l.switches.audible,
                    l.audio_streams.places().to_vec(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (true, false, vec![0]),
                (false, true, vec![0]),
                (false, true, vec![1]),
                (false, true, vec![2]),
            ]
        );

        // And merged through it: the row that is heard takes them back.
        let comp = doc.comp(comp_id).unwrap().clone();
        let ops = merge_layer(&comp, &comp.layers[0], streams.len()).unwrap();
        apply(&mut doc, ops);
        let layers = &doc.comp(comp_id).unwrap().layers;
        assert_eq!(layers.len(), 2);
        assert_eq!((layers[0].id, layers[0].switches.audible), (picture, false));
        assert_eq!(layers[1].audio_streams.places(), [0, 1, 2]);
    }

    /// A layer that was never split merges to every stream of its file, and a
    /// layer of a file with one stream has nothing to split or merge.
    #[test]
    fn a_layer_alone_merges_to_every_stream_of_its_file() {
        let (mut doc, comp_id, _) = recording();
        let comp = doc.comp(comp_id).unwrap().clone();
        assert!(merge_layer(&comp, &comp.layers[0], 1).is_none());
        assert!(split_layer(&comp, &comp.layers[0], &[stream(None)]).is_none());

        let ops = merge_layer(&comp, &comp.layers[0], 2).unwrap();
        apply(&mut doc, ops);
        let layers = &doc.comp(comp_id).unwrap().layers;
        assert_eq!(layers[0].audio_streams.places(), [0, 1]);
    }
}
