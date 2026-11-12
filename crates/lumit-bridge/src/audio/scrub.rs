//! Hearing the mix with the transport stopped: a burst for a click, and the
//! jog for a drag.

use super::{forget, kick_prepare, lock, send, Cmd};
use lumit_core::model::Document;
use std::sync::Arc;
use uuid::Uuid;

/// How long one scrub burst sounds, in seconds, unless the comp's frame is
/// longer. A twelfth of a second is what After Effects plays for each move of
/// its playhead: long enough to tell a kick from a snare, short enough that
/// the beat is heard on the frame it falls on and not three frames early.
const SCRUB_BURST_S: f64 = 1.0 / 12.0;

/// Sound `comp`'s mix at `frame` in one short burst, with the transport
/// stopped: the playhead dragged with Ctrl held.
///
/// When the mix is loaded this is one message to the audio thread. When it is
/// not, it is prepared in the background and the bursts start once it lands,
/// so the first drag of a session is silent for as long as the decode takes.
/// The first burst after an edit prepares again too, so scrubbing straight
/// after moving a clip is never the mix from before the move.
pub(crate) fn scrub(comp: Uuid, frame: u64, doc: Arc<Document>) {
    let Some(fps) = doc
        .comp(comp)
        .map(|c| c.frame_rate.fps())
        .filter(|fps| *fps > 0.0)
    else {
        return;
    };
    let mut st = lock();
    // The comp is what the user wants to hear now, as in `play`.
    st.wanted_preview = None;
    let loaded = st.loaded_comp == Some(comp);
    let (at, len) = (frame as f64 / fps, (1.0 / fps).max(SCRUB_BURST_S));
    let seen = Arc::downgrade(&doc);
    let mut changed = !st.scrub_doc.ptr_eq(&seen);
    if !loaded {
        // The transport stays stopped, so this comp's mix does not start
        // playing by itself when it lands.
        st.playing = false;
        if st.loaded_comp.is_some() {
            // Another comp's mix, or a footage preview, is in the engine.
            // Silence it, and prepare this one whatever was asked before.
            send(&st, Cmd::Unload);
            forget(&mut st);
            changed = true;
        }
    }
    // The mix for this document is in the engine: one message. Otherwise it
    // is on its way, or about to be, and the burst waits for it. Only a new
    // document has anything to prepare, so a comp with no sound in it is
    // asked about once and not on every frame the playhead crosses.
    if loaded && !changed && st.scrub_held.is_none() {
        send(&st, Cmd::Scrub { at, len });
    } else if changed || st.scrub_held.is_some() {
        st.scrub_held = Some((at, len));
    }
    if changed {
        st.scrub_doc = seen;
        kick_prepare(&mut st, comp, doc);
    }
}

/// Play `comp`'s mix toward `frame` at the speed of the drag, with the
/// transport stopped: the playhead dragged with scrubbing on. The pitch
/// follows the speed. Before the mix is loaded it asks for it and is silent
/// until it lands; the first move after an edit asks for the mix again.
pub(crate) fn jog(comp: Uuid, frame: f64, doc: Arc<Document>) {
    let Some(fps) = doc
        .comp(comp)
        .map(|c| c.frame_rate.fps())
        .filter(|fps| *fps > 0.0)
    else {
        return;
    };
    let mut st = lock();
    st.wanted_preview = None;
    let loaded = st.loaded_comp == Some(comp);
    let seen = Arc::downgrade(&doc);
    let mut changed = !st.scrub_doc.ptr_eq(&seen);
    if !loaded {
        st.playing = false;
        if st.loaded_comp.is_some() {
            send(&st, Cmd::Unload);
            forget(&mut st);
            changed = true;
        }
    }
    if loaded {
        send(&st, Cmd::Jog(frame.max(0.0) / fps));
    }
    if changed {
        st.scrub_doc = seen;
        kick_prepare(&mut st, comp, doc);
    }
}

/// The drag is over: the jog fades out.
pub(crate) fn jog_end() {
    let st = lock();
    send(&st, Cmd::JogEnd);
}
