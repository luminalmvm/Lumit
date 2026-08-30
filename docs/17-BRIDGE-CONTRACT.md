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
    document, a paint or shape or mask list, a clip's retime envelope, a layer's
    own Retime map (`render_frame_with_retime`, K-329), and its **driver graph
    nodes** (`render_frame_with_driver_preview`, K-471) — one layer's one
    state per request, so a gesture spanning more than that previews the part it
    grabbed. The driver call stages the graph's *nodes* only: a drag on a
    number changes no wire, no position and no badge, and staging them would be
    inventing a state the document cannot be in.
- **A parameter's hard range is kept engine-side, on ingest** (K-620).
    `BridgeEffectInstance::set_value` clamps what it is given to the range the
    effect's schema declares — statics and every keyframe alike, an expression
    passed through because it is a string until it runs — and both the preview
    and the commit stage through that one call. The bounds still cross in
    `BridgeParamKind` so a control can draw its travel and clamp its own
    reading, but that reading is the panel *agreeing* with the engine, never the
    only thing standing between a gesture and an illegal value. A route that
    forgets — a keyframe dragged in the graph editor, a Viewer pick, a driver
    wire, a preset — can no longer write past the range, and a scrub can no
    longer preview a picture the parameter cannot hold and then land elsewhere.
    A slider's *travel* is untouched: typing past it stays legal (docs/08 §1.2).

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
- **A long call says how far it has got, on a stream of its own.**
    `LumitBridgeState::open_project(path, on_change_stream, on_progress_stream)`
    takes an optional `StreamSink<OpenProgress>` and names each phase of the read
    as it begins — `ReadingFile`, `ResolvingMedia`, `PreparingProject`,
    `StartingPreview` — each carrying the share of the whole open behind it
    (K-628). The engine stops at `StartingPreview`, because the last stretch is
    the frontend's: the render worker starting and answering. The weights live in
    Rust, so the frontend draws a number rather than deciding one, and the phases
    are only the divisions the engine can honestly see — a deserialise has no
    inside to report from, and none is invented.
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
    Crossing between **two comps' clocks** is the engine's job for the same reason:
    `LayerReference::nested_entry_frame(outer_frame)` answers which frame of a Precomp
    layer's nested composition that layer is showing (K-624), running the outer frame
    through the layer's `start_offset` and its Retime property and out at the nested
    comp's own rate. `None` when the layer is not a Precomp layer or the comp it names has
    gone. One sync call, made when a layer is double-clicked and never in a rebuild
    (K-184) — a frontend working this out itself would be a second implementation of what
    the renderer already decides.
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

### The Project panel's item reads (K-451)

An item handle answers what the redesigned panel draws, and each is one call because each is
one question the document already knows the answer to:

- `ProjectReference::new_folder(name, parent)` — the bottom bar's Folder button. Both
    decisions are the engine's (`ops::new_folder_ops`): a blank name becomes the next unused
    "Folder N", and `parent` files it. The ops commit as one `Op::Batch`, which is what makes
    the folder and its filing arrive and leave together; a `parent` that no longer names a
    folder leaves it at the root rather than erroring.
- `ItemReference::move_to_folder(folder)` / `ItemReference::move_to_root()` — the panel's
    two filing gestures: a drag onto a folder row, **Move to folder** on the row menu, and
    **Move to root**. The composition is the engine's (`ops::move_to_folder_ops`) and it is
    one `Op::Batch`, so an item leaves its old folder in the same undo step it joins the new
    one — an item listed by two folders would draw twice in the panel. Filing something where
    it already sits commits nothing at all rather than an undo step that changed nothing. Two
    refusals, both calm: an item or folder that no longer exists is `InvalidItem`, and a
    folder asked to move inside itself or its own descendant is `FolderCycle` — that move
    would take the whole branch off the panel root with nothing left to drag it back by.
    **Filing several items together is the frontend's `beginUndoGroup`/`endUndoGroup` round
    the calls**, not a plural entry point: the seam stays one item per call, and the group is
    what makes a multi-selection one undo step.
- `FootageReference::file_path()` — the Path column: the relative path a saved project
    actually carries (K-173), falling back to the absolute one when a project has never been
    saved. **Display data, and it touches no disk** — `get_status` is the question about the
    filesystem, and keeping the two apart is what makes drawing the column free.
- `ItemReference::is_used()` — the `in use` badge. Direct placement only, and the rule is
    `Document::item_is_used`: a layer somewhere names this, not "some render might reach it".
    No cache on either side of the seam — the engine's sweep of a document far past any real
    one is well inside a frame, and a cache would be machinery bought with nothing.
- `ItemReference::label()` / `set_label(u8)` — the colour tag, an index into the same palette
    a layer's chip uses, `0` untagged. Untagging removes the entry rather than writing a zero,
    so a project nobody has tagged gains no line in its file (K-258).
- `BridgeMediaInfo` carries the container's own **codec names** (`video_codec`,
    `audio_codec`, each `None` when that stream is absent), the sound's `channels` and
    `sample_rate`, and an `is_still` flag. The flag exists because a still image probes *with*
    a video stream — one frame of it — so "is this a still" is `MediaProbe::runs_as_video`'s
    answer and never something a panel may infer. It replaced the panel's zero-picture-width
    guess, which could only say "there are no pixels" and said "this is sound". Codec names
    are the *file's* words, not ours, so they cross untranslated (K-303).

