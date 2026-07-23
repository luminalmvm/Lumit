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
//! - [`framecache`] — the rendered-frame cache (K-176): an LRU of RGBA frames
//!   keyed by comp/frame/scale under a document epoch, so a re-scrubbed frame
//!   skips the GPU. Its budget/clear/stats back the Settings → Performance cache
//!   controls; always compiled (inert without `render`, which is what fills it).
//! - [`cancel`] — engine-side render cancellation (K-176): a latest-wins
//!   generation high-water mark so a superseded comp render is skipped before it
//!   starts rather than stealing the renderer lock the next frame wants.
//! - [`ffi`] — the `extern "C"` surface: pointer marshalling, `catch_unwind`
//!   guards, and the string/buffer ownership contracts.

mod assets;
#[cfg(all(feature = "media", feature = "render"))]
mod audio;
mod beats;
mod cancel;
mod columns;
mod edits;
mod export;
mod ffi;
mod framecache;
mod fxkeys;
mod fxparams;
mod items;
mod media;
mod preset;
mod realtime;
mod recovery;
#[cfg(feature = "render")]
mod render;
mod retime;
mod sequence;
mod snapshot;
mod state;

use serde_json::json;

/// The C ABI generation. Bumped only when the exported function set or the JSON
/// shapes change incompatibly, so Dart can refuse a mismatched library.
///
/// v2 added the composition/layer/media detail to the snapshot and the
/// layer/transform/marker ops. v3 added the transform read-back, identity links,
/// work area and effect stack to the snapshot, plus the layer lifecycle,
/// comp-settings, keyframe, work-area and effect ops. v4 added export
/// (start/poll/cancel + the preset resolver), keyframe interpolation read-back
/// and set, the Retime read-back and its ops, and the blend-mode, matte, parent,
/// motion-blur and add-mask columns. v5 added footage placement
/// (`add_footage_layer`) and layer reorder (`reorder_layer`). v6 added the
/// Windows zero-copy Viewer path (`shared_supported`, `render_to_shared`,
/// K-177) — present but answering "unsupported" unless the `.dll` was built with
/// the `shared-texture` feature. v7 (this build) burns down the parity ledger's
/// bridge-ops section: the razor (`cut_clip_at_playhead`/`delete_clip_at_playhead`),
/// beat detection (`detect_beats`/`clear_beat_markers`), the project-item ops
/// (`delete_item`/`rename_item`/`move_to_root`/`relink`), the layer ops
/// (`rename_layer`/`convert_to_sequenced`/`trim_to_source_end`), the Retime
/// reverse/interpolation setters, the dedicated `autosave`, the text/solid/camera
/// property ops, recovery (`list_autosaves`/`restore_journal`), the `boot_log`,
/// the enum/bool/seed/point effect-param setters plus `reorder_effect`, the
/// param **ranges** and effect **category** in the snapshot, and the single-undo
/// `apply_keyframe_batch`. v8 (this build) burns down the parity ledger's
/// performance section: the bridge-side rendered-frame cache and its controls
/// (`set_cache_budget`/`clear_cache`/`cache_stats`), engine-side render
/// cancellation (`render_comp_frame_gen` carrying a latest-wins generation, and
/// `render_cancel_stale`), and the Project-panel thumbnail path (`thumbnail`).
/// v9 (this build) closes the last engine-surface parity blockers. Snapshot
/// completions (all additive): sequence-layer `clips`, layer `start_offset`
/// (frame + seconds) and local in/out seconds (the overrun-hatch ingredients),
/// `marker_details` (marker kind + beat confidence), the text/solid-size/
/// camera-zoom asset read-back, and effect `EffectKey` identity (namespace +
/// version) plus each animatable parameter's animation state. New ops:
/// `add_mask_geometry` (a mask from a drawn drag rect), the effect-param
/// keyframe ops (`toggle`/`add`/`remove`/`shift`/`set_interp`), the effect
/// preset ops (`save_effect_preset`/`load_effect_preset`, byte-compatible with
/// the egui `.lumfx`), and the realtime tier readout (`playback_tier`/
/// `reset_realtime`). Journal-append is now wired into every bridge commit, so
/// `restore_journal` recovers this frontend's own unsaved work. Every addition
/// is *additive*, so an older Dart client still reads every field it knew, but
/// the ABI number rises so a client that needs the new calls can insist on them.
/// v10 (this build) adds comp audio playback (docs/09; tester round 5 — the
/// Flutter frontend had no sound): `audio_prepare`/`audio_play`/`audio_pause`/
/// `audio_seek`/`audio_stop` and the per-tick `audio_clock` poll. The sound
/// card's clock is the playback master and the Dart Viewer chases it; a machine
/// with no output device answers calmly (`loaded` false) and playback simply
/// has no sound. Present but answering "no audio" unless the library was built
/// with the `media` + `render` features (the default set).
/// v11 (this build) adds the transform-preview fast path (the drag-a-numeric-
/// field lag report: every drag tick was running the full commit — undo push,
/// journal fsync, whole-document JSON serialise — once per pixel of mouse
/// movement). `preview_transform` stages an in-memory-only edit (no undo entry,
/// no journal write, no snapshot); `render_comp_frame_preview` renders a frame
/// under it, deliberately bypassing the rendered-frame cache so a throwaway
/// preview document can never evict real cached frames; `cancel_transform_preview`
/// drops the overlay without committing (Escape / drag-cancel); the existing
/// `set_transform` commits it for real, once, on drag-release, exactly as
/// before this fix. `preview_transform_supported` is the stateless capability
/// flag. Purely additive — an older Dart client that never calls the new
/// symbols behaves exactly as before — but the ABI rises so a client that
/// needs the fast path can insist on it.
pub(crate) const ABI_VERSION: u32 = 11;

/// `{"ok":false,"error":"…"}`. serde escapes any control character, so the
/// resulting string never carries an interior NUL and always makes a `CString`.
pub(crate) fn err_json(message: impl AsRef<str>) -> String {
    json!({ "ok": false, "error": message.as_ref() }).to_string()
}

/// `{"ok":true}` — the tiny stateless ack for calls that succeed without a
/// document change to report. `preview_transform`/`cancel_transform_preview`
/// callers must not treat this as a snapshot.
pub(crate) fn ok_json() -> String {
    json!({ "ok": true }).to_string()
}
