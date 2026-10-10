//! The long-form scenarios (docs/13-PERFORMANCE-RULES.md §2, B18 to B24).
//!
//! # In plain terms
//!
//! [`crate::scenarios`] asks how fast the engine draws a heavy frame. These
//! ask what a long cut costs around the drawing: opening a comp that names
//! three hundred files, the bookkeeping every frame pays whatever it shows,
//! playing across edit points, and committing one edit to a row of six
//! hundred clips. They run against [`crate::long_comp`], which is light to
//! draw on purpose, so a number here is the per-clip and per-file work and
//! not the composite.
//!
//! Two things the harness cannot reach, and what stands in for them:
//!
//! - **The playback scheduler** lives in `lumit-bridge`. B20 runs its own
//!   small loop in the same order the worker's takes: file what the read-ahead
//!   threads have decoded, post the coming frames' decodes and the files of
//!   the clips about to start, render. The threads themselves are the
//!   application's own ([`lumit_render::Prefetcher`]).
//! - **The read model** the interface is sent after an edit is built in
//!   `lumit-bridge` too, which is a C library and cannot be linked from here.
//!   B24 makes the engine reads that builder makes for every clip of every
//!   layer, which is where its time goes, without the structs it fills.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration as Wall, Instant};

use lumit_core::model::{Document, Layer, LayerKind};
use lumit_core::sequence::{self, Clip, ClipSource};
use lumit_core::time::{CompTime, Rational};
use lumit_core::{DocumentStore, Op};
use lumit_project::JournalFile;
use lumit_render::headless::HeadlessRenderer;
use lumit_render::plan::Quality;
use lumit_render::Prefetcher;
use uuid::Uuid;

use crate::comp::rat;
use crate::long_comp::{
    self, FPS, FRAMES, MAIN_PICTURE, MAIN_SOUND, MONTAGE_FRAMES, MONTAGE_START,
};
use crate::scenarios::{elapsed_ms, p95, scatter, span_scaled, Measurement, HALF, SCRUB};

/// Cold opens timed for B18, of which the fastest is kept: a comp opens once,
/// so there is no run of samples to take a percentile of, and the best of
/// three is the work itself without whatever else the machine was doing.
const OPEN_REPEATS: usize = 3;
/// Frames sampled across the two hours for B19.
const BOOKKEEPING_SAMPLES: usize = 64;
/// Frames B20 plays: four seconds of the montage, twenty of `V1`'s cuts.
const CUT_PLAY_FRAMES: u64 = 240;
/// How far ahead of the frame being rendered its source decodes are posted,
/// the playback scheduler's own figure.
const READ_AHEAD: u64 = 4;
/// Clips B21 steps through after the playback, one frame in each.
const DECODER_ITEMS: usize = 32;
/// Edits committed for B22, and for B23.
const TRIM_SAMPLES: usize = 20;
const RIPPLE_SAMPLES: usize = 10;
/// Read-model walks timed for B24.
const MODEL_SAMPLES: usize = 20;

/// The long-form comp, built once, ready to be driven by any scenario.
pub struct LongHarness {
    doc: Arc<Document>,
    comp: Uuid,
    warm: (Arc<Document>, Uuid),
}

impl LongHarness {
    /// Generate (or reuse) the media in `media_dir` and build the long-form
    /// comp over it. `Err` when the media cannot be made.
    pub fn new(media_dir: &Path) -> Result<Self, String> {
        let media = crate::media::generate_long(media_dir)?;
        let (doc, comp) = long_comp::build(&media)?;
        let (warm, warm_comp) = long_comp::warm_up(&media)?;
        Ok(Self {
            doc: Arc::new(doc),
            comp,
            warm: (Arc::new(warm), warm_comp),
        })
    }

