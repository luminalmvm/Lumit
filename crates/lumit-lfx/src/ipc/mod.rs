//! Out-of-process hosting: the plugin runs in another program.
//!
//! # In plain terms
//!
//! Everything else in this crate is the conversation with a plugin. This module
//! is what moves that conversation into a second process, so that a plugin
//! which crashes, hangs or eats all the memory takes down a program nobody was
//! editing in (docs/12 §3, docs/impl/lfx.md §3).
//!
//! There are two channels, because frames and sentences want different things:
//!
//! * [`proto`] - the **control plane**. A fixed, small vocabulary of
//!   length-prefixed messages: read this manifest, describe what is in the
//!   module, make an instance, here are the values, render this frame. Tens of
//!   bytes a message, and every one of them answered exactly once.
//! * [`ring`] - the **frame plane**. One block of shared memory per bundle, cut
//!   into slots sized by the depth they carry, so a picture is written once and
//!   read where it lies. The control plane carries the slot number, never the
//!   pixels.
//!
//! **The transport underneath both is shared; neither of these is.** The pipe,
//! where the broker executable is, and the handful of numbers every host must
//! answer the same way live in `lumit-ipc`, and this host's own three identity
//! strings are reserved there against the other two (docs/impl/lfx.md §3.1).
//! What stays here is what is about *pictures*: a block of sound and a frame
//! share nothing, so a common ring would be an abstraction with one and a half
//! users, and `Ready`, `Challenge` and `Hello` are variants of this host's own
//! enum rather than a trait somebody implements. The ordering rules travel as
//! `lumit_ipc::rules`' doc comments, and each host writes its own handshake.
//!
//! The second process itself is `lumit-lfx-broker`. The host's side of both
//! planes is [`broker`]: the spawn, the handshake, the manifest read before any
//! of the plugin's code, the deadlines, the three strikes and the replay. The
//! numbers that name an instance across the pipe are [`handles`]; the transport
//! under all of it is [`pipe`], which is `lumit-ipc`'s wearing this host's
//! [`identity`].

pub mod broker;
pub mod handles;
pub mod identity;
pub mod pipe;
pub mod proto;
pub mod ring;
