//! Probing footage and decoding frames — gated behind the `media` feature.
//!
//! # In plain terms
//!
//! When a project imports or opens footage, the bridge reads the file's vital
//! statistics (resolution, frame rate, frame count) so the panels can show them,
//! and later it decodes individual frames for the Viewer. That work needs
//! FFmpeg, wired in through `lumit-media`. The `media` cargo feature (default on)
//! pulls it; `--no-default-features` drops it, and then nothing probes — every
//! footage item simply reports status "unprobed" and the crate still builds and
//! tests without FFmpeg (CI parity).
//!
//! The *results* of probing are plain data ([`MediaStatus`]), always compiled,
//! so the snapshot code embeds them the same way whether the feature is on or
//! off. Probing runs **synchronously** on the calling thread at this phase (the
//! egui frontend probes on a background thread; the bridge will follow once the
//! command surface stabilises) — acceptable while the first files are small and
//! imported one at a time.

#[cfg(feature = "media")]
use std::collections::HashMap;
#[cfg(feature = "media")]
use uuid::Uuid;

/// A footage item's probe result — the plain-data mirror of `lumit-ui`'s
/// `MediaStatus` (docs/07 §3.3). Compiled with or without the `media` feature;
/// The decoded thumbnails, one downscaled RGBA per `(item, max_edge)`.
///
/// Held per project and cleared when the document changes identity, because a
/// relinked item's picture is a different picture. Probe *status* is not cached
/// here any more: `FootageReference::get_status` asks the file each time, since
/// a file can go missing between one question and the next.
#[derive(Default)]
pub(crate) struct MediaCache {
    #[cfg(feature = "media")]
    /// Keyed by item, size **and source frame**: a Sequence layer's clips
    /// each want the frame they start on, not the file's first (K-248).
    thumbs: HashMap<ThumbKey, Thumb>,
}

/// What a cached thumbnail is of: the item, the size asked for, and which
/// frame of it.
#[cfg(feature = "media")]
type ThumbKey = (Uuid, u32, i64);

/// A decoded thumbnail: width, height, and tightly packed RGBA8.
#[cfg(feature = "media")]
type Thumb = (u32, u32, Vec<u8>);

impl MediaCache {
    pub fn clear(&mut self) {
        #[cfg(feature = "media")]
        self.thumbs.clear();
    }

    /// A cached thumbnail for `(id, max_edge)`, if one was decoded already.
    #[cfg(feature = "media")]
    fn thumb_get(&self, id: Uuid, max_edge: u32, frame: i64) -> Option<Thumb> {
        self.thumbs.get(&(id, max_edge, frame)).cloned()
    }

    /// Store a decoded thumbnail for `(id, max_edge)`.
    #[cfg(feature = "media")]
    fn thumb_put(&mut self, id: Uuid, max_edge: u32, frame: i64, w: u32, h: u32, rgba: Vec<u8>) {
        self.thumbs.insert((id, max_edge, frame), (w, h, rgba));
    }
}
/// Decode one footage frame to tightly-packed RGBA8 (`media` feature only).
/// `None` on any failure (missing file, unreadable, frame index empty). Rebuilds
/// nothing it can load: the frame index comes from the engine's one sidecar-cache
/// helper ([`lumit_render::media_index`]), the same one the probe and the Viewer's
/// decode use, then a decoder is opened for this one call — synchronous, and not
/// yet pooled across calls (a later phase caches decoders per item).
#[cfg(feature = "media")]
pub(crate) fn decode_frame(
    path: &std::path::Path,
    frame: u64,
) -> Option<lumit_media::DecodedFrame> {
    if !path.is_file() {
        return None;
    }
    let index = lumit_render::media_index::load_or_build_index(path).ok()?;
    let mut decoder = lumit_media::VideoDecoder::open(path, index).ok()?;
    let count = decoder.frame_count();
    if count == 0 {
        return None;
    }
    let n = (frame as usize).min(count - 1);
    decoder.frame_rgba(n, None).ok()
}

/// Decode a thumbnail for `id` from `path`, memoised in `cache`.
///
/// It takes the cache and the resolved path rather than a whole bridge, which is
/// what let the v0 wrapper and the frb `FootageReference` drive the same decode,
/// the same box filter and the same cache. The v0 wrapper went with the Project
/// panel's port; `FootageReference::thumbnail` is the only caller now.
///
/// `max_edge` is clamped to 1..=4096: a request for a zero-pixel or absurd
/// thumbnail is a caller bug, not something to allocate for.
#[cfg(feature = "media")]
/// A thumbnail already decoded, if there is one. Read-only, so a caller can
/// check under a read lock and let it go before decoding anything.
#[cfg(feature = "media")]
pub(crate) fn thumb_cached(
    cache: &MediaCache,
    id: Uuid,
    max_edge: u32,
    frame: i64,
) -> Option<Thumb> {
    cache.thumb_get(id, max_edge.clamp(1, 4096), frame.max(0))
}

