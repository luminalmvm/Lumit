//! An export in the browser build. The desktop's renders on one thread and
//! writes on another (`run`). The page has one thread, which must never
//! wait, so here an export takes a step at a time: each starts a frame's
//! composite and read-back, or takes the read-backs that have come off the
//! card and hands those frames to the page, which encodes them with the
//! browser's own encoders and writes the file (`lumit_media::web::export`).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;

use lumit_core::model::Document;
use lumit_media::web::export::{Page, Settings, State};
use uuid::Uuid;

use super::frame::{self, Pack, Span};
use super::{
    apply_render_overrides, audio_samples_through, fps_rational, AudioJob, ExportEvent,
    ExportFormat, ExportHandle, ExportSpec,
};
use crate::headless::HeadlessRenderer;

/// Frames composited and on their way off the card at once.
const IN_FLIGHT: usize = 2;

/// How long to leave it when there is nothing to do but wait.
const WAIT: Duration = Duration::from_millis(4);

/// An export in progress, moved on by [`Self::step`].
pub struct WebExport {
    doc: Arc<Document>,
    comp: Uuid,
    spec: ExportSpec,
    span: Span,
    pack: Pack,
    renderer: HeadlessRenderer,
    page: Page,
    name: PathBuf,
    sound: Option<Vec<f32>>,
    channels: usize,
    /// Samples a channel handed to the page.
    fed: usize,
    /// Frames started, and frames handed to the page.
    started: usize,
    written: usize,
    coming: VecDeque<lumit_gpu::PendingReadback>,
    /// What [`lumit_media::frames_landed`] said when a frame last drew
    /// footage the page was still decoding. It is drawn again once that moves.
    landed: Option<u64>,
    finishing: bool,
    tx: Sender<ExportEvent>,
    cancel: Arc<AtomicBool>,
}

/// Start exporting `comp` of `doc` as `codec`, one of
/// [`lumit_media::web::export::codecs`], to a file called `name`.
pub fn start(
    doc: Arc<Document>,
    comp: Uuid,
    audio: &[AudioJob],
    name: PathBuf,
    spec: ExportSpec,
    codec: &str,
) -> Result<(ExportHandle, WebExport), String> {
    // The render settings that change the document do it on this export's own
    // throwaway snapshot, never on the project (docs/06 §7.2).
    let doc = apply_render_overrides(&doc, &spec.render_options()).unwrap_or(doc);
    let c = doc.comp(comp).ok_or("composition missing")?;
    if !matches!(spec.format, ExportFormat::Video(_)) {
        return Err(format!("export: unknown format '{codec}'"));
    }
    let span = Span::of(c, &spec);
    let sound = frame::sound(&spec, c, audio, &span);
    let mut renderer = HeadlessRenderer::new().map_err(|e| format!("export renderer: {e}"))?;
    renderer.refuse_substitution();
    renderer.sync_colour(&doc);
    spec.check_with_colour(renderer.colour())?;
    renderer.set_colour_output(spec.colour_space.ocio_name().map(str::to_owned));
    // Even, for the 4:2:0 the encoders write.
    let delivered = frame::delivered(&spec, c);
    let size = ((delivered.0 & !1).max(2), (delivered.1 & !1).max(2));
    let (fps_num, fps_den) = fps_rational(span.out_fps);
    let page = Page::open(&Settings {
        name: &name.to_string_lossy(),
        codec,
        width: size.0,
        height: size.1,
        fps_num,
        fps_den,
        bit_rate: spec
            .resolved_bitrate(size, span.out_fps)
            .map(|(target, _)| target),
        sound: sound.as_ref().map(|_| {
            (
                spec.audio_rate,
                spec.audio_layout.channels(),
                spec.audio_bit_rate,
            )
        }),
    })?;
    let (tx, events) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let _ = tx.send(ExportEvent::Encoder("WebCodecs"));
    let _ = tx.send(ExportEvent::Progress {
        frame: 0,
        total: span.total,
    });
    let export = WebExport {
        pack: Pack::new(&spec, c, size),
        channels: usize::from(spec.audio_layout.channels()),
        doc,
        comp,
        spec,
        span,
        renderer,
        page,
        name,
        sound,
        fed: 0,
        started: 0,
        written: 0,
        coming: VecDeque::new(),
        landed: None,
        finishing: false,
        tx,
        cancel: Arc::clone(&cancel),
    };
    Ok((ExportHandle { events, cancel }, export))
}

