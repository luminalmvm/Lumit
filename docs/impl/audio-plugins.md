# Audio plugin hosting: CLAP and VST3 (K-683)

**Built so far: AP1 (K-692) and AP2 (K-696).** §1–§5 and §7 plans 1, 2, 5, 6 and 7
describe code that exists; §3's chain worker, §4's latency shift, §6 and AP3–AP5 are still
design.

[12-PLUGINS.md](../12-PLUGINS.md) §6 says Lumit hosts audio plugins; this note is the
binding *how*: the two APIs, the isolation, where a plugin sits in the mix, the
parameter/state/latency contracts, the traps, and the ordered work packages AP1–AP5.
Ground truth for the surrounding audio engine is [09-AUDIO.md](../09-AUDIO.md) §3.1 and
the approved AudioWorkspace board (its canvas note's four decisions — in particular
**"the stack is the rack, no new FX panel"**).

**In plain terms:** an audio plugin (a third-party EQ, compressor, reverb) is a piece of
someone else's code that takes a short run of sound samples and hands back a processed
run. Hosting one means: find it on disk, load it in a separate process so its crash is
not ours, describe its knobs as ordinary Lumit properties, feed it the layer's sound in
small fixed-size blocks slightly ahead of the playhead, and put whatever comes back into
the mix where the dry sound would have gone. Everything difficult in this note is one of
those five steps refusing to be simple.

## 1. The two APIs, and why CLAP goes first

- **CLAP** (clap.audio): MIT-licensed, a single family of C headers, stable since 1.0
  (2022). Everything beyond the tiny core (`clap_entry` → factory → `clap_plugin`) is a
  named **extension** the host and plugin negotiate; a host implementing only
  `audio-ports`, `params`, `state`, `latency` and `render` is a *complete, honest* host —
  a missing extension degrades cleanly instead of crashing. Direct FFI from Rust via
  `clap-sys` (MIT/Apache). This is the easier first host by a wide margin: the whole
  shape — discovery, describe, process, state — is provable with an in-tree Rust test
  plugin (a `.clap` is a DLL exporting one symbol) before any third-party binary enters
  the picture.
- **VST3** (Steinberg): dual-licensed — proprietary Steinberg agreement *or* **GPLv3** —
  so Lumit takes the GPLv3 branch cleanly, no signed agreement. Two loads to bear: the
  ABI is COM-style C++ vtables, and the plugin is split into an `IComponent`/
  `IAudioProcessor` (the DSP) and an `IEditController` (parameters and UI) that may be
  *separate objects* the host must wire together. Steinberg ships an official plain-C
  projection (`vst3_c_api.h`, same dual licence) — bind that, never the C++ headers.
- **VST2 is dead for us**: Steinberg stopped granting licences in 2018; there is no
  GPLv3-compatible road. Not hosted, not planned, recorded in K-683.

The order is CLAP host first (AP1), then the API-agnostic seams (AP2, AP3), then VST3 as
a second front end (AP4) onto a pipeline already proven end to end. Both APIs collapse
into one internal definition (`AudioEffectDef`, §4) the way OFX collapsed into
`OfxEffectDef` — nothing downstream of describe knows which standard a plugin speaks.

## 2. Where a plugin sits in the mix (decided)

**A plugin is an insert on the layer, ahead of Volume and Pan.** Per audible layer:

```
decoded source → audio effects in the layer's effect stack, in stack order
              → Volume (the §6 gain envelope — the fader)
              → Pan
              → sum of audible layers → master (limiter last)   [docs/09 §3.1]
```

Why pre-fader: Volume keyframes are the montage editor's fades, and a fade must fade the
*processed* sound — riding Volume down through a compressor's input would change the
compression amount mid-fade and audibly pump; post-insert gain keeps a fade a fade. It is
also every DAW's insert/fader order, the mixer drawn on the board (per-layer strips into
a Master strip), and canvas decision 2 (layers + master, no buses in v1).

**"The stack is the rack"** (canvas decision 1): there is no separate audio-FX list. The
glossary's Effect already admits audio operations; an audio plugin instance is an entry
in the layer's ordinary effect stack, and the layer's insert chain *is* the audio-typed
subset of that stack in stack order. Effect controls shows its rows like any effect's; the
Graph panel shows it as a node (mint audio family, K-472). An audio effect on a layer
with no sound is inert with the same calm the visibility switch rules use (K-435).
Sequence layers take the chain on the layer's mixed output (after clip retiming), exactly
where visual effects already sit.

## 3. The block contract, and why the broker never meets the callback

