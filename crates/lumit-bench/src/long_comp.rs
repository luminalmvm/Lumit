//! docs/13 §1's long-form composition, built in code.
//!
//! # In plain terms
//!
//! The reference comp ([`crate::comp`]) is twenty seconds of heavy picture. A
//! cut is the other shape of work: two hours, light picture, and two thousand
//! clips drawn from three hundred files. Nothing in it is dear to draw. What
//! it stresses is everything the engine does once per clip or once per file,
//! which twenty seconds and three files cannot show.
//!
//! | §1 says | built by |
//! |---|---|
//! | 1080p60, 2 hours | [`build`]'s `Composition` |
//! | 3 picture Sequence layers | `V1`, `V2`, `V3` |
//! | 4 audio-only Sequence layers | `A1` to `A4` |
//! | 2,000 clips from 300 footage items | [`ROWS`] |
//! | linked picture and sound on the main rows | `V1` with `A1`, `V2` with `A2` |
//! | short and long clips, gaps | [`fill`] |
//! | a passage of fast cuts | the montage, [`MONTAGE_START`] |
//! | overlapping sound clips | `A3` and `A4` |
//!
//! Every length and place comes from one seeded generator and every id is
//! hashed from a name, so the same comp is built on every run and every
//! machine. Nothing here reads a clock.

use std::path::Path;

use lumit_core::anim::Property;
use lumit_core::model::{Composition, Document, LayerKind, LinearColour, MotionBlur, ProjectItem};
use lumit_core::sequence::{clips_span, Clip, ClipSource};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use uuid::Uuid;

use crate::comp::{add_footage, id, layer, rat};
use crate::media::{self, LongMedia, LONG_FILE_S, LONG_ITEMS};

/// Frames a second, the reference comp's own rate.
pub const FPS: i64 = 60;
/// The comp's length in frames: two hours.
pub const FRAMES: i64 = 2 * 60 * 60 * FPS;
/// How many clips the seven rows hold between them.
pub const CLIP_COUNT: usize = 2000;
/// The main picture row, and the sound row its clips are linked to.
pub const MAIN_PICTURE: &str = "V1";
pub const MAIN_SOUND: &str = "A1";

/// Where the montage starts, in frames: a minute in.
pub const MONTAGE_START: i64 = 60 * FPS;
/// How long the montage runs, in frames. Inside it `V1` cuts every
/// [`MONTAGE_CUT`] frames, `V2` every other cut and `V3` every fourth, each
/// clip from a different file, which is the passage the playback scenario
/// plays across.
pub const MONTAGE_FRAMES: i64 = 8 * FPS;
/// Frames between `V1`'s cuts inside the montage.
pub const MONTAGE_CUT: i64 = 12;

/// The longest a clip can be: the whole of its file.
const FILE_FRAMES: i64 = LONG_FILE_S * FPS;
/// The shortest clip the generator makes.
const MIN_CLIP: i64 = 30;

/// One row of the cut: its name, how many clips it holds outside the montage,
/// which footage items it draws on, and how much of its length they cover.
struct Row {
    name: &'static str,
    clips: usize,
    items: std::ops::Range<usize>,
    /// Per cent of the row covered by clips.
    cover: i64,
    /// Per cent of joins with no gap, so the two clips share an edit point.
    abut: u64,
    /// Montage clips are this many of `V1`'s cuts long.
    montage_every: i64,
    opacity: f64,
    /// The sound row carrying this row's linked sound, when it has one.
    sound: Option<&'static str>,
}

/// The picture rows. With the montage's clips they come to 950, and their
/// linked sound and the two free sound rows bring the comp to [`CLIP_COUNT`].
const ROWS: [Row; 3] = [
    Row {
        name: MAIN_PICTURE,
        clips: 560,
        items: 0..200,
        cover: 97,
        abut: 70,
        montage_every: 1,
        opacity: 100.0,
        sound: Some(MAIN_SOUND),
    },
    // Not fully opaque, either of them. A full-frame row over a full-frame row
    // hides it, the renderer rightly skips what cannot be seen, and the stack
    // would then cost one row where a multicam cut costs three.
    Row {
        name: "V2",
        clips: 180,
        items: 200..260,
        cover: 33,
        abut: 30,
        montage_every: 2,
        opacity: 80.0,
        sound: Some("A2"),
    },
    Row {
        name: "V3",
        clips: 140,
        items: 260..280,
        cover: 22,
        abut: 30,
        montage_every: 4,
        opacity: 60.0,
        sound: None,
    },
];

