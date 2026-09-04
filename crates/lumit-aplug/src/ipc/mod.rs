//! Out-of-process hosting: the plugin runs in another program.
//!
//! # In plain terms
//!
//! Everything else in this crate is the conversation with an audio plugin.
//! This module is what moves that conversation into a second process, so that a
//! plugin that crashes, hangs or eats all the memory takes down a program
//! nobody was editing in (docs/impl/audio-plugins.md §5).
//!
//! There are two channels, because sound and sentences want different things:
//!
//! * [`pipe`] — the **control plane**. A duplex pipe carrying length-prefixed
//!   `bincode` messages ([`proto`]): describe yourself, make an instance, here
//!   is a block, save yourself.
//! * [`ring`] — the **block plane**. One block of shared memory per module, cut
//!   into slots, so a block of sound is written once and read where it lies.
//!
//! A block is four kilobytes — 512 frames of interleaved stereo float — so the
//! ring here is a small thing, unlike the video host's, where a slot is a whole
//! 4K frame. It exists all the same: sending four kilobytes down a pipe every
//! ten milliseconds, per layer, is a copy and a serialisation nobody needs to
//! pay for.
//!
//! [`broker`] is the host's side of both: it starts the second process, gives
//! every action a deadline, restarts one that dies, and gives up on a plugin
//! that fails three times running. The second process itself is the
//! `lumit-aplug-broker` crate, which is this crate with a pipe in front of it.
//!
//! **The architecture is the OFX host's, the code is not shared.** `lumit-ofx`
//! proved this shape and its lessons are carried over verbatim as rules, but
//! the messages here carry blocks of sound and parameter events, which have
//! nothing in common with frames and clips: one crate for both would be an
//! abstraction with one and a half users (docs/impl/audio-plugins.md §5).

pub mod broker;
pub mod handles;
pub mod pipe;
pub mod proto;
pub mod ring;