### The Timeline's three drawing facts (K-441)

- **A marker's span crosses as frames.** `BridgeMarker.duration_frames` is `None` for a
    moment — which is what a plain cue is, and what every marker of a file written before
    markers could span opens as — and otherwise how many frames of the owner's own rate the
    pill's bar runs for. Frames rather than the exact rational the *time* crosses as, because
    frames are what the ruler draws with. The write-back still rides `core_markers`' id merge
    (K-270), with one rule the frames make necessary: a duration that still reads as the same
    number of frames keeps the document's **exact** rational, so a rename or a drag cannot
    quantise away a span finer than a frame that nobody resized.
- **A cache-bar frame is one byte in two nibbles.** `cached_frames` answers
    `framecache::bar::read_packed`: the low nibble is the storage state the bar has always
    drawn (`0` nothing, `1` held coarser, `2` held here, `3` parked coarser, `4` parked here)
    and the high nibble is the preview **divisor** the found picture was actually made at
    relative to the scale asked about (`1` full, `2` half, `3` third, `4` quarter, `0`
    nothing held). Tiers are probed finest first, so the answer is the best picture there is
    of that frame whatever order the tiers filled in. Two truths the painter carries rather
    than papers over: a frame held at a scale no adaptive tier renders at reads as nothing
    held, and on a sampled composition an unrefined frame wears its sample's tier.
- **A Sequence clip carries its source reach.** `BridgeClip.reach_start_frame` /
    `reach_end_frame` are where the whole source would sit on the **comp's** clock if none of
    it had been trimmed away — the ghost §12A.1 asks for, in the same comp frames
    `start_frame` already crosses in, so drawing it costs no time↔frame trip. `None` is the
    engine's own "not knowable" (`Clip::source_reach`): a **retimed** clip has no reach,
    because its map decides which source moment each frame shows, and a source whose length
    could not be read has none either rather than one pinned to a guess. Nothing is clamped.
    The source's length is the one thing the model cannot look up — a nested comp's is on the
    comp, a footage item's comes from the media probe — so the **seam** supplies it, which is
    why `read_layer_info` now takes the project's state and its document.

### The effect schema crosses as four lists, not one

An effect's parameters are one question; how the panel *arranges* them is another, and they
have different lifetimes. Four `#[frb(sync)]` free functions answer them, each keyed by the
effect's match name and each memoised on the Dart side for the life of the process — the
schema is static, and re-fetching it per card per rebuild was real hover-hot bridge traffic
(K-183, and the budget test that forbids bridge calls in a rebuild path):

- `list_parameters(effect)` — one `BridgeParamInfo` per declared parameter, in schema order:
    its id, its label, its **unit**, and its **kind**, which is what decides the control drawn.
    The **unit** (`BridgeUnit`, K-443) is what the row draws as its rider beside the value —
    `Raw` (no rider), `Percent`, `Px` (px@comp), `Degrees`, `Seconds`, `Frames` — and it is
    also what a point pick has to write in. It crosses because the *declaration* is the only
    thing that can tell Radial blur's per-cent `centre_x` from the dozen effects whose
    `centre_x` is px@comp; a Dart map keyed by parameter id could not, and was deleted with
    this. `Unit::Unset` and `Unit::PctDiag` never reach the seam — the first fails the engine
    build, the second is forbidden to every parameter (K-419) — and both would arrive as
    `Raw`, which draws no rider.
    The kinds
    are Float, Int, **Angle**, Choice, Bool, Colour, Seed, File, Layer, **MaskPath**,
    **Curve** and **Slider**.
    A `MaskPath` names one of the *owning layer's* masks (K-408) and crosses as a
    `BridgeEffectValue::MaskPath(Option<Uuid>)` — the mask id, or `None` for the panel's
    "First mask" entry. The **geometry never crosses**: the render flattens the curve
    engine-side, beside the op. The panel builds the dropdown from the mask names already in
    the layer entries it holds, so the row costs no call of its own per rebuild.
    A `Curve` is a **tone curve** (K-412) and crosses as
    `BridgeEffectValue::Curve(Vec<Vec<f32>>)` — an ordered list of 2..=16 `[x, y]` pairs in
    the unit square, the identity diagonal `[[0, 0], [1, 1]]` by default. Here the shape
    *does* cross, because a curve is at most sixteen pairs of numbers the user dragged and
    the panel is the thing dragging them. It is **static** (like File, Layer and MaskPath):
    a list that grows and shrinks has nothing to interpolate between two keyframes, which is
    why After Effects' own curve blob steps rather than animating. The engine straightens
    what it reads — sorted by x, repeated x dropped, clamped into the square, and the
    diagonal when fewer than two points survive — so a panel writing mid-drag never has to,
    and a write is never refused for being momentarily out of order.
    A **`Slider`** (K-414 — a closed range) carries `default`, `min` and `max`, and those
    two numbers are the travel *and* the hard bound, which is exactly what closed means. It
    is a kind of its own because it draws a control of its own — a track and thumb with the
    value beside it — the same reason `Angle` is. Its **value** still crosses as a
    `BridgeEffectValue::Float`, the arrangement `Int` and `Angle` already use: the kind says
    which control to draw, not how the number is stored, so a Slider row keeps every float
    path the panel has — keyframes, the graph editor, the expression seed.
    An **`Action`** (K-417) is a **button**, and the one kind that carries no value at all:
    no default, no range, and nothing in `BridgeEffectInstanceInfo::values`, because a press
    is an event and not a number that could be keyframed, undone or interpolated. It crosses
    only so the panel can draw one; the press goes back through
    `api::track::fire_effect_action(layer, effect, param)`, which is *not* an edit — nothing
    is staged, nothing is committed, and no undo entry appears. `defaultEffectValue` answers
    `null` for it, so Reset walks past the button rather than trying to put it back.
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
- `list_pairs(effect)` — the **vector pairs** (K-443): one `BridgeParamPair` per two adjacent
    `_x`/`_y` Float parameters, carrying the pair's `stem` and both halves' ids. The
    convention used to be read off the ids at the seam, in Dart, by whoever happened to need
    it; `EffectSchema::pairs()` is the declaration answering it now, so the panel's row
    folding, the link flag on the instance and the engine all get one answer.