/// The two sound rows with no picture: how many clips, and the shortest and
/// longest of them in frames. Both draw on the last twenty items.
const FREE_SOUND: [(&str, usize, i64, i64); 2] =
    [("A3", 130, 1200, FILE_FRAMES), ("A4", 120, MIN_CLIP, 300)];

/// Generate the media into `media_dir` and build the long-form comp over it.
/// Returns the document and the id of the comp to render.
pub fn long_comp(media_dir: &Path) -> Result<(Document, Uuid), String> {
    build(&media::generate_long(media_dir)?)
}

/// [`long_comp`] over media already generated.
pub fn build(media: &LongMedia) -> Result<(Document, Uuid), String> {
    if media.files.len() != LONG_ITEMS {
        return Err(format!(
            "the long comp wants {LONG_ITEMS} files and was given {}",
            media.files.len()
        ));
    }
    let mut doc = Document::new();
    let items: Vec<Uuid> = media
        .files
        .iter()
        .enumerate()
        .map(|(i, path)| add_footage(&mut doc, &format!("long_{i:03}.mp4"), path))
        .collect();
    let mut rng = Rng(0x4c75_6d69_7443_7574);

    let mut picture = Vec::new();
    let mut sound = Vec::new();
    for row in &ROWS {
        let before = row.clips * MONTAGE_START as usize / FRAMES as usize;
        let mut pieces = fill(&mut rng, 0, MONTAGE_START, before.max(1), row);
        let cut = MONTAGE_CUT * row.montage_every;
        pieces.extend((0..MONTAGE_FRAMES / cut).map(|i| (MONTAGE_START + i * cut, cut)));
        let after_start = MONTAGE_START + MONTAGE_FRAMES;
        pieces.extend(fill(
            &mut rng,
            after_start,
            FRAMES - after_start,
            row.clips - before.max(1),
            row,
        ));
        let clips: Vec<Clip> = pieces
            .iter()
            .enumerate()
            .map(|(k, &(start, len))| {
                let item = items[row.items.start + k % row.items.len()];
                clip(&mut rng, row.name, k, item, start, len)
            })
            .collect();
        // The first two rows carry their sound on a row of its own: the same
        // clip again, in the same place, sharing a link.
        if let Some(name) = row.sound {
            let mut linked = clips.clone();
            let mut paired = clips;
            for (k, (p, s)) in paired.iter_mut().zip(&mut linked).enumerate() {
                let link = id(&format!("long/link/{name}/{k}"));
                p.link = Some(link);
                s.link = Some(link);
                s.id = id(&format!("long/{name}/{k}"));
            }
            sound.push(sequence_layer(name, linked, true, 100.0));
            picture.push(sequence_layer(row.name, paired, false, row.opacity));
        } else {
            picture.push(sequence_layer(row.name, clips, false, row.opacity));
        }
    }
    for (name, count, shortest, longest) in FREE_SOUND {
        let clips = overlapping(&mut rng, name, count, shortest, longest, &items[280..]);
        sound.push(sequence_layer(name, clips, true, 100.0));
    }

    // Index 0 is the top of the stack: the last picture row, down to the main
    // one, then the sound.
    picture.reverse();
    picture.extend(sound);
    let comp_id = id("Long cut");
    doc.items
        .push(composition(comp_id, "Long cut", FRAMES, picture)?);
    Ok((doc, comp_id))
}

/// A one-second comp of three picture rows over the three encoded files, under
/// item ids the long comp does not use.
///
/// The scenarios render one frame of it first. That compiles the shaders and
/// loads the decoder libraries, which a running editor has long since done,
/// and opens no file the long comp names. What is timed afterwards is then the
/// long comp's own cost and nothing else.
pub fn warm_up(media: &LongMedia) -> Result<(Document, Uuid), String> {
    let mut doc = Document::new();
    let mut layers = Vec::new();
    for (row, base) in ROWS.iter().zip(&media.bases) {
        let name = format!("warm/{}", row.name);
        let item = add_footage(&mut doc, &name, base);
        let mut one = Clip::new(
            ClipSource::Footage(item),
            Rational::ZERO,
            rat(1, 1),
            Rational::ZERO,
            rat(1, 1),
        );
        one.id = id(&format!("{name}/clip"));
        layers.push(sequence_layer(&name, vec![one], false, row.opacity));
    }
    layers.reverse();
    let comp_id = id("Long cut warm-up");
    doc.items
        .push(composition(comp_id, "Long cut warm-up", FPS, layers)?);
    Ok((doc, comp_id))
}

