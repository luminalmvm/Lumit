//! The loaded mix's waveform, for the Sound mix row and a bus's lane.

use super::lock;
use uuid::Uuid;

/// The loaded mix summarised over `[start_s, end_s)` - `min`, `max`, `rms`
/// per bucket, the shape the waveform lanes draw - and how long the mix runs,
/// or `None` when `comp` is not the mix the engine holds (nothing loaded, or
/// another comp's). With `bus`, what is routed into that Bus layer, through
/// each source's fader and send level, before its own rack and fader; `None`
/// as well when the bus is not in the mix. A prepare running alongside is not
/// a refusal: the plan in hand is the mix as it was a moment ago, which is
/// what the row wants over an empty lane, and the next ask has the new one.
/// The plan's `Arc` is cloned under the lock and walked outside it, so a wide
/// window never holds the audio bookkeeping up.
pub(crate) fn mix_peaks(
    comp: Uuid,
    bus: Option<Uuid>,
    start_s: f64,
    end_s: f64,
    buckets: usize,
) -> Option<(Vec<f32>, f64)> {
    let loaded = {
        let st = lock();
        // Not `worker_busy`: a prepare marks the worker busy the moment it is
        // kicked, and the row asks for one on every poll, so refusing while
        // one runs is refusing always. The loaded plan is only ever replaced
        // by a prepare that finished, so the worst this answers is the mix as
        // it stood one poll ago.
        if st.loaded_comp != Some(comp) {
            return None;
        }
        st.loaded_plan.clone()?
    };
    let node = match bus {
        Some(bus) => Some(
            loaded
                .info
                .node_of(bus)
                .filter(|&n| loaded.info.nodes.get(usize::from(n)).is_some_and(|n| n.bus))?,
        ),
        None => None,
    };
    let duration = loaded.plan.total_frames as f64 / f64::from(loaded.rate);
    let peaks = lumit_render::mix::peaks::node_peaks(
        &loaded.plan,
        loaded.rate,
        node,
        start_s,
        end_s,
        buckets,
        &mut |sound| crate::peaks::pyramid_for(sound),
    );
    Some((peaks, duration))
}
