//! What a flow-synthesis retime feeds into a frame key.

/// Which model paints this job's in-between frames, for a job that asks one to.
///
/// The pack is not in the document, so serialising the Flow group cannot say
/// which one it is. Without this the pixels change when a pack is installed and
/// their name does not, and every effect on the layer hands back the picture it
/// worked out from the built-in engine's frames (docs/impl/addons.md §7).
pub(super) fn feed_synthesis(h: &mut blake3::Hasher, flow: &lumit_core::retime::FlowParams) {
    if flow.engine != lumit_core::retime::FlowEngineChoice::Rife {
        return;
    }
    if let Some(identity) = lumit_ml::synthesis::installed_identity() {
        h.update(b"synth/");
        h.update(&identity);
    }
}
