//! Audio streams across the bridge: what a file's streams are, which of them
//! a layer or a clip sounds, and the calls that choose, split and merge them
//! (docs/01-GLOSSARY.md: Audio stream).
//!
//! The edits themselves are in `crate::streams` for a Footage layer and in
//! [`crate::api::cut`] for the clips of a Sequence layer. This file is the
//! shapes that cross and the calls that reach them.
//!
//! Every call here runs on the UI thread and is `sync`, as a cut's are. What
//! a file's streams are is read from the probe the Project panel made of it
//! when it was imported (`crate::probe`), which every door into a composition
//! already reads the same way, so nothing here opens a file that has been
//! seen.

use flutter_rust_bridge::frb;
use lumit_core::model::{AudioStreams, LayerKind, ProjectItem};
use lumit_core::sequence::ClipSource;
use lumit_core::Op;
use uuid::Uuid;

use crate::api::{
    composition::CompositionReference, cut::BridgeCutResult, footage::FootageReference,
    layer::LayerReference, BridgeError,
};

/// One audio stream of a file, as the file describes it.
///
/// The codec, the name and the language are the file's own words and cross
/// untranslated. The interface numbers the streams itself, from one.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAudioStream {
    /// The stream's place among the file's audio streams, counted from
    /// nought: what the calls here name it by.
    pub place: u32,
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
    /// What the file calls the stream, where it calls it anything.
    pub title: Option<String>,
    /// The stream's language as the file writes it, where it says one.
    pub language: Option<String>,
}

/// The audio streams a layer or a clip has to choose from, and the ones it
/// sounds.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAudioStreams {
    /// The streams of the one file sounded. Empty for a layer that sounds no
    /// file, and for a Sequence layer whose clips are of several files, which
    /// has no one list to show.
    pub streams: Vec<BridgeAudioStream>,
    /// The places sounded, of those in `streams`.
    pub chosen: Vec<u32>,
    /// Whether any file sounded has more than one stream, which is when
    /// there is anything to split or merge.
    pub several: bool,
}

/// How a file with several audio streams arrives in a composition:
/// Settings ▸ Interface ▸ Editing, forwarded by the frontend as *Video arrives
/// as a Sequence layer* is.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeStreamsArrival {
    /// The first stream alone, as a file with one stream arrives.
    First,
    /// Every stream mixed, on the one layer.
    Mixed,
    /// A layer to each stream.
    Split,
}

impl BridgeStreamsArrival {
    /// What each sounding layer or clip of a file with `count` streams
    /// plays, the first of them the one that arrives with the picture.
    /// Always at least one.
    #[frb(ignore)]
    pub(crate) fn sounds(self, count: usize) -> Vec<AudioStreams> {
        match self {
            Self::Split if count > 1 => (0..count as u32).map(AudioStreams::only).collect(),
            Self::Mixed if count > 1 => vec![crate::streams::all(count)],
            _ => vec![AudioStreams::default()],
        }
    }
}

/// What a layer or clip that sounds no one file answers: no list, and
/// `several` as given.
fn no_list(several: bool) -> BridgeAudioStreams {
    BridgeAudioStreams {
        streams: Vec::new(),
        chosen: Vec::new(),
        several,
    }
}

/// The crossing shape of a file's streams.
fn described(streams: &[crate::streams::Stream]) -> Vec<BridgeAudioStream> {
    streams
        .iter()
        .enumerate()
        .map(|(place, s)| BridgeAudioStream {
            place: place as u32,
            codec: s.codec.clone(),
            channels: s.channels,
            sample_rate: s.sample_rate,
            title: s.title.clone(),
            language: s.language.clone(),
        })
        .collect()
}

/// `chosen` as places of a file with `count` streams: the ones it has.
fn places_of(chosen: &AudioStreams, count: usize) -> Vec<u32> {
    let there = |place: &u32| (*place as usize) < count;
    chosen.places().iter().copied().filter(there).collect()
}

impl FootageReference {
    /// Every audio stream this item's file carries, in the file's own order.
    /// Empty for a file with no sound and for one that will not open.
    #[frb(sync)]
    pub fn audio_streams(&self) -> Result<Vec<BridgeAudioStream>, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();
        let Some(ProjectItem::Footage(footage)) = snapshot.item(self.id()) else {
            return Err(BridgeError::InvalidItem);
        };
        Ok(described(&crate::streams::of_item(&proj, footage)))
    }
}