    /// Every scenario in order, each reported to `progress` as it lands.
    pub fn all(&self, progress: &mut dyn FnMut(Measurement)) -> Result<Vec<Measurement>, String> {
        let mut out = Vec::new();
        let mut land = |ms: Vec<Measurement>| {
            for m in ms {
                progress(m);
                out.push(m);
            }
        };
        land(vec![self.b18_first_frame()?]);
        land(vec![self.b19_frame_bookkeeping()?]);
        land(self.b20_b21_playback_across_edit_points()?.to_vec());
        land(vec![self.b22_trim_commit()?]);
        land(vec![self.b23_ripple_delete_commit()?]);
        land(vec![self.b24_read_model()?]);
        Ok(out)
    }

    /// A renderer that has drawn one frame of the warm-up comp at `quality`
    /// and has never seen the long one ([`long_comp::warm_up`]).
    fn warmed(&self, quality: Quality) -> Result<HeadlessRenderer, String> {
        let mut r = HeadlessRenderer::new()?;
        let (doc, comp) = &self.warm;
        r.render_prepared(doc, *comp, 0, quality, false, false)?;
        r.settle_gpu();
        Ok(r)
    }

    /// **B18**: the long comp opened cold, to its first frame on screen.
    ///
    /// The frame under the playhead of a comp nothing has drawn yet, with
    /// whatever has to be learnt about its files on the way. The playhead is
    /// at the head of the montage, where all three picture rows show a clip.
    pub fn b18_first_frame(&self) -> Result<Measurement, String> {
        let mut best = f64::INFINITY;
        for _ in 0..OPEN_REPEATS {
            let mut r = self.warmed(SCRUB)?;
            let t = Instant::now();
            r.render_prepared(
                &self.doc,
                self.comp,
                MONTAGE_START as u64,
                SCRUB,
                false,
                true,
            )?;
            r.settle_gpu();
            best = best.min(elapsed_ms(t));
        }
        Ok(Measurement {
            budget: "B18",
            value_ms: best,
            frames: OPEN_REPEATS as u64,
        })
    }

    /// **B19**: one frame's bookkeeping, over a spread of times.
    ///
    /// The two questions the playback worker asks about a frame before it
    /// draws anything: which source frames the frame a few ahead will decode,
    /// and what this frame is called in the cache. Nothing is decoded and
    /// nothing is drawn. Each sample is asked twice and the second timed, so
    /// a file's one probe is never in the number and this is the walks alone.
    pub fn b19_frame_bookkeeping(&self) -> Result<Measurement, String> {
        let mut r = HeadlessRenderer::new()?;
        let mut ms = Vec::with_capacity(BOOKKEEPING_SAMPLES);
        for i in 0..BOOKKEEPING_SAMPLES {
            let frame = scatter(i, 0, (FRAMES as u64 - READ_AHEAD) as usize);
            let _ = r.prefetch_wants(&self.doc, self.comp, frame + READ_AHEAD, HALF);
            let _ = r.frame_key(&self.doc, self.comp, frame, HALF);
            let t = Instant::now();
            let wants = r.prefetch_wants(&self.doc, self.comp, frame + READ_AHEAD, HALF);
            let name = r.frame_key(&self.doc, self.comp, frame, HALF);
            ms.push(elapsed_ms(t));
            // A frame with no name is never cached, and a scenario that timed
            // those would be timing the early return.
            if name.is_none() {
                return Err(format!("B19: frame {frame} of the long comp has no name"));
            }
            drop(wants);
        }
        Ok(Measurement {
            budget: "B19",
            value_ms: p95(&mut ms),
            frames: BOOKKEEPING_SAMPLES as u64,
        })
    }

