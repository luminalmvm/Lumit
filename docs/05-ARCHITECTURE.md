# Architecture

Two requirements drive everything here: the UI stays responsive under any load, and the
app never crashes. The crate list is in [GUIDE.md](GUIDE.md).

## 1. Dependencies

- They point down only: `lumit-bridge`, then the engine crates, then `lumit-core`. No
  engine crate depends on the bridge or a UI, so the UI can be replaced without touching
  the engine. It already was, once (egui to Flutter).
- `lumit-core` has no wgpu, FFmpeg or audio dependency. The document tests anywhere.
- `lumit-eval` depends only on `lumit-core`. Its seams are traits it defines
  (`SourceStamper`, `FrameSource`, `KernelExecutor`, `CacheStore`), so it tests against
  fakes. Today the shipped pixel path is the draw-list renderer in `lumit-render`.
- Heavy FFI crates live in one owning crate. wgpu is the exception: `lumit-gpu`,
  `lumit-flow` and `lumit-render` all use it.
- If two crates want each other, the shared piece moves down.

## 2. Threads

One process, plus a sandbox process per third-party plugin bundle.

| Thread | Role |
|---|---|
| UI | Input, edits, painting. Never evaluates, decodes, runs expressions or waits on a frame |
| Worker pool | `cores - 3`, at least 2. Interactive jobs pre-empt background ones at job boundaries |
| Decode | One per active stream, never on the pool, because a long-GOP seek would stall it |
| IO | Disk cache, journal, export files |
| Analysis | Camera tracking, one at a time |
| Audio pair | The cpal callback (lock-free reads only) and a thread that fills ahead of it: decodes the two-second blocks of sound the playhead is coming to |
| GPU submit | The only thread that submits to the wgpu queue |
| Share | While a project is shared: one accepting, and a reader and a writer per connection. The readers apply other people's edits. A host that asked its router to open the port has one more keeping it open |

**Cancellation.** Every request carries an epoch per consumer (the Viewer, each export,
background warming). Moving the playhead bumps the Viewer's. Jobs check at node and tile
boundaries and return `Err(Cancelled)`. A stale frame that finishes still goes in the
cache.

**Playback** is decode, evaluate, present over bounded queues 2 to 4 frames deep. The audio
clock is master: the frame shown is a function of the samples played. If evaluation falls
behind, frames drop. Audio never waits. A clip about to start has its file opened about a
second before its edit point, since opening one takes longer than those queues are deep.
Sound is never held whole: each file's is decoded in blocks under one byte budget, least
recently wanted dropped first. A block the callback reaches before it is decoded plays as
silence and is counted. An export waits for its blocks instead and reads every file from
the top, so its samples do not depend on what was played.

## 3. The document

- Every edit is a command (`AddLayer`, `SetKeyframe`, ...) with an inverse. Applying one
  makes a new immutable snapshot. Today that clones the whole `Document`.
- Undo and redo walk the journal. Autosave appends the same journal to disk. Recovery is
  the last snapshot plus a journal replay.
- Every entity has a stable UUID. Nothing is identified by index or position.
- The snapshot is published by one atomic pointer swap (`arc-swap`). Workers keep the
  snapshot they started with, so nobody reads a half-finished edit and edit and render
  never share a lock.
- A shared project has one host, and the host's order of edits is the order. A guest
  applies its own edits straight away and replays them over the host's as they arrive.
  An op writes a whole value, so one made against an older document is cut down to what
  its author changed before it is applied (`shared::land`). Two people on one layer then
  keep each other's changes, down to one parameter of one effect. Footage never crosses,
  each machine finds its own copy by fingerprint.
- Closing Lumit loses neither end's work. A host writes every edit since its last save to
  a log beside the journals, and lands them again when it shares the project next. A guest
  without its host writes the last document both had, the edits since and open conflicts
  for its copy to reopen from, less what a close left unsaved (`lumit-share`, `kept.rs`).

## 4. The evaluation graph

Layers in the UI, a graph underneath. On each edit the affected comp recompiles:

1. A layer becomes source, then effects, then masks and matte, then transform, then a blend
   over everything below. Adjustment layers apply to the composite so far. A Precomp is
   the nested comp behind one boundary node.
2. A Sequence layer becomes a switch: for a given time one clip is live and its subgraph
   is emitted, a clip that plays a comp as a Precomp's is, or two where clips overlap,
   crossfaded into one source by the incoming clip's fade.
3. Retime isn't a pixel node. It changes the time asked of the nodes above it. Frame
   interpolation adds a node only when the source time falls between frames.

**Two passes.** A cheap metadata pass works out each node's format, frame range and the
region it defines, pushes the needed region down, and folds out effects that do nothing.
The pixel pass then runs on workers, full frame per node, tiling only under pressure.

**Content hashing.** Each node hashes its type, version, parameters, time, quality and its
inputs' hashes. The hash is the cache key, so identical work dedupes and nothing needs
invalidating. An effect that reads other frames declares them so their hashes fold in.

## 5. GPU

- One wgpu device. DX12 on Windows, Metal on macOS, Vulkan on Linux.
- First-party effects are WGSL compute. The working format is fp16 scene-linear
  premultiplied RGBA.
- Textures come from a pool that spends against the resource governor.
- Readbacks never block the UI or submit threads.
- Device loss is routine on Windows. Everything GPU belongs to the renderer, so recovery
  is dropping it, building a new one, and refilling from the RAM and disk tiers.
- Every GPU effect has a CPU twin. It's the test oracle and the fallback.

## 6. Media

`lumit-media` wraps FFmpeg behind a `MediaSource` trait. Decode never runs on the worker
pool. Proxy level is part of the cache key. A file is probed when a frame first needs it,
or a moment before during playback, not when its comp opens. The render keeps at most
twelve decoders open and playback's read-ahead twelve more, a thread each. Both close the
one unused longest, never one a current frame uses.

## 7. Plugins and expressions

- First-party effects run in-process. Third-party OFX plugins run in a broker process per
  bundle. Frames cross in shared memory. A crashed or hung broker is restarted and the
  node draws as an error placeholder.
- A plugin node declares its region, its temporal needs and whether it's thread-safe. A
  plugin that isn't serialises on its own broker only.
- Expressions run in-process with no IO, no clock and seeded randomness. Results match per
  machine, not bit for bit across platforms. There's no time limit yet, so a runaway
  expression can stall a render thread.
