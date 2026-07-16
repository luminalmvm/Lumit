# Plugins, scripting, and expressions

**Status: canonical.** This document specifies Luminal's extensibility surfaces: the OFX
host (K-061), the KFX native plugin API (K-062), and the expression/scripting runtime
(K-063) — see [02-DECISIONS.md](02-DECISIONS.md). Terminology follows
[01-GLOSSARY.md](01-GLOSSARY.md) exactly. RFC-2119 keywords (MUST, SHOULD, MAY) are
binding. Process/thread architecture context: [05-ARCHITECTURE.md](05-ARCHITECTURE.md) §7;
effect placeholder behaviour is shared with [11-AE-IMPORT.md](11-AE-IMPORT.md) §6.

---

## 1. Philosophy

Plugins ship **after** the main application. Luminal's built-in effect suite (K-064) covers
the montage staples in-box, so v1 does not depend on third parties. But per K-062, **every
engine boundary is designed against these APIs from day one**: the evaluation graph's node
interface, the property system, ROI/DoD metadata, temporal frame requests, and the
thread-safety capability flag all exist in the first release because retrofitting them is
the multi-year mistake AE made (see [05-ARCHITECTURE.md](05-ARCHITECTURE.md) §8). A
built-in effect and a plugin effect are the same kind of node to the engine; the only
difference is trust and transport.

Two non-negotiables:

1. **A plugin must never take Luminal down.** Third-party code runs out of process. A crash,
   hang, or memory explosion in a plugin ends that plugin's process, not the session. The
   affected effect renders as an errored placeholder (identity output, calm badge) and the
   user keeps working.
2. **Plugin parameters are first-class properties.** The host owns parameter storage,
   keyframes, curves, expressions, and serialisation. A Twixtor-class OFX retimer's
   parameters sit in the Timeline and graph editor exactly like a built-in's, and
   expressions can read them.

To the user there is one concept: **Effect**. Built-in, OFX, and KFX effects all appear in
Effects & Presets, apply the same way, and stack in the same effect stack
([01-GLOSSARY.md](01-GLOSSARY.md) §6).

Missing-plugin behaviour: opening a project that references an uninstalled plugin produces
the same inert placeholder as AE import ([11-AE-IMPORT.md](11-AE-IMPORT.md) §6) — names,
parameters, and keyframes preserved, identity render, never lost on save.

---

## 2. OFX host

OpenFX is the sanctioned road to the gaming-edit staples: RE:Vision (Twixtor, ReelSmart
Motion Blur), BorisFX (Sapphire, Continuum, Mocha), NewBlue, Neat Video and others all ship
OFX builds already proven in Vegas and Resolve — the exact plugins Luminal's audience owns.
The standard is BSD-3-licensed, fee-free, and stewarded by the Academy Software Foundation;
implementing a host requires no permission and no payment. The host lives in `luminal-ofx`.

### 2.1 Conformance scope

Luminal implements a conformant OFX Image Effect host, targeting spec 1.4 semantics with the
1.5 additions adopted incrementally. Suites, in implementation order:

