//! What the render pipeline needs to know about a footage item's media file.
//!
//! # In plain terms
//!
//! Before anything can be drawn, the pipeline has to answer three questions
//! about each piece of footage: is the file actually there, how fast does it
//! run, and how many frames does it have? Reading that off disk is called
//! *probing*, and each frontend already does it its own way — the egui shell
//! probes on a background thread and keeps a `MediaRegistry`; the bridge probes
//! on the calling thread and keeps a `MediaCache`.
//!
//! Rather than pick one and force the other to convert, this module states the
//! *question* as a trait ([`SourceProbes`]) and the *answer* as a small plain
//! enum ([`SourceProbe`]). Each frontend implements the trait over whatever it
//! already holds, and the pipeline never learns which frontend it is serving.
//! That is what keeps this crate an engine crate: it depends on no frontend, and
//! the arrow in docs/05-ARCHITECTURE.md still points one way.

use uuid::Uuid;

/// One footage item's probe result, as the render pipeline needs it.
///
/// The four failure-ish states are deliberately distinct, because they render
/// differently: unprobed contributes nothing *and* makes the frame unkeyable
/// (so it is never cached under a promise it did not keep); audio-only
/// contributes no picture but is perfectly healthy, so it must never draw the
/// slate; missing and unreadable both draw the colour-bars slate (docs/07 §3.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceProbe {
    /// Not probed yet. The layer draws nothing this frame and the whole comp
    /// frame is not cacheable — it will be re-rendered once the probe lands.
    Unprobed,
    /// A readable file carrying a video stream.
    Video {
        /// The container's exact rate.
        fps: f64,
        /// Native pixel size — what transforms act in, regardless of the
        /// resolution the frame is actually decoded at.
        width: u32,
        height: u32,
        /// Decodable frame count, from the frame index.
        frames: usize,
        /// Whether the same file also carries an audio stream.
        audio: bool,
    },
    /// A readable file with no video stream (a music track, say). Not an
    /// error: it contributes no picture and **no slate**.
    AudioOnly,
    /// Not on disk — moved, renamed, or on an unmounted drive. Draws the slate
    /// and leads to the relink flow.
    Missing,
    /// Present but unreadable (corrupt or unsupported). Draws the slate.
    Failed,
}

impl SourceProbe {
    /// The video details, or `None` for every state that has no picture.
    #[must_use]
    pub fn video(self) -> Option<(f64, u32, u32, usize)> {
        match self {
            SourceProbe::Video {
                fps,
                width,
                height,
                frames,
                ..
            } => Some((fps, width, height, frames)),
            _ => None,
        }
    }

    /// Whether this source draws the missing-footage slate (docs/07 §3.3).
    /// `AudioOnly` deliberately does not: flagging a healthy audio file as
    /// missing would be actively wrong.
    #[must_use]
    pub fn slates(self) -> bool {
        matches!(self, SourceProbe::Missing | SourceProbe::Failed)
    }

    /// Whether this source carries sound.
    #[must_use]
    pub fn has_audio(self) -> bool {
        matches!(
            self,
            SourceProbe::AudioOnly | SourceProbe::Video { audio: true, .. }
        )
    }
}

/// A frontend's probe cache, seen through the one question the pipeline asks.
/// An item nobody has probed answers [`SourceProbe::Unprobed`].
pub trait SourceProbes {
    fn probe(&self, item: Uuid) -> SourceProbe;

    /// The same question about this item's **proxy** file, when it has one.
    ///
    /// A separate question rather than a second entry under the same id: the
    /// two files are probed independently and both answers are needed at once
    /// — the original's to lay the layer out, the proxy's to decide whether the
    /// stand-in may be believed ([`effective_media`]).
    ///
    /// The default answers `Unprobed`, which reads the original: a frontend
    /// that has never heard of proxies keeps working exactly as it did, and a
    /// proxy nobody has probed yet is simply not used until it has been.
    fn proxy_probe(&self, _item: Uuid) -> SourceProbe {
        SourceProbe::Unprobed
    }
}