    /// **B20** and **B21**: playback across edit points, cold, and the
    /// decoders left open afterwards.
    ///
    /// B20 plays the montage, where `V1` cuts to a different file every
    /// twelve frames, and reports the 95th percentile of what one frame took.
    /// A cut is one frame in twelve, so the percentile lands on one. The loop
    /// keeps the comp's own pace, sleeping out the rest of each frame's time
    /// as playback does, so the read-ahead threads have the time between
    /// frames they really have.
    ///
    /// B21 then steps through [`DECODER_ITEMS`] more clips, a frame in each,
    /// and counts every decoder still open, the render's and the read-ahead
    /// threads' together. It is a count and not a time: see [`Measurement`].
    pub fn b20_b21_playback_across_edit_points(&self) -> Result<[Measurement; 2], String> {
        let mut r = self.warmed(HALF)?;
        let mut ahead = Prefetcher::default();
        let start = MONTAGE_START as u64;
        let last = start + MONTAGE_FRAMES as u64 - 1;
        let count = span_scaled(CUT_PLAY_FRAMES);
        let frame_time = Wall::from_secs_f64(1.0 / FPS as f64);
        let mut posted_to = start;
        let mut cut_ahead_to = start;
        let mut ms = Vec::with_capacity(count as usize);
        for i in 0..count {
            let frame = start + i;
            let t = Instant::now();
            for done in ahead.drain() {
                r.preload_decoded(done.item, done.frame, done.target_width, done.decoded);
            }
            let ahead_to = (frame + READ_AHEAD).min(last);
            for future in posted_to.max(frame) + 1..=ahead_to {
                ahead.request(r.prefetch_wants(&self.doc, self.comp, future, HALF));
            }
            posted_to = posted_to.max(ahead_to);
            // And a second ahead, the files of the clips about to start.
            let through = (frame + FPS as u64).min(last);
            let after = cut_ahead_to.max(ahead_to);
            if after < through {
                let wants = r.cut_wants(&self.doc, self.comp, after, through, HALF);
                ahead.request_ahead(wants, through - frame);
                cut_ahead_to = through;
            }
            r.render_prepared(&self.doc, self.comp, frame, HALF, false, true)?;
            ms.push(elapsed_ms(t));
            // Never faster than the comp plays. A frame that ran late is not
            // made up for by rushing the ones after it, which is the rule
            // playback itself keeps: time lost to a stall stays lost.
            if let Some(early) = frame_time.checked_sub(t.elapsed()) {
                std::thread::sleep(early);
            }
        }
        r.settle_gpu();
        let b20 = Measurement {
            budget: "B20",
            value_ms: p95(&mut ms),
            frames: count,
        };

        // One frame in each of the clips after the montage, on the main row.
        let after = rat(MONTAGE_START + MONTAGE_FRAMES, FPS);
        let main = self.row(&self.doc, MAIN_PICTURE)?;
        let mut later: Vec<&Clip> = clips_of(main)
            .iter()
            .filter(|c| c.place_start >= after)
            .collect();
        later.sort_by_key(|c| c.place_start);
        for c in later.iter().take(DECODER_ITEMS) {
            let frame = (c.place_start.to_f64() * FPS as f64).round() as u64 + 1;
            r.render_prepared(&self.doc, self.comp, frame, SCRUB, false, true)?;
        }
        r.settle_gpu();
        let open = r.decode_memory().1 + ahead.open();
        let b21 = Measurement {
            budget: "B21",
            value_ms: open as f64,
            frames: count + DECODER_ITEMS as u64,
        };
        Ok([b20, b21])
    }

