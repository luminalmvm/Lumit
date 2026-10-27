//! Metal, on macOS. Images go to a plugin as `id<MTLBuffer>` in shared storage,
//! so the upload and the download are plain copies into the buffer's contents.
//!
//! Broker main thread only, like everything in [`crate::gpu`].

use std::collections::BTreeMap;

use metal::foreign_types::ForeignType;
use metal::objc::rc::autoreleasepool;
use metal::{Buffer, CommandQueue, Device, MTLResourceOptions};
use parking_lot::Mutex;

/// One device, with a command queue of our own.
pub(crate) struct Metal {
    device: Device,
    queue: CommandQueue,
    /// The buffers handed out, by the address the plugin is given. Holding
    /// them here is what keeps them alive until [`Metal::free`].
    buffers: Mutex<BTreeMap<usize, Buffer>>,
}

// SAFETY: Metal's device, queue and buffers may be used from any thread,
// which is what Apple documents for them; metal-rs only does not say so. The
// broker uses them from one, and the buffer map is behind its own lock.
unsafe impl Send for Metal {}
// SAFETY: as above.
unsafe impl Sync for Metal {}

impl Metal {
    /// Whether this Mac has a Metal device.
    pub(crate) fn present() -> bool {
        Device::system_default().is_some()
    }

    /// Open the device whose name Lumit's adapter starts with, or the default.
    pub(crate) fn open(adapter: Option<&str>) -> Option<Self> {
        let device = adapter
            .and_then(|summary| {
                Device::all()
                    .into_iter()
                    .find(|device| super::names_card(summary, device.name()))
            })
            .or_else(Device::system_default)?;
        let queue = device.new_command_queue();
        Some(Self {
            device,
            queue,
            buffers: Mutex::new(BTreeMap::new()),
        })
    }

    /// `kOfxImageEffectPropMetalCommandQueue`.
    pub(crate) fn queue(&self) -> usize {
        self.queue.as_ptr() as usize
    }

    pub(crate) fn alloc(&self, bytes: usize) -> Result<usize, String> {
        let buffer = self
            .device
            .new_buffer(bytes.max(1) as u64, MTLResourceOptions::StorageModeShared);
        let at = buffer.as_ptr() as usize;
        if at == 0 {
            return Err("Metal gave no buffer".to_owned());
        }
        self.buffers.lock().insert(at, buffer);
        Ok(at)
    }

    pub(crate) fn upload(&self, at: usize, bytes: &[u8]) -> Result<(), String> {
        let buffers = self.buffers.lock();
        let buffer = buffers
            .get(&at)
            .ok_or_else(|| "Metal has no such buffer".to_owned())?;
        if (buffer.length() as usize) < bytes.len() {
            return Err("the Metal buffer is short".to_owned());
        }
        // SAFETY: shared storage is host memory at least this long, and the
        // card is not using it: nothing has been queued on it yet.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.contents().cast(), bytes.len());
        }
        Ok(())
    }

    pub(crate) fn download(&self, at: usize, into: &mut [u8]) -> Result<(), String> {
        let buffers = self.buffers.lock();
        let buffer = buffers
            .get(&at)
            .ok_or_else(|| "Metal has no such buffer".to_owned())?;
        if (buffer.length() as usize) < into.len() {
            return Err("the Metal buffer is short".to_owned());
        }
        // SAFETY: as above; `finish` has waited for the card to be done with it.
        unsafe {
            std::ptr::copy_nonoverlapping(
                buffer.contents().cast::<u8>(),
                into.as_mut_ptr(),
                into.len(),
            );
        }
        Ok(())
    }

    pub(crate) fn free(&self, at: usize) {
        self.buffers.lock().remove(&at);
    }

    /// Wait for everything the plugin queued: a queue runs its command buffers
    /// in order, so an empty one committed last finishes last.
    pub(crate) fn finish(&self) -> Result<(), String> {
        // In a pool of its own, as the broker's thread has none to collect it.
        autoreleasepool(|| {
            let last = self.queue.new_command_buffer();
            last.commit();
            last.wait_until_completed();
        });
        Ok(())
    }
}
