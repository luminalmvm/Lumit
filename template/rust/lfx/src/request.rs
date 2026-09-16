//! One render request: the values, the geometry, and the walk over the region.
//!
//! # In plain terms
//!
//! Everything an effect is told about one frame arrives in a single struct
//! that is valid only for the length of the call. This module is that struct
//! read the way the header says to read it, which is two rules an author
//! should not have to rediscover:
//!
//! **The value array is walked by the host's own stride**, never by this
//! build's `size_of`. `lfx_value` is the one struct in the ABI with no size
//! prefix, because values cross as a dense array addressed by index; a plugin
//! that strided by its own idea of the element size after the struct grew
//! would read correct-looking kind tags over silently wrong values, and no
//! check at run time can see that.
//!
//! **The buffer is the definition and the region is inside it.** A frame
//! carries its own top-left corner, so the first requested pixel sits at
//! `roi_x0 - origin_x`. Anything that is a function of *where* a pixel is -
//! a vignette's centre, a gradient's ramp - belongs in [`Request::definition`],
//! not in [`Request::region`]: computing it from the region gives every tile
//! its own geometry, and the seams show only on the machines that tiled.
//!
//! # Thread role and contract
//!
//! **Any worker thread.** Two instances of one effect may be inside `process`
//! at once; one instance is never re-entered. Nothing here - the request, its
//! values, either frame - outlives the call that carried it, so a pointer kept
//! past the return names a buffer the host has since given to something else.

use std::marker::PhantomData;

use crate::halves::{from_half, to_half};
use crate::{sys, Status};

/// A rectangle in the space the region and the frames' origins share:
/// `x0`/`y0` inclusive, `x1`/`y1` exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    /// The left edge, inclusive.
    pub x0: i32,
    /// The top edge, inclusive.
    pub y0: i32,
    /// The right edge, exclusive.
    pub x1: i32,
    /// The bottom edge, exclusive.
    pub y1: i32,
}

impl Rect {
    /// How many pixels across.
    #[must_use]
    pub const fn width(&self) -> i32 {
        self.x1 - self.x0
    }

    /// How many pixels down.
    #[must_use]
    pub const fn height(&self) -> i32 {
        self.y1 - self.y0
    }
}

/// The dense value array, read the one way the header admits.
///
/// Every getter takes the element's index - its position among the
/// declarations that carry a value, counting a point once and a group not at
/// all - and the number to answer with when the element is not there or is not
/// of the kind asked for. There is no panicking getter: an effect that cannot
/// find its own control has been handed something it did not declare, and
/// carrying on with the declared default is what the host does too.
#[derive(Clone, Copy)]
pub struct Values<'a> {
    base: *const u8,
    stride: u32,
    count: u32,
    life: PhantomData<&'a sys::LfxValue>,
}

impl<'a> Values<'a> {
    fn of(call: &'a sys::LfxProcess) -> Self {
        Self {
            base: call.values.cast::<u8>(),
            stride: call.value_stride,
            count: call.value_count,
            life: PhantomData,
        }
    }

