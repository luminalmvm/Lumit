//! The pictures made of a layer's neighbour frames and of its own source, kept
//! from one frame to the next.
//!
//! Playing forward, a temporal effect reads nearly the same frames either side
//! as it did a frame ago: Denoise at two frames either side reads four, three
//! of which it read last time, and an Echo reads up to sixteen. The decode
//! hands each over as the very frame the cache holds, so a frame met again is
//! known by that alone and the picture made of it last time is used again
//! rather than uploaded and converted a second time. A layer's own source is
//! kept the same way, so a still, a text or a shape that has not changed is
//! uploaded once, and a frame made as a neighbour is found again as the next
//! frame's own source.
//!
//! A kept picture is only ever read. Every effect that reads neighbours writes
//! a picture of its own, so none of them is handed back to the frame's pool
//! and drawn over. They are charged to the governor while kept, and all go
//! back when it is short of memory, with Denoise's own spare pictures.
//!
//! Also the field a rebuilt neighbour is handed, which for Follow motion off
//! is one of no motion rather than a measurement.

use std::sync::{Arc, Weak};

use lumit_core::fx::temporal::FlowWant;

impl super::Realiser<'_> {
    /// The motion from `now` to `then` as `want` asks for it: measured, or
    /// for Follow motion off one of no motion at full trust, which says only
    /// that the neighbour is usable where it is.
    pub(super) fn field_to(
        &self,
        now: &wgpu::Texture,
        then: &wgpu::Texture,
        turned: bool,
        want: FlowWant,
    ) -> Option<wgpu::Texture> {
        match want {
            FlowWant::Still => Some(lumit_gpu::fx::common::upload_flow_texels(
                &self.ctx,
                &[0.0, 0.0, 1.0, 0.0],
                1,
                1,
            )),
            FlowWant::Measured | FlowWant::Full => self.measure_flow(now, then, turned),
        }
    }

    /// `pixels` as a picture, read as `format` in `colour_space`: the one kept
    /// from before where it is the same frame read the same way, otherwise
    /// one `upload` makes now. `raw` is the eight-bit upload that goes
    /// straight to the composite, a different picture from the linear one.
    /// Uploaded and not kept when the store is in use or there is none.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn kept_source(
        &self,
        pixels: &Arc<Vec<u8>>,
        w: u32,
        h: u32,
        format: lumit_media::decode::PixelFormat,
        colour_space: &Option<String>,
        raw: bool,
        upload: impl Fn(&[u8], u32, u32) -> wgpu::Texture,
    ) -> wgpu::Texture {
        match self.flow.map(|f| f.try_borrow_mut()) {
            Some(Ok(mut flow)) => {
                let read = (w, h, format, colour_space.clone(), raw);
                flow.neighbours
                    .picture(&self.ctx, self.colour_config, pixels, read, &upload)
            }
            _ => upload(pixels, w, h),
        }
    }

    /// Under memory pressure: every kept neighbour picture and Denoise's
    /// spare pictures go back, to be made again when next wanted.
    pub(super) fn let_go_of_kept(&self) {
        if let Some(Ok(mut flow)) = self.flow.map(|f| f.try_borrow_mut()) {
            flow.neighbours = NeighbourTextures::default();
        }
        lumit_gpu::fx::denoise::DenoiseKernels::let_go(self.fx);
    }
}

/// One kept picture and what it was made from.
struct Kept {
    /// The decoded frame. Held weakly, so keeping a picture never keeps a
    /// frame the cache has let go; while the handle lives, no other frame can
    /// be at its address, so the address is the frame's name.
    pixels: Weak<Vec<u8>>,
    read: Read,
    picture: wgpu::Texture,
    /// What it holds on the card, charged to the governor for as long as it
    /// is kept.
    _held: lumit_budget::Reservation,
    /// The frame it was last used in.
    used: u64,
}

/// How a picture was read: the size, the sample width, the colour space, and
/// whether it is the eight-bit upload that goes straight to the composite.
type Read = (
    u32,
    u32,
    lumit_media::decode::PixelFormat,
    Option<String>,
    bool,
);

