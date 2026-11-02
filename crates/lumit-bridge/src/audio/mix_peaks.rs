//! The loaded mix's waveform, for the Sound mix row.

use super::lock;
use lumit_audio::mix::MixPlan;
use uuid::Uuid;

/// The loaded mix summarised over `[start_s, end_s)` - `min`, `max`, `rms`
/// per bucket, the shape the waveform lanes draw - and how long the mix runs,
/// or `None` when `comp` is not the mix the engine holds (nothing loaded, or
/// another comp's). A prepare running alongside is not a refusal: the plan in
/// hand is the mix as it was a moment ago, which is what the row wants over an
/// empty lane, and the next ask has the new one. The plan's `Arc` is cloned
/// under the lock and walked outside it, so a wide window never holds the
/// audio bookkeeping up.
pub(crate) fn mix_peaks(
    comp: Uuid,
    start_s: f64,
    end_s: f64,
    buckets: usize,
) -> Option<(Vec<f32>, f64)> {
    let (plan, rate) = {
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
    let duration = plan.total_frames as f64 / f64::from(rate);
    Some((mix_peaks_of(&plan, rate, start_s, end_s, buckets), duration))
}

/// How many frames a bucket of the Sound mix row may cover and still be read
/// from the samples. Past it the row is drawn from the files' own peaks.
/// Four thousand buckets of this are under a minute and a half of mix, which
/// is as much as this ever has decoded for one answer.
const MIX_READ_FRAMES: f64 = 1024.0;

/// `plan` summarised over `[start_s, end_s)`: read from the samples where the
/// window is narrow enough to decode for, and put together from each file's
/// peak pyramid where it is not ([`MixPlan::peaks_wide`]). Waits, for the
/// decode in the first case and for any pyramid not yet built in the second,
/// so it is not for a caller that cannot.
fn mix_peaks_of(plan: &MixPlan, rate: u32, start_s: f64, end_s: f64, buckets: usize) -> Vec<f32> {
    let rate_f = f64::from(rate);
    if buckets == 0 || end_s <= start_s {
        return plan.peaks(rate, start_s, end_s, buckets);
    }
    if (end_s - start_s) * rate_f / buckets as f64 <= MIX_READ_FRAMES {
        let from = (start_s.max(0.0) * rate_f) as usize;
        let to = (end_s.max(0.0) * rate_f).ceil() as usize;
        plan.warm(from, to.min(plan.total_frames));
        return plan.peaks(rate, start_s, end_s, buckets);
    }
    plan.peaks_wide(rate, start_s, end_s, buckets, &mut |sound| {
        crate::peaks::pyramid_for(sound)
    })
}
