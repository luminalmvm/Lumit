# OFX hosting from Rust: the parts the spec can't show you

[12-PLUGINS.md](../12-PLUGINS.md) defines what the OFX host does; this note is the FFI
mechanics. OpenFX is a 2004-era C API built on string-keyed property bags and function
"suites" — the difficulty is bookkeeping discipline, not cleverness.

## 1. Loading a plugin

- Bundles: `*.ofx.bundle/Contents/Win64/*.ofx` (a DLL). `LoadLibraryW` / `dlopen`, resolve
  two exports: `OfxGetNumberOfPlugins()` and `OfxGetPlugin(i) -> *const OfxPlugin`.
- `OfxPlugin` gives `pluginIdentifier`, versions, and `setHost` + `mainEntry` function
  pointers. Call `setHost(&host)` **before anything else**; host outlives everything.
- Standard search paths: `C:\Program Files\Common Files\OFX\Plugins` (Windows),
  `/Library/OFX/Plugins` (macOS), plus `OFX_PLUGIN_PATH`.

## 2. The host struct and suites

`OfxHost` = a property set handle (describing us) + `fetchSuite(name, version) -> *void`.
Implement, minimum for real plugins (Twixtor/RSMB/Sapphire CPU): `OfxPropertySuiteV1`,
`OfxImageEffectSuiteV1`, `OfxParameterSuiteV1`, `OfxMemorySuiteV1`,
`OfxMultiThreadSuiteV1`, `OfxMessageSuiteV1` (+V2), `OfxInteractSuiteV1` can stub-fail
gracefully at first (overlays degrade to no overlay).

**Handles are the whole game.** Every `OfxImageEffectHandle`, `OfxPropertySetHandle`,
`OfxParamHandle` etc. is an opaque pointer *we* mint. Do it safely:

```rust
// One registry per plugin process; handles are indices, never raw Box pointers.
struct HandleRegistry<T> { slots: Slab<T>, magic: u32 }
// handle bits: [magic:16][kind:8][index:...] — validate kind+magic on every suite call,
// return kOfxStatErrBadHandle instead of UB when a plugin hands back garbage (they do).
```

Property sets: `HashMap<&'static str, PropValue>` where `PropValue` =
Int/Double/String/Pointer arrays (OFX properties are always arrays; scalar = len 1). Type
confusion (plugin asks for Int on a Double prop) → `kOfxStatErrValue`, never a cast.
Pre-populate host properties honestly: `kOfxImageEffectPropSupportedComponents` = RGBA,
`kOfxImageEffectPropSupportsTiles` = **0 in v1** (full-frame per
[06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md); saying 1 and lying is the classic
host bug), `kOfxImageEffectPropTemporalClipAccess` = 1 (retiming plugins need it),
depth `kOfxBitDepthFloat` only in v1.

## 3. Action sequence (get the order wrong and plugins crash)

```
load:      mainEntry(kOfxActionLoad)
describe:  kOfxActionDescribe                         → plugin fills descriptor props
           kOfxImageEffectActionDescribeInContext     → per context (Filter, General)
instance:  kOfxActionCreateInstance                   → after params exist with defaults
render:    kOfxImageEffectActionGetRegionOfDefinition
           kOfxImageEffectActionGetRegionsOfInterest  → even untiled, answer full RoD
           kOfxImageEffectActionGetClipPreferences
           kOfxImageEffectActionGetFramesNeeded       → temporal clips (Twixtor!)
           kOfxImageEffectActionIsIdentity            → yes ⇒ passthrough, no render at all
           kOfxImageEffectActionBeginSequenceRender
           kOfxImageEffectActionRender
           kOfxImageEffectActionEndSequenceRender
teardown:  kOfxActionDestroyInstance, kOfxActionUnload
```

That listing is the whole render order and `lumit-ofx::render::RENDER_ACTIONS` is its
transcription; a test records what the plugin observed and compares the two verbatim
(K-591). `getClipPreferences` and `isIdentity` are here because
[12-PLUGINS.md](../12-PLUGINS.md) §2.1 names them among the actions the host dispatches;
earlier revisions of this note left them implicit.