/// Which file a footage item's pixels are read from this render, and what the
/// pipeline should believe about them — **the one proxy resolution point**
/// (docs/06 §5.7).
///
/// # In plain terms
///
/// A footage item can carry a proxy: a small stand-in file the Viewer decodes
/// while you work. Two things then have to agree, or the frame cache starts
/// handing back the wrong picture: the decode planner, which says which file to
/// open, and the frame key, which names the finished frame. So both ask this
/// one function, and the answer carries the path *and* the probe together.
///
/// Two rules, both deliberate:
///
/// * **The probe returned is always the original's.** A proxy is a smaller
///   copy of the same footage, so the layer keeps the original's pixel size and
///   the original's rate and length: geometry is in px@comp against the
///   original's raster, and every transform, mask and effect parameter
///   goes on meaning what it meant. All the proxy changes is how many pixels
///   come back from the decode — which is exactly what the preview-resolution
///   tier already does, through the same `target_width` machinery.
/// * **A proxy that disagrees about the footage's length is not used.** A
///   stand-in with a different frame count or a different rate is a stand-in
///   for something else: frame 300 of it is not frame 300 of the original, and
///   quietly showing it would put the wrong picture on the timeline with
///   nothing on screen to say so. It falls back to the original, and so does a
///   proxy that is missing, unreadable, or not probed yet.
///
/// `None` when `item` is not a footage item at all.
#[must_use]
pub fn effective_media<'a>(
    doc: &'a lumit_core::model::Document,
    probes: &dyn SourceProbes,
    item: Uuid,
) -> Option<(&'a lumit_core::model::MediaRef, SourceProbe)> {
    let lumit_core::model::ProjectItem::Footage(f) = doc.item(item)? else {
        return None;
    };
    let original = probes.probe(item);
    let Some(proxy) = doc.proxy_in_use(item) else {
        return Some((&f.media, original));
    };
    // Fields are rows of the file's own frames, and a scaled-down copy has
    // already blended them together, so there is nothing left to separate.
    let rows_matter = interpretation(f).separates_fields();
    if !rows_matter && proxy_agrees(original, probes.proxy_probe(item)) {
        Some((proxy, original))
    } else {
        Some((&f.media, original))
    }
}

/// How the decoder is told to read this item's file: the item's
/// interpretation, in the media crate's terms.
///
/// The numbers come from a project file, or from someone else in a shared
/// project, so the ones that size a frame or a clip are held to what real
/// footage could want and anything outside that reads as the file's own.
#[must_use]
pub fn interpretation(f: &lumit_core::model::FootageItem) -> lumit_media::Interpretation {
    use lumit_core::interpretation::{AlphaMode, FieldOrder, LinearLight, Pulldown};
    /// After Effects' own ceiling, and far past any use of it.
    const MOST_LOOPS: u32 = 9999;
    let it = &f.interpretation;
    let within = |n: u32, d: u32, lo: f64, hi: f64| {
        d > 0 && (lo..=hi).contains(&(f64::from(n) / f64::from(d)))
    };
    lumit_media::Interpretation {
        alpha: match it.alpha {
            AlphaMode::Straight => lumit_media::Alpha::Straight,
            AlphaMode::Ignore => lumit_media::Alpha::Ignore,
            AlphaMode::Premultiplied { matte } => lumit_media::Alpha::Premultiplied(matte),
        },
        invert_alpha: it.invert_alpha,
        // A run of stills has no rate of its own to conform, only the one the
        // item gives it, which rides on the source separately.
        conform_fps: it
            .frame_rate
            .filter(|_| f.sequence.is_none())
            .map(|r| (r.num(), r.den()))
            .filter(|(n, d)| within(*n, *d, 0.01, 999.0)),
        fields: match it.fields {
            FieldOrder::Off => lumit_media::Fields::Off,
            FieldOrder::Upper => lumit_media::Fields::Upper,
            FieldOrder::Lower => lumit_media::Fields::Lower,
        },
        preserve_edges: it.preserve_edges,
        pulldown: match it.pulldown {
            Pulldown::Off => lumit_media::Pulldown::Off,
            Pulldown::ThreeTwo { phase } => lumit_media::Pulldown::ThreeTwo(phase % 5),
            Pulldown::Advanced { phase } => lumit_media::Pulldown::Advanced(phase % 5),
        },
        pixel_aspect: it
            .pixel_aspect
            .filter(|(n, d)| n != d && within(*n, *d, 0.1, 10.0)),
        // Once is the plain reading, so it is written the plain way.
        loops: match it.plays().min(MOST_LOOPS) {
            1 => 0,
            n => n,
        },
        // Preserved numbers are taken as they stand, which is linear. An
        // assigned colour space says what curve the file has by itself, so
        // linear light is only asked of a file with none.
        linear: match (it.preserve_rgb, &f.colour_space, it.linear_light) {
            (true, _, _) => Some(true),
            (false, Some(_), _) | (false, None, LinearLight::Auto) => None,
            (false, None, LinearLight::On) => Some(true),
            (false, None, LinearLight::Off) => Some(false),
        },
    }
}

use lumit_media::MediaSource;

/// What an analysis of this item reads and what its answers are filed under:
/// the file as the item interprets it, and the file's fingerprint, marked with
/// that reading when it changes which pictures there are. A track made of a
/// clip's fields is not a track of its frames.
#[must_use]
pub fn analysed(
    f: &lumit_core::model::FootageItem,
    path: std::path::PathBuf,
    fingerprint: &lumit_core::model::Fingerprint,
) -> (MediaSource, lumit_core::model::Fingerprint) {
    let interpret = interpretation(f);
    // The rate only re-times the same pictures in the same order, so an
    // analysis made before it was set still stands.
    let pictures = lumit_media::Interpretation {
        conform_fps: None,
        ..interpret
    };
    let mut fingerprint = fingerprint.clone();
    if !pictures.is_plain() {
        fingerprint.head_tail_hash = format!("{}#{pictures:?}", fingerprint.head_tail_hash);
    }
    // Another render pass is another picture.
    if let Some(pass) = &f.interpretation.render_pass {
        fingerprint.head_tail_hash = format!("{}#p{pass}", fingerprint.head_tail_hash);
    }
    let source = MediaSource {
        path,
        sequence_fps: f.sequence_fps(),
        source_layer: f.source_layer,
        render_pass: f.interpretation.render_pass.clone(),
        interpret,
        stamp: None,
    };
    (source, fingerprint)
}