**There is no `Point` kind, and that is deliberate.** A 2-D point crosses as two adjacent
`_x`/`_y` Float parameters that the panel folds into one row with a crosshair pick
([07-UI-SPEC.md](07-UI-SPEC.md) §6.1) — the naming convention is the whole mechanism, and
`list_pairs` is where it is now written down. The Lens flare's Light, Radial blur's Centre and
Depth of field's Focus point all ride it. An
`Angle` **is** its own kind, because no arrangement of existing rows draws a dial; its value
still crosses as a `BridgeEffectValue::Float`, since an angle is a number of degrees and the
kind only says which control to draw.

**A pair's chain is instance state, and rides the staging shape.**
`BridgeEffectInstanceInfo.linked_pairs` carries the stems this instance has chained (empty is
"all unlinked", which is every older project), in the **read model** so a chain glyph is drawn
per point row per rebuild with no call of its own.
`BridgeEffectInstance::set_pair_linked(stem, linked)` toggles it on the **staged** copy and
answers whether anything moved, exactly as `set_custom_name` does;
`LayerReference::set_effects` is the commit, so a toggle is one `SetLayerEffects` op and one
undo step like every other effect-stack edit. The **proportional drag itself is not on the
seam at all**: scaling y as x is dragged is UI-time arithmetic for the life of a gesture, and
the document's business is only which pairs are tied together.

### The Custom shader: rows that belong to the instance, not to the effect (K-642)

Every list above is keyed by **match name**, so every one of them answers a fact about the
*effect*. The Custom shader ([impl/custom-shader.md](impl/custom-shader.md)) is the one entry
whose controls are a fact about the *instance*: they come from the shader that copy of it
holds. Four instance-scoped members close that gap, and nothing else on the seam changes —
a derived row is an ordinary `BridgeParamInfo` and the panel cannot tell it from a declared
one.

- `BridgeEffectInstance::list_parameters()` — the **owed call**: the effect's declared rows
    followed by this instance's derived ones, in order, in one piece. For every effect but
    the Custom shader it is the same list `list_parameters(effect)` gives.
- `BridgeEffectInstanceInfo.derived_params` — the derived tail alone, carried in the **read
    model** and empty for every other effect. That is the panel's road, and it is why the
    rows cost no crossing: the declared half stays memoised Dart-side under the match name
    (it never changes), and the half that could not be memoised there rides the model the
    panel already holds. A fetch per card per rebuild is the traffic the budget test forbids.
- `shader_source()` / `shader_origin()` / `set_shader_source(source, origin)` — the text
    itself, which is **instance state and not a parameter** (§1.2 there): `Value` is `Copy`
    and hashed field by field, a kilobyte of text is neither, and two shader sources cannot
    be interpolated. So it does not ride `values` with the numbers; it is read on the gesture
    that opens an editor and written on the **staged** copy, `LayerReference::set_effects`
    being the commit — one `SetLayerEffects`, one undo step, exactly as `set_custom_name` is.
    `origin` remembers the file the text was read from and is **never read at render**: the
    text is copied in, because a project must be one file that opens on another machine.
- `shader_status()` — `error` is the refusal (`@binding` declared, a host name shadowed, no
    `shade`, a `vec3` field) or the compiler's own message with its **line numbers remapped
    onto the user's text**, and `None` both for a shader that compiles and for an instance
    with no source, because an effect nobody has filled in yet is a passthrough rather than a
    failure. `notes` is one calm sentence per annotation that would not parse, beside the rows
    that did. The same answer reaches the panel without a call as `badge_reason:
    "shader_failed"` plus the message in `badge_detail` — the fifth `BADGE_REASONS` key, and
    the only one that is not about somebody else's binary.
- `shader_graph()` / `set_shader_graph(graph)` / `detach_shader_graph()` — the **inner
    graph** (custom-shader.md §4, CS4), as its JSON text, riding the same staged road the
    source does: `LayerReference::set_effects` is the commit, so a graph edit is one
    `SetLayerEffects` and one undo step. `set_shader_graph` makes the graph master and, when
    it compiles, writes the compiled WGSL into `source` in the same staging (the §4.1 cached
    text) and drops `origin`; a graph that will not compile is stored anyway, with the badge
    carrying its sentence — being broken is a state to work in. `detach_shader_graph` is the
    §4.1 one-way door: the compiled text stays, the `graph` key goes. A text that is not a
    graph document at all is `InvalidShaderGraph`, a caller bug rather than a user state.
