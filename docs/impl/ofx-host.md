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
           (kOfxImageEffectActionGetRegionsOfInterest — even untiled, answer full RoD)
           kOfxImageEffectActionGetFramesNeeded       → temporal clips (Twixtor!)
           kOfxImageEffectActionRender                → between Begin/EndSequenceRender
teardown:  kOfxActionDestroyInstance, kOfxActionUnload
```

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

Watchdog: per-action deadline (render: 30 s default), three strikes → plugin disabled for
the session with a calm badge. Broker crash = restart + replay describe/instance from our
cached descriptor state; the render that died returns identity + badge.

## 5. Test plan

1. Conformance bench first: **openfx-misc** (Natron's plugin set, ~80 plugins, source
   available) then **ntsc-rs** — both free; run describe→render across contexts, assert no
   bad-handle returns, valid output.
2. Handle fuzzing: call every suite function with forged/expired handles → correct OFX
   status codes, zero UB (run under ASan in CI).
3. Temporal: a test plugin requesting frames t±5 — prefetch batching delivers all frames
   in one shipment.
4. Crash isolation: plugin that segfaults on frame 100 → broker restarts, session
   continues, layer shows badge (the Gate-4 demo, [16-ROADMAP.md](../16-ROADMAP.md)).
5. Real targets: Twixtor and RSMB demo builds render inside Luminal matching their Vegas
   output on the same input within codec tolerance.