Session rate is the engine's **48 kHz** (docs/09 §2); block size is a fixed
**512 frames** (~10.7 ms), fp32. CLAP/VST3 both take **planar** float per channel; our
buffers are interleaved stereo — de/re-interleave at the plugin boundary, always into
separate in/out buffers (never in-place: `canProcessInPlace`/in-place port pairs are an
optimisation some plugins get wrong).

The realtime callback (docs/09 §3.1, K-017) must never wait on another process. So
**plugins never run in the callback's pull path**: a per-layer **chain worker** thread
pre-renders processed blocks ahead of the playhead into a small ring (8 blocks ≈ 85 ms
of lookahead), and the `MixPlan`'s placed entry for that layer reads the processed ring
instead of the raw decoded `Arc<AudioBuffer>`. A layer whose stack holds no audio effect
keeps the decoded buffer untouched — the empty chain costs nothing, which keeps today's
behaviour byte-identical (a regression test pins this).

**A dying plugin costs one block.** The chain worker gives each block a deadline (its
remaining lookahead margin, never less than one block period). A miss ships that block
**dry** — the chain's input, with a 5 ms equal-power ramp either side of the splice —
and files a strike; three strikes disable the plugin for the session with the calm badge,
the chain heals around it (identity), and `PluginPrefs`' per-plugin disable list is the
same one OFX reads (K-594). Dry over silence because in a montage the music continuing
slightly wrong beats a hole in it; the ramp because a splice click is worse than either.

