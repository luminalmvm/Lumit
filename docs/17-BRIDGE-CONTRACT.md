# 17 - Bridge contract (front/back boundary)

**Status: canonical.** This is the single source of truth for how the Flutter
frontend and the Rust engine talk to each other. It supersedes the scattered
descriptions that previously lived in the flutter-port notes (now archived
under [archive/flutter-port/](archive/flutter-port/)). If this doc and the code
disagree, fix one of them in the same commit.

## In plain terms

The application is two halves. The **frontend** is written in Dart and drawn by
Flutter: windows, panels, the timeline, dialogs, input. The **engine** is the
Rust workspace: the document, undo history, decoding, compositing, caching,
export. Dart cannot call Rust functions directly, so a **bridge** sits between
them. The bridge is one Rust crate (`lumit-bridge`) that compiles to a single
shared library (a `.dll` on Windows) which the Flutter runner loads at start-up.

Two kinds of information cross the boundary:

**Commands and readings** cross through generated bindings
([flutter_rust_bridge](https://github.com/fzyzcjy/flutter_rust_bridge), K-179).
Dart never holds a copy of the document. It holds **handles** — small opaque
tokens standing for one thing in it — and calls methods on them:
`layer.rename(name: 'hero shot')`. Rust pushes a small "something changed, and
here is which layer" message down a stream, so only the part of the interface
that actually changed is redrawn.

**Video frames** are far too large to send field by field, so they travel as
**raw pixel buffers** — or, on the fast path, are never copied at all and are
shared as GPU memory the frontend displays directly.

## The layering

```
flutter_ui/ (Dart)              widgets, layout, theme, input, dialogs
    |   dart: ffi
crates/lumit-bridge (Rust)      the C ABI surface: commands in, JSON/pixels out
    |   plain Rust calls
crates/lumit-core, -project,    the engine (unchanged by the bridge)
        -media, -eval, -gpu,
        -audio, -cache, -render
```

- `lumit-bridge` is a leaf crate. Engine crates never depend on it, and nothing
    in the engine depends on the frontend. The rule from
    [05-ARCHITECTURE.md](05-ARCHITECTURE.md) - engine crates never know the UI
    exists - is unbroken. The bridge is not an engine crate; it is the seam.
- The Viewer render path goes through `lumit-render`'s headless renderer
    (`lumit_render::headless`), an **engine** crate. The bridge depends on no
    frontend at all (K-178 retired the last such edge).
- Long-running work (decode, export, beat detection) runs on worker threads with
    channels inside the engine; the bridge exposes progress through poll functions
    the frontend calls on a cadence.

## The transport: flutter_rust_bridge (K-179)

The seam is generated, not hand-written. `crates/lumit-bridge/src/api/` declares
the surface in Rust; `flutter_rust_bridge_codegen generate`, run from
`flutter_ui/`, writes the Rust glue (`frb_generated.rs`) and the Dart bindings
(`flutter_ui/lib/src/rust/`). **Never edit generated files** — change `api/**`
and regenerate, and check the output is idempotent before committing.

**The reference types are the identity.** Dart holds opaque handles —
`ProjectReference`, `CompositionReference`, `LayerReference`, `ItemReference` —
with methods on them: `layer.rename(name:)`, `item.delete()`. There is no
document snapshot crossing the boundary, so there is nothing to diff, no mirror
class to keep in step, and no id to resolve. Alongside them a `ScopedChange`
stream names *which* reference an edit touched, so a panel rebuilds its own
subtree rather than everything.

Two consequences are binding, because both exist to make one gesture cost one
undo step:

- **An op takes a whole value, not a granular delta.** `set_transform` takes an
    entire animation; `set_value` takes an entire `BridgeEffectValue`;
    `set_span` carries all three edges. A keyframe drag that moves a key in time
    *and* value is therefore one write. The predecessor's granular
    add/remove/shift ops are deliberately absent and should not be reintroduced.
- **A drag stages rather than commits.** `render_frame_with_preview` renders a
    patched *clone* of the document engine-side, so a hundred drag ticks produce
    pixels without producing a hundred commits, journal writes and undo entries.
    Only the release commits. Everything editable on a staged
    `BridgeEffectInstance` follows that shape, not `set_value` alone:
    `set_custom_name` (the instance's own display name, K-321) stages onto the
    copy and `LayerReference::set_effects` is the commit, so a rename is one op
    and one undo step like any other stack edit. `render_frame_with_preview`'s
    siblings patch the other things a drag can be holding — a transform, a text
    document, a paint or shape or mask list, a clip's retime envelope, and a
    layer's own Retime map (`render_frame_with_retime`, K-329) — one layer's one
    state per request, so a gesture spanning more than that previews the part it
    grabbed.

### The four binding rules

These are the contract.

1. **No panic crosses the boundary.** A panic must never unwind into Dart —
    unwinding across languages is undefined behaviour
    ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)). The generated handler
    enforces this: every call, sync and async, runs inside `catch_unwind`, and a
    second one wraps that in case the first's own error path panics. Nothing in
    `api/**` needs to repeat it.

    What a panic *becomes* is the part to know. It reaches Dart as a **thrown
    exception**, not as a value — where a well-behaved error is a
    `Result<_, BridgeError>` and arrives as an ordinary return. So every function
    on the surface returns a `Result` with a calm sentence fit for the status
    line, and a throw means a bug rather than a refusal. The
    `no-panics-in-frb-api` CI grep exists to keep it that way.

2. **Memory ownership is the generator's.** flutter_rust_bridge marshals every
    value; nothing is hand-freed on either side, and the raw-pointer discipline
    the previous transport needed is gone. The one thing that still crosses as
    bulk bytes is a rendered frame — see "The frame paths" below.

3. **One lock, held briefly.** Each open project's state lives behind its own
    `RwLock` in a process-wide registry. The lock is held only for the duration
    of one state transition, never across re-entry, an await, or a GPU call
    ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)). The change observer is
    notified *after* the store's own lock is dropped, because it crosses into
    Dart and a lock held across that boundary is forbidden.

