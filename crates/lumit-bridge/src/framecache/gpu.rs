use std::sync::atomic::{AtomicU64, Ordering};

static ALLOCATED: AtomicU64 = AtomicU64::new(0);
static RESERVED: AtomicU64 = AtomicU64::new(0);
static TEXTURES: AtomicU64 = AtomicU64::new(0);
static BUFFERS: AtomicU64 = AtomicU64::new(0);
/// Whether the card draws from this process's own memory.
static UNIFIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn publish_unified(unified: bool) {
    UNIFIED.store(unified, Ordering::Relaxed);
}

pub(crate) fn unified() -> bool {
    UNIFIED.load(Ordering::Relaxed)
}

pub(crate) fn publish(allocated: u64, reserved: u64, textures: u64, buffers: u64) {
    ALLOCATED.store(allocated, Ordering::Relaxed);
    RESERVED.store(reserved, Ordering::Relaxed);
    TEXTURES.store(textures, Ordering::Relaxed);
    BUFFERS.store(buffers, Ordering::Relaxed);
}

/// `(allocated_bytes, reserved_bytes, textures, buffers)`. The two byte
/// figures are 0 on a backend that keeps no allocator accounting (Metal);
/// the two counts are kept by every backend.
pub(crate) fn stats() -> (u64, u64, u64, u64) {
    (
        ALLOCATED.load(Ordering::Relaxed),
        RESERVED.load(Ordering::Relaxed),
        TEXTURES.load(Ordering::Relaxed),
        BUFFERS.load(Ordering::Relaxed),
    )
}
