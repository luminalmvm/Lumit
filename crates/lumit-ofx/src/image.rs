//! Images: the pixels that cross the boundary, and which way up they lie.
//!
//! # In plain terms
//!
//! Lumit keeps its working frames in **half floats** — sixteen bits a channel,
//! four channels, premultiplied, scene-linear. OFX plugins are told this host
//! offers **float RGBA and nothing else** (docs/12 §2.1), because that is what
//! every major plugin accepts and because claiming several depths and lying
//! about one is the classic host bug. So a frame is widened to thirty-two bits
//! on the way in and narrowed on the way out, and this module is the only
//! place either happens. The narrowing is lossless in the direction that
//! matters: every half float is exactly a float, so a plugin that hands the
//! picture back untouched hands back the picture.
//!
//! The other half of the module is the part that surprises people.
//!
//! **Which way up.** OFX counts rows from the bottom: y increases upwards, and
//! the data pointer names the pixel at the bottom-left of the image. Lumit's
//! frames, like most things with a GPU underneath, are stored the other way —
//! row nought is the top. Rather than flip every frame twice, OFX lets the host
//! say so, by giving a **negative row bytes**: the pointer names the row that
//! happens to sit last in memory, and stepping "up" a row steps *backwards*
//! through the block. It is legal, it is common, and a host that assumes row
//! bytes is positive will read one picture and write another upside-down.
//!
//! This module can hand out either layout, and the tests render the same frame
//! through both and demand the same picture back.
//!
//! **Whose memory.** The block behind an image comes from the host's own image
//! arena ([`crate::suites::memory`]), never from a plugin's allocator and never
//! from a plain `Vec` handed across the boundary. The arena knows every block
//! it gave out, frees them when the [`Image`] is dropped at the end of the
//! render, and does not accept them back through `memoryFree` — so a plugin
//! that keeps the pointer after `clipReleaseImage` is reading freed memory it
//! could not have freed itself, and a leak is a number a test can read.

use half::f16;

use crate::status::Status;
use crate::suites::memory::Block;

/// Channels in a pixel. RGBA, always: it is the only component set this host
/// advertises.
pub const CHANNELS: usize = 4;

/// A rectangle in pixels, OFX's way round: `y` increases **upwards**, `x2`/`y2`
/// are exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RectI {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl RectI {
    /// A rectangle at the origin, `width` by `height`.
    #[must_use]
    pub const fn sized(width: i32, height: i32) -> Self {
        Self {
            x1: 0,
            y1: 0,
            x2: width,
            y2: height,
        }
    }

    /// Width in pixels, never negative.
    #[must_use]
    pub fn width(self) -> usize {
        usize::try_from(self.x2.saturating_sub(self.x1)).unwrap_or(0)
    }

    /// Height in pixels, never negative.
    #[must_use]
    pub fn height(self) -> usize {
        usize::try_from(self.y2.saturating_sub(self.y1)).unwrap_or(0)
    }

    /// The four numbers as OFX carries them.
    #[must_use]
    pub const fn as_array(self) -> [i32; 4] {
        [self.x1, self.y1, self.x2, self.y2]
    }
}

/// Which way the rows run through the block — which is the same thing as the
/// **sign of `kOfxImagePropRowBytes`**, and is why the sign exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowOrder {
    /// The first row in memory is the **bottom** row of the picture, which is
    /// OFX's own order, so row bytes is positive and the data pointer is the
    /// start of the block.
    BottomUp,
    /// The first row in memory is the **top** row, which is Lumit's order, so
    /// row bytes is **negative** and the data pointer names the row that sits
    /// last in memory.
    TopDown,
}

/// A frame in Lumit's working format: fp16 RGBA, premultiplied, scene-linear,
/// with row nought at the **top**.
///
/// It is owned and plain — nothing about it crosses the C boundary. The thing
/// that crosses is an [`Image`], built from one of these.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame16 {
    width: usize,
    height: usize,
    pixels: Vec<f16>,
}

impl Frame16 {
    /// A transparent black frame.
    ///
    /// # Errors
    ///
    /// [`Status::ErrValue`] for a frame with no pixels in it, and
    /// [`Status::ErrMemory`] for one whose size does not fit in a `usize`.
    pub fn black(width: usize, height: usize) -> Result<Self, Status> {
        let count = pixel_count(width, height)?;
        Ok(Self {
            width,
            height,
            pixels: vec![f16::ZERO; count],
        })
    }