| Suite | Notes |
|---|---|
| **Property** (`OfxPropertySuiteV1`) | The load-bearing one; every object is a property set. Mechanical but must be exact. |
| **Image Effect** (`OfxImageEffectSuiteV1`) | Effect/instance lifecycle, clip access (in OFX's sense of "clip" — an image input/output; not a Luminal clip). |
| **Parameter** (`OfxParameterSuiteV1`) | Definition of all standard parameter types; `paramGetValueAtTime` answered from Luminal's property system (§2.2). |
| **Memory, Multi-thread, Message** | Small; Multi-thread backed by the worker pool with the documented OFX semantics. |
| **Interact / Draw** (later) | On-Viewer overlay widgets. The 1.5 Draw Suite is preferred (host-abstracted 2D drawing); legacy OpenGL interacts are the fallback and an open question under wgpu (§Open questions). |
| **GPU render** (later) | OFX 1.5 GPU suites — CUDA and OpenCL on Windows, Metal on macOS (§2.4). |

Actions dispatched: `onLoad`, `describe`, `describeInContext`, `createInstance`,
`getRegionOfDefinition`, `getRegionsOfInterest`, `getFramesNeeded`, `getClipPreferences`,
`isIdentity`, `instanceChanged`, `render`, and lifecycle teardown. `getFramesNeeded` is
critical: retimer-class plugins (Twixtor) request source frames at arbitrary other times,
which maps directly onto the evaluation graph's declared temporal dependencies
([05-ARCHITECTURE.md](05-ARCHITECTURE.md) §4.2) so sampled frames participate in content
hashing and caching correctly.

**Contexts supported**: filter, general, generator, transition. (OFX's retimer context is
rarely used by real plugins — Twixtor ships as a filter/general effect — and is deferred.)
**Pixel formats advertised**: float RGBA only, which the spec permits and which matches the
working space; plugins adapt or are rejected at describe time with a report entry. Luminal's
fp16 scene-linear working frames convert to fp32 at the plugin boundary and back
([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) defines where in the pipeline the
conversion nodes sit).

### 2.2 Parameter mapping

OFX parameters (double, int, boolean, choice, RGBA, 2D/3D point, string, custom, group,
page) map onto Luminal properties one-to-one. The host owns animation: keyframes and curves
live in Luminal's property system, are edited in the Timeline and graph editor, serialise
into the `.lum` file, and are readable from expressions like any built-in property.
`paramGetValueAtTime` evaluates the property (including any expression on it) at the
requested time. Custom parameters carry opaque vendor blobs; Luminal stores and round-trips
them without interpretation. Parameter pages/groups become the Effect Controls layout.

### 2.3 Out-of-process hosting

Per [05-ARCHITECTURE.md](05-ARCHITECTURE.md) §7, OFX plugins load in a **plugin server
process per vendor bundle** (one process per `.ofx` bundle; bundles that misbehave are
isolated from each other, and a vendor's plugins that share global state stay together).
The main process runs a thin proxy node in the evaluation graph.

- **Control plane**: a small RPC protocol (describe, instance lifecycle, parameter change
  notifications, message suite traffic) over a local pipe.
- **Frame plane**: frames cross via **shared memory** — the host writes input frames into a
  shared ring, the plugin renders into shared output buffers; no per-frame copies through
  the pipe. A **shared-texture fast path** (DXGI shared handles on Windows, IOSurface on
  macOS) is the later optimisation for GPU-rendering plugins, reusing the KFX transport
  (§3.5).
- **Watchdog policy**: every plugin call carries a deadline (default 10 s for `render`,
  2 s for control actions; configurable per plugin in the quirks table). A missed deadline
  or a crashed process kills and restarts the server; the in-flight node renders as an
  errored placeholder for that frame. Three consecutive failures disable the plugin for the
  session with a calm notice — no modal, no red alarm. State is reconstructible because the
  host owns all parameters.
- Plugins advertising thread-unsafe render serialise on their own server process without
  stalling the rest of the graph.
- **Multi-frame scheduling (K-066)**: OFX plugins cannot be forced to be
  frame-parallel, so the host schedules from each plugin's declared
  `kOfxImageEffectPluginRenderThreadSafety`: *fully safe* → frames render in parallel
  across pooled instances (same adaptive-concurrency policy as KFX §3.4); *instance safe*
  → parallel across instances, serialised within one; *unsafe* → bundle-serialised. Depth:
  the host offers fp32 (all major OFX plugins accept it) and converts fp16 comps at the
  boundary — the depth guarantee of K-066 is delivered by the host here, by the plugin in
  KFX.

### 2.4 GPU rendering

CPU float-RGBA rendering ships first — every major OFX plugin retains a CPU path. The OFX
1.5 GPU render suites come later: CUDA and OpenCL (Windows), Metal (macOS). The host passes
a command queue/stream and GPU-resident buffers; interop with wgpu goes through the same
external-memory machinery as K-014's CUDA nodes. Vulkan is not in the OFX standard yet.
Which suites the target plugins actually require is an open question inherited from
[05-ARCHITECTURE.md](05-ARCHITECTURE.md) and needs a plugin-by-plugin audit before the GPU
milestone is scheduled.

### 2.5 The quirks reality

Every OFX host implements the spec slightly differently and commercial plugins carry
per-host workaround tables; Luminal will need the mirror image. `luminal-ofx` maintains a
**quirks table** (data file: per plugin identifier + version → deviations, timeout
overrides, suite-version pins) from day one, so workarounds are data, not scattered code.

**Test bench**, in order: Natron's `openfx-misc` (~80 open plugins, broad API coverage),
`ntsc-rs` (open, real-world, popular with the target audience), then demo licences of
Twixtor, ReelSmart Motion Blur, and Sapphire. A plugin-host conformance run against this
bench is CI-automated for the open plugins and a manual checklist for the commercial ones.
Natron's HostSupport library and the openfx Support library are reference reading, not
dependencies.

Vendor relations: the OpenFX TSC includes the vendors that matter most to Luminal's
audience; engage early so "Luminal" appears in vendors' supported-host lists and licence
activation works. This is outreach, not engineering, but it gates real-world usability.

### 2.6 Discovery

Standard OFX directories scanned at start-up and on demand (`C:\Program Files\Common
Files\OFX\Plugins` on Windows, `/Library/OFX/Plugins` on macOS, plus the `OFX_PLUGIN_PATH`
environment variable), with a per-plugin enable/disable list in preferences. Discovered
effects appear in Effects & Presets under their OFX-declared grouping, visually identical
to built-ins apart from a small provenance tag in the effect's context menu.

---

## 3. KFX: the native plugin API

KFX is the first-party API — for effects that want what OFX cannot offer: Luminal-native
UI richness, host motion vectors, the fp16 working format without conversion, first-class
temporal access, and a modern, typed, sandbox-first contract. KFX competes with OFX only
where OFX is weak; it does not try to out-standard the standard. Host side: `luminal-kfx`,
sharing the sandbox/IPC substrate with `luminal-ofx`.

### 3.1 Shape: CLAP-shaped C ABI

The design copies what CLAP got right and what OFX got wrong:

- **A stable, minimal C ABI core**: plugin entry point returning a factory; descriptor
  (reverse-DNS id, name, version, categories); instance `create` / `destroy`; `describe`
  (declare parameters and capabilities); `process` (render one request). That is the whole
  core. The canonical header is C; first-party Rust and C++ wrappers ship alongside.
- **Everything else is an extension**: a named, versioned, **typed struct of function
  pointers** queried at runtime — `host->get_extension("kfx.gpu-frames", 1)`,
  `plugin->get_extension("kfx.temporal", 1)`. No stringly-typed property soup: OFX's
  untyped get/set on string keys is the single design mistake KFX most deliberately
  avoids. Planned first extensions: `kfx.temporal` (frames-needed declarations + fetching
  input frames at other times), `kfx.gpu-frames` (shared-texture I/O), `kfx.overlay`
  (Viewer interaction/drawing), `kfx.motion-vectors` (host-computed optical flow, the
  built-in flow engine exposed to plugins), `kfx.audio` (audio effects, later,
  [09-AUDIO.md](09-AUDIO.md)).
- Struct layouts are ABI-frozen and size-prefixed so they can grow; nothing is ever
  re-ordered or removed.

### 3.2 Parameters: declared, host-owned

At `describe`, a plugin declares parameters **descriptively**: kind (float, int, bool,
choice, colour, 2D/3D point, curve, string, file, group), range, default, unit, flags
(animatable, hidden). The host owns everything from there — UI in Effect Controls,
keyframes and curves in the Timeline and graph editor, expression access, serialisation
into `.lum` files, undo. At `process` time the plugin receives a read-only,
time-resolved value block. Plugins MUST NOT store parameter state internally; the host's
values are the only truth. This is the one OFX idea kept whole, minus the string soup.

### 3.3 Frame I/O contract

- Frames are **scene-linear, premultiplied alpha, RGBA float** — the working space, no
  colour conversion at the boundary ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)).
