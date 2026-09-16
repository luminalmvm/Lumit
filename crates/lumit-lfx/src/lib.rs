//! `lumit-lfx` - Lumit's own native plugin host (docs/12-PLUGINS.md §3 says
//! what it must do, docs/impl/lfx.md says how).
//!
//! # In plain terms
//!
//! Somebody writes an effect in C or Rust, builds it against the header
//! `lumit-lfx-abi` publishes, drops the bundle on Lumit, and it appears in
//! **Effects & presets** next to Gaussian blur - same category, same rows,
//! keyframeable, expression-readable. This crate is the Lumit side of that: it
//! finds the bundle, has a broker open it at arm's length, turns what the
//! plugin declares into the same `EffectSchema` a built-in carries, and
//! drives one frame at a time across a shared-memory ring.
//!
//! What is here today is most of that road. The namespace
//! seam's arithmetic (docs/impl/lfx.md §4.1): what an LFX release's frames
//! are called, and the refusal that keeps it honest. **Describe**
//! (§2.2 to §2.4): the sink a plugin pushes its typed declarations into, and
//! the lowering that turns them into the same `EffectSchema` a built-in carries -
//! the parameter kinds onto Lumit's own vocabulary, the mandatory units, the
//! eight picture families, the traits whose every zero means "unstated", every
//! ceiling the header declares asked where the stranger's numbers arrive, and
//! the routes each row takes home to the value the plugin reads. And the two
//! planes the second process is reached over (§3.2, §3.4): [`ipc`] holds
//! the control protocol - a closed vocabulary, versioned apart from the ABI, in
//! which every message is answered exactly once - and the shared-memory ring,
//! whose slots are sized by the depth of the frames they carry and whose bytes
//! are charged to the governor's ledger. And the in-process round trip (§4.2,
//! §10): [`local`] opens a bundle in this process, fills in the frozen
//! describe sink so a stranger's typed declarations become the same
//! [`describe::Declaration`]s the broker will decode, creates an instance, and
//! hands it one frame at a time at whichever depth it is given - which is the
//! whole of the ABI edge, exercised by a plain `cargo test` against
//! `lumit-lfx-testplug`'s twelve personalities. And the second process itself
//! (§3.3, §3.5): [`ipc::broker`] is the supervisor. It starts
//! `lumit-lfx-broker`, has it read the bundle's own [`manifest`] **before any
//! of the plugin's code**, has the module opened lazily on the first describe
//! with the switched-off list travelling with it, compares what the code
//! answered against what the listing claimed, mints the handles an instance is
//! named by, holds every action to the deadline its [`quirks`] entry gives it,
//! and replays the whole session into a replacement broker when a plugin takes
//! one down. And discovery (§5): [`bundle`] is where a plugin lives on
//! disk - the `.lfx.bundle` layout, the ordered per-target architecture list,
//! the sorted walk that never opens one bundle looking for another, and the
//! search paths - and [`discover`] is the scan over it, which spawns a broker
//! per bundle, reads the listing before any of that bundle's code, negotiates
//! [`extensions`] from it before anything is instantiated, and keeps what
//! registered and what it turned away in two session tables. And the catalogue
//! entry itself (§4.2): [`def`] is what a described plugin becomes - an
//! ordinary `EffectDef`, with the resolved bag turned into the dense value
//! array the plugin reads, both depths crossing as themselves, identity byte
//! for byte on every road out that is not a picture, and a badge taken on read
//! so a stale reason cannot mark a later frame. And the live instances behind
//! that entry (§4.4): [`pool`] is one live instance per in-flight frame,
//! leased for the length of a frame and grown under the first
//! adaptive-concurrency policy this project has written down - the render
//! worker count, the ring the ledger granted, a collapse to one under pressure,
//! the bundle-wide lock a thread-unsafe declaration pins every plugin of a
//! bundle behind, and the declared working memory a frame is refused for
//! before it is dispatched. And the installer (§6): [`install`] is what
//! dropping a `.lfxpack` on Lumit does - every entry's name swept before a byte
//! is read, the detached Ed25519 signature checked before the JSON is parsed,
//! the key's fingerprint compared against [`trust`]'s store, a bounded unpack
//! into a folder no scan looks in, a layout check, the staged bundle manifested
//! in a broker out of process, and then one rename into the directory every
//! host searches. The render pass that drives all of it and the
//! Addons page that lists it land in the packages after this one, and
//! [`LfxRejection`] grows a variant with each of them.
//!
//! **Nothing in the editor process reaches [`local`].** docs/12:354-356 forbids
//! an in-process path in version 1, so the editor holds a `BrokerHost` and a
//! pipe. The broker binary at the other end of that pipe is the module's other
//! caller, and it opens a stranger's bundle in earnest - so the isolation is
//! the process boundary rather than the module's audience, and every
//! raw-pointer read in it is production code. What the in-process spelling buys
//! the suite is that the part no amount of care makes obviously correct can be
//! driven with no second process, no pipe and no ring.
//!
//! # What is here and what is elsewhere
//!
//! The frozen C ABI - the header, its `#[repr(C)]` mirror and the layout suite
//! that pins the two together - is `lumit-lfx-abi`, deliberately MIT and
//! deliberately empty of behaviour. Nothing that decides anything may live
//! there, which is why this crate reads its constants rather than repeating
//! them - the depth on the wire is the header's own `lfx_pixel_format` and the
//! tag on a value is its own `lfx_param_kind` - and takes every decision
//! itself. This crate is GPLv3 like the rest of the application.
//!
//! Nor does the C ABI reach far into this crate. `lfx_describe_sink`'s function
//! pointers are filled in by whoever is holding the plugin - the in-process
//! host in tests, the broker in the shipping path - and each of them turns one
//! `*const lfx_*_param` into a [`describe::Declaration`]. Keeping the raw
//! pointers out is what lets the whole of the lowering be tested with no
//! plugin, no process and no `unsafe` at all: [`local`] is the one module that
//! holds any, and the one place the workspace's `unsafe_code = "deny"` is given
//! up.
//!
//! The namespace itself is `lumit-core`: `EffectNamespace::Lfx`, the `lfx:`
//! match-name prefix beside the other three in `fx::builtins`, and the
//! `EffectNamespace::is_catalogued` predicate every walk that admits a picture
//! effect asks - the arena walk, the three in `fx::temporal` and the frame
//! key's own gate in `lumit-eval`, five in all and enumerated on the predicate
//! itself. An LFX effect is an ordinary catalogue entry, so the engine needs
//! no more knowledge of this crate than it has of the OFX host: none.