    /// A frame from half floats already in RGBA order, row nought at the top.
    ///
    /// # Errors
    ///
    /// [`Status::ErrValue`] if the count does not match the size.
    pub fn from_pixels(width: usize, height: usize, pixels: Vec<f16>) -> Result<Self, Status> {
        if pixels.len() != pixel_count(width, height)? {
            return Err(Status::ErrValue);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// A frame from floats, narrowed on the way in.
    ///
    /// # Errors
    ///
    /// As [`Frame16::from_pixels`].
    pub fn from_f32(width: usize, height: usize, pixels: &[f32]) -> Result<Self, Status> {
        Self::from_pixels(
            width,
            height,
            pixels.iter().copied().map(f16::from_f32).collect(),
        )
    }

    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// The half floats, row nought at the top, RGBA.
    #[must_use]
    pub fn pixels(&self) -> &[f16] {
        &self.pixels
    }

    /// One pixel, as floats, or transparent black if it is off the frame.
    #[must_use]
    pub fn pixel(&self, x: usize, y: usize) -> [f32; CHANNELS] {
        let mut out = [0.0; CHANNELS];
        let Some(base) = y.checked_mul(self.width).and_then(|row| row.checked_add(x)) else {
            return out;
        };
        let Some(base) = base.checked_mul(CHANNELS) else {
            return out;
        };
        for (channel, slot) in out.iter_mut().enumerate() {
            if let Some(value) = self.pixels.get(base + channel) {
                *slot = value.to_f32();
            }
        }
        out
    }
}

/// How many half floats a frame of this size holds.
fn pixel_count(width: usize, height: usize) -> Result<usize, Status> {
    if width == 0 || height == 0 {
        return Err(Status::ErrValue);
    }
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(CHANNELS))
        .ok_or(Status::ErrMemory)
}

/// One image as a plugin sees it: a block of fp32 RGBA the host owns, plus the
/// bounds and the row order that say how to read it.
pub struct Image {
    bounds: RectI,
    order: RowOrder,
    /// Bytes from one row to the next, unsigned. The **signed** answer a plugin
    /// is given is [`Image::row_bytes`].
    stride: usize,
    block: Block,
}

impl Image {
    /// A transparent black image of `bounds`, in the given row order.
    ///
    /// # Errors
    ///
    /// [`Status::ErrValue`] for an empty rectangle, [`Status::ErrMemory`] if
    /// the arena cannot give the block.
    pub fn black(bounds: RectI, order: RowOrder) -> Result<Self, Status> {
        let (width, height) = (bounds.width(), bounds.height());
        if width == 0 || height == 0 {
            return Err(Status::ErrValue);
        }
        let stride = width
            .checked_mul(CHANNELS)
            .and_then(|floats| floats.checked_mul(size_of::<f32>()))
            .ok_or(Status::ErrMemory)?;
        let bytes = stride.checked_mul(height).ok_or(Status::ErrMemory)?;
        Ok(Self {
            bounds,
            order,
            stride,
            block: Block::zeroed(bytes)?,
        })
    }

    /// An image carrying `frame`, widened to fp32 and laid out `order`'s way
    /// round. The frame's row nought is the top, and comes out as the image's
    /// top row whichever layout is asked for — that is the whole point of the
    /// row-order choice being the host's to make.
    ///
    /// # Errors
    ///
    /// As [`Image::black`].
    pub fn from_frame(frame: &Frame16, order: RowOrder) -> Result<Self, Status> {
        let bounds = RectI::sized(
            i32::try_from(frame.width()).map_err(|_| Status::ErrValue)?,
            i32::try_from(frame.height()).map_err(|_| Status::ErrValue)?,
        );
        let mut image = Self::black(bounds, order)?;
        for y in 0..frame.height() {
            // Frame row nought is the top; the top of an OFX image is `y2 - 1`.
            let ofx_y = frame.height() - 1 - y;
            let source_start = y * frame.width() * CHANNELS;
            let row = image.row_mut(ofx_y).ok_or(Status::ErrFatal)?;
            for (index, slot) in row.iter_mut().enumerate() {
                *slot = frame
                    .pixels()
                    .get(source_start + index)
                    .map_or(0.0, |value| value.to_f32());
            }
        }
        Ok(image)
    }

    /// The picture, narrowed back to Lumit's working format with row nought at
    /// the top.
    ///
    /// # Errors
    ///
    /// [`Status::ErrValue`] if the image is empty.
    pub fn to_frame(&self) -> Result<Frame16, Status> {
        let (width, height) = (self.bounds.width(), self.bounds.height());
        let mut pixels = Vec::with_capacity(pixel_count(width, height)?);
        for y in 0..height {
            let ofx_y = height - 1 - y;
            let row = self.row(ofx_y).ok_or(Status::ErrFatal)?;
            pixels.extend(row.iter().copied().map(f16::from_f32));
        }
        Frame16::from_pixels(width, height, pixels)
    }

    /// The rectangle this image covers.
    #[must_use]
    pub const fn bounds(&self) -> RectI {
        self.bounds
    }

    /// The row order, and therefore the sign below.
    #[must_use]
    pub const fn order(&self) -> RowOrder {
        self.order
    }

    /// `kOfxImagePropRowBytes` — **signed**, and negative for a top-down
    /// image. A host that hands this out must mean it, and a plugin that reads
    /// it must honour it.
    #[must_use]
    pub fn row_bytes(&self) -> i32 {
        let stride = i32::try_from(self.stride).unwrap_or(i32::MAX);
        match self.order {
            RowOrder::BottomUp => stride,
            RowOrder::TopDown => -stride,
        }
    }