- `shader_graph_view(graph)` / `list_shader_nodes()` — module-level, pure questions with no
    document behind them: the resolved boxes (ports, nominal types, a Parameter box's own
    label — the one user-authored word, never translated) plus the one compile sentence, and
    the §4.3 vocabulary for the add-search. The engine stays the single validator: the canvas
    refuses a drop by building the candidate graph and asking, never by learning the type
    rules itself. Both are gesture-time calls, never rebuild traffic. Port and kind ids cross
    as ids, not English — the frontend's arb owns their words, so K-303 has nothing to walk.

**A derived row's value.** The document is not made to carry a row it has never been told
about: the derived defaults are filled onto the two copies the bridge makes — the one
`get_info` reads, so the row draws a value rather than a dash, and the staged one a handle
holds, so `set_value` can write it. A staged copy reaches the document only alongside an edit
the user actually made, which is what keeps §1.5's "nothing is added automatically" true while
still leaving every derived control live.

### The layer graph: derived boxes down one way, stored wiring both (K-471)

`api::graph` is the Graph panel's whole surface, and it is shaped by the one rule the
model rests on: **`Layer::effects` is still the only authority for the picture**
([impl/node-graph.md](impl/node-graph.md) §1.1). So what crosses splits in two, and the
split is the design:

- **Derived, read only.** `LayerReference::get_graph()` answers a `BridgeLayerGraph`
    whole: every box the canvas draws — the Source, one per effect **in stack order**, the
    Layer out, then the drivers — each with the sockets it draws, its English label and its
    bypass state. None of it is stored anywhere; it is worked out from the layer on each
    ask, so there is nothing to write back and nothing to keep in step. Filtering `nodes`
    to its `Effect` boxes *is* the effect stack, which is why the stack view can never be
    made to lie: the graph has no second opinion to disagree with.
- **Stored, read and written.** `BridgeGraphWiring` — the wires, the canvas positions,
    which boxes wear the `E` badge, and the **named groups** (K-651) — comes back inside
    the same read, is edited, and is handed straight to
    `LayerReference::set_graph(drivers, wiring)`, which commits one `Op::SetLayerGraph`.
    Add a driver, connect, disconnect, drag a box, toggle exposure, name a region: each
    gesture is one write and therefore one undo step, and auto-wire folds its edge into
    the same commit as the add. There is deliberately **no per-wire call**.

**A group names boxes, never geometry** (K-651). `BridgeNodeGroup` carries a name, a label
palette *index* and its members; the wash's rectangle is worked out from where those
members are sitting, so it follows a dragged box and no colour crosses the bridge —
the same rule the port types follow. `save_node_group(name, colour, nodes)` hands back the
JSON and Dart chooses where it goes (the engine never opens a file dialogue, exactly as
`save_preset`); `insert_node_group(text, x, y)` mints fresh ids, re-points the wires that
were inside the set and commits **once**, so a whole rig arrives and leaves in one undo
step. `list_node_groups()` lists the `.lumgrp` files beside the `.lumfx` presets.

**One call, not one per node** (K-183). `get_graph` is asked on selection and on document
change and held in Dart; the budget test forbids it in a rebuild path, and nothing about a
box needs a second question.

**Drivers ride every path an effect already has.** A driver *is* an `EffectInstance`, so
`LayerReference::get_graph_drivers()` hands out `BridgeEffectInstance` staged copies
exactly as `get_effects` does for the stack, and `set_value` / `set_custom_name` /
`set_enabled` stage onto them with `set_graph` as the commit — keyframes, the stopwatch,
expressions and the graph editor all work on a driver row unchanged. `new_driver(name)`
mints one **without committing**, because dropping a node is rarely the whole gesture.
The **property path** the Timeline and the fold rows address a driver's parameter by is
`<layer>/graph/<node>/<param>`, beside the existing `<layer>/effects/<effect>/<param>`:
the second segment names the group on the layer, as `transform`, `masks`, `effects` and
`audio` already do, and `graph` is what the layer's own field is called. It is a frontend
path — nothing about it crosses the bridge — but it is spelled here so the two panels that
will draw a driver row agree, and `effectIdOfPath` cannot mistake one for the other.

**A points wire is an edge like any other, and its source is an effect** (K-492,
[impl/points-stream.md](impl/points-stream.md) §1). `BridgeOutputRef` has a third arm,
`EffectData { effect, port }` — the first wire whose source is a *stack* effect rather
than a driver or the layer's own alpha. It carries **data, never a picture**: it cannot
reorder, branch or skip the image chain, so filtering `nodes` to its `Effect` boxes is
still the effect stack and the stack view still has nothing to lie about. An effect box's
`outputs` are its picture **plus whatever data outputs its signature declares**, which is
how Particulate's teal Points socket reaches the canvas with no Particulate-specific code
at this seam; a driver box's `inputs` likewise gain its signature's **data inputs** — the
wire-only ones with no stored value, no keyframes and no panel row. Two refusals are its
own: a points stream into a socket of another type is the ordinary type mismatch, and a
stack-to-stack points wire drawn back **up** the stack gets the loop sentence, because the
consumer's own output would be part of its input. A *reorder* that inverts such a wire is
not refused at all — a stack edit cannot be refused on the wiring's behalf, so the edge is
dropped inside the same commit and the same undo step, exactly as deleting the producer
drops it.