- **Every colour depth is mandatory (K-066)**: a KFX plugin MUST process both fp16 and
  fp32 frames correctly — the host sends whichever the comp is set to (K-026) and never
  converts to accommodate a plugin. A plugin MAY declare a preferred depth as a
  performance hint only. The validator (§3.6) runs the conformance suite at both depths.
- **ROI-aware**: a process request carries the output ROI and DoD; the plugin declared its
  ROI-expansion function at describe time, and receives exactly the input region it asked
  for. Full-frame is the degenerate case, not the assumption.
- **Temporal access by request**: via `kfx.temporal`, a plugin declares which input frames
  at which times it needs for a given output time (metadata pass), and fetches them during
  `process`. Declared dependencies feed content hashing, so temporal effects cache
  correctly ([05-ARCHITECTURE.md](05-ARCHITECTURE.md) §4.2).

### 3.4 Threading contract

Stated precisely, in the header comments, per function — the CLAP lesson:

- The host MAY call `process` from **any worker thread**, and MAY process different
  instances of the same plugin concurrently.
- The host MUST NOT re-enter one instance concurrently: for a given instance, `process`
  calls are serialised. A plugin wanting cross-instance shared state must synchronise it
  itself and declare the `kfx.thread-unsafe` capability to opt out of instance-level
  concurrency (the host then serialises that plugin bundle-wide, as with OFX).