    /// How many elements the host wrote.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.count
    }

    /// Whether the effect declared nothing that carries a value.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The element at `index`, or `None`.
    ///
    /// `param` is the element's own index, carried so a plugin walking a
    /// strided array can check that it walked it correctly; a mismatch is this
    /// build having strode wrongly rather than the host having filled the
    /// array in wrongly.
    #[must_use]
    pub fn at(&self, index: u32) -> Option<sys::LfxValue> {
        if self.base.is_null() || self.stride == 0 || index >= self.count {
            return None;
        }
        let offset = (index as usize).checked_mul(self.stride as usize)?;
        // SAFETY: `index` is below the count the host declared and the stride
        // is the host's own, so this element is inside the array the host
        // wrote - and the header guarantees both the base and the stride are
        // aligned for the element, so the read is an aligned one.
        let element = unsafe { self.base.add(offset).cast::<sys::LfxValue>().read() };
        (element.param == index).then_some(element)
    }

    /// A Float, a Slider or an Angle.
    #[must_use]
    pub fn number(&self, index: u32, spare: f64) -> f64 {
        match self.at(index) {
            Some(value)
                if matches!(
                    value.kind,
                    sys::LFX_PARAM_FLOAT | sys::LFX_PARAM_SLIDER | sys::LFX_PARAM_ANGLE
                ) =>
            {
                // SAFETY: the kind tag says which arm of the union to read,
                // which is what makes a kind mismatch impossible here rather
                // than a status at run time.
                unsafe { value.v.f }
            }
            _ => spare,
        }
    }

    /// An Int or a Seed.
    #[must_use]
    pub fn whole(&self, index: u32, spare: i64) -> i64 {
        match self.at(index) {
            Some(value) if matches!(value.kind, sys::LFX_PARAM_INT | sys::LFX_PARAM_SEED) => {
                // SAFETY: the kind tag says which arm to read.
                unsafe { value.v.i }
            }
            _ => spare,
        }
    }

    /// A switch.
    #[must_use]
    pub fn flag(&self, index: u32, spare: bool) -> bool {
        match self.at(index) {
            // SAFETY: the kind tag says which arm to read.
            Some(value) if value.kind == sys::LFX_PARAM_BOOL => unsafe { value.v.b },
            _ => spare,
        }
    }

    /// The index a dropdown is on. It may be one this build has never heard of -
    /// a project saved by a later version of the same plugin - so the answer
    /// is matched rather than indexed with.
    #[must_use]
    pub fn chosen(&self, index: u32, spare: u32) -> u32 {
        match self.at(index) {
            // SAFETY: the kind tag says which arm to read.
            Some(value) if value.kind == sys::LFX_PARAM_CHOICE => unsafe { value.v.choice },
            _ => spare,
        }
    }

    /// Scene-linear RGBA. Declared wide and arrived narrow: a declaration is
    /// read once and precision there costs nothing, but the host's resolved
    /// value bag is single precision.
    #[must_use]
    pub fn colour(&self, index: u32, spare: [f32; 4]) -> [f32; 4] {
        match self.at(index) {
            // SAFETY: the kind tag says which arm to read.
            Some(value) if value.kind == sys::LFX_PARAM_COLOUR => unsafe { value.v.rgba },
            _ => spare,
        }
    }

    /// **Both** axes of a point: one declaration is one element, however many
    /// rows the panel folded it into.
    #[must_use]
    pub fn point(&self, index: u32, spare: [f32; 2]) -> [f32; 2] {
        match self.at(index) {
            // SAFETY: the kind tag says which arm to read.
            Some(value) if value.kind == sys::LFX_PARAM_POINT2 => unsafe { value.v.xy },
            _ => spare,
        }
    }

    /// All three axes of a 3D point.
    #[must_use]
    pub fn point3(&self, index: u32, spare: [f32; 3]) -> [f32; 3] {
        match self.at(index) {
            // SAFETY: the kind tag says which arm to read.
            Some(value) if value.kind == sys::LFX_PARAM_POINT3 => unsafe { value.v.xyz },
            _ => spare,
        }
    }
}

/// One request to render one frame.
pub struct Request<'a> {
    call: &'a sys::LfxProcess,
}

impl<'a> Request<'a> {
    /// Wrap a request the host handed over.
    pub(crate) const fn new(call: &'a sys::LfxProcess) -> Self {
        Self { call }
    }

    /// The frozen struct itself, for anything this wrapper does not cover.
    #[must_use]
    pub const fn raw(&self) -> &sys::LfxProcess {
        self.call
    }

    /// The comp frame being rendered, as a decimal.
    #[must_use]
    pub const fn time(&self) -> f64 {
        self.call.time
    }

