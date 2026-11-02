//! The browser build's Viewer: a picture goes onto its view's canvas.

use super::{HeadlessRenderer, PreparedFrame, SharedFrameInfo};

impl HeadlessRenderer {
    /// Show an already-rendered frame on its view's canvas, where the front end
    /// takes it as an image. The browser's sibling of the shared texture.
    pub fn present_prepared(
        &mut self,
        prepared: &PreparedFrame,
        view: u32,
    ) -> Result<SharedFrameInfo, String> {
        let (width, height) = prepared.size();
        let handle = lumit_gpu::device::show_on_canvas(&self.gpu, &prepared.texture, view)?;
        Ok(SharedFrameInfo {
            handle,
            width,
            height,
            format: "rgba8888",
        })
    }
}

impl HeadlessRenderer {
    /// The bytes of a read-back [`Self::start_preview`] started, once they
    /// are off the card, and `None` until then. Never waits: the page can't,
    /// and the copy lands between its turns.
    pub fn poll_preview(
        &mut self,
        pending: &mut lumit_gpu::PendingReadback,
    ) -> Option<Result<Vec<u8>, String>> {
        pending
            .poll(&self.gpu)
            .map(|bytes| bytes.map_err(|e| format!("headless preview: {e}")))
    }
}