- `describe` and instance lifecycle run on a single host-designated control thread.
- Every callback in every extension is annotated with its allowed calling context; the
  validator (§3.6) enforces the annotations at test time.

**Multi-frame rendering is mandatory (K-066).** The two rules above make it so: because
the host may run many instances concurrently, it renders **different frames in parallel by
default** through a host-owned instance pool (one instance per in-flight frame, parameters
applied per evaluation snapshot). Plugins therefore MUST NOT assume frames arrive in order,
one at a time, or on one thread. `kfx.thread-unsafe` is the sole, discouraged opt-out and
serialises the bundle. **Scheduling is entirely the host's decision**: how many instances,
which frames, and in what order is chosen by the host from the plugin's declared traits
(cost class, temporal window, ROI honesty) and its *measured* per-frame cost and memory
ledger — the same adaptive-concurrency approach as the engine's own nodes
(instances scale up while throughput rises and VRAM/RAM budgets hold, back off under
governor pressure). Plugins get no say at render time; they declare, the host optimises.

### 3.5 Sandbox and transport

KFX plugins run **out of process, always** — there is no in-process "trusted" mode in v1
(one fewer code path, and the crash-isolation promise stays unconditional; measure before
ever adding one). Same server-process model, watchdog, and restart policy as OFX (§2.3).

- **CPU path (mandatory)**: shared-memory frame buffers, zero-copy between host and plugin
  process.
- **GPU path (extension)**: platform shared textures — **DXGI shared handles + fences** on
  Windows, **IOSurface/Metal shared textures** on macOS — imported into the plugin's own
  device context; synchronisation via shareable fences. wgpu can import both. The host
  passes an input texture set and an output texture; the plugin renders and signals.
- IPC control plane is versioned independently of the ABI so the wire protocol can evolve
  without breaking compiled plugins.

### 3.6 Versioning, negotiation, deliverables

- Core ABI version is a single integer that intends to reach 2 never. Extensions carry
  semver; host and plugin exchange supported extension lists at load and MUST degrade
  gracefully when an extension is absent (a plugin requiring a missing extension fails to
  instantiate with a clear message and becomes a placeholder).
- **Deliverables shipped with the first KFX release**: MIT-licensed headers (deliberately
  more permissive than Luminal's GPLv3 so proprietary vendors can adopt without licence
  anxiety), a **`kfx-validator` CLI** (loads a plugin, exercises lifecycle/threading/ROI
  contracts, fuzzes parameter edges, checks the threading annotations under a stress
  scheduler — CLAP's proxy-validator idea), and a **plugin template repository** (Rust and
  C, CI configured, one working example effect per extension).
- Conformance claim: "passes `kfx-validator`" is the bar for listing in any future plugin
  directory.

### 3.7 Presentation

KFX effects appear in Effects & Presets identically to built-ins: same categories, same
search, same apply gestures, same Effect Controls layout rules
([07-UI-SPEC.md](07-UI-SPEC.md)), same preset save/load (K-065). No plugin ghetto.

---

## 4. Scripting and expressions

### 4.1 Engine

**QuickJS-ng, embedded in `luminal-expr`** (K-063), ES2018 surface. Rationale over V8:
trivially embeddable, byte-identical behaviour across machines and runs (no JIT tiers),
built-in memory/time limits, and per-property snippets are interpreter-friendly — the
host-call cost dominates, not JS execution. The engine version is pinned per project-file
version so old projects evaluate identically forever. V8 remains the named escape hatch if
profiling ever shows expression throughput as a real bottleneck.