pub mod bundle;
pub mod def;
pub mod describe;
pub mod discover;
pub mod extensions;
pub mod install;
pub mod ipc;
pub mod local;
pub mod manifest;
pub mod pool;
pub mod quirks;
mod rejection;
pub mod schema;
pub mod trust;
pub mod version;

pub use def::{BrokerHost, LfxDef, LfxInstances, MAX_OFFSET};
pub use discover::{
    discovered, is_disabled, plugin_of, refusal_of, refusals, scan, set_disabled, set_enabled,
    AddonRow, DiscoveredPlugin, Gated, LfxHost, Refusal, Rendering, ScanOptions, ScanOutcome,
};
pub use extensions::OFFERED_EXTENSIONS;
pub use install::{install, InstallError, InstallOptions, Installed, LayoutFault, PackManifest};
pub use ipc::broker::{
    broker_exe, nothing_disabled, Broker, BrokerConfig, BrokerError, DisableList, Picture,
    ProcessJob, Rendered, DISABLED_REASON, MAX_LIVE_INSTANCES, MAX_NOTES,
};
pub use pool::{
    Lease, Policy, Pool, PoolError, Serial, Throughput, MAX_POOL_INSTANCES, MAX_POOL_MEMOS,
    MAX_POOL_ROWS,
};
pub use quirks::{Quirks, QuirksTable};
pub use rejection::{subject, Ceiling, LfxRejection, Word, INTERNED, SUBJECTS};
pub use trust::{TrustError, TrustStore, Trusted};