    /// **B22**: one trim committed at scale.
    ///
    /// The end of one clip of the main picture row moves by a frame, and its
    /// linked sound with it, which is one undo step over two rows of six
    /// hundred clips each. Timed from reading the document to the edit being
    /// in the crash journal, since an edit is not safe until it is.
    pub fn b22_trim_commit(&self) -> Result<Measurement, String> {
        let (store, journal) = self.journalled("trim")?;
        let frame = rat(1, FPS);
        let mut ms = Vec::with_capacity(TRIM_SAMPLES);
        for i in 0..TRIM_SAMPLES {
            let t = Instant::now();
            let doc = store.snapshot();
            let mut ops = Vec::new();
            let main = clips_of(self.row(&doc, MAIN_PICTURE)?);
            // A different clip each time, across the whole row.
            let target = &main[scatter(i, 0, main.len()) as usize];
            let end = target.place_end().checked_sub(frame).map_err(time)?;
            for name in [MAIN_PICTURE, MAIN_SOUND] {
                let row = self.row(&doc, name)?;
                let mut clips = clips_of(row).to_vec();
                let at = clips
                    .iter()
                    .position(|c| c.link == target.link)
                    .ok_or("B22: the clip has no linked sound")?;
                clips[at] = clips[at]
                    .trim_end(end)
                    .ok_or("B22: the clip would not trim")?;
                ops.extend(clip_ops(self.comp, row, clips));
            }
            store
                .commit(Op::Batch { ops })
                .map_err(|e| format!("B22: {e}"))?;
            ms.push(elapsed_ms(t));
        }
        let _ = journal.clear();
        Ok(Measurement {
            budget: "B22",
            value_ms: p95(&mut ms),
            frames: TRIM_SAMPLES as u64,
        })
    }

    /// **B23**: a ripple delete near the start, committed.
    ///
    /// One clip of the main picture row goes, with its linked sound, and
    /// everything after it on every row closes up by its length. That is the
    /// edit that touches the most: all seven rows are rewritten and nearly
    /// every clip moves. Each sample takes the first clip from the start whose
    /// ripple no other row refuses, found before the clock starts.
    pub fn b23_ripple_delete_commit(&self) -> Result<Measurement, String> {
        let (store, journal) = self.journalled("ripple")?;
        let mut ms = Vec::with_capacity(RIPPLE_SAMPLES);
        for _ in 0..RIPPLE_SAMPLES {
            let doc = store.snapshot();
            let mut main: Vec<&Clip> = clips_of(self.row(&doc, MAIN_PICTURE)?).iter().collect();
            main.sort_by_key(|c| c.place_start);
            let gone = main
                .into_iter()
                .find(|c| self.ripple_delete(&doc, c).is_some())
                .ok_or("B23: no clip of the main row can be ripple deleted")?;
            let t = Instant::now();
            let ops = self
                .ripple_delete(&doc, gone)
                .ok_or("B23: the ripple would overlap")?;
            store
                .commit(Op::Batch { ops })
                .map_err(|e| format!("B23: {e}"))?;
            ms.push(elapsed_ms(t));
        }
        let _ = journal.clear();
        Ok(Measurement {
            budget: "B23",
            value_ms: p95(&mut ms),
            frames: RIPPLE_SAMPLES as u64,
        })
    }

    /// The ops of a ripple delete of `gone` and whatever is linked to it: on
    /// every row those clips go and each clip starting at or after `gone`'s
    /// end moves back by its length. `None` when a moved clip would land on
    /// one that stayed, which is an edit the Cut workspace refuses.
    fn ripple_delete(&self, doc: &Document, gone: &Clip) -> Option<Vec<Op>> {
        let comp = doc.comp(self.comp)?;
        let back = Rational::ZERO.checked_sub(gone.place_duration).ok()?;
        let mut ops = Vec::new();
        for row in &comp.layers {
            let kept: Vec<Clip> = clips_of(row)
                .iter()
                .filter(|c| c.id != gone.id && (c.link.is_none() || c.link != gone.link))
                .cloned()
                .collect();
            let moved = sequence::shift_from(&kept, gone.place_end(), back)?;
            ops.extend(clip_ops(self.comp, row, moved));
        }
        Some(ops)
    }

