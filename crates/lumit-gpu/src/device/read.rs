//! Read-backs in the middle of a frame, in the browser build.
//!
//! A few effects copy a picture or a buffer off the card and carry on with it
//! on the processor. The desktop waits for the copy. In the browser it lands
//! only once the page's event loop runs, after the frame is done, so a frame
//! there takes the last copy that landed for the same place and asks for a new
//! one. When that lands with a different answer the picture is worth drawing
//! again, which the worker does on an idle turn, as it does once a Lens flare
//! has baked ([`landed`]).

use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::sync::mpsc;

/// The most places kept at once, each with its last answer.
const PLACES: usize = 8;

struct Place {
    key: u64,
    landed: Option<Vec<u8>>,
    copy: Option<Underway>,
}

/// A copy on its way: the buffer, its map, and how its rows are laid out.
struct Underway {
    buffer: wgpu::Buffer,
    done: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    /// The bytes of a row the caller wants, and of a row in the buffer.
    row: usize,
    stride: usize,
}

thread_local! {
    static PLACES_HELD: RefCell<Vec<Place>> = const { RefCell::new(Vec::new()) };
    static LANDED: Cell<u64> = const { Cell::new(0) };
    static STOOD_IN: Cell<u64> = const { Cell::new(0) };
}

/// The bytes of `buffer`, which the caller has copied into and submitted, as
/// the last copy for `key` left them. `None` before one has landed.
pub fn buffer(key: impl Hash, buffer: &wgpu::Buffer) -> Option<Vec<u8>> {
    let size = buffer.size() as usize;
    read(key, || (buffer.clone(), size, size))
}

/// `tex`'s texels, row after row with nothing between, as the last copy for
/// `key` left them. `None` before one has landed.
pub fn texture(ctx: &crate::GpuContext, key: impl Hash, tex: &wgpu::Texture) -> Option<Vec<u8>> {
    read(key, || {
        let size = tex.size();
        let row = size.width * tex.format().block_copy_size(None).unwrap_or(4);
        let stride =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read-in-place"),
            size: u64::from(stride) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        // What is read may still be in the frame's batch.
        ctx.flush();
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        ctx.submit([encoder.finish()]);
        (buffer, row as usize, stride as usize)
    })
}

/// How many copies have landed with an answer their place did not have.
pub fn landed() -> u64 {
    collect();
    LANDED.with(Cell::get)
}

/// How many times a frame has taken an earlier copy than its own.
pub fn stand_ins() -> u64 {
    STOOD_IN.with(Cell::get)
}

/// The last answer at `key`, starting a new copy there unless one is still
/// on its way. Only one at a time, so a place never piles copies up.
fn read(key: impl Hash, start: impl FnOnce() -> (wgpu::Buffer, usize, usize)) -> Option<Vec<u8>> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    let key = hasher.finish();
    collect();
    STOOD_IN.with(|n| n.set(n.get() + 1));
    PLACES_HELD.with(|held| {
        let mut held = held.borrow_mut();
        let at = match held.iter().position(|place| place.key == key) {
            Some(at) => at,
            None => {
                if held.len() >= PLACES {
                    held.remove(0);
                }
                held.push(Place {
                    key,
                    landed: None,
                    copy: None,
                });
                held.len() - 1
            }
        };
        let place = held.get_mut(at)?;
        if place.copy.is_none() {
            let (buffer, row, stride) = start();
            let (tx, done) = mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |r| _ = tx.send(r));
            place.copy = Some(Underway {
                buffer,
                done,
                row,
                stride,
            });
        }
        place.landed.clone()
    })
}

/// Take in every copy that has landed. A failed one is let go, and the next
/// frame asks again.
fn collect() {
    PLACES_HELD.with(|held| {
        for place in held.borrow_mut().iter_mut() {
            let Some(copy) = &place.copy else {
                continue;
            };
            match copy.done.try_recv() {
                Err(mpsc::TryRecvError::Empty) => continue,
                Ok(Ok(())) => {
                    let data = copy.buffer.slice(..).get_mapped_range();
                    let bytes: Vec<u8> = data
                        .chunks(copy.stride.max(1))
                        .flat_map(|row| row.get(..copy.row).unwrap_or(row))
                        .copied()
                        .collect();
                    drop(data);
                    copy.buffer.unmap();
                    if place.landed.as_ref() != Some(&bytes) {
                        place.landed = Some(bytes);
                        LANDED.with(|n| n.set(n.get() + 1));
                    }
                }
                Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {}
            }
            place.copy = None;
        }
    });
}
