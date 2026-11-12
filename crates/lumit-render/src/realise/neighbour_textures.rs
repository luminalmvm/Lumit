//! The pictures made of a layer's neighbour frames, kept from one frame to the
//! next.
//!
//! Playing forward, a temporal effect reads nearly the same frames either side
//! as it did a frame ago: Denoise at two frames either side reads four, three
//! of which it read last time, and an Echo reads up to sixteen. The decode
//! hands each over as the very frame the cache holds, so a frame met again is
//! known by that alone and the picture made of it last time is used again
//! rather than uploaded and converted a second time.
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
    /// How it was read: the size, the sample width and the colour space.
    read: (u32, u32, lumit_media::decode::PixelFormat, Option<String>),
    picture: wgpu::Texture,
    /// What it holds on the card, charged to the governor for as long as it
    /// is kept.
    _held: lumit_budget::Reservation,
    /// When it was last used, in calls.
    used: u64,
}

/// The kept pictures, most recently used last.
#[derive(Default)]
pub struct NeighbourTextures {
    kept: Vec<Kept>,
    /// Calls so far, for [`Kept::used`].
    calls: u64,
    /// The colour config and the working format the pictures were made under.
    /// When either changes, nothing kept is the picture it would make now.
    made_under: Option<(Weak<crate::colour::Loaded>, wgpu::TextureFormat)>,
}

impl NeighbourTextures {
    /// The picture of each of `frames` (`(offset, pixels, width, height)`),
    /// read as `format` in `colour_space`: one kept from before where it is
    /// the same frame read the same way, otherwise one `upload` makes now,
    /// which is kept for next time if the governor has room for it.
    ///
    /// Keeps what this call used and as many again of the most recent, so a
    /// second layer playing beside the first keeps its own.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pictures(
        &mut self,
        ctx: &lumit_gpu::GpuContext,
        config: Option<&Arc<crate::colour::Loaded>>,
        frames: &[(i32, Arc<Vec<u8>>, u32, u32)],
        format: lumit_media::decode::PixelFormat,
        colour_space: &Option<String>,
        upload: impl Fn(&[u8], u32, u32) -> wgpu::Texture,
    ) -> Vec<(i32, wgpu::Texture)> {
        let under = (config.map_or_else(Weak::new, Arc::downgrade), ctx.working());
        let same = self
            .made_under
            .as_ref()
            .is_some_and(|(c, f)| c.ptr_eq(&under.0) && *f == under.1);
        if !same {
            self.kept.clear();
            self.made_under = Some(under);
        }
        self.calls += 1;
        let now = self.calls;
        let out: Vec<(i32, wgpu::Texture)> = frames
            .iter()
            .map(|(offset, pixels, w, h)| {
                let read = (*w, *h, format, colour_space.clone());
                let found = self.kept.iter_mut().find(|k| {
                    std::ptr::eq(k.pixels.as_ptr(), Arc::as_ptr(pixels)) && k.read == read
                });
                if let Some(k) = found {
                    k.used = now;
                    return (*offset, k.picture.clone());
                }
                let picture = upload(pixels, *w, *h);
                let bytes = lumit_gpu::texture_bytes(picture.format(), *w, *h);
                if let Some(held) = ctx.ledger().try_reserve(lumit_budget::Tier::Vram, bytes) {
                    self.kept.push(Kept {
                        pixels: Arc::downgrade(pixels),
                        read,
                        picture: picture.clone(),
                        _held: held,
                        used: now,
                    });
                }
                (*offset, picture)
            })
            .collect();
        // A frame nobody holds any more will never be asked for again.
        self.kept.retain(|k| k.pixels.strong_count() > 0);
        let room = frames.len() * 2;
        if self.kept.len() > room {
            self.kept.sort_by_key(|k| k.used);
            let drop = self.kept.len() - room;
            self.kept.drain(..drop);
        }
        out
    }
}