An expression is a per-property script whose final statement's value replaces the
property's keyframed value each frame, matching the property's dimension count. Expressions
read the property graph; they never write it. AE expression compatibility is the point: the
community's copy-paste knowledge (`wiggle`, `loopOut`, …) must transfer wholesale, and
imported AE expressions ([11-AE-IMPORT.md](11-AE-IMPORT.md)) must run unmodified when they
stay inside the implemented surface.

### 4.2 Initial API subset

The v1 surface — the montage-expression core. Names and semantics match AE exactly:

| Area | Provided |
|---|---|
| Globals | `time` (comp time, seconds), `value`, `thisComp`, `thisLayer`, `thisProperty`, `comp("name")` |
| Property access | read-only graph traversal: `thisComp.layer("x").transform.position`, `effect("name")("param")` — including OFX/KFX plugin parameters (§2.2, §3.2) |
| Property methods | `valueAtTime(t)`, `numKeys`, `key(i)` (`.time`/`.value`/`.index`), `nearestKey(t)`, `speedAtTime(t)` |
| Randomness | `wiggle(freq, amp, octaves, amp_mult, t)`, `seedRandom(seed, timeless)`, `random(...)`, `gaussRandom(...)`, `noise(...)` — all seeded-deterministic (§4.3) |
| Loops | `loopIn`/`loopOut` (`"cycle"`, `"pingpong"`, `"offset"`, `"continue"`), `loopInDuration`/`loopOutDuration` |
| Interpolation | `linear`, `ease`, `easeIn`, `easeOut`, `clamp` |
| Time | `posterizeTime(fps)`, `timeToFrames`, `framesToTime` |
| Vectors | `length`, `normalize`, `add`, `sub`, `mul`, `div`, `dot`, `cross`, `lookAt` |
| Markers | `marker.key(i)`, `marker.nearestKey(t)` — beat-marker-driven animation is a first-class montage idiom |

Deliberately post-v1: `sampleImage` (pulls rendered pixels — serialises the graph),
layer-space transforms (`toComp` et al. — needs the 2.5D matrix surface finalised),
`sourceRectAtTime`, text/path creation APIs. An expression using an unimplemented name
fails cleanly at first evaluation: the property falls back to its keyframed value, the
expression is disabled with a calm badge naming the missing API, and the import report /
Timeline filter can list all disabled expressions.

### 4.3 Determinism rules

Binding, because distributed and repeated exports MUST agree bit-for-bit:

- No `Date`, no wall clock, no filesystem, no network, no `import`/`require`, no
  `setTimeout`, no locale-sensitive APIs. The global environment is frozen.
- `Math.random` is replaced by the seeded model: the PRNG is keyed by
  (property UUID, frame, user seed via `seedRandom`), exactly AE's semantics including
  `timeless`. Same project, same frame, same value — on any machine, any run.
- No JIT variance by construction (QuickJS is an interpreter); JS numbers are IEEE-754
  doubles, bit-identical across x86/ARM for the exposed operations.
- Evaluation order is fixed by the property dependency graph; expressions on different
  properties MUST NOT observe each other's evaluation order. Cross-property reads see the
  referenced property's evaluated value at the requested time, computed independently.
- Each evaluation runs under a memory limit and an interrupt-based time budget; a runaway
  expression is stopped, the property holds its last good value, and the expression gains
  the disabled badge — never a frozen UI, never a killed export (an export completes with
  the expression disabled and says so in the export log).

### 4.4 Performance model

- Expressions evaluate per property per frame on worker threads (never the UI thread,
  K-017), each in an isolated context.
- **Constant-expression detection**: an expression whose result provably cannot vary with
  time (no `time`, no property reads that vary, no randomness) is evaluated once per
  snapshot and cached — AE's 17.0 optimisation, done at parse time.
- `posterizeTime(fps)` clamps the evaluation clock, so wrapped expressions evaluate at the
  posterised rate and intermediate frames reuse the cached result.
- Expression results participate in content hashing: a property whose expression inputs
  are unchanged hashes identically, so frames above it stay cached
  ([05-ARCHITECTURE.md](05-ARCHITECTURE.md) §4.2).
