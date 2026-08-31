//! The block: fixed-size sound, in and out, and the events that ride with it.
//!
//! # In plain terms
//!
//! Plugins do not take a whole song. They take a short, fixed run of samples —
//! Lumit's is **512 frames at 48 kHz**, about eleven milliseconds — and hand
//! back the same length, processed. Everything about the block is fixed so that
//! two exports of the same project produce identical sound: the size never
//! varies with the playhead, and a layer's chain always starts counting from
//! that layer's first sample (docs/impl/audio-plugins.md §3).
//!
//! Two shapes of the same sound meet here. Lumit carries stereo **interleaved**
//! — left, right, left, right — because that is what sound cards and files
//! want. Both plugin standards want **planar**: one array of lefts, one array
//! of rights. So the boundary de-interleaves on the way in and re-interleaves
//! on the way out, into buffers that are allocated once and never grow
//! (docs/14's budgeted allocations).
//!
//! **Never in place.** The input and output buffers are separate, always. CLAP
//! lets a plugin say it can work in place, and that flag is where plugin bugs
//! live: a plugin that overwrites its input halfway through and then reads it
//! again produces sound that is wrong in a way nobody can hear until the mix is
//! finished.
//!
//! # Denormals
//!
//! A denormal is a floating-point number so close to zero the processor stops
//! being fast about it — a reverb tail decaying into silence generates
//! thousands of them, and on some machines each one costs a hundred times what
//! a normal number costs. Both plugin standards assume the host has switched
//! denormals off before calling `process`, so [`Denormals`] does that and puts
//! the setting back afterwards.

use clap_sys::events::{
    clap_event_header, clap_event_param_value, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE,
};
use serde::{Deserialize, Serialize};

/// Frames in one block. Fixed (docs/impl/audio-plugins.md §3): a block
/// boundary is a fact about the layer, not about the playhead.
pub const BLOCK_FRAMES: usize = 512;

/// The session rate (docs/09 §2).
pub const SAMPLE_RATE: f64 = 48_000.0;

/// Channels. v1 hosts stereo effect plugins only (§4).
pub const CHANNELS: usize = 2;

/// Samples in one interleaved block: every frame's channels, back to back.
pub const INTERLEAVED_LEN: usize = BLOCK_FRAMES * CHANNELS;

/// One parameter value, at one moment inside a block.
///
/// `time` is the frame offset within the block, which is how CLAP places an
/// event; a value baked at the block's start has `time` nought, which is what
/// the envelope precedent (K-172) delivers.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParamEvent {
    /// Frames from the start of the block.
    pub time: u32,
    /// The plugin's own stable parameter id — never an index (§4).
    pub id: u32,
    /// The plain value, in the plugin's own units. CLAP traffics plain values;
    /// only VST3 normalises (§9).
    pub value: f64,
}

/// One block's buffers and events, allocated once.
///
/// Reused across every block of a chain: `load` fills the input, `process`
/// fills the output, `store` takes it away. Nothing here allocates after
/// [`Block::new`].
pub struct Block {
    /// Planar input: channel 0's frames, then channel 1's.
    input: Vec<f32>,
    /// Planar output, same layout.
    output: Vec<f32>,
    /// The events for this block, time-sorted. CLAP calls an unsorted list
    /// undefined, and real plugins crash on one (§9).
    events: Vec<clap_event_param_value>,
}

impl Default for Block {
    fn default() -> Self {
        Self::new()
    }
}

impl Block {
    /// Every buffer this block will ever need.
    #[must_use]
    pub fn new() -> Self {
        Self {
            input: vec![0.0; BLOCK_FRAMES * CHANNELS],
            output: vec![0.0; BLOCK_FRAMES * CHANNELS],
            // A block's events are one per automated parameter at this control
            // rate; sixteen covers every plugin worth automating and the Vec
            // grows once if one ever exceeds it.
            events: Vec::with_capacity(16),
        }
    }

    /// Fill the input from Lumit's interleaved stereo.
    ///
    /// A `src` shorter than a whole block leaves the rest silent, which is what
    /// the last block of a layer is.
    pub fn load(&mut self, src: &[f32]) {
        self.input.fill(0.0);
        for (frame, samples) in src.chunks_exact(CHANNELS).take(BLOCK_FRAMES).enumerate() {
            for (channel, sample) in samples.iter().enumerate() {
                if let Some(slot) = self.input.get_mut(channel * BLOCK_FRAMES + frame) {
                    *slot = *sample;
                }
            }
        }
    }

    /// Write the output back out as interleaved stereo.
    pub fn store(&self, dst: &mut [f32]) {
        for (frame, samples) in dst
            .chunks_exact_mut(CHANNELS)
            .take(BLOCK_FRAMES)
            .enumerate()
        {
            for (channel, sample) in samples.iter_mut().enumerate() {
                *sample = self
                    .output
                    .get(channel * BLOCK_FRAMES + frame)
                    .copied()
                    .unwrap_or(0.0);
            }
        }
    }

    /// The events this block carries, **sorted by time** — the sort is here
    /// rather than at the call site so no caller can forget it.
    ///
    /// The sort is stable, so two events at the same frame keep the order they
    /// were given in, which is the order the chain baked them.
    pub fn set_events(&mut self, events: &[ParamEvent]) {
        self.events.clear();
        self.events.extend(events.iter().map(to_clap));
        self.events.sort_by_key(|event| event.header.time);
    }

    /// The input, planar, as the plugin will see it.
    #[must_use]
    pub fn input(&self) -> &[f32] {
        &self.input
    }