**A refusal is a calm sentence, not a broken document** (§1.5 of the note). A wire to a
missing node or port, a type mismatch, a second wire on one socket, or a loop among the
drivers each arrives as `OpError::InvalidGraph` carrying the engine's own words, and the
document is left exactly as it was. Refusal rather than degradation, because unlike a
dangling matte none of those states can be reached by deleting some *other* entity.
The sentence is pinned by the Rust tests, not read in Dart: a **sync** call's
`BridgeError` crosses opaque, as every op on this seam's does, so the panel's own job is
to decline a mismatched drop *before* committing — both sockets carry their type in the
read model, which is the same "evaluate it locally against what you already hold" rule
the greying rules follow. The engine's refusal is the backstop, not the message channel.

**Port types cross; colours never do.** `BridgePortType` has the model's seven variants and
the frontend maps each to a `port.*` theme token (K-472 §6.1) — five colours for seven
types. A port also carries its **English label**, declared beside the port in the engine
(`fx::Port`) rather than worked out from its id at the seam, so it rides the K-303 chain
like every other engine word: `fx-labels.txt` lists it and `engine_labels_test.dart` fails
without its entry.

**The Drivers family has a listing of its own**, `list_drivers()`, in the same shape as
`list_effects()` and with the same category heading beside it. Two listings rather than one
filtered by the frontend, because the distinction is the engine's: a driver makes a value,
not a picture, so it belongs in the graph canvas's search and never in the Add-effect menu,
where it would add a node that changes no pixel.

**A catalogue entry carries the ports it declares.** `BridgeEffectInfo` answers `inputs`
and `outputs` beside the name and the label — the same `BridgePort` the read model uses,
with `wired` always false, because nothing is wired on an entry that has no instance yet.
Two things depend on knowing an entry's sockets *before* the node is in the document, and
both were impossible without it: **auto-wire folds into the add's own commit**, so "drag a
wire out, pick a driver" is one `set_graph` and one undo step rather than an add followed
by a connect; and the **Tab search filters by the wire in hand**, so every row it offers is
one that would actually connect. Derived from `Signature::Data` and the schema's own
`ParamKind::port_type`, so the entry's sockets and the wires `LayerGraph::validate`
accepts cannot disagree.

### The camera track: an event down, readings up (K-417)

`api::track` is the Camera track effect's whole surface, and it is shaped by one fact: the
analysis is a minutes-long job on its own thread, over the *media file*, in `lumit-render`.
Nothing here does the work; this is the doorway.

- **Down** — `fire_effect_action(layer, effect, param)` presses Analyse or Cancel (see
  `Action`, above). `add_solved_camera(tracked)` adds a Camera layer holding a *link* to the
  tracked layer; `set_camera_solve_link(camera, tracked)` makes or clears one on an existing
  camera; `convert_camera_to_keyframes(camera)` bakes the derived motion into one key per
  composition frame and severs the link, as **one** undoable batch;
  `add_layer_at_points(tracked, tracks, frame, solid)` drops a 3D Null or Solid at the mean
  solved position of the named tracks, turned to face the camera at that frame.
  `clear_camera_corrections(camera)` puts a nudged camera's own properties back to the pose
  the link was made at (K-578), leaving the link itself alone, as one undoable batch —
  refused when there is no link or nothing in the lane, so the command is never offered on a
  heading where it would do nothing.
- **Up, and polled** — `track_status(layer)` is one `BridgeTrackStatus`: a stage
  (idle/queued/tracking/solving/done/cancelled/failed), the frames done and total, the
  solve's mean reprojection error, its point count, the frames it covers **and the frames the
  clip has**, and — on a refusal — a `BridgeTrackFailure`. The last pair is the partial track
  (K-540): `frames < clip_frames` says the analysis stopped before the end of the shot, and
  since the span is always a prefix — the job follows the source from its first frame and can
  only ever stop early — those two numbers are the whole of the bar the panel draws and the
  sentence it writes. It is **read, never subscribed to**: the engine keeps the reading as
  a value and whoever repaints samples it, exactly as the cache bar is sampled. The panel
  polls twice a second *only while a job is moving*, and stops the moment it is not.
- **The failure is a reason, not a sentence** — the same K-303 chain the import report uses,
  one step stricter: the reason crosses as an enum with no text at all, and Dart's switch
  over the generated enum picks the arb key. A reason added to the engine is a compile error
  in Dart rather than an English island in a translated window.
- **The point cloud** — `tracked_points(layer, frame)` answers where each solved point lands
  on **composition pixels** at that composition frame, plus a `depth` already normalised
  0..1 over the cloud on that frame. The engine projects; the interface draws. Which solved
  frame that is comes from `lumit_core::track::tracked_solved_frame`, the same walk the
  camera link takes, so the dots and the camera they were solved with cannot disagree. Asked
  for **once per frame change, never per rebuild** — the Levels histogram's rule (K-413), and
  the budget test is the gate.