4. **The library is required, not optional.** flutter_rust_bridge compares a
    content hash at start-up and refuses to run against a mismatched or missing
    library; there is no degraded placeholder mode. So `cargo build -p
    lumit_bridge` is a build dependency of the Flutter tests, and a stale library
    fails loudly rather than misbehaving quietly. Widget tests therefore drive
    the **real engine** — see `flutter_ui/test/frb/frb_test_support.dart` for why
    that is better coverage than a fake and not merely a constraint.

### Commands down, references up

The engine owns the document; the frontend never mutates it directly.

**And the engine owns the decisions (K-181).** The frontend holds *interaction
state* — where the playhead is, the zoom, the selection, the pan — and acts on it
the instant the user does, with no round trip to wait on. What it does not hold
is *policy*. It states facts ("the playhead is at 40", "play from here", "the
document changed") and paints what comes back; scheduling, timing, invalidation
and degradation are the engine's, because the engine is the half holding the
inputs to those decisions. The test: if a Dart change would need a clock, a
queue, a retry, a staleness flag, or a count of work in flight, it belongs on the
other side of this boundary.

- **Commands down.** Every user action becomes one call on a reference handle.
    Each edit maps onto a real, unit-tested `lumit_core` op (`AddLayer`,
    `SetTransformProperty`, `SetLayerEffects`, and so on), so undo/redo
    journalling is one clean step and is untouched by the existence of the
    bridge.
- **References up, not state.** Nothing returns a document. A reader asks the
    handle it already holds (`layer.getSwitches()`, `comp.getLayers()`), and a
    `ScopedChange` on the change stream names which reference an edit touched so
    only that subtree rebuilds. This is the whole difference from the previous
    transport, which returned a refreshed snapshot of the entire document after
    every edit.
- **A capability is not document state, and reads as its own answer.** Most reads
    ask the document; a few ask the *machine*, and the two must not be conflated.
    `ProjectReference::anti_aliasing` returns what the project asks for;
    `anti_aliasing_in_use` returns what this graphics card will actually give
    (K-274, K-286). Keeping them as two calls is what lets a limited adapter be
    reported without rewriting the project — and the capability read takes no
    engine lock, because a panel asking what the card can do must never queue
    behind a frame.
- **Rational time crosses as integers.** Frame counts and rates cross as exact
    `{num, den}` pairs or integer frame indices derived from a composition's own
    frame rate, never as floating-point seconds
    ([04-RETIMING.md](04-RETIMING.md), [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md)).
    `CompositionReference::time_of_frame`/`frame_at_time` exist so no frontend
    has to do that arithmetic itself: at 29.97 fps a frame is 1001/30000 s, and a
    keyframe placed in floating point does not land on the frame it was set on.
- **Keyframe times cross on the composition's clock (K-213).** The engine keys every
    animatable property in the layer's **own** time — comp time less its `start_offset` —
    which is what makes a layer's animation travel with it when it is moved. The frontend
    thinks in comp frames: that is what the ruler counts, what a lane draws against, and
    what a key drag commits. So the seam converts, in both directions, by the owning
    layer's `start_offset`: `BridgeScalar::read_at` carries a key out to comp time,
    `animation_at` carries it back. Both take the offset as an argument rather than
    defaulting it, so a new reader cannot forget one. Anything crossing with keyframes in
    it — the whole transform, a Retime property, an effect parameter, a camera's zoom, a
    volume curve, a staged `BridgeEffectInstance` — carries the same conversion. Read raw,
    every key on a layer that had been moved drew at the start of the composition.

### The effect schema crosses as three lists, not one

An effect's parameters are one question; how the panel *arranges* them is another, and they
have different lifetimes. Three `#[frb(sync)]` free functions answer them, each keyed by the
effect's match name and each memoised on the Dart side for the life of the process — the
schema is static, and re-fetching it per card per rebuild was real hover-hot bridge traffic
(K-183, and the budget test that forbids bridge calls in a rebuild path):

- `list_parameters(effect)` — one `BridgeParamInfo` per declared parameter, in schema order:
    its id, its label, and its **kind**, which is what decides the control drawn. The kinds
    are Float, Int, **Angle**, Choice, Bool, Colour, Seed, File, Layer and **MaskPath**.
    A `MaskPath` names one of the *owning layer's* masks (K-408) and crosses as a
    `BridgeEffectValue::MaskPath(Option<Uuid>)` — the mask id, or `None` for the panel's
    "First mask" entry. The **geometry never crosses**: the render flattens the curve
    engine-side, beside the op. The panel builds the dropdown from the mask names already in
    the layer entries it holds, so the row costs no call of its own per rebuild.
- `list_parameter_groups(effect)` — the twirls (K-145). A group names a *contiguous run* of
    the schema's parameters and renders where its first member sits; an empty label renders
    headerless, and `visible_when_param`/`visible_when_values` hide the whole run while a
    sibling Choice holds a different value (K-259).
- `list_enabled_when(effect)` — the **greying rules** (K-313): `param` is editable only while
    `on` satisfies `cond` (a bool is some value, a choice is/is not some index, a layer
    reference actually names a layer). `lumit_core::fx::param_enabled` is the same rule in
    Rust and the authority the tests pin; the panel evaluates it locally against values it
    already holds, because a round trip per row per rebuild for an answer it can compute is
    exactly the traffic the budget forbids. Greying is an **affordance, not a lock** — a
    write to a greyed parameter is still accepted, and the resolve step implements the real
    branch independently and never consults these rules, so the two cannot drift into
    disagreeing about pixels.

**There is no `Point` kind, and that is deliberate.** A 2-D point crosses as two adjacent
`_x`/`_y` Float parameters that the panel folds into one row with a crosshair pick
([07-UI-SPEC.md](07-UI-SPEC.md) §6.1) — the naming convention is the whole mechanism. The
Lens flare's Light, Radial blur's Centre and Depth of field's Focus point all ride it. An
`Angle` **is** its own kind, because no arrangement of existing rows draws a dial; its value
still crosses as a `BridgeEffectValue::Float`, since an angle is a number of degrees and the
kind only says which control to draw.

### The After Effects import crosses once, as a report

`LumitBridgeState::import_ae_bundle(path, on_change_stream)` is the whole surface of
[11-AE-IMPORT.md](11-AE-IMPORT.md)'s user half. It reads a Lumit Bridge bundle, maps it to a
`Document`, and **adopts that document exactly as opening a `.lum` does** — `api::state::adopt`
is the one road both take, so the displaced project's media caches, change sink and render
worker (a whole GPU device) are let go on either route. It answers `None` for a folder that is
not a bundle, the way `open_project` answers `None` for a `.lum` that will not open; short of
that an import **always completes**, and what could not be carried across is in the report
rather than in an error. The project it leaves open has no path: an import is not a file.

The `BridgeImportReport` crosses **once**, whole, on that call, and is held in Dart — the
report window filters and lists from the object it was handed, so no bridge call rides a
rebuild (K-183, and the budget test that forbids it).

Its rows are the worked example of the K-303 rule below. A reason is *not* sent as a
sentence: it crosses as a stable id plus its facts by name — `blend_mode_unavailable` with
`ae_mode: "Dissolve"` — because "blend mode Dissolve has no equivalent" is a different whole
text for every blend mode and a whole-text lookup could never hold it. The frontend writes the
sentence (`importReason` in `flutter_ui/lib/l10n/engine_labels.dart`), and the engine's own
English rides along as `english`, the fallback for an id this build has no sentence for.

### Versioning

There is no ABI number to gate on. flutter_rust_bridge embeds a content hash of
the declared surface in both the Rust glue and the Dart bindings and checks them
at start-up, so a Dart side built against a different `api/**` than the loaded
library refuses to start rather than calling into the wrong function. The
practical rule that follows: **after any change under `api/**`, regenerate and
rebuild**, and check the generated output is idempotent before committing.

## The frame paths (pixels, not JSON)

A video frame is too large to marshal field by field, so frames have their own
path, documented beside the types in
[`api/state.rs`](../crates/lumit-bridge/src/api/state.rs).

- **Zero-copy shared textures are the ONLY frame transport (K-177, K-183).**
    The engine renders into a shared texture and hands the frontend a handle,
    which the runner registers as a Flutter external texture — no pixels ever
    cross the boundary. Default-on cargo features (`shared-texture` for D3D12
    on Windows, `shared-texture-linux` for Vulkan/DMA-BUF, `shared-texture-macos`
    for Metal/IOSurface, K-195), each inert off its platform. The CPU read-back
    transport that serialised every pixel (8.8 ms per 1080p frame in the SSE
    codec alone) is deleted; a build with no zero-copy path at all drops every
    frame. Both publish variants are always *declared*, so the generated Dart is
    one shape on every platform and the Viewer holds one `switch` over the pair.
    There are two variants, not three: macOS reports `RenderedSharedTexture`,
    the same payload Windows does, because both are one opaque integer naming a
    surface plus its size (an NT handle there, an `IOSurfaceID` here). Only
    Linux needs more (fd, stride, offset, DRM format).
- **Small stills still cross as pixels**, deliberately: footage thumbnails
    (`BridgeRenderedFrame`), the 256×256 scope traces, and the dropper's
    129×129 windows (`BridgeSampledPixels`, K-210 — 66 KiB). All are bounded and
    rare, which is what makes the per-byte codec tolerable there. A window is a
    *reading*, not a picture: it answers "what is around this pixel", and the
    size cap is enforced engine-side (`worker_thread::cut_patch`, `MAX_WINDOW`)
    rather than trusted from the caller, so no request can turn this into a
    frame transport by the back door. It is deliberately **bigger than the nine
    pixels the magnifier shows**: the frontend reads its grid out of the window
    it already holds, so following the pointer costs no calls at all and a read
    happens only when the pointer nears the window's edge. The request names a
    **fraction of the picture**, not a pixel, and the reply says which raster it
    cut from: the engine may be working at preview resolution, so neither side
    can name a pixel in the other's grid.
- **Readings ride the frame stream and have their own lane.** A scope trace
    (`WorkerResponse::Scope`) and a dropper patch (`WorkerResponse::Sampled`)
    come back on the worker's one response stream, so neither needs a second
    channel. In the worker's drain policy each is its own class: a frame, a
    trace and a patch are three different questions, and none may supersede
    another — only its own kind, where the newest wins (a pointer that has moved
    on makes the previous position worthless).
- **Instrumentation rides it too (K-276).** Two further messages come back on the
    same stream, both small and both about a frame rather than being one:
    `WorkerResponse::RenderProgress` (`BridgeRenderProgress`: frame, stage code,
    0..1 fraction, and a `done` flag) says how far the frame the user is waiting
    on has got, and `WorkerResponse::FrameProfile` (`BridgeFrameProfile`: the
    frame, its total, and per-layer/per-effect milliseconds with ids as strings)
    says what a measured frame cost. Two rules bound them. **Progress is sent
    only for a frame someone is waiting on** — the worker turns it on around the
    interactive render paths and off again, so playback, the idle cache fill and
    scope traces are silent — and the *worker*, not the engine, sends the closing
    `done`, so a frame that faults or is served from the cache still ends its
    own bar. **Timings are sent only while the frontend has asked for them**
    (`api::cache::set_render_profiling`, read per frame): measuring fences the
    graphics card at each node, so an unasked-for frame costs exactly what it
    did before this existed.

## Display text crosses the bridge in English (K-303)

Some of what the bridge sends is meant to be read by a person: `BridgeEffectInfo`'s
`label` and `category_label`, the parameter and choice labels in an effect's schema, and
the keymap's `description` and context headings. **These stay British English on the wire
and are always sent alongside the stable id they belong to** (`match_name`, the parameter
`id`, the action id). The engine has no notion of a language and is not being given one.

The frontend translates them on arrival, by looking the English text up in
`flutter_ui/lib/l10n/engine_labels.dart`. Two consequences bind anything added here:

- **A new display string in the engine needs a matching entry in that table**, in the same
  commit. `flutter_ui/test/l10n/engine_labels_test.dart` reads the Rust sources and fails
  otherwise, so it cannot be forgotten quietly.
- **Build a display string with `format!` and it cannot be translated**, because the
  lookup is by whole text. Send the pieces and let the frontend assemble them, or give the
  string a stable id of its own. The import report's reasons are the worked example (above):
  an id and its named facts cross, and `flutter_ui/lib/l10n/engine_labels.dart` writes the
  sentence. `test/l10n/engine_labels_test.dart` reads the Rust enum, so a reason added to the
  engine with no sentence on this side fails the same gate a new effect label does.

Nothing else the bridge sends is display text: a layer name, a comp name, a file path and
a preset name are the *user's* words, and are passed through untouched.

## Feature gates

- **`media`** (default on) pulls `lumit-media` (FFmpeg) for probing and decoding.
    Without it, footage does not probe and thumbnails are absent.
- **Note.** `--no-default-features` builds and tests (K-273). It is **not** a
    build without FFmpeg: `lumit-render` and `lumit-audio` depend on
    `lumit-media` unconditionally and the bridge depends on both, so the library
    is still linked. The feature governs the bridge's own decode paths. The API surface is
    identical whatever the features are — the generated Dart is one shape
    everywhere — so a function never *disappears* with a feature: it stays
    compiled and its body degrades. Beat detection is the shape to copy: always
    present, and `NoAudioPipeline` on a build with no audio pipeline. What a
    media-less build actually loses is decoding — no probe, no thumbnails, no
    waveform peaks, and the decode-ahead thread drains its queue without
    producing anything — never a call that is not there.
- **`render`** (default on) enables the composited-comp Viewer path and export
through the headless seam.
- **`shared-texture`**, **`shared-texture-linux`**, **`shared-texture-macos`**
(all default on) enable the zero-copy path above, one per platform; each is inert
off its own target, so one default set builds everywhere. They scope the interop
code only. The **backend pin is not one of them** (K-205): `GpuContext::headless`
selects DX12 on Windows, Vulkan on Linux and Metal on macOS in every build,
feature or no feature, because a mixed-backend instance has no remaining use now
that read-back is gone.

## Threading and long-running work

- **Export** runs on its own encode thread inside `lumit-bridge::export`, driving
    `lumit-render` (K-017). The bridge holds the handle and drains progress on
    `api::export::export_poll`.
- **Playback / realtime tier.** A genuine render reports its measured cost to
    `lumit-eval`'s realtime controller (K-171); the frontend reads the current tier
    and scale back through `api::shell::playback_tier` to drive the Auto
    resolution setting.
    **The Viewer does not ask.** Each published frame carries the tier it was made
    at (`BridgeSharedFrameInfo::tier`), thus the two places that show the tier are
    given it. They asked for it in their `build()` before, which is one call across
    the boundary for each of them for each frame of playback. The transport is the
    same shape of question and gets the same answer: it reports what the build
    compiled to, thus the frontend reads it once and keeps it.
- **Probing** runs on a worker thread inside `lumit-bridge::probe`, with a
    synchronous fallback. `request` queues a file and returns at once — import,
    project open and relink all queue — and `ensure_probed` is what every route
    that needs a file's statistics calls: a look-up when the worker has been
    there, a probe on the spot when it has not. The fallback is what makes this
    a speed-up rather than a change of behaviour, and it is why the synchronous
    ops that need a real answer (`add_footage_layer` above all) can stay
    synchronous.
    **Nothing is drained or polled**, deliberately. An answer is filed under the
    file's own size and modification time, so it can only be read back for the
    file it was taken from; a file that has been replaced or has gone away
    re-stamps and is read again, which keeps `get_status` as honest as it was
    when it opened the container every time. The cache is bounded and is emptied
    when a project closes, which also cancels that project's queued work.
- **Beat detection** runs on its own worker inside `lumit-bridge::beats`, in the
    same shape: one analysis at a time, jobs carrying the generation they were
    made in so closing a project drops them, and the caller analysing inline
    when there is no worker to hand it to. `detect_beats` waits for its own
    answer and still returns the count, so the surface is unchanged; what moved
    is where the seconds are spent. It matters because the pool
    flutter_rust_bridge runs asynchronous calls on is *shared* — thumbnails,
    `has_audio`, `media_info` — and a couple of detections could hold the lot.
    The analysis answers times and confidences; the marker ids are minted by
    the caller afterwards, which is what keeps "the same audio finds the same
    beats" a checkable claim (docs/impl/beat-detection.md §5.4).

The historical record of the port that produced this seam is frozen in
[archive/flutter-port/](archive/flutter-port/).