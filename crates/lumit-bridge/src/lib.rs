//! lumit-bridge: the seam the Flutter frontend calls the Rust engine through
//! (docs/flutter-port/03-ARCHITECTURE.md).
//!
//! # In plain terms
//!
//! The Flutter application is written in Dart and cannot call Rust functions
//! directly. This crate is the translator between them. It compiles to a single
//! shared library (a `.dll` on Windows) that Dart loads at start-up; Dart then
//! calls the plain C functions this crate exports — `lumit_bridge_new_project`,
//! `lumit_bridge_snapshot`, and so on — passing text in and getting text back.
//!
//! The text is JSON (docs call this "bridge v0", a hand-rolled JSON-over-C-ABI
//! seam we keep until the API stabilises and `flutter_rust_bridge` generates
//! this layer instead). Every call returns a UTF-8, NUL-terminated string that
//! Rust allocated; the caller reads it and then hands it back to
//! [`ffi::lumit_bridge_free_string`] so Rust can release the memory. Every reply
//! is either `{"ok":true, …}` or `{"ok":false,"error":"…"}` — the error is a
//! calm sentence for the status line, never a crash.
//!
//! The one exception to "everything is JSON" is [`ffi::lumit_bridge_decode_frame`],
//! which returns a raw block of RGBA pixels (a video frame is far too large to
//! encode as text); it has its own ownership contract, documented beside
//! [`ffi::lumit_bridge_free_buffer`].
//!
//! The engine-side document and its undo history live behind one process-wide
//! lock (there is exactly one Flutter window driving it, so a single client is
//! assumed). No exported function crosses the C boundary with a panic: each
//! body runs inside [`std::panic::catch_unwind`] and turns a panic into an
//! ordinary error reply.
//!
//! Engine crates never depend on this crate, and nothing depends on it — it is
//! a leaf over `lumit-core`, `lumit-project` and (behind the default-on `media`
//! feature) `lumit-media` (docs/05-ARCHITECTURE.md). Behind the default-on
//! `render` feature it *also* borrows `lumit-ui`'s headless compositor for the
//! composited-comp Viewer path — the one deliberate, temporary exception, logged
//! as K-175: the bridge reaches into the UI crate's renderer through the
//! `lumit_ui::headless` seam until the pixel pass moves into an engine crate.
//! The docs/05 rule (engine crates never depend on the UI) is unbroken; this is
//! the bridge, not an engine crate.
//!
//! ## Module map
//!
//! The crate is split so no one file grows unwieldy (owner's file-length
//! preference):
//! - [`state`] — the shared engine state and the v0.1/v0.2 state transitions
//!   (`String → String` JSON in, JSON out); this is where the core ops live.
//! - [`edits`] — the v0.3 edit ops: layer lifecycle, comp settings, keyframes,
//!   the work area and effects, split out to keep [`state`] under length.
//! - [`snapshot`] — turning a [`lumit_core::model::Document`] into the snapshot
//!   JSON the panels read (snapshot v3 adds the transform read-back, identity
//!   links, the work area and the effect stack to v2's comps/layers/media).
//! - [`media`] — probing footage and decoding frames, gated behind the `media`
//!   feature; plain-data probe results that the snapshot embeds either way.
//! - `render` — the composited-comp Viewer path, gated behind the `render`
//!   feature; holds one session-lifetime headless renderer borrowed from
//!   `lumit-ui` and turns `(comp, frame)` into an RGBA8 buffer (K-175).
//! - [`ffi`] — the `extern "C"` surface: pointer marshalling, `catch_unwind`
//!   guards, and the string/buffer ownership contracts.

mod columns;
mod edits;
mod export;
mod ffi;
mod media;
#[cfg(feature = "render")]
mod render;
mod retime;
mod snapshot;
mod state;

use serde_json::json;

/// The C ABI generation. Bumped only when the exported function set or the JSON
/// shapes change incompatibly, so Dart can refuse a mismatched library.
///
/// v2 added the composition/layer/media detail to the snapshot and the
/// layer/transform/marker ops. v3 added the transform read-back, identity links,
/// work area and effect stack to the snapshot, plus the layer lifecycle,
/// comp-settings, keyframe, work-area and effect ops. v4 (this build) adds
/// export (start/poll/cancel + the preset resolver), keyframe interpolation
/// read-back and set, the Retime read-back and its ops, and the blend-mode,
/// matte, parent, motion-blur and add-mask columns. Every addition is
/// *additive*, so an older Dart client still reads every field it knew, but the
/// ABI number rises so a client that needs the new calls can insist on them.
pub(crate) const ABI_VERSION: u32 = 4;

/// `{"ok":false,"error":"…"}`. serde escapes any control character, so the
/// resulting string never carries an interior NUL and always makes a `CString`.
pub(crate) fn err_json(message: impl AsRef<str>) -> String {
    json!({ "ok": false, "error": message.as_ref() }).to_string()
}