/// What a file's alpha looks like it means, from its own frames. `None` when
/// it can't be read or no frame looked at has any transparency in it.
///
/// Decodes up to three frames, so it is for a worker thread.
#[must_use]
pub fn guess_alpha(src: &lumit_media::MediaSource) -> Option<lumit_media::Alpha> {
    use lumit_media::interpret::guess_alpha;
    let raw = src.raw();
    if let Some(frame) = lumit_media::read_own(&raw, 0, None) {
        return guess_alpha(&frame.ok()?);
    }
    let index = crate::media_index::load_or_build_index(&raw).ok()?;
    let frames = index.frame_count();
    let mut decoder = lumit_media::VideoDecoder::open(&raw, index).ok()?;
    // The middle first: a clip often opens and closes on nothing at all.
    let mut tried = Vec::new();
    for n in [frames / 2, 0, frames.saturating_sub(1)] {
        if tried.contains(&n) {
            continue;
        }
        tried.push(n);
        if let Some(guess) = guess_alpha(&decoder.frame_rgba(n, None).ok()?) {
            return Some(guess);
        }
    }
    None
}

/// The pulldown cadence a file's own frames show, or `Off` when none stands
/// out. Looks at a run of frames from the start and, if that says nothing (a
/// still opening, say), one from the middle.
///
/// Decodes up to thirty full frames, so it is for a worker thread.
#[must_use]
pub fn guess_pulldown(src: &lumit_media::MediaSource) -> lumit_media::Pulldown {
    use lumit_media::interpret::{comb_score, guess_pulldown};
    const RUN: usize = 15;
    let raw = src.raw();
    let Ok(index) = crate::media_index::load_or_build_index(&raw) else {
        return lumit_media::Pulldown::Off;
    };
    let frames = index.frame_count();
    let Ok(mut decoder) = lumit_media::VideoDecoder::open(&raw, index) else {
        return lumit_media::Pulldown::Off;
    };
    let mut starts = vec![0];
    if frames / 2 >= RUN {
        starts.push(frames / 2);
    }
    for start in starts {
        let scores: Vec<f64> = (start..(start + RUN).min(frames))
            .map_while(|n| decoder.frame_rgba(n, None).ok())
            .map(|frame| comb_score(&frame))
            .collect();
        let guess = guess_pulldown(&scores, start);
        if guess != lumit_media::Pulldown::Off {
            return guess;
        }
    }
    lumit_media::Pulldown::Off
}

/// Whether a proxy's own probe agrees with the original's about *what footage
/// this is*: same frame count, same rate. Dimensions are free to differ — that
/// is the whole point of a proxy — and are taken from the original either way.
///
/// The rate is compared loosely (a thousandth of a frame per second), because
/// two containers can state 29.97 as 30000/1001 and as a rounded double and
/// mean the same footage; the frame count is compared exactly, because it is a
/// count.
fn proxy_agrees(original: SourceProbe, proxy: SourceProbe) -> bool {
    let (Some((ofps, _, _, oframes)), Some((pfps, _, _, pframes))) =
        (original.video(), proxy.video())
    else {
        return false;
    };
    oframes == pframes && (ofps - pfps).abs() < 1e-3
}

/// Nothing is probed — the do-nothing implementation a build with no media
/// support (or a test with no files) hands in.
pub struct NoProbes;

impl SourceProbes for NoProbes {
    fn probe(&self, _item: Uuid) -> SourceProbe {
        SourceProbe::Unprobed
    }
}

impl SourceProbes for std::collections::HashMap<Uuid, SourceProbe> {
    fn probe(&self, item: Uuid) -> SourceProbe {
        self.get(&item).copied().unwrap_or(SourceProbe::Unprobed)
    }
}

/// Originals and proxies as two plain maps — the pair form, for a caller (or a
/// test) that holds both without a probe cache of its own.
impl SourceProbes
    for (
        std::collections::HashMap<Uuid, SourceProbe>,
        std::collections::HashMap<Uuid, SourceProbe>,
    )
{
    fn probe(&self, item: Uuid) -> SourceProbe {
        self.0.probe(item)
    }

    fn proxy_probe(&self, item: Uuid) -> SourceProbe {
        self.1.probe(item)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Audio-only media must never slate — the bug that painted colour bars
    /// over a perfectly good music layer. Missing and unreadable both do.
    #[test]
    fn only_missing_and_failed_draw_the_slate() {
        assert!(!SourceProbe::AudioOnly.slates());
        assert!(!SourceProbe::Unprobed.slates());
        assert!(SourceProbe::Missing.slates());
        assert!(SourceProbe::Failed.slates());
        assert!(!SourceProbe::Video {
            fps: 30.0,
            width: 8,
            height: 8,
            frames: 10,
            audio: false,
        }
        .slates());
    }
}