    /// The resolved controls, in declaration order.
    #[must_use]
    pub fn values(&self) -> Values<'a> {
        Values::of(self.call)
    }

    /// The region asked for.
    #[must_use]
    pub const fn region(&self) -> Rect {
        Rect {
            x0: self.call.roi_x0,
            y0: self.call.roi_y0,
            x1: self.call.roi_x1,
            y1: self.call.roi_y1,
        }
    }

    /// **The buffer**, which is the input's definition and not the region.
    #[must_use]
    pub const fn definition(&self) -> Rect {
        Rect {
            x0: self.call.dod_x0,
            y0: self.call.dod_y0,
            x1: self.call.dod_x1,
            y1: self.call.dod_y1,
        }
    }

    /// Whether the host has stopped wanting this frame.
    ///
    /// Worth asking only if the effect declared [`crate::Traits::cancellable`];
    /// the honest answer to a `true` is [`Status::Cancelled`], promptly.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        match self.call.cancelled {
            // SAFETY: the host's own function, handed the request it belongs
            // to, called while the host is still waiting for this frame.
            Some(ask) => unsafe { ask(std::ptr::from_ref(self.call)) },
            None => false,
        }
    }

    /// Walk the region, handing each pixel to `shade` and writing back what it
    /// answers.
    ///
    /// `shade` is called as `shade(x, y, [r, g, b, a])` with `x` and `y` in the
    /// space [`Self::definition`] is given in. Both depths are handled here so
    /// the maths is written once, in `f32`, which is the whole of what "every
    /// colour depth is mandatory" costs an author. Cancellation is polled once
    /// a row.
    ///
    /// Only the region is written. What the host does with the rest of the
    /// buffer is its own business, and writing outside the region asked for is
    /// how one tile scribbles over another.
    pub fn for_each_pixel<F>(&self, mut shade: F) -> Status
    where
        F: FnMut(i32, i32, [f32; 4]) -> [f32; 4],
    {
        if self.call.input.is_null() || self.call.output.is_null() {
            return Status::Failed;
        }
        // SAFETY: the host's contract: both frames are valid for the call.
        let (input, output) = unsafe { (&*self.call.input, &mut *self.call.output) };
        if input.data.is_null() || output.data.is_null() {
            return Status::Failed;
        }
        // The host sends the project's depth and never converts to accommodate
        // a plugin, so three formats that disagree is a request this build
        // cannot honour rather than one to guess at.
        if input.format != self.call.pixel_format || output.format != self.call.pixel_format {
            return Status::Unsupported;
        }
        let sample_bytes = match self.call.pixel_format {
            sys::LFX_RGBA_F32 => size_of::<f32>(),
            sys::LFX_RGBA_F16 => size_of::<u16>(),
            _ => return Status::Unsupported,
        };

        let wanted = self.region();
        let x0 = wanted.x0.max(input.origin_x);
        let y0 = wanted.y0.max(input.origin_y);
        let x1 = wanted
            .x1
            .min(input.origin_x.saturating_add(input.width as i32));
        let y1 = wanted
            .y1
            .min(input.origin_y.saturating_add(input.height as i32));

        for y in y0..y1 {
            if self.cancelled() {
                return Status::Cancelled;
            }
            let from_row = (y - input.origin_y) as usize * input.row_bytes as usize;
            let to_row = (y - output.origin_y) as usize * output.row_bytes as usize;
            // SAFETY: `y` is inside the input's own height and the row stride
            // is the frame's own, so both offsets are inside the buffers the
            // host handed over. The output is the same size as the input:
            // `regions_agree` on the host's side is what holds that.
            let (source, destination) = unsafe {
                (
                    input.data.cast::<u8>().add(from_row).cast_const(),
                    output.data.cast::<u8>().add(to_row),
                )
            };
            for x in x0..x1 {
                let from = (x - input.origin_x) as usize * 4 * sample_bytes;
                let to = (x - output.origin_x) as usize * 4 * sample_bytes;
                // SAFETY: `x` is inside the frames' own width, so four samples
                // at `from` and at `to` are inside the same buffers.
                unsafe {
                    if sample_bytes == size_of::<f32>() {
                        let read = source.add(from).cast::<f32>();
                        let write = destination.add(to).cast::<f32>();
                        let painted = shade(
                            x,
                            y,
                            [
                                read.read(),
                                read.add(1).read(),
                                read.add(2).read(),
                                read.add(3).read(),
                            ],
                        );
                        for (channel, sample) in painted.iter().enumerate() {
                            write.add(channel).write(*sample);
                        }
                    } else {
                        let read = source.add(from).cast::<u16>();
                        let write = destination.add(to).cast::<u16>();
                        let painted = shade(
                            x,
                            y,
                            [
                                from_half(read.read()),
                                from_half(read.add(1).read()),
                                from_half(read.add(2).read()),
                                from_half(read.add(3).read()),
                            ],
                        );
                        for (channel, sample) in painted.iter().enumerate() {
                            write.add(channel).write(to_half(*sample));
                        }
                    }
                }
            }
        }
        Status::Ok
    }
}
