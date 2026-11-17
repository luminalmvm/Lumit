//! A footage file heard by itself, with no composition behind it.

use super::{engine, ensure_device, forget, lock, send, Cmd, Device, Loaded};
use lumit_audio::plan::MixPlan;
use lumit_render::mix::graph::{GraphInfo, GraphNode};
use std::sync::Arc;
use uuid::Uuid;

/// Play one footage file's own sound from `start_s` seconds into it, with no
/// composition behind it: the Project panel's preview button (docs/07 §3.1)
/// from the top, and the Source viewer from wherever its playhead is.
///
/// `sound` is the file and which of its audio streams are heard, mixed.
///
/// [`clock`] then reads **the file's own time**, seconds from the top of the
/// file, and so starts at `start_s` and not at zero. A start past the end of
/// the file is held to the end, and one that is not a number is the top.
///
/// The mix is a single clip at unity, loaded under the **item's** id. A footage
/// id is never a comp id, so a preview and a comp's mix cannot be taken for one
/// another, and starting either silences the other — which is what one pair of
/// speakers means. Stopping is [`stop`] and where it has got to is [`clock`],
/// exactly as for a comp.
///
/// Returns at once: the file is opened on its own thread, so a slow disk does
/// not hold the press.
pub(crate) fn preview(item: Uuid, sound: lumit_media::audio::SoundSource, start_s: f64) {
    // `max` answers zero for a start that is not a number.
    let start_s = start_s.max(0.0);
    {
        let mut st = lock();
        if matches!(st.device, Device::Unavailable) {
            return;
        }
        st.wanted_preview = Some(item);
        st.slow = 1;
        send(&st, Cmd::Slow(1));
        // This file's preview is already in the engine: a second press is a
        // seek, not a second look at the file.
        if st.loaded_comp == Some(item) && st.loaded_preview.as_ref() == Some(&sound) {
            st.playing = true;
            let length = st
                .loaded_plan
                .as_ref()
                .map(|l| l.plan.total_frames as f64 / f64::from(l.rate));
            send(&st, Cmd::Start(length.map_or(start_s, |l| start_s.min(l))));
            return;
        }
    }

    engine::spawn(move || {
        let Some((tx, rate, generation)) = ensure_device() else {
            return;
        };

        // The same source a comp's mix reads: previewing a clip and then
        // dropping it in a comp decodes each block of it once.
        let source = lumit_audio::stream::pool().source(&sound, rate);
        // How long the preview is, is how long the file is. Finding that
        // reads the end of the file and not the whole of it.
        let frames = source.frames_exactly();
        if frames == 0 {
            return; // a file with no sound in it, or one that will not open
        }
        let duration_s = frames as f64 / f64::from(rate);

        // One clip at unity on one strip, the item's own.
        let plan = Arc::new(MixPlan::single(
            vec![lumit_audio::plan::PlacedClip {
                source: Arc::clone(&source),
                start_frame: 0,
                src_start: 0,
                len: frames,
                gain: [1.0, 1.0],
                envelope: None,
                node: 0,
                silent: false,
            }],
            frames,
        ));
        let info = GraphInfo {
            nodes: vec![GraphNode {
                layer: item,
                bus: false,
                sends: Vec::new(),
            }],
        };
        // Decoded where it is about to start, so the press is heard from its
        // first sample.
        let start_s = start_s.min(duration_s);
        let from = (start_s * f64::from(rate)) as usize;
        plan.warm(from, from.saturating_add(rate as usize));

        let mut st = lock();
        // The output moved, or something else took the engine while this file
        // was opened (a comp played, or another preview was pressed). Drop the
        // work rather than stamping it over what the user is listening to now.
        if st.device_gen != generation || st.wanted_preview != Some(item) {
            return;
        }
        if st.loaded_comp.is_some() {
            forget(&mut st);
        }
        let _ = tx.send(Cmd::Load {
            plan: Arc::clone(&plan),
            start: Some(start_s),
            play: true,
        });
        st.loaded_comp = Some(item);
        st.loaded_preview = Some(sound);
        // No signature: a preview is not a comp, so no edit can arrive that
        // would want to be recognised as the same mix.
        st.loaded_sig = None;
        st.playing = true;
        st.loaded_plan = Some(Loaded {
            plan,
            rate,
            info: Arc::new(info),
            doc: None,
        });
    });
}
