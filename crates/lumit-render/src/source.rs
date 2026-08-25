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
///   original's raster (K-419), and every transform, mask and effect parameter
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
    if proxy_agrees(original, probes.proxy_probe(item)) {
        Some((proxy, original))
    } else {
        Some((&f.media, original))
    }
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

    /// Only a probed video stream reports a picture; the has-audio question is
    /// answered by both audio-only files and video files with a sound track.
    #[test]
    fn video_and_audio_are_reported_independently() {
        let v = SourceProbe::Video {
            fps: 24.0,
            width: 1920,
            height: 1080,
            frames: 240,
            audio: true,
        };
        assert_eq!(v.video(), Some((24.0, 1920, 1080, 240)));
        assert!(v.has_audio());
        assert!(SourceProbe::AudioOnly.video().is_none());
        assert!(SourceProbe::AudioOnly.has_audio());
        assert!(!SourceProbe::Missing.has_audio());
    }

    /// A plain map is a probe source, and an id it does not hold is unprobed —
    /// the shape tests and simple callers lean on.
    #[test]
    fn a_map_answers_unprobed_for_unknown_items() {
        let mut map = std::collections::HashMap::new();
        let known = Uuid::now_v7();
        map.insert(known, SourceProbe::Missing);
        assert_eq!(map.probe(known), SourceProbe::Missing);
        assert_eq!(map.probe(Uuid::now_v7()), SourceProbe::Unprobed);
        assert_eq!(NoProbes.probe(known), SourceProbe::Unprobed);
    }
}