- A per-frame expression budget is measured by the profiler; the Timeline's render-time
  view attributes cost to expression-heavy properties so users can find their own hot
  spots. Budget numbers live in [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md).

### 4.5 App scripting (future)

Project automation — batch import, layer generation, export-queue control, panels — is a
**separate runtime** from expressions, post-v1. Sketch, recorded now so nothing forecloses
it: same QuickJS engine, different embedding; scripts run against the command API
([05-ARCHITECTURE.md](05-ARCHITECTURE.md) §3) so every scripted change is an undoable
command; a Deno-style permission model — filesystem, network, and export rights granted
per script, per session, with the grant UI naming exactly what was requested; no ambient
authority. Expressions never gain these capabilities; the two runtimes stay distinct.

---

## 5. Security model

Plugins and expressions are **untrusted input**, including the ones the user installed on
purpose — the threat model covers both malice and bugs, and shared project files (K-065)
mean effects and expressions arrive from strangers by design.

Boundaries:

- **Expressions**: in-process but hermetic — no IO of any kind, no non-determinism (§4.3),
  memory and time limits per evaluation. The worst an expression can do is compute the
  wrong number slowly, once, then get disabled. Project files cannot smuggle capability:
  there is no API to reach the filesystem, network, or other processes from expression
  context at all.
- **Plugins (OFX and KFX)**: process isolation is the primary boundary — a plugin process
  owns no project data beyond the frames and parameter values sent to it, and its death is
  routine (§2.3). The plugin server process SHOULD run with reduced privilege: on Windows a
  restricted token / job object with no network by default; the exact mitigation set
  (token restrictions, job limits, AppContainer feasibility given plugins' licence
  activation needing user-profile and occasionally network access) is an open question
  below. Sandboxing MUST NOT silently break plugin licensing checks — a blocked capability
  surfaces as a per-plugin permission the user can grant.
- **Bundles and project files**: opening a `.lum` project or an AE-import bundle
  executes nothing — no plugin runs until the comp actually evaluates, expressions run
  only under §4.3's hermetic rules, and parsers treat all input as hostile (fuzzed in CI).
- **No dynamic code from projects**: project files carry expression *text* interpreted
  under the sandbox, never native code, never file paths that are auto-executed or
  auto-fetched.

---

## Open questions

- **OFX GPU suite audit** (carried from [05-ARCHITECTURE.md](05-ARCHITECTURE.md)): which of
  CUDA/OpenCL/Metal the actual targets (Twixtor, RSMB, Sapphire) require per platform, and
  whether any refuses to run CPU-only at acceptable speed — this schedules the GPU
  milestone.
- **Legacy OpenGL interacts**: plugins predating the 1.5 Draw Suite expect a GL context for
  overlay drawing; under wgpu/DX12 that means a GL interop context or an emulation shim.
  How many target plugins still need it, and is a shim worth it?
- **Windows mitigation set for plugin processes**: exactly which token restrictions, job
  object limits, and firewall rules the plugin server starts with, and which vendors'
  activation flows break under them — needs empirical testing with real licensed plugins.
- **OFX Message suite UX**: plugin-raised dialogs and progress from an out-of-process
  server need a policy (marshal to the UI thread; do modal vendor dialogs get shown, calm
  toast instead?). Decide with [07-UI-SPEC.md](07-UI-SPEC.md).
- **KFX curve/path parameter kind**: shape-warping effects want a bezier path parameter;
  does v1 of the parameter set include paths, or does that wait for the `kfx.overlay`
  extension where on-Viewer editing makes them usable?
- **Expression editor scope**: inline editor per property is assumed in
  [07-UI-SPEC.md](07-UI-SPEC.md); syntax highlighting, error ribbon, and pick-whip writing
  are UI-spec questions, but does the expression *language service* (autocomplete against
  the implemented subset) live in `luminal-expr` or the UI crate?
- **Shared plugin-server pooling**: one process per bundle is clean but a Sapphire-scale
  suite (hundreds of effects, one bundle) becomes a serialisation choke point if it flags
  thread-unsafe; measure whether per-instance worker processes are needed for heavyweight
  bundles.
