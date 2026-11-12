//! The frames either side a temporal effect reads, shared with the
//! decoded-frame cache rather than copied out of it.
//!
//! Playing forward, all but one of a frame's neighbours were neighbours a
//! frame ago, so they are already decoded and filed. Handing over the filed
//! frame itself costs nothing, and the render can tell it has had this very
//! frame before and keep the picture it made of it.

use std::sync::Arc;

use super::{decode, CachedFrame, CompJob, FrameCacheKey, OpenDecoders, Request};

/// The neighbour frames `job`'s temporal effects read, by offset (none for a
/// plain layer). A neighbour that fails to decode is simply dropped: a
/// missing echo tap degrades the effect, never the frame. Missing footage has
/// none: it is drawn as bars, and the frames either side of bars are not the
/// file's either.
pub(super) fn of_job(
    decoders: &mut OpenDecoders,
    cache: &mut lumit_cache::ByteLru<FrameCacheKey, CachedFrame>,
    job: &CompJob,
) -> Vec<(i32, Arc<Vec<u8>>)> {
    if job.slate {
        return Vec::new();
    }
    job.temporal
        .iter()
        .filter_map(|&(offset, frame)| {
            let nreq = Request {
                generation: 0,
                item: job.item,
                source: job.source.clone(),
                frame,
                target_width: job.target_width,
                slate: None,
                // A neighbour frame is the same picture at another moment, so
                // it is read through the same channels: an echo of an
                // extracted depth pass is an echo of the depth pass.
                channels: job.channels.clone(),
            };
            shared(decoders, cache, &nreq).map(|p| (offset, p))
        })
        .collect()
}

/// The frame `req` names, as the cache holds it, decoding it first if it is
/// not there. `None` when it will not decode: a missing neighbour degrades the
/// effect, never the frame.
pub(super) fn shared(
    decoders: &mut OpenDecoders,
    cache: &mut lumit_cache::ByteLru<FrameCacheKey, CachedFrame>,
    req: &Request,
) -> Option<Arc<Vec<u8>>> {
    let key = (req.item, req.frame, req.target_width, req.channels.clone());
    let stamp = req.source.frame_stamp(req.frame);
    let filed = |cache: &mut lumit_cache::ByteLru<FrameCacheKey, CachedFrame>| {
        cache
            .get(&key)
            .filter(|hit| hit.stamp == stamp)
            .map(|hit| Arc::clone(&hit.rgba))
    };
    if let Some(hit) = filed(cache) {
        return Some(hit);
    }
    // Not filed, or filed from a file saved over since: the decode deals with
    // both, and files what it read unless it is a stand-in.
    let px = decode(decoders, cache, req, false).ok()?;
    filed(cache).or_else(|| Some(Arc::new(px.rgba)))
}