/// The kept pictures. One stays while its frame is held and it was used in
/// this frame or the one before, which covers a window of neighbours and a
/// still alike.
#[derive(Default)]
pub struct NeighbourTextures {
    kept: Vec<Kept>,
    /// The frame being drawn, counted by [`Self::next_frame`].
    frame: u64,
    /// The colour config and the working format the pictures were made under.
    /// When either changes, nothing kept is the picture it would make now.
    made_under: Option<(Weak<crate::colour::Loaded>, wgpu::TextureFormat)>,
}

impl super::CompositeFlow {
    /// A new frame is being drawn: what was not used in the last one goes.
    pub fn next_frame(&mut self) {
        self.neighbours.next_frame();
    }

    /// Fail a frame whose model will not paint rather than draw it with the
    /// built-in engine. Set on the renderer an export writes through.
    pub fn refuse_substitution(&mut self) {
        self.refuse = true;
    }

    /// Why the last frame was failed, once.
    pub fn take_refusal(&mut self) -> Option<String> {
        self.refused.take()
    }
}

impl NeighbourTextures {
    /// A new frame is being drawn. A picture not used in the last one goes,
    /// and so does one whose frame nobody holds any more.
    fn next_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        self.kept
            .retain(|k| k.used + 1 >= frame && k.pixels.strong_count() > 0);
    }

    /// The picture of each of `frames` (`(offset, pixels, width, height)`),
    /// read as `format` in `colour_space`, as [`Self::picture`] finds or
    /// makes it.
    pub(super) fn pictures(
        &mut self,
        ctx: &lumit_gpu::GpuContext,
        config: Option<&Arc<crate::colour::Loaded>>,
        frames: &[(i32, Arc<Vec<u8>>, u32, u32)],
        format: lumit_media::decode::PixelFormat,
        colour_space: &Option<String>,
        upload: impl Fn(&[u8], u32, u32) -> wgpu::Texture,
    ) -> Vec<(i32, wgpu::Texture)> {
        frames
            .iter()
            .map(|(offset, pixels, w, h)| {
                let read = (*w, *h, format, colour_space.clone(), false);
                (*offset, self.picture(ctx, config, pixels, read, &upload))
            })
            .collect()
    }

    /// The picture of `pixels` read as `read` says: one kept from before
    /// where it is the same frame read the same way, otherwise one `upload`
    /// makes now, which is kept if the governor has room and no pressure.
    fn picture(
        &mut self,
        ctx: &lumit_gpu::GpuContext,
        config: Option<&Arc<crate::colour::Loaded>>,
        pixels: &Arc<Vec<u8>>,
        read: Read,
        upload: &impl Fn(&[u8], u32, u32) -> wgpu::Texture,
    ) -> wgpu::Texture {
        let under = (config.map_or_else(Weak::new, Arc::downgrade), ctx.working());
        let same = self
            .made_under
            .as_ref()
            .is_some_and(|(c, f)| c.ptr_eq(&under.0) && *f == under.1);
        if !same {
            self.kept.clear();
            self.made_under = Some(under);
        }
        let found = self
            .kept
            .iter_mut()
            .find(|k| std::ptr::eq(k.pixels.as_ptr(), Arc::as_ptr(pixels)) && k.read == read);
        if let Some(k) = found {
            k.used = self.frame;
            return k.picture.clone();
        }
        let (w, h) = (read.0, read.1);
        let picture = upload(pixels, w, h);
        // A frame nobody holds any more will never be asked for again.
        self.kept.retain(|k| k.pixels.strong_count() > 0);
        // Nothing new is kept under pressure, as the effect cache files nothing.
        let bytes = lumit_gpu::texture_bytes(picture.format(), w, h);
        let held = match ctx.vram_pressure().should_trim() {
            true => None,
            false => ctx.ledger().try_reserve(lumit_budget::Tier::Vram, bytes),
        };
        if let Some(held) = held {
            self.kept.push(Kept {
                pixels: Arc::downgrade(pixels),
                read,
                picture: picture.clone(),
                _held: held,
                used: self.frame,
            });
        }
        picture
    }
}