- **The badge** — `camera_link(camera, frame)` answers the `BridgeLinkState`
  (unlinked/derived/held/unresolved) and the tracked layer's id, once per frame change.
- **Edited since track is not a call** — `BridgeLayerInfo.track_corrected` (K-578) says a
  solve-linked camera carries a correction, and on a *tracked* layer says a camera following
  it does. Both rows that draw the dot — the camera's Transform heading and the Camera
  track's status row — are rebuilt on every document revision, and a correction is a
  document edit, so this rides in the read model rather than being asked for per repaint
  (K-184).
- **What is not a call**: which layer of a composition is the tracked one. The read model
  (K-184) already carries every layer's every effect, so the interface finds the layer whose
  stack holds an enabled Camera track with Show points on, from data it is already holding.

### The export dialogue's settings cross flat (K-479, K-485, K-503)

`BridgeExportSpec` is `lumit_render::export::ExportSpec` **flattened**: no engine enum
crosses, every choice is a number or a short stable string, and **every field added since
the struct was first drawn carries a generated default**, so a caller that sets none of
them asks for the export Lumit has always written. That is not tidiness — a required field
would break every existing call site the moment the engine grew an option, which is a seam
dictating a frontend change it has no business dictating.

The flattening rules, in the order they matter:

- **Zero is "today's answer", never "no answer".** `audio_rate: 0` is 48 kHz,
    `audio_depth: 0` is sixteen bits, `audio_channels: 0` is stereo, `width`/`height` of
    zero are the composition's own frame, `fps: 0.0` is its own rate. One convention, so a
    field nobody touched cannot change a byte of the file.
- **An answer this build does not recognise is today's answer too.** `motion_blur: 99` is
    the compositions' own settings; `resample: "bicubic"` is the bilinear default. A number
    out of range must not refuse an export over a field the user never set — a stored preset
    or a Dart side one version ahead is exactly where such a number comes from.
- **An enum crosses as the number the rows are drawn in**, with `0` the default row:
    `motion_blur` 0/1/2 = current settings / on for checked layers / off for all layers;
    `retime_blend` 0/1 = current settings / off for all layers, and there is deliberately no
    third answer (K-502).
- **A colour space crosses as its stored name**, not as a label: `""` (sRGB / Rec. 709),
    `linear`, `rec709`, `rec2020`, `display-p3`, or an OCIO config's own name. The wording
    belongs in `app_en.arb` like every other string (K-005), and a name this build does not
    know is shown as it arrived, exactly as a codec name is (K-303).
- **`resample`** is `"high"` for Lanczos-3 and anything else — blank included — for the
    bilinear filter every export has always used (K-498).

`BridgeFormatCaps` is the same treatment of `FormatCaps`, and it is what the dialogue
**disables** rows from; the engine refuses the same combinations again before a frame is
rendered, so the two cannot disagree about what a file will hold. Two of its rows are
deliberately not lists:

- `audio_24_bit` is a flag, because the engine's list is `AudioDepth::ALL` and has exactly
    two members. `the_caps_row_says_what_the_engine_says` asserts the flag against
    `FormatCaps::audio_depths` for every format *and* fails the moment a third sample width
    exists, at which point it becomes a `Vec<u32>` like `depths`.
- The sample **rates** are not a caps row at all: they do not vary by format, so
    `export_audio_rates()` answers them once and the existing `audio` flag decides whether
    the row is live. The same test holds the two in step. Channel layout needs no row either
    — every format that carries sound carries both mono and stereo.

**Reordering the queue carries no undo (K-503).** `export_queue_move(id, index)` sits beside
`export_queue_cancel`/`export_queue_remove` and commits no op: the queue is not in the `.lum`,
it does not survive a restart, and its order is interaction state of the kind the frontend
would hold if it were not process-wide. It refuses in the export's own words — "that export is
already running", "that export has already run", "that export is no longer in the queue" — and
an `index` past the end lands the row last, which is what dragging one off the bottom means.

### A footage item's second file: the proxy (K-501)

A proxy is a second `MediaRef` on a footage item, so it reads and writes the same way the
original does — but it is three questions, not one, and the panel draws a different row for
each:

- `FootageReference::get_proxy()` — one call, `None` for an item with no proxy, otherwise the
    path to show, this item's own tick (`enabled`), and whether the document actually reads it
    (`in_use`: the project's master switch **and** the item's tick). Three row states from one
    read, which is the one-call-per-structure rule (K-451).
- Writes are ordinary commits: `set_proxy(path)` (attach or replace, switched on),
    `clear_proxy()`, `set_use_proxy(on)` — refused on an item with nothing attached, because
    the panel does not draw that tick — and `ProjectReference::set_use_proxies(on)` for the
    project-wide master. Attaching and switching on together is the frontend's
    `beginUndoGroup`/`endUndoGroup` round two calls, not a plural entry point.
- **Whether the proxy *file* is usable is not on the seam.** The renderer answers that itself
    (`lumit_render::source::effective_media`) and falls back to the original without reporting
    missing media — a present clip must not open the relink dialogue. A panel wanting a "proxy
    is broken" mark would need a new query over the renderer; there is none.