impl LayerReference {
    /// The audio streams this layer has to choose from and the ones it
    /// sounds.
    ///
    /// A Footage layer answers for its file. A Sequence layer keeps the choice
    /// on each clip, so it answers for its clips together: the file's streams
    /// and every place any clip sounds where the clips are all of one file,
    /// and no list where they are of several. Any other kind sounds no file.
    #[frb(sync)]
    pub fn audio_streams(&self) -> Result<BridgeAudioStreams, BridgeError> {
        let layer = self.item()?;
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();
        let streams_of = |item: Uuid| match snapshot.item(item) {
            Some(ProjectItem::Footage(f)) => crate::streams::of_item(&proj, f),
            _ => Vec::new(),
        };
        match &layer.kind {
            LayerKind::Footage { item } => {
                let streams = streams_of(*item);
                Ok(BridgeAudioStreams {
                    chosen: places_of(&layer.audio_streams, streams.len()),
                    several: streams.len() > 1,
                    streams: described(&streams),
                })
            }
            LayerKind::Sequence { clips } => {
                let mut items: Vec<Uuid> = Vec::new();
                for clip in clips {
                    if let ClipSource::Footage(item) = clip.source {
                        if !items.contains(&item) {
                            items.push(item);
                        }
                    }
                }
                let [only] = items.as_slice() else {
                    let several = items.iter().any(|item| streams_of(*item).len() > 1);
                    return Ok(no_list(several));
                };
                let streams = streams_of(*only);
                let mut places: Vec<u32> = Vec::new();
                for clip in clips {
                    places.extend(places_of(&clip.audio_streams, streams.len()));
                }
                places.sort_unstable();
                places.dedup();
                Ok(BridgeAudioStreams {
                    chosen: places,
                    several: streams.len() > 1,
                    streams: described(&streams),
                })
            }
            _ => Ok(no_list(false)),
        }
    }