Param changed actions (`kOfxActionInstanceChanged`) must fire between renders, wrapped in
`kOfxActionBeginInstanceChanged/End...` — Sapphire relies on it.

Images: `clipGetImage(clip, time)` returns a property set with data pointer, bounds, row
bytes (**can be negative** — bottom-up; honour it), pixel depth, premultiplication state.
We hand out fp32 RGBA premultiplied ([12-PLUGINS.md](../12-PLUGINS.md)); convert at the
boundary from fp16. Pin the buffer until `clipReleaseImage`.

`OfxMultiThreadSuiteV1`: implement `multiThread` over our worker pool but **cap
`multiThreadNumCPUs` honestly** and make `multiThreadIndex` correct — plugins allocate
per-thread scratch by it. Mutex functions: plain `parking_lot` wrappers.

## 4. Out-of-process transport

Per [12-PLUGINS.md](../12-PLUGINS.md): one broker process per vendor bundle. Transport:
control via a length-prefixed bincode protocol over a duplex pipe
(`interprocess` crate, named pipe/UDS); frames via shared memory ring
(`CreateFileMapping`/`memfd_create`, triple-buffered, frame header = bounds + rowbytes +
premult + hash). All suite calls the plugin makes re-enter *our stub inside the broker*;
the broker resolves what it can locally (memory suite, threading) and forwards the rest
(clip images, param reads — batched per render action into one prefetch, or Twixtor's
per-frame fetches will drown in round-trips: `GetFramesNeeded` tells us exactly what to
ship ahead).

Watchdog: per-action deadline, three strikes → plugin disabled for the session with a calm
badge. Broker crash = restart + replay describe/instance from our cached descriptor state;
the render that died returns identity + badge. **The shipped deadline is
[12-PLUGINS.md](../12-PLUGINS.md) §2.3's — 10 s for a render, 2 s for a control action —
not the 30 s this note first sketched** (K-592); both live as named constants in
`quirks.rs` and the quirks table's `render_timeout_ms` is the per-plugin exception.

What the built transport pins, beyond the sketch above (K-592):

- **The pipe is its own, not the child's stdout.** A third-party plugin's `printf` would
  otherwise land in the middle of a length-prefixed message and desynchronise the protocol
  for good; the child's standard output goes nowhere.
- **The broker's first word is the protocol version.** A mismatch refuses the broker with
  a sentence rather than deserialising bytes of another shape.