    /// **B24**: the engine's side of the read model, for the whole comp.
    ///
    /// After every edit the interface is sent each layer again with every one
    /// of its clips. This makes the reads that costs: per clip, its source's
    /// name, its frames on the comp's ruler, its speed and its Retime.
    pub fn b24_read_model(&self) -> Result<Measurement, String> {
        let comp = self.doc.comp(self.comp).ok_or("B24: the comp has gone")?;
        let mut ms = Vec::with_capacity(MODEL_SAMPLES);
        let mut seen = 0;
        for _ in 0..MODEL_SAMPLES {
            let t = Instant::now();
            seen = 0;
            for layer in &comp.layers {
                let offset = layer.start_offset.0;
                let frame = |t: Rational| {
                    comp.frame_rate
                        .frame_at(CompTime(offset.checked_add(t).unwrap_or(t)))
                };
                for clip in clips_of(layer) {
                    let (ClipSource::Footage(id) | ClipSource::Comp(id)) = clip.source;
                    let name = self.doc.item(id).map(|i| i.name().to_string());
                    let span = (frame(clip.place_start), frame(clip.place_end()));
                    let speed = clip.constant_speed();
                    let retime = clip.effective_retime();
                    seen += usize::from(name.is_some());
                    std::hint::black_box((name, span, speed, retime));
                }
            }
            ms.push(elapsed_ms(t));
        }
        if seen != long_comp::CLIP_COUNT {
            return Err(format!("B24 read {seen} clips of the long comp"));
        }
        Ok(Measurement {
            budget: "B24",
            value_ms: p95(&mut ms),
            frames: MODEL_SAMPLES as u64,
        })
    }

    /// The row called `name` in `doc`'s copy of the long comp.
    fn row<'a>(&self, doc: &'a Document, name: &str) -> Result<&'a Layer, String> {
        doc.comp(self.comp)
            .and_then(|c| c.layers.iter().find(|l| l.name == name))
            .ok_or_else(|| format!("the long comp has no row called {name}"))
    }

    /// A store holding the long comp whose every committed edit is appended
    /// to a journal on disk before the commit returns, as the application's
    /// change observer does it.
    fn journalled(&self, name: &str) -> Result<(DocumentStore, JournalFile), String> {
        // A folder of its own: clearing a journal takes its empty folder too.
        let path = std::env::temp_dir()
            .join("lumit-bench-journal")
            .join(format!("{name}-{}", std::process::id()))
            .join("ops.journal");
        let journal = JournalFile::at_path(path);
        journal
            .clear()
            .map_err(|e| format!("clearing the bench journal: {e}"))?;
        let store = DocumentStore::new(Document::clone(&self.doc));
        let observer = journal.clone();
        store.set_callback(Arc::new(move |change| {
            let _ = observer.append(&change.op);
        }));
        Ok((store, journal))
    }
}

/// A Sequence layer's clips. Empty for any other kind, which the long comp
/// has none of.
fn clips_of(layer: &Layer) -> &[Clip] {
    match &layer.kind {
        LayerKind::Sequence { clips } => clips,
        _ => &[],
    }
}

/// The ops that write `clips` to `row`: the list, and the row's bar when its
/// first start or last end has moved. The same two the Cut workspace commits.
fn clip_ops(comp: Uuid, row: &Layer, clips: Vec<Clip>) -> Vec<Op> {
    let span = sequence::clips_span(&clips);
    let mut ops = vec![Op::SetSequenceClips {
        comp,
        layer: row.id,
        clips,
    }];
    if let Some((start, end)) = span {
        if (start, end) != (row.in_point.0, row.out_point.0) {
            ops.push(Op::SetLayerSpan {
                comp,
                layer: row.id,
                in_point: CompTime(start),
                out_point: CompTime(end),
                start_offset: row.start_offset,
            });
        }
    }
    ops
}

/// A time that overflowed, as a scenario's error.
fn time(e: lumit_core::TimeError) -> String {
    format!("long comp time: {e}")
}

// Each scenario on its own, as the reference comp's are. Ignored: they need
// ffmpeg and a graphics adapter, and they take as long as they take.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn harness() -> LongHarness {
        LongHarness::new(&std::env::temp_dir().join("lumit-bench-media")).unwrap()
    }

    #[test]
    #[ignore = "a timing run: cargo test --release -p lumit-bench -- --ignored long"]
    fn long_cut_budgets() {
        harness().all(&mut |m| eprintln!("{m:?}")).unwrap();
    }
}
