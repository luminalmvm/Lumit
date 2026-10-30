//! Grouping clips, as Vegas groups events. Clips sharing a group are picked,
//! moved and trimmed together in the Cut timeline while Linked is on. Apart
//! from a clip's link, which pairs a picture clip with its sound.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::layer::LayerReference;
use crate::api::BridgeError;

impl LayerReference {
    /// Put one clip in `group`, or take it out of any with `None`.
    #[frb(sync)]
    pub fn set_clip_group(&self, clip: Uuid, group: Option<Uuid>) -> Result<(), BridgeError> {
        let (mut clips, index) = self.clips_and_index(clip)?;
        clips[index].group = group;
        self.commit_clips(clips)
    }
}