    /// Choose which audio streams this layer sounds, mixed. One undo step.
    ///
    /// On a Sequence layer every footage clip takes the choice. An empty
    /// list is the first stream, which is what a layer sounds until it is
    /// told otherwise. [`BridgeError::NotFootage`] on a layer that sounds no
    /// file of its own.
    #[frb(sync)]
    pub fn set_audio_streams(&self, streams: Vec<u32>) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let streams = AudioStreams::of(&streams);
        match layer.kind {
            LayerKind::Footage { .. } => self.commit(Op::SetLayerAudioStreams {
                comp: self.comp_id,
                layer: self.layer_id,
                streams,
            }),
            LayerKind::Sequence { mut clips } => {
                for clip in &mut clips {
                    if matches!(clip.source, ClipSource::Footage(_)) {
                        clip.audio_streams = streams.clone();
                    }
                }
                self.commit(Op::SetSequenceClips {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    clips,
                })
            }
            _ => Err(BridgeError::NotFootage),
        }
    }

    /// The audio streams one clip of this Sequence layer has to choose from
    /// and the ones it sounds. Nothing for a clip of a composition.
    #[frb(sync)]
    pub fn clip_audio_streams(&self, clip: Uuid) -> Result<BridgeAudioStreams, BridgeError> {
        let layer = self.item()?;
        let LayerKind::Sequence { clips } = &layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let clip = clips
            .iter()
            .find(|c| c.id == clip)
            .ok_or(BridgeError::InvalidLayer)?;
        let ClipSource::Footage(item) = clip.source else {
            return Ok(no_list(false));
        };
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        let snapshot = proj.store.snapshot();
        let Some(ProjectItem::Footage(footage)) = snapshot.item(item) else {
            return Ok(no_list(false));
        };
        let streams = crate::streams::of_item(&proj, footage);
        Ok(BridgeAudioStreams {
            chosen: places_of(&clip.audio_streams, streams.len()),
            several: streams.len() > 1,
            streams: described(&streams),
        })
    }

    /// Choose which audio streams one clip of this Sequence layer sounds,
    /// mixed. One undo step.
    #[frb(sync)]
    pub fn set_clip_audio_streams(&self, clip: Uuid, streams: Vec<u32>) -> Result<(), BridgeError> {
        let layer = self.item()?;
        let LayerKind::Sequence { mut clips } = layer.kind else {
            return Err(BridgeError::NotSequence);
        };
        let found = clips
            .iter_mut()
            .find(|c| c.id == clip)
            .ok_or(BridgeError::InvalidLayer)?;
        found.audio_streams = AudioStreams::of(&streams);
        self.commit(Op::SetSequenceClips {
            comp: self.comp_id,
            layer: self.layer_id,
            clips,
        })
    }

    /// **Split audio streams**: give every audio stream this layer sounds a
    /// row of its own.
    ///
    /// **In plain terms.** A recording with the game on one stream and the
    /// microphone on another arrives as one row that sounds the first. Split,
    /// it is a row to a stream: this layer keeps the picture and the first
    /// stream, and each of the others is an Audio layer directly below with
    /// the same span, Volume and effects, named for what the file calls the
    /// stream. Each can then be turned down, cut or muted by itself.
    ///
    /// A layer that sounds several streams mixed is split into those. A
    /// Sequence layer is split clip by clip, each stream a linked clip on a
    /// sound row ([`CompositionReference::cut_split_audio_streams`]).
    ///
    /// One undo step. `false` when the layer sounds one stream and there is
    /// nothing to split.
    #[frb(sync)]
    pub fn split_audio_streams(&self) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        if let LayerKind::Sequence { clips } = &layer.kind {
            let clips = clips.iter().map(|c| c.id).collect();
            let comp = CompositionReference::new(self.project_id, self.comp_id);
            return Ok(comp.cut_split_audio_streams(clips)? == BridgeCutResult::Done);
        }
        let LayerKind::Footage { item } = layer.kind else {
            return Ok(false);
        };
        let ops = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let snapshot = proj.store.snapshot();
            let Some(ProjectItem::Footage(footage)) = snapshot.item(item) else {
                return Ok(false);
            };
            let streams = crate::streams::of_item(&proj, footage);
            let comp = snapshot
                .comp(self.comp_id)
                .ok_or(BridgeError::InvalidComp)?;
            crate::streams::split_layer(comp, &layer, &streams)
        };
        let Some((ops, _made)) = ops else {
            return Ok(false);
        };
        self.commit(Op::Batch { ops })?;
        Ok(true)
    }

    /// **Merge audio streams**, the way back from a split: this row sounds
    /// the streams mixed and the rows split off beside it go.
    ///
    /// The rows folded are the Audio layers of the same file that start it at
    /// the same moment, so no sound moves in time. What each carried of its
    /// own, a Volume ride or an effect, goes with it: the row kept is heard
    /// through its own. Asked of a split-off row while the layer with the
    /// picture is sounding beside it, that layer is the one kept. A layer with
    /// nothing split off beside it is set to every stream its file has.
    ///
    /// A Sequence layer is merged clip by clip
    /// ([`CompositionReference::cut_merge_audio_streams`]).
    ///
    /// One undo step. `false` when it changes nothing.
    #[frb(sync)]
    pub fn merge_audio_streams(&self) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        if let LayerKind::Sequence { clips } = &layer.kind {
            let clips = clips.iter().map(|c| c.id).collect();
            let comp = CompositionReference::new(self.project_id, self.comp_id);
            return Ok(comp.cut_merge_audio_streams(clips)? == BridgeCutResult::Done);
        }
        let LayerKind::Footage { item } = layer.kind else {
            return Ok(false);
        };
        let ops = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            let snapshot = proj.store.snapshot();
            let Some(ProjectItem::Footage(footage)) = snapshot.item(item) else {
                return Ok(false);
            };
            let count = crate::streams::of_item(&proj, footage).len();
            let comp = snapshot
                .comp(self.comp_id)
                .ok_or(BridgeError::InvalidComp)?;
            crate::streams::merge_layer(comp, &layer, count)
        };
        let Some(ops) = ops else {
            return Ok(false);
        };
        self.commit(Op::Batch { ops })?;
        Ok(true)
    }
}