/// A 1080p60 comp of `frames` frames holding `layers`.
fn composition(
    comp_id: Uuid,
    name: &str,
    frames: i64,
    layers: Vec<lumit_core::model::Layer>,
) -> Result<ProjectItem, String> {
    Ok(ProjectItem::Composition(Composition {
        graph: None,
        master_volume_db: 0.0,
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: comp_id,
        name: name.into(),
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(FPS as u32, 1).map_err(|e| format!("long comp rate: {e}"))?,
        duration: Duration(rat(frames, FPS)),
        background: LinearColour::BLACK,
        work_area: None,
        motion_blur: MotionBlur::default(),
        layers,
        markers: Vec::new(),
        extra: serde_json::Map::new(),
    }))
}

/// A Sequence layer holding `clips`, its bar running from the first clip's
/// start to the last clip's end as the Cut workspace keeps it.
fn sequence_layer(
    name: &str,
    clips: Vec<Clip>,
    audio_only: bool,
    opacity: f64,
) -> lumit_core::model::Layer {
    let (start, end) = clips_span(&clips).unwrap_or((Rational::ZERO, rat(FRAMES, FPS)));
    let mut l = layer(name, LayerKind::Sequence { clips }, end);
    l.in_point = CompTime(start);
    l.audio_only = audio_only;
    // A picture row's sound is the linked clip on a sound row, so the picture
    // row itself is silent and nothing is heard twice.
    l.switches.audible = audio_only;
    l.transform.opacity = Property::fixed(opacity);
    l
}

/// One clip of `item`, `len` frames long at `start`, reading its file from a
/// seeded place that leaves room for all of it.
fn clip(rng: &mut Rng, row: &str, k: usize, item: Uuid, start: i64, len: i64) -> Clip {
    let source_in = rng.range(0, FILE_FRAMES - len);
    let mut c = Clip::new(
        ClipSource::Footage(item),
        rat(source_in, FPS),
        rat(source_in + len, FPS),
        rat(start, FPS),
        rat(len, FPS),
    );
    // Hashed from its row and place in it. `Clip::new` takes an id from the
    // clock, and a clip's id feeds the name its frames are cached under.
    c.id = id(&format!("long/{row}/{k}"));
    c
}

/// Lay `n` clips over `len` frames from `start`: a mix of short and long ones
/// covering `row.cover` per cent of the span, the rest left as gaps between
/// them. Answers each clip's start and length in frames, in order, never
/// overlapping.
fn fill(rng: &mut Rng, start: i64, len: i64, n: usize, row: &Row) -> Vec<(i64, i64)> {
    let raw: Vec<i64> = (0..n)
        .map(|_| {
            if rng.chance(40) {
                rng.range(90, 300)
            } else {
                rng.range(600, FILE_FRAMES)
            }
        })
        .collect();
    let (sum, target) = (raw.iter().sum::<i64>().max(1), len * row.cover / 100);
    // Scaled down to fit and never up: a clip is no longer than its file.
    let lens: Vec<i64> = raw
        .iter()
        .map(|r| (r * target.min(sum) / sum).max(MIN_CLIP))
        .collect();
    let slack = (len - lens.iter().sum::<i64>()).max(0);
    let weights: Vec<i64> = (0..=n)
        .map(|_| {
            if rng.chance(row.abut) {
                0
            } else {
                rng.range(1, 100)
            }
        })
        .collect();
    let total = weights.iter().sum::<i64>().max(1);
    let mut cursor = start;
    let mut out = Vec::with_capacity(n);
    for (i, clip_len) in lens.into_iter().enumerate() {
        cursor += slack * weights[i] / total;
        out.push((cursor, clip_len));
        cursor += clip_len;
    }
    out
}

/// A sound row with no picture: `n` clips of `shortest..=longest` frames, some
/// starting before the one ahead of them has ended, which is a crossfade, and
/// the rest after a gap.
fn overlapping(
    rng: &mut Rng,
    row: &str,
    n: usize,
    shortest: i64,
    longest: i64,
    items: &[Uuid],
) -> Vec<Clip> {
    // What is left of the comp once every clip is laid end to end, shared out
    // among the joins that are gaps.
    let lens: Vec<i64> = (0..n).map(|_| rng.range(shortest, longest)).collect();
    let spare = (FRAMES - lens.iter().sum::<i64>()).max(0) / n as i64;
    let mut cursor = 0;
    let mut out = Vec::with_capacity(n);
    for (k, len) in lens.into_iter().enumerate() {
        out.push(clip(rng, row, k, items[k % items.len()], cursor, len));
        cursor += len;
        if rng.chance(45) {
            // Back over the tail of this clip, by less than the shortest one.
            cursor -= rng.range(10, MIN_CLIP - 5);
        } else {
            cursor += rng.range(spare / 2, spare);
        }
    }
    out
}