- **The ring is sized by a byte budget, once per bundle**: 512 MiB, clamped to between
  three slots (this note's triple buffering, as the floor) and sixty-four. Slot header:
  bounds, row bytes, premultiplication, payload length, FNV-1a hash — checked on read,
  because a stale slot is the one failure shared memory has and it is silent. At 1080p
  that is fifteen slots; at 4K it is the floor of three, so a `t ± 5` prefetch at that
  size does not fit and is refused — a bigger budget, not a different design.
- Frames cross the ring as fp32 RGBA, tightly packed top-down. The header's row bytes
  describe *the ring*; OFX's negative row bytes are applied at the plugin boundary inside
  the broker.
- The prefetch hook sits between `getFramesNeeded` and `isIdentity` — the only point in
  §3's order where a frame at another time may be asked for.

## 4a. Becoming an effect

A described plugin is turned into an `EffectDef` (`lumit-ofx::def::OfxEffectDef`) and
registered into the same catalogue the built-ins live in — the seam
[effect-registry.md](effect-registry.md) §2.6 was written for (K-593). What that pins here:

- **`apply_cpu` is the render.** The bag arrives keyed by hashed ids; `schema::value_routes`
  is the way back to the plugin's own parameter names, built by the same `rows_of` that
  minted the rows so the reverse of the `foo_x`/`foo_y` split is never guessed at. Numbers,
  switches, choices and colours cross. A **path** does not — the bag carries a file-table
  slot rather than the string — so a plugin's path parameter keeps its declared default.
- **The GPU pass is a read-back** (`lumit-render::gpufx::ofx`): working texture → linear
  fp32 → the definition → back. It talks to an `EffectDef` and nothing else, so
  `lumit-render` depends on no plugin host. `AuxKind::None`, no matte of its own.
- **`getFramesNeeded` is asked per instance and per frame** through
  `EffectDef::frames_needed`, and the offsets it answers reach `stack_temporal_window` — and
  through it the frame key and the neighbour decode. The static declaration §3's describe
  produced is the fallback and the gate.
- **A failed render is identity, byte for byte**: the definition returns without writing,
  and files a sentence under the instance for the badge.
- **Two hosts.** `LocalHost` drives a bundle in this process; `BrokerHost` drives §4's
  broker, which is the shipping arrangement. Both pool one live plugin instance per effect
  instance. Thread safety needs nothing new — `instance::render_lock` already turns the
  plugin's declaration into no lock, the instance's, or the bundle's (K-066).

## 4b. Going looking (K-594)

`lumit-ofx::discover` is [12-PLUGINS.md](../12-PLUGINS.md) §2.6's scan: §1's search paths
(already `bundle::search_paths`), a bundle apiece, §3's describe, §4a's `EffectDef`, and a
`register` callback for the composition root. What it pins:

- **`scan` takes a callback, not a catalogue.** A definition lands in two tables joined only
  by a `match_name` string ([effect-registry.md](effect-registry.md) §5), and the pairing is
  the composition root's (`lumit_render::gpufx::ofx::register`). `lumit-bridge` is that
  root; no engine crate gains a dependency on another.
- **`Hosting::Broker` is the shipping arrangement, `Hosting::InProcess` is the tests'.**
  Proving that a folder of bundles becomes the right set of catalogue entries needs no
  second process, and `CARGO_BIN_EXE_…` names the broker only inside its own crate anyway.
- **The switched-off list is read before describe, and again per render.** The stored one
  (`lumit_project::PluginPrefs`) keeps a plugin's code from running at all; the running one
  (`discover::set_enabled`) is what a plugin switched off mid-session is gated by, because
  registration is additive and never removes (K-593).
- **A rescan is guarded by name before any work.** That is what keeps it idempotent and what
  stops the second scan re-leaking a schema — §4a's recorded ceiling, discharged.
- **The ring is built for 1080p at scan time.** The scan runs before any composition is
  open, and §4 sizes a ring once per broker. A 4K comp then renders through the three-slot
  floor; the upgrade is a broker respawned at the comp's size.
- **A bundle's plugins share one broker.** `BrokerHost` holds an `Arc<Mutex<Broker>>`;
  eighty plugins in one bundle are one process, not eighty.

## 5. Test plan

1. Conformance bench first: **openfx-misc** (Natron's plugin set, ~80 plugins, source
   available) then **ntsc-rs** — both free; run describe→render across contexts, assert no
   bad-handle returns, valid output. **Landed** as `crates/lumit-ofx/tests/conformance.rs`
   over a runner in `crates/lumit-bench` (`ofx::ensure`, its binary `ofx-bench`), and the
   CI job `OFX conformance bench`. What running it pinned (K-595):

   - **The bench is fetched and built, never committed.** Same trade as the reference
     media: a runner clones each project and builds it into one folder, is idempotent, and
     a folder that already holds bundles needs no compiler at all — so a vendored or
     prebuilt drop works through the same path. The overview's open question is answered
     that way, with the honest qualifier that the job does not yet *insist*:
     `LUMIT_REQUIRE_OFX_BENCH` turns a missing bench from a named skip into a failure, and
     it is set the day a run proves the source build works on a hosted runner.
   - **The host tallies every status it answers** (`status::answered`, bumped in the
     suites' guard). "Assert no bad-handle returns" had nowhere to read from before that:
     a plugin swallows the code and carries on, which is exactly how a host bug hides
     behind a picture that came out looking right.
   - **Every context, not the first.** `describe_in` drives one named context, because a
     plugin is a different effect in each — different clips, different parameters — and
     the scan's "first drivable one" is a menu decision, not a conformance one.
   - **A rejection is a row, not a failure**: a context this host does not drive, a
     describe that failed, a parameter set Lumit cannot declare. The table counts them
     separately and the assertion is only about the third column.

2. Handle fuzzing: call every suite function with forged/expired handles → correct OFX
   status codes, zero UB (run under ASan in CI). **Landed** as
   `crates/lumit-ofx/tests/handle_fuzz.rs` — every entry point of all six suites against a
   corpus of forged, stale, wrong-kind, freed and randomly mutated handles, deterministic
   by seed, with the CI job `OFX handle fuzzing (ASan)` running it on nightly under
   AddressSanitizer (leak detection off: this host leaks the `OfxHost` and its registry
   slots on purpose, §1). It found one thing, folded back into the parameter suite: the
   entry points that are **not built yet** answered `kOfxStatErrUnsupported` to a handle
   nobody minted. A forged handle is `kOfxStatErrBadHandle` at every entry point of every
   suite, built or not — "unsupported" tells a plugin the feature is missing when the
   truth is that its handle is rubbish, and it is the one answer that would have it try
   the same handle somewhere else.

3. Temporal: a test plugin requesting frames t±5 — prefetch batching delivers all frames
   in one shipment. **Landed** as `crates/lumit-ofx-broker/tests/broker.rs`, asserting the
   shipment count and not only the pixels — and, end to end through the engine, as
   `lumit-eval`'s `a_plugins_eleven_sampled_frames_are_what_its_key_depends_on`: the eleven
   frames the plugin asks for are the ones the layer's frame key depends on, so a change to
   any of them retires the cached frame and a change outside the window retires nothing.

4. Crash isolation: plugin that segfaults on frame 100 → broker restarts, session
   continues, layer shows badge (the Gate-4 demo, [16-ROADMAP.md](../16-ROADMAP.md)).
   **Landed** in the same file. The broker tests live in the broker's own crate because
   `CARGO_BIN_EXE_…` exists only for tests in the package that owns the binary. The
   **badge** half is asserted through the bridge instead
   (`lumit-bridge`'s `a_plugin_that_fails_a_frame_badges_its_layer_and_the_next_frame_clears_it`):
   a failed render is identity byte for byte, the layer wears `plugin_failed` with the
   plugin's own sentence beneath it, a switched-off one wears `plugin_disabled` with no
   sentence, and the next frame that works takes the badge off. Splitting it that way is
   deliberate: the process dying needs a second process, the badge needs the whole seam
   from the definition to the read model, and neither test can prove the other's half.

5. Real targets: Twixtor and RSMB demo builds render inside Lumit matching their Vegas
   output on the same input within codec tolerance.

### How to run the commercial pass

Twixtor and RSMB are paid plugins with licence servers; they cannot be in CI, and no
harness here pretends otherwise. This is the checklist, run by hand on the Windows
machine before a release that claims OFX support, with the result recorded in the pull
request that ran it rather than in a document that would rot:

1. Install the demo builds into `C:\Program Files\Common Files\OFX\Plugins`. Start
   Lumit and open **Effects & presets**: each vendor's own grouping should be a heading,
   with its effects under it. A plugin that is not there is a line in the scan report —
   read it before assuming anything.
2. Drop a 1080p clip with real motion on the timeline (the bench media generator's
   `ref_a.mp4` will do). Add Twixtor, set it to half speed, and scrub. What is being
   watched: the frames either side arrive (§4's prefetch — a retimer with no neighbours
   renders a stutter, not an error), the controls in Effect controls are the plugin's own
   in the plugin's own order, and nothing modal ever appears.
3. Export the same range from Vegas with the same plugin and settings, and compare the two
   files frame by frame. Within codec tolerance is the bar; a *structural* difference —
   frames in the wrong order, a frame repeated, motion going the wrong way — is a failure
   and belongs in `quirks.json` with an entry saying what was done about it.
4. Repeat with RSMB on the same clip, which exercises the same path with a much shorter
   temporal window.
5. Then the unhappy half, which is the part worth having: pull the licence (or let the
   demo expire), and check the layer wears a calm badge with the plugin's own words under
   it and the comp still composites. Switch the plugin off in the browser's context menu
   mid-session and check the layer keeps its picture.

Record in the pull request: the plugin versions, the machine, which of the five steps
passed, and any quirks entry the run earned. A pass with no quirks entry is a result too.