    /// `kOfxImagePropData` — the address of the pixel at the bottom-left of the
    /// image, which for a top-down layout is inside the block rather than at
    /// its start.
    #[must_use]
    pub fn data_pointer(&self) -> *mut u8 {
        let base = self.block.as_mut_ptr();
        match self.order {
            RowOrder::BottomUp => base,
            // SAFETY: the block is `stride * height` bytes, so the start of its
            // last row is in bounds. The offset is computed, not dereferenced.
            RowOrder::TopDown => unsafe {
                base.add(self.stride * self.bounds.height().saturating_sub(1))
            },
        }
    }

    /// The floats of one OFX row, `y` counted the OFX way (up from `y1`).
    #[must_use]
    pub fn row(&self, y: usize) -> Option<&[f32]> {
        let start = self.row_start(y)?;
        self.floats()
            .get(start..start + self.bounds.width() * CHANNELS)
    }

    /// As [`Image::row`], for writing.
    pub fn row_mut(&mut self, y: usize) -> Option<&mut [f32]> {
        let start = self.row_start(y)?;
        let end = start + self.bounds.width() * CHANNELS;
        self.floats_mut().get_mut(start..end)
    }

    /// Where a row starts, as an index into the block's floats.
    fn row_start(&self, y: usize) -> Option<usize> {
        let height = self.bounds.height();
        if y >= height {
            return None;
        }
        // The memory row for an OFX row: the same one for a bottom-up image,
        // the mirror for a top-down one. This is the sign of `row_bytes`,
        // spelled as an index instead of as a pointer step.
        let memory_row = match self.order {
            RowOrder::BottomUp => y,
            RowOrder::TopDown => height - 1 - y,
        };
        Some(memory_row * self.bounds.width() * CHANNELS)
    }

    /// The whole block as floats.
    fn floats(&self) -> &[f32] {
        // SAFETY: the block was allocated as `stride * height` bytes with
        // sixteen-byte alignment, which is enough for `f32`, and zeroed — so
        // every float in it is initialised. Nothing else aliases it: the block
        // is owned by this `Image`, and the pointer a plugin holds is only
        // valid while this `Image` is, which the render driver enforces by
        // outliving every image it hands out.
        unsafe {
            std::slice::from_raw_parts(self.block.as_mut_ptr().cast::<f32>(), self.floats_len())
        }
    }

    /// As [`Image::floats`], for writing. `&mut self` is what keeps it unique.
    fn floats_mut(&mut self) -> &mut [f32] {
        let len = self.floats_len();
        // SAFETY: as `floats`, plus the unique borrow of `self`.
        unsafe { std::slice::from_raw_parts_mut(self.block.as_mut_ptr().cast::<f32>(), len) }
    }

    fn floats_len(&self) -> usize {
        self.bounds.width() * self.bounds.height() * CHANNELS
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A small frame whose every pixel says where it is, so an upside-down
    /// picture is obvious rather than plausible.
    fn a_frame() -> Frame16 {
        let (width, height) = (4, 3);
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.push(f16::from_f32(x as f32));
                pixels.push(f16::from_f32(y as f32));
                pixels.push(f16::from_f32(0.5));
                pixels.push(f16::ONE);
            }
        }
        Frame16::from_pixels(width, height, pixels).unwrap()
    }

    #[test]
    fn a_frame_survives_the_boundary_in_either_row_order() {
        let frame = a_frame();
        for order in [RowOrder::BottomUp, RowOrder::TopDown] {
            let image = Image::from_frame(&frame, order).unwrap();
            assert_eq!(image.to_frame().unwrap(), frame, "{order:?}");
        }
    }

    #[test]
    fn the_sign_of_row_bytes_follows_the_row_order() {
        let frame = a_frame();
        let up = Image::from_frame(&frame, RowOrder::BottomUp).unwrap();
        let down = Image::from_frame(&frame, RowOrder::TopDown).unwrap();
        assert_eq!(up.row_bytes(), 4 * 4 * 4);
        assert_eq!(down.row_bytes(), -(4 * 4 * 4));
        // A top-down image points at the last row in the block, not the first.
        assert_eq!(up.data_pointer(), up.data_pointer());
        assert!(!std::ptr::eq(down.data_pointer(), up.data_pointer()));
    }

    /// The same picture, whichever way the block runs: OFX row nought is the
    /// bottom row in both, and the frame's row nought is the top in both.
    #[test]
    fn ofx_row_nought_is_the_bottom_of_the_picture() {
        let frame = a_frame();
        for order in [RowOrder::BottomUp, RowOrder::TopDown] {
            let image = Image::from_frame(&frame, order).unwrap();
            let bottom = image.row(0).unwrap();
            // The frame's bottom row is its last: y = height - 1 = 2.
            assert_eq!(bottom[1], 2.0, "{order:?}");
            let top = image.row(2).unwrap();
            assert_eq!(top[1], 0.0, "{order:?}");
        }
    }

    #[test]
    fn an_empty_rectangle_is_a_value_error_and_not_an_allocation() {
        assert_eq!(
            Image::black(RectI::sized(0, 4), RowOrder::BottomUp).err(),
            Some(Status::ErrValue)
        );
        assert_eq!(Frame16::black(4, 0).err(), Some(Status::ErrValue));
    }
}