**Determinism** (docs/09 §8: two exports produce identical PCM): block boundaries are a
pure function of the layer, not the playhead — each layer's chain processes from its
first sample in 512-frame blocks, always. Export (offline mode: VST3 `kOffline`, CLAP
`render` extension) runs that schedule from sample 0 with no deadline and no drops.
Preview started before the layer's in point is identical; preview after a **seek** into
the middle of a stateful effect (a reverb tail, a compressor's envelope) pre-rolls the
chain from up to 2 s before the seek (or the layer start when nearer), which is the
honest industry compromise — bit-exactness across a mid-layer seek would mean processing
from layer start every seek. Recorded, not hidden: the export is the contract, the
preview converges within the pre-roll. Plugins are trusted to be deterministic for the
export claim; one that is not (true randomness, unseeded) is a quirks-table entry, not an
engine failure.

Denormals: enable FTZ/DAZ on the chain worker (and the broker's processing thread) —
both APIs assume the host does, and a reverb tail hitting denormals is the classic
mystery CPU spike.

**Tempo**: both APIs offer transport/tempo to the plugin (CLAP `clap_event_transport`,
VST3 `ProcessContext`). When the comp has a confirmed BPM grid (docs/09 §5) its tempo and
phase are supplied; otherwise the tempo-valid flag stays off. This is the cheap half of
the beyond-NLE edge the canvas note names — a tempo-synced delay locks to the beat grid.

## 4. Parameters are properties; state is a blob

**Parameter ↔ property mapping** follows OFX §2.2's shape exactly: describe walks the
plugin's parameters (CLAP `params` extension: stable `u32` id, name, range, default,
flags; VST3 `IEditController` parameter list) and mints schema rows the way
[effect-registry.md](effect-registry.md) §2.6 mints them, so keyframes, curves,
expressions, drivers and the Timeline treat a plugin parameter as any property. The
**stable parameter id is the key**, never the index — plugins reorder and insert
parameters across versions. Hidden and non-automatable parameters
(`CLAP_PARAM_IS_HIDDEN`, missing `kCanAutomate`) get no rows; they live in the state
blob. VST3 traffics **normalised 0..1** values in its automation queues while properties
store plain values — convert at the boundary via the controller's
`plainParamToNormalized`, and never cache the conversion across a state load (plugins
re-scale ranges from state).

The row's **id is spelled from that number**: a CLAP parameter `1234` becomes the schema
row `p1234` (K-692). It is not pretty and it is not the parameter's name, because the name
is not stable — a vendor rewording a label would otherwise silently orphan a keyframed
value in every saved project. VST3 mints the same shape from its own `ParamID`.

Latency is asked for **only while the plugin is active**: CLAP's `latency.get` is an
active-state call and a describe never activates, so a descriptor records *whether* a
plugin reports latency and the chain reads the number off the live instance.

**Delivery**: keyframed values are baked per block — one value at each 512-frame block
start (~10.7 ms control rate, the same rate the Volume envelope already uses, K-172) —
as CLAP `CLAP_EVENT_PARAM_VALUE` events / VST3 `IParameterChanges` queue points, sorted
by time (CLAP requires it). Sample-accurate intra-block ramps are a later refinement if
a sweep ever audibly staircases; the envelope precedent says it will not at 10 ms.

**State**: an opaque blob in the `.lum`, per instance: `{api, plugin id, plugin version,
bytes}` — CLAP `state` extension streams one blob; VST3 stores **two** (processor
`IComponent::getState` + controller `IEditController::getState`). Never parsed, always
round-tripped, exactly like OFX custom parameters (docs/12 §2.2) under the K-040
versioned schema. Missing plugin at load = docs/12 §1's inert placeholder: rows, blob and
keyframes preserved, identity render, badge, nothing lost on save. Instantiation order:
create → load state (while deactivated; VST3 also `setComponentState` on the controller
so the two halves agree) → apply property values (**properties win** over state for
parameters that have rows — otherwise a stale blob overrides keyframes) → activate.

**Latency**: plugins report it (CLAP `latency` extension; VST3 `getLatencySamples` after
`setupProcessing`). Because chains are pre-rendered, compensation is free: shift the
layer's processed placement earlier by the chain's summed latency so the wet sound lands
where the dry did — lookahead limiters just work. A latency change on a parameter change
re-activates per CLAP's rules and re-places. Cap a chain at 10 000 samples with a badge
beyond (a "linear-phase everything" chain pushing a quarter second should be visible).

**Buses**: v1 hosts **stereo effect plugins only** — main in/out at 2ch (CLAP
`audio-ports` mains; VST3 `setBusArrangements(kStereo)`), aux/sidechain buses left
inactive, instruments (no main audio in) rejected at scan with a report row. The board's
*Duck under* wiring is what will eventually feed a sidechain; it needs the driver seam,
not a v1 guess.

## 5. Isolation: the OFX broker architecture, re-armed for audio

`lumit-aplug` (host: `clap` and `vst3` modules behind one `AudioEffectDef`) and
`lumit-aplug-broker` (the child binary), mirroring `lumit-ofx`/`lumit-ofx-broker` —
**the architecture is reused, the code is not shared**: the proto types (audio blocks,
param events) share nothing with frames, so a common crate would be an abstraction with
one and a half users. What carries over verbatim as *rules* (all learned the hard way,
K-589/K-592/K-594):

- One broker process per vendor module (a `.clap` file / `.vst3` bundle); a bundle's
  plugins share one broker.
- Handle registry with magic+kind validation — bad handle answers, never UB.
- The pipe is its own (never the child's stdout — plugins `printf`); first word is the
  protocol version; length-prefixed bincode.
- Blocks cross a shared-memory ring with per-slot header and hash. Blocks are 4 KB, not
  frames — the ring is small and the budget trivial.
- Watchdog: **per-block deadline = the lookahead margin** (§3), not OFX's 10 s; control
  actions (describe, activate, state) keep the 2 s. Three strikes → session disable +
  badge. Broker crash = restart + replay describe/instance from cached descriptors +
  state blob; the block that died ships dry.
- Discovery enumerates **inside the broker** (a `clap_entry.init` or `GetPluginFactory`
  is already third-party code running); descriptors are cached; the disable list is read
  before describe and again per block-batch. Search paths: CLAP —
  `%COMMONPROGRAMFILES%\CLAP`, `%LOCALAPPDATA%\Programs\Common\CLAP`, `CLAP_PATH`;
  VST3 — `%COMMONPROGRAMFILES%\VST3` (folder-bundle or legacy single file), plus the
  macOS equivalents.
- A quirks table from day one, same shape as OFX §2.5.
- `LocalHost` for tests, `BrokerHost` shipping — and the seams are built against
  `BrokerHost`, the OFX lesson.

Threading (docs/14: no locks across FFI): all FFI happens inside the broker process. In
the main process the chain worker's broker calls are deadline-bounded pipe writes/reads,
never under a lock the callback or UI can want. CLAP's main-thread/audio-thread function
split maps to the broker's control loop vs its processing thread; one instance is
processed single-threaded (parallelism is across layers, which is where the work is).

## 6. The editor window (decided honestly)

**v1 is parameters-only**: the derived rows in Effect controls and the board's
selected-layer rack are the surface, and an EQ/compressor/limiter-class plugin is
genuinely usable through rows. Plugin GUIs are native windows (CLAP `gui` extension;
VST3 `IPlugView`), and Flutter cannot adopt a foreign HWND into its own tree — but the
plugin already lives in the broker process, and a broker can own a **floating top-level
window** the plugin draws into (CLAP floating gui; VST3 attached to a broker-created
HWND). That is the follow-on package after AP5, not a v1 promise, and the honest cost of
deferring is real: a FabFilter-class visual EQ without its curve display is diminished,
not broken. Recorded in K-683 so nobody discovers the gap in a release note.

## 7. Test plans (implement with each package)

1. **In-tree test plugin** (`lumit-aplug-testplug`, mirroring `lumit-ofx-testplug`): a
   Rust cdylib exporting `clap_entry` with switchable personalities — fixed gain
   (processed == expected, sample-exact), reporter (records every host call for
   order-of-actions assertions), latency N, crash-on-block-N, hang, state echo,
   param-event echo. Most of AP1–AP3's tests drive it.
2. **Order of actions** pinned verbatim like `RENDER_ACTIONS` (K-591): factory →
   describe → create → load state → set params → activate → start_processing → blocks →
   stop → deactivate → destroy; a test records what the plugin observed and compares.
3. **Chain seam**: empty chain leaves the decoded buffer untouched (pointer-equal);
   gain-plugin chain matches hand-computed output through *both* the live plan and the
   baked mixdown, sample for sample (the K-172 preview==export shape); insert runs
   before Volume and Pan (fade over compressor: assert post-fade block == fade × wet,
   not wet-of-faded); latency N shifts placement by exactly N.
4. **Determinism**: two offline bakes of a stateful test chain are byte-identical; a
   mid-layer seek's preview converges to the export within the pre-roll window.
5. **Isolation** (the Gate-4 shape): crash-on-block-N → exactly one dry block (assert
   the splice ramps), broker restarts, session continues, layer badges; hang → deadline
   fires, same path; three strikes → disabled for the session, chain heals, badge says
   so; switched off mid-session → next batch renders without it.
6. **State**: save → reload project → blob handed back byte-identical; missing plugin →
   inert placeholder preserving rows + blob + keyframes; properties win over stale state.
7. **Param automation**: keyframed sweep arrives as sorted per-block events; VST3
   normalised/plain conversion round-trips the ends and the middle of an off-centre
   range.
8. **Conformance bench** (free, both APIs, CI like the OFX bench): **Airwindows
   Consolidated** (MIT; hundreds of effects, CLAP + VST3) and **Surge XT Effects**
   (GPLv3; CLAP + VST3) — describe → instantiate → state round-trip → 100 blocks, assert
   no bad-handle answers and finite output; rejections are rows, not failures (the OFX
   bench's discipline, K-595).
9. **Manual pass before a release that claims hosting** (the OFX commercial checklist's
   shape): Valhalla Supermassive (free) and a FabFilter demo, on the Windows machine —
   audition, automate, save/reload, pull the plugin, check the calm badge. Result
   recorded in the PR that ran it.

## 8. Ordered work packages

| # | Package | Lands |
|---|---|---|
| AP1 | CLAP host core ✅ | `lumit-aplug`: discovery, module load, describe → `AudioEffectDef`, params/state/latency/process against `LocalHost`; test plugin; plans 1, 2, 6, 7 (K-692) |
| AP2 | Broker isolation ✅ | `lumit-aplug-broker`: pipe + block ring + watchdog + disable list + quirks table + brokered scan; plan 5 (K-696) |
| AP3 | Mix-graph seam | chain worker, processed ring, dry fallback, latency shift, param envelopes, `.lum` persistence, offline bake; plans 3, 4 |
| AP4 | VST3 host | `vst3_c_api` bindings, component/controller wiring, bus arrangement, queues, two-blob state, through the same broker and seam; plan 8 both APIs |
| AP5 | Panel surface | Effects & presets audio category, rows in Effect controls, badges, per-plugin disable UI, arb keys; GUI window recorded as the follow-on |

## 9. Traps, collected

- VST3's component/controller split: state loads must hit both halves
  (`setComponentState` on the controller) or knobs and sound disagree after reload.
- VST3 normalised values in queues vs plain in properties — convert at the boundary,
  every time, from the controller.
- CLAP event lists must be time-sorted; an unsorted list is undefined per spec and
  crashes real plugins.
- Never call state/describe functions while processing; deactivate first (both APIs).
- Separate in/out buffers always; in-place is where plugin bugs live.
- FTZ/DAZ on every thread that calls `process`, broker included.
- Parameter **ids**, never indices, as the persistent key.
- The callback never waits on the broker — the lookahead ring is the only coupling, and
  a miss is one dry block by design, not a stall.