**MAKE-PROXY is the export's own three-call shape**, for the same reason: it is minutes of
work with nothing to look at. `FootageReference::proxy_path()` says where the file would go
(so a file already there can be pointed out first), `make_proxy()` starts the transcode and
returns as soon as it is running, `proxy_poll()` reports `Idle`/`Running`/`Done`/`Failed`, and
`proxy_cancel()` stops it — a cancelled job leaves no half-written file. One runs at a time;
a second is a calm refusal, because two transcodes share one disk.

One thing that call shape does *not* share with the export: **polling is what finishes the
job.** `lumit_render::proxy` writes a file and never sees a document, so the bridge commits the
`Op::SetItemProxy` itself when the file lands — from the item the job was started for, held in
the job rather than passed in, so the proxy arrives whether or not the panel that asked for it
is still on screen. The document's lock is taken after the job's own is dropped, never across
it (the one-lock-held-briefly rule above).

### Colour management crosses as names down, a summary up (K-489, K-490)

The project stores a path to an OCIO config and a colour-space name per footage item, and
**everything else is derived** — the parse, the resolved chains, the baked tables. So the
seam is one read of derived state and two ordinary document edits.

- **Read**: `ProjectReference::colour_summary() -> BridgeColourSummary` — the config's
    display path, whether it is loaded and usable, the refusal if it is not, the active
    colour space names, and each display with its views. **One call for the whole
    structure** (K-451), fetched on a document change and held in Dart; it reads the config
    file to see whether it has changed, so it never belongs in a `build()` (K-183, and the
    budget test that forbids it). `FootageReference::colour_space()` is the per-item read.
- **Write**: `ProjectReference::set_colour_config(path)` and
    `FootageReference::set_colour_space(space)`, both `None` for "back to the built-in
    family". Ordinary commits, so one gesture is one undo step and both travel in the
    `.lum` — colour management changes what a comp looks like, so it is the project's
    property and not the machine's. The path is stored as a `MediaRef` (K-173), built by
    the same `media_ref_at` a proxy is attached with.
- **The Viewer's chosen display and view ride the look message**, as
    `set_viewer_look(..., colour_view: Option<Vec<String>>)` — the two-name list
    `[display, view]`, and anything else (an omitted argument included) is the built-in
    transform. Session state, like the exposure beside it: never in the document, never
    sent to an export's own renderer. **The trap**: the look is set *whole*, so a caller
    that sends one without the view has said "no view", not "leave the view alone". The
    frontend holds the choice alongside the exposure and sends all of it every time.