    /// The output, planar.
    #[must_use]
    pub fn output(&self) -> &[f32] {
        &self.output
    }

    /// Everything the boundary needs at once: the two planar buffers and the
    /// sorted events. One method rather than three because the buffers are
    /// handed over as `*mut` and the events as `*const` **in the same call**,
    /// and three borrows of one block cannot be taken separately.
    pub(crate) fn parts(&mut self) -> (&mut [f32], &mut [f32], &[clap_event_param_value]) {
        (&mut self.input, &mut self.output, &self.events)
    }

    /// The two planar buffers, for a boundary that carries its events some other
    /// way — VST3's ride in a queue object rather than in the call (K-707).
    pub(crate) fn planes(&mut self) -> (&mut [f32], &mut [f32]) {
        (&mut self.input, &mut self.output)
    }
}

/// One Lumit parameter event, in CLAP's own shape.
pub(crate) fn to_clap(event: &ParamEvent) -> clap_event_param_value {
    clap_event_param_value {
        header: clap_event_header {
            size: u32::try_from(size_of::<clap_event_param_value>()).unwrap_or(0),
            time: event
                .time
                .min(u32::try_from(BLOCK_FRAMES).unwrap_or(u32::MAX)),
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: CLAP_EVENT_PARAM_VALUE,
            flags: 0,
        },
        param_id: event.id,
        cookie: std::ptr::null_mut(),
        // Not a per-note event: the whole port, every key, every channel,
        // which is what an automated effect parameter is.
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value: event.value,
    }
}

// ------------------------------------------------------------ event lists --

/// The `clap_input_events` a block's events are read through.
///
/// Built fresh for each `process` call and living on that call's stack: the
/// list holds a pointer to `slot`, and `slot` holds the events, which belong to
/// the block. Both must outlive the call — CLAP reads the list from inside
/// `process` and never after it.
pub(crate) fn input_events(slot: &mut &[clap_event_param_value]) -> clap_input_events {
    clap_input_events {
        ctx: std::ptr::from_mut(slot).cast(),
        size: Some(list_size),
        get: Some(list_get),
    }
}

/// # Safety
///
/// `list.ctx` must be the `&[clap_event_param_value]` [`input_events`] put
/// there.
unsafe extern "C" fn list_size(list: *const clap_input_events) -> u32 {
    if list.is_null() {
        return 0;
    }
    // SAFETY: the host built this list and set `ctx` from a live slice.
    let events = unsafe { &*(*list).ctx.cast::<&[clap_event_param_value]>() };
    u32::try_from(events.len()).unwrap_or(0)
}

/// # Safety
///
/// As [`list_size`].
unsafe extern "C" fn list_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    if list.is_null() {
        return std::ptr::null();
    }
    // SAFETY: as `list_size`.
    let events = unsafe { &*(*list).ctx.cast::<&[clap_event_param_value]>() };
    match events.get(index as usize) {
        Some(event) => std::ptr::addr_of!(event.header),
        None => std::ptr::null(),
    }
}

/// The `clap_output_events` a plugin writes its own gestures into.
///
/// v1 has nowhere to put them — the plugin's editor window is the follow-on
/// package (§6) and nothing else changes a parameter from the plugin's side —
/// so the sink accepts and drops. It must exist: CLAP requires a non-null
/// output list, and a plugin that finds none is a plugin that crashes.
pub(crate) fn output_events() -> clap_output_events {
    clap_output_events {
        ctx: std::ptr::null_mut(),
        try_push: Some(sink_push),
    }
}

unsafe extern "C" fn sink_push(
    _list: *const clap_output_events,
    _event: *const clap_event_header,
) -> bool {
    true
}

// -------------------------------------------------------------- denormals --

/// Flush-to-zero and denormals-are-zero, for as long as this value lives.
///
/// Both plugin standards assume the host has set them; a reverb tail hitting
/// denormals is the classic mystery CPU spike (§3). The previous setting is
/// restored on drop, so a thread that borrowed the processing role gives it
/// back exactly as it found it.
pub struct Denormals {
    /// The MXCSR word as it was, or `None` where this architecture has no such
    /// word to save.
    previous: Option<u32>,
}

impl Denormals {
    /// Switch them off.
    #[must_use]
    pub fn on() -> Self {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        let previous = {
            /// MXCSR bit 15.
            const FLUSH_TO_ZERO: u32 = 0x8000;
            /// MXCSR bit 6.
            const DENORMALS_ARE_ZERO: u32 = 0x0040;
            let mut csr: u32 = 0;
            // SAFETY: `stmxcsr` writes four bytes to the address given, and
            // `csr` is a live `u32`. The instruction is SSE, which is baseline
            // on every target this crate builds for.
            unsafe { std::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack)) };
            let raised = csr | FLUSH_TO_ZERO | DENORMALS_ARE_ZERO;
            // SAFETY: `ldmxcsr` reads four bytes from the address given.
            unsafe { std::arch::asm!("ldmxcsr [{}]", in(reg) &raised, options(nostack)) };
            Some(csr)
        };
        // ponytail: aarch64 keeps the same flag in FPCR's FZ bit; nothing in
        // this project builds for it yet, so the guard is honestly a no-op
        // there rather than a wrong write.
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        let previous = None;

        Self { previous }
    }
}

impl Drop for Denormals {
    fn drop(&mut self) {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if let Some(csr) = self.previous {
            // SAFETY: as `Denormals::on`.
            unsafe { std::arch::asm!("ldmxcsr [{}]", in(reg) &csr, options(nostack)) };
        }
    }
}
