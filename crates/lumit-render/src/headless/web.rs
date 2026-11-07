//! The browser build's Viewer: a picture goes onto its view's canvas, and a
//! read-back lands on a later turn.

use super::{Arc, Document, HeadlessRenderer, PreparedFrame, Quality, SharedFrameInfo, Uuid};

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

/// The browser build's read-backs, which land on a later turn rather than
/// being waited for.
impl HeadlessRenderer {
    /// [`Self::render_preview`] without the wait. The bytes come back through
    /// [`Self::poll_read`], at the read-back's own [`size`](lumit_gpu::PendingReadback::size).
    pub fn start_preview_scaled(
        &mut self,
        doc: &Arc<Document>,
        comp_id: Uuid,
        frame: u64,
        quality: Quality,
        scale: f32,
    ) -> Result<lumit_gpu::PendingReadback, String> {
        let (shown, cw, ch) = self.preview_display_texture(doc, comp_id, frame, quality)?;
        let Some(parts) = self.parts.as_ref() else {
            return Err("headless preview: renderer is unavailable after an earlier fault".into());
        };
        let (sw, sh) = lumit_gpu::composite::scaled_size(cw, ch, scale);
        if (sw, sh) == (shown.width(), shown.height()) {
            return Ok(parts.colour.start_readback8(&self.gpu, &shown));
        }
        let reduced = parts.colour.display_scaled(
            &self.gpu,
            &shown,
            sw,
            sh,
            lumit_gpu::DisplayParams::NEUTRAL,
        );
        Ok(parts.colour.start_readback8(&self.gpu, &reduced))
    }

    /// Start reading a held frame back ([`Self::read_back_frame_texture`]
    /// without the wait). `None` when it is not held.
    #[must_use]
    pub fn start_held_read(&self, key: u128, bgra: bool) -> Option<lumit_gpu::PendingReadback> {
        let held = self.frame_textures.peek(&(key, bgra))?;
        let parts = self.parts.as_ref()?;
        Some(parts.colour.start_readback8(&self.gpu, &held.texture))
    }

    /// Start a scope trace ([`Self::render_scope`] without the wait).
    pub fn start_scope(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        kind: u32,
        colours: [[u8; 3]; 5],
    ) -> Result<lumit_gpu::PendingReadback, String> {
        let kind = lumit_gpu::scope::ScopeKind::from_code(kind)
            .ok_or_else(|| format!("headless scope: unknown kind {kind}"))?;
        let parts = self.parts.as_ref().ok_or("headless scope: no renderer")?;
        let frame = self.scope.upload_frame(&self.gpu, rgba, width, height);
        let trace = self
            .scope
            .run(&self.gpu, kind, colours.into(), &frame, width, height);
        Ok(parts.colour.start_readback8(&self.gpu, &trace))
    }

    /// The bytes of a read-back started above, once they have landed. `None`
    /// while they have not.
    pub fn poll_read(
        &self,
        read: &mut lumit_gpu::PendingReadback,
    ) -> Option<Result<Vec<u8>, String>> {
        read.poll(&self.gpu)
            .map(|bytes| bytes.map_err(|e| e.to_string()))
    }
}