- **The export's two colour questions are asked of the project**, because whether a name
    can be delivered depends on this project's config:
    `ProjectReference::can_deliver_colour_space(name)` is what the dialogue's dropdown
    enables a row on (K-485's disabled-not-hidden rule), and
    `CompositionReference::export_spec_check(spec)` — which replaced the free-standing
    `export_spec_check` — is the pre-queue check. `BridgeExportSpec::colour_space` itself
    **does not change shape**: it is still one stable string, and a config's own name is
    simply one more value it can hold.

**A refusal is not a sentence.** Every reason a config can be unusable names something in
the middle of it, so it crosses as `ColourError::key` plus `::args` — the K-303 rule below,
with the import report as the other worked example. `problem_english` rides along as the
fallback. **The config's own names — spaces, displays, views, and the facts inside a
refusal — are the user's words and are never translated**, so they must not go through
`engineLabel` on the way to the screen.

### The layer switches (K-497)

`BridgeLayerSwitches` is one read of every switch, and `set_switch(BridgeLayerSwitch, on)` is
one write per switch — one op each, so a click is one undo step and toggling one never
disturbs another. `Guide` joined them: reference-only, drawn in the Viewer and absent from
every delivered file at every depth. It is the one switch in the group that a **locked** layer
refuses, and the reason is the line the group divides on — shy changes what the Timeline
lists, guide changes what the file carries, and a lock is a lock against the second.

`Adjustment` joined them next (K-537): the layer sets its own picture aside and runs its
effect stack on the composite beneath it. It is read like the rest — `BridgeLayerSwitches
::adjustment`, which answers **true for a layer born an adjustment as well as one switched
into being one**, so the frontend draws the cell from the switch and never from the kind —
and written like the rest, `set_switch(Adjustment, on)`. Two things make it the odd one in
the group, both of them engine-side rather than seam-side. It is **refused** on the four
kinds with no picture to set aside — camera, light, null, audio — with
`BridgeError::NotConvertible`, where the other switches take any layer. And turning it
**off on a layer born an adjustment** ([`LayerKind::Adjustment`], what *New adjustment
layer* makes) is not one op but a batch: that layer has no picture to give back, so it is
handed a fresh comp-sized white solid and normalised to a solid with the switch off — still
one undo step. `set_switch` delegates to `set_adjustment(on)` rather than repeating any of
that, so the Timeline's plural switch handler and the direct call cannot disagree.

### The After Effects import crosses once, as a report

`LumitBridgeState::import_ae_bundle(path, on_change_stream)` is the whole surface of
[11-AE-IMPORT.md](11-AE-IMPORT.md)'s user half. **One call takes both front doors** (K-418):
an After Effects project file read directly, or a Lumit Bridge bundle as a folder or a zip.
The frontend does not choose between them — `lumit_import::open_ae` decides from the bytes
(RIFX magic is an `.aep`, anything else is a bundle), so the picker's only job is to offer
both and the report that comes back is the same shape either way. It maps what it opened to a
`Document` and **adopts that document exactly as opening a `.lum` does** — `api::state::adopt`
is the one road both take, so the displaced project's media caches, change sink and render
worker (a whole GPU device) are let go on either route. It answers `None` for a path that is
neither, the way `open_project` answers `None` for a `.lum` that will not open; short of
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
- **A composition's own small still crosses as pixels too** (K-667, superseding
    K-468's Viewer photograph). `CompositionReference::thumbnail(frame, max_edge)`
    answers a `BridgeRenderedFrame` whose longest edge is `max_edge` — 128 px for
    the welcome screen's recent rows, which is 36 KiB against a 1080p frame's 8
    MiB. It is **not** a second frame transport and cannot become one: the caller
    names the size, the picture is asked for once after a save or an open, and the
    Viewer's own frames still cross only as handles. Asynchronous, because it
    renders; the document snapshot is taken under the read guard and the guard is
    dropped before the render begins. It drives the session-lifetime renderer in
    `render.rs` — the one the export-input builder already shares — rather than
    the Viewer worker's, so a picture taken because a project was saved neither
    waits behind a frame somebody is watching nor holds one up.
- **Small stills still cross as pixels**, deliberately: footage thumbnails
    (`BridgeRenderedFrame`), the 256×256 scope traces and the dropper's
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
- **The Viewer's picture may be cut short at an effect (K-528, superseding
    K-486's thumbnail seam).** `CompositionReference::render_frame(frame, scale,
    mode, prefix)` takes an optional `BridgePrefixPoint` — a layer (which
    carries its composition) and the effect instance to stop **after**, or no
    effect at all for the layer's own picture before any of them — the Source
    box on the node canvas, which takes the chip like every other box. With one
    set, the engine renders the composition with that layer's effect stack
    truncated there and publishes it down the ordinary zero-copy frame
    transport, at the Viewer's own quality. There is no second render path and
    no second viewport: the prefix is a *way of looking*, like the exposure and
    the tone map beside it.
    The point rides the render request rather than having a call of its own, so
    turning the chip on or off costs the one render it was always going to
    cost. **The worker latches it**, exactly as it latches where the user is
    looking: the drag previews, playback and the idle fill that follow all show
    the same picture, so nine `RenderCompRequestWithPreview` constructors did
    not each grow a parameter. A render request that carries no point clears it.
    A point naming another composition's layer, a layer that has gone, or an
    effect the layer no longer carries **cuts nothing** and shows the honest
    picture — a stale chip is harmless rather than wrong.
    The frame key already hashes each layer's effects, so a truncated stack
    names its own frame and the cache cannot hand the chip the full picture; no
    field was added to the key. **But a prefix change renames every frame
    without moving the document revision**, which is the one case the worker's
    name memo cannot see, so latching a new point empties it — the same rule
    the viewer look already follows.
- **Readings ride the frame stream and have their own lane.** A scope trace
    (`WorkerResponse::Scope`) and a dropper patch (`WorkerResponse::Sampled`)
    come back on the worker's one response stream, so neither needs a second
    channel. In the worker's drain policy each is its own class: a frame, a
    trace and a patch are three different questions, and none may supersede
    another — only its own kind, where the newest wins (a pointer that has
    moved on makes the previous position worthless; so does a playhead).

    **A patch is cut from the frame that is on screen, not from a new one.** A
    dropper read walks the same ladder a trace does, in the same order — the
    frame held on the card, then the one banked in memory, and a composite only
    if neither has it. A window is a question about a few pixels of the picture
    the user is looking at; answering it by compositing that picture again is
    both slower and less true, and on a zero-copy build (where the shown frame
    lives in VRAM and nowhere else) it is what every read used to do.
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
  string a stable id of its own. The import report's reasons are the worked example (above),
  and the colour config's refusals the second: an id and its named facts cross, and
  `flutter_ui/lib/l10n/engine_labels.dart` writes the sentence. `test/l10n/engine_labels_test.dart`
  reads both Rust enums, so a reason or a refusal added to the engine with no sentence on
  this side fails the same gate a new effect label does.

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
    **`BridgeExportSpec` is the whole of `lumit_render::export::ExportSpec`** (K-485),
    flattened: format key, frame, bitrate (auto / a typed rate and its peak / blank for the
    encoder's own quality), rate, range, sound, depth, channels and alpha, colour space,
    the four crop insets plus *use region of interest* and the Viewer's region, ordered
    metadata, the render options, and the two *when done* ticks. `to_export_spec` is the
    one place it becomes the engine's type, and the queue stores the spec itself rather
    than a serialised copy of it, so a queued item can be read back exactly as it was
    added. Four sync calls let the dialogue ask the engine rather than re-deriving its
    answers in Dart — `export_format_caps`, `export_crop_for`,
    `export_resolved_bitrate` and `CompositionReference::export_spec_check` (on the
    composition, because the colour question is the project's) — and four more are the
    preset store
    (`export_preset_list`/`_get`/`_save`/`_delete`, over `lumit_render::export_presets`).
    Naming a codec needs the `media` feature: without it the conversion answers a calm
    "this build has no encoder" and every capability reads false, which disables the
    dialogue rather than removing a call.
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