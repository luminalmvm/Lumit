//! A freeze frame in the Cut workspace: the frame under the playhead held for
//! a length, as a clip of its own, with what follows moved later to make room.

use super::*;
use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
use lumit_core::sequence::Fade;

/// A flat map: the source moment `at`, held for `length`. Two keys, so the
/// speed line has a line to draw, at nought.
#[frb(ignore)]
fn held(at: f64, length: Rational) -> Property {
    let key = |time| Keyframe {
        time,
        value: at,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    Property {
        animation: Animation::Keyframed(vec![key(Rational::ZERO), key(length)]),
        extra: serde_json::Map::new(),
    }
}

impl Edit<'_> {
    /// Hold the frame `clip` shows at `at_frame` for `hold_frames`. The clip is
    /// cut there, a clip of the same source with a flat map goes between the
    /// halves, and everything from there on moves later. With `linked` the
    /// clips linked to it are cut and moved too, which leaves silence under the
    /// hold. Without it they stay, and play on through it.
    pub(super) fn freeze(
        mut self,
        clip: Uuid,
        at_frame: i64,
        hold_frames: i64,
        linked: bool,
    ) -> Cut {
        if hold_frames < 1 {
            return Err(BridgeCutResult::Nothing.into());
        }
        let (at, hold) = (self.time(at_frame)?, self.time(hold_frames)?);
        let (r, c) = self.find(clip)?;
        let this = self.rows[r].clips[c].clone();
        if self.rows[r].locked {
            return Err(BridgeCutResult::Locked.into());
        }
        if at < this.place_start || at >= this.place_end() {
            return Err(BridgeCutResult::Nothing.into());
        }
        let moment = this.source_at(at).ok_or(BridgeCutResult::Uncuttable)?;
        // A clip's effects are keyed from its own start, and the hold starts here.
        let mut frozen = this.duplicate();
        lumit_core::preset::shift_keys(&mut frozen.effects, this.place_start.checked_sub(at)?);
        frozen.grade_versions = lumit_core::grade::versions::respawned(
            &frozen.grade_versions,
            this.place_start.checked_sub(at)?,
        );
        let sound = self.rows[r].audio_only;
        frozen = Clip {
            source_in: moment,
            source_out: moment,
            place_start: at,
            place_duration: hold,
            retime: Some(held(moment.to_f64(), hold)),
            fade_in: Fade::default(),
            fade_out: Fade::default(),
            link: None,
            // A picture held has no sound to give, and a sound row's hold is
            // the silence of no time passing.
            audible: sound,
            takes: Vec::new(),
            transition: None,
            ..frozen
        };
        self.razor(&[r], at, linked)?;
        self.ripple(at, hold)?;
        self.rows[r].clips.push(frozen);
        self.commit()
    }
}

/// [`Edit::freeze`] as one edit on `of`.
#[frb(ignore)]
pub(crate) fn freeze_clip(
    of: &CompositionReference,
    clip: Uuid,
    at_frame: i64,
    hold_frames: i64,
    linked: bool,
) -> Result<BridgeCutResult, BridgeError> {
    cut(of, |edit| edit.freeze(clip, at_frame, hold_frames, linked))
}