/// SplitMix64. Seeded, so the comp is the same comp on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A whole number from `lo` to `hi`, both included.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (hi - lo).max(0) as u64 + 1;
        lo + (self.next() % span) as i64
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use lumit_core::model::Layer;

    /// The comp over paths that need not exist: nothing here opens a file.
    fn built() -> (Document, Uuid) {
        let files = (0..LONG_ITEMS)
            .map(|i| format!("long_{i:03}.mp4").into())
            .collect();
        let media = LongMedia {
            files,
            bases: Vec::new(),
        };
        build(&media).unwrap()
    }

    fn clips(l: &Layer) -> &[Clip] {
        match &l.kind {
            LayerKind::Sequence { clips } => clips,
            _ => panic!("{} is not a Sequence layer", l.name),
        }
    }

    #[test]
    fn the_long_comp_is_the_one_the_budgets_are_stated_against() {
        let (doc, comp_id) = built();
        let comp = doc.comp(comp_id).unwrap();
        let end = comp.duration.0;
        assert_eq!(end, rat(7200, 1));
        assert_eq!(doc.items.len(), LONG_ITEMS + 1);

        let picture: Vec<&Layer> = comp.layers.iter().filter(|l| !l.audio_only).collect();
        let sound: Vec<&Layer> = comp.layers.iter().filter(|l| l.audio_only).collect();
        assert_eq!((picture.len(), sound.len()), (3, 4));
        let all = comp.layers.iter().flat_map(clips);
        assert_eq!(all.clone().count(), CLIP_COUNT);
        assert!(all
            .clone()
            .all(|c| c.place_start >= Rational::ZERO && c.place_end() <= end));
        // A clip never asks for more of its file than the file holds.
        assert!(all
            .clone()
            .all(|c| c.source_in >= Rational::ZERO && c.source_out <= rat(LONG_FILE_S, 1)));

        // One frame shows one clip: a picture row never overlaps itself, and
        // the generator leaves both gaps and shared edit points on it.
        for row in &picture {
            let mut sorted = clips(row).to_vec();
            sorted.sort_by_key(|c| c.place_start);
            let joins = sorted
                .windows(2)
                .map(|w| (w[0].place_end(), w[1].place_start));
            assert!(joins.clone().all(|(end, next)| end <= next), "{}", row.name);
            assert!(joins.clone().any(|(end, next)| end < next), "{}", row.name);
            assert!(joins.clone().any(|(end, next)| end == next), "{}", row.name);
        }
        // Sound may overlap, and on the free rows some of it does.
        let crossfades = sound.iter().filter(|row| {
            let mut sorted = clips(row).to_vec();
            sorted.sort_by_key(|c| c.place_start);
            sorted
                .windows(2)
                .any(|w| w[1].place_start < w[0].place_end())
        });
        assert_eq!(crossfades.count(), 2);

        // Every clip of the main picture row has its sound in the same place.
        let main = picture.iter().find(|l| l.name == MAIN_PICTURE).unwrap();
        let main_sound = sound.iter().find(|l| l.name == MAIN_SOUND).unwrap();
        for c in clips(main) {
            let partner: Vec<&Clip> = clips(main_sound)
                .iter()
                .filter(|s| s.link == c.link)
                .collect();
            assert_eq!(partner.len(), 1);
            assert_eq!(
                (
                    partner[0].place_start,
                    partner[0].place_duration,
                    partner[0].source
                ),
                (c.place_start, c.place_duration, c.source)
            );
        }

        // The montage: twelve-frame cuts between different files.
        let at = |f: i64| rat(f, FPS);
        let montage: Vec<&Clip> = clips(main)
            .iter()
            .filter(|c| c.place_start >= at(MONTAGE_START))
            .filter(|c| c.place_start < at(MONTAGE_START + MONTAGE_FRAMES))
            .collect();
        assert_eq!(montage.len() as i64, MONTAGE_FRAMES / MONTAGE_CUT);
        assert!(montage.windows(2).all(|w| w[0].source != w[1].source));

        // The same comp every time, ids and all.
        assert_eq!(built().0.items, doc.items);
    }
}
