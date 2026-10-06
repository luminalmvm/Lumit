# Bridge contract

How the Flutter frontend and the Rust engine talk. If this and the code disagree, fix one
of them in the same commit.

## The layering

```
flutter_ui/ (Dart)           widgets, layout, theme, input
    |  generated bindings
crates/lumit-bridge          the surface: calls in, readings and frames out
    |  plain Rust calls
engine crates                core, project, media, eval, gpu, audio, cache, render
```

The bridge is a leaf. No engine crate depends on it, and it depends on no frontend.

## The transport

`crates/lumit-bridge/src/api/**` declares everything Dart can call.
`flutter_rust_bridge_codegen` (pinned version, run from `flutter_ui/`) writes
`frb_generated.rs` and `flutter_ui/lib/src/rust/**`. Never hand-edit those. Run
`.\scripts\codegen.ps1` after any `api/**` change, then rebuild. A content hash in both
halves means a mismatched library refuses to start.

## The four rules

1. **No panic crosses.** The generated code wraps every call in `catch_unwind`. A panic
   still reaches Dart as a thrown exception, so every function returns
   `Result<_, BridgeError>` with a calm sentence, and a throw means a bug.
2. **The generator owns memory.** Nothing is hand-freed. Only frames cross as bulk data.
3. **One lock, held briefly.** Each project's state sits behind its own `RwLock`, held for
   one transition. The change observer is told after the lock drops.
4. **The library is required.** No placeholder mode. Flutter tests drive the real engine,
   so `cargo build -p lumit_bridge` comes first.

## Commands down, references up

- Dart never holds the document. It holds handles (`ProjectReference`,
  `CompositionReference`, `LayerReference`, `ItemReference`) and calls methods on them.
- A `ScopedChange` stream names which handle an edit touched, so only that part redraws.
- Each user action is one call that maps to one `lumit_core` op, so one undo step.
- An op takes a whole value, not a delta. A key drag in time and value is one write.
- A drag stages rather than commits. `render_frame_with_preview` and its siblings render a
  patched clone. Only the release commits.
- The engine clamps parameter values to their declared range. The frontend's clamp is only
  it agreeing.
- The frontend holds interaction state (playhead, zoom, selection). The engine holds
  policy. If a Dart change needs a clock, a queue, a retry or a staleness flag, it belongs
  in Rust.
- Time crosses as exact `{num, den}` or frame indices, never float seconds. Use
  `time_of_frame` and `frame_at_time` rather than doing the maths in Dart.
- Keyframes are stored in layer time and cross in comp time, converted by the layer's
  `start_offset` in both directions.
- Long calls report progress on their own stream.

## Frames

- The Viewer's frames cross as zero-copy shared textures only: D3D12 on Windows,
  IOSurface on macOS, DMA-BUF on Linux. No pixels are copied.
- Small, bounded stills still cross as pixels: thumbnails, scope traces, the dropper's
  window. The engine caps their size.
- Scope traces, dropper reads, progress and frame timings ride the frame response stream.
  Timings only while the frontend asks for them.

## Display text

Effect labels, parameter names and keymap descriptions cross in British English beside a
stable id. The frontend translates them through `flutter_ui/lib/l10n/engine_labels.dart`,
so a new engine string needs an entry there. Don't build display strings with `format!`.
Send the pieces. Layer names, file paths and the like are the user's and pass through.

## Features

`media`, `render` and the three `shared-texture` features are on by default. A feature
changes what a function does, never whether it exists, so the Dart side is one shape on
every platform.

## Threads

Export runs on its own thread inside `lumit-bridge::export`. Long work runs on engine
threads, and the frontend polls or listens for progress.
