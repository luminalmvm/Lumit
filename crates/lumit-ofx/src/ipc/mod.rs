//! Out-of-process hosting: the plugin runs in another program.
//!
//! # In plain terms
//!
//! Everything else in this crate is the conversation with a plugin. This module
//! is what moves that conversation into a second process, so that a plugin that
//! crashes, hangs or eats all the memory takes down a program nobody was
//! editing in (docs/12 §1, §2.3).
//!
//! There are two channels, because frames and sentences want different things:
//!
//! * [`pipe`] — the **control plane**. A duplex pipe carrying length-prefixed
//!   `bincode` messages ([`proto`]): describe yourself, make an instance, here
//!   are the values, render this frame.
//! * [`shm`] — the **frame plane**. One block of shared memory per bundle, cut
//!   into slots, so a picture is written once and read where it lies. The pipe
//!   carries the slot number, never the pixels.
//!
//! [`broker`] is the host's side of both: it starts the second process, gives
//! every action a deadline, restarts one that dies, and gives up on a plugin
//! that fails three times running. The second process itself is the
//! `lumit-ofx-broker` crate, which is this crate with a pipe in front of it.
//!
//! **The transport is shared with the other hosts; nothing else is.** The pipe,
//! where the broker executable is, and the handful of numbers every host must
//! answer the same way live in `lumit-ipc`; [`pipe`] is that module wearing this
//! host's name, and [`identity`] holds the three strings that are this host's
//! own and must not be anybody else's (docs/impl/lfx.md §3.1). [`proto`] and
//! [`shm`] stay here, because a picture and a block of sound share nothing.

pub mod broker;
pub mod identity;
pub mod pipe;
pub mod proto;
pub mod shm;