/// Decode and downscale one frame, touching no cache and holding no lock.
///
/// Split out from [`thumbnail_from_path`] because decoding a video frame takes
/// long enough to matter: doing it with the project locked stalls every reader,
/// and the render worker is one of them ([14-ENGINEERING-RULES.md] §3 — no
/// locks held across expensive work). Callers check [`thumb_cached`], let the
/// lock go, decode here, and take the lock again only to [`thumb_store`].
#[cfg(feature = "media")]
pub(crate) fn thumb_decode(path: &std::path::Path, max_edge: u32, frame: i64) -> Option<Thumb> {
    let max_edge = max_edge.clamp(1, 4096);
    let decoded = decode_frame(path, frame.max(0).unsigned_abs())?;
    Some(downscale_to_max_edge(
        decoded.width,
        decoded.height,
        &decoded.rgba,
        max_edge,
    ))
}

/// Remember a decoded thumbnail.
#[cfg(feature = "media")]
pub(crate) fn thumb_store(
    cache: &mut MediaCache,
    id: Uuid,
    max_edge: u32,
    frame: i64,
    thumb: &Thumb,
) {
    let (w, h, rgba) = thumb;
    cache.thumb_put(
        id,
        max_edge.clamp(1, 4096),
        frame.max(0),
        *w,
        *h,
        rgba.clone(),
    );
}

#[cfg(feature = "media")]
pub(crate) fn thumbnail_from_path(
    cache: &mut MediaCache,
    id: Uuid,
    max_edge: u32,
    path: &std::path::Path,
    at_frame: i64,
) -> Option<(u32, u32, Vec<u8>)> {
    let max_edge = max_edge.clamp(1, 4096);
    let at_frame = at_frame.max(0);
    if let Some(hit) = cache.thumb_get(id, max_edge, at_frame) {
        return Some(hit);
    }
    let frame = decode_frame(path, at_frame.unsigned_abs())?;
    let (w, h, rgba) = downscale_to_max_edge(frame.width, frame.height, &frame.rgba, max_edge);
    cache.thumb_put(id, max_edge, at_frame, w, h, rgba.clone());
    Some((w, h, rgba))
}

/// Downscale tightly-packed RGBA8 `src` (`sw`×`sh`) so its longer edge is at
/// most `max_edge`, preserving aspect. A box (area-average) filter — cheap and
/// clean enough for a panel thumbnail. Returns the source unchanged when it
/// already fits (never upscales) or when a degenerate size would result.
#[cfg(feature = "media")]
fn downscale_to_max_edge(sw: u32, sh: u32, src: &[u8], max_edge: u32) -> (u32, u32, Vec<u8>) {
    if sw == 0 || sh == 0 || src.len() < (sw as usize * sh as usize * 4) {
        return (sw, sh, src.to_vec());
    }
    let longer = sw.max(sh);
    if longer <= max_edge {
        return (sw, sh, src.to_vec());
    }
    let scale = f64::from(max_edge) / f64::from(longer);
    let dw = ((f64::from(sw) * scale).round() as u32).max(1);
    let dh = ((f64::from(sh) * scale).round() as u32).max(1);
    let mut out = vec![0u8; dw as usize * dh as usize * 4];
    let (sw_u, sh_u, dw_u, dh_u) = (sw as usize, sh as usize, dw as usize, dh as usize);
    for dy in 0..dh_u {
        // The source-row band this destination row averages over.
        let y0 = dy * sh_u / dh_u;
        let y1 = (((dy + 1) * sh_u).div_ceil(dh_u)).min(sh_u).max(y0 + 1);
        for dx in 0..dw_u {
            let x0 = dx * sw_u / dw_u;
            let x1 = (((dx + 1) * sw_u).div_ceil(dw_u)).min(sw_u).max(x0 + 1);
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in y0..y1 {
                let row = sy * sw_u * 4;
                for sx in x0..x1 {
                    let i = row + sx * 4;
                    r += u32::from(src[i]);
                    g += u32::from(src[i + 1]);
                    b += u32::from(src[i + 2]);
                    a += u32::from(src[i + 3]);
                    n += 1;
                }
            }
            let n = n.max(1);
            let o = (dy * dw_u + dx) * 4;
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
            out[o + 3] = (a / n) as u8;
        }
    }
    (dw, dh, out)
}