impl WebExport {
    /// One step: how long to leave it before the next, or `None` once the
    /// export has ended and said how.
    pub fn step(&mut self) -> Option<Duration> {
        match self.advance() {
            Ok(Some(wait)) => Some(wait),
            Ok(None) => {
                let _ = self.tx.send(ExportEvent::Done(self.name.clone()));
                None
            }
            Err(error) => {
                self.page.cancel();
                let _ = self.tx.send(ExportEvent::Failed(error));
                None
            }
        }
    }

    fn advance(&mut self) -> Result<Option<Duration>, String> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let room = match self.page.state() {
            State::Room => true,
            State::Busy => false,
            State::Done => return Ok(None),
            State::Failed(error) => return Err(error),
        };
        // What has come off the card, in order, to the page.
        while let Some(coming) = self.coming.front_mut() {
            let Some(rgba) = self.renderer.poll_read(coming) else {
                break;
            };
            self.coming.pop_front();
            self.page.frame(&self.pack.frame(rgba?), self.written)?;
            self.written += 1;
            // After each picture, the sound that covers it, so the two are
            // written together.
            let rate = self.spec.audio_rate;
            self.feed(audio_samples_through(self.written, self.span.out_fps, rate))?;
            let _ = self.tx.send(ExportEvent::Progress {
                frame: self.written,
                total: self.span.total,
            });
        }
        if self.written == self.span.total {
            if !self.finishing {
                // Any samples the per-frame rounding left behind.
                self.feed(usize::MAX)?;
                self.page.finish()?;
                self.finishing = true;
            }
            return Ok(Some(WAIT));
        }
        let waiting = self.landed == Some(lumit_media::frames_landed());
        if self.started < self.span.total && self.coming.len() < IN_FLIGHT && room && !waiting {
            self.start_frame()?;
        }
        Ok(Some(WAIT))
    }

    fn start_frame(&mut self) -> Result<(), String> {
        let quality = self.spec.render.quality;
        let src = self.span.source(self.started) as u64;
        let stand_ins = lumit_media::stand_ins();
        let coming = self
            .renderer
            .start_preview(&self.doc, self.comp, src, quality)?;
        // A frame drawn while footage was still being decoded is never
        // written: it is drawn again once more has landed.
        if lumit_media::stand_ins() != stand_ins {
            drop(coming);
            self.renderer.clear_decoded();
            self.landed = Some(lumit_media::frames_landed());
            return Ok(());
        }
        self.coming.push_back(coming);
        self.started += 1;
        self.landed = None;
        // The page decodes the next frame's footage while this one comes
        // off the card.
        if self.started < self.span.total {
            let next = self.span.source(self.started) as u64;
            for want in self
                .renderer
                .prefetch_wants(&self.doc, self.comp, next, quality)
            {
                lumit_media::web::frames::ahead(&want.source, want.frame, want.target_width);
            }
        }
        Ok(())
    }

    /// The sound up to `through` samples a channel, that has not gone yet.
    fn feed(&mut self, through: usize) -> Result<(), String> {
        let Some(sound) = &self.sound else {
            return Ok(());
        };
        let upto = through.min(sound.len() / self.channels);
        if upto > self.fed {
            let samples = &sound[self.fed * self.channels..upto * self.channels];
            self.page.sound(samples, self.fed)?;
            self.fed = upto;
        }
        Ok(())
    }
}
